//! Vista rápida (barra espaciadora) en una ventana nativa propia: se mueve, se redimensiona,
//! no toma el foco y sigue a la selección. Ver `reports/vista-rapida-diseno.md` y
//! `reports/vista-rapida-plan.md`.
//!
//! No conoce a `App`: recibe lo que necesita (`Env`, `Model`) y devuelve `Event`s. El marco
//! (sin barra de título, sin foco, maximizar) es de `qlwin`; el cuerpo, de `qlview` y `qlmedia`.
//!
//! Capturando (`capture::inline_windows`), la misma ventana se dibuja dentro de la principal.

use super::config::QuickLookCfg;
use super::design::{self, Chip, IconBtn, Tone, h, icon, radius, sp, stroke};
use super::preview::{PreviewItem, Textures};
use super::prevhost::PreviewHost;
use super::theme::Theme;
use super::{qlmedia, qlview, qlwin};
use crate::fsx;
use egui::{Align2, Color32, Id, Key, Rect, RectAlign, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use egui_phosphor::regular as ic;
use parking_lot::Mutex;
use windows::Win32::Foundation::HWND;

/// Título de la ventana nativa: por él la encuentra `qlwin::adopt`.
pub const TITLE: &str = "rayo-vista-rapida";

// Medidas propias de esta ventana (ningún token nuevo: ver el diseño, §8).
/// Tamaño la primera vez (con encabezado), centrada sobre Rayo.
pub const QL_DEFAULT: Vec2 = Vec2::new(760.0, 560.0);
pub const QL_MIN: Vec2 = Vec2::new(380.0, 300.0);
/// Por debajo de este ancho: «Abrir con» solo ícono, sin tira, sin volumen, placa chica.
pub const QL_NARROW: f32 = 540.0;
/// Con varios seleccionados y por debajo de este ancho, la herramienta se va.
pub const QL_NARROW_MULTI: f32 = 440.0;
/// Lo mínimo que conserva el nombre en el encabezado.
pub const NAME_MIN: f32 = 96.0;
/// Ancho máximo del botón «Abrir con ‹programa›».
pub const OPEN_WITH_MAX: f32 = 240.0;
/// Tira de miniaturas del PDF y sus miniaturas.
pub const STRIP_W: f32 = 128.0;
pub const STRIP_THUMB: f32 = 96.0;
/// Placa de la carátula del audio (angosta: 96).
pub const ART: f32 = 144.0;
pub const ART_NARROW: f32 = 96.0;
/// Columna máxima del Markdown y ancho máximo de una página de PDF.
pub const MD_MAX: f32 = 680.0;
pub const PDF_MAX: f32 = 960.0;
/// Placa de reproducir sobre el póster del video.
pub const PLAY_PLATE: f32 = 48.0;
/// Glifo grande (carpeta, tipo) e ícono de Windows grande del cuerpo.
pub const HERO_GLYPH: f32 = 104.0;
pub const HERO_ICON: f32 = 96.0;
/// Tarjeta de datos de la carpeta; aviso de archivo bloqueado.
pub const FOLDER_CARD_W: f32 = 280.0;
pub const NOTICE_W: f32 = 320.0;
/// Zoom de imagen: paso de la rueda y máximo (encajada es el mínimo).
pub const ZOOM_STEP: f32 = 1.25;
pub const ZOOM_MAX: f32 = 8.0;
/// Cuánto texto se lee.
pub const TEXT_MAX: usize = 512 * 1024;
/// Demora antes de abrir lo caro (audio, video, visor de Office, GIF), en segundos.
pub const OPEN_DELAY: f64 = 0.120;
/// Si a los 150 ms no hay nada que mostrar: «Cargando».
pub const LOADING_AFTER: f64 = 0.150;
/// Cierre en dos tiempos: cuánto se espera a que el visor y el video vuelvan a casa.
pub const CLOSE_WAIT: f64 = 0.300;
/// Lo mínimo visible (px) del lugar recordado para volver a abrir ahí.
pub const VISIBLE_MIN: [i32; 2] = [120, 80];

/// Separador vertical del encabezado: alto.
const SEP_H: f32 = 16.0;
/// «Abrir con»: margen izquierdo y derecho.
const OPEN_PAD: (f32, f32) = (8.0, 10.0);

/// La rueda que llegó a la principal con el puntero sobre la Vista rápida («Desplazar ventanas
/// inactivas» apagado): se le pasa a la ventana en su próximo cuadro.
static WHEEL: Mutex<Vec<egui::Event>> = Mutex::new(Vec::new());

/// La Vista rápida. Vive en `App`; `show` se llama una vez por cuadro.
#[derive(Default)]
pub struct QuickLook {
    pub open: bool,
    /// `qlwin::adopt` ya encontró y preparó la ventana.
    adopted: bool,
    /// Cierre en dos tiempos: desde cuándo (NaN: recién pedido; ver plan §4).
    closing: Option<f64>,
    /// Rectángulo con el que se abrió (px): el `ViewportBuilder` lleva siempre este.
    opened_rect: [i32; 4],
    /// Con varios seleccionados: cuál se ve.
    index: usize,
    view: qlview::ViewState,
    media: qlmedia::MediaState,
    /// La ventana está declarada (existe para egui).
    declared: bool,
    /// Oculta a mano mientras se cierra.
    hidden: bool,
    /// Cuándo se abrió (fundido del contenido) y cuándo pasó al archivo actual (demora).
    opened_at: f64,
    shown_at: f64,
    /// Lo que se ve (ruta; vacío = nada), de dónde se lee (para soltar el PDF) y una copia,
    /// para las teclas.
    shown: String,
    shown_src: String,
    current: Option<PreviewItem>,
    /// El archivo actual se reproduce (← → son de `qlmedia`).
    is_media: bool,
    /// Teclas para `qlmedia` (las atiende `show`, que tiene el reproductor).
    media_keys: Vec<Key>,
    /// La selección múltiple que se recorre (huella) y cuántos son.
    group_sig: u64,
    group_len: usize,
    /// F11 pendiente.
    toggle_max: bool,
    /// El visor de Windows está en esta ventana; el video también.
    host_in: bool,
    video_in: bool,
    /// En el cuadro anterior había una ventana nativa en el cuerpo (sin tooltips encima).
    native: bool,
    /// Ícono de una app empaquetada (PNG → textura), por ruta.
    app_icon: Option<(String, Option<egui::TextureHandle>)>,
    /// `cfg` cambió y falta guardarlo (se guarda al soltar).
    cfg_dirty: bool,
}

/// Lo que la ventana usa de la app.
pub struct Env<'a> {
    pub t: &'a Theme,
    pub tex: &'a mut Textures,
    pub host: Option<&'a PreviewHost>,
    pub player: Option<&'a fsx::media::Player>,
    pub cfg: &'a mut QuickLookCfg,
    /// Ventana principal (dueña de esta).
    pub main: Option<HWND>,
}

