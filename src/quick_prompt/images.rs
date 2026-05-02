//! Clipboard image capture and lifecycle for Quick Prompt.
//!
//! Each open of the overlay starts a fresh per-session temp dir under
//! `temp_dir().join("godly-qp").join(<8-hex>)`. Pasted images are
//! hashed by raw RGBA content so the same screenshot pasted twice
//! shares one chip. Full-resolution PNGs and small thumbnail PNGs
//! both live in the session dir until either:
//!   * submit moves the full PNGs into `<target>/.quick-prompt/<hash>.png`
//!     and appends `@.quick-prompt/<hash>.png` references to the prompt;
//!   * cancel removes the session dir wholesale.

use std::io;
use std::path::{Path, PathBuf};

use unshit::app::{ClipboardContent, ClipboardContext, ClipboardError};

/// One pasted image plus the on-disk paths the UI needs to render it
/// and the submit path needs to move it into the worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickPromptImage {
    /// 16-hex content hash. Stable across sessions for the same
    /// pixel data so dedup works.
    pub hash: String,
    /// Full-resolution PNG, lives in the session temp dir until
    /// submit (moved into the worktree) or cancel (deleted).
    pub temp_path: PathBuf,
    /// Small PNG (max 64x64) used by the chip strip. Lives next to
    /// `temp_path` and is removed alongside it.
    pub thumb_path: PathBuf,
    /// Original pixel dimensions.
    pub width: u32,
    /// Original pixel dimensions.
    pub height: u32,
}

/// Largest dimension (in pixels) the chip thumbnail occupies on disk.
/// The renderer rescales to fit the chip's CSS box, so we deliberately
/// pick something a touch larger than the rendered size to keep the
/// chip crisp on HiDPI displays.
const THUMBNAIL_MAX_EDGE: u32 = 96;

/// Path to the per-overlay-session temp dir.
pub fn session_dir(session_hex: &str) -> PathBuf {
    std::env::temp_dir().join("godly-qp").join(session_hex)
}

/// Read the clipboard. Returns `Ok(None)` when there is no image on
/// the clipboard (text only, or empty) and `Err` only for genuine
/// clipboard failures. Images are written to disk under
/// `session_dir(session_hex)`.
pub fn capture_clipboard_image(
    clipboard: &ClipboardContext,
    session_hex: &str,
) -> Result<Option<QuickPromptImage>, ClipboardError> {
    let Some(content) = clipboard.read_image()? else {
        return Ok(None);
    };
    let ClipboardContent::Image {
        width,
        height,
        bytes,
    } = content
    else {
        // read_image is documented to return only Image, but match
        // exhaustively defensively.
        return Ok(None);
    };

    save_image_to_session(session_hex, width, height, bytes)
        .map(Some)
        .map_err(|e| ClipboardError::Other(format!("failed to write pasted image to disk: {e}")))
}

/// Encode raw RGBA pixels as PNG (full-res) and a thumbnail PNG, save
/// both under `session_dir(session_hex)`, and return the resulting
/// `QuickPromptImage`. Public for tests.
pub fn save_image_to_session(
    session_hex: &str,
    width: usize,
    height: usize,
    bytes: Vec<u8>,
) -> io::Result<QuickPromptImage> {
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
    let dir = session_dir(session_hex);
    std::fs::create_dir_all(&dir)?;

    let hash = content_hash(&bytes);
    let temp_path = dir.join(format!("{hash}.png"));
    let thumb_path = dir.join(format!("{hash}.thumb.png"));

    let buf: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_raw(width as u32, height as u32, bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "ImageBuffer::from_raw rejected the clipboard pixels",
            )
        })?;
    buf.save(&temp_path)
        .map_err(|e| io::Error::other(e.to_string()))?;

    let thumb = make_thumbnail(&buf);
    thumb
        .save(&thumb_path)
        .map_err(|e| io::Error::other(e.to_string()))?;

    Ok(QuickPromptImage {
        hash,
        temp_path,
        thumb_path,
        width: width as u32,
        height: height as u32,
    })
}

