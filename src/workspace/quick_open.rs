//! Quick Open: índice liviano de archivos del workspace para búsqueda rápida.
//!
//! Escanea el workspace recursivamente al abrir (respetando ignore list),
//! construye un índice de paths relativos, y filtra con ranking por
//! relevancia. Todo el filtrado se hace en `update_filter()` — NUNCA
//! en render. El render solo dibuja desde el cache de `filtered`.
//!
//! Límite de `MAX_FILES` para no explotar RAM en workspaces gigantes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Modo activo del Quick Open.
///
/// `FileSearch` es el modo por defecto (buscar archivos). Cuando el usuario
/// escribe `:` como primer carácter, cambia a `GoToLine` para saltar a una
/// línea del archivo activo sin abrir un modal separado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuickOpenMode {
    /// Búsqueda de archivos (modo por defecto).
    #[default]
    FileSearch,
    /// Go-to-line inline: input es un número de línea.
    GoToLine,
}

/// Máximo de archivos a indexar para evitar consumo excesivo de RAM.
const MAX_FILES: usize = 10_000;

/// Máximo de items visibles en la lista del quick open.
pub const MAX_VISIBLE_ITEMS: usize = 15;

/// Nombres de directorios/archivos que se ignoran al escanear.
///
/// Lista coherente con `tree.rs` — mismos criterios de exclusión.
const IGNORED_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".DS_Store",
    "thumbs.db",
    "Thumbs.db",
    ".hg",
    ".svn",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".cache",
];

/// Estado del Quick Open (Ctrl+P).
///
/// Mantiene el índice de archivos del workspace, el input de búsqueda,
/// la lista filtrada (como índices al `file_index`) y la selección actual.
/// El filtrado se cachea en `update_filter()` — render solo dibuja.
#[derive(Debug)]
pub struct QuickOpenState {
    /// Si el quick open está visible.
    pub visible: bool,
    /// Texto de búsqueda del usuario.
    pub input: String,
    /// Posición del cursor dentro del input.
    pub cursor_pos: usize,
    /// Todos los archivos conocidos del workspace (paths relativos).
    pub file_index: Vec<PathBuf>,
    /// Índices filtrados dentro de `file_index` (ordenados por relevancia).
    pub filtered: Vec<usize>,
    /// Índice de la selección dentro de `filtered`.
    pub selected_index: usize,
    /// Offset de scroll para listas largas.
    pub scroll_offset: usize,
    /// Si está escaneando el workspace actualmente.
    pub scanning: bool,
    /// Modo activo — FileSearch o GoToLine inline.
    pub mode: QuickOpenMode,
    /// Total de líneas del archivo activo (para go-to-line hint).
    pub total_lines: usize,
}

