# Codex-Snabb — IDE TUI en Rust (RAM/CPU First)

**Package**: `ide-tui` v0.0.4  
**Brand**: Codex-Snabb  
**License**: MIT  
**GitHub**: https://github.com/kockono/Codex-Snabb

IDE de terminal construido en Rust con `ratatui` + `crossterm`. La restricción de diseño principal es **performance**: RAM y CPU son budgets duros, no sugerencias.

---

## Documentación clave

- **Arquitectura**: `architecture.md` — event loop, state particionado, crates, render incremental, budgets por subsistema.
- **Roadmap**: `roadmap.md` — fases, prioridades, qué entra en MVP y qué se difiere.
- **Tareas**: `tasks.md` — épicas y orden de implementación.
- **Contexto AI**: `.claude/CONTEXT.md` — mapa completo del proyecto para agentes.

---

## Stack

| Crate | Versión | Rol |
|-------|---------|-----|
| `ratatui` | 0.29.0 | TUI framework |
| `crossterm` | 0.28.1 | Terminal backend |
| `tokio` | 1.52.1 | Async runtime (rt-multi-thread) |
| `tokio-util` | 0.7.18 | CancellationToken |
| `tree-sitter` | 0.25.10 | Syntax highlighting (primario) |
| `syntect` | 5.3.0 | Syntax highlighting (fallback) |
| `alacritty_terminal` | 0.26.0 | Emulación VT/ANSI |
| `portable-pty` | 0.8.1 | PTY spawn |
| `lsp-types` | 0.97.0 | Tipos LSP |
| `anyhow` + `thiserror` | 1.x / 2.x | Error handling |
| `serde` + `serde_json` | 1.x | Serialización |
| `smallvec` | 1.15.1 | Vectores stack-allocated |
| `rfd` | 0.15.4 | Diálogo nativo de carpeta |
| `arboard` | 3.6.1 | Clipboard |
| `tracing` + `tracing-subscriber` | 0.1 / 0.3 | Observabilidad |

**Edición**: Rust 2024 (requiere Rust 1.85+)  
**Release profile**: `opt-level=3`, `lto="thin"`, `strip=true`  
**Single crate** — sin workspace multi-crate.

---

## Skill obligatorio

Antes de escribir cualquier código Rust, cargá el skill `.agents/skills/rust-best-practices/SKILL.md`.

Esto **NO es opcional**. El skill contiene reglas concretas de ownership, allocations, concurrencia, rendering y anti-patterns específicos de este proyecto.

---

## Reglas inviolables

1. **NUNCA hacer `.clone()` sin justificación en comentario** (`// CLONE: {razón}`). Cada clone es una alocación de heap.
2. **NUNCA usar `unwrap()` o `expect()` en código de producción.** Son panics. Usar `?` con contexto siempre.
3. **SIEMPRE preferir borrowing sobre ownership transfer.** `&str` sobre `String`, `&[T]` sobre `Vec<T>` en parámetros.
4. **SIEMPRE medir antes de optimizar.** Usar `criterion` para benchmarks y `DHAT`/`heaptrack` para allocations. No adivinar.
5. **Todo código nuevo debe respetar los budgets de RAM/CPU** definidos en `src/core/budgets.rs` (fuente de verdad):
   - Idle RAM sin LSP: < 85 MB
   - RAM en uso normal: < 100 MB
   - Input-to-render: < 16 ms (target), < 33 ms (hard limit)
   - Cold startup: < 150 ms
   - CPU idle: ~0-1%
6. **NUNCA alocar dentro de render loops.** Pre-computar y cachear fuera del render.
7. **NUNCA usar canales unbounded.** Siempre bounded con capacidad explícita.
8. **NUNCA hacer `tokio::spawn` sin `CancellationToken`.** Toda tarea async debe ser cancelable.
9. **NUNCA usar `#[allow(dead_code)]`.** Usar `#[expect(dead_code, reason = "...")]` con justificación explícita.
10. **NUNCA hacer `format!()` dentro de render loops.** Pre-computar strings antes del render.

> **Nota de budgets**: `architecture.md` tiene valores más aspiracionales (40/70 MB). La fuente de verdad es `src/core/budgets.rs` (85/100 MB). Usá siempre los valores del código.

---

## Convenciones de código

**Naming**:
- Archivos: `snake_case.rs`
- Tipos/structs/enums: `PascalCase`
- Funciones/métodos: `snake_case`
- Constantes: `SCREAMING_SNAKE_CASE`

