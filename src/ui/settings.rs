use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{Dimension, Display, FlexDirection, Overflow};
use unshit::prelude::SvgNode;

use unshit::core::event::Modifiers;
use unshit::core::shortcut::KeyCombo;

use crate::keybinds::{KeybindAction, KeybindError, KeybindErrorKind};
use crate::state::{
    dispatch, is_on, mutate_with, SettingsSection, SharedState, ToggleKey, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_settings_modal(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_class("set-modal")
        .with_style(StyleDeclaration::Display(Display::Grid))
        .with_style(StyleDeclaration::Width(Dimension::Px(860.0)))
        .with_style(StyleDeclaration::Height(Dimension::Percent(76.0)))
        .with_style(StyleDeclaration::MaxHeight(Dimension::Px(760.0)))
        .with_child(build_modal_header(shared))
        .with_child(build_modal_content(state, shared))
        .with_child(build_modal_footer(shared))
}

pub fn build_settings_page(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("settings-page")
        .with_id("settings-page")
        .with_child(build_settings_page_rail(state.settings_section, shared))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-content")
                .with_child(build_settings_page_header(state.settings_section))
                .with_child(build_settings_page_body(state, shared)),
        )
}

fn build_settings_page_rail(active: SettingsSection, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-page-rail")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-rail-head")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("settings"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("sub")
                        .with_text("v0.4.2"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-search")
                .with_child(svg_icon(icon_search()))
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("set-page-search-input")
                        .with_placeholder("find a setting..."),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("kbd")
                        .with_text("Ctrl F"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-nav")
                .with_child(settings_nav_group("workspace"))
                .with_child(settings_nav_item(
                    SettingsSection::Appearance,
                    active,
                    shared,
                ))
                .with_child(settings_nav_item(SettingsSection::Shell, active, shared))
                .with_child(settings_nav_item(SettingsSection::Sessions, active, shared))
                .with_child(settings_nav_group("automation"))
                .with_child(settings_nav_item(SettingsSection::Keybinds, active, shared))
                .with_child(settings_nav_item(
                    SettingsSection::Notifications,
                    active,
                    shared,
                ))
                .with_child(settings_nav_item(
                    SettingsSection::DangerZone,
                    active,
                    shared,
                )),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-page-foot")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("dot")
                        .with_class("status-running"),
                )
                .with_child(ElementDef::new(Tag::Span).with_text("ptyd up"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("build")
                        .with_text("build 4a2f1c"),
                ),
        )
}

fn settings_nav_group(label: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("group")
        .with_text(label.to_string())
}

fn settings_nav_item(
    section: SettingsSection,
    active: SettingsSection,
    shared: &SharedState,
) -> ElementDef {
    let s = shared.clone();
    let mut item = ElementDef::new(Tag::Button)
        .with_class("set-page-nav-item")
        .with_child(svg_icon(settings_nav_icon(section)))
        .with_child(ElementDef::new(Tag::Span).with_text(section.label()));
    if section == active {
        item = item.with_class("active");
    }
    item.on_click(move || {
        mutate_with(&s, |st| {
            st.settings_section = section;
            if section == SettingsSection::Sessions {
                crate::state::refresh_sessions(st);
            }
        });
    })
}

fn settings_nav_icon(section: SettingsSection) -> SvgNode {
    match section {
        SettingsSection::Appearance => icon_grid(),
        SettingsSection::Shell => icon_terminal(),
        SettingsSection::Keybinds => icon_chevrons(),
        SettingsSection::Sessions => icon_folder(),
        SettingsSection::Notifications => icon_agent(),
        SettingsSection::DangerZone => icon_close(),
    }
}

fn build_settings_page_header(active: SettingsSection) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-page-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("crumb")
                .with_text(format!("settings · {}", active.label())),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("page-title")
                .with_text(active.label()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("blurb")
                .with_text(settings_section_desc(active)),
        )
}

fn settings_section_desc(active: SettingsSection) -> &'static str {
    match active {
        SettingsSection::Appearance => "Font size, sidebar width, preview.",
        SettingsSection::Shell => "Default shell, font, scrollback.",
        SettingsSection::Keybinds => "Every binding, grouped.",
        SettingsSection::Sessions => "Daemon sessions and workspace attachment.",
        SettingsSection::Notifications => "Desktop notifications and focused panes.",
        SettingsSection::DangerZone => "Destructive session and close behavior.",
    }
}

fn build_settings_page_body(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut body = ElementDef::new(Tag::Div)
        .with_class("set-page-body")
        .with_style(StyleDeclaration::Width(Dimension::Percent(100.0)))
        .with_style(StyleDeclaration::MaxWidth(Dimension::Px(820.0)));
    body = match state.settings_section {
        SettingsSection::Appearance => {
            body.with_child(build_appearance_page_section(state, shared))
        }
        SettingsSection::Shell => body.with_child(build_shell_section(state, shared)),
        SettingsSection::Keybinds => body.with_child(build_keybinds_section(state, shared)),
        SettingsSection::Sessions => body.with_child(build_sessions_section(state, shared)),
        SettingsSection::Notifications => body.with_child(build_notifications_section(shared)),
        SettingsSection::DangerZone => body.with_child(build_danger_zone_section(state, shared)),
    };
    body.with_child(build_settings_page_savebar(shared))
}

fn build_appearance_page_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-page-section")
        .with_child(set_card("terminal", None).with_child(settings_page_field(
            "Font size",
            Some("Terminal output size in points."),
            font_stepper(state.font_size_pt, shared),
        )))
        .with_child(set_card("layout", None).with_child(settings_page_field(
            "Sidebar width",
            Some("Width of the workspace sidebar."),
            readout_with_unit(&format!("{:.0}", state.sidebar_width), "px"),
        )))
        .with_child(set_card("preview", None).with_child(build_appearance_preview()))
}

