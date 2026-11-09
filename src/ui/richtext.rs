//! Texto con formato para la vista previa: Markdown (títulos, listas, tareas, tablas, citas,
//! bloques de código) y código con colores de sintaxis. Sin dependencias: un resaltador simple
//! que entiende lo común a casi todos los lenguajes (palabras clave, textos, números, comentarios).

use super::theme::{self, Theme};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Ui, vec2};
use egui_phosphor::regular as ic;

/// Extensiones que se muestran con colores de sintaxis.
pub fn is_code(ext: &str) -> bool {
    matches!(
        ext,
        "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "kt" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "go" | "rb" | "php" | "swift" | "lua" | "dart" | "vue"
            | "css" | "scss" | "sass" | "less" | "sql" | "sh" | "bat" | "cmd" | "ps1" | "zig" | "nix" | "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "gradle"
            | "html" | "htm" | "xml" | "svg"
    )
}

struct Palette {
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    kind: Color32,
    text: Color32,
}

fn palette(t: &Theme) -> Palette {
    let c = |d: u32, l: u32| {
        let v = if t.dark { d } else { l };
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    };
    Palette { keyword: c(0xC792EA, 0x8B3FD9), string: c(0x9ECE6A, 0x2E7D32), number: c(0xF7A35C, 0xB35C00), comment: t.faint, kind: c(0x5FC8D8, 0x0B7285), text: t.text }
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "const", "static", "struct", "enum", "impl", "trait", "pub", "use", "mod", "crate", "self", "Self", "super", "match", "if", "else", "for", "while",
    "loop", "return", "break", "continue", "in", "as", "where", "async", "await", "move", "ref", "type", "unsafe", "dyn", "def", "class", "import", "from", "lambda", "yield",
    "with", "try", "except", "finally", "raise", "pass", "not", "and", "or", "is", "None", "True", "False", "function", "var", "new", "this", "typeof", "instanceof", "export",
    "default", "extends", "interface", "implements", "package", "private", "protected", "public", "void", "int", "float", "double", "bool", "char", "string", "null", "true",
    "false", "nil", "func", "go", "defer", "chan", "select", "case", "switch", "do", "catch", "throw", "throws", "static", "final", "override", "virtual", "namespace",
    "using", "include", "define", "elif", "then", "fi", "esac", "echo", "local", "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "JOIN", "ORDER", "BY", "GROUP",
];

/// Una línea de código con colores. `block` dice si se está dentro de un comentario /* … */.
fn code_line(job: &mut LayoutJob, line: &str, p: &Palette, font: &FontId, block: &mut bool) {
    let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if *block {
            let end = line[i..].find("*/").map_or(b.len(), |k| i + k + 2);
            job.append(&line[i..end], 0.0, fmt(p.comment));
            *block = end >= b.len() && !line[i..].contains("*/");
            i = end;
            continue;
        }
        let rest = &line[i..];
        if rest.starts_with("//") || rest.starts_with('#') && !rest.starts_with("#[") && !rest.starts_with("#!") || rest.starts_with("--") || rest.starts_with("<!--") {
            job.append(rest, 0.0, fmt(p.comment));
            return;
        }
        if rest.starts_with("/*") {
            let end = rest[2..].find("*/").map(|k| i + 2 + k + 2);
            match end {
                Some(e) => {
                    job.append(&line[i..e], 0.0, fmt(p.comment));
                    i = e;
                }
                None => {
                    job.append(rest, 0.0, fmt(p.comment));
                    *block = true;
                    return;
                }
            }
            continue;
        }
        let c = b[i] as char;
        if c == '"' || c == '\'' || c == '`' {
            // Hasta la comilla de cierre (saltando las escapadas).
            let mut j = i + 1;
            while j < b.len() && b[j] as char != c {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            // Un apóstrofo suelto (lifetimes de Rust, contracciones) no abre un texto.
            if c == '\'' && end - i > 4 && !line[i + 1..].contains('\'') {
                job.append(&line[i..i + 1], 0.0, fmt(p.text));
                i += 1;
                continue;
            }
            job.append(&line[i..end], 0.0, fmt(p.string));
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_') {
                j += 1;
            }
            job.append(&line[i..j], 0.0, fmt(p.number));
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < b.len() && ((b[j] as char).is_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) {
                j += 1;
            }
            // Fin seguro en un límite de carácter.
            while !line.is_char_boundary(j) {
                j += 1;
            }
            let w = &line[i..j];
            let color = if KEYWORDS.contains(&w) {
                p.keyword
            } else if w.chars().next().is_some_and(|c| c.is_uppercase()) {
                p.kind
            } else {
                p.text
            };
            job.append(w, 0.0, fmt(color));
            i = j;
            continue;
        }
        let mut j = i + 1;
        while !line.is_char_boundary(j) {
            j += 1;
        }
        job.append(&line[i..j], 0.0, fmt(p.text));
        i = j;
    }
}

