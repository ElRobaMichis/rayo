//! Servicio de Windows de Rayo ("RayoIndexer").
//!
//! Hace solo lo que necesita permisos: leer la MFT y el USN Journal. La app,
//! sin permisos, le pide por una named pipe el índice de un disco o los cambios
//! desde cierto USN, y los aplica a su propio índice. Es el mismo reparto que
//! usa Everything entre su servicio y su ventana.
//!
//! Protocolo: cada mensaje es un JSON precedido por su largo (u32). Después de
//! la respuesta pueden venir datos binarios (el índice o la lista de cambios).

use super::cache;
use super::store::VolumeIndex;
use super::usn::{Change, ReadError, Volume};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_BUSY, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Services::*;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::core::{PCWSTR, PWSTR, w};

pub const SERVICE_NAME: &str = "RayoIndexer";
pub const PIPE_NAME: &str = r"\\.\pipe\RayoIndexer";
/// Acceso del cliente: leer/escribir datos, leer atributos y sincronizar.
const CLIENT_ACCESS: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0080 | 0x0010_0000;

/// Sube cuando cambia el protocolo o el formato del índice.
pub const PROTOCOL: u64 = 1;

/// Solo el servicio escribe registro (en C:\ProgramData\Rayo\servicio.log).
static LOG_ON: AtomicBool = AtomicBool::new(false);

pub fn log_path() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("Rayo").join("servicio.log")
}

fn log(msg: &str) {
    if !LOG_ON.load(Ordering::Relaxed) {
        return;
    }
    let path = log_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    // Que no crezca sin límite.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::remove_file(&path);
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

fn last_error() -> String {
    let e = windows::core::Error::from_thread();
    format!("{} ({:#x})", e.message(), e.code().0)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Dónde se instala el ejecutable del servicio (solo administradores pueden escribir ahí).
pub fn installed_exe() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    base.join("Rayo").join("rayo.exe")
}

// ---------------------------------------------------------------- E/S sobre la tubería

pub struct Pipe(HANDLE);

unsafe impl Send for Pipe {}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut got = 0u32;
        let n = buf.len().min(1 << 20);
        unsafe { ReadFile(self.0, Some(&mut buf[..n]), Some(&mut got), None) }.map_err(|e| std::io::Error::other(e.message()))?;
        Ok(got as usize)
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut put = 0u32;
        let n = buf.len().min(1 << 20);
        unsafe { WriteFile(self.0, Some(&buf[..n]), Some(&mut put), None) }.map_err(|e| std::io::Error::other(e.message()))?;
        Ok(put as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn write_msg(w: &mut impl Write, v: &Value) -> std::io::Result<()> {
    let b = serde_json::to_vec(v)?;
    w.write_all(&(b.len() as u32).to_le_bytes())?;
    w.write_all(&b)
}

fn read_msg(r: &mut impl Read) -> std::io::Result<Value> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("mensaje demasiado grande"));
    }
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    serde_json::from_slice(&b).map_err(std::io::Error::other)
}

fn write_changes(w: &mut impl Write, list: &[Change]) -> std::io::Result<()> {
    for c in list {
        w.write_all(&c.frn.to_le_bytes())?;
        w.write_all(&c.parent.to_le_bytes())?;
        w.write_all(&c.usn.to_le_bytes())?;
        w.write_all(&c.reason.to_le_bytes())?;
        w.write_all(&c.attrs.to_le_bytes())?;
        w.write_all(&c.size.to_le_bytes())?;
        w.write_all(&c.mtime.to_le_bytes())?;
        w.write_all(&(c.name.len() as u32).to_le_bytes())?;
        w.write_all(c.name.as_bytes())?;
    }
    Ok(())
}

