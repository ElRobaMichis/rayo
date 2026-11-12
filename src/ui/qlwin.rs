//! Marco nativo de la ventana de Vista rápida: sin barra de título (como `win.rs`), sin
//! foco, poseída por la principal, con bordes, sombra y esquinas de Windows.
//!
//! - `adopt` la encuentra (por su título, entre las ventanas de este hilo), le pone el
//!   *subclass*, los estilos, la dueña y el rectángulo exacto en píxeles.
//! - El *subclass* quita la barra de título y decide qué es «barra» (`set_caption`), y
//!   nunca deja que la ventana se active: un clic en ella trae a Rayo al frente.
//! - Maximizar es propio (al área útil del monitor), no el de Windows: el de Windows la
//!   activaría.

use super::ql::{QL_DEFAULT, QL_MIN, VISIBLE_MIN};
use super::theme::Theme;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateSolidBrush, DeleteObject, FillRect, GetMonitorInfoW, HBRUSH, HDC, HGDIOBJ, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    MonitorFromRect, MonitorFromWindow, ScreenToClient,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

/// Lo que pasó en la ventana desde la última consulta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinEvent {
    /// Doble clic en el encabezado: maximizar / restaurar.
    ToggleMax,
    /// Terminó de moverse o redimensionarse: guardar el lugar.
    Moved,
}

/// La ventana adoptada (0 = ninguna) y su dueña.
static QL: AtomicIsize = AtomicIsize::new(0);
static MAIN: AtomicIsize = AtomicIsize::new(0);
/// Pincel del fondo (`sunken`): sin destello blanco al agrandar.
static BRUSH: AtomicIsize = AtomicIsize::new(0);
/// Colores ya aplicados (fondo, borde, oscuro), para no repetirlos en cada cuadro.
static APPLIED: AtomicU64 = AtomicU64::new(u64::MAX);
/// Zonas del encabezado que arrastran la ventana (px del área cliente).
static CAPTION: Mutex<Vec<RECT>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Vec<WinEvent>> = Mutex::new(Vec::new());
/// Maximizada: el rectángulo de antes (para restaurar y para recordar).
static RESTORE: Mutex<Option<RECT>> = Mutex::new(None);
/// Mientras se coloca: el rectángulo que tiene que quedar aunque cambie de escala.
static FORCE: Mutex<Option<RECT>> = Mutex::new(None);

const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTTOP: isize = 12;
const HTTOPLEFT: isize = 13;
const HTTOPRIGHT: isize = 14;

/// Estilos que la ventana tiene que conservar (winit los reescribe al mostrarla).
const EX_KEEP: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0);

fn rect_of(r: [i32; 4]) -> RECT {
    RECT { left: r[0], top: r[1], right: r[0] + r[2], bottom: r[1] + r[3] }
}

fn arr_of(r: &RECT) -> [i32; 4] {
    [r.left, r.top, r.right - r.left, r.bottom - r.top]
}

fn colorref(c: egui::Color32) -> COLORREF {
    COLORREF(c.r() as u32 | (c.g() as u32) << 8 | (c.b() as u32) << 16)
}

fn wake() {
    super::win::burst(0);
}

fn push(e: WinEvent) {
    EVENTS.lock().push(e);
    wake();
}

/// La ventana de la Vista rápida, si está adoptada y viva.
pub fn hwnd() -> Option<HWND> {
    let h = HWND(QL.load(Ordering::Relaxed) as *mut _);
    (!h.is_invalid() && unsafe { IsWindow(Some(h)) }.as_bool()).then_some(h)
}

fn main_hwnd() -> Option<HWND> {
    let h = HWND(MAIN.load(Ordering::Relaxed) as *mut _);
    (!h.is_invalid()).then_some(h)
}

/// Borde invisible para redimensionar (izquierda, derecha y abajo) a esta escala.
fn border_at(dpi: u32) -> i32 {
    unsafe { GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) }
}

