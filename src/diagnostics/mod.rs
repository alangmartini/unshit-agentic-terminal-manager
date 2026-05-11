pub mod config;
pub mod server;

#[cfg(windows)]
mod transport;

#[cfg(not(windows))]
mod transport;

pub use config::DiagnosticConfig;
