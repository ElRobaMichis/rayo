//! Movimiento: lo que hace que navegar se sienta suave en vez de a saltos. La selección se desliza
//! a la fila nueva, el hover aparece y se apaga con un fundido, al entrar a una carpeta su
//! contenido entra deslizándose y los íconos aparecen sin parpadear.
//!
//! Reglas: nada hace esperar (el contenido ya está; el movimiento corre encima), todo se puede
//! interrumpir (si el destino cambia a medio camino, sigue desde donde iba) y dura una de las
//! tres duraciones de `design::dur` (80, 120 o 180 ms), siempre con `ease_out`. Solo se piden
//! cuadros mientras algo se mueve: quieto, Rayo no gasta nada. Si en Windows están desactivadas
//! las animaciones, aquí también.

use super::design::dur;
use egui::{Context, Id};
use std::sync::atomic::{AtomicBool, Ordering};

// Tiempos que no son animación (esperas y cuánto se muestra algo), en segundos.
/// Arrastrando algo sobre una pestaña: cuánto hay que quedarse para que se abra.
pub const DWELL_TAB: f64 = 0.45;
/// Puntero quieto sobre una carpeta: cuánto esperar antes de leerla por adelantado.
pub const DWELL_PREFETCH: f64 = 0.08;
/// Cuánto se ve un aviso.
pub const TOAST: f64 = 2.5;
/// Cuánto se ve una confirmación en el lugar («Copiado»).
pub const FEEDBACK: f64 = 1.5;

static ON: AtomicBool = AtomicBool::new(true);

/// Lee si Windows tiene las animaciones activadas ("Efectos de animación").
pub fn init() {
    ON.store(windows_allows(), Ordering::Relaxed);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

fn windows_allows() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
    let mut v = windows::core::BOOL(1);
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut v as *mut _ as *mut std::ffi::c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok();
    !ok || v.as_bool()
}

/// Curva que arranca rápido y frena suave (lo que se siente natural en pantalla).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn now(ctx: &Context) -> f64 {
    ctx.input(|i| i.time)
}

fn pass(ctx: &Context) -> u64 {
    ctx.cumulative_pass_nr()
}

#[derive(Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    start: f64,
    seen: u64,
}

/// Un valor que va hacia `target` en `dur` segundos. Si el destino cambia a medio camino, sigue
/// desde donde iba. Lo que no se dibujó en el cuadro anterior (otra carpeta, fuera de la vista)
/// vuelve directo a su sitio, sin recorridos raros.
pub fn glide(ctx: &Context, id: Id, target: f32, dur: f32) -> f32 {
    if !on() {
        return target;
    }
    let (t, p) = (now(ctx), pass(ctx));
    let mut g = ctx.data(|d| d.get_temp::<Glide>(id)).filter(|g| g.seen + 2 >= p).unwrap_or(Glide { from: target, to: target, start: f64::MIN, seen: p });
    if g.to != target {
        g = Glide { from: value(&g, t, dur), to: target, start: t, seen: p };
    }
    g.seen = p;
    ctx.data_mut(|d| d.insert_temp(id, g));
    let v = value(&g, t, dur);
    if v != g.to {
        ctx.request_repaint();
    }
    v
}

fn value(g: &Glide, t: f64, dur: f32) -> f32 {
    let k = ((t - g.start) as f32 / dur).clamp(0.0, 1.0);
    if k >= 1.0 { g.to } else { g.from + (g.to - g.from) * ease_out(k) }
}

/// Fundido entre 0 y 1 (hover, resaltados).
pub fn fade(ctx: &Context, id: Id, on: bool, dur: f32) -> f32 {
    glide(ctx, id, if on { 1.0 } else { 0.0 }, dur)
}

/// Cuánto lleva algo que llegó en el instante `at` (de `ctx.input().time`): para íconos y
/// miniaturas que aparecen con un fundido en vez de cambiar de golpe.
pub fn since(ctx: &Context, at: f64, dur: f32) -> f32 {
    if !on() {
        return 1.0;
    }
    let k = ((ctx.input(|i| i.time) - at) as f32 / dur).clamp(0.0, 1.0);
    if k < 1.0 {
        ctx.request_repaint();
    }
    ease_out(k)
}

#[derive(Clone)]
struct Nav {
    path: String,
    /// Cuándo empezó la entrada (None: esperando que la carpeta tenga algo que mostrar).
    start: Option<f64>,
    sign: f32,
}

/// Al cambiar de carpeta, el contenido entra deslizándose: desde la derecha al entrar a una
/// subcarpeta, desde la izquierda al subir, y con un fundido si es otro lugar. Espera a que la
/// carpeta tenga algo que mostrar (`ready`), así no se anima un "Cargando…".
/// Devuelve (desplazamiento x, opacidad).
pub fn nav(ctx: &Context, id: Id, path: &str, ready: bool) -> (f32, f32) {
    // La receta única de entrada de contenido: 12 px y opacidad desde 0.3.
    const DIST: f32 = 12.0;
    const FROM: f32 = 0.3;
    if !on() {
        return (0.0, 1.0);
    }
    let t = now(ctx);
    let mut n = match ctx.data(|d| d.get_temp::<Nav>(id)) {
        Some(n) => n,
        None => {
            // La primera vez (al abrir la pestaña) no se anima.
            ctx.data_mut(|d| d.insert_temp(id, Nav { path: path.to_string(), start: Some(f64::MIN), sign: 0.0 }));
            return (0.0, 1.0);
        }
    };
    if n.path != path {
        let (old, new) = (n.path.to_lowercase(), path.to_lowercase());
        let inside = |a: &str, b: &str| a.len() > b.len() && a.starts_with(b.trim_end_matches('\\')) && a[b.trim_end_matches('\\').len()..].starts_with('\\');
        let sign = if inside(&new, &old) {
            1.0
        } else if inside(&old, &new) {
            -1.0
        } else {
            0.0
        };
        n = Nav { path: path.to_string(), start: None, sign };
    }
    if n.start.is_none() && ready {
        n.start = Some(t);
    }
    ctx.data_mut(|d| d.insert_temp(id, n.clone()));
    let Some(start) = n.start else { return (0.0, 1.0) };
    let k = ((t - start) as f32 / dur::SLOW).clamp(0.0, 1.0);
    if k >= 1.0 {
        return (0.0, 1.0);
    }
    ctx.request_repaint();
    let e = ease_out(k);
    (n.sign * DIST * (1.0 - e), FROM + (1.0 - FROM) * e)
}
