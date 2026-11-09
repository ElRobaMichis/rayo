//! Registro de diagnóstico: unas pocas líneas de lo que pasa cuando otra app le pide algo a Rayo
//! ("mostrar en carpeta", abrir una carpeta) y de cómo se resolvió. Sirve para entender un fallo
//! que no se puede repetir a pedido. Queda en `%LOCALAPPDATA%\Rayo\diagnostico.log` (o en la
//! carpeta de datos de `RAYO_DATA_DIR`), con tope de tamaño: al llenarse, lo anterior pasa a
//! `diagnostico.old.log`.

use std::io::Write;
use std::sync::OnceLock;

const MAX: u64 = 256 * 1024;

fn path() -> Option<&'static std::path::PathBuf> {
    static P: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    P.get_or_init(|| {
        let dir = crate::instance::custom_data_dir().or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Rayo")))?;
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("diagnostico.log"))
    })
    .as_ref()
}

pub fn log(msg: &str) {
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let Some(p) = path() else { return };
    let _g = LOCK.lock();
    if std::fs::metadata(p).is_ok_and(|m| m.len() > MAX) {
        let _ = std::fs::rename(p, p.with_file_name("diagnostico.old.log"));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let (secs, ms) = (now.as_secs(), now.subsec_millis());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{}:{:02}.{ms:03} {msg}", crate::fsx::fmt_date(secs as u32), secs % 60);
    }
}
