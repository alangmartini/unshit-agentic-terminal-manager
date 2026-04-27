//! Shell selection types and resolution.
//!
//! `ShellSpec` carries the program (path or PATH lookup name) plus its
//! launch args. Resolution prefers a per workspace override over the app
//! wide default; both empty means "let the daemon's `default_shell()`
//! decide", preserving the pre feature behavior.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A shell program plus its launch args. Stored in `workspaces.json`
/// and forwarded across IPC as `(shell, shell_args)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSpec {
    /// Absolute path or PATH lookup name. Empty means "fall back".
    pub program: String,
    /// Args forwarded to the program before any daemon side cwd args.
    #[serde(default)]
    pub args: Vec<String>,
}

impl ShellSpec {
    /// Returns true when `program` is empty, regardless of args. An
    /// empty `program` is treated as "no preference set".
    pub fn is_empty(&self) -> bool {
        self.program.is_empty()
    }
}

/// Resolve which shell a pane should spawn with. Workspace override
/// wins over the app wide default; both `None` (or both `is_empty`)
/// means "let the daemon decide", preserving today's behavior.
pub fn resolve(workspace: Option<&ShellSpec>, app: Option<&ShellSpec>) -> Option<ShellSpec> {
    workspace
        .filter(|s| !s.is_empty())
        .or(app.filter(|s| !s.is_empty()))
        .cloned()
}

/// Pick a sensible default shell from a list of discovered binaries.
/// Prefers `pwsh` over `powershell` so users on a fresh machine land
/// on the modern shell. Returns an empty spec when no preferred shell
/// is present so the daemon's own `default_shell()` keeps the floor.
pub fn infer_default_shell(installed: &[PathBuf]) -> ShellSpec {
    for preferred in ["pwsh", "powershell"] {
        if let Some(hit) = installed.iter().find(|p| stem_matches(p, preferred)) {
            return ShellSpec {
                program: hit.display().to_string(),
                args: Vec::new(),
            };
        }
    }
    ShellSpec::default()
}

fn stem_matches(path: &std::path::Path, name: &str) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

/// Stems we probe for on PATH and at well known install locations.
/// Order matters for the Settings dropdown: discovered shells appear
/// in stem order, then in PATH order within each stem.
const STEMS: &[&str] = &[
    "pwsh",
    "powershell",
    "cmd",
    "bash",
    "zsh",
    "fish",
    "nu",
    "wsl",
];

/// Cap on the number of discovered binaries returned. Keeps the
/// Settings dropdown manageable even on machines with PATHs full of
/// shell shims.
const MAX_DISCOVERED: usize = 16;

/// Walk PATH for known shell stems plus a small set of well known
/// fixed install paths. Deduplicates by canonical path so the same
/// binary reachable via two PATH entries (or via PATH and a fixed
/// probe) only shows up once. Capped at `MAX_DISCOVERED` so a
/// pathological PATH can't blow up the UI.
pub fn discover_installed() -> Vec<PathBuf> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();
    discover_from(&dirs, &fixed_well_known_paths())
}

/// Per stem extension. On Windows we want `.exe`; everywhere else the
/// bare stem is the executable name.
fn executable_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Locations we probe in addition to PATH. Today this covers the two
/// common Windows installs that often live outside PATH: Git Bash and
/// WSL. Other platforms get an empty list.
fn fixed_well_known_paths() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\Windows\System32\wsl.exe"),
        ]
    } else {
        Vec::new()
    }
}

/// Pure variant of [`discover_installed`] that takes the inputs
/// explicitly so unit tests can drive it without touching real env.
fn discover_from(path_dirs: &[PathBuf], fixed: &[PathBuf]) -> Vec<PathBuf> {
    use std::collections::HashSet;

    let mut out: Vec<PathBuf> = Vec::new();
    let mut canonical_seen: HashSet<PathBuf> = HashSet::new();

    'walk: for dir in path_dirs {
        for stem in STEMS {
            if out.len() >= MAX_DISCOVERED {
                break 'walk;
            }
            let candidate = dir.join(executable_name(stem));
            try_push(candidate, &mut out, &mut canonical_seen);
        }
    }
    for path in fixed {
        if out.len() >= MAX_DISCOVERED {
            break;
        }
        try_push(path.clone(), &mut out, &mut canonical_seen);
    }
    out
}

