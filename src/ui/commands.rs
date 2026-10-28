//! Catálogo de comandos: cada acción de Rayo con su nombre, ícono, atajo y en
//! qué contexto tiene sentido. De aquí salen la paleta (Ctrl+K) y el menú de
//! clic derecho, así que una función nueva aparece en los dos sin duplicar nada.

use super::actions::Action;
use super::theme::Theme;
use crate::index::store::search_key;
use egui::{Align2, FontId, Id, Key, Modifiers, Sense, pos2, vec2};
use egui_phosphor::regular as ic;

/// Sobre qué se ejecuta un comando.
#[derive(Clone, Debug, Default)]
pub struct Target {
    /// Elementos seleccionados (rutas completas). Puede estar vacío.
    pub paths: Vec<String>,
    /// Carpeta actual.
    pub dir: String,
    /// La selección es una sola carpeta.
    pub single_dir: bool,
    /// Hay dos paneles.
    pub split: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Needs {
    /// Siempre disponible.
    Nothing,
    /// Algo seleccionado.
    Selection,
    /// Una sola cosa seleccionada.
    One,
    /// La vista dividida activa.
    Split,
    /// Selección y vista dividida.
    SelectionSplit,
    /// Comprimidos o algo de adentro de uno.
    Archives,
}

/// Un comando. Su nombre y sus palabras de búsqueda están en los idiomas
/// (`cmd.<id>` y `cmd.<id>.buscar`); el atajo se escribe en inglés ("Ctrl+Shift+C")
/// y se muestra con los nombres de teclas del idioma.
pub struct Command {
    pub id: &'static str,
    pub icon: &'static str,
    keys: &'static str,
    pub needs: Needs,
}

impl Command {
    pub fn label(&self) -> &'static str {
        crate::i18n::tr_dyn(&format!("cmd.{}", self.id)).unwrap_or(self.id)
    }

    /// Palabras extra para encontrarlo en la paleta.
    pub fn words(&self) -> &'static str {
        crate::i18n::tr_dyn(&format!("cmd.{}.buscar", self.id)).unwrap_or("")
    }

    /// El atajo, con los nombres de las teclas en el idioma actual ("Ctrl+Mayús+C").
    pub fn keys(&self) -> String {
        keys_label(self.keys)
    }
}

