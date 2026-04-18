# Roadmap — IDE TUI en Rust (RAM/CPU First + UX Moderna)

## 1. Visión del producto

Construir un IDE TUI en Rust para trabajo diario serio, con identidad clara:

- **performance primero**: RAM/CPU/latencia como restricción de diseño
- **UX moderna**: estética **cyberpunk / VSCode-like** dentro de una TUI
- **features útiles**: no muchas por marketing, sino las correctas en el orden correcto
- **arquitectura austera**: módulos pequeños, opt-in, medibles y extensibles

La idea NO es copiar VSCode entero en terminal. La idea es tomar sus flujos más valiosos y llevarlos a una implementación TUI disciplinada.

---

## 2. Principios rectores

### 2.1 RAM/CPU first

1. Nada costoso corre por defecto.
2. Todo background work debe tener presupuesto explícito.
3. Render incremental y centrado en viewport.
4. Input nunca debe quedar bloqueado por IO, search, Git, terminal o LSP.
5. Toda feature pesada debe tener versión **austera primero** y versión **avanzada después**.

### 2.2 UX moderna sin romper performance

La UX moderna entra, pero con disciplina:

- tema visual coherente, no motor gráfico complejo
- alto contraste, jerarquía visual clara y feedback rápido
- layout tipo IDE: explorer, editor, paneles, status bar, command surfaces
- animaciones: **ninguna** o mínimas y opcionales
- decoración visual: sólo si su costo es prácticamente cero

### 2.3 Filosofía de alcance

Si una feature “estilo VSCode” requiere indexación global agresiva, watchers permanentes, caches masivos o render complejo, **NO entra en MVP**.

---

## 3. Budgets y límites operativos

### 3.1 Metas iniciales

- arranque en frío: **< 150 ms** en proyecto pequeño
- arranque en caliente: **< 80 ms**
- memoria idle sin LSP: **< 40 MB**
- memoria en uso normal MVP: **< 70 MB**
- latencia input-to-render: **objetivo < 16 ms**, máximo aceptable **< 33 ms**
- CPU idle: **~0% a 1%**

### 3.2 Presupuestos por subsistema

- **UI/render**: cero trabajo relevante en idle salvo eventos
- **search global**: bajo demanda, cancelable, con resultados progresivos
- **Git**: sin refresco continuo pesado; refresh manual o por eventos puntuales
- **terminal**: scrollback acotado y trimming explícito
- **project manager**: metadata mínima, sin indexador de workspaces
- **theming**: palette precomputada; nada de efectos dinámicos costosos

---

## 4. Módulos de producto

### 4.1 Shell visual moderna

Debe verse moderna, sobria y cyberpunk:

- sidebar / explorer
- editor central
- panel inferior para terminal, search, source control y problemas
- status bar y tabs austeros
- tema oscuro por defecto, alto contraste, colores neón moderados

**Importante:** el look entra por diseño de layout, color y tipología visual del estado; no por efectos caros.

### 4.2 Editor

Primero:

- abrir / editar / guardar
- navegación y selección básica
- undo/redo
- búsqueda local en buffer
- `Ctrl+D` estilo VSCode como **seleccionar la siguiente coincidencia para edición multicursor**

Después:

- multicursor más robusto
- acciones de selección expandida
- rename local asistido más inteligente

### 4.3 File Explorer + Project Manager

Primero:

- árbol de archivos lazy
- abrir carpeta/workspace
- recientes
- cambio rápido entre proyectos recientes

Después:

- múltiples roots/workspaces complejos
- metadata persistente más rica

### 4.4 Ctrl+P — Quick Open

Primero:

- buscador de archivos del proyecto abierto
- fuzzy matching austero
- priorización por recientes y path corto
- sin indexación global agresiva

Después:

- símbolos de workspace
- ranking más inteligente
- caches persistentes opcionales

### 4.5 Ctrl+Shift+P — Command Palette

Primero:

- catálogo central de comandos
- búsqueda fuzzy simple
- acciones de navegación, edición, paneles y toggles

Después:

- comandos aportados por plugins/extensiones
- acciones contextuales más ricas

### 4.6 Search / Replace global

Primero:

- búsqueda global bajo demanda
- resultados progresivos
- filtros:
  - match case
  - whole word
  - regex
  - include files
  - exclude files
- replace por archivo y replace seleccionado con confirmación

Después:

- replace masivo multiarchivo con preview más rica
- búsqueda estructurada o semántica
- historial persistente sofisticado

### 4.7 Source Control

#### MVP austero

- panel integrado de Git
- branch actual
- archivos modified / added / deleted / untracked
- diff básico por archivo
- stage / unstage por archivo
- discard con confirmación
- commit message básico
- refresh manual y algunos refreshes puntuales

#### Post-MVP

- stage por hunk
- diff inline más fino
- navegación de cambios entre editor y panel
- blame simple bajo demanda

#### Futuro “GitLens-like” realista

- inline blame opcional
- historial por archivo
- autores recientes / commits relacionados
- navegación contextual a commit/log

**NO entra temprano:** graph complejo, blame permanente en cada línea, minería histórica continua, dashboards pesados.

### 4.8 Terminal integrada

Primero:

- una sesión simple por panel
- shell/proceso interno
- input básico
- salida incremental
- scrollback acotado

Después:

- múltiples sesiones
- split terminales
- restore más sofisticado

