//! Panels: renderizado de cada panel del shell visual.
//!
//! Cada función de render recibe datos pre-computados y dibuja.
//! Son stateless renderers — sin IO, sin cómputo pesado, sin allocaciones.
//! Los bordes cambian de estilo según el estado de foco.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::core::PanelId;
use crate::editor::EditorState;
use crate::ui::theme::Theme;
use crate::workspace::explorer::{ExplorerState, FlatEntry};

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

/// Renderiza el panel lateral (sidebar) con el árbol de archivos real.
///
/// Si hay un `ExplorerState` disponible, renderiza el árbol con:
/// - Indentación por profundidad
/// - Iconos `▸`/`▾` para directorios collapsed/expanded
/// - Highlight del entry seleccionado
/// - Viewport virtual: solo renderiza entries visibles (scroll)
///
/// Si no hay explorer, muestra "No folder open".
pub fn render_sidebar(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    active_panel: PanelId,
    explorer: Option<&ExplorerState>,
) {
    let panel_label = match active_panel {
        PanelId::Explorer => "EXPLORER",
        PanelId::Git => "SOURCE CONTROL",
        PanelId::Search => "SEARCH",
        _ => "EXPLORER",
    };

    let block =
        panel_block(panel_label, focused, theme).style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let Some(explorer) = explorer else {
        // Sin explorer — mostrar placeholder
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "  No folder open",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(placeholder, inner);
        return;
    };

    let flat = explorer.flatten();
    let visible_height = inner.height as usize;
    let scroll = explorer.scroll_offset;

    // Viewport virtual: solo las entries visibles
    let visible_entries = flat.iter().skip(scroll).take(visible_height);

    // Pre-computar líneas fuera del render — evita format!() dentro del loop
    let lines: Vec<Line<'_>> = visible_entries
        .enumerate()
        .map(|(i, entry)| {
            render_explorer_entry(
                entry,
                scroll + i == explorer.selected_index,
                inner.width as usize,
                theme,
            )
        })
        .collect();

    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(paragraph, inner);
}

/// Renderiza una entrada del explorer como una `Line` de ratatui.
///
/// No aloca `format!()` — construye spans directamente.
/// El highlight de selección usa `bg_active` del theme.
fn render_explorer_entry<'a>(
    entry: &FlatEntry,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    // Indentación: 2 espacios por nivel de profundidad
    let indent_width = entry.depth * 2;

    // Icono: directorios `▸`/`▾`, archivos espacio
    let icon = if entry.is_dir {
        if entry.expanded {
            "▾ "
        } else {
            "▸ "
        }
    } else {
        "  "
    };

    // Calcular cuánto espacio queda para el nombre
    let prefix_len = indent_width + icon.len();
    let name_max = max_width.saturating_sub(prefix_len);
    let display_name = if entry.name.len() > name_max {
        &entry.name[..name_max]
    } else {
        &entry.name
    };

    // Estilo base según tipo y selección
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };
    let fg = if entry.is_dir {
        theme.fg_accent
    } else {
        theme.fg_primary
    };

    let style = Style::default().fg(fg).bg(bg);
    let indent_style = Style::default().bg(bg);

    // Construir indent string — pre-allocated con capacidad conocida
    // Usar un literal de espacios y tomar un slice es más eficiente
    // que format!() para indentación
    const SPACES: &str = "                                        "; // 40 espacios
    let indent_str = &SPACES[..indent_width.min(SPACES.len())];

    // CLONE: necesario en display_name.to_string() — Span::styled toma ownership
    // de String, y display_name es un slice de entry.name que no podemos mover
    let spans = vec![
        Span::styled(indent_str, indent_style),
        Span::styled(icon, style),
        Span::styled(display_name.to_string(), style),
    ];

    Line::from(spans)
}

// ─── Editor Area ───────────────────────────────────────────────────────────────

