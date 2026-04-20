//! Brackets: detección de pares de brackets matching.
//!
//! Cuando el cursor está sobre un bracket (`{`, `}`, `(`, `)`, `[`, `]`),
//! encuentra el par correspondiente. Se pre-computa FUERA del render loop
//! y se pasa como dato al render.
//!
//! Límite de búsqueda: máximo 10_000 caracteres en cada dirección
//! para no colgar en archivos mal formateados o sin par.

use super::buffer::TextBuffer;
use super::cursor::Position;

/// Brackets de apertura reconocidos.
const OPEN_BRACKETS: &[char] = &['(', '{', '['];
/// Brackets de cierre reconocidos.
const CLOSE_BRACKETS: &[char] = &[')', '}', ']'];

/// Límite máximo de caracteres a recorrer buscando el par.
/// Evita bloquear en archivos enormes con brackets desbalanceados.
const SEARCH_LIMIT: usize = 10_000;

/// Obtiene el carácter en una posición del buffer.
///
/// Retorna `None` si la posición está fuera de rango.
fn char_at(buffer: &TextBuffer, pos: Position) -> Option<char> {
    let line = buffer.line(pos.line)?;
    line.chars().nth(pos.col)
}

/// Busca el bracket de cierre correspondiente hacia adelante.
///
/// Empieza en la posición siguiente a `pos` y avanza carácter por
/// carácter, manteniendo un counter de nesting. Incrementa con `open`,
/// decrementa con `close`. Cuando counter == 0, retorna esa posición.
///
/// Se detiene después de `SEARCH_LIMIT` caracteres sin encontrar par.
fn find_forward(buffer: &TextBuffer, pos: Position, open: char, close: char) -> Option<Position> {
    let mut depth: usize = 1;
    let mut chars_scanned: usize = 0;
    let total_lines = buffer.line_count();

    // Empezar en la misma línea, después de pos.col
    let mut line_idx = pos.line;
    let mut start_col = pos.col + 1;

    while line_idx < total_lines {
        if let Some(line) = buffer.line(line_idx) {
            for (col, ch) in line.chars().enumerate() {
                if line_idx == pos.line && col < start_col {
                    continue;
                }
                chars_scanned += 1;
                if chars_scanned > SEARCH_LIMIT {
                    return None;
                }
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Position {
                            line: line_idx,
                            col,
                        });
                    }
                }
            }
        }
        line_idx += 1;
        start_col = 0;
    }

    None
}

/// Busca el bracket de apertura correspondiente hacia atrás.
///
/// Empieza en la posición anterior a `pos` y retrocede carácter por
/// carácter, manteniendo un counter de nesting. Incrementa con `close`,
/// decrementa con `open`. Cuando counter == 0, retorna esa posición.
///
/// Se detiene después de `SEARCH_LIMIT` caracteres sin encontrar par.
fn find_backward(buffer: &TextBuffer, pos: Position, open: char, close: char) -> Option<Position> {
    let mut depth: usize = 1;
    let mut chars_scanned: usize = 0;

    // Empezar en la misma línea, antes de pos.col
    let mut line_idx = pos.line;
    let mut first_pass = true;

    loop {
        if let Some(line) = buffer.line(line_idx) {
            // Recopilar chars con sus columnas para iterar en reversa.
            // No usamos .rev() directamente en char_indices porque
            // necesitamos controlar el punto de inicio.
            let chars_with_cols: Vec<(usize, char)> = line.char_indices().collect();

            for &(col, ch) in chars_with_cols.iter().rev() {
                // En la primera línea, solo procesar antes de pos.col
                if first_pass && col >= pos.col {
                    continue;
                }
                chars_scanned += 1;
                if chars_scanned > SEARCH_LIMIT {
                    return None;
                }
                if ch == close {
                    depth += 1;
                } else if ch == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Position {
                            line: line_idx,
                            col,
                        });
                    }
                }
            }
        }
        first_pass = false;
        if line_idx == 0 {
            break;
        }
        line_idx -= 1;
    }

    None
}

