//! Search: búsqueda global en workspace, replace, filtros avanzados.
//!
//! Provee `SearchState` como sub-estado del `AppState`, con campos para
//! query, opciones, resultados cacheados, y navegación. La búsqueda se
//! ejecuta on-demand (Enter) — NUNCA en render.
//!
//! El motor (`engine.rs`) recorre el workspace respetando ignore y globs.
//! Los resultados se almacenan en `SearchState` y el render los consulta
//! por referencia.
//!
//! Resultados agrupados por archivo (estilo VS Code):
//! - `FileGroup`: matches agrupados por archivo con path, nombre, count
//! - `FlatSearchItem`: lista aplanada para render + navegación con teclado/mouse
//! - `collapsed_files`: file groups colapsados (fold/unfold)

pub mod engine;

use std::collections::HashSet;
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

// ─── FileGroup ─────────────────────────────────────────────────────────────────

/// Matches agrupados por archivo — un grupo por cada archivo con resultados.
///
/// Almacena el path, nombre del archivo, conteo de matches, y los índices
/// en `SearchResults.matches` que pertenecen a este archivo.
#[derive(Debug)]
pub struct FileGroup {
    /// Path relativo del archivo al workspace root.
    #[expect(dead_code, reason = "se usará para acciones de replace por archivo")]
    pub path: String,
    /// Solo el nombre del archivo (último componente del path).
    pub filename: String,
    /// Directorio del archivo (path sin el filename). Vacío si está en la raíz.
    pub dir: String,
    /// Cantidad de matches en este archivo.
    pub match_count: usize,
    /// Índices en `SearchResults.matches` que pertenecen a este grupo.
    pub matches: Vec<usize>,
}

// ─── FlatSearchItem ────────────────────────────────────────────────────────────

