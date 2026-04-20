//! Mouse: procesamiento de eventos de mouse (click, drag, scroll).
//!
//! Hit-testing contra el layout para resolver en qué panel cayó el evento,
//! y dispatch de acciones contextuales. Extraído de mod.rs.

use ratatui::layout::Rect;
use tokio_util::sync::CancellationToken;

use super::helpers::get_workspace_root;
use super::{process_effects, reduce, AppState};
use crate::core::settings::SidebarSection;
use crate::core::{Action, PanelId};
use crate::ui::layout::IdeLayout;

// ─── Types and constants ───────────────────────────────────────────────────────

/// Dirección de scroll del mouse.
#[derive(Debug, Clone, Copy)]
pub(super) enum ScrollDirection {
    Up,
    Down,
}

/// Cantidad de líneas que el scroll del mouse desplaza por evento (paneles no-editor).
/// 1 línea por evento: scroll suave. El usuario controla la velocidad
/// con la velocidad del wheel (más eventos = más scroll).
const MOUSE_SCROLL_LINES: usize = 1;

/// Scroll del mouse en el editor: 3 líneas por evento.
/// El editor se beneficia de avance más rápido dado que los archivos
/// suelen ser mucho más largos que las listas de explorer/search.
const EDITOR_SCROLL_LINES: usize = 3;

/// Resultado de hit-test que distingue activity bar de paneles normales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HitTestResult {
    /// Click en un panel normal del IDE.
    Panel(PanelId),
    /// Click en la activity bar — incluye la fila relativa dentro de la barra.
    ActivityBar { row_in_bar: u16 },
}

// ─── Pure functions ────────────────────────────────────────────────────────────

/// Calcula el ancho real del gutter (números de línea + separador `│ `).
///
/// Debe coincidir EXACTAMENTE con la lógica de render en `panels.rs`:
/// `digit_count(line_count).max(4)` + 2 (separador).
pub(super) fn editor_gutter_width(line_count: usize) -> u16 {
    let digits = crate::ui::panels::digit_count(line_count).max(4);
    let separator = 2; // "│ "
    (digits + separator) as u16
}

/// Determina en qué panel cayó una posición (col, row) absoluta.
///
/// Usa el layout almacenado en `AppState.last_layout`. Si no hay layout
/// (primer frame), retorna `None`. La función es pura — no muta estado.
pub(super) fn hit_test_panel(layout: &IdeLayout, col: u16, row: u16) -> Option<HitTestResult> {
    let point_in_rect = |r: Rect, c: u16, rw: u16| -> bool {
        c >= r.x && c < r.x + r.width && rw >= r.y && rw < r.y + r.height
    };

    // Activity bar siempre visible — verificar primero
    if point_in_rect(layout.activity_bar, col, row) {
        return Some(HitTestResult::ActivityBar {
            row_in_bar: row - layout.activity_bar.y,
        });
    }

    // Verificar paneles en orden de prioridad visual
    if layout.sidebar_visible && point_in_rect(layout.sidebar, col, row) {
        return Some(HitTestResult::Panel(PanelId::Explorer));
    }
    if point_in_rect(layout.editor_area, col, row) {
        return Some(HitTestResult::Panel(PanelId::Editor));
    }
    if layout.bottom_panel_visible && point_in_rect(layout.bottom_panel, col, row) {
        return Some(HitTestResult::Panel(PanelId::Terminal));
    }
    // Title bar y status bar no son paneles enfocables
    None
}

// ─── Mouse click ───────────────────────────────────────────────────────────────

