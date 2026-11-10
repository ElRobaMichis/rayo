//! Conexiones remotas en los mismos paneles: SMB, WebDAV y SFTP, y las carpetas de la nube.
//!
//! En vez de reimplementar cada protocolo, se usa lo que Windows ya monta como rutas `\\…`
//! (y que Rayo ya sabe mostrar, vigilar y copiar sin trabarse):
//! - SMB: `\\servidor\recurso`.
//! - WebDAV: el cliente WebDAV de Windows, `\\servidor@SSL@puerto\carpeta`.
//! - SFTP: SSHFS-Win (gratuito), `\\sshfs.r\usuario@servidor!puerto\carpeta`.
//!
//! La contraseña nunca va al archivo de configuración: si se pide recordarla, la guarda el
//! Administrador de credenciales de Windows.

use crate::index::walk::wide;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Kind {
    #[default]
    Smb,
    WebDav,
    Sftp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Connection {
    pub name: String,
    pub kind: Kind,
    pub host: String,
    /// 0 = el de siempre (445, 443, 22).
    pub port: u16,
    /// Carpeta dentro del servidor ("compartida\fotos", "/home/yo").
    pub path: String,
    pub user: String,
    /// WebDAV con HTTPS (casi siempre).
    pub secure: bool,
}

impl Connection {
    /// La raíz que se conecta (lo que Windows autentica).
    pub fn root(&self) -> String {
        let host = self.host.trim().trim_start_matches(r"\\").trim_start_matches("https://").trim_start_matches("http://").trim_end_matches(['/', '\\']);
        match self.kind {
            Kind::Smb => {
                // \\servidor\recurso: el recurso es el primer tramo de la carpeta.
                let share = self.path.trim_matches(['/', '\\']).split(['/', '\\']).next().unwrap_or("");
                format!(r"\\{host}\{share}")
            }
            Kind::WebDav => {
                let mut s = format!(r"\\{host}");
                if self.secure {
                    s.push_str("@SSL");
                }
                if self.port != 0 {
                    s.push_str(&format!("@{}", self.port));
                }
                s.push_str(r"\DavWWWRoot");
                s
            }
            Kind::Sftp => {
                let user = if self.user.trim().is_empty() { String::new() } else { format!("{}@", self.user.trim()) };
                let port = if self.port != 0 && self.port != 22 { format!("!{}", self.port) } else { String::new() };
                format!(r"\\sshfs.r\{user}{host}{port}")
            }
        }
    }

    /// La carpeta que se abre en la pestaña.
    pub fn unc(&self) -> String {
        let rest = self.path.trim_matches(['/', '\\']).replace('/', "\\");
        let base = self.root();
        match self.kind {
            Kind::Smb => {
                // El recurso ya está en la raíz; lo que sigue, después.
                let after = rest.split_once('\\').map(|(_, r)| r).unwrap_or("");
                if after.is_empty() { base } else { format!(r"{base}\{after}") }
            }
            Kind::WebDav => {
                if rest.is_empty() {
                    base
                } else {
                    // Con carpeta, sin DavWWWRoot (\\servidor@SSL\carpeta).
                    format!(r"{}\{rest}", base.trim_end_matches(r"\DavWWWRoot"))
                }
            }
            Kind::Sftp => {
                if rest.is_empty() { base } else { format!(r"{base}\{rest}") }
            }
        }
    }

    fn cred_target(&self) -> String {
        format!("rayo:{:?}:{}@{}", self.kind, self.user.trim(), self.host.trim()).to_lowercase()
    }
}

/// ¿Está instalado SSHFS-Win (para SFTP)?
pub fn sshfs_installed() -> bool {
    ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var_os(v)).any(|d| std::path::Path::new(&d).join(r"SSHFS-Win\bin\sshfs.exe").is_file())
}

