# rusty

A next-generation terminal emulator written in Rust.

## Features

- **GPU-accelerated rendering** via `wgpu` — no CPU fallback, maximum frame throughput
- **Built-in multiplexer** — split panes, tabs, and session restore without a separate tmux/screen
- **First-class Git display** — branch, dirty state, ahead/behind counts rendered inline, always on
- **Type hinting** — fish-style inline suggestions from command history and filesystem paths
- **ANSI/VT100 compliant** parser built on the `vte` state machine
- **Modular crate architecture** — each subsystem is independently compilable

## Architecture

```
crates/
  rusty-core       Cell, Grid, Parser, Cursor — zero-dependency terminal primitives
  rusty-pty        PTY spawn / read / write / resize
  rusty-mux        Multiplexer: panes, tabs, sessions, layout tree, session restore
  rusty-git        Git status (branch, dirty, ahead/behind) via libgit2
  rusty-hint       Fish-style type-ahead completion engine
  rusty-renderer   wgpu pipeline, glyph atlas, frame renderer
  rusty-platform   Clipboard, font discovery (OS-specific)
  rusty-ui         winit window, input mapping, status bar
  rusty-app        Binary entry point
```

Dependency flow (no cycles):

```
rusty-app → rusty-ui → rusty-mux ──► rusty-core
                     → rusty-git       ▲
                     → rusty-hint ─────┘
                     → rusty-renderer → rusty-core
            rusty-mux → rusty-pty
```

## MVP Build Order

Following the spec — implement in this sequence to stay unblocked:

1. [ ] PTY spawn + raw read/write (`rusty-pty`)
2. [ ] Print-only parser + grid (`rusty-core`)
3. [ ] winit window + software blit (`rusty-ui`)
4. [ ] Full ANSI colors + cursor movement
5. [ ] Multiplexer: tabs then split panes (`rusty-mux`)
6. [ ] Git status overlay (`rusty-git`)
7. [ ] Type hinting (`rusty-hint`)
8. [ ] GPU rendering pipeline (`rusty-renderer`)
9. [ ] Scrollback, session restore, optimization pass

## Getting Started

**Prerequisites:** Rust stable (1.75+), a C compiler (for `libgit2` and `libssh2`).

```bash
# Check everything compiles
cargo check --workspace

# Build the binary
cargo build --release -p rusty-app

# Run
./target/release/rusty
```

**Faster iteration during development** — only recompile the crate you're touching:

```bash
cargo check -p rusty-core
cargo check -p rusty-mux
```

For even faster link times on macOS/Linux, add the `mold` linker:

```toml
# .cargo/config.toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

## Why Rust (not Go)

Go's GC introduces unpredictable pauses that cause frame jitter in a GPU render loop. Rust gives deterministic latency with zero-cost abstractions — critical for the PTY read → parse → grid update → draw path that must complete within a frame budget (~8ms at 120 Hz).

Compile-time mitigations:
- Fine-grained crates mean only changed code recompiles
- `cargo check` for fast feedback without codegen
- `mold` linker cuts link time significantly on Linux

## License

TBD
