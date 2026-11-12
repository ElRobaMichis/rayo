//! Barra lateral, pestañas, barra de herramientas y barra de estado.

use super::actions::Action;
use super::config::{Config, TabGroup, ViewMode};
use super::design::{self, Chip, DialogW, IconBtn, Kind, MenuItem, Row, Sel, Tone, Trailing, dur, h, icon, radius, sp, stroke, text};
use super::motion::SlotKey;
use super::state::Tab;
use super::theme::{self, Theme};
use super::views::elided;
use crate::fsx::{self, Drive, DriveKind, Place, transfer};
use egui::{Align2, Color32, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

/// Margen del pill de una fila de la barra lateral (ícono en x=16, texto en x=40).
const NAV_INSET: f32 = sp::S;
/// Botones mini de un encabezado de sección: a 12 del borde, 2 entre sí.
const HEAD_BTN_EDGE: f32 = sp::M;
/// Medidor de una unidad: centro a 29 px del borde de arriba de la fila.
const DRIVE_METER_Y: f32 = 29.0;

/// Botón de ícono de 28×28 (barra de herramientas, pie de la barra lateral, terminal).
pub fn icon_btn(ui: &mut Ui, t: &Theme, icon: &str, enabled: bool, on: bool, tip: &str) -> Response {
    let resp = design::icon_button(ui, t, icon, IconBtn { mini: false, on, enabled, nav: false });
    design::tip(resp, t, tip)
}

/// Botón mini de 20×20 que existe siempre (así no cambia lo que hay bajo el ratón) pero solo se
/// dibuja si `visible`.
fn mini_btn(ui: &mut Ui, t: &Theme, rect: Rect, id: Id, glyph: &str, visible: bool) -> Response {
    if visible { design::icon_button_at(ui, t, rect, id, glyph, IconBtn::mini()) } else { ui.interact(rect, id, Sense::click()) }
}

/// Atajo de un comando como tecla en línea de un tooltip: «Ctrl+Mayús+L» → «Ctrl Mayús L».
fn kbd_keys(keys: &str) -> String {
    match keys.strip_suffix("++") {
        Some(head) => format!("{} +", head.replace('+', " ")),
        None => keys.replace('+', " "),
    }
}

/// Atajo (en teclas en línea) de un comando, con los propios del usuario.
fn cmd_keys(id: &str, shortcuts: &[(String, String)]) -> String {
    kbd_keys(&super::commands::shortcut_of(id, shortcuts))
}

/// Pie de la barra lateral (botón de tema): la lista termina encima, no pasa por debajo.
pub const SIDEBAR_FOOT: f32 = h::CTL + 2.0 * sp::S;

/// Encabezado de sección: micro-etiqueta en x=16 y caret pegado (con hover o si está plegada).
/// `first`: el primero lleva menos aire arriba. `fold_id`: el del cuerpo (`fold`), para que el caret
/// gire con él.
fn section_header(ui: &mut Ui, t: &Theme, label: &str, collapsed: bool, first: bool, fold_id: Id) -> (Response, Rect) {
    let k = super::motion::fade(ui.ctx(), fold_id, !collapsed, dur::SLOW);
    let (resp, rect, _) = section_header_ex(ui, t, label, first, Some(k));
    (resp, rect)
}

/// Igual; `open`: cuánto está abierta (0 a 1), o None si no se pliega (el estante: sin caret).
/// Devuelve además el rectángulo de la etiqueta.
fn section_header_ex(ui: &mut Ui, t: &Theme, label: &str, first: bool, open: Option<f32>) -> (Response, Rect, Rect) {
    ui.add_space(if first { 0.0 } else { sp::M });
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::click());
    let p = ui.painter();
    let hovered = ui.rect_contains_pointer(rect);
    let lr = design::micro(p, pos2(rect.left() + sp::L, rect.center().y), Align2::LEFT_CENTER, label, if hovered && open.is_some() { t.dim } else { t.faint });
    if let Some(k) = open {
        // Un solo glifo que gira con el pliegue; plegada se ve siempre, desplegada con hover, y
        // durante el giro nunca desaparece de golpe.
        let alpha = if hovered { 1.0 } else { 1.0 - k };
        if alpha > 0.0 {
            let g = p.layout_no_wrap(ic::CARET_RIGHT.to_string(), design::icon_font(icon::S), t.faint);
            let pos = pos2(lr.right() + sp::XS, rect.center().y - g.size().y / 2.0);
            let shape = egui::epaint::TextShape::new(pos, g, t.faint).with_angle_and_anchor(k * std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER).with_opacity_factor(alpha);
            p.add(shape);
        }
    }
    (resp, rect, lr)
}

/// Cuerpo plegable con recorte (secciones y grupos de la barra lateral). Devuelve cuánto está
/// abierto (0 a 1). El alto va de 0 al del contenido; el contenido entra 12 px desde arriba con
/// opacidad desde 0,3 (la receta de siempre). `born`: algo recién creado crece desde alto 0.
/// Se dibuja siempre en un hijo con `id_salt` (y plegado se saltan los mismos ids), así los ids
/// automáticos de lo que sigue no cambian entre cuadros.
fn fold(ui: &mut Ui, id: Id, open: bool, born: Option<f64>, body: impl FnOnce(&mut Ui)) -> f32 {
    let ctx = ui.ctx().clone();
    let mut k = super::motion::fade(&ctx, id, open, dur::SLOW);
    if let Some(at) = born {
        k = k.min(super::motion::since(&ctx, at, dur::SLOW));
    }
    if !open && k <= 0.0 {
        // Plegado: no se dibuja (costo cero); se gastan los ids que gastaría.
        ui.skip_ahead_auto_ids(2);
        return 0.0;
    }
    let h_id = id.with("h");
    let prev_h: f32 = ctx.data(|d| d.get_temp(h_id)).unwrap_or(0.0);
    let avail = ui.available_rect_before_wrap();
    let top = avail.top();
    let ppp = ctx.pixels_per_point();
    let shift = ((1.0 - k) * prev_h.min(12.0) * ppp).round() / ppp;
    let max = Rect::from_min_max(pos2(avail.left(), top - shift), pos2(avail.right(), top - shift + 100_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(max).layout(*ui.layout()));
    if k < 1.0 {
        let clip = Rect::from_min_max(pos2(avail.left(), top), pos2(avail.right(), top + ((k * prev_h * ppp).round() / ppp))).intersect(ui.clip_rect());
        child.set_clip_rect(clip);
        child.multiply_opacity(0.3 + 0.7 * k);
    }
    body(&mut child);
    let h = child.min_rect().height();
    ctx.data_mut(|d| d.insert_temp(h_id, h));
    let used = if k >= 1.0 { h } else { (k * h * ppp).round() / ppp };
    ui.allocate_space(vec2(avail.width(), used));
    k
}

/// Pista bajo una sección («Usa + para…», «y N más»): `small` `faint`, alineada con el texto de
/// las filas (x = 40).
fn side_hint(ui: &mut Ui, t: &Theme, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    let x = rect.left() + NAV_INSET + sp::S + icon::M + sp::S;
    let g = elided(ui.painter(), text, text::small(), t.faint, rect.right() - NAV_INSET - x);
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, t.faint);
}

/// Rectángulo del botón mini `k` (0 = el de más a la derecha) de un encabezado de sección.
fn head_btn_rect(hrect: Rect, k: usize) -> Rect {
    let cx = hrect.right() - HEAD_BTN_EDGE - h::CHIP / 2.0 - k as f32 * (h::CHIP + sp::XXS);
    Rect::from_center_size(pos2(cx, hrect.center().y), vec2(h::CHIP, h::CHIP))
}

/// Ítem de la barra lateral: fila de navegación; activo = capa gris (el ícono en acento lo pone
/// quien llama). `count`: conteo a la derecha.
#[allow(clippy::too_many_arguments)]
fn side_item(ui: &mut Ui, t: &Theme, icon: &str, icon_color: Color32, fill: bool, label: &str, selected: bool, count: Option<&str>) -> Response {
    side_row(ui, t, Row { icon_color: Some(icon_color), icon_filled: fill, ..side_spec(icon, label, selected, count) })
}

/// La fila de la barra lateral sin ícono definido (punto de color, ícono de archivo).
fn side_spec<'a>(icon: &'a str, label: &'a str, selected: bool, count: Option<&'a str>) -> Row<'a> {
    Row {
        trailing: count.map_or(Trailing::None, Trailing::Count),
        sel: if selected { Sel::Unfocused } else { Sel::None },
        inset: NAV_INSET,
        height: h::NAV,
        ..Row::new(icon, label)
    }
}

fn side_row(ui: &mut Ui, t: &Theme, spec: Row) -> Response {
    let (rect, a) = ui.allocate_exact_size(vec2(ui.available_width(), h::NAV), Sense::hover());
    theme::want_cjk(spec.label);
    design::row_at(ui, t, rect, a.id, spec)
}

/// Ícono de un archivo del estante: el de Windows si ya llegó (como en la lista); si no, el glifo
/// de Rayo en contorno `dim`. La carpeta, rellena en `folder`.
fn shelf_icon(ui: &Ui, t: &Theme, center: egui::Pos2, path: &str) {
    let p = ui.painter();
    let is_dir = fsx::is_dir_like(path);
    let ext = fsx::ext_of(path);
    let (g, c) = crate::ui::icons::for_entry(t, is_dir, &ext);
    if is_dir {
        p.text(center, Align2::CENTER_CENTER, g, design::icon_fill(icon::M), c);
        return;
    }
    let px = super::shellicons::px_for(ui.ctx(), icon::M);
    // Recién llegado, el de Windows aparece mientras el glifo se desvanece.
    let mut glyph_a = 1.0;
    if let Some((tex, shown)) = super::shellicons::get(path, &ext, px) {
        let side = px as f32 / ui.ctx().pixels_per_point();
        let r = Rect::from_center_size(center, vec2(side, side));
        p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(shown));
        glyph_a = 1.0 - shown;
    }
    if glyph_a > 0.0 {
        p.text(center, Align2::CENTER_CENTER, g, design::icon_font(icon::M), t.dim.gamma_multiply(glyph_a));
    }
}

/// Resalta una fila de la barra lateral mientras se arrastran archivos encima. Devuelve lo que se
/// soltó en ella (de Rayo o de otra app), si se soltó algo.
fn sidebar_drop(ui: &Ui, t: &Theme, resp: &Response) -> Option<Vec<String>> {
    if crate::ui::views::drag_hovering(resp) {
        let r = resp.rect.shrink2(vec2(NAV_INSET, 1.0));
        ui.painter().rect_filled(r, radius::M, t.accent_soft);
        design::focus_ring(ui.painter(), t, r, radius::M);
    }
    crate::ui::views::take_drop(resp)
}

/// Soltar archivos en un lugar, una unidad o un favorito: se copian o se mueven ahí, como al
/// soltarlos en una carpeta (mismo disco = mover, otro disco = copiar; Ctrl copia, Mayús mueve).
fn drop_into(ui: &Ui, t: &Theme, resp: &Response, dest: &str, actions: &mut Vec<Action>) {
    if let Some(files) = sidebar_drop(ui, t, resp) {
        let paths: Vec<String> = files.into_iter().filter(|f| droppable_into(f, dest)).collect();
        if !paths.is_empty() {
            actions.push(Action::DropFiles { paths, dest: dest.to_string(), force: crate::ui::views::drop_force() });
        }
    }
}

/// Menú de clic derecho de un lugar, una unidad o un favorito.
fn location_menu(ui: &mut Ui, t: &Theme, path: &str, split: bool, groups: &[String], actions: &mut Vec<Action>) {
    if design::menu_item(ui, t, ic::FOLDER_OPEN, t!("comun.abrir"), "").clicked() {
        actions.push(Action::Navigate(path.to_string()));
    }
    if design::menu_item(ui, t, ic::TABS, t!("barra.abrir_pestana_nueva"), "").clicked() {
        actions.push(Action::NewTab(Some(path.to_string())));
    }
    if split && design::menu_item(ui, t, ic::SQUARE_SPLIT_HORIZONTAL, t!("barra.abrir_otro_panel"), "").clicked() {
        actions.push(Action::OpenOther(path.to_string()));
    }
    if !groups.is_empty() {
        let r = design::menu_item_ex(ui, t, MenuItem { icon: ic::STAR, label: t!("comun.agregar_favoritos"), submenu: true, ..Default::default() });
        egui::containers::menu::SubMenu::new().show(ui, &r, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_max_width(design::MENU_W);
            for (gi, g) in groups.iter().enumerate() {
                if design::menu_item(ui, t, ic::FOLDER, g, "").clicked() {
                    actions.push(Action::AddFavorite(gi, path.to_string()));
                }
            }
        });
    }
    design::menu_sep(ui, t);
    if design::menu_item(ui, t, ic::TERMINAL, t!("barra.abrir_terminal_aqui"), "").clicked() {
        actions.push(Action::Terminal(path.to_string()));
    }
    if design::menu_item(ui, t, ic::FOLDER_OPEN, t!("barra.mostrar_explorador"), "").clicked() {
        actions.push(Action::ExplorerAt(path.to_string()));
    }
    if design::menu_item(ui, t, ic::INFO, t!("barra.propiedades"), "").clicked() {
        actions.push(Action::Properties(path.to_string()));
    }
}

/// Unidad: línea 1 como un ítem, con el espacio libre a la derecha (con hover, el botón de
/// expulsar); línea 2, el medidor de puntos. El detalle va al tooltip.
fn drive_row(ui: &mut Ui, t: &Theme, d: &Drive, selected: bool, split: bool, groups: &[String], actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h::DRIVE), Sense::click());
    let resp = if d.total > 0 { design::tip(resp, t, &tf!("barra.libres", libre = fsx::human_size(d.free), total = fsx::human_size(d.total))) } else { resp };
    drop_into(ui, t, &resp, &d.root(), actions);
    let p = ui.painter().clone();
    let pill = rect.shrink2(vec2(NAV_INSET, 1.0));
    // Por posición (no `hovered`): el botón de expulsar queda encima de la fila.
    let hovering = ui.rect_contains_pointer(rect);
    let hov = design::hover_amount(ui.ctx(), resp.id, hovering);
    design::state_fill(&p, t, pill, radius::M, hov, resp.is_pointer_button_down_on(), if selected { Sel::Unfocused } else { Sel::None });
    let icon = match d.kind {
        DriveKind::Removable => ic::USB,
        DriveKind::Network => ic::GLOBE,
        DriveKind::Optical => ic::DISC,
        _ if d.ejectable => ic::USB,
        _ => ic::HARD_DRIVE,
    };
    let y1 = rect.top() + h::NAV / 2.0;
    let x_icon = pill.left() + sp::S;
    let x_text = x_icon + icon::M + sp::S;
    let right = pill.right() - sp::S;
    p.text(pos2(x_icon + icon::M / 2.0, y1), Align2::CENTER_CENTER, icon, design::icon_font(icon::M), if selected { t.accent } else { t.dim });
    let title = if d.label.is_empty() {
        match d.kind {
            DriveKind::Fixed => tf!("barra.disco_local", letra = d.letter),
            DriveKind::Removable => tf!("barra.usb", letra = d.letter),
            DriveKind::Network => tf!("barra.red", letra = d.letter),
            _ => format!("({}:)", d.letter),
        }
    } else {
        format!("{} ({}:)", d.label, d.letter)
    };
    // A la derecha: el espacio libre, o con hover el botón de expulsar.
    let mut used = 0.0;
    if d.ejectable && hovering {
        let er = head_btn_rect(Rect::from_x_y_ranges(rect.x_range(), rect.top()..=rect.top() + h::NAV), 0);
        let eresp = design::icon_button_at(ui, t, er, Id::new(("eject", d.letter)), ic::EJECT, IconBtn::mini());
        let eresp = design::tip(eresp, t, t!("barra.expulsar_forma_segura"));
        if eresp.clicked() {
            actions.push(Action::Eject(d.letter));
        }
        used = right - er.left();
    } else if d.total > 0 {
        used = p.text(pos2(right, y1), Align2::RIGHT_CENTER, fsx::human_size(d.free), text::mono_s(), t.dim).width();
    }
    let g = elided(&p, &title, text::body(), t.text, right - x_text - if used > 0.0 { used + sp::S } else { 0.0 });
    p.galley(pos2(x_text, y1 - g.size().y / 2.0), g, t.text);
    if d.total > 0 {
        let frac = d.used_frac().clamp(0.0, 1.0);
        let meter = Rect::from_min_max(pos2(x_text, rect.top() + DRIVE_METER_Y - 1.5), pos2(right, rect.top() + DRIVE_METER_Y + 1.5));
        design::dot_meter(&p, t, meter, frac, if frac > 0.9 { t.danger } else { t.accent });
    }
    if resp.clicked() {
        actions.push(Action::Navigate(d.root()));
    }
    if resp.middle_clicked() {
        actions.push(Action::NewTab(Some(d.root())));
    }
    design::context_menu(&resp, t, |ui| {
        location_menu(ui, t, &d.root(), split, groups, actions);
        if d.ejectable {
            design::menu_sep(ui, t);
            if design::menu_item(ui, t, ic::EJECT, t!("barra.expulsar"), "").clicked() {
                actions.push(Action::Eject(d.letter));
            }
        }
    });
}

pub struct SidebarIn<'a> {
    pub t: &'a Theme,
    pub cfg: &'a mut Config,
    pub places: &'a [Place],
    pub drives: &'a [Drive],
    /// Carpeta de la pestaña actual (vacía si la pestaña no es una carpeta).
    pub current: &'a str,
    /// Lugar especial que muestra la pestaña actual: "recientes", "papelera" o "".
    pub special: &'a str,
    pub group_edit: &'a mut Option<(usize, String)>,
    /// Grupo de favoritos recién creado (o recién nombrado): cuál y cuándo, para que crezca.
    pub group_born: &'a mut Option<(usize, f64)>,
    /// Colección que muestra la pestaña actual ("" si no es una).
    pub collection: &'a str,
    /// Lo que está en el estante.
    pub shelf: &'a [String],
    /// Carpetas de la nube de este equipo (nombre, carpeta).
    pub clouds: &'a [(String, String)],
}

/// Clave del pliegue de una sección fija de la barra lateral.
fn side_fold(name: &'static str) -> Id {
    Id::new(("side-fold", name))
}

/// Favorito recién agregado (grupo, ruta en minúsculas, cuándo): su fila se lava con
/// `accent_soft` que se apaga.
pub fn fav_added_key() -> Id {
    Id::new("fav-added")
}

