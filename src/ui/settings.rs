use unshit::core::element::*;

use crate::state::{
    dispatch, is_on, mutate_with, AgentEntry, SettingsSection, SharedState, UiSnapshot, KEYBINDS,
};
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
    let section = match state.settings_section {
        SettingsSection::General => build_general_section(state, shared),
        SettingsSection::Appearance => build_appearance_section(state, shared),
        SettingsSection::Shell => build_shell_section(state, shared),
        SettingsSection::Keybinds => build_keybinds_section(state, shared),
        SettingsSection::Agents => build_agents_section(state, shared),
    };
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_child(section)
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
        .with_child(setting_row(
            "Confirm before closing",
            "Warn when closing a tab with a running process",
            toggle_button(
                is_on(state, "confirm-before-closing"),
                "confirm-before-closing",
                shared,
            ),
        ))
        .with_child(setting_row(
            "Start minimized",
            "Launch to system tray on startup",
            toggle_button(is_on(state, "start-minimized"), "start-minimized", shared),
        ))
        .with_child(setting_row(
            "Check for updates",
            "Notify when a new version is available",
            toggle_button(
                is_on(state, "check-for-updates"),
                "check-for-updates",
                shared,
            ),
        ))
}

fn build_appearance_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    // Theme chips
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

    // Font size stepper
    let font_stepper = build_stepper(
        state.font_size_pt.to_string(),
        "font.dec",
        "font.inc",
        shared,
    );

    // Cursor style group
    let mut cursor_group = ElementDef::new(Tag::Div).with_class("cursor-group");
    for (style, preview_class, label) in [
        ("block", "block-cursor", "block"),
        ("line", "underline-cursor", "line"),
        ("bar", "bar-cursor", "bar"),
    ] {
        let mut opt = ElementDef::new(Tag::Button).with_class("cursor-option");
        if state.cursor_style == style {
            opt = opt.with_class("active");
        }
        let s = shared.clone();
        let cmd = format!("cursor.set:{}", style);
        opt = opt
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("cursor-preview")
                    .with_class(preview_class),
            )
            .with_child(ElementDef::new(Tag::Span).with_text(label))
            .on_click(move || {
                mutate_with(&s, |st| {
                    dispatch(st, &cmd);
                });
            });
        cursor_group = cursor_group.with_child(opt);
    }

    // Opacity slider
    let opacity_val = format!("{}%", state.opacity);
    let opacity_s = shared.clone();
    let slider_control = ElementDef::new(Tag::Div)
        .with_class("slider-control")
        .with_child(
            ElementDef::new(Tag::Input)
                .with_class("slider")
                .with_input_type(InputType::Range)
                .with_min(50.0)
                .with_max(100.0)
                .with_step(1.0)
                .on_change(move |val| {
                    let cmd = format!("opacity.set:{}", val);
                    mutate_with(&opacity_s, |st| {
                        dispatch(st, &cmd);
                    });
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("slider-val")
                .with_class("tnum")
                .with_text(opacity_val),
        );

    // Line height stepper
    let lh_display = format!(
        "{}.{}",
        state.line_height_10x / 10,
        state.line_height_10x % 10
    );
    let lh_stepper = build_stepper(lh_display, "line_height.dec", "line_height.inc", shared);

    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("appearance"),
        )
        .with_child(setting_row(
            "Theme",
            "Visual palette for the entire application",
            theme_chips,
        ))
        .with_child(setting_row(
            "Font size",
            "Terminal output size in points",
            font_stepper,
        ))
        .with_child(setting_row(
            "Cursor style",
            "Terminal cursor appearance",
            cursor_group,
        ))
        .with_child(setting_row(
            "Terminal opacity",
            "Window transparency level",
            slider_control,
        ))
        .with_child(setting_row(
            "Line height",
            "Spacing between terminal rows",
            lh_stepper,
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
            ElementDef::new(Tag::Input)
                .with_class("input")
                .with_class("compact")
                .with_placeholder(" /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}`~?"),
        ))
}

