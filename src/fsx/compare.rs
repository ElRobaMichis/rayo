//! Comparar dos carpetas (recursivo) para sincronizarlas.
//!
//! Cada elemento queda como: solo a la izquierda, solo a la derecha, más nuevo
//! de un lado, distinto (misma fecha, otro tamaño) o igual. Las carpetas que
//! están de un solo lado no se recorren por dentro: se copian enteras.

use super::{join, list_dir};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    OnlyLeft,
    OnlyRight,
    LeftNewer,
    RightNewer,
    Different,
    Same,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Copiar de izquierda a derecha.
    ToRight,
    /// Copiar de derecha a izquierda.
    ToLeft,
    /// No hacer nada.
    Skip,
    /// Mandar a la papelera el de la izquierda.
    DeleteLeft,
    /// Mandar a la papelera el de la derecha.
    DeleteRight,
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Ruta relativa (con `\`).
    pub rel: String,
    pub is_dir: bool,
    pub left: Option<(u64, u32)>,
    pub right: Option<(u64, u32)>,
    pub status: Status,
    pub action: Dir,
}

impl Item {
    /// Acciones que tienen sentido para este elemento (para alternarlas con un clic).
    pub fn choices(&self) -> Vec<Dir> {
        let mut v = Vec::new();
        if self.left.is_some() {
            v.push(Dir::ToRight);
        }
        if self.right.is_some() {
            v.push(Dir::ToLeft);
        }
        match self.status {
            Status::OnlyLeft => v.push(Dir::DeleteLeft),
            Status::OnlyRight => v.push(Dir::DeleteRight),
            _ => {}
        }
        v.push(Dir::Skip);
        v
    }


    /// Qué conviene hacer por defecto al sincronizar en ambas direcciones.
    pub fn default_action(status: Status) -> Dir {
        match status {
            Status::OnlyLeft | Status::LeftNewer => Dir::ToRight,
            Status::OnlyRight | Status::RightNewer => Dir::ToLeft,
            Status::Different | Status::Same => Dir::Skip,
        }
    }
}

/// Diferencias de hasta 2 s se consideran la misma fecha (FAT guarda de a 2 s).
const TOLERANCE: u32 = 2;

pub fn compare(left: &str, right: &str, cancel: &AtomicBool, scanned: &AtomicUsize) -> Result<Vec<Item>, String> {
    list_dir(left).map_err(|e| format!("{left}: {e}"))?;
    list_dir(right).map_err(|e| format!("{right}: {e}"))?;
    let mut out = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let side = |base: &str| {
            let dir = if rel.is_empty() { base.to_string() } else { join(base, &rel) };
            list_dir(&dir).unwrap_or_default()
        };
        // Unir los dos lados por nombre (sin distinguir mayúsculas).
        let mut merged: BTreeMap<String, (Option<super::FileEntry>, Option<super::FileEntry>)> = BTreeMap::new();
        for e in side(left) {
            let k = e.lower.clone();
            merged.entry(k).or_default().0 = Some(e);
        }
        for e in side(right) {
            let k = e.lower.clone();
            merged.entry(k).or_default().1 = Some(e);
        }
        scanned.fetch_add(merged.len(), Ordering::Relaxed);
        for (_, (l, r)) in merged {
            let name = l.as_ref().or(r.as_ref()).map(|e| e.name.clone()).unwrap_or_default();
            let child = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
            let is_dir = l.as_ref().or(r.as_ref()).is_some_and(|e| e.is_dir);
            let status = match (&l, &r) {
                (Some(_), None) => Status::OnlyLeft,
                (None, Some(_)) => Status::OnlyRight,
                (Some(a), Some(b)) if a.is_dir && b.is_dir => {
                    stack.push(child);
                    continue;
                }
                (Some(a), Some(b)) => {
                    let (ta, tb) = (a.mtime, b.mtime);
                    if ta.abs_diff(tb) <= TOLERANCE {
                        if a.size == b.size { Status::Same } else { Status::Different }
                    } else if ta > tb {
                        Status::LeftNewer
                    } else {
                        Status::RightNewer
                    }
                }
                (None, None) => continue,
            };
            out.push(Item {
                rel: child,
                is_dir,
                left: l.map(|e| (e.size, e.mtime)),
                right: r.map(|e| (e.size, e.mtime)),
                status,
                action: Item::default_action(status),
            });
        }
    }
    out.sort_by(|a, b| a.rel.to_lowercase().cmp(&b.rel.to_lowercase()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn set_mtime(p: &std::path::Path, secs_ago: u64) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    #[test]
    fn classifies_differences() {
        let d = std::env::temp_dir().join(format!("rayo-compare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (l, r) = (d.join("izq"), d.join("der"));
        std::fs::create_dir_all(l.join("comun")).unwrap();
        std::fs::create_dir_all(r.join("comun")).unwrap();
        std::fs::create_dir_all(l.join("solo-carpeta")).unwrap();
        std::fs::write(l.join("solo-carpeta").join("x.txt"), "x").unwrap();
        std::fs::write(l.join("igual.txt"), "abc").unwrap();
        std::fs::write(r.join("igual.txt"), "abc").unwrap();
        std::fs::write(l.join("comun").join("nuevo.txt"), "v2").unwrap();
        std::fs::write(r.join("comun").join("nuevo.txt"), "v1").unwrap();
        std::fs::write(r.join("solo-der.txt"), "d").unwrap();
        let now = 0;
        set_mtime(&l.join("igual.txt"), 100 + now);
        set_mtime(&r.join("igual.txt"), 100);
        set_mtime(&r.join("comun").join("nuevo.txt"), 5000);
        let items = compare(&l.to_string_lossy(), &r.to_string_lossy(), &AtomicBool::new(false), &AtomicUsize::new(0)).unwrap();
        let find = |rel: &str| items.iter().find(|i| i.rel == rel).unwrap_or_else(|| panic!("{rel}: {items:?}"));
        assert_eq!(find("igual.txt").status, Status::Same);
        assert_eq!(find(r"comun\nuevo.txt").status, Status::LeftNewer);
        assert_eq!(find(r"comun\nuevo.txt").action, Dir::ToRight);
        assert_eq!(find("solo-der.txt").status, Status::OnlyRight);
        assert_eq!(find("solo-carpeta").status, Status::OnlyLeft);
        assert!(find("solo-carpeta").is_dir);
        // Lo que está dentro de una carpeta de un solo lado no se lista aparte.
        assert!(items.iter().all(|i| !i.rel.starts_with(r"solo-carpeta\")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
