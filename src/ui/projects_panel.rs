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

    // Layout: [+] button (1) + separator (1) + list (rest) + footer (1)
    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // [+] nuevo proyecto
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // lista de proyectos
            Constraint::Length(1), // footer
        ])
        .split(inner);

    // [+] Nuevo proyecto
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

    // Separador — pre-computado a longitud fija, sin format!()
    render_separator(f, sections[1], theme);

    // Lista de proyectos
    let list_area = sections[2];
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
        let lines: Vec<Line<'_>> = state
            .projects
            .iter()
            .skip(state.scroll_offset)
            .take(visible)
            .enumerate()
            .map(|(i, project)| {
                let idx = state.scroll_offset + i;
                let is_selected = idx == state.selected;
                let is_active = state.active_project == Some(idx);

                let bg = if is_selected {
                    theme.bg_active
                } else {
                    theme.bg_secondary
                };

                let lock_icon = if project.locked { "L" } else { "U" };
                let sel_indicator = if is_selected { "> " } else { "  " };
                let active_dot = if is_active { "* " } else { "  " };

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

                Line::from(vec![
                    Span::styled(sel_indicator, Style::default().fg(theme.fg_accent).bg(bg)),
                    Span::styled(active_dot, Style::default().fg(theme.diff_add).bg(bg)),
                    Span::styled(lock_icon, Style::default().fg(theme.fg_secondary).bg(bg)),
                    Span::styled(" ", Style::default().bg(bg)),
                    Span::styled(project.name.as_str(), name_style),
                ])
            })
            .collect();

        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary)),
            list_area,
        );
    }

    // Footer
    let footer_text = if focused {
        " [Enter] Abrir  [L] Candado  [D] Eliminar  [+] Nuevo"
    } else {
        " PROJECTS"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer_text,
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_active)),
        sections[3],
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

    if inner.height < 3 {
        return;
    }

    // Layout: current path (1) + separator (1) + tree list (fill) + footer (1)
    let sections = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // ruta actual
            Constraint::Length(1), // separador
            Constraint::Fill(1),   // arbol
            Constraint::Length(1), // footer
        ])
        .split(inner);

    // Ruta actual — usar as_str directamente, sin format!()
    let root_display = state.current_root.to_string_lossy();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" >> ", Style::default().fg(theme.fg_accent)),
            Span::styled(
                root_display.as_ref(),
                Style::default()
                    .fg(theme.fg_primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        sections[0],
    );

    // Separador
    render_separator(f, sections[1], theme);

    // Árbol de directorios
    let tree_area = sections[2];
    let max_visible = tree_area.height as usize;
    let visible_entries: Vec<Line<'_>> = state
        .entries
        .iter()
        .skip(state.scroll_offset)
        .take(max_visible)
        .enumerate()
        .map(|(i, entry)| {
            let idx = state.scroll_offset + i;
            let is_selected = idx == state.selected;
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

    // Footer
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " [Up/Dn] Nav  [Enter] Expand  [S] Select  [BS] Parent  [Esc] Cancel",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_active)),
        sections[3],
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
