//! Vistas de contenido: columnas (estilo Finder), detalles y resultados de búsqueda.

use super::actions::Action;
use super::icons;
use super::config::ColumnWidths;
use super::state::{Col, Results};
use crate::index::ResultSort;
use super::theme::{self, Theme};
use crate::fsx::{self, FileEntry, SortKey};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use std::sync::Arc;

pub const ROW_H: f32 = 26.0;

/// Contexto compartido al dibujar.
pub struct Cx<'a> {
    pub t: &'a Theme,
    pub now: u32,
    pub actions: &'a mut Vec<Action>,
    pub rename: &'a mut Option<Rename>,
    pub cut: &'a [String],
    pub tab_id: u64,
    pub groups: &'a [String],
    /// Barras de proporción en la columna Tamaño ("qué ocupa espacio").
    pub space_bars: bool,
    /// Hay dos paneles (para "enviar al otro panel").
    pub split: bool,
    /// Dónde está ahora el resaltado de la selección, que se desliza de fila en fila (si hay uno
    /// solo seleccionado). Las filas no dibujan el suyo y sus letras cambian de color según cuánto
    /// las cubre.
    pub pill: Option<Rect>,
}

/// Archivos que se están arrastrando (rutas completas).
#[derive(Clone, Debug)]
pub struct DragFiles(pub Vec<String>);

/// Arrastre que viene de otra aplicación (Explorador, navegador…): dónde está el
/// puntero y, al soltar, las rutas. Lo llena la app al comienzo de cada cuadro.
#[derive(Clone, Default)]
pub struct ExternalDrag {
    pub pos: Option<Pos2>,
    pub dropped: Vec<String>,
}

pub fn external_id() -> Id {
    Id::new("external-drag")
}

/// ¿Hay archivos arrastrándose sobre esta zona (desde Rayo o desde otra app)?
pub fn drag_hovering(resp: &egui::Response) -> bool {
    resp.dnd_hover_payload::<DragFiles>().is_some()
        || resp.ctx.data(|d| d.get_temp::<ExternalDrag>(external_id())).and_then(|e| e.pos).is_some_and(|p| resp.interact_rect.contains(p))
}

/// Archivos soltados sobre esta zona (desde Rayo o desde otra app).
pub fn take_drop(resp: &egui::Response) -> Option<Vec<String>> {
    if let Some(files) = resp.dnd_release_payload::<DragFiles>() {
        return Some(files.0.clone());
    }
    resp.ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<ExternalDrag>(external_id());
        (!e.dropped.is_empty() && e.pos.is_some_and(|p| resp.interact_rect.contains(p))).then(|| std::mem::take(&mut e.dropped))
    })
}

/// Ctrl fuerza copiar y Mayús fuerza mover; si no, decide el disco (como el Explorador).
pub fn drop_force() -> Option<bool> {
    use crate::fsx::shell::key_down;
    if key_down(0x11) {
        Some(false)
    } else if key_down(0x10) {
        Some(true)
    } else {
        None
    }
}

/// Si se soltaron archivos sobre `resp`, pide copiarlos o moverlos a `dest`.
fn accept_drop(cx: &mut Cx, resp: &egui::Response, dest: &str) {
    if let Some(files) = take_drop(resp) {
        // No soltar una carpeta dentro de sí misma.
        let paths: Vec<String> = files.iter().filter(|p| !p.eq_ignore_ascii_case(dest)).cloned().collect();
        if !paths.is_empty() {
            cx.actions.push(Action::DropFiles { paths, dest: dest.to_string(), force: drop_force() });
        }
    }
}

/// Marca visual de "se puede soltar aquí".
fn drop_highlight(ui: &Ui, t: &Theme, resp: &egui::Response, rect: Rect) {
    if drag_hovering(resp) {
        ui.painter().rect_stroke(rect.shrink(2.0), 6.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        // Una carpeta bajo el puntero tiene prioridad sobre el fondo.
        let pass = ui.ctx().cumulative_pass_nr();
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("row-drop-hover"), pass));
    }
}

/// El fondo se marca y acepta lo soltado solo si no hay una carpeta debajo del puntero.
fn background_drop(ui: &Ui, cx: &mut Cx, bg: &egui::Response, rect: Rect, dir: &str) {
    let pass = ui.ctx().cumulative_pass_nr();
    if ui.ctx().data(|d| d.get_temp::<u64>(Id::new("row-drop-hover"))) == Some(pass) {
        return;
    }
    let t = *cx.t;
    drop_highlight(ui, &t, bg, rect);
    accept_drop(cx, bg, dir);
}

/// Indicador del filtro escrito sobre la carpeta (Esc lo borra).
pub fn filter_chip(ui: &mut Ui, t: &Theme, filter: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::Frame::new().fill(t.accent.gamma_multiply(0.15)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
            ui.label(egui::RichText::new(format!("{}  {}", ic::FUNNEL, tf!("lista.filtrando", filtro = filter))).size(12.0).color(t.accent));
        });
        ui.label(egui::RichText::new(t!("lista.esc_quitar")).size(11.0).color(t.faint));
    });
    ui.add_space(2.0);
}

/// Indicador de la vista plana (clic en la X para salir).
pub fn flat_chip(ui: &mut Ui, t: &Theme, n: usize, loading: bool, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        egui::Frame::new().fill(t.accent.gamma_multiply(0.15)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
            let txt = if loading { t!("lista.vista_plana_juntando_archivos").to_string() } else { format!("{}  {}", ic::STACK, tp!(n, "lista.vista_plana_archivos_todas_subcarpetas", n = fsx::thousands(n))) };
            ui.label(egui::RichText::new(txt).size(12.0).color(t.accent));
            if ui.add(egui::Button::new(egui::RichText::new(ic::X).size(11.0).color(t.accent)).frame(false)).on_hover_text(t!("lista.salir_vista_plana_esc")).clicked() {
                actions.push(Action::ToggleFlat);
            }
        });
    });
    ui.add_space(2.0);
}

pub struct Rename {
    pub dir: String,
    pub old: String,
    pub buf: String,
    pub first_frame: bool,
}

pub fn elided(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(max_w.max(10.0));
    p.layout_job(job)
}

/// Texto con la parte que coincide resaltada.
fn highlighted(p: &Painter, text: &str, needle: &str, t: &Theme, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    let font = FontId::proportional(13.5);
    let mut job = LayoutJob::default();
    // La coincidencia se busca en la clave (sin acentos) y se traduce a bytes del original.
    let (key, map) = crate::index::store::search_key_with_map(text);
    let pos = if needle.is_empty() { None } else { key.find(needle).map(|k| (map[k], map[k + needle.len()])) };
    match pos {
        Some((s, e)) if s < e => {
            job.append(&text[..s], 0.0, TextFormat::simple(font.clone(), color));
            let mut hl = TextFormat::simple(theme::semibold(13.5), t.accent);
            hl.background = t.accent.gamma_multiply(0.18);
            job.append(&text[s..e], 0.0, hl);
            job.append(&text[e..], 0.0, TextFormat::simple(font, color));
        }
        _ => job.append(text, 0.0, TextFormat::simple(font, color)),
    }
    job.wrap = TextWrapping::truncate_at_width(max_w.max(10.0));
    p.layout_job(job)
}

/// Fondo de una fila: la selección, o el hover (`hover` de 0 a 1: aparece y se apaga con un fundido).
pub fn row_bg(p: &Painter, rect: Rect, t: &Theme, selected: bool, active: bool, hover: f32) {
    let r = rect.shrink2(vec2(4.0, 1.0));
    if selected {
        p.rect_filled(r, 5.0, if active { t.sel } else { t.sel_inactive });
    } else if hover > 0.01 {
        p.rect_filled(r, 5.0, t.hover.gamma_multiply(hover));
    }
}

/// Cuánto se ve el hover de este elemento (entra rápido, se apaga un poco más lento: al pasar el
/// mouse por una lista deja una estela corta).
pub fn hover_amount(resp: &egui::Response, show: bool) -> f32 {
    let on = show && resp.hovered();
    super::motion::fade(&resp.ctx, resp.id.with("hover"), on, if on { 0.07 } else { 0.16 })
}

/// Cuánto cubre el resaltado deslizante a una fila (0 a 1).
fn pill_cover(pill: Rect, r: Rect) -> f32 {
    let fx = 1.0 - ((pill.center().x - r.center().x).abs() / pill.width().max(1.0));
    let fy = 1.0 - ((pill.center().y - r.center().y).abs() / pill.height().max(1.0));
    fx.clamp(0.0, 1.0) * fy.clamp(0.0, 1.0)
}

