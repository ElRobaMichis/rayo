//! La aplicación: une estado, vistas y acciones.

use super::actions::Action;
use super::chrome;
use super::config::{Config, FavGroup, ViewMode};
use super::preview::{self, PreviewItem, Textures};
use super::state::*;
use super::theme::{self, Theme};
use super::views::{self, ColumnData, Cx, Rename};
use crate::fsx::shell::{self, FileOp};
use crate::fsx::transfer::{self, Transfers};
use crate::fsx::{self, Drive, FileEntry, Place};
use crate::index::{IndexManager, ntfs_fixed_volumes, search::now_unix};
use egui::{Frame, Id, Key, Modifiers, Panel, Sense, Ui, vec2};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::HWND;


/// Nombre sin extensión.
fn stem(p: &str) -> String {
    let n = fsx::file_name(p);
    match n.rfind('.') {
        Some(i) if i > 0 => n[..i].to_string(),
        _ => n,
    }
}

/// "x.zip" → "x (2).zip" si ya existe.
fn shell_free_name(p: &str) -> String {
    if std::path::Path::new(&fsx::long_path(p)).exists() { transfer::free_name(p) } else { p.to_string() }
}

/// Algo que se puede deshacer con Ctrl+Z.
enum Undo {
    Rename { dir: String, old: String, new: String },
    /// Renombrado en lote: (viejo, nuevo).
    Batch { dir: String, pairs: Vec<(String, String)> },
    Recycle(Vec<String>),
    /// Copias creadas (se mandan a la papelera).
    Copied(Vec<String>),
    /// Movimientos: (dónde estaba, dónde quedó).
    Moved(Vec<(String, String)>),
    NewFolder(String),
}

pub struct App {
    t: Theme,
    cfg: Config,
    mgr: Arc<IndexManager>,
    dirs: DirCache,
    tabs: Vec<Tab>,
    cur: usize,
    next_id: u64,
    search: SearchWorker,
    tex: Textures,
    drives: Arc<Mutex<Vec<Drive>>>,
    places: Vec<Place>,
    quicklook: bool,
    rename: Option<Rename>,
    group_edit: Option<(usize, String)>,
    toast: Option<(String, f64)>,
    hwnd: Option<HWND>,
    watched_sig: u64,
    epoch_seen: u64,
    cut: Vec<String>,
    text_cache: Option<(String, Option<String>)>,
    paste_latch: bool,
    /// Menú nativo pendiente: se muestra un cuadro después, cuando el menú propio ya se cerró.
    pending_menu: Option<(Vec<String>, (i32, i32), u8)>,
    /// Monitor de rendimiento (F12): tiempos de los últimos cuadros.
    perf: Option<std::collections::VecDeque<(f64, f32)>>,
    /// Resultado de instalar o quitar el servicio (llega desde otro hilo).
    service_result: Arc<Mutex<Option<String>>>,
    /// Carpetas recargadas para confirmar que algo desapareció: ruta → generación.
    prune_check: std::collections::HashMap<String, u64>,
    /// Pestaña actual de cada panel (índices en `tabs`). Siempre `cur == pane[focus]`.
    pane: [usize; 2],
    focus: usize,
    /// Dónde se dibujó cada panel en el cuadro anterior (para enfocarlo con un clic).
    pane_rects: [egui::Rect; 2],
    /// Cola de copias propia.
    xfer: Arc<Transfers>,
    show_transfers: bool,
    palette: super::commands::Palette,
    /// Herramientas abiertas: renombrado masivo (diálogo), duplicados y comparación (en una pestaña).
    batch: Option<super::tools::BatchRename>,
    undo: Vec<Undo>,
    /// Copias o movimientos en curso que se podrán deshacer al terminar: (trabajo, es mover).
    awaiting: Vec<(u64, bool)>,
    /// Archivos arrastrados fuera de la ventana: se entregan al arrastre de Windows al final del cuadro.
    drag_out: Option<Vec<String>>,
    /// Copias temporales de archivos de adentro de comprimidos (para la vista previa):
    /// ruta → None mientras se extrae, Some(resultado) al terminar.
    extracted: Arc<Mutex<std::collections::HashMap<String, Option<Option<String>>>>>,
    /// Visores de vista previa de Windows (Word, Excel…) en el panel.
    prevhost: Option<super::prevhost::PreviewHost>,
    /// "¿Quién lo está usando?"
    locks: Option<super::tools::LocksDialog>,
    /// Lo que quedó sin borrar porque otro programa lo tiene abierto: (rutas, definitivo).
    lock_report: Arc<Mutex<Option<(Vec<String>, bool)>>>,
    /// Carpetas abiertas en alguna pestaña que se están moviendo: (de, a, hasta cuándo esperar).
    /// Al terminar, la pestaña las sigue a su lugar nuevo.
    follow_moves: Vec<(String, String, f64)>,
    /// Diálogo para hacer (o dejar de hacer) de Rayo el explorador predeterminado:
    /// Some(iniciar con Windows) mientras está abierto.
    default_dlg: Option<bool>,
    /// El diálogo de idioma, abierto.
    lang_dlg: Option<super::language::LangDialog>,
    /// Espacio libre de la fila de arriba en este cuadro (hace de barra de título).
    caption: Vec<egui::Rect>,
    /// Primera vez sin instalar: preguntar si instalar o usar sin instalar.
    first_run: bool,
    /// "Continuar como administrador": qué no se pudo hacer y por qué.
    admin: Option<(String, crate::admin::AdminOp)>,
    /// Lo que quedó sin hacer por permisos en otro hilo (p. ej. al borrar).
    admin_report: Arc<Mutex<Option<crate::admin::AdminOp>>>,
    /// El menú de Windows (opciones de otras apps) para el clic derecho.
    shell_menus: Option<fsx::ctxmenu::ShellMenus>,
    /// El menú de clic derecho abierto (en sus propias ventanas).
    menu: Option<MenuWin>,
    /// Copias / movimientos en curso, por si fallan por permisos: (trabajo, origen, destino, mover).
    admin_watch: Vec<(u64, Vec<String>, String, bool)>,
    /// Página del PDF en la vista rápida (y de qué archivo).
    ql_page: (String, u32),
    /// Diálogo "Seleccionar por patrón": (patrón, sumar a la selección, primer cuadro).
    pattern: Option<(String, bool, bool)>,
    hash: Option<super::tools::HashDialog>,
}

