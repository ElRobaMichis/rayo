//! Herramientas: renombrado en lote (diálogo), buscador de duplicados y
//! comparar / sincronizar carpetas (cada una en su propia pestaña).

use super::actions::Action;
use super::state::Wake;
use super::theme::{self, Theme};
use super::views::{self, elided};
use super::widgets::{self, Tone};
use crate::fsx::{self, compare, dupes, rename, transfer::Transfers};
use crate::index::IndexManager;
use egui::{Align2, FontId, Id, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Lo que necesita una herramienta para dibujarse y actuar.
pub struct ToolCx<'a> {
    pub t: &'a Theme,
    pub actions: &'a mut Vec<Action>,
    pub xfer: &'a Transfers,
    pub mgr: &'a Arc<IndexManager>,
    /// Ruta del panel de enfrente (con la vista dividida).
    pub other: Option<String>,
}

fn wake_of(ctx: &egui::Context) -> Wake {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

fn header(ui: &mut Ui, t: &Theme, icon: &str, title: &str) {
    ui.label(RichText::new(icon).font(theme::icon_font(20.0, true)).color(t.accent));
    ui.label(RichText::new(title).font(theme::semibold(18.0)).color(t.text));
}

fn check_icon(ui: &Ui, rect: egui::Rect, t: &Theme, on: bool) {
    let (icon, color) = if on { (egui_phosphor::fill::CHECK_SQUARE, t.accent) } else { (ic::SQUARE, t.faint) };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon_font(17.0, on), color);
}

// ================================================================ renombrado en lote

pub enum Outcome {
    Open,
    Cancel,
    /// Se aplicó; trae lo renombrado (viejo, nuevo) y los errores.
    Applied(Vec<(String, String)>, Vec<String>),
}

pub struct BatchRename {
    pub dir: String,
    files: Vec<rename::Source>,
    existing: HashSet<String>,
    rules: rename::Rules,
    plan: Result<Vec<rename::Planned>, String>,
    planned_for: Option<(rename::Rules, usize)>,
    exif: Option<Arc<Mutex<Vec<Option<String>>>>>,
    exif_done: Arc<AtomicUsize>,
    exif_seen: usize,
    first_frame: bool,
}

impl BatchRename {
    /// `files`: (nombre, fecha de modificación) de lo que se renombra; `existing`: todos los nombres de la carpeta.
    pub fn new(dir: String, files: Vec<(String, u32)>, existing: HashSet<String>) -> Self {
        let files = files.into_iter().map(|(name, mtime)| rename::Source { name, mtime, exif: None }).collect();
        BatchRename {
            dir,
            files,
            existing,
            rules: Default::default(),
            plan: Ok(Vec::new()),
            planned_for: None,
            exif: None,
            exif_done: Default::default(),
            exif_seen: 0,
            first_frame: true,
        }
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// Lee la fecha de las fotos en otro hilo (solo si la plantilla la usa).
    fn start_exif(&mut self, wake: Wake) {
        let out = Arc::new(Mutex::new(vec![None; self.files.len()]));
        self.exif = Some(out.clone());
        let paths: Vec<(usize, String)> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| fsx::thumbs::is_image_ext(&fsx::ext_of(&f.name)))
            .map(|(k, f)| (k, fsx::join(&self.dir, &f.name)))
            .collect();
        let done = self.exif_done.clone();
        std::thread::spawn(move || {
            fsx::shell::com_init();
            for (k, p) in paths {
                let d = fsx::thumbs::exif_date(&p);
                out.lock()[k] = d;
                if done.fetch_add(1, Ordering::Relaxed) % 16 == 0 {
                    wake();
                }
            }
            done.fetch_add(1, Ordering::Relaxed);
            wake();
        });
    }

    pub fn ui(&mut self, ctx: &egui::Context, t: &Theme) -> Outcome {
        let mut out = Outcome::Open;
        if rename::uses(&self.rules.template, "exif") && self.exif.is_none() {
            self.start_exif(wake_of(ctx));
        }
        if let Some(ex) = &self.exif {
            let n = self.exif_done.load(Ordering::Relaxed);
            if n != self.exif_seen {
                self.exif_seen = n;
                let ex = ex.lock();
                for (f, d) in self.files.iter_mut().zip(ex.iter()) {
                    f.exif = d.clone();
                }
            }
        }
        let key = (self.rules.clone(), self.exif_seen);
        if self.planned_for.as_ref() != Some(&key) {
            let dir_name = fsx::file_name(&self.dir);
            self.plan = rename::plan(&dir_name, &self.files, &self.rules, &self.existing);
            self.planned_for = Some(key);
        }

        let modal = egui::Modal::new(Id::new("batch-rename")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.horizontal(|ui| {
                header(ui, t, ic::TEXT_AA, &tp!(self.files.len(), "herr.renombrar_elementos"));
                ui.label(RichText::new(fsx::file_name(&self.dir)).color(t.dim));
            });
            ui.add_space(12.0);
            let plan_err = self.plan.is_err();
            let r = &mut self.rules;

            // Nombre nuevo: plantilla y piezas para insertar donde está el cursor.
            widgets::caption(ui, t, t!("herr.nombre_nuevo"));
            let tid = Id::new("batch-template");
            let hint = rename::token("nombre");
            let mut f = widgets::Field::new("batch-template", ic::PENCIL_SIMPLE, &hint);
            f.tone = if plan_err && !r.regex { Tone::Bad } else { Tone::Normal };
            let te = widgets::field(ui, t, &mut r.template, f);
            if self.first_frame {
                te.edit.request_focus();
                self.first_frame = false;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(t!("herr.insertar")).size(11.5).color(t.faint));
                for (tok, tip) in [
                    (rename::token("nombre"), t!("herr.nombre_original")),
                    (rename::token("n"), t!("herr.numero_desde_digitos_abajo")),
                    (rename::token("fecha"), t!("herr.fecha_modificacion")),
                    (rename::token("exif"), t!("herr.fecha_tomo_foto")),
                    (rename::token("carpeta"), t!("herr.nombre_carpeta")),
                ] {
                    if widgets::chip(ui, t, &tok, true).on_hover_text(tip).clicked() {
                        widgets::insert_at_cursor(ui.ctx(), tid, &mut r.template, &tok);
                    }
                }
            });
            ui.add_space(12.0);

