//! Search: búsqueda local dentro de un buffer.
//!
//! Busca coincidencias de texto plano en el buffer actual.
//! Las coincidencias se almacenan como posiciones (línea, columna)
//! para navegación rápida sin re-escanear.

use super::buffer::TextBuffer;

/// Una coincidencia de búsqueda en el buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    /// Línea donde se encontró la coincidencia.
    pub line: usize,
    /// Columna de inicio (byte offset).
    pub start_col: usize,
    /// Columna de fin (byte offset, exclusivo).
    pub end_col: usize,
}

/// Estado de búsqueda local en el buffer.
///
/// Contiene el query, las coincidencias encontradas, y cuál es
/// la coincidencia activa para navegación (next/prev).
#[derive(Debug)]
pub struct BufferSearch {
    /// Texto a buscar.
    pub query: String,
    /// Coincidencias encontradas, ordenadas por posición.
    pub matches: Vec<SearchMatch>,
    /// Índice de la coincidencia activa (para highlight y navegación).
    pub current_match: Option<usize>,
    /// Si la búsqueda distingue mayúsculas/minúsculas.
    pub case_sensitive: bool,
}

impl BufferSearch {
    /// Crea una búsqueda nueva con el query dado.
    #[expect(dead_code, reason = "se usará cuando se implemente búsqueda en editor")]
    pub fn new(query: &str, case_sensitive: bool) -> Self {
        Self {
            query: query.to_owned(),
            matches: Vec::new(),
            current_match: None,
            case_sensitive,
        }
    }

    /// Ejecuta la búsqueda sobre el buffer y almacena las coincidencias.
    ///
    /// Limpia coincidencias previas. Si el query está vacío, no busca.
    /// La búsqueda es lineal — O(n) sobre el contenido del buffer.
    #[expect(dead_code, reason = "se usará cuando se implemente búsqueda en editor")]
    pub fn search(&mut self, buffer: &TextBuffer) {
        self.matches.clear();
        self.current_match = None;

        if self.query.is_empty() {
            return;
        }

        let query = if self.case_sensitive {
            self.query.as_str().to_owned()
        } else {
            self.query.to_lowercase()
        };

        for line_idx in 0..buffer.line_count() {
            let Some(line) = buffer.line(line_idx) else {
                continue;
            };

            let haystack = if self.case_sensitive {
                line.to_owned()
            } else {
                line.to_lowercase()
            };

            let mut offset = 0;
            while let Some(pos) = haystack[offset..].find(&query) {
                let start_col = offset + pos;
                let end_col = start_col + self.query.len();
                self.matches.push(SearchMatch {
                    line: line_idx,
                    start_col,
                    end_col,
                });
                offset = start_col + 1; // avanzar para encontrar coincidencias overlapping
            }
        }

        if !self.matches.is_empty() {
            self.current_match = Some(0);
        }
    }

    /// Avanza a la siguiente coincidencia (wrap around al final).
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente navegación de búsqueda"
    )]
    pub fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current_match = Some(match self.current_match {
            Some(idx) => (idx + 1) % self.matches.len(),
            None => 0,
        });
    }

    /// Retrocede a la coincidencia anterior (wrap around al inicio).
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente navegación de búsqueda"
    )]
    pub fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current_match = Some(match self.current_match {
            Some(0) | None => self.matches.len() - 1,
            Some(idx) => idx - 1,
        });
    }

    /// Limpia la búsqueda (query, coincidencias, todo).
    #[expect(dead_code, reason = "se usará cuando se implemente cerrar búsqueda")]
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
    }

    /// Cantidad de coincidencias encontradas.
    #[expect(dead_code, reason = "se usará para mostrar conteo en status bar")]
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }
}
