//! Folder picker: mini browser de carpetas para seleccionar un directorio.
//!
//! Muestra el árbol de directorios navegable con flechas.
//! Solo permite seleccionar directorios (los archivos se muestran dimmed).
//! Enter en directorio = navegar dentro. 'S' = confirmar selección.

use std::path::PathBuf;

/// Una entrada en el folder picker.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    /// Nombre del directorio (solo el último segmento).
    pub name: String,
    /// Ruta absoluta.
    pub path: PathBuf,
    /// Si está expandido (muestra sus hijos).
    pub expanded: bool,
    /// Profundidad de indentación (0 = raíz).
    pub depth: usize,
    /// Si es directorio (true) o archivo (false).
    pub is_dir: bool,
}

/// Estado del folder picker modal.
#[derive(Debug)]
pub struct FolderPickerState {
    /// Si el picker está visible.
    pub visible: bool,
    /// Directorio raíz actual del picker.
    pub current_root: PathBuf,
    /// Entradas visibles (aplanadas del árbol).
    pub entries: Vec<FolderEntry>,
    /// Índice seleccionado en `entries`.
    pub selected: usize,
    /// Scroll offset.
    pub scroll_offset: usize,
    /// Ruta confirmada (Some cuando usuario confirmó).
    pub confirmed_path: Option<PathBuf>,
}

impl FolderPickerState {
    pub fn new() -> Self {
        Self {
            visible: false,
            current_root: PathBuf::new(),
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            confirmed_path: None,
        }
    }

    /// Abre el picker en el directorio dado.
    pub fn open(&mut self, root: PathBuf) {
        self.visible = true;
        self.selected = 0;
        self.scroll_offset = 0;
        self.confirmed_path = None;
        // CLONE: necesario — root se mueve a current_root y también se usa para load_root
        self.current_root = root.clone();
        self.load_root(&root);
    }

    /// Cierra el picker y limpia estado.
    pub fn close(&mut self) {
        self.visible = false;
        self.entries.clear();
        self.confirmed_path = None;
    }

    /// Carga el directorio raíz (primer nivel).
    fn load_root(&mut self, root: &PathBuf) {
        self.entries.clear();
        self.entries.push(FolderEntry {
            name: root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("/")
                .to_owned(),
            // CLONE: necesario — entry necesita ownership del path
            path: root.clone(),
            expanded: true,
            depth: 0,
            is_dir: true,
        });
        self.load_children(root, 1);
    }

    /// Carga los hijos de un directorio al árbol (un nivel).
    fn load_children(&mut self, dir: &PathBuf, depth: usize) {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };

        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        let mut files: Vec<(String, PathBuf)> = Vec::new();

        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Ignorar hidden y comunes de build
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                dirs.push((name, path));
            } else {
                files.push((name, path));
            }
        }

        // Ordenar: dirs primero, luego archivos, ambos alfabéticos
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, path) in dirs {
            self.entries.push(FolderEntry {
                name,
                path,
                expanded: false,
                depth,
                is_dir: true,
            });
        }
        // Mostrar archivos dimmed (no seleccionables para confirmar) — max 5 para no saturar
        let shown = files.len().min(5);
        for (name, path) in files.iter().take(shown) {
            self.entries.push(FolderEntry {
                // CLONE: necesario — iterando por referencia con take()
                name: name.clone(),
                path: path.clone(),
                expanded: false,
                depth,
                is_dir: false,
            });
        }
        if files.len() > 5 {
            // placeholder indicando que hay más archivos
            let extra = files.len() - 5;
            // Pre-compute el string — no se llama en render loop
            let mut label = String::with_capacity(24);
            use std::fmt::Write;
            let _ = write!(label, "... {} archivos mas", extra);
            self.entries.push(FolderEntry {
                name: label,
                path: PathBuf::new(),
                expanded: false,
                depth,
                is_dir: false,
            });
        }
    }

    /// Navega arriba en la lista.
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Navega abajo en la lista.
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Enter en la entrada seleccionada: expande/colapsa si es dir.
    pub fn enter_selected(&mut self) {
        let Some(entry) = self.entries.get(self.selected) else {
            return;
        };
        if !entry.is_dir {
            return;
        }
        // CLONE: necesario — tomamos datos antes de mutar self.entries
        let path = entry.path.clone();
        let depth = entry.depth;
        let expanded = entry.expanded;

        if expanded {
            // Colapsar: eliminar todos los hijos (depth > entry.depth)
            let start = self.selected + 1;
            let end = self.entries[start..]
                .iter()
                .position(|e| e.depth <= depth)
                .map(|i| start + i)
                .unwrap_or(self.entries.len());
            self.entries.drain(start..end);
            if let Some(e) = self.entries.get_mut(self.selected) {
                e.expanded = false;
            }
        } else {
            // Expandir: insertar hijos después de la entrada actual
            let insert_at = self.selected + 1;
            let prev_len = self.entries.len();
            // Cargar en un vec temporal y luego insertar
            let mut temp = FolderPickerState::new();
            temp.load_children(&path, depth + 1);
            let new_entries = temp.entries;
            // Insertar en orden reverso para mantener posiciones correctas
            for (i, entry) in new_entries.into_iter().enumerate() {
                self.entries.insert(insert_at + i, entry);
            }
            let has_children = self.entries.len() > prev_len;
            if let Some(e) = self.entries.get_mut(self.selected) {
                e.expanded = has_children;
            }
        }
    }

    /// Sube al directorio padre.
    pub fn go_parent(&mut self) {
        if let Some(parent) = self.current_root.parent().map(|p| p.to_path_buf()) {
            self.open(parent);
        }
    }

    /// Confirma el directorio seleccionado como proyecto.
    /// Solo funciona si la entrada seleccionada es un directorio.
    pub fn confirm_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected)
            && entry.is_dir
            && !entry.path.as_os_str().is_empty()
        {
            // CLONE: necesario — guardamos el path confirmado
            self.confirmed_path = Some(entry.path.clone());
            self.visible = false;
        }
    }

    fn ensure_visible(&mut self) {
        const MAX_VISIBLE: usize = 18;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + MAX_VISIBLE {
            self.scroll_offset = self.selected - MAX_VISIBLE + 1;
        }
    }
}