            // Buscar y reemplazar.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let regex_w = 150.0;
                let half = ((ui.available_width() - regex_w - 2.0 * 8.0) / 2.0).max(150.0);
                ui.vertical(|ui| {
                    ui.set_width(half);
                    widgets::caption(ui, t, t!("herr.buscar"));
                    let mut f = widgets::Field::new("batch-find", ic::MAGNIFYING_GLASS, if r.regex { t!("herr.expresion_regular_p_ej_d") } else { t!("herr.texto_cambiar") });
                    f.mono = r.regex;
                    f.tone = if plan_err && r.regex { Tone::Bad } else { Tone::Normal };
                    widgets::field(ui, t, &mut r.find, f);
                });
                ui.vertical(|ui| {
                    ui.set_width(half);
                    widgets::caption(ui, t, t!("herr.reemplazar"));
                    let mut f = widgets::Field::new("batch-replace", ic::ARROW_BEND_DOWN_RIGHT, if r.regex { t!("herr.usa_1_2_capturado") } else { t!("herr.vacio_borrar_texto") });
                    f.mono = r.regex;
                    widgets::field(ui, t, &mut r.replace, f);
                });
                ui.vertical(|ui| {
                    ui.add_space(ui.text_style_height(&egui::TextStyle::Small) + 4.0);
                    widgets::toggle_chip(ui, t, &mut r.regex, ic::BRACKETS_CURLY, t!("herr.expresion_regular")).on_hover_text(t!("herr.buscar_expresion_regular_reemplazo_1"));
                });
            });
            ui.add_space(12.0);

            // Formato, numeración y extensión.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                use rename::Case;
                ui.vertical(|ui| {
                    widgets::caption(ui, t, t!("herr.mayusculas_minusculas"));
                    widgets::segmented(
                        ui,
                        t,
                        &mut r.case,
                        &[
                            (Case::Keep, t!("herr.sin_cambios"), t!("herr.deja_nombre_como_esta")),
                            (Case::Lower, t!("herr.minusculas"), t!("herr.todo_minusculas")),
                            (Case::Upper, t!("herr.mayusculas"), t!("herr.todo_mayusculas")),
                            (Case::Title, t!("herr.tipo_titulo"), t!("herr.cada_palabra_mayuscula")),
                        ],
                    );
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    widgets::caption(ui, t, t!("herr.desde"));
                    widgets::stepper(ui, t, &mut r.start, 0, 999_999, 96.0);
                });
                ui.vertical(|ui| {
                    widgets::caption(ui, t, t!("herr.digitos"));
                    let mut pad = r.pad as u32;
                    if widgets::stepper(ui, t, &mut pad, 1, 8, 84.0) {
                        r.pad = pad as usize;
                    }
                });
                ui.vertical(|ui| {
                    widgets::caption(ui, t, t!("herr.extension"));
                    let mut f = widgets::Field::new("batch-ext", ic::DOT_OUTLINE, t!("herr.misma"));
                    f.width = 120.0;
                    f.no_clear = true;
                    widgets::field(ui, t, &mut r.new_ext, f);
                });
            });
            ui.add_space(14.0);

            // Vista previa.
            let (changes, problems, first_problem) = match &self.plan {
                Ok(p) => (p.iter().filter(|x| x.old != x.new).count(), p.iter().filter(|x| x.problem.is_some()).count(), p.iter().find_map(|x| x.problem.clone())),
                Err(_) => (0, 0, None),
            };
            egui::Frame::new().fill(t.bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                // Encabezado: antes → después.
                let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                let hf = FontId::proportional(11.5);
                ui.painter().text(pos2(hr.left() + 6.0, hr.center().y), Align2::LEFT_CENTER, t!("herr.ahora"), hf.clone(), t.faint);
                ui.painter().text(pos2(hr.center().x + 18.0, hr.center().y), Align2::LEFT_CENTER, t!("herr.quedara"), hf, t.faint);
                match &self.plan {
                    Err(e) => {
                        ui.set_min_height(120.0);
                        ui.label(RichText::new(format!("{}  {e}", ic::WARNING)).color(t.danger));
                    }
                    Ok(plan) => {
                        let row_h = 24.0;
                        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show_rows(ui, row_h, plan.len(), |ui, range| {
                            for p in &plan[range] {
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                                let painter = ui.painter();
                                if resp.hovered() {
                                    painter.rect_filled(rect, 6.0, t.hover);
                                }
                                let half = rect.width() / 2.0 - 18.0;
                                let font = FontId::proportional(13.0);
                                let g = elided(painter, &p.old, font.clone(), t.dim, half);
                                painter.galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g, t.dim);
                                let changed = p.old != p.new;
                                painter.text(rect.center(), Align2::CENTER_CENTER, ic::ARROW_RIGHT, theme::icon_font(12.0, false), if changed { t.accent } else { t.faint });
                                let color = if p.problem.is_some() {
                                    t.danger
                                } else if changed {
                                    t.text
                                } else {
                                    t.faint
                                };
                                let g = elided(painter, &p.new, font, color, half);
                                painter.galley(pos2(rect.center().x + 18.0, rect.center().y - g.size().y / 2.0), g, color);
                                if let Some(pr) = &p.problem {
                                    resp.on_hover_text(pr);
                                }
                            }
                        });
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if problems > 0 {
                    ui.label(RichText::new(format!("{} {}", ic::WARNING, tf!("herr.n_con_problemas", problemas = problems, primero = first_problem.unwrap_or_default()))).color(t.danger));
                } else {
                    ui.label(RichText::new(tp!(changes, "herr.cambian_nombre")).color(t.dim));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = changes > 0 && problems == 0;
                    if widgets::primary_button(ui, t, ic::CHECK, t!("herr.renombrar"), ok).on_hover_text("Ctrl+Enter").clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl)) {
                        if let Ok(plan) = &self.plan {
                            let (done, errors) = rename::apply(&self.dir, plan);
                            out = Outcome::Applied(done, errors);
                        }
                    }
                    if widgets::secondary_button(ui, t, "", t!("comun.cancelar")).clicked() {
                        out = Outcome::Cancel;
                    }
                });
            });
        });
        if modal.should_close() && matches!(out, Outcome::Open) {
            out = Outcome::Cancel;
        }
        out
    }
}

// ================================================================ hash

pub struct HashDialog {
    files: Vec<String>,
    results: Arc<Mutex<Vec<Option<Result<fsx::hash::Hashes, String>>>>>,
    done: Arc<std::sync::atomic::AtomicU64>,
    total: u64,
    cancel: Arc<AtomicBool>,
    expected: String,
    cloud: bool,
    sizes: Vec<u64>,
    /// Último valor copiado (archivo, algoritmo) y cuándo, para mostrar "Copiado".
    copied: Option<(usize, &'static str, f64)>,
}

impl HashDialog {
    pub fn new(files: Vec<String>, ctx: &egui::Context) -> Self {
        let sizes: Vec<u64> = files.iter().map(|f| std::fs::metadata(fsx::long_path(f)).map(|m| m.len()).unwrap_or(0)).collect();
        let total = sizes.iter().sum();
        let cloud = files.iter().any(|f| fsx::is_online_only(f));
        let results: Arc<Mutex<Vec<Option<Result<fsx::hash::Hashes, String>>>>> = Arc::new(Mutex::new(vec![None; files.len()]));
        let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (r, d, c, list, wake) = (results.clone(), done.clone(), cancel.clone(), files.clone(), wake_of(ctx));
        std::thread::spawn(move || {
            for (k, f) in list.iter().enumerate() {
                let res = fsx::hash::file(f, &d, &c);
                r.lock()[k] = Some(res);
                wake();
            }
        });
        HashDialog { files, results, done, total, cancel, expected: String::new(), cloud, sizes, copied: None }
    }

    /// Devuelve false al cerrarse.
    pub fn ui(&mut self, ctx: &egui::Context, t: &Theme) -> bool {
        let mut open = true;
        let results = self.results.lock().clone();
        let finished = results.iter().all(|r| r.is_some());
        let now = ctx.input(|i| i.time);
        let expected = self.expected.trim().to_string();
        // ¿Coincide con alguno? (sí / no / todavía no se sabe)
        let verdict = if expected.is_empty() {
            None
        } else if results.iter().flatten().filter_map(|r| r.as_ref().ok()).any(|h| h.matches(&expected).is_some()) {
            Some(true)
        } else if finished {
            Some(false)
        } else {
            None
        };
        let modal = egui::Modal::new(Id::new("hash")).show(ctx, |ui| {
            ui.set_width(680.0);
            ui.horizontal(|ui| header(ui, t, ic::FINGERPRINT, t!("herr.hash")));
            if self.cloud {
                ui.label(RichText::new(format!("{}  {}", ic::CLOUD, t!("herr.algun_archivo_esta_solo_nube"))).color(t.dim));
            }
            ui.add_space(12.0);

            // Comparar con el hash que te dieron.
            widgets::caption(ui, t, t!("herr.comparar_hash_esperado"));
            let mut f = widgets::Field::new("hash-esperado", ic::SCALES, t!("herr.pega_hash_te_dieron_sha"));
            f.mono = true;
            f.tone = match verdict {
                Some(true) => Tone::Good,
                Some(false) => Tone::Bad,
                None => Tone::Normal,
            };
            widgets::field(ui, t, &mut self.expected, f);
            ui.add_space(2.0);
            let (msg, color) = match verdict {
                Some(true) => (format!("{}  {}", egui_phosphor::fill::CHECK_CIRCLE, t!("herr.coincide_archivo_correcto")), widgets::GOOD),
                Some(false) => (format!("{}  {}", egui_phosphor::fill::X_CIRCLE, t!("herr.no_coincide_archivo_puede_estar")), t.danger),
                None if !expected.is_empty() => (format!("{}  {}", ic::HOURGLASS, t!("herr.esperando_termine_calculo")), t.dim),
                None => (t!("herr.sirve_comprobar_descarga_si_coincide").to_string(), t.faint),
            };
            ui.label(RichText::new(msg).size(12.0).color(color));
            ui.add_space(10.0);

            // Progreso.
            if !finished {
                let done = self.done.load(Ordering::Relaxed);
                let frac = if self.total > 0 { (done as f32 / self.total as f32).clamp(0.0, 1.0) } else { 0.0 };
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, t.hover);
                ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(r.width() * frac, r.height())), 3.0, t.accent);
                ui.label(RichText::new(tf!("herr.leyendo", hecho = fsx::human_size(done), total = fsx::human_size(self.total))).size(12.0).color(t.dim));
                ui.add_space(6.0);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }

