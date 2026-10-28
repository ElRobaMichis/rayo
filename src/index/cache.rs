//! Caché del índice en disco: los arreglos se escriben tal cual, así que
//! cargarlo es prácticamente una copia de memoria.

use super::store::*;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"RAYOIDX5";

pub fn data_dir() -> PathBuf {
    let d = crate::instance::custom_data_dir()
        .map(|d| d.join("cache"))
        .unwrap_or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Rayo"));
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn cache_path(letter: char) -> PathBuf {
    data_dir().join(format!("index-{}.bin", letter.to_ascii_uppercase()))
}

fn as_bytes<T: Copy>(s: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}

fn read_vec<T: Copy + Default>(r: &mut impl Read, len: usize) -> std::io::Result<Vec<T>> {
    let mut v = vec![T::default(); len];
    let bytes = unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, std::mem::size_of_val(v.as_slice())) };
    r.read_exact(bytes)?;
    Ok(v)
}

pub fn save(v: &VolumeIndex) -> std::io::Result<()> {
    let path = cache_path(v.letter);
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        write_index(&mut w, v)?;
        // Al disco de verdad antes de reemplazar el archivo: si se corta la luz justo
        // después, el renombrado no puede quedar apuntando a datos que nunca se escribieron.
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

/// Escribe el índice completo (a un archivo o, desde el servicio, a la tubería).
pub fn write_index(w: &mut impl Write, v: &VolumeIndex) -> std::io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&(v.letter as u32).to_le_bytes())?;
    w.write_all(&[v.source as u8])?;
    for n in [v.len(), v.lower.len(), v.upper.len()] {
        w.write_all(&(n as u64).to_le_bytes())?;
    }
    w.write_all(&v.lower)?;
    w.write_all(as_bytes(&v.upper))?;
    w.write_all(as_bytes(&v.name_off))?;
    w.write_all(as_bytes(&v.parent))?;
    w.write_all(&v.flags)?;
    w.write_all(as_bytes(&v.size))?;
    w.write_all(as_bytes(&v.mtime))?;
    // Acentos: ids, inicios y parches.
    w.write_all(&(v.acc_ids.len() as u64).to_le_bytes())?;
    w.write_all(&(v.acc_data.len() as u64).to_le_bytes())?;
    w.write_all(as_bytes(&v.acc_ids))?;
    w.write_all(as_bytes(&v.acc_start))?;
    w.write_all(as_bytes(&v.acc_data))?;
    // Journal y carpetas por número de registro (para ponerse al día al abrir).
    w.write_all(&v.usn_journal.to_le_bytes())?;
    w.write_all(&v.usn_next.to_le_bytes())?;
    let pairs: Vec<[u32; 2]> = v.dir_map.iter().map(|(&r, &e)| [r, e]).collect();
    w.write_all(&(pairs.len() as u64).to_le_bytes())?;
    w.write_all(as_bytes(&pairs))?;
    Ok(())
}

/// Lee solo el origen guardado en la caché (MFT o recorrido).
pub fn load_source(letter: char) -> Option<Source> {
    let mut f = File::open(cache_path(letter)).ok()?;
    let mut head = [0u8; 13];
    f.read_exact(&mut head).ok()?;
    Some(match head[12] {
        1 => Source::Mft,
        2 => Source::Walk,
        _ => Source::Cache,
    })
}

pub fn load(letter: char) -> std::io::Result<VolumeIndex> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(cache_path(letter))?);
    let res = read_index(&mut r, letter);
    if res.is_err() {
        // Dañada (p. ej. un corte de luz mientras se guardaba): se borra y el índice se
        // vuelve a armar desde el disco.
        drop(r);
        let _ = std::fs::remove_file(cache_path(letter));
    }
    res
}

/// Revisa que lo leído tenga sentido antes de usarlo: un archivo dañado no puede tumbar
/// a Rayo (los nombres y las carpetas se usan como posiciones dentro de los arreglos).
fn validate(v: &VolumeIndex) -> std::io::Result<()> {
    let bad = |m: &str| Err(std::io::Error::other(format!("caché dañada: {m}")));
    let n = v.name_off.len();
    if n == 0 {
        return Ok(());
    }
    let lower = v.lower.len();
    if lower == 0 || v.lower[lower - 1] != 0 {
        return bad("nombres");
    }
    let mut prev: Option<u32> = None;
    for &o in &v.name_off {
        // Cada nombre termina en 0, así que los inicios crecen siempre.
        if o as usize >= lower || prev.is_some_and(|p| o <= p) {
            return bad("inicio de nombre");
        }
        prev = Some(o);
    }
    if v.parent.iter().any(|&p| p != NONE && p as usize >= n) {
        return bad("carpeta padre");
    }
    if v.acc_ids.windows(2).any(|w| w[0] >= w[1]) || v.acc_ids.last().is_some_and(|&i| i as usize >= n) {
        return bad("acentos");
    }
    if v.acc_start.windows(2).any(|w| w[0] > w[1]) || v.acc_start.last().is_some_and(|&s| s as usize > v.acc_data.len()) {
        return bad("acentos");
    }
    if v.dir_map.values().any(|&e| e as usize >= n) {
        return bad("carpetas");
    }
    Ok(())
}

