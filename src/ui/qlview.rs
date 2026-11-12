//! Cuerpo de la Vista rápida: imagen con zoom, GIF, PDF continuo con tira, texto, carpeta
//! y vacíos. Dibuja en el `Rect` que le dan (a sangre); no conoce a `App` ni a la ventana.
//!
//! Piezas nuevas que nacen aquí (pasan a `design.rs` cuando otra superficie las pida): la
//! píldora flotante (`pill`), el paginador del PDF y la píldora del GIF.

use super::config::QuickLookCfg;
use super::design::{self, IconBtn, Tone, h, icon, radius, sp, stroke};
use super::motion;
use super::preview::{self, Kind, Pic, PreviewItem, Textures};
use super::prevhost::Status;
use super::ql::{FOLDER_CARD_W, HERO_GLYPH, HERO_ICON, LOADING_AFTER, MD_MAX, NOTICE_W, OPEN_DELAY, PDF_MAX, QL_NARROW, STRIP_THUMB, STRIP_W, ZOOM_MAX, ZOOM_STEP};
use super::theme::Theme;
use super::views;
use crate::fsx::{self, anim, pdf, thumbs, thumbs::is_image_ext};
use crossbeam_channel::{Receiver, bounded, unbounded};
use egui::{Align2, Color32, ColorImage, CornerRadius, Key, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use egui_phosphor::regular as ic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Lo que flota sobre el contenido va a esta distancia del borde.
const FLOAT_GAP: f32 = sp::M;
/// Lado de una página de PDF a pedido: en pasos de 256 y con tope (una A4 a 3072 son ~26 MB).
const PAGE_SIDE_MAX: u32 = 3072;
/// Miniaturas de la tira, en píxeles.
const STRIP_TEX: u32 = 256;
/// Puntos del medidor de la píldora del GIF.
const GIF_DOTS: f32 = 24.0;
/// Dónde se deja (en `ctx.data`) el pedido de la foto encajada de la Vista rápida, para
/// precalentar con el mismo la foto elegida y las vecinas.
pub const FIT_KEY_ID: &str = "ql-fit-key";

/// Zoom, desplazamiento, página, animación, tira. Se reinicia con `reset` al cambiar de archivo.
#[derive(Default)]
pub struct ViewState {
    /// Página actual del PDF (0 = la primera). Si quien llama la cambia, es «ir a esa página».
    pub page: u32,
    /// Estado del visor de Windows para este archivo; lo pone quien llama antes de `body`.
    pub handler: Option<Status>,
    /// Doble clic en una carpeta o en «sin vista previa»: abrir. Lo apaga quien lo atiende.
    pub open: bool,
    /// «Desbloquear» en el aviso de archivo descargado de Internet. Lo apaga quien lo atiende.
    pub unblock: bool,
    /// Archivo al que corresponde lo de abajo, y desde cuándo se muestra.
    path: String,
    since: f64,
    /// Imagen: None = encajada; si no, píxeles de pantalla por píxel de la imagen (1 = 100 %).
    zoom: Option<f32>,
    /// Corrimiento del centro de la imagen respecto del centro del cuerpo (puntos).
    pan: Vec2,
    anim: Option<Anim>,
    anim_tried: bool,
    /// Último instante con el puntero sobre el cuerpo (la píldora del GIF se va después).
    hover_at: f64,
    /// PDF: páginas, proporción (alto / ancho) de la primera y de cada una, cuando llegan.
    pages: Option<u32>,
    ratio0: f32,
    sizes: Vec<(f32, f32)>,
    sizes_rx: Option<Receiver<Option<Sizes>>>,
    /// La página que se publicó en `page` el cuadro anterior (si `page` cambió, es un salto).
    shown_page: u32,
    /// La página a la que la tira se desplazó por última vez.
    strip_page: Option<u32>,
    /// Imagen: la versión que se ve (y desde cuándo) y, mientras la nueva entra, la anterior.
    top: Option<(Pic, f64)>,
    base: Option<Pic>,
    /// Lo último que se dibujó (imagen, opaca, rectángulo) y, al cambiar de archivo, eso mismo
    /// retenido hasta un instante (`HOLD_PREV`) mientras llega lo nuevo.
    shown: Option<(Pic, bool, Rect)>,
    held: Option<(Pic, bool, Rect, f64)>,
    /// Lado pedido en el último cuadro y cuándo cambió el zoom por última vez.
    side: u32,
    zoom_at: f64,
}

/// Ancho y alto de cada página de un PDF.
type Sizes = Vec<(f32, f32)>;

// ---------------------------------------------------------------- contrato

/// Dibuja el cuerpo en `rect`. Some = «pon el visor de Windows aquí» (ruta, rectángulo en puntos).
#[allow(clippy::too_many_arguments)]
pub fn body(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, text: Option<&str>, tex: &mut Textures, cfg: &mut QuickLookCfg, st: &mut ViewState) -> Option<(String, Rect)> {
    let now = ui.input(|i| i.time);
    if st.path != item.path {
        let held = st.shown.take();
        reset(st);
        st.held = held.map(|(pic, opaque, r)| (pic, opaque, r, now + motion::HOLD_PREV));
        st.path = item.path.clone();
        st.since = now;
    }
    if ui.rect_contains_pointer(rect) {
        st.hover_at = now;
    }
    let e = &item.entry;
    let _p = super::perf::scope("ql.cuerpo");
    match preview::kind(item, text.is_some()) {
        Kind::Text => text_body(ui, t, rect, item, text.unwrap_or("")),
        Kind::Icon if e.is_dir => folder(ui, t, rect, item, st),
        Kind::Icon => no_preview(ui, t, rect, item, st, file_status(item)),
        Kind::Handler if st.handler != Some(Status::Failed) => return handler(ui, t, rect, item, st, now),
        _ if e.ext == "pdf" => pdf_body(ui, t, rect, item, tex, cfg, st, now),
        _ => picture(ui, t, rect, item, tex, st, now),
    }
    None
}

/// Micro-etiqueta del encabezado: tipo · lo propio del contenido · tamaño
/// («PDF · 12 PÁGINAS · 318 KB»).
pub fn subtitle(item: &PreviewItem, tex: &mut Textures, st: &ViewState) -> String {
    let _p = super::perf::scope("ql.subtítulo");
    let e = &item.entry;
    let mine = st.path == item.path;
    let download = if e.is_dir { None } else { fsx::downloading(&e.name) };
    let mut parts = vec![if download.is_some() {
        t!("lista.descarga").to_string()
    } else if e.is_dir {
        t!("comun.carpeta").to_string()
    } else if e.ext.is_empty() {
        fsx::kind_label(e)
    } else {
        e.ext.to_uppercase()
    }];
    let peek = tex.peek(item.source());
    if e.is_dir {
        if let Some(n) = item.children {
            parts.push(tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
        }
    } else if e.cloud == fsx::Cloud::OnlineOnly {
        parts.push(t!("rapida.solo_nube").to_string());
    } else if e.ext == "pdf" {
        if let Some(n) = st.pages.filter(|_| mine).or(peek.and_then(|te| te.pages)) {
            parts.push(tp!(n, "rapida.paginas_n", n = fsx::thousands(n as usize)));
        }
    } else if is_image_ext(&e.ext) {
        if let Some((w, h)) = peek.and_then(|te| te.orig) {
            parts.push(format!("{w} × {h}"));
        } else if let Some(d) = fsx::props::info(&item.path).and_then(|i| i.dims.clone()) {
            // Mientras llega la foto (o si solo hay la miniatura del shell): las de Windows.
            parts.push(d);
        }
        if let Some(n) = st.anim.as_ref().filter(|_| mine).map(|a| a.count).filter(|&n| n > 1) {
            parts.push(tp!(n, "rapida.fotogramas_n", n = fsx::thousands(n as usize)));
        }
    } else if super::qlmedia::kind(&e.ext).is_some() {
        // Medidas y duración de las propiedades de Windows («00:01:28» → «1:28»).
        if let Some(info) = fsx::props::info(&item.path) {
            parts.extend(info.dims.clone());
            let dur = info.rows.iter().find(|(f, _)| *f == fsx::props::Field::Duration);
            parts.extend(dur.map(|(_, d)| short_duration(d)));
        }
    }
    if let Some(s) = e.display_size() {
        parts.push(fsx::human_size(s));
    } else if e.is_dir && item.real.is_none() && !fsx::archive::is_inside(&item.path) {
        let d = fsx::dirsize::get(&item.path);
        if d.done || d.bytes > 0 {
            parts.push(fsx::human_size(d.bytes));
        }
    }
    parts.join(" · ")
}

/// La herramienta del contenido en el encabezado (mostrar / ocultar la tira del PDF), o nada.
pub fn tool(ui: &mut Ui, t: &Theme, item: &PreviewItem, cfg: &mut QuickLookCfg, st: &mut ViewState) {
    if item.entry.ext != "pdf" || st.path != item.path || st.pages.unwrap_or(0) < 2 {
        return;
    }
    let r = design::icon_button(ui, t, ic::SQUARE_SPLIT_HORIZONTAL, IconBtn { on: cfg.strip, ..Default::default() });
    if design::tip(r, t, t!("rapida.miniaturas")).clicked() {
        cfg.strip = !cfg.strip;
    }
}

/// ← → RePág AvPág sobre el contenido (página del PDF). ¿La consumió?
pub fn key(st: &mut ViewState, item: &PreviewItem, key: Key) -> bool {
    if item.entry.ext != "pdf" || st.path != item.path {
        return false;
    }
    let Some(n) = st.pages.filter(|&n| n > 1) else { return false };
    let d: i64 = match key {
        Key::ArrowRight | Key::PageDown => 1,
        Key::ArrowLeft | Key::PageUp => -1,
        _ => return false,
    };
    st.page = (st.shown_page as i64 + d).clamp(0, n as i64 - 1) as u32;
    true
}

/// Al cambiar de archivo: encajada, página 1, sin animación (la que había se corta).
pub fn reset(st: &mut ViewState) {
    let old = std::mem::take(st);
    // Los fotogramas de un GIF: cientos de MB que soltar, en otro hilo.
    if let Some(a) = old.anim {
        preview::drop_later(a);
    }
}

// ---------------------------------------------------------------- piezas

/// Esquinas de abajo redondeadas (izquierda, derecha) con radio `r`.
/// El pozo: `sunken`, con la trama de puntos si es para mirar (no para leer).
fn well(p: &Painter, t: &Theme, rect: Rect, cr: CornerRadius, grid: bool) {
    let _p = super::perf::scope("ql.pozo");
    p.rect_filled(rect, cr, t.sidebar);
    if grid {
        design::dot_grid(p, t, rect.shrink(sp::XS));
    }
}

/// Píldora flotante (zoom, GIF, paginador): `raised`, `border_strong`, radio M, sombra de popover.
fn pill(p: &Painter, t: &Theme, r: Rect) {
    p.add(design::popover_shadow(t).as_shape(r, radius::M));
    p.rect_filled(r, radius::M, t.raised);
    p.rect_stroke(r, radius::M, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
}

/// Imagen con radio `S` y borde fino si es opaca (foto, página); suelta si tiene transparencia.
fn image(p: &Painter, t: &Theme, pic: &Pic, opaque: bool, r: Rect, a: f32) {
    let tint = Color32::WHITE.gamma_multiply(a);
    if opaque {
        pic.paint(p, r, Some(radius::S.into()), tint);
        p.rect_stroke(r, radius::S, Stroke::new(stroke::HAIR, t.border.gamma_multiply(a)), StrokeKind::Outside);
    } else {
        pic.paint(p, r, None, tint);
    }
}

/// Una capa de la foto (relevo): con las esquinas redondeadas si es opaca, sin marco.
fn layer(p: &Painter, pic: &Pic, opaque: bool, r: Rect, a: f32) {
    pic.paint(p, r, opaque.then(|| radius::S.into()), Color32::WHITE.gamma_multiply(a));
}

/// Página de papel mientras llega su imagen.
fn blank_page(p: &Painter, t: &Theme, r: Rect) {
    p.rect_filled(r, radius::S, Color32::WHITE);
    p.rect_stroke(r, radius::S, Stroke::new(stroke::HAIR, t.border), StrokeKind::Outside);
}

/// Medidor que recorre (10 puntos) y micro-etiqueta `faint`, centrados en `rect`.
fn loading(p: &Painter, t: &Theme, rect: Rect, label: &str) {
    const METER: f32 = 10.0 * 5.0 - 2.0;
    let g = p.layout_job(design::micro_job(label, t.faint));
    let w = METER + sp::S + g.size().x;
    let x = rect.center().x - w / 2.0;
    design::dot_meter_busy(p, t, Rect::from_min_size(pos2(x, rect.center().y - 1.5), vec2(METER, 3.0)));
    p.galley(pos2(x + METER + sp::S, rect.center().y - g.size().y / 2.0), g, t.faint);
}

/// Nada que mostrar todavía: el pozo solo y, pasados `LOADING_AFTER`, «Cargando».
fn waiting(ui: &Ui, p: &Painter, t: &Theme, rect: Rect, st: &ViewState, now: f64, label: &str) {
    let left = st.since + LOADING_AFTER - now;
    if left <= 0.0 {
        loading(p, t, rect, label);
    } else {
        ui.ctx().request_repaint_after(secs(left));
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.001))
}

/// Duración de Windows («00:01:28») como en un reproductor («1:28»; «1:02:03» pasada la hora).
fn short_duration(d: &str) -> String {
    let mut parts: Vec<&str> = d.split(':').collect();
    while parts.len() > 2 && parts[0].trim_start_matches('0').is_empty() {
        parts.remove(0);
    }
    let first = parts[0].trim_start_matches('0');
    parts[0] = if first.is_empty() { "0" } else { first };
    parts.join(":")
}

/// Línea de estado de un archivo sin vista previa: solo en la nube, descarga en curso.
fn file_status(item: &PreviewItem) -> Option<(Tone, &'static str, &'static str)> {
    let e = &item.entry;
    if !e.is_dir && fsx::downloading(&e.name).is_some() {
        Some((Tone::Accent, ic::DOWNLOAD_SIMPLE, t!("previa.descarga_en_curso")))
    } else if e.cloud == fsx::Cloud::OnlineOnly {
        Some((Tone::Neutral, ic::CLOUD, t!("previa.solo_nube_abrelo_descargarlo")))
    } else {
        None
    }
}

/// Doble clic en el cuerpo: abrir (carpeta, sin vista previa).
fn open_on_double_click(ui: &mut Ui, rect: Rect, st: &mut ViewState) {
    if ui.interact(rect, egui::Id::new(("ql-open", &st.path)), Sense::click()).double_clicked() {
        st.open = true;
    }
}

// ---------------------------------------------------------------- carpeta, sin vista previa, aviso

/// Carpeta: glifo grande, nombre y la tarjeta con contenido, tamaño y fecha (se va si es angosta).
fn folder(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, st: &mut ViewState) {
    open_on_double_click(ui, rect, st);
    let p = ui.painter_at(rect);
    well(&p, t, rect, CornerRadius::ZERO, true);
    let e = &item.entry;
    let name = views::elided(&p, &e.name, design::text::title(), t.text, rect.width() - 2.0 * sp::L);
    let card_h = 3.0 * h::HEAD + 2.0 * sp::M;
    let head_h = HERO_GLYPH + sp::S + name.size().y;
    let card = rect.width() >= QL_NARROW && rect.height() >= head_h + sp::L + card_h + 2.0 * sp::L;
    let block_h = head_h + if card { sp::L + card_h } else { 0.0 };
    let top = (rect.center().y - block_h / 2.0).max(rect.top() + sp::L);
    let cx = rect.center().x;
    p.text(pos2(cx, top + HERO_GLYPH / 2.0), Align2::CENTER_CENTER, egui_phosphor::fill::FOLDER, design::icon_fill(HERO_GLYPH), t.folder);
    let name_w = name.size().x;
    p.galley(pos2(cx - name_w / 2.0, top + HERO_GLYPH + sp::S), name, t.text);
    if !card {
        return;
    }
    let contents = item.children.map_or_else(|| "—".to_string(), |n| tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
    let size = if let Some(s) = e.display_size() {
        fsx::human_size(s)
    } else if item.real.is_none() && !fsx::archive::is_inside(&item.path) {
        // Fuera del índice (red, USB): se suma en segundo plano y se ve avanzar.
        let d = fsx::dirsize::get(&item.path);
        if d.done { fsx::human_size(d.bytes) } else { format!("{} {}", fsx::human_size(d.bytes), t!("previa.calculando")) }
    } else {
        "—".to_string()
    };
    let r = Rect::from_min_size(pos2(cx - FOLDER_CARD_W / 2.0, top + head_h + sp::L), vec2(FOLDER_CARD_W, card_h));
    p.rect_filled(r, radius::L, t.raised);
    p.rect_stroke(r, radius::L, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(sp::M)).layout(egui::Layout::top_down(egui::Align::Min)));
    cu.spacing_mut().item_spacing.y = 0.0;
    design::meta_row(&mut cu, t, t!("previa.contenido"), &contents);
    design::meta_row(&mut cu, t, t!("comun.tamano"), &size);
    design::meta_row(&mut cu, t, t!("comun.modificado"), &fsx::fmt_date(e.mtime));
}

/// Sin vista previa: el ícono grande del archivo (el de Windows), el nombre y una línea de estado.
fn no_preview(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, st: &mut ViewState, status: Option<(Tone, &str, &str)>) {
    open_on_double_click(ui, rect, st);
    let p = ui.painter_at(rect);
    well(&p, t, rect, CornerRadius::ZERO, true);
    let e = &item.entry;
    // Una descarga a medias se nombra por el archivo que va a ser.
    let shown = if e.is_dir { &e.name } else { fsx::downloading(&e.name).unwrap_or(&e.name) };
    let name = views::elided(&p, shown, design::text::title(), t.text, rect.width() - 2.0 * sp::L);
    let status_w = status.map(|(_, _, s)| (icon::M + sp::S + design::text_width(&p, s, design::text::small())).min(rect.width() - 2.0 * sp::L));
    let block_h = HERO_GLYPH + sp::S + name.size().y + if status.is_some() { sp::S + h::HEAD } else { 0.0 };
    let top = (rect.center().y - block_h / 2.0).max(rect.top() + sp::S);
    let cx = rect.center().x;
    views::file_icon(&p, t, pos2(cx, top + HERO_GLYPH / 2.0), HERO_ICON, HERO_GLYPH, &item.path, e.is_dir, &e.ext, 1.0);
    let y = top + HERO_GLYPH + sp::S;
    let name_size = name.size();
    p.galley(pos2(cx - name_size.x / 2.0, y), name, t.text);
    if let (Some((tone, glyph, text)), Some(w)) = (status, status_w) {
        let r = Rect::from_min_size(pos2(cx - w / 2.0, y + name_size.y + sp::S), vec2(w, h::HEAD));
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
        design::status_line(&mut cu, t, tone, glyph, text);
    }
}

/// Descargado de Internet: Windows bloquea el visor. El aviso del panel, centrado, con «Desbloquear».
fn blocked(ui: &mut Ui, t: &Theme, rect: Rect, st: &mut ViewState) {
    let p = ui.painter_at(rect);
    well(&p, t, rect, CornerRadius::ZERO, true);
    let w = NOTICE_W.min(rect.width() - 2.0 * sp::L);
    // Alto medido el cuadro anterior, para centrarlo.
    let hid = egui::Id::new(("ql-blocked-h", &st.path));
    let hgt = ui.data(|d| d.get_temp::<f32>(hid)).unwrap_or(88.0);
    let r = Rect::from_min_size(pos2(rect.center().x - w / 2.0, rect.center().y - hgt / 2.0), vec2(w, hgt));
    let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
    let out = egui::Frame::new().fill(t.warn_soft).corner_radius(radius::M).inner_margin(sp::S).show(&mut cu, |ui| {
        ui.set_width(w - 2.0 * sp::S);
        design::status_line(ui, t, Tone::Warn, ic::SHIELD_WARNING, t!("previa.descargado_internet"));
        ui.add_space(sp::XS);
        let b = design::button(ui, t, design::Kind::Secondary, ic::LOCK_SIMPLE_OPEN, t!("previa.desbloquear"), true);
        if design::tip(b, t, t!("previa.desbloquear_ayuda")).clicked() {
            st.unblock = true;
        }
    });
    let real = out.response.rect.height();
    if (real - hgt).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(hid, real));
        ui.ctx().request_repaint();
    }
}

