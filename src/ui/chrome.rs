//! Barra lateral, pestañas, barra de herramientas y barra de estado.

use super::actions::Action;
use super::config::{Config, ViewMode};
use super::state::Tab;
use super::theme::{self, Theme};
use super::views::elided;
use crate::fsx::{self, Drive, DriveKind, Place, transfer};
use egui::{Align2, Color32, FontId, Id, Rect, Response, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

pub fn icon_btn(ui: &mut Ui, t: &Theme, icon: &str, enabled: bool, on: bool, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 6.0, t.sel_inactive);
    } else if enabled && resp.hovered() {
        p.rect_filled(rect, 6.0, t.hover);
    }
    let c = if !enabled {
        t.faint
    } else if on {
        t.accent
    } else {
        t.text
    };
    p.text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon_font(16.0, false), c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

fn section_header(ui: &mut Ui, t: &Theme, label: &str, collapsed: bool) -> (Response, Rect) {
    ui.add_space(10.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let p = ui.painter();
    let c = if resp.hovered() { t.text } else { t.faint };
    p.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, label.to_uppercase(), theme::semibold(11.0), c);
    if resp.hovered() || collapsed {
        p.text(
            pos2(rect.right() - 54.0, rect.center().y),
            Align2::LEFT_CENTER,
            if collapsed { ic::CARET_RIGHT } else { ic::CARET_DOWN },
            theme::icon_font(10.0, false),
            t.faint,
        );
    }
    (resp, rect)
}

fn side_item(ui: &mut Ui, t: &Theme, icon: &str, icon_color: Color32, fill: bool, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let p = ui.painter();
    let r = rect.shrink2(vec2(6.0, 1.0));
    if selected {
        p.rect_filled(r, 6.0, t.sel_inactive);
    } else if resp.hovered() {
        p.rect_filled(r, 6.0, t.hover);
    }
    p.text(pos2(rect.left() + 16.0, rect.center().y), Align2::LEFT_CENTER, icon, theme::icon_font(16.0, fill), icon_color);
    let g = elided(p, label, FontId::proportional(13.5), t.text, rect.width() - 52.0);
    p.galley(pos2(rect.left() + 40.0, rect.center().y - g.size().y / 2.0), g, t.text);
    resp
}

