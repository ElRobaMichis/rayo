//! Operaciones del shell de Windows: abrir, propiedades, papelera, copiar,
//! mover, portapapeles de archivos, expulsar unidades y el menú contextual nativo.

use crate::index::walk::wide;
use std::cell::RefCell;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCSTR, PCWSTR, PSTR};

pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn exec(verb: &str, file: &str, params: Option<&str>, dir: Option<&str>) -> bool {
    let v = wide(verb);
    let f = wide(file);
    let p = params.map(wide);
    let d = dir.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(v.as_ptr()),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: p.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        lpDirectory: d.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.is_ok()
}

/// Ejecuta este mismo programa con permisos de administrador (Windows pide
/// confirmación) y espera a que termine. Devuelve el código de salida.
pub fn run_self_elevated(args: &str) -> Result<u32, String> {
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let file = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params = wide(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|_| t!("shell.cancelo_permiso_administrador").to_string())?;
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = windows::Win32::Foundation::CloseHandle(info.hProcess);
    }
    Ok(code)
}

pub fn open(path: &str) -> bool {
    exec("open", path, None, None)
}

pub fn open_with(path: &str) -> bool {
    exec("openas", path, None, None)
}

pub fn properties(path: &str) {
    let w = wide(path);
    unsafe {
        let _ = SHObjectProperties(None, SHOP_FILEPATH, PCWSTR(w.as_ptr()), PCWSTR::null());
    }
}

pub fn reveal_in_explorer(path: &str) {
    crate::integration::allow_explorer();
    exec("open", "explorer.exe", Some(&format!("/select,\"{path}\"")), None);
}

/// Abre una carpeta en el Explorador de Windows (no con el programa predeterminado,
/// que puede ser Rayo).
pub fn explorer_at(dir: &str) {
    crate::integration::allow_explorer();
    exec("open", "explorer.exe", Some(&format!("\"{dir}\"")), None);
}

/// Ejecuta lo escrito en la barra de ruta como en «Ejecutar» de Windows (`cmd`,
/// `powershell`, `code .`, `notepad notas.txt`, una dirección web…) con la carpeta
/// actual como carpeta de trabajo. Devuelve qué se abrió, o None si no existe.
pub fn run_typed(line: &str, dir: &str) -> Option<String> {
    let line = line.trim();
    let (prog, args) = if let Some(rest) = line.strip_prefix('"') {
        match rest.find('"') {
            Some(i) => (rest[..i].to_string(), rest[i + 1..].trim().to_string()),
            None => (rest.to_string(), String::new()),
        }
    } else {
        match line.find(char::is_whitespace) {
            Some(i) => (line[..i].to_string(), line[i..].trim().to_string()),
            None => (line.to_string(), String::new()),
        }
    };
    if prog.is_empty() {
        return None;
    }
    let base = prog.to_lowercase();
    let base = base.trim_end_matches(".exe");
    // Estos no usan la carpeta de trabajo: se les pasa la carpeta.
    if args.is_empty() {
        match base {
            "wt" | "terminal" => {
                open_terminal(dir);
                return Some(t!("shell.terminal").into());
            }
            "explorer" => {
                explorer_at(dir);
                return Some(t!("shell.explorador").into());
            }
            _ => {}
        }
    }
    let file = find_on_path(&prog).unwrap_or_else(|| prog.clone());
    let f = wide(&file);
    let p = wide(&args);
    let d = wide(dir);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(p.as_ptr()) },
        lpDirectory: if dir.is_empty() { PCWSTR::null() } else { PCWSTR(d.as_ptr()) },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.ok().map(|_| prog)
}

/// Busca un programa en el PATH (con las extensiones de PATHEXT si no tiene).
fn find_on_path(prog: &str) -> Option<String> {
    if prog.contains(['\\', '/', ':']) {
        return None;
    }
    let exts: Vec<String> = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()).split(';').filter(|e| !e.is_empty()).map(|e| e.to_lowercase()).collect();
    let has_ext = exts.iter().any(|e| prog.to_lowercase().ends_with(e.as_str()));
    let path = std::env::var("PATH").unwrap_or_default();
    for d in path.split(';').map(|d| d.trim().trim_matches('"')).filter(|d| !d.is_empty()) {
        let d = super::expand_user_path(d);
        if has_ext {
            let c = super::join(&d, prog);
            if std::path::Path::new(&c).is_file() {
                return Some(c);
            }
        } else {
            for e in &exts {
                let c = super::join(&d, &format!("{prog}{e}"));
                if std::path::Path::new(&c).is_file() {
                    return Some(c);
                }
            }
        }
    }
    None
}

