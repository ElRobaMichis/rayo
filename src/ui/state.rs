//! Estado de navegación: caché de carpetas, pestañas, columnas y búsqueda.

use crate::fsx::{self, FileEntry, SortKey};
use crate::index::search::Hit;
use crate::index::{IndexManager, ResultSort, SearchOpts};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

// ---------------------------------------------------------------- caché de carpetas

pub struct DirState {
    pub raw: Option<Arc<Vec<FileEntry>>>,
    pub view: Arc<Vec<FileEntry>>,
    pub view_key: Option<(SortKey, bool, bool)>,
    pub loading: bool,
    pub error: Option<String>,
    pub last_used: u64,
    /// Aumenta con cada listado recibido.
    pub generation: u64,
}

type ListResult = (String, Result<Vec<FileEntry>, String>);

pub struct DirCache {
    pub map: HashMap<String, DirState>,
    tx: Sender<String>,
    rx: Receiver<ListResult>,
    pub frame: u64,
}

impl DirCache {
    pub fn new(mgr: Arc<IndexManager>, wake: Wake) -> Self {
        let (tx, req_rx) = unbounded::<String>();
        let (res_tx, rx) = unbounded::<ListResult>();
        for i in 0..2 {
            let req_rx = req_rx.clone();
            let res_tx = res_tx.clone();
            let mgr = mgr.clone();
            let wake = wake.clone();
            std::thread::Builder::new()
                .name(format!("listado-{i}"))
                .spawn(move || {
                    while let Ok(path) = req_rx.recv() {
                        // Vista plana: todos los archivos de la carpeta y sus subcarpetas.
                        if let Some(dir) = path.strip_prefix(fsx::FLAT_KEY) {
                            let r = mgr.files_under(dir, 300_000).map(Ok).unwrap_or_else(|| fsx::list_flat(dir, 300_000));
                            let _ = res_tx.send((path, r));
                            wake();
                            continue;
                        }
                        let mut r = fsx::list_dir(&path);
                        if let Ok(list) = &mut r {
                            if list.iter().any(|e| e.is_dir) {
                                if let Some(sizes) = mgr.child_sizes(&path) {
                                    for e in list.iter_mut().filter(|e| e.is_dir) {
                                        e.dir_size = sizes.get(&e.lower).copied();
                                    }
                                }
                            }
                        }
                        let _ = res_tx.send((path, r));
                        wake();
                    }
                })
                .ok();
        }
        DirCache { map: HashMap::new(), tx, rx, frame: 0 }
    }

    pub fn request(&mut self, path: &str) {
        let frame = self.frame;
        let st = self.map.entry(path.to_string()).or_insert_with(|| DirState {
            raw: None,
            view: Arc::new(Vec::new()),
            view_key: None,
            loading: false,
            error: None,
            last_used: frame,
            generation: 0,
        });
        st.last_used = frame;
        if st.raw.is_none() && !st.loading && st.error.is_none() {
            st.loading = true;
            let _ = self.tx.send(path.to_string());
        }
    }

    pub fn reload(&mut self, path: &str) {
        if let Some(st) = self.map.get_mut(path) {
            if !st.loading {
                st.loading = true;
                st.error = None;
                let _ = self.tx.send(path.to_string());
            }
        } else {
            self.request(path);
        }
    }

    pub fn reload_all(&mut self) {
        let keys: Vec<String> = self.map.keys().cloned().collect();
        for k in keys {
            self.reload(&k);
        }
    }

