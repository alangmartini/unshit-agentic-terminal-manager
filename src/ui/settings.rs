use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{Dimension, FlexDirection, Overflow};

use crate::state::{dispatch, is_on, mutate_with, SettingsSection, SharedState, UiSnapshot};
use crate::ui::icons::*;

pub fn build_settings_modal(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_style(StyleDeclaration::Width(Dimension::Px(680.0)))
        .with_style(StyleDeclaration::Height(Dimension::Percent(80.0)))
        .with_style(StyleDeclaration::MaxHeight(Dimension::Percent(80.0)))
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
    let section = match state.settings_section {
        SettingsSection::General => build_general_section(state, shared),
        SettingsSection::Appearance => build_appearance_section(state, shared),
        SettingsSection::Shell => build_shell_section(state, shared),
        SettingsSection::Keybinds => build_keybinds_section(),
        SettingsSection::Agents => build_agents_section(state, shared),
    };
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_style(StyleDeclaration::FlexGrow(1.0))
        .with_style(StyleDeclaration::FlexBasis(Dimension::Auto))
        .with_style(StyleDeclaration::Overflow(Overflow::Scroll))
        .with_style(StyleDeclaration::MinHeight(Dimension::Px(0.0)))
        .with_child(section)
}

// -- section builders -------------------------------------------------------

fn build_general_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("general")
        .with_child(setting_row(
            "Default shell",
            "Command run when opening a new terminal",
            select_display("bash"),
        ))
        .with_child(setting_row(
            "Working directory",
            "Starting directory for new terminals",
            text_input_display("~/projects/main"),
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
        .with_child(setting_row(
            "Confirm before closing",
            "Warn when closing a tab with a running process",
            toggle_button(is_on(state, "confirm-close"), "confirm-close", shared),
        ))
        .with_child(setting_row(
            "Start minimized",
            "Launch to system tray on startup",
            toggle_button(is_on(state, "start-minimized"), "start-minimized", shared),
        ))
        .with_child(setting_row(
            "Check for updates",
            "Notify when a new version is available",
            toggle_button(is_on(state, "check-updates"), "check-updates", shared),
        ))
}

fn build_appearance_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("appearance")
        .with_child(setting_row(
            "Theme",
            "Visual palette for the entire application",
            theme_chip_group(state, shared),
        ))
        .with_child(setting_row(
            "Font size",
            "Terminal output size in points",
            font_stepper(state.font_size_pt, shared),
        ))
        .with_child(setting_row(
            "Cursor style",
            "Terminal cursor appearance",
            cursor_style_group(),
        ))
        .with_child(setting_row(
            "Terminal opacity",
            "Window transparency level",
            slider_control("100%"),
        ))
        .with_child(setting_row(
            "Line height",
            "Spacing between terminal rows",
            static_stepper("1.4"),
        ))
        .with_child(setting_row(
            "Glow effect",
            "Subtle CRT-style text shadow on output",
            toggle_button(is_on(state, "glow-effect"), "glow-effect", shared),
        ))
        .with_child(setting_row(
            "Background texture",
            "Warm ambient gradient behind content",
            toggle_button(
                is_on(state, "background-texture"),
                "background-texture",
                shared,
            ),
        ))
        .with_child(setting_row(
            "Font ligatures",
            "Combine character pairs like => and !=",
            toggle_button(is_on(state, "font-ligatures"), "font-ligatures", shared),
        ))
}

fn build_shell_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("shell")
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
            text_input_display("50000"),
        ))
        .with_child(setting_row(
            "Scroll on output",
            "Auto-scroll terminal when new output arrives",
            toggle_button(is_on(state, "scroll-on-output"), "scroll-on-output", shared),
        ))
        .with_child(setting_row(
            "Bell notification",
            "Flash tab badge when terminal rings the bell",
            toggle_button(
                is_on(state, "bell-notification"),
                "bell-notification",
                shared,
            ),
        ))
        .with_child(setting_row(
            "Word separators",
            "Characters that break word selection on double-click",
            compact_input_display(" /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}`~?"),
        ))
}

