//! Imágenes para la vista previa y Quick Look.
//!
//! - Fotos: Windows Imaging Component (WIC), decodificador nativo que respeta la
//!   rotación EXIF y escala con buena calidad.
//! - Todo lo demás (videos, PDF, programas): la miniatura del shell de Windows,
//!   la misma que usa el Explorador (con su caché).

use crate::index::walk::wide;
use crossbeam_channel::{Receiver, unbounded};
use std::collections::HashSet;
use std::sync::Arc;
use windows::Win32::Foundation::{GENERIC_READ, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::*;
use windows::core::Interface;
use windows::core::PCWSTR;

pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    pub premultiplied: bool,
    /// Dimensiones originales de la imagen (si es una foto).
    pub orig: Option<(u32, u32)>,
    /// Cantidad de páginas (PDF).
    pub pages: Option<u32>,
}

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "tif", "tiff", "ico", "heic", "heif", "avif", "jfif", "dng", "jxr"];

/// Pedido de "lo que se asoma" en el ícono de una carpeta (la ruta va después).
pub const PEEK_PREFIX: &str = "\u{1}vistazo:";

pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

fn exif_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(reader) = frame.GetMetadataQueryReader() else { return 1 };
        for q in ["/app1/ifd/{ushort=274}", "/ifd/{ushort=274}"] {
            let w = wide(q);
            let mut pv = PROPVARIANT::default();
            if reader.GetMetadataByName(PCWSTR(w.as_ptr()), &mut pv).is_ok() {
                let v = pv.Anonymous.Anonymous.Anonymous.uiVal;
                if (1..=8).contains(&v) {
                    return v;
                }
            }
        }
    }
    1
}

/// Decodifica una imagen con WIC, escalada para que su lado mayor sea `max_side`.
pub fn wic_load(path: &str, max_side: u32) -> Option<Rgba> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let w = wide(path);
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        decode(&factory, &decoder, max_side)
    }
}

/// Decodifica una imagen que está en memoria (PNG, BMP…).
pub fn wic_decode_bytes(bytes: &[u8], max_side: u32) -> Option<Rgba> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateMemStream(Some(bytes))?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        decode(&factory, &decoder, max_side)
    }
}

fn decode(factory: &IWICImagingFactory, decoder: &IWICBitmapDecoder, max_side: u32) -> Option<Rgba> {
    decode_as(factory, decoder, max_side, true)
}

/// `decode`, girando según el EXIF de la imagen (`exif`) o tal como está guardada.
fn decode_as(factory: &IWICImagingFactory, decoder: &IWICBitmapDecoder, max_side: u32, exif: bool) -> Option<Rgba> {
    unsafe {
        let frame = decoder.GetFrame(0).ok()?;
        let (mut fw, mut fh) = (0u32, 0u32);
        frame.GetSize(&mut fw, &mut fh).ok()?;
        if fw == 0 || fh == 0 {
            return None;
        }
        let orient = if exif { exif_orientation(&frame) } else { 1 };
        let scale = (max_side as f32 / fw.max(fh) as f32).min(1.0);
        let (nw, nh) = (((fw as f32 * scale) as u32).max(1), ((fh as f32 * scale) as u32).max(1));
        // Si el formato sabe decodificar ya reducido (JPEG: 1/2, 1/4, 1/8), se hace eso:
        // una foto de cámara de 24 MP pasa de ~0,2 s a unos milisegundos.
        let mut source: IWICBitmapSource = match (scale < 1.0).then(|| native_scaled(factory, &frame, fw, fh, nw, nh)).flatten() {
            Some(reduced) => reduced,
            None => frame.into(),
        };
        if scale < 1.0 {
            let (mut sw, mut sh) = (0u32, 0u32);
            source.GetSize(&mut sw, &mut sh).ok()?;
            if (sw, sh) != (nw, nh) {
                let scaler = factory.CreateBitmapScaler().ok()?;
                scaler.Initialize(&source, nw, nh, WICBitmapInterpolationModeHighQualityCubic).ok()?;
                source = scaler.into();
            }
        }
        // El giro EXIF se hace después, sobre los píxeles ya decodificados. El de WIC
        // (`IWICBitmapFlipRotator`) pide la imagen por columnas al girar 90°: con el decodificador
        // de JPEG detrás, vuelve a decodificar una y otra vez (una foto vertical de 40 MP a
        // 1536 px tardaba 610 ms en vez de 200, y entera, más de un minuto).
        let converted = WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &source).ok()?;
        let (mut cw, mut ch) = (0u32, 0u32);
        converted.GetSize(&mut cw, &mut ch).ok()?;
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        converted.CopyPixels(std::ptr::null(), cw * 4, &mut px).ok()?;
        let (px, cw, ch) = orient_pixels(px, cw, ch, orient);
        let orig = if matches!(orient, 5..=8) { (fh, fw) } else { (fw, fh) };
        Some(Rgba { w: cw, h: ch, px, premultiplied: false, orig: Some(orig), pages: None })
    }
}

/// La miniatura que la cámara guardó dentro de la foto (EXIF, típicamente 160×120): está
/// en los primeros kilobytes del archivo, así que sale en ~10 ms aunque la foto pese
/// 26 MB y esté en un disco mecánico. Se le quitan las franjas negras (fotos 3:2 en una
/// miniatura 4:3) y se gira según la orientación de la foto.
pub fn embedded_thumb(path: &str) -> Option<Rgba> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let w = wide(path);
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut fw, mut fh) = (0u32, 0u32);
        frame.GetSize(&mut fw, &mut fh).ok()?;
        let thumb = frame.GetThumbnail().ok()?;
        let converted = WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &thumb).ok()?;
        let (mut tw, mut th) = (0u32, 0u32);
        converted.GetSize(&mut tw, &mut th).ok()?;
        if tw < 32 || th < 32 {
            return None;
        }
        let mut px = vec![0u8; (tw * th * 4) as usize];
        converted.CopyPixels(std::ptr::null(), tw * 4, &mut px).ok()?;
        let (x0, y0, x1, y1) = content_box(&px, tw, th);
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut cropped = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            let row = ((y * tw + x0) * 4) as usize;
            cropped.extend_from_slice(&px[row..row + (cw * 4) as usize]);
        }
        let orient = exif_orientation(&frame);
        let (px, w, h) = orient_pixels(cropped, cw, ch, orient);
        let orig = if matches!(orient, 5..=8) { (fh, fw) } else { (fw, fh) };
        Some(Rgba { w, h, px, premultiplied: false, orig: Some(orig), pages: None })
    }
}

/// Recuadro sin las franjas negras de los bordes (hasta un cuarto de cada lado).
fn content_box(px: &[u8], w: u32, h: u32) -> (u32, u32, u32, u32) {
    let dark = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        px[i].max(px[i + 1]).max(px[i + 2]) < 24
    };
    let row_dark = |y: u32| (0..w).all(|x| dark(x, y));
    let col_dark = |x: u32, y0: u32, y1: u32| (y0..y1).all(|y| dark(x, y));
    let (mut y0, mut y1) = (0, h);
    while y0 < h / 4 && row_dark(y0) {
        y0 += 1;
    }
    while y1 > h - h / 4 && row_dark(y1 - 1) {
        y1 -= 1;
    }
    let (mut x0, mut x1) = (0, w);
    while x0 < w / 4 && col_dark(x0, y0, y1) {
        x0 += 1;
    }
    while x1 > w - w / 4 && col_dark(x1 - 1, y0, y1) {
        x1 -= 1;
    }
    (x0, y0, x1, y1)
}

/// Gira / refleja píxeles RGBA según la orientación EXIF (1–8).
fn orient_pixels(px: Vec<u8>, w: u32, h: u32, orient: u16) -> (Vec<u8>, u32, u32) {
    if !(2..=8).contains(&orient) {
        return (px, w, h);
    }
    let swap = orient >= 5;
    let (nw, nh) = if swap { (h, w) } else { (w, h) };
    // A dónde va el píxel (x, y) de la imagen guardada: `start + x·sx + y·sy` (en píxeles). Sin
    // decidir por píxel: una foto entera de 40 MP se gira en ~100 ms.
    let (wi, hi, nwi) = (w as isize, h as isize, nw as isize);
    let (start, sx, sy) = match orient {
        2 => (wi - 1, -1, nwi),
        3 => ((hi - 1) * nwi + wi - 1, -1, -nwi),
        4 => ((hi - 1) * nwi, 1, -nwi),
        5 => (0, nwi, 1),
        6 => (hi - 1, nwi, -1),
        7 => ((wi - 1) * nwi + hi - 1, -nwi, -1),
        _ => ((wi - 1) * nwi, -nwi, 1),
    };
    let src: Vec<u32> = px.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut dst = vec![0u32; src.len()];
    for (y, row) in src.chunks_exact(w as usize).enumerate() {
        let mut d = start + y as isize * sy;
        for &p in row {
            dst[d as usize] = p;
            d += sx;
        }
    }
    let out: Vec<u8> = dst.into_iter().flat_map(u32::to_ne_bytes).collect();
    (out, nw, nh)
}

