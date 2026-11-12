//! Texto con formato para la vista previa: Markdown (títulos, listas, tareas, tablas, citas,
//! bloques de código) y código con colores de sintaxis. Sin dependencias: un resaltador simple
//! que entiende lo común a casi todos los lenguajes (palabras clave, textos, números, comentarios).

use super::design::{self, icon, radius, sp, stroke};
use super::theme::Theme;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Margin, Rect, Sense, Stroke, Ui, pos2, vec2};
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
    code_from(ui, t, text.lines(), large, false);
}

/// `code` de unas líneas, empezando (o no) dentro de un comentario /* … */ que abrió un trozo
/// anterior.
fn code_from<'a>(ui: &mut Ui, t: &Theme, lines: impl Iterator<Item = &'a str>, large: bool, comment: bool) {
    let p = palette(t);
    let font = code_font(large);
    let mut job = LayoutJob::default();
    let mut block = comment;
    for (k, line) in lines.enumerate() {
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
fn markdown_lines(ui: &mut Ui, t: &Theme, lines: &[&str], large: bool, base: usize) {
    let width = ui.max_rect().width();
    let body = body_font(large);
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
                egui::ScrollArea::horizontal().id_salt(("md-code", base + i)).show(ui, |ui| code(ui, t, &code_body.join("\n"), large));
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
            table(ui, t, &header, &rows, large, &body, base + i);
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

// ---------------------------------------------------------------- documento largo

/// Cómo se dibuja un documento.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Style {
    Markdown,
    Code,
    Plain,
}

impl Style {
    pub fn of(ext: &str) -> Style {
        if matches!(ext, "md" | "markdown" | "mdx") {
            Style::Markdown
        } else if is_code(ext) {
            Style::Code
        } else {
            Style::Plain
        }
    }
}

/// Líneas por trozo en código y texto plano: lo que se arma de una vez cuando entra a la vista.
const CHUNK: usize = 24;
/// Tiempo por cuadro para medir trozos fuera de la vista (que la barra no salte después). Poco:
/// el panel y la Vista rápida pueden estar midiendo a la vez, y el cuadro entero tiene 4 ms.
const MEASURE_BUDGET: std::time::Duration = std::time::Duration::from_micros(600);

/// Un trozo del documento: líneas `start..end`; `comment`, si empieza dentro de /* … */.
struct Block {
    start: usize,
    end: usize,
    comment: bool,
}

/// Lo que se sabe de un documento: dónde empieza cada línea, sus trozos y el alto de cada uno
/// (estimado hasta que se mide de verdad, con el ancho `width`).
struct Doc {
    key: (u64, Style, bool),
    lines: Vec<(usize, usize)>,
    blocks: Vec<Block>,
    width: f32,
    heights: Vec<f32>,
    measured: Vec<bool>,
    /// Lo más ancho medido (el código se desplaza a lo ancho).
    wide: f32,
}

/// Lo que agrega cada línea al alto de un texto en `font` (lo que mide egui, no `row_height`).
fn line_height(f: &mut egui::epaint::FontsView<'_>, font: &FontId) -> f32 {
    let one = f.layout_no_wrap("x".into(), font.clone(), Color32::WHITE).size().y;
    let two = f.layout_no_wrap("x\nx".into(), font.clone(), Color32::WHITE).size().y;
    two - one
}

type Shared = std::sync::Arc<parking_lot::Mutex<Doc>>;

fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(text.len() / 32 + 1);
    let mut start = 0;
    let b = text.as_bytes();
    for nl in memchr::memchr_iter(b'\n', b) {
        let end = if nl > start && b[nl - 1] == b'\r' { nl - 1 } else { nl };
        out.push((start, end));
        start = nl + 1;
    }
    if start < b.len() {
        out.push((start, b.len()));
    }
    out
}

/// Si al terminar la línea se sigue dentro de un comentario /* … */ (lo mismo que `code_line`,
/// sin armar nada).
fn comment_after(line: &str, mut block: bool) -> bool {
    // Casi todas las líneas: sin «/*» (o, dentro de un comentario, sin «*/») nada cambia.
    if !block && !line.contains("/*") || block && !line.contains("*/") {
        return block;
    }
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // Nunca en medio de una letra de varios bytes (ñ, 漢).
        if !line.is_char_boundary(i) {
            i += 1;
            continue;
        }
        if block {
            match line[i..].find("*/") {
                Some(k) => {
                    block = false;
                    i += k + 2;
                }
                None => return true,
            }
            continue;
        }
        let rest = &line[i..];
        if rest.starts_with("//") || rest.starts_with('#') && !rest.starts_with("#[") && !rest.starts_with("#!") || rest.starts_with("--") || rest.starts_with("<!--") {
            return false;
        }
        if rest.starts_with("/*") {
            block = true;
            i += 2;
            continue;
        }
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            if c == b'\'' && (j + 1).min(b.len()) - i > 4 && !line[i + 1..].contains('\'') {
                i += 1;
            } else {
                i = (j + 1).min(b.len());
            }
            continue;
        }
        i += 1;
    }
    block
}