impl QuickOpenState {
    /// Crea un estado inicial (quick open cerrado, índice vacío).
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::with_capacity(64),
            cursor_pos: 0,
            file_index: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            scanning: false,
            mode: QuickOpenMode::FileSearch,
            total_lines: 0,
        }
    }

    /// Abre el quick open: limpia input, muestra todos los archivos.
    pub fn open(&mut self) {
        self.visible = true;
        self.input.clear();
        self.cursor_pos = 0;
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.mode = QuickOpenMode::FileSearch;
        self.total_lines = 0;
        // Mostrar todos los archivos inicialmente
        self.filtered = (0..self.file_index.len()).collect();
    }

    /// Cierra el quick open y limpia el estado de búsqueda.
    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.cursor_pos = 0;
        self.filtered.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.mode = QuickOpenMode::FileSearch;
    }

    /// Escanea el workspace recursivamente para construir el índice de archivos.
    ///
    /// Los paths se almacenan RELATIVOS al root para display limpio.
    /// Respeta la ignore list y el límite de `MAX_FILES`.
    /// IO síncrono — aceptable porque se ejecuta una sola vez al inicio
    /// y al abrir el quick open (no en el render loop).
    pub fn build_index(&mut self, root: &Path) -> Result<()> {
        self.scanning = true;
        self.file_index.clear();

        // Pre-alocar con capacidad razonable — evitar grow repetidos
        self.file_index.reserve(1024);

        scan_directory_recursive(root, root, &mut self.file_index)
            .with_context(|| format!("error al escanear workspace: {}", root.display()))?;

        self.scanning = false;

        tracing::info!(
            files = self.file_index.len(),
            root = %root.display(),
            "índice de quick open construido"
        );

        Ok(())
    }

    /// Actualiza la lista filtrada según el input actual.
    ///
    /// Ranking de relevancia (case-insensitive):
    /// 1. Exact match en filename
    /// 2. Prefix match en filename
    /// 3. Contains en filename
    /// 4. Contains en path completo (relativo al root)
    /// 5. Fuzzy básico (cada char del query aparece en orden en el path)
    pub fn update_filter(&mut self) {
        self.filtered.clear();

        if self.input.is_empty() {
            // Sin filtro — mostrar todos
            self.filtered = (0..self.file_index.len()).collect();
            self.clamp_selection();
            return;
        }

        let query_lower = self.input.to_lowercase();

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut contains_name: Vec<usize> = Vec::new();
        let mut contains_path: Vec<usize> = Vec::new();
        let mut fuzzy: Vec<usize> = Vec::new();

        for (idx, path) in self.file_index.iter().enumerate() {
            // Extraer filename para comparaciones por nombre
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            let path_str = path.to_string_lossy().to_lowercase();

            if filename == query_lower {
                exact.push(idx);
            } else if filename.starts_with(&query_lower) {
                prefix.push(idx);
            } else if filename.contains(&query_lower) {
                contains_name.push(idx);
            } else if path_str.contains(&query_lower) {
                contains_path.push(idx);
            } else if fuzzy_match(&query_lower, &path_str) {
                fuzzy.push(idx);
            }
        }

        // Capacidad conocida — evitar re-allocaciones
        let total =
            exact.len() + prefix.len() + contains_name.len() + contains_path.len() + fuzzy.len();
        self.filtered.reserve(total);
        self.filtered.extend(exact);
        self.filtered.extend(prefix);
        self.filtered.extend(contains_name);
        self.filtered.extend(contains_path);
        self.filtered.extend(fuzzy);

        self.clamp_selection();
    }

    /// Mueve la selección una posición arriba (con wrap).
    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                // Wrap al final
                self.selected_index = self.filtered.len() - 1;
            }
            self.ensure_visible();
        }
    }

    /// Mueve la selección una posición abajo (con wrap).
    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index + 1 < self.filtered.len() {
                self.selected_index += 1;
            } else {
                // Wrap al inicio
                self.selected_index = 0;
            }
            self.ensure_visible();
        }
    }

    /// Inserta un carácter en el input y re-filtra.
    ///
    /// Comportamiento por modo:
    /// - **FileSearch**: si el input está vacío y se escribe `:`, cambia a GoToLine
    ///   (sin insertar el `:`). Caso contrario, inserción normal + re-filtrado.
    /// - **GoToLine**: solo acepta dígitos ASCII, máximo 6 chars.
    pub fn insert_char(&mut self, ch: char) {
        // Si el input está vacío y se escribe ':', cambiar a modo go-to-line
        if self.input.is_empty() && ch == ':' && self.mode == QuickOpenMode::FileSearch {
            self.mode = QuickOpenMode::GoToLine;
            // NO insertar el ':' en el input — es el prefijo visual, no parte del número
            return;
        }
        // En modo go-to-line: solo aceptar dígitos
        if self.mode == QuickOpenMode::GoToLine {
            if ch.is_ascii_digit() && self.input.len() < 6 {
                self.input.push(ch);
            }
            return;
        }
        // Modo file search: comportamiento normal
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
        self.update_filter();
    }

    /// Elimina el carácter antes del cursor y re-filtra.
    ///
    /// Comportamiento por modo:
    /// - **GoToLine**: si el input está vacío (borrar el `:` implícito), vuelve
    ///   a FileSearch mostrando todos los archivos. Si hay dígitos, borra el último.
    /// - **FileSearch**: borrado normal con re-filtrado.
    pub fn delete_char(&mut self) {
        // En modo go-to-line: lógica especial
        if self.mode == QuickOpenMode::GoToLine {
            if self.input.is_empty() {
                // Borrar el ':' implícito → volver a file search
                self.mode = QuickOpenMode::FileSearch;
                self.update_filter(); // mostrar todos los archivos
            } else {
                self.input.pop();
            }
            return;
        }
        // Modo file search: comportamiento normal
        if self.cursor_pos > 0 {
            // Encontrar el boundary del char anterior
            let prev_boundary = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.input.drain(prev_boundary..self.cursor_pos);
            self.cursor_pos = prev_boundary;
            self.update_filter();
        }
    }

    /// Parsea el input como número de línea (1-indexed) en modo GoToLine.
    ///
    /// Retorna `None` si el input está vacío, es cero, o el modo no es GoToLine.
    /// Clampea al rango `[1, total_lines]`.
    pub fn parsed_line(&self) -> Option<usize> {
        if self.mode != QuickOpenMode::GoToLine {
            return None;
        }
        let n: usize = self.input.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some(n.min(self.total_lines).max(1))
    }

    /// Si el modo activo es go-to-line inline.
    pub fn is_goto_mode(&self) -> bool {
        self.mode == QuickOpenMode::GoToLine
    }

    /// Path del archivo seleccionado actualmente, si existe.
    pub fn selected_path(&self) -> Option<&Path> {
        let &file_idx = self.filtered.get(self.selected_index)?;
        self.file_index.get(file_idx).map(|p| p.as_path())
    }

    /// Ajusta la selección si se sale del rango de filtered.
    fn clamp_selection(&mut self) {
        if self.filtered.is_empty() || self.selected_index >= self.filtered.len() {
            self.selected_index = 0;
        }
        self.scroll_offset = 0;
    }

    /// Ajusta el scroll para que la selección sea visible.
    fn ensure_visible(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + MAX_VISIBLE_ITEMS {
            self.scroll_offset = self.selected_index - MAX_VISIBLE_ITEMS + 1;
        }
    }
}

