//! Herramientas: renombrado en lote (diálogo), buscador de duplicados y
//! comparar / sincronizar carpetas (cada una en su propia pestaña).

use super::actions::Action;
use super::design::{self, Chip, DialogW, IconBtn, Kind, ListCol, Row2, Sel, h, icon, radius, sp, stroke, text};
use super::state::Wake;
use super::theme::Theme;
use super::views::{self, elided};
use super::widgets::{self, Tone};
use crate::fsx::{self, compare, dupes, rename, transfer::Transfers};
use crate::index::IndexManager;
use egui::{Align2, Id, Margin, Painter, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
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

/// Margen de las filas de lista: pill a 4 del borde y texto a 8 dentro del pill.
const PAD: f32 = sp::XS + sp::S;

/// Capas de una fila de lista (4.1): pill con margen 4 a los lados y 1 px de aire, hover que
/// entra y sale. `id` estable: dentro de `show_rows` los automáticos cambian al desplazarse.
fn row_fill(ui: &Ui, t: &Theme, rect: Rect, id: Id, hovered: bool, sel: Sel) {
    let hov = design::hover_amount(ui.ctx(), id, hovered);
    design::state_fill(ui.painter(), t, rect.shrink2(vec2(sp::XS, 1.0)), radius::M, hov, false, sel);
}

/// El ícono de 16 de un archivo (el de Windows si ya llegó), como en las listas de la vista.
fn file_icon(p: &Painter, t: &Theme, center: Pos2, path: &str, name: &str, is_dir: bool) {
    views::file_icon(p, t, center, icon::M, icon::M, path, is_dir, &fsx::ext_of(name), 1.0);
}

/// Estado vacío de una herramienta: ocupa lo que queda.
fn empty(ui: &mut Ui, t: &Theme, glyph: &str, title: &str, hints: &[&str]) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    design::empty_state_lines(ui.painter(), t, area, glyph, design::Tone::Neutral, title, hints);
}

/// Alto de un rótulo `small` más los 4 de `design::labeled`: para alinear un control sin rótulo
/// con la base de los que lo tienen.
fn label_gap(ui: &Ui) -> f32 {
    ui.painter().layout_no_wrap("Ag".into(), text::small(), egui::Color32::WHITE).size().y + sp::XS
}

/// Ancho de un `design::button` (para repartir una barra antes de dibujarla).
fn button_w(p: &Painter, kind: Kind, glyph: &str, label: &str) -> f32 {
    let font = if matches!(kind, Kind::Primary | Kind::Danger) { text::strong() } else { text::body() };
    design::text_width(p, label, font) + if glyph.is_empty() { 0.0 } else { icon::M + 6.0 } + 2.0 * sp::M
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
    /// Editar como texto: una línea por archivo con el nombre nuevo.
    text_mode: bool,
    text: String,
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
            text_mode: false,
            text: String::new(),
        }
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
        if self.text_mode {
            self.plan = rename::plan_text(&self.files, &self.text, &self.existing);
            self.planned_for = None;
        } else if self.planned_for.as_ref() != Some(&key) {
            let dir_name = fsx::file_name(&self.dir);
            self.plan = rename::plan(&dir_name, &self.files, &self.rules, &self.existing);
            self.planned_for = Some(key);
        }

        let title = tp!(self.files.len(), "herr.renombrar_elementos");
        let dir_name = fsx::file_name(&self.dir);
        let modal = design::dialog(ctx, t, "batch-rename", DialogW::L, |ui| {
            let was = self.text_mode;
            design::dialog_header_with(ui, t, ic::TEXT_AA, design::Tone::Neutral, &title, Some(dir_name.as_str()), |ui| {
                widgets::segmented(ui, t, &mut self.text_mode, &[(false, t!("herr.modo_reglas"), ""), (true, t!("herr.modo_texto"), t!("herr.editar_texto_ayuda"))]);
            });
            if self.text_mode && !was {
                // Empieza con lo que dejaría el plan actual (o los nombres de ahora).
                self.text = match &self.plan {
                    Ok(p) => p.iter().map(|x| x.new.as_str()).collect::<Vec<_>>().join("\n"),
                    Err(_) => self.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join("\n"),
                };
            }
            ui.add_space(sp::L);
            let plan_err = self.plan.is_err();
            if self.text_mode {
                // Como texto: una sola línea de ayuda; el editor y la vista previa, lado a lado.
                design::dialog_help(ui, t, t!("herr.editar_texto_ayuda"));
                ui.add_space(sp::S);
                let col = (ui.available_width() - sp::M) / 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = sp::M;
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        egui::ScrollArea::vertical().id_salt("batch-text").max_height(320.0).show(ui, |ui| {
                            let te = design::textarea(ui, t, Id::new("batch-text-edit"), &mut self.text, 8, true, plan_err);
                            if self.first_frame {
                                te.request_focus();
                                self.first_frame = false;
                            }
                        });
                    });
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        preview(ui, t, &self.plan);
                    });
                });
            } else {
                self.rules_ui(ui, t, plan_err);
                ui.add_space(sp::L);
                preview(ui, t, &self.plan);
            }

            let (changes, problems, first_problem) = match &self.plan {
                Ok(p) => (p.iter().filter(|x| x.old != x.new).count(), p.iter().filter(|x| x.problem.is_some()).count(), p.iter().find_map(|x| x.problem.clone())),
                Err(_) => (0, 0, None),
            };
            let text_mode = self.text_mode;
            let plan = &self.plan;
            let dir = &self.dir;
            design::dialog_footer(
                ui,
                |ui| match plan {
                    // En reglas, el error de la expresión ya está bajo «Buscar».
                    Err(e) if text_mode => {
                        design::status_line(ui, t, design::Tone::Danger, "", e);
                    }
                    Err(_) => {}
                    Ok(_) if problems > 0 => {
                        design::status_line(ui, t, design::Tone::Danger, "", &tf!("herr.n_con_problemas", problemas = problems, primero = first_problem.unwrap_or_default()));
                    }
                    Ok(_) => {
                        ui.label(RichText::new(tp!(changes, "herr.cambian_nombre")).font(text::small()).color(t.dim));
                    }
                },
                |ui| {
                    let ok = changes > 0 && problems == 0;
                    let b = design::button(ui, t, Kind::Primary, ic::CHECK, t!("herr.renombrar"), ok);
                    if design::tip_keys(b, t, t!("herr.renombrar"), "Ctrl Enter").clicked() || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl)) {
                        if let Ok(plan) = plan {
                            let (done, errors) = rename::apply(dir, plan);
                            out = Outcome::Applied(done, errors);
                        }
                    }
                    if design::button(ui, t, Kind::Secondary, "", t!("comun.cancelar"), true).clicked() {
                        out = Outcome::Cancel;
                    }
                },
            );
        });
        if modal.should_close() && matches!(out, Outcome::Open) {
            out = Outcome::Cancel;
        }
        out
    }

    /// Modo reglas: plantilla, buscar y reemplazar, formato, numeración y extensión.
    fn rules_ui(&mut self, ui: &mut Ui, t: &Theme, plan_err: bool) {
        let r = &mut self.rules;
        let full = ui.available_width();
        // Nombre nuevo: plantilla y piezas para insertar donde está el cursor.
        let tid = Id::new("batch-template");
        let hint = rename::token("nombre");
        let te = design::labeled(ui, t, t!("herr.nombre_nuevo"), full, |ui| {
            let mut f = widgets::Field::new("batch-template", ic::PENCIL_SIMPLE, &hint);
            f.tone = if plan_err && !r.regex { Tone::Bad } else { Tone::Normal };
            widgets::field(ui, t, &mut r.template, f)
        });
        if self.first_frame {
            te.edit.request_focus();
            self.first_frame = false;
        }
        ui.add_space(sp::XS);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp::XS;
            design::micro_label(ui, t!("herr.insertar"), t.faint);
            ui.add_space(sp::XS);
            for (tok, tip) in [
                (rename::token("nombre"), t!("herr.nombre_original")),
                (rename::token("n"), t!("herr.numero_desde_digitos_abajo")),
                (rename::token("fecha"), t!("herr.fecha_modificacion")),
                (rename::token("exif"), t!("herr.fecha_tomo_foto")),
                (rename::token("carpeta"), t!("herr.nombre_carpeta")),
            ] {
                if design::tip(widgets::chip(ui, t, &tok, true), t, tip).clicked() {
                    widgets::insert_at_cursor(ui.ctx(), tid, &mut r.template, &tok);
                }
            }
        });
        ui.add_space(sp::M);

        // Buscar y reemplazar; la casilla, a la altura de los campos.
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            let regex_w = design::text_width(ui.painter(), t!("herr.expresion_regular"), text::body()) + icon::M + sp::S;
            let half = ((ui.available_width() - regex_w - 2.0 * sp::S) / 2.0).max(150.0);
            design::labeled(ui, t, t!("herr.buscar"), half, |ui| {
                let mut f = widgets::Field::new("batch-find", ic::MAGNIFYING_GLASS, if r.regex { t!("herr.expresion_regular_p_ej_d") } else { t!("herr.texto_cambiar") });
                f.mono = r.regex;
                f.tone = if plan_err && r.regex { Tone::Bad } else { Tone::Normal };
                widgets::field(ui, t, &mut r.find, f);
            });
            design::labeled(ui, t, t!("herr.reemplazar"), half, |ui| {
                let mut f = widgets::Field::new("batch-replace", ic::ARROW_BEND_DOWN_RIGHT, if r.regex { t!("herr.usa_1_2_capturado") } else { t!("herr.vacio_borrar_texto") });
                f.mono = r.regex;
                widgets::field(ui, t, &mut r.replace, f);
            });
            ui.vertical(|ui| {
                ui.add_space(label_gap(ui));
                let c = design::checkbox(ui, t, &mut r.regex, t!("herr.expresion_regular"));
                design::tip(c, t, t!("herr.buscar_expresion_regular_reemplazo_1"));
            });
        });
        // Un error de la expresión, bajo «Buscar».
        if let Err(e) = &self.plan {
            ui.add_space(sp::XS);
            design::status_line(ui, t, design::Tone::Danger, "", e);
        }
        ui.add_space(sp::M);

        // Formato, numeración y extensión.
        let r = &mut self.rules;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            use rename::Case;
            let cases = [
                (Case::Keep, t!("herr.sin_cambios"), t!("herr.deja_nombre_como_esta")),
                (Case::Lower, t!("herr.minusculas"), t!("herr.todo_minusculas")),
                (Case::Upper, t!("herr.mayusculas"), t!("herr.todo_mayusculas")),
                (Case::Title, t!("herr.tipo_titulo"), t!("herr.cada_palabra_mayuscula")),
            ];
            // Mismas cuentas que `widgets::segmented`: el rótulo no ensancha la columna.
            let seg_w = cases.iter().map(|(_, l, _)| design::text_width(ui.painter(), l, text::body()) + 2.0 * sp::M).sum::<f32>() + 2.0 * sp::XXS;
            design::labeled(ui, t, t!("herr.mayusculas_minusculas"), seg_w, |ui| {
                widgets::segmented(ui, t, &mut r.case, &cases);
            });
            ui.add_space(sp::S);
            design::labeled(ui, t, t!("herr.desde"), 88.0, |ui| {
                widgets::stepper(ui, t, &mut r.start, 0, 999_999, 88.0);
            });
            design::labeled(ui, t, t!("herr.digitos"), 88.0, |ui| {
                let mut pad = r.pad as u32;
                if widgets::stepper(ui, t, &mut pad, 1, 8, 88.0) {
                    r.pad = pad as usize;
                }
            });
            // «Extensión» toma lo que sobra: la fila no ensancha el diálogo (760, `DialogW::L`).
            let ext_w = ui.available_width().max(96.0);
            design::labeled(ui, t, t!("herr.extension"), ext_w, |ui| {
                let mut f = widgets::Field::new("batch-ext", ic::DOT_OUTLINE, t!("herr.misma"));
                f.width = ext_w;
                f.no_clear = true;
                widgets::field(ui, t, &mut r.new_ext, f);
            });
        });
    }
}

