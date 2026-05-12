# Verify Report — image-viewer

**Date**: 2026-05-11  
**Change**: image-viewer (inline image preview in TUI)  
**Verdict**: FAIL (compilation errors)

---

## Compilation

**FAIL** — 11 compilation errors in `src/app/mouse.rs`.  

The `process_effects` function signature was changed from 2 parameters to 3 (`effects`, `shutdown`, `&mut AppState`), but all 11 call sites in `mouse.rs` still pass only 2 arguments (`&effects`, `&CancellationToken::new()`), missing the required `&mut AppState` parameter.

```
error[E0061]: this function takes 3 arguments but 2 arguments were supplied
  --> src/app/mouse.rs:194, 277, 289, 592, 601, 628, 636, 643, 653, 857, 885
```

This is a **merge oversight** — the `process_effects` signature was updated in `app/mod.rs` but the call sites in `mouse.rs` (which uses `super::process_effects`) were not updated to pass the new `state` parameter.

**0 warnings** related to image-viewer code.

---

## Files

| File | Exists | Notes |
|------|--------|-------|
| `src/core/file_types.rs` | ✅ PASS | Created, 87 lines |
| `src/editor/image.rs` | ✅ PASS | Created, 88 lines |
| `openspec/changes/image-viewer/proposal.md` | ✅ PASS | Exists |
| `openspec/changes/image-viewer/spec.md` | ✅ PASS | Exists |
| `openspec/changes/image-viewer/design.md` | ✅ PASS | Exists |
| `openspec/changes/image-viewer/tasks.md` | ✅ PASS | Exists |
| `openspec/changes/image-viewer/apply-progress.md` | ✅ PASS | Exists |

---

## Spec Compliance

### REQ-01: Extensiones soportadas (jpg, jpeg, png, gif, webp, bmp case-insensitive)
**PASS** — `src/core/file_types.rs` defines `IMAGE_EXTENSIONS = &["jpg", "jpeg", "png", "gif", "webp", "bmp"]` with `to_ascii_lowercase()` comparison. Unit tests cover lowercase, uppercase, mixed-case, non-image extensions, and no-extension paths.

### REQ-02: Apertura desde explorer, quick open, CLI
**PASS** — Both entry points implemented:
- `ExplorerToggle` branch (line ~797): `if is_image_file(&path)` → `open_image_tab` + `Effect::DecodeImage`
- `QuickOpenConfirm` branch (line ~1115): Same pattern with `is_image_file` check

### REQ-03: Tab read-only, sin ícono de modificado
**PASS (with caveat)** — 
- Keymap uses `active_is_diff() || active_is_image()` (line 3073 in mod.rs) for read-only key blocking.
- `TabInfo::is_dirty` correctly returns `false` for image tabs (buffer is empty = not modified).
- **Caveat**: `tab_info()` doesn't have an explicit branch for image tabs — image content tabs fall through to the "normal tab" path. This works because image buffers are always empty/unused, but it would be cleaner to have an explicit branch similar to `diff_view`.

### REQ-04: Detección lazy de protocolo (Picker)
**PASS** — `AppState::image_picker: Option<Picker>` starts as `None`. `spawn_decode_image` lazy-inits via `Picker::from_query_stdio()` with fallback to `from_fontsize((8, 16))` on error. Only initialized on first image open.

### REQ-05: Decodificación async en spawn_blocking
**PASS** — `tokio::task::spawn_blocking` wraps the full decode pipeline (ImageReader → decode → resize → ImageViewContent::new). Results sent via bounded async channel `mpsc::channel::<Event>(16)`.

### REQ-06: Imagen escalada al área, aspect ratio
**PASS** — `Picker::new_resize_protocol(img)` creates a `StatefulProtocol` that auto-resizes to fit the render area while preserving aspect ratio (ratatui-image v8 behavior). The render uses `f.render_stateful_widget(image, content_area, &mut iv.protocol)` which re-encodes on area change.

### REQ-07: Imagen cacheada por tab, no re-decodifica
**PASS** — `ImageViewContent` is stored in `EditorState::image_view: Option<ImageViewContent>`. Decoded once via `spawn_blocking`, stored in the tab's state, and reused on every render. No re-decode.

### REQ-08: Al cerrar tab, imagen liberada
**PASS** — `EditorState` contains `image_view: Option<ImageViewContent>`. When a tab is closed via `close_active()`, the `EditorState` is removed from the Vec, and `Drop` is called automatically. `StatefulProtocol` owns its internal buffer, which is freed on drop. No manual cleanup needed (Rust ownership model).

### REQ-09: Error de decodificación visible
**PASS** — `ImageViewContent::with_error(path, error)` creates a placeholder with `error: Some(String)`. The render method in `render_image_tab` matches `Some(iv) if iv.error.is_some()` and displays the error message with `iv.error.as_deref()`. Error messages come from `anyhow` formatting (`format!("{e:#}")`).

