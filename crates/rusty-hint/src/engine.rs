use crate::index::CompletionIndex;

/// A suggestion to render as phantom (grayed-out) text after the cursor.
#[derive(Debug, Clone)]
pub struct Hint {
    /// The full completed string. Render only the suffix after what's typed.
    pub completion: String,
}

pub struct HintEngine {
    index: CompletionIndex,
}

impl HintEngine {
    pub fn new() -> Self {
        Self { index: CompletionIndex::new() }
    }

    pub fn record(&mut self, completed_command: String) {
        self.index.push_command(completed_command);
    }

    /// Given the current line buffer, return a hint if one exists.
    pub fn suggest(&self, line: &str) -> Option<Hint> {
        let completion = self.index.best_match(line)?;
        Some(Hint { completion: completion.to_owned() })
    }
}

impl Default for HintEngine {
    fn default() -> Self { Self::new() }
}
