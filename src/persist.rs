use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::state::AppState;

static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedWorkspace {
    pub name: String,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub workspaces: Vec<PersistedWorkspace>,
    #[serde(default)]
    pub active_workspace: usize,
}

impl PersistedState {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            workspaces: state
                .workspaces
                .iter()
                .map(|w| PersistedWorkspace {
                    name: w.name.clone(),
                    path: w.path.clone(),
                    collapsed: w.collapsed,
                })
                .collect(),
            active_workspace: state.active_workspace,
        }
    }

    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, body)
    }

    pub fn read_from(path: &Path) -> std::io::Result<Self> {
        let body = std::fs::read_to_string(path)?;
        serde_json::from_str(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Default location for the persisted workspaces file. Lives outside the repo
/// so it is not tracked by git.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("com.godly.terminal").join("workspaces.json"))
}

/// Install the config path used by `save_workspaces` / `load_workspaces`.
/// Main installs the real path at startup. Tests that exercise persistence
/// install a temp path. Tests that do not install get a no-op save/load.
pub fn install(path: PathBuf) {
    let _ = CONFIG_PATH.set(path);
}

fn configured_path() -> Option<&'static Path> {
    CONFIG_PATH.get().map(|p| p.as_path())
}

pub fn save_workspaces(state: &AppState) {
    let Some(path) = configured_path() else {
        return;
    };
    let persisted = PersistedState::from_state(state);
    if let Err(e) = persisted.write_to(path) {
        log::warn!("failed to save workspaces to {}: {}", path.display(), e);
    }
}

pub fn load_workspaces() -> Option<PersistedState> {
    let path = configured_path()?;
    if !path.exists() {
        return None;
    }
    match PersistedState::read_from(path) {
        Ok(p) => Some(p),
        Err(e) => {
            log::warn!("failed to load workspaces from {}: {}", path.display(), e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir()
            .join(format!("godly-persist-{}-{}-{}", tag, pid, n))
            .join("workspaces.json")
    }

    #[test]
    fn round_trip_preserves_workspaces() {
        let state = seed_state();
        let persisted = PersistedState::from_state(&state);
        let path = unique_temp_path("round-trip");
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();
        assert_eq!(loaded.workspaces.len(), state.workspaces.len());
        for (loaded, original) in loaded.workspaces.iter().zip(state.workspaces.iter()) {
            assert_eq!(loaded.name, original.name);
            assert_eq!(loaded.path, original.path);
            assert_eq!(loaded.collapsed, original.collapsed);
        }
        assert_eq!(loaded.active_workspace, state.active_workspace);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_creates_parent_dir() {
        let path = unique_temp_path("parent");
        let persisted = PersistedState {
            workspaces: vec![PersistedWorkspace {
                name: "alpha".into(),
                path: Some(PathBuf::from("/tmp/alpha")),
                collapsed: true,
            }],
            active_workspace: 0,
        };
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();
        assert_eq!(loaded.workspaces[0].name, "alpha");
        assert_eq!(loaded.workspaces[0].path, Some(PathBuf::from("/tmp/alpha")));
        assert!(loaded.workspaces[0].collapsed);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_from_missing_file_errors() {
        let path = unique_temp_path("missing");
        assert!(PersistedState::read_from(&path).is_err());
    }
}