/// Resalta una fila de la barra lateral mientras se arrastran archivos encima. Devuelve lo que se
/// soltó en ella (de Rayo o de otra app), si se soltó algo.
fn sidebar_drop(ui: &Ui, t: &Theme, resp: &Response) -> Option<Vec<String>> {
    if crate::ui::views::drag_hovering(resp) {
        let r = resp.rect.shrink2(vec2(6.0, 1.0));
        ui.painter().rect_filled(r, 6.0, t.accent.gamma_multiply(0.18));
        ui.painter().rect_stroke(r, 6.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
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
fn location_menu(ui: &mut Ui, path: &str, split: bool, groups: &[String], actions: &mut Vec<Action>) {
    let item = |ui: &mut Ui, icon: &str, text: &str| ui.add(egui::Button::new(format!("{icon}  {text}")).min_size(vec2(220.0, 0.0))).clicked();
    if item(ui, ic::FOLDER_OPEN, t!("comun.abrir")) {
        actions.push(Action::Navigate(path.to_string()));
    }
    if item(ui, ic::TABS, t!("barra.abrir_pestana_nueva")) {
        actions.push(Action::NewTab(Some(path.to_string())));
    }
    if split && item(ui, ic::SQUARE_SPLIT_HORIZONTAL, t!("barra.abrir_otro_panel")) {
        actions.push(Action::OpenOther(path.to_string()));
    }
    if !groups.is_empty() {
        ui.menu_button(format!("{}  {}", ic::STAR, t!("comun.agregar_favoritos")), |ui| {
            for (gi, g) in groups.iter().enumerate() {
                if ui.button(g).clicked() {
                    actions.push(Action::AddFavorite(gi, path.to_string()));
                }
            }
        });
    }
    ui.separator();
    if item(ui, ic::TERMINAL, t!("barra.abrir_terminal_aqui")) {
        actions.push(Action::Terminal(path.to_string()));
    }
    if item(ui, ic::FOLDER_OPEN, t!("barra.mostrar_explorador")) {
        actions.push(Action::ExplorerAt(path.to_string()));
    }
    if item(ui, ic::INFO, t!("barra.propiedades")) {
        actions.push(Action::Properties(path.to_string()));
    }
}

fn drive_row(ui: &mut Ui, t: &Theme, d: &Drive, selected: bool, split: bool, groups: &[String], actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    drop_into(ui, t, &resp, &d.root(), actions);
    let p = ui.painter().clone();
    let r = rect.shrink2(vec2(6.0, 1.0));
    if selected {
        p.rect_filled(r, 6.0, t.sel_inactive);
    } else if resp.hovered() {
        p.rect_filled(r, 6.0, t.hover);
    }
    let icon = match d.kind {
        DriveKind::Removable => ic::USB,
        DriveKind::Network => ic::GLOBE,
        DriveKind::Optical => ic::DISC,
        _ if d.ejectable => ic::USB,
        _ => ic::HARD_DRIVE,
    };
    p.text(pos2(rect.left() + 16.0, rect.top() + 15.0), Align2::LEFT_CENTER, icon, theme::icon_font(17.0, false), t.dim);
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
    let eject_w = if d.ejectable { 28.0 } else { 0.0 };
    let g = elided(&p, &title, FontId::proportional(13.0), t.text, rect.width() - 52.0 - eject_w);
    p.galley(pos2(rect.left() + 40.0, rect.top() + 6.0), g, t.text);
    if d.total > 0 {
        let bar = Rect::from_min_size(pos2(rect.left() + 40.0, rect.top() + 26.0), vec2(rect.width() - 56.0 - eject_w, 4.0));
        p.rect_filled(bar, 2.0, t.border);
        let frac = d.used_frac().clamp(0.0, 1.0);
        let fill = if frac > 0.9 { t.danger } else { t.accent };
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, 4.0)), 2.0, fill);
        p.text(
            pos2(rect.left() + 40.0, rect.top() + 38.0),
            Align2::LEFT_CENTER,
            tf!("barra.libres", libre = fsx::human_size(d.free), total = fsx::human_size(d.total)),
            FontId::proportional(10.5),
            t.faint,
        );
    }
    if resp.clicked() {
        actions.push(Action::Navigate(d.root()));
    }
    if resp.middle_clicked() {
        actions.push(Action::NewTab(Some(d.root())));
    }
    if d.ejectable {
        let er = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 16.0), vec2(24.0, 22.0));
        let eresp = ui.interact(er, Id::new(("eject", d.letter)), Sense::click()).on_hover_text(t!("barra.expulsar_forma_segura"));
        if eresp.hovered() {
            p.rect_filled(er, 5.0, t.border);
        }
        p.text(er.center(), Align2::CENTER_CENTER, ic::EJECT, theme::icon_font(14.0, false), if eresp.hovered() { t.text } else { t.dim });
        if eresp.clicked() {
            actions.push(Action::Eject(d.letter));
        }
    }
    resp.context_menu(|ui| {
        location_menu(ui, &d.root(), split, groups, actions);
        if d.ejectable {
            ui.separator();
            if ui.button(format!("{}  {}", ic::EJECT, t!("barra.expulsar"))).clicked() {
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
}

pub fn sidebar(ui: &mut Ui, s: SidebarIn, actions: &mut Vec<Action>) {
    let t = s.t;
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let cur_lower = s.current.to_lowercase();

        let (h, _) = section_header(ui, t, t!("barra.lugares"), s.cfg.places_collapsed);
        if h.clicked() {
            s.cfg.places_collapsed ^= true;
        }
        let groups: Vec<String> = s.cfg.groups.iter().map(|g| g.name.clone()).collect();
        let split = s.cfg.split;
        if !s.cfg.places_collapsed {
            for p in s.places {
                let sel = p.path.to_lowercase() == cur_lower;
                let r = side_item(ui, t, p.icon, if sel { t.accent } else { t.dim }, false, p.name, sel);
                drop_into(ui, t, &r, &p.path, actions);
                if r.clicked() {
                    actions.push(Action::Navigate(p.path.clone()));
                }
                if r.middle_clicked() {
                    actions.push(Action::NewTab(Some(p.path.clone())));
                }
                r.context_menu(|ui| location_menu(ui, &p.path, split, &groups, actions));
            }
            // Lugares especiales: lo reciente de todo el equipo y la papelera.
            let sel = s.special == "recientes";
            let r = side_item(ui, t, ic::CLOCK_COUNTER_CLOCKWISE, if sel { t.accent } else { t.dim }, false, t!("barra.recientes"), sel);
            if r.clicked() || r.middle_clicked() {
                actions.push(Action::OpenRecent);
            }
            let sel = s.special == "papelera";
            let r = side_item(ui, t, ic::TRASH_SIMPLE, if sel { t.accent } else { t.dim }, false, t!("comun.papelera"), sel);
            if let Some(files) = sidebar_drop(ui, t, &r) {
                // Soltar en la papelera: mandar ahí (como la tecla Supr).
                actions.push(Action::Delete(files, false));
            }
            if r.clicked() || r.middle_clicked() {
                actions.push(Action::OpenRecycle);
            }
        }

        let (h, _) = section_header(ui, t, t!("barra.unidades"), s.cfg.drives_collapsed);
        if h.clicked() {
            s.cfg.drives_collapsed ^= true;
        }
        if !s.cfg.drives_collapsed {
            for d in s.drives {
                let sel = cur_lower == d.root().to_lowercase();
                drive_row(ui, t, d, sel, split, &groups, actions);
            }
        }

        let ngroups = s.cfg.groups.len();
        for gi in 0..ngroups {
            let editing = s.group_edit.as_ref().is_some_and(|(i, _)| *i == gi);
            if editing {
                ui.add_space(10.0);
                let (_, buf) = s.group_edit.as_mut().unwrap();
                let mut f = super::widgets::Field::new(("grupo-nombre", gi), egui_phosphor::regular::TAG, t!("barra.nombre_grupo"));
                f.width = ui.available_width() - 12.0;
                f.height = 30.0;
                let r = super::widgets::field(ui, t, buf, f).edit;
                r.request_focus();
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) || r.lost_focus() {
                    let name = buf.trim().to_string();
                    if !name.is_empty() {
                        s.cfg.groups[gi].name = name;
                    }
                    *s.group_edit = None;
                }
                continue;
            }
            let collapsed = s.cfg.groups[gi].collapsed;
            let name = s.cfg.groups[gi].name.clone();
            let (h, hrect) = section_header(ui, t, &name, collapsed);
            if let Some(files) = sidebar_drop(ui, t, &h) {
                // Soltar carpetas en el título de un grupo: agregarlas a sus favoritos.
                for f in files.into_iter().filter(|f| fsx::is_dir_like(f)) {
                    actions.push(Action::AddFavorite(gi, f));
                }
            }
            if h.clicked() {
                s.cfg.groups[gi].collapsed ^= true;
            }
            // Botones del grupo: agregar carpeta actual y menú.
            let add_r = Rect::from_center_size(pos2(hrect.right() - 36.0, hrect.center().y), vec2(20.0, 20.0));
            let more_r = Rect::from_center_size(pos2(hrect.right() - 14.0, hrect.center().y), vec2(20.0, 20.0));
            let add = ui.interact(add_r, Id::new(("gadd", gi)), Sense::click()).on_hover_text(t!("barra.agregar_carpeta_actual"));
            let more = ui.interact(more_r, Id::new(("gmore", gi)), Sense::click());
            let p = ui.painter();
            for (r, resp, icon) in [(add_r, &add, ic::PLUS), (more_r, &more, ic::DOTS_THREE)] {
                if resp.hovered() {
                    p.rect_filled(r, 4.0, t.hover);
                }
                if h.hovered() || resp.hovered() {
                    p.text(r.center(), Align2::CENTER_CENTER, icon, theme::icon_font(12.0, false), t.dim);
                }
            }
            if add.clicked() && !s.current.is_empty() {
                actions.push(Action::AddFavorite(gi, s.current.to_string()));
            }
            egui::Popup::menu(&more).show(|ui| {
                if ui.button(format!("{}  {}", ic::PENCIL_SIMPLE, t!("barra.renombrar_grupo"))).clicked() {
                    actions.push(Action::RenameGroup(gi));
                }
                if ui.button(format!("{}  {}", ic::TRASH, t!("barra.eliminar_grupo"))).clicked() {
                    actions.push(Action::DeleteGroup(gi));
                }
            });
            if collapsed {
                continue;
            }
            if s.cfg.groups[gi].items.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(t!("barra.usa_agregar_carpeta_actual")).size(11.5).color(t.faint));
                });
            }
            for (ii, path) in s.cfg.groups[gi].items.iter().enumerate() {
                let sel = path.to_lowercase() == cur_lower;
                let label = if path.len() <= 3 { path.clone() } else { fsx::file_name(path) };
                let r = side_item(ui, t, ic::FOLDER, t.folder, true, &label, sel).on_hover_text(path);
                drop_into(ui, t, &r, path, actions);
                if r.clicked() {
                    actions.push(Action::Navigate(path.clone()));
                }
                if r.middle_clicked() {
                    actions.push(Action::NewTab(Some(path.clone())));
                }
                r.context_menu(|ui| {
                    location_menu(ui, path, split, &[], actions);
                    ui.separator();
                    if ui.button(format!("{}  {}", ic::X, t!("barra.quitar_favoritos"))).clicked() {
                        actions.push(Action::RemoveFavorite(gi, ii));
                    }
                });
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            if ui.button(egui::RichText::new(format!("{}  {}", ic::PLUS, t!("comun.nuevo_grupo"))).color(t.dim)).clicked() {
                actions.push(Action::NewGroup);
            }
        });
        ui.add_space(12.0);
    });
}