/// Trozos de Markdown: los mismos elementos que arma `markdown_lines` (un bloque de código,
/// una tabla o una cita van enteros; lo demás, línea por línea).
fn md_blocks(text: &str, lines: &[(usize, usize)]) -> Vec<Block> {
    let line = |i: usize| text[lines[i].0..lines[i].1].trim_start();
    let n = lines.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let l = line(i);
        let end = if l.starts_with("```") || l.starts_with("~~~") {
            let fence = &l[..3];
            let mut j = i + 1;
            while j < n && !line(j).starts_with(fence) {
                j += 1;
            }
            (j + 1).min(n)
        } else if l.starts_with('|') && i + 1 < n && is_table_rule(line(i + 1)) {
            let mut j = i + 2;
            while j < n && line(j).starts_with('|') {
                j += 1;
            }
            j
        } else if l.starts_with('>') {
            let mut j = i + 1;
            while j < n && line(j).starts_with('>') {
                j += 1;
            }
            j
        } else {
            i + 1
        };
        out.push(Block { start: i, end, comment: false });
        i = end;
    }
    out
}

impl Doc {
    fn new(text: &str, style: Style, key: (u64, Style, bool)) -> Doc {
        let lines = line_ranges(text);
        let blocks = match style {
            Style::Markdown => md_blocks(text, &lines),
            Style::Code | Style::Plain => {
                let mut out = Vec::with_capacity(lines.len() / CHUNK + 1);
                let mut comment = false;
                let mut i = 0;
                while i < lines.len() {
                    let end = (i + CHUNK).min(lines.len());
                    out.push(Block { start: i, end, comment });
                    if style == Style::Code {
                        for l in &lines[i..end] {
                            comment = comment_after(&text[l.0..l.1], comment);
                        }
                    }
                    i = end;
                }
                out
            }
        };
        let n = blocks.len();
        Doc { key, lines, blocks, width: 0.0, heights: vec![0.0; n], measured: vec![false; n], wide: 0.0 }
    }

    /// Alto aproximado de un trozo sin medirlo: líneas (partidas por el ancho) por alto de renglón.
    fn estimate(&self, text: &str, k: usize, style: Style, row: f32, mono_row: f32, char_w: f32) -> f32 {
        let b = &self.blocks[k];
        let per_row = (self.width / char_w).max(8.0);
        // Por bytes, no por letras (contarlas en 500 KB es medio milisegundo): con acentos estima
        // un poco de más, y medir lo corrige.
        let wrapped = |a: usize, z: usize| -> f32 { self.lines[a..z].iter().map(|l| (((l.1 - l.0) as f32) / per_row).ceil().max(1.0)).sum() };
        let n = (b.end - b.start) as f32;
        match style {
            Style::Code => n * mono_row,
            Style::Plain => wrapped(b.start, b.end) * mono_row,
            Style::Markdown => {
                let first = text[self.lines[b.start].0..self.lines[b.start].1].trim_start();
                if first.is_empty() {
                    sp::S
                } else if first.starts_with("```") || first.starts_with("~~~") {
                    (n - 2.0).max(0.0) * mono_row + 2.0 * sp::S + sp::XXS + sp::XS
                } else if first.starts_with('|') {
                    (n - 1.0) * (row + 2.0 * sp::XS) + sp::XXS + sp::XS
                } else if first.starts_with('#') {
                    row * 1.6 + sp::S + sp::XS
                } else {
                    wrapped(b.start, b.end) * row
                }
            }
        }
    }
}

