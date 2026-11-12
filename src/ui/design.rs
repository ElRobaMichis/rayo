//! Sistema de diseño de Rayo, «Calma eléctrica» (ver `reports/diseno-rayo.md`): superficies de
//! grafito, bordes de 1 px, el dato en mono y un solo color saturado, el azul, para lo que pasa
//! *ahora* (foco, selección con foco, acción primaria, lo instantáneo).
//!
//! Todo tamaño, radio, alto, duración y tipo de letra de la interfaz sale de aquí, y las piezas
//! comunes (fila, menú, botones, chips, teclas, marcos, medidor y trama de puntos) se dibujan
//! siempre con estas funciones. Ninguna guarda estado propio.

use super::motion;
use super::theme::{self, Theme};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Frame, Id, Margin, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

/// Espacio (base 4).
pub mod sp {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 40.0;
}

/// Radios: chip/tecla, fila/control, popover/tarjeta, diálogo. Radio exterior = interior + margen.
pub mod radius {
    pub const S: f32 = 4.0;
    pub const M: f32 = 6.0;
    pub const L: f32 = 10.0;
    pub const XL: f32 = 14.0;
}

/// Alturas.
pub mod h {
    /// Encabezados (de columnas, de sección), fila de metadatos, barra de estado.
    pub const HEAD: f32 = 24.0;
    /// Fila de datos: archivos, resultados, ítem de menú.
    pub const ROW: f32 = 26.0;
    /// Ítem de barra lateral, fila de la paleta.
    pub const NAV: f32 = 28.0;
    /// Botón, campo, segmentado, botón de ícono, barra de ruta.
    pub const CTL: f32 = 28.0;
    pub const CHIP: f32 = 20.0;
    pub const KBD: f32 = 18.0;
    pub const PALETTE: f32 = 40.0;
    pub const TITLE: f32 = 40.0;
    pub const TOOLBAR: f32 = 44.0;
    pub const DRIVE: f32 = 40.0;
}

/// Íconos de interfaz: carets y X / toda fila, botón y campo / encabezado de diálogo.
pub mod icon {
    pub const S: f32 = 12.0;
    pub const M: f32 = 16.0;
    pub const L: f32 = 20.0;
    pub const HERO: f32 = 40.0;
    /// X de cerrar pestaña y botones de la barra de título: 14 en vez de 12 (compensación óptica).
    pub const TAB_X: f32 = 14.0;
    pub const CAPTION: f32 = 14.0;
}

/// Ancho de panel por debajo del cual la búsqueda se pliega a un botón.
pub const NARROW_PANE: f32 = 520.0;

pub mod stroke {
    /// Bordes y divisores.
    pub const HAIR: f32 = 1.0;
    /// Foco y zona para soltar (siempre en acento, por dentro).
    pub const FOCUS: f32 = 1.5;
    /// Panel activo, subrayado de grupo, borde de columna al arrastrar.
    pub const MARK: f32 = 2.0;
}

/// Duraciones (con `motion::ease_out`).
pub mod dur {
    /// Hover que entra; aparición de popover y tooltip.
    pub const FAST: f32 = 0.08;
    /// Hover que sale; selección que se desliza; avisos.
    pub const BASE: f32 = 0.12;
    /// Cambio de carpeta; columnas; miniaturas e íconos que llegan.
    pub const SLOW: f32 = 0.18;
}

/// Tipografía: Geist para nombres y frases, Geist Mono para todo lo que se compara o se copia
/// (tamaños, fechas, conteos, atajos, rutas). Dos pesos: 400 y 500.
pub mod text {
    use egui::{FontFamily, FontId};
    fn medium(size: f32) -> FontId {
        FontId::new(size, FontFamily::Name("semibold".into()))
    }
    /// Secundario, ayuda, tooltips, chips, nombre de mosaico.
    pub fn small() -> FontId {
        FontId::proportional(12.0)
    }
    /// Nombres, barra lateral, menús, pestañas, botones, campos, cuerpo de diálogo.
    pub fn body() -> FontId {
        FontId::proportional(13.0)
    }
    /// Última miga, coincidencia de búsqueda, botón primario.
    pub fn strong() -> FontId {
        medium(13.0)
    }
    /// Título de diálogo, nombre en la vista previa, texto de la paleta.
    pub fn title() -> FontId {
        medium(15.0)
    }
    /// Bienvenida, atajo que se está grabando.
    pub fn display() -> FontId {
        medium(20.0)
    }
    /// Micro-etiquetas, conteos, atajos, barra de estado, insignias.
    pub fn mono_s() -> FontId {
        FontId::monospace(11.0)
    }
    /// Tamaño y fecha en filas, valores, rutas, hashes, código.
    pub fn mono() -> FontId {
        FontId::monospace(12.0)
    }
    /// Terminal, código en Vista rápida.
    pub fn mono_l() -> FontId {
        FontId::monospace(13.0)
    }
}

/// Fuente de íconos (contorno; relleno solo para carpetas, casillas marcadas, estrella y el rayo).
pub fn icon_font(size: f32) -> FontId {
    theme::icon_font(size, false)
}

pub fn icon_fill(size: f32) -> FontId {
    theme::icon_font(size, true)
}

// ---------------------------------------------------------------- tono

/// Tono de un chip, de un aviso o de la placa de un diálogo.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Ok,
    Warn,
    Danger,
}

impl Tone {
    pub fn text(self, t: &Theme) -> Color32 {
        match self {
            Tone::Neutral => t.dim,
            Tone::Accent => t.accent,
            Tone::Ok => t.ok,
            Tone::Warn => t.warn,
            Tone::Danger => t.danger,
        }
    }

    pub fn soft(self, t: &Theme) -> Color32 {
        match self {
            Tone::Neutral => t.hover,
            Tone::Accent => t.accent_soft,
            Tone::Ok => t.ok_soft,
            Tone::Warn => t.warn_soft,
            Tone::Danger => t.danger_soft,
        }
    }
}

// ---------------------------------------------------------------- estados

/// Selección de una fila (o de un ítem activo).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sel {
    #[default]
    None,
    /// Con foco: tinte de acento.
    Focused,
    /// Sin foco, o ítem activo de navegación: capa gris.
    Unfocused,
}

/// Cuánto hover tiene `id` (0-1): entra rápido y sale un poco más lento.
pub fn hover_amount(ctx: &egui::Context, id: Id, hovered: bool) -> f32 {
    motion::fade(ctx, id.with("hover"), hovered, if hovered { dur::FAST } else { dur::BASE })
}

/// Las capas de estado (4.0): seleccionado, presionado, hover. El texto no cambia de color.
pub fn state_fill(p: &Painter, t: &Theme, rect: Rect, r: f32, hover: f32, pressed: bool, sel: Sel) {
    let fill = match sel {
        Sel::Focused => t.sel,
        Sel::Unfocused => t.sel_inactive,
        Sel::None if pressed => t.sel_inactive,
        Sel::None if hover > 0.01 => t.hover.gamma_multiply(hover),
        Sel::None => return,
    };
    p.rect_filled(rect, r, fill);
}

/// Anillo de foco de teclado / zona para soltar: trazo de acento por dentro, mismo radio.
pub fn focus_ring(p: &Painter, t: &Theme, rect: Rect, r: f32) {
    p.rect_stroke(rect, r, Stroke::new(stroke::FOCUS, t.accent), StrokeKind::Inside);
}

// ---------------------------------------------------------------- texto

/// Micro-etiqueta: mono chica, MAYÚSCULAS, con un poco de aire entre letras.
pub fn micro_job(text: &str, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, TextFormat { font_id: text::mono_s(), color, extra_letter_spacing: 0.6, ..Default::default() });
    job
}

/// Pinta una micro-etiqueta anclada en `pos` según `align`. Devuelve el rectángulo que ocupa.
pub fn micro(p: &Painter, pos: Pos2, align: Align2, text: &str, color: Color32) -> Rect {
    let g = p.layout_job(micro_job(text, color));
    let rect = align.anchor_size(pos, g.size());
    p.galley(rect.min, g, color);
    rect
}

