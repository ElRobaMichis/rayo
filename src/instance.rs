//! Una sola instancia de Rayo: si ya hay una abierta, la segunda le pasa su
//! ruta (si la tiene) por WM_COPYDATA, le cede el primer plano y termina.
//!
//! Al cerrar la ventana, Rayo puede quedarse en segundo plano (como el
//! Explorador, que siempre está corriendo): volver a abrirlo es instantáneo.
//! Mientras está oculto muestra un ícono en la bandeja para abrirlo o salir.
//!
//! Con `RAYO_DATA_DIR=<carpeta>`, Rayo guarda su configuración y su índice ahí y corre como
//! una instancia aparte: sirve para probar cambios (o sacar capturas) sin tocar el Rayo de
//! todos los días.

use crossbeam_channel::{Receiver, Sender, unbounded};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// Carpeta de datos propia (`RAYO_DATA_DIR`), si se pidió.
pub fn custom_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("RAYO_DATA_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// Nombre de la ventana oculta y del mutex de la instancia (distinto con `RAYO_DATA_DIR`).
fn instance_name(prefix: &str) -> &'static [u16] {
    static NAMES: OnceLock<std::sync::Mutex<Vec<(String, &'static [u16])>>> = OnceLock::new();
    let names = NAMES.get_or_init(Default::default);
    let mut names = names.lock().unwrap();
    if let Some((_, n)) = names.iter().find(|(p, _)| p == prefix) {
        return n;
    }
    let mut name = format!("{prefix}RayoInstanciaUnica");
    if let Some(dir) = custom_data_dir() {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        dir.to_string_lossy().to_lowercase().hash(&mut h);
        name.push_str(&format!("-{:x}", h.finish()));
    }
    let wide: &'static [u16] = Box::leak(name.encode_utf16().chain([0]).collect::<Vec<u16>>().into_boxed_slice());
    names.push((prefix.to_string(), wide));
    wide
}

fn class() -> PCWSTR {
    PCWSTR(instance_name("").as_ptr())
}
const MSG_ID: usize = 0x5241_594F;
/// Clic en el ícono de la bandeja.
const WM_TRAY: u32 = WM_APP + 7;
/// Otra instancia pide cerrar Rayo del todo (`rayo.exe --quit`).
const WM_QUIT_REQ: u32 = WM_APP + 8;
/// Mostrar (1) o quitar (0) el ícono de la bandeja.
const WM_TRAY_SET: u32 = WM_APP + 9;

/// Mensajes de otras instancias: la ruta a abrir (vacía = solo enfocar).
static INBOX: OnceLock<(Sender<String>, Receiver<String>)> = OnceLock::new();
static MAIN_HWND: OnceLock<usize> = OnceLock::new();
static MSG_HWND: OnceLock<usize> = OnceLock::new();
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// Se pidió salir de verdad (no quedarse en segundo plano).
static QUIT: AtomicBool = AtomicBool::new(false);
/// Arrancó oculto (`--background`) y todavía no se mostró.
pub static START_HIDDEN: AtomicBool = AtomicBool::new(false);
/// La ventana volvió a mostrarse desde segundo plano (la app recarga lo visible).
pub static SHOWN_AGAIN: AtomicBool = AtomicBool::new(false);

fn inbox() -> &'static (Sender<String>, Receiver<String>) {
    INBOX.get_or_init(unbounded)
}

fn find_running() -> Option<HWND> {
    unsafe { FindWindowW(class(), PCWSTR::null()).ok().filter(|h| !h.is_invalid()) }
}

/// Llamar al inicio. Devuelve `false` si ya había otra instancia (y ya se le
/// avisó), en cuyo caso este proceso debe terminar.
pub fn claim(path_arg: Option<&str>) -> bool {
    unsafe {
        // El handle del mutex no se cierra nunca: vive mientras viva el proceso.
        let _mutex = CreateMutexW(None, false, PCWSTR(instance_name("Local\\").as_ptr()));
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        if !exists {
            return true;
        }
        // La otra instancia puede estar arrancando: esperar a que cree su ventana.
        for _ in 0..40 {
            if let Some(target) = find_running() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(target, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let data: Vec<u16> = path_arg.unwrap_or("").encode_utf16().collect();
                let cds = COPYDATASTRUCT { dwData: MSG_ID, cbData: (data.len() * 2) as u32, lpData: data.as_ptr() as *mut _ };
                SendMessageW(target, WM_COPYDATA, None, Some(LPARAM(&cds as *const _ as isize)));
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // No respondió: mejor abrir otra ventana que no abrir nada.
        true
    }
}

/// ¿Hay un Rayo abierto (visible o en segundo plano)?
pub fn is_running() -> bool {
    find_running().is_some()
}

/// `rayo.exe --quit`: cierra del todo el Rayo que esté abierto (también si está en segundo plano).
pub fn quit_running() -> bool {
    match find_running() {
        Some(h) => unsafe { SendMessageW(h, WM_QUIT_REQ, None, None).0 == 1 },
        None => false,
    }
}

/// ¿Se pidió salir de verdad?
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}

/// Salir de verdad: la ventana se cierra en lugar de quedarse en segundo plano.
pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
    if let Some(&h) = MAIN_HWND.get() {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Muestra u oculta el ícono de la bandeja (se puede llamar desde cualquier hilo).
pub fn set_tray(visible: bool) {
    if let Some(&h) = MSG_HWND.get() {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_TRAY_SET, WPARAM(visible as usize), LPARAM(0));
        }
    }
}

