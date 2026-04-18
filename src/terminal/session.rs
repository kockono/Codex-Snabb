//! Session: sesión de terminal con PTY real via `portable-pty`.
//!
//! Gestiona una sesión de shell interactiva: spawn del proceso, lectura
//! non-blocking de output via thread dedicado + bounded channel, escritura
//! de input al PTY, scroll del scrollback, y resize.
//!
//! El diseño sigue la arquitectura de workers: un thread dedicado lee
//! del PTY y envía a un bounded channel. El event loop drena el channel
//! sin bloquear via `try_recv()`.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Flag atómico para señalizar stop al thread lector.
/// Más liviano que `CancellationToken` para un thread de std.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Capacidad del bounded channel entre el thread lector y el event loop.
/// 1024 chunks es suficiente para absorber ráfagas de output
/// sin consumir memoria excesiva.
const READER_CHANNEL_CAPACITY: usize = 1024;

/// Tamaño del buffer de lectura del PTY en bytes.
/// 4KB es un buen balance entre syscalls y memoria.
const READ_BUFFER_SIZE: usize = 4096;

/// Sesión de terminal con PTY real.
///
/// Encapsula el proceso hijo, el writer para enviar input,
/// un bounded channel para recibir output sin bloquear,
/// y el scrollback con scroll.
pub struct TerminalSession {
    /// Writer del master PTY para enviar input al shell.
    writer: Box<dyn Write + Send>,
    /// Handle del master PTY (necesario para resize).
    master: Box<dyn MasterPty + Send>,
    /// Receiver del bounded channel — drena output del thread lector.
    output_rx: mpsc::Receiver<Vec<u8>>,
    /// Líneas de output acumuladas (scrollback).
    output_lines: Vec<String>,
    /// Línea actual en construcción (antes del próximo `\n`).
    current_line: String,
    /// Límite máximo de líneas en scrollback.
    max_scrollback: usize,
    /// Offset de scroll desde abajo: 0 = ver las más recientes.
    scroll_offset: usize,
    /// Flag para detener el thread lector en drop.
    stop_flag: Arc<AtomicBool>,
    /// Handle del thread lector para join en drop.
    reader_thread: Option<thread::JoinHandle<()>>,
}

impl TerminalSession {
    /// Crea una nueva sesión de terminal con un shell.
    ///
    /// Detecta el shell disponible en Windows (`powershell.exe` o `cmd.exe`).
    /// Lanza el PTY con `portable-pty` y crea un thread dedicado para
    /// leer output y enviarlo a un bounded channel.
    pub fn spawn(working_dir: &Path, size: (u16, u16)) -> Result<Self> {
        let shell = detect_shell();
        tracing::info!(shell = %shell, "lanzando terminal");

        let pty_system = native_pty_system();

        let pty_size = PtySize {
            rows: size.1,
            cols: size.0,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(pty_size)
            .context("no se pudo abrir PTY")?;

        let mut cmd = CommandBuilder::new(&shell);
        cmd.cwd(working_dir);

        // Spawn del proceso en el slave del PTY
        let _child = pair
            .slave
            .spawn_command(cmd)
            .context("no se pudo lanzar shell en PTY")?;

        // Obtener reader y writer del master
        let reader = pair
            .master
            .try_clone_reader()
            .context("no se pudo clonar reader del PTY")?;
        let writer = pair
            .master
            .take_writer()
            .context("no se pudo tomar writer del PTY")?;

        // Bounded channel para output del thread lector
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(READER_CHANNEL_CAPACITY);

        // Flag de stop para el thread lector
        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop_flag);

        // Thread dedicado que lee del PTY y envía al channel
        let reader_thread = thread::Builder::new()
            .name("terminal-reader".into())
            .spawn(move || {
                reader_loop(reader, tx, thread_stop);
            })
            .context("no se pudo crear thread de lectura del terminal")?;

        Ok(Self {
            writer,
            master: pair.master,
            output_rx: rx,
            output_lines: Vec::with_capacity(256),
            current_line: String::with_capacity(256),
            max_scrollback: 5000,
            scroll_offset: 0,
            stop_flag,
            reader_thread: Some(reader_thread),
        })
    }

