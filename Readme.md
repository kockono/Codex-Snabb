# IDE-Rust

> IDE TUI en Rust, inspirado en flujos tipo VS Code, con una restricción central NO negociable: **RAM/CPU first**.

![Vista conceptual del proyecto](image.png)

## ¿Qué es?

**IDE-Rust** busca llevar a termin al una experiencia de trabajo estilo IDE agradable sin necesitar una curva de aprendizaje alta y sin el costo de un entorno gráfico pesado a cambio de una curva de aprendizaje baja. Lo mejor de 2 mundos.

La propuesta no es “meter VS Code entero en una TTY”. La propuesta es más disciplinada:

- layout familiar de IDE
- editor central + explorer + panel inferior + overlays rápidos
- arquitectura explícita para event loop, efectos y workers
- budgets medibles de memoria, CPU y latencia desde el diseño

En otras palabras: una **TUI seria para trabajo diario**, hecha en Rust, donde performance y predictibilidad importan tanto como la UX.

## Problema que intenta resolver

Muchas herramientas de terminal son potentes, pero el flujo completo suele quedar repartido entre varios programas o interfaces austeras al punto de romper continuidad.

IDE-Rust intenta cerrar esa brecha con una TUI que combine:

- navegación de proyecto
- edición
- búsqueda
- terminal integrada
- surfaces rápidas como `Ctrl+P` y command palette

Todo eso sin aceptar como “normal” un consumo excesivo de RAM, CPU o renders innecesarios.

## Stack confirmado por el repo

- **Rust** (`edition = "2024"`)
- **ratatui** para rendering TUI
- **crossterm** para input/terminal
- **tokio** para runtime async
- **portable-pty** para terminal integrada
- **regex** + **globset** para búsqueda y filtros
- **tracing** para observabilidad
- **anyhow** + **thiserror** para manejo de errores
- **tree-sitter** para syntax highlighting incremental
- **syntect** como fallback para lenguajes sin grammar tree-sitter

Referencia: `Cargo.toml`.

## Syntax highlighting — Lenguajes soportados

El highlighting usa **tree-sitter** como motor principal (parsing incremental, <1ms por keystroke) con **syntect** como fallback para extensiones no reconocidas.

| Lenguaje | Extensiones | Motor | Estado |
|----------|-------------|-------|--------|
| Rust | `.rs` | tree-sitter | ✅ Soportado |
| TypeScript | `.ts`, `.tsx` | tree-sitter | ✅ Soportado |
| Go | `.go` | tree-sitter | ✅ Soportado |
| JSON | `.json` | tree-sitter | ✅ Soportado |
| CSS | `.css` | tree-sitter | ✅ Soportado |
| Bash / Shell | `.sh`, `.bash` | tree-sitter | ✅ Soportado |
| Python | `.py` | syntect | 🔄 Pendiente grammar |
| JavaScript | `.js`, `.jsx` | syntect | 🔄 Pendiente grammar |
| C | `.c`, `.h` | syntect | 🔄 Pendiente grammar |
| C++ | `.cpp`, `.hpp` | syntect | 🔄 Pendiente grammar |
| HTML | `.html`, `.htm` | syntect | 🔄 Pendiente grammar |
| TOML | `.toml` | syntect | 🔄 Grammar viejo (^0.20) |
| YAML | `.yml`, `.yaml` | syntect | 🔄 Grammar viejo (^0.19) |
| Markdown | `.md` | syntect | 🔄 Grammar viejo (^0.20) |
| Otros | `*` | syntect | ✅ Fallback automático |

> Los lenguajes en estado 🔄 usan syntect como fallback automático — funcionan con colores, solo sin parsing incremental. Se irán migrando a tree-sitter a medida que sus grammars actualicen compatibilidad con el core `^0.25`.

### Flujo central

```text
crossterm input -> Action -> reducer/store -> Effects -> workers -> Event -> invalidation -> render
```

### Decisiones técnicas confirmadas

- **UI thread único** para input, reducción de estado y scheduling de render
- **workers dedicados** para IO/subsistemas pesados
- **message passing tipado** entre acciones, efectos y eventos
- **estado particionado** (`ui`, `workspace`, `editor`, `search`, `git`, `terminal`, `lsp`)
- **render por regiones/paneles**, no redraw conceptual completo
- **virtualización por viewport** para editor, explorer, search y terminal
- **cómputo fuera del render** para evitar allocaciones en el frame loop
- **colas acotadas + cancelación explícita** como regla de diseño

## Principios de performance

Este proyecto está diseñado con budgets explícitos, no con “optimización después”.

Metas documentadas hoy:

- **cold startup:** `< 150 ms`
- **warm startup:** `< 80 ms`
- **idle RAM sin LSP:** `< 40 MB`
- **RAM normal de uso:** `< 70 MB`
- **input-to-render:** objetivo `< 16 ms`, hard limit `< 33 ms`
- **CPU idle:** `~0% a 1%`

Principios operativos asociados:

- nada costoso corre por defecto
- search/Git/LSP deben ser cancelables
- terminal con scrollback acotado
- theming con palette precomputada
- observabilidad desde el inicio, no al final

Referencias: `architecture.md`, `roadmap.md`.

## Documentación clave

- `architecture.md` — event loop, estado, render pipeline, budgets y tradeoffs
- `roadmap.md` — visión de producto, MVP, post-MVP y límites de alcance
- `tasks.md` — breakdown de épicas y orden recomendado
- `Cargo.toml` — stack técnico real del repo

## En una frase

**IDE-Rust es un IDE TUI en Rust con layout tipo VS Code, event loop explícito y budgets de RAM/CPU definidos desde el diseño, pensado para ofrecer UX moderna en terminal sin despilfarrar recursos.**