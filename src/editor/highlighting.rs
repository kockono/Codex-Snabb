//! Highlighting: motor de syntax highlighting con syntect.
//!
//! Usa los ~50 grammars embebidos de syntect para colorear código.
//! `HighlightEngine` se carga en background (thread separado) para no
//! bloquear el primer render (~100-500ms de carga del SyntaxSet).
//! `HighlightCache` vive en cada `EditorState` y cachea tokens coloreados
//! solo para el viewport visible + margen de scroll.
//!
//! Performance:
//! - `SyntaxSet` y theme: carga en background thread, no bloquea startup
//! - Highlight solo del viewport visible (lazy) con margen ±20 líneas
//! - Cache parcial: edición en línea N solo invalida desde N en adelante
//! - Debounce de 150ms: no re-tokenizar en cada keystroke
//! - NUNCA highlight en el render loop — siempre cachear antes

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, Theme as SyntectTheme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use super::buffer::TextBuffer;

// ─── Constantes ───────────────────────────────────────────────────────────────

/// Margen de líneas extra a tokenizar antes/después del viewport visible.
/// Suaviza el scroll: al desplazarse, las líneas adyacentes ya están cacheadas.
const VIEWPORT_MARGIN: usize = 20;

/// Tiempo mínimo entre la última edición y el re-highlight (debounce).
/// Evita re-tokenizar en cada keystroke durante escritura rápida.
const DEBOUNCE_MS: u64 = 150;

// ─── HighlightEngine ──────────────────────────────────────────────────────────

/// Estado interno del motor de highlighting una vez cargado.
///
/// Contiene `SyntaxSet` (~2MB) y un theme oscuro. Inmutable después
/// de la construcción — se pasa por `&HighlightEngine` a los editores.
#[derive(Debug)]
struct HighlightEngineInner {
    /// Grammars embebidos de syntect (Rust, Python, JS, etc.).
    syntax_set: SyntaxSet,
    /// Theme para colorear tokens.
    theme: SyntectTheme,
}

/// Motor de syntax highlighting con carga lazy en background.
///
/// Al construir con `new()`, spawnea un `std::thread` que carga el
/// `SyntaxSet` (operación costosa ~100-500ms). Mientras carga, el editor
/// funciona sin highlighting (texto plano). Cuando termina, el cache
/// se marca dirty y el siguiente frame muestra colores.
///
/// Usa `std::sync::mpsc::sync_channel(1)` (bounded) para enviar el
/// resultado del thread de carga al hilo principal.
#[derive(Debug)]
pub struct HighlightEngine {
    /// Motor interno — `None` mientras se carga en background.
    inner: Option<HighlightEngineInner>,
    /// Si la carga está en progreso.
    loading: bool,
    /// Canal bounded para recibir el resultado de la carga.
    /// Se consume (take) una vez que llega el resultado.
    receiver: Option<std::sync::mpsc::Receiver<HighlightEngineInner>>,
}

impl HighlightEngine {
    /// Crea el motor e inicia la carga de syntaxes en background.
    ///
    /// La carga del `SyntaxSet` (~2MB) y `ThemeSet` se hace en un
    /// `std::thread` separado para no bloquear el primer render.
    /// El editor funciona sin colores hasta que la carga termine.
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<HighlightEngineInner>(1);

        std::thread::spawn(move || {
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
            let inner = HighlightEngineInner { syntax_set, theme };
            // Enviar resultado al hilo principal — si el receiver se droppó, ignorar
            let _ = tx.send(inner);
        });

