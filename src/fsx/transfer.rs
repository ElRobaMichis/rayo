//! Cola de copias propia (en lugar del diálogo de copia del Explorador).
//!
//! - Cada trabajo sabe qué discos físicos toca: los que comparten un disco esperan su turno (dos
//!   copias a la vez en un disco mecánico se estorban), y los que no comparten nada van a la vez.
//! - Entre discos sin cabezal (NVMe, SSD) se copian varios archivos a la vez; con un disco
//!   mecánico o un USB de por medio, de a uno y en el orden de las carpetas.
//! - Empieza a copiar enseguida, mientras termina de contar (no hay "Calculando…").
//! - Si lo que se copia no entra en el destino, avisa al contar, no a mitad de camino.
//! - En la red pide que el tráfico vaya comprimido (SMB), si el servidor lo admite.
//! - Pausa, cancelación, velocidad real y tiempo restante.
//! - Si el destino ya existe, pregunta: reemplazar, omitir o conservar ambos
//!   (con la opción de repetir la respuesta para el resto).
//! - Mover dentro del mismo disco es instantáneo (se renombra); entre discos
//!   se copia y después se borra el original.
//! - Rutas de más de 260 caracteres (formato `\\?\`).

use super::{disks, join, long_path, short_path};
use crossbeam_channel::{Receiver, Sender, bounded, select, unbounded};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
    /// Sacar de un comprimido (las fuentes son rutas de adentro o comprimidos enteros).
    Extract,
    /// Crear un ZIP (el destino es la ruta del ZIP).
    Compress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Waiting,
    Scanning,
    Running,
    Paused,
    /// Esperando que el usuario decida qué hacer con un archivo que ya existe.
    Conflict,
    Done,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Replace,
    Skip,
    KeepBoth,
}

#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub src: String,
    pub dst: String,
    pub src_size: u64,
    pub dst_size: u64,
    pub src_mtime: u32,
    pub dst_mtime: u32,
}

/// Estado visible de un trabajo (lo lee la interfaz).
#[derive(Debug, Clone)]
pub struct JobView {
    pub id: u64,
    pub op: Op,
    pub label: String,
    pub dest: String,
    pub state: State,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_files: usize,
    pub done_files: usize,
    pub current: String,
    pub bytes_per_sec: f64,
    pub errors: Vec<String>,
    pub conflict: Option<ConflictInfo>,
    /// Lo que quedó creado en el destino (origen, destino), para poder deshacerlo.
    /// Solo elementos nuevos: no incluye archivos que reemplazaron a otros.
    pub created: Vec<(String, String)>,
    /// Todavía se están contando los archivos (ya se copia, pero el total puede crecer).
    pub counting: bool,
    /// Cuántos archivos se copian a la vez (1 con discos mecánicos o USB).
    pub workers: usize,
}

impl JobView {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            if self.total_files == 0 { 0.0 } else { self.done_files as f32 / self.total_files as f32 }
        } else {
            (self.done_bytes as f64 / self.total_bytes as f64) as f32
        }
    }

    pub fn eta_secs(&self) -> Option<u64> {
        (!self.counting && self.bytes_per_sec > 1.0 && self.total_bytes > self.done_bytes).then(|| ((self.total_bytes - self.done_bytes) as f64 / self.bytes_per_sec) as u64)
    }

    pub fn finished(&self) -> bool {
        matches!(self.state, State::Done | State::Canceled)
    }
}

struct Job {
    view: Mutex<JobView>,
    /// Discos físicos que toca (fuentes y destino): define con qué otros trabajos puede ir a la vez.
    disks: Vec<String>,
    sources: Vec<String>,
    /// Ruta relativa en el destino de cada fuente (vacío = su nombre, directo en el destino).
    rels: Vec<String>,
    /// Qué hacer con los archivos que ya existen sin preguntar (sincronizar = reemplazar).
    policy: Option<Choice>,
    paused: AtomicBool,
    cancel: AtomicBool,
    answer: Mutex<Option<(Choice, bool)>>,
}

pub type OnDone = Arc<dyn Fn(Vec<String>) + Send + Sync>;

/// La cola de transferencias.
pub struct Transfers {
    jobs: Arc<Mutex<Vec<Arc<Job>>>>,
    tx: Sender<Arc<Job>>,
    /// Avisa al planificador que algo cambió (un trabajo terminó o se canceló uno en espera).
    nudge: Sender<()>,
    next_id: Mutex<u64>,
}