/// Micro-etiqueta en el flujo de `ui`.
pub fn micro_label(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let g = ui.painter().layout_job(micro_job(text, color));
    let (rect, r) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(rect.min, g, color);
    r
}

/// Ancho de un texto con esa fuente.
pub fn text_width(p: &Painter, text: &str, font: FontId) -> f32 {
    p.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

/// Atajo en una columna (menú, paleta): mono chica, `faint`, sin caja, alineado a la derecha.
pub fn keys_text(p: &Painter, t: &Theme, right_center: Pos2, keys: &str) -> Rect {
    p.text(right_center, Align2::RIGHT_CENTER, keys, text::mono_s(), t.faint)
}

/// Tecla en línea con texto (campo, tooltip, pista): borde fino, mono chica.
pub fn kbd_paint(p: &Painter, t: &Theme, left_center: Pos2, keys: &str) -> Rect {
    let w = text_width(p, keys, text::mono_s()) + 10.0;
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - h::KBD / 2.0), vec2(w, h::KBD));
    p.rect_stroke(rect, radius::S, Stroke::new(stroke::HAIR, t.border_strong), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, keys, text::mono_s(), t.faint);
    rect
}

pub fn kbd(ui: &mut Ui, t: &Theme, keys: &str) -> Response {
    let w = text_width(ui.painter(), keys, text::mono_s()) + 10.0;
    let (rect, r) = ui.allocate_exact_size(vec2(w, h::KBD), Sense::hover());
    kbd_paint(ui.painter(), t, rect.left_center(), keys);
    r
}

// ---------------------------------------------------------------- fila

/// Lo que va al final de una fila, alineado a la derecha.
#[derive(Clone, Copy, Default)]
pub enum Trailing<'a> {
    #[default]
    None,
    /// Valor (tamaño, fecha): mono, `dim`.
    Value(&'a str),
    /// Conteo: mono chica, `faint`.
    Count(&'a str),
    /// Atajo de teclado: mono chica, `faint`.
    Keys(&'a str),
    /// Flecha de submenú o de carpeta.
    Caret,
}

/// Una fila (4.1): `[margen][8][ícono 16][8][texto…][8][final][8][margen]`.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    pub icon: &'a str,
    /// Color del ícono (`dim` si es None).
    pub icon_color: Option<Color32>,
    /// Ícono relleno (carpetas, estrella puesta, rayo).
    pub icon_filled: bool,
    pub label: &'a str,
    /// Color del texto (`text` si es None).
    pub color: Option<Color32>,
    pub strong: bool,
    pub trailing: Trailing<'a>,
    pub sel: Sel,
    pub enabled: bool,
    /// Margen del pill a cada lado (4 en listas, 8 en la barra lateral, 0 en menús).
    pub inset: f32,
    pub height: f32,
    /// Punto de color (etiqueta) en la columna del ícono, en lugar del ícono.
    pub dot: Option<Color32>,
}

impl<'a> Row<'a> {
    pub fn new(icon: &'a str, label: &'a str) -> Self {
        Row { icon, icon_color: None, icon_filled: false, label, color: None, strong: false, trailing: Trailing::None, sel: Sel::None, enabled: true, inset: 0.0, height: h::ROW, dot: None }
    }
}

/// Dibuja una fila que ocupa todo el ancho disponible y devuelve su respuesta.
pub fn row(ui: &mut Ui, t: &Theme, id: Id, spec: Row) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), spec.height), Sense::hover());
    row_at(ui, t, rect, id, spec)
}

/// La misma fila en un rectángulo ya reservado.
pub fn row_at(ui: &mut Ui, t: &Theme, rect: Rect, id: Id, spec: Row) -> Response {
    let r = ui.interact(rect, id, if spec.enabled { Sense::click() } else { Sense::hover() });
    let pill = Rect::from_min_max(pos2(rect.left() + spec.inset, rect.top() + 1.0), pos2(rect.right() - spec.inset, rect.bottom() - 1.0));
    let p = ui.painter();
    let hov = if spec.enabled { hover_amount(ui.ctx(), id, r.hovered()) } else { 0.0 };
    state_fill(p, t, pill, radius::M, hov, spec.enabled && r.is_pointer_button_down_on(), spec.sel);
    let (fg, ic_c) = if spec.enabled { (spec.color.unwrap_or(t.text), spec.icon_color.unwrap_or(t.dim)) } else { (t.disabled, t.disabled) };
    let mut x = pill.left() + sp::S;
    if let Some(c) = spec.dot {
        tag_dot(p, t, pos2(x + icon::M / 2.0, pill.center().y), c);
        x += icon::M + sp::S;
    } else if !spec.icon.is_empty() {
        let font = if spec.icon_filled { icon_fill(icon::M) } else { icon_font(icon::M) };
        p.text(pos2(x + icon::M / 2.0, pill.center().y), Align2::CENTER_CENTER, spec.icon, font, ic_c);
        x += icon::M + sp::S;
    }
    // El final primero, para saber cuánto lugar le queda al texto.
    let right = pill.right() - sp::S;
    let used = match spec.trailing {
        Trailing::None => 0.0,
        Trailing::Value(v) => p.text(pos2(right, pill.center().y), Align2::RIGHT_CENTER, v, text::mono(), t.dim).width(),
        Trailing::Count(v) | Trailing::Keys(v) => keys_text(p, t, pos2(right, pill.center().y), v).width(),
        Trailing::Caret => p.text(pos2(right, pill.center().y), Align2::RIGHT_CENTER, ic::CARET_RIGHT, icon_font(icon::S), t.faint).width(),
    };
    let max_w = (right - if used > 0.0 { used + sp::S } else { 0.0 }) - x;
    let font = if spec.strong { text::strong() } else { text::body() };
    let mut job = LayoutJob::single_section(spec.label.to_string(), TextFormat::simple(font, fg));
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(0.0));
    let g = p.layout_job(job);
    p.galley(pos2(x, pill.center().y - g.size().y / 2.0), g, fg);
    if spec.enabled && r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    r
}

// ---------------------------------------------------------------- menús

/// Un ítem de menú.
#[derive(Clone, Copy, Default)]
pub struct MenuItem<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub keys: &'a str,
    /// Acción irreversible: texto e ícono en `danger`.
    pub danger: bool,
    /// Lo «actual» (la carpeta por donde va la ruta): ícono en acento y texto `strong`.
    pub current: bool,
    /// Tiene submenú: flecha a la derecha.
    pub submenu: bool,
    /// Casilla: Some(marcada).
    pub checked: Option<bool>,
    pub disabled: bool,
    /// Con el cursor de teclado encima (o con su submenú abierto).
    pub highlighted: bool,
}

/// Ítem de menú con ícono, texto y atajo. Al hacer clic cierra el menú.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str, keys: &str) -> Response {
    menu_item_ex(ui, t, MenuItem { icon, label, keys, ..Default::default() })
}

