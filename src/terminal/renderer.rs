//! Renderer: convierte el grid de `Term` a `ratatui::text::Line`.
//!
//! Pre-computa las líneas coloreadas FUERA del render_widget —
//! el resultado se pasa directamente a `Paragraph::new(lines)`.
//! Zero allocations durante el draw call — toda la computación
//! sucede antes de entrar al render pass de ratatui.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line as GridLine};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};
use alacritty_terminal::Term;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

// ─── Color Conversion ──────────────────────────────────────────────────────────

/// Convierte un `NamedColor` de alacritty a un `Color` de ratatui.
///
/// Mapeo directo sin allocación. Los named colors cubren los 16
/// colores ANSI estándar.
fn named_to_ratatui(nc: NamedColor) -> Color {
    match nc {
        NamedColor::Black => Color::Black,
        NamedColor::Red => Color::Red,
        NamedColor::Green => Color::Green,
        NamedColor::Yellow => Color::Yellow,
        NamedColor::Blue => Color::Blue,
        NamedColor::Magenta => Color::Magenta,
        NamedColor::Cyan => Color::Cyan,
        NamedColor::White => Color::White,
        NamedColor::BrightBlack => Color::DarkGray,
        NamedColor::BrightRed => Color::LightRed,
        NamedColor::BrightGreen => Color::LightGreen,
        NamedColor::BrightYellow => Color::LightYellow,
        NamedColor::BrightBlue => Color::LightBlue,
        NamedColor::BrightMagenta => Color::LightMagenta,
        NamedColor::BrightCyan => Color::LightCyan,
        NamedColor::BrightWhite => Color::White,
        // Foreground/Background/Cursor etc. — usar Reset para
        // que ratatui aplique el default del theme
        _ => Color::Reset,
    }
}

/// Convierte un `Color` de alacritty (ANSI) a un `Color` de ratatui.
///
/// Soporta Named, Indexed (256-color), y Spec (true color RGB).
pub fn ansi_to_ratatui(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Named(nc) => named_to_ratatui(nc),
        AnsiColor::Indexed(idx) => Color::Indexed(idx),
        AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

// ─── Build Lines ───────────────────────────────────────────────────────────────

/// Construye las líneas de ratatui desde la grilla del terminal.
///
/// `height` = filas visibles del viewport, `width` = columnas visibles.
/// Pre-computa TODO fuera del render — el caller pasa el `Vec<Line>` a
/// `Paragraph::new()` sin procesamiento adicional.
///
/// Optimizaciones:
/// - `Vec::with_capacity` para líneas y spans — zero re-allocations
/// - Agrupa celdas consecutivas con el mismo estilo en un solo `Span`
/// - Skippea wide-char spacers (no emite char duplicado)
/// - Solo procesa las `height` líneas del viewport (virtualización)
pub fn build_lines(term: &Term<VoidListener>, height: usize, width: usize) -> Vec<Line<'static>> {
    if height == 0 || width == 0 {
        return Vec::new();
    }

    let grid = term.grid();
    let screen_lines = grid.screen_lines();
    let num_cols = grid.columns();
    let lines_to_show = height.min(screen_lines);
    let cols_to_show = width.min(num_cols);

    let mut result = Vec::with_capacity(lines_to_show);

    // Buffer reutilizable para acumular chars del span actual.
    // Se limpia en cada span nuevo — preserva capacidad del heap.
    let mut span_buf = String::with_capacity(cols_to_show);

    // Iterar las líneas visibles del grid.
    // Line(0) = primera línea de la pantalla, Line(screen_lines - 1) = última.
    for line_idx in 0..lines_to_show {
        let row = &grid[GridLine(line_idx as i32)];

        let mut spans: Vec<Span<'static>> = Vec::with_capacity(8);
        span_buf.clear();

        // Estilo de la primera celda — inicialización segura
        let first_cell = &row[Column(0)];
        let mut current_style = cell_style(first_cell);

        for col in 0..cols_to_show {
            let cell = &row[Column(col)];

            // Skip wide-char spacers — el char principal ya fue emitido
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }

            let style = cell_style(cell);

            if style != current_style {
                // Flush del span actual
                if !span_buf.is_empty() {
                    // CLONE: necesario — span_buf se reutiliza, Span toma ownership
                    spans.push(Span::styled(span_buf.clone(), current_style));
                    span_buf.clear();
                }
                current_style = style;
            }

            span_buf.push(cell.c);
        }

        // Flush último span de la línea
        if !span_buf.is_empty() {
            // CLONE: necesario — span_buf se reutiliza entre líneas
            spans.push(Span::styled(span_buf.clone(), current_style));
            span_buf.clear();
        }

        result.push(Line::from(spans));
    }

    result
}

/// Computa el `Style` de ratatui para una celda de alacritty.
///
/// Mapea foreground, background, y flags (bold, italic, underline, dim)
/// a los estilos correspondientes de ratatui. No aloca.
fn cell_style(cell: &alacritty_terminal::term::cell::Cell) -> Style {
    let fg = ansi_to_ratatui(cell.fg);
    let bg = ansi_to_ratatui(cell.bg);
    let mut style = Style::default().fg(fg).bg(bg);

    let flags = cell.flags;
    if flags.contains(Flags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if flags.contains(Flags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if flags.intersects(Flags::UNDERLINE | Flags::DOUBLE_UNDERLINE | Flags::UNDERCURL) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if flags.contains(Flags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if flags.contains(Flags::INVERSE) {
        style = style.add_modifier(Modifier::REVERSED);
    }
    if flags.contains(Flags::STRIKEOUT) {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    if flags.contains(Flags::HIDDEN) {
        style = style.add_modifier(Modifier::HIDDEN);
    }

    style
}
