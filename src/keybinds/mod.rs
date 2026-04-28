//! Keybind action registry.
//!
//! Single source of truth that maps each user-facing action to:
//! - a stable snake_case id (used for JSON persistence),
//! - a dispatch command string (fed to `state::dispatch`),
//! - a default key combo, and
//! - a human label for the Settings UI.

pub mod loader;
pub mod registry;
pub mod state;

pub use state::{KeybindError, KeybindErrorKind, KeybindsState};

use unshit::core::shortcut::KeyCombo;

/// A user-facing action that can be bound to a key combo.
///
/// Order of variants in `ALL` is the order shown in Settings > Keybinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeybindAction {
    NewTerminal,
    CloseTab,
    SplitRight,
    SplitDown,
    Unsplit,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NextTab,
    PrevTab,
    CommandPalette,
    ToggleSidebar,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    Fullscreen,
}

impl KeybindAction {
    /// Every variant, in display order.
    pub const ALL: &'static [KeybindAction] = &[
        Self::NewTerminal,
        Self::CloseTab,
        Self::SplitRight,
        Self::SplitDown,
        Self::Unsplit,
        Self::FocusLeft,
        Self::FocusRight,
        Self::FocusUp,
        Self::FocusDown,
        Self::NextTab,
        Self::PrevTab,
        Self::CommandPalette,
        Self::ToggleSidebar,
        Self::OpenSettings,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::Fullscreen,
    ];

    /// Stable snake_case identifier for JSON serialization.
    pub fn id(self) -> &'static str {
        match self {
            Self::NewTerminal => "new_terminal",
            Self::CloseTab => "close_tab",
            Self::SplitRight => "split_right",
            Self::SplitDown => "split_down",
            Self::Unsplit => "unsplit",
            Self::FocusLeft => "focus_left",
            Self::FocusRight => "focus_right",
            Self::FocusUp => "focus_up",
            Self::FocusDown => "focus_down",
            Self::NextTab => "next_tab",
            Self::PrevTab => "prev_tab",
            Self::CommandPalette => "command_palette",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::OpenSettings => "open_settings",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::Fullscreen => "fullscreen",
        }
    }

    /// Parse a stable id back to an action.
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.id() == s)
    }

    /// Human-readable label for the Settings UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewTerminal => "New terminal",
            Self::CloseTab => "Close tab",
            Self::SplitRight => "Split right",
            Self::SplitDown => "Split down",
            Self::Unsplit => "Unsplit",
            Self::FocusLeft => "Focus pane left",
            Self::FocusRight => "Focus pane right",
            Self::FocusUp => "Focus pane up",
            Self::FocusDown => "Focus pane down",
            Self::NextTab => "Next tab",
            Self::PrevTab => "Previous tab",
            Self::CommandPalette => "Command palette",
            Self::ToggleSidebar => "Toggle sidebar",
            Self::OpenSettings => "Settings",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::Fullscreen => "Fullscreen",
        }
    }

    /// Dispatch command string fed to `state::dispatch`.
    ///
    /// `Fullscreen` points at `window.toggle_fullscreen` which has no arm
    /// yet; that wiring is out of scope for A1 and picked up later.
    pub fn dispatch_command(self) -> &'static str {
        match self {
            Self::NewTerminal => "tab.new",
            Self::CloseTab => "tab.close.active",
            Self::SplitRight => "pane.split_right",
            Self::SplitDown => "pane.split_down",
            Self::Unsplit => "pane.close",
            Self::FocusLeft => "pane.focus_left",
            Self::FocusRight => "pane.focus_right",
            Self::FocusUp => "pane.focus_up",
            Self::FocusDown => "pane.focus_down",
            Self::NextTab => "tab.next",
            Self::PrevTab => "tab.prev",
            Self::CommandPalette => "palette.toggle",
            Self::ToggleSidebar => "sidebar.toggle",
            Self::OpenSettings => "modal.open",
            Self::ZoomIn => "font.inc",
            Self::ZoomOut => "font.dec",
            Self::Fullscreen => "window.toggle_fullscreen",
        }
    }

    /// Default key combo as a parsable string (Windows/Linux conventions).
    pub fn default_combo_str(self) -> &'static str {
        match self {
            Self::NewTerminal => "Ctrl+T",
            Self::CloseTab => "Ctrl+Shift+W",
            Self::SplitRight => "Ctrl+D",
            Self::SplitDown => "Ctrl+Shift+D",
            Self::Unsplit => "Ctrl+W",
            Self::FocusLeft => "Ctrl+Left",
            Self::FocusRight => "Ctrl+Right",
            Self::FocusUp => "Ctrl+Up",
            Self::FocusDown => "Ctrl+Down",
            Self::NextTab => "Ctrl+Tab",
            Self::PrevTab => "Ctrl+Shift+Tab",
            Self::CommandPalette => "Ctrl+K",
            Self::ToggleSidebar => "Ctrl+B",
            Self::OpenSettings => "Ctrl+,",
            Self::ZoomIn => "Ctrl+=",
            Self::ZoomOut => "Ctrl+-",
            Self::Fullscreen => "F11",
        }
    }

    /// Parsed default key combo.
    pub fn default_combo(self) -> KeyCombo {
        KeyCombo::parse(self.default_combo_str()).expect("default combo must parse")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_has_seventeen_variants() {
        assert_eq!(KeybindAction::ALL.len(), 17);
    }

    #[test]
    fn ids_are_unique() {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for action in KeybindAction::ALL {
            assert!(seen.insert(action.id()), "duplicate id for {:?}", action);
        }
    }

    #[test]
    fn ids_are_snake_case() {
        for action in KeybindAction::ALL {
            let id = action.id();
            assert!(!id.is_empty(), "empty id for {:?}", action);
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "id '{}' is not snake_case",
                id
            );
        }
    }

    #[test]
    fn from_id_roundtrip() {
        for action in KeybindAction::ALL {
            assert_eq!(KeybindAction::from_id(action.id()), Some(*action));
        }
    }

    #[test]
    fn from_id_unknown_returns_none() {
        assert_eq!(KeybindAction::from_id("not_a_real_action"), None);
        assert_eq!(KeybindAction::from_id(""), None);
    }

    #[test]
    fn default_combos_parse() {
        for action in KeybindAction::ALL {
            let s = action.default_combo_str();
            KeyCombo::parse(s).unwrap_or_else(|e| {
                panic!(
                    "default combo '{}' for {:?} failed to parse: {}",
                    s, action, e
                )
            });
        }
    }

    #[test]
    fn default_combos_are_unique() {
        let mut seen: HashSet<KeyCombo> = HashSet::new();
        for action in KeybindAction::ALL {
            assert!(
                seen.insert(action.default_combo()),
                "duplicate default combo for {:?}",
                action
            );
        }
    }

    #[test]
    fn labels_are_non_empty() {
        for action in KeybindAction::ALL {
            assert!(!action.label().is_empty(), "empty label for {:?}", action);
        }
    }

    #[test]
    fn dispatch_commands_are_non_empty() {
        for action in KeybindAction::ALL {
            let cmd = action.dispatch_command();
            assert!(!cmd.is_empty(), "empty dispatch command for {:?}", action);
            assert!(
                !cmd.contains(' '),
                "dispatch command '{}' has whitespace",
                cmd
            );
        }
    }

    /// Spot-check against actual arms in `state::dispatch`. Fullscreen is
    /// excluded: its arm is out of scope for A1.
    #[test]
    fn dispatch_commands_match_state_rs() {
        assert_eq!(KeybindAction::NewTerminal.dispatch_command(), "tab.new");
        assert_eq!(
            KeybindAction::CloseTab.dispatch_command(),
            "tab.close.active"
        );
        assert_eq!(
            KeybindAction::SplitRight.dispatch_command(),
            "pane.split_right"
        );
        assert_eq!(
            KeybindAction::SplitDown.dispatch_command(),
            "pane.split_down"
        );
        assert_eq!(KeybindAction::Unsplit.dispatch_command(), "pane.close");
        assert_eq!(
            KeybindAction::FocusLeft.dispatch_command(),
            "pane.focus_left"
        );
        assert_eq!(
            KeybindAction::FocusRight.dispatch_command(),
            "pane.focus_right"
        );
        assert_eq!(KeybindAction::FocusUp.dispatch_command(), "pane.focus_up");
        assert_eq!(
            KeybindAction::FocusDown.dispatch_command(),
            "pane.focus_down"
        );
        assert_eq!(KeybindAction::NextTab.dispatch_command(), "tab.next");
        assert_eq!(KeybindAction::PrevTab.dispatch_command(), "tab.prev");
        assert_eq!(
            KeybindAction::CommandPalette.dispatch_command(),
            "palette.toggle"
        );
        assert_eq!(
            KeybindAction::ToggleSidebar.dispatch_command(),
            "sidebar.toggle"
        );
        assert_eq!(KeybindAction::OpenSettings.dispatch_command(), "modal.open");
        assert_eq!(KeybindAction::ZoomIn.dispatch_command(), "font.inc");
        assert_eq!(KeybindAction::ZoomOut.dispatch_command(), "font.dec");
    }
}