fn monitor_dpi(mon: HMONITOR) -> u32 {
    let (mut x, mut y) = (96u32, 96u32);
    unsafe {
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    x.max(96)
}

fn work_of(mon: HMONITOR) -> Option<RECT> {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(mon, &mut mi) }.as_bool().then_some(mi.rcWork)
}

/// ¿Quedan a la vista al menos 120 × 80 px de `r` en el área útil de algún monitor?
fn visible_enough(r: [i32; 4]) -> bool {
    if r[2] <= 0 || r[3] <= 0 {
        return false;
    }
    let rc = rect_of(r);
    let mon = unsafe { MonitorFromRect(&rc, MONITOR_DEFAULTTONULL) };
    if mon.is_invalid() {
        return false;
    }
    let Some(work) = work_of(mon) else { return false };
    let w = rc.right.min(work.right) - rc.left.max(work.left);
    let h = rc.bottom.min(work.bottom) - rc.top.max(work.top);
    w >= VISIBLE_MIN[0] && h >= VISIBLE_MIN[1]
}

/// Dónde abrir: el lugar recordado si sigue a la vista; si no, 760 × 560 centrada sobre Rayo,
/// en su monitor, sin salirse del área útil. Rectángulo exterior en px (x, y, ancho, alto).
pub fn placement(saved: Option<[i32; 4]>, main: Option<HWND>) -> [i32; 4] {
    if let Some(r) = saved.filter(|&r| visible_enough(r)) {
        return r;
    }
    unsafe {
        let mut owner = RECT::default();
        let main = main.filter(|&m| !IsIconic(m).as_bool() && GetWindowRect(m, &mut owner).is_ok());
        let mon = match main {
            Some(m) => MonitorFromWindow(m, MONITOR_DEFAULTTONEAREST),
            None => MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY),
        };
        let work = work_of(mon).unwrap_or(RECT { left: 0, top: 0, right: 1920, bottom: 1040 });
        if main.is_none() {
            owner = work;
        }
        let dpi = monitor_dpi(mon);
        let (s, b) = (dpi as f32 / 96.0, border_at(dpi));
        let w = ((QL_DEFAULT.x * s).round() as i32 + 2 * b).min(work.right - work.left + 2 * b);
        let h = ((QL_DEFAULT.y * s).round() as i32 + b).min(work.bottom - work.top + b);
        let cx = (owner.left + owner.right) / 2;
        let cy = (owner.top + owner.bottom) / 2;
        let x = (cx - w / 2).clamp(work.left - b, (work.right + b - w).max(work.left - b));
        let y = (cy - h / 2).clamp(work.top, (work.bottom + b - h).max(work.top));
        [x, y, w, h]
    }
}

/// Busca la ventana `title` de este hilo y la prepara: subclass, estilos, dueña `main`,
/// DWM y el rectángulo `rect` (px de pantalla: x, y, ancho, alto). ¿La encontró?
pub fn adopt(title: &str, main: Option<HWND>, rect: [i32; 4], t: &Theme) -> bool {
    let Some(h) = find(title) else { return false };
    QL.store(h.0 as isize, Ordering::Relaxed);
    MAIN.store(main.map_or(0, |m| m.0 as isize), Ordering::Relaxed);
    *RESTORE.lock() = None;
    APPLIED.store(u64::MAX, Ordering::Relaxed);
    set_theme(t);
    let want = rect_of(rect);
    unsafe {
        let _ = SetWindowSubclass(h, Some(ql_proc), 2, 0);
        // Sin foco y fuera de la barra de tareas; las hijas (visor, video) no quedan bajo OpenGL.
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | EX_KEEP.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        SetWindowLongPtrW(h, GWL_STYLE, style | WS_CLIPCHILDREN.0 as isize);
        // Poseída por la principal: siempre encima de Rayo, nunca de otros programas.
        if let Some(m) = main {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, m.0 as isize);
        }
        // Windows 11; en 10 falla y no importa.
        dwm(h, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_ROUND);
        // La animación de apertura de Windows escala: la ventana aparece en su lugar, quieta.
        dwm(h, DWMWA_TRANSITIONS_FORCEDISABLED, 1u32);
        // El rectángulo exacto; Windows recalcula el marco ya sin barra de título. Si al
        // pasar a otro monitor cambia la escala, winit la acomoda a su modo: se repite.
        *FORCE.lock() = Some(want);
        let flags = SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER;
        let _ = SetWindowPos(h, None, want.left, want.top, rect[2], rect[3], flags);
        let mut now = RECT::default();
        if GetWindowRect(h, &mut now).is_ok() && now != want {
            let _ = SetWindowPos(h, None, want.left, want.top, rect[2], rect[3], flags);
        }
        *FORCE.lock() = None;
    }
    true
}

