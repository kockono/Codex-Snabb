//! Explorer: estado del panel de explorador de archivos.
//!
//! Mantiene el árbol de archivos, selección actual, y scroll.
//! Aplana el árbol para renderizado eficiente con viewport virtual.
//! Solo las entries visibles se renderizan — nunca el árbol completo.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::tree::{load_directory, FileEntry};

/// Entry aplanado para renderizado — una fila visible del explorer.
///
/// Contiene solo los datos necesarios para dibujar: profundidad,
/// nombre, tipo, estado de expansión. El path se incluye para
/// abrir archivos al hacer Enter.
#[derive(Debug)]
pub struct FlatEntry {
    /// Nivel de indentación (0 = raíz).
    pub depth: usize,
    /// Nombre del archivo/directorio (solo componente final).
    pub name: String,
    /// Si es un directorio.
    pub is_dir: bool,
    /// Si el directorio está expandido (false para archivos).
    pub expanded: bool,
    /// Path absoluto.
    pub path: PathBuf,
}

/// Estado del panel explorador de archivos.
///
/// Administra el árbol lazy, la selección actual, y el scroll
/// para viewport virtual. Las operaciones de navegación mantienen
/// la selección dentro de bounds válidos.
///
/// El árbol aplanado se cachea para evitar recomputar en cada frame.
/// Se invalida (`cache_dirty = true`) en operaciones que mutan el árbol.
#[derive(Debug)]
pub struct ExplorerState {
    /// Directorio raíz del workspace.
    pub root: PathBuf,
    /// Entries del directorio raíz (hijos directos).
    pub entries: Vec<FileEntry>,
    /// Índice seleccionado en la vista aplanada.
    pub selected_index: usize,
    /// Offset de scroll para viewport virtual.
    pub scroll_offset: usize,
    /// Cache del árbol aplanado — evita recomputar cada frame.
    flat_cache: Option<Vec<FlatEntry>>,
    /// Flag de invalidación — `true` cuando el árbol mutó y el cache es stale.
    cache_dirty: bool,
}

impl ExplorerState {
    /// Crea un explorer cargando el directorio raíz.
    ///
    /// Solo carga el primer nivel (lazy). Subdirectorios se cargan
    /// al expandir.
    pub fn new(root: &Path) -> Result<Self> {
        let entries = load_directory(root)
            .with_context(|| format!("no se pudo cargar explorer en: {}", root.display()))?;
        Ok(Self {
            // CLONE: necesario — el explorer necesita ownership del path raíz
            root: root.to_path_buf(),
            entries,
            selected_index: 0,
            scroll_offset: 0,
            flat_cache: None,
            cache_dirty: true,
        })
    }

    /// Aplana el árbol recursivamente para renderizado (sin cache).
    ///
    /// Solo incluye entries visibles (dirs expandidos y sus hijos).
    /// Retorna un Vec de `FlatEntry` con profundidad para indentación.
    /// Usado internamente cuando se necesita un Vec owned temporal.
    pub fn flatten(&self) -> Vec<FlatEntry> {
        let mut result = Vec::new();
        flatten_recursive(&self.entries, 0, &mut result);
        result
    }

    /// Retorna el árbol aplanado cacheado — recomputa solo si dirty.
    ///
    /// Preferir este método sobre `flatten()` en render loops y cualquier
    /// hot path. Evita re-alocar el Vec cada frame cuando el árbol no cambió.
    pub fn ensure_flat_cache(&mut self) -> &[FlatEntry] {
        if self.cache_dirty || self.flat_cache.is_none() {
            let mut result = Vec::new();
            flatten_recursive(&self.entries, 0, &mut result);
            self.flat_cache = Some(result);
            self.cache_dirty = false;
        }
        // SAFETY: siempre Some después del bloque de arriba
        self.flat_cache.as_deref().unwrap_or(&[])
    }

    /// Retorna el flat cache si está disponible y no dirty.
    ///
    /// Usado en render (donde solo se tiene `&self`). Si el cache está
    /// actualizado (fue llenado por `ensure_flat_cache` antes del render),
    /// retorna el slice cacheado. Si no, retorna `None` y el caller
    /// debe usar `flatten()` como fallback.
    pub fn cached_flat(&self) -> Option<&[FlatEntry]> {
        if !self.cache_dirty {
            self.flat_cache.as_deref()
        } else {
            None
        }
    }

    /// Invalida el flat cache — debe llamarse después de mutar el árbol.
    fn invalidate_cache(&mut self) {
        self.cache_dirty = true;
    }

    /// Mover selección arriba.
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Mover selección abajo.
    pub fn move_down(&mut self) {
        let flat_count = self.ensure_flat_cache().len();
        if flat_count > 0 && self.selected_index < flat_count - 1 {
            self.selected_index += 1;
        }
    }

