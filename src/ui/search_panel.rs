//! Search Panel: renderizado del panel de búsqueda global en la sidebar.
//!
//! Se muestra cuando `SearchState::visible` es `true`, reemplazando el explorer
//! en la sidebar. Layout: campos de input arriba, resultados agrupados por archivo
//! abajo, resumen al fondo.
//!
//! Reglas de render:
//! - Sin `format!()` dentro de loops
//! - Sin allocaciones innecesarias
//! - Viewport virtual para resultados (solo renderiza visibles)
//! - Todos los datos llegan pre-computados desde `SearchState`

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::search::{SearchField, SearchState};
use crate::ui::theme::Theme;

/// Renderiza el panel de búsqueda global dentro de la sidebar.
pub fn render_search_panel(
    f: &mut Frame,
    area: Rect,
    state: &SearchState,
    theme: &Theme,
    focused: bool,
) {
    // Bloque exterior con estilo de foco
    let (border_color, border_type, title_style) = if focused {
        (
            theme.border_focused,
            BorderType::Double,
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            theme.border_unfocused,
            BorderType::Plain,
            Style::default().fg(theme.fg_secondary),
        )
    };

    let block = Block::default()
        .title(Line::from(Span::styled("SEARCH", title_style)))
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Calcular cuántas líneas de input necesitamos
    let input_lines: u16 = if state.replace_visible { 4 } else { 3 };
    // Mínimo 1 línea para resultados + 1 para resumen
    let min_results_height: u16 = 2;

    if inner.height < input_lines + min_results_height {
        // Espacio insuficiente — solo mostrar inputs
        render_input_fields(f, inner, state, theme);
        return;
    }

    // Layout: inputs + resultados + summary (1 línea)
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(input_lines), // campos de input
            Constraint::Fill(1),             // resultados
            Constraint::Length(1),           // resumen
        ])
        .split(inner);

    render_input_fields(f, layout[0], state, theme);
    render_results(f, layout[1], state, theme);
    render_summary(f, layout[2], state, theme);
}

/// Renderiza los campos de input (query, replace, include, exclude).
fn render_input_fields(f: &mut Frame, area: Rect, state: &SearchState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    let mut constraints = Vec::with_capacity(4);
    constraints.push(Constraint::Length(1)); // query
    if state.replace_visible {
        constraints.push(Constraint::Length(1)); // replace
    }
    constraints.push(Constraint::Length(1)); // include
    constraints.push(Constraint::Length(1)); // exclude

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;

    // Query line con toggles
    render_query_line(f, layout[idx], state, theme);
    idx += 1;

    // Replace (si visible)
    if state.replace_visible && idx < layout.len() {
        render_field_line(
            f,
            layout[idx],
            theme,
            "\u{21d4} ", // ⇔
            &state.replace_text,
            state.active_field == SearchField::Replace,
        );
        idx += 1;
    }

    // Include
    if idx < layout.len() {
        render_field_line(
            f,
            layout[idx],
            theme,
            "\u{1f4c1} ", // 📁
            &state.options.include_pattern,
            state.active_field == SearchField::Include,
        );
        idx += 1;
    }

    // Exclude
    if idx < layout.len() {
        render_field_line(
            f,
            layout[idx],
            theme,
            "\u{1f6ab} ", // 🚫
            &state.options.exclude_pattern,
            state.active_field == SearchField::Exclude,
        );
    }
}