fn tray(hwnd: HWND, add: bool) {
    let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
    unsafe {
        if !add {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            return;
        }
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        // El mismo ícono de la ventana.
        let icon = MAIN_HWND
            .get()
            .map(|&m| SendMessageW(HWND(m as *mut _), WM_GETICON, Some(WPARAM(ICON_SMALL as usize)), None).0)
            .filter(|&i| i != 0)
            .map(|i| HICON(i as *mut _));
        nid.hIcon = match icon {
            Some(i) => i,
            None => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        };
        let tip: Vec<u16> = t!("fondo.rayo_segundo_plano").encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let open: Vec<u16> = t!("fondo.abrir_rayo").encode_utf16().chain([0]).collect();
        let quit: Vec<u16> = crate::ui::commands::get("quit").label().encode_utf16().chain([0]).collect();
        let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR(quit.as_ptr()));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Necesario para que el menú se cierre al hacer clic fuera.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, Some(0), hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 {
            1 => show_main(),
            2 => request_quit(),
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA => {
            let cds = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
            if cds.dwData == MSG_ID {
                let units = unsafe { std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2) };
                let _ = inbox().0.send(String::from_utf16_lossy(units));
                show_main();
                return LRESULT(1);
            }
        }
        WM_QUIT_REQ => {
            request_quit();
            return LRESULT(1);
        }
        WM_TRAY_SET => {
            tray(hwnd, wp.0 != 0);
            return LRESULT(0);
        }
        WM_TRAY => {
            match lp.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_main(),
                WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Muestra la ventana principal (también si estaba en segundo plano) y la trae al frente.
fn show_main() {
    if let Some(&h) = MAIN_HWND.get() {
        let h = HWND(h as *mut _);
        START_HIDDEN.store(false, Ordering::Relaxed);
        unsafe {
            if !IsWindowVisible(h).as_bool() {
                SHOWN_AGAIN.store(true, Ordering::Relaxed);
                let _ = ShowWindowAsync(h, SW_SHOW);
            } else if IsIconic(h).as_bool() {
                let _ = ShowWindowAsync(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
        if let Some(&m) = MSG_HWND.get() {
            tray(HWND(m as *mut _), false);
        }
    }
    if let Some(w) = WAKE.get() {
        w();
    }
}

/// Escucha a otras instancias (en un hilo propio). `main` es la ventana principal.
pub fn listen(main: HWND, wake: impl Fn() + Send + Sync + 'static) {
    let _ = MAIN_HWND.set(main.0 as usize);
    let _ = WAKE.set(Box::new(wake));
    std::thread::Builder::new()
        .name("instancia".into())
        .spawn(|| unsafe {
            let Ok(inst) = GetModuleHandleW(None) else { return };
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class(), ..Default::default() };
            RegisterClassW(&wc);
            // Ventana oculta (no "solo mensajes"): el menú de la bandeja necesita una ventana que pueda tener el foco.
            let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW, class(), w!("Rayo"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) else {
                return;
            };
            let _ = MSG_HWND.set(hwnd.0 as usize);
            if START_HIDDEN.load(Ordering::Relaxed) {
                tray(hwnd, true);
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .ok();
}

/// Quita el ícono de la bandeja al salir.
pub fn shutdown() {
    crate::integration::release();
    if let Some(&m) = MSG_HWND.get() {
        tray(HWND(m as *mut _), false);
    }
}

/// Abre algo en Rayo desde otro hilo (el vigía del Explorador) y muestra la ventana.
/// Ruta vacía: solo mostrar Rayo.
pub fn deliver(path: &str) {
    let _ = inbox().0.send(path.to_string());
    show_main();
}

/// Rutas que pidieron abrir otras instancias desde el último cuadro.
pub fn take_requests() -> Vec<String> {
    inbox().1.try_iter().collect()
}

/// Ruta pasada por línea de comandos (si existe).
pub fn path_from_args() -> Option<String> {
    std::env::args().skip(1).find(|a| !a.starts_with("--")).map(|a| a.trim_matches('"').to_string()).filter(|p| std::path::Path::new(p).exists())
}

// ---------------------------------------------------------------- inicio con Windows

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Agrega (o quita) Rayo del inicio de sesión del usuario, arrancando oculto.
pub fn set_autostart(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
    unsafe {
        if on {
            let exe = crate::install::app_exe();
            let cmd: Vec<u16> = format!("\"{}\" --background", exe.display()).encode_utf16().chain([0]).collect();
            RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Rayo"), REG_SZ.0, Some(cmd.as_ptr() as *const _), (cmd.len() * 2) as u32).ok().map_err(|e| e.message())
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Rayo"));
            Ok(())
        }
    }
}
