//! Rendered Markdown side preview for editor panes.
//!
//! `editor_pane` builds the source grid; [`attach`] wraps it with the
//! Markdown toolbar and, while the preview is open, the rendered document
//! beside it.

use unshit::core::element::*;
use unshit::core::event::{Event, EventType};

use super::editor_pane::wheel_scroll_patch;
use crate::markdown::{MarkdownBlock, MarkdownDocument, MarkdownView, MAX_PREVIEW_BLOCKS};
use crate::state::{mutate_with, PaneId, SharedState};

/// Element id of the preview's scroll container. The source grid's wheel
/// handler targets it to keep both panes at the same vertical position.
pub fn body_id(pane_id: PaneId) -> String {
    format!("markdown-preview-body-{}", pane_id.0)
}

/// Add the Markdown toolbar to `body`, followed by the source grid, with the
/// rendered preview beside it when the view is open.
pub fn attach(
    body: ElementDef,
    grid_el: ElementDef,
    view: &MarkdownView,
    pane_id: PaneId,
    shared: &SharedState,
) -> ElementDef {
    let body = body
        .with_child(build_toolbar(shared, matches!(view, MarkdownView::Open(_))))
        .with_class("has-markdown-toolbar");
    match view {
        MarkdownView::Closed => body.with_child(grid_el),
        MarkdownView::Open(document) => body
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("markdown-split-row")
                    .with_key("markdown-split-row")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("editor-source")
                            .with_key("editor-source")
                            .with_child(grid_el),
                    )
                    .with_child(build_preview(document, pane_id, shared)),
            )
            .with_class("has-markdown-preview"),
    }
}

fn build_toolbar(shared: &SharedState, preview_open: bool) -> ElementDef {
    let preview_state = shared.clone();
    let action = if preview_open {
        "Hide preview"
    } else {
        "Show preview"
    };
    ElementDef::new(Tag::Div)
        .with_class("editor-toolbar")
        .with_key("markdown-toolbar")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("editor-toolbar-title")
                .with_text("Markdown"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("editor-toolbar-button")
                .with_text(action)
                .on_click(move || {
                    mutate_with(&preview_state, |st| {
                        crate::state::dispatch(st, "editor.markdown_preview.toggle");
                    });
                }),
        )
}

fn build_preview(document: &MarkdownDocument, pane_id: PaneId, shared: &SharedState) -> ElementDef {
    let scroll_shared = shared.clone();
    let mut body = ElementDef::new(Tag::Div)
        .with_class("markdown-body")
        .with_id(body_id(pane_id))
        .with_key(body_id(pane_id))
        .on(
            EventType::Scroll,
            move |event: &Event| -> Option<Box<dyn std::any::Any>> {
                let Event::Scroll(se) = event else {
                    return None;
                };
                // The preview scrolls natively; the source grid follows.
                let patch = wheel_scroll_patch(&scroll_shared, pane_id, se, None);
                Some(Box::new(patch.unwrap_or_default()))
            },
        );
    append_blocks(&document.blocks, &mut body, pane_id, shared);
    if document.truncated {
        body = body.with_child(
            ElementDef::new(Tag::Div)
                .with_class("markdown-truncated")
                .with_text(format!(
                    "Preview shows the first {MAX_PREVIEW_BLOCKS} blocks."
                )),
        );
    }
    ElementDef::new(Tag::Div)
        .with_class("markdown-preview")
        .with_key("markdown-preview")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("markdown-preview-header")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("markdown-preview-title")
                        .with_text("Preview"),
                ),
        )
        .with_child(body)
}

