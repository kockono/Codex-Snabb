//! Editor: buffer model, cursor, viewport, undo/redo, búsqueda local.
//!
//! Integra todos los sub-módulos del editor en un solo `EditorState`.
//! El `EditorState` es el punto de entrada para todas las operaciones
//! de edición — coordina buffer, cursor, viewport y undo stack.

pub mod buffer;
pub mod cursor;
pub mod search;
pub mod undo;
pub mod viewport;

use std::path::Path;

use anyhow::Result;

use buffer::TextBuffer;
use cursor::{Cursor, Position};
use undo::{EditOperation, UndoStack};
use viewport::Viewport;

use crate::core::Direction;

/// Estado completo del editor.
///
/// Contiene el buffer de texto, cursor, viewport, undo stack y búsqueda.
/// Todas las operaciones de edición pasan por acá para mantener
/// la coordinación entre sub-sistemas (ej: insertar char -> registrar undo
/// -> ajustar cursor -> ajustar viewport).
#[derive(Debug)]
pub struct EditorState {
    /// Buffer de texto editable.
    pub buffer: TextBuffer,
    /// Cursor con posición y columna deseada.
    pub cursor: Cursor,
    /// Viewport virtual (qué porción del buffer es visible).
    pub viewport: Viewport,
    /// Historial de undo/redo.
    pub undo_stack: UndoStack,
    /// Búsqueda local activa (None si no hay búsqueda).
    #[expect(dead_code, reason = "se usará cuando se implemente búsqueda en editor")]
    pub search: Option<search::BufferSearch>,
}