// ---------------------------------------------------------------- pestañas

/// Pestañas de un panel: `show` son los índices (en `tabs`) que le pertenecen.
/// Alto de la fila de arriba (pestañas y botones de la ventana): es la barra de título.
pub const TITLE_H: f32 = 40.0;
/// Cuadro y tamaño de la X de las pestañas y del "+" (iguales, para que se vean parejos).
const TAB_BTN: f32 = 22.0;
/// La X se dibuja en diagonal (ocupa menos): un poco más grande para verse igual que el "+".
const TAB_X_ICON: f32 = 14.0;
const TAB_PLUS_ICON: f32 = 12.0;
/// Ancho de los tres botones de la ventana.
pub const WINDOW_BUTTONS_W: f32 = 3.0 * 46.0;

/// Pestañas de un panel. `reserve`: espacio a la derecha que no se usa (ahí van los
/// botones de la ventana). Devuelve dónde termina el botón "+" (lo que sigue es espacio
/// libre: barra de título).
pub fn tab_bar(ui: &mut Ui, t: &Theme, tabs: &[Tab], show: &[usize], cur: usize, reserve: f32, actions: &mut Vec<Action>) -> f32 {
    let full = ui.available_rect_before_wrap();
    let n = show.len().max(1);
    let plus_w = 36.0;
    let tab_w = ((full.width() - reserve - plus_w - 12.0) / n as f32).clamp(90.0, 220.0);
    let mut x = full.left() + 6.0;
    let p = ui.painter().clone();
    for (k, &i) in show.iter().enumerate() {
        let tab = &tabs[i];
        let r = Rect::from_min_size(pos2(x, full.top() + 6.0), vec2(tab_w - 4.0, full.height() - 6.0));
        let resp = ui.interact(r, Id::new(("tab", tab.id)), Sense::click());
        let active = i == cur;
        // Arrastrar archivos sobre una pestaña: tras un momento se cambia a ella; soltarlos ahí los lleva a su carpeta.
        if tab.tool.is_none() && crate::ui::views::drag_hovering(&resp) {
            p.rect_stroke(r.shrink(1.0), 7.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            if !active {
                let key = Id::new("tab-drag-hover");
                let now = ui.input(|i| i.time);
                let pass = ui.ctx().cumulative_pass_nr();
                // (pestaña, desde cuándo, último cuadro): si el puntero se fue, el tiempo vuelve a empezar.
                let since = ui.data_mut(|d| {
                    let t0 = match d.get_temp::<(u64, f64, u64)>(key) {
                        Some((id, t0, last)) if id == tab.id && last + 1 >= pass => t0,
                        _ => now,
                    };
                    d.insert_temp(key, (tab.id, t0, pass));
                    t0
                });
                if now - since > 0.45 {
                    actions.push(Action::SwitchTab(i));
                    ui.data_mut(|d| d.remove::<(u64, f64, u64)>(key));
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                }
            }
        }
        if tab.tool.is_none() {
            if let Some(files) = crate::ui::views::take_drop(&resp) {
                actions.push(Action::DropFiles { paths: files, dest: tab.path().to_string(), force: crate::ui::views::drop_force() });
            }
        }
        // Se mira la posición del ratón (no `hovered`): la X queda encima de la pestaña y,
        // si dependiera de ella, la X aparecería y desaparecería en cuadros alternos.
        let hovering = ui.rect_contains_pointer(r);
        if active {
            p.rect_filled(r, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, t.bg);
        } else if hovering {
            p.rect_filled(r.shrink2(vec2(0.0, 2.0)), 6.0, t.hover);
        }
        let icon = if let Some(tool) = &tab.tool {
            tool.icon()
        } else if tab.search.text.trim_start().to_lowercase().starts_with("recientes:") {
            ic::CLOCK_COUNTER_CLOCKWISE
        } else if tab.search.active() {
            ic::MAGNIFYING_GLASS
        } else {
            ic::FOLDER
        };
        p.text(pos2(r.left() + 12.0, r.center().y), Align2::LEFT_CENTER, icon, theme::icon_font(14.0, true), if active { t.folder } else { t.dim });
        let recent = tab.search.text.trim_start().to_lowercase().starts_with("recientes:");
        let title = if recent && tab.tool.is_none() {
            t!("barra.recientes").to_string()
        } else if tab.search.active() && tab.tool.is_none() {
            format!("“{}”", tab.search.text.trim())
        } else {
            tab.title()
        };
        let close_r = Rect::from_center_size(pos2(r.right() - 15.0, r.center().y), vec2(TAB_BTN, TAB_BTN));
        let g = elided(&p, &title, FontId::proportional(12.5), if active { t.text } else { t.dim }, r.width() - 56.0);
        p.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, t.text);
        if resp.clicked() {
            actions.push(Action::SwitchTab(i));
        }
        if resp.middle_clicked() {
            actions.push(Action::CloseTab(i));
        }
        // La X existe siempre (así no cambia lo que hay bajo el ratón); se ve en la activa y al pasar por encima.
        let cresp = ui.interact(close_r, Id::new(("tabx", tab.id)), Sense::click());
        if active || hovering {
            if cresp.hovered() {
                p.rect_filled(close_r, 5.0, t.border);
            }
            p.text(close_r.center(), Align2::CENTER_CENTER, ic::X, theme::icon_font(TAB_X_ICON, false), t.dim);
            if cresp.clicked() {
                actions.push(Action::CloseTab(i));
            }
        }
        let next_active = show.get(k + 1) == Some(&cur);
        if !active && !next_active && k + 1 < show.len() {
            p.line_segment([pos2(r.right() + 1.5, r.top() + 9.0), pos2(r.right() + 1.5, r.bottom() - 7.0)], egui::Stroke::new(1.0, t.border));
        }
        x += tab_w;
    }
    // Mismo tamaño y misma altura que la X de las pestañas (centrado con su texto).
    let pr = Rect::from_center_size(pos2(x + 4.0 + TAB_BTN / 2.0, full.top() + 6.0 + (full.height() - 6.0) / 2.0), vec2(TAB_BTN, TAB_BTN));
    let presp = ui.interact(pr, Id::new(("newtab", show.first().copied().unwrap_or(0))), Sense::click()).on_hover_text(t!("barra.pestana_nueva_ctrl_t"));
    if presp.hovered() {
        p.rect_filled(pr, 5.0, t.hover);
    }
    // +1,25 px: a este tamaño la tipografía dibuja el "+" un poco más arriba que la X.
    p.text(pr.center() + vec2(0.0, 1.25), Align2::CENTER_CENTER, ic::PLUS, theme::icon_font(TAB_PLUS_ICON, false), t.dim);
    if presp.clicked() {
        actions.push(Action::NewTab(None));
    }
    ui.allocate_rect(full, Sense::hover());
    pr.right()
}

