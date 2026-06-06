use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

use crate::completions::{CommandSpec, FlagSpec};

// ── Cache ─────────────────────────────────────────────────────────────────────
// We parse `--help` at most once per command per session.

static CACHE: Mutex<Option<HashMap<String, CommandSpec>>> = Mutex::new(None);

fn cache_get(command: &str) -> Option<CommandSpec> {
    let lock = CACHE.lock().ok()?;
    lock.as_ref()?.get(command).cloned()
}

fn cache_insert(spec: CommandSpec) {
    if let Ok(mut lock) = CACHE.lock() {
        lock.get_or_insert_with(HashMap::new)
            .insert(spec.command.clone(), spec);
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Return a `CommandSpec` derived from running `<command> --help`.
/// Returns `None` if the command doesn't exist or produces no flag output.
/// Results are cached in-process.
pub fn spec_from_help(command: &str) -> Option<CommandSpec> {
    if let Some(cached) = cache_get(command) {
        return Some(cached);
    }

    let output = run_help(command)?;
    let spec = parse_help_output(command, &output);
    if spec.flags.is_empty() && spec.subcommands.is_empty() {
        return None;
    }

    cache_insert(spec.clone());
    Some(spec)
}

// ── Runner ────────────────────────────────────────────────────────────────────

fn run_help(command: &str) -> Option<String> {
    use std::process::Stdio;
    use std::time::Duration;

    for flag in ["--help", "-h"] {
        // Fully detach from the PTY: null stdin so programs that invoke a
        // pager (git, man, less) don't inherit our terminal and dump output
        // there. Capture stdout + stderr explicitly.
        let mut child = Command::new(command)
            .arg(flag)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Some programs (git) respect NO_PAGER / GIT_TERMINAL_PROMPT.
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("MANPAGER", "cat")
            .env("NO_COLOR", "1")
            .spawn()
            .ok()?;

        // Wait with a timeout so a broken command can't stall Tab.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }

        let out = child.wait_with_output().ok()?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse the text output of `--help` into a `CommandSpec`.
///
/// Recognises patterns like:
///   -v, --verbose          Enable verbose output
///   --output=<file>        Write output to file
///   -o <file>              Write output to file
///   --flag                 Description text
fn parse_help_output(command: &str, text: &str) -> CommandSpec {
    let mut flags: Vec<FlagSpec> = Vec::new();
    let mut seen_long: std::collections::HashSet<String> = Default::default();

    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }

        if let Some(flag) = parse_flag_line(trimmed) {
            // Deduplicate by long flag name.
            let key = flag.long.clone().or_else(|| flag.short.clone()).unwrap_or_default();
            if !key.is_empty() && seen_long.insert(key) {
                flags.push(flag);
            }
        }
    }

    CommandSpec {
        command: command.to_owned(),
        description: None,
        flags,
        subcommands: vec![],
        args: crate::completions::ArgsType::Any,
    }
}

/// Try to extract a `FlagSpec` from a single help line.
///
/// Handles formats:
///   -v, --verbose [=VALUE]        description
///   --long-flag[=VALUE]           description
///   -s <VALUE>                    description
fn parse_flag_line(line: &str) -> Option<FlagSpec> {
    // Split into flag-part and description-part at two+ spaces.
    let (flags_part, description) = split_flag_desc(line);

    let mut short: Option<String> = None;
    let mut long: Option<String>  = None;
    let mut takes_value = false;
    let mut value_hint: Option<String> = None;

    // Tokenise on whitespace and commas.
    for raw in flags_part.split(|c: char| c == ',' || c == ' ' || c == '\t') {
        let token = raw.trim().trim_end_matches(',');
        if token.is_empty() { continue; }

        if let Some(rest) = token.strip_prefix("--") {
            // Long flag: --foo, --foo=VAL, --foo[=VAL]
            let (name, val) = split_long_flag(rest);
            if name.is_empty() { continue; }
            takes_value = takes_value || val.is_some();
            if val.is_some() && value_hint.is_none() {
                value_hint = val.map(clean_value_hint);
            }
            long = Some(name.to_owned());
        } else if let Some(rest) = token.strip_prefix('-') {
            // Short flag: -v, -o VAL
            let ch: &str = &rest[..rest.len().min(1)];
            if ch.chars().all(|c| c.is_ascii_alphanumeric()) {
                short = Some(ch.to_owned());
            }
        } else if takes_value && value_hint.is_none() {
            // Bare UPPER_CASE token right after a short flag → value placeholder.
            if token.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c == '-') {
                value_hint = Some(token.to_lowercase());
                takes_value = true;
            }
        }
    }

    if long.is_none() && short.is_none() {
        return None;
    }

    Some(FlagSpec {
        long,
        short,
        description: description.map(str::to_owned),
        takes_value,
        value_hint,
    })
}

/// Split `--foo=<VALUE>` or `--foo[=VALUE]` into `("foo", Some("VALUE"))`.
fn split_long_flag(rest: &str) -> (&str, Option<&str>) {
    // Remove optional bracket wrapping around `=VAL` part.
    let rest = rest.trim_end_matches(']');
    if let Some(eq) = rest.find('=') {
        (&rest[..eq], Some(&rest[eq + 1..]))
    } else if let Some(bracket) = rest.find('[') {
        (&rest[..bracket], None)
    } else {
        (rest, None)
    }
}

/// Clean angle-brackets and other wrappers from value hints.
fn clean_value_hint(s: &str) -> String {
    s.trim_matches(|c| c == '<' || c == '>' || c == '[' || c == ']')
        .to_lowercase()
}

/// Split a flag-line into (flags_part, Option<description>).
/// Description starts after 2+ consecutive spaces.
fn split_flag_desc(line: &str) -> (&str, Option<&str>) {
    // Find two or more spaces as delimiter.
    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b' ' {
            let start = i;
            while i < bytes.len() && bytes[i] == b' ' { i += 1; }
            if i - start >= 2 {
                let desc = line[i..].trim();
                return (&line[..start], if desc.is_empty() { None } else { Some(desc) });
            }
        } else {
            i += 1;
        }
    }
    (line, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_long_and_short() {
        let line = "  -v, --verbose          Enable verbose output";
        let f = parse_flag_line(line.trim()).unwrap();
        assert_eq!(f.short.as_deref(), Some("v"));
        assert_eq!(f.long.as_deref(), Some("verbose"));
        assert_eq!(f.description.as_deref(), Some("Enable verbose output"));
        assert!(!f.takes_value);
    }

    #[test]
    fn parses_long_with_value() {
        let line = "  --output=<file>         Write output to file";
        let f = parse_flag_line(line.trim()).unwrap();
        assert_eq!(f.long.as_deref(), Some("output"));
        assert!(f.takes_value);
        assert_eq!(f.value_hint.as_deref(), Some("file"));
    }

    #[test]
    fn split_desc_two_spaces() {
        let (flags, desc) = split_flag_desc("-v  description here");
        assert_eq!(flags, "-v");
        assert_eq!(desc, Some("description here"));
    }
}
