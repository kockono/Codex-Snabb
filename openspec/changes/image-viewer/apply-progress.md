# Apply Progress: image-viewer

**Batches applied**: Phase 1 + Phase 2 + Phase 3 (Foundation, Core State & Async IO, Rendering)
**Status**: ✅ Implementation complete — Phase 4 son smoke tests manuales que requieren `cargo build` + ejecución
**Mode**: Standard (sin Strict TDD — el proyecto no tiene runner de tests)
**Date**: 2026-05-10 (Phase 1+2) / 2026-05-10 (Phase 3)

## Completed Tasks

### Phase 1 — Foundation

- [x] **1.1** `Cargo.toml`: Agregado `ratatui-image = "8"` (sin defaults, features `crossterm,image-defaults`) + `image = "0.25"` con formats explícitos (`jpeg,png,gif,webp,bmp`).
  - **Desviación**: el design pedía v11, pero v11 requiere `ratatui ^0.30` y el proyecto usa 0.29. v8 es la última versión compatible.
- [x] **1.2** `src/core/file_types.rs` **creado**: `is_image_file(&Path) -> bool` case-insensitive, con `IMAGE_EXTENSIONS` slice. Tests unitarios incluidos.
- [x] **1.3** `src/core/mod.rs`: Re-exportado `is_image_file`. Agregadas variantes `Event::ImageLoaded { tab_index, content: Box<ImageViewContent> }`, `Event::ImageLoadError { tab_index, path, error }`, `Effect::DecodeImage { path, tab_index }`.
  - **Desviación**: el design original incluía `area: Rect` en Event/Effect. En ratatui-image v8 `StatefulProtocol` auto-resize en cada render, así que no necesitamos `area` upfront.
- [x] **1.4** `src/editor/image.rs` **creado**: `ImageViewContent { path, protocol: StatefulProtocol, error: Option<String> }`. Custom `Debug` impl (StatefulProtocol no es Debug). Constructor `new(path, img, &Picker)` y `with_error(path, error)` para placeholder de fallos.
  - **Desviación**: v8 expone `StatefulProtocol` como struct concreto, no como trait. Por eso no usamos `Box<dyn StatefulProtocol>` sino el tipo directo (más eficiente, menos indirection).

### Phase 2 — Core State & Async IO

- [x] **2.1** `src/editor/mod.rs`: Campo `image_view: Option<image::ImageViewContent>` en `EditorState`. Inicializado a `None` en `new()`, `open_file()`, y los dos helpers de tests (`editor_with`, `editor_with_text` en `app/helpers.rs`).
- [x] **2.2** `src/editor/tabs.rs`:
  - `active_is_image() -> bool` (paralelo a `active_is_diff()`).
  - `open_image_tab(&Path) -> usize`: crea placeholder con `EditorState::new()` y `buffer.set_file_path(path)` (para que el tab muestre el nombre durante la carga). Dedup: reusa tab existente si ya hay una imagen abierta o un placeholder con el mismo path.
  - `set_image_content(idx, content)`: setea `image_view` después del decode.
  - `src/editor/buffer.rs`: agregado `TextBuffer::set_file_path(PathBuf)` para soportar placeholders sin tocar contenido.
- [x] **2.3** `src/app/mod.rs` `AppState`:
  - `image_picker: Option<ratatui_image::picker::Picker>` (lazy init).
  - `async_event_tx: tokio::sync::mpsc::Sender<Event>` + `async_event_rx: tokio::sync::mpsc::Receiver<Event>` con capacidad bounded `ASYNC_EVENT_CHANNEL_CAPACITY = 16`.
  - Inicialización en ambos constructors `new()` y `with_file()`.
- [x] **2.4** Branch en `Action::ExplorerToggle` y `Action::QuickOpenConfirm`: si `is_image_file(path)` → `tabs.open_image_tab(&path)` + emitir `Effect::DecodeImage { path, tab_index }`. Si no, flujo normal de `tabs.open_file()`. Las rutas de `search`/`LSP go-to-definition` no necesitan branch (line/col positioning no aplica a imágenes).
- [x] **2.5** Handler `Effect::DecodeImage`: función `spawn_decode_image(state, shutdown, path, tab_index)`.
  - Lazy init del Picker via `Picker::from_query_stdio()` con fallback a `Picker::from_fontsize((8, 16))` si la query falla.
  - Clona el Picker (es `Clone + Send + Sync`) y lo pasa a `spawn_blocking`.
  - Dentro del blocking: `ImageReader::open + decode`, downscale a 1920×1080 con `Lanczos3` si excede, construcción del protocol.
  - Resultado via bounded channel a `Event::ImageLoaded` / `Event::ImageLoadError`.
  - `CancellationToken::child_token()` del shutdown global para cancelación limpia.
  - `tokio::select!` entre cancellation y el blocking task.
