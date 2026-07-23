use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// True when `title` was set by the user via the rename dialog.
    /// Restored into `AppState::custom_titled_panes` so guest-program
    /// titles (OSC 0/2) keep deferring to the manual name after a
    /// relaunch. Defaults to false for configs predating the field.
    #[serde(default)]
    pub custom_title: bool,
    /// Optional Claude/Codex restart identity. Legacy files default to
    /// no record, preserving the existing fresh-shell behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_restart: Option<crate::agent_restore::AgentRestart>,
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
    /// Stable daemon routing identity. Legacy files omit it and receive
    /// a collision-free id during restore.
    #[serde(default)]
    pub num: u32,
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
    /// Worktree-tabs mode: when true, new tabs open on a fresh git
    /// worktree of their workspace's repo. Defaults to false so
    /// upgraders keep plain new-tab behavior.
    #[serde(default)]
    pub worktree_tabs: bool,
    /// Explicit opt-in for automatic agent conversation restoration.
    /// False for legacy files and first runs.
    #[serde(default)]
    pub auto_resume_agents: bool,
    /// App wide default shell. Empty for upgraders predating the
    /// feature so the daemon's own `default_shell()` keeps the floor;
    /// inference only runs in `seed_state` for true first runs.
    #[serde(default)]
    pub default_shell: ShellSpec,
}

/// Capture a single tab's pane grid into its persisted form.
fn persisted_tab(
    tab: &TerminalTab,
    custom_titled: &HashSet<u32>,
    agent_restarts: &std::collections::HashMap<u32, crate::agent_restore::AgentRestart>,
) -> PersistedTab {
    let managed_agent = tab
        .panes
        .iter()
        .flatten()
        .filter_map(|pane| agent_restarts.get(&pane.id.0))
        .find(|restart| restart.managed)
        .map(|restart| restart.agent);
    PersistedTab {
        id: tab.id.clone(),
        name: managed_agent
            .map(|agent| format!("qp: {}", agent.label()))
            .unwrap_or_else(|| tab.name.clone()),
        subtitle: tab.subtitle.clone(),
        panes: tab
            .panes
            .iter()
            .map(|row| {
                row.iter()
                    .map(|p| persisted_pane(p, custom_titled, agent_restarts))
                    .collect()
            })
            .collect(),
        active_pane: tab.active_pane.0,
        row_ratios: tab.row_ratios.clone(),
        col_ratios: tab.col_ratios.clone(),
    }
}

fn persisted_pane(
    pane: &Pane,
    custom_titled: &HashSet<u32>,
    agent_restarts: &std::collections::HashMap<u32, crate::agent_restore::AgentRestart>,
) -> PersistedPane {
    let agent_restart = agent_restarts.get(&pane.id.0).cloned();
    let managed_agent = agent_restart
        .as_ref()
        .filter(|restart| restart.managed)
        .map(|restart| restart.agent);
    PersistedPane {
        id: pane.id.0,
        title: managed_agent
            .map(|agent| format!("qp: {}", agent.label()))
            .unwrap_or_else(|| pane.title.clone()),
        subtitle: pane.subtitle.clone(),
        custom_title: managed_agent.is_none() && custom_titled.contains(&pane.id.0),
        agent_restart,
    }
}

/// Remove editor panes from a persisted tab. Editor panes are not
/// restored across restarts (SPEC: no editor-session persistence), so
/// persisting them would respawn them as terminal panes on load. Ratios
/// are absorbed into a neighbor exactly like a live pane close. Returns
/// `false` when the tab has no panes left and should be dropped.
fn strip_editor_panes(tab: &mut PersistedTab, editor_ids: &HashSet<u32>) -> bool {
    let mut row = 0;
    while row < tab.panes.len() {
        let mut col = 0;
        while col < tab.panes[row].len() {
            if editor_ids.contains(&tab.panes[row][col].id) {
                tab.panes[row].remove(col);
                if let Some(ratios) = tab.col_ratios.get_mut(row) {
                    if col < ratios.len() {
                        let closed = ratios.remove(col);
                        if !ratios.is_empty() {
                            let absorb = if col > 0 { col - 1 } else { 0 };
                            ratios[absorb] += closed;
                        }
                    }
                }
            } else {
                col += 1;
            }
        }
        if tab.panes[row].is_empty() {
            tab.panes.remove(row);
            if row < tab.col_ratios.len() {
                tab.col_ratios.remove(row);
            }
            if row < tab.row_ratios.len() {
                let closed = tab.row_ratios.remove(row);
                if !tab.row_ratios.is_empty() {
                    let absorb = if row > 0 { row - 1 } else { 0 };
                    tab.row_ratios[absorb] += closed;
                }
            }
        } else {
            row += 1;
        }
    }
    if tab.panes.is_empty() {
        return false;
    }
    if editor_ids.contains(&tab.active_pane) {
        tab.active_pane = tab.panes[0][0].id;
    }
    true
}

