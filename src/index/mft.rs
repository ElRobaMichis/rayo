//! Lectura de la Master File Table (MFT) de NTFS en crudo.
//!
//! En vez de abrir carpeta por carpeta, se lee la tabla completa del volumen
//! en bloques grandes y secuenciales, y se interpreta cada registro en paralelo.
//! Trae nombre, carpeta padre, tamaño, fecha y atributos de todos los archivos.
//! Requiere permisos de administrador.

use super::store::*;
use parking_lot::Mutex;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::core::PCWSTR;

const CHUNK: usize = 8 << 20;
const ROOT_RECORD: u32 = 5;
const FIRST_USER_RECORD: u32 = 24;

struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn new(len: usize) -> Self {
        let layout = Layout::from_size_align(len.max(4096), 4096).unwrap();
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        AlignedBuf { ptr, len }
    }
    fn as_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, Layout::from_size_align(self.len.max(4096), 4096).unwrap()) }
    }
}

struct Volume(HANDLE);

// El handle solo se usa desde el hilo lector.
unsafe impl Send for Volume {}
unsafe impl Sync for Volume {}

impl Drop for Volume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Volume {
    fn open(letter: char) -> Result<Self, String> {
        let path: Vec<u16> = format!("\\\\.\\{}:", letter).encode_utf16().chain([0]).collect();
        let h = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_NO_BUFFERING,
                None,
            )
        }
        .map_err(|e| format!("no se pudo abrir el volumen {letter}: ({e}); requiere administrador"))?;
        Ok(Volume(h))
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), String> {
        unsafe {
            SetFilePointerEx(self.0, offset as i64, None, FILE_BEGIN).map_err(|e| e.to_string())?;
            let mut read = 0u32;
            ReadFile(self.0, Some(buf), Some(&mut read), None).map_err(|e| e.to_string())?;
            if (read as usize) < buf.len() {
                return Err("lectura incompleta".into());
            }
        }
        Ok(())
    }
}

#[inline]
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline]
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Aplica las "update sequence" (fixups) de un registro. Devuelve false si está corrupto.
fn apply_fixup(rec: &mut [u8]) -> bool {
    let usa_off = u16le(rec, 4) as usize;
    let usa_count = u16le(rec, 6) as usize;
    if usa_count < 2 || usa_off + usa_count * 2 > rec.len() {
        return false;
    }
    let stride = rec.len() / (usa_count - 1);
    let usn = [rec[usa_off], rec[usa_off + 1]];
    for k in 1..usa_count {
        let pos = k * stride - 2;
        if pos + 2 > rec.len() || rec[pos..pos + 2] != usn {
            return false;
        }
        rec[pos] = rec[usa_off + k * 2];
        rec[pos + 1] = rec[usa_off + k * 2 + 1];
    }
    true
}

/// Decodifica un runlist: lista de (LCN, clusters). LCN `None` = hueco disperso.
fn decode_runs(b: &[u8]) -> Vec<(Option<u64>, u64)> {
    let mut runs = Vec::new();
    let mut p = 0usize;
    let mut lcn: i64 = 0;
    while p < b.len() {
        let h = b[p];
        if h == 0 {
            break;
        }
        let ls = (h & 0x0F) as usize;
        let os = (h >> 4) as usize;
        p += 1;
        if p + ls + os > b.len() || ls == 0 || ls > 8 || os > 8 {
            break;
        }
        let mut len = 0u64;
        for i in 0..ls {
            len |= (b[p + i] as u64) << (8 * i);
        }
        p += ls;
        if os == 0 {
            runs.push((None, len));
        } else {
            let mut off: i64 = 0;
            for i in 0..os {
                off |= (b[p + i] as i64) << (8 * i);
            }
            // Extensión de signo.
            let shift = 64 - 8 * os as u32;
            off = (off << shift) >> shift;
            lcn += off;
            runs.push((Some(lcn as u64), len));
        }
        p += os;
    }
    runs
}

