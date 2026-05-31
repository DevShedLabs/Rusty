use std::path::Path;

/// What kind of native rendering to apply to the next command's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderTrigger {
    Markdown,
    Json,
}

/// Commands that output file content we can detect with certainty.
const VIEW_COMMANDS: &[&str] = &["cat", "bat", "less", "more", "head", "tail"];

/// Inspect the committed command line and return a trigger if we should
/// intercept and natively render the output.
pub fn detect_trigger(line: &str) -> Option<RenderTrigger> {
    // Strip prompt prefix — e.g. "❯ " or "$ " or "% " before the command.
    // Find the first token that looks like a command (ASCII alphanumeric or path).
    let line = line.trim();
    let line = match line.find(|c: char| c.is_ascii_alphanumeric() || c == '/' || c == '.') {
        Some(pos) => &line[pos..],
        None      => line,
    };
    let mut tokens = line.split_whitespace();
    let cmd = tokens.next()?;

    // Strip path prefix (e.g. `/bin/cat` → `cat`).
    let cmd = Path::new(cmd).file_name()?.to_str()?;

    if !VIEW_COMMANDS.contains(&cmd) {
        return None;
    }

    // Find the last argument that looks like a file path.
    let file_arg = tokens
        .filter(|t| !t.starts_with('-')) // skip flags
        .last()?;

    let ext = Path::new(file_arg).extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "md" | "markdown" | "mdx" => Some(RenderTrigger::Markdown),
        "json" | "jsonc"          => Some(RenderTrigger::Json),
        _                         => None,
    }
}