fn build_keybinds_section(_state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut section = ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("keybinds"),
        );

    for (label, keys) in KEYBINDS {
        section = section.with_child(keybind_row(label, keys));
    }

    let s = shared.clone();
    section = section.with_child(
        ElementDef::new(Tag::Div)
            .with_class("keybind-footer")
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("btn")
                    .with_class("ghost")
                    .with_text("reset to defaults")
                    .on_click(move || {
                        // Placeholder: reset keybinds to defaults
                        let _ = &s;
                    }),
            ),
    );

    section
}

fn build_agents_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    // Timeout stepper
    let timeout_stepper = build_stepper(
        state.agent_timeout.to_string(),
        "agent_timeout.dec",
        "agent_timeout.inc",
        shared,
    );

    let agent_count = state.agents.len().to_string();

    let mut section = ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_text("agents"),
        )
        .with_child(setting_row(
            "Auto-discovery",
            "Detect installed AI agents on PATH",
            toggle_button(is_on(state, "auto-discovery"), "auto-discovery", shared),
        ))
        .with_child(setting_row(
            "Default timeout",
            "Seconds before an agent task is canceled",
            timeout_stepper,
        ))
        .with_child(
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
                        .with_text(agent_count),
                ),
        );

    for (i, agent) in state.agents.iter().enumerate() {
        section = section.with_child(agent_row(agent, i, shared));
    }

    section
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn build_stepper(
    display: String,
    dec_cmd: &str,
    inc_cmd: &str,
    shared: &SharedState,
) -> ElementDef {
    let dec_s = shared.clone();
    let inc_s = shared.clone();
    let dec = dec_cmd.to_string();
    let inc = inc_cmd.to_string();
    ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("\u{2212}")
                .on_click(move || {
                    mutate_with(&dec_s, |st| {
                        dispatch(st, &dec);
                    });
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(display),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("+")
                .on_click(move || {
                    mutate_with(&inc_s, |st| {
                        dispatch(st, &inc);
                    });
                }),
        )
}

fn keybind_row(label: &str, keys: &[&str]) -> ElementDef {
    let mut keybind_keys = ElementDef::new(Tag::Div).with_class("keybind-keys");
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            keybind_keys = keybind_keys.with_child(
                ElementDef::new(Tag::Span)
                    .with_class("keybind-sep")
                    .with_text("+"),
            );
        }
        keybind_keys = keybind_keys.with_child(
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
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("setting-label")
                        .with_text(label),
                ),
        )
        .with_child(keybind_keys)
}