- [x] **2.6** Handler `Event::ImageLoaded` / `Event::ImageLoadError`: función `handle_async_event(state, event)`.
  - `ImageLoaded` → `tabs.set_image_content(tab_index, *content)` + `update_status_cache()`.
  - `ImageLoadError` → construye `ImageViewContent::with_error(...)` placeholder + `set_image_content(...)`.
  - Loop principal hace `while let Ok(ev) = state.async_event_rx.try_recv() { handle_async_event(...) }` después de `process_effects`.
  - `process_effects` ahora toma `&mut AppState` (refactor leve de su firma).
  - **Cleanup en close**: el `Drop` de `EditorState` libera automáticamente el `StatefulProtocol` al cerrar la tab — no se necesita lógica explícita.

### Phase 3 — Rendering

- [x] **3.1** `src/ui/panels.rs` `render_image_tab()` creada (insertada después de `render_diff_tab`):
  - Signature: `pub fn render_image_tab(f, area, theme, focused, editor: &mut EditorState, tab_infos)`.
  - Estructura paralela a `render_diff_tab`: `panel_block` + `Layout` vertical (tab_bar 1 / content fill / footer 1).
  - Match exhaustivo sobre `editor.image_view.as_mut()`:
    - `None` → `Paragraph::new("Loading image...")` con `Alignment::Center` y `fg_secondary`.
    - `Some(iv)` con `iv.error.is_some()` → `iv.error.as_deref()` directo a `Paragraph` (sin clonar el String) con `fg_warning`.
    - `Some(iv)` sin error → `StatefulImage::new()` + `f.render_stateful_widget(image, content_area, &mut iv.protocol)`. **Importante**: en ratatui-image v8 `StatefulImage::new()` no toma argumentos (v11 sí toma `Option`).
  - Footer estático: `" [Ctrl+W] Cerrar tab   [Ctrl+Tab] Siguiente tab"`.
  - **ZERO allocations garantizadas**: todos los strings son `&'static str` o `&str` derivado del `String` del error vía `as_deref()`.
- [x] **3.2** `src/ui/mod.rs::render()`:
  - Signatura cambiada de `&AppState` a `&mut AppState` (requerido porque `StatefulImage` necesita `&mut StatefulProtocol`).
  - Agregado `let active_is_image_tab = state.tabs.active_is_image();` antes del `active_is_diff_tab`.
  - Rama: `if active_is_image_tab { ... pre-compute tab_infos, call active_mut, render_image_tab(), bottom_panel } else if active_is_diff_tab { ... } else { ... }`.
  - Pre-cómputo de `tab_infos` ANTES de `active_mut()` para no violar el borrow checker.
  - Call site en `app/mod.rs::event_loop` actualizado: `ui::render(frame, &mut state, theme)`.
- **Bonus**: `tabs.active_is_image()` extendida para detectar también el placeholder de carga (buffer vacío + `is_image_file(path)`). Esto bloquea input al teclado durante la ventana de decode async, evitando que el usuario tipee en un buffer vacío que se va a sobrescribir.

## Files Changed

| File | Action | What Was Done |
|------|--------|---------------|
| `Cargo.toml` | Modified | + `ratatui-image = "8"` (no defaults) + `image = "0.25"` (formats explícitos) |
| `src/core/file_types.rs` | **Created** | `is_image_file(&Path) -> bool` + tests |
| `src/core/mod.rs` | Modified | + `pub mod file_types` + re-export + `Event::ImageLoaded/Error` + `Effect::DecodeImage` |
| `src/editor/image.rs` | **Created** | `ImageViewContent` struct + constructors + Debug impl |
| `src/editor/mod.rs` | Modified | + `pub mod image` + `image_view` field + inicialización en `new`/`open_file`/tests |
| `src/editor/buffer.rs` | Modified | + `set_file_path(PathBuf)` |
| `src/editor/tabs.rs` | Modified | + `active_is_image()` (con loading detection) + `open_image_tab()` + `set_image_content()` |
| `src/app/mod.rs` | Modified | + `image_picker` + `async_event_tx/rx` + `spawn_decode_image()` + `handle_async_event()` + branches en `ExplorerToggle`/`QuickOpenConfirm` + keymap read-only + `ui::render(&mut state, ...)` |
| `src/app/helpers.rs` | Modified | + `image_view: None` en `editor_with_text` test helper |
| `src/ui/panels.rs` | Modified | + `render_image_tab()` después de `render_diff_tab` |
| `src/ui/mod.rs` | Modified | + branch `active_is_image_tab` en `render()` + signature `&mut AppState` |

