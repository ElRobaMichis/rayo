//! Renombrado masivo: plan con vista previa y aplicación segura.
//!
//! La plantilla arma el nombre (sin extensión) con estas piezas:
//! `{nombre}` nombre original (con buscar/reemplazar aplicado), `{n}` contador,
//! `{fecha}` fecha de modificación, `{exif}` fecha en que se tomó la foto,
//! `{carpeta}` nombre de la carpeta y `{ext}` extensión original. Cada pieza tiene su nombre
//! en el idioma de Rayo (`{name}`, `{date}`…) y se aceptan siempre las de español e inglés.

use super::{ext_of, join, long_path};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
    Title,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub template: String,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub case: Case,
    pub start: u32,
    pub pad: usize,
    /// Extensión nueva (sin punto). Vacía = conservar la original.
    pub new_ext: String,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { template: token("nombre"), find: String::new(), replace: String::new(), regex: false, case: Case::Keep, start: 1, pad: 2, new_ext: String::new() }
    }
}

/// Una pieza de la plantilla, como se escribe en el idioma actual: `token("nombre")` → "{name}" en inglés.
pub fn token(piece: &str) -> String {
    format!("{{{}}}", token_name(piece))
}

fn token_name(piece: &str) -> &'static str {
    match piece {
        "nombre" => t!("plantilla.nombre"),
        "n" => t!("plantilla.n"),
        "fecha" => t!("plantilla.fecha"),
        "exif" => t!("plantilla.exif"),
        "carpeta" => t!("plantilla.carpeta"),
        _ => t!("plantilla.ext"),
    }
}

/// Los nombres que se aceptan para una pieza: español, inglés y el del idioma actual.
fn token_names(piece: &str) -> Vec<&str> {
    let english = match piece {
        "nombre" => "name",
        "fecha" => "date",
        "carpeta" => "folder",
        other => other,
    };
    let mut v = vec![piece, english, token_name(piece)];
    v.dedup();
    v
}

/// Si la plantilla usa una pieza (con cualquiera de sus nombres).
pub fn uses(template: &str, piece: &str) -> bool {
    token_names(piece).iter().any(|n| template.contains(&format!("{{{n}}}")))
}

/// Datos de cada archivo que la plantilla puede usar.
#[derive(Debug, Clone, Default)]
pub struct Source {
    pub name: String,
    pub mtime: u32,
    /// Fecha de la foto ("2024-07-26 18.33.12"), si se conoce.
    pub exif: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub old: String,
    pub new: String,
    /// Motivo por el que no se puede aplicar (nombre repetido, inválido…).
    pub problem: Option<String>,
}

fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

fn apply_case(s: &str, c: Case) -> String {
    match c {
        Case::Keep => s.to_string(),
        Case::Lower => s.to_lowercase(),
        Case::Upper => s.to_uppercase(),
        Case::Title => {
            let mut out = String::with_capacity(s.len());
            let mut start = true;
            for ch in s.chars() {
                if start {
                    out.extend(ch.to_uppercase());
                } else {
                    out.extend(ch.to_lowercase());
                }
                start = !ch.is_alphanumeric();
            }
            out
        }
    }
}

const INVALID: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Arma el plan sin tocar el disco. `existing` son los nombres que ya hay en la
/// carpeta (para detectar choques con archivos que no se renombran).
pub fn plan(dir_name: &str, files: &[Source], rules: &Rules, existing: &HashSet<String>) -> Result<Vec<Planned>, String> {
    let re = if rules.regex && !rules.find.is_empty() {
        Some(regex::RegexBuilder::new(&rules.find).case_insensitive(true).build().map_err(|e| tf!("renombrar.expresion_invalida", error = e))?)
    } else {
        None
    };
    let mut out = Vec::with_capacity(files.len());
    for (k, f) in files.iter().enumerate() {
        let (stem, ext) = split_name(&f.name);
        let mut base = stem.to_string();
        if !rules.find.is_empty() {
            base = match &re {
                Some(re) => re.replace_all(&base, rules.replace.as_str()).into_owned(),
                None => replace_ci(&base, &rules.find, &rules.replace),
            };
        }
        let n = format!("{:0width$}", rules.start as usize + k, width = rules.pad);
        let date = super::fmt_date(f.mtime).get(..10).unwrap_or("").to_string();
        let exif = f.exif.clone().unwrap_or_else(|| super::fmt_date(f.mtime).replace(':', "."));
        let mut new_stem = rules.template.clone();
        for (piece, value) in [("nombre", base.as_str()), ("n", n.as_str()), ("fecha", date.as_str()), ("exif", exif.as_str()), ("carpeta", dir_name), ("ext", ext)] {
            for name in token_names(piece) {
                new_stem = new_stem.replace(&format!("{{{name}}}"), value);
            }
        }
        new_stem = apply_case(new_stem.trim(), rules.case);
        let new_ext = if rules.new_ext.trim().is_empty() { ext.to_string() } else { rules.new_ext.trim().trim_start_matches('.').to_string() };
        let new = if new_ext.is_empty() { new_stem.clone() } else { format!("{new_stem}.{new_ext}") };
        out.push(Planned { old: f.name.clone(), new, problem: None });
    }
    // Problemas: vacío, caracteres inválidos, repetidos, choques con archivos que se quedan.
    let renamed: HashSet<String> = out.iter().map(|p| p.old.to_lowercase()).collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &out {
        *seen.entry(p.new.to_lowercase()).or_default() += 1;
    }
    for p in &mut out {
        let l = p.new.to_lowercase();
        p.problem = if p.new.trim().is_empty() || p.new.starts_with('.') && split_name(&p.new).0.is_empty() {
            Some(t!("renombrar.nombre_quedaria_vacio").into())
        } else if p.new.contains(INVALID) {
            Some(t!("renombrar.tiene_caracteres_no_permitidos").into())
        } else if p.new.ends_with(' ') || p.new.ends_with('.') {
            Some(t!("renombrar.no_puede_terminar_espacio_punto").into())
        } else if seen[&l] > 1 {
            Some(t!("renombrar.dos_archivos_quedarian_mismo_nombre").into())
        } else if existing.contains(&l) && !renamed.contains(&l) {
            Some(t!("renombrar.ya_existe_archivo_ese_nombre").into())
        } else {
            None
        };
    }
    let _ = ext_of;
    Ok(out)
}