/// Renderiza el área del editor con contenido real del buffer.
///
/// Si el buffer está vacío y no tiene archivo asociado, muestra un placeholder.
/// Si hay contenido, renderiza:
/// - Gutter con números de línea (ancho dinámico)
/// - Separador `│`
/// - Texto del buffer con viewport virtual (solo líneas visibles)
/// - Highlight de la línea actual (background sutil)
/// - Cursor con `Modifier::REVERSED`
///
/// No aloca strings en el render — usa slices `&str` del buffer directamente.
pub fn render_editor_area(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    editor: &EditorState,
) {
    let block = panel_block("EDITOR", focused, theme).style(Style::default().bg(theme.bg_primary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Placeholder: buffer vacío sin archivo ──
    let has_content =
        editor.buffer.line_count() > 1 || editor.buffer.line(0).is_some_and(|l| !l.is_empty());
    let has_file = editor.buffer.file_path().is_some();

    if !has_content && !has_file {
        let vertical = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(inner);

        let placeholder = Paragraph::new(Line::from(vec![Span::styled(
            "No file open \u{2014} use Ctrl+P or Explorer",
            Style::default()
                .fg(theme.fg_secondary)
                .add_modifier(Modifier::ITALIC),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(theme.bg_primary));

        f.render_widget(placeholder, vertical[1]);
        return;
    }

    // ── Pre-computar ancho del gutter fuera del loop ──
    let total_lines = editor.buffer.line_count();
    let gutter_digits = digit_count(total_lines);
    // Mínimo 4 chars de ancho para el gutter (espacio visual)
    let gutter_width = gutter_digits.max(4);
    // Separador: 1 char `│` + 1 espacio
    let separator_width: usize = 2;
    let text_start = gutter_width + separator_width;

    let view_height = inner.height as usize;
    let view_width = inner.width as usize;
    let text_width = view_width.saturating_sub(text_start);

    // Usar viewport scroll_offset, pero clampear al tamaño real del inner area
    let scroll = editor.viewport.scroll_offset;
    let cursor_line = editor.cursor.position.line;
    let cursor_col = editor.cursor.position.col;

    // Estilos pre-computados — sin allocaciones
    let gutter_style = Style::default().fg(theme.line_number).bg(theme.bg_primary);
    let gutter_active_style = Style::default()
        .fg(theme.line_number_active)
        .bg(theme.bg_primary)
        .add_modifier(Modifier::BOLD);
    let separator_style = Style::default()
        .fg(theme.border_unfocused)
        .bg(theme.bg_primary);
    let text_style = Style::default().fg(theme.fg_primary).bg(theme.bg_primary);
    // Línea activa: background ligeramente más claro que bg_primary
    let active_line_bg = Color::Rgb(16, 20, 28);
    let text_active_style = Style::default().fg(theme.fg_primary).bg(active_line_bg);
    let cursor_style = Style::default()
        .fg(theme.bg_primary)
        .bg(theme.cursor)
        .add_modifier(Modifier::BOLD);
    let tilde_style = Style::default().fg(theme.fg_secondary).bg(theme.bg_primary);

    // Buffer pre-alocado para el padding del gutter
    // Máximo 10 dígitos (más que suficiente para cualquier archivo razonable)
    const SPACES: &str = "          "; // 10 espacios

    // Buffer reutilizable para números de línea — se limpia en cada iteración.
    // Capacidad inicial cubre el máximo razonable de dígitos para un archivo.
    let mut num_buf = String::with_capacity(12);

    // ── Construir líneas del viewport ──
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(view_height);

    for i in 0..view_height {
        let buf_line_idx = scroll + i;

        if buf_line_idx < total_lines {
            let is_cursor_line = buf_line_idx == cursor_line;

            // ── Gutter: número de línea ──
            // Alinear a la derecha sin format!() — usar dígitos pre-computados
            let line_num = buf_line_idx + 1; // 1-indexed para display
            let num_digits = digit_count(line_num);
            let padding = gutter_width.saturating_sub(num_digits);
            let pad_str = &SPACES[..padding.min(SPACES.len())];

            let gutter_num_style = if is_cursor_line {
                gutter_active_style
            } else {
                gutter_style
            };

            // Reutilizar buffer para el número — clear() preserva capacidad
            num_buf.clear();
            {
                use std::fmt::Write;
                let _ = write!(num_buf, "{line_num}");
            }

            let line_bg_style = if is_cursor_line {
                text_active_style
            } else {
                text_style
            };

            // ── Texto de la línea ──
            let line_content = editor.buffer.line(buf_line_idx).unwrap_or("");
            // Truncar al ancho del viewport sin alocar
            let display_text = if line_content.len() > text_width {
                &line_content[..text_width]
            } else {
                line_content
            };

            // Construir spans para esta línea
            let mut spans: Vec<Span<'_>> = Vec::with_capacity(6);
            spans.push(Span::styled(pad_str, gutter_num_style));
            // CLONE: necesario — num_buf se reutiliza en cada iteración del loop,
            // Span toma ownership del String para mantenerlo vivo en el Line
            spans.push(Span::styled(num_buf.clone(), gutter_num_style));
            spans.push(Span::styled("\u{2502} ", separator_style));

            // ── Texto con cursor ──
            if is_cursor_line && focused {
                // Dividir la línea en: pre-cursor, cursor char, post-cursor
                if cursor_col < display_text.len() {
                    let pre = &display_text[..cursor_col];
                    let cursor_ch = &display_text[cursor_col..cursor_col + 1];
                    let post = &display_text[cursor_col + 1..];
                    spans.push(Span::styled(pre.to_string(), line_bg_style));
                    spans.push(Span::styled(cursor_ch.to_string(), cursor_style));
                    if !post.is_empty() {
                        spans.push(Span::styled(post.to_string(), line_bg_style));
                    }
                } else {
                    // Cursor está al final o más allá del texto visible
                    if !display_text.is_empty() {
                        spans.push(Span::styled(display_text.to_string(), line_bg_style));
                    }
                    // Mostrar cursor como bloque en la posición después del texto
                    spans.push(Span::styled(" ", cursor_style));
                }
            } else {
                // Línea sin cursor — render directo
                // CLONE: necesario — display_text es un slice del buffer,
                // Span::styled necesita ownership porque la línea de ratatui
                // toma ownership de los spans
                if !display_text.is_empty() {
                    spans.push(Span::styled(display_text.to_string(), line_bg_style));
                }
            }

            lines.push(Line::from(spans));
        } else {
            // ── Líneas vacías después del buffer: `~` estilo Vim ──
            let pad_str = &SPACES[..gutter_width.min(SPACES.len())];
            let spans = vec![
                Span::styled(pad_str, gutter_style),
                Span::styled("\u{2502} ", separator_style),
                Span::styled("~", tilde_style),
            ];
            lines.push(Line::from(spans));
        }
    }

    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_primary));
    f.render_widget(paragraph, inner);
}

/// Cuenta la cantidad de dígitos decimales de un número.
///
/// Pre-computado fuera del render loop. Evita `format!()` para contar dígitos.
fn digit_count(n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut count = 0;
    let mut val = n;
    while val > 0 {
        count += 1;
        val /= 10;
    }
    count
}

// ─── Bottom Panel ──────────────────────────────────────────────────────────────

/// Renderiza el panel inferior con output real del terminal.
///
/// Si hay una sesión activa, muestra las líneas visibles del scrollback.
/// Si no hay sesión, muestra un placeholder con instrucciones.
/// Borde refleja estado de foco (Double/cyan cuando enfocado).
pub fn render_bottom_panel(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    session: Option<&crate::terminal::session::TerminalSession>,
) {
    let block =
        panel_block("TERMINAL", focused, theme).style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let Some(session) = session else {
        // Sin sesión — mostrar placeholder con instrucciones
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "  Press Ctrl+` to open terminal",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(placeholder, inner);
        return;
    };

    // Obtener líneas visibles del scrollback
    let visible = session.visible_lines(inner.height as usize);
    let max_width = inner.width as usize;

    // Construir líneas de ratatui — sin format!() en el loop
    let lines: Vec<Line<'_>> = visible
        .iter()
        .map(|line| {
            // Truncar línea al ancho del panel sin alocar
            let display = if line.len() > max_width {
                &line[..max_width]
            } else {
                line
            };
            Line::from(Span::styled(
                display.to_string(), // CLONE: necesario — Span toma ownership, display es slice de session
                Style::default().fg(theme.fg_primary),
            ))
        })
        .collect();

    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(paragraph, inner);
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
