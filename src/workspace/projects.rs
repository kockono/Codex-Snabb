//! Projects panel: lista de workspaces guardados con persistencia JSON.
//!
//! Mantiene una lista de proyectos que el usuario agrega manualmente
//! via folder picker. Persiste en `~/.config/ide-tui/projects.json`
//! (Linux/Mac) o `%APPDATA%\ide-tui\projects.json` (Windows).
//! Sin allocaciones en hot paths — solo IO en load/save explícitos.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Un proyecto guardado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// Nombre display (nombre de la carpeta raíz).
    pub name: String,
    /// Ruta absoluta al directorio raíz del proyecto.
    pub path: PathBuf,
    /// Si el candado está activo — no cambia el explorer al seleccionar.
    pub locked: bool,
}

impl ProjectEntry {
    pub fn new(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("proyecto")
            .to_owned();
        Self {
            name,
            path,
            locked: false,
        }
    }
}

/// Estado del panel de proyectos.
#[derive(Debug)]
pub struct ProjectsState {
    /// Lista de proyectos guardados.
    pub projects: Vec<ProjectEntry>,
    /// Índice del proyecto seleccionado en la lista.
    pub selected: usize,
    /// Scroll offset para listas largas.
    pub scroll_offset: usize,
    /// Si el panel está visible (sidebar activa en modo Projects).
    pub visible: bool,
    /// Índice del proyecto actualmente activo (workspace abierto).
    pub active_project: Option<usize>,

    // ── Path input inline (fila 1 del panel) ──
    /// Texto que el usuario va escribiendo en el input de ruta inline.
    pub path_input: String,
    /// Si el input de ruta inline tiene el foco del teclado.
    pub path_input_focused: bool,
    /// Si hay un error de ruta no válida activo.
    pub path_input_error: bool,
    /// Ticks restantes para mostrar el error efímero (se cuenta hacia 0).
    pub path_input_error_ticks: u8,
}

impl ProjectsState {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            visible: false,
            active_project: None,
            path_input: String::new(),
            path_input_focused: false,
            path_input_error: false,
            path_input_error_ticks: 0,
        }
    }

    /// Ruta al archivo de persistencia.
    /// Usa el config dir del sistema: ~/.config/ide-tui/projects.json (Linux/Mac)
    /// o %APPDATA%\ide-tui\projects.json (Windows).
    fn config_path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var("APPDATA").ok().map(PathBuf::from)
        } else {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        }?;
        Some(base.join("ide-tui").join("projects.json"))
    }

    /// Carga proyectos desde disco. Silencioso si no existe el archivo.
    pub fn load(&mut self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        if let Ok(projects) = serde_json::from_str::<Vec<ProjectEntry>>(&content) {
            self.projects = projects;
        }
    }

    /// Guarda proyectos a disco. Silencioso en error.
    fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.projects) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Agrega un proyecto si la ruta no existe ya.
    pub fn add(&mut self, path: PathBuf) {
        // No duplicar
        if self.projects.iter().any(|p| p.path == path) {
            return;
        }
        self.projects.push(ProjectEntry::new(path));
        self.save();
    }

    /// Elimina el proyecto en el índice dado.
    pub fn remove(&mut self, idx: usize) {
        if idx < self.projects.len() {
            self.projects.remove(idx);
            self.clamp_selection();
            self.save();
        }
    }

    /// Toggle del candado del proyecto en el índice dado.
    pub fn toggle_lock(&mut self, idx: usize) {
        if let Some(p) = self.projects.get_mut(idx) {
            p.locked = !p.locked;
            self.save();
        }
    }

    pub fn move_up(&mut self) {
        if !self.projects.is_empty() {
            if self.selected > 0 {
                self.selected -= 1;
            } else {
                self.selected = self.projects.len() - 1;
            }
            self.ensure_visible();
        }
    }

    pub fn move_down(&mut self) {
        if !self.projects.is_empty() {
            if self.selected + 1 < self.projects.len() {
                self.selected += 1;
            } else {
                self.selected = 0;
            }
            self.ensure_visible();
        }
    }

    pub fn selected_project(&self) -> Option<&ProjectEntry> {
        self.projects.get(self.selected)
    }

    fn clamp_selection(&mut self) {
        if self.projects.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.projects.len() {
            self.selected = self.projects.len() - 1;
        }
        self.scroll_offset = 0;
    }

    fn ensure_visible(&mut self) {
        const MAX_VISIBLE: usize = 10;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + MAX_VISIBLE {
            self.scroll_offset = self.selected - MAX_VISIBLE + 1;
        }
    }

    // ── Path input inline ──────────────────────────────────────────────────────

    /// Agrega un carácter al final del input de ruta inline.
    pub fn path_input_push(&mut self, ch: char) {
        self.path_input.push(ch);
    }

    /// Elimina el último carácter del input de ruta inline (Backspace).
    pub fn path_input_backspace(&mut self) {
        self.path_input.pop();
    }

    /// Intenta confirmar el path escrito.
    ///
    /// Si el path existe como directorio → limpia el input, quita el foco,
    /// y retorna `Some(PathBuf)` con la ruta.
    ///
    /// Si el path no existe o está vacío → activa el error efímero y retorna `None`.
    pub fn path_input_confirm(&mut self) -> Option<PathBuf> {
        if self.path_input.is_empty() {
            return None;
        }
        // Capturar el path ANTES de mutar self
        let candidate = PathBuf::from(&self.path_input);
        if std::path::Path::new(&self.path_input).is_dir() {
            self.path_input.clear();
            self.path_input_focused = false;
            self.path_input_error = false;
            self.path_input_error_ticks = 0;
            Some(candidate)
        } else {
            // Ruta no válida: activar error efímero (~2s a 20fps = 40 ticks)
            self.path_input_error = true;
            self.path_input_error_ticks = 40;
            None
        }
    }

    /// Cancela la edición del path: limpia el input, quita el foco y el error.
    pub fn path_input_escape(&mut self) {
        self.path_input.clear();
        self.path_input_focused = false;
        self.path_input_error = false;
        self.path_input_error_ticks = 0;
    }

    /// Decrementa el contador de error efímero.
    /// Cuando llega a 0 limpia el estado de error.
    pub fn path_input_tick(&mut self) {
        if self.path_input_error_ticks > 0 {
            self.path_input_error_ticks -= 1;
            if self.path_input_error_ticks == 0 {
                self.path_input_error = false;
            }
        }
    }

    /// Da foco al input de ruta inline.
    pub fn path_input_focus(&mut self) {
        self.path_input_focused = true;
    }
}