pub fn sidebar(ui: &mut Ui, s: SidebarIn, actions: &mut Vec<Action>) {
    let t = s.t;
    // Hairline a la derecha: el borde se arrastra para cambiar el ancho.
    let full = ui.max_rect();
    ui.painter().vline(full.right() - stroke::HAIR / 2.0, full.y_range(), Stroke::new(stroke::HAIR, t.border));
    // «LUGARES» queda centrado con la barra de herramientas (alto 44) que está al lado.
    ui.add_space((h::TOOLBAR - h::HEAD) / 2.0);
    // Abajo queda el pie del botón de tema (lo pinta `app`): la lista se desplaza por encima.
    let list_h = (ui.available_height() - SIDEBAR_FOOT).max(h::NAV);
    egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink([false, false]).max_height(list_h).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let cur_lower = s.current.to_lowercase();

        let fid = side_fold("lugares");
        let (hd, _) = section_header(ui, t, t!("barra.lugares"), s.cfg.places_collapsed, true, fid);
        if hd.clicked() {
            s.cfg.places_collapsed ^= true;
        }
        let groups: Vec<String> = s.cfg.groups.iter().map(|g| g.name.clone()).collect();
        let split = s.cfg.split;
        fold(ui, fid, !s.cfg.places_collapsed, None, |ui| {
            for p in s.places {
                let sel = p.path.to_lowercase() == cur_lower;
                let r = side_item(ui, t, p.icon, if sel { t.accent } else { t.dim }, false, p.name, sel, None);
                drop_into(ui, t, &r, &p.path, actions);
                if r.clicked() {
                    actions.push(Action::Navigate(p.path.clone()));
                }
                if r.middle_clicked() {
                    actions.push(Action::NewTab(Some(p.path.clone())));
                }
                design::context_menu(&r, t, |ui| location_menu(ui, t, &p.path, split, &groups, actions));
            }
            // Lugares especiales: lo reciente de todo el equipo y la papelera.
            let sel = s.special == "recientes";
            let r = side_item(ui, t, ic::CLOCK_COUNTER_CLOCKWISE, if sel { t.accent } else { t.dim }, false, t!("barra.recientes"), sel, None);
            if r.clicked() || r.middle_clicked() {
                actions.push(Action::OpenRecent);
            }
            let sel = s.special == "papelera";
            let r = side_item(ui, t, ic::TRASH_SIMPLE, if sel { t.accent } else { t.dim }, false, t!("comun.papelera"), sel, None);
            if let Some(files) = sidebar_drop(ui, t, &r) {
                // Soltar en la papelera: mandar ahí (como la tecla Supr).
                actions.push(Action::Delete(files, false));
            }
            if r.clicked() || r.middle_clicked() {
                actions.push(Action::OpenRecycle);
            }
        });

        let fid = side_fold("unidades");
        let (hd, _) = section_header(ui, t, t!("barra.unidades"), s.cfg.drives_collapsed, false, fid);
        if hd.clicked() {
            s.cfg.drives_collapsed ^= true;
        }
        fold(ui, fid, !s.cfg.drives_collapsed, None, |ui| {
            for d in s.drives {
                let sel = cur_lower == d.root().to_lowercase();
                drive_row(ui, t, d, sel, split, &groups, actions);
            }
        });

        // Etiquetas de color (solo las que se usan): todo lo de un color, esté donde esté.
        let counts = fsx::tags::counts();
        if counts.iter().any(|&n| n > 0) {
            let fid = side_fold("etiquetas");
            let (hd, _) = section_header(ui, t, t!("barra.etiquetas"), s.cfg.tags_collapsed, false, fid);
            if hd.clicked() {
                s.cfg.tags_collapsed ^= true;
            }
            fold(ui, fid, !s.cfg.tags_collapsed, None, |ui| {
                for (i, &n) in counts.iter().enumerate().filter(|(_, n)| **n > 0) {
                    let sel = s.special == fsx::tags::COLORS[i].0;
                    let count = fsx::thousands(n);
                    let spec = Row { dot: Some(t.tags[i.min(6)]), ..side_spec("", crate::ui::views::tag_label(i), sel, Some(&count)) };
                    let r = design::tip(side_row(ui, t, spec), t, &tp!(n, "comun.elementos_n", n = fsx::thousands(n)));
                    if r.clicked() || r.middle_clicked() {
                        actions.push(Action::OpenTag(i));
                    }
                }
            });
        }

        // Nube: OneDrive, Dropbox, iCloud, Google Drive… (lo que haya en este equipo).
        if !s.clouds.is_empty() {
            let fid = side_fold("nube");
            let (hd, _) = section_header(ui, t, t!("barra.nube"), s.cfg.cloud_collapsed, false, fid);
            if hd.clicked() {
                s.cfg.cloud_collapsed ^= true;
            }
            fold(ui, fid, !s.cfg.cloud_collapsed, None, |ui| {
                for (name, path) in s.clouds {
                    let sel = path.to_lowercase() == cur_lower;
                    let r = design::tip(side_item(ui, t, ic::CLOUD, if sel { t.accent } else { t.dim }, false, name, sel, None), t, path);
                    drop_into(ui, t, &r, path, actions);
                    if r.clicked() {
                        actions.push(Action::Navigate(path.clone()));
                    }
                    if r.middle_clicked() {
                        actions.push(Action::NewTab(Some(path.clone())));
                    }
                }
            });
        }

        // Conexiones: SMB, WebDAV y SFTP guardados; "+" agrega una.
        {
            let fid = side_fold("conexiones");
            let (hd, hrect) = section_header(ui, t, t!("barra.conexiones"), s.cfg.connections_collapsed, false, fid);
            if hd.clicked() {
                s.cfg.connections_collapsed ^= true;
            }
            let visible = ui.rect_contains_pointer(hrect) || s.cfg.connections.is_empty();
            let add = design::tip(mini_btn(ui, t, head_btn_rect(hrect, 0), Id::new("conn-add"), ic::PLUS, visible), t, t!("barra.nueva_conexion"));
            if add.clicked() {
                actions.push(Action::EditConnection(None));
            }
            let connections = &s.cfg.connections;
            fold(ui, fid, !s.cfg.connections_collapsed, None, |ui| {
                for (i, c) in connections.iter().enumerate() {
                    use fsx::remote::Kind;
                    let icon = match c.kind {
                        Kind::Smb => ic::HARD_DRIVES,
                        Kind::WebDav => ic::GLOBE,
                        Kind::Sftp => ic::LOCK_KEY,
                    };
                    let label = if c.name.trim().is_empty() { c.host.clone() } else { c.name.clone() };
                    // Con "\\nas\fotos" y "\\nas\fotos2", estar en la segunda no marca la primera.
                    let root = c.root().to_lowercase();
                    let sel = cur_lower.strip_prefix(&root).is_some_and(|r| r.is_empty() || r.starts_with('\\') || root.ends_with('\\'));
                    let r = design::tip(side_item(ui, t, icon, if sel { t.accent } else { t.dim }, false, &label, sel, None), t, &c.unc());
                    if r.clicked() {
                        actions.push(Action::Connect(i));
                    }
                    design::context_menu(&r, t, |ui| {
                        if design::menu_item(ui, t, ic::PENCIL_SIMPLE, t!("barra.editar_conexion"), "").clicked() {
                            actions.push(Action::EditConnection(Some(i)));
                        }
                        if design::menu_item(ui, t, ic::KEY, t!("barra.olvidar_contrasena"), "").clicked() {
                            actions.push(Action::ForgetPassword(i));
                        }
                        if design::menu_item_ex(ui, t, MenuItem { icon: ic::TRASH, label: t!("barra.borrar_conexion"), danger: true, ..Default::default() }).clicked() {
                            actions.push(Action::DeleteConnection(i));
                        }
                    });
                }
            });
        }

        // Colecciones: carpetas virtuales (soltar archivos en una los agrega).
        let colls = fsx::collections::list();
        if !colls.is_empty() {
            let fid = side_fold("colecciones");
            let (hd, _) = section_header(ui, t, t!("barra.colecciones"), s.cfg.collections_collapsed, false, fid);
            if hd.clicked() {
                s.cfg.collections_collapsed ^= true;
            }
            fold(ui, fid, !s.cfg.collections_collapsed, None, |ui| {
                for (name, n) in &colls {
                    let count = fsx::thousands(*n);
                    let sel = s.collection == name;
                    let r = side_item(ui, t, ic::BOOKMARKS_SIMPLE, if sel { t.accent } else { t.dim }, false, name, sel, Some(&count));
                    let r = design::tip(r, t, &tp!(*n, "comun.elementos_n", n = fsx::thousands(*n)));
                    if let Some(files) = sidebar_drop(ui, t, &r) {
                        actions.push(Action::AddToCollection(name.clone(), files));
                    }
                    if r.clicked() || r.middle_clicked() {
                        actions.push(Action::OpenCollection(name.clone()));
                    }
                    design::context_menu(&r, t, |ui| {
                        if design::menu_item_ex(ui, t, MenuItem { icon: ic::TRASH, label: t!("barra.borrar_coleccion"), danger: true, ..Default::default() }).clicked() {
                            actions.push(Action::DeleteCollection(name.clone()));
                        }
                    });
                }
            });
        }

        // Estante: un lugar donde juntar archivos de varias carpetas para llevarlos juntos a otro
        // lado. Se ve si tiene algo, o mientras se arrastran archivos (para soltarlos ahí).
        let dragging = egui::DragAndDrop::has_payload_of_type::<crate::ui::views::DragFiles>(ui.ctx()) || ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if !s.shelf.is_empty() || dragging {
            let (_, hrect, lr) = section_header_ex(ui, t, t!("barra.estante"), false, None);
            if !s.shelf.is_empty() {
                // El conteo, junto a la etiqueta.
                ui.painter().text(pos2(lr.right() + sp::XS, hrect.center().y), Align2::LEFT_CENTER, fsx::thousands(s.shelf.len()), text::mono_s(), t.faint);
            }
            let hd = ui.interact(hrect, Id::new("shelf-drag"), Sense::click_and_drag());
            if let Some(files) = sidebar_drop(ui, t, &hd) {
                actions.push(Action::ShelfAdd(files));
            }
            // Arrastrar el título lleva todo lo del estante.
            if hd.drag_started() && !s.shelf.is_empty() {
                hd.dnd_set_drag_payload(crate::ui::views::DragFiles(s.shelf.to_vec()));
            }
            if !s.shelf.is_empty() {
                // Acciones en el encabezado, como las demás secciones: copiar aquí, mover aquí, vaciar.
                let here = !s.current.is_empty();
                let btns = [
                    (2, ic::COPY, here, t!("barra.estante_copiar_aqui"), Action::ShelfToHere(false)),
                    (1, ic::ARROW_BEND_DOWN_RIGHT, here, t!("barra.estante_mover_aqui"), Action::ShelfToHere(true)),
                    (0, ic::X, true, t!("barra.vaciar_estante"), Action::ShelfClear),
                ];
                for (k, glyph, enabled, tip, action) in btns {
                    let r = design::icon_button_at(ui, t, head_btn_rect(hrect, k), Id::new(("shelf-btn", k)), glyph, IconBtn { enabled, ..IconBtn::mini() });
                    if design::tip(r, t, tip).clicked() {
                        actions.push(action);
                    }
                }
            }
            if s.shelf.is_empty() {
                // Vacío: un recuadro con trama de puntos para soltar.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), sp::XXL), Sense::hover());
                let well = r.shrink2(vec2(NAV_INSET, sp::XS));
                design::dot_grid(ui.painter(), t, well.shrink(sp::XS));
                ui.painter().rect_stroke(well, radius::M, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
                design::micro(ui.painter(), well.center(), Align2::CENTER_CENTER, t!("barra.suelta_estante"), t.faint);
                if let Some(files) = sidebar_drop(ui, t, &resp) {
                    actions.push(Action::ShelfAdd(files));
                }
            } else {
                for p in s.shelf.iter().take(8) {
                    // El ícono se pinta aparte (el de Windows, como en la lista); " " deja su lugar.
                    let r = side_row(ui, t, side_spec(" ", &fsx::file_name(p), false, None));
                    shelf_icon(ui, t, pos2(r.rect.left() + NAV_INSET + sp::S + icon::M / 2.0, r.rect.center().y), p);
                    let r = design::tip(r, t, p);
                    if r.drag_started() {
                        r.dnd_set_drag_payload(crate::ui::views::DragFiles(vec![p.clone()]));
                    }
                    design::context_menu(&r, t, |ui| {
                        if design::menu_item(ui, t, ic::X, t!("barra.quitar_estante"), "").clicked() {
                            actions.push(Action::ShelfRemove(p.clone()));
                        }
                    });
                }
                if s.shelf.len() > 8 {
                    side_hint(ui, t, &tf!("barra.y_mas", n = s.shelf.len() - 8));
                }
            }
        }

        let now = ui.input(|i| i.time);
        let added: Option<(usize, String, f64)> = ui.data(|d| d.get_temp(fav_added_key()));
        let ngroups = s.cfg.groups.len();
        for gi in 0..ngroups {
            let born = s.group_born.filter(|(i, _)| *i == gi).map(|(_, at)| at);
            let editing = s.group_edit.as_ref().is_some_and(|(i, _)| *i == gi);
            if editing {
                // Grupo nuevo: crece desde alto 0 con el campo ya enfocado y todo el texto
                // seleccionado (escribir no espera a la animación); empuja lo de abajo.
                let group_edit = &mut *s.group_edit;
                let groups_cfg = &mut s.cfg.groups;
                let mut committed = false;
                fold(ui, Id::new(("fav-new", gi)), true, born, |ui| {
                    ui.add_space(sp::M);
                    let (_, buf) = group_edit.as_mut().unwrap();
                    let mut f = super::widgets::Field::new(("grupo-nombre", gi), egui_phosphor::regular::TAG, t!("barra.nombre_grupo"));
                    f.width = ui.available_width() - sp::M;
                    let r = super::widgets::field(ui, t, buf, f).edit;
                    if !r.has_focus()
                        && let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), r.id)
                    {
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(buf.chars().count()))));
                        st.store(ui.ctx(), r.id);
                    }
                    r.request_focus();
                    if born.is_some_and(|at| now - at < 0.5) {
                        // Mientras crece, que no quede bajo el pie.
                        ui.scroll_to_rect(r.rect, None);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) || r.lost_focus() {
                        let name = buf.trim().to_string();
                        if !name.is_empty() {
                            groups_cfg[gi].name = name;
                        }
                        *group_edit = None;
                        committed = true;
                    }
                });
                if committed {
                    // Ya con nombre: el encabezado reemplaza al campo y la pista entra con el pliegue.
                    *s.group_born = Some((gi, now));
                }
                continue;
            }
            let collapsed = s.cfg.groups[gi].collapsed;
            let name = s.cfg.groups[gi].name.clone();
            // El nombre va en la clave: al borrar un grupo los índices se corren y el siguiente no
            // debe heredar una animación ajena.
            let fid = Id::new(("fav-fold", gi, &name));
            let (hd, hrect) = section_header(ui, t, &name, collapsed, false, fid);
            if let Some(files) = sidebar_drop(ui, t, &hd) {
                // Soltar carpetas en el título de un grupo: agregarlas a sus favoritos.
                for f in files.into_iter().filter(|f| fsx::is_dir_like(f)) {
                    actions.push(Action::AddFavorite(gi, f));
                }
            }
            if hd.clicked() {
                s.cfg.groups[gi].collapsed ^= true;
            }
            // Botones del grupo: agregar carpeta actual y menú (con hover, o mientras el menú está abierto).
            let more_id = Id::new(("gmore", gi));
            let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
            let visible = ui.rect_contains_pointer(hrect) || menu_open;
            let add = design::tip(mini_btn(ui, t, head_btn_rect(hrect, 1), Id::new(("gadd", gi)), ic::PLUS, visible), t, t!("barra.agregar_carpeta_actual"));
            let more = mini_btn(ui, t, head_btn_rect(hrect, 0), more_id, ic::DOTS_THREE, visible);
            if add.clicked() && !s.current.is_empty() {
                actions.push(Action::AddFavorite(gi, s.current.to_string()));
            }
            design::popup_menu(&more, t, |ui| {
                if design::menu_item(ui, t, ic::PENCIL_SIMPLE, t!("barra.renombrar_grupo"), "").clicked() {
                    actions.push(Action::RenameGroup(gi));
                }
                if design::menu_item_ex(ui, t, MenuItem { icon: ic::TRASH, label: t!("barra.eliminar_grupo"), danger: true, ..Default::default() }).clicked() {
                    actions.push(Action::DeleteGroup(gi));
                }
            });
            let group = &s.cfg.groups[gi];
            fold(ui, fid, !group.collapsed, born, |ui| {
                if group.items.is_empty() {
                    side_hint(ui, t, t!("barra.usa_agregar_carpeta_actual"));
                }
                for (ii, path) in group.items.iter().enumerate() {
                    let sel = path.to_lowercase() == cur_lower;
                    let label = if path.len() <= 3 { path.clone() } else { fsx::file_name(path) };
                    let r = design::tip(side_item(ui, t, ic::FOLDER, if sel { t.accent } else { t.folder }, true, &label, sel, None), t, path);
                    // Recién agregado (p. ej. soltando una carpeta en el título): un lavado que se apaga.
                    if let Some((_, _, at)) = added.as_ref().filter(|(g, p, _)| *g == gi && *p == path.to_lowercase()) {
                        let a = 1.0 - super::motion::since(ui.ctx(), *at, dur::SLOW);
                        if a > 0.0 {
                            ui.painter().rect_filled(r.rect.shrink2(vec2(NAV_INSET, 1.0)), radius::M, t.accent_soft.gamma_multiply(a));
                        }
                    }
                    drop_into(ui, t, &r, path, actions);
                    if r.clicked() {
                        actions.push(Action::Navigate(path.clone()));
                    }
                    if r.middle_clicked() {
                        actions.push(Action::NewTab(Some(path.clone())));
                    }
                    design::context_menu(&r, t, |ui| {
                        location_menu(ui, t, path, split, &[], actions);
                        design::menu_sep(ui, t);
                        if design::menu_item(ui, t, ic::X, t!("barra.quitar_favoritos"), "").clicked() {
                            actions.push(Action::RemoveFavorite(gi, ii));
                        }
                    });
                }
            });
        }
        ui.add_space(sp::M);
        // Una fila más de la barra lateral (ícono y texto en la misma rejilla), discreta.
        let spec = Row { icon_color: Some(t.faint), color: Some(t.dim), inset: NAV_INSET, height: h::NAV, ..Row::new(ic::PLUS, t!("comun.nuevo_grupo")) };
        if design::row(ui, t, Id::new("sidebar-new-group"), spec).clicked() {
            actions.push(Action::NewGroup);
        }
        ui.add_space(sp::M);
    });
}

// ---------------------------------------------------------------- pestañas