/// Colores del texto de una fila (nombre, datos): con el resaltado deslizante, pasan de normales a
/// los de seleccionado a medida que este llega.
fn row_fg(t: &Theme, pill: Option<Rect>, r: Rect, selected: bool, active: bool) -> (Color32, Color32, bool) {
    match pill {
        Some(p) if active => {
            let k = pill_cover(p, r);
            let (a, b) = fg(t, false, false);
            let (c, d) = fg(t, true, true);
            (a.lerp_to_gamma(c, k), b.lerp_to_gamma(d, k), k > 0.5)
        }
        Some(_) => {
            let (a, b) = fg(t, false, false);
            (a, b, false)
        }
        None => {
            let (a, b) = fg(t, selected, active);
            (a, b, selected && active)
        }
    }
}

/// El resaltado que se desliza a la fila `i` (en coordenadas de la lista, que ya incluyen el desplazamiento).
fn pill_rect(ui: &Ui, id: Id, target: Rect) -> Rect {
    let ctx = ui.ctx();
    let x = super::motion::glide(ctx, id.with("px"), target.left() - ui.max_rect().left(), 0.11);
    let y = super::motion::glide(ctx, id.with("py"), target.top() - ui.max_rect().top(), 0.11);
    Rect::from_min_size(ui.max_rect().min + vec2(x, y), target.size())
}

/// Ícono del estado en la nube (OneDrive): solo en la nube, descargado, siempre en el equipo.
pub fn cloud_glyph(t: &Theme, c: fsx::Cloud) -> Option<(&'static str, Color32, &'static str)> {
    match c {
        fsx::Cloud::None => None,
        fsx::Cloud::OnlineOnly => Some((ic::CLOUD, t.folder, t!("lista.solo_nube_descarga_abrirlo"))),
        fsx::Cloud::Local => Some((ic::CLOUD_CHECK, t.dim, t!("lista.disponible_este_equipo_sincronizado"))),
        fsx::Cloud::Pinned => Some((egui_phosphor::fill::CHECK_CIRCLE, Color32::from_rgb(0x3F, 0xB9, 0x50), t!("lista.siempre_este_equipo"))),
    }
}

fn fg(t: &Theme, selected: bool, active: bool) -> (Color32, Color32) {
    if selected && active { (t.accent_text, t.accent_text.gamma_multiply(0.8)) } else { (t.text, t.dim) }
}

/// Lista virtualizada: solo se dibujan las filas visibles.
/// `glide`: fila del resaltado deslizante y su color (si hay una sola seleccionada); a cada fila
/// se le pasa dónde está ese resaltado.
fn virtual_list(ui: &mut Ui, id: Id, n: usize, scroll_to: Option<usize>, glide: Option<(usize, Color32)>, mut row: impl FnMut(&mut Ui, usize, Rect, Option<Rect>)) -> egui::scroll_area::ScrollAreaOutput<()> {
    let out = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        ui.set_height(ROW_H * n as f32 + 6.0);
        let top = ui.max_rect().top();
        let left = ui.max_rect().left();
        if let Some(i) = scroll_to {
            let r = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            ui.scroll_to_rect(r.expand2(vec2(0.0, ROW_H * 0.5)), None);
        }
        let pill = glide.map(|(i, color)| {
            let target = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            let r = pill_rect(ui, id, target);
            ui.painter().rect_filled(r.shrink2(vec2(4.0, 1.0)), 5.0, color);
            r
        });
        let first = (viewport.min.y / ROW_H).floor().max(0.0) as usize;
        let last = ((viewport.max.y / ROW_H).ceil() as usize + 1).min(n);
        for i in first..last {
            let r = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            row(ui, i, r, pill);
        }
    });
    out
}

// ---------------------------------------------------------------- selección por rectángulo

#[derive(Clone)]
struct Marquee {
    /// Punto de inicio en coordenadas del contenido (no cambia al desplazarse).
    origin: Pos2,
    /// Lo que ya estaba seleccionado (con Ctrl o Mayús se suma).
    base: Vec<String>,
    last: Option<Vec<String>>,
}

/// Dónde empezó el arrastre que inicia un rectángulo (si `resp` lo empezó en este cuadro).
fn drag_origin(resp: &egui::Response) -> Option<Pos2> {
    if resp.drag_started_by(egui::PointerButton::Primary) { resp.ctx.input(|i| i.pointer.press_origin()) } else { None }
}

/// Rectángulo de selección: se dibuja, selecciona lo que toca y desplaza la lista cerca de los bordes.
/// `hits` recibe el rectángulo en coordenadas del contenido y devuelve los nombres que toca.
fn marquee(ui: &Ui, cx: &mut Cx, ci: usize, start: Option<Pos2>, base: Vec<String>, scroll: &egui::scroll_area::ScrollAreaOutput<()>, hits: impl Fn(Rect) -> Vec<String>) {
    let ctx = ui.ctx().clone();
    let id = Id::new(("marquee", cx.tab_id, ci));
    let origin = scroll.inner_rect.min - scroll.state.offset;
    if let Some(p) = start {
        let add = ctx.input(|i| i.modifiers.ctrl || i.modifiers.shift);
        let m = Marquee { origin: pos2(p.x - origin.x, p.y - origin.y), base: if add { base } else { Vec::new() }, last: None };
        ctx.data_mut(|d| d.insert_temp(id, m));
    }
    let Some(mut m) = ctx.data(|d| d.get_temp::<Marquee>(id)) else { return };
    let (down, pos) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
    let Some(pos) = pos.filter(|_| down) else {
        ctx.data_mut(|d| d.remove::<Marquee>(id));
        return;
    };
    let rect = Rect::from_two_pos(m.origin, pos2(pos.x - origin.x, pos.y - origin.y));
    let mut names = m.base.clone();
    for n in hits(rect) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if m.last.as_ref() != Some(&names) {
        cx.actions.push(Action::SetSelection { col: ci, names: names.clone() });
        m.last = Some(names);
    }
    let vp = scroll.inner_rect;
    let screen = rect.translate(origin.to_vec2());
    let p = ui.painter_at(vp);
    p.rect_filled(screen, 3.0, cx.t.accent.gamma_multiply(0.14));
    p.rect_stroke(screen, 3.0, egui::Stroke::new(1.0, cx.t.accent.gamma_multiply(0.85)), egui::StrokeKind::Inside);
    // Cerca de los bordes (o fuera), la lista se desplaza sola.
    let edge = 28.0;
    let dy = if pos.y < vp.top() + edge {
        -(vp.top() + edge - pos.y).min(80.0) * 0.4
    } else if pos.y > vp.bottom() - edge {
        (pos.y - (vp.bottom() - edge)).min(80.0) * 0.4
    } else {
        0.0
    };
    if dy != 0.0 {
        let mut st = scroll.state;
        let max = (scroll.content_size.y - vp.height()).max(0.0);
        st.offset.y = (st.offset.y + dy).clamp(0.0, max);
        st.store(&ctx, scroll.id);
    }
    ctx.data_mut(|d| d.insert_temp(id, m));
    ctx.request_repaint();
}

/// Filas de una lista que toca el rectángulo (en coordenadas del contenido).
fn list_hits(entries: &[FileEntry], r: Rect) -> Vec<String> {
    let n = entries.len();
    if n == 0 || r.max.y < 0.0 || r.min.y >= ROW_H * n as f32 {
        return Vec::new();
    }
    let a = (r.min.y / ROW_H).floor().max(0.0) as usize;
    let b = ((r.max.y / ROW_H).floor() as usize).min(n - 1);
    entries[a..=b].iter().map(|e| e.name.clone()).collect()
}

/// Clic en un espacio vacío: deselecciona (salvo con Ctrl o Mayús).
/// La fila del resaltado deslizante: la del cursor, si es lo único seleccionado.
fn glide_row(t: &Theme, col: &Col, entries: &[FileEntry], active: bool) -> Option<(usize, Color32)> {
    if !col.multi.is_empty() {
        return None;
    }
    let c = col.cursor.as_ref()?;
    let i = entries.iter().position(|e| &e.name == c)?;
    Some((i, if active { t.sel } else { t.sel_inactive }))
}

fn background_click(cx: &mut Cx, bg: &egui::Response, ci: usize) {
    if bg.clicked() && !bg.ctx.input(|i| i.modifiers.ctrl || i.modifiers.shift) {
        cx.actions.push(Action::SetSelection { col: ci, names: Vec::new() });
    }
}

fn empty_message(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(18.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(text).color(t.faint).size(12.5));
    });
}

/// Si un nombre nuevo cambia la extensión: (la de antes, la nueva), como se muestran (".pdf",
/// o "sin extensión"). Las carpetas no tienen extensión que cuidar.
pub fn ext_change(old: &str, new: &str) -> Option<(String, String)> {
    let ext = |n: &str| {
        let n = fsx::file_name(n);
        n.rfind('.').filter(|&i| i > 0 && i + 1 < n.len()).map(|i| n[i..].to_lowercase())
    };
    let (a, b) = (ext(old), ext(new.trim()));
    if a == b {
        return None;
    }
    let show = |e: Option<String>| e.unwrap_or_else(|| t!("lista.sin_extension").to_string());
    Some((show(a), show(b)))
}