// ---------------------------------------------------------------- visor de Windows

/// Office y otros visores: el pozo con «Cargando vista previa» debajo de la ventana nativa, que
/// se pide pasados `OPEN_DELAY` (recorrer con ↓ no abre veinte visores).
fn handler(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, st: &mut ViewState, now: f64) -> Option<(String, Rect)> {
    if fsx::props::info(&item.path).is_some_and(|i| i.blocked) {
        blocked(ui, t, rect, st);
        return None;
    }
    // El visor de fuentes va con aire (ver abajo): sin trama, el margen se funde con su fondo.
    let font = matches!(item.entry.ext.as_str(), "ttf" | "otf" | "ttc" | "fon" | "fnt");
    let p = ui.painter_at(rect);
    well(&p, t, rect, CornerRadius::ZERO, !font);
    // Ya visible, la ventana nativa lo tapa: no animar el medidor por debajo.
    if st.handler != Some(Status::Ready) {
        waiting(ui, &p, t, rect, st, now, t!("previa.cargando_vista_previa"));
    }
    let left = st.since + OPEN_DELAY - now;
    if left > 0.0 {
        ui.ctx().request_repaint_after(secs(left));
        return None;
    }
    // El visor de fuentes escribe pegado a sus bordes; su fondo ya es `sunken`, así que el aire
    // no se ve como marco. Office y los demás, a sangre.
    Some((item.source().to_string(), if font { rect.shrink(sp::L) } else { rect }))
}

