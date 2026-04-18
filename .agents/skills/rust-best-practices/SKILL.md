---
name: rust-best-practices
description: >
  Best practices de Rust para maximizar eficiencia de RAM y CPU en un IDE TUI.
  Trigger: Cuando se escriba, revise o modifique codigo Rust en este proyecto.
license: Apache-2.0
metadata:
  author: gentleman-programming
  version: "1.0"
---

# Rust Best Practices — IDE TUI RAM/CPU First

Apply these rules BEFORE and DURING any Rust code in this project. These are not suggestions — they are constraints. Every rule exists because this is a TUI IDE where memory and CPU budgets are hard limits (see `architecture.md`).

## Extended Reference

For general Rust idioms (borrowing patterns, clippy, testing, generics, type-state, documentation), consult the reference chapters:

- [Chapter 1 - Coding Styles and Idioms](references/chapter_01.md)
- [Chapter 2 - Clippy and Linting](references/chapter_02.md)
- [Chapter 3 - Performance Mindset](references/chapter_03.md)
- [Chapter 4 - Error Handling](references/chapter_04.md)
- [Chapter 5 - Automated Testing](references/chapter_05.md)
- [Chapter 6 - Generics and Dispatch](references/chapter_06.md)
- [Chapter 7 - Type State Pattern](references/chapter_07.md)
- [Chapter 8 - Comments vs Documentation](references/chapter_08.md)
- [Chapter 9 - Understanding Pointers](references/chapter_09.md)

The rules below **override** any conflicting advice in those chapters when applied to this project.

---

## 1. Ownership and Borrowing

- **Borrow over clone — ALWAYS.** If a function does not need to own data, take `&T` or `&mut T`.
- **`&str` over `String` in parameters** that do not need ownership. Accept `&str`, return `String` only when necessary.
- **NEVER `.clone()` in hot paths** (event loop, reducer, render, iterators over visible data). Every clone is a heap allocation.
- **`Cow<'_, str>`** when ownership is conditional — e.g., a function that sometimes returns a borrowed slice and sometimes an owned string.
- **Small `Copy` types (<=24 bytes)** can be passed by value. Anything bigger: borrow.
- **Justify every `.clone()`** with a `// CLONE: {reason}` comment. If you can't justify it, refactor.

```rust
// GOOD: borrows, no allocation
fn process_line(line: &str) -> Option<&str> { ... }

// BAD: unnecessary ownership
fn process_line(line: String) -> Option<String> { ... }

// CONDITIONAL: Cow when ownership varies
fn normalize(input: &str) -> Cow<'_, str> {
    if needs_transform(input) {
        Cow::Owned(transform(input))
    } else {
        Cow::Borrowed(input)
    }
}
```

## 2. Allocations

- **`Vec::with_capacity(n)`** when the size is known or estimable. Never grow a Vec by repeated pushes when you know the final size.
- **Reuse buffers in loops.** Declare the buffer outside the loop, `.clear()` at the start of each iteration.
- **`SmallVec<[T; N]>`** or stack-allocated arrays for small collections of known maximum size (e.g., cursors, selections, keybindings). Avoids heap allocation entirely.
- **Avoid `Box<dyn Trait>` in hot paths.** Use enums or generics instead. Dynamic dispatch has indirection cost AND prevents inlining.
- **`CompactString`** (or `smol_str`, `compact_str`) for short strings that appear frequently (command IDs, action names, token labels). Standard `String` has 24-byte overhead + heap allocation even for "hello".
- **Arena allocation** for related short-lived objects (e.g., render frame data). Allocate once, drop all at once.

```rust
// GOOD: pre-allocated
let mut results = Vec::with_capacity(estimated_matches);

// GOOD: buffer reuse
let mut buf = String::with_capacity(256);
for line in lines {
    buf.clear();
    process_into(&mut buf, line);
    output.push(buf.clone()); // CLONE: transferring ownership to output vec
}

// GOOD: stack-allocated small collection
let cursors: SmallVec<[CursorPos; 8]> = SmallVec::new();
```

## 3. Concurrency

- **Bounded channels ALWAYS** (`mpsc::channel` with explicit capacity) for backpressure. Unbounded channels are memory leaks waiting to happen.
- **`tokio::spawn` only for real IO** (filesystem, network, PTY). For CPU-bound computation, use `spawn_blocking`.
- **Avoid `Arc<Mutex<T>>` as the default.** Prefer message-passing (channels) when possible. If you must share state, prefer `Arc<RwLock<T>>` for read-heavy access patterns.
- **`CancellationToken`** on EVERY async task. No fire-and-forget tasks. Every spawned task must be cancellable and must respect cancellation promptly.
- **Structured concurrency:** tasks spawned by a subsystem must be tracked and joined on shutdown. No orphan tasks.

