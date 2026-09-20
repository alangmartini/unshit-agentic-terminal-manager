use super::*;
use crate::diff_review::git::{parse_patch, File, Report};
use crate::state::{seed_state, MutexExt};
use std::sync::{Arc, Mutex};
use unshit_test::TestHarness;

fn fixture() -> SharedState {
    let mut state = seed_state();
    let mut review = Review::new("C:/projects/unshit".into());
    review.report = Some(Arc::new(Report {
        patches: None,
        root: review.root.clone(),
        base: "a123456789012345678901234567890123456789".into(),
        head: "b123456789012345678901234567890123456789".into(),
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

fn import_fixture(shared: &SharedState, path: &std::path::Path) {
    let report = crate::diff_review::patch_file::load(path).unwrap();
    let mut state = shared.lock_recover();
    let review = state.diff_review.as_mut().unwrap();
    review.mode = "patch";
    review.patch_path = Some(path.to_path_buf());
    review.lines = Arc::new(report.patches.as_ref().unwrap()[0].clone());
    review.split_rows = Arc::new(crate::diff_review::split::align(&review.lines));
    review.hunks = Arc::new(crate::diff_review::collect_hunks(
        &review.lines,
        &review.split_rows,
    ));
    review.report = Some(Arc::new(report));
    review.set_file_filter("");
}

#[test]
fn imported_patch_renders_and_keeps_review_controls() {
    let path = std::env::temp_dir().join(format!("review-ui-{}.patch", std::process::id()));
    std::fs::write(
        &path,
        "diff --git a/example.rs b/example.rs\n@@ -12 +12 @@\n-old\n+new\n",
    )
    .unwrap();
    let shared = fixture();
    import_fixture(&shared, &path);
    std::fs::remove_file(&path).unwrap();
    for width in [800.0, 1280.0] {
        let mut harness = TestHarness::new(
            include_str!("../../../assets/styles.css"),
            || tree(&shared),
            width,
            720.0,
        );
        harness.step();
        assert!(harness.query("#diff-open-patch").is_some());
        assert!(harness.query("#diff-count").is_none());
        assert!(harness.query(".diff-added").is_some());
        assert!(harness.query(".diff-removed").is_some());
        let summary = harness.query(".diff-summary").unwrap();
        assert!(
            matches!(summary.content, ElementContent::Text(ref text) if text.contains("1 files   +1  -1") && text.contains("Patch file"))
        );
        dispatch(&mut shared.lock_recover(), "review.hunk_next");
        dispatch(&mut shared.lock_recover(), "review.view:split");
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
        assert!(harness.query(".diff-added").is_some());
        assert_eq!(
            harness.query(".diff-split-side-label").unwrap().content,
            ElementContent::Text("Before".into())
        );
        dispatch(&mut shared.lock_recover(), "review.view:unified");
    }
}

#[test]
fn viewed_toggle_updates_progress_without_hiding_the_patch() {
    for width in [800.0, 1280.0] {
        let shared = fixture();
        let mut harness = TestHarness::new(
            include_str!("../../../assets/styles.css"),
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
            include_str!("../../../assets/styles.css"),
            || tree(&shared),
            width,
            720.0,
        );
        harness.step();
        assert!(harness.query(".terminal-grid").is_none());
        let summary = harness.query(".diff-summary").unwrap();
        let ElementContent::Text(text) = summary.content else {
            panic!("Missing range summary")
        };
        let report = shared
            .lock_recover()
            .diff_review
            .as_ref()
            .unwrap()
            .report
            .clone()
            .unwrap();
        assert!(text.contains(&report.base) && text.contains(&report.head));
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
fn compare_controls_stay_visible_and_keep_independent_ref_inputs() {
    for width in [800.0, 1280.0] {
        let shared = fixture();
        let mut harness = TestHarness::new(
            include_str!("../../../assets/styles.css"),
            || tree(&shared),
            width,
            720.0,
        );
        harness.step();
        harness.locator_by_text("Compare branches").click();
        harness.rebuild(|| tree(&shared));
        harness.step();
        for selector in ["#diff-from", "#diff-to"] {
            let field = harness.query(selector).expect(selector).layout_rect;
            assert!(field.width >= 160.0 && field.height >= 28.0, "{field:?}");
            assert!(
                field.x >= 0.0 && field.x + field.width <= width,
                "{field:?}"
            );
        }
        harness.locator("#diff-from").fill("release/old");
        harness.locator("#diff-to").fill("origin/release/new");
        let request = shared.lock_recover().diff_review.as_ref().unwrap().request;
        harness.locator("#diff-to").press("Enter");
        assert_ne!(
            shared.lock_recover().diff_review.as_ref().unwrap().request,
            request
        );
        // Changing modes must remount the appropriate live input buffers.
        harness.locator_by_text("Compare base").click();
        harness.rebuild(|| tree(&shared));
        harness.step();
        harness.locator("#diff-base").fill("trunk");
        harness.locator_by_text("Compare branches").click();
        harness.rebuild(|| tree(&shared));
        harness.step();
        harness.locator("#diff-from").expect_value("release/old");
        harness
            .locator("#diff-to")
            .expect_value("origin/release/new");
        harness.locator_by_text("Compare base").click();
        harness.rebuild(|| tree(&shared));
        harness.step();
        harness.locator("#diff-base").expect_value("trunk");
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
            include_str!("../../../assets/styles.css"),
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
                harness.query_all(".diff-line").len() + harness.query_all(".diff-split-row").len()
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
        include_str!("../../../assets/styles.css"),
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
            include_str!("../../../assets/styles.css"),
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
        include_str!("../../../assets/styles.css"),
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
    if let Some(patch) = std::env::var_os("TM_DIFF_VISUAL_PATCH") {
        import_fixture(&shared, std::path::Path::new(&patch));
    } else if let Ok(mode) = std::env::var("TM_DIFF_VISUAL_RANGE") {
        let mut state = shared.lock_recover();
        let review = state.diff_review.as_mut().unwrap();
        review.mode = match mode.as_str() {
            "base" => "base",
            "branches" => "branches",
            _ => "last",
        };
    }
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
        include_str!("../../../assets/styles.css"),
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
