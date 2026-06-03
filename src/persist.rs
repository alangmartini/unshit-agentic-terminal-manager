use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::shell::ShellSpec;
use crate::state::{AppState, Pane, TerminalTab};

static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// One pane within a persisted tab. Only the durable identity and label
/// are stored; runtime fields (`pid`, `cpu`) are recomputed on restore.
/// The `id` is load-bearing: the daemon keys surviving sessions by
/// `(workspace_id, pane_id)`, so restoring the same pane id is what lets
/// a relaunch reattach the shell instead of spawning a fresh one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedPane {
    pub id: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
}

/// A persisted terminal tab: its pane grid plus the split ratios needed
/// to redraw the same layout. `panes` is row-major to mirror
/// `TerminalTab::panes`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedTab {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub subtitle: String,
    pub panes: Vec<Vec<PersistedPane>>,
    #[serde(default)]
    pub active_pane: u32,
    #[serde(default)]
    pub row_ratios: Vec<f32>,
    #[serde(default)]
    pub col_ratios: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedWorkspace {
    pub name: String,
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub collapsed: bool,
    /// Per workspace shell override. Empty for upgraders predating
    /// the feature so they fall back to `default_shell` and then to
    /// the daemon's own `default_shell()` fallback.
    #[serde(default)]
    pub shell: ShellSpec,
    /// Terminal tabs (with their pane layout) open in this workspace.
    /// Empty for upgraders predating layout persistence; such configs
    /// fall back to a fresh default terminal on the next launch.
    #[serde(default)]
    pub tabs: Vec<PersistedTab>,
    /// Index of the active tab within `tabs`.
    #[serde(default)]
    pub active_tab: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub workspaces: Vec<PersistedWorkspace>,
    #[serde(default)]
    pub active_workspace: usize,
    /// F7: the user's remembered close-app choice. Both default to false
    /// (prompt on every close) so upgrading an existing config without
    /// these keys restores the prompt rather than silently applying a
    /// destructive action.
    #[serde(default)]
    pub remember_close_choice: bool,
    #[serde(default)]
    pub kill_all_on_close: bool,
    /// App wide default shell. Empty for upgraders predating the
    /// feature so the daemon's own `default_shell()` keeps the floor;
    /// inference only runs in `seed_state` for true first runs.
    #[serde(default)]
    pub default_shell: ShellSpec,
}

/// Capture a single tab's pane grid into its persisted form.
fn persisted_tab(tab: &TerminalTab) -> PersistedTab {
    PersistedTab {
        id: tab.id.clone(),
        name: tab.name.clone(),
        subtitle: tab.subtitle.clone(),
        panes: tab
            .panes
            .iter()
            .map(|row| row.iter().map(persisted_pane).collect())
            .collect(),
        active_pane: tab.active_pane.0,
        row_ratios: tab.row_ratios.clone(),
        col_ratios: tab.col_ratios.clone(),
    }
}

fn persisted_pane(pane: &Pane) -> PersistedPane {
    PersistedPane {
        id: pane.id.0,
        title: pane.title.clone(),
        subtitle: pane.subtitle.clone(),
    }
}

/// Effective tabs for a workspace, accounting for the live/stored split:
/// the active workspace keeps its tabs in `state.tabs`, and the active
/// tab's panes/ratios live in the top-level `state.panes` fields rather
/// than in `state.tabs[active_tab]`. Inactive workspaces hold everything
/// in `workspaces[i].tabs`.
fn workspace_tabs(state: &AppState, ws_idx: usize) -> (Vec<PersistedTab>, usize) {
    if ws_idx == state.active_workspace {
        let tabs = state
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let mut pt = persisted_tab(tab);
                if i == state.active_tab {
                    // Overlay the live (authoritative) pane grid for the
                    // active tab; `state.tabs[active_tab]` is only synced
                    // on tab switch and may be stale.
                    pt.panes = state
                        .panes
                        .iter()
                        .map(|row| row.iter().map(persisted_pane).collect())
                        .collect();
                    pt.active_pane = state.active_pane.0;
                    pt.row_ratios = state.row_ratios.clone();
                    pt.col_ratios = state.col_ratios.clone();
                }
                pt
            })
            .collect();
        (tabs, state.active_tab)
    } else {
        let ws = &state.workspaces[ws_idx];
        (ws.tabs.iter().map(persisted_tab).collect(), ws.active_tab)
    }
}

