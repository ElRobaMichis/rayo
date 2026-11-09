//! Preferencias guardadas en %APPDATA%\Rayo\config.json.

use crate::fsx::SortKey;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Columns,
    Details,
    Grid,
}

/// Vista y orden recordados para una carpeta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderView {
    pub view: ViewMode,
    pub sort: SortKey,
    pub desc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FavGroup {
    pub name: String,
    pub items: Vec<String>,
    #[serde(default)]
    pub collapsed: bool,
}

/// Anchos de columnas que se ajustan arrastrando su borde.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnWidths {
    /// Vista de detalles: Modificado (fecha y antigüedad) y Tipo; Tamaño es fijo. El nombre toma el resto.
    pub date: f32,
    pub kind: f32,
    pub size: f32,
    /// Resultados de búsqueda: ancho del nombre (0 = automático).
    pub search_name: f32,
}

impl ColumnWidths {
    pub const DEFAULT: ColumnWidths = ColumnWidths { date: 196.0, kind: 120.0, size: 86.0, search_name: 0.0 };
}

impl Default for ColumnWidths {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dark: bool,
    pub show_hidden: bool,
    pub show_preview: bool,
    pub view: ViewMode,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub col_width: f32,
    pub groups: Vec<FavGroup>,
    pub tabs: Vec<String>,
    pub active_tab: usize,
    /// Vista dividida en dos paneles y las pestañas del panel derecho.
    pub split: bool,
    pub tabs_right: Vec<String>,
    pub active_right: usize,
    /// Barras de uso de espacio en la columna Tamaño.
    pub space_bars: bool,
    pub places_collapsed: bool,
    pub drives_collapsed: bool,
    /// Al cerrar la ventana, Rayo sigue en segundo plano y vuelve a abrir al instante.
    pub stay_resident: bool,
    /// Arrancar (oculto) al iniciar sesión en Windows.
    pub start_with_windows: bool,
    /// Vista y orden de las carpetas que nunca se cambiaron.
    pub default_view: FolderView,
    /// Vista y orden elegidos por carpeta (ruta en minúsculas), del más viejo al más nuevo.
    pub folder_views: Vec<(String, FolderView)>,
    /// Búsquedas recientes, de la más nueva a la más vieja.
    pub search_history: Vec<String>,
    /// Rayo abre las carpetas, Win+E y "Mostrar en carpeta" en vez del Explorador.
    pub default_explorer: bool,
    /// Se eligió usarlo sin instalar (no volver a preguntar).
    pub portable: bool,
    /// Opciones fijadas arriba del menú de clic derecho ("cmd:copy", "shell:…").
    pub menu_pins: Vec<String>,
    /// Opciones de Windows ocultas en el menú de clic derecho.
    pub menu_hidden: Vec<String>,
    /// Idioma de la interfaz ("es", "en"…); None = el de Windows. Una configuración de antes
    /// de que hubiera idiomas no lo tiene: esa persona usaba Rayo en español y así sigue.
    #[serde(default = "spanish_before_languages")]
    pub lang: Option<String>,
    pub columns: ColumnWidths,
    pub tags_collapsed: bool,
}

fn spanish_before_languages() -> Option<String> {
    Some("es".into())
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dark: true,
            show_hidden: false,
            show_preview: true,
            view: ViewMode::Columns,
            sort: SortKey::Name,
            sort_desc: false,
            col_width: 250.0,
            groups: vec![FavGroup { name: t!("comun.favoritos").into(), items: Vec::new(), collapsed: false }],
            tabs: Vec::new(),
            active_tab: 0,
            split: false,
            tabs_right: Vec::new(),
            active_right: 0,
            space_bars: true,
            places_collapsed: false,
            drives_collapsed: false,
            stay_resident: true,
            start_with_windows: false,
            default_view: FolderView { view: ViewMode::Details, sort: SortKey::Name, desc: false },
            folder_views: Vec::new(),
            search_history: Vec::new(),
            default_explorer: false,
            portable: false,
            menu_pins: Vec::new(),
            menu_hidden: Vec::new(),
            lang: None,
            columns: ColumnWidths::DEFAULT,
            tags_collapsed: false,
        }
    }
}

