//! Quién está usando un archivo: el Administrador de reinicio de Windows (Restart
//! Manager) dice qué procesos lo tienen abierto, sin permisos de administrador.
//! Para una carpeta se preguntan los archivos de adentro.

use crate::index::walk::wide;
use windows::Win32::Foundation::{CloseHandle, ERROR_MORE_DATA, FILETIME, HWND, LPARAM, WPARAM};
use windows::Win32::System::RestartManager::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, PWSTR};

/// Cuántos archivos de una carpeta se revisan como mucho.
const MAX_FILES: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Programa con ventana: se le puede pedir que se cierre.
    Window,
    Explorer,
    Service,
    Console,
    /// Parte de Windows: no se ofrece cerrarlo.
    Critical,
    Other,
}

#[derive(Clone, Debug)]
pub struct Locker {
    pub pid: u32,
    /// Nombre para mostrar ("Microsoft Word").
    pub name: String,
    /// Ejecutable ("WINWORD.EXE"), si se pudo leer.
    pub exe: String,
    pub kind: Kind,
    /// Es este mismo Rayo (la vista previa, una miniatura…).
    pub is_self: bool,
    start: FILETIME,
}

/// ¿El error es "otro proceso lo está usando"? (32 = uso compartido, 33 = bloqueo).
pub fn is_in_use_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(32 | 33))
}

/// Archivos a registrar: los mismos, o los de adentro de cada carpeta.
fn files_of(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    for p in paths {
        if std::path::Path::new(&super::long_path(p)).is_dir() {
            stack.push(p.clone());
        } else {
            out.push(p.clone());
        }
    }
    while let Some(d) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(rd) = std::fs::read_dir(super::long_path(&d)) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = super::join(&d, &e.file_name().to_string_lossy());
            if ft.is_dir() && !ft.is_symlink() {
                stack.push(p);
            } else if ft.is_file() {
                out.push(p);
            }
        }
    }
    out.truncate(MAX_FILES);
    out
}

/// Procesos que tienen abierto alguno de estos archivos (o algo dentro de estas carpetas).
pub fn who_uses(paths: &[String]) -> Result<Vec<Locker>, String> {
    let files = files_of(paths);
    if files.is_empty() {
        return Ok(Vec::new());
    }
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        let r = RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr()));
        if r.0 != 0 {
            return Err(tf!("uso.no_pudo_consultar_windows_error", codigo = r.0));
        }
        let result = (|| {
            let wides: Vec<Vec<u16>> = files.iter().map(|f| wide(f)).collect();
            let ptrs: Vec<PCWSTR> = wides.iter().map(|w| PCWSTR(w.as_ptr())).collect();
            // De a tandas: listas enormes hacen fallar el registro.
            for chunk in ptrs.chunks(500) {
                let r = RmRegisterResources(session, Some(chunk), None, None);
                if r.0 != 0 {
                    return Err(tf!("uso.no_pudo_consultar_windows_error", codigo = r.0));
                }
            }
            let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
            for _ in 0..4 {
                let (mut needed, mut n, mut reasons) = (0u32, infos.len() as u32, 0u32);
                let ptr = if infos.is_empty() { None } else { Some(infos.as_mut_ptr()) };
                let r = RmGetList(session, &mut needed, &mut n, ptr, &mut reasons);
                if r == ERROR_MORE_DATA {
                    infos = vec![RM_PROCESS_INFO::default(); needed as usize + 2];
                    continue;
                }
                if r.0 != 0 {
                    return Err(tf!("uso.no_pudo_consultar_windows_error", codigo = r.0));
                }
                infos.truncate(n as usize);
                break;
            }
            let me = std::process::id();
            let mut out: Vec<Locker> = infos
                .iter()
                .map(|i| {
                    let pid = i.Process.dwProcessId;
                    let name = String::from_utf16_lossy(&i.strAppName).trim_end_matches('\0').to_string();
                    // RM_APP_TYPE: 1 y 2 ventanas, 3 servicio, 4 Explorador, 5 consola, 1000 crítico.
                    let kind = match i.ApplicationType.0 {
                        1 | 2 => Kind::Window,
                        3 => Kind::Service,
                        4 => Kind::Explorer,
                        5 => Kind::Console,
                        1000 => Kind::Critical,
                        _ => Kind::Other,
                    };
                    let exe = exe_name(pid);
                    let name = if name.is_empty() { exe.clone() } else { name };
                    Locker { pid, name, exe, kind, is_self: pid == me, start: i.Process.ProcessStartTime }
                })
                .collect();
            out.sort_by_key(|l| (l.is_self, l.name.to_lowercase()));
            out.dedup_by_key(|l| l.pid);
            Ok(out)
        })();
        let _ = RmEndSession(session);
        result
    }
}

fn exe_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..n as usize]);
        super::file_name(&full)
    }
}

/// Ventanas principales (visibles, sin dueño) de un proceso.
fn windows_of(pid: u32) -> Vec<HWND> {
    struct Ctx {
        pid: u32,
        out: Vec<HWND>,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let cx = unsafe { &mut *(lp.0 as *mut Ctx) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        let owned = unsafe { GetWindow(h, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
        if pid == cx.pid && unsafe { IsWindowVisible(h) }.as_bool() && !owned {
            cx.out.push(h);
        }
        BOOL(1)
    }
    let mut cx = Ctx { pid, out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut cx as *mut Ctx as isize));
    }
    cx.out
}

impl Locker {
    /// Se le puede pedir que se cierre (tiene ventana): como tocar su X, guarda o pregunta.
    pub fn can_close(&self) -> bool {
        !self.is_self && !windows_of(self.pid).is_empty()
    }

    /// Se ofrece forzar el cierre (no a Windows, al Explorador ni a servicios).
    pub fn can_kill(&self) -> bool {
        !self.is_self && matches!(self.kind, Kind::Window | Kind::Console | Kind::Other)
    }

    /// Le pide a sus ventanas que se cierren (lo mismo que tocar la X).
    pub fn close(&self) -> bool {
        let ws = windows_of(self.pid);
        for h in &ws {
            unsafe {
                let _ = PostMessageW(Some(*h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        !ws.is_empty()
    }

    /// Termina el proceso (se pierde lo que no se haya guardado). Comprueba que el
    /// número de proceso siga siendo el mismo programa.
    pub fn kill(&self) -> Result<(), String> {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, self.pid).map_err(|_| t!("uso.no_hay_permiso_cerrarlo").to_string())?;
            let (mut c, mut x, mut k, mut u) = Default::default();
            let same = GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).is_ok() && c.dwLowDateTime == self.start.dwLowDateTime && c.dwHighDateTime == self.start.dwHighDateTime;
            let r = if same { TerminateProcess(h, 1).map_err(|e| e.message()) } else { Err(t!("uso.programa_ya_cerro").into()) };
            let _ = CloseHandle(h);
            r
        }
    }

    /// ¿Sigue abierto?
    pub fn alive(&self) -> bool {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, self.pid) else { return false };
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code).is_ok() && code == 259; // STILL_ACTIVE
            let _ = CloseHandle(h);
            ok
        }
    }
}
