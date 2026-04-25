use std::path::Path;
use std::process::ExitCode;

use unshit_ptyd::{
    client, daemon, parse_args, protocol::Response, status_line, transport, ParsedArgs,
    DAEMON_VERSION, HELP_TEXT,
};

fn main() -> ExitCode {
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