pub fn menu_item_ex(ui: &mut Ui, t: &Theme, m: MenuItem) -> Response {
    let icon = match m.checked {
        Some(true) => ic::CHECK,
        Some(false) => "",
        None => m.icon,
    };
    let color = if m.danger { Some(t.danger) } else { None };
    let icon_color = if m.danger {
        Some(t.danger)
    } else if m.current || m.checked == Some(true) {
        Some(t.accent)
    } else {
        None
    };
    let trailing = if m.submenu {
        Trailing::Caret
    } else if !m.keys.is_empty() {
        Trailing::Keys(m.keys)
    } else {
        Trailing::None
    };
    // Pide solo el ancho de su contenido (como los botones de egui): el menú mide en la primera
    // pasada y después todas las filas se estiran al ancho del más largo.
    let font = if m.current { text::strong() } else { text::body() };
    let mut want = sp::S + icon::M + sp::S + text_width(ui.painter(), m.label, font) + sp::S;
    if m.submenu {
        want += icon::S + sp::S;
    } else if !m.keys.is_empty() {
        want += text_width(ui.painter(), m.keys, text::mono_s()) + sp::L;
    }
    let (rect, _) = ui.allocate_at_least(vec2(want.max(160.0), h::ROW), Sense::hover());
    let id = ui.id().with(("menu-item", m.label, rect.min.y as i32));
    // Una casilla sin marcar deja el lugar del ícono vacío, así los textos quedan alineados.
    let spec = Row {
        icon: if icon.is_empty() && m.checked.is_some() { " " } else { icon },
        icon_color,
        icon_filled: false,
        label: m.label,
        color,
        strong: m.current,
        trailing,
        sel: if m.highlighted { Sel::Unfocused } else { Sel::None },
        enabled: !m.disabled,
        inset: 0.0,
        height: h::ROW,
        dot: None,
    };
    let r = row_at(ui, t, rect, id, spec);
    if r.clicked() && !m.submenu {
        ui.close();
    }
    r
}

/// Separador de menú: línea fina con 4 px arriba y abajo y 8 a los lados.
pub fn menu_sep(ui: &mut Ui, t: &Theme) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), sp::S + 1.0), Sense::hover());
    let y = rect.center().y.round() + 0.5;
    ui.painter().hline(rect.left() + sp::S..=rect.right() - sp::S, y, Stroke::new(stroke::HAIR, t.border));
}

/// Título de un grupo dentro de un menú (micro-etiqueta).
pub fn menu_title(ui: &mut Ui, t: &Theme, label: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    micro(ui.painter(), pos2(rect.left() + sp::S, rect.center().y), Align2::LEFT_CENTER, label, t.faint);
}

/// Ancho de un menú: `menu_item` llena el ancho disponible y, en la pasada de medida de egui,
/// eso sería todo el `default_area_size` (600).
pub const MENU_W: f32 = 240.0;
/// Lista flotante (migas, sugerencias).
pub const LIST_W: f32 = 300.0;
/// Panel flotante (copias).
pub const PANEL_W: f32 = 440.0;

/// Dentro de un menú: filas pegadas y ancho de menú.
fn menu_body(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.set_max_width(MENU_W);
}

/// Menú de clic derecho con el marco de popover.
pub fn context_menu(resp: &Response, t: &Theme, add: impl FnOnce(&mut Ui)) {
    egui::Popup::context_menu(resp).frame(popover_frame(t)).show(|ui| {
        menu_body(ui);
        add(ui);
    });
}

/// Menú que abre un clic en `resp` (botón, selector), con el marco de popover.
pub fn popup_menu(resp: &Response, t: &Theme, add: impl FnOnce(&mut Ui)) {
    egui::Popup::menu(resp).frame(popover_frame(t)).show(|ui| {
        menu_body(ui);
        add(ui);
    });
}

// ---------------------------------------------------------------- botones

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Uno por diálogo, a la derecha.
    Primary,
    /// Transparente con contorno (es el `ui.button` de egui, con el estilo global).
    Secondary,
    /// Sin borde, texto `dim`.
    Ghost,
    /// Acción irreversible confirmada.
    Danger,
    /// Fantasma con texto e ícono en `danger` («Forzar cierre», «Vaciar papelera»).
    GhostDanger,
}

/// Botón de 28 px (4.5).
pub fn button(ui: &mut Ui, t: &Theme, kind: Kind, icon: &str, label: &str, enabled: bool) -> Response {
    let font = if matches!(kind, Kind::Primary | Kind::Danger) { text::strong() } else { text::body() };
    let tw = text_width(ui.painter(), label, font.clone());
    let iw = if icon.is_empty() { 0.0 } else { icon::M + 6.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(tw + iw + 2.0 * sp::M, h::CTL), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let down = enabled && r.is_pointer_button_down_on();
    let hov = enabled && r.hovered();
    let (fill, border, fg, ic_c) = match kind {
        _ if !enabled => (
            if matches!(kind, Kind::Primary | Kind::Danger) { t.hover } else { Color32::TRANSPARENT },
            if kind == Kind::Secondary { Some(t.border) } else { None },
            t.disabled,
            t.disabled,
        ),
        Kind::Primary => (if down { t.accent_fill_pressed } else if hov { t.accent_fill_hover } else { t.accent_fill }, None, t.accent_text, t.accent_text),
        Kind::Danger => {
            let f = if down { t.danger_fill.gamma_multiply(0.92) } else if hov { t.danger_fill.linear_multiply(1.08) } else { t.danger_fill };
            (f, None, t.accent_text, t.accent_text)
        }
        Kind::Secondary => (if down { t.sel_inactive } else if hov { t.hover } else { Color32::TRANSPARENT }, Some(t.border_strong), t.text, t.dim),
        Kind::Ghost => (if down { t.sel_inactive } else if hov { t.hover } else { Color32::TRANSPARENT }, None, if hov { t.text } else { t.dim }, if hov { t.text } else { t.dim }),
        Kind::GhostDanger => (if down { t.sel_inactive } else if hov { t.hover } else { Color32::TRANSPARENT }, None, t.danger, t.danger),
    };
    p.rect_filled(rect, radius::M, fill);
    if let Some(b) = border {
        p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, b), StrokeKind::Inside);
    }
    let mut x = rect.left() + sp::M;
    if !icon.is_empty() {
        p.text(pos2(x + icon::M / 2.0, rect.center().y), Align2::CENTER_CENTER, icon, icon_font(icon::M), ic_c);
        x += iw;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font, fg);
    if r.has_focus() {
        focus_ring(p, t, rect, radius::M);
    }
    r
}

/// Opciones de un botón de ícono.
#[derive(Clone, Copy, Debug)]
pub struct IconBtn {
    /// 20×20 con ícono de 12 (cerrar pestaña, +, ⋯, X de un campo).
    pub mini: bool,
    /// Encendido: capa gris e ícono en acento.
    pub on: bool,
    pub enabled: bool,
    /// Navegación (‹ › de la Vista rápida, paginador del PDF): mini, pero `dim` en reposo y glifo
    /// de 14 (`faint` a 12 se leía como deshabilitado al lado de los botones de 28).
    pub nav: bool,
}

impl Default for IconBtn {
    fn default() -> Self {
        IconBtn { mini: false, on: false, enabled: true, nav: false }
    }
}

impl IconBtn {
    pub fn mini() -> Self {
        IconBtn { mini: true, ..Default::default() }
    }

    /// Mini de navegación (ver `nav`).
    pub fn nav(enabled: bool) -> Self {
        IconBtn { mini: true, nav: true, enabled, ..Default::default() }
    }
}

/// Botón de ícono: 28×28 (o 20×20 mini).
pub fn icon_button(ui: &mut Ui, t: &Theme, icon_glyph: &str, o: IconBtn) -> Response {
    let s = if o.mini { h::CHIP } else { h::CTL };
    let (rect, _) = ui.allocate_exact_size(vec2(s, s), Sense::hover());
    let id = ui.id().with(("icon-btn", icon_glyph, rect.min.x as i32, rect.min.y as i32));
    icon_button_at(ui, t, rect, id, icon_glyph, o)
}

