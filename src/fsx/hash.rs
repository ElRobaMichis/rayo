//! Huellas de archivos (SHA-256, SHA-1, MD5) con la criptografía de Windows,
//! las tres en una sola lectura.

use super::long_path;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use windows::Win32::Security::Cryptography::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hashes {
    pub sha256: String,
    pub sha1: String,
    pub md5: String,
}

impl Hashes {
    /// ¿Coincide el texto pegado con alguna? (sin importar mayúsculas ni espacios)
    pub fn matches(&self, expected: &str) -> Option<&'static str> {
        let e: String = expected.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_lowercase();
        if e.is_empty() {
            return None;
        }
        [("SHA-256", &self.sha256), ("SHA-1", &self.sha1), ("MD5", &self.md5)].into_iter().find(|(_, h)| **h == e).map(|(n, _)| n)
    }
}

struct Hasher(BCRYPT_HASH_HANDLE, usize);

impl Hasher {
    fn new(alg: BCRYPT_ALG_HANDLE, len: usize) -> Option<Self> {
        let mut h = BCRYPT_HASH_HANDLE::default();
        unsafe { BCryptCreateHash(alg, &mut h, None, None, 0).is_ok().then_some(Hasher(h, len)) }
    }
    fn update(&self, data: &[u8]) {
        unsafe {
            let _ = BCryptHashData(self.0, data, 0);
        }
    }
    fn finish(self) -> String {
        let mut out = vec![0u8; self.1];
        unsafe {
            let _ = BCryptFinishHash(self.0, &mut out, 0);
        }
        out.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl Drop for Hasher {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyHash(self.0);
        }
    }
}

/// Calcula las tres huellas. `done` suma los bytes leídos (para el progreso).
pub fn file(path: &str, done: &AtomicU64, cancel: &AtomicBool) -> Result<Hashes, String> {
    let mut f = std::fs::File::open(long_path(path)).map_err(|e| e.to_string())?;
    let sha256 = Hasher::new(BCRYPT_SHA256_ALG_HANDLE, 32).ok_or(t!("hash.sha_256_no_disponible"))?;
    let sha1 = Hasher::new(BCRYPT_SHA1_ALG_HANDLE, 20).ok_or(t!("hash.sha_1_no_disponible"))?;
    let md5 = Hasher::new(BCRYPT_MD5_ALG_HANDLE, 16).ok_or(t!("hash.md5_no_disponible"))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelado".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        sha256.update(&buf[..n]);
        sha1.update(&buf[..n]);
        md5.update(&buf[..n]);
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(Hashes { sha256: sha256.finish(), sha1: sha1.finish(), md5: md5.finish() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hashes() {
        let p = std::env::temp_dir().join(format!("rayo-hash-{}.txt", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        let h = file(&p.to_string_lossy(), &AtomicU64::new(0), &AtomicBool::new(false)).unwrap();
        assert_eq!(h.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(h.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(h.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(h.matches("BA7816BF 8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"), Some("SHA-256"));
        assert_eq!(h.matches("1234"), None);
        let _ = std::fs::remove_file(p);
    }
}
