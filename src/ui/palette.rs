//! Command Palette: overlay centrado para búsqueda y ejecución de comandos.
//!
//! La palette es un overlay modal que captura todo el input mientras está
//! visible. Filtra comandos del registry según el texto de búsqueda y
//! permite ejecutar el seleccionado con Enter.
//!
//! El filtrado se hace en `update_filter()` — NUNCA en render.
//! El render solo dibuja desde el cache de `filtered`.

use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::command::{CommandEntry, CommandRegistry};
use crate::ui::layout::{self, IdeLayout};
use crate::ui::theme::Theme;

// ─── PaletteState ──────────────────────────────────────────────────────────────

/// Estado de la command palette.
///
/// Mantiene el input de búsqueda, la lista filtrada (como índices al registry),
/// la selección actual y el scroll. El filtrado se hace en `update_filter()`,
/// no en render.
#[derive(Debug)]
pub struct PaletteState {
    /// Si la palette está visible.
    pub visible: bool,
    /// Texto de búsqueda del usuario.
    pub input: String,
    /// Posición del cursor dentro del input.
    pub cursor_pos: usize,
    /// Índices de comandos que matchean el filtro actual.
    /// Se cachean en `update_filter()` para no recalcular en render.
    pub filtered: Vec<usize>,
    /// Índice de la selección dentro de `filtered`.
    pub selected_index: usize,
    /// Offset de scroll para listas largas.
    pub scroll_offset: usize,
}

/// Máximo de items visibles en la lista de la palette.
const MAX_VISIBLE_ITEMS: usize = 15;

impl PaletteState {
    /// Crea un estado inicial (palette cerrada).
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::with_capacity(64),
            cursor_pos: 0,
            filtered: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
        }
    }

    /// Abre la palette: limpia input, muestra todos los comandos.
    pub fn open(&mut self, registry: &CommandRegistry) {
        self.visible = true;
        self.input.clear();
        self.cursor_pos = 0;
        self.selected_index = 0;
        self.scroll_offset = 0;

        // Mostrar todos los comandos inicialmente
        self.filtered = (0..registry.len()).collect();
    }

    /// Cierra la palette y limpia el estado.
    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.cursor_pos = 0;
        self.filtered.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Actualiza la lista filtrada según el input actual.
    ///
    /// Usa `CommandRegistry::search()` para obtener matches ordenados
    /// por relevancia y mapea los resultados a índices del registry.
    pub fn update_filter(&mut self, registry: &CommandRegistry) {
        let matches = registry.search(&self.input);
        let all_commands = registry.all();

        self.filtered.clear();
        self.filtered.reserve(matches.len());

        for matched_cmd in &matches {
            // Encontrar el índice de este comando en el registry
            if let Some(idx) = all_commands
                .iter()
                .position(|c| std::ptr::eq(c, *matched_cmd))
            {
                self.filtered.push(idx);
            }
        }

        // Reset selección si se sale del rango
        if self.selected_index >= self.filtered.len() {
            self.selected_index = 0;
        }
        self.scroll_offset = 0;
    }

    /// Mueve la selección una posición arriba.
    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index > 0 {
                self.selected_index -= 1;
            } else {
                // Wrap al final
                self.selected_index = self.filtered.len() - 1;
            }
            self.ensure_visible();
        }
    }

    /// Mueve la selección una posición abajo.
    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            if self.selected_index + 1 < self.filtered.len() {
                self.selected_index += 1;
            } else {
                // Wrap al inicio
                self.selected_index = 0;
            }
            self.ensure_visible();
        }
    }

    /// Inserta un carácter en el input y re-filtra.
    pub fn insert_char(&mut self, ch: char, registry: &CommandRegistry) {
        self.input.insert(self.cursor_pos, ch);
        self.cursor_pos += ch.len_utf8();
        self.update_filter(registry);
    }

    /// Elimina el carácter antes del cursor y re-filtra.
    pub fn delete_char(&mut self, registry: &CommandRegistry) {
        if self.cursor_pos > 0 {
            // Encontrar el boundary del char anterior
            let prev_boundary = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map(|(idx, _)| idx)
                .unwrap_or(0);
            self.input.drain(prev_boundary..self.cursor_pos);
            self.cursor_pos = prev_boundary;
            self.update_filter(registry);
        }
    }

    /// Retorna el comando actualmente seleccionado.
    pub fn selected_command<'a>(&self, registry: &'a CommandRegistry) -> Option<&'a CommandEntry> {
        let &cmd_idx = self.filtered.get(self.selected_index)?;
        registry.all().get(cmd_idx)
    }

    /// Ajusta el scroll para que la selección sea visible.
    fn ensure_visible(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + MAX_VISIBLE_ITEMS {
            self.scroll_offset = self.selected_index - MAX_VISIBLE_ITEMS + 1;
        }
    }
}

// ─── Render ────────────────────────────────────────────────────────────────────

