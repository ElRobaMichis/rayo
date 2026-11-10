//! Lo que abre la captura de pantalla (`RAYO_CAPTURA_ABRIR`, ver `capture.rs` y
//! `scripts/captura.ps1`): cada paso deja a la vista una parte de la interfaz tal como la vería
//! alguien que la abrió a mano. Solo corre mientras se captura.
//!
//! La carpeta de ejemplo (`Imágenes\Japón 2026`, con Kioto, Osaka, Hakone, fotos, notas…) la
//! arma el script; su ruta llega en `RAYO_CAPTURA_CARPETA`.

use super::*;
use crate::ui::capture;

impl App {
    /// Carpeta de ejemplo (la del script) o, si no se dijo, la de la pestaña actual.
    fn cap_folder(&self) -> String {
        std::env::var("RAYO_CAPTURA_CARPETA").ok().filter(|v| !v.is_empty()).map(|p| fsx::normalize(&p)).unwrap_or_else(|| self.tabs[self.cur].path().to_string())
    }

    fn cap_target(&self, paths: Vec<String>) -> crate::ui::commands::Target {
        let dir = self.tabs[self.cur].active_col().path.clone();
        let single_dir = paths.len() == 1 && std::path::Path::new(&fsx::long_path(&paths[0])).is_dir();
        crate::ui::commands::Target { paths, dir, single_dir, split: self.cfg.split }
    }

    /// Deja seleccionados estos nombres en la última columna de la pestaña actual.
    fn cap_select(&mut self, ctx: &egui::Context, names: &[&str]) {
        let col = self.tabs[self.cur].cols.len() - 1;
        self.apply(ctx, Action::SetSelection { col, names: names.iter().map(|s| s.to_string()).collect() });
        let tab = self.tab();
        if let Some(first) = names.first() {
            let c = tab.cols.last_mut().unwrap();
            c.cursor = Some(first.to_string());
            c.scroll_to_cursor = true;
        }
    }