// ---------------------------------------------------------------- texto

/// Texto, Markdown o código sobre `bg`, margen 16 × 20; Markdown en una columna de 680 como
/// máximo. Si el archivo es más largo que lo leído, lo dice al final.
fn text_body(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, text: &str) {
    let _p = super::perf::scope("ql.texto");
    let e = &item.entry;
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.bg);
    // Lo leído: el archivo pesa más (un BOM quitado son 3 bytes; lo inválido solo alarga el texto).
    let cut = e.size > text.len() as u64 + 3;
    let kb = (text.len() + 3) / 1024;
    let md = matches!(e.ext.as_str(), "md" | "markdown" | "mdx");
    let sideways = super::richtext::is_code(&e.ext);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let area = if sideways { egui::ScrollArea::both() } else { egui::ScrollArea::vertical() };
    area.id_salt(("ql-text", &item.path)).auto_shrink([false, false]).show(&mut child, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, sp::L as i8)).show(ui, |ui| {
            let avail = rect.width() - 40.0;
            let content = |ui: &mut Ui| {
                preview::text_view(ui, t, text, &e.ext, true);
                if cut {
                    ui.add_space(sp::L);
                    let _ = design::micro_label(ui, &tf!("rapida.primeros_kb", n = kb), t.faint);
                }
            };
            if md {
                let col = avail.min(MD_MAX);
                ui.horizontal_top(|ui| {
                    ui.add_space(((avail - col) / 2.0).max(0.0));
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        content(ui);
                    });
                });
            } else {
                if !sideways {
                    ui.set_max_width(avail);
                }
                content(ui);
            }
        });
    });
}

