# Design: Image Viewer in Codex-Snabb

## Technical Approach

Extend the existing `DiffViewContent` pattern: add `ImageViewContent` to `EditorState`, add a new `Event::ImageLoaded` async result, plug a third render branch in `ui/mod.rs`, and gate all image paths behind `is_image_file()`. The `ratatui-image` `Picker` lives on `AppState` (lazy init, first image open). Decoding runs in `spawn_blocking` behind a bounded channel.

---

## Architecture Decisions

| # | Decision | Choice | Rejected | Rationale |
|---|----------|--------|----------|-----------|
| 1 | Image protocol | `ratatui-image` v11 `StatefulProtocol` + `StatefulImage` | Raw sixel / custom | StatefulProtocol auto-re-encodes on resize; StatefulImage owns the render-pass state. Zero manual resize handling. |
| 2 | Picker placement | `Option<ratatui_image::picker::Picker>` on `AppState` | Per-tab Picker | Picker does one-time terminal query (PTY I/O). Singleton shared across tabs saves ≈4 KB + PTY round-trip per image. |
| 3 | Picker init | Lazy on first `Action::OpenFile(img)` | Eager at startup | Avoids startup latency and PTY query when user never opens images. |
| 4 | Decode strategy | `spawn_blocking` → `image::open` → downscale → channel send | `tokio::spawn` with async reader | Image decoding is CPU-bound. `tokio::spawn` steals async workers. `spawn_blocking` uses the blocking pool. |
| 5 | Async result channel | Existing `mpsc::Sender<Event>` (bounded) → `Event::ImageLoaded` | New dedicated channel | Event loop already has an event channel; adding a variant costs zero architecture changes. Capacity 8 is sufficient. |
| 6 | Memory per tab | `Option<ImageViewContent>` on `EditorState`, drop on tab close | LRU cache of last N | Drop-on-close is deterministic. Only active tab's protocol buffer lives in RAM. Matches spec requirement: inactive tabs free memory. |
| 7 | `image` crate features | `default-features = false, features = ["jpeg", "png", "gif", "webp", "bmp"]` | `image = "0.25"` with defaults | Defaults pull in rayon (thread pool) and AVIF (libdav1d). Explicit features cut binary size and eliminate rayon allocation spikes. |
| 8 | File-type detection | `core/file_types.rs` (new, ZERO-alloc) | `mime_guess` crate | Avoids extra dep. `path.extension()` is O(1) borrowed; no allocation. |
| 9 | Error state | `image_error: Option<String>` on `EditorState` | Separate error modal | Consistent with how `diff_view` works. Render branch shows error inline. |

---

## Data Flow

```
Explorer/QuickOpen click on img.png
  │
  ▼
Action::OpenFile(PathBuf)               ← reducer receives
  │
  ├─ is_image_file(&path)?
  │       │ NO → tabs.open_file(&path)  (existing text path)
  │       │ YES ↓
  │
  ├─ Lazy-init state.image_picker if None
  │    Picker::from_query_stdio()        (blocking — but lazy, once only)
  │
  ├─ tabs.open_image_tab(&path)          (new: inserts EditorState with loading=true)
  │
  ├─ Effect::DecodeImage { path, tab_index }
  │
  ▼
effect processor: spawn_blocking + CancellationToken
  │
  ├─ image::open(&path)? → downscale if > 1920×1080
  ├─ picker.new_protocol(img, area, Resize::Fit(None)) → Box<dyn StatefulProtocol>
  ├─ tx.send(Event::ImageLoaded { tab_index, content: ImageViewContent })
  │    OR tx.send(Event::ImageLoadError { tab_index, error: String })
  │
  ▼
Event::ImageLoaded { tab_index, content }
  │
  ▼
reducer branch → state.tabs.set_image_content(tab_index, content)
  │
  ▼
ui/mod.rs: active_is_image_tab() == true
  │
  ▼
panels::render_image_tab(f, area, editor_mut, &tab_infos)
    StatefulImage widget → f.render_stateful_widget(...)
```

