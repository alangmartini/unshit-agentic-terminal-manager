use crate::element::Tag;
use crate::event::{Key, Modifiers};
use std::collections::HashMap;
use std::fmt;

/// A single key + modifiers combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl KeyCombo {
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// Plain key with no modifiers.
    pub fn plain(key: Key) -> Self {
        Self { key, modifiers: Modifiers::empty() }
    }

    /// Creates a combo using Ctrl on Windows/Linux, Cmd on macOS.
    pub fn command(key: Key) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self { key, modifiers: Modifiers::META }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self { key, modifiers: Modifiers::CTRL }
        }
    }

    /// Parse a string like "Ctrl+S", "Shift+Tab", or "Escape" into a `KeyCombo`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty key combo".to_string());
        }
        let parts: Vec<&str> = s.split('+').collect();
        let key_str = parts.last().unwrap().trim();
        let key = Key::from_name(key_str).ok_or_else(|| format!("unknown key: {}", key_str))?;
        let mut modifiers = Modifiers::empty();
        for part in &parts[..parts.len() - 1] {
            let m = Modifiers::parse_name(part.trim())
                .ok_or_else(|| format!("unknown modifier: {}", part.trim()))?;
            modifiers |= m;
        }
        Ok(KeyCombo::new(key, modifiers))
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.modifiers, self.key)
    }
}

/// A full shortcut: either a single combo or a two-step chord.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shortcut {
    Single(KeyCombo),
    Chord(KeyCombo, KeyCombo),
}

impl Shortcut {
    /// Parse a shortcut string. Chords use ", " (comma space) as separator.
    /// Examples: "Ctrl+S", "Ctrl+K, Ctrl+C".
    pub fn parse(s: &str) -> Result<Self, String> {
        if let Some((a, b)) = s.split_once(", ") {
            let first = KeyCombo::parse(a)?;
            let second = KeyCombo::parse(b)?;
            Ok(Shortcut::Chord(first, second))
        } else {
            Ok(Shortcut::Single(KeyCombo::parse(s)?))
        }
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shortcut::Single(combo) => write!(f, "{}", combo),
            Shortcut::Chord(a, b) => write!(f, "{}, {}", a, b),
        }
    }
}

/// Contextual condition for when a shortcut is active.
#[derive(Clone, Debug)]
pub enum WhenClause {
    Always,
    ContextFlag(String),
    FocusedTag(Tag),
    FocusedClass(String),
    And(Vec<WhenClause>),
    Not(Box<WhenClause>),
}

impl WhenClause {
    /// Parse a when-clause string.
    /// - "always" -> Always
    /// - "!something" -> Not(parse the rest)
    /// - "focusedTag:button" -> FocusedTag(Tag::Button)
    /// - "focusedClass:primary" -> FocusedClass("primary")
    /// - anything else -> ContextFlag(s)
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("always") {
            return Ok(WhenClause::Always);
        }
        if let Some(rest) = s.strip_prefix('!') {
            let inner = WhenClause::parse(rest)?;
            return Ok(WhenClause::Not(Box::new(inner)));
        }
        if let Some(tag_name) = s.strip_prefix("focusedTag:") {
            let tag =
                Tag::from_str(tag_name).ok_or_else(|| format!("unknown tag: {}", tag_name))?;
            return Ok(WhenClause::FocusedTag(tag));
        }
        if let Some(class_name) = s.strip_prefix("focusedClass:") {
            return Ok(WhenClause::FocusedClass(class_name.to_string()));
        }
        Ok(WhenClause::ContextFlag(s.to_string()))
    }
}

/// Priority level for binding resolution. Higher values win.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BindingPriority {
    Default = 0,
    Extension = 1,
    User = 2,
}

/// A registered keybinding rule.
pub struct KeyBinding {
    pub shortcut: Shortcut,
    pub command: String,
    pub when: WhenClause,
    pub priority: BindingPriority,
}

/// Result of looking up a key combo in the registry.
pub enum LookupResult<'a> {
    /// No bindings matched.
    None,
    /// One or more bindings matched (may also be a chord leader).
    Matched { bindings: Vec<&'a KeyBinding>, is_chord_leader: bool },
    /// This combo is only a chord leader (no single-key bindings).
    ChordLeader,
}

