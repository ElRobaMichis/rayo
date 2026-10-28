//! Rayo como explorador predeterminado, en dos partes:
//!
//! 1. **Registro (solo del usuario, sin administrador).** Abrir una carpeta, una
//!    unidad o un .zip (si los abre el visor de Windows) desde cualquier programa abre Rayo. Es lo mismo que hacen
//!    Files o Directory Opus. Antes de tocar nada se guarda lo que había, y apagarlo
//!    lo deja exactamente igual; además queda un `.reg` para deshacerlo a mano.
//! 2. **Vigía.** Lo que no pasa por el registro (Win+E en Windows 11, "Mostrar en
//!    carpeta" del navegador, "Abrir ubicación del archivo", `explorer /select`) abre una ventana del
//!    Explorador: mientras Rayo está abierto, esa ventana se saca de la pantalla (y de
//!    la barra de tareas) en cuanto se crea, antes de dibujarse; se lee qué carpeta y
//!    qué archivo mostraba, se cierra y se abre lo mismo en Rayo. No llega a verse. Solo carpetas reales e Inicio / Este equipo; el Panel
//!    de control, la Papelera o la Red siguen en el Explorador. Con Mayús apretada
//!    se respeta el Explorador.

use crate::index::walk::wide;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

// ================================================================ registro

/// Dónde se escribe (en las pruebas, una rama aparte del registro).
pub struct Roots {
    pub classes: &'static str,
    pub backup: &'static str,
}

pub const REAL: Roots = Roots { classes: "Software\\Classes", backup: "Software\\Rayo\\Explorador" };

const VERB: &str = "rayo";
/// Clases cuyo "abrir" pasa a Rayo: carpetas, unidades y los .zip del visor de Windows.
const CLASSES: [&str; 3] = ["Directory", "Drive", "CompressedFolder"];
/// "Este equipo" / Inicio: lo que abre Win+E.
const WINE_CLSID: &str = "CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";

fn key_exists(sub: &str) -> bool {
    let mut h = HKEY::default();
    let w = wide(sub);
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr()), None, KEY_READ, &mut h) };
    if r.is_ok() {
        unsafe {
            let _ = RegCloseKey(h);
        }
        true
    } else {
        false
    }
}

/// Valor de texto (`name` vacío = el valor predeterminado de la clave).
fn get_str(sub: &str, name: &str) -> Option<String> {
    let (s, n) = (wide(sub), wide(name));
    let mut buf = vec![0u16; 2048];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) }, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let units = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..units]))
}

fn set_str(sub: &str, name: &str, value: &str) -> Result<(), String> {
    let (s, n, v) = (wide(sub), wide(name), wide(value));
    let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) }, REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32) };
    r.ok().map_err(|e| tf!("predet.no_pudo_escribir", clave = sub, error = e.message()))
}

fn delete_value(sub: &str, name: &str) {
    let (s, n) = (wide(sub), wide(name));
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) });
    }
}

/// Borra una clave con todo lo de adentro. Solo en HKEY_CURRENT_USER y solo claves que creó Rayo.
fn delete_tree(sub: &str) {
    let s = wide(sub);
    unsafe {
        let r = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()));
        if r.is_ok() || r == ERROR_FILE_NOT_FOUND {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()));
        }
    }
}

/// Borra la clave solo si quedó vacía (sin subclaves ni valores).
fn delete_if_empty(sub: &str) {
    let s = wide(sub);
    let mut h = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), None, KEY_READ, &mut h).is_err() {
            return;
        }
        let (mut keys, mut values) = (0u32, 0u32);
        let ok = RegQueryInfoKeyW(h, None, None, None, Some(&mut keys), None, None, Some(&mut values), None, None, None, None).is_ok();
        let _ = RegCloseKey(h);
        if ok && keys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()));
        }
    }
}

/// El Rayo al que apunta el registro: el instalado (si existe) o este.
fn exe() -> Result<String, String> {
    Ok(crate::install::app_exe().display().to_string())
}

