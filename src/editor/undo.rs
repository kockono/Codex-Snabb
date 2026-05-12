//! Undo/Redo: stack simple de operaciones de edición.
//!
//! Cada operación atómica se registra. Al hacer undo, la operación
//! se invierte y se mueve al redo stack. Al hacer una nueva edición,
//! el redo stack se limpia (no hay branching de historial).
//!
//! Capacidad limitada por `max_history` para controlar uso de RAM.

use super::cursor::Position;

/// Tamaño máximo por defecto del historial de undo.
const DEFAULT_MAX_HISTORY: usize = 1000;

/// Operación atómica de edición — almacena suficiente info para revertir.
///
/// Cada variante es la operación DIRECTA (lo que el usuario hizo).
/// La inversa se computa al hacer undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOperation {
    /// Carácter insertado en posición. Para undo: borrar ese carácter.
    InsertChar { pos: Position, ch: char },
    /// Carácter eliminado de posición. Para undo: re-insertar ese carácter.
    DeleteChar { pos: Position, ch: char },
    /// Newline insertado en posición. Para undo: unir las dos líneas.
    InsertNewline { pos: Position },
    /// Newline eliminado (dos líneas unidas). Para undo: re-dividir.
    DeleteNewline { pos: Position, col: usize },
    /// Dos líneas intercambiadas (Alt+Up/Down). Para undo: volver a swap.
    SwapLines { a: usize, b: usize },
    /// Línea reemplazada (toggle comment). Para undo: restaurar `old`.
    ReplaceLine {
        line_idx: usize,
        old: String,
        new: String,
    },
    /// Línea insertada en el índice. Para undo: borrar esa línea.
    /// Para redo: re-insertar el contenido (clone necesario para re-aplicar).
    InsertLine { line: usize, content: String },
    /// Bloque de texto insertado atómicamente (paste, multi-char).
    ///
    /// `start` es donde empezó la inserción, `end` la posición justo después
    /// del último byte. `text` es el contenido para redo (y el rango para
    /// validar undo). Undo = `delete_range(start, end)`. Redo = `insert_text(start, &text)`.
    InsertText {
        start: Position,
        end: Position,
        text: String,
    },
    /// Bloque de texto eliminado atómicamente (selección, paste-replace).
    ///
    /// `text` es lo que se borró — necesario para undo. Undo = `insert_text(start, &text)`.
    /// Redo = `delete_range(start, end)`.
    DeleteRange {
        start: Position,
        end: Position,
        text: String,
    },
}

/// Stack dual de undo/redo con capacidad limitada.
///
/// `undo` contiene las operaciones más recientes al final (LIFO).
/// `redo` contiene las operaciones deshechas, listas para rehacer.
/// Cada nueva edición limpia el redo stack.
#[derive(Debug)]
pub struct UndoStack {
    /// Operaciones que se pueden deshacer (más reciente al final).
    undo: Vec<EditOperation>,
    /// Operaciones deshechas que se pueden rehacer.
    redo: Vec<EditOperation>,
    /// Cantidad máxima de operaciones en el historial.
    max_history: usize,
}

impl UndoStack {
    /// Crea un UndoStack con capacidad por defecto.
    pub fn new() -> Self {
        Self {
            undo: Vec::with_capacity(128),
            redo: Vec::with_capacity(64),
            max_history: DEFAULT_MAX_HISTORY,
        }
    }

    /// Registra una nueva operación en el undo stack.
    ///
    /// Limpia el redo stack (nueva edición invalida el historial de redo).
    /// Si se excede `max_history`, elimina la operación más antigua.
    pub fn push(&mut self, op: EditOperation) {
        self.redo.clear();
        if self.undo.len() >= self.max_history {
            self.undo.remove(0);
        }
        self.undo.push(op);
    }

    /// Deshace la última operación.
    ///
    /// Retorna la operación para que el caller la revierta en el buffer.
    /// La operación se mueve al redo stack.
    pub fn undo(&mut self) -> Option<EditOperation> {
        let op = self.undo.pop()?;
        // CLONE: necesario para guardar la misma operación en redo
        self.redo.push(op.clone());
        Some(op)
    }

