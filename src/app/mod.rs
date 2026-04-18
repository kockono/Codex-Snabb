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
    event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio_util::sync::CancellationToken;

use crate::core::command::CommandRegistry;
use crate::core::{Action, AppConfig, Direction, Effect, Event, PanelId};
use crate::editor::EditorState;
use crate::observe::{FrameTimer, Metrics};
use crate::ui::{self, Theme};
use crate::ui::palette::PaletteState;
use crate::workspace::ExplorerState;

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
    /// Datos pre-computados para la status bar (se actualizan en cada frame).
    /// Evita allocaciones dentro del render — se computan antes.
    pub status_line: String,
    pub status_file: String,
}

impl AppState {
    /// Crea un nuevo estado con valores por defecto y editor vacío.
    ///
    /// Intenta inicializar el explorer con el directorio de trabajo actual.
    /// Si falla, el explorer queda como `None` — la app funciona sin él.
    fn new(config: AppConfig) -> Self {
        let explorer = std::env::current_dir()
            .ok()
            .and_then(|cwd| {
                ExplorerState::new(&cwd)
                    .map_err(|e| tracing::warn!(error = %e, "no se pudo inicializar explorer"))
                    .ok()
            });

        let mut commands = CommandRegistry::new();
        commands.register_defaults();

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
            status_line: String::from("Ln 1, Col 1"),
            status_file: String::from("[no file]"),
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

        let explorer = explorer_root.and_then(|root| {
            ExplorerState::new(&root)
                .map_err(|e| tracing::warn!(error = %e, "no se pudo inicializar explorer"))
                .ok()
        });

        let mut commands = CommandRegistry::new();
        commands.register_defaults();

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
            status_line: String::from("Ln 1, Col 1"),
            status_file,
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
/// distintas según el panel enfocado y si la palette está abierta:
///
/// - **Palette abierta**: captura TODO el input (Esc cierra, Enter confirma,
///   flechas navegan, chars se escriben en búsqueda)
/// - **Global**: Ctrl+atajos, Esc, Tab (siempre activos cuando palette cerrada)
/// - **Editor**: flechas mueven cursor, chars insertan texto
/// - **Explorer**: flechas navegan el árbol, Enter abre/expande
fn keymap(
    event: &crossterm::event::Event,
    focused_panel: PanelId,
    palette_visible: bool,
) -> Action {
    let CrosstermEvent::Key(key) = event else {
        return Action::Noop;
    };
    if key.kind != KeyEventKind::Press {
        return Action::Noop;
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
            state.palette.open(&state.commands);
            tracing::debug!("command palette abierta");
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

        // Acciones no implementadas aún — no producen efectos
        Action::Noop
        | Action::FocusPanel(_)
        | Action::OpenFile(_)
        | Action::CloseBuffer
        | Action::OpenQuickOpen
        | Action::OpenGlobalSearch
        | Action::ToggleTerminal
        | Action::OpenGitPanel => vec![],
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

    // Setup terminal: raw mode + alternate screen
    terminal::enable_raw_mode()
        .context("no se pudo activar raw mode")?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen)
        .context("no se pudo entrar a alternate screen")?;

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

        // 3. Mapear evento a acción (sensible al panel enfocado y palette)
        let action = match &event {
            Some(Event::Input(crossterm_event)) => {
                keymap(crossterm_event, state.focused_panel, state.palette.visible)
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

        // 7. Render frame actual
        terminal.draw(|frame| {
            ui::render(frame, &state, theme);
        }).context("error en render")?;

        // 8. Registrar métricas del frame (solo reduce + render, no poll wait)
        let frame_time = frame_timer.elapsed_us();
        state.metrics.record_frame(frame_time);
        state.metrics.record_input_latency(frame_time);

        // 9. Log de warning si el frame excede el budget target
        if crate::core::budgets::DEFAULT_BUDGETS.frame_exceeds_target(frame_time) {
            tracing::warn!(
                frame_time_us = frame_time,
                avg_us = state.metrics.avg_frame_time_us,
                "frame excede budget target de 16ms"
            );
        }

        // 10. Salir si el estado lo indica o shutdown externo
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
    crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)
        .context("no se pudo salir de alternate screen")?;
    Ok(())
}