impl Transfers {
    /// `wake` repinta la interfaz; `on_done` recibe las carpetas que cambiaron.
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, on_done: OnDone) -> Self {
        let (tx, rx): (Sender<Arc<Job>>, Receiver<Arc<Job>>) = unbounded();
        let (nudge, nudged): (Sender<()>, Receiver<()>) = unbounded();
        let nudge_done = nudge.clone();
        std::thread::Builder::new()
            .name("copias".into())
            .spawn(move || schedule(rx, nudged, nudge_done, wake, on_done))
            .ok();
        Transfers { jobs: Default::default(), tx, nudge, next_id: Mutex::new(1) }
    }

    pub fn enqueue(&self, op: Op, sources: Vec<String>, dest_dir: String) -> u64 {
        let what = if sources.len() == 1 { format!("«{}»", super::file_name(&sources[0])) } else { tf!("comun.elementos", n = sources.len()) };
        let verb = if op == Op::Copy { t!("comun.copiando") } else { t!("comun.moviendo") };
        let label = tf!("copia.etiqueta", accion = verb, que = what, destino = super::file_name(&dest_dir).trim_end_matches(':'));
        self.push(op, sources, Vec::new(), dest_dir, None, label)
    }

    /// Copia cada fuente a una ruta relativa propia dentro de `dest_dir`,
    /// reemplazando lo que ya exista (para sincronizar carpetas).
    pub fn enqueue_sync(&self, pairs: Vec<(String, String)>, dest_dir: String) -> u64 {
        let label = tp!(pairs.len(), "copia.sincronizando", destino = super::file_name(&dest_dir).trim_end_matches(':'));
        let (sources, rels) = pairs.into_iter().unzip();
        self.push(Op::Copy, sources, rels, dest_dir, Some(Choice::Replace), label)
    }

    /// Extrae a `dest_dir`. Cada fuente es algo de adentro de un comprimido o un
    /// comprimido entero; para estos, `rels` dice en qué subcarpeta (vacío = directo).
    pub fn enqueue_extract(&self, sources: Vec<String>, rels: Vec<String>, dest_dir: String) -> u64 {
        let what = if sources.len() == 1 { format!("«{}»", super::file_name(&sources[0])) } else { tf!("comun.elementos", n = sources.len()) };
        let label = tf!("copia.extrayendo", que = what, destino = super::file_name(&dest_dir).trim_end_matches(':'));
        self.push(Op::Extract, sources, rels, dest_dir, None, label)
    }

    /// Comprime las fuentes en un ZIP nuevo.
    pub fn enqueue_compress(&self, sources: Vec<String>, zip_path: String) -> u64 {
        let label = tf!("comun.comprimiendo", nombre = super::file_name(&zip_path));
        self.push(Op::Compress, sources, Vec::new(), zip_path, None, label)
    }

    /// Devuelve cada elemento a una ruta completa (para deshacer un movimiento).
    pub fn enqueue_moves(&self, pairs: Vec<(String, String)>) -> Vec<u64> {
        let mut by_dir: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> = Default::default();
        for (from, to) in pairs {
            let Some(dir) = super::parent_of(&to) else { continue };
            let e = by_dir.entry(dir).or_default();
            e.0.push(from);
            e.1.push(super::file_name(&to));
        }
        by_dir
            .into_iter()
            .map(|(dir, (sources, rels))| {
                let label = tp!(sources.len(), "copia.deshaciendo", destino = super::file_name(&dir));
                self.push(Op::Move, sources, rels, dir, None, label)
            })
            .collect()
    }

    fn push(&self, op: Op, sources: Vec<String>, rels: Vec<String>, dest_dir: String, policy: Option<Choice>, label: String) -> u64 {
        let id = {
            let mut n = self.next_id.lock();
            *n += 1;
            *n
        };
        let disks = disks::disks_of(sources.iter().map(String::as_str).chain([dest_dir.as_str()]));
        let job = Arc::new(Job {
            disks,
            view: Mutex::new(JobView {
                id,
                op,
                label,
                dest: dest_dir,
                state: State::Waiting,
                total_bytes: 0,
                done_bytes: 0,
                total_files: 0,
                done_files: 0,
                current: String::new(),
                bytes_per_sec: 0.0,
                errors: Vec::new(),
                conflict: None,
                created: Vec::new(),
                counting: false,
                workers: 1,
            }),
            sources,
            rels,
            policy,
            paused: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            answer: Mutex::new(None),
        });
        self.jobs.lock().push(job.clone());
        let _ = self.tx.send(job);
        id
    }

    pub fn snapshot(&self) -> Vec<JobView> {
        self.jobs.lock().iter().map(|j| j.view.lock().clone()).collect()
    }

    fn find(&self, id: u64) -> Option<Arc<Job>> {
        self.jobs.lock().iter().find(|j| j.view.lock().id == id).cloned()
    }

    pub fn set_paused(&self, id: u64, paused: bool) {
        if let Some(j) = self.find(id) {
            j.paused.store(paused, Ordering::Relaxed);
            let mut v = j.view.lock();
            if paused && v.state == State::Running {
                v.state = State::Paused;
            } else if !paused && v.state == State::Paused {
                v.state = State::Running;
            }
        }
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.find(id) {
            j.cancel.store(true, Ordering::Relaxed);
            j.paused.store(false, Ordering::Relaxed);
            if j.view.lock().state == State::Conflict {
                *j.answer.lock() = Some((Choice::Skip, true));
            }
            let _ = self.nudge.send(());
        }
    }

    /// Respuesta a un conflicto; `all` = hacer lo mismo con los siguientes.
    pub fn resolve(&self, id: u64, choice: Choice, all: bool) {
        if let Some(j) = self.find(id) {
            *j.answer.lock() = Some((choice, all));
        }
    }

    /// Quita de la lista los trabajos terminados.
    pub fn clear_finished(&self) {
        self.jobs.lock().retain(|j| !j.view.lock().finished());
    }

    pub fn active(&self) -> bool {
        self.jobs.lock().iter().any(|j| !j.view.lock().finished())
    }
}

// ---------------------------------------------------------------- planificador

/// Arranca cada trabajo en cuanto sus discos están libres: los que comparten un disco físico van
/// uno después del otro (en el orden en que se pidieron) y los que no comparten nada, a la vez.
fn schedule(rx: Receiver<Arc<Job>>, nudged: Receiver<()>, nudge: Sender<()>, wake: Arc<dyn Fn() + Send + Sync>, on_done: OnDone) {
    let mut waiting: VecDeque<Arc<Job>> = VecDeque::new();
    let mut running: Vec<Arc<Job>> = Vec::new();
    let mut open = true;
    loop {
        select! {
            recv(rx) -> j => match j {
                Ok(j) => waiting.push_back(j),
                Err(_) => open = false,
            },
            recv(nudged) -> _ => {}
        }
        running.retain(|j| !j.view.lock().finished());
        let mut k = 0;
        while k < waiting.len() {
            let job = waiting[k].clone();
            if job.cancel.load(Ordering::Relaxed) {
                job.view.lock().state = State::Canceled;
                waiting.remove(k);
                wake();
                continue;
            }
            let shares = |other: &Arc<Job>| other.disks.iter().any(|d| job.disks.contains(d));
            // Ni con lo que está corriendo, ni adelantándose a uno anterior que espera el mismo disco.
            if running.iter().any(shares) || waiting.iter().take(k).any(shares) {
                k += 1;
                continue;
            }
            waiting.remove(k);
            running.push(job.clone());
            let (wake, on_done, nudge) = (wake.clone(), on_done.clone(), nudge.clone());
            let spawned = std::thread::Builder::new().name("copia".into()).spawn(move || {
                let touched = run_job(&job, &*wake);
                wake();
                on_done(touched);
                let _ = nudge.send(());
            });
            if spawned.is_err() {
                running.pop();
            }
        }
        if !open && waiting.is_empty() && running.is_empty() {
            return;
        }
    }
}