fn set_card(name: &str, meta: Option<&str>) -> ElementDef {
    let mut head = ElementDef::new(Tag::Div)
        .with_class("set-card-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("name")
                .with_text(name),
        );
    if let Some(meta) = meta {
        head = head.with_child(
            ElementDef::new(Tag::Span)
                .with_class("name-meta")
                .with_text(format!("\u{00b7} {meta}")),
        );
    }
    ElementDef::new(Tag::Div)
        .with_class("set-card")
        .with_style(StyleDeclaration::Width(Dimension::Percent(100.0)))
        .with_child(head)
}

fn settings_page_field(label: &str, desc: Option<&str>, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_class("set-field")
        .with_child(setting_meta(label, desc))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("set-control")
                .with_child(control),
        )
}

fn readout_with_unit(value: &str, unit: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-inline-control")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("input-text")
                .with_class("input-num")
                .with_text(value),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("set-unit")
                .with_text(unit),
        )
}

fn build_appearance_preview() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("preview-tile")
        .with_child(preview_line(vec![
            preview_span("prompt", "\u{276F} "),
            preview_span("path", "~/code/main/dashboard "),
            preview_span("branch", "(main)"),
        ]))
        .with_child(preview_line(vec![
            preview_span("prompt", "\u{276F} "),
            preview_span("cmd", "npm run dev"),
        ]))
        .with_child(preview_line(vec![preview_span(
            "azure",
            "\u{2192} vite v5.4.0 ready in 312 ms",
        )]))
        .with_child(preview_line(vec![preview_span(
            "sage",
            "\u{2713} recompiled in 84ms",
        )]))
        .with_child(preview_line(vec![preview_span(
            "rust",
            "\u{2717} src/lib/format.test.ts (2)",
        )]))
        .with_child(preview_line(vec![preview_span(
            "muted",
            "  expected 42 to be 41",
        )]))
        .with_child(preview_line(vec![
            preview_span("prompt", "\u{276F} "),
            ElementDef::new(Tag::Span).with_class("cur"),
        ]))
}

fn preview_line(parts: Vec<ElementDef>) -> ElementDef {
    let mut line = ElementDef::new(Tag::Div).with_class("preview-line");
    for part in parts {
        line = line.with_child(part);
    }
    line
}

fn preview_span(class: &str, text: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class(class).with_text(text)
}

fn build_settings_page_savebar(shared: &SharedState) -> ElementDef {
    let discard_state = shared.clone();
    let save_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("set-page-savebar")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("dirty")
                .with_text("\u{2022} 2 unsaved changes"),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("spacer"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("ghost")
                .with_text("discard")
                .on_click(move || {
                    mutate_with(&discard_state, |st| dispatch(st, "modal.close"));
                }),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("ghost")
                .with_text("reset to defaults"),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("primary")
                .with_text("save changes")
                .on_click(move || {
                    mutate_with(&save_state, |st| dispatch(st, "modal.close"));
                }),
        )
}

fn build_modal_header(shared: &SharedState) -> ElementDef {
    let close_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-header")
        .with_class("set-head")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-title-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("modal-mark")
                        .with_class("set-mark")
                        .with_text("\u{25C6}"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("modal-title")
                        .with_class("set-title")
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
                .with_child(svg_icon(icon_close())),
        )
}

fn build_modal_nav(active: SettingsSection, shared: &SharedState) -> ElementDef {
    let mut nav = ElementDef::new(Tag::Div)
        .with_class("modal-nav")
        .with_class("set-nav-rail");
    for section in SettingsSection::all() {
        let mut item = ElementDef::new(Tag::Button)
            .with_class("modal-nav-item")
            .with_class("set-nav")
            .with_text(section.label());
        if section == active {
            item = item.with_class("active");
        }
        let s = shared.clone();
        let target = section;
        item = item.on_click(move || {
            mutate_with(&s, |st| {
                st.settings_section = target;
                if target == SettingsSection::Sessions {
                    crate::state::refresh_sessions(st);
                }
            });
        });
        nav = nav.with_child(item);
    }
    nav
}

fn build_modal_body(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let section = match state.settings_section {
        SettingsSection::Appearance => build_appearance_section(state, shared),
        SettingsSection::Shell => build_shell_section(state, shared),
        SettingsSection::Keybinds => build_keybinds_section(state, shared),
        SettingsSection::Sessions => build_sessions_section(state, shared),
        SettingsSection::Notifications => build_notifications_section(shared),
        SettingsSection::DangerZone => build_danger_zone_section(state, shared),
    };
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_class("set-content")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_style(StyleDeclaration::FlexGrow(1.0))
        .with_style(StyleDeclaration::FlexBasis(Dimension::Auto))
        .with_style(StyleDeclaration::Overflow(Overflow::Scroll))
        .with_style(StyleDeclaration::MinHeight(Dimension::Px(0.0)))
        .with_child(section)
}

fn build_modal_content(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("set-body")
        .with_child(build_modal_nav(state.settings_section, shared))
        .with_child(build_modal_body(state, shared))
}

// -- section builders -------------------------------------------------------

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
}

fn build_shell_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let installed = crate::shell::discover_installed();
    let mut section = section_shell("shell").with_child(shell_scope_block(
        ShellScope::AppDefault,
        "App default",
        "Shell launched for new panes when no workspace overrides it",
        &state.default_shell,
        &installed,
        shared,
    ));

    if !state.workspaces.is_empty() {
        let mut overrides = ElementDef::new(Tag::Div)
            .with_class("workspace-overrides")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("modal-section-title")
                    .with_text("workspace overrides"),
            );
        for (idx, ws) in state.workspaces.iter().enumerate() {
            overrides = overrides.with_child(shell_scope_block(
                ShellScope::Workspace(idx),
                &ws.name,
                "Override the app default for this workspace only",
                &ws.shell,
                &installed,
                shared,
            ));
        }
        section = section.with_child(overrides);
    }

    section
}

/// Which shell scope a picker mutates: the app wide default, or a
/// specific workspace override (carries the workspace index used in
/// the dispatch command).
#[derive(Clone, Copy)]
enum ShellScope {
    AppDefault,
    Workspace(usize),
}

