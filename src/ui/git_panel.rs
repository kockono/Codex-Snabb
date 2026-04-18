//! Git Panel: renderizado del panel de source control en la sidebar.
//!
//! Se muestra cuando `GitState::visible` es `true`, reemplazando el explorer
//! en la sidebar (prioridad: search > git > explorer).
//!
//! Layout:
//! - Branch name arriba
//! - Sección "Staged Changes" con conteo
//! - Sección "Changes" con conteo
//! - Input de commit (si commit_mode)
//! - Diff viewer (si show_diff)
//!
//! Reglas de render:
//! - Sin `format!()` dentro de loops
//! - Sin allocaciones innecesarias
//! - Viewport virtual para listas
//! - Datos pre-computados desde `GitState`

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::git::commands::FileChangeType;
use crate::git::GitState;
use crate::ui::theme::Theme;

/// Renderiza el panel de Git / source control dentro de la sidebar.
pub fn render_git_panel(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme, focused: bool) {
    // Bloque exterior con estilo de foco
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

    let block = Block::default()
        .title(Line::from(Span::styled("SOURCE CONTROL", title_style)))
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if !state.is_repo {
        let p = Paragraph::new(Line::from(Span::styled(
            "  Not a git repository",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(p, inner);
        return;
    }

    // Si show_diff, renderizar vista de diff
    if state.show_diff {
        render_diff_view(f, inner, state, theme);
        return;
    }

    // Calcular alturas: branch(1) + commit(si aplica) + resto para archivos
    let commit_height: u16 = if state.commit_mode { 2 } else { 0 };
    let min_file_height: u16 = 1;

    if inner.height < 1 + commit_height + min_file_height {
        // Espacio insuficiente — solo branch
        render_branch_line(f, inner, state, theme);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // branch
            Constraint::Fill(1),               // archivos
            Constraint::Length(commit_height), // commit input (si aplica)
        ])
        .split(inner);

    render_branch_line(f, layout[0], state, theme);
    render_file_list(f, layout[1], state, theme);

    if state.commit_mode {
        render_commit_input(f, layout[2], state, theme);
    }
}

/// Renderiza la línea del branch actual.
fn render_branch_line(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    let branch_display = if state.branch.is_empty() {
        "(detached)"
    } else {
        &state.branch
    };

    let line = Line::from(vec![
        Span::styled(
            " \u{e0a0} ", // nerd font branch icon (fallback: usamos texto plano)
            Style::default()
                .fg(theme.fg_accent_alt)
                .bg(theme.bg_secondary),
        ),
        Span::styled(
            branch_display,
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let p = Paragraph::new(line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza la lista de archivos con secciones staged/unstaged.
fn render_file_list(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme) {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return;
    }

    if state.files.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "  No changes",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(p, area);
        return;
    }

    // Separar archivos en staged y unstaged para display
    let staged_count = state.files.iter().filter(|f| f.staged).count();
    let unstaged_count = state.files.iter().filter(|f| !f.staged).count();

    // Construir líneas de display: headers + archivos
    // Pre-computar fuera del render loop
    let mut display_lines: Vec<Line<'_>> = Vec::with_capacity(state.files.len() + 4);
    let mut file_to_display: Vec<usize> = Vec::with_capacity(state.files.len());

    // Buffer reutilizable para conteos
    let mut count_buf = String::with_capacity(16);

    // Sección staged
    if staged_count > 0 {
        count_buf.clear();
        {
            use std::fmt::Write;
            let _ = write!(count_buf, " Staged Changes ({staged_count})");
        }
        display_lines.push(Line::from(Span::styled(
            // CLONE: necesario — count_buf se reutiliza, Span toma ownership
            count_buf.clone(),
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD),
        )));
        // Índice especial para header — no corresponde a archivo
        file_to_display.push(usize::MAX);

        for (i, file) in state.files.iter().enumerate() {
            if !file.staged {
                continue;
            }
            let selected = i == state.selected_index;
            display_lines.push(render_file_entry(
                file,
                selected,
                area.width as usize,
                theme,
            ));
            file_to_display.push(i);
        }
    }

    // Sección unstaged
    if unstaged_count > 0 {
        count_buf.clear();
        {
            use std::fmt::Write;
            let _ = write!(count_buf, " Changes ({unstaged_count})");
        }
        display_lines.push(Line::from(Span::styled(
            // CLONE: necesario — count_buf se reutiliza, Span toma ownership
            count_buf.clone(),
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD),
        )));
        file_to_display.push(usize::MAX);

        for (i, file) in state.files.iter().enumerate() {
            if file.staged {
                continue;
            }
            let selected = i == state.selected_index;
            display_lines.push(render_file_entry(
                file,
                selected,
                area.width as usize,
                theme,
            ));
            file_to_display.push(i);
        }
    }

    // Viewport virtual
    let scroll = state
        .scroll_offset
        .min(display_lines.len().saturating_sub(1));
    let visible: Vec<Line<'_>> = display_lines
        .into_iter()
        .skip(scroll)
        .take(visible_height)
        .collect();

    let p = Paragraph::new(visible).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza una entrada de archivo en la lista.
///
/// Formato: `  [indicador] X  path/to/file`
/// donde X es la letra de status con color semántico.
fn render_file_entry<'a>(
    file: &crate::git::commands::GitFileStatus,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    let indicator = if selected { " \u{25b8} " } else { "   " };
    let indicator_style = Style::default()
        .fg(theme.fg_accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    // Letra y color según tipo de cambio
    let (status_char, status_color) = match file.status {
        FileChangeType::Modified => ("M", theme.fg_accent_alt), // amarillo/magenta
        FileChangeType::Added => ("A", theme.diff_add),         // verde
        FileChangeType::Deleted => ("D", theme.diff_remove),    // rojo
        FileChangeType::Renamed => ("R", theme.fg_accent),      // cyan
        FileChangeType::Untracked => ("?", theme.fg_secondary), // gris
        FileChangeType::Copied => ("C", theme.fg_accent),       // cyan
    };

    let status_style = Style::default().fg(status_color).bg(bg);
    let path_style = Style::default().fg(theme.fg_primary).bg(bg);

    // Truncar path al ancho disponible
    let prefix_len = indicator.len() + 2 + 1; // indicator + "X " + espacio
    let path_max = max_width.saturating_sub(prefix_len);
    let display_path = if file.path.len() > path_max {
        &file.path[..path_max]
    } else {
        &file.path
    };

    Line::from(vec![
        Span::styled(indicator, indicator_style),
        Span::styled(status_char, status_style),
        Span::styled("  ", Style::default().bg(bg)),
        // CLONE: necesario — display_path es slice del GitFileStatus.path,
        // Span necesita ownership para mantener el contenido vivo
        Span::styled(display_path.to_string(), path_style),
    ])
}

/// Renderiza el input de commit message.
fn render_commit_input(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    let label_style = Style::default()
        .fg(theme.fg_accent)
        .bg(theme.bg_secondary)
        .add_modifier(Modifier::BOLD);

    let input_style = Style::default().fg(theme.fg_primary).bg(theme.bg_secondary);

    let cursor_style = Style::default()
        .fg(theme.fg_accent)
        .bg(theme.bg_secondary)
        .add_modifier(Modifier::SLOW_BLINK);

    let lines = vec![
        Line::from(Span::styled(" Commit:", label_style)),
        Line::from(vec![
            Span::styled(" ", Style::default().bg(theme.bg_secondary)),
            Span::styled(state.commit_input.as_str(), input_style),
            Span::styled("_", cursor_style),
        ]),
    ];

    let p = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, area);
}

/// Renderiza la vista de diff del archivo seleccionado.
fn render_diff_view(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme) {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return;
    }

    // Título del diff
    let diff_title = state
        .files
        .get(state.selected_index)
        .map(|f| f.path.as_str())
        .unwrap_or("(unknown)");

    // Header: 1 línea para título
    if area.height < 2 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // título
            Constraint::Fill(1),   // contenido del diff
        ])
        .split(area);

    // Título
    let title_line = Line::from(vec![
        Span::styled(
            " DIFF: ",
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            diff_title,
            Style::default().fg(theme.fg_primary).bg(theme.bg_secondary),
        ),
        Span::styled(
            " (Esc/d to close)",
            Style::default()
                .fg(theme.fg_secondary)
                .bg(theme.bg_secondary),
        ),
    ]);
    let title_p = Paragraph::new(title_line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(title_p, layout[0]);

    // Contenido del diff
    let diff_height = layout[1].height as usize;
    let Some(ref diff_content) = state.diff_content else {
        let p = Paragraph::new(Line::from(Span::styled(
            "  Loading diff...",
            Style::default().fg(theme.fg_secondary),
        )))
        .style(Style::default().bg(theme.bg_secondary));
        f.render_widget(p, layout[1]);
        return;
    };

    let max_width = layout[1].width as usize;
    let diff_lines: Vec<Line<'_>> = diff_content
        .lines()
        .skip(state.diff_scroll)
        .take(diff_height)
        .map(|line| render_diff_line(line, max_width, theme))
        .collect();

    let p = Paragraph::new(diff_lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(p, layout[1]);
}

/// Renderiza una línea de diff con colores semánticos.
///
/// - Líneas con `+` → verde (diff_add)
/// - Líneas con `-` → rojo (diff_remove)
/// - Headers `@@` → cyan (fg_accent)
/// - Resto → texto normal
fn render_diff_line<'a>(line: &str, max_width: usize, theme: &'a Theme) -> Line<'a> {
    let display = if line.len() > max_width {
        &line[..max_width]
    } else {
        line
    };

    let style = if display.starts_with('+') {
        Style::default().fg(theme.diff_add).bg(theme.bg_secondary)
    } else if display.starts_with('-') {
        Style::default()
            .fg(theme.diff_remove)
            .bg(theme.bg_secondary)
    } else if display.starts_with("@@") {
        Style::default()
            .fg(theme.fg_accent)
            .bg(theme.bg_secondary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_primary).bg(theme.bg_secondary)
    };

    // CLONE: necesario — display es un slice del diff_content, Span toma ownership
    Line::from(Span::styled(display.to_string(), style))
}
