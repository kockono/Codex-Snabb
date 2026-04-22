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

use std::path::Path;

use crate::editor::highlighting::HighlightToken;
use crate::editor::indent;

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
    /// Posición del cursor formateada (ej: "Ln 42, Col 7").
    pub cursor_pos: &'a str,
    /// Branch display completo: "⎇ main ↑2 ↓1 ⟳" — pre-formateado fuera del render.
    pub git_status: &'a str,
    /// Encoding del archivo activo.
    pub encoding: &'a str,
    /// Porcentaje de scroll en el archivo (ej: "18%"). Pre-formateado fuera del render.
    pub scroll_pct: &'a str,
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

/// Renderiza la barra de menú del IDE estilo VS Code.
///
/// Muestra opciones de menú: File, Edit, Selection, View, Go, Run, Terminal, Help.
/// Son labels estáticos por ahora — la interactividad (click → dropdown) es futuro.
/// No aloca — usa literales y estilos estáticos.
pub fn render_title_bar(f: &mut Frame, area: Rect, theme: &Theme) {
    let normal = Style::default().fg(theme.fg_secondary);
    let bar_items = Line::from(vec![
        Span::styled(" File ", normal),
        Span::styled(" Edit ", normal),
        Span::styled(" Selection ", normal),
        Span::styled(" View ", normal),
        Span::styled(" Go ", normal),
        Span::styled(" Run ", normal),
        Span::styled(" Terminal ", normal),
        Span::styled(" Help ", normal),
    ]);

    let bar = Paragraph::new(bar_items).style(Style::default().bg(theme.bg_status));

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
        (
            " P ",
            active_section == SidebarSection::Projects && !settings_active,
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
        PanelId::Projects => "PROJECTS",
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

    // Usar flat cache si disponible (pre-computado antes del render), fallback a flatten()
    let owned_flat;
    let flat: &[FlatEntry] = match explorer.cached_flat() {
        Some(cached) => cached,
        None => {
            owned_flat = explorer.flatten();
            &owned_flat
        }
    };
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
/// Incluye icono por extensión con color semántico antes del nombre.
/// No aloca `format!()` — construye spans directamente.
/// El highlight de selección usa `bg_active` del theme.
fn render_explorer_entry<'a>(
    entry: &FlatEntry,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    use crate::ui::icons;

    // Indentación: 2 espacios por nivel de profundidad
    let indent_width = entry.depth * 2;

    // Icono: emoji de carpeta (📁/📂, 2 celdas) o file icon (2 chars) por extensión.
    // Directorios ya NO usan indicador ▸/▾ separado — el emoji es suficiente.
    let (file_icon_str, icon_color, icon_display_width) = if entry.is_dir {
        // Emoji ocupa 2 celdas en terminal, pero el str tiene 4 bytes UTF-8
        (icons::dir_icon(entry.expanded), icons::dir_icon_color(), 2_usize)
    } else {
        // File icons ASCII: siempre 2 bytes = 2 celdas
        (icons::file_icon(&entry.name), icons::icon_color(&entry.name), 2_usize)
    };

    // Calcular cuánto espacio queda para el nombre
    // icon (icon_display_width) + " " (1) + nombre
    let icon_total_len = icon_display_width + 1; // +1 espacio después del icono
    let prefix_len = indent_width + icon_total_len;
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

    let name_style = Style::default().fg(fg).bg(bg);
    let indent_style = Style::default().bg(bg);
    let icon_style = Style::default().fg(icon_color).bg(bg);

    // Construir indent string — pre-allocated con capacidad conocida
    // Usar un literal de espacios y tomar un slice es más eficiente
    // que format!() para indentación
    const SPACES: &str = "                                        "; // 40 espacios
    let indent_str = &SPACES[..indent_width.min(SPACES.len())];

    // CLONE: necesario en display_name.to_string() — Span::styled toma ownership
    // de String, y display_name es un slice de entry.name que no podemos mover
    let spans = vec![
        Span::styled(indent_str, indent_style),
        Span::styled(file_icon_str, icon_style),
        Span::styled(" ", indent_style),
        Span::styled(display_name.to_string(), name_style),
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
#[expect(
    clippy::too_many_arguments,
    reason = "render entry point — bracket_match es pre-computado, no tiene sentido crear struct wrapper"
)]
pub fn render_editor_area(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    editor: &EditorState,
    diagnostics: &[lsp::Diagnostic],
    tab_infos: &[TabInfo],
    bracket_match: Option<(Position, Position)>,
    file_path: Option<&Path>,
    workspace_root: Option<&Path>,
) {
    let block = panel_block("EDITOR", focused, theme).style(Style::default().bg(theme.bg_primary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Tab bar (1) + Breadcrumbs (1) + Content (resto) ──
    let (tab_bar_area, breadcrumbs_area, content_area) = {
        let split = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(1), // tab bar
                Constraint::Length(1), // breadcrumbs
                Constraint::Fill(1),   // editor content
            ])
            .split(inner);
        (split[0], split[1], split[2])
    };
    render_tab_bar(f, tab_bar_area, theme, tab_infos);
    render_breadcrumbs(f, breadcrumbs_area, theme, file_path, workspace_root);

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

    // ── Pre-computar indent guides para el viewport ──
    let tab_width: usize = 4; // configurable en el futuro
    let viewport_lines: Vec<Option<&str>> = (0..view_height)
        .map(|i| {
            let idx = scroll + i;
            if idx < total_lines {
                editor.buffer.line(idx)
            } else {
                None
            }
        })
        .collect();
    let viewport_indents = indent::compute_viewport_indents(&viewport_lines, tab_width);

    // Estilo para indent guides
    let indent_guide_style = Style::default()
        .fg(theme.border_unfocused)
        .bg(theme.bg_primary);
    let indent_guide_active_style = Style::default().fg(theme.fg_secondary).bg(theme.bg_primary);
    // Estilo para indent guides en línea activa (con bg de línea activa)
    let indent_guide_cursor_style = Style::default()
        .fg(theme.border_unfocused)
        .bg(active_line_bg);
    let indent_guide_active_cursor_style =
        Style::default().fg(theme.fg_secondary).bg(active_line_bg);

    // ── Pre-computar estilos de bracket match ──
    let bracket_style = Style::default()
        .fg(theme.fg_accent)
        .add_modifier(Modifier::BOLD);
    let bracket_unmatched_style = Style::default()
        .fg(theme.fg_error)
        .add_modifier(Modifier::BOLD);

    // Columna del cursor para indent guide "activo" (nivel del cursor)
    let cursor_col = editor.cursors.primary().position.col;
    let cursor_indent_level = {
        // Redondear hacia abajo al múltiplo de tab_width más cercano
        (cursor_col / tab_width) * tab_width
    };

    // Buffer pre-alocado para el padding del gutter
    // Máximo 10 dígitos (más que suficiente para cualquier archivo razonable)
    const SPACES: &str = "          "; // 10 espacios

    // Buffer reutilizable para números de línea — se limpia en cada iteración.
    // Capacidad inicial cubre el máximo razonable de dígitos para un archivo.
    let mut num_buf = String::with_capacity(12);

    // ── Construir líneas del viewport ──
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(view_height);

    for (i, &line_indent_val) in viewport_indents.iter().enumerate().take(view_height) {
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
            let mut spans: Vec<Span<'_>> = Vec::with_capacity(12);
            spans.push(Span::styled(pad_str, gutter_num_style));
            // CLONE: necesario — num_buf se reutiliza en cada iteración del loop,
            // Span toma ownership del String para mantenerlo vivo en el Line
            spans.push(Span::styled(num_buf.clone(), gutter_num_style));
            spans.push(Span::styled("\u{2502} ", separator_style));

            // ── Indent guides: reemplazar espacios por `│` en posiciones de guía ──
            let line_indent = line_indent_val;
            let guide_cols = indent::guide_positions(line_indent, tab_width);

            // Determinar si esta línea tiene bracket match(es) que renderizar
            let bracket_at: Option<(usize, bool)> = bracket_match.and_then(|(a, b)| {
                if a.line == buf_line_idx {
                    Some((a.col, true)) // matched
                } else if b.line == buf_line_idx {
                    Some((b.col, true)) // matched
                } else {
                    None
                }
            });
            // Detectar bracket sin par: cursor sobre bracket pero sin match
            let unmatched_bracket_at: Option<usize> = if bracket_match.is_none() {
                // Verificar si el cursor está en esta línea y sobre un bracket
                if buf_line_idx == primary_cursor_line {
                    let ch = line_content.chars().nth(cursor_col);
                    if ch.is_some_and(crate::editor::brackets::is_bracket) {
                        Some(cursor_col)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let has_guides = !guide_cols.is_empty();
            let has_brackets = bracket_at.is_some() || unmatched_bracket_at.is_some();

            // ── Renderizar texto con selecciones, cursores, diagnósticos, guides y brackets ──
            if !display_text.is_empty() {
                // Si hay indent guides, producir spans para la zona de indentación
                // y luego el resto del texto. Los guides reemplazan espacios por `│`.
                if has_guides || has_brackets {
                    let indent_end = line_indent.min(display_text.len());

                    // Renderizar zona de indentación con guides intercalados
                    if has_guides && indent_end > 0 {
                        let mut col = 0;
                        for &guide_col in &guide_cols {
                            if guide_col >= indent_end || guide_col >= text_width {
                                break;
                            }
                            // Espacios antes del guide
                            if col < guide_col {
                                let space_count = guide_col - col;
                                // Usar slice de SPACES para los espacios intermedios
                                const INDENT_SPACES: &str =
                                    "                                        ";
                                let space_str =
                                    &INDENT_SPACES[..space_count.min(INDENT_SPACES.len())];
                                spans.push(Span::styled(space_str, line_bg_style));
                            }
                            // El guide `│` — activo si es el nivel del cursor
                            let is_active_guide =
                                guide_col == cursor_indent_level && is_cursor_line;
                            let guide_style = if is_cursor_line {
                                if is_active_guide {
                                    indent_guide_active_cursor_style
                                } else {
                                    indent_guide_cursor_style
                                }
                            } else if guide_col == cursor_indent_level {
                                indent_guide_active_style
                            } else {
                                indent_guide_style
                            };
                            spans.push(Span::styled("\u{2502}", guide_style));
                            col = guide_col + 1;
                        }
                        // Espacios restantes hasta el fin de la indentación
                        if col < indent_end {
                            let remaining = indent_end - col;
                            const INDENT_SPACES: &str = "                                        ";
                            let space_str = &INDENT_SPACES[..remaining.min(INDENT_SPACES.len())];
                            spans.push(Span::styled(space_str, line_bg_style));
                        }

                        // Resto del texto después de la indentación (con highlighting)
                        let rest = &display_text[indent_end..];
                        if !rest.is_empty() {
                            // Recopilar rangos de diagnósticos para esta línea
                            let line_diags: Vec<(usize, usize, &lsp::DiagnosticSeverity)> =
                                diagnostics
                                    .iter()
                                    .filter(|d| d.line == buf_line_idx as u32)
                                    .map(|d| {
                                        let start = (d.col_start as usize)
                                            .saturating_sub(indent_end)
                                            .min(rest.len());
                                        let end = (d.col_end as usize)
                                            .saturating_sub(indent_end)
                                            .min(rest.len());
                                        (start, end, &d.severity)
                                    })
                                    .collect();

                            let highlight_tokens = editor.highlight_cache.get_line(buf_line_idx);

                            let text_spans = render_line_with_selections(
                                rest,
                                buf_line_idx,
                                &selections,
                                &secondary_cursor_positions,
                                &line_diags,
                                line_bg_style,
                                selection_style,
                                secondary_cursor_style,
                                diag_error_style,
                                diag_warning_style,
                                highlight_tokens,
                                is_cursor_line,
                                active_line_bg,
                                indent_end,
                                bracket_at,
                                unmatched_bracket_at,
                                bracket_style,
                                bracket_unmatched_style,
                            );
                            spans.extend(text_spans);
                        }
                    } else {
                        // Sin guides pero con brackets — renderizar texto completo
                        let line_diags: Vec<(usize, usize, &lsp::DiagnosticSeverity)> = diagnostics
                            .iter()
                            .filter(|d| d.line == buf_line_idx as u32)
                            .map(|d| {
                                let start = (d.col_start as usize).min(display_text.len());
                                let end = (d.col_end as usize).min(display_text.len());
                                (start, end, &d.severity)
                            })
                            .collect();

                        let highlight_tokens = editor.highlight_cache.get_line(buf_line_idx);

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
                            highlight_tokens,
                            is_cursor_line,
                            active_line_bg,
                            0,
                            bracket_at,
                            unmatched_bracket_at,
                            bracket_style,
                            bracket_unmatched_style,
                        );
                        spans.extend(text_spans);
                    }
                } else {
                    // Sin guides ni brackets — renderizar texto normal
                    let line_diags: Vec<(usize, usize, &lsp::DiagnosticSeverity)> = diagnostics
                        .iter()
                        .filter(|d| d.line == buf_line_idx as u32)
                        .map(|d| {
                            let start = (d.col_start as usize).min(display_text.len());
                            let end = (d.col_end as usize).min(display_text.len());
                            (start, end, &d.severity)
                        })
                        .collect();

                    let highlight_tokens = editor.highlight_cache.get_line(buf_line_idx);

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
                        highlight_tokens,
                        is_cursor_line,
                        active_line_bg,
                        0,
                        None,
                        None,
                        bracket_style,
                        bracket_unmatched_style,
                    );
                    spans.extend(text_spans);
                }
            } else if has_guides {
                // Línea vacía con indent guides del contexto
                let mut col = 0;
                for &guide_col in &guide_cols {
                    if guide_col >= text_width {
                        break;
                    }
                    if col < guide_col {
                        let space_count = guide_col - col;
                        const INDENT_SPACES: &str = "                                        ";
                        let space_str = &INDENT_SPACES[..space_count.min(INDENT_SPACES.len())];
                        spans.push(Span::styled(space_str, line_bg_style));
                    }
                    let is_active_guide = guide_col == cursor_indent_level;
                    let guide_style = if is_cursor_line {
                        if is_active_guide {
                            indent_guide_active_cursor_style
                        } else {
                            indent_guide_cursor_style
                        }
                    } else if is_active_guide {
                        indent_guide_active_style
                    } else {
                        indent_guide_style
                    };
                    spans.push(Span::styled("\u{2502}", guide_style));
                    col = guide_col + 1;
                }
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

/// Convierte un color de syntect a un color de ratatui.
///
/// Mapeo directo RGB → RGB. No aloca.
fn syntect_color_to_ratatui(color: syntect::highlighting::Color) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

/// Renderiza una línea de texto con syntax highlighting, selecciones, cursores,
/// diagnósticos y bracket matching.
///
/// Capas de estilo (de fondo a frente):
/// 1. **Syntax highlighting** (foreground del token) — base visual
/// 2. **Línea activa** (background sutil) — indica cursor
/// 3. **Selección** (background de selección) — override de bg
/// 4. **Diagnósticos** (underline con color de severidad) — se agrega al estilo
/// 5. **Bracket match** (accent + bold) — resaltado de bracket par
/// 6. **Cursores secundarios** (reversed) — prioridad visual máxima
///
/// `col_offset`: offset de columna para ajustar las coordenadas cuando
/// el texto empieza después de la zona de indentación (indent guides).
///
/// Divide la línea en segmentos según cambios de estilo y los emite como spans.
/// No usa `format!()` — construye spans directamente.
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
    highlight_tokens: Option<&[HighlightToken]>,
    is_cursor_line: bool,
    active_line_bg: Color,
    col_offset: usize,
    bracket_at: Option<(usize, bool)>,
    unmatched_bracket_at: Option<usize>,
    bracket_style: Style,
    bracket_unmatched_style: Style,
) -> Vec<Span<'a>> {
    let text_len = text.len();
    if text_len == 0 {
        return vec![];
    }

    // Determinar qué columnas están seleccionadas y cuáles tienen cursor secundario.
    // Las coordenadas de selección/cursor son absolutas (columna en el buffer),
    // pero `text` puede empezar en `col_offset` (después de indent guides).
    // Ajustar restando col_offset para mapear a posición dentro de `text`.
    let mut selected_ranges: Vec<(usize, usize)> = Vec::new();
    for sel in selections {
        let start = sel.start();
        let end = sel.end();

        if start.line <= line_idx && end.line >= line_idx {
            let sel_start_col = if start.line == line_idx { start.col } else { 0 };
            let sel_end_col = if end.line == line_idx {
                end.col
            } else {
                col_offset + text_len
            };
            // Ajustar al espacio local del texto (restar col_offset)
            let local_start = sel_start_col.saturating_sub(col_offset).min(text_len);
            let local_end = sel_end_col.saturating_sub(col_offset).min(text_len);
            if local_start < local_end {
                selected_ranges.push((local_start, local_end));
            }
        }
    }

    // Columnas con cursores secundarios (ajustadas a espacio local)
    let cursor_cols: Vec<usize> = secondary_cursors
        .iter()
        .filter(|p| p.line == line_idx && p.col >= col_offset && p.col < col_offset + text_len)
        .map(|p| p.col - col_offset)
        .collect();

    // Columna de bracket match en esta línea (ajustada a espacio local)
    let local_bracket_col: Option<(usize, Style)> = bracket_at
        .and_then(|(col, _matched)| {
            if col >= col_offset && col < col_offset + text_len {
                Some((col - col_offset, bracket_style))
            } else {
                None
            }
        })
        .or_else(|| {
            unmatched_bracket_at.and_then(|col| {
                if col >= col_offset && col < col_offset + text_len {
                    Some((col - col_offset, bracket_unmatched_style))
                } else {
                    None
                }
            })
        });

    let has_overlays = !selected_ranges.is_empty()
        || !cursor_cols.is_empty()
        || !diagnostics.is_empty()
        || local_bracket_col.is_some();

    // ── Fast path: highlight tokens sin overlays ──
    // Renderizar directamente los tokens coloreados sin char-by-char iteration.
    if !has_overlays {
        if let Some(tokens) = highlight_tokens {
            return render_highlight_tokens_fast(
                tokens,
                text,
                col_offset + text_len,
                is_cursor_line,
                active_line_bg,
                normal_style,
                col_offset,
            );
        }
        // Sin highlight ni overlays — color uniforme
        // CLONE: necesario — text es slice del buffer, Span toma ownership
        return vec![Span::styled(text.to_string(), normal_style)];
    }

    // ── Slow path: char-by-char con overlays + syntax highlight ──
    // Pre-computar lookup de colores de syntax por byte offset.
    // Construye un mapa (byte_offset → foreground_color) de los tokens.
    let syntax_colors: Vec<(usize, Color)> = if let Some(tokens) = highlight_tokens {
        build_syntax_color_ranges(tokens)
    } else {
        Vec::new()
    };

    let mut result: Vec<Span<'a>> = Vec::with_capacity(8);

    // Recopilar byte offsets de char boundaries
    let char_boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();

    if char_boundaries.is_empty() {
        return result;
    }

    let mut current_start = 0;
    let mut current_style = char_style_with_highlight(
        0,
        col_offset,
        &selected_ranges,
        &cursor_cols,
        diagnostics,
        &syntax_colors,
        normal_style,
        selection_style,
        cursor_style,
        diag_error_style,
        diag_warning_style,
        is_cursor_line,
        active_line_bg,
        local_bracket_col,
    );

    for &byte_offset in char_boundaries.iter().skip(1) {
        let style = char_style_with_highlight(
            byte_offset,
            col_offset,
            &selected_ranges,
            &cursor_cols,
            diagnostics,
            &syntax_colors,
            normal_style,
            selection_style,
            cursor_style,
            diag_error_style,
            diag_warning_style,
            is_cursor_line,
            active_line_bg,
            local_bracket_col,
        );

        if style != current_style {
            let segment = &text[current_start..byte_offset];
            if !segment.is_empty() {
                // CLONE: necesario — segment es slice del buffer
                result.push(Span::styled(segment.to_string(), current_style));
            }
            current_start = byte_offset;
            current_style = style;
        }
    }

    // Flush final
    if current_start < text_len {
        let segment = &text[current_start..];
        if !segment.is_empty() {
            // CLONE: necesario — segment es slice del buffer
            result.push(Span::styled(segment.to_string(), current_style));
        }
    }

    result
}

/// Fast path: renderizar tokens de highlight directamente como spans.
///
/// Solo se usa cuando no hay selecciones, cursores secundarios, diagnósticos
/// ni bracket matches. Mucho más eficiente que la iteración char-by-char.
///
/// `line_text`: el texto visible de la línea (después de col_offset).
/// Se usa como fallback para texto no cubierto por tokens (ej: caracteres
/// recién tipeados que aún no fueron re-tokenizados por syntect).
///
/// `col_offset`: byte offset del inicio de `text` en la línea completa.
/// Cuando hay indent guides, el texto empieza después de la zona de
/// indentación, así que los tokens de highlight deben ajustarse.
fn render_highlight_tokens_fast<'a>(
    tokens: &[HighlightToken],
    line_text: &str,
    total_len: usize,
    is_cursor_line: bool,
    active_line_bg: Color,
    normal_style: Style,
    col_offset: usize,
) -> Vec<Span<'a>> {
    let mut result = Vec::with_capacity(tokens.len() + 1);
    let mut consumed: usize = 0;

    let bg = if is_cursor_line {
        active_line_bg
    } else {
        // Extraer bg del normal_style — mantener consistencia
        normal_style.bg.unwrap_or(Color::Reset)
    };

    for token in tokens {
        if consumed >= total_len {
            break;
        }
        let token_text = &token.text;
        if token_text.is_empty() {
            continue;
        }

        let token_end = consumed + token_text.len();

        // Si el token termina antes del col_offset, saltarlo
        if token_end <= col_offset {
            consumed = token_end;
            continue;
        }

        // Si el token empieza antes del col_offset, recortar el inicio
        let start_in_token = col_offset.saturating_sub(consumed);
        let display_full = &token_text[start_in_token..];

        // Truncar al espacio restante
        let available = total_len.saturating_sub(consumed + start_in_token);
        let display = if display_full.len() > available {
            crate::ui::truncate_str(display_full, available)
        } else {
            display_full
        };

        if !display.is_empty() {
            let fg = syntect_color_to_ratatui(token.style.foreground);
            let style = Style::default().fg(fg).bg(bg);
            // CLONE: necesario — display puede ser slice del cache, Span toma ownership
            result.push(Span::styled(display.to_string(), style));
        }
        consumed = token_end;
    }

    // ── Tail fallback: texto no cubierto por tokens ──
    // Si los tokens cacheados no cubren todo el texto visible (ej: carácter
    // recién tipeado al final de la línea), renderizar el resto con estilo
    // neutro. Esto evita que texto nuevo sea invisible durante el debounce.
    let covered_in_text = consumed.saturating_sub(col_offset);
    if covered_in_text < line_text.len() {
        let remainder = &line_text[covered_in_text..];
        if !remainder.is_empty() {
            let tail_style = Style::default()
                .fg(normal_style.fg.unwrap_or(Color::Reset))
                .bg(bg);
            // CLONE: necesario — remainder es slice del buffer, Span toma ownership
            result.push(Span::styled(remainder.to_string(), tail_style));
        }
    }

    result
}

/// Construye un mapa de rangos (byte_offset_start, fg_color) desde tokens de highlight.
///
/// Cada entrada indica: "desde este byte offset, el foreground es este color".
/// Se usa para lookup rápido en la iteración char-by-char.
fn build_syntax_color_ranges(tokens: &[HighlightToken]) -> Vec<(usize, Color)> {
    let mut ranges = Vec::with_capacity(tokens.len());
    let mut offset = 0;

    for token in tokens {
        if !token.text.is_empty() {
            let fg = syntect_color_to_ratatui(token.style.foreground);
            ranges.push((offset, fg));
            offset += token.text.len();
        }
    }

    ranges
}

/// Busca el color de syntax para una posición (byte offset) dada.
///
/// Busca el último rango cuyo offset_start <= col. O(n) pero
/// los tokens por línea son pocos (típicamente < 20).
fn syntax_fg_at(col: usize, syntax_colors: &[(usize, Color)]) -> Option<Color> {
    // Buscar el último rango que empieza en o antes de col
    let mut result = None;
    for &(start, color) in syntax_colors {
        if start <= col {
            result = Some(color);
        } else {
            break;
        }
    }
    result
}

/// Determina el estilo compuesto de un carácter en una columna dada.
///
/// Prioridad de capas:
/// 1. **Cursor secundario** (REVERSED) — máxima prioridad
/// 2. **Bracket match** (accent + bold) — resaltado de bracket par
/// 3. **Selección** — override de background, mantiene fg de syntax
/// 4. **Diagnóstico** — agrega underline al estilo base
/// 5. **Syntax highlight** — foreground del token
/// 6. **Normal** — estilo base (fg_primary + bg_primary/active_line)
///
/// `col`: byte offset dentro del texto local (puede empezar en `col_offset`).
/// `col_offset`: offset de la zona de indentación para ajustar syntax lookups.
/// `local_bracket_col`: columna local del bracket match (si existe) con su estilo.
#[expect(
    clippy::too_many_arguments,
    reason = "render helper — pasar struct de estilos agregaría complejidad sin beneficio"
)]
fn char_style_with_highlight(
    col: usize,
    col_offset: usize,
    selected_ranges: &[(usize, usize)],
    cursor_cols: &[usize],
    diagnostics: &[(usize, usize, &lsp::DiagnosticSeverity)],
    syntax_colors: &[(usize, Color)],
    normal_style: Style,
    selection_style: Style,
    cursor_style: Style,
    diag_error_style: Style,
    diag_warning_style: Style,
    is_cursor_line: bool,
    active_line_bg: Color,
    local_bracket_col: Option<(usize, Style)>,
) -> Style {
    // Cursor secundario tiene prioridad visual máxima
    if cursor_cols.contains(&col) {
        return cursor_style;
    }

    // Bracket match: prioridad alta (debajo de cursor secundario)
    if let Some((bracket_col, bstyle)) = local_bracket_col
        && col == bracket_col
    {
        return bstyle;
    }

    // Base: syntax highlight o normal
    // Ajustar col con col_offset para buscar en syntax_colors (que usa offsets absolutos)
    let abs_col = col + col_offset;
    let base_fg = syntax_fg_at(abs_col, syntax_colors);
    let base_bg = if is_cursor_line {
        active_line_bg
    } else {
        normal_style.bg.unwrap_or(Color::Reset)
    };
    let mut style = if let Some(fg) = base_fg {
        Style::default().fg(fg).bg(base_bg)
    } else {
        normal_style
    };

    // Selección: override background, mantener foreground (syntax)
    for &(start, end) in selected_ranges {
        if col >= start && col < end {
            style = style.bg(selection_style.bg.unwrap_or(Color::Reset));
            return style;
        }
    }

    // Diagnóstico: agregar underline al estilo existente
    for &(start, end, severity) in diagnostics {
        if col >= start && col < end {
            match severity {
                lsp::DiagnosticSeverity::Error => {
                    return style
                        .fg(diag_error_style.fg.unwrap_or(Color::Reset))
                        .add_modifier(Modifier::UNDERLINED);
                }
                lsp::DiagnosticSeverity::Warning => {
                    return style
                        .fg(diag_warning_style.fg.unwrap_or(Color::Reset))
                        .add_modifier(Modifier::UNDERLINED);
                }
                _ => {}
            }
        }
    }

    style
}

