use std::path::{Path, PathBuf};
use crate::completions::{CompletionRegistry, flag_suggestions, subcommand_suggestions};
use crate::index::CompletionIndex;

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
    index:    CompletionIndex,
    registry: CompletionRegistry,
    pub line: String,
    ghost:    Option<Hint>,
    /// Current working directory of the shell, updated via OSC 7.
    pub cwd:  PathBuf,
}

impl HintEngine {
    pub fn new() -> Self {
        Self {
            index:    CompletionIndex::new(),
            registry: CompletionRegistry::load(),
            line:     String::new(),
            ghost:    None,
            cwd:      std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
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

    pub fn update_line(&mut self, line: &str) {
        self.line = line.to_owned();
        self.ghost = self.compute_ghost();
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
                // Fall back to file completions.
                entries.extend(dir_completions("", &self.cwd, last_token));
            }
        }

        // Show history only when we have no spec/flag/path entries — i.e. we
        // couldn't find anything smarter. This prevents history from burying
        // flag completions when a TOML spec (or --help parse) is available.
        let history: Vec<CompletionEntry> = if entries.is_empty() {
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

        // Flag completions — offered when prefix starts with '-' or no subcommands exist.
        if prefix.starts_with('-') || spec.subcommands.is_empty() {
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
    fn default() -> Self { Self::new() }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn engine_with_toml_dir() -> HintEngine {
        // Point the loader at the workspace completions-toml/ directory.
        let manifest = env!("CARGO_MANIFEST_DIR");
        let dir = PathBuf::from(manifest).join("../../completions-toml");
        env::set_var("RUSTY_COMPLETIONS_DIR", &dir);
        HintEngine::new()
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
        let mut engine = HintEngine::new();
        engine.index.push_command("unknowncmd --foo bar".to_owned());
        engine.update_line("unknowncmd ");
        let entries = engine.completions();
        // No spec, so file completions or history should appear (not crash).
        // We just assert it doesn't panic and returns something or empty.
        let _ = entries;
    }
}