    /// Un paso de la captura. Los que necesitan esperar a que algo se abra se dejan para después
    /// con `capture::later` (con un nombre que empieza con `#`).
    pub(super) fn capture_step(&mut self, ctx: &egui::Context, step: &str) {
        crate::diag::log(&format!("captura: paso «{step}»"));
        let folder = self.cap_folder();
        let at = |name: &str| fsx::join(&folder, name);
        match step {
            "paleta" => self.palette.show(),
            "borrar" => self.confirm_delete = Some(self.tabs[self.cur].selected_paths()),
            // ---- diálogos
            "hash" => {
                let t = self.cap_target(vec![at("torii-fushimi.jpg")]);
                self.run_command(ctx, "hash", t);
            }
            "renombrar" => {
                self.cap_select(ctx, &["arashiyama-bambu.jpg", "itinerario.md", "torii-fushimi.jpg"]);
                let t = self.cap_target(vec![at("arashiyama-bambu.jpg"), at("itinerario.md"), at("torii-fushimi.jpg")]);
                self.run_command(ctx, "batch_rename", t);
                capture::later(ctx, 0.3, "#renombrar-escribir");
            }
            "#renombrar-escribir" => {
                // Se escribe en la plantilla (enfocada al abrir): «{nombre}» → «{nombre}-{n}».
                capture::keys(&[Key::End]);
                capture::type_text(&format!("-{}", crate::fsx::rename::token("n")));
            }
            "bloqueos" => {
                // El script deja `notas.txt` abierto desde PowerShell: aparece quién lo usa.
                let p = at("notas.txt");
                let retry = (t!("app.volver_intentar_papelera").to_string(), Action::Delete(vec![p.clone()], false));
                self.locks = Some(crate::ui::tools::LocksDialog::new(vec![p], Some(t!("app.no_pudo_borrar").into()), Some(retry), ctx));
            }
            "conexion" => {
                self.apply(ctx, Action::EditConnection(None));
                if let Some(f) = self.conn_form.as_mut() {
                    f.1 = fsx::remote::Connection { name: "NAS de casa".into(), kind: fsx::remote::Kind::Smb, host: "nas.local".into(), port: 0, path: "fotos\\2026".into(), user: "usuario".into(), secure: true };
                }
            }
            "barra" => self.toolbar_dlg = true,
            "patron" => self.pattern = Some(("*.jpg".into(), false, true)),
            "idioma" => self.lang_dlg = Some(crate::ui::language::LangDialog::new()),
            "predeterminado" => self.default_dlg = Some(true),
            "admin" => {
                let dir = "C:\\Program Files".to_string();
                self.admin = Some((t!("app.windows_pide_permiso_administrador_crear").into(), crate::admin::AdminOp::NewFolder { dir }));
            }
            "primer-uso" => self.first_run = true,
            // ---- copias
            "conflicto" => {
                // Kioto y Osaka tienen las mismas `foto-N.txt`: copiar una pide qué hacer.
                self.transfer(ctx, vec![fsx::join(&at("Kioto"), "foto-1.txt")], at("Osaka"), false);
            }
            "copias" => {
                // El script deja un archivo grande (`video-hakone.mp4`): su copia sigue en curso al
                // capturar; detrás, en la cola, otra.
                self.transfer(ctx, vec![at("video-hakone.mp4")], at("Hakone"), false);
                self.transfer(ctx, vec![at("Kioto")], at("Hakone"), false);
                self.show_transfers = true;
                // El aviso «Copiando…» quedaría encima de la lista.
                self.toast = None;
            }
            // ---- herramientas en una pestaña
            "duplicados" => {
                let t = self.cap_target(Vec::new());
                let t = crate::ui::commands::Target { dir: folder.clone(), ..t };
                self.run_command(ctx, "dupes_here", t);
            }
            "comparar" => {
                let tool = crate::ui::tools::Tool::Compare(crate::ui::tools::CompareTool::new(at("Kioto"), at("Osaka"), ctx));
                self.open_tool(ctx, at("Kioto"), tool);
            }
            "papelera" => self.apply(ctx, Action::OpenRecycle),
            "terminal" => {
                self.apply(ctx, Action::ToggleTerminal);
                capture::later(ctx, 0.8, "#terminal-escribir");
            }
            "#terminal-escribir" => {
                // Con un repositorio (paso `git`), su estado; si no, la lista de la carpeta.
                let git = std::path::Path::new(&fsx::long_path(&at(".git"))).is_dir();
                capture::type_text(if git { "git status --short" } else { "Get-ChildItem -Name" });
                capture::keys(&[Key::Enter]);
            }
            "git" => self.cap_select(ctx, &["itinerario.md"]),
            // ---- paneles y pestañas
            "cuatro" => {
                self.apply(ctx, Action::ToggleQuad);
                for (p, sub) in [(1, "Kioto"), (2, "Osaka"), (3, "Hakone")] {
                    let ti = self.pane[p];
                    self.tabs[ti].navigate(&at(sub), false);
                }
            }
            "dividida" => {
                self.apply(ctx, Action::ToggleSplit);
                let ti = self.pane[1];
                self.tabs[ti].navigate(&at("Kioto"), false);
                self.set_focus(0);
            }
            "grupos" => {
                use crate::ui::config::TabGroup;
                self.cfg.tab_groups = vec![
                    TabGroup { id: 1, name: "Viaje".into(), color: 4, collapsed: false },
                    TabGroup { id: 2, name: "Fotos".into(), color: 3, collapsed: false },
                    TabGroup { id: 3, name: "Trabajo".into(), color: 1, collapsed: true },
                ];
                let start = self.cur;
                for (sub, g) in [("Kioto", 1), ("Osaka", 1), ("Hakone", 2), ("Kioto", 3), ("Osaka", 3)] {
                    self.apply(ctx, Action::NewTab(Some(at(sub))));
                    self.tabs[self.cur].group = g;
                }
                self.tabs[start].group = 1;
                self.apply(ctx, Action::SwitchTab(start));
            }
            // ---- barra lateral
            "etiquetas" => {
                fsx::tags::toggle(&[at("torii-fushimi.jpg")], 0);
                fsx::tags::toggle(&[at("arashiyama-bambu.jpg")], 4);
                fsx::tags::toggle(&[at("itinerario.md")], 3);
                fsx::tags::toggle(&[at("itinerario.md")], 1);
                fsx::tags::toggle(&[at("Kioto")], 2);
                self.cfg.tags_collapsed = false;
                self.refresh_tag_search();
            }
            "colecciones" => {
                let name = "Para imprimir";
                fsx::collections::add(name, &[at("torii-fushimi.jpg"), at("arashiyama-bambu.jpg"), at("itinerario.md"), at("presupuesto.csv")]);
                fsx::collections::add("Reservas", &[at("reservas-hoteles.pdf")]);
                self.cfg.collections_collapsed = false;
                self.apply(ctx, Action::OpenCollection(name.into()));
            }
            "repisa" => self.apply(ctx, Action::ShelfAdd(vec![at("torii-fushimi.jpg"), at("presupuesto.csv"), at("Kioto"), at("itinerario.md")])),
            // ---- vistas de la lista
            "expandir" => {
                self.apply(ctx, Action::SetView(ViewMode::Details));
                self.apply(ctx, Action::ToggleExpand("Kioto".into()));
                self.cap_select(ctx, &["Kioto"]);
            }
            "plana" => self.apply(ctx, Action::ToggleFlat),
            "buscar" => {
                let s = &mut self.tabs[self.cur].search;
                s.text = "foto".into();
                s.here = true;
            }
            "renombrar-inline" => {
                self.cap_select(ctx, &["notas.txt"]);
                // A medio escribir, cambiando la extensión: se ve el aviso al lado.
                self.rename = Some(Rename { dir: folder.clone(), old: "notas.txt".into(), buf: "notas.md".into(), first_frame: true });
            }
            "nueva-carpeta" => self.apply(ctx, Action::NewFolder(folder.clone())),
            "descarga" => self.cap_select(ctx, &["mapa-kioto.pdf.crdownload"]),
            // ---- vista previa
            "vistarapida" => self.quicklook = true,
            "markdown" => self.cap_select(ctx, &["itinerario.md"]),
            "pdf" => self.cap_select(ctx, &["reservas-hoteles.pdf"]),
            "red" => {
                let p = std::env::var("RAYO_CAPTURA_RED").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "\\\\localhost\\C$\\Windows".into());
                self.apply(ctx, Action::Navigate(p));
            }
            "toast" => self.toast_ok(ctx, tp!(2, "app.copiado_n")),
            // ---- menús
            "menu" => {
                // El menú de clic derecho sobre la foto (dentro de la ventana: ver `inline_menus`).
                self.cap_select(ctx, &["torii-fushimi.jpg"]);
                let req = crate::ui::ctxmenu::MenuReq::Items { paths: vec![at("torii-fushimi.jpg")], is_dir: false, dir: folder.clone(), reveal: None };
                self.apply(ctx, Action::OpenMenu(req));
                if let Some(m) = self.menu.as_mut() {
                    // Junto a la fila de la foto (en píxeles de la pantalla, como un clic de verdad).
                    let inner = ctx.input(|i| i.viewport().inner_rect).unwrap_or(egui::Rect::ZERO);
                    let ppp = ctx.pixels_per_point();
                    let p = inner.min + vec2(560.0, 250.0);
                    m.anchor = ((p.x * ppp) as i32, (p.y * ppp) as i32);
                }
            }
            "menu-barra" => {
                // Clic derecho sobre la pestaña.
                let id = self.tabs[self.cur].id;
                capture::click_on(ctx, Id::new(("tab", id)), egui::PointerButton::Secondary);
            }
            "menu-favoritos" => {
                capture::click_on(ctx, Id::new(("gmore", 0usize)), egui::PointerButton::Primary);
            }
            "historial" => {
                for q in ["torii", "*.jpg", "presupuesto", "reservas hotel"] {
                    if !self.cfg.search_history.iter().any(|x| x == q) {
                        self.cfg.search_history.insert(0, q.into());
                    }
                }
                self.tabs[self.cur].search.focus = true;
            }
            "sugerencias" => {
                let tab = self.tab();
                tab.path_edit = Some(tab.path().to_string());
                capture::later(ctx, 0.3, "#sugerencias-escribir");
            }
            "#sugerencias-escribir" => {
                // Al abrirse, la ruta queda toda seleccionada: se escribe encima, terminando en «\».
                capture::type_text(&format!("{folder}\\"));
            }
            "migas" => {
                // El ▸ del último tramo de la ruta: sus subcarpetas.
                let id = self.tabs[self.cur].id;
                if let Some(i) = (0..64usize).rev().find(|&i| ctx.read_response(Id::new(("crumb-sep", id, i))).is_some()) {
                    capture::click_on(ctx, Id::new(("crumb-sep", id, i)), egui::PointerButton::Primary);
                }
            }
            other => crate::diag::log(&format!("captura: paso desconocido «{other}»")),
        }
    }
}