/// Conecta (autentica) y devuelve la carpeta para abrir. Tarda lo que tarde el servidor: llamar
/// desde otro hilo. `password`: la escrita ahora; si es None, la recordada (si hay).
pub fn connect(c: &Connection, password: Option<&str>, remember: bool) -> Result<String, String> {
    use windows::Win32::NetworkManagement::WNet::*;
    if c.kind == Kind::Sftp && !sshfs_installed() {
        return Err(t!("remoto.falta_sshfs").into());
    }
    let saved = if password.is_none() { load_password(&c.cred_target()) } else { None };
    let pass = password.map(str::to_string).or(saved);
    let mut remote = wide(&c.root());
    let net = NETRESOURCEW { dwType: RESOURCETYPE_DISK, lpRemoteName: windows::core::PWSTR(remote.as_mut_ptr()), ..Default::default() };
    let pw = pass.as_deref().map(wide);
    let user = (!c.user.trim().is_empty()).then(|| wide(c.user.trim()));
    let r = unsafe {
        WNetAddConnection2W(
            &net,
            pw.as_ref().map_or(windows::core::PCWSTR::null(), |p| windows::core::PCWSTR(p.as_ptr())),
            user.as_ref().map_or(windows::core::PCWSTR::null(), |u| windows::core::PCWSTR(u.as_ptr())),
            CONNECT_TEMPORARY,
        )
    };
    // 0 = bien; 1219 = ya hay una conexión a ese servidor (con otras credenciales): se usa esa.
    if r.0 != 0 && r.0 != 1219 {
        return Err(windows::core::Error::from(windows::core::HRESULT::from_win32(r.0)).message());
    }
    if remember {
        if let Some(p) = password {
            save_password(&c.cred_target(), c.user.trim(), p);
        }
    }
    Ok(c.unc())
}

/// Olvida la contraseña guardada de una conexión.
pub fn forget_password(c: &Connection) {
    use windows::Win32::Security::Credentials::*;
    let t = wide(&c.cred_target());
    let _ = unsafe { CredDeleteW(windows::core::PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None) };
}

fn save_password(target: &str, user: &str, password: &str) {
    use windows::Win32::Security::Credentials::*;
    let mut t = wide(target);
    let mut u = wide(user);
    let blob: Vec<u8> = password.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: windows::core::PWSTR(t.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: windows::core::PWSTR(u.as_mut_ptr()),
        ..Default::default()
    };
    let _ = unsafe { CredWriteW(&cred, 0) };
}

fn load_password(target: &str) -> Option<String> {
    use windows::Win32::Security::Credentials::*;
    let t = wide(target);
    let mut p: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(windows::core::PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None, &mut p).ok()?;
        let c = &*p;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let s = String::from_utf16_lossy(&units);
        CredFree(p as *const _);
        Some(s)
    }
}

// ---------------------------------------------------------------- nube

/// Carpetas de la nube en este equipo: (nombre, carpeta). OneDrive, Dropbox, iCloud, Google
/// Drive y lo que esté registrado como proveedor de nube de Windows.
pub fn cloud_folders() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, path: String| {
        let p = super::normalize(&path);
        if std::path::Path::new(&p).is_dir() && !out.iter().any(|(_, x)| x.eq_ignore_ascii_case(&p)) {
            out.push((name.to_string(), p));
        }
    };
    // Proveedores registrados en Windows (OneDrive, Dropbox, iCloud, Box…).
    for (name, path) in sync_roots() {
        add(&name, path);
    }
    // OneDrive: solo las cuentas con sesión iniciada. Windows crea la carpeta y la variable
    // %OneDrive% aunque nunca se configure, así que eso solo no prueba nada.
    for (name, path) in onedrive_accounts() {
        add(&name, path);
    }
    // Dropbox: info.json dice dónde está.
    for base in ["APPDATA", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v)) {
        if let Ok(b) = std::fs::read(std::path::Path::new(&base).join(r"Dropbox\info.json")) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                for k in ["personal", "business"] {
                    if let Some(p) = v.get(k).and_then(|x| x.get("path")).and_then(|x| x.as_str()) {
                        add("Dropbox", p.to_string());
                    }
                }
            }
        }
    }
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    add("iCloud Drive", format!(r"{home}\iCloudDrive"));
    // Google Drive para escritorio: una unidad con esa etiqueta (su "Mi unidad" adentro).
    for d in super::list_drives() {
        if d.label.to_lowercase().contains("google drive") {
            let root = format!("{}:\\", d.letter);
            let my = ["My Drive", "Mi unidad"].iter().map(|n| format!("{root}{n}")).find(|p| std::path::Path::new(p).is_dir());
            add("Google Drive", my.unwrap_or(root));
        }
    }
    out
}