/// Item en la lista aplanada de resultados — para render y navegación.
///
/// La lista aplanada intercala headers de archivo con líneas de match,
/// respetando el estado de fold (collapsed_files). Se reconstruye
/// cuando cambia el fold o los resultados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatSearchItem {
    /// Header de un grupo de archivo (clickeable para fold/unfold).
    FileHeader {
        /// Índice en `SearchState.file_groups`.
        group_index: usize,
    },
    /// Línea de match dentro de un grupo de archivo.
    MatchLine {
        /// Índice en `SearchState.file_groups`.
        group_index: usize,
        /// Índice en `SearchResults.matches`.
        match_index: usize,
    },
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
    /// Índice del match seleccionado en `results.matches` (legacy, kept for navigation).
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
    /// Resultados agrupados por archivo (se construyen después de ejecutar búsqueda).
    pub file_groups: Vec<FileGroup>,
    /// Índices de file groups colapsados (fold).
    pub collapsed_files: HashSet<usize>,
    /// Lista aplanada para render + navegación por teclado/mouse.
    pub flat_items: Vec<FlatSearchItem>,
    /// Índice seleccionado en la lista aplanada.
    pub selected_flat_index: usize,
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
            file_groups: Vec::new(),
            collapsed_files: HashSet::new(),
            flat_items: Vec::new(),
            selected_flat_index: 0,
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
    /// Después de obtener resultados, construye los file groups y flat items.
    pub fn execute_search(&mut self, root: &Path, max_results: usize) {
        match engine::search_workspace(root, &self.options, max_results) {
            Ok(results) => {
                self.results = Some(results);
                self.selected_match = 0;
                self.scroll_offset = 0;
                self.collapsed_files.clear();
                self.build_file_groups();
                self.rebuild_flat_items();
                self.selected_flat_index = 0;
            }
            Err(e) => {
                tracing::error!(error = %e, "error en búsqueda global");
                // Limpiar resultados en error
                self.results = None;
                self.file_groups.clear();
                self.flat_items.clear();
                self.selected_flat_index = 0;
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

    /// Navegar al siguiente match en resultados (legacy, usa flat_next para el nuevo UI).
    #[expect(dead_code, reason = "mantenido para compatibilidad — el nuevo UI usa flat_next/flat_prev")]
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

    /// Navegar al match anterior en resultados (legacy, usa flat_prev para el nuevo UI).
    #[expect(dead_code, reason = "mantenido para compatibilidad — el nuevo UI usa flat_next/flat_prev")]
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

    // ─── Flat navigation (nuevo sistema estilo VS Code) ──────────────────────

    /// Construye los file groups a partir de los resultados de búsqueda.
    ///
    /// Agrupa matches por archivo, extrayendo filename y directorio.
    /// Se llama después de `execute_search()`.
    pub fn build_file_groups(&mut self) {
        self.file_groups.clear();

        let Some(ref results) = self.results else {
            return;
        };

        if results.matches.is_empty() {
            return;
        }

        let mut current_path: Option<&std::path::Path> = None;

        for (i, m) in results.matches.iter().enumerate() {
            let is_new_file = current_path != Some(m.path.as_path());

            if is_new_file {
                current_path = Some(&m.path);

                let path_str = m.path.to_string_lossy();
                let filename = m
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dir = m
                    .path
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();

                self.file_groups.push(FileGroup {
                    path: path_str.into_owned(),
                    filename,
                    dir,
                    match_count: 0,
                    matches: Vec::new(),
                });
            }

            if let Some(group) = self.file_groups.last_mut() {
                group.match_count += 1;
                group.matches.push(i);
            }
        }
    }

    /// Reconstruye la lista aplanada respetando collapsed_files.
    ///
    /// Para cada file group, emite un FileHeader. Si el grupo NO está
    /// colapsado, emite también cada MatchLine del grupo.
    pub fn rebuild_flat_items(&mut self) {
        self.flat_items.clear();

        // Pre-calcular capacidad: headers + matches de grupos expandidos
        let capacity = self.file_groups.len()
            + self
                .file_groups
                .iter()
                .enumerate()
                .filter(|(i, _)| !self.collapsed_files.contains(i))
                .map(|(_, g)| g.matches.len())
                .sum::<usize>();
        self.flat_items.reserve(capacity);

        for (group_idx, group) in self.file_groups.iter().enumerate() {
            self.flat_items.push(FlatSearchItem::FileHeader {
                group_index: group_idx,
            });

            if !self.collapsed_files.contains(&group_idx) {
                for &match_idx in &group.matches {
                    self.flat_items.push(FlatSearchItem::MatchLine {
                        group_index: group_idx,
                        match_index: match_idx,
                    });
                }
            }
        }
    }

    /// Toggle fold de un file group (colapsar/expandir).
    pub fn toggle_fold(&mut self, group_index: usize) {
        if self.collapsed_files.contains(&group_index) {
            self.collapsed_files.remove(&group_index);
        } else {
            self.collapsed_files.insert(group_index);
        }
        self.rebuild_flat_items();
        // Clampear selección si quedó fuera de rango
        if self.selected_flat_index >= self.flat_items.len() {
            self.selected_flat_index = self.flat_items.len().saturating_sub(1);
        }
    }

    /// Retorna el item seleccionado en la lista aplanada (si existe).
    pub fn selected_item(&self) -> Option<&FlatSearchItem> {
        self.flat_items.get(self.selected_flat_index)
    }

    /// Verifica si un file group está colapsado.
    pub fn is_collapsed(&self, group_index: usize) -> bool {
        self.collapsed_files.contains(&group_index)
    }

    /// Navegar al siguiente item en la lista aplanada (Down).
    pub fn flat_next(&mut self) {
        if !self.flat_items.is_empty() && self.selected_flat_index + 1 < self.flat_items.len() {
            self.selected_flat_index += 1;
            self.ensure_flat_visible();
        }
    }

    /// Navegar al item anterior en la lista aplanada (Up).
    pub fn flat_prev(&mut self) {
        if self.selected_flat_index > 0 {
            self.selected_flat_index -= 1;
            self.ensure_flat_visible();
        }
    }

    /// Ejecutar acción de Enter en el item seleccionado.
    ///
    /// Retorna `Some(match_index)` si se debe abrir un match en el editor.
    /// Retorna `None` si se hizo toggle fold de un file header.
    pub fn flat_enter(&mut self) -> Option<usize> {
        let item = self.flat_items.get(self.selected_flat_index).copied();
        match item {
            Some(FlatSearchItem::FileHeader { group_index }) => {
                self.toggle_fold(group_index);
                None
            }
            Some(FlatSearchItem::MatchLine { match_index, .. }) => {
                self.selected_match = match_index;
                Some(match_index)
            }
            None => None,
        }
    }

    /// Colapsar el file header seleccionado (Left en header expandido).
    ///
    /// Retorna `true` si se ejecutó un fold.
    #[expect(dead_code, reason = "disponible para keybindings futuros — Left/Right usan SearchToggleFold")]
    pub fn flat_collapse(&mut self) -> bool {
        if let Some(&FlatSearchItem::FileHeader { group_index }) =
            self.flat_items.get(self.selected_flat_index)
            && !self.is_collapsed(group_index)
        {
            self.toggle_fold(group_index);
            return true;
        }
        false
    }

    /// Expandir el file header seleccionado (Right en header colapsado).
    ///
    /// Retorna `true` si se ejecutó un unfold.
    #[expect(dead_code, reason = "disponible para keybindings futuros — Left/Right usan SearchToggleFold")]
    pub fn flat_expand(&mut self) -> bool {
        if let Some(&FlatSearchItem::FileHeader { group_index }) =
            self.flat_items.get(self.selected_flat_index)
            && self.is_collapsed(group_index)
        {
            self.toggle_fold(group_index);
            return true;
        }
        false
    }

    /// Ajusta scroll para que la selección flat sea visible.
    fn ensure_flat_visible(&mut self) {
        const MAX_VISIBLE: usize = 20;
        if self.selected_flat_index < self.scroll_offset {
            self.scroll_offset = self.selected_flat_index;
        } else if self.selected_flat_index >= self.scroll_offset + MAX_VISIBLE {
            self.scroll_offset = self.selected_flat_index - MAX_VISIBLE + 1;
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

    /// Ajusta scroll para que el match seleccionado sea visible (legacy).
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