```rust
// GOOD: bounded channel with backpressure
let (tx, rx) = tokio::sync::mpsc::channel::<SearchResult>(64);

// GOOD: cancellable task
let token = CancellationToken::new();
let child_token = token.child_token();
tokio::spawn(async move {
    tokio::select! {
        _ = child_token.cancelled() => { /* cleanup */ }
        result = do_work() => { /* process */ }
    }
});

// BAD: unbounded, uncancellable
let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
tokio::spawn(async move { loop { /* forever */ } });
```

## 4. Structs and Enums

- **Order fields by size descending** to minimize padding. Largest fields first, smallest last.
- **`#[repr(C)]` only when FFI requires it.** The default Rust repr optimizes field ordering automatically — let it.
- **Enums over trait objects** when variants are finite and known at compile time. An enum of 5 variants is cheaper than `Box<dyn Widget>`.
- **Niche optimization:** use `Option<NonZeroU32>`, `Option<NonZeroUsize>`, etc. `Option<NonZeroU32>` is the same size as `u32` (4 bytes vs 8 for `Option<u32>`).
- **Avoid large enum variants.** If one variant is significantly larger than others, `Box` the large one to keep the enum small.

```rust
// GOOD: niche optimization — same size as u32
struct BufferId(NonZeroU32);
type OptionalBufferId = Option<BufferId>; // 4 bytes, not 8

// GOOD: enum instead of trait object
enum PanelWidget {
    Editor(EditorState),
    Explorer(ExplorerState),
    Terminal(TerminalState),
    Search(SearchState),
}

// GOOD: box the large variant
enum Event {
    KeyPress(KeyEvent),          // small
    SearchResults(Box<Vec<Match>>), // large variant boxed
    Resize(u16, u16),           // small
}
```

## 5. Iterators and Closures

- **Lazy iterators ALWAYS** (`.iter().filter().map()`) over imperative loops with `push`. The compiler optimizes iterator chains into tight loops — no intermediate allocations.
- **Avoid unnecessary `.collect()`** — if you only need to iterate, don't collect into a Vec. Chain iterators directly.
- **Capacity hint on `.collect()`** when you must collect: use `.collect::<Vec<_>>()` and consider `.iter().filter().collect::<Vec<_>>()` with a prior `Vec::with_capacity()` or `.size_hint()`.
- **`for_each` over `for` loop** when there is no control flow (`break`, `continue`, `return`). Enables better optimizations.
- **`Iterator::chain`** over concatenating Vecs. No allocation, lazy evaluation.

```rust
// GOOD: lazy, no intermediate allocation
let visible: Vec<&Line> = buffer.lines()
    .skip(viewport.start)
    .take(viewport.height)
    .filter(|l| !l.is_folded())
    .collect();

// BAD: unnecessary collect, then iterate again
let all_lines: Vec<Line> = buffer.lines().collect();
let visible: Vec<&Line> = all_lines.iter()
    .skip(viewport.start)
    .take(viewport.height)
    .collect();
```

## 6. Error Handling

- **`thiserror`** for library-level errors (crate boundaries, public APIs). Typed, structured, matchable.
- **`anyhow`** for application-level errors (main, CLI, top-level handlers). Context-rich, ergonomic.
- **NEVER `unwrap()` or `expect()` in production code.** These are panics. Use `?` propagation always.
- **`unwrap()` is acceptable ONLY in tests** and in `const` contexts where the value is guaranteed.
- **Error context:** always add context when propagating errors across module boundaries.

```rust
// GOOD: thiserror for library errors
#[derive(Debug, thiserror::Error)]
enum EditorError {
    #[error("buffer {id} not found")]
    BufferNotFound { id: BufferId },
    #[error("file too large: {size} bytes (max: {max})")]
    FileTooLarge { size: u64, max: u64 },
}

// GOOD: anyhow with context at app level
fn open_project(path: &Path) -> anyhow::Result<Project> {
    let config = read_config(path)
        .with_context(|| format!("failed to open project at {}", path.display()))?;
    Ok(Project::from_config(config))
}
```

## 7. Rendering (ratatui-specific)

- **Render ONLY invalidated regions.** Never full-redraw when only one panel changed.
- **Do NOT compute layout every frame** if it hasn't changed. Cache layout results and invalidate only on resize or panel toggle.
- **Widgets are stateless renderers.** Compute all data BEFORE the render pass, not during. The render function receives pre-computed state and draws — nothing else.
- **NEVER `format!()` inside render loops.** Pre-compute and cache formatted strings. `format!()` allocates a new String every call.
- **Viewport virtualization:** for editor, explorer, search results, and terminal — only render visible rows. Never iterate the full dataset.
- **Throttle renders** during verbose events (terminal output, search streaming). Cap at ~30-60 FPS equivalent.

