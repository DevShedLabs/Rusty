pub mod index;
pub mod engine;
pub mod completions;
pub mod man_parser;

pub use engine::{CompletionEntry, EntryKind, Hint, HintEngine};
