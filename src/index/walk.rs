//! Indexado sin privilegios: recorre todas las carpetas en paralelo con
//! `FindFirstFileExW` (FindExInfoBasic + LARGE_FETCH), que trae nombre, tamaño
//! y fecha en una sola llamada por bloque de entradas.

use super::store::*;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

pub(crate) struct RawItem {
    pub name_end: u32,
    pub attrs: u32,
    pub size: u64,
    pub mtime: u32,
    pub sub: u32,
}

pub(crate) struct Listing {
    pub dir_id: u32,
    pub names: String,
    pub items: Vec<RawItem>,
}

pub fn attrs_to_flags(attrs: u32) -> u8 {
    let mut f = 0;
    if attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        f |= F_DIR;
    }
    if attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0 {
        f |= F_HIDDEN;
    }
    if attrs & FILE_ATTRIBUTE_SYSTEM.0 != 0 {
        f |= F_SYSTEM;
    }
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        f |= F_REPARSE;
    }
    f
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Lista una carpeta llamando `f` por cada hijo. `dir` debe terminar en `\`.
pub fn list_dir_raw(dir: &str, mut f: impl FnMut(&[u16], &WIN32_FIND_DATAW)) {
    let pattern = wide(&format!("{dir}*"));
    let mut data = WIN32_FIND_DATAW::default();
    let h: HANDLE = match unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    } {
        Ok(h) => h,
        Err(_) => return,
    };
    loop {
        let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
        let name = &data.cFileName[..len];
        let dot = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
        if !dot {
            f(name, &data);
        }
        if let Err(e) = unsafe { FindNextFileW(h, &mut data) } {
            if e.code() != ERROR_NO_MORE_FILES.to_hresult() {
                // Error a mitad de camino: nos quedamos con lo leído.
            }
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
}

#[inline]
pub fn find_size(d: &WIN32_FIND_DATAW) -> u64 {
    ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64
}

#[inline]
pub fn find_mtime(d: &WIN32_FIND_DATAW) -> u32 {
    let ft = ((d.ftLastWriteTime.dwHighDateTime as u64) << 32) | d.ftLastWriteTime.dwLowDateTime as u64;
    filetime_to_unix(ft)
}

struct Ctx<'a> {
    next_id: AtomicU32,
    out: Mutex<Vec<Listing>>,
    progress: &'a AtomicUsize,
}

fn visit<'s>(s: &rayon::Scope<'s>, path: String, dir_id: u32, ctx: &'s Ctx<'s>) {
    let mut names = String::new();
    let mut items = Vec::new();
    let mut subdirs = Vec::new();
    list_dir_raw(&path, |name, d| {
        let start = names.len();
        names.extend(char::decode_utf16(name.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')));
        let attrs = d.dwFileAttributes;
        let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        let recurse = is_dir && attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0;
        let sub = if recurse {
            let id = ctx.next_id.fetch_add(1, Ordering::Relaxed);
            subdirs.push((start, names.len(), id));
            id
        } else {
            NONE
        };
        items.push(RawItem {
            name_end: names.len() as u32,
            attrs,
            size: if is_dir { 0 } else { find_size(d) },
            mtime: find_mtime(d),
            sub,
        });
    });
    ctx.progress.fetch_add(items.len(), Ordering::Relaxed);
    for &(a, b, id) in &subdirs {
        let sub_path = format!("{path}{}\\", &names[a..b]);
        s.spawn(move |s| visit(s, sub_path, id, ctx));
    }
    ctx.out.lock().push(Listing { dir_id, names, items });
}

/// Indexa un volumen completo recorriendo sus carpetas.
pub fn walk_volume(letter: char, progress: &AtomicUsize) -> VolumeIndex {
    let ctx = Ctx { next_id: AtomicU32::new(1), out: Mutex::new(Vec::new()), progress };
    let root = format!("\\\\?\\{}:\\", letter.to_ascii_uppercase());
    // El cuello de botella es la E/S, así que conviene tener más hilos que núcleos.
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8) * 2;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.scope(|s| visit(s, root, 0, &ctx));

    let mut listings = ctx.out.into_inner();
    listings.sort_unstable_by_key(|l| l.dir_id);
    let total_dirs = ctx.next_id.load(Ordering::Relaxed) as usize;
    let entries: usize = listings.iter().map(|l| l.items.len()).sum();
    let bytes: usize = listings.iter().map(|l| l.names.len() + l.items.len()).sum();

    let mut v = VolumeIndex::with_capacity(letter, Source::Walk, entries + 1, bytes + 16);
    let mut dir_entry = vec![NONE; total_dirs];
    dir_entry[0] = 0;
    for l in listings {
        let parent = dir_entry[l.dir_id as usize];
        if parent == NONE {
            continue;
        }
        let mut start = 0usize;
        for it in &l.items {
            let end = it.name_end as usize;
            let name = &l.names[start..end];
            start = end;
            let id = v.push(name, parent, attrs_to_flags(it.attrs), it.size, it.mtime);
            if it.sub != NONE {
                dir_entry[it.sub as usize] = id;
            }
        }
    }
    v.compute_dir_sizes();
    v.shrink();
    v
}

/// Indexa una sola carpeta y todo su contenido dentro de un índice existente.
pub fn walk_into(v: &mut VolumeIndex, dir_entry: u32, dir_path: &str) {
    let mut stack = vec![(dir_entry, format!("{}\\", crate::fsx::long_path(dir_path.trim_end_matches('\\'))))];
    while let Some((parent, path)) = stack.pop() {
        let mut found = Vec::new();
        list_dir_raw(&path, |name, d| {
            found.push((String::from_utf16_lossy(name), d.dwFileAttributes, find_size(d), find_mtime(d)));
        });
        for (name, attrs, size, mtime) in found {
            let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
            let id = v.add_entry(parent, &name, attrs_to_flags(attrs), if is_dir { 0 } else { size }, mtime);
            if is_dir && attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
                stack.push((id, format!("{path}{name}\\")));
            }
        }
    }
}
