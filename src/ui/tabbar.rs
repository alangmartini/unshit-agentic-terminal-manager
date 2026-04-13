use unshit::core::element::*;

use crate::state::{
    dispatch, mutate_close_tab, mutate_with, SharedState, TabStatus, TerminalTab, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_tabbar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut tabs = ElementDef::new(Tag::Div).with_class("tabs").with_id("tabs");
    for (index, tab) in state.tabs.iter().enumerate() {
        tabs = tabs.with_child(build_tab(index, tab, index == state.active_tab, shared));
    }
    let add_state = shared.clone();
    tabs = tabs.with_child(
        ElementDef::new(Tag::Button)
            .with_class("tab-add")
            .on_click(move || {
                mutate_with(&add_state, |st| dispatch(st, "tab.new"));
            })
            .with_child(svg_icon(icon_plus())),
    );

    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let settings_state = shared.clone();
    let actions = ElementDef::new(Tag::Div)
        .with_class("tabbar-actions")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-h")
                .on_click(move || {
                    mutate_with(&split_h_state, |st| dispatch(st, "pane.split_right"));
                })
                .with_child(svg_icon(icon_split_h())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-v")
                .on_click(move || {
                    mutate_with(&split_v_state, |st| dispatch(st, "pane.split_down"));
                })
                .with_child(svg_icon(icon_split_v())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-grid")
                .with_child(svg_icon(icon_grid())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-balance")
                .with_child(svg_icon(icon_balance())),
        )
        .with_child(ElementDef::new(Tag::Div).with_class("tabbar-divider"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-settings")
                .on_click(move || {
                    mutate_with(&settings_state, |st| dispatch(st, "modal.open"));
                })
                .with_child(svg_icon(icon_settings())),
        );

    ElementDef::new(Tag::Div)
        .with_class("tabbar")
        .with_child(tabs)
        .with_child(actions)
}

fn build_tab(index: usize, tab: &TerminalTab, is_active: bool, shared: &SharedState) -> ElementDef {
    let status_class = match tab.status {
        TabStatus::Running => "running",
        TabStatus::Idle => "idle",
        TabStatus::Stopped => "stopped",
    };

    let mut btn = ElementDef::new(Tag::Button).with_class("tab");
    if is_active {
        btn = btn.with_class("active");
    }
    let activate_state = shared.clone();
    btn = btn.on_click(move || {
        mutate_with(&activate_state, |st| {
            dispatch(st, &format!("tab.switch:{}", index));
        });
    });

    let close_state = shared.clone();
    btn.with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-status")
            .with_class(status_class.to_string()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-name")
            .with_text(tab.name.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-subtitle")
            .with_text(tab.subtitle.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-close")
            .with_text("\u{00D7}")
            .on_click(move || {
                mutate_with(&close_state, |st| mutate_close_tab(st, index));
            }),
    )
}
