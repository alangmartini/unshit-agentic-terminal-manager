//! Hand-rolled CLI parser for the xtask binary.
//!
//! Kept dep-free so `cargo xtask` compiles in seconds.

use std::path::PathBuf;

use crate::desktop_regression::options::{parse_desktop_regression, DesktopRegressionOpts};

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    DesktopRegression(DesktopRegressionOpts),
    Profile(ProfileOpts),
    TypingPerf(TypingPerfOpts),
    Help,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ProfileKind {
    Cpu,
    Memory,
    All,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileOpts {
    pub kind: ProfileKind,
    pub out_dir: PathBuf,
    pub rate: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TypingPerfMode {
    Human,
    Stress,
    All,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TypingPerfOpts {
    pub runs: u32,
    pub duration_secs: u64,
    pub mode: TypingPerfMode,
    pub out_dir: PathBuf,
    pub skip_build: bool,
}

impl Default for TypingPerfOpts {
    fn default() -> Self {
        Self {
            runs: 3,
            duration_secs: 8,
            mode: TypingPerfMode::All,
            out_dir: PathBuf::from("target/perf/typing"),
            skip_build: false,
        }
    }
}

impl Default for ProfileOpts {
    fn default() -> Self {
        Self {
            kind: ProfileKind::All,
            out_dir: PathBuf::from("target/profile"),
            rate: 1000,
        }
    }
}

impl Cli {
    /// Parse raw argv (without the binary name).
    pub fn parse<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Result<Self, String> {
        let mut iter = args
            .iter()
            .map(|s| s.as_ref().to_string_lossy().into_owned());
        let first = match iter.next() {
            Some(s) => s,
            None => return Ok(Cli::Help),
        };

        match first.as_str() {
            "-h" | "--help" | "help" => Ok(Cli::Help),
            "desktop-regression" => parse_desktop_regression(iter).map(Cli::DesktopRegression),
            "profile" => parse_profile(iter),
            "typing-perf" => parse_typing_perf(iter),
            other => Err(format!(
                "unknown subcommand '{other}'. Run `cargo xtask --help`."
            )),
        }
    }
}

fn parse_typing_perf(mut iter: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut opts = TypingPerfOpts::default();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--runs" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--runs requires a value".to_owned())?;
                opts.runs = parse_positive_u32("--runs", &value)?;
            }
            "--duration" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--duration requires a value".to_owned())?;
                opts.duration_secs = parse_duration_secs(&value)?;
            }
            "--mode" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--mode requires a value".to_owned())?;
                opts.mode = parse_typing_perf_mode(&value)?;
            }
            "--out-dir" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--out-dir requires a value".to_owned())?;
                opts.out_dir = PathBuf::from(value);
            }
            "--skip-build" => opts.skip_build = true,
            "-h" | "--help" | "help" => return Ok(Cli::Help),
            other if other.starts_with("--runs=") => {
                opts.runs = parse_positive_u32("--runs", &other["--runs=".len()..])?;
            }
            other if other.starts_with("--duration=") => {
                opts.duration_secs = parse_duration_secs(&other["--duration=".len()..])?;
            }
            other if other.starts_with("--mode=") => {
                opts.mode = parse_typing_perf_mode(&other["--mode=".len()..])?;
            }
            other if other.starts_with("--out-dir=") => {
                opts.out_dir = PathBuf::from(&other["--out-dir=".len()..]);
            }
            other => return Err(format!("unknown flag '{other}'")),
        }
    }
    Ok(Cli::TypingPerf(opts))
}

fn parse_positive_u32(flag: &str, value: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| format!("{flag} expected a positive integer, got '{value}'"))?;
    if parsed == 0 {
        return Err(format!("{flag} must be at least 1"));
    }
    Ok(parsed)
}

fn parse_duration_secs(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("--duration expected integer seconds, got '{value}'"))?;
    if parsed < 4 {
        return Err("--duration must be at least 4 seconds".to_owned());
    }
    Ok(parsed)
}

fn parse_typing_perf_mode(value: &str) -> Result<TypingPerfMode, String> {
    match value {
        "human" => Ok(TypingPerfMode::Human),
        "stress" => Ok(TypingPerfMode::Stress),
        "all" => Ok(TypingPerfMode::All),
        _ => Err(format!(
            "--mode expected human, stress, or all; got '{value}'"
        )),
    }
}