// ---------------------------------------------------------------- imagen y GIF

/// Foto (con zoom), GIF animado o miniatura de Windows, encajada con 16 de aire.
fn picture(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, tex: &mut Textures, st: &mut ViewState, now: f64) {
    let _p = super::perf::scope("ql.imagen");
    let e = &item.entry;
    let src = item.source();
    let photo = is_image_ext(&e.ext);
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let p = ui.painter_at(rect);
    let area = rect.shrink(sp::L);
    if e.ext == "gif" {
        anim_tick(&ctx, src, st, now);
    }
    let size_of = |o: (u32, u32)| vec2(o.0 as f32, o.1 as f32);
    let fit_of = |n: Vec2| (area.width() * ppp / n.x).min(area.height() * ppp / n.y).min(4.0);
    // Una foto se pide encajada en el área (el hilo de carga decide el lado cuando sabe las
    // medidas y la orientación) y, antes, lo que haya enseguida (`QUICK`: la vista previa que trae
    // adentro, con las medidas del original). Ampliada, al nivel que haga falta, sin tope.
    let (aw, ah) = ((area.width() * ppp) as u32, (area.height() * ppp) as u32);
    // Con histéresis: un área que baila un píxel (al acomodarse la ventana) no cambia el pedido.
    let prev = ctx.data(|d| d.get_temp::<u32>(egui::Id::new(FIT_KEY_ID)));
    let fit_key = match prev.and_then(|k| thumbs::fit_area(k).map(|a| (k, a))) {
        Some((k, (pw, ph))) if pw.abs_diff(aw) <= 48 && ph.abs_diff(ah) <= 48 => k,
        _ => thumbs::fit_key(aw, ah),
    };
    if photo {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(FIT_KEY_ID), fit_key));
    }
    let orig = tex.peek(src).and_then(|te| te.orig).map(size_of);
    let natural = |te: &preview::TexEntry| te.orig.map(size_of).or(te.pic.as_ref().map(|x| x.size_vec2()));
    let known = if photo { orig } else { tex.peek(src).and_then(natural) };
    let side = match (photo, st.zoom, orig) {
        (true, Some(z), Some(n)) => {
            // Ampliando: el nivel más chico que cubra lo que ocupa en pantalla, en potencias de
            // dos, hasta el original; se pide cuando el zoom lleva `ZOOM_SETTLE` quieto.
            let want = ((n.x.max(n.y) * z) as u32).next_power_of_two().max(thumbs::FIT_MIN).min((n.x.max(n.y) as u32).next_multiple_of(256));
            let settle = st.zoom_at + motion::ZOOM_SETTLE - now;
            if settle > 0.0 && st.side != 0 {
                ctx.request_repaint_after(secs(settle));
                st.side
            } else {
                want
            }
        }
        (true, None, Some(n)) => {
            // Si ya hay una versión que alcanza (precalentada para otro tamaño de ventana, o la de
            // un zoom anterior), esa: no se decodifica otra casi igual.
            let need = thumbs::fit_side((n.x as u32, n.y as u32), ((area.width() * ppp) as u32, (area.height() * ppp) as u32));
            // (Tampoco una mucho más grande: reducida a menos de la mitad, sin mipmaps, centellea.)
            tex.best_key(src).filter(|&(_, px)| px as f32 >= need as f32 * 0.9 && px as f32 <= need as f32 * 2.2).map_or(fit_key, |(k, _)| k)
        }
        (true, ..) => fit_key,
        (false, ..) => {
            let side_px = match known {
                Some(n) => n.x.max(n.y) * st.zoom.unwrap_or_else(|| fit_of(n)),
                None => area.width().max(area.height()) * ppp,
            };
            (side_px.max(1.0) as u32).clamp(256, 1024).next_multiple_of(256)
        }
    };
    st.side = side;
    if photo && orig.is_none() {
        let _ = tex.get_photo(src, thumbs::QUICK);
    }
    let got = if photo { tex.get_photo(src, side) } else { tex.get_now(src, side) };
    // Lo que se ve ahora: la imagen de este archivo o, hasta `HOLD_PREV`, la del anterior.
    // Una foto lleva marco por su tipo, no por el análisis de cada versión (si no, aparece y
    // desaparece entre una y otra). Lo que no es foto y tiene transparencia es un ícono
    // (programa, comprimido): sin vista previa.
    let framed = matches!(e.ext.as_str(), "jpg" | "jpeg" | "jfif" | "heic" | "heif" | "dng" | "jxr" | "tif" | "tiff" | "bmp");
    let cur = got.and_then(|te| te.pic.clone().filter(|_| !te.icon && (photo || te.opaque)).map(|pic| (pic, te.opaque || framed)));
    if cur.is_none() {
        if got.is_some() {
            no_preview(ui, t, rect, item, st, file_status(item));
            return;
        }
        well(&p, t, rect, CornerRadius::ZERO, true);
        if let Some((pic, opaque, r, until)) = st.held.clone().filter(|h| h.3 > now) {
            // La foto anterior, quieta, mientras llega esta (si llega antes, cambio de golpe).
            image(&p, t, &pic, opaque, r, 1.0);
            ctx.request_repaint_after(secs(until - now));
            st.held = Some((pic, opaque, r, until));
            return;
        }
        st.held = None;
        // Para una foto, el indicador recién a los 300 ms: a 150 aparecía justo antes que ella.
        let left = st.since + motion::LOADER_AFTER - now;
        if left <= 0.0 {
            loading(&p, t, rect, t!("rapida.cargando"));
        } else {
            ctx.request_repaint_after(secs(left));
        }
        return;
    }
    let (cur_pic, opaque) = cur.expect("hay imagen");
    let held = st.held.take().is_some_and(|h| h.3 > now);
    // Relevo: una versión con más píxeles entra por encima en `FAST` (la de abajo queda al 100 %);
    // de golpe si la de arriba lleva menos de `SWAP_INSTANT` a la vista o tiene transparencia.
    // Una con menos píxeles (otra versión que quedó a mano) no reemplaza a la que se ve.
    let px_of = |pic: &Pic| pic.size_vec2().x * pic.size_vec2().y;
    match st.top.take() {
        None => {
            // Primera imagen: en un pozo vacío, `FAST`; si reemplaza a la foto anterior retenida, de golpe.
            st.top = Some((cur_pic.clone(), if held { f64::MIN } else { now }));
        }
        Some((top, t0)) if top.same(&cur_pic) || px_of(&cur_pic) <= px_of(&top) => st.top = Some((top, t0)),
        Some((top, t0)) => {
            if now - t0 < motion::SWAP_INSTANT || !opaque {
                st.base = None;
                st.top = Some((cur_pic.clone(), f64::MIN));
            } else {
                if motion::since(&ctx, t0, design::dur::FAST) >= 1.0 || st.base.is_none() {
                    st.base = Some(top);
                }
                st.top = Some((cur_pic.clone(), now));
            }
        }
    }
    let (pic, at) = st.top.clone().expect("recién puesta");
    // Geometría: la proporción del original y el área; sin original todavía, la proporción de lo
    // que haya, encajada (una foto de cámara siempre es más grande que el área). Nunca los
    // píxeles de una miniatura como tamaño natural.
    let nat = match orig.filter(|_| photo) {
        Some(n) => n,
        None if photo => {
            let s = pic.size_vec2();
            s * (area.width() * ppp / s.x).min(area.height() * ppp / s.y)
        }
        None => known.unwrap_or(pic.size_vec2()),
    };
    well(&p, t, rect, CornerRadius::ZERO, true);
    let mut pic = pic;
    // El GIF, cuando ya hay dos fotogramas: su textura en vez de la quieta.
    if let Some(x) = st.anim.as_ref().filter(|a| a.frames.len() >= 2).and_then(|a| a.tex.as_ref()) {
        pic = Pic::of(x);
    }
    let fit = fit_of(nat);
    let zmax = ZOOM_MAX.max(fit);
    // Zoom: rueda hacia el puntero, doble clic encajar ↔ 100 %, arrastrar mueve.
    // Sin el original todavía no hay zoom (su cuenta depende de él).
    let zoomable = photo && orig.is_some();
    let zoom_before = st.zoom;
    let resp = ui.interact(rect, egui::Id::new(("ql-img", &item.path)), if zoomable { Sense::click_and_drag() } else { Sense::hover() });
    if zoomable {
        let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
        if resp.hovered() {
            let notches: f32 = ui.input(|i| {
                i.events
                    .iter()
                    .map(|ev| match ev {
                        egui::Event::MouseWheel { unit, delta, .. } => match unit {
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            egui::MouseWheelUnit::Page => delta.y * 3.0,
                        },
                        _ => 0.0,
                    })
                    .sum()
            });
            if notches != 0.0 {
                let z0 = st.zoom.unwrap_or(fit);
                zoom_at(st, rect.center(), pointer, z0, (z0 * ZOOM_STEP.powf(notches)).clamp(fit, zmax), fit);
            }
        }
        if resp.double_clicked() {
            if st.zoom.is_some() {
                (st.zoom, st.pan) = (None, Vec2::ZERO);
            } else {
                zoom_at(st, rect.center(), pointer, fit, 1.0f32.clamp(fit, zmax), fit);
            }
        }
        if st.zoom.is_some() {
            if resp.dragged() {
                st.pan += resp.drag_delta();
            }
            if resp.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if resp.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
    }
    if st.zoom != zoom_before {
        st.zoom_at = now;
    }
    if st.zoom != zoom_before || resp.dragged() {
        preview::moving(&ctx);
    }
    let z = st.zoom.unwrap_or(fit);
    let size = nat * z / ppp;
    // No se puede sacar la imagen del cuerpo: siempre queda al menos un cuarto a la vista.
    let lim = |s: f32, b: f32| ((s + b) / 2.0 - s.min(b) / 4.0).max(0.0);
    st.pan = vec2(st.pan.x.clamp(-lim(size.x, rect.width()), lim(size.x, rect.width())), st.pan.y.clamp(-lim(size.y, rect.height()), lim(size.y, rect.height())));
    let r = Rect::from_center_size(rect.center() + st.pan, size);
    // Al alejar el zoom, la de detalle queda mucho más grande que lo que ocupa (reducida a menos
    // de la mitad, sin mipmaps, centellea): en cuanto hay una versión que alcanza, esa, de golpe.
    let disp = r.width().max(r.height()) * ppp;
    let side_of = |x: &Pic| x.size_vec2().max_elem();
    let mut pic = pic;
    let mut at = at;
    if st.anim.is_none() && side_of(&pic) > disp * 2.2 && !cur_pic.same(&pic) && side_of(&cur_pic) >= disp * 0.9 {
        (pic, at) = (cur_pic.clone(), f64::MIN);
        st.top = Some((cur_pic.clone(), at));
        st.base = None;
    }
    // La de abajo (la versión anterior) al 100 %; la nueva encima, entrando en `FAST`; el marco,
    // una vez y entero.
    let k = motion::since(&ctx, at, design::dur::FAST);
    if let Some(b) = st.base.as_ref().filter(|_| k < 1.0) {
        layer(&p, b, opaque, r, 1.0);
    } else {
        st.base = None;
    }
    layer(&p, &pic, opaque, r, k);
    if opaque {
        p.rect_stroke(r, radius::S, Stroke::new(stroke::HAIR, t.border), StrokeKind::Outside);
    }
    st.shown = Some((pic.clone(), opaque, r));
    // «Enfocando»: con algo a la vista y la versión nítida que tarda más de `SHARPEN_HINT`
    // (un disco mecánico arrancando), el medidor de puntos en una píldora abajo a la derecha (o
    // dentro de la del zoom, antes del porcentaje).
    let sharpening = photo && !tex.has(src, side) && {
        let since = st.since.max(st.zoom_at);
        let left = since + motion::SHARPEN_HINT - now;
        if left > 0.0 {
            ctx.request_repaint_after(secs(left));
        }
        left <= 0.0
    };
    const METER: f32 = 5.0 * 5.0 - 2.0;
    let meter = |pr: Rect| design::dot_meter_busy(&p, t, Rect::from_min_size(pos2(pr.left() + sp::S, pr.center().y - 1.5), vec2(METER, 3.0)));
    if sharpening && st.zoom.is_none() {
        let g = p.layout_job(design::micro_job(t!("rapida.enfocando"), t.faint));
        let w = sp::S + METER + sp::S + g.size().x + sp::S;
        let pr = Rect::from_min_size(pos2(rect.right() - FLOAT_GAP - w, rect.bottom() - FLOAT_GAP - h::CTL), vec2(w, h::CTL));
        pill(&p, t, pr);
        meter(pr);
        p.galley(pos2(pr.left() + sp::S + METER + sp::S, pr.center().y - g.size().y / 2.0), g, t.faint);
    }
    // Píldora de zoom, abajo a la derecha, solo si no está encajada.
    if let Some(z) = st.zoom {
        let label = format!("{:.0} %", z * 100.0);
        let tw = design::text_width(&p, &label, design::text::mono_s());
        let extra = if sharpening { METER + sp::S } else { 0.0 };
        let w = sp::S + extra + tw + sp::S + h::CHIP + sp::S;
        let pr = Rect::from_min_size(pos2(rect.right() - FLOAT_GAP - w, rect.bottom() - FLOAT_GAP - h::CTL), vec2(w, h::CTL));
        pill(&p, t, pr);
        if sharpening {
            meter(pr);
        }
        p.text(pos2(pr.left() + sp::S + extra, pr.center().y), Align2::LEFT_CENTER, label, design::text::mono_s(), t.text);
        let br = Rect::from_center_size(pos2(pr.right() - sp::S - h::CHIP / 2.0, pr.center().y), Vec2::splat(h::CHIP));
        let b = design::icon_button_at(ui, t, br, egui::Id::new("ql-fit"), ic::CORNERS_IN, IconBtn::mini());
        if design::tip(b, t, t!("rapida.encajar")).clicked() {
            (st.zoom, st.pan) = (None, Vec2::ZERO);
        }
    }
    gif_pill(ui, &p, t, rect, st, now);
}

