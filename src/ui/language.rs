//! Elegir el idioma de Rayo y ayudar a traducirlo (plantilla y revisión de traducciones).

use super::design::{self, DialogW, Kind, Row, Sel, Tone, Trailing, h, sp, text};
use super::theme::Theme;
use crate::i18n;
use egui::{Align2, RichText, Sense, vec2};
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
        let modal = design::dialog(ctx, t, "idioma", DialogW::M, |ui| {
            design::dialog_header(ui, t, ic::TRANSLATE, Tone::Accent, t!("idioma.titulo"), None);
            ui.add_space(sp::L);

            // El de Windows y cada idioma disponible, como botones de radio en una tarjeta.
            let windows = i18n::windows_language();
            let windows_name = self.list.iter().find(|l| l.code == windows).map(|l| l.name.clone()).unwrap_or_else(|| "English".into());
            design::card_frame(t).inner_margin(sp::XS).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                if row(ui, t, &tf!("idioma.igual_que_windows", idioma = windows_name), "", chosen.is_none()) {
                    out = LangOut::Pick(None);
                }
                for l in &self.list {
                    let extra = if l.percent < 100 { tf!("idioma.porcentaje", n = l.percent) } else { String::new() };
                    if row(ui, t, &l.name, &extra, chosen == Some(l.code.as_str())) {
                        out = LangOut::Pick(Some(l.code.clone()));
                    }
                }
            });

            // Traducir a otro idioma.
            ui.add_space(sp::L);
            ui.label(RichText::new(t!("idioma.traducir_titulo")).font(text::strong()).color(t.text));
            for (k, step) in [t!("idioma.paso_1"), t!("idioma.paso_2"), t!("idioma.paso_3")].iter().enumerate() {
                ui.add_space(sp::XS);
                step_row(ui, t, k + 1, step);
            }
            ui.add_space(sp::M);
            ui.horizontal(|ui| {
                if design::button(ui, t, Kind::Secondary, ic::FOLDER_OPEN, t!("idioma.abrir_carpeta"), true).clicked() {
                    match i18n::write_template() {
                        Ok(dir) => out = LangOut::ShowFolder(dir.to_string_lossy().into_owned()),
                        Err(e) => self.report = Some(e),
                    }
                }
                if design::button(ui, t, Kind::Secondary, ic::LIST_CHECKS, t!("idioma.revisar"), true).clicked() {
                    self.list = i18n::available();
                    self.report = Some(i18n::check_all());
                }
            });
            if let Some(r) = &self.report {
                ui.add_space(sp::S);
                design::card_frame(t).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(r.as_str()).font(text::mono()).color(t.dim)).selectable(true));
                    });
                });
            }
            // Aplica al instante: se cierra con «Listo».
            design::dialog_footer(
                ui,
                |_| {},
                |ui| {
                    if design::button(ui, t, Kind::Primary, "", t!("idioma.listo"), true).clicked() {
                        out = LangOut::Close;
                    }
                },
            );
        });
        if modal.should_close() && matches!(out, LangOut::Open) {
            out = LangOut::Close;
        }
        out
    }
}

/// Un paso numerado: el número en mono `faint` en su columna de 16; el texto (`body`, `dim`)
/// con sangría colgante, debajo de sí mismo.
fn step_row(ui: &mut egui::Ui, t: &Theme, n: usize, step: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = sp::S;
        let (r, _) = ui.allocate_exact_size(vec2(16.0, text::body().size + sp::XS), Sense::hover());
        ui.painter().text(r.left_center(), Align2::LEFT_CENTER, format!("{n}."), text::mono(), t.faint);
        ui.add(egui::Label::new(RichText::new(step).font(text::body()).color(t.dim)).wrap());
    });
}

/// Una fila elegible (como un botón de radio): fila de opción de diálogo, la elegida con la capa
/// de seleccionado y el círculo en acento. Devuelve si se hizo clic.
fn row(ui: &mut egui::Ui, t: &Theme, name: &str, extra: &str, on: bool) -> bool {
    let spec = Row {
        icon: if on { egui_phosphor::fill::RADIO_BUTTON } else { ic::CIRCLE },
        icon_color: Some(if on { t.accent } else { t.faint }),
        icon_filled: on,
        trailing: if extra.is_empty() { Trailing::None } else { Trailing::Count(extra) },
        sel: if on { Sel::Unfocused } else { Sel::None },
        height: h::NAV,
        ..Row::new("", name)
    };
    let id = ui.id().with(("idioma", name));
    design::row(ui, t, id, spec).clicked()
}
