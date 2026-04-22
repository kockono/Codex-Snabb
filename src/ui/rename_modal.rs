//! Rename modal: overlay de input de nombre para renombrar archivos en el explorer.
//!
//! Stateless renderer — recibe datos pre-computados y dibuja.
//! Sin allocaciones en el render excepto los strings pre-construidos
//! que se pasan ya formados.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::ui::layout::{modal_rect, IdeLayout};
use crate::ui::theme::Theme;
use crate::workspace::rename::RenameState;

/// Renderiza el modal "Rename".
///
/// Pre-computar TODOS los strings antes del render (nunca `format!()` aquí).
/// Recibe `cursor_visible` para el efecto blink del cursor en el input.
pub fn render_rename_modal(
    f: &mut Frame,
    layout: &IdeLayout,
    state: &RenameState,
    theme: &Theme,
    cursor_visible: bool,
) {
    // Altura del modal: 1 borde + 1 input + 1 error (condicional)
    //                   + 1 footer + 1 borde = 5 sin error, 6 con error.
    // Con padding interno queda 7 sin error / 8 con error (mismo que save_as).
    let has_error = state.error.is_some();
    let modal_height: u16 = if has_error { 8 } else { 7 };

    let overlay_rect = modal_rect(layout, modal_height);
    f.render_widget(Clear, overlay_rect);

    // ── Bloque exterior ──
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " \u{270F} ", // ✏
                Style::default()
                    .fg(theme.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Rename ", Style::default().fg(theme.fg_primary)),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_rect);
    f.render_widget(block, overlay_rect);

    if inner.height < 3 {
        return;
    }

    // ── Pre-computar textos (NUNCA format!() en render) ──

    // Input: usar placeholder si está vacío, o el texto escrito
    let is_placeholder = state.input.is_empty();
    let input_text: &str = if is_placeholder {
        "Nuevo nombre..."
    } else {
        state.input.as_str()
    };

    // Cursor blink: "|" si visible, "" si no — NO allocar en render
    let cursor_indicator: &str = if cursor_visible && !is_placeholder {
        "|"
    } else {
        ""
    };

    let input_style = if is_placeholder {
        Style::default()
            .fg(theme.fg_secondary)
            .bg(theme.bg_active)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default()
            .fg(theme.fg_primary)
            .bg(theme.bg_active)
            .add_modifier(Modifier::BOLD)
    };

    // ── Fila 0: input de nombre ──
    let input_area = ratatui::layout::Rect::new(inner.x, inner.y, inner.width, 1);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Nombre: ",
                Style::default().fg(theme.fg_accent).bg(theme.bg_active),
            ),
            Span::styled(input_text, input_style),
            Span::styled(
                cursor_indicator,
                Style::default().fg(theme.fg_accent).bg(theme.bg_active),
            ),
        ]))
        .style(Style::default().bg(theme.bg_active)),
        input_area,
    );

    // ── Fila 1 (condicional): error efímero ──
    let error_row_offset: u16 = if has_error { 1 } else { 0 };
    if has_error {
        let err_msg = state.error.as_deref().unwrap_or("");
        let error_area = ratatui::layout::Rect::new(inner.x, inner.y + 1, inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("   ", Style::default().bg(theme.bg_secondary)),
                Span::styled(
                    "\u{2717} ", // ✗
                    Style::default()
                        .fg(theme.diff_remove)
                        .bg(theme.bg_secondary),
                ),
                Span::styled(
                    err_msg,
                    Style::default()
                        .fg(theme.diff_remove)
                        .bg(theme.bg_secondary),
                ),
            ]))
            .style(Style::default().bg(theme.bg_secondary)),
            error_area,
        );
    }

    // ── Footer: instrucciones ──
    let footer_y = inner.y + 1 + error_row_offset;
    if footer_y < inner.y + inner.height {
        let footer_area = ratatui::layout::Rect::new(inner.x, footer_y, inner.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [Enter] Renombrar   [Esc] Cancelar",
                Style::default().fg(theme.fg_secondary),
            )))
            .style(Style::default().bg(theme.bg_active)),
            footer_area,
        );
    }
}
