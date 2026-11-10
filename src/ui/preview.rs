//! Panel de vista previa y Quick Look (barra espaciadora).

use super::actions::Action;
use super::design::{self, IconBtn, h, icon, radius, sp, stroke};
use super::icons;
use super::theme::Theme;
use crate::fsx::thumbs::{Rgba, ThumbKey, ThumbLoader, is_image_ext};
use crate::fsx::{self, FileEntry};
use egui::{Align2, Color32, ColorImage, CornerRadius, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub struct TexEntry {
    pub tex: Option<TextureHandle>,
    pub orig: Option<(u32, u32)>,
    /// Páginas (si es un PDF).
    pub pages: Option<u32>,
    /// Cuándo llegó (para que aparezca con un fundido); muy atrás si reemplaza a otra del mismo archivo.
    pub at: f64,
    /// Sin transparencia (foto, página): la única que lleva marco.
    pub opaque: bool,
    /// El ícono del tipo que da Windows cuando no tiene la miniatura: un dibujo chico centrado en
    /// un lienzo vacío (un 48 en 256). En una foto no es su miniatura: ver `get_photo`.
    pub icon: bool,
}

/// Cómo es una imagen recibida: (opaca, ícono en un lienzo vacío). Mira una muestra de ~64×64
/// píxeles, así cuesta lo mismo una miniatura que una página de 4096. El ícono que da Windows en
/// lugar de la miniatura es uno de 48 centrado en 256, a veces con un marco semitransparente en
/// el borde: cuenta solo lo bien opaco (α ≥ 128).
fn shape(img: &Rgba) -> (bool, bool) {
    let (w, h) = (img.w as usize, img.h as usize);
    if w == 0 || h == 0 || img.px.len() < w * h * 4 {
        return (false, false);
    }
    let step = (w.max(h) / 64).max(1);
    let mut opaque = true;
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in (step / 2..h).step_by(step) {
        for x in (step / 2..w).step_by(step) {
            let a = img.px[(y * w + x) * 4 + 3];
            opaque &= a == 255;
            if a >= 128 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    // El borde también cuenta para «opaca» (el marco de Windows es semitransparente).
    opaque &= img.px[3] == 255 && img.px[(w * h - 1) * 4 + 3] == 255;
    let icon = !opaque && w.max(h) >= 96 && x1 >= x0 && (x1 - x0 + step) * 2 <= w && (y1 - y0 + step) * 2 <= h;
    (opaque, icon)
}

/// Texturas de miniaturas con desalojo por antigüedad.
pub struct Textures {
    pub loader: Arc<ThumbLoader>,
    map: HashMap<ThumbKey, TexEntry>,
    order: VecDeque<ThumbKey>,
}

const MAX_TEX: usize = 240;

impl Textures {
    pub fn new(loader: Arc<ThumbLoader>) -> Self {
        super::shellicons::init(loader.clone());
        Textures { loader, map: HashMap::new(), order: VecDeque::new() }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.loader.next_frame();
        // Subir texturas tiene su costo: se suben las que entren en ~3 ms por cuadro y el resto en
        // el siguiente, así una carpeta llena de fotos no traba el movimiento.
        let t0 = std::time::Instant::now();
        let budget = std::time::Duration::from_millis(3);
        let now = ctx.input(|i| i.time);
        loop {
            if t0.elapsed() > budget {
                if !self.loader.rx.is_empty() {
                    ctx.request_repaint();
                }
                break;
            }
            let Ok(r) = self.loader.rx.try_recv() else { break };
            if r.key.path.starts_with(crate::fsx::thumbs::ICON_PREFIX) {
                super::shellicons::deliver(ctx, &r.key, r.img);
                continue;
            }
            // Si ya se veía otra versión del mismo archivo, la nueva la reemplaza sin fundido.
            let replaces = self.map.iter().any(|(k, e)| k.path == r.key.path && e.tex.is_some());
            let at = if replaces { f64::MIN } else { now };
            let entry = match r.img {
                Some(img) => {
                    let size = [img.w as usize, img.h as usize];
                    let ci = if img.premultiplied {
                        ColorImage::from_rgba_premultiplied(size, &img.px)
                    } else {
                        ColorImage::from_rgba_unmultiplied(size, &img.px)
                    };
                    let (opaque, icon) = shape(&img);
                    let tex = ctx.load_texture(&r.key.path, ci, TextureOptions::LINEAR);
                    TexEntry { tex: Some(tex), orig: img.orig, pages: img.pages, at, opaque, icon }
                }
                None => TexEntry { tex: None, orig: None, pages: None, at, opaque: false, icon: false },
            };
            self.order.push_back(r.key.clone());
            self.map.insert(r.key, entry);
            while self.order.len() > MAX_TEX {
                if let Some(k) = self.order.pop_front() {
                    self.map.remove(&k);
                }
            }
        }
    }

    /// Devuelve la textura si ya está; si no, la pide (y devuelve una más chica si existe).
    pub fn get(&mut self, path: &str, size: u32) -> Option<&TexEntry> {
        self.get_with(path, size, false)
    }

    /// Lo mismo, urgente (vista previa, vista rápida): no espera detrás de las miniaturas.
    pub fn get_now(&mut self, path: &str, size: u32) -> Option<&TexEntry> {
        self.get_with(path, size, true)
    }

    fn get_with(&mut self, path: &str, size: u32, urgent: bool) -> Option<&TexEntry> {
        let key = ThumbKey { path: path.to_string(), size };
        if !self.map.contains_key(&key) {
            self.loader.request(key.clone(), urgent);
            // Mientras tanto, cualquier versión ya cargada del mismo archivo.
            let alt = self.map.keys().find(|k| k.path == path).cloned()?;
            return self.map.get(&alt);
        }
        self.map.get(&key)
    }

    /// La imagen de una foto (vista previa, vista rápida). Si Windows dio el ícono del tipo en vez
    /// de la miniatura (le pasa al arrancar: durante unos segundos su extractor devuelve el ícono
    /// de 48 en un lienzo de 256), se pide de más de 512, que Rayo decodifica él mismo (ver
    /// `thumbs::load`). Sin esto el ícono quedaba para siempre: lo recibido no se vuelve a pedir.
    pub fn get_photo(&mut self, path: &str, size: u32) -> Option<&TexEntry> {
        let key = ThumbKey { path: path.to_string(), size };
        let icon = size <= 512 && self.map.get(&key).is_some_and(|e| e.icon);
        self.get_with(path, if icon { 640 } else { size }, true)
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

pub const TEXT_EXTS: &[&str] = &[
    "txt", "md", "log", "ini", "cfg", "conf", "toml", "yaml", "yml", "json", "xml", "csv", "tsv", "rs", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "py", "js",
    "mjs", "cjs", "ts", "tsx", "jsx", "go", "rb", "php", "swift", "lua", "dart", "vue", "html", "htm", "css", "scss", "sass", "less", "sql", "sh", "bat", "cmd", "ps1",
    "gitignore", "env", "reg", "srt", "vtt", "properties", "gradle", "lock", "editorconfig", "svg", "zig", "nix", "mk", "makefile", "dockerfile",
];

/// Lee el inicio de un archivo de texto (o None si parece binario).
pub fn read_text_snippet(path: &str, max: usize) -> Option<String> {
    use std::io::Read;
    if fsx::is_online_only(path) {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    if buf.iter().take(2048).any(|&b| b == 0) {
        return None;
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if s.starts_with('\u{feff}') {
        s.remove(0);
    }
    Some(s)
}

impl PreviewItem {
    /// Ruta para leer el contenido (la copia temporal si viene de un comprimido).
    pub fn source(&self) -> &str {
        self.real.as_deref().unwrap_or(&self.path)
    }
}

pub struct PreviewItem {
    pub path: String,
    /// Copia temporal si el archivo está dentro de un comprimido.
    pub real: Option<String>,
    pub entry: FileEntry,
    /// Elementos dentro (si es carpeta y ya se listó).
    pub children: Option<usize>,
}

pub fn wants_text(e: &FileEntry) -> bool {
    !e.is_dir && (TEXT_EXTS.contains(&e.ext.as_str()) || (e.ext.is_empty() && e.size < 256 * 1024))
}

/// Qué se muestra de un archivo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Texto o código (Markdown con formato).
    Text,
    /// Imagen, página de PDF o miniatura de Windows.
    Picture,
    /// Visor de vista previa de Windows (Word, Excel, PowerPoint…).
    Handler,
    /// Solo el ícono (carpetas, archivos solo en la nube, sin vista previa).
    Icon,
}

/// Formatos que se ven mejor con miniatura que con un visor (aunque tengan uno).
const PICTURE_EXTS: &[&str] = &["pdf", "exe", "lnk", "url", "dll", "msi", "ico", "cur", "zip", "7z", "rar"];

pub fn kind(item: &PreviewItem, has_text: bool) -> Kind {
    let e = &item.entry;
    // Una descarga a medias: la flecha (su miniatura sería la de un archivo genérico).
    if e.is_dir || fsx::downloading(&e.name).is_some() {
        return Kind::Icon;
    }
    if has_text {
        return Kind::Text;
    }
    if e.cloud == fsx::Cloud::OnlineOnly {
        return Kind::Icon;
    }
    if is_image_ext(&e.ext) || PICTURE_EXTS.contains(&e.ext.as_str()) {
        return Kind::Picture;
    }
    if super::prevhost::handler_for(&e.ext).is_some() {
        return Kind::Handler;
    }
    Kind::Picture
}

/// Texto plano, código con colores o Markdown con formato. `large`: Vista rápida (`body` /
/// `mono_l`); si no, el panel (`small` / `mono`). El texto plano y el Markdown se ajustan al ancho;
/// el código no (ver `scrolls_sideways`).
pub fn text_view(ui: &mut Ui, t: &Theme, text: &str, ext: &str, large: bool) {
    // Un texto en chino o japonés en una carpeta con nombres ASCII: que se cargue la fuente.
    super::theme::want_cjk(text);
    if matches!(ext, "md" | "markdown" | "mdx") {
        super::richtext::markdown(ui, t, text, large);
    } else if super::richtext::is_code(ext) {
        super::richtext::code(ui, t, text, large);
    } else {
        let font = if large { design::text::mono_l() } else { design::text::mono() };
        ui.add(egui::Label::new(egui::RichText::new(text).font(font).color(t.text)).wrap());
    }
}

/// ¿Se desplaza también a lo ancho? Solo el código (sus líneas no se parten).
fn scrolls_sideways(ext: &str) -> bool {
    super::richtext::is_code(ext)
}

/// El texto de `text_view` en `rect`, con su barra (solo vertical, salvo el código). Devuelve el
/// alto del contenido.
fn text_scroll(ui: &mut Ui, t: &Theme, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, text: &str, ext: &str, large: bool) -> f32 {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let area = if scrolls_sideways(ext) { egui::ScrollArea::both() } else { egui::ScrollArea::vertical() };
    let out = area.id_salt(id).auto_shrink([false, false]).show(&mut child, |ui| {
        // La barra del pozo no tapa la última letra de cada línea.
        ui.set_max_width(rect.width() - sp::S);
        text_view(ui, t, text, ext, large);
    });
    out.content_size.y
}

/// Ícono de contenido (no de interfaz) del panel y de Vista rápida.
const PANE_ICON: f32 = 84.0;
const QL_ICON: f32 = 120.0;

/// El ícono grande del archivo, relleno y a color, sin marco. Una descarga en curso: la flecha.
fn big_icon(p: &egui::Painter, t: &Theme, center: egui::Pos2, e: &FileEntry, size: f32) {
    let (glyph, color) = if !e.is_dir && fsx::downloading(&e.name).is_some() { (ic::DOWNLOAD_SIMPLE, t.accent) } else { icons::for_entry(t, e.is_dir, &e.ext) };
    p.text(center, Align2::CENTER_CENTER, glyph, design::icon_fill(size), color);
}

fn fit(tex_size: egui::Vec2, area: Rect) -> Rect {
    let s = (area.width() / tex_size.x).min(area.height() / tex_size.y).min(1.0 * 4.0);
    let size = tex_size * s;
    Rect::from_center_size(area.center(), size)
}

const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// Imagen con radio `S` y borde fino (foto o página: en el pozo del panel y en Vista rápida).
fn framed_image(p: &egui::Painter, t: &Theme, tex: egui::TextureId, r: Rect) {
    p.add(egui::epaint::RectShape::filled(r, radius::S, Color32::WHITE).with_texture(tex, UV));
    p.rect_stroke(r, radius::S, Stroke::new(stroke::HAIR, t.border), StrokeKind::Outside);
}

/// Una imagen recibida: con marco si es opaca (foto, página); si tiene transparencia (ícono,
/// PNG recortado), suelta sobre el fondo.
fn picture(p: &egui::Painter, t: &Theme, te: &TexEntry, tx: &TextureHandle, r: Rect) {
    if te.opaque {
        framed_image(p, t, tx.id(), r);
    } else {
        p.image(tx.id(), r, UV, Color32::WHITE);
    }
}

/// Hairline `border` a lo ancho (en vez de `ui.separator()`).
fn hairline(ui: &mut Ui, t: &Theme) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), stroke::HAIR), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(stroke::HAIR, t.border));
}

/// Un dato editable, con el rótulo en la misma columna que los fijos. Con hover, capa gris; al
/// hacer clic, un campo. Devuelve el texto nuevo al confirmar (Enter o al salir del campo) si cambió.
fn edit_row(ui: &mut Ui, t: &Theme, path: &str, field: &str, label: &str, value: &str) -> Option<String> {
    let id = egui::Id::new(("meta-edit", path, field));
    let te_id = id.with("te");
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::click());
    let p = ui.painter().clone();
    p.text(rect.left_center(), Align2::LEFT_CENTER, label, design::text::small(), t.dim);
    let val_left = rect.left() + design::text_width(&p, label, design::text::small()) + sp::M;
    // Mientras se edita, el texto vive en la memoria temporal del campo.
    let mut editing: Option<String> = ui.data(|d| d.get_temp(id));
    let started = editing.is_none() && resp.clicked();
    if started {
        editing = Some(value.to_string());
        ui.memory_mut(|m| m.request_focus(te_id));
    }
    let mut out = None;
    match editing {
        None => {
            // El pill sobresale 4 px a la derecha para que el valor quede alineado con los fijos.
            let pill = Rect::from_min_max(pos2(val_left - sp::XS, rect.top() + 1.0), pos2(rect.right() + sp::XS, rect.bottom() - 1.0));
            let hov = design::hover_amount(ui.ctx(), id, resp.hovered());
            design::state_fill(&p, t, pill, radius::S, hov, resp.is_pointer_button_down_on(), design::Sel::None);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            // Vacío: una raya apagada; «Agregar…» solo con el cursor encima.
            let (txt, font, color) = match (value.is_empty(), resp.hovered()) {
                (false, _) => (value, design::text::mono(), t.text),
                (true, true) => (t!("previa.agregar"), design::text::small(), t.faint),
                (true, false) => ("—", design::text::mono(), t.disabled),
            };
            let g = super::views::elided(&p, txt, font, color, rect.right() - val_left);
            p.galley(pos2(rect.right() - g.size().x, rect.center().y - g.size().y / 2.0), g, color);
        }
        Some(mut buf) => {
            let field_rect = Rect::from_min_max(pos2(val_left - sp::XS, rect.top() + 1.0), pos2(rect.right() + sp::XS, rect.bottom() - 1.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field_rect));
            let frame = egui::Frame::NONE.fill(t.sidebar).corner_radius(radius::S).inner_margin(egui::Margin::symmetric(sp::XS as i8, sp::XXS as i8));
            let te = egui::TextEdit::singleline(&mut buf)
                .id(te_id)
                .font(design::text::mono())
                .text_color(t.text)
                .hint_text(egui::RichText::new(t!("previa.agregar")).color(t.faint))
                .desired_width(field_rect.width() - 2.0 * sp::XS)
                .frame(frame);
            let r = child.add(te);
            if r.has_focus() {
                design::focus_ring(&p, t, r.rect, radius::S);
                ui.data_mut(|d| d.insert_temp(id, buf.clone()));
            }
            if r.lost_focus() || (!r.has_focus() && !started) {
                ui.data_mut(|d| d.remove::<String>(id));
                if r.lost_focus() && buf.trim() != value {
                    out = Some(buf.trim().to_string());
                }
            } else if started {
                ui.data_mut(|d| d.insert_temp(id, buf.clone()));
            }
        }
    }
    out
}

/// Calificación con estrellas (como en las propiedades de Windows). Devuelve la nueva al hacer clic.
fn rating_row(ui: &mut Ui, t: &Theme, rating: u32) -> Option<u32> {
    let stars = match rating {
        0 => 0,
        1..=12 => 1,
        13..=37 => 2,
        38..=62 => 3,
        63..=87 => 4,
        _ => 5,
    };
    let mut out = None;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    let p = ui.painter().clone();
    p.text(rect.left_center(), Align2::LEFT_CENTER, t!("previa.calificacion"), design::text::small(), t.dim);
    // Las cinco estrellas a la derecha, como los valores.
    let cell = icon::M + sp::XS;
    for k in 1..=5u32 {
        let r = Rect::from_min_size(pos2(rect.right() - (6 - k) as f32 * cell, rect.top()), vec2(cell, h::HEAD));
        let resp = ui.interact(r, ui.id().with(("meta-star", k)), Sense::click());
        let on = k <= stars;
        let (glyph, font) = if on { (egui_phosphor::fill::STAR, design::icon_fill(icon::M)) } else { (egui_phosphor::regular::STAR, design::icon_font(icon::M)) };
        let c = if on { t.warn } else if resp.hovered() { t.dim } else { t.faint };
        p.text(pos2(r.right() - icon::M / 2.0, r.center().y), Align2::CENTER_CENTER, glyph, font, c);
        if resp.clicked() {
            // Un clic en la estrella que ya era la última la quita (0 estrellas).
            out = Some(if k == stars { 0 } else { [1, 25, 50, 75, 99][k as usize - 1] });
        }
    }
    out
}

fn field_label(f: fsx::props::Field) -> &'static str {
    use fsx::props::Field;
    match f {
        Field::Camera => t!("previa.camara"),
        Field::Lens => t!("previa.lente"),
        Field::Exposure => t!("previa.exposicion"),
        Field::Taken => t!("previa.tomada"),
        Field::Duration => t!("previa.duracion"),
        Field::Resolution => t!("previa.resolucion"),
        Field::FrameRate => t!("previa.fotogramas"),
        Field::Bitrate => t!("previa.calidad"),
        Field::Artist => t!("previa.artista"),
        Field::Album => t!("previa.album"),
    }
}

