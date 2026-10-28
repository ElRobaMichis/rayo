//! Almacenamiento compacto del índice de un volumen.
//!
//! Todos los nombres viven en un solo buffer (`lower`) como *clave de búsqueda*:
//! en minúsculas y sin acentos, separados por `\0`. Así una búsqueda es un único
//! barrido SIMD sobre memoria contigua y "cancion" encuentra "Canción".
//! Las mayúsculas originales se guardan en un bitmap (1 bit por byte) y los
//! acentos como parches de 4 bytes (posición, carácter original). Así no se
//! duplica el texto de los nombres.

pub const NONE: u32 = u32::MAX;

pub const F_DIR: u8 = 1;
pub const F_HIDDEN: u8 = 2;
pub const F_SYSTEM: u8 = 4;
pub const F_DELETED: u8 = 8;
pub const F_REPARSE: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Empty,
    /// Leído de la MFT en crudo (requiere administrador).
    Mft,
    /// Recorrido de carpetas en paralelo (sin administrador).
    Walk,
    /// Cargado desde la caché en disco.
    Cache,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Empty => "vacío",
            Source::Mft => "MFT",
            Source::Walk => "recorrido",
            Source::Cache => "caché",
        }
    }
}

/// Pasa un carácter a minúscula solo si conserva la longitud en UTF-8 y se
/// puede reconstruir exactamente con `to_uppercase`.
#[inline]
pub fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut it = c.to_lowercase();
    let l = match it.next() {
        Some(l) => l,
        None => return c,
    };
    if it.next().is_some() || l.len_utf8() != c.len_utf8() {
        return c;
    }
    let mut u = l.to_uppercase();
    if u.next() == Some(c) && u.next().is_none() { l } else { c }
}

pub fn fold_str(s: &str) -> String {
    s.chars().map(fold_char).collect()
}

/// Quita el diacrítico de una letra latina ya en minúscula ("é" → "e", "ñ" → "n").
#[inline]
pub fn strip_accent(c: char) -> char {
    if c.is_ascii() {
        return c;
    }
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ǐ' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ǒ' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => 's',
        'ţ' | 'ť' | 'ŧ' | 'ț' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ǔ' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

/// Clave de búsqueda de un texto: minúsculas y sin acentos.
pub fn search_key(s: &str) -> String {
    s.chars().map(|c| strip_accent(fold_char(c))).collect()
}

/// Clave de búsqueda y, por cada byte de la clave, el byte del texto original
/// donde empieza ese carácter (con un elemento extra al final). Sirve para
/// resaltar coincidencias en el nombre original.
pub fn search_key_with_map(s: &str) -> (String, Vec<usize>) {
    let mut key = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len() + 1);
    for (i, c) in s.char_indices() {
        let k = strip_accent(fold_char(c));
        for _ in 0..k.len_utf8() {
            map.push(i);
        }
        key.push(k);
    }
    map.push(s.len());
    (key, map)
}

#[derive(Default)]
pub struct VolumeIndex {
    pub letter: char,
    pub source: Source,
    /// Nombres en minúsculas; `lower[0] == 0` y cada nombre termina en `\0`.
    pub lower: Vec<u8>,
    /// Bit por byte de `lower`: 1 si el carácter que empieza ahí era mayúscula.
    pub upper: Vec<u64>,
    pub name_off: Vec<u32>,
    pub parent: Vec<u32>,
    pub flags: Vec<u8>,
    /// Tamaño en bytes. En carpetas, la suma recursiva de su contenido.
    pub size: Vec<u64>,
    /// Fecha de modificación en segundos Unix.
    pub mtime: Vec<u32>,
    pub first_child: Vec<u32>,
    pub next_sibling: Vec<u32>,
    /// Profundidad de cada entrada (raíz = 0), para el puntaje de búsqueda.
    pub depth: Vec<u8>,
    /// Entradas cuyo nombre lleva acentos (ordenadas) y dónde empiezan sus parches.
    pub acc_ids: Vec<u32>,
    pub acc_start: Vec<u32>,
    /// Parches: (byte dentro de la clave, carácter original). Todos caben en u16.
    pub acc_data: Vec<[u16; 2]>,
    pub deleted: usize,
    /// Carpetas por número de registro de la MFT (para aplicar el USN Journal).
    pub dir_map: std::collections::HashMap<u32, u32>,
    /// Journal del que viene el índice y hasta qué USN está al día (0 = sin journal).
    pub usn_journal: u64,
    pub usn_next: i64,
}