/// Resize so the longest edge is `THUMBNAIL_MAX_EDGE`, preserving
/// aspect ratio. Smaller images are returned unchanged.
fn make_thumbnail(
    src: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
) -> image::ImageBuffer<image::Rgba<u8>, Vec<u8>> {
    let (w, h) = src.dimensions();
    let max = w.max(h);
    if max <= THUMBNAIL_MAX_EDGE {
        return src.clone();
    }
    let scale = THUMBNAIL_MAX_EDGE as f32 / max as f32;
    let new_w = (w as f32 * scale).round().max(1.0) as u32;
    let new_h = (h as f32 * scale).round().max(1.0) as u32;
    image::imageops::thumbnail(src, new_w, new_h)
}

/// 16-char lowercase hex. Uses 64-bit FNV-1a so the binary stays free
/// of an extra cryptographic hash dep; collisions at this width are
/// negligible for the per-overlay image counts we expect (single
/// digits) and the spec's contract is "stable filename per content"
/// rather than a specific algorithm.
fn content_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Move every image's `temp_path` into `<target>/.quick-prompt/<hash>.png`
/// and return the per-image relative path strings (`.quick-prompt/<hash>.png`)
/// so callers can inline them into the agent prompt. Thumbnails are
/// NOT moved; they are left in the session dir for `cleanup_session`
/// to remove together with anything else that did not get moved.
pub fn move_into_target(
    images: &[QuickPromptImage],
    target_root: &Path,
) -> io::Result<Vec<String>> {
    let dest_dir = target_root.join(".quick-prompt");
    std::fs::create_dir_all(&dest_dir)?;
    let mut refs = Vec::with_capacity(images.len());
    for img in images {
        let dest = dest_dir.join(format!("{}.png", img.hash));
        // Try rename first (single-volume fast path); fall back to
        // copy+delete when the temp dir is on a different drive than
        // the worktree (common on Windows: %TEMP% vs %APPDATA%).
        if std::fs::rename(&img.temp_path, &dest).is_err() {
            std::fs::copy(&img.temp_path, &dest)?;
            let _ = std::fs::remove_file(&img.temp_path);
        }
        refs.push(format!(".quick-prompt/{}.png", img.hash));
    }
    Ok(refs)
}

/// Append an `Attached images:` block to `prompt` with one
/// `@.quick-prompt/<hash>.png` reference per line. Empty `refs`
/// returns `prompt` unchanged so the agent does not see a stray
/// header on prompts with no attachments.
pub fn append_image_references(prompt: &str, refs: &[String]) -> String {
    if refs.is_empty() {
        return prompt.to_string();
    }
    let mut out =
        String::with_capacity(prompt.len() + refs.iter().map(|r| r.len() + 2).sum::<usize>() + 32);
    out.push_str(prompt);
    out.push_str("\n\nAttached images:\n");
    for r in refs {
        out.push('@');
        out.push_str(r);
        out.push('\n');
    }
    out
}