/// Decodifica la imagen ya reducida, si el formato lo permite, al tamaño nativo más chico
/// que no sea menor que el pedido (así no se pierde calidad al terminar de escalar).
fn native_scaled(factory: &IWICImagingFactory, frame: &IWICBitmapFrameDecode, fw: u32, fh: u32, nw: u32, nh: u32) -> Option<IWICBitmapSource> {
    unsafe {
        let t: IWICBitmapSourceTransform = frame.cast().ok()?;
        let (mut w, mut h) = (nw, nh);
        t.GetClosestSize(&mut w, &mut h).ok()?;
        // Algunos decodificadores devuelven algo más chico que lo pedido: subir de a dobles.
        while (w < nw || h < nh) && w < fw {
            let (mut w2, mut h2) = ((w * 2).min(fw), (h * 2).min(fh));
            t.GetClosestSize(&mut w2, &mut h2).ok()?;
            if (w2, h2) == (w, h) {
                break;
            }
            (w, h) = (w2, h2);
        }
        if w >= fw || w < nw || h < nh {
            return None;
        }
        let mut fmt = GUID_WICPixelFormat24bppBGR;
        t.GetClosestPixelFormat(&mut fmt).ok()?;
        let bpp = if fmt == GUID_WICPixelFormat24bppBGR {
            24
        } else if fmt == GUID_WICPixelFormat32bppBGRA || fmt == GUID_WICPixelFormat32bppBGR || fmt == GUID_WICPixelFormat32bppRGBA {
            32
        } else if fmt == GUID_WICPixelFormat8bppGray {
            8
        } else {
            return None;
        };
        let stride = (w * bpp / 8 + 3) & !3;
        let mut buf = vec![0u8; (stride * h) as usize];
        t.CopyPixels(std::ptr::null(), w, h, &fmt, WICBitmapTransformRotate0, stride, &mut buf).ok()?;
        let bmp = factory.CreateBitmapFromMemory(w, h, &fmt, stride, &buf).ok()?;
        Some(bmp.into())
    }
}

/// Miniatura o ícono grande del shell de Windows.
pub fn shell_image(path: &str, size: i32, icon_only: bool) -> Option<Rgba> {
    shell_image_with(path, size, icon_only, false, false)
}

/// La miniatura que Windows ya tenga guardada en su caché (no genera nada: si no está, None).
pub fn shell_cached(path: &str, size: i32) -> Option<Rgba> {
    shell_image_with(path, size, false, true, false)
}

/// Solo una miniatura de verdad: si Windows no puede darla, None (no el ícono del tipo). Al
/// arrancar, unos segundos, Windows contesta con el ícono en vez de la foto; aceptarlo dejaría
/// ese ícono como miniatura para siempre.
fn shell_thumb(path: &str, size: i32) -> Option<Rgba> {
    shell_image_with(path, size, false, false, true)
}

fn shell_image_with(path: &str, size: i32, icon_only: bool, cache_only: bool, thumb_only: bool) -> Option<Rgba> {
    // Archivos solo en la nube: la miniatura que Windows ya tenga guardada, o el ícono (nunca descargar).
    let cloud = !icon_only && super::is_online_only(path);
    unsafe {
        let w = wide(path);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let flags = if icon_only {
            SIIGBF_ICONONLY
        } else if cache_only {
            // Sin THUMBNAILONLY, si la miniatura no está en la caché Windows puede dar el ícono
            // del tipo (medido: 20 de 123 fotos de una carpeta, al abrirla).
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK | SIIGBF_INCACHEONLY | SIIGBF_THUMBNAILONLY
        } else if cloud {
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK | SIIGBF_INCACHEONLY
        } else if thumb_only {
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK | SIIGBF_THUMBNAILONLY
        } else {
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK
        };
        let hbmp = match factory.GetImage(SIZE { cx: size, cy: size }, flags) {
            Ok(b) => b,
            Err(_) if cloud && !cache_only => factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY).ok()?,
            Err(_) => return None,
        };
        let img = bitmap_pixels(hbmp);
        let _ = DeleteObject(hbmp.into());
        img
    }
}

