use std::path::Path;
use unshit::core::element::*;
use unshit::core::event::{Event, EventType, Key, KeyEventKind, Modifiers, RequestScrollIntoView};
use unshit::core::style::parse::StyleDeclaration;

use crate::explorer::{Explorer, Listing};
use crate::state::{dispatch, load_explorer_directory, mutate_with, SharedState, UiSnapshot};
use crate::ui::icons::{icon_file, icon_folder, svg_icon};

pub fn command_button(label: &str, command: &'static str, shared: &SharedState) -> ElementDef {
    let shared = shared.clone();
    ElementDef::new(Tag::Button)
        .with_class("explorer-action")
        .with_text(label)
        .on_click(move || {
            mutate_with(&shared, |state| {
                dispatch(state, command);
            })
        })
}

pub fn build_tabs(snapshot: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut workspaces = command_button("Workspaces", "sidebar.workspaces", shared);
    let mut explorer = command_button("Explorer", "explorer.show", shared);
    if snapshot.explorer.active {
        explorer = explorer.with_class("active");
    } else {
        workspaces = workspaces.with_class("active");
    }
    ElementDef::new(Tag::Div)
        .with_class("sidebar-tabs")
        .with_child(workspaces)
        .with_child(explorer)
}

pub fn build_explorer(snapshot: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let explorer = &snapshot.explorer;
    let mut panel = ElementDef::new(Tag::Div).with_class("file-explorer");
    panel = panel.with_child(
        ElementDef::new(Tag::Div)
            .with_class("explorer-toolbar")
            .with_child(command_button("Reveal", "explorer.reveal", shared))
            .with_child(command_button("Refresh", "explorer.refresh", shared))
            .with_child(command_button(
                "Collapse all",
                "explorer.collapse_all",
                shared,
            )),
    );
    let keyboard_state = shared.clone();
    let mut tree = ElementDef::new(Tag::Div)
        .with_class("sidebar-scroll")
        .with_key("explorer-tree")
        .with_tab_index(0)
        .captures_keyboard(explorer.keyboard_focus)
        .with_autofocus(explorer.keyboard_focus)
        .on(EventType::KeyboardCapture, move |event| {
            if let Event::Keyboard(key) = event {
                if key.kind == KeyEventKind::Pressed
                    && (key.modifiers.is_empty()
                        || (key.modifiers == Modifiers::SHIFT && matches!(key.key, Key::Char(_))))
                {
                    let revealed = mutate_with(&keyboard_state, |state| {
                        handle_key(state, key.key)
                            .then(|| state.explorer.selected.clone())
                            .flatten()
                    });
                    if let Some(path) = revealed {
                        return Some(Box::new(RequestScrollIntoView(crate::explorer::row_id(
                            &path,
                        ))));
                    }
                }
            }
            None
        });
    if let Some(root) = &explorer.root {
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy();
        tree = tree.with_child(row(root, &name, true, 0, explorer, shared));
        append_directory(&mut tree, root, 1, explorer, shared);
    } else {
        tree = tree.with_child(message("Select a workspace folder to browse files."));
    }
    panel
        .with_child(tree)
        .with_child(message("Type a name to jump"))
}

fn message(text: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("explorer-message")
        .with_text(text)
}

fn append_directory(
    tree: &mut ElementDef,
    path: &Path,
    depth: usize,
    explorer: &Explorer,
    shared: &SharedState,
) {
    if !explorer.expanded.contains(path) {
        return;
    }
    match explorer.listings.get(path).map(|listing| listing.as_ref()) {
        None | Some(Listing::Loading) => tree.children.push(message("Loading…")),
        Some(Listing::Error(error)) => tree.children.push(message(&format!(
            "Could not read folder: {error}. Use Refresh to retry."
        ))),
        Some(Listing::Ready(entries)) => {
            if entries.is_empty() {
                tree.children.push(message("Empty folder"));
            }
            for entry in entries {
                tree.children.push(row(
                    &entry.path,
                    &entry.name,
                    entry.directory,
                    depth,
                    explorer,
                    shared,
                ));
                if entry.directory {
                    append_directory(tree, &entry.path, depth + 1, explorer, shared);
                }
            }
        }
    }
}

