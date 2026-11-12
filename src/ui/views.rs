//! Vistas de contenido: columnas (estilo Finder), detalles y resultados de búsqueda.

use super::actions::Action;
use super::design::{self, Chip, Sel, Tone, dur, h, icon, radius, sp, stroke, text};
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

pub const ROW_H: f32 = h::ROW;
/// Margen del pill de una fila a cada lado (4.1).
const ROW_INSET: f32 = sp::XS;
/// Borde izquierdo del ícono de una fila y comienzo del texto.
const ICON_X: f32 = ROW_INSET + sp::S;
const TEXT_X: f32 = ICON_X + icon::M + sp::S;
/// Aire a la izquierda (y a la derecha, si se alinea ahí) de los valores de una columna.
const CELL_X: f32 = sp::M;

/// Opacidad de contenido: atenuado (oculto o cortado, 50 %) o fundido de algo que llega.
fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// Opacidad del contenido atenuado (oculto o cortado): en claro, 0,6 (al 50 % el gris queda en 3,4:1).
fn faded(t: &Theme) -> f32 {
    if t.dark { 0.5 } else { 0.6 }
}

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
    /// solo seleccionado). Las filas no dibujan el suyo (ni hover donde las cubre).
    pub pill: Option<Rect>,
    /// Lista con carpetas desplegadas: las filas van con sangría y muestran solo su nombre.
    pub tree: Option<&'a std::collections::BTreeSet<String>>,
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
        let r = rect.shrink(sp::XXS);
        ui.painter().rect_filled(r, radius::M, t.accent_soft);
        design::focus_ring(ui.painter(), t, r, radius::M);
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
    ui.add_space(sp::XS);
    ui.horizontal(|ui| {
        // Alineado con la columna de íconos de las filas.
        ui.add_space(ICON_X);
        let txt = tf!("lista.filtrando", filtro = filter);
        design::chip(ui, t, Chip { icon: ic::FUNNEL, ..Chip::new(&txt, Tone::Accent) });
        ui.label(egui::RichText::new(t!("lista.esc_quitar")).font(text::small()).color(t.faint));
    });
    ui.add_space(sp::XXS);
}

/// Indicador de la vista plana (clic en la X para salir).
pub fn flat_chip(ui: &mut Ui, t: &Theme, n: usize, loading: bool, actions: &mut Vec<Action>) {
    ui.add_space(sp::XS);
    ui.horizontal(|ui| {
        ui.add_space(ICON_X);
        let txt = if loading { t!("lista.vista_plana_juntando_archivos").to_string() } else { tp!(n, "lista.vista_plana_archivos_todas_subcarpetas", n = fsx::thousands(n)) };
        let icon = if loading { "" } else { ic::STACK };
        let out = design::chip(ui, t, Chip { icon, closable: true, ..Chip::new(&txt, Tone::Accent) });
        // El atajo va como tecla en línea (el texto todavía lo trae entre paréntesis).
        let tip = t!("lista.salir_vista_plana_esc");
        design::tip_keys(out.response, t, tip.trim_end_matches("(Esc)").trim_end(), "Esc");
        if out.closed {
            actions.push(Action::ToggleFlat);
        }
    });
    ui.add_space(sp::XXS);
}

pub struct Rename {
    pub dir: String,
    pub old: String,
    pub buf: String,
    pub first_frame: bool,
}

pub fn elided(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    theme::want_cjk(text);
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(max_w.max(10.0));
    p.layout_job(job)
}

/// Nombre de la vista plana («Hakone\foto-1.txt»): la subcarpeta en `faint` y el nombre en
/// `color`. Si no entra, la subcarpeta se acorta por el medio (el nombre manda).
fn flat_name(p: &Painter, t: &Theme, name: &str, color: Color32, a: f32, max_w: f32) -> Arc<egui::Galley> {
    let Some(i) = name.rfind('\\') else { return elided(p, name, text::body(), color, max_w) };
    theme::want_cjk(name);
    let (dir, file) = (&name[..=i], &name[i + 1..]);
    let room = max_w - design::text_width(p, file, text::body());
    let dir = if room >= design::text_width(p, "…\\", text::body()) { design::elide_middle(p, dir, text::body(), room) } else { String::new() };
    let mut job = LayoutJob::default();
    job.append(&dir, 0.0, TextFormat::simple(text::body(), alpha(t.faint, a)));
    job.append(file, 0.0, TextFormat::simple(text::body(), color));
    job.wrap = TextWrapping::truncate_at_width(max_w.max(10.0));
    p.layout_job(job)
}

/// Texto con la parte que coincide resaltada.
/// `on_sel`: la fila está seleccionada (en claro, la coincidencia va en `accent_hover`, T4).
fn highlighted(p: &Painter, text: &str, needle: &str, t: &Theme, color: Color32, max_w: f32, on_sel: bool) -> Arc<egui::Galley> {
    theme::want_cjk(text);
    let font = text::body();
    let mut job = LayoutJob::default();
    // La coincidencia se busca en la clave (sin acentos) y se traduce a bytes del original.
    let (key, map) = crate::index::store::search_key_with_map(text);
    let pos = if needle.is_empty() { None } else { key.find(needle).map(|k| (map[k], map[k + needle.len()])) };
    match pos {
        Some((s, e)) if s < e => {
            job.append(&text[..s], 0.0, TextFormat::simple(font.clone(), color));
            let mut hl = TextFormat::simple(text::strong(), if on_sel && !t.dark { t.accent_hover } else { t.accent });
            hl.background = t.accent_soft;
            job.append(&text[s..e], 0.0, hl);
            job.append(&text[e..], 0.0, TextFormat::simple(font, color));
        }
        _ => job.append(text, 0.0, TextFormat::simple(font, color)),
    }
    job.wrap = TextWrapping::truncate_at_width(max_w.max(10.0));
    p.layout_job(job)
}

/// El pill de una fila: 1 px de aire arriba y abajo, `ROW_INSET` a los lados.
fn row_pill(rect: Rect) -> Rect {
    rect.shrink2(vec2(ROW_INSET, 1.0))
}

/// Selección de una fila: tinte de acento con foco, capa gris sin él.
fn sel_of(selected: bool, active: bool) -> Sel {
    match (selected, active) {
        (false, _) => Sel::None,
        (true, true) => Sel::Focused,
        (true, false) => Sel::Unfocused,
    }
}

/// Fondo de una fila: la selección, o el hover (`hover` de 0 a 1: aparece y se apaga con un fundido).
pub fn row_bg(p: &Painter, rect: Rect, t: &Theme, selected: bool, active: bool, hover: f32) {
    design::state_fill(p, t, row_pill(rect), radius::M, hover, false, sel_of(selected, active));
}

/// Cuánto se ve el hover de este elemento (entra rápido, se apaga un poco más lento: al pasar el
/// mouse por una lista deja una estela corta).
fn hover_amount(resp: &egui::Response, show: bool) -> f32 {
    design::hover_amount(&resp.ctx, resp.id, show && resp.hovered())
}

/// Cuánto cubre el resaltado deslizante a una fila (0 a 1).
fn pill_cover(pill: Rect, r: Rect) -> f32 {
    let fx = 1.0 - ((pill.center().x - r.center().x).abs() / pill.width().max(1.0));
    let fy = 1.0 - ((pill.center().y - r.center().y).abs() / pill.height().max(1.0));
    fx.clamp(0.0, 1.0) * fy.clamp(0.0, 1.0)
}

/// El resaltado que se desliza a la fila `i` (en coordenadas de la lista, que ya incluyen el desplazamiento).
fn pill_rect(ui: &Ui, id: Id, target: Rect) -> Rect {
    let ctx = ui.ctx();
    let x = super::motion::glide(ctx, id.with("px"), target.left() - ui.max_rect().left(), dur::BASE);
    let y = super::motion::glide(ctx, id.with("py"), target.top() - ui.max_rect().top(), dur::BASE);
    Rect::from_min_size(ui.max_rect().min + vec2(x, y), target.size())
}

/// Ícono del estado en la nube (OneDrive): solo en la nube, descargado, siempre en el equipo.
pub fn cloud_glyph(t: &Theme, c: fsx::Cloud) -> Option<(&'static str, Color32, &'static str)> {
    match c {
        fsx::Cloud::None => None,
        fsx::Cloud::OnlineOnly => Some((ic::CLOUD, t.dim, t!("lista.solo_nube_descarga_abrirlo"))),
        fsx::Cloud::Local => Some((ic::CLOUD_CHECK, t.dim, t!("lista.disponible_este_equipo_sincronizado"))),
        fsx::Cloud::Pinned => Some((egui_phosphor::fill::CHECK_CIRCLE, t.ok, t!("lista.siempre_este_equipo"))),
    }
}

/// Ancla de desplazamiento: cuando la lista cambia debajo (el listado del disco reemplaza al del
/// índice, aparece un archivo arriba), el primer elemento visible queda en el mismo lugar de la
/// pantalla en vez de correrse. Se guarda (lista, nombre del primero visible, cuánto se pasó de su
/// borde) en cada cuadro; si la lista cambió y ese nombre está en otro lugar, devuelve el
/// desplazamiento que lo deja donde estaba. Con un botón apretado (arrastre, marquesina) espera.
#[derive(Clone)]
struct Anchor {
    list: (usize, usize),
    name: String,
    idx: usize,
    top: f32,
    past: f32,
}

