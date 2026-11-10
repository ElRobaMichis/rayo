//! Menú de clic derecho unificado: las opciones de Rayo y las de Windows y otras apps
//! (7-Zip, VLC, "Enviar a", "Abrir con"…) en un solo menú, con el estilo de Rayo.
//!
//! - Va en ventanas propias (el menú y cada submenú), así que sale por fuera de la
//!   ventana de Rayo como un menú de verdad, ajustado a los bordes de cada monitor.
//! - Arriba, una búsqueda: escribir filtra todas las opciones, también las de submenús.
//! - Una estrella en cada opción la fija arriba de todo; un ojo tachado oculta las
//!   opciones de Windows que no se usan (se pueden volver a mostrar al final del menú).
//! - Sin duplicados: si Rayo ya hace algo (copiar, eliminar…), no se repite lo de Windows.

use super::actions::Action;
use super::commands::{self, Target};
use super::design::{self, Sel, h, icon, radius, sp, text};
use super::theme::Theme;
use crate::fsx::ctxmenu::{ShellItem, ShellMenu};
use crate::fsx::thumbs::Rgba;
use crate::index::store::search_key;
use egui::{Align2, Color32, Id, Rect, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use std::sync::Arc;

pub enum Icon {
    Glyph(&'static str),
    /// Un círculo de color (etiqueta `i`).
    Color(usize),
    Image(Arc<Rgba>, String),
    None,
}

#[derive(Clone)]
pub enum Run {
    Cmd(&'static str, Target),
    Act(Action),
    Shell(u64, u32),
}

/// Una opción del menú (de Rayo o de Windows).
pub struct Entry {
    /// Para fijarla u ocultarla ("cmd:copy", "shell:7-zip.extract"…); vacío = ninguna de las dos.
    pub key: String,
    pub icon: Icon,
    pub label: String,
    pub keys: String,
    pub run: Option<Run>,
    pub children: Vec<Entry>,
    pub disabled: bool,
    pub separator: bool,
    /// Se puede ocultar (las de Windows).
    pub hideable: bool,
    /// Está oculta (solo se ve si se pidió ver las ocultas).
    pub hidden: bool,
    /// Marcada (etiqueta ya puesta): check de 12 en acento a la derecha.
    pub checked: bool,
}

impl Entry {
    fn blank() -> Entry {
        Entry { key: String::new(), icon: Icon::None, label: String::new(), keys: String::new(), run: None, children: Vec::new(), disabled: false, separator: false, hideable: false, hidden: false, checked: false }
    }

    pub fn sep() -> Entry {
        Entry { separator: true, ..Entry::blank() }
    }

    /// Un comando de Rayo (si tiene sentido para lo elegido).
    pub fn cmd(id: &'static str, t: &Target) -> Option<Entry> {
        let c = commands::get(id);
        commands::available(c, t).then(|| Entry {
            key: format!("cmd:{id}"),
            icon: Icon::Glyph(c.icon),
            label: c.label().to_string(),
            keys: c.keys(),
            run: Some(Run::Cmd(id, t.clone())),
            ..Entry::blank()
        })
    }

    pub fn act(key: &str, icon: &'static str, label: &str, a: Action) -> Entry {
        Entry { key: key.to_string(), icon: Icon::Glyph(icon), label: label.to_string(), run: Some(Run::Act(a)), ..Entry::blank() }
    }

    /// Un submenú (None si quedó vacío).
    pub fn sub(icon: &'static str, label: &str, children: Vec<Entry>) -> Option<Entry> {
        let children = tidy(children);
        (!children.is_empty()).then(|| Entry { icon: Icon::Glyph(icon), label: label.to_string(), children, ..Entry::blank() })
    }
}

/// Sin separadores al principio, al final ni repetidos.
pub fn tidy(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for e in entries {
        if e.separator && out.last().is_none_or(|l| l.separator) {
            continue;
        }
        out.push(e);
    }
    while out.last().is_some_and(|l| l.separator) {
        out.pop();
    }
    out
}

fn push(v: &mut Vec<Entry>, e: Option<Entry>) {
    if let Some(e) = e {
        v.push(e);
    }
}

// ---------------------------------------------------------------- qué menú

/// Qué se pidió con el clic derecho.
#[derive(Clone, Debug)]
pub enum MenuReq {
    /// Archivos o carpetas (de la misma carpeta `dir`). `reveal`: "Mostrar en su carpeta" (búsqueda).
    Items { paths: Vec<String>, is_dir: bool, dir: String, reveal: Option<String> },
    /// El fondo de una carpeta.
    Background { dir: String },
}

impl MenuReq {
    /// Para el menú de Windows: los elementos (o la carpeta) y si es el fondo.
    pub fn shell_key(&self) -> (Vec<String>, bool) {
        match self {
            MenuReq::Items { paths, .. } => (paths.clone(), false),
            MenuReq::Background { dir } => (vec![dir.clone()], true),
        }
    }
}

/// Lo que hace falta para armar el menú.
pub struct BuildCx<'a> {
    pub groups: &'a [String],
    pub split: bool,
    pub native: Option<&'a ShellMenu>,
    pub hidden: &'a [String],
    pub show_hidden: bool,
}

/// Las opciones de Rayo y las de Windows (None mientras se cargan) y cuántas hay ocultas.
pub fn build(req: &MenuReq, b: &BuildCx) -> (Vec<Entry>, Option<Vec<Entry>>, usize) {
    let (rayo, t) = match req {
        MenuReq::Items { paths, is_dir, dir, reveal } => item_entries(paths, *is_dir, dir, reveal.as_deref(), b),
        MenuReq::Background { dir } => background_entries(dir, b),
    };
    let mut hidden_count = 0;
    // Las opciones de Rayo también se pueden ocultar (como las de Windows).
    let rayo = hide_own(rayo, b.hidden, b.show_hidden, &mut hidden_count);
    let native = b.native.map(|m| {
        let mut v = native_entries(m, b.hidden, b.show_hidden, &mut hidden_count);
        if matches!(req, MenuReq::Items { .. }) {
            v.push(Entry::sep());
            push(&mut v, Entry::cmd("native_menu", &t));
        }
        tidy(v)
    });
    (tidy(rayo), native, hidden_count)
}

/// Marca como ocultables las opciones de Rayo y salta (o marca, si se pidió verlas) las ocultas.
fn hide_own(entries: Vec<Entry>, hidden: &[String], show_hidden: bool, count: &mut usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for mut e in entries {
        if e.separator {
            out.push(e);
            continue;
        }
        if !e.children.is_empty() {
            e.children = tidy(hide_own(std::mem::take(&mut e.children), hidden, show_hidden, count));
            if e.children.is_empty() {
                continue;
            }
            out.push(e);
            continue;
        }
        if !e.key.is_empty() {
            e.hideable = true;
            if hidden.contains(&e.key) {
                *count += 1;
                if !show_hidden {
                    continue;
                }
                e.hidden = true;
            }
        }
        out.push(e);
    }
    tidy(out)
}

fn item_entries(paths: &[String], is_dir: bool, dir: &str, reveal: Option<&str>, b: &BuildCx) -> (Vec<Entry>, Target) {
    let t = Target { paths: paths.to_vec(), dir: dir.to_string(), single_dir: paths.len() == 1 && is_dir, split: b.split };
    // Con 7-Zip se usa el suyo para extraer / comprimir; con "Abrir con" de Windows, ese.
    let seven = b.native.is_some_and(has_7zip);
    let open_with = b.native.is_some_and(has_open_with);
    let mut e = Vec::new();
    if let Some(p) = reveal {
        let mut r = Entry::act("act:reveal", ic::FOLDER_OPEN, t!("comun.mostrar_su_carpeta"), Action::Reveal(p.to_string()));
        r.keys = "Ctrl+Enter".into();
        e.push(r);
        e.push(Entry::sep());
    }
    push(&mut e, Entry::cmd("open", &t));
    if is_dir {
        push(&mut e, Entry::cmd("open_tab", &t));
    } else if !open_with {
        push(&mut e, Entry::cmd("open_with", &t));
    }
    push(&mut e, Entry::cmd("quicklook", &t));
    e.push(Entry::sep());
    push(&mut e, Entry::cmd("copy", &t));
    push(&mut e, Entry::cmd("cut", &t));
    if t.single_dir {
        push(&mut e, Entry::cmd("paste", &Target { dir: paths[0].clone(), ..t.clone() }));
    }
    push(&mut e, Entry::cmd("copy_path", &t));
    if t.split {
        push(&mut e, Entry::sub(ic::SQUARE_SPLIT_HORIZONTAL, t!("menu.otro_panel"), [Entry::cmd("copy_other", &t), Entry::cmd("move_other", &t)].into_iter().flatten().collect()));
    }
    e.push(Entry::sep());
    if paths.len() == 1 {
        push(&mut e, Entry::cmd("rename", &t));
    }
    push(&mut e, Entry::cmd("batch_rename", &t));
    if !seven {
        push(&mut e, Entry::cmd("extract_here", &t));
        push(&mut e, Entry::cmd("extract_folder", &t));
        push(&mut e, Entry::cmd("compress", &t));
    }
    if !is_dir {
        push(&mut e, Entry::cmd("hash", &t));
    }
    // Colecciones: agregar o quitar (✓ si todo lo elegido ya está), o una nueva.
    {
        let mut subs: Vec<Entry> = crate::fsx::collections::list()
            .into_iter()
            .map(|(name, _)| {
                let all = crate::fsx::collections::contains_all(&name, paths);
                let a = if all { Action::RemoveFromCollection(name.clone(), paths.to_vec()) } else { Action::AddToCollection(name.clone(), paths.to_vec()) };
                Entry::act(&format!("coll:{name}"), if all { ic::CHECK } else { ic::BOOKMARKS_SIMPLE }, &name, a)
            })
            .collect();
        if !subs.is_empty() {
            subs.push(Entry::sep());
        }
        subs.push(Entry::act("coll-new", ic::PLUS, t!("menu.nueva_coleccion"), Action::NewCollection(paths.to_vec())));
        push(&mut e, Entry::sub(ic::BOOKMARKS_SIMPLE, t!("menu.coleccion"), subs));
        push(&mut e, Entry::cmd("add_to_shelf", &t));
    }
    push(&mut e, Entry::cmd("who_uses", &t));
    // Etiquetas de color: marcadas las que ya tiene todo lo elegido.
    let mut tags: Vec<Entry> = (0..7)
        .map(|i| {
            let mut e = Entry::act("", "", super::views::tag_label(i), Action::ToggleTag(paths.to_vec(), i));
            e.icon = Icon::Color(i);
            e.checked = paths.iter().all(|p| crate::fsx::tags::get(p) & (1 << i) != 0);
            e
        })
        .collect();
    if paths.iter().any(|p| crate::fsx::tags::get(p) != 0) {
        tags.push(Entry::sep());
        tags.push(Entry::act("", ic::X, t!("menu.quitar_etiquetas"), Action::ClearTags(paths.to_vec())));
    }
    push(&mut e, Entry::sub(ic::TAG, t!("menu.etiqueta"), tags));
    // Solo si algo de lo elegido viene de Internet (es lo que bloquea la vista previa).
    if paths.iter().take(32).any(|p| crate::fsx::props::is_blocked(p)) {
        push(&mut e, Entry::cmd("unblock", &t));
    }
    if t.split {
        push(&mut e, Entry::cmd("link_other", &t));
    }
    if t.single_dir && crate::fsx::archive::split(dir).is_none() {
        let inside = Target { dir: paths[0].clone(), paths: Vec::new(), ..t.clone() };
        let mut tools = vec![Entry::cmd("space", &inside), Entry::cmd("dupes_here", &inside)];
        if inside.split {
            tools.push(Entry::cmd("compare_other", &inside));
        }
        if !b.native.is_some_and(has_terminal) {
            tools.push(Entry::cmd("terminal", &inside));
        }
        push(&mut e, Entry::sub(ic::WRENCH, t!("menu.herramientas"), tools.into_iter().flatten().collect()));
        let favs = b.groups.iter().enumerate().map(|(gi, g)| Entry::act("", ic::FOLDER_SIMPLE, g, Action::AddFavorite(gi, paths[0].clone()))).collect();
        push(&mut e, Entry::sub(ic::STAR, t!("comun.agregar_favoritos"), favs));
    }
    push(&mut e, Entry::cmd("explorer", &t));
    e.push(Entry::sep());
    push(&mut e, Entry::cmd("delete", &t));
    push(&mut e, Entry::cmd("properties", &t));
    (e, t)
}

fn background_entries(dir: &str, b: &BuildCx) -> (Vec<Entry>, Target) {
    let t = Target { paths: Vec::new(), dir: dir.to_string(), single_dir: true, split: b.split };
    let mut e = Vec::new();
    if commands::available(commands::get("new_folder"), &t) {
        let mut new = vec![Entry::cmd("new_folder", &t)];
        if let Some(items) = crate::fsx::shellnew::items() {
            new.push(Some(Entry::sep()));
            for (k, it) in items.iter().enumerate() {
                let mut label = it.name.clone();
                if let Some(c) = label.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                let (glyph, _) = super::icons::for_entry(&Theme::get(true), false, it.ext.trim_start_matches('.'));
                let mut entry = Entry::act(&format!("nuevo:{}", it.ext), glyph, &label, Action::NewFile(dir.to_string(), k));
                entry.keys = it.ext.clone();
                new.push(Some(entry));
            }
        }
        push(&mut e, Entry::sub(ic::PLUS_CIRCLE, t!("comun.nuevo"), new.into_iter().flatten().collect()));
    }
    push(&mut e, Entry::cmd("paste", &t));
    push(&mut e, Entry::cmd("paste_link", &t));
    push(&mut e, Entry::cmd("undo", &t));
    e.push(Entry::sep());
    let view = ["view_columns", "view_details", "view_grid", "flat", "view_default", "space_bars", "hidden", "split"].into_iter().filter_map(|id| Entry::cmd(id, &t)).collect();
    push(&mut e, Entry::sub(ic::EYE, t!("menu.ver"), view));
    let tools = ["space", "dupes_here", "compare_other", "compare", "open_other"].into_iter().filter_map(|id| Entry::cmd(id, &t)).collect();
    push(&mut e, Entry::sub(ic::WRENCH, t!("menu.herramientas"), tools));
    e.push(Entry::sep());
    // Si Windows ofrece su "Abrir en Terminal", se usa ese.
    if !b.native.is_some_and(has_terminal) {
        push(&mut e, Entry::cmd("terminal", &t));
    }
    push(&mut e, Entry::cmd("explorer", &t));
    push(&mut e, Entry::cmd("refresh", &t));
    let favs = b.groups.iter().enumerate().map(|(gi, g)| Entry::act("", ic::FOLDER_SIMPLE, g, Action::AddFavorite(gi, dir.to_string()))).collect();
    push(&mut e, Entry::sub(ic::STAR, t!("comun.agregar_favoritos"), favs));
    push(&mut e, Entry::cmd("properties", &t));
    (e, t)
}

// ---------------------------------------------------------------- opciones de Windows

/// Verbos de Windows que Rayo ya tiene (o que no aplican en Rayo): no se repiten.
const HIDDEN_VERBS: &[&str] = &[
    "open", "opennewwindow", "opennewtab", "opennewprocess", "explore", "find", "cut", "copy", "paste", "pastelink", "delete", "rename", "properties", "copyaspath",
    "rayo", "pintohome", "pintohomefile", "unpinfromhome", "refresh", "undo", "redo",
];

/// Textos de Windows que Rayo ya tiene en el menú del fondo de una carpeta.
const HIDDEN_BACKGROUND: &[&str] = &["new", "nuevo", "view", "ver", "sort by", "ordenar por", "group by", "agrupar por", "refresh", "actualizar", "paste", "pegar", "undo", "deshacer", "properties", "propiedades"];

/// ¿Tiene 7-Zip? (entonces se usa el suyo en vez de extraer / comprimir de Rayo).
pub fn has_7zip(menu: &ShellMenu) -> bool {
    menu.items.iter().any(|i| i.text.contains("7-Zip"))
}

/// ¿Windows ofrece abrir la Terminal aquí? Entonces no hace falta el de Rayo.
pub fn has_terminal(menu: &ShellMenu) -> bool {
    menu.items.iter().any(|i| {
        let t = i.text.to_lowercase();
        i.verb.to_ascii_lowercase().contains("terminal") || t.contains("in terminal") || t.contains("en terminal")
    })
}

/// ¿Windows ofrece "Abrir con" (con sus apps)? Entonces no hace falta el de Rayo.
pub fn has_open_with(menu: &ShellMenu) -> bool {
    menu.items.iter().any(|i| !i.children.is_empty() && i.children.iter().any(|c| c.verb.eq_ignore_ascii_case("openas")))
}

/// Ícono para las opciones de Windows que no traen uno (None: la columna queda vacía).
fn fallback_icon(item: &ShellItem) -> Option<&'static str> {
    let v = item.verb.to_ascii_lowercase();
    let t = item.text.to_lowercase();
    Some(if t.contains("7-zip") || v.starts_with("sevenzip") {
        ic::FILE_ZIP
    } else if v == "extract" || t.contains("extra") {
        ic::ARCHIVE
    } else if t.starts_with("open with") || t.starts_with("abrir con") {
        ic::APP_WINDOW
    } else if t.starts_with("send to") || t.starts_with("enviar a") {
        ic::PAPER_PLANE_TILT
    } else if t.starts_with("give access") || t.starts_with("conceder acceso") || t.starts_with("dar acceso") {
        ic::USERS
    } else if v.contains("share") {
        ic::SHARE_NETWORK
    } else if v == "previousversions" {
        ic::CLOCK_COUNTER_CLOCKWISE
    } else if v.contains("pin") {
        ic::PUSH_PIN
    } else if v == "link" {
        ic::LINK
    } else if v == "runas" {
        ic::SHIELD_CHECK
    } else if v.contains("terminal") || t.contains("terminal") || t.contains("bash") || t.contains("powershell") {
        ic::TERMINAL_WINDOW
    } else if t.contains("crc") || t.contains("sha") {
        ic::FINGERPRINT
    } else if !item.children.is_empty() {
        ic::DOTS_THREE_CIRCLE
    } else {
        return None;
    })
}

/// Las opciones de Windows, ya filtradas, como entradas del menú. Las ocultas se saltan
/// (o se incluyen marcadas, si se pidió verlas) y se cuentan en `hidden_count`.
fn native_entries(menu: &ShellMenu, hidden: &[String], show_hidden: bool, hidden_count: &mut usize) -> Vec<Entry> {
    fn conv(items: &[ShellItem], token: u64, background: bool, parent: &str, hidden: &[String], show_hidden: bool, count: &mut usize) -> Vec<Entry> {
        let mut out = Vec::new();
        for it in items {
            if it.separator {
                out.push(Entry::sep());
                continue;
            }
            let v = it.verb.to_ascii_lowercase();
            if HIDDEN_VERBS.contains(&v.as_str()) {
                continue;
            }
            if background && parent.is_empty() && HIDDEN_BACKGROUND.contains(&it.text.to_lowercase().trim_end_matches('…').trim_end_matches("...")) {
                continue;
            }
            let path = if parent.is_empty() { it.text.clone() } else { format!("{parent} › {}", it.text) };
            let key = if it.children.is_empty() && !it.verb.is_empty() { format!("shell:{}", it.verb) } else { format!("shell:{path}") };
            let is_hidden = hidden.contains(&key);
            if is_hidden {
                *count += 1;
                if !show_hidden {
                    continue;
                }
            }
            let icon = match &it.icon {
                Some(img) => Icon::Image(img.clone(), format!("shell-icon:{token}:{path}")),
                None => fallback_icon(it).map_or(Icon::None, Icon::Glyph),
            };
            let mut e = Entry { key, icon, label: it.text.clone(), disabled: it.disabled, hideable: true, hidden: is_hidden, ..Entry::blank() };
            if !it.children.is_empty() {
                e.children = tidy(conv(&it.children, token, background, &path, hidden, show_hidden, count));
                if e.children.is_empty() {
                    continue;
                }
            } else {
                e.run = Some(Run::Shell(token, it.id));
            }
            out.push(e);
        }
        out
    }
    tidy(conv(&menu.items, menu.token, menu.background, "", hidden, show_hidden, hidden_count))
}

// ---------------------------------------------------------------- filas de cada nivel

/// Una fila de una ventana del menú.
pub enum Row<'e> {
    Item { e: &'e Entry, label: Option<String>, pinned: bool, first: bool },
    Sep,
    Loading,
    /// "N opciones ocultas": mostrarlas u ocultarlas otra vez.
    HiddenFooter(usize, bool),
    Nothing,
}

// Medidas de las filas: `measure` y el dibujo usan exactamente estas (y las mismas fuentes), así
// cada ventana se abre del tamaño justo.
const ROW_H: f32 = h::ROW;
/// Separador: el de `design::menu_sep` (4 + 1 + 4).
const SEP_H: f32 = sp::S + 1.0;
/// Búsqueda: campo de 28 y 4 de aire debajo.
const SEARCH_H: f32 = h::CTL + sp::XS;
/// Margen interior de cada ventana del menú.
pub const PAD: f32 = sp::XS;
/// Ancho mínimo y máximo de las filas.
const MIN_W: f32 = 230.0;
const MAX_W: f32 = 440.0;
/// Columna del texto: `[8][ícono 16][8]`.
const TEXT_X: f32 = sp::S + icon::M + sp::S;
/// Botón mini (estrella, ojo) y lo que ocupa con su separación.
const BTN: f32 = h::CHIP;
const BTN_STEP: f32 = BTN + sp::XXS;

/// El marco de cada ventana del menú: el de los popovers, sin radio (las esquinas las pone DWM)
/// ni sombra (también la pone Windows).
pub fn frame(t: &Theme) -> egui::Frame {
    design::popover_frame(t).corner_radius(egui::CornerRadius::ZERO).shadow(egui::Shadow::NONE).inner_margin(PAD)
}

fn flatten<'e>(entries: &'e [Entry], parent: &str, out: &mut Vec<(String, &'e Entry)>) {
    for e in entries {
        if e.separator {
            continue;
        }
        let label = if parent.is_empty() { e.label.clone() } else { format!("{parent} › {}", e.label) };
        if e.children.is_empty() {
            out.push((label, e));
        } else {
            flatten(&e.children, &label, out);
        }
    }
}

/// Filas del menú principal: búsqueda (resultados) o fijadas + Rayo + Windows.
pub fn top_rows<'e>(rayo: &'e [Entry], native: &'e [Entry], pins: &[String], query: &str, loading: bool, hidden_count: usize, show_hidden: bool) -> Vec<Row<'e>> {
    let mut all: Vec<(String, &Entry)> = Vec::new();
    flatten(rayo, "", &mut all);
    flatten(native, "", &mut all);
    let mut rows = Vec::new();
    if !query.trim().is_empty() {
        let words: Vec<String> = search_key(query.trim()).split_whitespace().map(str::to_string).collect();
        let found: Vec<(String, &Entry)> = all.into_iter().filter(|(label, e)| e.run.is_some() && !e.disabled && !e.hidden && words.iter().all(|w| search_key(label).contains(w.as_str()))).take(14).collect();
        if found.is_empty() {
            rows.push(Row::Nothing);
        }
        for (k, (label, e)) in found.into_iter().enumerate() {
            rows.push(Row::Item { e, label: Some(label), pinned: pins.contains(&e.key), first: k == 0 });
        }
        return rows;
    }
    for p in pins {
        if let Some((label, e)) = all.iter().find(|(_, e)| &e.key == p && !e.hidden) {
            let short = label.rsplit(" › ").next().unwrap_or(label).to_string();
            rows.push(Row::Item { e, label: Some(short), pinned: true, first: false });
        }
    }
    if !rows.is_empty() {
        rows.push(Row::Sep);
    }
    for e in rayo {
        rows.push(if e.separator { Row::Sep } else { Row::Item { e, label: None, pinned: pins.contains(&e.key), first: false } });
    }
    if !native.is_empty() || loading {
        rows.push(Row::Sep);
    }
    if loading {
        rows.push(Row::Loading);
    }
    for e in native {
        rows.push(if e.separator { Row::Sep } else { Row::Item { e, label: None, pinned: pins.contains(&e.key), first: false } });
    }
    if hidden_count > 0 {
        rows.push(Row::Sep);
        rows.push(Row::HiddenFooter(hidden_count, show_hidden));
    }
    rows
}

/// Filas de un submenú.
pub fn sub_rows<'e>(children: &'e [Entry], pins: &[String]) -> Vec<Row<'e>> {
    children.iter().map(|e| if e.separator { Row::Sep } else { Row::Item { e, label: None, pinned: pins.contains(&e.key), first: false } }).collect()
}

