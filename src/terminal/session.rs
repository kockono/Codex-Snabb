//! Session: sesión de terminal con PTY real via `portable-pty` + `alacritty_terminal`.
//!
//! Gestiona una sesión de shell interactiva: spawn del proceso, lectura
//! non-blocking de output via thread dedicado + bounded channel, y parsing
//! VT completo via `alacritty_terminal::Term`.
//!
//! El diseño sigue la arquitectura de workers: un thread dedicado lee
//! del PTY y envía a un bounded channel. El event loop drena el channel
//! sin bloquear via `try_recv()`. Los bytes crudos se alimentan al VT
//! parser de alacritty que mantiene la grilla de celdas con atributos.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::vte::ansi::Processor as VteProcessor;
use alacritty_terminal::Term;
use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Capacidad del bounded channel entre el thread lector y el event loop.
/// 1024 chunks es suficiente para absorber ráfagas de output
/// sin consumir memoria excesiva.
const READER_CHANNEL_CAPACITY: usize = 1024;

/// Tamaño del buffer de lectura del PTY en bytes.
/// 4KB es un buen balance entre syscalls y memoria.
const READ_BUFFER_SIZE: usize = 4096;

// ─── TermDimensions ────────────────────────────────────────────────────────────

/// Implementación mínima de `Dimensions` para crear/resize un `Term`.
///
/// `alacritty_terminal::term::test::TermSize` existe pero está en un módulo
/// de test. Replicamos con los mismos campos para no depender de internals.
struct TermDimensions {
    columns: usize,
    screen_lines: usize,
}

impl TermDimensions {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            columns: cols as usize,
            screen_lines: rows as usize,
        }
    }
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

// ─── TerminalSession ───────────────────────────────────────────────────────────

/// Sesión de terminal con PTY real + VT emulator via alacritty_terminal.
///
/// Encapsula el proceso hijo, el writer para enviar input,
/// un bounded channel para recibir output sin bloquear,
/// y el `Term<VoidListener>` que mantiene la grilla con atributos.
pub struct TerminalSession {
    /// VT emulator — grilla de celdas con colores y atributos.
    pub term: Term<VoidListener>,
    /// VTE parser que procesa los bytes crudos y llama a `Handler` en `term`.
    vte_processor: VteProcessor,
    /// Writer del master PTY para enviar input al shell.
    writer: Box<dyn Write + Send>,
    /// Handle del master PTY (necesario para resize).
    master: Box<dyn MasterPty + Send>,
    /// Receiver del bounded channel — drena output del thread lector.
    output_rx: mpsc::Receiver<Vec<u8>>,
    /// Flag para detener el thread lector en drop.
    stop_flag: Arc<AtomicBool>,
    /// Handle del thread lector para join en drop.
    reader_thread: Option<thread::JoinHandle<()>>,
    /// Columnas actuales del PTY.
    cols: u16,
    /// Filas actuales del PTY.
    rows: u16,
}

impl TerminalSession {
    /// Crea una nueva sesión de terminal con un shell.
    ///
    /// Detecta el shell disponible, lanza el PTY con `portable-pty`,
    /// crea un thread lector y un `Term<VoidListener>` como VT emulator.
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let shell = detect_shell();
        tracing::info!(shell = %shell, "lanzando terminal");

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("no se pudo abrir PTY")?;

        // Spawn shell
        let cmd = CommandBuilder::new(&shell);
        let _child = pair
            .slave
            .spawn_command(cmd)
            .context("no se pudo lanzar shell en PTY")?;

        // Drop slave — el master mantiene el PTY vivo
        drop(pair.slave);

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

        // Init alacritty Term
        let dimensions = TermDimensions::new(cols, rows);
        let config = TermConfig::default();
        let term = Term::new(config, &dimensions, VoidListener);
        let vte_processor = VteProcessor::new();