            // Un recuadro por archivo.
            let mut copy: Option<(usize, &'static str, String)> = None;
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (k, path) in self.files.iter().enumerate() {
                    egui::Frame::new().fill(t.bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(egui_phosphor::fill::FILE).font(theme::icon_font(15.0, true)).color(t.faint));
                            ui.label(RichText::new(fsx::file_name(path)).font(theme::semibold(13.5)).color(t.text));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(fsx::human_size(self.sizes[k])).size(12.0).color(t.faint));
                            });
                        });
                        ui.add_space(6.0);
                        match &results[k] {
                            None => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new(t!("herr.calculando")).size(12.5).color(t.faint));
                                });
                            }
                            Some(Err(e)) => {
                                ui.label(RichText::new(format!("{}  {e}", ic::WARNING)).color(t.danger));
                            }
                            Some(Ok(h)) => {
                                let hit = if expected.is_empty() { None } else { h.matches(&expected) };
                                for (alg, v) in [("SHA-256", &h.sha256), ("SHA-1", &h.sha1), ("MD5", &h.md5)] {
                                    let just_copied = self.copied.is_some_and(|(f, a, at)| f == k && a == alg && now - at < 1.5);
                                    if hash_row(ui, t, alg, v, hit == Some(alg), just_copied) {
                                        copy = Some((k, alg, v.clone()));
                                    }
                                }
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
            if let Some((k, alg, v)) = copy {
                ui.ctx().copy_text(v);
                self.copied = Some((k, alg, now));
            }
            if self.copied.is_some_and(|(_, _, at)| now - at < 1.5) {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::secondary_button(ui, t, "", if finished { t!("comun.cerrar") } else { t!("comun.cancelar") }).clicked() {
                    open = false;
                }
            });
        });
        if modal.should_close() {
            open = false;
        }
        if !open {
            self.cancel.store(true, Ordering::Relaxed);
        }
        open
    }
}

/// Una fila de hash: algoritmo en una pastilla, el valor (seleccionable) y copiar.
/// `hit`: es el que coincide con el esperado. Devuelve si se pidió copiar.
fn hash_row(ui: &mut Ui, t: &Theme, alg: &str, value: &str, hit: bool, copied: bool) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
    let p = ui.painter().clone();
    if hit {
        p.rect_filled(rect, 7.0, widgets::GOOD.gamma_multiply(0.12));
    } else if ui.rect_contains_pointer(rect) {
        p.rect_filled(rect, 7.0, t.hover);
    }
    // Pastilla con el algoritmo.
    let chip = egui::Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 10.0), vec2(62.0, 20.0));
    p.rect_filled(chip, 5.0, if hit { widgets::GOOD.gamma_multiply(0.25) } else { t.hover });
    p.text(chip.center(), Align2::CENTER_CENTER, alg, theme::semibold(11.0), if hit { widgets::GOOD } else { t.dim });
    // Copiar (a la derecha).
    let cr = egui::Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(26.0, 24.0));
    let cresp = ui.interact(cr, Id::new(("hash-copy", alg, value)), Sense::click());
    if cresp.hovered() {
        p.rect_filled(cr, 6.0, t.border);
    }
    let (icon, color) = if copied { (ic::CHECK, widgets::GOOD) } else { (ic::COPY, if cresp.hovered() { t.text } else { t.dim }) };
    p.text(cr.center(), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, false), color);
    if copied {
        p.text(pos2(cr.left() - 6.0, rect.center().y), Align2::RIGHT_CENTER, t!("comun.copiado"), FontId::proportional(11.5), widgets::GOOD);
    }
    // El valor, seleccionable con el mouse.
    let vr = egui::Rect::from_min_max(pos2(chip.right() + 12.0, rect.top()), pos2(cr.left() - if copied { 60.0 } else { 6.0 }, rect.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add(egui::Label::new(RichText::new(value).font(FontId::monospace(12.0)).color(if hit { widgets::GOOD } else { t.text })).selectable(true).truncate());
    if hit {
        child.label(RichText::new(egui_phosphor::fill::CHECK_CIRCLE).font(theme::icon_font(13.0, true)).color(widgets::GOOD));
    }
    cresp.on_hover_text(t!("herr.copiar")).clicked()
}

// ================================================================ quién usa un archivo

/// "¿Quién lo está usando?": qué programas tienen abierto lo elegido, con botones
/// para pedirles que se cierren y, cuando ya nadie lo usa, reintentar lo que falló.
pub struct LocksDialog {
    paths: Vec<String>,
    result: Arc<Mutex<Option<Result<Vec<fsx::locks::Locker>, String>>>>,
    /// Lo que no se pudo hacer (borrar, renombrar…) y cómo se llama, para reintentarlo.
    retry: Option<(String, Action)>,
    /// Por qué se abrió solo ("No se pudo borrar").
    reason: Option<String>,
    /// Para volver a mirar un momento después de cerrar algo.
    recheck_at: Option<f64>,
    confirm_kill: Option<u32>,
    error: Option<String>,
    wake: Wake,
    /// La app debe soltar lo que Rayo tenga abierto (vista previa).
    pub release_self: bool,
}

impl LocksDialog {
    pub fn new(paths: Vec<String>, reason: Option<String>, retry: Option<(String, Action)>, ctx: &egui::Context) -> Self {
        let mut d = LocksDialog { paths, result: Default::default(), retry, reason, recheck_at: None, confirm_kill: None, error: None, wake: wake_of(ctx), release_self: false };
        d.scan();
        d
    }

    fn scan(&mut self) {
        *self.result.lock() = None;
        let (r, paths, wake) = (self.result.clone(), self.paths.clone(), self.wake.clone());
        std::thread::spawn(move || {
            let res = fsx::locks::who_uses(&paths);
            *r.lock() = Some(res);
            wake();
        });
    }

    /// Devuelve false al cerrarse. Lo que haya que reintentar va a `actions`.
    pub fn ui(&mut self, ctx: &egui::Context, t: &Theme, actions: &mut Vec<Action>) -> bool {
        let mut open = true;
        let now = ctx.input(|i| i.time);
        if self.recheck_at.is_some_and(|at| now >= at) {
            self.recheck_at = None;
            self.scan();
        } else if let Some(at) = self.recheck_at {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.05)));
        }
        let result = self.result.lock().clone();
        let modal = egui::Modal::new(Id::new("locks")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| header(ui, t, ic::LOCK_SIMPLE, t!("herr.quien_esta_usando")));
            let what = if self.paths.len() == 1 { format!("«{}»", fsx::file_name(&self.paths[0])) } else { tf!("comun.elementos", n = self.paths.len()) };
            if let Some(r) = &self.reason {
                ui.label(RichText::new(tf!("herr.esta_abierto_otro_programa", motivo = r, que = what)).color(t.dim));
            } else {
                ui.label(RichText::new(what).color(t.dim));
            }
            ui.add_space(10.0);
            match &result {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(t!("herr.preguntando_windows")).color(t.dim));
                    });
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(t.danger));
                }
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(RichText::new(format!("{}  {}", egui_phosphor::fill::CHECK_CIRCLE, t!("herr.ningun_programa_tiene_abierto_ahora"))).color(egui::Color32::from_rgb(0x3F, 0xB9, 0x50)));
                    if self.retry.is_none() {
                        ui.label(RichText::new(t!("herr.si_igual_no_deja_borrar")).size(12.0).color(t.faint));
                    }
                }
                Some(Ok(list)) => {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for l in list {
                            ui.horizontal(|ui| {
                                let icon = match l.kind {
                                    fsx::locks::Kind::Service => ic::GEAR,
                                    fsx::locks::Kind::Console => ic::TERMINAL_WINDOW,
                                    fsx::locks::Kind::Explorer => ic::FOLDER_OPEN,
                                    fsx::locks::Kind::Critical => ic::SHIELD,
                                    _ => ic::APP_WINDOW,
                                };
                                ui.label(RichText::new(icon).font(theme::icon_font(18.0, false)).color(t.dim));
                                ui.vertical(|ui| {
                                    let name = if l.is_self { t!("herr.rayo_vista_previa_miniatura").to_string() } else { l.name.clone() };
                                    ui.label(RichText::new(name).font(theme::semibold(13.5)).color(t.text));
                                    let mut sub = if l.exe.is_empty() { tf!("herr.proceso", pid = l.pid) } else { tf!("herr.proceso_programa", programa = l.exe, pid = l.pid) };
                                    match l.kind {
                                        fsx::locks::Kind::Service => sub.push_str(t!("herr.servicio_windows")),
                                        fsx::locks::Kind::Critical => sub.push_str(t!("herr.parte_windows")),
                                        fsx::locks::Kind::Explorer => sub.push_str(t!("herr.explorador")),
                                        _ => {}
                                    }
                                    ui.label(RichText::new(sub).size(11.5).color(t.faint));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if l.is_self {
                                        if ui.button(t!("herr.soltarlo")).on_hover_text(t!("herr.cierra_vista_previa_liberar_archivo")).clicked() {
                                            self.release_self = true;
                                            self.recheck_at = Some(now + 0.6);
                                        }
                                    } else if self.confirm_kill == Some(l.pid) {
                                        if ui.button(RichText::new(t!("herr.si_forzar")).color(t.danger)).clicked() {
                                            self.confirm_kill = None;
                                            match l.kill() {
                                                Ok(()) => self.recheck_at = Some(now + 0.5),
                                                Err(e) => self.error = Some(tf!("herr.no_pudo_cerrar", nombre = l.name, error = e)),
                                            }
                                        }
                                        if ui.button(t!("herr.no")).clicked() {
                                            self.confirm_kill = None;
                                        }
                                        ui.label(RichText::new(t!("herr.pierde_no_este_guardado")).size(11.5).color(t.danger));
                                    } else {
                                        if l.can_kill() && ui.button(t!("herr.forzar_cierre")).on_hover_text(t!("herr.termina_programa_instante_pierde_no")).clicked() {
                                            self.confirm_kill = Some(l.pid);
                                        }
                                        if l.can_close() && ui.button(t!("comun.cerrar")).on_hover_text(t!("herr.como_tocar_su_x_programa")).clicked() {
                                            l.close();
                                            self.recheck_at = Some(now + 1.5);
                                        }
                                    }
                                });
                            });
                            ui.add_space(6.0);
                        }
                    });
                }
            }
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(t.danger));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let free = matches!(&result, Some(Ok(l)) if l.is_empty());
                    if let Some((label, a)) = &self.retry {
                        let b = egui::Button::new(RichText::new(label.as_str()).color(if free { t.text } else { t.dim }));
                        if ui.add(b).clicked() {
                            actions.push(a.clone());
                            open = false;
                        }
                    }
                    if ui.button(t!("herr.volver_mirar")).clicked() {
                        self.error = None;
                        self.scan();
                    }
                    if ui.button(t!("comun.cerrar")).clicked() {
                        open = false;
                    }
                });
            });
        });
        if modal.should_close() {
            open = false;
        }
        open
    }
}