/// Tamaño de una ventana del menú (en puntos), para abrirla ya del tamaño justo.
pub fn measure(ctx: &egui::Context, rows: &[Row], search: bool) -> egui::Vec2 {
    let text_w = |s: &str, font: egui::FontId| ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x);
    let mut w: f32 = MIN_W;
    let mut h = if search { SEARCH_H } else { 0.0 };
    for r in rows {
        match r {
            Row::Item { e, label, .. } => {
                let keys_w = if e.keys.is_empty() { 0.0 } else { text_w(&e.keys, text::mono_s()) };
                // + 1 por el redondeo: que el texto no se corte por una fracción.
                w = w.max(TEXT_X + text_w(label.as_deref().unwrap_or(&e.label), text::body()) + trailing_w(e, keys_w) + 1.0);
                h += ROW_H;
            }
            Row::Sep => h += SEP_H,
            _ => h += ROW_H,
        }
    }
    // + el borde de 1 px de cada lado (y uno de margen por el redondeo a píxeles).
    vec2(w.min(MAX_W) + PAD * 2.0 + 2.0, h + PAD * 2.0 + 3.0)
}

/// Lleva estrella (se puede fijar arriba).
fn has_star(e: &Entry) -> bool {
    e.children.is_empty() && !e.key.is_empty() && !e.disabled && !e.hidden
}