/// Dibuja un trozo en `ui`.
fn draw_block(ui: &mut Ui, t: &Theme, text: &str, doc: &Doc, k: usize, style: Style, large: bool) {
    let b = &doc.blocks[k];
    let (from, to) = (doc.lines[b.start].0, doc.lines[b.end - 1].1);
    match style {
        Style::Markdown => {
            let lines: Vec<&str> = doc.lines[b.start..b.end].iter().map(|l| &text[l.0..l.1]).collect();
            markdown_lines(ui, t, &lines, large, b.start);
        }
        // Línea por línea: una vacía al final del trozo también es una fila.
        Style::Code => code_from(ui, t, doc.lines[b.start..b.end].iter().map(|l| &text[l.0..l.1]), large, b.comment),
        Style::Plain => {
            let font = if large { design::text::mono_l() } else { design::text::mono() };
            ui.add(egui::Label::new(egui::RichText::new(&text[from..to]).font(font).color(t.text)).wrap());
        }
    }
}

/// Un documento largo (Markdown, código o texto) a lo ancho de `ui`, dentro de un `ScrollArea`.
/// Solo se arma y se dibuja lo que está a la vista: un archivo de 500 KB cuesta por cuadro lo
/// mismo que uno de 5. Lo que no se ve se mide de a poco (≈1,5 ms por cuadro, empezando por lo
/// más cercano) para que la barra no cambie de tamaño al desplazarse; si un trozo de arriba
/// resulta de otro alto, el desplazamiento se corrige y lo que se está leyendo no salta.
pub fn document(ui: &mut Ui, t: &Theme, text: &str, ext: &str, large: bool) {
    let _p = super::perf::scope("documento");
    let style = Style::of(ext);
    let key = (xxhash_rust::xxh3::xxh3_64(text.as_bytes()), style, large);
    let id = ui.id().with("doc");
    let mut fresh = false;
    let shared: Shared = match ui.data(|d| d.get_temp::<Shared>(id)).filter(|d| d.lock().key == key) {
        Some(d) => d,
        None => {
            let _p = super::perf::scope("documento.nuevo");
            fresh = true;
            // Un texto en chino o japonés en una carpeta con nombres ASCII: que se cargue la fuente.
            // (Solo si hay letras de 3 bytes: las de chino, japonés y coreano lo son; recorrer
            // 500 KB de texto con acentos letra por letra es medio milisegundo.)
            if text.bytes().any(|b| (0xE1..=0xEF).contains(&b)) {
                super::theme::want_cjk(text);
            }
            let d: Shared = std::sync::Arc::new(parking_lot::Mutex::new(Doc::new(text, style, key)));
            ui.data_mut(|m| m.insert_temp(id, d.clone()));
            d
        }
    };
    let mut doc = shared.lock();
    if doc.blocks.is_empty() {
        return;
    }
    let width = ui.available_width().max(1.0);
    let body = body_font(large);
    let mono = code_font(large);
    let (row, mono_row, char_w) = ui.fonts_mut(|f| (f.row_height(&body), line_height(f, &mono), f.glyph_width(&mono, 'n').max(1.0)));
    let gap = ui.spacing().item_spacing.y;
    // Otro ancho: las medidas ya no valen (se miden de nuevo; mientras, sirven de estimación).
    if (doc.width - width).abs() > 0.5 {
        let _p = super::perf::scope("documento.ancho");
        let first = doc.width == 0.0;
        doc.width = width;
        // El código no se parte: su alto se sabe sin armarlo (una fila por línea; egui da a cada
        // fila el alto de la fuente pedida, aunque una letra salga de otra). No hay nada que medir.
        // El ancho (para desplazarse a lo ancho) es el de la línea más larga en una letra de ancho
        // fijo, las de chino y japonés dobles; al dibujarse cada trozo se corrige.
        let exact = style == Style::Code;
        doc.measured.iter_mut().for_each(|m| *m = exact);
        if first || exact {
            for k in 0..doc.blocks.len() {
                doc.heights[k] = doc.estimate(text, k, style, row, mono_row, char_w);
            }
        }
        if exact {
            let longest = doc.lines.iter().map(|l| text[l.0..l.1].chars().map(|c| if c == '\t' { 4 } else if c >= '\u{1100}' { 2 } else { 1 }).sum::<usize>()).max().unwrap_or(0);
            doc.wide = doc.wide.max(longest as f32 * char_w);
        }
    }
    let top = ui.cursor().min;
    let clip = ui.clip_rect();
    let n = doc.blocks.len();
    // Dónde empieza cada trozo.
    let mut tops = Vec::with_capacity(n + 1);
    let mut y = 0.0;
    for h in &doc.heights {
        tops.push(y);
        y += h + gap;
    }
    let total = (y - gap).max(0.0);
    let (vis_a, vis_b) = (clip.top() - top.y, clip.bottom() - top.y);
    let first_vis = tops.partition_point(|&y0| y0 <= vis_a).saturating_sub(1);
    // Lo visible: se dibuja (y de paso se mide).
    let vis_scope = super::perf::scope("documento.visible");
    let t_vis = std::time::Instant::now();
    let mut k = first_vis;
    while k < n && tops[k] <= vis_b {
        if tops[k] + doc.heights[k] >= vis_a {
            let r = Rect::from_min_size(pos2(top.x, top.y + tops[k]), vec2(width, f32::INFINITY));
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("blk", k)).max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
            draw_block(&mut child, t, text, &doc, k, style, large);
            let m = child.min_rect();
            doc.heights[k] = m.height();
            doc.wide = doc.wide.max(m.width());
            doc.measured[k] = true;
        }
        k += 1;
    }
    drop(vis_scope);
    // Lo que no se ve: se mide, lo más cercano primero (hacia los dos lados de lo visible),
    // hasta agotar el tiempo del cuadro.
    let _p = super::perf::scope("documento.medir");
    let t0 = std::time::Instant::now();
    let mut left = doc.measured.iter().filter(|m| !**m).count();
    // Un cuadro que ya fue caro (el documento recién abierto, mucho nuevo a la vista) no mide:
    // sigue el próximo.
    if left > 0 && (fresh || t_vis.elapsed() > MEASURE_BUDGET) {
        ui.ctx().request_repaint();
    } else if left > 0 {
        let mut shift = 0.0;
        let near = (0..n).flat_map(|d| [first_vis.checked_add(d), first_vis.checked_sub(d).filter(|_| d > 0)]).flatten().filter(|&k| k < n);
        for k in near {
            if left == 0 || t0.elapsed() > MEASURE_BUDGET {
                break;
            }
            if doc.measured[k] {
                continue;
            }
            let r = Rect::from_min_size(pos2(top.x, top.y + tops[k]), vec2(width, f32::INFINITY));
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("blk", k)).max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)).invisible());
            draw_block(&mut child, t, text, &doc, k, style, large);
            let m = child.min_rect();
            if k < first_vis {
                shift += m.height() - doc.heights[k];
            }
            doc.heights[k] = m.height();
            doc.wide = doc.wide.max(m.width());
            doc.measured[k] = true;
            left -= 1;
        }
        // Lo de arriba cambió de alto: correr el desplazamiento lo mismo, y lo que se lee queda quieto.
        if shift.abs() > 0.1 && vis_a > 0.0 {
            ui.scroll_with_delta_animation(vec2(0.0, -shift), egui::style::ScrollAnimation::none());
        }
        if left > 0 {
            ui.ctx().request_repaint();
        }
    }
    let w = if style == Style::Code { width.max(doc.wide) } else { width };
    ui.allocate_rect(Rect::from_min_size(top, vec2(w, total)), Sense::hover());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, t: &Theme, text: &str, ext: &str, offset: Option<f32>) -> (std::time::Duration, f32) {
        let mut out = (std::time::Duration::ZERO, 0.0);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 1000.0))), ..Default::default() };
        let t0 = std::time::Instant::now();
        let mut full = ctx.run_ui(input, |ui| {
            let mut area = egui::ScrollArea::vertical().id_salt("doc-test").auto_shrink([false, false]);
            if let Some(o) = offset {
                area = area.vertical_scroll_offset(o);
            }
            let r = area.show(ui, |ui| document(ui, t, text, ext, true));
            out.1 = r.content_size.y;
        });
        full.textures_delta.clear();
        let _ = ctx.tessellate(full.shapes, 1.0);
        out.0 = t0.elapsed();
        out
    }

    /// Un documento de ~500 KB: cada cuadro arma solo lo visible.
    #[test]
    fn long_documents_draw_only_what_is_visible() {
        let t = Theme::dark();
        let mut md = String::new();
        let mut rs = String::new();
        let mut txt = String::new();
        for k in 0..6000 {
            md.push_str(&format!("## Título {k}\n\nUn párrafo con **negrita**, `código` y [un enlace](https://x.y) número {k}.\n\n- elemento {k}\n- [x] tarea\n\n```rust\nfn f{k}() {{ /* nota */ }}\n```\n\n| a | b |\n|---|---|\n| {k} | dos |\n\n"));
            rs.push_str(&format!("/* comentario\n   largo {k} */\nfn funcion_{k}(x: u32) -> u32 {{ x + {k} }} // fin\n"));
            txt.push_str(&format!("línea {k} de un registro bastante largo para que tenga que partirse en dos renglones en una ventana angosta de prueba\n"));
        }
        for (text, ext) in [(&md, "md"), (&rs, "rs"), (&txt, "log")] {
            assert!(text.len() > 400_000);
            let ctx = egui::Context::default();
            super::super::theme::setup_fonts(&ctx);
            let mut worst = std::time::Duration::ZERO;
            let mut size = 0.0;
            // Abrir, esperar a que todo esté medido y saltar al medio y al final.
            for f in 0..400 {
                let offset = match f {
                    150 => Some(size / 2.0),
                    250 => Some(size),
                    _ => None,
                };
                let (dt, s) = frame(&ctx, &t, text, ext, offset);
                if f == 0 {
                    eprintln!("{ext}: primer cuadro {dt:?}");
                }
                if f > 0 {
                    worst = worst.max(dt);
                }
                size = s;
            }
            let (steady, _) = frame(&ctx, &t, text, ext, None);
            eprintln!("{ext}: alto {size:.0}, peor cuadro {worst:?}, cuadro quieto {steady:?}");
            assert!(size > 50_000.0, "{ext}: el documento entero cuenta para la barra");
            // Los servidores de CI son lentos y compartidos: ahí el tope es 4 veces más holgado,
            // que igual atrapa el error de fondo (armar el documento entero: 60–80 ms por cuadro).
            let slack = if std::env::var_os("CI").is_some() { 4 } else { 1 };
            let ms = |n: u64| std::time::Duration::from_millis(n * slack);
            assert!(worst < ms(6), "{ext}: el peor cuadro tarda {worst:?}");
            assert!(steady < ms(4), "{ext}: un cuadro quieto tarda {steady:?}");
        }
    }

    /// El alto del código se sabe sin armarlo (no se mide): tiene que ser el de verdad.
    #[test]
    fn code_height_is_exact_without_layout() {
        let t = Theme::dark();
        let text = "fn a() {}\n\n\tlet x = \"cadena\"; // nota\n/* abre\n cierra */ ñandú 漢字 😀\n\n".repeat(30);
        let ctx = egui::Context::default();
        super::super::theme::setup_fonts(&ctx);
        for large in [false, true] {
            let doc = Doc::new(&text, Style::Code, (0, Style::Code, large));
            let mut out = Vec::new();
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 1000.0))), ..Default::default() };
            let mut full = ctx.run_ui(input, |ui| {
                let mono = code_font(large);
                let (row, mono_row, char_w) = ui.fonts_mut(|f| (f.row_height(&body_font(large)), line_height(f, &mono), f.glyph_width(&mono, 'n')));
                for k in 0..doc.blocks.len() {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, f32::INFINITY))));
                    draw_block(&mut child, &t, &text, &doc, k, Style::Code, large);
                    out.push((child.min_rect().height(), doc.estimate(&text, k, Style::Code, row, mono_row, char_w)));
                }
            });
            full.textures_delta.clear();
            assert!(!out.is_empty());
            for (real, est) in out {
                assert!((real - est).abs() < 0.01, "alto real {real}, estimado {est}");
            }
        }
    }

    #[test]
    fn block_comments_carry_across_chunks() {
        assert!(comment_after("let a = 1; /* abre", false));
        assert!(comment_after("sigue", true));
        assert!(!comment_after("cierra */ let b = 2;", true));
        assert!(!comment_after("let s = \"/* no es comentario\";", false));
        assert!(!comment_after("// /* tampoco", false));
        // Letras de varios bytes en medio: no se corta una letra.
        assert!(comment_after("ñandú /* 漢字 'ñ' \"á\" abre", false));
        let text = "# T\n\n```\ncódigo\n\n```\n| a | b |\n|---|---|\n| 1 | 2 |\n> cita\n> sigue\nfin";
        let lines = line_ranges(text);
        let blocks: Vec<(usize, usize)> = md_blocks(text, &lines).iter().map(|b| (b.start, b.end)).collect();
        assert_eq!(blocks, [(0, 1), (1, 2), (2, 6), (6, 9), (9, 11), (11, 12)]);
    }
}
