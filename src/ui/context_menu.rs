//! Context menu: menú contextual que aparece al hacer right-click en el explorer.
//!
//! Diseño visual:
//! ```text
//! ┌─────────────────────────┐
//! │ Rename                  │
//! │ Delete                  │
//! │─────────────────────────│
//! │ Copy                    │
//! │ Copy Path               │
//! │ Copy Relative Path      │
//! │─────────────────────────│
//! │ Reveal in File Explorer │
//! └─────────────────────────┘
//! ```
//!
//! Aparece en la posición del click (clampeada a los bounds de terminal).
//! Navegación: flechas + Enter confirma, Esc cierra.
//! Click fuera → cierra.

use std::path::PathBuf;

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::ui::Theme;

// ─── Types ─────────────────────────────────────────────────────────────────────

/// Items disponibles en el context menu del explorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    Rename,
    Delete,
    Copy,
    CopyPath,
    CopyRelativePath,
    RevealInExplorer,
}

/// Estado del context menu flotante.
///
/// Aparece en la posición del right-click y desaparece en Esc o click fuera.
/// Los items son fijos — el orden y separadores están hardcodeados.
#[derive(Debug)]
pub struct ContextMenuState {
    /// Si el menú está visible.
    pub visible: bool,
    /// Item actualmente resaltado (0-indexed sobre la lista de items, NO filas —
    /// las filas de separador no cuentan).
    pub selected: usize,
    /// Columna de terminal donde aparece el menú (esquina superior izquierda).
    pub x: u16,
    /// Fila de terminal donde aparece el menú (esquina superior izquierda).
    pub y: u16,
    /// Path del archivo/dir sobre el que se hizo right-click.
    pub target_path: Option<PathBuf>,
}

/// Lista ordenada de items del context menu.
///
/// Los separadores son manejados por el renderer — no son items seleccionables.
static MENU_ITEMS: &[ContextMenuItem] = &[
    ContextMenuItem::Rename,
    ContextMenuItem::Delete,
    ContextMenuItem::Copy,
    ContextMenuItem::CopyPath,
    ContextMenuItem::CopyRelativePath,
    ContextMenuItem::RevealInExplorer,
];

impl ContextMenuState {
    /// Crea un nuevo context menu en estado cerrado.
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: 0,
            x: 0,
            y: 0,
            target_path: None,
        }
    }

    /// Abre el menú en la posición dada para el path indicado.
    pub fn open(&mut self, x: u16, y: u16, path: PathBuf) {
        self.visible = true;
        self.selected = 0;
        self.x = x;
        self.y = y;
        self.target_path = Some(path);
    }

    /// Cierra el menú y limpia el estado.
    pub fn close(&mut self) {
        self.visible = false;
        self.selected = 0;
        self.target_path = None;
    }

    /// Mueve la selección hacia arriba (wrapping).
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = MENU_ITEMS.len().saturating_sub(1);
        }
    }

    /// Mueve la selección hacia abajo (wrapping).
    pub fn move_down(&mut self) {
        self.selected = (self.selected + 1) % MENU_ITEMS.len().max(1);
    }

    /// Retorna el item actualmente seleccionado, si el menú está visible.
    pub fn selected_item(&self) -> Option<ContextMenuItem> {
        if self.visible {
            MENU_ITEMS.get(self.selected).copied()
        } else {
            None
        }
    }
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Render ────────────────────────────────────────────────────────────────────

/// Ancho fijo del menú (chars): "Reveal in File Explorer" = 23 + 2 padding + 2 borders = 27.
/// Redondeamos a 27 para que quepa el item más largo con padding.
const MENU_WIDTH: u16 = 27;

/// Separadores en el menú: índices de FILA (no de item) donde aparece la línea horizontal.
/// El layout de filas (dentro del bloque, excluyendo borders):
///   0 → Rename
///   1 → Delete
///   2 → ── separador ──
///   3 → Copy
///   4 → Copy Path
///   5 → Copy Relative Path
///   6 → ── separador ──
///   7 → Reveal in File Explorer
const MENU_INNER_HEIGHT: u16 = 8; // 6 items + 2 separadores