/// Lee un índice escrito con `write_index`.
pub fn read_index(r: &mut impl Read, letter: char) -> std::io::Result<VolumeIndex> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("versión de caché distinta"));
    }
    let mut b4 = [0u8; 4];
    r.read_exact(&mut b4)?;
    let mut src = [0u8; 1];
    r.read_exact(&mut src)?;
    let mut counts = [0usize; 3];
    for c in &mut counts {
        let mut b8 = [0u8; 8];
        r.read_exact(&mut b8)?;
        *c = u64::from_le_bytes(b8) as usize;
    }
    let [n, lower_len, upper_len] = counts;
    let mut v = VolumeIndex {
        letter: char::from_u32(u32::from_le_bytes(b4)).unwrap_or(letter),
        source: match src[0] {
            1 => Source::Mft,
            2 => Source::Walk,
            _ => Source::Cache,
        },
        ..Default::default()
    };
    v.lower = read_vec(r, lower_len)?;
    v.upper = read_vec(r, upper_len)?;
    v.name_off = read_vec(r, n)?;
    v.parent = read_vec(r, n)?;
    v.flags = read_vec(r, n)?;
    v.size = read_vec(r, n)?;
    v.mtime = read_vec(r, n)?;
    let mut b8 = [0u8; 8];
    r.read_exact(&mut b8)?;
    let n_ids = u64::from_le_bytes(b8) as usize;
    r.read_exact(&mut b8)?;
    let n_data = u64::from_le_bytes(b8) as usize;
    v.acc_ids = read_vec(r, n_ids)?;
    v.acc_start = read_vec(r, n_ids)?;
    v.acc_data = read_vec(r, n_data)?;
    r.read_exact(&mut b8)?;
    v.usn_journal = u64::from_le_bytes(b8);
    r.read_exact(&mut b8)?;
    v.usn_next = i64::from_le_bytes(b8);
    r.read_exact(&mut b8)?;
    let n_pairs = u64::from_le_bytes(b8) as usize;
    let pairs: Vec<[u32; 2]> = read_vec(r, n_pairs)?;
    v.dir_map = pairs.into_iter().map(|[r, e]| (r, e)).collect();
    validate(&v)?;
    v.deleted = v.flags.iter().filter(|&&f| f & F_DELETED != 0).count();
    v.link_children();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VolumeIndex {
        // `new` ya trae la raíz (entrada 0).
        let mut v = VolumeIndex::new('Z', Source::Walk);
        let dir = v.push("Fotos", 0, F_DIR, 0, 0);
        v.push("Año nuevo.jpg", dir, 0, 1234, 0);
        v
    }

    /// Un índice sano va y vuelve igual.
    #[test]
    fn roundtrip() {
        let mut buf = Vec::new();
        write_index(&mut buf, &sample()).unwrap();
        let v = read_index(&mut buf.as_slice(), 'Z').unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v.name(2), "Año nuevo.jpg");
    }

    /// Un índice dañado (como el que deja un corte de luz: partes en ceros o cortado)
    /// se rechaza con un error, nunca con una caída.
    #[test]
    fn damaged_is_rejected() {
        let mut buf = Vec::new();
        write_index(&mut buf, &sample()).unwrap();
        // Todo en ceros salvo la cabecera.
        let mut zeros = buf.clone();
        for b in &mut zeros[37..] {
            *b = 0;
        }
        assert!(read_index(&mut zeros.as_slice(), 'Z').is_err());
        // Cortado a la mitad.
        let half = &buf[..buf.len() / 2];
        assert!(read_index(&mut &half[..], 'Z').is_err());
        // Una carpeta padre imposible.
        let mut v = sample();
        v.parent[2] = 99;
        let mut bad = Vec::new();
        write_index(&mut bad, &v).unwrap();
        assert!(read_index(&mut bad.as_slice(), 'Z').is_err());
    }
}
