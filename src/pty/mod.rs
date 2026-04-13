//! PTY process manager for a terminal emulator.
//!
//! Manages pseudo-terminal sessions mapped to UI pane IDs. Each pane gets its
//! own shell process with independent stdin/stdout and size tracking. Built on
//! top of `portable_pty` (0.8) for cross-platform support (Windows, macOS,
//! Linux).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};

/// A single PTY session: the child process, a writer for stdin, the master PTY
/// handle (needed for resize), and the current terminal size.
pub struct PtyPair {
    child: Box<dyn Child + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    size: PtySize,
}

/// Manages PTY sessions keyed by pane ID.
pub struct PtyManager {
    pairs: HashMap<u32, PtyPair>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyManager {
    /// Create an empty manager with no active sessions.
    pub fn new() -> Self {
        Self {
            pairs: HashMap::new(),
        }
    }

    /// Spawn a new shell process for the given pane.
    ///
    /// Returns the reader (stdout) half so the caller can consume output in a
    /// background thread. The writer half is stored internally and accessible
    /// via [`write`](Self::write).
    ///
    /// The shell is chosen from the `SHELL` environment variable. If that is
    /// unset, it falls back to `bash` on Unix or `powershell.exe` on Windows.
    /// The working directory is set to the user's home directory.
    pub fn spawn(
        &mut self,
        pane_id: u32,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Box<dyn Read + Send>> {
        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pty_pair = pty_system
            .openpty(size)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let shell = default_shell();

        let mut cmd = CommandBuilder::new(&shell);
        if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }

        let child = pty_pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let reader = pty_pair
            .master
            .try_clone_reader()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let writer = pty_pair
            .master
            .take_writer()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        self.pairs.insert(
            pane_id,
            PtyPair {
                child,
                writer,
                master: pty_pair.master,
                size,
            },
        );

        Ok(reader)
    }

    /// Write raw bytes to the PTY stdin for the given pane.
    ///
    /// Returns an error if the pane does not exist or the write fails.
    pub fn write(&mut self, pane_id: u32, data: &[u8]) -> std::io::Result<()> {
        let pair = self.pairs.get_mut(&pane_id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no PTY for pane {pane_id}"),
            )
        })?;
        pair.writer.write_all(data)?;
        pair.writer.flush()
    }

    /// Resize the PTY for the given pane to new column/row dimensions.
    ///
    /// This is a best-effort operation: if the pane does not exist or the
    /// resize call fails, it is silently ignored. This keeps the caller's
    /// resize logic simple.
    pub fn resize(&mut self, pane_id: u32, cols: u16, rows: u16) {
        if let Some(pair) = self.pairs.get_mut(&pane_id) {
            let new_size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            if pair.master.resize(new_size).is_ok() {
                pair.size = new_size;
            }
        }
    }

    /// Kill the child process and remove the PTY entry for the given pane.
    ///
    /// Silently ignored if the pane does not exist.
    pub fn destroy(&mut self, pane_id: u32) {
        if let Some(mut pair) = self.pairs.remove(&pane_id) {
            let _ = pair.child.kill();
        }
    }

    /// Kill all child processes and remove every PTY entry.
    pub fn destroy_all(&mut self) {
        let ids: Vec<u32> = self.pairs.keys().copied().collect();
        for id in ids {
            self.destroy(id);
        }
    }

    /// Check whether a PTY session exists for the given pane.
    pub fn has(&self, pane_id: u32) -> bool {
        self.pairs.contains_key(&pane_id)
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        self.destroy_all();
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Detect the default shell for the current platform.
///
/// Reads the `SHELL` environment variable first. If unset, falls back to
/// `bash` on Unix-like systems and `powershell.exe` on Windows.
fn default_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        return shell;
    }

    if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        "bash".to_string()
    }
}
