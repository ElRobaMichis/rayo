//! Controles con el estilo de Rayo para los diálogos y herramientas: campo de texto,
//! botones, chips, selector segmentado y número con − / +. Los dibujan las piezas de
//! `design.rs`; aquí quedan las firmas que usa el resto de la app.

use super::design::{self, Chip, IconBtn, Kind, Sel, h, icon, radius, sp, stroke, text};
use super::theme::Theme;
use egui::{Align2, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;

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
    /// Alto (`h::CTL` por defecto; `h::PALETTE` = campo de la paleta, con letra de título y sin borde).
    pub height: f32,
    /// Sin X para vaciarlo (campos chicos, como la extensión).
    pub no_clear: bool,
    /// Que Tab / ↑ ↓ no saquen el foco del campo.
    pub lock_focus: bool,
    /// Contraseña: se muestran puntos.
    pub password: bool,
    /// Sin foco, la ruta se acorta por el medio (`design::elide_middle`); con foco, el texto real.
    pub elide_middle: bool,
}

impl<'a> Field<'a> {
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, icon: &'static str, hint: &'a str) -> Self {
        Field { id: Id::new(id), icon, hint, width: f32::INFINITY, mono: false, tone: Tone::Normal, trailing: None, droppable: false, height: FIELD_H, no_clear: false, lock_focus: false, password: false, elide_middle: false }
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

/// Alto de un control (alias de `h::CTL`, lo usan otros archivos).
pub const FIELD_H: f32 = h::CTL;

/// Campo de texto (4.6): relleno hundido, ícono, borde fino que pasa a acento con el foco (o a
/// verde / rojo según `tone`), X para vaciarlo y, opcional, un botón al final.
pub fn field(ui: &mut Ui, t: &Theme, buf: &mut String, f: Field) -> FieldOut {
    let w = f.width.min(ui.available_width());
    let (rect, outer) = ui.allocate_exact_size(vec2(w, f.height), Sense::click());
    let big = f.height >= h::PALETTE;
    let focused = ui.memory(|m| m.has_focus(f.id));
    let dragging = f.droppable && egui::DragAndDrop::has_any_payload(ui.ctx());
    let drop_here = dragging && outer.contains_pointer();
    let p = ui.painter().clone();
    p.rect_filled(rect, radius::M, if drop_here { t.accent_soft } else { t.sidebar });
    // Borde: el de la paleta no lleva (salvo para soltar o con tono).
    let border = match f.tone {
        _ if drop_here => Some(t.accent),
        Tone::Good => Some(t.ok),
        Tone::Bad => Some(t.danger),
        Tone::Normal if big => None,
        Tone::Normal if focused => Some(t.accent),
        Tone::Normal if outer.hovered() || dragging => Some(t.faint),
        Tone::Normal => Some(t.border_strong),
    };
    if let Some(b) = border {
        let width = if focused || drop_here { stroke::FOCUS } else { stroke::HAIR };
        p.rect_stroke(rect, radius::M, Stroke::new(width, b), StrokeKind::Inside);
    }
    let icon_color = match f.tone {
        Tone::Good => t.ok,
        Tone::Bad => t.danger,
        Tone::Normal if drop_here => t.accent,
        Tone::Normal => t.dim,
    };
    let pad = if big { sp::M } else { sp::S };
    p.text(pos2(rect.left() + pad + icon::M / 2.0, rect.center().y), Align2::CENTER_CENTER, f.icon, design::icon_font(icon::M), icon_color);

    // A la derecha: botón final y X para vaciar.
    let mut right = rect.right() - sp::XXS;
    let mut trailing_clicked = false;
    if let Some((glyph, tip)) = f.trailing {
        let s = rect.height() - 2.0 * sp::XXS;
        let br = Rect::from_min_size(pos2(right - s, rect.center().y - s / 2.0), vec2(s, s));
        let r = design::icon_button_at(ui, t, br, f.id.with("trailing"), glyph, IconBtn::default());
        let r = if tip.is_empty() { r } else { design::tip(r, t, tip) };
        trailing_clicked = r.clicked();
        right = br.left();
    }
    let mut cleared = false;
    if !buf.is_empty() && !f.no_clear {
        let xr = Rect::from_min_size(pos2(right - sp::XXS - h::CHIP, rect.center().y - h::CHIP / 2.0), vec2(h::CHIP, h::CHIP));
        if design::icon_button_at(ui, t, xr, f.id.with("clear"), ic::X, IconBtn::mini()).clicked() {
            buf.clear();
            cleared = true;
        }
        right = xr.left();
    }

    let text_x = rect.left() + pad + icon::M + sp::S;
    let edit_rect = Rect::from_min_max(pos2(text_x, rect.top() + sp::XXS), pos2(right - sp::XS, rect.bottom() - sp::XXS));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let font = if big {
        text::title()
    } else if f.mono {
        text::mono()
    } else {
        text::body()
    };
    let hint_font = if big { text::title() } else { text::body() };
    // Sin foco, la ruta acortada por el medio en lugar del `TextEdit` (con el mismo id: un clic o
    // Tab le dan el foco y en el cuadro siguiente aparece el campo de verdad).
    if f.elide_middle && !focused && !buf.is_empty() && !f.password {
        let edit = child.interact(edit_rect, f.id, Sense::click());
        let shown = design::elide_middle(&p, buf, font.clone(), edit_rect.width());
        p.text(pos2(edit_rect.left(), rect.center().y), Align2::LEFT_CENTER, &shown, font, t.text);
        if outer.clicked() || edit.clicked() || cleared {
            edit.request_focus();
        }
        if (outer.hovered() || edit.hovered()) && !drop_here {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        let edit = if shown != *buf { design::tip(edit, t, buf) } else { edit };
        return FieldOut { outer, edit, trailing_clicked, cleared };
    }
    let edit = child.add(
        egui::TextEdit::singleline(buf)
            .id(f.id)
            .frame(egui::Frame::NONE)
            .font(font)
            .hint_text(egui::RichText::new(f.hint).color(t.faint).font(hint_font))
            .lock_focus(f.lock_focus)
            .password(f.password)
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

/// Botón principal (relleno de acento). Desactivado: gris y sin respuesta.
pub fn primary_button(ui: &mut Ui, t: &Theme, glyph: &str, label: &str, enabled: bool) -> Response {
    design::button(ui, t, Kind::Primary, glyph, label, enabled)
}

/// Botón secundario: transparente con borde fino.
pub fn secondary_button(ui: &mut Ui, t: &Theme, glyph: &str, label: &str) -> Response {
    design::button(ui, t, Kind::Secondary, glyph, label, true)
}

/// Selector de opciones excluyentes (4.6): contenedor hundido, la activa con la capa de
/// seleccionado. Devuelve si cambió.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let font = text::body();
    let widths: Vec<f32> = options.iter().map(|(_, l, _)| design::text_width(ui.painter(), l, font.clone()) + 2.0 * sp::M).collect();
    let total: f32 = widths.iter().sum::<f32>() + 2.0 * sp::XXS;
    let (rect, _) = ui.allocate_exact_size(vec2(total, h::CTL), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, radius::M, t.sidebar);
    p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, t.border), StrokeKind::Inside);
    let mut x = rect.left() + sp::XXS;
    let mut changed = false;
    for (k, (v, label, tip)) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + sp::XXS), vec2(widths[k], rect.height() - 2.0 * sp::XXS));
        let id = ui.id().with(("seg", label));
        let resp = ui.interact(r, id, Sense::click());
        // Sin ayuda no se muestra un tooltip vacío.
        let resp = if tip.is_empty() { resp } else { design::tip(resp, t, tip) };
        let on = *value == *v;
        let hov = if on { 0.0 } else { design::hover_amount(ui.ctx(), id, resp.hovered()) };
        design::state_fill(&p, t, r, radius::S, hov, !on && resp.is_pointer_button_down_on(), if on { Sel::Unfocused } else { Sel::None });
        p.text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), if on || resp.hovered() { t.text } else { t.dim });
        if resp.clicked() && !on {
            *value = *v;
            changed = true;
        }
        x += widths[k];
    }
    changed
}