/// Reemplazo sin distinguir mayúsculas.
fn replace_ci(s: &str, find: &str, rep: &str) -> String {
    let lower = s.to_lowercase();
    let f = find.to_lowercase();
    if f.is_empty() || lower.len() != s.len() {
        return s.replace(find, rep);
    }
    let mut out = String::new();
    let mut last = 0;
    for (i, _) in lower.match_indices(&f) {
        out.push_str(&s[last..i]);
        out.push_str(rep);
        last = i + f.len();
    }
    out.push_str(&s[last..]);
    out
}

/// Aplica el plan en dos pasos (primero a nombres temporales) para que
/// intercambios como a→b y b→a no choquen. Devuelve lo renombrado (viejo, nuevo) y los errores.
pub fn apply(dir: &str, plan: &[Planned]) -> (Vec<(String, String)>, Vec<String>) {
    let todo: Vec<&Planned> = plan.iter().filter(|p| p.problem.is_none() && p.old != p.new).collect();
    let mut errors = Vec::new();
    let mut done = Vec::new();
    let mut staged = Vec::new();
    let tag = std::process::id();
    for (k, p) in todo.iter().enumerate() {
        let tmp = format!("~rayo-{tag}-{k}.tmp");
        match std::fs::rename(long_path(&join(dir, &p.old)), long_path(&join(dir, &tmp))) {
            Ok(()) => staged.push((tmp, *p)),
            Err(e) => errors.push(format!("{}: {e}", p.old)),
        }
    }
    for (tmp, p) in staged {
        // Nunca pisar un archivo que no es parte del plan (pudo aparecer después de la vista previa).
        let target = join(dir, &p.new);
        if std::fs::symlink_metadata(long_path(&target)).is_ok() {
            let _ = std::fs::rename(long_path(&join(dir, &tmp)), long_path(&join(dir, &p.old)));
            errors.push(tf!("renombrar.ya_existe", viejo = p.old, nuevo = p.new));
            continue;
        }
        match std::fs::rename(long_path(&join(dir, &tmp)), long_path(&target)) {
            Ok(()) => done.push((p.old.clone(), p.new.clone())),
            Err(e) => {
                // Si falla, devolverle su nombre original.
                let _ = std::fs::rename(long_path(&join(dir, &tmp)), long_path(&join(dir, &p.old)));
                errors.push(format!("{}: {e}", p.old));
            }
        }
    }
    (done, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(names: &[&str]) -> Vec<Source> {
        names.iter().map(|n| Source { name: n.to_string(), mtime: 1_700_000_000, exif: None }).collect()
    }

    #[test]
    fn plans_and_detects_problems() {
        let files = src(&["IMG_001.JPG", "IMG_002.JPG", "notas.txt"]);
        let r = Rules { template: "Viaje {n}".into(), case: Case::Keep, ..Default::default() };
        let p = plan("Fotos", &files, &r, &HashSet::new()).unwrap();
        assert_eq!(p[0].new, "Viaje 01.JPG");
        assert_eq!(p[2].new, "Viaje 03.txt");
        assert!(p.iter().all(|x| x.problem.is_none()));

        let r = Rules { find: "img_".into(), replace: "foto-".into(), new_ext: "jpg".into(), case: Case::Lower, ..Default::default() };
        let p = plan("Fotos", &files[..2], &r, &HashSet::new()).unwrap();
        assert_eq!(p[0].new, "foto-001.jpg");

        let r = Rules { regex: true, find: r"(\d+)$".into(), replace: "[$1]".into(), ..Default::default() };
        let p = plan("Fotos", &files[..1], &r, &HashSet::new()).unwrap();
        assert_eq!(p[0].new, "IMG_[001].JPG");

        // Todos con el mismo nombre: conflicto.
        let r = Rules { template: "igual".into(), ..Default::default() };
        let p = plan("Fotos", &files[..2], &r, &HashSet::new()).unwrap();
        assert!(p.iter().all(|x| x.problem.is_some()));

        // Choque con un archivo que se queda.
        let existing: HashSet<String> = ["otro.txt".to_string()].into();
        let r = Rules { template: "otro".into(), ..Default::default() };
        let p = plan("x", &src(&["a.txt"]), &r, &existing).unwrap();
        assert!(p[0].problem.is_some());
    }

    #[test]
    fn swaps_names_safely() {
        let d = std::env::temp_dir().join(format!("rayo-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.txt"), "A").unwrap();
        std::fs::write(d.join("b.txt"), "B").unwrap();
        let dir = d.to_string_lossy().into_owned();
        let plan = vec![
            Planned { old: "a.txt".into(), new: "b.txt".into(), problem: None },
            Planned { old: "b.txt".into(), new: "a.txt".into(), problem: None },
        ];
        let (done, errors) = apply(&dir, &plan);
        assert!(errors.is_empty());
        assert_eq!(done.len(), 2);
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "A");
        let _ = std::fs::remove_dir_all(&d);
    }
}