/// Fuzzy match básico: cada carácter del query aparece en orden en el target.
///
/// "mr" matchea "main.rs", "abc" matchea "a_big_cat". Case-insensitive
/// (caller pasa strings ya lowercased).
fn fuzzy_match(query: &str, target: &str) -> bool {
    let mut target_chars = target.chars();
    for query_char in query.chars() {
        // Buscar el próximo char del query en lo que queda del target
        let found = target_chars.any(|tc| tc == query_char);
        if !found {
            return false;
        }
    }
    true
}

/// Escanea un directorio recursivamente, agregando archivos al índice.
///
/// Los paths se almacenan relativos al `root`. Se detiene al alcanzar
/// `MAX_FILES` para limitar consumo de RAM. Directorios en la ignore
/// list se saltan completamente.
fn scan_directory_recursive(dir: &Path, root: &Path, index: &mut Vec<PathBuf>) -> Result<()> {
    if index.len() >= MAX_FILES {
        return Ok(());
    }

    let read_dir = std::fs::read_dir(dir)
        .with_context(|| format!("no se pudo leer directorio: {}", dir.display()))?;

    for entry_result in read_dir {
        if index.len() >= MAX_FILES {
            break;
        }

        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                // Log y continuar — un entry inaccesible no debe parar el scan
                tracing::debug!(error = %e, dir = %dir.display(), "entry inaccesible, saltando");
                continue;
            }
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if is_ignored(&name_str) {
            continue;
        }

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                tracing::debug!(error = %e, "no se pudo obtener tipo de archivo, saltando");
                continue;
            }
        };

        let entry_path = entry.path();

        if file_type.is_dir() {
            // Recursión en subdirectorio
            scan_directory_recursive(&entry_path, root, index)?;
        } else if file_type.is_file() {
            // Path relativo al workspace root
            if let Ok(relative) = entry_path.strip_prefix(root) {
                index.push(relative.to_path_buf());
            }
        }
        // Symlinks y otros tipos se ignoran
    }

    Ok(())
}

/// Verifica si un nombre de archivo/directorio está en la ignore list.
///
/// Comparación case-insensitive para compatibilidad cross-platform.
fn is_ignored(name: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    IGNORED_NAMES
        .iter()
        .any(|ignored| ignored.to_ascii_lowercase() == name_lower)
}

// ─── GoToLineState ─────────────────────────────────────────────────────────────

/// Estado del modal Go to Line (Ctrl+G).
///
/// Modal pequeño que permite saltar a un número de línea específico.
/// Acepta solo dígitos (0-9) y Backspace. Enter confirma, Esc cancela.
/// El input se parsea como número 1-indexed y se clampea al rango válido.
#[derive(Debug)]
pub struct GoToLineState {
    /// Si el modal está visible.
    pub visible: bool,
    /// Dígitos que el usuario ha escrito (sin el ':').
    /// Capacidad fija de 8 — suficiente para archivos de millones de líneas.
    pub input: String,
    /// Total de líneas del archivo activo (para mostrar hint y clampear).
    pub total_lines: usize,
}

impl GoToLineState {
    /// Crea un estado inicial (modal cerrado).
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::with_capacity(8),
            total_lines: 0,
        }
    }

    /// Abre el modal con el total de líneas del archivo actual.
    pub fn open(&mut self, total_lines: usize) {
        self.visible = true;
        self.input.clear();
        self.total_lines = total_lines;
    }

    /// Cierra el modal y limpia el input.
    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    /// Agrega un dígito al input (ignora no-dígitos, máximo 6 chars).
    pub fn push_char(&mut self, ch: char) {
        if ch.is_ascii_digit() && self.input.len() < 6 {
            self.input.push(ch);
        }
    }

    /// Borra el último dígito del input.
    pub fn pop_char(&mut self) {
        self.input.pop();
    }

    /// Parsea el input como número de línea (1-indexed).
    ///
    /// Retorna `None` si vacío, cero, o inválido.
    /// Clampea al rango `[1, total_lines]`.
    pub fn parsed_line(&self) -> Option<usize> {
        let n: usize = self.input.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some(n.min(self.total_lines).max(1))
    }
}
