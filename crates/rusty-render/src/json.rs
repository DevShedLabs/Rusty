use crate::doc::{Color, RenderDoc, Span, Style};

pub fn render(source: &str, _width: usize) -> RenderDoc {
    match parse_and_render(source) {
        Ok(doc) => doc,
        Err(_)  => render_raw_with_error(source),
    }
}

fn parse_and_render(source: &str) -> Result<RenderDoc, ()> {
    let trimmed = source.trim();
    if trimmed.is_empty() { return Err(()); }
    // Must start with { or [ to be JSON.
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') { return Err(()); }

    let mut formatted = String::new();
    format_json(trimmed, &mut formatted).map_err(|_| ())?;

    let mut doc = RenderDoc::new();
    for line in formatted.lines() {
        doc.push_line(colorize_json_line(line));
    }
    Ok(doc)
}

fn format_json(s: &str, out: &mut String) -> Result<(), ()> {
    let mut indent      = 0usize;
    let mut in_string   = false;
    let mut escape_next = false;
    let mut chars       = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if escape_next { out.push(ch); escape_next = false; continue; }

        if in_string {
            if ch == '\\' { out.push(ch); escape_next = true; continue; }
            if ch == '"'  { in_string = false; }
            out.push(ch);
            continue;
        }

        match ch {
            '"' => { in_string = true; out.push(ch); }
            '{' | '[' => {
                out.push(ch);
                if matches!(chars.peek(), Some('}') | Some(']')) { continue; }
                indent += 1;
                out.push('\n');
                push_indent(out, indent);
            }
            '}' | ']' => {
                indent = indent.saturating_sub(1);
                out.push('\n');
                push_indent(out, indent);
                out.push(ch);
            }
            ',' => { out.push(','); out.push('\n'); push_indent(out, indent); }
            ':'  => { out.push_str(": "); }
            ' ' | '\t' | '\n' | '\r' => {}
            _ => { out.push(ch); }
        }
    }
    Ok(())
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth { out.push_str("  "); }
}

fn colorize_json_line(line: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    // Leading whitespace.
    let trimmed    = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    if indent_len > 0 {
        spans.push(Span::plain(&line[..indent_len]));
    }
    let rest = trimmed;

    // Object key: starts and ends with a quote, followed by `:`.
    if rest.starts_with('"') {
        // Find the closing quote of the key.
        if let Some(close) = find_closing_quote(rest, 1) {
            let key_part = &rest[..close + 1]; // includes both quotes
            let after    = &rest[close + 1..];
            if after.trim_start().starts_with(':') {
                spans.push(Span::colored(key_part, Color::BLUE));
                spans.push(Span::colored(":", Color::DIM));
                let value = after.trim_start().trim_start_matches(':').trim_start();
                spans.extend(colorize_value(value));
                return spans;
            }
        }
    }

    // Standalone value (array element, or bare line).
    spans.extend(colorize_value(rest));
    spans
}

/// Find the index of the closing `"` starting search from `from`, handling `\"`.
fn find_closing_quote(s: &str, from: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'\\' { i += 2; continue; }
        if bytes[i] == b'"'  { return Some(i); }
        i += 1;
    }
    None
}

fn colorize_value(s: &str) -> Vec<Span> {
    let s = s.trim_end_matches(',').trim();
    if s.is_empty() { return vec![]; }

    if s.starts_with('"') {
        return vec![Span::colored(s, Color::GREEN)];
    }
    if s == "true" || s == "false" {
        return vec![Span::colored(s, Color::YELLOW)];
    }
    if s == "null" {
        return vec![Span { text: s.to_owned(), style: Style { fg: Some(Color::DIM), italic: true, ..Style::default() } }];
    }
    if s.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return vec![Span::colored(s, Color::ORANGE)];
    }
    if matches!(s, "{" | "}" | "[" | "]" | "{}" | "[]") {
        return vec![Span::colored(s, Color::DIM)];
    }

    vec![Span::plain(s)]
}

fn render_raw_with_error(source: &str) -> RenderDoc {
    let mut doc = RenderDoc::new();
    doc.push_line(vec![
        Span::colored("⚠ ", Color::RED),
        Span::colored("Invalid JSON — showing raw output", Color::DIM),
    ]);
    doc.push_line(vec![]);
    for line in source.lines() {
        doc.push_line(vec![Span::plain(line)]);
    }
    doc
}
