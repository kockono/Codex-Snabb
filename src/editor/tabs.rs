//! Tabs: gestión de múltiples buffers abiertos con pestañas.
//!
//! `TabState` mantiene un Vec de `EditorState` y un índice activo.
//! Provee operaciones de navegación (next/prev/switch), apertura y cierre.
//! `TabInfo` es un DTO ligero para renderizado — sin allocaciones de heap
//! innecesarias en el path del render.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{DiffViewContent, EditorState};

/// Info ligera de una tab para renderizado.
///
/// Se construye fuera del render loop y se pasa por referencia.
/// `name` es solo el filename (no el path completo) para minimizar
/// espacio visual en la barra de tabs.
#[derive(Debug)]
pub struct TabInfo {
    /// Nombre del archivo (solo filename, no path completo).
    pub name: String,
    /// Si esta tab es la activa.
    pub is_active: bool,
    /// Si el buffer fue modificado (dirty).
    pub is_dirty: bool,
    /// Path completo del archivo, si existe.
    #[expect(
        dead_code,
        reason = "se usará para tooltips de tabs y acciones contextuales"
    )]
    pub path: Option<PathBuf>,
}

/// Estado de múltiples tabs/buffers abiertos.
///
/// Siempre tiene al menos un editor (invariante). Si se cierra la última
/// tab, se reemplaza con un editor vacío. El `active_index` siempre
/// apunta a un editor válido.
#[derive(Debug)]
pub struct TabState {
    /// Todos los buffers abiertos.
    editors: Vec<EditorState>,
    /// Índice de la tab activa (siempre < editors.len()).
    active_index: usize,
}

impl TabState {
    /// Crea un TabState con un solo editor vacío.
    pub fn new() -> Self {
        Self {
            editors: vec![EditorState::new()],
            active_index: 0,
        }
    }

    /// Crea un TabState con un editor que tiene un archivo abierto.
    pub fn with_editor(editor: EditorState) -> Self {
        Self {
            editors: vec![editor],
            active_index: 0,
        }
    }

    /// Referencia al editor activo.
    pub fn active(&self) -> &EditorState {
        &self.editors[self.active_index]
    }

    /// Referencia mutable al editor activo.
    pub fn active_mut(&mut self) -> &mut EditorState {
        &mut self.editors[self.active_index]
    }

    /// Abre un archivo en una tab.
    ///
    /// Si el archivo ya está abierto en alguna tab, cambia a esa tab
    /// en vez de abrir una nueva (evita duplicados). Si no, crea un
    /// nuevo `EditorState` y lo agrega al final.
    pub fn open_file(&mut self, path: &Path) -> Result<()> {
        // Buscar si el archivo ya está abierto
        for (i, editor) in self.editors.iter().enumerate() {
            if let Some(existing_path) = editor.buffer.file_path()
                && existing_path == path
            {
                // Ya abierto — solo cambiar a esa tab
                self.active_index = i;
                return Ok(());
            }
        }

        // No está abierto — crear nuevo editor
        let editor = EditorState::open_file(path)?;
        self.editors.push(editor);
        self.active_index = self.editors.len() - 1;
        Ok(())
    }

    /// Abre (o reutiliza) una tab virtual de diff/file para el archivo dado.
    ///
    /// Si ya existe una tab de diff para `file_path`, la activa y actualiza
    /// su contenido y el flag `is_file_content`. Si no existe, crea un
    /// `EditorState` nuevo con `diff_view` poblado.
    ///
    /// Retorna el índice de la tab abierta/reusada.
    pub fn open_diff_tab(
        &mut self,
        content: String,
        file_path: Option<PathBuf>,
        is_file_content: bool,
    ) -> usize {
        // Buscar tab de diff existente para el mismo archivo
        for (i, editor) in self.editors.iter().enumerate() {
            if let Some(ref dv) = editor.diff_view
                && dv.file_path == file_path
            {
                // Reusar — activar la tab existente. El contenido se actualiza
                // a continuación vía update_active_diff() (el caller lo hace).
                self.active_index = i;
                return i;
            }
        }

        // No existe — crear nueva tab con buffer vacío y diff_view poblado
        let mut editor = EditorState::new();
        editor.diff_view = Some(DiffViewContent {
            content,
            file_path,
            is_file_content,
            scroll_offset: 0,
        });
        self.editors.push(editor);
        self.active_index = self.editors.len() - 1;
        self.active_index
    }

    /// Actualiza el contenido de la tab de diff activa (si lo es).
    ///
    /// Si la tab activa no es una vista de diff, no hace nada.
    /// Resetea `scroll_offset` a 0 (nuevo contenido = arriba de todo).
    pub fn update_active_diff(&mut self, content: String, is_file_content: bool) {
        if let Some(ref mut dv) = self.editors[self.active_index].diff_view {
            dv.content = content;
            dv.is_file_content = is_file_content;
            dv.scroll_offset = 0;
        }
    }

    /// Retorna `true` si la tab activa es una tab virtual de diff.
    pub fn active_is_diff(&self) -> bool {
        self.editors
            .get(self.active_index)
            .is_some_and(|e| e.diff_view.is_some())
    }

