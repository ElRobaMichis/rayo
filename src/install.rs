//! Rayo se instala solo: el mismo .exe que se baja de GitHub se copia a
//! `%LOCALAPPDATA%\Programs\Rayo`, crea su acceso en el menú Inicio y se registra en
//! "Aplicaciones instaladas" (de donde se desinstala). Todo en el usuario: sin
//! permisos de administrador. Quien prefiera, lo usa sin instalar (portátil).
//!
//! Para probar sin tocar lo real: `RAYO_INSTALL_TEST=<carpeta>` instala ahí, con un
//! acceso en esa carpeta y otra clave de desinstalación.

use crate::index::walk::wide;
use std::path::{Path, PathBuf};
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

fn test_root() -> Option<PathBuf> {
    std::env::var_os("RAYO_INSTALL_TEST").map(PathBuf::from)
}

fn uninstall_key() -> &'static str {
    if test_root().is_some() { r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Rayo-prueba" } else { r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Rayo" }
}

/// Carpeta de la instalación.
pub fn install_dir() -> PathBuf {
    match test_root() {
        Some(r) => r.join("Programs").join("Rayo"),
        None => std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Programs").join("Rayo"),
    }
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("rayo.exe")
}

fn shortcut_path() -> PathBuf {
    match test_root() {
        Some(r) => r.join("Rayo.lnk"),
        None => std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(r"Microsoft\Windows\Start Menu\Programs\Rayo.lnk"),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    norm(a) == norm(b)
}

/// ¿Este proceso es el Rayo instalado?
pub fn is_installed_copy() -> bool {
    std::env::current_exe().is_ok_and(|e| same_file(&e, &installed_exe()))
}

/// ¿Rayo está instalado en este usuario?
pub fn is_installed() -> bool {
    installed_exe().is_file()
}

/// El Rayo al que deben apuntar el registro y el inicio con Windows: el instalado si
/// existe (aunque esté corriendo otra copia, p. ej. una compilación de prueba).
pub fn app_exe() -> PathBuf {
    if is_installed() { installed_exe() } else { std::env::current_exe().unwrap_or_default() }
}

/// ¿Esta copia puede tocar el registro (explorador predeterminado, inicio con Windows)?
/// Una copia suelta no debe "robarle" esas rutas al Rayo instalado.
pub fn owns_registrations() -> bool {
    // En las pruebas nunca: no tocar el registro real.
    test_root().is_none() && (is_installed_copy() || !is_installed())
}

fn set_str(sub: &str, name: &str, value: &str) -> Result<(), String> {
    let (s, n, v) = (wide(sub), wide(name), wide(value));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), PCWSTR(n.as_ptr()), REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32) }.ok().map_err(|e| e.message())
}

fn set_dword(sub: &str, name: &str, value: u32) -> Result<(), String> {
    let (s, n) = (wide(sub), wide(name));
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(s.as_ptr()), PCWSTR(n.as_ptr()), REG_DWORD.0, Some(&value as *const u32 as *const _), 4) }.ok().map_err(|e| e.message())
}

fn create_shortcut(target: &Path, at: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::Interface;
    crate::fsx::shell::com_init();
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message())?;
        let t = wide(&target.display().to_string());
        let dir = wide(&target.parent().map(|p| p.display().to_string()).unwrap_or_default());
        let desc = wide(t!("instalar.explorador_archivos_rapido_minimalista"));
        link.SetPath(PCWSTR(t.as_ptr())).map_err(|e| e.message())?;
        link.SetWorkingDirectory(PCWSTR(dir.as_ptr())).map_err(|e| e.message())?;
        link.SetDescription(PCWSTR(desc.as_ptr())).map_err(|e| e.message())?;
        link.SetIconLocation(PCWSTR(t.as_ptr()), 0).map_err(|e| e.message())?;
        if let Some(parent) = at.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file: IPersistFile = link.cast().map_err(|e| e.message())?;
        let p = wide(&at.display().to_string());
        file.Save(PCWSTR(p.as_ptr()), true).map_err(|e| e.message())
    }
}

