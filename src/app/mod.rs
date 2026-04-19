//! App: bootstrap, event loop, scheduler, wiring general.
//!
//! Punto de entrada de la lógica de la aplicación. Configura la terminal,
//! ejecuta el event loop principal, y garantiza cleanup limpio en cualquier
//! caso (éxito, error, panic). El event loop sigue el modelo:
//!
//! ```text
//! input -> keymap -> Action -> reduce(state) -> Vec<Effect> -> process effects
//! ```
//!
//! El reducer es puro: recibe estado + acción, retorna efectos.
//! Los efectos se procesan después (por ahora solo `Effect::Quit`).
//! Métricas se registran en cada ciclo via `FrameTimer`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    cursor::SetCursorStyle,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode,
        KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio_util::sync::CancellationToken;

use crate::core::command::CommandRegistry;
use crate::core::{Action, AppConfig, Direction, Effect, Event, PanelId};
use crate::editor::EditorState;
use crate::git::GitState;
use crate::lsp::LspState;
use crate::observe::{FrameTimer, Metrics};
use crate::search::SearchState;
use crate::terminal::TerminalState;
use crate::ui::layout::IdeLayout;
use crate::ui::{self, Theme};
use crate::ui::palette::PaletteState;
use crate::workspace::ExplorerState;
use crate::workspace::QuickOpenState;

// ─── AppState ──────────────────────────────────────────────────────────────────

/// Estado central de la aplicación.
///
/// Contiene todo el estado mutable del sistema. El reducer lo modifica
/// en respuesta a acciones y produce efectos. Cada subsistema agrega
/// su sub-estado acá.
#[derive(Debug)]
pub struct AppState {
    /// Si la aplicación sigue ejecutando.
    pub running: bool,
    /// Panel que tiene el foco actualmente.
    pub focused_panel: PanelId,
    /// Configuración de la aplicación.
    pub config: AppConfig,
    /// Métricas de performance del sistema.
    pub metrics: Metrics,
    /// Si la sidebar está visible.
    pub sidebar_visible: bool,
    /// Si el bottom panel está visible.
    pub bottom_panel_visible: bool,
    /// Estado del editor de texto.
    pub editor: EditorState,
    /// Estado del explorador de archivos.
    pub explorer: Option<ExplorerState>,
    /// Registry central de comandos del sistema.
    pub commands: CommandRegistry,
    /// Estado de la command palette (overlay Ctrl+Shift+P).
    pub palette: PaletteState,
    /// Estado del quick open (overlay Ctrl+P).
    pub quick_open: QuickOpenState,
    /// Estado del panel de búsqueda global (Ctrl+Shift+F).
    pub search: SearchState,
    /// Estado de la terminal integrada (PTY + scrollback).
    pub terminal: TerminalState,
    /// Estado del panel de Git / source control.
    pub git: GitState,
    /// Estado del subsistema LSP (language server protocol).
    pub lsp: LspState,
    /// Datos pre-computados para la status bar (se actualizan en cada frame).
    /// Evita allocaciones dentro del render — se computan antes.
    pub status_line: String,
    pub status_file: String,
    /// Layout del último frame renderizado, para resolver posiciones de mouse.
    /// Se actualiza cada frame antes del render. `IdeLayout` es Copy (struct de Rects).
    pub last_layout: Option<IdeLayout>,
}

impl AppState {
    /// Crea un nuevo estado con valores por defecto y editor vacío.
    ///
    /// Intenta inicializar el explorer con el directorio de trabajo actual.
    /// Si falla, el explorer queda como `None` — la app funciona sin él.
    fn new(config: AppConfig) -> Self {
        let cwd = std::env::current_dir().ok();

        let explorer = cwd.as_deref().and_then(|cwd| {
            ExplorerState::new(cwd)
                .map_err(|e| tracing::warn!(error = %e, "no se pudo inicializar explorer"))
                .ok()
        });

        let mut commands = CommandRegistry::new();
        commands.register_defaults();

        // Construir índice de quick open desde el workspace root
        let mut quick_open = QuickOpenState::new();
        if let Some(ref root) = cwd
            && let Err(e) = quick_open.build_index(root)
        {
            tracing::warn!(error = %e, "no se pudo construir índice de quick open");
        }

        // Inicializar git state — verificar si es repo y refrescar
        let mut git = GitState::new();
        if let Some(ref cwd) = cwd {
            git.refresh(cwd);
        }

        Self {
            running: true,
            focused_panel: PanelId::Editor,
            config,
            metrics: Metrics::new(),
            sidebar_visible: true,
            bottom_panel_visible: true,
            editor: EditorState::new(),
            explorer,
            commands,
            palette: PaletteState::new(),
            quick_open,
            search: SearchState::new(),
            terminal: TerminalState::new(),
            git,
            lsp: LspState::new(),
            status_line: String::from("Ln 1, Col 1"),
            status_file: String::from("[no file]"),
            last_layout: None,
        }
    }

    /// Crea un nuevo estado con un archivo abierto.
    ///
    /// El explorer se inicializa con el directorio del archivo si tiene
    /// uno, o con el directorio de trabajo actual como fallback.
    fn with_file(config: AppConfig, path: &std::path::Path) -> Result<Self> {
        let editor = EditorState::open_file(path)?;
        let status_file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("[no file]"));

        // Derivar directorio para el explorer: padre del archivo o cwd
        let explorer_root = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());

        let explorer = explorer_root.as_deref().and_then(|root| {
            ExplorerState::new(root)
                .map_err(|e| tracing::warn!(error = %e, "no se pudo inicializar explorer"))
                .ok()
        });

        let mut commands = CommandRegistry::new();
        commands.register_defaults();

        // Construir índice de quick open desde el workspace root
        let mut quick_open = QuickOpenState::new();
        if let Some(ref root) = explorer_root
            && let Err(e) = quick_open.build_index(root)
        {
            tracing::warn!(error = %e, "no se pudo construir índice de quick open");
        }

        // Inicializar git state desde el directorio del explorer
        let mut git = GitState::new();
        if let Some(ref root) = explorer_root {
            git.refresh(root);
        }

        Ok(Self {
            running: true,
            focused_panel: PanelId::Editor,
            config,
            metrics: Metrics::new(),
            sidebar_visible: true,
            bottom_panel_visible: true,
            editor,
            explorer,
            commands,
            palette: PaletteState::new(),
            quick_open,
            search: SearchState::new(),
            terminal: TerminalState::new(),
            git,
            lsp: LspState::new(),
            status_line: String::from("Ln 1, Col 1"),
            status_file,
            last_layout: None,
        })
    }

    /// Actualiza los strings pre-computados de la status bar.
    ///
    /// Se llama después de cualquier acción que modifique el cursor o el buffer.
    /// Reutiliza la capacidad existente del String para minimizar allocaciones.
    fn update_status_cache(&mut self) {
        // Actualizar posición del cursor primario (1-indexed para display)
        self.status_line.clear();
        // Escribir sin format!() — usamos write! con buffer reutilizado
        use std::fmt::Write;
        let primary = self.editor.cursors.primary();
        let _ = write!(
            self.status_line,
            "Ln {}, Col {}",
            primary.position.line + 1,
            primary.position.col + 1
        );

        // Actualizar nombre de archivo
        if let Some(path) = self.editor.buffer.file_path() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if self.editor.buffer.is_dirty() {
                self.status_file.clear();
                let _ = write!(self.status_file, "{name} [+]");
            } else {
                self.status_file.clear();
                self.status_file.push_str(&name);
            }
        } else {
            self.status_file.clear();
            self.status_file.push_str("[no file]");
        }
    }
}

// ─── Keymap ────────────────────────────────────────────────────────────────────

