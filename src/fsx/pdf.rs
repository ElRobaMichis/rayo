//! Páginas de PDF como imagen, con el lector de PDF que trae Windows
//! (`Windows.Data.Pdf`, el mismo de Edge): sin programas externos.

use super::thumbs::Rgba;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Data::Pdf::PdfDocument;

/// Separador entre la ruta y el número de página en las claves de miniatura
/// (`C:\doc.pdf|3`). `|` no puede aparecer en una ruta de Windows.
pub const PAGE_SEP: char = '|';

/// Clave de una página (la primera es la ruta sola).
pub fn page_key(path: &str, page: u32) -> String {
    if page == 0 { path.to_string() } else { format!("{path}{PAGE_SEP}{page}") }
}

/// Sin uso por este tiempo, el documento abierto se suelta solo.
const IDLE: Duration = Duration::from_secs(10);

/// El último documento abierto: la Vista rápida pide decenas de páginas del mismo (tira de
/// miniaturas, páginas continuas) y abrirlo cada vez cuesta. Mientras está abierto, Windows puede
/// no dejar borrar el archivo: `forget` antes de borrar, mover o renombrar.
struct Session {
    path: String,
    doc: PdfDocument,
    used: Instant,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

/// Suelta el documento si es ese (al cerrar la Vista rápida, al pasar a otro archivo, antes de
/// borrarlo).
pub fn forget(path: &str) {
    let old = {
        let mut s = session();
        if s.as_ref().is_some_and(|s| s.path == path) { s.take() } else { None }
    };
    drop(old);
}

/// El documento abierto (de la sesión, o lo abre y lo deja en ella). En un hilo multihilo de COM.
fn document(path: &str) -> Option<PdfDocument> {
    use windows::Storage::StorageFile;
    use windows::core::HSTRING;
    if let Some(s) = session().as_mut().filter(|s| s.path == path) {
        s.used = Instant::now();
        return Some(s.doc.clone());
    }
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).ok()?.join().ok()?;
    let doc = PdfDocument::LoadFromFileAsync(&file).ok()?.join().ok()?;
    let old = session().replace(Session { path: path.to_string(), doc: doc.clone(), used: Instant::now() });
    drop(old);
    janitor();
    Some(doc)
}

/// Un hilo que suelta el documento tras `IDLE` sin uso.
fn janitor() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = std::thread::Builder::new().name("pdf-sesion".into()).spawn(|| {
            mta();
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let old = {
                    let mut s = session();
                    if s.as_ref().is_some_and(|s| s.used.elapsed() > IDLE) { s.take() } else { None }
                };
                drop(old);
            }
        });
    });
}

fn mta() {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// WinRT con espera bloqueante: en un hilo propio del modelo multihilo de COM.
fn in_mta<T: Send>(f: impl FnOnce() -> Option<T> + Send) -> Option<T> {
    std::thread::scope(|s| {
        s.spawn(|| {
            mta();
            f()
        })
        .join()
        .ok()
        .flatten()
    })
}

/// Ancho y alto de cada página (en DIP, como `Rgba::orig`). Para el diseño continuo: el alto de
/// cada página antes de dibujarla. Puede tardar con cientos de páginas: en segundo plano.
pub fn sizes(path: &str) -> Option<Vec<(f32, f32)>> {
    if super::is_online_only(path) {
        return None;
    }
    in_mta(|| {
        let doc = document(path)?;
        let n = doc.PageCount().ok()?;
        (0..n).map(|i| doc.GetPage(i).ok()?.Size().ok().map(|s| (s.Width, s.Height))).collect()
    })
}

/// Dibuja una página (desde 0) con su lado mayor de `max_side` píxeles.
/// Devuelve la imagen y la cantidad de páginas.
pub fn render(path: &str, page: u32, max_side: u32) -> Option<(Rgba, u32)> {
    if super::is_online_only(path) {
        return None;
    }
    in_mta(|| render_mta(path, page, max_side))
}

fn render_mta(path: &str, page: u32, max_side: u32) -> Option<(Rgba, u32)> {
    use windows::Data::Pdf::PdfPageRenderOptions;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    let doc = document(path)?;
    let pages = doc.PageCount().ok()?;
    if pages == 0 {
        return None;
    }
    let p = doc.GetPage(page.min(pages - 1)).ok()?;
    let size = p.Size().ok()?;
    let opts = PdfPageRenderOptions::new().ok()?;
    if size.Width >= size.Height {
        opts.SetDestinationWidth(max_side).ok()?;
    } else {
        opts.SetDestinationHeight(max_side).ok()?;
    }
    let stream = InMemoryRandomAccessStream::new().ok()?;
    p.RenderWithOptionsToStreamAsync(&stream, &opts).ok()?.join().ok()?;
    let len = stream.Size().ok()? as u32;
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    reader.LoadAsync(len).ok()?.join().ok()?;
    let mut bytes = vec![0u8; len as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mut img = super::thumbs::wic_decode_bytes(&bytes, max_side)?;
    // Las páginas sin fondo salen transparentes: se ponen sobre blanco, como en papel.
    for px in img.px.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
        px[3] = 255;
    }
    img.orig = Some((size.Width as u32, size.Height as u32));
    Some((img, pages))
}

#[cfg(test)]
pub mod tests {
    /// PDF mínimo de `pages` páginas (A4 vertical) con un texto en cada una.
    pub fn tiny_pdf(pages: u32) -> Vec<u8> {
        let mut objs: Vec<String> = Vec::new();
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
        objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
        objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
        for i in 0..pages {
            let content = format!("BT /F1 48 Tf 72 700 Td (Pagina {}) Tj ET", i + 1);
            objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", 5 + i * 2));
            objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (k, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", k + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    #[test]
    fn renders_pages() {
        let path = std::env::temp_dir().join(format!("rayo-pdf-{}.pdf", std::process::id()));
        std::fs::write(&path, tiny_pdf(3)).unwrap();
        let p = path.to_string_lossy().into_owned();
        let (img, pages) = super::render(&p, 1, 400).expect("no se pudo dibujar el PDF");
        assert_eq!(pages, 3);
        assert_eq!(img.h, 400, "A4 vertical: el alto es el lado mayor");
        assert!(img.w > 250 && img.w < 300, "{}", img.w);
        // Hay texto negro sobre blanco.
        assert!(img.px.chunks_exact(4).any(|p| p[0] < 80));
        assert!(img.px.chunks_exact(4).filter(|p| p[0] > 240).count() > img.px.len() / 8);
        super::forget(&p);
        let _ = std::fs::remove_file(path);
    }

    /// Medidas de todas las páginas desde la sesión, y `forget` suelta el archivo.
    #[test]
    fn sizes_and_forget() {
        let path = std::env::temp_dir().join(format!("rayo-pdf-sizes-{}.pdf", std::process::id()));
        std::fs::write(&path, tiny_pdf(4)).unwrap();
        let p = path.to_string_lossy().into_owned();
        let s = super::sizes(&p).expect("medidas");
        assert_eq!(s.len(), 4);
        // En DIP (96 por pulgada): A4 vertical, 595 × 842 puntos.
        assert!(s.iter().all(|&(w, h)| (w - 595.0 * 96.0 / 72.0).abs() < 1.0 && (h / w - 842.0 / 595.0).abs() < 0.01), "{s:?}");
        // La sesión sigue abierta: otra página sale del mismo documento.
        assert_eq!(super::render(&p, 3, 200).map(|(_, n)| n), Some(4));
        super::forget(&p);
        std::fs::remove_file(&path).expect("tras forget el archivo se puede borrar");
    }
}
