//! «Abrir con ‹programa›»: nombre, ejecutable e ícono del programa predeterminado de una
//! extensión (`AssocQueryStringW`), preguntado en un hilo y recordado.
//!
//! Se pregunta por extensión, sin tocar el archivo: uno que está solo en la nube no se
//! descarga por mirarlo. La asociación puede cambiar con Rayo abierto: la respuesta vence a
//! los 60 s (mientras se vuelve a preguntar, se sigue mostrando la vieja).

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::UI::Shell::*;
use windows::core::{PCWSTR, PWSTR};

static WAKE: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

/// Para despertar a la interfaz cuando llega una respuesta.
pub fn set_wake(w: Arc<dyn Fn() + Send + Sync>) {
    *WAKE.lock() = Some(w);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultApp {
    /// Nombre para mostrar («Fotos»).
    pub name: String,
    /// Ejecutable (su ícono sale de `shellicons::get(exe, "exe", px)`); None en las apps empaquetadas.
    pub exe: Option<String>,
    /// PNG del ícono de una app empaquetada (se carga con `thumbs::wic_load`).
    pub icon_png: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Todavía se está preguntando (el botón dice «Abrir», sin ícono).
    Pending,
    /// No hay programa, o Windows preguntaría: «Abrir con…».
    None,
    Some(DefaultApp),
}

/// Cuánto vale una respuesta.
const TTL: Duration = Duration::from_secs(60);

struct Entry {
    /// None = todavía no hay respuesta.
    answer: Option<Lookup>,
    at: Instant,
    asking: bool,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static C: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Tipos que se abren solos (programas, scripts, accesos directos): el botón dice «Abrir»,
/// sin «con». `ext` en minúsculas.
pub fn runs_itself(ext: &str) -> bool {
    matches!(ext, "exe" | "com" | "bat" | "cmd" | "ps1" | "msi" | "lnk" | "url" | "scr" | "appref-ms")
}

/// Programa predeterminado de la extensión `ext` (sin punto, en minúsculas). Caché por
/// extensión; la primera vez pregunta en un hilo, devuelve `Pending` y despierta al saberlo.
pub fn default_app(ext: &str) -> Lookup {
    if ext.is_empty() || runs_itself(ext) {
        return Lookup::None;
    }
    let (answer, go) = {
        let mut c = cache().lock();
        match c.get_mut(ext) {
            Some(e) => {
                let go = e.at.elapsed() >= TTL && !e.asking;
                e.asking |= go;
                (e.answer.clone().unwrap_or(Lookup::Pending), go)
            }
            None => {
                c.insert(ext.to_string(), Entry { answer: None, at: Instant::now(), asking: true });
                (Lookup::Pending, true)
            }
        }
    };
    if go {
        ask(ext.to_string());
    }
    answer
}

fn ask(ext: String) {
    let key = ext.clone();
    let spawned = std::thread::Builder::new().name("asociaciones".into()).spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let answer = query(&ext);
        let changed = {
            let mut c = cache().lock();
            let e = c.entry(ext).or_insert(Entry { answer: None, at: Instant::now(), asking: false });
            let changed = e.answer.as_ref() != Some(&answer);
            *e = Entry { answer: Some(answer), at: Instant::now(), asking: false };
            changed
        };
        if changed && let Some(w) = WAKE.lock().clone() {
            w();
        }
    });
    if spawned.is_err() {
        // Sin hilo: «Abrir con…», y se vuelve a intentar cuando venza.
        if let Some(e) = cache().lock().get_mut(&key) {
            e.answer.get_or_insert(Lookup::None);
            e.at = Instant::now();
            e.asking = false;
        }
    }
}

