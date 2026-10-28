//! Controles con el estilo de Rayo para los diálogos y herramientas: campo de texto,
//! botón principal, botón de ícono y selector segmentado.

use super::theme::{self, Theme};
use egui::{Align2, Color32, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

/// Verde de "todo bien" (coincide, listo).
pub const GOOD: Color32 = Color32::from_rgb(0x3F, 0xB9, 0x50);

/// Cómo se pinta el borde de un campo.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Normal,
    Good,
    Bad,
}

pub struct Field<'a> {
    pub id: Id,
    pub icon: &'static str,
    pub hint: &'a str,
    pub width: f32,
    pub mono: bool,
    pub tone: Tone,
    /// Botón de ícono al final (p. ej. "Elegir carpeta…"): (ícono, ayuda).
    pub trailing: Option<(&'static str, &'a str)>,
    /// Se puede soltar algo encima (se resalta mientras se arrastra).
    pub droppable: bool,
    /// Alto (FIELD_H por defecto; más alto = letra más grande, como en la paleta de comandos).
    pub height: f32,
    /// Sin X para vaciarlo (campos chicos, como la extensión).
    pub no_clear: bool,
    /// Que Tab / ↑ ↓ no saquen el foco del campo.
    pub lock_focus: bool,
}

impl<'a> Field<'a> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, icon: &'static str, hint: &'a str) -> Self {
        Field { id: Id::new(id), icon, hint, width: f32::INFINITY, mono: false, tone: Tone::Normal, trailing: None, droppable: false, height: FIELD_H, no_clear: false, lock_focus: false }
    }
}

pub struct FieldOut {
    /// Todo el campo (para soltar cosas encima).
    pub outer: Response,
    /// El texto en sí (foco, cambios, Enter).
    pub edit: Response,
    pub trailing_clicked: bool,
    /// Se vació con la X.
    pub cleared: bool,
}

pub const FIELD_H: f32 = 34.0;

/// Campo de texto: fondo hundido, ícono, borde que se ilumina con el foco (o en verde / rojo
/// según `tone`), X para vaciarlo y, opcional, un botón al final.
pub fn field(ui: &mut Ui, t: &Theme, buf: &mut String, f: Field) -> FieldOut {
    let w = f.width.min(ui.available_width());
    let (rect, outer) = ui.allocate_exact_size(vec2(w, f.height), Sense::click());
    let big = f.height >= 40.0;
    let focused = ui.memory(|m| m.has_focus(f.id));
    let dragging = f.droppable && egui::DragAndDrop::has_any_payload(ui.ctx());
    let drop_here = dragging && outer.contains_pointer();
    let p = ui.painter().clone();
    let border = match f.tone {
        _ if drop_here || focused && f.tone == Tone::Normal => t.accent,
        Tone::Good => GOOD,
        Tone::Bad => t.danger,
        Tone::Normal if outer.hovered() || dragging => t.faint,
        Tone::Normal => t.border,
    };
    let fill = if drop_here { t.accent.gamma_multiply(0.12) } else { t.bg };
    p.rect_filled(rect, 8.0, fill);
    p.rect_stroke(rect, 8.0, Stroke::new(if focused || drop_here { 1.5 } else { 1.0 }, border), StrokeKind::Inside);
    // Anillo suave alrededor con el foco, como los campos de Windows 11.
    if focused && f.tone == Tone::Normal {
        p.rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0, t.accent.gamma_multiply(0.25)), StrokeKind::Inside);
    }
    let icon_color = match f.tone {
        Tone::Good => GOOD,
        Tone::Bad => t.danger,
        Tone::Normal if focused || drop_here => t.accent,
        Tone::Normal => t.dim,
    };
    let icon_x = if big { 18.0 } else { 15.0 };
    p.text(pos2(rect.left() + icon_x, rect.center().y), Align2::CENTER_CENTER, f.icon, theme::icon_font(if big { 17.0 } else { 15.0 }, false), icon_color);

    // A la derecha: botón final y X para vaciar.
    let mut right = rect.right() - 4.0;
    let mut trailing_clicked = false;
    if let Some((icon, tip)) = f.trailing {
        let br = Rect::from_center_size(pos2(right - 13.0, rect.center().y), vec2(26.0, 26.0));
        let r = icon_button_at(ui, t, br, f.id.with("trailing"), icon, false).on_hover_text(tip);
        trailing_clicked = r.clicked();
        right = br.left() - 2.0;
    }
    let mut cleared = false;
    if !buf.is_empty() && !f.no_clear {
        let xr = Rect::from_center_size(pos2(right - 12.0, rect.center().y), vec2(22.0, 22.0));
        let r = ui.interact(xr, f.id.with("clear"), Sense::click());
        if r.hovered() {
            p.rect_filled(xr, 5.0, t.hover);
        }
        p.text(xr.center(), Align2::CENTER_CENTER, ic::X, theme::icon_font(12.0, false), if r.hovered() { t.text } else { t.faint });
        if r.clicked() {
            buf.clear();
            cleared = true;
        }
        right = xr.left() - 2.0;
    }

    let edit_rect = Rect::from_min_max(pos2(rect.left() + if big { 38.0 } else { 32.0 }, rect.top() + 4.0), pos2(right - 2.0, rect.bottom() - 4.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let font = if f.mono { FontId::monospace(12.5) } else { FontId::proportional(if big { 15.0 } else { 13.5 }) };
    let edit = child.add(
        egui::TextEdit::singleline(buf)
            .id(f.id)
            .frame(egui::Frame::NONE)
            .font(font)
            .hint_text(egui::RichText::new(f.hint).color(t.faint).size(if big { 15.0 } else { 13.0 }))
            .lock_focus(f.lock_focus)
            .desired_width(edit_rect.width())
            .vertical_align(egui::Align::Center),
    );
    // Un clic en el ícono o el margen también lleva el cursor al campo.
    if outer.clicked() || cleared {
        edit.request_focus();
    }
    if outer.hovered() && !drop_here {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    FieldOut { outer, edit, trailing_clicked, cleared }
}

/// Botón de ícono cuadrado, sin fondo hasta pasar por encima.
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, size: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let id = ui.next_auto_id().with(icon);
    icon_button_at(ui, t, rect, id, icon, false)
}

/// Igual, pero con un borde (para que se vea como botón aunque esté quieto).
pub fn icon_button_outlined(ui: &mut Ui, t: &Theme, icon: &str, size: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let id = ui.next_auto_id().with(icon);
    icon_button_at(ui, t, rect, id, icon, true)
}

fn icon_button_at(ui: &mut Ui, t: &Theme, rect: Rect, id: Id, icon: &str, outlined: bool) -> Response {
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    if r.is_pointer_button_down_on() {
        p.rect_filled(rect, 6.0, t.border);
    } else if r.hovered() {
        p.rect_filled(rect, 6.0, t.hover);
    }
    if outlined {
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    }
    p.text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, false), if r.hovered() { t.text } else { t.dim });
    r
}

