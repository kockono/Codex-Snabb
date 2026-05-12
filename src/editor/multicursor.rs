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
    ///
    /// Respeta char boundaries multi-byte UTF-8.
    pub fn move_left(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        if self.position.col > 0 {
            let line = buffer.line(self.position.line).unwrap_or("");
            self.position.col = super::unicode::prev_char_boundary(line, self.position.col);
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
    ///
    /// Respeta char boundaries multi-byte UTF-8.
    pub fn move_right(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);
        let line_len = buffer.line_len(self.position.line);
        if self.position.col < line_len {
            let line = buffer.line(self.position.line).unwrap_or("");
            self.position.col += super::unicode::char_len_at(line, self.position.col);
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

    /// Mueve el cursor al inicio de la palabra anterior (Ctrl+Left).
    ///
    /// Reglas (estilo VS Code/IntelliJ), implementadas char-aware:
    /// - Si `col > 0`, retrocede saltando whitespace y luego retrocede mientras
    ///   los chars sean de la misma clase (word vs non-word). Para clusters de
    ///   non-word-non-space (ej `;}`), retrocede hasta cambiar de clase.
    /// - Si ya está en `col == 0`, salta al final de la línea anterior.
    /// - Soporta multi-byte UTF-8: avanza/retrocede por char boundaries.
    pub fn move_word_left(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);

        // Caso degenerado: inicio de línea → ir al final de la anterior
        if self.position.col == 0 {
            if self.position.line > 0 {
                self.position.line -= 1;
                self.position.col = buffer.line_len(self.position.line);
            }
            self.desired_col = self.position.col;
            if selecting {
                self.extend_selection();
            }
            return;
        }

        let line = match buffer.line(self.position.line) {
            Some(l) => l,
            None => {
                if selecting {
                    self.extend_selection();
                }
                return;
            }
        };

        let mut col = self.position.col.min(line.len());

        // Helper: char a la izquierda de `col` (None si col == 0).
        let char_left_of = |c: usize| -> Option<char> {
            if c == 0 {
                return None;
            }
            let prev = super::unicode::prev_char_boundary(line, c);
            line[prev..].chars().next()
        };

        // 1) Saltar whitespace que esté a la izquierda (retrocediendo por chars)
        while let Some(ch) = char_left_of(col) {
            if ch.is_whitespace() {
                col = super::unicode::prev_char_boundary(line, col);
            } else {
                break;
            }
        }

        // 2) Saltar el cluster de la misma clase (word vs non-word).
        //    Clase determinada por el char inmediatamente a la izquierda.
        if let Some(first) = char_left_of(col) {
            let in_word = super::is_word_char(first);
            while let Some(ch) = char_left_of(col) {
                if ch.is_whitespace() || super::is_word_char(ch) != in_word {
                    break;
                }
                col = super::unicode::prev_char_boundary(line, col);
            }
        }

        self.position.col = col;
        self.desired_col = col;
        if selecting {
            self.extend_selection();
        }
    }

    /// Mueve el cursor al final de la palabra siguiente (Ctrl+Right).
    ///
    /// Reglas char-aware:
    /// - Avanza saltando el cluster actual (mismo clase: word vs non-word, sin
    ///   contar whitespace).
    /// - Salta whitespace que siga.
    /// - Si está al final de la línea, salta al inicio de la siguiente.
    /// - Soporta multi-byte UTF-8.
    pub fn move_word_right(&mut self, buffer: &TextBuffer, selecting: bool) {
        self.handle_selection_mode(selecting);

        let line = match buffer.line(self.position.line) {
            Some(l) => l,
            None => {
                if selecting {
                    self.extend_selection();
                }
                return;
            }
        };
        let line_len = line.len();

        // Caso degenerado: final de línea → ir al inicio de la siguiente
        if self.position.col >= line_len {
            if self.position.line + 1 < buffer.line_count() {
                self.position.line += 1;
                self.position.col = 0;
            }
            self.desired_col = self.position.col;
            if selecting {
                self.extend_selection();
            }
            return;
        }

        let mut col = self.position.col;

        // Helper: char en `col` (None si fuera de rango o no boundary).
        let char_at = |c: usize| -> Option<char> {
            if c >= line.len() || !line.is_char_boundary(c) {
                return None;
            }
            line[c..].chars().next()
        };

        // 1) Saltar el cluster de la misma clase (si no estamos en whitespace)
        if let Some(first) = char_at(col) {
            if !first.is_whitespace() {
                let in_word = super::is_word_char(first);
                while let Some(ch) = char_at(col) {
                    if ch.is_whitespace() || super::is_word_char(ch) != in_word {
                        break;
                    }
                    col += super::unicode::char_len_at(line, col);
                }
            }
        }

        // 2) Saltar whitespace que siga
        while let Some(ch) = char_at(col) {
            if ch.is_whitespace() {
                col += super::unicode::char_len_at(line, col);
            } else {
                break;
            }
        }

        self.position.col = col;
        self.desired_col = col;
        if selecting {
            self.extend_selection();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor_at(line: usize, col: usize) -> CursorInstance {
        CursorInstance::new(Position { line, col })
    }

    // ── move_word_right ──

    #[test]
    fn move_word_right_jumps_to_end_of_word() {
        let buf = TextBuffer::from_text("hello world");
        let mut c = cursor_at(0, 0);
        c.move_word_right(&buf, false);
        // "hello" → 5, then skip space → 6
        assert_eq!(c.position, Position { line: 0, col: 6 });
    }

    #[test]
    fn move_word_right_at_end_jumps_to_next_line() {
        let buf = TextBuffer::from_text("foo\nbar");
        let mut c = cursor_at(0, 3);
        c.move_word_right(&buf, false);
        assert_eq!(c.position, Position { line: 1, col: 0 });
    }

    #[test]
    fn move_word_right_in_whitespace_skips_to_next_word() {
        let buf = TextBuffer::from_text("    foo");
        let mut c = cursor_at(0, 0);
        c.move_word_right(&buf, false);
        // Whitespace skip puts cursor at 4 (start of foo)
        assert_eq!(c.position, Position { line: 0, col: 4 });
    }

    #[test]
    fn move_word_right_through_punctuation() {
        let buf = TextBuffer::from_text("a;b");
        let mut c = cursor_at(0, 0);
        c.move_word_right(&buf, false); // word "a" → 1
        assert_eq!(c.position.col, 1);
        c.move_word_right(&buf, false); // ";" punct cluster → 2
        assert_eq!(c.position.col, 2);
    }

    // ── move_word_left ──

    #[test]
    fn move_word_left_jumps_to_start_of_word() {
        let buf = TextBuffer::from_text("hello world");
        let mut c = cursor_at(0, 11);
        c.move_word_left(&buf, false);
        // From end of "world", goes to start of "world" → 6
        assert_eq!(c.position, Position { line: 0, col: 6 });
    }

    #[test]
    fn move_word_left_at_start_jumps_to_prev_line_end() {
        let buf = TextBuffer::from_text("foo\nbar");
        let mut c = cursor_at(1, 0);
        c.move_word_left(&buf, false);
        assert_eq!(c.position, Position { line: 0, col: 3 });
    }

    #[test]
    fn move_word_left_skips_trailing_whitespace() {
        let buf = TextBuffer::from_text("foo    ");
        let mut c = cursor_at(0, 7);
        c.move_word_left(&buf, false);
        // skip 4 spaces, then move over "foo" to col 0
        assert_eq!(c.position, Position { line: 0, col: 0 });
    }

    #[test]
    fn move_word_left_at_buffer_start_is_noop() {
        let buf = TextBuffer::from_text("foo");
        let mut c = cursor_at(0, 0);
        c.move_word_left(&buf, false);
        assert_eq!(c.position, Position { line: 0, col: 0 });
    }

    // ── selection mode ──

    #[test]
    fn move_word_right_selecting_creates_selection() {
        let buf = TextBuffer::from_text("hello world");
        let mut c = cursor_at(0, 0);
        c.move_word_right(&buf, true);
        assert!(c.has_selection());
    }

    #[test]
    fn move_word_left_non_selecting_clears_selection() {
        let buf = TextBuffer::from_text("hello world");
        let mut c = cursor_at(0, 5);
        c.start_selection();
        c.move_word_left(&buf, false);
        assert!(!c.has_selection());
    }

    // ── unicode-aware char movement ──

    #[test]
    fn move_right_through_two_byte_chars() {
        // "héllo" — h(1) é(2) l(1) l(1) o(1) = 6 bytes
        let buf = TextBuffer::from_text("héllo");
        let mut c = cursor_at(0, 0);
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 1); // h → é
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 3); // é → l (saltó 2 bytes)
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 4);
    }

    #[test]
    fn move_left_through_two_byte_chars() {
        let buf = TextBuffer::from_text("código");
        let mut c = cursor_at(0, 7); // fin de línea (7 bytes)
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 6); // o → g
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 5);
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 4);
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 3); // i → d
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 1); // d → ó (saltó 2 bytes)
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 0); // ó → c
    }

    #[test]
    fn move_right_through_emoji() {
        // "😀hello" — 😀 = 4 bytes
        let buf = TextBuffer::from_text("😀hello");
        let mut c = cursor_at(0, 0);
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 4); // 😀 → h
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 5);
    }

    #[test]
    fn move_right_through_cjk() {
        // "中文" — 中 = 3 bytes, 文 = 3 bytes
        let buf = TextBuffer::from_text("中文");
        let mut c = cursor_at(0, 0);
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 3);
        c.move_right(&buf, false);
        assert_eq!(c.position.col, 6);
    }

    #[test]
    fn move_left_at_emoji_does_not_panic() {
        let buf = TextBuffer::from_text("😀");
        let mut c = cursor_at(0, 4);
        c.move_left(&buf, false);
        assert_eq!(c.position.col, 0);
    }

    // ── unicode-aware word movement ──

    #[test]
    fn move_word_right_through_spanish_text() {
        // "código fuente" — código bytes: c(1) ó(2) d(1) i(1) g(1) o(1) = 7 bytes.
        // Pero `is_word_char` (ASCII-only) considera 'ó' como NO word.
        // Por lo tanto "código" se ve como cluster1="c" + non-word "ó" + cluster2="digo".
        // Desde col 0 con primer char 'c' (word), el cluster word va de col 0..1,
        // luego whitespace none → cursor termina en col 1.
        let buf = TextBuffer::from_text("código fuente");
        let mut c = cursor_at(0, 0);
        c.move_word_right(&buf, false);
        // 'c' es word, 'ó' es non-word → cluster word termina en col 1.
        assert_eq!(c.position.col, 1);
    }

    #[test]
    fn move_word_right_skip_through_unicode_no_panic() {
        // Verifica el invariante crítico: NO panic con multi-byte chars.
        let buf = TextBuffer::from_text("año pasó rápido");
        let mut c = cursor_at(0, 0);
        for _ in 0..20 {
            c.move_word_right(&buf, false);
        }
        // No debe haber panic; col debe estar en boundary.
        let line = buf.line(0).unwrap();
        assert!(line.is_char_boundary(c.position.col));
    }

    #[test]
    fn move_word_left_through_spanish_no_panic() {
        let buf = TextBuffer::from_text("código fuente");
        let line = buf.line(0).unwrap();
        let mut c = cursor_at(0, line.len());
        for _ in 0..20 {
            c.move_word_left(&buf, false);
        }
        assert!(line.is_char_boundary(c.position.col));
    }
}