/// Mapeo de fila interna → item index (None = separador).
///
/// Pre-computado para evitar cualquier cómputo en render.
/// Índices: fila 0..7
static ROW_TO_ITEM: &[Option<usize>] = &[
    Some(0), // Rename
    Some(1), // Delete
    None,    // separador
    Some(2), // Copy
    Some(3), // Copy Path
    Some(4), // Copy Relative Path
    None,    // separador
    Some(5), // Reveal in File Explorer
];

/// Labels de items — orden debe coincidir con `MENU_ITEMS` y `ROW_TO_ITEM`.
static ITEM_LABELS: &[&str] = &[
    "Rename",
    "Delete",
    "Copy",
    "Copy Path",
    "Copy Relative Path",
    "Reveal in File Explorer",
];

/// Renderiza el context menu sobre cualquier otra cosa en la pantalla.
///
/// `frame_area` se usa para clampear la posición y evitar overflow de terminal.
/// NO usa `format!()` ni aloca dentro del render.
pub fn render_context_menu(
    f: &mut Frame,
    frame_area: Rect,
    state: &ContextMenuState,
    theme: &Theme,
) {
    if !state.visible {
        return;
    }

    // Altura total = MENU_INNER_HEIGHT + 2 borders
    let total_height = MENU_INNER_HEIGHT + 2;
    let total_width = MENU_WIDTH;

    // Clampear posición para que el menú no salga de los bounds del terminal
    let clamped_x = if state.x + total_width > frame_area.width {
        frame_area.width.saturating_sub(total_width)
    } else {
        state.x
    };
    let clamped_y = if state.y + total_height > frame_area.height {
        frame_area.height.saturating_sub(total_height)
    } else {
        state.y
    };

    let menu_rect = Rect::new(clamped_x, clamped_y, total_width, total_height);

    // Limpiar el área del menú (borrar lo que hay debajo)
    f.render_widget(Clear, menu_rect);

    // Borde del menú
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused))
        .style(Style::default().bg(theme.bg_secondary));
    let inner = block.inner(menu_rect);
    f.render_widget(block, menu_rect);

    // Renderizar cada fila del inner area
    for (row_idx, item_opt) in ROW_TO_ITEM.iter().enumerate() {
        let row_y = inner.y + row_idx as u16;
        if row_y >= inner.y + inner.height {
            break;
        }
        let row_rect = Rect::new(inner.x, row_y, inner.width, 1);

        match item_opt {
            None => {
                // Separador: línea horizontal con border_unfocused color
                // Pre-computado: usamos "─" repetido para llenar el ancho
                // NO usamos format!() — construimos el separador con repeat en &'static
                // Para evitar allocación, renderizamos con estilo directamente.
                // La línea de separador tiene inner.width chars de "─".
                // Como inner.width es variable, necesitamos alocar — pero es fuera
                // del hot path (solo 2 separadores, solo cuando visible).
                // CLONE: alocación aceptable — solo cuando el menú está visible (~2/frame)
                let sep_line = "─".repeat(inner.width as usize);
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        sep_line,
                        Style::default().fg(theme.border_unfocused),
                    ))),
                    row_rect,
                );
            }
            Some(item_idx) => {
                let label = ITEM_LABELS[*item_idx];
                let is_selected = state.selected == *item_idx;

                let (bg, fg) = if is_selected {
                    (theme.bg_active, theme.fg_accent)
                } else {
                    (theme.bg_secondary, theme.fg_primary)
                };

                // Padding: " " al inicio del label
                let line = Line::from(vec![
                    Span::styled(" ", Style::default().bg(bg).fg(fg)),
                    Span::styled(label, Style::default().bg(bg).fg(fg)),
                ]);
                f.render_widget(
                    Paragraph::new(line).style(Style::default().bg(bg)),
                    row_rect,
                );
            }
        }
    }
}
