//! Build the initial `(combo, dispatch_command)` list registered with
//! the framework's shortcut resolver.
//!
//! Entries come from three sources:
//! 1. `KeybindAction::ALL` defaults (the editable set surfaced in
//!    Settings > Keybinds).
//! 2. Aliases for a handful of actions (e.g. `Ctrl+Shift+H` as an
//!    alias for split right) so muscle memory from other terminals
//!    works.
//! 3. Non-editable system shortcuts: `Escape` to close modals,
//!    the platform's primary-modifier tab switches, and clipboard
//!    copy/paste bindings.

use super::loader::UserKeybinds;
use super::KeybindAction;

/// Number of primary-modifier tab-switch bindings (one per numeric key 1..=9).
const TAB_SWITCH_COUNT: usize = 9;

#[cfg(target_os = "macos")]
const PRIMARY_MODIFIER: &str = "Meta";
#[cfg(not(target_os = "macos"))]
const PRIMARY_MODIFIER: &str = "Ctrl";

fn primary_combo(key: &str) -> String {
    format!("{PRIMARY_MODIFIER}+{key}")
}

/// Build the full list of `(combo, dispatch_command)` pairs to register
/// with the framework on startup, with user overrides applied.
///
/// The framework snapshots these at build time, so changes to
/// `overrides` after startup do not propagate until the next run.
pub fn shortcut_bindings_with_overrides(overrides: &UserKeybinds) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    for action in KeybindAction::ALL {
        let combo = overrides
            .get(action)
            .copied()
            .unwrap_or_else(|| action.default_combo());
        out.push((combo.to_string(), action.dispatch_command().to_string()));
    }

    out.extend(alias_bindings());
    out.extend(system_bindings());
    out
}

/// Build the bindings list with defaults only (no user overrides).
pub fn default_shortcut_bindings() -> Vec<(String, String)> {
    shortcut_bindings_with_overrides(&UserKeybinds::new())
}

/// Convenience aliases that map a second combo to an existing action's
/// dispatch command.
///
/// `Ctrl+Shift+H` follows the tmux convention where H means "stack
/// panes horizontally" (so the new pane lands beside the current one).
fn alias_bindings() -> Vec<(String, String)> {
    vec![
        (
            primary_combo("Shift+H"),
            KeybindAction::SplitRight.dispatch_command().to_string(),
        ),
        (
            primary_combo("K"),
            KeybindAction::CommandPalette.dispatch_command().to_string(),
        ),
        (
            primary_combo("Shift+="),
            KeybindAction::ZoomIn.dispatch_command().to_string(),
        ),
    ]
}

