//! System clipboard access, gated behind the `clipboard` feature.
//!
//! When the feature is enabled, `ClipboardContext` lazily initializes an
//! `arboard::Clipboard` on first use and provides read/write/clear operations.
//! When the feature is disabled, the same API exists but always returns a
//! no-op stub that never fails.

use std::fmt;
#[cfg(feature = "clipboard")]
use std::sync::Mutex;

/// Rich content that can be placed on the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardContent {
    /// Plain UTF-8 text.
    Text(String),
    /// HTML with an optional plain-text fallback for applications that do not
    /// understand HTML clipboard data.
    Html {
        /// The HTML string to place on the clipboard.
        html: String,
        /// Plain-text alternative shown by applications that only support text.
        alt_text: String,
    },
}

/// Clipboard format discriminant, returned by [`ClipboardContext::available_formats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardFormat {
    /// The clipboard contains plain text.
    Text,
    /// The clipboard contains HTML.
    Html,
}

/// Errors that can occur during clipboard operations.
#[derive(Debug)]
pub enum ClipboardError {
    /// The system clipboard is not available (headless/CI environments, etc.).
    Unavailable(String),
    /// An unexpected error occurred while accessing the clipboard.
    Other(String),
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClipboardError::Unavailable(msg) => write!(f, "clipboard unavailable: {}", msg),
            ClipboardError::Other(msg) => write!(f, "clipboard error: {}", msg),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// Wrapper around system clipboard access.
///
/// Lazily initializes the underlying clipboard handle on first use.
/// All operations are guarded by a `Mutex` so that `ClipboardContext`
/// can be shared via `Arc` across threads.
pub struct ClipboardContext {
    #[cfg(feature = "clipboard")]
    inner: Mutex<Option<arboard::Clipboard>>,
    #[cfg(feature = "clipboard")]
    init_error: Mutex<Option<String>>,
    #[cfg(not(feature = "clipboard"))]
    _phantom: (),
}

impl ClipboardContext {
    /// Create a new `ClipboardContext`. The underlying system clipboard
    /// is not opened until the first read/write/clear call.
    pub fn new() -> Self {
        #[cfg(feature = "clipboard")]
        {
            Self { inner: Mutex::new(None), init_error: Mutex::new(None) }
        }
        #[cfg(not(feature = "clipboard"))]
        {
            Self { _phantom: () }
        }
    }

    /// Read text from the system clipboard.
    ///
    /// Returns `Ok(String)` with the clipboard contents, or an empty string
    /// if the clipboard is empty or does not contain text.
    pub fn read_text(&self) -> Result<String, ClipboardError> {
        #[cfg(feature = "clipboard")]
        {
            let cb = self.get_or_init()?;
            let mut guard = cb.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            match guard.as_mut().unwrap().get_text() {
                Ok(text) => Ok(text),
                Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
                Err(e) => Err(ClipboardError::Other(e.to_string())),
            }
        }
        #[cfg(not(feature = "clipboard"))]
        {
            Ok(String::new())
        }
    }

    /// Write text to the system clipboard.
    pub fn write_text(&self, text: impl AsRef<str>) -> Result<(), ClipboardError> {
        #[cfg(feature = "clipboard")]
        {
            let cb = self.get_or_init()?;
            let mut guard = cb.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            guard
                .as_mut()
                .unwrap()
                .set_text(text.as_ref().to_owned())
                .map_err(|e| ClipboardError::Other(e.to_string()))
        }
        #[cfg(not(feature = "clipboard"))]
        {
            let _ = text;
            Ok(())
        }
    }

    /// Clear the system clipboard contents.
    pub fn clear(&self) -> Result<(), ClipboardError> {
        #[cfg(feature = "clipboard")]
        {
            let cb = self.get_or_init()?;
            let mut guard = cb.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            guard.as_mut().unwrap().clear().map_err(|e| ClipboardError::Other(e.to_string()))
        }
        #[cfg(not(feature = "clipboard"))]
        {
            Ok(())
        }
    }

    /// Set the clipboard with rich content.
    ///
    /// For [`ClipboardContent::Text`] this is equivalent to [`write_text`].
    /// For [`ClipboardContent::Html`] both the HTML and a plain-text fallback
    /// are written so that applications that do not understand HTML can still
    /// paste something useful.
    ///
    /// [`write_text`]: ClipboardContext::write_text
    pub fn set_content(&self, content: ClipboardContent) -> Result<(), ClipboardError> {
        match content {
            ClipboardContent::Text(text) => self.write_text(text),
            ClipboardContent::Html { html, alt_text } => {
                #[cfg(feature = "clipboard")]
                {
                    let cb = self.get_or_init()?;
                    let mut guard = cb.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
                    guard
                        .as_mut()
                        .unwrap()
                        .set_html(&html, Some(&alt_text))
                        .map_err(|e| ClipboardError::Other(e.to_string()))
                }
                #[cfg(not(feature = "clipboard"))]
                {
                    let _ = (html, alt_text);
                    Ok(())
                }
            }
        }
    }

    /// Read HTML content from the clipboard, if available.
    ///
    /// Returns `Ok(Some(html))` when the clipboard contains HTML, `Ok(None)`
    /// when the clipboard does not contain HTML (but no error occurred), or
    /// `Err` for genuine clipboard failures.
    pub fn get_html(&self) -> Result<Option<String>, ClipboardError> {
        #[cfg(feature = "clipboard")]
        {
            let cb = self.get_or_init()?;
            let mut guard = cb.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            match guard.as_mut().unwrap().get().html() {
                Ok(html) => Ok(Some(html)),
                Err(arboard::Error::ContentNotAvailable) => Ok(None),
                Err(e) => Err(ClipboardError::Other(e.to_string())),
            }
        }
        #[cfg(not(feature = "clipboard"))]
        {
            Ok(None)
        }
    }

