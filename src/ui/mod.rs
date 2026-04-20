//! UI: composición de panes, foco, render con ratatui, theme/tokens cyberpunk.
//!
//! Este módulo concentra todo lo visual: el shell de la aplicación,
//! el layout de paneles, el tema de colores y la función de render.
//! Los widgets son stateless renderers — reciben datos pre-computados
//! y dibujan. Nada de IO ni cómputo pesado en render.

pub mod branch_picker;
pub mod git_panel;
pub mod icons;
pub mod layout;
pub mod palette;
pub mod panels;
pub mod quick_open;
pub mod search_panel;
pub mod settings_panel;
pub mod theme;

pub use theme::Theme;

use ratatui::Frame;

// ─── String Truncation Helper ──────────────────────────────────────────────────

/// Trunca un `&str` a un máximo de `max_width` caracteres (no bytes).
///
/// Retorna un slice válido que nunca corta caracteres multi-byte (UTF-8).
/// Esto es necesario porque `&str[..n]` con `n` en medio de un carácter
/// multi-byte causa panic. Anchos de viewport/columna son caracteres,
/// no bytes — usar esta función en lugar de slicing directo.
///
/// # Ejemplo
/// ```ignore
/// let s = "─hello";
/// // s.len() == 6 (─ ocupa 3 bytes UTF-8)
/// // truncate_str(s, 3) == "─he" (3 caracteres, no 3 bytes)
/// ```
pub(crate) fn truncate_str(s: &str, max_width: usize) -> &str {
    if s.len() <= max_width {
        // Fast path: si el total de bytes <= max_width, el string
        // tiene como mucho max_width caracteres (cada char >= 1 byte)
        return s;
    }
    // Encontrar el byte offset del carácter en posición max_width
    match s.char_indices().nth(max_width) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s, // string tiene menos de max_width caracteres
    }
}

use crate::app::AppState;
use crate::core::settings::SidebarSection;
use crate::core::PanelId;

use layout::IdeLayout;
use panels::StatusBarData;