fn dwm<T>(h: HWND, attr: DWMWINDOWATTRIBUTE, v: T) {
    unsafe {
        let _ = DwmSetWindowAttribute(h, attr, &v as *const T as *const _, std::mem::size_of::<T>() as u32);
    }
}

fn find(title: &str) -> Option<HWND> {
    struct Search {
        want: Vec<u16>,
        found: HWND,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let s = unsafe { &mut *(lp.0 as *mut Search) };
        let mut buf = [0u16; 64];
        let n = unsafe { GetWindowTextW(h, &mut buf) }.max(0) as usize;
        if buf[..n] == s.want[..] {
            s.found = h;
            return false.into();
        }
        true.into()
    }
    let mut s = Search { want: title.encode_utf16().collect(), found: HWND::default() };
    unsafe {
        let _ = EnumThreadWindows(windows::Win32::System::Threading::GetCurrentThreadId(), Some(each), LPARAM(&mut s as *mut Search as isize));
    }
    (!s.found.is_invalid()).then_some(s.found)
}

/// Colores del tema: fondo al agrandar, borde `border_strong`, marco oscuro o claro. Barato:
/// solo hace algo si cambiaron.
pub fn set_theme(t: &Theme) {
    let Some(h) = hwnd() else { return };
    let maxed = maximized();
    let key = (colorref(t.sidebar).0 as u64) | (colorref(t.border_strong).0 as u64) << 24 | (t.dark as u64) << 48 | (maxed as u64) << 49;
    if APPLIED.swap(key, Ordering::Relaxed) == key {
        return;
    }
    let brush = unsafe { CreateSolidBrush(colorref(t.sidebar)) };
    let old = BRUSH.swap(brush.0 as isize, Ordering::Relaxed);
    if old != 0 {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(old as *mut _));
        }
    }
    // Maximizada, sin borde: caería 1 px fuera del área útil, sobre el monitor de al lado.
    dwm(h, DWMWA_BORDER_COLOR, if maxed { DWMWA_COLOR_NONE } else { colorref(t.border_strong).0 });
    dwm(h, DWMWA_USE_IMMERSIVE_DARK_MODE, BOOL::from(t.dark));
}

/// `DWMWA_COLOR_NONE`: sin borde de DWM.
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
/// `DWM_WINDOW_CORNER_PREFERENCE`: sin redondear (maximizada) y redondeada (normal).
const CORNER_SQUARE: u32 = 1;
const CORNER_ROUND: u32 = 2;

/// Zonas del encabezado que arrastran la ventana (puntos de egui; `ppp` = píxeles por punto).
pub fn set_caption(rects: &[egui::Rect], ppp: f32) {
    let px = |r: &egui::Rect| RECT { left: (r.left() * ppp).round() as i32, top: (r.top() * ppp).round() as i32, right: (r.right() * ppp).round() as i32, bottom: (r.bottom() * ppp).round() as i32 };
    *CAPTION.lock() = rects.iter().map(px).collect();
}

