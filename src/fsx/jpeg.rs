//! Lo que una foto JPEG trae adentro, leído de una sola vez del principio del archivo.
//!
//! Una foto de cámara de 26 MB guarda, en sus primeros ~120 KB, todo lo que hace falta para
//! mostrarla enseguida: la orientación (EXIF), las medidas del original (el SOF) y una o dos
//! vistas previas ya comprimidas. Leer eso de un disco mecánico es un salto del cabezal y
//! 256 KB (~8 ms); la foto entera son ~120 ms más decodificarla. Así la cuadrícula, el panel y
//! el primer cuadro de la Vista rápida no esperan a la foto.
//!
//! - EXIF IFD1: la miniatura de siempre (160×120 en casi todas las cámaras).
//! - FlashPix (APP2 «FPXR»): Fujifilm guarda ahí una de 640×480.
//! - MPF (APP2 «MPF»): Sony, Nikon, Panasonic… guardan una grande (1616×1080, 1920×1280) al
//!   final del archivo; cuesta un segundo salto del cabezal, y solo se lee si la piden.

use std::io::{Read, Seek, SeekFrom};

/// Lo que se lee de una vez del principio del archivo.
const HEAD: usize = 256 << 10;
/// Tope de una vista previa MPF (las de verdad pesan 0,2–1,5 MB).
const MPF_MAX: u32 = 4 << 20;
/// Lado que se le supone a una vista previa MPF antes de decodificarla.
const MPF_GUESS: u32 = 1600;

/// Una vista previa incrustada: sus bytes (un JPEG completo) o dónde están (MPF, más adelante).
#[derive(Debug, Clone)]
pub enum Source {
    Bytes(Vec<u8>),
    At { offset: u64, len: u32 },
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub src: Source,
    /// Medidas de la vista previa (sin girar), si se conocen sin decodificarla.
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Default)]
pub struct Peek {
    /// Orientación EXIF (1–8).
    pub orient: u16,
    /// Medidas del original tal como está guardado (sin girar).
    pub size: Option<(u32, u32)>,
    /// Vistas previas, de la más chica a la más grande.
    pub previews: Vec<Preview>,
}

impl Peek {
    /// Medidas del original como se ve (girado según EXIF).
    pub fn shown_size(&self) -> Option<(u32, u32)> {
        self.size.map(|(w, h)| if matches!(self.orient, 5..=8) { (h, w) } else { (w, h) })
    }

    /// La vista previa más chica cuyo lado mayor llega a `side` (o la más grande, si ninguna).
    pub fn best_for(&self, side: u32) -> Option<&Preview> {
        self.previews.iter().find(|p| p.w.max(p.h) >= side).or(self.previews.last())
    }
}

/// Lee el principio de un JPEG y saca lo que tiene. None si no es un JPEG.
pub fn peek(path: &str) -> Option<Peek> {
    use std::os::windows::fs::OpenOptionsExt;
    // Sin lectura anticipada (FILE_FLAG_RANDOM_ACCESS): de una foto de 26 MB solo se quiere el
    // principio, y en un disco mecánico lo que Windows leería de más es tiempo del cabezal.
    let mut f = std::fs::OpenOptions::new().read(true).custom_flags(0x1000_0000).open(super::long_path(path)).ok()?;
    let mut buf = vec![0u8; HEAD];
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(_) => return None,
        }
    }
    buf.truncate(n);
    let mut p = parse(&buf)?;
    // Vistas previas MPF que caen dentro de lo ya leído: se copian (sin segundo salto).
    for pv in &mut p.previews {
        if let Source::At { offset, len } = pv.src {
            let end = offset as usize + len as usize;
            if end <= buf.len() {
                pv.src = Source::Bytes(buf[offset as usize..end].to_vec());
            }
        }
    }
    Some(p)
}

/// Los bytes de una vista previa (leyendo el archivo si es MPF y está más adelante).
pub fn bytes_of(path: &str, pv: &Preview) -> Option<Vec<u8>> {
    match &pv.src {
        Source::Bytes(b) => Some(b.clone()),
        Source::At { offset, len } => {
            let mut f = std::fs::File::open(super::long_path(path)).ok()?;
            f.seek(SeekFrom::Start(*offset)).ok()?;
            let mut b = vec![0u8; *len as usize];
            f.read_exact(&mut b).ok()?;
            b.starts_with(&[0xFF, 0xD8]).then_some(b)
        }
    }
}

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

