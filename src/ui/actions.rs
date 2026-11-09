//! Acciones que piden las vistas; se ejecutan al final de cada cuadro.

use super::config::ViewMode;
use crate::fsx::SortKey;

#[derive(Debug, Clone)]
pub enum Action {
    Navigate(String),
    /// Leer una carpeta antes de entrar (el mouse se detuvo sobre ella): al entrar ya está lista.
    Prefetch(String),
    NewTab(Option<String>),
    CloseTab(usize),
    // Grupos de pestañas
    /// Grupo nuevo con esta pestaña.
    NewTabGroup(usize),
    /// Pasar una pestaña a un grupo (0 = sacarla del suyo).
    SetTabGroup(usize, u64),
    ToggleTabGroup(u64),
    RenameTabGroup(u64, String),
    RecolorTabGroup(u64, usize),
    /// Deshacer el grupo (las pestañas quedan).
    Ungroup(u64),
    /// Cerrar todas las pestañas del grupo.
    CloseTabGroup(u64),
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
    /// Quitar la marca de "descargado de Internet" (de estos archivos y de lo de adentro de estas carpetas).
    Unblock(Vec<String>),
    /// Cambiar una propiedad de un archivo (título, comentarios, calificación).
    SetProperty(String, &'static str, crate::fsx::props::Value),
    /// Mostrar u ocultar una columna extra de la vista de detalles.
    ToggleColumn(String),
    /// Se terminó de ajustar el ancho de una columna: guardarlo.
    SaveColumns(super::config::ColumnWidths),
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
    /// Cuatro paneles (2 × 2) o dos.
    ToggleQuad,
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
    /// Un botón propio de la barra: el comando sobre lo seleccionado ahora.
    RunCommand(String),
    /// Abrir la ventana para elegir los botones de la barra.
    CustomizeToolbar,
    /// Guardar las pestañas de ahora como espacio de trabajo (con este nombre).
    SaveWorkspace(String),
    OpenWorkspace(String),
    DeleteWorkspace(String),
    /// Alias (lo que se escribe en la paleta) → carpeta.
    SetAlias(String, String),
    /// Atajo propio (teclas, comando).
    SetShortcut(String, String),
    /// Abrir una carpeta en el panel de enfrente.
    OpenOther(String),
    /// Pestaña de la papelera.
    OpenRecycle,
    /// Vista plana de la pestaña actual (encender / apagar).
    ToggleFlat,
    /// Desplegar o plegar una carpeta dentro de la lista (ruta relativa a la carpeta de la pestaña).
    ToggleExpand(String),
    /// Pestaña de archivos recientes.
    OpenRecent,
    /// Todo lo que tiene esa etiqueta de color (en la pestaña actual).
    OpenTag(usize),
    /// Poner o quitar una etiqueta de color.
    ToggleTag(Vec<String>, usize),
    ClearTags(Vec<String>),
}