    /// Rehace la última operación deshecha.
    ///
    /// Retorna la operación para que el caller la re-aplique.
    /// La operación vuelve al undo stack.
    pub fn redo(&mut self) -> Option<EditOperation> {
        let op = self.redo.pop()?;
        // CLONE: necesario para guardar la misma operación en undo
        self.undo.push(op.clone());
        Some(op)
    }

    /// Limpia todo el historial (undo y redo).
    #[expect(dead_code, reason = "se usará al recargar archivo o cerrar buffer")]
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_line_push_undo_redo_round_trip() {
        let mut stack = UndoStack::new();
        let op = EditOperation::InsertLine {
            line: 2,
            // CLONE: test setup needs owned string
            content: String::from("hola"),
        };
        stack.push(op.clone());

        // undo retorna la operación y la mueve a redo
        let undone = stack.undo().expect("undo should return op");
        assert!(matches!(
            undone,
            EditOperation::InsertLine { line: 2, .. }
        ));
        if let EditOperation::InsertLine { content, .. } = &undone {
            assert_eq!(content, "hola");
        }

        // tras undo, no hay más undo
        assert!(stack.undo().is_none());

        // redo retorna la misma operación
        let redone = stack.redo().expect("redo should return op");
        assert!(matches!(
            redone,
            EditOperation::InsertLine { line: 2, .. }
        ));
    }

    #[test]
    fn insert_line_distinct_from_replace_line() {
        // Triangulación: InsertLine y ReplaceLine son variantes distintas
        // y no se confunden en pattern matching.
        let insert = EditOperation::InsertLine {
            line: 0,
            content: String::from("a"),
        };
        let replace = EditOperation::ReplaceLine {
            line_idx: 0,
            old: String::from("a"),
            new: String::from("b"),
        };
        assert!(!matches!(insert, EditOperation::ReplaceLine { .. }));
        assert!(!matches!(replace, EditOperation::InsertLine { .. }));
    }

    // ── InsertText / DeleteRange ──

    #[test]
    fn insert_text_push_undo_redo_round_trip() {
        let mut stack = UndoStack::new();
        let op = EditOperation::InsertText {
            start: Position { line: 0, col: 2 },
            end: Position { line: 1, col: 3 },
            // CLONE: test setup needs owned string
            text: String::from("ñoño\ncódigo"),
        };
        stack.push(op);

        let undone = stack.undo().expect("undo should return op");
        match undone {
            EditOperation::InsertText { start, end, text } => {
                assert_eq!(start, Position { line: 0, col: 2 });
                assert_eq!(end, Position { line: 1, col: 3 });
                assert_eq!(text, "ñoño\ncódigo");
            }
            _ => panic!("expected InsertText"),
        }

        let redone = stack.redo().expect("redo should return op");
        assert!(matches!(redone, EditOperation::InsertText { .. }));
    }

    #[test]
    fn delete_range_push_undo_redo_round_trip() {
        let mut stack = UndoStack::new();
        let op = EditOperation::DeleteRange {
            start: Position { line: 1, col: 0 },
            end: Position { line: 2, col: 5 },
            // CLONE: test setup needs owned string
            text: String::from("borrado\nmulti"),
        };
        stack.push(op);

        let undone = stack.undo().expect("undo should return op");
        match undone {
            EditOperation::DeleteRange { start, end, text } => {
                assert_eq!(start, Position { line: 1, col: 0 });
                assert_eq!(end, Position { line: 2, col: 5 });
                assert_eq!(text, "borrado\nmulti");
            }
            _ => panic!("expected DeleteRange"),
        }

        let redone = stack.redo().expect("redo should return op");
        assert!(matches!(redone, EditOperation::DeleteRange { .. }));
    }

    #[test]
    fn insert_text_distinct_from_insert_char() {
        let block = EditOperation::InsertText {
            start: Position::zero(),
            end: Position { line: 0, col: 5 },
            text: String::from("hello"),
        };
        let single = EditOperation::InsertChar {
            pos: Position::zero(),
            ch: 'h',
        };
        assert!(!matches!(block, EditOperation::InsertChar { .. }));
        assert!(!matches!(single, EditOperation::InsertText { .. }));
    }

}
