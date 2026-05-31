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

/// Strip shell prompt noise from the end of captured PTY output.
///
/// The problem: the shell may write `}\r\n%~/path ❯ ` with no separation
/// between file content and prompt, or even `}%prompt` on one line.
/// We use content-type-aware truncation:
/// - JSON: truncate after the last `}` or `]` (whichever comes last).
/// - General: walk chars from the end, drop everything after the last
///   content character (non-prompt, non-whitespace).
pub fn strip_trailing_prompt(s: &str) -> &str {
    strip_trailing_prompt_generic(s)
}

pub fn strip_trailing_prompt_json(s: &str) -> &str {
    // Find the last closing brace or bracket — that's where JSON ends.
    if let Some(pos) = s.rfind(|c| c == '}' || c == ']') {
        return s[..=pos].trim_end_matches('\n').trim_end_matches('\r')
            // include the char itself
            .get(..pos + 1)
            .unwrap_or(s);
    }
    strip_trailing_prompt_generic(s)
}

fn strip_trailing_prompt_generic(s: &str) -> &str {
    // Prompt characters that never appear in file content.
    const PROMPT_CHARS: &[char] = &['❯', '→', '➜', '»', '›', '●', '✗', '✓'];

    // Find the byte position of the last prompt character.
    // Everything from there to the end is prompt noise.
    let mut cut = s.len();
    for (i, ch) in s.char_indices().rev() {
        if PROMPT_CHARS.contains(&ch) {
            cut = i;
        } else if !ch.is_whitespace() && ch != '%' && ch != '$' && ch != '#' {
            // Hit real content before another prompt char — stop scanning.
            break;
        }
    }

    // Only cut if we actually found a prompt char and it's near the end
    // (within 300 bytes — prompts are short).
    if cut < s.len() && s.len() - cut < 300 {
        return s[..cut].trim_end();
    }

    // Fallback: line-by-line strip of obvious prompt lines.
    let mut end = s.len();
    for line in s.lines().rev() {
        let t = line.trim();
        if t.is_empty() {
            end = end.saturating_sub(line.len() + 1);
            continue;
        }
        let has_prompt = t.chars().any(|c| PROMPT_CHARS.contains(&c))
            || t == "%" || t == "$" || t == "#";
        if has_prompt {
            end = end.saturating_sub(line.len() + 1);
            continue;
        }
        break;
    }
    s[..end].trim_end()
}
