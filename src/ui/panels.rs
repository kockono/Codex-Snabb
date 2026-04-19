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

use crate::core::settings::SidebarSection;
use crate::core::PanelId;
use crate::editor::cursor::Position;
use crate::editor::selection::Selection;
use crate::editor::tabs::TabInfo;
use crate::editor::EditorState;
use crate::lsp;
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

// ─── Activity Bar ──────────────────────────────────────────────────────────────

/// Renderiza la activity bar: columna delgada (3 cols) con iconos de sección.
///
/// Siempre visible (no se oculta con Ctrl+B). Muestra iconos apilados
/// verticalmente: Explorer, Git, Search en la parte superior, y Settings
/// siempre en la parte inferior.
///
/// El icono activo tiene highlight con `fg_accent`. Los demás usan `fg_secondary`.
/// No aloca — usa literales y estilos estáticos.
pub fn render_activity_bar(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    active_section: SidebarSection,
    settings_active: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    // Fondo de la activity bar
    let bg_style = Style::default().bg(theme.bg_status);

    // Ícono activo vs inactivo
    let style_for = |active: bool| -> Style {
        if active {
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_status)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg_secondary).bg(theme.bg_status)
        }
    };

    // Definir iconos (centrados en 3 columnas: " X ")
    let icons: &[(&str, bool)] = &[
        (
            " E ",
            active_section == SidebarSection::Explorer && !settings_active,
        ),
        (
            " G ",
            active_section == SidebarSection::Git && !settings_active,
        ),
        (
            " S ",
            active_section == SidebarSection::Search && !settings_active,
        ),
    ];

    // Construir líneas: iconos en la parte superior, settings en la parte inferior
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(area.height as usize);

    for (i, &(icon, active)) in icons.iter().enumerate() {
        if i < area.height as usize {
            lines.push(Line::from(Span::styled(icon, style_for(active))));
        }
    }

    // Rellenar hasta la penúltima fila
    let settings_row = (area.height as usize).saturating_sub(2);
    while lines.len() < settings_row {
        lines.push(Line::from(Span::styled("   ", bg_style)));
    }

    // Settings en la penúltima fila
    if lines.len() < area.height as usize {
        let settings_style = style_for(settings_active);
        lines.push(Line::from(Span::styled("⚙ ", settings_style)));
    }

    // Última fila vacía
    while lines.len() < area.height as usize {
        lines.push(Line::from(Span::styled("   ", bg_style)));
    }

    let paragraph = Paragraph::new(lines).style(bg_style);
    f.render_widget(paragraph, area);
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
    let display_name = crate::ui::truncate_str(&entry.name, name_max);

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