impl EditorState {
    /// Crea un editor vacío (buffer vacío, cursor en 0,0).
    pub fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursor: Cursor::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
        }
    }

    /// Abre un archivo y crea un editor con su contenido.
    pub fn open_file(path: &Path) -> Result<Self> {
        let buffer = TextBuffer::from_file(path)?;
        Ok(Self {
            buffer,
            cursor: Cursor::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
        })
    }

    /// Inserta un carácter en la posición actual del cursor.
    ///
    /// Registra la operación en el undo stack, avanza el cursor,
    /// y asegura que el viewport siga visible.
    pub fn insert_char(&mut self, ch: char) {
        let pos = self.cursor.position;
        self.buffer.insert_char(pos, ch);
        self.undo_stack.push(EditOperation::InsertChar { pos, ch });
        // Mover cursor a la derecha después de insertar
        self.cursor.position.col += 1;
        self.cursor.sync_desired_col();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Elimina el carácter antes del cursor (backspace).
    ///
    /// Captura el estado necesario ANTES de mutar el buffer para
    /// poder registrar la operación de undo correctamente.
    pub fn delete_char(&mut self) {
        let pos = self.cursor.position;

        if pos.col > 0 {
            // Caso 1: borrar carácter dentro de la línea
            // Capturar info antes de mutar
            let deleted = self.buffer.delete_char(pos);
            if let Some(ch) = deleted {
                let del_pos = Position {
                    line: pos.line,
                    col: pos.col - 1,
                };
                self.undo_stack
                    .push(EditOperation::DeleteChar { pos: del_pos, ch });
                self.cursor.position.col = pos.col - 1;
                self.cursor.sync_desired_col();
                self.viewport.ensure_cursor_visible(&self.cursor.position);
            }
        } else if pos.line > 0 {
            // Caso 2: al inicio de línea — unir con la anterior
            // Capturar largo de la línea anterior ANTES de unir
            let prev_line_len = self.buffer.line_len(pos.line - 1);

            let deleted = self.buffer.delete_char(pos);
            if deleted.is_some() {
                // Registrar como DeleteNewline: al hacer undo, se re-divide la línea
                // en (prev_line, prev_line_len) — exactamente donde estaba la unión
                self.undo_stack.push(EditOperation::DeleteNewline {
                    pos: Position {
                        line: pos.line - 1,
                        col: 0,
                    },
                    col: prev_line_len,
                });
                // Cursor va al punto de unión
                self.cursor.position = Position {
                    line: pos.line - 1,
                    col: prev_line_len,
                };
                self.cursor.sync_desired_col();
                self.viewport.ensure_cursor_visible(&self.cursor.position);
            }
        }
        // Caso 3: inicio del buffer (line=0, col=0) — nada que borrar
    }

    /// Inserta un salto de línea (Enter) en la posición actual.
    ///
    /// Divide la línea en dos, mueve el cursor al inicio de la nueva línea.
    pub fn insert_newline(&mut self) {
        let pos = self.cursor.position;
        self.buffer.insert_newline(pos);
        self.undo_stack.push(EditOperation::InsertNewline { pos });
        // Cursor va al inicio de la nueva línea
        self.cursor.position.line += 1;
        self.cursor.position.col = 0;
        self.cursor.sync_desired_col();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Mueve el cursor en la dirección indicada y ajusta viewport.
    pub fn move_cursor(&mut self, direction: Direction) {
        match direction {
            Direction::Up => self.cursor.move_up(&self.buffer),
            Direction::Down => self.cursor.move_down(&self.buffer),
            Direction::Left => self.cursor.move_left(&self.buffer),
            Direction::Right => self.cursor.move_right(&self.buffer),
        }
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Mueve el cursor al inicio de la línea actual.
    pub fn move_to_line_start(&mut self) {
        self.cursor.move_to_line_start();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Mueve el cursor al final de la línea actual.
    pub fn move_to_line_end(&mut self) {
        self.cursor.move_to_line_end(&self.buffer);
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Mueve el cursor al inicio absoluto del buffer.
    pub fn move_to_buffer_start(&mut self) {
        self.cursor.move_to_start();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Mueve el cursor al final absoluto del buffer.
    pub fn move_to_buffer_end(&mut self) {
        self.cursor.move_to_end(&self.buffer);
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Deshace la última operación de edición.
    pub fn undo(&mut self) {
        let Some(op) = self.undo_stack.undo() else {
            return;
        };
        match op {
            EditOperation::InsertChar { pos, .. } => {
                // Undo insert = borrar el carácter en esa posición
                self.buffer.remove_char_at(pos);
                self.cursor.position = pos;
            }
            EditOperation::DeleteChar { pos, ch } => {
                // Undo delete = re-insertar el carácter
                self.buffer.raw_insert_char(pos, ch);
                self.cursor.position = Position {
                    line: pos.line,
                    col: pos.col + 1,
                };
            }
            EditOperation::InsertNewline { pos } => {
                // Undo newline = unir las dos líneas
                self.buffer.join_lines(pos.line);
                self.cursor.position = pos;
            }
            EditOperation::DeleteNewline { pos, col } => {
                // Undo delete newline = re-dividir la línea
                self.buffer.split_line_at(pos.line, col);
                self.cursor.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
        }
        self.cursor.sync_desired_col();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Rehace la última operación deshecha.
    pub fn redo(&mut self) {
        let Some(op) = self.undo_stack.redo() else {
            return;
        };
        match op {
            EditOperation::InsertChar { pos, ch } => {
                self.buffer.raw_insert_char(pos, ch);
                self.cursor.position = Position {
                    line: pos.line,
                    col: pos.col + 1,
                };
            }
            EditOperation::DeleteChar { pos, .. } => {
                self.buffer.remove_char_at(pos);
                self.cursor.position = pos;
            }
            EditOperation::InsertNewline { pos } => {
                self.buffer.insert_newline(pos);
                self.cursor.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
            EditOperation::DeleteNewline { pos, col } => {
                self.buffer.join_lines(pos.line);
                self.cursor.position = Position {
                    line: pos.line,
                    col,
                };
            }
        }
        self.cursor.sync_desired_col();
        self.viewport.ensure_cursor_visible(&self.cursor.position);
    }

    /// Guarda el archivo asociado al buffer.
    pub fn save(&mut self) -> Result<()> {
        self.buffer.save()
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
