//! Visores de vista previa de Windows (los del panel del Explorador): Word, Excel,
//! PowerPoint, Outlook, fuentes, videos… lo que cada programa haya registrado.
//!
//! Cada visor dibuja en una ventana nativa propia; Rayo le presta una ventana hija
//! colocada sobre el panel. Todo corre en un hilo aparte: un visor lento (Office
//! tarda en abrir) nunca congela la interfaz.

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Mostrar este archivo en este rectángulo, dentro de este padre (0 = la ventana
    /// principal; si no, la de la Vista rápida). Píxeles del área cliente del padre.
    Show(String, [i32; 4], isize),
    /// Ocultar y volver a la ventana principal; el número se anota en `Shared::homed`.
    Home(u64),
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
    side: Mutex<Side>,
    shared: Arc<Shared>,
    /// La ventana principal (`show_in` con ella es `show`).
    main: isize,
}

/// Lo último que pidió la app.
#[derive(Default)]
struct Side {
    last: Option<(String, [i32; 4])>,
    shown: bool,
    /// Padre pedido: 0 = la principal; si no, la Vista rápida («afuera»).
    parent: isize,
    /// Última orden `Home` enviada.
    home_seq: u64,
    /// `Shared::lost` la última vez que se miró.
    lost_seen: u64,
}

/// Lo que el hilo del visor le cuenta a la app.
#[derive(Default)]
struct Shared {
    /// Última orden `Home` atendida.
    homed: AtomicU64,
    /// Veces que el contenedor murió (con la ventana que lo tenía) y se volvió a crear.
    lost: AtomicU64,
}

