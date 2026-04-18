//! Quick Open: overlay centrado para búsqueda rápida de archivos (Ctrl+P).
//!
//! Similar a la command palette pero para archivos del workspace.
//! Muestra un input de búsqueda arriba y la lista de archivos filtrados abajo.
//! El filtrado se hace en `QuickOpenState::update_filter()` — NUNCA en render.
//! El render solo dibuja desde el cache de `filtered`.

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::ui::theme::Theme;
use crate::workspace::quick_open::{QuickOpenState, MAX_VISIBLE_ITEMS};

// ─── Render ────────────────────────────────────────────────────────────────────

/// Renderiza el Quick Open como overlay centrado.
///
/// Overlay centrado: ~60% ancho, max `MAX_VISIBLE_ITEMS` items de alto.
/// Input field arriba con icono de búsqueda, lista de paths relativos debajo.
/// Seleccionado con highlight. Borde magenta (accent_alt) para diferenciar
/// de la command palette (cyan/accent).
///
/// Precondición: `state.visible == true`.
/// NO aloca `format!()` dentro del loop de items — pre-computa antes.
pub fn render_quick_open(f: &mut Frame, area: Rect, state: &QuickOpenState, theme: &Theme) {
    if !state.visible {
        return;
    }

    // ── Calcular área del overlay ──
    // ~60% del ancho, centrado
    let overlay_width = (area.width * 60 / 100)
        .max(30)
        .min(area.width.saturating_sub(4));
    let x_offset = (area.width.saturating_sub(overlay_width)) / 2;

    // Alto: input(1) + borde arriba(1) + items + borde abajo(1)
    let visible_items = state.filtered.len().min(MAX_VISIBLE_ITEMS);
    // 3 = borde arriba + input line + borde abajo
    // +1 para el separador entre input y lista
    let overlay_height = (visible_items as u16 + 4).min(area.height.saturating_sub(4));
    let y_offset = area.height / 6; // ~1/6 desde arriba

    let overlay_rect = Rect::new(
        area.x + x_offset,
        area.y + y_offset,
        overlay_width,
        overlay_height,
    );

    // ── Limpiar el área del overlay ──
    f.render_widget(Clear, overlay_rect);

    // ── Bloque exterior con borde magenta (diferente a palette cyan) ──
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Quick Open ",
            Style::default()
                .fg(theme.fg_accent_alt)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent_alt))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_rect);
    f.render_widget(block, overlay_rect);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Layout interno: input (1 línea) + lista (resto) ──
    let inner_layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // input
            Constraint::Fill(1),   // lista de archivos
        ])
        .split(inner);

    let input_area = inner_layout[0];
    let list_area = inner_layout[1];

    // ── Render input field ──
    let input_line = Line::from(vec![
        Span::styled(
            "\u{1f50d} ",
            Style::default()
                .fg(theme.fg_accent_alt)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(state.input.as_str(), Style::default().fg(theme.fg_primary)),
        Span::styled(
            "_",
            Style::default()
                .fg(theme.fg_accent_alt)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    let input_paragraph = Paragraph::new(input_line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(input_paragraph, input_area);

    // ── Render lista de archivos ──
    if list_area.height == 0 {
        return;
    }

    let visible_count =
        (list_area.height as usize).min(state.filtered.len().saturating_sub(state.scroll_offset));

    // Pre-computar las líneas fuera del render — sin format!() en el loop
    let lines: Vec<Line<'_>> = state
        .filtered
        .iter()
        .skip(state.scroll_offset)
        .take(visible_count)
        .enumerate()
        .map(|(i, &file_idx)| {
            let path = &state.file_index[file_idx];
            let is_selected = state.scroll_offset + i == state.selected_index;
            render_file_item(path, is_selected, list_area.width as usize, theme)
        })
        .collect();

    let list_paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(list_paragraph, list_area);
}

/// Renderiza un item de archivo como una `Line` de ratatui.
///
/// Formato: `   path/to/file.rs` o ` ▸ path/to/file.rs` (seleccionado).
/// El item seleccionado usa `bg_active` como fondo.
/// No usa `format!()` — construye spans directamente.
fn render_file_item<'a>(
    path: &Path,
    selected: bool,
    _max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    let indicator = if selected { " \u{25b8} " } else { "   " };

    let indicator_style = Style::default()
        .fg(theme.fg_accent_alt)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    let path_style = if selected {
        Style::default()
            .fg(theme.fg_primary)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_primary).bg(bg)
    };

    // Convertir path a string — usamos to_string_lossy que retorna Cow.
    // Para Span necesitamos un owned String cuando Cow es Borrowed de un OsStr.
    // CLONE: necesario — Span::styled requiere ownership del string para display
    let path_display = path.to_string_lossy().into_owned();

    Line::from(vec![
        Span::styled(indicator, indicator_style),
        Span::styled(path_display, path_style),
    ])
}

use std::path::Path;