/// Lo que pasó desde la última vez (y lo olvida).
pub fn take_events() -> Vec<WinEvent> {
    std::mem::take(&mut *EVENTS.lock())
}

/// ¿Está maximizada (al área útil)?
pub fn maximized() -> bool {
    RESTORE.lock().is_some()
}

/// Maximizar al área útil del monitor, o volver al rectángulo de antes. Sin activarla.
pub fn toggle_max() {
    let Some(h) = hwnd() else { return };
    let restore = RESTORE.lock().take();
    let flags = SWP_NOACTIVATE | SWP_NOZORDER;
    unsafe {
        match restore {
            Some(r) => {
                // Vuelven las esquinas redondeadas; el borde lo repone `set_theme` en el cuadro siguiente.
                dwm(h, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_ROUND);
                APPLIED.store(u64::MAX, Ordering::Relaxed);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, flags);
            }
            None => {
                let mut cur = RECT::default();
                if GetWindowRect(h, &mut cur).is_err() {
                    return;
                }
                let Some(work) = work_of(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST)) else { return };
                *RESTORE.lock() = Some(cur);
                // Llena el área útil: esquinas en ángulo recto (redondeadas se veía lo de atrás en
                // las cuatro) y sin el borde de 1 px, que quedaba afuera, sobre el monitor de al lado.
                dwm(h, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_SQUARE);
                dwm(h, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
                APPLIED.store(u64::MAX, Ordering::Relaxed);
                // Los bordes invisibles quedan fuera del área útil: lo que se ve la llena justo.
                let b = super::win::frame_thickness(h);
                let _ = SetWindowPos(h, None, work.left - b, work.top, work.right - work.left + 2 * b, work.bottom - work.top + b, flags);
            }
        }
    }
}

/// Rectángulo sin maximizar (px de pantalla), para recordarlo.
pub fn normal_rect() -> Option<[i32; 4]> {
    if let Some(r) = *RESTORE.lock() {
        return Some(arr_of(&r));
    }
    let h = hwnd()?;
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    Some(arr_of(&r))
}

/// Ocultarla ya (primer tiempo del cierre).
pub fn hide() {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// Volver a mostrarla sin activarla (reabierta mientras se cerraba).
pub fn show_again() {
    if let Some(h) = hwnd() {
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }
}

/// ¿El puntero está sobre la ventana (su área visible)?
pub fn cursor_inside() -> bool {
    let Some(h) = hwnd() else { return false };
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        let mut pt = POINT::default();
        let mut rc = RECT::default();
        if GetCursorPos(&mut pt).is_err() || GetClientRect(h, &mut rc).is_err() {
            return false;
        }
        let mut origin = POINT::default();
        let _ = ClientToScreen(h, &mut origin);
        super::win::inside(&rc, pt.x - origin.x, pt.y - origin.y)
    }
}

