use std::process::ExitCode;

use unshit_ptyd::{parse_args, status_line, ArgError, ParsedArgs, DAEMON_VERSION, HELP_TEXT};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(args.iter().map(String::as_str)) {
        Ok(ParsedArgs::Run) => {
            eprintln!("unshit-ptyd: daemon loop not implemented in slice 1");
            eprintln!("see SPEC.md section 11 for the rollout plan");
            ExitCode::from(2)
        }
        Ok(ParsedArgs::Status) => {
            println!("{}", status_line());
            ExitCode::SUCCESS
        }
        Ok(ParsedArgs::Help) => {
            print!("{HELP_TEXT}");
            ExitCode::SUCCESS
        }
        Ok(ParsedArgs::Version) => {
            println!("unshit-ptyd {DAEMON_VERSION}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("unshit-ptyd: {err}");
            eprintln!("try --help for usage");
            usage_exit_code(&err)
        }
    }
}

/// Map an arg error onto a POSIX-style exit code.
///
/// Exit code 2 is the convention for "usage error" (BSD `sysexits.h`
/// calls it `EX_USAGE`). Kept as a pure function for test coverage.
fn usage_exit_code(_err: &ArgError) -> ExitCode {
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_exit_code_for_unknown_flag_is_two() {
        let code = usage_exit_code(&ArgError::UnknownFlag("--x".to_string()));
        // ExitCode does not implement PartialEq, so compare via Debug.
        // EX_USAGE = 2.
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(2)));
    }

    #[test]
    fn usage_exit_code_for_positional_is_two() {
        let code = usage_exit_code(&ArgError::UnexpectedPositional("x".to_string()));
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(2)));
    }
}
