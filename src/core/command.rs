//! Registry central de comandos del IDE.
//!
//! Cada comando tiene un ID único, label, categoría, keybinding opcional
//! y la acción que ejecuta. El registry permite búsqueda fuzzy simple
//! (substring case-insensitive) y lookup por ID.
//!
//! El catálogo vive en memoria y nunca ejecuta IO al filtrar.
//! La palette solo consulta este catálogo pre-registrado.

use crate::core::Action;

// ─── CommandEntry ──────────────────────────────────────────────────────────────

/// Entrada de un comando registrado en el sistema.
///
/// Todos los campos son `&'static str` — sin allocaciones de heap.
/// La `action` se clona al ejecutar el comando (Action implementa Clone).
#[derive(Debug, Clone)]
pub struct CommandEntry {
    /// Identificador único, snake_case: "file.save", "view.toggle_sidebar".
    pub id: &'static str,
    /// Label para display: "Save File", "Toggle Sidebar".
    pub label: &'static str,
    /// Categoría de agrupación: "File", "View", "Edit", "Navigate".
    pub category: &'static str,
    /// Keybinding para display: "Ctrl+S", "Ctrl+B". None si no tiene.
    pub keybinding: Option<&'static str>,
    /// Acción que ejecuta este comando.
    pub action: Action,
}

// ─── CommandRegistry ───────────────────────────────────────────────────────────

/// Registry central de todos los comandos del IDE.
///
/// Almacena comandos en un Vec — la cantidad es fija y pequeña (~20-30),
/// no justifica un HashMap. La búsqueda lineal es más eficiente para
/// este tamaño que el overhead de hashing.
#[derive(Debug)]
pub struct CommandRegistry {
    commands: Vec<CommandEntry>,
}

impl CommandRegistry {
    /// Crea un registry vacío.
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Registra un comando en el registry.
    pub fn register(&mut self, entry: CommandEntry) {
        self.commands.push(entry);
    }

    /// Registra todos los comandos default del sistema.
    ///
    /// Incluye comandos de File, Edit, View, Navigate, Terminal, Git,
    /// Explorer y App. Cada uno con su keybinding correspondiente.
    pub fn register_defaults(&mut self) {
        self.commands.clear();
        // Pre-alocar capacidad conocida — sabemos exactamente cuántos hay
        self.commands.reserve(23);

        // ── File ──
        self.register(CommandEntry {
            id: "file.save",
            label: "Save File",
            category: "File",
            keybinding: Some("Ctrl+S"),
            action: Action::SaveFile,
        });
        self.register(CommandEntry {
            id: "file.close_buffer",
            label: "Close Buffer",
            category: "File",
            keybinding: None,
            action: Action::CloseBuffer,
        });
        self.register(CommandEntry {
            id: "file.open_quick_open",
            label: "Quick Open File",
            category: "File",
            keybinding: Some("Ctrl+P"),
            action: Action::OpenQuickOpen,
        });

        // ── Edit ──
        self.register(CommandEntry {
            id: "edit.undo",
            label: "Undo",
            category: "Edit",
            keybinding: Some("Ctrl+Z"),
            action: Action::Undo,
        });
        self.register(CommandEntry {
            id: "edit.redo",
            label: "Redo",
            category: "Edit",
            keybinding: Some("Ctrl+Y"),
            action: Action::Redo,
        });

        // ── View ──
        self.register(CommandEntry {
            id: "view.toggle_sidebar",
            label: "Toggle Sidebar",
            category: "View",
            keybinding: Some("Ctrl+B"),
            action: Action::ToggleSidebar,
        });
        self.register(CommandEntry {
            id: "view.toggle_bottom_panel",
            label: "Toggle Bottom Panel",
            category: "View",
            keybinding: Some("Ctrl+J"),
            action: Action::ToggleBottomPanel,
        });
        self.register(CommandEntry {
            id: "view.focus_next",
            label: "Focus Next Panel",
            category: "View",
            keybinding: Some("Tab"),
            action: Action::FocusNext,
        });
        self.register(CommandEntry {
            id: "view.focus_prev",
            label: "Focus Previous Panel",
            category: "View",
            keybinding: Some("Shift+Tab"),
            action: Action::FocusPrev,
        });

        // ── Navigate ──
        self.register(CommandEntry {
            id: "navigate.open_command_palette",
            label: "Command Palette",
            category: "Navigate",
            keybinding: Some("Ctrl+Shift+P"),
            action: Action::OpenCommandPalette,
        });
        self.register(CommandEntry {
            id: "navigate.open_global_search",
            label: "Global Search",
            category: "Navigate",
            keybinding: None,
            action: Action::OpenGlobalSearch,
        });

        // ── Terminal ──
        self.register(CommandEntry {
            id: "terminal.toggle",
            label: "Toggle Terminal",
            category: "Terminal",
            keybinding: None,
            action: Action::ToggleTerminal,
        });

        // ── Git ──
        self.register(CommandEntry {
            id: "git.open_panel",
            label: "Open Git Panel",
            category: "Git",
            keybinding: None,
            action: Action::OpenGitPanel,
        });

        // ── LSP ──
        self.register(CommandEntry {
            id: "lsp.start",
            label: "LSP: Start Server",
            category: "LSP",
            keybinding: None,
            action: Action::LspStart,
        });
        self.register(CommandEntry {
            id: "lsp.stop",
            label: "LSP: Stop Server",
            category: "LSP",
            keybinding: None,
            action: Action::LspStop,
        });
        self.register(CommandEntry {
            id: "lsp.hover",
            label: "LSP: Show Hover Info",
            category: "LSP",
            keybinding: Some("Ctrl+K"),
            action: Action::LspHover,
        });
        self.register(CommandEntry {
            id: "lsp.goto_definition",
            label: "LSP: Go to Definition",
            category: "LSP",
            keybinding: Some("F12"),
            action: Action::LspGotoDefinition,
        });
        self.register(CommandEntry {
            id: "lsp.completion",
            label: "LSP: Trigger Completion",
            category: "LSP",
            keybinding: Some("Ctrl+Space"),
            action: Action::LspCompletion,
        });

        // ── Explorer ──
        self.register(CommandEntry {
            id: "explorer.refresh",
            label: "Refresh Explorer",
            category: "Explorer",
            keybinding: Some("R"),
            action: Action::ExplorerRefresh,
        });

        // ── App ──
        self.register(CommandEntry {
            id: "app.quit",
            label: "Quit Application",
            category: "App",
            keybinding: Some("Esc"),
            action: Action::Quit,
        });
    }