fn append_blocks(
    blocks: &[MarkdownBlock],
    parent: &mut ElementDef,
    pane_id: PaneId,
    shared: &SharedState,
) {
    for block in blocks {
        let child = match block {
            MarkdownBlock::Heading {
                level,
                text,
                source_line,
                anchor,
            } => {
                let heading_shared = shared.clone();
                let target_line = source_line + 1;
                ElementDef::new(Tag::Div)
                    .with_class("markdown-heading")
                    .with_class(format!("markdown-h{level}"))
                    .with_id(format!("markdown-anchor-{}-{anchor}", pane_id.0))
                    .with_key(format!("markdown-anchor-{anchor}"))
                    .with_text(text.clone())
                    .on_click(move || {
                        mutate_with(&heading_shared, |st| {
                            if let Some(editor) = st.editors.get_mut(&pane_id.0) {
                                editor.goto_line(target_line, None);
                            }
                        });
                    })
            }
            MarkdownBlock::Paragraph(text) => ElementDef::new(Tag::Div)
                .with_class("markdown-paragraph")
                .with_text(text.clone()),
            MarkdownBlock::Code { language, text } => ElementDef::new(Tag::Div)
                .with_class("markdown-code")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("markdown-code-language")
                        .with_text(if language.is_empty() {
                            "text"
                        } else {
                            language.as_str()
                        }),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("markdown-code-body")
                        .with_text(text.clone()),
                ),
            MarkdownBlock::Quote(blocks) => {
                let mut quote = ElementDef::new(Tag::Div).with_class("markdown-quote");
                append_blocks(blocks, &mut quote, pane_id, shared);
                quote
            }
            MarkdownBlock::List { ordered, items } => {
                let mut list = ElementDef::new(Tag::Div).with_class("markdown-list");
                if *ordered {
                    list = list.with_class("ordered");
                }
                for (index, item_blocks) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", index + 1)
                    } else {
                        "•".to_string()
                    };
                    let mut content = ElementDef::new(Tag::Div).with_class("markdown-list-content");
                    append_blocks(item_blocks, &mut content, pane_id, shared);
                    list = list.with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("markdown-list-item")
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("markdown-list-marker")
                                    .with_text(marker),
                            )
                            .with_child(content),
                    );
                }
                list
            }
        };
        parent.children.push(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;
    use crate::ui::editor_pane::build_editor_pane_body;
    use unshit_test::TestHarness;

    struct Fixture {
        shared: SharedState,
        pane_id: PaneId,
        view: MarkdownView,
        grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
        path: std::path::PathBuf,
    }

    impl Fixture {
        fn open(tag: &str, source: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "tm-editor-{tag}-{}-{}.md",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::write(&path, source).unwrap();
            let mut state = seed_state();
            crate::state::dispatch(&mut state, &format!("editor.open:{}", path.display()));
            let shared: SharedState = std::sync::Arc::new(std::sync::Mutex::new(state));
            let snapshot = shared.lock().unwrap().ui_snapshot();
            let pane_id = snapshot.active_pane;
            let view = snapshot.markdown_panes.get(&pane_id.0).cloned().unwrap();
            let grids = shared
                .lock()
                .unwrap()
                .editors
                .iter()
                .map(|(&id, editor)| (id, editor.grid.clone()))
                .collect();
            Self {
                shared,
                pane_id,
                view,
                grids,
                path,
            }
        }

        fn body(&self) -> ElementDef {
            build_editor_pane_body(
                self.pane_id,
                true,
                13,
                None,
                Some(&self.view),
                &self.shared,
                &self.grids,
            )
        }

        fn harness(&self) -> TestHarness {
            let (pane_id, view) = (self.pane_id, self.view.clone());
            let (shared, grids) = (self.shared.clone(), self.grids.clone());
            TestHarness::new(
                include_str!("../../assets/styles.css"),
                move || ElementTree {
                    root: ElementDef::new(Tag::Div)
                        .with_class("app")
                        .with_class("theme-amber")
                        .with_child(build_editor_pane_body(
                            pane_id,
                            true,
                            13,
                            None,
                            Some(&view),
                            &shared,
                            &grids,
                        )),
                },
                1000.0,
                620.0,
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn find_by_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if el.classes.iter().any(|candidate| candidate == class) {
            return Some(el);
        }
        el.children
            .iter()
            .find_map(|child| find_by_class(child, class))
    }

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    fn scroll_fixture() -> Fixture {
        let mut source = "# Top\n\n".to_string();
        for index in 0..100 {
            source.push_str(&format!(
                "Paragraph {index} with enough text to occupy preview height.\n\n"
            ));
        }
        Fixture::open("markdown-scroll", &source)
    }

    #[test]
    fn markdown_editor_renders_document_preview_beside_source() {
        let fixture = Fixture::open("pane-preview", "# Heading\n\nParagraph\n");
        let body = fixture.body();

        assert_eq!(body.children.len(), 2, "toolbar and split row");
        assert!(has_class(&body.children[0], "editor-toolbar"));
        let split = &body.children[1];
        assert!(has_class(split, "markdown-split-row"));
        assert_eq!(split.children.len(), 2, "source and preview");
        assert!(has_class(&split.children[0], "editor-source"));
        assert!(has_class(&split.children[1], "markdown-preview"));
    }

    #[test]
    fn clicking_markdown_heading_moves_source_cursor_to_its_line() {
        let mut source = String::new();
        for index in 0..30 {
            source.push_str(&format!("Intro line {index}\n"));
        }
        source.push_str("\n# Target\n\nBody\n");
        let fixture = Fixture::open("heading-click", &source);
        let body = fixture.body();

        let heading = find_by_class(&body, "markdown-heading").expect("preview heading");
        heading.on_click.as_ref().expect("heading click handler")();
        let guard = fixture.shared.lock().unwrap();
        let editor = guard.editors.get(&fixture.pane_id.0).expect("editor");
        assert_eq!(editor.buffer.cursor().line, 31);
        assert!(editor.top_line > 0);
    }

    #[test]
    fn scrolling_preview_drives_markdown_source_grid() {
        let fixture = scroll_fixture();
        let mut harness = fixture.harness();
        let preview = harness.query(".markdown-body").expect("preview body");
        let x = preview.layout_rect.x + preview.layout_rect.width / 2.0;
        let y = preview.layout_rect.y + preview.layout_rect.height / 2.0;

        harness.mouse_move(x, y);
        harness.mouse_wheel(x, y, 0.0, -120.0);

        assert!(
            harness.query(".markdown-body").unwrap().scroll_y > 0.0,
            "native preview scrolling must still run"
        );
        let guard = fixture.shared.lock().unwrap();
        let editor = guard.editors.get(&fixture.pane_id.0).expect("editor");
        assert!(
            editor.top_line > 0,
            "preview wheel must drive the source grid through a paint patch"
        );
    }

    #[test]
    fn scrolling_source_drives_markdown_preview() {
        let fixture = scroll_fixture();
        let mut harness = fixture.harness();
        let source = harness.query(".editor-content").expect("source grid");
        let x = source.layout_rect.x + source.layout_rect.width / 2.0;
        let y = source.layout_rect.y + source.layout_rect.height / 2.0;

        harness.mouse_move(x, y);
        harness.mouse_wheel(x, y, 0.0, -120.0);

        let guard = fixture.shared.lock().unwrap();
        let editor = guard.editors.get(&fixture.pane_id.0).expect("editor");
        assert!(editor.top_line > 0, "source wheel must scroll the grid");
        drop(guard);
        assert!(
            harness.query(".markdown-body").unwrap().scroll_y > 0.0,
            "the companion scroll patch must move the preview without a rebuild"
        );
    }
}