fn read_changes(r: &mut impl Read, count: usize) -> std::io::Result<Vec<Change>> {
    let mut out = Vec::with_capacity(count);
    let mut b8 = [0u8; 8];
    let mut b4 = [0u8; 4];
    let mut u64r = |r: &mut dyn Read| -> std::io::Result<u64> {
        r.read_exact(&mut b8)?;
        Ok(u64::from_le_bytes(b8))
    };
    for _ in 0..count {
        let frn = u64r(r)?;
        let parent = u64r(r)?;
        let usn = u64r(r)? as i64;
        r.read_exact(&mut b4)?;
        let reason = u32::from_le_bytes(b4);
        r.read_exact(&mut b4)?;
        let attrs = u32::from_le_bytes(b4);
        let size = u64r(r)?;
        r.read_exact(&mut b4)?;
        let mtime = u32::from_le_bytes(b4);
        r.read_exact(&mut b4)?;
        let len = u32::from_le_bytes(b4) as usize;
        if len > 4096 {
            return Err(std::io::Error::other("nombre inválido"));
        }
        let mut name = vec![0u8; len];
        r.read_exact(&mut name)?;
        out.push(Change { frn, parent, usn, reason, attrs, name: String::from_utf8_lossy(&name).into_owned(), size, mtime });
    }
    Ok(out)
}

fn letter_of(v: &Value) -> Option<char> {
    let c = v["letter"].as_str()?.chars().next()?.to_ascii_uppercase();
    c.is_ascii_uppercase().then_some(c)
}

// ---------------------------------------------------------------- servidor

fn handle_client(pipe: Pipe) {
    let mut volumes: HashMap<char, Volume> = HashMap::new();
    let mut reader = BufReader::new(Pipe(pipe.0));
    let mut writer = BufWriter::with_capacity(1 << 20, pipe);
    log("cliente conectado");
    while let Ok(req) = read_msg(&mut reader) {
        if req["cmd"] != "changes" {
            log(&format!("comando: {req}"));
        }
        let ok = match req["cmd"].as_str() {
            Some("hello") => write_msg(&mut writer, &json!({"ok": true, "protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION")})).is_ok(),
            Some("journal") => {
                let r = letter_of(&req).ok_or("letra inválida".to_string()).and_then(Volume::open).and_then(|v| v.journal());
                let msg = match r {
                    Ok(j) => json!({"ok": true, "id": j.id, "next": j.next}),
                    Err(e) => json!({"ok": false, "error": e}),
                };
                write_msg(&mut writer, &msg).is_ok()
            }
            Some("build") => {
                let r = letter_of(&req).ok_or("letra inválida".to_string()).and_then(|l| super::mft::read_mft(l, &AtomicUsize::new(0)));
                match r {
                    Ok(v) => {
                        log(&format!("índice listo: {} entradas", v.live_len()));
                        write_msg(&mut writer, &json!({"ok": true})).is_ok() && cache::write_index(&mut writer, &v).is_ok()
                    }
                    Err(e) => {
                        log(&format!("error al indexar: {e}"));
                        write_msg(&mut writer, &json!({"ok": false, "error": e})).is_ok()
                    }
                }
            }
            Some("changes") => {
                let Some(letter) = letter_of(&req) else { break };
                let journal = req["journal"].as_u64().unwrap_or(0);
                let from = req["from"].as_i64().unwrap_or(0);
                let wait = req["wait"].as_u64().unwrap_or(0).min(60) as u32;
                let vol = match volumes.entry(letter) {
                    std::collections::hash_map::Entry::Occupied(o) => Ok(o.into_mut()),
                    std::collections::hash_map::Entry::Vacant(e) => Volume::open(letter).map(|v| e.insert(v)),
                };
                match vol.map(|v| v.changes(journal, from, wait)) {
                    Ok(Ok((list, next))) => {
                        write_msg(&mut writer, &json!({"ok": true, "next": next, "count": list.len()})).is_ok() && write_changes(&mut writer, &list).is_ok()
                    }
                    Ok(Err(ReadError::Reset)) => write_msg(&mut writer, &json!({"ok": true, "reset": true})).is_ok(),
                    Ok(Err(ReadError::Other(e))) | Err(e) => write_msg(&mut writer, &json!({"ok": false, "error": e})).is_ok(),
                }
            }
            _ => write_msg(&mut writer, &json!({"ok": false, "error": "comando desconocido"})).is_ok(),
        };
        if !ok || writer.flush().is_err() {
            break;
        }
    }
    log("cliente desconectado");
    let h = writer.get_ref().0;
    unsafe {
        let _ = DisconnectNamedPipe(h);
    }
    // Evitar cerrar dos veces: `reader` comparte el handle con `writer`.
    std::mem::forget(reader);
}

