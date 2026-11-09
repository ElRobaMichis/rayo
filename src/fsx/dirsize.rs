//! Tamaño de carpetas que el índice no cubre (red, NAS, USB, discos sin indexar): se recorre la
//! carpeta en un hilo aparte y se va mostrando lo sumado hasta el momento. El resultado se recuerda
//! un minuto.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
    /// Ya terminó de recorrerla (si no, lo sumado hasta ahora).
    pub done: bool,
}

struct Calc {
    bytes: AtomicU64,
    files: AtomicU64,
    done: AtomicBool,
    started: Instant,
}

static CALCS: Mutex<Option<HashMap<String, Arc<Calc>>>> = Mutex::new(None);

type Wake = Arc<dyn Fn() + Send + Sync>;
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

/// Con qué repintar mientras se calcula.
pub fn set_wake(w: Wake) {
    *WAKE.lock() = Some(w);
}

/// El tamaño de una carpeta (lo que se lleva sumado, si todavía se está calculando). La primera
/// vez empieza a calcularlo.
pub fn get(path: &str) -> DirSize {
    let key = path.to_lowercase();
    let mut g = CALCS.lock();
    let map = g.get_or_insert_with(HashMap::new);
    let stale = map.get(&key).is_some_and(|c| c.done.load(Ordering::Relaxed) && c.started.elapsed() > Duration::from_secs(60));
    if stale {
        map.remove(&key);
    }
    if map.len() > 500 {
        map.retain(|_, c| !c.done.load(Ordering::Relaxed));
    }
    let calc = map
        .entry(key)
        .or_insert_with(|| {
            let c = Arc::new(Calc { bytes: AtomicU64::new(0), files: AtomicU64::new(0), done: AtomicBool::new(false), started: Instant::now() });
            let (c2, root) = (c.clone(), path.to_string());
            let _ = std::thread::Builder::new().name("tamano-carpeta".into()).spawn(move || walk(&root, &c2));
            c
        })
        .clone();
    DirSize { bytes: calc.bytes.load(Ordering::Relaxed), files: calc.files.load(Ordering::Relaxed), done: calc.done.load(Ordering::Relaxed) }
}

fn walk(root: &str, c: &Calc) {
    let wake = || {
        if let Some(w) = WAKE.lock().as_ref() {
            w();
        }
    };
    let mut stack = vec![root.to_string()];
    let mut last = Instant::now();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(super::long_path(&d)) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(super::join(&d, &e.file_name().to_string_lossy()));
            } else if let Ok(m) = e.metadata() {
                c.bytes.fetch_add(m.len(), Ordering::Relaxed);
                c.files.fetch_add(1, Ordering::Relaxed);
            }
        }
        if last.elapsed() > Duration::from_millis(150) {
            last = Instant::now();
            wake();
        }
    }
    c.done.store(true, Ordering::Relaxed);
    wake();
}

#[cfg(test)]
mod tests {
    #[test]
    fn sums_a_tree() {
        let d = std::env::temp_dir().join(format!("rayo-dirsize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a").join("b")).unwrap();
        std::fs::write(d.join("x.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(d.join("a").join("b").join("y.bin"), vec![0u8; 24]).unwrap();
        let p = d.to_string_lossy().into_owned();
        let mut s = super::get(&p);
        for _ in 0..200 {
            if s.done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            s = super::get(&p);
        }
        assert_eq!(s, super::DirSize { bytes: 1024, files: 2, done: true });
        let _ = std::fs::remove_dir_all(&d);
    }
}
