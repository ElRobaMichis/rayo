//! Catálogo de comandos: cada acción de Rayo con su nombre, ícono, atajo y en
//! qué contexto tiene sentido. De aquí salen la paleta (Ctrl+K) y el menú de
//! clic derecho, así que una función nueva aparece en los dos sin duplicar nada.

use super::actions::Action;
use super::design::{self, DialogW, IconBtn, Row, Sel, Tone, Trailing, h, icon, radius, sp, stroke, text};
use super::theme::Theme;
use crate::index::store::search_key;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align2, Id, Key, Modifiers, Rect, Response, RichText, Sense, Ui, pos2, vec2};
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
    cmd!("terminal_panel", ic::TERMINAL_WINDOW, "Ctrl+`", Nothing),
    cmd!("explorer", ic::FOLDER_OPEN, "", Nothing),
    cmd!("favorite", ic::STAR, "", Nothing),
    cmd!("save_workspace", ic::BOOKMARKS, "", Nothing),
    cmd!("add_alias", ic::AT, "", Nothing),
    cmd!("assign_shortcut", ic::KEYBOARD, "", Nothing),
    cmd!("customize_toolbar", ic::SLIDERS_HORIZONTAL, "", Nothing),
    cmd!("add_to_shelf", ic::TRAY, "", Selection),
    cmd!("new_collection", ic::BOOKMARKS_SIMPLE, "", Selection),
    // Vista
    cmd!("view_columns", ic::COLUMNS, "Ctrl+1", Nothing),
    cmd!("view_details", ic::LIST_BULLETS, "Ctrl+2", Nothing),
    cmd!("flat", ic::STACK, "Ctrl+Shift+L", Nothing),
    cmd!("view_grid", ic::SQUARES_FOUR, "Ctrl+3", Nothing),
    cmd!("view_default", ic::CHECK_CIRCLE, "", Nothing),
    cmd!("split", ic::SQUARE_SPLIT_HORIZONTAL, "Ctrl+\\", Nothing),
    cmd!("quad", ic::SQUARES_FOUR, "Ctrl+Shift+\\", Nothing),
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
    /// Nombre de una colección nueva para estos elementos.
    NewCollection(Vec<String>),
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

    /// Abrir pidiendo el nombre de una colección nueva.
    pub fn ask_collection(&mut self, paths: Vec<String>) {
        self.show();
        self.mode = Mode::NewCollection(paths);
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
            Mode::SaveWorkspace | Mode::AddAlias | Mode::Record(..) | Mode::NewCollection(_) => {}
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
        let (field_icon, hint) = match &self.mode {
            Mode::SaveWorkspace => (ic::BOOKMARKS, t!("paleta.nombre_espacio").to_string()),
            Mode::AddAlias => (ic::AT, tf!("paleta.alias_para", carpeta = crate::fsx::file_name(&target.dir))),
            Mode::PickCommand => (ic::KEYBOARD, t!("paleta.elige_comando_atajo").to_string()),
            Mode::NewCollection(_) => (ic::BOOKMARKS_SIMPLE, t!("paleta.nombre_coleccion").to_string()),
            _ => (if entries.first().is_some_and(|e| matches!(e.0, Entry::Go(_))) && !raw.is_empty() { ic::FOLDER_OPEN } else { ic::COMMAND }, t!("paleta.escribe").to_string()),
        };
        let screen = ctx.content_rect();
        let area = egui::Area::new(Id::new("palette")).order(egui::Order::Foreground).fixed_pos(pos2(screen.center().x - WIDTH / 2.0, screen.top() + TOP)).show(ctx, |ui| {
            veil(ui, theme, screen);
            frame(theme).show(ui, |ui| {
                ui.set_width(WIDTH - sp::S - FRAME_RIGHT);
                let r = input(ui, theme, &mut self.text, field_icon, &hint);
                if self.first_frame {
                    r.request_focus();
                    self.first_frame = false;
                }
                if r.changed() {
                    self.cursor = 0;
                }
                if let Mode::NewCollection(paths) = &self.mode {
                    help(ui, theme, t!("paleta.enter_crea_coleccion"));
                    if enter && !raw.is_empty() {
                        run = Some(Action::AddToCollection(raw.clone(), paths.clone()));
                    }
                    return;
                }
                match self.mode {
                    Mode::SaveWorkspace | Mode::AddAlias => {
                        let what = if self.mode == Mode::SaveWorkspace { t!("paleta.enter_guarda_espacio") } else { t!("paleta.enter_guarda_alias") };
                        help(ui, theme, what);
                        if enter && !raw.is_empty() {
                            run = Some(if self.mode == Mode::SaveWorkspace { Action::SaveWorkspace(raw.clone()) } else { Action::SetAlias(raw.clone(), target.dir.clone()) });
                        }
                        return;
                    }
                    _ => {}
                }
                ui.add_space(sp::XS);
                // La lista va a todo el ancho y la barra de desplazamiento, fina, en los 6 px que
                // le reserva el área a la derecha: así no pisa el pill de la fila.
                ui.scope(|ui| {
                    ui.spacing_mut().scroll.floating_allocated_width = SCROLL_W;
                    egui::ScrollArea::vertical().max_height(LIST_ROWS * h::NAV).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        // Mismo ancho haya o no barra, para que las filas queden a plomo con el campo.
                        ui.set_width(INNER_W);
                        // «Ir a» sin la ruta: la ruta va aparte, en mono.
                        let go = tf!("paleta.ir_a", ruta = "").trim().to_string();
                        for (k, (e, _)) in entries.iter().enumerate() {
                            let (glyph, label, path, keys) = match e {
                                Entry::Cmd(c) => (c.icon, c.label().to_string(), "", shortcut_of(c.id, data.shortcuts)),
                                Entry::Go(d) => (ic::FOLDER_OPEN, go.clone(), d.as_str(), String::new()),
                                Entry::Workspace(n) => (ic::BOOKMARKS, tf!("paleta.abrir_espacio", nombre = n), "", t!("paleta.supr_borra").to_string()),
                                Entry::Alias(a, d) => (ic::AT, a.to_string(), *d, String::new()),
                            };
                            let resp = row(ui, theme, k, glyph, &label, path, &keys, &query, k == self.cursor);
                            if moved && k == self.cursor {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() || (enter && k == self.cursor) {
                                match e {
                                    Entry::Cmd(c) if self.mode == Mode::PickCommand => next_mode = Some(Mode::Record(c.id, Vec::new(), 0.0)),
                                    Entry::Cmd(c) if c.id == "save_workspace" => next_mode = Some(Mode::SaveWorkspace),
                                    Entry::Cmd(c) if c.id == "add_alias" => next_mode = Some(Mode::AddAlias),
                                    Entry::Cmd(c) if c.id == "assign_shortcut" => next_mode = Some(Mode::PickCommand),
                                    Entry::Cmd(c) if c.id == "new_collection" => next_mode = Some(Mode::NewCollection(target.paths.clone())),
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
                });
                if total == 0 {
                    let (rr, _) = ui.allocate_exact_size(vec2(INNER_W, h::NAV), Sense::hover());
                    ui.painter().text(pos2(rr.left() + TEXT_X, rr.center().y), Align2::LEFT_CENTER, t!("paleta.sin_coincidencias"), text::small(), theme.faint);
                }
                footer(ui, theme, INNER_W, sp::S, &[(&[ic::ARROW_UP, ic::ARROW_DOWN], "", t!("paleta.navegar")), (&[ic::ARROW_ELBOW_DOWN_LEFT], "", t!("paleta.ejecutar")), (&[], "esc", t!("paleta.cerrar"))]);
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
        // Un diálogo S (no el marco de la paleta): sin botones, porque captura el teclado.
        design::dialog(ctx, theme, "palette-record", DialogW::S, |ui| {
            design::dialog_header(ui, theme, ic::KEYBOARD, Tone::Accent, t!("paleta.asignar_atajo"), Some(get(id).label()));
            ui.add_space(sp::L);
            // Las teclas en `display` de acento, centradas en una tarjeta de 64.
            design::card_frame(theme).show(ui, |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0 - 2.0 * sp::M - 2.0 * stroke::HAIR), Sense::hover());
                let g = ui.painter().layout_job(keys_job(theme, &chords));
                ui.painter().galley(r.center() - g.size() / 2.0, g, theme.accent);
            });
            ui.add_space(sp::S);
            design::dialog_help(ui, theme, t!("paleta.ayuda_atajo"));
            ui.add_space(sp::L - sp::XS);
            let w = ui.available_width();
            footer(ui, theme, w, 0.0, &[(&[ic::ARROW_ELBOW_DOWN_LEFT], "", t!("conexion.guardar")), (&[ic::BACKSPACE], "", t!("paleta.borrar")), (&[], "esc", t!("comun.cancelar"))]);
        });
    }
}

/// Lo grabado: cada combinación en `display` de acento, separadas por una flecha a medio tono.
/// Sin nada todavía, «Esperando teclas…» en `faint`.
fn keys_job(t: &Theme, chords: &[String]) -> LayoutJob {
    let mut job = LayoutJob::default();
    if chords.is_empty() {
        job.append(t!("paleta.esperando_teclas"), 0.0, TextFormat::simple(text::display(), t.faint));
        return job;
    }
    for (k, c) in chords.iter().enumerate() {
        if k > 0 {
            let arrow = TextFormat { valign: egui::Align::Center, ..TextFormat::simple(design::icon_font(icon::M), t.accent.gamma_multiply(0.5)) };
            job.append(ic::ARROW_RIGHT, sp::S, arrow);
        }
        job.append(&keys_label(c), if k > 0 { sp::S } else { 0.0 }, TextFormat::simple(text::display(), t.accent));
    }
    job
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

/// Ancho de la paleta y distancia al borde de arriba.
const WIDTH: f32 = 560.0;
const TOP: f32 = 88.0;
/// Filas visibles de la lista (sin fila cortada al pie).
const LIST_ROWS: f32 = 12.0;
/// Margen derecho del marco: 2 en vez de 8, para que la barra de desplazamiento quede fuera del pill.
const FRAME_RIGHT: f32 = 2.0;
/// Lo que el área de la lista reserva para su barra flotante.
const SCROLL_W: f32 = 6.0;
/// Ancho del campo, las filas y el pie: el del marco menos 8 a cada lado.
const INNER_W: f32 = WIDTH - 2.0 * sp::S;
/// Columna del texto en las filas y en el campo: `[8][ícono 16][8]`.
const TEXT_X: f32 = sp::S + icon::M + sp::S;

/// Marco de la paleta: el de los diálogos, con margen 8 (2 a la derecha, donde va la barra).
fn frame(t: &Theme) -> egui::Frame {
    design::dialog_frame(t).inner_margin(egui::Margin { left: sp::S as i8, right: FRAME_RIGHT as i8, top: sp::S as i8, bottom: sp::S as i8 })
}

/// Velo detrás de la paleta: oscurece un poco la ventana (solo se pinta, no ataja clics).
fn veil(ui: &Ui, t: &Theme, screen: Rect) {
    let a = if t.dark { 76 } else { 41 };
    ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(a));
}

/// El campo de la paleta: 40 de alto, relleno hundido sin borde, ícono 16, texto `title` y X
/// para vaciarlo. Devuelve la respuesta del texto.
fn input(ui: &mut Ui, t: &Theme, buf: &mut String, glyph: &str, hint: &str) -> Response {
    let id = Id::new("palette-text");
    let (rect, outer) = ui.allocate_exact_size(vec2(INNER_W, h::PALETTE), Sense::click());
    ui.painter().rect_filled(rect, radius::M, t.sidebar);
    ui.painter().text(pos2(rect.left() + sp::S + icon::M / 2.0, rect.center().y), Align2::CENTER_CENTER, glyph, design::icon_font(icon::M), t.dim);
    let mut right = rect.right() - sp::S;
    let mut cleared = false;
    if !buf.is_empty() {
        let xr = Rect::from_min_max(pos2(right - h::CHIP, rect.center().y - h::CHIP / 2.0), pos2(right, rect.center().y + h::CHIP / 2.0));
        if design::icon_button_at(ui, t, xr, id.with("clear"), ic::X, IconBtn::mini()).clicked() {
            buf.clear();
            cleared = true;
        }
        right = xr.left() - sp::XS;
    }
    let er = Rect::from_min_max(pos2(rect.left() + TEXT_X, rect.top()), pos2(right, rect.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(er).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let edit = child.add(
        egui::TextEdit::singleline(buf)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(text::title())
            .hint_text(RichText::new(hint).font(text::title()).color(t.faint))
            .desired_width(er.width())
            .vertical_align(egui::Align::Center),
    );
    // Un clic en el ícono o el margen también lleva el cursor al campo.
    if outer.clicked() || cleared {
        edit.request_focus();
    }
    if outer.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    edit
}

/// Ayuda bajo el campo, en los modos que piden un nombre.
fn help(ui: &mut Ui, t: &Theme, s: &str) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(sp::S as i8, sp::S as i8)).show(ui, |ui| {
        ui.label(RichText::new(s).font(text::small()).color(t.faint));
    });
}

/// Pie: una línea fina con 4 px arriba y abajo y las teclas (glifos o tecla escrita, como «esc»)
/// con su micro-etiqueta. Lo usan la paleta (↑↓ · ↵ · Esc) y la grabación de un atajo.
fn footer(ui: &mut Ui, t: &Theme, w: f32, inset: f32, items: &[(&[&str], &str, &str)]) {
    // Los 4 de arriba son el espacio entre ítems que deja la lista.
    ui.spacing_mut().item_spacing.y = 0.0;
    let (sep, _) = ui.allocate_exact_size(vec2(w, stroke::HAIR + sp::XS), Sense::hover());
    ui.painter().hline(sep.x_range(), sep.top().round() + 0.5, egui::Stroke::new(stroke::HAIR, t.border));
    let (rect, _) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let mut x = rect.left() + inset;
    for (k, (glyphs, key, label)) in items.iter().enumerate() {
        if k > 0 {
            x += sp::L;
        }
        for g in glyphs.iter() {
            x = p.text(pos2(x, cy), Align2::LEFT_CENTER, *g, design::icon_font(icon::S), t.faint).right();
        }
        if !key.is_empty() {
            x = design::micro(p, pos2(x, cy), Align2::LEFT_CENTER, key, t.faint).right();
        }
        x = design::micro(p, pos2(x + sp::XS, cy), Align2::LEFT_CENTER, label, t.faint).right();
    }
}

/// Lo escrito, resaltado en el nombre: cada palabra en acento y `strong`.
fn highlight(label: &str, query: &str, t: &Theme, max_w: f32) -> LayoutJob {
    super::theme::want_cjk(label);
    let (key, map) = crate::index::store::search_key_with_map(label);
    let mut marks = vec![false; label.len()];
    for w in query.split_whitespace() {
        if let Some(k) = key.find(w) {
            marks[map[k]..map[k + w.len()]].fill(true);
        }
    }
    let fmt = |on: bool| if on { TextFormat::simple(text::strong(), t.accent) } else { TextFormat::simple(text::body(), t.text) };
    let mut job = LayoutJob::default();
    let (mut start, mut on) = (0, false);
    for (i, _) in label.char_indices() {
        if marks[i] != on {
            if i > start {
                job.append(&label[start..i], 0.0, fmt(on));
            }
            (start, on) = (i, marks[i]);
        }
    }
    if start < label.len() {
        job.append(&label[start..], 0.0, fmt(on));
    }
    job.wrap = TextWrapping::truncate_at_width(max_w.max(0.0));
    job
}

/// Una fila de la paleta (`NAV`, la fila de 4.1 sin margen): ícono en su columna, el nombre con
/// lo escrito resaltado, la ruta (si hay) detrás en mono `dim` y el atajo a la derecha. La del
/// cursor, con el tinte de selección.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, t: &Theme, k: usize, glyph: &str, label: &str, path: &str, keys: &str, query: &str, selected: bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::NAV), Sense::hover());
    let spec = Row {
        trailing: if keys.is_empty() { Trailing::None } else { Trailing::Keys(keys) },
        sel: if selected { Sel::Focused } else { Sel::None },
        height: h::NAV,
        ..Row::new(glyph, "")
    };
    let id = ui.id().with(("palette-row", k));
    let resp = design::row_at(ui, t, rect, id, spec);
    // El nombre aparte, para poder resaltar lo buscado.
    let p = ui.painter();
    let right = rect.right() - sp::S - if keys.is_empty() { 0.0 } else { design::text_width(p, keys, text::mono_s()) + sp::S };
    let x = rect.left() + TEXT_X;
    let g = p.layout_job(highlight(label, query, t, right - x));
    let px = x + g.size().x + sp::S;
    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, t.text);
    // La ruta, acortada por el medio en lo que queda.
    if !path.is_empty() && right - px > 2.0 * h::NAV {
        let s = design::elide_middle(p, path, text::mono(), right - px);
        p.text(pos2(px, rect.center().y), Align2::LEFT_CENTER, s, text::mono(), t.dim);
    }
    resp
}
