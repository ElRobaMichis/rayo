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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
/// Factor de las duraciones (×1000): `RAYO_MOVIMIENTO_LENTO=10` las hace diez veces más lentas para
/// mirarlas con calma o capturar un cuadro a mitad de camino. Solo para desarrollo.
static SLOW_BY: AtomicU32 = AtomicU32::new(1000);

/// Lee si Windows tiene las animaciones activadas ("Efectos de animación"). Se vuelve a llamar al
/// mostrar la ventana: Rayo queda residente y el ajuste pudo cambiar mientras tanto.
pub fn init() {
    ON.store(windows_allows(), Ordering::Relaxed);
    let k = std::env::var("RAYO_MOVIMIENTO_LENTO").ok().and_then(|v| v.parse::<f32>().ok()).filter(|k| k.is_finite() && *k > 0.0).unwrap_or(1.0);
    SLOW_BY.store((k * 1000.0).round() as u32, Ordering::Relaxed);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Una duración de `design::dur` con el factor de desarrollo aplicado.
fn scaled(dur: f32) -> f32 {
    dur * SLOW_BY.load(Ordering::Relaxed) as f32 / 1000.0
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
    let dur = scaled(dur);
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
    // Duración 0 (o negativa): directo al destino (0/0 daría NaN).
    if dur <= 0.0 {
        return g.to;
    }
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
    let dur = scaled(dur);
    if dur <= 0.0 {
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
    let k = ((t - start) as f32 / scaled(dur::SLOW)).clamp(0.0, 1.0);
    if k >= 1.0 {
        return (0.0, 1.0);
    }
    ctx.request_repaint();
    let e = ease_out(k);
    (n.sign * DIST * (1.0 - e), FROM + (1.0 - FROM) * e)
}

// ---------------------------------------------------------------- filas con clave (tira de pestañas)

/// Clave estable de un lugar de la tira de pestañas: una pestaña o la etiqueta de un grupo (por id,
/// que nunca se reutiliza).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SlotKey {
    Tab(u64),
    Group(u64),
}

/// [x, ancho, presencia (0 = no está, 1 = está)].
type Geo = [f32; 3];

#[derive(Clone, Copy, Debug)]
struct Slot {
    key: SlotKey,
    from: Geo,
    to: Geo,
    start: f64,
    /// Ya no está en la fila: se cierra y se descarta al llegar.
    leaving: bool,
    /// La que se va: la sobreviviente a cuya derecha se cierra (None: el principio de la fila).
    anchor: Option<SlotKey>,
    /// Ancho que tenía como destino (con eso se decide qué mostrar mientras se cierra).
    full_w: f32,
}

impl Slot {
    fn progress(&self, t: f64, dur: f32) -> f32 {
        if dur <= 0.0 { 1.0 } else { ((t - self.start) as f32 / dur).clamp(0.0, 1.0) }
    }

    fn at(&self, t: f64, dur: f32) -> Geo {
        let k = self.progress(t, dur);
        if k >= 1.0 {
            return self.to;
        }
        let e = ease_out(k);
        [0, 1, 2].map(|i| self.from[i] + (self.to[i] - self.from[i]) * e)
    }

    fn done(&self, t: f64, dur: f32) -> bool {
        self.progress(t, dur) >= 1.0
    }

    fn settled(key: SlotKey, x: f32, w: f32) -> Slot {
        Slot { key, from: [x, w, 1.0], to: [x, w, 1.0], start: f64::MIN, leaving: false, anchor: None, full_w: w }
    }
}

/// Una fila de cosas con clave estable (chips y pestañas) que se acomodan solas: lo que aparece
/// crece desde el borde de la anterior, lo que se va se cierra contra la anterior que sigue, y lo
/// que cambia de lugar viaja. Todo lo que cambia en un cuadro arranca a la vez y con la misma
/// curva, así lo pintado es siempre una mezcla de dos filas válidas: nunca se solapan.
#[derive(Clone, Default, Debug)]
pub struct Strip {
    slots: Vec<Slot>,
    seen: u64,
    /// La que viaja (cambió de orden respecto de las demás): se pinta encima.
    top: Option<SlotKey>,
}

/// Un lugar pintado en este cuadro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub key: SlotKey,
    pub x: f32,
    pub w: f32,
    /// Ancho de destino: con esto se decide lo que tiene umbrales (título recortado, X), no con
    /// el ancho que se está animando.
    pub target_w: f32,
    /// 1 = presente. Sube de 0 a 1 al entrar, baja a 0 al irse.
    pub presence: f32,
    pub leaving: bool,
    /// Es la que viaja (cambió de orden): se pinta encima y tapa lo que cruza.
    pub top: bool,
}