fn row(
    path: &Path,
    name: &str,
    directory: bool,
    depth: usize,
    explorer: &Explorer,
    shared: &SharedState,
) -> ElementDef {
    let expanded = explorer.expanded.contains(path);
    let path = path.to_owned();
    let click_state = shared.clone();
    let context_state = shared.clone();
    let context_path = path.clone();
    let context_root = explorer.root.clone();
    let id = crate::explorer::row_id(&path);
    let mut row = ElementDef::new(Tag::Button)
        .with_id(id.clone())
        .with_key(id)
        .with_class("explorer-row")
        .with_style(StyleDeclaration::PaddingLeft(8.0 + depth as f32 * 16.0));
    if explorer.selected.as_ref() == Some(&path) {
        row = row.with_class("selected");
    }
    row.with_child(
        ElementDef::new(Tag::Span)
            .with_class("explorer-chevron")
            .with_text(if !directory {
                ""
            } else if expanded {
                "⌄"
            } else {
                "›"
            }),
    )
    .with_child(
        svg_icon(if directory {
            icon_folder()
        } else {
            icon_file()
        })
        .with_class(if directory {
            "explorer-folder-icon"
        } else {
            "explorer-file-icon"
        }),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("explorer-name")
            .with_text(name),
    )
    .on_context_menu(move |x, y| {
        if let Some(root) = &context_root {
            mutate_with(&context_state, |state| {
                state.explorer.cancel_pending_reveal();
                state.explorer.clear_typeahead();
                state.explorer.selected = Some(context_path.clone());
                let scale = state.scale_factor.max(1e-3);
                state.ctx_menu = Some(crate::state::CtxMenu {
                    x: x / scale,
                    y: y / scale,
                    target: crate::state::CtxMenuTarget::Explorer {
                        path: context_path.clone(),
                        root: root.clone(),
                        directory,
                    },
                });
            });
        }
    })
    .on_click(move || {
        mutate_with(&click_state, |state| {
            state.explorer.cancel_pending_reveal();
            state.explorer.clear_typeahead();
            state.explorer.keyboard_focus = directory;
            state.explorer.selected = Some(path.clone());
            if directory {
                if !state.explorer.expanded.remove(&path) {
                    state.explorer.expanded.insert(path.clone());
                    load_explorer_directory(state, path.clone());
                }
            } else {
                crate::state::dispatch_editor_open_path(state, &path.to_string_lossy());
            }
        })
    })
}

