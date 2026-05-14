use std::path::PathBuf;

use terminal_manager_diagnostics::ObserveMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopRegressionOpts {
    pub list: bool,
    pub sequential_isolated: bool,
    pub suite_ids: Vec<String>,
    pub skip_build: bool,
    pub exe_path: Option<PathBuf>,
    pub observe: ObserveMode,
    pub interactive: bool,
    pub keep_open_on_failure: bool,
    pub record: bool,
    pub replay: Option<PathBuf>,
    pub artifact_root: PathBuf,
}

impl Default for DesktopRegressionOpts {
    fn default() -> Self {
        Self {
            list: false,
            sequential_isolated: false,
            suite_ids: Vec::new(),
            skip_build: false,
            exe_path: None,
            observe: ObserveMode::Basic,
            interactive: false,
            keep_open_on_failure: false,
            record: false,
            replay: None,
            artifact_root: PathBuf::from("artifacts/windows/desktop-regression"),
        }
    }
}

pub fn parse_desktop_regression<I>(_iter: I) -> Result<DesktopRegressionOpts, String>
where
    I: Iterator<Item = String>,
{
    let mut iter = _iter;
    let mut opts = DesktopRegressionOpts::default();

    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--list" => opts.list = true,
            "--sequential-isolated" => opts.sequential_isolated = true,
            "--suite" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--suite requires a value".to_owned())?;
                opts.suite_ids.push(value);
            }
            "--skip-build" => opts.skip_build = true,
            "--exe-path" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--exe-path requires a value".to_owned())?;
                opts.exe_path = Some(PathBuf::from(value));
            }
            "--observe" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--observe requires a value".to_owned())?;
                opts.observe = parse_observe_mode(&value)?;
            }
            "--interactive" => opts.interactive = true,
            "--keep-open-on-failure" => opts.keep_open_on_failure = true,
            "--record" => opts.record = true,
            "--replay" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--replay requires a value".to_owned())?;
                opts.replay = Some(PathBuf::from(value));
            }
            "--artifact-root" | "--artifacts-root" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                opts.artifact_root = PathBuf::from(value);
            }
            "-h" | "--help" | "help" => {
                return Err("desktop-regression help is shown by `cargo xtask --help`".to_owned())
            }
            other if other.starts_with("--suite=") => {
                opts.suite_ids.push(other["--suite=".len()..].to_owned());
            }
            other if other.starts_with("--exe-path=") => {
                opts.exe_path = Some(PathBuf::from(&other["--exe-path=".len()..]));
            }
            other if other.starts_with("--observe=") => {
                opts.observe = parse_observe_mode(&other["--observe=".len()..])?;
            }
            other if other.starts_with("--replay=") => {
                opts.replay = Some(PathBuf::from(&other["--replay=".len()..]));
            }
            other if other.starts_with("--artifact-root=") => {
                opts.artifact_root = PathBuf::from(&other["--artifact-root=".len()..]);
            }
            other if other.starts_with("--artifacts-root=") => {
                opts.artifact_root = PathBuf::from(&other["--artifacts-root=".len()..]);
            }
            other => return Err(format!("unknown desktop-regression flag '{other}'")),
        }
    }

    Ok(opts)
}

pub fn validate_options(opts: &DesktopRegressionOpts) -> Result<(), String> {
    if opts.list
        && (!opts.suite_ids.is_empty()
            || opts.skip_build
            || opts.sequential_isolated
            || opts.exe_path.is_some()
            || opts.observe != DesktopRegressionOpts::default().observe
            || opts.interactive
            || opts.keep_open_on_failure
            || opts.record
            || opts.replay.is_some()
            || opts.artifact_root != DesktopRegressionOpts::default().artifact_root)
    {
        return Err("--list cannot be combined with run options".to_owned());
    }

    if opts.keep_open_on_failure && !opts.interactive {
        return Err("--keep-open-on-failure requires --interactive".to_owned());
    }

    if opts.skip_build && opts.exe_path.is_none() {
        return Err("--skip-build requires --exe-path".to_owned());
    }

    if opts.exe_path.is_some() && !opts.skip_build {
        return Err("--exe-path requires --skip-build".to_owned());
    }

    if opts.record && opts.replay.is_some() {
        return Err("--record cannot be combined with --replay".to_owned());
    }

    if opts.sequential_isolated && opts.replay.is_some() {
        return Err("--sequential-isolated cannot be combined with --replay".to_owned());
    }

    Ok(())
}

