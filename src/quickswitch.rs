//! Quick Switch (como en Listary): con un diálogo de Abrir o Guardar al frente, de cualquier
//! programa, Ctrl+G lo lleva a la carpeta que está abierta en Rayo. El nombre que ya se había
//! escrito en el diálogo se conserva.
//!
//! Reemplazar esos diálogos no se puede sin meterse dentro de cada programa; esto da lo mismo
//! en la práctica: lo que tienes abierto en Rayo, en el diálogo, con una tecla.
//!
//! Un gancho de teclado (WH_KEYBOARD_LL) mira Ctrl+G solo cuando la ventana del frente es un
//! diálogo de archivos (tiene el cuadro de nombre de archivo); en cualquier otra ventana la
//! tecla sigue de largo, sin cambios.

use crate::index::walk::wide;
use parking_lot::Mutex;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// La carpeta a la que hay que ir (la de la pestaña de Rayo); la actualiza la interfaz.
static TARGET: Mutex<String> = Mutex::new(String::new());

pub fn set_target(dir: &str) {
    let mut t = TARGET.lock();
    if *t != dir {
        *t = dir.to_string();
    }
}

/// Arranca el gancho (una vez).
pub fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new().name("quick-switch".into()).spawn(|| unsafe {
            let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) else { return };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        });
    });
}

fn down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn hook_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && (w.0 as u32 == WM_KEYDOWN) {
            let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
            let only_ctrl = down(VK_CONTROL.0) && !down(VK_MENU.0) && !down(VK_SHIFT.0) && !down(VK_LWIN.0) && !down(VK_RWIN.0);
            if k.vkCode == 0x47 && only_ctrl {
                let fg = GetForegroundWindow();
                if let Some(edit) = file_name_box(fg) {
                    let dir = TARGET.lock().clone();
                    if !dir.is_empty() {
                        // El trabajo, fuera del gancho (que tiene que volver enseguida).
                        let (dlg, edit) = (fg.0 as isize, edit.0 as isize);
                        std::thread::spawn(move || jump(HWND(dlg as *mut _), HWND(edit as *mut _), &dir));
                        return LRESULT(1);
                    }
                }
            }
        }
        CallNextHookEx(None, code, w, l)
    }
}

/// Si la ventana es un diálogo de Abrir / Guardar: su cuadro de nombre de archivo.
unsafe fn file_name_box(h: HWND) -> Option<HWND> {
    unsafe {
        if h.0.is_null() {
            return None;
        }
        let mut class = [0u16; 32];
        let n = GetClassNameW(h, &mut class);
        if String::from_utf16_lossy(&class[..n as usize]) != "#32770" {
            return None;
        }
        // Diálogo nuevo (y el clásico con lista): el combo 1148 con su cuadro de texto adentro.
        if let Ok(combo) = GetDlgItem(Some(h), 1148) {
            let combo_box = FindWindowExW(Some(combo), None, PCWSTR(wide("ComboBox").as_ptr()), PCWSTR::null()).unwrap_or(combo);
            if let Ok(edit) = FindWindowExW(Some(combo_box), None, PCWSTR(wide("Edit").as_ptr()), PCWSTR::null()) {
                return Some(edit);
            }
        }
        // Diálogo clásico sin combo: el cuadro 1152.
        GetDlgItem(Some(h), 1152).ok()
    }
}

/// Escribe la carpeta en el cuadro de nombre, "Aceptar" (el diálogo navega ahí) y devuelve el
/// nombre que había.
fn jump(dlg: HWND, edit: HWND, dir: &str) {
    unsafe {
        let mut buf = [0u16; 1024];
        let n = GetWindowTextW(edit, &mut buf);
        let before = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let path = wide(dir);
        let _ = SendMessageW(edit, WM_SETTEXT, Some(WPARAM(0)), Some(LPARAM(path.as_ptr() as isize)));
        // IDOK con un nombre que es una carpeta: el diálogo entra a ella (no abre ni guarda).
        let _ = SendMessageW(dlg, WM_COMMAND, Some(WPARAM(1)), Some(LPARAM(0)));
        std::thread::sleep(std::time::Duration::from_millis(120));
        let old = wide(&before);
        let _ = SendMessageW(edit, WM_SETTEXT, Some(WPARAM(0)), Some(LPARAM(old.as_ptr() as isize)));
    }
    crate::diag::log(&format!("quick switch: diálogo → «{dir}»"));
}