    /// Retorna `true` si la tab activa es una vista de imagen.
    ///
    /// Sigue el mismo patrón que `active_is_diff()` para que el reducer
    /// pueda ignorar acciones de edición sobre tabs de imagen (read-only).
    ///
    /// Considera dos casos:
    /// 1. `image_view = Some(...)`: imagen ya cargada — caso obvio.
    /// 2. `image_view = None` + buffer vacío + file_path es de imagen: estamos
    ///    en la ventana de carga async (placeholder creado por `open_image_tab`).
    ///    Sin este check, la tab respondería a input de teclado durante la carga.
    pub fn active_is_image(&self) -> bool {
        let Some(editor) = self.editors.get(self.active_index) else {
            return false;
        };
        if editor.image_view.is_some() {
            return true;
        }
        // Placeholder de carga: buffer vacío, sin diff, y path es imagen.
        if editor.diff_view.is_none()
            && editor.buffer.line_count() <= 1
            && editor.buffer.line_len(0) == 0
            && let Some(path) = editor.buffer.file_path()
            && crate::core::is_image_file(path)
        {
            return true;
        }
        false
    }

    /// Abre una tab placeholder para una imagen mientras se decodifica async.
    ///
    /// El buffer queda vacío y `image_view` se setea como `None` — el reducer
    /// lo poblará cuando reciba `Event::ImageLoaded` (o `Event::ImageLoadError`)
    /// del worker de decodificación.
    ///
    /// Dedup: si ya existe una tab con la misma `path` (texto O imagen), la
    /// reusa en vez de crear una nueva. Esto evita abrir N copias de la misma
    /// imagen al click rápido en el explorer.
    ///
    /// Retorna el índice de la tab abierta/reusada.
    pub fn open_image_tab(&mut self, path: &Path) -> usize {
        // Buscar tab existente con el mismo path. Para tabs de imagen
        // miramos `image_view.path`; para placeholders mientras carga
        // miramos `buffer.file_path()`.
        for (i, editor) in self.editors.iter().enumerate() {
            // Tab de imagen ya cargada
            if let Some(ref iv) = editor.image_view
                && iv.path == path
            {
                self.active_index = i;
                return i;
            }
            // Placeholder con file_path seteado (durante carga)
            if let Some(existing) = editor.buffer.file_path()
                && existing == path
                && editor.image_view.is_none()
                && editor.diff_view.is_none()
            {
                // Solo reusar si el buffer está vacío (es un placeholder de imagen)
                if editor.buffer.line_count() <= 1 && editor.buffer.line_len(0) == 0 {
                    self.active_index = i;
                    return i;
                }
            }
        }

        // Crear placeholder: EditorState vacío con el path guardado en el buffer
        // para que el tab name muestre el nombre del archivo durante la carga.
        let mut editor = EditorState::new();
        // Asociar el path al buffer (sin cargar contenido, ya que es una imagen)
        // CLONE: path se convierte a PathBuf — necesario para almacenar en buffer.
        editor.buffer.set_file_path(path.to_path_buf());
        self.editors.push(editor);
        self.active_index = self.editors.len() - 1;
        self.active_index
    }

    /// Setea el `image_view` de la tab `idx` (poblado tras decode async).
    ///
    /// No-op si el índice está fuera de rango. Limpia cualquier `diff_view`
    /// previo (no debería existir en una tab de imagen, pero por consistencia).
    pub fn set_image_content(&mut self, idx: usize, content: super::image::ImageViewContent) {
        if let Some(editor) = self.editors.get_mut(idx) {
            editor.diff_view = None;
            editor.image_view = Some(content);
        }
    }

    /// Cierra la tab activa.
    ///
    /// Si hay más de una tab, mueve el foco a la anterior (o siguiente
    /// si estamos en la primera). Si es la última tab, la reemplaza
    /// con un editor vacío.
    pub fn close_active(&mut self) {
        if self.editors.len() <= 1 {
            // Última tab — reemplazar con editor vacío
            self.editors[0] = EditorState::new();
            self.active_index = 0;
            return;
        }

        self.editors.remove(self.active_index);

        // Ajustar índice: si cerramos la última, retroceder
        if self.active_index >= self.editors.len() {
            self.active_index = self.editors.len() - 1;
        }
    }

    /// Ir a la tab siguiente (wraps al inicio).
    pub fn next_tab(&mut self) {
        if self.editors.len() > 1 {
            self.active_index = (self.active_index + 1) % self.editors.len();
        }
    }

    /// Ir a la tab anterior (wraps al final).
    pub fn prev_tab(&mut self) {
        if self.editors.len() > 1 {
            if self.active_index == 0 {
                self.active_index = self.editors.len() - 1;
            } else {
                self.active_index -= 1;
            }
        }
    }

    /// Cambiar a una tab por índice.
    ///
    /// Si el índice está fuera de rango, no hace nada.
    pub fn switch_to(&mut self, index: usize) {
        if index < self.editors.len() {
            self.active_index = index;
        }
    }

