//! Etiquetas de color, como en el Finder: siete colores que se le ponen a archivos y carpetas
//! (uno o varios) para encontrarlos después desde la barra lateral, estén donde estén.
//!
//! Se guardan en `etiquetas.json`, junto a la configuración, por ruta. Si Rayo renombra algo,
//! la etiqueta lo sigue; lo que ya no existe se descarta al listar.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;

/// Clave (para la búsqueda `etiqueta:rojo`) y color de cada etiqueta, en el orden del Finder.
pub const COLORS: [(&str, [u8; 3]); 7] = [
    ("rojo", [0xFF, 0x5F, 0x57]),
    ("naranja", [0xFF, 0x9F, 0x0A]),
    ("amarillo", [0xFF, 0xD6, 0x0A]),
    ("verde", [0x32, 0xD7, 0x4B]),
    ("azul", [0x0A, 0x84, 0xFF]),
    ("morado", [0xBF, 0x5A, 0xF2]),
    ("gris", [0x98, 0x98, 0x9D]),
];

/// Los mismos colores en inglés, para que `tag:red` también funcione.
const ENGLISH: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

/// Ruta en minúsculas → (ruta tal cual, colores como bits).
type Map = HashMap<String, (String, u8)>;

static STORE: Mutex<Option<Map>> = Mutex::new(None);

fn file() -> PathBuf {
    crate::instance::custom_data_dir()
        .unwrap_or_else(|| std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Rayo"))
        .join("etiquetas.json")
}

fn with<R>(f: impl FnOnce(&mut Map) -> R) -> R {
    let mut g = STORE.lock();
    let map = g.get_or_insert_with(|| {
        let Ok(bytes) = std::fs::read(file()) else { return Map::default() };
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        match serde_json::from_slice::<Vec<(String, u8)>>(bytes) {
            Ok(v) => v.into_iter().filter(|(_, m)| *m != 0).map(|(p, m)| (p.to_lowercase(), (p, m))).collect(),
            Err(e) => {
                // Ilegible (¿un formato de otra versión?): se guarda una copia antes de que el
                // próximo cambio lo reemplace, para no perder las etiquetas.
                let _ = std::fs::copy(file(), file().with_extension("json.bak"));
                crate::diag::log(&format!("etiquetas: no se pudo leer etiquetas.json ({e}); copia en etiquetas.json.bak"));
                Map::default()
            }
        }
    });
    f(map)
}

fn save(map: &Map) {
    let mut v: Vec<(&String, u8)> = map.values().map(|(p, m)| (p, *m)).collect();
    v.sort();
    if let Ok(json) = serde_json::to_vec(&v) {
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

/// Los colores de un archivo o carpeta (bit `i` = `COLORS[i]`).
pub fn get(path: &str) -> u8 {
    with(|m| m.get(&path.to_lowercase()).map_or(0, |e| e.1))
}

/// Pone el color a todos; si ya lo tenían todos, se lo quita (como el Finder).
pub fn toggle(paths: &[String], color: usize) {
    let bit = 1u8 << color;
    with(|m| {
        let all = paths.iter().all(|p| m.get(&p.to_lowercase()).is_some_and(|e| e.1 & bit != 0));
        for p in paths {
            let e = m.entry(p.to_lowercase()).or_insert_with(|| (p.clone(), 0));
            if all { e.1 &= !bit } else { e.1 |= bit }
        }
        m.retain(|_, e| e.1 != 0);
        save(m);
    });
}

/// Quita todas las etiquetas.
pub fn clear(paths: &[String]) {
    with(|m| {
        for p in paths {
            m.remove(&p.to_lowercase());
        }
        save(m);
    });
}

/// Algo cambió de nombre o de lugar: las etiquetas lo siguen (también lo de adentro de una carpeta).
pub fn moved(from: &str, to: &str) {
    with(|m| {
        let f = from.to_lowercase();
        let inside = format!("{}\\", f.trim_end_matches('\\'));
        let keys: Vec<String> = m.keys().filter(|k| **k == f || k.starts_with(&inside)).cloned().collect();
        if keys.is_empty() {
            return;
        }
        for k in keys {
            if let Some((p, mask)) = m.remove(&k) {
                let np = format!("{to}{}", &p[from.len().min(p.len())..]);
                m.insert(np.to_lowercase(), (np, mask));
            }
        }
        save(m);
    });
}

/// Todo lo que tiene ese color (lo que ya no existe se olvida).
pub fn paths(color: usize) -> Vec<String> {
    let bit = 1u8 << color;
    with(|m| {
        let before = m.len();
        m.retain(|_, e| std::path::Path::new(&super::long_path(&e.0)).exists());
        if m.len() != before {
            save(m);
        }
        let mut v: Vec<String> = m.values().filter(|e| e.1 & bit != 0).map(|e| e.0.clone()).collect();
        v.sort_by_key(|p| p.to_lowercase());
        v
    })
}

/// Cuántos elementos tiene cada color.
pub fn counts() -> [usize; 7] {
    with(|m| {
        let mut c = [0; 7];
        for e in m.values() {
            for (i, n) in c.iter_mut().enumerate() {
                if e.1 & (1 << i) != 0 {
                    *n += 1;
                }
            }
        }
        c
    })
}

/// `etiqueta:rojo` o `tag:red` → el color pedido.
pub fn query_color(text: &str) -> Option<usize> {
    let t = text.trim().to_lowercase();
    let (key, val) = t.split_once(':')?;
    if !matches!(key, "etiqueta" | "tag") {
        return None;
    }
    let val = val.trim();
    COLORS.iter().position(|c| c.0 == val).or_else(|| ENGLISH.iter().position(|c| *c == val))
}

/// La búsqueda que muestra todo lo de un color.
pub fn query_for(color: usize) -> String {
    format!("etiqueta:{}", COLORS[color].0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_names() {
        assert_eq!(query_color("etiqueta:rojo"), Some(0));
        assert_eq!(query_color(" Tag:Blue "), Some(4));
        assert_eq!(query_color("etiqueta:fucsia"), None);
        assert_eq!(query_color("rojo"), None);
        assert_eq!(query_for(6), "etiqueta:gris");
    }
}
