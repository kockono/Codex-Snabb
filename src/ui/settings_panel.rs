//! Settings panel: overlay de edición de keybindings.
//!
//! Renderiza un overlay centrado (80% × 70%) con:
//! - Campo de búsqueda
//! - Tabla de dos columnas: Command | Keybinding
//! - Selección con highlight, modo edición con "Press key..."
//! - Footer con instrucciones contextuales
//!
//! Stateless renderer — recibe datos pre-computados, no aloca en render.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::settings::KeybindingsState;
use crate::ui::theme::Theme;

/// Renderiza el overlay de settings (keybindings editor).
///
/// Overlay centrado al 80% × 70% del área total. Incluye:
/// - Header con campo de búsqueda
/// - Tabla scrollable de Command | Keybinding
/// - Footer con atajos contextuales
///
/// No aloca strings en el render — usa referencias y literales.
pub fn render_settings(f: &mut Frame, area: Rect, state: &KeybindingsState, theme: &Theme) {
    if !state.visible {
        return;
    }

    // ── Calcular dimensiones del overlay (80% × 70%) ──
    let overlay_width = (area.width * 80 / 100).min(area.width).max(40);
    let overlay_height = (area.height * 70 / 100).min(area.height).max(10);
    let overlay_x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let overlay_y = area.y + (area.height.saturating_sub(overlay_height)) / 2;

    let overlay_area = Rect::new(overlay_x, overlay_y, overlay_width, overlay_height);

    // Clear el área bajo el overlay
    f.render_widget(Clear, overlay_area);

    // ── Bloque principal con borde ──
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Keybindings ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_area);
    f.render_widget(block, overlay_area);

    if inner.height < 4 || inner.width < 20 {
        return; // Demasiado pequeño para renderizar
    }

    // ── Layout vertical interno: search(1) + separator(1) + table(fill) + footer(1) ──
    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // búsqueda
            Constraint::Length(1), // separador
            Constraint::Length(1), // header de tabla
            Constraint::Fill(1),   // tabla de keybindings
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let search_area = sections[0];
    let separator_area = sections[1];
    let header_area = sections[2];
    let table_area = sections[3];
    let footer_area = sections[4];

    // ── Búsqueda ──
    let search_text = if state.search_input.is_empty() {
        Span::styled(
            " \u{1F50D} search keybindings...",
            Style::default().fg(theme.fg_secondary),
        )
    } else {
        Span::styled(
            format!(" \u{1F50D} {}", state.search_input),
            Style::default().fg(theme.fg_primary),
        )
    };
    let search_line =
        Paragraph::new(Line::from(search_text)).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(search_line, search_area);

    // ── Separador ──
    let sep_width = separator_area.width as usize;
    // Usar truncate_str para corte char-safe en string multi-byte
    const DASHES: &str = "────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────";
    let sep_str = crate::ui::truncate_str(DASHES, sep_width);
    let separator = Paragraph::new(Line::from(Span::styled(
        sep_str,
        Style::default().fg(theme.border_unfocused),
    )))
    .style(Style::default().bg(theme.bg_secondary));
    f.render_widget(separator, separator_area);

    // ── Header de tabla ──
    let cmd_col_width = (header_area.width as usize) / 2;
    let kb_col_width = (header_area.width as usize).saturating_sub(cmd_col_width);

    let header_line = Line::from(vec![
        Span::styled(
            pad_right("  Command", cmd_col_width),
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            pad_right("Keybinding", kb_col_width),
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let header_para = Paragraph::new(header_line).style(Style::default().bg(theme.bg_active));
    f.render_widget(header_para, header_area);

    // ── Tabla de keybindings ──
    let visible_height = table_area.height as usize;

    if !state.filtered.is_empty() {
        // Viewport virtual: solo entries visibles
        let scroll = state.scroll_offset;
        let visible_entries = state.filtered.iter().skip(scroll).take(visible_height);

        let mut lines: Vec<Line<'_>> = Vec::with_capacity(visible_height);

        for (visual_idx, &entry_idx) in visible_entries.enumerate() {
            let is_selected = scroll + visual_idx == state.selected_index;
            let entry = &state.entries[entry_idx];

            let bg = if is_selected {
                theme.bg_active
            } else {
                theme.bg_secondary
            };

            let cmd_fg = if is_selected {
                theme.fg_accent
            } else {
                theme.fg_primary
            };

            // Formato: "Category: Label"
            let cmd_display = format!("{}: {}", entry.category, entry.command_label);
            let cmd_text = truncate_and_pad(&cmd_display, cmd_col_width);

            // Keybinding: modo edición vs display normal
            let kb_text = if state.editing_index == Some(entry_idx) {
                // Modo edición: "Press key combination..."
                pad_right("Press key...", kb_col_width)
            } else if entry.keybinding.is_empty() {
                // Sin keybinding
                let marker = if is_selected { "(none)  [+]" } else { "(none)" };
                pad_right(marker, kb_col_width)
            } else {
                // Keybinding normal con indicador de edición si seleccionado
                let display = if is_selected {
                    format!("{}  [\u{270E}]", entry.keybinding)
                } else {
                    entry.keybinding.clone() // CLONE: necesario — keybinding es String, necesitamos ownership para format condicional
                };
                truncate_and_pad(&display, kb_col_width)
            };

            // Estilo del keybinding
            let kb_style = if state.editing_index == Some(entry_idx) {
                Style::default()
                    .fg(theme.fg_accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)
            } else if entry.is_custom {
                Style::default().fg(theme.fg_accent_alt).bg(bg)
            } else if entry.keybinding.is_empty() {
                Style::default().fg(theme.fg_secondary).bg(bg)
            } else {
                Style::default().fg(theme.fg_primary).bg(bg)
            };

            // Indicador de selección
            let marker = if is_selected { "\u{25B8} " } else { "  " };

            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(theme.fg_accent).bg(bg)),
                Span::styled(cmd_text, Style::default().fg(cmd_fg).bg(bg)),
                Span::styled(kb_text, kb_style),
            ]));
        }

        let table_para = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
        f.render_widget(table_para, table_area);
    } else {
        // Sin resultados de búsqueda
        let no_results = Paragraph::new(Line::from(Span::styled(
            "  No matching commands",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(no_results, table_area);
    }

    // ── Footer con instrucciones contextuales ──
    let footer_text = if state.editing_index.is_some() {
        " Press key combination... | [Esc] Cancel"
    } else {
        " [Enter] Edit | [Delete] Remove | [Esc] Close"
    };

    let footer = Paragraph::new(Line::from(Span::styled(
        footer_text,
        Style::default().fg(theme.fg_secondary),
    )))
    .alignment(Alignment::Left)
    .style(Style::default().bg(theme.bg_active));
    f.render_widget(footer, footer_area);
}

/// Trunca un string a `width` chars y rellena con espacios a la derecha.
///
/// No aloca si el string ya tiene el tamaño correcto.
fn truncate_and_pad(s: &str, width: usize) -> String {
    let truncated = crate::ui::truncate_str(s, width);
    let char_count = truncated.chars().count();
    if char_count >= width {
        truncated.to_string()
    } else {
        let mut result = String::with_capacity(width);
        result.push_str(truncated);
        for _ in 0..(width - char_count) {
            result.push(' ');
        }
        result
    }
}

/// Rellena un `&str` con espacios a la derecha hasta `width`.
fn pad_right(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count >= width {
        s.to_string()
    } else {
        let mut result = String::with_capacity(width);
        result.push_str(s);
        for _ in 0..(width - char_count) {
            result.push(' ');
        }
        result
    }
}
