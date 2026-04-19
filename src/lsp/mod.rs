//! LSP: lifecycle de servidores, requests, cancelación, debounce.
//!
//! Cliente LSP austero: activación lazy por lenguaje, requests cancelables
//! con debounce, diagnósticos y completion solo para buffers activos.
//! No guarda modelos semánticos enormes — el servidor los provee.
//!
//! El `LspState` coordina el cliente, almacena datos recibidos (diagnósticos,
//! hover, completions), y expone una API limpia al `AppState`.

pub mod client;
pub mod transport;

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};

use client::{LspClient, LspMessage};

// ─── Tipos de datos ────────────────────────────────────────────────────────────

/// Diagnóstico individual reportado por el language server.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Línea del diagnóstico (0-indexed).
    pub line: u32,
    /// Columna de inicio del rango (0-indexed).
    pub col_start: u32,
    /// Columna de fin del rango (0-indexed).
    pub col_end: u32,
    /// Severidad del diagnóstico.
    pub severity: DiagnosticSeverity,
    /// Mensaje descriptivo.
    pub message: String,
    /// Fuente del diagnóstico (ej: "rustc", "clippy").
    #[expect(dead_code, reason = "se usará para display detallado de diagnósticos")]
    pub source: Option<String>,
}

/// Severidad de un diagnóstico LSP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// Error que impide compilación.
    Error,
    /// Advertencia que no impide compilación.
    Warning,
    /// Información general.
    Information,
    /// Sugerencia de mejora.
    Hint,
}

/// Información de hover para mostrar como tooltip.
#[derive(Debug, Clone)]
pub struct HoverInfo {
    /// Contenido textual del hover.
    pub content: String,
    /// Línea donde se solicitó el hover (0-indexed).
    #[expect(dead_code, reason = "se usará para posicionar el tooltip del hover")]
    pub line: u32,
    /// Columna donde se solicitó el hover (0-indexed).
    #[expect(dead_code, reason = "se usará para posicionar el tooltip del hover")]
    pub col: u32,
}

/// Item de autocompletado.
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// Texto a mostrar en la lista de completions.
    pub label: String,
    /// Detalle adicional (tipo, firma, etc.).
    #[expect(dead_code, reason = "se usará para tooltip detallado en completions")]
    pub detail: Option<String>,
    /// Tipo de item (función, variable, struct, etc.).
    pub kind: Option<String>,
    /// Texto a insertar (puede diferir del label). Si None, se usa label.
    pub insert_text: Option<String>,
}

/// Resultado de go-to-definition.
#[derive(Debug, Clone)]
pub struct DefinitionResult {
    /// URI del archivo destino.
    pub uri: String,
    /// Línea destino (0-indexed).
    pub line: u32,
    /// Columna destino (0-indexed).
    pub col: u32,
}

// ─── Debounce ──────────────────────────────────────────────────────────────────

/// Intervalo mínimo entre notificaciones `did_change` (300ms).
/// Evita saturar el server con cambios en cada keystroke.
const DEBOUNCE_MS: u128 = 300;

// ─── LspState ──────────────────────────────────────────────────────────────────

/// Estado completo del subsistema LSP.
///
/// Contiene el cliente opcional (None si no hay server activo),
/// los datos recibidos del server (diagnósticos, hover, completions),
/// y el tracking de versiones de documentos para did_change.
#[derive(Debug)]
pub struct LspState {
    /// Cliente LSP activo (None si no hay server).
    pub client: Option<LspClient>,
    /// Diagnósticos por URI: uri → lista de diagnósticos.
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Hover info activo (se limpia al mover el cursor).
    pub hover_content: Option<HoverInfo>,
    /// Items de autocompletado disponibles.
    pub completions: Vec<CompletionItem>,
    /// Si la lista de completions está visible.
    pub completion_visible: bool,
    /// Índice del completion seleccionado.
    pub completion_selected: usize,
    /// Versiones de documentos abiertos: uri → version counter.
    pub document_versions: HashMap<String, i32>,
    /// Timestamp del último did_change enviado (para debounce).
    last_change_time: Option<Instant>,
    /// Si hay un cambio pendiente por debounce.
    change_pending: bool,
    /// URI del archivo con cambio pendiente.
    pending_change_uri: Option<String>,
    /// Resultado de go-to-definition pendiente.
    pub definition_result: Option<DefinitionResult>,
    /// Status line message del LSP (diagnóstico bajo el cursor).
    pub status_message: Option<String>,
}