fn notify_shell() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// ¿Rayo es hoy el predeterminado (según el registro)?
pub fn is_default(r: &Roots) -> bool {
    get_str(&format!("{}\\Directory\\shell", r.classes), "").as_deref() == Some(VERB) && key_exists(&format!("{}\\Directory\\shell\\{VERB}\\command", r.classes))
}

/// Hace de Rayo el explorador predeterminado. Guarda antes lo que había.
pub fn enable(r: &Roots) -> Result<(), String> {
    let exe = exe()?;
    let c = r.classes;
    // Respaldo (una sola vez: si ya estaba activo, se conserva el primero).
    if get_str(r.backup, "Activo").is_none() {
        for name in CLASSES {
            backup_default(r, name)?;
        }
        set_str(r.backup, "ClsidExistia", if key_exists(&format!("{c}\\{WINE_CLSID}")) { "1" } else { "0" })?;
        set_str(r.backup, "Activo", "1")?;
    }
    let cmd_dir = format!("\"{exe}\" \"%1\"");
    for class in CLASSES {
        // Una clase que se agregó en una versión nueva de Rayo: respaldar lo que tenía.
        if get_str(r.backup, &format!("{class}Respaldado")).is_none() {
            backup_default(r, class)?;
        }
        let shell = format!("{c}\\{class}\\shell");
        set_str(&format!("{shell}\\{VERB}"), "", t!("predet.abrir_rayo"))?;
        set_str(&format!("{shell}\\{VERB}\\command"), "", &cmd_dir)?;
        set_str(&shell, "", VERB)?;
    }
    // Win+E.
    let wine = format!("{c}\\{WINE_CLSID}\\shell\\opennewwindow\\command");
    set_str(&wine, "", &format!("\"{exe}\""))?;
    set_str(&wine, "DelegateExecute", "")?;
    notify_shell();
    write_undo_file(r);
    Ok(())
}

/// Guarda el "abrir" que tenía una clase antes de pasarlo a Rayo.
fn backup_default(r: &Roots, class: &str) -> Result<(), String> {
    match get_str(&format!("{}\\{class}\\shell", r.classes), "") {
        Some(v) if v != VERB => set_str(r.backup, &format!("{class}Antes"), &v)?,
        Some(_) => {}
        None => delete_value(r.backup, &format!("{class}Antes")),
    }
    set_str(r.backup, &format!("{class}Respaldado"), "1")
}

/// Deja todo como estaba antes de `enable`.
pub fn disable(r: &Roots) -> Result<(), String> {
    let c = r.classes;
    for class in CLASSES {
        let shell = format!("{c}\\{class}\\shell");
        delete_tree(&format!("{shell}\\{VERB}"));
        match get_str(r.backup, &format!("{class}Antes")) {
            Some(v) => set_str(&shell, "", &v)?,
            None => {
                if get_str(&shell, "").as_deref() == Some(VERB) {
                    delete_value(&shell, "");
                }
            }
        }
        delete_if_empty(&shell);
        delete_if_empty(&format!("{c}\\{class}"));
    }
    if get_str(r.backup, "ClsidExistia").as_deref() == Some("1") {
        delete_tree(&format!("{c}\\{WINE_CLSID}\\shell\\opennewwindow"));
        delete_if_empty(&format!("{c}\\{WINE_CLSID}\\shell"));
    } else {
        delete_tree(&format!("{c}\\{WINE_CLSID}"));
    }
    delete_tree(r.backup);
    notify_shell();
    Ok(())
}

/// Si Rayo cambió de lugar (otra compilación, se movió la carpeta), actualiza la ruta.
pub fn refresh(r: &Roots) {
    if !is_default(r) {
        return;
    }
    let Ok(exe) = exe() else { return };
    let want = format!("\"{exe}\" \"%1\"");
    // También si a esta versión le faltan clases (p. ej. los .zip, que se sumaron después).
    let stale = CLASSES.iter().any(|class| get_str(&format!("{}\\{class}\\shell\\{VERB}\\command", r.classes), "").as_deref() != Some(want.as_str()));
    if stale {
        let _ = enable(r);
    }
}

