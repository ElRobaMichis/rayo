//! Cola de copias propia (en lugar del diálogo de copia del Explorador).
//!
//! - Los trabajos se ejecutan de a uno, en orden, en un hilo propio.
//! - Pausa, cancelación, velocidad real y tiempo restante.
//! - Si el destino ya existe, pregunta: reemplazar, omitir o conservar ambos
//!   (con la opción de repetir la respuesta para el resto).
//! - Mover dentro del mismo disco es instantáneo (se renombra); entre discos
//!   se copia y después se borra el original.
//! - Rutas de más de 260 caracteres (formato `\\?\`).

use super::{join, long_path, short_path};
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
        (self.bytes_per_sec > 1.0 && self.total_bytes > self.done_bytes).then(|| ((self.total_bytes - self.done_bytes) as f64 / self.bytes_per_sec) as u64)
    }

    pub fn finished(&self) -> bool {
        matches!(self.state, State::Done | State::Canceled)
    }
}

struct Job {
    view: Mutex<JobView>,
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
    next_id: Mutex<u64>,
}

impl Transfers {
    /// `wake` repinta la interfaz; `on_done` recibe las carpetas que cambiaron.
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>, on_done: OnDone) -> Self {
        let (tx, rx): (Sender<Arc<Job>>, Receiver<Arc<Job>>) = unbounded();
        std::thread::Builder::new()
            .name("copias".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    if job.cancel.load(Ordering::Relaxed) {
                        job.view.lock().state = State::Canceled;
                        wake();
                        continue;
                    }
                    let touched = run_job(&job, &*wake);
                    wake();
                    on_done(touched);
                }
            })
            .ok();
        Transfers { jobs: Default::default(), tx, next_id: Mutex::new(1) }
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
        let job = Arc::new(Job {
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

// ---------------------------------------------------------------- ejecución

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
    for (src, name) in sources {
        let name = name.clone();
        let Some(m) = meta(src) else {
            errors.push(tf!("copia.no_existe", origen = src));
            continue;
        };
        if !m.is_dir() {
            out.push(Item { src: src.clone(), rel: name, is_dir: false, size: m.len() });
            continue;
        }
        out.push(Item { src: src.clone(), rel: name.clone(), is_dir: true, size: 0 });
        let mut stack = vec![(src.clone(), name)];
        while let Some((dir, rel)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            match super::list_dir(&dir) {
                Ok(list) => {
                    for e in list {
                        let s = join(&dir, &e.name);
                        let r = format!("{rel}\\{}", e.name);
                        if e.is_dir {
                            out.push(Item { src: s.clone(), rel: r.clone(), is_dir: true, size: 0 });
                            stack.push((s, r));
                        } else {
                            out.push(Item { src: s, rel: r, is_dir: false, size: e.size });
                        }
                    }
                }
                Err(e) => errors.push(format!("{dir}: {e}")),
            }
        }
    }
    out
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
    base: u64,
    wake: &'a dyn Fn(),
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
    p.job.view.lock().done_bytes = p.base + transferred as u64;
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

fn copy_file(job: &Job, src: &str, dst: &str, size: u64, base: u64, wake: &dyn Fn()) -> Result<(), String> {
    let s: Vec<u16> = long_path(src).encode_utf16().chain([0]).collect();
    let d: Vec<u16> = long_path(dst).encode_utf16().chain([0]).collect();
    let progress = Progress { job, base, wake, last_wake: Mutex::new(Instant::now()) };
    // Sin caché del sistema para archivos grandes: más rápido y no desplaza lo que hay en memoria.
    let flags = if size > 256 << 20 { COPY_FILE_NO_BUFFERING } else { COPYFILE_FLAGS(0) };
    unsafe { CopyFileExW(PCWSTR(s.as_ptr()), PCWSTR(d.as_ptr()), Some(progress_cb), Some(&progress as *const _ as *const _), None, flags) }.map_err(|e| e.message())
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

fn run_job(job: &Job, wake: &dyn Fn()) -> Vec<String> {
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

    let items = scan(&pending, &job.cancel, &mut errors);
    let total_bytes: u64 = items.iter().map(|i| i.size).sum();
    let total_files = items.iter().filter(|i| !i.is_dir).count();
    {
        let mut v = job.view.lock();
        v.state = State::Running;
        v.total_bytes = total_bytes;
        v.total_files = total_files;
    }
    wake();

    let started = Instant::now();
    let mut done_bytes = 0u64;
    let mut policy: Option<Choice> = job.policy;
    let mut copied_ok: Vec<&Item> = Vec::new();
    // Carpetas de destino ya renombradas por "conservar ambos": origen → destino.
    let mut renamed_dirs: Vec<(String, String)> = Vec::new();
    // Fuentes de primer nivel y carpetas nuevas que crearon (lo de adentro se deshace con ellas).
    let tops: std::collections::HashSet<&str> = pending.iter().map(|(s, _)| s.as_str()).collect();
    let mut created_tops: Vec<String> = Vec::new();
    for it in &items {
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
        {
            let mut v = job.view.lock();
            v.current = it.rel.clone();
        }
        if it.is_dir {
            let same = short_path(&it.src).eq_ignore_ascii_case(&short_path(&dst));
            if same && op == Op::Copy {
                // Copiar una carpeta en su misma ubicación: "Carpeta (2)".
                let new_dst = free_name(&dst);
                renamed_dirs.push((dst.clone(), new_dst.clone()));
                dst = new_dst;
            }
            let existed = meta(&dst).is_some();
            match std::fs::create_dir_all(long_path(&dst)) {
                Err(e) => errors.push(format!("{dst}: {e}")),
                Ok(()) if !existed && tops.contains(it.src.as_str()) => {
                    created.push((it.src.clone(), dst.clone()));
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
                    done_bytes += it.size;
                    let mut v = job.view.lock();
                    v.done_bytes = done_bytes;
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
        match copy_file(job, &it.src, &target, it.size, done_bytes, wake) {
            Ok(()) => {
                copied_ok.push(it);
                let inside_new = created_tops.iter().any(|d| target.to_lowercase().starts_with(d.as_str()));
                if !replaced && !inside_new {
                    created.push((it.src.clone(), target.clone()));
                }
            }
            Err(e) if job.cancel.load(Ordering::Relaxed) => {
                // Archivo a medias: se borra.
                let _ = std::fs::remove_file(long_path(&target));
                let _ = e;
                break;
            }
            Err(e) => errors.push(format!("{}: {e}", it.src)),
        }
        done_bytes += it.size;
        let secs = started.elapsed().as_secs_f64().max(0.001);
        let mut v = job.view.lock();
        v.done_bytes = done_bytes;
        v.done_files += 1;
        v.bytes_per_sec = done_bytes as f64 / secs;
    }

    // Mover entre discos: borrar los originales copiados bien y las carpetas que quedaron vacías.
    if op == Op::Move && !job.cancel.load(Ordering::Relaxed) {
        for it in &copied_ok {
            if let Err(e) = std::fs::remove_file(long_path(&it.src)) {
                errors.push(tf!("copia.no_pudo_borrar_original", origen = it.src, error = e));
            }
        }
        let mut dirs: Vec<&Item> = items.iter().filter(|i| i.is_dir).collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.rel.len()));
        for d in dirs {
            let _ = std::fs::remove_dir(long_path(&d.src));
        }
    }

    let mut v = job.view.lock();
    v.state = if job.cancel.load(Ordering::Relaxed) { State::Canceled } else { State::Done };
    v.current.clear();
    v.errors = errors;
    v.created = created;
    v.done_bytes = v.done_bytes.max(if v.state == State::Done { total_bytes } else { 0 });
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
        for _ in 0..500 {
            let v = t.snapshot().into_iter().find(|j| j.id == id).unwrap();
            if v.state == State::Conflict {
                t.resolve(id, Choice::KeepBoth, true);
            }
            if v.finished() {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("el trabajo no terminó");
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
