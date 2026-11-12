//! Medición de tirones, para desarrollo. Con `RAYO_PERF=1`:
//!
//! - cada cuadro de la ventana principal que pasa de `SLOW_MS` de CPU (interfaz + triángulos +
//!   dibujo; sin la espera al monitor) queda en el diagnóstico con lo que tardó cada parte: las
//!   del cuadro, las de la Vista rápida (que se dibuja dentro), los MB de texturas subidos y los
//!   tramos marcados con `scope`;
//! - cada cuadro, una línea en `perf.csv` (junto al diagnóstico) para sacar mediana, p99 y peor
//!   por escenario.
//!
//! Apagado (lo normal) no cuesta nada: `scope` lee un byte y devuelve `None`; no se instala nada
//! en eframe ni en egui.

use std::cell::RefCell;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

/// Un cuadro que tarda más que esto (ms) se anota con detalle: el presupuesto a 240 Hz. Otro
/// umbral: `RAYO_PERF_MS`.
pub const SLOW_MS: f32 = 4.0;

fn slow_ms() -> f32 {
    static MS: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *MS.get_or_init(|| std::env::var("RAYO_PERF_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(SLOW_MS))
}

/// 0 = sin mirar todavía, 1 = apagado, 2 = prendido.
static STATE: AtomicU8 = AtomicU8::new(0);

/// ¿Se está midiendo? (`RAYO_PERF` con algo que no sea «0»).
#[inline]
pub fn on() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let v = std::env::var_os("RAYO_PERF").is_some_and(|v| !v.is_empty() && v != "0");
            STATE.store(if v { 2 } else { 1 }, Ordering::Relaxed);
            v
        }
    }
}

/// Un tramo con nombre: cuenta desde que se crea hasta que se suelta.
pub struct Scope {
    name: &'static str,
    t0: Instant,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let dt = self.t0.elapsed();
        FRAME.with(|f| f.borrow_mut().add(self.name, dt));
    }
}

/// Mide un tramo del hilo de la interfaz: `let _p = perf::scope("ql.cuerpo");`.
#[inline]
pub fn scope(name: &'static str) -> Option<Scope> {
    if on() { Some(Scope { name, t0: Instant::now() }) } else { None }
}

/// Lo que se juntó durante el cuadro en curso.
#[derive(Default)]
struct Frame {
    /// Tramos por nombre (en el orden en que aparecieron): total y veces.
    scopes: Vec<(&'static str, Duration, u32)>,
    /// Ventanas inmediatas dibujadas dentro de este cuadro (la Vista rápida).
    others: Vec<eframe::FrameTimes>,
    /// Bytes de texturas mandados a la GPU (todas las ventanas).
    upload: usize,
    uploads: u32,
}

impl Frame {
    fn add(&mut self, name: &'static str, dt: Duration) {
        match self.scopes.iter_mut().find(|s| s.0 == name) {
            Some(s) => {
                s.1 += dt;
                s.2 += 1;
            }
            None => self.scopes.push((name, dt, 1)),
        }
    }
}

struct Log {
    csv: Option<std::io::BufWriter<std::fs::File>>,
    flushed: Instant,
}

thread_local! {
    static FRAME: RefCell<Frame> = RefCell::new(Frame::default());
    static LOG: RefCell<Log> = RefCell::new(Log { csv: open_csv(), flushed: Instant::now() });
}

fn open_csv() -> Option<std::io::BufWriter<std::fs::File>> {
    let dir = crate::instance::custom_data_dir().or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Rayo")))?;
    let _ = std::fs::create_dir_all(&dir);
    let f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("perf.csv")).ok()?;
    let mut w = std::io::BufWriter::new(f);
    let _ = writeln!(w, "ms_unix,cpu,update,tess,paint,swap,ql_update,ql_tess,ql_paint,ql_wait,total,upload_kb");
    Some(w)
}

