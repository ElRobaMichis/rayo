//! GIF animado con WIC: fotogramas ya compuestos sobre el lienzo y sus demoras.
//!
//! Cada fotograma de un GIF es un parche (posición y tamaño propios) que se pinta sobre lo que
//! dejó el anterior, según su «disposición»: 0 y 1 dejar, 2 borrar su rectángulo, 3 volver al
//! lienzo de antes. Sin componer, los GIF optimizados se ven rotos.

use crate::index::walk::wide;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::{VT_BOOL, VT_UI1, VT_UI2};
use windows::core::PCWSTR;

/// Presupuesto de memoria de todos los fotogramas de un GIF (la escala se fija antes de empezar).
pub const BUDGET: usize = 128 * 1024 * 1024;
/// Lado mayor máximo de un fotograma.
pub const MAX_SIDE: u32 = 1024;
/// Demoras menores que esto (ms) se muestran a `DEFAULT_DELAY`, como en los navegadores.
const MIN_DELAY: u32 = 20;
const DEFAULT_DELAY: u32 = 100;

/// Un fotograma: el lienzo completo ya compuesto, RGBA premultiplicado.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    pub delay_ms: u32,
}

/// El decodificador, solo si es un GIF (nunca baja un archivo que está solo en la nube).
fn open(path: &str) -> Option<IWICBitmapDecoder> {
    if super::is_online_only(path) {
        return None;
    }
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let w = wide(path);
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        (decoder.GetContainerFormat().ok()? == GUID_ContainerFormatGif).then_some(decoder)
    }
}

/// Un número de los metadatos (`/grctlext/Delay`, `/imgdesc/Left`…).
fn meta(r: &IWICMetadataQueryReader, q: &str) -> Option<u32> {
    unsafe {
        let w = wide(q);
        let mut pv = PROPVARIANT::default();
        r.GetMetadataByName(PCWSTR(w.as_ptr()), &mut pv).ok()?;
        let inner = &pv.Anonymous.Anonymous;
        if inner.vt == VT_UI2 {
            Some(inner.Anonymous.uiVal as u32)
        } else if inner.vt == VT_UI1 {
            Some(inner.Anonymous.bVal as u32)
        } else if inner.vt == VT_BOOL {
            Some((inner.Anonymous.boolVal.0 != 0) as u32)
        } else {
            None
        }
    }
}

/// Cantidad de fotogramas (barato: para el subtítulo y para saber si se anima).
pub fn frame_count(path: &str) -> Option<u32> {
    unsafe { open(path)?.GetFrameCount().ok() }
}

/// Tamaño de salida: lado mayor hasta `max_side` y todos los fotogramas dentro de `budget`.
/// Un GIF grande y largo se ve más chico, nunca incompleto.
fn out_size(cw: u32, ch: u32, n: u32, max_side: u32, budget: usize) -> (u32, u32) {
    let full = cw as f64 * ch as f64 * 4.0 * n.max(1) as f64;
    let s = (max_side as f64 / cw.max(ch) as f64).min((budget as f64 / full).sqrt()).min(1.0);
    (((cw as f64 * s).floor() as u32).max(1), ((ch as f64 * s).floor() as u32).max(1))
}

