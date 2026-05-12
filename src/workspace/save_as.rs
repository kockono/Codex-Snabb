//! Save As modal: input de path para guardar buffers sin archivo asociado.
//!
//! Se abre cuando Ctrl+S se presiona en un buffer `[untitled]` o cuando
//! se intenta cerrar un buffer dirty sin path asignado.
//! Reutiliza la misma estética que el folder picker pero sin árbol de directorios.

use std::path::PathBuf;

/// Estado del modal "Guardar como".
///
/// Contiene el input de path, error efímero y visibilidad.
/// No tiene árbol de directorios — solo input libre de path.
#[derive(Debug)]
pub struct SaveAsState {
    /// Si el modal está visible.
    pub visible: bool,
    /// Path escrito por el usuario en el input.
    pub path_input: String,
    /// Error efímero (path inválido, directorio inexistente, etc.).
    pub error: Option<String>,
    /// Countdown de ticks para limpiar `error`. 40 ticks ≈ 2s a 20 FPS.
    pub error_ticks: u8,
}

impl SaveAsState {
    /// Crea un nuevo estado inicial (invisible, sin input).
    pub fn new() -> Self {
        Self {
            visible: false,
            path_input: String::new(),
            error: None,
            error_ticks: 0,
        }
    }

    /// Abre el modal pre-cargando el path inicial (workspace root + separador).
    ///
    /// Si `initial_dir` es `Some(path)`, el input arranca con esa ruta ya escrita
    /// para que el usuario solo tenga que añadir el nombre del archivo.
    pub fn open(&mut self, initial_dir: Option<&std::path::Path>) {
        self.visible = true;
        self.path_input.clear();
        if let Some(s) = initial_dir.and_then(|d| d.to_str()) {
            self.path_input.push_str(s);
            // Agregar separador de path al final si no lo tiene
            let sep = std::path::MAIN_SEPARATOR;
            if !self.path_input.ends_with(sep) {
                self.path_input.push(sep);
            }
        }
        self.error = None;
        self.error_ticks = 0;
    }

    /// Cierra el modal sin guardar.
    pub fn close(&mut self) {
        self.visible = false;
        self.path_input.clear();
        self.error = None;
        self.error_ticks = 0;
    }

    /// Agrega un carácter al input de path.
    pub fn push_char(&mut self, ch: char) {
        self.path_input.push(ch);
    }

    /// Elimina el último carácter del input de path.
    pub fn backspace(&mut self) {
        self.path_input.pop();
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

    /// Valida el input y retorna el `PathBuf` confirmado si es válido.
    ///
    /// Reglas de validación:
    /// - El input no puede estar vacío.
    /// - El directorio padre debe existir (o ser raíz/vacío para paths relativos simples).
    ///
    /// Si la validación falla: setea `error` + `error_ticks`, retorna `None`.
    /// Si la validación pasa: cierra el modal y retorna `Some(path)`.
    pub fn confirm(&mut self) -> Option<PathBuf> {
        if self.path_input.is_empty() {
            self.error = Some("Escribí un path".into());
            self.error_ticks = 40;
            return None;
        }

        let p = PathBuf::from(&self.path_input);

        // El directorio padre debe existir (o ser "" para paths relativos al cwd).
        let parent_ok = p
            .parent()
            .map(|par| par == std::path::Path::new("") || par.exists())
            .unwrap_or(true);

        if !parent_ok {
            self.error = Some("Directorio no existe".into());
            self.error_ticks = 40;
            return None;
        }

        self.visible = false;
        self.path_input.clear();
        self.error = None;
        self.error_ticks = 0;
        Some(p)
    }
}

impl Default for SaveAsState {
    fn default() -> Self {
        Self::new()
    }
}