/// Qué mostrar.
pub struct Model<'a> {
    /// El enfocado; None = «Sin selección».
    pub item: Option<&'a PreviewItem>,
    pub text: Option<&'a str>,
    /// Rutas de la selección múltiple, en el orden de la lista (vacío si hay una sola).
    pub group: &'a [String],
    /// Resultado de búsqueda o vista plana: herramienta «Mostrar en su carpeta».
    pub from_search: bool,
}

/// Lo que la app tiene que hacer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Close,
    Open { path: String, is_dir: bool },
    OpenWith(String),
    Reveal(String),
    /// «Desbloquear» un archivo descargado de Internet (el aviso del cuerpo).
    Unblock(String),
    /// Cambió `cfg` (lugar, tira, volumen): guardar.
    SaveCfg,
}

/// Lo que hizo el encabezado en este cuadro.
#[derive(Default)]
struct HeadOut {
    /// Zonas con botones (el resto arrastra la ventana).
    buttons: Vec<Rect>,
}

fn copy_item(it: &PreviewItem) -> PreviewItem {
    PreviewItem { path: it.path.clone(), real: it.real.clone(), entry: it.entry.clone(), children: it.children }
}

/// Tooltip de un botón del encabezado. Con una ventana nativa en el cuerpo, a un costado (`side`)
/// y no debajo, donde el video o el visor de Windows lo taparían.
fn head_tip(r: egui::Response, t: &Theme, native: bool, text: &str, keys: &str, side: RectAlign) -> egui::Response {
    if native { design::tip_keys_beside(r, t, text, keys, side) } else { design::tip_keys(r, t, text, keys) }
}

fn px_rect(r: Rect, ppp: f32) -> [i32; 4] {
    [(r.left() * ppp).round() as i32, (r.top() * ppp).round() as i32, (r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32]
}

/// Desde el gancho de entrada de la principal: si la rueda llegó ahí con el puntero sobre la
/// Vista rápida, se la lleva (la ventana la recibe en su cuadro).
pub fn take_wheel(raw: &mut egui::RawInput, open: bool) {
    if !open || !raw.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })) || !qlwin::cursor_inside() {
        return;
    }
    let mut w = WHEEL.lock();
    raw.events.retain(|e| {
        if matches!(e, egui::Event::MouseWheel { .. }) {
            w.push(e.clone());
            false
        } else {
            true
        }
    });
}