    /// Procesa listados terminados. Devuelve true si llegó algo.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((path, r)) = self.rx.try_recv() {
            any = true;
            if let Some(st) = self.map.get_mut(&path) {
                st.loading = false;
                match r {
                    Ok(list) => {
                        if self.frame < 60 {
                            crate::index::mem_log(&format!("listado: {} ({} elementos)", crate::fsx::file_name(&path), list.len()));
                        }
                        st.generation += 1;
                        st.raw = Some(Arc::new(list));
                        st.error = None;
                        st.view_key = None;
                    }
                    Err(e) => {
                        if st.raw.is_none() {
                            st.error = Some(e);
                        }
                    }
                }
            }
        }
        any
    }

    /// Lista ordenada y filtrada de una carpeta (pide el listado si hace falta).
    pub fn view(&mut self, path: &str, sort: SortKey, desc: bool, hidden: bool) -> Arc<Vec<FileEntry>> {
        self.request(path);
        let st = self.map.get_mut(path).unwrap();
        let key = (sort, desc, hidden);
        if st.view_key != Some(key) {
            if let Some(raw) = &st.raw {
                let mut v: Vec<FileEntry> = raw.iter().filter(|e| hidden || !e.hidden).cloned().collect();
                fsx::sort_entries(&mut v, sort, desc);
                st.view = Arc::new(v);
                st.view_key = Some(key);
            }
        }
        st.view.clone()
    }

    pub fn state(&self, path: &str) -> Option<&DirState> {
        self.map.get(path)
    }

    /// Libera carpetas que no se usan hace rato.
    pub fn evict(&mut self) {
        if self.map.len() <= 96 {
            return;
        }
        let frame = self.frame;
        self.map.retain(|_, s| frame.saturating_sub(s.last_used) < 600 || s.loading);
    }
}

// ---------------------------------------------------------------- pestañas

#[derive(Clone, Debug, Default)]
pub struct Col {
    pub path: String,
    /// Elemento enfocado (por nombre, sobrevive a recargas).
    pub cursor: Option<String>,
    pub multi: BTreeSet<String>,
    pub anchor: Option<String>,
    pub scroll_to_cursor: bool,
}

impl Col {
    pub fn new(path: String) -> Self {
        Col { path, ..Default::default() }
    }

    /// Nombres seleccionados (selección múltiple o el cursor).
    pub fn selected(&self) -> Vec<String> {
        if !self.multi.is_empty() {
            self.multi.iter().cloned().collect()
        } else {
            self.cursor.iter().cloned().collect()
        }
    }

    pub fn is_selected(&self, name: &str) -> bool {
        if self.multi.is_empty() { self.cursor.as_deref() == Some(name) } else { self.multi.contains(name) }
    }
}

#[derive(Clone, Debug)]
pub struct ResultRow {
    pub name: String,
    pub dir: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u32,
    pub ext: String,
}

impl ResultRow {
    pub fn path(&self) -> String {
        fsx::join(&self.dir, &self.name)
    }
}

#[derive(Default)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub epochs: Vec<u64>,
    pub total: usize,
    pub ms: f64,
    pub query: String,
    /// Tiene todos los resultados, no solo los mejores.
    pub full: bool,
    /// Filas ya armadas (nombre, carpeta…): solo se arman las que se muestran.
    rows: parking_lot::Mutex<HashMap<usize, Option<ResultRow>>>,
    mgr: Option<Arc<IndexManager>>,
}

impl Results {
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn row(&self, i: usize) -> Option<ResultRow> {
        if let Some(r) = self.rows.lock().get(&i) {
            return r.clone();
        }
        let r = self.build_row(i);
        let mut cache = self.rows.lock();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(i, r.clone());
        r
    }

    fn build_row(&self, i: usize) -> Option<ResultRow> {
        let h = *self.hits.get(i)?;
        let mgr = self.mgr.as_ref()?;
        let slot = mgr.slots.get(h.vol as usize)?;
        // Si el índice se reemplazó, los ids ya no valen (la búsqueda se repite sola).
        if self.epochs.get(h.vol as usize).copied() != Some(slot.epoch.load(Ordering::Relaxed)) {
            return None;
        }
        let v = slot.index.read();
        if h.id as usize >= v.len() {
            return None;
        }
        let name = v.name(h.id);
        let is_dir = v.is_dir(h.id);
        Some(ResultRow {
            ext: if is_dir { String::new() } else { fsx::ext_of(&name) },
            name,
            dir: v.parent_path(h.id),
            is_dir,
            size: v.size[h.id as usize],
            mtime: v.mtime[h.id as usize],
        })
    }
}

