//! Buscador de duplicados.
//!
//! 1. Candidatos: archivos con exactamente el mismo tamaño (sale del índice al instante).
//! 2. Se descartan los enlaces duros (mismo archivo con dos nombres: borrar uno no libera nada).
//! 3. Se comparan los primeros 16 KB; solo los que siguen iguales se leen completos.
//! 4. Hash xxh3 de 128 bits: rapidísimo y sin colisiones en la práctica.

use super::long_path;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub size: u64,
    pub mtime: u32,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub size: u64,
    pub files: Vec<Candidate>,
}

impl Group {
    /// Espacio que se recupera dejando una sola copia.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Archivos que están solo en la nube: no se leen (se descargarían).
    pub skipped_cloud: AtomicU64,
}

const HEAD: usize = 16 * 1024;

/// Identidad del archivo en el volumen (para reconocer enlaces duros).
fn file_id(f: &std::fs::File) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info) }.ok()?;
    Some((info.dwVolumeSerialNumber, ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64))
}

fn hash_head(path: &str, prog: &Progress) -> Option<((u32, u64), u128)> {
    let mut f = std::fs::File::open(long_path(path)).ok()?;
    let id = file_id(&f)?;
    let mut buf = vec![0u8; HEAD];
    let mut n = 0;
    while n < HEAD {
        let k = f.read(&mut buf[n..]).ok()?;
        if k == 0 {
            break;
        }
        n += k;
    }
    prog.done.fetch_add(n as u64, Ordering::Relaxed);
    Some((id, xxhash_rust::xxh3::xxh3_128(&buf[..n])))
}

fn hash_full(path: &str, cancel: &AtomicBool, prog: &Progress) -> Option<u128> {
    let mut f = std::fs::File::open(long_path(path)).ok()?;
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let k = f.read(&mut buf).ok()?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        prog.done.fetch_add(k as u64, Ordering::Relaxed);
    }
    Some(h.digest128())
}

/// Busca duplicados entre los candidatos. Devuelve los grupos ordenados por
/// espacio recuperable (de mayor a menor).
pub fn find(mut cands: Vec<Candidate>, cancel: &AtomicBool, prog: &Progress) -> Vec<Group> {
    // 1. Por tamaño.
    cands.sort_unstable_by_key(|c| c.size);
    let mut by_size: Vec<Vec<Candidate>> = Vec::new();
    for c in cands {
        match by_size.last_mut() {
            Some(g) if g[0].size == c.size => g.push(c),
            _ => by_size.push(vec![c]),
        }
    }
    by_size.retain(|g| g.len() > 1 && g[0].size > 0);
    // Leer un archivo de OneDrive que está solo en la nube lo descargaría: se dejan fuera.
    for g in &mut by_size {
        let before = g.len();
        g.retain(|c| !super::is_online_only(&c.path));
        prog.skipped_cloud.fetch_add((before - g.len()) as u64, Ordering::Relaxed);
    }
    by_size.retain(|g| g.len() > 1);
    let total: u64 = by_size.iter().map(|g| g.len() as u64 * g[0].size.min(HEAD as u64)).sum::<u64>()
        + by_size.iter().filter(|g| g[0].size > HEAD as u64).map(|g| g.len() as u64 * g[0].size).sum::<u64>();
    prog.total.store(total, Ordering::Relaxed);

    let mut groups: Vec<Group> = by_size
        .into_par_iter()
        .flat_map_iter(|g| {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let size = g[0].size;
            // 2 y 3. Identidad + inicio del archivo.
            let mut seen_ids = std::collections::HashSet::new();
            let mut by_head: HashMap<u128, Vec<Candidate>> = HashMap::new();
            for c in g {
                if let Some((id, h)) = hash_head(&c.path, prog) {
                    if seen_ids.insert(id) {
                        by_head.entry(h).or_default().push(c);
                    }
                }
            }
            let mut out = Vec::new();
            for (_, same) in by_head {
                if same.len() < 2 {
                    continue;
                }
                if size <= HEAD as u64 {
                    out.push(Group { size, files: same });
                    continue;
                }
                // 4. Contenido completo.
                let mut by_full: HashMap<u128, Vec<Candidate>> = HashMap::new();
                for c in same {
                    if let Some(h) = hash_full(&c.path, cancel, prog) {
                        by_full.entry(h).or_default().push(c);
                    }
                }
                out.extend(by_full.into_values().filter(|v| v.len() > 1).map(|files| Group { size, files }));
            }
            out
        })
        .collect();
    groups.sort_unstable_by(|a, b| b.wasted().cmp(&a.wasted()));
    for g in &mut groups {
        g.files.sort_by(|a, b| a.mtime.cmp(&b.mtime).then_with(|| a.path.cmp(&b.path)));
    }
    groups
}

/// Candidatos recorriendo carpetas (para discos sin índice, como USB o red).
pub fn walk_candidates(dir: &str, min_size: u64, cancel: &AtomicBool) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_string()];
    while let Some(d) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Ok(list) = super::list_dir(&d) {
            for e in list {
                let p = super::join(&d, &e.name);
                if e.is_dir {
                    stack.push(p);
                } else if e.size >= min_size {
                    out.push(Candidate { path: p, size: e.size, mtime: e.mtime });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_real_duplicates_only() {
        let d = std::env::temp_dir().join(format!("rayo-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let big: Vec<u8> = (0..100_000u32).flat_map(|x| x.to_le_bytes()).collect();
        let mut big2 = big.clone();
        *big2.last_mut().unwrap() ^= 1; // Mismo tamaño e inicio, distinto final.
        std::fs::write(d.join("a.bin"), &big).unwrap();
        std::fs::write(d.join("sub").join("a copia.bin"), &big).unwrap();
        std::fs::write(d.join("casi.bin"), &big2).unwrap();
        std::fs::write(d.join("x.txt"), b"igual").unwrap();
        std::fs::write(d.join("y.txt"), b"igual").unwrap();
        // Enlace duro: el mismo archivo, no un duplicado.
        std::fs::hard_link(d.join("x.txt"), d.join("x-enlace.txt")).unwrap();
        let dir = d.to_string_lossy().into_owned();
        let cancel = AtomicBool::new(false);
        let prog = Progress::default();
        let groups = find(walk_candidates(&dir, 1, &cancel), &cancel, &prog);
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].files.len(), 2);
        assert!(groups[0].files.iter().all(|f| f.path.ends_with("a.bin") || f.path.ends_with("a copia.bin")));
        assert_eq!(groups[1].files.len(), 2);
        assert_eq!(groups[0].wasted(), 400_000);
        let _ = std::fs::remove_dir_all(&d);
    }
}