/// Renderiza el frame completo del IDE.
///
/// Usa el layout pre-computado de `state.last_layout` (calculado antes
/// del render en el event loop). Solo recomputa como fallback si
/// `last_layout` no existe (primer frame). Los datos para la status bar
/// se derivan del estado ANTES de entrar al render — sin allocaciones
/// dentro del draw.
///
/// La función recibe `&AppState` y `&Theme` por referencia.
/// El theme se crea una vez fuera del event loop.
pub fn render(f: &mut Frame, state: &AppState, theme: &Theme) {
    let area = f.area();

    // Usar layout pre-computado del event loop. Fallback a recompute solo
    // en el primer frame antes de que last_layout exista.
    let layout = state.last_layout.unwrap_or_else(|| {
        IdeLayout::compute(area, state.sidebar_visible, state.bottom_panel_visible)
    });

    // Determinar qué panel tiene foco
    let focused = state.focused_panel;

    // ── Title bar ──
    panels::render_title_bar(f, layout.title_bar, theme);

    // ── Activity bar ──
    // Determinar sección activa de la sidebar para highlight de icono.
    // La activity bar se renderiza SIEMPRE — no depende de sidebar_visible.
    let active_section = if state.search.visible {
        SidebarSection::Search
    } else if state.git.visible {
        SidebarSection::Git
    } else {
        SidebarSection::Explorer
    };
    panels::render_activity_bar(
        f,
        layout.activity_bar,
        theme,
        active_section,
        state.keybindings.visible,
    );

    // ── Sidebar ──
    // Prioridad de paneles en sidebar: search > git > explorer
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
        } else if state.git.visible {
            // Git activo: renderizar panel de git en la sidebar
            git_panel::render_git_panel(f, layout.sidebar, &state.git, theme, sidebar_focused);
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
    let editor = state.tabs.active();
    // Pre-computar info de tabs para la barra de pestañas
    let tab_infos = state.tabs.tab_info();
    // Obtener diagnósticos para el archivo actual (si hay LSP activo)
    let current_diagnostics = editor
        .buffer
        .file_path()
        .map(|p| state.lsp.diagnostics_for(p))
        .unwrap_or(&[]);
    // Path del archivo activo y workspace root para breadcrumbs
    let active_file_path = editor.buffer.file_path();
    let workspace_root = state.explorer.as_ref().map(|e| e.root.as_path());
    panels::render_editor_area(
        f,
        layout.editor_area,
        theme,
        editor_focused,
        editor,
        current_diagnostics,
        &tab_infos,
        state.bracket_match,
        active_file_path,
        workspace_root,
    );

    // ── Hardware cursor: posicionar la línea vertical del terminal ──
    // Solo cuando el editor tiene foco, no hay overlays activos, y el cursor es visible
    // (blink). Cuando cursor_visible es false, no se posiciona — la terminal oculta el cursor.
    if editor_focused
        && state.cursor_visible
        && !state.palette.visible
        && !state.quick_open.visible
        && !state.branch_picker.visible
        && !state.keybindings.visible
    {
        // Inner area del editor (descontar bordes del Block + tab bar + breadcrumbs)
        let inner_x = layout.editor_area.x + 1;
        // +1 borde superior, +1 tab bar, +1 breadcrumbs = +3 desde editor_area.y
        let chrome_offset: u16 = 2; // tab bar (1) + breadcrumbs (1)
        let inner_y = layout.editor_area.y + 1 + chrome_offset;
        let inner_h = layout.editor_area.height.saturating_sub(2 + chrome_offset) as usize;

        let editor = state.tabs.active();
        let scroll = editor.viewport.scroll_offset;
        let cursor_line = editor.cursors.primary().position.line;
        let cursor_col = editor.cursors.primary().position.col;

        // Verificar que el cursor está dentro del viewport visible
        if cursor_line >= scroll && cursor_line < scroll + inner_h {
            let visual_row = (cursor_line - scroll) as u16;

            // Gutter width: dígitos del total de líneas (mín 4) + separador (2)
            let total_lines = editor.buffer.line_count();
            let gutter_width = panels::digit_count(total_lines).max(4);
            let separator_width: u16 = 2;
            let text_offset = gutter_width as u16 + separator_width;

            let abs_col = inner_x + text_offset + cursor_col as u16;
            let abs_row = inner_y + visual_row;

            f.set_cursor_position((abs_col, abs_row));
        }
    }

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
    // Branch real del repo git (o fallback si no es repo)
    let branch_display = if state.git.is_repo && !state.git.branch.is_empty() {
        &state.git.branch
    } else if state.git.is_repo {
        "(detached)"
    } else {
        "no git"
    };
    // Si hay un mensaje de diagnóstico LSP, mostrarlo como file_name override
    let file_display = state
        .lsp
        .status_message
        .as_deref()
        .unwrap_or(&state.status_file);
    let status_data = StatusBarData {
        mode: if state.lsp.has_server() {
            "LSP"
        } else {
            "NORMAL"
        },
        file_name: file_display,
        cursor_pos: &state.status_line,
        branch: branch_display,
        encoding: "UTF-8",
    };
    panels::render_status_bar(f, layout.status_bar, theme, &status_data);

    // ── LSP Overlays (hover, completions) ──
    // Se renderizan antes de los overlays modales (palette, quick open)
    // porque los modales tienen prioridad visual.
    if editor_focused
        && !state.palette.visible
        && !state.quick_open.visible
        && !state.branch_picker.visible
        && !state.keybindings.visible
    {
        // Hover tooltip
        if let Some(ref hover) = state.lsp.hover_content {
            panels::render_lsp_hover(f, layout.editor_area, theme, hover, state.tabs.active());
        }

        // Completion dropdown
        if state.lsp.completion_visible && !state.lsp.completions.is_empty() {
            panels::render_lsp_completions(
                f,
                layout.editor_area,
                theme,
                &state.lsp.completions,
                state.lsp.completion_selected,
                state.tabs.active(),
            );
        }
    }

    // ── Overlays ──
    // Prioridad: Settings > Branch picker > Quick open > Palette.
    // Clear + dibujo garantizan que el overlay tape lo que hay debajo.
    if state.keybindings.visible {
        settings_panel::render_settings(f, area, &state.keybindings, theme);
    } else if state.branch_picker.visible {
        branch_picker::render_branch_picker(f, area, &state.branch_picker, theme);
    } else if state.quick_open.visible {
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
