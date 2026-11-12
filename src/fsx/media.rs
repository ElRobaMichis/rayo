//! Audio y video para la Vista rápida, con Media Foundation (MFPlay), en un hilo propio con
//! su bucle de mensajes (como `prevhost`). El video va en una ventana hija que se le presta.
//!
//! Un reproductor MFPlay por archivo: abrir crea uno, cerrar lo apaga (`Shutdown`) y el archivo
//! queda libre. Los avisos de MFPlay llegan por un objeto COM propio (`Callback`, hecho a mano:
//! el repositorio no usa `#[implement]`) en este mismo hilo, como mensajes de ventana.
//!
//! La ventana del video vive siempre (oculta) como hija de la principal; se muda a la ventana
//! que pida `place_video` y vuelve a casa al cerrar (cierre en dos tiempos, `is_closed`).
// El permiso se va al integrar (hasta entonces solo lo usa la tarjeta vieja).
#![allow(dead_code)]

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, FillRect, GetStockObject, BLACK_BRUSH, HBRUSH, PAINTSTRUCT};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::Variant::{VT_I8, VT_UI8};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, w};

/// En qué anda el reproductor.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Phase {
    #[default]
    Idle,
    Opening,
    Paused,
    Playing,
    Ended,
    /// Sin Media Foundation o sin códec: póster y «Abrir con».
    Failed,
}

/// Lo que la interfaz lee en cada cuadro.
#[derive(Clone, Default, Debug)]
pub struct State {
    pub path: String,
    pub phase: Phase,
    pub pos_ms: u64,
    pub dur_ms: u64,
    /// Tamaño del cuadro, si tiene video.
    pub video: Option<(u32, u32)>,
    /// Clic sobre la ventana del video (el hilo ya alternó reproducir / pausa). `state()` lo
    /// entrega una vez y lo apaga.
    pub clicked: bool,
}

#[derive(Debug)]
enum Cmd {
    Open { path: String, autoplay: bool, volume: f32, muted: bool },
    Play,
    Pause,
    Seek(u64),
    Volume(f32, bool),
    /// Padre (como número) y rectángulo en píxeles de su área cliente; None = oculta.
    Place(usize, Option<[i32; 4]>),
    Close,
    Release(Vec<String>, Sender<()>),
}

struct Shared {
    state: Mutex<State>,
    /// Hay algo abierto o la ventana del video está fuera de casa (ver `is_closed`).
    busy: AtomicBool,
    /// Hilo «medios», para despertarlo con un mensaje (0 = todavía no arrancó).
    tid: AtomicU32,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock());
    }
    /// Cambio de fase: se despierta a la interfaz (la posición no despierta a nadie).
    fn phase(&self, ph: Phase) {
        let changed = {
            let mut s = self.state.lock();
            let c = s.phase != ph;
            s.phase = ph;
            c
        };
        if changed {
            (self.wake)();
        }
    }
}

/// Reproductor: órdenes por canal al hilo «medios»; el estado, en un `Mutex`.
pub struct Player {
    tx: Sender<Cmd>,
    sh: Arc<Shared>,
    /// Último `place_video` enviado: solo se manda lo que cambia.
    placed: Mutex<Option<(usize, Option<[i32; 4]>)>>,
}

impl Player {
    pub fn new(main: HWND, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = unbounded();
        let sh = Arc::new(Shared { state: Mutex::new(State::default()), busy: AtomicBool::new(false), tid: AtomicU32::new(0), wake });
        let s2 = sh.clone();
        let m = main.0 as usize;
        std::thread::Builder::new().name("medios".into()).spawn(move || run(m, rx, s2)).ok();
        Player { tx, sh, placed: Mutex::new(None) }
    }

