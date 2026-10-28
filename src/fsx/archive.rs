//! Archivos comprimidos como carpetas.
//!
//! Una ruta como `C:\Descargas\fotos.zip\viaje\a.jpg` apunta a algo dentro de un
//! comprimido. Los ZIP se leen con un lector propio (sin programas externos); 7z,
//! RAR, TAR, ISO y compañía, con el 7-Zip instalado. Todo es de solo lectura:
//! para cambiar algo, se extrae.

use super::{FileEntry, file_name, join, long_path};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// Se leen con el lector propio.
pub const ZIP_EXTS: &[&str] = &["zip", "jar", "cbz", "epub", "apk", "xpi", "vsix", "nupkg"];
/// Se leen con 7-Zip (si está instalado).
pub const SEVEN_EXTS: &[&str] = &["7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "cab", "iso", "wim", "lzh", "arj", "cbr", "cb7"];

/// Ruta de `7z.exe`, si 7-Zip está instalado.
pub fn seven_zip() -> Option<&'static str> {
    static P: OnceLock<Option<String>> = OnceLock::new();
    P.get_or_init(|| {
        ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|d| format!("{d}\\7-Zip\\7z.exe"))
            .find(|p| std::path::Path::new(p).is_file())
    })
    .as_deref()
}

fn ext_lower(name: &str) -> String {
    super::ext_of(name)
}

fn is_zip_name(name: &str) -> bool {
    ZIP_EXTS.contains(&ext_lower(name).as_str())
}

/// ¿Rayo puede abrir este archivo como carpeta? (solo mira el nombre)
pub fn is_archive_name(name: &str) -> bool {
    let e = ext_lower(name);
    ZIP_EXTS.contains(&e.as_str()) || (SEVEN_EXTS.contains(&e.as_str()) && seven_zip().is_some())
}

/// Divide `C:\a\b.zip\x\y` en (`C:\a\b.zip`, `x\y`), si `b.zip` es un archivo comprimido.
/// Para el comprimido mismo devuelve la parte interna vacía.
pub fn split(path: &str) -> Option<(String, String)> {
    let p = path.trim_start_matches(r"\\?\").trim_end_matches('\\');
    // Rápido: si ningún tramo tiene extensión de comprimido, no hay nada que mirar.
    let mut cut = 0;
    for (i, part) in p.split('\\').enumerate() {
        let end = cut + part.len();
        if i > 0 && is_archive_name(part) {
            let prefix = &p[..end];
            if std::fs::metadata(long_path(prefix)).is_ok_and(|m| m.is_file()) {
                let inner = p[end..].trim_start_matches('\\').to_string();
                return Some((prefix.to_string(), inner));
            }
        }
        cut = end + 1;
    }
    None
}

/// ¿Está dentro de un comprimido (no es el comprimido mismo)?
pub fn is_inside(path: &str) -> bool {
    split(path).is_some_and(|(_, inner)| !inner.is_empty())
}

// ---------------------------------------------------------------- listado

#[derive(Clone, Debug)]
pub struct Entry {
    /// Ruta interna con `\`.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u32,
}

pub struct Listing {
    pub entries: Vec<Entry>,
    /// Tamaño total de cada carpeta (ruta interna en minúsculas).
    dir_sizes: HashMap<String, u64>,
}

type Cached = (String, (u64, u64), Arc<Listing>);
static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());

fn signature(archive: &str) -> Result<(u64, u64), String> {
    let m = std::fs::metadata(long_path(archive)).map_err(|e| e.to_string())?;
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Ok((m.len(), t))
}

/// Contenido del comprimido (se recuerda mientras no cambie en disco).
pub fn listing(archive: &str) -> Result<Arc<Listing>, String> {
    let key = archive.to_lowercase();
    let sig = signature(archive)?;
    if let Some(c) = CACHE.lock().iter().find(|c| c.0 == key && c.1 == sig) {
        return Ok(c.2.clone());
    }
    let mut entries = if is_zip_name(archive) { read_zip(archive)? } else { read_7z(archive)? };
    // Carpetas implícitas (muchos ZIP no las guardan).
    let mut dirs: HashSet<String> = entries.iter().filter(|e| e.is_dir).map(|e| e.path.to_lowercase()).collect();
    let mut extra = Vec::new();
    for e in &entries {
        let mut p = e.path.as_str();
        while let Some(i) = p.rfind('\\') {
            p = &p[..i];
            if dirs.insert(p.to_lowercase()) {
                extra.push(Entry { path: p.to_string(), is_dir: true, size: 0, mtime: e.mtime });
            }
        }
    }
    entries.extend(extra);
    let mut dir_sizes: HashMap<String, u64> = HashMap::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        let mut p = e.path.as_str();
        while let Some(i) = p.rfind('\\') {
            p = &p[..i];
            *dir_sizes.entry(p.to_lowercase()).or_default() += e.size;
        }
    }
    let l = Arc::new(Listing { entries, dir_sizes });
    let mut c = CACHE.lock();
    c.retain(|x| x.0 != key);
    c.push((key, sig, l.clone()));
    if c.len() > 6 {
        c.remove(0);
    }
    Ok(l)
}