fn ms(s: f32) -> f32 {
    s * 1000.0
}

/// Fin de un cuadro de una ventana (lo llama eframe).
///
/// La Vista rápida se dibuja dentro de la interfaz de la raíz. Lo que su cuadro pasa esperando a
/// OpenGL (cambiar de contexto, presentar) no es trabajo: el controlador frena ahí hasta el
/// próximo refresco del monitor. Se cuenta aparte («espera») y no entra en «cpu».
fn on_frame(t: eframe::FrameTimes) {
    if t.viewport != egui::ViewportId::ROOT {
        FRAME.with(|f| f.borrow_mut().others.push(t));
        return;
    }
    let f = FRAME.with(|f| std::mem::take(&mut *f.borrow_mut()));
    // Vista rápida: (interfaz, triángulos, dibujo, espera).
    let ql = f.others.iter().fold((0.0, 0.0, 0.0, 0.0), |a, o| {
        let work = o.update + o.tessellate + o.paint;
        (a.0 + ms(o.update), a.1 + ms(o.tessellate), a.2 + ms(o.paint), a.3 + ms((o.total - work).max(0.0)))
    });
    let cpu = (ms(t.update) - ql.3).max(0.0) + ms(t.tessellate + t.paint);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(w) = l.csv.as_mut() {
            let _ = writeln!(
                w,
                "{now},{cpu:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
                ms(t.update),
                ms(t.tessellate),
                ms(t.paint),
                ms(t.swap + t.switch),
                ql.0,
                ql.1,
                ql.2,
                ql.3,
                ms(t.total),
                f.upload / 1024
            );
        }
        if l.flushed.elapsed() > Duration::from_secs(1) {
            l.flushed = Instant::now();
            if let Some(w) = l.csv.as_mut() {
                let _ = w.flush();
            }
        }
    });
    if cpu <= slow_ms() {
        return;
    }
    let mut msg = format!("perf: cuadro {cpu:.1} ms (interfaz {:.1} · triángulos {:.1} · dibujo {:.1})", ms(t.update) - ql.3, ms(t.tessellate), ms(t.paint));
    if !f.others.is_empty() {
        msg += &format!(" · vista rápida (interfaz {:.1} · triángulos {:.1} · dibujo {:.1} · espera {:.1})", ql.0, ql.1, ql.2, ql.3);
    }
    if f.uploads > 0 {
        msg += &format!(" · {} texturas {:.1} MB", f.uploads, f.upload as f32 / 1048576.0);
    }
    for (name, dt, n) in &f.scopes {
        let d = dt.as_secs_f32() * 1000.0;
        if d >= 0.2 {
            msg += &if *n > 1 { format!(" | {name} {d:.1}×{n}") } else { format!(" | {name} {d:.1}") };
        }
    }
    crate::diag::log(&msg);
}

/// Cuenta los bytes de texturas que cada pasada manda a la GPU.
struct Uploads;

impl egui::Plugin for Uploads {
    fn debug_name(&self) -> &'static str {
        "rayo-perf"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, out: &mut egui::FullOutput) {
        let deltas = out.textures_delta.set.iter().flat_map(|(_, d)| d.iter());
        let (bytes, n) = deltas.fold((0, 0), |(b, n), d| (b + d.image.width() * d.image.height() * 4, n + 1));
        FRAME.with(|f| {
            let mut f = f.borrow_mut();
            f.upload += bytes;
            f.uploads += n;
        });
    }
}

/// Engancha la medición (una vez; solo si `on`).
pub fn install(ctx: &egui::Context) {
    static DONE: std::sync::Once = std::sync::Once::new();
    if !on() {
        return;
    }
    DONE.call_once(|| {
        let _ = eframe::FRAME_HOOK.set(on_frame);
        ctx.add_plugin(Uploads);
        crate::diag::log(&format!("perf: midiendo (se anotan los cuadros de más de {} ms)", slow_ms()));
    });
}
