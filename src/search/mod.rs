//! Search: búsqueda global en workspace, replace, filtros avanzados.
//!
//! Provee `SearchState` como sub-estado del `AppState`, con campos para
//! query, opciones, resultados cacheados, y navegación. La búsqueda se
//! ejecuta on-demand (Enter) — NUNCA en render.
//!
//! El motor (`engine.rs`) recorre el workspace respetando ignore y globs.
//! Los resultados se almacenan en `SearchState` y el render los consulta
//! por referencia.

pub mod engine;

use std::path::Path;

use engine::{SearchMatch, SearchOptions, SearchResults};

// ─── SearchField ───────────────────────────────────────────────────────────────

/// Campo activo en el panel de búsqueda.
///
/// Determina dónde se insertan caracteres (Tab cicla entre campos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchField {
    /// Campo de query (texto a buscar).
    Query,
    /// Campo de replace (texto de reemplazo).
    Replace,
    /// Patrón de inclusión de archivos (glob).
    Include,
    /// Patrón de exclusión de archivos (glob).
    Exclude,
}

// ─── SearchState ───────────────────────────────────────────────────────────────

/// Estado completo del panel de búsqueda global.
///
/// Sub-estado de `AppState`. Mantiene opciones, resultados cacheados,
/// navegación y campos de input. El render solo lee — nunca muta.
#[derive(Debug)]
pub struct SearchState {
    /// Si el panel de búsqueda está visible.
    pub visible: bool,
    /// Opciones de búsqueda (query, flags, globs).
    pub options: SearchOptions,
    /// Resultados de la última búsqueda (None si no se ha buscado).
    pub results: Option<SearchResults>,
    /// Índice del match seleccionado en `results.matches`.
    pub selected_match: usize,
    /// Offset de scroll en la lista de resultados.
    pub scroll_offset: usize,
    /// Campo de input activo (Query, Replace, Include, Exclude).
    pub active_field: SearchField,
    /// Posición del cursor en el campo activo.
    pub cursor_pos: usize,
    /// Texto de reemplazo.
    pub replace_text: String,
    /// Si el campo de replace está visible.
    pub replace_visible: bool,
}

impl SearchState {
    /// Crea un estado inicial (panel cerrado, sin resultados).
    pub fn new() -> Self {
        Self {
            visible: false,
            options: SearchOptions::default(),
            results: None,
            selected_match: 0,
            scroll_offset: 0,
            active_field: SearchField::Query,
            cursor_pos: 0,
            replace_text: String::with_capacity(64),
            replace_visible: false,
        }
    }

    /// Abre el panel de búsqueda. Foco en Query.
    pub fn open(&mut self) {
        self.visible = true;
        self.active_field = SearchField::Query;
        self.cursor_pos = self.options.query.len();
    }

    /// Cierra el panel de búsqueda. No limpia resultados por si reabre.
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Ejecuta búsqueda con las opciones actuales.
    ///
    /// Se llama desde el reducer, NO desde render.
    /// Los resultados se cachean en `self.results`.
    pub fn execute_search(&mut self, root: &Path, max_results: usize) {
        match engine::search_workspace(root, &self.options, max_results) {
            Ok(results) => {
                self.results = Some(results);
                self.selected_match = 0;
                self.scroll_offset = 0;
            }
            Err(e) => {
                tracing::error!(error = %e, "error en búsqueda global");
                // Limpiar resultados en error
                self.results = None;
            }
        }
    }

    /// Toggle case sensitive.
    pub fn toggle_case_sensitive(&mut self) {
        self.options.case_sensitive = !self.options.case_sensitive;
    }

    /// Toggle whole word.
    pub fn toggle_whole_word(&mut self) {
        self.options.whole_word = !self.options.whole_word;
    }

    /// Toggle regex.
    pub fn toggle_regex(&mut self) {
        self.options.use_regex = !self.options.use_regex;
    }

    /// Toggle visibilidad del campo replace.
    pub fn toggle_replace(&mut self) {
        self.replace_visible = !self.replace_visible;
        if self.replace_visible && self.active_field == SearchField::Query {
            // Mantener foco en query — el usuario puede Tab a replace
        }
    }

    /// Avanzar al siguiente campo (Tab).
    ///
    /// Ciclo: Query → Replace (si visible) → Include → Exclude → Query
    pub fn next_field(&mut self) {
        self.active_field = match self.active_field {
            SearchField::Query => {
                if self.replace_visible {
                    SearchField::Replace
                } else {
                    SearchField::Include
                }
            }
            SearchField::Replace => SearchField::Include,
            SearchField::Include => SearchField::Exclude,
            SearchField::Exclude => SearchField::Query,
        };
        self.cursor_pos = self.active_field_text().len();
    }

    /// Retroceder al campo anterior (Shift+Tab).
    pub fn prev_field(&mut self) {
        self.active_field = match self.active_field {
            SearchField::Query => SearchField::Exclude,
            SearchField::Replace => SearchField::Query,
            SearchField::Include => {
                if self.replace_visible {
                    SearchField::Replace
                } else {
                    SearchField::Query
                }
            }
            SearchField::Exclude => SearchField::Include,
        };
        self.cursor_pos = self.active_field_text().len();
    }

