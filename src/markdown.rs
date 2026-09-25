//! Markdown presentation model for editor side previews.

use std::collections::BTreeMap;
use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Tag, TagEnd};

pub const MAX_PREVIEW_BLOCKS: usize = 512;
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownBlock {
    Heading {
        level: u8,
        text: String,
        source_line: usize,
        anchor: String,
    },
    Paragraph(String),
    Code {
        language: String,
        text: String,
    },
    Quote(Vec<MarkdownBlock>),
    List {
        ordered: bool,
        items: Vec<Vec<MarkdownBlock>>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub blocks: Vec<MarkdownBlock>,
    pub truncated: bool,
}

/// How a Markdown editor pane presents itself in the UI snapshot.
#[derive(Clone, Debug)]
pub enum MarkdownView {
    /// Source only; the toolbar offers to open the preview.
    Closed,
    Open(Arc<MarkdownDocument>),
}

#[derive(Debug)]
enum DraftNode {
    Container,
    Heading(u8, String, usize),
    Paragraph(String),
    Code(String, String),
    Quote,
    List(bool),
    Item(String),
}

#[derive(Debug)]
struct Draft {
    node: DraftNode,
    children: Vec<usize>,
}

pub fn parse(text: &str) -> Arc<MarkdownDocument> {
    use pulldown_cmark::{Event, Options, Parser};

    let mut input_truncated = false;
    let text = if text.len() > MAX_PREVIEW_BYTES {
        let mut end = MAX_PREVIEW_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        input_truncated = true;
        &text[..end]
    } else {
        text
    };
    let mut drafts = vec![Draft {
        node: DraftNode::Container,
        children: Vec::new(),
    }];
    let mut stack = vec![0usize];
    let options = Options::ENABLE_STRIKETHROUGH;

    // Heading lines are counted incrementally: headings start in document
    // order, so each one only scans the text since the previous heading.
    let mut heading_line = 0usize;
    let mut scanned = 0usize;

    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        let current = *stack.last().expect("draft stack is never empty");
        match event {
            Event::Start(tag) => {
                let source_line = if matches!(tag, Tag::Heading { .. }) {
                    heading_line += text[scanned..range.start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count();
                    scanned = range.start;
                    heading_line
                } else {
                    0
                };
                let id = start_tag(&mut drafts, current, tag, source_line);
                // Containers stay leaves so their inline text folds into the
                // enclosing block.
                if !matches!(drafts[id].node, DraftNode::Container) {
                    stack.push(id);
                }
            }
            Event::End(tag) => {
                // Only block tags are ever pushed, so the root is all that
                // is left once the stack has one entry.
                if end_tag_pops(tag) && stack.len() > 1 {
                    stack.pop();
                }
            }
            Event::Text(value) | Event::Code(value) => {
                append_inline(&mut drafts, current, &value);
            }
            Event::SoftBreak | Event::HardBreak => append_inline(&mut drafts, current, " "),
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[done] " } else { "[ ] " };
                append_inline(&mut drafts, current, marker);
            }
            _ => {}
        }
    }
    let mut document = MarkdownDocument {
        truncated: input_truncated,
        ..MarkdownDocument::default()
    };
    let mut count = 0usize;
    let mut anchors = BTreeMap::new();
    append_children(
        &drafts,
        0,
        &mut document.blocks,
        &mut count,
        &mut document.truncated,
        &mut anchors,
    );
    Arc::new(document)
}

fn start_tag(drafts: &mut Vec<Draft>, parent: usize, tag: Tag, source_line: usize) -> usize {
    let node = match tag {
        Tag::Paragraph => DraftNode::Paragraph(String::new()),
        Tag::Heading { level, .. } => DraftNode::Heading(
            match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            },
            String::new(),
            source_line,
        ),
        Tag::CodeBlock(kind) => DraftNode::Code(
            match kind {
                CodeBlockKind::Fenced(info) => info
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                CodeBlockKind::Indented => String::new(),
            },
            String::new(),
        ),
        Tag::BlockQuote(_) => DraftNode::Quote,
        Tag::List(start) => DraftNode::List(start.is_some()),
        Tag::Item => DraftNode::Item(String::new()),
        // Uncommon containers are flattened into paragraph text.
        _ => DraftNode::Container,
    };
    drafts.push(Draft {
        node,
        children: Vec::new(),
    });
    let id = drafts.len() - 1;
    drafts[parent].children.push(id);
    id
}

fn end_tag_pops(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::CodeBlock
            | TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::Item
    )
}

fn append_inline(drafts: &mut [Draft], id: usize, value: &str) {
    match &mut drafts[id].node {
        DraftNode::Paragraph(text)
        | DraftNode::Heading(_, text, _)
        | DraftNode::Code(_, text)
        | DraftNode::Item(text) => {
            text.push_str(value);
        }
        DraftNode::Quote | DraftNode::List(_) | DraftNode::Container => {}
    }
}

