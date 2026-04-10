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
                            ElementDef::new(Tag::Span).with_class("crumb").with_text("main"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("crumb-sep").with_text("/"),
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
                            ElementDef::new(Tag::Span).with_class("kbd").with_text("\u{2318}K"),
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
