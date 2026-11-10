//! Acceso al sistema de archivos de Windows: listado de carpetas, unidades,
//! carpetas conocidas y utilidades de formato.

pub mod collections;
pub mod dirsize;
pub mod git;
pub mod remote;
pub mod pinyin;
pub mod shell;
pub mod thumbs;
pub mod transfer;
pub mod disks;
pub mod rename;
pub mod dupes;
pub mod compare;
pub mod archive;
pub mod pdf;
pub mod shellnew;
pub mod recycle;
pub mod hash;
pub mod links;
pub mod locks;
pub mod props;
pub mod tags;
pub mod ctxmenu;

use crate::index::store::filetime_to_unix;
use crate::index::walk::wide;
use std::cmp::Ordering;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub lower: String,
    pub ext: String,
    pub is_dir: bool,
    pub hidden: bool,
    pub size: u64,
    /// Tamaño recursivo de la carpeta según el índice.
    pub dir_size: Option<u64>,
    pub mtime: u32,
    pub ctime: u32,
    /// Estado en OneDrive (u otro proveedor de nube).
    pub cloud: Cloud,
    /// Enlace simbólico o unión (lo de adentro está en otro lado): al mover se copia pero no se
    /// borra, y no se recorre en la vista plana, los duplicados ni al comparar. Los archivos de
    /// OneDrive no cuentan como enlace.
    pub link: bool,
}

/// Estado de un archivo sincronizado con la nube (OneDrive, Dropbox, iCloud…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cloud {
    /// Archivo normal.
    #[default]
    None,
    /// Solo en la nube: leerlo lo descargaría.
    OnlineOnly,
    /// Descargado, sincronizado.
    Local,
    /// "Mantener siempre en este dispositivo".
    Pinned,
}

const ATTR_RECALL_ON_OPEN: u32 = 0x0004_0000;
const ATTR_PINNED: u32 = 0x0008_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
const ATTR_OFFLINE: u32 = 0x0000_1000;
const ATTR_REPARSE: u32 = 0x0000_0400;

/// Estado de nube a partir de los atributos (y la etiqueta de reparse) que da el listado.
pub fn cloud_of(attrs: u32, reparse_tag: u32) -> Cloud {
    if attrs & (ATTR_RECALL_ON_DATA_ACCESS | ATTR_OFFLINE | ATTR_RECALL_ON_OPEN) != 0 {
        Cloud::OnlineOnly
    } else if attrs & ATTR_PINNED != 0 {
        Cloud::Pinned
    } else if attrs & ATTR_REPARSE != 0 && reparse_tag & 0xFFFF_0FFF == 0x9000_001A {
        Cloud::Local
    } else {
        Cloud::None
    }
}

/// ¿Leer este archivo lo descargaría de la nube? (consulta solo los atributos: no descarga nada)
pub fn is_online_only(path: &str) -> bool {
    let w = wide(&long_path(path));
    let a = unsafe { GetFileAttributesW(PCWSTR(w.as_ptr())) };
    a != INVALID_FILE_ATTRIBUTES && a & (ATTR_RECALL_ON_DATA_ACCESS | ATTR_OFFLINE | ATTR_RECALL_ON_OPEN) != 0
}

impl FileEntry {
    pub fn display_size(&self) -> Option<u64> {
        if self.is_dir { self.dir_size } else { Some(self.size) }
    }
}

/// Ruta en formato extendido (`\\?\C:\...` o `\\?\UNC\servidor\...`) para que
/// las llamadas de Windows acepten más de 260 caracteres.
pub fn long_path(p: &str) -> String {
    if p.starts_with(r"\\?\") {
        p.to_string()
    } else if let Some(rest) = p.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{rest}")
    } else {
        format!(r"\\?\{}", p.replace('/', "\\"))
    }
}

/// Quita el prefijo extendido para mostrar o guardar la ruta.
pub fn short_path(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') { format!("{dir}{name}") } else { format!("{dir}\\{name}") }
}

pub fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches('\\');
    // Ruta de red: la raíz es \\servidor\recurso (más arriba no hay carpeta que listar).
    if let Some(rest) = t.strip_prefix(r"\\") {
        if rest.split('\\').filter(|s| !s.is_empty()).count() <= 2 {
            return None;
        }
        return t.rfind('\\').map(|i| t[..i].to_string());
    }
    let i = t.rfind('\\')?;
    if i <= 2 { Some(format!("{}\\", &t[..2])) } else { Some(t[..i].to_string()) }
}

