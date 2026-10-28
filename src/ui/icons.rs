//! Ícono y color según el tipo de archivo.

use super::theme::Theme;
use egui::Color32;
use egui_phosphor::regular as ic;

const fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn for_entry(t: &Theme, is_dir: bool, ext: &str) -> (&'static str, Color32) {
    if is_dir {
        return (ic::FOLDER, t.folder);
    }
    let purple = rgb(0xB57CFF);
    let pink = rgb(0xF472B6);
    let red = rgb(0xF0605D);
    let orange = rgb(0xF59E42);
    let amber = rgb(0xE9B949);
    let green = rgb(0x3FC07A);
    let teal = rgb(0x2EC4B6);
    let blue = rgb(0x5B9BFF);
    let gray = t.dim;
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "ico" | "jfif" | "raw" | "cr2" | "nef" | "arw" | "dng" => (ic::IMAGE, purple),
        "svg" => (ic::FILE_SVG, purple),
        "psd" | "ai" | "fig" | "xd" | "sketch" => (ic::PAINT_BRUSH, purple),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" | "3gp" => (ic::FILM_STRIP, pink),
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" | "mid" => (ic::MUSIC_NOTES, pink),
        "pdf" => (ic::FILE_PDF, red),
        "doc" | "docx" | "odt" | "rtf" => (ic::FILE_DOC, blue),
        "xls" | "xlsx" | "ods" => (ic::FILE_XLS, green),
        "csv" | "tsv" => (ic::FILE_CSV, green),
        "ppt" | "pptx" | "odp" | "key" => (ic::FILE_PPT, orange),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "cab" => (ic::FILE_ZIP, amber),
        "iso" | "img" | "vhd" | "vhdx" => (ic::DISC, amber),
        "exe" | "msi" | "appx" | "msix" => (ic::APP_WINDOW, teal),
        "bat" | "cmd" | "ps1" | "sh" => (ic::TERMINAL, teal),
        "dll" | "sys" | "drv" | "ocx" => (ic::GEAR, gray),
        "lnk" | "url" => (ic::LINK, blue),
        "txt" | "log" => (ic::FILE_TEXT, gray),
        "md" | "markdown" => (ic::FILE_MD, gray),
        "ini" | "cfg" | "conf" | "toml" | "yaml" | "yml" | "env" | "reg" => (ic::FILE_INI, gray),
        "json" | "xml" => (ic::BRACKETS_CURLY, amber),
        "rs" => (ic::FILE_RS, orange),
        "js" | "mjs" | "cjs" => (ic::FILE_JS, amber),
        "jsx" => (ic::FILE_JSX, blue),
        "ts" | "mts" => (ic::FILE_TS, blue),
        "tsx" => (ic::FILE_TSX, blue),
        "py" | "pyw" | "ipynb" => (ic::FILE_PY, blue),
        "html" | "htm" => (ic::FILE_HTML, orange),
        "css" | "scss" | "sass" | "less" => (ic::FILE_CSS, blue),
        "c" | "h" => (ic::FILE_C, blue),
        "cpp" | "cc" | "cxx" | "hpp" => (ic::FILE_CPP, blue),
        "sql" | "db" | "sqlite" | "mdb" => (ic::DATABASE, teal),
        "cs" | "java" | "kt" | "go" | "rb" | "php" | "swift" | "lua" | "dart" | "vue" | "zig" => (ic::FILE_CODE, teal),
        "ttf" | "otf" | "woff" | "woff2" => (ic::TEXT_AA, gray),
        "epub" | "mobi" => (ic::BOOK_OPEN, orange),
        "torrent" => (ic::MAGNET, green),
        "lock" => (ic::FILE_LOCK, gray),
        _ => (ic::FILE, gray),
    }
}