/// Procesa un click de mouse — resuelve panel, cambia foco, ejecuta acción contextual.
pub(super) fn reduce_mouse_click(state: &mut AppState, col: u16, row: u16) {
    let Some(layout) = state.last_layout else {
        return; // Sin layout aún — primer frame
    };

    // ── Click en status bar: detectar click en zona del branch o fetch icon ──
    {
        let sb = layout.status_bar;
        if row >= sb.y && row < sb.y + sb.height {
            // La status bar tiene: " MODE " + " " + git_status + "  " + file_name...
            // git_status es: "⎇ main ↑2 ↓1 ⟳" — con fetch icon ⟳ al final
            // El mode ("NORMAL" o "LSP") ocupa ~8 chars (espacio + texto + espacio + separador)
            let mode_width: u16 = if state.lsp.has_server() { 6 } else { 8 }; // " LSP " vs " NORMAL "
            let git_status_start = sb.x + mode_width;

            if state.git.is_repo {
                // Calcular ancho display del git_status: "⎇ " + branch + ahead/behind + " ⟳"
                // ⎇ = 1 char display, espacio = 1, branch len, posibles " ↑N" " ↓N", " ⟳" = 2
                let branch_len = state.git.branch.len() as u16;
                // "⎇ " = 2 chars display
                let mut git_display_width: u16 = 2 + branch_len;
                if state.git.ahead > 0 {
                    // " ↑" + digits
                    git_display_width += 2 + digit_count_u32(state.git.ahead);
                }
                if state.git.behind > 0 {
                    // " ↓" + digits
                    git_display_width += 2 + digit_count_u32(state.git.behind);
                }
                // " ⟳" = 2 chars display
                git_display_width += 2;

                let git_status_end = git_status_start + git_display_width;
                // ⟳ icon is the last 1 display-character (at position git_status_end - 1)
                let fetch_col = git_status_end - 1;

                if col == fetch_col {
                    // Click en ⟳ → git fetch
                    let root = get_workspace_root(state);
                    match crate::git::commands::fetch(&root) {
                        Ok(()) => {
                            state.git.refresh(&root);
                            tracing::info!("git fetch via mouse click");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "git fetch falló via mouse");
                        }
                    }
                    return;
                }

                // Click en la zona del branch name → abrir branch picker
                // Branch zone: from git_status_start + 2 (after "⎇ ") to git_status_start + 2 + branch_len
                let branch_zone_start = git_status_start + 2;
                let branch_zone_end = branch_zone_start + branch_len;
                if col >= branch_zone_start && col < branch_zone_end {
                    let root = get_workspace_root(state);
                    match state.branch_picker.open(&root) {
                        Ok(()) => {
                            tracing::debug!("branch picker abierto via mouse click");
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "error al abrir branch picker via mouse");
                        }
                    }
                    return;
                }
            }
            // Click en otra parte de la status bar — ignorar
            return;
        }
    }

    let Some(hit) = hit_test_panel(&layout, col, row) else {
        return; // Click en zona no interactiva (title bar)
    };

    match hit {
        HitTestResult::ActivityBar { row_in_bar } => {
            // Resolver qué icono fue clickeado
            // Iconos: 0=Explorer, 1=Git, 2=Search, penúltima fila=Settings
            let bar_height = layout.activity_bar.height;
            let settings_row = bar_height.saturating_sub(2);

            if row_in_bar == settings_row {
                // Click en Settings (último icono)
                let effects = reduce(state, &Action::SettingsOpen);
                process_effects(&effects, &CancellationToken::new());
            } else {
                // Click en iconos de sección (0, 1, 2)
                let section = match row_in_bar {
                    0 => Some(SidebarSection::Explorer),
                    1 => Some(SidebarSection::Git),
                    2 => Some(SidebarSection::Search),
                    _ => None,
                };
                if let Some(section) = section {
                    let effects = reduce(state, &Action::ActivityBarSelect(section));
                    process_effects(&effects, &CancellationToken::new());
                }
            }
        }
        HitTestResult::Panel(panel) => {
            // Si la sidebar muestra search o git, redirigir foco al panel activo
            let panel = if panel == PanelId::Explorer && state.search.visible {
                PanelId::Search
            } else if panel == PanelId::Explorer && state.git.visible {
                PanelId::Git
            } else {
                panel
            };

            // Cambiar foco al panel clickeado
            state.focused_panel = panel;
            tracing::debug!(?panel, col, row, "mouse click → foco");

            match panel {
                PanelId::Search => {
                    // Click en search panel — resolver qué flat item fue clickeado
                    reduce_mouse_click_search(state, &layout, row);
                }
                PanelId::Explorer => {
                    reduce_mouse_click_explorer(state, &layout, row);
                }
                PanelId::Editor => {
                    reduce_mouse_click_editor(state, &layout, col, row);
                }
                // Terminal y otros: solo cambio de foco por ahora
                _ => {}
            }
        }
    }
}

