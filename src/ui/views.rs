//! Vistas de contenido: columnas (estilo Finder), detalles y resultados de búsqueda.

use super::actions::Action;
use super::icons;
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

pub fn row_bg(p: &Painter, rect: Rect, t: &Theme, selected: bool, active: bool, hovered: bool) {
    let r = rect.shrink2(vec2(4.0, 1.0));
    if selected {
        p.rect_filled(r, 5.0, if active { t.sel } else { t.sel_inactive });
    } else if hovered {
        p.rect_filled(r, 5.0, t.hover);
    }
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
fn virtual_list(ui: &mut Ui, id: Id, n: usize, scroll_to: Option<usize>, mut row: impl FnMut(&mut Ui, usize, Rect)) -> egui::scroll_area::ScrollAreaOutput<()> {
    let out = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        ui.set_height(ROW_H * n as f32 + 6.0);
        let top = ui.max_rect().top();
        let left = ui.max_rect().left();
        if let Some(i) = scroll_to {
            let r = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            ui.scroll_to_rect(r.expand2(vec2(0.0, ROW_H * 0.5)), None);
        }
        let first = (viewport.min.y / ROW_H).floor().max(0.0) as usize;
        let last = ((viewport.max.y / ROW_H).ceil() as usize + 1).min(n);
        for i in first..last {
            let r = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            row(ui, i, r);
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
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if esc {
        *cx.rename = None;
    } else if enter || out.response.lost_focus() {
        cx.actions.push(Action::CommitRename);
    }
}

/// Dibuja una fila de archivo genérica (ícono + nombre + texto a la derecha).
#[allow(clippy::too_many_arguments)]
fn file_row(ui: &mut Ui, cx: &mut Cx, rect: Rect, e: &FileEntry, dir: &str, selected: bool, active: bool, right: Option<&str>, chevron: bool, name_limit: Option<f32>) -> egui::Response {
    let id = Id::new(("row", cx.tab_id, dir, &e.name));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let p = ui.painter_at(rect);
    row_bg(&p, rect, cx.t, selected, active, resp.hovered());
    if e.is_dir {
        drop_highlight(ui, cx.t, &resp, rect);
    }
    let (text_c, dim_c) = fg(cx.t, selected, active);
    let (glyph, mut icolor) = icons::for_entry(cx.t, e.is_dir, &e.ext);
    if selected && active {
        icolor = cx.t.accent_text;
    }
    let path = fsx::join(dir, &e.name);
    let faded = e.hidden || cx.cut.contains(&path);
    let alpha = if faded { 0.5 } else { 1.0 };
    p.text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, glyph, theme::icon_font(16.0, true), icolor.gamma_multiply(alpha));
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
        let c = if selected && active { cx.t.accent_text } else { c };
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
        let g = elided(&p, &e.name, FontId::proportional(13.5), text_c.gamma_multiply(alpha), limit - name_x);
        p.galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, text_c);
    }
    resp
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
            if interrupted || resp.double_clicked() || !selected || selection.len() != 1 || cx.rename.is_some() {
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
    if !(resp.clicked() && !resp.double_clicked() && selected && selection.len() == 1 && cx.rename.is_none() && ctx.input(|i| i.modifiers.is_none())) {
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
    if resp.double_clicked() {
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
    let scroll_to = if col.scroll_to_cursor {
        col.scroll_to_cursor = false;
        col.cursor.as_ref().and_then(|c| data.entries.iter().position(|e| &e.name == c))
    } else {
        None
    };
    let selection = col.selected();
    let out = virtual_list(ui, Id::new(("col", cx.tab_id, ci, &dir)), data.entries.len(), scroll_to, |ui, i, r| {
        let e = &data.entries[i];
        let selected = col.is_selected(&e.name);
        let st = size_text(e);
        let right = st.as_deref();
        let resp = file_row(ui, cx, r, e, &dir, selected, active, right, e.is_dir, None);
        let name_area = Rect::from_min_max(pos2(r.left() + 36.0, r.top()), pos2(r.right() - 12.0, r.bottom()));
        handle_row_response(cx, &resp, ci, &dir, e, selected, selection.clone(), None, name_area, true);
    });
    marquee(ui, cx, ci, start, selection, &out, |r| list_hits(data.entries, r));
    background_drop(ui, cx, &bg, rect, &dir);
}

// ---------------------------------------------------------------- detalles

const W_DATE: f32 = 140.0;
const W_AGE: f32 = 56.0;
const W_KIND: f32 = 120.0;
const W_SIZE: f32 = 86.0;

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
    let scroll_to = if col.scroll_to_cursor {
        col.scroll_to_cursor = false;
        col.cursor.as_ref().and_then(|c| data.entries.iter().position(|e| &e.name == c))
    } else {
        None
    };
    let selection = col.selected();
    // Para las barras de "qué ocupa espacio": proporción de cada elemento sobre el total de la carpeta.
    let total: u64 = data.entries.iter().filter_map(|e| e.display_size()).sum();
    let out = virtual_list(ui, Id::new(("det", cx.tab_id, &dir)), data.entries.len(), scroll_to, |ui, i, r| {
        let e = &data.entries[i];
        let selected = col.is_selected(&e.name);
        let (_, x1, x2, x3, x4) = details_columns(r);
        let resp = file_row(ui, cx, r, e, &dir, selected, focused, None, false, Some(x1 - 8.0));
        let p = ui.painter_at(r);
        let (_, dim_c) = fg(&t, selected, focused);
        if cx.space_bars && total > 0 {
            if let Some(sz) = e.display_size() {
                let frac = (sz as f64 / total as f64) as f32;
                let bar = Rect::from_min_size(pos2(x3 + 10.0, r.bottom() - 6.0), vec2((x4 - x3 - 24.0) * frac, 3.0));
                let c = if frac > 0.25 { t.danger } else if frac > 0.1 { Color32::from_rgb(0xF5, 0x9E, 0x42) } else { t.accent };
                p.rect_filled(bar, 1.5, c.gamma_multiply(if selected && focused { 0.9 } else { 0.7 }));
            }
        }
        p.text(pos2(x1 + 10.0, r.center().y), Align2::LEFT_CENTER, fsx::fmt_date(e.mtime), FontId::proportional(12.0), dim_c);
        age_pill(&p, &t, x2 - 8.0, r.center().y, e.mtime, cx.now);
        let kind = if e.is_dir { t!("comun.carpeta").to_string() } else if e.ext.is_empty() { t!("comun.archivo").into() } else { e.ext.to_uppercase() };
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
    marquee(ui, cx, ci, start, selection, &out, |r| list_hits(data.entries, r));
    background_drop(ui, cx, &bg, rest, &dir);
}

/// Bordes x de las columnas de la vista de detalles: (nombre, fecha, tipo, tamaño, fin).
fn details_columns(r: Rect) -> (f32, f32, f32, f32, f32) {
    let x4 = r.right();
    let x3 = x4 - W_SIZE;
    let x2 = x3 - W_KIND;
    let x1 = x2 - W_DATE - W_AGE;
    (r.left(), x1.max(r.left() + 160.0), x2, x3, x4)
}

// ---------------------------------------------------------------- resultados de búsqueda

/// Posiciones x de las columnas de resultados: (nombre, carpeta, borde derecho del tamaño, fecha).
fn search_columns(r: Rect) -> (f32, f32, f32, f32) {
    let nx = r.left() + 36.0;
    let date_x = r.right() - 150.0;
    let size_r = date_x - 14.0;
    let name_w = ((size_r - 90.0 - nx) * 0.42).max(140.0);
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
    virtual_list(ui, Id::new(("search", cx.tab_id)), n, scroll_to, |ui, i, r| {
        if i + 1 == n {
            reached_end = true;
        }
        let Some(row) = res.row(i) else { return };
        let selected = i == *cursor;
        let resp = ui.interact(r, Id::new(("sr", cx.tab_id, i)), Sense::click());
        let p = ui.painter_at(r);
        row_bg(&p, r, &t, selected, true, resp.hovered());
        let (text_c, dim_c) = fg(&t, selected, true);
        let (glyph, mut ic_c) = icons::for_entry(&t, row.is_dir, &row.ext);
        if selected {
            ic_c = t.accent_text;
        }
        p.text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, glyph, theme::icon_font(16.0, true), ic_c);
        let (nx, dir_x, size_r, date_x) = search_columns(r);
        let name_w = dir_x - nx - 12.0;
        let g = if selected {
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
        if resp.double_clicked() {
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
fn wants_thumbnail(e: &FileEntry) -> bool {
    !e.is_dir
        && (fsx::thumbs::is_image_ext(&e.ext)
            || matches!(e.ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "mpg" | "pdf" | "exe" | "lnk" | "ico" | "psd" | "svg" | "msi"))
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
    let scroll_to = if col.scroll_to_cursor {
        col.scroll_to_cursor = false;
        col.cursor.as_ref().and_then(|c| data.entries.iter().position(|e| &e.name == c))
    } else {
        None
    };
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
                let p = ui.painter_at(r);
                if selected {
                    p.rect_filled(r, 8.0, if focused { t.sel.gamma_multiply(0.55) } else { t.sel_inactive });
                } else if resp.hovered() {
                    p.rect_filled(r, 8.0, t.hover);
                }
                if e.is_dir {
                    drop_highlight(ui, &t, &resp, r);
                }
                // Miniatura (o ícono grande).
                let thumb = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(r.width() - 16.0, 92.0));
                let mut drew = false;
                if wants_thumbnail(e) {
                    if let Some(te) = tex.get(&path, (thumb.width().max(thumb.height()) * ppp).ceil() as u32 / 32 * 32 + 32) {
                        if let Some(tx) = &te.tex {
                            let size = tx.size_vec2();
                            let s = (thumb.width() / size.x).min(thumb.height() / size.y).min(1.0 * ppp.recip() * 4.0);
                            let img = Rect::from_center_size(thumb.center(), size * s);
                            p.image(tx.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            drew = true;
                        }
                    }
                }
                if !drew {
                    let (glyph, color) = icons::for_entry(&t, e.is_dir, &e.ext);
                    p.text(thumb.center(), Align2::CENTER_CENTER, glyph, theme::icon_font(54.0, true), color);
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
