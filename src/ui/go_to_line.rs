//! Go to Line: modal pequeño para saltar a una línea específica.
//!
//! Overlay centrado (~30% ancho, 5 líneas de alto) que muestra
//! un input de dígitos con hint del rango válido. Sin allocaciones
//! en el render — el hint se pasa pre-formateado desde `ui/mod.rs`.

use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::ui::layout::{self, IdeLayout};
use crate::ui::theme::Theme;
use crate::workspace::quick_open::GoToLineState;

/// Renderiza el modal Go to Line como overlay centrado.
///
/// El `hint` (ej: "1 – 420") se pasa pre-formateado desde fuera
/// para evitar `format!()` dentro del render. El modal se dibuja
/// DESPUÉS de otros overlays (máxima prioridad visual).
///
/// Precondición: `state.visible == true`.
pub fn render_go_to_line(
    f: &mut Frame,
    layout: &IdeLayout,
    state: &GoToLineState,
    theme: &Theme,
    hint: &str,
) {
    if !state.visible {
        return;
    }

    // Go to Line: fixed 5 lines tall, positioned via modal_rect
    let overlay_rect = layout::modal_rect(layout, 5);

    f.render_widget(Clear, overlay_rect);

    let block = Block::default()
        .title(Line::from(Span::styled(
            " Go to Line ",
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

    if inner.height < 2 {
        return;
    }

    // Layout: input(1) + spacer(fill) + footer(1)
    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // ── Input line: " :42_   1 – 420" ──
    // Left side: prefix + digits + cursor blink
    // Right side: hint (dimmed)
    let input_spans = Line::from(vec![
        Span::styled(
            " :",
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
        // Spacing: fill the gap between input and hint
        // Use a simple space padding — the hint goes on the right
        Span::styled("  ", Style::default()),
        Span::styled(hint, Style::default().fg(theme.fg_secondary)),
    ]);

    f.render_widget(
        Paragraph::new(input_spans).style(Style::default().bg(theme.bg_secondary)),
        sections[0],
    );

    // ── Footer ──
    let footer = Paragraph::new(Line::from(Span::styled(
        " [Enter] Ir   [Esc] Cancelar",
        Style::default().fg(theme.fg_secondary),
    )))
    .alignment(Alignment::Left)
    .style(Style::default().bg(theme.bg_active));
    f.render_widget(footer, sections[2]);
}