/// O(1) por cuadro: solo si la lista es otra y el ancla ya no está en su lugar se la busca.
fn anchor_fix(ui: &Ui, id: Id, entries: &[FileEntry], top_of: impl Fn(usize) -> f32) -> Option<f32> {
    let _p = super::perf::scope("lista.ancla");
    let list = (entries.as_ptr() as usize, entries.len());
    let (same, moved) = ui.data_mut(|d| d.get_temp_mut_or_default::<Option<Anchor>>(id).as_ref().map(|a| (a.list == list, entries.get(a.idx).is_none_or(|e| e.name != a.name))))?;
    if same || !moved || ui.input(|i| i.pointer.any_down()) {
        return None;
    }
    let a: Anchor = ui.data(|d| d.get_temp::<Option<Anchor>>(id)).flatten()?;
    let i = entries.iter().position(|e| e.name == a.name)?;
    let top = top_of(i);
    ((top - a.top).abs() > 0.5).then_some((top + a.past).max(0.0))
}

fn anchor_keep(ui: &Ui, id: Id, entries: &[FileEntry], first: usize, top: f32, viewport_top: f32) {
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let Some(e) = entries.get(first) else { return };
    let list = (entries.as_ptr() as usize, entries.len());
    ui.data_mut(|d| {
        let slot = d.get_temp_mut_or_default::<Option<Anchor>>(id);
        match slot {
            // Sin asignar si no cambió nada (el nombre solo se copia al cambiar el primero visible).
            Some(a) if a.idx == first && a.name == e.name => {
                (a.list, a.top, a.past) = (list, top, viewport_top - top);
            }
            _ => *slot = Some(Anchor { list, name: e.name.clone(), idx: first, top, past: viewport_top - top }),
        }
    });
}

/// Lista virtualizada: solo se dibujan las filas visibles.
/// `glide`: fila del resaltado deslizante y su color (si hay una sola seleccionada); a cada fila
/// se le pasa dónde está ese resaltado.
fn virtual_list(ui: &mut Ui, id: Id, n: usize, scroll_to: Option<usize>, glide: Option<(usize, Color32)>, mut row: impl FnMut(&mut Ui, usize, Rect, Option<Rect>)) -> egui::scroll_area::ScrollAreaOutput<()> {
    virtual_list_in(ui, id, n, None, scroll_to, glide, &mut row)
}

/// `virtual_list` con el ancla de desplazamiento sobre `entries` (ver `anchor_fix`).
fn virtual_list_in(ui: &mut Ui, id: Id, n: usize, entries: Option<&[FileEntry]>, scroll_to: Option<usize>, glide: Option<(usize, Color32)>, row: &mut impl FnMut(&mut Ui, usize, Rect, Option<Rect>)) -> egui::scroll_area::ScrollAreaOutput<()> {
    let anchor_id = id.with("ancla");
    let mut area = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]);
    if let Some(off) = entries.filter(|_| scroll_to.is_none()).and_then(|e| anchor_fix(ui, anchor_id, e, |i| i as f32 * ROW_H)) {
        area = area.vertical_scroll_offset(off);
    }
    let out = area.show_viewport(ui, |ui, viewport| {
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
            ui.painter().rect_filled(row_pill(r), radius::M, color);
            r
        });
        let first = (viewport.min.y / ROW_H).floor().max(0.0) as usize;
        let last = ((viewport.max.y / ROW_H).ceil() as usize + 1).min(n);
        for i in first..last {
            let r = Rect::from_min_size(pos2(left, top + i as f32 * ROW_H), vec2(width, ROW_H));
            row(ui, i, r, pill);
        }
        if let Some(e) = entries {
            anchor_keep(ui, anchor_id, e, first, first as f32 * ROW_H, viewport.min.y);
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
    p.rect_filled(screen, radius::S, cx.t.accent_soft);
    p.rect_stroke(screen, radius::S, egui::Stroke::new(stroke::HAIR, cx.t.accent), egui::StrokeKind::Inside);
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

/// Mientras se lee una carpeta: nada los primeros `LOADER_AFTER` (en un SSD llega antes y no
/// destella ninguna etiqueta); después «Cargando», y desde el segundo, en la red «Esperando al
/// servidor» y en un disco mecánico o USB «Despertando el disco» (dormido, tarda segundos).
fn loading_message(ui: &mut Ui, t: &Theme, dir: &str) {
    let id = Id::new("carpeta-cargando");
    let now = ui.input(|i| i.time);
    let since = match ui.data(|d| d.get_temp::<(String, f64)>(id)) {
        Some((d, at)) if d == dir => at,
        _ => {
            ui.data_mut(|d| d.insert_temp(id, (dir.to_string(), now)));
            now
        }
    };
    let waited = now - since;
    if waited < super::motion::LOADER_AFTER {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(super::motion::LOADER_AFTER - waited));
        return;
    }
    if waited < 1.0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(1.0 - waited));
    }
    let msg = if waited < 1.0 {
        t!("lista.cargando")
    } else if fsx::is_remote(dir) {
        t!("lista.esperando_servidor")
    } else if !fsx::disks::volume(dir).kind.parallel_friendly() {
        t!("lista.despertando_disco")
    } else {
        t!("lista.cargando")
    };
    empty_message(ui, t, "", msg);
}

/// Vacío o cargando, en lo que queda del área. En una zona grande, el estado vacío con trama e
/// ícono; en una columna, solo la micro-etiqueta (`icon` vacío: cargando, sin ícono).
fn empty_message(ui: &mut Ui, t: &Theme, icon: &str, msg: &str) {
    let area = ui.available_rect_before_wrap();
    let p = ui.painter_at(area);
    if icon.is_empty() {
        design::micro(&p, area.center(), Align2::CENTER_CENTER, msg, t.faint);
    } else {
        design::empty_state(&p, t, area, icon, msg, "");
    }
}

/// Espera con trabajo detrás (el índice se está construyendo): el medidor que recorre, centrado.
fn busy_message(ui: &mut Ui, t: &Theme, msg: &str) {
    let area = ui.available_rect_before_wrap();
    let r = Rect::from_center_size(area.center(), vec2(area.width(), h::HEAD));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Center)));
    design::busy(&mut child, t, msg);
}