fn agent_row(agent: &AgentEntry, index: usize, shared: &SharedState) -> ElementDef {
    let badge = ElementDef::new(Tag::Span)
        .with_class("agent-badge")
        .with_class(agent.status.css_class())
        .with_text(agent.status.label());

    let mut toggle = ElementDef::new(Tag::Button).with_class("toggle");
    if agent.enabled {
        toggle = toggle.with_class("on");
    }
    let s = shared.clone();
    toggle = toggle.on_click(move || {
        mutate_with(&s, |st| {
            if let Some(a) = st.agents.get_mut(index) {
                a.enabled = !a.enabled;
            }
        });
    });

    ElementDef::new(Tag::Div)
        .with_class("agent-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-icon")
                .with_child(ElementDef::new(Tag::Div).with_svg(icon_agent_prompt())),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-info")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-name")
                        .with_text(&agent.name),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-path")
                        .with_text(&agent.path),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-controls")
                .with_child(badge)
                .with_child(toggle),
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SettingsSection};
    use std::sync::{Arc, Mutex};

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
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
        // Should have title row and close button
        assert_eq!(el.children.len(), 2);
        // Close button should have on_click
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
        // First child should have "active" class
        assert!(el.children[0].classes.contains(&"active".to_string()));
        // Others should not
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
        assert!(!el.children[0].classes.contains(&"active".to_string()));
        assert!(!el.children[1].classes.contains(&"active".to_string()));
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
    fn modal_body_renders_one_active_section() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert!(el.classes.contains(&"modal-body".to_string()));
        assert_eq!(el.children.len(), 1);
    }

    // -- build_general_section --------------------------------------------------

    #[test]
    fn general_section_has_correct_class_and_title() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_general_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
        // First child is the section title
        let title = &el.children[0];
        assert!(title.classes.contains(&"modal-section-title".to_string()));
    }

    #[test]
    fn general_section_has_setting_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_general_section(&snap, &shared);
        // title + 6 setting rows
        assert_eq!(el.children.len(), 7);
    }

    // -- build_appearance_section -----------------------------------------------

    #[test]
    fn appearance_section_has_correct_class() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
    }

    #[test]
    fn appearance_section_theme_chips_mark_amber_active() {
        let snap = make_snapshot(); // theme defaults to "amber"
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // title + 8 setting rows
        assert_eq!(el.children.len(), 9);
        // Theme row is children[1], which is a setting_row.
        // setting_row has 2 children: setting-meta and the control (theme-chips).
        let theme_row = &el.children[1];
        let theme_chips = &theme_row.children[1]; // the control element
        assert!(theme_chips.classes.contains(&"theme-chips".to_string()));
        // First chip (amber) should have "active"
        assert!(theme_chips.children[0]
            .classes
            .contains(&"active".to_string()));
        // Others should not
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
        // cyan is the 3rd theme (index 2)
        assert!(!theme_chips.children[0]
            .classes
            .contains(&"active".to_string()));
        assert!(!theme_chips.children[1]
            .classes
            .contains(&"active".to_string()));
        assert!(theme_chips.children[2]
            .classes
            .contains(&"active".to_string()));
        assert!(!theme_chips.children[3]
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
    fn appearance_section_has_font_stepper() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let font_row = &el.children[2];
        let stepper = &font_row.children[1];
        assert!(stepper.classes.contains(&"stepper".to_string()));
        // stepper has 3 children: dec button, value span, inc button
        assert_eq!(stepper.children.len(), 3);
    }

    // -- build_shell_section ----------------------------------------------------

    #[test]
    fn shell_section_has_correct_title() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
        let title = &el.children[0];
        assert!(title.classes.contains(&"modal-section-title".to_string()));
    }

    #[test]
    fn shell_section_has_five_setting_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        // title + 5 rows
        assert_eq!(el.children.len(), 6);
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
        // First child is the hint
        assert!(el.children[0].classes.contains(&"modal-hint".to_string()));
        // Second child is the footer actions
        let actions = &el.children[1];
        assert!(actions
            .classes
            .contains(&"modal-footer-actions".to_string()));
        // actions has cancel and save buttons
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
        assert_eq!(meta.children.len(), 2); // label span + desc span
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

    // -- closure invocation tests (cover on_click bodies) ----------------------

    #[test]
    fn close_button_click_closes_modal() {
        let shared = make_shared();
        // Open the modal first
        shared.lock().unwrap().settings_open = true;
        let el = build_modal_header(&shared);
        let close_btn = &el.children[1];
        // Invoke the on_click closure
        (close_btn.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().settings_open);
    }

    #[test]
    fn nav_item_click_changes_section() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        // Click the Appearance nav item (index 1)
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
        // Click the Shell nav item (index 2)
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
        // Click "green" chip (index 1)
        (theme_chips.children[1].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, "green");
    }

    #[test]
    fn theme_chip_click_changes_to_cyan() {
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        // Click "cyan" chip (index 2)
        (theme_chips.children[2].on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().theme, "cyan");
    }

    #[test]
    fn theme_chip_click_changes_to_mono() {
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_appearance_section(&snap, &shared);
        let theme_chips = &el.children[1].children[1];
        // Click "mono" chip (index 3)
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
        // font.dec should decrease (or clamp at min)
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
        // Initially off
        assert!(!shared
            .lock()
            .unwrap()
            .toggles
            .get("test-toggle")
            .copied()
            .unwrap_or(false));
        // Click to turn on
        (el.on_click.as_ref().unwrap())();
        assert!(shared
            .lock()
            .unwrap()
            .toggles
            .get("test-toggle")
            .copied()
            .unwrap_or(false));
        // Click again to turn off
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

    #[test]
    fn modal_nav_marks_keybinds_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Keybinds, &shared);
        assert!(el.children[3].classes.contains(&"active".to_string()));
        assert!(!el.children[0].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_agents_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Agents, &shared);
        assert!(el.children[4].classes.contains(&"active".to_string()));
        assert!(!el.children[0].classes.contains(&"active".to_string()));
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
        assert!(!theme_chips.children[0]
            .classes
            .contains(&"active".to_string()));
    }

    // -- section switching via build_modal_body --------------------------------

    #[test]
    fn modal_body_renders_general_by_default() {
        let snap = make_snapshot(); // defaults to General
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert!(title.classes.contains(&"modal-section-title".to_string()));
    }

    #[test]
    fn modal_body_renders_keybinds_when_active() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Keybinds;
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert_eq!(el.children.len(), 1);
        let section = &el.children[0];
        assert!(section.classes.contains(&"modal-section".to_string()));
    }

    #[test]
    fn modal_body_renders_agents_when_active() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Agents;
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert_eq!(el.children.len(), 1);
    }

    // -- build_keybinds_section ------------------------------------------------

    #[test]
    fn keybinds_section_has_correct_class() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
    }

    #[test]
    fn keybinds_section_has_correct_children_count() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        // title + 12 keybind rows + footer = 14
        assert_eq!(el.children.len(), 14);
    }

    #[test]
    fn keybinds_section_first_row_is_new_terminal() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        let first_row = &el.children[1]; // after title
        assert!(first_row.classes.contains(&"keybind-row".to_string()));
    }

    #[test]
    fn keybinds_section_has_footer() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        let footer = el.children.last().unwrap();
        assert!(footer.classes.contains(&"keybind-footer".to_string()));
    }

    // -- keybind_row helper ----------------------------------------------------

    #[test]
    fn keybind_row_has_correct_structure() {
        let el = keybind_row("Test", &["Ctrl", "T"]);
        assert!(el.classes.contains(&"keybind-row".to_string()));
        assert_eq!(el.children.len(), 2); // setting-meta + keybind-keys
    }

    #[test]
    fn keybind_row_keys_has_kbd_and_sep() {
        let el = keybind_row("Test", &["Ctrl", "Shift", "D"]);
        let keys_div = &el.children[1];
        assert!(keys_div.classes.contains(&"keybind-keys".to_string()));
        // 3 kbd elements + 2 separators = 5
        assert_eq!(keys_div.children.len(), 5);
        assert!(keys_div.children[0]
            .classes
            .contains(&"keybind-key".to_string()));
        assert!(keys_div.children[1]
            .classes
            .contains(&"keybind-sep".to_string()));
    }

    #[test]
    fn keybind_row_single_key_has_no_separator() {
        let el = keybind_row("Fullscreen", &["F11"]);
        let keys_div = &el.children[1];
        assert_eq!(keys_div.children.len(), 1);
    }

    // -- build_agents_section --------------------------------------------------

    #[test]
    fn agents_section_has_correct_class() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        assert!(el.classes.contains(&"modal-section".to_string()));
    }

    #[test]
    fn agents_section_has_correct_children_count() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        // title + auto-discovery row + timeout row + list header + 3 agent rows = 7
        assert_eq!(el.children.len(), 7);
    }

    #[test]
    fn agents_section_has_list_header() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_agents_section(&snap, &shared);
        let header = &el.children[3]; // after title + 2 setting rows
        assert!(header.classes.contains(&"agent-list-header".to_string()));
    }

    // -- agent_row helper ------------------------------------------------------

    #[test]
    fn agent_row_has_correct_structure() {
        let agent = crate::state::AgentEntry {
            name: "test".to_string(),
            path: "/bin/test".to_string(),
            status: crate::state::AgentStatus::Running,
            enabled: true,
        };
        let shared = make_shared();
        let el = agent_row(&agent, 0, &shared);
        assert!(el.classes.contains(&"agent-row".to_string()));
        // icon + info + controls = 3
        assert_eq!(el.children.len(), 3);
    }

    #[test]
    fn agent_row_badge_matches_status() {
        let agent = crate::state::AgentEntry {
            name: "test".to_string(),
            path: "/bin/test".to_string(),
            status: crate::state::AgentStatus::Idle,
            enabled: true,
        };
        let shared = make_shared();
        let el = agent_row(&agent, 0, &shared);
        let controls = &el.children[2];
        let badge = &controls.children[0];
        assert!(badge.classes.contains(&"agent-badge".to_string()));
        assert!(badge.classes.contains(&"idle".to_string()));
    }

    #[test]
    fn agent_row_toggle_reflects_enabled() {
        let agent = crate::state::AgentEntry {
            name: "test".to_string(),
            path: "/bin/test".to_string(),
            status: crate::state::AgentStatus::Disabled,
            enabled: false,
        };
        let shared = make_shared();
        let el = agent_row(&agent, 0, &shared);
        let controls = &el.children[2];
        let toggle = &controls.children[1];
        assert!(toggle.classes.contains(&"toggle".to_string()));
        assert!(!toggle.classes.contains(&"on".to_string()));
    }

    #[test]
    fn agent_row_toggle_click_toggles_enabled() {
        let shared = make_shared();
        // Use the seeded agents (index 0 = claude, enabled)
        let snap = make_snapshot();
        let el = agent_row(&snap.agents[0], 0, &shared);
        let toggle = &el.children[2].children[1];
        assert!(toggle.on_click.is_some());
        (toggle.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().agents[0].enabled);
    }

    // -- appearance: cursor group ----------------------------------------------

    #[test]
    fn appearance_cursor_group_has_three_options() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // cursor row is children[3] (title, theme, font, cursor)
        let cursor_row = &el.children[3];
        let cursor_group = &cursor_row.children[1];
        assert!(cursor_group.classes.contains(&"cursor-group".to_string()));
        assert_eq!(cursor_group.children.len(), 3);
    }

    #[test]
    fn appearance_cursor_group_marks_block_active() {
        let snap = make_snapshot(); // defaults to "block"
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let cursor_group = &el.children[3].children[1];
        assert!(cursor_group.children[0]
            .classes
            .contains(&"active".to_string()));
        assert!(!cursor_group.children[1]
            .classes
            .contains(&"active".to_string()));
        assert!(!cursor_group.children[2]
            .classes
            .contains(&"active".to_string()));
    }

    #[test]
    fn appearance_cursor_group_marks_bar_active() {
        let mut state = seed_state();
        state.cursor_style = "bar".to_string();
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        let cursor_group = &el.children[3].children[1];
        assert!(!cursor_group.children[0]
            .classes
            .contains(&"active".to_string()));
        assert!(cursor_group.children[2]
            .classes
            .contains(&"active".to_string()));
    }

    // -- appearance: opacity slider --------------------------------------------

    #[test]
    fn appearance_opacity_slider_exists() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // opacity row is children[4] (title, theme, font, cursor, opacity)
        let opacity_row = &el.children[4];
        let slider_control = &opacity_row.children[1];
        assert!(slider_control
            .classes
            .contains(&"slider-control".to_string()));
        assert_eq!(slider_control.children.len(), 2); // input + val span
    }

    // -- appearance: line height stepper ---------------------------------------

    #[test]
    fn appearance_line_height_stepper_exists() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // line height row is children[5]
        let lh_row = &el.children[5];
        let stepper = &lh_row.children[1];
        assert!(stepper.classes.contains(&"stepper".to_string()));
        assert_eq!(stepper.children.len(), 3);
    }

    // -- build_stepper helper --------------------------------------------------

    #[test]
    fn build_stepper_has_three_children() {
        let shared = make_shared();
        let el = build_stepper("42".to_string(), "test.dec", "test.inc", &shared);
        assert!(el.classes.contains(&"stepper".to_string()));
        assert_eq!(el.children.len(), 3);
        // dec btn, value, inc btn
        assert!(el.children[0].classes.contains(&"stepper-btn".to_string()));
        assert!(el.children[1].classes.contains(&"stepper-val".to_string()));
        assert!(el.children[2].classes.contains(&"stepper-btn".to_string()));
    }
}