/// Fila de renombrado en línea.
fn rename_row(ui: &mut Ui, cx: &mut Cx, rect: Rect, x: f32) {
    let Some(rn) = cx.rename.as_mut() else { return };
    // Tan ancho como el nombre (con aire para escribir), no toda la fila.
    let text_w = ui.painter().layout_no_wrap(rn.buf.clone(), egui::FontId::proportional(13.5), cx.t.text).size().x;
    let right = (x + (text_w + 60.0).max(220.0)).min(rect.right() - 8.0);
    let edit_rect = Rect::from_min_max(pos2(x, rect.top() + 2.0), pos2(right, rect.bottom() - 2.0));
    ui.painter().rect_filled(edit_rect, 5.0, cx.t.bg);
    ui.painter().rect_stroke(edit_rect, 5.0, egui::Stroke::new(1.5, cx.t.accent), egui::StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect.shrink2(vec2(6.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let id = Id::new("rename-edit");
    let out = egui::TextEdit::singleline(&mut rn.buf).id(id).frame(egui::Frame::NONE).desired_width(edit_rect.width() - 12.0).vertical_align(egui::Align::Center).show(&mut child);
    if rn.first_frame {
        out.response.request_focus();
        let stem = rn.buf.rfind('.').filter(|&i| i > 0).unwrap_or(rn.buf.len());
        let chars = rn.buf[..stem].chars().count();
        let mut state = out.state.clone();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
        state.store(ui.ctx(), id);
        rn.first_frame = false;
    }
    // Cambiar la extensión puede dejar el archivo sin poder abrirse: se avisa al lado, sin el
    // cuadro de diálogo de Windows (y se deshace con Ctrl+Z).
    if let Some((a, b)) = ext_change(&rn.old, &rn.buf) {
        if !std::path::Path::new(&fsx::long_path(&fsx::join(&rn.dir, &rn.old))).is_dir() {
            let warn = Color32::from_rgb(0xF5, 0x9E, 0x42);
            let long = tf!("lista.cambia_extension", antes = a, despues = b);
            let room = rect.right() - edit_rect.right() - 16.0;
            let font = FontId::proportional(12.0);
            let g = ui.painter().layout_no_wrap(format!("{}  {long}", ic::WARNING), font.clone(), warn);
            let g = if g.size().x <= room { g } else { ui.painter().layout_no_wrap(format!("{}  {a} → {b}", ic::WARNING), font, warn) };
            if g.size().x <= room {
                ui.painter().galley(pos2(edit_rect.right() + 10.0, rect.center().y - g.size().y / 2.0), g, warn);
            }
        }
    }
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if esc {
        *cx.rename = None;
    } else if enter || out.response.lost_focus() {
        cx.actions.push(Action::CommitRename);
    }
}

/// Dibuja una fila de archivo genérica (ícono + nombre + texto a la derecha).
#[allow(clippy::too_many_arguments)]
/// El nombre de una etiqueta de color, en el idioma de la interfaz.
pub fn tag_label(i: usize) -> &'static str {
    match i {
        0 => t!("etiqueta.rojo"),
        1 => t!("etiqueta.naranja"),
        2 => t!("etiqueta.amarillo"),
        3 => t!("etiqueta.verde"),
        4 => t!("etiqueta.azul"),
        5 => t!("etiqueta.morado"),
        _ => t!("etiqueta.gris"),
    }
}

pub fn tag_color(i: usize) -> Color32 {
    let c = fsx::tags::COLORS[i.min(6)].1;
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Los puntos de color de las etiquetas de un archivo, desde `x` (centrados en `cy`). Devuelve
/// cuánto ocupan.
fn tag_dots(p: &Painter, t: &Theme, x: f32, cy: f32, mask: u8) -> f32 {
    let mut w = 0.0;
    for i in (0..7).filter(|i| mask & (1 << i) != 0) {
        let c = pos2(x + w + 4.5, cy);
        // Con un borde del color de fondo, para que se distingan si se tocan.
        p.circle(c, 4.5, tag_color(i), egui::Stroke::new(1.0, t.bg));
        w += 7.0;
    }
    if w > 0.0 { w + 2.0 } else { 0.0 }
}

/// El ícono de un archivo: el de Windows si ya llegó (el de cada programa, o el de su tipo);
/// mientras tanto, y para las carpetas de las listas, el de Rayo. `glyph`: tamaño del de Rayo.
#[allow(clippy::too_many_arguments)]
fn file_icon(p: &Painter, t: &Theme, center: Pos2, points: f32, glyph: f32, path: &str, is_dir: bool, ext: &str, color: Option<Color32>, alpha: f32) {
    // Una descarga en curso: la flecha de descarga (no el ícono de un archivo a medias).
    if !is_dir && fsx::downloading(&fsx::file_name(path)).is_some() {
        p.text(center, Align2::CENTER_CENTER, ic::DOWNLOAD_SIMPLE, theme::icon_font(glyph, true), color.unwrap_or(t.accent).gamma_multiply(alpha));
        return;
    }
    if !is_dir {
        let px = super::shellicons::px_for(p.ctx(), points);
        if let Some((tex, shown)) = super::shellicons::get(path, ext, px) {
            // Recién llegado: el de Rayo se desvanece mientras aparece el de Windows.
            if shown < 1.0 {
                let (g, c) = icons::for_entry(t, is_dir, ext);
                p.text(center, Align2::CENTER_CENTER, g, theme::icon_font(glyph, true), color.unwrap_or(c).gamma_multiply(alpha * (1.0 - shown)));
            }
            // Alineado a los píxeles de la pantalla y del tamaño exacto: nítido, como en el Explorador.
            let ppp = p.ctx().pixels_per_point();
            let side = px as f32 / ppp;
            let min = center - vec2(side, side) / 2.0;
            let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            let r = Rect::from_min_size(min, vec2(side, side));
            p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha * shown));
            return;
        }
    }
    let (g, c) = icons::for_entry(t, is_dir, ext);
    p.text(center, Align2::CENTER_CENTER, g, theme::icon_font(glyph, true), color.unwrap_or(c).gamma_multiply(alpha));
}

/// Si el mouse se detiene un momento sobre una carpeta (o el cursor queda en ella), se lee antes de
/// entrar: al abrirla, su contenido ya está.
fn prefetch(cx: &mut Cx, resp: &egui::Response, path: &str, cursor: bool) {
    let ctx = &resp.ctx;
    let key = Id::new("prefetch-dwell");
    let now = ctx.input(|i| i.time);
    // Con el teclado: la carpeta del cursor, sin esperar (una vez).
    let ckey = Id::new("prefetch-cursor");
    if cursor && ctx.data(|d| d.get_temp::<Id>(ckey)) != Some(resp.id) {
        ctx.data_mut(|d| d.insert_temp(ckey, resp.id));
        cx.actions.push(Action::Prefetch(path.to_string()));
    }
    if !resp.hovered() {
        return;
    }
    let (who, since, sent): (Id, f64, bool) = ctx.data(|d| d.get_temp(key)).unwrap_or((Id::NULL, 0.0, false));
    if who != resp.id {
        ctx.data_mut(|d| d.insert_temp(key, (resp.id, now, false)));
        ctx.request_repaint_after(std::time::Duration::from_millis(90));
        return;
    }
    if !sent && now - since >= 0.08 {
        cx.actions.push(Action::Prefetch(path.to_string()));
        ctx.data_mut(|d| d.insert_temp(key, (who, since, true)));
    } else if !sent {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

fn file_row(ui: &mut Ui, cx: &mut Cx, rect: Rect, e: &FileEntry, dir: &str, selected: bool, active: bool, right: Option<&str>, chevron: bool, name_limit: Option<f32>) -> egui::Response {
    let id = Id::new(("row", cx.tab_id, dir, &e.name));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    if e.is_dir {
        prefetch(cx, &resp, &fsx::join(dir, &e.name), selected && active);
    }
    let p = ui.painter_at(rect);
    let pill = cx.pill;
    // Con el resaltado deslizante, la fila no dibuja su selección (ya está debajo).
    let hover = hover_amount(&resp, !selected || pill.is_some_and(|pl| pill_cover(pl, rect) < 0.5));
    row_bg(&p, rect, cx.t, selected && pill.is_none(), active, hover);
    if e.is_dir {
        drop_highlight(ui, cx.t, &resp, rect);
    }
    let (text_c, dim_c, on_sel) = row_fg(cx.t, pill, rect, selected, active);
    let icolor = on_sel.then_some(cx.t.accent_text);
    let path = fsx::join(dir, &e.name);
    let faded = e.hidden || cx.cut.contains(&path);
    let alpha = if faded { 0.5 } else { 1.0 };
    file_icon(&p, cx.t, pos2(rect.left() + 22.0, rect.center().y), 16.0, 16.0, &path, e.is_dir, &e.ext, icolor, alpha);
    let name_x = rect.left() + 36.0;
    let mut right_w = 12.0;
    if chevron {
        p.text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, ic::CARET_RIGHT, theme::icon_font(11.0, false), dim_c);
        right_w += 14.0;
    }
    if let Some(rt) = right {
        let g = p.layout_no_wrap(rt.to_string(), FontId::proportional(11.5), dim_c);
        let w = g.size().x;
        p.galley(pos2(rect.right() - right_w - w, rect.center().y - g.size().y / 2.0), g, dim_c);
        right_w += w + 8.0;
    }
    // Estado en la nube: al final del nombre (en detalles, al final de su columna).
    let mut name_limit = name_limit;
    if let Some((g, c, tip)) = cloud_glyph(cx.t, e.cloud) {
        let c = if on_sel { cx.t.accent_text } else { c };
        let x = match name_limit {
            Some(l) => {
                name_limit = Some(l - 20.0);
                l - 4.0
            }
            None => {
                let x = rect.right() - right_w;
                right_w += 20.0;
                x
            }
        };
        let ir = Rect::from_center_size(pos2(x - 8.0, rect.center().y), vec2(16.0, 16.0));
        p.text(ir.center(), Align2::CENTER_CENTER, g, theme::icon_font(13.0, false), c);
        if resp.hovered() && ui.rect_contains_pointer(ir) {
            resp.clone().on_hover_text(tip);
        }
    }
    let renaming = cx.rename.as_ref().is_some_and(|r| r.dir == dir && r.old == e.name);
    if renaming {
        rename_row(ui, cx, rect, name_x - 4.0);
    } else {
        let limit = name_limit.unwrap_or(f32::INFINITY).min(rect.right() - right_w);
        match fsx::downloading(&e.name).filter(|_| !e.is_dir) {
            // Descarga en curso: el nombre que tendrá y "Descargando…".
            Some(base) => {
                let tag = p.layout_no_wrap(t!("lista.descargando").to_string(), FontId::proportional(12.0), if on_sel { text_c } else { cx.t.accent });
                let g = elided(&p, base, FontId::proportional(13.5), text_c.gamma_multiply(alpha), (limit - name_x - tag.size().x - 10.0).max(20.0));
                let gw = g.size().x;
                p.galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, text_c);
                p.galley(pos2(name_x + gw + 10.0, rect.center().y - tag.size().y / 2.0), tag, text_c);
            }
            None => {
                let mask = fsx::tags::get(&path);
                let dots = if mask == 0 { 0.0 } else { 7.0 * mask.count_ones() as f32 + 10.0 };
                let g = elided(&p, &e.name, FontId::proportional(13.5), text_c.gamma_multiply(alpha), limit - name_x - dots);
                let gw = g.size().x;
                p.galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, text_c);
                if mask != 0 {
                    tag_dots(&p, cx.t, name_x + gw + 6.0, rect.center().y, mask);
                }
            }
        }
    }
    resp
}