/// Píxeles RGBA de un mapa de bits de Windows (no lo libera).
pub fn bitmap_pixels(hbmp: HBITMAP) -> Option<Rgba> {
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(hbmp.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (bw, bh) = (bm.bmWidth, bm.bmHeight.abs());
        if bw <= 0 || bh <= 0 {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: bw,
                biHeight: -bh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (bw * bh * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, hbmp, 0, bh as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if lines == 0 {
            return None;
        }
        let mut any_alpha = false;
        // Las miniaturas vienen premultiplicadas, pero los íconos grandes que da Windows no (p. ej. el
        // de Arc): un color más intenso que su opacidad es imposible premultiplicado, y delata el caso.
        // Tratarlos mal deja los bordes semitransparentes como recortados.
        let mut straight = false;
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            any_alpha |= p[3] != 0;
            straight |= p[0].max(p[1]).max(p[2]) > p[3];
        }
        if !any_alpha {
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        Some(Rgba { w: bw as u32, h: bh as u32, px, premultiplied: any_alpha && !straight, orig: None, pages: None })
    }
}

// ---------------------------------------------------------------- hilo de carga

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ThumbKey {
    pub path: String,
    pub size: u32,
}

pub struct ThumbResult {
    pub key: ThumbKey,
    /// Medidas, páginas y forma (sus píxeles ya pasaron a `bands`).
    pub img: Option<Rgba>,
    /// La imagen lista para subir a la GPU, en franjas (una sola si es chica). Se arma en el hilo
    /// de carga: convertir una foto de varios megapíxeles en el hilo de la interfaz se comía
    /// varios cuadros seguidos, y subirla entera de una vez (una foto de 4096 son 48 MB) también.
    pub bands: Vec<Band>,
    /// (opaca, ícono en un lienzo vacío): ver `Rgba::shape`.
    pub shape: (bool, bool),
    /// Color medio (ver `Rgba::mean_color`).
    pub color: [u8; 3],
    /// Pedido solo de la caché en disco y no estaba: no es un fallo (se pedirá de verdad después).
    pub cheap_miss: bool,
}

/// Una franja horizontal de una imagen grande: sus filas (`rows`) más `GUTTER` filas de las
/// vecinas arriba y abajo, para que al dibujarla filtrada la unión no se note. `top` es la fila
/// de la imagen con la que empieza la textura.
pub struct Band {
    pub img: egui::ColorImage,
    pub rows: std::ops::Range<usize>,
    pub top: usize,
}

/// Bytes por franja: subir 4 MB a la GPU cuesta ~0,5 ms (una foto entera, 6 ms).
pub const BAND_BYTES: usize = 4 << 20;
/// Una imagen de hasta esto va en una sola textura.
const SINGLE_BYTES: usize = 6 << 20;
/// Filas de más en cada lado de una franja.
pub const GUTTER: usize = 2;

impl Rgba {
    /// Color medio (RGB, sin premultiplicar), de una muestra de ~32×32: la «huella» con la que
    /// se pinta el lugar de la miniatura si su textura se desalojó.
    pub fn mean_color(&self) -> [u8; 3] {
        let (w, h) = (self.w as usize, self.h as usize);
        if w == 0 || h == 0 || self.px.len() < w * h * 4 {
            return [128; 3];
        }
        let step = (w.max(h) / 32).max(1);
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for y in (step / 2..h).step_by(step) {
            for x in (step / 2..w).step_by(step) {
                let p = &self.px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                let k = if self.premultiplied && p[3] > 0 { 255.0 / p[3] as f32 } else { 1.0 };
                for c in 0..3 {
                    sum[c] += (p[c] as f32 * k).min(255.0) as u64;
                }
                n += 1;
            }
        }
        let n = n.max(1);
        [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
    }

    /// Cómo es: (opaca, ícono en un lienzo vacío). Mira una muestra de ~64×64 píxeles, así
    /// cuesta lo mismo una miniatura que una página de 4096. El ícono que da Windows en lugar de
    /// la miniatura es uno de 48 centrado en 256, a veces con un marco semitransparente en el
    /// borde: cuenta solo lo bien opaco (α ≥ 128).
    pub fn shape(&self) -> (bool, bool) {
        let (w, h) = (self.w as usize, self.h as usize);
        if w == 0 || h == 0 || self.px.len() < w * h * 4 {
            return (false, false);
        }
        let step = (w.max(h) / 64).max(1);
        let mut opaque = true;
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for y in (step / 2..h).step_by(step) {
            for x in (step / 2..w).step_by(step) {
                let a = self.px[(y * w + x) * 4 + 3];
                opaque &= a == 255;
                if a >= 128 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        // El borde también cuenta para «opaca» (el marco de Windows es semitransparente).
        opaque &= self.px[3] == 255 && self.px[(w * h - 1) * 4 + 3] == 255;
        let icon = !opaque && w.max(h) >= 96 && x1 >= x0 && (x1 - x0 + step) * 2 <= w && (y1 - y0 + step) * 2 <= h;
        (opaque, icon)
    }

    /// La imagen en franjas de ~`BAND_BYTES` (o entera, si es chica).
    pub fn to_bands(&self) -> Vec<Band> {
        let (w, h) = (self.w as usize, self.h as usize);
        let color = |a: usize, b: usize| {
            let px = &self.px[a * w * 4..b * w * 4];
            let size = [w, b - a];
            if self.premultiplied { egui::ColorImage::from_rgba_premultiplied(size, px) } else { egui::ColorImage::from_rgba_unmultiplied(size, px) }
        };
        if w == 0 || h == 0 || self.px.len() < w * h * 4 || w * h * 4 <= SINGLE_BYTES {
            let size = [w, h];
            let img = if self.px.len() >= w * h * 4 { color(0, h) } else { egui::ColorImage::filled(size, egui::Color32::TRANSPARENT) };
            return vec![Band { img, rows: 0..h, top: 0 }];
        }
        let step = (BAND_BYTES / (w * 4)).max(64);
        (0..h)
            .step_by(step)
            .map(|a| {
                let b = (a + step).min(h);
                let (top, bottom) = (a.saturating_sub(GUTTER), (b + GUTTER).min(h));
                Band { img: color(top, bottom), rows: a..b, top }
            })
            .collect()
    }
}

/// Lo que se quiere cargar: en qué orden se pidió, en qué cuadro se pidió por última vez
/// (lo que dejó de pedirse salió de la pantalla) y si es urgente (vista previa).
struct Want {
    first: u64,
    frame: u64,
    urgent: bool,
    /// Precalentar (la foto que probablemente se va a ver): después de todo lo demás.
    warm: bool,
    /// Solo de la caché en disco (desplazándose rápido: nada de leer la foto).
    cheap: bool,
    /// Cuándo se pidió por primera vez (para medir la espera en la cola).
    since: std::time::Instant,
}

#[derive(Default)]
struct Queue {
    wanted: std::collections::HashMap<ThumbKey, Want>,
    busy: HashSet<ThumbKey>,
    seq: u64,
    /// Versión de cada archivo según el listado (`thumbcache::stamp_of`), para la caché en disco.
    stamps: std::collections::HashMap<String, u64>,
}

/// Carga imágenes en segundo plano.
///
/// - Nada se descarta: lo visible se carga en el orden en que se pidió (de arriba hacia
///   abajo) y lo que deja de pedirse (salió de la pantalla) se olvida.
/// - La vista previa y la vista rápida son urgentes: tienen un hilo solo para ellas, así
///   nunca esperan detrás de una galería llena de fotos.
pub struct ThumbLoader {
    queue: Arc<(parking_lot::Mutex<Queue>, parking_lot::Condvar)>,
    frame: Arc<std::sync::atomic::AtomicU64>,
    pub rx: Receiver<ThumbResult>,
}

/// Hilos para miniaturas (además del de la vista previa): leer una foto de cámara cuesta
/// ~90 ms aunque se decodifique reducida, así que conviene repartir.
const WORKERS: usize = 3;
/// Cuadros sin pedirse tras los que se olvida un pedido urgente.
const URGENT_FRAMES: u64 = 60;

impl ThumbLoader {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let (res_tx, rx) = unbounded::<ThumbResult>();
        let queue: Arc<(parking_lot::Mutex<Queue>, parking_lot::Condvar)> = Default::default();
        let frame = Arc::new(std::sync::atomic::AtomicU64::new(0));
        for i in 0..=WORKERS {
            // El hilo 0 atiende solo lo urgente.
            let urgent_only = i == 0;
            let (queue, frame, res_tx, wake) = (queue.clone(), frame.clone(), res_tx.clone(), wake.clone());
            std::thread::Builder::new()
                .name(if urgent_only { "vista-previa".into() } else { format!("miniaturas-{i}") })
                .spawn(move || {
                    super::shell::com_init();
                    loop {
                        let (waited, mut cheap, mut stamp);
                        let key = {
                            let (lock, cv) = &*queue;
                            let mut q = lock.lock();
                            loop {
                                // Olvidar lo que hace unos cuadros que no se pide (lo urgente, tras
                                // un segundo: las páginas de un PDF que se pasaron de largo).
                                let now = frame.load(std::sync::atomic::Ordering::Relaxed);
                                q.wanted.retain(|_, w| w.frame + if w.urgent { URGENT_FRAMES } else { 2 } >= now);
                                // Primero lo urgente, y lo que se sigue pidiendo antes que lo que se pidió hace rato.
                                let pick = q
                                    .wanted
                                    .iter()
                                    .filter(|(_, w)| !urgent_only || w.urgent)
                                    .min_by_key(|(_, w)| (!w.urgent, w.warm, std::cmp::Reverse(w.frame), w.first))
                                    .map(|(k, _)| k.clone());
                                if let Some(k) = pick {
                                    let w = q.wanted.remove(&k);
                                    q.busy.insert(k.clone());
                                    waited = w.as_ref().map_or(std::time::Duration::ZERO, |w| w.since.elapsed());
                                    cheap = w.is_some_and(|w| w.cheap);
                                    stamp = q.stamps.get(&k.path).copied().unwrap_or(0);
                                    break k;
                                }
                                cv.wait_for(&mut q, std::time::Duration::from_millis(500));
                            }
                        };
                        let started = std::time::Instant::now();
                        // Lo que ya está en la caché en disco: sin tocar el disco de la foto.
                        let small = key.size <= super::thumbcache::MAX_SIDE && !key.path.starts_with(ICON_PREFIX) && !key.path.starts_with(PEEK_PREFIX);
                        if !small {
                            stamp = 0;
                        }
                        let cached = super::thumbcache::get(&key.path, key.size, stamp);
                        let from_ssd = cached.is_some();
                        if from_ssd {
                            method("ssd");
                        }
                        let cheap_miss = cheap && !from_ssd;
                        cheap &= !from_ssd;
                        let mut img = if from_ssd || cheap { cached } else { load(&key) };
                        if !from_ssd && !cheap_miss && stamp != 0 && let Some(i) = img.as_ref().filter(|i| !i.shape().1) {
                            super::thumbcache::put(&key.path, key.size, stamp, i);
                        }
                        if trace_on() {
                            trace(&key, waited, started.elapsed(), img.as_ref());
                        }
                        let shape = img.as_ref().map_or((false, false), |i| i.shape());
                        let color = img.as_ref().map_or([128; 3], |i| i.mean_color());
                        let bands = img.as_mut().map_or_else(Vec::new, |i| {
                            let b = i.to_bands();
                            i.px = Vec::new();
                            b
                        });
                        // Sigue «en curso» hasta que la interfaz lo recibe (`received`): si no, un
                        // pedido de ese mismo cuadro lo volvía a cargar.
                        let _ = res_tx.send(ThumbResult { key, img, bands, shape, color, cheap_miss });
                        wake();
                    }
                })
                .ok();
        }
        Arc::new(ThumbLoader { queue, frame, rx })
    }

    /// Pedir una imagen (se llama en cada cuadro mientras se necesite).
    pub fn request(&self, key: ThumbKey, urgent: bool) {
        self.request_with(key, urgent, false, false, 0);
    }

    /// Pedir para precalentar: se atiende después de todo lo que se ve.
    pub fn request_warm(&self, key: ThumbKey, stamp: u64) {
        self.request_with(key, false, true, false, stamp);
    }

    /// Pedir una miniatura con la versión del archivo según el listado (`stamp`, 0 = no se
    /// sabe): así se busca primero en la caché en disco. `cheap`: solo de ahí.
    pub fn request_thumb(&self, key: ThumbKey, stamp: u64, cheap: bool) {
        self.request_with(key, false, false, cheap, stamp);
    }

    fn request_with(&self, key: ThumbKey, urgent: bool, warm: bool, cheap: bool, stamp: u64) {
        let (lock, cv) = &*self.queue;
        let mut q = lock.lock();
        if stamp != 0 && q.stamps.get(&key.path) != Some(&stamp) {
            if q.stamps.len() > 50_000 {
                q.stamps.clear();
            }
            q.stamps.insert(key.path.clone(), stamp);
        }
        if q.busy.contains(&key) {
            return;
        }
        let frame = self.frame.load(std::sync::atomic::Ordering::Relaxed);
        if let Some(w) = q.wanted.get_mut(&key) {
            w.frame = frame;
            w.urgent |= urgent;
            w.warm &= warm;
            w.cheap &= cheap;
            if !urgent {
                return;
            }
        } else {
            let first = q.seq;
            q.seq += 1;
            q.wanted.insert(key, Want { first, frame, urgent, warm, cheap, since: std::time::Instant::now() });
        }
        cv.notify_all();
    }

    /// La interfaz recibió el resultado de `key`: ya se puede volver a pedir.
    pub fn received(&self, key: &ThumbKey) {
        self.queue.0.lock().busy.remove(key);
    }

    /// Un cuadro nuevo de la interfaz (para saber qué dejó de pedirse).
    pub fn next_frame(&self) {
        self.frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- medición (RAYO_PERF)

thread_local! {
    /// Con qué se resolvió la última carga de este hilo (para `thumbs.csv`).
    static METHOD: std::cell::Cell<&'static str> = const { std::cell::Cell::new("") };
}

fn method(m: &'static str) {
    METHOD.with(|c| c.set(m));
}

fn trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RAYO_PERF").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Con `RAYO_PERF`: una línea por imagen cargada en `thumbs.csv` (junto a `perf.csv`): cuándo
/// terminó, cuánto esperó en la cola, cuánto tardó, con qué método y qué salió.
fn trace(key: &ThumbKey, waited: std::time::Duration, took: std::time::Duration, img: Option<&Rgba>) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<parking_lot::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let f = FILE.get_or_init(|| {
        let dir = crate::instance::custom_data_dir().or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Rayo")))?;
        let path = dir.join("thumbs.csv");
        let fresh = !path.exists();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        if fresh {
            let _ = writeln!(f, "ms_unix,hilo,espera_ms,carga_ms,lado,metodo,resultado,ruta");
        }
        Some(parking_lot::Mutex::new(f))
    });
    let Some(f) = f else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    let res = match img {
        Some(i) if i.shape().1 => format!("icono {}x{}", i.w, i.h),
        Some(i) => format!("{}x{}", i.w, i.h),
        None => "nada".into(),
    };
    let m = METHOD.with(|c| c.replace(""));
    let hilo = std::thread::current().name().unwrap_or("").to_string();
    let _ = writeln!(f.lock(), "{now},{hilo},{:.1},{:.1},{},{m},{res},\"{}\"", waited.as_secs_f64() * 1000.0, took.as_secs_f64() * 1000.0, key.size, key.path);
}

/// Marca de un pedido de ícono (no de miniatura) en la ruta de un `ThumbKey`.
pub const ICON_PREFIX: &str = "\u{1}icono:";

/// Carga una imagen: página de PDF, foto o miniatura del shell (o el ícono, con `ICON_PREFIX`).
fn load(key: &ThumbKey) -> Option<Rgba> {
    if let Some(path) = key.path.strip_prefix(ICON_PREFIX) {
        // Grandes (cuadrícula): Windows elige el mejor dibujo del ícono y lo entrega del tamaño
        // pedido. Las listas de íconos del sistema solo tienen 16, 32, 48 y 256, y el de 256 de
        // muchos programas es uno chico centrado en un lienzo vacío.
        if key.size > 48 {
            return match shell_image(path, key.size as i32, true) {
                // Ícono que solo existe chico (p. ej. el de un juego de Steam, de 32): Windows lo agranda
                // y le pone un marco gris alrededor. Mejor el de 48 sin marco, que Rayo agranda suave.
                Some(img) if framed(&img) => system_icon(path, 48).or(Some(img)),
                Some(img) => Some(img),
                None => system_icon(path, key.size),
            };
        }
        let small = system_icon(path, key.size);
        // Si el chico es solo el grande achicado por Windows sin filtrar (escalera), Rayo lo achica bien.
        if let Some(better) = small.as_ref().and_then(|s| reshrunk(path, s, key.size)) {
            return Some(better);
        }
        return small.or_else(|| shell_image(path, key.size as i32, true));
    }
    super::dev_slow();
    if let Some(dir) = key.path.strip_prefix(PEEK_PREFIX) {
        return folder_peek(dir, key.size);
    }
    // "C:\\doc.pdf|3" = página 3 de un PDF.
    let (path, page) = match key.path.rsplit_once(super::pdf::PAGE_SEP) {
        Some((p, n)) => (p.to_string(), n.parse().unwrap_or(0)),
        None => (key.path.clone(), 0),
    };
    let ext = super::ext_of(&path);
    if ext == "pdf" {
        return super::pdf::render(&path, page, key.size)
            .map(|(mut img, pages)| {
                img.pages = Some(pages);
                img
            })
            .or_else(|| shell_image(&path, key.size as i32, false));
    }
    if is_image_ext(&ext) && !super::is_online_only(&path) {
        return photo(&path, &ext, key.size);
    }
    shell_image(&path, key.size as i32, false)
}

/// Lado de pedido especial: «lo que haya enseguida» de una foto (la vista previa que trae
/// adentro, o la miniatura que Windows ya tenga), del tamaño que tenga. Para el primer cuadro de
/// la Vista rápida, mientras se decodifica la foto.
pub const QUICK: u32 = 0;

/// Marca de un pedido «encajada en un área» (ver `fit_key`).
const FIT_BIT: u32 = 1 << 31;
/// Lado mínimo de una foto encajada: más que el tope de las miniaturas del shell (512).
pub const FIT_MIN: u32 = 768;

/// Pedido de una foto encajada en un área de `w`×`h` píxeles (Vista rápida): el lado lo decide
/// el hilo de carga cuando ya sabe las medidas y la orientación del original (que salen de la
/// misma lectura). Así se puede pedir antes de saberlas, y precalentar las vecinas, sin
/// decodificar dos veces una foto vertical. El área va en pasos de 32 px.
pub fn fit_key(w: u32, h: u32) -> u32 {
    FIT_BIT | (w.div_ceil(32).min(0x7FFF) << 15) | h.div_ceil(32).min(0x7FFF)
}

/// El área de un `fit_key`, si lo es.
pub fn fit_area(size: u32) -> Option<(u32, u32)> {
    (size & FIT_BIT != 0).then(|| (((size >> 15) & 0x7FFF) * 32, (size & 0x7FFF) * 32))
}

/// El lado (en pasos de 256) con que se pide una foto de `orig` (como se ve) encajada en `area`
/// píxeles: nunca más que el original, nunca menos que `FIT_MIN`.
pub fn fit_side(orig: (u32, u32), area: (u32, u32)) -> u32 {
    let (ow, oh) = (orig.0.max(1) as f32, orig.1.max(1) as f32);
    let fit = (area.0 as f32 / ow).min(area.1 as f32 / oh).min(4.0);
    let side = (ow.max(oh) * fit).max(1.0) as u32;
    side.clamp(FIT_MIN, 4096).next_multiple_of(256).min(orig.0.max(orig.1).next_multiple_of(256)).max(FIT_MIN)
}

/// `wic_load` encajada en un área (ver `fit_key`).
fn wic_load_fit(path: &str, area: (u32, u32)) -> Option<Rgba> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let w = wide(path);
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut fw, mut fh) = (0u32, 0u32);
        frame.GetSize(&mut fw, &mut fh).ok()?;
        let shown = if matches!(exif_orientation(&frame), 5..=8) { (fh, fw) } else { (fw, fh) };
        decode(&factory, &decoder, fit_side(shown, area))
    }
}

/// ¿Sirve como miniatura de una foto de lado `size`? No el ícono del tipo (con
/// `SIIGBF_INCACHEONLY` Windows a veces lo da en vez de fallar: era la miniatura que quedaba
/// para siempre como ícono) ni una mucho más chica que lo pedido (la caché puede tener una de 96).
fn real_thumb(img: &Rgba, size: u32) -> bool {
    !img.shape().1 && img.w.max(img.h) as f32 >= size as f32 * 0.75
}

/// Una foto, de lo más rápido a lo más lento:
///
/// 1. JPEG: la vista previa que guardó la cámara (un salto del cabezal y 256 KB: ~8 ms en frío
///    en un disco mecánico, ~1 ms en caché), si alcanza para el tamaño pedido.
/// 2. La caché de miniaturas de Windows (la del Explorador), si tiene una de verdad.
/// 3. La foto entera: Windows (que la guarda en su caché) o WIC, decodificada ya reducida.
///    Leer 26 MB de un disco mecánico cuesta ~120 ms, y de a varias a la vez el cabezal salta
///    y rinde la mitad: en esos discos se lee de a una (`heavy`).
fn photo(path: &str, ext: &str, size: u32) -> Option<Rgba> {
    // Miniatura (cuadrícula): si Windows ya la tiene en su caché, es lo más rápido (~1 ms, sin
    // leer la foto). Llega de 256: se achica aquí al tamaño exacto (en la GPU, sin mipmaps,
    // quedaría arenosa).
    // (En un disco mecánico, también de a una: preguntarle a la caché de Windows lee el registro
    // del archivo en el disco, y entreverado con las lecturas de los otros hilos hace saltar el
    // cabezal.)
    if size != QUICK
        && size <= 256
        && let Some(img) = super::disks::one_at_a_time(path, || shell_cached(path, size as i32)).filter(|i| real_thumb(i, size))
    {
        method("shell-cache");
        return Some(fit_within(img, size));
    }
    if matches!(ext, "jpg" | "jpeg" | "jfif")
        && fit_area(size).is_none()
        && let Some(img) = jpeg_preview(path, size)
    {
        method("incrustada");
        return Some(img);
    }
    let cache_side = if size == QUICK { 256 } else { size };
    if size <= 512
        && let Some(img) = super::disks::one_at_a_time(path, || shell_cached(path, cache_side as i32)).filter(|i| real_thumb(i, cache_side))
    {
        method("shell-cache");
        return Some(img);
    }
    if size == QUICK {
        return None;
    }
    if let Some(area) = fit_area(size) {
        return heavy(path, || wic_load_fit(path, area).inspect(|_| method("wic-encajada")));
    }
    heavy(path, || {
        let shell = || shell_thumb(path, size as i32).filter(|i| real_thumb(i, size.min(256))).inspect(|_| method("shell"));
        let wic = || wic_load(path, size).inspect(|_| method("wic"));
        // Hasta 512, primero Windows (la deja en su caché para la próxima vez); más grande, WIC.
        if size <= 512 { shell().or_else(wic) } else { wic().or_else(shell) }
    })
}

/// `img` achicada (calidad alta) para que su lado mayor sea `side`, si es más grande.
fn fit_within(img: Rgba, side: u32) -> Rgba {
    let big = img.w.max(img.h);
    if big <= side || img.w == 0 || img.h == 0 {
        return img;
    }
    let k = side as f32 / big as f32;
    let (nw, nh) = (((img.w as f32 * k).round() as u32).max(1), ((img.h as f32 * k).round() as u32).max(1));
    let scaled = (|| unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let fmt = if img.premultiplied { GUID_WICPixelFormat32bppPRGBA } else { GUID_WICPixelFormat32bppRGBA };
        let bmp = factory.CreateBitmapFromMemory(img.w, img.h, &fmt, img.w * 4, &img.px).ok()?;
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler.Initialize(&bmp, nw, nh, WICBitmapInterpolationModeHighQualityCubic).ok()?;
        let mut px = vec![0u8; (nw * nh * 4) as usize];
        scaler.CopyPixels(std::ptr::null(), nw * 4, &mut px).ok()?;
        Some(px)
    })();
    match scaled {
        Some(px) => Rgba { w: nw, h: nh, px, ..img },
        None => img,
    }
}

/// Lo que lee una foto entera, de a uno por disco si el disco tiene cabezal (ver
/// `disks::one_at_a_time`).
fn heavy<T>(path: &str, f: impl FnOnce() -> T) -> T {
    super::disks::one_at_a_time(path, f)
}

/// La vista previa que trae un JPEG (ver `jpeg`), lista: sin franjas negras, girada según EXIF y
/// del tamaño pedido. Elige una que tenga el doble del lado pedido si la hay (achicada se ve más
/// nítida que la justa), si no la que alcance; None si ninguna llega a ~3/4 de lo pedido.
/// Con `QUICK`, la más grande, entera.
pub fn jpeg_preview(path: &str, side: u32) -> Option<Rgba> {
    // En un disco mecánico, de a una lectura (también las cortas): tres hilos leyendo el
    // principio de fotos distintas a la vez hacían saltar el cabezal (~100 ms cada una en vez
    // de ~8).
    let (p, bytes) = super::disks::one_at_a_time(path, || {
        let p = super::jpeg::peek(path)?;
        let pv = if side == QUICK {
            p.previews.last()?
        } else {
            p.previews.iter().find(|v| v.w.max(v.h) >= side * 2).or_else(|| p.best_for(side))?
        };
        if side != QUICK && (pv.w.max(pv.h) as f32) < side as f32 * 0.75 {
            return None;
        }
        let bytes = super::jpeg::bytes_of(path, pv)?;
        Some((p, bytes))
    })?;
    let img = unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateMemStream(Some(&bytes))?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        // La vista previa no trae su propia orientación: se usa la de la foto.
        decode_as(&factory, &decoder, if side == QUICK { u32::MAX } else { side }, false)?
    };
    let (px, w, h) = letterbox_crop(img.px, img.w, img.h, p.size);
    let (px, w, h) = orient_pixels(px, w, h, p.orient);
    Some(Rgba { w, h, px, premultiplied: false, orig: p.shown_size(), pages: None })
}

