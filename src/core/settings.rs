//! Estado del overlay de settings con editor de keybindings.
//!
//! Gestiona la UI de configuración de atajos: búsqueda, navegación,
//! edición modal de keybinds. Los cambios se aplican en memoria al
//! `CommandRegistry` — sin persistencia a disco por ahora.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::core::command::CommandRegistry;

// ─── KeybindEntry ──────────────────────────────────────────────────────────────

/// Entrada de un keybinding para display en la tabla de settings.
///
/// Cada entry corresponde a un comando registrado en el `CommandRegistry`.
/// `keybinding` es la representación display del atajo (ej: "Ctrl+S").
#[derive(Debug, Clone)]
pub struct KeybindEntry {
    /// ID del comando (ej: "file.save").
    pub command_id: &'static str,
    /// Label para display (ej: "Save File").
    pub command_label: &'static str,
    /// Categoría (ej: "File", "Edit").
    pub category: &'static str,
    /// Display string del keybinding. Vacío si no tiene.
    pub keybinding: String,
    /// Si fue modificado por el usuario en esta sesión.
    pub is_custom: bool,
}

// ─── SidebarSection ────────────────────────────────────────────────────────────

/// Sección de la sidebar seleccionada en la activity bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    /// Explorador de archivos.
    Explorer,
    /// Panel de Git / source control.
    Git,
    /// Panel de búsqueda global.
    Search,
}

// ─── KeybindingsState ──────────────────────────────────────────────────────────

/// Estado del overlay de keybindings settings.
///
/// Controla visibilidad, búsqueda, selección y edición modal de atajos.
/// Los datos se cargan del `CommandRegistry` al abrir.
#[derive(Debug)]
pub struct KeybindingsState {
    /// Si el overlay está visible.
    pub visible: bool,
    /// Entradas cargadas del registry.
    pub entries: Vec<KeybindEntry>,
    /// Índices filtrados por búsqueda (indexan en `entries`).
    pub filtered: Vec<usize>,
    /// Índice seleccionado dentro de `filtered`.
    pub selected_index: usize,
    /// Offset de scroll para viewport virtual.
    pub scroll_offset: usize,
    /// Input de búsqueda actual.
    pub search_input: String,
    /// Índice en `entries` del keybind que se está editando (modo captura).
    pub editing_index: Option<usize>,
    /// Display string del nuevo keybind mientras se edita.
    pub new_keybind_input: String,
}

