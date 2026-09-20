//! Local Git review, rendered from immutable snapshots without filesystem IO.
mod split;
use crate::diff_review::{Review, PAGE_FILES, PAGE_LINES};
use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};
use unshit::core::element::*;

fn label(class: &str, text: impl Into<String>) -> ElementDef {
    ElementDef::new(Tag::Div).with_class(class).with_text(text)
}

fn button(shared: &SharedState, text: &str, command: impl Into<String>) -> ElementDef {
    let shared = shared.clone();
    let command = command.into();
    ElementDef::new(Tag::Button)
        .with_class("diff-button")
        .with_tab_index(0)
        .with_text(text)
        .on_click(move || {
            mutate_with(&shared, |st| dispatch(st, &command));
        })
}

#[derive(Clone, Copy)]
enum Field {
    Count,
    Base,
    From,
    To,
}

fn input(shared: &SharedState, review: &Review, field: Field) -> ElementDef {
    let change = shared.clone();
    let submit = shared.clone();
    let (id, title, value, placeholder) = match field {
        Field::Count => ("diff-count", "Count", &review.count, "Commit count"),
        Field::Base => (
            "diff-base",
            "Base ref",
            &review.base_ref,
            "Auto: default branch",
        ),
        Field::From => (
            "diff-from",
            "From",
            &review.from_ref,
            "Auto: default branch",
        ),
        Field::To => ("diff-to", "To", &review.to_ref, "Branch, tag or commit"),
    };
    let input = ElementDef::new(Tag::Input)
        .with_class("diff-input")
        .with_id(id)
        .with_tab_index(0)
        .with_value(value)
        .with_placeholder(placeholder)
        .on_change(move |text| {
            mutate_with(&change, |st| {
                if let Some(r) = st.diff_review.as_mut() {
                    let value = text.chars().take(1024).collect();
                    match field {
                        Field::Count => r.count = value,
                        Field::Base => r.base_ref = value,
                        Field::From => r.from_ref = value,
                        Field::To => r.to_ref = value,
                    }
                }
            });
        })
        .on_submit(move |_| {
            mutate_with(&submit, |st| dispatch(st, "review.refresh"));
        });
    ElementDef::new(Tag::Div)
        .with_class("diff-field")
        .with_key(id)
        .with_child(label("diff-field-label", title))
        .with_child(input)
}

pub fn build(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let Some(review) = &snap.diff_review else {
        return ElementDef::new(Tag::Div).with_class("confirm-dialog-hidden");
    };
    let mut modes = ElementDef::new(Tag::Div).with_class("diff-controls");
    for (mode, title) in [
        ("last", "Last N commits"),
        ("unpushed", "Unpushed"),
        ("base", "Compare base"),
        ("branches", "Compare branches"),
    ] {
        let mut tab = button(shared, title, format!("review.mode:{mode}"));
        if review.mode == mode {
            tab = tab.with_class("diff-active");
        }
        modes = modes.with_child(tab);
    }
    let mut views = ElementDef::new(Tag::Div).with_class("diff-view-switch");
    for (split, title, command) in [
        (false, "Unified", "review.view:unified"),
        (true, "Side by side", "review.view:split"),
    ] {
        let mut view = button(shared, title, command);
        if review.side_by_side == split {
            view = view.with_class("diff-active");
        }
        views = views.with_child(view);
    }
    modes = modes.with_child(views);
    let mut fields = ElementDef::new(Tag::Div).with_class("diff-controls");
    match review.mode {
        "base" => fields = fields.with_child(input(shared, review, Field::Base)),
        "branches" => {
            fields = fields
                .with_child(input(shared, review, Field::From))
                .with_child(input(shared, review, Field::To));
        }
        "unpushed" | "patch" => {}
        _ => fields = fields.with_child(input(shared, review, Field::Count)),
    }
    fields = fields
        .with_child(button(shared, "Open patch…", "review.patch_open").with_id("diff-open-patch"))
        .with_child(button(shared, "Refresh", "review.refresh"));
    let mut toolbar = ElementDef::new(Tag::Div)
        .with_class("diff-toolbar")
        .with_child(modes)
        .with_child(fields);
    let hint = match review.mode {
        "base" => Some("Current HEAD since the common ancestor. Leave Base ref empty to use the default branch."),
        "branches" => Some("Direct comparison from the first ref to the second. Leave From empty for the default branch; HEAD is your current commit."),
        _ => None,
    };
    if let Some(hint) = hint {
        toolbar = toolbar.with_child(label("diff-range-hint", hint));
    }
    let header = ElementDef::new(Tag::Div)
        .with_class("diff-header")
        .with_child(label("diff-title", "Changes"))
        .with_child(label(
            "diff-subtitle",
            if review.mode == "patch" {
                review.patch_path.as_ref().unwrap_or(&review.root)
            } else {
                &review.root
            }
            .display()
            .to_string(),
        ))
        .with_child(button(shared, "Close · Esc", "review.close").with_autofocus(true));
    let mut content = ElementDef::new(Tag::Div).with_class("diff-content");
    if let Some(report) = &review.report {
        let added: usize = report.files.iter().filter_map(|f| f.added).sum();
        let removed: usize = report.files.iter().filter_map(|f| f.removed).sum();
        let range = if report.patches.is_some() {
            "Patch file".to_string()
        } else {
            format!("{} → {}", report.base, report.head)
        };
        content = content.with_child(label(
            "diff-summary",
            format!(
                "{} files   +{}  -{}\n{}\n{}",
                report.files.len(),
                added,
                removed,
                report.label,
                range,
            ),
        ));
        if report.files.is_empty() {
            content = content.with_child(label(
                "diff-empty",
                "No changes in this range.\nWorking-tree and staged edits are not included.",
            ));
        } else {
            content = content.with_child(label(
                "diff-review-progress",
                format!(
                    "{} of {} files viewed",
                    review.viewed.len(),
                    report.files.len()
                ),
            ));
            content = content.with_child(build_files_and_patch(shared, review));
        }
    } else {
        content = content.with_child(label(
            "diff-empty",
            if review.loading {
                if review.mode == "patch" {
                    "Reading patch file…"
                } else {
                    "Reading Git history…"
                }
            } else {
                "Choose a Git range, open a patch file, or drop a .patch here."
            },
        ));
    }
    if let Some(error) = &review.error {
        content = content.with_child(label("diff-error", error));
    }
    ElementDef::new(Tag::Div)
        .with_class("diff-overlay")
        .with_id("diff-review")
        .on_click(|| {})
        .with_child(header)
        .with_child(toolbar)
        .with_child(content)
        .with_child(label(
            "diff-footer",
            if review.mode == "patch" {
                "PATCH REVIEW   ·   Read only   ·   Drop a .patch or .diff file to open it"
            } else {
                "LOCAL GIT REVIEW   ·   Read only   ·   Refresh uses local refs; no automatic fetch"
            },
        ))
}