impl ShellScope {
    fn set_cmd_prefix(&self) -> String {
        match self {
            ShellScope::AppDefault => "shell.set_default:".to_string(),
            ShellScope::Workspace(idx) => format!("shell.set_workspace:{idx}:"),
        }
    }

    fn clear_cmd(&self) -> String {
        match self {
            ShellScope::AppDefault => "shell.clear_default".to_string(),
            ShellScope::Workspace(idx) => format!("shell.clear_workspace:{idx}"),
        }
    }
}

/// One editable scope in the Shell tab. Bundles label + description,
/// the chip picker (one chip per discovered shell, plus a "Use
/// default" chip for workspace scopes), a custom path input for
/// shells that aren't on PATH, and an args input.
fn shell_scope_block(
    scope: ShellScope,
    label: &str,
    desc: &str,
    current: &crate::shell::ShellSpec,
    installed: &[std::path::PathBuf],
    shared: &SharedState,
) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("shell-scope-block")
        .with_child(setting_meta(label, Some(desc)))
        .with_child(shell_picker(scope, current, installed, shared))
        .with_child(shell_custom_program_input(scope, current, shared))
        .with_child(shell_args_input(scope, current, shared))
}

/// Chip group of every discovered shell. The chip whose path matches
/// `current.program` is marked active. Workspace pickers also get a
/// "Use default" chip that dispatches the matching `shell.clear_*`.
fn shell_picker(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    installed: &[std::path::PathBuf],
    shared: &SharedState,
) -> ElementDef {
    let mut picker = ElementDef::new(Tag::Div).with_class("shell-picker");

    if let ShellScope::Workspace(_) = scope {
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("shell-chip")
            .with_class("clear")
            .with_text("use default");
        if current.is_empty() {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let cmd = scope.clear_cmd();
        chip = chip.on_click(move || {
            mutate_with(&s, |st| dispatch(st, &cmd));
        });
        picker = picker.with_child(chip);
    }

    let labels = crate::shell::label_installed_shells(installed);
    for (path, label) in installed.iter().zip(labels.iter()) {
        let program = path.display().to_string();
        let active = !current.program.is_empty() && current.program == program;
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("shell-chip")
            .with_text(label.as_str());
        if active {
            chip = chip.with_class("active");
        }
        let s = shared.clone();
        let prefix = scope.set_cmd_prefix();
        let prog = program.clone();
        let args = current.args.clone();
        chip = chip.on_click(move || {
            let spec = crate::shell::ShellSpec {
                program: prog.clone(),
                args: args.clone(),
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        });
        picker = picker.with_child(chip);
    }

    picker
}

/// Text input that reads as the current `program` (via placeholder)
/// and on submit dispatches a fresh `shell.set_*` with the typed path
/// and the existing args. Lets users pick a shell that isn't on the
/// PATH probe (e.g. portable installs, custom toolchains).
fn shell_custom_program_input(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    shared: &SharedState,
) -> ElementDef {
    let placeholder = if current.program.is_empty() {
        "custom shell path (press enter to apply)".to_string()
    } else {
        current.program.clone()
    };
    let s = shared.clone();
    let prefix = scope.set_cmd_prefix();
    let args = current.args.clone();
    ElementDef::new(Tag::Input)
        .with_class("input")
        .with_class("shell-custom-input")
        .with_placeholder(placeholder)
        .on_submit(move |text| {
            let typed = text.trim().to_string();
            if typed.is_empty() {
                return;
            }
            let spec = crate::shell::ShellSpec {
                program: typed,
                args: args.clone(),
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        })
}

/// Always visible args text input. Placeholder shows the current
/// args (space joined) so the user can see what's set without
/// pre-population (the framework's input doesn't seed initial value).
/// On submit, splits on whitespace and dispatches a fresh
/// `shell.set_*` with the existing program.
fn shell_args_input(
    scope: ShellScope,
    current: &crate::shell::ShellSpec,
    shared: &SharedState,
) -> ElementDef {
    let placeholder = if current.args.is_empty() {
        "optional args, space separated".to_string()
    } else {
        current.args.join(" ")
    };
    let s = shared.clone();
    let prefix = scope.set_cmd_prefix();
    let program = current.program.clone();
    ElementDef::new(Tag::Input)
        .with_class("input")
        .with_class("shell-args-input")
        .with_placeholder(placeholder)
        .on_submit(move |text| {
            let args: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
            let spec = crate::shell::ShellSpec {
                program: program.clone(),
                args,
            };
            let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
            mutate_with(&s, |st| {
                dispatch(st, &format!("{prefix}{json}"));
            });
        })
}

fn build_keybinds_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut section = section_shell("keybinds")
        .with_child(keybind_restart_banner())
        .with_child(keybind_error_banner(state.keybinds.error.as_ref()));

    for action in KeybindAction::ALL {
        section = section.with_child(editable_keybind_row(*action, state, shared));
    }

    section.with_child(keybind_footer(shared))
}

fn build_notifications_section(shared: &SharedState) -> ElementDef {
    let test_notification_shared = shared.clone();
    let test_notification = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_id("settings-test-notification")
        .with_text("send test")
        .on_click(move || {
            mutate_with(&test_notification_shared, |st| {
                dispatch(st, "notifications.test");
            });
        });

    section_shell("notifications").with_child(setting_row(
        "test notification",
        "sends a notification targeted at the active workspace and terminal",
        test_notification,
    ))
}

fn keybind_restart_banner() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("keybind-banner")
        .with_text("keybind changes take effect after restarting the app")
}

fn keybind_error_banner(err: Option<&KeybindError>) -> ElementDef {
    let mut banner = ElementDef::new(Tag::Div).with_class("keybind-banner-error");
    match err {
        None => banner.with_class("hidden"),
        Some(e) => {
            let msg = match &e.kind {
                KeybindErrorKind::Conflict { other, combo } => {
                    format!(
                        "{} is already bound to \"{}\"; pick another combo.",
                        combo,
                        other.label()
                    )
                }
                KeybindErrorKind::InvalidCombo { combo, message } => {
                    format!("\"{}\" is not a valid combo: {}", combo, message)
                }
            };
            banner = banner.with_text(msg.as_str());
            banner
        }
    }
}

fn editable_keybind_row(
    action: KeybindAction,
    state: &UiSnapshot,
    shared: &SharedState,
) -> ElementDef {
    let is_recording = state.keybinds.recording == Some(action);
    let is_overridden = state.keybinds.overrides.contains_key(&action);
    let has_error = state
        .keybinds
        .error
        .as_ref()
        .map(|e| e.action == action)
        .unwrap_or(false);
    let combo = state.keybinds.effective(action);

    let mut row = ElementDef::new(Tag::Div)
        .with_class("keybind-row")
        .with_child(setting_meta(action.label(), None))
        .with_child(combo_cell(action, combo, is_recording, has_error, shared));

    if is_overridden {
        row = row.with_child(reset_row_button(action, shared));
    }

    row
}

fn combo_cell(
    action: KeybindAction,
    combo: KeyCombo,
    is_recording: bool,
    has_error: bool,
    shared: &SharedState,
) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("keybind-cell");
    if is_recording {
        btn = btn.with_class("recording");
    }
    if has_error {
        btn = btn.with_class("conflict");
    }

    if is_recording {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("keybind-recording-label")
                .with_text("press keys... (esc to cancel)"),
        );
    } else {
        for part in combo_parts(combo) {
            btn = btn.with_child(pill("keybind-key", None, &part));
        }
    }

    let s = shared.clone();
    let command = if is_recording {
        "keybind.cancel_record".to_string()
    } else {
        format!("keybind.record:{}", action.id())
    };
    btn.on_click(move || {
        mutate_with(&s, |st| dispatch(st, &command));
    })
}