/// Archivo para deshacerlo a mano (doble clic) aunque Rayo ya no esté.
pub fn undo_file() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Rayo").join("restaurar-explorador.reg")
}

fn write_undo_file(r: &Roots) {
    if r.classes != REAL.classes {
        return;
    }
    let c = format!("HKEY_CURRENT_USER\\{}", r.classes);
    let mut s = String::from("Windows Registry Editor Version 5.00\r\n\r\n; Vuelve a dejar el Explorador de Windows como predeterminado.\r\n\r\n");
    for class in CLASSES {
        s.push_str(&format!("[-{c}\\{class}\\shell\\{VERB}]\r\n\r\n[{c}\\{class}\\shell]\r\n"));
        match get_str(r.backup, &format!("{class}Antes")) {
            Some(v) => s.push_str(&format!("@=\"{}\"\r\n\r\n", v.replace('\\', "\\\\").replace('"', "\\\""))),
            None => s.push_str("@=-\r\n\r\n"),
        }
    }
    if get_str(r.backup, "ClsidExistia").as_deref() == Some("1") {
        s.push_str(&format!("[-{c}\\{WINE_CLSID}\\shell\\opennewwindow]\r\n\r\n"));
    } else {
        s.push_str(&format!("[-{c}\\{WINE_CLSID}]\r\n\r\n"));
    }
    s.push_str(&format!("[-HKEY_CURRENT_USER\\{}]\r\n", r.backup));
    // Los .reg van en UTF-16 con BOM.
    let mut bytes = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let p = undo_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, bytes);
}

// ================================================================ vigía

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging as wm;

static WATCHING: AtomicBool = AtomicBool::new(false);
static STARTED: OnceLock<()> = OnceLock::new();
/// Hasta cuándo (ms desde el arranque) se deja tranquilo al Explorador: Rayo mismo lo abrió.
static ALLOW_UNTIL: AtomicU64 = AtomicU64::new(0);
const WM_EXPLORER_CREATED: u32 = wm::WM_APP + 40;
const WM_EXPLORER_SHOWN: u32 = wm::WM_APP + 41;
const WM_EXPLORER_GONE: u32 = wm::WM_APP + 42;

/// Diagnóstico del vigía (solo con RAYO_PRUEBA_VIGIA): %TEMP%/rayo-vigia.log.
fn trace(msg: &str) {
    if std::env::var_os("RAYO_PRUEBA_VIGIA").is_none() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("rayo-vigia.log")) {
        let _ = writeln!(f, "{:>7} {msg}", now_ms());
    }
}

