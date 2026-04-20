//! Highlighting: motor de syntax highlighting con syntect.
//!
//! Usa los ~50 grammars embebidos de syntect para colorear código.
//! `HighlightEngine` se carga UNA VEZ en `AppState` (~2MB inmutable).
//! `HighlightCache` vive en cada `EditorState` y cachea los tokens
//! coloreados por línea. Se invalida cuando el buffer cambia (dirty).
//!
//! Performance:
//! - `SyntaxSet` y theme: carga única al inicio
//! - Re-highlight completo en invalidación (OK para MVP, archivos < 10k líneas)
//! - Cache: Vec<Vec<(Style, String)>> — una entrada por línea
//! - NUNCA highlight en el render loop — siempre cachear antes

use std::path::Path;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, Theme as SyntectTheme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use super::buffer::TextBuffer;

// ─── HighlightEngine ──────────────────────────────────────────────────────────

/// Motor de syntax highlighting — singleton en `AppState`.
///
/// Contiene `SyntaxSet` (~2MB) y un theme oscuro. Inmutable después
/// de la construcción — se pasa por `&HighlightEngine` a los editores.
#[derive(Debug)]
pub struct HighlightEngine {
    /// Grammars embebidos de syntect (Rust, Python, JS, etc.).
    syntax_set: SyntaxSet,
    /// Theme para colorear tokens.
    theme: SyntectTheme,
}

impl HighlightEngine {
    /// Crea el motor con syntaxes default y un theme oscuro.
    ///
    /// Usa `base16-ocean.dark` — combina bien con la paleta cyberpunk
    /// del IDE (fondos oscuros, texto claro, acentos brillantes).
    pub fn new() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        // base16-ocean.dark: paleta oscura con buenos contrastes
        // Fallback a Monokai si no existe (ambos vienen con syntect)
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.get("base16-mocha.dark"))
            .cloned() // CLONE: necesario — ThemeSet owns los themes, necesitamos mover uno
            .unwrap_or_else(|| {
                // Si ningún theme existe (imposible con defaults), usar el primero
                theme_set
                    .themes
                    .into_values()
                    .next()
                    .expect("syntect debe tener al menos un theme embebido")
            });
        Self { syntax_set, theme }
    }

    /// Detecta la syntax por extensión del archivo.
    ///
    /// Retorna `None` si la extensión no tiene grammar conocido.
    pub fn detect_syntax(&self, path: &Path) -> Option<&SyntaxReference> {
        let extension = path.extension()?.to_str()?;
        self.syntax_set
            .find_syntax_by_extension(extension)
            .or_else(|| {
                // Fallback: intentar por nombre de archivo (Makefile, Dockerfile, etc.)
                let file_name = path.file_name()?.to_str()?;
                self.syntax_set.find_syntax_by_extension(file_name)
            })
    }

    /// Referencia al `SyntaxSet` para búsquedas externas.
    pub fn syntax_set(&self) -> &SyntaxSet {
        &self.syntax_set
    }

    /// Referencia al theme para mapeo de colores.
    pub fn theme(&self) -> &SyntectTheme {
        &self.theme
    }
}

impl Default for HighlightEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── HighlightCache ───────────────────────────────────────────────────────────

/// Token coloreado cacheado — segmento de texto con su estilo.
///
/// Almacena el estilo de syntect y el texto owned (necesario porque
/// el buffer puede cambiar entre highlight y render).
#[derive(Debug, Clone)]
pub struct HighlightToken {
    /// Estilo de syntect (foreground, background, font_style).
    pub style: SyntectStyle,
    /// Texto del token.
    pub text: String,
}

/// Cache de highlight por línea para un buffer.
///
/// Se asocia a un `EditorState`. Se invalida cuando el buffer
/// cambia. `ensure_highlighted()` re-procesa todo el buffer
/// si está dirty — aceptable para MVP (archivos < 10k líneas).
#[derive(Debug)]
pub struct HighlightCache {
    /// Tokens coloreados por línea — índice = línea del buffer.
    lines: Vec<Vec<HighlightToken>>,
    /// Nombre de la syntax detectada (para debug/display).
    syntax_name: String,
    /// Si el cache necesita re-procesamiento.
    dirty: bool,
}