/// Cuentas de OneDrive con sesión iniciada: (nombre, carpeta). Una cuenta conectada guarda su
/// correo (`UserEmail`); la "Personal" que Windows deja preparada sin iniciar sesión, solo la carpeta.
fn onedrive_accounts() -> Vec<(String, String)> {
    use windows::Win32::System::Registry::*;
    let base = r"Software\Microsoft\OneDrive\Accounts";
    let read = |sub: &str, value: &str| -> Option<String> {
        let (k, v) = (wide(&format!(r"{base}\{sub}")), wide(value));
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        unsafe { RegGetValueW(HKEY_CURRENT_USER, windows::core::PCWSTR(k.as_ptr()), windows::core::PCWSTR(v.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) }.ok().ok()?;
        let n = (len as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..n.min(buf.len())])).filter(|s| !s.trim().is_empty())
    };
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        let path = wide(base);
        if RegOpenKeyExW(HKEY_CURRENT_USER, windows::core::PCWSTR(path.as_ptr()), None, KEY_READ, &mut key).is_err() {
            return out;
        }
        let mut i = 0;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            i += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            if read(&sub, "UserEmail").is_none() {
                continue;
            }
            let Some(folder) = read(&sub, "UserFolder") else { continue };
            let label = if sub.eq_ignore_ascii_case("Personal") { "OneDrive".to_string() } else { read(&sub, "DisplayName").map_or("OneDrive (trabajo)".to_string(), |d| format!("OneDrive · {d}")) };
            out.push((label, folder));
        }
        let _ = RegCloseKey(key);
    }
    out
}

/// Lo registrado en SyncRootManager: el nombre del proveedor y su carpeta.
fn sync_roots() -> Vec<(String, String)> {
    use windows::Win32::System::Registry::*;
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager");
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, windows::core::PCWSTR(path.as_ptr()), None, KEY_READ, &mut key).is_err() {
            return out;
        }
        let mut i = 0;
        loop {
            let mut name = [0u16; 512];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            i += 1;
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            // "OneDrive!S-1-5-…!Personal|…" → "OneDrive"
            let provider = sub.split('!').next().unwrap_or(&sub).to_string();
            let roots = wide(&format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager\{sub}\UserSyncRoots"));
            let mut rk = HKEY::default();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, windows::core::PCWSTR(roots.as_ptr()), None, KEY_READ, &mut rk).is_err() {
                continue;
            }
            let mut j = 0;
            loop {
                let mut vname = [0u16; 256];
                let mut vlen = vname.len() as u32;
                let mut data = [0u16; 1024];
                let mut dlen = (data.len() * 2) as u32;
                if RegEnumValueW(rk, j, Some(windows::core::PWSTR(vname.as_mut_ptr())), &mut vlen, None, None, Some(data.as_mut_ptr() as *mut u8), Some(&mut dlen)).is_err() {
                    break;
                }
                j += 1;
                let n = (dlen as usize / 2).saturating_sub(1);
                let p = String::from_utf16_lossy(&data[..n.min(data.len())]);
                if !p.is_empty() {
                    out.push((provider.clone(), p));
                }
            }
            let _ = RegCloseKey(rk);
        }
        let _ = RegCloseKey(key);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unc_paths() {
        let smb = Connection { kind: Kind::Smb, host: "nas".into(), path: "fotos/2024".into(), ..Default::default() };
        assert_eq!(smb.root(), r"\\nas\fotos");
        assert_eq!(smb.unc(), r"\\nas\fotos\2024");
        let dav = Connection { kind: Kind::WebDav, host: "https://nube.ejemplo.com/".into(), secure: true, ..Default::default() };
        assert_eq!(dav.root(), r"\\nube.ejemplo.com@SSL\DavWWWRoot");
        assert_eq!(dav.unc(), r"\\nube.ejemplo.com@SSL\DavWWWRoot");
        let dav2 = Connection { kind: Kind::WebDav, host: "nube.ejemplo.com".into(), port: 8443, secure: true, path: "/remote.php/dav/files/yo".into(), ..Default::default() };
        assert_eq!(dav2.unc(), r"\\nube.ejemplo.com@SSL@8443\remote.php\dav\files\yo");
        let sftp = Connection { kind: Kind::Sftp, host: "servidor".into(), user: "yo".into(), port: 2222, path: "/var/www".into(), ..Default::default() };
        assert_eq!(sftp.root(), r"\\sshfs.r\yo@servidor!2222");
        assert_eq!(sftp.unc(), r"\\sshfs.r\yo@servidor!2222\var\www");
    }
}
