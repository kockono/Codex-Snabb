//! Core: tipos compartidos, acciones, eventos, efectos, config, budgets e IDs.
//!
//! Este módulo es la base que todos los demás importan. Define el vocabulario
//! común del sistema: qué acciones existen, qué eventos se emiten, qué efectos
//! produce el reducer, la configuración base y los budgets de performance.
//!
//! Nada en este módulo ejecuta IO ni spawnea tareas. Es puro vocabulario tipado.

pub mod budgets;
pub mod command;
pub mod ids;
pub mod settings;

use std::path::PathBuf;

use crossterm::event::KeyEvent;

// ─── Action ────────────────────────────────────────────────────────────────────

/// Acciones que el usuario o el sistema pueden disparar.
///
/// Cada acción representa una intención. El reducer las procesa
/// y actualiza el estado. Ninguna acción ejecuta IO directamente.
///
/// Las variantes no usadas aún llevan `#[expect(dead_code)]` con razón.
/// Se van habilitando a medida que se implementan los subsistemas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Salir de la aplicación limpiamente sin chequear buffers dirty.
    /// DEPRECADO: el flujo recomendado es `QuitRequested` → modal de confirmación.
    /// Se conserva el handler en el reducer por si una integración externa lo emite.
    #[expect(
        dead_code,
        reason = "kept as low-level escape hatch; UI usa QuitRequested + modal"
    )]
    Quit,
    /// No-op: acción vacía que no produce cambios de estado.
    Noop,

    // ── Navegación ──
    /// Mover foco al siguiente panel en orden.
    /// Enfocar el Explorer directamente — abre sidebar si estaba cerrada.
    FocusExplorer,
    FocusNext,
    /// Mover foco al panel anterior en orden.
    FocusPrev,
    /// Mover foco a un panel específico.
    FocusPanel(PanelId),

    // ── Editor ──
    /// Insertar un carácter en la posición del cursor.
    InsertChar(char),
    /// Eliminar el carácter antes del cursor (backspace).
    DeleteChar,
    /// Insertar un salto de línea (Enter).
    InsertNewline,
    /// Mover el cursor en una dirección.
    MoveCursor(Direction),
    /// Mover cursor al inicio de la línea actual (Home).
    MoveToLineStart,
    /// Mover cursor al final de la línea actual (End).
    MoveToLineEnd,
    /// Mover cursor al inicio absoluto del buffer (Ctrl+Home).
    MoveToBufferStart,
    /// Mover cursor al final absoluto del buffer (Ctrl+End).
    MoveToBufferEnd,
    /// Deshacer la última operación de edición (Ctrl+Z).
    Undo,
    /// Rehacer la última operación deshecha (Ctrl+Y).
    Redo,

    /// Mover cursor por palabra (Ctrl+Left/Right).
    MoveCursorWord(Direction),
    /// Mover cursor por palabra extendiendo la selección (Ctrl+Shift+Left/Right).
    MoveCursorWordSelecting(Direction),
    /// Toggle line comment (Ctrl+/) según extensión del archivo.
    ToggleLineComment,
    /// Mover la línea del cursor primario hacia arriba o abajo (Alt+Up/Down).
    MoveLine(Direction),
    /// Duplicar la(s) línea(s) del cursor hacia arriba o abajo (Shift+Alt+Up/Down).
    DuplicateLine(Direction),
    /// Tab dentro del editor: indentar selección si existe, sino navegar foco.
    EditorTab,
    /// Shift+Tab dentro del editor: des-indentar selección si existe, sino foco previo.
    EditorBackTab,
    /// Seleccionar la línea completa del cursor primario (Ctrl+L).
    SelectLine,

    // ── Multicursor / Selección ──
    /// Agregar un cursor en la línea inmediatamente superior al primario (Ctrl+Alt+Up).
    AddCursorAbove,
    /// Agregar un cursor en la línea inmediatamente inferior al primario (Ctrl+Alt+Down).
    AddCursorBelow,
    /// Extender selección al inicio de la línea para todos los cursores (Shift+Home).
    MoveToLineStartSelecting,
    /// Extender selección al final de la línea para todos los cursores (Shift+End).
    MoveToLineEndSelecting,
    /// Extender selección al inicio absoluto del buffer (Ctrl+Shift+Home).
    MoveToBufferStartSelecting,
    /// Extender selección al final absoluto del buffer (Ctrl+Shift+End).
    MoveToBufferEndSelecting,
    /// Seleccionar la siguiente ocurrencia del texto seleccionado (Ctrl+D).
    SelectNextOccurrence,
    /// Limpiar cursores secundarios (Esc con multicursor activo).
    /// DEPRECADO: reemplazado por `EscapeHierarchy`. Se conserva en el enum
    /// para compatibilidad con dispatch de paneles que aún lo emiten.
    #[expect(
        dead_code,
        reason = "kept for transition; replaced by EscapeHierarchy"
    )]
    ClearMultiCursor,
    /// Esc jerárquico: limpia multicursor → selección → foco al editor → no-op.
    /// Reemplaza el comportamiento previo de `ClearMultiCursor` que cerraba la app.
    EscapeHierarchy,
    /// Solicitar quit de la aplicación (Ctrl+Q). Si hay buffers dirty,
    /// muestra el modal de confirmación; si no, sale inmediatamente.
    QuitRequested,
    /// Confirmar "Save All" en el modal de quit. Guarda buffers titled y
    /// arranca el flujo de Save As para los untitled.
    QuitConfirmSaveAll,
    /// Confirmar "Don't Save" en el modal de quit. Sale descartando cambios.
    QuitConfirmDiscard,
    /// Cancelar el modal de quit (Esc / botón Cancel). Mantiene la app abierta.
    QuitCancel,
    /// Mover foco al siguiente botón del modal de quit (Tab).
    QuitModalCycleNext,
    /// Mover foco al botón anterior del modal de quit (Shift+Tab).
    QuitModalCyclePrev,
    /// Mover cursor extendiendo la selección (Shift + flechas).
    MoveCursorSelecting(Direction),
    /// Seleccionar todo el contenido del buffer (Ctrl+A).
    SelectAll,
    /// Copiar la selección activa al portapapeles del sistema (Ctrl+C).
    CopySelection,
    /// Cortar la selección activa al portapapeles del sistema (Ctrl+X).
    CutSelection,
    /// Pegar el contenido del portapapeles del sistema (Ctrl+V).
    PasteClipboard,

    // ── Archivos ──
    /// Abrir un archivo por path.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente apertura de archivos via UI"
    )]
    OpenFile(PathBuf),
    /// Guardar el archivo del buffer activo.
    SaveFile,
    /// Cerrar el buffer activo.
    CloseBuffer,

    // ── Save As modal ──
    /// Abrir el modal "Guardar como" (buffer sin path asociado).
    #[expect(dead_code, reason = "disponible para keybinding futuro — se dispara via SaveFile en buffers untitled")]
    SaveAsOpen,
    /// Escribir un carácter en el input de path del modal Save As.
    SaveAsChar(char),
    /// Borrar el último carácter del input de path del modal Save As.
    SaveAsBackspace,
    /// Confirmar el path y guardar el buffer.
    SaveAsConfirm,
    /// Cancelar el modal Save As sin guardar.
    SaveAsCancel,

    // ── Rename modal ──
    /// Abrir el modal de rename para el path dado.
    /// Disponible para keybinding futuro — se dispara directamente desde ContextMenuItem::Rename.
    #[expect(dead_code, reason = "disponible para keybinding futuro — context menu llama state.rename.open() directamente")]
    RenameOpen(PathBuf),
    /// Escribir un carácter en el input de nombre del modal Rename.
    RenameChar(char),
    /// Borrar el último carácter del input de nombre del modal Rename.
    RenameBackspace,
    /// Confirmar y ejecutar el rename.
    RenameConfirm,
    /// Cancelar el modal Rename sin renombrar.
    RenameCancel,

    // ── Tabs ──
    /// Ir a la pestaña siguiente (Ctrl+Tab).
    NextTab,
    /// Ir a la pestaña anterior (Ctrl+Shift+Tab).
    PrevTab,
    /// Cerrar la pestaña activa (Ctrl+W).
    CloseTab,
    /// Cambiar a una pestaña por índice (click de mouse en tab).
    #[expect(dead_code, reason = "se dispara via mouse click en tabs — no hay keybinding directo")]
    SwitchTab(usize),

    // ── Comandos ──
    /// Abrir el command palette (Alt+Shift+P).
    OpenCommandPalette,
    /// Abrir quick open (Ctrl+P).
    OpenQuickOpen,

    // ── Search ──
    /// Abrir el panel de búsqueda global.
    OpenGlobalSearch,
    /// Cerrar el panel de búsqueda global.
    SearchClose,
    /// Insertar carácter en el campo activo del search.
    SearchInsertChar(char),
    /// Borrar carácter del campo activo del search.
    SearchDeleteChar,
    /// Siguiente campo de input en search (Tab).
    SearchNextField,
    /// Campo anterior de input en search (Shift+Tab).
    SearchPrevField,
    /// Ejecutar la búsqueda con las opciones actuales.
    #[expect(dead_code, reason = "disponible via command registry — Enter en search usa SearchSelectAndOpen")]
    SearchExecute,
    /// Navegar al siguiente match en resultados.
    SearchNextMatch,
    /// Navegar al match anterior en resultados.
    SearchPrevMatch,
    /// Toggle case sensitive en search.
    SearchToggleCase,
    /// Toggle whole word en search.
    SearchToggleWholeWord,
    /// Toggle regex en search.
    SearchToggleRegex,
    /// Toggle visibilidad del campo replace.
    SearchToggleReplace,
    /// Reemplazar el match seleccionado.
    SearchReplaceCurrent,
    /// Reemplazar todos los matches del archivo actual.
    SearchReplaceAllInFile,
    /// Toggle fold del file header seleccionado en resultados agrupados.
    SearchToggleFold,
    /// Toggle expansión/colapso de la fila de filtros (include/exclude).
    SearchToggleFilters,
    /// Abrir el match seleccionado y navegar al archivo/línea.
    SearchSelectAndOpen,

    // ── Terminal ──
    /// Alternar visibilidad del panel de terminal.
    ToggleTerminal,
    /// Enviar un carácter al terminal (legacy — se mantiene para compat del reducer).
    #[expect(dead_code, reason = "legacy — keymap ahora usa TerminalSendBytes; reducer mantiene handler")]
    TerminalInput(char),
    /// Enviar Enter al terminal (legacy — se mantiene para compat del reducer).
    #[expect(dead_code, reason = "legacy — keymap ahora usa TerminalSendBytes; reducer mantiene handler")]
    TerminalEnter,
    /// Enviar Ctrl+C al terminal (legacy — se mantiene para compat del reducer).
    #[expect(dead_code, reason = "legacy — keymap ahora usa TerminalSendBytes; reducer mantiene handler")]
    TerminalCtrlC,
    /// Scrollear output del terminal hacia arriba.
    TerminalScrollUp,
    /// Scrollear output del terminal hacia abajo.
    TerminalScrollDown,
    /// Crear nueva sesión de terminal si no existe.
    #[expect(dead_code, reason = "se dispara internamente via ToggleTerminal")]
    TerminalSpawn,

    // ── Terminal multi-pane ──
    /// Split horizontal del pane activo (side-by-side).
    TerminalSplitHorizontal,
    /// Split vertical del pane activo (top/bottom).
    TerminalSplitVertical,
    /// Cerrar el pane activo de terminal.
    TerminalClosePane,
    /// Mover foco al siguiente pane de terminal.
    TerminalFocusNext,
    /// Mover foco al pane anterior de terminal.
    TerminalFocusPrev,
    /// Mover foco a un pane específico por ID.
    #[expect(dead_code, reason = "reducer lo maneja — no hay keybinding directo aún, se usará via mouse click en pane")]
    TerminalFocusPane(u32),
    /// Bytes crudos para enviar al PTY del pane activo.
    /// Reemplaza conceptualmente TerminalInput/Enter/CtrlC.
    TerminalSendBytes(Vec<u8>),

    // ── File search (Ctrl+F dentro del editor) ──
    /// Abrir el search bar del archivo actual (Ctrl+F).
    OpenFileSearch,
    /// Insertar carácter en el query del file search.
    FileSearchInsertChar(char),
    /// Borrar carácter del query del file search (Backspace).
    FileSearchDeleteChar,
    /// Saltar al siguiente match del file search (Enter / F3).
    FileSearchNext,
    /// Saltar al match anterior del file search (Shift+Enter / Shift+F3).
    FileSearchPrev,
    /// Cerrar el file search (Esc).
    FileSearchClose,
    /// Toggle case-sensitive del file search (Alt+C).
    FileSearchToggleCase,

    // ── Explorer ──
    /// Mover selección arriba en el explorer.
    ExplorerUp,
    /// Mover selección abajo en el explorer.
    ExplorerDown,
    /// Toggle expand/collapse de directorio, o abrir archivo.
    ExplorerToggle,
    /// Refrescar el árbol del explorer desde disco.
    ExplorerRefresh,
    /// Colapsar directorio seleccionado en el explorer.
    ExplorerCollapse,
    /// Iniciar creación de archivo nuevo en el explorer (input modal inline).
    ExplorerNewFile,
    /// Iniciar creación de carpeta nueva en el explorer (input modal inline).
    ExplorerNewFolder,
    /// Eliminar el archivo o carpeta seleccionado en el explorer.
    ExplorerDeleteSelected,
    /// Insertar carácter en el input de nuevo archivo del explorer.
    ExplorerNewFileInput(char),
    /// Borrar carácter del input de nuevo archivo del explorer.
    ExplorerNewFileBackspace,
    /// Confirmar creación del nuevo archivo.
    ExplorerNewFileConfirm,
    /// Cancelar creación del nuevo archivo.
    ExplorerNewFileCancel,
    /// Insertar carácter en el input de nueva carpeta del explorer.
    ExplorerNewFolderInput(char),
    /// Borrar carácter del input de nueva carpeta del explorer.
    ExplorerNewFolderBackspace,
    /// Confirmar creación de la nueva carpeta.
    ExplorerNewFolderConfirm,
    /// Cancelar creación de la nueva carpeta.
    ExplorerNewFolderCancel,

    // ── Paneles ──
    /// Alternar visibilidad de la sidebar (Ctrl+B).
    ToggleSidebar,
    /// Alternar visibilidad del panel inferior (Ctrl+J).
    ToggleBottomPanel,

    // ── Mouse ──
    /// Click izquierdo del mouse en posición absoluta de terminal.
    MouseClick {
        /// Columna (0-indexed, coordenada de terminal).
        col: u16,
        /// Fila (0-indexed, coordenada de terminal).
        row: u16,
    },
    /// Scroll hacia arriba del mouse en posición absoluta de terminal.
    MouseScrollUp {
        /// Columna donde ocurrió el scroll.
        col: u16,
        /// Fila donde ocurrió el scroll.
        row: u16,
    },
    /// Scroll hacia abajo del mouse en posición absoluta de terminal.
    MouseScrollDown {
        /// Columna donde ocurrió el scroll.
        col: u16,
        /// Fila donde ocurrió el scroll.
        row: u16,
    },
    /// Click del botón del medio del mouse (rueda) en posición absoluta de terminal.
    /// Se usa para cerrar la tab sobre la que se hace click — igual que los browsers.
    MouseMiddleClick {
        /// Columna (0-indexed, coordenada de terminal).
        col: u16,
        /// Fila (0-indexed, coordenada de terminal).
        row: u16,
    },
    /// Drag del mouse (botón izquierdo presionado + movimiento).
    /// Se usa para selección de texto arrastrando el mouse.
    MouseDrag {
        /// Columna actual del drag (0-indexed, coordenada de terminal).
        col: u16,
        /// Fila actual del drag (0-indexed, coordenada de terminal).
        row: u16,
    },
    /// Right-click del mouse en posición absoluta de terminal.
    /// Abre el context menu del panel bajo el cursor.
    MouseRightClick {
        /// Columna (0-indexed, coordenada de terminal).
        col: u16,
        /// Fila (0-indexed, coordenada de terminal).
        row: u16,
    },

    // ── Git ──
    /// Abrir el panel de Git / source control.
    OpenGitPanel,
    /// Cerrar el panel de Git.
    GitClose,
    /// Refrescar status del repo git.
    GitRefresh,
    /// Mover selección arriba en la lista de archivos git.
    GitUp,
    /// Mover selección abajo en la lista de archivos git.
    GitDown,
    /// Toggle stage/unstage del archivo seleccionado.
    GitStageToggle,
    /// Stage o unstage un archivo específico por índice (click en `[+]`/`[-]` del row).
    GitStageFile(usize),
    /// Descartar cambios del working tree de un archivo específico por índice
    /// (click en `[×]` del row). Solo aplica a archivos unstaged Modified/Deleted.
    GitDiscardFile(usize),
    /// Stage de todos los archivos unstaged a la vez (click en `[+]` del header "Changes").
    GitStageAll,
    GitUnstageAll,
    /// Toggle mostrar/ocultar diff del archivo seleccionado.
    ///
    /// En el modelo de tabs virtuales de diff, abre (o reusa) una tab de diff
    /// para el archivo seleccionado. Si la tab activa ya es esa tab de diff
    /// y hay más de una tab abierta, la cierra (toggle).
    GitToggleDiff,
    /// Abre el diff del archivo seleccionado como tab virtual.
    ///
    /// Idempotente: si ya hay una tab de diff para el archivo, la activa
    /// y refresca su contenido. Equivalente a `GitToggleDiff` cuando la
    /// tab activa NO es la del diff de ese archivo.
    #[expect(
        dead_code,
        reason = "se construirá desde el command palette / context menu — alias semántico de GitToggleDiff"
    )]
    GitOpenDiffTab,
    /// Scrollear diff hacia arriba.
    GitDiffScrollUp,
    /// Scrollear diff hacia abajo.
    GitDiffScrollDown,
    /// Entrar en modo commit (escribir mensaje).
    GitStartCommit,
    /// Ejecutar el commit con el mensaje actual.
    GitCommitConfirm,
    /// Cancelar el modo commit.
    GitCommitCancel,
    /// Insertar carácter en el mensaje de commit.
    GitCommitInput(char),
    /// Borrar último carácter del mensaje de commit.
    GitCommitDeleteChar,
    /// Ejecutar git fetch para sincronizar con el remoto.
    GitFetch,
    /// Ejecutar git push para enviar commits al remoto.
    GitPush,
    /// Ejecutar git pull para traer cambios del remoto.
    GitPull,

    // ── LSP ──
    /// Arrancar el language server para el archivo actual.
    LspStart,
    /// Detener el language server activo.
    LspStop,
    /// Solicitar hover info en la posición del cursor.
    LspHover,
    /// Solicitar go-to-definition en la posición del cursor.
    LspGotoDefinition,
    /// Abrir autocompletado LSP en la posición del cursor.
    LspCompletion,
    /// Mover selección arriba en la lista de completions.
    LspCompletionUp,
    /// Mover selección abajo en la lista de completions.
    LspCompletionDown,
    /// Confirmar e insertar el completion seleccionado.
    LspCompletionConfirm,
    /// Cerrar la lista de completions.
    LspCompletionCancel,

    // ── Command Palette ──
    /// Mover selección arriba en la palette.
    PaletteUp,
    /// Mover selección abajo en la palette.
    PaletteDown,
    /// Insertar carácter en el input de la palette.
    PaletteInsertChar(char),
    /// Borrar carácter del input de la palette.
    PaletteDeleteChar,
    /// Confirmar y ejecutar el comando seleccionado en la palette.
    PaletteConfirm,
    /// Cerrar la command palette.
    PaletteClose,

    // ── Go to Line ──
    /// Abrir modal Go to Line (Ctrl+G).
    OpenGoToLine,
    /// Insertar dígito en Go to Line.
    GoToLineInsertChar(char),
    /// Borrar último dígito en Go to Line.
    GoToLineDeleteChar,
    /// Confirmar y saltar a la línea.
    GoToLineConfirm,
    /// Cancelar Go to Line.
    GoToLineClose,

    // ── Quick Open ──
    /// Mover selección arriba en el quick open.
    QuickOpenUp,
    /// Mover selección abajo en el quick open.
    QuickOpenDown,
    /// Insertar carácter en el input del quick open.
    QuickOpenInsertChar(char),
    /// Borrar carácter del input del quick open.
    QuickOpenDeleteChar,
    /// Confirmar y abrir el archivo seleccionado en el quick open.
    QuickOpenConfirm,
    /// Cerrar el quick open.
    QuickOpenClose,

    // ── Branch Picker ──
    /// Abrir el branch picker (click en branch de status bar).
    #[expect(
        dead_code,
        reason = "se dispara via mouse click directo — disponible para keybinding futuro"
    )]
    BranchPickerOpen,
    /// Cerrar el branch picker (Esc).
    BranchPickerClose,
    /// Mover selección arriba en el branch picker.
    BranchPickerUp,
    /// Mover selección abajo en el branch picker.
    BranchPickerDown,
    /// Insertar carácter en el input del branch picker.
    BranchPickerInsertChar(char),
    /// Borrar carácter del input del branch picker.
    BranchPickerDeleteChar,
    /// Confirmar y hacer checkout de la rama seleccionada.
    BranchPickerConfirm,

    // ── Settings ──
    /// Abrir el overlay de settings (keybindings editor).
    SettingsOpen,
    /// Cerrar el overlay de settings.
    SettingsClose,
    /// Mover selección arriba en la tabla de keybindings.
    SettingsUp,
    /// Mover selección abajo en la tabla de keybindings.
    SettingsDown,
    /// Insertar carácter en el campo de búsqueda del settings.
    SettingsSearchInsert(char),
    /// Borrar carácter del campo de búsqueda del settings.
    SettingsSearchDelete,
    /// Empezar a editar el keybind del entry seleccionado.
    SettingsStartEdit,
    /// Cancelar la edición del keybind.
    SettingsCancelEdit,
    /// Capturar un KeyEvent como nuevo keybind en modo edición.
    SettingsCaptureKey(KeyEvent),
    /// Quitar el keybind del entry seleccionado.
    SettingsRemoveKeybind,

    // ── Activity Bar ──
    /// Click en un icono de la activity bar para cambiar sección de sidebar.
    ActivityBarSelect(crate::core::settings::SidebarSection),

    // ── Projects panel ──
    /// Abrir diálogo nativo del SO para agregar nuevo proyecto.
    ProjectsAddNew,
    /// El diálogo nativo retornó una carpeta seleccionada por el usuario.
    /// Contiene el path absoluto de la carpeta elegida.
    ProjectsNativePickerResult(std::path::PathBuf),
    /// Cancelar folder picker sin agregar proyecto.
    #[expect(dead_code, reason = "disponible via command registry — FolderPickerCancel se usa directamente")]
    ProjectsCancelAdd,
    /// Seleccionar un proyecto de la lista (índice).
    #[expect(dead_code, reason = "disponible para keybinding directo — se usa via ProjectsOpen")]
    ProjectsSelect(usize),
    /// Toggle del candado de un proyecto (índice).
    ProjectsToggleLock(usize),
    /// Eliminar proyecto de la lista (índice).
    ProjectsRemove(usize),
    /// Navegar arriba en la lista de proyectos.
    ProjectsMoveUp,
    /// Navegar abajo en la lista de proyectos.
    ProjectsMoveDown,
    /// Activar/abrir el proyecto seleccionado (switch workspace).
    ProjectsOpen,

    // ── Context menu (menú contextual del explorer) ──
    /// Abrir context menu en posición (x, y) para el path del explorer seleccionado.
    /// Se dispara via MouseRightClick — no hay keybinding directo.
    ContextMenuOpen {
        /// Columna de terminal donde aparece el menú.
        x: u16,
        /// Fila de terminal donde aparece el menú.
        y: u16,
    },
    /// Cerrar el context menu.
    ContextMenuClose,
    /// Mover selección arriba en el context menu.
    ContextMenuUp,
    /// Mover selección abajo en el context menu.
    ContextMenuDown,
    /// Confirmar el item seleccionado en el context menu.
    ContextMenuConfirm,

    // ── Folder picker (modal de selección de carpeta) ──
    /// Navegar arriba en el folder picker.
    FolderPickerUp,
    /// Navegar abajo en el folder picker.
    FolderPickerDown,
    /// Expandir/navegar al directorio seleccionado en el picker.
    FolderPickerEnter,
    /// Subir al directorio padre en el picker.
    FolderPickerParent,
    /// Confirmar directorio actual como proyecto.
    FolderPickerConfirm,
    /// Cancelar el folder picker.
    FolderPickerCancel,
    /// Alternar foco entre el input de path y el árbol en el folder picker.
    FolderPickerToggleFocus,
    /// Insertar un carácter en el input de path del folder picker.
    FolderPickerPathInput(char),
    /// Borrar último carácter del input de path del folder picker.
    FolderPickerPathBackspace,
    /// Confirmar el path escrito en el input (navegar al directorio).
    FolderPickerPathConfirm,
    /// Limpiar input de path y devolver foco al árbol (sin cerrar picker).
    FolderPickerPathEscape,
}