fn now_ms() -> u64 {
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

fn explorer_allowed() -> bool {
    now_ms() < ALLOW_UNTIL.load(Ordering::Relaxed) || crate::fsx::shell::key_down(0x10)
}

/// Rayo va a abrir el Explorador a propósito ("Mostrar en el Explorador"): no interceptarlo.
pub fn allow_explorer() {
    ALLOW_UNTIL.store(now_ms() + 4000, Ordering::Relaxed);
}

/// Prende o apaga el vigía (el hilo se crea la primera vez).
pub fn watch(on: bool) {
    WATCHING.store(on, Ordering::Relaxed);
    if on && STARTED.set(()).is_ok() {
        let _ = std::thread::Builder::new().name("vigia-explorador".into()).spawn(watch_thread);
    }
}

fn is_cabinet(hwnd: HWND) -> bool {
    let mut cls = [0u16; 32];
    let n = unsafe { wm::GetClassNameW(hwnd, &mut cls) } as usize;
    String::from_utf16_lossy(&cls[..n]) == "CabinetWClass"
}

fn watch_thread() {
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
    crate::fsx::shell::com_init();
    unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, id_child: i32, _: u32, _: u32) {
        // Solo ventanas (no sus partes) de la clase de las carpetas del Explorador.
        if id_object != 0 || id_child != 0 || hwnd.is_invalid() || !WATCHING.load(Ordering::Relaxed) || !is_cabinet(hwnd) {
            return;
        }
        let msg = match event {
            0x8000 => WM_EXPLORER_CREATED,
            0x8001 => WM_EXPLORER_GONE,
            _ => WM_EXPLORER_SHOWN,
        };
        unsafe {
            let _ = wm::PostThreadMessageW(windows::Win32::System::Threading::GetCurrentThreadId(), msg, WPARAM(hwnd.0 as usize), LPARAM(0));
        }
    }
    unsafe {
        // EVENT_OBJECT_CREATE, DESTROY y SHOW; fuera de proceso y sin las ventanas de Rayo.
        let hook = SetWinEventHook(0x8000, 0x8002, None, Some(on_event), 0, 0, 0x0000 | 0x0002);
        trace(&format!("vigía iniciado (gancho válido: {})", !hook.is_invalid()));
        if hook.is_invalid() {
            return;
        }
        let mut msg = wm::MSG::default();
        while wm::GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let hwnd = HWND(msg.wParam.0 as *mut _);
            match msg.message {
                WM_EXPLORER_CREATED => {
                    trace(&format!("ventana del Explorador creada: {:?}", msg.wParam.0));
                    if WATCHING.load(Ordering::Relaxed) && !explorer_allowed() && !left_alone(hwnd) {
                        park(hwnd);
                    }
                }
                WM_EXPLORER_SHOWN => {
                    trace(&format!("ventana del Explorador mostrada: {:?}", msg.wParam.0));
                    take_over(hwnd);
                }
                WM_EXPLORER_GONE => forget(hwnd),
                _ => {
                    let _ = wm::TranslateMessage(&msg);
                    wm::DispatchMessageW(&msg);
                }
            }
        }
    }
}

// ---------------------------------------------------------------- ventanas del Explorador

/// La ventana del Explorador que el vigía tiene oculta en este momento (0 = ninguna):
/// si Rayo se cierra justo entonces, `release` la vuelve a mostrar.
static HIDDEN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Una ventana sacada de la pantalla mientras se decide qué hacer.
#[derive(Clone, Copy)]
struct Parked {
    hwnd: isize,
    /// Dónde estaba.
    rect: RECT,
    /// Su estilo extendido original, si se le quitó el botón de la barra de tareas.
    ex_style: Option<isize>,
}

static PARKED: std::sync::Mutex<Vec<Parked>> = std::sync::Mutex::new(Vec::new());

/// Ventanas ya revisadas que se dejan al Explorador: (ventana, hasta cuándo no volver a mirarla).
/// Así mostrarla de nuevo no la vuelve a revisar (y no se arma un bucle).
static LEFT_ALONE: std::sync::Mutex<Vec<(isize, u64)>> = std::sync::Mutex::new(Vec::new());

fn left_alone(hwnd: HWND) -> bool {
    let now = now_ms();
    let mut v = LEFT_ALONE.lock().unwrap_or_else(|e| e.into_inner());
    v.retain(|&(_, until)| until > now);
    v.iter().any(|&(h, _)| h == hwnd.0 as isize)
}

fn leave_alone(hwnd: HWND) {
    let mut v = LEFT_ALONE.lock().unwrap_or_else(|e| e.into_inner());
    v.push((hwnd.0 as isize, now_ms() + 3000));
}

/// La ventana se cerró: olvidarla.
fn forget(hwnd: HWND) {
    PARKED.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| p.hwnd != hwnd.0 as isize);
    LEFT_ALONE.lock().unwrap_or_else(|e| e.into_inner()).retain(|&(h, _)| h != hwnd.0 as isize);
}