/// Acepta clientes hasta que `stop` se active. Solo usuarios locales autenticados.
pub fn serve(stop: &AtomicBool) {
    serve_on(PIPE_NAME, stop)
}

fn serve_on(pipe_name: &str, stop: &AtomicBool) {
    // Permisos explícitos (no genéricos):
    //  - usuarios autenticados: leer y escribir datos (0x12018B), pero NO crear
    //    instancias de la tubería (así nadie puede hacerse pasar por el servicio);
    //  - SYSTEM, administradores y el dueño de la tubería (quien la creó): todo (0x1F01FF).
    let sddl = w!("D:(A;;0x12018B;;;AU)(A;;0x1F01FF;;;SY)(A;;0x1F01FF;;;BA)(A;;0x1F01FF;;;OW)");
    let mut sd = PSECURITY_DESCRIPTOR::default();
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut sd, None) }.is_err() {
        log(&format!("descriptor de seguridad inválido: {}", last_error()));
        return;
    }
    log("servicio escuchando");
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
    let name = wide(pipe_name);
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        let mut mode = PIPE_ACCESS_DUPLEX;
        if first {
            // La primera instancia de la tubería debe ser nuestra (evita que otro programa la ocupe antes).
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let h = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                1 << 20,
                64 * 1024,
                0,
                Some(&sa),
            )
        };
        if h.is_invalid() {
            log(&format!("no se pudo crear la instancia de la tubería: {}", last_error()));
            if first {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            continue;
        }
        first = false;
        let connected = unsafe { ConnectNamedPipe(h, None) };
        let already = connected.as_ref().is_err_and(|e| e.code() == windows::Win32::Foundation::ERROR_PIPE_CONNECTED.to_hresult());
        if connected.is_ok() || already {
            let pipe = Pipe(h);
            if std::thread::Builder::new().name("cliente".into()).spawn(move || handle_client(pipe)).is_err() {
                log("no se pudo crear el hilo del cliente");
            }
        } else {
            log(&format!("falló la espera de cliente: {}", connected.err().map(|e| e.message()).unwrap_or_default()));
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
}

// ---------------------------------------------------------------- cliente

pub enum Changes {
    Reset,
    Batch(Vec<Change>, i64),
}

pub struct Client {
    reader: BufReader<Pipe>,
    writer: Pipe,
}

/// PID del servicio RayoIndexer según el administrador de servicios de Windows.
fn service_pid() -> Option<u32> {
    let scm = Sc(unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }.ok()?);
    let name = wide(SERVICE_NAME);
    let svc = Sc(unsafe { OpenServiceW(scm.0, PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS) }.ok()?);
    let mut st = SERVICE_STATUS_PROCESS::default();
    let bytes = unsafe { std::slice::from_raw_parts_mut(&mut st as *mut _ as *mut u8, std::mem::size_of::<SERVICE_STATUS_PROCESS>()) };
    let mut needed = 0u32;
    unsafe { QueryServiceStatusEx(svc.0, SC_STATUS_PROCESS_INFO, Some(bytes), &mut needed) }.ok()?;
    (st.dwProcessId != 0).then_some(st.dwProcessId)
}

/// Comprueba que el proceso al otro lado de la tubería sea el servicio RayoIndexer.
/// Un usuario normal no puede abrir un proceso de SYSTEM para leer su ruta, así
/// que se compara el PID con el que informa el administrador de servicios: solo
/// un administrador puede registrar un servicio con ese nombre.
fn server_is_trusted(h: HANDLE) -> bool {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(h, &mut pid) }.is_err() {
        return false;
    }
    service_pid() == Some(pid)
}