/// Cambia el zoom de `z0` a `z1` dejando quieto el punto de la imagen bajo `m`.
fn zoom_at(st: &mut ViewState, center: Pos2, m: Pos2, z0: f32, z1: f32, fit: f32) {
    if z1 <= fit * 1.001 {
        (st.zoom, st.pan) = (None, Vec2::ZERO);
        return;
    }
    let c = center + st.pan;
    st.pan = (m - (m - c) * (z1 / z0)) - center;
    st.zoom = Some(z1);
}

/// Píldora del GIF, abajo al centro: pausa / reproducir, avance de fotogramas y «12 / 24». Con
/// el puntero encima y hasta 1,5 s después; siempre, si Windows tiene las animaciones apagadas.
fn gif_pill(ui: &mut Ui, p: &Painter, t: &Theme, rect: Rect, st: &mut ViewState, now: f64) {
    let Some(a) = st.anim.as_mut().filter(|a| a.frames.len() >= 2 && a.count > 1) else { return };
    let left = st.hover_at + motion::FEEDBACK - now;
    if left <= 0.0 && motion::on() {
        return;
    }
    if left > 0.0 {
        ui.ctx().request_repaint_after(secs(left));
    }
    let n = a.count;
    let meter_w = GIF_DOTS * 5.0 - 2.0;
    let tw = design::text_width(p, &format!("{n} / {n}"), design::text::mono_s());
    let w = sp::S + h::CHIP + sp::S + meter_w + sp::S + tw + sp::S;
    let pr = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - FLOAT_GAP - h::CTL / 2.0), vec2(w, h::CTL));
    pill(p, t, pr);
    let br = Rect::from_min_size(pos2(pr.left() + sp::S, pr.center().y - h::CHIP / 2.0), Vec2::splat(h::CHIP));
    let (glyph, tip) = if a.playing { (ic::PAUSE, t!("rapida.pausar")) } else { (ic::PLAY, t!("rapida.reproducir")) };
    let b = design::icon_button_at(ui, t, br, egui::Id::new("ql-gif-play"), glyph, IconBtn::mini());
    if design::tip(b, t, tip).clicked() {
        a.playing = !a.playing;
        a.next_at = now + a.delay(a.shown);
    }
    let mr = Rect::from_min_size(pos2(br.right() + sp::S, pr.center().y - 1.5), vec2(meter_w, 3.0));
    design::dot_meter(p, t, mr, (a.shown + 1) as f32 / n as f32, t.accent);
    p.text(pos2(mr.right() + sp::S, pr.center().y), Align2::LEFT_CENTER, format!("{} / {n}", a.shown + 1), design::text::mono_s(), t.dim);
}

