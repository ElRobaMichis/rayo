//! "Nuevo ▸": los tipos de archivo que se pueden crear vacíos, los mismos que
//! ofrece el Explorador (cada programa los registra en `ShellNew`).

use super::{join, long_path};
use crate::index::walk::wide;
use std::sync::OnceLock;
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

#[derive(Clone, Debug)]
pub struct NewItem {
    /// Extensión con punto (".txt").
    pub ext: String,
    /// Nombre del tipo ("Documento de texto").
    pub name: String,
    kind: Template,
}

#[derive(Clone, Debug)]
enum Template {
    Empty,
    Data(Vec<u8>),
    File(String),
}

static ITEMS: OnceLock<Vec<NewItem>> = OnceLock::new();

/// Los tipos (se calculan una sola vez, en segundo plano al arrancar).
pub fn items() -> Option<&'static [NewItem]> {
    ITEMS.get().map(|v| v.as_slice())
}

pub fn preload() {
    std::thread::spawn(|| {
        let _ = ITEMS.get_or_init(scan);
    });
}

fn open_key(parent: HKEY, sub: &str) -> Option<HKEY> {
    let w = wide(sub);
    let mut k = HKEY::default();
    unsafe { RegOpenKeyExW(parent, PCWSTR(w.as_ptr()), Some(0), KEY_READ, &mut k).is_ok().then_some(k) }
}

fn close(k: HKEY) {
    unsafe {
        let _ = RegCloseKey(k);
    }
}

/// Valor de un registro (texto o binario). `name` vacío = el valor predeterminado.
fn value(k: HKEY, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let w = wide(name);
    let mut ty = REG_VALUE_TYPE::default();
    let mut len = 0u32;
    unsafe {
        if RegQueryValueExW(k, PCWSTR(w.as_ptr()), None, Some(&mut ty), None, Some(&mut len)).is_err() {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        RegQueryValueExW(k, PCWSTR(w.as_ptr()), None, Some(&mut ty), Some(buf.as_mut_ptr()), Some(&mut len)).ok().ok()?;
        buf.truncate(len as usize);
        Some((ty, buf))
    }
}

fn text(bytes: &[u8]) -> String {
    let u: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&u)
}

fn template_in(k: HKEY) -> Option<Template> {
    if value(k, "NullFile").is_some() {
        return Some(Template::Empty);
    }
    if let Some((_, d)) = value(k, "Data") {
        return Some(Template::Data(d));
    }
    if let Some((_, f)) = value(k, "FileName") {
        return Some(Template::File(text(&f)));
    }
    None
}

/// Nombre de los tipos comunes en el idioma de Rayo (Windows puede estar en otro idioma).
fn known_name(ext: &str) -> Option<&'static str> {
    Some(match ext {
        ".txt" => t!("nuevo.documento_texto"),
        ".md" => t!("nuevo.documento_markdown"),
        ".rtf" => t!("nuevo.documento_texto_enriquecido"),
        ".docx" | ".doc" => t!("nuevo.documento_word"),
        ".xlsx" | ".xls" => t!("nuevo.hoja_calculo_excel"),
        ".pptx" | ".ppt" => t!("nuevo.presentacion_powerpoint"),
        ".accdb" => t!("nuevo.base_datos_access"),
        ".pub" => t!("nuevo.publicacion_publisher"),
        ".vsdx" => t!("nuevo.dibujo_visio"),
        ".odt" => t!("nuevo.documento_texto_opendocument"),
        ".ods" => t!("nuevo.hoja_calculo_opendocument"),
        ".odp" => t!("nuevo.presentacion_opendocument"),
        ".odg" => t!("nuevo.dibujo_opendocument"),
        ".bmp" => t!("nuevo.imagen_mapa_bits"),
        ".zip" => t!("nuevo.carpeta_comprimida"),
        ".contact" => t!("nuevo.contacto"),
        _ => return None,
    })
}

/// Nombre del tipo según Windows ("Documento de texto", "Documento de Microsoft Word"…).
fn type_name(ext: &str) -> String {
    if let Some(n) = known_name(ext) {
        return n.to_string();
    }
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW};
    let w = wide(&format!("x{ext}"));
    let mut info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(PCWSTR(w.as_ptr()), FILE_ATTRIBUTE_NORMAL, Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES);
    }
    let n = text(unsafe { std::slice::from_raw_parts(info.szTypeName.as_ptr() as *const u8, info.szTypeName.len() * 2) });
    if n.is_empty() { tf!("comun.archivo_extension", extension = ext.trim_start_matches('.').to_uppercase()) } else { n }
}