fn build_keybinds_section() -> ElementDef {
    section_shell("keybinds")
        .with_child(keybind_row("New terminal", &["Ctrl", "T"]))
        .with_child(keybind_row("Close tab", &["Ctrl", "W"]))
        .with_child(keybind_row("Split right", &["Ctrl", "D"]))
        .with_child(keybind_row("Split down", &["Ctrl", "Shift", "D"]))
        .with_child(keybind_row("Next tab", &["Ctrl", "Tab"]))
        .with_child(keybind_row("Previous tab", &["Ctrl", "Shift", "Tab"]))
        .with_child(keybind_row("Command palette", &["Ctrl", "K"]))
        .with_child(keybind_row("Toggle sidebar", &["Ctrl", "B"]))
        .with_child(keybind_row("Settings", &["Ctrl", ","]))
        .with_child(keybind_row("Zoom in", &["Ctrl", "="]))
        .with_child(keybind_row("Zoom out", &["Ctrl", "-"]))
        .with_child(keybind_row("Fullscreen", &["F11"]))
        .with_child(keybind_footer())
}

fn build_agents_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("agents")
        .with_child(setting_row(
            "Auto-discovery",
            "Detect installed AI agents on PATH",
            toggle_button(is_on(state, "auto-discovery"), "auto-discovery", shared),
        ))
        .with_child(setting_row(
            "Default timeout",
            "Seconds before an agent task is canceled",
            static_stepper("300"),
        ))
        .with_child(agent_list_header(3))
        .with_child(agent_row(
            "claude",
            "~/.local/bin/claude",
            "running",
            "running",
            is_on(state, "agent-claude"),
            "agent-claude",
            shared,
        ))
        .with_child(agent_row(
            "amp",
            "~/.local/bin/amp",
            "idle",
            "idle",
            is_on(state, "agent-amp"),
            "agent-amp",
            shared,
        ))
        .with_child(agent_row(
            "codex",
            "~/.local/bin/codex",
            "disabled",
            "disabled",
            is_on(state, "agent-codex"),
            "agent-codex",
            shared,
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

// -- helpers ----------------------------------------------------------------

fn section_shell(title: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text(title),
        )
}

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("setting-meta")
                .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
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

fn select_display(value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("input")
        .with_class("select")
        .with_text(value)
}

fn text_input_display(value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("input")
        .with_text(value)
}

fn compact_input_display(value: &str) -> ElementDef {
    text_input_display(value).with_class("compact")
}

fn theme_chip_group(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut chips = ElementDef::new(Tag::Div).with_class("theme-chips");
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
        chips = chips.with_child(chip);
    }
    chips
}

fn font_stepper(value: u32, shared: &SharedState) -> ElementDef {
    let dec = shared.clone();
    let inc = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("\u{2212}")
                .on_click(move || {
                    mutate_with(&dec, |st| dispatch(st, "font.dec"));
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(value.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("+")
                .on_click(move || {
                    mutate_with(&inc, |st| dispatch(st, "font.inc"));
                }),
        )
}

fn static_stepper(value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("\u{2212}"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(value),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("+"),
        )
}

fn cursor_style_group() -> ElementDef {
    let variants = [
        ("block-cursor", "block", true),
        ("underline-cursor", "line", false),
        ("bar-cursor", "bar", false),
    ];
    let mut group = ElementDef::new(Tag::Div).with_class("cursor-group");
    for (preview_class, label, active) in variants {
        let mut option = ElementDef::new(Tag::Button).with_class("cursor-option");
        if active {
            option = option.with_class("active");
        }
        option = option
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("cursor-preview")
                    .with_class(preview_class),
            )
            .with_child(ElementDef::new(Tag::Span).with_text(label));
        group = group.with_child(option);
    }
    group
}

fn slider_control(value_label: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("slider-control")
        .with_child(ElementDef::new(Tag::Div).with_class("slider"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("slider-val")
                .with_class("tnum")
                .with_text(value_label),
        )
}

fn keybind_row(label: &str, keys: &[&str]) -> ElementDef {
    let mut keys_wrap = ElementDef::new(Tag::Div).with_class("keybind-keys");
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            keys_wrap = keys_wrap.with_child(
                ElementDef::new(Tag::Span)
                    .with_class("keybind-sep")
                    .with_text("+"),
            );
        }
        keys_wrap = keys_wrap.with_child(
            ElementDef::new(Tag::Span)
                .with_class("keybind-key")
                .with_text(*key),
        );
    }
    ElementDef::new(Tag::Div)
        .with_class("keybind-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("setting-meta")
                .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("setting-label")
                        .with_text(label),
                ),
        )
        .with_child(keys_wrap)
}