// ─── Event ─────────────────────────────────────────────────────────────────────

/// Eventos internos del sistema.
///
/// Los eventos llegan al event loop desde distintas fuentes:
/// input de teclado, ticks del scheduler, o respuestas de workers.
/// El event loop los convierte en `Action` via keymap.
#[derive(Debug)]
pub enum Event {
    /// Evento de input del usuario (teclado, mouse, resize).
    Input(crossterm::event::Event),
    /// Tick periódico para tareas de mantenimiento.
    Tick,

    // ── Respuestas de workers ──
    /// Archivo cargado exitosamente por el worker de filesystem.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente carga async de archivos"
    )]
    FileLoaded {
        /// Path del archivo cargado.
        path: PathBuf,
        /// Contenido completo del archivo.
        content: String,
    },
    /// Error al cargar un archivo.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente carga async de archivos"
    )]
    FileError {
        /// Path del archivo que falló.
        path: PathBuf,
        /// Descripción del error.
        error: String,
    },
    /// Resultados de búsqueda global (placeholder — se definirá en épica 6).
    #[expect(dead_code, reason = "se definirá en épica 6 — global search")]
    SearchResult,
    /// Estado de Git actualizado (placeholder — se definirá en épica 9).
    #[expect(dead_code, reason = "se definirá en épica 9 — git panel")]
    GitStatus,

    // ── Lifecycle ──
    /// Terminal redimensionada.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente re-layout en resize"
    )]
    Resize(u16, u16),
    /// Señal de shutdown externo.
    #[expect(dead_code, reason = "se usará cuando se implemente shutdown por señal")]
    Shutdown,
}

