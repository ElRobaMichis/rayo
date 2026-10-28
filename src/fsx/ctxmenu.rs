//! El menú contextual de Windows (el del Explorador) leído como datos —textos, íconos,
//! submenús y verbos— para mostrarlo dentro del menú de Rayo, con su estilo. Así
//! aparecen 7-Zip, VLC, Git, "Enviar a", "Abrir con"… sin que Rayo los conozca.
//!
//! Todo corre en un hilo propio (los complementos de otras apps pueden tardar en
//! cargar): el menú de Rayo abre al instante y estas opciones llegan un momento después.

use super::thumbs::Rgba;
use crate::index::walk::wide;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCSTR, PCWSTR, PSTR};

/// Una opción del menú de Windows.
#[derive(Clone)]
pub struct ShellItem {
    /// Número de comando (para ejecutarla); 0 en separadores y submenús.
    pub id: u32,
    pub text: String,
    /// Verbo canónico ("open", "cut", "7-zip.extract"…) o vacío si no tiene.
    pub verb: String,
    pub icon: Option<Arc<Rgba>>,
    pub disabled: bool,
    pub separator: bool,
    pub children: Vec<ShellItem>,
}

/// El menú de Windows para un conjunto de archivos (o el fondo de una carpeta).
pub struct ShellMenu {
    pub token: u64,
    pub paths: Vec<String>,
    pub background: bool,
    pub items: Vec<ShellItem>,
}

/// Qué pasó al elegir una opción.
pub enum Outcome {
    /// Se ejecutó (las carpetas de estos elementos pueden haber cambiado).
    Invoked(Vec<String>),
    /// "Cambiar nombre": lo hace Rayo, en la lista.
    Rename(String),
}

enum Req {
    Build { token: u64, paths: Vec<String>, background: bool, extended: bool },
    Invoke { token: u64, id: u32, pos: (i32, i32) },
}

pub struct ShellMenus {
    tx: Sender<Req>,
    next: AtomicU64,
    ready: Arc<Mutex<Option<Arc<ShellMenu>>>>,
    outcome: Arc<Mutex<Option<Outcome>>>,
}

impl ShellMenus {
    /// `owner`: la ventana de Rayo (la usan los complementos para sus diálogos).
    pub fn new(owner: HWND, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = unbounded();
        let ready: Arc<Mutex<Option<Arc<ShellMenu>>>> = Default::default();
        let outcome: Arc<Mutex<Option<Outcome>>> = Default::default();
        let (r, o, owner) = (ready.clone(), outcome.clone(), owner.0 as isize);
        let _ = std::thread::Builder::new().name("menu-windows".into()).spawn(move || worker(HWND(owner as *mut _), rx, r, o, wake));
        ShellMenus { tx, next: AtomicU64::new(1), ready, outcome }
    }

    /// Pide el menú de Windows para estos elementos (o para el fondo de la carpeta `paths[0]`).
    pub fn build(&self, paths: Vec<String>, background: bool, extended: bool) {
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Req::Build { token, paths, background, extended });
    }

    /// El último menú listo, si es para estos elementos.
    pub fn get(&self, paths: &[String], background: bool) -> Option<Arc<ShellMenu>> {
        self.ready.lock().clone().filter(|m| m.background == background && m.paths == paths)
    }

    pub fn invoke(&self, token: u64, id: u32) {
        let _ = self.tx.send(Req::Invoke { token, id, pos: super::shell::cursor_pos() });
    }

    pub fn take_outcome(&self) -> Option<Outcome> {
        self.outcome.lock().take()
    }
}

/// Lo que vive mientras el menú está abierto (para poder ejecutar la opción elegida).
struct Live {
    token: u64,
    cm: IContextMenu,
    menu: HMENU,
    paths: Vec<String>,
}

