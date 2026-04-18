//! Layout: cómputo del layout principal tipo IDE.
//!
//! Define las áreas del shell visual: title bar, sidebar, editor, bottom panel
//! y status bar. El layout se computa una vez y se invalida solo en resize
//! o toggle de paneles — nunca se recalcula cada frame sin cambio.
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │                  Title Bar                    │
//! ├──────────┬───────────────────────────────────┤
//! │          │                                   │
//! │ Sidebar  │          Editor Area              │
//! │          │                                   │
//! │          ├───────────────────────────────────┤
//! │          │        Bottom Panel               │
//! ├──────────┴───────────────────────────────────┤
//! │                 Status Bar                    │
//! └──────────────────────────────────────────────┘
//! ```

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Resultado del cómputo de layout principal del IDE.
///
/// Contiene las áreas precalculadas para cada región del shell.
/// Se computa via `IdeLayout::compute()` y se cachea hasta que
/// cambie el tamaño de terminal o se toggle un panel.
#[derive(Debug, Clone, Copy)]
pub struct IdeLayout {
    /// Barra superior con nombre del IDE (1 línea).
    pub title_bar: Rect,
    /// Panel lateral izquierdo (explorer, git, search).
    pub sidebar: Rect,
    /// Área principal del editor de texto.
    pub editor_area: Rect,
    /// Panel inferior (terminal, problems, output).
    pub bottom_panel: Rect,
    /// Barra inferior de estado (1 línea).
    pub status_bar: Rect,
    /// Si la sidebar está visible en este layout.
    pub sidebar_visible: bool,
    /// Si el bottom panel está visible en este layout.
    pub bottom_panel_visible: bool,
}

/// Ancho mínimo de la sidebar en columnas.
const SIDEBAR_MIN_COLS: u16 = 20;
/// Ancho máximo de la sidebar en columnas.
const SIDEBAR_MAX_COLS: u16 = 40;
/// Porcentaje del ancho total para la sidebar.
const SIDEBAR_PCT: u16 = 20;
/// Altura mínima del bottom panel en líneas.
const BOTTOM_PANEL_MIN_ROWS: u16 = 5;
/// Porcentaje del alto del área central para el bottom panel.
const BOTTOM_PANEL_PCT: u16 = 30;

impl IdeLayout {
    /// Computa el layout del IDE para un área dada.
    ///
    /// El layout respeta los mínimos y máximos de cada panel.
    /// Se debe llamar solo cuando cambia `area`, `sidebar_visible`
    /// o `bottom_panel_visible` — no en cada frame.
    pub fn compute(area: Rect, sidebar_visible: bool, bottom_panel_visible: bool) -> Self {
        // Layout vertical principal: title bar (1) + center + status bar (1)
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // title bar
                Constraint::Fill(1),   // área central (sidebar + editor + bottom)
                Constraint::Length(1), // status bar
            ])
            .split(area);

        let title_bar = vertical[0];
        let center = vertical[1];
        let status_bar = vertical[2];

        // Layout horizontal del centro: sidebar + main content
        let (sidebar, main_area) = if sidebar_visible {
            let sidebar_width = compute_sidebar_width(center.width);
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sidebar_width), Constraint::Fill(1)])
                .split(center);
            (horizontal[0], horizontal[1])
        } else {
            (Rect::default(), center)
        };

        // Layout vertical del main content: editor + bottom panel
        let (editor_area, bottom_panel) = if bottom_panel_visible {
            let bottom_height = compute_bottom_panel_height(main_area.height);
            let vertical_main = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(bottom_height)])
                .split(main_area);
            (vertical_main[0], vertical_main[1])
        } else {
            (main_area, Rect::default())
        };

        Self {
            title_bar,
            sidebar,
            editor_area,
            bottom_panel,
            status_bar,
            sidebar_visible,
            bottom_panel_visible,
        }
    }
}

/// Calcula el ancho de la sidebar respetando mínimos y máximos.
///
/// ~20% del ancho total, clamped entre `SIDEBAR_MIN_COLS` y `SIDEBAR_MAX_COLS`.
fn compute_sidebar_width(total_width: u16) -> u16 {
    let pct_width = total_width * SIDEBAR_PCT / 100;
    pct_width.clamp(SIDEBAR_MIN_COLS, SIDEBAR_MAX_COLS)
}

/// Calcula la altura del bottom panel respetando el mínimo.
///
/// ~30% del alto disponible, con mínimo de `BOTTOM_PANEL_MIN_ROWS`.
fn compute_bottom_panel_height(total_height: u16) -> u16 {
    let pct_height = total_height * BOTTOM_PANEL_PCT / 100;
    pct_height.max(BOTTOM_PANEL_MIN_ROWS)
}