/// Arriba de la barra lateral: el ícono y el nombre de Rayo, discretos.
pub fn app_title(ui: &Ui, t: &Theme, band: Rect) {
    let p = ui.painter();
    // A la altura del texto de las pestañas (que empiezan 6 px más abajo).
    let cy = band.top() + 3.0 + band.height() / 2.0;
    let c = pos2(band.left() + 22.0, cy + 1.0);
    // La carpeta con el rayo, como el ícono de la app.
    p.text(c, Align2::CENTER_CENTER, egui_phosphor::fill::FOLDER, theme::icon_font(17.0, true), t.accent);
    p.text(c + vec2(0.5, 1.5), Align2::CENTER_CENTER, egui_phosphor::fill::LIGHTNING, theme::icon_font(8.5, true), Color32::WHITE);
    p.text(pos2(c.x + 16.0, cy), Align2::LEFT_CENTER, "Rayo", theme::semibold(12.5), t.dim);
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
                ui.painter().rect_filled(r, 0.0, if close { Color32::from_rgb(0xC4, 0x2B, 0x1C) } else { t.hover });
            }
            let color = if close && hover { Color32::WHITE } else { t.text };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, theme::icon_font(if k == 1 { 12.0 } else { 14.0 }, false), color);
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

pub fn toolbar(ui: &mut Ui, t: &Theme, cfg: &Config, tab: &mut Tab, search_id: Id, actions: &mut Vec<Action>) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(8.0);
        if icon_btn(ui, t, ic::ARROW_LEFT, !tab.back.is_empty(), false, t!("barra.atras_alt")).clicked() {
            actions.push(Action::Back);
        }
        if icon_btn(ui, t, ic::ARROW_RIGHT, !tab.fwd.is_empty(), false, t!("barra.adelante_alt")).clicked() {
            actions.push(Action::Forward);
        }
        let can_up = fsx::parent_of(tab.path()).is_some();
        if icon_btn(ui, t, ic::ARROW_UP, can_up, false, t!("barra.subir_alt")).clicked() {
            actions.push(Action::Up);
        }
        if icon_btn(ui, t, ic::ARROW_CLOCKWISE, true, false, t!("barra.actualizar_f5")).clicked() {
            actions.push(Action::Refresh);
        }
        ui.add_space(6.0);

        // Lado derecho primero para saber cuánto espacio queda para la ruta.
        // En paneles angostos la búsqueda se achica para dejarle lugar a la ruta.
        let search_w = (ui.available_width() * 0.3).clamp(140.0, 300.0);
        let right_w = 30.0 * 4.0 + search_w + 24.0;
        let crumb_w = (ui.available_width() - right_w).max(100.0);
        breadcrumb(ui, t, tab, crumb_w, actions);
        ui.add_space(8.0);

        if icon_btn(ui, t, ic::COLUMNS, true, cfg.view == ViewMode::Columns, t!("barra.vista_columnas_ctrl_1")).clicked() {
            actions.push(Action::SetView(ViewMode::Columns));
        }
        if icon_btn(ui, t, ic::LIST_BULLETS, true, cfg.view == ViewMode::Details, t!("barra.vista_detalles_ctrl_2")).clicked() {
            actions.push(Action::SetView(ViewMode::Details));
        }
        if icon_btn(ui, t, ic::SQUARES_FOUR, true, cfg.view == ViewMode::Grid, t!("barra.vista_cuadricula_ctrl_3")).clicked() {
            actions.push(Action::SetView(ViewMode::Grid));
        }
        if icon_btn(ui, t, ic::SIDEBAR_SIMPLE, true, cfg.show_preview, t!("barra.panel_vista_previa_ctrl_p")).clicked() {
            actions.push(Action::TogglePreview);
        }
        ui.add_space(8.0);
        search_box(ui, t, tab, search_id, search_w, &cfg.search_history, actions);
    });
}