impl KeybindingsState {
    /// Crea un estado inicial vacío (invisible).
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            search_input: String::new(),
            editing_index: None,
            new_keybind_input: String::new(),
        }
    }

    /// Abre el overlay cargando todos los comandos del registry.
    ///
    /// Construye la lista de entries con sus keybindings actuales.
    /// Si un comando tiene override custom, se marca con `is_custom`.
    pub fn open(&mut self, registry: &CommandRegistry) {
        self.visible = true;
        self.search_input.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.editing_index = None;
        self.new_keybind_input.clear();

        // Cargar entries del registry
        let commands = registry.all();
        self.entries.clear();
        self.entries.reserve(commands.len());

        for cmd in commands {
            let (display_keybind, is_custom) = if let Some(custom) = registry.custom_keybind(cmd.id)
            {
                (custom.to_string(), true)
            } else if let Some(kb) = cmd.keybinding {
                (kb.to_string(), false)
            } else {
                (String::new(), false)
            };

            self.entries.push(KeybindEntry {
                command_id: cmd.id,
                command_label: cmd.label,
                category: cmd.category,
                keybinding: display_keybind,
                is_custom,
            });
        }

        // Filtrado inicial: mostrar todo
        self.filtered = (0..self.entries.len()).collect();
    }

    /// Cierra el overlay y limpia estado de edición.
    pub fn close(&mut self) {
        self.visible = false;
        self.editing_index = None;
        self.new_keybind_input.clear();
    }

    /// Recalcula el filtrado basado en `search_input`.
    ///
    /// Busca substring case-insensitive en label, category y keybinding.
    pub fn update_filter(&mut self) {
        self.filtered.clear();

        if self.search_input.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let query = self.search_input.to_lowercase();
            for (i, entry) in self.entries.iter().enumerate() {
                let label_lower = entry.command_label.to_lowercase();
                let cat_lower = entry.category.to_lowercase();
                let kb_lower = entry.keybinding.to_lowercase();

                if label_lower.contains(&query)
                    || cat_lower.contains(&query)
                    || kb_lower.contains(&query)
                {
                    self.filtered.push(i);
                }
            }
        }

        // Clampear selección al rango válido
        if self.filtered.is_empty() {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(self.filtered.len() - 1);
        }
        self.scroll_offset = 0;
    }

    /// Mueve la selección hacia arriba.
    pub fn move_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    /// Mueve la selección hacia abajo.
    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            let max = self.filtered.len() - 1;
            self.selected_index = (self.selected_index + 1).min(max);
        }
    }

    /// Inserta un carácter en el campo de búsqueda y recalcula filtro.
    pub fn insert_search_char(&mut self, ch: char) {
        self.search_input.push(ch);
        self.update_filter();
    }

    /// Borra el último carácter del campo de búsqueda y recalcula filtro.
    pub fn delete_search_char(&mut self) {
        self.search_input.pop();
        self.update_filter();
    }

    /// Entra en modo edición para el keybind seleccionado.
    ///
    /// El siguiente KeyEvent (que no sea Esc) se capturará como nuevo keybind.
    pub fn start_editing(&mut self) {
        if let Some(&entry_idx) = self.filtered.get(self.selected_index) {
            self.editing_index = Some(entry_idx);
            self.new_keybind_input.clear();
        }
    }

    /// Cancela la edición del keybind sin aplicar cambios.
    pub fn cancel_editing(&mut self) {
        self.editing_index = None;
        self.new_keybind_input.clear();
    }

    /// Setea el nuevo keybind para la entry en edición.
    ///
    /// Actualiza el display string y marca como custom.
    pub fn set_keybind(&mut self, keybind: &str) {
        if let Some(idx) = self.editing_index
            && let Some(entry) = self.entries.get_mut(idx)
        {
            entry.keybinding = keybind.to_string();
            entry.is_custom = true;
        }
        self.editing_index = None;
        self.new_keybind_input.clear();
    }

    /// Quita el keybind del entry seleccionado.
    pub fn remove_keybind(&mut self) {
        if let Some(&entry_idx) = self.filtered.get(self.selected_index)
            && let Some(entry) = self.entries.get_mut(entry_idx)
        {
            entry.keybinding.clear();
            entry.is_custom = true;
        }
    }

    /// Aplica todos los cambios custom al `CommandRegistry`.
    ///
    /// Recorre las entries marcadas como `is_custom` y actualiza el registry.
    pub fn apply_to_registry(&self, registry: &mut CommandRegistry) {
        for entry in &self.entries {
            if entry.is_custom {
                let kb = if entry.keybinding.is_empty() {
                    None
                } else {
                    Some(entry.keybinding.as_str())
                };
                registry.update_keybinding(entry.command_id, kb);
            }
        }
    }

    /// Ajusta scroll para mantener la selección visible.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + visible_height {
            self.scroll_offset = self.selected_index - visible_height + 1;
        }
    }
}

// ─── format_keybind ────────────────────────────────────────────────────────────