/// Alto de la fila de arriba (pestañas y botones de la ventana): es la barra de título.
pub const TITLE_H: f32 = h::TITLE;
/// La pestaña ocupa desde 6 px bajo el borde de arriba hasta el fondo de la tira.
const TAB_TOP: f32 = 6.0;
/// La X se dibuja en diagonal (ocupa menos): un poco más grande para verse igual que el "+".
const TAB_X_ICON: f32 = icon::TAB_X;
const TAB_PLUS_ICON: f32 = icon::S;
/// A este tamaño la tipografía dibuja el "+" un poco más arriba que la X.
const TAB_PLUS_NUDGE: f32 = 1.25;
/// Centro de la X: a 15 px del borde derecho de la pestaña.
const TAB_X_INSET: f32 = 15.0;
/// Ancho de una pestaña: de 34 (solo el ícono) a 220; la activa, al menos 120 si se puede.
const TAB_MIN_W: f32 = 34.0;
const TAB_MAX_W: f32 = 220.0;
const TAB_ACTIVE_MIN_W: f32 = 120.0;
/// Ancho del "+" (con su aire).
const TAB_PLUS_W: f32 = 36.0;
/// Ancho máximo de la etiqueta de un grupo.
const GROUP_CHIP_MAX: f32 = 160.0;
/// Ancho de los tres botones de la ventana.
pub const WINDOW_BUTTONS_W: f32 = 3.0 * 46.0;
/// Íconos de los botones de la ventana: maximizar se ve más grande, va uno más chico.
const CAPTION_ICON: f32 = icon::CAPTION;
const CAPTION_ICON_MAX: f32 = icon::S;
/// Destello de la etiqueta de un grupo al crearlo, plegarlo, desplegarlo o recibir una pestaña.
const GROUP_PULSE: f32 = 0.28;

/// Lo que va en la barra de pestañas, en orden: etiquetas de grupo y pestañas (índices en `tabs`),
/// y mientras se arrastra una pestaña, su hueco.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BarItem {
    Group(usize),
    Tab(usize),
    /// El lugar vacío de la pestaña arrastrada (índice en `tabs`).
    Hole(usize),
}

/// Orden de la barra: las pestañas de un grupo van juntas, detrás de su etiqueta (en el lugar de la
/// primera); las de un grupo plegado no se ven, salvo la activa. `skip`: la pestaña arrastrada.
fn bar_items(tabs: &[Tab], groups: &[TabGroup], show: &[usize], cur: usize, skip: Option<u64>) -> Vec<BarItem> {
    let mut out = Vec::new();
    let mut done: Vec<u64> = Vec::new();
    let show: Vec<usize> = show.iter().copied().filter(|&i| Some(tabs[i].id) != skip).collect();
    for &i in &show {
        let g = tabs[i].group;
        let Some(gi) = groups.iter().position(|x| x.id == g).filter(|_| g != 0) else {
            out.push(BarItem::Tab(i));
            continue;
        };
        if done.contains(&g) {
            continue;
        }
        done.push(g);
        out.push(BarItem::Group(gi));
        for &j in show.iter().filter(|&&j| tabs[j].group == g) {
            if !groups[gi].collapsed || j == cur {
                out.push(BarItem::Tab(j));
            }
        }
    }
    out
}

/// Orden visible de las pestañas (el de la barra, sin mirar si un grupo está plegado): por panel,
/// y las de un grupo juntas en el lugar de la primera. Recibe (panel, grupo) de cada pestaña (0 =
/// sin grupo, o un grupo que ya no existe) y devuelve la permutación.
pub fn visible_order(tabs: &[(u8, u64)]) -> Vec<usize> {
    let mut out = Vec::with_capacity(tabs.len());
    for pane in 0..=tabs.iter().map(|t| t.0).max().unwrap_or(0) {
        let mut done: Vec<u64> = Vec::new();
        for (i, &(p, g)) in tabs.iter().enumerate() {
            if p != pane || (g != 0 && done.contains(&g)) {
                continue;
            }
            if g == 0 {
                out.push(i);
            } else {
                done.push(g);
                out.extend(tabs.iter().enumerate().filter(|(_, t)| t.0 == pane && t.1 == g).map(|(j, _)| j));
            }
        }
    }
    out
}

/// Mover la pestaña `cur` un lugar a la izquierda (`dir` < 0) o a la derecha, con las mismas paradas
/// que el arrastre. `order`: (id, grupo) de las pestañas del panel en orden visible (normalizado);
/// `collapsed`: los grupos plegados. Devuelve (antes de qué pestaña queda, grupo), o None si no se
/// mueve (en el extremo).
pub fn tab_step(order: &[(u64, u64)], collapsed: &[u64], cur: u64, dir: i32) -> Option<(Option<u64>, u64)> {
    let k = order.iter().position(|t| t.0 == cur)?;
    let g = order[k].1;
    let id_at = |i: usize| order.get(i).map(|t| t.0);
    // Primera y última posición del grupo `g` (sus pestañas van juntas).
    let span = |g: u64, around: usize| {
        let mut s = around;
        while s > 0 && order[s - 1].1 == g {
            s -= 1;
        }
        let mut e = around;
        while e + 1 < order.len() && order[e + 1].1 == g {
            e += 1;
        }
        (s, e)
    };
    let folded = |g: u64| collapsed.contains(&g);
    if dir > 0 {
        if g != 0 {
            if !folded(g) && order.get(k + 1).is_some_and(|t| t.1 == g) {
                return Some((id_at(k + 2), g));
            }
            // Última del grupo (o grupo plegado): sale y queda justo después.
            let (_, e) = span(g, k);
            return Some((id_at(e + 1), 0));
        }
        let next = order.get(k + 1)?;
        if next.1 == 0 {
            Some((id_at(k + 2), 0))
        } else if !folded(next.1) {
            // Entra como primera.
            Some((Some(next.0), next.1))
        } else {
            // Un grupo plegado se salta entero.
            let (_, e) = span(next.1, k + 1);
            Some((id_at(e + 1), 0))
        }
    } else {
        if g != 0 {
            if !folded(g) && k > 0 && order[k - 1].1 == g {
                return Some((Some(order[k - 1].0), g));
            }
            // Primera del grupo (o grupo plegado): sale por delante de la etiqueta.
            let (s, _) = span(g, k);
            return Some((if s == k { id_at(k + 1) } else { id_at(s) }, 0));
        }
        if k == 0 {
            return None;
        }
        let prev = order[k - 1];
        if prev.1 == 0 {
            Some((Some(prev.0), 0))
        } else if !folded(prev.1) {
            // Entra como última.
            Some((id_at(k + 1), prev.1))
        } else {
            let (s, _) = span(prev.1, k - 1);
            Some((id_at(s), 0))
        }
    }
}

/// Qué es cada lugar de la fila al decidir dónde cae una pestaña arrastrada.
#[derive(Clone, Copy, PartialEq, Debug)]
enum DropKind {
    Chip { group: u64, collapsed: bool },
    Tab { group: u64 },
}

/// Dónde cae la pestaña levantada: cuántos lugares (chips y pestañas) tienen su centro a la
/// izquierda de su borde izquierdo `l`. Se calcula sobre la fila de destino sin la arrastrada,
/// nunca sobre lo pintado (que se mueve y haría oscilar el hueco).
fn drop_index(centers: &[f32], l: f32) -> usize {
    centers.iter().filter(|&&c| c < l).count()
}

/// Lugar y grupo de lo que se soltaría. `items`: la fila de destino sin la arrastrada (qué es,
/// borde izquierdo, borde derecho); `l`: borde izquierdo de la levantada; `px`: el puntero.
/// Devuelve (índice, grupo, chip plegado sobre el que está el puntero). En una frase: se une a
/// un grupo mientras su borde izquierdo esté dentro del tramo del grupo.
fn drop_target(items: &[(DropKind, f32, f32)], l: f32, px: f32) -> (usize, u64, Option<u64>) {
    // Sobre la etiqueta de un grupo plegado: entra al final de ese grupo.
    for (i, &(k, a, b)) in items.iter().enumerate() {
        if let DropKind::Chip { group, collapsed: true } = k
            && px >= a
            && px < b
        {
            return (i + 1, group, Some(group));
        }
    }
    let centers: Vec<f32> = items.iter().map(|&(_, a, b)| (a + b) / 2.0).collect();
    let index = drop_index(&centers, l);
    let before = index.checked_sub(1).and_then(|i| items.get(i)).copied();
    let after = items.get(index).copied();
    let group = match (before, after) {
        (_, Some((DropKind::Tab { group }, ..))) if group != 0 => group,
        (Some((DropKind::Tab { group }, _, right)), _) if group != 0 => {
            if l < right {
                group
            } else {
                0
            }
        }
        (Some((DropKind::Chip { group, collapsed: false }, ..)), _) => group,
        _ => 0,
    };
    (index, group, None)
}

/// Una pestaña que se está arrastrando para reordenarla (estado propio en `ctx.data`, no un
/// `egui::DragAndDrop`: eso encendería las zonas para soltar archivos).
#[derive(Clone, Debug)]
struct TabDrag {
    id: u64,
    /// Dónde se la agarró: fracción de su ancho.
    grab: f32,
    from_pane: usize,
    /// El panel cuya tira tiene el hueco.
    host: usize,
    /// Cuándo empezó (la sombra y el relleno aparecen en `FAST`).
    start: f64,
    /// Lugar de la levantada en el último cuadro (x, ancho del lugar).
    lift: [f32; 2],
    /// Ya se soltó en este cuadro: las otras tiras todavía no la muestran.
    dropped: bool,
}

fn drag_key() -> Id {
    Id::new("tab-drag")
}

fn strip_id(pane: usize) -> Id {
    Id::new(("tabstrip", pane))
}

/// ¿Se está arrastrando una pestaña?
pub fn tab_drag_active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).is_some_and(|d| !d.dropped)
}

/// Cancelar el arrastre (Esc, clic derecho, la ventana pierde el foco): nada cambió en la app, la
/// pestaña vuelve a su lugar desde donde estaba.
pub fn tab_drag_cancel(ctx: &egui::Context) {
    let d = ctx.data_mut(|m| {
        let d = m.get_temp::<TabDrag>(drag_key());
        m.remove::<TabDrag>(drag_key());
        d
    });
    if let Some(d) = d.filter(|d| !d.dropped) {
        super::motion::strip_seed(ctx, strip_id(d.from_pane), SlotKey::Tab(d.id), d.lift[0], d.lift[1]);
    }
}

/// Una vez por cuadro, antes de dibujar los paneles: qué tira tiene el hueco (la que tiene el
/// puntero, con 12 px de margen arriba y abajo; fuera de todas, la última). `strips`: la franja de
/// pestañas de cada panel visible.
pub fn tab_drag_host(ctx: &egui::Context, strips: &[Rect]) {
    let Some(mut d) = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())) else { return };
    if d.dropped {
        ctx.data_mut(|m| m.remove::<TabDrag>(drag_key()));
        return;
    }
    let (pos, secondary, focused) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.secondary_pressed(), i.focused));
    if secondary || !focused || strips.is_empty() {
        tab_drag_cancel(ctx);
        return;
    }
    if let Some(h) = pos.and_then(|p| strips.iter().position(|r| r.expand2(vec2(0.0, sp::M)).contains(p))) {
        d.host = h;
    }
    if d.host >= strips.len() {
        d.host = d.from_pane.min(strips.len() - 1);
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    ctx.data_mut(|m| m.insert_temp(drag_key(), d));
}

/// Nombre de un grupo recortado para que su chip no pase de `GROUP_CHIP_MAX`.
fn group_name_fit(p: &egui::Painter, name: &str, color: Color32) -> String {
    let fits = |s: &str| design::chip_width(p, &Chip { text: s, dot: Some(color), ..Default::default() }) <= GROUP_CHIP_MAX;
    if fits(name) {
        return name.to_string();
    }
    let mut s: String = name.to_string();
    while !s.is_empty() && !fits(&format!("{s}…")) {
        s.pop();
    }
    format!("{s}…")
}

/// Ancho de la etiqueta de un grupo en la barra; `hidden`: cuántas pestañas guarda plegado.
fn group_chip_w(p: &egui::Painter, t: &Theme, g: &TabGroup, hidden: usize) -> f32 {
    let color = t.tags[g.color.min(6)];
    let count = if hidden > 0 { fsx::thousands(hidden) } else { String::new() };
    design::chip_width(p, &Chip { text: &group_name_fit(p, &g.name, color), dot: Some(color), count: &count, ..Default::default() })
}

/// Una etiqueta de grupo en la tira.
struct GroupChip<'a> {
    g: &'a TabGroup,
    pane: usize,
    /// Lo pintado en este cuadro (puede estar creciendo o cerrándose).
    rect: Rect,
    /// Ancho de destino: el contenido se arma con este y se recorta a lo pintado.
    target_w: f32,
    /// Pestañas que guarda plegado (las que no se ven).
    hidden: usize,
    /// Se está yendo: solo se pinta, apagándose.
    presence: f32,
    leaving: bool,
    /// Zona para soltar la pestaña arrastrada (el puntero está encima y el grupo está plegado).
    drop_zone: bool,
    dragging: bool,
}

/// Renombrando un grupo: (grupo, texto, primer cuadro, panel).
type GroupRename = (u64, String, bool, usize);

fn rename_key() -> Id {
    Id::new("group-rename")
}

/// Empezar a renombrar el grupo `g` en el panel `pane` (el campo aparece en su etiqueta).
pub fn start_group_rename(ctx: &egui::Context, g: u64, name: &str, pane: usize) {
    ctx.data_mut(|d| d.insert_temp::<GroupRename>(rename_key(), (g, name.to_string(), true, pane)));
}

/// La etiqueta de un grupo: clic pliega o despliega, doble clic cambia el nombre, clic derecho
/// ofrece color, nombre, desagrupar y cerrar. Plegado, muestra cuántas pestañas guarda.
fn group_chip(ui: &mut Ui, t: &Theme, c: GroupChip, actions: &mut Vec<Action>) {
    let g = c.g;
    let ctx = ui.ctx().clone();
    let color = t.tags[g.color.min(6)];
    let clip = c.rect.intersect(ui.clip_rect());
    let mut p = ui.painter().with_clip_rect(clip);
    let pill = Rect::from_min_size(pos2(c.rect.left(), c.rect.center().y - h::CHIP / 2.0), vec2(c.rect.width(), h::CHIP));
    let full = Rect::from_min_size(pill.min, vec2(c.target_w, h::CHIP));
    let name = group_name_fit(&p, &g.name, color);
    let chip = Chip { text: &name, dot: Some(color), ..Default::default() };
    if c.leaving {
        p.multiply_opacity(c.presence);
        design::chip_paint_rect(&p, t, pill, &chip, false);
        return;
    }
    p.multiply_opacity(0.3 + 0.7 * c.presence);
    let renaming: Option<GroupRename> = ui.data(|d| d.get_temp(rename_key())).filter(|r: &GroupRename| r.0 == g.id && r.3 == c.pane);
    if let Some((id, mut buf, first, pane)) = renaming {
        // Como el renombrado de archivos: campo hundido con anillo de foco. Se arma al ancho de
        // destino (escribir no espera a que la etiqueta termine de crecer) y se recorta.
        p.rect_filled(pill, radius::S, t.sidebar);
        design::focus_ring(&p, t, pill, radius::S);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full.shrink2(vec2(sp::S - sp::XXS, 0.0))));
        child.set_clip_rect(clip);
        let out = egui::TextEdit::singleline(&mut buf).frame(egui::Frame::NONE).desired_width(full.width() - 2.0 * sp::S).font(text::small()).vertical_align(egui::Align::Center).show(&mut child);
        if first {
            out.response.request_focus();
            let mut st = out.state.clone();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(buf.chars().count()))));
            st.store(ui.ctx(), out.response.id);
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if enter || (out.response.lost_focus() && !first) {
            actions.push(Action::RenameTabGroup(id, buf));
            ui.data_mut(|d| d.remove::<GroupRename>(rename_key()));
        } else if esc {
            ui.data_mut(|d| d.remove::<GroupRename>(rename_key()));
        } else {
            ui.data_mut(|d| d.insert_temp::<GroupRename>(rename_key(), (id, buf, false, pane)));
        }
        return;
    }
    let resp = ui.interact(pill, Id::new(("tabgroup", c.pane, g.id)), Sense::click());
    let resp = if c.dragging {
        resp
    } else {
        let tip = if !g.collapsed {
            t!("barra.grupo_desplegado").to_string()
        } else if c.hidden > 0 {
            tp!(c.hidden, "barra.grupo_plegado_n")
        } else {
            t!("barra.grupo_plegado").to_string()
        };
        design::tip(resp, t, &tip)
    };
    design::chip_paint_rect(&p, t, pill, &chip, resp.hovered() && !c.dragging);
    // Plegado: cuántas guarda (aparece y se apaga con el plegado) y la raya del grupo bajo la
    // etiqueta. Al desplegar se sigue viendo el último conteo mientras se apaga.
    let fold = super::motion::fade(&ctx, Id::new(("tabgroup-fold", c.pane, g.id)), g.collapsed, dur::SLOW);
    let count_key = Id::new(("tabgroup-count", c.pane, g.id));
    if c.hidden > 0 {
        ui.data_mut(|d| d.insert_temp(count_key, c.hidden));
    }
    let shown = if c.hidden > 0 { c.hidden } else { ui.data(|d| d.get_temp::<usize>(count_key)).unwrap_or(0) };
    if fold > 0.0 && shown > 0 {
        let base = design::chip_width(&p, &chip);
        let x = pill.left() + base - sp::S + sp::XS;
        p.text(pos2(x, pill.center().y), Align2::LEFT_CENTER, fsx::thousands(shown), text::mono_s(), t.faint.gamma_multiply(fold));
    }
    if fold > 0.0 {
        let y = c.rect.bottom() - stroke::MARK / 2.0;
        p.line_segment([pos2(pill.left(), y), pos2(pill.right(), y)], Stroke::new(stroke::MARK, color.gamma_multiply(fold)));
    }
    // Destello en el color del grupo (no el acento) al crearlo, plegarlo, desplegarlo o recibir una pestaña.
    if let Some(at) = ui.data(|d| d.get_temp::<f64>(Id::new(("tabgroup-pulse", g.id)))) {
        let a = GROUP_PULSE * (1.0 - super::motion::since(&ctx, at, dur::SLOW));
        if a > 0.0 {
            p.rect_filled(pill, radius::S, color.gamma_multiply(a));
        }
    }
    if c.drop_zone {
        p.rect_filled(pill, radius::S, t.accent_soft);
        design::focus_ring(&p, t, pill, radius::S);
    }
    if resp.double_clicked() {
        // El primer clic ya plegó o desplegó: se deshace (la animación vuelve sola) y se renombra.
        actions.push(Action::ToggleTabGroup(g.id));
        start_group_rename(ui.ctx(), g.id, &g.name, c.pane);
    } else if resp.clicked() {
        actions.push(Action::ToggleTabGroup(g.id));
    }
    design::context_menu(&resp, t, |ui| {
        ui.horizontal(|ui| {
            // Siete botones mini con un punto de 12; el elegido, con capa gris.
            ui.spacing_mut().item_spacing.x = sp::XXS;
            ui.add_space(sp::XS);
            for k in 0..t.tags.len() {
                let cresp = design::icon_button(ui, t, "", IconBtn { on: k == g.color, ..IconBtn::mini() });
                let center = cresp.rect.center();
                ui.painter().circle_filled(center, 6.0, t.tags[k]);
                if !t.dark {
                    ui.painter().circle_stroke(center, 6.0 - stroke::HAIR / 2.0, Stroke::new(stroke::HAIR, Color32::from_black_alpha(41)));
                }
                let cresp = design::tip(cresp, t, crate::ui::views::tag_label(k));
                if cresp.clicked() {
                    actions.push(Action::RecolorTabGroup(g.id, k));
                    ui.close();
                }
            }
        });
        design::menu_sep(ui, t);
        if design::menu_item(ui, t, ic::PENCIL_SIMPLE, t!("barra.renombrar_grupo"), "").clicked() {
            start_group_rename(ui.ctx(), g.id, &g.name, c.pane);
        }
        if design::menu_item(ui, t, ic::SQUARES_FOUR, t!("barra.desagrupar"), "").clicked() {
            actions.push(Action::Ungroup(g.id));
        }
        if design::menu_item(ui, t, ic::X, t!("barra.cerrar_grupo"), "").clicked() {
            actions.push(Action::CloseTabGroup(g.id));
        }
    });
}