// ---------------------------------------------------------------- ejecución

#[derive(Clone)]
struct Item {
    src: String,
    /// Ruta relativa dentro del destino.
    rel: String,
    is_dir: bool,
    size: u64,
}

fn meta(p: &str) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(long_path(p)).ok()
}

fn mtime_of(m: &std::fs::Metadata) -> u32 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as u32)
}

fn volume_of(p: &str) -> String {
    let p = short_path(p);
    if let Some(rest) = p.strip_prefix(r"\\") {
        let mut parts = rest.splitn(3, '\\');
        format!(r"\\{}\{}", parts.next().unwrap_or(""), parts.next().unwrap_or("")).to_lowercase()
    } else {
        p.get(..2).unwrap_or("").to_lowercase()
    }
}

/// Recorre las fuentes y arma la lista de carpetas y archivos a copiar.
fn scan(sources: &[(String, String)], cancel: &AtomicBool, errors: &mut Vec<String>) -> Vec<Item> {
    let mut out = Vec::new();
    scan_each(sources, cancel, errors, &mut |it| out.push(it));
    out
}

/// Igual que `scan`, pero entrega cada elemento apenas lo encuentra (cada carpeta antes que lo de
/// adentro), para empezar a copiar sin esperar a terminar de contar.
fn scan_each(sources: &[(String, String)], cancel: &AtomicBool, errors: &mut Vec<String>, emit: &mut dyn FnMut(Item)) {
    for (src, name) in sources {
        let name = name.clone();
        let Some(m) = meta(src) else {
            errors.push(tf!("copia.no_existe", origen = src));
            continue;
        };
        if !m.is_dir() {
            emit(Item { src: src.clone(), rel: name, is_dir: false, size: m.len() });
            continue;
        }
        emit(Item { src: src.clone(), rel: name.clone(), is_dir: true, size: 0 });
        let mut stack = vec![(src.clone(), name)];
        while let Some((dir, rel)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match super::list_dir(&dir) {
                Ok(list) => {
                    // Primero los archivos de la carpeta, después las subcarpetas: en un disco
                    // mecánico se lee cada carpeta de corrido.
                    let mut subdirs = Vec::new();
                    for e in list {
                        let s = join(&dir, &e.name);
                        let r = format!("{rel}\\{}", e.name);
                        if e.is_dir {
                            subdirs.push((s, r));
                        } else {
                            emit(Item { src: s, rel: r, is_dir: false, size: e.size });
                        }
                    }
                    for (s, r) in subdirs.into_iter().rev() {
                        emit(Item { src: s.clone(), rel: r.clone(), is_dir: true, size: 0 });
                        stack.push((s, r));
                    }
                }
                Err(e) => errors.push(format!("{dir}: {e}")),
            }
        }
    }
}

/// Espacio libre para el usuario en el disco de una ruta.
fn free_space(path: &str) -> Option<u64> {
    let w: Vec<u16> = long_path(path).encode_utf16().chain([0]).collect();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), None, None) }.ok().map(|_| free)
}

/// "foto.jpg" → "foto (2).jpg", "foto (3).jpg"… hasta encontrar un nombre libre.
pub fn free_name(dst: &str) -> String {
    let dir = super::parent_of(dst).unwrap_or_default();
    let name = super::file_name(dst);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for k in 2..10_000 {
        let candidate = join(&dir, &format!("{stem} ({k}){ext}"));
        if meta(&candidate).is_none() {
            return candidate;
        }
    }
    dst.to_string()
}

struct Progress<'a> {
    job: &'a Job,
    /// Bytes copiados de todo el trabajo (lo suman todos los archivos en curso).
    total: &'a AtomicU64,
    /// Lo que ya se sumó de este archivo.
    last: AtomicU64,
    started: Instant,
    wake: &'a (dyn Fn() + Sync),
    last_wake: Mutex<Instant>,
}

unsafe extern "system" fn progress_cb(
    _total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_done: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const core::ffi::c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    let p = unsafe { &*(data as *const Progress) };
    let t = transferred as u64;
    let delta = t.saturating_sub(p.last.swap(t, Ordering::Relaxed));
    let done = p.total.fetch_add(delta, Ordering::Relaxed) + delta;
    {
        let mut v = p.job.view.lock();
        v.done_bytes = done;
        v.bytes_per_sec = done as f64 / p.started.elapsed().as_secs_f64().max(0.001);
    }
    // Pausa: se queda esperando aquí, en medio del archivo.
    while p.job.paused.load(Ordering::Relaxed) && !p.job.cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    if p.job.cancel.load(Ordering::Relaxed) {
        return PROGRESS_CANCEL;
    }
    let mut last = p.last_wake.lock();
    if last.elapsed() > Duration::from_millis(100) {
        *last = Instant::now();
        (p.wake)();
    }
    PROGRESS_CONTINUE
}

