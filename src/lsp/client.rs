//! Cliente LSP austero: comunicación con language servers via JSON-RPC.
//!
//! Maneja el ciclo de vida del proceso del server, el handshake initialize,
//! y los requests/notifications del protocolo LSP. No usa un crate client
//! completo — es más liviano y controlable para un IDE con budgets de RAM.
//!
//! El cliente envía requests asíncronamente y recoge responses via polling.
//! Cada request pendiente se trackea por id para matchear con la response.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};
use lsp_types::{
    ClientCapabilities, HoverProviderCapability, InitializeParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, WorkspaceFolder,
};

use super::transport::LspTransport;

/// Mensaje parseado del language server.
///
/// Discrimina entre responses a requests, errores, y notifications push.
#[derive(Debug)]
pub enum LspMessage {
    /// Response exitosa a un request previo.
    Response {
        /// ID del request original.
        id: i64,
        /// Resultado JSON del server.
        result: serde_json::Value,
    },
    /// Response de error a un request previo.
    Error {
        /// ID del request original.
        id: i64,
        /// Código de error LSP.
        code: i64,
        /// Mensaje descriptivo del error.
        message: String,
    },
    /// Notification push del server (ej: diagnostics, progress).
    Notification {
        /// Método de la notification.
        method: String,
        /// Parámetros JSON de la notification.
        params: serde_json::Value,
    },
}

/// Cliente LSP que maneja la comunicación con un language server.
///
/// El cliente:
/// - Lanza el server como subproceso
/// - Ejecuta el handshake initialize/initialized
/// - Envía requests y notifications del protocolo
/// - Recoge y parsea responses vía polling non-blocking
pub struct LspClient {
    /// Proceso del language server.
    process: Child,
    /// Transporte JSON-RPC sobre stdin/stdout.
    transport: LspTransport,
    /// Siguiente ID para requests (monotónicamente creciente).
    next_id: i64,
    /// Capabilities reportadas por el server en initialize response.
    server_capabilities: Option<ServerCapabilities>,
    /// Requests pendientes: id → nombre del método (para matchear responses).
    pending_requests: HashMap<i64, String>,
    /// Si el handshake initialize/initialized se completó.
    initialized: bool,
}

impl std::fmt::Debug for LspClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspClient")
            .field("next_id", &self.next_id)
            .field("initialized", &self.initialized)
            .field("pending_requests", &self.pending_requests.len())
            .finish()
    }
}

impl LspClient {
    /// Lanza un language server como subproceso y crea el cliente.
    ///
    /// El server se lanza con stdin/stdout como pipes para JSON-RPC.
    /// stderr se hereda para que los logs del server vayan al log del IDE.
    pub fn start(command: &str, args: &[&str], root_uri: &str) -> Result<Self> {
        let process = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // Silenciar stderr del server
            .spawn()
            .with_context(|| format!("no se pudo lanzar LSP server: {command}"))?;

        let stdin = process.stdin.as_ref().is_some();
        let stdout = process.stdout.as_ref().is_some();
        if !stdin || !stdout {
            bail!("LSP server sin stdin/stdout pipes");
        }

        // SAFETY: acabamos de verificar que son Some
        // Necesitamos tomar ownership de stdin/stdout para el transporte.
        // El compilador no permite moverlos de &Child, así que re-creamos
        // el Child con los pipes extraídos.
        let mut process = process;
        let child_stdin = process
            .stdin
            .take()
            .context("stdin del LSP server no disponible")?;
        let child_stdout = process
            .stdout
            .take()
            .context("stdout del LSP server no disponible")?;

        let transport = LspTransport::new(child_stdin, child_stdout);

        let mut client = Self {
            process,
            transport,
            next_id: 1,
            server_capabilities: None,
            pending_requests: HashMap::new(),
            initialized: false,
        };

        // Ejecutar handshake
        client.initialize(root_uri)?;

        Ok(client)
    }