// ================================================================ herramientas en pestaña

pub enum Tool {
    Dupes(DupesTool),
    Compare(CompareTool),
    Recycle(RecycleTool),
}

impl Tool {
    pub fn title(&self) -> String {
        match self {
            Tool::Dupes(d) => tf!("herr.titulo_duplicados", carpeta = short_name(&d.scope)),
            Tool::Compare(_) => t!("herr.comparar_carpetas").into(),
            Tool::Recycle(_) => t!("comun.papelera").into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tool::Dupes(_) => ic::COPY_SIMPLE,
            Tool::Compare(_) => ic::ARROWS_LEFT_RIGHT,
            Tool::Recycle(_) => ic::TRASH_SIMPLE,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, cx: &mut ToolCx) {
        egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 8 }).show(ui, |ui| match self {
            Tool::Dupes(d) => d.ui(ui, cx),
            Tool::Compare(c) => c.ui(ui, cx),
            Tool::Recycle(r) => r.ui(ui, cx),
        });
    }
}

fn short_name(p: &str) -> String {
    if p.len() <= 3 { p.trim_end_matches('\\').to_string() } else { fsx::file_name(p) }
}

// ---------------------------------------------------------------- duplicados

#[derive(Default)]
struct DupesJob {
    cancel: AtomicBool,
    prog: dupes::Progress,
    /// 0 = juntando candidatos, 1 = comparando contenido, 2 = listo.
    phase: AtomicUsize,
    candidates: AtomicUsize,
    from_index: AtomicBool,
    result: Mutex<Option<Vec<dupes::Group>>>,
}

pub struct DupesTool {
    pub scope: String,
    min_size: u64,
    job: Arc<DupesJob>,
    groups: Vec<dupes::Group>,
    marked: HashSet<String>,
    hint: Option<String>,
    started: std::time::Instant,
    took: Option<f32>,
}

const MIN_SIZES: &[(u64, &str)] = &[(1, ""), (100 << 10, "100 KB"), (1 << 20, "1 MB"), (10 << 20, "10 MB"), (100 << 20, "100 MB")];

