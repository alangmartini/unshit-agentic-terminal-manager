use unshit::core::element::*;

use crate::state::{dispatch, is_on, mutate_with, SettingsSection, SharedState, UiSnapshot};
use crate::ui::icons::*;

pub fn build_settings_modal(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_child(build_modal_header(shared))
        .with_child(build_modal_nav(state.settings_section, shared))
        .with_child(build_modal_body(state, shared))
        .with_child(build_modal_footer(shared))
}

fn build_modal_header(shared: &SharedState) -> ElementDef {
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-title-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("modal-mark")
                        .with_text("\u{25C6}"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("modal-title")
                        .with_id("settings-title")
                        .with_text("settings"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("settings-close")
                .on_click(move || {
                    mutate_with(&close_state, |st| dispatch(st, "modal.close"));
                })
                .with_child(ElementDef::new(Tag::Div).with_svg(icon_close())),
        )
}

fn build_modal_nav(active: SettingsSection, shared: &SharedState) -> ElementDef {
    let mut nav = ElementDef::new(Tag::Div).with_class("modal-nav");
    for section in SettingsSection::all() {
        let mut item = ElementDef::new(Tag::Button)
            .with_class("modal-nav-item")
            .with_text(section.label());
        if section == active {
            item = item.with_class("active");
        }
        let s = shared.clone();
        let target = section;
        item = item.on_click(move || {
            mutate_with(&s, |st| st.settings_section = target);
        });
        nav = nav.with_child(item);
    }
    nav
}

fn build_modal_body(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_child(build_general_section(state, shared))
        .with_child(build_appearance_section(state, shared))
        .with_child(build_shell_section(state, shared))
}

fn build_general_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("general"),
        )
        .with_child(setting_row(
            "Default shell",
            "Command run when opening a new terminal",
            ElementDef::new(Tag::Div)
                .with_class("input")
                .with_class("select")
                .with_text("bash"),
        ))
        .with_child(setting_row(
            "Working directory",
            "Starting directory for new terminals",
            ElementDef::new(Tag::Input)
                .with_class("input")
                .with_placeholder("~/projects/main"),
        ))
        .with_child(setting_row(
            "Restore on startup",
            "Reopen last active session and panes",
            toggle_button(
                is_on(state, "restore-on-startup"),
                "restore-on-startup",
                shared,
            ),
        ))
}

fn build_appearance_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut theme_chips = ElementDef::new(Tag::Div).with_class("theme-chips");
    for theme in ["amber", "green", "cyan", "mono"] {
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("theme-chip")
            .with_class(theme);
        if state.theme == theme {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let theme_name = theme.to_string();
        chip = chip.on_click(move || {
            mutate_with(&s, |st| st.theme = theme_name.clone());
        });
        theme_chips = theme_chips.with_child(chip);
    }

    let dec_state = shared.clone();
    let inc_state = shared.clone();
    let stepper = ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("\u{2212}")
                .on_click(move || {
                    mutate_with(&dec_state, |st| dispatch(st, "font.dec"));
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(state.font_size_pt.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("+")
                .on_click(move || {
                    mutate_with(&inc_state, |st| dispatch(st, "font.inc"));
                }),
        );

    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("appearance"),
        )
        .with_child(setting_row("Theme", "Visual palette", theme_chips))
        .with_child(setting_row("Font size", "Terminal output size", stepper))
        .with_child(setting_row(
            "Glow effect",
            "Subtle CRT-style text shadow",
            toggle_button(is_on(state, "glow-effect"), "glow-effect", shared),
        ))
        .with_child(setting_row(
            "Background texture",
            "Warm ambient gradient",
            toggle_button(
                is_on(state, "background-texture"),
                "background-texture",
                shared,
            ),
        ))
}

fn build_shell_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("shell"),
        )
        .with_child(setting_row(
            "Shell integration",
            "Inject prompt markers for smart scrollback",
            toggle_button(
                is_on(state, "shell-integration"),
                "shell-integration",
                shared,
            ),
        ))
        .with_child(setting_row(
            "History size",
            "Lines retained per pane",
            ElementDef::new(Tag::Input)
                .with_class("input")
                .with_placeholder("50000"),
        ))
}

fn build_modal_footer(shared: &SharedState) -> ElementDef {
    let cancel_state = shared.clone();
    let save_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-footer")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("modal-hint")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("kbd")
                        .with_text("esc"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("modal-hint-text")
                        .with_text(" close"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-footer-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("ghost")
                        .with_id("settings-cancel")
                        .with_text("cancel")
                        .on_click(move || {
                            mutate_with(&cancel_state, |st| dispatch(st, "modal.close"));
                        }),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("primary")
                        .with_text("save changes")
                        .on_click(move || {
                            mutate_with(&save_state, |st| dispatch(st, "modal.close"));
                        }),
                ),
        )
}

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("setting-meta")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("setting-label")
                        .with_text(label),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("setting-desc")
                        .with_text(desc),
                ),
        )
        .with_child(control)
}

fn toggle_button(on: bool, key: &str, shared: &SharedState) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("toggle");
    if on {
        btn = btn.with_class("on");
    }
    let key_owned = key.to_string();
    let s = shared.clone();
    btn.on_click(move || {
        mutate_with(&s, |st| {
            let next = !st.toggles.get(&key_owned).copied().unwrap_or(false);
            st.toggles.insert(key_owned.clone(), next);
        });
    })
}