/// Rótulo a la izquierda, valor mono a la derecha (si no entra, baja a la línea siguiente).
fn info_row(ui: &mut Ui, t: &Theme, k: &str, v: &str) -> egui::Response {
    design::meta_row(ui, t, k, v)
}

/// Panel de vista previa. Si el archivo usa un visor de Windows, devuelve dónde ponerlo.
#[allow(clippy::too_many_arguments)]
pub fn preview_pane(
    ui: &mut Ui,
    t: &Theme,
    item: Option<&PreviewItem>,
    multi: Option<(usize, u64)>,
    tex: &mut Textures,
    text: Option<&str>,
    // Ya no se usa (la antigüedad repetía «Modificado»); queda por la firma.
    _now: u32,
    host: Option<&super::prevhost::PreviewHost>,
    actions: &mut Vec<Action>,
) -> Option<(String, Rect)> {
    let mut want = None;
    if let Some((n, size)) = multi {
        // Varios: la trama con el ícono, cuántos y cuánto pesan (como un estado vacío).
        let area = ui.available_rect_before_wrap();
        let p = ui.painter();
        let c = pos2(area.center().x, area.top() + sp::XXL + 32.0);
        let patch = Rect::from_center_size(c, vec2(160.0, 64.0));
        design::dot_grid(p, t, patch);
        p.text(c, Align2::CENTER_CENTER, ic::FILES, design::icon_font(icon::HERO), t.faint);
        let title = tf!("previa.elementos_seleccionados", n = n);
        p.text(pos2(c.x, patch.bottom() + sp::S), Align2::CENTER_TOP, title, design::text::strong(), t.dim);
        p.text(pos2(c.x, patch.bottom() + sp::S + 22.0), Align2::CENTER_TOP, fsx::human_size(size), design::text::mono(), t.faint);
        return None;
    }
    let Some(item) = item else {
        // Vacío: en un panel angosto queda solo la micro-etiqueta.
        let area = ui.available_rect_before_wrap();
        design::empty_state(ui.painter(), t, area, ic::EYE, t!("previa.selecciona_archivo"), "");
        return None;
    };
    let e = &item.entry;
    let mut kind = kind(item, text.is_some());
    // Si el visor de Windows falló, se usa la miniatura.
    if kind == Kind::Handler && host.and_then(|h| h.status_of(item.source())) == Some(super::prevhost::Status::Failed) {
        kind = Kind::Picture;
    }
    let pane_h = ui.available_height();
    let clip = ui.clip_rect();
    // Lo que no es el pozo (aire, nombre, botones, metadatos), medido en el cuadro anterior: el
    // pozo alto se queda con el resto, así la última fila no se corta ni aparece otra barra.
    let rest_id = egui::Id::new(("pv-below", &item.path));
    let rest = ui.data(|d| d.get_temp::<f32>(rest_id));
    let mut well_h = 0.0;
    let out = egui::ScrollArea::vertical().id_salt("preview").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(sp::M);
        let w = ui.available_width() - 2.0 * sp::M;
        // El texto ocupa lo que mide (lo medido en el cuadro anterior), con un tope: una nota de
        // una línea no reserva medio panel.
        let tall = (pane_h - rest.unwrap_or(248.0)).clamp(220.0, 900.0);
        let text_h_id = egui::Id::new(("pv-text-h", &item.path));
        let h = match kind {
            Kind::Text => ui.data(|d| d.get_temp::<f32>(text_h_id)).map_or(tall, |c| (c + 2.0 * sp::M).clamp(96.0, tall)),
            Kind::Handler => tall,
            _ => (w * 0.8).clamp(120.0, 320.0),
        };
        well_h = h;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let area = Rect::from_center_size(rect.center(), vec2(w, h));
        let p = ui.painter().clone();
        let mut dims = None;
        let mut pages = None;
        let mut drew = false;
        // El pozo: hundido, radio L. Con imagen o ícono lleva la trama de puntos alrededor.
        p.rect_filled(area, radius::L, t.sidebar);
        if matches!(kind, Kind::Picture | Kind::Icon) {
            design::dot_grid(&p, t, area.shrink(sp::XS));
        }
        match kind {
            Kind::Text => {
                let content = text_scroll(ui, t, area.shrink(sp::M), ("pv-text", &item.path), text.unwrap_or(""), &e.ext, false);
                if ui.data(|d| d.get_temp::<f32>(text_h_id)).is_none_or(|c| (c - content).abs() > 0.5) {
                    ui.data_mut(|d| d.insert_temp(text_h_id, content));
                    ui.ctx().request_repaint();
                }
                drew = true;
            }
            Kind::Handler => {
                design::micro(&p, area.center(), Align2::CENTER_CENTER, t!("previa.cargando_vista_previa"), t.faint);
                let visible = area.intersect(clip);
                if visible.height() > 40.0 {
                    want = Some((item.source().to_string(), visible));
                }
                drew = true;
            }
            Kind::Picture => {
                let ppp = ui.ctx().pixels_per_point();
                let size = ((w.max(h) * ppp).min(768.0) as u32).next_multiple_of(128);
                let photo = is_image_ext(&e.ext);
                let got = if photo { tex.get_photo(item.source(), size) } else { tex.get_now(item.source(), size) };
                if let Some(te) = got {
                    dims = te.orig;
                    pages = te.pages;
                    // Una foto que llegó como el ícono del tipo no es su miniatura: el ícono
                    // grande sin marco mientras llega la de verdad.
                    if let Some(tx) = te.tex.as_ref().filter(|_| !(te.icon && photo)) {
                        // Con aire dentro del pozo, para que la trama se vea alrededor.
                        let air = if w >= 240.0 { sp::XL } else { sp::L };
                        picture(&p, t, te, tx, fit(tx.size_vec2() / ppp, area.shrink(air)));
                        drew = true;
                    }
                }
            }
            Kind::Icon => {}
        }
        if !drew {
            big_icon(&p, t, area.center(), e, PANE_ICON);
        }
        if super::views::double_clicked(&resp) {
            actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
        }
        ui.add_space(sp::M);
        // El bloque de texto ocupa las mismas columnas que el pozo (bordes a plomo).
        let block = Rect::from_x_y_ranges(area.x_range(), ui.cursor().top()..=ui.max_rect().bottom());
        ui.scope_builder(egui::UiBuilder::new().max_rect(block).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            ui.set_width(w);
            // Una descarga en curso se nombra por el archivo que va a ser.
            let download = if e.is_dir { None } else { fsx::downloading(&e.name) };
            // Nombre: `title`, hasta dos líneas.
            let mut job = egui::text::LayoutJob::single_section(download.unwrap_or(&e.name).to_string(), egui::text::TextFormat::simple(design::text::title(), t.text));
            job.wrap = egui::text::TextWrapping { max_width: w, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
            let g = ui.painter().layout_job(job);
            let (nr, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
            ui.painter().galley(nr.min, g, t.text);
            // Subtítulo: la extensión sola («JPG · 12.4 KB»); «Carpeta» o «Archivo» si no hay;
            // «Descarga» mientras se baja.
            let mut sub = if download.is_some() {
                t!("lista.descarga").to_string()
            } else if e.is_dir || e.ext.is_empty() {
                fsx::kind_label(e)
            } else {
                e.ext.to_uppercase()
            };
            if let Some(s) = e.display_size() {
                sub.push_str(&format!(" · {}", fsx::human_size(s)));
            }
            ui.add_space(sp::XXS);
            let _ = design::micro_label(ui, &sub, t.dim);
            ui.add_space(sp::M);
            ui.horizontal(|ui| {
                if design::button(ui, t, design::Kind::Secondary, ic::ARROW_SQUARE_OUT, t!("comun.abrir"), true).clicked() {
                    actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
                }
                let ql = design::button(ui, t, design::Kind::Ghost, ic::EYE, t!("previa.vista_rapida"), true);
                if design::tip(ql, t, t!("previa.barra_espaciadora")).clicked() {
                    actions.push(Action::QuickLook);
                }
            });
            // Estados del archivo: líneas de estado bajo las acciones.
            if e.cloud == fsx::Cloud::OnlineOnly {
                ui.add_space(sp::S);
                design::status_line(ui, t, design::Tone::Neutral, ic::CLOUD, t!("previa.solo_nube_abrelo_descargarlo"));
            }
            if download.is_some() {
                ui.add_space(sp::S);
                design::status_line(ui, t, design::Tone::Accent, ic::DOWNLOAD_SIMPLE, t!("previa.descarga_en_curso"));
            }
            let props = if e.is_dir { None } else { fsx::props::info(&item.path) };
            if props.as_ref().is_some_and(|p| p.blocked) {
                // Descargado de Internet: Windows no deja ver PDF ni Office en la vista previa.
                ui.add_space(sp::M);
                egui::Frame::new().fill(t.warn_soft).corner_radius(radius::M).inner_margin(sp::S).show(ui, |ui| {
                    ui.set_width(w - 2.0 * sp::S);
                    design::status_line(ui, t, design::Tone::Warn, ic::SHIELD_WARNING, t!("previa.descargado_internet"));
                    ui.add_space(sp::XS);
                    let b = design::button(ui, t, design::Kind::Secondary, ic::LOCK_SIMPLE_OPEN, t!("previa.desbloquear"), true);
                    if design::tip(b, t, t!("previa.desbloquear_ayuda")).clicked() {
                        actions.push(Action::Unblock(vec![item.path.clone()]));
                    }
                });
            }
            ui.add_space(sp::M);
            hairline(ui, t);
            ui.add_space(sp::XS);
            // Metadatos: filas `HEAD` pegadas, una sola columna de rótulos. Primero lo que es del
            // contenido, después tamaño y fechas, y al final dónde está.
            ui.spacing_mut().item_spacing.y = 0.0;
            if let Some(n) = pages {
                info_row(ui, t, t!("previa.paginas"), &n.to_string());
            } else if let Some((dw, dh)) = dims {
                info_row(ui, t, t!("previa.dimensiones"), &format!("{dw} × {dh}"));
            }
            for (f, v) in props.iter().flat_map(|p| p.rows.iter()) {
                info_row(ui, t, field_label(*f), v);
            }
            if let Some(n) = item.children {
                info_row(ui, t, t!("previa.contenido"), &tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
            }
            // Tamaño legible; los bytes exactos, en el tooltip.
            if let Some(s) = e.display_size() {
                let exact = tf!("previa.tamano_bytes", tamano = fsx::human_size(s), bytes = fsx::thousands(s as usize));
                design::tip(info_row(ui, t, t!("comun.tamano"), &fsx::human_size(s)), t, &exact);
            } else if e.is_dir && item.real.is_none() && !fsx::archive::is_inside(&item.path) {
                // Fuera del índice (red, USB): se calcula en segundo plano y se ve avanzar.
                let d = fsx::dirsize::get(&item.path);
                let exact = tf!("previa.tamano_bytes", tamano = fsx::human_size(d.bytes), bytes = fsx::thousands(d.bytes as usize));
                let mut v = fsx::human_size(d.bytes);
                if !d.done {
                    v.push_str(&format!(" {}", t!("previa.calculando")));
                }
                design::tip(info_row(ui, t, t!("comun.tamano"), &v), t, &exact);
            }
            info_row(ui, t, t!("comun.modificado"), &fsx::fmt_date(e.mtime));
            if e.ctime != 0 {
                info_row(ui, t, t!("previa.creado"), &fsx::fmt_date(e.ctime));
            }
            if let Some((_, _, tip)) = super::views::cloud_glyph(t, e.cloud) {
                info_row(ui, t, t!("previa.nube"), tip);
            }
            let _ = design::meta_row_path(ui, t, t!("previa.ubicacion"), fsx::parent_of(&item.path).as_deref().unwrap_or(""));
            // Título, comentarios y calificación: se editan aquí y se guardan en el archivo.
            if let Some(info) = props.as_ref().filter(|i| i.editable && item.real.is_none()) {
                ui.add_space(sp::S);
                if let Some(v) = edit_row(ui, t, &item.path, "titulo", t!("previa.titulo"), &info.title) {
                    actions.push(Action::SetProperty(item.path.clone(), "System.Title", fsx::props::Value::Text(v)));
                }
                if let Some(v) = edit_row(ui, t, &item.path, "comentario", t!("previa.comentarios"), &info.comment) {
                    actions.push(Action::SetProperty(item.path.clone(), "System.Comment", fsx::props::Value::Text(v)));
                }
                if let Some(r) = rating_row(ui, t, info.rating) {
                    actions.push(Action::SetProperty(item.path.clone(), "System.Rating", fsx::props::Value::Number(r)));
                }
            }
        });
        ui.add_space(sp::L);
    });
    let measured = (out.content_size.y - well_h).max(0.0);
    if rest.is_none_or(|r| (r - measured).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(rest_id, measured));
        ui.ctx().request_repaint();
    }
    want
}