    /// Toggle expand/collapse del entry seleccionado.
    ///
    /// Si es directorio: alterna expansión. Si es archivo: no-op (el
    /// caller decide si abrir el archivo).
    /// Retorna `true` si el entry seleccionado es un archivo (señal
    /// para que el caller abra el archivo).
    pub fn toggle_selected(&mut self) -> Result<bool> {
        let flat = self.flatten();
        let Some(selected) = flat.get(self.selected_index) else {
            return Ok(false);
        };

        if !selected.is_dir {
            return Ok(true); // Es archivo — el caller decide qué hacer
        }

        // CLONE: necesario — necesitamos el path para buscar en el árbol
        // después de soltar el borrow inmutable de `flat`
        let target_path = selected.path.clone();
        drop(flat);

        // Buscar y toggle el directorio en el árbol
        toggle_entry_by_path(&mut self.entries, &target_path)?;
        self.invalidate_cache();

        Ok(false)
    }

    /// Path del entry seleccionado actualmente, si existe.
    pub fn selected_path(&self) -> Option<PathBuf> {
        let flat = self.flatten();
        // CLONE: necesario — el path se retorna como owned porque el FlatEntry
        // se destruye al salir de flatten()
        flat.get(self.selected_index).map(|e| e.path.clone())
    }

    /// Si el entry seleccionado es un archivo.
    #[expect(
        dead_code,
        reason = "se usará desde command palette o keybindings futuros"
    )]
    pub fn selected_is_file(&self) -> bool {
        let flat = self.flatten();
        flat.get(self.selected_index).is_some_and(|e| !e.is_dir)
    }

    /// Recarga el árbol desde disco.
    ///
    /// Preserva la selección si el índice sigue siendo válido.
    /// Si no, clampea al último entry visible.
    pub fn refresh(&mut self) -> Result<()> {
        self.entries = load_directory(&self.root)
            .with_context(|| format!("no se pudo refrescar: {}", self.root.display()))?;
        self.invalidate_cache();
        // Clampear selección — usar ensure_flat_cache para evitar double-compute
        let flat_count = self.ensure_flat_cache().len();
        if flat_count == 0 {
            self.selected_index = 0;
        } else if self.selected_index >= flat_count {
            self.selected_index = flat_count - 1;
        }
        Ok(())
    }

    /// Ajusta el scroll para mantener la selección visible.
    ///
    /// Si la selección está arriba del viewport, scroll up.
    /// Si está abajo, scroll down.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
        if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }

    /// Collapse el directorio seleccionado (si está expandido).
    ///
    /// Retorna `true` si se colapsó algo, `false` si no era directorio
    /// expandido.
    pub fn collapse_selected(&mut self) -> Result<bool> {
        let flat = self.flatten();
        let Some(selected) = flat.get(self.selected_index) else {
            return Ok(false);
        };

        if !selected.is_dir || !selected.expanded {
            return Ok(false);
        }

        // CLONE: necesario — mismo razonamiento que toggle_selected
        let target_path = selected.path.clone();
        drop(flat);

        collapse_entry_by_path(&mut self.entries, &target_path)?;
        self.invalidate_cache();
        Ok(true)
    }
}

/// Aplana el árbol recursivamente.
///
/// Solo desciende en directorios expandidos y cargados.
fn flatten_recursive(entries: &[FileEntry], depth: usize, result: &mut Vec<FlatEntry>) {
    for entry in entries {
        match entry {
            FileEntry::File { name, path } => {
                result.push(FlatEntry {
                    depth,
                    // CLONE: necesario — FlatEntry necesita ownership para el render
                    name: name.clone(),
                    is_dir: false,
                    expanded: false,
                    path: path.clone(),
                });
            }
            FileEntry::Directory {
                name,
                path,
                children,
                expanded,
                ..
            } => {
                result.push(FlatEntry {
                    depth,
                    // CLONE: necesario — FlatEntry necesita ownership para el render
                    name: name.clone(),
                    is_dir: true,
                    expanded: *expanded,
                    path: path.clone(),
                });
                if *expanded {
                    flatten_recursive(children, depth + 1, result);
                }
            }
        }
    }
}

/// Busca un entry por path en el árbol y hace toggle de expand.
fn toggle_entry_by_path(entries: &mut [FileEntry], target: &Path) -> Result<bool> {
    for entry in entries.iter_mut() {
        if let FileEntry::Directory { path, children, .. } = entry {
            if *path == target {
                entry.toggle_expand()?;
                return Ok(true);
            }
            // Buscar recursivamente en hijos
            if toggle_entry_by_path(children, target)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Busca un entry por path en el árbol y colapsa (sin toggle).
fn collapse_entry_by_path(entries: &mut [FileEntry], target: &Path) -> Result<bool> {
    for entry in entries.iter_mut() {
        if let FileEntry::Directory {
            path,
            expanded,
            children,
            ..
        } = entry
        {
            if *path == target {
                *expanded = false;
                return Ok(true);
            }
            if collapse_entry_by_path(children, target)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