/// Stores keybindings in HashMaps for O(1) lookup.
pub struct ShortcutRegistry {
    /// Single-key bindings: KeyCombo -> list of bindings sorted by priority.
    singles: HashMap<KeyCombo, Vec<KeyBinding>>,
    /// Chord bindings: first combo -> second combo -> list of bindings.
    chords: HashMap<KeyCombo, HashMap<KeyCombo, Vec<KeyBinding>>>,
}

impl ShortcutRegistry {
    pub fn new() -> Self {
        Self { singles: HashMap::new(), chords: HashMap::new() }
    }

    /// Register a new keybinding.
    pub fn register(&mut self, binding: KeyBinding) {
        match &binding.shortcut {
            Shortcut::Single(combo) => {
                let entries = self.singles.entry(*combo).or_default();
                entries.push(binding);
                entries.sort_by(|a, b| b.priority.cmp(&a.priority));
            }
            Shortcut::Chord(leader, follower) => {
                let inner = self.chords.entry(*leader).or_default();
                let entries = inner.entry(*follower).or_default();
                entries.push(binding);
                entries.sort_by(|a, b| b.priority.cmp(&a.priority));
            }
        }
    }

    /// Remove all bindings that match the given shortcut.
    pub fn unregister(&mut self, shortcut: &Shortcut) {
        match shortcut {
            Shortcut::Single(combo) => {
                self.singles.remove(combo);
            }
            Shortcut::Chord(leader, follower) => {
                if let Some(inner) = self.chords.get_mut(leader) {
                    inner.remove(follower);
                    if inner.is_empty() {
                        self.chords.remove(leader);
                    }
                }
            }
        }
    }

    /// Look up a single key combo. Returns matching bindings and whether
    /// this combo is also a chord leader.
    pub fn lookup(&self, combo: &KeyCombo) -> LookupResult<'_> {
        let has_singles = self.singles.get(combo);
        let is_chord_leader = self.chords.contains_key(combo);

        match (has_singles, is_chord_leader) {
            (Some(bindings), true) => {
                LookupResult::Matched { bindings: bindings.iter().collect(), is_chord_leader: true }
            }
            (Some(bindings), false) => LookupResult::Matched {
                bindings: bindings.iter().collect(),
                is_chord_leader: false,
            },
            (None, true) => LookupResult::ChordLeader,
            (None, false) => LookupResult::None,
        }
    }

    /// Look up the second step of a chord.
    pub fn lookup_chord(&self, leader: &KeyCombo, follower: &KeyCombo) -> Vec<&KeyBinding> {
        self.chords
            .get(leader)
            .and_then(|inner| inner.get(follower))
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }
}

