// src/agents/changelog_parser.rs

use comrak::nodes::NodeValue;
use comrak::{parse_document, Arena, Options};

/// A normalized block in a parsed changelog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangelogBlock {
    /// A section heading (from ## or ### headers).
    Heading(String),
    /// A bullet item (from list items). May contain nested bullets.
    Bullet(String),
    /// A prose paragraph.
    Paragraph(String),
}

/// Structured changelog: a flat list of blocks representing the release body.
#[derive(Debug, Clone)]
pub struct Changelog {
    pub blocks: Vec<ChangelogBlock>,
}

/// Headers that wrap the actual content without adding structure.
const SKIP_HEADERS: &[&str] = &["What's Changed", "Changelog", "Full Changelog"];

fn is_skip_header(name: &str) -> bool {
    SKIP_HEADERS.iter().any(|h| name.starts_with(h))
}

/// Extract plain text content from an AST node and its inline children.
fn collect_text<'a>(node: &'a comrak::nodes::AstNode<'a>) -> String {
    let mut text = String::new();
    for child in node.children() {
        match &child.data.borrow().value {
            NodeValue::Text(t) => text.push_str(t),
            NodeValue::Code(c) => {
                text.push('`');
                text.push_str(&c.literal);
                text.push('`');
            }
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push(' '),
            NodeValue::Emph => {
                let inner = collect_text(child);
                text.push_str(&inner);
            }
            NodeValue::Strong => {
                let inner = collect_text(child);
                text.push_str(&inner);
            }
            NodeValue::Link(link) => {
                let label = collect_text(child);
                if label.is_empty() {
                    text.push_str(&link.url);
                } else {
                    text.push_str(&label);
                }
            }
            NodeValue::Strikethrough => {
                let inner = collect_text(child);
                text.push_str(&inner);
            }
            _ => {
                // Recurse into other inline nodes
                let inner = collect_text(child);
                text.push_str(&inner);
            }
        }
    }
    text
}

/// Collect bullet items from a list node, flattening nested lists.
fn collect_list_items<'a>(node: &'a comrak::nodes::AstNode<'a>, items: &mut Vec<String>) {
    for child in node.children() {
        if let NodeValue::Item(_) = &child.data.borrow().value {
            let mut item_text = String::new();
            let mut has_nested = false;
            for item_child in child.children() {
                match &item_child.data.borrow().value {
                    NodeValue::Paragraph => {
                        let para = collect_text(item_child);
                        if !para.is_empty() {
                            if !item_text.is_empty() {
                                item_text.push(' ');
                            }
                            item_text.push_str(&para);
                        }
                    }
                    NodeValue::List(_) => {
                        // Push the parent item first, then recurse
                        has_nested = true;
                        if !item_text.is_empty() {
                            items.push(item_text.clone());
                            item_text.clear();
                        }
                        collect_list_items(item_child, items);
                    }
                    _ => {
                        let text = collect_text(item_child);
                        if !text.is_empty() {
                            if !item_text.is_empty() {
                                item_text.push(' ');
                            }
                            item_text.push_str(&text);
                        }
                    }
                }
            }
            if !item_text.is_empty() || !has_nested {
                items.push(item_text);
            }
        }
    }
}