/// Animación de un GIF: fotogramas que llegan de un hilo, cuál se ve y cuándo toca el siguiente.
struct Anim {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    /// Fotogramas que tiene el archivo (0 = todavía no se sabe).
    count: u32,
    frames: Vec<(Arc<ColorImage>, u32)>,
    done: bool,
    /// Una sola textura: se le sube el fotograma que toca.
    tex: Option<TextureHandle>,
    uploaded: Option<usize>,
    shown: usize,
    next_at: f64,
    playing: bool,
    started: bool,
}

enum Msg {
    Count(u32),
    Frame(Arc<ColorImage>, u32),
    Done,
}

impl Drop for Anim {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Anim {
    /// Decodifica en un hilo «gif»; corta en cuanto se suelta (`Drop`).
    fn start(ctx: egui::Context, path: String) -> Anim {
        let (tx, rx) = unbounded();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let _ = std::thread::Builder::new().name("gif".into()).spawn(move || {
            fsx::shell::com_init();
            let wake = || ctx.request_repaint_of(egui::ViewportId::ROOT);
            let n = anim::frame_count(&path).unwrap_or(0);
            let _ = tx.send(Msg::Count(n));
            wake();
            if n < 2 || stop.load(Ordering::Relaxed) {
                return;
            }
            anim::decode_gif(&path, anim::MAX_SIDE, anim::BUDGET, &stop, |f| {
                // `fsx::anim` no conoce egui: aquí se arma la imagen (y se suelta la copia).
                let img = Arc::new(ColorImage::from_rgba_premultiplied([f.w as usize, f.h as usize], &f.px));
                let _ = tx.send(Msg::Frame(img, f.delay_ms));
                wake();
            });
            let _ = tx.send(Msg::Done);
            wake();
        });
        Anim { rx, cancel, count: 0, frames: Vec::new(), done: false, tex: None, uploaded: None, shown: 0, next_at: 0.0, playing: false, started: false }
    }

