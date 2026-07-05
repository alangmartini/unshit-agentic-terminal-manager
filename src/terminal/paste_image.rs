//! Clipboard-image paste support for terminal panes.
//!
//! Windows Terminal parity: when `terminal.paste` finds no text on the
//! clipboard but does find a bitmap (ShareX Ctrl+Print, Win+Shift+S,
//! browser "Copy image"), the image is written to a PNG under a stable
//! temp dir and the file's path is pasted into the PTY instead. Agent
//! CLIs (Claude Code, Codex) detect image paths in the prompt exactly
//! like a drag-and-dropped file, and a plain shell just receives a
//! path string.
//!
//! Files are content-addressed (same FNV-1a hash the Quick Prompt
//! image pipeline uses) so pasting the same screenshot twice reuses
//! one file instead of littering the temp dir.

use std::io;
use std::path::{Path, PathBuf};

use crate::quick_prompt::images::content_hash;

/// Stable directory pasted clipboard PNGs live in. Deliberately NOT
/// per-session: content-hashed names make repeat pastes idempotent,
/// and the OS temp cleaner reclaims the dir eventually.
pub fn paste_dir() -> PathBuf {
    std::env::temp_dir().join("godly-paste")
}

/// Encode raw RGBA clipboard pixels as a PNG under [`paste_dir`] and
/// return the file's absolute path. Re-pasting identical pixels hits
/// the already-encoded file and skips the PNG encode entirely.
pub fn save_clipboard_png(width: usize, height: usize, bytes: Vec<u8>) -> io::Result<PathBuf> {
    if width == 0 || height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard image had zero width or height",
        ));
    }
    if bytes.len() != width.saturating_mul(height).saturating_mul(4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "clipboard image bytes ({}) do not match width*height*4 ({})",
                bytes.len(),
                width * height * 4
            ),
        ));
    }
    let dir = paste_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("clipboard-{}.png", content_hash(&bytes)));
    if path.exists() {
        return Ok(path);
    }
    let buf: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width as u32, height as u32, bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "ImageBuffer::from_raw rejected the clipboard pixels",
            )
        })?;
    buf.save(&path).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(path)
}

/// Render `path` as the text to paste into the PTY. Paths containing
/// whitespace are wrapped in double quotes so shells and agent-CLI
/// path detection treat them as one token (`%TEMP%` usually sits under
/// `C:\Users\<name with spaces>\...` on Windows).
pub fn pasteable_path_text(path: &Path) -> String {
    let s = path.display().to_string();
    if s.chars().any(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            bytes.extend_from_slice(&color);
        }
        bytes
    }

    #[test]
    fn save_clipboard_png_writes_decodable_png() {
        let path = save_clipboard_png(3, 2, solid_rgba(3, 2, [200, 10, 30, 255])).expect("save");
        assert!(path.exists());
        assert_eq!(path.parent().unwrap(), paste_dir());
        let img = image::open(&path).expect("written file must decode as an image");
        assert_eq!((img.width(), img.height()), (3, 2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_clipboard_png_is_idempotent_for_same_pixels() {
        let bytes = solid_rgba(2, 2, [1, 2, 3, 255]);
        let a = save_clipboard_png(2, 2, bytes.clone()).expect("save a");
        let b = save_clipboard_png(2, 2, bytes).expect("save b");
        assert_eq!(a, b, "same pixels must map to the same file");
        let _ = std::fs::remove_file(&a);
    }

    #[test]
    fn save_clipboard_png_rejects_zero_dimensions() {
        let err = save_clipboard_png(0, 4, vec![]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn save_clipboard_png_rejects_byte_count_mismatch() {
        let err = save_clipboard_png(2, 2, vec![0u8; 3]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn pasteable_path_text_quotes_paths_with_whitespace() {
        assert_eq!(
            pasteable_path_text(Path::new(r"C:\Users\Alan Beelink\shot.png")),
            "\"C:\\Users\\Alan Beelink\\shot.png\""
        );
    }

    #[test]
    fn pasteable_path_text_leaves_plain_paths_unquoted() {
        assert_eq!(
            pasteable_path_text(Path::new(r"C:\tmp\shot.png")),
            r"C:\tmp\shot.png"
        );
    }
}
