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
    /// Texto del input de path editable (vacío = mostrar placeholder).
    pub path_input: String,
    /// Si el foco está en el input de path (true) o en el árbol (false).
    pub path_input_focused: bool,
    /// Mensaje de error efímero (path no encontrado, etc.).
    pub path_error: Option<String>,
    /// Countdown de ticks para limpiar `path_error`. 40 ticks ≈ 2s a 20 FPS.
    pub path_error_ticks: u8,
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
            path_input: String::new(),
            path_input_focused: false,
            path_error: None,
            path_error_ticks: 0,
        }
    }

    /// Abre el picker en el directorio dado.
    pub fn open(&mut self, root: PathBuf) {
        self.visible = true;
        self.selected = 0;
        self.scroll_offset = 0;
        self.confirmed_path = None;
        self.path_input.clear();
        self.path_input_focused = false;
        self.path_error = None;
        self.path_error_ticks = 0;
        // CLONE: necesario — root se mueve a current_root y también se usa para load_root
        self.current_root = root.clone();
        self.load_root(&root);
    }

    /// Cierra el picker y limpia estado.
    pub fn close(&mut self) {
        self.visible = false;
        self.entries.clear();
        self.confirmed_path = None;
        self.path_input.clear();
        self.path_input_focused = false;
        self.path_error = None;
        self.path_error_ticks = 0;
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

    // ── Path input methods ──

    /// Alterna el foco entre el input de path y el árbol de directorios.
    pub fn toggle_focus(&mut self) {
        self.path_input_focused = !self.path_input_focused;
        self.path_error = None;
        self.path_error_ticks = 0;
    }

    /// Agrega un carácter al input de path.
    pub fn path_input_push(&mut self, ch: char) {
        self.path_input.push(ch);
    }

    /// Elimina el último carácter del input de path.
    pub fn path_input_backspace(&mut self) {
        self.path_input.pop();
    }

    /// Intenta navegar al path escrito en el input.
    ///
    /// Si el path existe y es un directorio, actualiza `current_root`,
    /// refresca las entradas, limpia el input y devuelve foco al árbol.
    /// Si no existe, setea `path_error` con mensaje y retorna `false`.
    pub fn try_navigate_to_input(&mut self) -> bool {
        let path = PathBuf::from(&self.path_input);
        if path.is_dir() {
            // CLONE: necesario — path se mueve a current_root y también se usa para load_root
            self.current_root = path.clone();
            self.selected = 0;
            self.scroll_offset = 0;
            self.load_root(&path);
            self.path_input.clear();
            self.path_input_focused = false;
            self.path_error = None;
            self.path_error_ticks = 0;
            true
        } else {
            self.path_error = Some(String::from("Path not found"));
            self.path_error_ticks = 40; // ~2s a 20 FPS
            false
        }
    }

    /// Decrementa el countdown del error efímero. Limpia `path_error` cuando llega a 0.
    pub fn tick_error(&mut self) {
        if self.path_error_ticks > 0 {
            self.path_error_ticks -= 1;
            if self.path_error_ticks == 0 {
                self.path_error = None;
            }
        }
    }

    /// Limpia el input de path y devuelve foco al árbol. NO cierra el picker.
    pub fn path_input_escape(&mut self) {
        self.path_input.clear();
        self.path_input_focused = false;
        self.path_error = None;
        self.path_error_ticks = 0;
    }

    /// Retorna el texto a mostrar en el input: el texto escrito si no está vacío,
    /// o el `current_root` como placeholder.
    pub fn display_text(&self) -> &str {
        if self.path_input.is_empty() {
            self.current_root.to_str().unwrap_or("")
        } else {
            self.path_input.as_str()
        }
    }
}
