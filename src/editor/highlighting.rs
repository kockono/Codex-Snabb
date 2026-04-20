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
use syntect::highlighting::{
    HighlightState, Style as SyntectStyle, Theme as SyntectTheme, ThemeSet,
};
use syntect::parsing::{ParseState, SyntaxReference, SyntaxSet};

use super::buffer::TextBuffer;

// ─── Constantes ───────────────────────────────────────────────────────────────

/// Margen de líneas extra a tokenizar antes/después del viewport visible.
/// Suaviza el scroll: al desplazarse, las líneas adyacentes ya están cacheadas.
/// 50 líneas: con scroll de 1 línea por evento, da ~50 eventos de margen.
const VIEWPORT_MARGIN: usize = 50;

/// Tiempo mínimo entre la última edición y el re-highlight (debounce).
/// Evita re-tokenizar en cada keystroke durante escritura rápida.
/// Solo aplica a invalidaciones por edición, NO a scroll.
const DEBOUNCE_MS: u64 = 150;

/// Intervalo de líneas entre checkpoints de parse state.
/// Cada N líneas se guarda el estado del parser para poder retomar
/// desde el checkpoint más cercano al viewport en vez de línea 0.
/// 25 líneas: más checkpoints = re-inicio más rápido durante pre-cache.
const CHECKPOINT_INTERVAL: usize = 25;

/// Cantidad de líneas a pre-cachear por frame en idle.
/// 200 líneas por tick: a ~60fps idle, un archivo de 12k líneas
/// se pre-cachea en ~1 segundo sin impactar latencia de input.
const PRECACHE_CHUNK_SIZE: usize = 200;

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

/// Estado del parser guardado en un checkpoint.
///
/// Contiene `ParseState` y `HighlightState` de syntect clonados en un
/// punto específico del archivo. Permite retomar el highlighting desde
/// ese punto sin recorrer desde línea 0.
struct SavedParseState {
    /// Estado del parser de syntect (stack de contextos, prototipos).
    parse_state: ParseState,
    /// Estado del highlighter (estilos acumulados, scope stack).
    highlight_state: HighlightState,
}

