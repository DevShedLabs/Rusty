use crate::doc::{Color, RenderDoc, Span, Style};

pub fn render(source: &str, width: usize) -> RenderDoc {
    let mut doc = RenderDoc::new();

    for line in source.lines() {
        doc.push_line(render_line(line, width));
    }

    doc
}

fn render_line(line: &str, width: usize) -> Vec<Span> {
    // Headings
    if let Some(rest) = line.strip_prefix("### ") {
        return heading(rest, 3);
    }
    if let Some(rest) = line.strip_prefix("## ") {
        return heading(rest, 2);
    }
    if let Some(rest) = line.strip_prefix("# ") {
        return heading(rest, 1);
    }

    // Horizontal rule
    if line.trim_matches('-').is_empty() && line.len() >= 3
        || line.trim_matches('*').is_empty() && line.len() >= 3 {
        return vec![Span::colored("─".repeat(width.min(80)), Color::DIM)];
    }

    // Blockquote
    if let Some(rest) = line.strip_prefix("> ") {
        let mut spans = vec![Span::colored("▌ ", Color::CYAN)];
        spans.extend(inline(rest));
        return spans;
    }

    // Fenced code block marker
    if line.starts_with("```") {
        let lang = line.trim_start_matches('`').trim();
        if lang.is_empty() {
            return vec![Span::colored("  ···", Color::DIM)];
        } else {
            return vec![
                Span::colored("  ···", Color::DIM),
                Span::colored(format!(" {lang}"), Color::MAGENTA),
            ];
        }
    }

    // Unordered list
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        let mut spans = vec![Span::colored("  • ", Color::BLUE)];
        spans.extend(inline(rest));
        return spans;
    }

    // Ordered list (e.g. "1. ")
    if let Some(pos) = line.find(". ") {
        let num = &line[..pos];
        if num.chars().all(|c| c.is_ascii_digit()) {
            let mut spans = vec![
                Span::colored(format!("  {}. ", num), Color::YELLOW),
            ];
            spans.extend(inline(&line[pos + 2..]));
            return spans;
        }
    }

    // Empty line
    if line.trim().is_empty() {
        return vec![Span::plain("")];
    }

    // Normal paragraph — inline formatting only.
    inline(line)
}

fn heading(text: &str, level: u8) -> Vec<Span> {
    let (prefix, color) = match level {
        1 => ("  ", Color::BLUE),
        2 => ("  ", Color::CYAN),
        _ => ("  ", Color::GREEN),
    };
    let size_hint = match level {
        1 => "█ ",
        2 => "▌ ",
        _ => "░ ",
    };
    vec![
        Span::colored(format!("{prefix}{size_hint}"), color),
        Span { text: text.to_owned(), style: Style { fg: Some(Color::WHITE), bold: true, ..Style::default() } },
    ]
}

/// Parse inline Markdown: **bold**, *italic*, `code`, [text](url).
fn inline(s: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut rest = s;

    while !rest.is_empty() {
        // `code`
        if let Some(idx) = rest.find('`') {
            if idx > 0 { spans.push(plain_text(&rest[..idx])); }
            rest = &rest[idx + 1..];
            if let Some(end) = rest.find('`') {
                spans.push(Span {
                    text:  rest[..end].to_owned(),
                    style: Style { fg: Some(Color::ORANGE), bg: Some(Color::BG_CODE), ..Style::default() },
                });
                rest = &rest[end + 1..];
            }
            continue;
        }

        // **bold**
        if let Some(idx) = rest.find("**") {
            if idx > 0 { spans.push(plain_text(&rest[..idx])); }
            rest = &rest[idx + 2..];
            if let Some(end) = rest.find("**") {
                spans.push(Span {
                    text:  rest[..end].to_owned(),
                    style: Style { fg: Some(Color::WHITE), bold: true, ..Style::default() },
                });
                rest = &rest[end + 2..];
            }
            continue;
        }

        // *italic*
        if let Some(idx) = rest.find('*') {
            if idx > 0 { spans.push(plain_text(&rest[..idx])); }
            rest = &rest[idx + 1..];
            if let Some(end) = rest.find('*') {
                spans.push(Span {
                    text:  rest[..end].to_owned(),
                    style: Style { fg: Some(Color::FG), italic: true, ..Style::default() },
                });
                rest = &rest[end + 1..];
            }
            continue;
        }

        // [text](url) — show text in blue, url dimmed
        if let Some(idx) = rest.find('[') {
            if idx > 0 { spans.push(plain_text(&rest[..idx])); }
            rest = &rest[idx + 1..];
            if let Some(text_end) = rest.find("](") {
                let link_text = rest[..text_end].to_owned();
                rest = &rest[text_end + 2..];
                if let Some(url_end) = rest.find(')') {
                    let url = rest[..url_end].to_owned();
                    spans.push(Span::colored(link_text, Color::BLUE));
                    spans.push(Span::colored(format!(" ({url})"), Color::DIM));
                    rest = &rest[url_end + 1..];
                    continue;
                }
            }
        }

        // No more markers — rest is plain text.
        spans.push(plain_text(rest));
        break;
    }

    spans
}

fn plain_text(s: &str) -> Span {
    Span { text: s.to_owned(), style: Style { fg: Some(Color::FG), ..Style::default() } }
}
