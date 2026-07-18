use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::AgentKind;

const MANAGED_STATUS: &str = "Terminal Manager agent recovery";
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEdit {
    Unchanged,
    Changed,
}

#[derive(Debug)]
pub struct HookInstallReport {
    pub claude: io::Result<HookEdit>,
    pub codex: io::Result<HookEdit>,
}

impl HookInstallReport {
    pub fn all_succeeded(&self) -> bool {
        self.claude.is_ok() && self.codex.is_ok()
    }

    pub fn changed_count(&self) -> usize {
        [&self.claude, &self.codex]
            .into_iter()
            .filter(|result| matches!(result, Ok(HookEdit::Changed)))
            .count()
    }
}

pub fn install_managed_hooks() -> HookInstallReport {
    manage_default_hooks(install_managed_hooks_at)
}

pub fn uninstall_managed_hooks() -> HookInstallReport {
    manage_default_hooks(uninstall_managed_hooks_at)
}

fn manage_default_hooks(
    operation: fn(&Path, &Path, &Path) -> HookInstallReport,
) -> HookInstallReport {
    let executable = std::env::current_exe();
    let home = dirs::home_dir();
    match (executable, home) {
        (Ok(executable), Some(home)) => operation(
            &executable,
            &home.join(".claude/settings.json"),
            &home.join(".codex/hooks.json"),
        ),
        (executable, home) => {
            let kind = executable
                .err()
                .map(|error| error.kind())
                .unwrap_or(io::ErrorKind::NotFound);
            let message = if home.is_none() {
                "user home directory is unavailable"
            } else {
                "current executable path is unavailable"
            };
            HookInstallReport {
                claude: Err(io::Error::new(kind, message)),
                codex: Err(io::Error::new(kind, message)),
            }
        }
    }
}

pub fn install_managed_hooks_at(
    executable: &Path,
    claude_settings: &Path,
    codex_hooks: &Path,
) -> HookInstallReport {
    HookInstallReport {
        claude: edit_hook_file(claude_settings, AgentKind::Claude, executable, false),
        codex: edit_hook_file(codex_hooks, AgentKind::Codex, executable, false),
    }
}

pub fn uninstall_managed_hooks_at(
    executable: &Path,
    claude_settings: &Path,
    codex_hooks: &Path,
) -> HookInstallReport {
    HookInstallReport {
        claude: edit_hook_file(claude_settings, AgentKind::Claude, executable, true),
        codex: edit_hook_file(codex_hooks, AgentKind::Codex, executable, true),
    }
}

fn edit_hook_file(
    path: &Path,
    agent: AgentKind,
    executable: &Path,
    remove: bool,
) -> io::Result<HookEdit> {
    let _edit_lock = HookEditLock::acquire(path)?;
    let original = read_hook_file_no_follow(path)?;
    if remove && original.is_none() {
        return Ok(HookEdit::Unchanged);
    }
    let mut document = match original.as_deref() {
        None => json!({}),
        Some(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => json!({}),
        Some(bytes) => serde_json::from_slice::<Value>(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
    };
    let changed = if remove {
        remove_owned_session_start_hook(&mut document, agent)?
    } else {
        merge_owned_session_start_hook(&mut document, agent, executable)?
    };
    if !changed {
        return Ok(HookEdit::Unchanged);
    }

    // Refuse to overwrite a user edit that landed after our read.
    let current = read_hook_file_no_follow(path)?;
    if current != original {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "hook settings changed concurrently",
        ));
    }
    let mut body = serde_json::to_vec_pretty(&document).map_err(io::Error::other)?;
    body.push(b'\n');
    crate::persist::atomic_write(path, &body)?;
    Ok(HookEdit::Changed)
}