/// Renderiza el área del editor con barra de tabs y contenido real del buffer.
///
/// Si el buffer está vacío y no tiene archivo asociado, muestra un placeholder.
/// Si hay contenido, renderiza:
/// - Barra de tabs (1 línea) con pestañas de archivos abiertos
/// - Gutter con números de línea (ancho dinámico)
/// - Separador `│`
/// - Texto del buffer con viewport virtual (solo líneas visibles)
/// - Highlight de la línea actual (background sutil)
///
/// El cursor visual es el hardware cursor de la terminal (SteadyBar),
/// posicionado por `f.set_cursor_position()` en `ui::render()`.
///
/// No aloca strings en el render — usa slices `&str` del buffer directamente.
pub fn render_editor_area(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    editor: &EditorState,
    diagnostics: &[lsp::Diagnostic],
    tab_infos: &[TabInfo],
) {
    let block = panel_block("EDITOR", focused, theme).style(Style::default().bg(theme.bg_primary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Tab bar: 1 línea de pestañas arriba del contenido ──
    let (tab_bar_area, content_area) = {
        let split = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Fill(1)])
            .split(inner);
        (split[0], split[1])
    };
    render_tab_bar(f, tab_bar_area, theme, tab_infos);

    let inner = content_area;
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
    let primary_cursor_line = editor.cursors.primary().position.line;

    // Recopilar todas las selecciones activas para renderizar
    let selections: Vec<Selection> = editor
        .cursors
        .cursors
        .iter()
        .filter_map(|c| c.selection.filter(|s| !s.is_empty()))
        .collect();

    // Recopilar posiciones de cursores secundarios para renderizar
    let secondary_cursor_positions: Vec<Position> = editor
        .cursors
        .cursors
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != editor.cursors.primary_index)
        .map(|(_, c)| c.position)
        .collect();

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
    let selection_style = Style::default().fg(theme.fg_primary).bg(theme.selection);
    let secondary_cursor_style = Style::default()
        .fg(theme.fg_accent)
        .add_modifier(Modifier::REVERSED);
    // Estilos de diagnóstico (subrayado con color de severidad)
    let diag_error_style = Style::default()
        .fg(theme.fg_error)
        .add_modifier(Modifier::UNDERLINED);
    let diag_warning_style = Style::default()
        .fg(theme.fg_warning)
        .add_modifier(Modifier::UNDERLINED);

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
            let is_cursor_line = buf_line_idx == primary_cursor_line;

            // ── Gutter: número de línea ──
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
            // Truncar al ancho del viewport sin alocar — char-safe para multi-byte
            let display_text = crate::ui::truncate_str(line_content, text_width);

            // Construir spans para esta línea
            let mut spans: Vec<Span<'_>> = Vec::with_capacity(8);
            spans.push(Span::styled(pad_str, gutter_num_style));
            // CLONE: necesario — num_buf se reutiliza en cada iteración del loop,
            // Span toma ownership del String para mantenerlo vivo en el Line
            spans.push(Span::styled(num_buf.clone(), gutter_num_style));
            spans.push(Span::styled("\u{2502} ", separator_style));

            // ── Renderizar texto con selecciones, cursores y diagnósticos ──
            if !display_text.is_empty() {
                // Recopilar rangos de diagnósticos para esta línea
                let line_diags: Vec<(usize, usize, &lsp::DiagnosticSeverity)> = diagnostics
                    .iter()
                    .filter(|d| d.line == buf_line_idx as u32)
                    .map(|d| {
                        let start = (d.col_start as usize).min(display_text.len());
                        let end = (d.col_end as usize).min(display_text.len());
                        (start, end, &d.severity)
                    })
                    .collect();

                let text_spans = render_line_with_selections(
                    display_text,
                    buf_line_idx,
                    &selections,
                    &secondary_cursor_positions,
                    &line_diags,
                    line_bg_style,
                    selection_style,
                    secondary_cursor_style,
                    diag_error_style,
                    diag_warning_style,
                );
                spans.extend(text_spans);
            }

            lines.push(Line::from(spans));
        } else {
            // ── Líneas vacías después del buffer ──
            let pad_str = &SPACES[..gutter_width.min(SPACES.len())];
            let spans = vec![
                Span::styled(pad_str, gutter_style),
                Span::styled("\u{2502} ", separator_style),
            ];
            lines.push(Line::from(spans));
        }
    }

    let paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_primary));
    f.render_widget(paragraph, inner);
}