/// Non-editable system shortcuts. These don't appear in Settings >
/// Keybinds; they're hard-wired.
///
/// The primary modifier's copy/paste bindings dispatch the app-level
/// clipboard commands. On macOS the terminal's bare `Ctrl+V` remains
/// unbound so it reaches the PTY's literal-input handling; the
/// `Ctrl+Shift+V` and `Ctrl+Shift+C` variants remain available for users
/// who rely on terminal conventions. A bare `Ctrl+C` is never registered:
/// the terminal keyboard handler conditionally copies a live selection and
/// otherwise lets the interrupt byte reach the shell.
fn system_bindings() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![("Escape".to_string(), "modal.close".to_string())];

    out.push((primary_combo("Shift+F"), "fps_overlay.toggle".to_string()));
    out.push((primary_combo("V"), "terminal.paste".to_string()));
    out.push(("Ctrl+Shift+V".to_string(), "terminal.paste".to_string()));
    out.push(("Shift+Insert".to_string(), "terminal.paste".to_string()));
    #[cfg(target_os = "macos")]
    out.push((primary_combo("C"), "terminal.copy".to_string()));
    out.push(("Ctrl+Shift+C".to_string(), "terminal.copy".to_string()));

    for i in 0..TAB_SWITCH_COUNT {
        out.push((
            primary_combo(&(i + 1).to_string()),
            format!("tab.switch:{}", i),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use unshit::core::shortcut::KeyCombo;

    fn pairs() -> Vec<(String, String)> {
        default_shortcut_bindings()
    }

    fn find(combo: &str) -> Option<String> {
        pairs()
            .into_iter()
            .find(|(c, _)| c == combo)
            .map(|(_, cmd)| cmd)
    }

    #[test]
    fn every_action_has_its_default_combo_registered() {
        for action in KeybindAction::ALL {
            let cmd = find(action.default_combo_str()).unwrap_or_else(|| {
                panic!(
                    "default combo '{}' for {:?} not in bindings",
                    action.default_combo_str(),
                    action
                )
            });
            assert_eq!(
                cmd,
                action.dispatch_command(),
                "combo '{}' should dispatch '{}' for {:?}",
                action.default_combo_str(),
                action.dispatch_command(),
                action
            );
        }
    }

    #[test]
    fn pane_focus_defaults_use_primary_alt_arrows() {
        #[cfg(target_os = "macos")]
        let expected = [
            ("Alt+Meta+Left", "pane.focus_left"),
            ("Alt+Meta+Right", "pane.focus_right"),
            ("Alt+Meta+Up", "pane.focus_up"),
            ("Alt+Meta+Down", "pane.focus_down"),
        ];
        #[cfg(not(target_os = "macos"))]
        let expected = [
            ("Ctrl+Alt+Left", "pane.focus_left"),
            ("Ctrl+Alt+Right", "pane.focus_right"),
            ("Ctrl+Alt+Up", "pane.focus_up"),
            ("Ctrl+Alt+Down", "pane.focus_down"),
        ];

        for (combo, command) in expected {
            assert_eq!(find(combo).as_deref(), Some(command));
        }

        // Bare Ctrl+arrows remain available for terminal word navigation on
        // every platform; Cmd+arrows are only consumed by editable actions
        // when an action explicitly requests them.
        for combo in ["Ctrl+Left", "Ctrl+Right", "Ctrl+Up", "Ctrl+Down"] {
            assert!(find(combo).is_none(), "{combo} must reach the terminal");
        }
    }

    #[test]
    fn primary_v_dispatches_terminal_paste() {
        assert_eq!(find(&primary_combo("V")).as_deref(), Some("terminal.paste"));
        #[cfg(target_os = "macos")]
        assert!(
            find("Ctrl+V").is_none(),
            "Ctrl+V must reach the PTY on macOS"
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(find("Ctrl+V").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn ctrl_shift_v_aliases_terminal_paste() {
        // Linux-terminal convention: Ctrl+Shift+V pastes so the shell's
        // Ctrl+V literal-input handler is not shadowed.
        assert_eq!(find("Ctrl+Shift+V").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn primary_shift_h_aliases_split_right() {
        // tmux convention: H stacks panes horizontally -> new pane beside.
        assert_eq!(
            find(&primary_combo("Shift+H")).as_deref(),
            Some("pane.split_right")
        );
    }

    #[test]
    fn unsplit_default_is_registered() {
        assert_eq!(
            find(KeybindAction::Unsplit.default_combo_str()).as_deref(),
            Some("pane.close")
        );
    }

    #[test]
    fn f2_renames_active_session() {
        assert_eq!(find("F2").as_deref(), Some("session.rename_active"));
    }

    #[test]
    fn primary_shift_w_closes_active_tab() {
        // Primary+Shift+W forcibly closes the whole tab regardless of how
        // many panes it holds.
        assert_eq!(
            find(KeybindAction::CloseTab.default_combo_str()).as_deref(),
            Some("tab.close.active")
        );
    }

    #[test]
    fn escape_closes_modals() {
        assert_eq!(find("Escape").as_deref(), Some("modal.close"));
    }

    #[test]
    fn primary_digits_switch_tabs() {
        for i in 0..TAB_SWITCH_COUNT {
            let combo = primary_combo(&(i + 1).to_string());
            assert_eq!(
                find(&combo).as_deref(),
                Some(format!("tab.switch:{}", i).as_str())
            );
        }
    }

    #[test]
    fn palette_alias_registered() {
        assert_eq!(find(&primary_combo("K")).as_deref(), Some("palette.toggle"));
    }

    #[test]
    fn ctrl_shift_p_registered_once_as_palette_default() {
        let matches = pairs()
            .into_iter()
            .filter(|(combo, cmd)| {
                combo == KeybindAction::CommandPalette.default_combo_str()
                    && cmd == "palette.toggle"
            })
            .count();

        assert_eq!(matches, 1);
    }

    #[test]
    fn zoom_in_alias_registered() {
        assert_eq!(find(&primary_combo("Shift+=")).as_deref(), Some("zoom.in"));
    }

    #[test]
    fn every_combo_is_parsable() {
        for (combo, _) in pairs() {
            KeyCombo::parse(&combo)
                .unwrap_or_else(|e| panic!("combo '{}' failed to parse: {}", combo, e));
        }
    }

    #[test]
    fn combos_are_unique() {
        let mut seen: HashSet<String> = HashSet::new();
        for (combo, _) in pairs() {
            assert!(seen.insert(combo.clone()), "duplicate combo: {}", combo);
        }
    }
}

#[cfg(test)]
mod tests_copy_paste_bindings {
    use super::*;

    fn pairs() -> Vec<(String, String)> {
        default_shortcut_bindings()
    }

    fn find(combo: &str) -> Option<String> {
        pairs()
            .into_iter()
            .find(|(c, _)| c == combo)
            .map(|(_, cmd)| cmd)
    }

    #[test]
    fn shift_insert_pastes() {
        // Classic paste binding on systems where it is not intercepted
        // by a TUI. System binding so it does not pollute the editable
        // actions list.
        assert_eq!(find("Shift+Insert").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn copy_variants_are_registered() {
        // Unconditional copy command. Bare Ctrl+C is handled in the
        // terminal keyboard handler and is conditional (only copies if
        // a selection exists, otherwise sends SIGINT).
        assert_eq!(find(&primary_combo("C")).as_deref(), Some("terminal.copy"));
        assert_eq!(find("Ctrl+Shift+C").as_deref(), Some("terminal.copy"));
    }

    #[test]
    fn primary_v_and_ctrl_shift_v_both_paste() {
        // The primary paste binding and Linux-terminal Ctrl+Shift+V map to
        // the same action. On macOS bare Ctrl+V intentionally remains free
        // for the PTY.
        assert_eq!(find(&primary_combo("V")).as_deref(), Some("terminal.paste"));
        assert_eq!(find("Ctrl+Shift+V").as_deref(), Some("terminal.paste"));
        #[cfg(target_os = "macos")]
        assert!(find("Ctrl+V").is_none());
        #[cfg(not(target_os = "macos"))]
        assert_eq!(find("Ctrl+V").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn no_bare_ctrl_c_binding_exists() {
        // Ctrl+C MUST NOT be a static shortcut binding. It is handled
        // conditionally in the terminal's keyboard handler: copy if a
        // selection exists, otherwise send SIGINT (0x03) to the shell.
        // If it were a static shortcut, it would always dispatch "terminal.copy"
        // and never reach the shell, breaking interrupt handling.
        let cmd = find("Ctrl+C");
        assert!(
            cmd.is_none(),
            "bare Ctrl+C must not be in bindings (found: {:?}); it is handled conditionally",
            cmd
        );
    }

    #[test]
    fn copy_paste_bindings_are_in_system_not_editable_actions() {
        // Verify that copy/paste bindings do not interfere with the
        // editable KeybindAction list. This is a sanity check that
        // system_bindings() is responsible for these, not the
        // configurable action list.
        let bindings = pairs();
        let mut copy_paste_combos = vec![
            primary_combo("V"),
            "Ctrl+Shift+V".to_string(),
            "Shift+Insert".to_string(),
            "Ctrl+Shift+C".to_string(),
        ];
        if cfg!(target_os = "macos") {
            copy_paste_combos.push(primary_combo("C"));
        }
        for combo in copy_paste_combos {
            let found = bindings.iter().any(|(c, _)| c == &combo);
            assert!(found, "copy/paste binding {} must be present", combo);
        }
    }
}