impl PersistedState {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            workspaces: state
                .workspaces
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let (tabs, active_tab) = workspace_tabs(state, i);
                    PersistedWorkspace {
                        name: w.name.clone(),
                        path: w.path.clone(),
                        collapsed: w.collapsed,
                        shell: w.shell.clone(),
                        tabs,
                        active_tab,
                    }
                })
                .collect(),
            active_workspace: state.active_workspace,
            remember_close_choice: state
                .toggles
                .get(&crate::state::ToggleKey::RememberCloseChoice)
                .copied()
                .unwrap_or(false),
            kill_all_on_close: state
                .toggles
                .get(&crate::state::ToggleKey::KillAllOnClose)
                .copied()
                .unwrap_or(false),
            default_shell: state.default_shell.clone(),
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

    /// True when at least one workspace carries a persisted tab/pane
    /// layout. Configs written before layout persistence (or by a
    /// kill-all-and-quit) have no tabs, in which case the caller falls
    /// back to seeding a fresh default terminal instead of restoring.
    pub fn has_layout(&self) -> bool {
        self.workspaces.iter().any(|w| !w.tabs.is_empty())
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
                shell: ShellSpec::default(),
                tabs: vec![],
                active_tab: 0,
            }],
            active_workspace: 0,
            remember_close_choice: false,
            kill_all_on_close: false,
            default_shell: ShellSpec::default(),
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

    #[test]
    fn from_state_captures_active_workspace_tabs() {
        // seed_state seeds the active workspace's tabs in the live fields,
        // not in `workspaces[0].tabs`. from_state must read the live fields.
        let state = seed_state();
        let persisted = PersistedState::from_state(&state);
        assert!(persisted.has_layout());
        let tabs = &persisted.workspaces[0].tabs;
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].panes.len(), 1);
        assert_eq!(tabs[0].panes[0].len(), 1);
        assert_eq!(tabs[0].panes[0][0].id, 1);
        assert_eq!(tabs[0].active_pane, 1);
    }

    #[test]
    fn has_layout_false_for_legacy_config_without_tabs() {
        let json = r#"{
            "workspaces": [{"name":"alpha","path":null,"collapsed":false}],
            "active_workspace": 0,
            "remember_close_choice": false,
            "kill_all_on_close": false
        }"#;
        let loaded: PersistedState = serde_json::from_str(json).unwrap();
        assert!(!loaded.has_layout());
    }

    #[test]
    fn round_trip_preserves_tab_and_pane_layout() {
        // Build a richer layout: a second tab containing a horizontal split.
        let mut state = seed_state();
        crate::state::mutate_add_tab(&mut state);
        let new_pane = state.active_pane;
        crate::state::mutate_split_right(&mut state, new_pane);

        let persisted = PersistedState::from_state(&state);
        let path = unique_temp_path("layout-round-trip");
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();

        let tabs = &loaded.workspaces[0].tabs;
        assert_eq!(tabs.len(), 2);
        assert_eq!(loaded.workspaces[0].active_tab, 1);
        // The split tab carries two panes in one row, with two col ratios.
        let split_tab = &tabs[1];
        assert_eq!(split_tab.panes.len(), 1);
        assert_eq!(split_tab.panes[0].len(), 2);
        assert_eq!(split_tab.col_ratios[0].len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trip_preserves_non_default_default_shell() {
        let mut state = seed_state();
        state.default_shell = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec!["--login".into()],
        };
        let persisted = PersistedState::from_state(&state);
        let path = unique_temp_path("default-shell-round-trip");
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();
        assert_eq!(loaded.default_shell.program, "/bin/bash");
        assert_eq!(loaded.default_shell.args, vec!["--login".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trip_preserves_per_workspace_shell() {
        let mut state = seed_state();
        state.workspaces[0].shell = crate::shell::ShellSpec {
            program: "/bin/fish".into(),
            args: vec!["-i".into()],
        };
        let persisted = PersistedState::from_state(&state);
        let path = unique_temp_path("ws-shell-round-trip");
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();
        assert_eq!(loaded.workspaces[0].shell.program, "/bin/fish");
        assert_eq!(loaded.workspaces[0].shell.args, vec!["-i".to_string()]);
        assert!(
            loaded.workspaces[1].shell.is_empty(),
            "workspaces without an override must round trip as empty"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persisted_workspace_deserializes_with_empty_shell_when_field_is_missing() {
        // A workspaces.json from before the per-workspace shell feature
        // omits the field. Serde must hydrate it as the empty spec so
        // upgraders keep falling back to the app default.
        let json = r#"{"name":"alpha","path":null,"collapsed":false}"#;
        let loaded: PersistedWorkspace = serde_json::from_str(json).unwrap();
        assert!(
            loaded.shell.is_empty(),
            "missing shell field must deserialize to an empty ShellSpec, got {:?}",
            loaded.shell
        );
    }

    #[test]
    fn deserializes_with_default_shell_when_field_is_missing() {
        // An old workspaces.json predating the default shell feature
        // omits the field entirely. Serde must hydrate it as the empty
        // spec so the daemon's own `default_shell()` continues to win
        // for upgraders. Inference only kicks in for true first runs.
        let json = r#"{
            "workspaces": [{"name":"alpha","path":null,"collapsed":false}],
            "active_workspace": 0,
            "remember_close_choice": false,
            "kill_all_on_close": false
        }"#;
        let loaded: PersistedState = serde_json::from_str(json).unwrap();
        assert!(
            loaded.default_shell.is_empty(),
            "missing default_shell must deserialize to an empty ShellSpec, got {:?}",
            loaded.default_shell
        );
    }
}
