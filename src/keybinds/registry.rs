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
            "Ctrl+Shift+P".to_string(),
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
/// `Ctrl+V` and `Ctrl+Shift+V` both dispatch `terminal.paste` so the
/// user can paste clipboard text into the focused PTY using either
/// the conventional Windows binding or the Linux-terminal convention
/// where `Ctrl+Shift+V` sidesteps the shell's `Ctrl+V` literal-input
/// handling. Paste is a system binding rather than an editable action
/// because rebinding it would risk leaving the user with no way to
/// paste at all.
fn system_bindings() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("Escape".to_string(), "modal.close".to_string()),
        ("Ctrl+Shift+F".to_string(), "fps_overlay.toggle".to_string()),
        ("Ctrl+V".to_string(), "terminal.paste".to_string()),
        ("Ctrl+Shift+V".to_string(), "terminal.paste".to_string()),
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
        assert_eq!(find("Ctrl+Shift+P").as_deref(), Some("palette.toggle"));
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