/// Una cadena de la asociación de `.ext` (None si no hay o si Windows preguntaría).
fn assoc_str(ext_w: &[u16], what: ASSOCSTR) -> Option<String> {
    unsafe {
        let flags = ASSOCF_INIT_IGNOREUNKNOWN | ASSOCF_NOTRUNCATE;
        let mut len = 0u32;
        // Primero el largo; algunos valores (rutas de apps empaquetadas) pasan de MAX_PATH.
        let _ = AssocQueryStringW(flags, what, PCWSTR(ext_w.as_ptr()), PCWSTR::null(), None, &mut len);
        if len == 0 {
            len = 1024;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let mut n = buf.len() as u32;
        AssocQueryStringW(flags, what, PCWSTR(ext_w.as_ptr()), PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut n).ok().ok()?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// `@{Paquete?ms-resource://…}` → texto o ruta, resuelta con la escala y el idioma del sistema.
fn indirect(s: &str) -> Option<String> {
    if !s.starts_with('@') {
        return Some(s.to_string());
    }
    let w = crate::index::walk::wide(s);
    let mut out = vec![0u16; 1024];
    unsafe { SHLoadIndirectString(PCWSTR(w.as_ptr()), &mut out, None).ok()? };
    let end = out.iter().position(|&c| c == 0).unwrap_or(out.len());
    let r = String::from_utf16_lossy(&out[..end]);
    (!r.is_empty()).then_some(r)
}

fn query(ext: &str) -> Lookup {
    lookup(&format!(".{ext}"))
}

/// Programa de una asociación: `.ext` o un ProgID.
fn lookup(assoc: &str) -> Lookup {
    let ext_w = crate::index::walk::wide(assoc);
    let exe = assoc_str(&ext_w, ASSOCSTR_EXECUTABLE);
    // Sin programa, Windows contesta con su diálogo de elegir (OpenWith.exe).
    if exe.as_deref().is_some_and(|e| e.to_lowercase().ends_with("openwith.exe")) {
        return Lookup::None;
    }
    let Some(name) = assoc_str(&ext_w, ASSOCSTR_FRIENDLYAPPNAME).and_then(|n| indirect(&n)) else {
        return Lookup::None;
    };
    // Apps empaquetadas (Fotos, Reproductor multimedia): su ícono es un PNG del paquete; el
    // ejecutable (si lo hay) está en WindowsApps y no da un ícono útil.
    let icon_png = assoc_str(&ext_w, ASSOCSTR_APPICONREFERENCE)
        .filter(|r| r.starts_with('@'))
        .and_then(|r| indirect(&r))
        .filter(|p| p.to_lowercase().ends_with(".png") && std::path::Path::new(p).is_file());
    let exe = if icon_png.is_some() { None } else { exe.filter(|e| std::path::Path::new(e).is_file()) };
    Lookup::Some(DefaultApp { name, exe, icon_png })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo que contesta este equipo (diagnóstico: `cargo test assoc -- --nocapture`).
    #[test]
    fn answers_common_types() {
        let _shell = crate::shell_test_lock();
        for ext in ["pdf", "jpg", "png", "txt", "mp3", "mp4", "docx", "zip", "html", "xyzzy"] {
            let r = query(ext);
            println!("{ext}: {r:?}");
            if let Lookup::Some(a) = &r {
                assert!(!a.name.is_empty());
                assert!(a.exe.is_some() || a.icon_png.is_some(), "{ext}: sin ícono");
            }
        }
        assert_eq!(query("xyzzy"), Lookup::None);
        assert_eq!(default_app("exe"), Lookup::None);
    }

    /// El ícono de cada respuesta se puede cargar (ejecutable o PNG del paquete).
    #[test]
    fn icons_load() {
        let _shell = crate::shell_test_lock();
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let mut apps: Vec<(String, DefaultApp)> = Vec::new();
        for ext in ["pdf", "jpg", "txt", "mp3", "mp4", "docx"] {
            if let Lookup::Some(a) = query(ext) {
                apps.push((ext.into(), a));
            }
        }
        // Una app empaquetada de un equipo de desarrollo: en otros (CI) no está y se salta.
        for prog in ["AppXafhv650gh677b2mpxsdax0p1y8xbnctk"] {
            if let Lookup::Some(a) = lookup(prog) {
                apps.push((prog.into(), a));
            }
        }
        for (what, a) in apps {
            let img = match (&a.exe, &a.icon_png) {
                (Some(exe), _) => crate::fsx::thumbs::system_icon(exe, 32),
                (_, Some(png)) => crate::fsx::thumbs::wic_load(png, 32),
                _ => None,
            };
            println!("{what}: {} → {:?}", a.name, img.as_ref().map(|i| (i.w, i.h)));
            assert!(img.is_some(), "{what}: el ícono no carga");
        }
    }

    /// Apps empaquetadas: los ProgID `AppX…` registrados para algunas extensiones.
    #[test]
    fn answers_packaged_apps() {
        let _shell = crate::shell_test_lock();
        let mut seen = 0;
        for ext in ["pdf", "jpg", "png", "mp3", "mp4", "txt", "html"] {
            let out = std::process::Command::new("reg").args(["query", &format!(r"HKCR\.{ext}\OpenWithProgids")]).output();
            let Ok(out) = out else { continue };
            for prog in String::from_utf8_lossy(&out.stdout).split_whitespace().filter(|w| w.starts_with("AppX")) {
                let r = lookup(prog);
                println!("{ext} {prog}: {r:?}");
                if let Lookup::Some(a) = r {
                    seen += 1;
                    assert!(!a.name.starts_with('@'), "nombre sin resolver: {}", a.name);
                }
            }
        }
        println!("apps empaquetadas: {seen}");
    }

    #[test]
    fn caches_and_wakes() {
        let _shell = crate::shell_test_lock();
        let (tx, rx) = crossbeam_channel::bounded::<()>(4);
        set_wake(Arc::new(move || {
            let _ = tx.try_send(());
        }));
        assert_eq!(default_app("txt"), Lookup::Pending);
        let _ = rx.recv_timeout(Duration::from_secs(10));
        let r = default_app("txt");
        println!("txt (caché): {r:?}");
        assert_ne!(r, Lookup::Pending);
    }
}
