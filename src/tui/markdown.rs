use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use std::sync::LazyLock;

// ── Styles (matching CLI's changelog_skin in src/cli/styles.rs) ─────

fn header_style() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}

fn bullet_style() -> Style {
    Style::default().fg(Color::Magenta)
}

fn bold_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn code_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Rgb(50, 40, 25))
}

fn url_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

// ── Inline span parsing ─────────────────────────────────────────────

// The `[label](url)` alternative must precede the bare-URL branch: alternation
// is first-match, so the bare pattern would otherwise claim the inner URL and
// leave `[label](` + `)` as literal text.
static INLINE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\[([^\]]+)\]\(([^)\s]+)\)|\*\*(.+?)\*\*|`([^`]+)`|(https?://[^\s)\]>]+)")
        .unwrap()
});

fn parse_inline_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut last_end = 0;

    for cap in INLINE_RE.captures_iter(text) {
        let whole = cap.get(0).unwrap();
        if whole.start() > last_end {
            spans.push(Span::raw(text[last_end..whole.start()].to_owned()));
        }
        if let Some(label) = cap.get(1) {
            // Markdown link: render the label only (URL dropped — terminal
            // links aren't clickable and the raw URL is pure noise in
            // link-dense changelogs like Zed's).
            spans.push(Span::styled(label.as_str().to_owned(), url_style()));
        } else if let Some(bold) = cap.get(3) {
            spans.push(Span::styled(bold.as_str().to_owned(), bold_style()));
        } else if let Some(code) = cap.get(4) {
            spans.push(Span::styled(code.as_str().to_owned(), code_style()));
        } else if let Some(url) = cap.get(5) {
            spans.push(Span::styled(url.as_str().to_owned(), url_style()));
        }
        last_end = whole.end();
    }

    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_owned()));
    }

    // If nothing matched, return the whole string as a single span
    if spans.is_empty() {
        spans.push(Span::raw(text.to_owned()));
    }

    spans
}

// ── Highlight post-processing ────────────────────────────────────────

fn highlight_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

/// Split spans to highlight case-insensitive occurrences of `query`.
fn highlight_spans(spans: Vec<Span<'static>>, query: &str) -> Vec<Span<'static>> {
    if query.is_empty() {
        return spans;
    }
    let query_lower = query.to_lowercase();
    let mut result = Vec::new();
    for span in spans {
        let text = span.content.as_ref();
        let text_lower = text.to_lowercase();
        let mut last = 0;
        for (start, _) in text_lower.match_indices(&query_lower) {
            if start > last {
                result.push(Span::styled(text[last..start].to_owned(), span.style));
            }
            result.push(Span::styled(
                text[start..start + query.len()].to_owned(),
                highlight_style(),
            ));
            last = start + query.len();
        }
        if last < text.len() {
            result.push(Span::styled(text[last..].to_owned(), span.style));
        } else if last == 0 {
            result.push(span);
        }
    }
    result
}

// ── Public API ──────────────────────────────────────────────────────

/// Convert a raw markdown changelog body into styled ratatui lines.
pub fn changelog_to_lines(body: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        // ## or ### headers → magenta bold
        if let Some(rest) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
        {
            lines.push(Line::from(Span::styled(rest.to_owned(), header_style())));
            continue;
        }

        // # headers → skip (wrapper headers like "What's Changed")
        if trimmed.starts_with('#') && trimmed.chars().nth(1) == Some(' ') {
            continue;
        }

        // Bullet points
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let mut spans = vec![Span::styled("  \u{2022} ", bullet_style())];
            spans.extend(parse_inline_spans(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // Plain text → indented with inline parsing
        let mut spans = vec![Span::raw("  ".to_owned())];
        spans.extend(parse_inline_spans(trimmed));
        lines.push(Line::from(spans));
    }

    lines
}

/// Convert a raw markdown changelog body into styled lines with search highlighting.
pub fn changelog_to_lines_highlighted(body: &str, query: &str) -> Vec<Line<'static>> {
    changelog_to_lines(body)
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line.spans;
            Line::from(highlight_spans(spans, query))
        })
        .collect()
}

/// Check if a line contains a case-insensitive match for the query.
pub fn line_contains_match(line: &Line, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    let query_lower = query.to_lowercase();
    line.spans
        .iter()
        .any(|span| span.content.to_lowercase().contains(&query_lower))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_magenta_bold() {
        let lines = changelog_to_lines("## Bug Fixes");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Bug Fixes");
        assert_eq!(lines[0].spans[0].style, header_style());
    }

    #[test]
    fn h3_header_is_magenta_bold() {
        let lines = changelog_to_lines("### New Features");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "New Features");
        assert_eq!(lines[0].spans[0].style, header_style());
    }

    #[test]
    fn h1_header_is_skipped() {
        let lines = changelog_to_lines("# What's Changed");
        assert!(lines.is_empty());
    }

    #[test]
    fn bullet_with_inline_code() {
        let lines = changelog_to_lines("- Fixed `crash` on startup");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "  \u{2022} ");
        assert_eq!(lines[0].spans[0].style, bullet_style());
        assert_eq!(lines[0].spans[1].content, "Fixed ");
        assert_eq!(lines[0].spans[2].content, "crash");
        assert_eq!(lines[0].spans[2].style, code_style());
        assert_eq!(lines[0].spans[3].content, " on startup");
    }

    #[test]
    fn bold_text() {
        let lines = changelog_to_lines("- **Full Changelog**: see repo");
        assert_eq!(lines[0].spans[1].content, "Full Changelog");
        assert_eq!(lines[0].spans[1].style, bold_style());
    }

    #[test]
    fn markdown_link_renders_label_only() {
        let lines =
            changelog_to_lines("- Improved picker ([#61215](https://github.com/z/z/pull/61215))");
        let contents: Vec<&str> = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Label survives, styled as a URL; the raw URL and []() syntax are gone.
        let label = lines[0]
            .spans
            .iter()
            .find(|s| s.content == "#61215")
            .expect("link label span");
        assert_eq!(label.style, url_style());
        assert!(!contents.iter().any(|c| c.contains("https://")));
        assert!(!contents.iter().any(|c| c.contains("](")));
    }

    #[test]
    fn markdown_link_mixed_with_bare_url() {
        let lines = changelog_to_lines("- [docs](https://a.io/d) and https://b.io/x");
        let contents: Vec<&str> = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(contents.contains(&"docs"));
        assert!(contents.contains(&"https://b.io/x"));
        assert!(!contents.iter().any(|c| c.contains("a.io")));
    }

    #[test]
    fn url_is_cyan_underlined() {
        let lines = changelog_to_lines("- See https://github.com/foo/bar");
        let url_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.starts_with("https://"))
            .unwrap();
        assert_eq!(url_span.style, url_style());
    }

    #[test]
    fn blank_lines_preserved() {
        let lines = changelog_to_lines("## Header\n\n- item");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].spans.len(), 0); // empty line
    }

    #[test]
    fn plain_text_indented() {
        let lines = changelog_to_lines("Some plain text");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "  ");
        assert_eq!(lines[0].spans[1].content, "Some plain text");
    }

    #[test]
    fn highlight_splits_spans() {
        let lines = changelog_to_lines_highlighted("- Fixed crash on startup", "crash");
        let spans = &lines[0].spans;
        // bullet, "Fixed ", "crash" (highlighted), " on startup"
        let highlighted = spans.iter().find(|s| s.content == "crash").unwrap();
        assert_eq!(highlighted.style, highlight_style());
    }

    #[test]
    fn highlight_case_insensitive() {
        let lines = changelog_to_lines_highlighted("- Added STREAMING support", "streaming");
        let highlighted = lines[0]
            .spans
            .iter()
            .find(|s| s.content == "STREAMING")
            .unwrap();
        assert_eq!(highlighted.style, highlight_style());
    }

    #[test]
    fn highlight_preserves_no_match_lines() {
        let lines = changelog_to_lines_highlighted("## Bug Fixes\n- Fixed crash", "crash");
        // Header line should be unchanged
        assert_eq!(lines[0].spans[0].style, header_style());
        // Bullet line should have highlight
        assert!(lines[1].spans.iter().any(|s| s.style == highlight_style()));
    }

    #[test]
    fn line_contains_match_works() {
        let lines = changelog_to_lines("- Fixed crash on startup");
        assert!(line_contains_match(&lines[0], "crash"));
        assert!(!line_contains_match(&lines[0], "missing"));
    }

    #[test]
    fn mixed_inline_formatting() {
        let lines = changelog_to_lines("- Added **new** `feature` at https://example.com");
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "  \u{2022} "); // bullet
        assert_eq!(spans[1].content, "Added "); // plain
        assert_eq!(spans[2].content, "new"); // bold
        assert_eq!(spans[2].style, bold_style());
        assert_eq!(spans[3].content, " "); // plain
        assert_eq!(spans[4].content, "feature"); // code
        assert_eq!(spans[4].style, code_style());
        assert_eq!(spans[5].content, " at "); // plain
        assert_eq!(spans[6].content, "https://example.com"); // url
        assert_eq!(spans[6].style, url_style());
    }
}