fn keybind_footer() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("keybind-footer")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("ghost")
                .with_text("reset to defaults"),
        )
}

fn agent_list_header(count: u32) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("agent-list-header")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("agent-list-title")
                .with_text("configured agents"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("agent-list-count")
                .with_text(count.to_string()),
        )
}

fn agent_row(
    name: &str,
    path: &str,
    badge_kind: &str,
    badge_label: &str,
    enabled: bool,
    toggle_key: &str,
    shared: &SharedState,
) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("agent-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-icon")
                .with_child(ElementDef::new(Tag::Div).with_svg(icon_agent())),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-info")
                .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-name")
                        .with_text(name),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-path")
                        .with_text(path),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-controls")
                .with_child(agent_badge(badge_kind, badge_label))
                .with_child(toggle_button(enabled, toggle_key, shared)),
        )
}

fn agent_badge(kind: &str, label: &str) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class("agent-badge")
        .with_class(kind)
        .with_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SettingsSection};
    use std::sync::{Arc, Mutex};
    use unshit::core::element::ElementContent;

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    fn make_snapshot_section(section: SettingsSection) -> UiSnapshot {
        let mut state = seed_state();
        state.settings_section = section;
        state.ui_snapshot()
    }

    fn text_of(el: &ElementDef) -> Option<&str> {
        match &el.content {
            ElementContent::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    // -- build_settings_modal ---------------------------------------------------

    #[test]
    fn settings_modal_has_modal_class() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_settings_modal(&snap, &shared);
        assert!(el.classes.contains(&"modal".to_string()));
    }

    #[test]
    fn settings_modal_has_four_children() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_settings_modal(&snap, &shared);
        // header, nav, body, footer
        assert_eq!(el.children.len(), 4);
    }

    // -- build_modal_header -----------------------------------------------------

    #[test]
    fn modal_header_has_correct_class() {
        let shared = make_shared();
        let el = build_modal_header(&shared);
        assert!(el.classes.contains(&"modal-header".to_string()));
    }

    #[test]
    fn modal_header_contains_title_and_close_button() {
        let shared = make_shared();
        let el = build_modal_header(&shared);
        assert_eq!(el.children.len(), 2);
        let close_btn = &el.children[1];
        assert!(close_btn.on_click.is_some());
        assert_eq!(close_btn.id.as_deref(), Some("settings-close"));
    }

    // -- build_modal_nav --------------------------------------------------------

    #[test]
    fn modal_nav_has_nav_class() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        assert!(el.classes.contains(&"modal-nav".to_string()));
    }

    #[test]
    fn modal_nav_has_five_items() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        assert_eq!(el.children.len(), 5);
    }

    #[test]
    fn modal_nav_marks_general_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        assert!(el.children[0].classes.contains(&"active".to_string()));
        for child in &el.children[1..] {
            assert!(!child.classes.contains(&"active".to_string()));
        }
    }

    #[test]
    fn modal_nav_marks_appearance_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert!(!el.children[0].classes.contains(&"active".to_string()));
        assert!(el.children[1].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_shell_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Shell, &shared);
        assert!(el.children[2].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_keybinds_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Keybinds, &shared);
        assert!(el.children[3].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_agents_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Agents, &shared);
        assert!(el.children[4].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_items_have_click_handlers() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        for child in &el.children {
            assert!(child.on_click.is_some());
        }
    }

    // -- build_modal_body -------------------------------------------------------

    #[test]
    fn modal_body_renders_only_active_section() {
        let snap = make_snapshot_section(SettingsSection::General);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert!(el.classes.contains(&"modal-body".to_string()));
        assert_eq!(el.children.len(), 1);
    }

    #[test]
    fn modal_body_switches_to_appearance() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("appearance"));
    }

    #[test]
    fn modal_body_switches_to_keybinds() {
        let snap = make_snapshot_section(SettingsSection::Keybinds);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("keybinds"));
    }

    #[test]
    fn modal_body_switches_to_agents() {
        let snap = make_snapshot_section(SettingsSection::Agents);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("agents"));
    }

    // -- build_general_section --------------------------------------------------

    #[test]
    fn general_section_has_title_and_six_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_general_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
        // title + 6 rows
        assert_eq!(el.children.len(), 7);
        let title = &el.children[0];
        assert!(title.classes.contains(&"modal-section-title".to_string()));
    }

    // -- build_appearance_section -----------------------------------------------

    #[test]
    fn appearance_section_has_title_and_eight_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // title + 8 rows (theme, font, cursor, opacity, line-height, glow, bg, ligatures)
        assert_eq!(el.children.len(), 9);
    }

    #[test]
    fn appearance_section_theme_chips_mark_amber_active() {
        let snap = make_snapshot(); // theme defaults to "amber"
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let theme_row = &el.children[1];
        let theme_chips = &theme_row.children[1];
        assert!(theme_chips.classes.contains(&"theme-chips".to_string()));
        assert!(theme_chips.children[0]
            .classes
            .contains(&"active".to_string()));
        for chip in &theme_chips.children[1..] {
            assert!(!chip.classes.contains(&"active".to_string()));
        }
    }

    #[test]
    fn appearance_section_theme_chips_mark_cyan_active() {
        let mut state = seed_state();
        state.theme = "cyan".to_string();
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        assert!(theme_chips.children[2]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn appearance_section_theme_chips_mark_green_active() {
        let mut state = seed_state();
        state.theme = "green".to_string();
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        assert!(theme_chips.children[1]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn appearance_section_theme_chips_mark_mono_active() {
        let mut state = seed_state();
        state.theme = "mono".to_string();
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        assert!(theme_chips.children[3]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn appearance_section_has_font_stepper() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let font_row = &el.children[2];
        let stepper = &font_row.children[1];
        assert!(stepper.classes.contains(&"stepper".to_string()));
        assert_eq!(stepper.children.len(), 3);
    }

    #[test]
    fn appearance_section_has_cursor_group() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let cursor_row = &el.children[3];
        let cursor_group = &cursor_row.children[1];
        assert!(cursor_group.classes.contains(&"cursor-group".to_string()));
        assert_eq!(cursor_group.children.len(), 3);
        // First option active by default
        assert!(cursor_group.children[0]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn appearance_section_has_slider_control() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let opacity_row = &el.children[4];
        let slider = &opacity_row.children[1];
        assert!(slider.classes.contains(&"slider-control".to_string()));
        assert_eq!(slider.children.len(), 2);
    }

    // -- build_shell_section ----------------------------------------------------

    #[test]
    fn shell_section_has_title_and_five_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        // title + 5 rows
        assert_eq!(el.children.len(), 6);
    }

    // -- build_keybinds_section -------------------------------------------------

    #[test]
    fn keybinds_section_has_title_twelve_rows_and_footer() {
        let el = build_keybinds_section();
        // title + 12 keybind rows + footer
        assert_eq!(el.children.len(), 14);
    }

    #[test]
    fn keybinds_section_first_row_has_correct_keys() {
        let el = build_keybinds_section();
        let first_row = &el.children[1];
        assert!(first_row.classes.contains(&"keybind-row".to_string()));
        let keys_wrap = &first_row.children[1];
        assert!(keys_wrap.classes.contains(&"keybind-keys".to_string()));
        assert_eq!(keys_wrap.children.len(), 3);
    }

    #[test]
    fn keybinds_section_footer_has_reset_button() {
        let el = build_keybinds_section();
        let footer = el.children.last().unwrap();
        assert!(footer.classes.contains(&"keybind-footer".to_string()));
        let btn = &footer.children[0];
        assert!(btn.classes.contains(&"btn".to_string()));
        assert_eq!(text_of(btn), Some("reset to defaults"));
    }

    // -- build_agents_section ---------------------------------------------------

    #[test]
    fn agents_section_has_expected_children() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        // title + auto-discovery + timeout + header + 3 agent rows
        assert_eq!(el.children.len(), 7);
    }

    #[test]
    fn agents_section_claude_row_has_running_badge() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        let claude_row = &el.children[4];
        assert!(claude_row.classes.contains(&"agent-row".to_string()));
        let controls = &claude_row.children[2];
        let badge = &controls.children[0];
        assert!(badge.classes.contains(&"agent-badge".to_string()));
        assert!(badge.classes.contains(&"running".to_string()));
    }

    #[test]
    fn agents_section_codex_toggle_is_off() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        let codex_row = &el.children[6];
        let controls = &codex_row.children[2];
        let toggle = &controls.children[1];
        assert!(toggle.classes.contains(&"toggle".to_string()));
        assert!(!toggle.classes.contains(&"on".to_string()));
    }

    #[test]
    fn agents_section_claude_toggle_is_on() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        let claude_row = &el.children[4];
        let controls = &claude_row.children[2];
        let toggle = &controls.children[1];
        assert!(toggle.classes.contains(&"on".to_string()));
    }

    #[test]
    fn agents_list_header_has_count() {
        let el = agent_list_header(3);
        assert!(el.classes.contains(&"agent-list-header".to_string()));
        assert_eq!(el.children.len(), 2);
        let count = &el.children[1];
        assert_eq!(text_of(count), Some("3"));
    }

    // -- build_modal_footer -----------------------------------------------------

    #[test]
    fn modal_footer_has_correct_class() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        assert!(el.classes.contains(&"modal-footer".to_string()));
    }

    #[test]
    fn modal_footer_has_hint_and_actions() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0].classes.contains(&"modal-hint".to_string()));
        let actions = &el.children[1];
        assert!(actions
            .classes
            .contains(&"modal-footer-actions".to_string()));
        assert_eq!(actions.children.len(), 2);
    }

    #[test]
    fn modal_footer_cancel_button_has_id() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let cancel = &actions.children[0];
        assert_eq!(cancel.id.as_deref(), Some("settings-cancel"));
        assert!(cancel.on_click.is_some());
    }

    #[test]
    fn modal_footer_save_button_has_click_handler() {
        let shared = make_shared();
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let save = &actions.children[1];
        assert!(save.classes.contains(&"primary".to_string()));
        assert!(save.on_click.is_some());
    }

    // -- setting_row ------------------------------------------------------------

    #[test]
    fn setting_row_has_meta_and_control() {
        let control = ElementDef::new(Tag::Input).with_class("input");
        let el = setting_row("Label", "Description", control);
        assert!(el.classes.contains(&"setting-row".to_string()));
        assert_eq!(el.children.len(), 2);
        let meta = &el.children[0];
        assert!(meta.classes.contains(&"setting-meta".to_string()));
        assert_eq!(meta.children.len(), 2);
    }

    // -- toggle_button ----------------------------------------------------------

    #[test]
    fn toggle_button_on_has_on_class() {
        let shared = make_shared();
        let el = toggle_button(true, "test-key", &shared);
        assert!(el.classes.contains(&"toggle".to_string()));
        assert!(el.classes.contains(&"on".to_string()));
        assert!(el.on_click.is_some());
    }

    #[test]
    fn toggle_button_off_lacks_on_class() {
        let shared = make_shared();
        let el = toggle_button(false, "test-key", &shared);
        assert!(el.classes.contains(&"toggle".to_string()));
        assert!(!el.classes.contains(&"on".to_string()));
        assert!(el.on_click.is_some());
    }

    // -- closure invocation tests ----------------------------------------------

    #[test]
    fn close_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_header(&shared);
        let close_btn = &el.children[1];
        (close_btn.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    #[test]
    fn nav_item_click_changes_section() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        (el.children[1].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Appearance
        );
    }

    #[test]
    fn nav_item_click_changes_to_shell() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        (el.children[2].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Shell
        );
    }

    #[test]
    fn nav_item_click_changes_to_keybinds() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        (el.children[3].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Keybinds
        );
    }

    #[test]
    fn nav_item_click_changes_to_agents() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        (el.children[4].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Agents
        );
    }

    #[test]
    fn theme_chip_click_changes_theme() {
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        (theme_chips.children[1].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, "green");
    }

    #[test]
    fn theme_chip_click_changes_to_cyan() {
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        (theme_chips.children[2].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, "cyan");
    }

    #[test]
    fn theme_chip_click_changes_to_mono() {
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        (theme_chips.children[3].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, "mono");
    }

    #[test]
    fn font_dec_button_decreases_font_size() {
        let shared = make_shared();
        let initial = shared.lock().unwrap().font_size_pt;
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let stepper = &el.children[2].children[1];
        let dec_btn = &stepper.children[0];
        (dec_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().font_size_pt;
        assert!(after <= initial);
    }

    #[test]
    fn font_inc_button_increases_font_size() {
        let shared = make_shared();
        let initial = shared.lock().unwrap().font_size_pt;
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let stepper = &el.children[2].children[1];
        let inc_btn = &stepper.children[2];
        (inc_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().font_size_pt;
        assert!(after >= initial);
    }

    #[test]
    fn toggle_button_click_toggles_state() {
        let shared = make_shared();
        let el = toggle_button(false, "test-toggle", &shared);
        assert!(!shared
            .lock()
            .unwrap()
            .toggles
            .get("test-toggle")
            .copied()
            .unwrap_or(false));
        (el.on_click.as_ref().unwrap())();
        assert!(shared
            .lock()
            .unwrap()
            .toggles
            .get("test-toggle")
            .copied()
            .unwrap_or(false));
        (el.on_click.as_ref().unwrap())();
        assert!(!shared
            .lock()
            .unwrap()
            .toggles
            .get("test-toggle")
            .copied()
            .unwrap_or(false));
    }

    #[test]
    fn cancel_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let cancel = &actions.children[0];
        (cancel.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    #[test]
    fn save_button_click_closes_modal() {
        let shared = make_shared();
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_footer(&shared);
        let actions = &el.children[1];
        let save = &actions.children[1];
        (save.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    // -- helper widget tests ----------------------------------------------------

    #[test]
    fn select_display_has_input_and_select_classes() {
        let el = select_display("bash");
        assert!(el.classes.contains(&"input".to_string()));
        assert!(el.classes.contains(&"select".to_string()));
        assert_eq!(text_of(&el), Some("bash"));
    }

    #[test]
    fn text_input_display_has_input_class() {
        let el = text_input_display("~/path");
        assert!(el.classes.contains(&"input".to_string()));
        assert_eq!(text_of(&el), Some("~/path"));
    }

    #[test]
    fn compact_input_display_has_compact_modifier() {
        let el = compact_input_display("stuff");
        assert!(el.classes.contains(&"input".to_string()));
        assert!(el.classes.contains(&"compact".to_string()));
    }

    #[test]
    fn static_stepper_has_three_children() {
        let el = static_stepper("1.4");
        assert!(el.classes.contains(&"stepper".to_string()));
        assert_eq!(el.children.len(), 3);
        let val = &el.children[1];
        assert_eq!(text_of(val), Some("1.4"));
    }

    #[test]
    fn cursor_style_group_marks_block_active() {
        let el = cursor_style_group();
        assert!(el.classes.contains(&"cursor-group".to_string()));
        assert!(el.children[0].classes.contains(&"active".to_string()));
        assert!(!el.children[1].classes.contains(&"active".to_string()));
        assert!(!el.children[2].classes.contains(&"active".to_string()));
    }

    #[test]
    fn slider_control_has_slider_and_val() {
        let el = slider_control("75%");
        assert!(el.classes.contains(&"slider-control".to_string()));
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0].classes.contains(&"slider".to_string()));
        assert_eq!(text_of(&el.children[1]), Some("75%"));
    }

    #[test]
    fn keybind_row_has_meta_and_keys() {
        let el = keybind_row("Test", &["Ctrl", "A"]);
        assert!(el.classes.contains(&"keybind-row".to_string()));
        assert_eq!(el.children.len(), 2);
        let keys = &el.children[1];
        assert!(keys.classes.contains(&"keybind-keys".to_string()));
        // "Ctrl" + "+" + "A" = 3 children
        assert_eq!(keys.children.len(), 3);
        assert!(keys.children[0]
            .classes
            .contains(&"keybind-key".to_string()));
        assert!(keys.children[1]
            .classes
            .contains(&"keybind-sep".to_string()));
    }

    #[test]
    fn keybind_row_single_key_no_separator() {
        let el = keybind_row("Fullscreen", &["F11"]);
        let keys = &el.children[1];
        assert_eq!(keys.children.len(), 1);
    }

    #[test]
    fn agent_badge_has_kind_class() {
        let el = agent_badge("running", "running");
        assert!(el.classes.contains(&"agent-badge".to_string()));
        assert!(el.classes.contains(&"running".to_string()));
        assert_eq!(text_of(&el), Some("running"));
    }

    #[test]
    fn agent_row_has_icon_info_and_controls() {
        let shared = make_shared();
        let el = agent_row(
            "test",
            "~/bin/test",
            "idle",
            "idle",
            true,
            "agent-test",
            &shared,
        );
        assert!(el.classes.contains(&"agent-row".to_string()));
        assert_eq!(el.children.len(), 3);
        assert!(el.children[0].classes.contains(&"agent-icon".to_string()));
        assert!(el.children[1].classes.contains(&"agent-info".to_string()));
        assert!(el.children[2]
            .classes
            .contains(&"agent-controls".to_string()));
    }
}