#[derive(Clone, Copy, Default)]
struct Meta {
    in_use: bool,
    dir: bool,
    attrs: u32,
    size: u64,
    has_size: bool,
    mtime: u32,
}

struct ChunkOut {
    first_rec: u32,
    metas: Vec<Meta>,
    names: String,
    /// (registro, registro padre, fin del nombre en `names`)
    items: Vec<(u32, u32, u32)>,
    /// Tamaños encontrados en registros de extensión: (registro base, tamaño)
    ext_sizes: Vec<(u32, u64)>,
}

fn parse_chunk(buf: &mut [u8], first_rec: u32, rec_size: usize) -> ChunkOut {
    let count = buf.len() / rec_size;
    let mut out = ChunkOut {
        first_rec,
        metas: vec![Meta::default(); count],
        names: String::new(),
        items: Vec::with_capacity(count),
        ext_sizes: Vec::new(),
    };
    let mut name16 = Vec::with_capacity(256);
    for (k, rec) in buf.chunks_exact_mut(rec_size).enumerate() {
        if &rec[0..4] != b"FILE" {
            continue;
        }
        let hflags = u16le(rec, 22);
        if hflags & 1 == 0 || !apply_fixup(rec) {
            continue;
        }
        let rec_no = first_rec + k as u32;
        let base = (u64le(rec, 32) & 0xFFFF_FFFF_FFFF) as u32;
        let owner = if base != 0 { base } else { rec_no };
        let used = (u32le(rec, 24) as usize).min(rec.len());
        let mut m = Meta { in_use: true, dir: hflags & 2 != 0, ..Default::default() };
        let mut p = u16le(rec, 20) as usize;
        while p + 16 <= used {
            let ty = u32le(rec, p);
            if ty == 0xFFFF_FFFF || ty == 0 {
                break;
            }
            let len = u32le(rec, p + 4) as usize;
            if len < 16 || p + len > used {
                break;
            }
            let non_res = rec[p + 8] != 0;
            let name_len = rec[p + 9];
            let a = &rec[p..p + len];
            match ty {
                0x10 if !non_res && len >= 24 => {
                    let vo = u16le(a, 20) as usize;
                    if vo + 36 <= len {
                        m.mtime = filetime_to_unix(u64le(a, vo + 8));
                        m.attrs = u32le(a, vo + 32);
                    }
                }
                0x30 if !non_res && len >= 24 => {
                    let vo = u16le(a, 20) as usize;
                    if vo + 66 <= len {
                        let v = &a[vo..];
                        let parent = (u64le(v, 0) & 0xFFFF_FFFF_FFFF) as u32;
                        let nlen = v[64] as usize;
                        let ns = v[65];
                        if ns != 2 && 66 + nlen * 2 <= v.len() {
                            name16.clear();
                            for i in 0..nlen {
                                name16.push(u16le(v, 66 + i * 2));
                            }
                            out.names.push_str(&String::from_utf16_lossy(&name16));
                            out.items.push((owner, parent, out.names.len() as u32));
                        }
                    }
                }
                0x80 if name_len == 0 => {
                    let size = if non_res {
                        if len >= 56 && u64le(a, 16) == 0 { Some(u64le(a, 48)) } else { None }
                    } else {
                        Some(u32le(a, 16) as u64)
                    };
                    if let Some(s) = size {
                        if base != 0 {
                            out.ext_sizes.push((base, s));
                        } else {
                            m.size = s;
                            m.has_size = true;
                        }
                    }
                }
                _ => {}
            }
            p += len;
        }
        if base == 0 {
            out.metas[k] = m;
        }
    }
    out
}