/// Procesa click en el explorer — seleccionar entry, abrir/toggle.
fn reduce_mouse_click_explorer(state: &mut AppState, layout: &IdeLayout, row: u16) {
    let Some(ref mut explorer) = state.explorer else {
        return;
    };

    // Calcular el inner area de la sidebar (descontar bordes del Block)
    // Block con Borders::ALL tiene 1px de borde arriba (+ título) y 1px abajo
    let inner_y = layout.sidebar.y + 1; // Borde superior + título
    let inner_height = layout.sidebar.height.saturating_sub(2); // Bordes superior e inferior

    if row < inner_y || row >= inner_y + inner_height {
        return; // Click en el borde, no en contenido
    }

    // Índice visual dentro del inner area
    let visual_row = (row - inner_y) as usize;
    // Índice real en la lista aplanada = scroll_offset + visual_row
    let flat_index = explorer.scroll_offset + visual_row;

    let flat = explorer.flatten();
    let flat_len = flat.len();
    if flat_index >= flat_len {
        return; // Click debajo de los entries
    }

    let is_dir = flat[flat_index].is_dir;
    // CLONE: necesario — necesitamos el path para abrir archivo después de drop(flat)
    let entry_path = flat[flat_index].path.clone();
    drop(flat);

    // Seleccionar el entry clickeado
    explorer.selected_index = flat_index;

    if is_dir {
        // Toggle expand/collapse del directorio
        if let Err(e) = explorer.toggle_selected() {
            tracing::error!(error = %e, "error en toggle de explorer por mouse");
        }
    } else {
        // Abrir archivo en una tab del editor
        match state.tabs.open_file(&entry_path) {
            Ok(()) => {
                state
                    .tabs
                    .active_mut()
                    .init_highlighting(&state.highlight_engine);
                state.focused_panel = PanelId::Editor;
                state.update_status_cache();
                tracing::info!(path = %entry_path.display(), "archivo abierto por mouse click");
            }
            Err(e) => {
                tracing::error!(error = %e, "error al abrir archivo por mouse click");
            }
        }
    }
}

/// Procesa click en el search panel — seleccionar item en la lista aplanada.
fn reduce_mouse_click_search(state: &mut AppState, layout: &IdeLayout, row: u16) {
    // Calcular inner area de la sidebar (descontar bordes del Block)
    let inner_y = layout.sidebar.y + 1; // Borde superior + título
    let inner_height = layout.sidebar.height.saturating_sub(2); // Bordes

    if row < inner_y || row >= inner_y + inner_height {
        return;
    }

    // Calcular cuántas filas de input hay (query + replace? + include + exclude)
    let input_lines: u16 = if state.search.replace_visible { 4 } else { 3 };
    // +1 para summary al fondo
    let results_start = inner_y + input_lines;
    let results_end = inner_y + inner_height - 1; // última fila es summary

    if row < results_start || row >= results_end {
        return; // Click en inputs o summary, no en resultados
    }

    // Índice visual dentro del área de resultados
    let visual_row = (row - results_start) as usize;
    let flat_index = state.search.scroll_offset + visual_row;

    if flat_index >= state.search.flat_items.len() {
        return;
    }

    state.search.selected_flat_index = flat_index;

    // Ejecutar acción según tipo de item
    match state.search.flat_items[flat_index] {
        crate::search::FlatSearchItem::FileHeader { group_index } => {
            state.search.toggle_fold(group_index);
        }
        crate::search::FlatSearchItem::MatchLine { match_index, .. } => {
            state.search.selected_match = match_index;
            super::navigate_to_search_match(state);
        }
    }
}

