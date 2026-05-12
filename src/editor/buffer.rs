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

    /// Asocia un path al buffer sin tocar el contenido ni el flag `dirty`.
    ///
    /// Útil para crear placeholders de tabs (ej: imágenes) donde queremos
    /// que el tab muestre el nombre del archivo aunque el buffer esté vacío.
    pub fn set_file_path(&mut self, path: PathBuf) {
        self.file_path = Some(path);
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

    /// Inserta `text` en `pos` (byte offset). Maneja `\n` embebido dividiendo
    /// líneas. Retorna la `Position` inmediatamente después del último byte
    /// insertado — útil para reposicionar el cursor.
    ///
    /// **Invariante**: `pos.col` DEBE ser un char boundary válido. Si no lo es,
    /// el `String::insert_str` o `split_off` interno panicará — lo cual es
    /// **correcto**: indica que el caller violó la invariante de `Position.col`
    /// (ver `cursor.rs:21`). El panic es preferible a corrupción silenciosa.
    ///
    /// Si `pos.line` está fuera de rango, no hace nada y retorna `pos`.
    pub fn insert_text(&mut self, pos: Position, text: &str) -> Position {
        if pos.line >= self.lines.len() {
            return pos;
        }
        if text.is_empty() {
            return pos;
        }

        // Caso simple: no hay newlines — insertar en la línea actual.
        if !text.contains('\n') {
            let line = &mut self.lines[pos.line];
            let col = pos.col.min(line.len());
            line.insert_str(col, text);
            self.dirty = true;
            return Position {
                line: pos.line,
                col: col + text.len(),
            };
        }

        // Caso multilínea: dividir el texto por '\n', construir nuevas líneas.
        // Estrategia:
        //   line_actual = "AAAA[pos.col]BBBB"
        //   text        = "X\nY\nZ"
        //   resultado:
        //     line_actual = "AAAAX"
        //     [insertadas] "Y"
        //     line_siguiente = "ZBBBB"  (esta línea es "Z" + el "right" original)
        let line = &mut self.lines[pos.line];
        let col = pos.col.min(line.len());
        // CLONE: split_off mueve la cola; necesitamos el "right" como String owned
        //        para reconstruir la última línea con el último piece + right.
        let right: String = line.split_off(col);

        // Recolectar los pieces del split — sin alocar el Vec si no hace falta:
        //   text.split('\n') es lazy y barato.
        // Para reconstrucción necesitamos: primer piece va al final de la línea
        // actual; pieces intermedios son líneas nuevas; último piece + right
        // es la línea final.
        let mut iter = text.split('\n');
        // SAFETY: text es no vacío y contiene al menos un '\n' (chequeado arriba),
        //         por ende split('\n') tiene >= 2 elementos.
        let first = iter.next().unwrap_or("");
        line.push_str(first);

        // Recolectar el resto en un Vec para conocer cuál es el último.
        // Capacidad estimada por nº de '\n' — 1 alloc.
        let nl_count = text.bytes().filter(|&b| b == b'\n').count();
        let mut rest: Vec<&str> = Vec::with_capacity(nl_count);
        for piece in iter {
            rest.push(piece);
        }
        // SAFETY: nl_count >= 1 garantiza rest no vacío.
        let last_piece = rest.pop().unwrap_or("");

        // Insertar líneas intermedias (pieces) en orden.
        // Position de inserción: pos.line + 1, +2, ... (cada uno empuja al siguiente).
        let mut insert_at = pos.line + 1;
        for piece in rest {
            // CLONE: piece es &str del input; insert_line consume String.
            self.lines.insert(insert_at, piece.to_string());
            insert_at += 1;
        }

        // Última línea: last_piece + right (el "BBBB" original).
        // Construirla con capacidad exacta para evitar realloc.
        let mut last_line = String::with_capacity(last_piece.len() + right.len());
        last_line.push_str(last_piece);
        last_line.push_str(&right);
        self.lines.insert(insert_at, last_line);

        self.dirty = true;
        Position {
            line: insert_at,
            col: last_piece.len(),
        }
    }

    /// Elimina el rango `[start..end]` del buffer y retorna el texto borrado.
    ///
    /// Maneja rangos multi-línea uniendo el remanente. El texto retornado se
    /// usa típicamente para undo (`EditOperation::DeleteRange`).
    ///
    /// **Invariante**: `start.col` y `end.col` DEBEN ser char boundaries.
    /// Si no lo son, panicará en `String::drain` o slicing — comportamiento
    /// correcto frente a invariante violada.
    ///
    /// Si `start >= end`, retorna `String` vacío.
    /// Si las líneas están fuera de rango, retorna `String` vacío.
    pub fn delete_range(&mut self, start: Position, end: Position) -> String {
        if start >= end {
            return String::new();
        }
        if start.line >= self.lines.len() || end.line >= self.lines.len() {
            return String::new();
        }

        if start.line == end.line {
            // Caso simple: una sola línea — drain del rango.
            let line = &mut self.lines[start.line];
            let s = start.col.min(line.len());
            let e = end.col.min(line.len());
            if s >= e {
                return String::new();
            }
            let removed: String = line.drain(s..e).collect();
            self.dirty = true;
            return removed;
        }

        // Caso multi-línea: armar el texto eliminado, luego reconstruir.
        // 1. Reservar capacidad estimada para el resultado.
        // 2. Recolectar:
        //    - desde start.col hasta el fin de start.line  (+ '\n')
        //    - líneas intermedias completas                (+ '\n' cada una)
        //    - desde el inicio de end.line hasta end.col   (sin '\n' final)
        // 3. Reescribir start.line = left_de_start + right_de_end
        // 4. Eliminar líneas (start.line + 1 ..= end.line)
        let estimated = self.lines[start.line..=end.line]
            .iter()
            .map(String::len)
            .sum::<usize>()
            + (end.line - start.line); // newlines
        let mut removed = String::with_capacity(estimated);

        // Tomar el "left" de start.line: lo que sobrevive desde 0 hasta start.col.
        // CLONE: necesitamos copiar `left` antes de mutar self.lines[start.line].
        //        Justifica clone: no hay forma de tomar slice y mutar el mismo
        //        Vec<String> sin desreferencia inestable.
        let left_of_start: String = {
            let line = &self.lines[start.line];
            let s = start.col.min(line.len());
            line[..s].to_string()
        };
        // Append parte borrada desde la primera línea.
        {
            let line = &self.lines[start.line];
            let s = start.col.min(line.len());
            removed.push_str(&line[s..]);
            removed.push('\n');
        }
        // Líneas intermedias completas.
        for idx in (start.line + 1)..end.line {
            removed.push_str(&self.lines[idx]);
            removed.push('\n');
        }
        // Última línea: 0..end.col.
        let right_of_end: String = {
            let line = &self.lines[end.line];
            let e = end.col.min(line.len());
            removed.push_str(&line[..e]);
            // Y guardamos el remanente desde end.col hasta el final.
            line[e..].to_string()
        };

        // Reescribir self.lines[start.line] = left + right.
        let mut new_first = String::with_capacity(left_of_start.len() + right_of_end.len());
        new_first.push_str(&left_of_start);
        new_first.push_str(&right_of_end);
        self.lines[start.line] = new_first;

        // Eliminar las líneas (start.line + 1 ..= end.line).
        // drain consume y descarta — O(n) en el largo del rango.
        self.lines.drain((start.line + 1)..=end.line);

        self.dirty = true;
        removed
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

    // ── insert_text — single line ──

    #[test]
    fn insert_text_ascii_single_line() {
        let mut buf = TextBuffer::from_text("abcdef");
        let end = buf.insert_text(Position { line: 0, col: 3 }, "XYZ");
        assert_eq!(buf.line(0), Some("abcXYZdef"));
        assert_eq!(end, Position { line: 0, col: 6 });
        assert!(buf.is_dirty());
    }

    #[test]
    fn insert_text_multibyte_single_line() {
        // "código" — bytes: c(1) ó(2) d(1) i(1) g(1) o(1) = 7 bytes
        let mut buf = TextBuffer::from_text("código");
        // Insertar "ñ" (2 bytes) en col=1 (después de 'c')
        let end = buf.insert_text(Position { line: 0, col: 1 }, "ñ");
        assert_eq!(buf.line(0), Some("cñódigo"));
        assert_eq!(end, Position { line: 0, col: 3 }); // 1 + 2 bytes
    }

    #[test]
    fn insert_text_emoji() {
        let mut buf = TextBuffer::from_text("hi");
        let end = buf.insert_text(Position { line: 0, col: 0 }, "😀");
        assert_eq!(buf.line(0), Some("😀hi"));
        assert_eq!(end, Position { line: 0, col: 4 });
    }

    #[test]
    fn insert_text_empty_string_is_noop() {
        let mut buf = TextBuffer::from_text("hello");
        let end = buf.insert_text(Position { line: 0, col: 2 }, "");
        assert_eq!(buf.line(0), Some("hello"));
        assert_eq!(end, Position { line: 0, col: 2 });
        assert!(!buf.is_dirty());
    }

    // ── insert_text — multiline ──

    #[test]
    fn insert_text_with_single_newline_splits_line() {
        let mut buf = TextBuffer::from_text("AAAABBBB");
        // Insertar "X\nY" en col=4 → AAAA + X | Y + BBBB
        let end = buf.insert_text(Position { line: 0, col: 4 }, "X\nY");
        assert_eq!(buf.line(0), Some("AAAAX"));
        assert_eq!(buf.line(1), Some("YBBBB"));
        assert_eq!(end, Position { line: 1, col: 1 });
    }

    #[test]
    fn insert_text_multibyte_multiline() {
        // "código" tiene ó=2 bytes
        let mut buf = TextBuffer::from_text("código");
        // Insertar "ñ\nó" (ñ=2 bytes, ó=2 bytes) en col=1 (después de c)
        let end = buf.insert_text(Position { line: 0, col: 1 }, "ñ\nó");
        // línea 0: "c" + "ñ" = "cñ" (3 bytes)
        // línea 1: "ó" + "ódigo" = "óódigo"
        assert_eq!(buf.line(0), Some("cñ"));
        assert_eq!(buf.line(1), Some("óódigo"));
        assert_eq!(end, Position { line: 1, col: 2 }); // ó = 2 bytes
    }

    #[test]
    fn insert_text_three_lines() {
        let mut buf = TextBuffer::from_text("AB");
        let end = buf.insert_text(Position { line: 0, col: 1 }, "X\nY\nZ");
        assert_eq!(buf.line(0), Some("AX"));
        assert_eq!(buf.line(1), Some("Y"));
        assert_eq!(buf.line(2), Some("ZB"));
        assert_eq!(end, Position { line: 2, col: 1 });
    }

    #[test]
    fn insert_text_at_end_of_line() {
        let mut buf = TextBuffer::from_text("hello");
        let end = buf.insert_text(Position { line: 0, col: 5 }, " world");
        assert_eq!(buf.line(0), Some("hello world"));
        assert_eq!(end, Position { line: 0, col: 11 });
    }

    #[test]
    fn insert_text_out_of_range_line_is_noop() {
        let mut buf = TextBuffer::from_text("hello");
        let end = buf.insert_text(Position { line: 99, col: 0 }, "x");
        assert_eq!(buf.line(0), Some("hello"));
        assert_eq!(end, Position { line: 99, col: 0 });
        assert!(!buf.is_dirty());
    }

    // ── delete_range — single line ──

    #[test]
    fn delete_range_ascii_same_line() {
        let mut buf = TextBuffer::from_text("abcdef");
        let removed = buf.delete_range(
            Position { line: 0, col: 1 },
            Position { line: 0, col: 4 },
        );
        assert_eq!(removed, "bcd");
        assert_eq!(buf.line(0), Some("aef"));
        assert!(buf.is_dirty());
    }

    #[test]
    fn delete_range_multibyte_same_line() {
        // "código" — ó está en bytes 1..3
        let mut buf = TextBuffer::from_text("código");
        let removed = buf.delete_range(
            Position { line: 0, col: 1 },
            Position { line: 0, col: 3 },
        );
        assert_eq!(removed, "ó");
        assert_eq!(buf.line(0), Some("cdigo"));
    }

    #[test]
    fn delete_range_empty_when_start_eq_end() {
        let mut buf = TextBuffer::from_text("hello");
        let removed = buf.delete_range(
            Position { line: 0, col: 2 },
            Position { line: 0, col: 2 },
        );
        assert_eq!(removed, "");
        assert_eq!(buf.line(0), Some("hello"));
        assert!(!buf.is_dirty());
    }

    // ── delete_range — multiline ──

    #[test]
    fn delete_range_across_two_lines() {
        let mut buf = TextBuffer::from_text("hello\nworld");
        // Borrar desde col 2 línea 0 hasta col 3 línea 1 → "llo\nwor"
        let removed = buf.delete_range(
            Position { line: 0, col: 2 },
            Position { line: 1, col: 3 },
        );
        assert_eq!(removed, "llo\nwor");
        assert_eq!(buf.line(0), Some("held"));
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn delete_range_across_three_lines() {
        let mut buf = TextBuffer::from_text("aaa\nbbb\nccc");
        let removed = buf.delete_range(
            Position { line: 0, col: 1 },
            Position { line: 2, col: 1 },
        );
        assert_eq!(removed, "aa\nbbb\nc");
        assert_eq!(buf.line(0), Some("acc"));
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn delete_range_multibyte_multiline() {
        let mut buf = TextBuffer::from_text("ñoño\ncódigo");
        // Borrar desde col 2 línea 0 (después de "ñ") hasta col 3 línea 1 (después de "có")
        // ñ=2, o=1, ñ=2, o=1 → "ñoño" tiene bytes: ñ(2) o(1) ñ(2) o(1) = 6 bytes
        // Borrar [2..6] de línea 0 = "oño", + "\n", + [0..3] de línea 1 = "có" → "oño\ncó"
        let removed = buf.delete_range(
            Position { line: 0, col: 2 },
            Position { line: 1, col: 3 },
        );
        assert_eq!(removed, "oño\ncó");
        // línea 0 sobrevive: "ñ" (bytes 0..2) + "digo" (bytes 3..7 de "código") = "ñdigo"
        assert_eq!(buf.line(0), Some("ñdigo"));
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn delete_range_full_line() {
        let mut buf = TextBuffer::from_text("aaa\nbbb\nccc");
        // Borrar línea 1 entera
        let removed = buf.delete_range(
            Position { line: 0, col: 3 }, // fin de aaa
            Position { line: 1, col: 3 }, // fin de bbb
        );
        assert_eq!(removed, "\nbbb");
        assert_eq!(buf.line(0), Some("aaa"));
        assert_eq!(buf.line(1), Some("ccc"));
        assert_eq!(buf.line_count(), 2);
    }

    // ── round-trip insert_text + delete_range ──

    #[test]
    fn insert_text_then_delete_range_reverts() {
        let mut buf = TextBuffer::from_text("hello world");
        let pos = Position { line: 0, col: 5 };
        let inserted = " querido";
        let end = buf.insert_text(pos, inserted);
        assert_eq!(buf.line(0), Some("hello querido world"));

        let removed = buf.delete_range(pos, end);
        assert_eq!(removed, inserted);
        assert_eq!(buf.line(0), Some("hello world"));
    }

    #[test]
    fn insert_text_then_delete_range_multibyte_reverts() {
        let mut buf = TextBuffer::from_text("xy");
        let pos = Position { line: 0, col: 1 };
        let inserted = "ñoño\ncódigo";
        let end = buf.insert_text(pos, inserted);
        // Esperado: "xñoño" en línea 0, "códigoy" en línea 1
        assert_eq!(buf.line(0), Some("xñoño"));
        assert_eq!(buf.line(1), Some("códigoy"));

        let removed = buf.delete_range(pos, end);
        assert_eq!(removed, inserted);
        assert_eq!(buf.line(0), Some("xy"));
        assert_eq!(buf.line_count(), 1);
    }
}