/// Nombre interno seguro: sin `..`, sin unidad, con `\`.
fn clean(name: &str) -> Option<String> {
    let n = name.replace('/', "\\");
    let n = n.trim_matches('\\');
    if n.is_empty() || n.contains(':') || n.split('\\').any(|c| c == ".." || c == ".") {
        return None;
    }
    Some(n.to_string())
}

fn read_zip(archive: &str) -> Result<Vec<Entry>, String> {
    let f = std::fs::File::open(long_path(archive)).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| tf!("comprimido.no_zip_valido", error = e))?;
    let mut out = Vec::with_capacity(z.len());
    for i in 0..z.len() {
        let Ok(e) = z.by_index_raw(i) else { continue };
        let Some(path) = clean(e.name()) else { continue };
        let mtime = e.last_modified().map_or(0, |d| local_to_unix(d.year(), d.month(), d.day(), d.hour(), d.minute(), d.second()));
        out.push(Entry { path, is_dir: e.is_dir(), size: e.size(), mtime });
    }
    Ok(out)
}

fn command(exe: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(exe);
    // Sin ventana de consola y sin esperar nada por teclado (contraseñas).
    c.creation_flags(0x0800_0000).stdin(std::process::Stdio::null());
    c
}

fn read_7z(archive: &str) -> Result<Vec<Entry>, String> {
    let exe = seven_zip().ok_or(t!("comprimido.abrir_este_formato_hace_falta"))?;
    let out = command(exe).args(["l", "-slt", "-ba", "-sccUTF-8", "-p", "--", archive]).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() && text.trim().is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.lines().find(|l| !l.trim().is_empty()).unwrap_or(t!("comprimido.no_pudo_leer")).trim().to_string();
        return Err(if msg.to_lowercase().contains("password") { t!("comprimido.comprimido_tiene_contrasena").into() } else { msg });
    }
    let mut res = Vec::new();
    for block in text.split("\n\n").chain(text.split("\r\n\r\n")).filter(|b| b.contains("Path = ")) {
        let mut path = None;
        let (mut size, mut mtime, mut is_dir) = (0u64, 0u32, false);
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(v) = line.strip_prefix("Path = ") {
                path = clean(v);
            } else if let Some(v) = line.strip_prefix("Size = ") {
                size = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("Modified = ") {
                mtime = parse_7z_date(v);
            } else if let Some(v) = line.strip_prefix("Attributes = ") {
                is_dir |= v.starts_with('D');
            } else if line == "Folder = +" {
                is_dir = true;
            }
        }
        if let Some(path) = path {
            res.push(Entry { path, is_dir, size, mtime });
        }
    }
    // Con saltos de línea \r\n el texto se partió dos veces: quitar repetidos.
    let mut seen = HashSet::new();
    res.retain(|e| seen.insert(e.path.to_lowercase()));
    Ok(res)
}

/// "2026-10-27 11:49:00.9658734" (hora local) → segundos Unix.
fn parse_7z_date(v: &str) -> u32 {
    let n: Vec<u32> = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).take(6).filter_map(|s| s.parse().ok()).collect();
    if n.len() < 6 {
        return 0;
    }
    local_to_unix(n[0] as u16, n[1] as u8, n[2] as u8, n[3] as u8, n[4] as u8, n[5] as u8)
}