impl Client {
    pub fn connect() -> Option<Client> {
        Self::connect_to(PIPE_NAME, true)
    }

    fn connect_to(pipe_name: &str, verify: bool) -> Option<Client> {
        let name = wide(pipe_name);
        for _ in 0..3 {
            let h = unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    CLIENT_ACCESS,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            };
            match h {
                Ok(h) => {
                    if verify && !server_is_trusted(h) {
                        unsafe {
                            let _ = CloseHandle(h);
                        }
                        return None;
                    }
                    let mut c = Client { reader: BufReader::with_capacity(1 << 20, Pipe(h)), writer: Pipe(h) };
                    let hello = c.call(&json!({"cmd": "hello"})).ok()?;
                    if hello["protocol"].as_u64() != Some(PROTOCOL) {
                        return None;
                    }
                    return Some(c);
                }
                Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => unsafe {
                    let _ = WaitNamedPipeW(PCWSTR(name.as_ptr()), 1000);
                },
                Err(_) => return None,
            }
        }
        None
    }

    fn call(&mut self, req: &Value) -> Result<Value, String> {
        write_msg(&mut self.writer, req).map_err(|e| e.to_string())?;
        let r = read_msg(&mut self.reader).map_err(|e| e.to_string())?;
        if r["ok"].as_bool() == Some(true) { Ok(r) } else { Err(r["error"].as_str().unwrap_or("error del servicio").to_string()) }
    }

    pub fn journal(&mut self, letter: char) -> Result<(u64, i64), String> {
        let r = self.call(&json!({"cmd": "journal", "letter": letter.to_string()}))?;
        Ok((r["id"].as_u64().unwrap_or(0), r["next"].as_i64().unwrap_or(0)))
    }

    /// Índice completo leído de la MFT por el servicio.
    pub fn build(&mut self, letter: char) -> Result<VolumeIndex, String> {
        self.call(&json!({"cmd": "build", "letter": letter.to_string()}))?;
        cache::read_index(&mut self.reader, letter).map_err(|e| e.to_string())
    }

    /// Cambios desde `from`; con `wait` > 0 espera (hasta esos segundos) a que haya alguno.
    pub fn changes(&mut self, letter: char, journal: u64, from: i64, wait: u32) -> Result<Changes, String> {
        let r = self.call(&json!({"cmd": "changes", "letter": letter.to_string(), "journal": journal, "from": from, "wait": wait}))?;
        if r["reset"].as_bool() == Some(true) {
            return Ok(Changes::Reset);
        }
        let count = r["count"].as_u64().unwrap_or(0) as usize;
        let list = read_changes(&mut self.reader, count).map_err(|e| e.to_string())?;
        Ok(Changes::Batch(list, r["next"].as_i64().unwrap_or(from)))
    }
}

/// Explica paso a paso por qué el cliente se conecta o no (para diagnóstico).
pub fn diagnose() -> String {
    let mut out = String::new();
    let name = wide(PIPE_NAME);
    let h = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            CLIENT_ACCESS,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    let h = match h {
        Ok(h) => {
            out.push_str("tubería: abierta\n");
            h
        }
        Err(e) => {
            out.push_str(&format!("tubería: no se pudo abrir ({})\n", e.message()));
            return out;
        }
    };
    let mut pid = 0u32;
    match unsafe { GetNamedPipeServerProcessId(h, &mut pid) } {
        Ok(()) => out.push_str(&format!("proceso del servidor: {pid}\n")),
        Err(e) => out.push_str(&format!("proceso del servidor: desconocido ({})\n", e.message())),
    }
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(p) => {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            match unsafe { QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) } {
                Ok(()) => out.push_str(&format!("ejecutable: {}\n", String::from_utf16_lossy(&buf[..len as usize]))),
                Err(e) => out.push_str(&format!("ejecutable: no se pudo leer ({})\n", e.message())),
            }
            unsafe {
                let _ = CloseHandle(p);
            }
        }
        Err(e) => out.push_str(&format!("abrir el proceso: falló ({})\n", e.message())),
    }
    out.push_str(&format!("PID del servicio según Windows: {:?}\n", service_pid()));
    out.push_str(&format!("de confianza: {}\n", server_is_trusted(h)));
    let mut c = Client { reader: BufReader::new(Pipe(h)), writer: Pipe(h) };
    match c.call(&json!({"cmd": "hello"})) {
        Ok(v) => out.push_str(&format!("saludo: {v}\n")),
        Err(e) => out.push_str(&format!("saludo: falló ({e})\n")),
    }
    std::mem::forget(c.writer);
    out
}