/// El nombre de un tamaño mínimo ("" es sin mínimo).
fn min_size_label(l: &'static str) -> &'static str {
    if l.is_empty() { t!("herr.todos_tamanos") } else { l }
}

impl DupesTool {
    pub fn new(scope: String, min_size: u64, mgr: &Arc<IndexManager>, ctx: &egui::Context) -> Self {
        let mut d = DupesTool {
            scope,
            min_size,
            job: Default::default(),
            groups: Vec::new(),
            marked: HashSet::new(),
            hint: None,
            started: std::time::Instant::now(),
            took: None,
        };
        d.start(mgr.clone(), wake_of(ctx));
        d
    }

    fn start(&mut self, mgr: Arc<IndexManager>, wake: Wake) {
        self.job.cancel.store(true, Ordering::Relaxed);
        let job = Arc::new(DupesJob::default());
        self.job = job.clone();
        self.groups.clear();
        self.marked.clear();
        self.took = None;
        self.started = std::time::Instant::now();
        let (scope, min) = (self.scope.clone(), self.min_size);
        std::thread::Builder::new()
            .name("duplicados".into())
            .spawn(move || {
                // Del índice al instante; si el disco no está indexado, recorriendo carpetas.
                let cands = match mgr.dupe_candidates(&scope, min) {
                    Some(c) => {
                        job.from_index.store(true, Ordering::Relaxed);
                        c
                    }
                    None => dupes::walk_candidates(&scope, min, &job.cancel),
                };
                job.candidates.store(cands.len(), Ordering::Relaxed);
                job.phase.store(1, Ordering::Relaxed);
                wake();
                let groups = dupes::find(cands, &job.cancel, &job.prog);
                *job.result.lock() = Some(groups);
                job.phase.store(2, Ordering::Relaxed);
                wake();
            })
            .ok();
    }

    /// Marca o desmarca, sin dejar nunca un grupo sin ninguna copia.
    fn toggle(&mut self, gi: usize, path: &str) {
        if self.marked.remove(path) {
            return;
        }
        let g = &self.groups[gi];
        let unmarked = g.files.iter().filter(|f| !self.marked.contains(&f.path)).count();
        if unmarked <= 1 {
            self.hint = Some(t!("herr.siempre_queda_menos_copia_cada").into());
        } else {
            self.marked.insert(path.to_string());
        }
    }

    /// Marca todas las copias menos una (la más antigua o la más nueva).
    fn mark_all(&mut self, keep_oldest: bool) {
        self.marked.clear();
        for g in &self.groups {
            // Los archivos de cada grupo están ordenados por fecha.
            let keep = if keep_oldest { 0 } else { g.files.len() - 1 };
            for (k, f) in g.files.iter().enumerate() {
                if k != keep {
                    self.marked.insert(f.path.clone());
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut ToolCx) {
        let t = cx.t;
        let phase = self.job.phase.load(Ordering::Relaxed);
        if phase == 2 && self.took.is_none() {
            self.groups = self.job.result.lock().take().unwrap_or_default();
            self.took = Some(self.started.elapsed().as_secs_f32());
        }
        ui.horizontal(|ui| {
            header(ui, t, ic::COPY_SIMPLE, t!("herr.archivos_duplicados"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(format!("{}  {}", ic::ARROW_CLOCKWISE, t!("herr.buscar_otra_vez"))).clicked() {
                    self.start(cx.mgr.clone(), wake_of(ui.ctx()));
                }
                let label = MIN_SIZES.iter().find(|m| m.0 == self.min_size).map_or("—", |m| min_size_label(m.1));
                let mut min = self.min_size;
                egui::ComboBox::from_id_salt("dupes-min").selected_text(label).width(80.0).show_ui(ui, |ui| {
                    for (v, l) in MIN_SIZES {
                        ui.selectable_value(&mut min, *v, min_size_label(l));
                    }
                });
                ui.label(RichText::new(t!("herr.ignorar_menores")).color(t.dim));
                if min != self.min_size {
                    self.min_size = min;
                    self.start(cx.mgr.clone(), wake_of(ui.ctx()));
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&self.scope).color(t.dim)).truncate());
        ui.add_space(10.0);

        if phase < 2 {
            let (done, total) = (self.job.prog.done.load(Ordering::Relaxed), self.job.prog.total.load(Ordering::Relaxed));
            let what = if phase == 0 {
                t!("herr.buscando_archivos_mismo_tamano").to_string()
            } else {
                tf!("herr.comparando_contenido_candidatos", n = fsx::thousands(self.job.candidates.load(Ordering::Relaxed)))
            };
            ui.label(RichText::new(what).color(t.text));
            ui.add_space(6.0);
            let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(f).desired_width(420.0).text(tf!("herr.progreso_hecho_total", hecho = fsx::human_size(done), total = fsx::human_size(total))));
                if ui.button(t!("comun.cancelar")).clicked() {
                    self.job.cancel.store(true, Ordering::Relaxed);
                }
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            return;
        }
        if self.groups.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(egui_phosphor::fill::CHECK_CIRCLE).font(theme::icon_font(40.0, true)).color(t.accent));
                let msg = if self.job.cancel.load(Ordering::Relaxed) { t!("herr.busqueda_cancelada") } else { t!("herr.no_hay_archivos_duplicados") };
                ui.label(RichText::new(msg).font(theme::semibold(16.0)).color(t.text));
            });
            return;
        }

        let files: usize = self.groups.iter().map(|g| g.files.len() - 1).sum();
        let wasted: u64 = self.groups.iter().map(|g| g.wasted()).sum();
        let marked_size: u64 = self.groups.iter().flat_map(|g| g.files.iter().filter(|f| self.marked.contains(&f.path)).map(move |_| g.size)).sum();
        let src = if self.job.from_index.load(Ordering::Relaxed) { t!("herr.indice") } else { t!("herr.recorriendo_carpetas") };
        let cloud = self.job.prog.skipped_cloud.load(Ordering::Relaxed);
        if cloud > 0 {
            ui.label(RichText::new(format!("{} {}", ic::CLOUD, tf!("herr.archivos_solo_nube_no_revisados", nube = cloud))).color(t.dim));
        }
        ui.label(
            RichText::new(tf!("herr.grupos_copias_mas_pueden_liberar", grupos = fsx::thousands(self.groups.len()), copias = fsx::thousands(files), tamano = fsx::human_size(wasted), segundos = format!("{:.1}", self.took.unwrap_or(0.0)), como = src))
            .color(t.dim),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(t!("herr.marcar_copias_conservar_mas_antigua")).clicked() {
                self.mark_all(true);
            }
            if ui.button(t!("herr.conservar_mas_nueva")).clicked() {
                self.mark_all(false);
            }
            if ui.add_enabled(!self.marked.is_empty(), egui::Button::new(t!("herr.desmarcar_todo"))).clicked() {
                self.marked.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.marked.len();
                let label = format!("{}  {}", ic::TRASH, tf!("herr.mover_papelera", n = n, tamano = fsx::human_size(marked_size)));
                let ok = n > 0;
                if ui.add_enabled(ok, egui::Button::new(RichText::new(label).color(if ok { t.accent_text } else { t.faint })).fill(if ok { t.danger } else { t.hover })).clicked() {
                    let paths: Vec<String> = self.marked.drain().collect();
                    let gone: HashSet<&String> = paths.iter().collect();
                    for g in &mut self.groups {
                        g.files.retain(|f| !gone.contains(&f.path));
                    }
                    self.groups.retain(|g| g.files.len() > 1);
                    cx.actions.push(Action::Delete(paths, false));
                }
            });
        });
        if let Some(h) = &self.hint {
            ui.label(RichText::new(h).color(t.danger));
        }
        ui.add_space(6.0);

        // Lista: un encabezado por grupo y una fila por archivo.
        let mut rows: Vec<(usize, Option<usize>)> = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            rows.push((gi, None));
            rows.extend((0..g.files.len()).map(|fi| (gi, Some(fi))));
        }
        let row_h = 26.0;
        let mut toggle: Option<(usize, String)> = None;
        egui::ScrollArea::vertical().id_salt("dupes-list").auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for &(gi, fi) in &rows[range] {
                let g = &self.groups[gi];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let painter = ui.painter();
                match fi {
                    None => {
                        painter.line_segment([rect.left_top() + vec2(0.0, 2.0), rect.right_top() + vec2(0.0, 2.0)], egui::Stroke::new(1.0, t.border));
                        painter.text(
                            pos2(rect.left() + 4.0, rect.center().y + 2.0),
                            Align2::LEFT_CENTER,
                            tf!("herr.copias_cada", n = g.files.len(), tamano = fsx::human_size(g.size)),
                            theme::semibold(12.5),
                            t.dim,
                        );
                    }
                    Some(fi) => {
                        let f = &g.files[fi];
                        let on = self.marked.contains(&f.path);
                        if resp.hovered() {
                            painter.rect_filled(rect, 6.0, t.hover);
                        }
                        let cb = egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(20.0, 20.0));
                        check_icon(ui, cb, t, on);
                        let name = fsx::file_name(&f.path);
                        let date_w = 140.0;
                        let x = rect.left() + 36.0;
                        let g1 = elided(painter, &name, FontId::proportional(13.0), if on { t.faint } else { t.text }, (rect.width() - date_w - 40.0) * 0.45);
                        let w1 = g1.size().x;
                        painter.galley(pos2(x, rect.center().y - g1.size().y / 2.0), g1, t.text);
                        let parent = fsx::parent_of(&f.path).unwrap_or_default();
                        let g2 = elided(painter, &parent, FontId::proportional(12.0), t.faint, rect.width() - date_w - 56.0 - w1);
                        painter.galley(pos2(x + w1 + 12.0, rect.center().y - g2.size().y / 2.0), g2, t.faint);
                        painter.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, fsx::fmt_date(f.mtime), FontId::proportional(12.0), t.dim);
                        if resp.clicked() {
                            toggle = Some((gi, f.path.clone()));
                        }
                        if resp.double_clicked() {
                            toggle = None;
                            cx.actions.push(Action::NewTab(Some(parent.clone())));
                            cx.actions.push(Action::Reveal(f.path.clone()));
                        }
                        resp.context_menu(|ui| {
                            if ui.button(format!("{}  {}", ic::FOLDER_OPEN, t!("comun.mostrar_su_carpeta"))).clicked() {
                                cx.actions.push(Action::NewTab(Some(parent.clone())));
                                cx.actions.push(Action::Reveal(f.path.clone()));
                                ui.close();
                            }
                            if ui.button(format!("{}  {}", ic::ARROW_SQUARE_OUT, t!("comun.abrir"))).clicked() {
                                cx.actions.push(Action::Open { path: f.path.clone(), is_dir: false });
                                ui.close();
                            }
                        });
                    }
                }
            }
        });
        if let Some((gi, p)) = toggle {
            self.hint = None;
            self.toggle(gi, &p);
        }
    }
}