**Error handling**:
- `thiserror` → errores de módulo/librería (tipos concretos)
- `anyhow` → nivel de aplicación (context propagation)

**Commits** (conventional commits con scope):
```
type(scope): description
```
Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `chore`, `test`  
Scopes: `editor`, `ui`, `git-panel`, `projects`, `explorer`, `highlight`, `mouse`, `quick-open`, `app`, `deps`, `config`, `terminal`, `lsp`

---

## Comandos de desarrollo

```bash
cargo run                          # Modo dev
cargo run -- path/to/file.rs       # Abrir archivo
cargo build --release              # Release build
RUST_LOG=debug cargo run           # Con logging verboso
cargo clippy --all-targets --all-features -- -D warnings -D clippy::perf
cargo fmt
```

Logs: se escriben en `ide-tui.log` (no stderr — el TUI usa alternate screen).

---

## Estado de implementación

### Implementado (funcional)

- ✅ Editor: buffer, cursor, tabs, undo/redo, multicursor (Ctrl+D), bracket matching, auto-indent, selección, search local (Ctrl+F), viewport virtualizado, unicode
- ✅ Syntax highlighting: tree-sitter (Rust, TypeScript, Go, JSON, CSS, Bash) + syntect fallback (~50 lenguajes)
- ✅ File explorer: árbol lazy, creación/eliminación, context menu (right-click), folder picker nativo
- ✅ Git panel: status, diff como tab, stage/unstage, discard, commit, fetch, push, pull, branch picker
- ✅ Terminal: VT real (alacritty_terminal), multi-pane con splits H/V, scrollback, ANSI
- ✅ Global search: search recursivo, agrupado por archivo, filtros (case/regex/include/exclude), replace por archivo
- ✅ LSP: lifecycle, transporte TCP, tipos de diagnostics/hover/completion (integración en reducer a verificar)
- ✅ Projects panel: lista, diálogo nativo de carpeta, persistencia JSON
- ✅ Command palette: fuzzy search de 100+ comandos (Alt+Shift+P)
- ✅ Quick open (Ctrl+P) + Go to Line (Ctrl+G)
- ✅ Mouse: click, scroll, drag, right-click, middle-click, dispatch por panel
- ✅ Settings overlay: editor de keybindings en runtime
- ✅ Quit modal: save/discard/cancel para buffers sucios
- ✅ Observabilidad: FrameTimer con EMA, latencia de input, frames caídos

### Deuda técnica conocida

| Archivo | Líneas | Problema |
|---------|--------|----------|
| `app/mod.rs` | ~3218 | Reducer monolítico — todo en un archivo |
| `app/keymap.rs` | ~1759 | Keymap context-aware, difícil de extender |
| `ui/panels.rs` | ~2024 | Render monolítico del editor |
| `app/mouse.rs` | ~1024 | Hit-testing monolítico |

### No implementado aún (epics futuros)

- Word wrap toggle, minimap
- Git: stage por hunk, blame inline, log/history
- LSP: find references, rename symbol, code actions, diagnostics inline, signature help, format document
- Breadcrumbs, git decorations en explorer, Problems panel, Notifications
- Outline/symbols panel, code folding, split editor
- Tree-sitter grammars: Python, C, C++, HTML, TOML, YAML, Markdown

---

## Tests y CI

**⚠️ Sin tests**: El proyecto no tiene tests unitarios, de integración, ni benchmarks.  
**⚠️ Sin CI**: No hay `.github/workflows/` configurado.

Cuando se agreguen tests, usar:
- `#[test]` para unit tests
- `criterion` para benchmarks de performance
- `DHAT`/`heaptrack` para análisis de allocations

---

## Estructura del proyecto (resumen)

```
src/
├── main.rs          — Entry, tracing init
├── core/            — Action, Event, Effect, budgets (ZERO IO)
├── app/             — Event loop, reducer monolítico, keymap, mouse
├── editor/          — Buffer, cursor, tabs, highlight, multicursor, undo
├── explorer/        — Árbol de archivos, folder picker
├── source_control_git/ — Estado git, render, branch picker
├── search/          — Motor de búsqueda global, render
├── terminal/        — PTY session, PaneTree, VT renderer
├── lsp/             — Client lifecycle, transporte, diagnósticos
├── projects/        — Lista de proyectos, persistencia
├── workspace/       — Modales: quick open, save as, rename, quit
├── ui/              — Compositor de render, layout, theme, overlays
└── observe/         — Métricas, FrameTimer
```
