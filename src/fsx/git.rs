//! Estado de git sin configurar nada: si una carpeta está dentro de un repositorio (y git está
//! instalado), cada archivo muestra si cambió, es nuevo, se borró… y la barra de estado, la rama.
//!
//! Se le pregunta a `git status` en un hilo aparte (sin ventana), a lo sumo cada pocos segundos
//! mientras el repositorio está a la vista.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Estado de un archivo en el repositorio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
}

impl State {
    pub fn letter(self) -> &'static str {
        match self {
            State::Modified => "M",
            State::Added => "A",
            State::Deleted => "D",
            State::Renamed => "R",
            State::Untracked => "?",
            State::Conflict => "!",
        }
    }
}

#[derive(Default, Debug)]
pub struct Repo {
    pub branch: String,
    /// Ruta relativa en minúsculas (con `\`) → estado.
    pub files: HashMap<String, State>,
    /// Carpetas (relativas, minúsculas) con algo cambiado adentro.
    pub dirty_dirs: std::collections::HashSet<String>,
}

struct Entry {
    repo: Option<Arc<Repo>>,
    at: Instant,
    running: bool,
}

fn git_exe() -> Option<&'static std::path::PathBuf> {
    static G: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    G.get_or_init(|| std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("git.exe")).find(|f| f.is_file()))).as_ref()
}

/// La raíz del repositorio que contiene esta carpeta (se recuerda por carpeta).
pub fn root_of(dir: &str) -> Option<String> {
    static ROOTS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
    git_exe()?;
    if dir.is_empty() || dir.starts_with(super::FLAT_KEY) || super::archive::split(dir).is_some() {
        return None;
    }
    let key = dir.to_lowercase();
    if let Some(r) = ROOTS.lock().get_or_insert_with(HashMap::new).get(&key) {
        return r.clone();
    }
    // En la red no se busca (cada paso hacia arriba puede tardar).
    let found = if super::is_remote(dir) {
        None
    } else {
        let mut cur = Some(dir.to_string());
        let mut out = None;
        while let Some(c) = cur {
            if std::fs::symlink_metadata(super::long_path(&super::join(&c, ".git"))).is_ok() {
                out = Some(c);
                break;
            }
            cur = super::parent_of(&c);
        }
        out
    };
    let mut g = ROOTS.lock();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 5000 {
        m.clear();
    }
    m.insert(key, found.clone());
    found
}

static REPOS: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);
static WAKE: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

pub fn set_wake(w: Arc<dyn Fn() + Send + Sync>) {
    *WAKE.lock() = Some(w);
}

/// El estado del repositorio (el último leído); si es viejo, se vuelve a leer en segundo plano.
pub fn repo(root: &str) -> Option<Arc<Repo>> {
    let key = root.to_lowercase();
    let mut g = REPOS.lock();
    let m = g.get_or_insert_with(HashMap::new);
    let e = m.entry(key).or_insert_with(|| Entry { repo: None, at: Instant::now() - Duration::from_secs(60), running: false });
    if !e.running && e.at.elapsed() > Duration::from_secs(3) {
        e.running = true;
        let r = root.to_string();
        std::thread::Builder::new()
            .name("git".into())
            .spawn(move || {
                let repo = read(&r).map(Arc::new);
                if let Some(e) = REPOS.lock().get_or_insert_with(HashMap::new).get_mut(&r.to_lowercase()) {
                    e.repo = repo;
                    e.at = Instant::now();
                    e.running = false;
                }
                if let Some(w) = WAKE.lock().as_ref() {
                    w();
                }
            })
            .ok();
    }
    e.repo.clone()
}

/// Estado de un archivo o carpeta (para la marca en la lista).
pub fn state_of(path: &str, is_dir: bool) -> Option<(State, bool)> {
    let dir = super::parent_of(path)?;
    let root = root_of(&dir)?;
    let repo = repo(&root)?;
    let rel = path.get(root.trim_end_matches('\\').len() + 1..)?.to_lowercase();
    if let Some(s) = repo.files.get(&rel) {
        return Some((*s, false));
    }
    (is_dir && repo.dirty_dirs.contains(&rel)).then_some((State::Modified, true))
}

fn read(root: &str) -> Option<Repo> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(git_exe()?)
        .args(["-C", root, "--no-optional-locks", "status", "--porcelain=v1", "-b", "-z", "-unormal"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse(&out.stdout))
}

/// Lee la salida de `git status --porcelain=v1 -b -z`.
fn parse(raw: &[u8]) -> Repo {
    let mut repo = Repo::default();
    let text = String::from_utf8_lossy(raw);
    let mut parts = text.split('\0').filter(|s| !s.is_empty());
    while let Some(item) = parts.next() {
        if let Some(b) = item.strip_prefix("## ") {
            // "main...origin/main [ahead 1]" o "No commits yet on main".
            let b = b.strip_prefix("No commits yet on ").unwrap_or(b);
            repo.branch = b.split("...").next().unwrap_or(b).split(' ').next().unwrap_or("").to_string();
            continue;
        }
        if item.len() < 4 {
            continue;
        }
        let (xy, path) = item.split_at(3);
        let xy = xy.as_bytes();
        let (x, y) = (xy[0], xy[1]);
        let state = match (x, y) {
            (b'?', b'?') => State::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => State::Conflict,
            (b'R', _) | (_, b'R') => State::Renamed,
            (b'A', _) => State::Added,
            (b'D', _) | (_, b'D') => State::Deleted,
            _ => State::Modified,
        };
        // Renombrado: después viene la ruta de antes (se salta).
        if x == b'R' || x == b'C' {
            parts.next();
        }
        let rel = path.trim_end_matches('/').replace('/', "\\").to_lowercase();
        // Las carpetas de arriba: "algo cambió adentro".
        let mut cut = rel.as_str();
        while let Some(i) = cut.rfind('\\') {
            cut = &cut[..i];
            repo.dirty_dirs.insert(cut.to_string());
        }
        repo.files.insert(rel, state);
    }
    repo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let raw = b"## main...origin/main [ahead 1]\0 M src/ui/app.rs\0?? notas/\0A  nuevo.txt\0R  b.txt\0a.txt\0D  viejo.rs\0UU choque.rs\0";
        let r = parse(raw);
        assert_eq!(r.branch, "main");
        assert_eq!(r.files.get(r"src\ui\app.rs"), Some(&State::Modified));
        assert_eq!(r.files.get("notas"), Some(&State::Untracked));
        assert_eq!(r.files.get("nuevo.txt"), Some(&State::Added));
        assert_eq!(r.files.get("b.txt"), Some(&State::Renamed));
        assert!(!r.files.contains_key("a.txt"));
        assert_eq!(r.files.get("viejo.rs"), Some(&State::Deleted));
        assert_eq!(r.files.get("choque.rs"), Some(&State::Conflict));
        assert!(r.dirty_dirs.contains("src") && r.dirty_dirs.contains(r"src\ui"));
    }
}