impl Default for ShortcutRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(key: Key, mods: Modifiers, cmd: &str) -> KeyBinding {
        KeyBinding {
            shortcut: Shortcut::Single(KeyCombo::new(key, mods)),
            command: cmd.to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        }
    }

    #[test]
    fn register_and_lookup_single() {
        let mut reg = ShortcutRegistry::new();
        reg.register(bind(Key::Tab, Modifiers::empty(), "focus.next"));

        let combo = KeyCombo::plain(Key::Tab);
        match reg.lookup(&combo) {
            LookupResult::Matched { bindings, is_chord_leader } => {
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].command, "focus.next");
                assert!(!is_chord_leader);
            }
            _ => panic!("expected Matched"),
        }
    }

    #[test]
    fn lookup_miss_returns_none() {
        let reg = ShortcutRegistry::new();
        assert!(matches!(reg.lookup(&KeyCombo::plain(Key::Escape)), LookupResult::None));
    }

    #[test]
    fn higher_priority_wins() {
        let mut reg = ShortcutRegistry::new();
        let combo = KeyCombo::command(Key::Char('s'));

        reg.register(KeyBinding {
            shortcut: Shortcut::Single(combo),
            command: "default.save".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });
        reg.register(KeyBinding {
            shortcut: Shortcut::Single(combo),
            command: "user.save".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::User,
        });

        match reg.lookup(&combo) {
            LookupResult::Matched { bindings, .. } => {
                assert_eq!(bindings[0].command, "user.save");
                assert_eq!(bindings[1].command, "default.save");
            }
            _ => panic!("expected Matched"),
        }
    }

    #[test]
    fn unregister_removes_binding() {
        let mut reg = ShortcutRegistry::new();
        let combo = KeyCombo::plain(Key::Escape);
        reg.register(bind(Key::Escape, Modifiers::empty(), "close"));

        reg.unregister(&Shortcut::Single(combo));
        assert!(matches!(reg.lookup(&combo), LookupResult::None));
    }

    #[test]
    fn chord_leader_detected() {
        let mut reg = ShortcutRegistry::new();
        let leader = KeyCombo::command(Key::Char('k'));
        let follower = KeyCombo::command(Key::Char('c'));

        reg.register(KeyBinding {
            shortcut: Shortcut::Chord(leader, follower),
            command: "editor.comment".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });

        assert!(matches!(reg.lookup(&leader), LookupResult::ChordLeader));
        let results = reg.lookup_chord(&leader, &follower);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].command, "editor.comment");
    }

    #[test]
    fn platform_command_helper() {
        let combo = KeyCombo::command(Key::Char('s'));
        #[cfg(target_os = "macos")]
        assert_eq!(combo.modifiers, Modifiers::META);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(combo.modifiers, Modifiers::CTRL);
    }

    // --- Key::from_name tests ---

    #[test]
    fn key_from_name_enter() {
        assert_eq!(Key::from_name("enter"), Some(Key::Enter));
        assert_eq!(Key::from_name("ENTER"), Some(Key::Enter));
    }

    #[test]
    fn key_from_name_escape_aliases() {
        assert_eq!(Key::from_name("escape"), Some(Key::Escape));
        assert_eq!(Key::from_name("esc"), Some(Key::Escape));
    }

    #[test]
    fn key_from_name_char() {
        assert_eq!(Key::from_name("s"), Some(Key::Char('s')));
        assert_eq!(Key::from_name("S"), Some(Key::Char('s')));
        assert_eq!(Key::from_name("1"), Some(Key::Char('1')));
        assert_eq!(Key::from_name("/"), Some(Key::Char('/')));
    }

    #[test]
    fn key_from_name_f_key() {
        assert_eq!(Key::from_name("f1"), Some(Key::F(1)));
        assert_eq!(Key::from_name("F12"), Some(Key::F(12)));
    }

    #[test]
    fn key_from_name_arrows() {
        assert_eq!(Key::from_name("up"), Some(Key::ArrowUp));
        assert_eq!(Key::from_name("down"), Some(Key::ArrowDown));
        assert_eq!(Key::from_name("left"), Some(Key::ArrowLeft));
        assert_eq!(Key::from_name("right"), Some(Key::ArrowRight));
    }

    #[test]
    fn key_from_name_unknown() {
        assert_eq!(Key::from_name("xyz123"), None);
        assert_eq!(Key::from_name(""), None);
    }

    #[test]
    fn key_from_name_delete_aliases() {
        assert_eq!(Key::from_name("delete"), Some(Key::Delete));
        assert_eq!(Key::from_name("del"), Some(Key::Delete));
    }

    // --- Modifiers::parse_name tests ---

    #[test]
    fn modifier_from_name() {
        assert_eq!(Modifiers::parse_name("ctrl"), Some(Modifiers::CTRL));
        assert_eq!(Modifiers::parse_name("control"), Some(Modifiers::CTRL));
        assert_eq!(Modifiers::parse_name("shift"), Some(Modifiers::SHIFT));
        assert_eq!(Modifiers::parse_name("alt"), Some(Modifiers::ALT));
        assert_eq!(Modifiers::parse_name("meta"), Some(Modifiers::META));
        assert_eq!(Modifiers::parse_name("super"), Some(Modifiers::META));
        assert_eq!(Modifiers::parse_name("cmd"), Some(Modifiers::META));
        assert_eq!(Modifiers::parse_name("command"), Some(Modifiers::META));
        assert_eq!(Modifiers::parse_name("bogus"), None);
    }

    // --- KeyCombo::parse tests ---

    #[test]
    fn parse_combo_simple() {
        assert_eq!(KeyCombo::parse("Escape").unwrap(), KeyCombo::plain(Key::Escape));
    }

    #[test]
    fn parse_combo_with_modifier() {
        assert_eq!(
            KeyCombo::parse("Ctrl+S").unwrap(),
            KeyCombo::new(Key::Char('s'), Modifiers::CTRL)
        );
    }

    #[test]
    fn parse_combo_multi_modifier() {
        let combo = KeyCombo::parse("Ctrl+Shift+S").unwrap();
        assert_eq!(combo.key, Key::Char('s'));
        assert_eq!(combo.modifiers, Modifiers::CTRL | Modifiers::SHIFT);
    }

    #[test]
    fn parse_combo_empty_is_error() {
        assert!(KeyCombo::parse("").is_err());
    }

    #[test]
    fn parse_combo_unknown_key_is_error() {
        assert!(KeyCombo::parse("Ctrl+xyz123").is_err());
    }

    #[test]
    fn parse_combo_unknown_modifier_is_error() {
        assert!(KeyCombo::parse("Bogus+S").is_err());
    }

    // --- Shortcut::parse tests ---

    #[test]
    fn parse_shortcut_single() {
        let shortcut = Shortcut::parse("Ctrl+S").unwrap();
        assert_eq!(shortcut, Shortcut::Single(KeyCombo::new(Key::Char('s'), Modifiers::CTRL)));
    }

    #[test]
    fn parse_shortcut_chord() {
        let shortcut = Shortcut::parse("Ctrl+K, Ctrl+C").unwrap();
        assert_eq!(
            shortcut,
            Shortcut::Chord(
                KeyCombo::new(Key::Char('k'), Modifiers::CTRL),
                KeyCombo::new(Key::Char('c'), Modifiers::CTRL),
            )
        );
    }

    // --- WhenClause::parse tests ---

    #[test]
    fn parse_when_always() {
        assert!(matches!(WhenClause::parse("always").unwrap(), WhenClause::Always));
        assert!(matches!(WhenClause::parse("ALWAYS").unwrap(), WhenClause::Always));
    }

    #[test]
    fn parse_when_context_flag() {
        match WhenClause::parse("inputFocused").unwrap() {
            WhenClause::ContextFlag(s) => assert_eq!(s, "inputFocused"),
            other => panic!("expected ContextFlag, got {:?}", other),
        }
    }

    #[test]
    fn parse_when_not() {
        match WhenClause::parse("!editing").unwrap() {
            WhenClause::Not(inner) => match *inner {
                WhenClause::ContextFlag(s) => assert_eq!(s, "editing"),
                other => panic!("expected ContextFlag inside Not, got {:?}", other),
            },
            other => panic!("expected Not, got {:?}", other),
        }
    }

    #[test]
    fn parse_when_focused_tag() {
        match WhenClause::parse("focusedTag:button").unwrap() {
            WhenClause::FocusedTag(tag) => assert_eq!(tag, Tag::Button),
            other => panic!("expected FocusedTag, got {:?}", other),
        }
    }

    #[test]
    fn parse_when_focused_tag_unknown_is_error() {
        assert!(WhenClause::parse("focusedTag:nonexistent").is_err());
    }

    #[test]
    fn parse_when_focused_class() {
        match WhenClause::parse("focusedClass:primary").unwrap() {
            WhenClause::FocusedClass(s) => assert_eq!(s, "primary"),
            other => panic!("expected FocusedClass, got {:?}", other),
        }
    }

    // --- Round-trip tests ---

    #[test]
    fn parse_combo_roundtrip() {
        let combos = vec![
            KeyCombo::plain(Key::Escape),
            KeyCombo::plain(Key::Enter),
            KeyCombo::plain(Key::F(5)),
            KeyCombo::new(Key::Char('s'), Modifiers::CTRL),
            KeyCombo::new(Key::Char('s'), Modifiers::CTRL | Modifiers::SHIFT),
            KeyCombo::new(Key::Tab, Modifiers::ALT),
        ];
        for combo in combos {
            let displayed = combo.to_string();
            let parsed = KeyCombo::parse(&displayed).unwrap();
            assert_eq!(parsed, combo, "round-trip failed for '{}'", displayed);
        }
    }

    #[test]
    fn parse_shortcut_roundtrip() {
        let shortcuts = vec![
            Shortcut::Single(KeyCombo::new(Key::Char('s'), Modifiers::CTRL)),
            Shortcut::Chord(
                KeyCombo::new(Key::Char('k'), Modifiers::CTRL),
                KeyCombo::new(Key::Char('c'), Modifiers::CTRL),
            ),
        ];
        for shortcut in shortcuts {
            let displayed = shortcut.to_string();
            let parsed = Shortcut::parse(&displayed).unwrap();
            assert_eq!(parsed, shortcut, "round-trip failed for '{}'", displayed);
        }
    }
}