/// ¿Está el servicio instalado y respondiendo?
pub fn available() -> bool {
    Client::connect().is_some()
}

// ---------------------------------------------------------------- servicio de Windows

static STOP: AtomicBool = AtomicBool::new(false);
static mut STATUS_HANDLE: SERVICE_STATUS_HANDLE = SERVICE_STATUS_HANDLE(std::ptr::null_mut());

fn report(state: SERVICE_STATUS_CURRENT_STATE) {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN } else { 0 },
        dwWaitHint: 3000,
        ..Default::default()
    };
    unsafe {
        #[allow(static_mut_refs)]
        let _ = SetServiceStatus(STATUS_HANDLE, &status);
    }
}

unsafe extern "system" fn control_handler(control: u32, _event: u32, _data: *mut core::ffi::c_void, _ctx: *mut core::ffi::c_void) -> u32 {
    if control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN {
        STOP.store(true, Ordering::Relaxed);
        report(SERVICE_STOPPED);
        std::process::exit(0);
    }
    0
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    LOG_ON.store(true, Ordering::Relaxed);
    log(&format!("servicio iniciado (versión {})", env!("CARGO_PKG_VERSION")));
    let name = wide(SERVICE_NAME);
    match unsafe { RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(control_handler), None) } {
        Ok(h) => unsafe { STATUS_HANDLE = h },
        Err(_) => return,
    }
    report(SERVICE_RUNNING);
    serve(&STOP);
    report(SERVICE_STOPPED);
}

/// Punto de entrada con `--service` (lo lanza el administrador de servicios).
pub fn run_as_service() -> i32 {
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW { lpServiceName: PWSTR(name.as_mut_ptr()), lpServiceProc: Some(service_main) },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }.is_err() { 1 } else { 0 }
}

// ---------------------------------------------------------------- instalación (requiere administrador)

struct Sc(SC_HANDLE);

