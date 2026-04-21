//! Editor: buffer model, cursor, viewport, undo/redo, multicursor, selección.
//!
//! Integra todos los sub-módulos del editor en un solo `EditorState`.
//! El `EditorState` es el punto de entrada para todas las operaciones
//! de edición — coordina buffer, multicursor, viewport y undo stack.

pub mod brackets;
pub mod buffer;
pub mod cursor;
pub mod highlighting;
pub mod indent;
pub mod multicursor;
pub mod search;
pub mod selection;
pub mod tabs;
pub mod ts_highlight;
pub mod undo;
pub mod viewport;

use std::path::Path;

use anyhow::Result;

use buffer::TextBuffer;
use cursor::Position;
use highlighting::{HighlightCache, HighlightEngine};
use ts_highlight::TsHighlightEngine;
use multicursor::MultiCursorState;
use selection::Selection;
use undo::{EditOperation, UndoStack};
use viewport::Viewport;

use crate::core::Direction;

/// Estado completo del editor.
///
/// Contiene el buffer de texto, multicursor, viewport, undo stack y búsqueda.
/// Todas las operaciones de edición pasan por acá para mantener
/// la coordinación entre sub-sistemas (ej: insertar char -> registrar undo
/// -> ajustar cursores -> ajustar viewport).
#[derive(Debug)]
pub struct EditorState {
    /// Buffer de texto editable.
    pub buffer: TextBuffer,
    /// Sistema de multicursor (reemplaza al cursor único).
    pub cursors: MultiCursorState,
    /// Viewport virtual (qué porción del buffer es visible).
    pub viewport: Viewport,
    /// Historial de undo/redo.
    pub undo_stack: UndoStack,
    /// Búsqueda local activa (None si no hay búsqueda).
    #[expect(dead_code, reason = "se usará cuando se implemente búsqueda en editor")]
    pub search: Option<search::BufferSearch>,
    /// Cache de syntax highlighting para este buffer.
    pub highlight_cache: HighlightCache,
    /// Si el highlight del viewport fue diferido al próximo frame.
    ///
    /// Se activa al abrir un archivo nuevo para no bloquear el frame
    /// del open con trabajo pesado. El siguiente frame lo procesa normal.
    pub highlight_deferred: bool,
}

