//! JSON persistence for user-customized keybindings.
//!
//! The on-disk format is a flat `{ "<action_id>": "<combo>" }` map. The
//! action id matches `KeybindAction::id` and the combo is whatever
//! `KeyCombo::parse` accepts. Unknown action ids and malformed combos
//! are logged and skipped: corruption never escalates to "app fails to
//! boot." A missing file is treated as "no overrides" and is not an
//! error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use unshit::core::shortcut::KeyCombo;

use super::KeybindAction;

pub type UserKeybinds = HashMap<KeybindAction, KeyCombo>;

/// Path installed at app startup. Dispatch-initiated saves read this
/// via `save_if_installed`. Tests that don't install skip file I/O.
static KEYBINDS_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Record the user-config path for dispatch-initiated saves. Safe to
/// call once at app startup. Subsequent calls are ignored.
pub fn install(path: PathBuf) {
    let _ = KEYBINDS_PATH.set(path);
}

fn configured_path() -> Option<&'static Path> {
    KEYBINDS_PATH.get().map(|p| p.as_path())
}

/// Load overrides from the installed path. Returns an empty map if no
/// path has been installed or the file is missing/malformed.
pub fn load_if_installed() -> UserKeybinds {
    configured_path()
        .map(load_user_keybinds)
        .unwrap_or_default()
}

/// Save overrides to the installed path. Silently skipped when no path
/// is installed (e.g. in unit tests). Errors are logged, not returned:
/// a failed write must never block a dispatch.
pub fn save_if_installed(map: &UserKeybinds) {
    let Some(path) = configured_path() else {
        return;
    };
    if let Err(e) = save_user_keybinds(path, map) {
        log::warn!("failed to save keybinds to {}: {}", path.display(), e);
    }
}

/// Default location of the user keybindings file. `None` if we cannot
/// determine a config dir (rare; happens on stripped-down systems).
pub fn keybinds_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("com.godly.terminal").join("keybindings.json"))
}

/// Read user overrides from disk. A missing file yields an empty map.
/// A malformed file yields an empty map and a logged warning; the caller
/// should fall back to defaults in both cases.
pub fn load_user_keybinds(path: &Path) -> UserKeybinds {
    if !path.exists() {
        return UserKeybinds::new();
    }
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("failed to read keybindings from {}: {}", path.display(), e);
            return UserKeybinds::new();
        }
    };
    let raw: HashMap<String, String> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("failed to parse keybindings from {}: {}", path.display(), e);
            return UserKeybinds::new();
        }
    };
    let mut out = UserKeybinds::new();
    for (id, combo_str) in raw {
        let Some(action) = KeybindAction::from_id(&id) else {
            log::warn!("unknown keybind action id '{}' ignored", id);
            continue;
        };
        match KeyCombo::parse(&combo_str) {
            Ok(combo) => {
                out.insert(action, combo);
            }
            Err(e) => log::warn!(
                "bad combo '{}' for action '{}' ignored: {}",
                combo_str,
                id,
                e
            ),
        }
    }
    out
}

/// Write user overrides atomically: serialize to `<path>.tmp` then rename
/// over the target. Leaves no `.tmp` on a successful write. On Windows
/// `std::fs::rename` already uses `MOVEFILE_REPLACE_EXISTING` so the
/// replace-existing case is covered.
pub fn save_user_keybinds(path: &Path, map: &UserKeybinds) -> std::io::Result<()> {
    let raw: HashMap<String, String> = map
        .iter()
        .map(|(action, combo)| (action.id().to_string(), combo.to_string()))
        .collect();
    let body = serde_json::to_string_pretty(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use unshit::core::event::{Key, Modifiers};

    fn unique_temp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir()
            .join(format!("godly-keybinds-{}-{}-{}", tag, pid, n))
            .join("keybindings.json")
    }

    fn combo(s: &str) -> KeyCombo {
        KeyCombo::parse(s).unwrap()
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let path = unique_temp_path("missing");
        assert!(!path.exists());
        let loaded = load_user_keybinds(&path);
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_malformed_json_returns_empty() {
        let path = unique_temp_path("malformed");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not valid json").unwrap();
        let loaded = load_user_keybinds(&path);
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_unknown_action_is_ignored() {
        let path = unique_temp_path("unknown");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{ "new_terminal": "Ctrl+T", "bogus_action": "Ctrl+B" }"#,
        )
        .unwrap();
        let loaded = load_user_keybinds(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.get(&KeybindAction::NewTerminal),
            Some(&combo("Ctrl+T"))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_invalid_combo_is_ignored() {
        let path = unique_temp_path("bad-combo");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{ "new_terminal": "Ctrl+T", "close_tab": "Not+a+real+combo" }"#,
        )
        .unwrap();
        let loaded = load_user_keybinds(&path);
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key(&KeybindAction::NewTerminal));
        assert!(!loaded.contains_key(&KeybindAction::CloseTab));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trip_preserves_overrides() {
        let path = unique_temp_path("roundtrip");
        let mut map = UserKeybinds::new();
        map.insert(KeybindAction::NewTerminal, combo("Ctrl+Shift+T"));
        map.insert(
            KeybindAction::Unsplit,
            KeyCombo::new(Key::Char('q'), Modifiers::CTRL | Modifiers::ALT),
        );
        save_user_keybinds(&path, &map).unwrap();

        let loaded = load_user_keybinds(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get(&KeybindAction::NewTerminal),
            Some(&combo("Ctrl+Shift+T"))
        );
        assert_eq!(
            loaded.get(&KeybindAction::Unsplit),
            map.get(&KeybindAction::Unsplit)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_creates_parent_dir() {
        let path = unique_temp_path("parent");
        assert!(!path.parent().unwrap().exists());
        let mut map = UserKeybinds::new();
        map.insert(KeybindAction::CloseTab, combo("Ctrl+F4"));
        save_user_keybinds(&path, &map).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_overwrites_existing() {
        let path = unique_temp_path("overwrite");
        let mut first = UserKeybinds::new();
        first.insert(KeybindAction::NewTerminal, combo("Ctrl+T"));
        save_user_keybinds(&path, &first).unwrap();

        let mut second = UserKeybinds::new();
        second.insert(KeybindAction::NewTerminal, combo("Ctrl+Shift+T"));
        save_user_keybinds(&path, &second).unwrap();

        let loaded = load_user_keybinds(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.get(&KeybindAction::NewTerminal),
            Some(&combo("Ctrl+Shift+T"))
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_leaves_no_tmp_file() {
        let path = unique_temp_path("no-tmp");
        let mut map = UserKeybinds::new();
        map.insert(KeybindAction::NewTerminal, combo("Ctrl+T"));
        save_user_keybinds(&path, &map).unwrap();

        let tmp = path.with_extension("tmp");
        assert!(!tmp.exists(), "leftover .tmp file at {}", tmp.display());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_and_load_empty_map() {
        let path = unique_temp_path("empty");
        let map = UserKeybinds::new();
        save_user_keybinds(&path, &map).unwrap();
        let loaded = load_user_keybinds(&path);
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keybinds_config_path_uses_app_namespace() {
        let path = keybinds_config_path().expect("config dir available");
        let s = path.to_string_lossy();
        assert!(
            s.contains("com.godly.terminal"),
            "expected app namespace in path, got {}",
            s
        );
        assert!(
            s.ends_with("keybindings.json"),
            "expected keybindings.json filename, got {}",
            s
        );
    }
}
