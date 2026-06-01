# Rusty

![Rusty terminal emulator screenshot](rustyterm.svg)

A next-generation terminal emulator for macOS, written in Rust.

![Version](https://img.shields.io/github/v/tag/DevShedLabs/rusty?label=version&sort=semver)
![License](https://img.shields.io/github/license/DevShedLabs/devscan)
![Go](https://img.shields.io/badge/built%20with-Rust-00ADD8)

## What makes it different

Most terminals are thin wrappers around a VT100 parser. Rusty is built from scratch with first-class features that other terminals bolt on as plugins — or don't have at all.

- **Type hinting** — fish-style inline ghost text from your command history, accepted with Tab or →. Press Tab for a full popup showing files in your current directory, PATH commands, and history matches.
- **Native rendering** — `cat file.md` or `cat file.json` renders a native overlay with syntax highlighting instead of raw text. No pipes, no extra tools.
- **Scrollback selection and copy** — click and drag to select, Cmd+C to copy, in both the terminal view and rendered overlays.
- **Configurable colours** — edit `~/.config/rusty/config.toml` to theme every ANSI colour, background, foreground, cursor, and selection highlight.
- **GPU-accelerated** — Metal backend on Apple Silicon via `wgpu`. No CPU fallback, no Electron.

## Features

### Terminal

- Full ANSI/VT100 + SGR colour parsing (`vte` state machine)
- 256-colour and true-colour (RGB) support
- Cursor movement, erase, scroll regions
- 10,000-line scrollback buffer with mouse-wheel scroll
- Resize preserves content (reflow, not wipe)

### Type hinting

- **Ghost text** — dim inline suggestion from command history while you type
- **Popup completion** — Tab opens a list showing:
  - Current directory contents (blue = directories, white = files)
  - Matching PATH executables (green)
  - Matching history entries (yellow)
- Arrow keys navigate the popup, Tab or Enter accepts, Esc dismisses
- Any keystroke closes the popup and passes through normally
- History loaded from `~/.zsh_history` or `~/.bash_history` at startup
- CWD updated automatically via OSC 7 (add one line to `.zshrc` — see below)

### Native rendering overlay

Triggered automatically when you run:
```
cat file.md      # Markdown renderer
cat file.json    # JSON renderer
bat file.md      # also works with bat, less, more, head, tail
```

**Markdown** renders: headings (█▌░ size indicators), **bold**, *italic*, `` `inline code` `` (orange), blockquotes, lists, links, fenced code blocks, horizontal rules.

**JSON** renders: keys (blue), strings (green), numbers (orange), booleans (yellow), null (dim italic), pretty-printed with indentation.

Overlay controls:
- ↑↓ / PageUp/PageDown / mouse wheel — scroll
- Click and drag — select text
- Cmd+C — copy selection
- q / Esc / Enter — dismiss

### Input

- Full keyboard support: all printable chars, Shift, Option, Ctrl+key (→ control bytes), F1–F12, Home/End/PageUp/PageDown/Insert/Delete, arrows
- Cmd+C — copy terminal selection
- Cmd+V — paste from clipboard
- Click and drag to select text; auto-scrolls when dragging near edges

### Configuration

On first launch rusty writes `~/.config/rusty/config.toml` with all defaults. Edit it and restart to apply.

```toml
[palette]
background   = "#131313"
foreground   = "#d8d8d8"
cursor       = "#f8f8f0"
selection_bg = "#264f78"
selection_fg = "#ffffff"

# Normal ANSI colours (0-7)
black   = "#131313"
red     = "#e05a4f"
green   = "#87c36c"
yellow  = "#e5c076"
blue    = "#6ba3e0"
magenta = "#c07dd4"
cyan    = "#5bc8d4"
white   = "#c5c5c5"

# Bright ANSI colours (8-15)
bright_black   = "#525252"
bright_red     = "#ff7a70"
bright_green   = "#a8e08a"
bright_yellow  = "#ffdc9a"
bright_blue    = "#8fc3ff"
bright_magenta = "#da9ff5"
bright_cyan    = "#7fe3ee"
bright_white   = "#ffffff"

[font]
size   = 16.0
family = "JetBrains Mono"   # bundled — system font lookup coming later

[scroll]
natural = true    # macOS natural scroll direction
lines   = 3       # lines per wheel tick
history = 10000   # scrollback buffer size
```

### Recommended shell setup

Add to `~/.zshrc` for best results:

```zsh
# Tell rusty your current directory (enables accurate Tab completions)
precmd() { print -Pn "\e]7;file://%M${PWD}\a" }

# Modern prompt — current path + git branch, no username clutter
autoload -Uz vcs_info
precmd() {
    vcs_info
    print -Pn "\e]7;file://%M${PWD}\a"
}
zstyle ':vcs_info:git:*' formats ' %F{8}on%f %F{13} %b%f%u%c'
zstyle ':vcs_info:git:*' check-for-changes yes
zstyle ':vcs_info:git:*' unstagedstr '%F{11}●%f'
zstyle ':vcs_info:git:*' stagedstr '%F{10}●%f'
setopt PROMPT_SUBST
PROMPT='%F{12}%~%f${vcs_info_msg_0_}
%F{13}❯%f '
```

## Architecture

```
crates/
  rusty-config     TOML config loading, colour palette, font/scroll settings
  rusty-core       Cell, Grid, Parser, Cursor — zero-dependency terminal primitives
  rusty-pty        PTY spawn / read / write / resize (portable-pty)
  rusty-mux        Multiplexer: panes, tabs, sessions, layout tree, session restore
  rusty-hint       Type-ahead engine: history index, PATH commands, filesystem completions
  rusty-render     Native Markdown and JSON renderers, trigger detection
  rusty-git        Git status (branch, dirty, ahead/behind) via libgit2
  rusty-renderer   wgpu pipeline stubs, glyph atlas (GPU pipeline in progress)
  rusty-platform   Clipboard, font discovery (OS-specific)
  rusty-ui         winit window, Metal surface, software rasteriser, event loop
  rusty-app        Binary entry point
```

Dependency flow (no cycles):

```
rusty-app → rusty-ui ──→ rusty-mux   ──→ rusty-core
                     ──→ rusty-hint  ──→ rusty-core
                     ──→ rusty-render
                     ──→ rusty-config
                     ──→ rusty-pty
            rusty-mux ──→ rusty-pty
```

## Install

**Download** the latest `Rusty-macos.zip` from [Releases](https://github.com/DevShedLabs/rusty/releases), unzip, and drag `Rusty.app` to `/Applications`.

**First launch — Gatekeeper warning**

Because Rusty is not yet notarized with Apple, macOS will block it on the first open. To get past this, right-click (or Control-click) `Rusty.app` and choose **Open**, then click **Open** in the dialog. You only need to do this once.

Alternatively, from the terminal:

```bash
xattr -d com.apple.quarantine /Applications/Rusty.app
```

Then double-click as normal.

---

**Build from source:**

```bash
cargo build --release -p rusty-app
./target/release/rusty
```

**Build a `.app` bundle:**

```bash
./bundle.sh           # produces Rusty.app in the project root
open Rusty.app        # test it
cp -r Rusty.app /Applications/
```

**Prerequisites:** Rust stable 1.75+, a C compiler (Xcode Command Line Tools on macOS).

**Faster iteration** — only recompile the crate you're touching:

```bash
cargo check -p rusty-core
cargo check -p rusty-hint
```

## Roadmap

### Near-term

- **Tabs and split panes** — the layout tree (`rusty-mux`) is already designed for it; each pane is an independent PTY + grid. Keyboard shortcuts to create, navigate, and resize panes.
- **Git status display** — first-class git context in the prompt area and tab titles: branch name, ahead/behind counts, dirty indicator. Reads via `libgit2` (`rusty-git` crate exists, just needs wiring to the UI).
- **Font selection** — load any system font by name from `config.toml`. Currently bundled JetBrains Mono only.
- **Session restore** — serialise the tab/pane layout to disk on quit, restore on next launch. The session serialisation code is already in `rusty-mux`.

### Structured command completion (`rusty-completions`)

The current Tab popup pulls from history, CWD, and PATH. The next step is a full structured completion engine — a port of the [Upterm](https://github.com/railsware/upterm) TypeScript completion model to Rust, extended with user-definable TOML files.

**How it works:**

Each command has a registered `CompletionProvider`. On Tab press, rusty parses the current line, identifies the command, and calls the provider with full context:

```rust
pub struct CompletionContext {
    pub cwd:           PathBuf,
    pub argv:          Vec<String>,   // full tokenised command line
    pub current_token: String,        // token at cursor
    pub token_index:   usize,         // which argument position
}

pub struct Suggestion {
    pub label:  String,
    pub detail: Option<String>,       // shown in popup right column
    pub insert: Option<String>,       // override for what gets inserted
    pub kind:   SuggestionKind,       // Subcommand | Flag | File | Directory | Value | History
}
```

Providers compose — a `git checkout` provider combines live branch names (from libgit2), unstaged files, and static flag definitions.

**Two tiers — no code required for most commands:**

*Tier 1 — TOML definition files.* Drop a file in `~/.config/rusty/completions/` and rusty loads it at startup. Covers subcommands, flags, and static values:

```toml
# ~/.config/rusty/completions/cargo.toml
[command]
name = "cargo"

[[subcommands]]
name = "build"
detail = "Compile the current package"

  [[subcommands.flags]]
  label = "--release"
  detail = "Build with optimizations"

  [[subcommands.flags]]
  label = "--target"
  detail = "Cross-compile for the target triple"
  takes_value = true

[[subcommands]]
name = "test"
detail = "Run the test suite"

  [[subcommands.flags]]
  label = "--nocapture"
  detail = "Show stdout from passing tests"
```

*Tier 2 — Dynamic Rust providers.* For context-aware completions that need live data — `git branch`, `npm run` scripts from `package.json`, docker container names, etc.:

```rust
#[async_trait]
pub trait CompletionProvider: Send + Sync {
    async fn suggest(&self, ctx: &CompletionContext) -> Vec<Suggestion>;
}
```

**Built-in providers (porting from Upterm reference):**

| Command | Dynamic sources |
|---------|----------------|
| `git` | live branches, remotes, staged/unstaged files, aliases (via libgit2) |
| `npm` | subcommands + `scripts` from `package.json` in CWD |
| `brew` | subcommands |
| `cd`, `ls`, `cp`, `mv`, `rm` | file/directory completions |
| `find`, `grep`, `tail` | flags |
| `docker` | container/image names |
| `cargo` | subcommands + flags |

**User-defined completions** for any tool your team uses — `kubectl`, `gh`, internal CLIs — without waiting for a built-in. The TOML format is intentionally simple enough that a description in a PR is enough to write one.

### Medium-term

- **Full document renderer** — GitHub-style Markdown with `pulldown-cmark` (CommonMark spec, tables, task lists, footnotes), syntax-highlighted code blocks via `syntect` (VS Code themes, 500+ languages), and a proper layout engine with word-wrap, heading sizes, and table column sizing. The current overlay is a placeholder.
- **SSH integration** — connect to remote hosts and run a full terminal session without leaving rusty. Persistent sessions survive disconnects.
- **Image rendering** — inline images in the overlay (PNG/JPG via a software decoder, blitted into the framebuffer).
- **Plugin system** — WASM-based plugins that can hook into PTY output, render custom overlays, and add commands. Dev-awareness features (detect running servers, show ports, scan for outdated runtimes) would live here.

### Platform

- **Linux** — Vulkan backend via `wgpu`. The abstraction layer is in place; needs backend feature flags and `/dev/pts` PTY paths.
- **Windows** — DX12 backend via `wgpu` + ConPTY. Same abstraction layer.

## Platform

Currently macOS / Apple Silicon only. The Metal backend is selected at compile time. Linux (Vulkan) and Windows (DX12) support is planned — the abstraction layer is already in place via `wgpu`, it just needs the backend feature flags and platform-specific PTY paths.

## Why Rust

Go's garbage collector introduces unpredictable pauses that cause frame jitter in a GPU render loop. Rust gives deterministic latency on the critical path: PTY read → ANSI parse → grid update → rasterise → Metal submit — all must complete within ~8ms at 120 Hz.

Compile times are the tradeoff. Mitigations already in place:
- Fine-grained crates — only the changed crate recompiles
- `cargo check` for fast type-checking without codegen
- `target-cpu=apple-m1` in `.cargo/config.toml` for M-series optimised output

## License

MIT 