/// Botón mini de la tira de pestañas (X y "+"), con su ícono a su tamaño óptico.
fn tab_btn(ui: &Ui, t: &Theme, rect: Rect, resp: &Response, glyph: &str, size: f32, nudge: f32) {
    let p = ui.painter();
    let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
    design::state_fill(p, t, rect, radius::S, hov, resp.is_pointer_button_down_on(), Sel::None);
    p.text(rect.center() + vec2(0.0, nudge), Align2::CENTER_CENTER, glyph, design::icon_font(size), if resp.hovered() { t.text } else { t.faint });
}

/// Ícono y título de una pestaña.
fn tab_visual(tab: &Tab) -> (&'static str, String) {
    let q = tab.search.text.trim_start().to_lowercase();
    let recent = q.starts_with("recientes:");
    let icon = if let Some(tool) = &tab.tool {
        tool.icon()
    } else if recent {
        ic::CLOCK_COUNTER_CLOCKWISE
    } else if fsx::tags::query_color(&tab.search.text).is_some() {
        ic::TAG
    } else if fsx::collections::query_name(&tab.search.text).is_some() {
        ic::BOOKMARKS_SIMPLE
    } else if tab.search.active() {
        ic::MAGNIFYING_GLASS
    } else {
        ic::FOLDER
    };
    let tag = fsx::tags::query_color(&tab.search.text).filter(|_| tab.tool.is_none());
    let coll = fsx::collections::query_name(&tab.search.text).filter(|_| tab.tool.is_none());
    let title = if recent && tab.tool.is_none() {
        t!("barra.recientes").to_string()
    } else if let Some(c) = tag {
        crate::ui::views::tag_label(c).to_string()
    } else if let Some(n) = coll {
        n
    } else if tab.search.active() && tab.tool.is_none() {
        format!("“{}”", tab.search.text.trim())
    } else {
        tab.title()
    };
    (icon, title)
}

/// Ícono y título de una pestaña dentro de `r`, armados con el ancho de destino `target_w` (así el
/// título no cambia de recorte mientras la pestaña crece); quien llama recorta a lo pintado.
/// `hide_icon`: muy angosta y con el puntero encima, la X ocupa el lugar del ícono.
fn tab_content(p: &egui::Painter, t: &Theme, r: Rect, target_w: f32, tab: &Tab, active: bool, hide_icon: bool) {
    let (icon, title) = tab_visual(tab);
    let r = Rect::from_min_size(r.min, vec2((target_w - sp::XS).max(1.0), r.height()));
    let narrow = target_w < 84.0;
    let tiny = target_w < 58.0;
    if !hide_icon {
        // Ícono en +8 de la pestaña: a plomo con el de las filas.
        let ip = if tiny { (r.center(), Align2::CENTER_CENTER) } else { (pos2(r.left() + sp::S, r.center().y), Align2::LEFT_CENTER) };
        // La carpeta va rellena y en su color solo en la activa; en las demás, contorno.
        let folder = active && icon == ic::FOLDER;
        let font = if folder { design::icon_fill(icon::M) } else { design::icon_font(icon::M) };
        p.text(ip.0, ip.1, icon, font, if folder { t.folder } else { t.dim });
    }
    if !narrow {
        // Texto tras el ícono; a la derecha, el lugar de la X.
        let x_text = r.left() + sp::S + icon::M + sp::S;
        let g = elided(p, &title, text::body(), if active { t.text } else { t.dim }, r.right() - TAB_X_INSET - h::CHIP / 2.0 - sp::XS - x_text);
        p.galley(pos2(x_text, r.center().y - g.size().y / 2.0), g, t.text);
    }
}

/// Fondo de la pestaña activa: relleno del contenido con borde a la izquierda, arriba y a la
/// derecha (sin base: se funde con lo de abajo).
fn active_tab_bg(p: &egui::Painter, t: &Theme, r: Rect) {
    let top = radius::M as u8;
    let corners = egui::CornerRadius { nw: top, ne: top, sw: 0, se: 0 };
    p.rect_filled(r, corners, t.bg);
    let clipped = p.with_clip_rect(r.intersect(p.clip_rect()));
    clipped.rect_stroke(r.with_max_y(r.bottom() + 2.0 * stroke::HAIR), corners, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
}

/// Subrayado de grupo bajo una pestaña: largo `k` (0 a 1) desde la izquierda.
fn group_mark(p: &egui::Painter, r: Rect, color: Color32, k: f32) {
    if k <= 0.0 {
        return;
    }
    let y = r.bottom() - stroke::MARK / 2.0;
    let x0 = r.left() + sp::S;
    p.line_segment([pos2(x0, y), pos2(x0 + (r.width() - 2.0 * sp::S).max(0.0) * k, y)], Stroke::new(stroke::MARK, color));
}

/// Lo que necesita la tira de pestañas de un panel.
pub struct TabBarIn<'a> {
    pub t: &'a Theme,
    pub tabs: &'a [Tab],
    pub groups: &'a [TabGroup],
    pub pane: usize,
    /// Índices (en `tabs`) de las pestañas del panel, en orden.
    pub show: &'a [usize],
    /// La pestaña activa del panel.
    pub cur: usize,
    /// Espacio a la derecha que no se usa (ahí van los botones de la ventana).
    pub reserve: f32,
}

/// Pestañas de un panel. Devuelve dónde termina el botón "+" (lo que sigue es espacio libre:
/// barra de título). Cada etiqueta y cada pestaña tiene su lugar en una `motion::Strip`: abrir,
/// cerrar, plegar, desplegar, agrupar y reordenar se animan solos.
pub fn tab_bar(ui: &mut Ui, s: TabBarIn, actions: &mut Vec<Action>) -> f32 {
    let TabBarIn { t, tabs, groups, pane, show, cur, reserve } = s;
    let ctx = ui.ctx().clone();
    let full = ui.available_rect_before_wrap();
    let painter = ui.painter().clone();
    let ppp = ctx.pixels_per_point();
    let px = |v: f32| (v * ppp).round() / ppp;
    let top = full.top() + TAB_TOP;
    let height = full.height() - TAB_TOP;
    // ---- arrastre: si esta tira tiene el hueco, dónde va
    let mut drag = ctx.data(|d| d.get_temp::<TabDrag>(drag_key())).filter(|d| !d.dropped);
    if drag.as_ref().is_some_and(|d| !tabs.iter().any(|x| x.id == d.id)) {
        // Se cerró mientras se arrastraba.
        ctx.data_mut(|d| d.remove::<TabDrag>(drag_key()));
        drag = None;
    }
    let dragging = drag.is_some();
    let dragged = drag.as_ref().map(|d| d.id);
    let hosting = drag.as_ref().filter(|d| d.host == pane).cloned();
    let dragged_i = dragged.and_then(|id| tabs.iter().position(|x| x.id == id));
    let mut items = bar_items(tabs, groups, show, cur, dragged);
    // Pestañas que guarda cada grupo plegado (en este panel): las suyas que no se ven.
    let hidden_of = |g: &TabGroup| -> usize { if g.collapsed { show.iter().filter(|&&j| tabs[j].group == g.id && j != cur && Some(tabs[j].id) != dragged).count() } else { 0 } };
    let chip_w: Vec<f32> = groups.iter().map(|g| group_chip_w(&painter, t, g, hidden_of(g))).collect();
    let chips: f32 = items.iter().map(|it| if let BarItem::Group(gi) = it { chip_w[*gi] + sp::XS } else { 0.0 }).sum();
    let n = items.iter().filter(|it| matches!(it, BarItem::Tab(_))).count() + hosting.is_some() as usize;
    let n = n.max(1);
    // Se achican hasta caber (antes tenían un mínimo y, con muchas, tapaban los botones de la
    // ventana): primero se recorta el título, luego queda solo el ícono.
    let room = full.width() - reserve - TAB_PLUS_W - sp::M - chips;
    let even = (room / n as f32).clamp(TAB_MIN_W, TAB_MAX_W);
    // La activa conserva su título (al menos 120): las demás se achican primero. Mientras esta tira
    // tiene el hueco, el hueco se mide como la activa (lo será al soltar).
    let active_shown = hosting.is_some() || items.iter().any(|it| matches!(it, BarItem::Tab(i) if *i == cur));
    let (active_w, other_w) = if n > 1 && active_shown && even < TAB_ACTIVE_MIN_W {
        let a = TAB_ACTIVE_MIN_W.min(room - TAB_MIN_W * (n - 1) as f32).max(even);
        (a, ((room - a) / (n - 1) as f32).clamp(TAB_MIN_W, TAB_MAX_W))
    } else {
        (even, even)
    };
    let is_active = |it: &BarItem| match it {
        BarItem::Tab(i) => *i == cur && hosting.is_none(),
        BarItem::Hole(_) => true,
        BarItem::Group(_) => false,
    };
    let slot_w = |it: &BarItem| match it {
        BarItem::Group(gi) => chip_w[*gi] + sp::XS,
        _ if is_active(it) => active_w,
        _ => other_w,
    };
    // La primera pestaña a plomo con el pill de las filas (4 del borde).
    let x0 = full.left() + sp::XS;
    let layout = |items: &[BarItem]| -> Vec<(f32, f32)> {
        let mut x = x0;
        items
            .iter()
            .map(|it| {
                let w = slot_w(it);
                x += w;
                (x - w, w)
            })
            .collect()
    };
    // (borde izquierdo y ancho de la levantada, grupo del hueco, chip plegado de destino)
    let mut lift: Option<(f32, f32, u64, Option<u64>)> = None;
    let mut drop_before: Option<u64> = None;
    if let (Some(d), Some(di)) = (&hosting, dragged_i) {
        let wo = layout(&items);
        let kinds: Vec<(DropKind, f32, f32)> = items
            .iter()
            .zip(&wo)
            .map(|(it, &(x, w))| {
                let k = match it {
                    BarItem::Group(gi) => DropKind::Chip { group: groups[*gi].id, collapsed: groups[*gi].collapsed },
                    BarItem::Tab(j) => DropKind::Tab { group: if groups.iter().any(|g| g.id == tabs[*j].group) { tabs[*j].group } else { 0 } },
                    BarItem::Hole(_) => DropKind::Tab { group: 0 },
                };
                (k, x, x + w)
            })
            .collect();
        let pointer = ctx.input(|i| i.pointer.latest_pos()).map_or(d.lift[0] + d.grab * active_w, |p| p.x);
        let max_l = (full.right() - reserve - TAB_PLUS_W - sp::M - active_w).max(full.left() + sp::XS);
        let l = (pointer - d.grab * active_w).clamp(full.left() + sp::XS, max_l);
        let (index, group, into) = drop_target(&kinds, l, pointer);
        let rest: Vec<usize> = show.iter().copied().filter(|&j| Some(tabs[j].id) != dragged).collect();
        drop_before = match into {
            Some(g) => rest.iter().rposition(|&j| tabs[j].group == g).and_then(|k| rest.get(k + 1)).map(|&j| tabs[j].id),
            None => match items.get(index) {
                Some(BarItem::Tab(j)) => Some(tabs[*j].id),
                Some(BarItem::Group(gi)) => rest.iter().find(|&&j| tabs[j].group == groups[*gi].id).map(|&j| tabs[j].id),
                _ => None,
            },
        };
        items.insert(index, BarItem::Hole(di));
        lift = Some((l, active_w, group, into));
    }
    // ---- destinos y lo pintado
    let key_of = |it: &BarItem| match it {
        BarItem::Group(gi) => SlotKey::Group(groups[*gi].id),
        BarItem::Tab(i) | BarItem::Hole(i) => SlotKey::Tab(tabs[*i].id),
    };
    let targets: Vec<(SlotKey, f32, f32)> = items.iter().zip(layout(&items)).map(|(it, (x, w))| (key_of(it), x, w)).collect();
    let placed = super::motion::strip(&ctx, strip_id(pane), &targets);
    // Hairline al pie de la tira: la pestaña activa la tapa con su relleno y queda abierta abajo.
    painter.hline(full.x_range(), full.bottom() - stroke::HAIR / 2.0, Stroke::new(stroke::HAIR, t.border));
    let tab_order: Vec<BarItem> = items.iter().copied().filter(|it| !matches!(it, BarItem::Group(_))).collect();
    let mut x_end = x0;
    for pl in &placed {
        let (x, right) = (px(pl.x), px(pl.x + pl.w));
        x_end = x_end.max(right);
        if right - x < 0.5 {
            continue;
        }
        let item = items.iter().copied().find(|it| key_of(it) == pl.key && !pl.leaving);
        match pl.key {
            SlotKey::Group(gid) => {
                let Some(gi) = groups.iter().position(|g| g.id == gid) else {
                    // Un grupo que ya no existe: solo su píldora, apagándose.
                    let r = Rect::from_min_size(pos2(x + sp::XXS, top + (height - h::CHIP) / 2.0), vec2(right - x - sp::XS, h::CHIP));
                    painter.with_clip_rect(full).rect_filled(r, radius::S, t.hover.gamma_multiply(pl.presence));
                    continue;
                };
                let r = Rect::from_min_size(pos2(x + sp::XXS, top), vec2((right - x - sp::XS).max(0.0), height));
                let chip = GroupChip {
                    g: &groups[gi],
                    pane,
                    rect: r,
                    target_w: pl.target_w - sp::XS,
                    hidden: hidden_of(&groups[gi]),
                    presence: pl.presence,
                    leaving: pl.leaving,
                    drop_zone: lift.is_some_and(|l| l.3 == Some(gid)),
                    dragging,
                };
                group_chip(ui, t, chip, actions);
            }
            SlotKey::Tab(id) => {
                let r = Rect::from_min_size(pos2(x, top), vec2((right - x - sp::XS).max(0.0), height));
                let p = painter.with_clip_rect(r.intersect(ui.clip_rect()));
                if Some(id) == dragged {
                    // El hueco: vacío, con la raya del grupo al que entraría.
                    if let (Some((_, _, g, _)), Some(BarItem::Hole(_))) = (lift, item)
                        && let Some(gr) = groups.iter().find(|x| x.id == g && g != 0)
                    {
                        group_mark(&p, r, t.tags[gr.color.min(6)], 1.0);
                    }
                    continue;
                }
                let Some(i) = tabs.iter().position(|x| x.id == id) else {
                    // Cerrada: solo su píldora, apagándose.
                    p.rect_filled(Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - sp::XS)), radius::M, t.hover.gamma_multiply(pl.presence));
                    continue;
                };
                let tab = &tabs[i];
                if pl.leaving {
                    // Sigue existiendo (se plegó dentro de su grupo, pasó a otro panel): se cierra
                    // con su contenido, apagándose.
                    let mut p = p;
                    p.multiply_opacity(pl.presence);
                    tab_content(&p, t, r, pl.target_w, tab, false, false);
                    continue;
                }
                let active = item.is_some_and(|it| is_active(&it));
                tab_slot(ui, t, TabSlot { tab, i, r, target_w: pl.target_w, presence: pl.presence, travel: pl.top, active, dragging, pane, groups, next: tab_order.iter().position(|it| *it == BarItem::Tab(i)).and_then(|k| tab_order.get(k + 1)).copied(), is_active: &is_active }, actions);
            }
        }
    }
    // ---- la levantada: encima de todo (puede cruzar al otro panel), solo en horizontal
    if let (Some(d), Some((l, w, group, into)), Some(di)) = (&hosting, lift, dragged_i) {
        let w = super::motion::glide(&ctx, Id::new("tab-lift-w"), w, dur::SLOW);
        let y = super::motion::glide(&ctx, Id::new("tab-lift-y"), top, dur::SLOW);
        let lr = Rect::from_min_size(pos2(px(l), px(y)), vec2(px(w - sp::XS), height));
        let k = super::motion::since(&ctx, d.start, dur::FAST);
        let mut lp = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("tab-lift")));
        if into.is_some() {
            // Sobre la etiqueta de un grupo plegado: la levantada deja ver la zona para soltar.
            lp.multiply_opacity(0.7);
        }
        let mut shadow = design::popover_shadow(t);
        shadow.color = shadow.color.gamma_multiply(k);
        lp.add(shadow.as_shape(lr, radius::M));
        lp.rect_filled(lr, radius::M, t.bg.lerp_to_gamma(t.raised, k));
        lp.rect_stroke(lr, radius::M, Stroke::new(stroke::HAIR, t.border.lerp_to_gamma(t.border_strong, k)), StrokeKind::Inside);
        let inner = lp.with_clip_rect(lr);
        tab_content(&inner, t, lr, w, &tabs[di], true, false);
        if let Some(gr) = groups.iter().find(|x| x.id == group && group != 0) {
            group_mark(&inner, lr, t.tags[gr.color.min(6)], 1.0);
        }
        let released = ctx.input(|i| !i.pointer.primary_down());
        let mut d = d.clone();
        d.lift = [l, w];
        if released {
            // Soltar en cualquier lado = soltar en el hueco. La pestaña sale desde donde se la soltó.
            actions.push(Action::MoveTab { id: d.id, pane, before: drop_before, group });
            super::motion::strip_seed(&ctx, strip_id(pane), SlotKey::Tab(d.id), l, w);
            d.dropped = true;
        }
        ctx.data_mut(|m| m.insert_temp(drag_key(), d));
    }
    // Mismo tamaño y misma altura que la X de las pestañas (centrado con su texto).
    let pr = Rect::from_center_size(pos2(x_end + sp::XS + h::CHIP / 2.0, top + height / 2.0), vec2(h::CHIP, h::CHIP));
    let presp = ui.interact(pr, Id::new(("newtab", pane)), Sense::click());
    let presp = if dragging { presp } else { design::tip_keys(presp, t, t!("barra.pestana_nueva_ctrl_t"), &kbd_keys(&super::commands::get("new_tab").keys())) };
    tab_btn(ui, t, pr, &presp, ic::PLUS, TAB_PLUS_ICON, TAB_PLUS_NUDGE);
    if presp.clicked() {
        actions.push(Action::NewTab(None));
    }
    ui.allocate_rect(full, Sense::hover());
    pr.right()
}