pub fn file_name(path: &str) -> String {
    let t = path.trim_end_matches('\\');
    match t.rfind('\\') {
        Some(i) => t[i + 1..].to_string(),
        None => t.to_string(),
    }
}

/// Lo escrito por el usuario: `~` es su carpeta personal y se expanden variables (`%APPDATA%`).
pub fn expand_user_path(s: &str) -> String {
    let s = s.trim().trim_matches('"');
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let s = if s == "~" {
        home
    } else if let Some(rest) = s.strip_prefix("~\\").or_else(|| s.strip_prefix("~/")) {
        join(&home, rest)
    } else {
        s.to_string()
    };
    if !s.contains('%') {
        return s;
    }
    let w = wide(&s);
    let mut buf = vec![0u16; 4096];
    let n = unsafe { windows::Win32::System::Environment::ExpandEnvironmentStringsW(windows::core::PCWSTR(w.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() { s } else { String::from_utf16_lossy(&buf[..n - 1]) }
}

/// Normaliza una ruta: `c:` → `C:\`, sin barra final salvo en la raíz.
pub fn normalize(path: &str) -> String {
    let mut p = path.trim().replace('/', "\\");
    if p.len() >= 2 && p.as_bytes()[1] == b':' {
        let first = p[..1].to_ascii_uppercase();
        p.replace_range(..1, &first);
    }
    if p.len() == 2 && p.ends_with(':') {
        p.push('\\');
    }
    while p.len() > 3 && p.ends_with('\\') {
        p.pop();
    }
    p
}

/// Una descarga a medias de un navegador ("foto.jpg.crdownload" en Chrome, Edge o Brave,
/// "foto.jpg.part" en Firefox): devuelve el nombre que tendrá al terminar.
pub fn downloading(name: &str) -> Option<&str> {
    let l = name.to_ascii_lowercase();
    [".crdownload", ".part", ".partial", ".opdownload"]
        .iter()
        .find(|s| l.ends_with(*s) && name.len() > s.len())
        .map(|s| &name[..name.len() - s.len()])
}

pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// Prefijo de las claves de la vista plana en la caché de carpetas.
pub const FLAT_KEY: &str = "\u{1}plana\u{1}";

/// Todos los archivos bajo `dir` recorriendo carpetas (para discos sin índice).
pub fn list_flat(dir: &str, limit: usize) -> Result<Vec<FileEntry>, String> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_string(), String::new(), false)];
    let mut first = true;
    while let Some((d, rel, hidden_parent)) = stack.pop() {
        let list = match list_dir(&d) {
            Ok(l) => l,
            Err(e) if first => return Err(e),
            Err(_) => continue,
        };
        first = false;
        for mut e in list {
            let r = if rel.is_empty() { e.name.clone() } else { format!("{rel}\\{}", e.name) };
            let hidden = hidden_parent || e.hidden;
            if e.is_dir {
                // Un enlace o unión no se recorre (podría dar vueltas o salir del árbol).
                if !e.link {
                    stack.push((join(&d, &e.name), r, hidden));
                }
            } else {
                e.hidden = hidden;
                e.lower = r.to_lowercase();
                e.name = r;
                out.push(e);
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

/// ¿Se puede entrar como carpeta? (carpeta real, comprimido o carpeta dentro de uno)
pub fn is_dir_like(p: &str) -> bool {
    std::path::Path::new(&long_path(p)).is_dir() || archive::is_dir(p)
}

/// ¿Está en la red? (\\servidor\recurso o una unidad de red asignada, como Z:)
pub fn is_remote(p: &str) -> bool {
    if p.starts_with(r"\\") && !p.starts_with(r"\\?\") {
        return true;
    }
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let root: Vec<u16> = format!("{}:\\", &p[..1]).encode_utf16().chain([0]).collect();
        // DRIVE_REMOTE = 4. Es una consulta local: no toca la red.
        return unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) } == 4;
    }
    false
}

/// ¿Es una carpeta? Sin trabar la interfaz: en la red se pregunta en otro hilo y, si no contesta
/// en `ms` milisegundos (servidor caído, VPN desconectada), devuelve None ("no se sabe todavía").
pub fn probe_dir(p: &str, ms: u64) -> Option<bool> {
    if !is_remote(p) {
        return Some(std::path::Path::new(&long_path(p)).is_dir());
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let path = long_path(p);
    std::thread::spawn(move || {
        let _ = tx.send(std::path::Path::new(&path).is_dir());
    });
    rx.recv_timeout(std::time::Duration::from_millis(ms)).ok()
}

/// Separa (lo que existe, lo que ya no existe de verdad). Si falta su disco o recurso de red
/// (USB desconectado, servidor apagado), no va a ninguno de los dos: no se sabe, no se olvida.
pub fn split_existing(paths: Vec<String>) -> (Vec<String>, Vec<String>) {
    let (mut here, mut gone) = (Vec::new(), Vec::new());
    for p in paths {
        if std::path::Path::new(&long_path(&p)).exists() {
            here.push(p);
        } else if volume_root(&p).is_some_and(|r| std::path::Path::new(&long_path(&r)).exists()) {
            gone.push(p);
        }
    }
    (here, gone)
}

/// "C:\a\b" → "C:\"; "\\servidor\recurso\a" → "\\servidor\recurso\".
fn volume_root(p: &str) -> Option<String> {
    if let Some(rest) = p.strip_prefix(r"\\").filter(|_| !p.starts_with(r"\\?\")) {
        let mut parts = rest.split('\\').filter(|s| !s.is_empty());
        let (server, share) = (parts.next()?, parts.next()?);
        return Some(format!(r"\\{server}\{share}\"));
    }
    let b = p.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| format!("{}\\", &p[..2]))
}

/// ¿Existe? (también dentro de un comprimido)
pub fn exists_like(p: &str) -> bool {
    std::path::Path::new(&long_path(p)).exists() || archive::split(p).is_some_and(|(a, inner)| inner.is_empty() || archive::find(&a, &inner).is_some())
}

pub fn list_dir(dir: &str) -> Result<Vec<FileEntry>, String> {
    // Dentro de un comprimido (o el comprimido mismo): se lista su contenido.
    if let Some((archive, inner)) = archive::split(dir) {
        return archive::list(&archive, &inner);
    }
    let base = long_path(dir);
    let pattern = wide(&format!("{}*", if base.ends_with('\\') { base } else { format!("{base}\\") }));
    let mut d = WIN32_FIND_DATAW::default();
    let found = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut d as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let h: HANDLE = match found {
        Ok(h) => h,
        // La raíz de un disco o recurso vacío no tiene ni "." ni "..": no hay nada, no es un error.
        Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(Vec::new()),
        Err(e) => return Err(e.message().trim().to_string()),
    };
    let mut out = Vec::new();
    loop {
        let len = d.cFileName.iter().position(|&c| c == 0).unwrap_or(d.cFileName.len());
        let name = String::from_utf16_lossy(&d.cFileName[..len]);
        if name != "." && name != ".." {
            let attrs = d.dwFileAttributes;
            let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
            let ft = |t: windows::Win32::Foundation::FILETIME| filetime_to_unix(((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64);
            out.push(FileEntry {
                lower: name.to_lowercase(),
                ext: if is_dir { String::new() } else { ext_of(&name) },
                is_dir,
                hidden: attrs & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0,
                size: if is_dir { 0 } else { ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64 },
                dir_size: None,
                mtime: ft(d.ftLastWriteTime),
                ctime: ft(d.ftCreationTime),
                cloud: cloud_of(attrs, d.dwReserved0),
                // Etiqueta de "sustituto de nombre" (como `is_symlink` de std): uniones y enlaces.
                link: attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 && d.dwReserved0 & 0x2000_0000 != 0,
                name,
            });
        }
        if unsafe { FindNextFileW(h, &mut d) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
    Ok(out)
}

/// Comparación "natural": `archivo2` va antes que `archivo10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < ab.len() && j < bb.len() {
        if ab[i].is_ascii_digit() && bb[j].is_ascii_digit() {
            let si = i;
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let na = a[si..i].trim_start_matches('0');
            let nb = b[sj..j].trim_start_matches('0');
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            let o = ab[i].cmp(&bb[j]);
            if o != Ordering::Equal {
                return o;
            }
            i += 1;
            j += 1;
        }
    }
    (ab.len() - i).cmp(&(bb.len() - j))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Kind,
    Size,
}

pub fn sort_entries(v: &mut [FileEntry], key: SortKey, desc: bool) {
    // Nombres en chino: por su pinyin (como el Explorador en chino), no por el código Unicode.
    if matches!(key, SortKey::Name | SortKey::Kind) && v.iter().any(|e| pinyin::has_cjk(&e.lower)) {
        let keys: std::collections::HashMap<String, String> = v.iter().filter(|e| pinyin::has_cjk(&e.lower)).map(|e| (e.lower.clone(), pinyin::sort_key(&e.lower).into_owned())).collect();
        let k = |e: &FileEntry| keys.get(&e.lower).cloned().unwrap_or_else(|| e.lower.clone());
        v.sort_by(|a, b| {
            let dirs = b.is_dir.cmp(&a.is_dir);
            if dirs != Ordering::Equal {
                return dirs;
            }
            let o = match key {
                SortKey::Kind => a.ext.cmp(&b.ext).then_with(|| natural_cmp(&k(a), &k(b))),
                _ => natural_cmp(&k(a), &k(b)),
            };
            if desc { o.reverse() } else { o }
        });
        return;
    }
    v.sort_by(|a, b| {
        // Por tamaño y por fecha se mezclan carpetas y archivos: así se ve qué ocupa más espacio
        // y qué es lo último (la descarga de hoy arriba, aunque haya mil carpetas).
        let dirs = if matches!(key, SortKey::Size | SortKey::Modified) { Ordering::Equal } else { b.is_dir.cmp(&a.is_dir) };
        if dirs != Ordering::Equal {
            return dirs;
        }
        let o = match key {
            SortKey::Name => natural_cmp(&a.lower, &b.lower),
            SortKey::Modified => a.mtime.cmp(&b.mtime),
            SortKey::Kind => a.ext.cmp(&b.ext).then_with(|| natural_cmp(&a.lower, &b.lower)),
            SortKey::Size => a.display_size().unwrap_or(0).cmp(&b.display_size().unwrap_or(0)),
        };
        if desc { o.reverse() } else { o }
    });
}

// ---------------------------------------------------------------- unidades

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Other,
}

#[derive(Debug, Clone)]
pub struct Drive {
    pub letter: char,
    pub label: String,
    pub kind: DriveKind,
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub ejectable: bool,
}

impl Drive {
    pub fn root(&self) -> String {
        format!("{}:\\", self.letter)
    }
    pub fn used_frac(&self) -> f32 {
        if self.total == 0 { 0.0 } else { 1.0 - self.free as f32 / self.total as f32 }
    }
}

pub fn drive_mask() -> u32 {
    unsafe { GetLogicalDrives() }
}

fn is_usb(letter: char) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::*;
    let path = wide(&format!("\\\\.\\{letter}:"));
    let Ok(h) = (unsafe { CreateFileW(PCWSTR(path.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }) else {
        return false;
    };
    let q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, ..Default::default() };
    let mut out = [0u8; 1024];
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&q as *const _ as *const _),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(out.as_mut_ptr() as *mut _),
            out.len() as u32,
            Some(&mut got),
            None,
        )
    }
    .is_ok();
    unsafe {
        let _ = CloseHandle(h);
    }
    if !ok || (got as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return false;
    }
    let desc = unsafe { &*(out.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    desc.BusType == BusTypeUsb || desc.BusType == BusTypeSd || desc.BusType == BusTypeMmc
}

pub fn list_drives() -> Vec<Drive> {
    let mask = drive_mask();
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        let kind = match unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } {
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Network,
            5 => DriveKind::Optical,
            _ => DriveKind::Other,
        };
        let mut label = [0u16; 128];
        let mut fs = [0u16; 32];
        let has_media = unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut label), None, None, None, Some(&mut fs)) }.is_ok();
        let s = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(0)]);
        let (mut total, mut free) = (0u64, 0u64);
        if has_media {
            unsafe {
                let _ = GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), None, Some(&mut total), Some(&mut free));
            }
        }
        if kind == DriveKind::Removable && !has_media {
            continue;
        }
        let ejectable = kind == DriveKind::Removable || kind == DriveKind::Optical || (kind == DriveKind::Fixed && is_usb(letter));
        out.push(Drive { letter, label: s(&label), kind, fs: s(&fs), total, free, ejectable });
    }
    out
}

