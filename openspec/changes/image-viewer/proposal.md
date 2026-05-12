# Proposal: Image Viewer in Codex-Snabb

## Intent

Implement image file previews directly in the editor area (JPG, PNG, GIF, WEBP, BMP), matching VS Code's user experience. This avoids opening external viewers and improves developer workflow when handling assets.

## Scope

### In Scope
- Rendering JPG, PNG, GIF, WEBP, BMP images in the editor view.
- Multi-protocol support (Kitty > Sixel > half-blocks) via terminal detection.
- Lazy terminal capability detection (first load).
- Async background image decoding using `spawn_blocking`.
- Tab state extension matching the existing `DiffViewContent` pattern.

### Out of Scope
- Zoom and pan controls.
- RAW image formats.
- GIF frame-by-frame animations.

## Capabilities

### New Capabilities
- `image-viewer`: Displaying images inside the TUI editor view via `ratatui-image`.

### Modified Capabilities
- `editor-tabs`: Enhanced to handle non-text files, specifically opening and displaying an `ImageViewContent` state.

## Approach

Leverage `ratatui-image` (v11) and `image` crates. Disable `rayon` and default features in `image` to respect strict RAM and binary size budgets. Image decoding will be dispatched via `tokio::task::spawn_blocking` to avoid blocking the UI event loop (respecting the < 16ms render limit).
The rendering will hook into `src/ui/mod.rs` where the `DiffContent` branch exists, introducing an `ImageViewContent` inside `EditorState` to hold the image protocol data. Protocol capabilities will be determined lazily via `Picker::from_query_stdio()` on the first image load.

## Alternatives Considered

- **Sixel-only or Half-blocks only**: Simplifies implementation but provides suboptimal UX across different terminal emulators. `ratatui-image` multi-protocol handles this elegantly.
- **Custom implementation without ratatui-image**: High maintenance burden, difficult to match the efficiency of established crates, violates CPU budgets.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `Cargo.toml` | New | Add `ratatui-image` and `image` (no-default-features). |
| `src/app/mod.rs` | Modified | Handle `Action::OpenFile` properly for binary files, dispatch decode task. |
| `src/editor/mod.rs` | Modified | Add `ImageViewContent` to `EditorState`. |
| `src/editor/tabs.rs` | Modified | Add image state to tabs. |
| `src/ui/mod.rs` | Modified | Render branch for `ImageViewContent`. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Large RAM usage per image | High | Decode images lazily, ensure memory cleanup on tab close. |
| Windows Terminal half-blocks | Med | Accept lower fidelity as fallback; wait for WT upstream improvements. |
| Rust 1.88 requirement | Low | Note requirement in documentation for users building from source. |

## Rollback Plan

Revert `Cargo.toml` dependencies, drop `ImageViewContent` structs, and revert the UI branching logic in `src/ui/mod.rs` and `src/app/mod.rs`.

## Dependencies

- `ratatui-image` (v11.0.1)
- `image` (v0.25.10)

## Success Criteria

- [ ] Images (JPG/PNG) render correctly in tabs.
- [ ] UI event loop is not blocked during image load (input-to-render < 16ms).
- [ ] RAM budget stays < 100MB while viewing moderate-sized images.
