//! `unshit-ptyd` daemon crate.
//!
//! Slice 2 of the tmux-style persistence work (see `SPEC.md`). This
//! crate owns the IPC transport, the wire protocol, and the daemon
//! event loop for hello / shutdown. Session lifecycle and PTY ownership
//! arrive in slice 3.

pub mod client;
pub mod daemon;
pub mod protocol;
pub mod transport;

use std::path::PathBuf;

/// Daemon version pulled from the crate manifest. Reported on the wire
/// in `HelloAck` so clients can version-gate behavior.
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed CLI invocation.
#[derive(Debug, PartialEq, Eq)]
pub enum ParsedArgs {
    /// Run the daemon event loop on `socket` (or the default path).
    Run {
        socket: Option<PathBuf>,
    },
    /// Connect to an existing daemon and ask it to shut down.
    Shutdown {
        socket: Option<PathBuf>,
    },
    Status,
    Help,
    Version,
}

/// Errors produced by [`parse_args`].
#[derive(Debug, PartialEq, Eq)]
pub enum ArgError {
    UnknownFlag(String),
    UnexpectedPositional(String),
    /// `--socket` was the last token, or a flag that requires a value
    /// was followed by nothing.
    MissingValue(String),
    /// Two mode-selecting flags were passed together (e.g. `--status
    /// --shutdown`). Exactly one mode flag is allowed.
    ConflictingModes(String, String),
    /// A flag was accepted in isolation but is not valid in the chosen
    /// mode (e.g. `--socket` with `--status`).
    IncompatibleFlag {
        mode: String,
        flag: String,
    },
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgError::UnknownFlag(flag) => write!(f, "unknown flag: {flag}"),
            ArgError::UnexpectedPositional(v) => write!(f, "unexpected positional argument: {v}"),
            ArgError::MissingValue(flag) => write!(f, "missing value for flag: {flag}"),
            ArgError::ConflictingModes(a, b) => {
                write!(f, "conflicting mode flags: {a} and {b}")
            }
            ArgError::IncompatibleFlag { mode, flag } => {
                write!(f, "flag {flag} is not valid with {mode}")
            }
        }
    }
}

impl std::error::Error for ArgError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Run,
    Shutdown,
    Status,
    Help,
    Version,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Run => "(default run)",
            Mode::Shutdown => "--shutdown",
            Mode::Status => "--status",
            Mode::Help => "--help",
            Mode::Version => "--version",
        }
    }

    fn accepts_socket(self) -> bool {
        matches!(self, Mode::Run | Mode::Shutdown)
    }
}

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
    let mut mode: Option<Mode> = None;
    let mut socket: Option<PathBuf> = None;

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        let next_mode = match s {
            "--status" => Some(Mode::Status),
            "--help" | "-h" => Some(Mode::Help),
            "--version" | "-V" => Some(Mode::Version),
            "--shutdown" => Some(Mode::Shutdown),
            "--socket" => {
                let value = iter
                    .next()
                    .ok_or_else(|| ArgError::MissingValue("--socket".to_string()))?;
                socket = Some(PathBuf::from(value.as_ref()));
                None
            }
            other if other.starts_with('-') => {
                return Err(ArgError::UnknownFlag(other.to_string()));
            }
            other => {
                return Err(ArgError::UnexpectedPositional(other.to_string()));
            }
        };
        if let Some(m) = next_mode {
            if let Some(prev) = mode {
                return Err(ArgError::ConflictingModes(
                    prev.label().to_string(),
                    m.label().to_string(),
                ));
            }
            mode = Some(m);
        }
    }

    let resolved = mode.unwrap_or(Mode::Run);
    if socket.is_some() && !resolved.accepts_socket() {
        return Err(ArgError::IncompatibleFlag {
            mode: resolved.label().to_string(),
            flag: "--socket".to_string(),
        });
    }
    Ok(match resolved {
        Mode::Run => ParsedArgs::Run { socket },
        Mode::Shutdown => ParsedArgs::Shutdown { socket },
        Mode::Status => ParsedArgs::Status,
        Mode::Help => ParsedArgs::Help,
        Mode::Version => ParsedArgs::Version,
    })
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
    --status           Print a one-line health banner and exit
    --help, -h         Print this help and exit
    --version, -V      Print version and exit
    --shutdown         Connect to a running daemon and ask it to shut down
    --socket <path>    Override the default pipe / socket path

With no flags, runs the daemon event loop on the default socket path.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ParsedArgs, ArgError> {
        parse_args(args.iter().copied())
    }

    #[test]
    fn no_args_runs_daemon_with_default_socket() {
        assert_eq!(parse(&[]), Ok(ParsedArgs::Run { socket: None }));
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
    fn shutdown_flag_parsed() {
        assert_eq!(
            parse(&["--shutdown"]),
            Ok(ParsedArgs::Shutdown { socket: None })
        );
    }

    #[test]
    fn socket_overrides_default_on_run() {
        assert_eq!(
            parse(&["--socket", r"\\.\pipe\custom"]),
            Ok(ParsedArgs::Run {
                socket: Some(PathBuf::from(r"\\.\pipe\custom"))
            })
        );
    }

    #[test]
    fn shutdown_with_socket_pairs_correctly() {
        assert_eq!(
            parse(&["--shutdown", "--socket", "/tmp/x.sock"]),
            Ok(ParsedArgs::Shutdown {
                socket: Some(PathBuf::from("/tmp/x.sock"))
            })
        );
    }

    #[test]
    fn socket_before_shutdown_pairs_correctly() {
        assert_eq!(
            parse(&["--socket", "/tmp/x.sock", "--shutdown"]),
            Ok(ParsedArgs::Shutdown {
                socket: Some(PathBuf::from("/tmp/x.sock"))
            })
        );
    }

    #[test]
    fn socket_without_value_errors() {
        assert_eq!(
            parse(&["--socket"]),
            Err(ArgError::MissingValue("--socket".to_string()))
        );
    }

    #[test]
    fn socket_with_status_is_incompatible() {
        let err = parse(&["--status", "--socket", "/tmp/x"]).unwrap_err();
        assert!(
            matches!(err, ArgError::IncompatibleFlag { ref flag, .. } if flag == "--socket"),
            "{err:?}"
        );
    }

    #[test]
    fn two_mode_flags_conflict() {
        let err = parse(&["--status", "--shutdown"]).unwrap_err();
        assert!(matches!(err, ArgError::ConflictingModes(_, _)), "{err:?}");
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
    fn unknown_trailing_flag_after_mode_rejected() {
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