/// Subcarpetas que completan lo escrito en la barra de ruta (máximo 8).
fn path_suggestions(ctx: &egui::Context, typed: &str) -> Vec<String> {
    let typed = typed.trim().trim_matches('"');
    // Sin barra todavía: las unidades.
    let Some(cut) = typed.rfind(['\\', '/']) else {
        let t = typed.to_lowercase();
        return fsx::list_drives().into_iter().map(|d| format!("{}:\\", d.letter)).filter(|d| t.is_empty() || d.to_lowercase().starts_with(&t)).collect();
    };
    let dir = &typed[..=cut];
    let prefix = typed[cut + 1..].to_lowercase();
    // El listado de la carpeta se recuerda mientras no cambie la carpeta escrita.
    let key = Id::new("path-sugg");
    let cached: Option<(String, std::sync::Arc<Vec<String>>)> = ctx.data(|d| d.get_temp(key));
    let names = match cached {
        Some((d, n)) if d.eq_ignore_ascii_case(dir) => n,
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

fn breadcrumb(ui: &mut Ui, t: &Theme, tab: &mut Tab, width: f32, actions: &mut Vec<Action>) {
    let (rect, bg) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let p = ui.painter().clone();
    p.rect_filled(rect, 7.0, t.bg);
    p.rect_stroke(rect, 7.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if let Some(buf) = tab.path_edit.as_mut() {
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
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 3.0))));
        let mut out = egui::TextEdit::singleline(buf).id(id).lock_focus(true).frame(egui::Frame::NONE).desired_width(rect.width() - 12.0).vertical_align(egui::Align::Center).show(&mut child);
        let r = out.response.clone();
        if completed {
            // Cursor al final tras completar.
            let n = buf.chars().count();
            out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
            out.state.store(ui.ctx(), id);
        }
        if !r.has_focus() && !r.lost_focus() {
            r.request_focus();
        }
        if r.changed() {
            sel = None;
        }
        ui.data_mut(|d| d.insert_temp(sel_id, sel));
        // Lista desplegable bajo la barra.
        let mut clicked = None;
        if !sugg.is_empty() {
            mark_overlay(ui.ctx());
            egui::Area::new(id.with("sugg")).order(egui::Order::Foreground).fixed_pos(rect.left_bottom() + vec2(0.0, 4.0)).show(ui.ctx(), |ui| {
                egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(4).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                    ui.set_width(rect.width() - 8.0);
                    for (k, s) in sugg.iter().enumerate() {
                        let (rr, resp) = ui.allocate_exact_size(vec2(rect.width() - 8.0, 26.0), Sense::click());
                        if Some(k) == sel || resp.hovered() {
                            ui.painter().rect_filled(rr, 5.0, if Some(k) == sel { t.sel } else { t.hover });
                        }
                        ui.painter().text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, ic::FOLDER, theme::icon_font(14.0, true), t.folder);
                        let g = crate::ui::views::elided(ui.painter(), s, FontId::proportional(13.0), t.text, rr.width() - 44.0);
                        ui.painter().galley(pos2(rr.left() + 34.0, rr.center().y - g.size().y / 2.0), g, t.text);
                        if resp.clicked() {
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
    // Segmentos de la ruta
    let path = tab.path().to_string();
    let mut segs: Vec<(String, String)> = Vec::new();
    let mut cur = Some(path.clone());
    while let Some(c) = cur {
        let label = crumb_label(&c);
        cur = fsx::parent_of(&c);
        segs.push((label, c));
    }
    segs.reverse();
    let font = FontId::proportional(13.0);
    let sep_w = 18.0;
    let widths: Vec<f32> = segs.iter().map(|(l, _)| p.layout_no_wrap(l.clone(), font.clone(), t.text).size().x + 12.0).collect();
    // Cada tramo lleva su ▸ (también el último: abre sus subcarpetas). Si no cabe, se ocultan los primeros.
    let avail = rect.width() - 34.0;
    let mut start = 0;
    let mut total: f32 = widths.iter().sum::<f32>() + sep_w * segs.len() as f32;
    while total > avail && start + 1 < segs.len() {
        total -= widths[start] + sep_w;
        start += 1;
    }
    let menu_id = Id::new(("crumb-menu", tab.id));
    let open_key = ui.data(|d| d.get_temp::<CrumbMenu>(menu_id)).map(|m| m.key);
    let mut x = rect.left() + 6.0;
    if start > 0 {
        // "…": los tramos que no entran, en una lista.
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(24.0, rect.height() - 6.0));
        let resp = ui.interact(r, menu_id.with("more"), Sense::click()).on_hover_text(t!("barra.carpetas_anteriores"));
        if resp.hovered() || open_key == Some(usize::MAX) {
            p.rect_filled(r, 5.0, t.hover);
        }
        p.text(r.center(), Align2::CENTER_CENTER, "…", font.clone(), t.dim);
        if resp.clicked() {
            let items = segs[..start].iter().rev().cloned().collect();
            toggle_crumb_menu(ui, menu_id, CrumbMenu { key: usize::MAX, src: r, path: path.clone(), items: std::sync::Arc::new(items), current: None, scrolled: false });
        }
        x += 26.0;
    }
    let n = segs.len();
    for (i, (label, full)) in segs.iter().enumerate().skip(start) {
        let w = widths[i];
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
        let resp = ui.interact(r, Id::new(("crumb", tab.id, i)), Sense::click_and_drag());
        // Soltar archivos sobre un tramo los lleva a esa carpeta (como en el Explorador de antes).
        if crate::ui::views::drag_hovering(&resp) {
            p.rect_filled(r, 5.0, t.accent.gamma_multiply(0.18));
            p.rect_stroke(r, 5.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            p.rect_filled(r, 5.0, t.hover);
        }
        let last = i + 1 == n;
        let c = if last { t.text } else { t.dim };
        let f = if last { theme::semibold(13.0) } else { font.clone() };
        p.text(r.center(), Align2::CENTER_CENTER, label, f, c);
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
            resp.on_hover_text(t!("barra.arrastra_llevar_esta_carpeta_otro"));
        }
        x += w;
        // ▸: las subcarpetas de este tramo, con la siguiente marcada.
        let cr = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(sep_w, rect.height() - 6.0));
        let cresp = ui.interact(cr, Id::new(("crumb-sep", tab.id, i)), Sense::click());
        let open_here = open_key == Some(i);
        if cresp.hovered() || open_here {
            p.rect_filled(cr, 5.0, t.hover);
        }
        let caret = if open_here { ic::CARET_DOWN } else { ic::CARET_RIGHT };
        p.text(cr.center(), Align2::CENTER_CENTER, caret, theme::icon_font(10.0, false), if cresp.hovered() || open_here { t.dim } else { t.faint });
        if cresp.clicked() {
            let current = segs.get(i + 1).map(|(_, f)| f.clone());
            toggle_crumb_menu(ui, menu_id, CrumbMenu { key: i, src: cr, path: path.clone(), items: std::sync::Arc::new(subfolders(full)), current, scrolled: false });
        }
        x += sep_w;
    }
    if bg.clicked() {
        tab.path_edit = Some(path.clone());
    }
    bg.on_hover_text(t!("barra.clic_escribir_ruta_comando_ctrl"));
    crumb_menu(ui, t, menu_id, &path, actions);
}

fn crumb_label(path: &str) -> String {
    if path.len() <= 3 { path.trim_end_matches('\\').to_string() } else { fsx::file_name(path) }
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
    let w = 300.0;
    // Cada lista con su propio id: si no, la nueva hereda el tamaño de la anterior.
    let area = egui::Area::new(id.with(("area", m.key, m.items.len()))).order(egui::Order::Foreground).fixed_pos(m.src.left_bottom() + vec2(-4.0, 6.0)).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(4).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
            ui.set_width(w);
            if m.items.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t!("barra.sin_subcarpetas")).color(t.faint));
                ui.add_space(4.0);
                return;
            }
            egui::ScrollArea::vertical().id_salt(("crumb-scroll", m.key)).max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                for (name, full) in m.items.iter() {
                    let is_cur = m.current.as_ref().is_some_and(|c| c.eq_ignore_ascii_case(full));
                    let (rr, resp) = ui.allocate_exact_size(vec2(w - 8.0, 26.0), Sense::click());
                    if is_cur {
                        ui.painter().rect_filled(rr, 5.0, t.sel);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rr, 5.0, t.hover);
                    }
                    let icon = if full.len() <= 3 { ic::HARD_DRIVES } else { ic::FOLDER };
                    ui.painter().text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, icon, theme::icon_font(14.0, true), t.folder);
                    let f = if is_cur { theme::semibold(13.0) } else { FontId::proportional(13.0) };
                    let g = elided(ui.painter(), name, f, t.text, rr.width() - 44.0);
                    ui.painter().galley(pos2(rr.left() + 34.0, rr.center().y - g.size().y / 2.0), g, t.text);
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