// ---------------------------------------------------------------- papelera

pub struct RecycleTool {
    bin: fsx::recycle::Bin,
    marked: HashSet<usize>,
    filter: String,
}

impl RecycleTool {
    pub fn new(hwnd: windows::Win32::Foundation::HWND, mgr: Arc<IndexManager>, ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        let bin = fsx::recycle::Bin::open(
            hwnd,
            Arc::new(move |dirs: Vec<String>| {
                // Lo restaurado reaparece en su carpeta.
                mgr.dirty.lock().extend(dirs.iter().map(|d| d.to_lowercase()));
                mgr.wake_ui();
                ctx.request_repaint();
            }),
        );
        RecycleTool { bin, marked: HashSet::new(), filter: String::new() }
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut ToolCx) {
        let t = cx.t;
        let items = self.bin.items.lock().clone();
        let busy = self.bin.busy.load(Ordering::Relaxed);
        ui.horizontal(|ui| {
            header(ui, t, ic::TRASH_SIMPLE, t!("comun.papelera"));
            if busy {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(format!("{}  {}", ic::ARROW_CLOCKWISE, t!("herr.actualizar"))).clicked() {
                    self.marked.clear();
                    self.bin.refresh();
                }
                let mut f = widgets::Field::new("papelera-filtro", ic::FUNNEL, t!("herr.filtrar_nombre_carpeta"));
                f.width = 240.0;
                widgets::field(ui, t, &mut self.filter, f);
            });
        });
        let Some(items) = items else {
            ui.label(RichText::new(t!("herr.leyendo_papelera")).color(t.dim));
            return;
        };
        let key = crate::index::store::search_key(self.filter.trim());
        let shown: Vec<usize> = (0..items.len())
            .filter(|&k| key.is_empty() || crate::index::store::search_key(&items[k].name).contains(&key) || crate::index::store::search_key(&items[k].from).contains(&key))
            .collect();
        let total: u64 = items.iter().map(|i| i.size).sum();
        ui.label(RichText::new(tp!(items.len(), "herr.papelera_resumen", n = fsx::thousands(items.len()), tamano = fsx::human_size(total))).color(t.dim));
        ui.add_space(8.0);
        self.marked.retain(|k| *k < items.len());
        let n = self.marked.len();
        ui.horizontal(|ui| {
            let all = !shown.is_empty() && shown.iter().all(|k| self.marked.contains(k));
            if ui.button(if all { t!("herr.desmarcar_todo") } else { t!("herr.marcar_todo") }).clicked() {
                if all {
                    self.marked.clear();
                } else {
                    self.marked.extend(shown.iter().copied());
                }
            }
            let ok = n > 0 && !busy;
            if ui.add_enabled(ok, egui::Button::new(RichText::new(format!("{}  {}", ic::ARROW_COUNTER_CLOCKWISE, tf!("herr.restaurar_n", n = n))).color(if ok { t.accent_text } else { t.faint })).fill(if ok { t.accent } else { t.hover })).clicked() {
                self.bin.restore(self.marked.drain().collect());
            }
            if ui.add_enabled(ok, egui::Button::new(format!("{}  {}", ic::X_CIRCLE, t!("comun.eliminar_definitivamente")))).on_hover_text(t!("herr.windows_pedira_confirmacion")).clicked() {
                self.bin.delete(self.marked.drain().collect());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!items.is_empty() && !busy, egui::Button::new(RichText::new(format!("{}  {}", ic::TRASH, t!("herr.vaciar_papelera"))).color(t.danger))).on_hover_text(t!("herr.windows_pedira_confirmacion")).clicked() {
                    self.marked.clear();
                    self.bin.empty();
                }
            });
        });
        ui.add_space(6.0);
        if items.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(egui_phosphor::fill::CHECK_CIRCLE).font(theme::icon_font(40.0, true)).color(t.accent));
                ui.label(RichText::new(t!("herr.papelera_esta_vacia")).font(theme::semibold(16.0)).color(t.text));
            });
            return;
        }
        let row_h = 28.0;
        let mut toggle = None;
        egui::ScrollArea::vertical().id_salt("recycle-list").auto_shrink([false, false]).show_rows(ui, row_h, shown.len(), |ui, range| {
            for &k in &shown[range] {
                let it = &items[k];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let p = ui.painter();
                let on = self.marked.contains(&k);
                if on {
                    p.rect_filled(rect, 6.0, t.sel_inactive);
                } else if resp.hovered() {
                    p.rect_filled(rect, 6.0, t.hover);
                }
                check_icon(ui, egui::Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(20.0, 20.0)), t, on);
                let (glyph, color) = super::icons::for_entry(t, it.is_dir, &fsx::ext_of(&it.name));
                p.text(pos2(rect.left() + 40.0, rect.center().y), Align2::CENTER_CENTER, glyph, theme::icon_font(15.0, true), color);
                let date_w = 140.0;
                let size_w = 80.0;
                let name_w = (rect.width() - 64.0 - date_w - size_w) * 0.45;
                let g = elided(p, &it.name, FontId::proportional(13.0), t.text, name_w);
                p.galley(pos2(rect.left() + 56.0, rect.center().y - g.size().y / 2.0), g, t.text);
                let g = elided(p, &it.from, FontId::proportional(12.0), t.faint, rect.width() - 64.0 - date_w - size_w - name_w - 16.0);
                p.galley(pos2(rect.left() + 64.0 + name_w, rect.center().y - g.size().y / 2.0), g, t.faint);
                p.text(pos2(rect.right() - size_w - 12.0, rect.center().y), Align2::RIGHT_CENTER, fsx::fmt_date(it.deleted), FontId::proportional(12.0), t.dim);
                if !it.is_dir || it.size > 0 {
                    p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, fsx::human_size(it.size), FontId::proportional(12.0), t.dim);
                }
                if resp.clicked() {
                    toggle = Some(k);
                }
                resp.on_hover_text(it.original_path()).context_menu(|ui| {
                    if ui.button(format!("{}  {}", ic::ARROW_COUNTER_CLOCKWISE, t!("comun.restaurar"))).clicked() {
                        self.bin.restore(vec![k]);
                        ui.close();
                    }
                    if ui.button(format!("{}  {}", ic::FOLDER_OPEN, t!("herr.abrir_carpeta_original"))).clicked() {
                        cx.actions.push(Action::NewTab(Some(it.from.clone())));
                        ui.close();
                    }
                });
            }
        });
        if let Some(k) = toggle {
            if !self.marked.remove(&k) {
                self.marked.insert(k);
            }
        }
        if busy {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
    }
}