fn build_files_and_patch(shared: &SharedState, review: &Review) -> ElementDef {
    let report = review.report.as_ref().unwrap();
    let mut files = ElementDef::new(Tag::Div).with_class("diff-files");
    if review.file_matches.len() > PAGE_FILES {
        files = files.with_child(
            ElementDef::new(Tag::Div)
                .with_class("diff-pagination")
                .with_child(button(shared, "Previous files", "review.files_prev"))
                .with_child(button(shared, "Next files", "review.files_next"))
                .with_child(label(
                    "diff-file-stats",
                    format!(
                        "Page {} / {}",
                        review.file_page + 1,
                        review.file_matches.len().div_ceil(PAGE_FILES)
                    ),
                )),
        );
    }
    for &index in review
        .file_matches
        .iter()
        .skip(review.file_page * PAGE_FILES)
        .take(PAGE_FILES)
    {
        let file = &report.files[index];
        let name = file
            .old_path
            .as_ref()
            .map(|old| format!("{old} → {}", file.path))
            .unwrap_or_else(|| file.path.clone());
        let stats = match (file.added, file.removed) {
            (Some(a), Some(d)) => format!("+{a}  -{d}"),
            _ => "Binary".into(),
        };
        let mut entry = button(shared, "", format!("review.file:{index}"))
            .with_class("diff-file")
            .with_child(label("diff-file-name", name))
            .with_child(label("diff-file-stats", stats));
        if index == review.selected {
            entry = entry.with_class("diff-active");
        }
        if review.viewed.contains(&index) {
            entry = entry.with_child(label("diff-viewed-label", "Viewed"));
        }
        files = files.with_child(entry);
    }
    if review.file_matches.is_empty() {
        files = files.with_child(label(
            "diff-filter-empty",
            "No files match this filter. Clear it to show all changed files.",
        ));
    }
    let sidebar = ElementDef::new(Tag::Div)
        .with_class("diff-file-sidebar")
        .with_child(build_file_filter(shared, review))
        .with_child(files);
    let mut patch = ElementDef::new(Tag::Div).with_class("diff-patch-panel");
    if let Some(file) = report.files.get(review.selected) {
        let mut header = ElementDef::new(Tag::Div)
            .with_class("diff-path-header")
            .with_child(label("diff-path", &file.path));
        if !review.loading && review.error.is_none() {
            let viewed = review.viewed.contains(&review.selected);
            header = header.with_child(
                button(
                    shared,
                    if viewed {
                        "Viewed · Undo"
                    } else {
                        "Mark viewed"
                    },
                    "review.viewed",
                )
                .with_id("diff-viewed-toggle")
                .with_class(if viewed {
                    "diff-active"
                } else {
                    "diff-unviewed"
                }),
            );
        }
        patch = patch.with_child(header);
        if !review.file_matches.contains(&review.selected) {
            patch = patch.with_child(label(
                "diff-filter-notice",
                "The open file is outside this filter. Select a matching file or clear the filter.",
            ));
        }
    }
    if review.loading {
        patch = patch.with_child(label("diff-empty", "Loading file diff…"));
    } else {
        patch = patch.with_child(build_hunk_navigation(shared, review));
        let scroll_key = format!(
            "diff-lines-{}-{}-{}-{}-{:?}",
            review.request,
            review.selected,
            review.row_start,
            review.side_by_side,
            review.active_hunk
        );
        let mut lines = ElementDef::new(Tag::Div)
            .with_class("diff-lines")
            .with_id(scroll_key.clone())
            .with_key(scroll_key);
        if review.side_by_side {
            lines = lines.with_child(split::build(review));
        } else {
            for (index, line) in review
                .lines
                .iter()
                .enumerate()
                .skip(review.row_start)
                .take(PAGE_LINES)
            {
                lines = lines.with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("diff-line")
                        .with_class(if review.is_active_hunk_line(index) {
                            "diff-hunk-current"
                        } else {
                            "diff-row"
                        })
                        .with_class(format!("diff-{}", line.kind))
                        .with_child(label(
                            "diff-gutter",
                            line.old.map(|n| n.to_string()).unwrap_or_default(),
                        ))
                        .with_child(label(
                            "diff-gutter",
                            line.new.map(|n| n.to_string()).unwrap_or_default(),
                        ))
                        .with_child(label("diff-code", &line.text)),
                );
            }
        }
        patch = patch.with_child(lines);
        if review.row_count() > PAGE_LINES {
            patch = patch.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("diff-pagination")
                    .with_child(button(shared, "Previous", "review.prev"))
                    .with_child(label(
                        "diff-page-label",
                        format!(
                            "Rows {}–{} of {}",
                            review.row_start + 1,
                            (review.row_start + PAGE_LINES).min(review.row_count()),
                            review.row_count()
                        ),
                    ))
                    .with_child(button(shared, "Next", "review.next")),
            );
        }
    }
    ElementDef::new(Tag::Div)
        .with_class("diff-columns")
        .with_child(sidebar)
        .with_child(patch)
}

