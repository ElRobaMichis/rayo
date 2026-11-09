//! Lo que Windows sabe de un archivo más allá del tamaño y la fecha: cámara y exposición de una
//! foto, duración y resolución de un video, artista y álbum de una canción (con el sistema de
//! propiedades, ya en el formato del idioma: "f/1.8", "1/120 s", "00:03:25"…). Y la marca de
//! "descargado de Internet" (el flujo `:Zone.Identifier`), que bloquea la vista previa de PDF y Office.
//!
//! Leer propiedades de un video puede tardar: se hace en un hilo aparte y se guarda.

use super::long_path;
use crossbeam_channel::{Sender, unbounded};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Un dato que se muestra en la vista previa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Camera,
    Lens,
    Exposure,
    Taken,
    Duration,
    Resolution,
    FrameRate,
    Bitrate,
    Artist,
    Album,
}

#[derive(Debug, Default)]
pub struct Info {
    pub rows: Vec<(Field, String)>,
    /// Descargado de Internet (Windows bloquea su vista previa).
    pub blocked: bool,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    /// None = leyéndose.
    cache: Mutex<HashMap<String, Option<Arc<Info>>>>,
    tx: Sender<String>,
    wake: Mutex<Option<Wake>>,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| {
        let (tx, rx) = unbounded::<String>();
        std::thread::Builder::new()
            .name("propiedades".into())
            .spawn(move || {
                super::shell::com_init();
                for path in rx {
                    let info = Arc::new(read(&path));
                    let s = shared();
                    s.cache.lock().insert(path, Some(info));
                    if let Some(w) = s.wake.lock().as_ref() {
                        w();
                    }
                }
            })
            .ok();
        Shared { cache: Mutex::new(HashMap::new()), tx, wake: Mutex::new(None) }
    })
}

/// Con qué repintar cuando llega un dato.
pub fn set_wake(w: Wake) {
    *shared().wake.lock() = Some(w);
}

/// Los datos de un archivo, si ya están (si no, se piden y avisan al llegar).
pub fn info(path: &str) -> Option<Arc<Info>> {
    let s = shared();
    let mut cache = s.cache.lock();
    if let Some(v) = cache.get(path) {
        return v.clone();
    }
    if cache.len() > 2000 {
        cache.retain(|_, v| v.is_none());
    }
    cache.insert(path.to_string(), None);
    let _ = s.tx.send(path.to_string());
    None
}

/// Olvidar lo leído de estos archivos (cambiaron).
pub fn forget(paths: &[String]) {
    let mut cache = shared().cache.lock();
    for p in paths {
        cache.remove(p);
    }
}

fn zone_stream(path: &str) -> String {
    format!("{}:Zone.Identifier", long_path(path))
}

/// ¿Tiene la marca de "descargado de Internet"?
pub fn is_blocked(path: &str) -> bool {
    std::fs::metadata(zone_stream(path)).is_ok()
}

/// Quita la marca. Devuelve true si la tenía y se quitó.
pub fn unblock(path: &str) -> bool {
    std::fs::remove_file(zone_stream(path)).is_ok()
}

/// Quita la marca de estos archivos y de todo lo de adentro de estas carpetas. Devuelve cuántos
/// archivos se desbloquearon.
pub fn unblock_all(paths: &[String]) -> usize {
    let mut done: Vec<String> = Vec::new();
    let mut stack: Vec<String> = paths.to_vec();
    let mut seen = 0usize;
    while let Some(p) = stack.pop() {
        seen += 1;
        if seen > 500_000 {
            break;
        }
        let Ok(m) = std::fs::symlink_metadata(long_path(&p)) else { continue };
        if m.is_dir() {
            if let Ok(rd) = std::fs::read_dir(long_path(&p)) {
                for e in rd.flatten() {
                    stack.push(super::join(&p, &e.file_name().to_string_lossy()));
                }
            }
        } else if unblock(&p) {
            done.push(p);
        }
    }
    forget(&done);
    done.len()
}

const PHOTO: &[&str] = &["jpg", "jpeg", "jfif", "heic", "heif", "tif", "tiff", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "webp", "png"];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "mkv", "avi", "wmv", "webm", "mpg", "mpeg", "3gp", "ts", "mts", "m2ts"];
const AUDIO: &[&str] = &["mp3", "flac", "m4a", "aac", "wav", "ogg", "opus", "wma", "aiff", "alac"];

