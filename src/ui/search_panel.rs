//! Search Panel: renderizado del panel de búsqueda global en la sidebar.
//!
//! Se muestra cuando `SearchState::visible` es `true`, reemplazando el explorer
//! en la sidebar. Layout: campos de input arriba, resultados agrupados por archivo
//! abajo, resumen al fondo.
//!
//! Resultados estilo VS Code:
//! - File headers con icono, nombre bold, directorio dimmed, badge de count
//! - Fold/unfold de file groups (▾/▸)
//! - Match lines indentadas con highlight del término buscado
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

use crate::search::{FlatSearchItem, SearchField, SearchState};
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

    let cursor = if is_active { "|" } else { "" };
    let cursor_style = Style::default().fg(theme.fg_accent).bg(theme.bg_secondary);

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
    let cursor = if is_active { "|" } else { "" };
    let cursor_style = Style::default().fg(theme.fg_accent).bg(theme.bg_secondary);

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

/// Renderiza la lista de resultados agrupados con viewport virtual.
///
/// Usa la lista aplanada `state.flat_items` para renderizar:
/// - FileHeader: `▾/▸ Icon filename  dir  count_badge`
/// - MatchLine: `    contenido con highlight del match`
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

    // Viewport virtual sobre la lista aplanada
    let total = state.flat_items.len();
    let scroll = state.scroll_offset.min(total.saturating_sub(1));
    let max_width = area.width as usize;

    // Buffer para el badge de count — reutilizado entre file headers
    let mut count_buf = String::with_capacity(8);

    let lines: Vec<Line<'_>> = state
        .flat_items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(flat_idx, item)| {
            let is_selected = flat_idx == state.selected_flat_index;
            match *item {
                FlatSearchItem::FileHeader { group_index } => render_file_header(
                    &state.file_groups[group_index],
                    state.is_collapsed(group_index),
                    is_selected,
                    max_width,
                    theme,
                    &mut count_buf,
                ),
                FlatSearchItem::MatchLine { match_index, .. } => {
                    render_match_line(&results.matches[match_index], is_selected, max_width, theme)
                }
            }
        })
        .collect();

    let p = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza un file header en la lista de resultados.
///
/// Layout: `▾ Icon filename  dir  badge`
/// - `▾`/`▸`: indicador de fold (1 char)
/// - Icono de archivo por extensión (2 chars) con color semántico
/// - Filename en `fg_primary` + bold
/// - Directorio en `fg_secondary` (dimmed)
/// - Badge con count de matches en `fg_accent`
fn render_file_header<'a>(
    group: &crate::search::FileGroup,
    collapsed: bool,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
    count_buf: &mut String,
) -> Line<'a> {
    use crate::ui::icons;

    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    // Fold indicator
    let fold_char = if collapsed { "\u{25B8} " } else { "\u{25BE} " };
    let fold_style = Style::default().fg(theme.fg_secondary).bg(bg);

    // File icon por extensión
    let icon = icons::file_icon(&group.filename);
    let icon_color = icons::icon_color(&group.filename);
    let icon_style = Style::default().fg(icon_color).bg(bg);

    // Filename bold
    let name_style = Style::default()
        .fg(theme.fg_primary)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    // Directorio dimmed
    let dir_style = Style::default().fg(theme.fg_secondary).bg(bg);

    // Badge de count — reutilizar buffer, sin format!()
    count_buf.clear();
    {
        use std::fmt::Write;
        let _ = write!(count_buf, "{}", group.match_count);
    }

    let badge_style = Style::default().fg(theme.fg_accent).bg(bg);
    let space_style = Style::default().bg(bg);

    // Construir spans — capacidad fija, sin realloc
    let mut spans = Vec::with_capacity(8);
    spans.push(Span::styled(fold_char, fold_style));
    spans.push(Span::styled(icon, icon_style));
    spans.push(Span::styled(" ", space_style));

    // Truncar filename si es necesario — reservar espacio para dir y badge
    // fold(2) + icon(2) + space(1) + name + space(2) + dir + space(2) + badge
    let overhead = 2 + 2 + 1 + 2 + 2 + count_buf.len();
    let dir_len = group
        .dir
        .len()
        .min(max_width.saturating_sub(overhead + group.filename.len()));
    let name_max = max_width.saturating_sub(overhead + dir_len);
    let display_name = crate::ui::truncate_str(&group.filename, name_max);
    // CLONE: necesario — display_name es slice de group.filename, Span toma ownership
    spans.push(Span::styled(display_name.to_string(), name_style));

    // Directorio (solo si hay espacio y no está vacío)
    if !group.dir.is_empty() && dir_len > 0 {
        spans.push(Span::styled("  ", space_style));
        let display_dir = crate::ui::truncate_str(&group.dir, dir_len);
        // CLONE: necesario — display_dir es slice de group.dir
        spans.push(Span::styled(display_dir.to_string(), dir_style));
    }

    // Badge de count
    spans.push(Span::styled("  ", space_style));
    // CLONE: necesario — count_buf se reutiliza entre headers
    spans.push(Span::styled(count_buf.clone(), badge_style));

    Line::from(spans)
}

/// Renderiza una línea de match individual (indentada, con highlight).
///
/// Layout: `    contenido_con_match_resaltado`
/// - 4 espacios de indentación
/// - Contenido de la línea truncado al ancho disponible
/// - Match resaltado con `search_match` background color
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

    // Indentación de 4 espacios
    let indent = "    ";
    let indent_style = Style::default().bg(bg);

    // Truncar contenido de línea para el ancho disponible — char-safe para multi-byte
    let content_max = max_width.saturating_sub(indent.len());
    let trimmed_content = m.line_content.trim();
    let display_content = crate::ui::truncate_str(trimmed_content, content_max);

    let content_style = Style::default().fg(theme.fg_primary).bg(bg);

    // Calcular offset del match dentro del contenido trimmed
    let trim_offset = m.line_content.find(trimmed_content).unwrap_or(0);
    let adj_start = m.match_start.saturating_sub(trim_offset);
    let adj_end = m.match_end.saturating_sub(trim_offset);

    // Si el match cabe en lo visible, resaltarlo
    if adj_start < display_content.len() && adj_end <= trimmed_content.len() {
        let match_end_clamped = adj_end.min(display_content.len());
        if adj_start < match_end_clamped {
            let before = &display_content[..adj_start];
            let matched = &display_content[adj_start..match_end_clamped];
            let after = &display_content[match_end_clamped..];

            let match_style = Style::default()
                .fg(theme.fg_primary)
                .bg(theme.search_match)
                .add_modifier(Modifier::BOLD);

            return Line::from(vec![
                Span::styled(indent, indent_style),
                // CLONE: necesario — slices del display_content, Span toma ownership
                Span::styled(before.to_string(), content_style),
                Span::styled(matched.to_string(), match_style),
                Span::styled(after.to_string(), content_style),
            ]);
        }
    }

    // Fallback: sin highlight de match
    Line::from(vec![
        Span::styled(indent, indent_style),
        // CLONE: necesario — display_content es slice, Span toma ownership
        Span::styled(display_content.to_string(), content_style),
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
