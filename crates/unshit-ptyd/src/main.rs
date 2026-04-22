use std::process::ExitCode;

use unshit_ptyd::{parse_args, status_line, ParsedArgs, DAEMON_VERSION, HELP_TEXT};

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
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
            // EX_USAGE, per BSD sysexits.h.
            ExitCode::from(2)
        }
    }
}
