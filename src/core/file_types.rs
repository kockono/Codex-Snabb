//! Detección de tipo de archivo por extensión.
//!
//! Funciones zero-allocation para clasificar archivos según su extensión
//! (case-insensitive). Se usan en el reducer para decidir si un archivo
//! debe abrirse como texto (`EditorState`) o como imagen (`ImageViewContent`).

use std::path::Path;

/// Extensiones de imagen soportadas (sin punto, lowercase).
///
/// Si agregás una extensión acá, asegurate que el feature correspondiente
/// del crate `image` esté habilitado en `Cargo.toml`.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

/// Retorna `true` si el path tiene extensión de imagen soportada.
///
/// Comparación case-insensitive sobre la extensión solamente.
/// La única alocación es `to_ascii_lowercase()` sobre un slice corto
/// (típicamente 3-4 chars), por lo que el costo es despreciable.
///
/// # Ejemplos
///
/// ```ignore
/// use std::path::Path;
/// use crate::core::file_types::is_image_file;
///
/// assert!(is_image_file(Path::new("photo.JPG")));
/// assert!(is_image_file(Path::new("/tmp/x.png")));
/// assert!(!is_image_file(Path::new("main.rs")));
/// assert!(!is_image_file(Path::new("README")));
/// ```
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            // to_ascii_lowercase aloca un String, pero solo para la extensión.
            // Es la opción correcta para case-insensitive sin depender de unicode.
            let lower = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&lower.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_lowercase_image_extensions() {
        for ext in ["jpg", "jpeg", "png", "gif", "webp", "bmp"] {
            let p = format!("file.{ext}");
            assert!(
                is_image_file(Path::new(&p)),
                "esperaba que {p} sea imagen"
            );
        }
    }

    #[test]
    fn detects_uppercase_image_extensions() {
        for ext in ["JPG", "JPEG", "PNG", "GIF", "WEBP", "BMP"] {
            let p = format!("file.{ext}");
            assert!(
                is_image_file(Path::new(&p)),
                "esperaba que {p} sea imagen (case-insensitive)"
            );
        }
    }

    #[test]
    fn detects_mixed_case() {
        assert!(is_image_file(Path::new("Photo.PnG")));
        assert!(is_image_file(Path::new("a.WebP")));
    }

    #[test]
    fn rejects_non_image_extensions() {
        for p in ["main.rs", "README", "data.json", "x.txt", "y.svg", "z.tiff"] {
            assert!(!is_image_file(Path::new(p)), "no esperaba que {p} sea imagen");
        }
    }

    #[test]
    fn rejects_path_without_extension() {
        assert!(!is_image_file(Path::new("Makefile")));
        assert!(!is_image_file(Path::new("/tmp/.bashrc")));
    }
}
