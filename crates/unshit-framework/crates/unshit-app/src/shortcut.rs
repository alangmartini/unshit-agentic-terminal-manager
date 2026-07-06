use serde::Deserialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use unshit_core::element::Tag;
use unshit_core::event::{InteractionState, Key, Modifiers};
use unshit_core::shortcut::{
    BindingPriority, KeyBinding, KeyCombo, LookupResult, Shortcut, ShortcutRegistry, WhenClause,
};
use unshit_core::tree::NodeArena;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// Build a KeyCombo from winit's logical key and modifier state.
pub fn key_combo_from_winit(
    logical_key: &WinitKey,
    modifiers: &ModifiersState,
) -> Option<KeyCombo> {
    let key = match logical_key {
        WinitKey::Named(named) => match named {
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Tab => Key::Tab,
            NamedKey::ArrowUp => Key::ArrowUp,
            NamedKey::ArrowDown => Key::ArrowDown,
            NamedKey::ArrowLeft => Key::ArrowLeft,
            NamedKey::ArrowRight => Key::ArrowRight,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Delete => Key::Delete,
            NamedKey::Insert => Key::Insert,
            NamedKey::F1 => Key::F(1),
            NamedKey::F2 => Key::F(2),
            NamedKey::F3 => Key::F(3),
            NamedKey::F4 => Key::F(4),
            NamedKey::F5 => Key::F(5),
            NamedKey::F6 => Key::F(6),
            NamedKey::F7 => Key::F(7),
            NamedKey::F8 => Key::F(8),
            NamedKey::F9 => Key::F(9),
            NamedKey::F10 => Key::F(10),
            NamedKey::F11 => Key::F(11),
            NamedKey::F12 => Key::F(12),
            _ => return None,
        },
        WinitKey::Character(s) => {
            let s = s.as_str();
            if s == " " {
                Key::Space
            } else {
                let mut chars = s.chars();
                let ch = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                Key::Char(ch.to_ascii_lowercase())
            }
        }
        _ => return None,
    };

    Some(KeyCombo::new(key, modifiers_from_winit(modifiers)))
}

/// Build a KeyCombo for a dead-key press that committed text.
///
/// Dead keys (`'` / `"` / `~` / `^` on US-International and ABNT2 layouts)
/// report `WinitKey::Dead`, which `key_combo_from_winit` cannot map. The
/// press that commits a literal character (the dead key pressed twice)
/// still reports `Dead` as its logical key but carries the committed text.
/// Map it to a `Char` combo so keyboard-capture handlers (e.g. a terminal)
/// receive the character instead of the event being dropped.
pub fn dead_key_commit_combo(
    logical_key: &WinitKey,
    text: Option<&str>,
    modifiers: &ModifiersState,
) -> Option<KeyCombo> {
    if !matches!(logical_key, WinitKey::Dead(_)) {
        return None;
    }
    let ch = text?.chars().next()?;
    if ch.is_control() {
        return None;
    }
    Some(KeyCombo::new(Key::Char(ch), modifiers_from_winit(modifiers)))
}

fn modifiers_from_winit(modifiers: &ModifiersState) -> Modifiers {
    let mut mods = Modifiers::empty();
    if modifiers.shift_key() {
        mods |= Modifiers::SHIFT;
    }
    if modifiers.control_key() {
        mods |= Modifiers::CTRL;
    }
    if modifiers.alt_key() {
        mods |= Modifiers::ALT;
    }
    if modifiers.meta_key() {
        mods |= Modifiers::META;
    }
    mods
}

// -- JSON config types --

#[derive(Deserialize)]
pub struct KeybindingsConfig {
    pub bindings: Vec<KeybindingEntry>,
}

#[derive(Deserialize)]
pub struct KeybindingEntry {
    pub key: String,
    pub command: String,
    #[serde(default = "default_when")]
    pub when: String,
}

fn default_when() -> String {
    "always".to_string()
}

// -- Parsing helpers --

