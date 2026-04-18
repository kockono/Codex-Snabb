//! MultiCursor: sistema de cursores múltiples para edición simultánea.
//!
//! Mantiene un vector de `CursorInstance` con un cursor primario designado.
//! El cursor primario controla el scroll del viewport y el hardware cursor.
//! Los cursores secundarios se renderizan visualmente.
//!
//! Diseño austero: `Vec` para MVP. Si benchmarks justifican `SmallVec<[_; 4]>`,
//! se migra después — nunca por moda.

use super::buffer::TextBuffer;
use super::cursor::Position;
use super::selection::Selection;

/// Instancia individual de cursor con posición, selección y desired_col.
///
/// Tipo `Clone` — se necesita para crear copias al agregar cursores.
/// Tamaño: 2 × usize (position) + Option<Selection> (40 bytes) + usize = ~56 bytes.
#[derive(Debug, Clone)]
pub struct CursorInstance {
    /// Posición actual en el buffer.
    pub position: Position,
    /// Selección activa (None si no hay selección).
    pub selection: Option<Selection>,
    /// Columna deseada — se preserva al moverse verticalmente.
    pub desired_col: usize,
}

impl CursorInstance {
    /// Crea una instancia de cursor en una posición dada.
    pub fn new(position: Position) -> Self {
        Self {
            position,
            selection: None,
            desired_col: position.col,
        }
    }

    /// Crea una instancia con selección pre-establecida.
    pub fn with_selection(position: Position, selection: Selection) -> Self {
        Self {
            position,
            selection: Some(selection),
            desired_col: position.col,
        }
    }

    /// Inicia selección con anchor en la posición actual.
    pub fn start_selection(&mut self) {
        if self.selection.is_none() {
            self.selection = Some(Selection::new(self.position, self.position));
        }
    }

    /// Extiende la selección al position actual.
    pub fn extend_selection(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.head = self.position;
        }
    }

    /// Limpia la selección.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Verifica si tiene selección activa (no vacía).
    pub fn has_selection(&self) -> bool {
        self.selection.is_some_and(|s| !s.is_empty())
    }

    /// Sincroniza desired_col con la columna actual.
    pub fn sync_desired_col(&mut self) {
        self.desired_col = self.position.col;
    }

    // ── Movimiento ──

    /// Mueve hacia arriba con soporte de selección.
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

    /// Mueve hacia abajo con soporte de selección.
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

    /// Mueve a la izquierda con soporte de selección.
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

    /// Mueve a la derecha con soporte de selección.
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

    /// Maneja inicio/limpieza de selección según modo.
    fn handle_selection_mode(&mut self, selecting: bool) {
        if selecting {
            self.start_selection();
        } else {
            self.clear_selection();
        }
    }
}

/// Estado de múltiples cursores.
///
/// Siempre tiene al menos un cursor (el primario). El primario controla
/// el viewport y el hardware cursor de la terminal.
#[derive(Debug)]
pub struct MultiCursorState {
    /// Cursores activos — siempre >= 1.
    pub cursors: Vec<CursorInstance>,
    /// Índice del cursor primario (para scroll y hardware cursor).
    pub primary_index: usize,
}

impl MultiCursorState {
    /// Crea un estado con un solo cursor en (0, 0).
    pub fn new() -> Self {
        Self {
            cursors: vec![CursorInstance::new(Position::zero())],
            primary_index: 0,
        }
    }

    /// Referencia al cursor primario.
    pub fn primary(&self) -> &CursorInstance {
        &self.cursors[self.primary_index]
    }

    /// Referencia mutable al cursor primario.
    pub fn primary_mut(&mut self) -> &mut CursorInstance {
        &mut self.cursors[self.primary_index]
    }

    /// Agrega un cursor nuevo en la posición dada con selección opcional.
    ///
    /// No agrega duplicados (misma posición que un cursor existente).
    pub fn add_cursor(&mut self, pos: Position, selection: Option<Selection>) {
        // Evitar cursores duplicados en la misma posición
        let already_exists = self.cursors.iter().any(|c| c.position == pos);
        if already_exists {
            return;
        }

        let instance = if let Some(sel) = selection {
            CursorInstance::with_selection(pos, sel)
        } else {
            CursorInstance::new(pos)
        };
        self.cursors.push(instance);
    }

    /// Verifica si hay más de un cursor activo.
    pub fn has_multiple(&self) -> bool {
        self.cursors.len() > 1
    }

    /// Elimina todos los cursores excepto el primario (Esc).
    pub fn clear_secondary(&mut self) {
        let primary = self.cursors[self.primary_index].clone(); // CLONE: necesario — se va a limpiar el vec
        self.cursors.clear();
        self.cursors.push(primary);
        self.primary_index = 0;
    }

    /// Aplica una operación a todos los cursores.
    #[expect(dead_code, reason = "API genérica — se usará para operaciones batch")]
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&mut CursorInstance)) {
        for cursor in &mut self.cursors {
            f(cursor);
        }
    }

    /// Cantidad de cursores activos.
    #[expect(dead_code, reason = "se usará para mostrar count en status bar")]
    pub fn cursor_count(&self) -> usize {
        self.cursors.len()
    }

    /// Ordena cursores por posición (necesario para iterar en orden inverso
    /// durante ediciones que cambian offsets).
    ///
    /// Actualiza `primary_index` para que siga apuntando al mismo cursor.
    pub fn sort_by_position(&mut self) {
        // Recordar la posición del primario antes de ordenar
        let primary_pos = self.cursors[self.primary_index].position;

        self.cursors.sort_by(|a, b| a.position.cmp(&b.position));

        // Re-encontrar el primario por posición
        self.primary_index = self
            .cursors
            .iter()
            .position(|c| c.position == primary_pos)
            .unwrap_or(0);
    }
}

impl Default for MultiCursorState {
    fn default() -> Self {
        Self::new()
    }
}
