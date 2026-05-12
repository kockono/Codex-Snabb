//! Terminal: multi-pane con VT emulation via alacritty_terminal.
//!
//! Gestiona la terminal integrada: un `PaneTree` recursivo divide el área
//! entre múltiples panes, cada uno con su `TerminalSession` independiente.
//!
//! Arquitectura:
//! - `TerminalSession` encapsula un PTY + thread lector + bounded channel + Term
//! - `TerminalPane` encapsula una sesión + rect
//! - `PaneTree` define el layout recursivo de splits
//! - `TerminalState` orquesta todo — spawn, split, close, focus, poll

pub mod input;
pub mod pane;
pub mod renderer;
pub mod session;
pub mod tree;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use ratatui::layout::Rect;

use self::{
    pane::TerminalPane,
    session::TerminalSession,
    tree::{Orientation, PaneId, PaneTree},
};

/// Estado del subsistema de terminal en `AppState`.
///
/// Soporta múltiples panes de terminal con layout recursivo.
/// Mientras `tree` sea `None`, el terminal está desactivado.
pub struct TerminalState {
    /// Árbol de layout. `None` hasta el primer spawn.
    pub tree: Option<PaneTree>,
    /// Panes activos indexados por ID.
    pub panes: HashMap<PaneId, TerminalPane>,
    /// Pane con foco activo. `None` si no hay panes.
    pub active_pane: Option<PaneId>,
    /// Si el panel de terminal tiene el foco de input.
    pub focused: bool,
    /// Contador para asignar IDs únicos.
    next_id: PaneId,
}

impl TerminalState {
    /// Crea un estado de terminal vacío (sin panes).
    pub fn new() -> Self {
        Self {
            tree: None,
            panes: HashMap::new(),
            active_pane: None,
            focused: false,
            next_id: 1,
        }
    }

    /// Asigna un nuevo PaneId único.
    fn alloc_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Si hay al menos un pane activo.
    pub fn has_session(&self) -> bool {
        !self.panes.is_empty()
    }

    /// Crea un pane inicial si no existe ninguno.
    ///
    /// Idempotente: si ya hay panes, no hace nada.
    /// `working_dir` se ignora — el shell hereda el cwd del proceso.
    pub fn spawn_shell(&mut self, _working_dir: &Path, size: (u16, u16)) -> Result<()> {
        if !self.panes.is_empty() {
            return Ok(());
        }

        let id = self.alloc_id();
        let session = TerminalSession::new(size.0, size.1)?;
        let pane = TerminalPane::new(id, session);
        self.panes.insert(id, pane);
        self.tree = Some(PaneTree::Leaf(id));
        self.active_pane = Some(id);
        tracing::info!("sesión de terminal creada");
        Ok(())
    }

    /// Split del pane activo en la orientación dada.
    pub fn split(&mut self, orientation: Orientation, cols: u16, rows: u16) -> Result<()> {
        let active_id = match self.active_pane {
            Some(id) => id,
            None => return Ok(()),
        };
        let new_id = self.alloc_id();
        let session = TerminalSession::new(cols, rows)?;
        let pane = TerminalPane::new(new_id, session);
        self.panes.insert(new_id, pane);
        if let Some(tree) = &mut self.tree {
            tree.split_leaf(active_id, orientation, new_id);
        }
        self.active_pane = Some(new_id);
        Ok(())
    }

    /// Cierra el pane activo y reclama su espacio.
    pub fn close_active_pane(&mut self) {
        let active_id = match self.active_pane {
            Some(id) => id,
            None => return,
        };
        self.panes.remove(&active_id);
        if let Some(tree) = &mut self.tree {
            tree.remove_leaf(active_id);
        }
        if self.panes.is_empty() {
            self.tree = None;
            self.active_pane = None;
            self.focused = false;
        } else {
            // Foco al primer pane restante
            self.active_pane = self.tree.as_ref().map(|t| t.first_leaf());
        }
    }

    /// Mover foco al siguiente pane en orden depth-first.
    pub fn focus_next(&mut self) {
        if let (Some(tree), Some(active)) = (&self.tree, self.active_pane) {
            if let Some(next) = tree.next_after(active) {
                self.active_pane = Some(next);
            }
        }
    }

    /// Mover foco al pane anterior en orden depth-first (cicla al último si está en el primero).
    pub fn focus_prev(&mut self) {
        if let (Some(tree), Some(active)) = (&self.tree, self.active_pane) {
            let mut ids: Vec<PaneId> = Vec::new();
            tree.collect_ids(&mut ids);
            if let Some(pos) = ids.iter().position(|&id| id == active) {
                let prev = if pos == 0 { ids.len() - 1 } else { pos - 1 };
                self.active_pane = Some(ids[prev]);
            }
        }
    }

    /// Mover foco a un pane específico por ID.
    pub fn focus_pane(&mut self, id: PaneId) {
        if self.panes.contains_key(&id) {
            self.active_pane = Some(id);
        }
    }

    /// Envía bytes al PTY del pane activo.
    pub fn send_bytes_to_active(&mut self, bytes: &[u8]) -> Result<()> {
        if let Some(id) = self.active_pane {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.session.send_bytes(bytes)?;
            }
        }
        Ok(())
    }

    /// Drena output de todos los panes.
    #[expect(
        dead_code,
        reason = "disponible para migrar poll_output — se usará cuando se elimine legacy poll_output"
    )]
    pub fn poll_all_output(&mut self) {
        for pane in self.panes.values_mut() {
            pane.poll_output();
        }
    }

    /// Actualiza rects de todos los panes desde el tree y redimensiona PTYs.
    pub fn update_layout(&mut self, area: Rect) {
        if let Some(tree) = &self.tree {
            let mut rects: Vec<(PaneId, Rect)> = Vec::with_capacity(self.panes.len());
            tree.collect_rects(area, &mut rects);
            for (id, rect) in rects {
                if let Some(pane) = self.panes.get_mut(&id) {
                    let _ = pane.update_rect(rect);
                }
            }
        }
    }

    // ── Legacy compatibility API ──
    // Estos métodos mantienen la interfaz que `src/app/mod.rs` y `src/ui/`
    // esperan durante la transición (Batch 4/5 los eliminará).

    /// Referencia mutable a la sesión del pane activo (legacy compat).
    ///
    /// Los callers existentes acceden a `state.terminal.session` como
    /// `Option<TerminalSession>`. Este getter simula esa interfaz.
    pub fn session_mut(&mut self) -> Option<&mut TerminalSession> {
        let id = self.active_pane?;
        self.panes.get_mut(&id).map(|p| &mut p.session)
    }

    /// Referencia inmutable a la sesión del pane activo (legacy compat).
    #[expect(
        dead_code,
        reason = "legacy compat — render ahora usa &TerminalState directamente"
    )]
    pub fn session_ref(&self) -> Option<&TerminalSession> {
        let id = self.active_pane?;
        self.panes.get(&id).map(|p| &p.session)
    }

    /// Drena output disponible del PTY del pane activo sin bloquear (legacy compat).
    ///
    /// Retorna `true` si hubo nuevo output (para invalidar render).
    pub fn poll_output(&mut self) -> Result<bool> {
        // Poll ALL panes — no solo el activo — para que todos reciban output
        let mut had_output = false;
        for pane in self.panes.values_mut() {
            if pane.session.poll_output() {
                had_output = true;
            }
        }
        Ok(had_output)
    }
}

impl Default for TerminalState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TerminalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalState")
            .field("panes", &self.panes.len())
            .field("active_pane", &self.active_pane)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}
