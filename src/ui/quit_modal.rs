//! Quit modal: overlay de confirmación al cerrar la app con buffers dirty.
//!
//! Stateless renderer — recibe datos pre-computados y dibuja.
//! Sin allocaciones en el render: el `dirty_count` se pre-formatea fuera
//! con un buffer de tamaño conocido (ver `format_dirty_count`).

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::ui::theme::Theme;
use crate::workspace::quit_modal::QuitModalState;

/// Dimensiones fijas del modal — cabe entero salvo en terminales muy chicos.
const MODAL_WIDTH: u16 = 60;
const MODAL_HEIGHT: u16 = 7;

/// Renderiza el modal de confirmación de quit centrado sobre `area`.
///
/// `dirty_count` se computa fuera (count de buffers dirty) — el render solo
/// formatea el número y dibuja. Tres botones horizontales: Save All, Don't
/// Save, Cancel. El botón con foco se resalta con `theme.fg_accent` de fondo.
pub fn render_quit_modal(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    state: &QuitModalState,
    dirty_count: usize,
) {
    // Centrar el modal sobre el área total. Si la terminal es más chica
    // que el modal, clampear al área disponible.
    let modal_w = MODAL_WIDTH.min(area.width);
    let modal_h = MODAL_HEIGHT.min(area.height);
    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal_area = Rect::new(x, y, modal_w, modal_h);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " \u{26A0} ", // ⚠
                Style::default()
                    .fg(theme.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Save changes before quitting? ",
                Style::default().fg(theme.fg_primary),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    if inner.height < 3 || inner.width < 20 {
        return;
    }

    // ── Body: "{N} unsaved file(s)" ──
    // Pre-formatear con buffer de tamaño conocido — sin format!() y con
    // capacidad pre-allocada. dirty_count máximo razonable < 1e6 ⇒ 24 bytes.
    let mut body = String::with_capacity(32);
    use std::fmt::Write;
    let _ = if dirty_count == 1 {
        write!(body, "{} unsaved file", dirty_count)
    } else {
        write!(body, "{} unsaved files", dirty_count)
    };
    let body_area = Rect::new(inner.x, inner.y, inner.width, 1);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            body,
            Style::default()
                .fg(theme.fg_primary)
                .bg(theme.bg_secondary),
        )))
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().bg(theme.bg_secondary)),
        body_area,
    );

    // ── Botones ──
    // 3 botones centrados en una sola fila. Renderizamos manualmente para
    // tener control fino sobre el highlight del botón con foco.
    let labels: [&str; 3] = ["[ Save All ]", "[ Don't Save ]", "[ Cancel ]"];
    // Anchos de cada label en chars (== bytes para ASCII).
    let widths: [u16; 3] = [
        labels[0].len() as u16,
        labels[1].len() as u16,
        labels[2].len() as u16,
    ];
    let total_buttons_w: u16 = widths[0] + widths[1] + widths[2] + 4; // 2 espacios entre cada par
    if total_buttons_w > inner.width {
        // Terminal demasiado angosta — no renderizamos los botones, solo body+footer
        return;
    }
    let buttons_y = inner.y + 2;
    if buttons_y >= inner.y + inner.height {
        return;
    }
    let mut x = inner.x + inner.width.saturating_sub(total_buttons_w) / 2;
    for (i, label) in labels.iter().enumerate() {
        let is_focused = state.focused_button == i;
        let style = if is_focused {
            Style::default()
                .fg(theme.bg_secondary)
                .bg(theme.fg_accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.fg_primary)
                .bg(theme.bg_secondary)
        };
        let btn_area = Rect::new(x, buttons_y, widths[i], 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(*label, style)))
                .style(Style::default().bg(theme.bg_secondary)),
            btn_area,
        );
        x += widths[i] + 2; // +2 = separador entre botones
    }

    // ── Footer: hints de teclado ──
    let footer_y = inner.y + 4;
    if footer_y < inner.y + inner.height {
        let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [Tab] Cycle  [Enter] Confirm  [S] Save All  [D] Discard  [Esc] Cancel",
                Style::default()
                    .fg(theme.fg_secondary)
                    .bg(theme.bg_secondary),
            )))
            .style(Style::default().bg(theme.bg_secondary)),
            footer_area,
        );
    }
}