/// Una pestaña viva en la tira.
struct TabSlot<'a> {
    tab: &'a Tab,
    /// Índice en `tabs`.
    i: usize,
    /// Lo pintado en este cuadro.
    r: Rect,
    target_w: f32,
    presence: f32,
    /// Viaja a otro lugar (encima de las demás): lleva relleno para tapar lo que cruza.
    travel: bool,
    active: bool,
    /// Se está arrastrando una pestaña: sin hover, X ni tooltips.
    dragging: bool,
    pane: usize,
    groups: &'a [TabGroup],
    /// Lo que sigue en la tira (sin las etiquetas): para el divisor.
    next: Option<BarItem>,
    is_active: &'a dyn Fn(&BarItem) -> bool,
}

fn tab_slot(ui: &mut Ui, t: &Theme, s: TabSlot, actions: &mut Vec<Action>) {
    let TabSlot { tab, i, r, target_w, presence, travel, active, dragging, pane, groups, next, is_active } = s;
    let ctx = ui.ctx().clone();
    let narrow = target_w < 84.0;
    let tiny = target_w < 58.0;
    let mut p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
    // Lo que entra aparece con la receta de siempre (desde 0,3).
    if presence < 1.0 {
        p.multiply_opacity(0.3 + 0.7 * presence);
    }
    let resp = ui.interact(r, Id::new(("tab", tab.id)), Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) && !dragging {
        // Se levanta: pasa a ser la activa y su panel toma el foco.
        let grab = ctx.input(|i| i.pointer.press_origin()).map_or(0.5, |o| ((o.x - r.left()) / r.width().max(1.0)).clamp(0.0, 1.0));
        let now = ctx.input(|i| i.time);
        ctx.data_mut(|d| d.insert_temp(drag_key(), TabDrag { id: tab.id, grab, from_pane: pane, host: pane, start: now, lift: [r.left(), target_w], dropped: false }));
        actions.push(Action::SwitchTab(i));
    }
    // Arrastrar archivos sobre una pestaña: tras un momento se cambia a ella; soltarlos ahí los lleva a su carpeta.
    let drop_hover = tab.tool.is_none() && crate::ui::views::drag_hovering(&resp);
    if drop_hover && !active {
        let key = Id::new("tab-drag-hover");
        let now = ui.input(|i| i.time);
        let pass = ctx.cumulative_pass_nr();
        // (pestaña, desde cuándo, último cuadro): si el puntero se fue, el tiempo vuelve a empezar.
        let since = ui.data_mut(|d| {
            let t0 = match d.get_temp::<(u64, f64, u64)>(key) {
                Some((id, t0, last)) if id == tab.id && last + 1 >= pass => t0,
                _ => now,
            };
            d.insert_temp(key, (tab.id, t0, pass));
            t0
        });
        if now - since > super::motion::DWELL_TAB {
            actions.push(Action::SwitchTab(i));
            ui.data_mut(|d| d.remove::<(u64, f64, u64)>(key));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
    }
    if tab.tool.is_none() {
        if let Some(files) = crate::ui::views::take_drop(&resp) {
            actions.push(Action::DropFiles { paths: files, dest: tab.path().to_string(), force: crate::ui::views::drop_force() });
        }
    }
    // Se mira la posición del ratón (no `hovered`): la X queda encima de la pestaña y,
    // si dependiera de ella, la X aparecería y desaparecería en cuadros alternos.
    let hovering = !dragging && ui.rect_contains_pointer(r);
    if active {
        active_tab_bg(&p, t, r);
    } else {
        // Sin tocar el fondo de la tira: 4 px de aire abajo.
        let hr = Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - sp::XS));
        if travel {
            p.rect_filled(r, radius::M, t.chrome);
        }
        let hov = design::hover_amount(&ctx, resp.id, hovering);
        design::state_fill(&p, t, hr, radius::M, hov, false, Sel::None);
    }
    if drop_hover {
        p.rect_filled(r, radius::M, t.accent_soft);
        design::focus_ring(&p, t, r, radius::M);
    }
    tab_content(&p, t, r, target_w, tab, active, tiny && active && hovering);
    let (_, title) = tab_visual(tab);
    let resp = if narrow && !dragging { design::tip(resp, t, &title) } else { resp };
    // Las pestañas de un grupo llevan su color abajo: crece desde la izquierda al entrar y se
    // retrae con su último color al salir.
    let mark_id = Id::new(("tab-mark", tab.id));
    let gcolor = groups.iter().find(|g| g.id == tab.group && tab.group != 0).map(|g| t.tags[g.color.min(6)]);
    if let Some(c) = gcolor {
        ui.data_mut(|d| d.insert_temp(mark_id.with("c"), c));
    }
    let mark = super::motion::fade(&ctx, mark_id, gcolor.is_some(), dur::SLOW);
    if let Some(c) = gcolor.or_else(|| ui.data(|d| d.get_temp::<Color32>(mark_id.with("c")))) {
        group_mark(&p, r, c, mark);
    }
    design::context_menu(&resp, t, |ui| {
        if design::menu_item(ui, t, ic::PLUS_SQUARE, t!("barra.nuevo_grupo_con_pestana"), "").clicked() {
            actions.push(Action::NewTabGroup(i));
        }
        for g in groups.iter().filter(|g| g.id != tab.group) {
            if design::menu_item(ui, t, ic::ARROW_SQUARE_RIGHT, &tf!("barra.mover_a_grupo", nombre = g.name), "").clicked() {
                actions.push(Action::SetTabGroup(i, g.id));
            }
        }
        if tab.group != 0 && design::menu_item(ui, t, ic::MINUS_SQUARE, t!("barra.quitar_del_grupo"), "").clicked() {
            actions.push(Action::SetTabGroup(i, 0));
        }
        design::menu_sep(ui, t);
        if design::menu_item(ui, t, ic::X, t!("barra.cerrar_pestana"), "").clicked() {
            actions.push(Action::CloseTab(i));
        }
    });
    if resp.clicked() {
        actions.push(Action::SwitchTab(i));
    }
    if resp.middle_clicked() {
        actions.push(Action::CloseTab(i));
    }
    // La X existe siempre (así no cambia lo que hay bajo el ratón); se ve en la activa y al pasar
    // por encima. Se arma con el ancho de destino; mientras la pestaña crece queda recortada.
    let rt = Rect::from_min_size(r.min, vec2(target_w - sp::XS, r.height()));
    let close_r = if tiny { Rect::from_center_size(rt.center(), vec2(h::CHIP, h::CHIP)) } else { Rect::from_center_size(pos2(rt.right() - TAB_X_INSET, rt.center().y), vec2(h::CHIP, h::CHIP)) };
    // Muy angosta, la X va en el centro: solo en la activa (en las demás, un clic cambia a ella).
    let has_x = (!tiny || active) && !dragging && r.contains_rect(close_r);
    let cresp = ui.interact(if has_x { close_r } else { Rect::NOTHING }, Id::new(("tabx", tab.id)), Sense::click());
    if has_x && (hovering || (active && !tiny)) {
        tab_btn(ui, t, close_r, &cresp, ic::X, TAB_X_ICON, 0.0);
        if cresp.clicked() {
            actions.push(Action::CloseTab(i));
        }
    }
    // Divisor fino entre pestañas inactivas.
    if !active && next.is_some_and(|n| !is_active(&n) && !matches!(n, BarItem::Hole(_))) {
        let sx = r.right() + sp::XXS - stroke::HAIR / 2.0;
        ui.painter().line_segment([pos2(sx, r.top() + sp::S), pos2(sx, r.bottom() - sp::S)], Stroke::new(stroke::HAIR, t.border));
    }
}

/// Arriba de la barra lateral: el ícono y el nombre de Rayo, discretos.
pub fn app_title(ui: &Ui, t: &Theme, band: Rect) {
    let p = ui.painter();
    // A la altura del texto de las pestañas (que empiezan 6 px más abajo).
    let cy = band.top() + TAB_TOP / 2.0 + band.height() / 2.0;
    let c = pos2(band.left() + 24.0, cy + 1.0);
    // La carpeta con el rayo, como el ícono de la app.
    p.text(c, Align2::CENTER_CENTER, egui_phosphor::fill::FOLDER, design::icon_fill(icon::M), t.accent);
    p.text(c + vec2(0.5, 1.5), Align2::CENTER_CENTER, egui_phosphor::fill::LIGHTNING, design::icon_fill(icon::M / 2.0), t.accent_text);
    p.text(pos2(c.x + sp::L, cy), Align2::LEFT_CENTER, "Rayo", text::strong(), t.dim);
}

/// Minimizar, maximizar / restaurar y cerrar, arriba a la derecha (como en Windows 11).
/// Devuelve el rectángulo del botón de maximizar: para Windows es su botón (así muestra
/// el menú de Snap Layouts), y su clic lo maneja `win`.
pub fn window_buttons(ctx: &egui::Context, t: &Theme) -> Rect {
    let screen = ctx.content_rect();
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    let w = WINDOW_BUTTONS_W / 3.0;
    let origin = pos2(screen.right() - WINDOW_BUTTONS_W, screen.top());
    let mut max_rect = Rect::NOTHING;
    egui::Area::new(Id::new("window-buttons")).order(egui::Order::Middle).fixed_pos(origin).show(ctx, |ui| {
        ui.set_min_size(vec2(WINDOW_BUTTONS_W, TITLE_H));
        let icons = [(ic::MINUS, t!("barra.minimizar")), (if maximized { ic::COPY } else { ic::SQUARE }, if maximized { t!("comun.restaurar") } else { t!("barra.maximizar") }), (ic::X, t!("comun.cerrar"))];
        for (k, (icon, _tip)) in icons.into_iter().enumerate() {
            let r = Rect::from_min_size(origin + vec2(k as f32 * w, 0.0), vec2(w, TITLE_H));
            let resp = ui.interact(r, Id::new(("window-button", k)), Sense::click());
            let hover = resp.hovered() || (k == 1 && crate::ui::win::max_hovered());
            let close = k == 2;
            if hover {
                ui.painter().rect_filled(r, egui::CornerRadius::ZERO, if close { t.danger_fill } else { t.hover });
            }
            let color = if close && hover { t.accent_text } else { t.text };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, design::icon_font(if k == 1 { CAPTION_ICON_MAX } else { CAPTION_ICON }), color);
            if resp.clicked() {
                match k {
                    0 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                    1 => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized)),
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
            if k == 1 {
                max_rect = r;
            }
        }
    });
    max_rect
}

// ---------------------------------------------------------------- barra de herramientas

/// Entre grupos de botones hay 8 px (con los 2 del espaciado entre botones).
const GROUP_GAP: f32 = sp::S - sp::XXS;
/// Margen de la barra de herramientas a cada lado (el botón de atrás y la búsqueda).
const TOOLBAR_EDGE: f32 = 6.0;

/// Clic derecho en un botón de la barra: personalizarla.
fn customize_menu(resp: &Response, t: &Theme, actions: &mut Vec<Action>) {
    design::context_menu(resp, t, |ui| {
        if design::menu_item(ui, t, ic::SLIDERS_HORIZONTAL, t!("cmd.customize_toolbar"), "").clicked() {
            actions.push(Action::CustomizeToolbar);
        }
    });
}

/// Ancho del segmentado de vistas: tres opciones de 28 con 2 px de margen.
const VIEWS_W: f32 = 3.0 * h::CTL + 2.0 * sp::XXS;

/// Columnas / detalles / cuadrícula: un segmentado de tres íconos. Devuelve la respuesta de
/// cada opción.
fn view_switch(ui: &mut Ui, t: &Theme, view: ViewMode, shortcuts: &[(String, String)]) -> [Response; 3] {
    let (rect, _) = ui.allocate_exact_size(vec2(VIEWS_W, h::CTL), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, radius::M, t.sidebar);
    p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
    let opts = [
        (ViewMode::Columns, ic::COLUMNS, t!("barra.vista_columnas_ctrl_1"), "view_columns"),
        (ViewMode::Details, ic::LIST_BULLETS, t!("barra.vista_detalles_ctrl_2"), "view_details"),
        (ViewMode::Grid, ic::SQUARES_FOUR, t!("barra.vista_cuadricula_ctrl_3"), "view_grid"),
    ];
    let opt_w = (VIEWS_W - 2.0 * sp::XXS) / 3.0;
    opts.map(|(mode, glyph, tip, cmd)| {
        let k = match mode {
            ViewMode::Columns => 0.0,
            ViewMode::Details => 1.0,
            ViewMode::Grid => 2.0,
        };
        let r = Rect::from_min_size(pos2(rect.left() + sp::XXS + k * opt_w, rect.top() + sp::XXS), vec2(opt_w, h::CTL - 2.0 * sp::XXS));
        let resp = ui.interact(r, ui.id().with(("view-seg", k as u8)), Sense::click());
        let on = view == mode;
        let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
        design::state_fill(&p, t, r, radius::S, hov, resp.is_pointer_button_down_on(), if on { Sel::Unfocused } else { Sel::None });
        let c = if on {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.dim
        };
        p.text(r.center(), Align2::CENTER_CENTER, glyph, design::icon_font(icon::M), c);
        design::tip_keys(resp, t, tip, &cmd_keys(cmd, shortcuts))
    })
}

/// Panel angosto: las tres vistas en un solo botón (con el glifo de la actual) que abre un menú
/// con las tres, la actual marcada. Devuelve la elegida.
fn view_menu(ui: &mut Ui, t: &Theme, view: ViewMode, shortcuts: &[(String, String)], actions: &mut Vec<Action>) -> Option<ViewMode> {
    let opts = [
        (ViewMode::Columns, ic::COLUMNS, t!("barra.vista_columnas_ctrl_1"), "view_columns"),
        (ViewMode::Details, ic::LIST_BULLETS, t!("barra.vista_detalles_ctrl_2"), "view_details"),
        (ViewMode::Grid, ic::SQUARES_FOUR, t!("barra.vista_cuadricula_ctrl_3"), "view_grid"),
    ];
    let (_, glyph, label, cmd) = opts.iter().copied().find(|o| o.0 == view).unwrap_or(opts[1]);
    let r = design::tip_keys(icon_btn(ui, t, glyph, true, false, ""), t, label, &cmd_keys(cmd, shortcuts));
    customize_menu(&r, t, actions);
    let mut picked = None;
    design::popup_menu(&r, t, |ui| {
        for (mode, icon, label, cmd) in opts {
            let keys = cmd_keys(cmd, shortcuts);
            if design::menu_item_ex(ui, t, MenuItem { icon, label, keys: &keys, checked: Some(mode == view), ..Default::default() }).clicked() {
                picked = Some(mode);
            }
        }
    });
    picked
}

/// `enabled`: si cada botón propio (de `cfg.toolbar`) tiene sentido con lo seleccionado ahora.
#[allow(clippy::too_many_arguments)]
pub fn toolbar(ui: &mut Ui, t: &Theme, cfg: &Config, tab: &mut Tab, search_id: Id, enabled: &[bool], shortcuts: &[(String, String)], actions: &mut Vec<Action>) {
    // Panel angosto (cuatro paneles, dividido): la búsqueda se pliega a un botón y, al abrirla,
    // ocupa el lugar de la ruta hasta Esc o hasta perder el foco vacía (M-7).
    let narrow = ui.max_rect().width() < design::NARROW_PANE;
    let exp_id = search_id.with("expanded");
    let was = ui.data(|d| d.get_temp::<bool>(exp_id)).unwrap_or(false);
    let focused = ui.memory(|m| m.has_focus(search_id));
    // Mientras se aprieta (o se suelta) el botón, sigue abierta: el clic en el historial cuenta al soltar.
    let pressing = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    let expanded = narrow && (tab.search.focus || focused || !tab.search.text.is_empty() || (was && pressing));
    ui.data_mut(|d| d.insert_temp(exp_id, expanded));
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = sp::XXS;
        ui.add_space(TOOLBAR_EDGE);
        let nav = [
            (ic::ARROW_LEFT, !tab.back.is_empty(), t!("barra.atras_alt"), "Alt ←", Action::Back),
            (ic::ARROW_RIGHT, !tab.fwd.is_empty(), t!("barra.adelante_alt"), "Alt →", Action::Forward),
            (ic::ARROW_UP, fsx::parent_of(tab.path()).is_some(), t!("barra.subir_alt"), "Alt ↑", Action::Up),
        ];
        for (glyph, on, tip, keys, action) in nav {
            let r = design::tip_keys(icon_btn(ui, t, glyph, on, false, ""), t, tip, keys);
            customize_menu(&r, t, actions);
            if r.clicked() {
                actions.push(action);
            }
        }
        // En panel angosto «Actualizar» no entra: sigue en F5 y en el menú contextual.
        if !narrow {
            let r = design::tip_keys(icon_btn(ui, t, ic::ARROW_CLOCKWISE, true, false, ""), t, t!("barra.actualizar_f5"), &cmd_keys("refresh", shortcuts));
            customize_menu(&r, t, actions);
            if r.clicked() {
                actions.push(Action::Refresh);
            }
        }
        ui.add_space(GROUP_GAP);

        // Lado derecho primero para saber cuánto espacio queda para la ruta.
        // En paneles angostos la búsqueda se achica para dejarle lugar a la ruta; más angostos
        // todavía, queda un botón (o, abierta, va en el lugar de la ruta).
        let search_w = if narrow { h::CTL } else { (ui.available_width() * 0.3).clamp(140.0, 300.0) };
        // Lo que sigue a la ruta, tal como se reserva abajo (cada control lleva su `XXS` detrás):
        // grupo · [botones propios · grupo] · vistas · grupo · vista previa · [grupo · búsqueda] · borde.
        let btn = h::CTL + sp::XXS;
        // En panel angosto no van los botones propios y las tres vistas se pliegan a un botón.
        let show_custom = !narrow && !cfg.toolbar.is_empty();
        let custom_w = if show_custom { btn * cfg.toolbar.len() as f32 + GROUP_GAP } else { 0.0 };
        let views_w = if narrow { h::CTL } else { VIEWS_W };
        let search_slot = if expanded { 0.0 } else { GROUP_GAP + search_w };
        let right_w = sp::XXS + GROUP_GAP + custom_w + views_w + sp::XXS + GROUP_GAP + btn + search_slot + TOOLBAR_EDGE;
        let crumb_w = (ui.available_width() - right_w).max(if narrow { 72.0 } else { 100.0 });
        if expanded {
            search_box(ui, t, tab, search_id, crumb_w, &cfg.search_history, actions);
        } else {
            breadcrumb(ui, t, tab, crumb_w, narrow, actions);
        }
        ui.add_space(GROUP_GAP);

        // Botones propios (Ctrl+K → "Personalizar barra").
        for (k, id) in cfg.toolbar.iter().enumerate().filter(|_| show_custom) {
            let Some(c) = super::commands::ALL.iter().find(|c| c.id == id) else { continue };
            let r = icon_btn(ui, t, c.icon, enabled.get(k).copied().unwrap_or(true), false, "");
            let r = design::tip_keys(r, t, c.label(), &cmd_keys(c.id, shortcuts));
            customize_menu(&r, t, actions);
            if r.clicked() {
                actions.push(Action::RunCommand(c.id.to_string()));
            }
        }
        if show_custom {
            ui.add_space(GROUP_GAP);
        }

        if narrow {
            if let Some(mode) = view_menu(ui, t, cfg.view, shortcuts, actions) {
                actions.push(Action::SetView(mode));
            }
        } else {
            let [cols, details, grid] = view_switch(ui, t, cfg.view, shortcuts);
            customize_menu(&cols, t, actions);
            if cols.clicked() {
                actions.push(Action::SetView(ViewMode::Columns));
            }
            if details.clicked() {
                actions.push(Action::SetView(ViewMode::Details));
            }
            if grid.clicked() {
                actions.push(Action::SetView(ViewMode::Grid));
            }
        }
        ui.add_space(GROUP_GAP);
        let r = icon_btn(ui, t, ic::SIDEBAR_SIMPLE, true, cfg.show_preview, "");
        if design::tip_keys(r, t, t!("barra.panel_vista_previa_ctrl_p"), &cmd_keys("preview", shortcuts)).clicked() {
            actions.push(Action::TogglePreview);
        }
        if expanded {
            return;
        }
        ui.add_space(GROUP_GAP);
        if narrow {
            let r = icon_btn(ui, t, ic::MAGNIFYING_GLASS, true, false, "");
            if design::tip_keys(r, t, t!("barra.buscar_archivos_ctrl_f"), &cmd_keys("search", shortcuts)).clicked() {
                tab.search.focus = true;
            }
        } else {
            search_box(ui, t, tab, search_id, search_w, &cfg.search_history, actions);
        }
    });
}

