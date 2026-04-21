//! Branch Picker: overlay centrado para selección de rama git.
//!
//! Similar al Quick Open pero específico para ramas.
//! Muestra un input de búsqueda arriba y la lista de ramas debajo.
//! La rama actual se resalta en verde con `*`, las remotas en dimmed.
//!
//! El filtrado se hace en `BranchPicker::update_filter()` — NUNCA en render.
//! El render solo dibuja desde el cache de `filtered`.

use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::git::branch_picker::{BranchPicker, MAX_VISIBLE_ITEMS};
use crate::git::commands::BranchInfo;
use crate::ui::layout::{self, IdeLayout};
use crate::ui::theme::Theme;

// ─── Render ────────────────────────────────────────────────────────────────────

/// Renderiza el branch picker como overlay centrado.
///
/// Overlay centrado: ~50% ancho, max `MAX_VISIBLE_ITEMS` items de alto.
/// Input field arriba con icono de branch, lista de ramas debajo.
/// Rama actual: verde con `*`. Remotas: texto dimmed. Seleccionada: highlight.
///
/// Precondición: `state.visible == true`.
/// NO aloca `format!()` dentro del loop de items.
pub fn render_branch_picker(
    f: &mut Frame,
    layout: &IdeLayout,
    state: &BranchPicker,
    theme: &Theme,
) {
    if !state.visible {
        return;
    }

    // ── Calcular área del overlay via modal_rect ──
    let visible_items = state.filtered.len().min(MAX_VISIBLE_ITEMS);
    let modal_height = (visible_items as u16 + 5).max(6);
    let overlay_rect = layout::modal_rect(layout, modal_height);

    // ── Limpiar el área del overlay ──
    f.render_widget(Clear, overlay_rect);

    // ── Bloque exterior con borde accent ──
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Switch Branch ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_rect);
    f.render_widget(block, overlay_rect);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Layout interno: input (1 línea) + lista (resto) + footer (1) ──
    let inner_layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // input
            Constraint::Fill(1),   // lista de ramas
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let input_area = inner_layout[0];
    let list_area = inner_layout[1];
    let footer_area = inner_layout[2];

    // ── Render input field ──
    let input_line = Line::from(vec![
        Span::styled(
            "\u{e0a0} ", // nerd font branch icon
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(state.input.as_str(), Style::default().fg(theme.fg_primary)),
        Span::styled(
            "_",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    let input_paragraph = Paragraph::new(input_line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(input_paragraph, input_area);

    // ── Render lista de ramas ──
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
        .map(|(i, &branch_idx)| {
            let branch = &state.branches[branch_idx];
            let is_selected = state.scroll_offset + i == state.selected_index;
            render_branch_item(branch, is_selected, list_area.width as usize, theme)
        })
        .collect();

    let list_paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(list_paragraph, list_area);

    // ── Footer con atajos ──
    let footer = Paragraph::new(Line::from(Span::styled(
        " [\u{2191}\u{2193}] Navegar   [Enter] Checkout   [Esc] Cerrar",
        Style::default().fg(theme.fg_secondary),
    )))
    .alignment(Alignment::Left)
    .style(Style::default().bg(theme.bg_active));
    f.render_widget(footer, footer_area);
}

/// Renderiza un item de rama como una `Line` de ratatui.
///
/// Formato:
/// - Rama actual: `  * main` (verde, bold)
/// - Rama remota: `    remotes/origin/main` (dimmed)
/// - Rama local:  `    feature/editor` (texto normal)
/// - Seleccionada: `  ▸ branch-name` (highlight bg)
///
/// No usa `format!()` — construye spans directamente.
fn render_branch_item<'a>(
    branch: &BranchInfo,
    selected: bool,
    _max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    // Indicador: seleccionado `▸`, actual `*`, normal espacio
    let (indicator, indicator_style) = if selected {
        (
            " \u{25b8} ",
            Style::default()
                .fg(theme.fg_accent)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )
    } else if branch.is_current {
        (
            " * ",
            Style::default()
                .fg(theme.diff_add)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("   ", Style::default().fg(theme.fg_secondary).bg(bg))
    };

    // Estilo del nombre según tipo de rama
    let name_style = if branch.is_current {
        Style::default()
            .fg(theme.diff_add)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else if branch.is_remote {
        Style::default().fg(theme.fg_secondary).bg(bg)
    } else if selected {
        Style::default()
            .fg(theme.fg_primary)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_primary).bg(bg)
    };

    // CLONE: necesario — branch.name es String en BranchInfo,
    // Span::styled necesita ownership para display
    Line::from(vec![
        Span::styled(indicator, indicator_style),
        Span::styled(branch.name.clone(), name_style),
    ])
}