---

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `Cargo.toml` | Modify | Add `ratatui-image` v11 and `image` (no-default-features, explicit format features) |
| `src/core/file_types.rs` | **Create** | `is_image_file(&Path) -> bool`, `IMAGE_EXTENSIONS` const slice |
| `src/core/mod.rs` | Modify | Add `Event::ImageLoaded`, `Event::ImageLoadError`, `Effect::DecodeImage`; remove `dead_code` expect on `OpenFile` |
| `src/editor/image.rs` | **Create** | `ImageViewContent` struct |
| `src/editor/mod.rs` | Modify | Add `pub mod image;`, add `image_view: Option<ImageViewContent>`, `image_loading: bool`, `image_error: Option<String>` to `EditorState`; update `new()` |
| `src/editor/tabs.rs` | Modify | Add `open_image_tab()`, `active_is_image()`, `set_image_content()` |
| `src/app/mod.rs` | Modify | Add `image_picker: Option<ratatui_image::picker::Picker>` to `AppState`; implement `Action::OpenFile` reducer branch; add `Effect::DecodeImage` handler (spawn_blocking + cancel token); handle `Event::ImageLoaded` / `Event::ImageLoadError` |
| `src/ui/mod.rs` | Modify | Add `active_is_image_tab` branch in render section (parallel to `active_is_diff_tab`) |
| `src/ui/panels.rs` | Modify | Add `render_image_tab()` function |

---

## Interfaces / Contracts

```rust
// src/core/file_types.rs
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];

pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
```

```rust
// src/editor/image.rs
/// Contenido de una tab de imagen (read-only, protocol pre-encoded).
///
/// `StatefulProtocol` maneja resize automático cuando el área cambia.
/// Se construye una sola vez por apertura de imagen (no en render loop).
pub struct ImageViewContent {
    pub path: PathBuf,
    /// Box<dyn StatefulProtocol>: auto-re-encode on resize, owned by this tab.
    pub protocol: Box<dyn ratatui_image::protocol::StatefulProtocol>,
    /// Widget state que ratatui-image muta en cada render pass.
    pub image_state: ratatui_image::StatefulImage,
}
// NOT Clone — protocol is not Clone. Drop = free GPU/terminal buffer.
```

```rust
// src/core/mod.rs additions
pub enum Event {
    // ... existing ...
    ImageLoaded { tab_index: usize, content: crate::editor::image::ImageViewContent },
    ImageLoadError { tab_index: usize, error: String },
}

pub enum Effect {
    // ... existing ...
    DecodeImage { path: PathBuf, tab_index: usize, area: ratatui::layout::Rect },
}
```

```rust
// AppState addition
pub image_picker: Option<ratatui_image::picker::Picker>,
```

```rust
// render_image_tab signature
fn render_image_tab(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    focused: bool,
    editor: &mut EditorState,   // &mut — StatefulImage needs mutable state
    tab_infos: &[TabInfo],
)
```

---

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | `is_image_file` — extensions, case-insensitive, None path | `#[test]` in `file_types.rs` |
| Unit | `tabs.open_image_tab` — dedup, index correctness | `#[test]` in `tabs.rs` |
| Unit | `EditorState` with `image_loading=true` renders placeholder | manual render test |
| Integration | `Effect::DecodeImage` spawn → `Event::ImageLoaded` arrives | Mock channel test |
| Manual | Kitty/Sixel render in supported terminals | Visual inspection |

---

## Migration / Rollout

No data migration required. Feature is purely additive. Rollback: revert `Cargo.toml`, drop `editor/image.rs`, remove 3 render lines in `ui/mod.rs`, remove `Event::ImageLoaded/Error` and `Effect::DecodeImage`.

---

## Open Questions

- [ ] `Picker::from_query_stdio()` blocks (PTY query): acceptable at first-image-open or must it be deferred to `spawn_blocking`? (Likely < 5ms — measure before deciding.)
- [ ] `ratatui-image` v11 API: confirm `new_protocol(image, area, Resize::Fit(None))` signature matches current docs (API changed between v9 and v11).
- [ ] `Image` crate version compatibility: verify `image = "0.25"` works with `ratatui-image = "11"` (both pin to `image` 0.25.x).
