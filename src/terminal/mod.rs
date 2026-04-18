//! Terminal: sesiones, IO, scrollback limitado.
//!
//! Gestiona la terminal integrada: proceso shell en worker dedicado,
//! PTY detrás de un boundary claro, scrollback acotado por líneas/bytes,
//! y render solo del viewport visible. MVP: una sesión estable.
//!
//! Arquitectura:
//! - `TerminalSession` encapsula un PTY + thread lector + bounded channel
//! - `TerminalState` es el sub-estado en `AppState` — gestiona la sesión
//! - El event loop llama `poll_output()` cada ciclo (non-blocking)

pub mod session;

use std::path::Path;

use anyhow::Result;
use session::TerminalSession;

/// Estado del subsistema de terminal en `AppState`.
///
/// Contiene una sesión opcional (lazy spawn) y tracking de foco.
/// MVP: una sola sesión — múltiples sesiones es post-MVP.
#[derive(Debug)]
pub struct TerminalState {
    /// Sesión activa de terminal. `None` hasta que el usuario la abra.
    pub session: Option<TerminalSession>,
    /// Si el terminal tiene el foco de input.
    #[expect(
        dead_code,
        reason = "se usará para tracking de foco interno del terminal"
    )]
    pub focused: bool,
}

impl TerminalState {
    /// Crea un estado de terminal vacío (sin sesión).
    pub fn new() -> Self {
        Self {
            session: None,
            focused: false,
        }
    }

    /// Crea una nueva sesión de shell si no existe una.
    ///
    /// Si ya hay una sesión activa, no hace nada (idempotente).
    /// El `size` es (cols, rows) del área del bottom panel.
    pub fn spawn_shell(&mut self, working_dir: &Path, size: (u16, u16)) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }

        let session = TerminalSession::spawn(working_dir, size)?;
        self.session = Some(session);
        tracing::info!("sesión de terminal creada");
        Ok(())
    }

    /// Si hay una sesión activa.
    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    /// Drena output disponible del PTY sin bloquear.
    ///
    /// Retorna `true` si hubo nuevo output (para invalidar render).
    /// Si no hay sesión, retorna `false`.
    pub fn poll_output(&mut self) -> Result<bool> {
        if let Some(ref mut session) = self.session {
            session.poll_output()
        } else {
            Ok(false)
        }
    }
}
