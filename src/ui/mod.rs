//! UI: composición de panes, foco, render con ratatui, theme/tokens cyberpunk.
//!
//! Este módulo concentra todo lo visual: el shell de la aplicación,
//! el layout de paneles, el tema de colores y la función de render.
//! Los widgets son stateless renderers — reciben datos pre-computados
//! y dibujan. Nada de IO ni cómputo pesado en render.

pub mod branch_picker;
pub mod git_panel;
pub mod go_to_line;
pub mod icons;
pub mod layout;
pub mod palette;
pub mod panels;
pub mod projects_panel;
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
    } else if state.projects.visible {
        SidebarSection::Projects
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
    // Prioridad de paneles en sidebar: search > git > projects > explorer
    if layout.sidebar_visible {
        let sidebar_focused = matches!(
            focused,
            PanelId::Explorer | PanelId::Git | PanelId::Search | PanelId::Projects
        );
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
        } else if state.projects.visible {
            // Proyectos activo: renderizar panel de proyectos en la sidebar
            projects_panel::render_projects_panel(
                f,
                layout.sidebar,
                theme,
                &state.projects,
                sidebar_focused,
                state.cursor_visible,
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
        && !state.go_to_line.visible
        && !state.branch_picker.visible
        && !state.keybindings.visible
        && !state.folder_picker.visible
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
    // Datos pre-computados desde AppState — sin allocaciones en render
    // Pre-format git status string: "⎇ main ↑2 ↓1 ⟳" — una vez por frame, fuera del render loop
    use std::fmt::Write;
    let git_status_str: String = if state.git.is_repo {
        let branch = if state.git.branch.is_empty() {
            "(detached)"
        } else {
            &state.git.branch
        };
        let mut s = String::with_capacity(32);
        s.push_str("\u{2387} "); // ⎇
        s.push_str(branch);
        if state.git.ahead > 0 {
            s.push_str(" \u{2191}"); // ↑
            let _ = write!(s, "{}", state.git.ahead);
        }
        if state.git.behind > 0 {
            s.push_str(" \u{2193}"); // ↓
            let _ = write!(s, "{}", state.git.behind);
        }
        s.push_str(" \u{27F3}"); // ⟳
        s
    } else {
        String::from("no git")
    };
    let status_data = StatusBarData {
        mode: if state.lsp.has_server() {
            "LSP"
        } else {
            "NORMAL"
        },
        cursor_pos: &state.status_line,
        git_status: &git_status_str,
        encoding: "UTF-8",
        scroll_pct: &state.status_pct,
    };
    panels::render_status_bar(f, layout.status_bar, theme, &status_data);

    // ── LSP Overlays (hover, completions) ──
    // Se renderizan antes de los overlays modales (palette, quick open)
    // porque los modales tienen prioridad visual.
    if editor_focused
        && !state.palette.visible
        && !state.quick_open.visible
        && !state.go_to_line.visible
        && !state.branch_picker.visible
        && !state.keybindings.visible
        && !state.folder_picker.visible
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

    // ── Folder picker modal (máxima prioridad visual) ──
    if state.folder_picker.visible {
        projects_panel::render_folder_picker(f, &layout, theme, &state.folder_picker);
    }

    // ── Overlays ──
    // Prioridad: Go to Line > Settings > Branch picker > Quick open > Palette.
    // Clear + dibujo garantizan que el overlay tape lo que hay debajo.
    if state.go_to_line.visible {
        // Pre-format hint fuera del render — evitar format!() en render
        use std::fmt::Write as FmtWrite;
        let mut go_to_line_hint = String::with_capacity(16);
        let _ = write!(go_to_line_hint, "1 \u{2013} {}", state.go_to_line.total_lines);
        go_to_line::render_go_to_line(f, &layout, &state.go_to_line, theme, &go_to_line_hint);
    } else if state.keybindings.visible {
        settings_panel::render_settings(f, &layout, &state.keybindings, theme);
    } else if state.branch_picker.visible {
        branch_picker::render_branch_picker(f, &layout, &state.branch_picker, theme);
    } else if state.quick_open.visible {
        let active_file_name = state.tabs.active().buffer.file_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("[sin archivo]");
        quick_open::render_quick_open(f, &layout, &state.quick_open, theme, active_file_name, state.cursor_visible);
    } else if state.palette.visible {
        palette::render_palette(f, &layout, &state.palette, &state.commands, theme);
    }
}

/// Renderiza una pantalla de carga futurista con barra de progreso.
///
/// Se muestra durante la inicialización diferida (explorer, quick_open,
/// git, highlight). Recibe campos individuales en vez de un struct
/// para evitar dependencia circular (ui no importa app).
///
/// Allocaciones por frame: 2 Strings pequeños (bar + pct) — aceptable
/// en fase de startup, fuera del event loop principal.
pub fn render_loading(
    frame: &mut Frame,
    theme: &Theme,
    step: &str,
    progress: f32,
    done: bool,
) {
    use ratatui::{
        layout::Rect,
        style::{Modifier, Style},
        text::{Line, Span},
        widgets::{Block, BorderType, Borders, Paragraph},
    };

    let area = frame.area();

    // Fondo completo
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg_primary)),
        area,
    );

    if area.width < 20 || area.height < 8 {
        return;
    }

    // Caja central: 60% del ancho, 10 líneas, centrada
    let box_w = (area.width * 60 / 100).clamp(40, 80);
    let box_h: u16 = 10;
    let box_x = area.width.saturating_sub(box_w) / 2;
    let box_y = area.height.saturating_sub(box_h) / 2;
    let box_area = Rect::new(box_x, box_y, box_w, box_h);

    // Borde doble cyberpunk
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);

    if inner.height < 6 || inner.width < 10 {
        return;
    }

    // Layout interno:
    // +0: (blank)
    // +1: título
    // +2: subtítulo
    // +3: (blank)
    // +4: paso actual
    // +5: (blank)
    // +6: barra de progreso
    // +7: porcentaje

    // ── Título ──
    let title = "\u{2588} IDE TUI \u{2588}"; // █ IDE TUI █
    let title_len = title.chars().count() as u16;
    let title_x = inner.x + inner.width.saturating_sub(title_len) / 2;
    if inner.y + 1 < area.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(theme.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            Rect::new(title_x, inner.y + 1, title_len, 1),
        );
    }

    // ── Subtítulo ──
    let subtitle = "Rust TUI IDE \u{2014} RAM/CPU First"; // —
    let sub_len = subtitle.chars().count() as u16;
    let sub_x = inner.x + inner.width.saturating_sub(sub_len) / 2;
    if inner.y + 2 < area.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                subtitle,
                Style::default().fg(theme.fg_secondary),
            ))),
            Rect::new(sub_x, inner.y + 2, sub_len, 1),
        );
    }

    // ── Paso actual ──
    let step_len = step.chars().count() as u16;
    let step_display_w = step_len.min(inner.width.saturating_sub(2));
    let step_x = inner.x + inner.width.saturating_sub(step_display_w) / 2;
    if inner.y + 4 < area.height {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                step,
                Style::default().fg(theme.fg_accent_alt),
            ))),
            Rect::new(step_x, inner.y + 4, step_display_w, 1),
        );
    }

    // ── Barra de progreso ──
    // Una String por frame en startup — aceptable (no es el event loop)
    let bar_w = inner.width.saturating_sub(4) as usize;
    if bar_w > 0 && inner.y + 6 < area.height {
        let filled = ((progress * bar_w as f32) as usize).min(bar_w);
        let empty = bar_w.saturating_sub(filled);

        let mut bar = String::with_capacity(bar_w * 3 + 2);
        bar.push('[');
        for _ in 0..filled {
            bar.push('\u{2588}'); // █
        }
        for _ in 0..empty {
            bar.push('\u{2591}'); // ░
        }
        bar.push(']');

        let bar_x = inner.x + 2;
        let bar_color = if done { theme.diff_add } else { theme.fg_accent };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default().fg(bar_color),
            ))),
            Rect::new(bar_x, inner.y + 6, inner.width.saturating_sub(2), 1),
        );
    }

    // ── Porcentaje ──
    if inner.y + 7 < area.height {
        let pct = (progress * 100.0) as u8;
        let mut pct_str = String::with_capacity(8);
        use std::fmt::Write;
        let _ = write!(pct_str, "{}%", pct);
        let pct_len = pct_str.len() as u16;
        let pct_x = inner.x + inner.width.saturating_sub(pct_len) / 2;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pct_str,
                Style::default().fg(theme.fg_secondary),
            ))),
            Rect::new(pct_x, inner.y + 7, pct_len, 1),
        );
    }
}

/// Determina qué sub-panel de la sidebar está activo según el foco.
///
/// Si el foco está en Explorer/Git/Search, ese es el panel activo.
/// En cualquier otro caso, default a Explorer.
fn sidebar_active_panel(focused: PanelId) -> PanelId {
    match focused {
        PanelId::Explorer | PanelId::Git | PanelId::Search | PanelId::Projects => focused,
        _ => PanelId::Explorer,
    }
}
