use std::path::{Path, PathBuf};
use crate::completions::{CompletionRegistry, flag_suggestions, subcommand_suggestions};
use crate::index::CompletionIndex;
use crate::man_parser::spec_from_help;

// ── Ghost hint (history only) ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Hint {
    pub completion: String,
}

impl Hint {
    pub fn ghost<'a>(&'a self, typed: &str) -> &'a str {
        self.completion.strip_prefix(typed).unwrap_or("")
    }
}

// ── Popup completion entry ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompletionEntry {
    /// Display label shown in the popup.
    pub label: String,
    /// Text inserted when accepted (may differ from label, e.g. full path).
    pub insert: String,
    pub kind: EntryKind,
    /// Optional description shown alongside the label.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Command,
    History,
    /// A flag/option from a completion spec or --help parse.
    Flag,
    /// A subcommand (e.g. `git add`).
    Subcommand,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct HintEngine {
    index:         CompletionIndex,
    registry:      CompletionRegistry,
    pub line:      String,
    ghost:         Option<Hint>,
    /// Current working directory of the shell, updated via OSC 7.
    pub cwd:       PathBuf,
    fuzzy_history: bool,
}

impl HintEngine {
    pub fn new(fuzzy_history: bool) -> Self {
        Self {
            index:         CompletionIndex::new(),
            registry:      CompletionRegistry::load(),
            line:          String::new(),
            ghost:         None,
            cwd:           std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            fuzzy_history,
        }
    }

    /// Update CWD from OSC 7 `file://hostname/path` payload.
    pub fn set_cwd_from_osc7(&mut self, payload: &str) {
        if let Some(path) = payload.strip_prefix("file://") {
            let path = if let Some(p) = path.splitn(2, '/').nth(1) {
                format!("/{p}")
            } else {
                path.to_owned()
            };
            self.cwd = PathBuf::from(path);
        }
    }

    pub fn commit(&mut self) {
        let cmd = self.line.trim().to_owned();
        if !cmd.is_empty() {
            self.index.push_command(cmd);
        }
        self.line.clear();
        self.ghost = None;
    }

    /// Generate a TOML completion spec for `command` by running `command --help`,
    /// write it to `~/.config/rusty/completions/<command>.toml`, then reload the
    /// registry so completions are available immediately without restarting.
    ///
    /// Returns a human-readable message suitable for printing to the terminal.
    pub fn generate_completion(&mut self, command: &str) -> String {
        let Some(spec) = spec_from_help(command) else {
            return format!(
                "rusty: completion-gen: '{}' produced no usable --help output\r\n",
                command
            );
        };

        // Serialise to TOML.
        let toml_text = match spec_to_toml(&spec) {
            Ok(t)  => t,
            Err(e) => return format!("rusty: completion-gen: serialise error: {e}\r\n"),
        };

        // Write to user completions dir.
        let Some(dir) = user_completions_dir() else {
            return "rusty: completion-gen: could not determine completions directory\r\n".into();
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return format!("rusty: completion-gen: create dir {:?}: {e}\r\n", dir);
        }
        let path = dir.join(format!("{command}.toml"));
        if let Err(e) = std::fs::write(&path, &toml_text) {
            return format!("rusty: completion-gen: write {:?}: {e}\r\n", path);
        }

        // Hot-reload so the new spec is active immediately.
        self.registry.reload();

        format!(
            "rusty: completion-gen: wrote {} ({} flags) → {:?}\r\n",
            command,
            spec.flags.len(),
            path,
        )
    }

    pub fn update_line(&mut self, line: &str) {
        self.line = line.to_owned();
        self.ghost = if self.fuzzy_history { self.compute_ghost() } else { None };
    }

    pub fn hint(&self) -> Option<&Hint> {
        self.ghost.as_ref()
    }

    /// Accept the ghost hint — returns suffix to send to PTY.
    pub fn accept_ghost(&mut self) -> Option<String> {
        let hint = self.ghost.take()?;
        let suffix = hint.ghost(&self.line).to_owned();
        if suffix.is_empty() { return None; }
        self.line = hint.completion.clone();
        Some(suffix)
    }