impl VolumeIndex {
    pub fn new(letter: char, source: Source) -> Self {
        let mut v = VolumeIndex {
            letter: letter.to_ascii_uppercase(),
            source,
            lower: vec![0],
            upper: vec![0],
            ..Default::default()
        };
        let root = format!("{}:", v.letter);
        v.push(&root, NONE, F_DIR, 0, 0);
        v
    }

    pub fn with_capacity(letter: char, source: Source, entries: usize, name_bytes: usize) -> Self {
        let mut v = Self::new(letter, source);
        v.lower.reserve(name_bytes);
        v.upper.reserve(name_bytes / 64 + 1);
        v.name_off.reserve(entries);
        v.parent.reserve(entries);
        v.flags.reserve(entries);
        v.size.reserve(entries);
        v.mtime.reserve(entries);
        v
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.name_off.len()
    }

    pub fn live_len(&self) -> usize {
        self.len() - self.deleted
    }

    /// Agrega una entrada sin enlazarla al árbol (ver `link_children`).
    pub fn push(&mut self, name: &str, parent: u32, flags: u8, size: u64, mtime: u32) -> u32 {
        let id = self.name_off.len() as u32;
        let off = self.lower.len();
        let need = (off + name.len() + 1).div_ceil(64) + 1;
        if self.upper.len() < need {
            self.upper.resize(need, 0);
        }
        let mut buf = [0u8; 4];
        let mut first_patch = true;
        for c in name.chars() {
            let l = fold_char(c);
            let k = strip_accent(l);
            let pos = self.lower.len();
            if k != l {
                if first_patch {
                    self.acc_ids.push(id);
                    self.acc_start.push(self.acc_data.len() as u32);
                    first_patch = false;
                }
                self.acc_data.push([(pos - off) as u16, c as u32 as u16]);
            }
            self.lower.extend_from_slice(k.encode_utf8(&mut buf).as_bytes());
            if l != c {
                self.upper[pos >> 6] |= 1u64 << (pos & 63);
            }
        }
        self.lower.push(0);
        self.name_off.push(off as u32);
        self.parent.push(parent);
        self.flags.push(flags);
        self.size.push(size);
        self.mtime.push(mtime);
        if !self.first_child.is_empty() || id == 0 {
            // Árbol ya enlazado: mantenerlo al día.
            self.first_child.push(NONE);
            self.next_sibling.push(NONE);
            let mut d = 0u8;
            if parent != NONE {
                let p = parent as usize;
                self.next_sibling[id as usize] = self.first_child[p];
                self.first_child[p] = id;
                d = self.depth.get(p).map_or(0, |x| x.saturating_add(1));
            }
            self.depth.push(d);
        }
        id
    }

    #[inline]
    pub fn name_range(&self, i: u32) -> (usize, usize) {
        let i = i as usize;
        let s = self.name_off[i] as usize;
        let e = if i + 1 < self.name_off.len() {
            self.name_off[i + 1] as usize - 1
        } else {
            self.lower.len() - 1
        };
        (s, e)
    }

    #[inline]
    pub fn name_lower(&self, i: u32) -> &[u8] {
        let (s, e) = self.name_range(i);
        &self.lower[s..e]
    }

    #[inline]
    fn is_upper(&self, pos: usize) -> bool {
        self.upper.get(pos >> 6).is_some_and(|w| w & (1u64 << (pos & 63)) != 0)
    }

    /// Parches de acentos de una entrada (vacío si no tiene).
    fn patches(&self, i: u32) -> &[[u16; 2]] {
        match self.acc_ids.binary_search(&i) {
            Ok(k) => {
                let a = self.acc_start[k] as usize;
                let b = self.acc_start.get(k + 1).map_or(self.acc_data.len(), |&x| x as usize);
                &self.acc_data[a..b]
            }
            Err(_) => &[],
        }
    }

