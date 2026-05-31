use std::path::{Path, PathBuf};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File,
    Command,
    History,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct HintEngine {
    index: CompletionIndex,
    pub line: String,
    ghost:    Option<Hint>,
    /// Current working directory of the shell, updated via OSC 7.
    pub cwd: PathBuf,
}

impl HintEngine {
    pub fn new() -> Self {
        Self {
            index: CompletionIndex::new(),
            line:  String::new(),
            ghost: None,
            cwd:   std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Update CWD from OSC 7 `file://hostname/path` payload.
    pub fn set_cwd_from_osc7(&mut self, payload: &str) {
        // Strip `file://hostname` prefix.
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
    /// Returns entries sorted: directories first, then files, then commands.
    pub fn completions(&self) -> Vec<CompletionEntry> {
        let line = self.line.trim_start();
        let last_token = line.split_whitespace().last().unwrap_or("");
        let is_first_token = !line.contains(' ') || line.ends_with(' ');

        let mut entries: Vec<CompletionEntry> = Vec::new();

        if last_token.contains('/') || last_token.starts_with('~') || last_token.starts_with('.') {
            // Path completion.
            entries.extend(path_completions(last_token, &self.cwd));
        } else if is_first_token {
            // Command: current dir executables + PATH binaries.
            entries.extend(dir_completions("", &self.cwd, last_token));
            entries.extend(path_command_completions(last_token));
        } else {
            // Argument: files in cwd.
            entries.extend(dir_completions("", &self.cwd, last_token));
        }

        // History matches always shown at top.
        let history: Vec<CompletionEntry> = self.index.matches(line, 5)
            .into_iter()
            .map(|s| CompletionEntry {
                label:  s.to_owned(),
                insert: s.to_owned(),
                kind:   EntryKind::History,
            })
            .collect();

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

/// Completions for a partial path token (contains / or starts with ~ or .).
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

    // Recover original (un-expanded) dir prefix for display.
    let orig_dir = if let Some(p) = token.rfind('/') {
        token[..=p].to_owned()
    } else {
        String::new()
    };

    read_dir_entries(&search, &file_prefix, &orig_dir)
}

/// Completions from a specific directory with a prefix filter.
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
            // Skip hidden files unless prefix starts with dot.
            if name.starts_with('.') && !prefix.starts_with('.') { return None; }
            let ft    = e.file_type().ok()?;
            let is_dir = ft.is_dir();
            let kind  = if is_dir { EntryKind::Directory } else { EntryKind::File };
            let trail = if is_dir { "/" } else { "" };
            let insert = format!("{path_prefix}{name}{trail}");
            Some(CompletionEntry { label: format!("{name}{trail}"), insert, kind })
        })
        .collect();

    entries.sort_by(|a, b| {
        // Directories first, then alphabetical.
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

/// Executables from PATH directories matching prefix.
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
