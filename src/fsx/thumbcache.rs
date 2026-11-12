//! Caché propia de miniaturas, en el disco de la app (normalmente un SSD).
//!
//! Lo que la cuadrícula ya mostró una vez queda guardado aquí, con la clave (ruta, lado, fecha y
//! tamaño del archivo según el listado). Se lee **antes** de tocar el disco de la foto: con un
//! disco mecánico dormido, volver a una carpeta muestra sus miniaturas al instante desde el SSD
//! mientras el disco arranca (o sin despertarlo). La fecha y el tamaño vienen del listado (o del
//! índice): preguntárselos al archivo despertaría al disco.
//!
//! - Formato: una cabecera de 40 bytes y los píxeles en QOI (sin pérdida, ~0,3 ms por miniatura
//!   en cada sentido; sin dependencias).
//! - Se escribe en un hilo propio, a un archivo temporal que después se renombra: dos Rayo a la
//!   vez (o uno que se cierra a mitad) nunca dejan un archivo a medias con el nombre bueno.
//! - Lo dañado (cabecera, largo o contenido que no cierra) se borra al leerlo.
//! - Tope `MAX_BYTES`: al pasarlo se borra lo usado hace más tiempo (la fecha de modificación de
//!   cada archivo se renueva al usarlo, como mucho una vez por hora).

use super::thumbs::Rgba;
use crossbeam_channel::{Sender, unbounded};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

const MAGIC: &[u8; 8] = b"RAYOMIN1";
const HEADER: usize = 40;
/// Tope de la caché en disco.
pub const MAX_BYTES: u64 = 512 << 20;
/// Al pasar el tope se borra hasta quedar en esta fracción (así no se limpia a cada escritura).
const TRIM_TO: f64 = 0.8;
/// Lado máximo que se guarda (miniaturas de la cuadrícula, también a 200 %).
pub const MAX_SIDE: u32 = 512;

/// Huella de la versión del archivo (fecha de modificación y tamaño, del listado). 0 = no se sabe.
pub fn stamp_of(mtime: u32, size: u64) -> u64 {
    let s = xxhash_rust::xxh3::xxh3_64(&[mtime.to_le_bytes().as_slice(), size.to_le_bytes().as_slice()].concat());
    s.max(1)
}

fn key_of(path: &str, side: u32, stamp: u64) -> u64 {
    xxhash_rust::xxh3::xxh3_64(format!("{}|{side}|{stamp}", path.to_lowercase()).as_bytes())
}

fn dir() -> Option<&'static PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        if std::env::var_os("RAYO_SIN_CACHE_MINIATURAS").is_some() {
            return None;
        }
        let d = crate::index::cache::data_dir().join("miniaturas");
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    })
    .as_ref()
}

fn file_of(key: u64) -> Option<PathBuf> {
    Some(dir()?.join(format!("{:02x}", key >> 56)).join(format!("{key:016x}.qoi")))
}

/// La miniatura guardada, si está y está sana. No toca el disco de la foto.
pub fn get(path: &str, side: u32, stamp: u64) -> Option<Rgba> {
    if stamp == 0 || side > MAX_SIDE {
        return None;
    }
    let key = key_of(path, side, stamp);
    let file = file_of(key)?;
    let mut f = std::fs::File::open(&file).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    drop(f);
    match parse(&buf, key) {
        Some(img) => {
            writer().map(|w| w.send(Job::Touch(file)));
            Some(img)
        }
        None => {
            let _ = std::fs::remove_file(&file);
            None
        }
    }
}

/// Guarda una miniatura (en otro hilo).
pub fn put(path: &str, side: u32, stamp: u64, img: &Rgba) {
    if stamp == 0 || side > MAX_SIDE || img.w == 0 || img.h == 0 || img.px.len() != (img.w * img.h * 4) as usize {
        return;
    }
    let Some(w) = writer() else { return };
    let key = key_of(path, side, stamp);
    let copy = Rgba { w: img.w, h: img.h, px: img.px.clone(), premultiplied: img.premultiplied, orig: img.orig, pages: None };
    let _ = w.send(Job::Put(key, copy));
}