/// Botón principal (color de acento). Desactivado: gris y sin respuesta.
pub fn primary_button(ui: &mut Ui, t: &Theme, icon: &str, text: &str, enabled: bool) -> Response {
    let font = theme::semibold(13.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), t.accent_text);
    let icon_w = if icon.is_empty() { 0.0 } else { 22.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + icon_w + 28.0, FIELD_H), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let fill = if !enabled {
        t.hover
    } else if r.is_pointer_button_down_on() {
        t.sel
    } else if r.hovered() {
        t.accent.gamma_multiply(1.12)
    } else {
        t.accent
    };
    p.rect_filled(rect, 8.0, fill);
    let fg = if enabled { t.accent_text } else { t.faint };
    let mut x = rect.left() + 14.0;
    if !icon.is_empty() {
        p.text(pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, false), fg);
        x += icon_w;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, font, fg);
    if enabled && r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r
}

/// Botón secundario: borde fino, mismo alto que los campos.
pub fn secondary_button(ui: &mut Ui, t: &Theme, icon: &str, text: &str) -> Response {
    let font = FontId::proportional(13.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), t.text);
    let icon_w = if icon.is_empty() { 0.0 } else { 22.0 };
    let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + icon_w + 26.0, FIELD_H), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 8.0, if r.is_pointer_button_down_on() { t.border } else if r.hovered() { t.hover } else { Color32::TRANSPARENT });
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let mut x = rect.left() + 13.0;
    if !icon.is_empty() {
        p.text(pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, false), t.dim);
        x += icon_w;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, font, t.text);
    r
}

/// Selector de opciones excluyentes, como pastillas pegadas. Devuelve si cambió.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = options.iter().map(|(_, l, _)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), t.text).size().x + 24.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 8.0, t.bg);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let mut x = rect.left() + 3.0;
    let mut changed = false;
    for (k, (v, label, tip)) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(widths[k], rect.height() - 6.0));
        let resp = ui.interact(r, ui.id().with(("seg", label)), Sense::click()).on_hover_text(*tip);
        let on = *value == *v;
        if on {
            p.rect_filled(r, 6.0, t.sel_inactive);
            p.rect_stroke(r, 6.0, Stroke::new(1.0, t.accent.gamma_multiply(0.55)), StrokeKind::Inside);
        } else if resp.hovered() {
            p.rect_filled(r, 6.0, t.hover);
        }
        p.text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), if on { t.text } else { t.dim });
        if resp.clicked() && !on {
            *value = *v;
            changed = true;
        }
        x += widths[k];
    }
    changed
}