/// La rueda reenviada, dentro del cuadro de la ventana.
fn inject_wheel(ui: &Ui) {
    let evs = std::mem::take(&mut *WHEEL.lock());
    if evs.is_empty() {
        return;
    }
    let page = ui.ctx().content_rect().size();
    ui.ctx().input_mut(|i| {
        for e in evs {
            if let egui::Event::MouseWheel { unit, delta, .. } = &e {
                i.smooth_scroll_delta += match unit {
                    egui::MouseWheelUnit::Point => *delta,
                    egui::MouseWheelUnit::Line => *delta * 40.0,
                    egui::MouseWheelUnit::Page => *delta * page,
                };
            }
            i.events.push(e);
        }
    });
}

impl QuickLook {
    /// Qué archivo mostrar: con varios seleccionados, el que se está recorriendo (empieza por
    /// el enfocado y vuelve a él si la selección cambia); si no, None (el enfocado).
    /// La app lo pregunta antes de armar `Model.item`.
    pub fn pick(&mut self, focused: Option<&str>, group: &[String]) -> Option<String> {
        if group.len() < 2 {
            self.group_sig = 0;
            self.group_len = 0;
            return None;
        }
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        group.hash(&mut hs);
        let sig = hs.finish() | 1;
        if sig != self.group_sig {
            self.group_sig = sig;
            self.index = focused.and_then(|f| group.iter().position(|p| p == f)).unwrap_or(0);
        }
        self.group_len = group.len();
        self.index = self.index.min(group.len() - 1);
        Some(group[self.index].clone())
    }

    /// ¿El visor de Windows es de la ventana? (la app no lo toca mientras tanto)
    pub fn holds_host(&self) -> bool {
        self.host_in || self.closing.is_some()
    }

    /// Dibuja la ventana (o la cierra en dos tiempos). Una vez por cuadro; no hace nada si
    /// está cerrada.
    pub fn show(&mut self, ctx: &egui::Context, env: Env, model: Model) -> Vec<Event> {
        let mut ev = Vec::new();
        let mut env = env;
        let inline = super::capture::inline_windows();
        let now = ctx.input(|i| i.time);
        if !self.open && self.closing.is_none() {
            return ev;
        }
        if let Some(c) = self.closing {
            if c.is_nan() {
                self.begin_close(&mut env, &mut ev);
                self.closing = Some(now);
            }
            let t0 = self.closing.unwrap_or(now);
            let home = env.host.is_none_or(|h| h.is_home()) && env.player.is_none_or(|p| p.is_closed());
            if inline || !self.declared || home || now - t0 > CLOSE_WAIT {
                self.closing = None;
                self.declared = false;
                self.adopted = false;
                self.hidden = false;
                return ev;
            }
            // Sigue declarada (oculta) hasta que el visor y el video salgan de ella.
            let t = env.t;
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(TITLE), self.builder(ctx), |ui, _| {
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.sidebar)).show(ui, |_| {});
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return ev;
        }

        // Abrir: el lugar se decide una vez y queda fijo mientras viva la ventana.
        if !self.declared {
            self.declared = true;
            self.adopted = false;
            self.opened_at = now;
            self.opened_rect = qlwin::placement(env.cfg.rect, env.main);
        }
        if self.hidden {
            qlwin::show_again();
            self.hidden = false;
        }

        // Cambio de archivo: encajada, página 1, sonido nuevo.
        let path = model.item.map_or(String::new(), |i| i.path.clone());
        if path != self.shown {
            fsx::pdf::forget(&self.shown_src);
            self.shown_src = model.item.map_or(String::new(), |i| i.source().to_string());
            qlview::reset(&mut self.view);
            if let Some(pl) = env.player {
                qlmedia::stop(&mut self.media, pl);
            }
            self.shown = path;
            self.shown_at = now;
        }
        self.current = model.item.map(copy_item);
        self.is_media = model.item.is_some_and(|i| !i.entry.is_dir && qlmedia::kind(&i.entry.ext).is_some()) && env.player.is_some();
        if !self.is_media {
            self.media_keys.clear();
        }