fn parse(buf: &[u8], key: u64) -> Option<Rgba> {
    if buf.len() < HEADER || &buf[..8] != MAGIC {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes(buf[i..i + 4].try_into().unwrap_or_default());
    if u64::from_le_bytes(buf[8..16].try_into().ok()?) != key {
        return None;
    }
    let (w, h, ow, oh, flags, len) = (u32_at(16), u32_at(20), u32_at(24), u32_at(28), u32_at(32), u32_at(36) as usize);
    if w == 0 || h == 0 || w > MAX_SIDE * 2 || h > MAX_SIDE * 2 || buf.len() != HEADER + len {
        return None;
    }
    let px = qoi_decode(&buf[HEADER..], (w * h) as usize)?;
    Some(Rgba { w, h, px, premultiplied: flags & 1 != 0, orig: (flags & 2 != 0).then_some((ow, oh)), pages: None })
}

fn encode(key: u64, img: &Rgba) -> Vec<u8> {
    let body = qoi_encode(&img.px);
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&key.to_le_bytes());
    let (ow, oh) = img.orig.unwrap_or((0, 0));
    let flags = u32::from(img.premultiplied) | if img.orig.is_some() { 2 } else { 0 };
    for v in [img.w, img.h, ow, oh, flags, body.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&body);
    out
}

enum Job {
    Put(u64, Rgba),
    Touch(PathBuf),
}

fn writer() -> Option<&'static Sender<Job>> {
    static W: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    W.get_or_init(|| {
        let root = dir()?.clone();
        let (tx, rx) = unbounded::<Job>();
        std::thread::Builder::new()
            .name("cache-miniaturas".into())
            .spawn(move || {
                // Lo que ocupa la caché (se cuenta la primera vez que hace falta).
                let mut total: Option<u64> = None;
                let mut seq = 0u64;
                for job in rx {
                    match job {
                        Job::Touch(file) => {
                            let old = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() > 3600);
                            if old && let Ok(f) = std::fs::OpenOptions::new().write(true).open(&file) {
                                let _ = f.set_modified(std::time::SystemTime::now());
                            }
                        }
                        Job::Put(key, img) => {
                            let Some(file) = file_of(key) else { continue };
                            let bytes = encode(key, &img);
                            let Some(parent) = file.parent() else { continue };
                            let _ = std::fs::create_dir_all(parent);
                            seq += 1;
                            let tmp = parent.join(format!("{key:016x}.{}-{seq}.tmp", std::process::id()));
                            let ok = std::fs::File::create(&tmp).and_then(|mut f| f.write_all(&bytes)).is_ok() && std::fs::rename(&tmp, &file).is_ok();
                            if !ok {
                                let _ = std::fs::remove_file(&tmp);
                                continue;
                            }
                            let t = total.get_or_insert_with(|| size_of_dir(&root));
                            *t += bytes.len() as u64;
                            if *t > MAX_BYTES {
                                *t = trim(&root, (MAX_BYTES as f64 * TRIM_TO) as u64);
                            }
                        }
                    }
                }
            })
            .ok()?;
        Some(tx)
    })
    .as_ref()
}