// ---------------------------------------------------------------- carpetas conocidas

#[derive(Debug, Clone)]
pub struct Place {
    pub name: &'static str,
    pub icon: &'static str,
    pub path: String,
}

pub fn known_places() -> Vec<Place> {
    use egui_phosphor::regular as ic;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::*;
    let get = |id: &windows::core::GUID| -> Option<String> {
        unsafe {
            let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        }
    };
    let list: [(&'static str, &'static str, &windows::core::GUID); 7] = [
        (t!("archivo.inicio"), ic::HOUSE, &FOLDERID_Profile),
        (t!("archivo.escritorio"), ic::DESKTOP, &FOLDERID_Desktop),
        (t!("archivo.documentos"), ic::FILE_TEXT, &FOLDERID_Documents),
        (t!("archivo.descargas"), ic::DOWNLOAD_SIMPLE, &FOLDERID_Downloads),
        (t!("archivo.imagenes"), ic::IMAGES, &FOLDERID_Pictures),
        (t!("archivo.musica"), ic::MUSIC_NOTES, &FOLDERID_Music),
        (t!("archivo.videos"), ic::FILM_STRIP, &FOLDERID_Videos),
    ];
    list.into_iter().filter_map(|(name, icon, id)| Some(Place { name, icon, path: get(id)? })).collect()
}