fn reset_row_button(action: KeybindAction, shared: &SharedState) -> ElementDef {
    let s = shared.clone();
    let cmd = format!("keybind.reset:{}", action.id());
    ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_class("keybind-reset")
        .with_text("reset")
        .on_click(move || {
            mutate_with(&s, |st| dispatch(st, &cmd));
        })
}

/// Split a combo into the parts shown as individual key pills. Modifiers
/// are pushed in the canonical Ctrl, Shift, Alt, Meta order; then the key
/// name comes last.
fn combo_parts(combo: KeyCombo) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if combo.modifiers.contains(Modifiers::CTRL) {
        parts.push("Ctrl".to_string());
    }
    if combo.modifiers.contains(Modifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    if combo.modifiers.contains(Modifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if combo.modifiers.contains(Modifiers::META) {
        parts.push("Meta".to_string());
    }
    parts.push(combo.key.to_string());
    parts
}

fn build_sessions_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let refresh_shared = shared.clone();
    let refresh = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_id("settings-sessions-refresh")
        .with_text("refresh")
        .on_click(move || {
            mutate_with(&refresh_shared, |st| {
                dispatch(st, "sessions.refresh");
            });
        });

    let mut control = ElementDef::new(Tag::Div)
        .with_class("sessions-refresh-control")
        .with_child(refresh);
    if state.sessions_stale {
        control = control.with_child(
            ElementDef::new(Tag::Span)
                .with_class("sessions-refresh-stale")
                .with_text("stale"),
        );
    }

    let mut section = section_shell("sessions").with_child(setting_row(
        "daemon sessions",
        "sessions currently tracked by the session daemon; refresh to re-poll",
        control,
    ));

    if state.sessions.is_empty() {
        section = section.with_child(
            ElementDef::new(Tag::Div)
                .with_class("sessions-empty")
                .with_text("no sessions; press refresh to poll the daemon"),
        );
        return section;
    }

    for s in &state.sessions {
        section = section.with_child(session_row(s, shared));
    }
    section
}

fn session_row(s: &crate::state::SessionSnapshot, shared: &SharedState) -> ElementDef {
    let label = s.name.clone().unwrap_or_else(|| match s.pid {
        Some(p) => format!("shell ({p})"),
        None => format!("shell (session {})", s.session_id),
    });
    let meta = ElementDef::new(Tag::Span)
        .with_class("setting-desc")
        .with_child(ElementDef::new(Tag::Span).with_text(format!(
            "workspace {} · pane {} · ",
            s.workspace_id, s.pane_id
        )))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class(if s.alive {
                    "session-status-alive"
                } else {
                    "session-status-dead"
                })
                .with_text(if s.alive { "alive" } else { "dead" }),
        );

    let kill_shared = shared.clone();
    let session_id = s.session_id;
    let kill = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("danger")
        .with_text("kill")
        .on_click(move || {
            mutate_with(&kill_shared, |st| {
                dispatch(st, &format!("session.kill:{session_id}"));
            });
        });

    let rename_shared = shared.clone();
    let pane_id = s.pane_id;
    let rename = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("ghost")
        .with_text("rename")
        .on_click(move || {
            mutate_with(&rename_shared, |st| {
                dispatch(st, "modal.close");
                dispatch(st, &format!("tab.request_rename:{pane_id}"));
            });
        });

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
                .with_child(meta),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("session-row-actions")
                .with_child(rename)
                .with_child(kill),
        )
}

fn build_danger_zone_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let live_count = state.terminal_count;
    let button_shared = shared.clone();
    let kill_all = ElementDef::new(Tag::Button)
        .with_class("btn")
        .with_class("danger")
        .with_id("settings-kill-all-terminals")
        .on_click(move || {
            mutate_with(&button_shared, |st| {
                dispatch(st, "modal.close");
                dispatch(st, "app.request_kill_all_terminals");
            });
        })
        .with_text(if live_count == 0 {
            "kill all terminals".to_string()
        } else if live_count == 1 {
            "kill 1 terminal".to_string()
        } else {
            format!("kill {live_count} terminals")
        });

    let mut section = section_shell("danger zone").with_child(setting_row(
        "kill all terminals",
        "Destroys every running shell across every workspace. Workspaces are kept but emptied.",
        kill_all,
    ));

    if is_on(state, ToggleKey::RememberCloseChoice) {
        let kill_on_close = is_on(state, ToggleKey::KillAllOnClose);
        let desc = if kill_on_close {
            "Close currently kills every terminal and quits without asking. Reset to show the confirm prompt again."
        } else {
            "Close currently quits while leaving terminals running on the daemon. Reset to show the confirm prompt again."
        };
        let reset_shared = shared.clone();
        let reset = ElementDef::new(Tag::Button)
            .with_class("btn")
            .with_class("ghost")
            .with_id("settings-close-prompt-reset")
            .on_click(move || {
                mutate_with(&reset_shared, |st| {
                    dispatch(st, "app.close.reset_preference");
                });
            })
            .with_text("reset".to_string());
        section = section.with_child(setting_row("Close behavior", desc, reset));
    }

    section
}