impl EditorState {
    /// Crea un editor vacío (buffer vacío, cursor en 0,0).
    pub fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursors: MultiCursorState::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
            highlight_cache: HighlightCache::new(),
            highlight_deferred: false,
        }
    }

    /// Abre un archivo y crea un editor con su contenido.
    ///
    /// Si se pasa `engine`, detecta la syntax del archivo y prepara
    /// el cache de highlighting. Si `engine` es `None`, el cache queda
    /// vacío (sin syntax — el render usará color uniforme).
    pub fn open_file(path: &Path) -> Result<Self> {
        let buffer = TextBuffer::from_file(path)?;
        Ok(Self {
            buffer,
            cursors: MultiCursorState::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
            highlight_cache: HighlightCache::new(),
            // Diferir highlight al siguiente frame — el archivo acaba de abrirse.
            // Así el frame del open es instantáneo y el highlight corre después.
            highlight_deferred: true,
        })
    }

    /// Inicializa el cache de highlighting para el archivo actual.
    ///
    /// Intenta tree-sitter primero (6 lenguajes soportados). Si no hay
    /// grammar tree-sitter, cae a syntect como fallback (~50 lenguajes).
    /// Se llama después de `open_file` cuando el `HighlightEngine`
    /// está disponible (vive en `AppState`).
    pub fn init_highlighting(&mut self, engine: &HighlightEngine) {
        // Intentar tree-sitter primero
        if let Some(path) = self.buffer.file_path() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if let Some(ts_config) = ts_highlight::config_for_extension(ext) {
                let ts_engine = TsHighlightEngine::new(ts_config);
                self.highlight_cache.set_ts_engine(Some(ts_engine));
                tracing::debug!(ext, "tree-sitter grammar cargado");
                return; // No necesitamos syntect
            }
        }
        // Fallback: syntect (camino existente)
        if let Some(path) = self.buffer.file_path()
            && let Some(syntax) = engine.detect_syntax(path)
        {
            self.highlight_cache = HighlightCache::with_syntax(syntax.name.as_str());
        }
    }

    /// Re-destaca inmediatamente la línea del cursor primario.
    ///
    /// Elimina el parpadeo blanco de 80ms al tipear: la línea editada
    /// se re-colorea en el mismo frame sin esperar al debounce.
    /// `ensure_viewport_highlighted` sigue manejando el re-process
    /// contextual del viewport completo.
    pub fn rehighlight_cursor_line(&mut self, engine: &HighlightEngine) {
        let line = self.cursors.primary().position.line;
        self.highlight_cache
            .highlight_single_line(line, &self.buffer, engine);
    }

    /// Inserta un carácter en la posición de todos los cursores.
    ///
    /// Itera en ORDEN INVERSO por posición para que los offsets no se
    /// invaliden al insertar en posiciones anteriores.
    pub fn insert_char(&mut self, ch: char) {
        // Ordenar cursores por posición antes de iterar en reversa
        self.cursors.sort_by_position();

        // Iterar en orden inverso para preservar offsets
        for i in (0..self.cursors.cursors.len()).rev() {
            let pos = self.cursors.cursors[i].position;

            // Si el cursor tiene selección, borrar el texto seleccionado primero
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
                // Actualizar pos después del delete
                let new_pos = self.cursors.cursors[i].position;
                self.buffer.insert_char(new_pos, ch);
                self.undo_stack
                    .push(EditOperation::InsertChar { pos: new_pos, ch });
                self.cursors.cursors[i].position.col += 1;
                self.cursors.cursors[i].sync_desired_col();
                continue;
            }

            self.buffer.insert_char(pos, ch);
            self.undo_stack.push(EditOperation::InsertChar { pos, ch });
            self.cursors.cursors[i].position.col += 1;
            self.cursors.cursors[i].sync_desired_col();
            self.cursors.cursors[i].clear_selection();
        }
        // Invalidar solo desde la línea editada en adelante
        let edited_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Elimina el carácter antes de todos los cursores (backspace).
    ///
    /// Itera en ORDEN INVERSO para preservar offsets.
    pub fn delete_char(&mut self) {
        self.cursors.sort_by_position();

        for i in (0..self.cursors.cursors.len()).rev() {
            // Si hay selección, borrar el texto seleccionado
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
                continue;
            }

            let pos = self.cursors.cursors[i].position;

            if pos.col > 0 {
                let deleted = self.buffer.delete_char(pos);
                if let Some(ch) = deleted {
                    let del_pos = Position {
                        line: pos.line,
                        col: pos.col - 1,
                    };
                    self.undo_stack
                        .push(EditOperation::DeleteChar { pos: del_pos, ch });
                    self.cursors.cursors[i].position.col = pos.col - 1;
                    self.cursors.cursors[i].sync_desired_col();
                }
            } else if pos.line > 0 {
                let prev_line_len = self.buffer.line_len(pos.line - 1);
                let deleted = self.buffer.delete_char(pos);
                if deleted.is_some() {
                    self.undo_stack.push(EditOperation::DeleteNewline {
                        pos: Position {
                            line: pos.line - 1,
                            col: 0,
                        },
                        col: prev_line_len,
                    });
                    self.cursors.cursors[i].position = Position {
                        line: pos.line - 1,
                        col: prev_line_len,
                    };
                    self.cursors.cursors[i].sync_desired_col();
                }
            }
            self.cursors.cursors[i].clear_selection();
        }
        // Invalidar desde la línea más temprana editada
        let edited_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Inserta un salto de línea en la posición de todos los cursores.
    ///
    /// Itera en ORDEN INVERSO para preservar offsets.
    pub fn insert_newline(&mut self) {
        self.cursors.sort_by_position();

        for i in (0..self.cursors.cursors.len()).rev() {
            // Si hay selección, borrar primero
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
            }

            let pos = self.cursors.cursors[i].position;
            self.buffer.insert_newline(pos);
            self.undo_stack.push(EditOperation::InsertNewline { pos });
            self.cursors.cursors[i].position.line += 1;
            self.cursors.cursors[i].position.col = 0;
            self.cursors.cursors[i].sync_desired_col();
            self.cursors.cursors[i].clear_selection();
        }
        // Newline cambia estructura de líneas — invalidar desde la línea editada
        let edited_line = self.cursors.primary().position.line.saturating_sub(1);
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Borra el texto de una selección en un cursor específico.
    ///
    /// Elimina carácter por carácter desde el final hasta el inicio
    /// de la selección. Posiciona el cursor al inicio de la selección.
    fn delete_selection_at(&mut self, cursor_idx: usize) {
        let Some(sel) = self.cursors.cursors[cursor_idx].selection else {
            return;
        };
        if sel.is_empty() {
            return;
        }

        let start = sel.start();
        let end = sel.end();

        // Borrar el texto seleccionado — línea por línea desde el final
        if start.line == end.line {
            // Selección en una línea: borrar chars del rango
            for col in (start.col..end.col).rev() {
                let pos = Position {
                    line: start.line,
                    col: col + 1,
                };
                if let Some(ch) = self.buffer.delete_char(pos) {
                    self.undo_stack.push(EditOperation::DeleteChar {
                        pos: Position {
                            line: start.line,
                            col,
                        },
                        ch,
                    });
                }
            }
        } else {
            // Multi-línea: unir líneas y borrar contenido
            // Estrategia: borrar desde el final hacia el inicio
            // Primero, parte de la última línea (desde col 0 hasta end.col)
            // Luego, las líneas intermedias completas
            // Finalmente, parte de la primera línea (desde start.col hasta fin)

            // Unir todo en un solo paso: posicionar al inicio, borrar char a char
            // es ineficiente pero correcto para MVP
            let text_to_delete = sel.selected_text(&self.buffer);
            let chars_count = text_to_delete.len();

            // Posicionar al final de la selección y hacer backspace N veces
            // Esto es más sencillo y correcto con las operaciones existentes
            let mut current = end;
            for _ in 0..chars_count {
                if current.col > 0 || current.line > start.line || current.col > start.col {
                    let deleted = self.buffer.delete_char(current);
                    if let Some(ch) = deleted {
                        if ch == '\n' {
                            // Se unió con la línea anterior
                            if current.line > 0 {
                                let prev_len = self.buffer.line_len(current.line - 1);
                                current = Position {
                                    line: current.line - 1,
                                    col: prev_len,
                                };
                            }
                        } else {
                            current.col = current.col.saturating_sub(1);
                        }
                        // Registrar undo (simplificado)
                        self.undo_stack
                            .push(EditOperation::DeleteChar { pos: current, ch });
                    }
                }
            }
        }

        self.cursors.cursors[cursor_idx].position = start;
        self.cursors.cursors[cursor_idx].sync_desired_col();
        self.cursors.cursors[cursor_idx].clear_selection();
    }

    /// Mueve todos los cursores en la dirección indicada y ajusta viewport.
    pub fn move_cursor(&mut self, direction: Direction, selecting: bool) {
        // Borrow buffer fuera del loop para evitar conflicto con &mut self.cursors
        let buffer = &self.buffer;
        for cursor in &mut self.cursors.cursors {
            match direction {
                Direction::Up => cursor.move_up(buffer, selecting),
                Direction::Down => cursor.move_down(buffer, selecting),
                Direction::Left => cursor.move_left(buffer, selecting),
                Direction::Right => cursor.move_right(buffer, selecting),
            }
        }
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario al inicio de la línea actual.
    pub fn move_to_line_start(&mut self) {
        let primary = self.cursors.primary_mut();
        primary.position.col = 0;
        primary.desired_col = 0;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario al final de la línea actual.
    pub fn move_to_line_end(&mut self) {
        let line = self.cursors.primary().position.line;
        let line_len = self.buffer.line_len(line);
        let primary = self.cursors.primary_mut();
        primary.position.col = line_len;
        primary.desired_col = line_len;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario a una línea específica (0-indexed) y centra el viewport.
    ///
    /// Clampea la línea al rango válido del buffer. Cursor va a columna 0.
    /// El viewport se centra en la línea target para dar contexto visual.
    pub fn go_to_line(&mut self, line_idx: usize) {
        let clamped = line_idx.min(self.buffer.line_count().saturating_sub(1));
        let primary = self.cursors.primary_mut();
        primary.position.line = clamped;
        primary.position.col = 0;
        primary.desired_col = 0;
        primary.clear_selection();
        // Centrar viewport en la línea target
        let half_viewport = self.viewport.height / 2;
        self.viewport.scroll_offset = clamped.saturating_sub(half_viewport);
    }

    /// Mueve el cursor primario al inicio absoluto del buffer.
    pub fn move_to_buffer_start(&mut self) {
        let primary = self.cursors.primary_mut();
        primary.position = Position::zero();
        primary.desired_col = 0;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario al final absoluto del buffer.
    pub fn move_to_buffer_end(&mut self) {
        let last_line = self.buffer.line_count().saturating_sub(1);
        let last_col = self.buffer.line_len(last_line);
        let primary = self.cursors.primary_mut();
        primary.position = Position {
            line: last_line,
            col: last_col,
        };
        primary.desired_col = last_col;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Selecciona la siguiente ocurrencia del texto seleccionado (Ctrl+D).
    ///
    /// 1. Si no hay selección → seleccionar la palabra bajo el cursor.
    /// 2. Si hay selección → buscar la siguiente ocurrencia y agregar un cursor.
    pub fn select_next_occurrence(&mut self) {
        let primary = self.cursors.primary();
        let primary_sel = primary.selection;
        let primary_pos = primary.position;

        if primary_sel.is_none() || primary_sel.is_some_and(|s| s.is_empty()) {
            // Caso 1: sin selección — seleccionar la palabra bajo el cursor
            if let Some((word_start, word_end)) = self.word_at_position(primary_pos) {
                let sel = Selection::new(word_start, word_end);
                let primary = self.cursors.primary_mut();
                primary.position = word_end;
                primary.selection = Some(sel);
                primary.sync_desired_col();
            }
            return;
        }

        // Caso 2: ya hay selección — buscar la siguiente ocurrencia
        let sel = primary_sel.expect("ya verificamos que es Some");
        let search_text = sel.selected_text(&self.buffer);
        if search_text.is_empty() {
            return;
        }

        // Encontrar la posición después de la última selección (de cualquier cursor)
        let last_end = self
            .cursors
            .cursors
            .iter()
            .filter_map(|c| c.selection.map(|s| s.end()))
            .max()
            .unwrap_or(sel.end());

        // Buscar siguiente ocurrencia después de last_end
        if let Some((match_start, match_end)) =
            self.find_next_occurrence(&search_text, last_end, false)
        {
            let new_sel = Selection::new(match_start, match_end);
            self.cursors.add_cursor(match_end, Some(new_sel));
        } else {
            // Wrap around: buscar desde el inicio del buffer
            let buffer_start = Position::zero();
            if let Some((match_start, match_end)) =
                self.find_next_occurrence(&search_text, buffer_start, true)
            {
                // Verificar que no sea una ocurrencia que ya tiene cursor
                let already_has = self.cursors.cursors.iter().any(|c| {
                    c.selection
                        .is_some_and(|s| s.start() == match_start && s.end() == match_end)
                });
                if !already_has {
                    let new_sel = Selection::new(match_start, match_end);
                    self.cursors.add_cursor(match_end, Some(new_sel));
                }
            }
        }
    }

    /// Encuentra los límites de la palabra en la posición dada.
    ///
    /// Una "palabra" son caracteres alfanuméricos + `_` contiguos.
    /// Retorna (start, end) de la palabra, o None si no hay palabra.
    fn word_at_position(&self, pos: Position) -> Option<(Position, Position)> {
        let line = self.buffer.line(pos.line)?;
        let bytes = line.as_bytes();

        if pos.col >= bytes.len() || !is_word_char(bytes[pos.col]) {
            // Intentar hacia la izquierda si estamos justo después de una palabra
            if pos.col > 0 && pos.col <= bytes.len() && is_word_char(bytes[pos.col - 1]) {
                // Estamos justo después del final de una palabra
                let mut start = pos.col - 1;
                while start > 0 && is_word_char(bytes[start - 1]) {
                    start -= 1;
                }
                return Some((
                    Position {
                        line: pos.line,
                        col: start,
                    },
                    Position {
                        line: pos.line,
                        col: pos.col,
                    },
                ));
            }
            return None;
        }

        // Encontrar inicio de la palabra
        let mut start = pos.col;
        while start > 0 && is_word_char(bytes[start - 1]) {
            start -= 1;
        }

        // Encontrar fin de la palabra
        let mut end = pos.col;
        while end < bytes.len() && is_word_char(bytes[end]) {
            end += 1;
        }

        if start == end {
            return None;
        }

        Some((
            Position {
                line: pos.line,
                col: start,
            },
            Position {
                line: pos.line,
                col: end,
            },
        ))
    }

    /// Busca la siguiente ocurrencia de `text` en el buffer después de `after`.
    ///
    /// Si `stop_at_start` es true, solo busca hasta el inicio del buffer
    /// (para wrap-around sin loops infinitos).
    fn find_next_occurrence(
        &self,
        text: &str,
        after: Position,
        _stop_at_start: bool,
    ) -> Option<(Position, Position)> {
        let total_lines = self.buffer.line_count();

        // Buscar en la línea actual después de `after.col`
        if let Some(line_content) = self.buffer.line(after.line) {
            let search_from = after.col.min(line_content.len());
            if let Some(offset) = line_content[search_from..].find(text) {
                let col = search_from + offset;
                return Some((
                    Position {
                        line: after.line,
                        col,
                    },
                    Position {
                        line: after.line,
                        col: col + text.len(),
                    },
                ));
            }
        }

        // Buscar en las líneas siguientes
        for line_idx in (after.line + 1)..total_lines {
            if let Some(line_content) = self.buffer.line(line_idx)
                && let Some(col) = line_content.find(text)
            {
                return Some((
                    Position {
                        line: line_idx,
                        col,
                    },
                    Position {
                        line: line_idx,
                        col: col + text.len(),
                    },
                ));
            }
        }

        // Si estamos en wrap-around, buscar desde el inicio
        if after.line > 0 || after.col > 0 {
            for line_idx in 0..=after.line.min(total_lines.saturating_sub(1)) {
                if let Some(line_content) = self.buffer.line(line_idx) {
                    let end_col = if line_idx == after.line {
                        after.col
                    } else {
                        line_content.len()
                    };
                    let search_range = &line_content[..end_col.min(line_content.len())];
                    if let Some(col) = search_range.find(text) {
                        return Some((
                            Position {
                                line: line_idx,
                                col,
                            },
                            Position {
                                line: line_idx,
                                col: col + text.len(),
                            },
                        ));
                    }
                }
            }
        }

        None
    }

    /// Deshace la última operación de edición.
    pub fn undo(&mut self) {
        let Some(op) = self.undo_stack.undo() else {
            return;
        };
        let primary = self.cursors.primary_mut();
        match op {
            EditOperation::InsertChar { pos, .. } => {
                self.buffer.remove_char_at(pos);
                primary.position = pos;
            }
            EditOperation::DeleteChar { pos, ch } => {
                self.buffer.raw_insert_char(pos, ch);
                primary.position = Position {
                    line: pos.line,
                    col: pos.col + 1,
                };
            }
            EditOperation::InsertNewline { pos } => {
                self.buffer.join_lines(pos.line);
                primary.position = pos;
            }
            EditOperation::DeleteNewline { pos, col } => {
                self.buffer.split_line_at(pos.line, col);
                primary.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
        }
        primary.sync_desired_col();
        primary.clear_selection();
        // Limpiar cursores secundarios en undo
        self.cursors.clear_secondary();
        // Invalidar desde la línea afectada por el undo
        let undo_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(undo_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Rehace la última operación deshecha.
    pub fn redo(&mut self) {
        let Some(op) = self.undo_stack.redo() else {
            return;
        };
        let primary = self.cursors.primary_mut();
        match op {
            EditOperation::InsertChar { pos, ch } => {
                self.buffer.raw_insert_char(pos, ch);
                primary.position = Position {
                    line: pos.line,
                    col: pos.col + 1,
                };
            }
            EditOperation::DeleteChar { pos, .. } => {
                self.buffer.remove_char_at(pos);
                primary.position = pos;
            }
            EditOperation::InsertNewline { pos } => {
                self.buffer.insert_newline(pos);
                primary.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
            EditOperation::DeleteNewline { pos, col } => {
                self.buffer.join_lines(pos.line);
                primary.position = Position {
                    line: pos.line,
                    col,
                };
            }
        }
        primary.sync_desired_col();
        primary.clear_selection();
        self.cursors.clear_secondary();
        // Invalidar desde la línea afectada por el redo
        let redo_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(redo_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Guarda el archivo asociado al buffer.
    pub fn save(&mut self) -> Result<()> {
        self.buffer.save()
    }

    /// Limpia los cursores secundarios (Esc con multicursor activo).
    pub fn clear_multicursors(&mut self) {
        self.cursors.clear_secondary();
    }

    /// Verifica si hay múltiples cursores activos.
    pub fn has_multicursors(&self) -> bool {
        self.cursors.has_multiple()
    }
}

/// Verifica si un byte es parte de una "palabra" (alfanumérico o `_`).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