/// La fila del cursor a la que hay que llevar la vista. Si todavía no está en la lista (un archivo
/// recién creado, p. ej. al "mostrar en carpeta" una descarga), se sigue esperando mientras se lista.
fn scroll_target(col: &mut Col, entries: &[FileEntry], loading: bool) -> Option<usize> {
    if !col.scroll_to_cursor {
        return None;
    }
    let found = col.cursor.as_ref().and_then(|c| entries.iter().position(|e| &e.name == c));
    if found.is_some() || !loading {
        col.scroll_to_cursor = false;
        if found.is_none() {
            crate::diag::log(&format!("no se encontró «{}» en «{}» ({} elementos)", col.cursor.as_deref().unwrap_or(""), col.path, entries.len()));
        }
    }
    found
}

/// Cuenta de clics como la de Windows: dos clics seguidos (dentro del tiempo y la distancia de un
/// doble clic) forman un par, y el clic siguiente empieza de cero. egui, en cambio, cuenta un
/// tercer clic rápido como triple (y no como doble): seleccionar algo y enseguida hacerle doble
/// clic no lo abría.
#[derive(Clone, Copy, Default)]
struct Clicks {
    /// Hora de la entrada ya contada (otra pasada del mismo cuadro no vuelve a contarla).
    time: f64,
    /// El último clic, si todavía puede formar par.
    last: Option<(f64, Pos2)>,
    double: bool,
}

fn clicks_id() -> Id {
    Id::new("clics-windows")
}

/// Se llama una vez por cuadro, antes de dibujar.
pub fn track_clicks(ctx: &egui::Context) {
    let (clicked, pos, now) = ctx.input(|i| (i.pointer.button_clicked(egui::PointerButton::Primary), i.pointer.interact_pos(), i.time));
    let (delay, dist) = ctx.options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_dist));
    let mut c = ctx.data(|d| d.get_temp::<Clicks>(clicks_id())).unwrap_or_default();
    if c.time == now {
        return;
    }
    c.time = now;
    c.double = false;
    if let (true, Some(pos)) = (clicked, pos) {
        match c.last {
            Some((t, p)) if now - t < delay && p.distance(pos) < dist => {
                c.double = true;
                c.last = None;
            }
            _ => c.last = Some((now, pos)),
        }
    }
    ctx.data_mut(|d| d.insert_temp(clicks_id(), c));
}

/// Doble clic sobre este elemento, contado como en Windows (ver `track_clicks`).
pub fn double_clicked(resp: &egui::Response) -> bool {
    resp.clicked() && resp.ctx.data(|d| d.get_temp::<Clicks>(clicks_id())).is_some_and(|c| c.double)
}

/// Un renombrado "lento" en espera: clic sobre el nombre de lo que ya estaba seleccionado.
#[derive(Clone)]
struct SlowRename {
    row: Id,
    dir: String,
    name: String,
    at: f64,
}

/// Renombrar con un clic lento, como en todo Windows: clic sobre el nombre de lo único que ya
/// estaba seleccionado y, si en el tiempo de doble clic de Windows no llega otro clic ni una
/// tecla, se abre el renombrado (con doble clic, en cambio, se abre el archivo).
/// `name_area`: dónde está el nombre; con `measure`, solo hasta donde llega el texto.
fn slow_rename(cx: &mut Cx, resp: &egui::Response, dir: &str, e: &FileEntry, selected: bool, selection: &[String], name_area: Rect, measure: bool) {
    let key = Id::new("renombrar-lento");
    let ctx = resp.ctx.clone();
    let now = ctx.input(|i| i.time);
    let wait = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() } as f64 / 1000.0;
    if let Some(p) = ctx.data(|d| d.get_temp::<SlowRename>(key)) {
        if p.row == resp.id {
            let interrupted = ctx.input(|i| i.pointer.any_pressed() || i.events.iter().any(|ev| matches!(ev, egui::Event::Key { pressed: true, .. })));
            if interrupted || double_clicked(resp) || !selected || selection.len() != 1 || cx.rename.is_some() {
                ctx.data_mut(|d| d.remove::<SlowRename>(key));
            } else if now - p.at >= wait {
                ctx.data_mut(|d| d.remove::<SlowRename>(key));
                cx.actions.push(Action::StartRename(p.dir, p.name));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait - (now - p.at) + 0.01));
            }
            return;
        }
    }
    // `selected` es de antes de este clic: el clic que selecciona algo no lo renombra.
    // El segundo clic de un doble clic también cuenta como clic (y el primero ya seleccionó): no.
    if !(resp.clicked() && !double_clicked(resp) && selected && selection.len() == 1 && cx.rename.is_none() && ctx.input(|i| i.modifiers.is_none())) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let mut area = name_area;
    if measure {
        let w = ctx.fonts_mut(|f| f.layout_no_wrap(e.name.clone(), FontId::proportional(13.5), Color32::WHITE).size().x);
        area.max.x = area.max.x.min(area.min.x + w + 6.0);
    }
    if area.contains(pos) {
        ctx.data_mut(|d| d.insert_temp(key, SlowRename { row: resp.id, dir: dir.to_string(), name: e.name.clone(), at: now }));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait + 0.01));
    }
}