/// Quita las franjas negras de una vista previa 4:3 de una foto 3:2 (o al revés): recorta a la
/// proporción del original, centrado, si lo que sobra es oscuro (si no, la vista previa ya venía
/// recortada y se deja como está).
fn letterbox_crop(px: Vec<u8>, w: u32, h: u32, main: Option<(u32, u32)>) -> (Vec<u8>, u32, u32) {
    let Some((mw, mh)) = main.filter(|&(a, b)| a > 0 && b > 0) else { return (px, w, h) };
    let (aspect, own) = (mw as f32 / mh as f32, w as f32 / h as f32);
    if (aspect / own - 1.0).abs() < 0.02 || w < 8 || h < 8 {
        return (px, w, h);
    }
    // El recuadro de la foto dentro de la vista previa, hacia adentro (sin la fila de la unión).
    let (x0, y0, x1, y1) = if aspect > own {
        let y0 = ((h as f32 - w as f32 / aspect) / 2.0).ceil() as u32;
        (0, y0, w, (h - y0).max(y0 + 1))
    } else {
        let x0 = ((w as f32 - h as f32 * aspect) / 2.0).ceil() as u32;
        (x0, 0, (w - x0).max(x0 + 1), h)
    };
    // ¿Lo de afuera es negro? Se mira el promedio de lo que se va a quitar.
    let (mut sum, mut n) = (0u64, 0u64);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            if y < y0 || y >= y1 || x < x0 || x >= x1 {
                let i = ((y * w + x) * 4) as usize;
                sum += px[i].max(px[i + 1]).max(px[i + 2]) as u64;
                n += 1;
            }
        }
    }
    if n == 0 || sum / n > 28 {
        return (px, w, h);
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y1 {
        let row = ((y * w + x0) * 4) as usize;
        out.extend_from_slice(&px[row..row + (cw * 4) as usize]);
    }
    (out, cw, ch)
}