fn read(path: &str) -> Info {
    let blocked = is_blocked(path);
    let ext = super::ext_of(path).to_lowercase();
    let (photo, video, audio) = (PHOTO.contains(&ext.as_str()), VIDEO.contains(&ext.as_str()), AUDIO.contains(&ext.as_str()));
    let mut rows = Vec::new();
    if photo || video || audio {
        if let Some(store) = Store::open(path) {
            if photo {
                let make = store.get("System.Photo.CameraManufacturer");
                let model = store.get("System.Photo.CameraModel");
                let camera = match (make, model) {
                    // Muchas cámaras repiten la marca en el modelo ("Canon" + "Canon EOS R6").
                    (Some(a), Some(b)) if b.to_lowercase().starts_with(&a.to_lowercase()) => Some(b),
                    (Some(a), Some(b)) => Some(format!("{a} {b}")),
                    (a, b) => a.or(b),
                };
                rows.extend(camera.map(|c| (Field::Camera, c)));
                rows.extend(store.get("System.Photo.LensModel").map(|l| (Field::Lens, l)));
                let exposure: Vec<String> = ["System.Photo.FNumber", "System.Photo.ExposureTime", "System.Photo.ISOSpeed", "System.Photo.FocalLength"]
                    .iter()
                    .filter_map(|k| store.get(k))
                    .collect();
                if !exposure.is_empty() {
                    rows.push((Field::Exposure, exposure.join(" · ")));
                }
                rows.extend(store.get("System.Photo.DateTaken").map(|d| (Field::Taken, d)));
            }
            if video || audio {
                rows.extend(store.get("System.Media.Duration").map(|d| (Field::Duration, d)));
            }
            if video {
                if let (Some(w), Some(h)) = (store.number("System.Video.FrameWidth"), store.number("System.Video.FrameHeight")) {
                    rows.push((Field::Resolution, format!("{w} × {h}")));
                }
                rows.extend(store.get("System.Video.FrameRate").map(|f| (Field::FrameRate, f)));
            }
            if audio {
                rows.extend(store.get("System.Music.Artist").or_else(|| store.get("System.Music.AlbumArtist")).map(|a| (Field::Artist, a)));
                rows.extend(store.get("System.Music.AlbumTitle").map(|a| (Field::Album, a)));
                rows.extend(store.get("System.Audio.EncodingBitrate").map(|b| (Field::Bitrate, b)));
            }
        }
    }
    Info { rows, blocked }
}

struct Store(windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore);

impl Store {
    fn open(path: &str) -> Option<Store> {
        use windows::Win32::UI::Shell::PropertiesSystem::*;
        let w: Vec<u16> = path.encode_utf16().chain([0]).collect();
        unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(windows::core::PCWSTR(w.as_ptr()), None, GPS_BESTEFFORT) }.ok().map(Store)
    }

    fn value(&self, name: &str) -> Option<(windows::Win32::Foundation::PROPERTYKEY, Prop)> {
        use windows::Win32::UI::Shell::PropertiesSystem::*;
        let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unsafe {
            let mut key = windows::Win32::Foundation::PROPERTYKEY::default();
            PSGetPropertyKeyFromName(windows::core::PCWSTR(w.as_ptr()), &mut key).ok()?;
            let v = Prop(self.0.GetValue(&key).ok()?);
            (v.0.Anonymous.Anonymous.vt != windows::Win32::System::Variant::VT_EMPTY).then_some((key, v))
        }
    }

    /// El valor como lo muestra Windows (en el idioma del sistema).
    fn get(&self, name: &str) -> Option<String> {
        use windows::Win32::UI::Shell::PropertiesSystem::*;
        let (key, v) = self.value(name)?;
        unsafe {
            let p = PSFormatForDisplayAlloc(&key, &v.0, PDFF_DEFAULT).ok()?;
            let s = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            // Windows marca la dirección del texto con caracteres invisibles (p. ej. en las fechas).
            let s: String = s?.chars().filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}')).collect();
            let s = s.trim().to_string();
            (!s.is_empty()).then_some(s)
        }
    }

    fn number(&self, name: &str) -> Option<u64> {
        let (_, v) = self.value(name)?;
        unsafe { windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64(&v.0) }.ok().filter(|&n| n > 0)
    }
}

/// Un PROPVARIANT que se libera solo.
struct Prop(windows::Win32::System::Com::StructuredStorage::PROPVARIANT);

impl Drop for Prop {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Com::StructuredStorage::PropVariantClear(&mut self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un archivo con la marca de Internet se reconoce y se desbloquea (también dentro de carpetas).
    #[test]
    fn unblocks_downloaded_files() {
        let d = std::env::temp_dir().join(format!("rayo-props-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        let a = d.join("a.pdf").to_string_lossy().into_owned();
        let b = d.join("sub").join("b.docx").to_string_lossy().into_owned();
        let c = d.join("c.txt").to_string_lossy().into_owned();
        for f in [&a, &b, &c] {
            std::fs::write(f, b"x").unwrap();
        }
        for f in [&a, &b] {
            std::fs::write(format!("{f}:Zone.Identifier"), "[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        }
        assert!(is_blocked(&a) && is_blocked(&b) && !is_blocked(&c));
        assert_eq!(unblock_all(&[d.to_string_lossy().into_owned()]), 2);
        assert!(!is_blocked(&a) && !is_blocked(&b));
        assert_eq!(std::fs::read(&a).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Windows sabe la duración de un sonido del sistema.
    #[test]
    fn reads_audio_duration() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let wav = format!(r"{windir}\Media\Windows Notify System Generic.wav");
        if !std::path::Path::new(&wav).exists() {
            return;
        }
        super::super::shell::com_init();
        let info = read(&wav);
        println!("{:?}", info.rows);
        assert!(info.rows.iter().any(|(f, v)| *f == Field::Duration && v.contains(':')), "{:?}", info.rows);
    }
}
