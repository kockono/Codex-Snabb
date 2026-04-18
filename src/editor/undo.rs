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