/// Devuelve true si el arrastre empezó en `marquee_x` o más a la derecha (fuera del nombre):
/// ese arrastre es un rectángulo de selección, no mover archivos.
/// `name_area` / `measure`: dónde está el nombre (para renombrar con un clic lento).
#[allow(clippy::too_many_arguments)]
fn handle_row_response(cx: &mut Cx, resp: &egui::Response, ci: usize, dir: &str, e: &FileEntry, selected: bool, selection: Vec<String>, marquee_x: Option<f32>, name_area: Rect, measure: bool) -> bool {
    slow_rename(cx, resp, dir, e, selected, &selection, name_area, measure);
    let path = fsx::join(dir, &e.name);
    // Arrastrar: se lleva la selección si la fila está seleccionada; si no, solo esa fila (y queda seleccionada sola).
    if resp.drag_started() {
        let from = resp.ctx.input(|i| i.pointer.press_origin()).map_or(0.0, |p| p.x);
        if marquee_x.is_some_and(|x| from >= x) {
            return true;
        }
        if !selected {
            cx.actions.push(Action::DragSelect { col: ci, name: e.name.clone() });
        }
        let paths = if selected { selection.iter().map(|n| fsx::join(dir, n)).collect() } else { vec![path.clone()] };
        resp.dnd_set_drag_payload(DragFiles(paths));
    }
    // Soltar sobre una carpeta: dentro de ella; sobre un archivo: en su carpeta.
    accept_drop(cx, resp, if e.is_dir { &path } else { dir });
    if resp.clicked() || (resp.secondary_clicked() && !selected) {
        let mods = resp.ctx.input(|i| i.modifiers);
        cx.actions.push(Action::Select { col: ci, name: e.name.clone(), is_dir: e.is_dir, ctrl: mods.ctrl && resp.clicked(), shift: mods.shift && resp.clicked() });
    }
    if double_clicked(resp) {
        cx.actions.push(Action::Open { path: path.clone(), is_dir: e.is_dir });
    }
    if resp.middle_clicked() && e.is_dir {
        cx.actions.push(Action::NewTab(Some(path.clone())));
    }
    if resp.secondary_clicked() && resp.ctx.input(|i| i.modifiers.shift) {
        let paths = if selected { selection.iter().map(|n| fsx::join(dir, n)).collect() } else { vec![path.clone()] };
        cx.actions.push(Action::NativeMenu(paths));
        return false;
    }
    if resp.secondary_clicked() {
        let paths: Vec<String> = if selected { selection.iter().map(|n| fsx::join(dir, n)).collect() } else { vec![path.clone()] };
        cx.actions.push(Action::OpenMenu(super::ctxmenu::MenuReq::Items { paths, is_dir: e.is_dir, dir: dir.to_string(), reveal: None }));
    }
    false
}

/// Clic derecho en el fondo de una carpeta: pedir su menú.
fn background_menu_request(cx: &mut Cx, bg: &egui::Response, dir: &str) {
    if bg.secondary_clicked() {
        cx.actions.push(Action::OpenMenu(super::ctxmenu::MenuReq::Background { dir: dir.to_string() }));
    }
}

fn size_text(e: &FileEntry) -> Option<String> {
    match (e.is_dir, e.dir_size) {
        (true, Some(s)) => Some(fsx::human_size(s)),
        (true, None) => None,
        (false, _) => Some(fsx::human_size(e.size)),
    }
}

// ---------------------------------------------------------------- columnas

pub struct ColumnData<'a> {
    pub entries: &'a [FileEntry],
    pub loading: bool,
    pub error: Option<&'a str>,
}

pub fn column(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, active: bool, data: ColumnData) {
    let rect = ui.max_rect();
    let bg = ui.interact(rect, Id::new(("colbg", cx.tab_id, ci)), Sense::click_and_drag());
    background_click(cx, &bg, ci);
    let start = drag_origin(&bg);
    let dir = col.path.clone();
    // Lo soltado sobre el fondo se acepta después de las filas: así una carpeta lo recibe primero.
    if data.error.is_some() || data.entries.is_empty() {
        background_drop(ui, cx, &bg, rect, &dir);
    }
    background_menu_request(cx, &bg, &dir);
    if let Some(err) = data.error {
        empty_message(ui, cx.t, &tf!("lista.no_puede_abrir", error = err));
        return;
    }
    if data.entries.is_empty() {
        if data.loading {
            empty_message(ui, cx.t, t!("lista.cargando"));
        } else {
            empty_message(ui, cx.t, t!("lista.carpeta_vacia"));
        }
        return;
    }
    let scroll_to = scroll_target(col, data.entries, data.loading);
    let selection = col.selected();
    let glide = glide_row(cx.t, col, data.entries, active);
    let out = virtual_list(ui, Id::new(("col", cx.tab_id, ci, &dir)), data.entries.len(), scroll_to, glide, |ui, i, r, pill| {
        cx.pill = pill;
        let e = &data.entries[i];
        let selected = col.is_selected(&e.name);
        let st = size_text(e);
        let right = st.as_deref();
        let resp = file_row(ui, cx, r, e, &dir, selected, active, right, e.is_dir, None);
        let name_area = Rect::from_min_max(pos2(r.left() + 36.0, r.top()), pos2(r.right() - 12.0, r.bottom()));
        handle_row_response(cx, &resp, ci, &dir, e, selected, selection.clone(), None, name_area, true);
    });
    cx.pill = None;
    marquee(ui, cx, ci, start, selection, &out, |r| list_hits(data.entries, r));
    background_drop(ui, cx, &bg, rect, &dir);
}

// ---------------------------------------------------------------- detalles

/// Anchos de columnas vigentes (los de la configuración; cambian mientras se arrastra un borde).
static COLUMNS: parking_lot::Mutex<ColumnWidths> = parking_lot::Mutex::new(ColumnWidths::DEFAULT);

pub fn set_columns(c: ColumnWidths) {
    *COLUMNS.lock() = c;
}

fn columns() -> ColumnWidths {
    *COLUMNS.lock()
}

/// Por debajo de este ancho de "Modificado" no entra la píldora de antigüedad junto a la fecha.
const DATE_WITH_AGE: f32 = 176.0;

/// Borde arrastrable entre dos columnas (en el encabezado): una línea tenue que se resalta.
fn column_handle(ui: &Ui, t: &Theme, id: Id, x: f32, rows: egui::Rangef) -> egui::Response {
    let r = Rect::from_x_y_ranges(x - 4.0..=x + 4.0, rows);
    let resp = ui.interact(r, id, Sense::click_and_drag()).on_hover_text(t!("lista.ajustar_columna"));
    let active = resp.hovered() || resp.dragged();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
    }
    let (c, inset) = if active { (t.accent, 4.0) } else { (t.border, 7.0) };
    ui.painter().line_segment([pos2(x, r.top() + inset), pos2(x, r.bottom() - inset)], egui::Stroke::new(if active { 2.0 } else { 1.0 }, c));
    resp
}

/// Lee un borde de columna: arrastrarlo cambia los anchos (con `apply`), soltarlo o hacerle
/// doble clic (vuelve al ancho original, con `reset`) los guarda.
fn drive_handle(cx: &mut Cx, h: &egui::Response, apply: impl Fn(&mut ColumnWidths, f32), reset: impl Fn(&mut ColumnWidths)) {
    let mut c = columns();
    if h.dragged() && h.drag_delta().x != 0.0 {
        apply(&mut c, h.drag_delta().x);
    }
    if h.double_clicked() {
        reset(&mut c);
    }
    c.date = c.date.clamp(110.0, 420.0);
    c.kind = c.kind.clamp(50.0, 320.0);
    c.size = ColumnWidths::DEFAULT.size;
    if c != columns() {
        set_columns(c);
    }
    if h.drag_stopped() || h.double_clicked() {
        cx.actions.push(Action::SaveColumns(columns()));
    }
}

fn age_pill(p: &Painter, t: &Theme, x_right: f32, cy: f32, mtime: u32, now: u32) {
    if mtime == 0 {
        return;
    }
    let age = now.saturating_sub(mtime);
    let txt = fsx::fmt_age(mtime, now);
    let c = t.age_color(age);
    let g = p.layout_no_wrap(txt, FontId::proportional(11.0), Color32::BLACK);
    let w = (g.size().x + 12.0).max(40.0);
    let r = Rect::from_min_size(pos2(x_right - w, cy - 9.0), vec2(w, 18.0));
    let old = age >= 31_536_000;
    p.rect_filled(r, 9.0, if old { c } else { c.gamma_multiply(0.22) });
    let tc = if old { t.dim } else { c };
    p.text(r.center(), Align2::CENTER_CENTER, fsx::fmt_age(mtime, now), FontId::proportional(11.0), tc);
}