fn path() -> PathBuf {
    let d = crate::instance::custom_data_dir()
        .unwrap_or_else(|| std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Rayo"));
    let _ = std::fs::create_dir_all(&d);
    d.join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let p = path();
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        let (cfg, dropped) = Self::parse(&text);
        if !dropped.is_empty() {
            // Algo no se pudo leer (p. ej. una versión cambió el tipo de un ajuste): se guarda una
            // copia del archivo tal como estaba y se deja constancia de qué volvió a su valor original.
            let _ = std::fs::copy(&p, p.with_extension("json.bak"));
            crate::diag::log(&format!("configuración: no se pudieron leer {dropped:?}; quedan con su valor original"));
        }
        cfg
    }

    /// Lee la configuración sin perderla entera por un ajuste que no se entiende: lo que no se
    /// puede leer vuelve a su valor original y todo lo demás se conserva. Devuelve también qué
    /// ajustes se descartaron.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        // Un editor (el Bloc de notas de antes, PowerShell) puede guardar la marca BOM al principio.
        let text = text.trim_start_matches('\u{feff}');
        if let Ok(c) = serde_json::from_str::<Self>(text) {
            return (c, Vec::new());
        }
        let Ok(serde_json::Value::Object(user)) = serde_json::from_str::<serde_json::Value>(text) else {
            return (Self::default(), vec!["(archivo ilegible)".into()]);
        };
        let mut kept = serde_json::Map::new();
        let mut dropped = Vec::new();
        for (k, v) in user {
            let mut probe = kept.clone();
            probe.insert(k.clone(), v.clone());
            if serde_json::from_value::<Self>(serde_json::Value::Object(probe)).is_ok() {
                kept.insert(k, v);
            } else {
                dropped.push(k);
            }
        }
        (serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default(), dropped)
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let p = path();
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una configuración guardada por Rayo 0.2.4 se sigue leyendo entera: actualizar no debe
    /// perder pestañas, favoritos ni ajustes. (Si un cambio rompe esto, hay que migrar, no borrar.)
    #[test]
    fn reads_config_from_0_2_4() {
        let (c, dropped) = Config::parse(include_str!("testdata/config-0.2.4.json"));
        assert!(dropped.is_empty(), "no se pudieron leer: {dropped:?}");
        assert!(!c.dark && c.show_hidden && c.split && c.default_explorer && c.tags_collapsed);
        assert_eq!(c.view, ViewMode::Details);
        assert_eq!(c.tabs.len(), 2);
        assert_eq!(c.active_tab, 1);
        assert_eq!(c.groups.len(), 2);
        assert_eq!(c.groups[1].items, vec![r"C:\Trabajo\Informes".to_string()]);
        assert_eq!(c.default_view.view, ViewMode::Grid);
        assert_eq!(c.folder_views.len(), 1);
        assert_eq!(c.columns.date, 210.0);
        assert_eq!(c.lang.as_deref(), Some("es"));
    }

    /// Un ajuste que no se entiende vuelve a su valor original; lo demás se conserva.
    #[test]
    fn a_bad_setting_does_not_reset_the_rest() {
        let text = include_str!("testdata/config-0.2.4.json").replace("\"col_width\": 280.0", "\"col_width\": \"ancho\"");
        let (c, dropped) = Config::parse(&text);
        assert_eq!(dropped, vec!["col_width".to_string()]);
        assert_eq!(c.col_width, Config::default().col_width);
        assert_eq!(c.tabs.len(), 2);
        assert_eq!(c.groups.len(), 2);
        assert!(c.default_explorer);
    }

    /// Guardado con marca BOM (Bloc de notas, PowerShell): se lee igual.
    #[test]
    fn reads_config_with_bom() {
        let text = format!("\u{feff}{}", include_str!("testdata/config-0.2.4.json"));
        let (c, dropped) = Config::parse(&text);
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(c.tabs.len(), 2);
    }
}