impl Strip {
    /// Un cuadro: `targets` es la fila de destino (clave, x, ancho) en orden. Devuelve lo que se
    /// pinta (primero lo que se va, después en orden, al final la que viaja) y si algo se mueve.
    /// `animate` falso (animaciones de Windows apagadas): todo directo a su lugar.
    pub fn step(&mut self, targets: &[(SlotKey, f32, f32)], now: f64, pass: u64, animate: bool, dur: f32) -> (Vec<Placed>, bool) {
        let fresh = !animate
            || dur <= 0.0
            || self.slots.is_empty()
            || self.seen.saturating_add(2) < pass
            || !targets.iter().any(|(k, ..)| self.slots.iter().any(|s| s.key == *k));
        self.seen = pass;
        if fresh {
            self.slots = targets.iter().map(|&(k, x, w)| Slot::settled(k, x, w)).collect();
            self.top = None;
        } else if self.slots.iter().filter(|s| !s.leaving).map(|s| s.key).eq(targets.iter().map(|t| t.0)) {
            // Misma fila, otra geometría (ventana más ancha, otra pestaña activa): sin animar; lo
            // que estaba en vuelo sigue su camino desde el lugar nuevo.
            for s in self.slots.iter_mut() {
                if s.leaving {
                    let r = anchor_right(targets, s.anchor).unwrap_or(s.to[0]);
                    s.from[0] += r - s.to[0];
                    s.to[0] = r;
                } else if let Some(&(_, x, w)) = targets.iter().find(|t| t.0 == s.key) {
                    s.from[0] += x - s.to[0];
                    s.from[1] += w - s.to[1];
                    s.to[0] = x;
                    s.to[1] = w;
                    s.full_w = w;
                }
            }
        } else {
            self.restructure(targets, now, dur);
        }
        self.slots.retain(|s| !(s.leaving && s.done(now, dur)));
        if self.top.is_some_and(|k| !self.slots.iter().any(|s| s.key == k && !s.leaving && !s.done(now, dur))) {
            self.top = None;
        }
        let moving = self.slots.iter().any(|s| !s.done(now, dur));
        let place = |s: &Slot| {
            let g = s.at(now, dur);
            Placed { key: s.key, x: g[0], w: g[1].max(0.0), target_w: s.full_w, presence: g[2].clamp(0.0, 1.0), leaving: s.leaving, top: false }
        };
        let mut out: Vec<Placed> = self.slots.iter().filter(|s| s.leaving).map(place).collect();
        out.extend(self.slots.iter().filter(|s| !s.leaving && Some(s.key) != self.top).map(place));
        if let Some(s) = self.top.and_then(|k| self.slots.iter().find(|s| s.key == k && !s.leaving)) {
            out.push(Placed { top: true, ..place(s) });
        }
        (out, moving)
    }

