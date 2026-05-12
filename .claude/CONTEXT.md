# Codex-Snabb — Contexto del proyecto para AI

**Package**: `ide-tui` v0.0.4 | **Edición Rust**: 2024 (≥ 1.85) | **Single crate**

## Qué es este proyecto

IDE de terminal construido en Rust. Ofrece una experiencia tipo VS Code (explorador, editor con tabs, panel inferior con terminal/búsqueda/Git, command palette, status bar) completamente dentro de una terminal. La restricción de diseño primaria es **RAM y CPU como budgets duros**.

## Stack completo

| Layer | Crate | Versión |
|-------|-------|---------|
| TUI | `ratatui` + `crossterm` | 0.29 + 0.28 |
| Async | `tokio` (rt-multi-thread) | 1.52 |
| Cancellation | `tokio-util` (CancellationToken) | 0.7 |
| Highlight primario | `tree-sitter` + grammars | 0.25 |
| Highlight fallback | `syntect` | 5.3 |
| Terminal emulator | `alacritty_terminal` + `portable-pty` | 0.26 + 0.8 |
| LSP | `lsp-types` | 0.97 |
| Errors | `anyhow` + `thiserror` | 1.x + 2.x |
| Serialización | `serde` + `serde_json` | 1.x |
| Small collections | `smallvec` | 1.15 |
| Native dialog | `rfd` | 0.15 |
| Clipboard | `arboard` | 3.6 |
| Observability | `tracing` + `tracing-subscriber` | 0.1 + 0.3 |

## Budgets de performance (fuente de verdad: `src/core/budgets.rs`)

| Métrica | Target | Hard Limit |
|---------|--------|-----------|
| RAM idle (sin LSP) | < 85 MB | — |
| RAM trabajando | < 100 MB | — |
| Input-to-render | < 16 ms | < 33 ms |
| Cold startup | < 150 ms | — |
| CPU idle | ~0-1% | — |

> `architecture.md` tiene valores aspiracionales más bajos (40/70 MB). El código manda.

## Event flow

```
crossterm input → Action → reducer (app/mod.rs) → Effects → workers → Events → reducer → render
```

- Un solo thread de UI (input, reducción, scheduling de render)
- Workers en `tokio::spawn` con `CancellationToken` para FS, git, PTY, LSP
- Canales bounded siempre (`tokio::sync::mpsc::channel(N)`)
- `FrameTimer` (EMA) en `observe/mod.rs` para tracking de latencia

## Mapa de archivos clave

| Archivo | Rol |
|---------|-----|
| `src/main.rs` | Entry point, tracing init |
| `src/core/mod.rs` | `Action` enum (200+ variantes), `Event`, `Effect`, `PanelId` |
| `src/core/budgets.rs` | Constantes de budget (fuente de verdad) |
| `src/core/command.rs` | `CommandRegistry`, indexado del palette |
| `src/core/settings.rs` | Keybindings, secciones del sidebar |
| `src/app/mod.rs` | `AppState`, event loop, reducer (~3218 líneas) |
| `src/app/keymap.rs` | Keymap context-aware (~1759 líneas) |
| `src/app/mouse.rs` | Hit-testing, dispatch por panel (~1024 líneas) |
| `src/editor/mod.rs` | `EditorState`, lógica de coordenadas |
| `src/editor/buffer.rs` | `TextBuffer` (Vec<String>, line-based) |
| `src/editor/cursor.rs` | `CursorState`, `Position` (Copy, 16 bytes) |
| `src/editor/tabs.rs` | `TabState` (Vec<EditorState>) |
| `src/editor/highlighting.rs` | `syntect` pipeline (fallback) |
| `src/editor/ts_highlight.rs` | Tree-sitter engine (primario) |
| `src/editor/multicursor.rs` | `MultiCursorState`, Ctrl+D |
| `src/terminal/session.rs` | `TerminalSession` (PTY + alacritty Term) |
| `src/terminal/tree.rs` | `PaneTree` (splits H/V recursivos) |
| `src/lsp/mod.rs` | `LspState`, lifecycle (~648 líneas) |
| `src/lsp/client.rs` | `LspClient` (JSON-RPC stdin/stdout) |
| `src/ui/mod.rs` | Compositor de render |
| `src/ui/panels.rs` | Editor + tab bar + gutter + status bar (~2024 líneas) |
| `src/ui/theme.rs` | `Theme` (paleta cyberpunk, precomputada) |
| `src/ui/layout.rs` | `IdeLayout`, cálculo de Rects |
| `src/observe/mod.rs` | `Metrics`, `FrameTimer` (EMA) |
| `architecture.md` | Arquitectura completa (~400 líneas) |
| `roadmap.md` | Fases y prioridades |
| `tasks.md` | Épicas e implementación |