// ─── Effect ────────────────────────────────────────────────────────────────────

/// Efectos que el reducer produce para workers.
///
/// El reducer es puro: recibe estado + acción, produce nuevo estado + efectos.
/// Los efectos se despachan a workers async. Esto separa lógica de IO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Cargar un archivo desde disco (async via worker).
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente carga async de archivos"
    )]
    LoadFile(PathBuf),
    /// Guardar contenido a disco (async via worker).
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente guardado de archivos"
    )]
    SaveFile {
        /// Path destino.
        path: PathBuf,
        /// Contenido a escribir.
        content: String,
    },
    /// Ejecutar búsqueda global (placeholder — se definirá en épica 6).
    #[expect(dead_code, reason = "se definirá en épica 6 — global search")]
    RunSearch,
    /// Refrescar estado de Git (async via worker).
    #[expect(dead_code, reason = "se usará en épica 9 — git panel")]
    RefreshGitStatus,
    /// Crear una nueva sesión de terminal.
    #[expect(dead_code, reason = "se usará en épica 7 — terminal integrada")]
    SpawnTerminal,
    /// Señal para terminar el event loop.
    Quit,
    /// Sin efecto — el reducer no necesita despachar nada.
    #[expect(
        dead_code,
        reason = "se usa como retorno explícito de 'sin efecto' cuando se necesite"
    )]
    None,
}

