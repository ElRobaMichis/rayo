//! Visores de vista previa de Windows (los del panel del Explorador): Word, Excel,
//! PowerPoint, Outlook, fuentes, videos… lo que cada programa haya registrado.
//!
//! Cada visor dibuja en una ventana nativa propia; Rayo le presta una ventana hija
//! colocada sobre el panel. Todo corre en un hilo aparte: un visor lento (Office
//! tarda en abrir) nunca congela la interfaz.

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, FillRect, HDC};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CLSIDFromString, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, Interface, PCWSTR, PWSTR, w};

const HANDLER_KEY: PCWSTR = w!("{8895b1c6-b41f-4c1c-a562-0d564250836f}");

/// Visor registrado para una extensión (se recuerda).
pub fn handler_for(ext: &str) -> Option<GUID> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<GUID>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(g) = cache.lock().get(ext) {
        return *g;
    }
    let found = unsafe {
        let e = crate::index::walk::wide(&format!(".{ext}"));
        let mut buf = [0u16; 64];
        let mut len = buf.len() as u32;
        let hr = AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, PCWSTR(e.as_ptr()), HANDLER_KEY, Some(PWSTR(buf.as_mut_ptr())), &mut len);
        if hr.is_ok() { CLSIDFromString(PCWSTR(buf.as_ptr())).ok() } else { None }
    };
    cache.lock().insert(ext.to_string(), found);
    found
}

#[derive(Debug)]
enum Cmd {
    /// Mostrar este archivo en este rectángulo (píxeles de la ventana principal).
    Show(String, [i32; 4]),
    /// Ocultar sin descargar el visor (vuelve a aparecer al instante).
    Hide,
    /// Descargar el visor (se liberan el archivo y la memoria).
    Unload,
    /// Descargar y avisar cuando el archivo ya quedó libre.
    Release(crossbeam_channel::Sender<()>),
    Background([u8; 3]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Loading,
    Ready,
    Failed,
}

pub struct PreviewHost {
    tx: Sender<Cmd>,
    /// Estado de cada archivo pedido.
    pub status: Arc<Mutex<HashMap<String, Status>>>,
    last: Mutex<Option<(String, [i32; 4])>>,
    shown: Mutex<bool>,
}

/// Color de fondo (0x00RRGGBB). Atómico: la ventana lo lee al pintarse, incluso
/// mientras el mismo hilo está dentro de una llamada al visor.
static BG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x1B1C21);

fn bg() -> [u8; 3] {
    let v = BG.load(std::sync::atomic::Ordering::Relaxed);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn colorref(c: [u8; 3]) -> COLORREF {
    COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
}

unsafe extern "system" fn container_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ERASEBKGND {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
            let brush = CreateSolidBrush(colorref(bg()));
            FillRect(HDC(wp.0 as *mut _), &rc, brush);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush.into());
        }
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

struct Active {
    path: String,
    handler: IPreviewHandler,
}

fn open(path: &str, clsid: &GUID, container: HWND, rc: &RECT, bg: [u8; 3], fg: [u8; 3]) -> Option<IPreviewHandler> {
    unsafe {
        // Casi todos corren fuera del proceso (prevhost.exe): si uno falla, no tumba a Rayo.
        let log = |m: &str| crate::index::mem_log(&format!("visor: {m}"));
        let handler: IPreviewHandler = match CoCreateInstance(clsid, None, CLSCTX_LOCAL_SERVER) {
            Ok(h) => h,
            Err(e) => {
                log(&format!("fuera del proceso falló ({e}); dentro"));
                match CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER) {
                    Ok(h) => h,
                    Err(e) => {
                        log(&format!("no se pudo crear: {e}"));
                        return None;
                    }
                }
            }
        };
        let wpath = crate::index::walk::wide(path);
        let mode = 0u32; // STGM_READ
        let init = if let Ok(i) = handler.cast::<IInitializeWithStream>() {
            SHCreateStreamOnFileEx(PCWSTR(wpath.as_ptr()), 0x40 /* STGM_SHARE_DENY_NONE */, 0, false, None).and_then(|s| i.Initialize(&s, mode))
        } else if let Ok(i) = handler.cast::<IInitializeWithFile>() {
            i.Initialize(PCWSTR(wpath.as_ptr()), mode)
        } else if let Ok(i) = handler.cast::<IInitializeWithItem>() {
            SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wpath.as_ptr()), None).and_then(|item| i.Initialize(&item, mode))
        } else {
            return None;
        };
        if let Err(e) = &init {
            log(&format!("inicializar falló: {e}"));
        }
        init.ok()?;
        if let Ok(v) = handler.cast::<IPreviewHandlerVisuals>() {
            let _ = v.SetBackgroundColor(colorref(bg));
            let _ = v.SetTextColor(colorref(fg));
        }
        if let Err(e) = handler.SetWindow(container, rc) {
            log(&format!("SetWindow falló: {e}"));
            return None;
        }
        if let Err(e) = handler.DoPreview() {
            log(&format!("DoPreview falló: {e}"));
            return None;
        }
        log("DoPreview listo");
        Some(handler)
    }
}