const PARK_AT: i32 = -32000;

/// Saca la ventana de la pantalla (sin ocultarla ni cambiarle el tamaño) y recuerda dónde
/// estaba. Si todavía no se mostró, además se le quita el botón de la barra de tareas.
/// Así, si es para Rayo, nunca llega a verse.
fn park(hwnd: HWND) {
    unsafe {
        if wm::IsZoomed(hwnd).as_bool() {
            return;
        }
        let mut r = RECT::default();
        if wm::GetWindowRect(hwnd, &mut r).is_err() {
            return;
        }
        let mut v = PARKED.lock().unwrap_or_else(|e| e.into_inner());
        let known = v.iter().position(|p| p.hwnd == hwnd.0 as isize);
        if r.left > PARK_AT + 1000 {
            // El Explorador la puso en su lugar después de crearla: ese es el lugar a devolver.
            match known {
                Some(i) => v[i].rect = r,
                None => {
                    // Sin botón en la barra de tareas (solo antes de mostrarse: después, la
                    // barra no se enteraría del cambio).
                    let ex_style = (!wm::IsWindowVisible(hwnd).as_bool()).then(|| {
                        let ex = wm::GetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE);
                        wm::SetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE, (ex | wm::WS_EX_TOOLWINDOW.0 as isize) & !(wm::WS_EX_APPWINDOW.0 as isize));
                        ex
                    });
                    v.push(Parked { hwnd: hwnd.0 as isize, rect: r, ex_style });
                }
            }
        } else if known.is_none() {
            return;
        }
        drop(v);
        let _ = wm::SetWindowPos(hwnd, None, PARK_AT, PARK_AT, 0, 0, wm::SWP_NOSIZE | wm::SWP_NOZORDER | wm::SWP_NOACTIVATE);
    }
}

/// Devuelve la ventana a su lugar y con su botón en la barra de tareas (si se le quitó).
fn unpark(hwnd: HWND) {
    let p = {
        let mut v = PARKED.lock().unwrap_or_else(|e| e.into_inner());
        match v.iter().position(|p| p.hwnd == hwnd.0 as isize) {
            Some(i) => v.remove(i),
            None => return,
        }
    };
    unsafe {
        let _ = wm::SetWindowPos(hwnd, None, p.rect.left, p.rect.top, 0, 0, wm::SWP_NOSIZE | wm::SWP_NOZORDER | wm::SWP_NOACTIVATE);
        if let Some(ex) = p.ex_style {
            // Para que la barra de tareas vuelva a mostrar su botón hay que ocultarla y mostrarla.
            let visible = wm::IsWindowVisible(hwnd).as_bool();
            if visible {
                let _ = wm::ShowWindow(hwnd, wm::SW_HIDE);
            }
            wm::SetWindowLongPtrW(hwnd, wm::GWL_EXSTYLE, ex);
            if visible {
                let _ = wm::ShowWindow(hwnd, wm::SW_SHOW);
            }
        }
    }
}

/// Al cerrar Rayo (o al apagar el vigía): toda ventana del Explorador que esté fuera de
/// la pantalla u oculta a medio revisar vuelve a su lugar.
pub fn release() {
    WATCHING.store(false, Ordering::Relaxed);
    let parked: Vec<isize> = PARKED.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|p| p.hwnd).collect();
    for h in parked {
        unpark(HWND(h as *mut _));
    }
    let h = HIDDEN.swap(0, Ordering::SeqCst);
    if h != 0 {
        show_again(HWND(h as *mut _));
    }
}

fn show_again(hwnd: HWND) {
    unsafe {
        let _ = wm::ShowWindow(hwnd, wm::SW_SHOW);
        let _ = wm::SetForegroundWindow(hwnd);
    }
}