fn parse_profile(mut iter: impl Iterator<Item = String>) -> Result<Cli, String> {
    let kind_arg = iter
        .next()
        .ok_or_else(|| "`profile` requires a kind: cpu, memory, or all".to_string())?;

    if matches!(kind_arg.as_str(), "-h" | "--help" | "help") {
        return Ok(Cli::Help);
    }

    let kind = match kind_arg.as_str() {
        "cpu" => ProfileKind::Cpu,
        "memory" | "mem" | "heap" => ProfileKind::Memory,
        "all" => ProfileKind::All,
        other => {
            return Err(format!(
                "unknown profile kind '{other}' (expected cpu|memory|all)"
            ))
        }
    };

    let mut opts = ProfileOpts {
        kind,
        ..ProfileOpts::default()
    };

    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--out-dir" => {
                let v = iter
                    .next()
                    .ok_or_else(|| "--out-dir requires a value".to_string())?;
                opts.out_dir = PathBuf::from(v);
            }
            "--rate" => {
                let v = iter
                    .next()
                    .ok_or_else(|| "--rate requires a value".to_string())?;
                opts.rate = v
                    .parse()
                    .map_err(|_| format!("--rate expected integer, got '{v}'"))?;
            }
            "-h" | "--help" => return Ok(Cli::Help),
            other if other.starts_with("--out-dir=") => {
                opts.out_dir = PathBuf::from(&other["--out-dir=".len()..]);
            }
            other if other.starts_with("--rate=") => {
                let v = &other["--rate=".len()..];
                opts.rate = v
                    .parse()
                    .map_err(|_| format!("--rate expected integer, got '{v}'"))?;
            }
            other => return Err(format!("unknown flag '{other}'")),
        }
    }

    Ok(Cli::Profile(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &[&str]) -> Result<Cli, String> {
        Cli::parse(raw)
    }

    #[test]
    fn no_args_shows_help() {
        assert_eq!(parse(&[]).unwrap(), Cli::Help);
    }

    #[test]
    fn dash_h_shows_help() {
        assert_eq!(parse(&["-h"]).unwrap(), Cli::Help);
        assert_eq!(parse(&["--help"]).unwrap(), Cli::Help);
        assert_eq!(parse(&["help"]).unwrap(), Cli::Help);
    }

    #[test]
    fn profile_help_shows_help() {
        assert_eq!(parse(&["profile", "--help"]).unwrap(), Cli::Help);
        assert_eq!(parse(&["profile", "cpu", "--help"]).unwrap(), Cli::Help);
    }

    #[test]
    fn profile_requires_kind() {
        assert!(parse(&["profile"]).is_err());
    }

    #[test]
    fn profile_rejects_unknown_kind() {
        let err = parse(&["profile", "bogus"]).unwrap_err();
        assert!(err.contains("bogus"));
    }

    #[test]
    fn profile_cpu_defaults() {
        let cli = parse(&["profile", "cpu"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.kind, ProfileKind::Cpu);
        assert_eq!(opts.out_dir, PathBuf::from("target/profile"));
        assert_eq!(opts.rate, 1000);
    }

    #[test]
    fn profile_memory_accepts_aliases() {
        for alias in ["memory", "mem", "heap"] {
            let cli = parse(&["profile", alias]).unwrap();
            let Cli::Profile(opts) = cli else {
                panic!("expected Profile");
            };
            assert_eq!(opts.kind, ProfileKind::Memory);
        }
    }

    #[test]
    fn profile_all_parses() {
        let cli = parse(&["profile", "all"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.kind, ProfileKind::All);
    }

    #[test]
    fn profile_out_dir_space_separated() {
        let cli = parse(&["profile", "cpu", "--out-dir", "custom/dir"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.out_dir, PathBuf::from("custom/dir"));
    }

    #[test]
    fn profile_out_dir_equals_separated() {
        let cli = parse(&["profile", "cpu", "--out-dir=custom/dir"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.out_dir, PathBuf::from("custom/dir"));
    }

    #[test]
    fn profile_rate_parses() {
        let cli = parse(&["profile", "cpu", "--rate", "500"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.rate, 500);
    }

    #[test]
    fn profile_rate_equals_parses() {
        let cli = parse(&["profile", "cpu", "--rate=250"]).unwrap();
        let Cli::Profile(opts) = cli else {
            panic!("expected Profile");
        };
        assert_eq!(opts.rate, 250);
    }

    #[test]
    fn profile_rate_rejects_non_integer() {
        assert!(parse(&["profile", "cpu", "--rate", "fast"]).is_err());
    }

    #[test]
    fn profile_unknown_flag_errors() {
        let err = parse(&["profile", "cpu", "--bogus"]).unwrap_err();
        assert!(err.contains("--bogus"));
    }

    #[test]
    fn out_dir_missing_value_errors() {
        assert!(parse(&["profile", "cpu", "--out-dir"]).is_err());
    }

    #[test]
    fn rate_missing_value_errors() {
        assert!(parse(&["profile", "cpu", "--rate"]).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        let err = parse(&["frobnicate"]).unwrap_err();
        assert!(err.contains("frobnicate"));
    }

    #[test]
    fn desktop_regression_subcommand_parses() {
        let cli = parse(&["desktop-regression", "--list"]).unwrap();
        let Cli::DesktopRegression(opts) = cli else {
            panic!("expected DesktopRegression");
        };
        assert!(opts.list);
    }

    #[test]
    fn typing_perf_defaults_to_repeated_human_and_stress_runs() {
        let cli = parse(&["typing-perf"]).unwrap();
        let Cli::TypingPerf(opts) = cli else {
            panic!("expected TypingPerf");
        };
        assert_eq!(opts.runs, 3);
        assert_eq!(opts.duration_secs, 8);
        assert_eq!(opts.mode, TypingPerfMode::All);
        assert_eq!(opts.out_dir, PathBuf::from("target/perf/typing"));
        assert!(!opts.skip_build);
    }

    #[test]
    fn typing_perf_parses_run_controls() {
        let cli = parse(&[
            "typing-perf",
            "--runs",
            "5",
            "--duration",
            "12",
            "--mode",
            "stress",
            "--out-dir",
            "target/custom-typing",
            "--skip-build",
        ])
        .unwrap();
        let Cli::TypingPerf(opts) = cli else {
            panic!("expected TypingPerf");
        };
        assert_eq!(opts.runs, 5);
        assert_eq!(opts.duration_secs, 12);
        assert_eq!(opts.mode, TypingPerfMode::Stress);
        assert_eq!(opts.out_dir, PathBuf::from("target/custom-typing"));
        assert!(opts.skip_build);
    }

    #[test]
    fn typing_perf_rejects_zero_runs_and_short_duration() {
        assert!(parse(&["typing-perf", "--runs", "0"]).is_err());
        assert!(parse(&["typing-perf", "--duration", "2"]).is_err());
    }
}
