//! Editor: buffer model, cursor, viewport, undo/redo, multicursor, selección.
//!
//! Integra todos los sub-módulos del editor en un solo `EditorState`.
//! El `EditorState` es el punto de entrada para todas las operaciones
//! de edición — coordina buffer, multicursor, viewport y undo stack.

pub mod brackets;
pub mod buffer;
pub mod cursor;
pub mod highlighting;
pub mod image;
pub mod indent;
pub mod multicursor;
pub mod search;
pub mod selection;
pub mod tabs;
pub mod ts_highlight;
pub mod undo;
pub mod unicode;
pub mod viewport;

use std::path::{Path, PathBuf};

use anyhow::Result;

use buffer::TextBuffer;
use cursor::Position;
use highlighting::{HighlightCache, HighlightEngine};
use ts_highlight::TsHighlightEngine;
use multicursor::MultiCursorState;
use selection::Selection;
use undo::{EditOperation, UndoStack};
use viewport::Viewport;

use crate::core::Direction;

// ─── DiffViewContent ───────────────────────────────────────────────────────────

/// Contenido de una tab virtual de diff/file (read-only, no editable).
///
/// Cuando un `EditorState` tiene `diff_view = Some(...)`, esa tab se renderiza
/// como vista de diff/archivo en lugar de buffer editable. Las operaciones
/// de edición se ignoran sobre estas tabs — solo permiten scroll y cierre.
#[derive(Debug, Clone)]
pub struct DiffViewContent {
    /// Contenido completo (diff coloreado o texto del archivo).
    pub content: String,
    /// Path del archivo fuente del diff.
    pub file_path: Option<PathBuf>,
    /// `true` = mostrando archivo completo, `false` = mostrando diff real.
    pub is_file_content: bool,
    /// Offset de scroll actual (en líneas).
    pub scroll_offset: usize,
}

/// Estado completo del editor.
///
/// Contiene el buffer de texto, multicursor, viewport, undo stack y búsqueda.
/// Todas las operaciones de edición pasan por acá para mantener
/// la coordinación entre sub-sistemas (ej: insertar char -> registrar undo
/// -> ajustar cursores -> ajustar viewport).
#[derive(Debug)]
pub struct EditorState {
    /// Buffer de texto editable.
    pub buffer: TextBuffer,
    /// Sistema de multicursor (reemplaza al cursor único).
    pub cursors: MultiCursorState,
    /// Viewport virtual (qué porción del buffer es visible).
    pub viewport: Viewport,
    /// Historial de undo/redo.
    pub undo_stack: UndoStack,
    /// Búsqueda local activa (None si no hay búsqueda).
    pub search: Option<search::BufferSearch>,
    /// Cache de syntax highlighting para este buffer.
    pub highlight_cache: HighlightCache,
    /// Si el highlight del viewport fue diferido al próximo frame.
    ///
    /// Se activa al abrir un archivo nuevo para no bloquear el frame
    /// del open con trabajo pesado. El siguiente frame lo procesa normal.
    pub highlight_deferred: bool,
    /// Si esta tab es una vista de diff/file del git panel (no editable).
    ///
    /// `None` = tab normal de archivo (buffer editable).
    /// `Some(...)` = tab virtual de diff con contenido pre-computado y read-only.
    pub diff_view: Option<DiffViewContent>,
    /// Si esta tab es una vista de imagen (no editable).
    ///
    /// `None` = tab normal o de diff.
    /// `Some(...)` = tab de imagen con protocolo pre-codificado.
    /// Durante la fase de decodificación async, este campo es `None` y
    /// el reducer lo poblará con el resultado del `Effect::DecodeImage`.
    pub image_view: Option<image::ImageViewContent>,
}

impl EditorState {
    /// Crea un editor vacío (buffer vacío, cursor en 0,0).
    pub fn new() -> Self {
        Self {
            buffer: TextBuffer::new(),
            cursors: MultiCursorState::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
            highlight_cache: HighlightCache::new(),
            highlight_deferred: false,
            diff_view: None,
            image_view: None,
        }
    }

    /// Abre un archivo y crea un editor con su contenido.
    ///
    /// Si se pasa `engine`, detecta la syntax del archivo y prepara
    /// el cache de highlighting. Si `engine` es `None`, el cache queda
    /// vacío (sin syntax — el render usará color uniforme).
    pub fn open_file(path: &Path) -> Result<Self> {
        let buffer = TextBuffer::from_file(path)?;
        Ok(Self {
            buffer,
            cursors: MultiCursorState::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
            highlight_cache: HighlightCache::new(),
            // Diferir highlight al siguiente frame — el archivo acaba de abrirse.
            // Así el frame del open es instantáneo y el highlight corre después.
            highlight_deferred: true,
            diff_view: None,
            image_view: None,
        })
    }

