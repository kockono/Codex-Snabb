//! Panels: renderizado de cada panel del shell visual.
//!
//! Cada función de render recibe datos pre-computados y dibuja.
//! Son stateless renderers — sin IO, sin cómputo pesado, sin allocaciones.
//! Los bordes cambian de estilo según el estado de foco.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::core::PanelId;
use crate::ui::theme::Theme;

// ─── StatusBarData ─────────────────────────────────────────────────────────────

/// Datos pre-computados para la status bar.
///
/// Todos los campos son `&str` — la función de render no aloca.
/// Los datos se derivan del estado FUERA del render y se pasan por referencia.
#[derive(Debug)]
pub struct StatusBarData<'a> {
    /// Modo actual del editor (NORMAL, INSERT, etc.).
    pub mode: &'a str,
    /// Nombre del archivo activo.
    pub file_name: &'a str,
    /// Posición del cursor formateada (ej: "Ln 42, Col 7").
    pub cursor_pos: &'a str,
    /// Branch de Git activa.
    pub branch: &'a str,
    /// Encoding del archivo activo.
    pub encoding: &'a str,
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

/// Construye un bloque con borde estilizado según estado de foco.
///
/// - Enfocado: `border_focused` + `BorderType::Double` + título en accent
/// - No enfocado: `border_unfocused` + `BorderType::Plain` + título dimmed
fn panel_block<'a>(title: &'a str, focused: bool, theme: &'a Theme) -> Block<'a> {
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

    Block::default()
        .title(Line::from(Span::styled(title, title_style)))
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border_color))
}

// ─── Title Bar ─────────────────────────────────────────────────────────────────

/// Renderiza la barra de título del IDE.
///
/// Muestra el nombre del IDE con estilo cyberpunk. Sin bordes, 1 línea.
/// No aloca — usa literales y estilos estáticos.
pub fn render_title_bar(f: &mut Frame, area: Rect, theme: &Theme) {
    let title = Line::from(vec![
        Span::styled(
            " ⚡ ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "IDE TUI",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — RAM/CPU First", Style::default().fg(theme.fg_accent_alt)),
    ]);

    let bar = Paragraph::new(title).style(Style::default().bg(theme.bg_status));

    f.render_widget(bar, area);
}

// ─── Sidebar ───────────────────────────────────────────────────────────────────

/// Renderiza el panel lateral (sidebar).
///
/// Por ahora es placeholder con el nombre del sub-panel activo.
/// El contenido real (explorer, git, search) se implementará en épicas posteriores.
pub fn render_sidebar(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    active_panel: PanelId,
) {
    let panel_label = match active_panel {
        PanelId::Explorer => "EXPLORER",
        PanelId::Git => "SOURCE CONTROL",
        PanelId::Search => "SEARCH",
        _ => "EXPLORER", // default cuando el foco está en otro panel
    };

    let block =
        panel_block(panel_label, focused, theme).style(Style::default().bg(theme.bg_secondary));

    let content = Paragraph::new(Line::from(Span::styled(
        "  No folder open",
        Style::default().fg(theme.fg_secondary),
    )))
    .block(block);

    f.render_widget(content, area);
}

// ─── Editor Area ───────────────────────────────────────────────────────────────

/// Renderiza el área del editor.
///
/// Placeholder: muestra "No file open" centrado. El editor real se
/// implementará en épica 2. Borde refleja estado de foco.
pub fn render_editor_area(f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
    let block = panel_block("EDITOR", focused, theme).style(Style::default().bg(theme.bg_primary));

    // Centrar el mensaje verticalmente
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height > 0 && inner.width > 0 {
        let vertical = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(inner);

        let placeholder = Paragraph::new(Line::from(vec![Span::styled(
            "No file open",
            Style::default()
                .fg(theme.fg_secondary)
                .add_modifier(Modifier::ITALIC),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme.bg_primary));

        f.render_widget(placeholder, vertical[1]);
    }
}

// ─── Bottom Panel ──────────────────────────────────────────────────────────────

/// Renderiza el panel inferior (terminal/problems/output).
///
/// Placeholder con texto "Terminal". El contenido real se implementará
/// en épica 7. Borde refleja estado de foco.
pub fn render_bottom_panel(f: &mut Frame, area: Rect, theme: &Theme, focused: bool) {
    let block =
        panel_block("TERMINAL", focused, theme).style(Style::default().bg(theme.bg_secondary));

    let content = Paragraph::new(Line::from(Span::styled(
        "  Terminal placeholder",
        Style::default().fg(theme.fg_secondary),
    )))
    .block(block);

    f.render_widget(content, area);
}

// ─── Status Bar ────────────────────────────────────────────────────────────────

/// Renderiza la barra de estado inferior.
///
/// Muestra: modo, archivo, posición del cursor, branch, encoding.
/// Todos los datos llegan pre-computados via `StatusBarData` — sin allocaciones.
pub fn render_status_bar(f: &mut Frame, area: Rect, theme: &Theme, data: &StatusBarData<'_>) {
    // Construir los spans de la status bar sin format!()
    let left_spans = vec![
        Span::styled(" ", Style::default().bg(theme.fg_accent)),
        Span::styled(
            data.mode,
            Style::default()
                .fg(theme.bg_primary)
                .bg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().bg(theme.fg_accent)),
        Span::styled(" ", Style::default().bg(theme.bg_status)),
        Span::styled(
            data.branch,
            Style::default().fg(theme.fg_accent_alt).bg(theme.bg_status),
        ),
        Span::styled("  ", Style::default().bg(theme.bg_status)),
        Span::styled(
            data.file_name,
            Style::default().fg(theme.fg_primary).bg(theme.bg_status),
        ),
    ];

    let right_spans = vec![
        Span::styled(
            data.cursor_pos,
            Style::default().fg(theme.fg_primary).bg(theme.bg_status),
        ),
        Span::styled("  ", Style::default().bg(theme.bg_status)),
        Span::styled(
            data.encoding,
            Style::default().fg(theme.fg_secondary).bg(theme.bg_status),
        ),
        Span::styled(" ", Style::default().bg(theme.bg_status)),
    ];

    // Layout horizontal: left flush, right flush
    let horizontal = Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Fill(1)])
        .split(area);

    let left = Paragraph::new(Line::from(left_spans)).style(Style::default().bg(theme.bg_status));

    let right = Paragraph::new(Line::from(right_spans))
        .alignment(Alignment::Right)
        .style(Style::default().bg(theme.bg_status));

    f.render_widget(left, horizontal[0]);
    f.render_widget(right, horizontal[1]);
}
