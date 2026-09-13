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

fn input(shared: &SharedState, review: &Review, count: bool) -> ElementDef {
    let change = shared.clone();
    let submit = shared.clone();
    ElementDef::new(Tag::Input)
        .with_class("diff-input")
        .with_id(if count { "diff-count" } else { "diff-base" })
        .with_tab_index(0)
        .with_value(if count {
            &review.count
        } else {
            &review.base_ref
        })
        .with_placeholder(if count {
            "Commit count"
        } else {
            "Base branch, tag or SHA"
        })
        .on_change(move |text| {
            mutate_with(&change, |st| {
                if let Some(r) = st.diff_review.as_mut() {
                    let value = text.chars().take(1024).collect();
                    if count {
                        r.count = value;
                    } else {
                        r.base_ref = value;
                    }
                }
            });
        })
        .on_submit(move |_| {
            mutate_with(&submit, |st| dispatch(st, "review.refresh"));
        })
}

pub fn build(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let Some(review) = &snap.diff_review else {
        return ElementDef::new(Tag::Div).with_class("confirm-dialog-hidden");
    };
    let mut toolbar = ElementDef::new(Tag::Div).with_class("diff-toolbar");
    for (mode, title) in [
        ("last", "Last N commits"),
        ("unpushed", "Unpushed"),
        ("base", "Compare base"),
    ] {
        let mut tab = button(shared, title, format!("review.mode:{mode}"));
        if review.mode == mode {
            tab = tab.with_class("diff-active");
        }
        toolbar = toolbar.with_child(tab);
    }
    if review.mode != "unpushed" {
        toolbar = toolbar
            .with_child(label(
                "diff-field-label",
                if review.mode == "last" {
                    "Count"
                } else {
                    "Base ref"
                },
            ))
            .with_child(input(shared, review, review.mode == "last"));
    }
    toolbar = toolbar.with_child(button(shared, "Refresh", "review.refresh"));
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
    toolbar = toolbar.with_child(views);
    let header = ElementDef::new(Tag::Div)
        .with_class("diff-header")
        .with_child(label("diff-title", "Changes"))
        .with_child(label("diff-subtitle", review.root.display().to_string()))
        .with_child(button(shared, "Close · Esc", "review.close").with_autofocus(true));
    let mut content = ElementDef::new(Tag::Div).with_class("diff-content");
    if let Some(report) = &review.report {
        let added: usize = report.files.iter().filter_map(|f| f.added).sum();
        let removed: usize = report.files.iter().filter_map(|f| f.removed).sum();
        content = content.with_child(label(
            "diff-summary",
            format!(
                "{} files   +{}  -{}   ·   {} → {}\n{}",
                report.files.len(),
                added,
                removed,
                &report.base[..8],
                &report.head[..8],
                report.label
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
                "Reading Git history…"
            } else {
                "Choose a range and refresh to review committed changes."
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
            "LOCAL GIT REVIEW   ·   Read only   ·   Refresh uses local refs; no automatic fetch",
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
mod tests {
    use super::*;
    use crate::diff_review::git::{parse_patch, File, Report};
    use crate::state::{seed_state, MutexExt};
    use std::sync::{Arc, Mutex};
    use unshit_test::TestHarness;

    fn fixture() -> SharedState {
        let mut state = seed_state();
        let mut review = Review::new("C:/projects/unshit".into());
        review.report = Some(Arc::new(Report {
            root: review.root.clone(),
            base: "a123456789".into(),
            head: "b123456789".into(),
            label: "Last 1 commits · first-parent history".into(),
            files: vec![File {
                path: "src/main.rs".into(),
                old_path: None,
                added: Some(2),
                removed: Some(1),
            }],
        }));
        review.lines = Arc::new(parse_patch("diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -12,3 +12,4 @@ fn main() {\n     let app = App::new();\n-    app.run();\n+    app.with_review_panel();\n+    app.run();\n }\n"));
        state.diff_review = Some(review);
        let review = state.diff_review.as_mut().unwrap();
        review.set_file_filter("");
        review.split_rows = Arc::new(crate::diff_review::split::align(&review.lines));
        review.hunks = Arc::new(crate::diff_review::collect_hunks(
            &review.lines,
            &review.split_rows,
        ));
        Arc::new(Mutex::new(state))
    }

    fn tree(shared: &SharedState) -> ElementTree {
        crate::build_tree(
            &shared.lock_recover().ui_snapshot(),
            shared,
            &Default::default(),
            None,
        )
    }

    #[test]
    fn viewed_toggle_updates_progress_without_hiding_the_patch() {
        for width in [800.0, 1280.0] {
            let shared = fixture();
            let mut harness = TestHarness::new(
                include_str!("../../assets/styles.css"),
                || tree(&shared),
                width,
                720.0,
            );
            harness.step();
            let toggle = harness.query("#diff-viewed-toggle").unwrap().layout_rect;
            assert!(toggle.width > 0.0 && toggle.x + toggle.width <= width);
            harness.locator("#diff-viewed-toggle").click();
            harness.rebuild(|| tree(&shared));
            harness.step();
            assert_eq!(
                harness.query(".diff-review-progress").unwrap().content,
                ElementContent::Text("1 of 1 files viewed".into())
            );
            assert!(harness.query(".diff-viewed-label").is_some());
            assert!(harness.query(".diff-added").is_some());
            harness.locator("#diff-viewed-toggle").click();
            harness.rebuild(|| tree(&shared));
            harness.step();
            assert_eq!(
                harness.query(".diff-review-progress").unwrap().content,
                ElementContent::Text("0 of 1 files viewed".into())
            );
            assert!(harness.query(".diff-viewed-label").is_none());
        }
    }

    #[test]
    fn diff_review_layout_and_input_work_at_two_window_sizes() {
        for width in [800.0, 1280.0] {
            let shared = fixture();
            let mut harness = TestHarness::new(
                include_str!("../../assets/styles.css"),
                || tree(&shared),
                width,
                720.0,
            );
            harness.step();
            assert!(harness.query(".terminal-grid").is_none());
            for selector in [
                ".diff-overlay",
                ".diff-files",
                ".diff-lines",
                ".diff-added",
                ".diff-removed",
                "#diff-count",
            ] {
                let node = harness.query(selector).expect(selector);
                assert!(
                    node.layout_rect.width > 0.0 && node.layout_rect.height > 0.0,
                    "{selector}: {:?}",
                    node.layout_rect
                );
            }
            let files = harness.query(".diff-files").unwrap().layout_rect;
            let patch = harness.query(".diff-patch-panel").unwrap().layout_rect;
            assert!(patch.x >= files.x + files.width - 1.0);
            assert!(patch.x + patch.width <= width);
            harness.locator("#diff-count").fill("5");
            assert_eq!(
                shared.lock_recover().diff_review.as_ref().unwrap().count,
                "5"
            );
            harness.locator_by_text("Close · Esc").click();
            assert!(shared.lock_recover().diff_review.is_none());
        }
    }

    #[test]
    fn hunk_buttons_reveal_targets_and_reset_scroll_in_both_views() {
        for side_by_side in [false, true] {
            let shared = fixture();
            {
                let mut state = shared.lock_recover();
                let review = state.diff_review.as_mut().unwrap();
                let patch = format!(
                    "@@ -1,211 +1,211 @@\n{}{} same\n@@ -900 +900 @@\n-before\n+after\n",
                    "-old\n".repeat(210),
                    "+new\n".repeat(210)
                );
                review.lines = Arc::new(parse_patch(&patch));
                review.split_rows = Arc::new(crate::diff_review::split::align(&review.lines));
                review.hunks = Arc::new(crate::diff_review::collect_hunks(
                    &review.lines,
                    &review.split_rows,
                ));
                review.side_by_side = side_by_side;
            }
            let mut harness = TestHarness::new(
                include_str!("../../assets/styles.css"),
                || tree(&shared),
                800.0,
                720.0,
            );
            harness.step();
            let viewport = harness.query(".diff-lines").unwrap().layout_rect;
            harness.mouse_wheel(viewport.x + 60.0, viewport.y + 60.0, 0.0, -150.0);
            assert!(harness.query(".diff-lines").unwrap().scroll_y > 0.0);
            for expected in [0, 1] {
                harness.locator_by_text("Next hunk").click();
                harness.rebuild(|| tree(&shared));
                harness.step();
                let viewport = harness.query(".diff-lines").unwrap();
                assert_eq!(viewport.scroll_y, 0.0);
                let current = harness.query(".diff-hunk-current").unwrap().layout_rect;
                assert!(current.y >= viewport.layout_rect.y);
                assert!(
                    current.y < viewport.layout_rect.y + 48.0,
                    "hunk must be visible at top: {current:?}"
                );
                assert_eq!(
                    shared
                        .lock_recover()
                        .diff_review
                        .as_ref()
                        .unwrap()
                        .active_hunk,
                    Some(expected)
                );
                assert!(
                    harness.query_all(".diff-line").len()
                        + harness.query_all(".diff-split-row").len()
                        <= PAGE_LINES
                );
            }
            let last = shared
                .lock_recover()
                .diff_review
                .as_ref()
                .unwrap()
                .row_start;
            assert!(last >= PAGE_LINES);
            harness.locator_by_text("Next hunk").click();
            assert_eq!(
                shared
                    .lock_recover()
                    .diff_review
                    .as_ref()
                    .unwrap()
                    .row_start,
                last
            );
            harness.locator_by_text("Previous hunk").click();
            harness.rebuild(|| tree(&shared));
            harness.step();
            assert_eq!(
                shared
                    .lock_recover()
                    .diff_review
                    .as_ref()
                    .unwrap()
                    .active_hunk,
                Some(0)
            );
            harness.locator_by_text("File start").click();
            harness.rebuild(|| tree(&shared));
            harness.step();
            assert!(harness.query(".diff-hunk-current").is_none());
            assert_eq!(
                shared
                    .lock_recover()
                    .diff_review
                    .as_ref()
                    .unwrap()
                    .row_start,
                0
            );
        }
    }

    #[test]
    fn file_filter_input_empty_state_clear_and_selection() {
        let shared = fixture();
        {
            let mut state = shared.lock_recover();
            let review = state.diff_review.as_mut().unwrap();
            Arc::make_mut(review.report.as_mut().unwrap())
                .files
                .push(File {
                    path: "README.md".into(),
                    old_path: Some("docs/Old.md".into()),
                    added: Some(1),
                    removed: Some(0),
                });
            review.set_file_filter("");
        }
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            || tree(&shared),
            800.0,
            720.0,
        );
        harness.step();
        harness.locator("#diff-file-filter").fill("no");
        harness.rebuild(|| tree(&shared));
        harness.step();
        harness.type_text("-match");
        assert_eq!(
            shared
                .lock_recover()
                .diff_review
                .as_ref()
                .unwrap()
                .file_filter,
            "no-match"
        );
        harness.rebuild(|| tree(&shared));
        harness.step();
        assert!(harness.query_all(".diff-file").is_empty());
        assert!(harness.query(".diff-filter-empty").is_some());
        assert!(harness.query(".diff-filter-notice").is_some());
        assert!(
            harness.query(".diff-removed").is_some(),
            "open patch stays visible"
        );
        harness.locator_by_text("Clear filter").click();
        harness.rebuild(|| tree(&shared));
        harness.step();
        assert_eq!(harness.query_all(".diff-file").len(), 2);
        assert_eq!(
            harness
                .query("#diff-file-filter")
                .unwrap()
                .input_value
                .as_deref(),
            Some("")
        );
        assert!(harness.query(".diff-filter-notice").is_none());
        harness.locator("#diff-file-filter").fill("OLD");
        harness.rebuild(|| tree(&shared));
        harness.step();
        assert_eq!(harness.query_all(".diff-file").len(), 1);
        harness.locator(".diff-file").click();
        assert_eq!(
            shared.lock_recover().diff_review.as_ref().unwrap().selected,
            1
        );
    }

    #[test]
    fn split_view_toggle_alignment_and_horizontal_scrolling() {
        for width in [800.0, 1280.0] {
            let shared = fixture();
            let mut harness = TestHarness::new(
                include_str!("../../assets/styles.css"),
                || tree(&shared),
                width,
                720.0,
            );
            harness.step();
            harness.locator_by_text("Side by side").click();
            assert!(
                shared
                    .lock_recover()
                    .diff_review
                    .as_ref()
                    .unwrap()
                    .side_by_side
            );
            harness.rebuild(|| tree(&shared));
            harness.step();
            let old = harness.query_all(".diff-split-old");
            let new = harness.query_all(".diff-split-new");
            assert_eq!(old.len(), 4);
            assert_eq!(old.len(), new.len());
            for (old, new) in old.iter().zip(&new) {
                assert!((old.layout_rect.y - new.layout_rect.y).abs() < 1.0);
                assert!((old.layout_rect.height - new.layout_rect.height).abs() < 1.0);
                assert!((old.layout_rect.width - new.layout_rect.width).abs() < 2.0);
                assert!(new.layout_rect.x >= old.layout_rect.x + old.layout_rect.width - 1.0);
            }
            assert!(old[2].classes.iter().any(|c| c == "diff-split-gap"));
            let viewport = harness.query(".diff-lines").unwrap().layout_rect;
            if width == 800.0 {
                let table = harness.query(".diff-split-table").unwrap().layout_rect;
                assert!(table.width > viewport.width);
                harness.mouse_wheel(viewport.x + 40.0, viewport.y + 40.0, -150.0, 0.0);
                assert!(harness.query(".diff-lines").unwrap().scroll_x > 0.0);
            }
            harness.locator_by_text("Unified").click();
            harness.rebuild(|| tree(&shared));
            harness.step();
            assert!(harness.query(".diff-split-table").is_none());
            assert!(harness.query(".diff-removed").is_some());
        }
    }

    #[test]
    fn split_wrapped_rows_and_notes_share_height_and_vertical_scroll() {
        let shared = fixture();
        {
            let mut state = shared.lock_recover();
            let review = state.diff_review.as_mut().unwrap();
            let patch = format!(
                "@@ -1,81 +1,81 @@\n-old\n\\ No newline at end of file\n+{}\n{}",
                "long replacement text ".repeat(30),
                " context\n".repeat(80)
            );
            review.lines = Arc::new(parse_patch(&patch));
            review.split_rows = Arc::new(crate::diff_review::split::align(&review.lines));
            review.hunks = Arc::new(crate::diff_review::collect_hunks(
                &review.lines,
                &review.split_rows,
            ));
            review.side_by_side = true;
        }
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            || tree(&shared),
            1280.0,
            720.0,
        );
        harness.step();
        let old = harness.query(".diff-split-old").unwrap().layout_rect;
        let new = harness.query(".diff-split-new").unwrap().layout_rect;
        assert!(new.height > 44.0, "long replacement should wrap: {new:?}");
        assert!((old.height - new.height).abs() < 1.0);
        let gutter = harness
            .query(".diff-split-new .diff-gutter")
            .unwrap()
            .layout_rect;
        assert!((gutter.y - new.y).abs() < 1.0);
        assert!(
            gutter.height < new.height,
            "line number must stay at the top of a wrapped line"
        );
        assert!(harness.query(".diff-split-old .diff-split-note").is_some());
        assert!(harness.query(".diff-split-new .diff-split-note").is_none());
        let viewport = harness.query(".diff-lines").unwrap().layout_rect;
        harness.mouse_wheel(viewport.x + 60.0, viewport.y + 60.0, 0.0, -150.0);
        assert!(harness.query(".diff-lines").unwrap().scroll_y > 0.0);
    }

    #[test]
    fn diff_review_visual_dump_when_requested() {
        let Some(path) = std::env::var_os("TM_DIFF_VISUAL_DUMP") else {
            return;
        };
        let shared = fixture();
        if std::env::var("TM_DIFF_VISUAL_MODE").as_deref() == Ok("split") {
            shared
                .lock_recover()
                .diff_review
                .as_mut()
                .unwrap()
                .side_by_side = true;
        }
        if std::env::var("TM_DIFF_VISUAL_HUNK").as_deref() == Ok("1") {
            dispatch(&mut shared.lock_recover(), "review.hunk_next");
        }
        if std::env::var("TM_DIFF_VISUAL_VIEWED").as_deref() == Ok("1") {
            dispatch(&mut shared.lock_recover(), "review.viewed");
        }
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            || tree(&shared),
            1280.0,
            800.0,
        );
        assert!(
            harness.try_with_gpu(),
            "GPU required for requested screenshot"
        );
        harness.step();
        harness.screenshot().save(path).unwrap();
    }
}