        Ok(Self {
            term,
            vte_processor,
            writer,
            master: pair.master,
            output_rx: rx,
            stop_flag,
            reader_thread: Some(reader_thread),
            cols,
            rows,
        })
    }

    /// Drena output pendiente del PTY y lo alimenta al VT parser.
    ///
    /// Retorna `true` si hubo nuevo output (para invalidar render).
    pub fn poll_output(&mut self) -> bool {
        let mut had_output = false;

        while let Ok(bytes) = self.output_rx.try_recv() {
            had_output = true;
            // Alimentar bytes crudos al VT state machine
            self.vte_processor.advance(&mut self.term, &bytes);
        }

        had_output
    }

    /// Envía bytes al PTY (input del shell).
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer
            .write_all(bytes)
            .context("error al enviar bytes al terminal")?;
        self.writer
            .flush()
            .context("error al flush del writer del terminal")?;
        Ok(())
    }

    /// Redimensiona el PTY y actualiza el modelo de tamaño del Term.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        if self.cols == cols && self.rows == rows {
            return Ok(());
        }
        self.cols = cols;
        self.rows = rows;

        // Resize PTY primero — el shell recibe SIGWINCH
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("error al resize del PTY")?;

        // Luego resize del Term — actualiza la grilla
        let dimensions = TermDimensions::new(cols, rows);
        self.term.resize(dimensions);

        Ok(())
    }

    /// Columnas actuales.
    #[expect(dead_code, reason = "se usará en Batch 3+")]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Filas actuales.
    #[expect(dead_code, reason = "se usará en Batch 3+")]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    // ── Legacy API — mantener compatibilidad durante transición ──

    /// Envía un carácter de input al PTY (legacy).
    pub fn send_key(&mut self, ch: char) -> Result<()> {
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        self.send_bytes(encoded.as_bytes())
    }

    /// Envía Enter al PTY (legacy — `\r\n` en Windows).
    pub fn send_enter(&mut self) -> Result<()> {
        self.send_bytes(b"\r\n")
    }

    /// Envía Ctrl+C al PTY (legacy — byte 0x03).
    pub fn send_ctrl_c(&mut self) -> Result<()> {
        self.send_bytes(&[0x03])
    }

    /// Retorna las líneas visibles para el viewport de height líneas.
    ///
    /// Extrae texto de la grilla de `Term` — compatibilidad con el render
    /// actual que espera `Vec<&str>`. Batch 4 eliminará esto.
    #[expect(
        dead_code,
        reason = "legacy — render ahora usa build_lines(term, rect) desde renderer.rs"
    )]
    pub fn visible_lines(&self, height: usize) -> Vec<String> {
        if height == 0 {
            return Vec::new();
        }

        let grid = self.term.grid();
        let total_lines = grid.screen_lines();
        let num_cols = grid.columns();
        let lines_to_show = height.min(total_lines);

        let mut result = Vec::with_capacity(lines_to_show);

        // Iterar desde la primera línea visible hasta la última
        let start = total_lines.saturating_sub(lines_to_show);
        for line_idx in start..total_lines {
            let mut line_str = String::with_capacity(num_cols);
            let row = &grid[alacritty_terminal::index::Line(line_idx as i32)];
            for col in 0..num_cols {
                let cell = &row[alacritty_terminal::index::Column(col)];
                line_str.push(cell.c);
            }
            // Trim trailing whitespace para la línea
            let trimmed = line_str.trim_end();
            result.push(trimmed.to_string());
        }

        result
    }

    /// Scrollea hacia arriba N líneas (legacy — no-op con alacritty_terminal).
    pub fn scroll_up(&mut self, _lines: usize) {
        // alacritty_terminal maneja scroll internamente via Term::scroll_display
        // Se implementará correctamente en batch 4/5
    }

    /// Scrollea hacia abajo N líneas (legacy — no-op con alacritty_terminal).
    pub fn scroll_down(&mut self, _lines: usize) {
        // Se implementará correctamente en batch 4/5
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Señalizar al thread lector que se detenga
        self.stop_flag.store(true, Ordering::Relaxed);

        // Drop writer primero — envía EOF al shell, lo que hace que el
        // read() bloqueante del thread lector retorne 0 bytes.
        // Esto desbloquea el thread lector limpiamente sin timeout.
        drop(std::mem::replace(
            &mut self.writer,
            Box::new(std::io::sink()),
        ));

        // Ahora sí join — el thread debería terminar rápido porque
        // el write side cerrado causa EOF en el read del reader.
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

// Debug manual porque Box<dyn Write> y Box<dyn MasterPty> no implementan Debug.
impl std::fmt::Debug for TerminalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSession")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
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
                tracing::info!("terminal reader: EOF — proceso terminado");
                break;
            }
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    tracing::debug!("terminal reader: channel cerrado, saliendo");
                    break;
                }
            }
            Err(e) => {
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
        if which_exists("powershell.exe") {
            return "powershell.exe".to_string();
        }
        "cmd.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// Verifica si un ejecutable existe en el PATH.
fn which_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("/?")
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
