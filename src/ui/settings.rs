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
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_style(StyleDeclaration::FlexGrow(1.0))
        .with_style(StyleDeclaration::FlexBasis(Dimension::Auto))
        .with_style(StyleDeclaration::Overflow(Overflow::Scroll))
        .with_style(StyleDeclaration::MinHeight(Dimension::Px(0.0)))
        .with_child(build_general_section(state, shared))
        .with_child(build_appearance_section(state, shared))
        .with_child(build_shell_section(state, shared))
}

fn build_general_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal-section")
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
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
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
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
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
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
    fn modal_body_has_three_sections() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        assert!(el.classes.contains(&"modal-body".to_string()));
        assert_eq!(el.children.len(), 3);
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
        // title + 3 setting rows (default shell, working directory, restore on startup)
        assert_eq!(el.children.len(), 4);
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
        // children: title, theme row, font row, glow row, bg texture row
        assert_eq!(el.children.len(), 5);
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
    fn shell_section_has_two_setting_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        // title + 2 rows (shell integration, history size)
        assert_eq!(el.children.len(), 3);
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
}