fn search_box(ui: &mut Ui, t: &Theme, tab: &mut Tab, id: Id, width: f32, history: &[String], actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.min(ui.available_width() - 6.0).max(120.0), 30.0), Sense::hover());
    let p = ui.painter().clone();
    let focused = ui.memory(|m| m.has_focus(id));
    p.rect_filled(rect, 7.0, t.bg);
    p.rect_stroke(rect, 7.0, egui::Stroke::new(1.0, if focused { t.accent } else { t.border }), egui::StrokeKind::Inside);
    p.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, ic::MAGNIFYING_GLASS, theme::icon_font(14.0, false), t.dim);
    // Chip de alcance: todo el equipo / esta carpeta.
    let chip_txt = if tab.search.here { t!("barra.aqui") } else { t!("barra.todo") };
    let chip = Rect::from_min_size(pos2(rect.right() - 50.0, rect.top() + 5.0), vec2(44.0, 20.0));
    let chip_resp = ui.interact(chip, id.with("chip"), Sense::click()).on_hover_text(t!("barra.buscar_todo_equipo_solo_carpeta"));
    p.rect_filled(chip, 5.0, if chip_resp.hovered() { t.border } else { t.hover });
    p.text(chip.center(), Align2::CENTER_CENTER, chip_txt, FontId::proportional(11.5), if tab.search.here { t.accent } else { t.dim });
    if chip_resp.clicked() {
        tab.search.here ^= true;
    }
    let edit_rect = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top() + 3.0), pos2(chip.left() - 4.0, rect.bottom() - 3.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let hist_shown = tab.search.text.is_empty() && !history.is_empty();
    // Enter con una búsqueda elegida: se toma antes de que la caja lo use (y pierda el foco).
    let hist_enter = hist_shown && ui.data(|d| d.get_temp::<Option<usize>>(id.with("hist-sel"))).flatten().is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let r = child.add(
        egui::TextEdit::singleline(&mut tab.search.text)
            .id(id)
            // Con el historial a la vista, ↑ ↓ son de la lista (no pasan el foco a otro control).
            .lock_focus(hist_shown)
            .frame(egui::Frame::NONE)
            .hint_text(egui::RichText::new(t!("barra.buscar_archivos_ctrl_f")).color(t.faint))
            .desired_width(edit_rect.width())
            .vertical_align(egui::Align::Center),
    );
    if tab.search.focus {
        r.request_focus();
        tab.search.focus = false;
    }
    search_history(ui, t, tab, id, rect, r.has_focus() || hist_enter, hist_enter, history, actions);
    let _ = Color32::TRANSPARENT;
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
    let mut sel: Option<usize> = ui.data(|d| d.get_temp(sel_id)).flatten();
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
    let w = rect.width().max(260.0);
    let area = egui::Area::new(area_id).order(egui::Order::Foreground).fixed_pos(pos2(rect.right() - w, rect.bottom() + 4.0)).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(4).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
            ui.set_width(w - 8.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(t!("barra.busquedas_recientes")).size(11.5).color(t.faint));
            });
            for (k, q) in history.iter().take(shown).enumerate() {
                let (rr, resp) = ui.allocate_exact_size(vec2(w - 8.0, 26.0), Sense::click());
                let hovering = ui.rect_contains_pointer(rr);
                if Some(k) == sel || hovering {
                    ui.painter().rect_filled(rr, 5.0, if Some(k) == sel { t.sel } else { t.hover });
                }
                ui.painter().text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, ic::CLOCK_COUNTER_CLOCKWISE, theme::icon_font(13.0, false), t.dim);
                let g = elided(ui.painter(), q, FontId::proportional(13.0), t.text, rr.width() - 64.0);
                ui.painter().galley(pos2(rr.left() + 34.0, rr.center().y - g.size().y / 2.0), g, t.text);
                // Quitar esta búsqueda del historial.
                let xr = Rect::from_center_size(pos2(rr.right() - 14.0, rr.center().y), vec2(20.0, 20.0));
                let xresp = ui.interact(xr, area_id.with(("x", k)), Sense::click());
                if hovering || Some(k) == sel {
                    if xresp.hovered() {
                        ui.painter().rect_filled(xr, 4.0, t.border);
                    }
                    ui.painter().text(xr.center(), Align2::CENTER_CENTER, ic::X, theme::icon_font(11.0, false), t.dim);
                }
                if xresp.clicked() {
                    actions.push(Action::ForgetSearch(Some(k)));
                    // La lista queda abierta (la caja recupera el foco) para seguir quitando.
                    ui.memory_mut(|m| m.request_focus(id));
                } else if resp.clicked() {
                    pick = Some(q.clone());
                }
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if ui.add(egui::Button::new(egui::RichText::new(t!("barra.borrar_historial")).size(11.5).color(t.faint)).frame(false)).clicked() {
                    actions.push(Action::ForgetSearch(None));
                }
            });
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

