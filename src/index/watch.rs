//! Mantiene el índice al día escuchando los cambios del volumen con
//! `ReadDirectoryChangesW` (funciona sin permisos de administrador).

use super::store::*;
use super::walk::{attrs_to_flags, walk_into, wide};
use super::{IndexManager, Slot};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

pub struct Stat {
    pub attrs: u32,
    pub size: u64,
    pub mtime: u32,
}

pub fn stat(path: &str) -> Option<Stat> {
    let w = wide(&crate::fsx::long_path(path));
    let mut d = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(PCWSTR(w.as_ptr()), GetFileExInfoStandard, &mut d as *mut _ as *mut _) }.ok()?;
    let ft = ((d.ftLastWriteTime.dwHighDateTime as u64) << 32) | d.ftLastWriteTime.dwLowDateTime as u64;
    Some(Stat {
        attrs: d.dwFileAttributes,
        size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
        mtime: filetime_to_unix(ft),
    })
}

fn split_parent(full: &str) -> Option<(&str, &str)> {
    let i = full.rfind('\\')?;
    Some((&full[..i.max(3)], &full[i + 1..]))
}

fn add_or_update(v: &mut VolumeIndex, full: &str) {
    let Some(st) = stat(full) else { return };
    let Some((parent_path, name)) = split_parent(full) else { return };
    let Some(p) = v.find_path(parent_path) else { return };
    let is_dir = st.attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    if let Some(existing) = v.find_child(p, name) {
        v.update_file(existing, st.size, st.mtime);
        return;
    }
    let id = v.add_entry(p, name, attrs_to_flags(st.attrs), if is_dir { 0 } else { st.size }, st.mtime);
    if is_dir && st.attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0 {
        walk_into(v, id, full);
    }
}

fn apply(v: &mut VolumeIndex, action: FILE_ACTION, full: &str) {
    match action {
        FILE_ACTION_ADDED | FILE_ACTION_RENAMED_NEW_NAME => add_or_update(v, full),
        FILE_ACTION_REMOVED | FILE_ACTION_RENAMED_OLD_NAME => {
            if let Some(i) = v.find_path(full) {
                v.remove_subtree(i);
            }
        }
        FILE_ACTION_MODIFIED => {
            if let Some(i) = v.find_path(full) {
                if let Some(st) = stat(full) {
                    v.update_file(i, st.size, st.mtime);
                }
            }
        }
        _ => {}
    }
}

struct DirHandle(HANDLE);
impl Drop for DirHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Bucle bloqueante; corre en su propio hilo.
pub fn watch_volume(mgr: Arc<IndexManager>, slot: Arc<Slot>) {
    let root = format!("{}:\\", slot.letter);
    let w = wide(&root);
    let h = match unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    } {
        Ok(h) => DirHandle(h),
        Err(_) => return,
    };
    // u32 para garantizar la alineación que exige la API.
    let mut buf = vec![0u32; 64 * 1024 / 4];
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;
    loop {
        if mgr.shutdown.load(Ordering::Relaxed) {
            return;
        }
        let mut got = 0u32;
        let ok = unsafe {
            ReadDirectoryChangesW(h.0, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, true, filter, Some(&mut got), None, None)
        };
        if ok.is_err() {
            return;
        }
        if got == 0 {
            // Se desbordó el buffer: hubo demasiados cambios juntos.
            slot.needs_rescan.store(true, Ordering::Relaxed);
            mgr.changes.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
        let mut events = Vec::new();
        let mut off = 0usize;
        loop {
            if off + 12 > bytes.len() {
                break;
            }
            let next = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            let action = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
            let len = u32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()) as usize;
            let name_bytes = &bytes[off + 12..(off + 12 + len).min(bytes.len())];
            let name16: Vec<u16> = name_bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            events.push((FILE_ACTION(action), format!("{root}{}", String::from_utf16_lossy(&name16))));
            if next == 0 {
                break;
            }
            off += next;
        }
        // Si el USN Journal ya mantiene este disco, la vigilancia clásica sobra.
        if slot.usn_active.load(Ordering::Relaxed) {
            continue;
        }
        {
            let mut v = slot.index.write();
            for (action, full) in &events {
                apply(&mut v, *action, full);
            }
        }
        mgr.changes.fetch_add(1, Ordering::Relaxed);
        mgr.notify_dirs(events.iter().filter_map(|(_, full)| split_parent(full).map(|(p, _)| p)));
    }
}