fn run(parent: HWND, rx: Receiver<Cmd>, status: Arc<Mutex<HashMap<String, Status>>>, wake: Arc<dyn Fn() + Send + Sync>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(inst) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
        let class = w!("RayoVistaPrevia");
        let wc = WNDCLASSW { lpfnWndProc: Some(container_proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(container) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, 0, 0, 10, 10, Some(parent), None, Some(inst.into()), None) else {
            return;
        };
        let mut active: Option<Active> = None;
        let mut fg = [0xE8, 0xEA, 0xEF];
        loop {
            // Atender los mensajes de la ventana (el visor dibuja con ellos) y las órdenes de la app.
            MsgWaitForMultipleObjects(None, false, 30, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let mut cmds: Vec<Cmd> = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(c) => cmds.push(c),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(_) => {
                        if let Some(a) = active.take() {
                            let _ = a.handler.Unload();
                        }
                        let _ = DestroyWindow(container);
                        return;
                    }
                }
            }
            // De varias órdenes "mostrar" seguidas, solo cuenta la última.
            let last_show = cmds.iter().rposition(|c| matches!(c, Cmd::Show(..)));
            for (k, c) in cmds.into_iter().enumerate() {
                match c {
                    Cmd::Background(c) => {
                        BG.store((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32, std::sync::atomic::Ordering::Relaxed);
                        fg = if (c[0] as u32 + c[1] as u32 + c[2] as u32) < 384 { [0xE8, 0xEA, 0xEF] } else { [0x1D, 0x21, 0x27] };
                    }
                    Cmd::Hide => {
                        let _ = ShowWindow(container, SW_HIDE);
                    }
                    Cmd::Unload => {
                        let _ = ShowWindow(container, SW_HIDE);
                        if let Some(a) = active.take() {
                            let _ = a.handler.Unload();
                        }
                    }
                    Cmd::Release(done) => {
                        let _ = ShowWindow(container, SW_HIDE);
                        if let Some(a) = active.take() {
                            let _ = a.handler.Unload();
                        }
                        let _ = done.send(());
                    }
                    Cmd::Show(path, r) if Some(k) == last_show => {
                        let (x, y, w, h) = (r[0], r[1], r[2].max(1), r[3].max(1));
                        let _ = SetWindowPos(container, Some(HWND_TOP), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                        let rc = RECT { left: 0, top: 0, right: w, bottom: h };
                        if active.as_ref().is_some_and(|a| a.path == path) {
                            if let Some(a) = &active {
                                let _ = a.handler.SetRect(&rc);
                            }
                            continue;
                        }
                        if let Some(a) = active.take() {
                            let _ = a.handler.Unload();
                        }
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(container), None, true);
                        status.lock().insert(path.clone(), Status::Loading);
                        wake();
                        let ext = crate::fsx::ext_of(&path);
                        let clsid = handler_for(&ext);
                        let result = clsid.and_then(|clsid| open(&path, &clsid, container, &rc, bg(), fg));
                        let ok = result.is_some();
                        if let Some(handler) = result {
                            active = Some(Active { path: path.clone(), handler });
                        } else {
                            let _ = ShowWindow(container, SW_HIDE);
                        }
                        status.lock().insert(path, if ok { Status::Ready } else { Status::Failed });
                        wake();
                    }
                    Cmd::Show(..) => {}
                }
            }
        }
    }
}

impl PreviewHost {
    pub fn new(parent: HWND, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = unbounded();
        let status: Arc<Mutex<HashMap<String, Status>>> = Default::default();
        let st = status.clone();
        let p = parent.0 as usize;
        std::thread::Builder::new()
            .name("vista-previa".into())
            .spawn(move || run(HWND(p as *mut _), rx, st, wake))
            .ok();
        PreviewHost { tx, status, last: Mutex::new(None), shown: Mutex::new(false) }
    }

    /// Mostrar `path` en `rect` (píxeles). Solo envía algo si cambió.
    pub fn show(&self, path: &str, rect: [i32; 4]) {
        let mut last = self.last.lock();
        let mut shown = self.shown.lock();
        if *shown && last.as_ref().is_some_and(|(p, r)| p == path && *r == rect) {
            return;
        }
        *last = Some((path.to_string(), rect));
        *shown = true;
        let _ = self.tx.send(Cmd::Show(path.to_string(), rect));
    }

    pub fn hide(&self) {
        let mut shown = self.shown.lock();
        if *shown {
            *shown = false;
            let _ = self.tx.send(Cmd::Hide);
        }
    }

    pub fn unload(&self) {
        *self.shown.lock() = false;
        if self.last.lock().take().is_some() {
            let _ = self.tx.send(Cmd::Unload);
        }
    }

    /// Si el visor muestra uno de estos archivos (o algo dentro de estas carpetas), lo
    /// suelta y espera a que el archivo quede libre: los visores de Office o PDF lo
    /// tienen abierto y no dejarían borrarlo, moverlo ni renombrarlo.
    pub fn release(&self, paths: &[String]) {
        let hit = self.last.lock().as_ref().is_some_and(|(p, _)| {
            let p = p.to_lowercase();
            paths.iter().any(|q| {
                let q = q.to_lowercase();
                p == q || p.starts_with(&format!("{}\\", q.trim_end_matches('\\')))
            })
        });
        if !hit {
            return;
        }
        self.last.lock().take();
        *self.shown.lock() = false;
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.tx.send(Cmd::Release(tx)).is_ok() {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(1500));
        }
    }

    pub fn set_background(&self, c: [u8; 3]) {
        let _ = self.tx.send(Cmd::Background(c));
    }

    pub fn status_of(&self, path: &str) -> Option<Status> {
        self.status.lock().get(path).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crear el visor de RTF de Windows fuera de la app (diagnóstico de activación).
    #[test]
    fn creates_rtf_handler() {
        let Some(clsid) = handler_for("rtf") else { return };
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let t = std::time::Instant::now();
            let r: windows::core::Result<IPreviewHandler> = CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER);
            let _ = tx.send(format!("{:?} en {:?}", r.map(|_| "ok"), t.elapsed()));
        });
        let res = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or_else(|_| "SE COLGÓ".into());
        println!("crear visor RTF: {res}");
        assert!(res.starts_with("Ok"), "{res}");
    }
}
