//! Los íconos de Windows para los archivos, como en el Explorador: el de cada programa (`.exe`),
//! acceso directo (`.lnk`, `.url`) o ícono (`.ico`), que son propios de cada archivo, y el del
//! tipo para todo lo demás (todos los `.zip` comparten uno, que se pide una sola vez).
//!
//! Se cargan en los hilos de las miniaturas y se guardan aparte de ellas: no se pierden al pasar
//! por una carpeta llena de fotos. Mientras no llegan, se dibuja el ícono de Rayo.

use crate::fsx::thumbs::{ICON_PREFIX, Rgba, ThumbKey, ThumbLoader};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
struct Cache {
    loader: Option<Arc<ThumbLoader>>,
    /// Ícono listo (o None si Windows no dio ninguno), por clave de caché.
    ready: HashMap<String, Option<TextureHandle>>,
    /// Pedidos en curso: clave de caché → lo que se pidió al cargador.
    asked: HashMap<String, ThumbKey>,
    /// Pedidos que fallaron: cuántas veces y cuándo la última (se reintenta).
    failed: HashMap<String, (u32, std::time::Instant)>,
    ctx: Option<egui::Context>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

pub fn init(loader: Arc<ThumbLoader>) {
    CACHE.with(|c| c.borrow_mut().loader = Some(loader));
}

/// Tipos cuyo ícono cambia de un archivo a otro.
fn own_icon(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "url" | "ico" | "cur" | "ani" | "scr" | "cpl" | "msc" | "appref-ms" | "website")
}

fn cache_key(path: &str, ext: &str, size: u32) -> String {
    if own_icon(ext) { format!("{}|{size}", path.to_lowercase()) } else { format!(".{ext}|{size}") }
}

/// El ícono de Windows de un archivo, en `size` píxeles, si ya está cargado (si no, se pide).
pub fn get(path: &str, ext: &str, size: u32) -> Option<TextureHandle> {
    let ext = ext.to_lowercase();
    let key = cache_key(path, &ext, size);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(t) = c.ready.get(&key) {
            return t.clone();
        }
        if let Some((tries, at)) = c.failed.get(&key) {
            if at.elapsed() < std::time::Duration::from_millis(700 * u64::from(*tries)) {
                // Otro cuadro dentro de un rato, para reintentar.
                if let Some(ctx) = &c.ctx {
                    ctx.request_repaint_after(std::time::Duration::from_millis(800));
                }
                return None;
            }
        }
        let req = c.asked.entry(key).or_insert_with(|| ThumbKey { path: format!("{ICON_PREFIX}{path}"), size }).clone();
        // Se vuelve a pedir en cada cuadro mientras se necesite (lo que deja de pedirse se olvida).
        if let Some(l) = &c.loader {
            l.request(req, false);
        }
        None
    })
}

/// Lo que llegó del cargador para un pedido de ícono.
pub fn deliver(ctx: &egui::Context, key: &ThumbKey, img: Option<Rgba>) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.ctx.get_or_insert_with(|| ctx.clone());
        let Some(cache) = c.asked.iter().find(|(_, k)| *k == key).map(|(c, _)| c.clone()) else { return };
        c.asked.remove(&cache);
        // A veces Windows contesta que está ocupado (p. ej. el ícono de los PDF que abre el navegador):
        // se vuelve a intentar unas veces, con pausas, antes de quedarse con el ícono de Rayo.
        if img.is_none() {
            let tries = c.failed.get(&cache).map_or(0, |f| f.0) + 1;
            if tries < 5 {
                c.failed.insert(cache, (tries, std::time::Instant::now()));
                return;
            }
        }
        c.failed.remove(&cache);
        let tex = img.map(|img| {
            let size = [img.w as usize, img.h as usize];
            let ci = if img.premultiplied { ColorImage::from_rgba_premultiplied(size, &img.px) } else { ColorImage::from_rgba_unmultiplied(size, &img.px) };
            ctx.load_texture(&cache, ci, TextureOptions::LINEAR)
        });
        if c.ready.len() > 4000 {
            c.ready.clear();
        }
        c.ready.insert(cache, tex);
    });
}

/// Píxeles a pedir para dibujar un ícono de `points` puntos (nítido en pantallas con escala).
pub fn px_for(ctx: &egui::Context, points: f32) -> u32 {
    let px = (points * ctx.pixels_per_point()).ceil() as u32;
    match px {
        0..=16 => 16,
        17..=24 => 24,
        25..=32 => 32,
        33..=48 => 48,
        49..=64 => 64,
        65..=96 => 96,
        _ => 128,
    }
}
