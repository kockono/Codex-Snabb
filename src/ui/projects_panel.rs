//! Projects panel: lista de workspaces + folder picker modal.
//!
//! Stateless renderers — reciben datos pre-computados y dibujan.
//! Sin allocaciones en hot paths excepto las líneas de display
//! (que se construyen con capacidad pre-estimada).

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::ui::layout::{self, IdeLayout};
use crate::ui::theme::Theme;
use crate::workspace::folder_picker::FolderPickerState;
use crate::workspace::projects::ProjectsState;

/// Renderiza el panel de proyectos en la sidebar.
pub fn render_projects_panel(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    state: &ProjectsState,
    focused: bool,
    cursor_visible: bool,
) {
    let border_color = if focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };
    let title_style = if focused {
        Style::default()
            .fg(theme.fg_accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_secondary)
    };

    let block = Block::default()
        .title(Line::from(Span::styled(" PROJECTS ", title_style)))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Double
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    // ── Pre-computar todo ANTES del render (sin allocaciones en render loop) ──

    // Determinar si hay error activo para el layout condicional
    let has_error = state.path_input_error;

    // Layout dinámico: [+] nuevo (1) + path input (1) + error? (0|1) + separator (1) + list (fill) + footer (1)
    let constraints: Vec<Constraint> = if has_error {
        vec![
            Constraint::Length(1), // [+] Nuevo proyecto
            Constraint::Length(1), // [+] path input inline
            Constraint::Length(1), // error "Ruta no válida"
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // lista de proyectos
            Constraint::Length(1), // footer
        ]
    } else {
        vec![
            Constraint::Length(1), // [+] Nuevo proyecto
            Constraint::Length(1), // [+] path input inline
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // lista de proyectos
            Constraint::Length(1), // footer
        ]
    };

    // Índices dinámicos de cada sección
    let error_idx: usize = if has_error { 2 } else { usize::MAX }; // sentinel si no hay error
    let separator_idx: usize = if has_error { 3 } else { 2 };
    let list_idx: usize = if has_error { 4 } else { 3 };
    let footer_idx: usize = if has_error { 5 } else { 4 };

    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // ── Fila 0: [+] Nuevo proyecto ──
    let add_style = Style::default()
        .fg(theme.fg_accent)
        .add_modifier(Modifier::BOLD);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [+] ", add_style),
            Span::styled("Nuevo proyecto", Style::default().fg(theme.fg_secondary)),
        ])),
        sections[0],
    );

    // ── Fila 1: [+] path input inline ──
    // Pre-computar bg y estilos FUERA del render
    let input_bg = if state.path_input_focused {
        theme.bg_active
    } else {
        theme.bg_secondary
    };
    let is_placeholder = state.path_input.is_empty();
    let input_text: &str = if is_placeholder {
        "Agregar por ruta..."
    } else {
        state.path_input.as_str()
    };
    let text_style = if is_placeholder {
        Style::default()
            .fg(theme.fg_secondary)
            .bg(input_bg)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(theme.fg_primary).bg(input_bg)
    };
    // Cursor: "|" visible/oculto por cursor_visible (sistema de blink del app state)
    let cursor_indicator = if state.path_input_focused && cursor_visible {
        "|"
    } else {
        ""
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [+] ",
                Style::default()
                    .fg(theme.fg_accent)
                    .bg(input_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(input_text, text_style),
            Span::styled(
                cursor_indicator,
                Style::default().fg(theme.fg_accent).bg(input_bg),
            ),
        ]))
        .style(Style::default().bg(input_bg)),
        sections[1],
    );

    // ── Fila 2 (condicional): error "Ruta no válida" ──
    if has_error {
        // has_error garantiza que error_idx != usize::MAX y sections[error_idx] es válido
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("   ", Style::default().bg(theme.bg_secondary)),
                Span::styled(
                    "\u{2717} Ruta no v\u{e1}lida",
                    Style::default()
                        .fg(theme.diff_remove)
                        .bg(theme.bg_secondary),
                ),
            ]))
            .style(Style::default().bg(theme.bg_secondary)),
            sections[error_idx],
        );
    }

    // ── Separador ──
    render_separator(f, sections[separator_idx], theme);

    // ── Lista de proyectos ──
    let list_area = sections[list_idx];
    if list_area.height == 0 || state.projects.is_empty() {
        // Empty state
        let msg = "Sin proyectos guardados";
        let empty_y = list_area.y + list_area.height / 2;
        if empty_y < list_area.y + list_area.height {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    msg,
                    Style::default().fg(theme.fg_secondary),
                ))),
                Rect::new(
                    list_area.x + (list_area.width.saturating_sub(msg.len() as u16)) / 2,
                    empty_y,
                    msg.len() as u16,
                    1,
                ),
            );
        }
    } else {
        let visible = (list_area.height as usize)
            .min(state.projects.len().saturating_sub(state.scroll_offset));

        // Pre-computar anchos FUERA del loop — constantes para toda la lista
        // Prefijo: sel_indicator(2) + active_dot(2) + lock_icon(1) + " "(1) = 6 chars
        // Botón:   " [x]" = 4 chars
        let list_width = list_area.width as usize;
        let prefix_width: usize = 6;
        let delete_btn_width: usize = 4; // " [x]"
        let name_max = list_width.saturating_sub(prefix_width + delete_btn_width);

        // Usar for loop en lugar de .map().collect() para poder tener
        // variables locales (pad_str: String) que viven el tiempo necesario
        // para ser movidas al Span antes del push.
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible);
        for i in 0..visible {
            let idx = state.scroll_offset + i;
            let project = &state.projects[idx];
            let is_selected = idx == state.selected;
            let is_active = state.active_project == Some(idx);

            let bg = if is_selected {
                theme.bg_active
            } else {
                theme.bg_secondary
            };

            let lock_icon: &str = if project.locked { "L" } else { "U" };
            let sel_indicator: &str = if is_selected { "> " } else { "  " };
            let active_dot: &str = if is_active { "* " } else { "  " };

            let name_style = if is_active {
                Style::default()
                    .fg(theme.fg_accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(theme.fg_primary)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_primary).bg(bg)
            };

            // Truncar nombre al ancho disponible y rellenar con espacios
            // para que " [x]" quede siempre en el borde derecho.
            // Una sola alloc de String por fila visible — fuera del hot render path.
            let name_str = project.name.as_str();
            let name_display_len = name_str.len().min(name_max);
            let pad = name_max.saturating_sub(name_display_len);

            // name_padded: String owned — se mueve al Span (Cow::Owned).
            // pad se conoce en tiempo de ejecución: usamos with_capacity + extend.
            let mut name_padded = String::with_capacity(name_display_len + pad);
            name_padded.push_str(&name_str[..name_display_len]);
            for _ in 0..pad {
                name_padded.push(' ');
            }

            lines.push(Line::from(vec![
                Span::styled(sel_indicator, Style::default().fg(theme.fg_accent).bg(bg)),
                Span::styled(active_dot, Style::default().fg(theme.diff_add).bg(bg)),
                Span::styled(lock_icon, Style::default().fg(theme.fg_secondary).bg(bg)),
                Span::styled(" ", Style::default().bg(bg)),
                // name_padded: String moved into Cow::Owned — sin borrow issues
                Span::styled(name_padded, name_style),
                // Botón eliminar: " [x]" en diff_remove (rojo) para visibilidad
                Span::styled(" [x]", Style::default().fg(theme.diff_remove).bg(bg)),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary)),
            list_area,
        );
    }

    // ── Footer contextual ──
    let footer_text = if state.path_input_focused {
        " [Enter] Agregar  [Esc] Cancelar"
    } else if focused {
        " [Enter] Abrir  [L] Candado  [D] Eliminar"
    } else {
        " PROJECTS"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer_text,
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_active)),
        sections[footer_idx],
    );
}