impl Drop for Live {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

fn worker(owner: HWND, rx: Receiver<Req>, ready: Arc<Mutex<Option<Arc<ShellMenu>>>>, outcome: Arc<Mutex<Option<Outcome>>>, wake: Arc<dyn Fn() + Send + Sync>) {
    super::shell::com_init();
    let mut live: Option<Live> = None;
    loop {
        let req = match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(r) => r,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                pump();
                continue;
            }
            Err(_) => return,
        };
        match req {
            Req::Build { token, paths, background, extended } => {
                // Si llegaron varios pedidos seguidos, solo importa el último.
                live = None;
                let items = match unsafe { build(owner, &paths, background, extended) } {
                    Ok((cm, menu, items)) => {
                        live = Some(Live { token, cm, menu, paths: paths.clone() });
                        items
                    }
                    Err(_) => Vec::new(),
                };
                *ready.lock() = Some(Arc::new(ShellMenu { token, paths, background, items }));
                wake();
            }
            Req::Invoke { token, id, pos } => {
                let Some(l) = live.as_ref().filter(|l| l.token == token) else { continue };
                let verb = unsafe { verb_of(&l.cm, id) };
                if verb.eq_ignore_ascii_case("rename") {
                    *outcome.lock() = l.paths.first().cloned().map(Outcome::Rename);
                } else {
                    let info = CMINVOKECOMMANDINFOEX {
                        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                        fMask: 0x0000_4000 | CMIC_MASK_PTINVOKE, // CMIC_MASK_UNICODE
                        hwnd: owner,
                        lpVerb: PCSTR((id - 1) as usize as *const u8),
                        lpVerbW: PCWSTR((id - 1) as usize as *const u16),
                        nShow: SW_SHOWNORMAL.0,
                        ptInvoke: POINT { x: pos.0, y: pos.1 },
                        ..Default::default()
                    };
                    unsafe {
                        let _ = l.cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO);
                    }
                    *outcome.lock() = Some(Outcome::Invoked(l.paths.clone()));
                }
                wake();
            }
        }
        pump();
    }
}

/// Atender mensajes pendientes (algunos complementos los necesitan).
fn pump() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn build(owner: HWND, paths: &[String], background: bool, extended: bool) -> windows::core::Result<(IContextMenu, HMENU, Vec<ShellItem>)> {
    let cm: IContextMenu = if background {
        let dir = wide(paths.first().map(String::as_str).unwrap_or(""));
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(dir.as_ptr()), None)? };
        let folder: IShellFolder = unsafe { item.BindToHandler(None, &BHID_SFObject)? };
        unsafe { folder.CreateViewObject(owner)? }
    } else {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        let r = (|| {
            for p in paths {
                let w = wide(p);
                let mut pidl = std::ptr::null_mut();
                unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None)? };
                pidls.push(pidl);
            }
            let folder: IShellFolder = unsafe { SHBindToParent(pidls[0], None)? };
            let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|&p| unsafe { ILFindLastID(p) as *const _ }).collect();
            unsafe { folder.GetUIObjectOf::<IContextMenu>(owner, &children, None) }
        })();
        for p in pidls {
            unsafe { ILFree(Some(p)) };
        }
        r?
    };
    let menu = unsafe { CreatePopupMenu()? };
    let mut flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
    if extended {
        flags |= CMF_EXTENDEDVERBS;
    }
    if let Err(e) = unsafe { cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags) }.ok() {
        unsafe {
            let _ = DestroyMenu(menu);
        }
        return Err(e);
    }
    let cm3 = cm.cast::<IContextMenu3>().ok();
    let cm2 = cm.cast::<IContextMenu2>().ok();
    let items = unsafe { read_menu(&cm, cm3.as_ref(), cm2.as_ref(), menu, 0) };
    Ok((cm, menu, items))
}