    /// Drena todo el output disponible del PTY sin bloquear.
    ///
    /// Retorna `true` si hubo nuevo output (para invalidar el render).
    /// Parsea el output byte a byte: `\n` crea nueva línea,
    /// `\r` vuelve al inicio de la línea actual.
    pub fn poll_output(&mut self) -> Result<bool> {
        let mut had_output = false;

        // Drenar todos los chunks disponibles sin bloquear
        while let Ok(chunk) = self.output_rx.try_recv() {
            had_output = true;
            self.process_bytes(&chunk);
        }

        // Si hubo output nuevo, resetear scroll a bottom
        if had_output {
            self.scroll_offset = 0;
        }

        Ok(had_output)
    }

    /// Procesa bytes crudos del PTY y los agrega al scrollback.
    ///
    /// Lógica de parsing:
    /// - `\n` → empuja la línea actual al scrollback, empieza nueva línea
    /// - `\r` → vuelve al inicio de la línea actual (overwrites)
    /// - Otros bytes → se agregan a la línea actual
    fn process_bytes(&mut self, data: &[u8]) {
        for &byte in data {
            match byte {
                b'\n' => {
                    // Empujar línea actual al scrollback
                    let line =
                        std::mem::replace(&mut self.current_line, String::with_capacity(256));
                    self.output_lines.push(line);

                    // Trimming del scrollback si excede el límite
                    if self.output_lines.len() > self.max_scrollback {
                        let excess = self.output_lines.len() - self.max_scrollback;
                        self.output_lines.drain(..excess);
                    }
                }
                b'\r' => {
                    // Carriage return — volver al inicio de la línea actual
                    self.current_line.clear();
                }
                // Filtrar caracteres de control que no son printables
                // (excepto tab que sí queremos mostrar)
                b'\t' => {
                    self.current_line.push_str("    "); // Tab → 4 espacios
                }
                0x00..=0x06 | 0x0e..=0x1a | 0x1c..=0x1f => {
                    // Caracteres de control no-printables — ignorar
                }
                0x1b => {
                    // ESC — inicio de secuencia ANSI, ignorar por ahora.
                    // TODO: parsear secuencias ANSI para colores/cursor
                }
                0x07 => {
                    // BEL — bell, ignorar silenciosamente
                }
                _ => {
                    // Carácter printable — agregar a línea actual
                    if let Some(ch) = char::from_u32(byte as u32) {
                        self.current_line.push(ch);
                    }
                }
            }
        }
    }

    /// Envía un carácter de input al PTY.
    pub fn send_key(&mut self, ch: char) -> Result<()> {
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        self.writer
            .write_all(encoded.as_bytes())
            .context("error al enviar key al terminal")?;
        self.writer
            .flush()
            .context("error al flush del writer del terminal")?;
        Ok(())
    }

    /// Envía Enter al PTY (`\r\n` en Windows).
    pub fn send_enter(&mut self) -> Result<()> {
        self.writer
            .write_all(b"\r\n")
            .context("error al enviar Enter al terminal")?;
        self.writer
            .flush()
            .context("error al flush del writer del terminal")?;
        Ok(())
    }

    /// Envía Ctrl+C al PTY (byte 0x03 — ETX).
    pub fn send_ctrl_c(&mut self) -> Result<()> {
        self.writer
            .write_all(&[0x03])
            .context("error al enviar Ctrl+C al terminal")?;
        self.writer
            .flush()
            .context("error al flush del writer del terminal")?;
        Ok(())
    }