impl LspState {
    /// Crea un nuevo estado LSP vacío.
    pub fn new() -> Self {
        Self {
            client: None,
            diagnostics: HashMap::new(),
            hover_content: None,
            completions: Vec::new(),
            completion_visible: false,
            completion_selected: 0,
            document_versions: HashMap::new(),
            last_change_time: None,
            change_pending: false,
            pending_change_uri: None,
            definition_result: None,
            status_message: None,
        }
    }

    /// Arranca un language server para el workspace dado.
    ///
    /// Lanza el proceso, ejecuta el handshake initialize/initialized.
    /// Si ya hay un server activo, lo cierra primero.
    pub fn start_server(&mut self, command: &str, args: &[&str], root_path: &Path) -> Result<()> {
        // Cerrar server previo si existe
        if self.client.is_some() {
            self.stop()?;
        }

        let root_uri = path_to_uri(root_path);
        let client = LspClient::start(command, args, &root_uri)
            .with_context(|| format!("error arrancando LSP server: {command}"))?;

        self.client = Some(client);
        self.diagnostics.clear();
        self.hover_content = None;
        self.completions.clear();
        self.completion_visible = false;
        self.document_versions.clear();

        tracing::info!(command, "LSP server arrancado");
        Ok(())
    }

    /// Poll de mensajes del server y procesamiento.
    ///
    /// Debe llamarse en cada ciclo del event loop. Non-blocking.
    /// Parsea responses y notifications, actualiza el estado local.
    pub fn poll(&mut self) {
        let Some(ref mut client) = self.client else {
            return;
        };

        let messages = client.poll_messages();
        for msg in messages {
            match msg {
                LspMessage::Notification { method, params } => {
                    self.handle_notification(&method, params);
                }
                LspMessage::Response { id, result } => {
                    self.handle_response(id, result);
                }
                LspMessage::Error { id, code, message } => {
                    tracing::warn!(id, code, message, "LSP error response");
                }
            }
        }
    }