impl HighlightCache {
    /// Crea un cache vacío sin syntax.
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            syntax_name: String::new(),
            dirty: true,
        }
    }

    /// Crea un cache con la syntax detectada.
    pub fn with_syntax(syntax_name: &str) -> Self {
        Self {
            lines: Vec::new(),
            syntax_name: syntax_name.to_owned(),
            dirty: true,
        }
    }

    /// Marca el cache como dirty — necesita re-highlight.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Si el cache tiene syntax asociada.
    #[expect(
        dead_code,
        reason = "se usará para condicionales de rendering y status bar"
    )]
    pub fn has_syntax(&self) -> bool {
        !self.syntax_name.is_empty()
    }

    /// Nombre de la syntax detectada.
    #[expect(dead_code, reason = "se usará para mostrar syntax en status bar")]
    pub fn syntax_name(&self) -> &str {
        &self.syntax_name
    }

    /// Obtiene los tokens coloreados de una línea cacheada.
    ///
    /// Retorna `None` si la línea no existe en el cache o el cache
    /// está dirty. El llamador debe verificar con `is_valid()` antes.
    pub fn get_line(&self, line: usize) -> Option<&[HighlightToken]> {
        if self.dirty {
            return None;
        }
        self.lines.get(line).map(Vec::as_slice)
    }

    /// Si el cache está válido (no dirty).
    #[expect(dead_code, reason = "se usará para condicionales de rendering")]
    pub fn is_valid(&self) -> bool {
        !self.dirty
    }

    /// Re-procesa el highlighting completo del buffer.
    ///
    /// Usa `HighlightLines` de syntect que mantiene estado entre líneas
    /// para manejar contexto multi-línea (strings, block comments).
    ///
    /// Se llama ANTES del render, no durante. El resultado se cachea.
    pub fn ensure_highlighted(&mut self, buffer: &TextBuffer, engine: &HighlightEngine) {
        if !self.dirty {
            return; // Cache válido — noop
        }

        // Sin syntax detectada — no hay nada que cachear
        if self.syntax_name.is_empty() {
            self.dirty = false;
            self.lines.clear();
            return;
        }

        // Buscar la syntax por nombre en el engine
        let Some(syntax) = engine.syntax_set().find_syntax_by_name(&self.syntax_name) else {
            // Syntax desapareció — limpiar y marcar como sin syntax
            self.dirty = false;
            self.lines.clear();
            self.syntax_name.clear();
            return;
        };

        let line_count = buffer.line_count();
        // Pre-alocar con capacidad conocida
        let mut cached_lines = Vec::with_capacity(line_count);

        let mut highlighter = HighlightLines::new(syntax, engine.theme());

        for i in 0..line_count {
            let line_text = buffer.line(i).unwrap_or("");
            // syntect necesita la línea con newline para tracking de estado
            let line_with_newline = if i < line_count - 1 {
                // Usar Cow para evitar alocación cuando la línea ya tiene newline
                std::borrow::Cow::Owned(format!("{line_text}\n"))
            } else {
                // Última línea: puede no tener newline
                std::borrow::Cow::Borrowed(line_text)
            };

            // Highlight la línea — puede fallar si la regex es inválida
            let tokens = match highlighter.highlight_line(&line_with_newline, engine.syntax_set()) {
                Ok(ranges) => {
                    ranges
                        .into_iter()
                        .map(|(style, text)| {
                            // Remover trailing newline del token si existe
                            let clean_text = text.trim_end_matches('\n');
                            HighlightToken {
                                style,
                                text: clean_text.to_owned(),
                            }
                        })
                        .filter(|t| !t.text.is_empty()) // Descartar tokens vacíos
                        .collect()
                }
                Err(_) => {
                    // Error en syntect — retornar línea sin colorear
                    vec![HighlightToken {
                        style: SyntectStyle::default(),
                        text: line_text.to_owned(),
                    }]
                }
            };

            cached_lines.push(tokens);
        }

        self.lines = cached_lines;
        self.dirty = false;
    }
}

impl Default for HighlightCache {
    fn default() -> Self {
        Self::new()
    }
}