    /// Redimensiona el PTY.
    #[expect(
        dead_code,
        reason = "se usará cuando se implemente resize dinámico del terminal"
    )]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("error al resize del PTY")?;
        Ok(())
    }

    /// Retorna las líneas visibles para el viewport de height líneas.
    ///
    /// `scroll_offset == 0` muestra las líneas más recientes.
    /// Incluye la `current_line` (en construcción) como última línea visible.
    pub fn visible_lines(&self, height: usize) -> Vec<&str> {
        if height == 0 {
            return Vec::new();
        }

        // Todas las líneas = output_lines + current_line
        let total = self.output_lines.len() + 1; // +1 por current_line

        // Calcular rango visible considerando scroll_offset
        let end = total.saturating_sub(self.scroll_offset);
        let start = end.saturating_sub(height);

        let mut result = Vec::with_capacity(height);

        for i in start..end {
            if i < self.output_lines.len() {
                result.push(self.output_lines[i].as_str());
            } else {
                // Es la current_line (última)
                result.push(self.current_line.as_str());
            }
        }

        result
    }

    /// Scrollea hacia arriba N líneas.
    pub fn scroll_up(&mut self, lines: usize) {
        let total = self.output_lines.len() + 1;
        let max_offset = total.saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
    }

    /// Scrollea hacia abajo N líneas (hacia las más recientes).
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Cantidad total de líneas en el scrollback (incluyendo current_line).
    #[expect(dead_code, reason = "se usará para mostrar indicador de scroll en UI")]
    pub fn line_count(&self) -> usize {
        self.output_lines.len() + 1
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Señalizar al thread lector que se detenga
        self.stop_flag.store(true, Ordering::Relaxed);

        // Esperar a que el thread termine (con timeout implícito —
        // el thread sale cuando el read falla o ve el stop flag)
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

// Necesitamos implementar Debug manualmente porque Box<dyn Write>
// y Box<dyn MasterPty> no implementan Debug.
impl std::fmt::Debug for TerminalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSession")
            .field("output_lines", &self.output_lines.len())
            .field("current_line_len", &self.current_line.len())
            .field("max_scrollback", &self.max_scrollback)
            .field("scroll_offset", &self.scroll_offset)
            .finish_non_exhaustive()
    }
}

// ─── Reader Thread ─────────────────────────────────────────────────────────────

/// Loop de lectura del thread dedicado.
///
/// Lee del PTY en chunks y envía al bounded channel.
/// Se detiene cuando: el read retorna 0 bytes (child terminó),
/// el read falla (PTY cerrado), o el stop_flag está activo.
fn reader_loop(
    mut reader: Box<dyn std::io::Read + Send>,
    tx: mpsc::SyncSender<Vec<u8>>,
    stop_flag: Arc<AtomicBool>,
) {
    let mut buf = vec![0u8; READ_BUFFER_SIZE];

    loop {
        // Verificar stop flag antes de cada read
        if stop_flag.load(Ordering::Relaxed) {
            tracing::debug!("terminal reader: stop flag activo, saliendo");
            break;
        }

        match reader.read(&mut buf) {
            Ok(0) => {
                // EOF — el proceso hijo terminó
                tracing::info!("terminal reader: EOF — proceso terminado");
                break;
            }
            Ok(n) => {
                // Enviar chunk al channel. Si el channel está lleno,
                // el send bloquea (backpressure). Si el receiver se dropea,
                // el send falla y salimos.
                if tx.send(buf[..n].to_vec()).is_err() {
                    tracing::debug!("terminal reader: channel cerrado, saliendo");
                    break;
                }
            }
            Err(e) => {
                // Error de lectura — PTY cerrado o error de IO
                if !stop_flag.load(Ordering::Relaxed) {
                    tracing::warn!(error = %e, "terminal reader: error de lectura");
                }
                break;
            }
        }
    }
}

// ─── Shell Detection ───────────────────────────────────────────────────────────

/// Detecta el shell disponible en el sistema.
///
/// En Windows: intenta `powershell.exe` primero (más moderno),
/// fallback a `cmd.exe`. En otros OS: usa la variable `SHELL`
/// o fallback a `/bin/sh`.
fn detect_shell() -> String {
    if cfg!(windows) {
        // Intentar PowerShell primero
        if which_exists("powershell.exe") {
            return "powershell.exe".to_string();
        }
        // Fallback a cmd.exe (siempre disponible en Windows)
        "cmd.exe".to_string()
    } else {
        // Unix: usar variable SHELL o fallback
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// Verifica si un ejecutable existe en el PATH.
fn which_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("/?") // Argumento inocuo que no ejecuta nada pesado
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            let _ = child.kill();
            let _ = child.wait();
            true
        })
        .unwrap_or(false)
}