    /// Envía el request `initialize` y espera la response (blocking).
    ///
    /// Extrae las ServerCapabilities de la response para saber qué features
    /// soporta el server. Luego envía la notification `initialized`.
    fn initialize(&mut self, root_uri: &str) -> Result<()> {
        let root_lsp_uri: lsp_types::Uri = root_uri
            .parse()
            .map_err(|e| anyhow::anyhow!("URI inválido: {root_uri}: {e}"))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            #[expect(
                deprecated,
                reason = "algunos servers aún usan root_uri — lo enviamos por compatibilidad"
            )]
            root_uri: Some(root_lsp_uri.clone()),
            // CLONE: necesario — root_lsp_uri se usa para root_uri y workspace_folders
            capabilities: ClientCapabilities {
                text_document: Some(lsp_types::TextDocumentClientCapabilities {
                    completion: Some(lsp_types::CompletionClientCapabilities {
                        completion_item: Some(lsp_types::CompletionItemCapability {
                            snippet_support: Some(false),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    hover: Some(lsp_types::HoverClientCapabilities {
                        content_format: Some(vec![lsp_types::MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    definition: Some(lsp_types::GotoCapability {
                        ..Default::default()
                    }),
                    publish_diagnostics: Some(lsp_types::PublishDiagnosticsClientCapabilities {
                        ..Default::default()
                    }),
                    synchronization: Some(lsp_types::TextDocumentSyncClientCapabilities {
                        did_save: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_lsp_uri,
                name: String::from("workspace"),
            }]),
            ..Default::default()
        };

        let params_json =
            serde_json::to_value(&params).context("error serializando InitializeParams")?;

        let id = self.next_id;
        self.next_id += 1;
        self.pending_requests.insert(id, String::from("initialize"));
        self.transport.send_request(id, "initialize", params_json)?;

        // Esperar response del initialize — blocking con timeout
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if std::time::Instant::now() > deadline {
                bail!("timeout esperando initialize response del LSP server");
            }

            if let Some(raw) = self.transport.try_recv()
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
                && json.get("id").and_then(|v| v.as_i64()) == Some(id)
            {
                if let Some(result) = json.get("result") {
                    // Parsear ServerCapabilities
                    if let Some(caps) = result.get("capabilities") {
                        self.server_capabilities =
                            serde_json::from_value(caps.clone()).ok();
                        // CLONE: necesario — caps es referencia a json que no podemos mover
                    }
                }
                self.pending_requests.remove(&id);
                break;
            }

            // Yield para no saturar CPU
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Enviar notification initialized
        self.initialized_notification()?;
        self.initialized = true;

        tracing::info!("LSP server inicializado correctamente");
        Ok(())
    }

    /// Envía la notification `initialized` al server.
    ///
    /// Debe llamarse después de recibir la response a `initialize`.
    fn initialized_notification(&mut self) -> Result<()> {
        self.transport
            .send_notification("initialized", serde_json::json!({}))
    }

    /// Notifica al server que se abrió un archivo.
    pub fn did_open(&mut self, uri: &str, language_id: &str, text: &str) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text,
            }
        });
        self.transport
            .send_notification("textDocument/didOpen", params)
    }

    /// Notifica al server que el contenido de un archivo cambió.
    ///
    /// Usa full document sync para MVP — envía todo el texto.
    /// Incremental sync es más eficiente pero más complejo de implementar.
    pub fn did_change(&mut self, uri: &str, text: &str, version: i32) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "version": version,
            },
            "contentChanges": [{
                "text": text,
            }]
        });
        self.transport
            .send_notification("textDocument/didChange", params)
    }

    /// Notifica al server que se cerró un archivo.
    #[expect(dead_code, reason = "se usará cuando se implemente cierre de buffers")]
    pub fn did_close(&mut self, uri: &str) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
            }
        });
        self.transport
            .send_notification("textDocument/didClose", params)
    }

    /// Envía un request de hover en la posición dada.
    pub fn hover(&mut self, uri: &str, line: u32, character: u32) -> Result<i64> {
        let id = self.next_id;
        self.next_id += 1;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        self.pending_requests
            .insert(id, String::from("textDocument/hover"));
        self.transport
            .send_request(id, "textDocument/hover", params)?;
        Ok(id)
    }

    /// Envía un request de go-to-definition en la posición dada.
    pub fn goto_definition(&mut self, uri: &str, line: u32, character: u32) -> Result<i64> {
        let id = self.next_id;
        self.next_id += 1;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        self.pending_requests
            .insert(id, String::from("textDocument/definition"));
        self.transport
            .send_request(id, "textDocument/definition", params)?;
        Ok(id)
    }

    /// Envía un request de completion en la posición dada.
    pub fn completion(&mut self, uri: &str, line: u32, character: u32) -> Result<i64> {
        let id = self.next_id;
        self.next_id += 1;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        self.pending_requests
            .insert(id, String::from("textDocument/completion"));
        self.transport
            .send_request(id, "textDocument/completion", params)?;
        Ok(id)
    }

    /// Lee todos los mensajes disponibles del server y los parsea.
    ///
    /// Non-blocking — retorna inmediatamente si no hay mensajes.
    /// Consume todos los mensajes pendientes en el canal.
    pub fn poll_messages(&mut self) -> Vec<LspMessage> {
        let mut messages = Vec::new();

        while let Some(raw) = self.transport.try_recv() {
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(json) => {
                    if let Some(msg) = self.parse_message(json) {
                        messages.push(msg);
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "LSP: mensaje JSON inválido");
                }
            }
        }

        messages
    }

    /// Parsea un mensaje JSON del server en un LspMessage tipado.
    fn parse_message(&mut self, json: serde_json::Value) -> Option<LspMessage> {
        // Response (tiene "id" y "result" o "error")
        if let Some(id) = json.get("id").and_then(|v| v.as_i64()) {
            // Limpiar de pending requests
            self.pending_requests.remove(&id);

            if let Some(error) = json.get("error") {
                return Some(LspMessage::Error {
                    id,
                    code: error.get("code").and_then(|v| v.as_i64()).unwrap_or(-1),
                    message: error
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }

            if let Some(result) = json.get("result") {
                // CLONE: necesario — result es referencia a json, necesitamos ownership
                return Some(LspMessage::Response {
                    id,
                    result: result.clone(),
                });
            }

            return None;
        }

        // Notification (tiene "method" pero no "id")
        if let Some(method) = json.get("method").and_then(|v| v.as_str()) {
            let params = json
                .get("params")
                .cloned() // CLONE: necesario — params es referencia a json
                .unwrap_or(serde_json::Value::Null);
            return Some(LspMessage::Notification {
                method: method.to_string(),
                params,
            });
        }

        tracing::debug!("LSP: mensaje sin id ni method ignorado");
        None
    }

    /// Envía shutdown request + exit notification al server.
    ///
    /// Espera brevemente la response de shutdown antes de enviar exit.
    /// Si el server no responde, mata el proceso.
    pub fn shutdown(&mut self) -> Result<()> {
        if !self.initialized {
            // Matar el proceso directamente si no se inicializó
            let _ = self.process.kill();
            let _ = self.process.wait();
            return Ok(());
        }

        // Enviar shutdown request
        let id = self.next_id;
        self.next_id += 1;
        self.transport
            .send_request(id, "shutdown", serde_json::Value::Null)?;

        // Esperar response brevemente (2s)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Some(raw) = self.transport.try_recv()
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw)
                && json.get("id").and_then(|v| v.as_i64()) == Some(id)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Enviar exit notification
        let _ = self
            .transport
            .send_notification("exit", serde_json::Value::Null);

        // Esperar que el proceso termine (500ms) o matarlo
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            match self.process.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if std::time::Instant::now() > deadline {
                        tracing::warn!("LSP server no terminó, matando proceso");
                        let _ = self.process.kill();
                        let _ = self.process.wait();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "error esperando cierre del LSP server");
                    let _ = self.process.kill();
                    break;
                }
            }
        }

        self.initialized = false;
        tracing::info!("LSP server cerrado");
        Ok(())
    }

    /// Verifica si el server soporta hover.
    #[expect(
        dead_code,
        reason = "se usará para verificar capabilities antes de requests"
    )]
    pub fn supports_hover(&self) -> bool {
        self.server_capabilities.as_ref().is_some_and(|caps| {
            caps.hover_provider
                .as_ref()
                .is_some_and(|hp| !matches!(hp, HoverProviderCapability::Simple(false)))
        })
    }

    /// Verifica si el server soporta go-to-definition.
    #[expect(
        dead_code,
        reason = "se usará para verificar capabilities antes de requests"
    )]
    pub fn supports_definition(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .is_some_and(|caps| caps.definition_provider.is_some())
    }

    /// Verifica si el server soporta completion.
    #[expect(
        dead_code,
        reason = "se usará para verificar capabilities antes de requests"
    )]
    pub fn supports_completion(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .is_some_and(|caps| caps.completion_provider.is_some())
    }

    /// Obtiene el sync kind soportado por el server.
    #[expect(dead_code, reason = "se usará para optimizar did_change")]
    pub fn text_document_sync_kind(&self) -> TextDocumentSyncKind {
        self.server_capabilities
            .as_ref()
            .and_then(|caps| {
                caps.text_document_sync.as_ref().map(|sync| match sync {
                    TextDocumentSyncCapability::Kind(kind) => *kind,
                    TextDocumentSyncCapability::Options(opts) => {
                        opts.change.unwrap_or(TextDocumentSyncKind::FULL)
                    }
                })
            })
            .unwrap_or(TextDocumentSyncKind::FULL)
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Err(e) = self.shutdown() {
            tracing::warn!(error = %e, "error en shutdown del LSP client durante drop");
        }
    }
}