    pub fn accented_count(&self) -> usize {
        self.acc_ids.len()
    }

    /// Nombre en minúsculas pero con acentos (para comparar nombres exactos).
    pub fn name_casefold(&self, i: u32) -> std::borrow::Cow<'_, [u8]> {
        if self.patches(i).is_empty() {
            std::borrow::Cow::Borrowed(self.name_lower(i))
        } else {
            std::borrow::Cow::Owned(fold_str(&self.name(i)).into_bytes())
        }
    }

    /// Reconstruye el nombre con sus mayúsculas y acentos originales.
    pub fn name(&self, i: u32) -> String {
        let (s, e) = self.name_range(i);
        let st = std::str::from_utf8(&self.lower[s..e]).unwrap_or("");
        let mut out = String::with_capacity(st.len() + 8);
        let mut patches = self.patches(i).iter().peekable();
        for (p, c) in st.char_indices() {
            if let Some(&&[pos, orig]) = patches.peek() {
                if pos as usize == p {
                    patches.next();
                    out.push(char::from_u32(orig as u32).unwrap_or(c));
                    continue;
                }
            }
            if self.is_upper(s + p) {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    #[inline]
    pub fn is_dir(&self, i: u32) -> bool {
        self.flags[i as usize] & F_DIR != 0
    }

    #[inline]
    pub fn is_deleted(&self, i: u32) -> bool {
        self.flags[i as usize] & F_DELETED != 0
    }

    #[inline]
    pub fn depth(&self, i: u32) -> u32 {
        match self.depth.get(i as usize) {
            Some(&d) => d as u32,
            None => self.depth_slow(i),
        }
    }

    fn depth_slow(&self, mut i: u32) -> u32 {
        let mut d = 0;
        while i != NONE && i != 0 {
            i = self.parent[i as usize];
            d += 1;
            if d > 512 {
                break;
            }
        }
        d
    }

    pub fn full_path(&self, i: u32) -> String {
        if i == 0 {
            return format!("{}:\\", self.letter);
        }
        let mut chain = Vec::with_capacity(16);
        let mut cur = i;
        while cur != NONE && chain.len() < 512 {
            chain.push(cur);
            cur = self.parent[cur as usize];
        }
        let mut out = String::with_capacity(chain.len() * 16);
        for (k, &c) in chain.iter().rev().enumerate() {
            if k > 0 {
                out.push('\\');
            }
            out.push_str(&self.name(c));
        }
        out
    }

    /// Ruta de la carpeta que contiene `i`.
    pub fn parent_path(&self, i: u32) -> String {
        let p = self.parent[i as usize];
        if p == NONE { String::new() } else { self.full_path(p) }
    }

    /// Reconstruye las listas de hijos a partir de `parent`.
    pub fn link_children(&mut self) {
        let n = self.len();
        self.first_child = vec![NONE; n];
        self.next_sibling = vec![NONE; n];
        // En orden inverso para que los hijos queden en orden de inserción.
        for i in (1..n).rev() {
            let p = self.parent[i];
            if p != NONE && (p as usize) < n {
                self.next_sibling[i] = self.first_child[p as usize];
                self.first_child[p as usize] = i as u32;
            }
        }
        // Profundidades recorriendo el árbol desde la raíz.
        self.depth = vec![0u8; n];
        let mut stack = vec![0u32];
        // Con datos dañados podría haber ciclos: nunca más pasos que entradas.
        let mut budget = n;
        while let Some(d) = stack.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let dd = self.depth[d as usize].saturating_add(1);
            let mut c = self.first_child[d as usize];
            while c != NONE {
                self.depth[c as usize] = dd;
                if self.flags[c as usize] & F_DIR != 0 {
                    stack.push(c);
                }
                c = self.next_sibling[c as usize];
            }
        }
    }

    pub fn children(&self, dir: u32) -> ChildIter<'_> {
        ChildIter { idx: self, cur: self.first_child.get(dir as usize).copied().unwrap_or(NONE) }
    }

    /// Hijo de `dir` con este nombre (sin distinguir mayúsculas; los acentos sí
    /// cuentan, salvo que no haya otra opción).
    pub fn find_child(&self, dir: u32, name: &str) -> Option<u32> {
        let key = search_key(name);
        let exact = fold_str(name);
        let mut fallback = None;
        for c in self.children(dir) {
            if self.name_lower(c) == key.as_bytes() {
                if *self.name_casefold(c) == *exact.as_bytes() {
                    return Some(c);
                }
                fallback.get_or_insert(c);
            }
        }
        fallback
    }

    /// Busca la entrada de una ruta absoluta como `C:\Users\Ana`.
    pub fn find_path(&self, path: &str) -> Option<u32> {
        let p = path.trim_start_matches("\\\\?\\");
        let mut parts = p.split(['\\', '/']).filter(|s| !s.is_empty());
        let drive = parts.next()?;
        if !drive.to_ascii_uppercase().starts_with(self.letter) {
            return None;
        }
        let mut cur = 0u32;
        for comp in parts {
            cur = self.find_child(cur, comp)?;
        }
        Some(cur)
    }

    /// Recalcula el tamaño de todas las carpetas (suma recursiva).
    pub fn compute_dir_sizes(&mut self) {
        let n = self.len();
        if self.first_child.len() != n {
            self.link_children();
        }
        let mut order = Vec::with_capacity(n / 4);
        let mut stack = vec![0u32];
        // Con datos dañados podría haber ciclos: nunca más pasos que entradas.
        let mut budget = n;
        while let Some(d) = stack.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            order.push(d);
            let mut c = self.first_child[d as usize];
            while c != NONE {
                let f = self.flags[c as usize];
                if f & F_DIR != 0 && f & F_DELETED == 0 {
                    stack.push(c);
                }
                c = self.next_sibling[c as usize];
            }
        }
        for &d in order.iter().rev() {
            let mut sum = 0u64;
            let mut c = self.first_child[d as usize];
            while c != NONE {
                if self.flags[c as usize] & F_DELETED == 0 {
                    sum += self.size[c as usize];
                }
                c = self.next_sibling[c as usize];
            }
            self.size[d as usize] = sum;
        }
    }

    fn add_to_ancestors(&mut self, i: u32, delta: i64) {
        if delta == 0 {
            return;
        }
        let mut p = self.parent[i as usize];
        let mut guard = 0;
        while p != NONE && guard < 512 {
            let s = &mut self.size[p as usize];
            *s = (*s as i64 + delta).max(0) as u64;
            p = self.parent[p as usize];
            guard += 1;
        }
    }

    /// Agrega una entrada nueva ya enlazada y ajusta el tamaño de los ancestros.
    pub fn add_entry(&mut self, parent: u32, name: &str, flags: u8, size: u64, mtime: u32) -> u32 {
        let id = self.push(name, parent, flags, size, mtime);
        if flags & F_DIR == 0 {
            self.add_to_ancestors(id, size as i64);
        }
        id
    }

    pub fn update_file(&mut self, i: u32, size: u64, mtime: u32) {
        let old = self.size[i as usize];
        self.mtime[i as usize] = mtime;
        if !self.is_dir(i) {
            self.size[i as usize] = size;
            self.add_to_ancestors(i, size as i64 - old as i64);
        }
    }

    /// Renombra o mueve una entrada conservando su contenido: se agrega la entrada
    /// nueva, se le pasan los hijos y la vieja queda marcada como borrada.
    pub fn rename_entry(&mut self, old: u32, new_parent: u32, name: &str, mtime: u32) -> u32 {
        let o = old as usize;
        let total = self.size[o];
        self.add_to_ancestors(old, -(total as i64));
        let mt = if mtime != 0 { mtime } else { self.mtime[o] };
        let new = self.push(name, new_parent, self.flags[o], total, mt);
        let first = self.first_child[o];
        self.first_child[new as usize] = first;
        self.first_child[o] = NONE;
        let mut c = first;
        while c != NONE {
            self.parent[c as usize] = new;
            c = self.next_sibling[c as usize];
        }
        self.flags[o] |= F_DELETED;
        self.deleted += 1;
        self.add_to_ancestors(new, total as i64);
        // Profundidades del contenido movido.
        let mut stack = vec![new];
        while let Some(d) = stack.pop() {
            let dd = self.depth[d as usize].saturating_add(1);
            let mut c = self.first_child[d as usize];
            while c != NONE {
                self.depth[c as usize] = dd;
                stack.push(c);
                c = self.next_sibling[c as usize];
            }
        }
        new
    }

    /// Marca como borrada una entrada y todo su contenido.
    pub fn remove_subtree(&mut self, i: u32) {
        if i == 0 || self.is_deleted(i) {
            return;
        }
        let total = self.size[i as usize];
        self.add_to_ancestors(i, -(total as i64));
        let mut stack = vec![i];
        while let Some(d) = stack.pop() {
            if self.flags[d as usize] & F_DELETED == 0 {
                self.flags[d as usize] |= F_DELETED;
                self.deleted += 1;
            }
            let mut c = self.first_child[d as usize];
            while c != NONE {
                stack.push(c);
                c = self.next_sibling[c as usize];
            }
        }
    }

    pub fn memory_bytes(&self) -> usize {
        self.lower.capacity()
            + self.upper.capacity() * 8
            + self.name_off.capacity() * 4
            + self.parent.capacity() * 4
            + self.flags.capacity()
            + self.size.capacity() * 8
            + self.mtime.capacity() * 4
            + self.first_child.capacity() * 4
            + self.next_sibling.capacity() * 4
            + self.depth.capacity()
            + self.acc_ids.capacity() * 4
            + self.acc_start.capacity() * 4
            + self.acc_data.capacity() * 4
            + self.dir_map.capacity() * 12
    }

    pub fn shrink(&mut self) {
        self.lower.shrink_to_fit();
        self.upper.shrink_to_fit();
        self.name_off.shrink_to_fit();
        self.parent.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.size.shrink_to_fit();
        self.mtime.shrink_to_fit();
        self.first_child.shrink_to_fit();
        self.next_sibling.shrink_to_fit();
        self.depth.shrink_to_fit();
        self.acc_ids.shrink_to_fit();
        self.acc_start.shrink_to_fit();
        self.acc_data.shrink_to_fit();
    }
}

