//! Git Panel: renderizado del panel de source control en la sidebar.
//!
//! Se muestra cuando `GitState::visible` es `true`, reemplazando el explorer
//! en la sidebar (prioridad: search > git > explorer).
//!
//! Layout (VS Code style — siempre visible):
//! - Branch name arriba (1 línea)
//! - Input de commit — SIEMPRE visible (1 línea)
//! - Botón "✓ Commit" — SIEMPRE visible (1 línea)
//! - Separador (1 línea)
//! - Lista de archivos (resto del espacio)
//! - Diff viewer (si show_diff — reemplaza todo lo anterior)
//!
//! Reglas de render:
//! - Sin `format!()` dentro de loops
//! - Sin allocaciones innecesarias
//! - Viewport virtual para listas
//! - Datos pre-computados desde `GitState`
//! - Sin `Modifier::SLOW_BLINK` — usar `cursor_visible: bool` para blink

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::source_control_git::commands::FileChangeType;
use crate::source_control_git::GitState;
use crate::ui::theme::Theme;

/// Renderiza el panel de Git / source control dentro de la sidebar.
///
/// `cursor_visible` controla el blink del cursor en el input de commit.
/// `true` = cursor visible; `false` = cursor oculto (blink off).
pub fn render_git_panel(
    f: &mut Frame,
    area: Rect,
    state: &GitState,
    theme: &Theme,
    focused: bool,
    cursor_visible: bool,
) {
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

    // Layout VS Code style:
    // branch(1) + input(1) + button(1) + files(fill)
    // Mínimo requerido: 4 líneas para que todo sea visible.
    let min_height: u16 = 4;
    if inner.height < min_height {
        render_branch_line(f, inner, state, theme);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // branch
            Constraint::Length(1), // commit input (siempre visible)
            Constraint::Length(1), // botón [ ✓ Commit ]
            Constraint::Fill(1),   // archivos
        ])
        .split(inner);

    render_branch_line(f, layout[0], state, theme);
    render_commit_input_row(f, layout[1], state, theme, cursor_visible);
    render_commit_button_row(f, layout[2], theme);
    render_file_list(f, layout[3], state, theme);
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
            " \u{e0a0} ", // nerd font branch icon
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

/// Renderiza la fila del input de commit (siempre visible, estilo VS Code).
///
/// - Fondo: `theme.bg_active` para distinguirlo visualmente del file list.
/// - Vacío + sin foco: muestra placeholder `"Message (Ctrl+Enter)..."` en dim.
/// - Con texto o con foco: muestra el texto del input.
/// - Cursor: `"|"` si `commit_mode && cursor_visible`, sino `""`.
fn render_commit_input_row(
    f: &mut Frame,
    area: Rect,
    state: &GitState,
    theme: &Theme,
    cursor_visible: bool,
) {
    if area.height == 0 {
        return;
    }

    let bg = theme.bg_active;

    let (content_span, cursor_span) = if state.commit_input.is_empty() && !state.commit_mode {
        // Placeholder: vacío + sin foco
        let placeholder = Span::styled(
            "  Message (Ctrl+Enter)...",
            Style::default()
                .fg(theme.fg_secondary)
                .bg(bg)
                .add_modifier(Modifier::DIM),
        );
        let cursor = Span::styled("", Style::default().bg(bg));
        (placeholder, cursor)
    } else {
        // Texto real o modo activo
        let text = Span::styled(
            // CLONE: necesario — commit_input es &String, Span necesita owned o 'static
            // Usamos as_str() para obtener &str — no hay clone aquí.
            state.commit_input.as_str(),
            Style::default().fg(theme.fg_primary).bg(bg),
        );
        // Cursor: visible solo cuando commit_mode Y cursor_visible (blink)
        let cursor_str = if state.commit_mode && cursor_visible {
            "|"
        } else {
            ""
        };
        let cursor = Span::styled(cursor_str, Style::default().fg(theme.fg_accent).bg(bg));
        (text, cursor)
    };

    let prefix = Span::styled("  ", Style::default().bg(bg));
    let line = if state.commit_input.is_empty() && !state.commit_mode {
        // Placeholder: no prefix adicional — el placeholder ya tiene 2 espacios
        Line::from(vec![content_span, cursor_span])
    } else {
        Line::from(vec![prefix, content_span, cursor_span])
    };

    let p = Paragraph::new(line).style(Style::default().bg(bg));
    f.render_widget(p, area);
}

