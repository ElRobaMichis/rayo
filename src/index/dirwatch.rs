//! Carpetas a la vista que el índice no vigila: unidades de red, NAS, recursos compartidos (\\servidor),
//! memorias USB en FAT/exFAT. Sin esto, lo que cambia ahí (otra persona guarda un archivo, termina una
//! copia) no aparece hasta refrescar a mano.
//!
//! Cada carpeta tiene su hilo con `ReadDirectoryChangesW` (sin subcarpetas). Si el servidor no avisa
//! de cambios (algunos NAS no lo hacen), se compara cada pocos segundos una firma barata del listado.
//! Abrir una carpeta de red caída puede tardar: pasa en el hilo, nunca en la interfaz.

use super::IndexManager;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject};
use windows::core::PCWSTR;

/// Un manejador de Windows que se puede pasar a otro hilo y se cierra solo.
struct Owned(HANDLE);
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Carpeta vigilada → evento para detener su hilo.
static WATCHES: Mutex<Option<HashMap<String, Arc<Owned>>>> = Mutex::new(None);

/// ¿Esta carpeta necesita vigilancia propia? (lo que el índice no cubre).
pub fn needs_own_watch(mgr: &IndexManager, path: &str) -> bool {
    if path.starts_with(crate::fsx::FLAT_KEY) || crate::fsx::archive::split(path).is_some() {
        return false;
    }
    path.starts_with("\\\\") || mgr.slot_for(path).is_none()
}

/// Deja vigiladas exactamente estas carpetas (las que están a la vista y el índice no cubre).
pub fn sync(mgr: &Arc<IndexManager>, dirs: &[String]) {
    let mut g = WATCHES.lock();
    let map = g.get_or_insert_with(HashMap::new);
    let want: Vec<String> = dirs.iter().map(|d| d.to_lowercase()).collect();
    map.retain(|k, stop| {
        let keep = want.contains(k);
        if !keep {
            unsafe {
                let _ = SetEvent(stop.0);
            }
        }
        keep
    });
    for d in dirs {
        let k = d.to_lowercase();
        if map.contains_key(&k) {
            continue;
        }
        let Ok(ev) = (unsafe { CreateEventW(None, true, false, None) }) else { continue };
        let stop = Arc::new(Owned(ev));
        map.insert(k, stop.clone());
        let (mgr, dir) = (mgr.clone(), d.clone());
        let _ = std::thread::Builder::new().name("vigia-carpeta".into()).spawn(move || watch(&mgr, &dir, &stop));
    }
}

fn watch(mgr: &IndexManager, dir: &str, stop: &Owned) {
    let w: Vec<u16> = crate::fsx::long_path(dir).encode_utf16().chain([0]).collect();
    let h = unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
    };
    match h {
        Ok(h) => {
            let h = Owned(h);
            if !notifications(mgr, dir, &h, stop) {
                poll(mgr, dir, stop);
            }
        }
        Err(_) => poll(mgr, dir, stop),
    }
}

/// Espera avisos de Windows. Devuelve false si esta carpeta no los da (hay que sondear).
fn notifications(mgr: &IndexManager, dir: &str, h: &Owned, stop: &Owned) -> bool {
    let Ok(ev) = (unsafe { CreateEventW(None, true, false, None) }) else { return false };
    let ev = Owned(ev);
    let mut buf = vec![0u32; 16 * 1024 / 4];
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_ATTRIBUTES;
    let mut first = true;
    loop {
        let mut ov = OVERLAPPED { hEvent: ev.0, ..Default::default() };
        let ok = unsafe { ReadDirectoryChangesW(h.0, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, false, filter, None, Some(&mut ov), None) };
        if ok.is_err() {
            return !first;
        }
        first = false;
        let r = unsafe { WaitForMultipleObjects(&[ev.0, stop.0], false, u32::MAX) };
        if r != WAIT_OBJECT_0 {
            // Detenida (ya no está a la vista) o error: cancelar el pedido y terminar.
            unsafe {
                let _ = CancelIoEx(h.0, Some(&ov));
                let mut got = 0;
                let _ = GetOverlappedResult(h.0, &ov, &mut got, true);
            }
            return true;
        }
        let mut got = 0u32;
        if unsafe { GetOverlappedResult(h.0, &ov, &mut got, false) }.is_err() {
            // La carpeta desapareció o se cayó la conexión: seguir por sondeo.
            return false;
        }
        mgr.notify_dirs(std::iter::once(dir));
        // Los cambios en ráfaga (una copia de muchos archivos) se juntan en un solo aviso.
        if unsafe { WaitForSingleObject(stop.0, 150) } == WAIT_OBJECT_0 {
            return true;
        }
    }
}

/// Sin avisos: cada 3 s una firma del listado (nombres, tamaños, fechas); si cambió, se avisa.
fn poll(mgr: &IndexManager, dir: &str, stop: &Owned) {
    let mut last = signature(dir);
    loop {
        if unsafe { WaitForSingleObject(stop.0, 3000) } != WAIT_TIMEOUT {
            return;
        }
        let now = signature(dir);
        if now != last {
            last = now;
            mgr.notify_dirs(std::iter::once(dir));
        }
    }
}

fn signature(dir: &str) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut n = 0u64;
    for e in std::fs::read_dir(crate::fsx::long_path(dir)).ok()?.flatten() {
        e.file_name().hash(&mut h);
        if let Ok(m) = e.metadata() {
            m.len().hash(&mut h);
            m.modified().ok().hash(&mut h);
        }
        n += 1;
    }
    n.hash(&mut h);
    Some(h.finish())
}