fn read_hook_file_no_follow(path: &Path) -> io::Result<Option<Vec<u8>>> {
    reject_direct_link(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // Repeat after open to detect a link swapped in between the first check
    // and the no-follow open. The OS flag is the authoritative protection;
    // this produces a stable, user-facing error category.
    reject_direct_link(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn reject_direct_link(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut is_link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        is_link |= metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    if is_link {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "managed hook files and lock files must not be symlinks or reparse points",
        ))
    } else {
        Ok(())
    }
}

/// Cross-process edit lock backed by an OS advisory file lock.
///
/// The lock file intentionally survives release. The kernel owns the actual
/// lock and drops it when a process exits, including after a crash, so stale
/// PID metadata can never strand hook configuration permanently.
struct HookEditLock {
    file: File,
}

impl HookEditLock {
    fn acquire(target: &Path) -> io::Result<Self> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let lock_path = parent.join(".terminal-manager-agent-recovery.lock");
        reject_direct_link(&lock_path)?;

        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            // Allow other editors to open the lock file, but keep its inode-like
            // identity stable while any editor is using it.
            const FILE_SHARE_READ: u32 = 0x0000_0001;
            const FILE_SHARE_WRITE: u32 = 0x0000_0002;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
            options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let file = options.open(&lock_path)?;
        reject_direct_link(&lock_path)?;

        for attempt in 0..20 {
            match try_lock_file(&file) {
                Ok(()) => {
                    let mut lock = Self { file };
                    if let Err(error) = write_lock_owner(&mut lock.file) {
                        drop(lock);
                        return Err(error);
                    }
                    return Ok(lock);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && attempt < 19 => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "hook settings are being edited concurrently",
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded lock acquisition returns inside the loop")
    }
}

impl Drop for HookEditLock {
    fn drop(&mut self) {
        let _ = unlock_file(&self.file);
    }
}

fn write_lock_owner(file: &mut File) -> io::Result<()> {
    static NONCE: AtomicU64 = AtomicU64::new(0);

    let acquired_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = NONCE.fetch_add(1, Ordering::Relaxed);
    let nonce = format!(
        "{:x}-{:x}-{sequence:x}",
        std::process::id(),
        acquired_unix_ms
    );
    let owner = json!({
        "pid": std::process::id(),
        "nonce": nonce,
        "acquired_unix_ms": acquired_unix_ms,
    });

    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    serde_json::to_writer(&mut *file, &owner).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.flush()
}

fn try_lock_file(file: &File) -> io::Result<()> {
    Ok(file.try_lock()?)
}

fn unlock_file(file: &File) -> io::Result<()> {
    file.unlock()
}

fn merge_owned_session_start_hook(
    document: &mut Value,
    agent: AgentKind,
    executable: &Path,
) -> io::Result<bool> {
    let desired = desired_handler(agent, executable);
    let groups = session_start_groups_mut(document)?;

    let mut owned = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        let object = group.as_object().ok_or_else(wrong_hook_shape)?;
        let hooks = object
            .get("hooks")
            .and_then(Value::as_array)
            .ok_or_else(wrong_hook_shape)?;
        for (hook_index, hook) in hooks.iter().enumerate() {
            if is_owned_handler(hook, agent) {
                owned.push((group_index, hook_index));
            }
        }
    }
    if let [(group_index, hook_index)] = owned.as_slice() {
        let group = groups[*group_index]
            .as_object()
            .expect("validated group object");
        let hooks = group
            .get("hooks")
            .and_then(Value::as_array)
            .expect("validated hooks array");
        if group.get("matcher").and_then(Value::as_str) == Some(SESSION_START_MATCHER)
            && hooks[*hook_index] == desired
        {
            return Ok(false);
        }
    }

    remove_owned_from_groups(groups, agent)?;
    groups.push(json!({
        "matcher": SESSION_START_MATCHER,
        "hooks": [desired]
    }));
    Ok(true)
}

fn remove_owned_session_start_hook(document: &mut Value, agent: AgentKind) -> io::Result<bool> {
    let Some(root) = document.as_object_mut() else {
        return Err(wrong_hook_shape());
    };
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(false);
    };
    let Some(hooks) = hooks.as_object_mut() else {
        return Err(wrong_hook_shape());
    };
    let Some(groups) = hooks.get_mut("SessionStart") else {
        return Ok(false);
    };
    let groups = groups.as_array_mut().ok_or_else(wrong_hook_shape)?;
    remove_owned_from_groups(groups, agent)
}

fn remove_owned_from_groups(groups: &mut Vec<Value>, agent: AgentKind) -> io::Result<bool> {
    let mut changed = false;
    let mut rebuilt = Vec::with_capacity(groups.len());
    for mut group in groups.drain(..) {
        let object = group.as_object_mut().ok_or_else(wrong_hook_shape)?;
        let matcher_is_managed =
            object.get("matcher").and_then(Value::as_str) == Some(SESSION_START_MATCHER);
        let hooks = object
            .get_mut("hooks")
            .and_then(Value::as_array_mut)
            .ok_or_else(wrong_hook_shape)?;
        let before = hooks.len();
        hooks.retain(|hook| !is_owned_handler(hook, agent));
        let removed = hooks.len() != before;
        changed |= removed;
        let dedicated_managed_group = removed
            && hooks.is_empty()
            && matcher_is_managed
            && object.keys().all(|key| key == "matcher" || key == "hooks");
        if !dedicated_managed_group {
            rebuilt.push(group);
        }
    }
    *groups = rebuilt;
    Ok(changed)
}

fn session_start_groups_mut(document: &mut Value) -> io::Result<&mut Vec<Value>> {
    let root = document.as_object_mut().ok_or_else(wrong_hook_shape)?;
    if !root.contains_key("hooks") {
        root.insert("hooks".into(), json!({}));
    }
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or_else(wrong_hook_shape)?;
    if !hooks.contains_key("SessionStart") {
        hooks.insert("SessionStart".into(), json!([]));
    }
    hooks
        .get_mut("SessionStart")
        .and_then(Value::as_array_mut)
        .ok_or_else(wrong_hook_shape)
}

fn desired_handler(agent: AgentKind, executable: &Path) -> Value {
    let executable = executable.to_string_lossy();
    match agent {
        AgentKind::Claude => json!({
            "type": "command",
            "command": executable,
            "args": ["session-hook", "claude"],
            "timeout": 5,
            "statusMessage": MANAGED_STATUS
        }),
        AgentKind::Codex => json!({
            "type": "command",
            "command": format!("{} session-hook codex", posix_quote(&executable)),
            "commandWindows": format!("& {} session-hook codex", powershell_quote(&executable)),
            "timeout": 5,
            "statusMessage": MANAGED_STATUS
        }),
    }
}

fn is_owned_handler(value: &Value, agent: AgentKind) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("type").and_then(Value::as_str) != Some("command")
        || object.get("statusMessage").and_then(Value::as_str) != Some(MANAGED_STATUS)
    {
        return false;
    }
    match agent {
        AgentKind::Claude => object
            .get("args")
            .and_then(Value::as_array)
            .is_some_and(|args| {
                args == &vec![
                    Value::String("session-hook".into()),
                    Value::String("claude".into()),
                ]
            }),
        AgentKind::Codex => {
            let suffix = " session-hook codex";
            object
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command.ends_with(suffix))
                || object
                    .get("commandWindows")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.ends_with(suffix))
        }
    }
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn wrong_hook_shape() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "hook settings have an unexpected JSON shape",
    )
}

