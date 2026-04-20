//! Iconos de archivo por extensión — zero allocation.
//!
//! Retorna `&'static str` para cada extensión conocida. Los iconos son
//! texto ASCII de 2 caracteres para alineación consistente en terminales.
//! También provee colores semánticos por extensión para complementar
//! el theme cyberpunk.
//!
//! Alternativa emoji disponible pero desactivada por defecto — los emojis
//! tienen ancho inconsistente en Windows Terminal y causan desalineación.

use ratatui::style::Color;

/// Retorna un icono (2-char ASCII) para un nombre de archivo según su extensión.
///
/// Mapeo estático — no aloca. El icono siempre ocupa exactamente 2 celdas
/// de ancho en la terminal para alineación predecible.
pub fn file_icon(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("");
    // Si el filename no tiene extensión (ext == filename), usar default
    if ext == filename {
        return match filename {
            "Dockerfile" | "dockerfile" => "Dk",
            "Makefile" | "makefile" => "Mk",
            "Justfile" | "justfile" => "Jf",
            _ => "..",
        };
    }
    match ext {
        // Rust
        "rs" => "Rs",
        // Config
        "toml" => "Cf",
        "yaml" | "yml" => "Ym",
        "json" => "Js",
        // Web
        "ts" | "tsx" => "Ts",
        "js" | "jsx" => "Js",
        "html" => "Ht",
        "css" | "scss" | "sass" => "Cs",
        // Languages
        "py" => "Py",
        "go" => "Go",
        "lua" => "Lu",
        "rb" => "Rb",
        "java" => "Jv",
        "c" | "h" => "C ",
        "cpp" | "hpp" | "cc" => "C+",
        "cs" => "C#",
        // Data
        "md" => "Md",
        "txt" => "Tx",
        "sql" => "Sq",
        "xml" => "Xm",
        "csv" => "Cv",
        // Shell
        "sh" | "bash" | "zsh" => "Sh",
        "ps1" | "psm1" => "Ps",
        "bat" | "cmd" => ">_",
        // Config files
        "env" => "En",
        "gitignore" | "dockerignore" => "Ig",
        "lock" => "Lk",
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" => "Im",
        // Default
        _ => "..",
    }
}

/// Retorna un icono para directorios según estado de expansión.
///
/// No aloca — retorna literal estático de 2 caracteres.
pub fn dir_icon(expanded: bool) -> &'static str {
    if expanded {
        "v "
    } else {
        "> "
    }
}

/// Retorna el color sugerido para el icono según extensión del archivo.
///
/// Colores elegidos para complementar el theme cyberpunk:
/// - Rust → naranja (identidad del lenguaje)
/// - Python → amarillo
/// - TypeScript/JavaScript → cyan/azul
/// - Config → gris claro
/// - Data/docs → verde suave
/// - Shell → magenta
/// - Default → fg_secondary (dimmed)
pub fn icon_color(filename: &str) -> Color {
    let ext = filename.rsplit('.').next().unwrap_or("");
    // Sin extensión — verificar nombres especiales
    if ext == filename {
        return match filename {
            "Dockerfile" | "dockerfile" => Color::Rgb(0, 150, 255), // azul Docker
            "Makefile" | "makefile" | "Justfile" | "justfile" => Color::Rgb(180, 180, 180),
            _ => Color::Rgb(106, 115, 125), // fg_secondary
        };
    }
    match ext {
        // Rust — naranja cálido
        "rs" => Color::Rgb(255, 140, 50),
        // Config — gris claro
        "toml" | "yaml" | "yml" | "json" => Color::Rgb(180, 180, 180),
        // TypeScript — azul
        "ts" | "tsx" => Color::Rgb(0, 122, 204),
        // JavaScript — amarillo
        "js" | "jsx" => Color::Rgb(241, 224, 90),
        // HTML — naranja
        "html" => Color::Rgb(227, 76, 38),
        // CSS — azul/cyan
        "css" | "scss" | "sass" => Color::Rgb(86, 156, 214),
        // Python — amarillo/verde
        "py" => Color::Rgb(55, 118, 171),
        // Go — cyan
        "go" => Color::Rgb(0, 173, 216),
        // Lua — azul oscuro
        "lua" => Color::Rgb(0, 0, 200),
        // Ruby — rojo
        "rb" => Color::Rgb(204, 52, 45),
        // Java — rojo/naranja
        "java" => Color::Rgb(176, 114, 25),
        // C/C++ — azul
        "c" | "h" | "cpp" | "hpp" | "cc" => Color::Rgb(0, 89, 156),
        // C# — morado
        "cs" => Color::Rgb(104, 33, 122),
        // Markdown — verde suave
        "md" => Color::Rgb(83, 154, 111),
        // Text — gris
        "txt" => Color::Rgb(150, 150, 150),
        // SQL — naranja suave
        "sql" => Color::Rgb(226, 131, 68),
        // XML — naranja
        "xml" => Color::Rgb(227, 119, 44),
        // CSV — verde
        "csv" => Color::Rgb(63, 185, 80),
        // Shell — magenta
        "sh" | "bash" | "zsh" => Color::Rgb(200, 100, 200),
        "ps1" | "psm1" => Color::Rgb(0, 122, 204),
        "bat" | "cmd" => Color::Rgb(180, 180, 180),
        // Config files
        "env" | "lock" => Color::Rgb(150, 150, 100),
        "gitignore" | "dockerignore" => Color::Rgb(120, 120, 120),
        // Images — rosa
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" => Color::Rgb(200, 130, 200),
        // Default — dimmed
        _ => Color::Rgb(106, 115, 125),
    }
}

/// Color para iconos de directorio.
///
/// Usar el accent del theme para directorios — los hace visualmente
/// distintos de los archivos en el explorer.
pub fn dir_icon_color() -> Color {
    // fg_accent — cyan eléctrico del theme cyberpunk
    Color::Rgb(0, 212, 255)
}