/// Renderiza una línea de texto con highlights de selección, cursores y diagnósticos.
///
/// Divide la línea en segmentos según las selecciones activas, posiciones
/// de cursores secundarios y rangos de diagnósticos. Cada segmento recibe
/// el estilo apropiado. No usa `format!()` — construye spans directamente.
#[expect(
    clippy::too_many_arguments,
    reason = "render helper — pasar struct de estilos agregaría complejidad sin beneficio"
)]
fn render_line_with_selections<'a>(
    text: &str,
    line_idx: usize,
    selections: &[Selection],
    secondary_cursors: &[Position],
    diagnostics: &[(usize, usize, &lsp::DiagnosticSeverity)],
    normal_style: Style,
    selection_style: Style,
    cursor_style: Style,
    diag_error_style: Style,
    diag_warning_style: Style,
) -> Vec<Span<'a>> {
    let text_len = text.len();
    if text_len == 0 {
        return vec![];
    }

    // Determinar qué columnas están seleccionadas y cuáles tienen cursor secundario
    // Usar un enfoque eficiente: construir rangos de selección en esta línea
    let mut selected_ranges: Vec<(usize, usize)> = Vec::new();
    for sel in selections {
        let start = sel.start();
        let end = sel.end();

        // Verificar si esta selección toca esta línea
        if start.line <= line_idx && end.line >= line_idx {
            let sel_start_col = if start.line == line_idx { start.col } else { 0 };
            let sel_end_col = if end.line == line_idx {
                end.col
            } else {
                text_len
            };
            if sel_start_col < sel_end_col {
                selected_ranges.push((sel_start_col.min(text_len), sel_end_col.min(text_len)));
            }
        }
    }

    // Columnas con cursores secundarios
    let cursor_cols: Vec<usize> = secondary_cursors
        .iter()
        .filter(|p| p.line == line_idx && p.col < text_len)
        .map(|p| p.col)
        .collect();

    // Si no hay selecciones, cursores secundarios, ni diagnósticos, renderizar normal
    if selected_ranges.is_empty() && cursor_cols.is_empty() && diagnostics.is_empty() {
        // CLONE: necesario — text es un slice del buffer, Span toma ownership
        return vec![Span::styled(text.to_string(), normal_style)];
    }

    // Construir spans divididos por selección, cursores y diagnósticos.
    // Estrategia: recorrer por char boundaries (byte offsets válidos),
    // agrupar segmentos contiguos con el mismo estilo para minimizar spans.
    // IMPORTANTE: iterar por char_indices() garantiza que nunca cortamos
    // en medio de un carácter multi-byte (UTF-8 de 2-4 bytes).
    let mut result: Vec<Span<'a>> = Vec::with_capacity(8);

    // Recopilar byte offsets de char boundaries
    let char_boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();

    if char_boundaries.is_empty() {
        return result;
    }

    let mut current_start = 0;
    let mut current_style = char_style_with_diags(
        0,
        &selected_ranges,
        &cursor_cols,
        diagnostics,
        normal_style,
        selection_style,
        cursor_style,
        diag_error_style,
        diag_warning_style,
    );

    // Iterar desde el segundo char boundary hasta el final
    for &byte_offset in char_boundaries.iter().skip(1) {
        let style = char_style_with_diags(
            byte_offset,
            &selected_ranges,
            &cursor_cols,
            diagnostics,
            normal_style,
            selection_style,
            cursor_style,
            diag_error_style,
            diag_warning_style,
        );

        if style != current_style {
            // Flush segmento actual — current_start y byte_offset son char boundaries
            let segment = &text[current_start..byte_offset];
            if !segment.is_empty() {
                // CLONE: necesario — segment es slice del buffer
                result.push(Span::styled(segment.to_string(), current_style));
            }
            current_start = byte_offset;
            current_style = style;
        }
    }

    // Flush final: desde current_start hasta el final del string
    if current_start < text_len {
        let segment = &text[current_start..];
        if !segment.is_empty() {
            // CLONE: necesario — segment es slice del buffer
            result.push(Span::styled(segment.to_string(), current_style));
        }
    }

    result
}

/// Determina el estilo de un carácter en una columna dada.
///
/// Prioridad: cursor secundario > selección > diagnóstico > normal.
#[expect(
    clippy::too_many_arguments,
    reason = "render helper — pasar struct de estilos agregaría complejidad sin beneficio"
)]
fn char_style_with_diags(
    col: usize,
    selected_ranges: &[(usize, usize)],
    cursor_cols: &[usize],
    diagnostics: &[(usize, usize, &lsp::DiagnosticSeverity)],
    normal_style: Style,
    selection_style: Style,
    cursor_style: Style,
    diag_error_style: Style,
    diag_warning_style: Style,
) -> Style {
    // Cursor secundario tiene prioridad visual máxima
    if cursor_cols.contains(&col) {
        return cursor_style;
    }
    // Selección
    for &(start, end) in selected_ranges {
        if col >= start && col < end {
            return selection_style;
        }
    }
    // Diagnóstico (error tiene prioridad sobre warning)
    let mut has_warning = false;
    for &(start, end, severity) in diagnostics {
        if col >= start && col < end {
            match severity {
                lsp::DiagnosticSeverity::Error => return diag_error_style,
                lsp::DiagnosticSeverity::Warning => has_warning = true,
                _ => {}
            }
        }
    }
    if has_warning {
        return diag_warning_style;
    }
    normal_style
}

// ─── Tab Bar ───────────────────────────────────────────────────────────────────