pub fn open_terminal(dir: &str) {
    if !exec("open", "wt.exe", Some(&format!("-d \"{dir}\"")), Some(dir)) {
        exec("open", "cmd.exe", None, Some(dir));
    }
}

fn multi_sz(paths: &[String]) -> Vec<u16> {
    let mut v = Vec::new();
    for p in paths {
        v.extend(p.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileOp {
    Copy,
    Move,
    Recycle,
    DeleteForever,
}

/// Ejecuta una operación de archivos con el diálogo de progreso nativo de
/// Windows, en un hilo aparte para no congelar la interfaz. `done` recibe si el
/// usuario la canceló.
///
/// Al borrar no se muestran los errores de Windows: si algo queda sin borrar, Rayo
/// dice por qué (y quién lo tiene abierto).
pub fn file_op(op: FileOp, srcs: Vec<String>, dest: Option<String>, done: impl FnOnce(bool) + Send + 'static) {
    std::thread::spawn(move || {
        com_init();
        let (_, aborted) = run_file_op(op, &srcs, dest.as_deref());
        done(aborted);
    });
}

/// Lo mismo, esperando a que termine (para el proceso con permisos de administrador).
/// Devuelve si salió bien.
pub fn file_op_blocking(op: FileOp, srcs: Vec<String>, dest: Option<String>) -> bool {
    let (ok, aborted) = run_file_op(op, &srcs, dest.as_deref());
    ok && !aborted
}

/// (salió bien, el usuario canceló)
fn run_file_op(op: FileOp, srcs: &[String], dest: Option<&str>) -> (bool, bool) {
    {
        let from = multi_sz(srcs);
        let to = dest.map(|d| multi_sz(&[d.to_string()]));
        let (func, flags) = match op {
            FileOp::Copy => (FO_COPY, FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0),
            FileOp::Move => (FO_MOVE, FOF_ALLOWUNDO.0 | FOF_NOCONFIRMMKDIR.0),
            FileOp::Recycle => (FO_DELETE, FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0),
            FileOp::DeleteForever => (FO_DELETE, FOF_NOERRORUI.0),
        };
        let mut s = SHFILEOPSTRUCTW {
            wFunc: func,
            pFrom: PCWSTR(from.as_ptr()),
            pTo: to.as_ref().map_or(PCWSTR::null(), |t| PCWSTR(t.as_ptr())),
            fFlags: flags as u16,
            ..Default::default()
        };
        let r = unsafe { SHFileOperationW(&mut s) };
        (r == 0, s.fAnyOperationsAborted.as_bool())
    }
}

pub fn rename(path: &str, new_name: &str) -> Result<String, String> {
    let parent = super::parent_of(path).ok_or(t!("shell.ruta_invalida"))?;
    let target = super::join(&parent, new_name);
    // Nunca pisar otro elemento (cambiar solo mayúsculas sí se permite).
    if !target.eq_ignore_ascii_case(path) && std::fs::symlink_metadata(super::long_path(&target)).is_ok() {
        return Err(tf!("comun.ya_existe", nombre = new_name));
    }
    std::fs::rename(super::long_path(path), super::long_path(&target)).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Saca de la papelera los elementos que estaban en estas rutas (el borrado más
/// reciente de cada una). Devuelve cuántos se restauraron.
pub fn restore_from_recycle_bin(paths: &[String]) -> usize {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::CoTaskMemFree;
    // System.Recycle.DeletedFrom y System.Recycle.DateDeleted; System.ItemNameDisplay.
    const FMTID_RECYCLE: windows::core::GUID = windows::core::GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871);
    const DELETED_FROM: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_RECYCLE, pid: 2 };
    const DATE_DELETED: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_RECYCLE, pid: 3 };
    const NAME: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 10 };
    com_init();
    let wanted: std::collections::HashSet<String> = paths.iter().map(|p| p.to_lowercase()).collect();
    let mut best: std::collections::HashMap<String, (u64, IShellItem2)> = Default::default();
    unsafe {
        let Ok(bin) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) else { return 0 };
        let Ok(items) = bin.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return 0 };
        loop {
            let mut got = [None];
            let mut n = 0u32;
            if items.Next(&mut got, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = got[0].take() else { break };
            let Ok(item2) = item.cast::<IShellItem2>() else { continue };
            let text = |key: &PROPERTYKEY| -> Option<String> {
                let p = item2.GetString(key).ok()?;
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            };
            let (Some(from), Some(name)) = (text(&DELETED_FROM), text(&NAME)) else { continue };
            let full = super::join(&from, &name).to_lowercase();
            if !wanted.contains(&full) {
                continue;
            }
            let when = item2.GetFileTime(&DATE_DELETED).map(|f| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64).unwrap_or(0);
            if best.get(&full).is_none_or(|(t, _)| when > *t) {
                best.insert(full, (when, item2));
            }
        }
        let mut restored = 0;
        for (_, (_, item)) in best {
            let Ok(menu) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else { continue };
            let verb = b"undelete\0";
            let info = CMINVOKECOMMANDINFO {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
                lpVerb: PCSTR(verb.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            if menu.InvokeCommand(&info).is_ok() {
                restored += 1;
            }
        }
        restored
    }
}

pub fn new_folder(dir: &str) -> Result<String, String> {
    for k in 1..1000 {
        let name = if k == 1 { t!("shell.nueva_carpeta").to_string() } else { tf!("shell.nueva_carpeta_n", k = k) };
        let p = super::join(dir, &name);
        if !std::path::Path::new(&p).exists() {
            std::fs::create_dir(&p).map_err(|e| e.to_string())?;
            return Ok(p);
        }
    }
    Err(t!("shell.no_pudo_crear_carpeta").into())
}

// ---------------------------------------------------------------- portapapeles

fn drop_effect_format() -> u32 {
    let w = wide("Preferred DropEffect");
    unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) }
}