/// Renderiza el botón "✓ Commit" en 1 fila con corchetes y gris bajo.
///
/// ```text
///   [ ✓ Commit ]
/// ```
fn render_commit_button_row(f: &mut Frame, area: Rect, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    // Gris bajo sutil — se diferencia del bg sin ser llamativo
    let btn_bg = Color::Rgb(35, 40, 48); // #232830 — gris azulado oscuro
    let btn_fg = Color::Rgb(180, 185, 192); // #b4b9c0 — gris claro legible
    let bracket_fg = Color::Rgb(100, 108, 118); // #646c76 — corchetes más dim

    // Centrar el botón: "[ ✓ Commit ]" = 13 chars visibles
    // pad_left = (area_width - 13) / 2, mínimo 0
    let btn_visible_width = 19usize; // "[ ✓ Commit ]"
    let pad_left = (area.width as usize).saturating_sub(btn_visible_width) / 2;
    let pad_str = " ".repeat(pad_left);

    let line = Line::from(vec![
        Span::styled(pad_str, Style::default().bg(theme.bg_secondary)),
        Span::styled("[    ", Style::default().fg(bracket_fg).bg(btn_bg)),
        Span::styled(
            "\u{2713} Commit",
            Style::default()
                .fg(btn_fg)
                .bg(btn_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("    ]", Style::default().fg(bracket_fg).bg(btn_bg)),
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
        display_lines.push(render_section_header(
            &count_buf,
            Some("[-]"), // unstage all
            area.width,
            theme,
        ));
        // Índice especial para header — no corresponde a archivo
        file_to_display.push(usize::MAX);

        for (i, file) in state.files.iter().enumerate() {
            if !file.staged {
                continue;
            }
            let selected = i == state.selected_index;
            display_lines.push(render_file_entry(file, selected, area.width, theme));
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
        display_lines.push(render_section_header(
            &count_buf,
            Some("[+]"), // stage all
            area.width,
            theme,
        ));
        file_to_display.push(usize::MAX);

        for (i, file) in state.files.iter().enumerate() {
            if file.staged {
                continue;
            }
            let selected = i == state.selected_index;
            display_lines.push(render_file_entry(file, selected, area.width, theme));
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

/// Renderiza un header de sección ("Staged Changes (N)" / "Changes (N)").
///
/// `btn`: botón opcional alineado a la derecha — `Some("[+]")` para stage all,
/// `Some("[-]")` para unstage all, `None` para header sin botón.
fn render_section_header(
    text: &str,
    btn: Option<&'static str>,
    inner_width: u16,
    theme: &Theme,
) -> Line<'static> {
    let bg = theme.bg_secondary;
    let header_style = Style::default()
        .fg(theme.fg_accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    let Some(btn_label) = btn else {
        // Header simple — sin botón
        // CLONE: necesario — Span<'static> requiere ownership del texto
        return Line::from(Span::styled(text.to_string(), header_style));
    };

    // Botón al final — " [X]" = 1 + 3 = 4 chars
    const BTN_LEN: u16 = 4;
    let btn_style = Style::default()
        .fg(if btn_label == "[+]" { theme.fg_warning } else { theme.diff_add })
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    let text_chars = text.chars().count();
    let target = (inner_width as usize).saturating_sub(BTN_LEN as usize);
    let padding = target.saturating_sub(text_chars);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(4);
    // CLONE: necesario — Span<'static>
    spans.push(Span::styled(text.to_string(), header_style));
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg)));
    }
    spans.push(Span::styled(" ", Style::default().bg(bg)));
    spans.push(Span::styled(btn_label, btn_style));

    Line::from(spans)
}

/// Renderiza una entrada de archivo en la lista con botones inline.
///
/// Formato según estado:
/// - Unstaged Modified/Deleted: `M  path/to/file ──── [+] [↺]`  (restore)
/// - Unstaged Added/Untracked:  `A  new_file.rs  ──── [+] [↺]`  (clean -f)
/// - Staged:                    `M  path/to/file ──────────── [-]`
///
/// `[+]` amarillo (stage pendiente),
/// `[-]` verde (ya staged → unstage al click), `[↺]` rojo (discard).
///
/// `inner_width` es el ancho útil del file list area (sin bordes).
/// El selected highlight aplica solo al path, no al botón — los botones
/// mantienen su color semántico para que se distingan visualmente.
fn render_file_entry(
    file: &crate::source_control_git::commands::GitFileStatus,
    is_selected: bool,
    inner_width: u16,
    theme: &Theme,
) -> Line<'static> {
    let bg = if is_selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    // ── Pre-cómputo ──
    let stage_btn = if file.staged { "[-]" } else { "[+]" };

    let (status_char, status_color) = match file.status {
        FileChangeType::Modified => ("M", theme.fg_accent_alt), // magenta
        FileChangeType::Added | FileChangeType::Untracked => ("A", theme.diff_add), // verde
        FileChangeType::Deleted => ("D", theme.diff_remove),    // rojo
        FileChangeType::Renamed => ("R", theme.fg_accent),      // cyan
        FileChangeType::Copied => ("C", theme.fg_accent),       // cyan
    };

    // Todos los archivos unstaged muestran [↺]:
    //   Modified/Deleted → git restore (vuelve al index)
    //   Added/Untracked  → git clean -f (elimina del disco)
    let has_discard = !file.staged;

    // Layout de anchos — todo en chars de display:
    //   "M" (1) + "  " (2) = prefix 3
    //   sufijo (has_discard):  " [+] [↺]" = 8 chars
    //   sufijo (solo stage):   " [+]"     = 4 chars
    const PREFIX_LEN: u16 = 3;
    let suffix_len: u16 = if has_discard { 8 } else { 4 };
    let path_width = inner_width.saturating_sub(PREFIX_LEN + suffix_len) as usize;

    // Truncar path char-safe (multi-byte) usando helper del módulo ui.
    let display_path: String = if path_width == 0 {
        String::new()
    } else {
        crate::ui::truncate_str(&file.path, path_width).to_string()
    };
    let display_path_chars = display_path.chars().count();

    // ── Estilos ──
    let stage_style = if file.staged {
        // [-] = ya staged → click descontará. Verde para indicar "OK ya está".
        Style::default()
            .fg(theme.diff_add)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        // [+] = pendiente de stage. Amarillo para llamar la atención.
        Style::default()
            .fg(theme.fg_warning)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color).bg(bg);
    let path_style = if is_selected {
        Style::default()
            .fg(theme.fg_primary)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_primary).bg(bg)
    };
    let discard_style = Style::default()
        .fg(theme.fg_error)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    // ── Construcción de spans (sin format!() — todo &'static o ya owned) ──
    // Layout final por fila:
    //   has_discard:  "M  path/to/file ──── [+] [↺]"
    //   solo stage:   "M  path/to/file ──── [+]"
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(if has_discard { 7 } else { 5 });

    // status char "M"/"A"/"D"/...
    spans.push(Span::styled(status_char.to_string(), status_style));
    // "  " (dos espacios separadores)
    spans.push(Span::styled("  ", Style::default().bg(bg)));
    // path
    spans.push(Span::styled(display_path, path_style));

    // Padding hasta los botones del final
    let used = PREFIX_LEN as usize + display_path_chars;
    let target = (inner_width as usize).saturating_sub(suffix_len as usize);
    let padding = target.saturating_sub(used);
    if padding > 0 {
        spans.push(Span::styled(
            " ".repeat(padding),
            Style::default().bg(bg),
        ));
    }

    // " [+]" / " [-]" siempre al final
    spans.push(Span::styled(" ", Style::default().bg(bg)));
    spans.push(Span::styled(stage_btn.to_string(), stage_style));

    if has_discard {
        // " [↺]" a la derecha del [+]
        spans.push(Span::styled(" ", Style::default().bg(bg)));
        spans.push(Span::styled("[\u{21ba}]".to_string(), discard_style));
    }

    Line::from(spans)
}

/// Renderiza la vista de diff del archivo seleccionado.
///
/// LEGACY: este overlay global fue reemplazado por tabs virtuales de diff
/// (`EditorState::diff_view`). Se mantiene como referencia hasta el cleanup
/// — `ui::mod.rs` ya no lo llama.
#[expect(
    dead_code,
    reason = "legacy del overlay de diff — reemplazado por panels::render_diff_tab"
)]
pub fn render_diff_view(f: &mut Frame, area: Rect, state: &GitState, theme: &Theme) {
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

    // Mínimo: 1 título + 1 contenido + 1 footer
    if area.height < 3 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // título con nombre del archivo
            Constraint::Fill(1),   // contenido del diff
            Constraint::Length(1), // footer con atajos de teclado
        ])
        .split(area);

    // Título: " DIFF: " o " FILE: " (accent bold) + filename (primary)
    let title_prefix = if state.showing_file_content { " FILE: " } else { " DIFF: " };
    let title_line = Line::from(vec![
        Span::styled(
            title_prefix,
            Style::default()
                .fg(theme.fg_accent)
                .bg(theme.bg_secondary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            diff_title,
            Style::default().fg(theme.fg_primary).bg(theme.bg_secondary),
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
        // Footer incluso cuando no hay contenido
        render_diff_footer(f, layout[2], theme);
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

    // Footer con atajos — pre-computado, sin format!() en render
    render_diff_footer(f, layout[2], theme);
}

/// Renderiza el footer del diff con atajos de teclado.
///
/// Texto estático pre-computado — sin allocaciones en render.
///
/// LEGACY: solo invocada desde `render_diff_view` (también legacy). Cuando
/// el caller está marcado como `dead_code` el lint no dispara aquí, por eso
/// no llevamos `#[expect]` en esta función.
fn render_diff_footer(f: &mut Frame, area: Rect, theme: &Theme) {
    // Texto fijo — &'static str, cero allocaciones
    let footer_line = Line::from(Span::styled(
        " [↑↓/jk] Scroll   [D/Esc] Cerrar",
        Style::default().fg(theme.fg_secondary).bg(theme.bg_active),
    ));
    let p = Paragraph::new(footer_line).style(Style::default().bg(theme.bg_active));
    f.render_widget(p, area);
}

/// Renderiza una línea de diff con colores semánticos.
///
/// - Líneas con `+` → verde (diff_add)
/// - Líneas con `-` → rojo (diff_remove)
/// - Headers `@@` → cyan (fg_accent)
/// - Resto → texto normal
///
/// Visibilidad `pub(crate)` — se reusa desde `ui::panels` para renderizar
/// el contenido de tabs virtuales de diff dentro del área del editor.
pub(crate) fn render_diff_line<'a>(line: &str, max_width: usize, theme: &'a Theme) -> Line<'a> {
    let display = crate::ui::truncate_str(line, max_width);

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