/// Renderiza la barra de tabs del editor.
///
/// Cada tab muestra: `│ filename.ext ● │` (dirty) o `│ filename.ext × │` (activa, para cerrar).
/// Tab activa: background `bg_active`, texto `fg_accent`.
/// Tabs inactivas: background `bg_secondary`, texto `fg_secondary`.
/// `●` (U+25CF) en `fg_warning` cuando dirty.
/// `×` (U+00D7) solo en tab activa.
/// Si las tabs no caben, se trunca con `…` al final.
///
/// No aloca strings innecesarios — los nombres vienen pre-computados en `TabInfo`.
fn render_tab_bar(f: &mut Frame, area: Rect, theme: &Theme, tabs: &[TabInfo]) {
    if area.width == 0 || tabs.is_empty() {
        return;
    }

    let max_width = area.width as usize;
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(tabs.len() * 4);
    let mut used_width: usize = 0;
    let mut truncated = false;

    for tab in tabs {
        // Calcular ancho de esta tab: "│ " + name + " ●" o " ×" + " "
        // Indicador: dirty → " ●", activa → " ×", limpia+inactiva → nada
        let indicator = if tab.is_dirty {
            " \u{25CF}" // " ●"
        } else if tab.is_active {
            " \u{00D7}" // " ×"
        } else {
            ""
        };
        // "│ " (2) + name.len() + indicator.len() + " " (1 padding derecho)
        let tab_width = 2 + tab.name.len() + indicator.len() + 1;

        // Verificar si cabe — si no, mostrar "…" y cortar
        if used_width + tab_width + 1 > max_width {
            // No cabe — agregar "…" si queda espacio
            if used_width + 2 <= max_width {
                spans.push(Span::styled(
                    " \u{2026}",
                    Style::default()
                        .fg(theme.fg_secondary)
                        .bg(theme.bg_secondary),
                ));
            }
            truncated = true;
            break;
        }

        let (bg, fg) = if tab.is_active {
            (theme.bg_active, theme.fg_accent)
        } else {
            (theme.bg_secondary, theme.fg_secondary)
        };

        let tab_style = Style::default().fg(fg).bg(bg);
        let sep_style = Style::default()
            .fg(theme.border_unfocused)
            .bg(theme.bg_secondary);

        // Separador izquierdo
        spans.push(Span::styled("\u{2502} ", sep_style));
        // Nombre del archivo
        spans.push(Span::styled(tab.name.as_str(), tab_style));

        // Indicador dirty/close
        if tab.is_dirty {
            spans.push(Span::styled(
                " \u{25CF}",
                Style::default().fg(theme.fg_warning).bg(bg),
            ));
        } else if tab.is_active {
            spans.push(Span::styled(
                " \u{00D7}",
                Style::default().fg(theme.fg_secondary).bg(bg),
            ));
        }

        // Padding derecho
        spans.push(Span::styled(" ", Style::default().bg(bg)));
        used_width += tab_width;
    }

    // Separador final si no truncamos
    if !truncated && used_width < max_width {
        spans.push(Span::styled(
            "\u{2502}",
            Style::default()
                .fg(theme.border_unfocused)
                .bg(theme.bg_secondary),
        ));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(paragraph, area);
}

/// Cuenta la cantidad de dígitos decimales de un número.
///
/// Pre-computado fuera del render loop. Evita `format!()` para contar dígitos.
pub(crate) fn digit_count(n: usize) -> usize {
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
            // Truncar línea al ancho del panel sin alocar — char-safe para multi-byte
            let display = crate::ui::truncate_str(line, max_width);
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

// ─── LSP Hover ─────────────────────────────────────────────────────────────────

/// Renderiza un tooltip de hover LSP como overlay cerca del cursor.
///
/// Box con borde, fondo secundario, texto del hover.
/// Posicionado justo encima del cursor (o debajo si no hay espacio arriba).
/// No aloca strings adicionales — usa el content del HoverInfo directamente.
pub fn render_lsp_hover(
    f: &mut Frame,
    editor_area: Rect,
    theme: &Theme,
    hover: &lsp::HoverInfo,
    editor: &EditorState,
) {
    let content = &hover.content;
    if content.is_empty() {
        return;
    }

    // Calcular posición del cursor en coordenadas absolutas del terminal
    let inner_x = editor_area.x + 1;
    let inner_y = editor_area.y + 1;

    let scroll = editor.viewport.scroll_offset;
    let cursor_line = editor.cursors.primary().position.line;
    let cursor_col = editor.cursors.primary().position.col;

    // Verificar que el cursor está visible
    let inner_h = editor_area.height.saturating_sub(2) as usize;
    if cursor_line < scroll || cursor_line >= scroll + inner_h {
        return;
    }

    let visual_row = (cursor_line - scroll) as u16;
    let total_lines = editor.buffer.line_count();
    let gutter_width = digit_count(total_lines).max(4) as u16 + 2;

    // Posición del tooltip: encima del cursor si hay espacio, sino debajo
    let abs_col = inner_x + gutter_width + cursor_col as u16;
    let abs_row = inner_y + visual_row;

    // Calcular dimensiones del tooltip
    let lines: Vec<&str> = content.lines().collect();
    let max_line_width = lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let tooltip_width = (max_line_width as u16 + 4).clamp(10, 60);
    let tooltip_height = (lines.len() as u16 + 2).min(10);

    // Posicionar: intentar arriba del cursor primero
    let tooltip_y = if abs_row > tooltip_height {
        abs_row - tooltip_height
    } else {
        abs_row + 1
    };

    // Clampear al área del frame
    let frame_area = f.area();
    let tooltip_x = abs_col.min(frame_area.width.saturating_sub(tooltip_width));
    let tooltip_y = tooltip_y.min(frame_area.height.saturating_sub(tooltip_height));

    let tooltip_area = Rect::new(tooltip_x, tooltip_y, tooltip_width, tooltip_height);

    // Renderizar el tooltip
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Hover ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.bg_secondary));

    // Truncar contenido al espacio disponible
    let inner = block.inner(tooltip_area);
    let visible_lines: Vec<Line<'_>> = lines
        .iter()
        .take(inner.height as usize)
        .map(|line| {
            let display = crate::ui::truncate_str(line, inner.width as usize);
            Line::from(Span::styled(
                display.to_string(), // CLONE: necesario — display es slice, Span toma ownership
                Style::default().fg(theme.fg_primary),
            ))
        })
        .collect();

    // Clear the area under the tooltip first
    let clear = Paragraph::new("").style(Style::default().bg(theme.bg_secondary));
    f.render_widget(clear, tooltip_area);

    let paragraph = Paragraph::new(visible_lines)
        .block(block)
        .style(Style::default().bg(theme.bg_secondary));

    f.render_widget(paragraph, tooltip_area);
}

