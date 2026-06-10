//! Runtime keybind state: user overrides, recording mode, last error.
//!
//! Stored sparsely: `overrides` only contains entries that differ from
//! the default. `effective(action)` returns the override if present,
//! otherwise the default from `KeybindAction::default_combo`.

use unshit::core::shortcut::KeyCombo;

use super::loader::UserKeybinds;
use super::KeybindAction;

/// Reason a keybind update failed. Surfaces inline in Settings so the
/// user sees which action conflicts or which combo was malformed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindErrorKind {
    /// The combo already belongs to another action's effective binding.
    Conflict { other: KeybindAction, combo: String },
    /// The combo string failed to parse.
    InvalidCombo { combo: String, message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindError {
    pub action: KeybindAction,
    pub kind: KeybindErrorKind,
}

#[derive(Clone, Debug, Default)]
pub struct KeybindsState {
    /// Only entries that differ from defaults. A missing entry means
    /// "use `action.default_combo()`".
    pub overrides: UserKeybinds,
    /// Action currently capturing the next key combo, if any.
    pub recording: Option<KeybindAction>,
    /// Last validation failure, cleared on the next successful set or
    /// when the user starts a new recording.
    pub error: Option<KeybindError>,
    /// Live filter text from the Keybinds page toolbar. Matched
    /// case-insensitively against action labels, descriptions, and combo
    /// key names. Not persisted.
    pub filter: String,
}

impl KeybindsState {
    /// Build from user-supplied overrides (e.g. loaded from disk).
    pub fn with_overrides(overrides: UserKeybinds) -> Self {
        Self {
            overrides,
            recording: None,
            error: None,
            filter: String::new(),
        }
    }

    /// Effective combo for an action: override if present, otherwise
    /// the default.
    pub fn effective(&self, action: KeybindAction) -> KeyCombo {
        self.overrides
            .get(&action)
            .copied()
            .unwrap_or_else(|| action.default_combo())
    }

    /// If `combo` is already bound to a different action, return that
    /// action. Otherwise `None`.
    pub fn conflict(&self, combo: KeyCombo, except: KeybindAction) -> Option<KeybindAction> {
        for action in KeybindAction::ALL {
            if *action == except {
                continue;
            }
            if self.effective(*action) == combo {
                return Some(*action);
            }
        }
        None
    }

    /// Apply an override. Returns Ok on success or Err when the combo
    /// conflicts. Does not persist: the caller wires persistence.
    pub fn set(&mut self, action: KeybindAction, combo: KeyCombo) -> Result<(), KeybindError> {
        if let Some(other) = self.conflict(combo, action) {
            let err = KeybindError {
                action,
                kind: KeybindErrorKind::Conflict {
                    other,
                    combo: combo.to_string(),
                },
            };
            self.error = Some(err.clone());
            return Err(err);
        }
        if combo == action.default_combo() {
            self.overrides.remove(&action);
        } else {
            self.overrides.insert(action, combo);
        }
        self.recording = None;
        self.error = None;
        Ok(())
    }

    /// Drop the override for `action` so it reverts to the default.
    pub fn reset(&mut self, action: KeybindAction) {
        self.overrides.remove(&action);
        self.error = None;
    }

    /// Drop every override.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
        self.recording = None;
        self.error = None;
    }

    /// Begin recording a new combo for `action`. Clears any pending
    /// error so the UI shows a clean state while the user types.
    pub fn start_recording(&mut self, action: KeybindAction) {
        self.recording = Some(action);
        self.error = None;
    }

    pub fn cancel_recording(&mut self) {
        self.recording = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit::core::event::{Key, Modifiers};

    fn combo(s: &str) -> KeyCombo {
        KeyCombo::parse(s).unwrap()
    }

    #[test]
    fn effective_uses_default_when_no_override() {
        let state = KeybindsState::default();
        assert_eq!(state.effective(KeybindAction::NewTerminal), combo("Ctrl+T"));
    }

    #[test]
    fn command_palette_effective_uses_ctrl_shift_p_default() {
        let state = KeybindsState::default();
        assert_eq!(
            state.effective(KeybindAction::CommandPalette),
            combo("Ctrl+Shift+P")
        );
    }

    #[test]
    fn effective_returns_override_when_set() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::NewTerminal, combo("Ctrl+Shift+T"))
            .unwrap();
        assert_eq!(
            state.effective(KeybindAction::NewTerminal),
            combo("Ctrl+Shift+T")
        );
    }

    #[test]
    fn command_palette_override_remains_editable() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::CommandPalette, combo("Alt+P"))
            .unwrap();

        assert_eq!(
            state.effective(KeybindAction::CommandPalette),
            combo("Alt+P")
        );
        assert!(state.overrides.contains_key(&KeybindAction::CommandPalette));
    }

    #[test]
    fn set_rejects_conflict_with_other_default() {
        // Ctrl+W is Unsplit's default. Trying to bind NewTerminal to
        // Ctrl+W must be rejected.
        let mut state = KeybindsState::default();
        let result = state.set(KeybindAction::NewTerminal, combo("Ctrl+W"));
        assert!(result.is_err());
        assert!(state.error.is_some());
        assert!(!state.overrides.contains_key(&KeybindAction::NewTerminal));
    }

    #[test]
    fn set_rejects_conflict_with_other_override() {
        let mut state = KeybindsState::default();
        let unusual = KeyCombo::new(Key::F(7), Modifiers::CTRL | Modifiers::ALT);
        state.set(KeybindAction::NewTerminal, unusual).unwrap();

        let result = state.set(KeybindAction::CloseTab, unusual);
        assert!(result.is_err());
        match state.error.as_ref().map(|e| &e.kind) {
            Some(KeybindErrorKind::Conflict { other, .. }) => {
                assert_eq!(*other, KeybindAction::NewTerminal);
            }
            other => panic!("expected Conflict error, got {:?}", other),
        }
    }

    #[test]
    fn set_allows_rebinding_same_action_to_different_combo() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::NewTerminal, combo("Ctrl+Shift+T"))
            .unwrap();
        state
            .set(KeybindAction::NewTerminal, combo("Alt+N"))
            .unwrap();
        assert_eq!(state.effective(KeybindAction::NewTerminal), combo("Alt+N"));
    }

    #[test]
    fn set_to_default_drops_override() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::NewTerminal, combo("Alt+N"))
            .unwrap();
        assert!(state.overrides.contains_key(&KeybindAction::NewTerminal));
        state
            .set(KeybindAction::NewTerminal, combo("Ctrl+T"))
            .unwrap();
        assert!(
            !state.overrides.contains_key(&KeybindAction::NewTerminal),
            "setting to default should drop the override"
        );
    }

    #[test]
    fn set_clears_recording_and_error() {
        let mut state = KeybindsState::default();
        state.start_recording(KeybindAction::NewTerminal);
        state.error = Some(KeybindError {
            action: KeybindAction::NewTerminal,
            kind: KeybindErrorKind::InvalidCombo {
                combo: "x".into(),
                message: "stale".into(),
            },
        });
        state
            .set(KeybindAction::NewTerminal, combo("Alt+N"))
            .unwrap();
        assert!(state.recording.is_none());
        assert!(state.error.is_none());
    }

    #[test]
    fn reset_restores_default() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::NewTerminal, combo("Alt+N"))
            .unwrap();
        state.reset(KeybindAction::NewTerminal);
        assert!(!state.overrides.contains_key(&KeybindAction::NewTerminal));
        assert_eq!(state.effective(KeybindAction::NewTerminal), combo("Ctrl+T"));
    }

    #[test]
    fn reset_all_drops_every_override() {
        let mut state = KeybindsState::default();
        state
            .set(KeybindAction::NewTerminal, combo("Alt+N"))
            .unwrap();
        state.set(KeybindAction::CloseTab, combo("Alt+W")).unwrap();
        state.reset_all();
        assert!(state.overrides.is_empty());
    }

    #[test]
    fn start_recording_sets_flag_and_clears_error() {
        let mut state = KeybindsState::default();
        state.error = Some(KeybindError {
            action: KeybindAction::NewTerminal,
            kind: KeybindErrorKind::InvalidCombo {
                combo: "x".into(),
                message: "stale".into(),
            },
        });
        state.start_recording(KeybindAction::CloseTab);
        assert_eq!(state.recording, Some(KeybindAction::CloseTab));
        assert!(state.error.is_none());
    }

    #[test]
    fn cancel_recording_clears_flag() {
        let mut state = KeybindsState::default();
        state.start_recording(KeybindAction::NewTerminal);
        state.cancel_recording();
        assert!(state.recording.is_none());
    }

    #[test]
    fn conflict_ignores_except_self() {
        let state = KeybindsState::default();
        // Ctrl+T is NewTerminal's default; asking "does anyone other
        // than NewTerminal have Ctrl+T?" must return None.
        assert!(state
            .conflict(combo("Ctrl+T"), KeybindAction::NewTerminal)
            .is_none());
    }
}