/// Lee la MFT del volumen y construye el índice.
pub fn read_mft(letter: char, progress: &AtomicUsize) -> Result<VolumeIndex, String> {
    let letter = letter.to_ascii_uppercase();
    let vol = Volume::open(letter)?;
    // Posición del journal ANTES de leer: lo que cambie durante la lectura se
    // vuelve a aplicar después (aplicar dos veces un cambio no rompe nada).
    let journal = super::usn::Volume::open(letter).ok().and_then(|j| j.journal().ok());

    let mut boot = AlignedBuf::new(4096);
    vol.read_at(0, boot.as_mut())?;
    let b = boot.as_mut();
    if &b[3..11] != b"NTFS    " {
        return Err(format!("{letter}: no es NTFS"));
    }
    let bytes_per_sector = u16le(b, 0x0B) as u64;
    let spc_raw = b[0x0D];
    let sectors_per_cluster = if spc_raw > 0x80 { 1u64 << (256 - spc_raw as u32) } else { spc_raw as u64 };
    let cluster = bytes_per_sector * sectors_per_cluster;
    let mft_lcn = u64le(b, 0x30);
    let cpfr = b[0x40] as i8;
    let rec_size = if cpfr < 0 { 1usize << (-cpfr as u32) } else { cpfr as usize * cluster as usize };
    if rec_size == 0 || cluster == 0 || (cluster as usize) < rec_size {
        return Err("geometría NTFS no soportada".into());
    }

    // Registro 0 = el propio $MFT: de ahí sale dónde está guardada la tabla.
    let mut first = AlignedBuf::new(cluster as usize);
    vol.read_at(mft_lcn * cluster, first.as_mut())?;
    let rec0 = &mut first.as_mut()[..rec_size];
    if &rec0[0..4] != b"FILE" || !apply_fixup(rec0) {
        return Err("registro $MFT inválido".into());
    }
    let mut runs = Vec::new();
    let mut mft_bytes = 0u64;
    let mut p = u16le(rec0, 20) as usize;
    while p + 16 <= rec_size {
        let ty = u32le(rec0, p);
        let len = u32le(rec0, p + 4) as usize;
        if ty == 0xFFFF_FFFF || len < 16 || p + len > rec_size {
            break;
        }
        if ty == 0x80 && rec0[p + 8] != 0 && rec0[p + 9] == 0 {
            let a = &rec0[p..p + len];
            mft_bytes = u64le(a, 48);
            runs = decode_runs(&a[u16le(a, 32) as usize..]);
            break;
        }
        p += len;
    }
    if runs.is_empty() {
        return Err("no se encontró el runlist de $MFT".into());
    }
    let total_recs = (mft_bytes / rec_size as u64) as usize;

    // Lectura secuencial en un hilo; interpretación en paralelo.
    let results = Mutex::new(Vec::<ChunkOut>::new());
    let mut io_err = None;
    rayon::scope(|s| {
        let mut rec_cursor = 0u64;
        for &(lcn, clusters) in &runs {
            let run_bytes = clusters * cluster;
            let run_recs = run_bytes / rec_size as u64;
            let Some(lcn) = lcn else {
                rec_cursor += run_recs;
                continue;
            };
            let mut done = 0u64;
            while done < run_bytes && (rec_cursor as usize) < total_recs {
                let remaining_recs = (total_recs as u64 - rec_cursor) * rec_size as u64;
                let len = (run_bytes - done).min(CHUNK as u64).min(remaining_recs.next_multiple_of(cluster));
                let mut buf = AlignedBuf::new(len as usize);
                if let Err(e) = vol.read_at(lcn * cluster + done, buf.as_mut()) {
                    io_err = Some(e);
                    return;
                }
                let first_rec = rec_cursor as u32;
                let results = &results;
                s.spawn(move |_| {
                    let mut buf = buf;
                    let usable = (buf.len / rec_size) * rec_size;
                    let out = parse_chunk(&mut buf.as_mut()[..usable], first_rec, rec_size);
                    progress.fetch_add(out.items.len(), Ordering::Relaxed);
                    results.lock().push(out);
                });
                done += len;
                rec_cursor += len / rec_size as u64;
            }
        }
    });
    if let Some(e) = io_err {
        return Err(e);
    }
    let mut chunks = results.into_inner();
    chunks.sort_unstable_by_key(|c| c.first_rec);

    // Unir metadatos.
    let mut metas = vec![Meta::default(); total_recs];
    for c in &chunks {
        let start = c.first_rec as usize;
        let end = (start + c.metas.len()).min(total_recs);
        metas[start..end].copy_from_slice(&c.metas[..end - start]);
    }
    for c in &chunks {
        for &(base, s) in &c.ext_sizes {
            if let Some(m) = metas.get_mut(base as usize) {
                if !m.has_size {
                    m.size = s;
                    m.has_size = true;
                }
            }
        }
    }

    let n_items: usize = chunks.iter().map(|c| c.items.len()).sum();
    let n_bytes: usize = chunks.iter().map(|c| c.names.len() + c.items.len()).sum();
    let mut v = VolumeIndex::with_capacity(letter, Source::Mft, n_items + 1, n_bytes + 16);
    let mut rec_entry = vec![NONE; total_recs];
    rec_entry[ROOT_RECORD as usize] = 0;
    let mut parent_rec = Vec::with_capacity(n_items + 1);
    parent_rec.push(NONE);
    for c in &chunks {
        let mut start = 0usize;
        for &(rec, parent, end) in &c.items {
            let name = &c.names[start..end as usize];
            start = end as usize;
            if rec == ROOT_RECORD || rec < FIRST_USER_RECORD {
                continue;
            }
            let Some(m) = metas.get(rec as usize) else { continue };
            if !m.in_use {
                continue;
            }
            let mut flags = super::walk::attrs_to_flags(m.attrs);
            if m.dir {
                flags |= F_DIR;
            } else {
                flags &= !F_DIR;
            }
            let id = v.push(name, NONE, flags, if m.dir { 0 } else { m.size }, m.mtime);
            parent_rec.push(parent);
            if m.dir && rec_entry[rec as usize] == NONE {
                rec_entry[rec as usize] = id;
            }
        }
    }
    drop(chunks);
    drop(metas);

    // Resolver padres y descartar entradas huérfanas (no alcanzables desde la raíz).
    let n = v.len();
    for i in 1..n {
        let pr = parent_rec[i] as usize;
        v.parent[i] = rec_entry.get(pr).copied().unwrap_or(NONE);
    }
    if let Some(j) = journal {
        v.usn_journal = j.id;
        v.usn_next = j.next;
        v.dir_map = rec_entry.iter().enumerate().filter(|&(rec, &e)| e != NONE && e != 0 && rec as u32 != ROOT_RECORD).map(|(rec, &e)| (rec as u32, e)).collect();
    }
    drop(rec_entry);
    drop(parent_rec);
    v.link_children();
    let mut reachable = vec![false; n];
    reachable[0] = true;
    let mut stack = vec![0u32];
    while let Some(d) = stack.pop() {
        let mut c = v.first_child[d as usize];
        while c != NONE {
            if !reachable[c as usize] {
                reachable[c as usize] = true;
                if v.flags[c as usize] & F_DIR != 0 {
                    stack.push(c);
                }
            }
            c = v.next_sibling[c as usize];
        }
    }
    for (i, r) in reachable.iter().enumerate() {
        if !r {
            v.flags[i] |= F_DELETED;
            v.deleted += 1;
        }
    }
    v.dir_map.retain(|_, e| reachable[*e as usize]);
    v.compute_dir_sizes();
    v.shrink();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs() {
        // 0x21: 1 byte de longitud, 2 de offset.
        let r = decode_runs(&[0x21, 0x10, 0x00, 0x01, 0x11, 0x08, 0xF0, 0x00]);
        assert_eq!(r, vec![(Some(0x100), 0x10), (Some(0x100 - 0x10), 0x08)]);
    }
}
