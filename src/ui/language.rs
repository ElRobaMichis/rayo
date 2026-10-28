//! Elegir el idioma de Rayo y ayudar a traducirlo (plantilla y revisión de traducciones).

use super::theme::{self, Theme};
use super::widgets;
use crate::i18n;
use egui::{Align2, FontId, Id, RichText, Sense, pos2, vec2};
use egui_phosphor::regular as ic;

pub struct LangDialog {
    list: Vec<i18n::Available>,
    report: Option<String>,
}

pub enum LangOut {
    Open,
    Close,
    /// Usar este idioma (None: el de Windows).
    Pick(Option<String>),
    /// Mostrar esta carpeta en Rayo (la de idiomas, con la plantilla).
    ShowFolder(String),
}

impl LangDialog {
    pub fn new() -> Self {
        LangDialog { list: i18n::available(), report: None }
    }

    pub fn ui(&mut self, ctx: &egui::Context, t: &Theme, chosen: Option<&str>) -> LangOut {
        let mut out = LangOut::Open;
        let modal = egui::Modal::new(Id::new("idioma")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(ic::TRANSLATE).font(theme::icon_font(20.0, true)).color(t.accent));
                ui.label(RichText::new(t!("idioma.titulo")).font(theme::semibold(18.0)).color(t.text));
            });
            ui.add_space(10.0);

            // El de Windows y cada idioma disponible.
            let windows = i18n::windows_language();
            let windows_name = self.list.iter().find(|l| l.code == windows).map(|l| l.name.clone()).unwrap_or_else(|| "English".into());
            if row(ui, t, &tf!("idioma.igual_que_windows", idioma = windows_name), "", chosen.is_none()) {
                out = LangOut::Pick(None);
            }
            for l in &self.list {
                let extra = if l.percent < 100 { tf!("idioma.porcentaje", n = l.percent) } else { String::new() };
                if row(ui, t, &l.name, &extra, chosen == Some(l.code.as_str())) {
                    out = LangOut::Pick(Some(l.code.clone()));
                }
            }

            // Traducir a otro idioma.
            ui.add_space(14.0);
            ui.label(RichText::new(t!("idioma.traducir_titulo")).font(theme::semibold(14.0)).color(t.text));
            ui.add_space(2.0);
            for (k, step) in [t!("idioma.paso_1"), t!("idioma.paso_2"), t!("idioma.paso_3")].iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{}.", k + 1)).size(12.5).color(t.accent));
                    ui.label(RichText::new(*step).size(12.5).color(t.dim));
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::secondary_button(ui, t, ic::FOLDER_OPEN, t!("idioma.abrir_carpeta")).clicked() {
                    match i18n::write_template() {
                        Ok(dir) => out = LangOut::ShowFolder(dir.to_string_lossy().into_owned()),
                        Err(e) => self.report = Some(e),
                    }
                }
                if widgets::secondary_button(ui, t, ic::LIST_CHECKS, t!("idioma.revisar")).clicked() {
                    self.list = i18n::available();
                    self.report = Some(i18n::check_all());
                }
            });
            if let Some(r) = &self.report {
                ui.add_space(8.0);
                egui::Frame::new().fill(t.bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(r.as_str()).font(FontId::monospace(11.5)).color(t.dim)).selectable(true));
                    });
                });
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, t, "", t!("idioma.listo"), true).clicked() {
                    out = LangOut::Close;
                }
            });
        });
        if modal.should_close() && matches!(out, LangOut::Open) {
            out = LangOut::Close;
        }
        out
    }
}

/// Una fila elegible (como un botón de radio). Devuelve si se hizo clic.
fn row(ui: &mut egui::Ui, t: &Theme, name: &str, extra: &str, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 8.0, t.accent.gamma_multiply(0.15));
        p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, t.accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect, 8.0, t.hover);
    }
    let (icon, color) = if on { (egui_phosphor::fill::RADIO_BUTTON, t.accent) } else { (ic::CIRCLE, t.faint) };
    p.text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon_font(16.0, on), color);
    p.text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.5), t.text);
    if !extra.is_empty() {
        p.text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, extra, FontId::proportional(12.0), t.faint);
    }
    resp.clicked()
}
