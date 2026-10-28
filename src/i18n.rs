//! Idiomas. Cada texto de la interfaz tiene una clave (`menu.copiar_ruta`) y su traducción
//! sale de `idiomas/<código>.csv`, con las columnas `clave,texto,original (es),nota`.
//!
//! - Español e inglés van dentro del ejecutable.
//! - Cualquier otro idioma (o una corrección) se agrega soltando un CSV en la carpeta
//!   `idiomas` junto a `rayo.exe` o en `%APPDATA%\Rayo\idiomas`, sin recompilar.
//! - Si a un idioma le falta una frase, sale en inglés (y si tampoco está, en español).
//!
//! En el código: `t!("clave")` devuelve el texto (`&'static str`) y
//! `tf!("clave", n = 3, nombre = x)` lo devuelve con `{n}` y `{nombre}` reemplazados.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub const ES: &str = include_str!("../idiomas/es.csv");
pub const EN: &str = include_str!("../idiomas/en.csv");

/// Un idioma cargado: clave → texto.
#[derive(Default, Clone)]
pub struct Table {
    pub code: String,
    pub map: HashMap<String, &'static str>,
}

impl Table {
    fn name(&self) -> String {
        self.map.get("idioma.nombre").map(|s| s.to_string()).unwrap_or_else(|| self.code.clone())
    }
}

struct State {
    cur: Table,
    en: Table,
    es: Table,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);
/// Sube cada vez que cambia el idioma (para lo que guarde textos ya traducidos).
static GENERATION: AtomicU32 = AtomicU32::new(0);

pub fn generation() -> u32 {
    GENERATION.load(Ordering::Relaxed)
}

/// El texto de `key` en el idioma actual (o en inglés, o en español, o la clave misma).
pub fn tr(key: &'static str) -> &'static str {
    let g = STATE.read();
    let Some(s) = g.as_ref() else {
        drop(g);
        init(None);
        return tr(key);
    };
    s.cur.map.get(key).or_else(|| s.en.map.get(key)).or_else(|| s.es.map.get(key)).copied().unwrap_or(key)
}

/// Igual que `tr`, pero para claves armadas en tiempo de ejecución (las de los comandos).
pub fn tr_dyn(key: &str) -> Option<&'static str> {
    let g = STATE.read();
    let s = g.as_ref()?;
    s.cur.map.get(key).or_else(|| s.en.map.get(key)).or_else(|| s.es.map.get(key)).copied()
}

/// Reemplaza `{nombre}` por su valor.
pub fn fill(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) => {
                let name = &after[..j];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => {
                        use std::fmt::Write;
                        let _ = write!(out, "{v}");
                    }
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- carga

/// Lee un CSV de idioma (coma o punto y coma, como lo guarde Excel). Devuelve (clave, texto).
pub fn parse_csv(text: &str) -> Vec<(String, String)> {
    let text = text.trim_start_matches('\u{feff}');
    let first_line = text.lines().next().unwrap_or("");
    let sep = if first_line.matches(';').count() > first_line.matches(',').count() { ';' } else { ',' };
    let rows = csv_rows(text, sep);
    let Some(header) = rows.first() else { return Vec::new() };
    let col = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h.trim().eq_ignore_ascii_case(n)));
    let (Some(k), Some(v)) = (col(&["clave", "key"]), col(&["texto", "text", "traducción", "traduccion", "translation"])) else {
        return Vec::new();
    };
    rows.iter()
        .skip(1)
        .filter_map(|r| {
            let key = r.get(k)?.trim();
            let val = r.get(v)?;
            (!key.is_empty() && !key.starts_with('#') && !val.is_empty()).then(|| (key.to_string(), val.replace("\r\n", "\n")))
        })
        .collect()
}

fn csv_rows(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' && cell.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|c| !c.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            cell.push(c);
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

fn table(code: &str, sources: &[String]) -> Table {
    let mut map = HashMap::new();
    for src in sources {
        for (k, v) in parse_csv(src) {
            // Se guardan para siempre: son pocos KB y así `t!` devuelve `&'static str`.
            map.insert(k, &*Box::leak(v.into_boxed_str()));
        }
    }
    Table { code: code.to_string(), map }
}

/// Carpetas donde buscar idiomas extra: junto al ejecutable y en %APPDATA%\Rayo.
pub fn folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("idiomas"))) {
        v.push(dir);
    }
    if let Some(app) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(app).join("Rayo").join("idiomas"));
    }
    v
}

/// Los CSV de un idioma: el incluido (si es español o inglés) y los de las carpetas, en orden
/// (lo de las carpetas pisa a lo incluido, para poder corregir una traducción sin recompilar).
fn sources(code: &str) -> Vec<String> {
    let mut v = Vec::new();
    match code {
        "es" => v.push(ES.to_string()),
        "en" => v.push(EN.to_string()),
        _ => {}
    }
    for dir in folders() {
        if let Ok(s) = std::fs::read_to_string(dir.join(format!("{code}.csv"))) {
            v.push(s);
        }
    }
    v
}

