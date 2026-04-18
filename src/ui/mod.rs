//! UI: composición de panes, foco, render con ratatui, theme/tokens cyberpunk.
//!
//! Este módulo concentra todo lo visual: el shell de la aplicación,
//! el layout de paneles, el tema de colores y la función de render.
//! Los widgets son stateless renderers — reciben datos pre-computados
//! y dibujan. Nada de IO ni cómputo pesado en render.

pub mod layout;
pub mod palette;
pub mod panels;
pub mod quick_open;
pub mod search_panel;
pub mod theme;

pub use theme::Theme;

use ratatui::Frame;

use crate::app::AppState;
use crate::core::PanelId;

use layout::IdeLayout;
use panels::StatusBarData;

/// Renderiza el frame completo del IDE.
///
/// Computa el layout, determina qué panel tiene foco, y renderiza
/// cada región. Los datos para la status bar se derivan del estado
/// ANTES de entrar al render — sin allocaciones dentro del draw.
///
/// La función recibe `&AppState` y `&Theme` por referencia.
/// El theme se crea una vez fuera del event loop.
pub fn render(f: &mut Frame, state: &AppState, theme: &Theme) {
    let area = f.area();

    // Computar layout — en el futuro se cacheará y solo se recalculará
    // en resize o toggle de paneles
    let layout = IdeLayout::compute(area, state.sidebar_visible, state.bottom_panel_visible);

    // Determinar qué panel tiene foco
    let focused = state.focused_panel;

    // ── Title bar ──
    panels::render_title_bar(f, layout.title_bar, theme);

    // ── Sidebar ──
    if layout.sidebar_visible {
        let sidebar_focused = matches!(focused, PanelId::Explorer | PanelId::Git | PanelId::Search);
        if state.search.visible {
            // Búsqueda activa: renderizar panel de búsqueda en la sidebar
            search_panel::render_search_panel(
                f,
                layout.sidebar,
                &state.search,
                theme,
                sidebar_focused,
            );
        } else {
            panels::render_sidebar(
                f,
                layout.sidebar,
                theme,
                sidebar_focused,
                sidebar_active_panel(focused),
                state.explorer.as_ref(),
            );
        }
    }

    // ── Editor area ──
    let editor_focused = focused == PanelId::Editor;
    panels::render_editor_area(f, layout.editor_area, theme, editor_focused);

    // ── Bottom panel ──
    if layout.bottom_panel_visible {
        let bottom_focused = focused == PanelId::Terminal;
        panels::render_bottom_panel(
            f,
            layout.bottom_panel,
            theme,
            bottom_focused,
            state.terminal.session.as_ref(),
        );
    }

    // ── Status bar ──
    // Datos pre-computados desde AppState — sin allocaciones acá
    let status_data = StatusBarData {
        mode: "NORMAL",
        file_name: &state.status_file,
        cursor_pos: &state.status_line,
        branch: " main",
        encoding: "UTF-8",
    };
    panels::render_status_bar(f, layout.status_bar, theme, &status_data);

    // ── Overlays ──
    // Solo un overlay a la vez. Quick open tiene prioridad visual sobre palette.
    // Clear + dibujo garantizan que el overlay tape lo que hay debajo.
    if state.quick_open.visible {
        quick_open::render_quick_open(f, area, &state.quick_open, theme);
    } else if state.palette.visible {
        palette::render_palette(f, area, &state.palette, &state.commands, theme);
    }
}

/// Determina qué sub-panel de la sidebar está activo según el foco.
///
/// Si el foco está en Explorer/Git/Search, ese es el panel activo.
/// En cualquier otro caso, default a Explorer.
fn sidebar_active_panel(focused: PanelId) -> PanelId {
    match focused {
        PanelId::Explorer | PanelId::Git | PanelId::Search => focused,
        _ => PanelId::Explorer,
    }
}
