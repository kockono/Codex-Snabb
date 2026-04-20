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
    event::{self, DisableMouseCapture, EnableMouseCapture},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio_util::sync::CancellationToken;

use crate::core::command::CommandRegistry;
use crate::core::settings::{KeybindingsState, SidebarSection};
use crate::core::{Action, AppConfig, Effect, Event, PanelId};
use crate::editor::EditorState;
use crate::editor::highlighting::HighlightEngine;
use crate::editor::tabs::TabState;
use crate::git::branch_picker::BranchPicker;
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
    /// Estado de tabs/buffers del editor (múltiples archivos abiertos).
    pub tabs: TabState,
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
    /// Estado del branch picker (overlay de selección de rama).
    pub branch_picker: BranchPicker,
    /// Estado del subsistema LSP (language server protocol).
    pub lsp: LspState,
    /// Estado del overlay de settings / keybindings editor.
    pub keybindings: KeybindingsState,
    /// Datos pre-computados para la status bar (se actualizan en cada frame).
    /// Evita allocaciones dentro del render — se computan antes.
    pub status_line: String,
    pub status_file: String,
    /// Layout del último frame renderizado, para resolver posiciones de mouse.
    /// Se actualiza cada frame antes del render. `IdeLayout` es Copy (struct de Rects).
    pub last_layout: Option<IdeLayout>,
    /// Motor de syntax highlighting — singleton, ~2MB inmutable.
    /// Se carga UNA VEZ al inicio. Se pasa por referencia a los editores.
    pub highlight_engine: HighlightEngine,
    /// Par de brackets matching pre-computado para el cursor actual.
    /// Se actualiza en cada movimiento de cursor — no en cada frame.
    /// `(bracket_pos, matching_bracket_pos)` o `None` si no hay match.
    pub bracket_match: Option<(crate::editor::cursor::Position, crate::editor::cursor::Position)>,
    /// Si el cursor del editor es visible (para efecto blink).
    /// Se togglea cada N ticks (~500ms). `true` = visible.
    pub cursor_visible: bool,
    /// Contador de ticks para el blink del cursor.
    /// Se resetea a 0 en cada input del usuario.
    pub cursor_blink_counter: u32,
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
            tabs: TabState::new(),
            explorer,
            commands,
            palette: PaletteState::new(),
            quick_open,
            search: SearchState::new(),
            terminal: TerminalState::new(),
            git,
            branch_picker: BranchPicker::new(),
            lsp: LspState::new(),
            keybindings: KeybindingsState::new(),
            status_line: String::from("Ln 1, Col 1"),
            status_file: String::from("[no file]"),
            last_layout: None,
            highlight_engine: HighlightEngine::new(),
            bracket_match: None,
            cursor_visible: true,
            cursor_blink_counter: 0,
        }
    }

    /// Crea un nuevo estado con un archivo abierto.
    ///
    /// El explorer se inicializa con el directorio del archivo si tiene
    /// uno, o con el directorio de trabajo actual como fallback.
    fn with_file(config: AppConfig, path: &std::path::Path) -> Result<Self> {
        let highlight_engine = HighlightEngine::new();
        let mut editor = EditorState::open_file(path)?;
        editor.init_highlighting(&highlight_engine);
        let tabs = TabState::with_editor(editor);
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
            tabs,
            explorer,
            commands,
            palette: PaletteState::new(),
            quick_open,
            search: SearchState::new(),
            terminal: TerminalState::new(),
            git,
            branch_picker: BranchPicker::new(),
            lsp: LspState::new(),
            keybindings: KeybindingsState::new(),
            status_line: String::from("Ln 1, Col 1"),
            status_file,
            last_layout: None,
            highlight_engine,
            bracket_match: None,
            cursor_visible: true,
            cursor_blink_counter: 0,
        })
    }

    /// Actualiza los strings pre-computados de la status bar y bracket match.
    ///
    /// Se llama después de cualquier acción que modifique el cursor o el buffer.
    /// Reutiliza la capacidad existente del String para minimizar allocaciones.
    /// También re-computa el bracket match para la posición actual del cursor.
    fn update_status_cache(&mut self) {
        // Actualizar posición del cursor primario (1-indexed para display)
        self.status_line.clear();
        // Escribir sin format!() — usamos write! con buffer reutilizado
        use std::fmt::Write;
        let editor = self.tabs.active();
        let primary = editor.cursors.primary();
        let _ = write!(
            self.status_line,
            "Ln {}, Col {}",
            primary.position.line + 1,
            primary.position.col + 1
        );

        // Actualizar bracket match — solo re-computar cuando cursor cambia
        let cursor_pos = primary.position;
        self.bracket_match =
            crate::editor::brackets::compute_bracket_match(&editor.buffer, cursor_pos);

        // Actualizar nombre de archivo
        if let Some(path) = editor.buffer.file_path() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if editor.buffer.is_dirty() {
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

mod keymap;
use keymap::keymap;

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
            state.tabs.active_mut().insert_char(*ch);
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::DeleteChar => {
            state.tabs.active_mut().delete_char();
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::InsertNewline => {
            // Cerrar completions al insertar newline
            state.lsp.completion_visible = false;
            state.lsp.completions.clear();
            state.tabs.active_mut().insert_newline();
            state.update_status_cache();
            notify_lsp_change(state);
            vec![]
        }
        Action::MoveCursor(dir) => {
            state.tabs.active_mut().move_cursor(*dir, false);
            state.update_status_cache();
            // Limpiar hover al mover cursor
            state.lsp.hover_content = None;
            vec![]
        }
        Action::MoveCursorSelecting(dir) => {
            state.tabs.active_mut().move_cursor(*dir, true);
            state.update_status_cache();
            vec![]
        }
        Action::SelectNextOccurrence => {
            state.tabs.active_mut().select_next_occurrence();
            state.update_status_cache();
            vec![]
        }
        Action::ClearMultiCursor => {
            if state.tabs.active_mut().has_multicursors() {
                // Con multicursores activos, Esc limpia los secundarios
                state.tabs.active_mut().clear_multicursors();
                vec![]
            } else if state.tabs.active_mut().cursors.primary().has_selection() {
                // Con selección activa, Esc limpia la selección
                state.tabs.active_mut().cursors.primary_mut().clear_selection();
                vec![]
            } else {
                // Sin multicursor ni selección, Esc = Quit
                state.running = false;
                vec![Effect::Quit]
            }
        }
        Action::MoveToLineStart => {
            state.tabs.active_mut().move_to_line_start();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToLineEnd => {
            state.tabs.active_mut().move_to_line_end();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToBufferStart => {
            state.tabs.active_mut().move_to_buffer_start();
            state.update_status_cache();
            vec![]
        }
        Action::MoveToBufferEnd => {
            state.tabs.active_mut().move_to_buffer_end();
            state.update_status_cache();
            vec![]
        }
        Action::Undo => {
            state.tabs.active_mut().undo();
            state.update_status_cache();
            vec![]
        }
        Action::Redo => {
            state.tabs.active_mut().redo();
            state.update_status_cache();
            vec![]
        }
        Action::SaveFile => {
            match state.tabs.active_mut().save() {
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
                            // Abrir archivo en una tab del editor
                            if let Some(path) = explorer.selected_path() {
                                match state.tabs.open_file(&path) {
                                    Ok(()) => {
                                        state.tabs.active_mut().init_highlighting(&state.highlight_engine);
                                        state.focused_panel = PanelId::Editor;
                                        state.update_status_cache();
                                        // Notificar LSP del nuevo archivo abierto
                                        if state.lsp.has_server() {
                                            let text = buffer_full_text(state.tabs.active());
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

                match state.tabs.open_file(&absolute_path) {
                    Ok(()) => {
                        state.tabs.active_mut().init_highlighting(&state.highlight_engine);
                        state.focused_panel = PanelId::Editor;
                        state.update_status_cache();
                        // Notificar LSP del nuevo archivo abierto
                        if state.lsp.has_server() {
                            let text = buffer_full_text(state.tabs.active());
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
            state.search.flat_next();
            vec![]
        }
        Action::SearchPrevMatch => {
            state.search.flat_prev();
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
        Action::SearchToggleFold => {
            // Left/Right: colapsar/expandir file header según estado actual
            // Si el item seleccionado es un FileHeader, toggle fold
            if let Some(&crate::search::FlatSearchItem::FileHeader { group_index }) =
                state.search.selected_item()
            {
                state.search.toggle_fold(group_index);
            }
            vec![]
        }
        Action::SearchSelectAndOpen => {
            // Enter: si hay resultados y estamos en la lista, actuar según item
            if state.search.flat_items.is_empty() {
                // Sin resultados: ejecutar búsqueda
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
                }
            } else {
                // Hay resultados: flat_enter maneja toggle fold / abrir match
                if let Some(match_idx) = state.search.flat_enter() {
                    // Sincronizar selected_match con el índice del match
                    state.search.selected_match = match_idx;
                    navigate_to_search_match(state);
                }
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
            if let Some(path) = state.tabs.active().buffer.file_path() {
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
                            let text = buffer_full_text(state.tabs.active());
                            let file_path = state.tabs.active().buffer.file_path()
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
            if let Some(path) = state.tabs.active().buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.tabs.active().cursors.primary().position;
                if let Err(e) = state.lsp.request_hover(&path, pos.line as u32, pos.col as u32) {
                    tracing::warn!(error = %e, "error en LSP hover request");
                }
            }
            vec![]
        }
        Action::LspGotoDefinition => {
            if let Some(path) = state.tabs.active().buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.tabs.active().cursors.primary().position;
                if let Err(e) = state.lsp.request_definition(&path, pos.line as u32, pos.col as u32) {
                    tracing::warn!(error = %e, "error en LSP definition request");
                }
            }
            vec![]
        }
        Action::LspCompletion => {
            if let Some(path) = state.tabs.active().buffer.file_path().map(|p| p.to_path_buf()) {
                let pos = state.tabs.active().cursors.primary().position;
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
                        state.tabs.active_mut().insert_char(ch);
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

        // ── Acciones del Branch Picker ──
        Action::BranchPickerOpen => {
            // Solo abrir si no hay otros overlays visibles
            if !state.palette.visible && !state.quick_open.visible {
                let root = get_workspace_root(state);
                match state.branch_picker.open(&root) {
                    Ok(()) => {
                        tracing::debug!("branch picker abierto");
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "error al abrir branch picker");
                    }
                }
            }
            vec![]
        }
        Action::BranchPickerClose => {
            state.branch_picker.close();
            tracing::debug!("branch picker cerrado");
            vec![]
        }
        Action::BranchPickerUp => {
            state.branch_picker.move_up();
            vec![]
        }
        Action::BranchPickerDown => {
            state.branch_picker.move_down();
            vec![]
        }
        Action::BranchPickerInsertChar(ch) => {
            state.branch_picker.insert_char(*ch);
            vec![]
        }
        Action::BranchPickerDeleteChar => {
            state.branch_picker.delete_char();
            vec![]
        }
        Action::BranchPickerConfirm => {
            let root = get_workspace_root(state);
            match state.branch_picker.checkout_selected(&root) {
                Ok(()) => {
                    // Refrescar git state para actualizar branch en status bar
                    state.git.refresh(&root);
                    tracing::info!(branch = %state.git.branch, "checkout exitoso");
                }
                Err(e) => {
                    tracing::error!(error = %e, "error en checkout de branch");
                    // Cerrar picker incluso si falla
                    state.branch_picker.close();
                }
            }
            vec![]
        }

        // ── Acciones de Settings ──
        Action::SettingsOpen => {
            // Cerrar otros overlays al abrir settings
            state.palette.close();
            state.quick_open.close();
            state.branch_picker.close();
            state.keybindings.open(&state.commands);
            tracing::debug!("settings overlay abierto");
            vec![]
        }
        Action::SettingsClose => {
            // Aplicar cambios al registry antes de cerrar
            state.keybindings.apply_to_registry(&mut state.commands);
            state.keybindings.close();
            tracing::debug!("settings overlay cerrado");
            vec![]
        }
        Action::SettingsUp => {
            state.keybindings.move_up();
            vec![]
        }
        Action::SettingsDown => {
            state.keybindings.move_down();
            vec![]
        }
        Action::SettingsSearchInsert(ch) => {
            state.keybindings.insert_search_char(*ch);
            vec![]
        }
        Action::SettingsSearchDelete => {
            state.keybindings.delete_search_char();
            vec![]
        }
        Action::SettingsStartEdit => {
            state.keybindings.start_editing();
            tracing::debug!("settings: modo edición de keybind");
            vec![]
        }
        Action::SettingsCancelEdit => {
            state.keybindings.cancel_editing();
            tracing::debug!("settings: edición cancelada");
            vec![]
        }
        Action::SettingsCaptureKey(key_event) => {
            // Formatear el KeyEvent capturado como display string
            let keybind_str = crate::core::settings::format_keybind(key_event);
            if !keybind_str.is_empty() {
                state.keybindings.set_keybind(&keybind_str);
                tracing::info!(keybind = %keybind_str, "settings: keybind capturado");
            } else {
                state.keybindings.cancel_editing();
            }
            vec![]
        }
        Action::SettingsRemoveKeybind => {
            state.keybindings.remove_keybind();
            tracing::debug!("settings: keybind removido");
            vec![]
        }

        // ── Activity Bar ──
        Action::ActivityBarSelect(section) => {
            match section {
                SidebarSection::Explorer => {
                    state.sidebar_visible = true;
                    state.search.close();
                    state.git.visible = false;
                    state.focused_panel = PanelId::Explorer;
                }
                SidebarSection::Git => {
                    state.sidebar_visible = true;
                    state.search.close();
                    state.git.visible = true;
                    state.git.refresh(&get_workspace_root(state));
                    state.focused_panel = PanelId::Git;
                }
                SidebarSection::Search => {
                    state.sidebar_visible = true;
                    state.git.visible = false;
                    state.search.open();
                    state.focused_panel = PanelId::Search;
                }
            }
            tracing::debug!(?section, "activity bar: sección seleccionada");
            vec![]
        }

        // ── Acciones de tabs ──
        Action::NextTab => {
            state.tabs.next_tab();
            state.update_status_cache();
            tracing::debug!(tab = state.tabs.active_index(), "tab siguiente");
            vec![]
        }
        Action::PrevTab => {
            state.tabs.prev_tab();
            state.update_status_cache();
            tracing::debug!(tab = state.tabs.active_index(), "tab anterior");
            vec![]
        }
        Action::CloseTab | Action::CloseBuffer => {
            state.tabs.close_active();
            state.update_status_cache();
            tracing::debug!(tabs = state.tabs.tab_count(), "tab cerrada");
            vec![]
        }
        Action::SwitchTab(index) => {
            state.tabs.switch_to(*index);
            state.update_status_cache();
            tracing::debug!(tab = *index, "switch a tab");
            vec![]
        }

        // Acciones no implementadas aún — no producen efectos
        Action::Noop
        | Action::FocusPanel(_)
        | Action::OpenFile(_) => vec![],
    }
}

mod helpers;
use helpers::{buffer_full_text, get_workspace_root, notify_lsp_change};

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
    let needs_open = state.tabs.active().buffer.file_path()
        .is_none_or(|current| current != abs_path);

    if needs_open {
        match state.tabs.open_file(&abs_path) {
            Ok(()) => {
                state.tabs.active_mut().init_highlighting(&state.highlight_engine);
                tracing::info!(path = %abs_path.display(), "archivo abierto desde search");
            }
            Err(e) => {
                tracing::error!(error = %e, "error al abrir archivo desde search");
                return;
            }
        }
    }

    // Posicionar cursor
    let max_line = state.tabs.active().buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = state.tabs.active().buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    let editor = state.tabs.active_mut();
    let primary = editor.cursors.primary_mut();
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    primary.clear_selection();
    let pos = state.tabs.active().cursors.primary().position;
    state.tabs.active_mut().viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();
}

mod mouse;
use mouse::{reduce_mouse_click, reduce_mouse_drag, reduce_mouse_scroll, ScrollDirection};

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
                    state.branch_picker.visible,
                    state.search.visible,
                    &state.git,
                    state.lsp.completion_visible,
                    state.keybindings.visible,
                    state.keybindings.editing_index.is_some(),
                    &state.commands,
                )
            }
            Some(Event::Tick) => Action::Noop,
            _ => Action::Noop,
        };

        // 4. Registrar evento procesado
        if event.is_some() {
            state.metrics.record_event();
        }

        // 4.5. Cursor blink: togglear visibilidad cada 10 ticks (~500ms).
        //      Reset en cualquier input del usuario (cursor siempre visible al teclear).
        match &event {
            Some(Event::Tick) => {
                state.cursor_blink_counter += 1;
                // 8 ticks * 50ms = 400ms blink period
                if state.cursor_blink_counter >= 8 {
                    state.cursor_blink_counter = 0;
                    state.cursor_visible = !state.cursor_visible;
                }
            }
            Some(Event::Input(_)) => {
                // Cualquier input del usuario: cursor visible, reset counter
                state.cursor_visible = true;
                state.cursor_blink_counter = 0;
            }
            // Otros eventos async (FileLoaded, SearchResult, etc.): no afectan blink
            Some(_) | None => {}
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

            // Ajustar scroll del settings overlay
            if state.keybindings.visible {
                // Estimado: alto del overlay menos header/footer (~6 líneas de chrome)
                let settings_vis = (term_height as usize).saturating_sub(10);
                state.keybindings.ensure_visible(settings_vis);
            }

            // Ajustar scroll del git panel
            if state.git.visible {
                // Descontar branch line(1) + commit input(2 si aplica)
                let commit_lines = if state.git.commit_mode { 2 } else { 0 };
                let git_list_height = sidebar_height.saturating_sub(1 + commit_lines);
                state.git.ensure_visible(git_list_height);
            }
        }

        // 6.8. Pre-computar flat cache del explorer para evitar recompute en render.
        //      Se hace antes del render — el render solo lee el cache via cached_flat().
        if let Some(ref mut explorer) = state.explorer {
            explorer.ensure_flat_cache();
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
        //      con dimensiones correctas. Descontar bordes (2) + tab bar (1) + gutter dinámico.
        {
            // Restar 2 líneas para la barra de tabs (1) + breadcrumbs (1)
            let chrome_lines: usize = 2; // tab bar + breadcrumbs
            let editor_inner_h = (layout.editor_area.height.saturating_sub(2) as usize)
                .saturating_sub(chrome_lines);
            let editor_inner_w = layout.editor_area.width.saturating_sub(2) as usize;
            // Gutter width dinámico: dígitos del total de líneas (mín 4) + 2 (separador)
            let editor = state.tabs.active_mut();
            let total_lines = editor.buffer.line_count();
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
            editor.viewport.update_size(text_width, editor_inner_h);
        }

        // 7.6. Ensure syntax highlight — carga lazy del engine + viewport-aware cache.
        //      Se hace acá para NO alocar dentro del render loop.
        {
            // Intentar inicializar el engine si la carga en background terminó.
            // Si se inicializó en este frame, marcar caches de todas las tabs como dirty.
            let just_loaded = state.highlight_engine.try_init();
            if just_loaded {
                // El engine acaba de cargar — marcar todas las tabs como dirty
                // para que sus caches se re-procesen con colores
                for editor in state.tabs.all_editors_mut() {
                    editor.highlight_cache.invalidate();
                }
                tracing::info!("highlight engine listo — re-highlighting todos los buffers");
            }

            // Solo procesar highlight si el engine está listo
            if state.highlight_engine.is_ready() {
                let engine = &state.highlight_engine;
                let editor = state.tabs.active_mut();
                let vp_start = editor.viewport.scroll_offset;
                let vp_height = editor.viewport.height;
                editor.highlight_cache.ensure_viewport_highlighted(
                    &editor.buffer,
                    engine,
                    vp_start,
                    vp_height,
                );
            }
        }

        // 7.7. Pre-cache progresivo de highlighting en idle frames.
        //      Si el último evento fue un tick (no hubo input de usuario),
        //      usar el tiempo idle para pre-cachear el archivo activo.
        //      Solo 1 editor por frame para no exceder budget de frame time.
        {
            let was_tick = matches!(event, Some(Event::Tick));
            if was_tick && state.highlight_engine.is_ready() {
                let engine = &state.highlight_engine;
                let editor = state.tabs.active_mut();
                editor.highlight_cache.precache_chunk(
                    &editor.buffer,
                    engine,
                );
            }
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
                match state.tabs.open_file(&path) {
                    Ok(()) => {
                        state.tabs.active_mut().init_highlighting(&state.highlight_engine);
                        // Posicionar cursor en la definición
                        let primary = state.tabs.active_mut().cursors.primary_mut();
                        primary.position.line = def_result.line as usize;
                        primary.position.col = def_result.col as usize;
                        primary.sync_desired_col();
                        primary.clear_selection();
                        let pos = state.tabs.active().cursors.primary().position;
                        state.tabs.active_mut().viewport.ensure_cursor_visible(&pos);
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
            if let Some(path) = state.tabs.active().buffer.file_path().map(|p| p.to_path_buf()) {
                let cursor_line = state.tabs.active().cursors.primary().position.line as u32;
                state.lsp.update_status_for_cursor(&path, cursor_line);
            }

            // Flush de did_change pendiente (debounce)
            let editor_ref = state.tabs.active();
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


