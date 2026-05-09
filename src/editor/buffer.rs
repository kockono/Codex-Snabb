//! Buffer editable basado en `Vec<String>`.
//!
//! Implementación simple y medible para MVP. Una línea por elemento del Vec.
//! Si los benchmarks justifican migrar a rope/piece table, se hará después
//! — nunca por moda. La API pública se mantiene estable independientemente
//! de la representación interna.
//!
//! Invariantes:
//! - `lines` siempre tiene al menos 1 elemento (buffer vacío = vec![""])
//! - `dirty` se marca en cualquier mutación de contenido
//! - `file_path` es `Some` solo si el buffer se asoció a un archivo

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::cursor::Position;

/// Buffer de texto editable — almacena contenido como líneas individuales.
///
/// Cada línea es un `String` independiente. El buffer siempre tiene al menos
/// una línea (vacía). Las operaciones de edición marcan `dirty = true`.
#[derive(Debug)]
pub struct TextBuffer {
    /// Contenido del buffer — siempre >= 1 elemento.
    lines: Vec<String>,
    /// Si el contenido cambió desde la última vez que se guardó.
    dirty: bool,
    /// Path del archivo asociado, si existe.
    file_path: Option<PathBuf>,
}

impl TextBuffer {
    /// Crea un buffer vacío sin archivo asociado.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            dirty: false,
            file_path: None,
        }
    }

    /// Crea un buffer a partir de texto plano.
    ///
    /// Divide el texto por líneas. Un texto vacío produce un buffer
    /// con una sola línea vacía (invariante del buffer).
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let lines = if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        };
        Self {
            lines,
            dirty: false,
            file_path: None,
        }
    }

    /// Crea un buffer leyendo un archivo desde disco.
    ///
    /// Lectura síncrona por ahora — se hará async en una épica posterior.
    /// El path se asocia al buffer para `save()`.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("no se pudo leer el archivo: {}", path.display()))?;
        let mut buffer = Self::from_text(&content);
        // CLONE: necesario para almacenar ownership del path en el buffer
        buffer.file_path = Some(path.to_path_buf());
        Ok(buffer)
    }

    /// Guarda el contenido al path asociado.
    ///
    /// Falla si no hay path asociado. Marca `dirty = false` al guardar.
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("buffer sin archivo asociado — usá save_as()")?;
        let content = self.lines.join("\n");
        std::fs::write(path, &content)
            .with_context(|| format!("no se pudo guardar: {}", path.display()))?;
        self.dirty = false;
        Ok(())
    }

    /// Guarda el contenido a un path específico y lo asocia al buffer.
    pub fn save_as(&mut self, path: &Path) -> Result<()> {
        let content = self.lines.join("\n");
        std::fs::write(path, &content)
            .with_context(|| format!("no se pudo guardar: {}", path.display()))?;
        // CLONE: necesario para almacenar ownership del nuevo path
        self.file_path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Cantidad total de líneas en el buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Retorna una referencia a la línea en el índice dado, si existe.
    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    /// Retorna un slice de líneas para el viewport.
    ///
    /// `start` es el índice de la primera línea, `count` cuántas se piden.
    /// Si `start` está fuera de rango, retorna un slice vacío.
    /// Si `start + count` excede el buffer, retorna hasta el final.
    #[expect(
        dead_code,
        reason = "se usará para renderizar líneas visibles del viewport"
    )]
    pub fn lines_range(&self, start: usize, count: usize) -> &[String] {
        if start >= self.lines.len() {
            return &[];
        }
        let end = (start + count).min(self.lines.len());
        &self.lines[start..end]
    }

    /// Inserta un carácter en la posición dada.
    ///
    /// Si la posición está fuera de rango, no hace nada.
    /// La columna se clampea al largo de la línea.
    pub fn insert_char(&mut self, pos: Position, ch: char) {
        if pos.line >= self.lines.len() {
            return;
        }
        let line = &mut self.lines[pos.line];
        let col = pos.col.min(line.len());
        line.insert(col, ch);
        self.dirty = true;
    }

    /// Elimina el carácter antes de la posición (backspace).
    ///
    /// Si estamos al inicio de una línea (col == 0), une la línea con la anterior.
    /// Retorna el carácter eliminado (para undo) o `None` si no hay nada que borrar.
    pub fn delete_char(&mut self, pos: Position) -> Option<char> {
        if pos.line >= self.lines.len() {
            return None;
        }

        if pos.col > 0 {
            // Borrar carácter dentro de la línea
            let line = &mut self.lines[pos.line];
            let col = pos.col.min(line.len());
            if col == 0 {
                return None;
            }
            let ch = line.remove(col - 1);
            self.dirty = true;
            Some(ch)
        } else if pos.line > 0 {
            // Al inicio de línea: unir con la anterior (comportamiento de backspace)
            // Se retorna '\n' como indicador de que se unió una línea
            let current_line = self.lines.remove(pos.line);
            self.lines[pos.line - 1].push_str(&current_line);
            self.dirty = true;
            Some('\n')
        } else {
            // Inicio del buffer — nada que borrar
            None
        }
    }

    /// Inserta un salto de línea en la posición dada (Enter).
    ///
    /// Divide la línea actual en dos: el contenido antes del cursor
    /// queda en la línea actual, el contenido después pasa a una nueva línea.
    pub fn insert_newline(&mut self, pos: Position) {
        if pos.line >= self.lines.len() {
            return;
        }
        let line = &mut self.lines[pos.line];
        let col = pos.col.min(line.len());
        let remainder = line.split_off(col);
        self.lines.insert(pos.line + 1, remainder);
        self.dirty = true;
    }

    /// Elimina una línea completa por índice.
    ///
    /// Si solo queda una línea, la vacía en vez de eliminarla
    /// (mantiene invariante de al menos 1 línea).
    pub fn delete_line(&mut self, line: usize) {
        if line >= self.lines.len() {
            return;
        }
        if self.lines.len() == 1 {
            self.lines[0].clear();
        } else {
            self.lines.remove(line);
        }
        self.dirty = true;
    }

    /// Si el contenido fue modificado desde el último guardado.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Path del archivo asociado, si existe.
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// Longitud de una línea específica en bytes.
    ///
    /// Retorna 0 si la línea no existe.
    pub fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, String::len)
    }

    /// Reconstruye la línea que fue dividida por un newline (para undo).
    ///
    /// Une la línea `line` con la siguiente, eliminando la siguiente.
    /// Usado por undo de `InsertNewline`.
    pub fn join_lines(&mut self, line: usize) {
        if line + 1 >= self.lines.len() {
            return;
        }
        let next = self.lines.remove(line + 1);
        self.lines[line].push_str(&next);
        self.dirty = true;
    }

    /// Inserta una línea nueva en la posición dada, empujando las demás.
    ///
    /// Usado por undo de `DeleteNewline` (unión de líneas).
    pub fn split_line_at(&mut self, line: usize, col: usize) {
        if line >= self.lines.len() {
            return;
        }
        let current = &mut self.lines[line];
        let clamped_col = col.min(current.len());
        let remainder = current.split_off(clamped_col);
        self.lines.insert(line + 1, remainder);
        self.dirty = true;
    }

    /// Intercambia dos líneas del buffer.
    ///
    /// Operación O(1): usa `Vec::swap` que solo intercambia los punteros
    /// internos del Vec, sin allocar ni mover bytes. Si `a == b`, o si
    /// alguno de los índices está fuera de rango, no hace nada y no
    /// marca el buffer como dirty.
    pub fn swap_lines(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        if a >= self.lines.len() || b >= self.lines.len() {
            return;
        }
        self.lines.swap(a, b);
        self.dirty = true;
    }

    /// Reemplaza el contenido completo de una línea.
    ///
    /// `new` toma ownership del nuevo contenido — esto evita una alocación
    /// extra cuando el caller ya tiene la nueva línea owned (típico tras
    /// `toggle_comment`). Si `idx` está fuera de rango, no hace nada y
    /// no marca el buffer como dirty.
    pub fn replace_line(&mut self, idx: usize, new: String) {
        if idx >= self.lines.len() {
            return;
        }
        self.lines[idx] = new;
        self.dirty = true;
    }

    /// Inserta `content` como nueva línea en el índice `idx`, desplazando
    /// las siguientes una posición hacia abajo.
    ///
    /// Si `idx` excede `line_count()`, la línea se anexa al final
    /// (clamp a `line_count()`). `content` toma ownership — sin alocaciones
    /// extra cuando el caller ya tiene el `String` owned. Marca `dirty = true`.
    ///
    /// O(n) por el `Vec::insert` — aceptable para lógica del editor (no render).
    pub fn insert_line(&mut self, idx: usize, content: String) {
        let target = idx.min(self.lines.len());
        self.lines.insert(target, content);
        self.dirty = true;
    }

    /// Inserta un carácter para undo/redo sin re-marcar dirty
    /// (ya se marcó en la operación original).
    ///
    /// Nota: dirty ya se maneja correctamente porque las operaciones de undo
    /// sí mutan contenido.
    pub fn raw_insert_char(&mut self, pos: Position, ch: char) {
        self.insert_char(pos, ch);
    }

    /// Elimina un carácter en una posición específica (no backspace, sino forward delete).
    ///
    /// Usado por undo de `InsertChar` — elimina el carácter EN la posición, no antes.
    pub fn remove_char_at(&mut self, pos: Position) -> Option<char> {
        if pos.line >= self.lines.len() {
            return None;
        }
        let line = &mut self.lines[pos.line];
        if pos.col >= line.len() {
            return None;
        }
        let ch = line.remove(pos.col);
        self.dirty = true;
        Some(ch)
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── swap_lines ──

    #[test]
    fn swap_lines_swaps_adjacent_lines() {
        let mut buf = TextBuffer::from_text("a\nb\nc");
        buf.swap_lines(0, 1);
        assert_eq!(buf.line(0), Some("b"));
        assert_eq!(buf.line(1), Some("a"));
        assert_eq!(buf.line(2), Some("c"));
    }

    #[test]
    fn swap_lines_marks_dirty() {
        let mut buf = TextBuffer::from_text("a\nb");
        assert!(!buf.is_dirty());
        buf.swap_lines(0, 1);
        assert!(buf.is_dirty());
    }

    #[test]
    fn swap_lines_same_index_is_noop() {
        let mut buf = TextBuffer::from_text("a\nb");
        buf.swap_lines(0, 0);
        assert_eq!(buf.line(0), Some("a"));
        assert_eq!(buf.line(1), Some("b"));
        // Same-index swap should not flip dirty either
        assert!(!buf.is_dirty());
    }

    #[test]
    fn swap_lines_out_of_range_is_noop() {
        let mut buf = TextBuffer::from_text("a\nb");
        buf.swap_lines(0, 99);
        assert_eq!(buf.line(0), Some("a"));
        assert_eq!(buf.line(1), Some("b"));
        assert!(!buf.is_dirty());
    }

    #[test]
    fn swap_lines_far_apart() {
        let mut buf = TextBuffer::from_text("a\nb\nc\nd");
        buf.swap_lines(0, 3);
        assert_eq!(buf.line(0), Some("d"));
        assert_eq!(buf.line(1), Some("b"));
        assert_eq!(buf.line(2), Some("c"));
        assert_eq!(buf.line(3), Some("a"));
    }

    // ── replace_line ──

    #[test]
    fn replace_line_replaces_content() {
        let mut buf = TextBuffer::from_text("foo\nbar");
        buf.replace_line(0, String::from("hello"));
        assert_eq!(buf.line(0), Some("hello"));
        assert_eq!(buf.line(1), Some("bar"));
    }

    #[test]
    fn replace_line_marks_dirty() {
        let mut buf = TextBuffer::from_text("foo");
        assert!(!buf.is_dirty());
        buf.replace_line(0, String::from("bar"));
        assert!(buf.is_dirty());
    }

    #[test]
    fn replace_line_out_of_range_is_noop() {
        let mut buf = TextBuffer::from_text("foo");
        buf.replace_line(99, String::from("bar"));
        assert_eq!(buf.line(0), Some("foo"));
        assert!(!buf.is_dirty());
    }

    #[test]
    fn replace_line_with_empty_string() {
        let mut buf = TextBuffer::from_text("foo\nbar");
        buf.replace_line(0, String::new());
        assert_eq!(buf.line(0), Some(""));
        assert_eq!(buf.line(1), Some("bar"));
    }

    // ── insert_line ──

    #[test]
    fn insert_line_at_start_shifts_existing_lines() {
        let mut buf = TextBuffer::from_text("a\nb");
        buf.insert_line(0, String::from("zero"));
        assert_eq!(buf.line(0), Some("zero"));
        assert_eq!(buf.line(1), Some("a"));
        assert_eq!(buf.line(2), Some("b"));
        assert_eq!(buf.line_count(), 3);
    }

    #[test]
    fn insert_line_in_middle() {
        let mut buf = TextBuffer::from_text("a\nb\nc");
        buf.insert_line(1, String::from("middle"));
        assert_eq!(buf.line(0), Some("a"));
        assert_eq!(buf.line(1), Some("middle"));
        assert_eq!(buf.line(2), Some("b"));
        assert_eq!(buf.line(3), Some("c"));
    }

    #[test]
    fn insert_line_at_end_appends() {
        let mut buf = TextBuffer::from_text("a\nb");
        let line_count = buf.line_count();
        buf.insert_line(line_count, String::from("end"));
        assert_eq!(buf.line(0), Some("a"));
        assert_eq!(buf.line(1), Some("b"));
        assert_eq!(buf.line(2), Some("end"));
    }

    #[test]
    fn insert_line_beyond_range_clamps_to_end() {
        let mut buf = TextBuffer::from_text("a\nb");
        buf.insert_line(999, String::from("x"));
        // Triangulation: idx out of range MUST not panic — clamp to end.
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.line(2), Some("x"));
    }

    #[test]
    fn insert_line_marks_dirty() {
        let mut buf = TextBuffer::from_text("a");
        assert!(!buf.is_dirty());
        buf.insert_line(0, String::from("new"));
        assert!(buf.is_dirty());
    }
}