/// Drop editor panes (and tabs that only contained editors) from a
/// workspace's persisted tabs, remapping the active tab index.
fn strip_editor_tabs(
    tabs: Vec<PersistedTab>,
    active_tab: usize,
    editor_ids: &HashSet<u32>,
) -> (Vec<PersistedTab>, usize) {
    if editor_ids.is_empty() {
        return (tabs, active_tab);
    }
    let mut kept = Vec::with_capacity(tabs.len());
    let mut new_active = 0usize;
    for (idx, mut tab) in tabs.into_iter().enumerate() {
        if strip_editor_panes(&mut tab, editor_ids) {
            if idx <= active_tab {
                new_active = kept.len();
            }
            kept.push(tab);
        }
    }
    (kept, new_active)
}

/// Effective tabs for a workspace, accounting for the live/stored split:
/// the active workspace keeps its tabs in `state.tabs`, and the active
/// tab's panes/ratios live in the top-level `state.panes` fields rather
/// than in `state.tabs[active_tab]`. Inactive workspaces hold everything
/// in `workspaces[i].tabs`.
fn workspace_tabs(state: &AppState, ws_idx: usize) -> (Vec<PersistedTab>, usize) {
    let custom_titled = &state.custom_titled_panes;
    let agent_restarts = &state.agent_restarts;
    let editor_ids: HashSet<u32> = state.editors.keys().copied().collect();
    if ws_idx == state.active_workspace {
        let tabs = state
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let mut pt = persisted_tab(tab, custom_titled, agent_restarts);
                if i == state.active_tab {
                    // Overlay the live (authoritative) pane grid for the
                    // active tab; `state.tabs[active_tab]` is only synced
                    // on tab switch and may be stale.
                    pt.panes = state
                        .panes
                        .iter()
                        .map(|row| {
                            row.iter()
                                .map(|p| persisted_pane(p, custom_titled, agent_restarts))
                                .collect()
                        })
                        .collect();
                    pt.active_pane = state.active_pane.0;
                    pt.row_ratios = state.row_ratios.clone();
                    pt.col_ratios = state.col_ratios.clone();
                }
                pt
            })
            .collect();
        strip_editor_tabs(tabs, state.active_tab, &editor_ids)
    } else {
        let ws = &state.workspaces[ws_idx];
        strip_editor_tabs(
            ws.tabs
                .iter()
                .map(|t| persisted_tab(t, custom_titled, agent_restarts))
                .collect(),
            ws.active_tab,
            &editor_ids,
        )
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
                        num: w.num,
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
            worktree_tabs: state
                .toggles
                .get(&crate::state::ToggleKey::WorktreeTabs)
                .copied()
                .unwrap_or(false),
            auto_resume_agents: state
                .toggles
                .get(&crate::state::ToggleKey::AutoResumeAgents)
                .copied()
                .unwrap_or(false),
            default_shell: state.default_shell.clone(),
        }
    }

    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        atomic_write_private(path, body.as_bytes())
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

/// Write a complete file by syncing a same-directory temporary and
/// atomically replacing the destination. Keeping the temporary beside
/// the target is required: atomic rename guarantees do not cross file
/// systems, and a reboot must leave either the old or the new JSON.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_with_permissions(path, bytes, AtomicWritePermissions::Preserve)
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    atomic_write_with_permissions(path, bytes, AtomicWritePermissions::OwnerOnly)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AtomicWritePermissions {
    Preserve,
    OwnerOnly,
}