### 4.9 LSP austero

Primero:

- diagnóstico
- hover
- go to definition
- completion básica
- rename simple donde sea rentable

Después:

- referencias globales
- code actions más complejas
- semántica más profunda

---

## 5. Fases del roadmap

## Fase 0 — Fundaciones y observabilidad

**Objetivo:** crear la base técnica correcta antes de meter features visibles.

Incluye:

- event loop y scheduler liviano
- estado central explícito
- layout base tipo IDE
- render incremental
- sistema de acciones/comandos
- tracing, métricas y budgets internos
- tema visual base cyberpunk austero

**No incluye:** features pesadas de productividad todavía.

## Fase 1 — MVP Editor Navegable

**Objetivo:** edición sólida y shell usable.

Incluye:

- editor básico
- árbol de archivos lazy
- tabs/status bar
- búsqueda local
- `Ctrl+Shift+P` mínimo
- project manager mínimo (abrir proyecto + recientes)

**Diferido:** multicursor completo, Git panel, terminal, grep global.

## Fase 2 — Workspace Operable

**Objetivo:** operar un proyecto real sin matar budgets.

Incluye:

- `Ctrl+P` quick open
- grep global bajo demanda con filtros
- replace austero y confirmable
- terminal integrada mínima
- command palette más útil
- manejo de workspace reciente y cambio de proyecto

**Diferido:** indexación global, replace masivo sofisticado, ranking avanzado.

## Fase 3 — Productividad de edición

**Objetivo:** subir velocidad del usuario en texto y navegación.

Incluye:

- multicursor austero
- `Ctrl+D` para seleccionar siguiente coincidencia
- selección múltiple visible y estable
- operaciones de edición sincronizada

**Diferido:** multicursor rectangular/columnar, transformaciones complejas, macros avanzadas.

## Fase 4 — Source Control integrado

**Objetivo:** cubrir el flujo Git cotidiano dentro del IDE.

Incluye:

- panel Git mínimo
- diff básico
- stage/unstage por archivo
- commit message básico
- navegación a archivos cambiados

**Diferido:** hunk staging fino, blame inline, historial enriquecido.

## Fase 5 — LSP austero

**Objetivo:** sumar inteligencia real, controlada y opt-in.

Incluye:

- diagnóstico
- hover
- definición
- completion básica
- rename simple

## Fase 6 — Post-MVP de profundidad

Incluye:

- mejoras GitLens-like bajo demanda
- indexación incremental opcional
- múltiples terminales
- project manager más rico
- símbolos de workspace
- extensibilidad controlada

---

## 6. Qué entra primero y qué se difiere

| Feature | MVP austero | Post-MVP | Futuro |
|---|---|---|---|
| Estética moderna | layout + palette + contraste | temas extra | personalización profunda |
| `Ctrl+P` | archivos del workspace abierto | ranking mejorado | símbolos + caches persistentes |
| `Ctrl+Shift+P` | comandos internos | contexto más rico | extensiones/plugins |
| Multicursor | selección de siguientes coincidencias, edición sincronizada | cursores múltiples arbitrarios | column mode / macros |
| Search global | grep bajo demanda + filtros | replace con preview rica | búsqueda estructurada/semántica |
| Replace (`Ctrl+D`) | seleccionar próxima coincidencia para editar | rename local asistido | refactors más complejos |
| Git panel | estado, diff básico, stage/unstage por archivo, commit básico | hunk staging, blame simple | GitLens-like enriquecido |
| Terminal | sesión mínima con buffer acotado | varias sesiones | splits/restauración avanzada |
| Project manager | abrir/cambiar recientes | metadata de workspace | múltiples roots avanzados |

---

## 7. Qué NO construir al inicio

- graph visual complejo de Git
- blame inline permanente
- watchers sofisticados siempre activos
- indexación semántica global continua
- terminal emulator full-fat desde el día 1
- plugin runtime in-process
- múltiples roots complejos desde MVP
- themes con efectos dinámicos o runtime costoso
- multicursor avanzado antes de estabilizar el editor base
- search global live sobre cada tecla en todo el repo

Esto no es timidez. Es arquitectura con disciplina.

---

## 8. Riesgos y mitigaciones

### Riesgo: querer “verse como VSCode” demasiado rápido
**Mitigación:** copiar flujos, no peso estructural.

### Riesgo: quick open y grep empujan indexación prematura
**Mitigación:** usar escaneo lazy, resultados progresivos y refresh explícito.

### Riesgo: multicursor complica demasiado el editor
**Mitigación:** empezar por selección repetida de coincidencias, no por modelo arbitrario total.

### Riesgo: Git panel deriva en cliente Git completo
**Mitigación:** recortar a estado + diff + stage/commit básico en MVP.

### Riesgo: terminal integrada consume RAM sin control
**Mitigación:** scrollback limitado, trimming y sesiones mínimas.

---

## 9. Resumen ejecutivo

La dirección correcta es un IDE TUI en Rust con **apariencia moderna y flujos tipo VSCode**, pero construido desde una base RAM/CPU-first.

El orden correcto es:

1. fundaciones y observabilidad
2. editor y shell base
3. quick open + search + terminal mínima + project manager
4. multicursor austero
5. Git integrado austero
6. LSP y profundidad incremental

La regla madre sigue intacta:

> **si una feature mejora la demo pero empeora budgets, estabilidad o simplicidad, se difiere.**