/// Lleva ojo (se puede ocultar).
fn has_eye(e: &Entry) -> bool {
    e.hideable && !e.key.is_empty()
}

/// Lo que ocupa a la derecha del texto: atajo, estrella, ojo, marca, flecha y el margen de 8.
fn trailing_w(e: &Entry, keys_w: f32) -> f32 {
    let mut w = sp::S;
    if !e.children.is_empty() {
        w += icon::S + sp::S;
    }
    if e.checked {
        w += icon::S + sp::S;
    }
    if has_eye(e) {
        w += BTN_STEP;
    }
    if has_star(e) {
        w += BTN_STEP;
    }
    if keys_w > 0.0 {
        w += keys_w + sp::S;
    }
    w
}

// ---------------------------------------------------------------- dibujo

/// Lo que pasó en una ventana del menú en este cuadro.
#[derive(Default)]
pub struct LevelOut {
    /// Fila con submenú bajo el puntero (o tocada): (índice de la fila, su rectángulo en la ventana).
    pub hover_sub: Option<(usize, Rect)>,
    /// El puntero está sobre una fila sin submenú (cerrar los submenús de más adentro).
    pub hover_leaf: bool,
    pub run: Option<Run>,
    pub toggle_pin: Option<String>,
    pub toggle_hidden: Option<String>,
    pub toggle_show_hidden: bool,
}