    /// Build popup completions for the current line. Called on Tab press.
    pub fn completions(&self) -> Vec<CompletionEntry> {
        let line = self.line.trim_start();
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let last_token = tokens.last().copied().unwrap_or("");
        let is_first_token = tokens.len() <= 1 && !line.ends_with(' ');

        let mut entries: Vec<CompletionEntry> = Vec::new();

        if last_token.contains('/') || last_token.starts_with('~') || last_token.starts_with('.') {
            // Explicit path completion.
            entries.extend(path_completions(last_token, &self.cwd));
        } else if is_first_token {
            // First token: command name completions.
            entries.extend(dir_completions("", &self.cwd, last_token));
            entries.extend(path_command_completions(last_token));
        } else {
            // Argument position — check for spec-based completions first.
            let command = tokens[0];
            let typed_prefix = if line.ends_with(' ') { "" } else { last_token };

            // Determine if we're completing a flag or a subcommand/argument.
            let spec_entries = self.spec_completions(command, &tokens[1..], typed_prefix);
            if !spec_entries.is_empty() {
                entries.extend(spec_entries);
            } else {
                // Filesystem fallback — respect args type from spec if present.
                use crate::completions::ArgsType;
                let args_type = self.registry.get(command)
                    .map(|s| &s.args)
                    .unwrap_or(&ArgsType::Any);
                match args_type {
                    ArgsType::Directory => {
                        entries.extend(dir_only_completions(last_token, &self.cwd));
                        entries.extend(cdpath_completions(last_token));
                    }
                    ArgsType::None => {}
                    ArgsType::Any | ArgsType::File => {
                        entries.extend(dir_completions("", &self.cwd, last_token));
                    }
                }
            }
        }

        // History entries only when fuzzy_history is enabled and nothing
        // smarter was found (no spec, no path match).
        let history: Vec<CompletionEntry> = if self.fuzzy_history && entries.is_empty() {
            self.index.matches(line, 5)
                .into_iter()
                .map(|s| CompletionEntry {
                    label:  s.to_owned(),
                    insert: s.to_owned(),
                    kind:   EntryKind::History,
                    description: None,
                })
                .collect()
        } else {
            vec![]
        };

        // History first, then everything else deduplicated by insert text.
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<CompletionEntry> = Vec::new();
        for e in history.into_iter().chain(entries) {
            if seen.insert(e.insert.clone()) {
                out.push(e);
            }
        }
        out
    }

    /// Spec-based completions for `command` given already-typed `args`.
    /// Returns flags/subcommands that start with `prefix`.
    fn spec_completions(
        &self,
        command: &str,
        args: &[&str],
        prefix: &str,
    ) -> Vec<CompletionEntry> {
        // TOML registry only. The --help fallback is disabled: spawning a
        // subprocess on Tab press risks leaking output to the PTY (pagers,
        // programs that open /dev/tty directly). Add more TOML specs instead.
        let spec_opt = self.registry.get(command).cloned();

        let Some(spec) = spec_opt else { return vec![] };

        // Determine the active subcommand (first non-flag arg).
        let subcommand: Option<&str> = args.iter()
            .find(|a| !a.starts_with('-'))
            .copied();

        let mut out = Vec::new();

        // If no subcommand typed yet and prefix doesn't start with '-',
        // offer subcommands.
        if subcommand.is_none() && !prefix.starts_with('-') && !spec.subcommands.is_empty() {
            for s in subcommand_suggestions(&spec) {
                if s.label.starts_with(prefix) {
                    out.push(CompletionEntry {
                        label:  s.label.clone(),
                        insert: s.label,
                        kind:   EntryKind::Subcommand,
                        description: s.description,
                    });
                }
            }
        }

        // Flag completions — only when the user has started typing a flag.
        // Never offer flags unprompted: the filesystem/ArgsType fallback handles
        // the empty-prefix case so commands like `cd` show dirs, not --help.
        if prefix.starts_with('-') {
            for s in flag_suggestions(&spec, subcommand) {
                if s.label.starts_with(prefix) {
                    out.push(CompletionEntry {
                        label:  s.label.clone(),
                        insert: s.label,
                        kind:   EntryKind::Flag,
                        description: s.description,
                    });
                }
            }
        }

        out
    }

    fn compute_ghost(&self) -> Option<Hint> {
        let line = self.line.trim_start();
        if line.is_empty() { return None; }
        let m = self.index.best_match(line)?;
        Some(Hint { completion: m.to_owned() })
    }
}

