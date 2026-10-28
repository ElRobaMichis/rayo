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
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
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