// ---------------------------------------------------------------- formato

pub fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 { format!("{v:.0} {}", U[i]) } else if v >= 10.0 { format!("{v:.1} {}", U[i]) } else { format!("{v:.2} {}", U[i]) }
}

pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Desfase local en segundos, calculado una vez.
pub fn tz_offset() -> i64 {
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(crate::index::search::local_utc_offset_secs)
}

/// (año, mes, día, hora, minuto) en hora local.
pub fn civil(unix: u32) -> (i64, u32, u32, u32, u32) {
    let t = unix as i64 + tz_offset();
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // Algoritmo de Howard Hinnant (days_from_civil inverso).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d, (secs / 3600) as u32, ((secs % 3600) / 60) as u32)
}

pub fn fmt_date(unix: u32) -> String {
    if unix == 0 {
        return String::new();
    }
    let (y, m, d, h, mi) = civil(unix);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}")
}

/// Antigüedad corta: "ahora", "5 min", "3 h", "2 d", "3 sem", "4 m", "2 a".
pub fn fmt_age(unix: u32, now: u32) -> String {
    let s = now.saturating_sub(unix) as u64;
    match s {
        0..60 => t!("antiguedad.ahora").into(),
        60..3600 => tf!("antiguedad.minutos", n = s / 60),
        3600..86_400 => tf!("antiguedad.horas", n = s / 3600),
        86_400..604_800 => tf!("antiguedad.dias", n = s / 86_400),
        604_800..2_592_000 => tf!("antiguedad.semanas", n = s / 604_800),
        2_592_000..31_536_000 => tf!("antiguedad.meses", n = s / 2_592_000),
        _ => tf!("antiguedad.anios", n = s / 31_536_000),
    }
}