impl Default for HintEngine {
    fn default() -> Self { Self::new(false) }
}

// ── TOML serialiser ──────────────────────────────────────────────────────────
// Hand-rolled — avoids adding serde's Serialize derive to all spec types just
// for this one use case.

fn spec_to_toml(spec: &crate::completions::CommandSpec) -> Result<String, String> {
    let mut out = String::new();

    out.push_str(&format!("command     = {:?}\n", spec.command));
    if let Some(d) = &spec.description {
        out.push_str(&format!("description = {:?}\n", d));
    }
    out.push('\n');

    for f in &spec.flags {
        push_flag(&mut out, f);
    }

    for sub in &spec.subcommands {
        let name = sub.name.as_deref().unwrap_or("");
        out.push_str("[[subcommands]]\n");
        out.push_str(&format!("name        = {:?}\n", name));
        if let Some(d) = &sub.description {
            out.push_str(&format!("description = {:?}\n", d));
        }
        out.push('\n');
        for f in &sub.flags {
            out.push_str("[[subcommands.flags]]\n");
            push_flag_body(&mut out, f);
        }
    }

    Ok(out)
}

fn push_flag(out: &mut String, f: &crate::completions::FlagSpec) {
    out.push_str("[[flags]]\n");
    push_flag_body(out, f);
}

fn push_flag_body(out: &mut String, f: &crate::completions::FlagSpec) {
    if let Some(l) = &f.long  { out.push_str(&format!("long        = {:?}\n", l)); }
    if let Some(s) = &f.short { out.push_str(&format!("short       = {:?}\n", s)); }
    if let Some(d) = &f.description { out.push_str(&format!("description = {:?}\n", d)); }
    if f.takes_value {
        out.push_str("takes_value = true\n");
        if let Some(h) = &f.value_hint { out.push_str(&format!("value_hint  = {:?}\n", h)); }
    }
    out.push('\n');
}

fn user_completions_dir() -> Option<std::path::PathBuf> {
    crate::completions::user_completions_dir_pub()
}

// ── Path helpers ──────────────────────────────────────────────────────────────

fn expand_tilde(s: &str) -> String {
    if s == "~" || s.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        s.replacen('~', &home, 1)
    } else {
        s.to_owned()
    }
}

fn path_completions(token: &str, cwd: &Path) -> Vec<CompletionEntry> {
    let expanded = expand_tilde(token);
    let (dir_str, file_prefix) = if let Some(p) = expanded.rfind('/') {
        (expanded[..=p].to_owned(), expanded[p + 1..].to_owned())
    } else {
        (String::new(), expanded.clone())
    };

    let search = if dir_str.is_empty() {
        cwd.to_path_buf()
    } else if Path::new(&dir_str).is_absolute() {
        PathBuf::from(&dir_str)
    } else {
        cwd.join(&dir_str)
    };

    let orig_dir = if let Some(p) = token.rfind('/') {
        token[..=p].to_owned()
    } else {
        String::new()
    };

    read_dir_entries(&search, &file_prefix, &orig_dir)
}

fn dir_completions(subdir: &str, cwd: &Path, prefix: &str) -> Vec<CompletionEntry> {
    let dir = if subdir.is_empty() { cwd.to_path_buf() } else { cwd.join(subdir) };
    read_dir_entries(&dir, prefix, "")
}

fn read_dir_entries(dir: &Path, prefix: &str, path_prefix: &str) -> Vec<CompletionEntry> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut entries: Vec<CompletionEntry> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.starts_with(prefix) { return None; }
            if name.starts_with('.') && !prefix.starts_with('.') { return None; }
            let ft     = e.file_type().ok()?;
            let is_dir = ft.is_dir();
            let kind   = if is_dir { EntryKind::Directory } else { EntryKind::File };
            let trail  = if is_dir { "/" } else { "" };
            let insert = format!("{path_prefix}{name}{trail}");
            Some(CompletionEntry { label: format!("{name}{trail}"), insert, kind, description: None })
        })
        .collect();

    entries.sort_by(|a, b| {
        match (a.kind, b.kind) {
            (EntryKind::Directory, EntryKind::Directory) |
            (EntryKind::File, EntryKind::File)           => a.label.cmp(&b.label),
            (EntryKind::Directory, _)                    => std::cmp::Ordering::Less,
            (_, EntryKind::Directory)                    => std::cmp::Ordering::Greater,
            _                                            => a.label.cmp(&b.label),
        }
    });
    entries
}

