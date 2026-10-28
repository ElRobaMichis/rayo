//! Motor de búsqueda.
//!
//! Para generar candidatos se hace un solo barrido SIMD (`memchr::memmem`)
//! sobre el buffer contiguo de nombres, partido en trozos que se procesan en
//! paralelo. Cada coincidencia se traduce a su entrada con búsqueda binaria,
//! se filtra y se le asigna un puntaje.

use super::store::*;
use memchr::memmem::Finder;
use rayon::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Términos que deben aparecer en el nombre (en minúsculas).
    pub terms: Vec<Vec<u8>>,
    /// Términos que NO deben aparecer (`!texto`).
    pub neg: Vec<Vec<u8>>,
    /// Comodines `*` y `?` sobre el nombre completo.
    pub globs: Vec<Vec<u8>>,
    /// Términos con `\`: se buscan en la ruta completa.
    pub path_terms: Vec<Vec<u8>>,
    pub exts: Vec<Vec<u8>>,
    pub size_min: Option<u64>,
    pub size_max: Option<u64>,
    pub mtime_min: Option<u32>,
    pub mtime_max: Option<u32>,
    pub only_files: bool,
    pub only_dirs: bool,
    /// Rutas que NO deben aparecer (para "recientes": AppData, Windows, cachés…).
    pub neg_paths: Vec<Vec<u8>>,
    /// Sin archivos ocultos ni de sistema.
    pub no_hidden: bool,
}

/// Carpetas que llenan "recientes" de ruido (cachés, sistema, compilaciones).
const NOISE: &[&str] = &[
    "\\appdata\\", "\\windows\\", "\\program files", "\\programdata\\", "\\$recycle.bin", "\\$windows.~", "\\system volume information",
    "\\windowsapps\\", "\\.git\\", "\\node_modules\\", "\\target\\", "\\__pycache__\\", "\\.cache\\", "\\cache\\", "\\temp\\", "\\tmp\\",
    "\\.vscode\\", "\\.idea\\", "\\.cargo\\", "\\.rustup\\", "\\.gradle\\", "\\.npm\\", "\\.nuget\\", "\\.venv\\", "\\venv\\", "\\site-packages\\",
    "\\.m2\\", "\\obj\\", "\\build\\intermediates", "\\.android\\", "\\.dotnet\\", "\\steamapps\\", "\\onedrivetemp\\", "\\.local\\",
];

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.globs.is_empty()
            && self.path_terms.is_empty()
            && self.exts.is_empty()
            && self.size_min.is_none()
            && self.size_max.is_none()
            && self.mtime_min.is_none()
            && self.mtime_max.is_none()
            && !self.only_dirs
            && !self.only_files
            && self.neg_paths.is_empty()
    }
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let idx = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(idx);
    let n: f64 = num.parse().ok()?;
    let mul = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((n * mul) as u64)
}

pub fn now_unix() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

/// Inicio del día local aproximado (usa la zona horaria del sistema).
fn local_day_start(now: u32) -> u32 {
    let offset = local_utc_offset_secs();
    let local = now as i64 + offset;
    (local - local.rem_euclid(86_400) - offset) as u32
}

pub fn local_utc_offset_secs() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    let mut bias = tz.Bias as i64;
    if r == 2 {
        bias += tz.DaylightBias as i64;
    }
    -bias * 60
}

fn parse_date_range(v: &str, now: u32) -> Option<(Option<u32>, Option<u32>)> {
    let day = 86_400u32;
    let today = local_day_start(now);
    let v = v.to_lowercase();
    Some(match v.as_str() {
        "today" | "hoy" => (Some(today), None),
        "yesterday" | "ayer" => (Some(today - day), Some(today)),
        "thisweek" | "week" | "semana" | "estasemana" => (Some(now - 7 * day), None),
        "thismonth" | "month" | "mes" | "estemes" => (Some(now - 30 * day), None),
        "thisyear" | "year" | "año" | "ano" | "esteaño" => (Some(now - 365 * day), None),
        _ => {
            let (num, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit())?);
            let n: u32 = num.parse().ok()?;
            let secs = match unit {
                "m" | "min" => 60,
                "h" => 3600,
                "d" => day,
                "w" | "s" => 7 * day,
                "mo" => 30 * day,
                "y" | "a" => 365 * day,
                _ => return None,
            };
            (Some(now.saturating_sub(n * secs)), None)
        }
    })
}