/// Subcarpetas que completan lo escrito en la barra de ruta (máximo 8).
fn path_suggestions(ctx: &egui::Context, typed: &str) -> Vec<String> {
    let typed = typed.trim().trim_matches('"');
    // Sin barra todavía: las unidades. Solo las letras (esto corre en cada cuadro: preguntarle a
    // cada unidad su nombre y espacio trabaría la ventana con una unidad de red caída).
    let Some(cut) = typed.rfind(['\\', '/']) else {
        let t = typed.to_lowercase();
        let mask = fsx::drive_mask();
        return (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| format!("{}:\\", (b'A' + i) as char)).filter(|d| t.is_empty() || d.to_lowercase().starts_with(&t)).collect();
    };
    let dir = &typed[..=cut];
    let prefix = typed[cut + 1..].to_lowercase();
    // El listado de la carpeta se recuerda mientras no cambie la carpeta escrita.
    let key = Id::new("path-sugg");
    let cached: Option<(String, std::sync::Arc<Vec<String>>)> = ctx.data(|d| d.get_temp(key));
    let names = match cached {
        Some((d, n)) if d.eq_ignore_ascii_case(dir) => n,
        _ if fsx::is_remote(dir) => {
            // En la red, listar puede tardar (o no contestar nunca): en otro hilo, y las
            // sugerencias aparecen cuando llegan.
            ctx.data_mut(|d| d.insert_temp(key, (dir.to_string(), std::sync::Arc::new(Vec::<String>::new()))));
            let (c, d) = (ctx.clone(), dir.to_string());
            std::thread::spawn(move || {
                let n: Vec<String> = fsx::list_dir(&d).map(|l| l.into_iter().filter(|e| e.is_dir && !e.hidden).map(|e| e.name).collect()).unwrap_or_default();
                c.data_mut(|m| {
                    // Solo si sigue escrita la misma carpeta.
                    let same = m.get_temp::<(String, std::sync::Arc<Vec<String>>)>(key).is_some_and(|(k, _)| k.eq_ignore_ascii_case(&d));
                    if same {
                        m.insert_temp(key, (d.clone(), std::sync::Arc::new(n)));
                    }
                });
                c.request_repaint();
            });
            std::sync::Arc::new(Vec::new())
        }
        _ => {
            let n: Vec<String> = fsx::list_dir(dir).map(|l| l.into_iter().filter(|e| e.is_dir && !e.hidden).map(|e| e.name).collect()).unwrap_or_default();
            let n = std::sync::Arc::new(n);
            ctx.data_mut(|d| d.insert_temp(key, (dir.to_string(), n.clone())));
            n
        }
    };
    let key_of = |s: &str| crate::index::store::search_key(s);
    let p = key_of(&prefix);
    let mut starts: Vec<&String> = names.iter().filter(|n| key_of(n).starts_with(&p)).collect();
    starts.sort_by_key(|n| n.to_lowercase());
    let mut contains: Vec<&String> = if p.is_empty() { Vec::new() } else { names.iter().filter(|n| !key_of(n).starts_with(&p) && key_of(n).contains(&p)).collect() };
    contains.sort_by_key(|n| n.to_lowercase());
    // Si lo escrito ya es exactamente una carpeta, no hace falta sugerirla sola.
    let all: Vec<String> = starts.into_iter().chain(contains).take(8).map(|n| fsx::join(dir.trim_end_matches(['\\', '/']), n)).collect();
    if all.len() == 1 && all[0].eq_ignore_ascii_case(typed.trim_end_matches('\\')) {
        return Vec::new();
    }
    all
}


/// Barra de ruta y búsqueda: campo de 28, relleno hundido, borde fuerte (con hover `faint`).
fn field_frame(p: &egui::Painter, t: &Theme, rect: Rect, focused: bool, hovered: bool) {
    p.rect_filled(rect, radius::M, t.sidebar);
    if focused {
        design::focus_ring(p, t, rect, radius::M);
    } else {
        p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, if hovered { t.faint } else { t.border_strong }), StrokeKind::Inside);
    }
}

/// Migas de la ruta: alto 22, 6 px de margen a cada lado; separador ▸ de 18.
const CRUMB_H: f32 = 22.0;
const CRUMB_PAD: f32 = 6.0;
const CRUMB_SEP_W: f32 = 18.0;
/// El "…" de los tramos que no entran.
const CRUMB_MORE_W: f32 = 24.0;
/// Lista de subcarpetas de un ▸ y de sugerencias: `LIST_W` por fuera (sin el margen ni el borde).
const LIST_INNER_W: f32 = design::LIST_W - 2.0 * sp::XS - 2.0 * stroke::HAIR;

/// `compact` (panel angosto): el último tramo va sin su ▸ (sus subcarpetas siguen en el menú del
/// tramo anterior) para dejarle el lugar al nombre.
fn breadcrumb(ui: &mut Ui, t: &Theme, tab: &mut Tab, width: f32, compact: bool, actions: &mut Vec<Action>) {
    let (rect, bg) = ui.allocate_exact_size(vec2(width, h::CTL), Sense::click());
    let p = ui.painter().clone();
    if let Some(buf) = tab.path_edit.as_mut() {
        field_frame(&p, t, rect, true, false);
        let id = Id::new(("pathedit", tab.id));
        // Sugerencias: subcarpetas de lo ya escrito que empiezan (o contienen) lo que sigue.
        let sugg = path_suggestions(ui.ctx(), buf);
        let sel_id = id.with("sel");
        let mut sel: Option<usize> = ui.data(|d| d.get_temp(sel_id)).flatten();
        if sel.is_some_and(|s| s >= sugg.len()) {
            sel = None;
        }
        let (down, up, tab_key) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                !sugg.is_empty() && i.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
            )
        });
        if down && !sugg.is_empty() {
            sel = Some(sel.map_or(0, |s| (s + 1).min(sugg.len() - 1)));
        }
        if up {
            sel = sel.and_then(|s| s.checked_sub(1));
        }
        // Tab completa con la sugerencia elegida (o la primera) y sigue escribiendo.
        let mut completed = false;
        if tab_key {
            let pick = &sugg[sel.unwrap_or(0)];
            *buf = format!("{}\\", pick.trim_end_matches('\\'));
            sel = None;
            completed = true;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(sp::S, 0.0))));
        let mut out = egui::TextEdit::singleline(buf)
            .id(id)
            .lock_focus(true)
            .frame(egui::Frame::NONE)
            .font(text::mono())
            .desired_width(rect.width() - 2.0 * sp::S)
            .vertical_align(egui::Align::Center)
            .show(&mut child);
        let r = out.response.clone();
        if completed {
            // Cursor al final tras completar.
            let n = buf.chars().count();
            out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
            out.state.clone().store(ui.ctx(), id);
        }
        if !r.has_focus() && !r.lost_focus() {
            r.request_focus();
            // Al abrirse, la ruta queda toda seleccionada (como en el Explorador): Ctrl+C la copia
            // y lo que se escriba la reemplaza.
            let n = buf.chars().count();
            out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
            out.state.store(ui.ctx(), id);
        }
        if r.changed() {
            sel = None;
        }
        ui.data_mut(|d| d.insert_temp(sel_id, sel));
        // Lista desplegable bajo la barra.
        let mut clicked = None;
        if !sugg.is_empty() {
            mark_overlay(ui.ctx());
            // De `LIST_W`, alineada con el borde izquierdo del campo; cada una con su nombre (la
            // ruta entera va al tooltip).
            egui::Area::new(id.with("sugg")).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, sp::XS)).show(ui.ctx(), |ui| {
                design::popover_frame(t).show(ui, |ui| {
                    ui.set_width(LIST_INNER_W);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (k, s) in sugg.iter().enumerate() {
                        let drive = s.len() <= 3;
                        let name = if drive { s.clone() } else { fsx::file_name(s) };
                        theme::want_cjk(&name);
                        let icon = if drive { ic::HARD_DRIVES } else { ic::FOLDER };
                        let r = design::menu_item_ex(ui, t, MenuItem { icon, label: &name, highlighted: Some(k) == sel, ..Default::default() });
                        if design::tip(r, t, s).clicked() {
                            clicked = Some(s.clone());
                        }
                    }
                });
            });
        }
        if let Some(target) = clicked {
            tab.path_edit = None;
            actions.push(Action::Navigate(target));
        } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let a = match sel {
                Some(s) => Action::Navigate(sugg[s].clone()),
                None => Action::PathInput(buf.trim().to_string()),
            };
            tab.path_edit = None;
            actions.push(a);
        } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) || (r.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Enter)) && !completed && clicked.is_none() && !ui.input(|i| i.pointer.any_down())) {
            tab.path_edit = None;
        }
        return;
    }
    field_frame(&p, t, rect, false, ui.rect_contains_pointer(rect));
    let path = tab.path().to_string();
    // Recientes, una etiqueta o una colección: un solo tramo con su ícono y su nombre (no la
    // carpeta desde la que se abrió).
    if let Some((glyph, dot, name)) = special_view(t, tab) {
        let cy = rect.center().y;
        let xi = rect.left() + sp::S + icon::M / 2.0;
        match dot {
            Some(c) => design::tag_dot(&p, t, pos2(xi, cy), c),
            None => {
                p.text(pos2(xi, cy), Align2::CENTER_CENTER, glyph, design::icon_font(icon::M), t.dim);
            }
        }
        let xt = rect.left() + sp::S + icon::M + sp::S;
        let g = elided(&p, &name, text::strong(), t.text, rect.right() - sp::S - xt);
        p.galley(pos2(xt, cy - g.size().y / 2.0), g, t.text);
        if bg.clicked() {
            tab.path_edit = Some(path);
        }
        design::tip_keys(bg, t, t!("barra.clic_escribir_ruta_comando_ctrl"), "Ctrl L");
        return;
    }
    // Segmentos de la ruta
    let mut segs: Vec<(String, String)> = Vec::new();
    let mut cur = Some(path.clone());
    while let Some(c) = cur {
        let label = crumb_label(&c);
        cur = fsx::parent_of(&c);
        segs.push((label, c));
    }
    segs.reverse();
    let n = segs.len();
    // La última miga va en `strong`.
    let font_of = |i: usize| if i + 1 == n { text::strong() } else { text::body() };
    let mut widths: Vec<f32> = segs.iter().enumerate().map(|(i, (l, _))| design::text_width(&p, l, font_of(i)) + 2.0 * CRUMB_PAD).collect();
    // Cada tramo lleva su ▸ (también el último: abre sus subcarpetas). Si no cabe, se ocultan los primeros.
    // Se reserva el lugar del "…" y de su ▸.
    let avail = rect.width() - 2.0 * sp::XS - CRUMB_MORE_W - CRUMB_SEP_W;
    let mut start = 0;
    let last_sep = if compact { 0.0 } else { CRUMB_SEP_W };
    let mut total: f32 = widths.iter().sum::<f32>() + CRUMB_SEP_W * (segs.len() - 1) as f32 + last_sep;
    while total > avail && start + 1 < segs.len() {
        total -= widths[start] + CRUMB_SEP_W;
        start += 1;
    }
    // Si ni el último tramo entra, su texto se recorta (no se corta a mitad de letra).
    let room_last = rect.width() - 2.0 * sp::XS - last_sep - if start > 0 { CRUMB_MORE_W + CRUMB_SEP_W } else { 0.0 };
    if let Some(w) = widths.last_mut() {
        *w = w.min(room_last.max(2.0 * CRUMB_PAD + sp::L));
    }
    let menu_id = Id::new(("crumb-menu", tab.id));
    let open_key = ui.data(|d| d.get_temp::<CrumbMenu>(menu_id)).map(|m| m.key);
    let crumb_top = rect.top() + (h::CTL - CRUMB_H) / 2.0;
    let mut x = rect.left() + sp::XS;
    if start > 0 {
        // "…": los tramos que no entran, en una lista; detrás, su ▸ como el de cualquier tramo
        // (el clic en él también abre la lista, así no cae en el modo de escribir la ruta).
        let r = Rect::from_min_size(pos2(x, crumb_top), vec2(CRUMB_MORE_W, CRUMB_H));
        let hit = Rect::from_x_y_ranges(x..=x + CRUMB_MORE_W + CRUMB_SEP_W, rect.y_range());
        let resp = design::tip(ui.interact(hit, menu_id.with("more"), Sense::click()), t, t!("barra.carpetas_anteriores"));
        let open = open_key == Some(usize::MAX);
        let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
        design::state_fill(&p, t, r, radius::S, hov, resp.is_pointer_button_down_on(), if open { Sel::Unfocused } else { Sel::None });
        p.text(r.center(), Align2::CENTER_CENTER, ic::DOTS_THREE, design::icon_font(icon::M), if resp.hovered() || open { t.text } else { t.faint });
        let cr = Rect::from_min_size(pos2(r.right(), crumb_top), vec2(CRUMB_SEP_W, CRUMB_H));
        p.text(cr.center(), Align2::CENTER_CENTER, ic::CARET_RIGHT, design::icon_font(icon::S), t.faint);
        if resp.clicked() {
            let items = segs[..start].iter().rev().cloned().collect();
            toggle_crumb_menu(ui, menu_id, CrumbMenu { key: usize::MAX, src: r, path: path.clone(), items: std::sync::Arc::new(items), current: None, scrolled: false });
        }
        x += CRUMB_MORE_W + CRUMB_SEP_W;
    }
    for (i, (label, full)) in segs.iter().enumerate().skip(start) {
        let w = widths[i];
        // Se dibuja con aire arriba y abajo, pero el clic vale en toda la altura: antes esos 3 px
        // abrían el modo de escribir la ruta.
        let r = Rect::from_min_size(pos2(x, crumb_top), vec2(w, CRUMB_H));
        let resp = ui.interact(Rect::from_x_y_ranges(r.x_range(), rect.y_range()), Id::new(("crumb", tab.id, i)), Sense::click_and_drag());
        // Soltar archivos sobre un tramo los lleva a esa carpeta (como en el Explorador de antes).
        if crate::ui::views::drag_hovering(&resp) {
            p.rect_filled(r, radius::S, t.accent_soft);
            design::focus_ring(&p, t, r, radius::S);
        } else {
            let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
            design::state_fill(&p, t, r, radius::S, hov, resp.is_pointer_button_down_on(), Sel::None);
        }
        let last = i + 1 == n;
        theme::want_cjk(label);
        let color = if last { t.text } else { t.dim };
        let g = elided(&p, label, font_of(i), color, w - 2.0 * CRUMB_PAD);
        p.galley(r.center() - g.size() / 2.0, g, color);
        if let Some(files) = crate::ui::views::take_drop(&resp) {
            let paths: Vec<String> = files.into_iter().filter(|f| droppable_into(f, full)).collect();
            if !paths.is_empty() {
                actions.push(Action::DropFiles { paths, dest: full.clone(), force: crate::ui::views::drop_force() });
            }
        }
        // Arrastrar un tramo lleva esa carpeta: a otra app, al otro panel o a otra carpeta.
        if resp.drag_started() && full.len() > 3 {
            resp.dnd_set_drag_payload(crate::ui::views::DragFiles(vec![full.clone()]));
        }
        if resp.clicked() {
            actions.push(Action::Navigate(full.clone()));
        }
        if resp.middle_clicked() {
            actions.push(Action::NewTab(Some(full.clone())));
        }
        if last {
            design::tip(resp, t, t!("barra.arrastra_llevar_esta_carpeta_otro"));
        }
        x += w;
        if last && compact {
            break;
        }
        // ▸: las subcarpetas de este tramo, con la siguiente marcada.
        let cr = Rect::from_min_size(pos2(x, crumb_top), vec2(CRUMB_SEP_W, CRUMB_H));
        let cresp = ui.interact(Rect::from_x_y_ranges(cr.x_range(), rect.y_range()), Id::new(("crumb-sep", tab.id, i)), Sense::click());
        let open_here = open_key == Some(i);
        let hov = design::hover_amount(ui.ctx(), cresp.id, cresp.hovered());
        design::state_fill(&p, t, cr, radius::S, hov, cresp.is_pointer_button_down_on(), if open_here { Sel::Unfocused } else { Sel::None });
        let caret = if open_here { ic::CARET_DOWN } else { ic::CARET_RIGHT };
        p.text(cr.center(), Align2::CENTER_CENTER, caret, design::icon_font(icon::S), if cresp.hovered() || open_here { t.dim } else { t.faint });
        if cresp.clicked() {
            let current = segs.get(i + 1).map(|(_, f)| f.clone());
            toggle_crumb_menu(ui, menu_id, CrumbMenu { key: i, src: cr, path: path.clone(), items: std::sync::Arc::new(subfolders(full)), current, scrolled: false });
        }
        x += CRUMB_SEP_W;
    }
    if bg.clicked() {
        tab.path_edit = Some(path.clone());
    }
    design::tip_keys(bg, t, t!("barra.clic_escribir_ruta_comando_ctrl"), "Ctrl L");
    crumb_menu(ui, t, menu_id, &path, actions);
}

