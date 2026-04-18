//! Selection: rango de texto seleccionado entre anchor y head.
//!
//! Una selección define un rango continuo de texto entre dos posiciones:
//! `anchor` (donde empezó la selección) y `head` (donde está el cursor).
//! El head puede estar antes o después del anchor — la dirección importa
//! para extender la selección con Shift+flechas.

use super::buffer::TextBuffer;
use super::cursor::Position;

/// Rango de texto seleccionado entre anchor y head.
///
/// Tipo `Copy` — 32 bytes en 64-bit (2 × Position). Se pasa por valor.
/// `anchor` es donde empezó la selección, `head` es donde está el cursor.
/// Si anchor == head, la selección está vacía.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// Donde empezó la selección.
    pub anchor: Position,
    /// Donde está el cursor (puede ser antes o después del anchor).
    pub head: Position,
}

impl Selection {
    /// Crea una selección entre dos posiciones.
    pub fn new(anchor: Position, head: Position) -> Self {
        Self { anchor, head }
    }

    /// Selección vacía — anchor == head.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Posición menor (inicio real de la selección en el buffer).
    pub fn start(&self) -> Position {
        self.anchor.min(self.head)
    }

    /// Posición mayor (fin real de la selección en el buffer).
    pub fn end(&self) -> Position {
        self.anchor.max(self.head)
    }

    /// Extrae el texto seleccionado del buffer.
    ///
    /// Itera línea por línea entre start y end, extrayendo los rangos
    /// relevantes. Retorna `String` porque el texto puede abarcar
    /// múltiples líneas y necesita ownership.
    pub fn selected_text(&self, buffer: &TextBuffer) -> String {
        if self.is_empty() {
            return String::new();
        }

        let start = self.start();
        let end = self.end();

        if start.line == end.line {
            // Selección en una sola línea — caso más común
            return buffer
                .line(start.line)
                .map(|line| {
                    let s = start.col.min(line.len());
                    let e = end.col.min(line.len());
                    line[s..e].to_owned()
                })
                .unwrap_or_default();
        }

        // Selección multi-línea
        let mut result = String::new();

        for line_idx in start.line..=end.line {
            let Some(line) = buffer.line(line_idx) else {
                continue;
            };

            if line_idx == start.line {
                // Primera línea: desde start.col hasta el final
                let s = start.col.min(line.len());
                result.push_str(&line[s..]);
                result.push('\n');
            } else if line_idx == end.line {
                // Última línea: desde el inicio hasta end.col
                let e = end.col.min(line.len());
                result.push_str(&line[..e]);
            } else {
                // Líneas intermedias: completas
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }

    /// Verifica si una posición está dentro de la selección.
    #[expect(dead_code, reason = "se usará para hit-testing de selección con mouse")]
    pub fn contains(&self, pos: Position) -> bool {
        if self.is_empty() {
            return false;
        }
        let start = self.start();
        let end = self.end();
        pos >= start && pos < end
    }
}