// ─── LSP Completions ───────────────────────────────────────────────────────────

/// Renderiza la lista dropdown de autocompletado LSP.
///
/// Posicionada debajo del cursor. Max 10 items visibles.
/// El item seleccionado tiene highlight de fondo.
pub fn render_lsp_completions(
    f: &mut Frame,
    editor_area: Rect,
    theme: &Theme,
    completions: &[lsp::CompletionItem],
    selected: usize,
    editor: &EditorState,
) {
    if completions.is_empty() {
        return;
    }

    // Calcular posición del cursor en coordenadas absolutas
    let inner_x = editor_area.x + 1;
    let inner_y = editor_area.y + 1;

    let scroll = editor.viewport.scroll_offset;
    let cursor_line = editor.cursors.primary().position.line;
    let cursor_col = editor.cursors.primary().position.col;

    let inner_h = editor_area.height.saturating_sub(2) as usize;
    if cursor_line < scroll || cursor_line >= scroll + inner_h {
        return;
    }

    let visual_row = (cursor_line - scroll) as u16;
    let total_lines = editor.buffer.line_count();
    let gutter_width = digit_count(total_lines).max(4) as u16 + 2;

    let abs_col = inner_x + gutter_width + cursor_col as u16;
    let abs_row = inner_y + visual_row;

    // Dimensiones del dropdown
    let max_visible = 10.min(completions.len());
    let max_label_width = completions
        .iter()
        .take(max_visible)
        .map(|c| {
            let kind_len = c.kind.as_ref().map(|k| k.len() + 3).unwrap_or(0);
            c.label.len() + kind_len
        })
        .max()
        .unwrap_or(10);
    let dropdown_width = (max_label_width as u16 + 4).clamp(15, 50);
    let dropdown_height = max_visible as u16 + 2; // +2 para bordes

    // Posicionar debajo del cursor
    let dropdown_y = abs_row + 1;
    let frame_area = f.area();
    let dropdown_x = abs_col.min(frame_area.width.saturating_sub(dropdown_width));
    let dropdown_y = dropdown_y.min(frame_area.height.saturating_sub(dropdown_height));

    let dropdown_area = Rect::new(dropdown_x, dropdown_y, dropdown_width, dropdown_height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.bg_secondary));

    // Construir líneas para cada completion item
    let lines: Vec<Line<'_>> = completions
        .iter()
        .take(max_visible)
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == selected;
            let bg = if is_selected {
                theme.bg_active
            } else {
                theme.bg_secondary
            };
            let fg = if is_selected {
                theme.fg_accent
            } else {
                theme.fg_primary
            };

            let mut spans = vec![Span::styled(
                // CLONE: necesario — label es String en el CompletionItem
                item.label.clone(),
                Style::default().fg(fg).bg(bg),
            )];

            // Agregar kind si existe
            if let Some(ref kind) = item.kind {
                spans.push(Span::styled(
                    format!(" [{kind}]"),
                    Style::default().fg(theme.fg_secondary).bg(bg),
                ));
            }

            Line::from(spans)
        })
        .collect();

    // Clear area
    let clear = Paragraph::new("").style(Style::default().bg(theme.bg_secondary));
    f.render_widget(clear, dropdown_area);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().bg(theme.bg_secondary));

    f.render_widget(paragraph, dropdown_area);
}