/// Mapea un evento de crossterm a una acción del sistema.
///
/// El keymap es CONTEXT-AWARE — las mismas teclas producen acciones
/// distintas según el panel enfocado y los overlays activos:
///
/// - **Quick Open abierto**: captura TODO el input
/// - **Palette abierta**: captura TODO el input
/// - **Search panel activo**: captura input cuando sidebar tiene foco
/// - **Git panel activo**: captura input cuando sidebar tiene foco
/// - **Global**: Ctrl+atajos, Esc, Tab (siempre activos cuando overlays cerrados)
/// - **Editor**: flechas mueven cursor, chars insertan texto
/// - **Explorer**: flechas navegan el árbol, Enter abre/expande
fn keymap(
    event: &crossterm::event::Event,
    focused_panel: PanelId,
    palette_visible: bool,
    quick_open_visible: bool,
    search_visible: bool,
    git_state: &GitState,
    lsp_completion_visible: bool,
) -> Action {
    // ── Eventos de mouse ── se procesan ANTES del match de teclado
    if let CrosstermEvent::Mouse(mouse) = event {
        return match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => Action::MouseClick {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::ScrollUp => Action::MouseScrollUp {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::ScrollDown => Action::MouseScrollDown {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::Drag(MouseButton::Left) => Action::MouseDrag {
                col: mouse.column,
                row: mouse.row,
            },
            _ => Action::Noop,
        };
    }

    let CrosstermEvent::Key(key) = event else {
        return Action::Noop;
    };
    if key.kind != KeyEventKind::Press {
        return Action::Noop;
    }

    // ── LSP Completion visible: captura navegación, Enter, Esc ──
    if lsp_completion_visible && focused_panel == PanelId::Editor {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => return Action::LspCompletionCancel,
            (KeyCode::Enter, KeyModifiers::NONE) => return Action::LspCompletionConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => return Action::LspCompletionUp,
            (KeyCode::Down, KeyModifiers::NONE) => return Action::LspCompletionDown,
            // Tab = confirmar (estilo VS Code)
            (KeyCode::Tab, KeyModifiers::NONE) => return Action::LspCompletionConfirm,
            // Otros caracteres: cerrar completions y dejar pasar la acción normal
            _ => {
                // No capturar — caer al flujo normal para que chars se inserten
            }
        }
    }

    // ── Quick Open abierto: captura TODO el input ──
    if quick_open_visible {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::QuickOpenClose,
            (KeyCode::Enter, _) => Action::QuickOpenConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => Action::QuickOpenUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::QuickOpenDown,
            // Ctrl+P / Ctrl+N para vim-style navigation
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::QuickOpenUp,
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::QuickOpenDown,
            (KeyCode::Backspace, _) => Action::QuickOpenDeleteChar,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::QuickOpenInsertChar(ch)
            }
            // Cualquier otra tecla NO se propaga
            _ => Action::Noop,
        };
    }

    // ── Palette abierta: captura TODO el input ──
    if palette_visible {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::PaletteClose,
            (KeyCode::Enter, _) => Action::PaletteConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => Action::PaletteUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::PaletteDown,
            // Ctrl+P / Ctrl+N para vim-style navigation
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::PaletteUp,
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::PaletteDown,
            (KeyCode::Backspace, _) => Action::PaletteDeleteChar,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::PaletteInsertChar(ch)
            }
            // Cualquier otra tecla NO se propaga
            _ => Action::Noop,
        };
    }

    // ── Search panel activo: captura input cuando foco en Search ──
    if search_visible && focused_panel == PanelId::Search {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::SearchClose,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::SearchExecute,
            (KeyCode::Tab, KeyModifiers::NONE) => Action::SearchNextField,
            (KeyCode::BackTab, KeyModifiers::SHIFT) => Action::SearchPrevField,
            (KeyCode::Up, KeyModifiers::NONE) => Action::SearchPrevMatch,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SearchNextMatch,
            (KeyCode::F(3), KeyModifiers::NONE) => Action::SearchNextMatch,
            (KeyCode::F(3), mods) if mods.contains(KeyModifiers::SHIFT) => Action::SearchPrevMatch,
            (KeyCode::Backspace, _) => Action::SearchDeleteChar,
            // Alt+C → toggle case sensitive
            (KeyCode::Char('c'), KeyModifiers::ALT) => Action::SearchToggleCase,
            // Alt+W → toggle whole word
            (KeyCode::Char('w'), KeyModifiers::ALT) => Action::SearchToggleWholeWord,
            // Alt+R → toggle regex
            (KeyCode::Char('r'), KeyModifiers::ALT) => Action::SearchToggleRegex,
            // Ctrl+Shift+H → toggle replace
            (KeyCode::Char('H'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchToggleReplace
            }
            // Ctrl+Shift+1 → replace current
            (KeyCode::Char('!'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchReplaceCurrent
            }
            // Ctrl+Shift+A en search → replace all in file
            (KeyCode::Char('A'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchReplaceAllInFile
            }
            // Chars se escriben en el campo activo
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::SearchInsertChar(ch)
            }
            _ => Action::Noop,
        };
    }

    // ── Git panel activo: captura input cuando foco en Git ──
    if git_state.visible && focused_panel == PanelId::Git {
        // Modo commit: capturar chars para el mensaje
        if git_state.commit_mode {
            return match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => Action::GitCommitCancel,
                (KeyCode::Enter, KeyModifiers::NONE) => Action::GitCommitConfirm,
                (KeyCode::Backspace, _) => Action::GitCommitDeleteChar,
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    Action::GitCommitInput(ch)
                }
                _ => Action::Noop,
            };
        }

        // Modo diff: navegación del diff
        if git_state.show_diff {
            return match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    Action::GitToggleDiff
                }
                (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::GitDiffScrollUp,
                (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Action::GitDiffScrollDown
                }
                _ => Action::Noop,
            };
        }

        // Modo normal: navegación de lista de archivos
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::GitClose,
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::GitUp,
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Action::GitDown,
            (KeyCode::Enter, KeyModifiers::NONE) | (KeyCode::Char('s'), KeyModifiers::NONE) => {
                Action::GitStageToggle
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => Action::GitToggleDiff,
            (KeyCode::Char('c'), KeyModifiers::NONE) => Action::GitStartCommit,
            (KeyCode::Char('r'), KeyModifiers::NONE) => Action::GitRefresh,
            _ => Action::Noop,
        };
    }

    // ── Atajos globales (Ctrl+algo, Esc, Tab) ──
    match (key.code, key.modifiers) {
        // Esc: si hay multicursor activo, limpiar; sino, quit
        (KeyCode::Esc, _) => return Action::ClearMultiCursor,
        (KeyCode::Tab, KeyModifiers::NONE) => return Action::FocusNext,
        (KeyCode::BackTab, KeyModifiers::SHIFT) => return Action::FocusPrev,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => return Action::ToggleSidebar,
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => return Action::ToggleBottomPanel,
        // Ctrl+` abre/cierra terminal (con spawn automático si no hay sesión)
        (KeyCode::Char('`'), KeyModifiers::CONTROL) => return Action::ToggleTerminal,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => return Action::SaveFile,
        (KeyCode::Char('z'), KeyModifiers::CONTROL) => return Action::Undo,
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => return Action::Redo,
        // Ctrl+Shift+F abre búsqueda global.
        (KeyCode::Char('F'), mods)
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenGlobalSearch;
        }
        // Alt+Shift+P abre la command palette.
        // crossterm reporta Alt+Shift+P como 'P' mayúscula con ALT|SHIFT flags.
        (KeyCode::Char('P'), mods)
            if mods.contains(KeyModifiers::ALT) && mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenCommandPalette;
        }
        // Ctrl+P (sin Shift) abre quick open.
        // crossterm reporta Ctrl+P (sin Shift) como 'p' minúscula con CONTROL flag.
        // El guard !SHIFT asegura que no interfiera con Ctrl+Shift+P.
        (KeyCode::Char('p'), mods)
            if mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenQuickOpen;
        }
        _ => {}
    }

    // ── Context-aware: match sobre (panel enfocado, tecla) ──
    match focused_panel {
        PanelId::Terminal => match (key.code, key.modifiers) {
            // Esc sale del foco del terminal
            (KeyCode::Esc, _) => Action::FocusNext,
            // Ctrl+C → enviar Ctrl+C al terminal
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::TerminalCtrlC,
            // Enter → enviar Enter al terminal
            (KeyCode::Enter, KeyModifiers::NONE) => Action::TerminalEnter,
            // Shift+Up / PageUp → scroll up del terminal
            (KeyCode::Up, mods) if mods.contains(KeyModifiers::SHIFT) => Action::TerminalScrollUp,
            (KeyCode::PageUp, _) => Action::TerminalScrollUp,
            // Shift+Down / PageDown → scroll down del terminal
            (KeyCode::Down, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::TerminalScrollDown
            }
            (KeyCode::PageDown, _) => Action::TerminalScrollDown,
            // Caracteres → input al terminal
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::TerminalInput(ch)
            }
            _ => Action::Noop,
        },

        PanelId::Editor => match (key.code, key.modifiers) {
            // ── LSP ──
            // Ctrl+Space → autocompletado
            (KeyCode::Char(' '), KeyModifiers::CONTROL) => Action::LspCompletion,
            // F12 → go-to-definition
            (KeyCode::F(12), KeyModifiers::NONE) => Action::LspGotoDefinition,
            // Ctrl+K → hover (info de tipo)
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => Action::LspHover,

            // Movimiento de cursor
            (KeyCode::Up, KeyModifiers::NONE) => Action::MoveCursor(Direction::Up),
            (KeyCode::Down, KeyModifiers::NONE) => Action::MoveCursor(Direction::Down),
            (KeyCode::Left, KeyModifiers::NONE) => Action::MoveCursor(Direction::Left),
            (KeyCode::Right, KeyModifiers::NONE) => Action::MoveCursor(Direction::Right),
            (KeyCode::Home, KeyModifiers::NONE) => Action::MoveToLineStart,
            (KeyCode::End, KeyModifiers::NONE) => Action::MoveToLineEnd,

            // Shift + flechas → selección
            (KeyCode::Up, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Up)
            }
            (KeyCode::Down, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Down)
            }
            (KeyCode::Left, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Left)
            }
            (KeyCode::Right, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Right)
            }

            // Ctrl+D → seleccionar siguiente ocurrencia
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::SelectNextOccurrence,

            // Edición
            (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::InsertNewline,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::InsertChar(ch)
            }

            _ => Action::Noop,
        },

        PanelId::Explorer => match (key.code, key.modifiers) {
            // Navegación
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::ExplorerUp,
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Action::ExplorerDown,
            // Abrir/expandir
            (KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'), KeyModifiers::NONE) => {
                Action::ExplorerToggle
            }
            // Colapsar
            (KeyCode::Left | KeyCode::Char('h'), KeyModifiers::NONE) => Action::ExplorerCollapse,
            // Refresh
            (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ExplorerRefresh,

            _ => Action::Noop,
        },

        // Otros paneles — sin keybindings específicos aún
        _ => Action::Noop,
    }
}