/// Dibuja las filas de una ventana del menú. `open_sub`: la fila cuyo submenú está abierto.
pub fn draw_rows(ui: &mut Ui, t: &Theme, rows: &[Row], open_sub: Option<usize>, enter: bool) -> LevelOut {
    let mut out = LevelOut::default();
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    for (k, r) in rows.iter().enumerate() {
        match r {
            Row::Sep => design::menu_sep(ui, t),
            Row::Loading => {
                let (rr, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                // Medidor que recorre en la columna del ícono: 3 puntos (13 px) centrados en los 16.
                let mr = Rect::from_center_size(pos2(rr.left() + sp::S + icon::M / 2.0, rr.center().y), vec2(3.0 * 5.0 - 2.0, 3.0));
                design::dot_meter_busy(ui.painter(), t, mr);
                ui.painter().text(pos2(rr.left() + TEXT_X, rr.center().y), Align2::LEFT_CENTER, t!("menu.opciones_windows"), text::small(), t.faint);
            }
            Row::Nothing => {
                let (rr, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                ui.painter().text(pos2(rr.left() + TEXT_X, rr.center().y), Align2::LEFT_CENTER, t!("menu.nada_coincide"), text::small(), t.faint);
            }
            Row::HiddenFooter(n, showing) => {
                let (rr, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                let pill = rr.shrink2(vec2(0.0, 1.0));
                let hov = design::hover_amount(ui.ctx(), resp.id, resp.hovered());
                design::state_fill(ui.painter(), t, pill, radius::M, hov, resp.is_pointer_button_down_on(), Sel::None);
                if resp.hovered() {
                    out.hover_leaf = true;
                }
                let glyph = if *showing { ic::EYE_SLASH } else { ic::EYE };
                ui.painter().text(pos2(rr.left() + sp::S + icon::M / 2.0, rr.center().y), Align2::CENTER_CENTER, glyph, design::icon_font(icon::M), t.faint);
                let label = if *showing { t!("menu.esconder_otra_vez_ocultas").to_string() } else if *n == 1 { t!("menu.1_opcion_oculta_mostrar").to_string() } else { tf!("menu.opciones_ocultas_mostrar", n = n) };
                ui.painter().text(pos2(rr.left() + TEXT_X, rr.center().y), Align2::LEFT_CENTER, label, text::small(), t.faint);
                if resp.clicked() {
                    out.toggle_show_hidden = true;
                }
            }
            Row::Item { e, label, pinned, first } => {
                let res = row(ui, t, e, label.as_deref(), *pinned, *first || open_sub == Some(k), w);
                if res.hovered {
                    if e.children.is_empty() {
                        out.hover_leaf = true;
                    } else {
                        out.hover_sub = Some((k, res.resp.rect));
                    }
                }
                if res.star {
                    out.toggle_pin = Some(e.key.clone());
                } else if res.hide {
                    out.toggle_hidden = Some(e.key.clone());
                } else if !e.children.is_empty() && res.resp.clicked() {
                    out.hover_sub = Some((k, res.resp.rect));
                } else if (res.resp.clicked() || (*first && enter)) && e.children.is_empty() && !e.hidden {
                    out.run = e.run.clone();
                }
            }
        }
    }
    out
}

/// La búsqueda de arriba del menú principal: campo de 28 sin borde, a ras del marco. Devuelve si
/// se apretó Enter.
pub fn search_box(ui: &mut Ui, t: &Theme, query: &mut String, focus: bool) -> bool {
    // Alto exacto (SEARCH_H): sin el espacio entre elementos, la ventana mide justo lo que `measure` calcula.
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    let (sr, _) = ui.allocate_exact_size(vec2(w, h::CTL), Sense::hover());
    ui.painter().rect_filled(sr, radius::M, t.sidebar);
    ui.painter().text(pos2(sr.left() + sp::S + icon::M / 2.0, sr.center().y), Align2::CENTER_CENTER, ic::MAGNIFYING_GLASS, design::icon_font(icon::M), t.dim);
    let er = Rect::from_min_max(pos2(sr.left() + TEXT_X, sr.top()), pos2(sr.right() - sp::S, sr.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(er).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let edit = child.add(
        egui::TextEdit::singleline(query)
            .id(Id::new("menu-busqueda"))
            .frame(egui::Frame::NONE)
            .font(text::body())
            .hint_text(egui::RichText::new(t!("menu.buscar_opcion")).font(text::body()).color(t.faint))
            .desired_width(er.width())
            .vertical_align(egui::Align::Center),
    );
    if focus || !edit.has_focus() {
        edit.request_focus();
    }
    ui.add_space(SEARCH_H - h::CTL);
    ui.input(|i| i.key_pressed(egui::Key::Enter))
}

struct RowOut {
    resp: egui::Response,
    /// El puntero está sobre la fila (también sobre su estrella u ojo).
    hovered: bool,
    star: bool,
    hide: bool,
}

/// Botón mini (estrella, ojo) pintado en `vis`, que responde en toda la franja `hit`. Se pinta
/// solo si `show`; el clic vale siempre.
#[allow(clippy::too_many_arguments)]
fn mini_btn(ui: &Ui, t: &Theme, hit: Rect, vis: Rect, id: Id, show: bool, glyph: &str, filled: bool, color: Option<Color32>) -> bool {
    let r = ui.interact(hit, id, Sense::click());
    if show {
        let p = ui.painter();
        let hov = design::hover_amount(ui.ctx(), id, r.hovered());
        design::state_fill(p, t, vis, radius::S, hov, r.is_pointer_button_down_on(), Sel::None);
        let font = if filled { design::icon_fill(icon::S) } else { design::icon_font(icon::S) };
        p.text(vis.center(), Align2::CENTER_CENTER, glyph, font, color.unwrap_or(if r.hovered() { t.text } else { t.faint }));
    }
    r.clicked()
}

/// Una fila (4.1, sin margen de pill): ícono 16 en x=8, texto en x=32 y, a la derecha, atajo,
/// estrella, ojo (los dos solo con hover) y flecha de submenú.
fn row(ui: &mut Ui, t: &Theme, e: &Entry, label: Option<&str>, pinned: bool, highlight: bool, w: f32) -> RowOut {
    let sense = if e.disabled { Sense::hover() } else { Sense::click() };
    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), sense);
    let p = ui.painter().clone();
    // Mirar el rectángulo y no `resp.hovered()`: la estrella y el ojo quedan encima de la fila, y si la
    // fila dejara de contar como "bajo el puntero" al pasar por ellos, se esconderían y volverían a
    // aparecer en cada cuadro (parpadeo, y el clic casi nunca llegaba). Igual que la X de las pestañas.
    let inside = ui.rect_contains_pointer(r);
    let hovered = inside || highlight;
    if !e.disabled {
        // Con el cursor de teclado o con el submenú abierto: capa de seleccionado; con el puntero, hover.
        let hov = design::hover_amount(ui.ctx(), resp.id, inside);
        design::state_fill(&p, t, r.shrink2(vec2(0.0, 1.0)), radius::M, hov, resp.is_pointer_button_down_on(), if highlight { Sel::Unfocused } else { Sel::None });
    }
    // Acción irreversible: texto e ícono en `danger`.
    let danger = e.key == "cmd:delete";
    let (fg, ic_c) = if e.disabled {
        (t.disabled, t.disabled)
    } else if e.hidden {
        (t.faint, t.faint)
    } else if danger {
        (t.danger, t.danger)
    } else {
        (t.text, t.dim)
    };
    let cy = r.center().y;
    let ic_pos = pos2(r.left() + sp::S + icon::M / 2.0, cy);
    match &e.icon {
        Icon::Glyph(g) => {
            p.text(ic_pos, Align2::CENTER_CENTER, *g, design::icon_font(icon::M), ic_c);
        }
        Icon::Color(i) => design::tag_dot(&p, t, ic_pos, t.tags[(*i).min(6)]),
        Icon::Image(img, key) => {
            let id = Id::new(key);
            let tex = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
                let ci = if img.premultiplied {
                    egui::ColorImage::from_rgba_premultiplied([img.w as usize, img.h as usize], &img.px)
                } else {
                    egui::ColorImage::from_rgba_unmultiplied([img.w as usize, img.h as usize], &img.px)
                };
                let tex = ui.ctx().load_texture(key, ci, egui::TextureOptions::LINEAR);
                ui.ctx().data_mut(|d| d.insert_temp(id, tex.clone()));
                tex
            });
            let ir = Rect::from_center_size(ic_pos, vec2(icon::M, icon::M));
            let tint = if e.disabled || e.hidden { Color32::WHITE.gamma_multiply(0.5) } else { Color32::WHITE };
            p.image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        Icon::None => {}
    }
    // De derecha a izquierda, con las mismas medidas que `trailing_w`.
    let mut right = r.right() - sp::S;
    let mut star = false;
    let mut hide = false;
    if !e.children.is_empty() {
        p.text(pos2(right, cy), Align2::RIGHT_CENTER, ic::CARET_RIGHT, design::icon_font(icon::S), t.faint);
        right -= icon::S + sp::S;
    }
    // Etiqueta puesta: check de 12 en acento.
    if e.checked {
        p.text(pos2(right, cy), Align2::RIGHT_CENTER, ic::CHECK, design::icon_font(icon::S), if e.disabled { t.disabled } else { t.accent });
        right -= icon::S + sp::S;
    }
    // Franja que responde al clic y botón visible (20×20) de cada botón mini.
    let btn = |right: f32| {
        let hit = Rect::from_min_max(pos2(right - BTN_STEP, r.top()), pos2(right, r.bottom()));
        let vis = Rect::from_min_max(pos2(right - BTN, cy - BTN / 2.0), pos2(right, cy + BTN / 2.0));
        (hit, vis)
    };
    // Ocultar / volver a mostrar (de Rayo o de Windows): se ve al pasar por encima.
    if has_eye(e) {
        let (hit, vis) = btn(right);
        let glyph = if e.hidden { ic::EYE } else { ic::EYE_SLASH };
        hide = mini_btn(ui, t, hit, vis, resp.id.with("hide"), hovered || e.hidden, glyph, false, None);
        right -= BTN_STEP;
    }
    // La estrella: se ve al pasar por encima (o siempre, si está fijada).
    if has_star(e) {
        let (hit, vis) = btn(right);
        // Fijada: rellena en `text` (fijar no es «ahora»: el acento no).
        let (glyph, color) = if pinned { (egui_phosphor::fill::STAR, Some(t.text)) } else { (ic::STAR, None) };
        star = mini_btn(ui, t, hit, vis, resp.id.with("star"), hovered || pinned, glyph, pinned, color);
        right -= BTN_STEP;
    }
    // Atajo: mono chica, sin caja.
    if !e.keys.is_empty() {
        right = design::keys_text(&p, t, pos2(right, cy), &e.keys).left() - sp::S;
    }
    let x = r.left() + TEXT_X;
    let g = super::views::elided(&p, label.unwrap_or(&e.label), text::body(), fg, right - x);
    p.galley(pos2(x, cy - g.size().y / 2.0), g, fg);
    RowOut { resp, hovered: inside, star, hide }
}
