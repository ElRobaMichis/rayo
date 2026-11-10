//! Enlaces: simbólicos (cuando Windows lo permite sin ser administrador), o si no,
//! una unión para carpetas y un enlace duro para archivos del mismo disco.

use super::long_path;
use crate::index::walk::wide;
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

/// Crea en `link` un enlace a `target`. Devuelve qué tipo de enlace se creó.
pub fn make(target: &str, link: &str) -> Result<&'static str, String> {
    let is_dir = std::fs::metadata(long_path(target)).map_err(|e| e.to_string())?.is_dir();
    if std::fs::symlink_metadata(long_path(link)).is_ok() {
        return Err(tf!("comun.ya_existe", nombre = super::file_name(link)));
    }
    let (wl, wt) = (wide(link), wide(target));
    // 1. Simbólico (funciona sin permisos con el modo de desarrollador de Windows).
    let mut flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
    if is_dir {
        flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
    }
    if unsafe { CreateSymbolicLinkW(PCWSTR(wl.as_ptr()), PCWSTR(wt.as_ptr()), flags) } {
        return Ok(t!("enlace.enlace_simbolico"));
    }
    if is_dir {
        // 2. Carpeta: una unión (no pide permisos; solo en discos locales).
        return junction(target, link).map(|_| t!("enlace.union_carpeta"));
    }
    // 3. Archivo: un enlace duro (mismo disco).
    unsafe { CreateHardLinkW(PCWSTR(wl.as_ptr()), PCWSTR(wt.as_ptr()), None) }.map(|_| t!("enlace.enlace_duro")).map_err(|_| {
        t!("enlace.windows_no_permite_enlaces_simbolicos").to_string()
    })
}

/// Unión (punto de montaje) nativa: carpeta vacía + FSCTL_SET_REPARSE_POINT. Sin pasar
/// por `cmd /c mklink /J`, que interpretaría `&`, `%`, `^`… del nombre.
fn junction(target: &str, link: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE};
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    let t = super::short_path(target).replace('/', "\\");
    let t = t.trim_end_matches('\\');
    // Solo rutas absolutas de un disco ("C:\…"): una unión no puede apuntar a la red.
    let b = t.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b.len() > 2 && b[2] != b'\\') {
        return Err(t!("enlace.union_solo_local").to_string());
    }
    // La raíz de un disco conserva la barra: \??\C:\
    let t = if t.len() == 2 { format!("{t}\\") } else { t.to_string() };
    let subst: Vec<u16> = format!(r"\??\{t}").encode_utf16().collect();
    let print: Vec<u16> = t.encode_utf16().collect();
    let (sb, pb) = (subst.len() * 2, print.len() * 2);
    // REPARSE_DATA_BUFFER (MountPointReparseBuffer): cabecera de 8 + 8 + nombres con su NUL.
    let data = 8 + sb + 2 + pb + 2;
    if 8 + data > 16 * 1024 {
        // MAXIMUM_REPARSE_DATA_BUFFER_SIZE: ruta demasiado larga.
        return Err(std::io::Error::from_raw_os_error(206).to_string());
    }
    let mut buf = Vec::<u8>::with_capacity(8 + data);
    buf.extend(0xA000_0003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    buf.extend((data as u16).to_le_bytes()); // ReparseDataLength
    buf.extend(0u16.to_le_bytes()); // Reserved
    buf.extend(0u16.to_le_bytes()); // SubstituteNameOffset
    buf.extend((sb as u16).to_le_bytes()); // SubstituteNameLength (sin NUL)
    buf.extend(((sb + 2) as u16).to_le_bytes()); // PrintNameOffset
    buf.extend((pb as u16).to_le_bytes()); // PrintNameLength
    for u in subst.iter().chain(&[0u16]).chain(&print).chain(&[0u16]) {
        buf.extend(u.to_le_bytes());
    }
    let ll = long_path(link);
    std::fs::create_dir(&ll).map_err(|e| e.to_string())?;
    let wl = wide(&ll);
    let r = unsafe {
        CreateFileW(
            PCWSTR(wl.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .and_then(|h| unsafe {
        let mut got = 0u32;
        let r = DeviceIoControl(h, FSCTL_SET_REPARSE_POINT, Some(buf.as_ptr() as *const _), buf.len() as u32, None, 0, Some(&mut got), None);
        let _ = CloseHandle(h);
        r
    });
    // FAT/exFAT o red: no admiten uniones; no queda la carpeta vacía.
    r.map_err(|e| {
        let _ = std::fs::remove_dir(&ll);
        e.message()
    })
}

/// Nombre libre para el enlace en `dir` ("x", "x - enlace", "x - enlace (2)"…).
pub fn link_path(dir: &str, target: &str) -> String {
    let name = super::file_name(target);
    let first = super::join(dir, &name);
    if std::fs::symlink_metadata(long_path(&first)).is_err() && !first.eq_ignore_ascii_case(target) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !std::path::Path::new(&long_path(target)).is_dir() => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    for k in 1..1000 {
        let n = if k == 1 { tf!("enlace.nombre_enlace", nombre = stem, extension = ext) } else { tf!("enlace.nombre_enlace_n", nombre = stem, k = k, extension = ext) };
        let p = super::join(dir, &n);
        if std::fs::symlink_metadata(long_path(&p)).is_err() {
            return p;
        }
    }
    first
}

#[cfg(test)]
mod tests {
    #[test]
    fn links_files_and_folders() {
        let d = std::env::temp_dir().join(format!("rayo-enlaces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("carpeta")).unwrap();
        std::fs::write(d.join("carpeta").join("a.txt"), "hola").unwrap();
        // Sin fijar el idioma: otras pruebas corren a la vez y lo comparten.
        let base = d.to_string_lossy().into_owned();
        let target = super::super::join(&base, r"carpeta\a.txt");
        let link = super::link_path(&base, &target);
        let kind = super::make(&target, &link).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "hola", "{kind}");
        let dir_target = super::super::join(&base, "carpeta");
        let dir_link = super::link_path(&base, &dir_target);
        // "carpeta - enlace" (o "carpeta - link", según el idioma).
        assert!(dir_link.starts_with(&super::super::join(&base, "carpeta - ")) && dir_link != dir_target, "{dir_link}");
        let kind = super::make(&dir_target, &dir_link).unwrap();
        assert!(std::path::Path::new(&dir_link).join("a.txt").is_file(), "{kind}");
        println!("archivo y carpeta enlazados ({kind})");
        let _ = std::fs::remove_dir(&dir_link);
        // Unión directa (aunque haya modo de desarrollador), con caracteres que cmd interpretaría.
        let odd = super::super::join(&base, "R&D %TEMP% ^(x)=1");
        std::fs::create_dir(&odd).unwrap();
        let odd_link = super::super::join(&base, "R&D %TEMP% ^(x)=1 - union");
        super::junction(&odd, &odd_link).unwrap();
        std::fs::write(std::path::Path::new(&odd).join("b.txt"), "dentro").unwrap();
        assert_eq!(std::fs::read_to_string(std::path::Path::new(&odd_link).join("b.txt")).unwrap(), "dentro");
        assert!(std::fs::symlink_metadata(&odd_link).unwrap().file_type().is_symlink(), "no es una unión");
        assert!(super::junction(r"\\servidor\recurso", &super::super::join(&base, "red")).is_err());
        assert!(!std::path::Path::new(&super::super::join(&base, "red")).exists());
        let _ = std::fs::remove_dir(&odd_link);
        let _ = std::fs::remove_dir_all(&d);
    }
}
