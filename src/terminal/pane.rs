//! Pane: encapsula una sesión de terminal independiente con su rect.
//!
//! Cada `TerminalPane` contiene un `TerminalSession` y un `Rect` que
//! define su área en el layout. El resize del PTY se dispara cuando
//! el rect cambia de tamaño.

use ratatui::layout::Rect;

use crate::terminal::{session::TerminalSession, tree::PaneId};

/// Un pane individual de terminal con sesión y rect propios.
pub struct TerminalPane {
    /// Identificador único del pane.
    pub id: PaneId,
    /// Sesión de terminal (PTY + VT emulator).
    pub session: TerminalSession,
    /// Último rect calculado por el layout.
    pub last_rect: Rect,
}

impl TerminalPane {
    /// Crea un nuevo pane con una sesión de terminal.
    pub fn new(id: PaneId, session: TerminalSession) -> Self {
        Self {
            id,
            session,
            last_rect: Rect::default(),
        }
    }

    /// Actualiza el rect del pane y redimensiona el PTY si cambió el tamaño.
    pub fn update_rect(&mut self, rect: Rect) -> anyhow::Result<()> {
        if self.last_rect.width != rect.width || self.last_rect.height != rect.height {
            self.last_rect = rect;
            // Restar borde del rect para obtener tamaño interior
            let inner_h = rect.height.saturating_sub(2);
            let inner_w = rect.width.saturating_sub(2);
            if inner_w > 0 && inner_h > 0 {
                self.session.resize(inner_w, inner_h)?;
            }
        } else {
            self.last_rect = rect;
        }
        Ok(())
    }

    /// Drena output pendiente del PTY.
    pub(crate) fn poll_output(&mut self) {
        self.session.poll_output();
    }
}

impl std::fmt::Debug for TerminalPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalPane")
            .field("id", &self.id)
            .field("last_rect", &self.last_rect)
            .finish_non_exhaustive()
    }
}
