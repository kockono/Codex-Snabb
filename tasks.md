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