/// Rótulo chico encima de un campo.
pub fn caption(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(t.dim));
}

/// Inserta `text` donde está el cursor del campo `id` (o al final) y deja el cursor después.
pub fn insert_at_cursor(ctx: &egui::Context, id: Id, buf: &mut String, text: &str) {
    let state = egui::TextEdit::load_state(ctx, id);
    let at_char = state.as_ref().and_then(|s| s.cursor.char_range()).map(|r| usize::from(r.primary.index)).unwrap_or(usize::MAX);
    let byte = buf.char_indices().nth(at_char).map(|(b, _)| b).unwrap_or(buf.len());
    buf.insert_str(byte, text);
    let new_char = buf[..byte + text.len()].chars().count();
    let mut st = state.unwrap_or_default();
    st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(new_char))));
    st.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Pastilla chica clicable (p. ej. para insertar `{n}` en una plantilla).
pub fn chip(ui: &mut Ui, t: &Theme, text: &str, mono: bool) -> Response {
    let font = if mono { FontId::monospace(11.5) } else { FontId::proportional(12.0) };
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), t.text);
    let (rect, r) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 24.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, if r.is_pointer_button_down_on() { t.border } else if r.hovered() { t.hover } else { t.bg });
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, if r.hovered() { t.faint } else { t.border }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, text, font, if r.hovered() { t.text } else { t.dim });
    r
}

/// Interruptor como pastilla (encendido = acento). Devuelve si cambió.
pub fn toggle_chip(ui: &mut Ui, t: &Theme, on: &mut bool, icon: &str, text: &str) -> Response {
    let font = FontId::proportional(12.5);
    let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), t.text);
    let icon_w = if icon.is_empty() { 0.0 } else { 20.0 };
    let (rect, mut r) = ui.allocate_exact_size(vec2(g.size().x + icon_w + 22.0, FIELD_H), Sense::click());
    if r.clicked() {
        *on ^= true;
        r.mark_changed();
    }
    let p = ui.painter();
    let (fill, stroke, fg) = if *on {
        (t.accent.gamma_multiply(0.18), t.accent.gamma_multiply(0.7), t.text)
    } else if r.hovered() {
        (t.hover, t.faint, t.text)
    } else {
        (t.bg, t.border, t.dim)
    };
    p.rect_filled(rect, 8.0, fill);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let mut x = rect.left() + 11.0;
    if !icon.is_empty() {
        p.text(pos2(x + 7.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon_font(13.0, false), if *on { t.accent } else { fg });
        x += icon_w;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, font, fg);
    r
}

/// Número con − y + a los lados (y la rueda del mouse encima). Devuelve si cambió.
pub fn stepper(ui: &mut Ui, t: &Theme, value: &mut u32, min: u32, max: u32, width: f32) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, 8.0, t.bg);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, if r.hovered() { t.faint } else { t.border }), StrokeKind::Inside);
    let before = *value;
    let minus = Rect::from_min_size(rect.min + vec2(3.0, 4.0), vec2(26.0, rect.height() - 8.0));
    let plus = Rect::from_min_size(pos2(rect.right() - 29.0, rect.top() + 4.0), vec2(26.0, rect.height() - 8.0));
    for (br, icon, delta) in [(minus, ic::MINUS, -1i64), (plus, ic::PLUS, 1i64)] {
        let b = ui.interact(br, ui.id().with(("stepper", rect.min.x as i32, rect.min.y as i32, delta)), Sense::click());
        let can = if delta < 0 { *value > min } else { *value < max };
        if b.hovered() && can {
            p.rect_filled(br, 6.0, t.hover);
        }
        p.text(br.center(), Align2::CENTER_CENTER, icon, theme::icon_font(12.0, false), if can { t.dim } else { t.faint.gamma_multiply(0.5) });
        if b.clicked() && can {
            *value = (*value as i64 + delta).clamp(min as i64, max as i64) as u32;
        }
    }
    if r.hovered() {
        let scroll: f32 = ui.input(|i| i.raw.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum());
        if scroll > 0.0 && *value < max {
            *value += 1;
        } else if scroll < 0.0 && *value > min {
            *value -= 1;
        }
    }
    p.text(rect.center(), Align2::CENTER_CENTER, value.to_string(), FontId::proportional(13.5), t.text);
    *value != before
}
