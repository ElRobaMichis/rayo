//! Texto con formato para la vista previa: Markdown (títulos, listas, tareas, tablas, citas,
//! bloques de código) y código con colores de sintaxis. Sin dependencias: un resaltador simple
//! que entiende lo común a casi todos los lenguajes (palabras clave, textos, números, comentarios).

use super::design::{self, icon, radius, sp, stroke};
use super::theme::Theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Margin, Sense, Stroke, Ui, pos2, vec2};
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

/// Texto del cuerpo: `small` en el panel, `body` en Vista rápida (`large`).
fn body_font(large: bool) -> FontId {
    if large { design::text::body() } else { design::text::small() }
}

/// Código: `mono` en el panel, `mono_l` en Vista rápida.
fn code_font(large: bool) -> FontId {
    if large { design::text::mono_l() } else { design::text::mono() }
}

/// La misma fuente en Medium (negrita de Markdown: el único peso extra).
fn medium(f: &FontId) -> FontId {
    FontId::new(f.size, design::family_medium())
}

struct Palette {
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    kind: Color32,
    text: Color32,
}

/// Los colores de sintaxis salen del tema (`t.syntax`: palabra clave, cadena, número, tipo).
fn palette(t: &Theme) -> Palette {
    let [keyword, string, number, kind] = t.syntax;
    Palette { keyword, string, number, comment: t.faint, kind, text: t.text }
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
pub fn code(ui: &mut Ui, t: &Theme, text: &str, large: bool) {
    let p = palette(t);
    let font = code_font(large);
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
fn inline_job(text: &str, t: &Theme, large: bool, color: Color32, base: FontId, wrap: f32) -> LayoutJob {
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
            "**" | "__" => (m, TextFormat::simple(medium(&base), color)),
            "`" => {
                let mut f = TextFormat::simple(code_font(large), t.text);
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

/// Hairline a lo ancho (bajo un título, `---`). En `border_strong`: el `border` no se ve sobre el
/// pozo de la vista previa.
fn rule(ui: &mut Ui, t: &Theme, width: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, stroke::HAIR), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(stroke::HAIR, t.border_strong));
}

/// Tabla: encabezado en Medium, hairline entre filas, 8 de margen a cada lado de cada celda. Ocupa
/// todo el ancho (lo que sobra va a la última columna); si no entra, se desplaza a lo ancho.
fn table(ui: &mut Ui, t: &Theme, header: &[String], rows: &[Vec<String>], large: bool, body: &FontId, id: usize) {
    let ncol = rows.iter().map(Vec::len).chain([header.len()]).max().unwrap_or(0);
    if ncol == 0 {
        return;
    }
    let p = ui.painter().clone();
    let cells = |r: &[String], font: &FontId| -> Vec<_> { (0..ncol).map(|c| p.layout_job(inline_job(r.get(c).map_or("", String::as_str), t, large, t.text, font.clone(), f32::INFINITY))).collect() };
    let grid: Vec<Vec<_>> = std::iter::once(cells(header, &medium(body))).chain(rows.iter().map(|r| cells(r, body))).collect();
    let pad = vec2(sp::S, sp::XS);
    let mut col_w: Vec<f32> = (0..ncol).map(|c| grid.iter().map(|r| r[c].size().x).fold(0.0, f32::max) + 2.0 * pad.x).collect();
    let row_h: Vec<f32> = grid.iter().map(|r| r.iter().map(|g| g.size().y).fold(0.0, f32::max) + 2.0 * pad.y).collect();
    let inner = ui.available_width() - 2.0 * stroke::HAIR;
    let total: f32 = col_w.iter().sum();
    if total < inner {
        col_w[ncol - 1] += inner - total;
    }
    let size = vec2(col_w.iter().sum(), row_h.iter().sum());
    let line = Stroke::new(stroke::HAIR, t.border_strong);
    egui::Frame::new().stroke(line).corner_radius(radius::M).show(ui, |ui| {
        // En el pozo del panel, sin barra (flotaba al pie antes de que se viera la tabla): se sigue
        // desplazando, y en Vista rápida (`large`) se ve entera.
        let vis = if large { egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded } else { egui::scroll_area::ScrollBarVisibility::AlwaysHidden };
        egui::ScrollArea::horizontal().id_salt(("md-table", id)).scroll_bar_visibility(vis).show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let p = ui.painter();
            let mut y = rect.top();
            for (k, (row, h)) in grid.into_iter().zip(&row_h).enumerate() {
                if k > 0 {
                    p.hline(rect.x_range(), y, line);
                }
                let mut x = rect.left();
                for (g, w) in row.into_iter().zip(&col_w) {
                    p.galley(pos2(x + pad.x, y + pad.y), g, t.text);
                    x += w;
                }
                y += h;
            }
        });
    });
}