## Módulos: descripción rápida

```
core/       — Tipos compartidos. ZERO IO. Zero dependencias externas excepto serde.
app/        — Coordinador: event loop, reducer, keymap, mouse.
editor/     — Buffer, cursor, tabs, highlight, multicursor, undo, search, viewport.
explorer/   — Árbol de archivos lazy, folder picker.
source_control_git/ — Estado Git (CLI-based, no libgit2), render, branch picker.
search/     — Motor de búsqueda global recursivo, render.
terminal/   — PTY (portable-pty), emulador VT (alacritty_terminal), PaneTree.
lsp/        — Cliente LSP: lifecycle, JSON-RPC, tipos de hover/completion/diagnostics.
projects/   — Lista de proyectos, persistencia JSON, diálogo nativo (rfd).
workspace/  — Modales: quick open, go-to-line, save as, rename, quit confirm.
ui/         — Compositor de render, layout, theme, overlays, context menu.
observe/    — FrameTimer, métricas de performance.
```

## Estado de implementación (resumen)

### ✅ Implementado y funcional
- Editor completo (buffer, cursor, tabs, undo/redo, multicursor, brackets, auto-indent, search local, viewport virtualizado)
- Syntax highlighting dual (tree-sitter: Rust/TS/Go/JSON/CSS/Bash + syntect fallback)
- File explorer (lazy tree, creación/eliminación, context menu right-click)
- Git panel (status, diff, stage/unstage, discard, commit, fetch, push, pull, branch picker)
- Terminal real (VT emulation, multi-pane H/V splits, scrollback)
- Global search (recursivo, filtros, replace por archivo)
- LSP (lifecycle, transporte, tipos — integración en reducer a verificar)
- Projects panel, Command palette, Quick open (Ctrl+P), Go to line (Ctrl+G)
- Mouse completo, Settings overlay, Quit modal, Save As, Rename modals

### ⚠️ Deuda técnica
- `app/mod.rs` (~3218 líneas): reducer monolítico
- `ui/panels.rs` (~2024 líneas): render monolítico del editor
- `app/keymap.rs` (~1759 líneas): keymap difícil de extender
- **ZERO tests** en 27K+ líneas de código
- **Sin CI** (.github/ no existe)

### 🔲 No implementado
- Git: stage por hunk, blame, log/history
- LSP: find references, rename, code actions, diagnostics inline, signature help, format
- Tree-sitter: Python, C, C++, HTML, TOML, YAML, Markdown
- Breadcrumbs, Problems panel, Outline panel, Code folding, Split editor
- Word wrap toggle, Minimap

## Reglas de desarrollo (DO/DON'T)

### DO ✅
- Borrow sobre clone (comentar SIEMPRE con `// CLONE: razón`)
- `Cow<'_, str>` para ownership condicional
- `SmallVec<[T; N]>` para colecciones pequeñas
- `Vec::with_capacity(n)` cuando se conoce el tamaño
- Pre-computar strings antes de render loops
- Bounded channels con capacidad explícita
- `CancellationToken` en todo `tokio::spawn`
- `thiserror` para errores de módulo, `anyhow` para nivel app
- `#[expect(dead_code, reason = "...")]` para código intencionalmente sin usar
- Cachear layout results, recomputar sólo en resize/toggle
- Virtualizar viewport: renderizar sólo filas visibles

### DON'T ❌
- Nunca `unwrap()` o `expect()` en producción — usar `?` con contexto
- Nunca `format!()` dentro de render loops
- Nunca alocar dentro de `render_widget()` calls
- Nunca canales unbounded
- Nunca fire-and-forget tasks (siempre `CancellationToken`)
- Nunca `Arc<Mutex<T>>` como patrón default de concurrencia
- Nunca `.collect()` innecesario en iteradores
- Nunca `#[allow(dead_code)]` — usar `#[expect(..., reason = "...")]`
- Nunca confiar en los valores de `architecture.md` para budgets — usar `budgets.rs`

## Convenciones de commit

```
type(scope): description
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `chore`, `test`  
Scopes: `editor`, `ui`, `git-panel`, `projects`, `explorer`, `highlight`, `mouse`, `quick-open`, `app`, `deps`, `config`, `terminal`, `lsp`

## Skill obligatorio

**Siempre** cargar `.agents/skills/rust-best-practices/SKILL.md` antes de escribir código Rust. 12 capítulos con reglas de ownership, allocations, concurrencia, render, anti-patterns, y linting.

Comando de lint: `cargo clippy --all-targets --all-features -- -D warnings -D clippy::perf`