/// Carga el idioma `code` (o el de Windows, si es None). Si no existe, inglés.
pub fn init(code: Option<&str>) {
    let wanted = code.map(str::to_string).unwrap_or_else(windows_language);
    let es = table("es", &[ES.to_string()]);
    let en = table("en", &sources("en"));
    let cur = match wanted.as_str() {
        "es" => table("es", &sources("es")),
        "en" => en.clone(),
        other => {
            let t = table(other, &sources(other));
            if t.map.is_empty() { en.clone() } else { t }
        }
    };
    *STATE.write() = Some(State { cur, en, es });
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Código del idioma en uso ("es", "en", "pt"…).
pub fn current() -> String {
    STATE.read().as_ref().map(|s| s.cur.code.clone()).unwrap_or_else(|| "es".into())
}

/// El idioma de Windows del usuario ("es-MX" → "es").
pub fn windows_language() -> String {
    use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let mut count = 0u32;
    let mut len = 0u32;
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_ok() && len > 0 {
            let mut buf = vec![0u16; len as usize];
            if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len).is_ok() {
                let first: String = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]);
                if let Some(code) = first.split('-').next().filter(|c| !c.is_empty()) {
                    return code.to_lowercase();
                }
            }
        }
    }
    "en".into()
}

/// Un idioma disponible: código, nombre (en su idioma) y qué parte está traducida (0–100).
pub struct Available {
    pub code: String,
    pub name: String,
    pub percent: u32,
}

/// Español, inglés y los CSV que haya en las carpetas de idiomas.
pub fn available() -> Vec<Available> {
    let es = table("es", &[ES.to_string()]);
    let total = es.map.len().max(1);
    let mut codes = vec!["es".to_string(), "en".to_string()];
    for dir in folders() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("csv")) {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()).filter(|s| !s.starts_with('_')) {
                        let code = stem.to_lowercase();
                        if !codes.contains(&code) {
                            codes.push(code);
                        }
                    }
                }
            }
        }
    }
    codes
        .into_iter()
        .map(|code| {
            let t = table(&code, &sources(&code));
            let have = es.map.keys().filter(|k| t.map.contains_key(*k)).count();
            Available { name: t.name(), percent: (have * 100 / total) as u32, code }
        })
        .collect()
}

/// Crea la carpeta de idiomas del usuario (%APPDATA%\Rayo\idiomas) con una plantilla para
/// traducir: cada clave con el texto en español, en inglés y la nota, y la columna «texto»
/// vacía. Devuelve la carpeta.
pub fn write_template() -> Result<PathBuf, String> {
    let dir = folders().pop().ok_or_else(|| "APPDATA".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let en: HashMap<String, String> = parse_csv(EN).into_iter().collect();
    let rows = csv_rows(ES.trim_start_matches('\u{feff}'), ',');
    let esc = |s: &str| if s.contains([',', '"', '\n', '\r', ';']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::from("\u{feff}clave,texto,original (es),english,nota\r\n");
    for r in rows.iter().skip(1) {
        let (Some(k), Some(es)) = (r.first(), r.get(1)) else { continue };
        let note = r.get(3).map(String::as_str).unwrap_or("");
        // La columna «texto» queda vacía: es la que se traduce.
        out.push_str(&format!("{},,{},{},{}\r\n", esc(k), esc(es), esc(en.get(k).map(String::as_str).unwrap_or("")), esc(note)));
    }
    std::fs::write(dir.join("_plantilla.csv"), out).map_err(|e| e.to_string())?;
    Ok(dir)
}

// ---------------------------------------------------------------- revisión de traducciones

/// Los `{nombres}` de un texto, ordenados.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else { break };
        let name = &after[..j];
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            v.push(name.to_string());
        }
        rest = &after[j + 1..];
    }
    v.sort();
    v
}

/// Lo que le falta o le sobra a una traducción (comparada con el español).
pub struct Check {
    pub total: usize,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    /// Clave, variables del original, variables de la traducción.
    pub bad: Vec<(String, Vec<String>, Vec<String>)>,
}

/// Compara un CSV de idioma contra el español: qué falta, qué sobra y qué `{variables}` no coinciden.
pub fn check(text: &str) -> Check {
    let es: HashMap<String, String> = parse_csv(ES).into_iter().collect();
    let other: HashMap<String, String> = parse_csv(text).into_iter().collect();
    let mut missing: Vec<&String> = es.keys().filter(|k| !other.contains_key(*k)).collect();
    // El nombre del idioma no cuenta como texto sin traducir si falta (se usa el código).
    let mut extra: Vec<&String> = other.keys().filter(|k| !es.contains_key(*k)).collect();
    let mut bad: Vec<(&String, Vec<String>, Vec<String>)> = other
        .iter()
        .filter_map(|(k, v)| {
            let want = placeholders(es.get(k)?);
            let got = placeholders(v);
            (want != got).then(|| (k, want, got))
        })
        .collect();
    missing.sort();
    extra.sort();
    bad.sort();
    Check {
        total: es.len(),
        missing: missing.into_iter().cloned().collect(),
        extra: extra.into_iter().cloned().collect(),
        bad: bad.into_iter().map(|(k, a, b)| (k.clone(), a, b)).collect(),
    }
}