fn scan() -> Vec<NewItem> {
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let r = unsafe { RegEnumKeyExW(HKEY_CLASSES_ROOT, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if r.is_err() {
            break;
        }
        i += 1;
        let ext = String::from_utf16_lossy(&name[..len as usize]);
        if !ext.starts_with('.') || ext.len() < 2 {
            // Las extensiones van primero en orden alfabético (empiezan con "."): al terminar, listo.
            if !out.is_empty() || i > 4000 {
                if !ext.starts_with('.') && ext.as_bytes()[0] > b'.' {
                    break;
                }
            }
            continue;
        }
        let Some(k) = open_key(HKEY_CLASSES_ROOT, &ext) else { continue };
        // ShellNew directo bajo la extensión, o bajo su tipo (ProgID).
        let mut found = open_key(k, "ShellNew").and_then(|s| {
            let t = template_in(s);
            close(s);
            t
        });
        if found.is_none() {
            if let Some((_, prog)) = value(k, "") {
                let prog = text(&prog);
                if !prog.is_empty() {
                    found = open_key(k, &format!("{prog}\\ShellNew")).and_then(|s| {
                        let t = template_in(s);
                        close(s);
                        t
                    });
                }
            }
        }
        close(k);
        if let Some(kind) = found {
            // Los accesos directos y la "biblioteca" no son documentos.
            if matches!(ext.as_str(), ".lnk" | ".library-ms" | ".contact" | ".jnt") {
                continue;
            }
            out.push(NewItem { name: type_name(&ext), ext, kind });
        }
    }
    // Siempre útiles, aunque ningún programa los registre.
    if !out.iter().any(|n| n.ext == ".md") {
        out.push(NewItem { ext: ".md".into(), name: type_name(".md"), kind: Template::Empty });
    }
    if !out.iter().any(|n| n.ext == ".zip") {
        // ZIP vacío válido (solo el final del directorio central).
        let empty_zip = vec![0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.push(NewItem { ext: ".zip".into(), name: type_name(".zip"), kind: Template::Data(empty_zip) });
    }
    // El de texto primero (es el más usado); el resto por nombre.
    out.sort_by_key(|n| (n.ext != ".txt", n.name.to_lowercase()));
    out
}

/// Crea "Nuevo {tipo}.ext" en `dir` (con " (2)" si ya existe) y devuelve la ruta.
pub fn create(dir: &str, item: &NewItem) -> Result<String, String> {
    let base = tf!("nuevo.nombre_nuevo", tipo = item.name.trim());
    let base = base.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "");
    let mut path = join(dir, &format!("{base}{}", item.ext));
    let mut k = 2;
    while std::fs::symlink_metadata(long_path(&path)).is_ok() {
        path = join(dir, &format!("{base} ({k}){}", item.ext));
        k += 1;
    }
    let bytes = match &item.kind {
        Template::Empty => Vec::new(),
        Template::Data(d) => d.clone(),
        Template::File(f) => {
            // Rutas de plantilla: absolutas o en las carpetas de plantillas de Windows.
            let expanded = expand(f);
            let candidates = [
                expanded.clone(),
                format!("{}\\ShellNew\\{expanded}", std::env::var("WINDIR").unwrap_or_default()),
                format!("{}\\Microsoft\\Windows\\Templates\\{expanded}", std::env::var("APPDATA").unwrap_or_default()),
                format!("{}\\Microsoft\\Windows\\Templates\\{expanded}", std::env::var("ProgramData").unwrap_or_default()),
            ];
            candidates.iter().find_map(|c| std::fs::read(long_path(c)).ok()).unwrap_or_default()
        }
    };
    std::fs::write(long_path(&path), bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                out.push_str(&std::env::var(&after[..j]).unwrap_or_default());
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

#[cfg(test)]
mod tests {
    #[test]
    fn finds_text_document() {
        let items = super::scan();
        let txt = items.iter().find(|i| i.ext == ".txt").expect("debería existir «Nuevo documento de texto»");
        let d = std::env::temp_dir().join(format!("rayo-nuevo-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = super::create(&d.to_string_lossy(), txt).unwrap();
        let p2 = super::create(&d.to_string_lossy(), txt).unwrap();
        assert!(p2.contains("(2)"), "{p2}");
        assert!(std::path::Path::new(&p).is_file());
        println!("{} tipos: {}", items.len(), items.iter().map(|i| format!("{} ({})", i.name, i.ext)).collect::<Vec<_>>().join(", "));
        let _ = std::fs::remove_dir_all(d);
    }
}