/// Cache de highlight por viewport para un buffer.
///
/// Solo cachea tokens de las líneas visibles + margen. Invalidación
/// parcial: editar línea N solo invalida desde N en adelante.
/// Debounce: no re-tokenizar hasta que pasen 150ms desde la última edición.
///
/// Usa checkpoints de parse state cada `CHECKPOINT_INTERVAL` líneas para
/// evitar recorrer desde línea 0 en cada scroll. Esto reduce el costo
/// de scroll de O(viewport_end) a O(CHECKPOINT_INTERVAL + viewport_size).
///
/// `Debug` implementado manualmente — `SavedParseState` contiene tipos de
/// syntect que no necesitan debug detallado, solo mostramos la cantidad.
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
    /// Parse states guardados cada `CHECKPOINT_INTERVAL` líneas.
    /// Key = número de línea del checkpoint, Value = estado guardado.
    /// Permite retomar highlighting desde el checkpoint más cercano al viewport
    /// en vez de recorrer desde línea 0.
    checkpoints: HashMap<usize, SavedParseState>,
    /// Si el pre-cache progresivo del archivo completo está en progreso.
    /// Se inicia después del primer highlight del viewport y avanza en idle frames.
    precache_in_progress: bool,
    /// Hasta qué línea se ha pre-cacheado (exclusivo).
    /// Avanza en chunks de `PRECACHE_CHUNK_SIZE` por frame idle.
    precache_cursor: usize,
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
            checkpoints: HashMap::new(),
            precache_in_progress: false,
            precache_cursor: 0,
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
            checkpoints: HashMap::new(),
            precache_in_progress: false,
            precache_cursor: 0,
        }
    }

    /// Marca el cache como dirty desde una línea específica.
    ///
    /// Solo invalida desde `line` en adelante — las líneas anteriores
    /// mantienen su cache válido. También invalida checkpoints >= line
    /// y resetea el pre-cache si la edición afecta líneas ya pre-cacheadas.
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
        // Invalidar checkpoints desde la línea editada en adelante.
        // Los checkpoints antes de la edición siguen siendo válidos.
        self.checkpoints
            .retain(|&checkpoint_line, _| checkpoint_line < line);
        // Resetear pre-cache: si la edición está antes del cursor de pre-cache,
        // necesitamos re-procesar desde ahí. Si está después, no afecta.
        if line < self.precache_cursor {
            self.precache_cursor = line;
            self.precache_in_progress = true;
        }
    }

    /// Marca TODO el cache como dirty — para carga inicial o cambio de syntax.
    pub fn invalidate(&mut self) {
        self.last_edit_time = Some(Instant::now());
        self.dirty_from = Some(0);
        self.checkpoints.clear();
        // Resetear pre-cache completo — todo debe re-procesarse.
        self.precache_in_progress = false;
        self.precache_cursor = 0;
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
    ///
    /// Líneas marcadas como dirty (>= `dirty_from`) retornan `None` aunque tengan
    /// tokens stale en cache. Esto fuerza al render a usar texto plano inmediatamente
    /// durante el debounce, eliminando el lag perceptible al tipear.
    pub fn get_line(&self, line_idx: usize) -> Option<&[HighlightToken]> {
        // Si la línea está dirty, retornar None para que el render use plain text.
        // Esto elimina el lag de 150ms: el carácter aparece inmediatamente en plain
        // text, y los colores vuelven cuando syntect re-procesa tras el debounce.
        if self.dirty_from.is_some_and(|from| line_idx >= from) {
            return None;
        }
        self.lines.get(&line_idx).map(Vec::as_slice)
    }

    /// Si el cache está válido (no dirty en ninguna línea).
    #[expect(dead_code, reason = "se usará para condicionales de rendering")]
    pub fn is_valid(&self) -> bool {
        self.dirty_from.is_none()
    }

    /// Busca el checkpoint más cercano ANTES o en `target_line`.
    ///
    /// Retorna la línea del checkpoint encontrado, o `None` si no hay
    /// ningún checkpoint válido antes de `target_line`.
    fn find_nearest_checkpoint(&self, target_line: usize) -> Option<usize> {
        // Buscar el checkpoint alineado al intervalo <= target_line.
        // Empezar desde el checkpoint ideal y bajar hasta encontrar uno que exista.
        let ideal = (target_line / CHECKPOINT_INTERVAL) * CHECKPOINT_INTERVAL;
        let mut candidate = ideal;
        loop {
            if self.checkpoints.contains_key(&candidate) {
                return Some(candidate);
            }
            if candidate < CHECKPOINT_INTERVAL {
                return None; // No hay checkpoints antes de target_line
            }
            candidate -= CHECKPOINT_INTERVAL;
        }
    }

    /// Guarda un checkpoint del parse state en la línea indicada.
    ///
    /// Consume el highlighter para extraer el state, lo clona para el
    /// checkpoint, y reconstruye el highlighter para seguir procesando.
    ///
    /// Retorna el highlighter reconstruido.
    fn save_checkpoint<'a>(
        &mut self,
        highlighter: HighlightLines<'a>,
        line: usize,
        theme: &'a SyntectTheme,
    ) -> HighlightLines<'a> {
        // Consumir highlighter → extraer states
        let (hl_state, ps_state) = highlighter.state();

        // CLONE: necesario — guardamos una copia en el checkpoint y usamos
        // los originales para reconstruir el highlighter y seguir procesando.
        let saved = SavedParseState {
            parse_state: ps_state.clone(),
            highlight_state: hl_state.clone(),
        };
        self.checkpoints.insert(line, saved);

        // Reconstruir highlighter desde los states originales
        HighlightLines::from_state(theme, hl_state, ps_state)
    }

    /// Procesa el highlighting solo para las líneas del viewport visible.
    ///
    /// Usa checkpoints de parse state para evitar recorrer desde línea 0.
    /// En cada scroll, busca el checkpoint más cercano al viewport y retoma
    /// desde ahí, reduciendo el costo de O(viewport_end) a
    /// O(CHECKPOINT_INTERVAL + viewport_size).
    ///
    /// Lógica:
    /// 1. Si el engine no está listo: noop (el editor renderiza texto plano)
    /// 2. Si hay debounce activo por EDICIÓN: noop (scroll no tiene debounce)
    /// 3. Si el viewport no cambió y no hay líneas dirty en el rango: noop
    /// 4. Si hay que procesar: buscar checkpoint más cercano al viewport,
    ///    iterar desde checkpoint hasta viewport_end + margen,
    ///    guardando tokens del viewport y nuevos checkpoints cada N líneas.
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

        // Debounce: SOLO si la invalidación fue por edición (dirty_from is Some).
        // Scroll puro (solo cambio de viewport, dirty_from es None) procesa inmediatamente.
        if self.dirty_from.is_some()
            && let Some(edit_time) = self.last_edit_time
            && edit_time.elapsed() < Duration::from_millis(DEBOUNCE_MS)
        {
            return; // Esperando debounce de edición — no re-tokenizar
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
                // Cache limpio — ¿cambió el viewport y hay líneas sin cachear?
                // Si el archivo completo está pre-cacheado, verificar que todas
                // las líneas del viewport están en cache (cache hit puro).
                let viewport_fully_cached =
                    (effective_start..effective_end).all(|line| self.lines.contains_key(&line));
                if viewport_fully_cached {
                    // Actualizar metadata sin re-procesar
                    self.last_viewport_start = effective_start;
                    self.last_viewport_end = effective_end;
                    self.buffer_line_count = line_count;
                    return; // Cache hit puro — 0 trabajo
                }
                // Hay líneas sin cachear en el viewport
                true
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

        // Limpiar líneas dirty del cache (mantener las clean antes de dirty_from)
        if let Some(dirty_line) = self.dirty_from {
            self.lines.retain(|&line_idx, _| line_idx < dirty_line);
        }

        // NO limpiar líneas fuera del viewport — las mantiene el pre-cache.
        // Solo limpiamos en dirty; scroll no invalida líneas pre-cacheadas.

        // ── Determinar punto de inicio usando checkpoints ──
        //
        // Buscar el checkpoint más cercano ANTES de effective_start.
        // Si dirty_from existe y está antes del checkpoint, el checkpoint
        // es inválido (ya fue limpiado en invalidate_from), así que
        // find_nearest_checkpoint solo encuentra checkpoints válidos.
        let (start_line, mut highlighter) = match self
            .find_nearest_checkpoint(effective_start)
            .and_then(|cp_line| {
                let saved = self.checkpoints.get(&cp_line)?;
                // CLONE: necesario — from_state consume los states, pero
                // necesitamos mantener el checkpoint en el HashMap para futuros usos.
                let hl = HighlightLines::from_state(
                    theme,
                    saved.highlight_state.clone(),
                    saved.parse_state.clone(),
                );
                Some((cp_line, hl))
            }) {
            Some((cp_line, hl)) => (cp_line, hl),
            None => {
                // Sin checkpoints válidos — empezar desde línea 0
                (0, HighlightLines::new(syntax, theme))
            }
        };

        // Buffer reutilizable para evitar format!() allocation en cada línea.
        // Capacity 256: línea promedio de código ~80 chars + margen.
        let mut line_buf = String::with_capacity(256);

        for i in start_line..effective_end.min(line_count) {
            let line_text = buffer.line(i).unwrap_or("");

            // Preparar línea con newline usando buffer reutilizable.
            // syntect necesita newline para tracking de estado multi-línea.
            line_buf.clear();
            line_buf.push_str(line_text);
            if i < line_count - 1 {
                line_buf.push('\n');
            }

            // Highlight la línea — puede fallar si la regex es inválida
            let highlight_result = highlighter.highlight_line(&line_buf, syntax_set);

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
            // Líneas entre checkpoint y viewport: descartamos el resultado de highlight
            // pero el parse state de `highlighter` avanza correctamente.

            // Guardar checkpoint cada CHECKPOINT_INTERVAL líneas procesadas.
            // Solo si la línea siguiente es un múltiplo del intervalo y no tenemos
            // ya un checkpoint ahí. El checkpoint se guarda DESPUÉS de procesar la
            // línea (i), así el state incluye el contexto de esa línea.
            let next_line = i + 1;
            if next_line % CHECKPOINT_INTERVAL == 0
                && next_line < line_count
                && !self.checkpoints.contains_key(&next_line)
            {
                highlighter = self.save_checkpoint(highlighter, next_line, theme);
            }
        }

        // Actualizar metadata del cache
        self.last_viewport_start = effective_start;
        self.last_viewport_end = effective_end;
        self.buffer_line_count = line_count;
        self.dirty_from = None;

        // Iniciar pre-cache progresivo si no está ya en progreso.
        // Después del primer highlight del viewport, empezar a pre-cachear
        // el resto del archivo en idle frames.
        if !self.precache_in_progress && self.precache_cursor < line_count {
            self.precache_in_progress = true;
            // Iniciar pre-cache desde el final del viewport actual
            // (el viewport ya está cacheado, pre-cachear el resto).
            if self.precache_cursor < effective_end {
                self.precache_cursor = effective_end;
            }
        }
    }

    /// Pre-cachea un chunk de líneas fuera del viewport actual.
    ///
    /// Llamar en cada frame del event loop cuando no hay eventos de usuario.
    /// Procesa `PRECACHE_CHUNK_SIZE` líneas desde `precache_cursor`,
    /// guardando tokens y checkpoints. Avanza el cursor para el siguiente frame.
    ///
    /// Retorna `true` si todavía hay trabajo pendiente (más líneas por cachear).
    /// Retorna `false` si el pre-cache terminó o no hay nada que hacer.
    pub fn precache_chunk(&mut self, buffer: &TextBuffer, engine: &HighlightEngine) -> bool {
        // Nada que hacer si el pre-cache no está activo
        if !self.precache_in_progress {
            return false;
        }

        // Engine no listo — no pre-cachear
        if !engine.is_ready() {
            return false;
        }

        // Sin syntax — nada que pre-cachear
        if self.syntax_name.is_empty() {
            self.precache_in_progress = false;
            return false;
        }

        let line_count = buffer.line_count();

        // Ya terminamos — todo el archivo está pre-cacheado
        if self.precache_cursor >= line_count {
            self.precache_in_progress = false;
            tracing::debug!(
                lines_cached = self.lines.len(),
                checkpoints = self.checkpoints.len(),
                "pre-cache completo — archivo entero cacheado"
            );
            return false;
        }

        // Obtener referencia al syntax_set y theme del engine
        let Some(syntax_set) = engine.syntax_set() else {
            return false;
        };
        let Some(theme) = engine.theme() else {
            return false;
        };

        // Buscar la syntax por nombre
        let Some(syntax) = syntax_set.find_syntax_by_name(&self.syntax_name) else {
            self.precache_in_progress = false;
            return false;
        };

        // Buscar checkpoint más cercano al cursor de pre-cache
        let (start_line, mut highlighter) = match self
            .find_nearest_checkpoint(self.precache_cursor)
            .and_then(|cp_line| {
                let saved = self.checkpoints.get(&cp_line)?;
                // CLONE: necesario — from_state consume los states, pero
                // necesitamos mantener el checkpoint en el HashMap para futuros usos.
                let hl = HighlightLines::from_state(
                    theme,
                    saved.highlight_state.clone(),
                    saved.parse_state.clone(),
                );
                Some((cp_line, hl))
            }) {
            Some((cp_line, hl)) => (cp_line, hl),
            None => {
                // Sin checkpoints — empezar desde línea 0
                (0, HighlightLines::new(syntax, theme))
            }
        };

        let chunk_end = (self.precache_cursor + PRECACHE_CHUNK_SIZE).min(line_count);

        // Buffer reutilizable para evitar allocation en cada línea
        let mut line_buf = String::with_capacity(256);

        for i in start_line..chunk_end {
            let line_text = buffer.line(i).unwrap_or("");

            // Preparar línea con newline para syntect
            line_buf.clear();
            line_buf.push_str(line_text);
            if i < line_count - 1 {
                line_buf.push('\n');
            }

            // Highlight la línea
            let highlight_result = highlighter.highlight_line(&line_buf, syntax_set);

            // Solo cachear si la línea está en el rango del chunk actual
            // y no está ya cacheada (las líneas entre checkpoint y precache_cursor
            // solo se procesan para avanzar el parse state).
            let in_chunk = i >= self.precache_cursor;
            let already_cached = self.lines.contains_key(&i);

            if in_chunk && !already_cached {
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
                        vec![HighlightToken {
                            style: SyntectStyle::default(),
                            text: line_text.to_owned(),
                        }]
                    }
                };
                self.lines.insert(i, tokens);
            }

            // Guardar checkpoint cada CHECKPOINT_INTERVAL líneas
            let next_line = i + 1;
            if next_line % CHECKPOINT_INTERVAL == 0
                && next_line < line_count
                && !self.checkpoints.contains_key(&next_line)
            {
                highlighter = self.save_checkpoint(highlighter, next_line, theme);
            }
        }

        // Avanzar cursor de pre-cache
        self.precache_cursor = chunk_end;

        // ¿Terminamos?
        if self.precache_cursor >= line_count {
            self.precache_in_progress = false;
            tracing::debug!(
                lines_cached = self.lines.len(),
                checkpoints = self.checkpoints.len(),
                "pre-cache completo — archivo entero cacheado"
            );
            false
        } else {
            true // Hay más trabajo pendiente
        }
    }
}

impl std::fmt::Debug for HighlightCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HighlightCache")
            .field("lines_cached", &self.lines.len())
            .field("syntax_name", &self.syntax_name)
            .field("dirty_from", &self.dirty_from)
            .field("last_viewport_start", &self.last_viewport_start)
            .field("last_viewport_end", &self.last_viewport_end)
            .field("buffer_line_count", &self.buffer_line_count)
            .field("checkpoints_count", &self.checkpoints.len())
            .field("precache_in_progress", &self.precache_in_progress)
            .field("precache_cursor", &self.precache_cursor)
            .finish()
    }
}

impl Default for HighlightCache {
    fn default() -> Self {
        Self::new()
    }
}