/// Desde qué tamaño se copia sin la caché de Windows. Con caché la copia termina antes (Windows
/// sigue escribiendo por detrás, como hace el Explorador), pero un archivo enorme desplazaría de
/// la memoria todo lo demás: sin caché solo los que pasan de la mitad de la memoria libre.
fn unbuffered_from() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    if let Some(mb) = std::env::var("RAYO_COPY_UNBUFFERED_MB").ok().and_then(|v| v.parse::<u64>().ok()) {
        return mb << 20;
    }
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    let free = if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() { m.ullAvailPhys } else { 0 };
    (free / 2).max(512 << 20)
}

/// Copia un archivo sumando lo copiado a `total`. Al volver, el archivo cuenta entero (también si
/// falló: así la barra no se queda corta).
fn copy_file(job: &Job, src: &str, dst: &str, size: u64, total: &AtomicU64, started: Instant, compress: bool, wake: &(dyn Fn() + Sync)) -> Result<(), String> {
    let s: Vec<u16> = long_path(src).encode_utf16().chain([0]).collect();
    let d: Vec<u16> = long_path(dst).encode_utf16().chain([0]).collect();
    let progress = Progress { job, total, last: AtomicU64::new(0), started, wake, last_wake: Mutex::new(Instant::now()) };
    // Sin caché del sistema para archivos grandes: más rápido y no desplaza lo que hay en memoria.
    let mut flags = if size > unbuffered_from() { COPY_FILE_NO_BUFFERING } else { COPYFILE_FLAGS(0) };
    if compress {
        // En la red: que SMB comprima lo que viaja (si el servidor no sabe, se ignora).
        flags |= COPY_FILE_REQUEST_COMPRESSED_TRAFFIC;
    }
    let run = |flags: COPYFILE_FLAGS| unsafe { CopyFileExW(PCWSTR(s.as_ptr()), PCWSTR(d.as_ptr()), Some(progress_cb), Some(&progress as *const _ as *const _), None, flags) };
    let mut r = run(flags);
    if compress && r.as_ref().is_err_and(|e| e.code() == windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult()) {
        // Windows viejo: sin compresión.
        r = run(flags & !COPY_FILE_REQUEST_COMPRESSED_TRAFFIC);
    }
    let counted = progress.last.load(Ordering::Relaxed);
    total.fetch_add(size.saturating_sub(counted), Ordering::Relaxed);
    r.map_err(|e| e.message())
}

fn move_path(src: &str, dst: &str) -> Result<(), String> {
    let s: Vec<u16> = long_path(src).encode_utf16().chain([0]).collect();
    let d: Vec<u16> = long_path(dst).encode_utf16().chain([0]).collect();
    unsafe { MoveFileExW(PCWSTR(s.as_ptr()), PCWSTR(d.as_ptr()), MOVE_FILE_FLAGS(0)) }.map_err(|e| e.message())
}