fn build_modal_footer(shared: &SharedState) -> ElementDef {
    let cancel_state = shared.clone();
    let save_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("modal-footer")
        .with_class("set-foot")
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
        .with_class("set-card")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
                .with_class("set-card-head")
                .with_class("name")
                .with_text(title),
        )
}

fn setting_meta(label: &str, desc: Option<&str>) -> ElementDef {
    let mut meta = ElementDef::new(Tag::Div)
        .with_class("setting-meta")
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("setting-label")
                .with_class("set-label")
                .with_text(label),
        );
    if let Some(desc) = desc {
        meta = meta.with_child(
            ElementDef::new(Tag::Span)
                .with_class("setting-desc")
                .with_class("set-desc")
                .with_text(desc),
        );
    }
    meta
}

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_class("set-field")
        .with_child(setting_meta(label, Some(desc)))
        .with_child(control)
}

fn theme_chip_group(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut chips = ElementDef::new(Tag::Div)
        .with_class("theme-chips")
        .with_class("color-swatches");
    for theme in ["amber", "green", "cyan", "mono"] {
        let mut chip = ElementDef::new(Tag::Button)
            .with_class("theme-chip")
            .with_class("sw")
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

type StepCallback = Box<dyn Fn() + Send + Sync + 'static>;

struct StepCallbacks {
    on_dec: StepCallback,
    on_inc: StepCallback,
}

fn stepper(value: &str, callbacks: StepCallbacks) -> ElementDef {
    let dec = ElementDef::new(Tag::Button)
        .with_class("stepper-btn")
        .with_text("\u{2212}")
        .on_click(callbacks.on_dec);
    let inc = ElementDef::new(Tag::Button)
        .with_class("stepper-btn")
        .with_text("+")
        .on_click(callbacks.on_inc);
    ElementDef::new(Tag::Div)
        .with_class("stepper")
        .with_child(dec)
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("stepper-val")
                .with_class("tnum")
                .with_text(value),
        )
        .with_child(inc)
}

fn font_stepper(value: u32, shared: &SharedState) -> ElementDef {
    let dec_shared = shared.clone();
    let inc_shared = shared.clone();
    let callbacks = StepCallbacks {
        on_dec: Box::new(move || {
            mutate_with(&dec_shared, |st| dispatch(st, "font.dec"));
        }),
        on_inc: Box::new(move || {
            mutate_with(&inc_shared, |st| dispatch(st, "font.inc"));
        }),
    };
    stepper(&value.to_string(), callbacks)
}

fn pill(base: &str, modifier: Option<&str>, text: &str) -> ElementDef {
    let mut el = ElementDef::new(Tag::Span).with_class(base).with_text(text);
    if let Some(m) = modifier {
        el = el.with_class(m);
    }
    el
}

fn svg_icon(svg: SvgNode) -> ElementDef {
    ElementDef::new(Tag::Div).with_svg(svg)
}

fn keybind_footer(shared: &SharedState) -> ElementDef {
    let s = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("keybind-footer")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .with_class("ghost")
                .with_text("reset to defaults")
                .on_click(move || {
                    mutate_with(&s, |st| dispatch(st, "keybind.reset_all"));
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SettingsSection};
    use std::sync::{Arc, Mutex};
    use unshit::core::element::{ElementContent, ElementTree};
    use unshit::core::style::types::TextAlign;
    use unshit_test::TestHarness;

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
    fn settings_modal_matches_design_system_shell_structure() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_settings_modal(&snap, &shared);
        // header, set-body(nav + content), footer
        assert_eq!(el.children.len(), 3);
        assert!(el.children[1].classes.contains(&"set-body".to_string()));
        assert!(el.children[1].children[0]
            .classes
            .contains(&"set-nav-rail".to_string()));
        assert!(el.children[1].children[1]
            .classes
            .contains(&"set-content".to_string()));
    }

    #[test]
    fn settings_page_matches_design_system_page_structure() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);

        assert!(el.classes.contains(&"settings-page".to_string()));
        assert_eq!(el.children.len(), 2);
        assert!(el.children[0]
            .classes
            .contains(&"set-page-rail".to_string()));
        assert!(el.children[1]
            .classes
            .contains(&"set-page-content".to_string()));
        assert!(has_class_anywhere(&el, "set-page-search"));
        assert!(has_class_anywhere(&el, "set-page-header"));
        assert!(has_class_anywhere(&el, "set-page-savebar"));
    }

    #[test]
    fn settings_page_appearance_renders_applied_controls_and_preview() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let el = build_settings_page(&snap, &shared);

        assert_eq!(count_with_class(&el, "set-card"), 3);
        assert!(has_class_anywhere(&el, "stepper"));
        assert!(has_class_anywhere(&el, "set-inline-control"));
        assert!(has_class_anywhere(&el, "preview-tile"));
        let text = collect_text_recursive(&el);
        assert!(text.contains("Terminal output size"));
        assert!(text.contains("Width of the workspace sidebar"));
        for stripped in [
            "Theme",
            "Accent",
            "Scanline overlay",
            "Background grain",
            "Tab bar density",
        ] {
            assert!(
                !text.contains(stripped),
                "settings page should not render unapplied/fake setting {stripped:?}"
            );
        }
    }

    #[test]
    fn settings_page_controls_have_visible_layout_with_stylesheet() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
        let shared = make_shared();
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_class("settings")
                    .with_child(build_settings_page(&tree_snap, &tree_shared)),
            },
            1280.0,
            800.0,
        );
        harness.set_scale_factor(1.5);
        harness.step();

        for selector in [
            ".stepper",
            ".stepper-btn",
            ".set-inline-control",
            ".input-text",
            ".preview-tile",
        ] {
            let snap = harness.query(selector).expect(selector);
            assert!(
                snap.layout_rect.width > 0.0 && snap.layout_rect.height > 0.0,
                "{selector} should have non-zero layout, got {:?}",
                snap.layout_rect
            );
            assert!(
                snap.layout_rect.x >= 0.0 && snap.layout_rect.x + snap.layout_rect.width <= 1280.0,
                "{selector} should be horizontally visible, got {:?}",
                snap.layout_rect
            );
        }

        let stepper_btn = harness.query(".stepper-btn").expect(".stepper-btn");
        assert_eq!(
            stepper_btn.computed_style.text_align,
            TextAlign::Center,
            "button UA defaults should center stepper glyphs unless CSS overrides them"
        );
        let number = harness.query(".input-num").expect(".input-num");
        assert_eq!(
            number.computed_style.text_align,
            TextAlign::Right,
            "settings number readouts rely on source CSS text-align"
        );
    }

    #[test]
    fn settings_page_styles_match_design_system_geometry_and_effects() {
        let css = include_str!("../../assets/styles.css");
        assert!(css.contains("grid-template-columns: 240px minmax(0, 1fr);"));
        assert!(css.contains("max-width: 820px;"));
        assert!(css.contains("backdrop-filter: blur(6px);"));
        assert!(css.contains(".kbd-table"));
        assert!(css.contains(".kbd-binding"));
        assert!(
            css.contains(".preview-tile") && css.contains("border: 1px solid var(--border-hair);")
        );
    }

    #[test]
    fn settings_page_nav_item_click_changes_section() {
        let shared = make_shared();
        let rail = build_settings_page_rail(SettingsSection::Appearance, &shared);
        let nav = &rail.children[2];
        let shell = &nav.children[2];

        (shell.on_click.as_ref().unwrap())();

        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Shell
        );
    }

    #[test]
    fn settings_page_savebar_matches_design_system_actions() {
        let shared = make_shared();
        let el = build_settings_page_savebar(&shared);
        let text = collect_text_recursive(&el);

        assert!(el.classes.contains(&"set-page-savebar".to_string()));
        assert!(text.contains("2 unsaved changes"));
        assert!(text.contains("discard"));
        assert!(text.contains("reset to defaults"));
        assert!(text.contains("save changes"));
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
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert!(el.classes.contains(&"modal-nav".to_string()));
    }

    #[test]
    fn modal_nav_has_six_items() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert_eq!(el.children.len(), 6);
    }

    #[test]
    fn modal_nav_marks_appearance_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        assert!(el.children[0].classes.contains(&"active".to_string()));
        for child in &el.children[1..] {
            assert!(!child.classes.contains(&"active".to_string()));
        }
    }

    #[test]
    fn modal_nav_marks_shell_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Shell, &shared);
        assert!(el.children[1].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_keybinds_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Keybinds, &shared);
        assert!(el.children[2].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_sessions_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Sessions, &shared);
        assert!(el.children[3].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_notifications_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Notifications, &shared);
        assert!(el.children[4].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_marks_danger_zone_active() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::DangerZone, &shared);
        assert!(el.children[5].classes.contains(&"active".to_string()));
    }

    #[test]
    fn modal_nav_items_have_click_handlers() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        for child in &el.children {
            assert!(child.on_click.is_some());
        }
    }

    // -- build_modal_body -------------------------------------------------------

    #[test]
    fn modal_body_renders_only_active_section() {
        let snap = make_snapshot_section(SettingsSection::Appearance);
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
    fn modal_body_switches_to_shell() {
        let snap = make_snapshot_section(SettingsSection::Shell);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("shell"));
    }

    #[test]
    fn modal_body_switches_to_notifications() {
        let snap = make_snapshot_section(SettingsSection::Notifications);
        let shared = make_shared();
        let el = build_modal_body(&snap, &shared);
        let section = &el.children[0];
        let title = &section.children[0];
        assert_eq!(text_of(title), Some("notifications"));
    }

    // -- build_appearance_section -----------------------------------------------

    #[test]
    fn appearance_section_has_title_and_two_rows() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_appearance_section(&snap, &shared);
        // title + 2 rows (theme, font)
        assert_eq!(el.children.len(), 3);
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

    // -- build_shell_section ----------------------------------------------------

    fn find_first_with_class<'a>(root: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if root.classes.iter().any(|c| c == class) {
            return Some(root);
        }
        root.children
            .iter()
            .find_map(|c| find_first_with_class(c, class))
    }

    fn count_with_class(root: &ElementDef, class: &str) -> usize {
        let here = if root.classes.iter().any(|c| c == class) {
            1
        } else {
            0
        };
        here + root
            .children
            .iter()
            .map(|c| count_with_class(c, class))
            .sum::<usize>()
    }

    fn collect_text_recursive(root: &ElementDef) -> String {
        let mut acc = String::new();
        if let Some(t) = text_of(root) {
            acc.push_str(t);
            acc.push(' ');
        }
        for child in &root.children {
            acc.push_str(&collect_text_recursive(child));
        }
        acc
    }

    #[test]
    fn shell_section_starts_with_app_default_block() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        // first child after the title must be the app default scope block
        let first = &el.children[1];
        assert!(
            first.classes.contains(&"shell-scope-block".to_string()),
            "first body child must be a shell-scope-block, got classes: {:?}",
            first.classes
        );
    }

    #[test]
    fn shell_section_includes_shell_picker_under_app_default_block() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        assert!(
            find_first_with_class(&el, "shell-picker").is_some(),
            "shell section must include a shell-picker"
        );
    }

    #[test]
    fn shell_picker_marks_active_chip_when_program_matches() {
        // Build a snapshot whose default_shell.program matches a fake
        // discovered shell, then assert at least one chip carries the
        // "active" class. We feed the picker directly so the test does
        // not depend on what's installed on the host.
        let installed = vec![std::path::PathBuf::from("/bin/bash")];
        let current = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec![],
        };
        let shared = make_shared();
        let picker = shell_picker(ShellScope::AppDefault, &current, &installed, &shared);
        assert!(
            count_with_class(&picker, "active") >= 1,
            "matching program must mark a chip active"
        );
    }

    #[test]
    fn shell_picker_for_workspace_includes_use_default_chip() {
        let installed: Vec<std::path::PathBuf> = vec![];
        let current = crate::shell::ShellSpec::default();
        let shared = make_shared();
        let picker = shell_picker(ShellScope::Workspace(0), &current, &installed, &shared);
        assert!(
            collect_text_recursive(&picker).contains("use default"),
            "workspace picker must include a use default chip"
        );
    }

    #[test]
    fn shell_picker_for_app_default_omits_use_default_chip() {
        let installed: Vec<std::path::PathBuf> = vec![];
        let current = crate::shell::ShellSpec::default();
        let shared = make_shared();
        let picker = shell_picker(ShellScope::AppDefault, &current, &installed, &shared);
        assert!(
            !collect_text_recursive(&picker).contains("use default"),
            "app default picker must NOT have a use default chip"
        );
    }

    #[test]
    fn shell_section_has_one_workspace_override_block_per_workspace() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_shell_section(&snap, &shared);
        let overrides = find_first_with_class(&el, "workspace-overrides")
            .expect("workspace-overrides subsection must be present");
        let blocks = count_with_class(overrides, "shell-scope-block");
        assert_eq!(
            blocks,
            snap.workspaces.len(),
            "workspace overrides must have one block per workspace"
        );
    }

    // -- build_keybinds_section -------------------------------------------------

    #[test]
    fn keybinds_section_has_banner_one_row_per_action_and_footer() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        // title + restart banner + error banner + one row per action + footer
        let expected = 4 + KeybindAction::ALL.len();
        assert_eq!(el.children.len(), expected);
    }

    #[test]
    fn keybinds_row_shows_effective_combo_parts() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        // children: [title, restart_banner, error_banner, row0, row1, ..., footer]
        let first_row = &el.children[3];
        assert!(first_row.classes.contains(&"keybind-row".to_string()));
        // cell: [setting_meta, combo_cell, (maybe reset)]
        let combo_cell = &first_row.children[1];
        assert!(combo_cell.classes.contains(&"keybind-cell".to_string()));
        // Default NewTerminal is Ctrl+T so we expect 2 pills.
        assert_eq!(combo_cell.children.len(), 2);
    }

    #[test]
    fn keybinds_footer_has_reset_to_defaults() {
        let snap = make_snapshot();
        let shared = make_shared();
        let el = build_keybinds_section(&snap, &shared);
        let footer = el.children.last().unwrap();
        assert!(footer.classes.contains(&"keybind-footer".to_string()));
        let btn = &footer.children[0];
        assert_eq!(text_of(btn), Some("reset to defaults"));
    }

    #[test]
    fn keybinds_row_with_override_includes_reset_button() {
        let mut state = seed_state();
        state
            .keybinds
            .set(
                crate::keybinds::KeybindAction::NewTerminal,
                unshit::core::shortcut::KeyCombo::parse("Alt+N").unwrap(),
            )
            .unwrap();
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        // NewTerminal is the first row (index 3 after title + 2 banners).
        let first_row = &el.children[3];
        // With override: [meta, combo_cell, reset_btn] -> 3 children.
        assert_eq!(first_row.children.len(), 3);
        let reset = &first_row.children[2];
        assert!(reset.classes.contains(&"keybind-reset".to_string()));
    }

    #[test]
    fn keybinds_row_in_recording_state_shows_placeholder() {
        let mut state = seed_state();
        state
            .keybinds
            .start_recording(crate::keybinds::KeybindAction::NewTerminal);
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        let first_row = &el.children[3];
        let combo_cell = &first_row.children[1];
        assert!(combo_cell.classes.contains(&"recording".to_string()));
        assert_eq!(combo_cell.children.len(), 1);
        assert_eq!(
            text_of(&combo_cell.children[0]),
            Some("press keys... (esc to cancel)")
        );
    }

    #[test]
    fn keybinds_error_banner_visible_on_conflict() {
        let mut state = seed_state();
        // Provoke a conflict: set NewTerminal to Unsplit's default.
        let _ = state.keybinds.set(
            crate::keybinds::KeybindAction::NewTerminal,
            unshit::core::shortcut::KeyCombo::parse("Ctrl+W").unwrap(),
        );
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        let el = build_keybinds_section(&snap, &shared);
        let error_banner = &el.children[2];
        assert!(error_banner
            .classes
            .contains(&"keybind-banner-error".to_string()));
        assert!(!error_banner.classes.contains(&"hidden".to_string()));
    }

    // -- build_notifications_section ------------------------------------------

    #[test]
    fn notifications_section_test_button_pushes_notification() {
        let shared = make_shared();
        let el = build_notifications_section(&shared);
        assert_eq!(text_of(&el.children[0]), Some("notifications"));
        let test_btn =
            find_by_id(&el, "settings-test-notification").expect("test notification button");

        (test_btn.on_click.as_ref().unwrap())();

        let state = shared.lock().unwrap();
        let snap = state.ui_snapshot();
        let toast = snap.toasts.first().expect("test notification toast");
        assert_eq!(toast.title.as_deref(), Some("Test notification"));
        assert_eq!(
            toast.target,
            Some(crate::state::ToastTarget {
                workspace_id: crate::state::active_workspace_num(&state),
                pane_id: state.active_pane.0,
            })
        );
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
    fn nav_item_click_changes_to_shell() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[1].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Shell
        );
    }

    #[test]
    fn nav_item_click_changes_to_keybinds() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[2].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Keybinds
        );
    }

    #[test]
    fn nav_item_click_changes_to_sessions() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[3].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Sessions
        );
    }

    #[test]
    fn nav_item_click_changes_to_danger_zone() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[5].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::DangerZone
        );
    }

    #[test]
    fn nav_item_click_changes_to_notifications() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::Appearance, &shared);
        (el.children[4].on_click.as_ref().unwrap())();
        assert_eq!(
            shared.lock().unwrap().settings_section,
            SettingsSection::Notifications
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
    fn stepper_wires_callbacks_to_buttons() {
        let callbacks = StepCallbacks {
            on_dec: Box::new(|| {}),
            on_inc: Box::new(|| {}),
        };
        let el = stepper("7", callbacks);
        assert!(el.classes.contains(&"stepper".to_string()));
        assert_eq!(el.children.len(), 3);
        let dec = &el.children[0];
        let inc = &el.children[2];
        assert_eq!(text_of(&el.children[1]), Some("7"));
        assert!(dec.on_click.is_some());
        assert!(inc.on_click.is_some());
    }

    // -- build_sessions_section -------------------------------------------------

    #[test]
    fn sessions_section_empty_state_shows_placeholder() {
        let snap = make_snapshot_section(SettingsSection::Sessions);
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        assert!(el
            .children
            .iter()
            .any(|c| c.classes.contains(&"sessions-empty".to_string())));
    }

    #[test]
    fn sessions_section_renders_row_per_session() {
        let mut state = seed_state();
        state.settings_section = SettingsSection::Sessions;
        state.sessions = vec![
            crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 1,
                name: Some("build".into()),
                pid: Some(1234),
                alive: true,
            },
            crate::state::SessionSnapshot {
                session_id: 2,
                pane_id: 2,
                workspace_id: 1,
                name: None,
                pid: Some(5678),
                alive: false,
            },
        ];
        let snap = state.ui_snapshot();
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let rows: Vec<_> = el
            .children
            .iter()
            .filter(|c| c.classes.contains(&"setting-row".to_string()))
            .collect();
        // First row is the "daemon sessions / refresh" header row, then
        // one row per session.
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn sessions_section_named_session_shows_custom_label() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 42,
                name: Some("api-server".into()),
                pid: Some(1234),
                alive: true,
            }],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let labels: Vec<&str> = el
            .children
            .iter()
            .filter_map(|c| {
                c.children
                    .iter()
                    .find(|m| m.classes.contains(&"setting-meta".to_string()))
                    .and_then(|m| m.children.first())
                    .and_then(text_of)
            })
            .collect();
        assert!(labels.contains(&"api-server"));
    }

    #[test]
    fn sessions_section_unnamed_session_shows_pid_fallback() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![crate::state::SessionSnapshot {
                session_id: 1,
                pane_id: 1,
                workspace_id: 1,
                name: None,
                pid: Some(9999),
                alive: true,
            }],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let labels: Vec<String> = el
            .children
            .iter()
            .filter_map(|c| {
                c.children
                    .iter()
                    .find(|m| m.classes.contains(&"setting-meta".to_string()))
                    .and_then(|m| m.children.first())
                    .and_then(text_of)
                    .map(|s| s.to_string())
            })
            .collect();
        assert!(labels.iter().any(|l| l == "shell (9999)"));
    }

    #[test]
    fn sessions_section_alive_session_has_alive_status_class() {
        let snap = crate::state::UiSnapshot {
            sessions: vec![
                crate::state::SessionSnapshot {
                    session_id: 1,
                    pane_id: 1,
                    workspace_id: 1,
                    name: Some("a".into()),
                    pid: None,
                    alive: true,
                },
                crate::state::SessionSnapshot {
                    session_id: 2,
                    pane_id: 2,
                    workspace_id: 1,
                    name: Some("b".into()),
                    pid: None,
                    alive: false,
                },
            ],
            ..seed_state().ui_snapshot()
        };
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let rows: Vec<_> = el
            .children
            .iter()
            .filter(|c| c.classes.contains(&"setting-row".to_string()))
            .collect();
        // rows[0] is header; rows[1] alive, rows[2] dead.
        let alive_meta = &rows[1].children[0];
        let dead_meta = &rows[2].children[0];
        let has_status_class = |meta: &ElementDef, cls: &str| {
            meta.children.iter().any(|c| {
                c.children
                    .iter()
                    .any(|span| span.classes.iter().any(|k| k == cls))
            })
        };
        assert!(has_status_class(alive_meta, "session-status-alive"));
        assert!(has_status_class(dead_meta, "session-status-dead"));
    }

    #[test]
    fn sessions_section_refresh_button_click_dispatches_refresh() {
        let snap = make_snapshot_section(SettingsSection::Sessions);
        let shared = make_shared();
        let el = build_sessions_section(&snap, &shared);
        let refresh_btn = find_by_id(&el, "settings-sessions-refresh").expect("refresh button");
        // Invoking succeeds without panic; actual daemon call is a no-op
        // because no daemon is connected in unit tests.
        (refresh_btn.on_click.as_ref().unwrap())();
    }

    fn find_by_id<'a>(el: &'a ElementDef, target: &str) -> Option<&'a ElementDef> {
        if el.id.as_deref() == Some(target) {
            return Some(el);
        }
        el.children.iter().find_map(|c| find_by_id(c, target))
    }

    // refs #130: stale chip surfaces failed refreshes next to the button.
    #[test]
    fn sessions_section_renders_stale_chip_when_flag_set() {
        let mut snap = make_snapshot_section(SettingsSection::Sessions);
        assert!(!snap.sessions_stale);
        let shared = make_shared();
        let clean = build_sessions_section(&snap, &shared);
        assert!(!has_class_anywhere(&clean, "sessions-refresh-stale"));

        snap.sessions_stale = true;
        let stale = build_sessions_section(&snap, &shared);
        assert!(has_class_anywhere(&stale, "sessions-refresh-stale"));
    }

    fn has_class_anywhere(el: &ElementDef, class: &str) -> bool {
        if el.classes.iter().any(|c| c == class) {
            return true;
        }
        el.children.iter().any(|c| has_class_anywhere(c, class))
    }
}