    /// Inicializa el cache de highlighting para el archivo actual.
    ///
    /// Intenta tree-sitter primero (6 lenguajes soportados). Si no hay
    /// grammar tree-sitter, cae a syntect como fallback (~50 lenguajes).
    /// Se llama después de `open_file` cuando el `HighlightEngine`
    /// está disponible (vive en `AppState`).
    pub fn init_highlighting(&mut self, engine: &HighlightEngine) {
        // Intentar tree-sitter primero
        if let Some(path) = self.buffer.file_path() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if let Some(ts_config) = ts_highlight::config_for_extension(ext) {
                let ts_engine = TsHighlightEngine::new(ts_config);
                self.highlight_cache.set_ts_engine(Some(ts_engine));
                tracing::debug!(ext, "tree-sitter grammar cargado");
                return; // No necesitamos syntect
            }
        }
        // Fallback: syntect (camino existente)
        if let Some(path) = self.buffer.file_path()
            && let Some(syntax) = engine.detect_syntax(path)
        {
            self.highlight_cache = HighlightCache::with_syntax(syntax.name.as_str());
        }
    }

    /// Re-destaca inmediatamente la línea del cursor primario.
    ///
    /// Elimina el parpadeo blanco de 80ms al tipear: la línea editada
    /// se re-colorea en el mismo frame sin esperar al debounce.
    /// `ensure_viewport_highlighted` sigue manejando el re-process
    /// contextual del viewport completo.
    pub fn rehighlight_cursor_line(&mut self, engine: &HighlightEngine) {
        let line = self.cursors.primary().position.line;
        self.highlight_cache
            .highlight_single_line(line, &self.buffer, engine);
    }

    /// Inserta un carácter en la posición de todos los cursores.
    ///
    /// Itera en ORDEN INVERSO por posición para que los offsets no se
    /// invaliden al insertar en posiciones anteriores.
    pub fn insert_char(&mut self, ch: char) {
        // Ordenar cursores por posición antes de iterar en reversa
        self.cursors.sort_by_position();

        // Iterar en orden inverso para preservar offsets
        for i in (0..self.cursors.cursors.len()).rev() {
            let pos = self.cursors.cursors[i].position;

            // Si el cursor tiene selección, borrar el texto seleccionado primero
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
                // Actualizar pos después del delete
                let new_pos = self.cursors.cursors[i].position;
                self.buffer.insert_char(new_pos, ch);
                self.undo_stack
                    .push(EditOperation::InsertChar { pos: new_pos, ch });
                // Avanzar por el largo UTF-8 del char insertado, NO `+= 1`.
                self.cursors.cursors[i].position.col += ch.len_utf8();
                self.cursors.cursors[i].sync_desired_col();
                continue;
            }

            self.buffer.insert_char(pos, ch);
            self.undo_stack.push(EditOperation::InsertChar { pos, ch });
            // Avanzar por el largo UTF-8 del char insertado, NO `+= 1`.
            self.cursors.cursors[i].position.col += ch.len_utf8();
            self.cursors.cursors[i].sync_desired_col();
            self.cursors.cursors[i].clear_selection();
        }
        // Invalidar solo desde la línea editada en adelante
        let edited_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Elimina el carácter antes de todos los cursores (backspace).
    ///
    /// Itera en ORDEN INVERSO para preservar offsets.
    pub fn delete_char(&mut self) {
        self.cursors.sort_by_position();

        for i in (0..self.cursors.cursors.len()).rev() {
            // Si hay selección, borrar el texto seleccionado
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
                continue;
            }

            let pos = self.cursors.cursors[i].position;

            if pos.col > 0 {
                // Backspace char-aware: borrar desde el char boundary anterior
                // hasta pos.col. delete_range maneja el caso multi-byte correctamente.
                let line = self.buffer.line(pos.line).unwrap_or("");
                let prev = unicode::prev_char_boundary(line, pos.col);
                let del_pos = Position {
                    line: pos.line,
                    col: prev,
                };
                let removed = self.buffer.delete_range(del_pos, pos);
                if let Some(ch) = removed.chars().next() {
                    self.undo_stack
                        .push(EditOperation::DeleteChar { pos: del_pos, ch });
                    self.cursors.cursors[i].position.col = prev;
                    self.cursors.cursors[i].sync_desired_col();
                }
            } else if pos.line > 0 {
                let prev_line_len = self.buffer.line_len(pos.line - 1);
                let deleted = self.buffer.delete_char(pos);
                if deleted.is_some() {
                    self.undo_stack.push(EditOperation::DeleteNewline {
                        pos: Position {
                            line: pos.line - 1,
                            col: 0,
                        },
                        col: prev_line_len,
                    });
                    self.cursors.cursors[i].position = Position {
                        line: pos.line - 1,
                        col: prev_line_len,
                    };
                    self.cursors.cursors[i].sync_desired_col();
                }
            }
            self.cursors.cursors[i].clear_selection();
        }
        // Invalidar desde la línea más temprana editada
        let edited_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Inserta un salto de línea en la posición de todos los cursores.
    ///
    /// Itera en ORDEN INVERSO para preservar offsets.
    pub fn insert_newline(&mut self) {
        self.cursors.sort_by_position();

        for i in (0..self.cursors.cursors.len()).rev() {
            // Si hay selección, borrar primero
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                self.delete_selection_at(i);
            }

            let pos = self.cursors.cursors[i].position;
            self.buffer.insert_newline(pos);
            self.undo_stack.push(EditOperation::InsertNewline { pos });
            self.cursors.cursors[i].position.line += 1;
            self.cursors.cursors[i].position.col = 0;
            self.cursors.cursors[i].sync_desired_col();
            self.cursors.cursors[i].clear_selection();
        }
        // Newline cambia estructura de líneas — invalidar desde la línea editada
        let edited_line = self.cursors.primary().position.line.saturating_sub(1);
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Borra el texto de una selección en un cursor específico atómicamente.
    ///
    /// Usa `buffer.delete_range` (operación O(1) por línea) en lugar del antiguo
    /// loop char-por-char que: (a) corrompía multi-byte UTF-8 al iterar bytes
    /// como chars, (b) generaba N entradas de undo en vez de 1.
    /// Genera UNA entrada `EditOperation::DeleteRange` en el undo stack.
    /// Posiciona el cursor al inicio de la selección.
    fn delete_selection_at(&mut self, cursor_idx: usize) {
        let Some(sel) = self.cursors.cursors[cursor_idx].selection else {
            return;
        };
        if sel.is_empty() {
            return;
        }

        let start = sel.start();
        let end = sel.end();

        // CLONE: delete_range retorna String owned — necesario para undo.
        let deleted = self.buffer.delete_range(start, end);
        self.undo_stack.push(EditOperation::DeleteRange {
            start,
            end,
            text: deleted,
        });

        self.cursors.cursors[cursor_idx].position = start;
        self.cursors.cursors[cursor_idx].sync_desired_col();
        self.cursors.cursors[cursor_idx].clear_selection();
    }

    /// Mueve todos los cursores en la dirección indicada y ajusta viewport.
    pub fn move_cursor(&mut self, direction: Direction, selecting: bool) {
        // Borrow buffer fuera del loop para evitar conflicto con &mut self.cursors
        let buffer = &self.buffer;
        for cursor in &mut self.cursors.cursors {
            match direction {
                Direction::Up => cursor.move_up(buffer, selecting),
                Direction::Down => cursor.move_down(buffer, selecting),
                Direction::Left => cursor.move_left(buffer, selecting),
                Direction::Right => cursor.move_right(buffer, selecting),
            }
        }
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve todos los cursores por palabra (Ctrl+Left/Right).
    ///
    /// Aplica `move_word_left/right` a cada `CursorInstance` y asegura
    /// la visibilidad del cursor primario en el viewport.
    pub fn move_cursor_word(&mut self, direction: Direction, selecting: bool) {
        // Borrow buffer fuera del loop para evitar conflicto con &mut self.cursors
        let buffer = &self.buffer;
        for cursor in &mut self.cursors.cursors {
            match direction {
                Direction::Left => cursor.move_word_left(buffer, selecting),
                Direction::Right => cursor.move_word_right(buffer, selecting),
                // Up/Down no aplican a movimiento por palabra — se ignora.
                Direction::Up | Direction::Down => {}
            }
        }
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Toggle line comment para todas las líneas afectadas por los cursores.
    ///
    /// - Determina el prefijo de comentario por extensión del archivo.
    /// - Si la extensión no soporta comentario de línea (HTML/JSON), no-op.
    /// - Para multicursor con selecciones multi-línea, dedup las líneas
    ///   afectadas via `BTreeSet` para evitar doble-toggle.
    pub fn toggle_line_comment(&mut self) {
        // Tab de diff → no editable
        if self.diff_view.is_some() {
            return;
        }

        // Detectar prefijo desde la extensión del archivo
        let ext_owned: Option<String> = self
            .buffer
            .file_path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        let prefix = match ext_owned.as_deref().and_then(comment_prefix) {
            Some(p) => p,
            None => return, // sin mapping → no-op
        };

        // Recolectar líneas afectadas por TODOS los cursores (con dedup)
        use std::collections::BTreeSet;
        let mut affected: BTreeSet<usize> = BTreeSet::new();
        for cursor in &self.cursors.cursors {
            if let Some(sel) = cursor.selection.filter(|s| !s.is_empty()) {
                let start_line = sel.start().line;
                let end_line = sel.end().line;
                for line in start_line..=end_line {
                    affected.insert(line);
                }
            } else {
                affected.insert(cursor.position.line);
            }
        }

        if affected.is_empty() {
            return;
        }

        let min_line = *affected.iter().next().unwrap_or(&0);

        // Aplicar toggle_comment línea por línea y registrar undo
        for &line_idx in &affected {
            let old = match self.buffer.line(line_idx) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let new = toggle_comment(&old, prefix);
            if new == old {
                continue; // no-op (línea vacía / whitespace) — no registrar undo
            }
            // CLONE: necesario — `new` se mueve al buffer, pero también lo
            // necesitamos en el undo op para poder hacer redo después.
            self.buffer.replace_line(line_idx, new.clone());
            self.undo_stack.push(EditOperation::ReplaceLine {
                line_idx,
                old,
                new,
            });
        }

        // Invalidar highlight desde la primera línea tocada
        self.highlight_cache.invalidate_from(min_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve la línea del cursor primario hacia arriba o abajo (Alt+Up/Down).
    ///
    /// Reglas:
    /// - Solo afecta al cursor primario; los secundarios se limpian.
    /// - No-op si el movimiento sale de los bordes del buffer.
    /// - El cursor sigue a la línea movida (mismo `col`, ajustado a su largo).
    pub fn move_line(&mut self, direction: Direction) {
        if self.diff_view.is_some() {
            return;
        }

        let primary = self.cursors.primary();
        let current = primary.position.line;
        let line_count = self.buffer.line_count();

        let target = match direction {
            Direction::Up => {
                if current == 0 {
                    return;
                }
                current - 1
            }
            Direction::Down => {
                if current + 1 >= line_count {
                    return;
                }
                current + 1
            }
            // Left/Right no aplican a move_line.
            Direction::Left | Direction::Right => return,
        };

        self.buffer.swap_lines(current, target);
        self.undo_stack.push(EditOperation::SwapLines {
            a: current,
            b: target,
        });

        // Limpiar cursores secundarios y mover el primario a la línea movida
        self.cursors.clear_secondary();
        let primary_mut = self.cursors.primary_mut();
        primary_mut.position.line = target;
        // Clamp col al largo de la nueva línea
        let new_line_len = self.buffer.line_len(target);
        if primary_mut.position.col > new_line_len {
            primary_mut.position.col = new_line_len;
        }
        primary_mut.sync_desired_col();
        primary_mut.clear_selection();

        let invalidate_from = current.min(target);
        self.highlight_cache.invalidate_from(invalidate_from);
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Duplica la(s) línea(s) del cursor hacia arriba o abajo (Shift+Alt+Up/Down).
    ///
    /// Reglas:
    /// - Diff tab → no-op (read-only).
    /// - Multi-cursor: dedup por línea via `BTreeSet`. Iteración descendente
    ///   para evitar invalidación de índices al insertar líneas.
    /// - Cada inserción registra un `EditOperation::InsertLine` independiente
    ///   en el undo stack.
    /// - Up: copia se inserta en `idx`; cursor permanece en `idx`.
    /// - Down: copia se inserta en `idx+1`; cursor se mueve a `idx+1`.
    /// - Cursores con `line >= insertion_point` se desplazan +1 por cada
    ///   inserción que ocurra debajo o en su misma línea.
    pub fn duplicate_line(&mut self, direction: Direction) {
        if self.diff_view.is_some() {
            return;
        }

        // Recolectar líneas únicas de TODOS los cursores (con dedup ordenado)
        use std::collections::BTreeSet;
        let mut affected: BTreeSet<usize> = BTreeSet::new();
        for cursor in &self.cursors.cursors {
            affected.insert(cursor.position.line);
        }
        if affected.is_empty() {
            return;
        }
        let min_affected = *affected.iter().next().unwrap_or(&0);

        // Iterar descendente — bottom-up — para no invalidar índices al insertar.
        for &line_idx in affected.iter().rev() {
            // Línea fuente — `to_owned` clona el contenido (necesario porque
            // luego vamos a mutar el buffer y empujar al undo stack).
            let content: String = match self.buffer.line(line_idx) {
                // CLONE: necesario — content se mueve al buffer y al undo op.
                Some(s) => s.to_owned(),
                None => continue,
            };
            let target_idx = match direction {
                Direction::Up => line_idx,
                Direction::Down => line_idx + 1,
                // Left/Right no aplican a duplicate_line.
                Direction::Left | Direction::Right => return,
            };
            // CLONE: necesario — `content` se mueve al buffer (insert_line
            // consume el String) y la versión del undo necesita su propia copia.
            self.buffer.insert_line(target_idx, content.clone());
            self.undo_stack.push(EditOperation::InsertLine {
                line: target_idx,
                content,
            });
        }

        // Ajustar cursores en función de las líneas afectadas (no de los target indices).
        // Down: cada `a in affected` con `a <= L` contribuye +1 al shift de L.
        //   - a < L: la inserción en `a+1 <= L` empuja L hacia abajo.
        //   - a == L: regla especial — el cursor "sigue" a la copia inferior (+1).
        // Up: cada `a in affected` con `a < L` contribuye +1.
        //   - a == L: el cursor permanece sobre la copia superior (no shift).
        //   - a < L: la inserción en `a` empuja L hacia abajo.
        for cursor in self.cursors.cursors.iter_mut() {
            let original_line = cursor.position.line;
            let shift = match direction {
                Direction::Down => affected
                    .iter()
                    .filter(|&&a| a <= original_line)
                    .count(),
                Direction::Up => affected.iter().filter(|&&a| a < original_line).count(),
                Direction::Left | Direction::Right => 0,
            };
            cursor.position.line = original_line + shift;
            cursor.clear_selection();
            cursor.sync_desired_col();
        }

        self.highlight_cache.invalidate_from(min_affected);
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Indenta cada línea cubierta por una selección activa (4 espacios).
    ///
    /// Reglas:
    /// - Diff tab → no-op.
    /// - Solo procesa cursores con selección no-vacía. Sin selecciones → no-op.
    /// - Para cada línea afectada (dedup via `BTreeSet`): prepend "    "
    ///   y registra un `ReplaceLine` undo op por línea.
    /// - Ajusta `position.col` y los `col` de las selecciones por +4
    ///   en las líneas que fueron indentadas.
    pub fn indent_selection(&mut self) {
        if self.diff_view.is_some() {
            return;
        }

        use std::collections::BTreeSet;
        let mut affected: BTreeSet<usize> = BTreeSet::new();
        for cursor in &self.cursors.cursors {
            if let Some(sel) = cursor.selection.filter(|s| !s.is_empty()) {
                let start_line = sel.start().line;
                let end_line = sel.end().line;
                for line in start_line..=end_line {
                    affected.insert(line);
                }
            }
        }
        if affected.is_empty() {
            return;
        }
        let min_line = *affected.iter().next().unwrap_or(&0);

        for &line_idx in &affected {
            // CLONE: necesario — `old` se conserva en el undo op para revertir.
            let old: String = match self.buffer.line(line_idx) {
                Some(s) => s.to_owned(),
                None => continue,
            };
            let new = format!("    {old}");
            // CLONE: necesario — `new` se mueve al buffer pero también lo
            // necesitamos en el undo op para poder hacer redo.
            self.buffer.replace_line(line_idx, new.clone());
            self.undo_stack.push(EditOperation::ReplaceLine {
                line_idx,
                old,
                new,
            });
        }

        // Ajustar columnas de cursores y selecciones.
        // Solo se desplaza +4 para posiciones cuya `line` fue indentada.
        for cursor in self.cursors.cursors.iter_mut() {
            if affected.contains(&cursor.position.line) {
                cursor.position.col += 4;
                cursor.desired_col = cursor.position.col;
            }
            if let Some(sel) = cursor.selection.as_mut() {
                if affected.contains(&sel.anchor.line) {
                    sel.anchor.col += 4;
                }
                if affected.contains(&sel.head.line) {
                    sel.head.col += 4;
                }
            }
        }

        self.highlight_cache.invalidate_from(min_line);
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Des-indenta cada línea cubierta por una selección activa
    /// (remueve hasta 4 espacios iniciales por línea).
    ///
    /// Reglas:
    /// - Diff tab → no-op.
    /// - Solo procesa cursores con selección no-vacía. Sin selecciones → no-op.
    /// - Cuenta espacios iniciales (no tabs) hasta un máximo de 4.
    /// - Líneas sin espacios iniciales → no-op (no registran undo).
    /// - Ajusta `col` de cursores y selecciones según los espacios removidos.
    pub fn unindent_selection(&mut self) {
        if self.diff_view.is_some() {
            return;
        }

        use std::collections::BTreeSet;
        let mut affected: BTreeSet<usize> = BTreeSet::new();
        for cursor in &self.cursors.cursors {
            if let Some(sel) = cursor.selection.filter(|s| !s.is_empty()) {
                let start_line = sel.start().line;
                let end_line = sel.end().line;
                for line in start_line..=end_line {
                    affected.insert(line);
                }
            }
        }
        if affected.is_empty() {
            return;
        }
        let min_line = *affected.iter().next().unwrap_or(&0);

        // line_idx → cantidad de espacios efectivamente removidos (0..=4)
        let mut removed_per_line: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();

        for &line_idx in &affected {
            // CLONE: necesario para `old` en el undo op.
            let old: String = match self.buffer.line(line_idx) {
                Some(s) => s.to_owned(),
                None => continue,
            };
            // Contar espacios iniciales (NO tabs) hasta máximo 4.
            let removed = old
                .as_bytes()
                .iter()
                .take(4)
                .take_while(|&&b| b == b' ')
                .count();
            if removed == 0 {
                removed_per_line.insert(line_idx, 0);
                continue; // no-op, sin undo
            }
            let new: String = old[removed..].to_owned();
            // CLONE: necesario — `new` se mueve al buffer y se conserva en undo.
            self.buffer.replace_line(line_idx, new.clone());
            self.undo_stack.push(EditOperation::ReplaceLine {
                line_idx,
                old,
                new,
            });
            removed_per_line.insert(line_idx, removed);
        }

        // Ajustar cursores y selecciones según los espacios removidos por línea.
        for cursor in self.cursors.cursors.iter_mut() {
            if let Some(&removed) = removed_per_line.get(&cursor.position.line) {
                cursor.position.col = cursor.position.col.saturating_sub(removed);
                cursor.desired_col = cursor.position.col;
            }
            if let Some(sel) = cursor.selection.as_mut() {
                if let Some(&removed) = removed_per_line.get(&sel.anchor.line) {
                    sel.anchor.col = sel.anchor.col.saturating_sub(removed);
                }
                if let Some(&removed) = removed_per_line.get(&sel.head.line) {
                    sel.head.col = sel.head.col.saturating_sub(removed);
                }
            }
        }

        self.highlight_cache.invalidate_from(min_line);
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Mueve el cursor primario al inicio de la línea actual.
    pub fn move_to_line_start(&mut self) {
        let primary = self.cursors.primary_mut();
        primary.position.col = 0;
        primary.desired_col = 0;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario al final de la línea actual.
    pub fn move_to_line_end(&mut self) {
        let line = self.cursors.primary().position.line;
        let line_len = self.buffer.line_len(line);
        let primary = self.cursors.primary_mut();
        primary.position.col = line_len;
        primary.desired_col = line_len;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario a una línea específica (0-indexed) y centra el viewport.
    ///
    /// Clampea la línea al rango válido del buffer. Cursor va a columna 0.
    /// El viewport se centra en la línea target para dar contexto visual.
    pub fn go_to_line(&mut self, line_idx: usize) {
        let clamped = line_idx.min(self.buffer.line_count().saturating_sub(1));
        let primary = self.cursors.primary_mut();
        primary.position.line = clamped;
        primary.position.col = 0;
        primary.desired_col = 0;
        primary.clear_selection();
        // Centrar viewport en la línea target
        let half_viewport = self.viewport.height / 2;
        self.viewport.scroll_offset = clamped.saturating_sub(half_viewport);
    }

    /// Mueve el cursor primario al inicio absoluto del buffer.
    pub fn move_to_buffer_start(&mut self) {
        let primary = self.cursors.primary_mut();
        primary.position = Position::zero();
        primary.desired_col = 0;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Mueve el cursor primario al final absoluto del buffer.
    pub fn move_to_buffer_end(&mut self) {
        let last_line = self.buffer.line_count().saturating_sub(1);
        let last_col = self.buffer.line_len(last_line);
        let primary = self.cursors.primary_mut();
        primary.position = Position {
            line: last_line,
            col: last_col,
        };
        primary.desired_col = last_col;
        primary.clear_selection();
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Agrega un cursor encima del cursor topmost actual (línea mínima).
    ///
    /// La columna ancla viene del `desired_col` del primario — esto preserva
    /// la intención de columna a través de adiciones en cascada (Ctrl+Alt+↑
    /// presionado N veces). La columna se clampea a la longitud de la línea
    /// destino.
    ///
    /// No-op si el cursor topmost está en línea 0 o si la tab es de diff
    /// (read-only).
    ///
    /// NOTA: con un solo cursor, topmost == primary, así que el comportamiento
    /// es idéntico al original. La diferencia aparece en cascadas.
    pub fn add_cursor_above(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        // Ancla de columna desde el primary's desired_col — mantiene la
        // intención de columna estable a lo largo de cascadas.
        let anchor_col = self.cursors.primary().desired_col;

        // Línea de referencia = cursor topmost (línea mínima).
        // unwrap_or(0): cursors es no-vacío por invariante (siempre hay primary).
        let topmost_line = self
            .cursors
            .cursors
            .iter()
            .map(|c| c.position.line)
            .min()
            .unwrap_or(0);

        let target_line = match topmost_line.checked_sub(1) {
            Some(l) => l,
            None => return, // topmost ya en línea 0 → no-op
        };
        let target_col = anchor_col.min(self.buffer.line_len(target_line));
        let target_pos = Position {
            line: target_line,
            col: target_col,
        };
        self.cursors.add_cursor(target_pos, None);
        self.cursors.sort_by_position();
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Agrega un cursor debajo del cursor bottommost actual (línea máxima).
    ///
    /// La columna ancla viene del `desired_col` del primario — esto preserva
    /// la intención de columna a través de adiciones en cascada (Ctrl+Alt+↓
    /// presionado N veces). La columna se clampea a la longitud de la línea
    /// destino.
    ///
    /// No-op si el cursor bottommost está en la última línea o si la tab es
    /// de diff (read-only).
    pub fn add_cursor_below(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        let anchor_col = self.cursors.primary().desired_col;

        // Línea de referencia = cursor bottommost (línea máxima).
        // unwrap_or(0): cursors es no-vacío por invariante (siempre hay primary).
        let bottommost_line = self
            .cursors
            .cursors
            .iter()
            .map(|c| c.position.line)
            .max()
            .unwrap_or(0);

        let target_line = bottommost_line + 1;
        if target_line >= self.buffer.line_count() {
            return; // bottommost ya en la última línea → no-op
        }
        let target_col = anchor_col.min(self.buffer.line_len(target_line));
        let target_pos = Position {
            line: target_line,
            col: target_col,
        };
        self.cursors.add_cursor(target_pos, None);
        self.cursors.sort_by_position();
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Extiende selección de TODOS los cursores al inicio de su línea.
    ///
    /// Para cada cursor: inicia selección si no existe, mueve la columna a 0,
    /// actualiza `desired_col` a 0, y extiende la selección. No-op si la tab
    /// es de diff (read-only).
    pub fn move_to_line_start_selecting(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        for cursor in &mut self.cursors.cursors {
            cursor.start_selection();
            cursor.position.col = 0;
            cursor.desired_col = 0;
            cursor.extend_selection();
        }
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Extiende selección de TODOS los cursores al final de su línea.
    ///
    /// Para cada cursor: inicia selección si no existe, mueve la columna al
    /// final de su línea, sincroniza `desired_col`, y extiende la selección.
    /// No-op si la tab es de diff (read-only).
    pub fn move_to_line_end_selecting(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        for cursor in &mut self.cursors.cursors {
            cursor.start_selection();
            let line_len = self.buffer.line_len(cursor.position.line);
            cursor.position.col = line_len;
            cursor.desired_col = line_len;
            cursor.extend_selection();
        }
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Extiende selección de TODOS los cursores al inicio absoluto del buffer (0, 0).
    ///
    /// Para cada cursor: inicia selección si no existe, mueve la posición a
    /// (0, 0), y extiende la selección. No-op si la tab es de diff (read-only).
    pub fn move_to_buffer_start_selecting(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        for cursor in &mut self.cursors.cursors {
            cursor.start_selection();
            cursor.position = Position::zero();
            cursor.desired_col = 0;
            cursor.extend_selection();
        }
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Extiende selección de TODOS los cursores al final absoluto del buffer.
    ///
    /// Para cada cursor: inicia selección si no existe, mueve la posición a
    /// la última línea y su última columna, y extiende la selección. No-op
    /// si la tab es de diff (read-only).
    pub fn move_to_buffer_end_selecting(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        let last_line = self.buffer.line_count().saturating_sub(1);
        let last_col = self.buffer.line_len(last_line);
        for cursor in &mut self.cursors.cursors {
            cursor.start_selection();
            cursor.position = Position {
                line: last_line,
                col: last_col,
            };
            cursor.desired_col = last_col;
            cursor.extend_selection();
        }
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    /// Selecciona la siguiente ocurrencia del texto seleccionado (Ctrl+D).
    ///
    /// 1. Si no hay selección → seleccionar la palabra bajo el cursor.
    /// 2. Si hay selección → buscar la siguiente ocurrencia y agregar un cursor.
    pub fn select_next_occurrence(&mut self) {
        let primary = self.cursors.primary();
        let primary_sel = primary.selection;
        let primary_pos = primary.position;

        if primary_sel.is_none() || primary_sel.is_some_and(|s| s.is_empty()) {
            // Caso 1: sin selección — seleccionar la palabra bajo el cursor
            if let Some((word_start, word_end)) = self.word_at_position(primary_pos) {
                let sel = Selection::new(word_start, word_end);
                let primary = self.cursors.primary_mut();
                primary.position = word_end;
                primary.selection = Some(sel);
                primary.sync_desired_col();
            }
            return;
        }

        // Caso 2: ya hay selección — buscar la siguiente ocurrencia
        let sel = primary_sel.expect("ya verificamos que es Some");
        let search_text = sel.selected_text(&self.buffer);
        if search_text.is_empty() {
            return;
        }

        // Encontrar la posición después de la última selección (de cualquier cursor)
        let last_end = self
            .cursors
            .cursors
            .iter()
            .filter_map(|c| c.selection.map(|s| s.end()))
            .max()
            .unwrap_or(sel.end());

        // Buscar siguiente ocurrencia después de last_end
        if let Some((match_start, match_end)) =
            self.find_next_occurrence(&search_text, last_end, false)
        {
            let new_sel = Selection::new(match_start, match_end);
            self.cursors.add_cursor(match_end, Some(new_sel));
        } else {
            // Wrap around: buscar desde el inicio del buffer
            let buffer_start = Position::zero();
            if let Some((match_start, match_end)) =
                self.find_next_occurrence(&search_text, buffer_start, true)
            {
                // Verificar que no sea una ocurrencia que ya tiene cursor
                let already_has = self.cursors.cursors.iter().any(|c| {
                    c.selection
                        .is_some_and(|s| s.start() == match_start && s.end() == match_end)
                });
                if !already_has {
                    let new_sel = Selection::new(match_start, match_end);
                    self.cursors.add_cursor(match_end, Some(new_sel));
                }
            }
        }
    }

    /// Encuentra los límites de la palabra en la posición dada.
    ///
    /// Una "palabra" son caracteres ASCII alfanuméricos + `_` contiguos.
    /// Retorna (start, end) de la palabra en byte offsets, o None si no hay palabra.
    ///
    /// Implementación char-aware via `char_indices()` — segura sobre líneas
    /// con caracteres multi-byte UTF-8 (no entra a posiciones intermedias de byte).
    fn word_at_position(&self, pos: Position) -> Option<(Position, Position)> {
        let line = self.buffer.line(pos.line)?;
        if line.is_empty() {
            return None;
        }

        // Helper: clase del char en byte_idx; None si fuera de rango o no boundary.
        let class_at = |byte_idx: usize| -> Option<bool> {
            if byte_idx >= line.len() || !line.is_char_boundary(byte_idx) {
                return None;
            }
            line[byte_idx..].chars().next().map(is_word_char)
        };

        // Caso 1: el char EN pos.col es word_char → expandir en ambas direcciones
        if class_at(pos.col) == Some(true) {
            // Buscar inicio: retroceder mientras char anterior es word.
            let mut start = pos.col;
            loop {
                let prev = unicode::prev_char_boundary(line, start);
                if prev == start { break; }
                let is_word = line[prev..].chars().next().is_some_and(is_word_char);
                if !is_word { break; }
                start = prev;
            }
            // Buscar fin: avanzar mientras char actual es word.
            let mut end = pos.col;
            while end < line.len() {
                let ch = line[end..].chars().next();
                if ch.is_some_and(is_word_char) {
                    end += unicode::char_len_at(line, end);
                } else {
                    break;
                }
            }
            if start == end { return None; }
            return Some((
                Position { line: pos.line, col: start },
                Position { line: pos.line, col: end },
            ));
        }

        // Caso 2: el char en pos.col NO es word — probar el char anterior
        // (cursor justo después del final de una palabra).
        if pos.col > 0 {
            let prev = unicode::prev_char_boundary(line, pos.col);
            let prev_is_word = line[prev..].chars().next().is_some_and(is_word_char);
            if prev_is_word {
                // Expandir hacia la izquierda
                let mut start = prev;
                loop {
                    let p = unicode::prev_char_boundary(line, start);
                    if p == start { break; }
                    let is_w = line[p..].chars().next().is_some_and(is_word_char);
                    if !is_w { break; }
                    start = p;
                }
                return Some((
                    Position { line: pos.line, col: start },
                    Position { line: pos.line, col: pos.col },
                ));
            }
        }

        None
    }

    /// Busca la siguiente ocurrencia de `text` en el buffer después de `after`.
    ///
    /// Si `stop_at_start` es true, solo busca hasta el inicio del buffer
    /// (para wrap-around sin loops infinitos).
    fn find_next_occurrence(
        &self,
        text: &str,
        after: Position,
        _stop_at_start: bool,
    ) -> Option<(Position, Position)> {
        let total_lines = self.buffer.line_count();

        // Buscar en la línea actual después de `after.col`
        if let Some(line_content) = self.buffer.line(after.line) {
            let search_from = after.col.min(line_content.len());
            if let Some(offset) = line_content[search_from..].find(text) {
                let col = search_from + offset;
                return Some((
                    Position {
                        line: after.line,
                        col,
                    },
                    Position {
                        line: after.line,
                        col: col + text.len(),
                    },
                ));
            }
        }

        // Buscar en las líneas siguientes
        for line_idx in (after.line + 1)..total_lines {
            if let Some(line_content) = self.buffer.line(line_idx)
                && let Some(col) = line_content.find(text)
            {
                return Some((
                    Position {
                        line: line_idx,
                        col,
                    },
                    Position {
                        line: line_idx,
                        col: col + text.len(),
                    },
                ));
            }
        }

        // Si estamos en wrap-around, buscar desde el inicio
        if after.line > 0 || after.col > 0 {
            for line_idx in 0..=after.line.min(total_lines.saturating_sub(1)) {
                if let Some(line_content) = self.buffer.line(line_idx) {
                    let end_col = if line_idx == after.line {
                        after.col
                    } else {
                        line_content.len()
                    };
                    let search_range = &line_content[..end_col.min(line_content.len())];
                    if let Some(col) = search_range.find(text) {
                        return Some((
                            Position {
                                line: line_idx,
                                col,
                            },
                            Position {
                                line: line_idx,
                                col: col + text.len(),
                            },
                        ));
                    }
                }
            }
        }

        None
    }

    /// Deshace la última operación de edición.
    pub fn undo(&mut self) {
        let Some(op) = self.undo_stack.undo() else {
            return;
        };
        let primary = self.cursors.primary_mut();
        match op {
            EditOperation::InsertChar { pos, .. } => {
                self.buffer.remove_char_at(pos);
                primary.position = pos;
            }
            EditOperation::DeleteChar { pos, ch } => {
                self.buffer.raw_insert_char(pos, ch);
                primary.position = Position {
                    line: pos.line,
                    col: pos.col + ch.len_utf8(),
                };
            }
            EditOperation::InsertNewline { pos } => {
                self.buffer.join_lines(pos.line);
                primary.position = pos;
            }
            EditOperation::DeleteNewline { pos, col } => {
                self.buffer.split_line_at(pos.line, col);
                primary.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
            EditOperation::SwapLines { a, b } => {
                // Swap es su propia inversa.
                self.buffer.swap_lines(a, b);
                primary.position = Position {
                    line: a.min(b),
                    col: 0,
                };
            }
            EditOperation::ReplaceLine { line_idx, ref old, .. } => {
                // CLONE: necesario — `old` es ref del op, replace_line consume String
                self.buffer.replace_line(line_idx, old.clone());
                primary.position = Position {
                    line: line_idx,
                    col: 0,
                };
            }
            EditOperation::InsertLine { line, .. } => {
                // Undo: borrar la línea insertada.
                self.buffer.delete_line(line);
                primary.position = Position {
                    line: line.saturating_sub(1),
                    col: 0,
                };
            }
            EditOperation::InsertText { start, end, .. } => {
                // Undo de inserción atómica: borrar el rango insertado.
                // El String `text` no se necesita acá — solo para redo.
                let _ = self.buffer.delete_range(start, end);
                primary.position = start;
            }
            EditOperation::DeleteRange { start, ref text, .. } => {
                // Undo de borrado atómico: re-insertar el texto borrado.
                let new_end = self.buffer.insert_text(start, text);
                primary.position = new_end;
            }
        }
        primary.sync_desired_col();
        primary.clear_selection();
        // Limpiar cursores secundarios en undo
        self.cursors.clear_secondary();
        // Invalidar desde la línea afectada por el undo
        let undo_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(undo_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Rehace la última operación deshecha.
    pub fn redo(&mut self) {
        let Some(op) = self.undo_stack.redo() else {
            return;
        };
        let primary = self.cursors.primary_mut();
        match op {
            EditOperation::InsertChar { pos, ch } => {
                self.buffer.raw_insert_char(pos, ch);
                primary.position = Position {
                    line: pos.line,
                    col: pos.col + ch.len_utf8(),
                };
            }
            EditOperation::DeleteChar { pos, .. } => {
                self.buffer.remove_char_at(pos);
                primary.position = pos;
            }
            EditOperation::InsertNewline { pos } => {
                self.buffer.insert_newline(pos);
                primary.position = Position {
                    line: pos.line + 1,
                    col: 0,
                };
            }
            EditOperation::DeleteNewline { pos, col } => {
                self.buffer.join_lines(pos.line);
                primary.position = Position {
                    line: pos.line,
                    col,
                };
            }
            EditOperation::SwapLines { a, b } => {
                self.buffer.swap_lines(a, b);
                primary.position = Position {
                    line: a.max(b),
                    col: 0,
                };
            }
            EditOperation::ReplaceLine { line_idx, ref new, .. } => {
                // CLONE: necesario — replace_line consume String
                self.buffer.replace_line(line_idx, new.clone());
                primary.position = Position {
                    line: line_idx,
                    col: 0,
                };
            }
            EditOperation::InsertLine { line, ref content } => {
                // CLONE: redo necesita conservar `content` para futuras re-aplicaciones.
                self.buffer.insert_line(line, content.clone());
                primary.position = Position { line, col: 0 };
            }
            EditOperation::InsertText { start, ref text, .. } => {
                // Redo de inserción atómica: re-insertar el texto.
                let new_end = self.buffer.insert_text(start, text);
                primary.position = new_end;
            }
            EditOperation::DeleteRange { start, end, .. } => {
                // Redo de borrado atómico: re-borrar el rango.
                let _ = self.buffer.delete_range(start, end);
                primary.position = start;
            }
        }
        primary.sync_desired_col();
        primary.clear_selection();
        self.cursors.clear_secondary();
        // Invalidar desde la línea afectada por el redo
        let redo_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(redo_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Guarda el archivo asociado al buffer.
    pub fn save(&mut self) -> Result<()> {
        self.buffer.save()
    }

    /// Limpia los cursores secundarios (Esc con multicursor activo).
    pub fn clear_multicursors(&mut self) {
        self.cursors.clear_secondary();
    }

    /// Verifica si hay múltiples cursores activos.
    pub fn has_multicursors(&self) -> bool {
        self.cursors.has_multiple()
    }

    // ─── Select All ────────────────────────────────────────────────────────────

    /// Selecciona todo el contenido del buffer (Ctrl+A).
    ///
    /// Limpia los cursores secundarios y deja un único cursor primario con
    /// `anchor` en (0,0) y `head` al final del buffer. El head queda en la
    /// posición final para que un movimiento posterior con Shift+flecha
    /// extienda desde ahí (comportamiento estándar de editores).
    pub fn select_all(&mut self) {
        self.cursors.clear_secondary();
        let last_line = self.buffer.line_count().saturating_sub(1);
        let last_col = self.buffer.line_len(last_line);
        let start = Position { line: 0, col: 0 };
        let end = Position {
            line: last_line,
            col: last_col,
        };
        let primary = self.cursors.primary_mut();
        primary.position = end;
        primary.selection = Some(Selection::new(start, end));
        primary.sync_desired_col();
    }

    // ─── Select Line (Ctrl+L) ──────────────────────────────────────────────────

    /// Selecciona la línea completa de cada cursor activo.
    ///
    /// Para cada cursor: anchor=(line, 0), head=(line, line_len). El cursor
    /// queda al final de la línea para que un movimiento posterior con
    /// Shift+flecha extienda desde ahí (comportamiento estándar de editores).
    /// No muta el buffer — no se registra undo.
    /// No-op en tabs de diff (read-only).
    pub fn select_line(&mut self) {
        if self.diff_view.is_some() {
            return;
        }
        // Mutamos cada cursor del vector — no clonamos el buffer ni los cursors.
        // Leemos line_len antes para no sostener &self.buffer y &mut cursor a la vez.
        let lens: Vec<(usize, usize)> = self
            .cursors
            .cursors
            .iter()
            .map(|c| {
                let line = c.position.line;
                (line, self.buffer.line_len(line))
            })
            .collect();
        for (cursor, (line, line_len)) in self.cursors.cursors.iter_mut().zip(lens.into_iter()) {
            let start = Position { line, col: 0 };
            let end = Position { line, col: line_len };
            cursor.selection = Some(Selection::new(start, end));
            cursor.position = end;
            cursor.desired_col = line_len;
        }
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    // ─── Clipboard helpers ──────────────────────────────────────────────────────

    /// Borra la selección del cursor primario, si la hay.
    ///
    /// Reutiliza `delete_selection_at` internamente. No hace nada si no hay
    /// selección activa. Usado por Cut y Paste (cuando el paste reemplaza
    /// el rango seleccionado).
    pub fn delete_primary_selection(&mut self) {
        let idx = self.cursors.primary_index;
        let has_sel = self.cursors.cursors[idx]
            .selection
            .is_some_and(|s| !s.is_empty());
        if !has_sel {
            return;
        }
        self.delete_selection_at(idx);
        let edited_line = self.cursors.primary().position.line;
        self.highlight_cache.invalidate_from(edited_line);
        self.viewport
            .ensure_cursor_visible(&self.cursors.primary().position);
    }

    /// Inserta `text` en la posición de cada cursor activo como UNA operación
    /// atómica por cursor.
    ///
    /// Garantías:
    /// - Exactamente 1 entrada `EditOperation::InsertText` por cursor (no N
    ///   por carácter — esto evita rebuild caro del undo stack en pastes grandes).
    /// - Si un cursor tiene selección, primero se borra atómicamente
    ///   (`EditOperation::DeleteRange`).
    /// - Soporte multi-byte UTF-8 sin panic — usa `buffer.insert_text`.
    /// - 1 invalidación del highlight cache total, no N.
    /// - CRLF entrante (`\r\n`) se normaliza a LF antes de insertar.
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        // Normalizar CRLF → LF. CLONE: solo si el input contiene '\r' — si no, se evita.
        // Caso típico de paste desde Windows tiene CRLF; el resto pasa sin alocar.
        let normalized: std::borrow::Cow<'_, str> = if text.contains('\r') {
            // CLONE: replace devuelve String owned — necesario para normalizar.
            std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', ""))
        } else {
            std::borrow::Cow::Borrowed(text)
        };
        let text = normalized.as_ref();

        self.cursors.sort_by_position();
        let mut min_invalidated_line: Option<usize> = None;

        for i in (0..self.cursors.cursors.len()).rev() {
            // 1) Si hay selección activa, borrar atómicamente como DeleteRange.
            if let Some(sel) = self.cursors.cursors[i].selection
                && !sel.is_empty()
            {
                let start = sel.start();
                let end = sel.end();
                // CLONE: delete_range retorna String owned para guardar en undo.
                let deleted = self.buffer.delete_range(start, end);
                self.undo_stack.push(EditOperation::DeleteRange {
                    start,
                    end,
                    text: deleted,
                });
                self.cursors.cursors[i].position = start;
                self.cursors.cursors[i].clear_selection();
            }

            // 2) Insertar el bloque de texto atómicamente.
            let pos = self.cursors.cursors[i].position;
            let end_pos = self.buffer.insert_text(pos, text);
            self.undo_stack.push(EditOperation::InsertText {
                start: pos,
                end: end_pos,
                // CLONE: text se conserva como owned para soportar redo.
                text: text.to_string(),
            });
            self.cursors.cursors[i].position = end_pos;
            self.cursors.cursors[i].sync_desired_col();
            self.cursors.cursors[i].clear_selection();

            min_invalidated_line = Some(min_invalidated_line.map_or(pos.line, |m| m.min(pos.line)));
        }

        // 3) Invalidar highlight UNA vez, desde la línea más temprana editada.
        if let Some(line) = min_invalidated_line {
            self.highlight_cache.invalidate_from(line);
        }
        // 4) Ensure viewport visibility UNA vez (cursor primario).
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }

    // ─── File Search (Ctrl+F) ───────────────────────────────────────────────────

    /// Abre el search bar del archivo actual.
    ///
    /// Si hay texto seleccionado, lo usa como query inicial y ejecuta la
    /// búsqueda inmediatamente. Si no, abre el search bar vacío.
    pub fn open_file_search(&mut self) {
        // CLONE: necesario — el primary tiene &self pero después llamamos &mut self.
        // selected_text retorna String owned, no es ineficiente acá (operación rara,
        // sólo al abrir el search bar — fuera de hot path).
        let initial_query = self
            .cursors
            .primary()
            .selection
            .filter(|s| !s.is_empty())
            .map(|sel| sel.selected_text(&self.buffer))
            .unwrap_or_default();
        let case_sensitive = false;
        let mut s = search::BufferSearch::new(&initial_query, case_sensitive);
        if !initial_query.is_empty() {
            s.search(&self.buffer);
            // Saltar al primer match si lo hay
            if let Some(m) = s.matches.first().copied() {
                self.move_primary_to_match(m);
            }
        }
        self.search = Some(s);
    }

    /// Inserta un carácter en el query del file search y re-ejecuta la búsqueda.
    pub fn file_search_insert(&mut self, ch: char) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.query.push(ch);
        s.search(&self.buffer);
        if let Some(m) = s.matches.first().copied() {
            self.move_primary_to_match(m);
        }
    }

    /// Borra el último carácter del query del file search.
    pub fn file_search_delete(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.query.pop();
        s.search(&self.buffer);
        if let Some(m) = s.matches.first().copied() {
            self.move_primary_to_match(m);
        }
    }

    /// Salta al siguiente match del file search.
    pub fn file_search_next(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        if s.matches.is_empty() {
            return;
        }
        s.next_match();
        if let Some(idx) = s.current_match
            && let Some(m) = s.matches.get(idx).copied()
        {
            self.move_primary_to_match(m);
        }
    }

    /// Salta al match anterior del file search.
    pub fn file_search_prev(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        if s.matches.is_empty() {
            return;
        }
        s.prev_match();
        if let Some(idx) = s.current_match
            && let Some(m) = s.matches.get(idx).copied()
        {
            self.move_primary_to_match(m);
        }
    }

    /// Cierra el file search y limpia el estado.
    pub fn file_search_close(&mut self) {
        self.search = None;
    }

    /// Toggle case-sensitive del file search y re-ejecuta.
    pub fn file_search_toggle_case(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.case_sensitive = !s.case_sensitive;
        s.search(&self.buffer);
        if let Some(m) = s.matches.first().copied() {
            self.move_primary_to_match(m);
        }
    }

    /// Mueve el cursor primario al inicio de un match y asegura visibilidad.
    fn move_primary_to_match(&mut self, m: search::SearchMatch) {
        self.cursors.clear_secondary();
        let primary = self.cursors.primary_mut();
        primary.position = Position {
            line: m.line,
            col: m.start_col,
        };
        primary.sync_desired_col();
        primary.clear_selection();
        let pos = self.cursors.primary().position;
        self.viewport.ensure_cursor_visible(&pos);
    }
}

/// Verifica si un `char` es parte de una "palabra" (alfanumérico ASCII o `_`).
///
/// Firma `char` (no `u8`) para soportar iteración via `char_indices()` sobre
/// strings con caracteres multi-byte UTF-8 sin acceso unsafe a bytes.
/// Mantiene la semántica original (solo ASCII alfanumérico) — palabras con
/// acentos como "código" se delimitan en la 'ó' (no es ascii_alphanumeric).
/// Esa decisión es deliberada y consistente con la conducta existente.
///
/// Visibilidad `pub(super)` — multicursor.rs lo usa para movimiento por palabra.
pub(super) fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// ─── Pure helpers: comment toggle ─────────────────────────────────────────────

/// Mapea una extensión de archivo a su prefijo de comentario de línea.
///
/// Retorna `None` para extensiones donde `Ctrl+/` debe ser un no-op
/// (HTML, JSON, etc. — no usan comentarios de línea simples).
/// Cero allocaciones: el prefijo es `&'static str`.
fn comment_prefix(ext: &str) -> Option<&'static str> {
    match ext {
        // C-family
        "rs" | "js" | "ts" | "tsx" | "jsx" | "go" | "c" | "h" | "cpp" | "hpp" | "css"
        | "scss" | "java" | "kt" | "swift" | "dart" | "zig" => Some("// "),
        // Hash-comment family
        "py" | "rb" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "ini" | "conf" => {
            Some("# ")
        }
        // SQL
        "sql" => Some("-- "),
        _ => None,
    }
}

/// Aplica/quita un prefijo de comentario de línea sobre `line`.
///
/// Comportamiento:
/// - Línea vacía o solo whitespace → no se toca (retorna copia tal cual).
/// - Línea ya comentada (primer non-whitespace == prefijo) → quita el prefijo.
/// - Línea sin comentar → inserta el prefijo después del whitespace inicial.
///
/// Función pura: solo asigna `String` para el resultado. Trivialmente testeable.
fn toggle_comment(line: &str, prefix: &str) -> String {
    // Detectar inicio del primer non-whitespace
    let leading_ws_len = line
        .as_bytes()
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    let rest = &line[leading_ws_len..];

    // Línea vacía / solo whitespace → no-op
    if rest.is_empty() {
        return line.to_string();
    }

    // ¿Ya comentada?
    if let Some(after) = rest.strip_prefix(prefix) {
        // Quitar el prefijo
        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..leading_ws_len]);
        out.push_str(after);
        return out;
    }

    // El prefijo puede venir con espacio final (ej "// ") pero la línea
    // existente puede tener "//foo" sin el espacio. Manejamos el caso
    // de prefijo sin trailing space.
    let trimmed_prefix = prefix.trim_end();
    if !trimmed_prefix.is_empty()
        && trimmed_prefix != prefix
        && let Some(after) = rest.strip_prefix(trimmed_prefix)
    {
        let mut out = String::with_capacity(line.len());
        out.push_str(&line[..leading_ws_len]);
        out.push_str(after);
        return out;
    }

    // Sin comentar → agregar prefijo
    let mut out = String::with_capacity(line.len() + prefix.len());
    out.push_str(&line[..leading_ws_len]);
    out.push_str(prefix);
    out.push_str(rest);
    out
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    // ── comment_prefix ──

    #[test]
    fn comment_prefix_rs_returns_double_slash() {
        assert_eq!(comment_prefix("rs"), Some("// "));
    }

    #[test]
    fn comment_prefix_py_returns_hash() {
        assert_eq!(comment_prefix("py"), Some("# "));
    }

    #[test]
    fn comment_prefix_sql_returns_double_dash() {
        assert_eq!(comment_prefix("sql"), Some("-- "));
    }

    #[test]
    fn comment_prefix_unknown_returns_none() {
        assert_eq!(comment_prefix("unknown_ext"), None);
        assert_eq!(comment_prefix("json"), None);
        assert_eq!(comment_prefix("html"), None);
    }

    // ── toggle_comment ──

    #[test]
    fn toggle_comment_uncommented_line_adds_prefix() {
        let result = toggle_comment("let x = 5;", "// ");
        assert_eq!(result, "// let x = 5;");
    }

    #[test]
    fn toggle_comment_preserves_leading_whitespace() {
        let result = toggle_comment("    let x = 5;", "// ");
        assert_eq!(result, "    // let x = 5;");
    }

    #[test]
    fn toggle_comment_removes_prefix_when_already_commented() {
        let result = toggle_comment("    // const a = 1;", "// ");
        assert_eq!(result, "    const a = 1;");
    }

    #[test]
    fn toggle_comment_removes_prefix_without_trailing_space() {
        // Línea fue comentada como "//foo" — el toggle debe sacar el "//"
        let result = toggle_comment("//foo", "// ");
        assert_eq!(result, "foo");
    }

    #[test]
    fn toggle_comment_empty_line_is_noop() {
        let result = toggle_comment("", "// ");
        assert_eq!(result, "");
    }

    #[test]
    fn toggle_comment_whitespace_only_is_noop() {
        let result = toggle_comment("    ", "// ");
        assert_eq!(result, "    ");
    }

    #[test]
    fn toggle_comment_python_prefix() {
        let result = toggle_comment("x = 5", "# ");
        assert_eq!(result, "# x = 5");
        let back = toggle_comment(&result, "# ");
        assert_eq!(back, "x = 5");
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod editor_state_tests {
    use super::*;

    fn editor_with(text: &str) -> EditorState {
        EditorState {
            buffer: TextBuffer::from_text(text),
            cursors: MultiCursorState::new(),
            viewport: Viewport::new(),
            undo_stack: UndoStack::new(),
            search: None,
            highlight_cache: HighlightCache::new(),
            highlight_deferred: false,
            diff_view: None,
            image_view: None,
        }
    }

    // ── move_line ──

    #[test]
    fn move_line_down_swaps_with_next() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.move_line(Direction::Down);
        assert_eq!(ed.buffer.line(0), Some("b"));
        assert_eq!(ed.buffer.line(1), Some("a"));
        assert_eq!(ed.buffer.line(2), Some("c"));
        assert_eq!(ed.cursors.primary().position.line, 1);
    }

    #[test]
    fn move_line_up_swaps_with_previous() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 2, col: 0 };
        ed.move_line(Direction::Up);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("c"));
        assert_eq!(ed.buffer.line(2), Some("b"));
        assert_eq!(ed.cursors.primary().position.line, 1);
    }

    #[test]
    fn move_line_up_at_first_line_is_noop() {
        let mut ed = editor_with("a\nb");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.move_line(Direction::Up);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
        assert_eq!(ed.cursors.primary().position.line, 0);
    }

    #[test]
    fn move_line_down_at_last_line_is_noop() {
        let mut ed = editor_with("a\nb");
        ed.cursors.primary_mut().position = Position { line: 1, col: 0 };
        ed.move_line(Direction::Down);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
    }

    #[test]
    fn move_line_undo_round_trip() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.move_line(Direction::Down);
        // Estado tras mover: ["b", "a", "c"]
        assert_eq!(ed.buffer.line(0), Some("b"));
        ed.undo();
        // Tras undo, vuelve al estado original
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
        ed.redo();
        // Tras redo, vuelve a aplicarse
        assert_eq!(ed.buffer.line(0), Some("b"));
        assert_eq!(ed.buffer.line(1), Some("a"));
    }

    // ── toggle_line_comment ──
    // Necesita un file_path para resolver la extensión. Construímos un editor
    // con un path sintético via from_file no es posible (requiere I/O real),
    // así que insertamos directamente en el buffer.

    fn editor_with_ext(text: &str, ext: &str) -> EditorState {
        // Cada test obtiene un id único — process id + counter atómico — para
        // evitar colisiones entre tests que corren en paralelo. Antes solo
        // usábamos process id y los tests pisaban su archivo entre sí.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir();
        let path = dir.join(format!("__sdd_test_{}_{}.{}", std::process::id(), n, ext));
        std::fs::write(&path, text).expect("test write");
        let ed = EditorState::open_file(&path).expect("test open");
        // Limpiar archivo temporal — ya lo cargamos en buffer
        let _ = std::fs::remove_file(&path);
        ed
    }

    #[test]
    fn toggle_line_comment_adds_prefix_for_rs_file() {
        let mut ed = editor_with_ext("let x = 5;", "rs");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.toggle_line_comment();
        assert_eq!(ed.buffer.line(0), Some("// let x = 5;"));
    }

    #[test]
    fn toggle_line_comment_removes_prefix_when_already_commented() {
        let mut ed = editor_with_ext("// let x = 5;", "rs");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.toggle_line_comment();
        assert_eq!(ed.buffer.line(0), Some("let x = 5;"));
    }

    #[test]
    fn toggle_line_comment_undo_round_trip() {
        let mut ed = editor_with_ext("let x = 5;", "rs");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.toggle_line_comment();
        assert_eq!(ed.buffer.line(0), Some("// let x = 5;"));
        ed.undo();
        assert_eq!(ed.buffer.line(0), Some("let x = 5;"));
        ed.redo();
        assert_eq!(ed.buffer.line(0), Some("// let x = 5;"));
    }

    #[test]
    fn toggle_line_comment_no_op_for_unknown_extension() {
        let mut ed = editor_with_ext("<html></html>", "html");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.toggle_line_comment();
        assert_eq!(ed.buffer.line(0), Some("<html></html>"));
    }

    // ── move_cursor_word ──

    #[test]
    fn move_cursor_word_right_jumps_to_next_word() {
        let mut ed = editor_with("hello world");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.move_cursor_word(Direction::Right, false);
        assert_eq!(ed.cursors.primary().position.col, 6);
    }

    // ── add_cursor_above / add_cursor_below ──

    #[test]
    fn add_cursor_above_creates_cursor_on_previous_line_same_col() {
        let mut ed = editor_with("hello\nworld\nfoobar");
        ed.cursors.primary_mut().position = Position { line: 1, col: 3 };
        ed.cursors.primary_mut().desired_col = 3;

        ed.add_cursor_above();

        assert_eq!(ed.cursors.cursors.len(), 2);
        // Después de sort_by_position, el cursor en línea 0 debe estar primero
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 0, col: 3 });
        assert_eq!(ed.cursors.cursors[1].position, Position { line: 1, col: 3 });
    }

    #[test]
    fn add_cursor_below_creates_cursor_on_next_line_same_col() {
        let mut ed = editor_with("hello\nworld\nfoobar");
        ed.cursors.primary_mut().position = Position { line: 1, col: 3 };
        ed.cursors.primary_mut().desired_col = 3;

        ed.add_cursor_below();

        assert_eq!(ed.cursors.cursors.len(), 2);
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 1, col: 3 });
        assert_eq!(ed.cursors.cursors[1].position, Position { line: 2, col: 3 });
    }

    #[test]
    fn add_cursor_above_on_line_zero_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 2 };

        ed.add_cursor_above();

        assert_eq!(ed.cursors.cursors.len(), 1);
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 0, col: 2 });
    }

    #[test]
    fn add_cursor_below_on_last_line_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 1, col: 2 };

        ed.add_cursor_below();

        assert_eq!(ed.cursors.cursors.len(), 1);
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 1, col: 2 });
    }

    #[test]
    fn add_cursor_above_clamps_col_to_target_line_length() {
        // Línea 0 = "hi" (len 2), línea 1 = "hello world" (col 10)
        let mut ed = editor_with("hi\nhello world");
        ed.cursors.primary_mut().position = Position { line: 1, col: 10 };
        ed.cursors.primary_mut().desired_col = 10;

        ed.add_cursor_above();

        assert_eq!(ed.cursors.cursors.len(), 2);
        // Cursor nuevo en línea 0 con col clampeado a 2
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 0, col: 2 });
        // Primario en su posición original
        assert_eq!(ed.cursors.cursors[1].position, Position { line: 1, col: 10 });
    }

    #[test]
    fn add_cursor_below_clamps_col_to_target_line_length() {
        // Línea 0 = "hello world", línea 1 = "hi"
        let mut ed = editor_with("hello world\nhi");
        ed.cursors.primary_mut().position = Position { line: 0, col: 10 };
        ed.cursors.primary_mut().desired_col = 10;

        ed.add_cursor_below();

        assert_eq!(ed.cursors.cursors.len(), 2);
        assert_eq!(ed.cursors.cursors[0].position, Position { line: 0, col: 10 });
        assert_eq!(ed.cursors.cursors[1].position, Position { line: 1, col: 2 });
    }

    #[test]
    fn add_cursor_above_no_duplicate_at_existing_position() {
        // Si ya existe un cursor en la posición target, no duplicar.
        let mut ed = editor_with("aaa\nbbb\nccc");
        ed.cursors.primary_mut().position = Position { line: 1, col: 1 };
        ed.cursors
            .add_cursor(Position { line: 0, col: 1 }, None);
        // Re-set primary explícitamente (add_cursor agrega al final pero primary_index = 0)
        ed.cursors.primary_index = ed
            .cursors
            .cursors
            .iter()
            .position(|c| c.position == Position { line: 1, col: 1 })
            .expect("primary cursor present");

        let count_before = ed.cursors.cursors.len();
        ed.add_cursor_above();
        // No se agrega cursor duplicado
        assert_eq!(ed.cursors.cursors.len(), count_before);
    }

    #[test]
    fn add_cursor_above_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 1, col: 2 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.add_cursor_above();

        assert_eq!(ed.cursors.cursors.len(), 1);
    }

    #[test]
    fn add_cursor_below_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 2 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.add_cursor_below();

        assert_eq!(ed.cursors.cursors.len(), 1);
    }

    // ── add_cursor_above/below cascade (Bug #1 regression) ──
    //
    // Antes: usaba self.cursors.primary().position como referencia, así que
    // presionar Ctrl+Alt+↑ tres veces siempre intentaba agregar el cursor en
    // la misma línea (primary_line - 1) → dedup descartaba todas las llamadas
    // posteriores y solo se agregaba 1 cursor.
    //
    // Fix: usar el cursor topmost (línea mínima) para `above` y bottommost
    // (línea máxima) para `below`. La columna ancla viene del `desired_col`
    // del primario para preservar la intención de columna a través de
    // adiciones en cascada.

    #[test]
    fn add_cursor_above_three_times_creates_three_cursors_above() {
        // Buffer con 6 líneas, primary en línea 5
        let mut ed = editor_with("l0\nl1\nl2\nl3\nl4\nl5");
        ed.cursors.primary_mut().position = Position { line: 5, col: 0 };
        ed.cursors.primary_mut().desired_col = 0;

        ed.add_cursor_above();
        ed.add_cursor_above();
        ed.add_cursor_above();

        // Esperamos 4 cursores totales (primary en 5 + nuevos en 4, 3, 2)
        assert_eq!(ed.cursors.cursors.len(), 4);
        // Tras sort_by_position, ordenados ascendente por línea
        let lines: Vec<usize> = ed.cursors.cursors.iter().map(|c| c.position.line).collect();
        assert_eq!(lines, vec![2, 3, 4, 5]);
    }

    #[test]
    fn add_cursor_below_three_times_creates_three_cursors_below() {
        // Buffer con 6 líneas, primary en línea 1
        let mut ed = editor_with("l0\nl1\nl2\nl3\nl4\nl5");
        ed.cursors.primary_mut().position = Position { line: 1, col: 0 };
        ed.cursors.primary_mut().desired_col = 0;

        ed.add_cursor_below();
        ed.add_cursor_below();
        ed.add_cursor_below();

        // Esperamos 4 cursores totales (primary en 1 + nuevos en 2, 3, 4)
        assert_eq!(ed.cursors.cursors.len(), 4);
        let lines: Vec<usize> = ed.cursors.cursors.iter().map(|c| c.position.line).collect();
        assert_eq!(lines, vec![1, 2, 3, 4]);
    }

    #[test]
    fn add_cursor_above_uses_anchor_col_not_topmost_col() {
        // Buffer con líneas de longitudes 2, 4, 5, 6.
        // Primary en (3, 5) con desired_col 5.
        //
        // Llamadas en cascada:
        //   1. topmost = línea 3, target = 2, col = min(5, 5) = 5  → cursor (2, 5)
        //   2. topmost = línea 2, target = 1, col = min(5, 4) = 4  → cursor (1, 4)
        //   3. topmost = línea 1, target = 0, col = min(5, 2) = 2  → cursor (0, 2)
        //
        // Esto verifica que el ancla de columna es el desired_col del primary
        // (5), NO el col del cursor topmost actual (que va cambiando).
        let mut ed = editor_with("ab\nabcd\nabcde\nabcdef");
        ed.cursors.primary_mut().position = Position { line: 3, col: 5 };
        ed.cursors.primary_mut().desired_col = 5;

        ed.add_cursor_above();
        ed.add_cursor_above();
        ed.add_cursor_above();

        assert_eq!(ed.cursors.cursors.len(), 4);
        // Tras sort_by_position, ordenados por (line, col) ascendente
        let positions: Vec<Position> =
            ed.cursors.cursors.iter().map(|c| c.position).collect();
        assert_eq!(
            positions,
            vec![
                Position { line: 0, col: 2 },
                Position { line: 1, col: 4 },
                Position { line: 2, col: 5 },
                Position { line: 3, col: 5 },
            ]
        );
    }

    // ── move_to_line_start_selecting / move_to_line_end_selecting ──

    #[test]
    fn move_to_line_start_selecting_creates_selection_to_col_zero() {
        let mut ed = editor_with("hello world");
        ed.cursors.primary_mut().position = Position { line: 0, col: 5 };
        ed.cursors.primary_mut().desired_col = 5;

        ed.move_to_line_start_selecting();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 0 });
        assert_eq!(primary.desired_col, 0);
        assert!(primary.has_selection());
        let sel = primary.selection.expect("selection present");
        assert_eq!(sel.anchor, Position { line: 0, col: 5 });
        assert_eq!(sel.head, Position { line: 0, col: 0 });
    }

    #[test]
    fn move_to_line_end_selecting_creates_selection_to_line_end() {
        let mut ed = editor_with("hello world");
        ed.cursors.primary_mut().position = Position { line: 0, col: 3 };
        ed.cursors.primary_mut().desired_col = 3;

        ed.move_to_line_end_selecting();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 11 });
        assert!(primary.has_selection());
        let sel = primary.selection.expect("selection present");
        assert_eq!(sel.anchor, Position { line: 0, col: 3 });
        assert_eq!(sel.head, Position { line: 0, col: 11 });
    }

    #[test]
    fn move_to_line_start_selecting_applies_to_all_cursors() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 4 };
        ed.cursors.primary_mut().desired_col = 4;
        ed.cursors
            .add_cursor(Position { line: 1, col: 3 }, None);

        ed.move_to_line_start_selecting();

        assert_eq!(ed.cursors.cursors.len(), 2);
        for cursor in &ed.cursors.cursors {
            assert_eq!(cursor.position.col, 0);
            assert!(cursor.has_selection());
        }
    }

    #[test]
    fn move_to_line_end_selecting_applies_to_all_cursors() {
        let mut ed = editor_with("hello\nfoo");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.cursors
            .add_cursor(Position { line: 1, col: 0 }, None);

        ed.move_to_line_end_selecting();

        assert_eq!(ed.cursors.cursors.len(), 2);
        // Cursor en línea 0 → col 5 (len de "hello")
        let c0 = ed
            .cursors
            .cursors
            .iter()
            .find(|c| c.position.line == 0)
            .expect("line 0 cursor");
        assert_eq!(c0.position.col, 5);
        assert!(c0.has_selection());
        // Cursor en línea 1 → col 3 (len de "foo")
        let c1 = ed
            .cursors
            .cursors
            .iter()
            .find(|c| c.position.line == 1)
            .expect("line 1 cursor");
        assert_eq!(c1.position.col, 3);
        assert!(c1.has_selection());
    }

    #[test]
    fn move_to_line_start_selecting_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello world");
        ed.cursors.primary_mut().position = Position { line: 0, col: 5 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.move_to_line_start_selecting();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 5 });
        assert!(!primary.has_selection());
    }

    // ── move_to_buffer_start_selecting / move_to_buffer_end_selecting ──

    #[test]
    fn move_to_buffer_start_selecting_extends_to_origin() {
        let mut ed = editor_with("hello\nworld\nfoo");
        ed.cursors.primary_mut().position = Position { line: 2, col: 2 };
        ed.cursors.primary_mut().desired_col = 2;

        ed.move_to_buffer_start_selecting();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 0 });
        assert!(primary.has_selection());
        let sel = primary.selection.expect("selection present");
        assert_eq!(sel.anchor, Position { line: 2, col: 2 });
        assert_eq!(sel.head, Position { line: 0, col: 0 });
    }

    #[test]
    fn move_to_buffer_end_selecting_extends_to_last_position() {
        let mut ed = editor_with("hello\nworld\nfoo");
        ed.cursors.primary_mut().position = Position { line: 0, col: 1 };
        ed.cursors.primary_mut().desired_col = 1;

        ed.move_to_buffer_end_selecting();

        let primary = ed.cursors.primary();
        // Última línea = 2, último col = 3 (len de "foo")
        assert_eq!(primary.position, Position { line: 2, col: 3 });
        assert!(primary.has_selection());
        let sel = primary.selection.expect("selection present");
        assert_eq!(sel.anchor, Position { line: 0, col: 1 });
        assert_eq!(sel.head, Position { line: 2, col: 3 });
    }

    #[test]
    fn move_to_buffer_start_selecting_applies_to_all_cursors() {
        let mut ed = editor_with("hello\nworld\nfoo");
        ed.cursors.primary_mut().position = Position { line: 1, col: 2 };
        ed.cursors
            .add_cursor(Position { line: 2, col: 1 }, None);

        ed.move_to_buffer_start_selecting();

        assert_eq!(ed.cursors.cursors.len(), 2);
        for cursor in &ed.cursors.cursors {
            assert_eq!(cursor.position, Position { line: 0, col: 0 });
            assert!(cursor.has_selection());
        }
    }

    #[test]
    fn move_to_buffer_end_selecting_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.move_to_buffer_end_selecting();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 0 });
        assert!(!primary.has_selection());
    }

    // ── select_line ──

    #[test]
    fn select_line_selects_full_line_for_single_cursor() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 2 };

        ed.select_line();

        let primary = ed.cursors.primary();
        let sel = primary.selection.expect("selection MUST exist");
        assert_eq!(sel.start(), Position { line: 0, col: 0 });
        assert_eq!(sel.end(), Position { line: 0, col: 5 });
        assert_eq!(primary.position, Position { line: 0, col: 5 });
        assert_eq!(primary.desired_col, 5);
    }

    #[test]
    fn select_line_handles_empty_line() {
        // Triangulación: línea vacía → start == end pero la selection existe (puede estar empty)
        let mut ed = editor_with("\nfoo");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };

        ed.select_line();

        let primary = ed.cursors.primary();
        let sel = primary.selection.expect("selection MUST exist even for empty line");
        assert_eq!(sel.start(), Position { line: 0, col: 0 });
        assert_eq!(sel.end(), Position { line: 0, col: 0 });
        assert_eq!(primary.position, Position { line: 0, col: 0 });
    }

    #[test]
    fn select_line_works_with_multi_cursor() {
        let mut ed = editor_with("aaa\nbbbbb\ncc");
        ed.cursors.primary_mut().position = Position { line: 0, col: 1 };
        ed.cursors
            .add_cursor(Position { line: 2, col: 1 }, None);

        ed.select_line();

        // Both cursors should have full-line selections
        assert_eq!(ed.cursors.cursors.len(), 2);
        for cursor in &ed.cursors.cursors {
            let sel = cursor.selection.expect("each cursor MUST have a selection");
            assert_eq!(sel.start().col, 0);
            assert_eq!(sel.end().line, sel.start().line);
        }
        // Triangulación: cada línea tiene su propio largo distinto
        let line0_sel = ed.cursors.cursors[0].selection.unwrap();
        let line2_sel = ed.cursors.cursors[1].selection.unwrap();
        assert_eq!(line0_sel.end(), Position { line: 0, col: 3 });
        assert_eq!(line2_sel.end(), Position { line: 2, col: 2 });
    }

    #[test]
    fn select_line_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 2 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.select_line();

        let primary = ed.cursors.primary();
        assert_eq!(primary.position, Position { line: 0, col: 2 });
        assert!(primary.selection.is_none());
    }

    // ── duplicate_line ──

    #[test]
    fn duplicate_line_down_inserts_copy_below_and_moves_cursor() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 1, col: 0 };

        ed.duplicate_line(Direction::Down);

        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
        assert_eq!(ed.buffer.line(2), Some("b"));
        assert_eq!(ed.buffer.line(3), Some("c"));
        assert_eq!(ed.cursors.primary().position.line, 2);
    }

    #[test]
    fn duplicate_line_up_inserts_copy_above_and_keeps_cursor() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 1, col: 0 };

        ed.duplicate_line(Direction::Up);

        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
        assert_eq!(ed.buffer.line(2), Some("b"));
        assert_eq!(ed.cursors.primary().position.line, 1);
    }

    #[test]
    fn duplicate_line_down_undo_round_trip() {
        let mut ed = editor_with("a\nb");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };

        ed.duplicate_line(Direction::Down);
        assert_eq!(ed.buffer.line_count(), 3);
        assert_eq!(ed.buffer.line(1), Some("a"));

        ed.undo();
        assert_eq!(ed.buffer.line_count(), 2);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));

        ed.redo();
        assert_eq!(ed.buffer.line_count(), 3);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("a"));
    }

    #[test]
    fn duplicate_line_in_diff_tab_is_noop() {
        let mut ed = editor_with("hello\nworld");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.duplicate_line(Direction::Down);

        assert_eq!(ed.buffer.line_count(), 2);
        assert_eq!(ed.buffer.line(0), Some("hello"));
        assert_eq!(ed.buffer.line(1), Some("world"));
    }

    #[test]
    fn duplicate_line_multi_cursor_dedups_and_processes_bottom_up() {
        // Triangulación clave: dos cursores en líneas distintas — ambas deben
        // duplicarse exactamente una vez. El procesamiento bottom-up evita
        // que la inserción en línea 0 corra el índice 2 antes de procesarla.
        let mut ed = editor_with("a\nb\nc");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.cursors
            .add_cursor(Position { line: 2, col: 0 }, None);

        ed.duplicate_line(Direction::Down);

        // Esperado: a, a, b, c, c
        assert_eq!(ed.buffer.line_count(), 5);
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("a"));
        assert_eq!(ed.buffer.line(2), Some("b"));
        assert_eq!(ed.buffer.line(3), Some("c"));
        assert_eq!(ed.buffer.line(4), Some("c"));
    }

    // ── indent_selection / unindent_selection ──

    #[test]
    fn indent_selection_prepends_four_spaces_to_each_line() {
        let mut ed = editor_with("foo\nbar\nbaz");
        ed.cursors.primary_mut().position = Position { line: 0, col: 0 };
        ed.cursors.primary_mut().selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 1, col: 3 },
        ));

        ed.indent_selection();

        assert_eq!(ed.buffer.line(0), Some("    foo"));
        assert_eq!(ed.buffer.line(1), Some("    bar"));
        // baz NO está dentro de la selección
        assert_eq!(ed.buffer.line(2), Some("baz"));
    }

    #[test]
    fn indent_selection_undo_round_trip() {
        let mut ed = editor_with("foo\nbar");
        ed.cursors.primary_mut().selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 1, col: 3 },
        ));

        ed.indent_selection();
        assert_eq!(ed.buffer.line(0), Some("    foo"));
        ed.undo();
        // Undo debe restaurar al menos la primera línea procesada
        assert_eq!(ed.buffer.line(1), Some("bar"));
        ed.undo();
        assert_eq!(ed.buffer.line(0), Some("foo"));
    }

    #[test]
    fn unindent_selection_removes_up_to_four_leading_spaces() {
        let mut ed = editor_with("    foo\n  bar\nbaz");
        ed.cursors.primary_mut().selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 2, col: 3 },
        ));

        ed.unindent_selection();

        // Triangulación: distintos largos de leading whitespace
        assert_eq!(ed.buffer.line(0), Some("foo"));     // tenía 4 → quita 4
        assert_eq!(ed.buffer.line(1), Some("bar"));     // tenía 2 → quita 2
        assert_eq!(ed.buffer.line(2), Some("baz"));     // tenía 0 → no-op
    }

    #[test]
    fn unindent_selection_does_not_remove_non_space_chars() {
        let mut ed = editor_with("\t\tfoo");
        ed.cursors.primary_mut().selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 0, col: 1 },
        ));

        ed.unindent_selection();

        // Tabs NO son spaces — no se tocan.
        assert_eq!(ed.buffer.line(0), Some("\t\tfoo"));
    }

    #[test]
    fn indent_selection_in_diff_tab_is_noop() {
        let mut ed = editor_with("foo\nbar");
        ed.cursors.primary_mut().selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 1, col: 3 },
        ));
        ed.diff_view = Some(DiffViewContent {
            content: String::new(),
            file_path: None,
            is_file_content: false,
            scroll_offset: 0,
        });

        ed.indent_selection();

        assert_eq!(ed.buffer.line(0), Some("foo"));
        assert_eq!(ed.buffer.line(1), Some("bar"));
    }

    // ── Unicode + Atomic Paste integration ──

    /// Cuenta entradas en el undo_stack — para validar atomicidad.
    /// Usamos una pop+push para no exponer la longitud directa del stack.
    fn undo_count(ed: &mut EditorState) -> usize {
        let mut count = 0;
        let mut popped = Vec::new();
        while let Some(op) = ed.undo_stack.undo() {
            popped.push(op);
            count += 1;
        }
        // Restaurar todo via redo
        for _ in 0..popped.len() {
            ed.undo_stack.redo();
        }
        count
    }

    #[test]
    fn paste_unicode_text_no_panic() {
        let mut ed = editor_with("");
        ed.insert_str("ñoño\ncódigo");
        // Verificar que se insertó correctamente
        assert_eq!(ed.buffer.line(0), Some("ñoño"));
        assert_eq!(ed.buffer.line(1), Some("código"));
        // Validar atomicidad: 1 sola entrada InsertText (no N por char).
        // El editor inicial tiene 1 cursor sin selección → 1 entrada.
        assert_eq!(undo_count(&mut ed), 1);
    }

    #[test]
    fn type_accented_chars_no_panic() {
        let mut ed = editor_with("");
        ed.insert_char('ó');
        // 'ó' = 2 bytes → cursor.col debe ser 2.
        assert_eq!(ed.cursors.primary().position, Position { line: 0, col: 2 });
        ed.insert_char('a');
        // 'a' = 1 byte → cursor.col debe ser 3.
        assert_eq!(ed.cursors.primary().position, Position { line: 0, col: 3 });
        assert_eq!(ed.buffer.line(0), Some("óa"));
    }

    #[test]
    fn select_unicode_text_and_replace() {
        // Construir buffer con "héllo", seleccionar todo y reemplazar con "ñ".
        let mut ed = editor_with("héllo");
        // h(1) é(2) l(1) l(1) o(1) = 6 bytes
        let line_len = ed.buffer.line_len(0);
        assert_eq!(line_len, 6);

        let primary = ed.cursors.primary_mut();
        primary.position = Position { line: 0, col: line_len };
        primary.selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 0, col: line_len },
        ));

        ed.insert_str("ñ");
        assert_eq!(ed.buffer.line(0), Some("ñ"));
        // ñ = 2 bytes
        assert_eq!(ed.cursors.primary().position, Position { line: 0, col: 2 });
    }

    #[test]
    fn multicursor_with_unicode() {
        // Texto con multi-byte; agregar dos cursores e insertar 'X' en cada uno.
        // "héllo" — line 0
        // Con dos cursores: uno en col 0, otro en col 1 (después de 'h').
        let mut ed = editor_with("héllo");
        // Primary en col 0 (default)
        ed.cursors.add_cursor(Position { line: 0, col: 1 }, None);
        ed.insert_char('X');
        // Resultado esperado:
        //   - cursor 1 inserta X en col 1: "hX..." → no — primero ordena, itera reverso
        //   - cursor 0 inserta X en col 0: "Xh..."
        // Después del orden + reverse iteration:
        //   - itera col=1 primero: "hXéllo" cursor a col=2
        //   - itera col=0 después: "XhXéllo" cursor a col=1
        // Ambos cursores quedan en boundaries válidos.
        let line = ed.buffer.line(0).unwrap();
        // Validar invariante crítico: ningún cursor cae en medio de multi-byte.
        for c in &ed.cursors.cursors {
            assert!(line.is_char_boundary(c.position.col), "cursor col {} not on boundary in {:?}", c.position.col, line);
        }
        assert_eq!(line, "XhXéllo");
    }

    #[test]
    fn move_word_through_spanish_text_no_panic() {
        // Test integración: navegar word-by-word por texto con multi-byte
        // — verifica que no panica y siempre cae en boundaries.
        let mut ed = editor_with("año pasó rápido");
        let line_len = ed.buffer.line_len(0);
        // Mover word right repetidamente.
        for _ in 0..10 {
            ed.move_cursor_word(Direction::Right, false);
        }
        let line = ed.buffer.line(0).unwrap();
        let pos = ed.cursors.primary().position;
        assert!(line.is_char_boundary(pos.col));
        assert!(pos.col <= line_len);
    }

    #[test]
    fn backspace_through_emoji() {
        // "😀" — 4 bytes. Cursor al final, backspace borra los 4 bytes (1 char).
        let mut ed = editor_with("😀");
        let primary = ed.cursors.primary_mut();
        primary.position = Position { line: 0, col: 4 };

        ed.delete_char();
        assert_eq!(ed.buffer.line(0), Some(""));
        assert_eq!(ed.cursors.primary().position, Position { line: 0, col: 0 });
    }

    #[test]
    fn undo_redo_unicode_paste() {
        let mut ed = editor_with("");
        ed.insert_str("ñoño\ncódigo");
        assert_eq!(ed.buffer.line(0), Some("ñoño"));
        assert_eq!(ed.buffer.line(1), Some("código"));

        ed.undo();
        // Tras undo, buffer vuelve a estado inicial (1 línea vacía).
        assert_eq!(ed.buffer.line_count(), 1);
        assert_eq!(ed.buffer.line(0), Some(""));

        ed.redo();
        // Tras redo, vuelve el contenido pegado.
        assert_eq!(ed.buffer.line(0), Some("ñoño"));
        assert_eq!(ed.buffer.line(1), Some("código"));
    }

    #[test]
    fn paste_single_line_creates_one_undo_entry() {
        let mut ed = editor_with("");
        ed.insert_str("hello world");
        assert_eq!(ed.buffer.line(0), Some("hello world"));
        // Sin selección → solo 1 InsertText.
        assert_eq!(undo_count(&mut ed), 1);
    }

    #[test]
    fn paste_over_selection_creates_two_undo_entries() {
        // Selección activa + paste → 1 DeleteRange + 1 InsertText = 2 entries.
        let mut ed = editor_with("hello");
        let primary = ed.cursors.primary_mut();
        primary.position = Position { line: 0, col: 5 };
        primary.selection = Some(Selection::new(
            Position { line: 0, col: 0 },
            Position { line: 0, col: 5 },
        ));
        ed.insert_str("world");
        assert_eq!(ed.buffer.line(0), Some("world"));
        assert_eq!(undo_count(&mut ed), 2);
    }

    #[test]
    fn crlf_paste_normalizes_to_lf() {
        let mut ed = editor_with("");
        ed.insert_str("a\r\nb\r\nc");
        assert_eq!(ed.buffer.line(0), Some("a"));
        assert_eq!(ed.buffer.line(1), Some("b"));
        assert_eq!(ed.buffer.line(2), Some("c"));
    }
}
