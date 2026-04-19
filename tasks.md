# Tasks — IDE TUI en Rust

> Solo documentación por ahora. Estas tareas ordenan la futura implementación.

## Convenciones

- **MVP** = imprescindible para validar producto austero
- **Post-MVP** = mejora importante, pero no bloquea valor inicial
- **Futuro** = profundidad o sofisticación diferida

---

## Épica 0 — Fundaciones técnicas y budgets (MVP)

- [ ] Definir budgets de arranque, idle RAM, input latency y frame time
- [ ] Definir arquitectura lógica de módulos/crates y ownership de estado
- [ ] Diseñar `Action`, `Event`, `Effect` y contratos de message passing
- [ ] Definir política de colas, cancelación, debounce y prioridades
- [ ] Definir esquema de configuración base y feature flags opt-in
- [ ] Definir esquema de tracing/métricas por subsistema

**Dependencias:** ninguna. Todo lo demás depende de esto.

---

## Épica 1 — Shell visual base tipo IDE (MVP)

- [ ] Diseñar layout base: sidebar, editor, panel inferior, status bar
- [ ] Definir sistema de foco y navegación entre panes
- [ ] Definir tokens visuales del tema cyberpunk austero
- [ ] Documentar reglas de render incremental por panel
- [ ] Documentar estados visuales de selección, foco, error, Git y search

**Depende de:** Épica 0

---

## Épica 2 — Editor base (MVP)

- [ ] Elegir estrategia de buffer editable a validar por benchmark (piece table vs rope simple)
- [ ] Diseñar modelo de viewport, scroll y cursor principal
- [ ] Diseñar undo/redo y dirty tracking
- [ ] Diseñar búsqueda local dentro del buffer
- [ ] Definir contratos de apertura/guardado de archivos
- [ ] Definir cómo se representarán selecciones y highlights mínimos

**Depende de:** Épica 0, Épica 1

---

## Épica 3 — Workspace explorer y project manager (MVP)

- [ ] Diseñar árbol de archivos lazy con expand/collapse
- [ ] Definir ignore básico y refresh controlado
- [ ] Diseñar lista de workspaces/proyectos recientes
- [ ] Definir persistencia mínima de metadata de proyectos recientes
- [ ] Diseñar flujo de abrir/cambiar/cerrar proyecto

**Depende de:** Épica 0, Épica 1

---

## Épica 4 — Command system y `Ctrl+Shift+P` (MVP)

- [ ] Diseñar registry central de comandos
- [ ] Definir metadata de comando: id, label, aliases, category, enablement
- [ ] Diseñar keymap inicial y conflicto entre atajos
- [ ] Documentar comportamiento del command palette y ranking básico
- [ ] Definir comandos mínimos de navegación, paneles, archivos y workspace

**Depende de:** Épica 0, Épica 1

---

## Épica 5 — `Ctrl+P` Quick Open (MVP)

- [ ] Diseñar fuente de datos para quick open sin indexador agresivo
- [ ] Definir algoritmo de ranking austero (exact/prefix/fuzzy/recents)
- [ ] Diseñar actualización cancelable del listado de paths
- [ ] Definir integración entre quick open, explorer y buffers abiertos
- [ ] Documentar límites de RAM para cache de paths

**Depende de:** Épica 3, Épica 4

---

## Épica 6 — Global search / replace (MVP)

- [ ] Diseñar API de búsqueda global bajo demanda
- [ ] Definir filtros obligatorios: case, whole word, regex, include, exclude
- [ ] Diseñar streaming progresivo de resultados al panel
- [ ] Definir virtualización del panel de resultados
- [ ] Diseñar replace austero: por match y por archivo con confirmación
- [ ] Definir estrategia de cancelación de búsquedas obsoletas
- [ ] Documentar si se usará adapter externo rápido y fallback interno austero

**Depende de:** Épica 0, Épica 1, Épica 4

---

## Épica 7 — Terminal integrada mínima (MVP)

- [ ] Diseñar boundary del subsistema terminal/PTY
- [ ] Definir una sesión mínima y su lifecycle
- [ ] Documentar scrollback acotado por líneas/bytes
- [ ] Definir política de trimming y backpressure
- [ ] Diseñar foco/input del panel terminal
- [ ] Definir integración con ejecución de comandos del workspace