// ─── Reducer ───────────────────────────────────────────────────────────────────

/// Reducer: actualiza estado según la acción y retorna efectos.
///
/// Delega las operaciones de edición al `EditorState`.
/// Las acciones de editor solo se procesan si el foco está en el Editor.
fn reduce(state: &mut AppState, action: &Action) -> Vec<Effect> {
    match action {
        Action::Quit => {
            state.running = false;
            vec![Effect::Quit]
        }
        Action::FocusNext => {
            state.focused_panel = state.focused_panel.next();
            tracing::debug!(panel = ?state.focused_panel, "foco siguiente");
            vec![]
        }
        Action::FocusPrev => {
            state.focused_panel = state.focused_panel.prev();
            tracing::debug!(panel = ?state.focused_panel, "foco anterior");
            vec![]
        }
        Action::ToggleSidebar => {
            state.sidebar_visible = !state.sidebar_visible;
            tracing::debug!(visible = state.sidebar_visible, "toggle sidebar");
            vec![]
        }
        Action::ToggleBottomPanel => {
            state.bottom_panel_visible = !state.bottom_panel_visible;
            tracing::debug!(visible = state.bottom_panel_visible, "toggle bottom panel");
            vec![]
        }

        // ── Acciones de editor ──
        Action::InsertChar(ch) => {
            // Si hay completions visibles y el char no es alfanumérico, cerrar
            if state.lsp.completion_visible && !ch.is_alphanumeric() && *ch != '_' {
                state.lsp.completion_visible = false;
                state.lsp.completions.clear();
            }
            state.editor.insert_char(*ch);
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::DeleteChar => {
            state.editor.delete_char();
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::InsertNewline => {
            // Cerrar completions al insertar newline
            state.lsp.completion_visible = false;
            state.lsp.completions.clear();
            state.editor.insert_newline();
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::MoveCursor(dir) => {
            state.editor.move_cursor(*dir, false);
            state.update_status_cache();
            // Limpiar hover al mover cursor
            state.lsp.hover_content = None;
            vec![]
        }
        Action::MoveCursorSelecting(dir) => {
            state.editor.move_cursor(*dir, true);
            state.update_status_cache();
            vec![]
        }
        Action::SelectNextOccurrence => {
            state.editor.select_next_occurrence();
            state.update_status_cache();
            vec![]
        }
        Action::ClearMultiCursor => {
            if state.editor.has_multicursors() {
                // Con multicursores activos, Esc limpia los secundarios
                state.editor.clear_multicursors();
                vec![]
            } else if state.editor.cursors.primary().has_selection() {
                // Con selección activa, Esc limpia la selección
                state.editor.cursors.primary_mut().clear_selection();
                vec![]
            } else {
                // Sin multicursor ni selección, Esc = Quit
                state.running = false;
                vec![Effect::Quit]
            }
        }
        Action::MoveToLineStart => {
            state.editor.move_to_line_start();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToLineEnd => {
            state.editor.move_to_line_end();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToBufferStart => {
            state.editor.move_to_buffer_start();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToBufferEnd => {
            state.editor.move_to_buffer_end();
            state.update_status_cache();
            vec![]
        }
        Action::Undo => {
            state.editor.undo();
            state.update_status_cache();
            vec![]
        }
        Action::Redo => {
            state.editor.redo();
            state.update_status_cache();
            vec![]
        }
        Action::SaveFile => {
            match state.editor.save() {
                Ok(()) => {
                    tracing::info!("archivo guardado");
                    state.update_status_cache();
                }
                Err(e) => {
                    tracing::error!(error = %e, "error al guardar archivo");
                }
            }
            vec![]
        }

        // ── Acciones del explorer ──
        Action::ExplorerUp => {
            if let Some(ref mut explorer) = state.explorer {
                explorer.move_up();
            }
            vec![]
        }
        Action::ExplorerDown => {
            if let Some(ref mut explorer) = state.explorer {
                explorer.move_down();
            }
            vec![]
        }
        Action::ExplorerToggle => {
            if let Some(ref mut explorer) = state.explorer {
                match explorer.toggle_selected() {
                    Ok(is_file) => {
                        if is_file {
                            // Abrir archivo en el editor
                            if let Some(path) = explorer.selected_path() {
                                match EditorState::open_file(&path) {
                                    Ok(editor) => {
                                        state.editor = editor;
                                        state.focused_panel = PanelId::Editor;
                                        state.update_status_cache();
                                        // Notificar LSP del nuevo archivo abierto
                                        if state.lsp.has_server() {
                                            let text = buffer_full_text(&state.editor);
                                            if let Err(e) = state.lsp.notify_open(&path, &text) {
                                                tracing::warn!(error = %e, "error en LSP did_open");
                                            }
                                        }
                                        tracing::info!(path = %path.display(), "archivo abierto desde explorer");
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "error al abrir archivo desde explorer");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "error en toggle del explorer");
                    }
                }
            }
            vec![]
        }
        Action::ExplorerCollapse => {
            if let Some(ref mut explorer) = state.explorer
                && let Err(e) = explorer.collapse_selected()
            {
                tracing::error!(error = %e, "error al colapsar directorio");
            }
            vec![]
        }
        Action::ExplorerRefresh => {
            if let Some(ref mut explorer) = state.explorer
                && let Err(e) = explorer.refresh()
            {
                tracing::error!(error = %e, "error al refrescar explorer");
            }
            vec![]
        }

        // ── Acciones de la Command Palette ──
        Action::OpenCommandPalette => {
            // Solo abrir si el quick open NO está visible (un overlay a la vez)
            if !state.quick_open.visible {
                state.palette.open(&state.commands);
                tracing::debug!("command palette abierta");
            }
            vec![]
        }
        Action::PaletteClose => {
            state.palette.close();
            tracing::debug!("command palette cerrada");
            vec![]
        }
        Action::PaletteUp => {
            state.palette.move_up();
            vec![]
        }
        Action::PaletteDown => {
            state.palette.move_down();
            vec![]
        }
        Action::PaletteInsertChar(ch) => {
            state.palette.insert_char(*ch, &state.commands);
            vec![]
        }
        Action::PaletteDeleteChar => {
            state.palette.delete_char(&state.commands);
            vec![]
        }
        Action::PaletteConfirm => {
            // Obtener la acción del comando seleccionado, cerrar palette,
            // y ejecutar la acción recursivamente via reduce.
            // CLONE: necesario — la action se extrae del registry (que es inmutable
            // durante reduce) y luego se pasa a reduce que toma &mut state.
            let selected_action = state
                .palette
                .selected_command(&state.commands)
                .map(|cmd| cmd.action.clone());
            state.palette.close();

            if let Some(action) = selected_action {
                tracing::info!(action = ?action, "palette: ejecutando comando");
                // Recursión de reduce — ejecutar la acción del comando
                return reduce(state, &action);
            }
            vec![]
        }

        // ── Acciones de mouse ──
        Action::MouseClick { col, row } => {
            reduce_mouse_click(state, *col, *row);
            vec![]
        }
        Action::MouseScrollUp { col, row } => {
            reduce_mouse_scroll(state, *col, *row, ScrollDirection::Up);
            vec![]
        }
        Action::MouseScrollDown { col, row } => {
            reduce_mouse_scroll(state, *col, *row, ScrollDirection::Down);
            vec![]
        }
        Action::MouseDrag { col, row } => {
            reduce_mouse_drag(state, *col, *row);
            vec![]
        }

        // ── Acciones del Quick Open ──
        Action::OpenQuickOpen => {
            // Solo abrir si la palette NO está visible (un overlay a la vez)
            if !state.palette.visible {
                state.quick_open.open();
                tracing::debug!("quick open abierto");
            }
            vec![]
        }
        Action::QuickOpenClose => {
            state.quick_open.close();
            tracing::debug!("quick open cerrado");
            vec![]
        }
        Action::QuickOpenUp => {
            state.quick_open.move_up();
            vec![]
        }
        Action::QuickOpenDown => {
            state.quick_open.move_down();
            vec![]
        }
        Action::QuickOpenInsertChar(ch) => {
            state.quick_open.insert_char(*ch);
            vec![]
        }
        Action::QuickOpenDeleteChar => {
            state.quick_open.delete_char();
            vec![]
        }
        Action::QuickOpenConfirm => {
            // Obtener path seleccionado, abrir en editor, cerrar quick open.
            // CLONE: necesario — el path se extrae del quick_open state (inmutable
            // durante la lectura) y luego se usa para abrir archivo (que requiere
            // &mut state vía EditorState::open_file).
            let selected = state.quick_open.selected_path().map(|p| p.to_path_buf());
            state.quick_open.close();

            if let Some(relative_path) = selected {
                // Resolver path absoluto desde el workspace root
                let absolute_path = if let Some(ref explorer) = state.explorer {
                    explorer.root.join(&relative_path)
                } else if let Ok(cwd) = std::env::current_dir() {
                    cwd.join(&relative_path)
                } else {
                    relative_path
                };

                match EditorState::open_file(&absolute_path) {
                    Ok(editor) => {
                        state.editor = editor;
                        state.focused_panel = PanelId::Editor;
                        state.update_status_cache();
                        // Notificar LSP del nuevo archivo abierto
                        if state.lsp.has_server() {
                            let text = buffer_full_text(&state.editor);
                            if let Err(e) = state.lsp.notify_open(&absolute_path, &text) {
                                tracing::warn!(error = %e, "error en LSP did_open");
                            }
                        }
                        tracing::info!(path = %absolute_path.display(), "archivo abierto desde quick open");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "error al abrir archivo desde quick open");
                    }
                }
            }
            vec![]
        }

        // ── Acciones de búsqueda global ──
        Action::OpenGlobalSearch => {
            // Abrir panel de búsqueda: hacer sidebar visible, foco en Search
            state.sidebar_visible = true;
            state.search.open();
            state.focused_panel = PanelId::Search;
            tracing::debug!("búsqueda global abierta");
            vec![]
        }
        Action::SearchClose => {
            state.search.close();
            // Volver foco al editor
            state.focused_panel = PanelId::Editor;
            tracing::debug!("búsqueda global cerrada");
            vec![]
        }
        Action::SearchInsertChar(ch) => {
            state.search.insert_char(*ch);
            vec![]
        }
        Action::SearchDeleteChar => {
            state.search.delete_char();
            vec![]
        }
        Action::SearchNextField => {
            state.search.next_field();
            vec![]
        }
        Action::SearchPrevField => {
            state.search.prev_field();
            vec![]
        }
        Action::SearchExecute => {
            // Determinar workspace root para la búsqueda
            let root = state.explorer.as_ref()
                .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut self
                .or_else(|| std::env::current_dir().ok());

            if let Some(root) = root {
                let max = state.config.search_max_results;
                state.search.execute_search(&root, max);
                tracing::info!(
                    query = %state.search.options.query,
                    matches = state.search.results.as_ref().map(|r| r.total_matches).unwrap_or(0),
                    "búsqueda ejecutada"
                );
            } else {
                tracing::warn!("no hay workspace root para búsqueda");
            }
            vec![]
        }
        Action::SearchNextMatch => {
            state.search.next_match();
            // Navegar al match en el editor
            navigate_to_search_match(state);
            vec![]
        }
        Action::SearchPrevMatch => {
            state.search.prev_match();
            navigate_to_search_match(state);
            vec![]
        }
        Action::SearchToggleCase => {
            state.search.toggle_case_sensitive();
            tracing::debug!(case = state.search.options.case_sensitive, "toggle case");
            vec![]
        }
        Action::SearchToggleWholeWord => {
            state.search.toggle_whole_word();
            tracing::debug!(whole_word = state.search.options.whole_word, "toggle whole word");
            vec![]
        }
        Action::SearchToggleRegex => {
            state.search.toggle_regex();
            tracing::debug!(regex = state.search.options.use_regex, "toggle regex");
            vec![]
        }
        Action::SearchToggleReplace => {
            state.search.toggle_replace();
            tracing::debug!(replace = state.search.replace_visible, "toggle replace");
            vec![]
        }
        Action::SearchReplaceCurrent => {
            let root = state.explorer.as_ref()
                .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut self
                .or_else(|| std::env::current_dir().ok());

            if let Some(root) = root
                && let Err(e) = state.search.replace_current(&root)
            {
                tracing::error!(error = %e, "error en replace current");
            }
            vec![]
        }
        Action::SearchReplaceAllInFile => {
            let root = state.explorer.as_ref()
                .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut self
                .or_else(|| std::env::current_dir().ok());

            if let Some(root) = root
                && let Err(e) = state.search.replace_all_in_file(&root)
            {
                tracing::error!(error = %e, "error en replace all in file");
            }
            vec![]
        }

        // ── Acciones de terminal ──
        Action::ToggleTerminal => {
            if !state.terminal.has_session() {
                // Spawn shell si no hay sesión
                let cwd = state
                    .explorer
                    .as_ref()
                    .map(|e| e.root.clone()) // CLONE: necesario — root se usa después para spawn
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));

                // Calcular tamaño del bottom panel (usar layout si disponible)
                let size = state
                    .last_layout
                    .map(|l| (l.bottom_panel.width.saturating_sub(2), l.bottom_panel.height.saturating_sub(2)))
                    .unwrap_or((80, 10));

                if let Err(e) = state.terminal.spawn_shell(&cwd, size) {
                    tracing::error!(error = %e, "error al crear sesión de terminal");
                }
            }
            // Toggle visibilidad del bottom panel
            state.bottom_panel_visible = !state.bottom_panel_visible;
            if state.bottom_panel_visible {
                state.focused_panel = PanelId::Terminal;
            }
            tracing::debug!(visible = state.bottom_panel_visible, "toggle terminal");
            vec![]
        }
        Action::TerminalInput(ch) => {
            if let Some(ref mut session) = state.terminal.session
                && let Err(e) = session.send_key(*ch)
            {
                tracing::error!(error = %e, "error al enviar key al terminal");
            }
            vec![]
        }
        Action::TerminalEnter => {
            if let Some(ref mut session) = state.terminal.session
                && let Err(e) = session.send_enter()
            {
                tracing::error!(error = %e, "error al enviar Enter al terminal");
            }
            vec![]
        }
        Action::TerminalCtrlC => {
            if let Some(ref mut session) = state.terminal.session
                && let Err(e) = session.send_ctrl_c()
            {
                tracing::error!(error = %e, "error al enviar Ctrl+C al terminal");
            }
            vec![]
        }
        Action::TerminalScrollUp => {
            if let Some(ref mut session) = state.terminal.session {
                session.scroll_up(3);
            }
            vec![]
        }
        Action::TerminalScrollDown => {
            if let Some(ref mut session) = state.terminal.session {
                session.scroll_down(3);
            }
            vec![]
        }
        Action::TerminalSpawn => {
            if !state.terminal.has_session() {
                let cwd = state
                    .explorer
                    .as_ref()
                    .map(|e| e.root.clone()) // CLONE: necesario — root se usa después para spawn
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));

                let size = state
                    .last_layout
                    .map(|l| (l.bottom_panel.width.saturating_sub(2), l.bottom_panel.height.saturating_sub(2)))
                    .unwrap_or((80, 10));

                if let Err(e) = state.terminal.spawn_shell(&cwd, size) {
                    tracing::error!(error = %e, "error al crear sesión de terminal");
                }
            }
            vec![]
        }

        // ── Acciones de Git ──
        Action::OpenGitPanel => {
            // Abrir panel git en sidebar, foco en Git
            state.sidebar_visible = true;
            state.git.visible = true;
            state.git.refresh(
                &get_workspace_root(state),
            );
            state.focused_panel = PanelId::Git;
            tracing::debug!("git panel abierto");
            vec![]
        }
        Action::GitClose => {
            state.git.visible = false;
            state.git.show_diff = false;
            state.git.commit_mode = false;
            state.focused_panel = PanelId::Editor;
            tracing::debug!("git panel cerrado");
            vec![]
        }
        Action::GitRefresh => {
            state.git.refresh(&get_workspace_root(state));
            tracing::debug!("git status refrescado");
            vec![]
        }
        Action::GitUp => {
            state.git.move_up();
            vec![]
        }
        Action::GitDown => {
            state.git.move_down();
            vec![]
        }
        Action::GitStageToggle => {
            let root = get_workspace_root(state);
            if let Err(e) = state.git.stage_toggle(&root) {
                tracing::error!(error = %e, "error en stage/unstage");
            }
            vec![]
        }
        Action::GitToggleDiff => {
            let root = get_workspace_root(state);
            state.git.toggle_diff(&root);
            vec![]
        }
        Action::GitDiffScrollUp => {
            state.git.scroll_diff_up();
            vec![]
        }
        Action::GitDiffScrollDown => {
            state.git.scroll_diff_down();
            vec![]
        }
        Action::GitStartCommit => {
            state.git.commit_mode = true;
            state.git.commit_input.clear();
            tracing::debug!("modo commit activado");
            vec![]
        }
        Action::GitCommitConfirm => {
            let root = get_workspace_root(state);
            match state.git.commit(&root) {
                Ok(()) => {
                    tracing::info!("commit exitoso");
                }
                Err(e) => {
                    tracing::error!(error = %e, "error al hacer commit");
                }
            }
            vec![]
        }
        Action::GitCommitCancel => {
            state.git.commit_mode = false;
            state.git.commit_input.clear();
            tracing::debug!("modo commit cancelado");
            vec![]
        }
        Action::GitCommitInput(ch) => {
            state.git.commit_input.push(*ch);
            vec![]
        }
        Action::GitCommitDeleteChar => {
            state.git.commit_input.pop();
            vec![]
        }

        // ── Acciones LSP ──
        Action::LspStart => {
            if let Some(path) = state.editor.buffer.file_path() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if let Some((cmd, args)) = crate::lsp::detect_language_server(ext) {
                    let root = get_workspace_root(state);
                    match state.lsp.start_server(cmd, args, &root) {
                        Ok(()) => {
                            tracing::info!(cmd, "LSP server arrancado");
                            // Notificar archivo actualmente abierto
                            let text = buffer_full_text(&state.editor);
                            let file_path = state.editor.buffer.file_path()
                                .map(|p| p.to_path_buf());
                            if let Some(ref fp) = file_path
                                && let Err(e) = state.lsp.notify_open(fp, &text)
                            {
                                tracing::warn!(error = %e, "error en LSP did_open");
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "error arrancando LSP server");
                        }
                    }
                } else {
                    tracing::info!(ext, "no hay LSP server conocido para esta extensión");
                }
            }
            vec![]
        }
        Action::LspStop => {
            if let Err(e) = state.lsp.stop() {
                tracing::error!(error = %e, "error deteniendo LSP server");
            }
            vec![]
        }
        Action::LspHover => {
            if let Some(path) = state.editor.buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.editor.cursors.primary().position;
                if let Err(e) = state.lsp.request_hover(&path, pos.line as u32, pos.col as u32) {
                    tracing::warn!(error = %e, "error en LSP hover request");
                }
            }
            vec![]
        }
        Action::LspGotoDefinition => {
            if let Some(path) = state.editor.buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.editor.cursors.primary().position;
                if let Err(e) = state.lsp.request_definition(&path, pos.line as u32, pos.col as u32) {
                    tracing::warn!(error = %e, "error en LSP definition request");
                }
            }
            vec![]
        }
        Action::LspCompletion => {
            if let Some(path) = state.editor.buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.editor.cursors.primary().position;
                if let Err(e) = state.lsp.request_completion(&path, pos.line as u32, pos.col as u32) {
                    tracing::warn!(error = %e, "error en LSP completion request");
                }
            }
            vec![]
        }
        Action::LspCompletionUp => {
            if state.lsp.completion_visible && !state.lsp.completions.is_empty() {
                state.lsp.completion_selected = state
                    .lsp
                    .completion_selected
                    .saturating_sub(1);
            }
            vec![]
        }
        Action::LspCompletionDown => {
            if state.lsp.completion_visible && !state.lsp.completions.is_empty() {
                let max = state.lsp.completions.len().saturating_sub(1);
                state.lsp.completion_selected = (state.lsp.completion_selected + 1).min(max);
            }
            vec![]
        }
        Action::LspCompletionConfirm => {
            if state.lsp.completion_visible {
                let idx = state.lsp.completion_selected;
                if let Some(item) = state.lsp.completions.get(idx) {
                    // CLONE: necesario — insert_text se extrae del Vec antes de mutar editor
                    let text_to_insert = item
                        .insert_text
                        .as_deref()
                        .unwrap_or(&item.label)
                        .to_string();
                    // Insertar cada carácter del texto de completion
                    for ch in text_to_insert.chars() {
                        state.editor.insert_char(ch);
                    }
                    state.update_status_cache();
                }
                state.lsp.completion_visible = false;
                state.lsp.completions.clear();
            }
            vec![]
        }
        Action::LspCompletionCancel => {
            state.lsp.completion_visible = false;
            state.lsp.completions.clear();
            vec![]
        }

        // Acciones no implementadas aún — no producen efectos
        Action::Noop
        | Action::FocusPanel(_)
        | Action::OpenFile(_)
        | Action::CloseBuffer => vec![],
    }
}