/// `mode`: cómo se mantiene al día el índice ("USN (servicio)", "vigilancia"…).
pub fn status_bar(ui: &mut Ui, t: &Theme, left: &str, right: &str, busy: bool, mode: &str, actions: &mut Vec<Action>) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(left).size(11.5).color(t.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let usn = mode.starts_with("USN");
            let c = if busy || usn { t.accent } else { t.faint };
            let label = ui.add(egui::Label::new(egui::RichText::new(right).size(11.5).color(if busy { t.accent } else { t.faint })).sense(Sense::click()));
            ui.label(egui::RichText::new(ic::LIGHTNING).font(theme::icon_font(12.0, true)).color(c));
            if mode == crate::index::MODE_WATCH {
                // Sin servicio: ofrecer activarlo (una vez, con permiso de administrador).
                let b = ui
                    .add(egui::Button::new(egui::RichText::new(format!("{}  {}", ic::LIGHTNING, t!("barra.activar_indice_instantaneo"))).size(11.5).color(t.accent)).frame(false))
                    .on_hover_text(t!("barra.instala_pequeno_servicio_windows_lee"));
                if b.clicked() {
                    actions.push(Action::InstallService);
                }
            }
            if usn {
                label.on_hover_text(t!("barra.indice_mantiene_dia_usn_journal")).context_menu(|ui| {
                    if mode == crate::index::MODE_SERVICE && ui.button(format!("{}  {}", ic::TRASH, t!("barra.quitar_servicio_indexado"))).clicked() {
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
            if j.workers > 1 {
                s.push_str(&tf!("barra.a_la_vez", n = j.workers));
            }
            s
        }
        State::Done if j.errors.is_empty() => tp!(j.done_files, "barra.listo_archivos", n = fsx::thousands(j.done_files)),
        State::Done => tp!(j.errors.len(), "barra.terminado_errores"),
        State::Canceled => t!("barra.cancelado").into(),
    }
}

/// Barra compacta en la barra de estado. Devuelve true si se le hizo clic.
pub fn transfer_chip(ui: &mut Ui, t: &Theme, area: Rect, jobs: &[transfer::JobView]) -> bool {
    let resp = ui.interact(area, Id::new("xfer-chip"), Sense::click()).on_hover_text(t!("barra.ver_copias"));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(area, 5.0, t.hover);
    }
    let running: Vec<&transfer::JobView> = jobs.iter().filter(|j| !j.finished()).collect();
    let (label, frac, color) = match running.first() {
        Some(j) => {
            let extra = if running.len() > 1 { format!(" (+{})", running.len() - 1) } else { String::new() };
            let pct = (j.fraction() * 100.0) as u32;
            let c = if j.state == transfer::State::Conflict { t.danger } else { t.accent };
            (format!("{}{extra} · {pct}%", j.label), j.fraction(), c)
        }
        None => {
            let errs: usize = jobs.iter().map(|j| j.errors.len()).sum();
            let txt = if errs > 0 { tf!("barra.copias_terminadas_error", errores = errs) } else { t!("barra.copias_terminadas").to_string() };
            (txt, 1.0, if errs > 0 { t.danger } else { t.dim })
        }
    };
    let bar = Rect::from_min_size(pos2(area.left() + 8.0, area.bottom() - 5.0), vec2(area.width() - 16.0, 3.0));
    p.rect_filled(bar, 1.5, t.border);
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac.clamp(0.0, 1.0), 3.0)), 1.5, color);
    let g = elided(p, &label, FontId::proportional(11.5), t.text, area.width() - 16.0);
    p.galley(pos2(area.left() + 8.0, area.top() + 1.0), g, t.text);
    resp.clicked()
}