    /// Insertar carácter en el campo activo.
    pub fn insert_char(&mut self, ch: char) {
        let cursor = self.cursor_pos;
        let text = self.active_field_text_mut();
        let pos = cursor.min(text.len());
        text.insert(pos, ch);
        self.cursor_pos = pos + ch.len_utf8();
    }

    /// Eliminar carácter antes del cursor en el campo activo.
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let cursor = self.cursor_pos;
            let text = self.active_field_text_mut();
            let prev_boundary = text[..cursor]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            text.drain(prev_boundary..cursor);
            self.cursor_pos = prev_boundary;
        }
    }

    /// Navegar al siguiente match en resultados.
    pub fn next_match(&mut self) {
        if let Some(ref results) = self.results
            && !results.matches.is_empty()
        {
            if self.selected_match + 1 < results.matches.len() {
                self.selected_match += 1;
            } else {
                self.selected_match = 0; // Wrap
            }
            self.ensure_match_visible();
        }
    }

    /// Navegar al match anterior en resultados.
    pub fn prev_match(&mut self) {
        if let Some(ref results) = self.results
            && !results.matches.is_empty()
        {
            if self.selected_match > 0 {
                self.selected_match -= 1;
            } else {
                self.selected_match = results.matches.len() - 1; // Wrap
            }
            self.ensure_match_visible();
        }
    }

    /// Reemplazar el match seleccionado en disco.
    ///
    /// Después del reemplazo, re-ejecuta la búsqueda para actualizar resultados.
    pub fn replace_current(&mut self, root: &Path) -> anyhow::Result<()> {
        let m = self
            .selected_match_data()
            .ok_or_else(|| anyhow::anyhow!("no hay match seleccionado"))?;

        let file_path = root.join(&m.path);
        engine::replace_in_file(
            &file_path,
            m.line_number,
            m.match_start,
            m.match_end,
            &self.replace_text,
        )?;

        // Re-ejecutar búsqueda para actualizar resultados
        let max = self
            .results
            .as_ref()
            .map(|r| r.matches.capacity().max(1000))
            .unwrap_or(1000);
        self.execute_search(root, max);

        Ok(())
    }

    /// Reemplazar todos los matches del archivo del match seleccionado.
    ///
    /// Recorre todos los matches del mismo archivo en orden inverso
    /// (para que los byte offsets no se invaliden) y reemplaza cada uno.
    pub fn replace_all_in_file(&mut self, root: &Path) -> anyhow::Result<()> {
        let current_path = self
            .selected_match_data()
            .map(|m| m.path.clone()) // CLONE: necesario — necesitamos el path mientras mutamos results
            .ok_or_else(|| anyhow::anyhow!("no hay match seleccionado"))?;

        // Recolectar matches del mismo archivo en orden inverso (línea desc, offset desc)
        let matches_in_file: Vec<(usize, usize, usize)> = self
            .results
            .as_ref()
            .map(|r| {
                let mut v: Vec<(usize, usize, usize)> = r
                    .matches
                    .iter()
                    .filter(|m| m.path == current_path)
                    .map(|m| (m.line_number, m.match_start, m.match_end))
                    .collect();
                // Orden inverso: primero las líneas más altas y offsets más altos
                v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                v
            })
            .unwrap_or_default();

        let file_path = root.join(&current_path);
        for (line_num, start, end) in &matches_in_file {
            engine::replace_in_file(&file_path, *line_num, *start, *end, &self.replace_text)?;
        }

        // Re-ejecutar búsqueda para actualizar resultados
        let max = self
            .results
            .as_ref()
            .map(|r| r.matches.capacity().max(1000))
            .unwrap_or(1000);
        self.execute_search(root, max);

        Ok(())
    }

    /// Texto del campo activo (referencia inmutable).
    pub fn active_field_text(&self) -> &str {
        match self.active_field {
            SearchField::Query => &self.options.query,
            SearchField::Replace => &self.replace_text,
            SearchField::Include => &self.options.include_pattern,
            SearchField::Exclude => &self.options.exclude_pattern,
        }
    }

    /// Texto del campo activo (referencia mutable).
    pub fn active_field_text_mut(&mut self) -> &mut String {
        match self.active_field {
            SearchField::Query => &mut self.options.query,
            SearchField::Replace => &mut self.replace_text,
            SearchField::Include => &mut self.options.include_pattern,
            SearchField::Exclude => &mut self.options.exclude_pattern,
        }
    }

    /// Retorna el match seleccionado (si existe).
    pub fn selected_match_data(&self) -> Option<&SearchMatch> {
        self.results
            .as_ref()
            .and_then(|r| r.matches.get(self.selected_match))
    }

    /// Ajusta scroll para que el match seleccionado sea visible.
    ///
    /// `MAX_VISIBLE_RESULTS` es un estimado — el render real puede mostrar
    /// más o menos según el alto del panel.
    fn ensure_match_visible(&mut self) {
        const MAX_VISIBLE_RESULTS: usize = 20;
        if self.selected_match < self.scroll_offset {
            self.scroll_offset = self.selected_match;
        } else if self.selected_match >= self.scroll_offset + MAX_VISIBLE_RESULTS {
            self.scroll_offset = self.selected_match - MAX_VISIBLE_RESULTS + 1;
        }
    }
}