impl Drop for Sc {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

fn stop_and_wait(svc: &Sc) {
    let mut st = SERVICE_STATUS::default();
    unsafe {
        let _ = ControlService(svc.0, SERVICE_CONTROL_STOP, &mut st);
    }
    for _ in 0..50 {
        if unsafe { QueryServiceStatus(svc.0, &mut st) }.is_err() || st.dwCurrentState == SERVICE_STOPPED {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Copia el ejecutable a Archivos de programa y registra (o actualiza) el servicio.
pub fn install() -> Result<(), String> {
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dest = installed_exe();
    let scm = Sc(unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }.map_err(|e| tf!("servicio.necesitan_permisos_administrador", error = e.message()))?);
    let name = wide(SERVICE_NAME);
    let existing = unsafe { OpenServiceW(scm.0, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) }.ok().map(Sc);
    if let Some(svc) = &existing {
        stop_and_wait(svc);
    }
    if src != dest {
        std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut copied = Err(String::new());
        for _ in 0..20 {
            copied = std::fs::copy(&src, &dest).map(|_| ()).map_err(|e| e.to_string());
            if copied.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        copied.map_err(|e| tf!("servicio.no_pudo_copiar_ejecutable", error = e))?;
    }
    let cmd = wide(&format!("\"{}\" --service", dest.display()));
    let display = wide(t!("servicio.rayo_indexador_instantaneo"));
    let svc = match existing {
        Some(svc) => {
            unsafe {
                ChangeServiceConfigW(svc.0, ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE), SERVICE_AUTO_START, SERVICE_ERROR(SERVICE_NO_CHANGE), PCWSTR(cmd.as_ptr()), PCWSTR::null(), None, PCWSTR::null(), PCWSTR::null(), PCWSTR::null(), PCWSTR(display.as_ptr()))
            }
            .map_err(|e| e.message())?;
            svc
        }
        None => Sc(unsafe {
            CreateServiceW(
                scm.0,
                PCWSTR(name.as_ptr()),
                PCWSTR(display.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START,
                SERVICE_ERROR_NORMAL,
                PCWSTR(cmd.as_ptr()),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .map_err(|e| e.message())?),
    };
    let mut desc = wide(t!("servicio.lee_mft_usn_journal_ntfs"));
    let info = SERVICE_DESCRIPTIONW { lpDescription: PWSTR(desc.as_mut_ptr()) };
    unsafe {
        let _ = ChangeServiceConfig2W(svc.0, SERVICE_CONFIG_DESCRIPTION, Some(&info as *const _ as *const _));
    }
    unsafe { StartServiceW(svc.0, None) }.map_err(|e| e.message())?;
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let scm = Sc(unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) }.map_err(|e| e.message())?);
    let name = wide(SERVICE_NAME);
    if let Ok(h) = unsafe { OpenServiceW(scm.0, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) } {
        let svc = Sc(h);
        stop_and_wait(&svc);
        unsafe { DeleteService(svc.0) }.map_err(|e| e.message())?;
    }
    let exe = installed_exe();
    for _ in 0..20 {
        if std::fs::remove_file(&exe).is_ok() || !exe.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let _ = std::fs::remove_dir(exe.parent().unwrap());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_roundtrip() {
        let list = vec![
            Change { frn: 1, parent: 5, usn: 10, reason: 0x100, attrs: 0x20, name: "Canción.mp3".into(), size: 123, mtime: 456 },
            Change { frn: 2, parent: 1, usn: 11, reason: 0x200, attrs: 0x10, name: String::new(), size: 0, mtime: 0 },
        ];
        let mut buf = Vec::new();
        write_changes(&mut buf, &list).unwrap();
        let back = read_changes(&mut std::io::Cursor::new(buf), 2).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn protocol_over_pipe() {
        static STOP: AtomicBool = AtomicBool::new(false);
        let name = format!(r"\\.\pipe\RayoPrueba{}", std::process::id());
        let server_name = name.clone();
        std::thread::spawn(move || serve_on(&server_name, &STOP));
        let mut client = None;
        // Hasta 5 s: con la máquina ocupada (el antivirus revisando el ejecutable recién compilado,
        // una CI lenta) el hilo del servidor puede tardar en crear la tubería.
        for _ in 0..250 {
            client = Client::connect_to(&name, false);
            if client.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut c = client.expect("el cliente debería conectarse y saludar");
        // Sin administrador, el servicio de prueba no puede abrir el volumen:
        // el error debe llegar como mensaje, sin cortar la conexión.
        if let Err(e) = c.journal('C') {
            assert!(!e.is_empty());
        }
        match c.changes('C', 0, 0, 0) {
            Err(e) => assert!(!e.is_empty()),
            Ok(_) => {}
        }
        // La conexión sigue viva después de los errores.
        assert!(c.call(&json!({"cmd": "hello"})).is_ok());
        assert!(c.call(&json!({"cmd": "nada"})).is_err());
        // Varios clientes a la vez (la app abre uno por disco) y uno nuevo tras cerrar otro.
        let mut more: Vec<Client> = (0..3).map(|k| Client::connect_to(&name, false).unwrap_or_else(|| panic!("cliente extra {k}"))).collect();
        drop(more.pop());
        drop(c);
        assert!(Client::connect_to(&name, false).is_some(), "tras desconectar, debe aceptar clientes nuevos");
        STOP.store(true, Ordering::Relaxed);
    }
}
