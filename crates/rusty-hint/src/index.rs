use std::collections::VecDeque;
use std::path::PathBuf;

const MAX_HISTORY: usize = 10_000;

pub struct CompletionIndex {
    /// Commands from history, newest last (deduped).
    history: VecDeque<String>,
}

impl CompletionIndex {
    pub fn new() -> Self {
        let mut idx = Self { history: VecDeque::with_capacity(1024) };
        idx.load_history_file();
        idx
    }

    pub fn push_command(&mut self, cmd: String) {
        let cmd = cmd.trim().to_owned();
        if cmd.is_empty() { return; }
        self.history.retain(|s| s != &cmd);
        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(cmd);
    }

    fn load_history_file(&mut self) {
        let home = std::env::var("HOME").unwrap_or_default();
        for file in [".zsh_history", ".bash_history"] {
            let path = PathBuf::from(&home).join(file);
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            // Read newest-first so push_command deduplication keeps the latest.
            for line in text.lines().rev() {
                // zsh extended history: `: timestamp:elapsed;command`
                let cmd = if let Some(rest) = line.strip_prefix(": ") {
                    rest.splitn(2, ';').nth(1).unwrap_or(line)
                } else {
                    line
                };
                self.push_command(cmd.to_owned());
                if self.history.len() >= MAX_HISTORY { break; }
            }
            break;
        }
    }

    /// Most recent history entry that starts with `prefix`.
    pub fn best_match(&self, prefix: &str) -> Option<&str> {
        if prefix.is_empty() { return None; }
        self.history.iter().rev().find_map(|s| {
            if s.starts_with(prefix) && s.len() > prefix.len() {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// All history entries that start with `prefix`, newest first, up to `limit`.
    pub fn matches(&self, prefix: &str, limit: usize) -> Vec<&str> {
        self.history.iter().rev()
            .filter(|s| s.starts_with(prefix) && s.len() > prefix.len())
            .map(|s| s.as_str())
            .take(limit)
            .collect()
    }
}

impl Default for CompletionIndex {
    fn default() -> Self { Self::new() }
}