fn hwnd_of(h: &impl raw_window_handle::HasWindowHandle) -> Option<HWND> {
    match h.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(w) => Some(HWND(w.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// Menús nativos de Windows en tema oscuro/claro (uxtheme, ordinales 135 y 136).
fn native_menu_theme(dark: bool) {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{PCSTR, w};
    unsafe {
        let Ok(lib) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(set_mode) = GetProcAddress(lib, PCSTR(135 as *const u8)) {
            let f: extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            f(if dark { 2 } else { 3 });
        }
        if let Some(flush) = GetProcAddress(lib, PCSTR(136 as *const u8)) {
            let f: extern "system" fn() = std::mem::transmute(flush);
            f();
        }
    }
}

fn dark_title_bar(hwnd: HWND, dark: bool) {
    use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let v: i32 = dark as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const _ as *const _, 4);
    }
}

fn spawn_drive_watcher(drives: Arc<Mutex<Vec<Drive>>>, wake: Wake) {
    std::thread::Builder::new()
        .name("unidades".into())
        .spawn(move || {
            let mut mask = 0u32;
            let mut last_full = std::time::Instant::now();
            loop {
                let m = fsx::drive_mask();
                if m != mask || last_full.elapsed().as_secs() > 30 {
                    mask = m;
                    last_full = std::time::Instant::now();
                    let list = fsx::list_drives();
                    let changed = {
                        let cur = drives.lock();
                        cur.len() != list.len()
                            || cur.iter().zip(&list).any(|(a, b)| a.letter != b.letter || a.label != b.label || a.free / 100_000_000 != b.free / 100_000_000)
                    };
                    if changed {
                        *drives.lock() = list;
                        wake();
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
        })
        .ok();
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        crate::index::mem_log("app: inicio");
        if std::env::var_os("RAYO_DEBUG").is_some() {
            std::thread::spawn(|| {
                for i in 0..16 {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    crate::index::mem_log(&format!("t+{:.1}s", (i + 1) as f32 * 0.5));
                }
            });
        }
        let cfg = Config::load();
        crate::i18n::init(cfg.lang.as_deref());
        // Explorador predeterminado: corregir la ruta si Rayo se movió y vigilar el Explorador.
        // (RAYO_PRUEBA_VIGIA=1 prende solo el vigía, sin tocar el registro: para probarlo.)
        let owns = crate::install::owns_registrations();
        if cfg.default_explorer && owns {
            std::thread::spawn(|| crate::integration::refresh(&crate::integration::REAL));
        }
        // Inicio con Windows apuntando a este Rayo (si se movió o se instaló).
        if cfg.start_with_windows && owns {
            let _ = crate::instance::set_autostart(true);
        }
        let cfg_portable = cfg.portable;
        if cfg.default_explorer || std::env::var_os("RAYO_PRUEBA_VIGIA").is_some() {
            crate::integration::watch(true);
        }
        let t = Theme::get(cfg.dark);
        theme::setup_fonts(&ctx);
        theme::apply_style(&ctx, &t);
        // El doble clic dura lo que diga Windows (egui usa 0,3 s; Windows, 0,5 s por defecto).
        let dbl = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() } as f64 / 1000.0;
        ctx.options_mut(|o| o.input_options.max_double_click_delay = dbl.clamp(0.2, 1.0));
        let hwnd = hwnd_of(cc);
        if let Some(h) = hwnd {
            dark_title_bar(h, cfg.dark);
        }
        native_menu_theme(cfg.dark);
        let wake: Wake = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        if let Some(h) = hwnd {
            let w = wake.clone();
            crate::instance::listen(h, move || w());
            let w = wake.clone();
            super::win::install(h, move || w(), [t.bg.r(), t.bg.g(), t.bg.b()]);
            super::win::set_icons(h);
        }
        // Los primeros instantes se repinta seguido (ver `win`).
        super::win::burst(800);
        let prevhost = hwnd.map(|h| {
            let host = super::prevhost::PreviewHost::new(h, wake.clone());
            host.set_background([t.sidebar.r(), t.sidebar.g(), t.sidebar.b()]);
            host
        });
        let mgr = IndexManager::new(&ntfs_fixed_volumes());
        *mgr.wake.write() = Some(wake.clone());
        mgr.start();
        let dirs = DirCache::new(mgr.clone(), wake.clone());
        let search = SearchWorker::new(mgr.clone(), wake.clone());
        let tex = Textures::new(fsx::thumbs::ThumbLoader::new(wake.clone()));
        let xfer = {
            let mgr = mgr.clone();
            Arc::new(Transfers::new(
                wake.clone(),
                Arc::new(move |dirs: Vec<String>| {
                    // Recargar las carpetas que cambiaron (también si no están indexadas).
                    mgr.dirty.lock().extend(dirs.iter().map(|d| d.to_lowercase()));
                    mgr.wake_ui();
                }),
            ))
        };
        std::thread::spawn(fsx::archive::clean_temp);
        fsx::shellnew::preload();
        let drives = Arc::new(Mutex::new(fsx::list_drives()));
        let shell_menus = hwnd.map(|h| fsx::ctxmenu::ShellMenus::new(h, wake.clone()));
        spawn_drive_watcher(drives.clone(), wake);
        let places = fsx::known_places();
        let home = places.first().map(|p| p.path.clone()).unwrap_or_else(|| "C:\\".into());
        // Pestañas de cada panel (si no hay guardadas, una en Inicio).
        let mut tabs: Vec<Tab> = Vec::new();
        let mut next_id = 1;
        let mut pane = [0usize; 2];
        for (p, (list, active)) in [(&cfg.tabs, cfg.active_tab), (&cfg.tabs_right, cfg.active_right)].into_iter().enumerate() {
            let first = tabs.len();
            for path in list {
                if std::path::Path::new(path).is_dir() {
                    let mut t = Tab::new(next_id, path);
                    t.pane = p as u8;
                    tabs.push(t);
                    next_id += 1;
                }
            }
            if tabs.len() == first {
                let mut t = Tab::new(next_id, &home);
                t.pane = p as u8;
                tabs.push(t);
                next_id += 1;
            }
            pane[p] = first + active.min(tabs.len() - first - 1);
        }
        let mut cur = pane[0];
        // Si se abrió con una ruta como argumento: abrir esa carpeta (o mostrar ese archivo) en una pestaña nueva.
        if let Some(p) = crate::instance::path_from_args() {
            let mut t = Tab::new(next_id, &p);
            if !std::path::Path::new(&p).is_dir() && fsx::archive::split(&p).is_none() {
                t.reveal(&p);
            }
            tabs.insert(pane[0] + 1, t);
            next_id += 1;
            cur = pane[0] + 1;
            if pane[1] >= cur {
                pane[1] += 1;
            }
            pane[0] = cur;
        }
        let mut app = App {
            t,
            cfg,
            mgr,
            dirs,
            tabs,
            cur,
            next_id,
            search,
            tex,
            drives,
            places,
            quicklook: false,
            rename: None,
            group_edit: None,
            toast: None,
            hwnd,
            watched_sig: 0,
            epoch_seen: 0,
            cut: Vec::new(),
            text_cache: None,
            paste_latch: false,
            pending_menu: None,
            perf: None,
            prune_check: Default::default(),
            service_result: Default::default(),
            pane,
            focus: 0,
            pane_rects: [egui::Rect::NOTHING; 2],
            xfer,
            show_transfers: false,
            palette: Default::default(),
            batch: None,
            undo: Vec::new(),
            awaiting: Vec::new(),
            drag_out: None,
            extracted: Default::default(),
            prevhost,
            locks: None,
            lock_report: Default::default(),
            follow_moves: Vec::new(),
            default_dlg: None,
            lang_dlg: None,
            caption: Vec::new(),
            first_run: !crate::install::is_installed() && !crate::install::is_installed_copy() && !cfg_portable,
            admin: None,
            admin_report: Default::default(),
            admin_watch: Vec::new(),
            shell_menus,
            menu: None,
            ql_page: (String::new(), 0),
            pattern: None,
            hash: None,
        };
        app.warm_start();
        app
    }

    fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.cur]
    }

    fn set_focus(&mut self, p: usize) {
        let p = if self.cfg.split { p.min(1) } else { 0 };
        self.focus = p;
        self.cur = self.pane[p];
    }

    /// Índices de las pestañas de un panel, en orden.
    fn tabs_of(&self, p: usize) -> Vec<usize> {
        (0..self.tabs.len()).filter(|&i| self.tabs[i].pane as usize == p).collect()
    }

    /// Carpeta del panel de enfrente (solo con la vista dividida).
    fn other_path(&self) -> Option<String> {
        self.cfg.split.then(|| self.tabs[self.pane[1 - self.focus]].path().to_string())
    }

    /// Contexto actual para los comandos: selección del panel enfocado y su carpeta.
    fn target(&self) -> super::commands::Target {
        let tab = &self.tabs[self.cur];
        let paths = if tab.tool.is_some() {
            Vec::new()
        } else if tab.search.active() {
            tab.search.results.as_ref().and_then(|r| r.row(tab.search.cursor)).map(|r| vec![r.path()]).unwrap_or_default()
        } else {
            tab.selected_paths()
        };
        let single_dir = paths.len() == 1 && std::path::Path::new(&fsx::long_path(&paths[0])).is_dir();
        super::commands::Target { paths, dir: tab.active_col().path.clone(), single_dir, split: self.cfg.split }
    }

    /// Pone en la cola una copia o un movimiento, sin pedir nada si todo es obvio.
    fn transfer(&mut self, ctx: &egui::Context, paths: Vec<String>, dest: String, mv: bool) {
        // Soltar algo en la carpeta donde ya está no hace nada.
        let paths: Vec<String> = paths.into_iter().filter(|p| !fsx::parent_of(p).is_some_and(|d| d.eq_ignore_ascii_case(&dest))).collect();
        if paths.is_empty() {
            return;
        }
        let n = paths.len();
        if paths.iter().any(|p| fsx::archive::is_inside(p)) {
            // Sacar de un comprimido es extraer (el comprimido no se modifica).
            let id = self.xfer.enqueue_extract(paths, Vec::new(), dest.clone());
            self.awaiting.push((id, false));
            self.toast(ctx, tp!(n, "app.extrayendo_a", destino = fsx::file_name(&dest)));
            return;
        }
        let id = self.xfer.enqueue(if mv { transfer::Op::Move } else { transfer::Op::Copy }, paths.clone(), dest.clone());
        self.awaiting.push((id, mv));
        self.admin_watch.push((id, paths, dest.clone(), mv));
        self.dirs.reload(&dest);
        self.toast(ctx, if mv { tp!(n, "app.moviendo_a", destino = fsx::file_name(&dest)) } else { tp!(n, "app.copiando_a", destino = fsx::file_name(&dest)) });
    }

    fn toast(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ctx.input(|i| i.time)));
        ctx.request_repaint_after(std::time::Duration::from_millis(2600));
    }

    fn save_cfg(&mut self) {
        for p in 0..2 {
            // Las pestañas de herramientas no se guardan (la papelera sí: se guarda su carpeta).
            let idx: Vec<usize> = self
                .tabs_of(p)
                .into_iter()
                .filter(|&i| matches!(self.tabs[i].tool.as_deref(), None | Some(super::tools::Tool::Recycle(_))))
                .collect();
            let paths: Vec<String> = idx.iter().map(|&i| self.tabs[i].path().to_string()).collect();
            let active = idx.iter().position(|&i| i == self.pane[p]).unwrap_or(0);
            if p == 0 {
                self.cfg.tabs = paths;
                self.cfg.active_tab = active;
            } else {
                self.cfg.tabs_right = paths;
                self.cfg.active_right = active;
            }
        }
        self.cfg.save();
    }

    fn view_of(&mut self, path: &str) -> Arc<Vec<FileEntry>> {
        // En vista plana, la carpeta de la pestaña se lista con todas sus subcarpetas.
        let tab = &self.tabs[self.cur];
        let key = if tab.flat && path == tab.path() { format!("{}{path}", fsx::FLAT_KEY) } else { path.to_string() };
        self.dirs.view(&key, self.cfg.sort, self.cfg.sort_desc, self.cfg.show_hidden)
    }

    /// Lista visible de la columna activa (con el filtro escrito aplicado).
    fn active_list(&mut self) -> (usize, String, Arc<Vec<FileEntry>>) {
        let (ci, path, filter) = {
            let tab = &self.tabs[self.cur];
            (tab.active.min(tab.cols.len() - 1), tab.active_col().path.clone(), crate::index::store::search_key(tab.filter.trim()))
        };
        let list = self.view_of(&path);
        if filter.is_empty() {
            return (ci, path, list);
        }
        let filtered = list.iter().filter(|e| crate::index::store::search_key(&e.name).contains(&filter)).cloned().collect();
        (ci, path, Arc::new(filtered))
    }

    fn entry_in(&mut self, dir: &str, name: &str) -> Option<FileEntry> {
        self.view_of(dir).iter().find(|e| e.name == name).cloned()
    }

    // ------------------------------------------------------------ sondeos

    fn poll(&mut self, ctx: &egui::Context) {
        self.dirs.poll();
        self.tex.poll(ctx);
        while let Ok(r) = self.search.rx.try_recv() {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == r.tab) {
                if r.seq == tab.search.seq {
                    let reset = tab.search.results.as_ref().is_none_or(|old| old.query != r.results.query);
                    tab.search.results = Some(r.results);
                    if reset {
                        tab.search.cursor = 0;
                        tab.search.scroll_to_cursor = true;
                    }
                }
            }
        }
        // Carpetas visibles que cambiaron en disco.
        let dirty: Vec<String> = std::mem::take(&mut *self.mgr.dirty.lock());
        if !dirty.is_empty() {
            let inside = |k: &str| {
                let k = k.to_lowercase();
                match k.strip_prefix(fsx::FLAT_KEY) {
                    Some(root) => dirty.iter().any(|d| d.starts_with(root)),
                    None => dirty.contains(&k),
                }
            };
            let keys: Vec<String> = self.dirs.map.keys().filter(|k| inside(k)).cloned().collect();
            for k in keys {
                self.dirs.reload(&k);
            }
        }
        // Índice reemplazado: actualizar tamaños de carpetas y búsquedas.
        let epoch: u64 = self.mgr.slots.iter().map(|s| s.epoch.load(Ordering::Relaxed)).sum();
        if epoch != self.epoch_seen {
            self.epoch_seen = epoch;
            self.dirs.reload_all();
            for t in &mut self.tabs {
                t.search.sent = None;
            }
        }
    }

    /// Si algo seleccionado desapareció (borrado, movido), limpia la selección y
    /// cierra las columnas que dependían de ello.
    fn prune_missing(&mut self) {
        let tab = &mut self.tabs[self.cur];
        let mut reload = Vec::new();
        let last = tab.cols.len() - 1;
        for i in 0..tab.cols.len() {
            // En vista plana, la última columna se compara con la lista plana.
            let path = if tab.flat && i == last { format!("{}{}", fsx::FLAT_KEY, tab.cols[i].path) } else { tab.cols[i].path.clone() };
            let Some(st) = self.dirs.map.get(&path) else { continue };
            let Some(raw) = st.raw.as_ref().filter(|_| !st.loading) else { continue };
            let generation = st.generation;
            let col = &mut tab.cols[i];
            col.multi.retain(|n| raw.iter().any(|e| &e.name == n));
            let Some(c) = col.cursor.clone() else { continue };
            // Una carpeta que se está moviendo: la pestaña la va a seguir, no cerrarla.
            let moving = fsx::join(&col.path, &c);
            if self.follow_moves.iter().any(|(from, ..)| from.eq_ignore_ascii_case(&moving)) {
                continue;
            }
            if raw.iter().any(|e| e.name == c) {
                self.prune_check.remove(&path);
                continue;
            }
            // Falta: primero recargar; si tras la recarga sigue faltando, limpiar.
            match self.prune_check.get(&path) {
                Some(&g) if generation > g => {
                    self.prune_check.remove(&path);
                    col.cursor = None;
                    tab.cols.truncate(i + 1);
                    tab.active = tab.active.min(i);
                    break;
                }
                Some(_) => {}
                None => {
                    self.prune_check.insert(path.clone(), generation);
                    reload.push(path);
                }
            }
        }
        for p in reload {
            self.dirs.reload(&p);
        }
    }

    fn dispatch_search(&mut self) {
        let tab = &mut self.tabs[self.cur];
        if !tab.search.active() {
            tab.search.results = None;
            tab.search.sent = None;
            return;
        }
        let text = tab.search.text.trim().to_string();
        // Al cambiar el texto se vuelve a mostrar primero lo mejor (rápido).
        if tab.search.sent.as_ref().is_some_and(|k| k.0 != text) {
            tab.search.want_full = false;
        }
        let opts = tab.search.opts();
        let key = (text, tab.search.here, tab.path().to_string(), opts.sort, opts.desc, opts.limit.is_none());
        if tab.search.sent.as_ref() != Some(&key) {
            tab.search.seq += 1;
            let within = if key.1 { Some(key.2.clone()) } else { None };
            let _ = self.search.tx.send(SearchReq { tab: tab.id, seq: tab.search.seq, text: key.0.clone(), within, opts });
            tab.search.sent = Some(key);
        }
    }

    /// Avisa al índice qué carpetas se están mostrando.
    fn update_watched(&mut self) {
        use std::hash::{Hash, Hasher};
        let visible: Vec<usize> = if self.cfg.split { vec![self.pane[0], self.pane[1]] } else { vec![self.cur] };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for &i in &visible {
            for c in &self.tabs[i].cols {
                c.path.hash(&mut h);
            }
        }
        let sig = h.finish();
        if sig != self.watched_sig {
            self.watched_sig = sig;
            let mut w = self.mgr.watched.write();
            w.clear();
            for &i in &visible {
                for c in &self.tabs[i].cols {
                    w.insert(c.path.to_lowercase());
                }
            }
        }
    }

    // ------------------------------------------------------------ teclado

    fn keys(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        // Con un menú abierto, las teclas son del menú (Esc lo cierra).
        if self.palette.open || self.batch.is_some() || self.pattern.is_some() || self.hash.is_some() || self.locks.is_some() || self.default_dlg.is_some() || self.lang_dlg.is_some() || self.admin.is_some() || self.first_run || egui::Popup::is_any_open(ctx) || chrome::overlay_open(ctx) {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input() || self.rename.is_some();
        let consume = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        let ctrl = Modifiers::CTRL;
        let cs = Modifiers::CTRL | Modifiers::SHIFT;

        // Atajos que funcionan siempre.
        if consume(ctrl, Key::T) {
            actions.push(Action::NewTab(None));
        }
        if consume(ctrl, Key::W) {
            actions.push(Action::CloseTab(self.cur));
        }
        // Ctrl+Tab recorre las pestañas del panel enfocado.
        let mine = self.tabs_of(self.focus);
        let k = mine.iter().position(|&i| i == self.cur).unwrap_or(0);
        if consume(cs, Key::Tab) {
            actions.push(Action::SwitchTab(mine[(k + mine.len() - 1) % mine.len()]));
        } else if consume(ctrl, Key::Tab) {
            actions.push(Action::SwitchTab(mine[(k + 1) % mine.len()]));
        }
        if consume(ctrl, Key::Q) {
            crate::instance::request_quit();
        }
        if consume(ctrl, Key::Backslash) {
            actions.push(Action::ToggleSplit);
        }
        // Tab (sin escribir en ningún campo) pasa al otro panel.
        if self.cfg.split && !ctx.egui_wants_keyboard_input() && self.rename.is_none() && consume(Modifiers::NONE, Key::Tab) {
            let other = 1 - self.focus;
            self.set_focus(other);
        }
        if consume(ctrl, Key::F) {
            self.tab().search.focus = true;
        }
        // Paleta de comandos.
        if consume(ctrl, Key::K) || consume(cs, Key::P) {
            actions.push(Action::OpenPalette);
        }
        if !typing && self.tabs[self.cur].tool.is_none() {
            if consume(cs, Key::S) {
                let t = self.target();
                actions.push(Action::Command("select_pattern", t));
            }
            if consume(ctrl, Key::I) {
                let t = self.target();
                actions.push(Action::Command("invert_selection", t));
            }
        }
        if consume(cs, Key::L) {
            actions.push(Action::ToggleFlat);
        }
        if consume(cs, Key::R) {
            let t = self.target();
            actions.push(Action::Command("batch_rename", t));
        }
        if consume(ctrl, Key::L) {
            let p = self.tab().path().to_string();
            self.tab().path_edit = Some(p);
        }
        if consume(ctrl, Key::Num1) {
            actions.push(Action::SetView(ViewMode::Columns));
        }
        if consume(ctrl, Key::Num2) {
            actions.push(Action::SetView(ViewMode::Details));
        }
        if consume(ctrl, Key::Num3) {
            actions.push(Action::SetView(ViewMode::Grid));
        }
        if consume(ctrl, Key::P) {
            actions.push(Action::TogglePreview);
        }
        if consume(ctrl, Key::H) {
            actions.push(Action::ToggleHidden);
        }
        if consume(Modifiers::NONE, Key::F5) {
            actions.push(Action::Refresh);
        }
        let (extra1, extra2) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if extra1 {
            actions.push(Action::Back);
        }
        if extra2 {
            actions.push(Action::Forward);
        }
        if consume(Modifiers::ALT, Key::ArrowLeft) {
            actions.push(Action::Back);
        }
        if consume(Modifiers::ALT, Key::ArrowRight) {
            actions.push(Action::Forward);
        }
        if consume(Modifiers::ALT, Key::ArrowUp) {
            actions.push(Action::Up);
        }

        if !typing && consume(ctrl, Key::Z) {
            let t = self.target();
            actions.push(Action::Command("undo", t));
        }
        // Una pestaña con una herramienta no tiene lista de archivos.
        if self.tabs[self.cur].tool.is_some() {
            return;
        }
        let search_active = self.tabs[self.cur].search.active();
        if search_active {
            // Navegación de resultados (también con el foco en la caja de búsqueda).
            let n = self.tabs[self.cur].search.results.as_ref().map_or(0, |r| r.len());
            let sid = Id::new(("search", self.tabs[self.cur].id));
            let s = &mut self.tabs[self.cur].search;
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) && n > 0 {
                s.cursor = (s.cursor + 1).min(n - 1);
                s.scroll_to_cursor = true;
                // Soltar la caja de búsqueda para que Espacio abra la vista rápida.
                ctx.memory_mut(|m| m.surrender_focus(sid));
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                s.cursor = s.cursor.saturating_sub(1);
                s.scroll_to_cursor = true;
                ctx.memory_mut(|m| m.surrender_focus(sid));
            }
            let row = s.results.as_ref().and_then(|r| r.row(s.cursor));
            if let Some(row) = row {
                if consume(ctrl, Key::Enter) {
                    actions.push(Action::Reveal(row.path()));
                } else if consume(Modifiers::NONE, Key::Enter) {
                    actions.push(Action::Open { path: row.path(), is_dir: row.is_dir });
                }
            }
            if consume(Modifiers::NONE, Key::Escape) {
                if self.quicklook {
                    self.quicklook = false;
                } else {
                    let s = &mut self.tabs[self.cur].search;
                    s.text.clear();
                    ctx.memory_mut(|m| m.surrender_focus(Id::new(("search", self.tabs[self.cur].id))));
                }
            }
        }
        if typing {
            return;
        }
        if self.quicklook && self.ql_page.0.to_lowercase().ends_with(".pdf") {
            if consume(Modifiers::NONE, Key::ArrowRight) {
                self.ql_page.1 += 1;
            }
            if consume(Modifiers::NONE, Key::ArrowLeft) {
                self.ql_page.1 = self.ql_page.1.saturating_sub(1);
            }
        }
        if search_active {
            if consume(Modifiers::NONE, Key::Space) {
                self.quicklook ^= true;
            }
            // Escribir con la lista enfocada sigue editando la búsqueda.
            let typed: String = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
            let typed = typed.replace(' ', "");
            if !typed.is_empty() && !self.quicklook {
                let s = &mut self.tabs[self.cur].search;
                s.text.push_str(&typed);
                s.focus = true;
            }
            if consume(Modifiers::NONE, Key::Backspace) {
                let s = &mut self.tabs[self.cur].search;
                s.text.pop();
                s.focus = true;
            }
            return;
        }

        let columns = self.cfg.view == ViewMode::Columns;
        if consume(Modifiers::NONE, Key::Space) {
            self.quicklook ^= true;
        }
        if consume(Modifiers::NONE, Key::Escape) {
            if self.quicklook {
                self.quicklook = false;
            } else if !self.tab().filter.is_empty() {
                self.tab().filter.clear();
            } else if self.tab().flat && self.tabs[self.cur].active_col().selected().is_empty() {
                actions.push(Action::ToggleFlat);
            } else {
                self.tab().active_col_mut().multi.clear();
            }
        }
        // Mover el cursor (en la cuadrícula, ↑↓ saltan una fila y ←→ de a uno).
        let grid = self.cfg.view == ViewMode::Grid;
        let step = if grid { views::grid_columns(ctx, self.tabs[self.cur].id) as isize } else { 1 };
        let down = consume(Modifiers::NONE, Key::ArrowDown);
        let up = consume(Modifiers::NONE, Key::ArrowUp);
        let right = grid && consume(Modifiers::NONE, Key::ArrowRight);
        let left = grid && consume(Modifiers::NONE, Key::ArrowLeft);
        let home = consume(Modifiers::NONE, Key::Home);
        let end = consume(Modifiers::NONE, Key::End);
        let pgdn = consume(Modifiers::NONE, Key::PageDown);
        let pgup = consume(Modifiers::NONE, Key::PageUp);
        if down || up || left || right || home || end || pgdn || pgup {
            let cursor = self.tabs[self.cur].active_col().cursor.clone();
            let (ci, _, list) = self.active_list();
            if !list.is_empty() {
                let pos = cursor.and_then(|c| list.iter().position(|e| e.name == c));
                let n = list.len() as isize;
                let next = match pos {
                    None => 0,
                    Some(p) => {
                        let p = p as isize;
                        let d = if down {
                            step
                        } else if up {
                            -step
                        } else if right {
                            1
                        } else if left {
                            -1
                        } else if pgdn {
                            15 * step
                        } else if pgup {
                            -15 * step
                        } else if home {
                            -n
                        } else {
                            n
                        };
                        (p + d).clamp(0, n - 1)
                    }
                } as usize;
                let e = &list[next];
                actions.push(Action::Select { col: ci, name: e.name.clone(), is_dir: e.is_dir, ctrl: false, shift: false });
            }
        }
        if columns {
            if consume(Modifiers::NONE, Key::ArrowRight) {
                let tab = self.tab();
                let ci = tab.active.min(tab.cols.len() - 1);
                if ci + 1 < tab.cols.len() {
                    let next_path = tab.cols[ci + 1].path.clone();
                    let list = self.view_of(&next_path);
                    if let Some(first) = list.first() {
                        actions.push(Action::Select { col: ci + 1, name: first.name.clone(), is_dir: first.is_dir, ctrl: false, shift: false });
                    }
                }
            }
            if consume(Modifiers::NONE, Key::ArrowLeft) {
                let tab = self.tab();
                let ci = tab.active.min(tab.cols.len() - 1);
                if ci > 0 {
                    tab.cols.truncate(ci + 1);
                    tab.cols[ci].cursor = None;
                    tab.cols[ci].multi.clear();
                    tab.active = ci - 1;
                }
            }
        }
        if consume(Modifiers::NONE, Key::Enter) {
            if let Some((dir, name)) = self.tabs[self.cur].focused() {
                if let Some(e) = self.entry_in(&dir, &name) {
                    actions.push(Action::Open { path: fsx::join(&dir, &name), is_dir: e.is_dir });
                }
            }
        }
        if consume(Modifiers::ALT, Key::Enter) {
            let p = self.tabs[self.cur].selected_paths().first().cloned().unwrap_or_else(|| self.tabs[self.cur].path().to_string());
            actions.push(Action::Properties(p));
        }
        if consume(Modifiers::NONE, Key::Backspace) {
            // Con filtro, Retroceso lo edita; sin filtro, sube una carpeta.
            if self.tab().filter.is_empty() {
                actions.push(Action::Up);
            } else {
                self.tab().filter.pop();
            }
        }
        if consume(Modifiers::NONE, Key::F2) {
            if let Some((dir, name)) = self.tabs[self.cur].focused() {
                actions.push(Action::StartRename(dir, name));
            }
        }
        if consume(Modifiers::NONE, Key::Delete) {
            let sel = self.tabs[self.cur].selected_paths();
            if !sel.is_empty() {
                actions.push(Action::Delete(sel, false));
            }
        }
        if consume(cs, Key::N) {
            let p = self.tabs[self.cur].active_col().path.clone();
            actions.push(Action::NewFolder(p));
        }
        if consume(cs, Key::C) {
            let sel = self.tabs[self.cur].selected_paths();
            if !sel.is_empty() {
                actions.push(Action::CopyPath(sel));
            }
        }
        if consume(Modifiers::SHIFT, Key::F10) {
            let sel = self.tabs[self.cur].selected_paths();
            if !sel.is_empty() {
                actions.push(Action::NativeMenu(sel));
            }
        }
        if consume(ctrl, Key::A) {
            let path = self.tabs[self.cur].active_col().path.clone();
            let list = self.view_of(&path);
            let col = self.tab().active_col_mut();
            col.multi = list.iter().map(|e| e.name.clone()).collect();
        }
        // Copiar / cortar llegan como eventos; Mayús+Supr llega como "cortar".
        let (copy, cut, shift) = ctx.input(|i| {
            (i.events.iter().any(|e| matches!(e, egui::Event::Copy)), i.events.iter().any(|e| matches!(e, egui::Event::Cut)), i.modifiers.shift && !i.modifiers.ctrl)
        });
        let sel = self.tabs[self.cur].selected_paths();
        if cut && shift {
            if !sel.is_empty() {
                actions.push(Action::Delete(sel.clone(), true));
            }
        } else if (copy || cut) && !sel.is_empty() {
            actions.push(Action::Copy(sel.clone(), cut));
        }
        // Ctrl+V con archivos en el portapapeles no genera evento: se lee el teclado.
        let v_down = unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
            (GetAsyncKeyState(0x56) as u16 & 0x8000 != 0) && (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0)
        };
        if v_down && !self.paste_latch && ctx.input(|i| i.focused) {
            let p = self.tabs[self.cur].active_col().path.clone();
            actions.push(Action::Paste(p));
        }
        self.paste_latch = v_down;

        // Escribir filtra la carpeta enfocada (sin distinguir mayúsculas ni acentos).
        let typed: String = ctx.input(|i| {
            if i.modifiers.ctrl || i.modifiers.alt {
                return String::new();
            }
            i.events
                .iter()
                .filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None })
                .collect()
        });
        // El espacio abre la vista rápida; solo cuenta dentro de un filtro ya empezado.
        let typed = if self.tabs[self.cur].filter.is_empty() { typed.trim_start().to_string() } else { typed };
        if !typed.is_empty() {
            self.tab().filter.push_str(&typed);
            self.quicklook = false;
        }
        // Si lo enfocado quedó fuera del filtro, enfocar el primer resultado.
        if !self.tabs[self.cur].filter.is_empty() {
            let cursor = self.tabs[self.cur].active_col().cursor.clone();
            let (ci, _, list) = self.active_list();
            if !cursor.is_some_and(|c| list.iter().any(|e| e.name == c)) {
                if let Some(e) = list.first() {
                    actions.push(Action::Select { col: ci, name: e.name.clone(), is_dir: e.is_dir, ctrl: false, shift: false });
                }
            }
        }
    }

    // ------------------------------------------------------------ acciones

    fn apply(&mut self, ctx: &egui::Context, a: Action) {
        let columns = self.cfg.view == ViewMode::Columns;
        // La papelera es un lugar más: navegar desde ella la reemplaza en la misma pestaña,
        // y Atrás / Subir vuelven a la carpeta desde donde se abrió.
        if matches!(self.tabs[self.cur].tool.as_deref(), Some(super::tools::Tool::Recycle(_))) {
            match &a {
                Action::Back | Action::Up => {
                    self.tabs[self.cur].tool = None;
                    return;
                }
                Action::Forward | Action::Select { .. } => return,
                Action::Navigate(_) | Action::Reveal(_) | Action::Open { is_dir: true, .. } => self.tabs[self.cur].tool = None,
                _ => {}
            }
        }
        // Navegar desde otra herramienta (duplicados, comparar) no la pisa: abre una pestaña nueva.
        if self.tabs[self.cur].tool.is_some() {
            match &a {
                Action::Back | Action::Forward | Action::Up | Action::Select { .. } => return,
                Action::Navigate(_) | Action::Reveal(_) | Action::Open { is_dir: true, .. } => {
                    let path = self.tabs[self.cur].path().to_string();
                    self.apply(ctx, Action::NewTab(Some(path)));
                }
                _ => {}
            }
        }
        match a {
            Action::Navigate(p) => {
                let p = fsx::normalize(&p);
                if fsx::is_dir_like(&p) {
                    let tab = self.tab();
                    tab.search.text.clear();
                    tab.navigate(&p, true);
                } else if fsx::exists_like(&p) {
                    self.tab().reveal(&p);
                } else {
                    self.toast(ctx, tf!("app.no_existe", ruta = p));
                }
            }
            Action::PathInput(text) => {
                let raw = text.trim().to_string();
                if raw.is_empty() {
                    return;
                }
                if raw == ".." {
                    return self.apply(ctx, Action::Up);
                }
                let dir = self.tabs[self.cur].path().to_string();
                let p = fsx::expand_user_path(&raw);
                let absolute = p.as_bytes().get(1) == Some(&b':') || p.starts_with("\\\\");
                let rel = (!dir.is_empty()).then(|| fsx::join(&dir, &p));
                if absolute {
                    self.apply(ctx, Action::Navigate(p));
                } else if let Some(rel) = rel.filter(|r| fsx::exists_like(r)) {
                    // Una carpeta (o archivo) de la carpeta actual.
                    self.apply(ctx, Action::Navigate(rel));
                } else if p.contains(['\\', '/']) && !raw.contains("://") {
                    self.toast(ctx, tf!("app.no_existe", ruta = p));
                } else {
                    // Un programa: como en «Ejecutar», abierto en esta carpeta.
                    match shell::run_typed(&raw, &dir) {
                        Some(what) => self.toast(ctx, tf!("app.abriendo_esta_carpeta", que = what)),
                        None => self.toast(ctx, tf!("app.no_carpeta_ni_programa", texto = raw)),
                    }
                }
            }
            Action::OpenMenu(req) => {
                let (paths, background) = req.shell_key();
                if let Some(s) = &self.shell_menus {
                    // Con Mayús, también las opciones "extendidas" de Windows.
                    s.build(paths, background, ctx.input(|i| i.modifiers.shift));
                }
                self.menu = Some(MenuWin { req, anchor: shell::cursor_pos(), opened: ctx.input(|i| i.time), query: String::new(), show_hidden: false, subs: Vec::new(), styled: 0 });
            }
            Action::ShellInvoke(token, id) => {
                if let Some(s) = &self.shell_menus {
                    s.invoke(token, id);
                }
            }
            Action::TogglePin(key) => {
                if key.is_empty() {
                    return;
                }
                if let Some(i) = self.cfg.menu_pins.iter().position(|k| *k == key) {
                    self.cfg.menu_pins.remove(i);
                } else {
                    self.cfg.menu_pins.push(key);
                }
                self.save_cfg();
            }
            Action::ForgetSearch(k) => {
                match k {
                    Some(k) if k < self.cfg.search_history.len() => {
                        self.cfg.search_history.remove(k);
                    }
                    Some(_) => {}
                    None => self.cfg.search_history.clear(),
                }
                self.save_cfg();
            }
            Action::NewTab(p) => {
                let path = p.unwrap_or_else(|| self.tabs[self.cur].path().to_string());
                let id = self.next_id;
                self.next_id += 1;
                let mut t = Tab::new(id, &path);
                t.pane = self.focus as u8;
                let at = self.cur + 1;
                self.tabs.insert(at, t);
                for q in 0..2 {
                    if self.pane[q] >= at {
                        self.pane[q] += 1;
                    }
                }
                self.pane[self.focus] = at;
                self.cur = at;
            }
            Action::CloseTab(i) => {
                // Cada panel conserva al menos una pestaña.
                if i < self.tabs.len() {
                    let p = self.tabs[i].pane as usize;
                    let mine = self.tabs_of(p);
                    if mine.len() > 1 {
                        let k = mine.iter().position(|&x| x == i).unwrap_or(0);
                        let next = if k > 0 { mine[k - 1] } else { mine[1] };
                        self.tabs.remove(i);
                        for q in 0..2 {
                            if self.pane[q] == i {
                                self.pane[q] = if next > i { next - 1 } else { next };
                            } else if self.pane[q] > i {
                                self.pane[q] -= 1;
                            }
                        }
                        self.cur = self.pane[self.focus];
                    }
                }
            }
            Action::SwitchTab(i) => {
                if i < self.tabs.len() {
                    let p = self.tabs[i].pane as usize;
                    self.pane[p] = i;
                    self.set_focus(p);
                }
            }
            Action::ToggleSplit => {
                self.cfg.split ^= true;
                if !self.cfg.split {
                    self.set_focus(0);
                } else if self.tabs[self.pane[1]].pane != 1 {
                    // Primera vez: el panel derecho empieza en la misma carpeta.
                    let path = self.tabs[self.cur].path().to_string();
                    self.set_focus(1);
                    let id = self.next_id;
                    self.next_id += 1;
                    let mut t = Tab::new(id, &path);
                    t.pane = 1;
                    self.tabs.push(t);
                    self.pane[1] = self.tabs.len() - 1;
                    self.cur = self.pane[1];
                }
                self.save_cfg();
            }
            Action::SendToOther { paths, mv } => match self.other_path() {
                Some(dest) => self.transfer(ctx, paths, dest, mv),
                None => self.toast(ctx, t!("app.abre_vista_dividida_ctrl_enviar")),
            },
            Action::DropFiles { paths, dest, force } => {
                if self.read_only(ctx, &dest) {
                    return;
                }
                // Como en el Explorador: mismo disco = mover; otro disco = copiar.
                // Ctrl fuerza copiar y Mayús fuerza mover.
                let same_drive = paths.first().is_some_and(|p| p.get(..2).map(str::to_lowercase) == dest.get(..2).map(str::to_lowercase));
                let mv = force.unwrap_or(same_drive);
                if mv {
                    if let Some(h) = &self.prevhost {
                        h.release(&paths);
                    }
                    // Si se mueve una carpeta que está abierta (p. ej. arrastrando la ruta), la pestaña la sigue.
                    let now = ctx.input(|i| i.time);
                    for p in &paths {
                        let pl = p.to_lowercase();
                        let open = self.tabs.iter().any(|t| t.cols.iter().any(|c| {
                            let l = c.path.to_lowercase();
                            l == pl || l.starts_with(&format!("{pl}\\"))
                        }));
                        if open {
                            self.follow_moves.push((p.clone(), fsx::join(&dest, &fsx::file_name(p)), now + 120.0));
                        }
                    }
                }
                self.transfer(ctx, paths, dest, mv);
            }
            Action::TransferPause(id, p) => self.xfer.set_paused(id, p),
            Action::TransferCancel(id) => self.xfer.cancel(id),
            Action::TransferResolve(id, c, all) => self.xfer.resolve(id, c, all),
            Action::OpenPalette => self.palette.show(),
            Action::ToggleFlat => {
                let on = !self.tabs[self.cur].flat;
                if on && self.cfg.view == ViewMode::Columns {
                    self.apply(ctx, Action::SetView(ViewMode::Details));
                }
                let tab = self.tab();
                tab.flat = on;
                tab.filter.clear();
                let c = tab.active_col_mut();
                c.cursor = None;
                c.multi.clear();
                if on {
                    let key = format!("{}{}", fsx::FLAT_KEY, self.tabs[self.cur].path());
                    self.dirs.reload(&key);
                }
            }
            Action::OpenOther(path) => {
                if !self.cfg.split {
                    self.apply(ctx, Action::ToggleSplit);
                }
                let ti = self.pane[1 - self.focus];
                if self.tabs[ti].tool.is_some() {
                    let other = 1 - self.focus;
                    self.set_focus(other);
                    self.apply(ctx, Action::NewTab(Some(path)));
                } else {
                    self.tabs[ti].search.text.clear();
                    self.tabs[ti].navigate(&path, true);
                }
            }
            Action::OpenRecycle => {
                let Some(h) = self.hwnd else { return };
                if matches!(self.tabs[self.cur].tool.as_deref(), Some(super::tools::Tool::Recycle(_))) {
                    return;
                }
                let tool = super::tools::Tool::Recycle(super::tools::RecycleTool::new(h, self.mgr.clone(), ctx));
                if self.tabs[self.cur].tool.is_none() {
                    // En la misma pestaña, como cualquier lugar.
                    let tab = self.tab();
                    tab.search.text.clear();
                    tab.filter.clear();
                    tab.tool = Some(Box::new(tool));
                } else {
                    let dir = self.tabs[self.cur].path().to_string();
                    self.open_tool(ctx, dir, tool);
                }
            }
            Action::OpenRecent => {
                // En la misma pestaña (si es una carpeta o la papelera); Esc o cualquier lugar vuelven a ella.
                if matches!(self.tabs[self.cur].tool.as_deref(), Some(super::tools::Tool::Recycle(_))) {
                    self.tabs[self.cur].tool = None;
                }
                if self.tabs[self.cur].tool.is_some() {
                    let home = self.places.first().map(|p| p.path.clone()).unwrap_or_else(|| "C:\\".into());
                    self.apply(ctx, Action::NewTab(Some(home)));
                }
                let s = &mut self.tabs[self.cur].search;
                s.text = "recientes:".into();
                s.here = false;
                s.sort = crate::index::ResultSort::Modified;
                s.desc = true;
            }
            Action::Command(id, t) => self.run_command(ctx, id, t),
            Action::TransferClear => {
                self.xfer.clear_finished();
                self.show_transfers = false;
            }
            Action::Reveal(p) => {
                let tab = self.tab();
                tab.search.text.clear();
                tab.reveal(&p);
            }
            Action::Select { col, name, is_dir, ctrl, shift } => {
                self.rename = None;
                let path = self.tabs[self.cur].cols.get(col).map(|c| c.path.clone());
                let Some(path) = path else { return };
                if ctrl || shift {
                    let list = self.view_of(&path);
                    let tab = self.tab();
                    tab.cols.truncate(col + 1);
                    tab.active = col;
                    let c = &mut tab.cols[col];
                    if ctrl {
                        if c.multi.is_empty() {
                            if let Some(cur) = c.cursor.clone() {
                                c.multi.insert(cur);
                            }
                        }
                        if !c.multi.remove(&name) {
                            c.multi.insert(name.clone());
                        }
                        c.anchor = Some(name.clone());
                    } else {
                        let anchor = c.anchor.clone().or(c.cursor.clone()).unwrap_or(name.clone());
                        let a = list.iter().position(|e| e.name == anchor).unwrap_or(0);
                        let b = list.iter().position(|e| e.name == name).unwrap_or(0);
                        let (lo, hi) = (a.min(b), a.max(b));
                        c.multi = list[lo..=hi].iter().map(|e| e.name.clone()).collect();
                    }
                    c.cursor = Some(name);
                } else {
                    self.tab().select(col, &name, is_dir, columns);
                }
            }
            Action::SetSelection { col, names } => {
                self.rename = None;
                let cursor = names.first().cloned().filter(|n| self.cursor_ok(col, n));
                let tab = self.tab();
                if col < tab.cols.len() {
                    tab.cols.truncate(col + 1);
                    tab.active = col;
                    let c = &mut tab.cols[col];
                    c.cursor = cursor;
                    c.anchor = names.first().cloned();
                    c.multi = names.into_iter().collect();
                }
            }
            Action::DragSelect { col, name } => {
                let cursor_ok = self.cursor_ok(col, &name);
                let tab = self.tab();
                if col < tab.cols.len() {
                    tab.active = col;
                    let c = &mut tab.cols[col];
                    if cursor_ok {
                        c.cursor = Some(name.clone());
                    }
                    c.multi = [name.clone()].into_iter().collect();
                    c.anchor = Some(name);
                }
            }
            Action::FocusCol(ci) => {
                let tab = self.tab();
                if ci < tab.cols.len() {
                    tab.active = ci;
                }
            }
            Action::Open { path, is_dir } => {
                // Un comprimido (o una carpeta de adentro) se abre como carpeta.
                if is_dir || fsx::archive::is_dir(&path) {
                    let tab = self.tab();
                    tab.search.text.clear();
                    tab.navigate(&path, true);
                } else if fsx::archive::is_inside(&path) {
                    // Un archivo de adentro: se abre una copia temporal (los cambios no vuelven al comprimido).
                    self.toast(ctx, t!("app.abriendo_copia_temporal"));
                    let result = self.service_result.clone();
                    std::thread::spawn(move || match fsx::archive::materialize(&path) {
                        Some(tmp) if shell::open(&tmp) => {}
                        _ => *result.lock() = Some(t!("app.no_pudo_abrir_archivo").into()),
                    });
                } else if !shell::open(&path) {
                    self.toast(ctx, t!("app.no_pudo_abrir_archivo"));
                }
            }
            Action::OpenWith(p) => {
                shell::open_with(&p);
            }
            Action::QuickLook => self.quicklook = true,
            Action::StartRename(dir, _) if self.read_only(ctx, &dir) => {}
            Action::Delete(paths, _) if paths.first().is_some_and(|p| fsx::archive::is_inside(p)) => {
                self.toast(ctx, t!("app.comprimidos_son_solo_lectura_extrae"));
            }
            Action::NewFolder(dir) if self.read_only(ctx, &dir) => {}
            Action::NewFile(dir, k) => {
                let Some(item) = fsx::shellnew::items().and_then(|i| i.get(k)) else { return };
                match fsx::shellnew::create(&dir, item) {
                    Ok(p) => {
                        self.push_undo(Undo::Copied(vec![p.clone()]));
                        self.dirs.reload(&dir);
                        let name = fsx::file_name(&p);
                        let tab = self.tab();
                        if let Some(ci) = tab.cols.iter().position(|c| c.path == dir) {
                            tab.cols.truncate(ci + 1);
                            tab.cols[ci].cursor = Some(name.clone());
                            tab.cols[ci].multi.clear();
                            tab.cols[ci].scroll_to_cursor = true;
                            tab.active = ci;
                        }
                        // Como en el Explorador: queda listo para escribirle el nombre.
                        self.rename = Some(Rename { dir, old: name.clone(), buf: name, first_frame: true });
                    }
                    Err(e) => self.toast(ctx, tf!("app.no_pudo_crear_archivo", error = e)),
                }
            }
            Action::Paste(dest) if self.read_only(ctx, &dest) => {}
            Action::StartRename(dir, name) => {
                // En la vista plana el nombre trae su subcarpeta: se edita solo el nombre.
                let buf = fsx::file_name(&name);
                self.rename = Some(Rename { dir, old: name, buf, first_frame: true });
            }
            Action::CommitRename => {
                if let Some(r) = self.rename.take() {
                    let new = r.buf.trim().to_string();
                    if !new.is_empty() && new != r.old {
                        let old_path = fsx::join(&r.dir, &r.old);
                        if let Some(h) = &self.prevhost {
                            h.release(std::slice::from_ref(&old_path));
                        }
                        match shell::rename(&old_path, &new) {
                            Ok(new_path) => {
                                self.dirs.reload(&r.dir);
                                self.retarget_cols(&old_path, &new_path);
                                self.push_undo(Undo::Rename { dir: r.dir.clone(), old: r.old.clone(), new: new.clone() });
                                let new = match r.old.rfind('\\') {
                                    Some(i) => format!("{}\\{new}", &r.old[..i]),
                                    None => new,
                                };
                                let tab = self.tab();
                                if let Some(c) = tab.cols.iter_mut().find(|c| c.path == r.dir) {
                                    c.cursor = Some(new);
                                    c.multi.clear();
                                }
                            }
                            Err(e) if e.contains("os error 32") || e.contains("os error 33") => {
                                // Otro programa lo tiene abierto: decir cuál y ofrecer reintentar.
                                self.rename = Some(Rename { dir: r.dir.clone(), old: r.old.clone(), buf: r.buf.clone(), first_frame: false });
                                let retry = (t!("app.volver_intentar").to_string(), Action::CommitRename);
                                self.locks = Some(super::tools::LocksDialog::new(vec![old_path], Some(t!("app.no_pudo_renombrar").into()), Some(retry), ctx));
                            }
                            Err(e) if crate::admin::is_access_denied(&e) => {
                                let to = fsx::join(&r.dir, &new);
                                self.admin = Some((t!("app.windows_pide_permiso_administrador_renom").into(), crate::admin::AdminOp::Rename { from: old_path, to }));
                            }
                            Err(e) => self.toast(ctx, tf!("app.no_pudo_renombrar_error", error = e)),
                        }
                    }
                }
            }
            Action::Copy(paths, _) if paths.first().is_some_and(|p| fsx::archive::is_inside(p)) => {
                self.toast(ctx, t!("app.preparando_copia"));
                let result = self.service_result.clone();
                std::thread::spawn(move || {
                    let tmp: Vec<String> = paths.iter().filter_map(|p| fsx::archive::materialize(p)).collect();
                    let ok = !tmp.is_empty() && shell::clipboard_set_files(&tmp, false);
                    *result.lock() = Some(if ok { tp!(tmp.len(), "app.copiado_n") } else { t!("app.no_pudo_copiar").into() });
                });
            }
            Action::Copy(paths, cut) => {
                if shell::clipboard_set_files(&paths, cut) {
                    let n = paths.len();
                    self.cut = if cut { paths } else { Vec::new() };
                    self.toast(ctx, if cut { tp!(n, "app.cortado_n") } else { tp!(n, "app.copiado_n") });
                }
            }
            Action::Paste(dest) => {
                if let Some((files, cut)) = shell::clipboard_get_files() {
                    if cut {
                        self.cut.clear();
                    }
                    // Pegar en la misma carpeta de donde se copió crea una copia ("nombre (2)").
                    let n = files.len();
                    let id = self.xfer.enqueue(if cut { transfer::Op::Move } else { transfer::Op::Copy }, files, dest.clone());
                    self.awaiting.push((id, cut));
                    self.toast(ctx, if cut { tp!(n, "app.moviendo_n") } else { tp!(n, "app.copiando_n") });
                }
            }
            Action::CopyPath(paths) => {
                ctx.copy_text(paths.join("\r\n"));
                self.toast(ctx, t!("app.ruta_copiada"));
            }
            Action::Delete(paths, forever) => {
                if !forever {
                    self.push_undo(Undo::Recycle(paths.clone()));
                }
                if let Some(h) = &self.prevhost {
                    h.release(&paths);
                }
                let dirs: Vec<String> = paths.iter().filter_map(|p| fsx::parent_of(p)).collect();
                let mgr = self.mgr.clone();
                let (report, toast, admin_report) = (self.lock_report.clone(), self.service_result.clone(), self.admin_report.clone());
                let list = paths.clone();
                shell::file_op(if forever { FileOp::DeleteForever } else { FileOp::Recycle }, paths, None, move |aborted| {
                    mgr.dirty.lock().extend(dirs.iter().map(|d| d.to_lowercase()));
                    // Lo que sigue ahí sin que se haya cancelado: ¿quién lo tiene abierto?
                    let left: Vec<String> = list.into_iter().filter(|p| std::fs::symlink_metadata(fsx::long_path(p)).is_ok()).collect();
                    if !aborted && !left.is_empty() {
                        if fsx::locks::who_uses(&left).is_ok_and(|l| !l.is_empty()) {
                            *report.lock() = Some((left, forever));
                        } else {
                            // Nadie lo tiene abierto: lo más probable es que falte permiso.
                            let _ = &toast;
                            *admin_report.lock() = Some(crate::admin::AdminOp::Delete { paths: left, forever });
                        }
                    }
                    mgr.wake_ui();
                });
            }
            Action::Properties(p) => shell::properties(&p),
            Action::NativeMenu(paths) => {
                self.pending_menu = Some((paths, shell::cursor_pos(), 0));
                ctx.request_repaint();
            }
            Action::NewFolder(dir) => match shell::new_folder(&dir) {
                Ok(p) => {
                    self.push_undo(Undo::NewFolder(p.clone()));
                    self.dirs.reload(&dir);
                    let name = fsx::file_name(&p);
                    let tab = self.tab();
                    if let Some(ci) = tab.cols.iter().position(|c| c.path == dir) {
                        tab.cols.truncate(ci + 1);
                        tab.cols[ci].cursor = Some(name.clone());
                        tab.cols[ci].multi.clear();
                        tab.cols[ci].scroll_to_cursor = true;
                        tab.active = ci;
                    }
                    self.rename = Some(Rename { dir, old: name.clone(), buf: name, first_frame: true });
                }
                Err(e) if crate::admin::is_access_denied(&e) => {
                    self.admin = Some((t!("app.windows_pide_permiso_administrador_crear").into(), crate::admin::AdminOp::NewFolder { dir: dir.clone() }));
                }
                Err(e) => self.toast(ctx, tf!("app.no_pudo_crear_carpeta", error = e)),
            },
            Action::Terminal(d) => shell::open_terminal(&d),
            Action::ShowInExplorer(p) => shell::reveal_in_explorer(&p),
            Action::ExplorerAt(d) => {
                shell::explorer_at(&d);
            }
            Action::AddFavorite(gi, p) => {
                if let Some(g) = self.cfg.groups.get_mut(gi) {
                    if !g.items.iter().any(|x| x.eq_ignore_ascii_case(&p)) {
                        g.items.push(p);
                        g.collapsed = false;
                    }
                }
                self.save_cfg();
            }
            Action::RemoveFavorite(gi, ii) => {
                if let Some(g) = self.cfg.groups.get_mut(gi) {
                    if ii < g.items.len() {
                        g.items.remove(ii);
                    }
                }
                self.save_cfg();
            }
            Action::NewGroup => {
                self.cfg.groups.push(FavGroup { name: t!("comun.nuevo_grupo").into(), items: Vec::new(), collapsed: false });
                self.group_edit = Some((self.cfg.groups.len() - 1, t!("comun.nuevo_grupo").into()));
            }
            Action::DeleteGroup(gi) => {
                if gi < self.cfg.groups.len() {
                    self.cfg.groups.remove(gi);
                }
                self.save_cfg();
            }
            Action::RenameGroup(gi) => {
                let name = self.cfg.groups.get(gi).map(|g| g.name.clone()).unwrap_or_default();
                self.group_edit = Some((gi, name));
            }
            Action::Eject(letter) => match shell::eject(letter) {
                Ok(()) => self.toast(ctx, tf!("app.puede_retirar_seguridad", letra = letter)),
                Err(e) => self.toast(ctx, tf!("app.no_pudo_expulsar", letra = letter, error = e)),
            },
            Action::SetView(v) => {
                self.cfg.view = v;
                self.remember_view();
                if v == ViewMode::Details {
                    // En detalles se muestra la carpeta enfocada.
                    let tab = self.tab();
                    let a = tab.active.min(tab.cols.len() - 1);
                    if a + 1 < tab.cols.len() {
                        let keep = tab.cols[a].cursor.clone();
                        tab.cols.truncate(a + 1);
                        tab.cols[a].cursor = keep;
                    }
                }
            }
            Action::SetSort(k) => {
                if self.cfg.sort == k {
                    self.cfg.sort_desc ^= true;
                } else {
                    self.cfg.sort = k;
                    self.cfg.sort_desc = matches!(k, crate::fsx::SortKey::Modified | crate::fsx::SortKey::Size);
                }
                self.remember_view();
            }
            Action::ToggleTheme => {
                self.cfg.dark ^= true;
                self.t = Theme::get(self.cfg.dark);
                theme::apply_style(ctx, &self.t);
                super::win::set_background([self.t.bg.r(), self.t.bg.g(), self.t.bg.b()]);
                if let Some(h) = &self.prevhost {
                    h.set_background([self.t.sidebar.r(), self.t.sidebar.g(), self.t.sidebar.b()]);
                    h.unload();
                }
                if let Some(h) = self.hwnd {
                    dark_title_bar(h, self.cfg.dark);
                }
                native_menu_theme(self.cfg.dark);
                self.save_cfg();
            }
            Action::TogglePreview => self.cfg.show_preview ^= true,
            Action::ToggleHidden => self.cfg.show_hidden ^= true,
            Action::Back => self.tab().go_back(),
            Action::Forward => self.tab().go_forward(),
            Action::Up => self.tab().go_up(),
            Action::SetResultSort(k) => {
                let s = &mut self.tab().search;
                if s.sort == k {
                    s.desc ^= true;
                } else {
                    s.sort = k;
                    s.desc = matches!(k, crate::index::ResultSort::Size | crate::index::ResultSort::Modified);
                }
                s.cursor = 0;
                s.scroll_to_cursor = true;
            }
            Action::LoadAllResults => self.tab().search.want_full = true,
            Action::InstallService | Action::UninstallService => {
                let install = matches!(a, Action::InstallService);
                let mgr = self.mgr.clone();
                let ctx2 = ctx.clone();
                let result = self.service_result.clone();
                self.toast(ctx, if install { t!("app.instalando_servicio_acepta_aviso_windows") } else { t!("app.quitando_servicio") });
                std::thread::spawn(move || {
                    let r = shell::run_self_elevated(if install { "--install-service" } else { "--uninstall-service" });
                    let msg = match r {
                        Ok(0) if install => t!("app.indice_instantaneo_activado_mantiene_dia").to_string(),
                        Ok(0) => t!("app.servicio_quitado").to_string(),
                        Ok(_) => t!("app.no_pudo_completar_operacion").to_string(),
                        Err(e) => e,
                    };
                    *result.lock() = Some(msg);
                    mgr.recheck.store(true, Ordering::Relaxed);
                    ctx2.request_repaint();
                });
            }
            Action::Refresh => {
                let paths: Vec<String> = self.tabs[self.cur].cols.iter().map(|c| c.path.clone()).collect();
                for p in paths {
                    self.dirs.reload(&p);
                }
                self.tex.clear();
            }
        }
    }

    /// Ejecuta un comando del catálogo (menú o paleta).
    fn run_command(&mut self, ctx: &egui::Context, id: &'static str, t: super::commands::Target) {
        use super::tools::{CompareTool, DupesTool, Tool};
        let first = t.paths.first().cloned();
        let is_dir = |p: &str| std::path::Path::new(&fsx::long_path(p)).is_dir();
        let a = match id {
            "open" => {
                // La primera carpeta se abre aquí; las demás, en pestañas.
                let mut navigated = false;
                for p in &t.paths {
                    let d = is_dir(p);
                    if d && navigated {
                        self.apply(ctx, Action::NewTab(Some(p.clone())));
                        continue;
                    }
                    navigated |= d;
                    self.apply(ctx, Action::Open { path: p.clone(), is_dir: d });
                }
                return;
            }
            "open_with" => match first {
                Some(p) => Action::OpenWith(p),
                None => return,
            },
            "open_tab" => {
                for p in &t.paths {
                    if is_dir(p) {
                        self.apply(ctx, Action::NewTab(Some(p.clone())));
                    } else {
                        self.apply(ctx, Action::NewTab(fsx::parent_of(p)));
                        self.apply(ctx, Action::Reveal(p.clone()));
                    }
                }
                return;
            }
            "quicklook" => Action::QuickLook,
            "copy" => Action::Copy(t.paths, false),
            "cut" => Action::Copy(t.paths, true),
            "paste" => Action::Paste(t.dir),
            "copy_path" => Action::CopyPath(if t.paths.is_empty() { vec![t.dir] } else { t.paths }),
            "rename" => match first.as_deref().and_then(|p| fsx::parent_of(p).map(|d| (d, fsx::file_name(p)))) {
                Some((d, n)) => Action::StartRename(d, n),
                None => return,
            },
            "batch_rename" => return self.open_batch(ctx, &t),
            "undo" => return self.undo(ctx),
            "recycle" => Action::OpenRecycle,
            "who_uses" => {
                if !t.paths.is_empty() {
                    self.locks = Some(super::tools::LocksDialog::new(t.paths, None, None, ctx));
                }
                return;
            }
            "hash" => {
                let files: Vec<String> = t.paths.iter().filter(|p| std::path::Path::new(&fsx::long_path(p)).is_file()).cloned().collect();
                if files.is_empty() {
                    self.toast(ctx, t!("app.elige_uno_mas_archivos"));
                } else {
                    self.hash = Some(super::tools::HashDialog::new(files, ctx));
                }
                return;
            }
            "paste_link" | "link_other" => {
                let (targets, dest) = if id == "paste_link" {
                    (shell::clipboard_get_files().map(|f| f.0).unwrap_or_default(), t.dir.clone())
                } else {
                    (t.paths.clone(), self.other_path().unwrap_or_default())
                };
                if targets.is_empty() || dest.is_empty() {
                    self.toast(ctx, if id == "paste_link" { t!("app.primero_copia_ctrl_c_quieres") } else { t!("app.elige_enlazar") });
                    return;
                }
                let (mut made, mut kinds, mut errors) = (Vec::new(), Vec::new(), Vec::new());
                for target in targets {
                    let link = fsx::links::link_path(&dest, &target);
                    match fsx::links::make(&target, &link) {
                        Ok(kind) => {
                            made.push(link);
                            if !kinds.contains(&kind) {
                                kinds.push(kind);
                            }
                        }
                        Err(e) => errors.push(e),
                    }
                }
                self.dirs.reload(&dest);
                if !made.is_empty() {
                    let n = made.len();
                    self.push_undo(Undo::Copied(made));
                    self.toast(ctx, tp!(n, "app.enlaces_creados", tipos = kinds.join(", ")));
                } else if let Some(e) = errors.first() {
                    self.toast(ctx, tf!("app.no_pudo_crear_enlace", error = e));
                }
                return;
            }
            "flat" => Action::ToggleFlat,
            "view_default" => {
                self.cfg.default_view = super::config::FolderView { view: self.cfg.view, sort: self.cfg.sort, desc: self.cfg.sort_desc };
                self.cfg.folder_views.clear();
                for t in &mut self.tabs {
                    t.view = None;
                }
                self.save_cfg();
                self.toast(ctx, t!("app.esta_vista_este_orden_son"));
                return;
            }
            "recent" => Action::OpenRecent,
            "select_all" | "invert_selection" => {
                let (ci, _, list) = self.active_list();
                let selected: std::collections::HashSet<String> = self.tabs[self.cur].active_col().selected().into_iter().collect();
                let names = list.iter().filter(|e| id == "select_all" || !selected.contains(&e.name)).map(|e| e.name.clone()).collect();
                Action::SetSelection { col: ci, names }
            }
            "select_pattern" => {
                self.pattern = Some((String::new(), false, true));
                return;
            }
            "quit" => {
                crate::instance::request_quit();
                return;
            }
            "resident" => {
                self.cfg.stay_resident ^= true;
                self.save_cfg();
                let msg = if self.cfg.stay_resident {
                    t!("app.cerrar_rayo_queda_segundo_plano")
                } else {
                    t!("app.cerrar_rayo_cierra_todo")
                };
                self.toast(ctx, msg);
                return;
            }
            "default_explorer" => {
                self.default_dlg = Some(true);
                return;
            }
            "language" => {
                self.lang_dlg = Some(super::language::LangDialog::new());
                return;
            }
            "install" => {
                if crate::install::is_installed_copy() {
                    self.toast(ctx, t!("app.este_ya_rayo_instalado"));
                } else {
                    self.install_and_restart(ctx);
                }
                return;
            }
            "autostart" => {
                let on = !self.cfg.start_with_windows;
                match crate::instance::set_autostart(on) {
                    Ok(()) => {
                        self.cfg.start_with_windows = on;
                        self.save_cfg();
                        self.toast(ctx, if on { t!("app.rayo_iniciara_windows_oculto_listo") } else { t!("app.rayo_ya_no_inicia_windows") });
                    }
                    Err(e) => self.toast(ctx, tf!("app.no_pudo_cambiar_inicio_windows", error = e)),
                }
                return;
            }
            "extract_here" | "extract_folder" => {
                // Lo de adentro de un comprimido sale junto al comprimido; un comprimido, en su carpeta.
                let mut inside = Vec::new();
                let mut whole = Vec::new();
                for p in &t.paths {
                    match fsx::archive::split(p) {
                        Some((a, inner)) if !inner.is_empty() => inside.push((a, p.clone())),
                        Some(_) => whole.push(p.clone()),
                        None => {}
                    }
                }
                if let Some((a, _)) = inside.first() {
                    let dest = fsx::parent_of(a).unwrap_or_default();
                    let dest = if id == "extract_folder" { fsx::join(&dest, &stem(a)) } else { dest };
                    let id = self.xfer.enqueue_extract(inside.into_iter().map(|x| x.1).collect(), Vec::new(), dest);
                    self.awaiting.push((id, false));
                }
                if !whole.is_empty() {
                    let rels = whole.iter().map(|p| if id == "extract_folder" { stem(p) } else { String::new() }).collect();
                    let dest = fsx::parent_of(&whole[0]).unwrap_or(t.dir.clone());
                    let jid = self.xfer.enqueue_extract(whole, rels, dest);
                    self.awaiting.push((jid, false));
                }
                self.toast(ctx, t!("app.extrayendo"));
                return;
            }
            "compress" => {
                if t.paths.is_empty() || self.read_only(ctx, &t.dir) {
                    return;
                }
                let name = if t.paths.len() == 1 { stem(&t.paths[0]) } else { fsx::file_name(&t.dir) };
                let zip = shell_free_name(&fsx::join(&t.dir, &format!("{name}.zip")));
                let id = self.xfer.enqueue_compress(t.paths, zip.clone());
                self.awaiting.push((id, false));
                self.toast(ctx, tf!("comun.comprimiendo", nombre = fsx::file_name(&zip)));
                return;
            }
            "new_folder" => Action::NewFolder(t.dir),
            "delete" => Action::Delete(t.paths, false),
            "properties" => Action::Properties(first.unwrap_or(t.dir)),
            "native_menu" => Action::NativeMenu(t.paths),
            "copy_other" => Action::SendToOther { paths: t.paths, mv: false },
            "move_other" => Action::SendToOther { paths: t.paths, mv: true },
            "open_other" => {
                if self.cfg.split {
                    let ti = self.pane[1 - self.focus];
                    if self.tabs[ti].tool.is_none() {
                        self.tabs[ti].search.text.clear();
                        self.tabs[ti].navigate(&t.dir, true);
                    }
                }
                return;
            }
            "compare" | "compare_other" => {
                let right = self.other_path().filter(|o| !o.eq_ignore_ascii_case(&t.dir)).unwrap_or_default();
                let tool = Tool::Compare(CompareTool::new(t.dir.clone(), right, ctx));
                return self.open_tool(ctx, t.dir, tool);
            }
            "dupes_here" | "dupes_drive" => {
                let (scope, min) = if id == "dupes_drive" { (format!("{}\\", t.dir.get(..2).unwrap_or("C:")), 1 << 20) } else { (t.dir.clone(), 1) };
                let tool = Tool::Dupes(DupesTool::new(scope.clone(), min, &self.mgr, ctx));
                return self.open_tool(ctx, scope, tool);
            }
            "space" => {
                // Qué ocupa espacio: detalles, de mayor a menor, con barras.
                self.cfg.sort = fsx::SortKey::Size;
                self.cfg.sort_desc = true;
                self.cfg.space_bars = true;
                self.apply(ctx, Action::SetView(ViewMode::Details));
                if !self.tabs[self.cur].path().eq_ignore_ascii_case(&t.dir) {
                    self.apply(ctx, Action::Navigate(t.dir));
                }
                self.save_cfg();
                return;
            }
            "transfers" => {
                self.show_transfers = true;
                return;
            }
            "terminal" => Action::Terminal(t.dir),
            "explorer" => match first {
                Some(p) if t.paths.len() == 1 => Action::ShowInExplorer(p),
                _ => Action::ExplorerAt(t.dir),
            },
            "favorite" => {
                if self.cfg.groups.is_empty() {
                    self.cfg.groups.push(FavGroup { name: t!("comun.favoritos").into(), items: Vec::new(), collapsed: false });
                }
                Action::AddFavorite(0, if t.single_dir { first.unwrap_or(t.dir) } else { t.dir })
            }
            "view_columns" => Action::SetView(ViewMode::Columns),
            "view_details" => Action::SetView(ViewMode::Details),
            "view_grid" => Action::SetView(ViewMode::Grid),
            "split" => Action::ToggleSplit,
            "preview" => Action::TogglePreview,
            "hidden" => Action::ToggleHidden,
            "space_bars" => {
                self.cfg.space_bars ^= true;
                self.save_cfg();
                return;
            }
            "theme" => Action::ToggleTheme,
            "new_tab" => Action::NewTab(None),
            "close_tab" => Action::CloseTab(self.cur),
            "refresh" => Action::Refresh,
            "search" => {
                self.tab().search.focus = true;
                return;
            }
            "service" => {
                let mode = self.mgr.mode();
                if mode.contains(crate::index::MODE_WATCH) {
                    Action::InstallService
                } else {
                    self.toast(ctx, tf!("app.indice_ya_mantiene_dia", modo = crate::index::mode_label(&mode)));
                    return;
                }
            }
            _ => return,
        };
        self.apply(ctx, a);
    }

    /// Dentro de un comprimido no se puede escribir: avisa y devuelve true.
    fn read_only(&mut self, ctx: &egui::Context, dir: &str) -> bool {
        if fsx::archive::split(dir).is_some() {
            self.toast(ctx, t!("app.comprimidos_son_solo_lectura_extrae"));
            return true;
        }
        false
    }

    /// ¿Se puede poner el cursor en este elemento sin abrir columnas? (en la vista de
    /// columnas, enfocar una carpeta abre su contenido a la derecha y movería todo).
    fn cursor_ok(&mut self, col: usize, name: &str) -> bool {
        if self.cfg.view != ViewMode::Columns {
            return true;
        }
        let Some(path) = self.tabs[self.cur].cols.get(col).map(|c| c.path.clone()) else { return false };
        self.view_of(&path).iter().any(|e| e.name == name && !e.is_dir)
    }

    fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Las columnas que mostraban una ruta (o algo dentro) pasan a la nueva.
    fn retarget_cols(&mut self, old_path: &str, new_path: &str) {
        let old_lower = old_path.to_lowercase();
        for tab in &mut self.tabs {
            for c in &mut tab.cols {
                let l = c.path.to_lowercase();
                if l == old_lower || l.starts_with(&format!("{old_lower}\\")) {
                    c.path = format!("{new_path}{}", &c.path[old_path.len()..]);
                }
            }
        }
    }

    /// Posición del cursor dentro de la ventana (en puntos de egui), leída de Windows:
    /// durante un arrastre desde otra app la ventana no recibe movimientos del ratón.
    fn cursor_in_window(&self, ctx: &egui::Context) -> Option<egui::Pos2> {
        let hwnd = self.hwnd?;
        let (x, y) = shell::cursor_pos();
        let mut p = windows::Win32::Foundation::POINT { x, y };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut p);
        }
        let ppp = ctx.pixels_per_point();
        Some(egui::pos2(p.x as f32 / ppp, p.y as f32 / ppp))
    }

    /// El menú de clic derecho: una ventana para el menú y otra para cada submenú abierto,
    /// junto al cursor y dentro del área útil de su monitor (sale por fuera de Rayo).
    fn show_menu(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use super::ctxmenu::{self, Row};
        let Some(mut m) = self.menu.take() else { return };
        let t = self.t;
        let now = ctx.input(|i| i.time);
        let (paths, background) = m.req.shell_key();
        let native = self.shell_menus.as_ref().and_then(|s| s.get(&paths, background));
        let loading = native.is_none() && self.shell_menus.is_some();
        let groups: Vec<String> = self.cfg.groups.iter().map(|g| g.name.clone()).collect();
        let hidden = self.cfg.menu_hidden.clone();
        let pins = self.cfg.menu_pins.clone();
        let b = ctxmenu::BuildCx { groups: &groups, split: self.cfg.split, native: native.as_deref(), hidden: &hidden, show_hidden: m.show_hidden };
        let (rayo, natives, hidden_count) = ctxmenu::build(&m.req, &b);
        let natives = natives.unwrap_or_default();
        let ppp = ctx.pixels_per_point();
        let wa = super::win::work_area_at(m.anchor.0, m.anchor.1);
        let area = egui::Rect::from_min_max(egui::pos2(wa.left as f32 / ppp, wa.top as f32 / ppp), egui::pos2(wa.right as f32 / ppp, wa.bottom as f32 / ppp));
        let anchor = egui::pos2(m.anchor.0 as f32 / ppp, m.anchor.1 as f32 / ppp);

        // Niveles: el menú y los submenús abiertos (cada uno con sus filas).
        let mut levels: Vec<(Vec<Row>, egui::Pos2, egui::Vec2)> = Vec::new();
        let rows0 = ctxmenu::top_rows(&rayo, &natives, &pins, &m.query, loading, hidden_count, m.show_hidden);
        let mut size0 = ctxmenu::measure(ctx, &rows0, true);
        size0.y = size0.y.min(area.height() - 8.0);
        // Hacia abajo y a la derecha del cursor; si no entra, hacia arriba / a la izquierda.
        let mut p0 = anchor;
        if p0.x + size0.x > area.right() {
            p0.x = (anchor.x - size0.x).max(area.left());
        }
        if p0.y + size0.y > area.bottom() {
            p0.y = if anchor.y - size0.y >= area.top() { anchor.y - size0.y } else { (area.bottom() - size0.y).max(area.top()) };
        }
        levels.push((rows0, p0, size0));
        for (depth, &(k, row_top)) in m.subs.clone().iter().enumerate() {
            let (parent_rows, ppos, psize) = (&levels[depth].0, levels[depth].1, levels[depth].2);
            let children = match parent_rows.get(k) {
                Some(Row::Item { e, .. }) if !e.children.is_empty() => &e.children,
                _ => {
                    m.subs.truncate(depth);
                    break;
                }
            };
            let rows = ctxmenu::sub_rows(children, &pins);
            let mut size = ctxmenu::measure(ctx, &rows, false);
            size.y = size.y.min(area.height() - 8.0);
            let mut p = egui::pos2(ppos.x + psize.x - 3.0, ppos.y + row_top - ctxmenu::PAD);
            if p.x + size.x > area.right() {
                p.x = (ppos.x - size.x + 3.0).max(area.left());
            }
            if p.y + size.y > area.bottom() {
                p.y = (area.bottom() - size.y).max(area.top());
            }
            levels.push((rows, p, size));
        }

        let mut close = false;
        let mut any_focus = false;
        let mut run = None;
        for depth in 0..levels.len() {
            let (rows, pos, size) = (&levels[depth].0, levels[depth].1, levels[depth].2);
            let title = format!("rayo-menu-{depth}");
            let open_sub = m.subs.get(depth).map(|s| s.0);
            let first = depth == 0 && now - m.opened < 0.05;
            let builder = egui::ViewportBuilder::default()
                .with_title(title.clone())
                .with_decorations(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_always_on_top()
                .with_active(depth == 0)
                .with_position(pos)
                .with_inner_size(size);
            let query = &mut m.query;
            let (out, focused, esc) = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(&title), builder, |vctx, _class| {
                let mut out = ctxmenu::LevelOut::default();
                egui::CentralPanel::default().frame(Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(ctxmenu::PAD)).show(vctx, |ui| {
                    let enter = if depth == 0 { ctxmenu::search_box(ui, &t, query, first) } else { false };
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        out = ctxmenu::draw_rows(ui, &t, rows, open_sub, enter);
                    });
                });
                let focused = vctx.input(|i| i.viewport().focused.unwrap_or(false));
                let esc = vctx.input(|i| i.key_pressed(egui::Key::Escape));
                (out, focused, esc)
            });
            if m.styled & (1 << depth) == 0 && super::win::style_menu_window(&title, depth == 0) {
                m.styled |= 1 << depth;
            }
            any_focus |= focused;
            close |= esc;
            if let Some((k, rect)) = out.hover_sub {
                if m.subs.get(depth).map(|s| s.0) != Some(k) {
                    m.subs.truncate(depth);
                    m.subs.push((k, rect.top()));
                    m.styled &= (1u32 << (depth + 1)) - 1;
                }
            } else if out.hover_leaf {
                m.subs.truncate(depth);
                m.styled &= (1u32 << (depth + 1)) - 1;
            }
            if let Some(key) = out.toggle_pin {
                actions.push(Action::TogglePin(key));
            }
            if let Some(key) = out.toggle_hidden {
                match self.cfg.menu_hidden.iter().position(|k| *k == key) {
                    Some(i) => {
                        self.cfg.menu_hidden.remove(i);
                    }
                    None => self.cfg.menu_hidden.push(key),
                }
                self.save_cfg();
            }
            if out.toggle_show_hidden {
                m.show_hidden ^= true;
            }
            if out.run.is_some() {
                run = out.run;
            }
        }
        // Se cierra al elegir algo, con Esc o al hacer clic en cualquier otro lado.
        if let Some(r) = run {
            close = true;
            match r {
                ctxmenu::Run::Cmd(id, t) => actions.push(Action::Command(id, t)),
                ctxmenu::Run::Act(a) => actions.push(a),
                ctxmenu::Run::Shell(token, id) => actions.push(Action::ShellInvoke(token, id)),
            }
        }
        if !any_focus && now - m.opened > 0.35 {
            close = true;
        }
        if !close {
            self.menu = Some(m);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Instala (o actualiza) y vuelve a abrirse desde la carpeta de instalación.
    fn install_and_restart(&mut self, ctx: &egui::Context) {
        match crate::install::install().and_then(|_| crate::install::launch_installed()) {
            Ok(()) => {
                self.cfg.portable = false;
                self.save_cfg();
                crate::instance::shutdown();
                std::process::exit(0);
            }
            Err(e) => self.toast(ctx, tf!("app.no_pudo_instalar", error = e)),
        }
    }

    /// La primera vez (sin instalar): instalar en el usuario o usarlo así, portátil.
    fn first_run_dialog(&mut self, ctx: &egui::Context) {
        use egui::RichText;
        let t = self.t;
        let mut choice: Option<bool> = None;
        egui::Modal::new(Id::new("first-run")).show(ctx, |ui| {
            ui.set_width(500.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(egui_phosphor::fill::FOLDER).font(theme::icon_font(22.0, true)).color(t.accent));
                ui.label(RichText::new(t!("app.bienvenido_rayo")).font(theme::semibold(18.0)).color(t.text));
            });
            ui.add_space(8.0);
            ui.label(RichText::new(t!("app.instalas_queda_menu_inicio_aplicaciones")).color(t.dim));
            ui.add_space(6.0);
            ui.label(RichText::new(t!("app.sin_instalar_funciona_igual_como")).size(12.5).color(t.faint));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new(t!("app.instalar")).strong()).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(t!("app.usar_sin_instalar")).clicked() {
                        choice = Some(false);
                    }
                });
            });
        });
        match choice {
            Some(true) => {
                self.first_run = false;
                self.install_and_restart(ctx);
            }
            Some(false) => {
                self.first_run = false;
                self.cfg.portable = true;
                self.save_cfg();
            }
            None => {}
        }
    }

    /// "Continuar como administrador": repite solo la operación que Windows negó.
    fn admin_dialog(&mut self, ctx: &egui::Context) {
        use egui::RichText;
        let t = self.t;
        let Some((why, op)) = self.admin.clone() else { return };
        let mut close = false;
        let mut go = false;
        let modal = egui::Modal::new(Id::new("admin-op")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(egui_phosphor::regular::SHIELD_CHECK).font(theme::icon_font(20.0, true)).color(t.accent));
                ui.label(RichText::new(t!("app.hace_falta_permiso_administrador")).font(theme::semibold(17.0)).color(t.text));
            });
            ui.add_space(6.0);
            ui.label(RichText::new(&why).color(t.dim));
            ui.label(RichText::new(op.describe()).color(t.text));
            ui.add_space(4.0);
            ui.label(RichText::new(t!("app.windows_va_pedir_permiso_hara")).size(12.5).color(t.faint));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new(format!("{}  {}", egui_phosphor::regular::SHIELD_CHECK, t!("app.continuar_como_administrador"))).strong()).clicked() {
                        go = true;
                    }
                    if ui.button(t!("comun.cancelar")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if modal.should_close() {
            close = true;
        }
        if go {
            close = true;
            let (mgr, result) = (self.mgr.clone(), self.service_result.clone());
            std::thread::spawn(move || {
                let msg = match crate::admin::run_elevated(&op) {
                    Ok(()) => t!("app.listo_como_administrador").to_string(),
                    Err(e) => tf!("app.no_pudo", error = e),
                };
                mgr.dirty.lock().extend(op.touched_dirs().iter().map(|d| d.to_lowercase()));
                *result.lock() = Some(msg);
                mgr.wake_ui();
            });
        }
        if close {
            self.admin = None;
        }
    }

    /// Activar o desactivar Rayo como explorador predeterminado, explicando qué cambia.
    /// Cambia el idioma al momento (None: el de Windows) y lo recuerda.
    fn set_language(&mut self, code: Option<String>) {
        crate::i18n::init(code.as_deref());
        self.cfg.lang = code;
        self.save_cfg();
        // Lo que se armó con el idioma anterior.
        self.places = fsx::known_places();
    }

    fn default_dialog(&mut self, ctx: &egui::Context) {
        use egui::RichText;
        let t = self.t;
        let on = self.cfg.default_explorer;
        let mut autostart = self.default_dlg.unwrap_or(true);
        let mut close = false;
        let mut apply = false;
        let modal = egui::Modal::new(Id::new("default-explorer")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(egui_phosphor::regular::FOLDER_OPEN).font(theme::icon_font(20.0, true)).color(t.accent));
                ui.label(RichText::new(if on { t!("app.rayo_tu_explorador_predeterminado") } else { t!("app.usar_rayo_como_explorador_predeterminado") }).font(theme::semibold(18.0)).color(t.text));
            });
            ui.add_space(8.0);
            let line = |ui: &mut Ui, s: &str| {
                ui.label(RichText::new(format!("•  {s}")).color(t.dim));
            };
            if on {
                ui.label(RichText::new(t!("app.volver_explorador_windows_todo_queda")).color(t.dim));
            } else {
                ui.label(RichText::new(t!("app.abriran_rayo")).color(t.text));
                line(ui, t!("app.carpetas_unidades_archivos_zip_abras"));
                line(ui, t!("app.mostrar_carpeta_navegador_abrir_ubicacio"));
                ui.add_space(6.0);
                ui.label(RichText::new(t!("app.siguen_explorador_panel_control_papelera")).color(t.dim));
                ui.add_space(6.0);
                ui.label(RichText::new(t!("app.solo_cambia_tu_usuario_no")).size(12.5).color(t.faint));
                ui.add_space(8.0);
                if !self.cfg.start_with_windows {
                    ui.checkbox(&mut autostart, t!("app.iniciar_rayo_windows_oculto_mostrar"));
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if on { t!("app.volver_explorador_windows") } else { t!("app.activar") }).clicked() {
                        apply = true;
                    }
                    if ui.button(t!("comun.cancelar")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if modal.should_close() {
            close = true;
        }
        self.default_dlg = Some(autostart);
        if apply {
            close = true;
            let r = if on { crate::integration::disable(&crate::integration::REAL) } else { crate::integration::enable(&crate::integration::REAL) };
            match r {
                Ok(()) => {
                    self.cfg.default_explorer = !on;
                    crate::integration::watch(!on);
                    if !on {
                        // El vigía necesita que Rayo siga vivo al cerrar la ventana.
                        self.cfg.stay_resident = true;
                        if autostart && !self.cfg.start_with_windows && crate::instance::set_autostart(true).is_ok() {
                            self.cfg.start_with_windows = true;
                        }
                    }
                    self.save_cfg();
                    self.toast(ctx, if on { t!("app.listo_carpetas_vuelven_abrirse_explorado") } else { t!("app.listo_rayo_tu_explorador_predeterminado") });
                }
                Err(e) => {
                    // Si falló a mitad de camino, deshacer lo que se haya escrito.
                    if !on {
                        let _ = crate::integration::disable(&crate::integration::REAL);
                    }
                    self.toast(ctx, tf!("app.no_pudo_cambiar", error = e));
                }
            }
        }
        if close {
            self.default_dlg = None;
        }
    }

    /// Historial de búsquedas: se guarda lo que quedó escrito un momento (lo que se
    /// buscó de verdad, no cada letra). Si se siguió escribiendo lo mismo, reemplaza a la anterior.
    fn track_search_history(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let s = &mut self.tabs[self.cur].search;
        let text = s.text.trim().to_string();
        if text != s.hist_seen {
            if text.is_empty() {
                s.hist_recorded.clear();
            }
            s.hist_seen = text;
            s.hist_since = now;
            return;
        }
        if text.chars().count() < 2 || text.to_lowercase().starts_with("recientes:") || s.hist_recorded == text {
            return;
        }
        let waited = now - s.hist_since;
        if waited < 1.5 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.55 - waited));
            return;
        }
        let prev = std::mem::replace(&mut s.hist_recorded, text.clone());
        let h = &mut self.cfg.search_history;
        let (tl, pl) = (text.to_lowercase(), prev.to_lowercase());
        if !prev.is_empty() && h.first().is_some_and(|f| *f == prev) && (tl.starts_with(&pl) || pl.starts_with(&tl)) {
            h.remove(0);
        }
        h.retain(|q| !q.eq_ignore_ascii_case(&text));
        h.insert(0, text);
        h.truncate(25);
        self.save_cfg();
    }

    /// Arrastrar hacia otra app: Windows se encarga hasta que se suelta.
    fn run_drag_out(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.drag_out.take() else { return };
        let Some(h) = self.hwnd else { return };
        // Desde un comprimido se arrastran copias temporales.
        let paths: Vec<String> = paths.iter().map(|p| if fsx::archive::is_inside(p) { fsx::archive::materialize(p).unwrap_or_default() } else { p.clone() }).filter(|p| !p.is_empty()).collect();
        shell::drag_out(h, &paths);
        // El botón se soltó dentro del arrastre de Windows: avisarle a la ventana para que egui lo sepa.
        let (x, y) = shell::cursor_pos();
        let mut p = windows::Win32::Foundation::POINT { x, y };
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP};
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(h, &mut p);
            let lp = ((p.y as u32 & 0xFFFF) << 16) | (p.x as u32 & 0xFFFF);
            let _ = PostMessageW(Some(h), WM_LBUTTONUP, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(lp as isize));
        }
        // El destino pudo mover los archivos: recargar sus carpetas.
        self.mgr.dirty.lock().extend(paths.iter().filter_map(|p| fsx::parent_of(p)).map(|d| d.to_lowercase()));
        self.mgr.wake_ui();
        ctx.request_repaint();
    }

    /// Ctrl+Z: revierte la última operación.
    fn undo(&mut self, ctx: &egui::Context) {
        let Some(u) = self.undo.pop() else {
            self.toast(ctx, t!("app.no_hay_nada_deshacer"));
            return;
        };
        let reload_parents = |mgr: &Arc<IndexManager>, paths: &[String]| {
            mgr.dirty.lock().extend(paths.iter().filter_map(|p| fsx::parent_of(p)).map(|d| d.to_lowercase()));
            mgr.wake_ui();
        };
        match u {
            Undo::Rename { dir, old, new } => {
                let cur = fsx::join(&dir, &new);
                match shell::rename(&cur, &old) {
                    Ok(back) => {
                        self.dirs.reload(&dir);
                        self.retarget_cols(&cur, &back);
                        self.toast(ctx, tf!("app.deshecho_vuelve_llamarse", nuevo = new, viejo = old));
                    }
                    Err(e) => self.toast(ctx, tf!("app.no_pudo_deshacer", error = e)),
                }
            }
            Undo::Batch { dir, pairs } => {
                let plan: Vec<fsx::rename::Planned> = pairs.iter().map(|(o, n)| fsx::rename::Planned { old: n.clone(), new: o.clone(), problem: None }).collect();
                let (done, errors) = fsx::rename::apply(&dir, &plan);
                self.dirs.reload(&dir);
                match errors.first() {
                    None => self.toast(ctx, tf!("app.deshecho_nombres_restaurados", n = done.len())),
                    Some(e) => self.toast(ctx, tf!("app.no_pudieron_restaurar", n = errors.len(), error = e)),
                }
            }
            Undo::Recycle(paths) => {
                self.toast(ctx, t!("app.sacando_papelera"));
                let (mgr, result) = (self.mgr.clone(), self.service_result.clone());
                std::thread::spawn(move || {
                    let n = shell::restore_from_recycle_bin(&paths);
                    *result.lock() = Some(if n == 0 { t!("app.no_encontro_papelera").to_string() } else { tp!(n, "app.deshecho_restaurados") });
                    reload_parents(&mgr, &paths);
                });
            }
            Undo::Copied(paths) => {
                let n = paths.len();
                let mgr = self.mgr.clone();
                let p2 = paths.clone();
                shell::file_op(FileOp::Recycle, paths, None, move |_| reload_parents(&mgr, &p2));
                self.toast(ctx, tp!(n, "app.deshecho_copias_papelera"));
            }
            Undo::Moved(pairs) => {
                let n = pairs.len();
                self.xfer.enqueue_moves(pairs.into_iter().map(|(from, to)| (to, from)).collect());
                self.toast(ctx, tp!(n, "app.deshaciendo_devolviendo"));
            }
            Undo::NewFolder(p) => {
                let empty = std::fs::read_dir(fsx::long_path(&p)).map(|mut d| d.next().is_none()).unwrap_or(false);
                if empty {
                    let mgr = self.mgr.clone();
                    let p2 = vec![p.clone()];
                    shell::file_op(FileOp::Recycle, vec![p], None, move |_| reload_parents(&mgr, &p2));
                    self.toast(ctx, t!("app.deshecho_carpeta_nueva_eliminada"));
                } else {
                    self.toast(ctx, t!("app.carpeta_nueva_ya_tiene_archivos"));
                }
            }
        }
    }

    /// Vista y orden de cada pestaña visible: al entrar a una carpeta se usa lo que se
    /// eligió para ella (o lo predeterminado). La configuración refleja la pestaña enfocada.
    fn resolve_views(&mut self) {
        let panes = if self.cfg.split { 2 } else { 1 };
        for p in 0..panes {
            let ti = self.pane[p];
            let tab = &self.tabs[ti];
            if tab.view.is_some() && tab.view_seq == tab.nav_seq {
                continue;
            }
            let key = tab.path().to_lowercase();
            let mut v = self.cfg.folder_views.iter().rev().find(|(k, _)| *k == key).map(|x| x.1).unwrap_or(self.cfg.default_view);
            if tab.flat && v.view == ViewMode::Columns {
                v.view = ViewMode::Details;
            }
            let tab = &mut self.tabs[ti];
            tab.view = Some(v);
            tab.view_seq = tab.nav_seq;
        }
        if let Some(v) = self.tabs[self.cur].view {
            self.cfg.view = v.view;
            self.cfg.sort = v.sort;
            self.cfg.sort_desc = v.desc;
        }
    }

    /// La vista y el orden actuales quedan recordados para la carpeta de la pestaña.
    fn remember_view(&mut self) {
        let v = super::config::FolderView { view: self.cfg.view, sort: self.cfg.sort, desc: self.cfg.sort_desc };
        let tab = &mut self.tabs[self.cur];
        tab.view = Some(v);
        tab.view_seq = tab.nav_seq;
        if tab.tool.is_some() || tab.search.active() {
            return;
        }
        let key = tab.path().to_lowercase();
        let list = &mut self.cfg.folder_views;
        list.retain(|(k, _)| *k != key);
        if v != self.cfg.default_view {
            list.push((key, v));
        }
        if list.len() > 3000 {
            list.drain(..list.len() - 3000);
        }
    }

    /// "Seleccionar por patrón": `*.jpg; *.png`, o un texto (lo que lo contenga).
    fn pattern_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let t = self.t;
        let (ci, _, list) = self.active_list();
        let Some((pat, add, first)) = self.pattern.as_mut() else { return };
        let pats: Vec<Vec<u8>> = pat.split([';', ',']).map(|p| crate::index::store::search_key(p.trim()).into_bytes()).filter(|p| !p.is_empty()).collect();
        let matches = |name: &str| {
            let n = crate::index::store::search_key(name).into_bytes();
            pats.iter().any(|p| {
                if p.contains(&b'*') || p.contains(&b'?') {
                    crate::index::search::glob_match(p, &n)
                } else {
                    memchr::memmem::find(&n, p).is_some()
                }
            })
        };
        let hits: Vec<String> = list.iter().filter(|e| matches(&e.name)).map(|e| e.name.clone()).collect();
        let mut close = false;
        let mut apply = false;
        let modal = egui::Modal::new(Id::new("select-pattern")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::ASTERISK).font(theme::icon_font(18.0, false)).color(t.accent));
                ui.label(egui::RichText::new(t!("app.seleccionar_patron")).font(theme::semibold(16.0)));
            });
            ui.add_space(8.0);
            let mut f = super::widgets::Field::new("patron", egui_phosphor::regular::ASTERISK, t!("app.jpg_png_texto"));
            f.tone = if !pats.is_empty() && hits.is_empty() { super::widgets::Tone::Bad } else { super::widgets::Tone::Normal };
            let r = super::widgets::field(ui, &t, pat, f).edit;
            if *first {
                r.request_focus();
                *first = false;
            }
            ui.add_space(4.0);
            let (msg, color) = if pats.is_empty() {
                (t!("app.usa_cualquier_texto_letra_separa").to_string(), t.faint)
            } else if hits.is_empty() {
                (t!("app.ninguno_coincide").to_string(), t.danger)
            } else {
                (if hits.len() == 1 { t!("app.1_coincide").to_string() } else { tf!("app.coinciden", n = hits.len()) }, t.dim)
            };
            ui.label(egui::RichText::new(msg).size(12.0).color(color));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                super::widgets::toggle_chip(ui, &t, add, egui_phosphor::regular::PLUS, t!("app.sumar_ya_seleccionado"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::widgets::primary_button(ui, &t, "", t!("app.seleccionar"), !hits.is_empty()).clicked() || (!hits.is_empty() && ui.input(|i| i.key_pressed(Key::Enter))) {
                        apply = true;
                    }
                    if super::widgets::secondary_button(ui, &t, "", t!("comun.cancelar")).clicked() {
                        close = true;
                    }
                });
            });
        });
        if apply {
            let mut names = hits;
            if *add {
                for n in self.tabs[self.cur].active_col().selected() {
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
            actions.push(Action::SetSelection { col: ci, names });
            close = true;
        }
        if close || modal.should_close() {
            self.pattern = None;
        }
    }

    /// Cerrar la ventana sin salir: se oculta, guarda todo y libera memoria. Volver a
    /// abrir Rayo (acceso directo, Win+R, el ícono de la bandeja) la muestra al instante.
    fn go_background(&mut self) {
        self.save_cfg();
        self.quicklook = false;
        self.palette.open = false;
        self.rename = None;
        self.batch = None;
        self.tex.clear();
        if let Some(h) = self.hwnd {
            super::win::hide(h);
        }
        crate::instance::set_tray(true);
        let mgr = self.mgr.clone();
        std::thread::spawn(move || {
            mgr.save_all();
            std::thread::sleep(std::time::Duration::from_millis(400));
            super::win::trim_memory();
        });
    }

    /// Antes del primer cuadro: pedir las carpetas visibles y esperar un momento la
    /// activa, para que la ventana aparezca ya con su contenido.
    fn warm_start(&mut self) {
        let panes = if self.cfg.split { 2 } else { 1 };
        for p in 0..panes {
            let paths: Vec<String> = self.tabs[self.pane[p]].cols.iter().map(|c| c.path.clone()).collect();
            for path in paths {
                self.dirs.request(&path);
            }
        }
        let active = self.tabs[self.cur].active_col().path.clone();
        let t0 = std::time::Instant::now();
        while t0.elapsed() < std::time::Duration::from_millis(40) {
            self.dirs.poll();
            if self.dirs.state(&active).is_some_and(|s| !s.loading) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Abre una herramienta en una pestaña nueva del panel enfocado.
    fn open_tool(&mut self, ctx: &egui::Context, dir: String, tool: super::tools::Tool) {
        self.apply(ctx, Action::NewTab(Some(dir)));
        self.tabs[self.cur].tool = Some(Box::new(tool));
    }

    /// Renombrado en lote: lo seleccionado (si son 2 o más) o todos los archivos de la carpeta.
    fn open_batch(&mut self, ctx: &egui::Context, t: &super::commands::Target) {
        let dir = t.dir.clone();
        if self.read_only(ctx, &dir) {
            return;
        }
        let list = self.view_of(&dir);
        let sel: std::collections::HashSet<String> =
            t.paths.iter().filter(|p| fsx::parent_of(p).is_some_and(|d| d.eq_ignore_ascii_case(&dir))).map(|p| fsx::file_name(p)).collect();
        let files: Vec<(String, u32)> = if sel.len() >= 2 {
            list.iter().filter(|e| sel.contains(&e.name)).map(|e| (e.name.clone(), e.mtime)).collect()
        } else {
            list.iter().filter(|e| !e.is_dir).map(|e| (e.name.clone(), e.mtime)).collect()
        };
        if files.is_empty() {
            self.toast(ctx, t!("app.no_hay_archivos_renombrar_aqui"));
            return;
        }
        // Todos los nombres de la carpeta (también los ocultos) para no pisar nada.
        let existing = match self.dirs.state(&dir).and_then(|s| s.raw.clone()) {
            Some(raw) => raw.iter().map(|e| e.name.to_lowercase()).collect(),
            None => fsx::list_dir(&dir).map(|l| l.into_iter().map(|e| e.lower).collect()).unwrap_or_default(),
        };
        self.batch = Some(super::tools::BatchRename::new(dir, files, existing));
    }

    /// F12: muestra el tiempo de CPU por cuadro y los cuadros por segundo.
    /// Mientras está visible repinta sin parar para poder medir.
    fn perf_overlay(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F12)) {
            self.perf = if self.perf.is_some() { None } else { Some(Default::default()) };
        }
        let Some(hist) = self.perf.as_mut() else { return };
        let now = ctx.input(|i| i.time);
        hist.push_back((now, frame.info().cpu_usage.unwrap_or(0.0) * 1000.0));
        while hist.front().is_some_and(|(t, _)| now - t > 1.0) {
            hist.pop_front();
        }
        let fps = hist.len();
        let worst = hist.iter().map(|h| h.1).fold(0.0f32, f32::max);
        let avg = hist.iter().map(|h| h.1).sum::<f32>() / fps.max(1) as f32;
        let t = self.t;
        egui::Area::new(Id::new("perf")).order(egui::Order::Tooltip).anchor(egui::Align2::RIGHT_TOP, vec2(-12.0, 92.0)).show(ctx, |ui| {
            Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(10).show(ui, |ui| {
                ui.label(egui::RichText::new(tf!("app.cuadros_s", fps = fps)).font(theme::semibold(14.0)).color(t.accent));
                ui.label(egui::RichText::new(tf!("app.cpu_cuadro_ms_peor_ms", promedio = format!("{:.2}", avg), peor = format!("{:.2}", worst))).size(12.0).color(t.text));
                ui.label(egui::RichText::new(tf!("app.presupuesto_144_hz_ms", ms = format!("{:.2}", 1000.0 / 144.0))).size(11.0).color(t.dim));
            });
        });
        ctx.request_repaint();
    }

    fn run_native_menu(&mut self, ctx: &egui::Context) {
        let Some((paths, pos, frames)) = self.pending_menu.take() else { return };
        // Dos cuadros: uno para cerrar el menú propio y otro para pintarlo cerrado.
        if frames < 2 {
            self.pending_menu = Some((paths, pos, frames + 1));
            ctx.request_repaint();
            return;
        }
        let Some(h) = self.hwnd else { return };
        let shift = ctx.input(|i| i.modifiers.shift);
        match shell::native_context_menu(h, &paths, pos, shift) {
            shell::MenuOutcome::Rename => {
                if let Some(p) = paths.first() {
                    if let Some(dir) = fsx::parent_of(p) {
                        self.apply(ctx, Action::StartRename(dir, fsx::file_name(p)));
                    }
                }
            }
            shell::MenuOutcome::Invoked => {
                for p in &paths {
                    if let Some(d) = fsx::parent_of(p) {
                        self.dirs.reload(&d);
                    }
                }
            }
            shell::MenuOutcome::None => {}
        }
    }

    // ------------------------------------------------------------ paneles

    /// Un panel: pestañas, barra (atrás, ruta, vistas, búsqueda) y contenido.
    fn render_pane(&mut self, ui: &mut Ui, p: usize, now: u32, actions: &mut Vec<Action>) {
        // El otro panel se dibuja con la vista y el orden de su propia pestaña.
        let saved = (self.cfg.view, self.cfg.sort, self.cfg.sort_desc);
        if let Some(v) = self.tabs[self.pane[p]].view.filter(|_| p != self.focus) {
            (self.cfg.view, self.cfg.sort, self.cfg.sort_desc) = (v.view, v.sort, v.desc);
        }
        self.render_pane_inner(ui, p, now, actions);
        (self.cfg.view, self.cfg.sort, self.cfg.sort_desc) = saved;
    }

    fn render_pane_inner(&mut self, ui: &mut Ui, p: usize, now: u32, actions: &mut Vec<Action>) {
        let t = self.t;
        let rect = ui.max_rect();
        let focused = p == self.focus;
        let ti = self.pane[p];

        let tabs_rect = egui::Rect::from_min_size(rect.min, vec2(rect.width(), chrome::TITLE_H));
        ui.painter().rect_filled(tabs_rect, 0.0, t.chrome);
        let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(tabs_rect));
        let show = self.tabs_of(p);
        // El panel de la derecha (sin vista previa) tiene arriba los botones de la ventana.
        let rightmost = p == if self.cfg.split { 1 } else { 0 };
        let reserve = if rightmost && !self.cfg.show_preview { chrome::WINDOW_BUTTONS_W } else { 0.0 };
        let plus_end = chrome::tab_bar(&mut tui, &t, &self.tabs, &show, ti, reserve, actions);
        // Lo que sobra a la derecha de las pestañas es barra de título (arrastrar, doble clic).
        let free = egui::Rect::from_min_max(egui::pos2(plus_end + 4.0, tabs_rect.top()), egui::pos2(tabs_rect.right() - reserve, tabs_rect.bottom()));
        if free.width() > 4.0 {
            self.caption.push(free);
        }

        let bar_h = if self.tabs[ti].tool.is_some() { 0.0 } else { 46.0 };
        let bar_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), tabs_rect.bottom()), vec2(rect.width(), bar_h));
        let mut bui = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let cfg_snapshot = self.cfg.clone();
        let search_id = Id::new(("search", self.tabs[ti].id));
        if bar_h > 0.0 {
            chrome::toolbar(&mut bui, &t, &cfg_snapshot, &mut self.tabs[ti], search_id, actions);
        }
        ui.painter().line_segment([bar_rect.left_bottom(), bar_rect.right_bottom()], egui::Stroke::new(1.0, t.border));
        // Con dos paneles, una línea de acento marca el panel activo.
        if self.cfg.split && focused {
            ui.painter().line_segment([rect.left_top() + vec2(0.0, 1.0), rect.right_top() + vec2(0.0, 1.0)], egui::Stroke::new(2.0, t.accent));
        }

        let content = egui::Rect::from_min_max(egui::pos2(rect.left(), bar_rect.bottom() + 1.0), rect.max);
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(egui::Layout::top_down(egui::Align::Min)));
        cui.set_clip_rect(content);
        self.pane_content(&mut cui, ti, focused, now, actions);
    }

    /// Contenido de una pestaña: resultados de búsqueda, columnas, detalles o cuadrícula.
    fn pane_content(&mut self, ui: &mut Ui, ti: usize, focused: bool, now: u32, actions: &mut Vec<Action>) {
        let t = self.t;
        let other = self.other_path();
        let App { dirs, tabs, cfg, rename, cut, mgr, tex, xfer, .. } = self;
        let tab = &mut tabs[ti];
        if let Some(tool) = tab.tool.as_mut() {
            let mut tcx = super::tools::ToolCx { t: &t, actions, xfer, mgr, other };
            tool.ui(ui, &mut tcx);
            return;
        }
        let groups: Vec<String> = cfg.groups.iter().map(|g| g.name.clone()).collect();
        let mut cx = Cx { t: &t, now, actions, rename, cut, tab_id: tab.id, groups: &groups, space_bars: cfg.space_bars, split: cfg.split };
        if tab.search.active() {
            let indexing = if !mgr.is_ready() {
                Some(t!("app.indice_esta_construyendo").to_string())
            } else if mgr.building() {
                Some(t!("app.actualizando_indice").to_string())
            } else {
                None
            };
            ui.add_space(8.0);
            let query = tab.search.text.clone();
            let s = &mut tab.search;
            views::search_results(ui, &mut cx, s.results.as_ref(), &mut s.cursor, &mut s.scroll_to_cursor, indexing, &query, (s.sort, s.desc));
            return;
        }
        let (sort, desc, hidden) = (cfg.sort, cfg.sort_desc, cfg.show_hidden);
        // Filtro escrito sobre la carpeta enfocada (solo en la columna activa).
        let filter_key = crate::index::store::search_key(tab.filter.trim());
        let active_col = tab.active.min(tab.cols.len() - 1);
        let entries_of = |dirs: &mut DirCache, ci: usize, path: &str| -> Arc<Vec<FileEntry>> {
            let v = dirs.view(path, sort, desc, hidden);
            if filter_key.is_empty() || ci != active_col {
                v
            } else {
                Arc::new(v.iter().filter(|e| crate::index::store::search_key(&e.name).contains(&filter_key)).cloned().collect())
            }
        };
        let filter_bar = |ui: &mut Ui, tab: &mut Tab| {
            if tab.filter.is_empty() {
                return;
            }
            views::filter_chip(ui, &t, &tab.filter);
        };
        match cfg.view {
            ViewMode::Columns => {
                // Si la última columna tiene una carpeta enfocada, mostrar su contenido.
                if let Some(last) = tab.cols.last() {
                    if let Some(c) = last.cursor.clone() {
                        let v = dirs.view(&last.path, sort, desc, hidden);
                        let is_dir = v.iter().any(|e| e.name == c && e.is_dir);
                        tab.ensure_child_col(is_dir);
                    }
                }
                filter_bar(ui, tab);
                let w = cfg.col_width;
                egui::ScrollArea::horizontal().id_salt(("hcols", tab.id)).auto_shrink([false, false]).stick_to_right(true).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let h = ui.available_height();
                        let n = tab.cols.len();
                        let active = tab.active.min(n - 1);
                        for ci in 0..n {
                            let path = tab.cols[ci].path.clone();
                            let entries = entries_of(dirs, ci, &path);
                            let st = dirs.state(&path);
                            let loading = st.is_some_and(|s| s.loading);
                            let error = st.and_then(|s| s.error.clone());
                            let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(0.0, 4.0))).layout(egui::Layout::top_down(egui::Align::Min)));
                            views::column(&mut child, &mut cx, ci, &mut tab.cols[ci], focused && ci == active, ColumnData { entries: &entries, loading, error: error.as_deref() });
                            ui.painter().line_segment([rect.right_top(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
                        }
                    });
                });
            }
            ViewMode::Details | ViewMode::Grid => {
                let ci = tab.cols.len() - 1;
                tab.active = ci;
                let path = tab.cols[ci].path.clone();
                let key = if tab.flat { format!("{}{path}", fsx::FLAT_KEY) } else { path.clone() };
                let entries = entries_of(dirs, ci, &key);
                let st = dirs.state(&key);
                if tab.flat {
                    views::flat_chip(ui, &t, entries.len(), st.is_some_and(|s| s.loading), cx.actions);
                }
                let loading = st.is_some_and(|s| s.loading);
                let error = st.and_then(|s| s.error.clone());
                filter_bar(ui, tab);
                let data = ColumnData { entries: &entries, loading, error: error.as_deref() };
                if cfg.view == ViewMode::Grid {
                    views::grid(ui, &mut cx, ci, &mut tab.cols[ci], focused, data, tex);
                } else {
                    views::details(ui, &mut cx, ci, &mut tab.cols[ci], focused, data, (sort, desc));
                }
            }
        }
    }

    // ------------------------------------------------------------ elemento enfocado

    fn focused_item(&mut self) -> Option<PreviewItem> {
        let tab = &self.tabs[self.cur];
        if tab.search.active() {
            let row = tab.search.results.as_ref()?.row(tab.search.cursor)?;
            let path = row.path();
            let entry = FileEntry {
                lower: row.name.to_lowercase(),
                ext: row.ext.clone(),
                is_dir: row.is_dir,
                hidden: false,
                size: if row.is_dir { 0 } else { row.size },
                dir_size: if row.is_dir { Some(row.size) } else { None },
                mtime: row.mtime,
                ctime: 0,
                cloud: if !row.is_dir && fsx::is_online_only(&path) { fsx::Cloud::OnlineOnly } else { fsx::Cloud::None },
                name: row.name,
            };
            let children = if row.is_dir { self.dirs.state(&path).and_then(|s| s.raw.as_ref().map(|r| r.len())) } else { None };
            return Some(PreviewItem { path, real: None, entry, children });
        }
        let (dir, name) = tab.focused()?;
        let entry = self.entry_in(&dir, &name)?;
        let path = fsx::join(&dir, &name);
        let children = if entry.is_dir { self.dirs.state(&path).and_then(|s| s.raw.as_ref().map(|r| r.iter().filter(|e| self.cfg.show_hidden || !e.hidden).count())) } else { None };
        let real = if entry.is_dir { None } else { self.extracted_copy(&path, entry.size) };
        Some(PreviewItem { path, real, entry, children })
    }

    /// Copia temporal (en segundo plano) de un archivo de adentro de un comprimido, para verlo.
    fn extracted_copy(&mut self, path: &str, size: u64) -> Option<String> {
        if size > 64 << 20 || !fsx::archive::is_inside(path) {
            return None;
        }
        let mut m = self.extracted.lock();
        match m.get(path) {
            Some(Some(done)) => return done.clone(),
            Some(None) => return None,
            None => {}
        }
        m.insert(path.to_string(), None);
        drop(m);
        let (map, p, mgr) = (self.extracted.clone(), path.to_string(), self.mgr.clone());
        std::thread::spawn(move || {
            let r = fsx::archive::materialize(&p);
            map.lock().insert(p, Some(r));
            mgr.wake_ui();
        });
        None
    }

    fn text_for(&mut self, item: Option<&PreviewItem>) -> Option<String> {
        let item = item?;
        if !preview::wants_text(&item.entry) {
            return None;
        }
        if self.text_cache.as_ref().is_none_or(|(p, _)| p != item.source()) {
            let local = self.mgr.slot_for(item.source()).is_some();
            let txt = if local { preview::read_text_snippet(item.source(), 64 * 1024) } else { None };
            self.text_cache = Some((item.source().to_string(), txt));
        }
        self.text_cache.as_ref().and_then(|(_, t)| t.clone())
    }

    fn status(&mut self) -> (String, String, bool) {
        let tab = &self.tabs[self.cur];
        let left = if tab.search.active() {
            String::new()
        } else {
            let col = tab.active_col().clone();
            let list = self.view_of(&col.path);
            let mut s = tp!(list.len(), "comun.elementos_n", n = fsx::thousands(list.len()));
            let sel = col.selected();
            if !sel.is_empty() {
                let size: u64 = list.iter().filter(|e| sel.contains(&e.name)).filter_map(|e| e.display_size()).sum();
                s.push_str(&format!("  ·  {}  ·  {}", tp!(sel.len(), "app.seleccionados"), fsx::human_size(size)));
            }
            s
        };
        let mut busy = false;
        let mut parts = Vec::new();
        let mut total = 0;
        for slot in &self.mgr.slots {
            let st = slot.state.lock();
            if st.building {
                busy = true;
                parts.push(tf!("app.indexando", letra = slot.letter, n = fsx::thousands(slot.progress.load(Ordering::Relaxed))));
            }
            total += st.entries;
        }
        let right = if busy {
            parts.join("  ")
        } else {
            let mode = self.mgr.mode();
            let how = if mode.starts_with("USN") { tf!("app.al_dia_por", modo = crate::index::mode_label(&mode)) } else { t!("app.vigilando_cambios").to_string() };
            tf!("app.estado_archivos", n = fsx::thousands(total), como = how)
        };
        (left, right, busy)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.dirs.frame += 1;
        if matches!(self.dirs.frame, 1 | 2 | 10 | 60 | 300) {
            crate::index::mem_log(&format!("ui: cuadro {}", self.dirs.frame));
        }
        // Tras mostrarse (al arrancar o al volver de segundo plano) se repinta unos cuadros
        // seguidos: si el primero no llega a la pantalla, la ventana no queda en blanco.
        if super::win::bursting() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if crate::instance::SHOWN_AGAIN.swap(false, Ordering::Relaxed) {
            // De vuelta de segundo plano: lo visible pudo cambiar.
            for p in 0..if self.cfg.split { 2 } else { 1 } {
                let paths: Vec<String> = self.tabs[self.pane[p]].cols.iter().map(|c| c.path.clone()).collect();
                for path in paths {
                    self.dirs.reload(&path);
                }
            }
        }
        // Cerrar la ventana: quedarse en segundo plano (salvo que se pida salir).
        if ctx.input(|i| i.viewport().close_requested()) && self.cfg.stay_resident && !crate::instance::quit_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.go_background();
        }
        let now = now_unix();
        let t = self.t;
        let mut actions: Vec<Action> = Vec::new();
        self.poll(&ctx);
        // Rutas que pidió abrir otra instancia de Rayo.
        for p in crate::instance::take_requests() {
            if p.is_empty() {
                continue;
            }
            // Si esa carpeta ya está abierta en una pestaña, se va a ella (no se apilan pestañas iguales).
            // Un comprimido se abre por dentro, como una carpeta.
            let is_dir = std::path::Path::new(&p).is_dir() || fsx::archive::split(&p).is_some();
            let dir = if is_dir { fsx::normalize(&p) } else { fsx::parent_of(&p).unwrap_or_default() };
            match self.tabs.iter().position(|t| t.tool.is_none() && t.search.text.is_empty() && t.path().eq_ignore_ascii_case(&dir)) {
                Some(i) => actions.push(Action::SwitchTab(i)),
                None => actions.push(Action::NewTab(Some(dir))),
            }
            if !is_dir {
                actions.push(Action::Reveal(p));
            }
        }
        // Clic derecho fuera de un menú abierto: cerrarlo para que se abra el nuevo.
        let right_press = ctx.input(|i| if i.pointer.secondary_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(pos) = right_press {
            if egui::Popup::is_any_open(&ctx) && ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background) {
                egui::Popup::close_all(&ctx);
            }
        }
        // Un clic dentro de un panel lo enfoca (antes de procesar acciones y teclas).
        if self.cfg.split {
            let press = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
            // Solo clics sobre los paneles mismos (no sobre diálogos, menús o la paleta encima).
            let press = press.filter(|&pos| ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background));
            if let Some(pos) = press {
                if let Some(p) = (0..2).find(|&p| self.pane_rects[p].contains(pos)) {
                    if p != self.focus {
                        self.set_focus(p);
                    }
                }
            }
        } else if self.focus != 0 {
            self.set_focus(0);
        }
        // Archivos arrastrados desde otra aplicación.
        let (ext_hovering, ext_dropped): (bool, Vec<String>) =
            ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.iter().map(|f| f.path().to_string_lossy().into_owned()).collect()));
        if ext_hovering || !ext_dropped.is_empty() {
            let pos = self.cursor_in_window(&ctx);
            ctx.data_mut(|d| {
                let e = d.get_temp_mut_or_default::<views::ExternalDrag>(views::external_id());
                e.pos = pos;
                e.dropped.extend(ext_dropped);
            });
            if ext_hovering {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        self.prune_missing();
        self.resolve_views();
        self.keys(&ctx, &mut actions);
        self.dispatch_search();

        let search_id = Id::new(("search", self.tabs[self.cur].id));
        // Qué marcar en la barra lateral: la carpeta de la pestaña, o Recientes / Papelera
        // (esas pestañas no son una carpeta: no se marca la carpeta desde donde se abrieron).
        let (current_path, special) = {
            let tab = &self.tabs[self.cur];
            if matches!(tab.tool.as_deref(), Some(super::tools::Tool::Recycle(_))) {
                (String::new(), "papelera")
            } else if tab.search.text.trim_start().to_lowercase().starts_with("recientes:") {
                (String::new(), "recientes")
            } else if tab.tool.is_some() {
                (String::new(), "")
            } else {
                (tab.path().to_string(), "")
            }
        };

        // Las zonas de barra de título se vuelven a juntar en cada cuadro.
        self.caption.clear();
        // ---- barra lateral
        Panel::left("sidebar")
            .resizable(true)
            .default_size(236.0)
            .size_range(180.0..=380.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(t.sidebar))
            .show(ui, |ui| {
                // Arriba, la columna sigue hasta el borde de la ventana (con el nombre de la
                // app, como en Configuración de Windows); esa franja también es barra de título.
                let band = egui::Rect::from_min_size(ui.max_rect().min, vec2(ui.max_rect().width(), chrome::TITLE_H));
                chrome::app_title(ui, &t, band);
                self.caption.push(band);
                ui.add_space(chrome::TITLE_H);
                let drives = self.drives.lock().clone();
                chrome::sidebar(
                    ui,
                    chrome::SidebarIn { t: &t, cfg: &mut self.cfg, places: &self.places, drives: &drives, current: &current_path, special, group_edit: &mut self.group_edit },
                    &mut actions,
                );
                // Botones al pie.
                let r = ui.max_rect();
                let br = egui::Rect::from_min_size(egui::pos2(r.left() + 10.0, r.bottom() - 36.0), vec2(30.0, 28.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
                let icon = if t.dark { egui_phosphor::regular::SUN } else { egui_phosphor::regular::MOON };
                if chrome::icon_btn(&mut child, &t, icon, true, false, t!("app.cambiar_tema")).clicked() {
                    actions.push(Action::ToggleTheme);
                }
            });

        let _ = search_id;
        // ---- barra de estado
        let (left, right, busy) = self.status();
        let mode = self.mgr.mode();
        let jobs_now = self.xfer.snapshot();
        Panel::bottom("status").exact_size(26.0).show_separator_line(false).frame(Frame::new().fill(t.chrome)).show(ui, |ui| {
            // Copias en curso: barra de progreso compacta (clic para ver la lista).
            if !jobs_now.is_empty() {
                let r = ui.max_rect();
                let area = egui::Rect::from_min_size(egui::pos2(r.center().x - 170.0, r.top() + 4.0), vec2(340.0, r.height() - 8.0));
                if chrome::transfer_chip(ui, &t, area, &jobs_now) {
                    self.show_transfers ^= true;
                }
            }
            chrome::status_bar(ui, &t, &left, &right, busy, &mode, &mut actions);
        });
        let service_msg = self.service_result.lock().take();
        if let Some(msg) = service_msg {
            self.toast(&ctx, msg);
        }
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }

        // ---- vista previa
        let mut want_handler: Option<(String, egui::Rect)> = None;
        let item = self.focused_item();
        let text = self.text_for(item.as_ref());
        if self.cfg.show_preview {
            let multi = {
                let tab = &self.tabs[self.cur];
                let col = tab.active_col();
                if !tab.search.active() && col.multi.len() > 1 {
                    let path = col.path.clone();
                    let names = col.multi.clone();
                    let list = self.view_of(&path);
                    Some((names.len(), list.iter().filter(|e| names.contains(&e.name)).filter_map(|e| e.display_size()).sum()))
                } else {
                    None
                }
            };
            Panel::right("preview")
                .resizable(true)
                .default_size(300.0)
                .size_range(220.0..=560.0)
                .show_separator_line(false)
                .frame(Frame::new().fill(t.chrome))
                .show(ui, |ui| {
                    let r = ui.max_rect();
                    let band = egui::Rect::from_min_size(r.min, vec2(r.width(), chrome::TITLE_H));
                    ui.painter().line_segment([band.left_bottom(), r.left_bottom()], egui::Stroke::new(1.0, t.border));
                    // Arriba, la franja de título (con los botones de la ventana a la derecha).
                    let free = egui::Rect::from_min_max(band.min, egui::pos2(band.right() - chrome::WINDOW_BUTTONS_W, band.bottom()));
                    if free.width() > 4.0 {
                        self.caption.push(free);
                    }
                    let body = egui::Rect::from_min_max(egui::pos2(r.left(), band.bottom()), r.max);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(*ui.layout()));
                    want_handler = preview::preview_pane(&mut child, &t, item.as_ref(), multi, &mut self.tex, text.as_deref(), now, self.prevhost.as_ref(), &mut actions);
                });
        }

        // ---- contenido: uno o dos paneles, cada uno con sus pestañas y su barra
        egui::CentralPanel::default().frame(Frame::new().fill(t.bg)).show(ui, |ui| {
            let full = ui.max_rect();
            let rects = if self.cfg.split {
                let mid = full.center().x;
                [egui::Rect::from_min_max(full.min, egui::pos2(mid - 0.5, full.max.y)), egui::Rect::from_min_max(egui::pos2(mid + 0.5, full.min.y), full.max)]
            } else {
                [full, egui::Rect::NOTHING]
            };
            self.pane_rects = rects;
            let panes = if self.cfg.split { 2 } else { 1 };
            for p in 0..panes {
                let mut pui = ui.new_child(egui::UiBuilder::new().max_rect(rects[p]).layout(egui::Layout::top_down(egui::Align::Min)));
                pui.set_clip_rect(rects[p]);
                self.render_pane(&mut pui, p, now, &mut actions);
            }
            if self.cfg.split {
                let x = full.center().x;
                ui.painter().line_segment([egui::pos2(x, full.top()), egui::pos2(x, full.bottom())], egui::Stroke::new(1.0, t.border));
            }
        });

        // ---- arrastrar archivos: etiqueta junto al cursor
        if let Some(drag) = egui::DragAndDrop::payload::<views::DragFiles>(&ctx) {
            if let Some(pos) = ctx.pointer_hover_pos() {
                let n = drag.0.len();
                let label = if n == 1 { fsx::file_name(&drag.0[0]) } else { tf!("comun.elementos", n = n) };
                let (shift, ctrl) = ctx.input(|i| (i.modifiers.shift, i.modifiers.ctrl));
                let hint = if ctrl { t!("app.copiar") } else if shift { t!("app.mover") } else { "" };
                egui::Area::new(Id::new("drag-label")).order(egui::Order::Tooltip).fixed_pos(pos + vec2(14.0, 10.0)).interactable(false).show(&ctx, |ui| {
                    Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.accent)).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("{}  {label}{hint}", egui_phosphor::regular::FILES)).size(12.5).color(t.text));
                    });
                });
            }
        }

        // ---- cola de copias: conflictos y lista de transferencias
        let jobs = self.xfer.snapshot();
        if !self.awaiting.is_empty() {
            let finished: Vec<(u64, bool)> = self.awaiting.iter().copied().filter(|(id, _)| jobs.iter().find(|j| j.id == *id).is_none_or(|j| j.finished())).collect();
            for (id, mv) in finished {
                self.awaiting.retain(|a| a.0 != id);
                let Some(j) = jobs.iter().find(|j| j.id == id) else { continue };
                if j.created.is_empty() {
                    continue;
                }
                self.push_undo(if mv { Undo::Moved(j.created.clone()) } else { Undo::Copied(j.created.iter().map(|c| c.1.clone()).collect()) });
            }
        }
        // Copias que terminaron con "acceso denegado": ofrecer repetirlas como administrador.
        if !self.admin_watch.is_empty() {
            let done: Vec<usize> = (0..self.admin_watch.len()).filter(|&k| jobs.iter().find(|j| j.id == self.admin_watch[k].0).is_none_or(|j| j.finished())).collect();
            for k in done.into_iter().rev() {
                let (id, paths, dest, mv) = self.admin_watch.remove(k);
                let denied = jobs.iter().find(|j| j.id == id).is_some_and(|j| j.errors.iter().any(|e| crate::admin::is_access_denied(e)));
                if denied && self.admin.is_none() {
                    let op = if mv { crate::admin::AdminOp::Move { paths, dest } } else { crate::admin::AdminOp::Copy { paths, dest } };
                    self.admin = Some((t!("app.windows_pide_permiso_administrador_algun").into(), op));
                }
            }
        }
        if let Some(j) = jobs.iter().find(|j| j.state == transfer::State::Conflict) {
            chrome::conflict_dialog(&ctx, &t, j, &mut actions);
        }
        if self.show_transfers {
            if !chrome::transfers_popup(&ctx, &t, &jobs, &mut actions) {
                self.show_transfers = false;
            }
        }
        if jobs.iter().any(|j| !j.finished()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // ---- paleta de comandos y renombrado en lote
        let target = self.target();
        self.palette.ui(&ctx, &t, &target, &mut actions);
        let outcome = self.batch.as_mut().map(|b| (b.ui(&ctx, &t), b.dir.clone()));
        match outcome {
            None | Some((super::tools::Outcome::Open, _)) => {}
            Some((super::tools::Outcome::Cancel, _)) => self.batch = None,
            Some((super::tools::Outcome::Applied(done, errors), dir)) => {
                self.batch = None;
                if !done.is_empty() {
                    self.push_undo(Undo::Batch { dir: dir.clone(), pairs: done });
                }
                self.dirs.reload(&dir);
                if let Some(c) = self.tab().cols.iter_mut().find(|c| c.path == dir) {
                    c.multi.clear();
                    c.cursor = None;
                }
                match errors.len() {
                    0 => self.toast(&ctx, t!("app.renombrado_listo")),
                    n => self.toast(&ctx, tf!("app.no_pudieron_renombrar", n = n, error = errors[0])),
                }
            }
        }

        // ---- Quick Look
        if self.quicklook {
            match &item {
                Some(it) => {
                    if self.ql_page.0 != it.path {
                        self.ql_page = (it.path.clone(), 0);
                    }
                    let (open, want) = preview::quicklook(&ctx, &t, it, &mut self.tex, text.as_deref(), &mut self.ql_page.1, self.prevhost.as_ref(), &mut actions);
                    if !open {
                        self.quicklook = false;
                    }
                    // Con la vista rápida abierta, el visor va en ella (no en el panel).
                    want_handler = want;
                }
                None => self.quicklook = false,
            }
        }

        // ---- seleccionar por patrón y hash
        if self.pattern.is_some() {
            self.pattern_dialog(&ctx, &mut actions);
        }
        if let Some(h) = &mut self.hash {
            if !h.ui(&ctx, &t) {
                self.hash = None;
            }
        }
        self.track_search_history(&ctx);
        // Lo que pasó al elegir una opción de Windows en el menú de clic derecho.
        match self.shell_menus.as_ref().and_then(|s| s.take_outcome()) {
            Some(fsx::ctxmenu::Outcome::Rename(p)) => {
                if let Some(dir) = fsx::parent_of(&p) {
                    actions.push(Action::StartRename(dir, fsx::file_name(&p)));
                }
            }
            Some(fsx::ctxmenu::Outcome::Invoked(paths)) => {
                for p in &paths {
                    self.dirs.reload(p);
                    if let Some(d) = fsx::parent_of(p) {
                        self.dirs.reload(&d);
                    }
                }
            }
            None => {}
        }
        if !self.follow_moves.is_empty() {
            let now = ctx.input(|i| i.time);
            let moves = std::mem::take(&mut self.follow_moves);
            for (from, to, until) in moves {
                if !fsx::exists_like(&from) && fsx::exists_like(&to) {
                    // Toda la cadena de columnas cambia (la carpeta de arriba ya es otra).
                    let fl = from.to_lowercase();
                    for tab in &mut self.tabs {
                        let p = tab.path().to_string();
                        let pl = p.to_lowercase();
                        if pl == fl || pl.starts_with(&format!("{fl}\\")) {
                            let np = format!("{to}{}", &p[from.len()..]);
                            tab.navigate(&np, false);
                        }
                    }
                    self.dirs.reload(&to);
                } else if now < until {
                    self.follow_moves.push((from, to, until));
                }
            }
            if !self.follow_moves.is_empty() {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        if let Some((paths, forever)) = self.lock_report.lock().take() {
            let retry = (if forever { t!("app.volver_intentar_borrar") } else { t!("app.volver_intentar_papelera") }.to_string(), Action::Delete(paths.clone(), forever));
            self.locks = Some(super::tools::LocksDialog::new(paths, Some(t!("app.no_pudo_borrar").into()), Some(retry), &ctx));
        }
        if self.default_dlg.is_some() {
            self.default_dialog(&ctx);
        }
        if let Some(d) = &mut self.lang_dlg {
            match d.ui(&ctx, &t, self.cfg.lang.as_deref()) {
                super::language::LangOut::Open => {}
                super::language::LangOut::Close => self.lang_dlg = None,
                super::language::LangOut::Pick(code) => self.set_language(code),
                super::language::LangOut::ShowFolder(dir) => {
                    self.lang_dlg = None;
                    actions.push(Action::Navigate(dir));
                }
            }
        }
        if let Some(op) = self.admin_report.lock().take() {
            if self.admin.is_none() {
                self.admin = Some((t!("app.windows_no_dejo_hacerlo_tus").into(), op));
            }
        }
        if self.admin.is_some() {
            self.admin_dialog(&ctx);
        }
        if self.first_run {
            self.first_run_dialog(&ctx);
        }
        if let Some(l) = &mut self.locks {
            if !l.ui(&ctx, &t, &mut actions) {
                self.locks = None;
            } else if std::mem::take(&mut l.release_self) {
                if let Some(h) = &self.prevhost {
                    h.unload();
                }
            }
        }

        // ---- visor de Windows: se coloca donde se pidió; se oculta si algo queda encima
        if let Some(host) = &self.prevhost {
            let covered = egui::Popup::is_any_open(&ctx)
                || self.palette.open
                || self.batch.is_some()
                || self.pattern.is_some()
                || self.hash.is_some()
                || self.locks.is_some()
                || self.default_dlg.is_some()
                || self.lang_dlg.is_some()
                || self.admin.is_some()
                || self.first_run
                || chrome::overlay_open(&ctx)
                || self.show_transfers
                || jobs.iter().any(|j| j.state == transfer::State::Conflict)
                || egui::DragAndDrop::has_any_payload(&ctx);
            match want_handler {
                Some(_) if covered => host.hide(),
                Some((path, r)) => {
                    let ppp = ctx.pixels_per_point();
                    let px = [(r.left() * ppp).round() as i32, (r.top() * ppp).round() as i32, (r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32];
                    host.show(&path, px);
                    if host.status_of(&path) == Some(super::prevhost::Status::Loading) {
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
                None => host.unload(),
            }
        }

        // ---- aviso flotante
        if let Some((msg, at)) = self.toast.clone() {
            let el = ctx.input(|i| i.time) - at;
            if el > 2.5 {
                self.toast = None;
            } else {
                egui::Area::new(Id::new("toast")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -44.0)).show(&ctx, |ui| {
                    Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                        ui.label(egui::RichText::new(msg).color(t.text));
                    });
                });
            }
        }

        // Arrastrar archivos fuera de la ventana: pasa a ser un arrastre de Windows hacia otras apps.
        if let Some(drag) = egui::DragAndDrop::payload::<views::DragFiles>(&ctx) {
            let outside = ctx.input(|i| i.pointer.primary_down() && i.pointer.latest_pos().is_some_and(|p| !i.content_rect().contains(p)));
            if outside {
                self.drag_out = Some(drag.0.clone());
                egui::DragAndDrop::clear_payload(&ctx);
            }
        }
        // Lo soltado desde otra app que nadie recibió (p. ej. sobre la barra lateral) se descarta.
        if !ext_hovering {
            ctx.data_mut(|d| d.remove::<views::ExternalDrag>(views::external_id()));
        }

        // El menú de clic derecho, en sus propias ventanas.
        self.show_menu(&ctx, &mut actions);
        // Lo que cambian las acciones se ve en el cuadro siguiente: pedirlo, aunque no haya
        // entrada (p. ej. el renombrado con clic lento lo dispara un temporizador).
        if !actions.is_empty() {
            ctx.request_repaint();
        }
        for a in actions {
            self.apply(&ctx, a);
        }
        self.resolve_views();
        // Botones de la ventana (arriba a la derecha) y qué partes de la fila de arriba
        // son barra de título, para que Windows sepa dónde se arrastra la ventana.
        let max_button = chrome::window_buttons(&ctx, &t);
        super::win::set_caption(&self.caption, max_button, ctx.pixels_per_point());
        self.perf_overlay(&ctx, frame);
        self.run_drag_out(&ctx);
        if self.dirs.frame <= 3 {
            crate::index::mem_log(&format!("ui: fin del cuadro {}", self.dirs.frame));
        }
        if self.dirs.frame == 2 {
            crate::index::mem_log(&format!("cpu del primer cuadro {:.1} ms", frame.info().cpu_usage.unwrap_or(0.0) * 1000.0));
        }
        self.run_native_menu(&ctx);
        self.update_watched();
        self.dirs.evict();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::instance::shutdown();
        self.save_cfg();
        // Guardar el índice con su posición en el journal: al abrir solo se leen los cambios.
        self.mgr.save_all();
        self.mgr.shutdown.store(true, Ordering::Relaxed);
    }
}

/// El menú de clic derecho abierto.
struct MenuWin {
    req: super::ctxmenu::MenuReq,
    /// Dónde se hizo clic (píxeles de la pantalla).
    anchor: (i32, i32),
    opened: f64,
    query: String,
    show_hidden: bool,
    /// Submenús abiertos: (fila en el nivel de arriba, altura de esa fila en su ventana).
    subs: Vec<(usize, f32)>,
    /// Qué ventanas ya tienen el aspecto de menú de Windows (un bit por nivel).
    styled: u32,
}