pub fn error_kind(result: &io::Result<HookEdit>) -> Option<&'static str> {
    result.as_ref().err().map(|error| match error.kind() {
        io::ErrorKind::InvalidData => "invalid_config",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::WouldBlock => "concurrent_edit",
        io::ErrorKind::NotFound => "path_unavailable",
        _ => "io_error",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tm-agent-hooks-{tag}-{}-{count}",
            std::process::id()
        ));
        (
            root.join("bin/terminal manager's.exe"),
            root.join("claude/settings.json"),
            root.join("codex/hooks.json"),
        )
    }

    #[test]
    fn install_is_idempotent_and_preserves_unrelated_settings() {
        let (executable, claude, codex) = fixture("install");
        std::fs::create_dir_all(claude.parent().expect("parent")).expect("parent");
        std::fs::write(
            &claude,
            br#"{"theme":"dark","hooks":{"Stop":[{"matcher":"","hooks":[]}]}}"#,
        )
        .expect("seed settings");

        let first = install_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(first.claude.expect("claude"), HookEdit::Changed);
        assert_eq!(first.codex.expect("codex"), HookEdit::Changed);
        let claude_once = std::fs::read(&claude).expect("claude body");
        let codex_once = std::fs::read(&codex).expect("codex body");
        let second = install_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(second.changed_count(), 0);
        assert_eq!(std::fs::read(&claude).expect("claude twice"), claude_once);
        assert_eq!(std::fs::read(&codex).expect("codex twice"), codex_once);
        let value: Value = serde_json::from_slice(&claude_once).expect("json");
        assert_eq!(value["theme"], "dark");
        assert!(value["hooks"]["Stop"].is_array());
        let _ = std::fs::remove_dir_all(executable.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn malformed_json_is_never_overwritten() {
        let (executable, claude, codex) = fixture("malformed");
        std::fs::create_dir_all(claude.parent().expect("parent")).expect("parent");
        std::fs::write(&claude, b"{broken").expect("seed malformed");
        let report = install_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(
            report.claude.expect_err("must reject").kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(std::fs::read(&claude).expect("unchanged"), b"{broken");
        let _ = std::fs::remove_dir_all(executable.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn codex_commands_quote_spaces_and_apostrophes_for_both_shells() {
        let (executable, _, _) = fixture("quote");
        let handler = desired_handler(AgentKind::Codex, &executable);
        let posix = handler["command"].as_str().expect("posix");
        let windows = handler["commandWindows"].as_str().expect("windows");
        assert!(posix.starts_with('\''));
        assert!(posix.contains("'\"'\"'"));
        assert!(windows.starts_with("& '"));
        assert!(windows.contains("manager''s.exe"));
    }

    #[test]
    fn uninstall_removes_only_owned_handlers() {
        let (executable, claude, codex) = fixture("remove");
        let installed = install_managed_hooks_at(&executable, &claude, &codex);
        assert!(installed.all_succeeded());
        let mut claude_value: Value =
            serde_json::from_slice(&std::fs::read(&claude).expect("claude")).expect("json");
        claude_value["hooks"]["SessionStart"]
            .as_array_mut()
            .expect("groups")
            .push(json!({"matcher":"startup","hooks":[{"type":"command","command":"mine"}]}));
        crate::persist::atomic_write(
            &claude,
            &serde_json::to_vec_pretty(&claude_value).expect("serialize"),
        )
        .expect("write user hook");

        let removed = uninstall_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(removed.claude.expect("remove claude"), HookEdit::Changed);
        assert_eq!(removed.codex.expect("remove codex"), HookEdit::Changed);
        let body = std::fs::read_to_string(&claude).expect("body");
        assert!(body.contains("mine"));
        assert!(!body.contains(MANAGED_STATUS));
        let _ = std::fs::remove_dir_all(executable.parent().unwrap().parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn managed_hook_edits_refuse_symlinked_configuration() {
        use std::os::unix::fs::symlink;

        let (executable, claude, codex) = fixture("config-symlink");
        let root = executable.parent().unwrap().parent().unwrap();
        std::fs::create_dir_all(claude.parent().expect("claude parent")).expect("parent");
        let sentinel = root.join("sentinel-settings.json");
        let original = br#"{"theme":"unchanged"}"#;
        std::fs::write(&sentinel, original).expect("sentinel");
        symlink(&sentinel, &claude).expect("settings symlink");

        let installed = install_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(
            installed
                .claude
                .expect_err("install must reject link")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        let removed = uninstall_managed_hooks_at(&executable, &claude, &codex);
        assert_eq!(
            removed.claude.expect_err("remove must reject link").kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(std::fs::read(&sentinel).expect("sentinel body"), original);
        assert!(std::fs::symlink_metadata(&claude)
            .expect("link metadata")
            .file_type()
            .is_symlink());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn hook_edit_lock_refuses_symlinked_lock_file() {
        use std::os::unix::fs::symlink;

        let (_, target, _) = fixture("lock-symlink");
        let parent = target.parent().expect("target parent");
        std::fs::create_dir_all(parent).expect("parent");
        let sentinel = parent.join("sentinel-lock");
        std::fs::write(&sentinel, b"unchanged").expect("sentinel");
        let lock_path = parent.join(".terminal-manager-agent-recovery.lock");
        symlink(&sentinel, &lock_path).expect("lock symlink");

        assert_eq!(
            HookEditLock::acquire(&target)
                .expect_err("lock link must be rejected")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            std::fs::read(&sentinel).expect("sentinel body"),
            b"unchanged"
        );
        assert!(std::fs::symlink_metadata(&lock_path)
            .expect("link metadata")
            .file_type()
            .is_symlink());
        let _ = std::fs::remove_dir_all(parent.parent().expect("fixture root"));
    }

    #[test]
    fn crashed_lock_owner_is_recovered_while_live_owner_is_excluded() {
        use std::io::Read;

        let (_, target, _) = fixture("crashed-lock");
        let root = target
            .parent()
            .and_then(Path::parent)
            .expect("fixture root")
            .to_path_buf();
        let ready = root.join("child-ready");
        let exit = root.join("child-exit");
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("agent_restore::hooks::tests::hook_edit_lock_child_process")
            .arg("--nocapture")
            .env("TM_HOOK_LOCK_CHILD_TARGET", &target)
            .env("TM_HOOK_LOCK_CHILD_READY", &ready)
            .env("TM_HOOK_LOCK_CHILD_EXIT", &exit)
            .spawn()
            .expect("spawn lock owner");

        let mut ready_seen = false;
        for _ in 0..500 {
            if ready.exists() {
                ready_seen = true;
                break;
            }
            if child.try_wait().expect("child status").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let live_result = if ready_seen {
            HookEditLock::acquire(&target)
        } else {
            Err(io::Error::other("child never acquired lock"))
        };

        std::fs::write(&exit, b"exit without destructors").expect("signal child exit");
        let status = child.wait().expect("wait for lock owner");
        let recovered = HookEditLock::acquire(&target);

        assert!(ready_seen, "child process never acquired the lock");
        assert_eq!(
            live_result
                .err()
                .expect("live owner must exclude another editor")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(!status.success(), "child should bypass lock drop on exit");
        let mut recovered = recovered.expect("kernel must release a crashed owner's lock");
        let mut owner_body = Vec::new();
        recovered
            .file
            .seek(std::io::SeekFrom::Start(0))
            .expect("rewind lock owner");
        recovered
            .file
            .read_to_end(&mut owner_body)
            .expect("read lock owner");
        let owner: Value = serde_json::from_slice(&owner_body).expect("owner json");
        assert_eq!(owner["pid"], std::process::id());
        assert!(owner["nonce"]
            .as_str()
            .is_some_and(|nonce| !nonce.is_empty()));
        assert!(owner["acquired_unix_ms"].as_u64().is_some());
        drop(recovered);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hook_edit_lock_child_process() {
        let Some(target) = std::env::var_os("TM_HOOK_LOCK_CHILD_TARGET") else {
            return;
        };
        let ready =
            PathBuf::from(std::env::var_os("TM_HOOK_LOCK_CHILD_READY").expect("child ready path"));
        let exit =
            PathBuf::from(std::env::var_os("TM_HOOK_LOCK_CHILD_EXIT").expect("child exit path"));
        let _lock = HookEditLock::acquire(Path::new(&target)).expect("child lock");
        std::fs::write(ready, b"ready").expect("announce child lock");
        for _ in 0..3_000 {
            if exit.exists() {
                // Deliberately bypass Rust destructors to emulate a process crash.
                std::process::exit(86);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::process::exit(87);
    }
}