/// "Ctrl+Shift+C" con los nombres de las teclas del idioma.
pub fn keys_label(keys: &str) -> String {
    keys.split('+')
        .map(|k| match k {
            "Shift" => t!("tecla.mayus"),
            "Del" => t!("tecla.supr"),
            "Space" => t!("tecla.espacio"),
            "MiddleClick" => t!("tecla.clic_central"),
            "Enter" => t!("tecla.enter"),
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

macro_rules! cmd {
    ($id:literal, $icon:expr, $keys:literal, $needs:ident) => {
        Command { id: $id, icon: $icon, keys: $keys, needs: Needs::$needs }
    };
}

pub const ALL: &[Command] = &[
    // Archivos
    cmd!("open", ic::ARROW_SQUARE_OUT, "Enter", Selection),
    cmd!("open_with", ic::APP_WINDOW, "", One),
    cmd!("open_tab", ic::TABS, "MiddleClick", Selection),
    cmd!("quicklook", ic::EYE, "Space", One),
    cmd!("copy", ic::COPY, "Ctrl+C", Selection),
    cmd!("cut", ic::SCISSORS, "Ctrl+X", Selection),
    cmd!("paste", ic::CLIPBOARD, "Ctrl+V", Nothing),
    cmd!("undo", ic::ARROW_COUNTER_CLOCKWISE, "Ctrl+Z", Nothing),
    cmd!("copy_path", ic::LINK, "Ctrl+Shift+C", Selection),
    cmd!("rename", ic::PENCIL_SIMPLE, "F2", One),
    cmd!("batch_rename", ic::TEXT_AA, "Ctrl+Shift+R", Selection),
    cmd!("extract_here", ic::FILE_ZIP, "", Archives),
    cmd!("extract_folder", ic::FOLDER_SIMPLE_PLUS, "", Archives),
    cmd!("compress", ic::FILE_ZIP, "", Selection),
    cmd!("select_all", ic::CHECK_SQUARE, "Ctrl+A", Nothing),
    cmd!("select_pattern", ic::ASTERISK, "Ctrl+Shift+S", Nothing),
    cmd!("invert_selection", ic::SWAP, "Ctrl+I", Nothing),
    cmd!("recycle", ic::TRASH_SIMPLE, "", Nothing),
    cmd!("recent", ic::CLOCK_COUNTER_CLOCKWISE, "", Nothing),
    cmd!("who_uses", ic::LOCK_SIMPLE, "", Selection),
    cmd!("hash", ic::FINGERPRINT, "", Selection),
    cmd!("paste_link", ic::LINK_SIMPLE, "", Nothing),
    cmd!("link_other", ic::LINK_SIMPLE, "", SelectionSplit),
    cmd!("new_folder", ic::FOLDER_PLUS, "Ctrl+Shift+N", Nothing),
    cmd!("delete", ic::TRASH, "Del", Selection),
    cmd!("properties", ic::INFO, "Alt+Enter", Nothing),
    cmd!("native_menu", ic::DOTS_THREE, "Shift+F10", Selection),
    // Otro panel
    cmd!("copy_other", ic::COPY, "", SelectionSplit),
    cmd!("move_other", ic::ARROW_RIGHT, "", SelectionSplit),
    cmd!("open_other", ic::SQUARE_SPLIT_HORIZONTAL, "", Split),
    cmd!("compare_other", ic::ARROWS_LEFT_RIGHT, "", Split),
    // Herramientas
    cmd!("space", ic::CHART_PIE_SLICE, "", Nothing),
    cmd!("dupes_here", ic::COPY_SIMPLE, "", Nothing),
    cmd!("dupes_drive", ic::COPY_SIMPLE, "", Nothing),
    cmd!("compare", ic::ARROWS_LEFT_RIGHT, "", Nothing),
    cmd!("transfers", ic::LIST_BULLETS, "", Nothing),
    cmd!("terminal", ic::TERMINAL, "", Nothing),
    cmd!("explorer", ic::FOLDER_OPEN, "", Nothing),
    cmd!("favorite", ic::STAR, "", Nothing),
    // Vista
    cmd!("view_columns", ic::COLUMNS, "Ctrl+1", Nothing),
    cmd!("view_details", ic::LIST_BULLETS, "Ctrl+2", Nothing),
    cmd!("flat", ic::STACK, "Ctrl+Shift+L", Nothing),
    cmd!("view_grid", ic::SQUARES_FOUR, "Ctrl+3", Nothing),
    cmd!("view_default", ic::CHECK_CIRCLE, "", Nothing),
    cmd!("split", ic::SQUARE_SPLIT_HORIZONTAL, "Ctrl+\\", Nothing),
    cmd!("preview", ic::SIDEBAR_SIMPLE, "Ctrl+P", Nothing),
    cmd!("hidden", ic::EYE, "Ctrl+H", Nothing),
    cmd!("space_bars", ic::CHART_PIE_SLICE, "", Nothing),
    cmd!("theme", ic::MOON, "", Nothing),
    cmd!("language", ic::TRANSLATE, "", Nothing),
    cmd!("new_tab", ic::PLUS, "Ctrl+T", Nothing),
    cmd!("close_tab", ic::X, "Ctrl+W", Nothing),
    cmd!("refresh", ic::ARROW_CLOCKWISE, "F5", Nothing),
    cmd!("search", ic::MAGNIFYING_GLASS, "Ctrl+F", Nothing),
    cmd!("quit", ic::POWER, "Ctrl+Q", Nothing),
    cmd!("resident", ic::LIGHTNING, "", Nothing),
    cmd!("install", ic::DOWNLOAD_SIMPLE, "", Nothing),
    cmd!("default_explorer", ic::FOLDER_OPEN, "", Nothing),
    cmd!("autostart", ic::ROCKET_LAUNCH, "", Nothing),
    cmd!("service", ic::LIGHTNING, "", Nothing),
];

pub fn get(id: &str) -> &'static Command {
    ALL.iter().find(|c| c.id == id).unwrap_or(&ALL[0])
}

/// Comandos que no tienen sentido dentro de un comprimido (escriben, o necesitan una carpeta real).
const WRITES: &[&str] = &[
    "cut", "paste", "rename", "batch_rename", "new_folder", "delete", "compress", "native_menu", "move_other", "properties", "open_with", "explorer", "terminal",
    "space", "dupes_here", "dupes_drive", "compare", "compare_other", "paste_link", "link_other",
];

pub fn available(c: &Command, t: &Target) -> bool {
    // Dentro de un comprimido todo es de solo lectura.
    if WRITES.contains(&c.id) && crate::fsx::archive::split(&t.dir).is_some() {
        return false;
    }
    match c.needs {
        Needs::Nothing => true,
        Needs::Selection => !t.paths.is_empty(),
        Needs::One => t.paths.len() == 1,
        Needs::Split => t.split,
        Needs::SelectionSplit => t.split && !t.paths.is_empty(),
        Needs::Archives => {
            use crate::fsx::archive;
            !t.paths.is_empty() && t.paths.iter().all(|p| archive::is_archive_name(p) || archive::is_inside(p))
        }
    }
}

/// Un elemento de menú a partir del catálogo. Devuelve true si se eligió.
pub fn menu_item(ui: &mut egui::Ui, id: &'static str, t: &Target, actions: &mut Vec<Action>) -> bool {
    let c = get(id);
    if !available(c, t) {
        return false;
    }
    let r = ui.add(egui::Button::new(format!("{}  {}", c.icon, c.label())).shortcut_text(c.keys()).min_size(vec2(230.0, 0.0)));
    if r.clicked() {
        actions.push(Action::Command(id, t.clone()));
        return true;
    }
    false
}

/// Puntaje de coincidencia en la paleta: las palabras del texto deben aparecer
/// (sin acentos) en el nombre o en las palabras extra; mejor si es al inicio.
fn score(c: &Command, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let label = search_key(c.label());
    // Las palabras del idioma y el id en inglés ("copy_path"), para encontrarlo en cualquier idioma.
    let words = search_key(&format!("{} {}", c.words(), c.id.replace('_', " ")));
    let mut s = 0;
    for q in query.split_whitespace() {
        if label.starts_with(q) {
            s += 30;
        } else if label.split(' ').any(|w| w.starts_with(q)) {
            s += 20;
        } else if label.contains(q) {
            s += 10;
        } else if words.contains(q) {
            s += 5;
        } else {
            return None;
        }
    }
    Some(s)
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub text: String,
    pub cursor: usize,
    pub first_frame: bool,
}

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.text.clear();
        self.cursor = 0;
        self.first_frame = true;
    }

    /// Dibuja la paleta; empuja la acción elegida.
    pub fn ui(&mut self, ctx: &egui::Context, theme: &Theme, target: &Target, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let query = search_key(self.text.trim());
        // "C:\algo": ofrecer ir directo a esa ruta.
        let path_like = self.text.trim().len() > 2 && self.text.trim().as_bytes().get(1) == Some(&b':');
        let mut list: Vec<(&Command, i32)> = ALL.iter().filter(|c| available(c, target)).filter_map(|c| score(c, &query).map(|s| (c, s))).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1));
        let total = list.len() + usize::from(path_like);
        let mut moved = false;
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) && total > 0 {
            self.cursor = (self.cursor + 1).min(total - 1);
            moved = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            self.cursor = self.cursor.saturating_sub(1);
            moved = true;
        }
        self.cursor = self.cursor.min(total.saturating_sub(1));
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return;
        }
        let mut run: Option<Action> = None;
        let screen = ctx.content_rect();
        let area = egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x - 280.0, screen.top() + 90.0)).show(ctx, |ui| {
            egui::Frame::new().fill(theme.chrome).stroke(egui::Stroke::new(1.0, theme.border)).corner_radius(12).inner_margin(10).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                ui.set_width(560.0);
                let mut f = super::widgets::Field::new("palette-text", if path_like { ic::FOLDER_OPEN } else { ic::COMMAND }, t!("paleta.escribe"));
                f.height = 42.0;
                let r = super::widgets::field(ui, theme, &mut self.text, f).edit;
                if self.first_frame {
                    r.request_focus();
                    self.first_frame = false;
                }
                if r.changed() {
                    self.cursor = 0;
                }
                ui.add_space(6.0);
                let mut k = 0usize;
                if path_like {
                    let label = format!("{}  {}", ic::FOLDER_OPEN, tf!("paleta.ir_a", ruta = self.text.trim()));
                    if row(ui, theme, &label, "", k == self.cursor).clicked() || (enter && k == self.cursor) {
                        run = Some(Action::Navigate(self.text.trim().trim_matches('"').to_string()));
                    }
                    k += 1;
                }
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (c, _) in &list {
                        let label = format!("{}  {}", c.icon, c.label());
                        let resp = row(ui, theme, &label, &c.keys(), k == self.cursor);
                        if moved && k == self.cursor {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() || (enter && k == self.cursor) {
                            run = Some(Action::Command(c.id, target.clone()));
                        }
                        k += 1;
                    }
                });
                if total == 0 {
                    ui.label(egui::RichText::new(t!("paleta.sin_coincidencias")).color(theme.faint));
                }
            });
        });
        let inside = area.response.rect;
        if let Some(a) = run {
            actions.push(a);
            self.open = false;
        }
        // Clic fuera: cerrar.
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !inside.contains(p)));
        if outside {
            self.open = false;
        }
    }
}

fn row(ui: &mut egui::Ui, t: &Theme, label: &str, keys: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, t.sel);
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, t.hover);
    }
    let c = if selected { t.accent_text } else { t.text };
    p.text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), c);
    if !keys.is_empty() {
        p.text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, keys, FontId::proportional(11.5), if selected { c } else { t.faint });
    }
    resp
}