/// Renderiza la línea de query con los toggles de Aa, Ab, .*
fn render_query_line(f: &mut Frame, area: Rect, state: &SearchState, theme: &Theme) {
    let is_active = state.active_field == SearchField::Query;

    // Toggle indicators
    let aa_style = toggle_style(state.options.case_sensitive, theme);
    let ab_style = toggle_style(state.options.whole_word, theme);
    let re_style = toggle_style(state.options.use_regex, theme);

    let cursor = if is_active { "_" } else { "" };
    let cursor_style = Style::default()
        .fg(theme.fg_accent)
        .bg(theme.bg_secondary)
        .add_modifier(Modifier::SLOW_BLINK);

    let query_style = if is_active {
        Style::default().fg(theme.fg_primary).bg(theme.bg_secondary)
    } else {
        Style::default()
            .fg(theme.fg_secondary)
            .bg(theme.bg_secondary)
    };

    let icon_style = Style::default().fg(theme.fg_accent).bg(theme.bg_secondary);

    let line = Line::from(vec![
        Span::styled("\u{1f50d} ", icon_style), // 🔍
        Span::styled(state.options.query.as_str(), query_style),
        Span::styled(cursor, cursor_style),
        Span::styled(" ", Style::default().bg(theme.bg_secondary)),
        Span::styled("Aa", aa_style),
        Span::styled(" ", Style::default().bg(theme.bg_secondary)),
        Span::styled("Ab", ab_style),
        Span::styled(" ", Style::default().bg(theme.bg_secondary)),
        Span::styled(".*", re_style),
    ]);

    let p = Paragraph::new(line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza una línea de campo genérica (replace, include, exclude).
fn render_field_line(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    icon: &str,
    text: &str,
    is_active: bool,
) {
    let cursor = if is_active { "_" } else { "" };
    let cursor_style = Style::default()
        .fg(theme.fg_accent)
        .bg(theme.bg_secondary)
        .add_modifier(Modifier::SLOW_BLINK);

    let text_style = if is_active {
        Style::default().fg(theme.fg_primary).bg(theme.bg_secondary)
    } else {
        Style::default()
            .fg(theme.fg_secondary)
            .bg(theme.bg_secondary)
    };

    let icon_style = Style::default()
        .fg(theme.fg_secondary)
        .bg(theme.bg_secondary);

    let line = Line::from(vec![
        Span::styled(icon, icon_style),
        Span::styled(text, text_style),
        Span::styled(cursor, cursor_style),
    ]);

    let p = Paragraph::new(line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza la lista de resultados con viewport virtual.
fn render_results(f: &mut Frame, area: Rect, state: &SearchState, theme: &Theme) {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return;
    }

    let Some(ref results) = state.results else {
        let p = Paragraph::new(Line::from(Span::styled(
            "  Press Enter to search",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(p, area);
        return;
    };

    if results.matches.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "  No results found",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(p, area);
        return;
    }

    // Construir display lines: file headers + match lines
    let display = build_display_lines(results, state.selected_match, area.width as usize, theme);

    // Viewport virtual: solo las líneas visibles
    let total_lines = display.len();
    let scroll = state.scroll_offset.min(total_lines.saturating_sub(1));
    let visible: Vec<Line<'_>> = display
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    let p = Paragraph::new(visible).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Construye las líneas de display para los resultados.
///
/// Agrupa matches por archivo con un header por cada archivo.
fn build_display_lines<'a>(
    results: &crate::search::engine::SearchResults,
    selected_idx: usize,
    max_width: usize,
    theme: &'a Theme,
) -> Vec<Line<'a>> {
    let mut lines = Vec::with_capacity(results.matches.len() + results.files_matched);
    let mut current_file: Option<&std::path::Path> = None;

    for (i, m) in results.matches.iter().enumerate() {
        let is_new_file = current_file != Some(m.path.as_path());

        if is_new_file {
            current_file = Some(&m.path);

            // Contar matches de este archivo
            let file_match_count = results
                .matches
                .iter()
                .filter(|mm| mm.path == m.path)
                .count();

            // Header del archivo
            let path_display = m.path.to_string_lossy();
            let header_style = Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD);
            let count_style = Style::default()
                .fg(theme.fg_secondary)
                .bg(theme.bg_secondary);

            let count_str = format_match_count(file_match_count);

            lines.push(Line::from(vec![
                Span::styled(path_display.into_owned(), header_style),
                Span::styled(" ", Style::default().bg(theme.bg_secondary)),
                Span::styled(count_str, count_style),
            ]));
        }

        // Línea del match
        let is_selected = i == selected_idx;
        lines.push(render_match_line(m, is_selected, max_width, theme));
    }

    lines
}

/// Pre-computa el string de count de matches para un file header.
fn format_match_count(count: usize) -> String {
    let mut s = String::with_capacity(16);
    s.push('(');
    use std::fmt::Write;
    let _ = write!(s, "{count}");
    if count == 1 {
        s.push_str(" match)");
    } else {
        s.push_str(" matches)");
    }
    s
}

/// Renderiza una línea de match individual.
fn render_match_line<'a>(
    m: &crate::search::engine::SearchMatch,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    let indicator = if selected { " \u{25b8}" } else { "  " };
    let indicator_style = Style::default()
        .fg(theme.fg_accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    // Truncar contenido de línea para el ancho disponible — char-safe para multi-byte
    let prefix_len = indicator.len() + 1; // indicator + 1 espacio
    let content_max = max_width.saturating_sub(prefix_len);
    let display_content = if m.line_content.chars().count() > content_max {
        let truncated = crate::ui::truncate_str(&m.line_content, content_max.saturating_sub(3));
        format!("{truncated}...")
    } else {
        m.line_content.clone() // CLONE: necesario — Span toma ownership del String
    };

    let content_style = Style::default().fg(theme.fg_primary).bg(bg);

    // Si el match cabe en lo visible, resaltarlo
    if m.match_start < display_content.len() && m.match_end <= m.line_content.len() {
        let match_end_clamped = m.match_end.min(display_content.len());
        if m.match_start < match_end_clamped {
            let before = &display_content[..m.match_start];
            let matched = &display_content[m.match_start..match_end_clamped];
            let after = &display_content[match_end_clamped..];

            let match_style = Style::default()
                .fg(theme.fg_primary)
                .bg(theme.search_match)
                .add_modifier(Modifier::BOLD);

            return Line::from(vec![
                Span::styled(indicator, indicator_style),
                Span::styled(" ", content_style),
                Span::styled(before.to_string(), content_style),
                Span::styled(matched.to_string(), match_style),
                Span::styled(after.to_string(), content_style),
            ]);
        }
    }

    // Fallback: sin highlight de match
    Line::from(vec![
        Span::styled(indicator, indicator_style),
        Span::styled(" ", content_style),
        Span::styled(display_content, content_style),
    ])
}

/// Renderiza la línea de resumen al fondo.
fn render_summary(f: &mut Frame, area: Rect, state: &SearchState, theme: &Theme) {
    let summary = if let Some(ref results) = state.results {
        let mut s = String::with_capacity(48);
        use std::fmt::Write;
        let _ = write!(
            s,
            " {} results in {} files",
            results.total_matches, results.files_matched
        );
        if results.truncated {
            s.push_str(" (truncated)");
        }
        s
    } else {
        String::from(" Ctrl+Shift+F to search")
    };

    let line = Line::from(Span::styled(
        summary,
        Style::default().fg(theme.fg_secondary),
    ));
    let p = Paragraph::new(line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Estilo para un toggle indicator (activo vs inactivo).
fn toggle_style(active: bool, theme: &Theme) -> Style {
    if active {
        Style::default()
            .fg(theme.bg_primary)
            .bg(theme.fg_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.fg_secondary)
            .bg(theme.bg_secondary)
    }
}
