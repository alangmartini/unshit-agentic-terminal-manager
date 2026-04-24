use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{Dimension, Display, FlexDirection, Overflow};
use unshit::prelude::SvgNode;

use crate::state::{
    agent_enabled, dispatch, is_on, mutate_toggle_agent, mutate_with, AgentKey, SettingsSection,
    SharedState, ToggleKey, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_settings_modal(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("modal")
        .with_style(StyleDeclaration::Display(Display::Flex))
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
                .with_child(svg_icon(icon_close())),
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
        SettingsSection::General => build_general_section(state, shared),
        SettingsSection::Appearance => build_appearance_section(state, shared),
        SettingsSection::Shell => build_shell_section(state, shared),
        SettingsSection::Keybinds => build_keybinds_section(),
        SettingsSection::Agents => build_agents_section(state, shared),
        SettingsSection::Sessions => build_sessions_section(state, shared),
        SettingsSection::DangerZone => build_danger_zone_section(state, shared),
    };
    ElementDef::new(Tag::Div)
        .with_class("modal-body")
        .with_style(StyleDeclaration::Display(Display::Flex))
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
        .with_child(toggle_row(
            state,
            shared,
            "Restore on startup",
            "Reopen last active session and panes",
            ToggleKey::RestoreOnStartup,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Confirm before closing",
            "Warn when closing a tab with a running process",
            ToggleKey::ConfirmClose,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Start minimized",
            "Launch to system tray on startup",
            ToggleKey::StartMinimized,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Check for updates",
            "Notify when a new version is available",
            ToggleKey::CheckUpdates,
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
            stepper("1.4", None),
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Glow effect",
            "Subtle CRT-style text shadow on output",
            ToggleKey::GlowEffect,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Background texture",
            "Warm ambient gradient behind content",
            ToggleKey::BackgroundTexture,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Font ligatures",
            "Combine character pairs like => and !=",
            ToggleKey::FontLigatures,
        ))
}

fn build_shell_section(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    section_shell("shell")
        .with_child(toggle_row(
            state,
            shared,
            "Shell integration",
            "Inject prompt markers for smart scrollback",
            ToggleKey::ShellIntegration,
        ))
        .with_child(setting_row(
            "History size",
            "Lines retained per pane",
            text_input_display("50000"),
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Scroll on output",
            "Auto-scroll terminal when new output arrives",
            ToggleKey::ScrollOnOutput,
        ))
        .with_child(toggle_row(
            state,
            shared,
            "Bell notification",
            "Flash tab badge when terminal rings the bell",
            ToggleKey::BellNotification,
        ))
        .with_child(setting_row(
            "Word separators",
            "Characters that break word selection on double-click",
            compact_input_display(" /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}`~?"),
        ))
}

/// Memo key for the keybinds section.
///
/// The keybinds section is a pure function of compile-time constants (no
/// `&state`, no `&shared`). Tagging it with a stable memo key lets the
/// reconciler skip the entire subtree on every rebuild once it has been
/// mounted, which matters because the section is rebuilt on every tab
/// switch even when nothing about it has changed.
const KEYBINDS_MEMO_KEY: u64 = 0xBEEF_0001_CAFE_0002_u64;

fn build_keybinds_section() -> ElementDef {
    section_shell("keybinds")
        .with_memo_key(KEYBINDS_MEMO_KEY)
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
    let mut section = section_shell("agents")
        .with_child(toggle_row(
            state,
            shared,
            "Auto-discovery",
            "Detect installed AI agents on PATH",
            ToggleKey::AutoDiscovery,
        ))
        .with_child(setting_row(
            "Default timeout",
            "Seconds before an agent task is canceled",
            stepper("300", None),
        ))
        .with_child(agent_list_header(AGENT_SPECS.len()));
    for spec in AGENT_SPECS {
        section = section.with_child(agent_row(spec, agent_enabled(state, spec.key), shared));
    }
    section
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

    let mut section = section_shell("sessions").with_child(setting_row(
        "Daemon sessions",
        "Sessions currently tracked by the session daemon. Refresh to re-poll.",
        refresh,
    ));

    if state.sessions.is_empty() {
        section = section.with_child(
            ElementDef::new(Tag::Div)
                .with_class("sessions-empty")
                .with_text("No sessions. Press refresh to poll the daemon."),
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
        "Kill all terminals",
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
        .with_style(StyleDeclaration::Display(Display::Flex))
        .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-section-title")
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
                .with_text(label),
        );
    if let Some(desc) = desc {
        meta = meta.with_child(
            ElementDef::new(Tag::Span)
                .with_class("setting-desc")
                .with_text(desc),
        );
    }
    meta
}

fn setting_row(label: &str, desc: &str, control: ElementDef) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("setting-row")
        .with_child(setting_meta(label, Some(desc)))
        .with_child(control)
}