pub fn icon_button_at(ui: &mut Ui, t: &Theme, rect: Rect, id: Id, icon_glyph: &str, o: IconBtn) -> Response {
    let r = ui.interact(rect, id, if o.enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let rad = if o.mini { radius::S } else { radius::M };
    let hov = if o.enabled { hover_amount(ui.ctx(), id, r.hovered()) } else { 0.0 };
    state_fill(p, t, rect, rad, hov, o.enabled && r.is_pointer_button_down_on(), if o.on { Sel::Unfocused } else { Sel::None });
    let color = if !o.enabled {
        t.disabled
    } else if o.on {
        t.accent
    } else if r.hovered() {
        t.text
    } else if o.mini && !o.nav {
        t.faint
    } else {
        t.dim
    };
    let size = if o.nav {
        icon::CAPTION
    } else if o.mini {
        icon::S
    } else {
        icon::M
    };
    p.text(rect.center(), Align2::CENTER_CENTER, icon_glyph, icon_font(size), color);
    r
}

// ---------------------------------------------------------------- chips

/// Chip o insignia (4.8): alto 20 (18 si es compacto), radio 4.
#[derive(Clone, Copy, Default)]
pub struct Chip<'a> {
    pub text: &'a str,
    pub icon: &'a str,
    pub tone: Tone,
    pub mono: bool,
    /// Punto de color (grupo de pestañas, etiqueta); el texto va en `text`.
    pub dot: Option<Color32>,
    pub closable: bool,
    pub interactive: bool,
    /// Compacto (dentro de una fila de datos): alto 18 y margen 6, para que no toque los bordes.
    pub compact: bool,
    /// Va sobre una fila seleccionada (con o sin foco): en claro, el acento baja a `accent_hover`
    /// y el ámbar se oscurece, para mantener el contraste sobre chip + selección.
    pub on_sel: bool,
    /// Ancho fijo (columna de chips, como los de Hash) con el contenido centrado; 0 = el propio.
    pub width: f32,
}

impl<'a> Chip<'a> {
    pub fn new(text: &'a str, tone: Tone) -> Self {
        Chip { text, tone, ..Default::default() }
    }
}

pub struct ChipOut {
    pub response: Response,
    /// Se hizo clic en la X.
    pub closed: bool,
}

fn chip_font(c: &Chip) -> FontId {
    if c.mono { text::mono_s() } else { text::small() }
}

/// Alto y margen horizontal del texto de un chip.
fn chip_metrics(c: &Chip) -> (f32, f32) {
    if c.compact { (h::KBD, 6.0) } else { (h::CHIP, sp::S) }
}

/// Ancho que ocupa un chip.
pub fn chip_width(p: &Painter, c: &Chip) -> f32 {
    let mut w = text_width(p, c.text, chip_font(c)) + 2.0 * chip_metrics(c).1;
    if !c.icon.is_empty() || c.dot.is_some() {
        w += icon::S + sp::XS - 2.0;
    }
    if c.closable {
        w += icon::S + sp::XS;
    }
    w
}

/// Pinta un chip con su borde izquierdo en `left_center`. Devuelve su rectángulo.
pub fn chip_paint(p: &Painter, t: &Theme, left_center: Pos2, c: &Chip, hovered: bool) -> Rect {
    let (ch, pad) = chip_metrics(c);
    let natural = chip_width(p, c);
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - ch / 2.0), vec2(natural.max(c.width), ch));
    // Con ancho fijo, el contenido va centrado.
    let shift = (rect.width() - natural) / 2.0;
    let fill = if c.dot.is_some() { t.hover } else { c.tone.soft(t) };
    p.rect_filled(rect, radius::S, fill);
    if hovered {
        p.rect_filled(rect, radius::S, t.hover);
    }
    let fg = if c.dot.is_some() {
        t.text
    } else if c.on_sel && !t.dark && c.tone == Tone::Accent {
        t.accent_hover
    } else if c.on_sel && !t.dark && c.tone == Tone::Warn {
        // El ámbar sobre chip + selección no llega a 4,5:1: un 20 % hacia el texto.
        lerp(t.warn, t.text, 0.2)
    } else {
        c.tone.text(t)
    };
    let mut x = rect.left() + shift + if c.icon.is_empty() && c.dot.is_none() { pad } else { 6.0 };
    if let Some(d) = c.dot {
        p.circle_filled(pos2(x + 4.0, rect.center().y), 4.0, d);
        x += icon::S + sp::XS - 2.0;
    } else if !c.icon.is_empty() {
        p.text(pos2(x + icon::S / 2.0, rect.center().y), Align2::CENTER_CENTER, c.icon, icon_font(icon::S), fg);
        x += icon::S + sp::XS - 2.0;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, c.text, chip_font(c), fg);
    if c.closable {
        p.text(pos2(rect.right() - 6.0 - icon::S / 2.0, rect.center().y), Align2::CENTER_CENTER, ic::X, icon_font(icon::S - 2.0), fg);
    }
    rect
}

pub fn chip(ui: &mut Ui, t: &Theme, c: Chip) -> ChipOut {
    let w = chip_width(ui.painter(), &c).max(c.width);
    let (rect, _) = ui.allocate_exact_size(vec2(w, chip_metrics(&c).0), Sense::hover());
    let id = ui.id().with(("chip", c.text, rect.min.x as i32, rect.min.y as i32));
    let r = ui.interact(rect, id, if c.interactive || c.closable { Sense::click() } else { Sense::hover() });
    chip_paint(ui.painter(), t, rect.left_center(), &c, (c.interactive || c.closable) && r.hovered());
    let closed = c.closable && r.clicked() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| pos.x > rect.right() - icon::S - 8.0);
    ChipOut { response: r, closed }
}

// ---------------------------------------------------------------- marcos

/// El único marco de todo lo que flota: menús, popovers, sugerencias, historial, avisos.
pub fn popover_frame(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).stroke(Stroke::new(stroke::HAIR, t.border_strong)).corner_radius(radius::L).inner_margin(Margin::same(4)).shadow(popover_shadow(t))
}

pub fn tooltip_frame(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).stroke(Stroke::new(stroke::HAIR, t.border_strong)).corner_radius(radius::M).inner_margin(Margin::symmetric(8, 4)).shadow(popover_shadow(t))
}

/// Diálogo, paleta, Vista rápida.
pub fn dialog_frame(t: &Theme) -> Frame {
    Frame::new().fill(t.raised).stroke(Stroke::new(stroke::HAIR, t.border_strong)).corner_radius(radius::XL).inner_margin(Margin::same(24)).shadow(dialog_shadow(t))
}

pub fn popover_shadow(t: &Theme) -> egui::Shadow {
    egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if t.dark { 110 } else { 36 }) }
}

pub fn dialog_shadow(t: &Theme) -> egui::Shadow {
    egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(if t.dark { 140 } else { 48 }) }
}

/// Encabezado de diálogo: placa de 32 con ícono de 20 y fondo suave según el tono, título y
/// subtítulo opcional.
pub fn dialog_header(ui: &mut Ui, t: &Theme, icon_glyph: &str, tone: Tone, title: &str, sub: Option<&str>) {
    ui.horizontal(|ui| {
        header_plate(ui, t, icon_glyph, tone);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if sub.is_none() {
                ui.add_space(6.0);
            }
            ui.label(egui::RichText::new(title).font(text::title()).color(t.text));
            if let Some(s) = sub {
                ui.label(egui::RichText::new(s).font(text::small()).color(t.dim));
            }
        });
    });
}

/// La placa de 32 de un encabezado: fondo suave del tono e ícono de 20 (Neutral = acento).
fn header_plate(ui: &mut Ui, t: &Theme, icon_glyph: &str, tone: Tone) {
    let (plate, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
    let tone = if tone == Tone::Neutral { Tone::Accent } else { tone };
    ui.painter().rect_filled(plate, radius::M, tone.soft(t));
    ui.painter().text(plate.center(), Align2::CENTER_CENTER, icon_glyph, icon_font(icon::L), tone.text(t));
    ui.add_space(sp::M - ui.spacing().item_spacing.x);
}

/// Encabezado de diálogo con un control a la derecha (segmentado, botón).
pub fn dialog_header_with(ui: &mut Ui, t: &Theme, icon_glyph: &str, tone: Tone, title: &str, sub: Option<&str>, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        header_plate(ui, t, icon_glyph, tone);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            trailing(ui);
            // Lo que queda, de izquierda a derecha: el título junto a la placa.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                if sub.is_none() {
                    ui.add_space(6.0);
                }
                ui.label(egui::RichText::new(title).font(text::title()).color(t.text));
                if let Some(s) = sub {
                    ui.label(egui::RichText::new(s).font(text::small()).color(t.dim));
                }
            });
        });
    });
}

