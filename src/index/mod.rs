//! Índice de archivos de todos los volúmenes NTFS locales.

pub mod cache;
pub mod everything;
pub mod mft;
pub mod search;
pub mod service;
pub mod store;
pub mod usn;
pub mod walk;
pub mod watch;

use parking_lot::{Mutex, RwLock};
use search::{Compiled, Hit};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
pub use store::{Source, VolumeIndex};

/// Cómo se mantiene al día un disco (valores internos; en pantalla, `mode_label`).
pub const MODE_SERVICE: &str = "USN (servicio)";
pub const MODE_ADMIN: &str = "USN (administrador)";
pub const MODE_WATCH: &str = "vigilancia";

/// El modo del índice, para mostrar.
pub fn mode_label(mode: &str) -> &'static str {
    if mode.contains(MODE_SERVICE) {
        t!("indice.modo_servicio")
    } else if mode.contains(MODE_ADMIN) {
        t!("indice.modo_administrador")
    } else {
        t!("indice.modo_vigilancia")
    }
}

#[derive(Debug, Clone, Default)]
pub struct SlotState {
    pub building: bool,
    /// Cómo se mantiene al día: "USN (servicio)", "USN (administrador)" o "vigilancia".
    pub mode: String,
    pub source: Source,
    pub build_ms: u64,
    pub entries: usize,
    pub memory: usize,
    pub note: Option<String>,
}

pub struct Slot {
    pub letter: char,
    pub index: RwLock<VolumeIndex>,
    /// Cambia cada vez que el índice se reemplaza entero (los ids dejan de valer).
    pub epoch: AtomicU64,
    pub progress: AtomicUsize,
    pub needs_rescan: AtomicBool,
    pub state: Mutex<SlotState>,
    /// Órdenes precalculados (por nombre y por ruta) para ordenar resultados al instante.
    pub ranks: Mutex<Option<(u64, Arc<Ranks>)>>,
    /// El USN Journal mantiene este disco al día (la vigilancia clásica se ignora).
    pub usn_active: AtomicBool,
}

/// Quién lee la MFT y el journal: el servicio, o la propia app si tiene permisos.
enum Backend {
    Service(service::Client),
    Local(usn::Volume),
}

impl Backend {
    fn detect(letter: char) -> Option<(Backend, &'static str)> {
        if let Some(c) = service::Client::connect() {
            return Some((Backend::Service(c), MODE_SERVICE));
        }
        let v = usn::Volume::open(letter).ok()?;
        v.journal().ok()?;
        Some((Backend::Local(v), MODE_ADMIN))
    }

    fn build(&mut self, letter: char) -> Result<VolumeIndex, String> {
        match self {
            Backend::Service(c) => c.build(letter),
            Backend::Local(_) => mft::read_mft(letter, &AtomicUsize::new(0)),
        }
    }

    fn changes(&mut self, letter: char, journal: u64, from: i64, wait: u32) -> Result<service::Changes, String> {
        match self {
            Backend::Service(c) => c.changes(letter, journal, from, wait),
            Backend::Local(v) => match v.changes(journal, from, wait) {
                Ok((list, next)) => Ok(service::Changes::Batch(list, next)),
                Err(usn::ReadError::Reset) => Ok(service::Changes::Reset),
                Err(usn::ReadError::Other(e)) => Err(e),
            },
        }
    }
}

/// Posición de cada entrada en el orden alfabético por nombre y por ruta completa.
pub struct Ranks {
    /// Ids ordenados por nombre.
    pub order: Vec<u32>,
    pub name: Vec<u32>,
    pub path: Vec<u32>,
}