    /// Cambió la lista de claves: cada una va desde donde está ahora hacia su destino.
    fn restructure(&mut self, targets: &[(SlotKey, f32, f32)], now: f64, dur: f32) {
        let old = std::mem::take(&mut self.slots);
        let in_targets = |k: SlotKey| targets.iter().any(|t| t.0 == k);
        let mut slots: Vec<Slot> = Vec::with_capacity(targets.len() + 4);
        let mut prev_right: Option<f32> = None;
        for &(k, x, w) in targets {
            // Si hay una viva y una que se iba con la misma clave, manda la viva.
            let found = old.iter().find(|s| s.key == k && !s.leaving).or_else(|| old.iter().find(|s| s.key == k));
            let from = match found {
                Some(s) => s.at(now, dur),
                // Nueva: nace con ancho 0 en el borde derecho de la anterior.
                None => [prev_right.unwrap_or(x), 0.0, 0.0],
            };
            prev_right = Some(from[0] + from[1]);
            slots.push(Slot { key: k, from, to: [x, w, 1.0], start: now, leaving: false, anchor: None, full_w: w });
        }
        // La que viaja: entre las que siguen, la que cambió de orden y recorre más.
        let old_order: Vec<SlotKey> = old.iter().filter(|s| !s.leaving && in_targets(s.key)).map(|s| s.key).collect();
        let new_order: Vec<SlotKey> = targets.iter().map(|t| t.0).filter(|k| old_order.contains(k)).collect();
        let traveler = new_order
            .iter()
            .enumerate()
            .filter(|(i, k)| old_order.get(*i) != Some(*k))
            .filter_map(|(_, k)| slots.iter().find(|s| s.key == *k))
            .max_by(|a, b| (a.to[0] - a.from[0]).abs().total_cmp(&(b.to[0] - b.from[0]).abs()))
            .map(|s| s.key);
        let keep_top = self.top.filter(|k| in_targets(*k) && old.iter().any(|s| s.key == *k && !s.done(now, dur)));
        self.top = traveler.or(keep_top);
        // Las que se van (y las que ya se iban): se cierran contra la sobreviviente anterior.
        for (i, s) in old.iter().enumerate() {
            if in_targets(s.key) || (s.leaving && s.done(now, dur)) {
                continue;
            }
            let anchor = old[..i].iter().rev().find(|p| in_targets(p.key)).map(|p| p.key);
            let cur = s.at(now, dur);
            let r = anchor_right(targets, anchor).unwrap_or(cur[0]);
            let ghost = Slot { key: s.key, from: cur, to: [r, 0.0, 0.0], start: now, leaving: true, anchor, full_w: s.full_w };
            // En la lista va después de su ancla (y de las que ya se cierran contra ella).
            let at = match anchor.and_then(|a| slots.iter().position(|x| x.key == a && !x.leaving)) {
                None => slots.iter().take_while(|x| x.leaving).count(),
                Some(p) => p + 1 + slots[p + 1..].iter().take_while(|x| x.leaving).count(),
            };
            slots.insert(at, ghost);
        }
        self.slots = slots;
    }

    /// Lo que se suelta (una pestaña arrastrada): su lugar arranca desde donde se la soltó y
    /// se pinta encima de las demás mientras se acomoda.
    pub fn seed(&mut self, key: SlotKey, x: f32, w: f32, now: f64) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.key == key && !s.leaving) {
            s.from = [x, w, 1.0];
            s.start = now;
        } else {
            self.slots.retain(|s| s.key != key);
            let mut s = Slot::settled(key, x, w);
            s.start = now;
            self.slots.push(s);
        }
        self.top = Some(key);
    }
}

/// Borde derecho de destino de la sobreviviente `anchor` (sin ancla: el principio de la fila).
fn anchor_right(targets: &[(SlotKey, f32, f32)], anchor: Option<SlotKey>) -> Option<f32> {
    anchor.and_then(|a| targets.iter().find(|t| t.0 == a)).map(|t| t.1 + t.2).or(targets.first().map(|t| t.1))
}

/// `Strip::step` con la hora y el cuadro de egui, guardada en `ctx.data` con `id`. Pide cuadros
/// solo mientras algo se mueve.
pub fn strip(ctx: &Context, id: Id, targets: &[(SlotKey, f32, f32)]) -> Vec<Placed> {
    let (t, p) = (now(ctx), pass(ctx));
    let mut s = ctx.data(|d| d.get_temp::<Strip>(id)).unwrap_or_default();
    let (out, moving) = s.step(targets, t, p, on(), scaled(dur::SLOW));
    ctx.data_mut(|d| d.insert_temp(id, s));
    if moving {
        ctx.request_repaint();
    }
    out
}