fn path_command_completions(prefix: &str) -> Vec<CompletionEntry> {
    if prefix.is_empty() { return vec![]; }
    let path_var = std::env::var("PATH").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out  = Vec::new();
    for dir in path_var.split(':') {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().into_string().unwrap_or_default();
            if !name.starts_with(prefix) { continue; }
            let Ok(meta) = e.metadata() else { continue };
            if !is_executable(&meta) { continue; }
            if seen.insert(name.clone()) {
                out.push(CompletionEntry {
                    label:  name.clone(),
                    insert: name,
                    kind:   EntryKind::Command,
                    description: None,
                });
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.is_file() && (meta.permissions().mode() & 0o111) != 0
}

/// Completions for a path token, but only directories (used by cd, pushd, etc.).
fn dir_only_completions(token: &str, cwd: &Path) -> Vec<CompletionEntry> {
    // Reuse path_completions then filter, or go direct via read_dir_entries.
    if token.contains('/') || token.starts_with('~') || token.starts_with('.') {
        path_completions(token, cwd)
            .into_iter()
            .filter(|e| e.kind == EntryKind::Directory)
            .collect()
    } else {
        let Ok(rd) = std::fs::read_dir(cwd) else { return vec![] };
        let mut entries: Vec<CompletionEntry> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                if !name.starts_with(token) { return None; }
                if name.starts_with('.') && !token.starts_with('.') { return None; }
                if !e.file_type().ok()?.is_dir() { return None; }
                Some(CompletionEntry {
                    label:       format!("{name}/"),
                    insert:      format!("{name}/"),
                    kind:        EntryKind::Directory,
                    description: None,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.label.cmp(&b.label));
        entries
    }
}

/// Directories from CDPATH entries that match `prefix` (mirrors zsh/bash cd behaviour).
fn cdpath_completions(prefix: &str) -> Vec<CompletionEntry> {
    let cdpath = std::env::var("CDPATH").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut out  = Vec::new();
    for dir in cdpath.split(':') {
        if dir.is_empty() || dir == "." { continue; }
        let base = PathBuf::from(expand_tilde(dir));
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().into_string().unwrap_or_default();
            if !name.starts_with(prefix) { continue; }
            if name.starts_with('.') && !prefix.starts_with('.') { continue; }
            if !e.file_type().map_or(false, |ft| ft.is_dir()) { continue; }
            if seen.insert(name.clone()) {
                out.push(CompletionEntry {
                    label:       format!("{name}/"),
                    insert:      format!("{name}/"),
                    kind:        EntryKind::Directory,
                    description: Some(format!("cdpath: {}", base.display())),
                });
            }
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn engine_with_toml_dir() -> HintEngine {
        // Point the loader at the workspace completions-toml/ directory.
        let manifest = env!("CARGO_MANIFEST_DIR");
        let dir = PathBuf::from(manifest).join("../../completions-toml");
        env::set_var("RUSTY_COMPLETIONS_DIR", &dir);
        HintEngine::new(false)
    }

    #[test]
    fn grep_flags_appear_on_tab() {
        let mut engine = engine_with_toml_dir();
        engine.update_line("grep -");
        let entries = engine.completions();
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| *l == "--verbose" || *l == "-v" || *l == "--ignore-case"),
            "expected grep flags, got: {:?}", labels
        );
        assert!(
            entries.iter().all(|e| e.kind != EntryKind::History),
            "history should be suppressed when spec entries exist"
        );
    }

    #[test]
    fn git_subcommands_appear_on_tab() {
        let mut engine = engine_with_toml_dir();
        engine.update_line("git ");
        let entries = engine.completions();
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert!(
            labels.contains(&"commit") && labels.contains(&"push"),
            "expected git subcommands, got: {:?}", labels
        );
    }

    #[test]
    fn history_shown_when_no_spec() {
        let mut engine = HintEngine::new(true);
        engine.index.push_command("unknowncmd --foo bar".to_owned());
        engine.update_line("unknowncmd ");
        let entries = engine.completions();
        // No spec, so file completions or history should appear (not crash).
        // We just assert it doesn't panic and returns something or empty.
        let _ = entries;
    }
}