/// Lo que hay en una carpeta de un comprimido.
pub fn list(archive: &str, inner: &str) -> Result<Vec<FileEntry>, String> {
    let l = listing(archive)?;
    let inner_l = inner.to_lowercase();
    let mut out = Vec::new();
    for e in &l.entries {
        let parent = e.path.rfind('\\').map_or("", |i| &e.path[..i]);
        if !parent.eq_ignore_ascii_case(inner) && parent.to_lowercase() != inner_l {
            continue;
        }
        let name = file_name(&e.path);
        out.push(FileEntry {
            lower: name.to_lowercase(),
            ext: if e.is_dir { String::new() } else { super::ext_of(&name) },
            is_dir: e.is_dir,
            hidden: false,
            size: e.size,
            dir_size: if e.is_dir { Some(l.dir_sizes.get(&e.path.to_lowercase()).copied().unwrap_or(0)) } else { None },
            mtime: e.mtime,
            ctime: 0,
            cloud: super::Cloud::None,
            name,
        });
    }
    if out.is_empty() && !inner.is_empty() && !l.entries.iter().any(|e| e.is_dir && e.path.to_lowercase() == inner_l) {
        return Err(t!("comprimido.no_existe_dentro_comprimido").into());
    }
    Ok(out)
}

/// Datos de un elemento de adentro.
pub fn find(archive: &str, inner: &str) -> Option<Entry> {
    listing(archive).ok()?.entries.iter().find(|e| e.path.eq_ignore_ascii_case(inner)).cloned()
}

/// ¿Es una carpeta (o la raíz) dentro de un comprimido?
pub fn is_dir(path: &str) -> bool {
    match split(path) {
        Some((_, inner)) if inner.is_empty() => true,
        Some((a, inner)) => listing(&a).is_ok_and(|l| l.entries.iter().any(|e| e.is_dir && e.path.eq_ignore_ascii_case(&inner))),
        None => false,
    }
}

/// Tamaño de lo que se extraería (un archivo, una carpeta o todo).
pub fn size_of(archive: &str, inner: &str) -> u64 {
    let Ok(l) = listing(archive) else { return 0 };
    if inner.is_empty() {
        return l.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    }
    let prefix = format!("{}\\", inner.to_lowercase());
    l.entries.iter().filter(|e| !e.is_dir).filter(|e| e.path.eq_ignore_ascii_case(inner) || e.path.to_lowercase().starts_with(&prefix)).map(|e| e.size).sum()
}

// ---------------------------------------------------------------- extraer

/// Extrae cada (ruta interna, destino completo). Las carpetas salen con todo su contenido;
/// la ruta interna vacía es el comprimido entero (el destino es una carpeta).
pub fn extract(archive: &str, items: &[(String, String)], progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    if is_zip_name(archive) { extract_zip(archive, items, progress, cancel) } else { extract_7z(archive, items, progress, cancel) }
}

/// ¿`name` es `inner` o algo dentro de él?
fn under(name: &str, inner: &str) -> bool {
    inner.is_empty() || name.eq_ignore_ascii_case(inner) || (name.len() > inner.len() && name.as_bytes()[inner.len()] == b'\\' && name[..inner.len()].eq_ignore_ascii_case(inner))
}

/// Dónde va una entrada del comprimido según lo pedido (puede ir a varios destinos).
fn targets_for(name: &str, items: &[(String, String)]) -> Vec<String> {
    items
        .iter()
        .filter(|(inner, _)| under(name, inner))
        .map(|(inner, target)| if inner.is_empty() { join(target, name) } else { format!("{target}{}", &name[inner.len()..]) })
        .collect()
}

fn set_mtime(path: &str, unix: u32) {
    if unix == 0 {
        return;
    }
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(long_path(path)) {
        let _ = f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix as u64));
    }
}

fn extract_zip(archive: &str, items: &[(String, String)], progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    let f = std::fs::File::open(long_path(archive)).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    for (_, t) in items {
        if items.iter().any(|(i, _)| i.is_empty()) {
            let _ = std::fs::create_dir_all(long_path(t));
        }
    }
    let mut buf = vec![0u8; 256 * 1024];
    for i in 0..z.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelado".into());
        }
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = clean(e.name()) else { continue };
        let outs = targets_for(&name, items);
        let Some(out) = outs.first().cloned() else { continue };
        if e.is_dir() {
            for o in &outs {
                std::fs::create_dir_all(long_path(o)).map_err(|err| format!("{o}: {err}"))?;
            }
            continue;
        }
        if let Some(parent) = super::parent_of(&out) {
            let _ = std::fs::create_dir_all(long_path(&parent));
        }
        let mut w = std::fs::File::create(long_path(&out)).map_err(|err| format!("{out}: {err}"))?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                drop(w);
                let _ = std::fs::remove_file(long_path(&out));
                return Err("cancelado".into());
            }
            let n = e.read(&mut buf).map_err(|err| format!("{name}: {err}"))?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).map_err(|err| format!("{out}: {err}"))?;
            progress(n as u64);
        }
        drop(w);
        let mtime = e.last_modified().map_or(0, |d| local_to_unix(d.year(), d.month(), d.day(), d.hour(), d.minute(), d.second()));
        set_mtime(&out, mtime);
        // Si también va a otros destinos, se copia lo ya extraído.
        for o in &outs[1..] {
            if let Some(parent) = super::parent_of(o) {
                let _ = std::fs::create_dir_all(long_path(&parent));
            }
            std::fs::copy(long_path(&out), long_path(o)).map_err(|err| format!("{o}: {err}"))?;
        }
    }
    Ok(())
}