    /// Búsqueda fuzzy simple: substring case-insensitive.
    ///
    /// Retorna matches ordenados por relevancia:
    /// 1. Match exacto en label (case-insensitive)
    /// 2. Prefix match en label
    /// 3. Contains match en label, id o category
    ///
    /// Si el query está vacío, retorna todos los comandos.
    pub fn search(&self, query: &str) -> Vec<&CommandEntry> {
        if query.is_empty() {
            return self.commands.iter().collect();
        }

        let query_lower = query.to_lowercase();

        let mut exact: Vec<&CommandEntry> = Vec::new();
        let mut prefix: Vec<&CommandEntry> = Vec::new();
        let mut contains: Vec<&CommandEntry> = Vec::new();

        for cmd in &self.commands {
            let label_lower = cmd.label.to_lowercase();
            let id_lower = cmd.id.to_lowercase();
            let cat_lower = cmd.category.to_lowercase();

            if label_lower == query_lower {
                exact.push(cmd);
            } else if label_lower.starts_with(&query_lower) {
                prefix.push(cmd);
            } else if label_lower.contains(&query_lower)
                || id_lower.contains(&query_lower)
                || cat_lower.contains(&query_lower)
            {
                contains.push(cmd);
            }
        }

        // Capacidad conocida — evitar re-allocaciones
        let total = exact.len() + prefix.len() + contains.len();
        let mut results = Vec::with_capacity(total);
        results.extend(exact);
        results.extend(prefix);
        results.extend(contains);
        results
    }

    /// Retorna todos los comandos registrados.
    pub fn all(&self) -> &[CommandEntry] {
        &self.commands
    }

    /// Busca un comando por su ID.
    #[expect(
        dead_code,
        reason = "API pública — se usará para lookup directo de comandos"
    )]
    pub fn find_by_id(&self, id: &str) -> Option<&CommandEntry> {
        self.commands.iter().find(|cmd| cmd.id == id)
    }

    /// Cantidad de comandos registrados.
    pub fn len(&self) -> usize {
        self.commands.len()
    }
}