/// Instala (o actualiza con esta versión) y devuelve la ruta del Rayo instalado.
pub fn install() -> Result<PathBuf, String> {
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dest = installed_exe();
    std::fs::create_dir_all(install_dir()).map_err(|e| tf!("instalar.no_pudo_crear_carpeta", error = e))?;
    if !same_file(&src, &dest) {
        // El instalado puede estar terminando de cerrarse: se reintenta un momento.
        let mut last = String::new();
        let mut ok = false;
        for _ in 0..20 {
            match std::fs::copy(&src, &dest) {
                Ok(_) => {
                    ok = true;
                    break;
                }
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if !ok {
            return Err(tf!("instalar.no_pudo_copiar_rayo", error = last));
        }
    }
    create_shortcut(&dest, &shortcut_path())?;
    let key = uninstall_key();
    let exe = dest.display().to_string();
    let size_kb = std::fs::metadata(&dest).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    set_str(key, "DisplayName", "Rayo")?;
    set_str(key, "DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    set_str(key, "Publisher", "Rayo")?;
    set_str(key, "DisplayIcon", &format!("{exe},0"))?;
    set_str(key, "InstallLocation", &install_dir().display().to_string())?;
    set_str(key, "UninstallString", &format!("\"{exe}\" --uninstall"))?;
    set_dword(key, "NoModify", 1)?;
    set_dword(key, "NoRepair", 1)?;
    set_dword(key, "EstimatedSize", size_kb)?;
    Ok(dest)
}

/// Abre el Rayo instalado y espera a que este proceso termine antes de tomar el control.
pub fn launch_installed() -> Result<(), String> {
    std::process::Command::new(installed_exe()).arg(format!("--esperar={}", std::process::id())).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// `--esperar=PID`: no arrancar hasta que ese proceso (el que instaló) se haya cerrado.
pub fn wait_for_previous(args: &[String]) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    let Some(pid) = args.iter().find_map(|a| a.strip_prefix("--esperar=")).and_then(|p| p.parse::<u32>().ok()) else { return };
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, 10_000);
            let _ = CloseHandle(h);
        }
    }
}

fn ask(text: &str, title: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW};
    // En las pruebas se contesta que sí sin preguntar.
    if test_root().is_some() {
        return true;
    }
    let (t, c) = (wide(text), wide(title));
    unsafe { MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

fn tell(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
    let (t, c) = (wide(text), wide("Rayo"));
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONINFORMATION);
    }
}

/// Lo que lanza "Aplicaciones instaladas" (`rayo.exe --uninstall`): deja todo como estaba.
pub fn uninstall() -> i32 {
    if !ask(t!("instalar.desinstalar_rayo_si_rayo_tu"), t!("instalar.desinstalar_rayo")) {
        return 1;
    }
    let testing = test_root().is_some();
    if !testing {
        crate::instance::quit_running();
        // Explorador predeterminado e inicio con Windows: como antes de Rayo.
        if crate::integration::is_default(&crate::integration::REAL) {
            let _ = crate::integration::disable(&crate::integration::REAL);
        }
        let _ = crate::instance::set_autostart(false);
        // El servicio del índice (si se instaló) necesita permiso para quitarse.
        if crate::index::service::installed_exe().is_file()
            && ask(t!("instalar.quitar_tambien_servicio_indice_instantan"), t!("instalar.desinstalar_rayo"))
        {
            let _ = crate::fsx::shell::run_self_elevated("--uninstall-service");
        }
    }
    let _ = std::fs::remove_file(shortcut_path());
    let k = wide(uninstall_key());
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()));
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()));
    }
    if !testing && ask(t!("instalar.borrar_tambien_tu_configuracion_pestanas"), t!("instalar.desinstalar_rayo")) {
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let _ = std::fs::remove_dir_all(PathBuf::from(base).join("Rayo"));
            }
        }
    }
    // Este .exe está corriendo: la carpeta se borra un momento después de cerrarse.
    let dir = install_dir();
    if std::env::current_exe().is_ok_and(|e| e.starts_with(&dir)) {
        use std::os::windows::process::CommandExt;
        // Tal cual (raw_arg): cmd no entiende las comillas escapadas al estilo de Rust.
        let _ = std::process::Command::new("cmd")
            .raw_arg(format!("/c ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display()))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if !testing {
        tell(t!("instalar.rayo_desinstalo"));
    }
    0
}