/// Código con colores de sintaxis (archivos de código y bloques de Markdown).
pub fn code(ui: &mut Ui, t: &Theme, text: &str, size: f32) {
    let p = palette(t);
    let font = FontId::monospace(size - 1.0);
    let mut job = LayoutJob::default();
    let mut block = false;
    for (k, line) in text.lines().take(4000).enumerate() {
        if k > 0 {
            job.append("\n", 0.0, TextFormat::simple(font.clone(), p.text));
        }
        code_line(&mut job, line, &p, &font, &mut block);
    }
    ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
}

/// Texto de una línea de Markdown con **negrita**, *cursiva*, `código` y [enlaces](url).
fn inline_job(text: &str, t: &Theme, size: f32, color: Color32, base: FontId, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let plain = TextFormat::simple(base.clone(), color);
    let mut rest = text;
    while !rest.is_empty() {
        // El próximo marcador.
        let next = ["**", "__", "`", "*", "[", "!["].iter().filter_map(|m| rest.find(m).map(|i| (i, *m))).min_by_key(|x| (x.0, std::cmp::Reverse(x.1.len())));
        let Some((i, m)) = next else {
            job.append(rest, 0.0, plain.clone());
            break;
        };
        job.append(&rest[..i], 0.0, plain.clone());
        let after = &rest[i + m.len()..];
        let (close, fmt) = match m {
            "**" | "__" => (m, TextFormat::simple(theme::semibold(size), color)),
            "`" => {
                let mut f = TextFormat::simple(FontId::monospace(size - 1.0), t.accent);
                f.background = t.hover;
                ("`", f)
            }
            "*" => {
                let mut f = plain.clone();
                f.italics = true;
                ("*", f)
            }
            "![" => ("]", TextFormat::simple(base.clone(), t.dim)),
            _ => ("]", TextFormat::simple(base.clone(), t.accent)),
        };
        match after.find(close) {
            Some(j) if j > 0 => {
                let shown = &after[..j];
                if m == "![" {
                    job.append(&format!("{} {shown}", ic::IMAGE), 0.0, fmt);
                } else {
                    job.append(shown, 0.0, fmt);
                }
                rest = &after[j + close.len()..];
                // [texto](url): la url no se muestra.
                if (m == "[" || m == "![") && rest.starts_with('(') {
                    rest = rest.find(')').map_or(rest, |k| &rest[k + 1..]);
                }
            }
            _ => {
                job.append(m, 0.0, plain.clone());
                rest = after;
            }
        }
    }
    job
}