/// Pestaña de recientes, de una etiqueta o de una colección: (ícono, punto de la etiqueta, nombre).
fn special_view(t: &Theme, tab: &Tab) -> Option<(&'static str, Option<Color32>, String)> {
    if tab.tool.is_some() {
        return None;
    }
    if tab.search.text.trim_start().to_lowercase().starts_with("recientes:") {
        Some((ic::CLOCK_COUNTER_CLOCKWISE, None, t!("barra.recientes").to_string()))
    } else if let Some(c) = fsx::tags::query_color(&tab.search.text) {
        Some((ic::TAG, Some(t.tags[c.min(6)]), crate::ui::views::tag_label(c).to_string()))
    } else {
        fsx::collections::query_name(&tab.search.text).map(|n| (ic::BOOKMARKS_SIMPLE, None, n))
    }
}

fn crumb_label(path: &str) -> String {
    if path.len() <= 3 {
        path.trim_end_matches('\\').to_string()
    } else if path.starts_with(r"\\") && fsx::parent_of(path).is_none() {
        // Raíz de red: "servidor\recurso".
        path.trim_start_matches('\\').trim_end_matches('\\').to_string()
    } else {
        fsx::file_name(path)
    }
}

/// ¿Tiene sentido soltar `src` en `dest`? No sobre sí misma, ni donde ya está, ni dentro de sí misma.
fn droppable_into(src: &str, dest: &str) -> bool {
    let (s, d) = (src.to_lowercase(), dest.to_lowercase());
    let s = s.trim_end_matches('\\');
    s != d.trim_end_matches('\\') && fsx::parent_of(src).is_none_or(|p| !p.eq_ignore_ascii_case(dest)) && !d.starts_with(&format!("{s}\\"))
}

/// Subcarpetas visibles de `dir`: (nombre, ruta).
fn subfolders(dir: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = fsx::list_dir(dir).map(|l| l.into_iter().filter(|e| e.is_dir && !e.hidden).map(|e| (e.name.clone(), fsx::join(dir, &e.name))).collect()).unwrap_or_default();
    v.sort_by(|a, b| fsx::natural_cmp(&a.0, &b.0));
    v
}

/// Lista desplegable de un ▸ de la barra de ruta.
#[derive(Clone)]
struct CrumbMenu {
    /// Qué ▸ la abrió (usize::MAX = el "…").
    key: usize,
    /// Dónde está ese ▸: un clic ahí la cierra (no la vuelve a abrir).
    src: Rect,
    /// Carpeta de la pestaña al abrirse: si cambia, se cierra.
    path: String,
    items: std::sync::Arc<Vec<(String, String)>>,
    /// La carpeta que se marca (por donde va la ruta).
    current: Option<String>,
    scrolled: bool,
}

fn toggle_crumb_menu(ui: &Ui, id: Id, m: CrumbMenu) {
    ui.data_mut(|d| {
        if d.get_temp::<CrumbMenu>(id).is_some_and(|o| o.key == m.key) {
            d.remove::<CrumbMenu>(id);
        } else {
            d.insert_temp(id, m);
        }
    });
}

/// ¿Hay una lista de la barra de ruta o de la búsqueda abierta (en este cuadro o el
/// anterior)? El visor de Windows no debe taparla y sus teclas son de ella.
pub fn overlay_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(Id::new("chrome-overlay"))).is_some_and(|p| p + 1 >= ctx.cumulative_pass_nr())
}

fn mark_overlay(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new("chrome-overlay"), pass));
}

fn crumb_menu(ui: &mut Ui, t: &Theme, id: Id, path: &str, actions: &mut Vec<Action>) {
    let Some(mut m) = ui.data(|d| d.get_temp::<CrumbMenu>(id)) else { return };
    if m.path != path {
        ui.data_mut(|d| d.remove::<CrumbMenu>(id));
        return;
    }
    mark_overlay(ui.ctx());
    let mut close = false;
    // Cada lista con su propio id: si no, la nueva hereda el tamaño de la anterior.
    let area = egui::Area::new(id.with(("area", m.key, m.items.len()))).order(egui::Order::Foreground).fixed_pos(m.src.left_bottom() + vec2(-sp::XS, sp::XS)).show(ui.ctx(), |ui| {
        design::popover_frame(t).show(ui, |ui| {
            ui.set_width(LIST_INNER_W);
            ui.spacing_mut().item_spacing.y = 0.0;
            if m.items.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(LIST_INNER_W, h::ROW), Sense::hover());
                ui.painter().text(pos2(r.left() + sp::S, r.center().y), Align2::LEFT_CENTER, t!("barra.sin_subcarpetas"), text::small(), t.faint);
                return;
            }
            egui::ScrollArea::vertical().id_salt(("crumb-scroll", m.key)).max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (name, full) in m.items.iter() {
                    let is_cur = m.current.as_ref().is_some_and(|c| c.eq_ignore_ascii_case(full));
                    let icon = if full.len() <= 3 { ic::HARD_DRIVES } else { ic::FOLDER };
                    theme::want_cjk(name);
                    let resp = design::menu_item_ex(ui, t, MenuItem { icon, label: name, current: is_cur, ..Default::default() });
                    if is_cur && !m.scrolled {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                    if resp.clicked() {
                        actions.push(Action::Navigate(full.clone()));
                        close = true;
                    }
                    if resp.middle_clicked() {
                        actions.push(Action::NewTab(Some(full.clone())));
                        close = true;
                    }
                }
            });
        });
    });
    m.scrolled = true;
    // Se cierra con Esc o con un clic fuera (salvo en su propio ▸, que la cierra al soltar).
    let pressed_outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !m.src.contains(p)));
    let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    ui.data_mut(|d| {
        if close || pressed_outside || esc {
            d.remove::<CrumbMenu>(id);
        } else {
            d.insert_temp(id, m);
        }
    });
}

/// Ancho mínimo de la búsqueda para mostrar la tecla `Ctrl F` en línea.
const SEARCH_KBD_MIN_W: f32 = 180.0;
const SEARCH_KEYS: &str = "Ctrl F";

fn search_box(ui: &mut Ui, t: &Theme, tab: &mut Tab, id: Id, width: f32, history: &[String], actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width() - TOOLBAR_EDGE).max(120.0), h::CTL), Sense::hover());
    let p = ui.painter().clone();
    let focused = ui.memory(|m| m.has_focus(id));
    field_frame(&p, t, rect, focused, ui.rect_contains_pointer(rect));
    p.text(pos2(rect.left() + sp::S + icon::M / 2.0, rect.center().y), Align2::CENTER_CENTER, ic::MAGNIFYING_GLASS, design::icon_font(icon::M), t.dim);
    // A la derecha, en este orden: X (si hay texto), alcance y la tecla (vacía, sin foco y ancha).
    let mut right = rect.right() - sp::XS;
    let show_kbd = tab.search.text.is_empty() && !focused && rect.width() >= SEARCH_KBD_MIN_W;
    if show_kbd {
        // Mismo ancho que dibuja `kbd_paint` (texto + 10).
        let kw = design::text_width(&p, SEARCH_KEYS, text::mono_s()) + sp::S + sp::XXS;
        design::kbd_paint(&p, t, pos2(right - kw, rect.center().y), SEARCH_KEYS);
        right -= kw + sp::XS;
    }
    // Chip de alcance: todo el equipo / esta carpeta.
    let chip_txt = if tab.search.here { t!("barra.aqui") } else { t!("barra.todo") };
    let chip = Chip { text: chip_txt, tone: if tab.search.here { Tone::Accent } else { Tone::Neutral }, ..Default::default() };
    let cw = design::chip_width(&p, &chip);
    let chip_rect = Rect::from_min_size(pos2(right - cw, rect.center().y - h::CHIP / 2.0), vec2(cw, h::CHIP));
    let chip_resp = design::tip(ui.interact(chip_rect, id.with("chip"), Sense::click()), t, t!("barra.buscar_todo_equipo_solo_carpeta"));
    design::chip_paint(&p, t, chip_rect.left_center(), &chip, chip_resp.hovered());
    if chip_resp.clicked() {
        tab.search.here ^= true;
    }
    right = chip_rect.left() - sp::XXS;
    // X para vaciar.
    if !tab.search.text.is_empty() {
        let xr = Rect::from_min_size(pos2(right - h::CHIP, rect.center().y - h::CHIP / 2.0), vec2(h::CHIP, h::CHIP));
        if design::icon_button_at(ui, t, xr, id.with("clear"), ic::X, IconBtn::mini()).clicked() {
            tab.search.text.clear();
            ui.memory_mut(|m| m.request_focus(id));
        }
        right = xr.left() - sp::XXS;
    }
    let edit_rect = Rect::from_min_max(pos2(rect.left() + sp::S + icon::M + sp::S, rect.top()), pos2(right, rect.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let hist_shown = tab.search.text.is_empty() && !history.is_empty();
    // Enter con una búsqueda elegida: se toma antes de que la caja lo use (y pierda el foco).
    let hist_enter = hist_shown && ui.data(|d| d.get_temp::<Option<usize>>(id.with("hist-sel"))).flatten().is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    // Solo «Buscar»: el atajo va como tecla en línea.
    let hint = t!("barra.buscar_archivos_ctrl_f");
    let r = child.add(
        egui::TextEdit::singleline(&mut tab.search.text)
            .id(id)
            // Con el historial a la vista, ↑ ↓ son de la lista (no pasan el foco a otro control).
            .lock_focus(hist_shown)
            .frame(egui::Frame::NONE)
            .font(text::body())
            .hint_text(egui::RichText::new(hint).color(t.faint))
            .desired_width(edit_rect.width())
            .vertical_align(egui::Align::Center),
    );
    if tab.search.focus {
        r.request_focus();
        tab.search.focus = false;
    }
    search_history(ui, t, tab, id, rect, r.has_focus() || hist_enter, hist_enter, history, actions);
}

/// Búsquedas recientes: con la caja vacía y enfocada, una lista para repetirlas (↑ ↓ Enter).
#[allow(clippy::too_many_arguments)]
fn search_history(ui: &mut Ui, t: &Theme, tab: &mut Tab, id: Id, rect: Rect, focused: bool, enter: bool, history: &[String], actions: &mut Vec<Action>) {
    let area_id = id.with("hist");
    let rect_id = id.with("hist-rect");
    let sel_id = id.with("hist-sel");
    // Al apretar sobre la lista la caja pierde el foco: la lista sigue mientras el botón está
    // apretado y también en el cuadro en que se suelta, que es cuando egui cuenta el clic.
    let last_rect: Option<Rect> = ui.data(|d| d.get_temp(rect_id));
    let pressing_list = ui.input(|i| (i.pointer.any_down() || i.pointer.any_released()) && i.pointer.interact_pos().is_some_and(|p| last_rect.is_some_and(|r| r.contains(p))));
    if !tab.search.text.is_empty() || history.is_empty() || !(focused || pressing_list) {
        ui.data_mut(|d| {
            d.remove::<Rect>(rect_id);
            d.remove::<Option<usize>>(sel_id);
        });
        return;
    }
    mark_overlay(ui.ctx());
    let shown = history.len().min(10);
    // La lista pudo achicarse (se olvidó una búsqueda): la fila elegida tiene que seguir existiendo.
    let mut sel: Option<usize> = ui.data(|d| d.get_temp(sel_id)).flatten().filter(|&s| s < shown);
    let (down, up) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)));
    if down {
        sel = Some(sel.map_or(0, |s| (s + 1).min(shown - 1)));
    }
    if up {
        sel = sel.and_then(|s| s.checked_sub(1));
    }
    let mut pick: Option<String> = None;
    if enter {
        pick = sel.map(|s| history[s].clone());
    }
    let w = rect.width().max(design::LIST_W);
    let area = egui::Area::new(area_id).order(egui::Order::Foreground).fixed_pos(pos2(rect.right() - w, rect.bottom() + sp::XS)).show(ui.ctx(), |ui| {
        design::popover_frame(t).show(ui, |ui| {
            ui.set_width(w - 2.0 * sp::XS);
            ui.spacing_mut().item_spacing.y = 0.0;
            design::menu_title(ui, t, t!("barra.busquedas_recientes"));
            for (k, q) in history.iter().take(shown).enumerate() {
                theme::want_cjk(q);
                // Los espacios del atajo reservan el lugar de la X (el texto no pasa por debajo).
                let resp = design::menu_item_ex(ui, t, MenuItem { icon: ic::CLOCK_COUNTER_CLOCKWISE, label: q, keys: "   ", highlighted: Some(k) == sel, ..Default::default() });
                let rr = resp.rect;
                let hovering = ui.rect_contains_pointer(rr);
                // Quitar esta búsqueda del historial.
                let xr = Rect::from_center_size(pos2(rr.right() - sp::XS - h::CHIP / 2.0, rr.center().y), vec2(h::CHIP, h::CHIP));
                let xresp = mini_btn(ui, t, xr, area_id.with(("x", k)), ic::X, hovering || Some(k) == sel);
                if xresp.clicked() {
                    actions.push(Action::ForgetSearch(Some(k)));
                    // La lista queda abierta (la caja recupera el foco) para seguir quitando.
                    ui.memory_mut(|m| m.request_focus(id));
                } else if resp.clicked() {
                    pick = Some(q.clone());
                }
            }
            design::menu_sep(ui, t);
            if design::menu_item(ui, t, ic::TRASH, t!("barra.borrar_historial"), "").clicked() {
                actions.push(Action::ForgetSearch(None));
            }
        });
    });
    if let Some(q) = pick {
        let n = q.chars().count();
        tab.search.text = q;
        sel = None;
        // Seguir escribiendo al final.
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
            st.store(ui.ctx(), id);
        }
        ui.memory_mut(|m| m.request_focus(id));
    }
    let r = area.response.rect;
    ui.data_mut(|d| {
        d.insert_temp(rect_id, r);
        d.insert_temp(sel_id, sel);
    });
}

// ---------------------------------------------------------------- barra de estado

/// Texto de la barra de estado en el flujo de `ui` (mono chica, tal cual, sin mayúsculas).
fn status_text(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let w = design::text_width(ui.painter(), text, text::mono_s());
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
    design::mono_label(ui.painter(), rect.left_center(), Align2::LEFT_CENTER, text, color);
    resp
}

/// Texto de la barra de estado con un ícono delante que se puede apretar.
fn status_link(ui: &mut Ui, t: &Theme, glyph: &str, label: &str) -> Response {
    let tw = design::text_width(ui.painter(), label, text::mono_s());
    let w = icon::S + sp::XS + tw;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::click());
    let c = if resp.hovered() { t.accent_hover } else { t.accent };
    let p = ui.painter();
    p.text(pos2(rect.left() + icon::S / 2.0, rect.center().y), Align2::CENTER_CENTER, glyph, design::icon_fill(icon::S), c);
    design::mono_label(p, pos2(rect.left() + icon::S + sp::XS, rect.center().y), Align2::LEFT_CENTER, label, c);
    resp
}

/// `mode`: cómo se mantiene al día el índice ("USN (servicio)", "vigilancia"…); el detalle
/// (servicio / administrador) va en el tooltip, no en el texto.
pub fn status_bar(ui: &mut Ui, t: &Theme, left: &str, right: &str, busy: bool, mode: &str, actions: &mut Vec<Action>) {
    ui.horizontal_centered(|ui| {
        ui.add_space(sp::M);
        status_text(ui, left, t.dim);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(sp::M);
            let usn = mode.starts_with("USN");
            let c = if busy || usn { t.accent } else { t.faint };
            let shown = status_text(ui, right, if busy { t.accent } else { t.faint });
            let label = ui.interact(shown.rect, Id::new("status-index"), Sense::click());
            ui.label(egui::RichText::new(egui_phosphor::fill::LIGHTNING).font(design::icon_fill(icon::S)).color(c));
            if mode == crate::index::MODE_WATCH {
                // Sin servicio: ofrecer activarlo (una vez, con permiso de administrador).
                ui.add_space(sp::S);
                let b = design::tip(status_link(ui, t, egui_phosphor::fill::LIGHTNING, t!("barra.activar_indice_instantaneo")), t, t!("barra.instala_pequeno_servicio_windows_lee"));
                if b.clicked() {
                    actions.push(Action::InstallService);
                }
            }
            if usn {
                // El modo completo («USN (servicio)») va aquí; en la barra, solo «USN».
                let label = design::tip(label, t, &format!("{}\n{}", crate::index::mode_label(mode), t!("barra.indice_mantiene_dia_usn_journal")));
                design::context_menu(&label, t, |ui| {
                    if mode == crate::index::MODE_SERVICE && design::menu_item_ex(ui, t, MenuItem { icon: ic::TRASH, label: t!("barra.quitar_servicio_indexado"), danger: true, ..Default::default() }).clicked() {
                        actions.push(Action::UninstallService);
                    }
                });
            }
        });
    });
}

// ---------------------------------------------------------------- cola de copias

fn fmt_eta(secs: u64) -> String {
    match secs {
        0..60 => tf!("barra.eta_segundos", segundos = secs),
        60..3600 => tf!("barra.eta_minutos", minutos = secs / 60),
        _ => tf!("barra.eta_horas", horas = secs / 3600, minutos = (secs % 3600) / 60),
    }
}

fn job_line(j: &transfer::JobView) -> String {
    use transfer::State;
    match j.state {
        State::Waiting => t!("barra.espera").into(),
        State::Scanning => t!("barra.preparando").into(),
        State::Conflict => t!("barra.esperando_tu_decision").into(),
        State::Paused => tf!("barra.pausa", hecho = fsx::human_size(j.done_bytes), total = fsx::human_size(j.total_bytes)),
        State::Running => {
            let mut s = tf!("barra.progreso_copia", hecho = fsx::human_size(j.done_bytes), total = fsx::human_size(j.total_bytes), velocidad = fsx::human_size(j.bytes_per_sec as u64));
            if j.counting {
                s.push_str(t!("barra.contando"));
            } else if let Some(e) = j.eta_secs() {
                s.push_str(&tf!("barra.faltan", tiempo = fmt_eta(e)));
            }
            // «N a la vez» va al tooltip de la fila: en la línea la cortaba.
            s
        }
        State::Done if j.errors.is_empty() => tp!(j.done_files, "barra.listo_archivos", n = fsx::thousands(j.done_files)),
        State::Done => tp!(j.errors.len(), "barra.terminado_errores"),
        State::Canceled => t!("barra.cancelado").into(),
    }
}