fn build_file_filter(shared: &SharedState, review: &Review) -> ElementDef {
    let change = shared.clone();
    let input = ElementDef::new(Tag::Input)
        .with_class("diff-filter-input")
        .with_class("diff-input")
        .with_id("diff-file-filter")
        .with_key(format!("file-filter-{}", review.file_filter_reset))
        .with_tab_index(0)
        .with_placeholder("Filter files by path")
        .with_value(&review.file_filter)
        .on_change(move |text| {
            mutate_with(&change, |st| dispatch(st, &format!("review.filter:{text}")));
        });
    let total = review.report.as_ref().map_or(0, |r| r.files.len());
    ElementDef::new(Tag::Div)
        .with_class("diff-file-filter")
        .with_child(label("diff-field-label", "Filter files"))
        .with_child(input)
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("diff-filter-actions")
                .with_child(label(
                    "diff-filter-count",
                    format!("{} of {total} files", review.file_matches.len()),
                ))
                .with_child(button(shared, "Clear filter", "review.filter_clear")),
        )
}

fn build_hunk_navigation(shared: &SharedState, review: &Review) -> ElementDef {
    let status = if review.hunks.is_empty() {
        "No text hunks".to_string()
    } else if let Some(index) = review.active_hunk {
        format!("Hunk {} of {}", index + 1, review.hunks.len())
    } else {
        format!("{} hunks", review.hunks.len())
    };
    let mut bar = ElementDef::new(Tag::Div)
        .with_class("diff-hunk-navigation")
        .with_child(label("diff-hunk-status", status));
    for (text, command, enabled) in [
        (
            "Previous hunk",
            "review.hunk_prev",
            review.hunk_target(false).is_some(),
        ),
        (
            "Next hunk",
            "review.hunk_next",
            review.hunk_target(true).is_some(),
        ),
        (
            "File start",
            "review.file_start",
            review.row_start > 0 || review.active_hunk.is_some(),
        ),
    ] {
        bar = bar.with_child(if enabled {
            button(shared, text, command)
        } else {
            label("diff-button", text).with_class("diff-navigation-disabled")
        });
    }
    bar
}

#[cfg(test)]
mod tests;