fn table_cells(line: &str) -> Vec<String> {
    let l = line.trim().trim_start_matches('|').trim_end_matches('|');
    l.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_table_rule(line: &str) -> bool {
    let l = line.trim();
    l.contains('-') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Markdown: títulos, listas (y tareas), tablas, citas, bloques de código, separadores y
/// formato en línea.
pub fn markdown(ui: &mut Ui, t: &Theme, text: &str, size: f32) {
    let width = ui.available_width();
    let lines: Vec<&str> = text.lines().take(3000).collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        // Bloque de código: todo junto, en un recuadro, con colores.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = &trimmed[..3];
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                body.push(lines[i]);
                i += 1;
            }
            i += 1;
            ui.add_space(2.0);
            egui::Frame::new().fill(t.hover).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_width(width - 20.0);
                egui::ScrollArea::horizontal().id_salt(("md-code", i)).show(ui, |ui| code(ui, t, &body.join("\n"), size - 0.5));
            });
            ui.add_space(4.0);
            continue;
        }
        // Tabla: | a | b | seguida de |---|---|.
        if trimmed.starts_with('|') && lines.get(i + 1).is_some_and(|l| is_table_rule(l)) {
            let header = table_cells(trimmed);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            ui.add_space(2.0);
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                egui::ScrollArea::horizontal().id_salt(("md-table", i)).show(ui, |ui| {
                    egui::Grid::new(("md-grid", i)).striped(true).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                        for h in &header {
                            ui.label(inline_job(h, t, size, t.text, theme::semibold(size), f32::INFINITY));
                        }
                        ui.end_row();
                        for r in &rows {
                            for c in r {
                                ui.label(inline_job(c, t, size, t.text, FontId::proportional(size), f32::INFINITY));
                            }
                            ui.end_row();
                        }
                    });
                });
            });
            ui.add_space(4.0);
            continue;
        }
        i += 1;
        if trimmed.is_empty() {
            ui.add_space(size * 0.5);
            continue;
        }
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            ui.add_space(if level <= 2 { 8.0 } else { 4.0 });
            let fs = size + (4usize.saturating_sub(level)) as f32 * 2.5;
            ui.label(inline_job(trimmed[level..].trim(), t, fs, t.text, theme::semibold(fs), width));
            if level <= 2 {
                ui.separator();
            }
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            ui.separator();
            continue;
        }
        let indent = (line.len() - trimmed.len()) as f32 * 5.0;
        let list = ["- ", "* ", "+ "].iter().find(|m| trimmed.starts_with(*m)).map(|_| ("•".to_string(), &trimmed[2..])).or_else(|| {
            let n: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!n.is_empty() && trimmed[n.len()..].starts_with(". ")).then(|| (format!("{n}."), &trimmed[n.len() + 2..]))
        });
        if let Some((mark, rest)) = list {
            let task = rest.strip_prefix("[ ] ").map(|r| (r, false)).or_else(|| rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")).map(|r| (r, true)));
            ui.horizontal_top(|ui| {
                ui.add_space(indent + 2.0);
                // La marca en su columna; el texto se acomoda debajo de sí mismo.
                let (mark, color, rest) = match task {
                    Some((r, done)) => (if done { ic::CHECK_SQUARE } else { ic::SQUARE }.to_string(), if done { t.accent } else { t.dim }, r),
                    None => (mark, t.dim, rest),
                };
                let (mr, _) = ui.allocate_exact_size(vec2(16.0, size + 4.0), egui::Sense::hover());
                let font = if task.is_some() { theme::icon_font(size, false) } else { FontId::proportional(size) };
                ui.painter().text(mr.center(), egui::Align2::CENTER_CENTER, mark, font, color);
                let w = ui.available_width();
                let done = task.is_some_and(|t| t.1);
                let mut job = inline_job(rest, t, size, if done { t.dim } else { t.text }, FontId::proportional(size), w);
                if done {
                    for s in &mut job.sections {
                        s.format.strikethrough = egui::Stroke::new(1.0, t.dim);
                    }
                }
                ui.label(job);
            });
            continue;
        }
        if let Some(q) = trimmed.strip_prefix('>') {
            // Cita: las líneas seguidas van juntas, con una barra a la izquierda.
            let mut quote = vec![q.trim().to_string()];
            while i < lines.len() {
                let Some(more) = lines[i].trim_start().strip_prefix('>') else { break };
                quote.push(more.trim().to_string());
                i += 1;
            }
            let r = egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
                ui.label(inline_job(&quote.join(" "), t, size, t.dim, FontId::proportional(size), width - 16.0));
            });
            let rr = r.response.rect;
            ui.painter().line_segment([rr.left_top() + vec2(3.0, 0.0), rr.left_bottom() + vec2(3.0, 0.0)], egui::Stroke::new(3.0, t.accent.gamma_multiply(0.5)));
            continue;
        }
        ui.label(inline_job(trimmed, t, size, t.text, FontId::proportional(size), width));
    }
}
