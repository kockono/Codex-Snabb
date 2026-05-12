//! Rename modal: input de nombre para renombrar archivos/directorios en el explorer.
//!
//! Se abre cuando el usuario selecciona "Rename" en el context menu del explorer.
//! El input se pre-carga con el nombre actual del archivo (solo el nombre, sin path).
//! Valida que el nombre no esté vacío, no contenga separadores de path, y que el
//! destino no exista antes de llamar a `std::fs::rename`.

use std::path::PathBuf;

/// Estado del modal "Rename".
///
/// Contiene el path original (para construir el nuevo path), el input editado
/// por el usuario (solo el nombre de archivo, sin directorio), el error efímero
/// y la visibilidad del modal.
#[derive(Debug)]
pub struct RenameState {
    /// Si el modal está visible.
    pub visible: bool,
    /// Path completo del archivo/directorio siendo renombrado.
    pub original_path: Option<PathBuf>,
    /// Nombre editado por el usuario (solo el filename, sin directorio).
    pub input: String,
    /// Error efímero (destino existe, nombre inválido, error de fs, etc.).
    pub error: Option<String>,
    /// Countdown de ticks para limpiar `error`. 40 ticks ≈ 2s a 20 FPS.
    pub error_ticks: u8,
}

impl RenameState {
    /// Crea un nuevo estado inicial (invisible, sin input).
    pub fn new() -> Self {
        Self {
            visible: false,
            original_path: None,
            input: String::new(),
            error: None,
            error_ticks: 0,
        }
    }

    /// Abre el modal pre-cargando el nombre actual del archivo.
    ///
    /// `path` debe ser el path completo del archivo/directorio.
    /// El input se inicializa con solo el filename (no el directorio completo).
    pub fn open(&mut self, path: PathBuf) {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        self.input = filename;
        self.original_path = Some(path);
        self.visible = true;
        self.error = None;
        self.error_ticks = 0;
    }

    /// Cierra el modal sin realizar el rename.
    pub fn close(&mut self) {
        self.visible = false;
        self.original_path = None;
        self.input.clear();
        self.error = None;
        self.error_ticks = 0;
    }

    /// Agrega un carácter al input de nombre.
    pub fn push_char(&mut self, ch: char) {
        self.input.push(ch);
    }

    /// Elimina el último carácter del input de nombre.
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Decrementa el countdown del error efímero.
    ///
    /// Limpia `error` cuando llega a 0. Llamar en cada tick.
    pub fn tick_error(&mut self) {
        if self.error_ticks > 0 {
            self.error_ticks -= 1;
            if self.error_ticks == 0 {
                self.error = None;
            }
        }
    }

    /// Valida el input y ejecuta el rename en el filesystem.
    ///
    /// Reglas de validación:
    /// - El nombre no puede estar vacío.
    /// - El nombre no puede contener separadores de path (`/` o `\`).
    /// - El archivo destino no debe existir ya.
    ///
    /// Si la validación falla: retorna `Err(msg)` con el mensaje de error.
    /// Si el rename del filesystem falla: retorna `Err(msg)`.
    /// Si tiene éxito: cierra el modal y retorna `Ok(new_path)`.
    pub fn confirm(&mut self) -> Result<PathBuf, String> {
        if self.input.is_empty() {
            return Err("El nombre no puede estar vacío".into());
        }
        // No se permiten separadores de path — el rename es solo del filename
        if self.input.contains('/') || self.input.contains('\\') {
            return Err("El nombre no puede contener / o \\".into());
        }
        let original = self
            .original_path
            .as_ref()
            .ok_or_else(|| "Sin archivo seleccionado".to_owned())?;
        let parent = original
            .parent()
            .ok_or_else(|| "No se puede determinar el directorio padre".to_owned())?;
        let new_path = parent.join(&self.input);
        if new_path.exists() {
            return Err("Ya existe un archivo con ese nombre".into());
        }
        std::fs::rename(original, &new_path).map_err(|e| format!("Error al renombrar: {e}"))?;
        self.close();
        Ok(new_path)
    }
}

impl Default for RenameState {
    fn default() -> Self {
        Self::new()
    }
}
