//! Contenido de tabs de imagen — read-only, async-decoded.
//!
//! `ImageViewContent` encapsula el estado de una imagen abierta como tab
//! virtual del editor: el protocolo pre-codificado (Kitty / Sixel / iTerm2
//! / halfblocks) que `ratatui-image` consume en cada render, el path
//! original (para mostrar en la tab bar) y un slot de error opcional para
//! mostrar fallos de decode sin paniquear.
//!
//! Características de performance:
//! - `StatefulProtocol` se construye UNA sola vez al abrir el archivo (fuera
//!   del render loop). El render solo lo muta para re-encodear en resize.
//! - No es Clone (StatefulProtocol no lo es). Drop libera el buffer.
//! - El placeholder de error se construye con un protocolo halfblocks vacío
//!   armado vía `Picker::from_fontsize` — no requiere acceso al picker
//!   real del AppState (útil cuando el decode falla muy temprano).

use std::path::PathBuf;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

/// Contenido de una tab de imagen.
///
/// Es read-only desde el lado del editor: las acciones de edición se
/// ignoran cuando `EditorState::image_view` está `Some`. Solo se permite
/// scroll/zoom (no implementados en MVP) y cierre de tab.
pub struct ImageViewContent {
    /// Path original del archivo (para tab bar y diagnósticos).
    pub path: PathBuf,
    /// Estado del protocolo de imagen — re-encodea en cada resize.
    /// Es el "state" que `StatefulImage` widget consume vía
    /// `render_stateful_widget`.
    pub protocol: StatefulProtocol,
    /// Mensaje de error si la imagen no se pudo decodificar.
    /// Cuando es `Some`, el render muestra el mensaje en lugar de la imagen.
    pub error: Option<String>,
}

impl ImageViewContent {
    /// Crea un `ImageViewContent` a partir de una imagen ya decodificada.
    ///
    /// IMPORTANTE: este método construye el protocolo (encoding inicial),
    /// lo cual puede ser CPU-bound para imágenes grandes. **Llamar
    /// exclusivamente desde `spawn_blocking`** o desde un punto donde
    /// se acepte una pausa de varios ms.
    pub fn new(path: PathBuf, img: ::image::DynamicImage, picker: &Picker) -> Self {
        // `new_resize_protocol` toma `&self` (no `&mut`), es la API estable
        // para construir un `StatefulProtocol` que auto-resize en cada render.
        let protocol = picker.new_resize_protocol(img);
        Self {
            path,
            protocol,
            error: None,
        }
    }

    /// Crea un `ImageViewContent` en estado de error.
    ///
    /// Construye un protocolo placeholder (halfblocks vacío) que nunca se
    /// renderiza — el render path detecta `error.is_some()` y muestra el
    /// mensaje en su lugar. Esto evita tener `Option<StatefulProtocol>` y
    /// simplifica el código de render.
    pub fn with_error(path: PathBuf, error: String) -> Self {
        // Picker sin query stdio: 1x1 pixel font-size es suficiente porque
        // el protocolo nunca se va a renderizar (el render branchea por error).
        let placeholder_picker = Picker::from_fontsize((1, 1));
        // Imagen 1×1 transparente como input dummy.
        let dummy = ::image::DynamicImage::new_rgba8(1, 1);
        let protocol = placeholder_picker.new_resize_protocol(dummy);
        Self {
            path,
            protocol,
            error: Some(error),
        }
    }
}

// `StatefulProtocol` no implementa Debug en v8, así que implementamos un
// Debug manual minimal para que `EditorState` (que derives Debug) compile.
impl std::fmt::Debug for ImageViewContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageViewContent")
            .field("path", &self.path)
            .field("error", &self.error)
            .field("protocol", &"<StatefulProtocol>")
            .finish()
    }
}