/// `Strip::seed` sobre la fila guardada con `id`.
pub fn strip_seed(ctx: &Context, id: Id, key: SlotKey, x: f32, w: f32) {
    let t = now(ctx);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Strip>(id).seed(key, x, w, t));
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use SlotKey::{Group as G, Tab as T};

    const D: f32 = 0.18;

    fn row(items: &[(SlotKey, f32)]) -> Vec<(SlotKey, f32, f32)> {
        let mut x = 0.0;
        items
            .iter()
            .map(|&(k, w)| {
                let r = (k, x, w);
                x += w;
                r
            })
            .collect()
    }

    fn find(p: &[Placed], k: SlotKey) -> Placed {
        *p.iter().find(|p| p.key == k).unwrap()
    }

    #[test]
    fn first_time_goes_straight() {
        let mut s = Strip::default();
        let (p, moving) = s.step(&row(&[(T(1), 100.0), (T(2), 80.0)]), 10.0, 1, true, D);
        assert!(!moving);
        assert_eq!(p.len(), 2);
        assert_eq!((p[1].x, p[1].w, p[1].presence), (100.0, 80.0, 1.0));
    }

    #[test]
    fn not_animated_when_off_or_stale() {
        let mut s = Strip::default();
        s.step(&row(&[(T(1), 100.0)]), 0.0, 1, true, D);
        // Animaciones apagadas: la nueva aparece entera.
        let (p, moving) = s.step(&row(&[(T(1), 100.0), (T(2), 100.0)]), 1.0, 2, false, D);
        assert!(!moving && p.iter().all(|p| p.presence == 1.0 && p.w == 100.0));
        // Sin pintarse unos cuadros (otra vista): directo.
        let (p, moving) = s.step(&row(&[(T(2), 100.0)]), 2.0, 9, true, D);
        assert!(!moving && p.len() == 1 && p[0].x == 0.0);
        // Ninguna clave sobrevive (otro espacio de trabajo): directo.
        let (p, moving) = s.step(&row(&[(T(7), 100.0), (T(8), 100.0)]), 3.0, 10, true, D);
        assert!(!moving && p.len() == 2);
    }

    #[test]
    fn geometry_change_is_not_animated() {
        let mut s = Strip::default();
        s.step(&row(&[(T(1), 100.0), (T(2), 100.0)]), 0.0, 1, true, D);
        let (p, moving) = s.step(&row(&[(T(1), 60.0), (T(2), 140.0)]), 1.0, 2, true, D);
        assert!(!moving);
        assert_eq!((p[1].x, p[1].w), (60.0, 140.0));
    }

    #[test]
    fn new_one_grows_from_previous_edge_and_others_slide() {
        let mut s = Strip::default();
        s.step(&row(&[(T(1), 100.0), (T(3), 100.0)]), 0.0, 1, true, D);
        let r = row(&[(T(1), 100.0), (T(2), 100.0), (T(3), 100.0)]);
        let (p, moving) = s.step(&r, 1.0, 2, true, D);
        assert!(moving);
        let two = find(&p, T(2));
        assert_eq!((two.x, two.w, two.presence, two.target_w), (100.0, 0.0, 0.0, 100.0));
        let (p, _) = s.step(&r, 1.0 + D as f64 / 2.0, 3, true, D);
        let (two, three) = (find(&p, T(2)), find(&p, T(3)));
        assert!(two.w > 0.0 && two.w < 100.0 && three.x > 100.0 && three.x < 200.0);
        let (p, moving) = s.step(&r, 1.0 + D as f64, 4, true, D);
        assert!(!moving);
        assert_eq!(p.iter().map(|p| (p.x, p.w)).collect::<Vec<_>>(), vec![(0.0, 100.0), (100.0, 100.0), (200.0, 100.0)]);
    }

    #[test]
    fn the_blend_never_overlaps() {
        // Plegar: el chip se ensancha y sus pestañas se cierran contra él; a la vez se abre otra.
        let mut s = Strip::default();
        s.step(&row(&[(G(1), 60.0), (T(1), 100.0), (T(2), 100.0), (T(3), 120.0)]), 0.0, 1, true, D);
        let r = row(&[(G(1), 80.0), (T(3), 160.0), (T(4), 160.0)]);
        for i in 0..=10u64 {
            let (p, _) = s.step(&r, 1.0 + D as f64 * i as f64 / 10.0, 2 + i, true, D);
            let mut all = p.clone();
            all.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.w.total_cmp(&b.w)));
            for w in all.windows(2) {
                assert!(w[0].x + w[0].w <= w[1].x + 0.01, "se solapan en {i}: {:?} y {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn leaving_ones_close_against_the_chip_and_are_dropped() {
        let mut s = Strip::default();
        s.step(&row(&[(G(1), 60.0), (T(1), 100.0), (T(2), 100.0)]), 0.0, 1, true, D);
        let r = row(&[(G(1), 80.0)]);
        let (p, _) = s.step(&r, 1.0, 2, true, D);
        assert_eq!(p.iter().filter(|p| p.leaving).count(), 2);
        // Se pintan primero las que se van.
        assert!(p[0].leaving && p[1].leaving && !p[2].leaving);
        let (p, moving) = s.step(&r, 1.0 + D as f64 * 0.999, 3, true, D);
        assert!(moving);
        for g in p.iter().filter(|p| p.leaving) {
            assert!((g.x - 80.0).abs() < 1.0 && g.w < 1.0 && g.target_w == 100.0, "{g:?}");
        }
        let (p, moving) = s.step(&r, 2.0, 4, true, D);
        assert!(!moving && p.len() == 1);
    }

    #[test]
    fn retarget_mid_flight_continues_from_where_it_was() {
        let mut s = Strip::default();
        s.step(&row(&[(T(1), 100.0), (T(2), 100.0)]), 0.0, 1, true, D);
        // Se cierra la 2 y, a mitad, vuelve: revive desde su ancho actual, no desde 0.
        s.step(&row(&[(T(1), 100.0)]), 1.0, 2, true, D);
        let (p, _) = s.step(&row(&[(T(1), 100.0)]), 1.0 + D as f64 / 2.0, 3, true, D);
        let mid = find(&p, T(2)).w;
        assert!(mid > 0.0 && mid < 100.0);
        let (p, moving) = s.step(&row(&[(T(1), 100.0), (T(2), 100.0)]), 1.0 + D as f64 / 2.0, 4, true, D);
        let two = find(&p, T(2));
        assert!(moving && !two.leaving && (two.w - mid).abs() < 0.01);
        // La ventana cambia de ancho en vuelo: la animación se corre, no se reinicia.
        let (p, _) = s.step(&row(&[(T(1), 50.0), (T(2), 100.0)]), 1.0 + D as f64 / 2.0, 5, true, D);
        let two = find(&p, T(2));
        assert!((two.x - 50.0).abs() < 0.01 && (two.w - mid).abs() < 0.01);
    }

    #[test]
    fn the_one_that_changes_order_goes_on_top() {
        let mut s = Strip::default();
        s.step(&row(&[(T(1), 100.0), (T(2), 100.0), (T(3), 100.0), (T(4), 100.0)]), 0.0, 1, true, D);
        let r = row(&[(T(2), 100.0), (T(3), 100.0), (T(4), 100.0), (T(1), 100.0)]);
        let (p, _) = s.step(&r, 1.0, 2, true, D);
        assert_eq!(p.last().unwrap().key, T(1));
        assert!(p.last().unwrap().top);
        // Al llegar deja de estar encima.
        let (p, _) = s.step(&r, 2.0, 3, true, D);
        assert!(p.iter().all(|p| !p.top));
        assert_eq!(p.iter().map(|p| p.x).collect::<Vec<_>>(), vec![0.0, 100.0, 200.0, 300.0]);
    }

    #[test]
    fn seed_slides_from_the_drop_point() {
        let mut s = Strip::default();
        let r = row(&[(T(1), 100.0), (T(2), 100.0)]);
        s.step(&r, 0.0, 1, true, D);
        s.seed(T(1), 140.0, 100.0, 1.0);
        let (p, moving) = s.step(&r, 1.0, 2, true, D);
        assert!(moving);
        assert_eq!((p.last().unwrap().key, p.last().unwrap().x), (T(1), 140.0));
        let (p, _) = s.step(&r, 1.0 + D as f64, 3, true, D);
        assert_eq!(find(&p, T(1)).x, 0.0);
    }

    #[test]
    fn glide_with_zero_duration_is_not_nan() {
        let g = Glide { from: 0.0, to: 5.0, start: 1.0, seen: 0 };
        assert_eq!(value(&g, 1.0, 0.0), 5.0);
    }
}