### REQ-10: Keybindings read-only en image tabs
**PASS** — The keymap receives `editor_active_is_diff: bool` parameter. At call site (line 3073), it's computed as `state.tabs.active_is_diff() || state.tabs.active_is_image()`. When `active_is_image` is true, the keymap short-circuits to read-only mode (only scroll + close tab), same as diff tabs.

### REQ-11: Solo imagen activa en RAM
**WARNING** — There is no explicit mechanism to release images from inactive tabs. All image tabs retain their `StatefulProtocol` in memory even when not active. The spec suggests aggressive memory release for inactive images, but the current implementation keeps all decoded images in memory. This is acceptable for MVP (few images open simultaneously) but may need attention for heavy usage.

---

## Best Practices

### `.clone()` without `// CLONE:` comment
**WARNING** — All `.clone()` calls in image-related code have `// CLONE:` justification comments. ✅  
However, `path.to_path_buf()` in `tabs.rs:228` (inside `open_image_tab`) has a `// CLONE:` comment. ✅

### `unwrap()` or `expect()` in production code
**CRITICAL** — Line 2764 in `mod.rs`: `.expect("picker recién inicializado arriba")`  
This `.expect()` is actually **safe** — it follows `if state.image_picker.is_none() { ... init ... }` so the picker is guaranteed to be `Some`. However, by project rules (`CLAUDE.md`), production code should use `?` not `expect()`. This could be refactored to an `if let Some(picker) = state.image_picker.as_ref()` after init, or the init block could return the picker directly.

### `format!()` inside render loops
**PASS** — No `format!()` calls inside `render_image_tab`. All strings are `'static` literals. Error messages use `iv.error.as_deref()` (zero alloc).

### `#[allow(dead_code)]` in new code
**PASS** — No `#[allow(dead_code)]` found. `#[expect(dead_code, reason = "...")]` is used correctly in `tabs.rs:28`.

### `spawn_blocking` with `CancellationToken`
**PASS** — `tokio::select!` with `cancellation.cancelled()` is used. `shutdown.child_token()` provides cancellation propagation. The async task respects shutdown.

---

## Performance

### Zero allocations in render loop
**PASS** — `render_image_tab` has zero allocations:
- Loading state: static string `"Loading image..."`
- Error state: `iv.error.as_deref()` — borrows, no alloc
- Image render: `StatefulImage::new()` is stateless (v8), `render_stateful_widget` mutates the protocol in-place
- Footer: static string span, pre-computed
- `tab_infos` computed BEFORE `active_mut()` call — avoids borrow conflict

### Bounded channel with explicit capacity
**PASS** — `ASYNC_EVENT_CHANNEL_CAPACITY = 16` with `tokio::sync::mpsc::channel::<Event>(16)`. Bounded with explicit capacity. ✅

---

## Overall Verdict

**FAIL** — Compilation errors prevent building the project.

## Issues Found

### CRITICAL
1. **`process_effects` signature mismatch in mouse.rs** — 11 call sites in `src/app/mouse.rs` call `process_effects(&effects, &CancellationToken::new())` with 2 args, but the function now requires 3 args (`effects`, `shutdown`, `&mut AppState`). Every call site needs `state` passed through or an alternative approach (e.g., mouse handler receives `&mut AppState`).

### WARNING
1. **`.expect()` in `spawn_decode_image`** — Line 2764 uses `.expect("picker recién inicializado arriba")`. While logically safe, project rules require `?` with context rather than `expect()`. Could use `if let` pattern instead.
2. **No explicit image tab branch in `tab_info()`** — Image tabs fall through to the "normal tab" code path. Works correctly because the buffer is empty, but an explicit branch (like the diff tab has) would be more maintainable and resilient to future changes.
3. **REQ-11 partially unmet** — No mechanism to release images from inactive tabs. Acceptable for MVP but should be tracked for future optimization.

### SUGGESTION
1. **Consider adding a dedicated image tab branch in `tab_info()`** with `is_dirty: false` explicitly, mirroring the diff tab pattern. This improves clarity and prevents future bugs if someone adds content to an image tab's buffer.
2. **`StatefulProtocol` Debug impl** — Handled correctly with manual `Debug` impl for `ImageViewContent`. Good pattern.
3. **Downscale threshold** — 1920×1080 downscale threshold is reasonable. Consider making it a constant in `budgets.rs` or `image.rs` for discoverability.
4. **Lazy Picker initialization** — `Picker::from_query_stdio()` is called lazily. The fallback to `from_fontsize((8, 16))` on error is correct for terminals that don't support Kitty/Sixel/iTerm2 protocols.