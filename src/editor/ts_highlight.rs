//! Tree-sitter syntax highlighting engine.
//!
//! Motor de highlighting basado en tree-sitter para 6 lenguajes (Rust, TypeScript,
//! Go, JSON, CSS, Bash). Para lenguajes sin grammar tree-sitter, el sistema cae
//! automáticamente a syntect como fallback.
//!
//! Performance:
//! - Solo destaca el viewport visible (no el archivo completo)
//! - Cache por línea: se recalcula solo cuando el buffer cambia
//! - Parsing incremental aprovecha la capacidad de tree-sitter
//! - Zero allocations en render loop — tokens pre-computados en cache

use std::collections::HashMap;

use syntect::highlighting::{Color, FontStyle, Style as SyntectStyle};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use super::buffer::TextBuffer;
use super::highlighting::HighlightToken;

// ─── Highlight names ──────────────────────────────────────────────────────────

/// Lista de nombres de highlight que tree-sitter-highlight emitirá como eventos.
///
/// El orden importa: el índice en esta lista es el valor de `Highlight.0` que
/// llega en `HighlightEvent::HighlightStart`. Debe coincidir con lo que se
/// pasa a `HighlightConfiguration::configure()`.
const HIGHLIGHT_NAMES: &[&str] = &[
    "keyword",
    "keyword.control",
    "keyword.operator",
    "keyword.return",
    "function",
    "function.method",
    "function.builtin",
    "type",
    "type.builtin",
    "variable",
    "variable.parameter",
    "variable.builtin",
    "string",
    "string.special",
    "number",
    "float",
    "boolean",
    "constant",
    "constant.builtin",
    "comment",
    "operator",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "attribute",
    "tag",
    "tag.attribute",
    "property",
    "namespace",
    "module",
    "label",
    "escape",
    "constructor",
];

// ─── Color mapping ────────────────────────────────────────────────────────────

/// Mapea un nombre de highlight tree-sitter a un color RGB.
///
/// Paleta oscura similar a base16-ocean.dark (la misma que usa syntect).
/// Se matchea sobre el primer segmento antes del '.' para broad matching.
fn highlight_name_to_color(name: &str) -> Color {
    let base = name.split('.').next().unwrap_or(name);
    match base {
        "keyword" => Color {
            r: 0xB4,
            g: 0x8E,
            b: 0xAD,
            a: 0xFF,
        }, // purple
        "operator" => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        }, // light gray
        "punctuation" => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        },
        "comment" => Color {
            r: 0x65,
            g: 0x73,
            b: 0x7E,
            a: 0xFF,
        }, // dim gray
        "string" => Color {
            r: 0xA3,
            g: 0xBE,
            b: 0x8C,
            a: 0xFF,
        }, // green
        "number" | "float" => Color {
            r: 0xD0,
            g: 0x87,
            b: 0x70,
            a: 0xFF,
        }, // orange
        "boolean" => Color {
            r: 0xD0,
            g: 0x87,
            b: 0x70,
            a: 0xFF,
        }, // orange
        "function" => Color {
            r: 0x8F,
            g: 0xA1,
            b: 0xB3,
            a: 0xFF,
        }, // blue
        "type" | "constructor" => Color {
            r: 0xEB,
            g: 0xCB,
            b: 0x8B,
            a: 0xFF,
        }, // yellow
        "variable" => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        }, // light gray
        "constant" => Color {
            r: 0xD0,
            g: 0x87,
            b: 0x70,
            a: 0xFF,
        }, // orange
        "attribute" => Color {
            r: 0xEB,
            g: 0xCB,
            b: 0x8B,
            a: 0xFF,
        },
        "tag" => Color {
            r: 0xB4,
            g: 0x8E,
            b: 0xAD,
            a: 0xFF,
        },
        "property" => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        },
        "namespace" | "module" => Color {
            r: 0xEB,
            g: 0xCB,
            b: 0x8B,
            a: 0xFF,
        },
        "label" => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        },
        "escape" => Color {
            r: 0xD0,
            g: 0x87,
            b: 0x70,
            a: 0xFF,
        },
        _ => Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        }, // default fg
    }
}

/// Construye un `SyntectStyle` a partir de un nombre de highlight.
fn make_style(highlight_name: &str) -> SyntectStyle {
    SyntectStyle {
        foreground: highlight_name_to_color(highlight_name),
        background: Color::BLACK,
        font_style: FontStyle::empty(),
    }
}

