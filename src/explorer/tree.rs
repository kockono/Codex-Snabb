//! Árbol de archivos con carga lazy por directorio.
//!
//! Cada directorio carga sus hijos recién al expandir (no recursivo).
//! Esto mantiene el costo de RAM proporcional a la profundidad expandida,
//! no al tamaño total del workspace. Directorios ignorados (.git, target,
//! node_modules) se filtran al cargar.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Nombres de directorios/archivos que se ignoran al cargar el árbol.
///
/// Lista fija — no configurable por ahora. Se puede expandir en el futuro
/// si se agrega soporte para .gitignore o .editorconfig.
const IGNORED_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".DS_Store",
    "thumbs.db",
    "Thumbs.db",
];

/// Entrada del árbol de archivos — archivo o directorio.
///
/// Los directorios son lazy: `loaded` indica si los hijos se cargaron.
/// Al expandir un directorio no cargado, se lee el filesystem una sola vez.
#[derive(Debug)]
pub enum FileEntry {
    /// Archivo regular.
    File {
        /// Nombre del archivo (solo el componente final, no el path completo).
        name: String,
        /// Path absoluto al archivo.
        path: PathBuf,
    },
    /// Directorio con hijos lazy.
    Directory {
        /// Nombre del directorio (solo el componente final).
        name: String,
        /// Path absoluto al directorio.
        path: PathBuf,
        /// Hijos (vacío si `loaded == false`).
        children: Vec<FileEntry>,
        /// Si el directorio está expandido en el UI.
        expanded: bool,
        /// Si los hijos se cargaron del filesystem (lazy loading flag).
        loaded: bool,
    },
}

impl FileEntry {
    /// Nombre del entry (archivo o directorio).
    pub fn name(&self) -> &str {
        match self {
            Self::File { name, .. } | Self::Directory { name, .. } => name,
        }
    }

    /// Path completo del entry.
    #[expect(
        dead_code,
        reason = "API pública del árbol — se usará en búsqueda y quick open"
    )]
    pub fn path(&self) -> &Path {
        match self {
            Self::File { path, .. } | Self::Directory { path, .. } => path,
        }
    }

    /// Si el entry es un directorio.
    #[expect(
        dead_code,
        reason = "API pública del árbol — se usará en filtros y quick open"
    )]
    pub fn is_dir(&self) -> bool {
        matches!(self, Self::Directory { .. })
    }

    /// Si un nombre está en la lista de ignorados.
    ///
    /// Comparación case-insensitive para Windows/Mac donde
    /// `Thumbs.db` y `thumbs.db` son el mismo archivo.
    pub fn is_ignored(name: &str) -> bool {
        let name_lower = name.to_ascii_lowercase();
        IGNORED_NAMES
            .iter()
            .any(|ignored| ignored.to_ascii_lowercase() == name_lower)
    }

    /// Toggle expand/collapse de un directorio.
    ///
    /// Si el directorio no estaba cargado (`loaded == false`), carga los
    /// hijos del filesystem. Si ya estaba cargado, solo alterna `expanded`.
    /// En archivos es un no-op.
    pub fn toggle_expand(&mut self) -> Result<()> {
        if let Self::Directory {
            path,
            children,
            expanded,
            loaded,
            ..
        } = self
        {
            if !*loaded {
                *children = load_directory(path)?;
                *loaded = true;
            }
            *expanded = !*expanded;
        }
        Ok(())
    }
}

/// Lee un directorio y retorna sus entries (no recursivo).
///
/// Ordena: directorios primero, luego archivos, ambos alfabéticamente.
/// Filtra nombres ignorados. IO síncrono — aceptable porque lazy loading
/// minimiza el impacto (solo se carga al expandir).
pub fn load_directory(path: &Path) -> Result<Vec<FileEntry>> {
    let read_dir = std::fs::read_dir(path)
        .with_context(|| format!("no se pudo leer directorio: {}", path.display()))?;

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry_result in read_dir {
        let entry = entry_result
            .with_context(|| format!("error leyendo entrada en: {}", path.display()))?;

        let name = entry.file_name().to_string_lossy().into_owned();

        if FileEntry::is_ignored(&name) {
            continue;
        }

        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("no se pudo obtener tipo de: {}", entry_path.display()))?;

        if file_type.is_dir() {
            dirs.push(FileEntry::Directory {
                name,
                path: entry_path,
                children: Vec::new(),
                expanded: false,
                loaded: false,
            });
        } else if file_type.is_file() {
            files.push(FileEntry::File {
                name,
                path: entry_path,
            });
        }
        // Symlinks y otros tipos especiales se ignoran por ahora
    }

    // Orden alfabético case-insensitive
    dirs.sort_by(|a, b| {
        a.name()
            .to_ascii_lowercase()
            .cmp(&b.name().to_ascii_lowercase())
    });
    files.sort_by(|a, b| {
        a.name()
            .to_ascii_lowercase()
            .cmp(&b.name().to_ascii_lowercase())
    });

    // Directorios primero, luego archivos
    dirs.extend(files);
    Ok(dirs)
}
