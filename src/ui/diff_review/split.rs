use super::label;
use crate::diff_review::{
    split::{Cell, Row},
    Review, PAGE_LINES,
};
use unshit::core::element::*;

pub(super) fn build(review: &Review) -> ElementDef {
    let mut table = ElementDef::new(Tag::Div)
        .with_class("diff-split-table")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("diff-split-heading")
                .with_child(label(
                    "diff-split-side-label",
                    if review.mode == "patch" {
                        "Before"
                    } else {
                        "Before · base"
                    },
                ))
                .with_child(label(
                    "diff-split-side-label",
                    if review.mode == "patch" {
                        "After"
                    } else {
                        "After · HEAD"
                    },
                )),
        );
    for (index, row) in review
        .split_rows
        .iter()
        .enumerate()
        .skip(review.row_start)
        .take(PAGE_LINES)
    {
        let row = match row {
            Row::Pair { old, new } => pair(review, old.as_ref(), new.as_ref()),
            Row::Shared(line) if review.lines[*line].kind == "context" => {
                let cell = Cell {
                    line: *line,
                    note: None,
                };
                pair(review, Some(&cell), Some(&cell))
            }
            Row::Shared(line) => {
                let active = review.is_active_hunk_line(*line);
                let line = &review.lines[*line];
                ElementDef::new(Tag::Div)
                    .with_class("diff-line")
                    .with_class(if active {
                        "diff-hunk-current"
                    } else {
                        "diff-row"
                    })
                    .with_class(format!("diff-{}", line.kind))
                    .with_child(label("diff-code", &line.text).with_class("diff-split-code"))
            }
        };
        table = table.with_child(row.with_id(format!("diff-split-row-{index}")));
    }
    table
}

fn pair(review: &Review, old: Option<&Cell>, new: Option<&Cell>) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("diff-split-row")
        .with_child(cell(review, old, true))
        .with_child(cell(review, new, false))
}

fn cell(review: &Review, cell: Option<&Cell>, old: bool) -> ElementDef {
    let el = ElementDef::new(Tag::Div)
        .with_class("diff-split-cell")
        .with_class(if old {
            "diff-split-old"
        } else {
            "diff-split-new"
        });
    let Some(cell) = cell else {
        return el.with_class("diff-split-gap");
    };
    let line = &review.lines[cell.line];
    let number = if old { line.old } else { line.new };
    let mut el = el.with_class(format!("diff-{}", line.kind)).with_child(
        ElementDef::new(Tag::Div)
            .with_class("diff-split-source")
            .with_child(label(
                "diff-gutter",
                number.map(|n| n.to_string()).unwrap_or_default(),
            ))
            .with_child(label("diff-code", &line.text).with_class("diff-split-code")),
    );
    if let Some(note) = cell.note {
        el = el.with_child(label("diff-split-note", &review.lines[note].text));
    }
    el
}
