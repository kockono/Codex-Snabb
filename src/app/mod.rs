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
use crate::observe::{FrameTimer, Metrics};
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
        // Actualizar posición del cursor (1-indexed para display)
        self.status_line.clear();
        // Escribir sin format!() — usamos write! con buffer reutilizado
        use std::fmt::Write;
        let _ = write!(
            self.status_line,
            "Ln {}, Col {}",
            self.editor.cursor.position.line + 1,
            self.editor.cursor.position.col + 1
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
/// - **Quick Open abierto**: captura TODO el input (Esc cierra, Enter confirma,
///   flechas navegan, chars se escriben en búsqueda)
/// - **Palette abierta**: captura TODO el input (Esc cierra, Enter confirma,
///   flechas navegan, chars se escriben en búsqueda)
/// - **Global**: Ctrl+atajos, Esc, Tab (siempre activos cuando overlays cerrados)
/// - **Editor**: flechas mueven cursor, chars insertan texto
/// - **Explorer**: flechas navegan el árbol, Enter abre/expande
fn keymap(
    event: &crossterm::event::Event,
    focused_panel: PanelId,
    palette_visible: bool,
    quick_open_visible: bool,
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
            _ => Action::Noop,
        };
    }

    let CrosstermEvent::Key(key) = event else {
        return Action::Noop;
    };
    if key.kind != KeyEventKind::Press {
        return Action::Noop;
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

    // ── Atajos globales (Ctrl+algo, Esc, Tab) ──
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => return Action::Quit,
        (KeyCode::Tab, KeyModifiers::NONE) => return Action::FocusNext,
        (KeyCode::BackTab, KeyModifiers::SHIFT) => return Action::FocusPrev,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => return Action::ToggleSidebar,
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => return Action::ToggleBottomPanel,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => return Action::SaveFile,
        (KeyCode::Char('z'), KeyModifiers::CONTROL) => return Action::Undo,
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => return Action::Redo,
        // Ctrl+Shift+P abre la command palette.
        // crossterm reporta Ctrl+Shift+P como 'P' mayúscula con CONTROL|SHIFT flags.
        // Necesitamos un guard porque `|` en match es OR, no bitwise OR.
        (KeyCode::Char('P'), mods)
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
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
        PanelId::Editor => match (key.code, key.modifiers) {
            // Movimiento de cursor
            (KeyCode::Up, KeyModifiers::NONE) => Action::MoveCursor(Direction::Up),
            (KeyCode::Down, KeyModifiers::NONE) => Action::MoveCursor(Direction::Down),
            (KeyCode::Left, KeyModifiers::NONE) => Action::MoveCursor(Direction::Left),
            (KeyCode::Right, KeyModifiers::NONE) => Action::MoveCursor(Direction::Right),
            (KeyCode::Home, KeyModifiers::NONE) => Action::MoveToLineStart,
            (KeyCode::End, KeyModifiers::NONE) => Action::MoveToLineEnd,

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
            state.editor.insert_char(*ch);
            state.update_status_cache();
            vec![]
        }
        Action::DeleteChar => {
            state.editor.delete_char();
            state.update_status_cache();
            vec![]
        }
        Action::InsertNewline => {
            state.editor.insert_newline();
            state.update_status_cache();
            vec![]
        }
        Action::MoveCursor(dir) => {
            state.editor.move_cursor(*dir);
            state.update_status_cache();
            vec![]
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
                        tracing::info!(path = %absolute_path.display(), "archivo abierto desde quick open");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "error al abrir archivo desde quick open");
                    }
                }
            }
            vec![]
        }

        // Acciones no implementadas aún — no producen efectos
        Action::Noop
        | Action::FocusPanel(_)
        | Action::OpenFile(_)
        | Action::CloseBuffer
        | Action::OpenGlobalSearch
        | Action::ToggleTerminal
        | Action::OpenGitPanel => vec![],
    }
}

// ─── Mouse helpers ─────────────────────────────────────────────────────────────

/// Dirección de scroll del mouse.
#[derive(Debug, Clone, Copy)]
enum ScrollDirection {
    Up,
    Down,
}

/// Ancho del gutter (números de línea) en el editor.
/// Valor fijo por ahora — en el futuro se computará dinámicamente
/// según la cantidad de dígitos del número de línea más alto.
const EDITOR_GUTTER_WIDTH: u16 = 4;

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
    if layout.sidebar_visible && point_in_rect(layout.sidebar, col, row) {
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

    // Cambiar foco al panel clickeado
    state.focused_panel = panel;
    tracing::debug!(?panel, col, row, "mouse click → foco");

    match panel {
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

    // Columna en el buffer = col relativo al inner area - gutter
    let gutter = EDITOR_GUTTER_WIDTH;
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

    state.editor.cursor.position.line = clamped_line;
    state.editor.cursor.position.col = clamped_col;
    state.editor.cursor.sync_desired_col();
    state.editor.viewport.ensure_cursor_visible(&state.editor.cursor.position);
    state.update_status_cache();

    tracing::debug!(line = clamped_line, col = clamped_col, "mouse click → cursor editor");
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
            if let Some(ref mut explorer) = state.explorer {
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

    // Setup terminal: raw mode + alternate screen + captura de mouse
    terminal::enable_raw_mode()
        .context("no se pudo activar raw mode")?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)
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
        if let Some(ref mut explorer) = state.explorer {
            // Estimado del alto visible de la sidebar (descontar bordes)
            let term_height = terminal.size()
                .map(|s| s.height)
                .unwrap_or(24);
            // Restar: title bar(1) + status bar(1) + bordes del panel(2)
            let explorer_height = term_height.saturating_sub(4) as usize;
            explorer.ensure_visible(explorer_height);
        }

        // 7. Computar layout y almacenarlo para resolución de mouse.
        //    El layout se computa ANTES del render para que el reducer del
        //    próximo frame tenga las áreas actualizadas. `IdeLayout` es Copy.
        let term_size = terminal.size().context("no se pudo obtener tamaño de terminal")?;
        state.last_layout = Some(IdeLayout::compute(
            Rect::new(0, 0, term_size.width, term_size.height),
            state.sidebar_visible,
            state.bottom_panel_visible,
        ));

        // 8. Render frame actual
        terminal.draw(|frame| {
            ui::render(frame, &state, theme);
        }).context("error en render")?;

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
    crossterm::execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)
        .context("no se pudo desactivar mouse capture y salir de alternate screen")?;
    Ok(())
}
