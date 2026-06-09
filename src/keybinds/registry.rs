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
//!    `Ctrl+1` through `Ctrl+9` to jump to a tab, and the clipboard
//!    paste keybind (`Ctrl+V` / `Ctrl+Shift+V` -> `terminal.paste`).

use super::loader::UserKeybinds;
use super::KeybindAction;

/// Number of `Ctrl+N` tab-switch bindings (one per numeric key 1..=9).
const TAB_SWITCH_COUNT: usize = 9;

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
            "Ctrl+Shift+H".to_string(),
            KeybindAction::SplitRight.dispatch_command().to_string(),
        ),
        (
            "Ctrl+K".to_string(),
            KeybindAction::CommandPalette.dispatch_command().to_string(),
        ),
        (
            "Ctrl+Shift+=".to_string(),
            KeybindAction::ZoomIn.dispatch_command().to_string(),
        ),
    ]
}

/// Non-editable system shortcuts. These don't appear in Settings >
/// Keybinds; they're hard-wired.
///
/// `Ctrl+V`, `Ctrl+Shift+V`, and `Shift+Insert` all dispatch
/// `terminal.paste` so the user can paste clipboard text into the focused
/// PTY using the conventional Windows binding, the Linux-terminal
/// convention where `Ctrl+Shift+V` sidesteps the shell's `Ctrl+V`
/// literal-input handling, or the classic `Shift+Insert`. `Ctrl+Shift+C`
/// dispatches `terminal.copy` (the unconditional copy; a bare `Ctrl+C`
/// only copies when a selection exists and is handled in the terminal's
/// keyboard handler so it still sends an interrupt otherwise). These are
/// system bindings rather than editable actions because rebinding them
/// would risk leaving the user with no way to copy or paste at all.
fn system_bindings() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("Escape".to_string(), "modal.close".to_string()),
        ("Ctrl+Shift+F".to_string(), "fps_overlay.toggle".to_string()),
        ("Ctrl+V".to_string(), "terminal.paste".to_string()),
        ("Ctrl+Shift+V".to_string(), "terminal.paste".to_string()),
        ("Shift+Insert".to_string(), "terminal.paste".to_string()),
        ("Ctrl+Shift+C".to_string(), "terminal.copy".to_string()),
    ];
    for i in 0..TAB_SWITCH_COUNT {
        out.push((format!("Ctrl+{}", i + 1), format!("tab.switch:{}", i)));
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
    fn ctrl_v_dispatches_terminal_paste() {
        // Conventional Windows paste binding routed through the
        // app-level paste action so the focused PTY receives the text.
        assert_eq!(find("Ctrl+V").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn ctrl_shift_v_aliases_terminal_paste() {
        // Linux-terminal convention: Ctrl+Shift+V pastes so the shell's
        // Ctrl+V literal-input handler is not shadowed.
        assert_eq!(find("Ctrl+Shift+V").as_deref(), Some("terminal.paste"));
    }

    #[test]
    fn ctrl_shift_h_aliases_split_right() {
        // tmux convention: H stacks panes horizontally -> new pane beside.
        assert_eq!(find("Ctrl+Shift+H").as_deref(), Some("pane.split_right"));
    }

    #[test]
    fn ctrl_w_closes_focused_pane() {
        // In a split tab, Ctrl+W should close just the focused pane and
        // only fall through to closing the tab when that pane was the
        // last one (pane.close has the cascade built in).
        assert_eq!(find("Ctrl+W").as_deref(), Some("pane.close"));
    }

    #[test]
    fn f2_renames_active_session() {
        assert_eq!(find("F2").as_deref(), Some("session.rename_active"));
    }

    #[test]
    fn ctrl_shift_w_closes_active_tab() {
        // Ctrl+Shift+W forcibly closes the whole tab regardless of how
        // many panes it holds.
        assert_eq!(find("Ctrl+Shift+W").as_deref(), Some("tab.close.active"));
    }

    #[test]
    fn escape_closes_modals() {
        assert_eq!(find("Escape").as_deref(), Some("modal.close"));
    }

    #[test]
    fn ctrl_digits_switch_tabs() {
        for i in 0..TAB_SWITCH_COUNT {
            let combo = format!("Ctrl+{}", i + 1);
            assert_eq!(
                find(&combo).as_deref(),
                Some(format!("tab.switch:{}", i).as_str())
            );
        }
    }

    #[test]
    fn palette_alias_registered() {
        assert_eq!(find("Ctrl+K").as_deref(), Some("palette.toggle"));
    }

    #[test]
    fn ctrl_shift_p_registered_once_as_palette_default() {
        let matches = pairs()
            .into_iter()
            .filter(|(combo, cmd)| combo == "Ctrl+Shift+P" && cmd == "palette.toggle")
            .count();

        assert_eq!(matches, 1);
    }

    #[test]
    fn zoom_in_alias_registered() {
        assert_eq!(find("Ctrl+Shift+=").as_deref(), Some("font.inc"));
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
    fn ctrl_shift_c_copies() {
        // Unconditional copy command. Bare Ctrl+C is handled in the
        // terminal keyboard handler and is conditional (only copies if
        // a selection exists, otherwise sends SIGINT).
        assert_eq!(find("Ctrl+Shift+C").as_deref(), Some("terminal.copy"));
    }

    #[test]
    fn ctrl_v_and_ctrl_shift_v_both_paste() {
        // Both conventional Windows (Ctrl+V) and Linux-terminal (Ctrl+Shift+V)
        // paste bindings are present and map to the same action.
        assert_eq!(find("Ctrl+V").as_deref(), Some("terminal.paste"));
        assert_eq!(find("Ctrl+Shift+V").as_deref(), Some("terminal.paste"));
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
        let copy_paste_combos = vec!["Ctrl+V", "Ctrl+Shift+V", "Shift+Insert", "Ctrl+Shift+C"];
        for combo in copy_paste_combos {
            let found = bindings.iter().any(|(c, _)| c == combo);
            assert!(found, "copy/paste binding {} must be present", combo);
        }
    }
}