/// Procesa click en el editor — mover cursor o cambiar tab.
fn reduce_mouse_click_editor(state: &mut AppState, layout: &IdeLayout, col: u16, row: u16) {
    // Calcular inner area del editor (descontar bordes del Block)
    let inner_y = layout.editor_area.y + 1;
    let inner_x = layout.editor_area.x + 1;
    let inner_height = layout.editor_area.height.saturating_sub(2);

    if row < inner_y || row >= inner_y + inner_height {
        return; // Click en borde
    }

    // ── Click en la barra de tabs (primera fila del inner area) ──
    let tab_bar_row = inner_y;
    if row == tab_bar_row {
        resolve_tab_click(state, col, inner_x);
        return;
    }

    // ── Click en breadcrumbs (segunda fila del inner area) ──
    let breadcrumbs_row = inner_y + 1;
    if row == breadcrumbs_row {
        return; // Breadcrumbs no son interactivos por ahora
    }

    // Ajustar coordenadas: el contenido empieza 2 filas después (tab bar + breadcrumbs)
    let content_y = inner_y + 2;
    if row < content_y {
        return;
    }

    let editor = state.tabs.active_mut();

    // Línea en el buffer = viewport offset + fila visual (relativa al contenido, no al inner)
    let visual_row = (row - content_y) as usize;
    let target_line = editor.viewport.scroll_offset + visual_row;

    // Columna en el buffer = col relativo al inner area - gutter dinámico
    let gutter = editor_gutter_width(editor.buffer.line_count());
    let text_x = inner_x + gutter;
    let target_col = if col >= text_x {
        (col - text_x) as usize
    } else {
        0 // Click en el gutter — columna 0
    };

    // Clampear a límites del buffer
    let max_line = editor.buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = editor.buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    // Limpiar cursores secundarios al hacer click
    editor.cursors.clear_secondary();
    let primary = editor.cursors.primary_mut();
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    // Iniciar selección con anchor = head = click_pos.
    // Si el usuario no arrastra, anchor == head → selección vacía (equivale a sin selección).
    // Si arrastra, el drag handler actualiza head para extender la selección.
    let click_pos = crate::editor::cursor::Position {
        line: clamped_line,
        col: clamped_col,
    };
    primary.selection = Some(crate::editor::selection::Selection::new(
        click_pos, click_pos,
    ));
    let pos = editor.cursors.primary().position;
    editor.viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();

    tracing::debug!(
        line = clamped_line,
        col = clamped_col,
        "mouse click → cursor editor"
    );
}

/// Resuelve qué tab fue clickeada y ejecuta la acción correspondiente.
///
/// Recorre las tabs pre-computadas calculando los anchos acumulados
/// para determinar cuál tab contiene la columna clickeada.
/// Click en `×` de tab activa → cerrar tab.
/// Click en cualquier parte de una tab → cambiar a esa tab.
fn resolve_tab_click(state: &mut AppState, col: u16, inner_x: u16) {
    let tab_infos = state.tabs.tab_info();
    let click_col = col.saturating_sub(inner_x) as usize;

    let mut accumulated: usize = 0;
    for (i, tab) in tab_infos.iter().enumerate() {
        // Mismo cálculo que render_tab_bar (con iconos):
        // "│ " (2) + icon(2) + " "(1) + name.len() + indicator.len() + " " (1)
        let icon = crate::ui::icons::file_icon(&tab.name);
        let has_indicator = tab.is_dirty || tab.is_active;
        let indicator_len: usize = if has_indicator { 2 } else { 0 };
        let tab_width = 2 + icon.len() + 1 + tab.name.len() + indicator_len + 1;

        if click_col >= accumulated && click_col < accumulated + tab_width {
            // Click cayó en esta tab
            if tab.is_active && !tab.is_dirty {
                // Verificar si clickeó en la zona del "×" (últimos 2 chars antes del padding)
                // "│ "(2) + icon(2) + " "(1) + name = close_start
                let close_start = accumulated + 2 + icon.len() + 1 + tab.name.len();
                if click_col >= close_start && click_col < close_start + 2 {
                    // Click en el ×: cerrar tab
                    state.tabs.close_active();
                    state.update_status_cache();
                    tracing::debug!("tab cerrada via mouse click");
                    return;
                }
            }
            // Cambiar a esta tab
            state.tabs.switch_to(i);
            state.update_status_cache();
            tracing::debug!(tab = i, "tab seleccionada via mouse click");
            return;
        }

        accumulated += tab_width;
    }
}

// ─── Mouse drag ────────────────────────────────────────────────────────────────

/// Procesa drag del mouse — selección de texto arrastrando.
///
/// Solo actúa si el drag cae en el editor area. Extiende la selección
/// desde el anchor (seteado en el click) hasta la posición actual del drag.
pub(super) fn reduce_mouse_drag(state: &mut AppState, col: u16, row: u16) {
    let Some(layout) = state.last_layout else {
        return; // Sin layout — primer frame
    };

    let Some(hit) = hit_test_panel(&layout, col, row) else {
        return;
    };

    // Drag-to-select solo en el editor
    if let HitTestResult::Panel(PanelId::Editor) = hit {
        reduce_mouse_drag_editor(state, &layout, col, row);
    }
}

