use std::path::PathBuf;

use terminal_manager_diagnostics::ObserveMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopRegressionOpts {
    pub list: bool,
    pub suite_ids: Vec<String>,
    pub skip_build: bool,
    pub exe_path: Option<PathBuf>,
    pub observe: ObserveMode,
    pub interactive: bool,
    pub keep_open_on_failure: bool,
    pub record: bool,
    pub artifact_root: PathBuf,
}

impl Default for DesktopRegressionOpts {
    fn default() -> Self {
        Self {
            list: false,
            suite_ids: Vec::new(),
            skip_build: false,
            exe_path: None,
            observe: ObserveMode::Basic,
            interactive: false,
            keep_open_on_failure: false,
            record: false,
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
            || opts.exe_path.is_some()
            || opts.observe != DesktopRegressionOpts::default().observe
            || opts.interactive
            || opts.keep_open_on_failure
            || opts.record
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
            "--artifact-root",
            "target/dr",
        ])
        .unwrap();

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
        let opts = parse(&["--list", "--suite", "edge-resize-stability"]).unwrap();
        let err = validate_options(&opts).unwrap_err();
        assert!(err.contains("--list"));
    }

    #[test]
    fn parses_equals_forms_and_alias_artifact_root() {
        let opts = parse(&[
            "--suite=post-resize-glitches",
            "--observe=off",
            "--artifacts-root=custom/artifacts",
        ])
        .unwrap();

        assert_eq!(opts.suite_ids, vec!["post-resize-glitches"]);
        assert_eq!(opts.observe, ObserveMode::Off);
        assert_eq!(opts.artifact_root, PathBuf::from("custom/artifacts"));
    }
}