/// Copia una carpeta (o un archivo) entera.
fn copy_tree(src: &str, dst: &str) -> Result<(), String> {
    let m = std::fs::metadata(long_path(src)).map_err(|e| e.to_string())?;
    if m.is_dir() {
        std::fs::create_dir_all(long_path(dst)).map_err(|e| format!("{dst}: {e}"))?;
        for e in std::fs::read_dir(long_path(src)).map_err(|e| e.to_string())?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            copy_tree(&join(src, &name), &join(dst, &name))?;
        }
        Ok(())
    } else {
        if let Some(p) = super::parent_of(dst) {
            let _ = std::fs::create_dir_all(long_path(&p));
        }
        std::fs::copy(long_path(src), long_path(dst)).map(|_| ()).map_err(|e| format!("{dst}: {e}"))
    }
}

/// Mueve `src` a `dst`; si `dst` es una carpeta que ya existe, mezcla el contenido.
fn merge_move(src: &str, dst: &str) -> Result<(), String> {
    let s_meta = std::fs::metadata(long_path(src)).map_err(|e| e.to_string())?;
    let d_meta = std::fs::metadata(long_path(dst));
    if s_meta.is_dir() && d_meta.as_ref().is_ok_and(|m| m.is_dir()) {
        for e in std::fs::read_dir(long_path(src)).map_err(|e| e.to_string())?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            merge_move(&join(src, &name), &join(dst, &name))?;
        }
        let _ = std::fs::remove_dir(long_path(src));
        return Ok(());
    }
    if d_meta.is_ok_and(|m| m.is_file()) {
        let _ = std::fs::remove_file(long_path(dst));
    }
    if let Some(p) = super::parent_of(dst) {
        let _ = std::fs::create_dir_all(long_path(&p));
    }
    std::fs::rename(long_path(src), long_path(dst)).map_err(|e| format!("{dst}: {e}"))
}

fn extract_7z(archive: &str, items: &[(String, String)], progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    let exe = seven_zip().ok_or(t!("comprimido.extraer_este_formato_hace_falta"))?;
    // Se extrae a una carpeta temporal junto al destino (mismo disco: mover es instantáneo).
    let first = items.first().map(|i| i.1.clone()).ok_or(t!("comprimido.nada_extraer"))?;
    let base = super::parent_of(&first).unwrap_or(first);
    let tmp = join(&base, &format!(".rayo-extrayendo-{}-{}", std::process::id(), super::super::index::search::now_unix()));
    std::fs::create_dir_all(long_path(&tmp)).map_err(|e| e.to_string())?;
    let mut args: Vec<String> = vec!["x".into(), "-y".into(), "-sccUTF-8".into(), "-p".into(), format!("-o{tmp}"), "--".into(), archive.to_string()];
    if !items.iter().any(|(i, _)| i.is_empty()) {
        for (inner, _) in items {
            args.push(inner.clone());
            args.push(format!("{inner}\\*"));
        }
    }
    let mut child = command(exe).args(&args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = std::fs::remove_dir_all(long_path(&tmp));
            return Err("cancelado".into());
        }
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let mut result = Ok(());
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let msg = err.lines().find(|l| !l.trim().is_empty()).unwrap_or(t!("comprimido.7_zip_no_pudo_extraer")).trim().to_string();
        result = Err(if msg.to_lowercase().contains("password") { t!("comprimido.comprimido_tiene_contrasena").into() } else { msg });
    } else {
        // Lo más profundo primero; si otro pedido lo contiene, se copia (el otro lo moverá después).
        let mut order: Vec<&(String, String)> = items.iter().collect();
        order.sort_by_key(|(inner, _)| std::cmp::Reverse(if inner.is_empty() { 0 } else { inner.len() + 1 }));
        for (k, (inner, target)) in order.iter().enumerate() {
            let src = if inner.is_empty() { tmp.clone() } else { join(&tmp, inner) };
            let needed_later = order[k + 1..].iter().any(|(other, _)| under(inner, other));
            let r = if needed_later { copy_tree(&src, target) } else { merge_move(&src, target) };
            if let Err(e) = r {
                result = Err(e);
            }
        }
        progress(items.iter().map(|(i, _)| size_of(archive, i)).sum());
    }
    let _ = std::fs::remove_dir_all(long_path(&tmp));
    result
}

