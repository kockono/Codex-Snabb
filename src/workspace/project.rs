//! Project manager: gestión de proyectos recientes.
//!
//! Mantiene una lista ordenada de proyectos abiertos recientemente.
//! Persistencia en JSON simple. Placeholder por ahora — se usará
//! en épicas futuras con command palette para cambiar entre proyectos.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Proyecto reciente con metadatos mínimos.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecentProject {
    /// Nombre del proyecto (derivado del directorio).
    pub name: String,
    /// Path absoluto al directorio raíz.
    pub path: PathBuf,
    /// Timestamp de la última apertura.
    pub last_opened: SystemTime,
}

/// Gestor de proyectos recientes.
///
/// Mantiene una lista limitada por `max_recent`. Al agregar un proyecto
/// que ya existe, lo mueve al frente. Persistencia vía JSON.
#[derive(Debug)]
pub struct ProjectManager {
    /// Lista de proyectos recientes (más reciente primero).
    pub recent_projects: Vec<RecentProject>,
    /// Máximo de proyectos a recordar.
    pub max_recent: usize,
}

impl ProjectManager {
    /// Crea un project manager vacío con límite por defecto.
    pub fn new() -> Self {
        Self {
            recent_projects: Vec::new(),
            max_recent: 10,
        }
    }

    /// Agrega un proyecto a la lista de recientes.
    ///
    /// Si el proyecto ya existe (por path), lo mueve al frente y actualiza
    /// el timestamp. Si la lista excede `max_recent`, elimina el más antiguo.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente apertura de proyectos"
    )]
    pub fn add_recent(&mut self, path: &Path) {
        // Remover si ya existe (lo vamos a re-agregar al frente)
        self.recent_projects.retain(|p| p.path != path);

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        self.recent_projects.insert(
            0,
            RecentProject {
                name,
                // CLONE: necesario — el manager necesita ownership del path
                path: path.to_path_buf(),
                last_opened: SystemTime::now(),
            },
        );

        // Truncar si excede el límite
        self.recent_projects.truncate(self.max_recent);
    }

    /// Elimina un proyecto de la lista por índice.
    ///
    /// No-op si el índice está fuera de rango.
    #[expect(dead_code, reason = "se usará desde command palette en épicas futuras")]
    pub fn remove_recent(&mut self, index: usize) {
        if index < self.recent_projects.len() {
            self.recent_projects.remove(index);
        }
    }

    /// Carga la lista de recientes desde un archivo JSON.
    ///
    /// Si el archivo no existe, retorna un manager vacío — no es un error.
    /// Si el archivo existe pero tiene formato inválido, retorna error.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente persistencia de recientes"
    )]
    pub fn load(config_path: &Path) -> Result<Self> {
        if !config_path.exists() {
            return Ok(Self::new());
        }
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("no se pudo leer config: {}", config_path.display()))?;
        let recent_projects: Vec<RecentProject> = serde_json::from_str(&content)
            .with_context(|| format!("formato JSON inválido en: {}", config_path.display()))?;
        Ok(Self {
            recent_projects,
            max_recent: 10,
        })
    }

    /// Guarda la lista de recientes a un archivo JSON.
    ///
    /// Crea directorios padre si no existen.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente persistencia de recientes"
    )]
    pub fn save(&self, config_path: &Path) -> Result<()> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("no se pudo crear directorio: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(&self.recent_projects)
            .context("no se pudo serializar proyectos recientes")?;
        std::fs::write(config_path, content)
            .with_context(|| format!("no se pudo guardar config: {}", config_path.display()))?;
        Ok(())
    }
}

impl Default for ProjectManager {
    fn default() -> Self {
        Self::new()
    }
}