pub fn kind_label(e: &FileEntry) -> String {
    if e.is_dir {
        t!("comun.carpeta").into()
    } else if e.ext.is_empty() {
        t!("comun.archivo").into()
    } else {
        tf!("comun.archivo_extension", extension = e.ext.to_uppercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(parent_of("C:\\a\\b").as_deref(), Some("C:\\a"));
        assert_eq!(parent_of("C:\\a").as_deref(), Some("C:\\"));
        assert_eq!(parent_of("C:\\"), None);
        assert_eq!(parent_of(r"\\srv\share\a\b").as_deref(), Some(r"\\srv\share\a"));
        assert_eq!(parent_of(r"\\srv\share\a").as_deref(), Some(r"\\srv\share"));
        assert_eq!(parent_of(r"\\srv\share"), None);
        assert_eq!(parent_of(r"\\srv\share\"), None);
        assert_eq!(normalize("c:"), "C:\\");
        assert_eq!(normalize("c:/users/"), "C:\\users");
        assert_eq!(human_size(1536), "1.50 KB");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(long_path(r"C:\a"), r"\\?\C:\a");
        assert_eq!(long_path(r"\\srv\share\x"), r"\\?\UNC\srv\share\x");
        assert_eq!(short_path(&long_path(r"\\srv\share")), r"\\srv\share");
        assert_eq!(short_path(&long_path(r"C:\a")), r"C:\a");
    }
}