fn toggle_row(
    state: &UiSnapshot,
    shared: &SharedState,
    label: &str,
    desc: &str,
    key: ToggleKey,
) -> ElementDef {
    setting_row(label, desc, toggle_button(is_on(state, key), key, shared))
}

fn toggle_button(on: bool, key: ToggleKey, shared: &SharedState) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("toggle");
    if on {
        btn = btn.with_class("on");
    }
    let s = shared.clone();
    btn.on_click(move || {
        mutate_with(&s, |st| {
            let next = !st.toggles.get(&key).copied().unwrap_or(false);
            st.toggles.insert(key, next);
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

type StepCallback = Box<dyn Fn() + Send + Sync + 'static>;

struct StepCallbacks {
    on_dec: StepCallback,
    on_inc: StepCallback,
}

fn stepper(value: &str, callbacks: Option<StepCallbacks>) -> ElementDef {
    let (dec, inc) = match callbacks {
        Some(cb) => (
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("\u{2212}")
                .on_click(cb.on_dec),
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_text("+")
                .on_click(cb.on_inc),
        ),
        None => (
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_class("disabled")
                .with_text("\u{2212}"),
            ElementDef::new(Tag::Button)
                .with_class("stepper-btn")
                .with_class("disabled")
                .with_text("+"),
        ),
    };
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
    stepper(&value.to_string(), Some(callbacks))
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
    for key in keys {
        keys_wrap = keys_wrap.with_child(pill("keybind-key", None, key));
    }
    ElementDef::new(Tag::Div)
        .with_class("keybind-row")
        .with_child(setting_meta(label, None))
        .with_child(keys_wrap)
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

#[derive(Clone, Copy)]
enum AgentStatus {
    Running,
    Idle,
    Disabled,
}

impl AgentStatus {
    fn kind(self) -> &'static str {
        match self {
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Disabled => "disabled",
        }
    }
}

struct AgentSpec {
    name: &'static str,
    path: &'static str,
    status: AgentStatus,
    key: AgentKey,
}

const AGENT_SPECS: &[AgentSpec] = &[
    AgentSpec {
        name: "claude",
        path: "~/.local/bin/claude",
        status: AgentStatus::Running,
        key: AgentKey::Claude,
    },
    AgentSpec {
        name: "amp",
        path: "~/.local/bin/amp",
        status: AgentStatus::Idle,
        key: AgentKey::Amp,
    },
    AgentSpec {
        name: "codex",
        path: "~/.local/bin/codex",
        status: AgentStatus::Disabled,
        key: AgentKey::Codex,
    },
];

fn agent_list_header(count: usize) -> ElementDef {
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

fn agent_row(spec: &AgentSpec, enabled: bool, shared: &SharedState) -> ElementDef {
    let label = spec.status.kind();
    ElementDef::new(Tag::Div)
        .with_class("agent-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-icon")
                .with_child(svg_icon(icon_agent())),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-info")
                .with_style(StyleDeclaration::Display(Display::Flex))
                .with_style(StyleDeclaration::FlexDirection(FlexDirection::Column))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-name")
                        .with_text(spec.name),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("agent-path")
                        .with_text(spec.path),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("agent-controls")
                .with_child(agent_badge(spec.status.kind(), label))
                .with_child(agent_toggle_button(enabled, spec.key, shared)),
        )
}

/// Toggle button for an agent row. Mirrors `toggle_button` but writes to
/// the typed `state.agents` vec via `mutate_toggle_agent` instead of the
/// generic `state.toggles` map.
fn agent_toggle_button(on: bool, key: AgentKey, shared: &SharedState) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("toggle");
    if on {
        btn = btn.with_class("on");
    }
    let s = shared.clone();
    btn.on_click(move || {
        mutate_with(&s, |st| mutate_toggle_agent(st, key));
    })
}

fn agent_badge(kind: &str, label: &str) -> ElementDef {
    pill("agent-badge", Some(kind), label)
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
    fn modal_nav_has_seven_items() {
        let shared = make_shared();
        let el = build_modal_nav(SettingsSection::General, &shared);
        assert_eq!(el.children.len(), 7);
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
        // Separator lives in CSS (::before), so only the key pills render.
        assert_eq!(keys_wrap.children.len(), 2);
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

    #[test]
    fn keybinds_section_has_stable_memo_key() {
        // The keybinds section is a pure function of compile-time constants.
        // It must carry a stable memo key so the reconciler can skip its
        // subtree on every rebuild triggered by tab switches.
        let first = build_keybinds_section();
        let second = build_keybinds_section();
        assert_eq!(first.memo_key, Some(KEYBINDS_MEMO_KEY));
        assert_eq!(first.memo_key, second.memo_key);
    }

    #[test]
    fn keybinds_section_memo_key_is_nonzero() {
        // A key of 0 would still work, but a nonzero key makes it obvious in
        // debug output that memoization was deliberately configured.
        assert_ne!(KEYBINDS_MEMO_KEY, 0);
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

    #[test]
    fn agent_toggle_click_writes_to_agents_field() {
        // refs #107 - agent enabled state lives in `state.agents`, not in
        // the generic `state.toggles` map. Clicking an agent row's toggle
        // must flip the corresponding `Agent::enabled` field.
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_agents_section(&snap, &shared);
        let claude_row = &el.children[4];
        let toggle = &claude_row.children[2].children[1];

        let was_on = agent_enabled(&shared.lock().unwrap().ui_snapshot(), AgentKey::Claude);
        (toggle.on_click.as_ref().unwrap())();
        let is_on_now = agent_enabled(&shared.lock().unwrap().ui_snapshot(), AgentKey::Claude);
        assert_ne!(was_on, is_on_now);
    }

    #[test]
    fn agent_toggle_click_does_not_touch_toggles_map() {
        // refs #107 - before the split, agent toggles wrote to `state.toggles`.
        // After the split they must only touch `state.agents`.
        let shared = make_shared();
        let snap = make_snapshot();
        let el = build_agents_section(&snap, &shared);
        let claude_row = &el.children[4];
        let toggle = &claude_row.children[2].children[1];

        let toggles_before = shared.lock().unwrap().toggles.clone();
        (toggle.on_click.as_ref().unwrap())();
        let toggles_after = shared.lock().unwrap().toggles.clone();
        assert_eq!(toggles_before, toggles_after);
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
        let el = toggle_button(true, ToggleKey::GlowEffect, &shared);
        assert!(el.classes.contains(&"toggle".to_string()));
        assert!(el.classes.contains(&"on".to_string()));
        assert!(el.on_click.is_some());
    }

    #[test]
    fn toggle_button_off_lacks_on_class() {
        let shared = make_shared();
        let el = toggle_button(false, ToggleKey::GlowEffect, &shared);
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
        let key = ToggleKey::StartMinimized;
        let el = toggle_button(false, key, &shared);
        assert!(!shared
            .lock()
            .unwrap()
            .toggles
            .get(&key)
            .copied()
            .unwrap_or(false));
        (el.on_click.as_ref().unwrap())();
        assert!(shared
            .lock()
            .unwrap()
            .toggles
            .get(&key)
            .copied()
            .unwrap_or(false));
        (el.on_click.as_ref().unwrap())();
        assert!(!shared
            .lock()
            .unwrap()
            .toggles
            .get(&key)
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
    fn stepper_without_callbacks_disables_buttons() {
        let el = stepper("1.4", None);
        assert!(el.classes.contains(&"stepper".to_string()));
        assert_eq!(el.children.len(), 3);
        let val = &el.children[1];
        assert_eq!(text_of(val), Some("1.4"));
        let dec = &el.children[0];
        let inc = &el.children[2];
        assert!(dec.classes.contains(&"disabled".to_string()));
        assert!(inc.classes.contains(&"disabled".to_string()));
        assert!(dec.on_click.is_none());
        assert!(inc.on_click.is_none());
    }

    #[test]
    fn stepper_with_callbacks_wires_handlers() {
        let callbacks = StepCallbacks {
            on_dec: Box::new(|| {}),
            on_inc: Box::new(|| {}),
        };
        let el = stepper("7", Some(callbacks));
        let dec = &el.children[0];
        let inc = &el.children[2];
        assert!(!dec.classes.contains(&"disabled".to_string()));
        assert!(dec.on_click.is_some());
        assert!(inc.on_click.is_some());
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
        // Separator is now CSS ::before, so only two key pills render.
        assert_eq!(keys.children.len(), 2);
        assert!(keys
            .children
            .iter()
            .all(|c| c.classes.contains(&"keybind-key".to_string())));
        assert_eq!(text_of(&keys.children[0]), Some("Ctrl"));
        assert_eq!(text_of(&keys.children[1]), Some("A"));
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
        let spec = AgentSpec {
            name: "test",
            path: "~/bin/test",
            status: AgentStatus::Idle,
            key: AgentKey::Amp,
        };
        let el = agent_row(&spec, true, &shared);
        assert!(el.classes.contains(&"agent-row".to_string()));
        assert_eq!(el.children.len(), 3);
        assert!(el.children[0].classes.contains(&"agent-icon".to_string()));
        assert!(el.children[1].classes.contains(&"agent-info".to_string()));
        assert!(el.children[2]
            .classes
            .contains(&"agent-controls".to_string()));
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
        // First row is the "Daemon sessions / Refresh" header row, then
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
        // rows[0] is header; rows[1] alive, rows[2] dead
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
        let refresh_row = &el.children[1]; // first is section title, second is header setting-row
        let refresh_btn = refresh_row
            .children
            .iter()
            .find(|c| c.id.as_deref() == Some("settings-sessions-refresh"))
            .expect("refresh button");
        // Invoking succeeds without panic; actual daemon call is a no-op
        // because no daemon is connected in unit tests.
        (refresh_btn.on_click.as_ref().unwrap())();
    }
}