/// Markdown: títulos, listas (y tareas), tablas, citas, bloques de código, separadores y
/// formato en línea. Se ajusta al ancho de `ui` (el de su `max_rect`, nunca uno infinito).
pub fn markdown(ui: &mut Ui, t: &Theme, text: &str, large: bool) {
    let width = ui.max_rect().width();
    let body = body_font(large);
    let lines: Vec<&str> = text.lines().take(3000).collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        // Bloque de código: todo junto, en un recuadro, con colores.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence = &trimmed[..3];
            let mut code_body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code_body.push(lines[i]);
                i += 1;
            }
            i += 1;
            ui.add_space(sp::XXS);
            // Hundido con borde fino: también se distingue sobre el pozo de la vista previa.
            let frame = egui::Frame::new().fill(t.sidebar).stroke(Stroke::new(stroke::HAIR, t.border_strong)).corner_radius(radius::M).inner_margin(Margin::symmetric(sp::M as i8, sp::S as i8));
            frame.show(ui, |ui| {
                ui.set_width(width - 2.0 * sp::M - 2.0 * stroke::HAIR);
                egui::ScrollArea::horizontal().id_salt(("md-code", i)).show(ui, |ui| code(ui, t, &code_body.join("\n"), large));
            });
            ui.add_space(sp::XS);
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
            ui.add_space(sp::XXS);
            table(ui, t, &header, &rows, large, &body, i);
            ui.add_space(sp::XS);
            continue;
        }
        i += 1;
        if trimmed.is_empty() {
            ui.add_space(sp::S);
            continue;
        }
        let level = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
            ui.add_space(if level <= 2 { sp::S } else { sp::XS });
            // Dentro de la escala: H1 y H2 en `title`, el resto en `strong`.
            let font = if level <= 2 { design::text::title() } else { design::text::strong() };
            ui.label(inline_job(trimmed[level..].trim(), t, large, t.text, font, width));
            if level <= 2 {
                ui.add_space(sp::XS);
                rule(ui, t, width);
            }
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            ui.add_space(sp::XS);
            rule(ui, t, width);
            ui.add_space(sp::XS);
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
                ui.add_space(indent + sp::XXS);
                // La marca en su columna; el texto se acomoda debajo de sí mismo.
                let (mark, color, rest) = match task {
                    Some((r, done)) => (if done { ic::CHECK_SQUARE } else { ic::SQUARE }.to_string(), if done { t.accent } else { t.dim }, r),
                    None => (mark, t.dim, rest),
                };
                let (mr, _) = ui.allocate_exact_size(vec2(icon::M, body.size + sp::XS), egui::Sense::hover());
                let font = if task.is_some() { design::icon_font(if large { icon::M } else { icon::S }) } else { body.clone() };
                ui.painter().text(mr.center(), egui::Align2::CENTER_CENTER, mark, font, color);
                let w = ui.available_width();
                let done = task.is_some_and(|t| t.1);
                let mut job = inline_job(rest, t, large, if done { t.dim } else { t.text }, body.clone(), w);
                if done {
                    for s in &mut job.sections {
                        s.format.strikethrough = Stroke::new(stroke::HAIR, t.dim);
                    }
                }
                // En una fila horizontal egui no parte el texto si no se le pide.
                ui.add(egui::Label::new(job).wrap());
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
            let margin = Margin { left: sp::M as i8, right: 0, top: sp::XXS as i8, bottom: sp::XXS as i8 };
            let r = egui::Frame::new().inner_margin(margin).show(ui, |ui| {
                ui.label(inline_job(&quote.join(" "), t, large, t.dim, body.clone(), width - sp::L));
            });
            // Barra neutra: el azul queda para lo que pasa «ahora».
            let rr = r.response.rect;
            let x = vec2(sp::XXS, 0.0);
            ui.painter().line_segment([rr.left_top() + x, rr.left_bottom() + x], Stroke::new(stroke::MARK, t.border_strong));
            continue;
        }
        ui.label(inline_job(trimmed, t, large, t.text, body.clone(), width));
    }
}