/// Ícono chico mejor que el de Windows, cuando el ícono no trae dibujo chico (p. ej. accesos
/// directos con un solo PNG de 256, como el de Arc): Windows lo fabrica achicando el grande sin
/// filtrar y queda en escalera. Entonces Rayo achica el grande por área, al tamaño exacto. Nunca
/// reemplaza un dibujo chico hecho por el autor: si `small` no es esa reducción, devuelve None.
fn reshrunk(path: &str, small: &Rgba, size: u32) -> Option<Rgba> {
    // Un chico con suavizado de verdad (bordes semitransparentes) ya está bien: no se mira más.
    let covered = small.px.chunks_exact(4).filter(|p| p[3] > 0).count();
    let soft = small.px.chunks_exact(4).filter(|p| (0x30..=0xD0).contains(&p[3])).count();
    if covered == 0 || soft * 20 >= covered {
        return None;
    }
    let big = shell_image(path, 256, true)?;
    windows_shrunk(small, &big).then(|| shrink(&big, size))
}

/// Píxeles en premultiplicado y en `f32` (0-255).
fn premul(img: &Rgba) -> Vec<f32> {
    img.px
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as f32;
            let k = if img.premultiplied { 1.0 } else { a / 255.0 };
            [p[0] as f32 * k, p[1] as f32 * k, p[2] as f32 * k, a]
        })
        .collect()
}

