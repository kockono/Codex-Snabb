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

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio_util::sync::CancellationToken;

use crate::core::{Action, AppConfig, Effect, Event, PanelId};
use crate::observe::{FrameTimer, Metrics};
use crate::ui::{self, Theme};

// ─── AppState ──────────────────────────────────────────────────────────────────

/// Estado central de la aplicación.
///
/// Contiene todo el estado mutable del sistema. El reducer lo modifica
/// en respuesta a acciones y produce efectos. Los subsistemas futuros
/// (editor, workspace, search, etc.) agregarán sus sub-estados acá.
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
}

impl AppState {
    /// Crea un nuevo estado con valores por defecto.
    fn new(config: AppConfig) -> Self {
        Self {
            running: true,
            focused_panel: PanelId::Editor,
            config,
            metrics: Metrics::new(),
            sidebar_visible: true,
            bottom_panel_visible: true,
        }
    }
}

// ─── Keymap ────────────────────────────────────────────────────────────────────

/// Mapea un evento de crossterm a una acción del sistema.
///
/// Solo procesa key press events (ignora release y repeat).
/// Retorna `Action::Noop` para eventos no mapeados.
///
/// Mapeos actuales:
/// - `q` / `Esc` → `Action::Quit`
/// - `Tab` → `Action::FocusNext`
/// - `Shift+Tab` (BackTab) → `Action::FocusPrev`
/// - `Ctrl+B` → `Action::ToggleSidebar`
/// - `Ctrl+J` → `Action::ToggleBottomPanel`
/// - cualquier otro → `Action::Noop`
fn keymap(event: &crossterm::event::Event) -> Action {
    if let CrosstermEvent::Key(key) = event {
        if key.kind != KeyEventKind::Press {
            return Action::Noop;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Esc, _) => Action::Quit,
            (KeyCode::Tab, KeyModifiers::NONE) => Action::FocusNext,
            (KeyCode::BackTab, KeyModifiers::SHIFT) => Action::FocusPrev,
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => Action::ToggleSidebar,
            (KeyCode::Char('j'), KeyModifiers::CONTROL) => Action::ToggleBottomPanel,
            _ => Action::Noop,
        }
    } else {
        Action::Noop
    }
}

// ─── Reducer ───────────────────────────────────────────────────────────────────

/// Reducer puro: actualiza estado según la acción y retorna efectos.
///
/// Este es el corazón del message passing. Toda mutación de estado
/// pasa por acá. Los efectos retornados se procesan fuera del reducer.
///
/// Garantías:
/// - No ejecuta IO
/// - No aloca en heap (retorna Vec, pero con capacidad mínima)
/// - Determinístico: misma entrada → misma salida
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
        // Acciones no implementadas aún — no producen efectos
        Action::Noop
        | Action::FocusPanel(_)
        | Action::InsertChar(_)
        | Action::DeleteChar
        | Action::MoveCursor(_)
        | Action::OpenFile(_)
        | Action::SaveFile
        | Action::CloseBuffer
        | Action::OpenCommandPalette
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
/// Setup de terminal → event loop → cleanup.
/// Retorna `Result<()>` para propagar errores al caller (main).
pub async fn run() -> Result<()> {
    let _config = AppConfig::new();
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
    let result = event_loop(&mut terminal, &shutdown, &theme).await;

    // Cleanup: SIEMPRE restaurar terminal, incluso si hubo error
    cleanup_terminal()?;

    result
}

// ─── Event Loop ────────────────────────────────────────────────────────────────

/// Event loop principal.
///
/// Ciclo: poll evento → keymap → reduce → process effects → render.
/// Instrumentado con `FrameTimer` y `Metrics` para observabilidad.
///
/// No usa busy-polling: `event::poll` con timeout evita CPU idle > 0%.
/// El tick_rate controla la frecuencia máxima de polling.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    shutdown: &CancellationToken,
    theme: &Theme,
) -> Result<()> {
    let mut state = AppState::new(AppConfig::new());
    let tick_duration = Duration::from_millis(state.config.tick_rate_ms);

    loop {
        // Iniciar medición del frame completo
        let frame_timer = FrameTimer::start();

        // Render frame actual
        terminal.draw(|frame| {
            ui::render(frame, &state, theme);
        }).context("error en render")?;

        // Poll de eventos con timeout (no busy-poll)
        let event = poll_event(tick_duration)?;

        // Mapear evento a acción
        let action = match &event {
            Some(Event::Input(crossterm_event)) => keymap(crossterm_event),
            Some(Event::Tick) => Action::Noop,
            _ => Action::Noop,
        };

        // Registrar evento procesado
        if event.is_some() {
            state.metrics.record_event();
        }

        // Reducer: actualizar estado y obtener efectos
        let effects = reduce(&mut state, &action);

        // Procesar efectos
        process_effects(&effects, shutdown);

        // Registrar métricas del frame
        let frame_time = frame_timer.elapsed_us();
        state.metrics.record_frame(frame_time);
        state.metrics.record_input_latency(frame_time);

        // Log de warning si el frame excede el budget target
        if crate::core::budgets::DEFAULT_BUDGETS.frame_exceeds_target(frame_time) {
            tracing::warn!(
                frame_time_us = frame_time,
                avg_us = state.metrics.avg_frame_time_us,
                "frame excede budget target de 16ms"
            );
        }

        // Salir si el estado lo indica o shutdown externo
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
///
/// Retorna `Some(Event)` si hay evento disponible dentro del timeout,
/// `None` si no hay eventos (tick implícito).
/// No bloquea indefinidamente — respeta el tick rate.
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
///
/// Crítico: debe ejecutarse SIEMPRE, incluso en panic.
/// Raw mode off + leave alternate screen + show cursor.
fn cleanup_terminal() -> Result<()> {
    terminal::disable_raw_mode()
        .context("no se pudo desactivar raw mode")?;
    crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)
        .context("no se pudo salir de alternate screen")?;
    Ok(())
}