/// Rectángulo de un parche sobre el lienzo.
#[derive(Clone, Copy)]
struct Patch {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Recorre las filas del parche recortado al lienzo: (fila del parche, desde, hasta) en x.
fn rows(r: Patch, cw: u32, ch: u32) -> impl Iterator<Item = (u32, u32, u32)> {
    let x1 = (r.x + r.w).min(cw);
    let y1 = (r.y + r.h).min(ch);
    (r.y..y1).filter(move |_| r.x < x1).map(move |y| (y, r.x, x1))
}

/// Pinta el parche «sobre» el lienzo (premultiplicado).
fn over(canvas: &mut [u8], cw: u32, ch: u32, px: &[u8], r: Patch) {
    for (y, x0, x1) in rows(r, cw, ch) {
        for x in x0..x1 {
            let s = (((y - r.y) * r.w + (x - r.x)) * 4) as usize;
            let d = ((y * cw + x) * 4) as usize;
            let a = px[s + 3] as u32;
            if a == 255 {
                canvas[d..d + 4].copy_from_slice(&px[s..s + 4]);
            } else if a > 0 {
                for c in 0..4 {
                    canvas[d + c] = (px[s + c] as u32 + canvas[d + c] as u32 * (255 - a) / 255).min(255) as u8;
                }
            }
        }
    }
}

/// Disposición 2: el rectángulo vuelve a transparente.
fn clear(canvas: &mut [u8], cw: u32, ch: u32, r: Patch) {
    for (y, x0, x1) in rows(r, cw, ch) {
        canvas[((y * cw + x0) * 4) as usize..((y * cw + x1) * 4) as usize].fill(0);
    }
}

/// El lienzo (BGRA premultiplicado) a `dw`×`dh` en RGBA, promediando por área.
fn shrink(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![0u8; dw * dh * 4];
    // Qué filas o columnas de origen caen en cada una de destino (siempre al menos una).
    let span = |d: usize, dn: usize, sn: usize| {
        let a = d * sn / dn;
        (a, ((d + 1) * sn / dn).max(a + 1).min(sn))
    };
    for dy in 0..dh {
        let (y0, y1) = span(dy, dh, sh);
        for dx in 0..dw {
            let (x0, x1) = span(dx, dw, sw);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for p in src[(y * sw + x0) * 4..(y * sw + x1) * 4].chunks_exact(4) {
                    for c in 0..4 {
                        acc[c] += p[c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (dy * dw + dx) * 4;
            // BGRA → RGBA.
            for (c, k) in [2usize, 1, 0, 3].into_iter().enumerate() {
                out[o + c] = ((acc[k] + n / 2) / n) as u8;
            }
        }
    }
    out
}

/// Un fotograma de WIC: su parche en BGRA premultiplicado, posición, demora y disposición.
fn patch(frame: &IWICBitmapFrameDecode) -> Option<(Vec<u8>, Patch, u32, u32)> {
    unsafe {
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        let m = frame.GetMetadataQueryReader().ok();
        let get = |q: &str| m.as_ref().and_then(|r| meta(r, q));
        let r = Patch { x: get("/imgdesc/Left").unwrap_or(0), y: get("/imgdesc/Top").unwrap_or(0), w, h };
        let delay = get("/grctlext/Delay").unwrap_or(0) * 10;
        let delay = if delay < MIN_DELAY { DEFAULT_DELAY } else { delay };
        let disposal = get("/grctlext/Disposal").unwrap_or(0);
        let conv = WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, frame).ok()?;
        let mut px = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut px).ok()?;
        Some((px, r, delay, disposal))
    }
}

/// Decodifica todos los fotogramas y los entrega de a uno a `sink`. Corta si `cancel` se prende.
/// None si no es un GIF o no se pudo leer ni el primero; un fotograma roto a mitad de camino
/// termina la animación ahí.
pub fn decode_gif(path: &str, max_side: u32, budget: usize, cancel: &AtomicBool, mut sink: impl FnMut(Frame)) -> Option<()> {
    let decoder = open(path)?;
    unsafe {
        let n = decoder.GetFrameCount().ok()?;
        let first = decoder.GetFrame(0).ok()?;
        let (mut fw, mut fh) = (0u32, 0u32);
        first.GetSize(&mut fw, &mut fh).ok()?;
        // El lienzo es el de la cabecera; si falta, el del primer fotograma.
        let screen = decoder.GetMetadataQueryReader().ok();
        let dim = |q: &str| screen.as_ref().and_then(|r| meta(r, q)).filter(|&v| v > 0);
        let (cw, ch) = (dim("/logscrdesc/Width").unwrap_or(fw), dim("/logscrdesc/Height").unwrap_or(fh));
        if cw == 0 || ch == 0 {
            return None;
        }
        let (w, h) = out_size(cw, ch, n, max_side, budget);
        let mut canvas = vec![0u8; (cw * ch * 4) as usize];
        let mut saved: Option<Vec<u8>> = None;
        let mut prev: Option<(Patch, u32)> = None;
        for i in 0..n {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let Some((px, r, delay, disposal)) = decoder.GetFrame(i).ok().and_then(|f| patch(&f)) else {
                if i == 0 {
                    return None;
                }
                break;
            };
            // Lo que deja el anterior.
            match prev.take() {
                Some((pr, 2)) => clear(&mut canvas, cw, ch, pr),
                Some((_, 3)) => {
                    if let Some(s) = saved.take() {
                        canvas = s;
                    }
                }
                _ => {}
            }
            if disposal == 3 {
                saved = Some(canvas.clone());
            }
            over(&mut canvas, cw, ch, &px, r);
            prev = Some((r, disposal));
            sink(Frame { w, h, px: shrink(&canvas, cw, ch, w, h), delay_ms: delay });
        }
    }
    Some(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Los datos LZW de una imagen con paleta de 4 colores (código mínimo 2), sin comprimir: un
    /// «borrar» antes de cada píxel, así la tabla nunca crece y los códigos son siempre de 3 bits.
    fn lzw(pixels: &[u8]) -> Vec<u8> {
        let (clear, end) = (4u32, 5u32);
        let mut codes: Vec<u32> = pixels.iter().flat_map(|&p| [clear, p as u32]).collect();
        codes.push(end);
        let (mut bits, mut nb, mut out) = (0u64, 0u32, Vec::new());
        for c in codes {
            bits |= (c as u64) << nb;
            nb += 3;
            while nb >= 8 {
                out.push(bits as u8);
                bits >>= 8;
                nb -= 8;
            }
        }
        if nb > 0 {
            out.push(bits as u8);
        }
        // En bloques de hasta 255, con el 0 que cierra.
        let mut blocks = vec![2u8];
        for ch in out.chunks(255) {
            blocks.push(ch.len() as u8);
            blocks.extend_from_slice(ch);
        }
        blocks.push(0);
        blocks
    }

    /// GIF de 4×4 escrito a mano, paleta rojo, azul, verde, negro. Cada fotograma: (x, y, w, h,
    /// color, demora en centésimas, disposición).
    pub fn tiny_gif(frames: &[(u16, u16, u16, u16, u8, u16, u8)]) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&4u16.to_le_bytes());
        g.extend_from_slice(&4u16.to_le_bytes());
        g.extend_from_slice(&[0x91, 0, 0]);
        g.extend_from_slice(&[255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 0, 0]);
        for &(x, y, w, h, color, delay, disposal) in frames {
            g.extend_from_slice(&[0x21, 0xF9, 4, disposal << 2]);
            g.extend_from_slice(&delay.to_le_bytes());
            g.extend_from_slice(&[0, 0, 0x2C]);
            for v in [x, y, w, h] {
                g.extend_from_slice(&v.to_le_bytes());
            }
            g.push(0);
            g.extend(lzw(&vec![color; w as usize * h as usize]));
        }
        g.push(0x3B);
        g
    }

    fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * f.w + x) * 4) as usize;
        [f.px[i], f.px[i + 1], f.px[i + 2], f.px[i + 3]]
    }

    #[test]
    fn decodes_composed_frames() {
        crate::fsx::shell::com_init();
        // Rojo entero; un parche azul que se borra al terminar (2); un punto verde.
        let gif = tiny_gif(&[(0, 0, 4, 4, 0, 5, 1), (1, 1, 2, 2, 1, 1, 2), (3, 3, 1, 1, 2, 7, 0)]);
        let path = std::env::temp_dir().join(format!("rayo-gif-{}.gif", std::process::id()));
        std::fs::write(&path, gif).unwrap();
        let p = path.to_string_lossy().into_owned();
        assert_eq!(frame_count(&p), Some(3));
        let mut frames = Vec::new();
        decode_gif(&p, MAX_SIDE, BUDGET, &AtomicBool::new(false), |f| frames.push(f)).expect("no se pudo leer el GIF");
        assert_eq!(frames.len(), 3);
        assert_eq!(frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [50, 100, 70], "10 ms pasa a 100");
        assert!(frames.iter().all(|f| (f.w, f.h) == (4, 4)));
        let (red, blue, green) = ([255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]);
        assert_eq!(px(&frames[0], 2, 2), red);
        // El parche va sobre lo anterior, en su lugar.
        assert_eq!((px(&frames[1], 0, 0), px(&frames[1], 1, 1), px(&frames[1], 2, 2), px(&frames[1], 3, 3)), (red, blue, blue, red));
        // Disposición 2: el rectángulo del azul queda transparente; el resto sigue.
        assert_eq!((px(&frames[2], 0, 0), px(&frames[2], 1, 1), px(&frames[2], 2, 2), px(&frames[2], 3, 3)), (red, [0; 4], [0; 4], green));

        // Con poco presupuesto se achica (todos los fotogramas entran), no se corta.
        let mut small = Vec::new();
        decode_gif(&p, MAX_SIDE, 3 * 2 * 2 * 4, &AtomicBool::new(false), |f| small.push(f)).unwrap();
        assert_eq!(small.len(), 3);
        assert_eq!((small[0].w, small[0].h), (2, 2));
        assert_eq!(&small[0].px[..4], &red);
        // Cancelado: no entrega nada.
        let mut none = 0;
        assert!(decode_gif(&p, MAX_SIDE, BUDGET, &AtomicBool::new(true), |_| none += 1).is_none());
        assert_eq!(none, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn restores_previous_canvas() {
        crate::fsx::shell::com_init();
        // Disposición 3: el verde se va y vuelve el azul de antes.
        let gif = tiny_gif(&[(0, 0, 4, 4, 1, 10, 1), (0, 0, 2, 2, 2, 10, 3), (3, 3, 1, 1, 0, 10, 0)]);
        let path = std::env::temp_dir().join(format!("rayo-gif3-{}.gif", std::process::id()));
        std::fs::write(&path, gif).unwrap();
        let mut frames = Vec::new();
        decode_gif(&path.to_string_lossy(), MAX_SIDE, BUDGET, &AtomicBool::new(false), |f| frames.push(f)).unwrap();
        assert_eq!(px(&frames[1], 0, 0), [0, 255, 0, 255]);
        assert_eq!(px(&frames[2], 0, 0), [0, 0, 255, 255]);
        assert_eq!(px(&frames[2], 3, 3), [255, 0, 0, 255]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn not_a_gif() {
        crate::fsx::shell::com_init();
        let path = std::env::temp_dir().join(format!("rayo-nogif-{}.gif", std::process::id()));
        std::fs::write(&path, b"no soy un gif").unwrap();
        assert_eq!(frame_count(&path.to_string_lossy()), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn budget_scale() {
        assert_eq!(out_size(480, 270, 24, MAX_SIDE, BUDGET), (480, 270));
        assert_eq!(out_size(4000, 2000, 1, MAX_SIDE, BUDGET), (1024, 512));
        let (w, h) = out_size(1000, 1000, 500, MAX_SIDE, BUDGET);
        assert!(w as usize * h as usize * 4 * 500 <= BUDGET && w > 200, "{w}×{h}");
    }
}