/// Reducción por área (pesos fraccionarios: 256 → 20 es 12,8 a 1), separable. `src` es `sw`×`sw`.
fn area_shrink(src: &[f32], sw: usize, n: usize) -> Vec<f32> {
    let r = sw as f64 / n as f64;
    // Pesos de cada píxel de salida sobre los de entrada (iguales en x y en y).
    let weights: Vec<Vec<(usize, f32)>> = (0..n)
        .map(|o| {
            let (a, b) = (o as f64 * r, (o + 1) as f64 * r);
            (a.floor() as usize..(b.ceil() as usize).min(sw))
                .map(|i| (i, ((b.min(i as f64 + 1.0) - a.max(i as f64)) / r) as f32))
                .filter(|(_, w)| *w > 0.0)
                .collect()
        })
        .collect();
    let mut tmp = vec![0f32; n * sw * 4];
    for y in 0..sw {
        for (x, ws) in weights.iter().enumerate() {
            for &(i, w) in ws {
                for c in 0..4 {
                    tmp[(y * n + x) * 4 + c] += src[(y * sw + i) * 4 + c] * w;
                }
            }
        }
    }
    let mut out = vec![0f32; n * n * 4];
    for (y, ws) in weights.iter().enumerate() {
        for x in 0..n {
            for &(i, w) in ws {
                for c in 0..4 {
                    out[(y * n + x) * 4 + c] += tmp[(i * n + x) * 4 + c] * w;
                }
            }
        }
    }
    out
}

/// Lo que hace Windows: muestreo bilineal sin prefiltro.
fn bilinear_shrink(src: &[f32], sw: usize, n: usize) -> Vec<f32> {
    let r = sw as f32 / n as f32;
    let at = |x: usize, y: usize, c: usize| src[(y.min(sw - 1) * sw + x.min(sw - 1)) * 4 + c];
    let mut out = vec![0f32; n * n * 4];
    for y in 0..n {
        let sy = ((y as f32 + 0.5) * r - 0.5).max(0.0);
        let (y0, fy) = (sy.floor() as usize, sy.fract());
        for x in 0..n {
            let sx = ((x as f32 + 0.5) * r - 0.5).max(0.0);
            let (x0, fx) = (sx.floor() as usize, sx.fract());
            for c in 0..4 {
                let top = at(x0, y0, c) * (1.0 - fx) + at(x0 + 1, y0, c) * fx;
                let bottom = at(x0, y0 + 1, c) * (1.0 - fx) + at(x0 + 1, y0 + 1, c) * fx;
                out[(y * n + x) * 4 + c] = top * (1.0 - fy) + bottom * fy;
            }
        }
    }
    out
}

/// ¿`small` es `big` achicado por Windows sin filtrar? Se mira donde las dos reducciones (bien y
/// sin filtrar) difieren de verdad, los bordes: si ahí `small` coincide con la mala, es esa.
fn windows_shrunk(small: &Rgba, big: &Rgba) -> bool {
    let n = small.w as usize;
    if small.w != small.h || big.w != big.h || big.w as usize <= 2 * n || n == 0 {
        return false;
    }
    let (s, b) = (premul(small), premul(big));
    let good = area_shrink(&b, big.w as usize, n);
    let bad = bilinear_shrink(&b, big.w as usize, n);
    let dist = |p: &[f32], q: &[f32], i: usize| (0..4).map(|c| (p[i * 4 + c] - q[i * 4 + c]).abs()).fold(0.0, f32::max);
    let edges: Vec<usize> = (0..n * n).filter(|&i| dist(&bad, &good, i) > 24.0).collect();
    if edges.len() < 8 {
        return false;
    }
    let same = edges.iter().filter(|&&i| dist(&s, &bad, i) <= 6.0).count();
    same * 5 >= edges.len() * 4
}

/// `big` achicado por área a `n`×`n` (premultiplicado, sin enfoque: devolvería el borde duro).
fn shrink(big: &Rgba, n: u32) -> Rgba {
    let px = area_shrink(&premul(big), big.w as usize, n as usize).into_iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
    Rgba { w: n, h: n, px, premultiplied: true, orig: None, pages: None }
}

/// ¿Tiene el marco que Windows dibuja al agrandar un ícono chico? (todo el borde semitransparente)
fn framed(img: &Rgba) -> bool {
    let (w, h) = (img.w as usize, img.h as usize);
    if w < 8 || h < 8 {
        return false;
    }
    let alpha = |x: usize, y: usize| img.px[(y * w + x) * 4 + 3];
    let edge = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
    edge.into_iter().all(|(x, y)| (1..128).contains(&alpha(x, y)))
}

/// Lo que se asoma en el ícono de una carpeta: la miniatura de su primera foto (o video).
fn folder_peek(dir: &str, size: u32) -> Option<Rgba> {
    let rd = std::fs::read_dir(super::long_path(dir)).ok()?;
    let mut video = None;
    for e in rd.flatten().take(400) {
        let name = e.file_name().to_string_lossy().into_owned();
        let ext = super::ext_of(&name).to_lowercase();
        let path = super::join(dir, &name);
        if is_image_ext(&ext) && ext != "ico" {
            if super::is_online_only(&path) {
                continue;
            }
            return photo(&path, &ext, size);
        }
        if video.is_none() && matches!(ext.as_str(), "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v") {
            video = Some(path);
        }
    }
    video.and_then(|v| shell_image(&v, size as i32, false))
}

/// Fecha en que se tomó la foto (EXIF DateTimeOriginal), como "2024-07-26 18.33.12".
/// El hilo que llama debe tener COM iniciado.
pub fn exif_date(path: &str) -> Option<String> {
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    if super::is_online_only(path) {
        return None;
    }
    use windows::Win32::System::Variant::{VT_LPSTR, VT_LPWSTR};
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let w = wide(path);
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(w.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let reader = decoder.GetFrame(0).ok()?.GetMetadataQueryReader().ok()?;
        for q in ["/app1/ifd/exif/{ushort=36867}", "/ifd/exif/{ushort=36867}", "/app1/ifd/{ushort=306}", "/ifd/{ushort=306}"] {
            let wq = wide(q);
            let mut pv = PROPVARIANT::default();
            if reader.GetMetadataByName(PCWSTR(wq.as_ptr()), &mut pv).is_err() {
                continue;
            }
            let inner = &pv.Anonymous.Anonymous;
            let s = if inner.vt == VT_LPSTR {
                inner.Anonymous.pszVal.to_string().ok()
            } else if inner.vt == VT_LPWSTR {
                inner.Anonymous.pwszVal.to_string().ok()
            } else {
                None
            };
            let _ = PropVariantClear(&mut pv);
            // "2024:07:26 18:33:12" → "2024-07-26 18.33.12"
            if let Some(s) = s.filter(|s| s.len() >= 19 && s.as_bytes()[4] == b':') {
                let s = &s[..19];
                return Some(format!("{} {}", s[..10].replace(':', "-"), s[11..].replace(':', ".")));
            }
        }
        None
    }
}

#[cfg(test)]
mod reshrink_tests {
    use super::*;

    /// Un disco suavizado de 256 (como el ícono de un solo PNG grande).
    fn disc() -> Rgba {
        let mut px = vec![0u8; 256 * 256 * 4];
        for y in 0..256 {
            for x in 0..256 {
                let d = (((x as f32 - 127.5).powi(2) + (y as f32 - 127.5).powi(2)).sqrt() - 100.0).clamp(-0.5, 0.5);
                let a = ((0.5 - d) * 255.0) as u8;
                px[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].copy_from_slice(&[220, 40, 60, a]);
            }
        }
        Rgba { w: 256, h: 256, px, premultiplied: false, orig: None, pages: None }
    }

