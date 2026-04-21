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
    /// Salir de la aplicación limpiamente.
    Quit,
    /// No-op: acción vacía que no produce cambios de estado.
    Noop,

    // ── Navegación ──
    /// Mover foco al siguiente panel en orden.
    FocusNext,
    /// Mover foco al panel anterior en orden.
    FocusPrev,
    /// Mover foco a un panel específico.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente navegación directa a paneles"
    )]
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
    #[expect(
        dead_code,
        reason = "se habilitará cuando se agregue keybinding Ctrl+Home"
    )]
    MoveToBufferStart,
    /// Mover cursor al final absoluto del buffer (Ctrl+End).
    #[expect(
        dead_code,
        reason = "se habilitará cuando se agregue keybinding Ctrl+End"
    )]
    MoveToBufferEnd,
    /// Deshacer la última operación de edición (Ctrl+Z).
    Undo,
    /// Rehacer la última operación deshecha (Ctrl+Y).
    Redo,

    // ── Multicursor / Selección ──
    /// Seleccionar la siguiente ocurrencia del texto seleccionado (Ctrl+D).
    SelectNextOccurrence,
    /// Limpiar cursores secundarios (Esc con multicursor activo).
    ClearMultiCursor,
    /// Mover cursor extendiendo la selección (Shift + flechas).
    MoveCursorSelecting(Direction),

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
    /// Abrir el match seleccionado y navegar al archivo/línea.
    SearchSelectAndOpen,

    // ── Terminal ──
    /// Alternar visibilidad del panel de terminal.
    ToggleTerminal,
    /// Enviar un carácter al terminal.
    TerminalInput(char),
    /// Enviar Enter al terminal.
    TerminalEnter,
    /// Enviar Ctrl+C al terminal.
    TerminalCtrlC,
    /// Scrollear output del terminal hacia arriba.
    TerminalScrollUp,
    /// Scrollear output del terminal hacia abajo.
    TerminalScrollDown,
    /// Crear nueva sesión de terminal si no existe.
    #[expect(dead_code, reason = "se dispara internamente via ToggleTerminal")]
    TerminalSpawn,

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
    /// Toggle mostrar/ocultar diff del archivo seleccionado.
    GitToggleDiff,
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
}

impl PanelId {
    /// Paneles navegables en orden de ciclo (Tab / Shift+Tab).
    ///
    /// Solo incluye paneles persistentes, no overlays como CommandPalette
    /// o QuickOpen que son modales y capturan foco mientras están abiertos.
    const CYCLE_ORDER: &[PanelId] = &[PanelId::Explorer, PanelId::Editor, PanelId::Terminal];

    /// Retorna el siguiente panel en el ciclo de navegación.
    pub fn next(self) -> Self {
        let current_idx = Self::CYCLE_ORDER
            .iter()
            .position(|&p| p == self)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % Self::CYCLE_ORDER.len();
        Self::CYCLE_ORDER[next_idx]
    }

    /// Retorna el panel anterior en el ciclo de navegación.
    pub fn prev(self) -> Self {
        let current_idx = Self::CYCLE_ORDER
            .iter()
            .position(|&p| p == self)
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            Self::CYCLE_ORDER.len() - 1
        } else {
            current_idx - 1
        };
        Self::CYCLE_ORDER[prev_idx]
    }
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
