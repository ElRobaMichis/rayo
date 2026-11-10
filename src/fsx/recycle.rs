//! La papelera de Windows: ver lo que hay, restaurar, eliminar definitivamente y vaciar.
//!
//! Los elementos de la papelera son objetos del shell (COM): viven en un hilo propio
//! y la interfaz le manda órdenes por un canal.

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::sync::Arc;
use windows::Win32::Foundation::{HWND, PROPERTYKEY};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemServices::SFGAO_FOLDER;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{GUID, Interface, PCSTR};

#[derive(Clone, Debug)]
pub struct Item {
    /// Clave estable: la ruta física ($Recycle.Bin\<SID>\$R…). Las órdenes van por clave,
    /// no por posición, porque la lista se vuelve a leer tras cada una.
    pub id: String,
    pub name: String,
    /// Carpeta de donde se borró.
    pub from: String,
    /// Cuándo se borró (segundos Unix).
    pub deleted: u32,
    pub size: u64,
    pub is_dir: bool,
}

impl Item {
    pub fn original_path(&self) -> String {
        super::join(&self.from, &self.name)
    }
}

enum Cmd {
    Refresh,
    Restore(Vec<String>),
    Delete(Vec<String>),
    Empty,
}

pub struct Bin {
    tx: Sender<Cmd>,
    /// Contenido (None mientras se lee) y el último error.
    pub items: Arc<Mutex<Option<Vec<Item>>>>,
    pub busy: Arc<std::sync::atomic::AtomicBool>,
    /// Órdenes enviadas cuya lista nueva aún no llegó (lo sube la interfaz al enviar,
    /// lo baja el hilo al publicar): sin esto, un doble clic encola contra una lista vieja.
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

const FMTID_RECYCLE: GUID = GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871);
const DELETED_FROM: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_RECYCLE, pid: 2 };
const DATE_DELETED: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_RECYCLE, pid: 3 };
const FMTID_STORAGE: GUID = GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac);
const NAME: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 10 };
const SIZE: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 12 };

fn enumerate() -> Vec<(Item, IShellItem2)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    unsafe {
        let Ok(bin) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) else { return out };
        let Ok(items) = bin.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut got = [None];
            let mut n = 0u32;
            if items.Next(&mut got, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = got[0].take() else { break };
            let Ok(item2) = item.cast::<IShellItem2>() else { continue };
            let text = |key: &PROPERTYKEY| -> String {
                match item2.GetString(key) {
                    Ok(p) => {
                        let s = p.to_string().unwrap_or_default();
                        CoTaskMemFree(Some(p.0 as *const _));
                        s
                    }
                    Err(_) => String::new(),
                }
            };
            let name = text(&NAME);
            let from = text(&DELETED_FROM);
            if name.is_empty() {
                continue;
            }
            let raw = item2.GetFileTime(&DATE_DELETED).map(|f| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64).unwrap_or(0);
            let deleted = if raw == 0 { 0 } else { crate::index::store::filetime_to_unix(raw) };
            let size = item2.GetUInt64(&SIZE).unwrap_or(0);
            let is_dir = item.GetAttributes(SFGAO_FOLDER).is_ok_and(|a| a.0 & SFGAO_FOLDER.0 != 0);
            let mut id = match item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
                Ok(p) => {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as *const _));
                    s
                }
                Err(_) => String::new(),
            };
            // Sin ruta (o repetida): origen, nombre, fecha y tamaño.
            if id.is_empty() || seen.contains(&id) {
                id = format!("{from}\\{name}\u{1}{raw}\u{1}{size}");
            }
            seen.insert(id.clone());
            out.push((Item { id, name, from, deleted, size, is_dir }, item2));
        }
    }
    out.sort_by(|a, b| b.0.deleted.cmp(&a.0.deleted));
    out
}

/// Ejecuta un verbo del menú contextual ("undelete", "delete") sobre un elemento de la papelera.
fn verb(item: &IShellItem2, verb: &[u8], hwnd: HWND) -> bool {
    unsafe {
        let Ok(menu) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else { return false };
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR(verb.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        menu.InvokeCommand(&info).is_ok()
    }
}

impl Bin {
    /// `hwnd`: ventana dueña de las confirmaciones de Windows. `on_change` recibe las
    /// carpetas a las que volvieron cosas (para recargarlas) y repinta.
    pub fn open(hwnd: HWND, on_change: Arc<dyn Fn(Vec<String>) + Send + Sync>) -> Self {
        let (tx, rx): (Sender<Cmd>, Receiver<Cmd>) = unbounded();
        let items: Arc<Mutex<Option<Vec<Item>>>> = Default::default();
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (it, b, pe) = (items.clone(), busy.clone(), pending.clone());
        let h = hwnd.0 as usize;
        std::thread::Builder::new()
            .name("papelera".into())
            .spawn(move || {
                super::shell::com_init();
                let hwnd = HWND(h as *mut _);
                let mut current = enumerate();
                *it.lock() = Some(current.iter().map(|c| c.0.clone()).collect());
                b.store(false, std::sync::atomic::Ordering::Relaxed);
                on_change(Vec::new());
                while let Ok(cmd) = rx.recv() {
                    b.store(true, std::sync::atomic::Ordering::Relaxed);
                    on_change(Vec::new());
                    let mut touched = Vec::new();
                    match cmd {
                        Cmd::Refresh => {}
                        // Lo que ya no está (restaurado o borrado por otro lado) se ignora.
                        Cmd::Restore(ids) => {
                            for id in ids {
                                if let Some((info, item)) = current.iter().find(|c| c.0.id == id) {
                                    if verb(item, b"undelete\0", hwnd) {
                                        touched.push(info.from.clone());
                                    }
                                }
                            }
                        }
                        Cmd::Delete(ids) => {
                            // Con varios, Windows pregunta una vez por cada uno; se confirma de a uno.
                            for id in ids {
                                if let Some((_, item)) = current.iter().find(|c| c.0.id == id) {
                                    verb(item, b"delete\0", hwnd);
                                }
                            }
                        }
                        Cmd::Empty => unsafe {
                            // Windows pide confirmación antes de vaciar.
                            let _ = SHEmptyRecycleBinW(Some(hwnd), None, 0);
                        },
                    }
                    current = enumerate();
                    *it.lock() = Some(current.iter().map(|c| c.0.clone()).collect());
                    pe.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    b.store(false, std::sync::atomic::Ordering::Relaxed);
                    on_change(touched);
                }
            })
            .ok();
        Bin { tx, items, busy, pending }
    }

    /// Ocupado: trabajando, o con órdenes enviadas cuya lista nueva no llegó.
    pub fn is_busy(&self) -> bool {
        self.busy.load(std::sync::atomic::Ordering::Relaxed) || self.pending.load(std::sync::atomic::Ordering::Acquire) > 0
    }

    fn send(&self, cmd: Cmd) {
        self.pending.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if self.tx.send(cmd).is_err() {
            self.pending.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    }

    pub fn refresh(&self) {
        self.send(Cmd::Refresh);
    }
    pub fn restore(&self, ids: Vec<String>) {
        self.send(Cmd::Restore(ids));
    }
    pub fn delete(&self, ids: Vec<String>) {
        self.send(Cmd::Delete(ids));
    }
    pub fn empty(&self) {
        self.send(Cmd::Empty);
    }
}