fn type_exts(t: &str) -> Option<&'static [&'static str]> {
    Some(match t {
        "img" | "image" | "imagen" | "pic" | "foto" => {
            &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif", "svg", "ico", "raw", "cr2", "nef", "arw", "dng", "psd"]
        }
        "video" | "vid" => &["mp4", "mkv", "avi", "mov", "wmv", "webm", "flv", "m4v", "mpg", "mpeg", "3gp", "ts"],
        "audio" | "music" | "musica" | "música" => &["mp3", "flac", "wav", "aac", "ogg", "m4a", "wma", "opus", "aiff", "mid"],
        "doc" | "docs" | "documento" => &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "rtf", "md", "csv", "epub"],
        "zip" | "archive" | "comprimido" => &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "iso", "cab"],
        "exe" | "app" | "programa" => &["exe", "msi", "bat", "cmd", "ps1", "lnk", "appx", "msix"],
        "code" | "codigo" | "código" => &["rs", "c", "cpp", "h", "hpp", "cs", "java", "kt", "py", "js", "ts", "jsx", "tsx", "go", "rb", "php", "swift", "lua", "html", "css", "json", "toml", "yaml", "yml", "xml", "sql", "sh"],
        _ => return None,
    })
}

pub fn parse_query(input: &str) -> Query {
    let mut q = Query::default();
    let now = now_unix();
    for tok in tokenize(input) {
        let lower = tok.to_lowercase();
        if let Some((key, val)) = lower.split_once(':') {
            if key.len() > 1 && !val.is_empty() || matches!(key, "file" | "folder" | "archivo" | "carpeta" | "dir" | "recientes" | "recent" | "reciente") {
                match key {
                    // Lo que se modificó en los últimos días, sin el ruido del sistema.
                    "recientes" | "recent" | "reciente" => {
                        q.only_files = true;
                        q.no_hidden = true;
                        q.mtime_min = parse_date_range(if val.is_empty() { "7d" } else { val }, now).and_then(|r| r.0);
                        q.neg_paths.extend(NOISE.iter().map(|n| n.as_bytes().to_vec()));
                        continue;
                    }
                    "ext" => {
                        q.exts.extend(val.split([';', ',']).filter(|e| !e.is_empty()).map(|e| search_key(e.trim_start_matches('.')).into_bytes()));
                        continue;
                    }
                    "type" | "tipo" => {
                        if let Some(list) = type_exts(val) {
                            q.exts.extend(list.iter().map(|e| e.as_bytes().to_vec()));
                        }
                        continue;
                    }
                    "size" | "tamaño" | "tam" => {
                        let (op, num) = if let Some(r) = val.strip_prefix(">=") {
                            (">", r)
                        } else if let Some(r) = val.strip_prefix("<=") {
                            ("<", r)
                        } else if let Some(r) = val.strip_prefix('>') {
                            (">", r)
                        } else if let Some(r) = val.strip_prefix('<') {
                            ("<", r)
                        } else if let Some((a, b)) = val.split_once('-') {
                            q.size_min = parse_size(a);
                            q.size_max = parse_size(b);
                            continue;
                        } else {
                            (">", val)
                        };
                        if let Some(n) = parse_size(num) {
                            if op == ">" { q.size_min = Some(n) } else { q.size_max = Some(n) }
                        }
                        continue;
                    }
                    "dm" | "date" | "fecha" | "modified" => {
                        if let Some((a, b)) = parse_date_range(val, now) {
                            q.mtime_min = a;
                            q.mtime_max = b;
                        }
                        continue;
                    }
                    "file" | "archivo" => {
                        q.only_files = true;
                        if !val.is_empty() {
                            q.terms.push(search_key(val).into_bytes());
                        }
                        continue;
                    }
                    "folder" | "carpeta" | "dir" => {
                        q.only_dirs = true;
                        if !val.is_empty() {
                            q.terms.push(search_key(val).into_bytes());
                        }
                        continue;
                    }
                    _ => {}
                }
            }
        }
        let folded = search_key(&tok).into_bytes();
        if let Some(rest) = folded.strip_prefix(b"!") {
            if !rest.is_empty() {
                q.neg.push(rest.to_vec());
            }
        } else if folded.contains(&b'*') || folded.contains(&b'?') {
            q.globs.push(folded);
        } else if folded.contains(&b'\\') {
            q.path_terms.push(folded);
        } else if !folded.is_empty() {
            q.terms.push(folded);
        }
    }
    q
}

