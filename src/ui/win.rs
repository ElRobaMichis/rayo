//! Ganchos sobre la ventana principal de Windows.
//!
//! - Pinta el fondo con el color del tema (nunca un rectángulo blanco).
//! - Cuando Windows muestra, restaura o cambia de monitor la ventana, pide
//!   unos cuadros seguidos: el contenido de OpenGL no siempre llega a pantalla
//!   en el primer intento y, si la app ya estaba quieta, la ventana quedaría
//!   en blanco hasta mover el ratón.
//! - Al arrancar en segundo plano, evita que la ventana aparezca.
//! - Sin barra de título de Windows: las pestañas van arriba, en la misma fila que
//!   minimizar / maximizar / cerrar (como el Explorador de Windows 11). Se conserva
//!   todo lo nativo: bordes para redimensionar, sombra, esquinas redondeadas, Aero Snap,
//!   arrastrar desde el espacio libre de esa fila, doble clic para maximizar y el menú
//!   de Snap Layouts sobre el botón de maximizar.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, FillRect, HBRUSH, HDC, HGDIOBJ};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static BRUSH: AtomicIsize = AtomicIsize::new(0);
/// Hasta cuándo (ms desde el arranque) repintar seguido.
static BURST_UNTIL: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    crate::index::START.get().map_or(0, |s| s.elapsed().as_millis() as u64)
}

/// Repintar seguido durante `ms` milisegundos.
pub fn burst(ms: u64) {
    BURST_UNTIL.fetch_max(now_ms() + ms, Ordering::Relaxed);
    if let Some(w) = WAKE.get() {
        w();
    }
}

/// ¿Hay que seguir repintando? (la app pide el próximo cuadro si es así)
pub fn bursting() -> bool {
    now_ms() < BURST_UNTIL.load(Ordering::Relaxed)
}

/// Color de fondo de la ventana (se llama al cambiar de tema).
pub fn set_background(rgb: [u8; 3]) {
    let brush = unsafe { CreateSolidBrush(COLORREF(rgb[0] as u32 | (rgb[1] as u32) << 8 | (rgb[2] as u32) << 16)) };
    let old = BRUSH.swap(brush.0 as isize, Ordering::Relaxed);
    if old != 0 {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(old as *mut _));
        }
    }
}

// ---------------------------------------------------------------- barra de título propia

/// Zonas de la fila de arriba que hacen de barra de título (espacio libre, en píxeles de
/// la ventana) y el botón de maximizar. Las actualiza la app en cada cuadro.
static CAPTION: std::sync::Mutex<Vec<RECT>> = std::sync::Mutex::new(Vec::new());
static MAX_BUTTON: std::sync::Mutex<RECT> = std::sync::Mutex::new(RECT { left: 0, top: 0, right: 0, bottom: 0 });
static MAX_HOVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MAX_PRESSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTMAXBUTTON: isize = 9;
const HTTOP: isize = 12;
const HTTOPLEFT: isize = 13;
const HTTOPRIGHT: isize = 14;

/// La app dice qué partes de la fila de arriba son barra de título y dónde está el botón
/// de maximizar (en puntos de egui; `ppp` = píxeles por punto).
pub fn set_caption(rects: &[egui::Rect], max_button: egui::Rect, ppp: f32) {
    let px = |r: &egui::Rect| RECT { left: (r.left() * ppp) as i32, top: (r.top() * ppp) as i32, right: (r.right() * ppp) as i32, bottom: (r.bottom() * ppp) as i32 };
    *CAPTION.lock().unwrap_or_else(|e| e.into_inner()) = rects.iter().map(px).collect();
    *MAX_BUTTON.lock().unwrap_or_else(|e| e.into_inner()) = px(&max_button);
}

/// El ratón está sobre el botón de maximizar (egui no lo ve: para Windows es "su" botón).
pub fn max_hovered() -> bool {
    MAX_HOVER.load(Ordering::Relaxed)
}

fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Grosor del borde para redimensionar (el mismo que usa Windows para esta pantalla).
fn frame_thickness(hwnd: HWND) -> i32 {
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

fn wake() {
    if let Some(w) = WAKE.get() {
        w();
    }
}

fn toggle_maximized(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        // Sin barra de título: el área de la app llega hasta arriba. Los bordes de los
        // costados y de abajo quedan como los pone Windows (para redimensionar).
        WM_NCCALCSIZE if wp.0 != 0 => {
            let params = unsafe { &mut *(lp.0 as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            params.rgrc[0].top = top;
            // Maximizada, Windows deja la ventana un borde más allá de la pantalla.
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                params.rgrc[0].top += frame_thickness(hwnd);
            }
            return r;
        }
        WM_NCHITTEST => {
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            if r.0 != HTCLIENT {
                return r;
            }
            let mut pt = windows::Win32::Foundation::POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
            let mut rc = RECT::default();
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                let _ = GetClientRect(hwnd, &mut rc);
            }
            // El borde de arriba para redimensionar (ya no lo da la barra de título).
            if !unsafe { IsZoomed(hwnd) }.as_bool() {
                let b = frame_thickness(hwnd);
                if pt.y < b {
                    return LRESULT(if pt.x < b * 2 {
                        HTTOPLEFT
                    } else if pt.x >= rc.right - b * 2 {
                        HTTOPRIGHT
                    } else {
                        HTTOP
                    });
                }
            }
            if inside(&MAX_BUTTON.lock().unwrap_or_else(|e| e.into_inner()), pt.x, pt.y) {
                return LRESULT(HTMAXBUTTON);
            }
            if CAPTION.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|c| inside(c, pt.x, pt.y)) {
                return LRESULT(HTCAPTION);
            }
            return r;
        }
        // El botón de maximizar es "de Windows" (así aparece el menú de Snap Layouts):
        // su resaltado y su clic se manejan aquí.
        WM_NCMOUSEMOVE => {
            let over = wp.0 as isize == HTMAXBUTTON;
            if over != MAX_HOVER.swap(over, Ordering::Relaxed) {
                if over {
                    let mut tme = windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                        dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE | windows::Win32::UI::Input::KeyboardAndMouse::TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = windows::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut tme);
                    }
                }
                wake();
            }
            if !over {
                MAX_PRESSED.store(false, Ordering::Relaxed);
            }
        }
        0x02A2 /* WM_NCMOUSELEAVE */ => {
            if MAX_HOVER.swap(false, Ordering::Relaxed) {
                wake();
            }
            MAX_PRESSED.store(false, Ordering::Relaxed);
        }
        WM_MOUSEMOVE => {
            if MAX_HOVER.swap(false, Ordering::Relaxed) {
                wake();
            }
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wp.0 as isize == HTMAXBUTTON => {
            MAX_PRESSED.store(true, Ordering::Relaxed);
            return LRESULT(0);
        }
        WM_NCLBUTTONUP if wp.0 as isize == HTMAXBUTTON => {
            if MAX_PRESSED.swap(false, Ordering::Relaxed) {
                toggle_maximized(hwnd);
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => {
            let b = BRUSH.load(Ordering::Relaxed);
            if b != 0 {
                let mut rc = RECT::default();
                unsafe {
                    let _ = GetClientRect(hwnd, &mut rc);
                    FillRect(HDC(wp.0 as *mut _), &rc, HBRUSH(b as *mut _));
                }
                return LRESULT(1);
            }
        }
        WM_WINDOWPOSCHANGING => {
            if unsafe { &*(lp.0 as *const WINDOWPOS) }.flags.contains(SWP_SHOWWINDOW) {
                crate::index::mem_log("ventana: por mostrarse");
            }
            // Arranque en segundo plano: no dejar que la ventana aparezca hasta que se pida.
            if crate::instance::START_HIDDEN.load(Ordering::Relaxed) {
                let pos = unsafe { &mut *(lp.0 as *mut WINDOWPOS) };
                pos.flags &= !SWP_SHOWWINDOW;
            }
        }
        WM_WINDOWPOSCHANGED => {
            let pos = unsafe { &*(lp.0 as *const WINDOWPOS) };
            if pos.flags.contains(SWP_SHOWWINDOW) {
                crate::index::mem_log("ventana: mostrada");
            }
            if pos.flags.contains(SWP_SHOWWINDOW) || pos.flags.contains(SWP_FRAMECHANGED) {
                burst(500);
            }
        }
        WM_SHOWWINDOW | WM_ACTIVATEAPP | WM_DISPLAYCHANGE | WM_DPICHANGED | 0x031E /* WM_DWMCOMPOSITIONCHANGED */ => burst(500),
        WM_SIZE if wp.0 as u32 == SIZE_RESTORED || wp.0 as u32 == SIZE_MAXIMIZED => burst(300),
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// Instalar en el hilo de la ventana (al crear la app).
pub fn install(hwnd: HWND, wake: impl Fn() + Send + Sync + 'static, bg: [u8; 3]) {
    let _ = WAKE.set(Box::new(wake));
    set_background(bg);
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass), 1, 0);
        // Las ventanas hijas (visores de vista previa) no deben quedar debajo del dibujo de OpenGL.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_CLIPCHILDREN.0 as isize);
        // Que Windows vuelva a calcular el marco: desde ya, sin barra de título.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// El diálogo de Windows para elegir una carpeta (empieza en `start` si existe).
pub fn pick_folder(start: &str) -> Option<String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH};
    unsafe {
        // El hilo de la ventana ya tiene COM (STA) por el arrastrar y soltar; por las dudas.
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        if !start.trim().is_empty() {
            let w: Vec<u16> = start.trim().encode_utf16().chain([0]).collect();
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(windows::core::PCWSTR(w.as_ptr()), None) {
                let _ = dlg.SetFolder(&item);
            }
        }
        dlg.Show(Some(GetForegroundWindow())).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Área útil (sin la barra de tareas) del monitor donde está este punto, en píxeles.
pub fn work_area_at(x: i32, y: i32) -> RECT {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    unsafe {
        let mon = MonitorFromPoint(windows::Win32::Foundation::POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut mi).as_bool() { mi.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1040 } }
    }
}

/// Da a una ventana del menú de clic derecho el aspecto de un menú de Windows 11
/// (esquinas redondeadas, sin botón en la barra de tareas). Devuelve si la encontró.
pub fn style_menu_window(title: &str, take_focus: bool) -> bool {
    use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
    let w: Vec<u16> = title.encode_utf16().chain([0]).collect();
    unsafe {
        let Ok(h) = FindWindowW(windows::core::PCWSTR::null(), windows::core::PCWSTR(w.as_ptr())) else { return false };
        if h.is_invalid() {
            return false;
        }
        let round: u32 = 2; // DWMWCP_ROUND
        let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const u32 as *const _, 4);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mut nex = ex | WS_EX_TOOLWINDOW.0 as isize;
        if !take_focus {
            nex |= WS_EX_NOACTIVATE.0 as isize;
        }
        if nex != ex {
            SetWindowLongPtrW(h, GWL_EXSTYLE, nex);
        }
        if take_focus {
            let _ = SetForegroundWindow(h);
        }
        true
    }
}

/// Ocultar la ventana ahora mismo (pasar a segundo plano).
/// Pone en la ventana el ícono que lleva el .exe, en el tamaño justo para la pantalla
/// (nítido en la barra de tareas y en Alt+Tab, en vez de una imagen escalada).
pub fn set_icons(hwnd: HWND) {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let dpi = GetDpiForWindow(hwnd).max(96);
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let s = GetSystemMetricsForDpi(metric, dpi);
            // Recurso 1: el ícono que incrusta build.rs.
            if let Ok(h) = LoadImageW(Some(module.into()), windows::core::PCWSTR(1 as _), IMAGE_ICON, s, s, LR_DEFAULTCOLOR) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(h.0 as isize)));
            }
        }
    }
}

pub fn hide(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Devolver al sistema la memoria que no se está usando (al quedar en segundo plano).
pub fn trim_memory() {
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::GetCurrentProcess;
    crate::index::release_free_memory();
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}
