//! Helpers compartidos del módulo app.
//!
//! Funciones utilitarias puras o casi-puras usadas por el reducer
//! y otras partes del event loop. Extraídas de mod.rs para reducir
//! el tamaño del archivo principal.

use std::path::PathBuf;

use super::AppState;
use crate::editor::EditorState;

/// Helper: obtiene el workspace root desde el explorer o cwd.
pub(super) fn get_workspace_root(state: &AppState) -> PathBuf {
    state
        .explorer
        .as_ref()
        .map(|e| e.root.clone()) // CLONE: necesario — root se usa después de &mut state
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Helper: envía notificación LSP did_change si hay server activo.
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
