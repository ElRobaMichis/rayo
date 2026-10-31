//! Panel de vista previa y Quick Look (barra espaciadora).

use super::actions::Action;
use super::icons;
use super::theme::{self, Theme};
use crate::fsx::thumbs::{ThumbKey, ThumbLoader, is_image_ext};
use crate::fsx::{self, FileEntry};
use egui::{Align2, Color32, ColorImage, FontId, Rect, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub struct TexEntry {
    pub tex: Option<TextureHandle>,
    pub orig: Option<(u32, u32)>,
    /// Páginas (si es un PDF).
    pub pages: Option<u32>,
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
        Textures { loader, map: HashMap::new(), order: VecDeque::new() }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        self.loader.next_frame();
        while let Ok(r) = self.loader.rx.try_recv() {
            let entry = match r.img {
                Some(img) => {
                    let size = [img.w as usize, img.h as usize];
                    let ci = if img.premultiplied {
                        ColorImage::from_rgba_premultiplied(size, &img.px)
                    } else {
                        ColorImage::from_rgba_unmultiplied(size, &img.px)
                    };
                    let tex = ctx.load_texture(&r.key.path, ci, TextureOptions::LINEAR);
                    TexEntry { tex: Some(tex), orig: img.orig, pages: img.pages }
                }
                None => TexEntry { tex: None, orig: None, pages: None },
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
    if e.is_dir {
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

/// Texto plano o Markdown con formato.
pub fn text_view(ui: &mut Ui, t: &Theme, text: &str, ext: &str, size: f32) {
    if matches!(ext, "md" | "markdown" | "mdx") {
        markdown(ui, t, text, size);
    } else {
        ui.add(egui::Label::new(egui::RichText::new(text).font(FontId::monospace(size - 1.0)).color(t.text)).wrap_mode(egui::TextWrapMode::Extend));
    }
}

/// Texto de una línea de Markdown con **negrita**, *cursiva*, `código` y [enlaces](url).
fn inline_job(text: &str, t: &Theme, size: f32, color: Color32, base: FontId) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let plain = TextFormat::simple(base.clone(), color);
    let mut rest = text;
    while !rest.is_empty() {
        // El próximo marcador.
        let next = ["**", "`", "*", "["].iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|x| x.0);
        let Some((i, m)) = next else {
            job.append(rest, 0.0, plain.clone());
            break;
        };
        job.append(&rest[..i], 0.0, plain.clone());
        let after = &rest[i + m.len()..];
        let (close, fmt) = match m {
            "**" => ("**", TextFormat::simple(theme::semibold(size), color)),
            "`" => {
                let mut f = TextFormat::simple(FontId::monospace(size - 1.0), t.accent);
                f.background = t.hover;
                ("`", f)
            }
            "*" => {
                let mut f = plain.clone();
                f.italics = true;
                ("*", f)
            }
            _ => ("]", TextFormat::simple(base.clone(), t.accent)),
        };
        match after.find(close) {
            Some(j) if j > 0 => {
                job.append(&after[..j], 0.0, fmt);
                rest = &after[j + close.len()..];
                // [texto](url): la url no se muestra.
                if m == "[" && rest.starts_with('(') {
                    rest = rest.find(')').map_or(rest, |k| &rest[k + 1..]);
                }
            }
            _ => {
                job.append(m, 0.0, plain.clone());
                rest = after;
            }
        }
    }
    job
}

/// Markdown básico: títulos, listas, citas, código, separadores y formato en línea.
pub fn markdown(ui: &mut Ui, t: &Theme, text: &str, size: f32) {
    let mut in_code = false;
    let width = ui.available_width();
    for line in text.lines().take(3000) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            egui::Frame::new().fill(t.hover).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                ui.set_width(width - 12.0);
                ui.label(egui::RichText::new(line).font(FontId::monospace(size - 1.5)).color(t.text));
            });
            continue;
        }
        if trimmed.is_empty() {
            ui.add_space(size * 0.5);
            continue;
        }
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            ui.add_space(if level <= 2 { 8.0 } else { 4.0 });
            let fs = size + (4usize.saturating_sub(level)) as f32 * 2.5;
            ui.label(inline_job(trimmed[level..].trim(), t, fs, t.text, theme::semibold(fs)));
            if level <= 2 {
                ui.separator();
            }
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            ui.separator();
            continue;
        }
        let indent = (line.len() - trimmed.len()) as f32 * 4.0;
        let list = ["- ", "* ", "+ "].iter().find(|m| trimmed.starts_with(*m)).map(|_| ("•".to_string(), &trimmed[2..])).or_else(|| {
            let n: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!n.is_empty() && trimmed[n.len()..].starts_with(". ")).then(|| (format!("{n}."), &trimmed[n.len() + 2..]))
        });
        if let Some((mark, rest)) = list {
            let (rest, mark) = match rest.strip_prefix("[ ] ").map(|r| (r, "☐".to_string())).or_else(|| rest.strip_prefix("[x] ").map(|r| (r, "☑".to_string()))) {
                Some((r, m)) => (r, m),
                None => (rest, mark),
            };
            ui.horizontal_wrapped(|ui| {
                ui.add_space(indent + 4.0);
                ui.label(egui::RichText::new(mark).size(size).color(t.dim));
                ui.label(inline_job(rest, t, size, t.text, FontId::proportional(size)));
            });
            continue;
        }
        if let Some(q) = trimmed.strip_prefix('>') {
            ui.horizontal_wrapped(|ui| {
                let r = ui.label(inline_job(q.trim(), t, size, t.dim, FontId::proportional(size)));
                ui.painter().line_segment([r.rect.left_top() - vec2(6.0, 0.0), r.rect.left_bottom() - vec2(6.0, 0.0)], egui::Stroke::new(2.0, t.border));
            });
            continue;
        }
        ui.label(inline_job(trimmed, t, size, t.text, FontId::proportional(size)));
    }
}

