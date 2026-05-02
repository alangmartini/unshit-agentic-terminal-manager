//! State for the Quick Prompt overlay.
//!
//! Slice 1 keeps this minimal: the overlay is either open or closed.
//! Subsequent slices add prompt text, agent picker, image attachments,
//! autocomplete, and submit error messages. Keeping the field shape
//! small here means consumers do not have to opt into placeholders that
//! the spec removes later.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickPromptState {}

impl QuickPromptState {
    /// Construct the default open state. Distinct constructor so future
    /// slices can preload the persisted agent without churning callers.
    pub fn open_default() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_default_is_default() {
        assert_eq!(
            QuickPromptState::open_default(),
            QuickPromptState::default()
        );
    }
}