/// Procesa drag en el editor — extiende selección desde anchor hasta posición del drag.
fn reduce_mouse_drag_editor(state: &mut AppState, layout: &IdeLayout, col: u16, row: u16) {
    // Calcular inner area del editor (descontar bordes del Block + tab bar + breadcrumbs)
    let inner_y = layout.editor_area.y + 1 + 2; // +1 borde, +1 tab bar, +1 breadcrumbs
    let inner_x = layout.editor_area.x + 1;
    let inner_height = layout.editor_area.height.saturating_sub(4); // bordes + tab bar + breadcrumbs

    // Clampear row al rango visible del editor para permitir scroll
    // cuando el drag sale por arriba o abajo del viewport
    let clamped_row = row.clamp(inner_y, inner_y + inner_height.saturating_sub(1));

    let editor = state.tabs.active_mut();

    // Línea en el buffer = viewport offset + fila visual
    let visual_row = (clamped_row - inner_y) as usize;
    let target_line = editor.viewport.scroll_offset + visual_row;

    // Columna en el buffer = col relativo al inner area - gutter dinámico
    let gutter = editor_gutter_width(editor.buffer.line_count());
    let text_x = inner_x + gutter;
    let target_col = if col >= text_x {
        (col - text_x) as usize
    } else {
        0 // Drag en el gutter — columna 0
    };

    // Clampear a límites del buffer
    let max_line = editor.buffer.line_count().saturating_sub(1);
    let clamped_line = target_line.min(max_line);
    let max_col = editor.buffer.line_len(clamped_line);
    let clamped_col = target_col.min(max_col);

    let primary = editor.cursors.primary_mut();

    // Verificar que hay una selección activa (seteada por el click previo)
    if primary.selection.is_none() {
        return;
    }

    // Actualizar posición del cursor y head de la selección
    primary.position.line = clamped_line;
    primary.position.col = clamped_col;
    primary.sync_desired_col();
    primary.extend_selection();

    // Scroll automático si el drag lleva el cursor fuera del viewport
    let pos = editor.cursors.primary().position;
    editor.viewport.ensure_cursor_visible(&pos);
    state.update_status_cache();

    tracing::trace!(
        line = clamped_line,
        col = clamped_col,
        "mouse drag → selección editor"
    );
}

// ─── Mouse scroll ──────────────────────────────────────────────────────────────

