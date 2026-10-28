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
    unsafe {
        let frame = decoder.GetFrame(0).ok()?;
        let (mut fw, mut fh) = (0u32, 0u32);
        frame.GetSize(&mut fw, &mut fh).ok()?;
        if fw == 0 || fh == 0 {
            return None;
        }
        let orient = exif_orientation(&frame);
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
        let transform = match orient {
            2 => WICBitmapTransformFlipHorizontal,
            3 => WICBitmapTransformRotate180,
            4 => WICBitmapTransformFlipVertical,
            5 => WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0),
            6 => WICBitmapTransformRotate90,
            7 => WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0),
            8 => WICBitmapTransformRotate270,
            _ => WICBitmapTransformRotate0,
        };
        if transform != WICBitmapTransformRotate0 {
            let rot = factory.CreateBitmapFlipRotator().ok()?;
            rot.Initialize(&source, transform).ok()?;
            source = rot.into();
        }
        let converted = WICConvertBitmapSource(&GUID_WICPixelFormat32bppRGBA, &source).ok()?;
        let (mut cw, mut ch) = (0u32, 0u32);
        converted.GetSize(&mut cw, &mut ch).ok()?;
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        converted.CopyPixels(std::ptr::null(), cw * 4, &mut px).ok()?;
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
    let mut out = vec![0u8; px.len()];
    for y in 0..h {
        for x in 0..w {
            // A dónde va el píxel (x, y) de la imagen guardada.
            let (dx, dy) = match orient {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x),
            };
            let s = ((y * w + x) * 4) as usize;
            let d = ((dy * nw + dx) * 4) as usize;
            out[d..d + 4].copy_from_slice(&px[s..s + 4]);
        }
    }
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
    shell_image_with(path, size, icon_only, false)
}

/// La miniatura que Windows ya tenga guardada en su caché (no genera nada: si no está, None).
pub fn shell_cached(path: &str, size: i32) -> Option<Rgba> {
    shell_image_with(path, size, false, true)
}

fn shell_image_with(path: &str, size: i32, icon_only: bool, cache_only: bool) -> Option<Rgba> {
    // Archivos solo en la nube: la miniatura que Windows ya tenga guardada, o el ícono (nunca descargar).
    let cloud = !icon_only && super::is_online_only(path);
    unsafe {
        let w = wide(path);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let flags = if icon_only {
            SIIGBF_ICONONLY
        } else if cloud || cache_only {
            SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK | SIIGBF_INCACHEONLY
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
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            any_alpha |= p[3] != 0;
        }
        if !any_alpha {
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
        }
        Some(Rgba { w: bw as u32, h: bh as u32, px, premultiplied: any_alpha, orig: None, pages: None })
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
    pub img: Option<Rgba>,
}

/// Lo que se quiere cargar: en qué orden se pidió, en qué cuadro se pidió por última vez
/// (lo que dejó de pedirse salió de la pantalla) y si es urgente (vista previa).
struct Want {
    first: u64,
    frame: u64,
    urgent: bool,
}

#[derive(Default)]
struct Queue {
    wanted: std::collections::HashMap<ThumbKey, Want>,
    busy: HashSet<ThumbKey>,
    seq: u64,
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
                        let key = {
                            let (lock, cv) = &*queue;
                            let mut q = lock.lock();
                            loop {
                                // Olvidar lo que hace unos cuadros que no se pide.
                                let now = frame.load(std::sync::atomic::Ordering::Relaxed);
                                q.wanted.retain(|_, w| w.urgent || w.frame + 2 >= now);
                                let pick = q.wanted.iter().filter(|(_, w)| !urgent_only || w.urgent).min_by_key(|(_, w)| (!w.urgent, w.first)).map(|(k, _)| k.clone());
                                if let Some(k) = pick {
                                    q.wanted.remove(&k);
                                    q.busy.insert(k.clone());
                                    break k;
                                }
                                cv.wait_for(&mut q, std::time::Duration::from_millis(500));
                            }
                        };
                        let img = load(&key);
                        queue.0.lock().busy.remove(&key);
                        let _ = res_tx.send(ThumbResult { key, img });
                        wake();
                    }
                })
                .ok();
        }
        Arc::new(ThumbLoader { queue, frame, rx })
    }

    /// Pedir una imagen (se llama en cada cuadro mientras se necesite).
    pub fn request(&self, key: ThumbKey, urgent: bool) {
        let (lock, cv) = &*self.queue;
        let mut q = lock.lock();
        if q.busy.contains(&key) {
            return;
        }
        let frame = self.frame.load(std::sync::atomic::Ordering::Relaxed);
        if let Some(w) = q.wanted.get_mut(&key) {
            w.frame = frame;
            w.urgent |= urgent;
            if !urgent {
                return;
            }
        } else {
            let first = q.seq;
            q.seq += 1;
            q.wanted.insert(key, Want { first, frame, urgent });
        }
        cv.notify_all();
    }

    /// Un cuadro nuevo de la interfaz (para saber qué dejó de pedirse).
    pub fn next_frame(&self) {
        self.frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Carga una imagen: página de PDF, foto o miniatura del shell.
fn load(key: &ThumbKey) -> Option<Rgba> {
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
        // Miniaturas, de lo más rápido a lo más lento: la caché de Windows (la del Explorador,
        // ~4 ms), la miniatura que guardó la cámara (~10 ms, si alcanza el tamaño) y, si no,
        // la foto entera (~100–200 ms; Windows la guarda en su caché para la próxima vez).
        // Para la vista previa (grande) se decodifica la foto, con mejor calidad.
        return if key.size <= 512 {
            shell_cached(&path, key.size as i32)
                .or_else(|| embedded_thumb(&path).filter(|t| t.w.max(t.h) as f32 >= key.size as f32 * 0.75))
                .or_else(|| shell_image(&path, key.size as i32, false))
                .or_else(|| wic_load(&path, key.size))
        } else {
            wic_load(&path, key.size).or_else(|| shell_image(&path, key.size as i32, false))
        };
    }
    shell_image(&path, key.size as i32, false)
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(out[((0 * w + 1) * 4) as usize], 255);
    }
}
