//! Viewport: ventana virtual sobre el buffer de texto.
//!
//! Solo renderiza las líneas visibles — nunca itera el buffer completo.
//! El viewport sigue al cursor: si el cursor se mueve fuera del área
//! visible, el scroll se ajusta automáticamente.

use std::ops::Range;

use super::cursor::Position;

/// Viewport virtual que define qué porción del buffer es visible.
///
/// Tipo ligero (3 × usize = 24 bytes en 64-bit), `Copy`.
/// Se recalcula solo cuando cambia el scroll, el tamaño del terminal,
/// o la posición del cursor sale del rango visible.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Primera línea visible (0-indexed).
    pub scroll_offset: usize,
    /// Cantidad de líneas visibles en el área del editor.
    pub height: usize,
    /// Cantidad de columnas visibles (para scroll horizontal futuro).
    pub width: usize,
}

impl Viewport {
    /// Crea un viewport con valores por defecto.
    ///
    /// Tamaño 0×0 — se actualiza con `update_size` al primer render.
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            height: 0,
            width: 0,
        }
    }

    /// Ajusta el scroll para que el cursor siempre sea visible.
    ///
    /// Si el cursor está arriba del viewport, scrollea hacia arriba.
    /// Si está abajo, scrollea hacia abajo. Si ya está visible, no toca nada.
    pub fn ensure_cursor_visible(&mut self, cursor: &Position) {
        // Cursor arriba del viewport
        if cursor.line < self.scroll_offset {
            self.scroll_offset = cursor.line;
        }
        // Cursor abajo del viewport
        if self.height > 0 && cursor.line >= self.scroll_offset + self.height {
            self.scroll_offset = cursor.line - self.height + 1;
        }
    }

    /// Rango de líneas visibles (para iterar el buffer).
    ///
    /// El rango no excede el tamaño real del buffer — el caller
    /// debe pasarlo por `buffer.lines_range()` que ya clampea.
    #[expect(dead_code, reason = "se usará para renderizar solo líneas visibles")]
    pub fn visible_range(&self) -> Range<usize> {
        self.scroll_offset..self.scroll_offset + self.height
    }

    /// Actualiza el tamaño del viewport (se llama en resize o al cambiar layout).
    #[expect(
        dead_code,
        reason = "se usará cuando el editor responda a eventos de resize"
    )]
    pub fn update_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new()
    }
}
