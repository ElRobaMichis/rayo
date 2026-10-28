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
        let out = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd").creation_flags(0x0800_0000).args(["/c", "mklink", "/J", link, target]).output().map_err(|e| e.to_string())?
        };
        if out.status.success() {
            return Ok(t!("enlace.union_carpeta"));
        }
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    // 3. Archivo: un enlace duro (mismo disco).
    unsafe { CreateHardLinkW(PCWSTR(wl.as_ptr()), PCWSTR(wt.as_ptr()), None) }.map(|_| t!("enlace.enlace_duro")).map_err(|_| {
        t!("enlace.windows_no_permite_enlaces_simbolicos").to_string()
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
        crate::i18n::init(Some("es"));
        let base = d.to_string_lossy().into_owned();
        let target = super::super::join(&base, r"carpeta\a.txt");
        let link = super::link_path(&base, &target);
        let kind = super::make(&target, &link).unwrap();
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "hola", "{kind}");
        let dir_target = super::super::join(&base, "carpeta");
        let dir_link = super::link_path(&base, &dir_target);
        assert!(dir_link.ends_with("carpeta - enlace"), "{dir_link}");
        let kind = super::make(&dir_target, &dir_link).unwrap();
        assert!(std::path::Path::new(&dir_link).join("a.txt").is_file(), "{kind}");
        println!("archivo y carpeta enlazados ({kind})");
        let _ = std::fs::remove_dir(&dir_link);
        let _ = std::fs::remove_dir_all(&d);
    }
}