/// Rótulo chico encima de un campo.
pub fn caption(ui: &mut Ui, t: &Theme, label: &str) {
    ui.label(egui::RichText::new(label).font(text::small()).color(t.dim));
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

/// Chip neutro clicable (p. ej. para insertar `{n}` en una plantilla).
pub fn chip(ui: &mut Ui, t: &Theme, label: &str, mono: bool) -> Response {
    design::chip(ui, t, Chip { text: label, mono, tone: design::Tone::Neutral, interactive: true, ..Default::default() }).response
}

/// Número con − y + a los lados (y la rueda del mouse encima). Devuelve si cambió.
pub fn stepper(ui: &mut Ui, t: &Theme, value: &mut u32, min: u32, max: u32, width: f32) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(width, h::CTL), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, radius::M, t.sidebar);
    p.rect_stroke(rect, radius::M, Stroke::new(stroke::HAIR, if r.hovered() { t.faint } else { t.border_strong }), StrokeKind::Inside);
    let before = *value;
    let s = h::CTL - 2.0 * sp::XXS;
    let minus = Rect::from_min_size(rect.min + vec2(sp::XXS, sp::XXS), vec2(s, s));
    let plus = Rect::from_min_size(pos2(rect.right() - sp::XXS - s, rect.top() + sp::XXS), vec2(s, s));
    for (br, glyph, delta) in [(minus, ic::MINUS, -1i64), (plus, ic::PLUS, 1i64)] {
        let id = ui.id().with(("stepper", rect.min.x as i32, rect.min.y as i32, delta));
        let b = ui.interact(br, id, Sense::click());
        let can = if delta < 0 { *value > min } else { *value < max };
        let hov = if can { design::hover_amount(ui.ctx(), id, b.hovered()) } else { 0.0 };
        design::state_fill(&p, t, br, radius::S, hov, can && b.is_pointer_button_down_on(), Sel::None);
        let color = if !can {
            t.disabled
        } else if b.hovered() {
            t.text
        } else {
            t.dim
        };
        p.text(br.center(), Align2::CENTER_CENTER, glyph, design::icon_font(icon::S), color);
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
    p.text(rect.center(), Align2::CENTER_CENTER, value.to_string(), text::mono(), t.text);
    *value != before
}

/// Casilla (4.6): ver `design::check_icon`.
pub fn check_icon(p: &egui::Painter, t: &Theme, rect: Rect, on: bool) {
    design::check_icon(p, t, rect, on);
}

/// Casilla con texto: ver `design::checkbox`.
pub fn checkbox(ui: &mut Ui, t: &Theme, on: &mut bool, label: &str) -> Response {
    design::checkbox(ui, t, on, label)
}