    fn delay(&self, i: usize) -> f64 {
        self.frames.get(i).map_or(0.1, |f| f.1 as f64 / 1000.0)
    }
}

/// Arranca la decodificación (pasados `OPEN_DELAY`), recibe fotogramas, avanza según sus demoras
/// y sube a la textura el que toca. Pide el cuadro siguiente para cuando toque, nunca seguido.
fn anim_tick(ctx: &egui::Context, path: &str, st: &mut ViewState, now: f64) {
    let _p = super::perf::scope("ql.gif");
    if st.anim.is_none() {
        if st.anim_tried || fsx::is_online_only(path) {
            return;
        }
        let left = st.since + OPEN_DELAY - now;
        if left > 0.0 {
            ctx.request_repaint_after(secs(left));
            return;
        }
        st.anim_tried = true;
        st.anim = Some(Anim::start(ctx.clone(), path.to_string()));
    }
    let Some(a) = st.anim.as_mut() else { return };
    while let Ok(m) = a.rx.try_recv() {
        match m {
            Msg::Count(n) => a.count = n,
            Msg::Frame(img, d) => a.frames.push((img, d)),
            Msg::Done => a.done = true,
        }
    }
    let n = a.frames.len();
    if n < 2 {
        return;
    }
    if !a.started {
        // Con las animaciones de Windows apagadas, abre en pausa en el primer fotograma.
        a.started = true;
        a.playing = motion::on();
        a.next_at = now + a.delay(0);
    }
    if a.playing {
        let mut guard = 0;
        while now >= a.next_at && guard <= n {
            let next = if a.shown + 1 < n {
                a.shown + 1
            } else if a.done {
                0
            } else {
                // El siguiente todavía se está decodificando: el hilo avisa al llegar.
                break;
            };
            a.shown = next;
            a.next_at += a.delay(next);
            guard += 1;
        }
        // Tras una pausa larga (ventana tapada, sin cuadros): seguir desde ahora, sin correr.
        if now - a.next_at > 1.0 {
            a.next_at = now + a.delay(a.shown);
        }
        if a.next_at > now {
            ctx.request_repaint_after(secs(a.next_at - now));
        }
    }
    if a.uploaded != Some(a.shown) {
        let img = a.frames[a.shown].0.clone();
        match &mut a.tex {
            Some(x) => x.set(img, TextureOptions::LINEAR),
            None => a.tex = Some(ctx.load_texture("ql-gif", img, TextureOptions::LINEAR)),
        }
        a.uploaded = Some(a.shown);
    }
}

// ---------------------------------------------------------------- PDF

/// Páginas a lo ancho, una debajo de otra (ancho máximo `PDF_MAX`), tira de miniaturas a la
/// derecha y paginador flotante. Se piden solo las páginas a la vista y una de cada lado.
#[allow(clippy::too_many_arguments)]
fn pdf_body(ui: &mut Ui, t: &Theme, rect: Rect, item: &PreviewItem, tex: &mut Textures, cfg: &QuickLookCfg, st: &mut ViewState, now: f64) {
    let _p = super::perf::scope("ql.pdf");
    let src = item.source().to_string();
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let narrow = rect.width() < QL_NARROW;
    let side_for = |w: f32, h: f32| ((w.max(h) * ppp) as u32).clamp(256, PAGE_SIDE_MAX).next_multiple_of(256).min(PAGE_SIDE_MAX);
    // Cuántas páginas: las da la primera (o la miniatura de la lista, si ya estaba).
    if st.pages.is_none() {
        let strip_guess = if cfg.strip && !narrow { STRIP_W } else { 0.0 };
        let w = (rect.width() - strip_guess - 2.0 * sp::L).clamp(40.0, PDF_MAX);
        let first = tex.get_now(&src, side_for(w, w * 1.414)).map(|te| (te.pic.is_some(), te.pages, te.orig));
        match first {
            None => {
                let p = ui.painter_at(rect);
                well(&p, t, rect, CornerRadius::ZERO, true);
                waiting(ui, &p, t, rect, st, now, t!("rapida.cargando"));
                return;
            }
            Some((_, Some(n), orig)) if n > 0 => {
                st.pages = Some(n);
                st.ratio0 = orig.filter(|o| o.0 > 0).map_or(1.414, |(w, h)| h as f32 / w as f32);
            }
            // Protegido o roto: lo que dé Windows no es una página.
            _ => {
                no_preview(ui, t, rect, item, st, Some((Tone::Neutral, ic::WARNING, t!("rapida.pdf_no_abre"))));
                return;
            }
        }
    }
    let n = st.pages.unwrap_or(1).max(1);
    // Medidas de cada página, en segundo plano (mientras, todas como la primera).
    if n > 1 && st.sizes.is_empty() && st.sizes_rx.is_none() {
        let (tx, rx) = bounded(1);
        let (path, c) = (src.clone(), ctx.clone());
        let _ = std::thread::Builder::new().name("pdf-medidas".into()).spawn(move || {
            let _ = tx.send(pdf::sizes(&path));
            c.request_repaint_of(egui::ViewportId::ROOT);
        });
        st.sizes_rx = Some(rx);
    }
    if let Some(Ok(s)) = st.sizes_rx.as_ref().map(|rx| rx.try_recv()) {
        st.sizes = s.unwrap_or_default();
        st.sizes_rx = None;
    }
    let ratio = |i: usize| st.sizes.get(i).filter(|s| s.0 > 0.0).map_or(st.ratio0, |s| s.1 / s.0);
    let strip = cfg.strip && n > 1 && !narrow;
    let pages_r = if strip { Rect::from_min_max(rect.min, pos2(rect.right() - STRIP_W, rect.bottom())) } else { rect };
    let p = ui.painter_at(rect);
    well(&p, t, pages_r, CornerRadius::ZERO, true);
    // Diseño: ancho común, alto según cada página, 16 de margen y de separación.
    let pw = (pages_r.width() - 2.0 * sp::L).clamp(40.0, PDF_MAX);
    let mut tops = Vec::with_capacity(n as usize);
    let mut y = sp::L;
    for i in 0..n as usize {
        tops.push(y);
        y += pw * ratio(i) + sp::L;
    }
    let total = y;
    // Un salto pedido de afuera (← →, paginador, tira): `page` distinta de la que se mostró.
    let goto = (st.page != st.shown_page).then_some(st.page.min(n - 1) as usize);
    let mut cur = goto.unwrap_or(st.shown_page.min(n - 1) as usize);
    let mut area = egui::ScrollArea::vertical().id_salt(("ql-pdf", &item.path)).auto_shrink([false, false]);
    if let Some(g) = goto {
        area = area.vertical_scroll_offset(tops[g] - sp::L);
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pages_r));
    child.set_clip_rect(pages_r.intersect(ui.clip_rect()));
    area.show_viewport(&mut child, |ui, vp| {
        let (content, _) = ui.allocate_exact_size(vec2(pages_r.width(), total), Sense::hover());
        let x = content.center().x - pw / 2.0;
        let mut best = -1.0;
        for i in 0..n as usize {
            let (top, ph) = (tops[i], pw * ratio(i));
            // Las de afuera, salvo las vecinas de las visibles, no se piden.
            if top + ph < vp.top() - ph - sp::L || top > vp.bottom() + ph + sp::L {
                continue;
            }
            let r = Rect::from_min_size(pos2(x, content.top() + top), vec2(pw, ph));
            let seen = (top + ph).min(vp.bottom()) - top.max(vp.top());
            if goto.is_none() && seen > best {
                best = seen;
                cur = i;
            }
            let got = tex.get_now(&pdf::page_key(&src, i as u32), side_for(pw, ph)).and_then(|te| te.pic.clone());
            if seen > 0.0 {
                match got {
                    Some(pic) => image(ui.painter(), t, &pic, true, r, 1.0),
                    None => blank_page(ui.painter(), t, r),
                }
            }
        }
    });
    let mut jump = None;
    // Tira de miniaturas: 128, `panel`, hairline a la izquierda; la actual con el anillo de acento.
    if strip {
        let sr = Rect::from_min_max(pos2(rect.right() - STRIP_W, rect.top()), rect.max);
        p.rect_filled(sr, CornerRadius::ZERO, t.chrome);
        p.vline(sr.left() + stroke::HAIR / 2.0, sr.y_range(), Stroke::new(stroke::HAIR, t.border));
        let num_h = p.layout_no_wrap("0".into(), design::text::mono_s(), t.faint).size().y;
        let mut ys = Vec::with_capacity(n as usize);
        let mut y = sp::M;
        for i in 0..n as usize {
            ys.push(y);
            y += STRIP_THUMB * ratio(i) + 6.0 + num_h + sp::M;
        }
        let total = y;
        let follow = st.strip_page != Some(cur as u32);
        let mut sc = ui.new_child(egui::UiBuilder::new().max_rect(sr));
        sc.set_clip_rect(sr.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt(("ql-strip", &item.path)).auto_shrink([false, false]).show_viewport(&mut sc, |ui, vp| {
            let (content, _) = ui.allocate_exact_size(vec2(STRIP_W, total), Sense::hover());
            let x = content.center().x - STRIP_THUMB / 2.0;
            for i in 0..n as usize {
                let th = STRIP_THUMB * ratio(i);
                let r = Rect::from_min_size(pos2(x, content.top() + ys[i]), vec2(STRIP_THUMB, th));
                if follow && i == cur {
                    ui.scroll_to_rect(r.expand(sp::M), None);
                }
                if ys[i] + th + num_h + 6.0 < vp.top() || ys[i] > vp.bottom() {
                    continue;
                }
                let resp = ui.interact(r, egui::Id::new(("ql-strip-page", &st.path, i)), Sense::click());
                let painter = ui.painter();
                match tex.get(&pdf::page_key(&src, i as u32), STRIP_TEX).and_then(|te| te.pic.clone()) {
                    Some(pic) => image(painter, t, &pic, true, r, 1.0),
                    None => blank_page(painter, t, r),
                }
                let on = i == cur;
                if on {
                    painter.rect_stroke(r, radius::S, Stroke::new(stroke::FOCUS, t.accent), StrokeKind::Outside);
                } else if resp.hovered() {
                    painter.rect_stroke(r, radius::S, Stroke::new(stroke::HAIR, t.faint), StrokeKind::Outside);
                }
                painter.text(pos2(r.center().x, r.bottom() + 6.0), Align2::CENTER_TOP, (i + 1).to_string(), design::text::mono_s(), if on { t.text } else { t.faint });
                if resp.clicked() {
                    jump = Some(i);
                }
            }
        });
        st.strip_page = Some(cur as u32);
    }
    // Paginador flotante: ‹ 3 / 12 ›, abajo al centro del área de páginas.
    if n > 1 {
        let label = format!("{} / {n}", cur + 1);
        let tw = design::text_width(&p, &format!("{n} / {n}"), design::text::mono());
        let w = sp::XS + h::CHIP + sp::XS + sp::XS + tw + sp::XS + sp::XS + h::CHIP + sp::XS;
        let pr = Rect::from_center_size(pos2(pages_r.center().x, pages_r.bottom() - FLOAT_GAP - h::CTL / 2.0), vec2(w, h::CTL));
        pill(&p, t, pr);
        p.text(pr.center(), Align2::CENTER_CENTER, label, design::text::mono(), t.dim);
        for (dx, glyph, d, tip) in [(-1.0, ic::CARET_LEFT, -1i64, t!("rapida.pagina_anterior")), (1.0, ic::CARET_RIGHT, 1, t!("rapida.pagina_siguiente"))] {
            let c = pos2(pr.center().x + dx * (w / 2.0 - sp::XS - h::CHIP / 2.0), pr.center().y);
            let to = cur as i64 + d;
            let enabled = (0..n as i64).contains(&to);
            let b = design::icon_button_at(ui, t, Rect::from_center_size(c, Vec2::splat(h::CHIP)), egui::Id::new(("ql-page", d)), glyph, IconBtn::nav(enabled));
            if design::tip(b, t, tip).clicked() {
                jump = Some(to as usize);
            }
        }
    }
    st.shown_page = cur as u32;
    st.page = cur as u32;
    if let Some(j) = jump {
        st.page = j as u32;
        ctx.request_repaint();
    }
}