fn global_from_bytes(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()).ok()?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Some(h)
    }
}

/// Pone archivos en el portapapeles (compatible con el Explorador de Windows).
pub fn clipboard_set_files(paths: &[String], cut: bool) -> bool {
    let header = DROPFILES {
        pFiles: std::mem::size_of::<DROPFILES>() as u32,
        fWide: true.into(),
        ..Default::default()
    };
    let mut bytes = unsafe {
        std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<DROPFILES>()).to_vec()
    };
    for u in multi_sz(paths) {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let effect: u32 = if cut { 2 } else { 5 };
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        if let Some(h) = global_from_bytes(&bytes) {
            let _ = SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(h.0)));
        }
        if let Some(h) = global_from_bytes(&effect.to_le_bytes()) {
            let _ = SetClipboardData(drop_effect_format(), Some(HANDLE(h.0)));
        }
        let _ = CloseClipboard();
    }
    true
}

/// Lee archivos del portapapeles. Devuelve (rutas, es_cortar).
pub fn clipboard_get_files() -> Option<(Vec<String>, bool)> {
    unsafe {
        OpenClipboard(None).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
            let hdrop = HDROP(h.0);
            let n = DragQueryFileW(hdrop, u32::MAX, None);
            let mut paths = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                paths.push(String::from_utf16_lossy(&buf[..len]));
            }
            let mut cut = false;
            if let Ok(e) = GetClipboardData(drop_effect_format()) {
                let p = GlobalLock(HGLOBAL(e.0)) as *const u32;
                if !p.is_null() {
                    cut = *p & 2 != 0;
                    let _ = GlobalUnlock(HGLOBAL(e.0));
                }
            }
            out = Some((paths, cut));
        }
        let _ = CloseClipboard();
        out.filter(|(p, _)| !p.is_empty())
    }
}

// ---------------------------------------------------------------- expulsar

pub fn eject(letter: char) -> Result<(), String> {
    // El verbo "eject" del shell hace lo mismo que el menú del Explorador.
    if exec("eject", &format!("{letter}:\\"), None, None) {
        return Ok(());
    }
    eject_ioctl(letter)
}

fn eject_ioctl(letter: char) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::*;
    let path = wide(&format!("\\\\.\\{letter}:"));
    let h = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(|e| e.message())?;
    let io = |code: u32, input: Option<&[u8]>| unsafe {
        let mut got = 0u32;
        DeviceIoControl(
            h,
            code,
            input.map(|i| i.as_ptr() as *const _),
            input.map_or(0, |i| i.len() as u32),
            None,
            0,
            Some(&mut got),
            None,
        )
    };
    let r = (|| {
        io(FSCTL_LOCK_VOLUME, None).map_err(|_| t!("shell.unidad_esta_uso").to_string())?;
        io(FSCTL_DISMOUNT_VOLUME, None).map_err(|e| e.message())?;
        let _ = io(IOCTL_STORAGE_MEDIA_REMOVAL, Some(&[0u8]));
        io(IOCTL_STORAGE_EJECT_MEDIA, None).map_err(|e| e.message())
    })();
    unsafe {
        let _ = CloseHandle(h);
    }
    r
}

// ---------------------------------------------------------------- menú contextual nativo