// ─── PanelId ───────────────────────────────────────────────────────────────────

/// Identificador de panel en el layout principal.
///
/// Cada variante corresponde a un panel o overlay del IDE.
/// Se usa para tracking de foco y navegación entre panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// Panel principal del editor de texto.
    Editor,
    /// Panel lateral del explorador de archivos.
    Explorer,
    /// Panel inferior de terminal integrada.
    Terminal,
    /// Panel de búsqueda global.
    Search,
    /// Panel de Git / source control.
    Git,
    /// Overlay del command palette.
    #[expect(
        dead_code,
        reason = "definido para tracking de foco modal — no se construye directamente"
    )]
    CommandPalette,
    /// Overlay del quick open.
    #[expect(dead_code, reason = "se usará en épica 5 — quick open")]
    QuickOpen,
    /// Panel de proyectos guardados.
    Projects,
}



// ─── Direction ─────────────────────────────────────────────────────────────────

/// Dirección de movimiento del cursor.
///
/// Usada por `Action::MoveCursor` para indicar dirección sin ambigüedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Mover hacia arriba (línea anterior).
    Up,
    /// Mover hacia abajo (línea siguiente).
    Down,
    /// Mover hacia la izquierda (carácter anterior).
    Left,
    /// Mover hacia la derecha (carácter siguiente).
    Right,
}