fn try_push(path: PathBuf, out: &mut Vec<PathBuf>, seen: &mut std::collections::HashSet<PathBuf>) {
    if !path.is_file() {
        return;
    }
    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if seen.insert(canonical) {
        out.push(path);
    }
}

/// Build display labels for a discovered shell list. When a stem
/// is unique the label is the bare stem (e.g. `pwsh`). When a stem
/// repeats (multiple `bash.exe` installs are common on Windows: Git
/// Bash, MSYS2, etc.), the label is suffixed with the parent dir to
/// disambiguate; if every duplicate's parent dir is also identical
/// (e.g. all in `bin/`), the grandparent dir is included. The result
/// is positionally aligned with `installed`.
pub fn label_installed_shells(installed: &[PathBuf]) -> Vec<String> {
    use std::collections::HashMap;
    let stems: Vec<String> = installed.iter().map(|p| display_stem(p)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for s in &stems {
        *counts.entry(s.as_str()).or_insert(0) += 1;
    }
    installed
        .iter()
        .zip(stems.iter())
        .map(|(path, stem)| {
            if counts.get(stem.as_str()).copied().unwrap_or(0) <= 1 {
                return stem.clone();
            }
            let suffix = disambiguating_suffix(path);
            if suffix.is_empty() {
                stem.clone()
            } else {
                format!("{stem} ({suffix})")
            }
        })
        .collect()
}

fn display_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// For a duplicate-stem path, pick a short ancestor segment that
/// distinguishes installs. Falls back through parent and grandparent
/// because the immediate parent for shells is often a generic `bin`.
fn disambiguating_suffix(path: &std::path::Path) -> String {
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let grand = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match (parent, grand) {
        ("", "") => String::new(),
        (p, "") => p.to_string(),
        (p, g) if matches!(p, "bin" | "usr") && !g.is_empty() => format!("{g}\\{p}"),
        (p, _) => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str]) -> ShellSpec {
        ShellSpec {
            program: program.into(),
            args: args.iter().map(|a| (*a).into()).collect(),
        }
    }

    #[test]
    fn is_empty_returns_true_for_default() {
        assert!(ShellSpec::default().is_empty());
    }

    #[test]
    fn is_empty_returns_true_when_program_is_blank_even_with_args() {
        let s = spec("", &["--login"]);
        assert!(
            s.is_empty(),
            "is_empty should look only at program; args alone do not make a spec set"
        );
    }

    #[test]
    fn is_empty_returns_false_when_program_is_set() {
        let s = spec("pwsh.exe", &[]);
        assert!(!s.is_empty());
    }

    #[test]
    fn resolve_returns_workspace_override_when_both_set() {
        let ws = spec("pwsh.exe", &["-NoLogo"]);
        let app = spec("powershell.exe", &[]);
        let got = resolve(Some(&ws), Some(&app));
        assert_eq!(got, Some(ws));
    }

    #[test]
    fn resolve_falls_back_to_app_default_when_workspace_is_empty() {
        let ws = ShellSpec::default();
        let app = spec("pwsh.exe", &["-NoLogo"]);
        let got = resolve(Some(&ws), Some(&app));
        assert_eq!(got, Some(app));
    }

    #[test]
    fn resolve_returns_none_when_both_are_empty() {
        let ws = ShellSpec::default();
        let app = ShellSpec::default();
        let got = resolve(Some(&ws), Some(&app));
        assert!(
            got.is_none(),
            "both empty must yield None so the daemon falls back to default_shell()"
        );
    }

    #[test]
    fn resolve_returns_none_when_both_are_unset() {
        let got = resolve(None, None);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_falls_back_to_app_when_workspace_is_none() {
        let app = spec("bash", &["--login"]);
        let got = resolve(None, Some(&app));
        assert_eq!(got, Some(app));
    }

    #[test]
    fn shell_spec_round_trips_through_serde_json() {
        let original = spec("pwsh.exe", &["-NoLogo", "-NoProfile"]);
        let s = serde_json::to_string(&original).unwrap();
        let back: ShellSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn shell_spec_deserializes_with_default_args_when_field_is_missing() {
        // Old configs (or hand edited ones) may omit the args field
        // entirely. Serde must default it to an empty vector so the
        // upgrade path is silent.
        let json = r#"{"program":"pwsh.exe"}"#;
        let got: ShellSpec = serde_json::from_str(json).unwrap();
        assert_eq!(got.program, "pwsh.exe");
        assert!(
            got.args.is_empty(),
            "missing args field must deserialize to an empty vector"
        );
    }

    #[test]
    fn infer_default_shell_prefers_pwsh_over_powershell() {
        let installed = vec![
            PathBuf::from("/usr/bin/powershell.exe"),
            PathBuf::from("/usr/bin/pwsh.exe"),
        ];
        let got = infer_default_shell(&installed);
        assert_eq!(got.program, "/usr/bin/pwsh.exe");
        assert!(got.args.is_empty());
    }

    #[test]
    fn infer_default_shell_picks_powershell_when_pwsh_missing() {
        let installed = vec![PathBuf::from("/usr/bin/powershell.exe")];
        let got = infer_default_shell(&installed);
        assert_eq!(got.program, "/usr/bin/powershell.exe");
        assert!(got.args.is_empty());
    }

    #[test]
    fn infer_default_shell_returns_empty_spec_when_no_preferred_shell_present() {
        // Daemon should fall back to its own `default_shell()` when the
        // UI hands back an empty spec.
        let installed = vec![
            PathBuf::from("/bin/cmd"),
            PathBuf::from("/bin/zsh"),
            PathBuf::from("/bin/fish"),
        ];
        let got = infer_default_shell(&installed);
        assert!(
            got.is_empty(),
            "no preferred shell discovered must yield an empty spec, got {got:?}"
        );
    }

    #[test]
    fn infer_default_shell_returns_empty_spec_for_empty_install_list() {
        let got = infer_default_shell(&[]);
        assert!(got.is_empty());
    }

    #[test]
    fn infer_default_shell_match_is_case_insensitive_on_stem() {
        // discover_installed may return paths preserving on disk casing
        // ("Pwsh.EXE" on Windows). The match must still pick it.
        let installed = vec![PathBuf::from("/opt/Pwsh.EXE")];
        let got = infer_default_shell(&installed);
        assert_eq!(got.program, "/opt/Pwsh.EXE");
    }

    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("godly-discover-{tag}-{pid}-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp dir for discover test");
        dir
    }

    fn touch(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir for touch");
        }
        std::fs::write(path, b"").expect("touch test file");
    }

    fn fake_shell_in(dir: &std::path::Path, stem: &str) -> PathBuf {
        let path = dir.join(executable_name(stem));
        touch(&path);
        path
    }

    #[test]
    fn discover_from_returns_empty_when_no_dirs_and_no_fixed() {
        let got = discover_from(&[], &[]);
        assert!(got.is_empty());
    }

    #[test]
    fn discover_from_finds_known_shell_in_a_path_dir() {
        let dir = unique_temp_dir("finds");
        let bash = fake_shell_in(&dir, "bash");
        let got = discover_from(std::slice::from_ref(&dir), &[]);
        assert!(got.iter().any(|p| p == &bash), "expected bash in {got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_from_dedupes_when_same_dir_listed_twice() {
        let dir = unique_temp_dir("dedup-dirs");
        let _bash = fake_shell_in(&dir, "bash");
        let got = discover_from(&[dir.clone(), dir.clone()], &[]);
        let bash_hits = got
            .iter()
            .filter(|p| p.file_stem().and_then(|s| s.to_str()) == Some("bash"))
            .count();
        assert_eq!(bash_hits, 1, "duplicate dirs must collapse, got {got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_from_skips_missing_files() {
        // dir exists but contains no shell binaries
        let dir = unique_temp_dir("missing");
        let got = discover_from(std::slice::from_ref(&dir), &[]);
        assert!(got.is_empty(), "empty dir must yield no hits, got {got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_from_includes_fixed_paths_when_they_exist() {
        let dir = unique_temp_dir("fixed");
        let bash = fake_shell_in(&dir, "bash");
        // Pretend bash also lives at a "well known" location by passing
        // it as a fixed path. Dedup will collapse it but absent dedup
        // it would still appear because the file exists.
        let got = discover_from(&[], std::slice::from_ref(&bash));
        assert_eq!(got, vec![bash.clone()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_from_skips_missing_fixed_paths() {
        let missing = unique_temp_dir("fixed-missing").join(executable_name("does-not-exist"));
        assert!(!missing.exists());
        let got = discover_from(&[], &[missing]);
        assert!(got.is_empty());
    }

    #[test]
    fn discover_from_is_capped_at_max_discovered() {
        // Pile MAX_DISCOVERED + 5 fake binaries into the fixed list and
        // verify the cap holds.
        let dir = unique_temp_dir("cap");
        let mut fixed: Vec<PathBuf> = Vec::new();
        for i in 0..(MAX_DISCOVERED + 5) {
            let name = if cfg!(windows) {
                format!("fake{i}.exe")
            } else {
                format!("fake{i}")
            };
            let path = dir.join(name);
            touch(&path);
            fixed.push(path);
        }
        let got = discover_from(&[], &fixed);
        assert_eq!(got.len(), MAX_DISCOVERED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_installed_returns_stable_order_across_calls() {
        let a = discover_installed();
        let b = discover_installed();
        assert_eq!(a, b, "discover_installed must be deterministic");
    }

    #[test]
    fn discover_installed_does_not_panic() {
        // Smoke test: the public API must not panic even if PATH is
        // weird. This exercises the production path that hits real env.
        let _ = discover_installed();
    }

    #[test]
    fn label_installed_shells_returns_bare_stem_when_unique() {
        let installed = vec![
            PathBuf::from("/usr/bin/pwsh"),
            PathBuf::from("/usr/bin/bash"),
        ];
        let labels = label_installed_shells(&installed);
        assert_eq!(labels, vec!["pwsh".to_string(), "bash".to_string()]);
    }

    #[test]
    fn label_installed_shells_disambiguates_duplicate_stems_with_parent() {
        // Different parent dirs distinguish two bash.exe installs.
        let installed = vec![
            PathBuf::from(r"C:\Program Files\Git\cmd\bash.exe"),
            PathBuf::from(r"C:\msys64\bash.exe"),
        ];
        let labels = label_installed_shells(&installed);
        assert_eq!(
            labels,
            vec!["bash (cmd)".to_string(), "bash (msys64)".to_string()]
        );
    }

    #[test]
    fn label_installed_shells_uses_grandparent_when_parent_is_generic_bin() {
        // Two bash installs, both with parent "bin": the grandparent
        // segment carries the actual identity (Git vs msys64).
        let installed = vec![
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\msys64\usr\bin\bash.exe"),
        ];
        let labels = label_installed_shells(&installed);
        assert_eq!(
            labels,
            vec!["bash (Git\\bin)".to_string(), "bash (usr\\bin)".to_string()]
        );
    }

    #[test]
    fn label_installed_shells_does_not_disambiguate_unique_entries_in_a_mixed_list() {
        let installed = vec![
            PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            PathBuf::from(r"C:\msys64\usr\bin\bash.exe"),
        ];
        let labels = label_installed_shells(&installed);
        assert_eq!(labels[0], "pwsh");
        assert!(labels[1].starts_with("bash ("));
        assert!(labels[2].starts_with("bash ("));
        assert_ne!(labels[1], labels[2], "duplicate stems must be distinct");
    }
}
