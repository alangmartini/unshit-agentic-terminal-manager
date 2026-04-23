//! `unshit-ptyd` daemon crate.
//!
//! Slice 2 of the tmux-style persistence work (see `SPEC.md`). This
//! crate owns the IPC transport, the wire protocol, and the daemon
//! event loop for hello / shutdown. Session lifecycle and PTY ownership
//! arrive in slice 3.

pub mod protocol;

/// Daemon version pulled from the crate manifest. Reported on the wire
/// in `HelloAck` so clients can version-gate behavior.
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed CLI invocation.
///
/// `Run` is a loud-failing stub in slice 1 so accidental callers fail
/// instead of silently hanging; the real daemon loop lands in slice 2.
#[derive(Debug, PartialEq, Eq)]
pub enum ParsedArgs {
    Run,
    Status,
    Help,
    Version,
}

/// Errors produced by [`parse_args`].
#[derive(Debug, PartialEq, Eq)]
pub enum ArgError {
    UnknownFlag(String),
    UnexpectedPositional(String),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgError::UnknownFlag(flag) => write!(f, "unknown flag: {flag}"),
            ArgError::UnexpectedPositional(v) => write!(f, "unexpected positional argument: {v}"),
        }
    }
}

impl std::error::Error for ArgError {}

/// Parse CLI arguments.
///
/// The input should NOT include `argv[0]`. Caller is responsible for
/// stripping it (e.g. `std::env::args().skip(1)`).
pub fn parse_args<I, S>(args: I) -> Result<ParsedArgs, ArgError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut iter = args.into_iter();
    let Some(first) = iter.next() else {
        return Ok(ParsedArgs::Run);
    };
    let parsed = match first.as_ref() {
        "--status" => ParsedArgs::Status,
        "--help" | "-h" => ParsedArgs::Help,
        "--version" | "-V" => ParsedArgs::Version,
        other if other.starts_with('-') => {
            return Err(ArgError::UnknownFlag(other.to_string()));
        }
        other => {
            return Err(ArgError::UnexpectedPositional(other.to_string()));
        }
    };
    if let Some(extra) = iter.next() {
        let s = extra.as_ref();
        return Err(if s.starts_with('-') {
            ArgError::UnknownFlag(s.to_string())
        } else {
            ArgError::UnexpectedPositional(s.to_string())
        });
    }
    Ok(parsed)
}

/// One-line health banner printed for `--status`.
///
/// Whitespace-separated so `awk '{print $3}'` keeps working as future
/// slices add fields; format is pinned by tests.
pub fn status_line() -> String {
    format!("unshit-ptyd {} idle sessions=0", DAEMON_VERSION)
}

/// Help text printed for `--help`.
pub const HELP_TEXT: &str = "\
unshit-ptyd - background PTY daemon for terminal-manager

USAGE:
    unshit-ptyd [FLAGS]

FLAGS:
    --status       Print a one-line health banner and exit
    --help, -h     Print this help and exit
    --version, -V  Print version and exit

With no flags, runs the daemon event loop (not implemented in slice 1).
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParsedArgs, ArgError> {
        parse_args(args.iter().copied())
    }

    #[test]
    fn no_args_runs_daemon() {
        assert_eq!(parse(&[]), Ok(ParsedArgs::Run));
    }

    #[test]
    fn status_flag_parsed() {
        assert_eq!(parse(&["--status"]), Ok(ParsedArgs::Status));
    }

    #[test]
    fn help_flag_long_parsed() {
        assert_eq!(parse(&["--help"]), Ok(ParsedArgs::Help));
    }

    #[test]
    fn help_flag_short_parsed() {
        assert_eq!(parse(&["-h"]), Ok(ParsedArgs::Help));
    }

    #[test]
    fn version_flag_long_parsed() {
        assert_eq!(parse(&["--version"]), Ok(ParsedArgs::Version));
    }

    #[test]
    fn version_flag_short_parsed() {
        assert_eq!(parse(&["-V"]), Ok(ParsedArgs::Version));
    }

    #[test]
    fn unknown_long_flag_rejected() {
        assert_eq!(
            parse(&["--nope"]),
            Err(ArgError::UnknownFlag("--nope".to_string()))
        );
    }

    #[test]
    fn unknown_short_flag_rejected() {
        assert_eq!(parse(&["-q"]), Err(ArgError::UnknownFlag("-q".to_string())));
    }

    #[test]
    fn positional_argument_rejected() {
        assert_eq!(
            parse(&["run"]),
            Err(ArgError::UnexpectedPositional("run".to_string()))
        );
    }

    #[test]
    fn trailing_positional_after_flag_rejected() {
        assert_eq!(
            parse(&["--status", "extra"]),
            Err(ArgError::UnexpectedPositional("extra".to_string()))
        );
    }

    #[test]
    fn trailing_flag_after_flag_rejected() {
        assert_eq!(
            parse(&["--status", "--also"]),
            Err(ArgError::UnknownFlag("--also".to_string()))
        );
    }

    #[test]
    fn status_line_contains_version() {
        let line = status_line();
        assert!(
            line.contains(DAEMON_VERSION),
            "status line missing version: {line}"
        );
    }

    #[test]
    fn status_line_single_line() {
        let line = status_line();
        assert!(
            !line.contains('\n'),
            "status line must be one line: {line:?}"
        );
    }

    #[test]
    fn status_line_starts_with_daemon_name() {
        let line = status_line();
        assert!(
            line.starts_with("unshit-ptyd "),
            "status line must start with daemon name: {line:?}"
        );
    }

    #[test]
    fn status_line_reports_session_count() {
        // Tests that the banner stays scriptable: the session count
        // field must be present so future slices can set it without
        // reshaping the output. Awk-style column indexing depends on
        // this staying stable.
        let line = status_line();
        assert!(
            line.contains("sessions="),
            "status line must include sessions= field: {line:?}"
        );
    }

    #[test]
    fn arg_error_display_is_human_readable() {
        let e = ArgError::UnknownFlag("--nope".to_string());
        let s = format!("{e}");
        assert!(s.contains("--nope"), "display missing flag text: {s:?}");
        assert!(
            s.starts_with("unknown flag"),
            "display should start with category: {s:?}"
        );
    }

    #[test]
    fn help_text_mentions_all_flags() {
        assert!(HELP_TEXT.contains("--status"), "help missing --status");
        assert!(HELP_TEXT.contains("--help"), "help missing --help");
        assert!(HELP_TEXT.contains("--version"), "help missing --version");
    }
}