pub struct ChildIter<'a> {
    idx: &'a VolumeIndex,
    cur: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        while self.cur != NONE {
            let c = self.cur;
            self.cur = self.idx.next_sibling[c as usize];
            if !self.idx.is_deleted(c) {
                return Some(c);
            }
        }
        None
    }
}

/// FILETIME (100 ns desde 1601) a segundos Unix.
#[inline]
pub fn filetime_to_unix(ft: u64) -> u32 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    let secs = ft / 10_000_000;
    secs.saturating_sub(EPOCH_DIFF).min(u32::MAX as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_names() {
        let mut v = VolumeIndex::new('c', Source::Walk);
        let a = v.push("Canción ÁRBOL.TXT", 0, 0, 10, 0);
        let b = v.push("İstanbul K", 0, 0, 5, 0);
        assert_eq!(v.name(a), "Canción ÁRBOL.TXT");
        assert_eq!(v.name(b), "İstanbul K");
        assert_eq!(v.name_lower(a), "cancion arbol.txt".as_bytes());
        // Dos nombres que solo difieren en acentos se distinguen al buscar la ruta.
        let plain = v.push("cancion arbol.txt", 0, 0, 1, 0);
        assert_eq!(v.name(plain), "cancion arbol.txt");
        v.link_children();
        v.compute_dir_sizes();
        assert_eq!(v.size[0], 16);
        assert_eq!(v.full_path(a), "C:\\Canción ÁRBOL.TXT");
        assert_eq!(v.find_path("c:\\canción árbol.txt"), Some(a));
        assert_eq!(v.find_path("c:\\CANCION ARBOL.TXT"), Some(plain));
        let (key, map) = search_key_with_map("Canción");
        assert_eq!(key, "cancion");
        assert_eq!(&"Canción"[map[4]..map[7]], "ión");
    }
}