**Depende de:** Épica 0, Épica 1

---

## Épica 8 — Multicursor austero y `Ctrl+D` (Post-MVP temprano)

- [ ] Definir contrato exacto de `Ctrl+D`: seleccionar siguiente coincidencia para edición multicursor
- [ ] Diseñar representación de múltiples selecciones homogéneas
- [ ] Definir reglas de edición sincronizada
- [ ] Documentar merge/colisión de selecciones
- [ ] Diseñar feedback visual para cursores múltiples sin ruido excesivo

**Depende de:** Épica 2

---

## Épica 9 — Git / Source Control panel (Post-MVP temprano)

### MVP austero del panel Git

- [ ] Diseñar snapshot de estado Git del workspace
- [ ] Definir modelo de archivos changed/untracked/staged
- [ ] Diseñar panel integrado de source control
- [ ] Definir diff básico por archivo
- [ ] Diseñar stage/unstage por archivo
- [ ] Definir flujo de commit básico
- [ ] Definir política de refresh manual + triggers puntuales

### Post-MVP del panel Git

- [ ] Diseñar stage/unstage por hunk
- [ ] Diseñar navegación entre diff y editor
- [ ] Diseñar blame simple bajo demanda

### Futuro GitLens-like

- [ ] Diseñar inline blame opcional
- [ ] Diseñar historial por archivo y navegación a commits
- [ ] Evaluar budgets para features históricas enriquecidas

**Depende de:** Épica 3, Épica 4

---

## Épica 10 — LSP austero (Post-MVP)

- [ ] Diseñar lifecycle de servidores por lenguaje/proyecto
- [ ] Definir debounce y cancelación de requests
- [ ] Diseñar diagnósticos visibles para buffers activos
- [ ] Definir hover, definition y completion básica
- [ ] Diseñar rename simple con interacción segura
- [ ] Documentar budgets de RAM/CPU por servidor LSP

**Depende de:** Épica 2, Épica 4, Épica 5

---

## Épica 11 — Observabilidad y performance (MVP continuo)

- [ ] Definir métricas de frame time e input-to-render latency
- [ ] Definir métricas de colas por worker y cancelaciones
- [ ] Definir métricas de scrollback, buffers y resultados search
- [ ] Diseñar logs para operaciones Git, terminal, search y LSP
- [ ] Definir presupuesto máximo de memoria por subsistema visible
- [ ] Diseñar panel de diagnóstico interno para fases posteriores

**Dependencias:** arranca en Épica 0 y acompaña todo el proyecto.

---

## MVP consolidado

- [ ] Épica 0 — Fundaciones técnicas y budgets
- [ ] Épica 1 — Shell visual base tipo IDE
- [ ] Épica 2 — Editor base
- [ ] Épica 3 — Workspace explorer y project manager
- [ ] Épica 4 — Command system y `Ctrl+Shift+P`
- [ ] Épica 5 — `Ctrl+P` Quick Open
- [ ] Épica 6 — Global search / replace austero
- [ ] Épica 7 — Terminal integrada mínima
- [ ] Épica 11 — Observabilidad y performance

## Post-MVP

- [ ] Épica 8 — Multicursor austero y `Ctrl+D`
- [ ] Épica 9 — Git / Source Control panel austero
- [ ] Épica 10 — LSP austero

## Futuro

- [ ] GitLens-like bajo demanda
- [ ] múltiples terminales y splits
- [ ] símbolos de workspace e indexación incremental opcional
- [ ] project manager con múltiples roots más ricos
- [ ] extensibilidad y plugins fuera de proceso

---

## Orden recomendado de implementación

1. budgets + observabilidad + arquitectura de mensajes
2. shell visual y foco
3. editor base
4. explorer + project manager
5. command palette
6. quick open (`Ctrl+P`)
7. search global / replace austero
8. terminal mínima
9. multicursor (`Ctrl+D`)
10. Git panel austero
11. LSP austero

Ese orden no es capricho. Protege el corazón del sistema: primero control del costo, después edición, después productividad, después profundidad.

---

## Features estilo VS Code (Post-MVP Enhancement)

> Estas épicas mejoran la UX visual y funcional del IDE para acercarlo a la experiencia VS Code.
> Priorizadas por impacto visual y complejidad.