/// Remove the per-session temp dir and everything inside it. Called
/// by `quick_prompt.close` and by `quick_prompt.submit` on success
/// (after `move_into_target` has taken what it needs).
pub fn cleanup_session(session_hex: &str) {
    let dir = session_dir(session_hex);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_session_hex(tag: &str) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("test-{}-{}-{}", tag, std::process::id(), n)
    }

    fn solid_color_rgba(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            bytes.extend_from_slice(&color);
        }
        bytes
    }

    // --- content_hash ---------------------------------------------------

    #[test]
    fn content_hash_is_deterministic() {
        let bytes = vec![1u8, 2, 3, 4, 5];
        assert_eq!(content_hash(&bytes), content_hash(&bytes));
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let a = vec![1u8, 2, 3, 4];
        let b = vec![1u8, 2, 3, 5];
        assert_ne!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn content_hash_is_sixteen_hex_chars() {
        let h = content_hash(b"hello");
        assert_eq!(h.len(), 16);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    // --- save_image_to_session ------------------------------------------

    #[test]
    fn save_image_to_session_writes_full_and_thumb_pngs() {
        let hex = unique_session_hex("save");
        let img = save_image_to_session(&hex, 4, 4, solid_color_rgba(4, 4, [255, 0, 0, 255]))
            .expect("save");
        assert!(img.temp_path.exists(), "full-res PNG should exist");
        assert!(img.thumb_path.exists(), "thumb PNG should exist");
        assert_eq!(img.width, 4);
        assert_eq!(img.height, 4);
        cleanup_session(&hex);
    }

    #[test]
    fn save_image_to_session_dedups_by_hash() {
        let hex = unique_session_hex("dedup");
        let bytes = solid_color_rgba(2, 2, [10, 20, 30, 255]);
        let a = save_image_to_session(&hex, 2, 2, bytes.clone()).expect("save a");
        let b = save_image_to_session(&hex, 2, 2, bytes).expect("save b");
        // Same content, same hash, same path.
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.temp_path, b.temp_path);
        cleanup_session(&hex);
    }

    #[test]
    fn save_image_to_session_rejects_zero_dimensions() {
        let hex = unique_session_hex("zero");
        let err = save_image_to_session(&hex, 0, 0, vec![]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        cleanup_session(&hex);
    }

    #[test]
    fn save_image_to_session_rejects_byte_count_mismatch() {
        let hex = unique_session_hex("bad-bytes");
        let err = save_image_to_session(&hex, 2, 2, vec![0u8; 4]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        cleanup_session(&hex);
    }

    // --- thumbnail ------------------------------------------------------

    #[test]
    fn thumbnail_max_edge_caps_at_constant() {
        let hex = unique_session_hex("thumb-cap");
        let big = save_image_to_session(&hex, 200, 100, solid_color_rgba(200, 100, [0, 0, 0, 255]))
            .expect("save");
        let thumb = image::open(&big.thumb_path).expect("open thumb");
        let (w, h) = (thumb.width(), thumb.height());
        assert!(
            w.max(h) <= THUMBNAIL_MAX_EDGE,
            "thumb {}x{} should fit within {}",
            w,
            h,
            THUMBNAIL_MAX_EDGE
        );
        cleanup_session(&hex);
    }

    #[test]
    fn thumbnail_preserves_small_images_unchanged() {
        let hex = unique_session_hex("thumb-small");
        let small = save_image_to_session(&hex, 16, 16, solid_color_rgba(16, 16, [0, 255, 0, 255]))
            .expect("save");
        let thumb = image::open(&small.thumb_path).expect("open thumb");
        assert_eq!(thumb.width(), 16);
        assert_eq!(thumb.height(), 16);
        cleanup_session(&hex);
    }

    // --- move_into_target -----------------------------------------------

    #[test]
    fn move_into_target_relocates_temp_files() {
        let hex = unique_session_hex("move");
        let bytes = solid_color_rgba(2, 2, [0, 0, 255, 255]);
        let img = save_image_to_session(&hex, 2, 2, bytes).expect("save");
        let target = std::env::temp_dir().join(format!("godly-qp-target-{}", hex));
        std::fs::create_dir_all(&target).unwrap();

        let refs = move_into_target(std::slice::from_ref(&img), &target).expect("move");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], format!(".quick-prompt/{}.png", img.hash));
        let dest = target
            .join(".quick-prompt")
            .join(format!("{}.png", img.hash));
        assert!(dest.exists(), "moved file should exist at {:?}", dest);
        assert!(!img.temp_path.exists(), "temp file should be gone");

        let _ = std::fs::remove_dir_all(&target);
        cleanup_session(&hex);
    }

    // --- append_image_references ----------------------------------------

    #[test]
    fn append_image_references_returns_unchanged_when_empty() {
        assert_eq!(append_image_references("hello", &[]), "hello".to_string());
    }

    #[test]
    fn append_image_references_emits_block_with_one_line_per_image() {
        let refs = vec![
            ".quick-prompt/abc.png".to_string(),
            ".quick-prompt/def.png".to_string(),
        ];
        let out = append_image_references("look at these", &refs);
        assert!(out.contains("look at these"));
        assert!(out.contains("\n\nAttached images:\n"));
        assert!(out.contains("@.quick-prompt/abc.png\n"));
        assert!(out.contains("@.quick-prompt/def.png\n"));
    }

    // --- cleanup_session ------------------------------------------------

    #[test]
    fn cleanup_session_removes_dir() {
        let hex = unique_session_hex("cleanup");
        save_image_to_session(&hex, 1, 1, solid_color_rgba(1, 1, [1, 2, 3, 4])).expect("save");
        assert!(session_dir(&hex).exists());
        cleanup_session(&hex);
        assert!(!session_dir(&hex).exists());
    }

    #[test]
    fn cleanup_session_is_no_op_when_dir_absent() {
        let hex = unique_session_hex("absent");
        cleanup_session(&hex);
        cleanup_session(&hex); // second call must not panic
    }
}