/// Parte un texto en líneas de hasta `max_w` (por palabras).
fn wrap_lines(p: &Painter, text: &str, font: FontId, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        let next = if cur.is_empty() { w.to_string() } else { format!("{cur} {w}") };
        if cur.is_empty() || design::text_width(p, &next, font.clone()) <= max_w {
            cur = next;
        } else {
            lines.push(std::mem::replace(&mut cur, w.to_string()));
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Una carpeta que no se pudo leer: «No se puede abrir» con el ícono en `warn` y el error como
/// pista (en varias líneas si es largo). En una columna angosta, la micro-etiqueta en `warn` y,
/// debajo, el error.
fn error_message(ui: &mut Ui, t: &Theme, err: &str) {
    let area = ui.available_rect_before_wrap();
    let p = ui.painter_at(area);
    let title = t!("lista.no_puede_abrir_titulo");
    let max_w = area.width() - 2.0 * sp::L;
    // Lo mismo que mide `empty_state_lines` para decidir si va el estado completo.
    if area.width() >= 320.0 && area.height() >= 240.0 {
        let lines = wrap_lines(&p, err, text::small(), max_w.min(480.0));
        let hints: Vec<&str> = lines.iter().take(4).map(String::as_str).collect();
        design::empty_state_lines(&p, t, area, ic::WARNING, Tone::Warn, title, &hints);
        return;
    }
    let mut job = LayoutJob::single_section(err.to_owned(), TextFormat::simple(text::small(), t.faint));
    job.wrap.max_width = max_w;
    job.halign = egui::Align::Center;
    let g = p.layout_job(job);
    let title_h = p.layout_job(design::micro_job(title, t.warn)).size().y;
    let top = area.center().y - (title_h + sp::S + g.size().y) / 2.0;
    let tr = design::micro(&p, pos2(area.center().x, top), Align2::CENTER_TOP, title, t.warn);
    p.galley(pos2(area.center().x, tr.bottom() + sp::S), g, t.faint);
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

/// Ancho mínimo del campo de renombrar cuando tiene que dejar lugar al aviso de extensión.
const RENAME_MIN_W: f32 = 140.0;

/// Fila de renombrado en línea. `limit`: hasta dónde puede llegar (el fin de la columna del nombre
/// en detalles; en columnas y cuadrícula, el borde de la fila). Ni el campo ni el aviso lo pasan.
fn rename_row(ui: &mut Ui, cx: &mut Cx, rect: Rect, x: f32, limit: f32) {
    let Some(rn) = cx.rename.as_mut() else { return };
    let t = *cx.t;
    // Cambiar la extensión puede dejar el archivo sin poder abrirse: se avisa al lado con un chip,
    // sin el cuadro de diálogo de Windows (y se deshace con Ctrl+Z).
    let ext = ext_change(&rn.old, &rn.buf).filter(|_| !std::path::Path::new(&fsx::long_path(&fsx::join(&rn.dir, &rn.old))).is_dir());
    let chip_txt = ext.as_ref().map(|(a, b)| format!("{a} → {b}")).unwrap_or_default();
    let full = Chip { icon: ic::WARNING, compact: true, on_sel: true, ..Chip::new(&chip_txt, Tone::Warn) };
    let bare = Chip { text: "", ..full };
    // Tan ancho como el nombre (con aire para escribir), no toda la fila.
    let text_w = design::text_width(ui.painter(), &rn.buf, text::body());
    let natural = (x + (text_w + 60.0).max(220.0)).min(limit);
    // El chip entra completo, o el campo se achica para dejarle lugar, o queda solo el ícono.
    let chip = ext.as_ref().and_then(|_| {
        [full, bare].into_iter().find_map(|c| {
            let cw = design::chip_width(ui.painter(), &c);
            let right = natural.min(limit - sp::S - cw);
            (right - x >= RENAME_MIN_W.min(natural - x)).then_some((c, right))
        })
    });
    let right = chip.map_or(natural, |(_, r)| r);
    let edit_rect = Rect::from_min_max(pos2(x, rect.top() + sp::XXS), pos2(right, rect.bottom() - sp::XXS));
    ui.painter().rect_filled(edit_rect, radius::M, t.sidebar);
    design::focus_ring(ui.painter(), &t, edit_rect, radius::M);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect.shrink2(vec2(sp::XS, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let id = Id::new("rename-edit");
    let out = egui::TextEdit::singleline(&mut rn.buf).id(id).frame(egui::Frame::NONE).font(text::body()).desired_width(edit_rect.width() - 2.0 * sp::XS).vertical_align(egui::Align::Center).show(&mut child);
    if rn.first_frame {
        out.response.request_focus();
        let stem = rn.buf.rfind('.').filter(|&i| i > 0).unwrap_or(rn.buf.len());
        let chars = rn.buf[..stem].chars().count();
        let mut state = out.state.clone();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
        state.store(ui.ctx(), id);
        rn.first_frame = false;
    }
    if let (Some((c, _)), Some((a, b))) = (chip, &ext) {
        let cr = design::chip_paint(ui.painter(), &t, pos2(edit_rect.right() + sp::S, rect.center().y), &c, false);
        // La frase completa, en el tooltip del chip.
        let hover = ui.interact(cr, id.with("ext-chip"), Sense::hover());
        design::tip(hover, &t, &tf!("lista.cambia_extension", antes = a, despues = b));
    }
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if esc {
        *cx.rename = None;
    } else if enter || out.response.lost_focus() {
        cx.actions.push(Action::CommitRename);
    }
}

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

/// Punto de etiqueta (8 px, `design::tag_dot`); se pisan 1 px.
const TAG_DOT: f32 = 8.0;
const TAG_STEP: f32 = TAG_DOT - 1.0;

/// Cuánto ocupan los puntos de etiqueta de `mask` (con el aire que los separa del nombre).
fn tag_dots_width(mask: u8) -> f32 {
    if mask == 0 { 0.0 } else { sp::S + TAG_STEP * mask.count_ones() as f32 + 1.0 + sp::XS }
}

/// Los puntos de color de las etiquetas de un archivo, desde `x` (centrados en `cy`).
fn tag_dots(p: &Painter, t: &Theme, x: f32, cy: f32, mask: u8) {
    let mut w = 0.0;
    for i in (0..7).filter(|i| mask & (1 << i) != 0) {
        design::tag_dot(p, t, pos2(x + w + TAG_DOT / 2.0, cy), t.tags[i]);
        w += TAG_STEP;
    }
}

/// El ícono de un archivo: el de Windows si ya llegó (el de cada programa, o el de su tipo: dice
/// con qué se abre), en filas y en mosaico, a color y sin tratar. Mientras llega, o si Windows no
/// da ninguno, el glifo de Rayo: contorno en `dim` en las filas de 16, relleno y en el color del
/// tipo en el mosaico. La carpeta: siempre el glifo relleno en `folder`. No volver a limitar las
/// filas al glifo (decisión del dueño, ver diseno-rayo.md §3.3). `glyph`: tamaño del de Rayo.
/// `a`: opacidad (atenuado o fundido).
#[allow(clippy::too_many_arguments)]
pub fn file_icon(p: &Painter, t: &Theme, center: Pos2, points: f32, glyph: f32, path: &str, is_dir: bool, ext: &str, a: f32) {
    // Una descarga en curso: la flecha de descarga (no el ícono de un archivo a medias).
    if !is_dir && fsx::downloading(&fsx::file_name(path)).is_some() {
        p.text(center, Align2::CENTER_CENTER, ic::DOWNLOAD_SIMPLE, design::icon_fill(glyph), alpha(t.accent, a));
        return;
    }
    let large = points > icon::M;
    // El glifo de Rayo: en color y relleno en el mosaico; en las filas, contorno y gris.
    let rayo = |k: f32| {
        let (g, c) = icons::for_entry(t, is_dir, ext);
        let (font, c) = if large || is_dir { (design::icon_fill(glyph), c) } else { (design::icon_font(glyph), t.dim) };
        p.text(center, Align2::CENTER_CENTER, g, font, alpha(c, a * k));
    };
    if !is_dir {
        let px = super::shellicons::px_for(p.ctx(), points);
        if let Some((tex, shown)) = super::shellicons::get(path, ext, px) {
            // Recién llegado: el de Rayo se desvanece mientras aparece el de Windows.
            if shown < 1.0 {
                rayo(1.0 - shown);
            }
            // Alineado a los píxeles de la pantalla y del tamaño exacto: nítido, como en el Explorador.
            let ppp = p.ctx().pixels_per_point();
            let side = px as f32 / ppp;
            let min = center - vec2(side, side) / 2.0;
            let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            let r = Rect::from_min_size(min, vec2(side, side));
            p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), alpha(Color32::WHITE, a * shown));
            return;
        }
    }
    rayo(1.0);
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
    if !sent && now - since >= super::motion::DWELL_PREFETCH {
        cx.actions.push(Action::Prefetch(path.to_string()));
        ctx.data_mut(|d| d.insert_temp(key, (who, since, true)));
    } else if !sent {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

/// Dibuja una fila de archivo genérica (ícono + nombre + texto a la derecha).
#[allow(clippy::too_many_arguments)]
fn file_row(ui: &mut Ui, cx: &mut Cx, rect: Rect, e: &FileEntry, dir: &str, selected: bool, active: bool, right: Option<&str>, chevron: bool, name_limit: Option<f32>) -> egui::Response {
    let id = Id::new(("row", cx.tab_id, dir, &e.name));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    if e.is_dir {
        prefetch(cx, &resp, &fsx::join(dir, &e.name), selected && active);
    }
    let p = ui.painter_at(rect);
    let t = *cx.t;
    let pill = cx.pill;
    // Con el resaltado deslizante, la fila no dibuja su selección (ya está debajo).
    let hover = hover_amount(&resp, !selected || pill.is_some_and(|pl| pill_cover(pl, rect) < 0.5));
    row_bg(&p, rect, &t, selected && pill.is_none(), active, hover);
    if e.is_dir {
        drop_highlight(ui, &t, &resp, rect);
    }
    // Con la selección suave, el texto y el ícono no cambian de color sobre la fila seleccionada.
    let path = fsx::join(dir, &e.name);
    let faded = e.hidden || cx.cut.contains(&path);
    let a = if faded { self::faded(&t) } else { 1.0 };
    let indent = if cx.tree.is_some() { TREE_CHEVRON + tree_depth(&e.name) as f32 * TREE_STEP } else { 0.0 };
    file_icon(&p, &t, pos2(rect.left() + ICON_X + icon::M / 2.0 + indent, rect.center().y), icon::M, icon::M, &path, e.is_dir, &e.ext, a);
    let name_x = rect.left() + TEXT_X + indent;
    // En la lista desplegada se ve solo el nombre (la carpeta ya la da la sangría).
    let shown_name: &str = if cx.tree.is_some() { e.name.rsplit('\\').next().unwrap_or(&e.name) } else { &e.name };
    let mut right_w = ROW_INSET + sp::S;
    if chevron {
        p.text(pos2(rect.right() - right_w, rect.center().y), Align2::RIGHT_CENTER, ic::CARET_RIGHT, design::icon_font(icon::S), t.faint);
        right_w += icon::S + sp::XXS;
    }
    if let Some(rt) = right {
        let c = size_color(&t, e.is_dir);
        let g = p.layout_no_wrap(rt.to_string(), text::mono(), c);
        let w = g.size().x;
        p.galley(pos2(rect.right() - right_w - w, rect.center().y - g.size().y / 2.0), g, c);
        right_w += w + sp::S;
    }
    // El renombrado llega hasta el fin de la columna del nombre (en columnas, hasta el borde).
    let rename_limit = name_limit.unwrap_or(rect.right() - sp::S);
    // Estado en la nube: al final del nombre (en detalles, al final de su columna).
    let mut name_limit = name_limit;
    if let Some((g, c, tip)) = cloud_glyph(&t, e.cloud) {
        let slot = icon::S + sp::S;
        let x = match name_limit {
            Some(l) => {
                name_limit = Some(l - slot);
                l - sp::XS
            }
            None => {
                let x = rect.right() - right_w;
                right_w += slot;
                x
            }
        };
        let ir = Rect::from_center_size(pos2(x - icon::M / 2.0, rect.center().y), vec2(icon::M, icon::M));
        p.text(ir.center(), Align2::CENTER_CENTER, g, design::icon_font(icon::S), c);
        if resp.hovered() && ui.rect_contains_pointer(ir) {
            design::tip(resp.clone(), &t, tip);
        }
    }
    let renaming = cx.rename.as_ref().is_some_and(|r| r.dir == dir && r.old == e.name);
    if renaming {
        rename_row(ui, cx, rect, name_x - sp::XS, rename_limit);
    } else {
        let name_c = alpha(t.text, a);
        let limit = name_limit.unwrap_or(f32::INFINITY).min(rect.right() - right_w);
        match fsx::downloading(&e.name).filter(|_| !e.is_dir) {
            // Descarga en curso: el nombre que tendrá y "Descargando…".
            Some(base) => {
                // Sobre la selección, en claro, el acento baja a `accent_hover` (contraste, T4).
                let tag_c = if selected && !t.dark { t.accent_hover } else { t.accent };
                let tag = p.layout_no_wrap(t!("lista.descargando").to_string(), text::small(), tag_c);
                let g = elided(&p, base, text::body(), name_c, (limit - name_x - tag.size().x - sp::S).max(20.0));
                let gw = g.size().x;
                p.galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, name_c);
                p.galley(pos2(name_x + gw + sp::S, rect.center().y - tag.size().y / 2.0), tag, tag_c);
            }
            None => {
                let mask = fsx::tags::get(&path);
                let dots = tag_dots_width(mask);
                // Estado en git (si la carpeta es parte de un repositorio): una letra o, en
                // carpetas con cambios adentro, un punto.
                let git = fsx::git::state_of(&path, e.is_dir);
                let git_w = if git.is_some() { 18.0 } else { 0.0 };
                let max_w = limit - name_x - dots - git_w;
                // Vista plana: el nombre trae la subcarpeta adelante.
                let g = if cx.tree.is_none() && e.name.contains('\\') { flat_name(&p, &t, &e.name, name_c, a, max_w) } else { elided(&p, shown_name, text::body(), name_c, max_w) };
                let gw = g.size().x;
                p.galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, name_c);
                if mask != 0 {
                    tag_dots(&p, &t, name_x + gw + sp::S, rect.center().y, mask);
                }
                if let Some((st, inside)) = git {
                    use fsx::git::State;
                    let c = match st {
                        State::Modified | State::Renamed => t.warn,
                        State::Added | State::Untracked => t.ok,
                        State::Deleted | State::Conflict => t.danger,
                    };
                    let x = name_x + gw + dots + sp::S;
                    if inside {
                        p.circle_filled(pos2(x + GIT_DOT / 2.0, rect.center().y), GIT_DOT / 2.0, c);
                    } else {
                        p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, st.letter(), text::mono_s(), c);
                    }
                }
            }
        }
    }
    resp
}

/// Punto de git en carpetas con cambios adentro.
const GIT_DOT: f32 = 6.0;

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
        let w = ctx.fonts_mut(|f| f.layout_no_wrap(e.name.clone(), text::body(), Color32::WHITE).size().x);
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
fn handle_row_response(cx: &mut Cx, resp: &egui::Response, ci: usize, dir: &str, e: &FileEntry, selected: bool, selection: &[String], marquee_x: Option<f32>, name_area: Rect, measure: bool) -> bool {
    slow_rename(cx, resp, dir, e, selected, selection, name_area, measure);
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

/// Color del tamaño en una fila: el de una carpeta es aproximado (se suma por detrás), así que va
/// más apagado que el de un archivo.
fn size_color(t: &Theme, is_dir: bool) -> Color32 {
    if is_dir { t.faint } else { t.dim }
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

/// Sangría de una fila de la lista desplegada (por su profundidad), y el lugar del triángulo.
const TREE_STEP: f32 = 16.0;
const TREE_CHEVRON: f32 = 16.0;
/// Sin nada desplegado: centro del triángulo que aparece sobre la carpeta bajo el puntero, en los
/// 12 px a la izquierda del ícono (no corre los íconos).
const TREE_CARET_X: f32 = 6.0;

fn tree_depth(name: &str) -> usize {
    name.matches('\\').count()
}

pub fn column(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, active: bool, data: ColumnData) {
    let _p = super::perf::scope("lista.columna");
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
        error_message(ui, cx.t, err);
        return;
    }
    if data.entries.is_empty() {
        if data.loading {
            loading_message(ui, cx.t, &dir);
        } else {
            empty_message(ui, cx.t, ic::FOLDER_OPEN, t!("lista.carpeta_vacia"));
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
        let name_area = Rect::from_min_max(pos2(r.left() + TEXT_X, r.top()), pos2(r.right() - 12.0, r.bottom()));
        handle_row_response(cx, &resp, ci, &dir, e, selected, &selection, None, name_area, true);
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

/// Columnas extra que se pueden mostrar en la vista de detalles (clic derecho en el encabezado).
pub const EXTRA_COLUMNS: &[(&str, f32)] = &[("created", 150.0), ("ext", 70.0), ("dims", 110.0), ("duration", 80.0), ("tags", 70.0)];

static EXTRA: parking_lot::Mutex<Vec<&'static str>> = parking_lot::Mutex::new(Vec::new());

pub fn set_extra_columns(ids: &[String]) {
    *EXTRA.lock() = EXTRA_COLUMNS.iter().filter(|(id, _)| ids.iter().any(|x| x == id)).map(|(id, _)| *id).collect();
}

fn extra_columns() -> Vec<(&'static str, f32)> {
    let on = EXTRA.lock();
    EXTRA_COLUMNS.iter().filter(|(id, _)| on.contains(id)).copied().collect()
}

pub fn extra_label(id: &str) -> &'static str {
    match id {
        "created" => t!("previa.creado"),
        "ext" => t!("lista.extension"),
        "dims" => t!("previa.dimensiones"),
        "duration" => t!("previa.duracion"),
        _ => t!("lista.etiquetas"),
    }
}

/// Clic derecho en un encabezado: casillas para mostrar u ocultar las columnas extra.
fn column_menu(resp: &egui::Response, cx: &mut Cx) {
    let on_now: Vec<&str> = extra_columns().iter().map(|(id, _)| *id).collect();
    let t = *cx.t;
    design::context_menu(resp, &t, |ui| {
        for (id, _) in EXTRA_COLUMNS {
            let on = on_now.contains(id);
            if design::menu_item_ex(ui, &t, design::MenuItem { label: extra_label(id), checked: Some(on), ..Default::default() }).clicked() {
                cx.actions.push(Action::ToggleColumn(id.to_string()));
            }
        }
    });
}

/// El valor de una columna extra para un archivo.
fn extra_cell(p: &Painter, t: &Theme, id: &str, e: &FileEntry, path: &str, x: f32, w: f32, cy: f32) {
    let text = match id {
        "created" => (e.ctime != 0).then(|| fsx::fmt_date(e.ctime)),
        "ext" => (!e.is_dir && !e.ext.is_empty()).then(|| e.ext.to_uppercase()),
        "dims" | "duration" if !e.is_dir => fsx::props::info(path).and_then(|i| {
            if id == "dims" {
                i.dims.clone()
            } else {
                i.rows.iter().find(|(f, _)| *f == fsx::props::Field::Duration).map(|(_, v)| v.clone())
            }
        }),
        "tags" => {
            let mask = fsx::tags::get(path);
            if mask != 0 {
                tag_dots(p, t, x + CELL_X, cy, mask);
            }
            None
        }
        _ => None,
    };
    if let Some(s) = text {
        // La extensión es un tipo (`small`); fecha, medidas y duración son datos (mono).
        let font = if id == "ext" { text::small() } else { text::mono() };
        let g = elided(p, &s, font, t.dim, w - CELL_X - sp::XS);
        p.galley(pos2(x + CELL_X, cy - g.size().y / 2.0), g, t.dim);
    }
}

fn columns() -> ColumnWidths {
    *COLUMNS.lock()
}

/// Alto de la barra de "qué ocupa espacio", bajo el tamaño.
const SPACE_BAR_H: f32 = 3.0;

/// Por debajo de este ancho de "Modificado" no entra la píldora de antigüedad junto a la fecha.
const DATE_WITH_AGE: f32 = 176.0;

/// Borde arrastrable entre dos columnas (en el encabezado): una línea fina que, al arrastrarla,
/// se marca en acento.
fn column_handle(ui: &Ui, t: &Theme, id: Id, x: f32, rows: egui::Rangef) -> egui::Response {
    let r = Rect::from_x_y_ranges(x - 4.0..=x + 4.0, rows);
    let resp = design::tip(ui.interact(r, id, Sense::click_and_drag()), t, t!("lista.ajustar_columna"));
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
    }
    let (st, inset) = if resp.dragged() {
        (egui::Stroke::new(stroke::MARK, t.accent), sp::XS)
    } else if resp.hovered() {
        (egui::Stroke::new(stroke::HAIR, t.border_strong), sp::XS)
    } else {
        (egui::Stroke::new(stroke::HAIR, t.border), (r.height() - sp::M) / 2.0)
    };
    ui.painter().line_segment([pos2(x, r.top() + inset), pos2(x, r.bottom() - inset)], st);
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

/// Color de una micro-etiqueta de encabezado: la columna ordenada en `text`; con hover, `dim`.
fn header_color(t: &Theme, active: bool, hovered: bool) -> Color32 {
    if active {
        t.text
    } else if hovered {
        t.dim
    } else {
        t.faint
    }
}

/// Flecha del orden, en acento, junto a la etiqueta `lr` (a su izquierda si la columna va a la derecha).
fn sort_caret(p: &Painter, t: &Theme, lr: Rect, right: bool, desc: bool) {
    let arrow = if desc { ic::CARET_DOWN } else { ic::CARET_UP };
    let ax = if right { lr.left() - sp::XS - icon::S / 2.0 } else { lr.right() + sp::XS + icon::S / 2.0 };
    p.text(pos2(ax, lr.center().y), Align2::CENTER_CENTER, arrow, design::icon_font(icon::S), t.accent);
}

/// Antigüedad junto a la fecha: el calor se lee por brillo, no por tono. Lo tocado en las últimas
/// 24 h es «ahora» (chip de acento); hasta 30 días, `dim`; más viejo, `faint`. `on_sel`: la fila
/// está seleccionada con foco (en claro, el chip baja a `accent_hover`).
fn age_pill(p: &Painter, t: &Theme, x_right: f32, cy: f32, mtime: u32, now: u32, on_sel: bool) {
    const DAY: u32 = 86_400;
    if mtime == 0 {
        return;
    }
    let age = now.saturating_sub(mtime);
    let txt = fsx::fmt_age(mtime, now);
    if age < DAY {
        // Compacto y con el borde 2 px adentro: en una fila de 26 no toca a las de al lado, y su
        // texto termina donde terminaría el de las otras fechas (`x_right - 8`).
        let c = Chip { mono: true, compact: true, on_sel, ..Chip::new(&txt, Tone::Accent) };
        let w = design::chip_width(p, &c);
        design::chip_paint(p, t, pos2(x_right - 2.0 - w, cy), &c, false);
    } else {
        let c = if age <= 30 * DAY { t.dim } else { t.faint };
        p.text(pos2(x_right - sp::S, cy), Align2::RIGHT_CENTER, txt, text::mono_s(), c);
    }
}

pub fn details(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, focused: bool, data: ColumnData, sort: (SortKey, bool)) {
    let _p = super::perf::scope("lista.detalles");
    let t = *cx.t;
    // Encabezado: micro-etiquetas; la columna ordenada en `text` con su flecha en acento.
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    let p = ui.painter_at(hrect);
    // Por dentro del rectángulo: en `bottom()` la recortaría `painter_at`.
    p.hline(hrect.x_range(), hrect.bottom() - 0.5, egui::Stroke::new(stroke::HAIR, t.border));
    let cols_x = details_columns(hrect);
    // Con la lista desplegada, el nombre de las filas se corre la columna del triángulo.
    let name_x = TEXT_X + if cx.tree.is_some() { TREE_CHEVRON } else { 0.0 };
    let header = |key: SortKey, label: &str, x0: f32, x1: f32, right: bool, cx: &mut Cx| {
        let r = Rect::from_x_y_ranges(x0..=x1, hrect.y_range());
        let resp = ui.interact(r, Id::new(("hdr", cx.tab_id, label)), Sense::click());
        let active = sort.0 == key;
        // Cada encabezado se alinea con sus valores (el nombre, con el texto de las filas).
        let (pos, align) = if right {
            (pos2(x1 - CELL_X, r.center().y), Align2::RIGHT_CENTER)
        } else {
            (pos2(x0 + if key == SortKey::Name { name_x } else { CELL_X }, r.center().y), Align2::LEFT_CENTER)
        };
        let lr = design::micro(&p, pos, align, label, header_color(&t, active, resp.hovered()));
        if active {
            sort_caret(&p, &t, lr, right, sort.1);
        }
        if resp.clicked() {
            cx.actions.push(Action::SetSort(key));
        }
        // Clic derecho en un encabezado: qué columnas extra mostrar.
        column_menu(&resp, cx);
    };
    let extras = extra_columns_fit(hrect);
    let name_end = cols_x.1 - extras.iter().map(|(_, w)| w).sum::<f32>();
    header(SortKey::Name, t!("lista.nombre"), cols_x.0, name_end, false, cx);
    let mut ex = name_end;
    for (id, w) in &extras {
        design::micro(&p, pos2(ex + CELL_X, hrect.center().y), Align2::LEFT_CENTER, extra_label(id), t.faint);
        ex += w;
    }
    // Los encabezados de las columnas extra también abren el menú.
    let mut ex = name_end;
    for (id, w) in &extras {
        let resp = ui.interact(Rect::from_x_y_ranges(ex..=ex + w, hrect.y_range()), Id::new(("hdr-extra", cx.tab_id, *id)), Sense::click());
        column_menu(&resp, cx);
        ex += w;
    }
    if cols_x.2 > cols_x.1 {
        header(SortKey::Modified, t!("comun.modificado"), cols_x.1, cols_x.2, false, cx);
    }
    if cols_x.3 > cols_x.2 {
        header(SortKey::Kind, t!("lista.tipo"), cols_x.2, cols_x.3, false, cx);
    }
    header(SortKey::Size, t!("comun.tamano"), cols_x.3, cols_x.4, true, cx);
    // Bordes entre columnas: se arrastran para cambiar el ancho.
    let d = ColumnWidths::DEFAULT;
    let h = column_handle(ui, &t, Id::new("col-borde-0"), cols_x.1, if cols_x.2 > cols_x.1 { hrect.y_range() } else { egui::Rangef::NOTHING });
    drive_handle(cx, &h, |c, dx| c.date -= dx, |c| c.date = d.date);
    // Cada borde cambia el ancho de la columna de su derecha; lo de la izquierda se mueve con él
    // (el nombre toma la diferencia), así nunca se pisan dos columnas.
    let h = column_handle(ui, &t, Id::new("col-borde-1"), cols_x.2, if cols_x.3 > cols_x.2 { hrect.y_range() } else { egui::Rangef::NOTHING });
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
        error_message(ui, &t, err);
        return;
    }
    if data.entries.is_empty() {
        if data.loading {
            loading_message(ui, &t, &dir);
        } else {
            empty_message(ui, &t, ic::FOLDER_OPEN, t!("lista.carpeta_vacia"));
        }
        return;
    }
    let scroll_to = scroll_target(col, data.entries, data.loading);
    let selection = col.selected();
    // Para las barras de "qué ocupa espacio": proporción de cada elemento sobre el total de la carpeta.
    let total: u64 = data.entries.iter().filter_map(|e| e.display_size()).sum();
    let glide = glide_row(&t, col, data.entries, focused);
    let out = virtual_list_in(ui, Id::new(("det", cx.tab_id, &dir)), data.entries.len(), Some(data.entries), scroll_to, glide, &mut |ui, i, r, pill| {
        cx.pill = pill;
        let e = &data.entries[i];
        let selected = col.is_selected(&e.name);
        let (_, x1, x2, x3, x4) = details_columns(r);
        let name_end = x1 - extras.iter().map(|(_, w)| w).sum::<f32>();
        let resp = file_row(ui, cx, r, e, &dir, selected, focused, None, false, Some(name_end - sp::S));
        let p = ui.painter_at(r);
        // Triángulo para desplegar la carpeta aquí mismo (encima de la fila: recibe el clic).
        if let Some(open) = cx.tree.filter(|_| e.is_dir) {
            let depth = tree_depth(&e.name) as f32;
            let cr = Rect::from_center_size(pos2(r.left() + 13.0 + depth * TREE_STEP, r.center().y), vec2(icon::M, icon::M));
            let cresp = ui.interact(cr, resp.id.with("expand"), Sense::click());
            let is_open = open.contains(&e.lower);
            let glyph = if is_open { ic::CARET_DOWN } else { ic::CARET_RIGHT };
            let c = if cresp.hovered() { t.text } else { t.faint };
            p.text(cr.center(), Align2::CENTER_CENTER, glyph, design::icon_font(icon::S), c);
            if cresp.clicked() {
                cx.actions.push(Action::ToggleExpand(e.name.clone()));
            }
        } else if e.is_dir && cx.tree.is_none() && !e.name.contains('\\') {
            // Sin nada desplegado (y fuera de la vista plana): el triángulo aparece sobre la carpeta
            // bajo el puntero, para que desplegar se descubra con el mouse.
            let cr = Rect::from_center_size(pos2(r.left() + TREE_CARET_X, r.center().y), vec2(icon::S, icon::M));
            let cresp = ui.interact(cr, resp.id.with("expand"), Sense::click());
            if ui.rect_contains_pointer(r) && !drag_hovering(&resp) {
                let c = if cresp.hovered() { t.text } else { t.faint };
                p.text(cr.center(), Align2::CENTER_CENTER, ic::CARET_RIGHT, design::icon_font(icon::S), c);
            }
            if cresp.clicked() {
                cx.actions.push(Action::ToggleExpand(e.name.clone()));
            }
        }
        if cx.space_bars && total > 0 {
            if let Some(sz) = e.display_size() {
                // Un solo color: `dim` tenue; en acento lo que pasa del 25 % de la carpeta.
                let frac = (sz as f64 / total as f64) as f32;
                let bar = Rect::from_min_size(pos2(x3 + CELL_X, r.bottom() - 2.0 * SPACE_BAR_H), vec2((x4 - x3 - 2.0 * CELL_X) * frac, SPACE_BAR_H));
                let c = if frac > 0.25 { t.accent } else { alpha(t.dim, 0.45) };
                p.rect_filled(bar, radius::S, c);
            }
        }
        let mut ex = name_end;
        let full = fsx::join(&dir, &e.name);
        for (id, w) in &extras {
            extra_cell(&p, &t, id, e, &full, ex, *w, r.center().y);
            ex += w;
        }
        if x2 > x1 {
            p.text(pos2(x1 + CELL_X, r.center().y), Align2::LEFT_CENTER, fsx::fmt_date(e.mtime), text::mono(), t.dim);
        }
        if x2 - x1 >= DATE_WITH_AGE {
            age_pill(&p, &t, x2 - sp::S, r.center().y, e.mtime, cx.now, selected);
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
        if x3 > x2 {
            p.text(pos2(x2 + CELL_X, r.center().y), Align2::LEFT_CENTER, kind, text::small(), t.dim);
        }
        if let Some(s) = size_text(e) {
            p.text(pos2(r.right() - CELL_X, r.center().y), Align2::RIGHT_CENTER, s, text::mono(), size_color(&t, e.is_dir));
        }
        // Arrastrar desde fuera del nombre (fecha, tipo, tamaño) dibuja un rectángulo, como en el Explorador.
        let name_area = Rect::from_min_max(pos2(r.left() + TEXT_X, r.top()), pos2(name_end, r.bottom()));
        if handle_row_response(cx, &resp, ci, &dir, e, selected, &selection, Some(x1), name_area, true) {
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
    // Si no caben (un panel angosto), primero se oculta Tipo y después Modificado: nunca se pisan.
    let room = (x3 - r.left() - 160.0).max(0.0);
    let kind = if c.date + c.kind <= room { c.kind } else { 0.0 };
    let date = if c.date <= room - kind { c.date } else if room - kind >= 110.0 { room - kind } else { 0.0 };
    let x2 = x3 - kind;
    let x1 = x2 - date;
    (r.left(), x1, x2, x3, x4)
}

/// Las columnas extra que entran entre el nombre (que conserva al menos 220 px) y la fecha.
fn extra_columns_fit(r: Rect) -> Vec<(&'static str, f32)> {
    let (_, x1, ..) = details_columns(r);
    let mut room = x1 - r.left() - 220.0;
    let mut out = Vec::new();
    for (id, w) in extra_columns() {
        if w > room {
            break;
        }
        room -= w;
        out.push((id, w));
    }
    out
}

// ---------------------------------------------------------------- resultados de búsqueda

/// Posiciones x de las columnas de resultados: (nombre, carpeta, borde derecho del tamaño, fecha).
fn search_columns(r: Rect) -> (f32, f32, f32, f32) {
    let nx = r.left() + TEXT_X;
    let date_x = r.right() - 150.0;
    let size_r = date_x - 14.0;
    let auto = ((size_r - 90.0 - nx) * 0.42).max(140.0);
    let set = columns().search_name;
    let name_w = if set > 0.0 { set.clamp(140.0, (size_r - 90.0 - nx - 120.0).max(140.0)) } else { auto };
    (nx, nx + name_w + 12.0, size_r, date_x)
}

#[allow(clippy::too_many_arguments)]
pub fn search_results(ui: &mut Ui, cx: &mut Cx, res: Option<&Results>, cursor: &mut usize, scroll: &mut bool, indexing: Option<String>, query: &str, sort: (ResultSort, bool)) {
    let _p = super::perf::scope("lista.búsqueda");
    let t = *cx.t;
    let Some(res) = res else {
        match indexing.as_deref() {
            Some(ix) => busy_message(ui, &t, ix),
            None => empty_message(ui, &t, "", t!("lista.buscando")),
        }
        return;
    };
    // Una colección, una etiqueta o las recientes no son una búsqueda: se nombra lo que se muestra.
    if let Some(lc) = list_context(query) {
        ui.horizontal(|ui| {
            ui.add_space(ICON_X);
            match &lc {
                ListContext::Collection(name) => {
                    design::micro_label(ui, t!("menu.coleccion"), t.faint);
                    ui.label(egui::RichText::new(name).font(text::strong()).color(t.text));
                }
                ListContext::Tag(i) => {
                    design::micro_label(ui, t!("menu.etiqueta"), t.faint);
                    let (r, _) = ui.allocate_exact_size(vec2(TAG_DOT, TAG_DOT), Sense::hover());
                    design::tag_dot(ui.painter(), &t, r.center(), t.tags[*i]);
                    ui.label(egui::RichText::new(tag_label(*i)).font(text::strong()).color(t.text));
                }
                ListContext::Recent => {
                    ui.label(egui::RichText::new(t!("barra.recientes")).font(text::strong()).color(t.text));
                }
            }
            ui.label(egui::RichText::new(fsx::thousands(res.total)).font(text::mono_s()).color(t.faint));
            if res.total > res.len() && design::link(ui, &t, t!("lista.ver_todos")).clicked() {
                cx.actions.push(Action::LoadAllResults);
            }
        });
    } else {
        search_header(ui, cx, res, indexing.as_deref(), sort);
    }
    search_body(ui, cx, res, cursor, scroll, query, sort);
}

/// Lo que muestra una pestaña de búsqueda especial.
enum ListContext {
    Collection(String),
    Tag(usize),
    Recent,
}

fn list_context(query: &str) -> Option<ListContext> {
    if let Some(name) = fsx::collections::query_name(query) {
        Some(ListContext::Collection(name))
    } else if let Some(i) = fsx::tags::query_color(query) {
        Some(ListContext::Tag(i.min(6)))
    } else if query.trim_start().to_lowercase().starts_with("recientes:") {
        Some(ListContext::Recent)
    } else {
        None
    }
}

/// Cabecera de una búsqueda: cuántos resultados, en cuánto tiempo (con el rayo) y el chip de relevancia.
fn search_header(ui: &mut Ui, cx: &mut Cx, res: &Results, indexing: Option<&str>, sort: (ResultSort, bool)) {
    let t = *cx.t;
    ui.horizontal(|ui| {
        // Alineado con la columna de íconos de las filas.
        ui.add_space(ICON_X);
        let shown = res.len();
        let txt = if res.total > shown {
            tf!("lista.resultados_mejores", n = fsx::thousands(res.total), mostrados = fsx::thousands(shown))
        } else {
            tp!(res.total, "lista.resultados", n = fsx::thousands(res.total))
        };
        ui.label(egui::RichText::new(txt).font(text::strong()).color(t.text));
        // El tiempo con el rayo: lo instantáneo es «ahora».
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = sp::XXS;
            ui.label(egui::RichText::new(egui_phosphor::fill::LIGHTNING).font(design::icon_fill(icon::S)).color(t.accent));
            let ms = tf!("lista.milisegundos", ms = format!("{:.2}", res.ms));
            ui.label(egui::RichText::new(ms.trim_start_matches(['·', ' '])).font(text::mono_s()).color(t.accent));
        });
        if res.total > shown && design::link(ui, &t, t!("lista.ver_todos")).clicked() {
            cx.actions.push(Action::LoadAllResults);
        }
        if let Some(ix) = indexing {
            ui.label(egui::RichText::new(format!("· {ix}")).font(text::small()).color(t.faint));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(sp::M - ui.spacing().item_spacing.x);
            let on = sort.0 == ResultSort::Relevance;
            let c = Chip { icon: ic::SPARKLE, interactive: true, ..Chip::new(t!("lista.relevancia"), if on { Tone::Accent } else { Tone::Neutral }) };
            if design::tip(design::chip(ui, &t, c).response, &t, t!("lista.ordenar_relevancia")).clicked() {
                cx.actions.push(Action::SetResultSort(ResultSort::Relevance));
            }
        });
    });
}

/// Encabezados de columna y filas de los resultados.
fn search_body(ui: &mut Ui, cx: &mut Cx, res: &Results, cursor: &mut usize, scroll: &mut bool, query: &str, sort: (ResultSort, bool)) {
    let t = *cx.t;
    // Encabezados para ordenar por columna (como en Everything).
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    {
        let p = ui.painter_at(hrect);
        p.hline(hrect.x_range(), hrect.bottom() - 0.5, egui::Stroke::new(stroke::HAIR, t.border));
        let (nx, dir_x, size_r, date_x) = search_columns(hrect);
        let cols = [
            (ResultSort::Name, t!("lista.nombre"), nx, false),
            (ResultSort::Folder, t!("comun.carpeta"), dir_x, false),
            (ResultSort::Size, t!("comun.tamano"), size_r, true),
            (ResultSort::Modified, t!("comun.modificado"), date_x, false),
        ];
        for (key, label, x, right) in cols {
            let active = sort.0 == key;
            let w = p.layout_job(design::micro_job(label, t.faint)).size().x + sp::L;
            let r = if right { Rect::from_x_y_ranges(x - w..=x + 4.0, hrect.y_range()) } else { Rect::from_x_y_ranges(x - 4.0..=x + w, hrect.y_range()) };
            let resp = ui.interact(r, Id::new(("shdr", cx.tab_id, label)), Sense::click());
            let align = if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
            let lr = design::micro(&p, pos2(x, hrect.center().y), align, label, header_color(&t, active, resp.hovered()));
            if active {
                sort_caret(&p, &t, lr, right, sort.1);
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
        empty_message(ui, &t, ic::MAGNIFYING_GLASS, t!("lista.sin_resultados"));
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
        // Con la selección suave, el ícono y el resaltado de la coincidencia no cambian sobre la fila seleccionada.
        file_icon(&p, &t, pos2(r.left() + ICON_X + icon::M / 2.0, r.center().y), icon::M, icon::M, &row.path(), row.is_dir, &row.ext, 1.0);
        let (nx, dir_x, size_r, date_x) = search_columns(r);
        let name_w = dir_x - nx - 12.0;
        let g = highlighted(&p, &row.name, &needle, &t, t.text, name_w, selected);
        p.galley(pos2(nx, r.center().y - g.size().y / 2.0), g, t.text);
        // La carpeta, acortada por el medio: lo que distingue dos rutas largas es el final.
        let dir_w = size_r - 90.0 - dir_x;
        let shown_dir = design::elide_middle(&p, &row.dir, text::mono(), dir_w);
        let dr = p.text(pos2(dir_x, r.center().y), Align2::LEFT_CENTER, &shown_dir, text::mono(), t.dim);
        if shown_dir != row.dir && resp.hovered() && ui.rect_contains_pointer(dr) {
            design::tip(resp.clone(), &t, &row.dir);
        }
        p.text(pos2(date_x, r.center().y), Align2::LEFT_CENTER, fsx::fmt_date(row.mtime), text::mono(), t.dim);
        p.text(pos2(size_r, r.center().y), Align2::RIGHT_CENTER, fsx::human_size(row.size), text::mono(), size_color(&t, row.is_dir));
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
/// Margen de la cuadrícula alrededor de las celdas.
const GRID_PAD: f32 = 6.0;
/// Aire entre el borde de la celda y el mosaico (lo que se ve y recibe el clic).
const TILE_INSET: f32 = 3.0;
/// Íconos de contenido del mosaico (no son íconos de interfaz): ícono de Windows en píxeles
/// lógicos y glifo de Rayo; carpeta y emblema de las carpetas conocidas.
const TILE_ICON: f32 = 64.0;
const TILE_GLYPH: f32 = 58.0;
const TILE_FOLDER: f32 = 84.0;
const TILE_EMBLEM: f32 = 22.0;
/// Alto de la zona de la miniatura.
const THUMB_H: f32 = 92.0;

/// El mosaico de la celda (`col`, `row`), con la cuadrícula empezando en `origin`.
fn tile_rect(origin: Pos2, col: usize, row: usize) -> Rect {
    Rect::from_min_size(pos2(origin.x + GRID_PAD + col as f32 * TILE_W, origin.y + GRID_PAD + row as f32 * TILE_H), vec2(TILE_W, TILE_H)).shrink(TILE_INSET)
}

/// Una imagen con las esquinas redondeadas (miniaturas: radio S).
fn rounded_image(p: &Painter, tex: egui::TextureId, rect: Rect, uv: Rect, tint: Color32) {
    p.add(egui::epaint::RectShape::filled(rect, radius::S, tint).with_texture(tex, uv));
}

/// ¿Se está desplazando rápido? (más de 3 pantallas por segundo, o hace menos de `SCROLL_SETTLE`
/// que lo hacía). Entonces la cuadrícula solo pide lo que está en la caché en disco: las fotos
/// que pasan de largo no ocupan los hilos (una lectura en curso no se puede cancelar). También
/// avisa a `Textures` que algo se mueve (sube menos texturas por cuadro).
fn scroll_fast(ui: &Ui, id: Id, viewport: Rect) -> bool {
    let now = ui.input(|i| i.time);
    let prev: Option<(f32, f64, f64)> = ui.data(|d| d.get_temp(id));
    let (mut fast_at, moved) = match prev {
        Some((y, t, fast_at)) => {
            let dt = (now - t).max(1e-3) as f32;
            let speed = (viewport.min.y - y).abs() / dt / viewport.height().max(1.0);
            (if speed > 3.0 && (viewport.min.y - y).abs() > 1.0 { now } else { fast_at }, (viewport.min.y - y).abs() > 0.5)
        }
        None => (f64::MIN, false),
    };
    if ui.ctx().dragged_id().is_some() && moved {
        fast_at = now;
    }
    ui.data_mut(|d| d.insert_temp(id, (viewport.min.y, now, fast_at)));
    if moved {
        super::preview::moving(ui.ctx());
    }
    let left = fast_at + super::motion::SCROLL_SETTLE - now;
    if left > 0.0 {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
        return true;
    }
    false
}

/// Proporción que se supone antes de saber la real: foto 3:2, video 16:9, PDF vertical (1:√2).
fn default_aspect(ext: &str) -> f32 {
    match ext {
        "pdf" => 1.0 / std::f32::consts::SQRT_2,
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "mpg" => 16.0 / 9.0,
        _ => 1.5,
    }
}

/// El rectángulo de una miniatura en su caja: sale de la proporción (y del original, si es más
/// chico que la caja: un PNG de 32 px no se agranda), nunca de cuántos píxeles llegaron. Así una
/// versión mejor reemplaza a otra sin mover nada. Alineado a píxeles.
fn thumb_rect(caja: Rect, aspect: f32, orig: Option<(u32, u32)>, ppp: f32) -> Rect {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.5 };
    let mut s = if aspect >= caja.aspect_ratio() { vec2(caja.width(), caja.width() / aspect) } else { vec2(caja.height() * aspect, caja.height()) };
    if let Some((w, _)) = orig.filter(|o| o.0 > 0) {
        let nat = w as f32 / ppp;
        if nat < s.x {
            s *= nat / s.x;
        }
    }
    let r = Rect::from_center_size(caja.center(), s);
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(pos2(snap(r.min.x), snap(r.min.y)), pos2(snap(r.max.x), snap(r.max.y)))
}

/// La miniatura de un archivo con imagen en su caja de la cuadrícula. Mientras no llega: nada
/// los primeros `GRACE_THUMB`, después una placa quieta (con el color y la forma de la foto si ya
/// se vio en esta sesión); nunca el ícono genérico, que queda para el fallo definitivo (devuelve
/// false: lo dibuja quien llama). La primera vez que llega entra con un fundido `SLOW` sobre la
/// placa; si ya se había visto, de golpe.
#[allow(clippy::too_many_arguments)]
fn grid_thumb(ui: &Ui, p: &Painter, t: &Theme, caja: Rect, path: &str, e: &FileEntry, fast: bool, tex: &mut super::preview::Textures, ppp: f32, a: f32) -> bool {
    let ext = e.ext.as_str();
    use super::preview::Thumb;
    let px = (caja.width().max(caja.height()) * ppp).ceil() as u32 / 32 * 32 + 32;
    let ctx = ui.ctx();
    let placa = |r: Rect, color: Color32, k: f32| p.rect_filled(r, radius::S, alpha(color, a * k));
    match tex.thumb(path, px, fsx::thumbcache::stamp_of(e.mtime, e.size), fast) {
        Thumb::Failed => false,
        Thumb::Ready(te) => {
            let Some(tx) = &te.tex else { return false };
            let size = tx.size_vec2();
            // Si en vez de una miniatura llegó un ícono (un PDF que no se pudo dibujar, un video
            // sin vista: cuadrado, sin medidas ni páginas), va del tamaño de los demás.
            let is_icon = !fsx::thumbs::is_image_ext(ext) && te.orig.is_none() && te.pages.is_none() && size.x == size.y;
            let img = if is_icon { Rect::from_center_size(caja.center(), vec2(TILE_ICON, TILE_ICON)) } else { thumb_rect(caja, te.aspect().unwrap_or(1.5), te.orig, ppp) };
            let shown = super::motion::since(ctx, te.at, dur::SLOW);
            if shown < 1.0 && !is_icon {
                placa(img, t.hover, 1.0 - shown);
            }
            rounded_image(p, tx.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), alpha(Color32::WHITE, a * shown));
            true
        }
        Thumb::Waiting { print: Some(pr), .. } => {
            // Ya vista: su forma y su color medio desde el primer cuadro.
            placa(thumb_rect(caja, pr.aspect, None, ppp), pr.color, 1.0);
            true
        }
        Thumb::Waiting { since, print: None } => {
            let now = ctx.input(|i| i.time);
            let left = since + super::motion::GRACE_THUMB - now;
            if left > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
            } else {
                let k = super::motion::since(ctx, since + super::motion::GRACE_THUMB, dur::FAST);
                placa(thumb_rect(caja, default_aspect(ext), None, ppp), t.hover, k);
            }
            true
        }
    }
}

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
fn grid_folder(ui: &Ui, p: &Painter, t: &Theme, area: Rect, path: &str, tex: &mut super::preview::Textures, a: f32) {
    let center = area.center();
    p.text(center, Align2::CENTER_CENTER, ic::FOLDER, design::icon_fill(TILE_FOLDER), alpha(t.folder, a));
    // Frente de la carpeta (el glifo deja la pestaña arriba): ahí va la foto o el emblema.
    let front = Rect::from_center_size(center + vec2(0.0, 6.0), vec2(58.0, 40.0));
    let ppp = ui.ctx().pixels_per_point();
    let px = ((front.width().max(front.height()) * ppp).ceil() as u32 / 32 + 1) * 32;
    let peek = tex.get(&format!("{}{path}", fsx::thumbs::PEEK_PREFIX), px).and_then(|te| te.tex.clone().map(|tx| (tx, te.at)));
    if let Some((tx, at)) = peek {
        let shown = super::motion::since(ui.ctx(), at, dur::SLOW) * a;
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
        let card = front.expand(sp::XXS);
        p.rect_filled(card, radius::S, alpha(Color32::WHITE, 0.92 * shown));
        p.image(tx.id(), front, uv, alpha(Color32::WHITE, shown));
        return;
    }
    if let Some(g) = folder_emblem(path) {
        p.text(front.center(), Align2::CENTER_CENTER, g, design::icon_fill(TILE_EMBLEM), alpha(Color32::WHITE, 0.9 * a));
    }
}

/// Columnas de la cuadrícula de una pestaña en el último cuadro (para moverse con las flechas).
pub fn grid_columns(ctx: &egui::Context, tab_id: u64) -> usize {
    ctx.data(|d| d.get_temp::<usize>(Id::new(("grid-cols", tab_id)))).unwrap_or(1).max(1)
}

#[allow(clippy::too_many_arguments)]
pub fn grid(ui: &mut Ui, cx: &mut Cx, ci: usize, col: &mut Col, focused: bool, data: ColumnData, tex: &mut super::preview::Textures) {
    let _p = super::perf::scope("lista.cuadrícula");
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
        error_message(ui, &t, err);
        return;
    }
    if data.entries.is_empty() {
        if data.loading {
            loading_message(ui, &t, &dir);
        } else {
            empty_message(ui, &t, ic::FOLDER_OPEN, t!("lista.carpeta_vacia"));
        }
        return;
    }
    let n = data.entries.len();
    let cols = ((rest.width() - 2.0 * GRID_PAD) / TILE_W).floor().max(1.0) as usize;
    ui.ctx().data_mut(|d| d.insert_temp(Id::new(("grid-cols", cx.tab_id)), cols));
    let rows = n.div_ceil(cols);
    let scroll_to = scroll_target(col, data.entries, data.loading);
    let selection = col.selected();
    let ppp = ui.ctx().pixels_per_point();
    let base = selection.clone();
    let anchor_id = Id::new(("grid-ancla", cx.tab_id, &dir));
    let row_top = |i: usize| GRID_PAD + (i / cols) as f32 * TILE_H;
    let mut area = egui::ScrollArea::vertical().id_salt(("grid", cx.tab_id, &dir)).auto_shrink([false, false]);
    if let Some(off) = Some(data.entries).filter(|_| scroll_to.is_none()).and_then(|e| anchor_fix(ui, anchor_id, e, row_top)) {
        area = area.vertical_scroll_offset(off);
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_height(rows as f32 * TILE_H + 2.0 * GRID_PAD);
        let origin = ui.max_rect().min;
        if let Some(i) = scroll_to {
            let r = tile_rect(origin, 0, i / cols).expand(TILE_INSET);
            ui.scroll_to_rect(r, None);
        }
        // El resaltado se desliza de mosaico en mosaico (también en diagonal, al cambiar de fila).
        let pill = glide_row(&t, col, data.entries, focused).map(|(i, color)| {
            let target = tile_rect(origin, i % cols, i / cols);
            let r = pill_rect(ui, Id::new(("grid-pill", cx.tab_id, &dir)), target);
            ui.painter().rect_filled(r, radius::L, color);
            r
        });
        let first = (viewport.min.y / TILE_H).floor().max(0.0) as usize;
        let last = ((viewport.max.y / TILE_H).ceil() as usize + 1).min(rows);
        let fast = scroll_fast(ui, Id::new(("grid-vel", cx.tab_id, &dir)), viewport);
        anchor_keep(ui, anchor_id, data.entries, first * cols, row_top(first * cols), viewport.min.y);
        for row in first..last {
            for c in 0..cols {
                let i = row * cols + c;
                if i >= n {
                    break;
                }
                let e = &data.entries[i];
                let r = tile_rect(origin, c, row);
                let path = fsx::join(&dir, &e.name);
                let resp = ui.interact(r, Id::new(("tile", cx.tab_id, &path)), Sense::click_and_drag());
                let selected = col.is_selected(&e.name);
                if e.is_dir {
                    prefetch(cx, &resp, &path, selected && focused);
                }
                let p = ui.painter_at(r);
                let hover = hover_amount(&resp, !selected || pill.is_some_and(|pl| pill_cover(pl, r) < 0.5));
                design::state_fill(&p, &t, r, radius::L, hover, false, sel_of(selected && pill.is_none(), focused));
                if e.is_dir {
                    drop_highlight(ui, &t, &resp, r);
                }
                // Miniatura (o ícono grande).
                let thumb = Rect::from_min_size(r.min + vec2(sp::S, sp::S), vec2(r.width() - 2.0 * sp::S, THUMB_H));
                let a = if e.hidden || cx.cut.contains(&path) { faded(&t) } else { 1.0 };
                let mut drew = false;
                if e.is_dir {
                    grid_folder(ui, &p, &t, thumb, &path, tex, a);
                    drew = true;
                } else if wants_thumbnail(e) && e.cloud != fsx::Cloud::OnlineOnly {
                    drew = grid_thumb(ui, &p, &t, thumb, &path, e, fast, tex, ppp, a);
                }
                if !drew {
                    file_icon(&p, &t, thumb.center(), TILE_ICON, TILE_GLYPH, &path, e.is_dir, &e.ext, a);
                }
                let mask = fsx::tags::get(&path);
                if mask != 0 {
                    tag_dots(&p, &t, thumb.left() + 2.0, thumb.top() + 7.0, mask);
                }
                if let Some((g, c, _)) = cloud_glyph(&t, e.cloud) {
                    // Insignia: disco de 20 en la superficie que flota, con ícono de 12.
                    let b = pos2(thumb.right() - 6.0, thumb.top() + sp::S);
                    p.circle_filled(b, h::CHIP / 2.0, t.raised);
                    p.text(b, Align2::CENTER_CENTER, g, design::icon_font(icon::S), c);
                }
                // Nombre en hasta dos líneas.
                let renaming = cx.rename.as_ref().is_some_and(|rn| rn.dir == dir && rn.old == e.name);
                let name_rect = Rect::from_min_max(pos2(r.left() + sp::XS, thumb.bottom() + sp::XS), pos2(r.right() - sp::XS, r.bottom() - sp::XXS));
                if renaming {
                    rename_row(ui, cx, Rect::from_min_size(name_rect.min, vec2(name_rect.width(), h::HEAD)), name_rect.left(), name_rect.right());
                } else {
                    theme::want_cjk(&e.name);
                    let name_c = alpha(t.text, a);
                    let mut job = LayoutJob::single_section(e.name.clone(), TextFormat::simple(text::small(), name_c));
                    job.wrap = TextWrapping { max_width: name_rect.width(), max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
                    job.halign = egui::Align::Center;
                    let g = p.layout_job(job);
                    p.galley(pos2(name_rect.center().x, name_rect.top()), g, name_c);
                }
                handle_row_response(cx, &resp, ci, &dir, e, selected, &selection, None, name_rect, false);
            }
        }
        // Adelanto: una pantalla más abajo y media arriba, después de todo lo visible (al
        // desplazarse ya están). Solo lo barato de pedir: lo que deja de pedirse se olvida solo.
        let screen = last.saturating_sub(first).max(1);
        let thumb_w = TILE_W - 2.0 * TILE_INSET - 2.0 * sp::S;
        let px = (thumb_w.max(THUMB_H) * ppp).ceil() as u32 / 32 * 32 + 32;
        for row in (last..(last + screen).min(rows)).chain(first.saturating_sub(screen / 2)..first).filter(|_| !fast) {
            for e in data.entries[row * cols..((row + 1) * cols).min(n)].iter().filter(|e| !e.is_dir && wants_thumbnail(e) && e.cloud != fsx::Cloud::OnlineOnly) {
                tex.warm_thumb(&fsx::join(&dir, &e.name), px, fsx::thumbcache::stamp_of(e.mtime, e.size));
            }
        }
    });
    // Mosaicos que toca el rectángulo (en coordenadas del contenido: margen GRID_PAD y celdas de TILE_W × TILE_H).
    let entries = data.entries;
    marquee(ui, cx, ci, start, base, &out, |r| {
        let mut v = Vec::new();
        let rows_hit = ((r.min.y - GRID_PAD) / TILE_H).floor().max(0.0) as usize..=(((r.max.y - GRID_PAD) / TILE_H).floor().max(0.0) as usize).min(rows.saturating_sub(1));
        for row in rows_hit {
            for c in 0..cols {
                let i = row * cols + c;
                if i >= n {
                    break;
                }
                let tile = tile_rect(Pos2::ZERO, c, row);
                if tile.intersects(r) {
                    v.push(entries[i].name.clone());
                }
            }
        }
        v
    });
    background_drop(ui, cx, &bg, rest, &dir);
}