/// `Id` de cualquier valor con `Hash` (el `Id::new` de egui pide además `Debug`).
fn any_id(v: impl std::hash::Hash) -> Id {
    use std::hash::Hasher;
    let mut h = std::hash::DefaultHasher::new();
    v.hash(&mut h);
    Id::new(h.finish())
}

/// Ancho de un diálogo (4.11), por fuera del marco.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogW {
    S,
    M,
    L,
}

impl DialogW {
    /// 440 / 560 / 760.
    pub fn px(self) -> f32 {
        match self {
            DialogW::S => 440.0,
            DialogW::M => 560.0,
            DialogW::L => 760.0,
        }
    }

    /// Ancho del contenido: sin el margen de 24 ni el borde de 1 a cada lado.
    pub fn inner(self) -> f32 {
        self.px() - 2.0 * sp::XL - 2.0 * stroke::HAIR
    }
}

/// Diálogo modal (4.11): `dialog_frame` sobre `t.overlay`, de `w.px()` por fuera. Fija el ancho y
/// el ancho máximo del contenido.
pub fn dialog<R>(ctx: &egui::Context, t: &Theme, id: impl std::hash::Hash, w: DialogW, content: impl FnOnce(&mut Ui) -> R) -> egui::ModalResponse<R> {
    egui::Modal::new(any_id(id)).frame(dialog_frame(t)).backdrop_color(t.overlay).show(ctx, |ui| {
        ui.set_width(w.inner());
        ui.set_max_width(w.inner());
        content(ui)
    })
}

/// Texto del cuerpo de un diálogo: `body`, `dim`, con ajuste de línea.
pub fn dialog_body(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).font(text::body()).color(t.dim)).wrap())
}

/// Ayuda de un diálogo: `small`, `faint`, con ajuste de línea.
pub fn dialog_help(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).font(text::small()).color(t.faint)).wrap())
}

/// Pie de diálogo (M-13): 16 de aire; `left` a la izquierda (casilla, resumen, acción fantasma);
/// `buttons` de derecha a izquierda con 8 entre sí: el primero que se agrega queda a la derecha y
/// es el de Enter. 1 botón = Primary; 2 = Secondary + Primary; 3 = Ghost + Secondary + Primary;
/// irreversible = Danger en el lugar del primario. Nunca dos llenos.
pub fn dialog_footer(ui: &mut Ui, left: impl FnOnce(&mut Ui), buttons: impl FnOnce(&mut Ui)) {
    ui.add_space(sp::L);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sp::S;
        left(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
    });
}

/// Un control con su rótulo (`small`, `dim`) 4 px encima, en `width` de ancho.
pub fn labeled<R>(ui: &mut Ui, t: &Theme, label: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = sp::XS;
        ui.label(egui::RichText::new(label).font(text::small()).color(t.dim));
        add(ui)
    })
    .inner
}

/// Encabezado de herramienta en pestaña: placa de 32, título `title` y subtítulo (`small` `dim`,
/// o mono si `sub_mono`) acortados por el medio; `trailing` a la derecha.
pub fn tool_header(ui: &mut Ui, t: &Theme, icon_glyph: &str, title: &str, sub: Option<&str>, sub_mono: bool, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        header_plate(ui, t, icon_glyph, Tone::Neutral);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            trailing(ui);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let w = ui.available_width();
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 2.0;
                if sub.is_none() {
                    ui.add_space(6.0);
                }
                let p = ui.painter().clone();
                let shown = elide_middle(&p, title, text::title(), w);
                let r = ui.label(egui::RichText::new(&shown).font(text::title()).color(t.text));
                if shown != title {
                    tip(r, t, title);
                }
                if let Some(s) = sub {
                    let (font, color) = if sub_mono { (text::mono(), t.dim) } else { (text::small(), t.dim) };
                    let shown = elide_middle(&p, s, font.clone(), w);
                    let r = ui.label(egui::RichText::new(&shown).font(font).color(color));
                    if shown != s {
                        tip(r, t, s);
                    }
                }
            });
        });
    });
}

/// Una columna de `list_header`: rótulo anclado en `x` (desde el borde izquierdo) según `align`.
#[derive(Clone, Copy)]
pub struct ListCol<'a> {
    pub label: &'a str,
    pub x: f32,
    pub align: Align2,
}

/// Encabezado de una lista: alto `h::HEAD`, micro-etiquetas `faint`, hairline `border` abajo.
/// Devuelve su rectángulo.
pub fn list_header(ui: &mut Ui, t: &Theme, cols: &[ListCol]) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
    let p = ui.painter();
    for c in cols {
        micro(p, pos2(rect.left() + c.x, rect.center().y), c.align, c.label, t.faint);
    }
    p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(stroke::HAIR, t.border));
    rect
}

/// Tarjeta dentro de un diálogo (vista previa del renombrado, hashes): relleno hundido.
pub fn card_frame(t: &Theme) -> Frame {
    Frame::new().fill(t.sidebar).stroke(Stroke::new(stroke::HAIR, t.border)).corner_radius(radius::L).inner_margin(Margin::same(12))
}

// ---------------------------------------------------------------- datos

/// Rótulo a la izquierda (`small`, `dim`) y valor a la derecha (mono, `text`). Si el valor no
/// entra, baja a la línea siguiente alineado a la izquierda.
pub fn meta_row(ui: &mut Ui, t: &Theme, key: &str, value: &str) -> Response {
    let w = ui.available_width();
    let p = ui.painter().clone();
    let kw = text_width(&p, key, text::small());
    let vw = text_width(&p, value, text::mono());
    if kw + sp::M + vw <= w {
        let (rect, r) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
        p.text(rect.left_center(), Align2::LEFT_CENTER, key, text::small(), t.dim);
        p.text(rect.right_center(), Align2::RIGHT_CENTER, value, text::mono(), t.text);
        r
    } else {
        let mut job = LayoutJob::single_section(value.to_string(), TextFormat::simple(text::mono(), t.text));
        job.wrap.max_width = w;
        let g = p.layout_job(job);
        let (rect, r) = ui.allocate_exact_size(vec2(w, h::HEAD + g.size().y + 2.0), Sense::hover());
        p.text(pos2(rect.left(), rect.top() + h::HEAD / 2.0), Align2::LEFT_CENTER, key, text::small(), t.dim);
        p.galley(pos2(rect.left(), rect.top() + h::HEAD - 2.0), g, t.text);
        r
    }
}

// ---------------------------------------------------------------- puntos (la firma)