    fn to_rgba(v: Vec<f32>, n: u32) -> Rgba {
        Rgba { w: n, h: n, px: v.into_iter().map(|x| x.round().clamp(0.0, 255.0) as u8).collect(), premultiplied: true, orig: None, pages: None }
    }

    #[test]
    fn detects_windows_unfiltered_shrink() {
        let big = disc();
        let b = premul(&big);
        let bad = to_rgba(bilinear_shrink(&b, 256, 16), 16);
        let good = to_rgba(area_shrink(&b, 256, 16), 16);
        assert!(windows_shrunk(&bad, &big), "el achicado sin filtrar se reconoce");
        assert!(!windows_shrunk(&good, &big), "uno bien hecho se respeta");
        let s = shrink(&big, 20);
        assert_eq!((s.w, s.h, s.px.len()), (20, 20, 20 * 20 * 4));
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Cuánto tarda cada forma de sacar una miniatura de las fotos de una carpeta.
    #[test]
    #[ignore]
    fn thumbnails_speed() {
        let Some(dir) = std::env::var_os("RAYO_BENCH_FOTOS") else { return };
        crate::fsx::shell::com_init();
        let files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path().display().to_string()).filter(|p| is_image_ext(&crate::fsx::ext_of(p))).take(12).collect();
        for size in [256u32, 768] {
            let t = std::time::Instant::now();
            let mut dims = (0, 0);
            for f in &files {
                let img = wic_load(f, size).expect("wic");
                dims = (img.w, img.h);
            }
            eprintln!("WIC {size}px: {:.0} ms por foto ({}x{})", t.elapsed().as_secs_f64() * 1000.0 / files.len() as f64, dims.0, dims.1);
        }
        let t = std::time::Instant::now();
        for f in &files {
            shell_image(f, 256, false).expect("shell");
        }
        eprintln!("Shell 256px: {:.0} ms por foto", t.elapsed().as_secs_f64() * 1000.0 / files.len() as f64);
        let t = std::time::Instant::now();
        let mut got = (0, 0);
        for f in &files {
            if let Some(img) = embedded_thumb(f) {
                got = (img.w, img.h);
            }
        }
        eprintln!("Incrustada: {:.1} ms por foto ({}x{} sin franjas)", t.elapsed().as_secs_f64() * 1000.0 / files.len() as f64, got.0, got.1);
    }

    /// Lee sin pasar por la caché de Windows (FILE_FLAG_NO_BUFFERING): lo que cuesta de verdad
    /// en el disco, cada vez. Devuelve los bytes leídos.
    fn read_raw(path: &str, offset: u64, len: usize) -> usize {
        use std::io::{Read, Seek};
        use std::os::windows::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().read(true).custom_flags(0x2000_0000).open(path).unwrap();
        let chunk = 1 << 20;
        let layout = std::alloc::Layout::from_size_align(chunk, 4096).unwrap();
        let buf = unsafe { std::slice::from_raw_parts_mut(std::alloc::alloc(layout), chunk) };
        f.seek(std::io::SeekFrom::Start(offset & !4095)).unwrap();
        let mut got = 0;
        while got < len {
            let want = (len - got).next_multiple_of(4096).min(chunk);
            let n = f.read(&mut buf[..want]).unwrap_or(0);
            if n == 0 {
                break;
            }
            got += n;
        }
        unsafe { std::alloc::dealloc(buf.as_mut_ptr(), layout) };
        got
    }

    /// Disco mecánico: el principio de cada foto (lo que trae la vista previa incrustada) contra
    /// la foto entera, de a una y de a tres a la vez. `RAYO_DIAG_DIR`: carpeta con fotos.
    #[test]
    #[ignore]
    fn diag_hdd_reads() {
        let Ok(dir) = std::env::var("RAYO_DIAG_DIR") else { return };
        let skip: usize = std::env::var("RAYO_DIAG_SKIP").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path().display().to_string()).filter(|p| is_image_ext(&crate::fsx::ext_of(p))).collect();
        files.sort();
        let files = &files[skip..];
        let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
        // Cabeceras: 256 KB de 24 fotos, en orden.
        let t = std::time::Instant::now();
        for f in &files[..24] {
            read_raw(f, 0, 256 << 10);
        }
        eprintln!("cabecera 256 KB, en orden: {:.1} ms por foto", ms(t) / 24.0);
        // Las mismas al revés (salto hacia atrás en cada una).
        let t = std::time::Instant::now();
        for f in files[24..48].iter().rev() {
            read_raw(f, 0, 256 << 10);
        }
        eprintln!("cabecera 256 KB, al revés: {:.1} ms por foto", ms(t) / 24.0);
        // Cabeceras de a tres hilos.
        let t = std::time::Instant::now();
        std::thread::scope(|s| {
            for k in 0..3 {
                let part: Vec<&String> = files[48..72].iter().skip(k).step_by(3).collect();
                s.spawn(move || part.iter().for_each(|f| _ = read_raw(f, 0, 256 << 10)));
            }
        });
        eprintln!("cabecera 256 KB, 3 hilos: {:.1} ms por foto", ms(t) / 24.0);
        // Fotos enteras: 6 de a una, 6 de a tres a la vez.
        let t = std::time::Instant::now();
        let mut bytes = 0;
        for f in &files[72..78] {
            bytes += read_raw(f, 0, usize::MAX >> 1);
        }
        eprintln!("enteras de a una: {:.0} ms por foto, {:.0} MB/s", ms(t) / 6.0, bytes as f64 / 1e6 / (ms(t) / 1000.0));
        let t = std::time::Instant::now();
        let total = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for k in 0..3 {
                let part: Vec<&String> = files[78..84].iter().skip(k).step_by(3).collect();
                let total = &total;
                s.spawn(move || part.iter().for_each(|f| _ = total.fetch_add(read_raw(f, 0, usize::MAX >> 1), std::sync::atomic::Ordering::Relaxed)));
            }
        });
        let bytes = total.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("enteras de a tres: {:.0} ms por foto, {:.0} MB/s", ms(t) / 6.0, bytes as f64 / 1e6 / (ms(t) / 1000.0));
    }

    /// Diagnóstico: cada método sobre cada archivo de `RAYO_DIAG_FILES` (separados por `;`).
    #[test]
    #[ignore]
    fn diag_methods() {
        let Ok(list) = std::env::var("RAYO_DIAG_FILES") else { return };
        let size: u32 = std::env::var("RAYO_DIAG_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(128);
        crate::fsx::shell::com_init();
        let show = |name: &str, t: std::time::Instant, r: Option<Rgba>| {
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            match r {
                Some(i) => eprintln!("    {name:<14} {ms:8.1} ms  {}x{} shape={:?}", i.w, i.h, i.shape()),
                None => eprintln!("    {name:<14} {ms:8.1} ms  NONE"),
            }
        };
        for f in list.split(';').filter(|s| !s.is_empty()) {
            eprintln!("{f}");
            let methods = std::env::var("RAYO_DIAG_METHODS").unwrap_or_else(|_| "cached,embedded,shell_thumb,shell,wic,load".into());
            for m in methods.split(',') {
                let t = std::time::Instant::now();
                let num = |pre: &str| m.strip_prefix(pre).and_then(|n| n.parse::<u32>().ok());
                let r = match m {
                    _ if num("pv:").is_some() => jpeg_preview(f, num("pv:").unwrap_or(0)),
                    _ if num("wic:").is_some() => wic_load(f, num("wic:").unwrap_or(0)),
                    "peek" => {
                        let p = crate::fsx::jpeg::peek(f);
                        eprintln!("    peek: {:?}", p.as_ref().map(|p| (p.orient, p.size, p.previews.iter().map(|v| (v.w, v.h)).collect::<Vec<_>>())));
                        None
                    }
                    "cached" => shell_cached(f, size as i32),
                    "embedded" => embedded_thumb(f),
                    "shell_thumb" => shell_thumb(f, size as i32),
                    "shell" => shell_image(f, size as i32, false),
                    "wic" => wic_load(f, size),
                    "load" => load(&ThumbKey { path: f.to_string(), size }),
                    _ => continue,
                };
                show(m, t, r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una imagen grande va en franjas que cubren todas sus filas una sola vez, con `GUTTER`
    /// filas de las vecinas en cada lado; cada franja con los píxeles de sus filas.
    #[test]
    fn big_images_go_in_bands() {
        let (w, h) = (3000u32, 2001u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| [(i / w % 256) as u8, 7, 9, 255]).collect();
        let img = Rgba { w, h, px, premultiplied: true, orig: None, pages: None };
        let bands = img.to_bands();
        assert!(bands.len() > 1);
        let mut next = 0;
        for b in &bands {
            assert_eq!(b.rows.start, next);
            assert!(b.img.pixels.len() * 4 <= BAND_BYTES + 2 * GUTTER * w as usize * 4);
            assert_eq!(b.top, b.rows.start.saturating_sub(GUTTER));
            assert_eq!(b.img.size, [w as usize, (b.rows.end + GUTTER).min(h as usize) - b.top]);
            // La primera fila de la textura es la fila `top` de la imagen.
            assert_eq!(b.img.pixels[0].r(), (b.top % 256) as u8);
            next = b.rows.end;
        }
        assert_eq!(next, h as usize);
        // Una chica, entera.
        let small = Rgba { w: 64, h: 64, px: vec![255; 64 * 64 * 4], premultiplied: true, orig: None, pages: None };
        let b = small.to_bands();
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].rows.clone(), b[0].top, b[0].img.size), (0..64, 0, [64, 64]));
        assert_eq!(small.shape(), (true, false));
    }

    /// Una miniatura 4:3 con franjas negras arriba y abajo queda solo con la foto.
    #[test]
    fn crops_black_bars() {
        let (w, h) = (16u32, 12u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 2..10 {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[200, 120, 40, 255]);
            }
        }
        assert_eq!(content_box(&px, w, h), (0, 2, 16, 10));
    }

    /// Orientación 6 (girar 90°): una imagen de 3x2 pasa a 2x3 y el píxel de arriba a la
    /// izquierda termina arriba a la derecha.
    #[test]
    fn rotates_by_exif() {
        let mut px = vec![0u8; 3 * 2 * 4];
        px[0] = 255;
        let (out, w, h) = orient_pixels(px, 3, 2, 6);
        assert_eq!((w, h), (2, 3));
        assert_eq!(out[4], 255);
        // Las ocho, contra la definición píxel por píxel.
        let (w, h) = (5u32, 3u32);
        let px: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        for o in 1..=8u16 {
            let (out, nw, nh) = orient_pixels(px.clone(), w, h, o);
            assert_eq!((nw, nh), if o >= 5 { (h, w) } else { (w, h) });
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = match o {
                        1 => (x, y),
                        2 => (w - 1 - x, y),
                        3 => (w - 1 - x, h - 1 - y),
                        4 => (x, h - 1 - y),
                        5 => (y, x),
                        6 => (h - 1 - y, x),
                        7 => (h - 1 - y, w - 1 - x),
                        _ => (y, w - 1 - x),
                    };
                    assert_eq!(out[((dy * nw + dx) * 4) as usize], (y * w + x) as u8, "orientación {o}");
                }
            }
        }
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    /// Windows da el ícono propio de un programa y el del tipo de un archivo cualquiera, con
    /// transparencia (para los íconos de las listas).
    #[test]
    fn shell_gives_icons() {
        let _shell = crate::shell_test_lock();
        super::super::shell::com_init();
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let exe = shell_image(&format!(r"{windir}\notepad.exe"), 32, true).expect("ícono de notepad.exe");
        assert!(exe.w >= 16 && exe.h >= 16);
        assert!(exe.px.chunks(4).any(|p| p[3] == 0), "el ícono debería tener partes transparentes");
        let d = std::env::temp_dir().join(format!("rayo-icono-{}.zip", std::process::id()));
        std::fs::write(&d, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        let zip = shell_image(&d.to_string_lossy(), 32, true);
        let _ = std::fs::remove_file(&d);
        assert!(zip.is_some(), "ícono de un .zip");
    }
}