pub fn details(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, focused: bool, data: ColumnData, sort: (SortKey, bool)) {
    let t = *cx.t;
    // Encabezado
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter_at(hrect);
    p.line_segment([hrect.left_bottom(), hrect.right_bottom()], egui::Stroke::new(1.0, t.border));
    let cols_x = details_columns(hrect);
    let header = |key: SortKey, label: &str, x0: f32, x1: f32, right: bool, cx: &mut Cx| {
        let r = Rect::from_x_y_ranges(x0..=x1, hrect.y_range());
        let resp = ui.interact(r, Id::new(("hdr", cx.tab_id, label)), Sense::click());
        let active = sort.0 == key;
        let c = if active || resp.hovered() { t.text } else { t.dim };
        let (pos, align) = if right { (pos2(x1 - 14.0, r.center().y), Align2::RIGHT_CENTER) } else { (pos2(x0 + 10.0, r.center().y), Align2::LEFT_CENTER) };
        let lr = p.text(pos, align, label, theme::semibold(12.0), c);
        if active {
            let arrow = if sort.1 { ic::CARET_DOWN } else { ic::CARET_UP };
            let ax = if right { lr.left() - 9.0 } else { lr.right() + 9.0 };
            p.text(pos2(ax, r.center().y), Align2::CENTER_CENTER, arrow, theme::icon_font(10.0, false), t.accent);
        }
        if resp.clicked() {
            cx.actions.push(Action::SetSort(key));
        }
    };
    header(SortKey::Name, t!("lista.nombre"), cols_x.0, cols_x.1, false, cx);
    header(SortKey::Modified, t!("comun.modificado"), cols_x.1, cols_x.2, false, cx);
    header(SortKey::Kind, t!("lista.tipo"), cols_x.2, cols_x.3, false, cx);
    header(SortKey::Size, t!("comun.tamano"), cols_x.3, cols_x.4, true, cx);
    // Bordes entre columnas: se arrastran para cambiar el ancho.
    let d = ColumnWidths::DEFAULT;
    let h = column_handle(ui, &t, Id::new("col-borde-0"), cols_x.1, hrect.y_range());
    drive_handle(cx, &h, |c, dx| c.date -= dx, |c| c.date = d.date);
    // Cada borde cambia el ancho de la columna de su derecha; lo de la izquierda se mueve con él
    // (el nombre toma la diferencia), así nunca se pisan dos columnas.
    let h = column_handle(ui, &t, Id::new("col-borde-1"), cols_x.2, hrect.y_range());
    drive_handle(cx, &h, |c, dx| c.kind -= dx, |c| c.kind = d.kind);
    // Tamaño no se ajusta: es corto y lleva la barra de uso, que se deformaría.

    let rest = ui.available_rect_before_wrap();
    let bg = ui.interact(rest, Id::new(("detbg", cx.tab_id)), Sense::click_and_drag());
    background_click(cx, &bg, ci);
    let mut start = drag_origin(&bg);
    let dir = col.path.clone();
    // Lo soltado sobre el fondo se acepta después de las filas: así una carpeta lo recibe primero.
    if data.error.is_some() || data.entries.is_empty() {
        background_drop(ui, cx, &bg, rest, &dir);
    }
    background_menu_request(cx, &bg, &dir);
    if let Some(err) = data.error {
        empty_message(ui, &t, &tf!("lista.no_puede_abrir", error = err));
        return;
    }
    if data.entries.is_empty() {
        empty_message(ui, &t, if data.loading { t!("lista.cargando") } else { t!("lista.carpeta_vacia") });
        return;
    }
    let scroll_to = scroll_target(col, data.entries, data.loading);
    let selection = col.selected();
    // Para las barras de "qué ocupa espacio": proporción de cada elemento sobre el total de la carpeta.
    let total: u64 = data.entries.iter().filter_map(|e| e.display_size()).sum();
    let glide = glide_row(&t, col, data.entries, focused);
    let out = virtual_list(ui, Id::new(("det", cx.tab_id, &dir)), data.entries.len(), scroll_to, glide, |ui, i, r, pill| {
        cx.pill = pill;
        let e = &data.entries[i];
        let selected = col.is_selected(&e.name);
        let (_, x1, x2, x3, x4) = details_columns(r);
        let resp = file_row(ui, cx, r, e, &dir, selected, focused, None, false, Some(x1 - 8.0));
        let p = ui.painter_at(r);
        let (_, dim_c, _) = row_fg(&t, pill, r, selected, focused);
        if cx.space_bars && total > 0 {
            if let Some(sz) = e.display_size() {
                let frac = (sz as f64 / total as f64) as f32;
                let bar = Rect::from_min_size(pos2(x3 + 10.0, r.bottom() - 6.0), vec2((x4 - x3 - 24.0) * frac, 3.0));
                let c = if frac > 0.25 { t.danger } else if frac > 0.1 { Color32::from_rgb(0xF5, 0x9E, 0x42) } else { t.accent };
                p.rect_filled(bar, 1.5, c.gamma_multiply(if selected && focused { 0.9 } else { 0.7 }));
            }
        }
        p.text(pos2(x1 + 10.0, r.center().y), Align2::LEFT_CENTER, fsx::fmt_date(e.mtime), FontId::proportional(12.0), dim_c);
        if x2 - x1 >= DATE_WITH_AGE {
            age_pill(&p, &t, x2 - 8.0, r.center().y, e.mtime, cx.now);
        }
        let kind = if e.is_dir {
            t!("comun.carpeta").to_string()
        } else if fsx::downloading(&e.name).is_some() {
            t!("lista.descarga").to_string()
        } else if e.ext.is_empty() {
            t!("comun.archivo").into()
        } else {
            e.ext.to_uppercase()
        };
        p.text(pos2(x2 + 10.0, r.center().y), Align2::LEFT_CENTER, kind, FontId::proportional(12.0), dim_c);
        if let Some(s) = size_text(e) {
            let txt = if e.is_dir { format!("~{s}") } else { s };
            p.text(pos2(r.right() - 14.0, r.center().y), Align2::RIGHT_CENTER, txt, FontId::proportional(12.0), dim_c);
        }
        // Arrastrar desde fuera del nombre (fecha, tipo, tamaño) dibuja un rectángulo, como en el Explorador.
        let name_area = Rect::from_min_max(pos2(r.left() + 36.0, r.top()), pos2(x1, r.bottom()));
        if handle_row_response(cx, &resp, ci, &dir, e, selected, selection.clone(), Some(x1), name_area, true) {
            start = resp.ctx.input(|i| i.pointer.press_origin());
        }
    });
    cx.pill = None;
    marquee(ui, cx, ci, start, selection, &out, |r| list_hits(data.entries, r));
    background_drop(ui, cx, &bg, rest, &dir);
}

/// Bordes x de las columnas de la vista de detalles: (nombre, fecha, tipo, tamaño, fin).
fn details_columns(r: Rect) -> (f32, f32, f32, f32, f32) {
    let x4 = r.right();
    let c = columns();
    let x3 = x4 - ColumnWidths::DEFAULT.size;
    let x2 = x3 - c.kind;
    let x1 = x2 - c.date;
    (r.left(), x1.max(r.left() + 160.0), x2, x3, x4)
}

// ---------------------------------------------------------------- resultados de búsqueda

/// Posiciones x de las columnas de resultados: (nombre, carpeta, borde derecho del tamaño, fecha).
fn search_columns(r: Rect) -> (f32, f32, f32, f32) {
    let nx = r.left() + 36.0;
    let date_x = r.right() - 150.0;
    let size_r = date_x - 14.0;
    let auto = ((size_r - 90.0 - nx) * 0.42).max(140.0);
    let set = columns().search_name;
    let name_w = if set > 0.0 { set.clamp(140.0, (size_r - 90.0 - nx - 120.0).max(140.0)) } else { auto };
    (nx, nx + name_w + 12.0, size_r, date_x)
}