/// Vista previa del renombrado: tarjeta de margen 4 (el pill de las filas, radio M, anida en el
/// radio L) con «ahora → quedará» y una flecha de 36 en el medio.
fn preview(ui: &mut Ui, t: &Theme, plan: &Result<Vec<rename::Planned>, String>) {
    design::card_frame(t).inner_margin(Margin::same(sp::XS as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
        let new_x = |r: Rect| r.center().x + 18.0;
        design::micro(ui.painter(), pos2(hr.left() + PAD, hr.center().y), Align2::LEFT_CENTER, t!("herr.ahora"), t.faint);
        design::micro(ui.painter(), pos2(new_x(hr), hr.center().y), Align2::LEFT_CENTER, t!("herr.quedara"), t.faint);
        let Ok(plan) = plan else {
            ui.set_min_height(120.0);
            return;
        };
        let row_h = h::ROW;
        egui::ScrollArea::vertical().id_salt("batch-preview").max_height(320.0).auto_shrink([false, true]).show_rows(ui, row_h, plan.len(), |ui, range| {
            for k in range {
                let p = &plan[k];
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                let id = Id::new(("batch-row", k));
                let resp = ui.interact(rect, id, Sense::hover());
                row_fill(ui, t, rect, id, resp.hovered(), Sel::None);
                let painter = ui.painter();
                let half = rect.width() / 2.0 - 18.0 - PAD;
                let g = elided(painter, &p.old, text::body(), t.dim, half);
                painter.galley(pos2(rect.left() + PAD, rect.center().y - g.size().y / 2.0), g, t.dim);
                let changed = p.old != p.new;
                painter.text(rect.center(), Align2::CENTER_CENTER, ic::ARROW_RIGHT, design::icon_font(icon::S), if changed { t.dim } else { t.faint });
                let x = new_x(rect);
                let right = rect.right() - PAD;
                // Con problema: nombre en rojo y el motivo en un chip al final (solo el ícono si no entra).
                let mut name_end = right;
                if let Some(pr) = &p.problem {
                    let mut chip = Chip { text: pr, icon: ic::WARNING, tone: design::Tone::Danger, compact: true, ..Default::default() };
                    if design::chip_width(painter, &chip) > (right - x) * 0.6 {
                        chip.text = "";
                    }
                    let cw = design::chip_width(painter, &chip);
                    design::chip_paint(painter, t, pos2(right - cw, rect.center().y), &chip, false);
                    name_end = right - cw - sp::S;
                }
                let color = if p.problem.is_some() {
                    t.danger
                } else if changed {
                    t.text
                } else {
                    t.faint
                };
                let g = elided(painter, &p.new, text::body(), color, (name_end - x).max(0.0));
                painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
                if let Some(pr) = &p.problem {
                    design::tip(resp, t, pr);
                }
            }
        });
    });
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
        // Subtítulo: nombre y tamaño si es uno; «N elementos · total» si son varios.
        let sub = match self.files.as_slice() {
            [one] => format!("{} · {}", fsx::file_name(one), fsx::human_size(self.total)),
            many => format!("{} · {}", tp!(many.len(), "comun.elementos_n"), fsx::human_size(self.total)),
        };
        let modal = design::dialog(ctx, t, "hash", DialogW::M, |ui| {
            design::dialog_header(ui, t, ic::FINGERPRINT, design::Tone::Neutral, t!("herr.hash"), Some(&sub));
            ui.add_space(sp::L);

            // Comparar con el hash que te dieron.
            let w = ui.available_width();
            design::labeled(ui, t, t!("herr.comparar_hash_esperado"), w, |ui| {
                let mut f = widgets::Field::new("hash-esperado", ic::SCALES, t!("herr.pega_hash_te_dieron_sha"));
                f.mono = true;
                f.tone = match verdict {
                    Some(true) => Tone::Good,
                    Some(false) => Tone::Bad,
                    None => Tone::Normal,
                };
                widgets::field(ui, t, &mut self.expected, f);
            });
            match verdict {
                Some(true) => {
                    ui.add_space(sp::S);
                    design::status_line(ui, t, design::Tone::Ok, "", t!("herr.coincide_archivo_correcto"));
                }
                Some(false) => {
                    ui.add_space(sp::S);
                    design::status_line(ui, t, design::Tone::Danger, ic::X_CIRCLE, t!("herr.no_coincide_archivo_puede_estar"));
                }
                None if !expected.is_empty() => {
                    ui.add_space(sp::S);
                    design::status_line(ui, t, design::Tone::Neutral, ic::HOURGLASS, t!("herr.esperando_termine_calculo"));
                }
                None => {
                    ui.add_space(sp::XS);
                    design::dialog_help(ui, t, t!("herr.sirve_comprobar_descarga_si_coincide"));
                }
            }
            ui.add_space(sp::L);

            // Progreso: medidor y bytes en la misma línea.
            if !finished {
                let done = self.done.load(Ordering::Relaxed);
                let frac = if self.total > 0 { (done as f32 / self.total as f32).clamp(0.0, 1.0) } else { 0.0 };
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
                let p = ui.painter();
                let bytes = tf!("herr.progreso_hecho_total", hecho = fsx::human_size(done), total = fsx::human_size(self.total));
                let bw = p.text(r.right_center(), Align2::RIGHT_CENTER, bytes, text::mono_s(), t.dim).width();
                let mw = (r.width() - bw - sp::M).min(328.0).max(0.0);
                design::dot_meter(p, t, Rect::from_min_size(pos2(r.left(), r.center().y - 1.5), vec2(mw, 3.0)), frac, t.accent);
                ui.add_space(sp::S);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }

            // Una tarjeta por archivo: su ícono, nombre y tamaño; las tres filas de hash.
            let mut copy: Option<(usize, &'static str, String)> = None;
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (k, path) in self.files.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(sp::S);
                    }
                    design::card_frame(t).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), h::HEAD), Sense::hover());
                        let p = ui.painter();
                        let name = fsx::file_name(path);
                        file_icon(p, t, pos2(hr.left() + icon::M / 2.0, hr.center().y), path, &name, false);
                        let sw = p.text(hr.right_center(), Align2::RIGHT_CENTER, fsx::human_size(self.sizes[k]), text::mono(), t.faint).width();
                        let x = hr.left() + icon::M + sp::S;
                        let g = elided(p, &name, text::strong(), t.text, (hr.right() - sw - sp::S - x).max(0.0));
                        p.galley(pos2(x, hr.center().y - g.size().y / 2.0), g, t.text);
                        ui.add_space(sp::XS);
                        match &results[k] {
                            None => {
                                design::busy(ui, t, t!("herr.calculando"));
                            }
                            Some(Err(e)) => {
                                design::status_line(ui, t, design::Tone::Danger, "", e);
                            }
                            Some(Ok(h)) => {
                                let hit = if expected.is_empty() { None } else { h.matches(&expected) };
                                for (alg, v) in [("SHA-256", &h.sha256), ("SHA-1", &h.sha1), ("MD5", &h.md5)] {
                                    let just_copied = self.copied.is_some_and(|(f, a, at)| f == k && a == alg && now - at < super::motion::FEEDBACK);
                                    if hash_row(ui, t, alg, v, hit == Some(alg), just_copied) {
                                        copy = Some((k, alg, v.clone()));
                                    }
                                }
                            }
                        }
                    });
                }
            });
            if let Some((k, alg, v)) = copy {
                ui.ctx().copy_text(v);
                self.copied = Some((k, alg, now));
            }
            if self.copied.is_some_and(|(_, _, at)| now - at < super::motion::FEEDBACK) {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            if self.cloud {
                ui.add_space(sp::S);
                design::status_line(ui, t, design::Tone::Warn, ic::CLOUD, t!("herr.algun_archivo_esta_solo_nube"));
            }
            // Mientras calcula, «Cancelar»; terminado, «Listo».
            design::dialog_footer(
                ui,
                |_| {},
                |ui| {
                    let (kind, label) = if finished { (Kind::Primary, t!("comun.listo")) } else { (Kind::Secondary, t!("comun.cancelar")) };
                    if design::button(ui, t, kind, "", label, true).clicked() {
                        open = false;
                    }
                },
            );
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

/// Una fila de hash (24): algoritmo en un chip mono, el valor (seleccionable) y copiar.
/// `hit`: es el que coincide con el esperado (solo el chip lo marca). Devuelve si se pidió copiar.
fn hash_row(ui: &mut Ui, t: &Theme, alg: &str, value: &str, hit: bool, copied: bool) -> bool {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, h::HEAD), Sense::hover());
    let p = ui.painter().clone();
    let hov = design::hover_amount(ui.ctx(), Id::new(("hash-row", alg, value)), ui.rect_contains_pointer(rect));
    design::state_fill(&p, t, rect, radius::M, hov, false, Sel::None);
    // Chip con el algoritmo; los valores empiezan todos en la misma columna (la del chip más ancho).
    // Todos del ancho del más ancho, con el texto centrado.
    let col = design::chip_width(&p, &Chip { text: "SHA-256", mono: true, icon: ic::CHECK, ..Default::default() });
    let chip = if hit { Chip { text: alg, mono: true, tone: design::Tone::Ok, icon: ic::CHECK, width: col, ..Default::default() } } else { Chip { text: alg, mono: true, width: col, ..Default::default() } };
    let chip_r = design::chip_paint(&p, t, pos2(rect.left() + sp::XS, rect.center().y), &chip, false);
    // Copiar: botón mini a la derecha; recién copiado, ✓ y «Copiado» en verde.
    let cr = Rect::from_center_size(pos2(rect.right() - sp::XS - h::CHIP / 2.0, rect.center().y), vec2(h::CHIP, h::CHIP));
    let cresp = design::icon_button_at(ui, t, cr, Id::new(("hash-copy", alg, value)), if copied { "" } else { ic::COPY }, IconBtn::mini());
    let mut end = cr.left() - sp::S;
    if copied {
        p.text(cr.center(), Align2::CENTER_CENTER, ic::CHECK, design::icon_font(icon::S), t.ok);
        end = p.text(pos2(cr.left() - sp::XS, rect.center().y), Align2::RIGHT_CENTER, t!("comun.copiado"), text::small(), t.ok).left() - sp::S;
    }
    // El valor, seleccionable con el mouse.
    let vr = Rect::from_min_max(pos2(chip_r.left() + col + sp::M, rect.top()), pos2(end.max(chip_r.left() + col + sp::M), rect.bottom()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add(egui::Label::new(RichText::new(value).font(text::mono()).color(t.text)).selectable(true).truncate());
    design::tip(cresp, t, t!("herr.copiar")).clicked()
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
        // Abierto por un fallo: placa de aviso y el motivo de subtítulo.
        let what = if self.paths.len() == 1 { format!("«{}»", fsx::file_name(&self.paths[0])) } else { tf!("comun.elementos", n = self.paths.len()) };
        let (tone, sub) = match &self.reason {
            Some(r) => (design::Tone::Warn, tf!("herr.esta_abierto_otro_programa", motivo = r, que = what)),
            None => (design::Tone::Accent, what),
        };
        let modal = design::dialog(ctx, t, "locks", DialogW::M, |ui| {
            design::dialog_header(ui, t, ic::LOCK_SIMPLE, tone, t!("herr.quien_esta_usando"), Some(&sub));
            ui.add_space(sp::L);
            match &result {
                None => {
                    design::busy(ui, t, t!("herr.preguntando_windows"));
                }
                Some(Err(e)) => {
                    design::status_line(ui, t, design::Tone::Danger, "", e);
                }
                Some(Ok(list)) if list.is_empty() => {
                    design::status_line(ui, t, design::Tone::Ok, "", t!("herr.ningun_programa_tiene_abierto_ahora"));
                    if self.retry.is_none() {
                        design::dialog_help(ui, t, t!("herr.si_igual_no_deja_borrar"));
                    }
                }
                Some(Ok(list)) => {
                    // Tarjeta con una fila de dos líneas por programa; las acciones, fantasmas.
                    design::card_frame(t).inner_margin(Margin::same(sp::XS as i8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for l in list {
                                let glyph = match l.kind {
                                    fsx::locks::Kind::Service => ic::GEAR,
                                    fsx::locks::Kind::Console => ic::TERMINAL_WINDOW,
                                    fsx::locks::Kind::Explorer => ic::FOLDER_OPEN,
                                    fsx::locks::Kind::Critical => ic::SHIELD,
                                    _ => ic::APP_WINDOW,
                                };
                                let name = if l.is_self { t!("herr.rayo_vista_previa_miniatura").to_string() } else { l.name.clone() };
                                let mut sub = if l.exe.is_empty() { tf!("herr.proceso", pid = l.pid) } else { tf!("herr.proceso_programa", programa = l.exe, pid = l.pid) };
                                match l.kind {
                                    fsx::locks::Kind::Service => sub.push_str(t!("herr.servicio_windows")),
                                    fsx::locks::Kind::Critical => sub.push_str(t!("herr.parte_windows")),
                                    fsx::locks::Kind::Explorer => sub.push_str(t!("herr.explorador")),
                                    _ => {}
                                }
                                let confirming = !l.is_self && self.confirm_kill == Some(l.pid);
                                // Confirmar el cierre forzado ocurre en la misma fila, que se tiñe.
                                let spec = if confirming {
                                    Row2 { icon: glyph, title: &name, sub: t!("herr.pierde_no_este_guardado"), sub_color: Some(t.danger), tone: design::Tone::Danger, ..Default::default() }
                                } else {
                                    Row2 { icon: glyph, title: &name, sub: &sub, ..Default::default() }
                                };
                                design::row2(ui, t, Id::new(("lock", l.pid)), spec, |ui| {
                                    ui.spacing_mut().item_spacing.x = sp::XS;
                                    if l.is_self {
                                        let b = design::button(ui, t, Kind::Ghost, "", t!("herr.soltarlo"), true);
                                        if design::tip(b, t, t!("herr.cierra_vista_previa_liberar_archivo")).clicked() {
                                            self.release_self = true;
                                            self.recheck_at = Some(now + 0.6);
                                        }
                                    } else if confirming {
                                        if design::button(ui, t, Kind::Danger, "", t!("herr.si_forzar"), true).clicked() {
                                            self.confirm_kill = None;
                                            match l.kill() {
                                                Ok(()) => self.recheck_at = Some(now + 0.5),
                                                Err(e) => self.error = Some(tf!("herr.no_pudo_cerrar", nombre = l.name, error = e)),
                                            }
                                        }
                                        if design::button(ui, t, Kind::Ghost, "", t!("herr.no"), true).clicked() {
                                            self.confirm_kill = None;
                                        }
                                    } else {
                                        if l.can_kill() {
                                            let b = design::button(ui, t, Kind::GhostDanger, "", t!("herr.forzar_cierre"), true);
                                            if design::tip(b, t, t!("herr.termina_programa_instante_pierde_no")).clicked() {
                                                self.confirm_kill = Some(l.pid);
                                            }
                                        }
                                        if l.can_close() {
                                            let b = design::button(ui, t, Kind::Ghost, "", t!("comun.cerrar"), true);
                                            if design::tip(b, t, t!("herr.como_tocar_su_x_programa")).clicked() {
                                                l.close();
                                                self.recheck_at = Some(now + 1.5);
                                            }
                                        }
                                    }
                                });
                            }
                        });
                    });
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(sp::S);
                design::status_line(ui, t, design::Tone::Danger, "", e);
            }
            // Pie: «Volver a mirar» fantasma a la izquierda; a la derecha el reintento (solo
            // cuando ya nadie lo usa) y «Cancelar», o un solo «Listo» si no hay nada que reintentar.
            let free = matches!(&result, Some(Ok(l)) if l.is_empty());
            let mut rescan = false;
            design::dialog_footer(
                ui,
                |ui| {
                    if design::button(ui, t, Kind::Ghost, ic::ARROW_CLOCKWISE, t!("herr.volver_mirar"), result.is_some()).clicked() {
                        rescan = true;
                    }
                },
                |ui| match &self.retry {
                    Some((label, a)) => {
                        if design::button(ui, t, Kind::Primary, "", label, free).clicked() {
                            actions.push(a.clone());
                            open = false;
                        }
                        if design::button(ui, t, Kind::Secondary, "", t!("comun.cancelar"), true).clicked() {
                            open = false;
                        }
                    }
                    None => {
                        if design::button(ui, t, Kind::Primary, "", t!("comun.listo"), true).clicked() {
                            open = false;
                        }
                    }
                },
            );
            if rescan {
                self.error = None;
                self.scan();
            }
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

    /// Lo que va a la izquierda de la barra de estado con la herramienta abierta. Solo el resumen
    /// de duplicados: la papelera ya lo dice en su subtítulo y comparar en su fila.
    pub fn status(&self) -> Option<String> {
        match self {
            Tool::Dupes(d) => d.status(),
            Tool::Compare(_) | Tool::Recycle(_) => None,
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
        let m = Margin { left: sp::L as i8, right: sp::L as i8, top: sp::M as i8, bottom: sp::S as i8 };
        egui::Frame::new().inner_margin(m).show(ui, |ui| match self {
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

    /// «N grupos · N copias de más · se pueden liberar X · 0,3 s, con el índice», con el análisis
    /// terminado y algo encontrado.
    fn status(&self) -> Option<String> {
        if self.took.is_none() || self.groups.is_empty() {
            return None;
        }
        let files: usize = self.groups.iter().map(|g| g.files.len() - 1).sum();
        let wasted: u64 = self.groups.iter().map(|g| g.wasted()).sum();
        let src = if self.job.from_index.load(Ordering::Relaxed) { t!("herr.indice") } else { t!("herr.recorriendo_carpetas") };
        Some(tf!("herr.grupos_copias_mas_pueden_liberar", grupos = fsx::thousands(self.groups.len()), copias = fsx::thousands(files), tamano = fsx::human_size(wasted), segundos = format!("{:.1}", self.took.unwrap_or(0.0)), como = src))
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut ToolCx) {
        let t = cx.t;
        let phase = self.job.phase.load(Ordering::Relaxed);
        if phase == 2 && self.took.is_none() {
            self.groups = self.job.result.lock().take().unwrap_or_default();
            self.took = Some(self.started.elapsed().as_secs_f32());
        }
        // Encabezado: la ruta de subtítulo; tamaño mínimo y «Buscar otra vez» a la derecha.
        let mut restart = false;
        design::tool_header(ui, t, ic::COPY_SIMPLE, t!("herr.archivos_duplicados"), Some(self.scope.as_str()), true, |ui| {
            if design::button(ui, t, Kind::Secondary, ic::ARROW_CLOCKWISE, t!("herr.buscar_otra_vez"), true).clicked() {
                restart = true;
            }
            let opts: Vec<(u64, &str)> = MIN_SIZES.iter().map(|(v, l)| (*v, min_size_label(l))).collect();
            restart |= design::select(ui, t, "dupes-min", &mut self.min_size, &opts, 96.0);
            ui.label(RichText::new(t!("herr.ignorar_menores")).font(text::small()).color(t.dim));
        });
        if restart {
            self.start(cx.mgr.clone(), wake_of(ui.ctx()));
            ui.ctx().request_repaint();
            return;
        }
        ui.add_space(sp::M);
        let cloud = self.job.prog.skipped_cloud.load(Ordering::Relaxed);
        let cloud_line = |ui: &mut Ui| {
            if cloud > 0 {
                ui.add_space(sp::S);
                design::status_line(ui, t, design::Tone::Neutral, ic::CLOUD, &tf!("herr.archivos_solo_nube_no_revisados", nube = cloud));
            }
        };

        if phase < 2 {
            // Buscando: texto, medidor de puntos, bytes en mono y «Cancelar».
            let (done, total) = (self.job.prog.done.load(Ordering::Relaxed), self.job.prog.total.load(Ordering::Relaxed));
            let what = if phase == 0 {
                t!("herr.buscando_archivos_mismo_tamano").to_string()
            } else {
                tf!("herr.comparando_contenido_candidatos", n = fsx::thousands(self.job.candidates.load(Ordering::Relaxed)))
            };
            ui.label(RichText::new(what).font(text::body()).color(t.text));
            ui.add_space(sp::S);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sp::M;
                let (r, _) = ui.allocate_exact_size(vec2(320.0, h::HEAD), Sense::hover());
                let meter = Rect::from_min_size(pos2(r.left(), r.center().y - 1.5), vec2(r.width(), 3.0));
                if total > 0 {
                    design::dot_meter(ui.painter(), t, meter, done as f32 / total as f32, t.accent);
                    ui.label(RichText::new(tf!("herr.progreso_hecho_total", hecho = fsx::human_size(done), total = fsx::human_size(total))).font(text::mono_s()).color(t.dim));
                } else {
                    // Todavía no se sabe cuánto hay: el medidor que recorre.
                    design::dot_meter_busy(ui.painter(), t, meter);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if design::button(ui, t, Kind::Secondary, "", t!("comun.cancelar"), true).clicked() {
                        self.job.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
            cloud_line(ui);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            return;
        }
        if self.groups.is_empty() {
            cloud_line(ui);
            let msg = if self.job.cancel.load(Ordering::Relaxed) { t!("herr.busqueda_cancelada") } else { t!("herr.no_hay_archivos_duplicados") };
            empty(ui, t, ic::CHECK_CIRCLE, msg, &[]);
            return;
        }

        let marked_size: u64 = self.groups.iter().flat_map(|g| g.files.iter().filter(|f| self.marked.contains(&f.path)).map(move |_| g.size)).sum();

        // Barra de acciones: marcar a la izquierda; mover a la papelera (el único lleno) a la
        // derecha. Se mide antes: si no entra, el botón baja a otra fila. El resumen del análisis
        // va a la izquierda de la barra de estado (`Tool::status`).
        let n = self.marked.len();
        let trash_label = tf!("herr.mover_papelera", n = n, tamano = fsx::human_size(marked_size));
        let p = ui.painter().clone();
        let left_w = button_w(&p, Kind::Secondary, "", t!("herr.marcar_copias_conservar_mas_antigua"))
            + button_w(&p, Kind::Secondary, "", t!("herr.conservar_mas_nueva"))
            + button_w(&p, Kind::Ghost, "", t!("herr.desmarcar_todo"))
            + 2.0 * sp::S;
        let trash_w = button_w(&p, Kind::Danger, ic::TRASH, &trash_label);
        let avail = ui.available_width();
        let one_row = left_w + sp::S + trash_w <= avail;
        let mut trash = false;
        let right = |ui: &mut Ui, trash: &mut bool| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if design::button(ui, t, Kind::Danger, ic::TRASH, &trash_label, n > 0).clicked() {
                    *trash = true;
                }
            });
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            if design::button(ui, t, Kind::Secondary, "", t!("herr.marcar_copias_conservar_mas_antigua"), true).clicked() {
                self.mark_all(true);
            }
            if design::button(ui, t, Kind::Secondary, "", t!("herr.conservar_mas_nueva"), true).clicked() {
                self.mark_all(false);
            }
            if design::button(ui, t, Kind::Ghost, "", t!("herr.desmarcar_todo"), !self.marked.is_empty()).clicked() {
                self.marked.clear();
            }
            if one_row {
                right(ui, &mut trash);
            }
        });
        if !one_row {
            ui.add_space(sp::S);
            ui.horizontal(|ui| right(ui, &mut trash));
        }
        if trash {
            let paths: Vec<String> = self.marked.drain().collect();
            let gone: HashSet<&String> = paths.iter().collect();
            for g in &mut self.groups {
                g.files.retain(|f| !gone.contains(&f.path));
            }
            self.groups.retain(|g| g.files.len() > 1);
            cx.actions.push(Action::Delete(paths, false));
        }
        if let Some(hint) = &self.hint {
            ui.add_space(sp::S);
            design::status_line(ui, t, design::Tone::Warn, "", hint);
        }
        cloud_line(ui);
        ui.add_space(sp::S);

        // Lista: encabezado de columnas; un encabezado por grupo y una fila por archivo.
        let mut rows: Vec<(usize, Option<usize>)> = Vec::new();
        for (gi, g) in self.groups.iter().enumerate() {
            rows.push((gi, None));
            rows.extend((0..g.files.len()).map(|fi| (gi, Some(fi))));
        }
        let cols = DupeCols::new(ui.available_width());
        design::list_header(
            ui,
            t,
            &[
                ListCol { label: t!("lista.nombre"), x: cols.name_x, align: Align2::LEFT_CENTER },
                ListCol { label: t!("comun.carpeta"), x: cols.dir_x, align: Align2::LEFT_CENTER },
                ListCol { label: t!("comun.modificado"), x: cols.w - PAD, align: Align2::RIGHT_CENTER },
            ],
        );
        // Encabezados de grupo y archivos en la misma lista virtual: todas las filas miden `ROW`.
        let row_h = h::ROW;
        let mut toggle: Option<(usize, String)> = None;
        egui::ScrollArea::vertical().id_salt("dupes-list").auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
            for &(gi, fi) in &rows[range] {
                let g = &self.groups[gi];
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                match fi {
                    None => {
                        let painter = ui.painter();
                        painter.hline(rect.x_range(), rect.top() + sp::XXS + 0.5, Stroke::new(stroke::HAIR, t.border));
                        let label = tf!("herr.copias_cada", n = g.files.len(), tamano = fsx::human_size(g.size));
                        design::micro(painter, pos2(rect.left() + PAD, rect.center().y + sp::XXS), Align2::LEFT_CENTER, &label, t.faint);
                    }
                    Some(fi) => {
                        let f = &g.files[fi];
                        let on = self.marked.contains(&f.path);
                        let id = Id::new(("dupe", &f.path));
                        let resp = ui.interact(rect, id, Sense::click());
                        // Marcada: casilla de acento y capa gris; el nombre no se apaga.
                        row_fill(ui, t, rect, id, resp.hovered(), if on { Sel::Unfocused } else { Sel::None });
                        let c = DupeCols::new(rect.width());
                        let painter = ui.painter();
                        let cy = rect.center().y;
                        design::check_icon(painter, t, Rect::from_center_size(pos2(rect.left() + PAD + icon::M / 2.0, cy), vec2(icon::M, icon::M)), on);
                        let name = fsx::file_name(&f.path);
                        file_icon(painter, t, pos2(rect.left() + c.icon_x + icon::M / 2.0, cy), &f.path, &name, false);
                        let g1 = elided(painter, &name, text::body(), t.text, c.name_w);
                        painter.galley(pos2(rect.left() + c.name_x, cy - g1.size().y / 2.0), g1, t.text);
                        let parent = fsx::parent_of(&f.path).unwrap_or_default();
                        let shown = design::elide_middle(painter, &parent, text::mono(), c.dir_w);
                        painter.text(pos2(rect.left() + c.dir_x, cy), Align2::LEFT_CENTER, shown, text::mono(), t.faint);
                        painter.text(pos2(rect.right() - PAD, cy), Align2::RIGHT_CENTER, fsx::fmt_date(f.mtime), text::mono(), t.dim);
                        if resp.clicked() {
                            toggle = Some((gi, f.path.clone()));
                        }
                        if resp.double_clicked() {
                            toggle = None;
                            cx.actions.push(Action::NewTab(Some(parent.clone())));
                            cx.actions.push(Action::Reveal(f.path.clone()));
                        }
                        design::context_menu(&resp, t, |ui| {
                            if design::menu_item(ui, t, ic::FOLDER_OPEN, t!("comun.mostrar_su_carpeta"), "").clicked() {
                                cx.actions.push(Action::NewTab(Some(parent.clone())));
                                cx.actions.push(Action::Reveal(f.path.clone()));
                            }
                            if design::menu_item(ui, t, ic::ARROW_SQUARE_OUT, t!("comun.abrir"), "").clicked() {
                                cx.actions.push(Action::Open { path: f.path.clone(), is_dir: false });
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

/// Columnas de la lista de duplicados (M-14): casilla, ícono, nombre (hasta 320), carpeta y
/// fecha (130, a la derecha), con 8 entre sí. Las x van desde el borde izquierdo de la fila.
struct DupeCols {
    w: f32,
    icon_x: f32,
    name_x: f32,
    name_w: f32,
    dir_x: f32,
    dir_w: f32,
}

impl DupeCols {
    fn new(w: f32) -> Self {
        let icon_x = PAD + icon::M + sp::S;
        let name_x = icon_x + icon::M + sp::S;
        let date_w = 130.0;
        let mid = (w - name_x - PAD - date_w - 2.0 * sp::S).max(0.0);
        let name_w = (mid * 0.5).min(320.0);
        let dir_x = name_x + name_w + sp::S;
        DupeCols { w, icon_x, name_x, name_w, dir_x, dir_w: (mid - name_w).max(0.0) }
    }
}

// ---------------------------------------------------------------- papelera

pub struct RecycleTool {
    bin: fsx::recycle::Bin,
    /// Marcados por clave (no por posición: la lista se vuelve a leer tras cada orden).
    marked: HashSet<String>,
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
        let busy = self.bin.is_busy();
        // Encabezado: el resumen de subtítulo; filtro y «Actualizar» a la derecha.
        let summary = items.as_ref().map(|items| {
            let total: u64 = items.iter().map(|i| i.size).sum();
            tp!(items.len(), "herr.papelera_resumen", n = fsx::thousands(items.len()), tamano = fsx::human_size(total))
        });
        design::tool_header(ui, t, ic::TRASH_SIMPLE, t!("comun.papelera"), summary.as_deref(), false, |ui| {
            if design::button(ui, t, Kind::Secondary, ic::ARROW_CLOCKWISE, t!("herr.actualizar"), true).clicked() {
                self.marked.clear();
                self.bin.refresh();
            }
            let mut f = widgets::Field::new("papelera-filtro", ic::FUNNEL, t!("herr.filtrar_nombre_carpeta"));
            f.width = 240.0;
            widgets::field(ui, t, &mut self.filter, f);
            // Restaurando o borrando: el medidor que recorre.
            if busy && items.is_some() {
                design::busy(ui, t, "");
            }
        });
        let Some(items) = items else {
            ui.add_space(sp::M);
            design::busy(ui, t, t!("herr.leyendo_papelera"));
            return;
        };
        let key = crate::index::store::search_key(self.filter.trim());
        let shown: Vec<usize> = (0..items.len())
            .filter(|&k| key.is_empty() || crate::index::store::search_key(&items[k].name).contains(&key) || crate::index::store::search_key(&items[k].from).contains(&key))
            .collect();
        ui.add_space(sp::M);
        // Lo que ya no está en la papelera deja de estar marcado.
        if !busy {
            let ids: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
            self.marked.retain(|k| ids.contains(k.as_str()));
        }
        // Barra de acciones: un solo lleno («Restaurar»); lo irreversible, fantasma en rojo
        // (Windows vuelve a pedir confirmación).
        let n = self.marked.len();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            let all = !shown.is_empty() && shown.iter().all(|&k| self.marked.contains(&items[k].id));
            if design::button(ui, t, Kind::Secondary, "", if all { t!("herr.desmarcar_todo") } else { t!("herr.marcar_todo") }, true).clicked() {
                if all {
                    self.marked.clear();
                } else {
                    self.marked.extend(shown.iter().map(|&k| items[k].id.clone()));
                }
            }
            let ok = n > 0 && !busy;
            if design::button(ui, t, Kind::Primary, ic::ARROW_COUNTER_CLOCKWISE, &tf!("herr.restaurar_n", n = n), ok).clicked() {
                self.bin.restore(self.marked.drain().collect());
            }
            let b = design::button(ui, t, Kind::GhostDanger, ic::X_CIRCLE, t!("comun.eliminar_definitivamente"), ok);
            if design::tip(b, t, t!("herr.windows_pedira_confirmacion")).clicked() {
                self.bin.delete(self.marked.drain().collect());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = design::button(ui, t, Kind::GhostDanger, ic::TRASH, t!("herr.vaciar_papelera"), !items.is_empty() && !busy);
                if design::tip(b, t, t!("herr.windows_pedira_confirmacion")).clicked() {
                    self.marked.clear();
                    self.bin.empty();
                }
            });
        });
        ui.add_space(sp::S);
        if items.is_empty() {
            empty(ui, t, ic::CHECK_CIRCLE, t!("herr.papelera_esta_vacia"), &[]);
            return;
        }
        let cols = BinCols::new(ui.available_width());
        // «Estaba en» y «Eliminado» todavía no tienen clave propia: «Carpeta» y nada.
        let from_label = crate::i18n::tr_dyn("herr.estaba_en").unwrap_or(t!("comun.carpeta"));
        let deleted_label = crate::i18n::tr_dyn("herr.eliminado").unwrap_or("");
        design::list_header(
            ui,
            t,
            &[
                ListCol { label: t!("lista.nombre"), x: cols.name_x, align: Align2::LEFT_CENTER },
                ListCol { label: from_label, x: cols.from_x, align: Align2::LEFT_CENTER },
                ListCol { label: deleted_label, x: cols.date_r, align: Align2::RIGHT_CENTER },
                ListCol { label: t!("comun.tamano"), x: cols.w - PAD, align: Align2::RIGHT_CENTER },
            ],
        );
        let row_h = h::ROW;
        let mut toggle = None;
        egui::ScrollArea::vertical().id_salt("recycle-list").auto_shrink([false, false]).show_rows(ui, row_h, shown.len(), |ui, range| {
            for &k in &shown[range] {
                let it = &items[k];
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                let id = Id::new(("papelera", &it.id));
                let resp = ui.interact(rect, id, Sense::click());
                let on = self.marked.contains(&it.id);
                row_fill(ui, t, rect, id, resp.hovered(), if on { Sel::Unfocused } else { Sel::None });
                let c = BinCols::new(rect.width());
                let p = ui.painter();
                let cy = rect.center().y;
                // Casilla en la columna de ícono, luego el ícono real del archivo y el nombre.
                design::check_icon(p, t, Rect::from_center_size(pos2(rect.left() + PAD + icon::M / 2.0, cy), vec2(icon::M, icon::M)), on);
                file_icon(p, t, pos2(rect.left() + c.icon_x + icon::M / 2.0, cy), &it.id, &it.name, it.is_dir);
                let g = elided(p, &it.name, text::body(), t.text, c.name_w);
                p.galley(pos2(rect.left() + c.name_x, cy - g.size().y / 2.0), g, t.text);
                let from = design::elide_middle(p, &it.from, text::mono(), c.from_w);
                p.text(pos2(rect.left() + c.from_x, cy), Align2::LEFT_CENTER, from, text::mono(), t.faint);
                p.text(pos2(rect.left() + c.date_r, cy), Align2::RIGHT_CENTER, fsx::fmt_date(it.deleted), text::mono(), t.dim);
                if !it.is_dir || it.size > 0 {
                    p.text(pos2(rect.right() - PAD, cy), Align2::RIGHT_CENTER, fsx::human_size(it.size), text::mono(), t.dim);
                }
                if resp.clicked() {
                    toggle = Some(it.id.clone());
                }
                let resp = design::tip(resp, t, &it.original_path());
                design::context_menu(&resp, t, |ui| {
                    let restore = design::MenuItem { icon: ic::ARROW_COUNTER_CLOCKWISE, label: t!("comun.restaurar"), disabled: busy, ..Default::default() };
                    if design::menu_item_ex(ui, t, restore).clicked() {
                        self.marked.remove(&it.id);
                        self.bin.restore(vec![it.id.clone()]);
                    }
                    if design::menu_item(ui, t, ic::FOLDER_OPEN, t!("herr.abrir_carpeta_original"), "").clicked() {
                        cx.actions.push(Action::NewTab(Some(it.from.clone())));
                    }
                });
            }
        });
        if let Some(id) = toggle {
            if !self.marked.remove(&id) {
                self.marked.insert(id);
            }
        }
        if busy {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
    }
}

/// Columnas de la papelera (M-16): casilla, ícono, nombre (hasta 320), «estaba en», fecha (130)
/// y tamaño (76), con 8 entre sí. Las x van desde el borde izquierdo de la fila.
struct BinCols {
    w: f32,
    icon_x: f32,
    name_x: f32,
    name_w: f32,
    from_x: f32,
    from_w: f32,
    /// Borde derecho de la fecha.
    date_r: f32,
}

impl BinCols {
    fn new(w: f32) -> Self {
        let icon_x = PAD + icon::M + sp::S;
        let name_x = icon_x + icon::M + sp::S;
        let (date_w, size_w) = (130.0, 76.0);
        let date_r = w - PAD - size_w - sp::S;
        let mid = (date_r - date_w - sp::S - name_x - sp::S).max(0.0);
        let name_w = (mid * 0.5).min(320.0);
        let from_x = name_x + name_w + sp::S;
        BinCols { w, icon_x, name_x, name_w, from_x, from_w: (mid - name_w).max(0.0), date_r }
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
        design::labeled(ui, t, label, width, |ui| {
            let mut f = widgets::Field::new(("cmp-ruta", label), ic::FOLDER, t!("herr.escribe_pega_arrastra_carpeta"));
            f.width = width;
            f.mono = true;
            // Sin foco, el final de la ruta con «…» en el medio.
            f.elide_middle = true;
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

        design::tool_header(ui, t, ic::ARROWS_LEFT_RIGHT, t!("herr.comparar_sincronizar"), None, false, |_| {});
        ui.add_space(sp::M);
        let mut run = false;
        // Izquierda ⇄ Derecha [Comparar], todo a la misma altura.
        let ready = !self.left.trim().is_empty() && !self.right.trim().is_empty();
        // Un solo lleno por vista: «Comparar» lo es solo antes de la primera comparación.
        let compare_kind = if self.items.is_none() { Kind::Primary } else { Kind::Secondary };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            let compare_w = button_w(ui.painter(), compare_kind, ic::ARROW_CLOCKWISE, t!("herr.comparar"));
            let field_w = ((ui.available_width() - h::CTL - compare_w - 3.0 * sp::S) / 2.0).max(180.0);
            run |= Self::path_field(ui, t, t!("herr.izquierda"), &mut self.left, field_w);
            ui.vertical(|ui| {
                ui.add_space(label_gap(ui));
                let b = design::icon_button(ui, t, ic::ARROWS_LEFT_RIGHT, IconBtn::default());
                if design::tip(b, t, t!("herr.intercambiar_lados")).clicked() {
                    std::mem::swap(&mut self.left, &mut self.right);
                    run = true;
                }
            });
            run |= Self::path_field(ui, t, t!("herr.derecha"), &mut self.right, field_w);
            ui.vertical(|ui| {
                ui.add_space(label_gap(ui));
                if design::button(ui, t, compare_kind, ic::ARROW_CLOCKWISE, t!("herr.comparar"), ready).clicked() {
                    run = true;
                }
            });
        });
        if let Some(o) = &cx.other {
            ui.add_space(sp::S);
            if design::button(ui, t, Kind::Secondary, ic::SQUARE_SPLIT_HORIZONTAL, t!("herr.usar_carpeta_otro_panel_derecha"), true).clicked() {
                self.right = o.clone();
                run = true;
            }
        }
        if run && !self.left.trim().is_empty() && !self.right.trim().is_empty() {
            self.start(&ctx);
        }
        ui.add_space(sp::M);

        if let Some(j) = &self.job {
            design::busy(ui, t, &tf!("herr.comparando_elementos_revisados", n = fsx::thousands(j.scanned.load(Ordering::Relaxed))));
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            return;
        }
        if !self.waiting.is_empty() {
            design::busy(ui, t, t!("herr.sincronizando_progreso_esta_cola_copias"));
            return;
        }
        let items = match &mut self.items {
            None => {
                empty(ui, t, ic::ARROWS_LEFT_RIGHT, t!("herr.elige_dos_carpetas_compararlas"), &[t!("herr.escribe_pega_rutas_arrastralas_desde"), t!("herr.veras_cambio_cada_lado_podras")]);
                return;
            }
            Some(Err(e)) => {
                design::status_line(ui, t, design::Tone::Danger, "", e.as_str());
                return;
            }
            Some(Ok(items)) => items,
        };

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

        // Modo y resumen: [Sincronizar ⟨modo⟩ ☐ iguales] … [resumen →9 ←3 🗑2 (Sincronizar)].
        // Si no entra en una fila, el resumen y el botón bajan a la siguiente.
        let mut mode = self.mode;
        let mut sync = false;
        let summary = tf!("herr.diferencias_iguales", diferentes = diff, iguales = same);
        let (to_r_s, to_l_s, del_s) = (to_r.to_string(), to_l.to_string(), del.to_string());
        let chips: Vec<(Chip, &str)> = [(to_r, ic::ARROW_RIGHT, &to_r_s, t!("herr.copiar_derecha")), (to_l, ic::ARROW_LEFT, &to_l_s, t!("herr.copiar_izquierda"))]
            .into_iter()
            .filter(|c| c.0 > 0)
            .map(|(_, glyph, s, tip)| (Chip { text: s.as_str(), icon: glyph, mono: true, ..Default::default() }, tip))
            .chain((del > 0).then(|| (Chip { text: del_s.as_str(), icon: ic::TRASH, mono: true, ..Default::default() }, "")))
            .collect();
        let del_tip = tf!("herr.n_a_la_papelera", n = del);
        let sync_label = tf!("herr.sincronizar_tamano", tamano = fsx::human_size(bytes));
        let p = ui.painter().clone();
        let modes = [
            (Mode::Both, t!("herr.ambas_direcciones"), t!("herr.mas_nuevo_cada_lado_gana")),
            (Mode::ToRight, t!("herr.actualizar_derecha"), t!("herr.copia_derecha_nuevo_cambiado_no")),
            (Mode::Mirror, t!("herr.espejo_derecha"), t!("herr.derecha_queda_identica_sobra_va")),
        ];
        let left_w = design::text_width(&p, t!("herr.sincronizar"), text::small())
            + modes.iter().map(|(_, l, _)| design::text_width(&p, l, text::body()) + 2.0 * sp::M).sum::<f32>()
            + 2.0 * sp::XXS
            + design::text_width(&p, t!("herr.mostrar_iguales"), text::body())
            + icon::M
            + 4.0 * sp::S
            + sp::XS;
        let right_w = design::text_width(&p, &summary, text::mono_s()) + chips.iter().map(|(c, _)| design::chip_width(&p, c) + sp::S).sum::<f32>() + button_w(&p, Kind::Primary, ic::ARROWS_CLOCKWISE, &sync_label) + 2.0 * sp::S;
        let one_row = left_w + sp::L + right_w <= ui.available_width();
        let right = |ui: &mut Ui, sync: &mut bool| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = sp::S;
                if design::button(ui, t, Kind::Primary, ic::ARROWS_CLOCKWISE, &sync_label, to_r + to_l + del > 0).clicked() {
                    *sync = true;
                }
                for (c, tip) in chips.iter().rev() {
                    let r = design::chip(ui, t, *c).response;
                    design::tip(r, t, if tip.is_empty() { &del_tip } else { tip });
                }
                ui.label(RichText::new(&summary).font(text::mono_s()).color(t.dim));
            });
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = sp::S;
            ui.label(RichText::new(t!("herr.sincronizar")).font(text::small()).color(t.dim));
            widgets::segmented(ui, t, &mut mode, &modes);
            ui.add_space(sp::XS);
            design::checkbox(ui, t, &mut self.show_same, t!("herr.mostrar_iguales"));
            if one_row {
                right(ui, &mut sync);
            }
        });
        if !one_row {
            ui.add_space(sp::S);
            ui.horizontal(|ui| right(ui, &mut sync));
        }
        let mode_changed = mode != self.mode;
        self.mode = mode;
        ui.add_space(sp::S);

        // Lista: encabezado de columnas alineado con el texto de las filas.
        let visible: Vec<usize> = (0..items.len()).filter(|&k| self.show_same || items[k].status != Status::Same).collect();
        let row_h = h::ROW;
        let mut cycle: Option<usize> = None;
        let cols = CmpCols::new(ui.available_width());
        design::list_header(
            ui,
            t,
            &[
                ListCol { label: t!("herr.elemento"), x: cols.name_x, align: Align2::LEFT_CENTER },
                ListCol { label: &short_name(&self.left), x: cols.left_r, align: Align2::RIGHT_CENTER },
                ListCol { label: &short_name(&self.right), x: cols.w - PAD, align: Align2::RIGHT_CENTER },
            ],
        );
        if visible.is_empty() {
            empty(ui, t, ic::CHECK_CIRCLE, t!("herr.dos_carpetas_son_iguales"), &[]);
        }
        let (lroot, rroot) = (fsx::normalize(self.left.trim()), fsx::normalize(self.right.trim()));
        egui::ScrollArea::vertical().id_salt("compare-list").auto_shrink([false, false]).show_rows(ui, row_h, visible.len(), |ui, range| {
            for &k in &visible[range] {
                let it = &items[k];
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                let id = Id::new(("cmp-row", k));
                let resp = ui.interact(rect, id, Sense::hover());
                row_fill(ui, t, rect, id, resp.hovered(), Sel::None);
                let c = CmpCols::new(rect.width());
                let painter = ui.painter();
                let cy = rect.center().y;
                // El ícono real: del lado que lo tenga.
                let path = fsx::join(if it.left.is_some() { &lroot } else { &rroot }, &it.rel);
                file_icon(painter, t, pos2(rect.left() + PAD + icon::M / 2.0, cy), &path, &it.rel, it.is_dir);
                let g = elided(painter, &it.rel, text::body(), t.text, c.name_w);
                painter.galley(pos2(rect.left() + c.name_x, cy - g.size().y / 2.0), g, t.text);
                // El lado más nuevo en `text`, el otro en `dim`; lo que difiere sin saber cuál es
                // más nuevo, en `warn`; «—» si falta.
                let newer_l = matches!(it.status, Status::LeftNewer | Status::OnlyLeft);
                let newer_r = matches!(it.status, Status::RightNewer | Status::OnlyRight);
                let cell = |side: Option<(u64, u32)>, hl: bool| match side {
                    None => ("—".to_string(), t.faint),
                    Some((size, mtime)) => {
                        let s = if it.is_dir { fsx::fmt_date(mtime) } else { format!("{} · {}", fsx::human_size(size), fsx::fmt_date(mtime)) };
                        (s, if hl { t.text } else if it.status == Status::Different { t.warn } else { t.dim })
                    }
                };
                let (ls, lc) = cell(it.left, newer_l);
                let (rs, rc) = cell(it.right, newer_r);
                painter.text(pos2(rect.left() + c.left_r, cy), Align2::RIGHT_CENTER, ls, text::mono(), lc);
                painter.text(pos2(rect.right() - PAD, cy), Align2::RIGHT_CENTER, rs, text::mono(), rc);
                // Acción (clic para cambiarla): botón mini; solo «a la papelera» lleva color.
                let ar = Rect::from_center_size(pos2(rect.left() + c.act_x, cy), vec2(h::CHIP, h::CHIP));
                let aresp = design::icon_button_at(ui, t, ar, Id::new(("cmp-act", k)), "", IconBtn::mini());
                let (aicon, acol, tip) = match it.action {
                    Dir::ToRight => (ic::ARROW_RIGHT, t.text, t!("herr.copiar_derecha")),
                    Dir::ToLeft => (ic::ARROW_LEFT, t.text, t!("herr.copiar_izquierda")),
                    Dir::DeleteLeft => (ic::TRASH, t.danger, t!("herr.mandar_papelera_izquierda")),
                    Dir::DeleteRight => (ic::TRASH, t.danger, t!("herr.mandar_papelera_derecha")),
                    Dir::Skip => (if it.status == Status::Same { ic::EQUALS } else { ic::MINUS }, t.faint, t!("herr.no_hacer_nada")),
                };
                ui.painter().text(ar.center(), Align2::CENTER_CENTER, aicon, design::icon_font(icon::S), acol);
                if design::tip(aresp, t, &tf!("herr.clic_cambiar", accion = tip)).clicked() {
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
            let (l, r) = (lroot, rroot);
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

/// Columnas de comparar (M-15): ícono, elemento, lado izquierdo (200, a la derecha), acción (36)
/// y lado derecho (200, a la derecha), con 8 entre sí. Las x van desde el borde izquierdo.
struct CmpCols {
    w: f32,
    name_x: f32,
    name_w: f32,
    /// Borde derecho del lado izquierdo.
    left_r: f32,
    /// Centro del botón de acción.
    act_x: f32,
}

impl CmpCols {
    fn new(w: f32) -> Self {
        // Tamaño y fecha en mono: «123,4 MB · 2026-10-28 01:58» mide unos 190 px.
        let (side_w, act_w) = (200.0, 36.0);
        let right_l = w - PAD - side_w;
        let act_x = right_l - sp::S - act_w / 2.0;
        let left_r = act_x - act_w / 2.0 - sp::S;
        let name_x = PAD + icon::M + sp::S;
        CmpCols { w, name_x, name_w: (left_r - side_w - sp::S - name_x).max(40.0), left_r, act_x }
    }
}