/// Parse a single key name (case-insensitive) into a `Key`.
fn parse_key_name(s: &str) -> Result<Key, String> {
    match s.to_ascii_lowercase().as_str() {
        "enter" | "return" => Ok(Key::Enter),
        "escape" | "esc" => Ok(Key::Escape),
        "backspace" => Ok(Key::Backspace),
        "tab" => Ok(Key::Tab),
        "space" => Ok(Key::Space),
        "up" => Ok(Key::ArrowUp),
        "down" => Ok(Key::ArrowDown),
        "left" => Ok(Key::ArrowLeft),
        "right" => Ok(Key::ArrowRight),
        "home" => Ok(Key::Home),
        "end" => Ok(Key::End),
        "pageup" => Ok(Key::PageUp),
        "pagedown" => Ok(Key::PageDown),
        "delete" | "del" => Ok(Key::Delete),
        other => {
            // F1..F12
            if let Some(rest) = other.strip_prefix('f') {
                if let Ok(n) = rest.parse::<u8>() {
                    if (1..=12).contains(&n) {
                        return Ok(Key::F(n));
                    }
                }
            }
            // Single character
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Ok(Key::Char(ch.to_ascii_lowercase())),
                _ => Err(format!("unknown key name: '{}'", s)),
            }
        }
    }
}

/// Parse a modifier name (case-insensitive) into `Modifiers`.
fn parse_modifier(s: &str) -> Result<Modifiers, String> {
    match s.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(Modifiers::CTRL),
        "shift" => Ok(Modifiers::SHIFT),
        "alt" => Ok(Modifiers::ALT),
        "meta" | "super" | "cmd" | "command" => Ok(Modifiers::META),
        other => Err(format!("unknown modifier: '{}'", other)),
    }
}

/// Parse a string like "Ctrl+S" or "Escape" into a `KeyCombo`.
pub fn parse_key_combo(s: &str) -> Result<KeyCombo, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty key combo string".to_string());
    }
    let parts: Vec<&str> = s.split('+').collect();

    let key_part = parts.last().unwrap();
    let key = parse_key_name(key_part)?;

    let mut mods = Modifiers::empty();
    for &part in &parts[..parts.len() - 1] {
        mods |= parse_modifier(part.trim())?;
    }

    Ok(KeyCombo::new(key, mods))
}

/// Parse a shortcut string. Supports chords separated by ", ".
/// Examples: "Ctrl+S", "Ctrl+K, Ctrl+C"
pub fn parse_shortcut(s: &str) -> Result<Shortcut, String> {
    if let Some((first, second)) = s.split_once(", ") {
        let a = parse_key_combo(first)?;
        let b = parse_key_combo(second)?;
        Ok(Shortcut::Chord(a, b))
    } else {
        Ok(Shortcut::Single(parse_key_combo(s)?))
    }
}

/// Parse a when-clause string.
/// "always" -> Always, "!x" -> Not(parse x), "focusedTag:button" -> FocusedTag,
/// "focusedClass:name" -> FocusedClass, anything else -> ContextFlag.
pub fn parse_when(s: &str) -> Result<WhenClause, String> {
    let s = s.trim();
    if s == "always" {
        return Ok(WhenClause::Always);
    }
    if let Some(rest) = s.strip_prefix('!') {
        let inner = parse_when(rest)?;
        return Ok(WhenClause::Not(Box::new(inner)));
    }
    if let Some(tag_name) = s.strip_prefix("focusedTag:") {
        return Tag::from_str(tag_name)
            .map(WhenClause::FocusedTag)
            .ok_or_else(|| format!("unknown tag: '{}'", tag_name));
    }
    if let Some(class_name) = s.strip_prefix("focusedClass:") {
        return Ok(WhenClause::FocusedClass(class_name.to_string()));
    }
    Ok(WhenClause::ContextFlag(s.to_string()))
}

/// Load keybindings from a JSON string.
pub fn load_keybindings_from_json(json: &str) -> Result<Vec<KeyBinding>, String> {
    let config: KeybindingsConfig =
        serde_json::from_str(json).map_err(|e| format!("invalid keybindings JSON: {}", e))?;

    config
        .bindings
        .into_iter()
        .map(|entry| {
            let shortcut = parse_shortcut(&entry.key)?;
            let when = parse_when(&entry.when)?;
            Ok(KeyBinding {
                shortcut,
                command: entry.command,
                when,
                priority: BindingPriority::User,
            })
        })
        .collect()
}

/// Load keybindings from a JSON file on disk.
pub fn load_keybindings_from_file(path: &str) -> Result<Vec<KeyBinding>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read keybindings file {}: {}", path, e))?;
    load_keybindings_from_json(&content)
}

/// Resolves keyboard shortcuts against the registry, handling chord state
/// and when-clause evaluation.
pub struct ShortcutResolver {
    registry: ShortcutRegistry,
    pending_chord: Option<(KeyCombo, Instant)>,
    chord_timeout: Duration,
    context: HashSet<String>,
}