/// Renderiza el folder picker modal.
pub fn render_folder_picker(
    f: &mut Frame,
    layout: &IdeLayout,
    theme: &Theme,
    state: &FolderPickerState,
) {
    if !state.visible {
        return;
    }

    let modal_height = 24u16;
    let overlay_rect = layout::modal_rect(layout, modal_height);

    f.render_widget(Clear, overlay_rect);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " * ",
                Style::default()
                    .fg(theme.fg_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Seleccionar carpeta ",
                Style::default().fg(theme.fg_primary),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_rect);
    f.render_widget(block, overlay_rect);

    if inner.height < 4 {
        return;
    }

    // Pre-compute: ¿hay error de path? Si sí, necesitamos una fila extra.
    let has_error = state.path_error.is_some();

    // Layout: path input (1) + error? (0|1) + separator (1) + tree list (fill) + footer (1)
    let constraints = if has_error {
        vec![
            Constraint::Length(1), // path input
            Constraint::Length(1), // error message
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // arbol
            Constraint::Length(1), // footer
        ]
    } else {
        vec![
            Constraint::Length(1), // path input
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // arbol
            Constraint::Length(1), // footer
        ]
    };

    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Índices dinámicos según presencia de error
    let path_input_idx = 0;
    let error_idx: usize = if has_error { 1 } else { usize::MAX }; // sentinel si no hay error
    let separator_idx: usize = if has_error { 2 } else { 1 };
    let tree_idx: usize = if has_error { 3 } else { 2 };
    let footer_idx: usize = if has_error { 4 } else { 3 };

    // ── Path input row ──
    // Pre-compute display text y cursor indicator FUERA del render
    let input_bg = if state.path_input_focused {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    let display_text = state.display_text();
    let is_placeholder = state.path_input.is_empty();

    let text_style = if state.path_input_focused && !is_placeholder {
        // Texto activamente escrito por el usuario
        Style::default()
            .fg(theme.fg_primary)
            .bg(input_bg)
            .add_modifier(Modifier::BOLD)
    } else if is_placeholder {
        // Placeholder (current_root) en dim
        Style::default().fg(theme.fg_secondary).bg(input_bg)
    } else {
        Style::default().fg(theme.fg_primary).bg(input_bg)
    };

    let cursor_indicator = if state.path_input_focused { "_" } else { "" };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Path: ", Style::default().fg(theme.fg_accent).bg(input_bg)),
            Span::styled(display_text, text_style),
            Span::styled(
                cursor_indicator,
                Style::default()
                    .fg(theme.fg_accent)
                    .bg(input_bg)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]))
        .style(Style::default().bg(input_bg)),
        sections[path_input_idx],
    );

    // ── Error message (si existe) ──
    if has_error {
        // has_error is derived from path_error.is_some() — safe to index error section
        let err_msg = state.path_error.as_deref().unwrap_or("");
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("   ", Style::default().bg(theme.bg_secondary)),
                Span::styled(
                    err_msg,
                    Style::default()
                        .fg(theme.diff_remove)
                        .bg(theme.bg_secondary),
                ),
            ]))
            .style(Style::default().bg(theme.bg_secondary)),
            sections[error_idx],
        );
    }

    // Separador
    render_separator(f, sections[separator_idx], theme);

    // Árbol de directorios
    let tree_area = sections[tree_idx];
    let max_visible = tree_area.height as usize;
    let visible_entries: Vec<Line<'_>> = state
        .entries
        .iter()
        .skip(state.scroll_offset)
        .take(max_visible)
        .enumerate()
        .map(|(i, entry)| {
            let idx = state.scroll_offset + i;
            let is_selected = idx == state.selected && !state.path_input_focused;
            let bg = if is_selected {
                theme.bg_active
            } else {
                theme.bg_secondary
            };

            // Pre-compute indent — "  " per depth level, no allocations for known depths
            let indent_width = entry.depth * 2;
            let icon = if !entry.is_dir {
                "  "
            } else if entry.expanded {
                "v "
            } else {
                "> "
            };

            let name_style = if !entry.is_dir {
                Style::default().fg(theme.fg_secondary).bg(bg)
            } else if is_selected {
                Style::default()
                    .fg(theme.fg_accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_primary).bg(bg)
            };

            // Build indent string — small allocation but outside render loop (this IS the render)
            let indent = " ".repeat(indent_width);
            Line::from(vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(indent, Style::default().bg(bg)),
                Span::styled(icon, Style::default().fg(theme.fg_accent).bg(bg)),
                Span::styled(entry.name.as_str(), name_style),
            ])
        })
        .collect();

    f.render_widget(
        Paragraph::new(visible_entries).style(Style::default().bg(theme.bg_secondary)),
        tree_area,
    );

    // Footer — contextual según foco: input vs árbol
    let footer_text = if state.path_input_focused {
        " [Enter] Ir  [Esc] Cancelar path  [Tab] Al árbol"
    } else {
        " [S] Select  [Tab] Path  [Enter] Expand  [Esc] Cerrar"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer_text,
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_active)),
        sections[footer_idx],
    );
}

/// Renderiza un separador horizontal usando chars repetidos.
/// Evita format!() — usa repeat y trunca al ancho del área.
fn render_separator(f: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 {
        return;
    }
    // "─" es 3 bytes UTF-8 — construir string con repeat
    let sep: String = "\u{2500}".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            sep,
            Style::default().fg(theme.border_unfocused),
        ))),
        area,
    );
}