fn append_children(
    drafts: &[Draft],
    id: usize,
    out: &mut Vec<MarkdownBlock>,
    count: &mut usize,
    truncated: &mut bool,
    anchors: &mut BTreeMap<String, usize>,
) {
    for &child in &drafts[id].children {
        if *count == MAX_PREVIEW_BLOCKS {
            *truncated = true;
            return;
        }
        *count += 1;
        let block = match &drafts[child].node {
            DraftNode::Container => {
                let mut nested = Vec::new();
                append_children(drafts, child, &mut nested, count, truncated, anchors);
                MarkdownBlock::Paragraph(
                    nested.iter().map(block_text).collect::<Vec<_>>().join(" "),
                )
            }
            DraftNode::Item(text) => {
                let mut nested = Vec::new();
                if !text.trim().is_empty() {
                    nested.push(MarkdownBlock::Paragraph(normalize_space(text)));
                }
                append_children(drafts, child, &mut nested, count, truncated, anchors);
                MarkdownBlock::Paragraph(
                    nested.iter().map(block_text).collect::<Vec<_>>().join(" "),
                )
            }
            DraftNode::Heading(level, text, source_line) => {
                let text = normalize_space(text);
                let anchor = unique_anchor(&text, anchors);
                MarkdownBlock::Heading {
                    level: *level,
                    text,
                    source_line: *source_line,
                    anchor,
                }
            }
            DraftNode::Paragraph(text) => MarkdownBlock::Paragraph(normalize_space(text)),
            DraftNode::Code(language, text) => MarkdownBlock::Code {
                language: language.clone(),
                text: text.trim_end_matches('\n').to_string(),
            },
            DraftNode::Quote => {
                let mut nested = Vec::new();
                append_children(drafts, child, &mut nested, count, truncated, anchors);
                MarkdownBlock::Quote(nested)
            }
            DraftNode::List(ordered) => {
                let mut items = Vec::new();
                for &item in &drafts[child].children {
                    if *count == MAX_PREVIEW_BLOCKS {
                        *truncated = true;
                        break;
                    }
                    *count += 1;
                    let mut nested = Vec::new();
                    if let DraftNode::Item(text) = &drafts[item].node {
                        if !text.trim().is_empty() {
                            nested.push(MarkdownBlock::Paragraph(normalize_space(text)));
                        }
                    }
                    append_children(drafts, item, &mut nested, count, truncated, anchors);
                    items.push(nested);
                }
                MarkdownBlock::List {
                    ordered: *ordered,
                    items,
                }
            }
        };
        out.push(block);
    }
}

fn block_text(block: &MarkdownBlock) -> &str {
    match block {
        MarkdownBlock::Heading { text, .. } | MarkdownBlock::Paragraph(text) => text,
        MarkdownBlock::Code { text, .. } => text,
        MarkdownBlock::Quote(_) | MarkdownBlock::List { .. } => "",
    }
}

fn normalize_space(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for character in text.trim().chars() {
        if character.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(character);
            last_space = false;
        }
    }
    out
}

fn unique_anchor(text: &str, seen: &mut BTreeMap<String, usize>) -> String {
    let mut slug = String::new();
    let mut last_separator = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_separator = false;
        } else if !last_separator {
            slug.push('-');
            last_separator = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    };
    let count = seen.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base}-{count}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structure_without_inline_delimiters() {
        let fence = char::from(96).to_string().repeat(3);
        let source = format!(
            "# Title\n\nSome **bold** text.\n\n- one\n- two\n\n> quoted\n\n{fence}rust\nfn main() {{}}\n{fence}\n"
        );
        let document = parse(&source);

        assert_eq!(
            document.blocks,
            vec![
                MarkdownBlock::Heading {
                    level: 1,
                    text: "Title".into(),
                    source_line: 0,
                    anchor: "title".into()
                },
                MarkdownBlock::Paragraph("Some bold text.".into()),
                MarkdownBlock::List {
                    ordered: false,
                    items: vec![
                        vec![MarkdownBlock::Paragraph("one".into())],
                        vec![MarkdownBlock::Paragraph("two".into())]
                    ]
                },
                MarkdownBlock::Quote(vec![MarkdownBlock::Paragraph("quoted".into())]),
                MarkdownBlock::Code {
                    language: "rust".into(),
                    text: "fn main() {}".into()
                },
            ]
        );
    }

    #[test]
    fn caps_large_documents() {
        let source = (0..MAX_PREVIEW_BLOCKS + 20)
            .map(|index| format!("{index}\n\n"))
            .collect::<String>();
        let document = parse(&source);

        assert!(document.truncated);
        assert!(document.blocks.len() <= MAX_PREVIEW_BLOCKS);
    }

    #[test]
    fn heading_anchors_keep_source_lines_and_unique_slugs() {
        let document = parse("Intro\n\n# Same Name\n\nText\n\n# Same Name\n");
        let headings: Vec<_> = document
            .blocks
            .iter()
            .filter_map(|block| match block {
                MarkdownBlock::Heading {
                    source_line,
                    anchor,
                    ..
                } => Some((*source_line, anchor.clone())),
                _ => None,
            })
            .collect();

        assert_eq!(
            headings,
            vec![(2, "same-name".to_string()), (6, "same-name-2".to_string())]
        );
    }
}