/// El contenedor está fuera de la ventana principal (en la Vista rápida).
static AWAY: AtomicBool = AtomicBool::new(false);

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
    // En la Vista rápida, un clic en el visor no la activa (el teclado es de la principal).
    // Igual se pregunta al padre: su subclass trae a Rayo al frente.
    if msg == WM_MOUSEACTIVATE && AWAY.load(Ordering::Relaxed) {
        let r = unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        return LRESULT(if r.0 as u32 == MA_ACTIVATEANDEAT || r.0 as u32 == MA_NOACTIVATEANDEAT { MA_NOACTIVATEANDEAT } else { MA_NOACTIVATE } as isize);
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

fn create_container(main: HWND) -> Option<HWND> {
    unsafe {
        let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
        CreateWindowExW(WINDOW_EX_STYLE(0), w!("RayoVistaPrevia"), w!(""), WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, 0, 0, 10, 10, Some(main), None, Some(inst.into()), None).ok()
    }
}

fn run(main: HWND, rx: Receiver<Cmd>, status: Arc<Mutex<HashMap<String, Status>>>, shared: Arc<Shared>, wake: Arc<dyn Fn() + Send + Sync>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(inst) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
        let wc = WNDCLASSW { lpfnWndProc: Some(container_proc), hInstance: inst.into(), lpszClassName: w!("RayoVistaPrevia"), ..Default::default() };
        RegisterClassW(&wc);
        let Some(mut container) = create_container(main) else { return };
        let mut active: Option<Active> = None;
        let mut fg = [0xE8, 0xEA, 0xEF];
        // Padre actual: 0 = la principal.
        let mut cur: isize = 0;
        // Volvió a la principal con `Home` (que cambia de padre sin un `Show`): el `Show` siguiente
        // del panel cuenta como cambio de ventana aunque `parent == cur`.
        let mut rehomed = false;
        let set_cur = |cur: &mut isize, p: isize| {
            *cur = p;
            AWAY.store(p != 0, Ordering::Relaxed);
        };
        loop {
            // Atender los mensajes de la ventana (el visor dibuja con ellos) y las órdenes de la app.
            MsgWaitForMultipleObjects(None, false, 30, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Guardia: una ventana hija muere con su padre. Si la Vista rápida se cerró con el
            // contenedor adentro, se crea otro en la principal y se avisa (la app lo vuelve a pedir).
            if !IsWindow(Some(container)).as_bool() {
                crate::index::mem_log("visor: el contenedor murió con su ventana; se crea otro");
                if let Some(a) = active.take() {
                    let _ = a.handler.Unload();
                }
                let Some(c) = create_container(main) else { return };
                container = c;
                set_cur(&mut cur, 0);
                shared.lost.fetch_add(1, Ordering::AcqRel);
                wake();
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
                        BG.store((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32, Ordering::Relaxed);
                        fg = if (c[0] as u32 + c[1] as u32 + c[2] as u32) < 384 { [0xE8, 0xEA, 0xEF] } else { [0x1D, 0x21, 0x27] };
                    }
                    Cmd::Hide => {
                        let _ = ShowWindow(container, SW_HIDE);
                    }
                    Cmd::Home(seq) => {
                        let _ = ShowWindow(container, SW_HIDE);
                        if cur != 0 {
                            // Sin descargar el visor: al volver al panel no se recarga.
                            if SetParent(container, Some(main)).is_err() {
                                crate::index::mem_log("visor: no pudo volver a la ventana principal");
                            }
                            set_cur(&mut cur, 0);
                            rehomed = true;
                        }
                        shared.homed.store(seq, Ordering::Release);
                        wake();
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
                    Cmd::Show(path, r, parent) if Some(k) == last_show => {
                        let moved = parent != cur || std::mem::take(&mut rehomed);
                        if parent != cur {
                            // Cambio de padre: el visor sigue atado al mismo contenedor (no se recarga).
                            let _ = ShowWindow(container, SW_HIDE);
                            let target = if parent == 0 { main } else { HWND(parent as *mut _) };
                            if !IsWindow(Some(target)).as_bool() || SetParent(container, Some(target)).is_err() {
                                crate::index::mem_log("visor: no se pudo cambiar de ventana");
                                continue;
                            }
                            set_cur(&mut cur, parent);
                        }
                        let (x, y, w, h) = (r[0], r[1], r[2].max(1), r[3].max(1));
                        let _ = SetWindowPos(container, Some(HWND_TOP), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                        let rc = RECT { left: 0, top: 0, right: w, bottom: h };
                        // El visor de fuentes de Windows no se reacomoda al cambiar de ventana (queda el
                        // dibujo viejo, angosto, y uno nuevo corrido): para él se recarga, que es al instante.
                        let relayout_breaks = moved && matches!(crate::fsx::ext_of(&path).as_str(), "ttf" | "otf" | "ttc" | "fon" | "fnt");
                        if active.as_ref().is_some_and(|a| a.path == path) && !relayout_breaks {
                            // También tras cambiar de padre: otro monitor puede tener otra escala.
                            if let Some(a) = &active {
                                let _ = a.handler.SetRect(&rc);
                            }
                            if moved {
                                // Que todo lo de adentro se vuelva a pintar en su ventana nueva.
                                use windows::Win32::Graphics::Gdi::{RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow};
                                let _ = RedrawWindow(Some(container), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW);
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
                        #[cfg(test)]
                        tests::OPENS.fetch_add(1, Ordering::Relaxed);
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
        let shared: Arc<Shared> = Default::default();
        let (st, sh) = (status.clone(), shared.clone());
        let p = parent.0 as usize;
        std::thread::Builder::new()
            .name("vista-previa".into())
            .spawn(move || run(HWND(p as *mut _), rx, st, sh, wake))
            .ok();
        PreviewHost { tx, status, side: Mutex::new(Side::default()), shared, main: parent.0 as isize }
    }

    /// Lo que pidió la app, al día con el hilo: si el contenedor murió, ya está en la principal
    /// y oculto, sin visor.
    fn side(&self) -> parking_lot::MutexGuard<'_, Side> {
        let mut s = self.side.lock();
        let lost = self.shared.lost.load(Ordering::Acquire);
        if lost != s.lost_seen {
            s.lost_seen = lost;
            s.shown = false;
            s.parent = 0;
        }
        s
    }

    fn send_show(&self, s: &mut Side, parent: isize, path: &str, rect: [i32; 4]) {
        if s.shown && s.parent == parent && s.last.as_ref().is_some_and(|(p, r)| p == path && *r == rect) {
            return;
        }
        s.last = Some((path.to_string(), rect));
        s.shown = true;
        s.parent = parent;
        let _ = self.tx.send(Cmd::Show(path.to_string(), rect, parent));
    }

    /// Mostrar `path` en `rect` (píxeles) en el panel de la ventana principal. Solo envía algo si
    /// cambió. Mientras el contenedor está en la Vista rápida (hasta `home`), no hace nada.
    pub fn show(&self, path: &str, rect: [i32; 4]) {
        let mut s = self.side();
        if s.parent != 0 {
            return;
        }
        self.send_show(&mut s, 0, path, rect);
    }

    /// Como `show`, pero dentro de `parent` (la ventana de la Vista rápida): el contenedor
    /// cambia de padre sin recargar el visor. `rect` en píxeles del área cliente de `parent`
    /// (con la escala de esa ventana). Solo envía algo si cambió.
    pub fn show_in(&self, parent: HWND, path: &str, rect: [i32; 4]) {
        let p = parent.0 as isize;
        let p = if p == self.main { 0 } else { p };
        let mut s = self.side();
        self.send_show(&mut s, p, path, rect);
    }

    /// Ocultar y devolver el contenedor a la ventana principal (no bloquea: `SetParent` entre
    /// hilos manda mensajes al hilo de la interfaz). El visor queda cargado: si el panel muestra
    /// el mismo archivo, aparece sin recargar. Ver `is_home`. Si ya está en la principal no hace
    /// nada (ahí es del panel): se puede llamar en cada cuadro.
    pub fn home(&self) {
        let mut s = self.side();
        if s.parent == 0 {
            return;
        }
        s.parent = 0;
        s.shown = false;
        s.home_seq += 1;
        if self.tx.send(Cmd::Home(s.home_seq)).is_err() {
            // Sin hilo no hay a quién esperar.
            s.home_seq = self.shared.homed.load(Ordering::Acquire);
        }
    }

    /// ¿El contenedor ya está en la ventana principal? Falso desde un `show_in` hasta que el
    /// hilo atendió el `home`: la Vista rápida no se destruye antes (cierre en dos tiempos).
    pub fn is_home(&self) -> bool {
        let s = self.side();
        s.parent == 0 && self.shared.homed.load(Ordering::Acquire) >= s.home_seq
    }

    /// Ocultar el visor del panel. Mientras está en la Vista rápida, no hace nada.
    pub fn hide(&self) {
        let mut s = self.side();
        if s.shown && s.parent == 0 {
            s.shown = false;
            let _ = self.tx.send(Cmd::Hide);
        }
    }

    /// Descargar el visor del panel. Mientras está en la Vista rápida, no hace nada (para
    /// soltar un archivo, `release`).
    pub fn unload(&self) {
        let mut s = self.side();
        if s.parent != 0 {
            return;
        }
        s.shown = false;
        if s.last.take().is_some() {
            let _ = self.tx.send(Cmd::Unload);
        }
    }

    /// Si el visor muestra uno de estos archivos (o algo dentro de estas carpetas), lo
    /// suelta y espera a que el archivo quede libre: los visores de Office o PDF lo
    /// tienen abierto y no dejarían borrarlo, moverlo ni renombrarlo. Vale también en la
    /// Vista rápida.
    pub fn release(&self, paths: &[String]) {
        {
            let mut s = self.side();
            let hit = s.last.as_ref().is_some_and(|(p, _)| {
                let p = p.to_lowercase();
                paths.iter().any(|q| {
                    let q = q.to_lowercase();
                    p == q || p.starts_with(&format!("{}\\", q.trim_end_matches('\\')))
                })
            });
            if !hit {
                return;
            }
            s.last = None;
            s.shown = false;
        }
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
    use std::time::{Duration, Instant};

    /// Veces que se cargó un visor.
    pub(super) static OPENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    unsafe extern "system" fn def_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe { DefWindowProcW(h, m, wp, lp) }
    }

    /// Visible por sí mismo (las ventanas de prueba están ocultas).
    fn shown(c: HWND) -> bool {
        unsafe { GetWindowLongW(c, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 }
    }

    fn container_in(parent: HWND) -> Option<HWND> {
        unsafe { FindWindowExW(Some(parent), None, w!("RayoVistaPrevia"), PCWSTR::null()).ok() }
    }

    /// El contenedor pasa del panel a otra ventana y vuelve sin recargar el documento; si la
    /// otra ventana muere con él adentro, se recrea en la principal.
    #[test]
    fn moves_between_windows_without_reloading() {
        // `OPENS` es de todas las pruebas: las que cargan visores no corren a la vez.
        let _shell = crate::shell_test_lock();
        if handler_for("rtf").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join("rayo-prevhost-prueba");
        let _ = std::fs::create_dir_all(&dir);
        let rtf = dir.join("a.rtf");
        std::fs::write(&rtf, r"{\rtf1\ansi Hola, {\b mundo}.}").unwrap();
        cycle(&rtf.to_string_lossy());
        // `release` soltó el archivo: se puede borrar.
        std::fs::remove_file(&rtf).expect("el archivo sigue en uso");
    }

    /// Lo mismo con otros visores (Office, PDF…), a mano:
    /// `RAYO_PRUEBA_VISOR="C:\a.docx;C:\b.xlsx" cargo test --lib moves_other -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn moves_other_handlers() {
        let _shell = crate::shell_test_lock();
        let list = std::env::var("RAYO_PRUEBA_VISOR").unwrap_or_default();
        for p in list.split(';').filter(|p| !p.is_empty()) {
            let t = Instant::now();
            cycle(p);
            println!("{p}: bien ({:?})", t.elapsed());
        }
    }

    /// El visor de fuentes no se reacomoda al cambiar de ventana: se recarga al ir a la Vista
    /// rápida y también al volver (`Home` cambia de padre sin un `Show`; el `Show` siguiente del
    /// panel cuenta como cambio de ventana).
    #[test]
    fn font_handler_reloads_when_coming_home() {
        let _shell = crate::shell_test_lock();
        if handler_for("ttf").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join("rayo-prevhost-fuente");
        let _ = std::fs::create_dir_all(&dir);
        let ttf = dir.join("a.ttf");
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        if std::fs::copy(std::path::Path::new(&windir).join("Fonts").join("arial.ttf"), &ttf).is_err() {
            return;
        }
        let path = ttf.to_string_lossy().to_string();
        let (main, ql, _) = test_windows();
        let host = PreviewHost::new(main, Arc::new(|| {}));
        let opens = || OPENS.load(Ordering::Relaxed);
        host.show(&path, [0, 0, 200, 100]);
        assert!(until(20, || host.status_of(&path) == Some(Status::Ready)), "el visor de fuentes no cargó: {:?}", host.status_of(&path));
        let n0 = opens();
        host.show_in(ql, &path, [10, 10, 300, 200]);
        assert!(until(20, || opens() > n0 && host.status_of(&path) == Some(Status::Ready)), "no recargó al ir a la Vista rápida");
        let n1 = opens();
        host.home();
        assert!(until(5, || host.is_home()), "no volvió");
        host.show(&path, [0, 0, 200, 100]);
        assert!(until(20, || opens() > n1 && host.status_of(&path) == Some(Status::Ready)), "no recargó al volver al panel");
        // Otro `Show` en el panel, ya en casa: no recarga.
        let n2 = opens();
        host.show(&path, [0, 0, 220, 100]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(opens(), n2, "recargó sin cambiar de ventana");
        host.release(std::slice::from_ref(&path));
        unsafe {
            let _ = PostMessageW(Some(ql), WM_CLOSE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        let _ = std::fs::remove_file(&ttf);
    }

    /// Tres ventanas de prueba (principal, Vista rápida y otra), en un hilo con su bucle de
    /// mensajes (como la interfaz de la app).
    fn test_windows() -> (HWND, HWND, HWND) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || unsafe {
            let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW { lpfnWndProc: Some(def_proc), hInstance: inst.into(), lpszClassName: w!("RayoPruebaVentana"), ..Default::default() };
            RegisterClassW(&wc);
            let mk = || CreateWindowExW(WINDOW_EX_STYLE(0), w!("RayoPruebaVentana"), w!(""), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, 0, 0, 400, 300, None, None, Some(inst.into()), None).unwrap();
            let (main, ql, ql2) = (mk(), mk(), mk());
            let _ = tx.send((main.0 as isize, ql.0 as isize, ql2.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        let (main, ql, ql2) = rx.recv().unwrap();
        (HWND(main as *mut _), HWND(ql as *mut _), HWND(ql2 as *mut _))
    }

    fn cycle(path: &str) {
        let path = path.to_string();
        // Las ventanas, en un hilo con su bucle de mensajes (como la interfaz de la app).
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || unsafe {
            let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap();
            let wc = WNDCLASSW { lpfnWndProc: Some(def_proc), hInstance: inst.into(), lpszClassName: w!("RayoPruebaVentana"), ..Default::default() };
            RegisterClassW(&wc);
            let mk = || CreateWindowExW(WINDOW_EX_STYLE(0), w!("RayoPruebaVentana"), w!(""), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, 0, 0, 400, 300, None, None, Some(inst.into()), None).unwrap();
            let (main, ql, ql2) = (mk(), mk(), mk());
            let _ = tx.send((main.0 as isize, ql.0 as isize, ql2.0 as isize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        let (main, ql, ql2) = rx.recv().unwrap();
        let (main, ql, ql2) = (HWND(main as *mut _), HWND(ql as *mut _), HWND(ql2 as *mut _));
        let host = PreviewHost::new(main, Arc::new(|| {}));
        let parent = |c: HWND| unsafe { GetParent(c).unwrap_or_default() };

        // En el panel.
        host.show(&path, [0, 0, 200, 100]);
        assert!(until(20, || host.status_of(&path) == Some(Status::Ready)), "el visor RTF no cargó: {:?}", host.status_of(&path));
        let opens = OPENS.load(Ordering::Relaxed);
        let c = container_in(main).expect("contenedor");

        // A la Vista rápida: cambia de padre, no recarga.
        host.show_in(ql, &path, [10, 10, 300, 200]);
        assert!(!host.is_home());
        assert!(until(5, || parent(c) == ql), "no cambió de padre");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(OPENS.load(Ordering::Relaxed), opens, "recargó al cambiar de ventana");
        assert!(shown(c));
        // Mientras está afuera, el panel no se lo lleva.
        host.show(&path, [0, 0, 200, 100]);
        host.hide();
        host.unload();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(parent(c), ql);
        assert!(shown(c));

        // De vuelta: oculto, en la principal, y el panel lo muestra sin recargar.
        host.home();
        assert!(until(5, || host.is_home()), "no volvió");
        assert_eq!(parent(c), main);
        assert!(!shown(c));
        host.show(&path, [0, 0, 200, 100]);
        assert!(until(5, || shown(c)));
        assert_eq!(OPENS.load(Ordering::Relaxed), opens, "recargó al volver");

        // La otra ventana muere con el contenedor adentro: se recrea en la principal.
        host.show_in(ql2, &path, [0, 0, 300, 200]);
        assert!(until(5, || parent(c) == ql2));
        unsafe {
            let _ = PostMessageW(Some(ql2), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        assert!(until(5, || unsafe { !IsWindow(Some(c)).as_bool() }), "el contenedor no murió");
        assert!(until(5, || host.is_home() && container_in(main).is_some()), "no se recreó");
        host.show(&path, [0, 0, 200, 100]);
        assert!(until(20, || OPENS.load(Ordering::Relaxed) == opens + 1 && host.status_of(&path) == Some(Status::Ready)), "no volvió a cargar");
        let c2 = container_in(main).unwrap();
        assert!(shown(c2));
        host.release(std::slice::from_ref(&path));
        unsafe {
            let _ = PostMessageW(Some(ql), WM_CLOSE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

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