/// Recorre los segmentos hasta el comienzo de la imagen (SOS).
pub fn parse(b: &[u8]) -> Option<Peek> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut out = Peek { orient: 1, ..Default::default() };
    // Trozos de flujos FlashPix: (índice, posición, datos).
    let mut fpxr: Vec<(u16, u32, &[u8])> = Vec::new();
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            break;
        }
        let m = b[i + 1];
        if m == 0xFF {
            i += 1;
            continue;
        }
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            i += 2;
            continue;
        }
        let len = be16(b, i + 2)? as usize;
        if len < 2 {
            break;
        }
        let seg_start = i + 4;
        let seg_end = (i + 2 + len).min(b.len());
        let seg = &b[seg_start..seg_end];
        match m {
            0xE1 if seg.starts_with(b"Exif\0\0") => exif(&b[seg_start + 6..seg_end], seg_start + 6, &mut out),
            0xE2 if seg.starts_with(b"FPXR\0") && seg.len() > 13 && seg[6] == 2 => {
                let idx = u16::from_be_bytes([seg[7], seg[8]]);
                let off = u32::from_be_bytes([seg[9], seg[10], seg[11], seg[12]]);
                fpxr.push((idx, off, &seg[13..]));
            }
            0xE2 if seg.starts_with(b"MPF\0") => mpf(&seg[4..], (seg_start + 4) as u64, &mut out),
            // SOF0–SOF15 (menos DHT, JPG y DAC): medidas del original.
            0xC0..=0xCF if !matches!(m, 0xC4 | 0xC8 | 0xCC) => {
                let (h, w) = (be16(seg, 1)? as u32, be16(seg, 3)? as u32);
                if w > 0 && h > 0 {
                    out.size = Some((w, h));
                }
            }
            0xDA => break,
            _ => {}
        }
        i += 2 + len;
    }
    // Flujos FlashPix: se arman por índice y se busca un JPEG adentro.
    fpxr.sort_by_key(|(idx, off, _)| (*idx, *off));
    let mut k = 0;
    while k < fpxr.len() {
        let idx = fpxr[k].0;
        let mut stream: Vec<u8> = Vec::new();
        while k < fpxr.len() && fpxr[k].0 == idx {
            let (_, off, data) = fpxr[k];
            let off = off as usize;
            if off >= stream.len() {
                stream.resize(off, 0);
                stream.extend_from_slice(data);
            }
            k += 1;
        }
        if let Some(j) = embedded_jpeg(&stream) {
            out.previews.push(j);
        }
    }
    out.previews.sort_by_key(|p| p.w.max(p.h));
    Some(out)
}

/// Un JPEG completo dentro de `b` (de su SOI a su EOI), con sus medidas.
fn embedded_jpeg(b: &[u8]) -> Option<Preview> {
    let start = memchr::memmem::find(b, &[0xFF, 0xD8, 0xFF])?;
    let j = &b[start..];
    let mut i = 2;
    let mut dims = None;
    // Hasta el SOS por segmentos; después, el primer FFD9 es el final.
    loop {
        if i + 4 > j.len() || j[i] != 0xFF {
            return None;
        }
        let m = j[i + 1];
        let len = be16(j, i + 2)? as usize;
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            dims = Some((be16(j, i + 7)? as u32, be16(j, i + 5)? as u32));
        }
        i += 2 + len;
        if m == 0xDA {
            break;
        }
    }
    let end = i + memchr::memmem::find(j.get(i..)?, &[0xFF, 0xD9])? + 2;
    let (w, h) = dims?;
    Some(Preview { src: Source::Bytes(j[..end].to_vec()), w, h })
}