// ---------------------------------------------------------------- comparar carpetas

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Lo más nuevo de cada lado gana.
    Both,
    /// Actualizar la derecha con lo de la izquierda.
    ToRight,
    /// La derecha queda idéntica a la izquierda (lo que sobra va a la papelera).
    Mirror,
}

#[derive(Default)]
struct CmpJob {
    cancel: AtomicBool,
    scanned: AtomicUsize,
    result: Mutex<Option<Result<Vec<compare::Item>, String>>>,
}

pub struct CompareTool {
    pub left: String,
    pub right: String,
    job: Option<Arc<CmpJob>>,
    items: Option<Result<Vec<compare::Item>, String>>,
    mode: Mode,
    show_same: bool,
    /// Copias de la última sincronización: al terminar se vuelve a comparar.
    waiting: Vec<u64>,
}

impl CompareTool {
    pub fn new(left: String, right: String, ctx: &egui::Context) -> Self {
        let mut c = CompareTool { left, right, job: None, items: None, mode: Mode::Both, show_same: false, waiting: Vec::new() };
        if !c.right.is_empty() {
            c.start(ctx);
        }
        c
    }

    fn start(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        let job = Arc::new(CmpJob::default());
        self.job = Some(job.clone());
        self.items = None;
        let (l, r) = (fsx::normalize(self.left.trim()), fsx::normalize(self.right.trim()));
        let wake = wake_of(ctx);
        std::thread::Builder::new()
            .name("comparar".into())
            .spawn(move || {
                let res = if l.eq_ignore_ascii_case(&r) { Err(t!("herr.elige_dos_carpetas_distintas").to_string()) } else { compare::compare(&l, &r, &job.cancel, &job.scanned) };
                *job.result.lock() = Some(res);
                wake();
            })
            .ok();
    }

    fn apply_mode(&mut self) {
        let mode = self.mode;
        if let Some(Ok(items)) = &mut self.items {
            use compare::{Dir, Status};
            for it in items {
                it.action = match (mode, it.status) {
                    (Mode::Both, s) => compare::Item::default_action(s),
                    (_, Status::Same) => Dir::Skip,
                    (Mode::ToRight, Status::OnlyLeft | Status::LeftNewer | Status::Different) => Dir::ToRight,
                    (Mode::ToRight, _) => Dir::Skip,
                    (Mode::Mirror, Status::OnlyRight) => Dir::DeleteRight,
                    (Mode::Mirror, _) => Dir::ToRight,
                };
            }
        }
    }

