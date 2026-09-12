//! Local Git review, rendered from immutable snapshots without filesystem IO.
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
            mutate_with(&submit, |st| dispatch(st, "diff.refresh"));
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
        let mut tab = button(shared, title, format!("diff.mode:{mode}"));
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
    toolbar = toolbar.with_child(button(shared, "Refresh", "diff.refresh"));
    let header = ElementDef::new(Tag::Div)
        .with_class("diff-header")
        .with_child(label("diff-title", "Changes"))
        .with_child(label("diff-subtitle", review.root.display().to_string()))
        .with_child(button(shared, "Close · Esc", "diff.close").with_autofocus(true));
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
    ElementDef::new(Tag::Div).with_class("diff-overlay").with_id("diff-review")
        .on_click(|| {})
        .with_child(header).with_child(toolbar).with_child(content)
        .with_child(label("diff-footer", "LOCAL GIT REVIEW   ·   Unified diff   ·   Read only   ·   Refresh uses local refs; no automatic fetch"))
}

fn build_files_and_patch(shared: &SharedState, review: &Review) -> ElementDef {
    let report = review.report.as_ref().unwrap();
    let mut files = ElementDef::new(Tag::Div).with_class("diff-files");
    if report.files.len() > PAGE_FILES {
        files = files.with_child(
            ElementDef::new(Tag::Div)
                .with_class("diff-pagination")
                .with_child(button(shared, "Previous files", "diff.files_prev"))
                .with_child(button(shared, "Next files", "diff.files_next")),
        );
    }
    for (index, file) in report
        .files
        .iter()
        .enumerate()
        .skip(review.file_page * PAGE_FILES)
        .take(PAGE_FILES)
    {
        let name = file
            .old_path
            .as_ref()
            .map(|old| format!("{old} → {}", file.path))
            .unwrap_or_else(|| file.path.clone());
        let stats = match (file.added, file.removed) {
            (Some(a), Some(d)) => format!("+{a}  -{d}"),
            _ => "Binary".into(),
        };
        let mut entry = button(shared, "", format!("diff.file:{index}"))
            .with_class("diff-file")
            .with_child(label("diff-file-name", name))
            .with_child(label("diff-file-stats", stats));
        if index == review.selected {
            entry = entry.with_class("diff-active");
        }
        files = files.with_child(entry);
    }
    let mut patch = ElementDef::new(Tag::Div).with_class("diff-patch-panel");
    if let Some(file) = report.files.get(review.selected) {
        patch = patch.with_child(label("diff-path", &file.path));
    }
    if review.loading {
        patch = patch.with_child(label("diff-empty", "Loading file diff…"));
    } else {
        let mut lines = ElementDef::new(Tag::Div)
            .with_class("diff-lines")
            .with_id(format!(
                "diff-lines-{}-{}-{}",
                review.request, review.selected, review.page
            ));
        for line in review
            .lines
            .iter()
            .skip(review.page * PAGE_LINES)
            .take(PAGE_LINES)
        {
            lines = lines.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("diff-line")
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
        patch = patch.with_child(lines);
        if review.lines.len() > PAGE_LINES {
            patch = patch.with_child(
                ElementDef::new(Tag::Div)
                    .with_class("diff-pagination")
                    .with_child(button(shared, "Previous", "diff.prev"))
                    .with_child(label(
                        "diff-page-label",
                        format!(
                            "Page {} / {}",
                            review.page + 1,
                            review.lines.len().div_ceil(PAGE_LINES)
                        ),
                    ))
                    .with_child(button(shared, "Next", "diff.next")),
            );
        }
    }
    ElementDef::new(Tag::Div)
        .with_class("diff-columns")
        .with_child(files)
        .with_child(patch)
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
    fn diff_review_visual_dump_when_requested() {
        let Some(path) = std::env::var_os("TM_DIFF_VISUAL_DUMP") else {
            return;
        };
        let shared = fixture();
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
