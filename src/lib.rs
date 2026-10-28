/// El texto de una clave en el idioma actual (ver `i18n`).
#[macro_export]
macro_rules! t {
    ($k:literal) => {
        $crate::i18n::tr($k)
    };
}

/// El texto de una clave con sus `{variables}` reemplazadas: `tf!("clave", n = 3)`.
#[macro_export]
macro_rules! tf {
    ($k:literal $(, $n:ident = $v:expr)* $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($k), &[$((stringify!($n), &$v as &dyn ::std::fmt::Display)),*])
    };
}

/// Como `tf!`, pero elige `<clave>.uno` o `<clave>.varios` según `count`, que va en `{n}`:
/// `tp!(3, "app.copiado_n")` → "Copiados 3 elementos". Con `n = fsx::thousands(x)` se muestra
/// la cantidad con separador de miles.
#[macro_export]
macro_rules! tp {
    ($count:expr, $k:literal $(, $n:ident = $v:expr)* $(,)?) => {{
        let count = $count;
        $crate::i18n::fill(
            $crate::i18n::tr(if count == 1 { concat!($k, ".uno") } else { concat!($k, ".varios") }),
            &[$((stringify!($n), &$v as &dyn ::std::fmt::Display),)* ("n", &count as &dyn ::std::fmt::Display)],
        )
    }};
}

pub mod admin;
pub mod fsx;
pub mod i18n;
pub mod index;
pub mod install;
pub mod instance;
pub mod integration;
pub mod ui;