---

### Épica 12 — Syntax Highlighting (Post-MVP — Impacto Alto)

El cambio visual más grande. Sin colores, parece Notepad.

- [ ] Evaluar crate de highlighting: `tree-sitter` vs `syntect` (benchmark RAM/CPU)
- [ ] Implementar tokenización por línea con cache
- [ ] Mapear tokens a colores del theme cyberpunk
- [ ] Soportar lenguajes iniciales: Rust, Python, TypeScript, JSON, TOML, Markdown
- [ ] Renderizar tokens coloreados en el viewport del editor
- [ ] Invalidar cache de tokens solo en líneas modificadas
- [ ] Asegurar que no se tokeniza fuera del viewport visible
- [ ] Respetar budgets: tokenización NO debe exceder 5ms por frame

**Depende de:** Épica 2

---

### Épica 13 — Indent Guides y Bracket Matching (Post-MVP — Impacto Alto)

- [ ] Renderizar líneas verticales `│` grises en cada nivel de indentación
- [ ] Detectar nivel de indentación por línea (tabs vs spaces, configurable)
- [ ] Highlight del par de bracket cuando el cursor está sobre `{`, `(`, `[`, `}`, `)`, `]`
- [ ] Buscar el bracket matching hacia adelante y atrás en el buffer
- [ ] Resaltar ambos brackets con color accent
- [ ] Indicar bracket sin par con color de error

**Depende de:** Épica 2

---

### Épica 14 — Auto-close y Auto-indent (Post-MVP — Impacto Alto)