/// Dado una posición en el buffer donde hay un bracket,
/// encontrar su par matching.
///
/// Retorna `None` si no hay par, o si la posición no tiene un bracket.
/// La búsqueda se limita a `SEARCH_LIMIT` caracteres en cada dirección.
pub fn find_matching_bracket(buffer: &TextBuffer, pos: Position) -> Option<Position> {
    let ch = char_at(buffer, pos)?;

    if let Some(idx) = OPEN_BRACKETS.iter().position(|&c| c == ch) {
        // Buscar el cierre correspondiente hacia adelante
        find_forward(buffer, pos, OPEN_BRACKETS[idx], CLOSE_BRACKETS[idx])
    } else if let Some(idx) = CLOSE_BRACKETS.iter().position(|&c| c == ch) {
        // Buscar la apertura correspondiente hacia atrás
        find_backward(buffer, pos, OPEN_BRACKETS[idx], CLOSE_BRACKETS[idx])
    } else {
        None
    }
}

/// Pre-computa el bracket match para la posición actual del cursor.
///
/// Retorna `Some((cursor_pos, match_pos))` si el cursor está sobre
/// un bracket y se encontró su par. Retorna `None` si no hay bracket
/// en la posición del cursor o no se encontró par.
///
/// Se llama UNA VEZ antes del render, no en el render loop.
pub fn compute_bracket_match(
    buffer: &TextBuffer,
    cursor_pos: Position,
) -> Option<(Position, Position)> {
    find_matching_bracket(buffer, cursor_pos).map(|match_pos| (cursor_pos, match_pos))
}

/// Verifica si un carácter es un bracket (abierto o cerrado).
pub fn is_bracket(ch: char) -> bool {
    OPEN_BRACKETS.contains(&ch) || CLOSE_BRACKETS.contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: crea un TextBuffer desde texto.
    fn buf(text: &str) -> TextBuffer {
        TextBuffer::from_text(text)
    }

    #[test]
    fn test_simple_parens() {
        let buffer = buf("(hello)");
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 0 });
        assert_eq!(result, Some(Position { line: 0, col: 6 }));

        let result = find_matching_bracket(&buffer, Position { line: 0, col: 6 });
        assert_eq!(result, Some(Position { line: 0, col: 0 }));
    }

    #[test]
    fn test_nested_braces() {
        let buffer = buf("{ { } }");
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 0 });
        assert_eq!(result, Some(Position { line: 0, col: 6 }));

        let result = find_matching_bracket(&buffer, Position { line: 0, col: 2 });
        assert_eq!(result, Some(Position { line: 0, col: 4 }));
    }

    #[test]
    fn test_multiline_brackets() {
        let buffer = buf("fn main() {\n    println!(\"hi\");\n}");
        // `{` en línea 0, col 10
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 10 });
        assert_eq!(result, Some(Position { line: 2, col: 0 }));

        // `}` en línea 2, col 0
        let result = find_matching_bracket(&buffer, Position { line: 2, col: 0 });
        assert_eq!(result, Some(Position { line: 0, col: 10 }));
    }

    #[test]
    fn test_no_match() {
        let buffer = buf("(unclosed");
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 0 });
        assert_eq!(result, None);
    }

    #[test]
    fn test_not_a_bracket() {
        let buffer = buf("hello");
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 0 });
        assert_eq!(result, None);
    }

    #[test]
    fn test_position_out_of_range() {
        let buffer = buf("hello");
        let result = find_matching_bracket(&buffer, Position { line: 0, col: 99 });
        assert_eq!(result, None);
    }

    #[test]
    fn test_compute_bracket_match() {
        let buffer = buf("(x)");
        let result = compute_bracket_match(&buffer, Position { line: 0, col: 0 });
        assert_eq!(
            result,
            Some((Position { line: 0, col: 0 }, Position { line: 0, col: 2 },))
        );
    }

    #[test]
    fn test_is_bracket() {
        assert!(is_bracket('('));
        assert!(is_bracket(')'));
        assert!(is_bracket('{'));
        assert!(is_bracket('}'));
        assert!(is_bracket('['));
        assert!(is_bracket(']'));
        assert!(!is_bracket('a'));
        assert!(!is_bracket(' '));
    }
}