/// Helper: obtiene el workspace root desde el explorer o cwd.
fn get_workspace_root(state: &AppState) -> PathBuf {
    state
        .explorer
        .as_ref()
        .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut state
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Helper: envía notificación LSP did_change si hay server activo.
///
/// Usa debounce interno del LspState — no envía en cada keystroke.
fn notify_lsp_change(state: &mut AppState) {
    if !state.lsp.has_server() {
        return;
    }
    if let Some(path) = state.editor.buffer.file_path().map(|p| p.to_path_buf()) {
        let text = buffer_full_text(&state.editor);
        if let Err(e) = state.lsp.notify_change(&path, &text) {
            tracing::warn!(error = %e, "error en LSP did_change");
        }
    }
}

/// Helper: obtiene el texto completo del buffer del editor como un String.
///
/// Reconstruye el texto uniendo líneas con `\n`. Se usa para LSP did_open/did_change.
fn buffer_full_text(editor: &EditorState) -> String {
    let line_count = editor.buffer.line_count();
    // Pre-alocar con estimado razonable (80 chars por línea promedio)
    let mut text = String::with_capacity(line_count * 80);
    for i in 0..line_count {
        if i > 0 {
            text.push('\n');
        }
        if let Some(line) = editor.buffer.line(i) {
            text.push_str(line);
        }
    }
    text
}

// ─── Search navigation helper ──────────────────────────────────────────────────

/// Abre el archivo del match seleccionado y mueve el cursor a la posición.
///
/// Si el match apunta a un archivo diferente al editor actual, lo abre.
/// Luego posiciona el cursor en la línea y columna del match.
fn navigate_to_search_match(state: &mut AppState) {
    let Some(m) = state.search.selected_match_data() else {
        return;
    };

    // Resolver path absoluto
    let abs_path = if let Some(ref explorer) = state.explorer {
        explorer.root.join(&m.path)
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(&m.path)
    } else {
        return;
    };

    let target_line = m.line_number.saturating_sub(1); // 1-indexed → 0-indexed
    let target_col = m.match_start;

    // Verificar si necesitamos abrir otro archivo
    let needs_open = state.editor.buffer.file_path()
        .is_none_or(|current| current != abs_path);

    if needs_open {
        match EditorState::open_file(&abs_path) {
            Ok(editor) => {
                state.editor = editor;
                tracing::info!(path = %abs_path.display(), "archivo abierto desde search");
            }
            Err(e) => {
                tracing::error!(error = %e, "error al abrir archivo desde search");
                return;
            }
        }
    }

    // Posicionar cursor
    let max_line = state.editor.buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = state.editor.buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    let primary = state.editor.cursors.primary_mut();
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    primary.clear_selection();
    let pos = state.editor.cursors.primary().position;
    state.editor.viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();
}

// ─── Mouse helpers ─────────────────────────────────────────────────────────────

/// Dirección de scroll del mouse.
#[derive(Debug, Clone, Copy)]
enum ScrollDirection {
    Up,
    Down,
}

/// Calcula el ancho real del gutter (números de línea + separador `│ `).
///
/// Debe coincidir EXACTAMENTE con la lógica de render en `panels.rs`:
/// `digit_count(line_count).max(4)` + 2 (separador).
fn editor_gutter_width(line_count: usize) -> u16 {
    let digits = crate::ui::panels::digit_count(line_count).max(4);
    let separator = 2; // "│ "
    (digits + separator) as u16
}

/// Cantidad de líneas que el scroll del mouse desplaza por evento.
const MOUSE_SCROLL_LINES: usize = 3;

/// Determina en qué panel cayó una posición (col, row) absoluta.
///
/// Usa el layout almacenado en `AppState.last_layout`. Si no hay layout
/// (primer frame), retorna `None`. La función es pura — no muta estado.
fn hit_test_panel(layout: &IdeLayout, col: u16, row: u16) -> Option<PanelId> {
    let point_in_rect = |r: Rect, c: u16, rw: u16| -> bool {
        c >= r.x && c < r.x + r.width && rw >= r.y && rw < r.y + r.height
    };

    // Verificar paneles en orden de prioridad visual (overlays primero si los hubiera)
    // La sidebar puede mostrar Explorer o Search según el estado activo
    if layout.sidebar_visible && point_in_rect(layout.sidebar, col, row) {
        // El panel activo de la sidebar se resuelve en reduce_mouse_click
        // Retornamos Explorer como default — el reducer ajusta a Search si corresponde
        return Some(PanelId::Explorer);
    }
    if point_in_rect(layout.editor_area, col, row) {
        return Some(PanelId::Editor);
    }
    if layout.bottom_panel_visible && point_in_rect(layout.bottom_panel, col, row) {
        return Some(PanelId::Terminal);
    }
    // Title bar y status bar no son paneles enfocables
    None
}

/// Procesa un click de mouse — resuelve panel, cambia foco, ejecuta acción contextual.
fn reduce_mouse_click(state: &mut AppState, col: u16, row: u16) {
    let Some(layout) = state.last_layout else {
        return; // Sin layout aún — primer frame
    };

    let Some(panel) = hit_test_panel(&layout, col, row) else {
        return; // Click en zona no interactiva (title bar, status bar)
    };

    // Si la sidebar muestra search o git, redirigir foco al panel activo
    let panel = if panel == PanelId::Explorer && state.search.visible {
        PanelId::Search
    } else if panel == PanelId::Explorer && state.git.visible {
        PanelId::Git
    } else {
        panel
    };

    // Cambiar foco al panel clickeado
    state.focused_panel = panel;
    tracing::debug!(?panel, col, row, "mouse click → foco");

    match panel {
        PanelId::Search => {
            // Click en search panel — no hay acción de click específica por ahora
        }
        PanelId::Explorer => {
            reduce_mouse_click_explorer(state, &layout, row);
        }
        PanelId::Editor => {
            reduce_mouse_click_editor(state, &layout, col, row);
        }
        // Terminal y otros: solo cambio de foco por ahora
        _ => {}
    }
}

/// Procesa click en el explorer — seleccionar entry, abrir/toggle.
fn reduce_mouse_click_explorer(state: &mut AppState, layout: &IdeLayout, row: u16) {
    let Some(ref mut explorer) = state.explorer else {
        return;
    };

    // Calcular el inner area de la sidebar (descontar bordes del Block)
    // Block con Borders::ALL tiene 1px de borde arriba (+ título) y 1px abajo
    let inner_y = layout.sidebar.y + 1; // Borde superior + título
    let inner_height = layout.sidebar.height.saturating_sub(2); // Bordes superior e inferior

    if row < inner_y || row >= inner_y + inner_height {
        return; // Click en el borde, no en contenido
    }

    // Índice visual dentro del inner area
    let visual_row = (row - inner_y) as usize;
    // Índice real en la lista aplanada = scroll_offset + visual_row
    let flat_index = explorer.scroll_offset + visual_row;

    let flat = explorer.flatten();
    let flat_len = flat.len();
    if flat_index >= flat_len {
        return; // Click debajo de los entries
    }

    let is_dir = flat[flat_index].is_dir;
    // CLONE: necesario — necesitamos el path para abrir archivo después de drop(flat)
    let entry_path = flat[flat_index].path.clone();
    drop(flat);

    // Seleccionar el entry clickeado
    explorer.selected_index = flat_index;

    if is_dir {
        // Toggle expand/collapse del directorio
        if let Err(e) = explorer.toggle_selected() {
            tracing::error!(error = %e, "error en toggle de explorer por mouse");
        }
    } else {
        // Abrir archivo en el editor
        match EditorState::open_file(&entry_path) {
            Ok(editor) => {
                state.editor = editor;
                state.focused_panel = PanelId::Editor;
                state.update_status_cache();
                tracing::info!(path = %entry_path.display(), "archivo abierto por mouse click");
            }
            Err(e) => {
                tracing::error!(error = %e, "error al abrir archivo por mouse click");
            }
        }
    }
}

/// Procesa click en el editor — mover cursor a la posición clickeada.
fn reduce_mouse_click_editor(state: &mut AppState, layout: &IdeLayout, col: u16, row: u16) {
    // Calcular inner area del editor (descontar bordes del Block)
    let inner_y = layout.editor_area.y + 1;
    let inner_x = layout.editor_area.x + 1;
    let inner_height = layout.editor_area.height.saturating_sub(2);

    if row < inner_y || row >= inner_y + inner_height {
        return; // Click en borde
    }

    // Línea en el buffer = viewport offset + fila visual
    let visual_row = (row - inner_y) as usize;
    let target_line = state.editor.viewport.scroll_offset + visual_row;

    // Columna en el buffer = col relativo al inner area - gutter dinámico
    let gutter = editor_gutter_width(state.editor.buffer.line_count());
    let text_x = inner_x + gutter;
    let target_col = if col >= text_x {
        (col - text_x) as usize
    } else {
        0 // Click en el gutter — columna 0
    };

    // Clampear a límites del buffer
    let max_line = state.editor.buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = state.editor.buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    // Limpiar cursores secundarios al hacer click
    state.editor.cursors.clear_secondary();
    let primary = state.editor.cursors.primary_mut();
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    // Iniciar selección con anchor = head = click_pos.
    // Si el usuario no arrastra, anchor == head → selección vacía (equivale a sin selección).
    // Si arrastra, el drag handler actualiza head para extender la selección.
    let click_pos = crate::editor::cursor::Position {
        line: clamped_line,
        col: clamped_col,
    };
    primary.selection = Some(crate::editor::selection::Selection::new(click_pos, click_pos));
    let pos = state.editor.cursors.primary().position;
    state.editor.viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();

    tracing::debug!(line = clamped_line, col = clamped_col, "mouse click → cursor editor");
}

/// Procesa drag del mouse — selección de texto arrastrando.
///
/// Solo actúa si el drag cae en el editor area. Extiende la selección
/// desde el anchor (seteado en el click) hasta la posición actual del drag.
fn reduce_mouse_drag(state: &mut AppState, col: u16, row: u16) {
    let Some(layout) = state.last_layout else {
        return; // Sin layout — primer frame
    };

    let Some(panel) = hit_test_panel(&layout, col, row) else {
        return;
    };

    // Drag-to-select solo en el editor
    if panel == PanelId::Editor {
        reduce_mouse_drag_editor(state, &layout, col, row);
    }
}

/// Procesa drag en el editor — extiende selección desde anchor hasta posición del drag.
fn reduce_mouse_drag_editor(state: &mut AppState, layout: &IdeLayout, col: u16, row: u16) {
    // Calcular inner area del editor (descontar bordes del Block)
    let inner_y = layout.editor_area.y + 1;
    let inner_x = layout.editor_area.x + 1;
    let inner_height = layout.editor_area.height.saturating_sub(2);

    // Clampear row al rango visible del editor para permitir scroll
    // cuando el drag sale por arriba o abajo del viewport
    let clamped_row = row.clamp(inner_y, inner_y + inner_height.saturating_sub(1));

    // Línea en el buffer = viewport offset + fila visual
    let visual_row = (clamped_row - inner_y) as usize;
    let target_line = state.editor.viewport.scroll_offset + visual_row;

    // Columna en el buffer = col relativo al inner area - gutter dinámico
    let gutter = editor_gutter_width(state.editor.buffer.line_count());
    let text_x = inner_x + gutter;
    let target_col = if col >= text_x {
        (col - text_x) as usize
    } else {
        0 // Drag en el gutter — columna 0
    };

    // Clampear a límites del buffer
    let max_line = state.editor.buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = state.editor.buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    let primary = state.editor.cursors.primary_mut();

    // Verificar que hay una selección activa (seteada por el click previo)
    if primary.selection.is_none() {
        return;
    }

    // Actualizar posición del cursor y head de la selección
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    primary.extend_selection();

    // Scroll automático si el drag lleva el cursor fuera del viewport
    let pos = state.editor.cursors.primary().position;
    state.editor.viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();

    tracing::trace!(line = clamped_line, col = clamped_col, "mouse drag → selección editor");
}

/// Procesa scroll del mouse — scrollea el panel bajo el cursor.
fn reduce_mouse_scroll(state: &mut AppState, col: u16, row: u16, direction: ScrollDirection) {
    let Some(layout) = state.last_layout else {
        return;
    };

    let Some(panel) = hit_test_panel(&layout, col, row) else {
        return;
    };

    match panel {
        PanelId::Explorer => {
            // Si search está visible, scrollear resultados de búsqueda
            if state.search.visible {
                let match_count = state.search.results.as_ref()
                    .map(|r| r.matches.len())
                    .unwrap_or(0);
                match direction {
                    ScrollDirection::Up => {
                        state.search.scroll_offset = state.search.scroll_offset.saturating_sub(MOUSE_SCROLL_LINES);
                    }
                    ScrollDirection::Down => {
                        let max_scroll = match_count.saturating_sub(1);
                        state.search.scroll_offset = (state.search.scroll_offset + MOUSE_SCROLL_LINES).min(max_scroll);
                    }
                }
            } else if let Some(ref mut explorer) = state.explorer {
                let flat_count = explorer.flatten().len();
                match direction {
                    ScrollDirection::Up => {
                        explorer.scroll_offset = explorer.scroll_offset.saturating_sub(MOUSE_SCROLL_LINES);
                    }
                    ScrollDirection::Down => {
                        let max_scroll = flat_count.saturating_sub(1);
                        explorer.scroll_offset = (explorer.scroll_offset + MOUSE_SCROLL_LINES).min(max_scroll);
                    }
                }
            }
        }
        PanelId::Editor => {
            let line_count = state.editor.buffer.line_count();
            match direction {
                ScrollDirection::Up => {
                    state.editor.viewport.scroll_offset =
                        state.editor.viewport.scroll_offset.saturating_sub(MOUSE_SCROLL_LINES);
                }
                ScrollDirection::Down => {
                    let max_scroll = line_count.saturating_sub(1);
                    state.editor.viewport.scroll_offset =
                        (state.editor.viewport.scroll_offset + MOUSE_SCROLL_LINES).min(max_scroll);
                }
            }
        }
        PanelId::Terminal => {
            if let Some(ref mut session) = state.terminal.session {
                match direction {
                    ScrollDirection::Up => session.scroll_up(MOUSE_SCROLL_LINES),
                    ScrollDirection::Down => session.scroll_down(MOUSE_SCROLL_LINES),
                }
            }
        }
        // Otros paneles: scroll no implementado aún
        _ => {}
    }
}

// ─── Process Effects ───────────────────────────────────────────────────────────

/// Procesa los efectos producidos por el reducer.
///
/// Por ahora solo `Effect::Quit` tiene comportamiento (cancela el token).
/// A medida que se implementen workers, se despacharán acá.
fn process_effects(effects: &[Effect], shutdown: &CancellationToken) {
    for effect in effects {
        match effect {
            Effect::Quit => {
                shutdown.cancel();
            }
            // Efectos futuros se despacharán a workers acá
            Effect::None
            | Effect::LoadFile(_)
            | Effect::SaveFile { .. }
            | Effect::RunSearch
            | Effect::RefreshGitStatus
            | Effect::SpawnTerminal => {
                tracing::debug!(?effect, "efecto pendiente de implementación");
            }
        }
    }
}

// ─── Run ───────────────────────────────────────────────────────────────────────

/// Ejecuta la aplicación completa.
///
/// Acepta un path opcional para abrir un archivo al inicio.
/// Setup de terminal -> event loop -> cleanup.
pub async fn run(file: Option<PathBuf>) -> Result<()> {
    let shutdown = CancellationToken::new();
    let theme = Theme::default();

    // Setup terminal: raw mode + alternate screen + captura de mouse + cursor bar
    terminal::enable_raw_mode()
        .context("no se pudo activar raw mode")?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        SetCursorStyle::SteadyBar, // cursor estilo VS Code (línea vertical)
    )
    .context("no se pudo entrar a alternate screen con mouse capture")?;

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)
        .context("no se pudo crear terminal ratatui")?;

    // Event loop con cleanup garantizado
    let result = event_loop(&mut terminal, &shutdown, &theme, file).await;

    // Cleanup: SIEMPRE restaurar terminal, incluso si hubo error
    cleanup_terminal()?;

    result
}