/// Si al final la ventana no es para Rayo, vuelve a su lugar (y a verse).
struct Reshow(HWND, bool);
impl Drop for Reshow {
    fn drop(&mut self) {
        let was = HIDDEN.swap(0, Ordering::SeqCst);
        if self.1 {
            leave_alone(self.0);
            unpark(self.0);
            if was == self.0.0 as isize {
                show_again(self.0);
            }
        }
    }
}

/// Qué muestra una ventana del Explorador: (ubicación, primer elemento seleccionado).
/// La ubicación es una ruta ("C:\\…") o el nombre de una carpeta especial ("::{…}").
fn explorer_location(hwnd: HWND) -> Option<(String, Option<String>)> {
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider};
    use windows::Win32::UI::Shell::*;
    use windows::core::Interface;
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        for i in 0..sw.Count().ok()? {
            let Ok(disp) = sw.Item(&windows::Win32::System::Variant::VARIANT::from(i)) else { continue };
            let Ok(wb) = disp.cast::<IWebBrowser2>() else { continue };
            if wb.HWND().ok().map(|h| h.0 as isize) != Some(hwnd.0 as isize) {
                continue;
            }
            let sp: IServiceProvider = wb.cast().ok()?;
            let sb: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
            let view = sb.QueryActiveShellView().ok()?;
            let fv: IFolderView = view.cast().ok()?;
            let folder: IPersistFolder2 = fv.GetFolder().ok()?;
            let pidl = folder.GetCurFolder().ok()?;
            let name = SHGetNameFromIDList(pidl, SIGDN_DESKTOPABSOLUTEPARSING).ok();
            CoTaskMemFree(Some(pidl as *const _));
            let name = name?;
            let loc = name.to_string().unwrap_or_default();
            CoTaskMemFree(Some(name.0 as *const _));
            let sel = view.cast::<IFolderView2>().ok().and_then(|fv2| fv2.GetSelection(false).ok()).and_then(|arr| {
                if arr.GetCount().ok()? == 0 {
                    return None;
                }
                let item = arr.GetItemAt(0).ok()?;
                let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            });
            return Some((loc, sel));
        }
        None
    }
}

/// Inicio y Este equipo: se abre Rayo sin ruta.
fn is_home(loc: &str) -> bool {
    let l = loc.to_ascii_lowercase();
    l == "::{f874310e-b6b7-47dc-bc84-b9e6b38f5903}" || l == "::{20d04fe0-3aea-1069-a2d8-08002b30309d}"
}

/// Una carpeta del disco, o un comprimido (o una carpeta de adentro de uno), que Rayo abre
/// por dentro. El escritorio abre los .zip así, sin pasar por el registro.
fn is_real_folder(loc: &str) -> bool {
    if loc.as_bytes().get(1) != Some(&b':') {
        return false;
    }
    std::path::Path::new(loc).is_dir() || crate::fsx::archive::split(loc).is_some_and(|(a, _)| std::path::Path::new(&a).is_file())
}