/// Copia temporal de un archivo de adentro (para abrirlo, verlo o arrastrarlo a otra app).
pub fn materialize(path: &str) -> Option<String> {
    let (archive, inner) = split(path)?;
    if inner.is_empty() {
        return Some(path.to_string());
    }
    let sig = signature(&archive).ok()?;
    let key = xxhash_rust::xxh3::xxh3_64(format!("{}|{}|{}", archive.to_lowercase(), sig.0, sig.1).as_bytes());
    let dir = std::env::temp_dir().join("Rayo").join("comprimidos").join(format!("{key:016x}"));
    let target = join(&dir.to_string_lossy(), &inner);
    if std::fs::metadata(long_path(&target)).is_ok() {
        return Some(target);
    }
    extract(&archive, &[(inner, target.clone())], &|_| {}, &AtomicBool::new(false)).ok()?;
    Some(target)
}

/// Borra las copias temporales de más de un día (se llama al arrancar, en segundo plano).
pub fn clean_temp() {
    let dir = std::env::temp_dir().join("Rayo").join("comprimidos");
    let Ok(list) = std::fs::read_dir(&dir) else { return };
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in list.flatten() {
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

// ---------------------------------------------------------------- comprimir

/// Comprime en un ZIP: cada (origen, ruta dentro del ZIP con `\`, es carpeta).
pub fn compress(items: &[(String, String, bool)], zip_path: &str, progress: &dyn Fn(u64), cancel: &AtomicBool) -> Result<(), String> {
    use zip::write::SimpleFileOptions;
    let part = format!("{zip_path}.part");
    let f = std::fs::File::create(long_path(&part)).map_err(|e| e.to_string())?;
    let mut w = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let mut buf = vec![0u8; 256 * 1024];
    let res = (|| -> Result<(), String> {
        for (src, rel, is_dir) in items {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelado".into());
            }
            let name = rel.replace('\\', "/");
            let meta = std::fs::metadata(long_path(src)).map_err(|e| format!("{src}: {e}"))?;
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as u32);
            let (y, mo, d, h, mi, s) = unix_to_local(mtime);
            let mut opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(meta.len() >= 0xFFFF_FFFF);
            if let Ok(dt) = zip::DateTime::from_date_and_time(y.max(1980), mo, d, h, mi, s) {
                opts = opts.last_modified_time(dt);
            }
            if *is_dir {
                w.add_directory(format!("{name}/"), opts).map_err(|e| e.to_string())?;
                continue;
            }
            w.start_file(name, opts).map_err(|e| e.to_string())?;
            let mut r = std::fs::File::open(long_path(src)).map_err(|e| format!("{src}: {e}"))?;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelado".into());
                }
                let n = r.read(&mut buf).map_err(|e| format!("{src}: {e}"))?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                progress(n as u64);
            }
        }
        w.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
        Ok(())
    })();
    match res {
        Ok(()) => std::fs::rename(long_path(&part), long_path(zip_path)).map_err(|e| e.to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(long_path(&part));
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- fechas (hora local de Windows)

fn local_to_unix(y: u16, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> u32 {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{SystemTimeToFileTime, TzSpecificLocalTimeToSystemTime};
    let local = SYSTEMTIME { wYear: y, wMonth: mo as u16, wDay: d as u16, wHour: h as u16, wMinute: mi as u16, wSecond: s as u16, ..Default::default() };
    let mut utc = SYSTEMTIME::default();
    let mut ft = FILETIME::default();
    unsafe {
        if TzSpecificLocalTimeToSystemTime(None, &local, &mut utc).is_err() || SystemTimeToFileTime(&utc, &mut ft).is_err() {
            return 0;
        }
    }
    let t = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    crate::index::store::filetime_to_unix(t)
}

fn unix_to_local(unix: u32) -> (u16, u8, u8, u8, u8, u8) {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let t = (unix as u64 + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return (1980, 1, 1, 0, 0, 0);
        }
    }
    (local.wYear, local.wMonth as u8, local.wDay as u8, local.wHour as u8, local.wMinute as u8, local.wSecond as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_roundtrip_list_and_extract() {
        let d = std::env::temp_dir().join(format!("rayo-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src").join("sub")).unwrap();
        std::fs::write(d.join("src").join("a.txt"), "hola").unwrap();
        std::fs::write(d.join("src").join("sub").join("canción.txt"), "música").unwrap();
        let base = d.to_string_lossy().into_owned();
        let src = join(&base, "src");
        let items = vec![
            (join(&src, "a.txt"), "src\\a.txt".to_string(), false),
            (join(&src, "sub"), "src\\sub".to_string(), true),
            (join(&src, r"sub\canción.txt"), "src\\sub\\canción.txt".to_string(), false),
        ];
        let zip_path = join(&base, "prueba.zip");
        compress(&items, &zip_path, &|_| {}, &AtomicBool::new(false)).unwrap();

        // Navegar como carpeta.
        let (a, inner) = split(&join(&zip_path, r"src\sub")).unwrap();
        assert_eq!(a, zip_path);
        assert_eq!(inner, r"src\sub");
        assert!(is_dir(&zip_path));
        assert!(is_dir(&join(&zip_path, "src")));
        let root = list(&zip_path, "").unwrap();
        assert_eq!(root.len(), 1);
        assert!(root[0].is_dir && root[0].name == "src");
        assert_eq!(root[0].dir_size, Some(4 + "música".len() as u64));
        let sub = list(&zip_path, r"src\sub").unwrap();
        assert_eq!(sub[0].name, "canción.txt");
        assert!(sub[0].mtime > 0);

        // Extraer una carpeta de adentro.
        let out = join(&base, "salida");
        extract(&zip_path, &[(r"src\sub".into(), join(&out, "sub"))], &|_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read_to_string(join(&out, r"sub\canción.txt")).unwrap(), "música");

        // Copia temporal de un archivo.
        let tmp = materialize(&join(&zip_path, r"src\a.txt")).unwrap();
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "hola");

        // Un archivo que no es comprimido no se trata como carpeta.
        assert!(split(&join(&base, r"src\a.txt")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn seven_zip_list_and_extract() {
        let Some(exe) = seven_zip() else { return };
        let d = std::env::temp_dir().join(format!("rayo-7z-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src").join("sub")).unwrap();
        std::fs::write(d.join("src").join("a.txt"), "hola 7z").unwrap();
        std::fs::write(d.join("src").join("sub").join("canción.txt"), "música").unwrap();
        let base = d.to_string_lossy().into_owned();
        let archive = join(&base, "prueba.7z");
        let ok = command(exe).current_dir(d.join("src")).args(["a", "-bso0", &archive, "."]).status().unwrap();
        assert!(ok.success());
        let root = list(&archive, "").unwrap();
        assert!(root.iter().any(|e| e.name == "a.txt" && e.size == 7 && !e.is_dir));
        assert!(root.iter().any(|e| e.name == "sub" && e.is_dir));
        let sub = list(&archive, "sub").unwrap();
        assert_eq!(sub[0].name, "canción.txt");
        let out = join(&base, "salida");
        extract(&archive, &[("sub".into(), join(&out, "sub")), ("a.txt".into(), join(&out, "a.txt"))], &|_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read_to_string(join(&out, r"sub\canción.txt")).unwrap(), "música");
        assert_eq!(std::fs::read_to_string(join(&out, "a.txt")).unwrap(), "hola 7z");
        // No quedan carpetas temporales.
        assert!(std::fs::read_dir(&out).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".rayo")));
        // El comprimido entero y, a la vez, una carpeta de adentro.
        let todo = join(&base, "todo");
        extract(&archive, &[(String::new(), todo.clone()), ("sub".into(), join(&base, "solo-sub"))], &|_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read_to_string(join(&todo, r"sub\canción.txt")).unwrap(), "música");
        assert_eq!(std::fs::read_to_string(join(&base, r"solo-sub\canción.txt")).unwrap(), "música");
        let _ = std::fs::remove_dir_all(&d);
    }
}