/// Ventana de Quick Look sobre todo lo demás. Devuelve si sigue abierta y, si el
/// archivo usa un visor de Windows, dónde ponerlo. `page`: página del PDF.
#[allow(clippy::too_many_arguments)]
pub fn quicklook(
    ctx: &egui::Context,
    t: &Theme,
    item: &PreviewItem,
    tex: &mut Textures,
    text: Option<&str>,
    page: &mut u32,
    host: Option<&super::prevhost::PreviewHost>,
    actions: &mut Vec<Action>,
) -> (bool, Option<(String, Rect)>) {
    let screen = ctx.content_rect();
    let mut open = true;
    let mut want = None;
    let mut kind = kind(item, text.is_some());
    if kind == Kind::Handler && host.and_then(|h| h.status_of(item.source())) == Some(super::prevhost::Status::Failed) {
        kind = Kind::Picture;
    }
    egui::Area::new(egui::Id::new("quicklook")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (full, bg) = ui.allocate_exact_size(screen.size(), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(full, CornerRadius::ZERO, t.overlay);
        let card = Rect::from_center_size(full.center(), vec2((full.width() * 0.82).min(1400.0), (full.height() * 0.86).min(1000.0)));
        let card_resp = ui.interact(card, egui::Id::new("ql-card"), Sense::click());
        // Tarjeta de diálogo: `raised`, radio XL, sombra de diálogo.
        p.add(design::dialog_shadow(t).as_shape(card, radius::XL));
        p.rect_filled(card, radius::XL, t.raised);
        p.rect_stroke(card, radius::XL, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
        let e = &item.entry;
        // Encabezado: ícono, nombre, tipo y tamaño; a la derecha «Abrir» y cerrar.
        let head = Rect::from_min_size(card.min, vec2(card.width(), h::TOOLBAR));
        // El ícono real del archivo, el mismo de la lista.
        super::views::file_icon(&p, t, pos2(head.left() + sp::L + icon::M / 2.0, head.center().y), icon::M, icon::M, &item.path, e.is_dir, &e.ext, 1.0);
        let mut sub = fsx::kind_label(e);
        if let Some(s) = e.display_size() {
            sub.push_str(&format!(" · {}", fsx::human_size(s)));
        }
        let title_x = head.left() + sp::L + icon::M + sp::S;
        let title = super::views::elided(&p, &e.name, design::text::title(), t.text, head.width() - 360.0);
        let tw = title.size().x;
        p.galley(pos2(title_x, head.center().y - title.size().y / 2.0), title, t.text);
        design::micro(&p, pos2(title_x + tw + sp::M, head.center().y), Align2::LEFT_CENTER, &sub, t.dim);
        let close = Rect::from_center_size(pos2(head.right() - sp::S - h::CTL / 2.0, head.center().y), vec2(h::CTL, h::CTL));
        let cresp = design::icon_button_at(ui, t, close, egui::Id::new("ql-close"), ic::X, IconBtn::default());
        let actions_rect = Rect::from_min_max(head.min, pos2(close.left() - sp::S, head.bottom()));
        let mut bui = ui.new_child(egui::UiBuilder::new().id_salt("ql-open").max_rect(actions_rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if design::button(&mut bui, t, design::Kind::Primary, "", t!("comun.abrir"), true).clicked() {
            actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
            open = false;
        }
        if cresp.clicked() {
            open = false;
        }
        p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(stroke::HAIR, t.border));
        let body = Rect::from_min_max(pos2(card.left() + sp::L, head.bottom() + sp::L), card.max - vec2(sp::L, sp::L));
        let mut drew = false;
        if kind == Kind::Picture {
            let ppp = ctx.pixels_per_point();
            let side = ((body.width().max(body.height()) * ppp) as u32).clamp(256, 4096).next_multiple_of(256);
            let pdf = e.ext == "pdf";
            let size = if is_image_ext(&e.ext) || pdf { side } else { side.min(1024) };
            let key = if pdf { fsx::pdf::page_key(item.source(), *page) } else { item.source().to_string() };
            let mut footer = None;
            let photo = is_image_ext(&e.ext);
            let got = if photo { tex.get_photo(&key, size) } else { tex.get_now(&key, size) };
            if let Some(te) = got {
                // Foto que llegó como el ícono del tipo: el ícono grande mientras llega la de verdad.
                if let Some(tx) = te.tex.as_ref().filter(|_| !(te.icon && photo)) {
                    // Foto o página con radio `S` y borde fino: en claro, la página blanca no se
                    // funde con la tarjeta.
                    let r = fit(tx.size_vec2() / ppp, body.shrink2(vec2(0.0, if pdf { 18.0 } else { 0.0 })));
                    picture(&p, t, te, tx, r);
                    drew = true;
                    if let Some(n) = te.pages {
                        *page = (*page).min(n.saturating_sub(1));
                        footer = Some(n);
                    } else if let Some((w, h)) = te.orig {
                        p.text(pos2(card.right() - sp::L, card.bottom() - sp::S), Align2::RIGHT_BOTTOM, format!("{w} × {h}"), design::text::mono_s(), t.faint);
                    }
                }
            }
            // PDF: «3 / 12» en mono entre dos botones mini (también ← →).
            if let Some(n) = footer.filter(|&n| n > 1) {
                let y = card.bottom() - sp::L;
                let label = format!("{} / {n}", *page + 1);
                let half = design::text_width(&p, &label, design::text::mono()) / 2.0 + sp::S + h::CHIP / 2.0;
                p.text(pos2(card.center().x, y), Align2::CENTER_CENTER, label, design::text::mono(), t.dim);
                for (dx, glyph, d) in [(-half, ic::CARET_LEFT, -1i32), (half, ic::CARET_RIGHT, 1)] {
                    let r = Rect::from_center_size(pos2(card.center().x + dx, y), vec2(h::CHIP, h::CHIP));
                    if design::icon_button_at(ui, t, r, egui::Id::new(("ql-page", d)), glyph, IconBtn::mini()).clicked() {
                        *page = (*page as i32 + d).clamp(0, n as i32 - 1) as u32;
                    }
                }
            }
        }
        if kind == Kind::Handler {
            design::micro(&p, body.center(), Align2::CENTER_CENTER, t!("previa.cargando_vista_previa"), t.faint);
            want = Some((item.source().to_string(), body));
            drew = true;
        }
        if let Some(txt) = text {
            text_scroll(ui, t, body, "ql-text", txt, &e.ext, true);
            drew = true;
        }
        if !drew {
            big_icon(&p, t, body.center() - vec2(0.0, 30.0), e, QL_ICON);
            let mut lines = vec![fsx::kind_label(e)];
            if let Some(n) = item.children {
                lines.push(tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
            }
            lines.push(tf!("previa.modificado", fecha = fsx::fmt_date(e.mtime)));
            p.text(body.center() + vec2(0.0, 60.0), Align2::CENTER_TOP, lines.join(" · "), design::text::body(), t.dim);
        }
        if bg.clicked() && !card_resp.hovered() {
            open = false;
        }
    });
    (open, want)
}