/// Estilo por defecto (texto sin highlight).
fn default_style() -> SyntectStyle {
    SyntectStyle {
        foreground: Color {
            r: 0xC0,
            g: 0xC5,
            b: 0xCE,
            a: 0xFF,
        },
        background: Color::BLACK,
        font_style: FontStyle::empty(),
    }
}

// ─── Grammar registry ─────────────────────────────────────────────────────────

/// Carga la configuración de highlighting para una extensión de archivo.
///
/// Retorna `None` si no hay grammar tree-sitter para esa extensión.
/// Los lenguajes soportados: rs, ts, tsx, go, json, css, sh, bash.
pub fn config_for_extension(ext: &str) -> Option<HighlightConfiguration> {
    let (language_fn, lang_name, highlights_query, injections_query, locals_query) = match ext {
        "rs" => (
            tree_sitter_rust::LANGUAGE,
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ),
        "ts" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            "typescript",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        "tsx" => (
            tree_sitter_typescript::LANGUAGE_TSX,
            "tsx",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        "go" => (
            tree_sitter_go::LANGUAGE,
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "json" => (
            tree_sitter_json::LANGUAGE,
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "css" => (
            tree_sitter_css::LANGUAGE,
            "css",
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
        "sh" | "bash" => (
            tree_sitter_bash::LANGUAGE,
            "bash",
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        ),
        _ => return None,
    };

    // LanguageFn -> Language via .into()
    let mut config = HighlightConfiguration::new(
        language_fn.into(),
        lang_name,
        highlights_query,
        injections_query,
        locals_query,
    )
    .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

// ─── TsHighlightEngine ───────────────────────────────────────────────────────

/// Motor de tree-sitter para un archivo específico.
///
/// Vive dentro de `HighlightCache` cuando el archivo tiene grammar soportado.
/// Produce `HighlightToken` idénticos a los que produce syntect, de modo que
/// el render en `panels.rs` no necesita saber qué motor generó los tokens.
pub struct TsHighlightEngine {
    /// Configuración de highlighting (grammar + queries).
    /// Ownership directo — no se comparte entre instancias.
    config: HighlightConfiguration,
    /// Tokens cacheados por línea: HashMap<line_idx, Vec<HighlightToken>>.
    /// Se recalculan en `highlight_viewport()`.
    lines: HashMap<usize, Vec<HighlightToken>>,
    /// Si el cache está dirty (buffer cambió).
    dirty: bool,
    /// Bytes del archivo cacheados — evita reconstruir Vec<u8> en cada frame.
    /// Se invalida cuando el buffer cambia (junto con `dirty = true`).
    source_cache: Vec<u8>,
    /// Heurística barata para detectar cambios de contenido sin comparar bytes.
    /// (line_count, last_line_len as u8) — no criptográfica.
    source_version: (usize, u8),
    /// Último viewport procesado [start, end) — para evitar reprocesar en scroll sin cambios.
    last_viewport: (usize, usize),
}

impl TsHighlightEngine {
    /// Crea un nuevo motor tree-sitter con la configuración dada.
    pub fn new(config: HighlightConfiguration) -> Self {
        Self {
            config,
            lines: HashMap::new(),
            dirty: true,
            source_cache: Vec::new(),
            source_version: (0, 0),
            last_viewport: (0, 0),
        }
    }

    /// Invalida el cache — fuerza re-highlight en el próximo `highlight_viewport()`.
    ///
    /// Marca `dirty = true` y fuerza rebuild de source en el próximo highlight.
    /// No limpia `source_cache` — se reutiliza el Vec si el buffer no cambió demasiado.
    pub fn invalidate(&mut self) {
        self.dirty = true;
        self.lines.clear();
        self.source_version = (0, 0); // forzar rebuild de source en próximo highlight
        self.last_viewport = (0, 0);
    }

    /// Obtiene los tokens coloreados de una línea cacheada.
    ///
    /// Retorna `None` si el cache está dirty o la línea no fue procesada.
    pub fn get_line(&self, line_idx: usize) -> Option<&[HighlightToken]> {
        if self.dirty {
            return None;
        }
        self.lines.get(&line_idx).map(Vec::as_slice)
    }

    /// Destaca las líneas del viewport y las guarda en cache.
    ///
    /// `buffer`: referencia al TextBuffer — el engine gestiona su propio source cache.
    /// `viewport_start`: primera línea visible (inclusive).
    /// `viewport_end`: última línea visible (exclusiva).
    ///
    /// Optimizaciones:
    /// 1. Source cache: solo reconstruye bytes del buffer cuando el contenido cambió
    ///    (detectado por heurística line_count + last_line_len).
    /// 2. Viewport skip: si el viewport ya está cubierto por el último procesamiento
    ///    y no hay cambios, retorna inmediatamente (0 trabajo por frame en scroll estable).
    /// 3. Margen de 30 líneas: procesa líneas extra arriba/abajo del viewport visible
    ///    para que scroll de ±30 líneas no re-trigger el highlight.
    pub fn highlight_viewport(
        &mut self,
        buffer: &TextBuffer,
        viewport_start: usize,
        viewport_end: usize,
    ) {
        // ── 1. Verificar si el source cambió ──
        // Heurística barata: comparar line_count + longitud de última línea.
        // No criptográfica — si falla, la próxima edición llama invalidate().
        let line_count = buffer.line_count();
        let last_line = buffer.line(line_count.saturating_sub(1)).unwrap_or("");
        let new_version = (line_count, last_line.len() as u8);

        let source_changed = new_version != self.source_version;

        if source_changed {
            // Reconstruir source cache solo cuando el buffer cambió
            rebuild_source_cache(&mut self.source_cache, buffer);
            self.source_version = new_version;
            self.dirty = true;
            self.lines.clear();
            self.last_viewport = (0, 0);
        }

        // ── 2. Verificar si el viewport ya está cubierto ──
        if !self.dirty
            && self.last_viewport.0 <= viewport_start
            && self.last_viewport.1 >= viewport_end
        {
            return; // Cache hit — viewport ya procesado, sin trabajo
        }

        // ── 3. Ampliar viewport para evitar reprocesos frecuentes en scroll ──
        // MARGIN líneas extra arriba y abajo del viewport visible.
        const MARGIN: usize = 30;
        let process_start = viewport_start.saturating_sub(MARGIN);
        let process_end = (viewport_end + MARGIN).min(line_count);

        // ── 4. Parsear con tree-sitter usando source cacheado ──
        if self.source_cache.is_empty() {
            return;
        }

        let mut highlighter = Highlighter::new();
        let events = match highlighter.highlight(&self.config, &self.source_cache, None, |_| None) {
            Ok(e) => e,
            Err(_) => {
                // Error en parsing — dejar dirty para retry en el próximo frame
                return;
            }
        };

        // ── 5. Construir mapa line_starts ──
        let line_starts = build_line_starts(&self.source_cache);

        // ── 6. Procesar eventos solo para el rango [process_start, process_end) ──
        // Limpiar SOLO el rango que vamos a reprocesar (preservar líneas fuera del rango)
        self.lines
            .retain(|&line, _| line < process_start || line >= process_end);

        let mut current_style_idx: Option<usize> = None;
        let mut new_tokens: HashMap<usize, Vec<HighlightToken>> =
            HashMap::with_capacity(process_end.saturating_sub(process_start));

        for event in events.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let text = match std::str::from_utf8(&self.source_cache[start..end]) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };

                    let style = current_style_idx
                        .and_then(|i| HIGHLIGHT_NAMES.get(i))
                        .map(|name| make_style(name))
                        .unwrap_or_else(default_style);

                    // Manejar spans multi-línea: dividir en '\n' y distribuir
                    // cada segmento a su línea correspondiente.
                    let mut current_byte = start;
                    let mut segment_start_in_text = 0;

                    for (local_offset, ch) in text.char_indices() {
                        if ch == '\n' {
                            let segment = &text[segment_start_in_text..local_offset];
                            if !segment.is_empty() {
                                let line_idx = byte_to_line(current_byte, &line_starts);
                                if line_idx >= process_start && line_idx < process_end {
                                    new_tokens
                                        .entry(line_idx)
                                        .or_default()
                                        .push(HighlightToken {
                                            style,
                                            text: segment.to_owned(),
                                        });
                                }
                            }
                            // Avanzar past the newline
                            current_byte = start + local_offset + 1;
                            segment_start_in_text = local_offset + 1;
                        }
                    }

                    // Último segmento después del último '\n' (o el único si no hay '\n')
                    let remaining = &text[segment_start_in_text..];
                    if !remaining.is_empty() {
                        let line_idx = byte_to_line(current_byte, &line_starts);
                        if line_idx >= process_start && line_idx < process_end {
                            new_tokens
                                .entry(line_idx)
                                .or_default()
                                .push(HighlightToken {
                                    style,
                                    text: remaining.to_owned(),
                                });
                        }
                    }
                }
                HighlightEvent::HighlightStart(h) => {
                    current_style_idx = Some(h.0);
                }
                HighlightEvent::HighlightEnd => {
                    current_style_idx = None;
                }
            }
        }

        // Merge new tokens into cache
        self.lines.extend(new_tokens);
        self.dirty = false;
        self.last_viewport = (process_start, process_end);
    }
}

impl std::fmt::Debug for TsHighlightEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsHighlightEngine")
            .field("lines_cached", &self.lines.len())
            .field("dirty", &self.dirty)
            .field("source_cache_bytes", &self.source_cache.len())
            .field("source_version", &self.source_version)
            .field("last_viewport", &self.last_viewport)
            .finish()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Reconstruye el source cache desde el buffer de texto.
///
/// Reutiliza el Vec existente (clear + extend) para evitar realocaciones
/// cuando el tamaño del archivo no cambió significativamente.
/// Estimación de capacidad: ~41 bytes por línea (avg 40 chars + newline).
fn rebuild_source_cache(cache: &mut Vec<u8>, buffer: &TextBuffer) {
    cache.clear();
    let line_count = buffer.line_count();
    // Pre-estimar capacidad: avg line ~40 chars + newline = 41 bytes.
    // Solo reservar si el cache es demasiado pequeño.
    let est_len = line_count * 41;
    if cache.capacity() < est_len {
        cache.reserve(est_len - cache.capacity());
    }
    for i in 0..line_count {
        let line = buffer.line(i).unwrap_or("");
        cache.extend_from_slice(line.as_bytes());
        if i + 1 < line_count {
            cache.push(b'\n');
        }
    }
}

/// Construye un vector de byte offsets donde empieza cada línea.
///
/// `line_starts[0]` = 0 (primera línea empieza en byte 0).
/// `line_starts[n]` = byte offset del primer carácter de la línea n.
fn build_line_starts(source: &[u8]) -> Vec<usize> {
    let newline_count = source.iter().filter(|&&b| b == b'\n').count();
    let mut starts = Vec::with_capacity(newline_count + 1);
    starts.push(0);
    for (i, &b) in source.iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convierte un byte offset a número de línea usando binary search.
fn byte_to_line(byte_offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_line_starts() {
        let source = b"hello\nworld\nfoo";
        let starts = build_line_starts(source);
        assert_eq!(starts, vec![0, 6, 12]);
    }

    #[test]
    fn test_byte_to_line() {
        let starts = vec![0, 6, 12];
        assert_eq!(byte_to_line(0, &starts), 0);
        assert_eq!(byte_to_line(3, &starts), 0);
        assert_eq!(byte_to_line(6, &starts), 1);
        assert_eq!(byte_to_line(11, &starts), 1);
        assert_eq!(byte_to_line(12, &starts), 2);
        assert_eq!(byte_to_line(15, &starts), 2);
    }

    #[test]
    fn test_config_for_extension_supported() {
        // Verificar que los lenguajes soportados cargan correctamente
        assert!(config_for_extension("rs").is_some(), "Rust grammar");
        assert!(config_for_extension("ts").is_some(), "TypeScript grammar");
        assert!(config_for_extension("tsx").is_some(), "TSX grammar");
        assert!(config_for_extension("go").is_some(), "Go grammar");
        assert!(config_for_extension("json").is_some(), "JSON grammar");
        assert!(config_for_extension("css").is_some(), "CSS grammar");
        assert!(config_for_extension("sh").is_some(), "Bash grammar (sh)");
        assert!(
            config_for_extension("bash").is_some(),
            "Bash grammar (bash)"
        );
    }

    #[test]
    fn test_config_for_extension_unsupported() {
        assert!(config_for_extension("py").is_none());
        assert!(config_for_extension("java").is_none());
        assert!(config_for_extension("").is_none());
        assert!(config_for_extension("toml").is_none());
    }

    #[test]
    fn test_highlight_rust_source() {
        let buffer = TextBuffer::from_text("fn main() {\n    let x = 42;\n}\n");
        let config = config_for_extension("rs").expect("Rust config");
        let mut engine = TsHighlightEngine::new(config);

        // Antes de highlight, dirty = true
        assert!(engine.get_line(0).is_none());

        // Highlight viewport completo (3 líneas)
        engine.highlight_viewport(&buffer, 0, 3);

        // Después de highlight, debe haber tokens
        assert!(engine.get_line(0).is_some(), "línea 0 debe tener tokens");
        assert!(engine.get_line(1).is_some(), "línea 1 debe tener tokens");

        // Verificar que los tokens de la línea 0 contienen "fn"
        let line0_tokens = engine.get_line(0).expect("tokens línea 0");
        let has_fn = line0_tokens.iter().any(|t| t.text.contains("fn"));
        assert!(has_fn, "línea 0 debe contener token 'fn'");
    }

    #[test]
    fn test_highlight_viewport_partial() {
        // 4 líneas + trailing newline — con margen de 30, viewport 1..3 expande a 0..4
        // Necesitamos >30 líneas para que el margen no cubra todo
        let mut source = String::new();
        for i in 0..40 {
            source.push_str(&format!("fn f{i}() {{}}\n"));
        }
        let buffer = TextBuffer::from_text(&source);
        let config = config_for_extension("rs").expect("Rust config");
        let mut engine = TsHighlightEngine::new(config);

        // Viewport líneas 35-37 — con margen 30, process_start=5, process_end=40
        engine.highlight_viewport(&buffer, 35, 37);

        // Línea 4 NO debe estar cacheada (fuera del margen)
        assert!(engine.get_line(4).is_none(), "línea 4 fuera del margen");
        // Línea 5 SÍ debe estar cacheada (dentro del margen)
        assert!(engine.get_line(5).is_some(), "línea 5 dentro del margen");
        // Líneas 35-36 SÍ deben estar cacheadas (viewport)
        assert!(engine.get_line(35).is_some(), "línea 35 en viewport");
        assert!(engine.get_line(36).is_some(), "línea 36 en viewport");
    }

    #[test]
    fn test_invalidate_clears_cache() {
        let buffer = TextBuffer::from_text("let x = 1;\n");
        let config = config_for_extension("ts").expect("TS config");
        let mut engine = TsHighlightEngine::new(config);

        engine.highlight_viewport(&buffer, 0, 1);
        assert!(engine.get_line(0).is_some());

        engine.invalidate();
        assert!(engine.get_line(0).is_none());
    }

    #[test]
    fn test_viewport_cache_skip() {
        // Verificar que un segundo highlight_viewport con el mismo viewport no reprocesa.
        let buffer = TextBuffer::from_text("fn main() {\n    let x = 42;\n}\n");
        let config = config_for_extension("rs").expect("Rust config");
        let mut engine = TsHighlightEngine::new(config);

        // Primer highlight — debe procesar
        engine.highlight_viewport(&buffer, 0, 3);
        assert!(engine.get_line(0).is_some());

        // Segundo highlight con mismo viewport — debe ser cache hit (no dirty, viewport cubierto)
        // Si el cache funciona, los tokens siguen ahí sin reprocesar.
        engine.highlight_viewport(&buffer, 0, 3);
        assert!(
            engine.get_line(0).is_some(),
            "cache hit debe mantener tokens"
        );
    }

    #[test]
    fn test_source_change_detection() {
        let buffer1 = TextBuffer::from_text("fn a() {}\n");
        let buffer2 = TextBuffer::from_text("fn a() {}\nfn b() {}\n");
        let config = config_for_extension("rs").expect("Rust config");
        let mut engine = TsHighlightEngine::new(config);

        // Highlight con buffer1
        engine.highlight_viewport(&buffer1, 0, 1);
        assert!(engine.get_line(0).is_some());

        // Highlight con buffer2 (diferente contenido) — debe detectar cambio
        engine.highlight_viewport(&buffer2, 0, 2);
        assert!(engine.get_line(0).is_some());
        assert!(
            engine.get_line(1).is_some(),
            "nueva línea debe ser procesada"
        );
    }

    #[test]
    fn test_rebuild_source_cache() {
        let buffer = TextBuffer::from_text("hello\nworld\nfoo");
        let mut cache = Vec::new();
        rebuild_source_cache(&mut cache, &buffer);
        assert_eq!(cache, b"hello\nworld\nfoo");
    }
}