#[allow(clippy::too_many_arguments)]
pub fn search_results(ui: &mut Ui, cx: &mut Cx, res: Option<&Results>, cursor: &mut usize, scroll: &mut bool, indexing: Option<String>, query: &str, sort: (ResultSort, bool)) {
    let t = *cx.t;
    let Some(res) = res else {
        empty_message(ui, &t, indexing.as_deref().unwrap_or(t!("lista.buscando")));
        return;
    };
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let shown = res.len();
        let txt = if res.total > shown {
            tf!("lista.resultados_mejores", n = fsx::thousands(res.total), mostrados = fsx::thousands(shown))
        } else {
            tp!(res.total, "lista.resultados", n = fsx::thousands(res.total))
        };
        ui.label(egui::RichText::new(txt).font(theme::semibold(12.5)).color(t.text));
        ui.label(egui::RichText::new(tf!("lista.milisegundos", ms = format!("{:.2}", res.ms))).size(12.0).color(t.accent));
        if res.total > shown && ui.link(egui::RichText::new(t!("lista.ver_todos")).size(12.0)).clicked() {
            cx.actions.push(Action::LoadAllResults);
        }
        if let Some(ix) = &indexing {
            ui.label(egui::RichText::new(format!("· {ix}")).size(12.0).color(t.faint));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let on = sort.0 == ResultSort::Relevance;
            let label = egui::RichText::new(format!("{}  {}", ic::SPARKLE, t!("lista.relevancia"))).size(12.0).color(if on { t.accent } else { t.dim });
            if ui.add(egui::Button::new(label).frame(on)).on_hover_text(t!("lista.ordenar_relevancia")).clicked() {
                cx.actions.push(Action::SetResultSort(ResultSort::Relevance));
            }
        });
    });
    // Encabezados para ordenar por columna (como en Everything).
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    {
        let p = ui.painter_at(hrect);
        p.line_segment([hrect.left_bottom(), hrect.right_bottom()], egui::Stroke::new(1.0, t.border));
        let (nx, dir_x, size_r, date_x) = search_columns(hrect);
        let cols = [
            (ResultSort::Name, t!("lista.nombre"), nx, false),
            (ResultSort::Folder, t!("comun.carpeta"), dir_x, false),
            (ResultSort::Size, t!("comun.tamano"), size_r, true),
            (ResultSort::Modified, t!("comun.modificado"), date_x, false),
        ];
        for (key, label, x, right) in cols {
            let active = sort.0 == key;
            let g = p.layout_no_wrap(label.to_string(), theme::semibold(12.0), t.dim);
            let w = g.size().x + 16.0;
            let r = if right { Rect::from_x_y_ranges(x - w..=x + 4.0, hrect.y_range()) } else { Rect::from_x_y_ranges(x - 4.0..=x + w, hrect.y_range()) };
            let resp = ui.interact(r, Id::new(("shdr", cx.tab_id, label)), Sense::click());
            let c = if active || resp.hovered() { t.text } else { t.dim };
            let lr = p.text(pos2(x, hrect.center().y), if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, label, theme::semibold(12.0), c);
            if active {
                let arrow = if sort.1 { ic::CARET_DOWN } else { ic::CARET_UP };
                let ax = if right { lr.left() - 9.0 } else { lr.right() + 9.0 };
                p.text(pos2(ax, hrect.center().y), Align2::CENTER_CENTER, arrow, theme::icon_font(10.0, false), t.accent);
            }
            if resp.clicked() {
                cx.actions.push(Action::SetResultSort(key));
            }
        }
        // Borde entre Nombre y Carpeta: se arrastra para repartir el ancho.
        let name_w = dir_x - 12.0 - nx;
        let h = column_handle(ui, &t, Id::new(("res-borde", cx.tab_id)), dir_x - 8.0, hrect.y_range());
        drive_handle(cx, &h, move |c, dx| c.search_name = (if c.search_name > 0.0 { c.search_name } else { name_w }) + dx, |c| c.search_name = 0.0);
    }
    if res.is_empty() {
        empty_message(ui, &t, t!("lista.sin_resultados"));
        return;
    }
    let needle = query
        .split_whitespace()
        .find(|w| !w.contains(':') && !w.starts_with('!') && !w.contains('*') && !w.contains('\\'))
        .map(crate::index::store::search_key)
        .unwrap_or_default();
    let scroll_to = if *scroll {
        *scroll = false;
        Some(*cursor)
    } else {
        None
    };
    let n = res.len();
    let mut reached_end = false;
    let glide = (*cursor < n).then_some((*cursor, t.sel));
    virtual_list(ui, Id::new(("search", cx.tab_id)), n, scroll_to, glide, |ui, i, r, pill| {
        if i + 1 == n {
            reached_end = true;
        }
        let Some(row) = res.row(i) else { return };
        let selected = i == *cursor;
        let resp = ui.interact(r, Id::new(("sr", cx.tab_id, i)), Sense::click());
        let p = ui.painter_at(r);
        let hover = hover_amount(&resp, pill.is_none_or(|pl| pill_cover(pl, r) < 0.5));
        row_bg(&p, r, &t, selected && pill.is_none(), true, hover);
        let (text_c, dim_c, on_sel) = row_fg(&t, pill, r, selected, true);
        file_icon(&p, &t, pos2(r.left() + 22.0, r.center().y), 16.0, 16.0, &row.path(), row.is_dir, &row.ext, on_sel.then_some(t.accent_text), 1.0);
        let (nx, dir_x, size_r, date_x) = search_columns(r);
        let name_w = dir_x - nx - 12.0;
        let g = if on_sel {
            elided(&p, &row.name, FontId::proportional(13.5), text_c, name_w)
        } else {
            highlighted(&p, &row.name, &needle, &t, text_c, name_w)
        };
        p.galley(pos2(nx, r.center().y - g.size().y / 2.0), g, text_c);
        let dg = elided(&p, &row.dir, FontId::proportional(12.0), dim_c, size_r - 90.0 - dir_x);
        p.galley(pos2(dir_x, r.center().y - dg.size().y / 2.0), dg, dim_c);
        p.text(pos2(date_x, r.center().y), Align2::LEFT_CENTER, fsx::fmt_date(row.mtime), FontId::proportional(12.0), dim_c);
        let size = if row.is_dir { format!("~{}", fsx::human_size(row.size)) } else { fsx::human_size(row.size) };
        p.text(pos2(size_r, r.center().y), Align2::RIGHT_CENTER, size, FontId::proportional(12.0), dim_c);
        if resp.clicked() || resp.secondary_clicked() {
            *cursor = i;
        }
        if double_clicked(&resp) {
            cx.actions.push(Action::Open { path: row.path(), is_dir: row.is_dir });
        }
        if resp.secondary_clicked() {
            cx.actions.push(Action::OpenMenu(super::ctxmenu::MenuReq::Items { paths: vec![row.path()], is_dir: row.is_dir, dir: row.dir.clone(), reveal: Some(row.path()) }));
        }
    });
    // Al llegar al final de los mejores resultados, cargar el resto.
    if reached_end && !res.full && res.total > n {
        cx.actions.push(Action::LoadAllResults);
    }
}

// ---------------------------------------------------------------- cuadrícula

const TILE_W: f32 = 128.0;
const TILE_H: f32 = 148.0;

/// ¿Vale la pena pedir la miniatura real (foto, video, PDF, ícono del programa)?
/// Archivos que se muestran con su contenido (fotos, videos, documentos). Lo demás (programas,
/// accesos directos, zip, Word…) lleva su ícono de Windows, todos del mismo tamaño.
fn wants_thumbnail(e: &FileEntry) -> bool {
    !e.is_dir && (fsx::thumbs::is_image_ext(&e.ext) || matches!(e.ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "mpg" | "pdf" | "psd" | "svg"))
}

/// Emblema de las carpetas conocidas del usuario (Escritorio, Documentos…).
fn folder_emblem(path: &str) -> Option<&'static str> {
    let home = std::env::var("USERPROFILE").ok()?;
    let rest = path.get(home.len()..).filter(|_| path.len() > home.len() && path[..home.len()].eq_ignore_ascii_case(&home))?;
    Some(match rest.trim_start_matches('\\').to_lowercase().as_str() {
        "desktop" => ic::DESKTOP,
        "documents" => ic::FILE_TEXT,
        "downloads" => ic::DOWNLOAD_SIMPLE,
        "pictures" => ic::IMAGE,
        "music" => ic::MUSIC_NOTES,
        "videos" => ic::FILM_STRIP,
        _ => return None,
    })
}

/// Una carpeta en la cuadrícula: el ícono de Rayo y, si tiene fotos o videos, la primera asomándose.
#[allow(clippy::too_many_arguments)]
fn grid_folder(ui: &Ui, p: &Painter, t: &Theme, area: Rect, path: &str, tex: &mut super::preview::Textures, alpha: f32) {
    let center = area.center();
    p.text(center, Align2::CENTER_CENTER, ic::FOLDER, theme::icon_font(84.0, true), t.folder.gamma_multiply(alpha));
    // Frente de la carpeta (el glifo deja la pestaña arriba): ahí va la foto o el emblema.
    let front = Rect::from_center_size(center + vec2(0.0, 6.0), vec2(58.0, 40.0));
    let ppp = ui.ctx().pixels_per_point();
    let px = ((front.width().max(front.height()) * ppp).ceil() as u32 / 32 + 1) * 32;
    let peek = tex.get(&format!("{}{path}", fsx::thumbs::PEEK_PREFIX), px).and_then(|te| te.tex.clone().map(|tx| (tx, te.at)));
    if let Some((tx, at)) = peek {
        let shown = super::motion::since(ui.ctx(), at, 0.22) * alpha;
        // Recortada para llenar la tarjeta, con un borde claro, como una foto metida en la carpeta.
        let size = tx.size_vec2();
        let (fw, fh) = (front.width() / size.x, front.height() / size.y);
        let uv = if fw > fh {
            let k = fh / fw;
            Rect::from_min_max(pos2(0.0, (1.0 - k) / 2.0), pos2(1.0, (1.0 + k) / 2.0))
        } else {
            let k = fw / fh;
            Rect::from_min_max(pos2((1.0 - k) / 2.0, 0.0), pos2((1.0 + k) / 2.0, 1.0))
        };
        let card = front.expand(2.0);
        p.rect_filled(card, 4.0, Color32::WHITE.gamma_multiply(0.92 * shown));
        p.image(tx.id(), front, uv, Color32::WHITE.gamma_multiply(shown));
        return;
    }
    if let Some(g) = folder_emblem(path) {
        p.text(front.center(), Align2::CENTER_CENTER, g, theme::icon_font(22.0, true), Color32::WHITE.gamma_multiply(0.9 * alpha));
    }
}