#[derive(Default)]
pub struct SearchUi {
    pub text: String,
    /// Buscar solo dentro de la carpeta actual.
    pub here: bool,
    pub seq: u64,
    pub sent: Option<SentKey>,
    pub results: Option<Results>,
    pub sort: ResultSort,
    pub desc: bool,
    /// El usuario pidió ver todos los resultados (bajó hasta el final u ordenó).
    pub want_full: bool,
    pub cursor: usize,
    pub scroll_to_cursor: bool,
    pub focus: bool,
    pub changes_seen: u64,
    /// Para el historial: el texto, desde cuándo está así y el último que se guardó.
    pub hist_seen: String,
    pub hist_since: f64,
    pub hist_recorded: String,
}

/// Lo que define una búsqueda: texto, alcance, carpeta, orden, sentido y si es completa.
pub type SentKey = (String, bool, String, ResultSort, bool, bool);

impl SearchUi {
    pub fn opts(&self) -> SearchOpts {
        let full = self.want_full || self.sort != ResultSort::Relevance;
        SearchOpts { limit: if full { None } else { Some(SEARCH_LIMIT) }, sort: self.sort, desc: self.desc }
    }

    pub fn active(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

pub struct Tab {
    pub id: u64,
    /// Panel al que pertenece (0 = izquierdo, 1 = derecho).
    pub pane: u8,
    /// Filtro de la carpeta actual (lo que se escribe con la lista enfocada).
    pub filter: String,
    pub cols: Vec<Col>,
    pub active: usize,
    pub back: Vec<String>,
    pub fwd: Vec<String>,
    pub search: SearchUi,
    pub path_edit: Option<String>,
    /// Vista plana: los archivos de todas las subcarpetas en una sola lista.
    pub flat: bool,
    /// Aumenta con cada navegación (para aplicar la vista recordada de la carpeta).
    pub nav_seq: u64,
    /// Vista y orden de esta pestaña, y para qué navegación se resolvieron.
    pub view: Option<super::config::FolderView>,
    pub view_seq: u64,
    /// Herramienta que ocupa la pestaña (duplicados, comparar) en lugar de la carpeta.
    pub tool: Option<Box<super::tools::Tool>>,
}

fn chain(path: &str) -> Vec<Col> {
    let path = fsx::normalize(path);
    let mut parts: Vec<String> = Vec::new();
    let mut cur = Some(path.clone());
    while let Some(p) = cur {
        cur = fsx::parent_of(&p);
        parts.push(p);
    }
    parts.reverse();
    let mut cols: Vec<Col> = parts.into_iter().map(Col::new).collect();
    for i in 0..cols.len().saturating_sub(1) {
        let child = fsx::file_name(&cols[i + 1].path);
        cols[i].cursor = Some(child);
        cols[i].scroll_to_cursor = true;
    }
    cols
}

impl Tab {
    pub fn new(id: u64, path: &str) -> Self {
        let cols = chain(path);
        Tab { id, pane: 0, filter: String::new(), active: cols.len() - 1, cols, back: Vec::new(), fwd: Vec::new(), search: SearchUi::default(), path_edit: None, tool: None, flat: false, nav_seq: 1, view: None, view_seq: 0 }
    }

    pub fn path(&self) -> &str {
        &self.cols.last().unwrap().path
    }

    pub fn title(&self) -> String {
        if let Some(tool) = &self.tool {
            return tool.title();
        }
        let p = self.path();
        if p.len() <= 3 { p.trim_end_matches('\\').to_string() } else { fsx::file_name(p) }
    }

    pub fn active_col(&self) -> &Col {
        &self.cols[self.active.min(self.cols.len() - 1)]
    }

    pub fn active_col_mut(&mut self) -> &mut Col {
        let i = self.active.min(self.cols.len() - 1);
        &mut self.cols[i]
    }

    pub fn navigate(&mut self, path: &str, history: bool) {
        let path = fsx::normalize(path);
        if path == self.path() && self.cols.last().is_some_and(|c| c.cursor.is_none()) {
            return;
        }
        self.filter.clear();
        self.flat = false;
        self.nav_seq += 1;
        if history {
            self.back.push(self.path().to_string());
            self.fwd.clear();
        }
        self.cols = chain(&path);
        self.active = self.cols.len() - 1;
        self.path_edit = None;
    }

    /// Navega a la carpeta que contiene `path` y lo deja seleccionado.
    pub fn reveal(&mut self, path: &str) {
        if let Some(parent) = fsx::parent_of(path) {
            self.navigate(&parent, true);
            let col = self.cols.last_mut().unwrap();
            col.cursor = Some(fsx::file_name(path));
            col.scroll_to_cursor = true;
        }
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.fwd.push(self.path().to_string());
            self.navigate(&p, false);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(p) = self.fwd.pop() {
            self.back.push(self.path().to_string());
            self.navigate(&p, false);
        }
    }

    pub fn go_up(&mut self) {
        let cur = self.path().to_string();
        if let Some(parent) = fsx::parent_of(&cur) {
            self.navigate(&parent, true);
            let col = self.cols.last_mut().unwrap();
            col.cursor = Some(fsx::file_name(&cur));
            col.scroll_to_cursor = true;
        }
    }

    /// Selecciona un elemento de la columna `ci` (comportamiento de Finder):
    /// si es carpeta, se abre una columna nueva a la derecha.
    pub fn select(&mut self, ci: usize, name: &str, is_dir: bool, columns_view: bool) {
        // Pasar a otra columna quita el filtro de la anterior.
        if ci != self.active {
            self.filter.clear();
        }
        self.cols.truncate(ci + 1);
        let col = &mut self.cols[ci];
        col.cursor = Some(name.to_string());
        col.multi.clear();
        col.anchor = Some(name.to_string());
        col.scroll_to_cursor = true;
        self.active = ci;
        if is_dir && columns_view {
            let p = fsx::join(&col.path, name);
            self.cols.push(Col::new(p));
        }
    }

    /// Si en vista de columnas la última columna tiene una carpeta enfocada,
    /// agrega su columna.
    pub fn ensure_child_col(&mut self, last_is_dir: bool) {
        if last_is_dir {
            if let Some(name) = self.cols.last().and_then(|c| c.cursor.clone()) {
                let p = fsx::join(self.path(), &name);
                self.cols.push(Col::new(p));
            }
        }
    }

    /// Ruta y nombre del elemento enfocado.
    pub fn focused(&self) -> Option<(String, String)> {
        let c = self.active_col();
        c.cursor.as_ref().map(|n| (c.path.clone(), n.clone()))
    }

    /// Rutas completas de lo seleccionado en la columna activa.
    pub fn selected_paths(&self) -> Vec<String> {
        let c = self.active_col();
        c.selected().iter().map(|n| fsx::join(&c.path, n)).collect()
    }
}

// ---------------------------------------------------------------- búsqueda en segundo plano

pub struct SearchReq {
    pub tab: u64,
    pub seq: u64,
    pub text: String,
    pub within: Option<String>,
    pub opts: SearchOpts,
}

pub struct SearchRes {
    pub tab: u64,
    pub seq: u64,
    pub results: Results,
}

pub struct SearchWorker {
    pub tx: Sender<SearchReq>,
    pub rx: Receiver<SearchRes>,
}

/// Resultados que se calculan primero (los mejores); el resto, al pedirlos.
pub const SEARCH_LIMIT: usize = 1000;

impl SearchWorker {
    pub fn new(mgr: Arc<IndexManager>, wake: Wake) -> Self {
        let (tx, req_rx) = unbounded::<SearchReq>();
        let (res_tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("busqueda".into())
            .spawn(move || {
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(r) = req_rx.try_recv() {
                        req = r;
                    }
                    let out = mgr.search(&req.text, req.within.as_deref(), req.opts);
                    let results = Results {
                        hits: out.hits,
                        epochs: out.epochs,
                        total: out.total,
                        ms: out.elapsed.as_secs_f64() * 1000.0,
                        query: req.text.clone(),
                        full: req.opts.limit.is_none(),
                        rows: Default::default(),
                        mgr: Some(mgr.clone()),
                    };
                    let _ = res_tx.send(SearchRes { tab: req.tab, seq: req.seq, results });
                    wake();
                }
            })
            .ok();
        SearchWorker { tx, rx }
    }
}