/// Archivos de la caché: (ruta, bytes, última vez usado).
fn entries(root: &std::path::Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut v = Vec::new();
    for sub in std::fs::read_dir(root).into_iter().flatten().flatten() {
        for e in std::fs::read_dir(sub.path()).into_iter().flatten().flatten() {
            if let Ok(m) = e.metadata() {
                v.push((e.path(), m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    v
}

fn size_of_dir(root: &std::path::Path) -> u64 {
    entries(root).iter().map(|e| e.1).sum()
}

/// Borra lo usado hace más tiempo hasta quedar en `target` bytes. Devuelve lo que queda.
fn trim(root: &std::path::Path, target: u64) -> u64 {
    let mut v = entries(root);
    let mut total: u64 = v.iter().map(|e| e.1).sum();
    v.sort_by_key(|e| e.2);
    for (p, len, _) in v {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
    total
}

// ---------------------------------------------------------------- QOI (sin cabecera propia)

fn qoi_hash(p: [u8; 4]) -> usize {
    (p[0] as usize * 3 + p[1] as usize * 5 + p[2] as usize * 7 + p[3] as usize * 11) % 64
}

/// Píxeles RGBA en QOI («Quite OK Image»): sin pérdida y rápido en los dos sentidos.
pub fn qoi_encode(px: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(px.len() / 2);
    let mut index = [[0u8; 4]; 64];
    let mut prev = [0u8, 0, 0, 255];
    let mut run = 0u8;
    let n = px.len() / 4;
    for (i, c) in px.chunks_exact(4).enumerate() {
        let p = [c[0], c[1], c[2], c[3]];
        if p == prev {
            run += 1;
            if run == 62 || i == n - 1 {
                out.push(0xC0 | (run - 1));
                run = 0;
            }
            continue;
        }
        if run > 0 {
            out.push(0xC0 | (run - 1));
            run = 0;
        }
        let h = qoi_hash(p);
        if index[h] == p {
            out.push(h as u8);
        } else {
            index[h] = p;
            if p[3] == prev[3] {
                let dr = p[0].wrapping_sub(prev[0]) as i8;
                let dg = p[1].wrapping_sub(prev[1]) as i8;
                let db = p[2].wrapping_sub(prev[2]) as i8;
                let (dr_dg, db_dg) = (dr.wrapping_sub(dg), db.wrapping_sub(dg));
                if (-2..=1).contains(&dr) && (-2..=1).contains(&dg) && (-2..=1).contains(&db) {
                    out.push(0x40 | (((dr + 2) as u8) << 4) | (((dg + 2) as u8) << 2) | (db + 2) as u8);
                } else if (-32..=31).contains(&dg) && (-8..=7).contains(&dr_dg) && (-8..=7).contains(&db_dg) {
                    out.push(0x80 | (dg + 32) as u8);
                    out.push((((dr_dg + 8) as u8) << 4) | (db_dg + 8) as u8);
                } else {
                    out.extend_from_slice(&[0xFE, p[0], p[1], p[2]]);
                }
            } else {
                out.extend_from_slice(&[0xFF, p[0], p[1], p[2], p[3]]);
            }
        }
        prev = p;
    }
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    out
}

/// Lo contrario de `qoi_encode`: exactamente `n` píxeles o None (dañado).
pub fn qoi_decode(b: &[u8], n: usize) -> Option<Vec<u8>> {
    if !b.ends_with(&[0, 0, 0, 0, 0, 0, 0, 1]) {
        return None;
    }
    let b = &b[..b.len() - 8];
    let mut out = Vec::with_capacity(n * 4);
    let mut index = [[0u8; 4]; 64];
    let mut p = [0u8, 0, 0, 255];
    let mut i = 0;
    while out.len() < n * 4 {
        let op = *b.get(i)?;
        i += 1;
        let mut run = 1;
        match op {
            0xFE => {
                p[..3].copy_from_slice(b.get(i..i + 3)?);
                i += 3;
            }
            0xFF => {
                p.copy_from_slice(b.get(i..i + 4)?);
                i += 4;
            }
            _ => match op >> 6 {
                0 => p = index[op as usize],
                1 => {
                    p[0] = p[0].wrapping_add((op >> 4) & 3).wrapping_sub(2);
                    p[1] = p[1].wrapping_add((op >> 2) & 3).wrapping_sub(2);
                    p[2] = p[2].wrapping_add(op & 3).wrapping_sub(2);
                }
                2 => {
                    let b2 = *b.get(i)?;
                    i += 1;
                    let dg = (op & 0x3F).wrapping_sub(32);
                    p[0] = p[0].wrapping_add(dg).wrapping_add(b2 >> 4).wrapping_sub(8);
                    p[1] = p[1].wrapping_add(dg);
                    p[2] = p[2].wrapping_add(dg).wrapping_add(b2 & 0xF).wrapping_sub(8);
                }
                _ => run = (op & 0x3F) as usize + 1,
            },
        }
        index[qoi_hash(p)] = p;
        for _ in 0..run {
            out.extend_from_slice(&p);
        }
    }
    (out.len() == n * 4 && i == b.len()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// QOI ida y vuelta, sin pérdida, con corridas, diferencias chicas y medianas, alfa e índice;
    /// y lo dañado no se acepta.
    #[test]
    fn qoi_roundtrip_and_corruption() {
        let (w, h) = (37usize, 23usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let v = match (x / 5 + y / 3) % 5 {
                    0 => [10, 10, 10, 255],
                    1 => [(x * 7) as u8, (y * 3) as u8, 200, 255],
                    2 => [(x * 31 + y * 17) as u8, (x * 13) as u8, (y * 29) as u8, (x * 5) as u8],
                    3 => [100 + (x % 2) as u8, 101, 99, 255],
                    _ => [255, 0, 0, 128],
                };
                px.extend_from_slice(&v);
            }
        }
        let enc = qoi_encode(&px);
        assert_eq!(qoi_decode(&enc, w * h).as_deref(), Some(px.as_slice()));
        assert!(qoi_decode(&enc[..enc.len() - 1], w * h).is_none());
        assert!(qoi_decode(&enc, w * h + 1).is_none());
        // Un archivo entero: cabecera y contenido; con otra clave o cortado, no.
        let img = Rgba { w: w as u32, h: h as u32, px: px.clone(), premultiplied: true, orig: Some((7728, 5152)), pages: None };
        let file = encode(42, &img);
        let back = parse(&file, 42).expect("sano");
        assert_eq!((back.w, back.h, back.orig, back.premultiplied), (w as u32, h as u32, Some((7728, 5152)), true));
        assert_eq!(back.px, px);
        assert!(parse(&file, 43).is_none());
        assert!(parse(&file[..file.len() - 3], 42).is_none());
        assert_ne!(stamp_of(1, 2), stamp_of(2, 1));
    }
}