/// El ícono como lo muestra el Explorador: el de la lista de imágenes del sistema, que trae cada
/// tamaño estándar (16, 32, 48, 256) dibujado para ese tamaño si el ícono lo trae (si no, Windows
/// lo achica del grande: ver `reshrunk`). Devuelve el de la lista más cercana por arriba a `size`.
pub fn system_icon(path: &str, size: u32) -> Option<Rgba> {
    use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, GetSystemMetrics, ICONINFO, SM_CXSMICON};
    unsafe {
        let w = wide(path);
        let mut sfi = SHFILEINFOW::default();
        if SHGetFileInfoW(PCWSTR(w.as_ptr()), Default::default(), Some(&mut sfi), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) == 0 {
            return None;
        }
        let small = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let list = if size <= small {
            SHIL_SMALL
        } else if size <= 32 {
            SHIL_LARGE
        } else if size <= 48 {
            SHIL_EXTRALARGE
        } else {
            SHIL_JUMBO
        };
        let il: IImageList = SHGetImageList(list as i32).ok()?;
        let hicon = il.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0).ok()?;
        let mut ii = ICONINFO::default();
        let got = GetIconInfo(hicon, &mut ii).is_ok();
        let _ = DestroyIcon(hicon);
        if !got {
            return None;
        }
        let color = bitmap_pixels(ii.hbmColor);
        let mask = bitmap_pixels(ii.hbmMask);
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        let mut img = color?;
        // Los íconos de 32 bits traen transparencia propia (sin premultiplicar); los viejos, una máscara.
        // Se decide por si hay transparencia, no por `premultiplied`: un ícono de 32 bits que
        // `bitmap_pixels` reconoce como no premultiplicado (el de Arc) también trae la suya, y la
        // máscara de 1 bit le dejaría los bordes en escalera con un halo blanco.
        let own_alpha = img.px.chunks_exact(4).any(|p| p[3] != 255);
        if own_alpha {
            img.premultiplied = false;
        } else if let Some(m) = mask.filter(|m| m.w == img.w && m.h >= img.h) {
            for (p, q) in img.px.chunks_exact_mut(4).zip(m.px.chunks_exact(4)) {
                p[3] = if q[0] > 127 { 0 } else { 255 };
            }
        }
        Some(img)
    }
}

#[cfg(test)]
mod lcn_bench {
    /// Diagnóstico: ¿el orden por nombre de las fotos de una carpeta es su orden físico en el
    /// disco? (primer LCN de cada archivo, sin leer sus datos). `RAYO_DIAG_DIR`.
    #[test]
    #[ignore]
    fn diag_lcn_order() {
        use windows::Win32::Storage::FileSystem::*;
        use windows::Win32::System::IO::DeviceIoControl;
        use windows::Win32::System::Ioctl::*;
        let Ok(dir) = std::env::var("RAYO_DIAG_DIR") else { return };
        let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path().display().to_string()).collect();
        files.sort();
        let t = std::time::Instant::now();
        let mut lcns = Vec::new();
        for f in &files {
            let w = crate::index::walk::wide(f);
            let h = unsafe { CreateFileW(windows::core::PCWSTR(w.as_ptr()), FILE_READ_ATTRIBUTES.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) };
            let Ok(h) = h else { continue };
            let input = STARTING_VCN_INPUT_BUFFER { StartingVcn: 0 };
            let mut out = [0u64; 8];
            let mut got = 0u32;
            let _ = unsafe { DeviceIoControl(h, FSCTL_GET_RETRIEVAL_POINTERS, Some(&input as *const _ as *const _), std::mem::size_of_val(&input) as u32, Some(out.as_mut_ptr() as *mut _), 64, Some(&mut got), None) };
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
            // RETRIEVAL_POINTERS_BUFFER: ExtentCount (u32 + relleno), StartingVcn, [NextVcn, Lcn]…
            lcns.push((out[3] as i64, out[0] as u32));
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let ordered = lcns.windows(2).filter(|w| w[1].0 >= w[0].0).count();
        let extents: Vec<u32> = lcns.iter().map(|l| l.1).collect();
        let frag = extents.iter().filter(|&&e| e > 1).count();
        let jumps: i64 = lcns.windows(2).map(|w| (w[1].0 - w[0].0).abs()).sum();
        let mut sorted: Vec<i64> = lcns.iter().map(|l| l.0).collect();
        sorted.sort();
        let jumps_sorted: i64 = sorted.windows(2).map(|w| w[1] - w[0]).sum();
        eprintln!("{} archivos, LCN en {ms:.0} ms ({:.2} ms c/u); en orden por nombre: {ordered}/{}; fragmentados: {frag}; recorrido por nombre {jumps} vs ordenado {jumps_sorted} clústeres", lcns.len(), ms / lcns.len().max(1) as f64, lcns.len().saturating_sub(1));
    }
}
