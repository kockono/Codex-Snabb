# IDE-Rust

> Un IDE de terminal estilo VS Code, hecho en Rust y diseñado con una obsesión clara: **performance primero**.

![Vista conceptual del proyecto](img/image.png)

## ¿Qué es este proyecto?

**IDE-Rust** busca llevar la experiencia de un editor/IDE moderno al mundo de la terminal.

La idea es simple: tomar los flujos más útiles de herramientas como **VS Code** —editor central, explorer, paneles, búsqueda, terminal integrada y comandos rápidos— y llevarlos a una **TUI** (*Terminal User Interface*) hecha en Rust.

No quiere ser “VS Code completo dentro de la terminal”.
Quiere ser una versión **más austera, rápida y disciplinada**, pensada para trabajar en terminal sin sacrificar una experiencia de uso moderna.

## ¿Qué problema resuelve?

Hoy muchas herramientas de terminal son potentes, pero suelen sentirse:

- fragmentadas
- poco visuales
- incómodas para flujos largos de trabajo
- o demasiado limitadas frente a un IDE moderno

Este proyecto apunta a cerrar esa brecha: ofrecer una experiencia tipo IDE dentro de la terminal, pero sin pagar el costo excesivo de RAM y CPU de una app gráfica pesada.

## Inspiración

La inspiración principal es clara:

- **VS Code**, por sus flujos de trabajo y organización visual
- **las TUIs modernas**, por su velocidad, foco y portabilidad

La meta no es copiar una interfaz por marketing, sino capturar lo mejor de ese estilo:

- layout familiar
- navegación rápida
- paneles útiles
- buena jerarquía visual
- experiencia cómoda para trabajo diario

## Objetivos principales

El proyecto está diseñado alrededor de estos objetivos:

- **Performance first**: RAM, CPU y latencia son restricciones reales de diseño.
- **UX moderna en terminal**: look sobrio, claro y con vibra VS Code/cyberpunk, pero sin efectos caros.
- **Arquitectura austera**: módulos pequeños, medibles y extensibles.
- **Features con criterio**: primero lo que aporta valor real; lo complejo queda para más adelante.
- **Trabajo serio en terminal**: pensado para uso diario, no sólo como demo visual.

## Stack principal

El proyecto está planteado sobre:

- **Rust**
- **ratatui** para la interfaz TUI
- **crossterm** para manejo de terminal e input
- **tokio** para runtime async y coordinación de tareas

## Principios técnicos del proyecto

Hay una idea que manda sobre todas las demás: **la experiencia tiene que sentirse fluida**.

Por eso la documentación del proyecto pone foco en:

- render incremental
- estado particionado
- trabajo en background con límites claros
- cancelación explícita de tareas
- budgets concretos de memoria y latencia

En otras palabras: antes de sumar features, hay que sostener una base técnica sana.

## Estado actual

Actualmente el repositorio está centrado en **documentación, arquitectura y roadmap**.

Eso significa que la visión del producto ya está bastante clara, pero la implementación todavía está en una etapa temprana.

## Documentación clave

- `architecture.md` — decisiones de arquitectura, event loop, estado, render y budgets
- `roadmap.md` — visión de producto, prioridades y fases
- `tasks.md` — tareas y épicas planeadas

## En una frase

**Un IDE estilo VS Code para la terminal, hecho en Rust, con foco extremo en performance y una UX moderna sin despilfarrar recursos.**