## Deviations from Design

1. **ratatui-image versión**: v8 en vez de v11 (compatibilidad con ratatui 0.29).
2. **StatefulProtocol como struct concreto**, no `Box<dyn StatefulProtocol>`. La API de v8 lo permite directamente.
3. **Sin `area` en Effect/Event**: `StatefulProtocol` auto-resize en cada render — el área no se necesita upfront.
4. **`Event::ImageLoaded` lleva `Box<ImageViewContent>` ya construido**, no `DynamicImage`. El Picker es `Clone + Send + Sync` por lo que clonamos al worker, construimos el protocol allí, y enviamos el resultado completo. Esto elimina la necesidad de tener acceso a `&mut state.image_picker` desde un evento async.
5. **Canal de eventos async agregado**: el design no especificaba dónde recibir los Events del worker. Implementé un `mpsc::channel` bounded en AppState + `try_recv()` en el main loop (siguiendo el patrón LSP).
6. **`process_effects` firma cambiada**: ahora `(effects, shutdown, &mut state)` en vez de `(effects, shutdown)` para soportar `Effect::DecodeImage` que necesita mutar el picker.
7. **`ui::render()` firma cambiada a `&mut AppState`**: `StatefulImage` widget de ratatui-image requiere `&mut StatefulProtocol`. Para todas las demás ramas (diff, normal) el borrow es efectivamente inmutable — el `&mut` solo se materializa en la rama de imagen.
8. **`StatefulImage::new()` sin argumento** en v8 (el prompt original asumía v11 que toma `Option<Resize>`). Ajustado.
9. **`active_is_image()` extendida con loading detection**: detecta también el placeholder de carga (buffer vacío + path de imagen) para bloquear input durante el decode async. Sin esto, el usuario podría tipear en un buffer vacío que se va a sobrescribir al recibir `Event::ImageLoaded`.

## Issues Found

1. **`StatefulProtocol` no implementa `Debug`**: resuelto con custom `Debug` impl en `ImageViewContent` (output `<StatefulProtocol>` placeholder).
2. **Double-capture de `path` en `tokio::select!`** (cancellation arm + spawn_blocking arm) → resuelto agregando `path_for_log = path.clone()` para la rama de cancelación.
3. **Sin runner de tests** en el proyecto → los `#[cfg(test)]` quedan listos pero no se ejecutan.
4. **`StatefulImage::new()` API difiere entre v8 y v11** — el prompt sugería `StatefulImage::new(None)` (v11) pero v8 no acepta argumentos. Corregido en `render_image_tab`.
5. **Borrow conflict en `render()`** — `tab_info()` (immutable) + `active_mut()` (mutable) sobre `state.tabs`. Resuelto pre-computando `tab_infos` en una `let` antes de tomar el `&mut`.
6. **`active_is_image()` retornaba `false` durante la ventana de loading** — el placeholder tiene `image_view = None`. Resultado: las keymap rules read-only no se aplicaban y el usuario podía tipear en el buffer vacío. Resuelto extendiendo el check para detectar `buffer.file_path()` con extensión de imagen + buffer vacío.

## Remaining Tasks (Phase 4 — Manual Smoke Tests)

Estos tests requieren `cargo build` (no ejecutado por instrucción explícita) + ejecución manual del binario. NO son code tasks.

- [ ] 4.1 Abrir JPG/PNG válidos → render inline (depende del terminal: Kitty/Sixel/halfblocks fallback).
- [ ] 4.2 Abrir archivo corrupto (ej: renombrar `.txt` a `.png`) → mensaje de error sin panic.
- [ ] 4.3 Resize de terminal mientras tab de imagen está activa → re-scale automático.
- [ ] 4.4 Close tab de imagen → RAM baseline (drop del `StatefulProtocol`).

## Status

**10/10 code tasks complete (Phase 1+2+3).** Pending: Phase 4 manual smoke tests + `cargo build` verification.

**Not built/run** — el usuario pidió explícitamente no ejecutar `cargo build`. La verificación de compilación y comportamiento se hará en la fase de **verify** o por ejecución manual del usuario.

Ready for: **`sdd-verify`** o smoke testing manual con `cargo run -- some-image.png`.
