# Rusty

![Rusty terminal emulator screenshot](rustyterm.svg)

A next-generation terminal emulator for macOS, written in Rust.

![Version](https://img.shields.io/github/v/tag/DevShedLabs/rusty?label=version&sort=semver)
![License](https://img.shields.io/github/license/DevShedLabs/devscan)
![Go](https://img.shields.io/badge/built%20with-Rust-00ADD8)

## What makes it different

Most terminals are thin wrappers around a VT100 parser. Rusty is built from scratch with first-class features that other terminals bolt on as plugins — or don't have at all.

- **Tabs and split panes** — multiple independent shell sessions per window, split horizontally or vertically into any layout. Navigate with keyboard shortcuts or click the tab bar.
- **Type hinting** — fish-style inline ghost text from your command history, accepted with Tab or →. Press Tab for a full popup showing files in your current directory, PATH commands, and history matches.
- **Native rendering** — `cat file.md` or `cat file.json` renders a native overlay with syntax highlighting instead of raw text. No pipes, no extra tools.
- **Scrollback selection and copy** — click and drag to select, Cmd+C to copy, in both the terminal view and rendered overlays.
- **Configurable colours** — edit `~/.config/rusty/config.toml` to theme every ANSI colour, background, foreground, cursor, selection, tab bar, and pane borders.
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

### Tabs and split panes

Each tab is an independent shell session. Panes split the current tab into multiple independent PTYs, laid out in a recursive tree.

**Tabs**

| Key | Action |
|-----|--------|
| `Cmd+T` | New tab |
| `Cmd+W` | Close active pane; closes tab when last pane; quits when last tab |
| `Cmd+]` | Next tab |
| `Cmd+[` | Previous tab |
| Click tab | Switch to that tab |

Tab titles update automatically to the current directory name via OSC 7 (see shell setup below).

**Split panes**

| Key | Action |
|-----|--------|
| `Cmd+D` | Split active pane vertically (side by side) |
| `Cmd+Shift+D` | Split active pane horizontally (stacked) |
| `Cmd+Opt+Arrow` | Move focus to the nearest pane in that direction |

The active pane is highlighted with a 1px accent line along its top edge.

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

[hints]
fuzzy_history=false   # disabled by default as it currently causes issues in some cases.


[scroll]
natural = true    # macOS natural scroll direction
lines   = 3       # lines per wheel tick
history = 10000   # scrollback buffer size

[window]
opacity = 1.0     # 0.0 (fully transparent) → 1.0 (fully opaque)

[tabs]
bar_bg        = "#1e1e2e"   # tab bar background
bar_fg        = "#8888aa"   # inactive tab label colour
active_bg     = "#31314a"   # active tab background
active_fg     = "#e0e0ff"   # active tab label colour
active_border = "#89b4fa"   # accent line on the focused pane when splits are open
separator     = "#444466"   # divider between tabs and between split panes
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

- **Git status display** — first-class git context in the prompt area and tab titles: branch name, ahead/behind counts, dirty indicator. Reads via `libgit2` (`rusty-git` crate exists, just needs wiring to the UI).
- **Pane resize** — drag the divider between split panes to adjust the ratio.
- **Font selection** — load any system font by name from `config.toml`. Currently bundled JetBrains Mono only.
- **Session restore** — serialise the tab/pane layout to disk on quit, restore on next launch. The session serialisation code is already in `rusty-mux`.

### Structured command completion

Tab completions are powered by three layered sources, tried in order:

1. **TOML spec files** — bundled definitions for common commands (`git`, `grep`, …) plus user files in `~/.config/rusty/completions/`. Subcommands, flags, descriptions, and value hints — all declarative, no code.
2. **`--help` auto-parser** — for any command without a spec file, rusty runs `<cmd> --help` once per session, extracts flags and descriptions via regex, and caches the result.
3. **Filesystem fallback** — files and directories in CWD.

**Writing a TOML completion spec** (`~/.config/rusty/completions/cargo.toml`):

```toml
command     = "cargo"
description = "Rust package manager"

[[flags]]
long        = "verbose"
short       = "v"
description = "Use verbose output"

[[subcommands]]
name        = "build"
description = "Compile the current package"

[[subcommands.flags]]
long        = "release"
description = "Build with optimizations"

[[subcommands.flags]]
long        = "target"
description = "Cross-compile for the target triple"
takes_value = true
value_hint  = "triple"

[[subcommands]]
name        = "test"
description = "Run the test suite"

[[subcommands.flags]]
long        = "nocapture"
description = "Show stdout from passing tests"
```

Bundled specs live in `completions-toml/` in the repo. User files in `~/.config/rusty/completions/` override bundled ones. Drop a `.toml` there for any tool your team uses — `kubectl`, `gh`, internal CLIs — no rebuild needed.

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