impl Ranks {
    pub fn compute(v: &VolumeIndex) -> Ranks {
        use rayon::prelude::*;
        let n = v.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.par_sort_unstable_by(|&a, &b| v.name_lower(a).cmp(v.name_lower(b)).then(a.cmp(&b)));
        let mut name = vec![0u32; n];
        for (r, &id) in order.iter().enumerate() {
            name[id as usize] = r as u32;
        }
        // Recorrido del árbol en orden alfabético: la posición de cada entrada es su
        // lugar al ordenar por ruta completa.
        let mut path = vec![u32::MAX; n];
        let mut counter = 0u32;
        let mut stack = vec![0u32];
        let mut kids: Vec<u32> = Vec::new();
        while let Some(d) = stack.pop() {
            path[d as usize] = counter;
            counter += 1;
            if !v.is_dir(d) {
                continue;
            }
            kids.clear();
            kids.extend(v.children(d));
            kids.sort_unstable_by_key(|&c| name[c as usize]);
            stack.extend(kids.iter().rev());
        }
        Ranks { order, name, path }
    }
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct IndexManager {
    pub slots: Vec<Arc<Slot>>,
    /// Aumenta con cada cambio en cualquier volumen.
    pub changes: AtomicU64,
    pub shutdown: AtomicBool,
    /// Pide revisar ya si apareció el servicio (por ejemplo, tras instalarlo).
    pub recheck: AtomicBool,
    /// Carpetas que la interfaz está mostrando (en minúsculas).
    pub watched: RwLock<std::collections::HashSet<String>>,
    /// Carpetas vigiladas que cambiaron desde la última vez.
    pub dirty: Mutex<Vec<String>>,
    /// Despierta a la interfaz (pide un repintado).
    pub wake: RwLock<Option<Wake>>,
}

pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub total: usize,
    pub elapsed: Duration,
    pub epochs: Vec<u64>,
}

/// Orden de los resultados de búsqueda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResultSort {
    #[default]
    Relevance,
    Name,
    Folder,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOpts {
    /// `Some(n)`: solo los n mejores por relevancia (rápido). `None`: todos.
    pub limit: Option<usize>,
    pub sort: ResultSort,
    pub desc: bool,
}

/// Letras de los discos fijos con NTFS.
pub fn ntfs_fixed_volumes() -> Vec<char> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows::core::PCWSTR;
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = walk::wide(&format!("{letter}:\\"));
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != 3 {
            continue;
        }
        let mut fs = [0u16; 32];
        if unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), None, None, None, None, Some(&mut fs)) }.is_ok() {
            let len = fs.iter().position(|&c| c == 0).unwrap_or(0);
            if String::from_utf16_lossy(&fs[..len]) == "NTFS" {
                out.push(letter);
            }
        }
    }
    out
}