/// Renderiza la command palette como overlay centrado.
///
/// Overlay centrado: ~60% ancho, max `MAX_VISIBLE_ITEMS` items de alto.
/// Input field arriba con ">" prompt, lista de resultados debajo.
/// Seleccionado con highlight. Cada item: `Category: Label    Keybinding`.
///
/// Precondición: `palette.visible == true`.
/// NO aloca `format!()` dentro del loop de items — pre-computa antes.
pub fn render_palette(
    f: &mut Frame,
    layout: &IdeLayout,
    palette: &PaletteState,
    registry: &CommandRegistry,
    theme: &Theme,
) {
    if !palette.visible {
        return;
    }

    // ── Calcular área del overlay via modal_rect ──
    let visible_items = palette.filtered.len().min(MAX_VISIBLE_ITEMS);
    let modal_height = (visible_items as u16 + 5).max(6); // 5 chrome lines
    let overlay_rect = layout::modal_rect(layout, modal_height);

    // ── Limpiar el área del overlay ──
    f.render_widget(Clear, overlay_rect);

    // ── Bloque exterior con borde accent ──
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Command Palette ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.fg_accent))
        .style(Style::default().bg(theme.bg_secondary));

    let inner = block.inner(overlay_rect);
    f.render_widget(block, overlay_rect);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // ── Layout interno: input (1 línea) + lista (resto) + footer (1) ──
    let inner_layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(1), // input
            Constraint::Fill(1),   // lista de resultados
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let input_area = inner_layout[0];
    let list_area = inner_layout[1];
    let footer_area = inner_layout[2];

    // ── Render input field ──
    let input_line = Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            palette.input.as_str(),
            Style::default().fg(theme.fg_primary),
        ),
        Span::styled(
            "_",
            Style::default()
                .fg(theme.fg_accent)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    let input_paragraph = Paragraph::new(input_line).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(input_paragraph, input_area);

    // ── Render lista de resultados ──
    if list_area.height == 0 {
        return;
    }

    let all_commands = registry.all();
    let visible_count = (list_area.height as usize)
        .min(palette.filtered.len().saturating_sub(palette.scroll_offset));

    // Pre-computar las líneas fuera del render — sin format!() en el loop
    let lines: Vec<Line<'_>> = palette
        .filtered
        .iter()
        .skip(palette.scroll_offset)
        .take(visible_count)
        .enumerate()
        .map(|(i, &cmd_idx)| {
            let cmd = &all_commands[cmd_idx];
            let is_selected = palette.scroll_offset + i == palette.selected_index;
            render_palette_item(cmd, is_selected, list_area.width as usize, theme)
        })
        .collect();

    let list_paragraph = Paragraph::new(lines).style(Style::default().bg(theme.bg_secondary));
    f.render_widget(list_paragraph, list_area);

    // ── Footer con atajos ──
    let footer = Paragraph::new(Line::from(Span::styled(
        " [\u{2191}\u{2193}] Navegar   [Enter] Ejecutar   [Esc] Cerrar",
        Style::default().fg(theme.fg_secondary),
    )))
    .alignment(Alignment::Left)
    .style(Style::default().bg(theme.bg_active));
    f.render_widget(footer, footer_area);
}

/// Renderiza un item de la palette como `Line`.
///
/// Formato: `  Category: Label          Keybinding`
/// El item seleccionado usa `bg_active` como fondo.
/// No usa `format!()` — construye spans directamente.
fn render_palette_item<'a>(
    cmd: &CommandEntry,
    selected: bool,
    max_width: usize,
    theme: &'a Theme,
) -> Line<'a> {
    let bg = if selected {
        theme.bg_active
    } else {
        theme.bg_secondary
    };

    let indicator = if selected { " \u{25b8} " } else { "   " };

    let keybinding_text = cmd.keybinding.unwrap_or("");
    let keybinding_display_len = keybinding_text.len();

    // Calcular espacio disponible para el label
    // indicator(3) + category + ": " + label + padding + keybinding
    let prefix_len = indicator.len() + cmd.category.len() + 2; // ": "
    let padding_needed = max_width
        .saturating_sub(prefix_len)
        .saturating_sub(cmd.label.len())
        .saturating_sub(keybinding_display_len)
        .saturating_sub(1); // margen derecho

    // Construir spans sin allocaciones extras
    let category_style = Style::default().fg(theme.fg_secondary).bg(bg);
    let label_style = if selected {
        Style::default()
            .fg(theme.fg_primary)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_primary).bg(bg)
    };
    let keybinding_style = Style::default().fg(theme.fg_secondary).bg(bg);
    let indicator_style = Style::default()
        .fg(theme.fg_accent)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    // Generar padding como slice de espacios estáticos
    const SPACES: &str =
        "                                                                                ";
    let pad = &SPACES[..padding_needed.min(SPACES.len())];

    Line::from(vec![
        Span::styled(indicator, indicator_style),
        Span::styled(cmd.category, category_style),
        Span::styled(": ", category_style),
        Span::styled(cmd.label, label_style),
        Span::styled(pad, Style::default().bg(bg)),
        Span::styled(keybinding_text, keybinding_style),
    ])
}