/// Trama de puntos de 1 px cada 8: solo en vacíos, el pozo de la vista previa y la zona para
/// soltar del estante. Nunca detrás de texto o de una lista. Un solo mesh por llamada.
pub fn dot_grid(p: &Painter, t: &Theme, rect: Rect) {
    let ppp = p.ctx().pixels_per_point();
    let px = 1.0 / ppp;
    let step = 8.0;
    let x0 = (rect.left() / step).ceil() * step;
    let y0 = (rect.top() / step).ceil() * step;
    let mut mesh = egui::Mesh::default();
    let mut y = y0;
    while y < rect.bottom() {
        let mut x = x0;
        while x < rect.right() {
            // Alineado a píxel: cada punto cae entero en un píxel de la pantalla.
            let px_pos = pos2((x * ppp).round() / ppp, (y * ppp).round() / ppp);
            mesh.add_colored_rect(Rect::from_min_size(px_pos, vec2(px.max(0.75), px.max(0.75))), t.dots);
            x += step;
        }
        y += step;
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Medidor de puntos: cuadrados de 3×3 cada 5 px. El único indicador de progreso (unidades,
/// copias, hash, duplicados). El del frente se pinta con alfa igual a la fracción, para que el
/// avance sea continuo.
pub fn dot_meter(p: &Painter, t: &Theme, rect: Rect, frac: f32, color: Color32) {
    let n = ((rect.width() + 2.0) / 5.0).floor().max(1.0) as usize;
    let filled = frac.clamp(0.0, 1.0) * n as f32;
    let y = rect.center().y - 1.5;
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * 5.0, y), vec2(3.0, 3.0));
        let k = (filled - i as f32).clamp(0.0, 1.0);
        let c = if k >= 1.0 {
            color
        } else if k > 0.0 {
            lerp(t.border_strong, color, k)
        } else {
            t.border_strong
        };
        mesh.add_colored_rect(r, c);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Medidor de puntos indeterminado: ventana de 5 puntos en `accent` que recorre `rect` cada 1,2 s.
/// Pide repintar a 50 ms.
pub fn dot_meter_busy(p: &Painter, t: &Theme, rect: Rect) {
    const WIN: f32 = 5.0;
    let n = ((rect.width() + 2.0) / 5.0).floor().max(1.0) as usize;
    let phase = (p.ctx().input(|i| i.time) % 1.2) as f32 / 1.2;
    // La ventana entra por la izquierda y sale entera por la derecha.
    let head = phase * (n as f32 + WIN);
    let y = rect.center().y - 1.5;
    let mut mesh = egui::Mesh::default();
    for i in 0..n {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * 5.0, y), vec2(3.0, 3.0));
        // Dentro de la ventana, lleno; en los bordes, la fracción (avance continuo).
        let d = head - i as f32;
        let k = d.clamp(0.0, 1.0).min((WIN + 1.0 - d).clamp(0.0, 1.0));
        mesh.add_colored_rect(r, lerp(t.border_strong, t.accent, k));
    }
    p.add(egui::Shape::mesh(mesh));
    p.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}

/// Espera sin spinner: medidor de 12 puntos (58 px) que recorre, 8 de aire y la micro-etiqueta
/// `faint`. Alto `h::HEAD`.
pub fn busy(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    const METER_W: f32 = 12.0 * 5.0 - 2.0;
    let g = ui.painter().layout_job(micro_job(text, t.faint));
    let w = METER_W + if text.is_empty() { 0.0 } else { sp::S + g.size().x };
    let (rect, r) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
    let p = ui.painter();
    dot_meter_busy(p, t, Rect::from_min_size(pos2(rect.left(), rect.center().y - 1.5), vec2(METER_W, 3.0)));
    if !text.is_empty() {
        p.galley(pos2(rect.left() + METER_W + sp::S, rect.center().y - g.size().y / 2.0), g, t.faint);
    }
    r
}

fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// Estado vacío: en un área grande, trama de 160×96, ícono, título y pista; en una chica (una
/// columna), solo la micro-etiqueta.
pub fn empty_state(p: &Painter, t: &Theme, area: Rect, icon_glyph: &str, title: &str, hint: &str) {
    empty_state_tone(p, t, area, icon_glyph, Tone::Neutral, title, hint);
}

/// Igual, con el ícono en el color del tono (un error: `Tone::Warn`).
pub fn empty_state_tone(p: &Painter, t: &Theme, area: Rect, icon_glyph: &str, tone: Tone, title: &str, hint: &str) {
    empty_state_lines(p, t, area, icon_glyph, tone, title, &[hint]);
}

/// Estado vacío con varias pistas, una por línea (18 px entre sí).
pub fn empty_state_lines(p: &Painter, t: &Theme, area: Rect, icon_glyph: &str, tone: Tone, title: &str, hints: &[&str]) {
    let icon_color = if tone == Tone::Neutral { t.faint } else { tone.text(t) };
    if area.width() < 320.0 || area.height() < 240.0 {
        micro(p, area.center(), Align2::CENTER_CENTER, title, icon_color);
        return;
    }
    let c = pos2(area.center().x, area.top() + area.height() / 3.0);
    let patch = Rect::from_center_size(c, vec2(160.0, 96.0));
    dot_grid(p, t, patch);
    p.text(c, Align2::CENTER_CENTER, icon_glyph, icon_font(icon::HERO), icon_color);
    p.text(pos2(c.x, patch.bottom() + sp::L), Align2::CENTER_TOP, title, text::strong(), t.dim);
    let mut y = patch.bottom() + sp::L + 22.0;
    for hint in hints.iter().filter(|h| !h.is_empty()) {
        p.text(pos2(c.x, y), Align2::CENTER_TOP, *hint, text::small(), t.faint);
        y += 18.0;
    }
}

/// Punto de color de 8 px (etiqueta). En claro lleva un anillo fino por dentro, para que el
/// amarillo se vea sobre blanco.
pub fn tag_dot(p: &Painter, t: &Theme, center: Pos2, color: Color32) {
    p.circle_filled(center, 4.0, color);
    if !t.dark {
        p.circle_stroke(center, 4.0 - stroke::HAIR / 2.0, Stroke::new(stroke::HAIR, Color32::from_black_alpha(41)));
    }
}

/// Fuente con nombre (para las pocas veces que hace falta armar una a mano).
pub fn family_medium() -> FontFamily {
    FontFamily::Name("semibold".into())
}

// ---------------------------------------------------------------- texto mono y rutas

/// Texto mono chico (`mono_s`) tal cual: sin mayúsculas ni aire entre letras (barra de estado).
/// Devuelve el rectángulo que ocupa.
pub fn mono_label(p: &Painter, pos: Pos2, align: Align2, text: &str, color: Color32) -> Rect {
    p.text(pos, align, text, self::text::mono_s(), color)
}

/// Una ruta en una sola línea: rótulo a la izquierda (`small`, `dim`) y la ruta a la derecha
/// (mono, `dim`). Si no entra, queda la raíz, `…` y los últimos tramos que entren; entonces la
/// ruta completa va al tooltip.
pub fn meta_row_path(ui: &mut Ui, t: &Theme, key: &str, path: &str) -> Response {
    let w = ui.available_width();
    let p = ui.painter().clone();
    let (rect, r) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
    p.text(rect.left_center(), Align2::LEFT_CENTER, key, text::small(), t.dim);
    let room = (w - text_width(&p, key, text::small()) - sp::M).max(0.0);
    let shown = elide_middle(&p, path, text::mono(), room);
    p.text(rect.right_center(), Align2::RIGHT_CENTER, &shown, text::mono(), t.dim);
    if shown != path { tip(r, t, path) } else { r }
}

/// Acorta una ruta (o un texto) por el medio para que entre en `max_w` con `font`: queda la raíz,
/// `…` y los últimos tramos que entren (`C:\…\fotos\2024`). Sin separadores, el final con `…`.
pub fn elide_middle(p: &Painter, path: &str, font: FontId, max_w: f32) -> String {
    let fits = |s: &str| text_width(p, s, font.clone()) <= max_w;
    if fits(path) {
        return path.to_string();
    }
    let sep = if path.contains('\\') { '\\' } else { '/' };
    let parts: Vec<&str> = path.split(sep).collect();
    // Raíz: la unidad («C:»), el servidor y el recurso de una ruta UNC, o el esquema y el host.
    let n_root = if path.starts_with(r"\\") {
        4
    } else if path.contains("://") {
        3
    } else {
        1
    }
    .min(parts.len());
    let root = parts[..n_root].join(&sep.to_string());
    let rest = &parts[n_root..];
    // Cuantos más tramos del final, mejor (siempre se quita al menos uno).
    for k in (1..rest.len()).rev() {
        let s = format!("{root}{sep}…{sep}{}", rest[rest.len() - k..].join(&sep.to_string()));
        if fits(&s) {
            return s;
        }
    }
    // Ni el último tramo entra: el final de la ruta, con «…» adelante.
    let mut best = String::from("…");
    for (i, _) in path.char_indices().rev() {
        let s = format!("…{}", &path[i..]);
        if !fits(&s) {
            break;
        }
        best = s;
    }
    best
}

