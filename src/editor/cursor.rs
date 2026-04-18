//! Cursor: posición y movimiento dentro del buffer.
//!
//! El cursor mantiene `desired_col` para preservar la columna
//! al moverse verticalmente por líneas de distinta longitud.
//! Ejemplo: estás en columna 40, bajás a una línea de 10 chars,
//! el cursor va a col 10, pero si bajás otra vez a una de 50,
//! vuelve a col 40 — no a 10.

use super::buffer::TextBuffer;
use super::selection::Selection;

/// Posición lógica en el buffer (línea + columna, 0-indexed).
///
/// Tipo `Copy` — 16 bytes en 64-bit. Se pasa por valor sin problema.
/// Implementa `Ord` comparando primero `line` y luego `col` para
/// poder determinar el inicio/fin de selecciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Índice de línea (0-indexed).
    pub line: usize,
    /// Índice de columna en bytes (0-indexed).
    pub col: usize,
}

impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Position {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.line.cmp(&other.line).then(self.col.cmp(&other.col))
    }
}

impl Position {
    /// Crea una posición en (0, 0) — inicio del buffer.
    pub fn zero() -> Self {
        Self { line: 0, col: 0 }
    }
}

/// Cursor con posición actual, columna deseada y selección opcional.
///
/// Nota: el `EditorState` ahora usa `MultiCursorState` con `CursorInstance`.
/// Este struct se mantiene como API alternativa para contextos de un solo cursor.
#[derive(Debug)]
pub struct Cursor {
    /// Posición actual en el buffer.
    pub position: Position,
    /// Columna deseada — se preserva al moverse verticalmente.
    pub desired_col: usize,
    /// Selección activa (None si no hay selección).
    pub selection: Option<Selection>,
}

#[expect(
    dead_code,
    reason = "API de cursor simple — EditorState migró a MultiCursorState/CursorInstance"
)]
impl Cursor {
    /// Crea un cursor en la posición (0, 0).
    pub fn new() -> Self {
        Self {
            position: Position::zero(),
            desired_col: 0,
            selection: None,
        }
    }

    /// Inicia una selección con anchor en la posición actual.
    ///
    /// Si ya hay una selección, no hace nada — solo se inicia
    /// cuando no existe. Esto permite que Shift+flecha funcione
    /// de forma continua sin reiniciar el anchor.
    pub fn start_selection(&mut self) {
        if self.selection.is_none() {
            self.selection = Some(Selection::new(self.position, self.position));
        }
    }

    /// Actualiza el head de la selección a la posición actual del cursor.
    ///
    /// Se llama después de cada movimiento con Shift mantenido.
    pub fn extend_selection(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.head = self.position;
        }
    }

    /// Limpia la selección activa.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Verifica si hay una selección activa (no vacía).
    pub fn has_selection(&self) -> bool {
        self.selection.is_some_and(|s| !s.is_empty())
    }

    /// Retorna el texto seleccionado del buffer, si hay selección.
    pub fn selected_text(&self, buffer: &TextBuffer) -> Option<String> {
        self.selection.and_then(|sel| {
            if sel.is_empty() {
                None
            } else {
                Some(sel.selected_text(buffer))
            }
        })
    }

    /// Mueve el cursor una línea hacia arriba.
    ///
    /// Si ya está en la primera línea, no se mueve.
    /// Respeta `desired_col` y clampea a la longitud de la línea destino.
    /// Si `selecting == true`, extiende la selección; si `false`, la limpia.
    pub fn move_up(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        if self.position.line == 0 {
            return;
        }
        self.position.line -= 1;
        let line_len = buffer.line_len(self.position.line);
        self.position.col = self.desired_col.min(line_len);
        if selecting {
            self.extend_selection();
        }
    }

    /// Mueve el cursor una línea hacia abajo.
    ///
    /// Si ya está en la última línea, no se mueve.
    /// Respeta `desired_col` y clampea a la longitud de la línea destino.
    /// Si `selecting == true`, extiende la selección; si `false`, la limpia.
    pub fn move_down(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        if self.position.line + 1 >= buffer.line_count() {
            return;
        }
        self.position.line += 1;
        let line_len = buffer.line_len(self.position.line);
        self.position.col = self.desired_col.min(line_len);
        if selecting {
            self.extend_selection();
        }
    }

    /// Mueve el cursor un carácter a la izquierda.
    ///
    /// Si está al inicio de una línea, sube al final de la anterior.
    /// Actualiza `desired_col`.
    /// Si `selecting == true`, extiende la selección; si `false`, la limpia.
    pub fn move_left(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        if self.position.col > 0 {
            self.position.col -= 1;
        } else if self.position.line > 0 {
            self.position.line -= 1;
            self.position.col = buffer.line_len(self.position.line);
        }
        self.desired_col = self.position.col;
        if selecting {
            self.extend_selection();
        }
    }

    /// Mueve el cursor un carácter a la derecha.
    ///
    /// Si está al final de una línea, baja al inicio de la siguiente.
    /// Actualiza `desired_col`.
    /// Si `selecting == true`, extiende la selección; si `false`, la limpia.
    pub fn move_right(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        let line_len = buffer.line_len(self.position.line);
        if self.position.col < line_len {
            self.position.col += 1;
        } else if self.position.line + 1 < buffer.line_count() {
            self.position.line += 1;
            self.position.col = 0;
        }
        self.desired_col = self.position.col;
        if selecting {
            self.extend_selection();
        }
    }

    /// Maneja el modo de selección antes de un movimiento.
    ///
    /// Si `selecting`, inicia selección si no existe.
    /// Si `!selecting`, limpia cualquier selección existente.
    fn handle_selection_mode(&mut self, selecting: bool) {
        if selecting {
            self.start_selection();
        } else {
            self.clear_selection();
        }
    }

    /// Mueve el cursor al inicio de la línea actual.
    pub fn move_to_line_start(&mut self) {
        self.position.col = 0;
        self.desired_col = 0;
    }

    /// Mueve el cursor al final de la línea actual.
    pub fn move_to_line_end(&mut self, buffer: &TextBuffer) {
        self.position.col = buffer.line_len(self.position.line);
        self.desired_col = self.position.col;
    }

    /// Mueve el cursor al inicio absoluto del buffer (0, 0).
    pub fn move_to_start(&mut self) {
        self.position = Position::zero();
        self.desired_col = 0;
    }

    /// Mueve el cursor al final absoluto del buffer.
    pub fn move_to_end(&mut self, buffer: &TextBuffer) {
        let last_line = buffer.line_count().saturating_sub(1);
        self.position.line = last_line;
        self.position.col = buffer.line_len(last_line);
        self.desired_col = self.position.col;
    }

    /// Asegura que la posición del cursor sea válida dentro del buffer.
    ///
    /// Clampea línea y columna. Se llama después de operaciones que
    /// pueden invalidar la posición (delete, undo, etc.).
    #[expect(
        dead_code,
        reason = "se usará para validar cursor después de operaciones externas"
    )]
    pub fn clamp_to_buffer(&mut self, buffer: &TextBuffer) {
        let max_line = buffer.line_count().saturating_sub(1);
        self.position.line = self.position.line.min(max_line);
        let max_col = buffer.line_len(self.position.line);
        self.position.col = self.position.col.min(max_col);
    }

    /// Actualiza `desired_col` al valor actual de columna.
    ///
    /// Se llama después de movimientos horizontales explícitos.
    pub fn sync_desired_col(&mut self) {
        self.desired_col = self.position.col;
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}