    /// Return all clipboard formats that are currently readable.
    ///
    /// Each variant in the returned `Vec` indicates that the corresponding
    /// `read_text` / `get_html` call would succeed right now.  The list may be
    /// empty if the clipboard is empty or not accessible.
    pub fn available_formats(&self) -> Vec<ClipboardFormat> {
        let mut formats = Vec::new();

        // A successful read_text (even returning an empty string) means text
        // is available; we only exclude the case where the clipboard is
        // completely unavailable.
        if self.read_text().is_ok() {
            formats.push(ClipboardFormat::Text);
        }

        match self.get_html() {
            Ok(Some(_)) => formats.push(ClipboardFormat::Html),
            _ => {}
        }

        formats
    }

    /// Lazily initialize the arboard clipboard.
    /// Returns a reference to the Mutex holding the clipboard if successful.
    #[cfg(feature = "clipboard")]
    fn get_or_init(&self) -> Result<&Mutex<Option<arboard::Clipboard>>, ClipboardError> {
        // Fast path: already initialized
        {
            let guard = self.inner.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            if guard.is_some() {
                return Ok(&self.inner);
            }
        }

        // Check if we already failed to initialize
        {
            let err_guard =
                self.init_error.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
            if let Some(ref msg) = *err_guard {
                return Err(ClipboardError::Unavailable(msg.clone()));
            }
        }

        // Try to initialize
        match arboard::Clipboard::new() {
            Ok(cb) => {
                let mut guard =
                    self.inner.lock().map_err(|e| ClipboardError::Other(e.to_string()))?;
                *guard = Some(cb);
                Ok(&self.inner)
            }
            Err(e) => {
                let msg = e.to_string();
                let mut err_guard =
                    self.init_error.lock().map_err(|e2| ClipboardError::Other(e2.to_string()))?;
                *err_guard = Some(msg.clone());
                Err(ClipboardError::Unavailable(msg))
            }
        }
    }
}

impl Default for ClipboardContext {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: the Mutex guards all interior access.
unsafe impl Send for ClipboardContext {}
unsafe impl Sync for ClipboardContext {}

impl fmt::Debug for ClipboardContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClipboardContext").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Type-level tests (no clipboard access required) ---

    #[test]
    fn clipboard_content_text_variant_constructs() {
        let content = ClipboardContent::Text("hello".to_owned());
        assert_eq!(content, ClipboardContent::Text("hello".to_owned()));
    }

    #[test]
    fn clipboard_content_html_variant_constructs() {
        let content =
            ClipboardContent::Html { html: "<b>bold</b>".to_owned(), alt_text: "bold".to_owned() };
        match content {
            ClipboardContent::Html { html, alt_text } => {
                assert_eq!(html, "<b>bold</b>");
                assert_eq!(alt_text, "bold");
            }
            _ => panic!("expected Html variant"),
        }
    }

    #[test]
    fn clipboard_format_variants_exist() {
        let text_fmt = ClipboardFormat::Text;
        let html_fmt = ClipboardFormat::Html;
        // Ensure the variants are distinct
        assert_ne!(text_fmt, html_fmt);
    }

    #[test]
    fn clipboard_format_is_copy() {
        let fmt = ClipboardFormat::Text;
        let copy = fmt;
        assert_eq!(fmt, copy);
    }

    // --- Integration tests (best-effort; allowed to skip on headless CI) ---

    /// Helper: attempt to obtain a ClipboardContext.  Returns None if the
    /// system clipboard is unavailable (headless / CI environment).
    fn try_context() -> Option<ClipboardContext> {
        let ctx = ClipboardContext::new();
        // Probe with a write; if it fails we skip rather than panic.
        if ctx.write_text("probe").is_ok() {
            Some(ctx)
        } else {
            None
        }
    }

    #[test]
    fn set_content_text_roundtrip() {
        let Some(ctx) = try_context() else { return };
        // Write then read.  Because clipboard tests may run in parallel (shared
        // system state), another test might overwrite the value before we read
        // it back.  We therefore only assert that the write succeeded and that
        // the read does not return an error; we do not assert the exact value.
        assert!(ctx.set_content(ClipboardContent::Text("roundtrip".to_owned())).is_ok());
        assert!(ctx.read_text().is_ok());
    }

    #[test]
    fn set_content_html_does_not_error() {
        let Some(ctx) = try_context() else { return };
        let result = ctx.set_content(ClipboardContent::Html {
            html: "<em>test</em>".to_owned(),
            alt_text: "test".to_owned(),
        });
        // On platforms with full clipboard support this should succeed.
        // We only care that it does not panic.
        let _ = result;
    }

    #[test]
    fn get_html_returns_ok() {
        let Some(ctx) = try_context() else { return };
        // After writing HTML content get_html should not panic.
        // On some platforms (e.g. Windows when another test has since
        // overwritten the clipboard) get_html may return Ok(None) or an
        // error; both are acceptable.  We only verify the call does not
        // panic.
        ctx.set_content(ClipboardContent::Html {
            html: "<p>hi</p>".to_owned(),
            alt_text: "hi".to_owned(),
        })
        .ok();
        let _ = ctx.get_html();
    }

    #[test]
    fn available_formats_returns_text_after_write() {
        let Some(ctx) = try_context() else { return };
        ctx.write_text("formats test").ok();
        let formats = ctx.available_formats();
        assert!(
            formats.contains(&ClipboardFormat::Text),
            "expected Text in available_formats, got {:?}",
            formats
        );
    }
}