/// Texto fijo más largo de un comodín, anclado con el separador `\0` si toca el
/// inicio o el final del nombre (p. ej. `*.mp4` se busca como `.mp4` + separador).
fn glob_literal(g: &[u8]) -> Option<Vec<u8>> {
    let mut best: Option<(usize, usize)> = None;
    let mut start = 0;
    for i in 0..=g.len() {
        if i == g.len() || g[i] == b'*' || g[i] == b'?' {
            if i > start && best.is_none_or(|(a, b)| i - start > b - a) {
                best = Some((start, i));
            }
            start = i + 1;
        }
    }
    let (a, b) = best?;
    let mut p = Vec::with_capacity(b - a + 2);
    if a == 0 {
        p.push(0);
    }
    p.extend_from_slice(&g[a..b]);
    if b == g.len() {
        p.push(0);
    }
    Some(p)
}

/// Comodines: `*` = cualquier secuencia, `?` = un carácter.
pub fn glob_match(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while i < s.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == s[i]) {
            if pat[p] == b'?' {
                // Avanzar un carácter UTF-8 completo.
                i += utf8_len(s[i]);
            } else {
                i += 1;
            }
            p += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = p;
            mark = i;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

#[inline]
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Hit {
    pub vol: u8,
    pub id: u32,
    pub score: i32,
}

/// Patrón con el que se barre el buffer de nombres.
enum Primary<'q> {
    /// Un texto (SIMD con memmem). `adj` = 1 si el patrón empieza con el separador.
    One(Finder<'q>, u32),
    /// Pocos textos (p. ej. `ext:jpg;png`): una pasada SIMD por cada uno.
    Several(Vec<Finder<'q>>),
    /// Muchos textos (p. ej. `tipo:imagen`): una sola pasada con Aho-Corasick.
    Many(aho_corasick::AhoCorasick),
}

pub struct Compiled<'q> {
    q: &'q Query,
    terms: Vec<Finder<'q>>,
    neg: Vec<Finder<'q>>,
    path: Vec<Finder<'q>>,
    /// Patrón principal para el barrido.
    primary: Option<Primary<'q>>,
    primary_is_term: Option<usize>,
    ext_pats: Vec<Vec<u8>>,
    now: u32,
    /// Restringe a descendientes de esta entrada.
    pub within: Option<u32>,
}

impl<'q> Compiled<'q> {
    pub fn new(q: &'q Query) -> Self {
        let terms: Vec<Finder> = q.terms.iter().map(|t| Finder::new(t)).collect();
        let ext_pats: Vec<Vec<u8>> = q
            .exts
            .iter()
            .map(|e| {
                let mut p = vec![b'.'];
                p.extend_from_slice(e);
                p
            })
            .collect();
        let mut primary = None;
        let mut primary_is_term = None;
        if let Some((i, t)) = q.terms.iter().enumerate().max_by_key(|(_, t)| t.len()) {
            primary = Some(Primary::One(Finder::new(t.as_slice()), 0));
            primary_is_term = Some(i);
        } else if !ext_pats.is_empty() {
            // `ext:pdf;docx` sin texto: se barre buscando ".pdf" + separador, ".docx" + separador.
            let pats: Vec<Vec<u8>> = ext_pats
                .iter()
                .map(|e| {
                    let mut p = e.clone();
                    p.push(0);
                    p
                })
                .collect();
            primary = if pats.len() == 1 {
                Some(Primary::One(Finder::new(&pats[0]).into_owned(), 0))
            } else if pats.len() <= 6 {
                Some(Primary::Several(pats.iter().map(|p| Finder::new(p).into_owned()).collect()))
            } else {
                aho_corasick::AhoCorasick::builder()
                    .match_kind(aho_corasick::MatchKind::LeftmostFirst)
                    .build(&pats)
                    .ok()
                    .map(Primary::Many)
            };
        } else if let Some(p) = q.globs.iter().filter_map(|g| glob_literal(g)).max_by_key(|p| p.iter().filter(|&&b| b != 0).count()) {
            // `*.mp4`: se barre buscando el texto fijo y luego se verifica el comodín.
            let adj = u32::from(p.first() == Some(&0));
            primary = Some(Primary::One(Finder::new(&p).into_owned(), adj));
        }
        Compiled {
            q,
            terms,
            neg: q.neg.iter().map(|t| Finder::new(t)).collect(),
            path: q.path_terms.iter().map(|t| Finder::new(t)).collect(),
            primary,
            primary_is_term,
            ext_pats,
            now: now_unix(),
            within: None,
        }
    }

    fn lower_path(v: &VolumeIndex, i: u32) -> Vec<u8> {
        let mut chain = Vec::with_capacity(16);
        let mut cur = i;
        while cur != NONE && chain.len() < 512 {
            chain.push(cur);
            cur = v.parent[cur as usize];
        }
        let mut out = Vec::with_capacity(chain.len() * 16);
        for (k, &c) in chain.iter().rev().enumerate() {
            if k > 0 {
                out.push(b'\\');
            }
            out.extend_from_slice(v.name_lower(c));
        }
        out
    }

    /// Evalúa una entrada; devuelve el puntaje si cumple todos los filtros.
    #[inline]
    pub fn eval(&self, v: &VolumeIndex, i: u32, skip_term: Option<usize>) -> Option<i32> {
        self.eval_at(v, i, skip_term, None)
    }

    /// Como `eval`, pero con la posición (dentro del nombre) donde el barrido ya
    /// encontró el primer término, para no volver a buscarlo al dar el puntaje.
    #[inline]
    pub fn eval_at(&self, v: &VolumeIndex, i: u32, skip_term: Option<usize>, first_pos: Option<usize>) -> Option<i32> {
        let iu = i as usize;
        let f = v.flags[iu];
        if f & F_DELETED != 0 || i == 0 {
            return None;
        }
        let q = self.q;
        let is_dir = f & F_DIR != 0;
        if (q.only_dirs && !is_dir) || (q.only_files && is_dir) {
            return None;
        }
        let name = v.name_lower(i);
        for (k, t) in self.terms.iter().enumerate() {
            if Some(k) != skip_term && t.find(name).is_none() {
                return None;
            }
        }
        for t in &self.neg {
            if t.find(name).is_some() {
                return None;
            }
        }
        for g in &q.globs {
            if !glob_match(g, name) {
                return None;
            }
        }
        if !self.ext_pats.is_empty() && (is_dir || !self.ext_pats.iter().any(|e| name.ends_with(e))) {
            return None;
        }
        let size = v.size[iu];
        if q.size_min.is_some_and(|m| size < m) || q.size_max.is_some_and(|m| size > m) {
            return None;
        }
        let mt = v.mtime[iu];
        if q.mtime_min.is_some_and(|m| mt < m) || q.mtime_max.is_some_and(|m| mt >= m) {
            return None;
        }
        if q.no_hidden && f & (F_HIDDEN | F_SYSTEM) != 0 {
            return None;
        }
        if !self.path.is_empty() || !q.neg_paths.is_empty() {
            let p = Self::lower_path(v, i);
            if self.path.iter().any(|t| t.find(&p).is_none()) {
                return None;
            }
            if q.neg_paths.iter().any(|n| memchr::memmem::find(&p, n).is_some()) {
                return None;
            }
            // Carpetas y archivos ocultos por nombre (".config", "~$temporal.docx").
            if q.no_hidden && p.split(|&b| b == b'\\').skip(1).any(|c| matches!(c.first(), Some(b'.' | b'$' | b'~'))) {
                return None;
            }
        }
        let depth = v.depth(i);
        if let Some(w) = self.within {
            let mut cur = v.parent[iu];
            let mut ok = false;
            while cur != NONE {
                if cur == w {
                    ok = true;
                    break;
                }
                cur = v.parent[cur as usize];
            }
            if !ok {
                return None;
            }
        }
        Some(self.score(name, f, mt, depth, first_pos))
    }

    /// Puntaje: coincidencia exacta > prefijo > inicio de palabra > subcadena,
    /// con bonos por cercanía a la raíz y archivos recientes, y penalización
    /// para archivos ocultos o de sistema.
    fn score(&self, name: &[u8], flags: u8, mtime: u32, depth: u32, first_pos: Option<usize>) -> i32 {
        let mut s = 0i32;
        if let Some(t) = self.q.terms.first() {
            let tl = t.len();
            // Coincidencia exacta con el nombre o con el nombre sin extensión.
            let exact = name.len() >= tl
                && name[..tl] == t[..]
                && (name.len() == tl || (name[tl] == b'.' && !name[tl + 1..].contains(&b'.')));
            if exact {
                s += 1000;
            } else {
                let pos = first_pos.or_else(|| memchr::memmem::find(name, t));
                match pos {
                    Some(0) => s += 600,
                    Some(p) => s += if matches!(name[p - 1], b' ' | b'_' | b'-' | b'.' | b'(' | b'[') { 400 } else { 200 },
                    None => {}
                }
            }
            s -= (name.len() as i32 - tl as i32).clamp(0, 60);
        }
        if flags & F_DIR != 0 {
            s += 40;
        }
        if flags & (F_HIDDEN | F_SYSTEM) != 0 {
            s -= 250;
        }
        if name.first() == Some(&b'.') || name.first() == Some(&b'$') || name.first() == Some(&b'~') {
            s -= 80;
        }
        s -= (depth as i32).min(25) * 8;
        let age = self.now.saturating_sub(mtime);
        if age < 86_400 {
            s += 120;
        } else if age < 7 * 86_400 {
            s += 80;
        } else if age < 30 * 86_400 {
            s += 40;
        }
        s
    }
}

/// Número de trozos para paralelizar el barrido.
fn chunk_ranges(n: usize) -> Vec<(usize, usize)> {
    let parts = (rayon::current_num_threads() * 4).max(1);
    let step = n.div_ceil(parts).max(4096);
    (0..n).step_by(step).map(|a| (a, (a + step).min(n))).collect()
}

/// Busca en un volumen. Devuelve como mucho `limit` aciertos por trozo (los
/// mejores) y el total de coincidencias.
pub fn search_volume(v: &VolumeIndex, c: &Compiled, vol: u8, limit: usize) -> (Vec<Hit>, usize) {
    let n = v.len();
    if n <= 1 {
        return (Vec::new(), 0);
    }
    let ranges = chunk_ranges(n);
    let cmp = |x: &Hit, y: &Hit| y.score.cmp(&x.score).then(x.id.cmp(&y.id));
    let per_chunk: Vec<(Vec<Hit>, usize)> = ranges
        .par_iter()
        .map(|&(a, b)| {
            let mut hits = Vec::new();
            let mut count = 0usize;
            // Mantener solo los mejores: al pasar de 2×limit se recorta a limit, y lo
            // que no supera al último descartado ya no se guarda.
            let mut floor = i32::MIN;
            let mut push = |h: Hit, hits: &mut Vec<Hit>| {
                count += 1;
                if h.score < floor {
                    return;
                }
                hits.push(h);
                if hits.len() >= limit.saturating_mul(2) {
                    hits.select_nth_unstable_by(limit, cmp);
                    floor = floor.max(hits[limit].score);
                    hits.truncate(limit);
                }
            };
            let offs = &v.name_off[a..b];
            // El primer término es el patrón principal: su posición sirve para el puntaje.
            let pos_hint = c.primary_is_term == Some(0);
            let mut cur = 0usize;
            let mut last = usize::MAX;
            // Devuelve la posición (dentro del trozo) de la entrada que contiene `abs`.
            let mut on_match = |abs: u32, hits: &mut Vec<Hit>| -> usize {
                // Una pasada nueva (otro patrón) vuelve a empezar desde el principio.
                if abs < offs[cur] {
                    cur = 0;
                    last = usize::MAX;
                }
                // Avanzar el cursor a la entrada que contiene `abs`: primero unos pasos
                // lineales (las coincidencias suelen estar cerca) y si no, búsqueda binaria.
                let mut k = cur + 1;
                let lim = (cur + 8).min(offs.len());
                while k < lim && offs[k] <= abs {
                    k += 1;
                }
                cur = if k == lim && lim < offs.len() && offs[lim] <= abs { lim + offs[lim..].partition_point(|&o| o <= abs) - 1 } else { k - 1 };
                if cur == last {
                    return cur;
                }
                last = cur;
                let id = (a + cur) as u32;
                let hint = if pos_hint { Some((abs - offs[cur]) as usize) } else { None };
                if let Some(score) = c.eval_at(v, id, c.primary_is_term, hint) {
                    push(Hit { vol, id, score }, hits);
                }
                cur
            };
            let end = if b < n { v.name_off[b] as usize } else { v.lower.len() };
            match &c.primary {
                Some(Primary::One(finder, adj)) => {
                    // Con patrones que empiezan en el separador el barrido arranca un byte antes.
                    let adj = *adj as usize;
                    let start = v.name_off[a] as usize - adj;
                    let needle = finder.needle();
                    // Tras una coincidencia se salta al siguiente nombre: con letras muy
                    // frecuentes ("a") evita visitar cada aparición dentro del mismo nombre.
                    let mut from = start;
                    while from < end {
                        let hay = &v.lower[from..end];
                        let found = if needle.len() == 1 { memchr::memchr(needle[0], hay) } else { finder.find(hay) };
                        let Some(p) = found else { break };
                        let abs = from + p;
                        let e = on_match((abs + adj) as u32, &mut hits);
                        let next = if e + 1 < offs.len() { offs[e + 1] as usize - adj } else { end };
                        from = next.max(abs + 1);
                    }
                }
                Some(Primary::Several(finders)) => {
                    // Cada nombre termina en una sola extensión: las pasadas no se repiten.
                    let start = v.name_off[a] as usize;
                    for f in finders {
                        for pos in f.find_iter(&v.lower[start..end]) {
                            on_match((start + pos) as u32, &mut hits);
                        }
                    }
                }
                Some(Primary::Many(ac)) => {
                    let start = v.name_off[a] as usize;
                    for m in ac.find_iter(&v.lower[start..end]) {
                        on_match((start + m.start()) as u32, &mut hits);
                    }
                }
                None => {
                    // Sin texto que buscar: filtrar primero por los arreglos numéricos,
                    // que se recorren mucho más rápido que los nombres.
                    let q = c.q;
                    let numeric = q.size_min.is_some() || q.size_max.is_some() || q.mtime_min.is_some() || q.mtime_max.is_some();
                    let (smin, smax) = (q.size_min.unwrap_or(0), q.size_max.unwrap_or(u64::MAX));
                    let (tmin, tmax) = (q.mtime_min.unwrap_or(0), q.mtime_max.unwrap_or(u32::MAX));
                    let sizes = &v.size[a..b];
                    let mtimes = &v.mtime[a..b];
                    for k in 0..b - a {
                        if numeric {
                            let sz = sizes[k];
                            let mt = mtimes[k];
                            if sz < smin || sz > smax || mt < tmin || (q.mtime_max.is_some() && mt >= tmax) {
                                continue;
                            }
                        }
                        let id = (a + k) as u32;
                        if let Some(score) = c.eval(v, id, None) {
                            push(Hit { vol, id, score }, &mut hits);
                        }
                    }
                }
            }
            if hits.len() > limit {
                hits.select_nth_unstable_by(limit, cmp);
                hits.truncate(limit);
            }
            (hits, count)
        })
        .collect();
    let total: usize = per_chunk.iter().map(|h| h.1).sum();
    let mut out = Vec::with_capacity(per_chunk.iter().map(|h| h.0.len()).sum());
    for (h, _) in per_chunk {
        out.extend(h);
    }
    (out, total)
}

/// Deja los `limit` mejores aciertos ordenados por puntaje.
pub fn rank(hits: &mut Vec<Hit>, limit: usize) {
    let cmp = |a: &Hit, b: &Hit| b.score.cmp(&a.score).then(a.vol.cmp(&b.vol)).then(a.id.cmp(&b.id));
    if hits.len() > limit {
        hits.select_nth_unstable_by(limit, cmp);
        hits.truncate(limit);
    }
    hits.par_sort_unstable_by(cmp);
}

/// Motor ingenuo de referencia para el benchmark: un hilo, entrada por entrada.
pub fn search_naive(v: &VolumeIndex, q: &Query, vol: u8) -> Vec<Hit> {
    let c = Compiled::new(q);
    let mut out = Vec::new();
    for id in 1..v.len() as u32 {
        if let Some(score) = c.eval(v, id, None) {
            out.push(Hit { vol, id, score });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VolumeIndex {
        let mut v = VolumeIndex::new('c', Source::Walk);
        let docs = v.push("Documentos", 0, F_DIR, 0, 0);
        v.push("Factura Enero.pdf", docs, 0, 2048, 0);
        v.push("factura.txt", docs, 0, 10, 0);
        v.push("Canción.mp3", 0, 0, 5_000_000, 0);
        v.compute_dir_sizes();
        v
    }

    #[test]
    fn finds_terms_and_filters() {
        let v = sample();
        let run = |s: &str| {
            let q = parse_query(s);
            let c = Compiled::new(&q);
            let (mut h, _) = search_volume(&v, &c, 0, 100);
            rank(&mut h, 100);
            h.iter().map(|h| v.name(h.id)).collect::<Vec<_>>()
        };
        assert_eq!(run("factura"), vec!["factura.txt", "Factura Enero.pdf"]);
        assert_eq!(run("ext:pdf"), vec!["Factura Enero.pdf"]);
        let mut both = run("ext:pdf;txt");
        both.sort();
        assert_eq!(both, vec!["Factura Enero.pdf", "factura.txt"]);
        assert_eq!(run("tipo:doc").len(), 2);
        assert_eq!(run("factura !enero"), vec!["factura.txt"]);
        assert_eq!(run("CANCIÓN"), vec!["Canción.mp3"]);
        assert_eq!(run("cancion"), vec!["Canción.mp3"]);
        assert_eq!(run("ext:MP3"), vec!["Canción.mp3"]);
        assert_eq!(run("size:>1mb"), vec!["Canción.mp3"]);
        assert_eq!(run("*.txt"), vec!["factura.txt"]);
        let mut f = run("fact*");
        f.sort();
        assert_eq!(f, vec!["Factura Enero.pdf", "factura.txt"]);
        assert_eq!(run("*enero*"), vec!["Factura Enero.pdf"]);
        assert_eq!(run("documentos\\ enero"), vec!["Factura Enero.pdf"]);
        assert_eq!(run("folder:"), vec!["Documentos"]);
    }
}