// ─── Event Loop ────────────────────────────────────────────────────────────────

/// Event loop principal.
///
/// Ciclo: poll evento -> keymap -> reduce -> process effects -> render.
/// Instrumentado con `FrameTimer` y `Metrics` para observabilidad.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    shutdown: &CancellationToken,
    theme: &Theme,
    file: Option<PathBuf>,
) -> Result<()> {
    let config = AppConfig::new();
    let mut state = if let Some(ref path) = file {
        AppState::with_file(config, path)
            .with_context(|| format!("no se pudo abrir: {}", path.display()))?
    } else {
        AppState::new(config)
    };

    // Inicializar status cache
    state.update_status_cache();

    let tick_duration = Duration::from_millis(state.config.tick_rate_ms);

    loop {
        // 1. Poll de eventos con timeout (esto ESPERA — no cuenta como frame time)
        let event = poll_event(tick_duration)?;

        // 2. Iniciar medición del frame DESPUÉS del poll.
        let frame_timer = FrameTimer::start();

        // 3. Mapear evento a acción (sensible al panel enfocado y overlays)
        let action = match &event {
            Some(Event::Input(crossterm_event)) => {
                keymap(
                    crossterm_event,
                    state.focused_panel,
                    state.palette.visible,
                    state.quick_open.visible,
                    state.search.visible,
                    &state.git,
                    state.lsp.completion_visible,
                )
            }
            Some(Event::Tick) => Action::Noop,
            _ => Action::Noop,
        };

        // 4. Registrar evento procesado
        if event.is_some() {
            state.metrics.record_event();
        }

        // 5. Reducer: actualizar estado y obtener efectos
        let effects = reduce(&mut state, &action);

        // 6. Procesar efectos
        process_effects(&effects, shutdown);

        // 6.5. Ajustar scroll del explorer para mantener selección visible.
        //      Se hace antes del render para que el viewport sea correcto.
        //      El visible_height se calcula con un estimado razonable —
        //      el layout real lo determina el render, pero esto es suficiente
        //      para mantener el scroll correcto entre frames.
        {
            // Estimado del alto visible de la sidebar (descontar bordes)
            let term_height = terminal.size()
                .map(|s| s.height)
                .unwrap_or(24);
            // Restar: title bar(1) + status bar(1) + bordes del panel(2)
            let sidebar_height = term_height.saturating_sub(4) as usize;

            if let Some(ref mut explorer) = state.explorer {
                explorer.ensure_visible(sidebar_height);
            }

            // Ajustar scroll del git panel
            if state.git.visible {
                // Descontar branch line(1) + commit input(2 si aplica)
                let commit_lines = if state.git.commit_mode { 2 } else { 0 };
                let git_list_height = sidebar_height.saturating_sub(1 + commit_lines);
                state.git.ensure_visible(git_list_height);
            }
        }

        // 7. Computar layout y almacenarlo para resolución de mouse.
        //    El layout se computa ANTES del render para que el reducer del
        //    próximo frame tenga las áreas actualizadas. `IdeLayout` es Copy.
        let term_size = terminal.size().context("no se pudo obtener tamaño de terminal")?;
        let layout = IdeLayout::compute(
            Rect::new(0, 0, term_size.width, term_size.height),
            state.sidebar_visible,
            state.bottom_panel_visible,
        );
        state.last_layout = Some(layout);

        // 7.5. Actualizar viewport del editor con el tamaño real del editor area.
        //      Se hace ANTES del render para que ensure_cursor_visible funcione
        //      con dimensiones correctas. Descontar bordes (2) + gutter dinámico.
        {
            let editor_inner_h = layout.editor_area.height.saturating_sub(2) as usize;
            let editor_inner_w = layout.editor_area.width.saturating_sub(2) as usize;
            // Gutter width dinámico: dígitos del total de líneas (mín 4) + 2 (separador)
            let total_lines = state.editor.buffer.line_count();
            let gutter_digits = {
                let mut count = 0usize;
                let mut val = total_lines;
                if val == 0 { count = 1; } else {
                    while val > 0 { count += 1; val /= 10; }
                }
                count
            };
            let gutter_total = gutter_digits.max(4) + 2; // gutter + separator
            let text_width = editor_inner_w.saturating_sub(gutter_total);
            state.editor.viewport.update_size(text_width, editor_inner_h);
        }

        // 8. Render frame actual
        terminal.draw(|frame| {
            ui::render(frame, &state, theme);
        }).context("error en render")?;

        // 8.5. Poll de output del terminal (non-blocking).
        //      Se hace después del render para que el output nuevo
        //      se muestre en el próximo frame.
        if let Err(e) = state.terminal.poll_output() {
            tracing::warn!(error = %e, "error al poll de output del terminal");
        }

        // 8.6. Poll de mensajes LSP (non-blocking).
        //      Procesa diagnósticos, responses a hover/completion/definition.
        if state.lsp.has_server() {
            state.lsp.poll();

            // Procesar go-to-definition si hay resultado pendiente
            if let Some(def_result) = state.lsp.definition_result.take()
                && let Some(path) = crate::lsp::uri_to_path(&def_result.uri)
            {
                match EditorState::open_file(&path) {
                    Ok(editor) => {
                        state.editor = editor;
                        // Posicionar cursor en la definición
                        let primary = state.editor.cursors.primary_mut();
                        primary.position.line = def_result.line as usize;
                        primary.position.col = def_result.col as usize;
                        primary.sync_desired_col();
                        primary.clear_selection();
                        let pos = state.editor.cursors.primary().position;
                        state.editor.viewport.ensure_cursor_visible(&pos);
                        state.focused_panel = PanelId::Editor;
                        state.update_status_cache();
                        tracing::info!(
                            path = %path.display(),
                            line = def_result.line,
                            col = def_result.col,
                            "go-to-definition: archivo abierto"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "error abriendo archivo de definition");
                    }
                }
            }

            // Actualizar LSP status message para el cursor actual
            if let Some(path) = state.editor.buffer.file_path().map(|p| p.to_path_buf()) {
                let cursor_line = state.editor.cursors.primary().position.line as u32;
                state.lsp.update_status_for_cursor(&path, cursor_line);
            }

            // Flush de did_change pendiente (debounce)
            let editor_ref = &state.editor;
            state.lsp.flush_pending_change(|_uri| {
                Some(buffer_full_text(editor_ref))
            });
        }

        // 9. Registrar métricas del frame (solo reduce + render, no poll wait)
        let frame_time = frame_timer.elapsed_us();
        state.metrics.record_frame(frame_time);
        state.metrics.record_input_latency(frame_time);

        // 10. Log de warning si el frame excede el budget target
        if crate::core::budgets::DEFAULT_BUDGETS.frame_exceeds_target(frame_time) {
            tracing::warn!(
                frame_time_us = frame_time,
                avg_us = state.metrics.avg_frame_time_us,
                "frame excede budget target de 16ms"
            );
        }

        // 11. Salir si el estado lo indica o shutdown externo
        if !state.running || shutdown.is_cancelled() {
            tracing::info!(
                frames = state.metrics.frame_count,
                events = state.metrics.event_count,
                dropped = state.metrics.dropped_frames,
                avg_frame_us = state.metrics.avg_frame_time_us,
                "shutdown — métricas finales"
            );
            break;
        }
    }

    Ok(())
}

// ─── Poll Event ────────────────────────────────────────────────────────────────

/// Poll de eventos de crossterm con timeout.
fn poll_event(timeout: Duration) -> Result<Option<Event>> {
    if event::poll(timeout).context("error en poll de eventos")? {
        let crossterm_event = event::read().context("error leyendo evento")?;
        Ok(Some(Event::Input(crossterm_event)))
    } else {
        Ok(Some(Event::Tick))
    }
}

// ─── Cleanup ───────────────────────────────────────────────────────────────────

/// Restaura la terminal a su estado original.
fn cleanup_terminal() -> Result<()> {
    terminal::disable_raw_mode()
        .context("no se pudo desactivar raw mode")?;
    crossterm::execute!(
        std::io::stdout(),
        SetCursorStyle::DefaultUserShape, // restaurar cursor del usuario
        DisableMouseCapture,
        LeaveAlternateScreen,
    )
    .context("no se pudo restaurar terminal")?;
    Ok(())
}
