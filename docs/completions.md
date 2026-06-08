# Rusty Completion Engine

Tab completions in rusty are powered by a layered engine in `crates/rusty-hint/`. Each Tab press consults sources in order, stopping at the first that produces results.

## How it works

1. **TOML spec files** — static definitions for known commands: subcommands, flags, descriptions, value hints. Loaded at startup from two locations (user files win over bundled):
   - Bundled: `completions-toml/` in the repo (shipped with the binary)
   - User: `~/.config/rusty/completions/`

2. **Filesystem fallback** — files and/or directories in the current working directory, respecting `args` type declared in the spec.

3. **History** (optional) — disabled by default. Set `fuzzy_history = true` in `~/.config/rusty/config.toml` under `[hints]` to re-enable ghost text and history entries in the popup.

## Popup behaviour

| Key | Action |
|-----|--------|
| `Tab` | Open popup (or accept ghost hint if no popup) |
| `↑` / `↓` | Navigate entries |
| `Enter` or `Tab` | Accept selected entry |
| `Esc` | Dismiss popup |
| Any other key | Dismiss popup and pass key through normally |

Modifier-only keys (`Shift`, `Cmd`, `Ctrl`, `Option`) do **not** dismiss the popup, so system shortcuts like `Shift+Cmd+4` work while a completion menu is visible.

## Bundled specs

| File | Command |
|------|---------|
| `ansible.toml` | `ansible` |
| `ansible-playbook.toml` | `ansible-playbook` |
| `ansible-vault.toml` | `ansible-vault` |
| `cd.toml` | `cd` |
| `docker.toml` | `docker` |
| `git.toml` | `git` |
| `grep.toml` | `grep` |
| `npm.toml` | `npm` |

## Writing a custom spec

Drop a `.toml` file in `~/.config/rusty/completions/`. It is loaded at next startup and overrides any bundled spec with the same command name.

### Full schema

```toml
# Required — must match the binary name exactly.
command     = "mycli"
description = "Optional one-line description of the command"

# args controls filesystem completions when the user is not typing a flag.
# "any"       — files and directories (default)
# "directory" — directories only (e.g. cd, pushd)
# "file"      — files only
# "none"      — no filesystem completions
args = "any"

# Top-level flags (available everywhere, not scoped to a subcommand).
[[flags]]
long        = "verbose"       # --verbose  (omit if no long form)
short       = "v"             # -v         (omit if no short form)
description = "Enable verbose output"
takes_value = false           # true if the flag takes an argument

[[flags]]
long        = "output"
short       = "o"
description = "Write output to file"
takes_value = true
value_hint  = "file"          # shown as --output=<file> in the popup

# Subcommands.
[[subcommands]]
name        = "build"
description = "Compile the project"

# Flags scoped to this subcommand only.
[[subcommands.flags]]
long        = "release"
description = "Build with optimisations"

[[subcommands.flags]]
long        = "target"
description = "Target triple for cross-compilation"
takes_value = true
value_hint  = "triple"

[[subcommands]]
name        = "test"
description = "Run the test suite"

[[subcommands.flags]]
long        = "nocapture"
description = "Show stdout from passing tests"
```

### Minimal example — a simple internal tool

```toml
command = "deploy"
description = "Deploy services to staging or production"

[[subcommands]]
name        = "staging"
description = "Deploy to staging"

[[subcommands]]
name        = "production"
description = "Deploy to production"

[[subcommands.flags]]
long        = "dry-run"
description = "Show what would be deployed without doing it"
```

Save to `~/.config/rusty/completions/deploy.toml`, restart rusty, and `deploy <Tab>` shows `staging` and `production`.

## How completions are triggered

The engine inspects the current line at the moment Tab is pressed:

- **First token, no space** → command name completions (PATH binaries + executables in CWD)
- **First token + space** → spec subcommands (if spec has any), otherwise filesystem fallback
- **Prefix starts with `-`** → flag completions from the spec
- **Explicit path token** (contains `/`, starts with `~` or `.`) → path completion (directories for `cd`-type commands, files+dirs otherwise)
- **No spec found, no path token** → filesystem fallback only

The Tab key syncs the engine's view of the current line from the actual terminal grid before computing completions, so the popup is always accurate regardless of how the line was edited (Ctrl+C, shell history navigation, paste, etc.).

## Configuration

```toml
# ~/.config/rusty/config.toml

[hints]
# Set to true to re-enable fish-style ghost text and history in the popup.
# Disabled by default.
fuzzy_history = false
```

## Generating a spec automatically

Rusty includes a built-in generator that runs a command's `--help` output and produces a TOML spec for it.

```
rusty completion-gen <command>
```

For example:

```
rusty completion-gen curl
rusty completion-gen devscan
rusty completion-gen kubectl
```

The generator:

1. Runs `<command> --help` (falls back to `-h` if needed).
2. Parses flag lines (`--flag`, `-f`) and subcommand sections (any section whose header contains the word "commands" or "subcommands", e.g. `Available Commands:`, `COMMANDS`).
3. Writes a `.toml` file to `~/.config/rusty/completions/<command>.toml`.
4. Hot-reloads the registry so the spec is active immediately **in the current session**.

> **You must restart Rusty** for a newly generated spec to appear in a fresh session. Hot-reload only applies to the session in which `completion-gen` was run.

Generated specs can be edited by hand at `~/.config/rusty/completions/<command>.toml` to add missing flags, fix descriptions, or set `args = "file"` / `args = "directory"`.

### What gets parsed

| Help output pattern | Result |
|---------------------|--------|
| `-v, --verbose   description` | Flag with short + long form |
| `--output=<file>   description` | Flag with value hint |
| `Available Commands:` / `Commands:` / `Subcommands:` header | Starts subcommand section |
| `  name    description` (indented, two spaces) | Subcommand entry |

Programs that dump their help to a pager (`less`, `more`) are handled automatically — the generator sets `PAGER=cat` before invoking the command.

## Adding a spec to the repo

1. Create `completions-toml/<command>.toml` following the schema above.
2. Use the existing files (`git.toml`, `docker.toml`) as reference.
3. The bundled directory is resolved at runtime relative to the binary, so no code changes are needed — the file is picked up automatically on next build.
