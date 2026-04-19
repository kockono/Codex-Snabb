//! Transporte JSON-RPC sobre stdin/stdout para el protocolo LSP.
//!
//! El protocolo LSP usa JSON-RPC 2.0 con headers HTTP-like:
//!
//! ```text
//! Content-Length: 123\r\n
//! \r\n
//! {"jsonrpc":"2.0","method":"initialize","id":1,"params":{...}}
//! ```
//!
//! El transporte maneja la serialización de headers y el framing de mensajes.
//! Un thread lector dedicado consume stdout del server de forma non-blocking
//! para el event loop principal.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc;

use anyhow::{bail, Context, Result};

/// Capacidad del canal bounded entre el reader thread y el consumer.
/// 64 mensajes es suficiente para absorber ráfagas de diagnósticos
/// sin consumir memoria descontrolada.
const CHANNEL_CAPACITY: usize = 64;

/// Transporte JSON-RPC bidireccional sobre pipes de proceso.
///
/// El writer envía mensajes al stdin del server.
/// El reader thread lee del stdout del server y envía mensajes
/// por un canal bounded al consumer (el event loop).
pub struct LspTransport {
    /// Writer hacia el stdin del language server.
    writer: BufWriter<ChildStdin>,
    /// Receptor de mensajes parseados del server.
    reader_rx: mpsc::Receiver<String>,
    /// Handle del thread lector — se joinea en drop.
    /// Option para poder tomar ownership en drop.
    _reader_handle: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for LspTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspTransport")
            .field("writer", &"BufWriter<ChildStdin>")
            .field("reader_rx", &"mpsc::Receiver<String>")
            .finish()
    }
}

impl LspTransport {
    /// Crea un nuevo transporte LSP.
    ///
    /// Spawnea un thread dedicado que lee stdout del server, parsea
    /// los headers `Content-Length`, y envía el body por un canal bounded.
    /// Si el canal se llena (backpressure), el reader thread bloquea
    /// hasta que el consumer consuma mensajes.
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        let (tx, rx) = mpsc::sync_channel::<String>(CHANNEL_CAPACITY);

        let handle = std::thread::Builder::new()
            .name("lsp-reader".into())
            .spawn(move || {
                Self::reader_loop(stdout, tx);
            })
            // SAFETY: thread::spawn solo falla si el OS no puede crear threads
            .expect("no se pudo crear thread lsp-reader");

        Self {
            writer: BufWriter::new(stdin),
            reader_rx: rx,
            _reader_handle: Some(handle),
        }
    }

    /// Loop del thread lector: lee mensajes JSON-RPC del stdout del server.
    ///
    /// Parsea headers HTTP-like para obtener Content-Length, luego lee
    /// exactamente esa cantidad de bytes como body. Envía el body completo
    /// por el canal. Sale limpiamente cuando el pipe se cierra (EOF).
    fn reader_loop(stdout: ChildStdout, tx: mpsc::SyncSender<String>) {
        let mut reader = BufReader::new(stdout);
        let mut header_buf = String::with_capacity(128);

        loop {
            // Leer headers hasta encontrar línea vacía (\r\n)
            let content_length = match Self::read_headers(&mut reader, &mut header_buf) {
                Ok(Some(len)) => len,
                Ok(None) => {
                    // EOF — el server cerró stdout
                    tracing::debug!("lsp reader: EOF en stdout del server");
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "lsp reader: error leyendo headers");
                    break;
                }
            };

            // Leer body (exactamente content_length bytes)
            let mut body_buf = vec![0u8; content_length];
            if let Err(e) = reader.read_exact(&mut body_buf) {
                tracing::warn!(error = %e, "lsp reader: error leyendo body");
                break;
            }

            let body = match String::from_utf8(body_buf) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "lsp reader: body no es UTF-8 válido");
                    continue;
                }
            };

            // Enviar al consumer — bloquea si el canal está lleno (backpressure)
            if tx.send(body).is_err() {
                // El receptor se dropeó — el consumer cerró
                tracing::debug!("lsp reader: consumer cerrado, saliendo");
                break;
            }
        }
    }

    /// Lee los headers HTTP-like del mensaje LSP.
    ///
    /// Retorna el Content-Length si se encontró, None si es EOF,
    /// o error si los headers son inválidos.
    fn read_headers(
        reader: &mut BufReader<ChildStdout>,
        buf: &mut String,
    ) -> Result<Option<usize>> {
        let mut content_length: Option<usize> = None;

        loop {
            buf.clear();
            let bytes_read = reader
                .read_line(buf)
                .context("error leyendo header line del LSP server")?;

            if bytes_read == 0 {
                // EOF
                return Ok(None);
            }

            let trimmed = buf.trim();
            if trimmed.is_empty() {
                // Línea vacía = fin de headers
                break;
            }

            // Parsear "Content-Length: N"
            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .context("Content-Length no es un número válido")?,
                );
            }
            // Ignorar otros headers (Content-Type, etc.)
        }

        match content_length {
            Some(len) => Ok(Some(len)),
            None => bail!("headers LSP sin Content-Length"),
        }
    }

    /// Envía un mensaje raw con header Content-Length al server.
    ///
    /// Formato: `Content-Length: {len}\r\n\r\n{message}`
    pub fn send(&mut self, message: &str) -> Result<()> {
        let header = format!("Content-Length: {}\r\n\r\n", message.len());
        self.writer
            .write_all(header.as_bytes())
            .context("error escribiendo header al LSP server")?;
        self.writer
            .write_all(message.as_bytes())
            .context("error escribiendo body al LSP server")?;
        self.writer
            .flush()
            .context("error en flush al LSP server")?;
        Ok(())
    }

    /// Intenta recibir un mensaje del server sin bloquear.
    ///
    /// Retorna `None` si no hay mensajes disponibles.
    pub fn try_recv(&self) -> Option<String> {
        self.reader_rx.try_recv().ok()
    }

    /// Envía un JSON-RPC request (con id) al server.
    ///
    /// Construye el envelope JSON-RPC y lo envía con headers.
    pub fn send_request(&mut self, id: i64, method: &str, params: serde_json::Value) -> Result<()> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let msg = serde_json::to_string(&request).context("error serializando JSON-RPC request")?;
        self.send(&msg)
    }

    /// Envía una JSON-RPC notification (sin id) al server.
    ///
    /// Las notifications no esperan respuesta del server.
    pub fn send_notification(&mut self, method: &str, params: serde_json::Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let msg = serde_json::to_string(&notification)
            .context("error serializando JSON-RPC notification")?;
        self.send(&msg)
    }
}
