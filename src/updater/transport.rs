//! HTTP(S) access for the updater, plus a `file://` transport for tests.
//!
//! One `ureq` agent with the platform TLS stack (schannel on Windows, so the
//! system root store and any corporate interception certificates apply), a
//! fixed user agent (GitHub rejects anonymous requests without one) and
//! explicit timeouts: connect and response-header timeouts are short, the
//! body timeout is long enough for an installer over a slow link, and there
//! is no global deadline that could cut a legitimate download.
//!
//! `file://` URLs are only honoured when the caller opts in, which the
//! updater does exactly when `TM_UPDATE_FEED_URL` replaced the GitHub feed.
//! That keeps screenshot and e2e runs hermetic (a fake feed on disk) without
//! letting a production feed redirect an install to a local path.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

pub const USER_AGENT: &str = concat!(
    "terminal-manager/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/alangmartini/unshit-agentic-terminal-manager)"
);

#[derive(Debug)]
pub enum TransportError {
    /// The server answered with a non-success status.
    Http { status: u16 },
    /// DNS, TLS, connect or protocol failure before a status arrived.
    Network(String),
    /// Reading the body (or a `file://` source) failed.
    Io(std::io::Error),
    /// Not `http://`, `https://` or an allowed `file://`.
    UnsupportedScheme,
}

impl TransportError {
    /// Stable, low-cardinality label for telemetry.
    pub fn kind(&self) -> &'static str {
        match self {
            TransportError::Http { .. } => "http",
            TransportError::Network(_) => "network",
            TransportError::Io(_) => "io",
            TransportError::UnsupportedScheme => "unsupported_scheme",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            TransportError::Http { status } => Some(*status),
            _ => None,
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Http { status: 403 } | TransportError::Http { status: 429 } => {
                write!(
                    f,
                    "GitHub rate limit reached (HTTP {})",
                    self.http_status().unwrap()
                )
            }
            TransportError::Http { status } => write!(f, "server answered HTTP {status}"),
            TransportError::Network(message) => write!(f, "network error: {message}"),
            TransportError::Io(error) => write!(f, "read error: {error}"),
            TransportError::UnsupportedScheme => write!(f, "unsupported URL scheme"),
        }
    }
}

impl std::error::Error for TransportError {}

pub struct Transport {
    agent: ureq::Agent,
    allow_file: bool,
}

impl Transport {
    pub fn new(allow_file: bool) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
            .timeout_global(None)
            .max_redirects(5)
            .http_status_as_error(true)
            .tls_config(tls)
            .build();
        Self {
            agent: config.into(),
            allow_file,
        }
    }

    pub fn allows_file(&self) -> bool {
        self.allow_file
    }

    /// Open `url` for streaming. The reader stops after `limit` bytes so a
    /// misbehaving server cannot fill the disk; callers detect truncation by
    /// comparing against the expected size.
    pub fn open(
        &self,
        url: &str,
        accept: Option<&str>,
        limit: u64,
    ) -> Result<Box<dyn Read>, TransportError> {
        if let Some(path) = file_url_path(url) {
            if !self.allow_file {
                return Err(TransportError::UnsupportedScheme);
            }
            let file = std::fs::File::open(path).map_err(TransportError::Io)?;
            return Ok(Box::new(file.take(limit)));
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(TransportError::UnsupportedScheme);
        }
        let mut request = self.agent.get(url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        let response = request.call().map_err(map_ureq_error)?;
        let reader = response
            .into_body()
            .into_with_config()
            .limit(limit)
            .reader();
        Ok(Box::new(reader))
    }

    /// Fetch a small text document (the release feed).
    pub fn fetch_text(
        &self,
        url: &str,
        accept: Option<&str>,
        limit: u64,
    ) -> Result<String, TransportError> {
        let mut reader = self.open(url, accept, limit)?;
        let mut body = String::new();
        reader
            .read_to_string(&mut body)
            .map_err(TransportError::Io)?;
        Ok(body)
    }
}

fn map_ureq_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::StatusCode(status) => TransportError::Http { status },
        ureq::Error::Io(io) => TransportError::Io(io),
        other => TransportError::Network(other.to_string()),
    }
}

/// `file:///C:/dir/feed.json` → `C:\dir\feed.json`; `file:///tmp/x` → `/tmp/x`.
/// Percent-escapes are decoded. Returns `None` for any other scheme.
pub fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    // `file:///C:/x` carries a leading slash before the drive letter.
    let bytes = decoded.as_bytes();
    let path = if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        &decoded[1..]
    } else {
        decoded.as_str()
    };
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(
        path.replace('/', std::path::MAIN_SEPARATOR_STR),
    ))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_maps_to_local_path() {
        let path = file_url_path("file:///C:/Temp/tm%20feed/latest.json").unwrap();
        let text = path.to_string_lossy().replace('\\', "/");
        assert_eq!(text, "C:/Temp/tm feed/latest.json");
        assert!(file_url_path("https://example.invalid/x").is_none());
        assert!(file_url_path("file://").is_none());
    }

    #[test]
    fn file_transport_is_opt_in() {
        let dir = std::env::temp_dir().join(format!("tm-transport-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("feed.json");
        std::fs::write(&file, "{\"ok\":true}").unwrap();
        let url = format!("file:///{}", file.to_string_lossy().replace('\\', "/"));

        let closed = Transport::new(false);
        assert!(matches!(
            closed.open(&url, None, 1024),
            Err(TransportError::UnsupportedScheme)
        ));

        let open = Transport::new(true);
        assert_eq!(open.fetch_text(&url, None, 1024).unwrap(), "{\"ok\":true}");
        // The limit truncates instead of erroring; callers compare sizes.
        assert_eq!(open.fetch_text(&url, None, 4).unwrap(), "{\"ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_schemes_are_rejected() {
        let transport = Transport::new(true);
        assert!(matches!(
            transport.open("ftp://example.invalid/x", None, 10),
            Err(TransportError::UnsupportedScheme)
        ));
        assert_eq!(TransportError::Http { status: 404 }.kind(), "http");
        assert_eq!(
            TransportError::Http { status: 403 }.http_status(),
            Some(403)
        );
        assert!(TransportError::Http { status: 403 }
            .to_string()
            .contains("rate limit"));
    }
}