/// Dónde quedó el chip de copias (la lista se abre sobre él).
const XFER_CHIP_RECT: &str = "xfer-chip-rect";

/// Barra compacta en la barra de estado: nombre a la izquierda, porcentaje a la derecha y el
/// medidor de puntos debajo, a 1 px del fondo del chip. Devuelve true si se le hizo clic.
pub fn transfer_chip(ui: &mut Ui, t: &Theme, area: Rect, jobs: &[transfer::JobView]) -> bool {
    ui.ctx().data_mut(|d| d.insert_temp(Id::new(XFER_CHIP_RECT), area));
    let resp = design::tip(ui.interact(area, Id::new("xfer-chip"), Sense::click()), t, t!("barra.ver_copias"));
    let p = ui.painter();
    let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
    design::state_fill(p, t, area, radius::M, hov, resp.is_pointer_button_down_on(), Sel::None);
    let running: Vec<&transfer::JobView> = jobs.iter().filter(|j| !j.finished()).collect();
    let (label, pct, frac, color) = match running.first() {
        Some(j) => {
            let extra = if running.len() > 1 { format!(" (+{})", running.len() - 1) } else { String::new() };
            let pct = format!("{}%", (j.fraction() * 100.0) as u32);
            (format!("{}{extra}", j.label), pct, j.fraction(), t.accent)
        }
        None => {
            let errs: usize = jobs.iter().map(|j| j.errors.len()).sum();
            let txt = if errs > 0 { tf!("barra.copias_terminadas_error", errores = errs) } else { t!("barra.copias_terminadas").to_string() };
            (txt, String::new(), 1.0, if errs > 0 { t.danger } else { t.dim })
        }
    };
    let inner = area.shrink2(vec2(sp::S, 0.0));
    // Pegado al fondo: a 4 px el descendente del texto lo tocaba.
    let meter = Rect::from_min_max(pos2(inner.left(), area.bottom() - 4.0), pos2(inner.right(), area.bottom() - 1.0));
    // El texto, centrado en lo que queda sobre el medidor.
    let cy = (area.top() + meter.top()) / 2.0;
    let pw = if pct.is_empty() { 0.0 } else { p.text(pos2(inner.right(), cy), Align2::RIGHT_CENTER, &pct, text::mono_s(), t.dim).width() + sp::S };
    let g = elided(p, &label, text::small(), t.text, inner.width() - pw);
    p.galley(pos2(inner.left(), cy - g.size().y / 2.0), g, t.text);
    design::dot_meter(p, t, meter, frac, color);
    resp.clicked()
}

/// Botón fantasma de 20 (cabecera de la lista de copias).
fn ghost_small(ui: &mut Ui, t: &Theme, label: &str) -> Response {
    let w = design::text_width(ui.painter(), label, text::small()) + 2.0 * sp::S;
    let (rect, r) = ui.allocate_exact_size(vec2(w, h::CHIP), Sense::click());
    let hov = design::hover_amount(ui.ctx(), r.id, r.hovered());
    design::state_fill(ui.painter(), t, rect, radius::S, hov, r.is_pointer_button_down_on(), Sel::None);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, text::small(), if r.hovered() { t.text } else { t.dim });
    r
}

/// Bajo una fila de la lista de copias, alineado con su texto (ícono en x = 4 + 8).
const JOB_TEXT_X: f32 = sp::XS + sp::S + icon::M + sp::S;

fn job_indent(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(JOB_TEXT_X);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(ui.available_width() - sp::M);
            add(ui);
        });
    });
}

/// Lista de copias (sobre la barra de estado): popover de `PANEL_W`, cada trabajo una fila de dos
/// líneas con su medidor debajo. Devuelve false si se cerró.
pub fn transfers_popup(ctx: &egui::Context, t: &Theme, jobs: &[transfer::JobView], actions: &mut Vec<Action>) -> bool {
    use transfer::State;
    let mut open = true;
    // Centrada sobre su chip (no sobre la ventana); sin chip, sobre la barra de estado.
    let area = egui::Area::new(Id::new("xfer-popup")).order(egui::Order::Foreground);
    let area = match ctx.data(|d| d.get_temp::<Rect>(Id::new(XFER_CHIP_RECT))) {
        Some(chip) => area.pivot(Align2::CENTER_BOTTOM).fixed_pos(chip.center_top() - vec2(0.0, sp::S)),
        None => area.anchor(Align2::CENTER_BOTTOM, vec2(0.0, -(h::HEAD + sp::S))),
    };
    area.show(ctx, |ui| {
        design::popover_frame(t).show(ui, |ui| {
            ui.set_width(design::PANEL_W - 2.0 * sp::XS - 2.0 * stroke::HAIR);
            ui.spacing_mut().item_spacing.y = 0.0;
            // Cabecera: micro-etiqueta, «Limpiar terminadas» y cerrar.
            let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
            design::micro(ui.painter(), pos2(hr.left() + sp::S, hr.center().y), Align2::LEFT_CENTER, t!("barra.copias"), t.faint);
            let mut head = ui.new_child(egui::UiBuilder::new().max_rect(hr.shrink2(vec2(sp::XS, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
            head.spacing_mut().item_spacing.x = sp::XS;
            let close = design::icon_button(&mut head, t, ic::X, IconBtn::mini());
            if design::tip(close, t, t!("comun.cerrar")).clicked() {
                open = false;
            }
            if jobs.iter().any(|j| j.finished()) && ghost_small(&mut head, t, t!("barra.limpiar_terminadas")).clicked() {
                actions.push(Action::TransferClear);
            }
            for j in jobs.iter().rev() {
                design::menu_sep(ui, t);
                let icon = match j.op {
                    transfer::Op::Copy => ic::COPY,
                    transfer::Op::Move => ic::ARROW_RIGHT,
                    transfer::Op::Extract | transfer::Op::Compress => ic::FILE_ZIP,
                    transfer::Op::Delete => ic::TRASH,
                };
                let line = job_line(j);
                theme::want_cjk(&j.label);
                let spec = design::Row2 {
                    icon,
                    // Acento solo mientras corre (no en espera ni en pausa).
                    icon_color: Some(if j.state == State::Running { t.accent } else { t.dim }),
                    title: &j.label,
                    sub: &line,
                    sub_mono: true,
                    sub_color: (j.state == State::Conflict).then_some(t.warn),
                    ..Default::default()
                };
                let row = design::row2(ui, t, Id::new(("xfer-job", j.id)), spec, |ui| {
                    if j.finished() {
                        return;
                    }
                    ui.spacing_mut().item_spacing.x = sp::XXS;
                    if design::tip(design::icon_button(ui, t, ic::X, IconBtn::mini()), t, t!("comun.cancelar")).clicked() {
                        actions.push(Action::TransferCancel(j.id));
                    }
                    if j.state != State::Conflict {
                        let paused = j.state == State::Paused;
                        let (glyph, tip) = if paused { (ic::PLAY, t!("barra.continuar")) } else { (ic::PAUSE, t!("barra.pausar")) };
                        if design::tip(design::icon_button(ui, t, glyph, IconBtn::mini()), t, tip).clicked() {
                            actions.push(Action::TransferPause(j.id, !paused));
                        }
                    }
                });
                // La línea entera, con cuántos archivos van a la vez (en la fila no entra).
                if j.state == State::Running && j.workers > 1 {
                    design::tip(row, t, &format!("{line}{}", tf!("barra.a_la_vez", n = j.workers)));
                }
                // Bajo la fila: medidor, archivo en curso y errores.
                if !j.finished() {
                    job_indent(ui, |ui| {
                        let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), sp::S), Sense::hover());
                        let c = if j.state == State::Paused { t.accent.gamma_multiply(0.5) } else { t.accent };
                        design::dot_meter(ui.painter(), t, mr, j.fraction(), c);
                        if !j.current.is_empty() {
                            let w = ui.available_width();
                            let cur = design::elide_middle(ui.painter(), &j.current, text::mono_s(), w);
                            let (cr, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
                            design::mono_label(ui.painter(), cr.left_center(), Align2::LEFT_CENTER, &cur, t.faint);
                        }
                    });
                }
                if !j.errors.is_empty() {
                    job_indent(ui, |ui| {
                        for e in j.errors.iter().take(3) {
                            design::status_line(ui, t, Tone::Danger, "", e);
                        }
                    });
                }
                ui.add_space(sp::XS);
            }
        });
    });
    open
}

/// Un archivo ya existe en el destino: reemplazar, omitir o conservar ambos.
pub fn conflict_dialog(ctx: &egui::Context, t: &Theme, j: &transfer::JobView, actions: &mut Vec<Action>) {
    let Some(c) = &j.conflict else { return };
    let all_id = Id::new(("conflict-all", j.id));
    let mut all = ctx.data_mut(|d| *d.get_temp_mut_or(all_id, false));
    design::dialog(ctx, t, ("conflict", j.id), DialogW::M, |ui| {
        let name = fsx::file_name(&c.dst);
        theme::want_cjk(&name);
        design::dialog_header(ui, t, ic::COPY, Tone::Warn, t!("barra.este_archivo_ya_existe_destino"), Some(&name));
        ui.add_space(sp::L);
        // Comparación: rótulo (88), «Nuevo» y «El que ya está»; valores en mono.
        design::card_frame(t).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 3.0 * h::HEAD), Sense::hover());
            let p = ui.painter();
            let key_w = 88.0;
            let col_w = (rect.width() - key_w) / 2.3;
            let x1 = rect.left() + key_w;
            let x2 = x1 + 1.3 * col_w;
            let cy = |row: f32| rect.top() + (row + 0.5) * h::HEAD;
            design::micro(p, pos2(x1, cy(0.0)), Align2::LEFT_CENTER, t!("comun.nuevo"), t.faint);
            design::micro(p, pos2(x2, cy(0.0)), Align2::LEFT_CENTER, t!("barra.ya_esta"), t.faint);
            for (row, key) in [(1.0, t!("comun.tamano")), (2.0, t!("comun.modificado"))] {
                p.text(pos2(rect.left(), cy(row)), Align2::LEFT_CENTER, key, text::small(), t.dim);
            }
            let val = |x: f32, row: f32, s: &str| p.text(pos2(x, cy(row)), Align2::LEFT_CENTER, s, text::mono(), t.text);
            val(x1, 1.0, &fsx::human_size(c.src_size));
            val(x2, 1.0, &fsx::human_size(c.dst_size));
            // «más nuevo» como chip junto a la fecha más reciente (el texto traducido trae « (…)»).
            let newer = t!("barra.mas_nuevo").trim().trim_start_matches('(').trim_end_matches(')').trim();
            let chip = Chip { text: newer, tone: Tone::Accent, mono: true, compact: true, ..Default::default() };
            for (x, mine, other) in [(x1, c.src_mtime, c.dst_mtime), (x2, c.dst_mtime, c.src_mtime)] {
                let r = val(x, 2.0, &fsx::fmt_date(mine));
                if mine > other {
                    design::chip_paint(p, t, pos2(r.right() + sp::S, cy(2.0)), &chip, false);
                }
            }
        });
        ui.add_space(sp::S);
        design::checkbox(ui, t, &mut all, t!("barra.hacer_mismo_siguientes"));
        let mut cancel = false;
        let mut pick = None;
        design::dialog_footer(
            ui,
            |ui| cancel = design::button(ui, t, Kind::Ghost, "", t!("barra.cancelar_copia"), true).clicked(),
            |ui| {
                // De derecha a izquierda: «Reemplazar» (primario), «Conservar ambos», «Omitir».
                let answers = [
                    (Kind::Primary, t!("barra.reemplazar"), transfer::Choice::Replace),
                    (Kind::Secondary, t!("barra.conservar_ambos"), transfer::Choice::KeepBoth),
                    (Kind::Secondary, t!("barra.omitir"), transfer::Choice::Skip),
                ];
                for (kind, label, choice) in answers {
                    if design::button(ui, t, kind, "", label, true).clicked() {
                        pick = Some(choice);
                    }
                }
            },
        );
        if cancel {
            actions.push(Action::TransferCancel(j.id));
        }
        if let Some(choice) = pick {
            actions.push(Action::TransferResolve(j.id, choice, all));
        }
    });
    ctx.data_mut(|d| d.insert_temp(all_id, all));
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    #[test]
    fn visible_order_groups_tabs_where_the_first_one_is() {
        // Panel 0: a, b(g1), c, d(g1), e(g2); panel 1: f, g(g1).
        let tabs = [(0, 0), (0, 1), (0, 0), (0, 1), (0, 2), (1, 0), (1, 1)];
        assert_eq!(visible_order(&tabs), vec![0, 1, 3, 2, 4, 5, 6]);
        // Ya ordenado: queda igual.
        let sorted: Vec<(u8, u64)> = visible_order(&tabs).iter().map(|&i| tabs[i]).collect();
        assert_eq!(visible_order(&sorted), (0..sorted.len()).collect::<Vec<_>>());
        // Paneles mezclados en el vector: primero el 0.
        assert_eq!(visible_order(&[(1, 0), (0, 0), (2, 5), (0, 5), (2, 0)]), vec![1, 3, 0, 2, 4]);
    }

    #[test]
    fn drop_index_counts_centers_left_of_the_edge() {
        let centers = [50.0, 150.0, 250.0];
        assert_eq!(drop_index(&centers, 0.0), 0);
        assert_eq!(drop_index(&centers, 149.0), 1);
        assert_eq!(drop_index(&centers, 151.0), 2);
        assert_eq!(drop_index(&centers, 999.0), 3);
    }

    fn row(kinds: &[(DropKind, f32)]) -> Vec<(DropKind, f32, f32)> {
        let mut x = 0.0;
        kinds
            .iter()
            .map(|&(k, w)| {
                x += w;
                (k, x - w, x)
            })
            .collect()
    }

    use DropKind::{Chip, Tab as T};

    #[test]
    fn drop_target_joins_while_inside_the_group_span() {
        // [a] [● G] [g1] [g2] [b]   (anchos distintos)
        let items = row(&[(T { group: 0 }, 100.0), (Chip { group: 7, collapsed: false }, 60.0), (T { group: 7 }, 120.0), (T { group: 7 }, 80.0), (T { group: 0 }, 100.0)]);
        // Antes de todo: sin grupo.
        assert_eq!(drop_target(&items, 10.0, 60.0), (0, 0, None));
        // Entre a y el chip: B es un chip → ninguno.
        assert_eq!(drop_target(&items, 120.0, 170.0), (1, 0, None));
        // Entre el chip y g1: B es de G.
        assert_eq!(drop_target(&items, 200.0, 250.0), (2, 7, None));
        // Después de g2 (fin del grupo) con el borde izquierdo todavía dentro de g2: G.
        assert_eq!(drop_target(&items, 345.0, 400.0), (4, 7, None));
        // Ya fuera del tramo (pasó el borde derecho de g2 = 360): ninguno.
        assert_eq!(drop_target(&items, 362.0, 400.0), (4, 0, None));
        // Al final: sin grupo.
        assert_eq!(drop_target(&items, 500.0, 550.0), (5, 0, None));
    }

    #[test]
    fn drop_target_on_a_collapsed_chip_goes_inside() {
        // [a] [● G 3] [b]
        let items = row(&[(T { group: 0 }, 100.0), (Chip { group: 4, collapsed: true }, 70.0), (T { group: 0 }, 100.0)]);
        // El puntero sobre el chip plegado manda, aunque el borde izquierdo esté antes.
        assert_eq!(drop_target(&items, 40.0, 130.0), (2, 4, Some(4)));
        // Al lado, sin el puntero encima: después de un chip plegado no se entra.
        assert_eq!(drop_target(&items, 160.0, 175.0), (2, 0, None));
        // Un chip desplegado cuya única pestaña es la arrastrada: queda en el grupo.
        let items = row(&[(T { group: 0 }, 100.0), (Chip { group: 4, collapsed: false }, 70.0)]);
        assert_eq!(drop_target(&items, 160.0, 175.0), (2, 4, None));
    }

    #[test]
    fn keyboard_steps_follow_the_same_stops() {
        // a, [G: g1, g2], b — G desplegado.
        let order = [(1, 0), (2, 9), (3, 9), (4, 0)];
        let none: &[u64] = &[];
        // a → derecha: entra a G como primera.
        assert_eq!(tab_step(&order, none, 1, 1), Some((Some(2), 9)));
        // g1 → derecha: se intercambia con g2.
        assert_eq!(tab_step(&order, none, 2, 1), Some((Some(4), 9)));
        // g2 → derecha (última): sale y queda justo después del grupo.
        assert_eq!(tab_step(&order, none, 3, 1), Some((Some(4), 0)));
        // b → derecha: en el extremo, no se mueve.
        assert_eq!(tab_step(&order, none, 4, 1), None);
        // b → izquierda: entra a G como última.
        assert_eq!(tab_step(&order, none, 4, -1), Some((None, 9)));
        // g1 → izquierda (primera): sale por delante del chip.
        assert_eq!(tab_step(&order, none, 2, -1), Some((Some(3), 0)));
        // g2 → izquierda: se intercambia con g1.
        assert_eq!(tab_step(&order, none, 3, -1), Some((Some(2), 9)));
        // a → izquierda: extremo.
        assert_eq!(tab_step(&order, none, 1, -1), None);
        // G plegado: a → derecha lo salta entero; b → izquierda también.
        assert_eq!(tab_step(&order, &[9], 1, 1), Some((Some(4), 0)));
        assert_eq!(tab_step(&order, &[9], 4, -1), Some((Some(2), 0)));
        // Dentro de un grupo plegado (es la activa, a la vista): sale.
        assert_eq!(tab_step(&order, &[9], 2, 1), Some((Some(4), 0)));
        assert_eq!(tab_step(&order, &[9], 3, -1), Some((Some(2), 0)));
    }

    #[test]
    fn bar_items_hide_collapsed_except_the_active_and_skip_the_dragged() {
        let mut tabs: Vec<Tab> = (1..=4).map(|id| Tab::new(id, "C:\\")).collect();
        tabs[1].group = 5;
        tabs[2].group = 5;
        let mut groups = vec![TabGroup { id: 5, name: "G".into(), color: 0, collapsed: false }];
        let show = [0, 1, 2, 3];
        assert_eq!(bar_items(&tabs, &groups, &show, 0, None), vec![BarItem::Tab(0), BarItem::Group(0), BarItem::Tab(1), BarItem::Tab(2), BarItem::Tab(3)]);
        groups[0].collapsed = true;
        assert_eq!(bar_items(&tabs, &groups, &show, 2, None), vec![BarItem::Tab(0), BarItem::Group(0), BarItem::Tab(2), BarItem::Tab(3)]);
        assert_eq!(bar_items(&tabs, &groups, &show, 0, Some(4)), vec![BarItem::Tab(0), BarItem::Group(0)]);
    }
}
