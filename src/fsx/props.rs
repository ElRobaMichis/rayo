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
    /// Ancho × alto (fotos y videos), para la columna Dimensiones.
    pub dims: Option<String>,
    /// Se le pueden cambiar título, comentarios y calificación (fotos, música, video, Office).
    pub editable: bool,
    pub title: String,
    pub comment: String,
    /// Calificación de Windows (0, 1, 25, 50, 75 o 99 = 0 a 5 estrellas).
    pub rating: u32,
}

/// Tipos en los que Windows guarda título, comentarios y calificación dentro del archivo.
const EDITABLE: &[&str] = &["jpg", "jpeg", "jfif", "tif", "tiff", "heic", "mp3", "wma", "m4a", "flac", "mp4", "m4v", "mov", "wmv", "docx", "xlsx", "pptx"];

/// Un valor para escribir en las propiedades de un archivo.
#[derive(Debug, Clone)]
pub enum Value {
    Text(String),
    Number(u32),
}

/// Escribe una propiedad ("System.Title", "System.Comment", "System.Rating") en el archivo.
/// El hilo que llama debe tener COM iniciado.
pub fn set(path: &str, name: &str, value: &Value) -> Result<(), String> {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::{VT_LPWSTR, VT_UI4};
    use windows::Win32::UI::Shell::PropertiesSystem::*;
    let w: Vec<u16> = path.encode_utf16().chain([0]).collect();
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreFromParsingName(windows::core::PCWSTR(w.as_ptr()), None, GPS_READWRITE).map_err(|e| e.message())?;
        let n: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut key = windows::Win32::Foundation::PROPERTYKEY::default();
        PSGetPropertyKeyFromName(windows::core::PCWSTR(n.as_ptr()), &mut key).map_err(|e| e.message())?;
        let mut pv = Prop(PROPVARIANT::default());
        match value {
            Value::Text(t) => {
                // Copia en memoria de COM: PropVariantClear la libera.
                let wide: Vec<u16> = t.encode_utf16().chain([0]).collect();
                let mem = windows::Win32::System::Com::CoTaskMemAlloc(wide.len() * 2) as *mut u16;
                if mem.is_null() {
                    return Err("sin memoria".into());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), mem, wide.len());
                let inner = &mut *pv.0.Anonymous.Anonymous;
                inner.vt = VT_LPWSTR;
                inner.Anonymous.pwszVal = windows::core::PWSTR(mem);
            }
            Value::Number(v) => {
                let inner = &mut *pv.0.Anonymous.Anonymous;
                inner.vt = VT_UI4;
                inner.Anonymous.ulVal = *v;
            }
        }
        store.SetValue(&key, &pv.0).map_err(|e| e.message())?;
        store.Commit().map_err(|e| e.message())?;
    }
    forget(&[path.to_string()]);
    Ok(())
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
    let mut dims = None;
    let editable = EDITABLE.contains(&ext.as_str());
    let (mut title, mut comment, mut rating) = (String::new(), String::new(), 0u32);
    if editable {
        if let Some(store) = Store::open(path) {
            title = store.text("System.Title").unwrap_or_default();
            comment = store.text("System.Comment").unwrap_or_default();
            rating = store.number("System.Rating").unwrap_or(0) as u32;
        }
    }
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
                if let (Some(w), Some(h)) = (store.number("System.Image.HorizontalSize"), store.number("System.Image.VerticalSize")) {
                    dims = Some(format!("{w} × {h}"));
                }
            }
            if video || audio {
                rows.extend(store.get("System.Media.Duration").map(|d| (Field::Duration, d)));
            }
            if video {
                if let (Some(w), Some(h)) = (store.number("System.Video.FrameWidth"), store.number("System.Video.FrameHeight")) {
                    rows.push((Field::Resolution, format!("{w} × {h}")));
                    dims = Some(format!("{w} × {h}"));
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
    Info { rows, blocked, dims, editable, title, comment, rating }
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

    /// El texto guardado tal cual (sin el formato de pantalla).
    fn text(&self, name: &str) -> Option<String> {
        let (_, v) = self.value(name)?;
        unsafe {
            let mut buf = [0u16; 2048];
            windows::Win32::System::Com::StructuredStorage::PropVariantToString(&v.0, &mut buf).ok()?;
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..n]))
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

    /// Título, comentario y calificación se escriben en el archivo y se vuelven a leer.
    #[test]
    fn writes_title_comment_and_rating() {
        super::super::shell::com_init();
        let p = std::env::temp_dir().join(format!("rayo-meta-{}.jpg", std::process::id()));
        std::fs::write(&p, include_bytes!("testdata/foto.jpg")).unwrap();
        let path = p.to_string_lossy().into_owned();
        set(&path, "System.Title", &Value::Text("Atardecer en la playa".into())).expect("título");
        set(&path, "System.Comment", &Value::Text("Prueba de Rayo, con acentos: ñandú".into())).expect("comentario");
        set(&path, "System.Rating", &Value::Number(75)).expect("calificación");
        let info = read(&path);
        assert!(info.editable);
        assert_eq!(info.title, "Atardecer en la playa");
        assert_eq!(info.comment, "Prueba de Rayo, con acentos: ñandú");
        assert_eq!(info.rating, 75);
        let _ = std::fs::remove_file(&p);
    }
}
