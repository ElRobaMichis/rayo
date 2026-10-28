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
use super::theme::{self, Theme};
use crate::fsx::ctxmenu::{ShellItem, ShellMenu};
use crate::fsx::thumbs::Rgba;
use crate::index::store::search_key;
use egui::{Align2, Color32, FontId, Id, Rect, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use std::sync::Arc;

pub enum Icon {
    Glyph(&'static str),
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
}

impl Entry {
    fn blank() -> Entry {
        Entry { key: String::new(), icon: Icon::None, label: String::new(), keys: String::new(), run: None, children: Vec::new(), disabled: false, separator: false, hideable: false, hidden: false }
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
    push(&mut e, Entry::cmd("who_uses", &t));
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

/// Ícono para las opciones de Windows que no traen uno.
fn fallback_icon(item: &ShellItem) -> &'static str {
    let v = item.verb.to_ascii_lowercase();
    let t = item.text.to_lowercase();
    if t.contains("7-zip") || v.starts_with("sevenzip") {
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
        ic::DOT_OUTLINE
    }
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
                None => Icon::Glyph(fallback_icon(it)),
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

const ROW_H: f32 = 26.0;
const GAP: f32 = 1.0;
const SEP_H: f32 = 7.0;
const SEARCH_H: f32 = 33.0;
/// Margen interior de cada ventana del menú.
pub const PAD: f32 = 5.0;

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
    let text_w = |s: &str, size: f32| ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), FontId::proportional(size), Color32::WHITE).size().x);
    let mut w: f32 = 230.0;
    let mut h = if search { SEARCH_H } else { 0.0 };
    for r in rows {
        match r {
            Row::Item { e, label, .. } => {
                let keys = if e.keys.is_empty() { 0.0 } else { text_w(&e.keys, 11.0) + 16.0 };
                w = w.max(32.0 + text_w(label.as_deref().unwrap_or(&e.label), 13.0) + 12.0 + keys + 54.0);
                h += ROW_H + GAP;
            }
            Row::Sep => h += SEP_H,
            _ => h += ROW_H + GAP,
        }
    }
    // + el borde de 1 px de cada lado (y uno de margen por el redondeo a píxeles).
    vec2(w.min(440.0) + PAD * 2.0 + 2.0, h + PAD * 2.0 + 3.0)
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
    ui.spacing_mut().item_spacing.y = GAP;
    let w = ui.available_width();
    for (k, r) in rows.iter().enumerate() {
        match r {
            Row::Sep => {
                let (rr, _) = ui.allocate_exact_size(vec2(w, SEP_H - GAP), Sense::hover());
                ui.painter().line_segment([pos2(rr.left() + 6.0, rr.center().y), pos2(rr.right() - 6.0, rr.center().y)], egui::Stroke::new(1.0, t.border));
            }
            Row::Loading => {
                let (rr, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rr.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                child.spinner();
                child.label(egui::RichText::new(t!("menu.opciones_windows")).size(12.0).color(t.faint));
            }
            Row::Nothing => {
                let (rr, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                ui.painter().text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, t!("menu.nada_coincide"), FontId::proportional(13.0), t.faint);
            }
            Row::HiddenFooter(n, showing) => {
                let (rr, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rr, 5.0, t.hover);
                    out.hover_leaf = true;
                }
                let glyph = if *showing { ic::EYE_SLASH } else { ic::EYE };
                ui.painter().text(pos2(rr.left() + 14.0, rr.center().y), Align2::CENTER_CENTER, glyph, theme::icon_font(14.0, false), t.faint);
                let text = if *showing { t!("menu.esconder_otra_vez_ocultas").to_string() } else if *n == 1 { t!("menu.1_opcion_oculta_mostrar").to_string() } else { tf!("menu.opciones_ocultas_mostrar", n = n) };
                ui.painter().text(pos2(rr.left() + 32.0, rr.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), t.faint);
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

/// La búsqueda de arriba del menú principal. Devuelve si se apretó Enter.
pub fn search_box(ui: &mut Ui, t: &Theme, query: &mut String, focus: bool) -> bool {
    // Alto exacto (SEARCH_H): sin el espacio entre elementos, la ventana mide justo lo que `measure` calcula.
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    let (sr, _) = ui.allocate_exact_size(vec2(w, SEARCH_H - 3.0), Sense::hover());
    ui.painter().rect_filled(sr.shrink2(vec2(1.0, 2.0)), 6.0, t.bg);
    ui.painter().text(pos2(sr.left() + 14.0, sr.center().y), Align2::CENTER_CENTER, ic::MAGNIFYING_GLASS, theme::icon_font(13.0, false), t.faint);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(sr.left() + 28.0, sr.top() + 5.0), pos2(sr.right() - 8.0, sr.bottom() - 5.0))));
    let edit = child.add(egui::TextEdit::singleline(query).id(Id::new("menu-busqueda")).frame(egui::Frame::NONE).hint_text(egui::RichText::new(t!("menu.buscar_opcion")).color(t.faint)).desired_width(w - 40.0));
    if focus || !edit.has_focus() {
        edit.request_focus();
    }
    ui.add_space(3.0);
    ui.input(|i| i.key_pressed(egui::Key::Enter))
}

struct RowOut {
    resp: egui::Response,
    /// El puntero está sobre la fila (también sobre su estrella u ojo).
    hovered: bool,
    star: bool,
    hide: bool,
}

/// Una fila: ícono, texto, atajo, estrella y (si es de Windows) ojo para ocultarla.
fn row(ui: &mut Ui, t: &Theme, e: &Entry, label: Option<&str>, pinned: bool, highlight: bool, w: f32) -> RowOut {
    let sense = if e.disabled { Sense::hover() } else { Sense::click() };
    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), sense);
    let p = ui.painter().clone();
    // Mirar el rectángulo y no `resp.hovered()`: la estrella y el ojo quedan encima de la fila, y si la
    // fila dejara de contar como "bajo el puntero" al pasar por ellos, se esconderían y volverían a
    // aparecer en cada cuadro (parpadeo, y el clic casi nunca llegaba). Igual que la X de las pestañas.
    let inside = ui.rect_contains_pointer(r);
    let hovered = inside || highlight;
    if hovered && !e.disabled {
        p.rect_filled(r, 5.0, t.hover);
    }
    let dim = e.disabled || e.hidden;
    let fg = if dim { t.faint } else { t.text };
    let ic_pos = pos2(r.left() + 14.0, r.center().y);
    match &e.icon {
        Icon::Glyph(g) => {
            p.text(ic_pos, Align2::CENTER_CENTER, *g, theme::icon_font(14.0, false), if dim { t.faint } else { t.dim });
        }
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
            let ir = Rect::from_center_size(ic_pos, vec2(16.0, 16.0));
            let tint = if dim { Color32::from_white_alpha(110) } else { Color32::WHITE };
            p.image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        Icon::None => {}
    }
    let mut right = r.right() - 6.0;
    let mut star = false;
    let mut hide = false;
    if !e.children.is_empty() {
        p.text(pos2(right - 8.0, r.center().y), Align2::CENTER_CENTER, ic::CARET_RIGHT, theme::icon_font(11.0, false), t.faint);
        right -= 20.0;
    }
    // Ocultar / volver a mostrar (opciones de Windows): se ve al pasar por encima.
    if e.hideable && !e.key.is_empty() {
        let hit = Rect::from_min_max(pos2(right - 22.0, r.top()), pos2(right + 4.0, r.bottom()));
        let hr = Rect::from_center_size(pos2(right - 9.0, r.center().y), vec2(20.0, 20.0));
        let hresp = ui.interact(hit, resp.id.with("hide"), Sense::click());
        if hovered || e.hidden {
            if hresp.hovered() {
                p.rect_filled(hr, 4.0, t.border);
            }
            let glyph = if e.hidden { ic::EYE } else { ic::EYE_SLASH };
            p.text(hr.center(), Align2::CENTER_CENTER, glyph, theme::icon_font(12.5, false), t.faint);
        }
        hide = hresp.clicked();
        right -= 22.0;
    }
    // La estrella: se ve al pasar por encima (o siempre, si está fijada).
    if e.children.is_empty() && !e.key.is_empty() && !e.disabled && !e.hidden {
        let hit = Rect::from_min_max(pos2(right - 24.0, r.top()), pos2(right + 4.0, r.bottom()));
        let sr = Rect::from_center_size(pos2(right - 9.0, r.center().y), vec2(20.0, 20.0));
        let sresp = ui.interact(hit, resp.id.with("star"), Sense::click());
        if hovered || pinned {
            if sresp.hovered() {
                p.rect_filled(sr, 4.0, t.border);
            }
            let (glyph, font, color) = if pinned {
                (egui_phosphor::fill::STAR, theme::icon_font(12.5, true), t.accent)
            } else {
                (ic::STAR, theme::icon_font(12.5, false), t.faint)
            };
            p.text(sr.center(), Align2::CENTER_CENTER, glyph, font, color);
        }
        star = sresp.clicked();
        right -= 22.0;
    }
    // Atajo de teclado, como una pastilla.
    if !e.keys.is_empty() {
        let g = p.layout_no_wrap(e.keys.clone(), FontId::proportional(11.0), t.faint);
        let cr = Rect::from_min_max(pos2(right - g.size().x - 10.0, r.center().y - 9.0), pos2(right, r.center().y + 9.0));
        p.rect_stroke(cr, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        p.galley(pos2(cr.left() + 5.0, cr.center().y - g.size().y / 2.0), g, t.faint);
        right = cr.left() - 6.0;
    }
    let text = label.unwrap_or(&e.label);
    let g = super::views::elided(&p, text, FontId::proportional(13.0), fg, (right - (r.left() + 32.0)).max(20.0));
    p.galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, fg);
    RowOut { resp, hovered: inside, star, hide }
}
