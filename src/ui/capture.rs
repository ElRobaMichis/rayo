//! Captura de la ventana para revisar el diseño (solo desarrollo): con `RAYO_CAPTURA=ruta.png`,
//! a los `RAYO_CAPTURA_ESPERA` segundos (2,5 si no se dice) Rayo guarda lo que dibujó en un PNG y
//! se cierra. Lo usa `scripts/captura.ps1`. Desde afuera no se puede: Windows no deja leer lo que
//! dibuja OpenGL con `PrintWindow`.

use std::path::PathBuf;
use std::sync::OnceLock;

fn target() -> Option<&'static (PathBuf, f64)> {
    static T: OnceLock<Option<(PathBuf, f64)>> = OnceLock::new();
    T.get_or_init(|| {
        let path = std::env::var_os("RAYO_CAPTURA").filter(|v| !v.is_empty())?;
        let wait = std::env::var("RAYO_CAPTURA_ESPERA").ok().and_then(|v| v.parse().ok()).unwrap_or(2.5);
        Some((PathBuf::from(path), wait))
    })
    .as_ref()
}

/// ¿Se está sacando una captura? (para abrir lo que se quiera mostrar).
pub fn active() -> bool {
    target().is_some()
}

/// Qué abrir antes de capturar (`RAYO_CAPTURA_ABRIR`: `paleta`, `borrar`… o varios separados por
/// comas, como `terminal,git`). Devuelve la lista una sola vez, al segundo de arrancar.
pub fn open_once(ctx: &egui::Context) -> Option<String> {
    target()?;
    let what = std::env::var("RAYO_CAPTURA_ABRIR").ok().filter(|v| !v.is_empty())?;
    let id = egui::Id::new("rayo-captura-abierto");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) || ctx.input(|i| i.time) < 1.0 {
        return None;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
    Some(what)
}

/// Pasos que tocan ahora: los de `RAYO_CAPTURA_ABRIR` (al segundo de arrancar) y los que se
/// dejaron para después con `later` (p. ej. escribir en un campo que recién se abrió).
pub fn due(ctx: &egui::Context) -> Vec<String> {
    if target().is_none() {
        return Vec::new();
    }
    let mut out: Vec<String> = open_once(ctx).map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let now = ctx.input(|i| i.time);
    let mut later = LATER.lock();
    let mut k = 0;
    while k < later.len() {
        if later[k].0 <= now {
            out.push(later.remove(k).1);
        } else {
            k += 1;
        }
    }
    out
}

static LATER: parking_lot::Mutex<Vec<(f64, String)>> = parking_lot::Mutex::new(Vec::new());

/// Dejar un paso para dentro de `secs` segundos.
pub fn later(ctx: &egui::Context, secs: f64, step: &str) {
    let at = ctx.input(|i| i.time) + secs;
    LATER.lock().push((at, step.to_string()));
}

// ---------------------------------------------------------------- entrada simulada
//
// Algunas cosas solo se abren con el mouse (menús de egui, la lista de ▸ de la ruta): se simula
// el clic donde quedó ese control en el cuadro anterior. Los eventos entran por
// `App::raw_input_hook`, un grupo por cuadro.

static EVENTS: parking_lot::Mutex<std::collections::VecDeque<Vec<egui::Event>>> = parking_lot::Mutex::new(std::collections::VecDeque::new());

/// Mete los eventos simulados del cuadro (solo mientras se captura).
pub fn inject(raw: &mut egui::RawInput) {
    if target().is_none() {
        return;
    }
    // Como si la ventana tuviera el foco (la captura la abre sin activarla): si no, egui no da
    // el teclado a ningún campo y no se ve, p. ej., el historial de búsquedas.
    raw.focused = true;
    if let Some(batch) = EVENTS.lock().pop_front() {
        raw.events.extend(batch);
    }
}

fn push_frames(frames: Vec<Vec<egui::Event>>) {
    EVENTS.lock().extend(frames);
}

/// Clic (o clic derecho) en un punto de la ventana.
pub fn click(pos: egui::Pos2, button: egui::PointerButton) {
    let ev = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
    push_frames(vec![vec![egui::Event::PointerMoved(pos)], vec![ev(true)], vec![ev(false)]]);
}

/// Clic en el centro del control `id` (como quedó en el cuadro anterior). false si no está.
pub fn click_on(ctx: &egui::Context, id: egui::Id, button: egui::PointerButton) -> bool {
    match ctx.read_response(id) {
        Some(r) if r.rect.is_positive() => {
            click(r.rect.center(), button);
            true
        }
        _ => false,
    }
}

/// Teclas sueltas (sin modificadores), una por cuadro.
pub fn keys(list: &[egui::Key]) {
    let frames = list
        .iter()
        .map(|&key| {
            let ev = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
            vec![ev(true), ev(false)]
        })
        .collect();
    push_frames(frames);
}

/// Texto escrito (como si se tecleara de una vez).
pub fn type_text(s: &str) {
    push_frames(vec![vec![egui::Event::Text(s.to_string())]]);
}

/// El menú de clic derecho va en ventanas propias, que la captura no ve: mientras se captura
/// se dibuja dentro de la ventana principal.
pub fn inline_menus() -> bool {
    target().is_some()
}

/// Una vez por cuadro.
pub fn tick(ctx: &egui::Context) {
    let Some((path, wait)) = target() else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let id = egui::Id::new("rayo-captura-pedida");
    let asked = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !asked && ctx.input(|i| i.time) > *wait {
        ctx.data_mut(|d| d.insert_temp(id, true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    let image = ctx.input(|i| {
        i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
    });
    if let Some(image) = image {
        let [w, h] = image.size;
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
        if let Err(e) = std::fs::write(path, png(w as u32, h as u32, &rgba)) {
            crate::diag::log(&format!("captura: {e}"));
        }
        std::process::exit(0);
    }
}

/// PNG sin comprimir (bloques "stored" de deflate): no hace falta ninguna dependencia.
fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", ihdr.as_slice()), (b"IDAT", z.as_slice()), (b"IEND", &[][..])] {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend(crc.to_be_bytes());
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &x in data {
        c ^= x as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}