/// Pregunta qué hacer con un archivo que ya existe y espera la respuesta.
fn ask(job: &Job, info: ConflictInfo, wake: &dyn Fn()) -> (Choice, bool) {
    {
        let mut v = job.view.lock();
        v.state = State::Conflict;
        v.conflict = Some(info);
    }
    *job.answer.lock() = None;
    wake();
    loop {
        if let Some(a) = job.answer.lock().take() {
            let mut v = job.view.lock();
            v.state = State::Running;
            v.conflict = None;
            return a;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Progreso compartido por extraer y comprimir: suma bytes y repinta como mucho cada 100 ms.
fn progress_fn<'a>(job: &'a Job, wake: &'a dyn Fn(), started: Instant) -> impl Fn(u64) + 'a {
    let last = Mutex::new(Instant::now());
    move |n: u64| {
        {
            let mut v = job.view.lock();
            v.done_bytes += n;
            v.bytes_per_sec = v.done_bytes as f64 / started.elapsed().as_secs_f64().max(0.001);
        }
        let mut l = last.lock();
        if l.elapsed() > Duration::from_millis(100) {
            *l = Instant::now();
            wake();
        }
    }
}

fn finish(job: &Job, errors: Vec<String>, created: Vec<(String, String)>, total: u64) {
    let mut v = job.view.lock();
    v.state = if job.cancel.load(Ordering::Relaxed) { State::Canceled } else { State::Done };
    v.current.clear();
    v.errors = errors;
    v.created = created;
    if v.state == State::Done {
        v.done_bytes = v.done_bytes.max(total);
    }
}

fn run_extract(job: &Job, wake: &dyn Fn()) -> Vec<String> {
    use super::archive;
    let dest = {
        let mut v = job.view.lock();
        v.state = State::Scanning;
        v.dest.clone()
    };
    wake();
    let _ = std::fs::create_dir_all(long_path(&dest));
    let mut errors = Vec::new();
    // Por comprimido: (ruta de adentro, destino).
    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (k, src) in job.sources.iter().enumerate() {
        let Some((arch, inner)) = archive::split(src) else {
            errors.push(tf!("copia.no_comprimido", origen = src));
            continue;
        };
        let mut pairs = Vec::new();
        if inner.is_empty() {
            let base = match job.rels.get(k) {
                Some(r) if !r.is_empty() => join(&dest, r),
                _ => dest.clone(),
            };
            match archive::list(&arch, "") {
                Ok(list) => pairs.extend(list.into_iter().map(|e| (e.name.clone(), join(&base, &e.name)))),
                Err(e) => {
                    errors.push(format!("{}: {e}", super::file_name(&arch)));
                    continue;
                }
            }
        } else {
            pairs.push((inner.clone(), join(&dest, &super::file_name(&inner))));
        }
        match groups.iter_mut().find(|g| g.0.eq_ignore_ascii_case(&arch)) {
            Some(g) => g.1.extend(pairs),
            None => groups.push((arch, pairs)),
        }
    }
    // Lo que ya existe: preguntar (o seguir la respuesta para todos).
    let mut policy = job.policy;
    let mut created = Vec::new();
    for (arch, pairs) in &mut groups {
        let mut keep = Vec::new();
        for (inner, target) in std::mem::take(pairs) {
            if job.cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut target = target;
            let src = join(arch, &inner);
            if let Some(dm) = meta(&target) {
                let choice = match policy {
                    Some(p) => p,
                    None => {
                        let e = archive::find(arch, &inner);
                        let (c, all) = ask(
                            job,
                            ConflictInfo {
                                src: src.clone(),
                                dst: target.clone(),
                                src_size: archive::size_of(arch, &inner),
                                dst_size: dm.len(),
                                src_mtime: e.map_or(0, |e| e.mtime),
                                dst_mtime: mtime_of(&dm),
                            },
                            wake,
                        );
                        if all {
                            policy = Some(c);
                        }
                        c
                    }
                };
                match choice {
                    Choice::Skip => continue,
                    Choice::KeepBoth => {
                        target = free_name(&target);
                        created.push((src, target.clone()));
                    }
                    Choice::Replace => {}
                }
            } else {
                created.push((src, target.clone()));
            }
            keep.push((inner, target));
        }
        *pairs = keep;
    }
    let total: u64 = groups.iter().flat_map(|(a, p)| p.iter().map(move |(i, _)| super::archive::size_of(a, i))).sum();
    {
        let mut v = job.view.lock();
        v.state = State::Running;
        v.total_bytes = total;
        v.total_files = groups.iter().map(|g| g.1.len()).sum();
    }
    wake();
    let progress = progress_fn(job, wake, Instant::now());
    for (arch, pairs) in &groups {
        if pairs.is_empty() || job.cancel.load(Ordering::Relaxed) {
            continue;
        }
        job.view.lock().current = super::file_name(arch);
        if let Err(e) = archive::extract(arch, pairs, &progress, &job.cancel) {
            if e != "cancelado" {
                errors.push(format!("{}: {e}", super::file_name(arch)));
            }
        }
    }
    finish(job, errors, created, total);
    vec![dest]
}

fn run_compress(job: &Job, wake: &dyn Fn()) -> Vec<String> {
    let zip_path = {
        let mut v = job.view.lock();
        v.state = State::Scanning;
        v.dest.clone()
    };
    wake();
    let mut errors = Vec::new();
    let pairs: Vec<(String, String)> = job.sources.iter().map(|s| (s.clone(), super::file_name(s))).collect();
    let items = scan(&pairs, &job.cancel, &mut errors);
    let total: u64 = items.iter().map(|i| i.size).sum();
    {
        let mut v = job.view.lock();
        v.state = State::Running;
        v.total_bytes = total;
        v.total_files = items.iter().filter(|i| !i.is_dir).count();
        v.current = super::file_name(&zip_path);
    }
    wake();
    let list: Vec<(String, String, bool)> = items.iter().map(|i| (i.src.clone(), i.rel.clone(), i.is_dir)).collect();
    let progress = progress_fn(job, wake, Instant::now());
    let mut created = Vec::new();
    match super::archive::compress(&list, &zip_path, &progress, &job.cancel) {
        Ok(()) => created.push((zip_path.clone(), zip_path.clone())),
        Err(e) if e == "cancelado" => {}
        Err(e) => errors.push(e),
    }
    finish(job, errors, created, total);
    super::parent_of(&zip_path).into_iter().collect()
}

fn run_job(job: &Job, wake: &(dyn Fn() + Sync)) -> Vec<String> {
    // Leer la operación aparte: el candado no debe quedar tomado durante el trabajo.
    let op = job.view.lock().op;
    match op {
        Op::Extract => return run_extract(job, wake),
        Op::Compress => return run_compress(job, wake),
        Op::Copy | Op::Move => {}
    }
    let (op, dest) = {
        let mut v = job.view.lock();
        v.state = State::Scanning;
        (v.op, v.dest.clone())
    };
    wake();
    let _ = std::fs::create_dir_all(long_path(&dest));
    let mut touched = vec![dest.clone()];
    touched.extend(job.sources.iter().filter_map(|s| super::parent_of(s)));
    touched.extend(job.rels.iter().filter_map(|r| r.rfind('\\').map(|i| join(&dest, &r[..i]))));
    let mut errors = Vec::new();

    // Copiar una carpeta dentro de sí misma no tiene sentido (sería infinito).
    let dest_l = format!("{}\\", short_path(&dest).trim_end_matches('\\').to_lowercase());
    let rel_of = |k: usize| job.rels.get(k).cloned().unwrap_or_else(|| super::file_name(&job.sources[k]));
    let sources: Vec<(String, String)> = job
        .sources
        .iter()
        .enumerate()
        .map(|(k, s)| (s.clone(), rel_of(k)))
        .filter(|(s, _)| {
            let s_l = format!("{}\\", short_path(s).trim_end_matches('\\').to_lowercase());
            let inside = dest_l.starts_with(&s_l);
            if inside {
                errors.push(tf!("copia.no_puede_copiar_dentro_si", nombre = super::file_name(s)));
            }
            !inside
        })
        .collect();

    // Mover dentro del mismo disco: se renombra cada elemento (instantáneo).
    let mut created: Vec<(String, String)> = Vec::new();
    let mut pending = sources.clone();
    if op == Op::Move && volume_of(&dest) != "" {
        pending.retain(|(src, rel)| {
            if volume_of(src) != volume_of(&dest) {
                return true;
            }
            let dst = join(&dest, rel);
            if short_path(src).eq_ignore_ascii_case(&short_path(&dst)) {
                return false;
            }
            if meta(&dst).is_some() {
                return true; // Ya existe: se resuelve archivo por archivo más abajo.
            }
            match move_path(src, &dst) {
                Ok(()) => {
                    created.push((src.clone(), dst));
                    false
                }
                Err(e) => {
                    errors.push(format!("{src}: {e}"));
                    false
                }
            }
        });
    }

    // Estrategia según los discos: varios archivos a la vez solo si ningún disco tiene cabezal
    // (NVMe, SSD o red); con un disco mecánico o un USB, de a uno y en el orden de las carpetas.
    let dst_vol = disks::volume(&dest);
    let src_vols: Vec<disks::Volume> = pending.iter().map(|(s, _)| disks::volume(s)).collect();
    let network = dst_vol.kind == disks::Kind::Network || src_vols.iter().any(|v| v.kind == disks::Kind::Network);
    let parallel = dst_vol.kind.parallel_friendly() && src_vols.iter().all(|v| v.kind.parallel_friendly());
    let auto = if !parallel {
        1
    } else if network {
        4
    } else {
        std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(4, 8)
    };
    // RAYO_COPY_THREADS=n fuerza la cantidad (para medir y diagnosticar).
    let workers = std::env::var("RAYO_COPY_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(auto);
    // Si no entra, avisar al contar (no cuando el disco ya se llenó a mitad de camino).
    let needs_space = op == Op::Copy || src_vols.iter().any(|v| v.disks != dst_vol.disks);
    let free = if needs_space { free_space(&dest) } else { None };
    {
        let mut v = job.view.lock();
        v.state = State::Running;
        v.counting = true;
        v.workers = workers;
    }
    wake();

    struct Task {
        src: String,
        rel: String,
        target: String,
        size: u64,
        /// Anotarlo para deshacer (no si reemplazó a otro o si está dentro de una carpeta nueva).
        record: bool,
    }
    struct Shared {
        errors: Vec<String>,
        created: Vec<(String, String)>,
        /// Originales copiados bien (al mover, se borran al final).
        copied: Vec<String>,
        /// Carpetas de origen (al mover, se borran las que quedaron vacías).
        dirs: Vec<String>,
    }
    let started = Instant::now();
    let done = AtomicU64::new(0);
    let out_of_space = AtomicBool::new(false);
    let shared = Mutex::new(Shared { errors, created, copied: Vec::new(), dirs: Vec::new() });
    let (job_r, done_r, shared_r, oos_r, pending_r, dest_r) = (job, &done, &shared, &out_of_space, &pending, dest.as_str());

    std::thread::scope(|sc| {
        // 1. Contar: entrega cada carpeta y archivo apenas lo encuentra.
        let (item_tx, item_rx) = bounded::<Item>(4096);
        sc.spawn(move || {
            let mut errs = Vec::new();
            let (mut bytes, mut files) = (0u64, 0usize);
            scan_each(pending_r, &job_r.cancel, &mut errs, &mut |it: Item| {
                if !it.is_dir {
                    bytes += it.size;
                    files += 1;
                    let mut v = job_r.view.lock();
                    v.total_bytes = bytes;
                    v.total_files = files;
                }
                if free.is_some_and(|f| bytes > f) && !oos_r.swap(true, Ordering::Relaxed) {
                    let libre = super::human_size(free.unwrap_or(0));
                    shared_r.lock().errors.push(tf!("copia.sin_espacio", destino = super::file_name(dest_r).trim_end_matches(':'), libre = libre));
                    job_r.cancel.store(true, Ordering::Relaxed);
                }
                let _ = item_tx.send(it);
            });
            shared_r.lock().errors.extend(errs);
            job_r.view.lock().counting = false;
            wake();
        });

        // 2. Copiar: uno o varios a la vez.
        let (task_tx, task_rx) = bounded::<Task>(workers * 4);
        for _ in 0..workers {
            let task_rx = task_rx.clone();
            sc.spawn(move || {
                for t in task_rx {
                    // Con la tarea cancelada se vacía la cola sin copiar (quien reparte no se traba).
                    while job_r.paused.load(Ordering::Relaxed) && !job_r.cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if job_r.cancel.load(Ordering::Relaxed) {
                        continue;
                    }
                    job_r.view.lock().current = t.rel.clone();
                    match copy_file(job_r, &t.src, &t.target, t.size, done_r, started, network, wake) {
                        Ok(()) => {
                            let mut sh = shared_r.lock();
                            sh.copied.push(t.src.clone());
                            if t.record {
                                sh.created.push((t.src, t.target));
                            }
                        }
                        Err(_) if job_r.cancel.load(Ordering::Relaxed) => {
                            // Archivo a medias: se borra.
                            let _ = std::fs::remove_file(long_path(&t.target));
                        }
                        Err(e) => shared_r.lock().errors.push(format!("{}: {e}", t.src)),
                    }
                    let d = done_r.load(Ordering::Relaxed);
                    let mut v = job_r.view.lock();
                    v.done_files += 1;
                    v.done_bytes = d;
                    v.bytes_per_sec = d as f64 / started.elapsed().as_secs_f64().max(0.001);
                }
            });
        }
        drop(task_rx);

        // 3. Repartir (este hilo): carpetas, conflictos y nombres, en orden.
        let mut policy: Option<Choice> = job.policy;
        // Carpetas de destino ya renombradas por "conservar ambos": origen → destino.
        let mut renamed_dirs: Vec<(String, String)> = Vec::new();
        // Fuentes de primer nivel y carpetas nuevas que crearon (lo de adentro se deshace con ellas).
        let tops: std::collections::HashSet<&str> = pending.iter().map(|(s, _)| s.as_str()).collect();
        let mut created_tops: Vec<String> = Vec::new();
        for it in item_rx {
            if job.cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut dst = join(&dest, &it.rel);
            for (from, to) in &renamed_dirs {
                if let Some(rest) = dst.strip_prefix(from.as_str()) {
                    if rest.starts_with('\\') {
                        dst = format!("{to}{rest}");
                        break;
                    }
                }
            }
            if it.is_dir {
                shared.lock().dirs.push(it.src.clone());
                let same = short_path(&it.src).eq_ignore_ascii_case(&short_path(&dst));
                if same && op == Op::Copy {
                    // Copiar una carpeta en su misma ubicación: "Carpeta (2)".
                    let new_dst = free_name(&dst);
                    renamed_dirs.push((dst.clone(), new_dst.clone()));
                    dst = new_dst;
                }
                let existed = meta(&dst).is_some();
                match std::fs::create_dir_all(long_path(&dst)) {
                    Err(e) => shared.lock().errors.push(format!("{dst}: {e}")),
                    Ok(()) if !existed && tops.contains(it.src.as_str()) => {
                        shared.lock().created.push((it.src.clone(), dst.clone()));
                        created_tops.push(format!("{}\\", dst.to_lowercase()));
                    }
                    Ok(()) => {}
                }
                continue;
            }
            let mut target = dst.clone();
            let mut replaced = false;
            if let Some(dm) = meta(&dst) {
                let same = short_path(&it.src).eq_ignore_ascii_case(&short_path(&dst));
                let choice = if same {
                    // Pegar en la misma carpeta: siempre una copia nueva.
                    Choice::KeepBoth
                } else if let Some(p) = policy {
                    p
                } else {
                    let sm = meta(&it.src);
                    let (c, all) = ask(
                        job,
                        ConflictInfo {
                            src: it.src.clone(),
                            dst: dst.clone(),
                            src_size: it.size,
                            dst_size: dm.len(),
                            src_mtime: sm.as_ref().map_or(0, mtime_of),
                            dst_mtime: mtime_of(&dm),
                        },
                        wake,
                    );
                    if all {
                        policy = Some(c);
                    }
                    c
                };
                match choice {
                    Choice::Skip => {
                        let d = done.fetch_add(it.size, Ordering::Relaxed) + it.size;
                        let mut v = job.view.lock();
                        v.done_bytes = d;
                        v.done_files += 1;
                        continue;
                    }
                    Choice::KeepBoth => target = free_name(&dst),
                    Choice::Replace => replaced = true,
                }
            }
            if let Some(parent) = super::parent_of(&target) {
                let _ = std::fs::create_dir_all(long_path(&parent));
            }
            let inside_new = created_tops.iter().any(|d| target.to_lowercase().starts_with(d.as_str()));
            let task = Task { src: it.src.clone(), rel: it.rel.clone(), target, size: it.size, record: !replaced && !inside_new };
            if task_tx.send(task).is_err() {
                break;
            }
        }
        drop(task_tx);
    });

    let Shared { mut errors, created, copied, dirs } = shared.into_inner();
    let out_of_space = out_of_space.load(Ordering::Relaxed);

    // Mover entre discos: borrar los originales copiados bien y las carpetas que quedaron vacías.
    if op == Op::Move && !job.cancel.load(Ordering::Relaxed) {
        for src in &copied {
            if let Err(e) = std::fs::remove_file(long_path(src)) {
                errors.push(tf!("copia.no_pudo_borrar_original", origen = src, error = e));
            }
        }
        let mut dirs = dirs;
        dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
        for d in dirs {
            let _ = std::fs::remove_dir(long_path(&d));
        }
    }

    let mut v = job.view.lock();
    // Sin espacio no es una cancelación del usuario: termina con su error a la vista.
    v.state = if job.cancel.load(Ordering::Relaxed) && !out_of_space { State::Canceled } else { State::Done };
    v.current.clear();
    v.counting = false;
    v.errors = errors;
    v.created = created;
    let total_bytes = v.total_bytes;
    v.done_bytes = v.done_bytes.max(if v.state == State::Done && !out_of_space { total_bytes } else { 0 });
    touched
}


pub fn exists(p: &str) -> bool {
    Path::new(&long_path(p)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("rayo-transfer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    fn wait(t: &Transfers, id: u64) -> JobView {
        let mut v = None;
        for _ in 0..3000 {
            let v = v.insert(t.snapshot().into_iter().find(|j| j.id == id).unwrap());
            if v.state == State::Conflict {
                t.resolve(id, Choice::KeepBoth, true);
            }
            if v.finished() {
                return v.clone();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let all: Vec<(u64, State)> = t.snapshot().iter().map(|j| (j.id, j.state)).collect();
        panic!("el trabajo no terminó: {v:#?}\ntodos: {all:?}");
    }

    /// Muchos archivos en carpetas anidadas (en un SSD se copian varios a la vez): llegan todos,
    /// completos, y el progreso suma exactamente lo copiado.
    #[test]
    fn many_files_arrive_complete() {
        let root = tmp("muchos");
        let src = join(&root, "origen");
        let mut expected = Vec::new();
        for d in 0..12 {
            let dir = join(&src, &format!(r"sub{d}\nivel"));
            std::fs::create_dir_all(long_path(&dir)).unwrap();
            for f in 0..40 {
                let body = format!("archivo {d}-{f} ").repeat(1 + (f * 37) % 300);
                std::fs::write(long_path(&join(&dir, &format!("f{f}.txt"))), &body).unwrap();
                expected.push((format!(r"origen\sub{d}\nivel\f{f}.txt"), body));
            }
        }
        let dest = join(&root, "destino");
        let t = Transfers::new(Arc::new(|| {}), Arc::new(|_| {}));
        let id = t.enqueue(Op::Copy, vec![src.clone()], dest.clone());
        let v = wait(&t, id);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(v.done_files, expected.len());
        assert_eq!(v.done_bytes, v.total_bytes);
        assert!(!v.counting);
        for (rel, body) in &expected {
            assert_eq!(&std::fs::read_to_string(long_path(&join(&dest, rel))).unwrap(), body, "{rel}");
        }
        // Deshacer quita la carpeta nueva entera (una sola entrada).
        assert_eq!(v.created.len(), 1, "{:?}", v.created);
        println!("{} archivos, {} a la vez", v.done_files, v.workers);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Dos trabajos sobre el mismo disco van uno después del otro y terminan los dos bien.
    #[test]
    fn jobs_on_the_same_disk_run_in_order() {
        let root = tmp("orden");
        let src = join(&root, "a.bin");
        std::fs::write(long_path(&src), vec![7u8; 1 << 20]).unwrap();
        let (d1, d2) = (join(&root, "uno"), join(&root, "dos"));
        println!("{root}: {:?}", disks::volume(&root));
        let t = Transfers::new(Arc::new(|| {}), Arc::new(|_| {}));
        let a = t.enqueue(Op::Copy, vec![src.clone()], d1.clone());
        let b = t.enqueue(Op::Copy, vec![src.clone()], d2.clone());
        let (va, vb) = (wait(&t, a), wait(&t, b));
        assert!(va.errors.is_empty() && vb.errors.is_empty(), "{:?} {:?}", va.errors, vb.errors);
        assert_eq!(std::fs::read(long_path(&join(&d1, "a.bin"))).unwrap().len(), 1 << 20);
        assert_eq!(std::fs::read(long_path(&join(&d2, "a.bin"))).unwrap().len(), 1 << 20);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copies_moves_and_handles_long_paths() {
        let root = tmp("a");
        let src = join(&root, "origen");
        // Ruta de más de 260 caracteres.
        let deep = (0..8).fold(src.clone(), |p, k| join(&p, &format!("carpeta-muy-larga-con-nombre-extenso-{k:02}")));
        assert!(deep.len() > 260, "{}", deep.len());
        std::fs::create_dir_all(long_path(&deep)).unwrap();
        std::fs::write(long_path(&join(&deep, "dato.txt")), b"hola rayo").unwrap();
        std::fs::write(long_path(&join(&src, "a.txt")), b"12345").unwrap();
        let dest = join(&root, "destino");
        std::fs::create_dir_all(&dest).unwrap();

        let t = Transfers::new(Arc::new(|| {}), Arc::new(|_| {}));
        let id = t.enqueue(Op::Copy, vec![src.clone()], dest.clone());
        let v = wait(&t, id);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(v.total_files, 2);
        let copied_deep = deep.replacen(&src, &join(&dest, "origen"), 1);
        assert_eq!(std::fs::read(long_path(&join(&copied_deep, "dato.txt"))).unwrap(), b"hola rayo");

        // Copiar otra vez: conflicto → conservar ambos.
        let id = t.enqueue(Op::Copy, vec![join(&src, "a.txt")], join(&dest, "origen"));
        let v = wait(&t, id);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert!(exists(&join(&dest, r"origen\a (2).txt")));

        // Mover en el mismo disco: instantáneo y el original desaparece.
        let id = t.enqueue(Op::Move, vec![src.clone()], join(&dest, "otra"));
        std::fs::create_dir_all(join(&dest, "otra")).unwrap();
        let v = wait(&t, id);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert!(!exists(&src));
        assert!(exists(&join(&dest, r"otra\origen\a.txt")));

        // Una carpeta no se puede copiar dentro de sí misma.
        let inner = join(&dest, r"otra\origen");
        let id = t.enqueue(Op::Copy, vec![inner.clone()], join(&inner, "x"));
        let v = wait(&t, id);
        assert_eq!(v.errors.len(), 1);
        let _ = std::fs::remove_dir_all(long_path(&root));
    }

    #[test]
    fn sync_replaces_without_asking() {
        let root = tmp("sync");
        let (l, r) = (join(&root, "izq"), join(&root, "der"));
        std::fs::create_dir_all(join(&l, "sub")).unwrap();
        std::fs::create_dir_all(join(&r, "sub")).unwrap();
        std::fs::write(join(&l, r"sub\x.txt"), b"nuevo").unwrap();
        std::fs::write(join(&r, r"sub\x.txt"), b"viejo").unwrap();
        std::fs::write(join(&l, "solo.txt"), b"s").unwrap();
        let t = Transfers::new(Arc::new(|| {}), Arc::new(|_| {}));
        let pairs = vec![(join(&l, r"sub\x.txt"), r"sub\x.txt".to_string()), (join(&l, "solo.txt"), "solo.txt".to_string())];
        let id = t.enqueue_sync(pairs, r.clone());
        // Nunca debe pedir respuesta: si lo hiciera, `wait` elegiría "conservar ambos".
        let v = wait(&t, id);
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(std::fs::read(join(&r, r"sub\x.txt")).unwrap(), b"nuevo");
        assert!(!exists(&join(&r, r"sub\x (2).txt")));
        assert!(exists(&join(&r, "solo.txt")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compress_then_extract_through_the_queue() {
        let root = tmp("zipcola");
        let src = join(&root, "datos");
        std::fs::create_dir_all(join(&src, "sub")).unwrap();
        std::fs::write(join(&src, "a.txt"), b"uno").unwrap();
        std::fs::write(join(&src, r"sub\b.txt"), b"dos").unwrap();
        let t = Transfers::new(Arc::new(|| {}), Arc::new(|_| {}));
        let zip = join(&root, "datos.zip");
        let v = wait(&t, t.enqueue_compress(vec![src.clone()], zip.clone()));
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(v.created, vec![(zip.clone(), zip.clone())]);
        // El comprimido entero en una carpeta, y una carpeta de adentro suelta.
        let out = join(&root, "salida");
        let v = wait(&t, t.enqueue_extract(vec![zip.clone(), join(&zip, r"datos\sub")], vec!["todo".into(), String::new()], out.clone()));
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert_eq!(std::fs::read(join(&out, r"todo\datos\a.txt")).unwrap(), b"uno");
        assert_eq!(std::fs::read(join(&out, r"sub\b.txt")).unwrap(), b"dos");
        assert_eq!(v.created.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