    /// Cantidad de tabs abiertas.
    pub fn tab_count(&self) -> usize {
        self.editors.len()
    }

    /// Genera info de tabs para renderizado.
    ///
    /// Pre-computa nombre, estado dirty y activo para cada tab.
    /// Se llama fuera del render loop.
    ///
    /// Las tabs de diff virtual usan un nombre tipo `"DIFF: file.rs"` o
    /// `"FILE: file.rs"` y nunca aparecen como dirty.
    pub fn tab_info(&self) -> Vec<TabInfo> {
        let mut infos = Vec::with_capacity(self.editors.len());
        for (i, editor) in self.editors.iter().enumerate() {
            // Tabs de diff virtual: nombre con prefijo DIFF/FILE + filename
            if let Some(ref dv) = editor.diff_view {
                let prefix = if dv.is_file_content { "FILE" } else { "DIFF" };
                let fname = dv
                    .file_path
                    .as_deref()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                // CLONE: TabInfo necesita ownership del nombre — formato pre-computado
                let name = format!("{prefix}: {fname}");
                // CLONE: file_path se duplica para TabInfo (path completo del diff)
                let path = dv.file_path.clone();
                infos.push(TabInfo {
                    name,
                    is_active: i == self.active_index,
                    is_dirty: false, // tabs de diff nunca son dirty
                    path,
                });
                continue;
            }

            // Tab normal de archivo
            let name = editor
                .buffer
                .file_path()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("[untitled]"));

            // CLONE: necesario — file_path() retorna &Path, necesitamos ownership para TabInfo
            let path = editor.buffer.file_path().map(Path::to_path_buf);

            infos.push(TabInfo {
                name,
                is_active: i == self.active_index,
                is_dirty: editor.buffer.is_dirty(),
                path,
            });
        }
        infos
    }

    /// Slice inmutable de todos los editores abiertos.
    ///
    /// Se usa para operaciones de lectura masiva (contar dirty, buscar
    /// untitled) sin necesidad de tomar `&mut self`.
    pub fn editors(&self) -> &[EditorState] {
        &self.editors
    }

    /// Slice mutable de todos los editores abiertos.
    ///
    /// Se usa para operaciones que afectan a todas las tabs, como guardar
    /// múltiples buffers o invalidar caches de highlighting.
    pub fn editors_mut(&mut self) -> &mut [EditorState] {
        &mut self.editors
    }

    /// Cambia la tab activa por índice. Alias semántico de `switch_to`.
    ///
    /// Si el índice está fuera de rango, no hace nada.
    pub fn set_active(&mut self, idx: usize) {
        self.switch_to(idx);
    }

    /// Índice de la tab activa.
    pub fn active_index(&self) -> usize {
        self.active_index
    }
}

impl Default for TabState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_returns_slice_of_all_editors() {
        let mut tabs = TabState::new();
        // Por defecto hay 1 editor vacío
        assert_eq!(tabs.editors().len(), 1);

        // Agregar un editor (vía open_diff_tab que no toca disco)
        tabs.open_diff_tab(String::from("diff content"), None, false);
        assert_eq!(tabs.editors().len(), 2);
    }

    #[test]
    fn editors_mut_allows_iteration_with_mutation() {
        let mut tabs = TabState::new();
        // Usar paths distintos para evitar la dedup de open_diff_tab.
        tabs.open_diff_tab(String::from("diff1"), Some(PathBuf::from("a.rs")), false);
        tabs.open_diff_tab(String::from("diff2"), Some(PathBuf::from("b.rs")), false);
        // 1 editor vacío + 2 diff tabs = 3
        let editors = tabs.editors_mut();
        assert_eq!(editors.len(), 3);
        // Mutar: cambiar scroll_offset del último diff
        if let Some(ref mut dv) = editors[2].diff_view {
            dv.scroll_offset = 42;
        }
        // Verificar que la mutación persiste
        let dv2 = tabs.editors()[2]
            .diff_view
            .as_ref()
            .expect("diff view exists");
        assert_eq!(dv2.scroll_offset, 42);
    }

    #[test]
    fn set_active_changes_active_index_in_range() {
        let mut tabs = TabState::new();
        tabs.open_diff_tab(String::from("d1"), Some(PathBuf::from("a.rs")), false);
        tabs.open_diff_tab(String::from("d2"), Some(PathBuf::from("b.rs")), false);
        // Por construcción, el último open_diff_tab queda activo
        assert_eq!(tabs.active_index(), 2);
        tabs.set_active(0);
        assert_eq!(tabs.active_index(), 0);
        tabs.set_active(1);
        assert_eq!(tabs.active_index(), 1);
    }

    #[test]
    fn set_active_ignores_out_of_range_index() {
        let mut tabs = TabState::new();
        tabs.open_diff_tab(String::from("d1"), Some(PathBuf::from("a.rs")), false);
        // 2 editores → índices válidos 0..=1
        tabs.set_active(0);
        let prev = tabs.active_index();
        tabs.set_active(99); // fuera de rango
        // No debe cambiar
        assert_eq!(tabs.active_index(), prev);
    }
}