/// Procesa scroll del mouse — scrollea el panel bajo el cursor.
pub(super) fn reduce_mouse_scroll(
    state: &mut AppState,
    col: u16,
    row: u16,
    direction: ScrollDirection,
) {
    let Some(layout) = state.last_layout else {
        return;
    };

    let Some(hit) = hit_test_panel(&layout, col, row) else {
        return;
    };

    let panel = match hit {
        HitTestResult::Panel(p) => p,
        HitTestResult::ActivityBar { .. } => return, // No scroll en activity bar
    };

    match panel {
        PanelId::Explorer => {
            // Si search está visible, scrollear resultados de búsqueda (flat items)
            if state.search.visible {
                let flat_count = state.search.flat_items.len();
                match direction {
                    ScrollDirection::Up => {
                        state.search.scroll_offset = state
                            .search
                            .scroll_offset
                            .saturating_sub(MOUSE_SCROLL_LINES);
                    }
                    ScrollDirection::Down => {
                        let max_scroll = flat_count.saturating_sub(1);
                        state.search.scroll_offset =
                            (state.search.scroll_offset + MOUSE_SCROLL_LINES).min(max_scroll);
                    }
                }
            } else if let Some(ref mut explorer) = state.explorer {
                let flat_count = explorer.ensure_flat_cache().len();
                match direction {
                    ScrollDirection::Up => {
                        explorer.scroll_offset =
                            explorer.scroll_offset.saturating_sub(MOUSE_SCROLL_LINES);
                    }
                    ScrollDirection::Down => {
                        let max_scroll = flat_count.saturating_sub(1);
                        explorer.scroll_offset =
                            (explorer.scroll_offset + MOUSE_SCROLL_LINES).min(max_scroll);
                    }
                }
            }
        }
        PanelId::Editor => {
            let editor = state.tabs.active_mut();
            let line_count = editor.buffer.line_count();
            match direction {
                ScrollDirection::Up => {
                    editor.viewport.scroll_offset = editor
                        .viewport
                        .scroll_offset
                        .saturating_sub(EDITOR_SCROLL_LINES);
                }
                ScrollDirection::Down => {
                    let max_scroll = line_count.saturating_sub(1);
                    editor.viewport.scroll_offset =
                        (editor.viewport.scroll_offset + EDITOR_SCROLL_LINES).min(max_scroll);
                }
            }
        }
        PanelId::Terminal => {
            if let Some(ref mut session) = state.terminal.session {
                match direction {
                    ScrollDirection::Up => session.scroll_up(MOUSE_SCROLL_LINES),
                    ScrollDirection::Down => session.scroll_down(MOUSE_SCROLL_LINES),
                }
            }
        }
        // Otros paneles: scroll no implementado aún
        _ => {}
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

/// Cuenta dígitos decimales de un u32 (para calcular ancho display).
///
/// Pre-computado fuera del render. Evita `format!()`.
fn digit_count_u32(n: u32) -> u16 {
    if n == 0 {
        return 1;
    }
    let mut count: u16 = 0;
    let mut val = n;
    while val > 0 {
        count += 1;
        val /= 10;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_gutter_width_single_digit_lines() {
        // 9 lines → digit_count(9) = 1, max(1,4) = 4, + 2 separator = 6
        assert_eq!(editor_gutter_width(9), 6);
    }

    #[test]
    fn editor_gutter_width_double_digit_lines() {
        // 99 lines → digit_count(99) = 2, max(2,4) = 4, + 2 separator = 6
        assert_eq!(editor_gutter_width(99), 6);
    }

    #[test]
    fn editor_gutter_width_triple_digit_lines() {
        // 999 lines → digit_count(999) = 3, max(3,4) = 4, + 2 separator = 6
        assert_eq!(editor_gutter_width(999), 6);
    }

    #[test]
    fn editor_gutter_width_four_digit_lines() {
        // 1000 lines → digit_count(1000) = 4, max(4,4) = 4, + 2 separator = 6
        assert_eq!(editor_gutter_width(1000), 6);
    }

    #[test]
    fn editor_gutter_width_five_digit_lines() {
        // 10000 lines → digit_count(10000) = 5, max(5,4) = 5, + 2 separator = 7
        assert_eq!(editor_gutter_width(10000), 7);
    }

    #[test]
    fn hit_test_panel_inside_editor_area() {
        let layout = IdeLayout {
            title_bar: Rect::new(0, 0, 100, 1),
            activity_bar: Rect::new(0, 1, 3, 20),
            sidebar: Rect::new(3, 1, 25, 20),
            editor_area: Rect::new(28, 1, 72, 14),
            bottom_panel: Rect::new(28, 15, 72, 6),
            status_bar: Rect::new(0, 21, 100, 1),
            sidebar_visible: true,
            bottom_panel_visible: true,
        };
        let result = hit_test_panel(&layout, 50, 5);
        assert_eq!(result, Some(HitTestResult::Panel(PanelId::Editor)));
    }

    #[test]
    fn hit_test_panel_inside_sidebar_returns_explorer() {
        let layout = IdeLayout {
            title_bar: Rect::new(0, 0, 100, 1),
            activity_bar: Rect::new(0, 1, 3, 20),
            sidebar: Rect::new(3, 1, 25, 20),
            editor_area: Rect::new(28, 1, 72, 14),
            bottom_panel: Rect::new(28, 15, 72, 6),
            status_bar: Rect::new(0, 21, 100, 1),
            sidebar_visible: true,
            bottom_panel_visible: true,
        };
        let result = hit_test_panel(&layout, 10, 5);
        assert_eq!(result, Some(HitTestResult::Panel(PanelId::Explorer)));
    }

    #[test]
    fn hit_test_panel_outside_all_panels_returns_none() {
        let layout = IdeLayout {
            title_bar: Rect::new(0, 0, 100, 1),
            activity_bar: Rect::new(0, 1, 3, 20),
            sidebar: Rect::new(3, 1, 25, 20),
            editor_area: Rect::new(28, 1, 72, 14),
            bottom_panel: Rect::new(28, 15, 72, 6),
            status_bar: Rect::new(0, 21, 100, 1),
            sidebar_visible: true,
            bottom_panel_visible: true,
        };
        // Title bar row = 0, not a panel
        let result = hit_test_panel(&layout, 50, 0);
        assert_eq!(result, None);
    }
}