/// Formatea un `KeyEvent` de crossterm como string legible.
///
/// Produce strings como "Ctrl+S", "Alt+Shift+P", "F12", etc.
/// No aloca innecesariamente — construye con capacidad pre-estimada.
pub fn format_keybind(key: &KeyEvent) -> String {
    // Capacidad estimada: "Ctrl+Alt+Shift+Backspace" = 24 chars
    let mut parts: Vec<&str> = Vec::with_capacity(4);

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }

    // Buffer temporal para char/Fn — vive lo suficiente para join
    let char_buf;
    let fn_buf;

    let key_name: &str = match key.code {
        KeyCode::Char(c) => {
            char_buf = c.to_uppercase().to_string();
            &char_buf
        }
        KeyCode::F(n) => {
            fn_buf = format!("F{n}");
            &fn_buf
        }
        KeyCode::Enter => "Enter",
        KeyCode::Backspace => "Backspace",
        KeyCode::Tab => "Tab",
        KeyCode::Esc => "Esc",
        KeyCode::Delete => "Delete",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::Up => "Up",
        KeyCode::Down => "Down",
        KeyCode::Left => "Left",
        KeyCode::Right => "Right",
        KeyCode::Insert => "Insert",
        _ => return String::new(), // Teclas especiales no soportadas
    };

    parts.push(key_name);
    parts.join("+")
}

// ─── parse_keybind ─────────────────────────────────────────────────────────────

/// Parsea un string de keybind ("Ctrl+S") a modifiers y código para matching.
///
/// Retorna `None` si el string está vacío o no se puede parsear.
/// Se usa para hacer match entre un `KeyEvent` y un keybinding guardado.
pub fn parse_keybind(keybind: &str) -> Option<(KeyModifiers, KeyCode)> {
    if keybind.is_empty() {
        return None;
    }

    let parts: Vec<&str> = keybind.split('+').collect();
    let mut modifiers = KeyModifiers::NONE;
    let mut key_code = None;

    for part in &parts {
        match *part {
            "Ctrl" => modifiers |= KeyModifiers::CONTROL,
            "Alt" => modifiers |= KeyModifiers::ALT,
            "Shift" => modifiers |= KeyModifiers::SHIFT,
            "Enter" => key_code = Some(KeyCode::Enter),
            "Backspace" => key_code = Some(KeyCode::Backspace),
            "Tab" => key_code = Some(KeyCode::Tab),
            "Esc" => key_code = Some(KeyCode::Esc),
            "Delete" => key_code = Some(KeyCode::Delete),
            "Home" => key_code = Some(KeyCode::Home),
            "End" => key_code = Some(KeyCode::End),
            "PageUp" => key_code = Some(KeyCode::PageUp),
            "PageDown" => key_code = Some(KeyCode::PageDown),
            "Up" => key_code = Some(KeyCode::Up),
            "Down" => key_code = Some(KeyCode::Down),
            "Left" => key_code = Some(KeyCode::Left),
            "Right" => key_code = Some(KeyCode::Right),
            "Insert" => key_code = Some(KeyCode::Insert),
            s if s.starts_with('F') && s.len() > 1 => {
                if let Ok(n) = s[1..].parse::<u8>() {
                    key_code = Some(KeyCode::F(n));
                }
            }
            s if s.len() == 1 => {
                // Carácter único — convertir a minúscula para matching
                let ch = s.chars().next()?;
                key_code = Some(KeyCode::Char(ch.to_lowercase().next().unwrap_or(ch)));
            }
            // Nombres de tecla especial (Space, Backtick, etc.)
            "Space" => key_code = Some(KeyCode::Char(' ')),
            "`" => key_code = Some(KeyCode::Char('`')),
            _ => {} // Ignorar partes no reconocidas
        }
    }

    key_code.map(|code| (modifiers, code))
}

/// Verifica si un `KeyEvent` matchea un keybinding string.
///
/// Normaliza modifiers y char case para comparación robusta.
pub fn key_matches_keybind(key: &KeyEvent, keybind: &str) -> bool {
    let Some((expected_mods, expected_code)) = parse_keybind(keybind) else {
        return false;
    };

    // Comparar modifiers exactos
    if key.modifiers != expected_mods {
        return false;
    }

    // Comparar key code — normalizar Char case
    match (key.code, expected_code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a.to_lowercase().eq(b.to_lowercase()),
        (a, b) => a == b,
    }
}
