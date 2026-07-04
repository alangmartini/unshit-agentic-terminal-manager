// On Windows, build the daemon as a "windows" (GUI) subsystem binary in release
// so that when the terminal-manager UI auto-spawns it — or a user double-clicks
// unshit-ptyd.exe — no console window pops up alongside the app. The daemon owns
// PTYs and runs headless; it has no interactive stdout of its own. A console
// subsystem binary would otherwise get its own console window on launch, which is
// the stray "terminal that keeps showing" next to the app. Debug builds stay on
// the console subsystem so `cargo run -p unshit-ptyd` still surfaces logs during
// development. The CLI subcommands (--status/--version/--help/--shutdown) still
// print when run from a terminal via `attach_parent_console` below.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;
use std::process::ExitCode;

use unshit_ptyd::{
    client, daemon, parse_args, protocol::Response, status_line, transport, ParsedArgs,
    DAEMON_VERSION, HELP_TEXT,
};

/// Reattach stdio to the parent terminal on Windows release builds.
///
/// Release is a "windows" subsystem binary (see the crate attribute above), so
/// it owns no console. When a human runs `unshit-ptyd --status` (or --version /
/// --help / --shutdown) from an existing terminal, this reconnects stdout/stderr
/// to that console so the output still appears. It is a no-op when there is no
/// parent console — the UI auto-spawn (detached) and Explorer double-click cases
/// — which is exactly the no-extra-window behavior we want. Debug builds keep
/// their own console and skip this entirely.
fn attach_parent_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        // ATTACH_PARENT_PROCESS == (DWORD)-1.
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        extern "system" {
            fn AttachConsole(dw_process_id: u32) -> i32;
        }
        // SAFETY: plain kernel32 call. Returns 0 (ignored) when the process has
        // no parent console, leaving the daemon window-free as intended.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> ExitCode {
    attach_parent_console();
    match parse_args(std::env::args().skip(1)) {
        Ok(ParsedArgs::Run { socket }) => {
            let path = socket.unwrap_or_else(transport::default_socket_path);
            eprintln!("unshit-ptyd: listening on {}", path.display());
            run_daemon(&path)
        }
        Ok(ParsedArgs::Shutdown { socket }) => {
            let path = socket.unwrap_or_else(transport::default_socket_path);
            run_shutdown_client(&path)
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

fn run_daemon(path: &Path) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("unshit-ptyd: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };
    match rt.block_on(daemon::run(path)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("unshit-ptyd: daemon error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_shutdown_client(path: &Path) -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("unshit-ptyd: failed to build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };
    rt.block_on(shutdown_over_ipc(path))
}

async fn shutdown_over_ipc(path: &Path) -> ExitCode {
    let mut client = match client::Client::connect(path).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "unshit-ptyd: could not connect to daemon at {}: {}",
                path.display(),
                e
            );
            return ExitCode::from(1);
        }
    };
    let resp = match client.shutdown().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("unshit-ptyd: shutdown request failed: {e}");
            return ExitCode::from(1);
        }
    };
    match resp {
        Response::ShutdownAck { ok: true, .. } => {
            println!("unshit-ptyd: shutdown ok");
            ExitCode::SUCCESS
        }
        Response::ShutdownAck {
            ok: false, reason, ..
        } => {
            eprintln!(
                "unshit-ptyd: daemon refused shutdown: {}",
                reason.as_deref().unwrap_or("no reason given")
            );
            ExitCode::from(1)
        }
        other => {
            eprintln!("unshit-ptyd: unexpected response: {other:?}");
            ExitCode::from(1)
        }
    }
}
