//! "Continuar como administrador": cuando Windows niega una operación por permisos
//! (borrar en Program Files, copiar a una carpeta del sistema…), se repite solo esa
//! operación en un Rayo con permisos de administrador. Windows muestra su aviso de
//! siempre (UAC) y, al terminar, ese proceso se cierra. La app nunca queda elevada.

use crate::fsx::shell::{self, FileOp};

#[derive(Clone, Debug)]
pub enum AdminOp {
    Delete { paths: Vec<String>, forever: bool },
    Copy { paths: Vec<String>, dest: String },
    Move { paths: Vec<String>, dest: String },
    Rename { from: String, to: String },
    NewFolder { dir: String },
}

impl AdminOp {
    /// Para el aviso: qué se va a hacer.
    pub fn describe(&self) -> String {
        let what = |p: &[String]| if p.len() == 1 { format!("«{}»", crate::fsx::file_name(&p[0])) } else { tf!("comun.elementos", n = p.len()) };
        match self {
            AdminOp::Delete { paths, forever } => format!("{} {}", if *forever { t!("comun.eliminar_definitivamente") } else { t!("admin.enviar_papelera") }, what(paths)),
            AdminOp::Copy { paths, dest } => tf!("admin.copiar", que = what(paths), nombre = crate::fsx::file_name(dest)),
            AdminOp::Move { paths, dest } => tf!("admin.mover", que = what(paths), nombre = crate::fsx::file_name(dest)),
            AdminOp::Rename { from, to } => tf!("admin.renombrar_como", viejo = crate::fsx::file_name(from), nuevo = crate::fsx::file_name(to)),
            AdminOp::NewFolder { dir } => tf!("admin.crear_carpeta", nombre = crate::fsx::file_name(dir)),
        }
    }

    /// Carpetas a volver a leer cuando termina.
    pub fn touched_dirs(&self) -> Vec<String> {
        let parents = |p: &[String]| p.iter().filter_map(|x| crate::fsx::parent_of(x)).collect::<Vec<_>>();
        match self {
            AdminOp::Delete { paths, .. } => parents(paths),
            AdminOp::Copy { dest, .. } => vec![dest.clone()],
            AdminOp::Move { paths, dest } => {
                let mut v = parents(paths);
                v.push(dest.clone());
                v
            }
            AdminOp::Rename { from, .. } => crate::fsx::parent_of(from).into_iter().collect(),
            AdminOp::NewFolder { dir } => vec![dir.clone()],
        }
    }

    /// Una línea por dato (las rutas no pueden tener saltos de línea).
    fn encode(&self) -> String {
        let (kind, args): (&str, Vec<&str>) = match self {
            AdminOp::Delete { paths, forever } => (if *forever { "eliminar" } else { "papelera" }, paths.iter().map(String::as_str).collect()),
            AdminOp::Copy { paths, dest } => ("copiar", std::iter::once(dest.as_str()).chain(paths.iter().map(String::as_str)).collect()),
            AdminOp::Move { paths, dest } => ("mover", std::iter::once(dest.as_str()).chain(paths.iter().map(String::as_str)).collect()),
            AdminOp::Rename { from, to } => ("renombrar", vec![from.as_str(), to.as_str()]),
            AdminOp::NewFolder { dir } => ("carpeta", vec![dir.as_str()]),
        };
        std::iter::once(kind).chain(args).collect::<Vec<_>>().join("\n")
    }

    fn decode(s: &str) -> Option<AdminOp> {
        let mut lines = s.lines().map(str::to_string);
        let kind = lines.next()?;
        let rest: Vec<String> = lines.filter(|l| !l.is_empty()).collect();
        Some(match kind.as_str() {
            "papelera" | "eliminar" => AdminOp::Delete { paths: rest, forever: kind == "eliminar" },
            "copiar" | "mover" => {
                let (dest, paths) = rest.split_first()?;
                if kind == "copiar" {
                    AdminOp::Copy { paths: paths.to_vec(), dest: dest.clone() }
                } else {
                    AdminOp::Move { paths: paths.to_vec(), dest: dest.clone() }
                }
            }
            "renombrar" => AdminOp::Rename { from: rest.first()?.clone(), to: rest.get(1)?.clone() },
            "carpeta" => AdminOp::NewFolder { dir: rest.first()?.clone() },
            _ => return None,
        })
    }
}

/// ¿El error es "acceso denegado"? (lo que se arregla con permisos de administrador)
pub fn is_access_denied(msg: &str) -> bool {
    msg.contains("os error 5)") || msg.contains("os error 5 ")
}

/// En la app: pide permiso a Windows y hace la operación en un proceso aparte. Bloquea
/// hasta que termina (llamar desde un hilo).
pub fn run_elevated(op: &AdminOp) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let file = std::env::temp_dir().join(format!("rayo-admin-{}-{}.txt", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&file, op.encode()).map_err(|e| e.to_string())?;
    let r = shell::run_self_elevated(&format!("--admin-op \"{}\"", file.display()));
    let _ = std::fs::remove_file(&file);
    match r? {
        0 => Ok(()),
        _ => Err(t!("admin.windows_no_permitio_ni_permisos").into()),
    }
}

/// En el proceso con permisos de administrador (`rayo.exe --admin-op archivo`).
pub fn execute(file: &str) -> i32 {
    let Some(op) = std::fs::read_to_string(file).ok().and_then(|s| AdminOp::decode(&s)) else { return 2 };
    shell::com_init();
    let ok = match op {
        AdminOp::Delete { paths, forever } => shell::file_op_blocking(if forever { FileOp::DeleteForever } else { FileOp::Recycle }, paths, None),
        AdminOp::Copy { paths, dest } => shell::file_op_blocking(FileOp::Copy, paths, Some(dest)),
        AdminOp::Move { paths, dest } => shell::file_op_blocking(FileOp::Move, paths, Some(dest)),
        AdminOp::Rename { from, to } => std::fs::rename(crate::fsx::long_path(&from), crate::fsx::long_path(&to)).is_ok(),
        AdminOp::NewFolder { dir } => shell::new_folder(&dir).is_ok(),
    };
    if ok { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_roundtrip() {
        let ops = [
            AdminOp::Delete { paths: vec![r"C:\Program Files\X\a.txt".into(), r"C:\b".into()], forever: true },
            AdminOp::Copy { paths: vec![r"D:\fotos".into()], dest: r"C:\Program Files\App".into() },
            AdminOp::Move { paths: vec![r"C:\a".into()], dest: r"C:\Windows\x".into() },
            AdminOp::Rename { from: r"C:\Program Files\a".into(), to: r"C:\Program Files\b".into() },
            AdminOp::NewFolder { dir: r"C:\Program Files".into() },
        ];
        for op in ops {
            let back = AdminOp::decode(&op.encode()).unwrap();
            assert_eq!(format!("{op:?}"), format!("{back:?}"));
        }
        assert!(is_access_denied("Acceso denegado. (os error 5)"));
        assert!(!is_access_denied("(os error 53)"));
    }
}