impl ShortcutResolver {
    pub fn new() -> Self {
        Self {
            registry: ShortcutRegistry::new(),
            pending_chord: None,
            chord_timeout: Duration::from_millis(1500),
            context: HashSet::new(),
        }
    }

    /// Access the underlying registry for adding/removing bindings.
    pub fn registry_mut(&mut self) -> &mut ShortcutRegistry {
        &mut self.registry
    }

    /// Process a keypress. Returns the command string if a shortcut matched.
    pub fn process_key(
        &mut self,
        combo: KeyCombo,
        interaction: &InteractionState,
        arena: &NodeArena,
    ) -> Option<String> {
        // Check if we're completing a chord
        if let Some((leader, timestamp)) = self.pending_chord.take() {
            if timestamp.elapsed() < self.chord_timeout {
                let candidates = self.registry.lookup_chord(&leader, &combo);
                if let Some(binding) =
                    first_matching(&candidates, &self.context, interaction, arena)
                {
                    return Some(binding.command.clone());
                }
            }
            // Chord timed out or no match on second step; fall through to
            // single-key lookup for this new combo.
        }

        match self.registry.lookup(&combo) {
            LookupResult::Matched { bindings, is_chord_leader } => {
                // Try to find a matching single-key binding
                if let Some(binding) = first_matching(&bindings, &self.context, interaction, arena)
                {
                    // If this is also a chord leader, we still resolve the
                    // single-key binding immediately (VS Code behavior for
                    // combos that are both single bindings and chord leaders).
                    return Some(binding.command.clone());
                }
                // No single binding matched, but if it's a chord leader, enter chord state
                if is_chord_leader {
                    self.pending_chord = Some((combo, Instant::now()));
                }
                None
            }
            LookupResult::ChordLeader => {
                self.pending_chord = Some((combo, Instant::now()));
                None
            }
            LookupResult::None => None,
        }
    }

    /// Set a context flag (used in WhenClause::ContextFlag evaluation).
    pub fn set_context(&mut self, flag: &str, value: bool) {
        if value {
            self.context.insert(flag.to_string());
        } else {
            self.context.remove(flag);
        }
    }

    /// Cancel any pending chord.
    pub fn cancel_chord(&mut self) {
        self.pending_chord = None;
    }

    /// Returns true if currently waiting for a chord's second key.
    pub fn is_chord_pending(&self) -> bool {
        self.pending_chord.is_some()
    }

    /// Returns the pending chord leader key combo, if any.
    pub fn pending_chord_leader(&self) -> Option<KeyCombo> {
        self.pending_chord.as_ref().map(|(combo, _)| *combo)
    }

    /// Returns a formatted display string for the pending chord, if any.
    /// Example: "Ctrl+K ..."
    pub fn pending_chord_display(&self) -> Option<String> {
        self.pending_chord_leader().map(|combo| format!("{} ...", combo))
    }

    /// Set the chord timeout duration.
    pub fn set_chord_timeout(&mut self, timeout: Duration) {
        self.chord_timeout = timeout;
    }

    /// Register default framework shortcuts (Tab focus cycling, etc.).
    pub fn register_defaults(&mut self) {
        self.registry.register(KeyBinding {
            shortcut: Shortcut::Single(KeyCombo::plain(Key::Tab)),
            command: "focus.next".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });
        self.registry.register(KeyBinding {
            shortcut: Shortcut::Single(KeyCombo::new(Key::Tab, Modifiers::SHIFT)),
            command: "focus.prev".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });
    }
}

impl Default for ShortcutResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Find the first binding whose when-clause matches the current context.
fn first_matching<'a>(
    bindings: &[&'a KeyBinding],
    context: &HashSet<String>,
    interaction: &InteractionState,
    arena: &NodeArena,
) -> Option<&'a KeyBinding> {
    bindings.iter().find(|b| evaluate_when(&b.when, context, interaction, arena)).copied()
}

