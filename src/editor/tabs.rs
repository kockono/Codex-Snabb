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

    /// Iterador mutable sobre todos los editores.
    ///
    /// Se usa para operaciones que afectan a todas las tabs, como
    /// invalidar caches de highlighting cuando el engine termina de cargar.
    #[expect(dead_code, reason = "API pública para operaciones masivas sobre tabs")]
    pub fn all_editors_mut(&mut self) -> &mut [EditorState] {
        &mut self.editors
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
