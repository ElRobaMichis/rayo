//! Terminal integrada: PowerShell (o pwsh) en la carpeta actual, en un panel abajo (Ctrl+`).
//!
//! Corre en una pseudoconsola de Windows (ConPTY); un hilo lee lo que escribe el programa y lo
//! interpreta como una terminal (colores, cursor, borrar pantalla…), y aquí se dibuja esa
//! cuadrícula de caracteres. Las teclas se traducen a lo que espera una terminal.

use super::theme::Theme;
use egui::{Color32, FontId, Key, Rect, Sense, Ui, pos2, vec2};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Terminal {
    parser: Arc<Mutex<vt100::Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    /// Para cerrar el shell al quitar el panel (el proceso lo espera otro hilo).
    killer: Box<dyn ChildKiller + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: (u16, u16),
    /// Líneas hacia atrás que se están viendo (0 = lo último).
    scroll: usize,
}

/// PowerShell 7 si está instalado; si no, el de Windows.
fn shell() -> String {
    let found = std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("pwsh.exe")).find(|f| f.is_file()));
    found.map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "powershell.exe".into())
}

impl Terminal {
    pub fn spawn(cwd: &str, ctx: &egui::Context) -> Result<Terminal, String> {
        let size = (24u16, 100u16);
        let pair = native_pty_system().openpty(PtySize { rows: size.0, cols: size.1, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new(shell());
        cmd.arg("-NoLogo");
        cmd.cwd(cwd);
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        let killer = child.clone_killer();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(size.0, size.1, 5000)));
        let exited = Arc::new(AtomicBool::new(false));
        let (p, done, c, w) = (parser.clone(), exited.clone(), ctx.clone(), writer.clone());
        std::thread::Builder::new()
            .name("terminal".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut parser = p.lock();
                            parser.process(&buf[..n]);
                            // La consola de Windows pregunta dónde está el cursor (ESC[6n) y no
                            // muestra nada hasta tener respuesta: se la da la terminal.
                            if buf[..n].windows(4).any(|x| x == b"\x1b[6n") {
                                let (r, col) = parser.screen().cursor_position();
                                let reply = format!("\x1b[{};{}R", r + 1, col + 1);
                                let mut w = w.lock();
                                let _ = w.write_all(reply.as_bytes());
                                let _ = w.flush();
                            }
                            drop(parser);
                            c.request_repaint();
                        }
                    }
                }
                done.store(true, Ordering::Relaxed);
                c.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        // Con ConPTY la salida no se corta cuando el shell termina (`exit`): sigue abierta hasta
        // cerrar la pseudoconsola. Por eso se espera al proceso mismo.
        let (done, c) = (exited.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("terminal-espera".into())
            .spawn(move || {
                let _ = child.wait();
                done.store(true, Ordering::Relaxed);
                c.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        Ok(Terminal { parser, writer, master: pair.master, killer, exited, size, scroll: 0 })
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::Relaxed)
    }

    fn send(&mut self, bytes: &[u8]) {
        let mut w = self.writer.lock();
        let _ = w.write_all(bytes);
        let _ = w.flush();
        self.scroll = 0;
    }

    /// Ir a una carpeta (como escribir `cd` en la terminal).
    pub fn cd(&mut self, path: &str) {
        let cmd = format!("Set-Location -LiteralPath '{}'\r", path.replace('\'', "''"));
        self.send(cmd.as_bytes());
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == self.size || rows < 2 || cols < 10 {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.lock().set_size(rows, cols);
    }

    /// Dibuja la terminal en `rect`. `focused`: recibe el teclado. Devuelve la respuesta del área
    /// (un clic en ella le da el foco).
    pub fn ui(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, focused: bool) -> egui::Response {
        let resp = ui.interact(rect, ui.id().with("terminal"), Sense::click());
        let font = FontId::monospace(13.0);
        let (cw, ch) = {
            let g = ui.painter().layout_no_wrap("M".into(), font.clone(), Color32::WHITE);
            (g.size().x.max(1.0), (g.size().y + 1.0).max(1.0))
        };
        let inner = rect.shrink2(vec2(8.0, 6.0));
        let rows = (inner.height() / ch).floor().max(2.0) as u16;
        let cols = (inner.width() / cw).floor().max(10.0) as u16;
        self.resize(rows, cols);
        if focused {
            self.keys(ui);
        }
        // Rueda del mouse: historial.
        if resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                let lines = (dy / ch).round() as isize;
                self.scroll = (self.scroll as isize + lines).clamp(0, 5000) as usize;
            }
        }
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, if t.dark { Color32::from_rgb(0x0C, 0x0D, 0x10) } else { Color32::from_rgb(0xFA, 0xFA, 0xFB) });
        let mut parser = self.parser.lock();
        parser.set_scrollback(self.scroll);
        self.scroll = parser.screen().scrollback();
        let screen = parser.screen();
        let fg_default = if t.dark { Color32::from_rgb(0xD8, 0xDB, 0xE2) } else { Color32::from_rgb(0x1D, 0x21, 0x27) };
        for r in 0..rows {
            let y = inner.top() + r as f32 * ch;
            let mut job = egui::text::LayoutJob::default();
            for c in 0..cols {
                let Some(cell) = screen.cell(r, c) else { continue };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = color(cell.fgcolor(), fg_default, t.dark);
                let mut bg = match cell.bgcolor() {
                    vt100::Color::Default => None,
                    other => Some(color(other, fg_default, t.dark)),
                };
                if cell.inverse() {
                    let old = fg;
                    fg = bg.unwrap_or(if t.dark { Color32::from_rgb(0x0C, 0x0D, 0x10) } else { Color32::WHITE });
                    bg = Some(old);
                }
                let x = inner.left() + c as f32 * cw;
                if let Some(bg) = bg {
                    let w = if cell.is_wide() { 2.0 * cw } else { cw };
                    p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(w, ch)), 0.0, bg);
                }
                let s = cell.contents();
                let text = if s.is_empty() { " ".to_string() } else { s.to_string() };
                let mut f = egui::text::TextFormat::simple(font.clone(), fg);
                if cell.bold() {
                    f.color = fg.gamma_multiply(1.15);
                }
                if cell.underline() {
                    f.underline = egui::Stroke::new(1.0, fg);
                }
                job.append(&text, 0.0, f);
            }
            let g = p.layout_job(job);
            p.galley(pos2(inner.left(), y), g, fg_default);
        }
        // Cursor (solo viendo lo último).
        if self.scroll == 0 && !screen.hide_cursor() {
            let (cr, cc) = screen.cursor_position();
            let r = Rect::from_min_size(pos2(inner.left() + cc as f32 * cw, inner.top() + cr as f32 * ch), vec2(cw, ch));
            if focused {
                p.rect_filled(r, 1.0, t.accent.gamma_multiply(0.7));
            } else {
                p.rect_stroke(r, 1.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
        }
        resp
    }

    /// Teclado → lo que espera una terminal.
    fn keys(&mut self, ui: &mut Ui) {
        let events = ui.input_mut(|i| std::mem::take(&mut i.events));
        let mut keep = Vec::new();
        for e in events {
            match &e {
                egui::Event::Text(s) => self.send(s.as_bytes()),
                egui::Event::Paste(s) => {
                    let s = s.replace("\r\n", "\r").replace('\n', "\r");
                    self.send(s.as_bytes());
                }
                egui::Event::Copy => {
                    // Ctrl+C en una terminal interrumpe (como en la consola).
                    self.send(b"\x03");
                }
                // egui convierte Ctrl+X en "cortar": a la terminal le llega como ^X.
                egui::Event::Cut => self.send(b"\x18"),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(bytes) = key_bytes(*key, *modifiers) {
                        self.send(&bytes);
                    } else {
                        keep.push(e);
                    }
                }
                _ => keep.push(e),
            }
        }
        ui.input_mut(|i| i.events = keep);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

fn key_bytes(key: Key, m: egui::Modifiers) -> Option<Vec<u8>> {
    if m.ctrl && !m.alt {
        // Ctrl+letra → carácter de control.
        let name = key.name();
        if name.len() == 1 {
            let c = name.as_bytes()[0].to_ascii_uppercase();
            if c.is_ascii_uppercase() && c != b'V' && c != b'C' {
                return Some(vec![c - b'A' + 1]);
            }
        }
    }
    let b: &[u8] = match key {
        Key::Enter => b"\r",
        Key::Backspace => b"\x7f",
        Key::Tab => {
            if m.shift {
                b"\x1b[Z"
            } else {
                b"\t"
            }
        }
        Key::Escape => b"\x1b",
        Key::ArrowUp => b"\x1b[A",
        Key::ArrowDown => b"\x1b[B",
        Key::ArrowRight => {
            if m.ctrl {
                b"\x1b[1;5C"
            } else {
                b"\x1b[C"
            }
        }
        Key::ArrowLeft => {
            if m.ctrl {
                b"\x1b[1;5D"
            } else {
                b"\x1b[D"
            }
        }
        Key::Home => b"\x1b[H",
        Key::End => b"\x1b[F",
        Key::Delete => b"\x1b[3~",
        Key::PageUp => b"\x1b[5~",
        Key::PageDown => b"\x1b[6~",
        Key::Insert => b"\x1b[2~",
        _ => return None,
    };
    Some(b.to_vec())
}

/// Colores de la terminal (los 16 básicos según el tema, los 256 de xterm y RGB).
fn color(c: vt100::Color, default: Color32, dark: bool) -> Color32 {
    const DARK: [u32; 16] = [0x1E1F24, 0xF0616D, 0x7FD88F, 0xF5C76A, 0x5EA6F8, 0xC792EA, 0x5FC8D8, 0xC9CCD3, 0x5C6370, 0xFF7A85, 0x9EE6A9, 0xFFD98A, 0x82BBFF, 0xD7A9F2, 0x86DCE8, 0xFFFFFF];
    const LIGHT: [u32; 16] = [0x1D2127, 0xC4262E, 0x2E7D32, 0x9A6700, 0x1F5FCC, 0x8B3FD9, 0x0B7285, 0x6A717D, 0x5D6573, 0xE03A3E, 0x3A8F3F, 0xB57F00, 0x2F6FED, 0xA25BE6, 0x1592A6, 0x1D2127];
    let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    match c {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) if i < 16 => rgb(if dark { DARK[i as usize] } else { LIGHT[i as usize] }),
        vt100::Color::Idx(i) if i < 232 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}
