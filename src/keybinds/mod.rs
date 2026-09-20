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
    NewAgent,
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
    QuickPromptOpen,
    RenameSession,
    OpenFile,
    QuickOpen,
    DiffOpen,
    EditorSave,
    ToggleExplorer,
    ToggleSidebar,
    OpenSettings,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Fullscreen,
}

/// Display groups for the Keybinds settings page, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeybindGroup {
    Panes,
    Tabs,
    Navigation,
    Application,
}

impl KeybindGroup {
    /// Every group, in display order.
    pub const ALL: &'static [KeybindGroup] =
        &[Self::Panes, Self::Tabs, Self::Navigation, Self::Application];

    /// Section heading shown on the Keybinds settings page.
    pub fn title(self) -> &'static str {
        match self {
            Self::Panes => "Panes",
            Self::Tabs => "Tabs & Sessions",
            Self::Navigation => "Navigation",
            Self::Application => "Application",
        }
    }
}

impl KeybindAction {
    /// Every variant, in display order.
    pub const ALL: &'static [KeybindAction] = &[
        Self::NewTerminal,
        Self::NewAgent,
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
        Self::QuickPromptOpen,
        Self::RenameSession,
        Self::OpenFile,
        Self::QuickOpen,
        Self::DiffOpen,
        Self::EditorSave,
        Self::ToggleExplorer,
        Self::ToggleSidebar,
        Self::OpenSettings,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomReset,
        Self::Fullscreen,
    ];

    /// Stable snake_case identifier for JSON serialization.
    pub fn id(self) -> &'static str {
        match self {
            Self::NewTerminal => "new_terminal",
            Self::NewAgent => "new_agent",
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
            Self::QuickPromptOpen => "quick_prompt_open",
            Self::RenameSession => "rename_session",
            Self::OpenFile => "open_file",
            Self::QuickOpen => "quick_open",
            Self::DiffOpen => "diff_open",
            Self::EditorSave => "editor_save",
            Self::ToggleExplorer => "toggle_explorer",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::OpenSettings => "open_settings",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::ZoomReset => "zoom_reset",
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
            Self::NewAgent => "New agent",
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
            Self::QuickPromptOpen => "Quick prompt",
            Self::RenameSession => "Rename session",
            Self::OpenFile => "Open file",
            Self::QuickOpen => "Quick open",
            Self::DiffOpen => "Diff against…",
            Self::EditorSave => "Save file",
            Self::ToggleExplorer => "Toggle file explorer",
            Self::ToggleSidebar => "Toggle sidebar",
            Self::OpenSettings => "Settings",
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::ZoomReset => "Reset zoom",
            Self::Fullscreen => "Fullscreen",
        }
    }

    /// One-line description shown under the label in the Settings UI.
    pub fn description(self) -> &'static str {
        match self {
            Self::NewTerminal => "Open a new terminal tab",
            Self::NewAgent => {
                "Open a new tab running the default agent CLI in the active workspace"
            }
            Self::CloseTab => "Close the current tab",
            Self::SplitRight => "Open a new pane to the right",
            Self::SplitDown => "Open a new pane below",
            Self::Unsplit => "Close the focused split",
            Self::FocusLeft => "Move focus to the pane on the left",
            Self::FocusRight => "Move focus to the pane on the right",
            Self::FocusUp => "Move focus to the pane above",
            Self::FocusDown => "Move focus to the pane below",
            Self::NextTab => "Switch to the next tab",
            Self::PrevTab => "Switch to the previous tab",
            Self::CommandPalette => "Open the fuzzy command runner",
            Self::QuickPromptOpen => "Open the quick prompt",
            Self::RenameSession => "Edit the active session label",
            Self::OpenFile => "Open a file in the built-in editor",
            Self::QuickOpen => "Find a file in the workspace by name and open it",
            Self::DiffOpen => "Review a git range as a read-only diff pane",
            Self::EditorSave => "Save the focused editor pane",
            Self::ToggleExplorer => "Show or hide the file explorer",
            Self::ToggleSidebar => "Show or hide the workspace sidebar",
            Self::OpenSettings => "Open the settings window",
            Self::ZoomIn => "Scale the whole interface up, terminal and chrome together",
            Self::ZoomOut => "Scale the whole interface down, terminal and chrome together",
            Self::ZoomReset => "Return the interface to 100%",
            Self::Fullscreen => "Toggle the fullscreen window",
        }
    }

    /// Section the action is listed under on the Keybinds settings page.
    pub fn group(self) -> KeybindGroup {
        match self {
            Self::SplitRight
            | Self::SplitDown
            | Self::Unsplit
            | Self::FocusLeft
            | Self::FocusRight
            | Self::FocusUp
            | Self::FocusDown => KeybindGroup::Panes,
            Self::NewTerminal
            | Self::NewAgent
            | Self::CloseTab
            | Self::NextTab
            | Self::PrevTab
            | Self::RenameSession => KeybindGroup::Tabs,
            Self::CommandPalette | Self::QuickPromptOpen | Self::QuickOpen => {
                KeybindGroup::Navigation
            }
            Self::DiffOpen
            | Self::OpenFile
            | Self::EditorSave
            | Self::ToggleExplorer
            | Self::ToggleSidebar
            | Self::OpenSettings
            | Self::ZoomIn
            | Self::ZoomOut
            | Self::ZoomReset
            | Self::Fullscreen => KeybindGroup::Application,
        }
    }

    /// Dispatch command string fed to `state::dispatch`.
    ///
    /// `Fullscreen` points at `window.toggle_fullscreen` which has no arm
    /// yet; that wiring is out of scope for A1 and picked up later.
    pub fn dispatch_command(self) -> &'static str {
        match self {
            Self::NewTerminal => "tab.new",
            Self::NewAgent => "agent.new",
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
            Self::QuickPromptOpen => "quick_prompt.open",
            Self::RenameSession => "session.rename_active",
            Self::OpenFile => "editor.open",
            Self::QuickOpen => "palette.files",
            Self::DiffOpen => "diff.open",
            Self::EditorSave => "editor.save",
            Self::ToggleExplorer => "explorer.toggle",
            Self::ToggleSidebar => "sidebar.toggle",
            Self::OpenSettings => "modal.open",
            // Zoom is framework-owned: `zoom.*` is handled inside
            // unshit-app so one factor scales spacing, borders, icons
            // and text together. The per-surface font steppers in
            // Settings still dispatch `terminal_font.*` / `config_font.*`.
            Self::ZoomIn => "zoom.in",
            Self::ZoomOut => "zoom.out",
            Self::ZoomReset => "zoom.reset",
            Self::Fullscreen => "window.toggle_fullscreen",
        }
    }

    /// Default key combo as a parsable string for the current platform.
    pub fn default_combo_str(self) -> &'static str {
        #[cfg(target_os = "macos")]
        let combo = match self {
            Self::NewTerminal => "Meta+T",
            Self::NewAgent => "Shift+Meta+A",
            Self::CloseTab => "Shift+Meta+W",
            Self::SplitRight => "Meta+D",
            Self::SplitDown => "Shift+Meta+D",
            Self::Unsplit => "Meta+W",
            // Command is reserved for app shortcuts on macOS, so terminal
            // Control/Option word-navigation chords still reach the PTY.
            Self::FocusLeft => "Alt+Meta+Left",
            Self::FocusRight => "Alt+Meta+Right",
            Self::FocusUp => "Alt+Meta+Up",
            Self::FocusDown => "Alt+Meta+Down",
            // Cmd+Tab belongs to macOS app switching. These match common
            // browser/editor tab navigation without shadowing the OS.
            Self::NextTab => "Shift+Meta+]",
            Self::PrevTab => "Shift+Meta+[",
            Self::CommandPalette => "Shift+Meta+P",
            // Cmd+Shift+Q is macOS Log Out; use the non-reserved I chord.
            Self::QuickPromptOpen => "Shift+Meta+I",
            Self::RenameSession => "F2",
            Self::OpenFile => "Meta+O",
            Self::QuickOpen => "Meta+P",
            Self::DiffOpen => "Shift+Meta+G",
            Self::EditorSave => "Meta+S",
            Self::ToggleExplorer => "Meta+B",
            Self::ToggleSidebar => "Shift+Meta+B",
            Self::OpenSettings => "Meta+,",
            Self::ZoomIn => "Meta+=",
            Self::ZoomOut => "Meta+-",
            Self::ZoomReset => "Meta+0",
            Self::Fullscreen => "Ctrl+Meta+F",
        };

        #[cfg(not(target_os = "macos"))]
        let combo = match self {
            Self::NewTerminal => "Ctrl+T",
            // Ctrl+Shift chord: plain Ctrl+A must keep reaching the
            // terminal (readline line-start, select-all in TUIs).
            Self::NewAgent => "Ctrl+Shift+A",
            Self::CloseTab => "Ctrl+Shift+W",
            Self::SplitRight => "Ctrl+D",
            Self::SplitDown => "Ctrl+Shift+D",
            Self::Unsplit => "Ctrl+W",
            // Reserve Ctrl+Arrow for word navigation in terminal applications.
            Self::FocusLeft => "Ctrl+Alt+Left",
            Self::FocusRight => "Ctrl+Alt+Right",
            Self::FocusUp => "Ctrl+Alt+Up",
            Self::FocusDown => "Ctrl+Alt+Down",
            Self::NextTab => "Ctrl+Tab",
            Self::PrevTab => "Ctrl+Shift+Tab",
            Self::CommandPalette => "Ctrl+Shift+P",
            Self::QuickPromptOpen => "Ctrl+Shift+Q",
            Self::RenameSession => "F2",
            // Ctrl+Shift chords: plain Ctrl+O / Ctrl+S must keep
            // reaching terminal programs (nano, readline, XOFF).
            Self::OpenFile => "Ctrl+Shift+O",
            // Zed and VS Code put quick open on Ctrl+P, which readline
            // uses for "previous command" in every shell this app hosts.
            // The rule that moved Open file and Save off plain Ctrl+O /
            // Ctrl+S applies here too: Ctrl+Shift+E matches VS Code's
            // Explorer and leaves the terminal alone. Rebind it to
            // Ctrl+P if you want the editor convention.
            Self::QuickOpen => "Ctrl+Shift+E",
            // Ctrl+Shift+G is Source Control in VS Code.
            Self::DiffOpen => "Ctrl+Shift+G",
            Self::EditorSave => "Ctrl+Shift+S",
            Self::ToggleExplorer => "Ctrl+B",
            Self::ToggleSidebar => "Ctrl+Shift+B",
            Self::OpenSettings => "Ctrl+,",
            Self::ZoomIn => "Ctrl+=",
            Self::ZoomOut => "Ctrl+-",
            // Ctrl+1..Ctrl+9 switch tabs, so Ctrl+0 is free for the
            // browser-conventional zoom reset.
            Self::ZoomReset => "Ctrl+0",
            Self::Fullscreen => "F11",
        };

        combo
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
    fn all_has_twenty_six_variants() {
        assert_eq!(KeybindAction::ALL.len(), 26);
    }

    #[test]
    fn explorer_owns_primary_b_without_conflicting_with_sidebar() {
        let (explorer, sidebar) = if cfg!(target_os = "macos") {
            ("Meta+B", "Shift+Meta+B")
        } else {
            ("Ctrl+B", "Ctrl+Shift+B")
        };
        assert_eq!(KeybindAction::ToggleExplorer.default_combo_str(), explorer);
        assert_eq!(
            KeybindAction::ToggleExplorer.dispatch_command(),
            "explorer.toggle"
        );
        assert_eq!(KeybindAction::ToggleSidebar.default_combo_str(), sidebar);
    }

    #[test]
    fn new_agent_is_a_primary_shift_chord_in_the_tabs_group() {
        #[cfg(target_os = "macos")]
        let expected = "Shift+Meta+A";
        #[cfg(not(target_os = "macos"))]
        let expected = "Ctrl+Shift+A";
        assert_eq!(KeybindAction::NewAgent.default_combo_str(), expected);
        assert_eq!(KeybindAction::NewAgent.dispatch_command(), "agent.new");
        assert_eq!(KeybindAction::NewAgent.group(), KeybindGroup::Tabs);
        assert_eq!(
            KeybindAction::from_id("new_agent"),
            Some(KeybindAction::NewAgent)
        );
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
    fn command_palette_default_combo_uses_primary_modifier() {
        #[cfg(target_os = "macos")]
        let expected = "Shift+Meta+P";
        #[cfg(not(target_os = "macos"))]
        let expected = "Ctrl+Shift+P";
        assert_eq!(KeybindAction::CommandPalette.default_combo_str(), expected);
    }

    #[test]
    fn quick_prompt_default_avoids_macos_log_out_chord() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            KeybindAction::QuickPromptOpen.default_combo_str(),
            "Shift+Meta+I"
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            KeybindAction::QuickPromptOpen.default_combo_str(),
            "Ctrl+Shift+Q"
        );
    }

    #[test]
    fn pane_focus_defaults_preserve_ctrl_arrow_for_terminal_navigation() {
        #[cfg(target_os = "macos")]
        let expected = [
            "Alt+Meta+Left",
            "Alt+Meta+Right",
            "Alt+Meta+Up",
            "Alt+Meta+Down",
        ];
        #[cfg(not(target_os = "macos"))]
        let expected = [
            "Ctrl+Alt+Left",
            "Ctrl+Alt+Right",
            "Ctrl+Alt+Up",
            "Ctrl+Alt+Down",
        ];
        assert_eq!(KeybindAction::FocusLeft.default_combo_str(), expected[0]);
        assert_eq!(KeybindAction::FocusRight.default_combo_str(), expected[1]);
        assert_eq!(KeybindAction::FocusUp.default_combo_str(), expected[2]);
        assert_eq!(KeybindAction::FocusDown.default_combo_str(), expected[3]);
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
            KeybindAction::QuickPromptOpen.dispatch_command(),
            "quick_prompt.open"
        );
        assert_eq!(
            KeybindAction::RenameSession.dispatch_command(),
            "session.rename_active"
        );
        assert_eq!(
            KeybindAction::ToggleSidebar.dispatch_command(),
            "sidebar.toggle"
        );
        assert_eq!(KeybindAction::OpenSettings.dispatch_command(), "modal.open");
        assert_eq!(KeybindAction::ZoomIn.dispatch_command(), "zoom.in");
        assert_eq!(KeybindAction::ZoomOut.dispatch_command(), "zoom.out");
        assert_eq!(KeybindAction::ZoomReset.dispatch_command(), "zoom.reset");
    }
}