// ─── AppConfig ─────────────────────────────────────────────────────────────────

/// Configuración base de la aplicación.
///
/// Valores por defecto razonables para cada subsistema.
/// Se expandirá con feature flags y opciones de subsistemas.
#[derive(Debug)]
pub struct AppConfig {
    /// Intervalo de tick en milisegundos para el event loop.
    pub tick_rate_ms: u64,
    /// Throttle mínimo entre renders en milisegundos (~60fps = 16ms).
    #[expect(
        dead_code,
        reason = "se usará para throttling de render en el event loop"
    )]
    pub render_throttle_ms: u64,
    /// Tamaño máximo de archivo en bytes que el editor acepta abrir.
    #[expect(dead_code, reason = "se usará para validación al abrir archivos")]
    pub max_file_size_bytes: u64,
    /// Líneas de scrollback máximas por sesión de terminal.
    #[expect(dead_code, reason = "se usará en épica 7 — terminal integrada")]
    pub terminal_scrollback: usize,
    /// Resultados máximos que retorna una búsqueda global.
    pub search_max_results: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            // 50ms = ~20 polls/segundo. Buen balance entre responsividad
            // y CPU idle. 250ms se sentía como lag visible en la TUI.
            tick_rate_ms: 50,
            render_throttle_ms: 16,
            max_file_size_bytes: 10 * 1024 * 1024, // 10 MB
            terminal_scrollback: 5_000,
            search_max_results: 1_000,
        }
    }
}

impl AppConfig {
    /// Crea una configuración con valores por defecto razonables.
    pub fn new() -> Self {
        Self::default()
    }
}
