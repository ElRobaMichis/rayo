//! Colecciones: listas con nombre de archivos y carpetas de cualquier lugar, como carpetas
//! virtuales (en Directory Opus, "File Collections"). Agregar algo a una colección no lo mueve ni
//! lo copia; la colección solo recuerda dónde está. Se abren en una pestaña, como las etiquetas.
//!
//! Se guardan en `colecciones.json`, junto a la configuración.

use parking_lot::Mutex;
use std::path::PathBuf;

type Store = Vec<(String, Vec<String>)>;

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn file() -> PathBuf {
    crate::instance::custom_data_dir()
        .unwrap_or_else(|| std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Rayo"))
        .join("colecciones.json")
}

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut g = STORE.lock();
    let store = g.get_or_insert_with(|| {
        let Ok(bytes) = std::fs::read(file()) else { return Store::default() };
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        match serde_json::from_slice::<Store>(bytes) {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::copy(file(), file().with_extension("json.bak"));
                crate::diag::log(&format!("colecciones: no se pudo leer colecciones.json ({e}); copia en colecciones.json.bak"));
                Store::default()
            }
        }
    });
    f(store)
}

fn save(store: &Store) {
    if let Ok(json) = serde_json::to_vec(store) {
        let f = file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = f.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &f);
        }
    }
}

/// Las colecciones y cuántos elementos tiene cada una.
pub fn list() -> Vec<(String, usize)> {
    with(|s| s.iter().map(|(n, v)| (n.clone(), v.len())).collect())
}

/// Lo que hay en una colección (lo que ya no existe se olvida).
pub fn paths(name: &str) -> Vec<String> {
    with(|s| {
        let Some((_, v)) = s.iter_mut().find(|(n, _)| n == name) else { return Vec::new() };
        let before = v.len();
        v.retain(|p| std::path::Path::new(&super::long_path(p)).exists());
        let out = v.clone();
        if v.len() != before {
            save(s);
        }
        out
    })
}

/// Agrega a una colección (la crea si no existe).
pub fn add(name: &str, paths: &[String]) {
    with(|s| {
        let i = match s.iter().position(|(n, _)| n == name) {
            Some(i) => i,
            None => {
                s.push((name.to_string(), Vec::new()));
                s.len() - 1
            }
        };
        for p in paths {
            if !s[i].1.iter().any(|x| x.eq_ignore_ascii_case(p)) {
                s[i].1.push(p.clone());
            }
        }
        save(s);
    });
}

pub fn remove(name: &str, paths: &[String]) {
    with(|s| {
        if let Some((_, v)) = s.iter_mut().find(|(n, _)| n == name) {
            v.retain(|x| !paths.iter().any(|p| p.eq_ignore_ascii_case(x)));
        }
        save(s);
    });
}

pub fn delete(name: &str) {
    with(|s| {
        s.retain(|(n, _)| n != name);
        save(s);
    });
}

/// ¿Están todos estos en la colección?
pub fn contains_all(name: &str, paths: &[String]) -> bool {
    with(|s| s.iter().find(|(n, _)| n == name).is_some_and(|(_, v)| paths.iter().all(|p| v.iter().any(|x| x.eq_ignore_ascii_case(p)))))
}

/// Algo cambió de nombre o de lugar (dentro de Rayo): las colecciones lo siguen.
pub fn moved(from: &str, to: &str) {
    with(|s| {
        let f = from.to_lowercase();
        let inside = format!("{}\\", f.trim_end_matches('\\'));
        let mut changed = false;
        for (_, v) in s.iter_mut() {
            for p in v.iter_mut() {
                let l = p.to_lowercase();
                if l == f || l.starts_with(&inside) {
                    *p = format!("{to}{}", &p[from.len().min(p.len())..]);
                    changed = true;
                }
            }
        }
        if changed {
            save(s);
        }
    });
}

/// `coleccion:Viaje` o `collection:Viaje` → el nombre de la colección.
pub fn query_name(text: &str) -> Option<String> {
    let t = text.trim();
    let (key, val) = t.split_once(':')?;
    let key = key.to_lowercase();
    if !matches!(key.as_str(), "coleccion" | "colección" | "collection") {
        return None;
    }
    let val = val.trim();
    with(|s| s.iter().find(|(n, _)| n.eq_ignore_ascii_case(val)).map(|(n, _)| n.clone()))
}

pub fn query_for(name: &str) -> String {
    format!("coleccion:{name}")
}
