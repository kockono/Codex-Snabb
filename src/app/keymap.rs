//! Keymap: mapeo de eventos de crossterm a acciones del sistema.
//!
//! El keymap es CONTEXT-AWARE — las mismas teclas producen acciones
//! distintas según el panel enfocado y los overlays activos.

use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

use crate::core::command::CommandRegistry;
use crate::core::{Action, Direction, PanelId};
use crate::git::GitState;

/// Mapea un evento de crossterm a una acción del sistema.
///
/// El keymap es CONTEXT-AWARE — las mismas teclas producen acciones
/// distintas según el panel enfocado y los overlays activos:
///
/// - **Quick Open abierto**: captura TODO el input
/// - **Palette abierta**: captura TODO el input
/// - **Search panel activo**: captura input cuando sidebar tiene foco
/// - **Git panel activo**: captura input cuando sidebar tiene foco
/// - **Global**: Ctrl+atajos, Esc, Tab (siempre activos cuando overlays cerrados)
/// - **Editor**: flechas mueven cursor, chars insertan texto
/// - **Explorer**: flechas navegan el árbol, Enter abre/expande
#[expect(
    clippy::too_many_arguments,
    reason = "keymap requiere estado de cada overlay — refactorizar a struct agregaría indirección sin beneficio"
)]
pub(super) fn keymap(
    event: &crossterm::event::Event,
    focused_panel: PanelId,
    palette_visible: bool,
    quick_open_visible: bool,
    go_to_line_visible: bool,
    branch_picker_visible: bool,
    search_visible: bool,
    git_state: &GitState,
    lsp_completion_visible: bool,
    settings_visible: bool,
    settings_editing: bool,
    commands: &CommandRegistry,
) -> Action {
    // ── Eventos de mouse ── se procesan ANTES del match de teclado
    if let CrosstermEvent::Mouse(mouse) = event {
        return match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => Action::MouseClick {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::ScrollUp => Action::MouseScrollUp {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::ScrollDown => Action::MouseScrollDown {
                col: mouse.column,
                row: mouse.row,
            },
            MouseEventKind::Drag(MouseButton::Left) => Action::MouseDrag {
                col: mouse.column,
                row: mouse.row,
            },
            _ => Action::Noop,
        };
    }

    let CrosstermEvent::Key(key) = event else {
        return Action::Noop;
    };
    if key.kind != KeyEventKind::Press {
        return Action::Noop;
    }

    // ── Settings overlay visible: prioridad máxima ──
    if settings_visible {
        if settings_editing {
            // Modo edición: capturar la tecla como nuevo keybind
            return match key.code {
                KeyCode::Esc => Action::SettingsCancelEdit,
                _ => Action::SettingsCaptureKey(*key),
            };
        }
        // Modo normal del settings overlay
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::SettingsClose,
            (KeyCode::Up, KeyModifiers::NONE) => Action::SettingsUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SettingsDown,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::SettingsStartEdit,
            (KeyCode::Delete, _) => Action::SettingsRemoveKeybind,
            (KeyCode::Backspace, _) => Action::SettingsSearchDelete,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::SettingsSearchInsert(ch)
            }
            _ => Action::Noop,
        };
    }

    // ── Go to Line visible: captura dígitos, Enter, Esc ──
    if go_to_line_visible {
        return match key.code {
            KeyCode::Esc => Action::GoToLineClose,
            KeyCode::Enter => Action::GoToLineConfirm,
            KeyCode::Backspace => Action::GoToLineDeleteChar,
            KeyCode::Char(ch) if ch.is_ascii_digit() => Action::GoToLineInsertChar(ch),
            _ => Action::Noop,
        };
    }

    // ── LSP Completion visible: captura navegación, Enter, Esc ──
    if lsp_completion_visible && focused_panel == PanelId::Editor {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => return Action::LspCompletionCancel,
            (KeyCode::Enter, KeyModifiers::NONE) => return Action::LspCompletionConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => return Action::LspCompletionUp,
            (KeyCode::Down, KeyModifiers::NONE) => return Action::LspCompletionDown,
            // Tab = confirmar (estilo VS Code)
            (KeyCode::Tab, KeyModifiers::NONE) => return Action::LspCompletionConfirm,
            // Otros caracteres: cerrar completions y dejar pasar la acción normal
            _ => {
                // No capturar — caer al flujo normal para que chars se inserten
            }
        }
    }

    // ── Branch Picker abierto: captura TODO el input ──
    if branch_picker_visible {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::BranchPickerClose,
            (KeyCode::Enter, _) => Action::BranchPickerConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => Action::BranchPickerUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::BranchPickerDown,
            // Ctrl+P / Ctrl+N para vim-style navigation
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::BranchPickerUp,
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::BranchPickerDown,
            (KeyCode::Backspace, _) => Action::BranchPickerDeleteChar,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::BranchPickerInsertChar(ch)
            }
            // Cualquier otra tecla NO se propaga
            _ => Action::Noop,
        };
    }

    // ── Quick Open abierto: captura TODO el input ──
    if quick_open_visible {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::QuickOpenClose,
            (KeyCode::Enter, _) => Action::QuickOpenConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => Action::QuickOpenUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::QuickOpenDown,
            // Ctrl+P / Ctrl+N para vim-style navigation
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::QuickOpenUp,
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::QuickOpenDown,
            (KeyCode::Backspace, _) => Action::QuickOpenDeleteChar,
            // ':' en Quick Open → switch a Go to Line mode
            (KeyCode::Char(':'), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::OpenGoToLine,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::QuickOpenInsertChar(ch)
            }
            // Cualquier otra tecla NO se propaga
            _ => Action::Noop,
        };
    }

    // ── Palette abierta: captura TODO el input ──
    if palette_visible {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::PaletteClose,
            (KeyCode::Enter, _) => Action::PaletteConfirm,
            (KeyCode::Up, KeyModifiers::NONE) => Action::PaletteUp,
            (KeyCode::Down, KeyModifiers::NONE) => Action::PaletteDown,
            // Ctrl+P / Ctrl+N para vim-style navigation
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => Action::PaletteUp,
            (KeyCode::Char('n'), KeyModifiers::CONTROL) => Action::PaletteDown,
            (KeyCode::Backspace, _) => Action::PaletteDeleteChar,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::PaletteInsertChar(ch)
            }
            // Cualquier otra tecla NO se propaga
            _ => Action::Noop,
        };
    }

    // ── Search panel activo: captura input cuando foco en Search ──
    if search_visible && focused_panel == PanelId::Search {
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::SearchClose,
            // Enter: si foco en campo de input → ejecutar búsqueda;
            //        si hay resultados → toggle fold / abrir match
            (KeyCode::Enter, KeyModifiers::NONE) => Action::SearchSelectAndOpen,
            (KeyCode::Tab, KeyModifiers::NONE) => Action::SearchNextField,
            (KeyCode::BackTab, KeyModifiers::SHIFT) => Action::SearchPrevField,
            // Up/Down navegan por la lista aplanada (headers + matches)
            (KeyCode::Up, KeyModifiers::NONE) => Action::SearchPrevMatch,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SearchNextMatch,
            (KeyCode::F(3), KeyModifiers::NONE) => Action::SearchNextMatch,
            (KeyCode::F(3), mods) if mods.contains(KeyModifiers::SHIFT) => Action::SearchPrevMatch,
            // Left en FileHeader expandido → colapsar
            (KeyCode::Left, KeyModifiers::NONE) => Action::SearchToggleFold,
            // Right en FileHeader colapsado → expandir
            (KeyCode::Right, KeyModifiers::NONE) => Action::SearchToggleFold,
            (KeyCode::Backspace, _) => Action::SearchDeleteChar,
            // Alt+C → toggle case sensitive
            (KeyCode::Char('c'), KeyModifiers::ALT) => Action::SearchToggleCase,
            // Alt+W → toggle whole word
            (KeyCode::Char('w'), KeyModifiers::ALT) => Action::SearchToggleWholeWord,
            // Alt+R → toggle regex
            (KeyCode::Char('r'), KeyModifiers::ALT) => Action::SearchToggleRegex,
            // Ctrl+Shift+H → toggle replace
            (KeyCode::Char('H'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchToggleReplace
            }
            // Ctrl+Shift+1 → replace current
            (KeyCode::Char('!'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchReplaceCurrent
            }
            // Ctrl+Shift+A en search → replace all in file
            (KeyCode::Char('A'), mods)
                if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
            {
                Action::SearchReplaceAllInFile
            }
            // Chars se escriben en el campo activo
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::SearchInsertChar(ch)
            }
            _ => Action::Noop,
        };
    }

    // ── Git panel activo: captura input cuando foco en Git ──
    if git_state.visible && focused_panel == PanelId::Git {
        // Modo commit: capturar chars para el mensaje
        if git_state.commit_mode {
            return match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => Action::GitCommitCancel,
                (KeyCode::Enter, KeyModifiers::NONE) => Action::GitCommitConfirm,
                (KeyCode::Backspace, _) => Action::GitCommitDeleteChar,
                (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    Action::GitCommitInput(ch)
                }
                _ => Action::Noop,
            };
        }

        // Modo diff: navegación del diff
        if git_state.show_diff {
            return match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    Action::GitToggleDiff
                }
                (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::GitDiffScrollUp,
                (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                    Action::GitDiffScrollDown
                }
                _ => Action::Noop,
            };
        }

        // Modo normal: navegación de lista de archivos
        return match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::GitClose,
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::GitUp,
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Action::GitDown,
            (KeyCode::Enter, KeyModifiers::NONE) | (KeyCode::Char('s'), KeyModifiers::NONE) => {
                Action::GitStageToggle
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => Action::GitToggleDiff,
            (KeyCode::Char('c'), KeyModifiers::NONE) => Action::GitStartCommit,
            (KeyCode::Char('r'), KeyModifiers::NONE) => Action::GitRefresh,
            _ => Action::Noop,
        };
    }

    // ── Custom keybinds del registry (prioridad sobre hardcodeados) ──
    // Solo verificar si hay overrides para evitar overhead innecesario
    if let Some(custom_action) = commands.match_key_event(key) {
        return custom_action;
    }

    // ── Atajos globales (Ctrl+algo, Esc, Tab) ──
    match (key.code, key.modifiers) {
        // Esc: si hay multicursor activo, limpiar; sino, quit
        (KeyCode::Esc, _) => return Action::ClearMultiCursor,
        // Ctrl+Tab → siguiente pestaña
        (KeyCode::Tab, KeyModifiers::CONTROL) => return Action::NextTab,
        // Ctrl+Shift+Tab → pestaña anterior
        (KeyCode::BackTab, mods)
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::PrevTab;
        }
        // Ctrl+W → cerrar pestaña activa
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => return Action::CloseTab,
        (KeyCode::Tab, KeyModifiers::NONE) => return Action::FocusNext,
        (KeyCode::BackTab, KeyModifiers::SHIFT) => return Action::FocusPrev,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => return Action::ToggleSidebar,
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => return Action::ToggleBottomPanel,
        // Ctrl+` abre/cierra terminal (con spawn automático si no hay sesión)
        (KeyCode::Char('`'), KeyModifiers::CONTROL) => return Action::ToggleTerminal,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => return Action::SaveFile,
        (KeyCode::Char('z'), KeyModifiers::CONTROL) => return Action::Undo,
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => return Action::Redo,
        // Ctrl+Shift+F abre búsqueda global.
        (KeyCode::Char('F'), mods)
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenGlobalSearch;
        }
        // Alt+Shift+P abre la command palette.
        // crossterm reporta Alt+Shift+P como 'P' mayúscula con ALT|SHIFT flags.
        (KeyCode::Char('P'), mods)
            if mods.contains(KeyModifiers::ALT) && mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenCommandPalette;
        }
        // Ctrl+P (sin Shift) abre quick open.
        // crossterm reporta Ctrl+P (sin Shift) como 'p' minúscula con CONTROL flag.
        // El guard !SHIFT asegura que no interfiera con Ctrl+Shift+P.
        (KeyCode::Char('p'), mods)
            if mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::SHIFT) =>
        {
            return Action::OpenQuickOpen;
        }
        // Ctrl+G abre Go to Line.
        (KeyCode::Char('g'), KeyModifiers::CONTROL) => return Action::OpenGoToLine,
        _ => {}
    }

    // ── Context-aware: match sobre (panel enfocado, tecla) ──
    match focused_panel {
        PanelId::Terminal => match (key.code, key.modifiers) {
            // Esc sale del foco del terminal
            (KeyCode::Esc, _) => Action::FocusNext,
            // Ctrl+C → enviar Ctrl+C al terminal
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::TerminalCtrlC,
            // Enter → enviar Enter al terminal
            (KeyCode::Enter, KeyModifiers::NONE) => Action::TerminalEnter,
            // Shift+Up / PageUp → scroll up del terminal
            (KeyCode::Up, mods) if mods.contains(KeyModifiers::SHIFT) => Action::TerminalScrollUp,
            (KeyCode::PageUp, _) => Action::TerminalScrollUp,
            // Shift+Down / PageDown → scroll down del terminal
            (KeyCode::Down, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::TerminalScrollDown
            }
            (KeyCode::PageDown, _) => Action::TerminalScrollDown,
            // Caracteres → input al terminal
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                Action::TerminalInput(ch)
            }
            _ => Action::Noop,
        },

        PanelId::Editor => match (key.code, key.modifiers) {
            // ── LSP ──
            // Ctrl+Space → autocompletado
            (KeyCode::Char(' '), KeyModifiers::CONTROL) => Action::LspCompletion,
            // F12 → go-to-definition
            (KeyCode::F(12), KeyModifiers::NONE) => Action::LspGotoDefinition,
            // Ctrl+K → hover (info de tipo)
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => Action::LspHover,

            // Movimiento de cursor
            (KeyCode::Up, KeyModifiers::NONE) => Action::MoveCursor(Direction::Up),
            (KeyCode::Down, KeyModifiers::NONE) => Action::MoveCursor(Direction::Down),
            (KeyCode::Left, KeyModifiers::NONE) => Action::MoveCursor(Direction::Left),
            (KeyCode::Right, KeyModifiers::NONE) => Action::MoveCursor(Direction::Right),
            (KeyCode::Home, KeyModifiers::NONE) => Action::MoveToLineStart,
            (KeyCode::End, KeyModifiers::NONE) => Action::MoveToLineEnd,

            // Shift + flechas → selección
            (KeyCode::Up, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Up)
            }
            (KeyCode::Down, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Down)
            }
            (KeyCode::Left, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Left)
            }
            (KeyCode::Right, mods) if mods.contains(KeyModifiers::SHIFT) => {
                Action::MoveCursorSelecting(Direction::Right)
            }

            // Ctrl+D → seleccionar siguiente ocurrencia
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::SelectNextOccurrence,

            // Edición
            (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
            (KeyCode::Enter, KeyModifiers::NONE) => Action::InsertNewline,
            (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(ch),

            _ => Action::Noop,
        },

        PanelId::Explorer => match (key.code, key.modifiers) {
            // Navegación
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::ExplorerUp,
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Action::ExplorerDown,
            // Abrir/expandir
            (KeyCode::Enter | KeyCode::Right | KeyCode::Char('l'), KeyModifiers::NONE) => {
                Action::ExplorerToggle
            }
            // Colapsar
            (KeyCode::Left | KeyCode::Char('h'), KeyModifiers::NONE) => Action::ExplorerCollapse,
            // Refresh
            (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ExplorerRefresh,

            _ => Action::Noop,
        },

        // Otros paneles — sin keybindings específicos aún
        _ => Action::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    };

    /// Helper: construye un CrosstermEvent::Key con kind=Press para testear keymap.
    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> CrosstermEvent {
        CrosstermEvent::Key(KeyEvent::new_with_kind(
            code,
            modifiers,
            KeyEventKind::Press,
        ))
    }

    /// Helper: crea un CommandRegistry con defaults para tests.
    fn test_commands() -> CommandRegistry {
        let mut commands = CommandRegistry::new();
        commands.register_defaults();
        commands
    }

    #[test]
    fn ctrl_s_in_editor_returns_save_file() {
        let commands = test_commands();
        let event = key_event(KeyCode::Char('s'), KeyModifiers::CONTROL);
        let action = keymap(
            &event,
            PanelId::Editor,
            false, // palette
            false, // quick_open
            false, // go_to_line
            false, // branch_picker
            false, // search
            &GitState::new(),
            false, // lsp_completion
            false, // settings
            false, // settings_editing
            &commands,
        );
        assert_eq!(action, Action::SaveFile);
    }

    #[test]
    fn ctrl_p_returns_open_quick_open() {
        let commands = test_commands();
        let event = key_event(KeyCode::Char('p'), KeyModifiers::CONTROL);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::OpenQuickOpen);
    }

    #[test]
    fn ctrl_shift_f_returns_open_global_search() {
        let commands = test_commands();
        let event = key_event(
            KeyCode::Char('F'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::OpenGlobalSearch);
    }

    #[test]
    fn escape_in_search_panel_returns_search_close() {
        let commands = test_commands();
        let event = key_event(KeyCode::Esc, KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Search,
            false,
            false,
            false,
            false,
            true, // search_visible
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::SearchClose);
    }

    #[test]
    fn arrow_down_in_explorer_returns_explorer_down() {
        let commands = test_commands();
        let event = key_event(KeyCode::Down, KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Explorer,
            false,
            false,
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::ExplorerDown);
    }

    #[test]
    fn enter_in_palette_returns_palette_confirm() {
        let commands = test_commands();
        let event = key_event(KeyCode::Enter, KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Editor,
            true, // palette_visible
            false,
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::PaletteConfirm);
    }

    #[test]
    fn ctrl_g_returns_open_go_to_line() {
        let commands = test_commands();
        let event = key_event(KeyCode::Char('g'), KeyModifiers::CONTROL);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::OpenGoToLine);
    }

    #[test]
    fn colon_in_quick_open_returns_open_go_to_line() {
        let commands = test_commands();
        let event = key_event(KeyCode::Char(':'), KeyModifiers::SHIFT);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            true, // quick_open_visible
            false,
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::OpenGoToLine);
    }

    #[test]
    fn digit_in_go_to_line_returns_insert_char() {
        let commands = test_commands();
        let event = key_event(KeyCode::Char('5'), KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            true, // go_to_line_visible
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::GoToLineInsertChar('5'));
    }

    #[test]
    fn enter_in_go_to_line_returns_confirm() {
        let commands = test_commands();
        let event = key_event(KeyCode::Enter, KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            true, // go_to_line_visible
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::GoToLineConfirm);
    }

    #[test]
    fn esc_in_go_to_line_returns_close() {
        let commands = test_commands();
        let event = key_event(KeyCode::Esc, KeyModifiers::NONE);
        let action = keymap(
            &event,
            PanelId::Editor,
            false,
            false,
            true, // go_to_line_visible
            false,
            false,
            &GitState::new(),
            false,
            false,
            false,
            &commands,
        );
        assert_eq!(action, Action::GoToLineClose);
    }
}