    /// Procesa una notification del server.
    fn handle_notification(&mut self, method: &str, params: serde_json::Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                self.handle_diagnostics(params);
            }
            "window/logMessage" | "window/showMessage" => {
                // Log del server — redirigir a tracing
                if let Some(msg) = params.get("message").and_then(|v| v.as_str()) {
                    tracing::debug!(method, msg, "LSP server message");
                }
            }
            _ => {
                tracing::trace!(method, "LSP notification ignorada");
            }
        }
    }

    /// Procesa diagnósticos publicados por el server.
    fn handle_diagnostics(&mut self, params: serde_json::Value) {
        let uri = match params.get("uri").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => return,
        };

        let diags_json = match params.get("diagnostics").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                // Sin diagnósticos — limpiar los del URI
                self.diagnostics.remove(&uri);
                return;
            }
        };

        let diagnostics: Vec<Diagnostic> = diags_json
            .iter()
            .filter_map(|d| {
                let range = d.get("range")?;
                let start = range.get("start")?;
                let end = range.get("end")?;

                let severity_num = d.get("severity").and_then(|v| v.as_u64()).unwrap_or(1);

                let severity = match severity_num {
                    1 => DiagnosticSeverity::Error,
                    2 => DiagnosticSeverity::Warning,
                    3 => DiagnosticSeverity::Information,
                    4 => DiagnosticSeverity::Hint,
                    _ => DiagnosticSeverity::Information,
                };

                Some(Diagnostic {
                    line: start.get("line")?.as_u64()? as u32,
                    col_start: start.get("character")?.as_u64()? as u32,
                    col_end: end.get("character")?.as_u64()? as u32,
                    severity,
                    message: d
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    source: d.get("source").and_then(|v| v.as_str()).map(String::from),
                })
            })
            .collect();

        if diagnostics.is_empty() {
            self.diagnostics.remove(&uri);
        } else {
            self.diagnostics.insert(uri, diagnostics);
        }
    }

    /// Procesa una response a un request previo.
    fn handle_response(&mut self, _id: i64, result: serde_json::Value) {
        // Intentar parsear como hover response
        if self.try_parse_hover(&result) {
            return;
        }

        // Intentar parsear como completion response
        if self.try_parse_completion(&result) {
            return;
        }

        // Intentar parsear como definition response
        if self.try_parse_definition(&result) {
            return;
        }

        // Response desconocida
        tracing::trace!("LSP response sin handler específico");
    }

    /// Intenta parsear una response como hover result.
    fn try_parse_hover(&mut self, result: &serde_json::Value) -> bool {
        if result.is_null() {
            // Hover sin resultado — limpiar
            self.hover_content = None;
            return false;
        }

        // El hover result puede tener contents como string, MarkupContent, o array
        let content = if let Some(contents) = result.get("contents") {
            if let Some(s) = contents.as_str() {
                s.to_string()
            } else if let Some(obj) = contents.as_object() {
                // MarkupContent { kind, value }
                obj.get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            } else if let Some(arr) = contents.as_array() {
                // Array de MarkedString
                arr.iter()
                    .filter_map(|item| {
                        if let Some(s) = item.as_str() {
                            Some(s.to_string())
                        } else if let Some(obj) = item.as_object() {
                            obj.get("value").and_then(|v| v.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                return false;
            }
        } else {
            return false;
        };

        if content.is_empty() {
            self.hover_content = None;
            return false;
        }

        self.hover_content = Some(HoverInfo {
            content,
            line: 0,
            col: 0,
        });
        true
    }

    /// Intenta parsear una response como completion result.
    fn try_parse_completion(&mut self, result: &serde_json::Value) -> bool {
        if result.is_null() {
            return false;
        }

        // Completion result puede ser un array directo o un CompletionList
        let items = if let Some(arr) = result.as_array() {
            arr
        } else if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
            items
        } else {
            return false;
        };

        self.completions = items
            .iter()
            .filter_map(|item| {
                let label = item.get("label")?.as_str()?.to_string();
                let detail = item
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let kind = item
                    .get("kind")
                    .and_then(|v| v.as_u64())
                    .map(completion_kind_to_string);
                let insert_text = item
                    .get("insertText")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                Some(CompletionItem {
                    label,
                    detail,
                    kind,
                    insert_text,
                })
            })
            .collect();

        if !self.completions.is_empty() {
            self.completion_visible = true;
            self.completion_selected = 0;
        }

        true
    }

    /// Intenta parsear una response como definition result.
    fn try_parse_definition(&mut self, result: &serde_json::Value) -> bool {
        if result.is_null() {
            return false;
        }

        // Definition puede ser Location, Location[], o LocationLink[]
        let location = if result.get("uri").is_some() {
            // Location directa
            Some(result)
        } else if let Some(arr) = result.as_array() {
            // Array de locations — tomar la primera
            arr.first()
        } else {
            None
        };

        let Some(loc) = location else {
            return false;
        };

        let uri = loc.get("uri").and_then(|v| v.as_str());
        // Puede ser "targetUri" para LocationLink
        let uri = uri.or_else(|| loc.get("targetUri").and_then(|v| v.as_str()));

        let Some(uri) = uri else {
            return false;
        };

        let range = loc.get("range").or_else(|| loc.get("targetRange"));

        let (line, col) = if let Some(range) = range {
            let start = range.get("start");
            (
                start
                    .and_then(|s| s.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                start
                    .and_then(|s| s.get("character"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            )
        } else {
            (0, 0)
        };

        self.definition_result = Some(DefinitionResult {
            uri: uri.to_string(),
            line,
            col,
        });

        true
    }

    /// Notifica al server que se abrió un archivo.
    pub fn notify_open(&mut self, path: &Path, text: &str) -> Result<()> {
        let Some(ref mut client) = self.client else {
            return Ok(());
        };

        let uri = path_to_uri(path);
        let language_id = detect_language_id(path);

        // Inicializar version counter
        self.document_versions.insert(uri.clone(), 1);
        // CLONE: necesario — uri se usa después como key en HashMap y como param de did_open

        client.did_open(&uri, language_id, text)
    }

    /// Notifica al server que el contenido cambió.
    ///
    /// Implementa debounce simple: solo envía si pasaron más de 300ms
    /// desde la última notificación. Si no, marca como pendiente.
    pub fn notify_change(&mut self, path: &Path, text: &str) -> Result<()> {
        let Some(ref mut client) = self.client else {
            return Ok(());
        };

        let uri = path_to_uri(path);
        let now = Instant::now();

        // Debounce: verificar si pasó suficiente tiempo
        if let Some(last) = self.last_change_time
            && now.duration_since(last).as_millis() < DEBOUNCE_MS
        {
            self.change_pending = true;
            self.pending_change_uri = Some(uri);
            return Ok(());
        }

        // Incrementar versión del documento
        let version = self
            .document_versions
            .entry(uri.clone())
            // CLONE: necesario — uri se necesita como key y como param
            .and_modify(|v| *v += 1)
            .or_insert(1);

        client.did_change(&uri, text, *version)?;
        self.last_change_time = Some(now);
        self.change_pending = false;

        Ok(())
    }

    /// Envía el cambio pendiente si el debounce expiró.
    ///
    /// Debe llamarse en cada tick del event loop.
    pub fn flush_pending_change(&mut self, get_text: impl FnOnce(&str) -> Option<String>) {
        if !self.change_pending {
            return;
        }

        let Some(ref last) = self.last_change_time else {
            return;
        };

        if Instant::now().duration_since(*last).as_millis() < DEBOUNCE_MS {
            return;
        }

        // CLONE: necesario — pending_change_uri se consume y necesitamos el String
        let Some(uri) = self.pending_change_uri.take() else {
            return;
        };

        if let Some(text) = get_text(&uri) {
            let version = self
                .document_versions
                .entry(uri.clone())
                // CLONE: necesario — uri se necesita como key y como param
                .and_modify(|v| *v += 1)
                .or_insert(1);

            if let Some(ref mut client) = self.client
                && let Err(e) = client.did_change(&uri, &text, *version)
            {
                tracing::warn!(error = %e, "error en flush de did_change pendiente");
            }
        }

        self.change_pending = false;
        self.last_change_time = Some(Instant::now());
    }

    /// Envía request de hover en la posición dada.
    pub fn request_hover(&mut self, path: &Path, line: u32, col: u32) -> Result<()> {
        let Some(ref mut client) = self.client else {
            return Ok(());
        };

        let uri = path_to_uri(path);
        client.hover(&uri, line, col)?;
        Ok(())
    }

    /// Envía request de go-to-definition en la posición dada.
    pub fn request_definition(&mut self, path: &Path, line: u32, col: u32) -> Result<()> {
        let Some(ref mut client) = self.client else {
            return Ok(());
        };

        let uri = path_to_uri(path);
        client.goto_definition(&uri, line, col)?;
        Ok(())
    }

    /// Envía request de completion en la posición dada.
    pub fn request_completion(&mut self, path: &Path, line: u32, col: u32) -> Result<()> {
        let Some(ref mut client) = self.client else {
            return Ok(());
        };

        let uri = path_to_uri(path);
        client.completion(&uri, line, col)?;
        Ok(())
    }

    /// Para el language server.
    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut client) = self.client.take() {
            client.shutdown()?;
        }
        self.diagnostics.clear();
        self.hover_content = None;
        self.completions.clear();
        self.completion_visible = false;
        self.document_versions.clear();
        self.definition_result = None;
        tracing::info!("LSP server detenido");
        Ok(())
    }

    /// Retorna los diagnósticos para un archivo dado.
    pub fn diagnostics_for(&self, path: &Path) -> &[Diagnostic] {
        let uri = path_to_uri(path);
        self.diagnostics.get(&uri).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Verifica si hay un server activo.
    pub fn has_server(&self) -> bool {
        self.client.is_some()
    }

    /// Actualiza el status message basándose en la posición del cursor.
    ///
    /// Si el cursor está sobre una línea con diagnósticos, muestra el mensaje.
    pub fn update_status_for_cursor(&mut self, path: &Path, cursor_line: u32) {
        let uri = path_to_uri(path);
        self.status_message = self.diagnostics.get(&uri).and_then(|diags| {
            diags.iter().find(|d| d.line == cursor_line).map(|d| {
                let severity_str = match d.severity {
                    DiagnosticSeverity::Error => "Error",
                    DiagnosticSeverity::Warning => "Warning",
                    DiagnosticSeverity::Information => "Info",
                    DiagnosticSeverity::Hint => "Hint",
                };
                format!("{severity_str}: {}", d.message)
            })
        });
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

/// Convierte un path del filesystem a URI `file:///`.
///
/// En Windows: `C:\Users\foo` → `file:///C:/Users/foo`
/// En Unix: `/home/foo` → `file:///home/foo`
pub fn path_to_uri(path: &Path) -> String {
    // Canonicalizar para resolver symlinks y paths relativos
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    let path_str = canonical.to_string_lossy();

    // En Windows, el path viene con `\\?\` prefix de canonicalize y backslashes
    #[cfg(windows)]
    {
        let clean = path_str.strip_prefix(r"\\?\").unwrap_or(&path_str);
        let forward = clean.replace('\\', "/");
        format!("file:///{forward}")
    }

    #[cfg(not(windows))]
    {
        format!("file://{path_str}")
    }
}

/// Convierte un URI `file:///` a un path del filesystem.
///
/// Inversa de `path_to_uri`. Maneja URIs tanto de Windows como Unix.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let path_str = uri.strip_prefix("file:///")?;
    // Decodificar percent-encoding básico (espacios, etc.)
    let decoded = path_str.replace("%20", " ");

    #[cfg(windows)]
    {
        // En Windows: `C:/Users/foo` → `C:\Users\foo`
        let native = decoded.replace('/', "\\");
        Some(std::path::PathBuf::from(native))
    }

    #[cfg(not(windows))]
    {
        Some(std::path::PathBuf::from(format!("/{decoded}")))
    }
}

/// Detecta el language ID para el protocolo LSP basándose en la extensión.
fn detect_language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "go" => "go",
        "lua" => "lua",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" | "hpp" => "cpp",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "html" => "html",
        "css" => "css",
        _ => "plaintext",
    }
}

/// Auto-detecta el comando del language server para una extensión de archivo.
///
/// Retorna (comando, args) o None si no hay server conocido.
pub fn detect_language_server(extension: &str) -> Option<(&'static str, &'static [&'static str])> {
    match extension {
        "rs" => Some(("rust-analyzer", &[])),
        "py" => Some(("pylsp", &[])),
        "ts" | "tsx" | "js" | "jsx" => Some(("typescript-language-server", &["--stdio"])),
        "go" => Some(("gopls", &[])),
        "lua" => Some(("lua-language-server", &[])),
        _ => None,
    }
}

/// Convierte el número de kind de completion del protocolo LSP a string legible.
fn completion_kind_to_string(kind: u64) -> String {
    match kind {
        1 => "text".into(),
        2 => "method".into(),
        3 => "function".into(),
        4 => "constructor".into(),
        5 => "field".into(),
        6 => "variable".into(),
        7 => "class".into(),
        8 => "interface".into(),
        9 => "module".into(),
        10 => "property".into(),
        11 => "unit".into(),
        12 => "value".into(),
        13 => "enum".into(),
        14 => "keyword".into(),
        15 => "snippet".into(),
        16 => "color".into(),
        17 => "file".into(),
        18 => "reference".into(),
        19 => "folder".into(),
        20 => "enum_member".into(),
        21 => "constant".into(),
        22 => "struct".into(),
        23 => "event".into(),
        24 => "operator".into(),
        25 => "type_param".into(),
        _ => "unknown".into(),
    }
}