fn atomic_write_with_permissions(
    path: &Path,
    bytes: &[u8],
    permission_policy: AtomicWritePermissions,
) -> std::io::Result<()> {
    #[cfg(not(unix))]
    let _ = permission_policy;
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let existing_permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());

    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mode = if permission_policy == AtomicWritePermissions::OwnerOnly {
                0o600
            } else {
                existing_permissions
                    .as_ref()
                    .map(PermissionsExt::mode)
                    .unwrap_or(0o600)
            };
            options.mode(mode);
        }
        let mut temp = options.open(&temp_path)?;
        temp.write_all(bytes)?;
        temp.flush()?;
        temp.sync_all()?;
        drop(temp);
        #[cfg(unix)]
        if permission_policy == AtomicWritePermissions::OwnerOnly {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))?;
        } else if let Some(permissions) = existing_permissions.clone() {
            std::fs::set_permissions(&temp_path, permissions)?;
        }
        #[cfg(not(unix))]
        if let Some(permissions) = existing_permissions.clone() {
            std::fs::set_permissions(&temp_path, permissions)?;
        }
        atomic_replace(&temp_path, path)?;
        sync_parent_dir(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // ReplaceFileW preserves the destination's ACL and other security
    // metadata. It requires an existing destination; first writes use
    // MoveFileExW and inherit the private profile directory ACL.
    let moved = if destination.exists() {
        // SAFETY: both paths are NUL-terminated and the optional pointers
        // are intentionally null. The buffers live for the full call.
        unsafe {
            ReplaceFileW(
                destination_wide.as_ptr(),
                source_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        }
    } else {
        // SAFETY: both pointers refer to NUL-terminated buffers that remain
        // alive for the call. Flags request same-volume atomic replacement.
        unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    // Windows directory handles require backup-semantics flags. The
    // MOVEFILE_WRITE_THROUGH call above supplies the durable rename.
    Ok(())
}

#[cfg(not(windows))]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}

/// Default location for the persisted workspaces file. Lives outside the repo
/// so it is not tracked by git, and inside the instance-profile config dir so
/// dev/test instances never overwrite the installed app's layout.
pub fn default_config_path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|d| d.join("workspaces.json"))
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

pub fn save_workspaces(state: &AppState) -> bool {
    let Some(path) = configured_path() else {
        return true;
    };
    let persisted = PersistedState::from_state(state);
    if let Err(e) = persisted.write_to(path) {
        let error_kind = match e.kind() {
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            std::io::ErrorKind::WriteZero => "write_zero",
            _ => "io_error",
        };
        log::warn!(
            "{{\"event\":\"workspace_persist.failed\",\"level\":\"warn\",\"error_kind\":{error_kind:?}}}"
        );
        false
    } else {
        true
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
    fn editor_tabs_are_not_persisted() {
        let mut state = seed_state();
        let file =
            std::env::temp_dir().join(format!("tm-persist-editor-{}.txt", std::process::id()));
        std::fs::write(&file, "hello\nworld").unwrap();
        assert!(crate::state::dispatch(
            &mut state,
            &format!("editor.open:{}", file.display())
        ));
        assert_eq!(state.tabs.len(), 2);
        let editor_tab_count = state.tabs.len();

        let persisted = PersistedState::from_state(&state);
        let ws = &persisted.workspaces[state.active_workspace];
        // The editor tab is stripped; only the terminal tab persists.
        assert_eq!(ws.tabs.len(), editor_tab_count - 1);
        let persisted_pane_ids: Vec<u32> = ws
            .tabs
            .iter()
            .flat_map(|t| t.panes.iter().flatten().map(|p| p.id))
            .collect();
        for editor_id in state.editors.keys() {
            assert!(
                !persisted_pane_ids.contains(editor_id),
                "editor pane {editor_id} leaked into persistence"
            );
        }
        // Active tab index remapped into bounds.
        assert!(ws.active_tab < ws.tabs.len());
        let _ = std::fs::remove_file(&file);
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
                num: 1,
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
            worktree_tabs: false,
            auto_resume_agents: false,
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
    fn round_trip_preserves_worktree_tabs_toggle() {
        let mut state = seed_state();
        state
            .toggles
            .insert(crate::state::ToggleKey::WorktreeTabs, true);
        let persisted = PersistedState::from_state(&state);
        let path = unique_temp_path("worktree-tabs-round-trip");
        persisted.write_to(&path).unwrap();
        let loaded = PersistedState::read_from(&path).unwrap();
        assert!(loaded.worktree_tabs);

        // A config predating the field must default to off.
        let json = r#"{
            "workspaces": [{"name":"alpha","path":null,"collapsed":false}],
            "active_workspace": 0
        }"#;
        let legacy: PersistedState = serde_json::from_str(json).unwrap();
        assert!(!legacy.worktree_tabs);
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
        assert!(!loaded.auto_resume_agents);
    }

    #[test]
    fn agent_restart_and_auto_resume_round_trip_without_content() {
        let mut state = seed_state();
        const SECRET_PROMPT_PREFIX: &str = "SENTINEL_DO_NOT_PERSIST_7f5a";
        state.workspaces[0].num = 42;
        let cwd = unique_temp_path("agent-cwd")
            .parent()
            .expect("temp parent")
            .to_path_buf();
        state.agent_restarts.insert(
            1,
            crate::agent_restore::AgentRestart {
                agent: crate::agent_restore::AgentKind::Codex,
                cwd: cwd.clone(),
                resume_mode: crate::agent_restore::AgentResumeMode::CodexExec,
                session_id: Some("24c31fc8-8200-4773-8a0b-0447bd64bcdc".into()),
                observed_at_unix_ms: 42,
                managed: true,
                launch_phase: crate::agent_restore::AgentLaunchPhase::PendingManual,
            },
        );
        state
            .toggles
            .insert(crate::state::ToggleKey::AutoResumeAgents, true);
        state.panes[0][0].title = SECRET_PROMPT_PREFIX.to_string();
        state.tabs[0].name = SECRET_PROMPT_PREFIX.to_string();

        let persisted = PersistedState::from_state(&state);
        let body = serde_json::to_string(&persisted).expect("serialize");
        assert!(body.contains("24c31fc8-8200-4773-8a0b-0447bd64bcdc"));
        assert!(!body.contains("prompt"));
        assert!(!body.contains("transcript"));
        assert!(!body.contains(SECRET_PROMPT_PREFIX));
        assert!(body.contains("qp: Codex"));

        let restored: PersistedState = serde_json::from_str(&body).expect("deserialize");
        let record = restored.workspaces[0].tabs[0].panes[0][0]
            .agent_restart
            .as_ref()
            .expect("restart record");
        assert_eq!(record.agent, crate::agent_restore::AgentKind::Codex);
        assert_eq!(record.cwd, cwd);
        assert_eq!(
            record.resume_mode,
            crate::agent_restore::AgentResumeMode::CodexExec
        );
        assert!(record.managed);
        assert_eq!(
            record.launch_phase,
            crate::agent_restore::AgentLaunchPhase::PendingManual
        );
        assert_eq!(restored.workspaces[0].num, 42);
        assert!(restored.auto_resume_agents);
    }

    #[test]
    fn legacy_pane_defaults_to_no_agent_restart() {
        let pane: PersistedPane = serde_json::from_str(
            r#"{"id":7,"title":"shell","subtitle":"bash","custom_title":false}"#,
        )
        .expect("legacy pane");
        assert!(pane.agent_restart.is_none());
    }

    #[test]
    fn legacy_agent_restart_defaults_to_confirmed_launch_phase() {
        let record: crate::agent_restore::AgentRestart = serde_json::from_str(
            r#"{"agent":"claude","cwd":"C:\\dev","session_id":"24c31fc8-8200-4773-8a0b-0447bd64bcdc","managed":true}"#,
        )
        .expect("legacy restart record");
        assert_eq!(
            record.launch_phase,
            crate::agent_restore::AgentLaunchPhase::Confirmed
        );
    }

    #[test]
    fn atomic_write_replaces_existing_json_without_temp_artifacts() {
        let path = unique_temp_path("atomic-replace");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, b"old body").expect("seed target");

        atomic_write(&path, b"new body").expect("atomic replace");

        assert_eq!(std::fs::read(&path).expect("read target"), b"new body");
        let parent = path.parent().expect("parent");
        let leftovers: Vec<_> = std::fs::read_dir(parent)
            .expect("read parent")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
        let _ = std::fs::remove_dir_all(parent);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_write_tightens_legacy_world_readable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let path = unique_temp_path("private-migration");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, b"legacy").expect("legacy file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("legacy mode");

        PersistedState::default()
            .write_to(&path)
            .expect("private workspace write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }
}
