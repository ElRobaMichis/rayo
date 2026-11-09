//! Pinyin para nombres en chino: buscar 科幻小说 escribiendo "khxs" (iniciales) o "kehuan"
//! (pinyin completo), y ordenar los nombres en chino por su pinyin, como el Explorador en chino.

use pinyin::ToPinyin;

/// ¿Tiene caracteres chinos (CJK)?
pub fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}

/// (iniciales, pinyin completo), en minúsculas; lo que no es chino queda tal cual.
pub fn keys(s: &str) -> (String, String) {
    let mut init = String::with_capacity(s.len());
    let mut full = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        match c.to_pinyin() {
            Some(p) => {
                init.push_str(p.first_letter());
                full.push_str(p.plain());
            }
            None => {
                for l in c.to_lowercase() {
                    init.push(l);
                    full.push(l);
                }
            }
        }
    }
    (init, full)
}

/// Lo escrito parece pinyin (letras sin acentos, al menos 2): vale la pena buscar por pinyin.
pub fn is_query(q: &str) -> bool {
    let q = q.trim();
    q.len() >= 2 && q.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Clave para ordenar: el pinyin completo de los nombres en chino (los demás, sin cambios).
pub fn sort_key(lower: &str) -> std::borrow::Cow<'_, str> {
    if has_cjk(lower) { std::borrow::Cow::Owned(keys(lower).1) } else { std::borrow::Cow::Borrowed(lower) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_and_full() {
        assert!(has_cjk("科幻小说.txt") && !has_cjk("informe.pdf"));
        let (i, f) = keys("科幻小说.txt");
        assert_eq!(i, "khxs.txt");
        assert_eq!(f, "kehuanxiaoshuo.txt");
        assert!(is_query("khxs") && !is_query("k") && !is_query("año"));
        assert_eq!(sort_key("中国"), "zhongguo");
    }
}