// ---------------------------------------------------------------- estado, espera y avisos

/// Ícono por defecto de un tono (línea de estado, aviso).
fn tone_icon(tone: Tone) -> &'static str {
    match tone {
        Tone::Ok => ic::CHECK_CIRCLE,
        Tone::Warn | Tone::Danger => ic::WARNING,
        Tone::Accent | Tone::Neutral => ic::INFO,
    }
}

/// Un texto con ajuste de línea en `max_w`.
fn wrapped(p: &Painter, s: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(s.to_string(), TextFormat::simple(font, color));
    job.wrap.max_width = max_w.max(1.0);
    p.layout_job(job)
}

/// Alto de la primera línea de un texto ya armado.
fn first_line_h(g: &egui::Galley) -> f32 {
    g.rows.first().map_or(g.size().y, |r| r.row.height())
}

/// Línea de estado: [ícono 16 en el color del tono][8][texto `small`]. Neutral: ícono y texto en
/// `dim`; el resto, ambos en `tone.text(t)`. Alto mínimo `h::HEAD`; si el texto ocupa varias
/// líneas, sangra bajo el texto. `icon` vacío: el del tono (Ok ✓, Warn y Danger ⚠, el resto ⓘ).
pub fn status_line(ui: &mut Ui, t: &Theme, tone: Tone, icon: &str, text: &str) -> Response {
    let color = if tone == Tone::Neutral { t.dim } else { tone.text(t) };
    let glyph = if icon.is_empty() { tone_icon(tone) } else { icon };
    let x0 = icon::M + sp::S;
    let g = wrapped(ui.painter(), text, text::small(), color, ui.available_width() - x0);
    let line_h = first_line_h(&g);
    let top = ((h::HEAD - line_h) / 2.0).max(0.0);
    let (rect, r) = ui.allocate_exact_size(vec2(x0 + g.size().x, (g.size().y + 2.0 * top).max(h::HEAD)), Sense::hover());
    let p = ui.painter();
    p.text(pos2(rect.left() + icon::M / 2.0, rect.top() + top + line_h / 2.0), Align2::CENTER_CENTER, glyph, icon_font(icon::M), color);
    p.galley(pos2(rect.left() + x0, rect.top() + top), g, color);
    r
}

/// Ancho máximo de un tooltip, por fuera.
const TIP_MAX_W: f32 = 320.0;

/// Tooltip con el marco de Rayo (`tooltip_frame`, `small`, máx. 320).
pub fn tip(resp: Response, t: &Theme, text: &str) -> Response {
    tip_keys(resp, t, text, "")
}

/// Tooltip con el atajo como tecla en línea al final («Ctrl 2»).
pub fn tip_keys(resp: Response, t: &Theme, text: &str, keys: &str) -> Response {
    tip_keys_at(resp, t, text, keys, None)
}

/// Como `tip_keys`, pero siempre a un costado del control (`RectAlign::LEFT` o `RIGHT`), sin
/// probar otros lados: para cuando arriba y abajo hay una ventana nativa que lo taparía (video
/// o visor de Windows en la Vista rápida).
pub fn tip_keys_beside(resp: Response, t: &Theme, text: &str, keys: &str, side: egui::RectAlign) -> Response {
    tip_keys_at(resp, t, text, keys, Some(side))
}

fn tip_keys_at(resp: Response, t: &Theme, text: &str, keys: &str, side: Option<egui::RectAlign>) -> Response {
    if text.is_empty() && keys.is_empty() {
        return resp;
    }
    let mut tt = egui::Tooltip::for_enabled(&resp);
    tt.popup = tt.popup.frame(tooltip_frame(t));
    if let Some(a) = side {
        tt.popup = tt.popup.align(a).align_alternatives(&[]).gap(sp::XS);
    }
    tt.show(|ui| {
        let p = ui.painter().clone();
        let gap = if text.is_empty() || keys.is_empty() { 0.0 } else { sp::S };
        let kw = if keys.is_empty() { 0.0 } else { text_width(&p, keys, text::mono_s()) + 10.0 };
        let g = wrapped(&p, text, text::small(), t.text, TIP_MAX_W - 2.0 * sp::S - 2.0 * stroke::HAIR - kw - gap);
        let gw = if text.is_empty() { 0.0 } else { g.size().x };
        let (rect, _) = ui.allocate_exact_size(vec2(gw + gap + kw, g.size().y.max(h::KBD)), Sense::hover());
        if !text.is_empty() {
            p.galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.text);
        }
        if !keys.is_empty() {
            kbd_paint(&p, t, pos2(rect.left() + gw + gap, rect.center().y), keys);
        }
    });
    resp
}

/// Ancho máximo de un aviso, por fuera.
const TOAST_MAX_W: f32 = 480.0;

/// Un aviso (4.12).
#[derive(Clone, Copy, Default)]
pub struct Toast<'a> {
    pub text: &'a str,
    /// Neutral = sin ícono.
    pub tone: Tone,
    /// Tecla en línea al final del texto.
    pub keys: &'a str,
    /// Botón fantasma de 20 al final («Deshacer»).
    pub action: &'a str,
}

/// Contenido del aviso dentro de su `Area`: `popover_frame` con margen 12×8, ancho máx. 480.
/// Devuelve true si se apretó la acción.
pub fn toast(ui: &mut Ui, t: &Theme, spec: Toast) -> bool {
    let mut hit = false;
    popover_frame(t).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            let p = ui.painter().clone();
            let iw = if spec.tone == Tone::Neutral { 0.0 } else { icon::M + sp::S };
            let kw = if spec.keys.is_empty() { 0.0 } else { text_width(&p, spec.keys, text::mono_s()) + 10.0 + sp::S };
            let aw = if spec.action.is_empty() { 0.0 } else { text_width(&p, spec.action, text::body()) + 2.0 * sp::S + sp::M };
            if spec.tone != Tone::Neutral {
                let (r, _) = ui.allocate_exact_size(vec2(icon::M, icon::M), Sense::hover());
                // El aviso de error lleva la X en círculo (M-18), no el triángulo de advertencia.
                let glyph = if spec.tone == Tone::Danger { ic::X_CIRCLE } else { tone_icon(spec.tone) };
                p.text(r.center(), Align2::CENTER_CENTER, glyph, icon_font(icon::M), spec.tone.text(t));
            }
            let g = wrapped(&p, spec.text, text::body(), t.text, TOAST_MAX_W - 2.0 * sp::M - 2.0 * stroke::HAIR - iw - kw - aw);
            let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
            p.galley(r.min, g, t.text);
            if !spec.keys.is_empty() {
                kbd(ui, t, spec.keys);
            }
            if !spec.action.is_empty() {
                ui.add_space(sp::XS);
                let w = text_width(&p, spec.action, text::body()) + 2.0 * sp::S;
                let (rect, r) = ui.allocate_exact_size(vec2(w, h::CHIP), Sense::click());
                let hov = hover_amount(ui.ctx(), r.id, r.hovered());
                state_fill(&p, t, rect, radius::M, hov, r.is_pointer_button_down_on(), Sel::None);
                p.text(rect.center(), Align2::CENTER_CENTER, spec.action, text::body(), if r.hovered() { t.text } else { t.dim });
                hit = r.clicked();
            }
        });
    });
    hit
}

// ---------------------------------------------------------------- filas de dos líneas

/// Fila de dos líneas (procesos de bloqueos, trabajos de copia).
#[derive(Clone, Copy, Default)]
pub struct Row2<'a> {
    pub icon: &'a str,
    /// `dim` si es None.
    pub icon_color: Option<Color32>,
    /// `body`, `text`.
    pub title: &'a str,
    /// `small` `faint`, o `mono_s` `dim` si es dato (`sub_mono`).
    pub sub: &'a str,
    pub sub_mono: bool,
    pub sub_color: Option<Color32>,
    /// Distinto de Neutral: fondo `tone.soft(t)` (confirmación en la fila).
    pub tone: Tone,
}