/// El verbo canónico de un comando ("open", "rename", "7-zip.extract"…).
unsafe fn verb_of(cm: &IContextMenu, id: u32) -> String {
    if id == 0 {
        return String::new();
    }
    let mut buf = [0u16; 128];
    let ok = unsafe { cm.GetCommandString((id - 1) as usize, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32) }.is_ok();
    if !ok {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf16_lossy(&buf[..n])
}

/// Texto visible: sin el "&" de las teclas rápidas ni el atajo después de un tabulador.
fn clean_text(s: &str) -> String {
    let s = s.split('\t').next().unwrap_or("");
    s.replace("&&", "\u{1}").replace('&', "").replace('\u{1}', "&").trim().to_string()
}

unsafe fn read_menu(cm: &IContextMenu, cm3: Option<&IContextMenu3>, cm2: Option<&IContextMenu2>, menu: HMENU, depth: u32) -> Vec<ShellItem> {
    let mut out = Vec::new();
    let n = unsafe { GetMenuItemCount(Some(menu)) };
    for i in 0..n.max(0) {
        let mut buf = [0u16; 260];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP,
            dwTypeData: windows::core::PWSTR(buf.as_mut_ptr()),
            cch: (buf.len() - 1) as u32,
            ..Default::default()
        };
        if unsafe { GetMenuItemInfoW(menu, i as u32, true, &mut mii) }.is_err() {
            continue;
        }
        if mii.fType.contains(MFT_SEPARATOR) {
            out.push(ShellItem { id: 0, text: String::new(), verb: String::new(), icon: None, disabled: false, separator: true, children: Vec::new() });
            continue;
        }
        let text = clean_text(&String::from_utf16_lossy(&buf[..mii.cch as usize]));
        if text.is_empty() {
            // Dibujado por el complemento (sin texto que mostrar).
            continue;
        }
        let disabled = mii.fState.contains(MFS_DISABLED) || mii.fState.contains(MFS_GRAYED);
        let icon = unsafe { menu_icon(mii.hbmpItem) }.map(Arc::new);
        let mut children = Vec::new();
        if !mii.hSubMenu.is_invalid() && depth < 3 {
            // Los submenús de "Enviar a", 7-Zip, etc. se llenan recién al abrirse.
            let (wp, lp) = (WPARAM(mii.hSubMenu.0 as usize), LPARAM(i as isize));
            unsafe {
                if let Some(cm3) = cm3 {
                    let _ = cm3.HandleMenuMsg2(WM_INITMENUPOPUP, wp, lp, None);
                } else if let Some(cm2) = cm2 {
                    let _ = cm2.HandleMenuMsg(WM_INITMENUPOPUP, wp, lp);
                }
            }
            children = unsafe { read_menu(cm, cm3, cm2, mii.hSubMenu, depth + 1) };
            if children.iter().all(|c| c.separator) {
                continue;
            }
        }
        let verb = if children.is_empty() { unsafe { verb_of(cm, mii.wID) } } else { String::new() };
        out.push(ShellItem { id: if children.is_empty() { mii.wID } else { 0 }, text, verb, icon, disabled, separator: false, children });
    }
    out
}

/// Ícono de una opción (mapa de bits del menú). No se libera: es del menú.
unsafe fn menu_icon(h: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Rgba> {
    let v = h.0 as isize;
    // HBMMENU_CALLBACK (-1) y los íconos de sistema (1–11) no son mapas de bits.
    if (-1..=11).contains(&v) {
        return None;
    }
    super::thumbs::bitmap_pixels(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(items: &[ShellItem], depth: usize) {
        for it in items {
            if it.separator {
                eprintln!("{}---", "  ".repeat(depth));
                continue;
            }
            let icon = it.icon.as_ref().map(|i| format!("{}x{}", i.w, i.h)).unwrap_or_default();
            eprintln!("{}{} [{}] {} {}", "  ".repeat(depth), it.text, it.verb, icon, if it.disabled { "(desactivado)" } else { "" });
            dump(&it.children, depth + 1);
        }
    }

    /// Imprime el menú de Windows de un archivo (se corre a mano: RAYO_MENU_ARCHIVO=<ruta>).
    #[test]
    #[ignore]
    fn shows_windows_menu() {
        let Ok(path) = std::env::var("RAYO_MENU_ARCHIVO") else { return };
        crate::fsx::shell::com_init();
        let t = std::time::Instant::now();
        let (_cm, _menu, items) = unsafe { build(HWND(std::ptr::null_mut()), &[path], false, false) }.expect("menú");
        eprintln!("leído en {:?}", t.elapsed());
        dump(&items, 0);
    }
}