fn handle_key(state: &mut crate::state::AppState, key: Key) -> bool {
    state.explorer.cancel_pending_reveal();
    if let Key::Char(character) = key {
        return state
            .explorer
            .type_to_select(character, std::time::Instant::now());
    }
    state.explorer.clear_typeahead();
    let Some(path) = state
        .explorer
        .selected
        .clone()
        .or_else(|| state.explorer.root.clone())
    else {
        return false;
    };
    match key {
        Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End => {
            let paths = state.explorer.visible_paths();
            if paths.is_empty() {
                return false;
            }
            let index = paths.iter().position(|p| *p == path).unwrap_or(0);
            let next = match key {
                Key::Home => 0,
                Key::End => paths.len() - 1,
                Key::ArrowUp => index.saturating_sub(1),
                _ => (index + 1).min(paths.len() - 1),
            };
            state.explorer.selected = Some(paths[next].clone());
        }
        Key::ArrowLeft => {
            if !state.explorer.expanded.remove(&path) && Some(&path) != state.explorer.root.as_ref()
            {
                state.explorer.selected = path.parent().map(Path::to_owned);
            }
        }
        Key::ArrowRight | Key::Enter | Key::Space => {
            if state.explorer.is_directory(&path) {
                if key != Key::ArrowRight && state.explorer.expanded.remove(&path) {
                    return true;
                }
                state.explorer.expanded.insert(path.clone());
                load_explorer_directory(state, path);
            } else if key != Key::ArrowRight {
                state.explorer.keyboard_focus = false;
                crate::state::dispatch_editor_open_path(state, &path.to_string_lossy());
            }
        }
        Key::Escape | Key::Tab => state.explorer.keyboard_focus = false,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explorer::Entry;
    use std::sync::{Arc, Mutex};

    #[test]
    fn collapsing_selected_folder_cancels_pending_reveal() {
        let mut state = crate::state::seed_state();
        let root = std::path::PathBuf::from("project");
        state.explorer.set_root(Some(root.clone()));
        state.explorer.selected = Some(root.clone());
        let revision = state.explorer.reveal_revision;
        handle_key(&mut state, Key::ArrowLeft);
        assert_eq!(state.explorer.selected, Some(root.clone()));
        assert!(!state.explorer.expanded.contains(&root));
        assert_ne!(state.explorer.reveal_revision, revision);
    }

    #[test]
    fn keyboard_browses_visible_rows_and_opens_file_in_existing_editor() {
        let root = std::env::temp_dir().join(format!("tm-explorer-ui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        let file = root.join("folder").join("hello.txt");
        std::fs::write(&file, "hello\n").unwrap();
        let mut state = crate::state::seed_state();
        state.workspaces[0].path = Some(root.clone());
        dispatch(&mut state, "explorer.show");
        state.explorer.listings.insert(
            root.clone(),
            Arc::new(Listing::Ready(vec![Entry {
                path: root.join("folder"),
                name: "folder".into(),
                directory: true,
            }])),
        );
        state.explorer.listings.insert(
            root.join("folder"),
            Arc::new(crate::explorer::read_directory(&root.join("folder"))),
        );
        assert!(handle_key(&mut state, Key::ArrowDown));
        assert_eq!(state.explorer.selected, Some(root.join("folder")));
        handle_key(&mut state, Key::ArrowRight);
        handle_key(&mut state, Key::ArrowDown);
        assert_eq!(state.explorer.selected, Some(file.clone()));
        handle_key(&mut state, Key::Enter);
        assert_eq!(state.editors.len(), 1);
        assert!(!state.explorer.keyboard_focus);
        handle_key(&mut state, Key::Enter);
        assert_eq!(
            state.editors.len(),
            1,
            "reopening a file should reuse its editor"
        );
        handle_key(&mut state, Key::ArrowLeft);
        handle_key(&mut state, Key::ArrowLeft);
        assert_eq!(
            state.explorer.visible_paths(),
            vec![root.clone(), root.join("folder")]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_row_click_uses_editor_open_and_releases_tree_keyboard_capture() {
        let file =
            std::env::temp_dir().join(format!("tm-explorer-click-{}.txt", std::process::id()));
        std::fs::write(&file, "file opened by click").unwrap();
        let shared = Arc::new(Mutex::new(crate::state::seed_state()));
        let explorer = Explorer::default();
        let row = row(&file, "example.txt", false, 1, &explorer, &shared);
        row.on_click.unwrap()();
        let state = shared.lock().unwrap();
        assert_eq!(state.editors.len(), 1);
        assert_eq!(state.explorer.selected, Some(file.clone()));
        assert!(!state.explorer.keyboard_focus);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn explorer_hover_moves_between_rows_immediately() {
        use unshit_test::TestHarness;

        let root = std::env::temp_dir().join(format!(
            "tm-explorer-hover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = root.join("alpha.md");
        let second = root.join("beta.md");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&first, "alpha\n").unwrap();
        std::fs::write(&second, "beta\n").unwrap();

        let mut state = crate::state::seed_state();
        state.explorer.set_root(Some(root.clone()));
        state.explorer.listings.insert(
            root.clone(),
            Arc::new(Listing::Ready(vec![
                Entry {
                    path: first.clone(),
                    name: "alpha.md".into(),
                    directory: false,
                },
                Entry {
                    path: second.clone(),
                    name: "beta.md".into(),
                    directory: false,
                },
            ])),
        );
        let shared: SharedState = Arc::new(Mutex::new(state));
        let snapshot = shared.lock().unwrap().ui_snapshot();
        let tree_snapshot = snapshot.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("theme-amber")
                    .with_child(build_explorer(&tree_snapshot, &tree_shared)),
            },
            500.0,
            700.0,
        );

        let rows = harness.query_all(".explorer-row");
        assert!(rows.len() >= 3, "root, alpha, and beta rows");
        let first_rect = rows[1].layout_rect;
        let second_rect = rows[2].layout_rect;
        let base_first = rows[1].computed_style.background.clone();
        let base_second = rows[2].computed_style.background.clone();

        harness.mouse_move(
            first_rect.x + first_rect.width / 2.0,
            first_rect.y + first_rect.height / 2.0,
        );
        harness.step();
        let first_hover = harness.query_all(".explorer-row")[1]
            .computed_style
            .background
            .clone();
        let second_idle = harness.query_all(".explorer-row")[2]
            .computed_style
            .background
            .clone();
        assert_ne!(first_hover, base_first, "first row must highlight on entry");
        assert_eq!(
            second_idle, base_second,
            "hover must not bleed into siblings"
        );

        harness.mouse_move(
            second_rect.x + second_rect.width / 2.0,
            second_rect.y + second_rect.height / 2.0,
        );
        harness.step();
        let rows = harness.query_all(".explorer-row");
        assert_eq!(
            rows[1].computed_style.background, base_first,
            "leaving the first row must clear its hover in the same frame"
        );
        assert_ne!(
            rows[2].computed_style.background, base_second,
            "the second row must highlight immediately"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
