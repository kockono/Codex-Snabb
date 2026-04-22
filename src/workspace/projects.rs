//! Projects panel: lista de workspaces guardados con persistencia JSON.
//!
//! Mantiene una lista de proyectos que el usuario agrega manualmente
//! via diálogo nativo del SO (rfd::FileDialog). Persiste en
//! `~/.config/ide-tui/projects.json` (Linux/Mac) o
//! `%APPDATA%\ide-tui\projects.json` (Windows).
//! Sin allocaciones en hot paths — solo IO en load/save explícitos.

use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc;

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
// Debug implementado manualmente porque `mpsc::Receiver` no es Debug.
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

    // ── Receptor del diálogo nativo de carpeta ──
    /// Canal de un solo uso para recibir el resultado del diálogo nativo.
    /// `Some` mientras hay un diálogo abierto; `None` en caso contrario.
    /// Se consulta en cada tick — no bloquea el event loop.
    pub native_picker_rx: Option<mpsc::Receiver<PathBuf>>,
}

impl fmt::Debug for ProjectsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectsState")
            .field("projects", &self.projects)
            .field("selected", &self.selected)
            .field("scroll_offset", &self.scroll_offset)
            .field("visible", &self.visible)
            .field("active_project", &self.active_project)
            .field(
                "native_picker_rx",
                &self.native_picker_rx.as_ref().map(|_| "<Receiver>"),
            )
            .finish()
    }
}

impl ProjectsState {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            visible: false,
            active_project: None,
            native_picker_rx: None,
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

    // ── Diálogo nativo de carpeta ──────────────────────────────────────────────

    /// Consulta si el diálogo nativo retornó un resultado.
    ///
    /// Se llama en cada tick del event loop — no bloquea.
    /// Retorna `Some(PathBuf)` si el usuario confirmó una carpeta,
    /// `None` si el diálogo sigue abierto o fue cancelado.
    pub fn poll_native_picker(&mut self) -> Option<PathBuf> {
        let rx = self.native_picker_rx.as_ref()?;
        match rx.try_recv() {
            Ok(path) => {
                self.native_picker_rx = None;
                Some(path)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                // El hilo del diálogo terminó sin enviar (usuario canceló)
                self.native_picker_rx = None;
                None
            }
        }
    }
}