fn take_over(hwnd: HWND) {
    if !WATCHING.load(Ordering::Relaxed) || left_alone(hwnd) || explorer_allowed() {
        unpark(hwnd);
        return;
    }
    if !unsafe { wm::IsWindowVisible(hwnd) }.as_bool() {
        return;
    }
    // Fuera de la pantalla ya mismo (si no se alcanzó a hacer al crearse); maximizada no
    // se puede mover, así que se oculta.
    let mut r = RECT::default();
    let _ = unsafe { wm::GetWindowRect(hwnd, &mut r) };
    trace(&format!("  al mostrarse estaba en x={} ({})", r.left, if r.left <= PARK_AT + 1000 { "fuera de la pantalla" } else { "A LA VISTA" }));
    park(hwnd);
    let mut guard = Reshow(hwnd, true);
    if unsafe { wm::IsZoomed(hwnd) }.as_bool() {
        HIDDEN.store(hwnd.0 as isize, Ordering::SeqCst);
        unsafe {
            let _ = wm::ShowWindow(hwnd, wm::SW_HIDE);
        }
    }
    // La ventana tarda un momento en llegar a su carpeta (y en marcar el archivo).
    let start = std::time::Instant::now();
    let mut found: Option<(String, Option<String>, std::time::Instant)> = None;
    while start.elapsed() < std::time::Duration::from_millis(2500) {
        let got = explorer_location(hwnd);
        trace(&format!("  ubicación: {got:?}"));
        if let Some((loc, sel)) = got {
            if !(is_home(&loc) || is_real_folder(&loc)) {
                // Panel de control, Papelera, Red…: es del Explorador (vuelve a su lugar).
                if !loc.is_empty() {
                    return;
                }
            } else {
                let first = found.as_ref().map_or(std::time::Instant::now(), |f| f.2);
                // Inicio y los comprimidos no traen archivo marcado: no hace falta esperarlo.
                let archive = !std::path::Path::new(&loc).is_dir() && !is_home(&loc);
                let done = sel.is_some() || is_home(&loc) || archive || first.elapsed() > std::time::Duration::from_millis(350);
                found = Some((loc, sel, first));
                if done {
                    break;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let Some((loc, sel, _)) = found else {
        trace("  sin ubicación: se devuelve al Explorador");
        return;
    };
    // Rayo se queda con esto. Se oculta, vuelve a su lugar (así el Explorador recuerda una
    // posición normal para la próxima vez) y se cierra; nada de esto se ve.
    guard.1 = false;
    unsafe {
        let _ = wm::ShowWindow(hwnd, wm::SW_HIDE);
    }
    unpark(hwnd);
    unsafe {
        let _ = wm::PostMessageW(Some(hwnd), wm::WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    let target = match sel {
        Some(s) if !is_home(&loc) => s,
        _ if is_home(&loc) => String::new(),
        _ => loc,
    };
    crate::instance::deliver(&target);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: Roots = Roots { classes: "Software\\Rayo-prueba\\Classes", backup: "Software\\Rayo-prueba\\Respaldo" };

    /// Activar y desactivar en una rama de prueba deja el registro como estaba,
    /// incluso con un valor previo que hay que devolver.
    #[test]
    fn enable_disable_roundtrip() {
        delete_tree("Software\\Rayo-prueba");
        set_str(&format!("{}\\Drive\\shell", TEST.classes), "", "previo").unwrap();
        assert!(!is_default(&TEST));
        enable(&TEST).unwrap();
        assert!(is_default(&TEST));
        let cmd = get_str(&format!("{}\\Directory\\shell\\rayo\\command", TEST.classes), "").unwrap();
        assert!(cmd.ends_with("\"%1\"") && cmd.contains(".exe"), "{cmd}");
        assert_eq!(get_str(&format!("{}\\Drive\\shell", TEST.classes), "").as_deref(), Some("rayo"));
        assert!(key_exists(&format!("{}\\{WINE_CLSID}\\shell\\opennewwindow\\command", TEST.classes)));
        assert_eq!(get_str(&format!("{}\\CompressedFolder\\shell", TEST.classes), "").as_deref(), Some("rayo"));
        // Activar dos veces no pisa el respaldo.
        enable(&TEST).unwrap();
        disable(&TEST).unwrap();
        assert!(!is_default(&TEST));
        assert_eq!(get_str(&format!("{}\\Drive\\shell", TEST.classes), "").as_deref(), Some("previo"));
        assert!(!key_exists(&format!("{}\\Directory", TEST.classes)));
        assert!(!key_exists(&format!("{}\\Drive\\shell\\rayo", TEST.classes)));
        assert!(!key_exists(&format!("{}\\CLSID", TEST.classes)) || !key_exists(&format!("{}\\{WINE_CLSID}", TEST.classes)));
        assert!(!key_exists(TEST.backup));
        assert!(!key_exists(&format!("{}\\CompressedFolder", TEST.classes)));
        delete_tree("Software\\Rayo-prueba");
    }
}