        let before = *env.cfg;
        if inline {
            self.show_inline(ctx, &mut env, &model, &mut ev);
        } else {
            let t = env.t;
            let builder = self.builder(ctx);
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(TITLE), builder, |ui, _| {
                inject_wheel(ui);
                let vctx = ui.ctx().clone();
                if vctx.input(|i| i.viewport().close_requested()) {
                    vctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    ev.push(Event::Close);
                }
                egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.sidebar)).show(ui, |ui| {
                    let rect = ui.max_rect();
                    self.ui(ui, rect, &mut env, &model, true, &mut ev);
                });
                if *env.cfg != before {
                    self.cfg_dirty = true;
                }
                // Se guarda al soltar, no en cada cuadro de un arrastre.
                if self.cfg_dirty && !vctx.input(|i| i.pointer.any_down()) {
                    self.cfg_dirty = false;
                    ev.push(Event::SaveCfg);
                }
            });
            if !self.adopted {
                self.adopted = qlwin::adopt(TITLE, env.main, self.opened_rect, env.t);
                if !self.adopted {
                    crate::diag::log("vista rápida: no se encontró su ventana");
                }
                // El próximo cuadro la muestra (ya sin barra de título).
                ctx.request_repaint();
            }
            qlwin::set_theme(env.t);
            for e in qlwin::take_events() {
                match e {
                    qlwin::WinEvent::ToggleMax => qlwin::toggle_max(),
                    qlwin::WinEvent::Moved => self.remember(env.cfg, &mut ev),
                }
            }
            if std::mem::take(&mut self.toggle_max) {
                qlwin::toggle_max();
            }
        }
        if ev.contains(&Event::Close) {
            self.close();
        }
        ev
    }

    /// El `ViewportBuilder`: siempre el mismo mientras viva la ventana (si cambia, egui la
    /// mueve o la recrea). Solo `visible` pasa de false a true, una vez, tras `adopt`.
    fn builder(&self, ctx: &egui::Context) -> egui::ViewportBuilder {
        // Aproximado (escala de la principal): `adopt` pone el rectángulo exacto en px.
        let ppp = ctx.native_pixels_per_point().unwrap_or(1.0).max(0.5);
        let r = self.opened_rect;
        egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_decorations(true)
            .with_resizable(true)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_taskbar(false)
            .with_active(false)
            .with_drag_and_drop(false)
            .with_visible(self.adopted)
            .with_position(pos2(r[0] as f32 / ppp, r[1] as f32 / ppp))
            .with_inner_size(vec2(r[2] as f32 / ppp, r[3] as f32 / ppp))
    }

    /// Primer tiempo del cierre: corta el sonido, devuelve el visor, recuerda el lugar y
    /// oculta la ventana (desaparece al instante; se destruye cuando el visor salió).
    fn begin_close(&mut self, env: &mut Env, ev: &mut Vec<Event>) {
        if let Some(pl) = env.player {
            qlmedia::stop(&mut self.media, pl);
            if let Some(h) = qlwin::hwnd().filter(|_| self.video_in) {
                pl.place_video(h, None);
            }
        }
        if self.host_in {
            if let Some(h) = env.host {
                h.home();
            }
            self.host_in = false;
        }
        self.video_in = false;
        self.native = false;
        qlview::reset(&mut self.view);
        fsx::pdf::forget(&self.shown_src);
        self.shown.clear();
        self.shown_src.clear();
        self.current = None;
        self.media_keys.clear();
        self.toggle_max = false;
        if self.adopted {
            self.remember(env.cfg, ev);
            if !self.hidden {
                qlwin::hide();
                self.hidden = true;
            }
        }
        if std::mem::take(&mut self.cfg_dirty) && !ev.contains(&Event::SaveCfg) {
            ev.push(Event::SaveCfg);
        }
    }

    /// Guarda el lugar (sin maximizar) si cambió.
    fn remember(&mut self, cfg: &mut QuickLookCfg, ev: &mut Vec<Event>) {
        if let Some(r) = qlwin::normal_rect().filter(|r| r[2] > 0 && r[3] > 0) {
            if cfg.rect != Some(r) {
                cfg.rect = Some(r);
                if !ev.contains(&Event::SaveCfg) {
                    ev.push(Event::SaveCfg);
                }
            }
        }
    }

    /// Capturando: la ventana dentro de la principal, con un marco que imita al de Windows
    /// (la captura no ve otras ventanas). En `RAYO_CAPTURA_QL="x,y,ancho,alto"` (puntos) o,
    /// si no se dice, abajo a la derecha, pisando la lista.
    fn show_inline(&mut self, ctx: &egui::Context, env: &mut Env, model: &Model, ev: &mut Vec<Event>) {
        let screen = ctx.content_rect();
        let rect = super::capture::ql_rect().unwrap_or_else(|| {
            let size = vec2(QL_DEFAULT.x.min(screen.width() * 0.6), QL_DEFAULT.y.min(screen.height() * 0.7));
            Rect::from_min_size(screen.max - size - vec2(sp::XL, h::HEAD + sp::XL), size)
        });
        let t = env.t;
        let r8 = egui::CornerRadius::same(8);
        egui::Area::new(Id::new("captura-vista-rapida")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            let p = ui.painter().clone();
            p.add(design::dialog_shadow(t).as_shape(rect, r8));
            p.rect_filled(rect, r8, t.sidebar);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.set_clip_rect(rect);
            self.ui(&mut child, rect, env, model, false, ev);
            p.rect_stroke(rect, r8, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
        });
    }

    /// Encabezado y cuerpo en `rect`. `native`: dentro de la ventana nativa (si no, en la
    /// captura: sin visor de Windows ni video).
    fn ui(&mut self, ui: &mut Ui, rect: Rect, env: &mut Env, model: &Model, native: bool, ev: &mut Vec<Event>) {
        let t = env.t;
        let now = ui.input(|i| i.time);
        // El marco aparece al instante; el contenido funde en FAST.
        ui.multiply_opacity(super::motion::since(ui.ctx(), self.opened_at, design::dur::FAST));
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), h::TOOLBAR));
        let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);

        // ---- cuerpo (antes que el encabezado: el estado del visor va al subtítulo)
        let ppp = ui.ctx().pixels_per_point();
        let hwnd = if native { qlwin::hwnd() } else { None };
        let mut want_host = None;
        let mut video = None;
        match model.item {
            None => {
                let p = ui.painter();
                p.rect_filled(body, 0.0, t.sidebar);
                design::dot_grid(p, t, body);
                p.text(body.center() - vec2(0.0, sp::M), Align2::CENTER_BOTTOM, ic::EYE, design::icon_font(icon::HERO), t.faint);
                design::micro(p, body.center() + vec2(0.0, sp::S), Align2::CENTER_TOP, t!("rapida.sin_seleccion"), t.faint);
            }
            Some(item) => {
                self.view.handler = env.host.and_then(|h| h.status_of(item.source()));
                let mut child = ui.new_child(egui::UiBuilder::new().id_salt("ql-body").max_rect(body));
                child.set_clip_rect(body);
                match (self.is_media, env.player) {
                    (true, Some(pl)) => {
                        for k in self.media_keys.drain(..) {
                            qlmedia::key(&mut self.media, pl, k);
                        }
                        video = qlmedia::body(&mut child, t, body, item, env.tex, pl, env.cfg, &mut self.media);
                    }
                    _ => want_host = qlview::body(&mut child, t, body, item, model.text, env.tex, env.cfg, &mut self.view),
                }
                if std::mem::take(&mut self.view.open) {
                    ev.push(Event::Open { path: item.path.clone(), is_dir: item.entry.is_dir });
                }
                if std::mem::take(&mut self.view.unblock) {
                    ev.push(Event::Unblock(item.path.clone()));
                }
            }
        }
        // Visor de Windows (Office…): pasada la demora, su contenedor se muda a esta ventana.
        match (want_host, env.host, hwnd) {
            (Some((path, r)), Some(host), Some(h)) => {
                let wait = OPEN_DELAY - (now - self.shown_at);
                if wait > 0.0 && !self.host_in {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
                } else {
                    host.show_in(h, &path, px_rect(r, ppp));
                    self.host_in = true;
                    if host.status_of(&path) == Some(super::prevhost::Status::Loading) {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
            }
            _ => {
                if self.host_in {
                    if let Some(host) = env.host {
                        host.home();
                    }
                    self.host_in = false;
                }
            }
        }
        // Video: una ventana hija del reproductor (`place_video` solo actúa si cambió).
        if let (Some(pl), Some(h)) = (env.player, hwnd) {
            match video {
                Some(r) => {
                    pl.place_video(h, Some(px_rect(r, ppp)));
                    self.video_in = true;
                }
                None if self.video_in => {
                    pl.place_video(h, None);
                    self.video_in = false;
                }
                None => {}
            }
        }
        let native_body = self.host_in || self.video_in;

        // ---- encabezado
        let out = self.header(ui, head, env, model, ev);
        self.native = native_body;
        if native {
            // Lo que no es botón arrastra la ventana.
            let mut xs: Vec<(f32, f32)> = out.buttons.iter().map(|r| (r.left(), r.right())).collect();
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut caption = Vec::new();
            let mut x = head.left();
            for (l, r) in xs {
                if l > x {
                    caption.push(Rect::from_min_max(pos2(x, head.top()), pos2(l, head.bottom())));
                }
                x = x.max(r);
            }
            if x < head.right() {
                caption.push(Rect::from_min_max(pos2(x, head.top()), head.max));
            }
            qlwin::set_caption(&caption, ppp);
        }
    }

    /// Encabezado de 44: ‹ › y «2 / 5» (varios), ícono, nombre y micro-etiqueta; a la derecha la
    /// herramienta, «Abrir con ‹programa›», maximizar y cerrar.
    fn header(&mut self, ui: &mut Ui, head: Rect, env: &mut Env, model: &Model, ev: &mut Vec<Event>) -> HeadOut {
        let t = env.t;
        let mut out = HeadOut::default();
        let p = ui.painter().clone();
        p.rect_filled(head, 0.0, t.chrome);
        p.hline(head.x_range(), head.bottom() - stroke::HAIR / 2.0, Stroke::new(stroke::HAIR, t.border));
        // Maximizada, el encabezado no arrastra: el doble clic lo ve egui (y restaura).
        let bg = ui.interact(head, Id::new("ql-head"), Sense::click());
        if bg.double_clicked() && qlwin::maximized() {
            self.toggle_max = true;
        }
        // Con una ventana nativa en el cuerpo (video, visor de Windows), lo que egui dibuje encima
        // del cuerpo queda tapado: los tooltips van a un costado, dentro del encabezado.
        let native = self.native;
        let w = head.width();
        let cy = head.center().y;
        let multi = self.group_len > 1;

        // ---- derecha: cerrar, maximizar, separador
        let mut right = head.right() - sp::S;
        let close = Rect::from_min_size(pos2(right - h::CTL, cy - h::CTL / 2.0), Vec2::splat(h::CTL));
        let r = design::icon_button_at(ui, t, close, Id::new("ql-close"), ic::X, IconBtn::default());
        if r.clicked() {
            ev.push(Event::Close);
        }
        head_tip(r, t, native, t!("comun.cerrar"), "Esc", RectAlign::LEFT);
        out.buttons.push(close);
        right = close.left() - sp::XS;
        let maxed = qlwin::maximized();
        let mx = Rect::from_min_size(pos2(right - h::CTL, cy - h::CTL / 2.0), Vec2::splat(h::CTL));
        let r = design::icon_button_at(ui, t, mx, Id::new("ql-max"), if maxed { ic::ARROWS_IN_SIMPLE } else { ic::ARROWS_OUT_SIMPLE }, IconBtn::default());
        if r.clicked() {
            self.toggle_max = true;
        }
        head_tip(r, t, native, if maxed { t!("rapida.restaurar") } else { t!("rapida.maximizar") }, "F11", RectAlign::LEFT);
        out.buttons.push(mx);
        right = mx.left() - sp::XS;

        // ---- izquierda: varios seleccionados
        let mut x = head.left() + sp::M;
        if multi {
            let n = self.group_len;
            x = head.left() + sp::S;
            for (glyph, d, tip) in [(ic::CARET_LEFT, -1i32, t!("rapida.anterior")), (ic::CARET_RIGHT, 1, t!("rapida.siguiente"))] {
                let rr = Rect::from_min_size(pos2(x, cy - h::CHIP / 2.0), Vec2::splat(h::CHIP));
                let enabled = if d < 0 { self.index > 0 } else { self.index + 1 < n };
                let r = design::icon_button_at(ui, t, rr, Id::new(("ql-nav", d)), glyph, IconBtn::nav(enabled));
                if r.clicked() {
                    self.step(d);
                    ui.ctx().request_repaint();
                }
                if enabled {
                    head_tip(r, t, native, tip, if d < 0 { "←" } else { "→" }, RectAlign::RIGHT);
                }
                out.buttons.push(rr);
                x = rr.right() + sp::XXS;
            }
            let label = format!("{} / {}", fsx::thousands(self.index + 1), fsx::thousands(n));
            let chip = Chip { mono: true, ..Chip::new(&label, Tone::Neutral) };
            let cr = design::chip_paint(&p, t, pos2(x + sp::XS, cy), &chip, false);
            x = cr.right() + sp::M;
        }

        // ---- ícono
        let icon_c = pos2(x + icon::M / 2.0, cy);
        match model.item {
            Some(item) => super::views::file_icon(&p, t, icon_c, icon::M, icon::M, &item.path, item.entry.is_dir, &item.entry.ext, 1.0),
            None => {
                p.text(icon_c, Align2::CENTER_CENTER, ic::EYE, design::icon_font(icon::M), t.dim);
            }
        }
        let title_x = x + icon::M + sp::S;

        // ---- «Abrir con ‹programa›» (o «Abrir», «Abrir con…»)
        let mut tool_right = right;
        if let Some(item) = model.item {
            // Separador entre los botones del contenido y los de la ventana.
            p.vline(right - stroke::HAIR / 2.0, (cy - SEP_H / 2.0)..=(cy + SEP_H / 2.0), Stroke::new(stroke::HAIR, t.border));
            right -= stroke::HAIR + sp::XS;
            let (r, kind) = self.open_with(ui, t, item, right, cy, w < QL_NARROW, native);
            out.buttons.push(r.rect);
            if r.clicked() {
                ev.push(match kind {
                    OpenKind::OpenWith => Event::OpenWith(item.path.clone()),
                    OpenKind::Open => Event::Open { path: item.path.clone(), is_dir: item.entry.is_dir },
                });
            }
            tool_right = r.rect.left() - sp::S;
        }

        // ---- herramienta (una o ninguna): la del contenido; si no, «Mostrar en su carpeta»
        let mut name_right = tool_right;
        if let Some(item) = model.item.filter(|_| !(multi && w < QL_NARROW_MULTI)) {
            let lim = (title_x + NAME_MIN + sp::S).min(tool_right);
            let area = Rect::from_min_max(pos2(lim, head.top()), pos2(tool_right, head.bottom()));
            let mut tui = ui.new_child(egui::UiBuilder::new().id_salt("ql-tool").max_rect(area).layout(egui::Layout::right_to_left(egui::Align::Center)));
            qlview::tool(&mut tui, t, item, env.cfg, &mut self.view);
            let used = tui.min_rect();
            if used.width() > 0.5 {
                out.buttons.push(used);
                name_right = used.left() - sp::S;
            } else if model.from_search && tool_right - h::CTL > lim {
                let rr = Rect::from_min_size(pos2(tool_right - h::CTL, cy - h::CTL / 2.0), Vec2::splat(h::CTL));
                let r = design::icon_button_at(ui, t, rr, Id::new("ql-reveal"), ic::FOLDER_OPEN, IconBtn::default());
                if r.clicked() {
                    ev.push(Event::Reveal(item.path.clone()));
                }
                head_tip(r, t, native, t!("comun.mostrar_su_carpeta"), "", RectAlign::LEFT);
                out.buttons.push(rr);
                name_right = rr.left() - sp::S;
            }
        }

        // ---- nombre y micro-etiqueta
        let name_w = (name_right - title_x).max(NAME_MIN);
        match model.item {
            Some(item) => {
                let sub = qlview::subtitle(item, env.tex, &self.view);
                let name = super::views::elided(&p, &item.entry.name, design::text::strong(), t.text, name_w);
                let gap = 3.0;
                let mut job = design::micro_job(&sub, t.dim);
                job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
                let micro = p.layout_job(job);
                let total = name.size().y + gap + micro.size().y;
                let top = cy - total / 2.0;
                p.galley(pos2(title_x, top), name.clone(), t.text);
                p.galley(pos2(title_x, top + name.size().y + gap), micro, t.dim);
            }
            None => {
                let name = super::views::elided(&p, t!("cmd.quicklook"), design::text::strong(), t.text, name_w);
                p.galley(pos2(title_x, cy - name.size().y / 2.0), name, t.text);
            }
        }
        out
    }

    /// El botón secundario «Abrir con ‹programa›» con el ícono del programa, alineado a la
    /// derecha en `right`. Angosta: solo el ícono (el texto va al tooltip).
    #[allow(clippy::too_many_arguments)]
    fn open_with(&mut self, ui: &mut Ui, t: &Theme, item: &PreviewItem, right: f32, cy: f32, narrow: bool, native: bool) -> (egui::Response, OpenKind) {
        let e = &item.entry;
        let ext = e.ext.to_lowercase();
        // Qué dice, con qué ícono (textura del programa o glifo) y qué hace.
        let (label, glyph, tex, kind): (String, &str, Option<egui::TextureHandle>, OpenKind) = if e.is_dir {
            (t!("comun.abrir").into(), ic::FOLDER_OPEN, None, OpenKind::Open)
        } else if fsx::assoc::runs_itself(&ext) {
            (t!("comun.abrir").into(), ic::ARROW_SQUARE_OUT, None, OpenKind::Open)
        } else {
            match fsx::assoc::default_app(&ext) {
                fsx::assoc::Lookup::Pending => (t!("comun.abrir").into(), "", None, OpenKind::Open),
                fsx::assoc::Lookup::None => (t!("rapida.abrir_con_otro").into(), ic::APP_WINDOW, None, OpenKind::OpenWith),
                fsx::assoc::Lookup::Some(app) => {
                    let tex = self.app_icon_tex(ui.ctx(), &app);
                    (tf!("rapida.abrir_con", app = app.name), if tex.is_none() { ic::APP_WINDOW } else { "" }, tex, OpenKind::Open)
                }
            }
        };
        let has_icon = tex.is_some() || !glyph.is_empty();
        let font = design::text::body();
        let p = ui.painter().clone();
        let rect = if narrow {
            Rect::from_min_size(pos2(right - h::CTL, cy - h::CTL / 2.0), Vec2::splat(h::CTL))
        } else {
            let iw = if has_icon { icon::M + 6.0 } else { 0.0 };
            let pad = if has_icon { OPEN_PAD.0 } else { sp::M };
            let tw = design::text_width(&p, &label, font.clone());
            let w = (pad + iw + tw + OPEN_PAD.1).min(OPEN_WITH_MAX);
            Rect::from_min_size(pos2(right - w, cy - h::CTL / 2.0), vec2(w, h::CTL))
        };
        let r = ui.interact(rect, Id::new("ql-open-with"), Sense::click());
        let fill = if r.is_pointer_button_down_on() {
            t.sel_inactive
        } else if r.hovered() {
            t.hover
        } else {
            Color32::TRANSPARENT
        };
        p.rect_filled(rect, radius::M, fill);
        p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
        let icon_c = if narrow { rect.center() } else { pos2(rect.left() + OPEN_PAD.0 + icon::M / 2.0, cy) };
        if let Some(tx) = &tex {
            let ppp = ui.ctx().pixels_per_point();
            let side = tx.size()[0].max(1) as f32 / ppp;
            let side = side.min(icon::M * 1.5);
            let min = icon_c - Vec2::splat(side / 2.0);
            let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            p.image(tx.id(), Rect::from_min_size(min, Vec2::splat(side)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if !glyph.is_empty() {
            p.text(icon_c, Align2::CENTER_CENTER, glyph, design::icon_font(icon::M), t.dim);
        }
        if !narrow {
            let x = if has_icon { rect.left() + OPEN_PAD.0 + icon::M + 6.0 } else { rect.left() + sp::M };
            let g = super::views::elided(&p, &label, font, t.text, rect.right() - OPEN_PAD.1 - x);
            p.galley(pos2(x, cy - g.size().y / 2.0), g, t.text);
        }
        // Angosta, el nombre del programa va en el tooltip.
        let r = if narrow { head_tip(r, t, native, &label, "", RectAlign::LEFT) } else { r };
        (r, kind)
    }

    /// Ícono de 16 del programa: el de su ejecutable o, si es una app empaquetada, su PNG.
    fn app_icon_tex(&mut self, ctx: &egui::Context, app: &fsx::assoc::DefaultApp) -> Option<egui::TextureHandle> {
        let px = super::shellicons::px_for(ctx, icon::M);
        if let Some(exe) = &app.exe {
            return super::shellicons::get(exe, "exe", px).map(|(t, _)| t);
        }
        let png = app.icon_png.as_ref()?;
        let key = format!("{png}|{px}");
        if self.app_icon.as_ref().is_none_or(|(k, _)| *k != key) {
            let tex = fsx::thumbs::wic_load(png, px).map(|img| {
                let size = [img.w as usize, img.h as usize];
                let ci = if img.premultiplied { egui::ColorImage::from_rgba_premultiplied(size, &img.px) } else { egui::ColorImage::from_rgba_unmultiplied(size, &img.px) };
                ctx.load_texture("ql-app-icon", ci, egui::TextureOptions::LINEAR)
            });
            self.app_icon = Some((key, tex));
        }
        self.app_icon.as_ref().and_then(|(_, t)| t.clone())
    }

    /// Otro archivo de la selección (no da la vuelta).
    fn step(&mut self, d: i32) -> bool {
        let n = self.group_len;
        if n < 2 {
            return false;
        }
        let next = (self.index as i64 + d as i64).clamp(0, n as i64 - 1) as usize;
        self.index = next;
        true
    }

    /// ← → RePág AvPág F11 con la ventana abierta. `grid`: la vista es de cuadrícula (con un
    /// solo archivo, ← → son del explorador). ¿La consumió?
    pub fn key(&mut self, key: Key, grid: bool) -> bool {
        if !self.open {
            return false;
        }
        match key {
            Key::F11 => {
                self.toggle_max = true;
                true
            }
            Key::ArrowLeft | Key::ArrowRight => {
                let d = if key == Key::ArrowLeft { -1 } else { 1 };
                // Primero, otro archivo de la selección (aunque esté en el primero o el último).
                if self.group_len > 1 {
                    return self.step(d);
                }
                if grid {
                    return false;
                }
                if self.is_media {
                    self.media_keys.push(key);
                    return true;
                }
                match &self.current {
                    Some(item) => qlview::key(&mut self.view, item, key),
                    None => false,
                }
            }
            Key::PageUp | Key::PageDown => match &self.current {
                Some(item) if !self.is_media => qlview::key(&mut self.view, item, key),
                _ => false,
            },
            _ => false,
        }
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            // Reabierta mientras se cerraba: la misma ventana vuelve.
            self.closing = None;
        }
    }

    /// Cierra (en dos tiempos: ver `show`). Desaparece ya; el resto, en el próximo cuadro.
    pub fn close(&mut self) {
        if self.open {
            self.open = false;
            self.closing = Some(f64::NAN);
            if self.adopted && !self.hidden {
                qlwin::hide();
                self.hidden = true;
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    Open,
    OpenWith,
}
