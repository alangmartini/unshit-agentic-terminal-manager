//! Instance profiles: namespace every OS-shared resource (daemon pipe,
//! config dir, data dir, notification pipe) so multiple app instances
//! can run side by side without touching each other's sessions.
//!
//! Three tiers:
//! - **default** — the installed app. Unsuffixed pipe, config under
//!   `com.godly.terminal`. Selected when `TM_PROFILE` is unset and the
//!   executable does not live in a cargo `target` dir (or when
//!   `TM_PROFILE=default` forces it).
//! - **dev** — any repo build (`cargo run`, debug or release). Chosen
//!   automatically so dogfooding a work-in-progress build can never
//!   attach to the installed app's daemon or overwrite its config.
//! - **named/ephemeral** — `TM_PROFILE=<tag>` (tests, scripts) gives a
//!   fully separate namespace per tag. `TM_CONFIG_DIR` additionally
//!   redirects the config dir to an arbitrary (e.g. temp) path so test
//!   runs leave nothing behind in `%APPDATA%`.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const ENV_PROFILE: &str = "TM_PROFILE";
pub const ENV_CONFIG_DIR: &str = "TM_CONFIG_DIR";

const DEFAULT_NAMESPACE: &str = "com.godly.terminal";
const MAX_PROFILE_LEN: usize = 32;

/// The resolved profile for this process. `None` is the default
/// (installed-app) profile. Resolved once; the profile cannot change
/// mid-process because pipe names and config paths are already handed
/// out.
pub fn active_profile() -> Option<&'static str> {
    static PROFILE: OnceLock<Option<String>> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            resolve_profile(
                std::env::var_os(ENV_PROFILE),
                std::env::current_exe().ok().as_deref(),
                cfg!(debug_assertions),
            )
        })
        .as_deref()
}

fn resolve_profile(
    env_value: Option<std::ffi::OsString>,
    exe: Option<&Path>,
    debug_build: bool,
) -> Option<String> {
    if let Some(raw) = env_value {
        let raw = raw.to_string_lossy();
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
            return None;
        }
        let tag = sanitize_profile(trimmed);
        return (!tag.is_empty()).then_some(tag);
    }
    if debug_build || exe.is_some_and(exe_is_repo_build) {
        return Some("dev".to_string());
    }
    None
}

/// A repo build is any exe living under a cargo target dir (`target`,
/// `target-codex`, ...). Installed copies live under Program Files /
/// LocalAppData Programs, never inside a `target*` component.
fn exe_is_repo_build(exe: &Path) -> bool {
    exe.components().any(|c| match c {
        Component::Normal(name) => name.to_str().is_some_and(|s| {
            let s = s.to_ascii_lowercase();
            s == "target" || s.starts_with("target-")
        }),
        _ => false,
    })
}

/// Profiles come from user-controlled env, and feed both directory
/// names and pipe names: keep ascii alphanumerics, `-` and `_`
/// (lowercased), replace the rest, cap the length.
fn sanitize_profile(raw: &str) -> String {
    let tag: String = raw
        .chars()
        .take(MAX_PROFILE_LEN)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    tag.trim_matches(|c| c == '_' || c == '-').to_string()
}

fn namespace_dir_name(profile: Option<&str>) -> String {
    match profile {
        Some(tag) => format!("{DEFAULT_NAMESPACE}.{tag}"),
        None => DEFAULT_NAMESPACE.to_string(),
    }
}

/// Config directory for this instance (holds `workspaces.json`,
/// `quick_prompt.json`, `keybindings.json`). `TM_CONFIG_DIR` overrides
/// everything so tests can point at a throwaway temp dir; otherwise
/// the platform config dir namespaced by profile.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_CONFIG_DIR) {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::config_dir().map(|d| d.join(namespace_dir_name(active_profile())))
}

/// Data directory for this instance (Quick Prompt worktrees, etc.),
/// namespaced by profile the same way as [`config_dir`].
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(namespace_dir_name(active_profile())))
}

/// Marker appended to window titles so a dev/test instance is visually
/// distinguishable from the installed app in the taskbar and alt-tab.
pub fn title_suffix() -> String {
    match active_profile() {
        Some(tag) => format!(" [{tag}]"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_env_on_installed_release_exe_is_default_profile() {
        let exe = PathBuf::from(r"C:\Users\a\AppData\Local\Programs\Terminal Manager\terminal-manager.exe");
        assert_eq!(resolve_profile(None, Some(&exe), false), None);
    }

    #[test]
    fn unset_env_on_repo_release_build_is_dev() {
        let exe = PathBuf::from(r"C:\Users\a\dev\repo\target\release\terminal-manager.exe");
        assert_eq!(
            resolve_profile(None, Some(&exe), false),
            Some("dev".to_string())
        );
    }

    #[test]
    fn unset_env_on_alternate_target_dir_is_dev() {
        let exe = PathBuf::from(r"C:\Users\a\dev\repo\target-codex\debug\terminal-manager.exe");
        assert_eq!(
            resolve_profile(None, Some(&exe), false),
            Some("dev".to_string())
        );
    }

    #[test]
    fn debug_builds_are_dev_regardless_of_path() {
        let exe = PathBuf::from(r"C:\somewhere\terminal-manager.exe");
        assert_eq!(
            resolve_profile(None, Some(&exe), true),
            Some("dev".to_string())
        );
    }

    #[test]
    fn explicit_default_overrides_the_repo_build_heuristic() {
        let exe = PathBuf::from(r"C:\repo\target\debug\terminal-manager.exe");
        assert_eq!(
            resolve_profile(Some("default".into()), Some(&exe), true),
            None
        );
        assert_eq!(resolve_profile(Some("".into()), Some(&exe), true), None);
    }

    #[test]
    fn named_profiles_are_sanitized() {
        assert_eq!(
            resolve_profile(Some("Test Run/7".into()), None, false),
            Some("test_run_7".to_string())
        );
    }

    #[test]
    fn profile_that_sanitizes_to_empty_falls_back_to_default() {
        assert_eq!(resolve_profile(Some("///".into()), None, false), None);
    }

    #[test]
    fn namespace_dir_names() {
        assert_eq!(namespace_dir_name(None), "com.godly.terminal");
        assert_eq!(namespace_dir_name(Some("dev")), "com.godly.terminal.dev");
    }
}