/// Lista de copias (sobre la barra de estado). Devuelve false si se cerró.
pub fn transfers_popup(ctx: &egui::Context, t: &Theme, jobs: &[transfer::JobView], actions: &mut Vec<Action>) -> bool {
    let mut open = true;
    egui::Area::new(Id::new("xfer-popup")).order(egui::Order::Foreground).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -32.0)).show(ctx, |ui| {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(10).inner_margin(12).show(ui, |ui| {
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t!("barra.copias")).font(theme::semibold(14.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(ic::X).clicked() {
                        open = false;
                    }
                    if jobs.iter().any(|j| j.finished()) && ui.button(t!("barra.limpiar_terminadas")).clicked() {
                        actions.push(Action::TransferClear);
                    }
                });
            });
            ui.add_space(6.0);
            for j in jobs.iter().rev() {
                ui.separator();
                ui.horizontal(|ui| {
                    let icon = match j.op {
                        transfer::Op::Copy => ic::COPY,
                        transfer::Op::Move => ic::ARROW_RIGHT,
                        transfer::Op::Extract | transfer::Op::Compress => ic::FILE_ZIP,
                    };
                    ui.label(egui::RichText::new(icon).font(theme::icon_font(14.0, false)).color(t.accent));
                    ui.vertical(|ui| {
                        ui.set_width(330.0);
                        ui.label(egui::RichText::new(&j.label).size(12.5));
                        ui.label(egui::RichText::new(job_line(j)).size(11.5).color(t.dim));
                        if !j.finished() {
                            ui.add(egui::ProgressBar::new(j.fraction()).desired_height(4.0).fill(t.accent));
                            if !j.current.is_empty() {
                                ui.label(egui::RichText::new(&j.current).size(10.5).color(t.faint));
                            }
                        }
                        for e in j.errors.iter().take(3) {
                            ui.label(egui::RichText::new(e).size(10.5).color(t.danger));
                        }
                    });
                    if !j.finished() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(ic::X).on_hover_text(t!("comun.cancelar")).clicked() {
                                actions.push(Action::TransferCancel(j.id));
                            }
                            let paused = j.state == transfer::State::Paused;
                            let (icon, tip) = if paused { (ic::PLAY, t!("barra.continuar")) } else { (ic::PAUSE, t!("barra.pausar")) };
                            if ui.button(icon).on_hover_text(tip).clicked() {
                                actions.push(Action::TransferPause(j.id, !paused));
                            }
                        });
                    }
                });
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
    egui::Modal::new(Id::new(("conflict", j.id))).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(egui::RichText::new(t!("barra.este_archivo_ya_existe_destino")).font(theme::semibold(15.0)));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(fsx::file_name(&c.dst)).color(t.accent));
        ui.add_space(8.0);
        egui::Grid::new("conflict-grid").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
            ui.label("");
            ui.label(egui::RichText::new(t!("comun.nuevo")).color(t.dim));
            ui.label(egui::RichText::new(t!("barra.ya_esta")).color(t.dim));
            ui.end_row();
            ui.label(egui::RichText::new(t!("comun.tamano")).color(t.dim));
            ui.label(fsx::human_size(c.src_size));
            ui.label(fsx::human_size(c.dst_size));
            ui.end_row();
            ui.label(egui::RichText::new(t!("comun.modificado")).color(t.dim));
            let newer = |a: u32, b: u32| if a > b { t!("barra.mas_nuevo") } else { "" };
            ui.label(format!("{}{}", fsx::fmt_date(c.src_mtime), newer(c.src_mtime, c.dst_mtime)));
            ui.label(format!("{}{}", fsx::fmt_date(c.dst_mtime), newer(c.dst_mtime, c.src_mtime)));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.checkbox(&mut all, t!("barra.hacer_mismo_siguientes"));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(t!("barra.reemplazar")).clicked() {
                actions.push(Action::TransferResolve(j.id, transfer::Choice::Replace, all));
            }
            if ui.button(t!("barra.conservar_ambos")).clicked() {
                actions.push(Action::TransferResolve(j.id, transfer::Choice::KeepBoth, all));
            }
            if ui.button(t!("barra.omitir")).clicked() {
                actions.push(Action::TransferResolve(j.id, transfer::Choice::Skip, all));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("barra.cancelar_copia")).clicked() {
                    actions.push(Action::TransferCancel(j.id));
                }
            });
        });
    });
    ctx.data_mut(|d| d.insert_temp(all_id, all));
}