    fn send(&self, c: Cmd) {
        if self.tx.send(c).is_ok() {
            let tid = self.sh.tid.load(Ordering::Acquire);
            if tid != 0 {
                unsafe {
                    let _ = PostThreadMessageW(tid, WM_NULL, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    /// Abre `path` (tras la demora de 120 ms la pone quien llama); suena solo si `autoplay`.
    pub fn open(&self, path: &str, autoplay: bool, volume: f32, muted: bool) {
        self.sh.busy.store(true, Ordering::Release);
        self.sh.set(|s| *s = State { path: path.to_string(), phase: Phase::Opening, ..Default::default() });
        self.send(Cmd::Open { path: path.to_string(), autoplay, volume: volume.clamp(0.0, 1.0), muted });
    }

    /// Reproduce; desde el final, vuelve a empezar. Sin nada abierto, no hace nada.
    pub fn play(&self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    /// Salta a `ms` (se publica enseguida; varios seguidos se juntan).
    pub fn seek(&self, ms: u64) {
        self.sh.set(|s| {
            if s.dur_ms > 0 {
                s.pos_ms = ms.min(s.dur_ms);
            }
        });
        self.send(Cmd::Seek(ms));
    }

    pub fn set_volume(&self, v: f32, muted: bool) {
        self.send(Cmd::Volume(v.clamp(0.0, 1.0), muted));
    }

    /// Ventana hija del video dentro de `parent`, en píxeles de su área cliente; None la oculta.
    /// Se puede llamar en cada cuadro: solo se manda lo que cambió.
    pub fn place_video(&self, parent: HWND, rect: Option<[i32; 4]>) {
        let key = (parent.0 as usize, rect);
        let mut last = self.placed.lock();
        if *last == Some(key) {
            return;
        }
        *last = Some(key);
        drop(last);
        if rect.is_some() {
            self.sh.busy.store(true, Ordering::Release);
        }
        self.send(Cmd::Place(key.0, rect));
    }

    /// Corta y suelta el archivo (no bloquea). Ver `is_closed`.
    pub fn close(&self) {
        // La ventana vuelve a casa: el próximo `place_video` se manda aunque repita el lugar.
        *self.placed.lock() = None;
        self.sh.set(|s| *s = State::default());
        self.send(Cmd::Close);
    }

    /// ¿Ya no hay nada abierto ni ventana de video? (para el cierre en dos tiempos)
    pub fn is_closed(&self) -> bool {
        !self.sh.busy.load(Ordering::Acquire)
    }

    /// Como `PreviewHost::release`: si suena uno de estos archivos (o algo dentro de estas
    /// carpetas), lo suelta y espera hasta 1,5 s a que quede libre.
    pub fn release(&self, paths: &[String]) {
        let cur = self.sh.state.lock().path.to_lowercase();
        if cur.is_empty() || !paths.iter().any(|q| inside(&cur, q)) {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.send(Cmd::Release(paths.to_vec(), tx));
        let _ = rx.recv_timeout(Duration::from_millis(1500));
    }

    /// Lo que hay ahora. `clicked` se entrega una vez.
    pub fn state(&self) -> State {
        let mut s = self.sh.state.lock();
        let out = s.clone();
        s.clicked = false;
        out
    }
}

/// ¿`cur` (en minúsculas) es `q` o está dentro de la carpeta `q`?
fn inside(cur: &str, q: &str) -> bool {
    let q = q.to_lowercase();
    cur == q || cur.starts_with(&format!("{}\\", q.trim_end_matches('\\')))
}

// ---------------------------------------------------------------- avisos de MFPlay

/// Un aviso de MFPlay, ya copiado (el encabezado solo vale durante la llamada).
struct Ev {
    era: u64,
    kind: MFP_EVENT_TYPE,
    hr: HRESULT,
    item: Option<IMFPMediaItem>,
}

thread_local! {
    /// Avisos recibidos en este hilo, a atender después de despachar los mensajes.
    static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    /// El reproductor actual, para `UpdateVideo` desde el procedimiento de la ventana.
    static CURRENT: RefCell<Option<IMFPMediaPlayer>> = const { RefCell::new(None) };
    /// Clic sobre el video, pendiente de atender en el bucle.
    static CLICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `IMFPMediaPlayerCallback` a mano: puntero a la tabla, cuenta de referencias y generación
/// (avisos de un reproductor ya apagado se descartan).
#[repr(C)]
struct Callback {
    vtbl: *const IMFPMediaPlayerCallback_Vtbl,
    refs: AtomicU32,
    era: u64,
}

static CALLBACK_VTBL: IMFPMediaPlayerCallback_Vtbl = IMFPMediaPlayerCallback_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: cb_query, AddRef: cb_add_ref, Release: cb_release },
    OnMediaPlayerEvent: cb_event,
};

unsafe extern "system" fn cb_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        if out.is_null() || iid.is_null() {
            return windows::Win32::Foundation::E_POINTER;
        }
        if *iid == IUnknown::IID || *iid == IMFPMediaPlayerCallback::IID {
            cb_add_ref(this);
            *out = this;
            return HRESULT(0);
        }
        *out = std::ptr::null_mut();
        windows::Win32::Foundation::E_NOINTERFACE
    }
}

unsafe extern "system" fn cb_add_ref(this: *mut c_void) -> u32 {
    unsafe { (*(this as *const Callback)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn cb_release(this: *mut c_void) -> u32 {
    unsafe {
        let n = (*(this as *const Callback)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            drop(Box::from_raw(this as *mut Callback));
        }
        n
    }
}

unsafe extern "system" fn cb_event(this: *mut c_void, h: *const MFP_EVENT_HEADER) {
    unsafe {
        if h.is_null() {
            return;
        }
        let era = (*(this as *const Callback)).era;
        let kind = (*h).eEventType;
        let item = match kind {
            MFP_EVENT_TYPE_MEDIAITEM_CREATED => (*(h as *const MFP_MEDIAITEM_CREATED_EVENT)).pMediaItem.as_ref().cloned(),
            MFP_EVENT_TYPE_MEDIAITEM_SET => (*(h as *const MFP_MEDIAITEM_SET_EVENT)).pMediaItem.as_ref().cloned(),
            _ => None,
        };
        let ev = Ev { era, kind, hr: (*h).hrEvent, item };
        let _ = EVENTS.try_with(|q| q.borrow_mut().push(ev));
    }
}

fn new_callback(era: u64) -> IMFPMediaPlayerCallback {
    let b = Box::new(Callback { vtbl: &CALLBACK_VTBL, refs: AtomicU32::new(1), era });
    unsafe { IMFPMediaPlayerCallback::from_raw(Box::into_raw(b) as *mut c_void) }
}

// ---------------------------------------------------------------- ventana del video

unsafe extern "system" fn video_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                CLICK.set(true);
                return LRESULT(0);
            }
            WM_SIZE => {
                CURRENT.with_borrow(|p| p.as_ref().map(|p| p.UpdateVideo()));
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let has = CURRENT.with_borrow(|p| p.as_ref().is_some_and(|p| p.UpdateVideo().is_ok()));
                if !has {
                    FillRect(dc, &ps.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                }
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

// ---------------------------------------------------------------- el hilo

/// Lo abierto ahora.
struct Session {
    path: String,
    player: IMFPMediaPlayer,
    era: u64,
    /// Reproducir en cuanto el archivo esté listo (o al volver de un salto).
    want_play: bool,
    volume: f32,
    muted: bool,
    /// MFPlay ya tiene el archivo puesto (`MEDIAITEM_SET`).
    ready: bool,
    has_video: bool,
    /// Ya hubo un cuadro (primer `Playing`): la ventana se muestra.
    shown: bool,
    /// Salto en curso (hasta `POSITION_SET`) y el siguiente pendiente.
    seeking: Option<Instant>,
    next_seek: Option<u64>,
}

struct Thread {
    main: HWND,
    video: Option<HWND>,
    parent: HWND,
    rect: Option<[i32; 4]>,
    /// Lo que la ventana muestra ahora (para no repetir `SetWindowPos`).
    shown_at: Option<[i32; 4]>,
    cur: Option<Session>,
    era: u64,
    sh: Arc<Shared>,
}

fn hns(ms: u64) -> PROPVARIANT {
    let mut v = PROPVARIANT::default();
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = (ms as i64).saturating_mul(10_000);
    }
    v
}

fn ms_of(v: &PROPVARIANT) -> Option<u64> {
    unsafe {
        let inner = &*v.Anonymous.Anonymous;
        // La posición viene como VT_I8; la duración, como VT_UI8.
        match inner.vt {
            VT_I8 => Some((inner.Anonymous.hVal.max(0) / 10_000) as u64),
            VT_UI8 => Some(inner.Anonymous.uhVal / 10_000),
            _ => None,
        }
    }
}

impl Thread {
    fn shutdown(&mut self) {
        if let Some(s) = self.cur.take() {
            unsafe {
                let _ = s.player.Shutdown();
            }
        }
        CURRENT.set(None);
        EVENTS.with_borrow_mut(|q| q.clear());
        self.sync_window();
    }

    fn fail(&mut self) {
        self.shutdown();
        self.sh.phase(Phase::Failed);
    }

    fn open(&mut self, path: String, autoplay: bool, volume: f32, muted: bool) {
        self.shutdown();
        self.era += 1;
        let era = self.era;
        let cb = new_callback(era);
        let mut player: Option<IMFPMediaPlayer> = None;
        let made = unsafe { MFPCreateMediaPlayer(PCWSTR::null(), false, MFP_OPTION_NONE, &cb, self.video, Some(&mut player)) };
        let Some(player) = made.ok().and(player) else {
            return self.fail();
        };
        // Asíncrono: el hilo sigue atendiendo órdenes mientras abre (un archivo de red tarda).
        if unsafe { player.CreateMediaItemFromURL(&HSTRING::from(path.as_str()), false, era as usize, None) }.is_err() {
            unsafe {
                let _ = player.Shutdown();
            }
            return self.fail();
        }
        CURRENT.set(Some(player.clone()));
        self.cur = Some(Session { path, player, era, want_play: autoplay, volume, muted, ready: false, has_video: false, shown: false, seeking: None, next_seek: None });
    }

    fn play(&mut self) {
        let Some(s) = self.cur.as_mut() else { return };
        s.want_play = true;
        if !s.ready {
            return;
        }
        unsafe {
            if self.sh.state.lock().phase == Phase::Ended {
                let _ = s.player.SetPosition(&MFP_POSITIONTYPE_100NS, &hns(0));
                self.sh.set(|st| st.pos_ms = 0);
            }
            let _ = s.player.Play();
        }
    }

    fn pause(&mut self) {
        let Some(s) = self.cur.as_mut() else { return };
        s.want_play = false;
        if s.ready {
            unsafe {
                let _ = s.player.Pause();
            }
        }
    }

    fn seek(&mut self, ms: u64) {
        let Some(s) = self.cur.as_mut() else { return };
        // Sin abrir del todo, o con un salto en curso: queda pendiente (cuenta el último).
        if !s.ready || s.seeking.is_some_and(|t| t.elapsed() < Duration::from_millis(1000)) {
            s.next_seek = Some(ms);
            return;
        }
        s.next_seek = None;
        s.seeking = Some(Instant::now());
        unsafe {
            let _ = s.player.SetPosition(&MFP_POSITIONTYPE_100NS, &hns(ms));
        }
        // Desde el final: queda en pausa ahí (reproducir sigue desde el salto).
        let ended = self.sh.state.lock().phase == Phase::Ended;
        if ended {
            self.sh.phase(Phase::Paused);
        }
    }

    fn volume(&mut self, v: f32, muted: bool) {
        let Some(s) = self.cur.as_mut() else { return };
        s.volume = v;
        s.muted = muted;
        unsafe {
            let _ = s.player.SetVolume(v);
            let _ = s.player.SetMute(muted);
        }
    }

    fn place(&mut self, parent: usize, rect: Option<[i32; 4]>) {
        let parent = HWND(parent as *mut c_void);
        // Solo se muda para mostrarse: un «ocultar» después de `close` no la saca de casa.
        if let Some(v) = self.video
            && rect.is_some()
            && parent != self.parent
            && !parent.is_invalid()
        {
            unsafe {
                let _ = ShowWindow(v, SW_HIDE);
                let _ = SetParent(v, Some(parent));
            }
            self.parent = parent;
            self.shown_at = None;
        }
        self.rect = rect;
        self.sync_window();
    }

    /// Muestra u oculta la ventana del video según lo que hay.
    fn sync_window(&mut self) {
        let Some(v) = self.video else { return };
        let want = self.rect.filter(|_| self.cur.as_ref().is_some_and(|s| s.has_video && s.shown));
        if want == self.shown_at {
            return;
        }
        unsafe {
            match want {
                Some(r) => {
                    let _ = SetWindowPos(v, Some(HWND_TOP), r[0], r[1], r[2].max(1), r[3].max(1), SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    CURRENT.with_borrow(|p| p.as_ref().map(|p| p.UpdateVideo()));
                }
                None => {
                    let _ = ShowWindow(v, SW_HIDE);
                }
            }
        }
        self.shown_at = want;
    }

    /// Cerrar: corta, suelta, oculta y devuelve la ventana a la principal.
    fn close(&mut self) {
        self.shutdown();
        self.rect = None;
        self.sync_window();
        if let Some(v) = self.video
            && self.parent != self.main
        {
            unsafe {
                let _ = SetParent(v, Some(self.main));
            }
            self.parent = self.main;
            self.shown_at = None;
        }
        self.sh.set(|s| *s = State::default());
    }

    fn event(&mut self, ev: Ev) {
        let Some(s) = self.cur.as_mut() else { return };
        if ev.era != s.era {
            return;
        }
        match ev.kind {
            MFP_EVENT_TYPE_MEDIAITEM_CREATED => {
                let Some(item) = ev.item.filter(|_| ev.hr.is_ok()) else { return self.fail() };
                unsafe {
                    let (mut hv, mut sel) = (windows::core::BOOL(0), windows::core::BOOL(0));
                    let _ = item.HasVideo(Some(&mut hv), Some(&mut sel));
                    s.has_video = hv.as_bool() && sel.as_bool();
                    let dur = item.GetDuration(&MFP_POSITIONTYPE_100NS).ok().and_then(|v| ms_of(&v)).unwrap_or(0);
                    self.sh.set(|st| st.dur_ms = dur);
                    if s.player.SetMediaItem(&item).is_err() {
                        self.fail();
                    }
                }
            }
            MFP_EVENT_TYPE_MEDIAITEM_SET => {
                if ev.hr.is_err() {
                    return self.fail();
                }
                s.ready = true;
                unsafe {
                    let _ = s.player.SetVolume(s.volume);
                    let _ = s.player.SetMute(s.muted);
                    if let Some(ms) = s.next_seek.take() {
                        let _ = s.player.SetPosition(&MFP_POSITIONTYPE_100NS, &hns(ms));
                    }
                    let dur = s.player.GetDuration(&MFP_POSITIONTYPE_100NS).ok().and_then(|v| ms_of(&v));
                    let mut size = windows::Win32::Foundation::SIZE::default();
                    let video = (s.has_video && s.player.GetNativeVideoSize(Some(&mut size), None).is_ok() && size.cx > 0 && size.cy > 0).then_some((size.cx as u32, size.cy as u32));
                    self.sh.set(|st| {
                        if let Some(d) = dur {
                            st.dur_ms = d;
                        }
                        st.video = video;
                    });
                    if s.want_play {
                        let _ = s.player.Play();
                    } else {
                        self.sh.phase(Phase::Paused);
                    }
                }
            }
            MFP_EVENT_TYPE_PLAY => {
                if ev.hr.is_err() {
                    return self.fail();
                }
                s.shown = true;
                self.sh.phase(Phase::Playing);
                self.sync_window();
            }
            MFP_EVENT_TYPE_PAUSE | MFP_EVENT_TYPE_STOP => {
                if ev.hr.is_ok() && self.sh.state.lock().phase != Phase::Ended {
                    self.sh.phase(Phase::Paused);
                }
            }
            MFP_EVENT_TYPE_POSITION_SET => {
                s.seeking = None;
                if let Some(ms) = s.next_seek.take() {
                    self.seek(ms);
                }
            }
            MFP_EVENT_TYPE_PLAYBACK_ENDED => {
                s.want_play = false;
                self.sh.set(|st| st.pos_ms = st.dur_ms);
                self.sh.phase(Phase::Ended);
            }
            MFP_EVENT_TYPE_ERROR => self.fail(),
            _ => {}
        }
    }

    /// Publica la posición (sin despertar a nadie).
    fn poll(&mut self) {
        let Some(s) = self.cur.as_ref().filter(|s| s.ready && s.seeking.is_none()) else { return };
        let phase = self.sh.state.lock().phase;
        if phase != Phase::Playing && phase != Phase::Paused {
            return;
        }
        if let Some(ms) = unsafe { s.player.GetPosition(&MFP_POSITIONTYPE_100NS) }.ok().and_then(|v| ms_of(&v)) {
            self.sh.set(|st| st.pos_ms = if st.dur_ms > 0 { ms.min(st.dur_ms) } else { ms });
        }
    }

    fn click(&mut self) {
        let playing = self.sh.state.lock().phase == Phase::Playing;
        if playing {
            self.pause();
        } else {
            self.play();
        }
        self.sh.set(|s| s.clicked = true);
        (self.sh.wake)();
    }

    fn busy(&self) -> bool {
        self.cur.is_some() || self.parent != self.main || self.shown_at.is_some()
    }
}

fn run(main: usize, rx: Receiver<Cmd>, sh: Arc<Shared>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
        // La cola de mensajes existe desde el primer `PeekMessage`: recién ahí se puede despertar.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        sh.tid.store(windows::Win32::System::Threading::GetCurrentThreadId(), Ordering::Release);
        let main = HWND(main as *mut c_void);
        let video = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok().and_then(|inst| {
            let class = w!("RayoVideo");
            let wc = WNDCLASSW { lpfnWndProc: Some(video_proc), hInstance: inst.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
            CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WS_CHILD | WS_CLIPSIBLINGS, 0, 0, 16, 16, Some(main), None, Some(inst.into()), None).ok()
        });
        let mut th = Thread { main, video, parent: main, rect: None, shown_at: None, cur: None, era: 0, sh: sh.clone() };
        loop {
            // Sonando: se despierta cada 50 ms para publicar la posición; si no, solo con mensajes.
            let playing = sh.state.lock().phase == Phase::Playing;
            MsgWaitForMultipleObjectsEx(None, if playing { 50 } else { INFINITE }, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let mut cmds: Vec<Cmd> = Vec::new();
            let mut gone = false;
            loop {
                match rx.try_recv() {
                    Ok(c) => cmds.push(c),
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(_) => {
                        gone = true;
                        break;
                    }
                }
            }
            // De varios saltos o lugares seguidos, solo cuenta el último.
            let last_seek = cmds.iter().rposition(|c| matches!(c, Cmd::Seek(..)));
            let last_place = cmds.iter().rposition(|c| matches!(c, Cmd::Place(..)));
            for (k, c) in cmds.into_iter().enumerate() {
                match c {
                    Cmd::Open { path, autoplay, volume, muted } => th.open(path, autoplay, volume, muted),
                    Cmd::Play => th.play(),
                    Cmd::Pause => th.pause(),
                    Cmd::Seek(ms) if Some(k) == last_seek => th.seek(ms),
                    Cmd::Seek(_) => {}
                    Cmd::Volume(v, m) => th.volume(v, m),
                    Cmd::Place(p, r) if Some(k) == last_place => th.place(p, r),
                    Cmd::Place(..) => {}
                    Cmd::Close => th.close(),
                    Cmd::Release(paths, done) => {
                        let hit = th.cur.as_ref().is_some_and(|s| {
                            let cur = s.path.to_lowercase();
                            paths.iter().any(|q| inside(&cur, q))
                        });
                        if hit {
                            th.shutdown();
                            sh.set(|s| *s = State::default());
                            (sh.wake)();
                        }
                        let _ = done.send(());
                    }
                }
            }
            for ev in EVENTS.with_borrow_mut(std::mem::take) {
                th.event(ev);
            }
            if CLICK.replace(false) {
                th.click();
            }
            // Un salto que nunca avisó: no se queda trabado.
            if let Some(s) = th.cur.as_mut()
                && s.seeking.is_some_and(|t| t.elapsed() > Duration::from_millis(1000))
            {
                s.seeking = None;
                if let Some(ms) = s.next_seek.take() {
                    th.seek(ms);
                }
            }
            th.poll();
            sh.busy.store(th.busy() || !rx.is_empty(), Ordering::Release);
            if gone {
                th.shutdown();
                if let Some(v) = th.video {
                    let _ = DestroyWindow(v);
                }
                let _ = MFShutdown();
                return;
            }
        }
    }
}