// ─── Tab Bar ───────────────────────────────────────────────────────────────────

/// Renderiza la barra de tabs del editor.
///
/// Cada tab muestra: `│ Rs filename.ext ● │` con icono por extensión.
/// Tab activa: background `bg_active`, texto `fg_accent`.
/// Tabs inactivas: background `bg_secondary`, texto `fg_secondary`.
/// `●` (U+25CF) en `fg_warning` cuando dirty.
/// `×` (U+00D7) solo en tab activa.
/// Si las tabs no caben, se trunca con `…` al final.
///
/// No aloca strings innecesarios — los nombres vienen pre-computados en `TabInfo`.
fn render_tab_bar(f: &mut Frame, area: Rect, theme: &Theme, tabs: &[TabInfo]) {
    use crate::ui::icons;

    if area.width == 0 || tabs.is_empty() {
        return;
    }

    let max_width = area.width as usize;
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(tabs.len() * 5);
    let mut used_width: usize = 0;
    let mut truncated = false;

    for tab in tabs {
        // Obtener icono para esta tab
        let icon = icons::file_icon(&tab.name);
        let icon_color = icons::icon_color(&tab.name);

        // Calcular ancho de esta tab: "│ " + icon(2) + " " + name + " ●" o " ×" + " "
        // Indicador: dirty → " ●", activa → " ×", limpia+inactiva → nada
        let indicator = if tab.is_dirty {
            " \u{25CF}" // " ●"
        } else if tab.is_active {
            " \u{00D7}" // " ×"
        } else {
            ""
        };
        // "│ " (2) + icon(2) + " "(1) + name.len() + indicator.len() + " " (1 padding)
        let tab_width = 2 + icon.len() + 1 + tab.name.len() + indicator.len() + 1;

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
        // Icono con color semántico
        spans.push(Span::styled(icon, Style::default().fg(icon_color).bg(bg)));
        spans.push(Span::styled(" ", Style::default().bg(bg)));
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

// ─── Breadcrumbs ───────────────────────────────────────────────────────────────

/// Renderiza la barra de breadcrumbs debajo de las tabs.
///
/// Muestra el path relativo al workspace root desglosado en segmentos
/// separados por `>`. El último segmento (archivo) incluye su icono
/// con color semántico. Los segmentos intermedios (directorios) se
/// muestran en `fg_secondary`. El separador `>` en color dimmed.
///
/// Si no hay archivo abierto, renderiza una fila vacía con el background
/// de breadcrumbs. No aloca `format!()` — construye spans directamente.
fn render_breadcrumbs(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    file_path: Option<&Path>,
    workspace_root: Option<&Path>,
) {
    use crate::ui::icons;

    // Background de breadcrumbs: ligeramente diferente al editor
    let breadcrumb_bg = theme.bg_secondary;

    if area.width == 0 {
        return;
    }

    let Some(file_path) = file_path else {
        // Sin archivo — fila vacía con background
        let empty = Paragraph::new(Line::from(Span::styled(
            "",
            Style::default().bg(breadcrumb_bg),
        )))
        .style(Style::default().bg(breadcrumb_bg));
        f.render_widget(empty, area);
        return;
    };

    // Calcular path relativo al workspace root
    let relative = workspace_root
        .and_then(|root| file_path.strip_prefix(root).ok())
        .unwrap_or(file_path);

    // Separar en componentes del path
    let components: Vec<&str> = relative
        .iter()
        .filter_map(|c| c.to_str())
        .collect();

    if components.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "",
            Style::default().bg(breadcrumb_bg),
        )))
        .style(Style::default().bg(breadcrumb_bg));
        f.render_widget(empty, area);
        return;
    }

    let separator_style = Style::default()
        .fg(theme.border_unfocused)
        .bg(breadcrumb_bg);
    let dir_style = Style::default()
        .fg(theme.fg_secondary)
        .bg(breadcrumb_bg);

    let last_idx = components.len() - 1;
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(components.len() * 3 + 1);

    // Padding izquierdo
    spans.push(Span::styled(" ", Style::default().bg(breadcrumb_bg)));

    for (i, component) in components.iter().enumerate() {
        if i == last_idx {
            // Último segmento: archivo con icono + color accent
            let icon = icons::file_icon(component);
            let icon_color = icons::icon_color(component);
            spans.push(Span::styled(
                icon,
                Style::default().fg(icon_color).bg(breadcrumb_bg),
            ));
            spans.push(Span::styled(" ", Style::default().bg(breadcrumb_bg)));
            spans.push(Span::styled(
                *component,
                Style::default()
                    .fg(theme.fg_primary)
                    .bg(breadcrumb_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            // Segmentos intermedios: directorios en color secundario
            spans.push(Span::styled(*component, dir_style));
            // Separador ` > `
            spans.push(Span::styled(" \u{203A} ", separator_style));
        }
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(breadcrumb_bg));
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
            data.git_status,
            Style::default().fg(theme.fg_accent_alt).bg(theme.bg_status),
        ),
    ];

    // Color naranja/amber para el bloque de posición — igual a las imágenes de referencia
    let amber_bg = ratatui::style::Color::Rgb(229, 165, 10); // #e5a50a
    let amber_fg = ratatui::style::Color::Rgb(15, 15, 15);   // casi negro — alta legibilidad

    let right_spans = vec![
        Span::styled(
            data.encoding,
            Style::default().fg(theme.fg_secondary).bg(theme.bg_status),
        ),
        Span::styled("  ", Style::default().bg(theme.bg_status)),
        // Bloque naranja: "352:34  18%"
        Span::styled(" ", Style::default().bg(amber_bg)),
        Span::styled(
            data.cursor_pos,
            Style::default()
                .fg(amber_fg)
                .bg(amber_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default().bg(amber_bg)),
        Span::styled(
            data.scroll_pct,
            Style::default()
                .fg(amber_fg)
                .bg(amber_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default().bg(amber_bg)),
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
