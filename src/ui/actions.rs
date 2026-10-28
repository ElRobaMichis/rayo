//! Acciones que piden las vistas; se ejecutan al final de cada cuadro.

use super::config::ViewMode;
use crate::fsx::SortKey;

#[derive(Debug, Clone)]
pub enum Action {
    Navigate(String),
    NewTab(Option<String>),
    CloseTab(usize),
    SwitchTab(usize),
    Reveal(String),
    Select { col: usize, name: String, is_dir: bool, ctrl: bool, shift: bool },
    FocusCol(usize),
    /// Reemplaza la selección de una columna (rectángulo o clic en vacío). Cierra las columnas a su derecha.
    SetSelection { col: usize, names: Vec<String> },
    /// Se empezó a arrastrar algo no seleccionado: queda seleccionado solo eso.
    DragSelect { col: usize, name: String },
    Open { path: String, is_dir: bool },
    OpenWith(String),
    QuickLook,
    StartRename(String, String),
    CommitRename,
    Copy(Vec<String>, bool),
    Paste(String),
    CopyPath(Vec<String>),
    Delete(Vec<String>, bool),
    Properties(String),
    NativeMenu(Vec<String>),
    NewFolder(String),
    /// Archivo nuevo de un tipo de "Nuevo ▸" (índice en `fsx::shellnew::items`).
    NewFile(String, usize),
    Terminal(String),
    ShowInExplorer(String),
    ExplorerAt(String),
    AddFavorite(usize, String),
    RemoveFavorite(usize, usize),
    NewGroup,
    DeleteGroup(usize),
    RenameGroup(usize),
    Eject(char),
    SetView(ViewMode),
    SetSort(SortKey),
    ToggleTheme,
    TogglePreview,
    ToggleHidden,
    Back,
    Forward,
    Up,
    Refresh,
    SetResultSort(crate::index::ResultSort),
    LoadAllResults,
    InstallService,
    UninstallService,
    // Vista dividida
    ToggleSplit,
    /// Copiar (o mover) al panel de enfrente.
    SendToOther { paths: Vec<String>, mv: bool },
    /// Archivos soltados sobre una carpeta. `force`: Some(true) = mover, Some(false) = copiar.
    DropFiles { paths: Vec<String>, dest: String, force: Option<bool> },
    /// Ejecutar una opción del menú de Windows (token del menú, número de comando).
    ShellInvoke(u64, u32),
    /// Fijar o quitar una opción arriba del menú de clic derecho.
    TogglePin(String),
    /// Abrir el menú de clic derecho (en su propia ventana).
    OpenMenu(super::ctxmenu::MenuReq),
    /// Lo escrito en la barra de ruta: una ruta, una carpeta de aquí o un programa.
    PathInput(String),
    /// Quitar una búsqueda del historial (None = todas).
    ForgetSearch(Option<usize>),
    // Cola de copias
    TransferPause(u64, bool),
    TransferCancel(u64),
    TransferResolve(u64, crate::fsx::transfer::Choice, bool),
    TransferClear,
    /// Un comando del catálogo (menú o paleta) sobre una selección.
    Command(&'static str, super::commands::Target),
    /// Pide abrir la paleta de comandos.
    OpenPalette,
    /// Abrir una carpeta en el panel de enfrente.
    OpenOther(String),
    /// Pestaña de la papelera.
    OpenRecycle,
    /// Vista plana de la pestaña actual (encender / apagar).
    ToggleFlat,
    /// Pestaña de archivos recientes.
    OpenRecent,
}
