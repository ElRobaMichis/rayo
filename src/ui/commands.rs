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
    cmd!("unblock", ic::LOCK_SIMPLE_OPEN, "", Nothing),
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
    cmd!("save_workspace", ic::BOOKMARKS, "", Nothing),
    cmd!("add_alias", ic::AT, "", Nothing),
    cmd!("assign_shortcut", ic::KEYBOARD, "", Nothing),
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

/// Lo que la paleta necesita de la configuración para sus entradas dinámicas.
pub struct PaletteData<'a> {
    pub workspaces: &'a [super::config::Workspace],
    pub recent: &'a [String],
    pub aliases: &'a [(String, String)],
    pub shortcuts: &'a [(String, String)],
}

/// Qué está pidiendo la paleta.
#[derive(Default, Clone, PartialEq)]
pub enum Mode {
    /// Buscar y ejecutar (comandos, espacios de trabajo, carpetas recientes, alias).
    #[default]
    Commands,
    /// Nombre para guardar las pestañas como espacio de trabajo.
    SaveWorkspace,
    /// Alias para la carpeta actual.
    AddAlias,
    /// Elegir a qué comando asignarle un atajo.
    PickCommand,
    /// Grabando el atajo de este comando: las combinaciones apretadas y cuándo la última.
    Record(&'static str, Vec<String>, f64),
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub text: String,
    pub cursor: usize,
    pub first_frame: bool,
    pub mode: Mode,
}

/// Una fila de la paleta.
enum Entry<'a> {
    Cmd(&'static Command),
    Go(String),
    Workspace(&'a str),
    Alias(&'a str, &'a str),
}

/// El atajo de un comando: el propio del usuario si lo asignó, si no el de siempre.
pub fn shortcut_of(id: &str, shortcuts: &[(String, String)]) -> String {
    shortcuts.iter().find(|(_, c)| c == id).map(|(k, _)| keys_label(k)).unwrap_or_else(|| get(id).keys())
}

/// La combinación de una tecla apretada ("Ctrl+Shift+D"); None si es solo un modificador.
pub fn chord(key: Key, m: Modifiers) -> String {
    let mut parts = Vec::new();
    if m.ctrl || m.command {
        parts.push("Ctrl");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    let name = key.name();
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(name);
    s
}

impl Palette {
    pub fn show(&mut self) {
        self.open = true;
        self.text.clear();
        self.cursor = 0;
        self.first_frame = true;
        self.mode = Mode::Commands;
    }

    fn enter_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.text.clear();
        self.cursor = 0;
        self.first_frame = true;
    }

    /// Dibuja la paleta; empuja la acción elegida.
    pub fn ui(&mut self, ctx: &egui::Context, theme: &Theme, target: &Target, data: &PaletteData, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return;
        }
        // Grabando un atajo: no hay texto, se leen las teclas.
        if let Mode::Record(id, chords, last) = self.mode.clone() {
            return self.record_ui(ctx, theme, id, chords, last, actions);
        }
        let raw = self.text.trim().to_string();
        let query = search_key(&raw);
        let mut entries: Vec<(Entry, i32)> = Vec::new();
        match &self.mode {
            Mode::Commands => {
                // "C:\algo" o "\\servidor\algo": ofrecer ir directo a esa ruta.
                if raw.len() > 2 && (raw.as_bytes().get(1) == Some(&b':') || raw.starts_with(r"\\")) {
                    entries.push((Entry::Go(raw.trim_matches('"').to_string()), 1000));
                }
                // Un alias escrito tal cual va primero.
                for (a, dir) in data.aliases {
                    let k = search_key(a);
                    if !query.is_empty() && k == query {
                        entries.push((Entry::Alias(a, dir), 900));
                    } else if !query.is_empty() && k.starts_with(&query) {
                        entries.push((Entry::Alias(a, dir), 60));
                    }
                }
                for w in data.workspaces {
                    if let Some(sc) = score_text(&w.name, t!("paleta.espacio_de_trabajo"), &query) {
                        entries.push((Entry::Workspace(&w.name), sc + 1));
                    }
                }
                entries.extend(ALL.iter().filter(|c| available(c, target)).filter_map(|c| score(c, &query).map(|s| (Entry::Cmd(c), s))));
                // Carpetas recientes ("GoTo"): con algo escrito, las que coinciden; sin nada, las primeras.
                let mut n = 0;
                for (rank, d) in data.recent.iter().enumerate() {
                    let name = crate::fsx::file_name(d);
                    let hit = if query.is_empty() { (rank < 6).then_some(0) } else { score_text(&name, d, &query) };
                    if let Some(sc) = hit {
                        entries.push((Entry::Go(d.clone()), sc - 2 - rank as i32 / 4));
                        n += 1;
                        if n >= 8 {
                            break;
                        }
                    }
                }
            }
            Mode::PickCommand => {
                entries.extend(ALL.iter().filter_map(|c| score(c, &query).map(|s| (Entry::Cmd(c), s))));
            }
            Mode::SaveWorkspace | Mode::AddAlias | Mode::Record(..) => {}
        }
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        let total = entries.len();
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
        // Supr sobre un espacio de trabajo: borrarlo.
        let del = self.mode == Mode::Commands && matches!(entries.get(self.cursor), Some((Entry::Workspace(_), _))) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete));
        let mut run: Option<Action> = None;
        let mut next_mode: Option<Mode> = None;
        let (icon, hint) = match &self.mode {
            Mode::SaveWorkspace => (ic::BOOKMARKS, t!("paleta.nombre_espacio").to_string()),
            Mode::AddAlias => (ic::AT, tf!("paleta.alias_para", carpeta = crate::fsx::file_name(&target.dir))),
            Mode::PickCommand => (ic::KEYBOARD, t!("paleta.elige_comando_atajo").to_string()),
            _ => (if entries.first().is_some_and(|e| matches!(e.0, Entry::Go(_))) && !raw.is_empty() { ic::FOLDER_OPEN } else { ic::COMMAND }, t!("paleta.escribe").to_string()),
        };
        let screen = ctx.content_rect();
        let area = egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x - 280.0, screen.top() + 90.0)).show(ctx, |ui| {
            egui::Frame::new().fill(theme.chrome).stroke(egui::Stroke::new(1.0, theme.border)).corner_radius(12).inner_margin(10).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                ui.set_width(560.0);
                let mut f = super::widgets::Field::new("palette-text", icon, &hint);
                f.height = 42.0;
                let r = super::widgets::field(ui, theme, &mut self.text, f).edit;
                if self.first_frame {
                    r.request_focus();
                    self.first_frame = false;
                }
                if r.changed() {
                    self.cursor = 0;
                }
                match self.mode {
                    Mode::SaveWorkspace | Mode::AddAlias => {
                        ui.add_space(6.0);
                        let what = if self.mode == Mode::SaveWorkspace { t!("paleta.enter_guarda_espacio") } else { t!("paleta.enter_guarda_alias") };
                        ui.label(egui::RichText::new(what).size(12.0).color(theme.faint));
                        if enter && !raw.is_empty() {
                            run = Some(if self.mode == Mode::SaveWorkspace { Action::SaveWorkspace(raw.clone()) } else { Action::SetAlias(raw.clone(), target.dir.clone()) });
                        }
                        return;
                    }
                    _ => {}
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (k, (e, _)) in entries.iter().enumerate() {
                        let (label, keys) = match e {
                            Entry::Cmd(c) => (format!("{}  {}", c.icon, c.label()), shortcut_of(c.id, data.shortcuts)),
                            Entry::Go(d) => (format!("{}  {}", ic::FOLDER_OPEN, tf!("paleta.ir_a", ruta = d)), String::new()),
                            Entry::Workspace(n) => (format!("{}  {}", ic::BOOKMARKS, tf!("paleta.abrir_espacio", nombre = n)), t!("paleta.supr_borra").to_string()),
                            Entry::Alias(a, d) => (format!("{}  {a}  →  {}", ic::AT, d), String::new()),
                        };
                        let resp = row(ui, theme, &label, &keys, k == self.cursor);
                        if moved && k == self.cursor {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() || (enter && k == self.cursor) {
                            match e {
                                Entry::Cmd(c) if self.mode == Mode::PickCommand => next_mode = Some(Mode::Record(c.id, Vec::new(), 0.0)),
                                Entry::Cmd(c) if c.id == "save_workspace" => next_mode = Some(Mode::SaveWorkspace),
                                Entry::Cmd(c) if c.id == "add_alias" => next_mode = Some(Mode::AddAlias),
                                Entry::Cmd(c) if c.id == "assign_shortcut" => next_mode = Some(Mode::PickCommand),
                                Entry::Cmd(c) => run = Some(Action::Command(c.id, target.clone())),
                                Entry::Go(d) => run = Some(Action::Navigate(d.clone())),
                                Entry::Workspace(n) => run = Some(Action::OpenWorkspace(n.to_string())),
                                Entry::Alias(_, d) => run = Some(Action::Navigate(d.to_string())),
                            }
                        }
                        if del && k == self.cursor {
                            if let Entry::Workspace(n) = e {
                                run = Some(Action::DeleteWorkspace(n.to_string()));
                            }
                        }
                    }
                });
                if total == 0 {
                    ui.label(egui::RichText::new(t!("paleta.sin_coincidencias")).color(theme.faint));
                }
            });
        });
        let inside = area.response.rect;
        if let Some(m) = next_mode {
            self.enter_mode(m);
            return;
        }
        if let Some(a) = run {
            // Borrar un espacio no cierra la paleta (para seguir eligiendo).
            let keep = matches!(a, Action::DeleteWorkspace(_));
            actions.push(a);
            if !keep {
                self.open = false;
            }
        }
        // Clic fuera: cerrar.
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !inside.contains(p)));
        if outside {
            self.open = false;
        }
    }

    /// Grabar un atajo: cada combinación apretada se suma (hasta 3, para secuencias como
    /// "Ctrl+K D"); se guarda al dejar de apretar un momento o con Enter.
    fn record_ui(&mut self, ctx: &egui::Context, theme: &Theme, id: &'static str, mut chords: Vec<String>, mut last: f64, actions: &mut Vec<Action>) {
        let now = ctx.input(|i| i.time);
        let pressed: Vec<(Key, Modifiers)> = ctx.input_mut(|i| {
            let mut v = Vec::new();
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } => {
                    v.push((*key, *modifiers));
                    false
                }
                egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => false,
                _ => true,
            });
            v
        });
        let mut save = false;
        for (k, m) in pressed {
            if k == Key::Enter && m.is_none() {
                save = !chords.is_empty();
                continue;
            }
            if k == Key::Backspace && m.is_none() {
                chords.clear();
                continue;
            }
            if chords.len() < 3 {
                chords.push(chord(k, m));
                last = now;
            }
        }
        if !chords.is_empty() && now - last > 1.2 {
            save = true;
        }
        if save {
            actions.push(Action::SetShortcut(chords.join(" "), id.to_string()));
            self.open = false;
            return;
        }
        if !chords.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.mode = Mode::Record(id, chords.clone(), last);
        let screen = ctx.content_rect();
        egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x - 280.0, screen.top() + 90.0)).show(ctx, |ui| {
            egui::Frame::new().fill(theme.chrome).stroke(egui::Stroke::new(1.0, theme.border)).corner_radius(12).inner_margin(16).shadow(ui.visuals().popup_shadow).show(ui, |ui| {
                ui.set_width(548.0);
                ui.label(egui::RichText::new(format!("{}  {}", ic::KEYBOARD, tf!("paleta.presiona_atajo", comando = get(id).label()))).size(15.0).color(theme.text));
                ui.add_space(10.0);
                let shown = if chords.is_empty() { t!("paleta.esperando_teclas").to_string() } else { keys_label(&chords.join(" ")) };
                ui.label(egui::RichText::new(shown).font(super::theme::semibold(20.0)).color(if chords.is_empty() { theme.faint } else { theme.accent }));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(t!("paleta.ayuda_atajo")).size(12.0).color(theme.faint));
            });
        });
    }
}

/// Puntaje de un texto (nombre y palabras extra) para lo escrito.
fn score_text(name: &str, extra: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let label = search_key(name);
    let words = search_key(extra);
    let mut s = 0;
    for q in query.split_whitespace() {
        if label.starts_with(q) {
            s += 25;
        } else if label.contains(q) {
            s += 12;
        } else if words.contains(q) {
            s += 4;
        } else {
            return None;
        }
    }
    Some(s)
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
