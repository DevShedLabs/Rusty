use std::collections::VecDeque;

const MAX_HISTORY: usize = 10_000;

/// In-memory completion index: command history + path completions.
pub struct CompletionIndex {
    history: VecDeque<String>,
}

impl CompletionIndex {
    pub fn new() -> Self {
        Self { history: VecDeque::with_capacity(256) }
    }

    pub fn push_command(&mut self, cmd: String) {
        if self.history.len() == MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(cmd);
    }

    /// Return the most recent history entry that starts with `prefix`.
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
}

impl Default for CompletionIndex {
    fn default() -> Self { Self::new() }
}