```rust
// GOOD: pre-computed, no allocations in render
struct StatusBarData {
    branch: CompactString,      // cached, not computed each frame
    file_name: CompactString,   // cached
    cursor_pos: String,         // pre-formatted: "Ln 42, Col 7"
    dirty: bool,
}

fn render_status_bar(f: &mut Frame, area: Rect, data: &StatusBarData) {
    // Pure rendering — no computation, no format!, no allocation
    let spans = vec![
        Span::raw(&data.branch),
        Span::raw(" | "),
        Span::raw(&data.file_name),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
```

## 8. IO and Filesystem

- **Async IO for all file operations.** Never block the event loop with synchronous reads/writes.
- **Read in chunks** for large files. Never read an entire large file into memory at once.
- **Reusable read buffers.** Allocate once, reuse across reads.
- **`spawn_blocking`** for CPU-bound file processing (parsing, indexing) — not `tokio::spawn`.
- **Bounded queues** for filesystem events to prevent backpressure issues.

```rust
// GOOD: chunked async reading with reusable buffer
async fn read_file_chunked(path: &Path, tx: Sender<Vec<u8>>) -> Result<()> {
    let file = tokio::fs::File::open(path).await?;
    let mut reader = BufReader::with_capacity(8192, file);
    let mut buf = vec![0u8; 8192];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 { break; }
        tx.send(buf[..n].to_vec()).await?;
    }
    Ok(())
}
```

## 9. Testing and Profiling

- **`criterion`** for benchmarking hot paths (render, reducer, buffer operations, search). Not `#[bench]` — it's unstable.
- **`DHAT`** or **`heaptrack`** for allocation tracking. Run regularly to catch allocation regressions.
- **`tracing`** with compile-time level filtering for observability. Zero cost in release builds when properly configured.
- **Benchmark BEFORE optimizing.** Never guess where the bottleneck is. Measure, then fix.
- **Allocation budgets in tests.** Assert that hot paths stay within allocation limits.

```rust
// criterion benchmark example
fn bench_render(c: &mut Criterion) {
    let state = setup_test_state();
    c.bench_function("render_editor_viewport", |b| {
        b.iter(|| render_editor(black_box(&state)))
    });
}

// tracing with zero-cost release filtering
#[instrument(skip(state), level = "debug")]
fn reduce(state: &mut AppState, action: Action) {
    tracing::debug!(?action, "reducing");
    // ...
}
```

## 10. Anti-Patterns — EXPLICITLY PROHIBITED

These patterns are **BANNED** in this codebase. Any occurrence must be refactored:

| Anti-Pattern | Why It's Banned | Use Instead |
|---|---|---|
| `.clone()` without `// CLONE: {reason}` comment | Hidden allocations compound in hot paths | Borrow, `Cow`, or `Arc` for shared ownership |
| `String` parameter where `&str` suffices | Unnecessary ownership transfer forces caller to allocate | `&str` for read-only access |
| `Box<dyn Trait>` in hot paths | Heap indirection + no inlining + vtable lookup | Enum dispatch or generics |
| `unwrap()` / `expect()` in production | Panics crash the entire IDE | `?` propagation with context |
| Allocations inside render loops | New heap allocation every frame at 60 FPS = thousands per second | Pre-compute and cache outside render |
| `Arc<Mutex<T>>` as default concurrency | Contention, deadlock risk, hidden blocking | Message-passing with bounded channels |
| `thread::sleep` / busy-polling | Wastes CPU cycles, blocks threads | Event-driven: `tokio::select!`, channels, `Notify` |
| Unnecessary intermediate `.collect()` | Allocates a Vec just to iterate it again | Chain iterators directly |
| `format!()` in render/hot loops | Allocates a new `String` every call | Pre-format and cache |
| Unbounded channels | Memory grows without limit under load | Bounded channels with explicit capacity |
| `to_string()` / `to_owned()` in hot paths | Same as clone — hidden allocation | Borrow or `Cow` |
| Fire-and-forget `tokio::spawn` | Orphan tasks leak, can't cancel, can't shutdown cleanly | Track + `CancellationToken` |

## 11. Linting Configuration

Run regularly and enforce in CI:

```bash
# Full clippy check with performance lints
cargo clippy --all-targets --all-features -- -D warnings -D clippy::perf

# Key lints to watch
# - clippy::redundant_clone
# - clippy::large_enum_variant
# - clippy::needless_collect
# - clippy::unnecessary_to_owned
# - clippy::manual_memcpy
```

Use `#[expect(clippy::lint_name)]` with a justification comment instead of `#[allow(...)]`.

## 12. Project-Specific Budgets

These budgets come from `architecture.md` and `roadmap.md`. Every piece of code must respect them:

| Metric | Target | Hard Limit |
|---|---|---|
| Cold startup | < 150 ms | — |
| Warm startup | < 80 ms | — |
| Idle RAM (no LSP) | < 40 MB | — |
| Working RAM (MVP) | < 70 MB | — |
| Input-to-render latency | < 16 ms | < 33 ms |
| CPU idle | ~0-1% | — |

If a change pushes any metric beyond target, it must be flagged and justified before merging.