- [ ] Auto-close de brackets: `(` → `()`, `{` → `{}`, `[` → `[]`
- [ ] Auto-close de quotes: `"` → `""`, `'` → `''`, `` ` `` → `` ` `` `` ` ``
- [ ] No auto-close si el siguiente carácter ya es el cierre
- [ ] Auto-indent en Enter: mantener indentación de la línea anterior
- [ ] Indent extra después de `{`, `:`, `(` al final de línea
- [ ] Dedent en `}` → reducir indentación automáticamente
- [ ] Tab key: insertar indentación (spaces o tabs según config)
- [ ] Shift+Tab: dedent de la línea/selección actual

**Depende de:** Épica 2

---

### Épica 15 — Breadcrumbs y File Icons (Post-MVP — Impacto Alto)

- [ ] Renderizar breadcrumbs debajo de las tabs: `src > ui > panels.rs`
- [ ] Parsear el path del archivo activo y mostrarlo como segmentos
- [ ] Cada segmento clickeable (futuro: navegar al directorio)
- [ ] Iconos por extensión en el explorer:
  - `.rs` → `🦀` o char representativo
  - `.toml` → `⚙`
  - `.md` → `📝`
  - `.json` → `{}`
  - `.py` → `🐍`
  - `.ts`/`.js` → `TS`/`JS`
  - Carpetas: `📁` (cerrada) / `📂` (abierta)
  - Default: `📄`
- [ ] Iconos en las tabs también (al lado del nombre del archivo)

**Depende de:** Épica 3, Épica (tabs)

---

### Épica 16 — Git Decorations en Explorer (Post-MVP — Impacto Medio)

- [ ] Colorear archivos modificados en el explorer según estado Git
- [ ] Modificados: amarillo/naranja
- [ ] Nuevos/untracked: verde
- [ ] Eliminados: rojo (tachado o dimmed)
- [ ] Ignorados: gris dimmed
- [ ] Indicar en el nombre del directorio si contiene archivos modificados
- [ ] Refrescar decoraciones al cambiar de branch o después de stage/commit

**Depende de:** Épica 3, Épica 9

---

### Épica 17 — Ctrl+H Find & Replace en archivo + Ctrl+G Go to Line (Post-MVP — Impacto Medio)

- [ ] Ctrl+H: abrir barra de búsqueda/replace dentro del archivo actual (no global)
- [ ] Búsqueda inline con highlight de matches en el editor
- [ ] Match case, whole word, regex toggles (reusar lógica de search global)
- [ ] Replace por match y replace all
- [ ] Ctrl+G: modal "Go to Line..." con input numérico
- [ ] Enter → mover cursor a esa línea y centrar viewport
- [ ] Validar rango (1 a line_count)

**Depende de:** Épica 2, Épica 6

---

### Épica 18 — Problems Panel y Notifications (Post-MVP — Impacto Medio)

- [ ] Panel de problemas en el bottom panel (tab junto a Terminal)
- [ ] Listar errores y warnings del LSP agrupados por archivo
- [ ] Click en un problema → abrir archivo y mover cursor a la línea
- [ ] Contador de errores/warnings en la status bar
- [ ] Sistema de notificaciones/toasts en esquina inferior derecha
- [ ] "File saved" → toast 2 segundos
- [ ] "Branch switched to X" → toast 2 segundos
- [ ] "LSP: rust-analyzer started" → toast
- [ ] Auto-dismiss después de timeout configurable

**Depende de:** Épica 10, Épica 7

---

### Épica 19 — Outline/Symbols Panel (Post-MVP — Impacto Medio)

- [ ] Panel de outline en sidebar (nuevo icono en activity bar)
- [ ] Listar símbolos del archivo actual: funciones, structs, enums, traits, etc.
- [ ] Obtener símbolos del LSP (`textDocument/documentSymbol`)
- [ ] Click en símbolo → mover cursor a su posición
- [ ] Buscar/filtrar símbolos
- [ ] Jerarquía: métodos dentro de structs/impls

**Depende de:** Épica 10

---

### Épica 20 — Code Folding (Futuro — Impacto Medio)

- [ ] Detectar regiones plegables por bloques `{}`, indentación, o marcadores
- [ ] Indicador en el gutter: `▸` (colapsado) / `▾` (expandido)
- [ ] Click en indicador o shortcut para colapsar/expandir
- [ ] Ctrl+Shift+[ → colapsar bloque actual
- [ ] Ctrl+Shift+] → expandir bloque actual
- [ ] Colapsar muestra: `fn main() { ... }` (primera línea + `...`)
- [ ] El viewport debe respetar líneas colapsadas

**Depende de:** Épica 2, Épica 12

---

### Épica 21 — Split Editor (Futuro — Impacto Medio)

- [ ] Dividir editor en 2 paneles lado a lado
- [ ] Ctrl+\ → split horizontal
- [ ] Cada panel tiene su propio tab bar y editor state
- [ ] Click o Tab cambia foco entre splits
- [ ] Abrir el mismo archivo en dos splits (views, no copies)
- [ ] Cerrar un split restaura el layout single

**Depende de:** Épica 2, Épica (tabs)

---

### Épica 22 — Word Wrap, Scrollbar y Pulido Visual (Futuro — Impacto Bajo)

- [ ] Toggle word wrap (Alt+Z)
- [ ] Scrollbar visual (indicador de posición en el borde derecho del editor)
- [ ] Relative line numbers (toggle en settings)
- [ ] Pin tabs (no se cierran con Ctrl+W)
- [ ] Panel tabs en bottom panel (Terminal | Problems | Output)

**Depende de:** Épica 2

---

### Épica 23 — Welcome Page y Context Menu (Futuro — Impacto Bajo)

- [ ] Pantalla de bienvenida cuando no hay archivo abierto
- [ ] Mostrar: nombre del IDE, shortcuts principales, archivos recientes, proyecto actual
- [ ] Click derecho (context menu) en editor: cut, copy, paste, go to definition, find references
- [ ] Click derecho en explorer: new file, new folder, rename, delete, copy path

**Depende de:** Épica 2, Épica 3

---

### Orden recomendado de implementación post-MVP

1. Syntax highlighting (Épica 12) — cambio visual más grande
2. Indent guides + bracket matching (Épica 13) — fácil, impacto visual enorme
3. Auto-close brackets + auto-indent (Épica 14) — se siente como IDE
4. Breadcrumbs + file icons (Épica 15) — UX visual inmediata
5. Git decorations en explorer (Épica 16) — colores en archivos
6. Find/Replace en archivo + Go to Line (Épica 17) — funcionalidad esperada
7. Problems panel + notifications (Épica 18) — feedback profesional
8. Outline panel (Épica 19) — navegación de código
9. Code folding (Épica 20) — para archivos grandes
10. Split editor (Épica 21) — productividad avanzada
11. Word wrap y pulido (Épica 22)
12. Welcome page y context menu (Épica 23)