/// El resultado de `check`, para leer.
pub fn check_csv(code: &str, text: &str) -> String {
    use std::fmt::Write;
    let c = check(text);
    let es: HashMap<String, String> = parse_csv(ES).into_iter().collect();
    let done = c.total - c.missing.len();
    let mut out = String::new();
    let _ = writeln!(out, "{}", tf!("revision.resumen", idioma = code, hechos = done, total = c.total, porcentaje = done * 100 / c.total.max(1)));
    if !c.bad.is_empty() {
        let _ = writeln!(out, "\n{}", t!("revision.variables_distintas"));
        for (k, want, got) in &c.bad {
            let _ = writeln!(out, "  {k}: {}", tf!("revision.variables_detalle", original = format!("{want:?}"), traduccion = format!("{got:?}")));
        }
    }
    if !c.missing.is_empty() {
        let _ = writeln!(out, "\n{}", t!("revision.sin_traducir"));
        for k in &c.missing {
            let _ = writeln!(out, "  {k}  ·  {}", es.get(k).map(|s| s.replace('\n', " ")).unwrap_or_default());
        }
    }
    if !c.extra.is_empty() {
        let _ = writeln!(out, "\n{}", t!("revision.sobran"));
        for k in &c.extra {
            let _ = writeln!(out, "  {k}");
        }
    }
    out
}

/// Revisa todos los CSV de las carpetas de idiomas (y el inglés incluido).
pub fn check_all() -> String {
    let mut out = check_csv("en", EN);
    for dir in folders() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("csv")) {
                    let code = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_lowercase();
                    if code == "es" || code.starts_with('_') {
                        continue;
                    }
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        out.push_str(&format!("\n——— {} ———\n", p.display()));
                        out.push_str(&check_csv(&code, &s));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_quotes_newlines_and_semicolons() {
        let t = "\u{feff}clave;texto;original (es);nota\r\na.b;\"Hola; \"\"tú\"\"\nadiós\";x;\r\nc.d;Chau;;\r\n";
        let v = parse_csv(t);
        assert_eq!(v, vec![("a.b".to_string(), "Hola; \"tú\"\nadiós".to_string()), ("c.d".to_string(), "Chau".to_string())]);
    }

    #[test]
    fn fill_replaces_names() {
        assert_eq!(fill("{n} de {total} · {x}", &[("n", &3), ("total", &"5 MB")]), "3 de 5 MB · {x}");
    }

    /// Cada clave usada en el código existe en español y en inglés, con las mismas variables.
    #[test]
    fn every_key_is_translated() {
        let es: HashMap<String, String> = parse_csv(ES).into_iter().collect();
        let en: HashMap<String, String> = parse_csv(EN).into_iter().collect();
        let mut problems = Vec::new();
        // Las claves de t!("…"), tf!("…") y tp!(cantidad, "…") (esta, con .uno y .varios).
        let re_key = |src: &str| -> Vec<String> {
            let mut v = Vec::new();
            let b = src.as_bytes();
            let mut i = 0;
            while let Some(off) = src[i..].find("!(") {
                let at = i + off;
                i = at + 2;
                let start = (0..at).rev().take_while(|&k| b[k].is_ascii_alphanumeric() || b[k] == b'_').last().unwrap_or(at);
                let name = &src[start..at];
                let rest = &src[at + 2..];
                let lit = |s: &str| s.strip_prefix('"').and_then(|s| s.find('"').map(|j| s[..j].to_string()));
                match name {
                    "t" | "tf" => v.extend(lit(rest)),
                    "tp" => {
                        if let Some(k) = rest.find(", \"").and_then(|c| lit(&rest[c + 2..])) {
                            v.push(format!("{k}.uno"));
                            v.push(format!("{k}.varios"));
                        }
                    }
                    _ => {}
                }
            }
            v
        };
        fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        for f in files {
            if f.ends_with("i18n.rs") || f.ends_with("lib.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&f).unwrap();
            for k in re_key(&src) {
                match (es.get(&k), en.get(&k)) {
                    (None, _) => problems.push(format!("{}: «{k}» no está en es.csv", f.display())),
                    (Some(_), None) => problems.push(format!("{}: «{k}» no está en en.csv", f.display())),
                    (Some(a), Some(b)) if placeholders(a) != placeholders(b) => problems.push(format!("«{k}»: variables distintas en en.csv")),
                    _ => {}
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn english_is_complete() {
        let c = check(EN);
        assert!(c.missing.is_empty() && c.bad.is_empty(), "{}", check_csv("en", EN));
    }
}