unsafe extern "system" fn ql_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match msg {
        // Sin barra de título: el área de la app llega hasta arriba; los bordes de los costados
        // y de abajo quedan como los pone Windows (para redimensionar).
        WM_NCCALCSIZE if wp.0 != 0 => {
            let params = unsafe { &mut *(lp.0 as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            params.rgrc[0].top = top;
            return r;
        }
        WM_NCHITTEST => {
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            let maxed = RESTORE.lock().is_some();
            if r.0 != HTCLIENT {
                // Maximizada no se redimensiona.
                let border = (HTLEFT as isize..=HTBOTTOMRIGHT as isize).contains(&r.0);
                return if maxed && border { LRESULT(HTCLIENT) } else { r };
            }
            let mut pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
            let mut rc = RECT::default();
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
                let _ = GetClientRect(hwnd, &mut rc);
            }
            // Maximizada, el encabezado es de egui (doble clic restaura; no se arrastra).
            if maxed {
                return r;
            }
            // El borde de arriba para redimensionar (ya no lo da la barra de título).
            let b = super::win::frame_thickness(hwnd);
            if pt.y < b {
                return LRESULT(if pt.x < b * 2 {
                    HTTOPLEFT
                } else if pt.x >= rc.right - b * 2 {
                    HTTOPRIGHT
                } else {
                    HTTOP
                });
            }
            if CAPTION.lock().iter().any(|c| super::win::inside(c, pt.x, pt.y)) {
                return LRESULT(HTCAPTION);
            }
            return r;
        }
        // Doble clic en el encabezado: maximizar a lo propio (el de Windows la activaría).
        WM_NCLBUTTONDBLCLK if wp.0 as isize == HTCAPTION => {
            push(WinEvent::ToggleMax);
            return LRESULT(0);
        }
        WM_SYSCOMMAND => match (wp.0 as u32) & 0xFFF0 {
            SC_MAXIMIZE => {
                push(WinEvent::ToggleMax);
                return LRESULT(0);
            }
            SC_MINIMIZE => return LRESULT(0),
            _ => {}
        },
        // Nunca se activa: un clic trae a Rayo al frente (así el arrastre se ve en vivo) y el
        // teclado sigue en la lista.
        WM_MOUSEACTIVATE => {
            if let Some(m) = main_hwnd() {
                unsafe {
                    if GetForegroundWindow() != m {
                        let _ = SetForegroundWindow(m);
                    }
                }
            }
            return LRESULT(MA_NOACTIVATE as isize);
        }
        // Red de seguridad: si algo la activa igual, la activación vuelve a la principal.
        WM_ACTIVATE if (wp.0 & 0xFFFF) != 0 => {
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            if let Some(m) = main_hwnd() {
                unsafe {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(m);
                }
            }
            return r;
        }
        // winit reescribe los estilos al mostrarla: se conservan los nuestros.
        WM_STYLECHANGING => {
            let ss = unsafe { &mut *(lp.0 as *mut STYLESTRUCT) };
            if wp.0 as i32 == GWL_EXSTYLE.0 {
                ss.styleNew = (ss.styleNew | EX_KEEP.0) & !WS_EX_APPWINDOW.0;
            } else if wp.0 as i32 == GWL_STYLE.0 {
                ss.styleNew |= WS_CLIPCHILDREN.0;
            }
        }
        WM_GETMINMAXINFO => {
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            let mmi = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
            let (s, b) = (dpi as f32 / 96.0, border_at(dpi));
            mmi.ptMinTrackSize = POINT { x: (QL_MIN.x * s).round() as i32 + 2 * b, y: (QL_MIN.y * s).round() as i32 + b };
            return r;
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
        WM_EXITSIZEMOVE => {
            if RESTORE.lock().is_none() {
                push(WinEvent::Moved);
            }
        }
        WM_WINDOWPOSCHANGED => {
            let pos = unsafe { &*(lp.0 as *const WINDOWPOS) };
            if pos.flags.contains(SWP_SHOWWINDOW) || pos.flags.contains(SWP_FRAMECHANGED) {
                super::win::burst(300);
            }
        }
        // Otra escala: winit avisa a egui y la acomoda (contando una barra de título que no
        // hay); después se aplica el rectángulo que sugiere Windows, o el que se está poniendo.
        WM_DPICHANGED => {
            super::win::burst(300);
            let suggested = unsafe { *(lp.0 as *const RECT) };
            let r = unsafe { DefSubclassProc(hwnd, msg, wp, lp) };
            let want = (*FORCE.lock()).unwrap_or(suggested);
            unsafe {
                let _ = SetWindowPos(hwnd, None, want.left, want.top, want.right - want.left, want.bottom - want.top, SWP_NOACTIVATE | SWP_NOZORDER);
            }
            return r;
        }
        WM_NCDESTROY => {
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(ql_proc), 2);
            }
            if QL.load(Ordering::Relaxed) == hwnd.0 as isize {
                QL.store(0, Ordering::Relaxed);
                *RESTORE.lock() = None;
                CAPTION.lock().clear();
            }
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}
