use unshit::core::element::*;

use crate::state::{dispatch, mutate_with, SharedState};
use crate::ui::icons::*;

pub fn build_titlebar(shared: &SharedState) -> ElementDef {
    let search_state = shared.clone();
    let sidebar_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("titlebar")
        .with_class("role-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-left")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("brand")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-mark")
                                .with_child(svg_icon(icon_brand_chevron())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-name")
                                .with_text("terminal.mgr"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-version")
                                .with_text("v0.1.0"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("titlebar-breadcrumb")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_text("main"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb-sep")
                                .with_text("/"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_class("active")
                                .with_text("shell"),
                        ),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-right")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pill-btn")
                        .on_click(move || {
                            mutate_with(&search_state, |st| dispatch(st, "palette.toggle"));
                        })
                        .with_child(svg_icon(icon_search()))
                        .with_child(ElementDef::new(Tag::Span).with_text("search"))
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("kbd")
                                .with_text("\u{2318}K"),
                        ),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("titlebar-divider"))
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .on_click(move || {
                            mutate_with(&sidebar_state, |st| dispatch(st, "sidebar.toggle"));
                        })
                        .with_child(svg_icon(icon_sidebar_toggle())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .with_child(svg_icon(icon_fullscreen_corners())),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState};
    use std::sync::{Arc, Mutex};

    fn test_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    #[test]
    fn build_titlebar_does_not_panic() {
        let shared = test_shared();
        let _elem = build_titlebar(&shared);
    }

    #[test]
    fn build_titlebar_returns_div() {
        let shared = test_shared();
        let elem = build_titlebar(&shared);
        assert!(matches!(elem.tag, Tag::Div));
    }

    #[test]
    fn build_titlebar_has_children() {
        let shared = test_shared();
        let elem = build_titlebar(&shared);
        // Should have at least titlebar-left and titlebar-right children
        assert!(
            elem.children.len() >= 2,
            "titlebar should have at least 2 children, got {}",
            elem.children.len(),
        );
    }

    #[test]
    fn titlebar_has_correct_classes() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        assert!(el.classes.contains(&"titlebar".to_string()));
        assert!(el.classes.contains(&"role-header".to_string()));
    }

    #[test]
    fn titlebar_left_has_brand_and_breadcrumb() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let left = &el.children[0];
        assert!(left.classes.contains(&"titlebar-left".to_string()));
        // brand + breadcrumb
        assert_eq!(left.children.len(), 2);
        assert!(left.children[0].classes.contains(&"brand".to_string()));
        assert!(left.children[1]
            .classes
            .contains(&"titlebar-breadcrumb".to_string()));
    }

    #[test]
    fn titlebar_right_has_buttons() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let right = &el.children[1];
        assert!(right.classes.contains(&"titlebar-right".to_string()));
        // pill-btn (search), divider, sidebar toggle, fullscreen = 4
        assert_eq!(right.children.len(), 4);
    }

    #[test]
    fn search_button_click_toggles_palette() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let right = &el.children[1];
        let search_btn = &right.children[0];
        assert!(search_btn.classes.contains(&"pill-btn".to_string()));
        assert!(search_btn.on_click.is_some());
        // Invoke the click handler
        (search_btn.on_click.as_ref().unwrap())();
        assert!(shared.lock().unwrap().palette_open);
    }

    #[test]
    fn sidebar_toggle_click_toggles_sidebar() {
        let shared = test_shared();
        let initial = shared.lock().unwrap().sidebar_collapsed;
        let el = build_titlebar(&shared);
        let right = &el.children[1];
        // sidebar toggle is at index 2 (after search btn and divider)
        let sidebar_btn = &right.children[2];
        assert!(sidebar_btn.on_click.is_some());
        (sidebar_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().sidebar_collapsed;
        assert_ne!(initial, after);
    }

    #[test]
    fn brand_has_mark_name_and_version() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let brand = &el.children[0].children[0];
        assert_eq!(brand.children.len(), 3);
        assert!(brand.children[0]
            .classes
            .contains(&"brand-mark".to_string()));
        assert!(brand.children[1]
            .classes
            .contains(&"brand-name".to_string()));
        assert!(brand.children[2]
            .classes
            .contains(&"brand-version".to_string()));
    }

    #[test]
    fn breadcrumb_has_crumbs_and_separator() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let breadcrumb = &el.children[0].children[1];
        assert_eq!(breadcrumb.children.len(), 3);
        assert!(breadcrumb.children[0]
            .classes
            .contains(&"crumb".to_string()));
        assert!(breadcrumb.children[1]
            .classes
            .contains(&"crumb-sep".to_string()));
        assert!(breadcrumb.children[2]
            .classes
            .contains(&"crumb".to_string()));
        assert!(breadcrumb.children[2]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn fullscreen_button_has_no_click_handler() {
        let shared = test_shared();
        let el = build_titlebar(&shared);
        let right = &el.children[1];
        let fullscreen_btn = &right.children[3];
        assert!(fullscreen_btn.classes.contains(&"icon-btn".to_string()));
        // Fullscreen has no on_click attached
        assert!(fullscreen_btn.on_click.is_none());
    }
}