fn big_icon(p: &egui::Painter, t: &Theme, center: egui::Pos2, e: &FileEntry, size: f32) {
    let (glyph, color) = icons::for_entry(t, e.is_dir, &e.ext);
    p.text(center, Align2::CENTER_CENTER, glyph, theme::icon_font(size, true), color);
}

fn fit(tex_size: egui::Vec2, area: Rect) -> Rect {
    let s = (area.width() / tex_size.x).min(area.height() / tex_size.y).min(1.0 * 4.0);
    let size = tex_size * s;
    Rect::from_center_size(area.center(), size)
}

fn info_row(ui: &mut Ui, t: &Theme, k: &str, v: &str) {
    // El valor se acomoda en su columna (si no entra, sigue debajo de sí mismo, no del rótulo).
    ui.horizontal(|ui| {
        ui.add_sized([76.0, 16.0], egui::Label::new(egui::RichText::new(k).size(12.0).color(t.faint)));
        ui.add(egui::Label::new(egui::RichText::new(v).size(12.0).color(t.text)).wrap());
    });
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
    now: u32,
    host: Option<&super::prevhost::PreviewHost>,
    actions: &mut Vec<Action>,
) -> Option<(String, Rect)> {
    let mut want = None;
    if let Some((n, size)) = multi {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ic::FILES).font(theme::icon_font(64.0, true)).color(t.dim));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tf!("previa.elementos_seleccionados", n = n)).font(theme::semibold(14.0)));
            ui.label(egui::RichText::new(fsx::human_size(size)).color(t.dim));
        });
        return None;
    }
    let Some(item) = item else {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ic::EYE).font(theme::icon_font(40.0, false)).color(t.faint));
            ui.label(egui::RichText::new(t!("previa.selecciona_archivo")).color(t.faint));
        });
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
    egui::ScrollArea::vertical().id_salt("preview").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(12.0);
        let w = ui.available_width() - 24.0;
        let h = match kind {
            Kind::Text | Kind::Handler => (pane_h - 230.0).clamp(220.0, 900.0),
            _ => (w * 0.8).clamp(120.0, 320.0),
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let area = Rect::from_center_size(rect.center(), vec2(w, h));
        let p = ui.painter().clone();
        let mut dims = None;
        let mut pages = None;
        let mut drew = false;
        match kind {
            Kind::Text => {
                p.rect_filled(area, 6.0, t.sidebar);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink(10.0)));
                egui::ScrollArea::both().id_salt(("pv-text", &item.path)).auto_shrink([false, false]).show(&mut child, |ui| {
                    text_view(ui, t, text.unwrap_or(""), &e.ext, 12.5);
                });
                drew = true;
            }
            Kind::Handler => {
                p.rect_filled(area, 6.0, t.sidebar);
                p.text(area.center(), Align2::CENTER_CENTER, t!("previa.cargando_vista_previa"), FontId::proportional(12.0), t.faint);
                let visible = area.intersect(clip);
                if visible.height() > 40.0 {
                    want = Some((item.source().to_string(), visible));
                }
                drew = true;
            }
            Kind::Picture => {
                let px = (w.max(h) * ui.ctx().pixels_per_point()).min(768.0) as u32;
                if let Some(te) = tex.get_now(item.source(), px.next_multiple_of(128)) {
                    dims = te.orig;
                    pages = te.pages;
                    if let Some(tx) = &te.tex {
                        let r = fit(tx.size_vec2() / ui.ctx().pixels_per_point(), area);
                        p.rect_filled(r.expand(1.0), 6.0, t.border);
                        p.image(tx.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        drew = true;
                    }
                }
            }
            Kind::Icon => {}
        }
        if !drew {
            big_icon(&p, t, area.center(), e, 84.0);
            if e.cloud == fsx::Cloud::OnlineOnly {
                p.text(area.center() + vec2(0.0, 64.0), Align2::CENTER_TOP, t!("previa.solo_nube_abrelo_descargarlo"), FontId::proportional(11.5), t.dim);
            }
        }
        if super::views::double_clicked(&resp) {
            actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.label(egui::RichText::new(&e.name).font(theme::semibold(15.0)).color(t.text));
                let mut sub = fsx::kind_label(e);
                if let Some(s) = e.display_size() {
                    sub.push_str(&format!(" · {}", fsx::human_size(s)));
                }
                ui.label(egui::RichText::new(sub).size(12.0).color(t.dim));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(format!("{}  {}", ic::ARROW_SQUARE_OUT, t!("comun.abrir"))).clicked() {
                        actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
                    }
                    if ui.button(format!("{}  {}", ic::EYE, t!("previa.vista_rapida"))).on_hover_text(t!("previa.barra_espaciadora")).clicked() {
                        actions.push(Action::QuickLook);
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                if let Some(n) = pages {
                    info_row(ui, t, t!("previa.paginas"), &n.to_string());
                } else if let Some((dw, dh)) = dims {
                    info_row(ui, t, t!("previa.dimensiones"), &format!("{dw} × {dh}"));
                }
                if let Some(n) = item.children {
                    info_row(ui, t, t!("previa.contenido"), &tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
                }
                if let Some(s) = e.display_size() {
                    info_row(ui, t, t!("comun.tamano"), &tf!("previa.tamano_bytes", tamano = fsx::human_size(s), bytes = fsx::thousands(s as usize)));
                }
                info_row(ui, t, t!("comun.modificado"), &fsx::fmt_date(e.mtime));
                if e.ctime != 0 {
                    info_row(ui, t, t!("previa.creado"), &fsx::fmt_date(e.ctime));
                }
                info_row(ui, t, t!("previa.antiguedad"), &fsx::fmt_age(e.mtime, now));
                if let Some((_, _, tip)) = super::views::cloud_glyph(t, e.cloud) {
                    info_row(ui, t, t!("previa.nube"), tip);
                }
                info_row(ui, t, t!("previa.ubicacion"), fsx::parent_of(&item.path).as_deref().unwrap_or(""));
            });
        });
        ui.add_space(16.0);
    });
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
        p.rect_filled(full, 0.0, t.overlay);
        let card = Rect::from_center_size(full.center(), vec2((full.width() * 0.82).min(1400.0), (full.height() * 0.86).min(1000.0)));
        let card_resp = ui.interact(card, egui::Id::new("ql-card"), Sense::click());
        p.rect_filled(card, 12.0, t.chrome);
        p.rect_stroke(card, 12.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        let e = &item.entry;
        // Encabezado
        let head = Rect::from_min_size(card.min, vec2(card.width(), 44.0));
        let (glyph, color) = icons::for_entry(t, e.is_dir, &e.ext);
        p.text(pos2(head.left() + 18.0, head.center().y), Align2::LEFT_CENTER, glyph, theme::icon_font(18.0, true), color);
        let mut sub = fsx::kind_label(e);
        if let Some(s) = e.display_size() {
            sub.push_str(&format!(" · {}", fsx::human_size(s)));
        }
        let title = super::views::elided(&p, &e.name, theme::semibold(14.0), t.text, head.width() - 360.0);
        let tw = title.size().x;
        p.galley(pos2(head.left() + 44.0, head.center().y - title.size().y / 2.0), title, t.text);
        p.text(pos2(head.left() + 56.0 + tw, head.center().y), Align2::LEFT_CENTER, sub, FontId::proportional(12.0), t.dim);
        let close = Rect::from_center_size(pos2(head.right() - 24.0, head.center().y), vec2(28.0, 28.0));
        let cresp = ui.interact(close, egui::Id::new("ql-close"), Sense::click());
        if cresp.hovered() {
            p.rect_filled(close, 6.0, t.hover);
        }
        p.text(close.center(), Align2::CENTER_CENTER, ic::X, theme::icon_font(15.0, false), t.dim);
        let openr = Rect::from_min_size(pos2(close.left() - 92.0, close.top()), vec2(84.0, 28.0));
        let oresp = ui.interact(openr, egui::Id::new("ql-open"), Sense::click());
        p.rect_filled(openr, 6.0, if oresp.hovered() { t.accent } else { t.sel });
        p.text(openr.center(), Align2::CENTER_CENTER, t!("comun.abrir"), FontId::proportional(12.5), t.accent_text);
        if oresp.clicked() {
            actions.push(Action::Open { path: item.path.clone(), is_dir: e.is_dir });
            open = false;
        }
        if cresp.clicked() {
            open = false;
        }
        p.line_segment([head.left_bottom(), head.right_bottom()], egui::Stroke::new(1.0, t.border));
        let body = Rect::from_min_max(pos2(card.left() + 16.0, head.bottom() + 16.0), card.max - vec2(16.0, 16.0));
        let mut drew = false;
        if kind == Kind::Picture {
            let ppp = ctx.pixels_per_point();
            let side = ((body.width().max(body.height()) * ppp) as u32).clamp(256, 4096).next_multiple_of(256);
            let pdf = e.ext == "pdf";
            let size = if is_image_ext(&e.ext) || pdf { side } else { side.min(1024) };
            let key = if pdf { fsx::pdf::page_key(item.source(), *page) } else { item.source().to_string() };
            let mut footer = None;
            if let Some(te) = tex.get_now(&key, size) {
                if let Some(tx) = &te.tex {
                    let r = fit(tx.size_vec2() / ppp, body.shrink2(vec2(0.0, if pdf { 18.0 } else { 0.0 })));
                    p.image(tx.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    drew = true;
                    if let Some(n) = te.pages {
                        *page = (*page).min(n.saturating_sub(1));
                        footer = Some(n);
                    } else if let Some((w, h)) = te.orig {
                        p.text(pos2(card.right() - 18.0, card.bottom() - 10.0), Align2::RIGHT_BOTTOM, format!("{w} × {h}"), FontId::proportional(11.0), t.faint);
                    }
                }
            }
            // PDF: página actual y botones (también ← →).
            if let Some(n) = footer.filter(|&n| n > 1) {
                let y = card.bottom() - 16.0;
                let prev = Rect::from_center_size(pos2(card.center().x - 70.0, y), vec2(28.0, 22.0));
                let next = Rect::from_center_size(pos2(card.center().x + 70.0, y), vec2(28.0, 22.0));
                p.text(pos2(card.center().x, y), Align2::CENTER_CENTER, format!("{} / {n}", *page + 1), FontId::proportional(12.0), t.dim);
                for (r, icon, d) in [(prev, ic::CARET_LEFT, -1i32), (next, ic::CARET_RIGHT, 1)] {
                    let resp = ui.interact(r, egui::Id::new(("ql-page", d)), Sense::click());
                    if resp.hovered() {
                        p.rect_filled(r, 5.0, t.hover);
                    }
                    p.text(r.center(), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, false), t.text);
                    if resp.clicked() {
                        *page = (*page as i32 + d).clamp(0, n as i32 - 1) as u32;
                    }
                }
            }
        }
        if kind == Kind::Handler {
            p.text(body.center(), Align2::CENTER_CENTER, t!("previa.cargando_vista_previa"), FontId::proportional(13.0), t.faint);
            want = Some((item.source().to_string(), body));
            drew = true;
        }
        if let Some(txt) = text {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
            egui::ScrollArea::both().id_salt("ql-text").auto_shrink([false, false]).show(&mut child, |ui| {
                text_view(ui, t, txt, &e.ext, 14.0);
            });
            drew = true;
        }
        if !drew {
            big_icon(&p, t, body.center() - vec2(0.0, 30.0), e, 120.0);
            let mut lines = vec![fsx::kind_label(e)];
            if let Some(n) = item.children {
                lines.push(tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
            }
            lines.push(tf!("previa.modificado", fecha = fsx::fmt_date(e.mtime)));
            p.text(body.center() + vec2(0.0, 60.0), Align2::CENTER_TOP, lines.join("   ·   "), FontId::proportional(13.0), t.dim);
        }
        if bg.clicked() && !card_resp.hovered() {
            open = false;
        }
    });
    (open, want)
}