/// Columnas de la cuadrícula de una pestaña en el último cuadro (para moverse con las flechas).
pub fn grid_columns(ctx: &egui::Context, tab_id: u64) -> usize {
    ctx.data(|d| d.get_temp::<usize>(Id::new(("grid-cols", tab_id)))).unwrap_or(1).max(1)
}

#[allow(clippy::too_many_arguments)]
pub fn grid(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, focused: bool, data: ColumnData, tex: &mut super::preview::Textures) {
    let t = *cx.t;
    let rest = ui.available_rect_before_wrap();
    let bg = ui.interact(rest, Id::new(("gridbg", cx.tab_id)), Sense::click_and_drag());
    background_click(cx, &bg, ci);
    let start = drag_origin(&bg);
    let dir = col.path.clone();
    // Lo soltado sobre el fondo se acepta después de las filas: así una carpeta lo recibe primero.
    if data.error.is_some() || data.entries.is_empty() {
        background_drop(ui, cx, &bg, rest, &dir);
    }
    background_menu_request(cx, &bg, &dir);
    if let Some(err) = data.error {
        empty_message(ui, &t, &tf!("lista.no_puede_abrir", error = err));
        return;
    }
    if data.entries.is_empty() {
        empty_message(ui, &t, if data.loading { t!("lista.cargando") } else { t!("lista.carpeta_vacia") });
        return;
    }
    let n = data.entries.len();
    let cols = ((rest.width() - 12.0) / TILE_W).floor().max(1.0) as usize;
    ui.ctx().data_mut(|d| d.insert_temp(Id::new(("grid-cols", cx.tab_id)), cols));
    let rows = n.div_ceil(cols);
    let scroll_to = scroll_target(col, data.entries, data.loading);
    let selection = col.selected();
    let ppp = ui.ctx().pixels_per_point();
    let base = selection.clone();
    let out = egui::ScrollArea::vertical().id_salt(("grid", cx.tab_id, &dir)).auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        ui.set_height(rows as f32 * TILE_H + 12.0);
        let top = ui.max_rect().top() + 6.0;
        let left = ui.max_rect().left() + 6.0;
        if let Some(i) = scroll_to {
            let r = Rect::from_min_size(pos2(left, top + (i / cols) as f32 * TILE_H), vec2(TILE_W, TILE_H));
            ui.scroll_to_rect(r, None);
        }
        // El resaltado se desliza de mosaico en mosaico (también en diagonal, al cambiar de fila).
        let pill = glide_row(&t, col, data.entries, focused).map(|(i, color)| {
            let target = Rect::from_min_size(pos2(left + (i % cols) as f32 * TILE_W, top + (i / cols) as f32 * TILE_H), vec2(TILE_W, TILE_H)).shrink(3.0);
            let r = pill_rect(ui, Id::new(("grid-pill", cx.tab_id, &dir)), target);
            ui.painter().rect_filled(r, 8.0, if focused { color.gamma_multiply(0.55) } else { color });
            r
        });
        let first = (viewport.min.y / TILE_H).floor().max(0.0) as usize;
        let last = ((viewport.max.y / TILE_H).ceil() as usize + 1).min(rows);
        for row in first..last {
            for c in 0..cols {
                let i = row * cols + c;
                if i >= n {
                    break;
                }
                let e = &data.entries[i];
                let r = Rect::from_min_size(pos2(left + c as f32 * TILE_W, top + row as f32 * TILE_H), vec2(TILE_W, TILE_H)).shrink(3.0);
                let path = fsx::join(&dir, &e.name);
                let resp = ui.interact(r, Id::new(("tile", cx.tab_id, &path)), Sense::click_and_drag());
                let selected = col.is_selected(&e.name);
                if e.is_dir {
                    prefetch(cx, &resp, &path, selected && focused);
                }
                let p = ui.painter_at(r);
                let hover = hover_amount(&resp, !selected || pill.is_some_and(|pl| pill_cover(pl, r) < 0.5));
                if selected && pill.is_none() {
                    p.rect_filled(r, 8.0, if focused { t.sel.gamma_multiply(0.55) } else { t.sel_inactive });
                } else if hover > 0.01 {
                    p.rect_filled(r, 8.0, t.hover.gamma_multiply(hover));
                }
                if e.is_dir {
                    drop_highlight(ui, &t, &resp, r);
                }
                // Miniatura (o ícono grande).
                let thumb = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(r.width() - 16.0, 92.0));
                let mut drew = false;
                if e.is_dir {
                    let alpha = if e.hidden || cx.cut.contains(&path) { 0.5 } else { 1.0 };
                    grid_folder(ui, &p, &t, thumb, &path, tex, alpha);
                    drew = true;
                } else if wants_thumbnail(e) {
                    if let Some(te) = tex.get(&path, (thumb.width().max(thumb.height()) * ppp).ceil() as u32 / 32 * 32 + 32) {
                        if let Some(tx) = &te.tex {
                            let size = tx.size_vec2();
                            // Nunca más grande que sus píxeles: agrandada se vería borrosa.
                            let s = (thumb.width() / size.x).min(thumb.height() / size.y).min(1.0 / ppp);
                            let img = Rect::from_center_size(thumb.center(), size * s);
                            // Recién llegada: aparece con un fundido sobre el ícono, que se desvanece.
                            let shown = super::motion::since(ui.ctx(), te.at, 0.22);
                            if shown < 1.0 {
                                file_icon(&p, &t, thumb.center(), 64.0, 58.0, &path, e.is_dir, &e.ext, None, 1.0 - shown);
                            }
                            p.image(tx.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(shown));
                            drew = true;
                        }
                    }
                }
                if !drew {
                    file_icon(&p, &t, thumb.center(), 64.0, 58.0, &path, e.is_dir, &e.ext, None, 1.0);
                }
                let mask = fsx::tags::get(&path);
                if mask != 0 {
                    tag_dots(&p, &t, thumb.left() + 2.0, thumb.top() + 7.0, mask);
                }
                if let Some((g, c, _)) = cloud_glyph(&t, e.cloud) {
                    let b = Rect::from_center_size(pos2(thumb.right() - 6.0, thumb.top() + 8.0), vec2(20.0, 20.0));
                    p.circle_filled(b.center(), 10.0, t.chrome);
                    p.text(b.center(), Align2::CENTER_CENTER, g, theme::icon_font(13.0, false), c);
                }
                // Nombre en hasta dos líneas.
                let renaming = cx.rename.as_ref().is_some_and(|rn| rn.dir == dir && rn.old == e.name);
                let name_rect = Rect::from_min_max(pos2(r.left() + 4.0, thumb.bottom() + 4.0), pos2(r.right() - 4.0, r.bottom() - 2.0));
                if renaming {
                    rename_row(ui, cx, Rect::from_min_size(name_rect.min, vec2(name_rect.width(), 24.0)), name_rect.left());
                } else {
                    let color = if selected && focused { t.text } else { t.text };
                    let mut job = LayoutJob::single_section(e.name.clone(), TextFormat::simple(FontId::proportional(12.5), color));
                    job.wrap = TextWrapping { max_width: name_rect.width(), max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
                    job.halign = egui::Align::Center;
                    let g = p.layout_job(job);
                    p.galley(pos2(name_rect.center().x, name_rect.top()), g, color);
                }
                handle_row_response(cx, &resp, ci, &dir, e, selected, selection.clone(), None, name_rect, false);
            }
        }
    });
    // Mosaicos que toca el rectángulo (en coordenadas del contenido: margen de 6 y celdas de TILE_W × TILE_H).
    let entries = data.entries;
    marquee(ui, cx, ci, start, base, &out, |r| {
        let mut v = Vec::new();
        let rows_hit = ((r.min.y - 6.0) / TILE_H).floor().max(0.0) as usize..=(((r.max.y - 6.0) / TILE_H).floor().max(0.0) as usize).min(rows.saturating_sub(1));
        for row in rows_hit {
            for c in 0..cols {
                let i = row * cols + c;
                if i >= n {
                    break;
                }
                let tile = Rect::from_min_size(pos2(6.0 + c as f32 * TILE_W, 6.0 + row as f32 * TILE_H), vec2(TILE_W, TILE_H)).shrink(3.0);
                if tile.intersects(r) {
                    v.push(entries[i].name.clone());
                }
            }
        }
        v
    });
    background_drop(ui, cx, &bg, rest, &dir);
}