        Self {
            inner: None,
            loading: true,
            receiver: Some(rx),
        }
    }

    /// Intenta recibir el resultado de la carga en background.
    ///
    /// Non-blocking — se llama en cada frame del event loop.
    /// Retorna `true` si el engine se inicializó en ESTA llamada
    /// (útil para marcar caches como dirty).
    pub fn try_init(&mut self) -> bool {
        if self.inner.is_some() {
            return false; // Ya inicializado
        }

        let Some(ref receiver) = self.receiver else {
            return false; // Canal ya consumido
        };

        // try_recv: non-blocking, retorna inmediatamente
        match receiver.try_recv() {
            Ok(engine_inner) => {
                self.inner = Some(engine_inner);
                self.loading = false;
                self.receiver = None; // Canal consumido — liberar recurso
                tracing::info!("HighlightEngine cargado desde background thread");
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false, // Aún cargando
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // El thread terminó sin enviar resultado — error interno
                tracing::error!("background thread de highlight terminó sin resultado");
                self.loading = false;
                self.receiver = None;
                false
            }
        }
    }

    /// Si el engine está listo (carga completada).
    pub fn is_ready(&self) -> bool {
        self.inner.is_some()
    }

    /// Si la carga está en progreso.
    #[expect(
        dead_code,
        reason = "se usará para mostrar indicador de carga en status bar"
    )]
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Detecta la syntax por extensión del archivo.
    ///
    /// Retorna `None` si el engine no está listo o la extensión no tiene grammar.
    pub fn detect_syntax(&self, path: &Path) -> Option<&SyntaxReference> {
        let inner = self.inner.as_ref()?;
        let extension = path.extension()?.to_str()?;
        inner
            .syntax_set
            .find_syntax_by_extension(extension)
            .or_else(|| {
                // Fallback: intentar por nombre de archivo (Makefile, Dockerfile, etc.)
                let file_name = path.file_name()?.to_str()?;
                inner.syntax_set.find_syntax_by_extension(file_name)
            })
    }

    /// Referencia al `SyntaxSet` para búsquedas externas.
    ///
    /// Retorna `None` si el engine no está listo aún.
    pub fn syntax_set(&self) -> Option<&SyntaxSet> {
        self.inner.as_ref().map(|i| &i.syntax_set)
    }

    /// Referencia al theme para mapeo de colores.
    ///
    /// Retorna `None` si el engine no está listo aún.
    pub fn theme(&self) -> Option<&SyntectTheme> {
        self.inner.as_ref().map(|i| &i.theme)
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

/// Cache de highlight por viewport para un buffer.
///
/// Solo cachea tokens de las líneas visibles + margen. Invalidación
/// parcial: editar línea N solo invalida desde N en adelante.
/// Debounce: no re-tokenizar hasta que pasen 150ms desde la última edición.
#[derive(Debug)]
pub struct HighlightCache {
    /// Tokens coloreados por línea — solo las líneas cacheadas (viewport + margen).
    /// HashMap<línea_del_buffer, tokens> — disperso, no denso.
    lines: HashMap<usize, Vec<HighlightToken>>,
    /// Nombre de la syntax detectada (para debug/display).
    syntax_name: String,
    /// Desde qué línea está dirty (una edición en línea 50 invalida 50+, no 0-49).
    /// `Some(0)` = todo dirty. `None` = cache válido.
    dirty_from: Option<usize>,
    /// Inicio del último viewport procesado.
    last_viewport_start: usize,
    /// Fin del último viewport procesado (exclusivo).
    last_viewport_end: usize,
    /// Cantidad de líneas del buffer cuando se cacheo por última vez.
    buffer_line_count: usize,
    /// Timestamp de la última edición — para debounce.
    last_edit_time: Option<Instant>,
}

impl HighlightCache {
    /// Crea un cache vacío sin syntax.
    pub fn new() -> Self {
        Self {
            lines: HashMap::new(),
            syntax_name: String::new(),
            dirty_from: Some(0),
            last_viewport_start: 0,
            last_viewport_end: 0,
            buffer_line_count: 0,
            last_edit_time: None,
        }
    }

    /// Crea un cache con la syntax detectada.
    pub fn with_syntax(syntax_name: &str) -> Self {
        Self {
            lines: HashMap::new(),
            syntax_name: syntax_name.to_owned(),
            dirty_from: Some(0),
            last_viewport_start: 0,
            last_viewport_end: 0,
            buffer_line_count: 0,
            last_edit_time: None,
        }
    }

    /// Marca el cache como dirty desde una línea específica.
    ///
    /// Solo invalida desde `line` en adelante — las líneas anteriores
    /// mantienen su cache válido.
    pub fn invalidate_from(&mut self, line: usize) {
        self.last_edit_time = Some(Instant::now());
        match self.dirty_from {
            Some(existing) => {
                // Mantener el mínimo: si ya estaba dirty desde línea 30
                // y ahora se edita línea 20, dirty_from = 20
                self.dirty_from = Some(existing.min(line));
            }
            None => {
                self.dirty_from = Some(line);
            }
        }
    }

    /// Marca TODO el cache como dirty — para carga inicial o cambio de syntax.
    pub fn invalidate(&mut self) {
        self.last_edit_time = Some(Instant::now());
        self.dirty_from = Some(0);
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
    /// Retorna `None` si la línea no está en el cache (fuera del viewport,
    /// dirty, o engine aún cargando). El caller renderiza texto plano como fallback.
    pub fn get_line(&self, line: usize) -> Option<&[HighlightToken]> {
        self.lines.get(&line).map(Vec::as_slice)
    }

    /// Si el cache está válido (no dirty en ninguna línea).
    #[expect(dead_code, reason = "se usará para condicionales de rendering")]
    pub fn is_valid(&self) -> bool {
        self.dirty_from.is_none()
    }

    /// Procesa el highlighting solo para las líneas del viewport visible.
    ///
    /// Lógica:
    /// 1. Si el engine no está listo: noop (el editor renderiza texto plano)
    /// 2. Si hay debounce activo (editando rápido): noop
    /// 3. Si el viewport no cambió y no hay líneas dirty en el rango: noop
    /// 4. Si hay que procesar: iterar desde línea 0 hasta viewport_end + margen,
    ///    DESCARTANDO tokens de líneas antes del viewport (solo avanza parse state),
    ///    y GUARDANDO tokens solo para líneas dentro del viewport + margen.
    pub fn ensure_viewport_highlighted(
        &mut self,
        buffer: &TextBuffer,
        engine: &HighlightEngine,
        viewport_start: usize,
        viewport_height: usize,
    ) {
        // Engine no listo — noop
        if !engine.is_ready() {
            return;
        }

        // Sin syntax detectada — no hay nada que cachear
        if self.syntax_name.is_empty() {
            self.dirty_from = None;
            self.lines.clear();
            return;
        }

        // Debounce: si la última edición fue hace menos de DEBOUNCE_MS, esperar
        if let Some(edit_time) = self.last_edit_time
            && edit_time.elapsed() < Duration::from_millis(DEBOUNCE_MS)
        {
            return; // Esperando debounce — no re-tokenizar
        }

        // Calcular rango efectivo del viewport con margen
        let line_count = buffer.line_count();
        let effective_start = viewport_start.saturating_sub(VIEWPORT_MARGIN);
        let effective_end = (viewport_start + viewport_height + VIEWPORT_MARGIN).min(line_count);

        // Determinar si necesitamos re-procesar
        let needs_reprocess = match self.dirty_from {
            Some(dirty_line) => {
                // Hay líneas dirty — ¿afectan al viewport actual?
                dirty_line < effective_end
            }
            None => {
                // Cache limpio — ¿cambió el viewport?
                effective_start != self.last_viewport_start
                    || effective_end != self.last_viewport_end
                    || line_count != self.buffer_line_count
            }
        };

        if !needs_reprocess {
            return; // Cache válido para el viewport actual
        }

        // Obtener referencia al syntax_set y theme del engine
        let Some(syntax_set) = engine.syntax_set() else {
            return;
        };
        let Some(theme) = engine.theme() else {
            return;
        };

        // Buscar la syntax por nombre
        let Some(syntax) = syntax_set.find_syntax_by_name(&self.syntax_name) else {
            // Syntax desapareció — limpiar
            self.dirty_from = None;
            self.lines.clear();
            self.syntax_name.clear();
            return;
        };

        // Determinar desde dónde debemos re-tokenizar.
        // syntect necesita contexto multi-línea: un string abierto en línea 10
        // afecta las líneas 11+. Debemos procesar desde línea 0 siempre para
        // mantener el parse state correcto, pero solo GUARDAR tokens del viewport.
        //
        // Optimización: si dirty_from > 0 y tenemos cache válido antes de dirty_from,
        // solo necesitamos invalidar las líneas >= dirty_from.
        // Sin embargo, el parse state de syntect se propaga, así que siempre
        // empezamos desde 0 para garantizar correctness.

        let mut highlighter = HighlightLines::new(syntax, theme);

        // Limpiar líneas dirty del cache (mantener las clean antes de dirty_from)
        if let Some(dirty_line) = self.dirty_from {
            self.lines.retain(|&line_idx, _| line_idx < dirty_line);
        }

        // Limpiar líneas fuera del rango del viewport actual + margen
        self.lines
            .retain(|&line_idx, _| line_idx >= effective_start && line_idx < effective_end);

        for i in 0..effective_end.min(line_count) {
            let line_text = buffer.line(i).unwrap_or("");
            // syntect necesita la línea con newline para tracking de estado
            let line_with_newline = if i < line_count - 1 {
                std::borrow::Cow::Owned(format!("{line_text}\n"))
            } else {
                std::borrow::Cow::Borrowed(line_text)
            };

            // Highlight la línea — puede fallar si la regex es inválida
            let highlight_result = highlighter.highlight_line(&line_with_newline, syntax_set);

            // ¿Esta línea está en el rango que necesitamos cachear?
            let in_viewport = i >= effective_start && i < effective_end;

            // ¿Ya la tenemos cacheada y no está dirty?
            let already_cached = self.lines.contains_key(&i);

            if in_viewport && !already_cached {
                // Guardar tokens en cache
                let tokens = match highlight_result {
                    Ok(ranges) => ranges
                        .into_iter()
                        .map(|(style, text)| {
                            let clean_text = text.trim_end_matches('\n');
                            HighlightToken {
                                style,
                                text: clean_text.to_owned(),
                            }
                        })
                        .filter(|t| !t.text.is_empty())
                        .collect(),
                    Err(_) => {
                        // Error en syntect — retornar línea sin colorear
                        vec![HighlightToken {
                            style: SyntectStyle::default(),
                            text: line_text.to_owned(),
                        }]
                    }
                };
                self.lines.insert(i, tokens);
            }
            // Líneas ANTES del viewport: descartamos el resultado de highlight
            // pero el parse state de `highlighter` avanza correctamente.
        }

        // Actualizar metadata del cache
        self.last_viewport_start = effective_start;
        self.last_viewport_end = effective_end;
        self.buffer_line_count = line_count;
        self.dirty_from = None;
    }
}

impl Default for HighlightCache {
    fn default() -> Self {
        Self::new()
    }
}