/// Alto `h::DRIVE` (40), pill de radio M, ícono 16 en x = 8, título centrado en y = 13 y sub en
/// y = 28. `trailing` se dibuja de derecha a izquierda, centrado en vertical.
pub fn row2(ui: &mut Ui, t: &Theme, id: Id, spec: Row2, trailing: impl FnOnce(&mut Ui)) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::DRIVE), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    let pill = rect.shrink2(vec2(sp::XS, 1.0));
    let p = ui.painter().clone();
    if spec.tone != Tone::Neutral {
        p.rect_filled(pill, radius::M, spec.tone.soft(t));
    }
    // Con el cursor en un botón del final la fila sigue con hover.
    let hov = hover_amount(ui.ctx(), id, ui.rect_contains_pointer(pill));
    state_fill(&p, t, pill, radius::M, hov, false, Sel::None);
    let mut x = pill.left() + sp::S;
    if !spec.icon.is_empty() {
        p.text(pos2(x + icon::M / 2.0, pill.center().y), Align2::CENTER_CENTER, spec.icon, icon_font(icon::M), spec.icon_color.unwrap_or(t.dim));
        x += icon::M + sp::S;
    }
    let end = Rect::from_min_max(pos2(x, pill.top()), pos2(pill.right() - sp::S, pill.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(end).layout(egui::Layout::right_to_left(egui::Align::Center)));
    trailing(&mut child);
    let used = child.min_rect();
    let right = if used.width() > 0.5 { used.left() - sp::S } else { end.right() };
    let max_w = (right - x).max(0.0);
    let line = |s: &str, font: FontId, color: Color32, cy: f32| {
        let mut job = LayoutJob::single_section(s.to_string(), TextFormat::simple(font, color));
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let g = p.layout_job(job);
        p.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    };
    line(spec.title, text::body(), t.text, rect.top() + 13.0);
    if !spec.sub.is_empty() {
        let (font, color) = if spec.sub_mono { (text::mono_s(), t.dim) } else { (text::small(), t.faint) };
        line(spec.sub, font, spec.sub_color.unwrap_or(color), rect.top() + 28.0);
    }
    r
}

// ---------------------------------------------------------------- controles

/// Selector desplegable (sustituye a `egui::ComboBox`): control de 28, relleno `sidebar`, borde
/// `border_strong` (hover `faint`, abierto anillo de foco), texto `body`, caret de 12 en `faint`.
/// Abre `popup_menu` con la opción actual marcada. Devuelve si cambió.
pub fn select<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, id: impl std::hash::Hash, value: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width, h::CTL), Sense::hover());
    let r = ui.interact(rect, any_id(id), Sense::click());
    let mut changed = false;
    let open = egui::Popup::menu(&r)
        .frame(popover_frame(t))
        .show(|ui| {
            menu_body(ui);
            // Al menos tan ancho como el control (sin el margen de 4 ni el borde).
            ui.set_min_width(width - 2.0 * sp::XS - 2.0 * stroke::HAIR);
            for (v, label) in options {
                if menu_item_ex(ui, t, MenuItem { label, checked: Some(*v == *value), ..Default::default() }).clicked() && *v != *value {
                    *value = *v;
                    changed = true;
                }
            }
        })
        .is_some();
    let p = ui.painter();
    p.rect_filled(rect, radius::M, t.sidebar);
    if open || r.has_focus() {
        focus_ring(p, t, rect, radius::M);
    } else {
        p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, if r.hovered() { t.faint } else { t.border_strong }), StrokeKind::Inside);
    }
    let caret_x = rect.right() - sp::S - icon::S / 2.0;
    p.text(pos2(caret_x, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, icon_font(icon::S), t.faint);
    let label = options.iter().find(|o| o.0 == *value).map_or("", |o| o.1);
    let mut job = LayoutJob::single_section(label.to_string(), TextFormat::simple(text::body(), t.text));
    job.wrap = egui::text::TextWrapping::truncate_at_width((caret_x - icon::S / 2.0 - sp::S - rect.left() - sp::M).max(0.0));
    let g = p.layout_job(job);
    p.galley(pos2(rect.left() + sp::M, rect.center().y - g.size().y / 2.0), g, t.text);
    changed
}

/// Casilla (4.6): glifo de 16, marcada en acento (relleno) o vacía en `faint`, centrada en `rect`.
pub fn check_icon(p: &Painter, t: &Theme, rect: Rect, on: bool) {
    let (glyph, color) = if on { (egui_phosphor::fill::CHECK_SQUARE, t.accent) } else { (ic::SQUARE, t.faint) };
    p.text(rect.center(), Align2::CENTER_CENTER, glyph, if on { icon_fill(icon::M) } else { icon_font(icon::M) }, color);
}

/// Casilla con texto (sustituye a `ui.checkbox`). El texto se parte en el ancho disponible (no
/// ensancha el diálogo); la casilla queda a la altura de la primera línea. Devuelve la respuesta
/// (cambiada al tocarla).
pub fn checkbox(ui: &mut Ui, t: &Theme, on: &mut bool, label: &str) -> Response {
    let x0 = icon::M + sp::S;
    let g = wrapped(ui.painter(), label, text::body(), t.text, ui.available_width() - x0);
    let line_h = first_line_h(&g);
    let top = ((h::CTL - line_h) / 2.0).max(0.0);
    let (rect, mut r) = ui.allocate_exact_size(vec2(x0 + g.size().x, (g.size().y + 2.0 * top).max(h::CTL)), Sense::click());
    if r.clicked() {
        *on ^= true;
        r.mark_changed();
    }
    let p = ui.painter();
    let cy = rect.top() + top + line_h / 2.0;
    check_icon(p, t, Rect::from_center_size(pos2(rect.left() + icon::M / 2.0, cy), vec2(icon::M, icon::M)), *on);
    p.galley(pos2(rect.left() + x0, rect.top() + top), g, t.text);
    r
}

/// Enlace: `small`, `accent`; con hover, `accent_hover` y subrayado.
pub fn link(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    let size = ui.painter().layout_no_wrap(text.to_string(), text::small(), t.accent).size();
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let hov = r.hovered();
    let color = if hov { t.accent_hover } else { t.accent };
    let mut fmt = TextFormat::simple(text::small(), color);
    if hov {
        fmt.underline = Stroke::new(stroke::HAIR, color);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let g = ui.painter().layout_job(LayoutJob::single_section(text.to_string(), fmt));
    ui.painter().galley(rect.min, g, color);
    if r.has_focus() {
        focus_ring(ui.painter(), t, rect.expand(2.0), radius::S);
    }
    r
}

/// Campo multilínea: relleno `sidebar`, borde HAIR `border_strong` (hover `faint`; FOCUS
/// `accent` con foco; `danger` si `bad`), radio M, margen 8, mono 12 si `mono`. Devuelve la
/// respuesta del texto.
pub fn textarea(ui: &mut Ui, t: &Theme, id: Id, buf: &mut String, rows: usize, mono: bool, bad: bool) -> Response {
    let focused = ui.memory(|m| m.has_focus(id));
    let font = if mono { text::mono() } else { text::body() };
    let out = Frame::new().fill(t.sidebar).corner_radius(radius::M).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.add(egui::TextEdit::multiline(buf).id(id).frame(Frame::NONE).font(font).desired_rows(rows).desired_width(f32::INFINITY))
    });
    let rect = out.response.rect;
    let (w, c) = if bad {
        (if focused { stroke::FOCUS } else { stroke::HAIR }, t.danger)
    } else if focused {
        (stroke::FOCUS, t.accent)
    } else if ui.rect_contains_pointer(rect) {
        (stroke::HAIR, t.faint)
    } else {
        (stroke::HAIR, t.border_strong)
    };
    ui.painter().rect_stroke(rect, radius::M, Stroke::new(w, c), StrokeKind::Inside);
    out.inner
}
