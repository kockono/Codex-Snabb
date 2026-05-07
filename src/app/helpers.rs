//! Helpers compartidos del módulo app.
//!
//! Funciones utilitarias puras o casi-puras usadas por el reducer
//! y otras partes del event loop. Extraídas de mod.rs para reducir
//! el tamaño del archivo principal.

use std::path::PathBuf;

use super::AppState;
use crate::core::command::CommandRegistry;
use crate::core::PanelId;
use crate::editor::EditorState;

/// Helper: obtiene la ruta al archivo de persistencia de keybindings.
fn keybindings_config_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    } else {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    }?;
    Some(base.join("ide-tui").join("keybindings.json"))
}

/// Helper: carga keybindings desde disco al registry.
pub(super) fn load_keybindings(registry: &mut CommandRegistry) {
    let Some(path) = keybindings_config_path() else {
        return;
    };

    // Formato: command_id -> Option<keybinding>
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(overrides) = serde_json::from_str::<std::collections::HashMap<String, Option<String>>>(&content) {
            for (id, keybind) in overrides {
                registry.update_keybinding(&id, keybind.as_deref());
            }
        }
    }
}

/// Helper: guarda keybindings del registry a disco.
pub(super) fn save_keybindings(registry: &CommandRegistry) {
    let Some(path) = keybindings_config_path() else {
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let overrides = registry.overrides();
    // Convertir a HashMap<String, ...> para serialización
    let serializable: std::collections::HashMap<String, Option<String>> = overrides
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();

    if let Ok(json) = serde_json::to_string_pretty(&serializable) {
        let _ = std::fs::write(&path, json);
    }
}

/// Helper: obtiene el tamaño por defecto para un pane de terminal.
///
/// Usa el layout del último frame si está disponible (bottom_panel area
/// menos bordes). Fallback a 80x24 si no hay layout aún.
pub(super) fn get_terminal_default_size(state: &AppState) -> (u16, u16) {
    state
        .last_layout
        .map(|l| {
            (
                l.bottom_panel.width.saturating_sub(2),
                l.bottom_panel.height.saturating_sub(2),
            )
        })
        .unwrap_or((80, 24))
}

/// Construye el ciclo de paneles dinámico según qué paneles están visibles.
///
/// Git, Search y Projects entran al ciclo solo si están visibles en la sidebar.
/// Terminal entra solo si el bottom panel está visible.
fn active_cycle(state: &AppState) -> Vec<PanelId> {
    let mut cycle = Vec::with_capacity(5);
    // Sidebar — solo el panel activo entra al ciclo
    if state.sidebar_visible {
        if state.search.visible {
            cycle.push(PanelId::Search);
        } else if state.git.visible {
            cycle.push(PanelId::Git);
        } else if state.projects.visible {
            cycle.push(PanelId::Projects);
        } else {
            cycle.push(PanelId::Explorer);
        }
    }
    cycle.push(PanelId::Editor);
    if state.bottom_panel_visible {
        cycle.push(PanelId::Terminal);
    }
    cycle
}

/// Siguiente panel en el ciclo dinámico (Tab).
pub(super) fn focus_next_panel(state: &AppState) -> PanelId {
    let cycle = active_cycle(state);
    let current = cycle.iter().position(|&p| p == state.focused_panel).unwrap_or(0);
    cycle[(current + 1) % cycle.len()]
}

/// Panel anterior en el ciclo dinámico (Shift+Tab).
pub(super) fn focus_prev_panel(state: &AppState) -> PanelId {
    let cycle = active_cycle(state);
    let current = cycle.iter().position(|&p| p == state.focused_panel).unwrap_or(0);
    let prev = if current == 0 { cycle.len() - 1 } else { current - 1 };
    cycle[prev]
}

/// Helper: obtiene el workspace root desde el explorer o cwd.
pub(super) fn get_workspace_root(state: &AppState) -> PathBuf {
    state
        .explorer
        .as_ref()
        .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut state
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Helpr: envía notificación LSP did_change si hay server activo.
///
/// Usa debounce interno del LspState — no envía en cada keystroke.
pub(super) fn notify_lsp_change(state: &mut AppState) {
    if !state.lsp.has_server() {
        return;
    }
    if let Some(path) = state
        .tabs
        .active()
        .buffer
        .file_path()
        .map(|p| p.to_path_buf())
    {
        let text = buffer_full_text(state.tabs.active());
        if let Err(e) = state.lsp.notify_change(&path, &text) {
            tracing::warn!(error = %e, "error en LSP did_change");
        }
    }
}

/// Helper: obtiene el texto completo del buffer del editor como un String.
///
/// Reconstruye el texto uniendo líneas con `\n`. Se usa para LSP did_open/did_change.
pub(super) fn buffer_full_text(editor: &EditorState) -> String {
    let line_count = editor.buffer.line_count();
    // Pre-alocar con estimado razonable (80 chars por línea promedio)
    let mut text = String::with_capacity(line_count * 80);
    for i in 0..line_count {
        if i > 0 {
            text.push('\n');
        }
        if let Some(line) = editor.buffer.line(i) {
            text.push_str(line);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::buffer::TextBuffer;

    /// Helper: crea un EditorState con un buffer desde texto.
    fn editor_with_text(text: &str) -> EditorState {
        let buffer = TextBuffer::from_text(text);
        EditorState {
            buffer,
            cursors: crate::editor::multicursor::MultiCursorState::new(),
            viewport: crate::editor::viewport::Viewport::new(),
            undo_stack: crate::editor::undo::UndoStack::new(),
            search: None,
            highlight_cache: crate::editor::highlighting::HighlightCache::new(),
            highlight_deferred: false,
            diff_view: None,
        }
    }

    #[test]
    fn buffer_full_text_with_known_content() {
        let editor = editor_with_text("hello world");
        let result = buffer_full_text(&editor);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn buffer_full_text_with_empty_buffer() {
        let editor = editor_with_text("");
        let result = buffer_full_text(&editor);
        assert_eq!(result, "");
    }

    #[test]
    fn buffer_full_text_with_multiline_content() {
        let editor = editor_with_text("line one\nline two\nline three");
        let result = buffer_full_text(&editor);
        assert_eq!(result, "line one\nline two\nline three");
    }
}