/// Parse a GitHub release body into a normalized changelog IR using comrak.
pub fn parse_changelog(body: &str) -> Changelog {
    let arena = Arena::new();
    let opts = Options::default();
    let root = parse_document(&arena, body, &opts);

    let mut blocks = Vec::new();

    for node in root.children() {
        match &node.data.borrow().value {
            NodeValue::Heading(heading) if heading.level >= 2 => {
                let text = collect_text(node);
                let trimmed = text.trim().to_string();
                if !is_skip_header(&trimmed) && !trimmed.is_empty() {
                    blocks.push(ChangelogBlock::Heading(trimmed));
                }
            }
            NodeValue::List(_) => {
                let mut items = Vec::new();
                collect_list_items(node, &mut items);
                for item in items {
                    blocks.push(ChangelogBlock::Bullet(item));
                }
            }
            NodeValue::Paragraph => {
                let text = collect_text(node);
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    blocks.push(ChangelogBlock::Paragraph(trimmed));
                }
            }
            _ => {}
        }
    }

    Changelog { blocks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Section {
        name: String,
        changes: Vec<String>,
    }

    /// Convert IR back to legacy format for backward-compatibility tests.
    fn parse_release_body(body: &str) -> (Vec<Section>, Vec<String>) {
        let changelog = parse_changelog(body);
        let mut sections: Vec<Section> = Vec::new();
        let mut ungrouped: Vec<String> = Vec::new();
        let mut current_section: Option<Section> = None;

        for block in changelog.blocks {
            match block {
                ChangelogBlock::Heading(name) => {
                    if let Some(sec) = current_section.take() {
                        if !sec.changes.is_empty() {
                            sections.push(sec);
                        }
                    }
                    current_section = Some(Section {
                        name,
                        changes: Vec::new(),
                    });
                }
                ChangelogBlock::Bullet(text) | ChangelogBlock::Paragraph(text) => {
                    if let Some(ref mut sec) = current_section {
                        sec.changes.push(text);
                    } else {
                        ungrouped.push(text);
                    }
                }
            }
        }

        if let Some(sec) = current_section {
            if !sec.changes.is_empty() {
                sections.push(sec);
            }
        }

        (sections, ungrouped)
    }

    // --- Legacy API tests (must not regress) ---

    #[test]
    fn sectioned_changelog() {
        let body = "\
## Bug Fixes
- Fixed crash on startup
- Fixed memory leak

## Features
- Added dark mode
- Added export to CSV";

        let (sections, ungrouped) = parse_release_body(body);
        assert!(ungrouped.is_empty());
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Bug Fixes");
        assert_eq!(sections[0].changes.len(), 2);
        assert_eq!(sections[0].changes[0], "Fixed crash on startup");
        assert_eq!(sections[0].changes[1], "Fixed memory leak");
        assert_eq!(sections[1].name, "Features");
        assert_eq!(sections[1].changes.len(), 2);
        assert_eq!(sections[1].changes[0], "Added dark mode");
        assert_eq!(sections[1].changes[1], "Added export to CSV");
    }

    #[test]
    fn ungrouped_changes() {
        let body = "\
- Fixed crash on startup
- Added dark mode
- Improved performance";

        let (sections, ungrouped) = parse_release_body(body);
        assert!(sections.is_empty());
        assert_eq!(ungrouped.len(), 3);
        assert_eq!(ungrouped[0], "Fixed crash on startup");
        assert_eq!(ungrouped[1], "Added dark mode");
        assert_eq!(ungrouped[2], "Improved performance");
    }

    #[test]
    fn skip_whats_changed_header() {
        let body = "\
## What's Changed
- Fixed crash on startup
- Added dark mode";

        let (sections, ungrouped) = parse_release_body(body);
        assert!(sections.is_empty());
        assert_eq!(ungrouped.len(), 2);
        assert_eq!(ungrouped[0], "Fixed crash on startup");
        assert_eq!(ungrouped[1], "Added dark mode");
    }

    #[test]
    fn asterisk_bullets() {
        let body = "\
## Changes
* Fixed crash on startup
* Added dark mode";

        let (sections, ungrouped) = parse_release_body(body);
        assert!(ungrouped.is_empty());
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].changes.len(), 2);
        assert_eq!(sections[0].changes[0], "Fixed crash on startup");
        assert_eq!(sections[0].changes[1], "Added dark mode");
    }

    #[test]
    fn empty_body() {
        let (sections, ungrouped) = parse_release_body("");
        assert!(sections.is_empty());
        assert!(ungrouped.is_empty());
    }

    #[test]
    fn mixed_sections_and_ungrouped() {
        let body = "\
- Ungrouped item 1
- Ungrouped item 2

## Bug Fixes
- Fixed crash on startup

## Features
- Added dark mode";

        let (sections, ungrouped) = parse_release_body(body);
        assert_eq!(ungrouped.len(), 2);
        assert_eq!(ungrouped[0], "Ungrouped item 1");
        assert_eq!(ungrouped[1], "Ungrouped item 2");
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Bug Fixes");
        assert_eq!(sections[0].changes[0], "Fixed crash on startup");
        assert_eq!(sections[1].name, "Features");
        assert_eq!(sections[1].changes[0], "Added dark mode");
    }

    // --- New IR tests ---

    #[test]
    fn ir_preserves_paragraphs() {
        let body = "\
Some introductory text about this release.

## Changes
- Fixed a bug";

        let changelog = parse_changelog(body);
        assert_eq!(changelog.blocks.len(), 3);
        assert_eq!(
            changelog.blocks[0],
            ChangelogBlock::Paragraph("Some introductory text about this release.".to_string())
        );
        assert_eq!(
            changelog.blocks[1],
            ChangelogBlock::Heading("Changes".to_string())
        );
        assert_eq!(
            changelog.blocks[2],
            ChangelogBlock::Bullet("Fixed a bug".to_string())
        );
    }

    #[test]
    fn ir_handles_inline_formatting() {
        let body = "- Fixed **crash** in `main()` function";

        let changelog = parse_changelog(body);
        assert_eq!(changelog.blocks.len(), 1);
        assert_eq!(
            changelog.blocks[0],
            ChangelogBlock::Bullet("Fixed crash in `main()` function".to_string())
        );
    }

    #[test]
    fn ir_handles_links() {
        let body = "- See [the docs](https://example.com) for details";

        let changelog = parse_changelog(body);
        assert_eq!(changelog.blocks.len(), 1);
        assert_eq!(
            changelog.blocks[0],
            ChangelogBlock::Bullet("See the docs for details".to_string())
        );
    }

    #[test]
    fn ir_flattens_nested_lists() {
        let body = "\
- Parent item
  - Child item 1
  - Child item 2";

        let changelog = parse_changelog(body);
        assert_eq!(changelog.blocks.len(), 3);
        assert_eq!(
            changelog.blocks[0],
            ChangelogBlock::Bullet("Parent item".to_string())
        );
        assert_eq!(
            changelog.blocks[1],
            ChangelogBlock::Bullet("Child item 1".to_string())
        );
        assert_eq!(
            changelog.blocks[2],
            ChangelogBlock::Bullet("Child item 2".to_string())
        );
    }

    #[test]
    fn ir_skips_wrapper_headers() {
        let body = "\
## What's Changed
- Item 1

## Full Changelog
https://github.com/example/compare/v1...v2";

        let changelog = parse_changelog(body);
        // "What's Changed" and "Full Changelog" headers should be skipped
        assert!(changelog
            .blocks
            .iter()
            .all(|b| !matches!(b, ChangelogBlock::Heading(_))));
        assert_eq!(
            changelog.blocks[0],
            ChangelogBlock::Bullet("Item 1".to_string())
        );
    }

    #[test]
    fn ir_whitespace_only_body() {
        let changelog = parse_changelog("   \n\n  ");
        assert!(changelog.blocks.is_empty());
    }

    #[test]
    fn ir_mixed_prose_and_sections() {
        let body = "\
This release includes important updates.

## Breaking Changes
- Removed deprecated API
- Changed default timeout

Some additional notes about migration.

## Bug Fixes
- Fixed memory leak";

        let changelog = parse_changelog(body);
        let headings: Vec<_> = changelog
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChangelogBlock::Heading(h) => Some(h.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(headings, vec!["Breaking Changes", "Bug Fixes"]);

        let bullets: Vec<_> = changelog
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChangelogBlock::Bullet(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            bullets,
            vec![
                "Removed deprecated API",
                "Changed default timeout",
                "Fixed memory leak"
            ]
        );

        let paragraphs: Vec<_> = changelog
            .blocks
            .iter()
            .filter_map(|b| match b {
                ChangelogBlock::Paragraph(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            paragraphs,
            vec![
                "This release includes important updates.",
                "Some additional notes about migration."
            ]
        );
    }
}