/// Lector de TIFF (EXIF y MPF): orden de bytes y lecturas.
struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn new(b: &[u8]) -> Option<Tiff<'_>> {
        let le = match b.get(..4)? {
            [b'I', b'I', 42, 0] => true,
            [b'M', b'M', 0, 42] => false,
            _ => return None,
        };
        Some(Tiff { b, le })
    }
    fn u16(&self, i: usize) -> Option<u16> {
        let s: [u8; 2] = self.b.get(i..i + 2)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    }
    fn u32(&self, i: usize) -> Option<u32> {
        let s: [u8; 4] = self.b.get(i..i + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
    /// Entradas de un IFD: (etiqueta, tipo, cantidad, posición del valor) y el siguiente IFD.
    fn ifd(&self, at: usize) -> Option<(Vec<(u16, u16, u32, usize)>, u32)> {
        let n = self.u16(at)? as usize;
        if n > 512 {
            return None;
        }
        let mut v = Vec::with_capacity(n);
        for k in 0..n {
            let e = at + 2 + k * 12;
            v.push((self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?, e + 8));
        }
        let next = self.u32(at + 2 + n * 12).unwrap_or(0);
        Some((v, next))
    }
    /// Un valor entero corto o largo de una entrada.
    fn value(&self, typ: u16, at: usize) -> Option<u32> {
        match typ {
            3 => self.u16(at).map(u32::from),
            4 => self.u32(at),
            _ => None,
        }
    }
}

/// EXIF: orientación (IFD0) y miniatura (IFD1). `base`: dónde empieza el TIFF en el archivo.
fn exif(b: &[u8], _base: usize, out: &mut Peek) {
    let Some(t) = Tiff::new(b) else { return };
    let Some(ifd0) = t.u32(4) else { return };
    let Some((entries, next)) = t.ifd(ifd0 as usize) else { return };
    for (tag, typ, _, at) in entries {
        if tag == 0x0112
            && let Some(v) = t.value(typ, at)
            && (1..=8).contains(&v)
        {
            out.orient = v as u16;
        }
    }
    if next == 0 {
        return;
    }
    let Some((entries, _)) = t.ifd(next as usize) else { return };
    let get = |tag: u16| entries.iter().find(|e| e.0 == tag).and_then(|&(_, typ, _, at)| t.value(typ, at));
    if let (Some(off), Some(len)) = (get(0x0201), get(0x0202))
        && let Some(j) = b.get(off as usize..off as usize + len as usize)
        && let Some(pv) = embedded_jpeg(j)
    {
        out.previews.push(pv);
    }
}

/// MPF: la imagen grande de vista previa (la de índice 1 en adelante). `base`: dónde empieza el
/// TIFF del MPF en el archivo (los desplazamientos de las imágenes son desde ahí).
fn mpf(b: &[u8], base: u64, out: &mut Peek) {
    let Some(t) = Tiff::new(b) else { return };
    let Some(ifd0) = t.u32(4) else { return };
    let Some((entries, _)) = t.ifd(ifd0 as usize) else { return };
    let Some(&(_, _, count, at)) = entries.iter().find(|e| e.0 == 0xB002) else { return };
    let Some(list) = t.u32(at) else { return };
    for k in 1..(count as usize / 16) {
        let e = list as usize + k * 16;
        let (Some(attr), Some(len), Some(off)) = (t.u32(e), t.u32(e + 4), t.u32(e + 8)) else { continue };
        // Tipo (los 24 bits bajos): 0x010001 / 0x010002 = vista previa grande.
        if !matches!(attr & 0xFF_FFFF, 0x01_0001 | 0x01_0002) || len == 0 || len > MPF_MAX || off == 0 {
            continue;
        }
        // Las medidas no están en el índice: se supone una grande (1616×1080, 1920×1280…); las
        // de verdad salen al decodificarla.
        out.previews.push(Preview { src: Source::At { offset: base + off as u64, len }, w: MPF_GUESS, h: MPF_GUESS * 2 / 3 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un JPEG mínimo «de mentira» con SOF y SOS: alcanza para el analizador.
    fn tiny_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        j.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
        j.extend_from_slice(&h.to_be_bytes());
        j.extend_from_slice(&w.to_be_bytes());
        j.extend_from_slice(&[1, 1, 0x11, 0]);
        j.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
        j.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    /// EXIF (orientación 6 y miniatura en IFD1), FlashPix partido en dos segmentos y el SOF del
    /// original: se encuentra todo.
    #[test]
    fn finds_exif_fpxr_and_size() {
        let thumb = tiny_jpeg(160, 120);
        // TIFF big-endian: IFD0 con orientación, IFD1 con la miniatura.
        let mut t = b"MM\0\x2A\0\0\0\x08".to_vec();
        t.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        let ifd1 = t.len() as u32 + 4;
        t.extend_from_slice(&ifd1.to_be_bytes());
        let data_at = ifd1 + 2 + 2 * 12 + 4;
        t.extend_from_slice(&[0, 2]);
        t.extend_from_slice(&[0x02, 0x01, 0, 4, 0, 0, 0, 1]);
        t.extend_from_slice(&data_at.to_be_bytes());
        t.extend_from_slice(&[0x02, 0x02, 0, 4, 0, 0, 0, 1]);
        t.extend_from_slice(&(thumb.len() as u32).to_be_bytes());
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(&thumb);
        let mut f = vec![0xFF, 0xD8];
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&t);
        f.extend_from_slice(&[0xFF, 0xE1]);
        f.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        f.extend_from_slice(&app1);
        // Vista previa FlashPix de 640×480 con 20 bytes de cabecera propia, en dos trozos.
        let mut stream = vec![7u8; 20];
        stream.extend_from_slice(&tiny_jpeg(640, 480));
        let (a, b) = stream.split_at(17);
        for (off, part) in [(0u32, a), (17, b)] {
            let mut seg = b"FPXR\0\0\x02\0\x01".to_vec();
            seg.extend_from_slice(&off.to_be_bytes());
            seg.extend_from_slice(part);
            f.extend_from_slice(&[0xFF, 0xE2]);
            f.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
            f.extend_from_slice(&seg);
        }
        f.extend_from_slice(&tiny_jpeg(7728, 5152)[2..]);
        let p = parse(&f).expect("es un JPEG");
        assert_eq!(p.orient, 6);
        assert_eq!(p.size, Some((7728, 5152)));
        assert_eq!(p.shown_size(), Some((5152, 7728)));
        let sides: Vec<(u32, u32)> = p.previews.iter().map(|v| (v.w, v.h)).collect();
        assert_eq!(sides, vec![(160, 120), (640, 480)]);
        assert_eq!(p.best_for(300).map(|v| v.w), Some(640));
        assert_eq!(p.best_for(100).map(|v| v.w), Some(160));
        assert_eq!(p.best_for(4000).map(|v| v.w), Some(640));
        assert!(parse(b"\x89PNG").is_none());
    }
}
