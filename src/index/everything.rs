//! Cliente del IPC de Everything (voidtools), el mismo protocolo que usa su SDK:
//! se envía la consulta con WM_COPYDATA a la ventana oculta de Everything y la
//! respuesta llega por WM_COPYDATA a una ventana propia.

use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const COPYDATA_QUERY2W: usize = 18;
const REPLY_ID: u32 = 0x5241_594F;
const REQUEST_FILE_NAME: u32 = 0x1;
const REQUEST_PATH: u32 = 0x2;
const SORT_NAME_ASCENDING: u32 = 1;

thread_local! {
    static REPLY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let cds = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
        if cds.dwData == REPLY_ID as usize {
            let data = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }.to_vec();
            REPLY.with(|r| *r.borrow_mut() = Some(data));
            return LRESULT(1);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub struct Everything {
    reply: HWND,
}

pub struct EvResult {
    pub total: usize,
    pub paths: Vec<String>,
    pub elapsed: Duration,
}

fn everything_window() -> Option<HWND> {
    unsafe { FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()) }.ok().filter(|h| !h.is_invalid())
}

impl Everything {
    /// Conecta si Everything está corriendo. Se debe usar desde un solo hilo.
    pub fn connect() -> Option<Self> {
        everything_window()?;
        unsafe {
            let inst = GetModuleHandleW(None).ok()?;
            let class = w!("RayoEverythingReply");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let reply = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).ok()?;
            // Si este proceso corre como administrador y Everything no, Windows (UIPI)
            // bloquea sus respuestas: hay que permitir WM_COPYDATA explícitamente.
            let _ = ChangeWindowMessageFilterEx(reply, WM_COPYDATA, MSGFLT_ALLOW, None);
            Some(Everything { reply })
        }
    }

    pub fn query(&self, search: &str, max_results: u32) -> Option<EvResult> {
        let ev = everything_window()?;
        let mut buf = Vec::with_capacity(28 + search.len() * 2 + 2);
        for v in [self.reply.0 as usize as u32, REPLY_ID, 0, 0, max_results, REQUEST_FILE_NAME | REQUEST_PATH, SORT_NAME_ASCENDING] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for u in search.encode_utf16().chain([0]) {
            buf.extend_from_slice(&u.to_le_bytes());
        }
        let cds = COPYDATASTRUCT { dwData: COPYDATA_QUERY2W, cbData: buf.len() as u32, lpData: buf.as_mut_ptr() as *mut _ };
        REPLY.with(|r| *r.borrow_mut() = None);
        let t = Instant::now();
        unsafe {
            SendMessageW(ev, WM_COPYDATA, Some(WPARAM(self.reply.0 as usize)), Some(LPARAM(&cds as *const _ as isize)));
        }
        // Esperar la respuesta procesando mensajes, con tiempo límite.
        let mut msg = MSG::default();
        loop {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if let Some(data) = REPLY.with(|r| r.borrow_mut().take()) {
                let elapsed = t.elapsed();
                return Some(parse_list2(&data, elapsed));
            }
            if t.elapsed() > Duration::from_secs(3) {
                return None;
            }
            unsafe {
                MsgWaitForMultipleObjects(None, false, 20, QS_ALLINPUT);
            }
        }
    }
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}

/// EVERYTHING_IPC_LIST2 { totitems, numitems, offset, request_flags, sort_type, items[] }
/// Cada item: { flags, data_offset }; los datos piden nombre y ruta con su largo en u32.
fn parse_list2(b: &[u8], elapsed: Duration) -> EvResult {
    let total = u32_at(b, 0) as usize;
    let num = u32_at(b, 4) as usize;
    let mut paths = Vec::with_capacity(num);
    for i in 0..num {
        let off = u32_at(b, 20 + i * 8 + 4) as usize;
        let mut p = off;
        let mut read_str = || {
            let len = u32_at(b, p) as usize;
            p += 4;
            let units: Vec<u16> = (0..len).map(|k| u16::from_le_bytes([b[p + k * 2], b[p + k * 2 + 1]])).collect();
            p += len * 2 + 2;
            String::from_utf16_lossy(&units)
        };
        let name = read_str();
        let path = read_str();
        paths.push(format!("{path}\\{name}"));
    }
    EvResult { total, paths, elapsed }
}