    /// Rótulo y campo de una carpeta: se escribe, se pega, se arrastra encima o se elige
    /// con el diálogo de Windows. Devuelve si hay que volver a comparar.
    fn path_field(ui: &mut Ui, t: &Theme, label: &str, path: &mut String, width: f32) -> bool {
        let mut changed = false;
        ui.vertical(|ui| {
            ui.set_width(width);
            widgets::caption(ui, t, label);
            let mut f = widgets::Field::new(("cmp-ruta", label), egui_phosphor::fill::FOLDER, t!("herr.escribe_pega_arrastra_carpeta"));
            f.width = width;
            f.droppable = true;
            f.trailing = Some((ic::FOLDER_OPEN, t!("herr.elegir_carpeta")));
            // En rojo si la carpeta no existe (no mientras se escribe: a medio escribir nunca existe).
            let typing = ui.memory(|m| m.has_focus(f.id));
            f.tone = if !typing && !path.trim().is_empty() && !std::path::Path::new(&fsx::long_path(&fsx::normalize(path.trim()))).is_dir() { Tone::Bad } else { Tone::Normal };
            let out = widgets::field(ui, t, path, f);
            // Soltar una carpeta sobre el campo.
            if let Some(drop) = out.outer.dnd_release_payload::<views::DragFiles>() {
                if let Some(p) = drop.0.first() {
                    *path = p.clone();
                    changed = true;
                }
            }
            if out.trailing_clicked {
                if let Some(p) = super::win::pick_folder(path) {
                    *path = p;
                    changed = true;
                }
            }
            if out.edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                changed = true;
            }
        });
        changed
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut ToolCx) {
        let t = cx.t;
        let ctx = ui.ctx().clone();
        // Resultado listo.
        let done = self.job.as_ref().and_then(|j| j.result.lock().take());
        if let Some(res) = done {
            self.items = Some(res);
            self.job = None;
            self.apply_mode();
        }
        // Al terminar la sincronización, comparar otra vez.
        if !self.waiting.is_empty() {
            let jobs = cx.xfer.snapshot();
            let pending = jobs.iter().any(|j| self.waiting.contains(&j.id) && !j.finished());
            if !pending {
                self.waiting.clear();
                self.start(&ctx);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }

        ui.horizontal(|ui| header(ui, t, ic::ARROWS_LEFT_RIGHT, t!("herr.comparar_sincronizar")));
        ui.add_space(8.0);
        let mut run = false;
        // Izquierda ⇄ Derecha [Comparar], todo a la misma altura.
        let ready = !self.left.trim().is_empty() && !self.right.trim().is_empty();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let compare_w = 118.0;
            let field_w = ((ui.available_width() - 34.0 - compare_w - 3.0 * 8.0) / 2.0).clamp(180.0, 420.0);
            run |= Self::path_field(ui, t, t!("herr.izquierda"), &mut self.left, field_w);
            ui.vertical(|ui| {
                ui.add_space(ui.text_style_height(&egui::TextStyle::Small) + 4.0);
                if widgets::icon_button_outlined(ui, t, ic::ARROWS_LEFT_RIGHT, widgets::FIELD_H).on_hover_text(t!("herr.intercambiar_lados")).clicked() {
                    std::mem::swap(&mut self.left, &mut self.right);
                    run = true;
                }
            });
            run |= Self::path_field(ui, t, t!("herr.derecha"), &mut self.right, field_w);
            ui.vertical(|ui| {
                ui.add_space(ui.text_style_height(&egui::TextStyle::Small) + 4.0);
                if widgets::primary_button(ui, t, ic::ARROW_CLOCKWISE, t!("herr.comparar"), ready).clicked() {
                    run = true;
                }
            });
        });
        if let Some(o) = &cx.other {
            ui.add_space(6.0);
            if widgets::secondary_button(ui, t, ic::SQUARE_SPLIT_HORIZONTAL, t!("herr.usar_carpeta_otro_panel_derecha")).clicked() {
                self.right = o.clone();
                run = true;
            }
        }
        if run && !self.left.trim().is_empty() && !self.right.trim().is_empty() {
            self.start(&ctx);
        }
        ui.add_space(10.0);

        if let Some(j) = &self.job {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tf!("herr.comparando_elementos_revisados", n = fsx::thousands(j.scanned.load(Ordering::Relaxed)))).color(t.dim));
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            return;
        }
        if !self.waiting.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(t!("herr.sincronizando_progreso_esta_cola_copias")).color(t.dim));
            });
            return;
        }
        let items = match &mut self.items {
            None => {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(ic::ARROWS_LEFT_RIGHT).font(theme::icon_font(44.0, false)).color(t.faint));
                    ui.add_space(6.0);
                    ui.label(RichText::new(t!("herr.elige_dos_carpetas_compararlas")).font(theme::semibold(15.0)).color(t.text));
                    ui.label(RichText::new(t!("herr.escribe_pega_rutas_arrastralas_desde")).size(12.5).color(t.dim));
                    ui.label(RichText::new(t!("herr.veras_cambio_cada_lado_podras")).size(12.5).color(t.dim));
                });
                return;
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e.as_str()).color(t.danger));
                return;
            }
            Some(Ok(items)) => items,
        };

        // Modo y resumen.
        let mut mode = self.mode;
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("herr.sincronizar")).color(t.dim));
            widgets::segmented(
                ui,
                t,
                &mut mode,
                &[
                    (Mode::Both, t!("herr.ambas_direcciones"), t!("herr.mas_nuevo_cada_lado_gana")),
                    (Mode::ToRight, t!("herr.actualizar_derecha"), t!("herr.copia_derecha_nuevo_cambiado_no")),
                    (Mode::Mirror, t!("herr.espejo_derecha"), t!("herr.derecha_queda_identica_sobra_va")),
                ],
            );
            ui.add_space(12.0);
            ui.checkbox(&mut self.show_same, t!("herr.mostrar_iguales"));
        });
        let mode_changed = mode != self.mode;
        self.mode = mode;
        use compare::{Dir, Status};
        let size_of = |it: &compare::Item, left: bool| if left { it.left.map_or(0, |x| x.0) } else { it.right.map_or(0, |x| x.0) };
        let (mut to_r, mut to_l, mut del, mut bytes) = (0, 0, 0, 0u64);
        for it in items.iter() {
            match it.action {
                Dir::ToRight => {
                    to_r += 1;
                    bytes += size_of(it, true);
                }
                Dir::ToLeft => {
                    to_l += 1;
                    bytes += size_of(it, false);
                }
                Dir::DeleteLeft | Dir::DeleteRight => del += 1,
                Dir::Skip => {}
            }
        }
        let same = items.iter().filter(|i| i.status == Status::Same).count();
        let diff = items.len() - same;
        ui.add_space(6.0);
        let mut sync = false;
        ui.horizontal(|ui| {
            let mut parts = vec![tf!("herr.diferencias_iguales", diferentes = diff, iguales = same)];
            if to_r > 0 {
                parts.push(format!("→ {to_r}"));
            }
            if to_l > 0 {
                parts.push(format!("← {to_l}"));
            }
            if del > 0 {
                parts.push(tf!("herr.n_a_la_papelera", n = del));
            }
            ui.label(RichText::new(parts.join("   ")).color(t.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = to_r + to_l + del > 0;
                let label = tf!("herr.sincronizar_tamano", tamano = fsx::human_size(bytes));
                if widgets::primary_button(ui, t, ic::ARROWS_CLOCKWISE, &label, ok).clicked() {
                    sync = true;
                }
            });
        });
        ui.add_space(6.0);

        // Lista.
        let visible: Vec<usize> = (0..items.len()).filter(|&k| self.show_same || items[k].status != Status::Same).collect();
        let row_h = 26.0;
        let mut cycle: Option<usize> = None;
        let w = ui.available_width();
        let side_w = 170.0;
        let act_w = 40.0;
        let name_w = (w - 2.0 * side_w - act_w - 16.0).max(120.0);
        // Encabezado.
        let (hr, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
        let hp = ui.painter();
        let hf = FontId::proportional(11.5);
        hp.text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, t!("herr.elemento"), hf.clone(), t.faint);
        hp.text(pos2(hr.left() + name_w + side_w, hr.center().y), Align2::RIGHT_CENTER, short_name(&self.left), hf.clone(), t.faint);
        hp.text(pos2(hr.right() - 8.0, hr.center().y), Align2::RIGHT_CENTER, short_name(&self.right), hf, t.faint);
        if visible.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(egui_phosphor::fill::CHECK_CIRCLE).font(theme::icon_font(40.0, true)).color(t.accent));
                ui.label(RichText::new(t!("herr.dos_carpetas_son_iguales")).font(theme::semibold(16.0)).color(t.text));
            });
        }
        egui::ScrollArea::vertical().id_salt("compare-list").auto_shrink([false, false]).show_rows(ui, row_h, visible.len(), |ui, range| {
            for &k in &visible[range] {
                let it = &items[k];
                let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                let painter = ui.painter();
                if resp.hovered() {
                    painter.rect_filled(rect, 6.0, t.hover);
                }
                let icon = if it.is_dir { egui_phosphor::fill::FOLDER } else { egui_phosphor::fill::FILE };
                painter.text(pos2(rect.left() + 14.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon_font(14.0, true), if it.is_dir { t.folder } else { t.faint });
                let g = elided(painter, &it.rel, FontId::proportional(13.0), t.text, name_w - 36.0);
                painter.galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, t.text);
                let newer_l = matches!(it.status, Status::LeftNewer | Status::OnlyLeft);
                let newer_r = matches!(it.status, Status::RightNewer | Status::OnlyRight);
                let cell = |side: Option<(u64, u32)>, hl: bool| match side {
                    None => ("—".to_string(), t.faint),
                    Some((size, mtime)) => {
                        let s = if it.is_dir { fsx::fmt_date(mtime) } else { format!("{} · {}", fsx::human_size(size), fsx::fmt_date(mtime)) };
                        (s, if hl { t.text } else if it.status == Status::Different { t.danger } else { t.dim })
                    }
                };
                let (ls, lc) = cell(it.left, newer_l);
                let (rs, rc) = cell(it.right, newer_r);
                let f = FontId::proportional(12.0);
                painter.text(pos2(rect.left() + name_w + side_w, rect.center().y), Align2::RIGHT_CENTER, ls, f.clone(), lc);
                painter.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, rs, f, rc);
                // Acción (clic para cambiarla).
                let ar = egui::Rect::from_center_size(pos2(rect.left() + name_w + side_w + act_w / 2.0, rect.center().y), vec2(30.0, 22.0));
                let aresp = ui.interact(ar, Id::new(("cmp-act", k)), Sense::click());
                let (aicon, acol, tip) = match it.action {
                    Dir::ToRight => (ic::ARROW_RIGHT, t.accent, t!("herr.copiar_derecha")),
                    Dir::ToLeft => (ic::ARROW_LEFT, t.accent, t!("herr.copiar_izquierda")),
                    Dir::DeleteLeft => (ic::TRASH, t.danger, t!("herr.mandar_papelera_izquierda")),
                    Dir::DeleteRight => (ic::TRASH, t.danger, t!("herr.mandar_papelera_derecha")),
                    Dir::Skip => (if it.status == Status::Same { ic::EQUALS } else { ic::MINUS }, t.faint, t!("herr.no_hacer_nada")),
                };
                if aresp.hovered() {
                    ui.painter().rect_filled(ar, 5.0, t.border);
                }
                ui.painter().text(ar.center(), Align2::CENTER_CENTER, aicon, theme::icon_font(14.0, false), acol);
                if aresp.on_hover_text(tf!("herr.clic_cambiar", accion = tip)).clicked() {
                    cycle = Some(k);
                }
            }
        });
        if let Some(k) = cycle {
            let it = &mut items[k];
            let ch = it.choices();
            let pos = ch.iter().position(|d| *d == it.action).unwrap_or(0);
            it.action = ch[(pos + 1) % ch.len()];
        }

        if sync {
            let (l, r) = (fsx::normalize(self.left.trim()), fsx::normalize(self.right.trim()));
            let mut right_pairs = Vec::new();
            let mut left_pairs = Vec::new();
            let mut trash = Vec::new();
            for it in items.iter() {
                match it.action {
                    Dir::ToRight => right_pairs.push((fsx::join(&l, &it.rel), it.rel.clone())),
                    Dir::ToLeft => left_pairs.push((fsx::join(&r, &it.rel), it.rel.clone())),
                    Dir::DeleteLeft => trash.push(fsx::join(&l, &it.rel)),
                    Dir::DeleteRight => trash.push(fsx::join(&r, &it.rel)),
                    Dir::Skip => {}
                }
            }
            if !right_pairs.is_empty() {
                self.waiting.push(cx.xfer.enqueue_sync(right_pairs, r.clone()));
            }
            if !left_pairs.is_empty() {
                self.waiting.push(cx.xfer.enqueue_sync(left_pairs, l.clone()));
            }
            if !trash.is_empty() {
                cx.actions.push(Action::Delete(trash, false));
                if self.waiting.is_empty() {
                    // Solo borrados: comparar otra vez en un momento.
                    self.items = None;
                    self.start(&ctx);
                }
            }
        } else if mode_changed {
            self.apply_mode();
        }
    }
}
