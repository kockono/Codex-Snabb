//! Quit modal: estado del diálogo de confirmación al cerrar la app con buffers dirty.
//!
//! Se abre vía `Action::QuitRequested` (Ctrl+Q) cuando hay al menos un buffer dirty.
//! Tres botones: `[Save All]`, `[Don't Save]`, `[Cancel]`. Si hay buffers untitled
//! dirty, "Save All" arranca un sub-flujo que abre el modal Save As para cada uno
//! en orden, y al terminar todos sale de la aplicación.
//!
//! Sin allocaciones en el hot path: `pending_untitled` se aloca una sola vez
//! cuando el usuario confirma "Save All" con buffers untitled, y se libera al cerrar.

/// Estado del modal de quit con sub-flujo de Save All para buffers untitled.
///
/// Invariantes:
/// - `focused_button` está siempre en `0..=2` (Save All / Don't Save / Cancel).
/// - `current_untitled` es `Some(idx)` solo durante el sub-flujo de Save As, y
///   `idx < pending_untitled.len()`.
/// - `in_save_as_flow()` es `true` solo si `visible && current_untitled.is_some()`.
#[derive(Debug, Default)]
pub struct QuitModalState {
    /// Si el modal está visible.
    pub visible: bool,
    /// Botón con foco: 0 = Save All, 1 = Don't Save, 2 = Cancel.
    pub focused_button: usize,
    /// Índices de tabs con buffers untitled dirty pendientes de Save As.
    /// Se popula cuando el usuario confirma "Save All".
    pub pending_untitled: Vec<usize>,
    /// Índice actual dentro de `pending_untitled` (cursor del sub-flujo).
    /// `None` cuando no hay sub-flujo activo.
    pub current_untitled: Option<usize>,
}

impl QuitModalState {
    /// Crea un estado inicial (invisible, foco en "Save All", sin sub-flujo).
    pub fn new() -> Self {
        Self::default()
    }

    /// Muestra el modal con el foco por defecto en "Save All".
    pub fn show(&mut self) {
        self.visible = true;
        self.focused_button = 0;
    }

    /// Oculta el modal y limpia el sub-flujo de Save As.
    pub fn hide(&mut self) {
        self.visible = false;
        self.focused_button = 0;
        self.pending_untitled.clear();
        self.current_untitled = None;
    }

    /// Mueve el foco al siguiente botón con wrap (0 → 1 → 2 → 0).
    pub fn cycle_button_next(&mut self) {
        self.focused_button = (self.focused_button + 1) % 3;
    }

    /// Mueve el foco al botón anterior con wrap (0 → 2 → 1 → 0).
    pub fn cycle_button_prev(&mut self) {
        self.focused_button = (self.focused_button + 2) % 3;
    }

    /// `true` si el modal está conduciendo un sub-flujo de Save As para untitled.
    /// El reducer de `SaveAsConfirm` / `SaveAsCancel` consulta este flag para
    /// decidir si tiene que continuar el flujo de quit o no.
    pub fn in_save_as_flow(&self) -> bool {
        self.visible && self.current_untitled.is_some()
    }

    /// Avanza al siguiente buffer untitled del sub-flujo.
    ///
    /// Retorna:
    /// - `Some(tab_idx)` si hay un siguiente buffer untitled — el caller debe
    ///   activar esa tab y abrir el modal Save As.
    /// - `None` si ya no quedan más untitled — el caller debe salir de la app.
    pub fn advance_untitled(&mut self) -> Option<usize> {
        let next_idx = self.current_untitled.map_or(0, |i| i + 1);
        if next_idx < self.pending_untitled.len() {
            self.current_untitled = Some(next_idx);
            Some(self.pending_untitled[next_idx])
        } else {
            self.current_untitled = None;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_hidden_with_focus_zero() {
        let s = QuitModalState::new();
        assert!(!s.visible);
        assert_eq!(s.focused_button, 0);
        assert!(s.pending_untitled.is_empty());
        assert!(s.current_untitled.is_none());
    }

    #[test]
    fn show_sets_visible_and_resets_focus() {
        let mut s = QuitModalState::new();
        s.focused_button = 2; // estado previo
        s.show();
        assert!(s.visible);
        assert_eq!(s.focused_button, 0);
    }

    #[test]
    fn hide_clears_visibility_and_subflow() {
        let mut s = QuitModalState::new();
        s.show();
        s.pending_untitled = vec![1, 3, 5];
        s.current_untitled = Some(1);
        s.focused_button = 2;
        s.hide();
        assert!(!s.visible);
        assert_eq!(s.focused_button, 0);
        assert!(s.pending_untitled.is_empty());
        assert!(s.current_untitled.is_none());
    }

    #[test]
    fn cycle_next_wraps_after_last() {
        let mut s = QuitModalState::new();
        s.cycle_button_next(); // 0 → 1
        assert_eq!(s.focused_button, 1);
        s.cycle_button_next(); // 1 → 2
        assert_eq!(s.focused_button, 2);
        s.cycle_button_next(); // 2 → 0 (wrap)
        assert_eq!(s.focused_button, 0);
    }

    #[test]
    fn cycle_prev_wraps_before_first() {
        let mut s = QuitModalState::new();
        s.cycle_button_prev(); // 0 → 2 (wrap)
        assert_eq!(s.focused_button, 2);
        s.cycle_button_prev(); // 2 → 1
        assert_eq!(s.focused_button, 1);
        s.cycle_button_prev(); // 1 → 0
        assert_eq!(s.focused_button, 0);
    }

    #[test]
    fn in_save_as_flow_is_false_when_hidden() {
        let mut s = QuitModalState::new();
        s.current_untitled = Some(0); // estado inconsistente
        assert!(!s.in_save_as_flow(), "must require visible=true");
    }

    #[test]
    fn in_save_as_flow_is_false_without_current_untitled() {
        let mut s = QuitModalState::new();
        s.show();
        assert!(!s.in_save_as_flow());
    }

    #[test]
    fn in_save_as_flow_is_true_when_visible_and_in_subflow() {
        let mut s = QuitModalState::new();
        s.show();
        s.pending_untitled = vec![2];
        s.current_untitled = Some(0);
        assert!(s.in_save_as_flow());
    }

    #[test]
    fn advance_untitled_returns_first_when_none_active() {
        let mut s = QuitModalState::new();
        s.show();
        s.pending_untitled = vec![4, 7, 9];
        let first = s.advance_untitled();
        assert_eq!(first, Some(4));
        assert_eq!(s.current_untitled, Some(0));
    }

    #[test]
    fn advance_untitled_walks_through_all() {
        let mut s = QuitModalState::new();
        s.show();
        s.pending_untitled = vec![10, 20, 30];
        assert_eq!(s.advance_untitled(), Some(10));
        assert_eq!(s.advance_untitled(), Some(20));
        assert_eq!(s.advance_untitled(), Some(30));
        // Ya no quedan más → None y current_untitled vuelve a None
        assert_eq!(s.advance_untitled(), None);
        assert_eq!(s.current_untitled, None);
    }

    #[test]
    fn advance_untitled_with_empty_list_returns_none() {
        let mut s = QuitModalState::new();
        s.show();
        assert_eq!(s.advance_untitled(), None);
        assert_eq!(s.current_untitled, None);
    }
}
