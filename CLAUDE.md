# IDE TUI en Rust — RAM/CPU First

IDE de terminal construido en Rust con `ratatui` + `crossterm`. La restriccion de diseno principal es **performance**: RAM y CPU son budgets duros, no sugerencias.

## Documentacion clave

- **Arquitectura**: `architecture.md` — event loop, state particionado, crates, render incremental, budgets por subsistema.
- **Roadmap**: `roadmap.md` — fases, prioridades, que entra en MVP y que se difiere.
- **Tareas**: `tasks.md` — epicas y orden de implementacion.

## Skill obligatorio

Antes de escribir cualquier codigo Rust, carga el skill `.agents/skills/rust-best-practices/SKILL.md`.

Esto NO es opcional. El skill contiene reglas concretas de ownership, allocations, concurrencia, rendering y anti-patterns especificos de este proyecto.

## Reglas inviolables

1. **NUNCA hacer `.clone()` sin justificacion en comentario** (`// CLONE: {razon}`). Cada clone es una alocacion de heap.
2. **NUNCA usar `unwrap()` o `expect()` en codigo de produccion.** Son panics. Usar `?` con contexto siempre.
3. **SIEMPRE preferir borrowing sobre ownership transfer.** `&str` sobre `String`, `&[T]` sobre `Vec<T>` en parametros.
4. **SIEMPRE medir antes de optimizar.** Usar `criterion` para benchmarks y `DHAT`/`heaptrack` para allocations. No adivinar donde esta el cuello de botella.
5. **Todo codigo nuevo debe respetar los budgets de RAM/CPU** definidos en `architecture.md`:
   - Idle RAM sin LSP: < 85 MB
   - RAM en uso normal: < 100 MB
   - Input-to-render: < 20 ms (target), < 35 ms (hard limit)
   - CPU idle: ~0-1%
6. **NUNCA alocar dentro de render loops.** Pre-computar y cachear fuera del render.
7. **NUNCA usar canales unbounded.** Siempre bounded con capacidad explicita.
8. **NUNCA hacer `tokio::spawn` sin `CancellationToken`.** Toda tarea async debe ser cancelable.