thread_local! {
    static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

unsafe extern "system" fn menu_subclass(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let handled = ACTIVE_MENU.with(|m| {
            let m = m.borrow();
            let cm = m.as_ref()?;
            if let Ok(cm3) = cm.cast::<IContextMenu3>() {
                let mut res = LRESULT(0);
                unsafe { cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)) }.ok()?;
                return Some(res);
            }
            if let Ok(cm2) = cm.cast::<IContextMenu2>() {
                unsafe { cm2.HandleMenuMsg(msg, wp, lp) }.ok()?;
                return Some(LRESULT(0));
            }
            None
        });
        if let Some(r) = handled {
            return r;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// Resultado de un menú nativo cuando el verbo lo debe manejar la app.
pub enum MenuOutcome {
    None,
    Invoked,
    Rename,
}

/// Muestra el menú contextual del Explorador para estos archivos (misma carpeta).
pub fn native_context_menu(owner: HWND, paths: &[String], screen: (i32, i32), extended: bool) -> MenuOutcome {
    if paths.is_empty() {
        return MenuOutcome::None;
    }
    com_init();
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    let result = (|| -> windows::core::Result<MenuOutcome> {
        for p in paths {
            let w = wide(p);
            let mut pidl = std::ptr::null_mut();
            unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None)? };
            pidls.push(pidl);
        }
        let folder: IShellFolder = unsafe { SHBindToParent(pidls[0], None)? };
        let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|&p| unsafe { ILFindLastID(p) as *const _ }).collect();
        let cm: IContextMenu = unsafe { folder.GetUIObjectOf(owner, &children, None)? };
        let menu = unsafe { CreatePopupMenu()? };
        let mut flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
        if extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        unsafe { cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags).ok()? };
        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(cm.clone()));
        unsafe {
            let _ = SetWindowSubclass(owner, Some(menu_subclass), 0x5241_594F, 0);
        }
        let cmd = unsafe { TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, screen.0, screen.1, owner, None) }.0;
        unsafe {
            let _ = RemoveWindowSubclass(owner, Some(menu_subclass), 0x5241_594F);
        }
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
        let mut outcome = MenuOutcome::None;
        if cmd > 0 {
            let id = (cmd - 1) as usize;
            let mut verb = [0u8; 64];
            let verb_str = unsafe { cm.GetCommandString(id, GCS_VERBA, None, PSTR(verb.as_mut_ptr()), verb.len() as u32) }
                .ok()
                .map(|_| String::from_utf8_lossy(&verb[..verb.iter().position(|&b| b == 0).unwrap_or(0)]).to_string())
                .unwrap_or_default();
            if verb_str.eq_ignore_ascii_case("rename") {
                outcome = MenuOutcome::Rename;
            } else {
                let info = CMINVOKECOMMANDINFOEX {
                    cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    fMask: 0x0000_4000 | CMIC_MASK_PTINVOKE, // CMIC_MASK_UNICODE
                    hwnd: owner,
                    lpVerb: PCSTR(id as *const u8),
                    lpVerbW: PCWSTR(id as *const u16),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: POINT { x: screen.0, y: screen.1 },
                    ..Default::default()
                };
                unsafe { cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO)? };
                outcome = MenuOutcome::Invoked;
            }
        }
        unsafe {
            let _ = DestroyMenu(menu);
        }
        Ok(outcome)
    })();
    for p in pidls {
        unsafe { ILFree(Some(p)) };
    }
    result.unwrap_or(MenuOutcome::None)
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Arrastra archivos hacia otras aplicaciones (Explorador, navegador, Discord…) con
/// el arrastre nativo de Windows, imagen incluida. Bloquea hasta que se suelta.
/// La aplicación de destino hace la copia o el movimiento.
pub fn drag_out(hwnd: HWND, paths: &[String]) -> Option<windows::Win32::System::Ole::DROPEFFECT> {
    use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
    com_init();
    unsafe {
        let mut pidls: Vec<*const ITEMIDLIST> = Vec::new();
        for p in paths {
            let w = wide(p);
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        let result = (|| {
            if pidls.is_empty() {
                return None;
            }
            let arr = SHCreateShellItemArrayFromIDLists(&pidls).ok()?;
            let data: IDataObject = arr.BindToHandler(None, &BHID_DataObject).ok()?;
            SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK).ok()
        })();
        for p in pidls {
            CoTaskMemFree(Some(p as *const _));
        }
        result
    }
}

/// ¿Está presionada esta tecla ahora mismo? (sirve también durante arrastres de Windows).
pub fn key_down(vk: u16) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}