fn parse_observe_mode(value: &str) -> Result<ObserveMode, String> {
    match value {
        "off" => Ok(ObserveMode::Off),
        "basic" => Ok(ObserveMode::Basic),
        "full" => Ok(ObserveMode::Full),
        other => Err(format!(
            "invalid --observe value '{other}' (expected off|basic|full)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &[&str]) -> Result<DesktopRegressionOpts, String> {
        parse_desktop_regression(raw.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn parses_list_without_run_options() {
        let opts = parse(&["--list"]).unwrap();
        assert!(opts.list);
        assert_eq!(
            opts.artifact_root,
            PathBuf::from("artifacts/windows/desktop-regression")
        );
    }

    #[test]
    fn parses_run_options() {
        let opts = parse(&[
            "--sequential-isolated",
            "--suite",
            "edge-resize-stability",
            "--skip-build",
            "--exe-path",
            "target/debug/terminal-manager.exe",
            "--observe",
            "full",
            "--interactive",
            "--keep-open-on-failure",
            "--record",
            "--replay",
            "trace.jsonl",
            "--artifact-root",
            "target/dr",
        ])
        .unwrap();

        assert!(opts.sequential_isolated);
        assert_eq!(opts.suite_ids, vec!["edge-resize-stability"]);
        assert!(opts.skip_build);
        assert_eq!(
            opts.exe_path,
            Some(PathBuf::from("target/debug/terminal-manager.exe"))
        );
        assert_eq!(opts.observe, ObserveMode::Full);
        assert!(opts.interactive);
        assert!(opts.keep_open_on_failure);
        assert!(opts.record);
        assert_eq!(opts.replay, Some(PathBuf::from("trace.jsonl")));
        assert_eq!(opts.artifact_root, PathBuf::from("target/dr"));
    }

    #[test]
    fn rejects_invalid_observe_mode() {
        let err = parse(&["--observe", "verbose"]).unwrap_err();
        assert!(err.contains("verbose"));
    }

    #[test]
    fn rejects_keep_open_without_interactive() {
        let opts = parse(&["--keep-open-on-failure"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--interactive"));
    }

    #[test]
    fn rejects_skip_build_without_exe_path() {
        let opts = parse(&["--skip-build"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--exe-path"));
    }

    #[test]
    fn rejects_list_combined_with_run_options() {
        let opts = parse(&["--list", "--sequential-isolated"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--list"));
    }

    #[test]
    fn rejects_record_combined_with_replay() {
        let opts = parse(&["--record", "--replay", "trace.jsonl"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--record"));
    }

    #[test]
    fn rejects_sequential_isolated_combined_with_replay() {
        let opts = parse(&["--sequential-isolated", "--replay", "trace.jsonl"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--sequential-isolated"));
    }

    #[test]
    fn parses_equals_forms_and_alias_artifact_root() {
        let opts = parse(&[
            "--suite=post-resize-glitches",
            "--observe=off",
            "--replay=trace.jsonl",
            "--artifacts-root=custom/artifacts",
        ])
        .unwrap();

        assert_eq!(opts.suite_ids, vec!["post-resize-glitches"]);
        assert_eq!(opts.observe, ObserveMode::Off);
        assert_eq!(opts.replay, Some(PathBuf::from("trace.jsonl")));
        assert_eq!(opts.artifact_root, PathBuf::from("custom/artifacts"));
    }
}