/// Momento en que arrancó el proceso (lo fija `main`).
pub static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Registro de diagnóstico (solo con la variable RAYO_DEBUG).
pub fn mem_log(label: &str) {
    if std::env::var_os("RAYO_DEBUG").is_none() {
        return;
    }
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut _, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
    }
    let ms = START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
    let line = format!("{ms:>8.1} ms  {label:<28} privado {:>5} MB  ws {:>5} MB\n", c.PrivateUsage >> 20, c.WorkingSetSize >> 20);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(cache::data_dir().join("debug.log")) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Devuelve al sistema la memoria libre del heap (tras construir un índice
/// quedan muchos bloques temporales liberados).
pub fn release_free_memory() {
    use windows::Win32::System::Memory::{GetProcessHeap, HeapCompact, HEAP_FLAGS};
    unsafe {
        if let Ok(h) = GetProcessHeap() {
            HeapCompact(h, HEAP_FLAGS(0));
        }
    }
}

/// Ordena todos los resultados por nombre, carpeta, tamaño o fecha.
pub fn sort_hits(hits: &mut [search::Hit], vols: &[&VolumeIndex], ranks: &[Option<Arc<Ranks>>], sort: ResultSort, desc: bool) {
    use rayon::prelude::*;
    use std::cmp::Ordering as O;
    let tie = |a: &search::Hit, b: &search::Hit| a.vol.cmp(&b.vol).then(a.id.cmp(&b.id));
    match sort {
        ResultSort::Relevance => {}
        ResultSort::Size => hits.par_sort_unstable_by(|a, b| {
            let (x, y) = (vols[a.vol as usize].size[a.id as usize], vols[b.vol as usize].size[b.id as usize]);
            x.cmp(&y).then_with(|| tie(a, b))
        }),
        ResultSort::Modified => hits.par_sort_unstable_by(|a, b| {
            let (x, y) = (vols[a.vol as usize].mtime[a.id as usize], vols[b.vol as usize].mtime[b.id as usize]);
            x.cmp(&y).then_with(|| tie(a, b))
        }),
        ResultSort::Name => hits.par_sort_unstable_by(|a, b| {
            // Mismo disco y ambos con rango: comparar dos números. Si no (entradas
            // agregadas después de calcular los rangos, u otro disco): por texto.
            // Las dos formas dan el mismo orden total (nombre, disco, id).
            if a.vol == b.vol {
                if let Some(r) = &ranks[a.vol as usize] {
                    if let (Some(x), Some(y)) = (r.name.get(a.id as usize), r.name.get(b.id as usize)) {
                        return x.cmp(y);
                    }
                }
            }
            let (x, y) = (vols[a.vol as usize].name_lower(a.id), vols[b.vol as usize].name_lower(b.id));
            x.cmp(y).then_with(|| tie(a, b))
        }),
        ResultSort::Folder => {
            // Clave por resultado: (disco, posición en el recorrido alfabético del árbol).
            // Lo agregado después de calcular los rangos va justo detrás de su carpeta.
            let key = |h: &search::Hit| -> (u8, u32, u8, u32) {
                let v = &vols[h.vol as usize];
                match &ranks[h.vol as usize] {
                    Some(r) => match r.path.get(h.id as usize) {
                        Some(&p) if p != u32::MAX => (h.vol, p, 0, h.id),
                        _ => {
                            let parent = v.parent[h.id as usize];
                            let pp = r.path.get(parent as usize).copied().unwrap_or(u32::MAX);
                            (h.vol, pp, 1, h.id)
                        }
                    },
                    None => (h.vol, u32::MAX, 1, h.id),
                }
            };
            hits.par_sort_unstable_by_key(key);
        }
    }
    if desc {
        hits.reverse();
    }
    let _ = O::Equal;
}

/// MFT si hay permisos de administrador; si no, recorrido en paralelo.
pub fn build_volume(letter: char, progress: &AtomicUsize) -> VolumeIndex {
    match mft::read_mft(letter, progress) {
        Ok(v) => v,
        Err(_) => {
            progress.store(0, Ordering::Relaxed);
            walk::walk_volume(letter, progress)
        }
    }
}

/// Punto de entrada del proceso indexador. Devuelve el código de salida.
pub fn build_child(letter: char) -> i32 {
    use std::io::Write;
    let progress = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let reporter = {
        let (progress, done) = (progress.clone(), done.clone());
        std::thread::spawn(move || {
            let mut out = std::io::stdout();
            while !done.load(Ordering::Relaxed) {
                if writeln!(out, "p {}", progress.load(Ordering::Relaxed)).and_then(|_| out.flush()).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        })
    };
    let v = build_volume(letter, &progress);
    let ok = cache::save(&v).is_ok();
    done.store(true, Ordering::Relaxed);
    let _ = reporter.join();
    if ok { 0 } else { 1 }
}

impl IndexManager {
    pub fn new(letters: &[char]) -> Arc<Self> {
        Arc::new(IndexManager {
            slots: letters
                .iter()
                .map(|&l| {
                    Arc::new(Slot {
                        letter: l,
                        index: RwLock::new(VolumeIndex::new(l, Source::Empty)),
                        epoch: AtomicU64::new(0),
                        progress: AtomicUsize::new(0),
                        needs_rescan: AtomicBool::new(false),
                        state: Mutex::new(SlotState::default()),
                        ranks: Mutex::new(None),
                        usn_active: AtomicBool::new(false),
                    })
                })
                .collect(),
            changes: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            recheck: AtomicBool::new(false),
            watched: Default::default(),
            dirty: Default::default(),
            wake: Default::default(),
        })
    }

    fn replace(&self, slot: &Slot, v: VolumeIndex, ms: u64) {
        {
            let mut st = slot.state.lock();
            st.source = v.source;
            st.build_ms = ms;
            st.entries = v.live_len();
            st.memory = v.memory_bytes();
        }
        *slot.index.write() = v;
        let epoch = slot.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        self.changes.fetch_add(1, Ordering::Relaxed);
        self.wake_ui();
        // Órdenes por nombre y ruta, listos para cuando se ordene una búsqueda.
        let ranks = Arc::new(Ranks::compute(&slot.index.read()));
        *slot.ranks.lock() = Some((epoch, ranks));
    }

    fn ranks_of(&self, k: usize, v: &VolumeIndex) -> Arc<Ranks> {
        let slot = &self.slots[k];
        let epoch = slot.epoch.load(Ordering::Relaxed);
        let mut r = slot.ranks.lock();
        match r.as_ref() {
            Some((e, ranks)) if *e == epoch => ranks.clone(),
            _ => {
                let ranks = Arc::new(Ranks::compute(v));
                *r = Some((epoch, ranks.clone()));
                ranks
            }
        }
    }

    pub fn wake_ui(&self) {
        if let Some(w) = self.wake.read().as_ref() {
            w();
        }
    }

    /// Avisa a la interfaz si alguna de estas carpetas está a la vista.
    pub fn notify_dirs<'a>(&self, dirs: impl Iterator<Item = &'a str>) {
        let watched = self.watched.read();
        if watched.is_empty() {
            return;
        }
        let mut hit = false;
        for d in dirs {
            let l = d.to_lowercase();
            if watched.contains(&l) {
                let mut dirty = self.dirty.lock();
                if !dirty.contains(&l) {
                    dirty.push(l);
                }
                hit = true;
            }
        }
        drop(watched);
        if hit {
            self.wake_ui();
        }
    }

    /// Construye el índice de un volumen en un proceso aparte (ver `build_child`)
    /// y lo carga desde la caché. Si no se puede lanzar el proceso, lo hace aquí.
    pub fn build(&self, slot: &Slot) {
        slot.state.lock().building = true;
        slot.progress.store(0, Ordering::Relaxed);
        self.wake_ui();
        let t = Instant::now();
        let v = match self.build_in_child(slot) {
            Some(v) => v,
            None => {
                let v = build_volume(slot.letter, &slot.progress);
                let _ = cache::save(&v);
                v
            }
        };
        let ms = t.elapsed().as_millis() as u64;
        self.replace(slot, v, ms);
        release_free_memory();
        slot.needs_rescan.store(false, Ordering::Relaxed);
        slot.state.lock().building = false;
        self.wake_ui();
    }

    fn build_in_child(&self, slot: &Slot) -> Option<VolumeIndex> {
        use std::io::BufRead;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        let exe = std::env::current_exe().ok()?;
        let mut child = std::process::Command::new(exe)
            .args(["--build-index", &slot.letter.to_string()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
            .spawn()
            .ok()?;
        // El hijo informa su avance en líneas "p <n>" y su origen en "s <fuente>".
        let out = child.stdout.take()?;
        for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(n) = line.strip_prefix("p ").and_then(|n| n.parse().ok()) {
                slot.progress.store(n, Ordering::Relaxed);
            } else if let Some(note) = line.strip_prefix("n ") {
                slot.state.lock().note = Some(note.to_string());
            }
        }
        if !child.wait().ok()?.success() {
            return None;
        }
        let mut v = cache::load(slot.letter).ok()?;
        // La caché guarda el origen real (MFT o recorrido).
        v.source = cache::load_source(slot.letter).unwrap_or(Source::Walk);
        Some(v)
    }

    /// Arranca todo en segundo plano: caché → índice fresco → vigilancia de cambios.
    pub fn start(self: &Arc<Self>) {
        if std::env::var_os("RAYO_NO_INDEX").is_some() {
            return;
        }
        for slot in &self.slots {
            let mgr = self.clone();
            let slot = slot.clone();
            std::thread::Builder::new()
                .name(format!("indice-{}", slot.letter))
                .spawn(move || {
                    let t = Instant::now();
                    mem_log(&format!("{}: antes de caché", slot.letter));
                    if let Ok(v) = cache::load(slot.letter) {
                        mem_log(&format!("{}: caché leída", slot.letter));
                        mgr.replace(&slot, v, t.elapsed().as_millis() as u64);
                        release_free_memory();
                        mem_log(&format!("{}: caché instalada", slot.letter));
                    }
                    let mut classic_ready = false;
                    let mut watcher = false;
                    while !mgr.shutdown.load(Ordering::Relaxed) {
                        // Con servicio (o permisos): MFT + USN Journal, sin reindexar al abrir.
                        if let Some((backend, mode)) = Backend::detect(slot.letter) {
                            slot.state.lock().mode = mode.to_string();
                            slot.usn_active.store(true, Ordering::Relaxed);
                            mgr.wake_ui();
                            if let Err(e) = mgr.run_usn(&slot, backend) {
                                slot.state.lock().note = Some(e);
                            }
                            slot.usn_active.store(false, Ordering::Relaxed);
                            if mgr.shutdown.load(Ordering::Relaxed) {
                                break;
                            }
                            // El servicio se cayó: seguir con la vigilancia clásica.
                            classic_ready = false;
                        }
                        // Sin servicio: recorrido de carpetas + vigilancia de cambios.
                        slot.state.lock().mode = MODE_WATCH.to_string();
                        if !classic_ready {
                            if std::env::var_os("RAYO_NO_REBUILD").is_none() || slot.index.read().len() <= 1 {
                                mgr.build(&slot);
                            }
                            classic_ready = true;
                        }
                        if !watcher {
                            let mgr = mgr.clone();
                            let slot = slot.clone();
                            std::thread::spawn(move || watch::watch_volume(mgr, slot));
                            watcher = true;
                        }
                        if slot.needs_rescan.load(Ordering::Relaxed) {
                            mgr.build(&slot);
                        }
                        // Cada 10 s se revisa si apareció el servicio.
                        for _ in 0..20 {
                            std::thread::sleep(Duration::from_millis(500));
                            if mgr.shutdown.load(Ordering::Relaxed) || mgr.recheck.swap(false, Ordering::Relaxed) {
                                break;
                            }
                        }
                    }
                })
                .ok();
        }
    }

    /// Mantiene un disco al día con el USN Journal. Vuelve con error si el servicio
    /// deja de responder.
    fn run_usn(&self, slot: &Slot, mut backend: Backend) -> Result<(), String> {
        let letter = slot.letter;
        let rebuild = |backend: &mut Backend| -> Result<(), String> {
            slot.state.lock().building = true;
            self.wake_ui();
            let t = Instant::now();
            let r = backend.build(letter);
            slot.state.lock().building = false;
            let mut v = r?;
            v.source = Source::Mft;
            let _ = cache::save(&v);
            self.replace(slot, v, t.elapsed().as_millis() as u64);
            release_free_memory();
            Ok(())
        };
        let usable = {
            let v = slot.index.read();
            v.len() > 1 && v.source == Source::Mft && v.usn_journal != 0
        };
        if !usable {
            rebuild(&mut backend)?;
        }
        let mut applier = usn::Applier::default();
        let mut caught_up = false;
        let mut dirty = false;
        let mut last_save = Instant::now();
        while !self.shutdown.load(Ordering::Relaxed) {
            let (journal, from) = {
                let v = slot.index.read();
                (v.usn_journal, v.usn_next)
            };
            // Primero ponerse al día sin esperar; después, esperar cambios nuevos.
            let wait = if caught_up { 20 } else { 0 };
            match backend.changes(letter, journal, from, wait)? {
                service::Changes::Reset => {
                    rebuild(&mut backend)?;
                    applier = usn::Applier::default();
                    caught_up = false;
                }
                service::Changes::Batch(list, next) => {
                    if list.is_empty() {
                        caught_up = true;
                        if next > from {
                            slot.index.write().usn_next = next;
                        }
                    } else {
                        let mut touched = std::collections::HashSet::new();
                        {
                            let mut v = slot.index.write();
                            for c in &list {
                                if let Some(p) = applier.apply(&mut v, c) {
                                    touched.insert(p);
                                }
                            }
                            v.usn_next = next;
                            slot.state.lock().entries = v.live_len();
                        }
                        dirty = true;
                        self.changes.fetch_add(1, Ordering::Relaxed);
                        let paths: Vec<String> = {
                            let v = slot.index.read();
                            touched.iter().map(|&p| v.full_path(p)).collect()
                        };
                        self.notify_dirs(paths.iter().map(String::as_str));
                    }
                }
            }
            // Guardar de vez en cuando (por si la app se cierra de golpe).
            if dirty && last_save.elapsed() > Duration::from_secs(300) {
                let _ = cache::save(&slot.index.read());
                dirty = false;
                last_save = Instant::now();
            }
        }
        Ok(())
    }

    /// Guarda el índice de los discos al día por USN (al cerrar la app): al
    /// volver a abrir solo se leen los cambios desde aquí.
    pub fn save_all(&self) {
        for slot in &self.slots {
            if slot.usn_active.load(Ordering::Relaxed) {
                let _ = cache::save(&slot.index.read());
            }
        }
    }

    /// Modo de actualización de los discos, para la barra de estado.
    pub fn mode(&self) -> String {
        self.slots.first().map(|s| s.state.lock().mode.clone()).unwrap_or_default()
    }

    pub fn is_ready(&self) -> bool {
        self.slots.iter().any(|s| s.index.read().len() > 1)
    }

    pub fn search(&self, text: &str, within: Option<&str>, opts: SearchOpts) -> SearchOutcome {
        let t = Instant::now();
        let q = search::parse_query(text);
        let mut hits = Vec::new();
        let mut total = 0;
        let mut epochs = Vec::with_capacity(self.slots.len());
        // Se mantienen los índices bloqueados para lectura hasta terminar de ordenar.
        let guards: Vec<_> = self.slots.iter().map(|s| s.index.read()).collect();
        let limit = opts.limit.unwrap_or(usize::MAX);
        if !q.is_empty() {
            for (k, slot) in self.slots.iter().enumerate() {
                let v = &guards[k];
                epochs.push(slot.epoch.load(Ordering::Relaxed));
                let mut c = Compiled::new(&q);
                if let Some(w) = within {
                    if !w.to_ascii_uppercase().starts_with(slot.letter) {
                        continue;
                    }
                    match v.find_path(w) {
                        Some(id) if id != 0 => c.within = Some(id),
                        Some(_) => {}
                        None => continue,
                    }
                }
                let (h, n) = search::search_volume(v, &c, k as u8, limit);
                hits.extend(h);
                total += n;
            }
        }
        if opts.limit.is_some() || opts.sort == ResultSort::Relevance {
            search::rank(&mut hits, limit);
            if opts.desc {
                hits.reverse();
            }
        } else {
            let vols: Vec<&VolumeIndex> = guards.iter().map(|g| &**g).collect();
            let ranks: Vec<Option<Arc<Ranks>>> = if matches!(opts.sort, ResultSort::Name | ResultSort::Folder) {
                (0..vols.len()).map(|k| Some(self.ranks_of(k, vols[k]))).collect()
            } else {
                vec![None; vols.len()]
            };
            sort_hits(&mut hits, &vols, &ranks, opts.sort, opts.desc);
        }
        SearchOutcome { hits, total, elapsed: t.elapsed(), epochs }
    }

    pub fn slot_for(&self, path: &str) -> Option<&Arc<Slot>> {
        let c = path.trim_start_matches("\\\\?\\").chars().next()?.to_ascii_uppercase();
        self.slots.iter().find(|s| s.letter == c)
    }

    /// Tamaños (recursivos) de los hijos de una carpeta, por nombre en minúsculas.
    pub fn child_sizes(&self, dir: &str) -> Option<std::collections::HashMap<String, u64>> {
        let slot = self.slot_for(dir)?;
        let v = slot.index.read();
        if v.len() <= 1 {
            return None;
        }
        let d = v.find_path(dir)?;
        let mut m = std::collections::HashMap::new();
        for c in v.children(d) {
            m.insert(v.name(c).to_lowercase(), v.size[c as usize]);
        }
        Some(m)
    }

    /// Todos los archivos bajo `dir` (vista plana), con su ruta relativa. `None` si
    /// el disco no está indexado. Como mucho `limit`.
    pub fn files_under(&self, dir: &str, limit: usize) -> Option<Vec<crate::fsx::FileEntry>> {
        let slot = self.slot_for(dir)?;
        let v = slot.index.read();
        if v.len() <= 1 {
            return None;
        }
        let root = v.find_path(dir)?;
        let mut out = Vec::new();
        // (carpeta, ruta relativa, dentro de algo oculto)
        let mut stack = vec![(root, String::new(), false)];
        while let Some((d, rel, hidden_parent)) = stack.pop() {
            for c in v.children(d) {
                if v.is_deleted(c) {
                    continue;
                }
                let name = v.name(c);
                let r = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
                let hidden = hidden_parent || v.flags[c as usize] & (store::F_HIDDEN | store::F_SYSTEM) != 0;
                if v.is_dir(c) {
                    stack.push((c, r, hidden));
                } else {
                    out.push(crate::fsx::FileEntry {
                        lower: r.to_lowercase(),
                        ext: crate::fsx::ext_of(&name),
                        is_dir: false,
                        hidden,
                        size: v.size[c as usize],
                        dir_size: None,
                        mtime: v.mtime[c as usize],
                        ctime: 0,
                        cloud: crate::fsx::Cloud::None,
                        name: r,
                    });
                    if out.len() >= limit {
                        return Some(out);
                    }
                }
            }
        }
        Some(out)
    }

    /// Archivos bajo `dir` cuyo tamaño se repite (candidatos a duplicado), sacados
    /// del índice sin tocar el disco. `None` si ese disco no está indexado.
    pub fn dupe_candidates(&self, dir: &str, min_size: u64) -> Option<Vec<crate::fsx::dupes::Candidate>> {
        let slot = self.slot_for(dir)?;
        let v = slot.index.read();
        if v.len() <= 1 {
            return None;
        }
        let root = v.find_path(dir)?;
        let mut ids = Vec::new();
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            for c in v.children(d) {
                if v.is_deleted(c) {
                    continue;
                }
                if v.is_dir(c) {
                    stack.push(c);
                } else if v.size[c as usize] >= min_size.max(1) {
                    ids.push(c);
                }
            }
        }
        ids.sort_unstable_by_key(|&i| v.size[i as usize]);
        let size = |k: usize| v.size[ids[k] as usize];
        let out = (0..ids.len())
            .filter(|&k| (k > 0 && size(k - 1) == size(k)) || (k + 1 < ids.len() && size(k + 1) == size(k)))
            .map(|k| crate::fsx::dupes::Candidate { path: v.full_path(ids[k]), size: size(k), mtime: v.mtime[ids[k] as usize] })
            .collect();
        Some(out)
    }

    pub fn dir_size(&self, dir: &str) -> Option<u64> {
        let slot = self.slot_for(dir)?;
        let v = slot.index.read();
        v.find_path(dir).map(|d| v.size[d as usize])
    }

    pub fn total_entries(&self) -> usize {
        self.slots.iter().map(|s| s.state.lock().entries).sum()
    }

    pub fn building(&self) -> bool {
        self.slots.iter().any(|s| s.state.lock().building)
    }
}