/// Evaluate a WhenClause against the current context and focused element.
fn evaluate_when(
    clause: &WhenClause,
    context: &HashSet<String>,
    interaction: &InteractionState,
    arena: &NodeArena,
) -> bool {
    match clause {
        WhenClause::Always => true,
        WhenClause::ContextFlag(flag) => context.contains(flag),
        WhenClause::FocusedTag(tag) => {
            arena.get(interaction.focused).map(|elem| elem.tag == *tag).unwrap_or(false)
        }
        WhenClause::FocusedClass(class_name) => arena
            .get(interaction.focused)
            .map(|elem| elem.classes.iter().any(|c| c == class_name))
            .unwrap_or(false),
        WhenClause::And(clauses) => {
            clauses.iter().all(|c| evaluate_when(c, context, interaction, arena))
        }
        WhenClause::Not(inner) => !evaluate_when(inner, context, interaction, arena),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_interaction() -> InteractionState {
        InteractionState::default()
    }

    // -- dead_key_commit_combo -------------------------------------------------

    #[test]
    fn dead_key_with_committed_text_maps_to_char() {
        let combo = dead_key_commit_combo(
            &WinitKey::Dead(Some('\'')),
            Some("'"),
            &ModifiersState::default(),
        );
        assert_eq!(combo, Some(KeyCombo::new(Key::Char('\''), Modifiers::empty())));
    }

    #[test]
    fn dead_key_with_shift_keeps_modifier() {
        let combo =
            dead_key_commit_combo(&WinitKey::Dead(Some('"')), Some("\""), &ModifiersState::SHIFT);
        assert_eq!(combo, Some(KeyCombo::new(Key::Char('"'), Modifiers::SHIFT)));
    }

    #[test]
    fn dead_key_without_text_returns_none() {
        // The initial dead-key press composes silently; nothing to forward.
        let combo =
            dead_key_commit_combo(&WinitKey::Dead(Some('~')), None, &ModifiersState::default());
        assert_eq!(combo, None);
    }

    #[test]
    fn non_dead_key_returns_none() {
        let combo = dead_key_commit_combo(
            &WinitKey::Character("a".into()),
            Some("a"),
            &ModifiersState::default(),
        );
        assert_eq!(combo, None);
    }

    fn dummy_arena() -> NodeArena {
        NodeArena::new()
    }

    /// Standard Ctrl+K / Ctrl+C chord pair used across chord tests.
    fn chord_combos() -> (KeyCombo, KeyCombo) {
        (
            KeyCombo::new(Key::Char('k'), Modifiers::CTRL),
            KeyCombo::new(Key::Char('c'), Modifiers::CTRL),
        )
    }

    fn register_chord(
        resolver: &mut ShortcutResolver,
        leader: KeyCombo,
        follower: KeyCombo,
        command: &str,
    ) {
        resolver.registry_mut().register(KeyBinding {
            shortcut: Shortcut::Chord(leader, follower),
            command: command.to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });
    }

    #[test]
    fn process_single_key() {
        let mut resolver = ShortcutResolver::new();
        resolver.register_defaults();
        let arena = dummy_arena();
        let interaction = dummy_interaction();

        let result = resolver.process_key(KeyCombo::plain(Key::Tab), &interaction, &arena);
        assert_eq!(result.as_deref(), Some("focus.next"));

        let result =
            resolver.process_key(KeyCombo::new(Key::Tab, Modifiers::SHIFT), &interaction, &arena);
        assert_eq!(result.as_deref(), Some("focus.prev"));
    }

    #[test]
    fn unmatched_key_returns_none() {
        let mut resolver = ShortcutResolver::new();
        resolver.register_defaults();
        let arena = dummy_arena();
        let interaction = dummy_interaction();

        let result = resolver.process_key(KeyCombo::plain(Key::Escape), &interaction, &arena);
        assert_eq!(result, None);
    }

    #[test]
    fn chord_completion() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();
        let (leader, follower) = chord_combos();

        register_chord(&mut resolver, leader, follower, "editor.comment");

        let result = resolver.process_key(leader, &interaction, &arena);
        assert_eq!(result, None);
        assert!(resolver.is_chord_pending());

        let result = resolver.process_key(follower, &interaction, &arena);
        assert_eq!(result.as_deref(), Some("editor.comment"));
        assert!(!resolver.is_chord_pending());
    }

    #[test]
    fn chord_timeout() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();
        let (leader, follower) = chord_combos();

        // Zero timeout so chord expires immediately
        resolver.set_chord_timeout(Duration::ZERO);

        register_chord(&mut resolver, leader, follower, "editor.comment");

        resolver.registry_mut().register(KeyBinding {
            shortcut: Shortcut::Single(follower),
            command: "fallback.action".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });

        let result = resolver.process_key(leader, &interaction, &arena);
        assert_eq!(result, None);

        std::thread::sleep(Duration::from_millis(5));

        // Chord expired; falls through to single-key lookup
        let result = resolver.process_key(follower, &interaction, &arena);
        assert_eq!(result.as_deref(), Some("fallback.action"));
    }

    #[test]
    fn chord_escape_cancel() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();
        let (leader, follower) = chord_combos();

        register_chord(&mut resolver, leader, follower, "editor.comment");

        resolver.process_key(leader, &interaction, &arena);
        assert!(resolver.is_chord_pending());

        resolver.cancel_chord();
        assert!(!resolver.is_chord_pending());
    }

    #[test]
    fn chord_leader_with_single_binding() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();
        let (leader, follower) = chord_combos();

        resolver.registry_mut().register(KeyBinding {
            shortcut: Shortcut::Single(leader),
            command: "single.action".to_string(),
            when: WhenClause::Always,
            priority: BindingPriority::Default,
        });
        register_chord(&mut resolver, leader, follower, "editor.comment");

        // Single binding fires immediately; chord state is NOT entered
        let result = resolver.process_key(leader, &interaction, &arena);
        assert_eq!(result.as_deref(), Some("single.action"));
        assert!(!resolver.is_chord_pending());
    }

    #[test]
    fn display_key_formatting() {
        assert_eq!(format!("{}", Key::Char('s')), "S");
        assert_eq!(format!("{}", Key::Char('a')), "A");
        assert_eq!(format!("{}", Key::Enter), "Enter");
        assert_eq!(format!("{}", Key::Escape), "Escape");
        assert_eq!(format!("{}", Key::Tab), "Tab");
        assert_eq!(format!("{}", Key::Space), "Space");
        assert_eq!(format!("{}", Key::ArrowUp), "Up");
        assert_eq!(format!("{}", Key::ArrowDown), "Down");
        assert_eq!(format!("{}", Key::ArrowLeft), "Left");
        assert_eq!(format!("{}", Key::ArrowRight), "Right");
        assert_eq!(format!("{}", Key::Home), "Home");
        assert_eq!(format!("{}", Key::End), "End");
        assert_eq!(format!("{}", Key::PageUp), "PageUp");
        assert_eq!(format!("{}", Key::PageDown), "PageDown");
        assert_eq!(format!("{}", Key::Delete), "Delete");
        assert_eq!(format!("{}", Key::Backspace), "Backspace");
        assert_eq!(format!("{}", Key::F(1)), "F1");
        assert_eq!(format!("{}", Key::F(12)), "F12");
        assert_eq!(format!("{}", Key::Unknown), "Unknown");
    }

    #[test]
    fn display_combo_formatting() {
        let ctrl_s = KeyCombo::new(Key::Char('s'), Modifiers::CTRL);
        assert_eq!(format!("{}", ctrl_s), "Ctrl+S");

        let shift_tab = KeyCombo::new(Key::Tab, Modifiers::SHIFT);
        assert_eq!(format!("{}", shift_tab), "Shift+Tab");

        let escape = KeyCombo::plain(Key::Escape);
        assert_eq!(format!("{}", escape), "Escape");

        let ctrl_alt_del = KeyCombo::new(Key::Delete, Modifiers::CTRL | Modifiers::ALT);
        assert_eq!(format!("{}", ctrl_alt_del), "Ctrl+Alt+Delete");

        let all_mods = KeyCombo::new(
            Key::Char('x'),
            Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META,
        );
        assert_eq!(format!("{}", all_mods), "Ctrl+Alt+Shift+Meta+X");
    }

    #[test]
    fn display_shortcut_chord() {
        let (leader, follower) = chord_combos();
        let chord = Shortcut::Chord(leader, follower);
        assert_eq!(format!("{}", chord), "Ctrl+K, Ctrl+C");

        let single = Shortcut::Single(KeyCombo::new(Key::Char('s'), Modifiers::CTRL));
        assert_eq!(format!("{}", single), "Ctrl+S");
    }

    #[test]
    fn pending_chord_display_returns_text() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();
        let (leader, follower) = chord_combos();

        register_chord(&mut resolver, leader, follower, "editor.comment");

        assert_eq!(resolver.pending_chord_display(), None);
        assert_eq!(resolver.pending_chord_leader(), None);

        resolver.process_key(leader, &interaction, &arena);

        assert_eq!(resolver.pending_chord_leader(), Some(leader));
        assert_eq!(resolver.pending_chord_display(), Some("Ctrl+K ...".to_string()));
    }

    #[test]
    fn context_flag_gating() {
        let mut resolver = ShortcutResolver::new();
        let arena = dummy_arena();
        let interaction = dummy_interaction();

        resolver.registry_mut().register(KeyBinding {
            shortcut: Shortcut::Single(KeyCombo::plain(Key::Escape)),
            command: "dialog.close".to_string(),
            when: WhenClause::ContextFlag("dialogOpen".to_string()),
            priority: BindingPriority::Default,
        });

        // Without context flag, should not match
        let result = resolver.process_key(KeyCombo::plain(Key::Escape), &interaction, &arena);
        assert_eq!(result, None);

        // Set the flag, should match
        resolver.set_context("dialogOpen", true);
        let result = resolver.process_key(KeyCombo::plain(Key::Escape), &interaction, &arena);
        assert_eq!(result.as_deref(), Some("dialog.close"));

        // Unset, should stop matching
        resolver.set_context("dialogOpen", false);
        let result = resolver.process_key(KeyCombo::plain(Key::Escape), &interaction, &arena);
        assert_eq!(result, None);
    }

    // -- Phase 3: FocusedTag / FocusedClass tests --

    use unshit_core::element::{Element, Tag};

    fn arena_with_button() -> (NodeArena, unshit_core::id::NodeId) {
        let mut arena = NodeArena::new();
        let mut elem = Element::new(Tag::Button);
        elem.classes.push("primary".to_string());
        elem.classes.push("submit".to_string());
        let id = arena.alloc(elem);
        (arena, id)
    }

    fn interaction_focused(id: unshit_core::id::NodeId) -> InteractionState {
        let mut i = InteractionState::default();
        i.focused = id;
        i
    }

    #[test]
    fn focused_tag_matches() {
        let (arena, btn_id) = arena_with_button();
        let interaction = interaction_focused(btn_id);
        let context = HashSet::new();

        let result =
            evaluate_when(&WhenClause::FocusedTag(Tag::Button), &context, &interaction, &arena);
        assert!(result);
    }

    #[test]
    fn focused_tag_misses() {
        let (arena, btn_id) = arena_with_button();
        let interaction = interaction_focused(btn_id);
        let context = HashSet::new();

        let result =
            evaluate_when(&WhenClause::FocusedTag(Tag::Input), &context, &interaction, &arena);
        assert!(!result);
    }

    #[test]
    fn focused_class_matches() {
        let (arena, btn_id) = arena_with_button();
        let interaction = interaction_focused(btn_id);
        let context = HashSet::new();

        let result = evaluate_when(
            &WhenClause::FocusedClass("primary".to_string()),
            &context,
            &interaction,
            &arena,
        );
        assert!(result);
    }

    #[test]
    fn focused_class_misses() {
        let (arena, btn_id) = arena_with_button();
        let interaction = interaction_focused(btn_id);
        let context = HashSet::new();

        let result = evaluate_when(
            &WhenClause::FocusedClass("secondary".to_string()),
            &context,
            &interaction,
            &arena,
        );
        assert!(!result);
    }

    #[test]
    fn focused_tag_dangling_returns_false() {
        let arena = dummy_arena();
        let interaction = dummy_interaction(); // focused = DANGLING
        let context = HashSet::new();

        let result =
            evaluate_when(&WhenClause::FocusedTag(Tag::Button), &context, &interaction, &arena);
        assert!(!result);
    }

    #[test]
    fn not_focused_tag() {
        let mut arena = NodeArena::new();
        let div_elem = Element::new(Tag::Div);
        let div_id = arena.alloc(div_elem);
        let interaction = interaction_focused(div_id);
        let context = HashSet::new();

        // Not(FocusedTag(Button)) should be true when focused on a Div
        let result = evaluate_when(
            &WhenClause::Not(Box::new(WhenClause::FocusedTag(Tag::Button))),
            &context,
            &interaction,
            &arena,
        );
        assert!(result);
    }

    #[test]
    fn and_context_and_focused() {
        let mut arena = NodeArena::new();
        let input_elem = Element::new(Tag::Input);
        let input_id = arena.alloc(input_elem);
        let interaction = interaction_focused(input_id);
        let mut context = HashSet::new();
        context.insert("editing".to_string());

        let clause = WhenClause::And(vec![
            WhenClause::ContextFlag("editing".to_string()),
            WhenClause::FocusedTag(Tag::Input),
        ]);

        let result = evaluate_when(&clause, &context, &interaction, &arena);
        assert!(result);

        // Remove the context flag; And should fail
        context.clear();
        let result = evaluate_when(&clause, &context, &interaction, &arena);
        assert!(!result);
    }

    // -- Phase 4: JSON config loading tests --

    #[test]
    fn parse_key_combo_ctrl_s() {
        let combo = parse_key_combo("Ctrl+S").unwrap();
        assert_eq!(combo.key, Key::Char('s'));
        assert_eq!(combo.modifiers, Modifiers::CTRL);
    }

    #[test]
    fn parse_key_combo_plain_escape() {
        let combo = parse_key_combo("Escape").unwrap();
        assert_eq!(combo.key, Key::Escape);
        assert!(combo.modifiers.is_empty());
    }

    #[test]
    fn parse_key_combo_f5() {
        let combo = parse_key_combo("F5").unwrap();
        assert_eq!(combo.key, Key::F(5));
    }

    #[test]
    fn parse_key_combo_ctrl_alt_delete() {
        let combo = parse_key_combo("Ctrl+Alt+Delete").unwrap();
        assert_eq!(combo.key, Key::Delete);
        assert!(combo.modifiers.contains(Modifiers::CTRL));
        assert!(combo.modifiers.contains(Modifiers::ALT));
    }

    #[test]
    fn parse_key_combo_empty_is_err() {
        assert!(parse_key_combo("").is_err());
    }

    #[test]
    fn parse_shortcut_single() {
        let s = parse_shortcut("Ctrl+S").unwrap();
        assert!(matches!(s, Shortcut::Single(_)));
    }

    #[test]
    fn parse_shortcut_chord() {
        let s = parse_shortcut("Ctrl+K, Ctrl+C").unwrap();
        assert!(matches!(s, Shortcut::Chord(_, _)));
    }

    #[test]
    fn parse_when_always() {
        assert!(matches!(parse_when("always").unwrap(), WhenClause::Always));
    }

    #[test]
    fn parse_when_context_flag() {
        let w = parse_when("dialogOpen").unwrap();
        assert!(matches!(w, WhenClause::ContextFlag(ref s) if s == "dialogOpen"));
    }

    #[test]
    fn parse_when_negation() {
        let w = parse_when("!dialogOpen").unwrap();
        assert!(matches!(w, WhenClause::Not(_)));
    }

    #[test]
    fn parse_when_focused_tag() {
        let w = parse_when("focusedTag:button").unwrap();
        assert!(matches!(w, WhenClause::FocusedTag(Tag::Button)));
    }

    #[test]
    fn parse_when_focused_class() {
        let w = parse_when("focusedClass:primary").unwrap();
        assert!(matches!(w, WhenClause::FocusedClass(ref s) if s == "primary"));
    }

    #[test]
    fn load_keybindings_from_json_basic() {
        let json = r#"{ "bindings": [
            { "key": "Ctrl+S", "command": "file.save" },
            { "key": "Escape", "command": "close" }
        ]}"#;
        let bindings = load_keybindings_from_json(json).unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].command, "file.save");
        assert_eq!(bindings[1].command, "close");
        assert!(matches!(bindings[0].priority, BindingPriority::User));
    }

    #[test]
    fn load_keybindings_chord() {
        let json = r#"{ "bindings": [
            { "key": "Ctrl+K, Ctrl+C", "command": "comment" }
        ]}"#;
        let bindings = load_keybindings_from_json(json).unwrap();
        assert!(matches!(bindings[0].shortcut, Shortcut::Chord(_, _)));
    }

    #[test]
    fn load_keybindings_with_when() {
        let json = r#"{ "bindings": [
            { "key": "Escape", "command": "close", "when": "dialogOpen" }
        ]}"#;
        let bindings = load_keybindings_from_json(json).unwrap();
        assert!(matches!(bindings[0].when, WhenClause::ContextFlag(ref s) if s == "dialogOpen"));
    }

    #[test]
    fn load_keybindings_invalid_json() {
        assert!(load_keybindings_from_json("not json").is_err());
    }

    #[test]
    fn load_keybindings_invalid_key() {
        let json = r#"{ "bindings": [
            { "key": "", "command": "test" }
        ]}"#;
        assert!(load_keybindings_from_json(json).is_err());
    }
}
