//! Release feed (GitHub `releases/latest`) and installer download.
//!
//! The feed is the JSON GitHub returns for the newest non-draft,
//! non-prerelease release. Only four things are read from it: the tag (the
//! version), the release name, the HTML page (for "what's new") and the
//! installer asset (`terminal-manager-<version>-setup.exe`) with its size
//! and `sha256:` digest. Release notes are deliberately not surfaced in the
//! UI; the release page is one click away.
//!
//! Downloads stream to `<name>.partial`, hashing as they go, and are renamed
//! into place only after the byte count and digest match what the feed
//! advertised. A mismatch deletes the partial file and fails the update.

use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::transport::{Transport, TransportError};
use super::version;

pub const GITHUB_ACCEPT: &str = "application/vnd.github+json";
/// The `releases/latest` payload is a few KB; 2 MiB leaves room for long notes.
const FEED_LIMIT_BYTES: u64 = 2 * 1024 * 1024;
/// Installers are ~10 MB; anything past this is not one of ours.
pub const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// The subset of a GitHub release object the updater reads.
#[derive(Debug, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    /// `sha256:<64 hex>`; GitHub adds it to every asset uploaded since 2025.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallerAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub tag: String,
    /// Release name, or the tag when the release has no name.
    pub title: String,
    pub html_url: String,
    pub installer: Option<InstallerAsset>,
}

#[derive(Debug)]
pub enum FeedError {
    Transport(TransportError),
    Parse(String),
    BadTag(String),
}

impl FeedError {
    pub fn kind(&self) -> &'static str {
        match self {
            FeedError::Transport(error) => error.kind(),
            FeedError::Parse(_) => "parse",
            FeedError::BadTag(_) => "bad_tag",
        }
    }

    pub fn http_status(&self) -> Option<u16> {
        match self {
            FeedError::Transport(error) => error.http_status(),
            _ => None,
        }
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedError::Transport(error) => write!(f, "Could not reach GitHub: {error}."),
            FeedError::Parse(detail) => {
                write!(f, "GitHub returned unexpected release data ({detail}).")
            }
            FeedError::BadTag(tag) => write!(f, "Release tag {tag:?} is not a version."),
        }
    }
}

impl std::error::Error for FeedError {}

/// Fetch and parse the latest release.
pub fn fetch_latest(transport: &Transport, feed_url: &str) -> Result<ReleaseInfo, FeedError> {
    let body = transport
        .fetch_text(feed_url, Some(GITHUB_ACCEPT), FEED_LIMIT_BYTES)
        .map_err(FeedError::Transport)?;
    parse_release(&body)
}

pub fn parse_release(json: &str) -> Result<ReleaseInfo, FeedError> {
    // A UTF-8 BOM is not JSON; tolerate it so a feed written by a Windows
    // tool (PowerShell 5's `Set-Content -Encoding UTF8`) still parses.
    let json = json.trim_start_matches('\u{feff}');
    let release: GhRelease =
        serde_json::from_str(json).map_err(|error| FeedError::Parse(error.to_string()))?;
    let version = version::parse_tag(&release.tag_name)
        .ok_or_else(|| FeedError::BadTag(release.tag_name.clone()))?;
    let installer = select_installer(&version, &release.assets);
    let title = release
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| release.tag_name.clone());
    Ok(ReleaseInfo {
        version,
        tag: release.tag_name,
        title,
        html_url: release.html_url,
        installer,
    })
}

/// Pick the installer asset: the exact `terminal-manager-<version>-setup.exe`
/// first, else any `terminal-manager-*-setup.exe` that is not the non-GPU
/// test build.
pub fn select_installer(version: &Version, assets: &[GhAsset]) -> Option<InstallerAsset> {
    let exact = format!("terminal-manager-{version}-setup.exe");
    let pick = assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(&exact))
        .or_else(|| {
            assets.iter().find(|asset| {
                let name = asset.name.to_ascii_lowercase();
                name.starts_with("terminal-manager-")
                    && name.ends_with("-setup.exe")
                    && !name.contains("non-gpu")
            })
        })?;
    Some(InstallerAsset {
        name: pick.name.clone(),
        url: pick.browser_download_url.clone(),
        size: pick.size,
        sha256: pick.digest.as_deref().and_then(parse_digest),
    })
}

/// `sha256:<64 hex>` → raw digest.
pub fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.trim().strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug)]
pub enum DownloadError {
    Transport(TransportError),
    Io(std::io::Error),
    SizeMismatch { expected: u64, actual: u64 },
    DigestMismatch,
    TooLarge,
}

impl DownloadError {
    pub fn kind(&self) -> &'static str {
        match self {
            DownloadError::Transport(error) => error.kind(),
            DownloadError::Io(_) => "io",
            DownloadError::SizeMismatch { .. } => "size_mismatch",
            DownloadError::DigestMismatch => "digest_mismatch",
            DownloadError::TooLarge => "too_large",
        }
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Transport(error) => {
                write!(f, "The installer download failed: {error}.")
            }
            DownloadError::Io(error) => write!(f, "The installer could not be saved: {error}."),
            DownloadError::SizeMismatch { expected, actual } => write!(
                f,
                "The downloaded installer is {actual} bytes but the release lists {expected}; it was discarded."
            ),
            DownloadError::DigestMismatch => write!(
                f,
                "The downloaded installer does not match the release's SHA-256 digest; it was discarded."
            ),
            DownloadError::TooLarge => {
                write!(f, "The installer download exceeded the size limit and was discarded.")
            }
        }
    }
}

impl std::error::Error for DownloadError {}

#[derive(Debug)]
pub struct Downloaded {
    pub path: PathBuf,
    pub bytes: u64,
    pub elapsed_ms: u64,
    /// `false` when the feed carried no digest, so only the size was checked.
    pub verified_digest: bool,
}

/// Download `asset` into `dir`, verify it and return the final path.
/// `progress(received, total)` is called after every chunk; `total` is the
/// advertised size (0 when unknown).
pub fn download_installer(
    transport: &Transport,
    asset: &InstallerAsset,
    dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Downloaded, DownloadError> {
    std::fs::create_dir_all(dir).map_err(DownloadError::Io)?;
    let file_name = safe_asset_name(&asset.name);
    let final_path = dir.join(&file_name);
    let partial_path = dir.join(format!("{file_name}.partial"));
    let started = Instant::now();
    let limit = if asset.size > 0 {
        asset.size.min(MAX_INSTALLER_BYTES)
    } else {
        MAX_INSTALLER_BYTES
    };

    // One extra byte past the limit so an oversized body is detected as
    // such rather than passing as a truncated-but-complete file.
    let mut reader = transport
        .open(&asset.url, None, limit + 1)
        .map_err(DownloadError::Transport)?;
    let result = stream_to_partial(&mut *reader, &partial_path, limit, asset.size, progress);
    let (received, digest) = match result {
        Ok(ok) => ok,
        Err(error) => {
            let _ = std::fs::remove_file(&partial_path);
            return Err(error);
        }
    };

    let verified_digest = match asset.sha256 {
        Some(expected) if expected != digest => {
            let _ = std::fs::remove_file(&partial_path);
            return Err(DownloadError::DigestMismatch);
        }
        Some(_) => true,
        None => false,
    };
    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&partial_path, &final_path).map_err(DownloadError::Io)?;
    Ok(Downloaded {
        path: final_path,
        bytes: received,
        elapsed_ms: started.elapsed().as_millis() as u64,
        verified_digest,
    })
}

fn stream_to_partial(
    reader: &mut dyn Read,
    partial_path: &Path,
    limit: u64,
    expected_size: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(u64, [u8; 32]), DownloadError> {
    let mut file = File::create(partial_path).map_err(DownloadError::Io)?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut buffer = vec![0u8; DOWNLOAD_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut buffer).map_err(DownloadError::Io)?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > limit {
            return Err(DownloadError::TooLarge);
        }
        file.write_all(&buffer[..read]).map_err(DownloadError::Io)?;
        hasher.update(&buffer[..read]);
        progress(received, expected_size);
    }
    file.flush().map_err(DownloadError::Io)?;
    drop(file);
    if expected_size > 0 && received != expected_size {
        return Err(DownloadError::SizeMismatch {
            expected: expected_size,
            actual: received,
        });
    }
    Ok((received, hasher.finalize().into()))
}

/// Restrict the asset name to a plain `.exe` file name: the feed is remote
/// input and must not pick the destination path.
pub fn safe_asset_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.len() > 4 && cleaned.to_ascii_lowercase().ends_with(".exe") {
        cleaned
    } else {
        format!("{cleaned}.exe")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const FIXTURE: &str = r#"{
      "url": "https://api.github.com/repos/alangmartini/unshit-agentic-terminal-manager/releases/1",
      "html_url": "https://github.com/alangmartini/unshit-agentic-terminal-manager/releases/tag/v0.5.0",
      "tag_name": "v0.5.0",
      "name": "0.5.0",
      "draft": false,
      "prerelease": false,
      "body": "Release notes are not read by the updater.",
      "assets": [
        {
          "name": "terminal-manager-0.5.0-non-gpu-setup.exe",
          "browser_download_url": "https://github.com/x/releases/download/v0.5.0/terminal-manager-0.5.0-non-gpu-setup.exe",
          "size": 10,
          "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        },
        {
          "name": "terminal-manager-0.5.0-setup.exe",
          "browser_download_url": "https://github.com/x/releases/download/v0.5.0/terminal-manager-0.5.0-setup.exe",
          "size": 8921088,
          "digest": "sha256:00ff10a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e"
        }
      ]
    }"#;

    #[test]
    fn parses_release_and_picks_exact_installer() {
        let release = parse_release(FIXTURE).unwrap();
        assert_eq!(release.version, Version::new(0, 5, 0));
        assert_eq!(release.tag, "v0.5.0");
        assert_eq!(release.title, "0.5.0");
        assert!(release.html_url.ends_with("/tag/v0.5.0"));
        let installer = release.installer.expect("installer asset");
        assert_eq!(installer.name, "terminal-manager-0.5.0-setup.exe");
        assert_eq!(installer.size, 8_921_088);
        let digest = installer.sha256.expect("digest parsed");
        assert_eq!(digest[0], 0x00);
        assert_eq!(digest[1], 0xff);
        assert_eq!(digest[31], 0x6e);
    }

    #[test]
    fn falls_back_to_any_setup_asset_but_never_non_gpu() {
        let assets = vec![
            GhAsset {
                name: "terminal-manager-0.5.0-non-gpu-setup.exe".into(),
                browser_download_url: "u1".into(),
                size: 1,
                digest: None,
            },
            GhAsset {
                name: "Terminal-Manager-0.5.0-hotfix-setup.exe".into(),
                browser_download_url: "u2".into(),
                size: 2,
                digest: Some("not-a-digest".into()),
            },
        ];
        let picked = select_installer(&Version::new(0, 5, 0), &assets).unwrap();
        assert_eq!(picked.url, "u2");
        assert_eq!(picked.sha256, None, "malformed digest reads as absent");
        let only_non_gpu = &assets[..1];
        assert!(select_installer(&Version::new(0, 5, 0), only_non_gpu).is_none());
    }

    #[test]
    fn release_without_name_uses_tag_as_title() {
        let json = r#"{"tag_name":"v0.6.0","name":"","html_url":"h","assets":[]}"#;
        let release = parse_release(json).unwrap();
        assert_eq!(release.title, "v0.6.0");
        assert!(release.installer.is_none());
    }

    #[test]
    fn feed_with_a_utf8_bom_still_parses() {
        let json = "\u{feff}{\"tag_name\":\"v0.6.0\",\"assets\":[]}";
        assert_eq!(parse_release(json).unwrap().version, Version::new(0, 6, 0));
    }

    #[test]
    fn bad_json_and_bad_tag_are_distinct_kinds() {
        assert_eq!(parse_release("nope").unwrap_err().kind(), "parse");
        let json = r#"{"tag_name":"nightly","assets":[]}"#;
        assert_eq!(parse_release(json).unwrap_err().kind(), "bad_tag");
    }

    #[test]
    fn digest_parsing_is_strict() {
        assert!(parse_digest("sha256:abc").is_none());
        assert!(parse_digest("md5:00000000000000000000000000000000").is_none());
        let ok = parse_digest(&format!("sha256:{}", "ab".repeat(32))).unwrap();
        assert!(ok.iter().all(|b| *b == 0xab));
        assert_eq!(hex(&ok[..2]), "abab");
    }

    #[test]
    fn asset_names_are_sanitised_to_a_plain_exe() {
        assert_eq!(
            safe_asset_name("terminal-manager-0.5.0-setup.exe"),
            "terminal-manager-0.5.0-setup.exe"
        );
        // Separators and traversal collapse into a flat file name.
        let traversal = safe_asset_name("..\\..\\evil");
        assert_eq!(traversal, "_.._evil.exe");
        assert!(!traversal.contains(['\\', '/']));
        assert_eq!(safe_asset_name("a/b c.EXE"), "a_b_c.EXE");
    }

    /// Minimal HTTP/1.1 server: one response per accepted connection, in
    /// order. Returns the base URL.
    fn serve(responses: Vec<(u16, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                // Consume the request head.
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    line.clear();
                }
                let mut stream = reader.into_inner();
                let reason = if status == 200 { "OK" } else { "Error" };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tm-update-feed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn end_to_end_feed_download_and_verify_over_http() {
        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let digest: [u8; 32] = Sha256::digest(&payload).into();
        // Two servers: the asset server's URL has to be known before the
        // feed body that points at it can be written.
        let base = serve(vec![(200, payload.clone())]);
        let feed_json = format!(
            r#"{{"tag_name":"v9.9.9","name":"nine","html_url":"{base}/page","assets":[{{"name":"terminal-manager-9.9.9-setup.exe","browser_download_url":"{base}/asset","size":{},"digest":"sha256:{}"}}]}}"#,
            payload.len(),
            hex(&digest)
        );
        let feed_base = serve(vec![(200, feed_json.into_bytes())]);
        let transport = Transport::new(false);

        let release = fetch_latest(&transport, &format!("{feed_base}/latest")).unwrap();
        assert_eq!(release.version, Version::new(9, 9, 9));
        let asset = release.installer.clone().unwrap();

        let dir = temp_dir("ok");
        let mut ticks = Vec::new();
        let downloaded = download_installer(&transport, &asset, &dir, &mut |got, total| {
            ticks.push((got, total));
        })
        .unwrap();
        assert_eq!(downloaded.bytes, payload.len() as u64);
        assert!(downloaded.verified_digest);
        assert_eq!(
            downloaded.path.file_name().unwrap().to_string_lossy(),
            "terminal-manager-9.9.9-setup.exe"
        );
        assert_eq!(std::fs::read(&downloaded.path).unwrap(), payload);
        assert!(!ticks.is_empty());
        assert_eq!(ticks.last().unwrap().0, payload.len() as u64);
        assert!(ticks
            .iter()
            .all(|(_, total)| *total == payload.len() as u64));
        assert!(!dir
            .join("terminal-manager-9.9.9-setup.exe.partial")
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_mismatch_discards_the_file() {
        let payload = b"not the installer".to_vec();
        let base = serve(vec![(200, payload.clone())]);
        let asset = InstallerAsset {
            name: "terminal-manager-9.9.9-setup.exe".into(),
            url: format!("{base}/asset"),
            size: payload.len() as u64,
            sha256: Some([0x11; 32]),
        };
        let dir = temp_dir("digest");
        let error =
            download_installer(&Transport::new(false), &asset, &dir, &mut |_, _| {}).unwrap_err();
        assert_eq!(error.kind(), "digest_mismatch");
        assert!(!dir.join("terminal-manager-9.9.9-setup.exe").exists());
        assert!(!dir
            .join("terminal-manager-9.9.9-setup.exe.partial")
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn size_mismatch_and_http_errors_are_reported() {
        let payload = b"short".to_vec();
        let base = serve(vec![(200, payload.clone()), (404, b"gone".to_vec())]);
        let transport = Transport::new(false);
        let asset = InstallerAsset {
            name: "terminal-manager-9.9.9-setup.exe".into(),
            url: format!("{base}/asset"),
            size: 999,
            sha256: None,
        };
        let dir = temp_dir("size");
        let error = download_installer(&transport, &asset, &dir, &mut |_, _| {}).unwrap_err();
        assert_eq!(error.kind(), "size_mismatch");

        let error = fetch_latest(&transport, &format!("{base}/latest")).unwrap_err();
        assert_eq!(error.kind(), "http");
        assert_eq!(error.http_status(), Some(404));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_without_digest_verifies_size_only() {
        let payload = vec![7u8; 1234];
        let base = serve(vec![(200, payload.clone())]);
        let asset = InstallerAsset {
            name: "terminal-manager-9.9.9-setup.exe".into(),
            url: format!("{base}/asset"),
            size: payload.len() as u64,
            sha256: None,
        };
        let dir = temp_dir("nodigest");
        let downloaded =
            download_installer(&Transport::new(false), &asset, &dir, &mut |_, _| {}).unwrap();
        assert!(!downloaded.verified_digest);
        assert_eq!(downloaded.bytes, 1234);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The production HTTPS path end to end: the live GitHub feed, the
    /// redirect from github.com to the release-asset host, an installer-sized
    /// TLS stream and whatever digest GitHub publishes for the asset. Nothing
    /// is launched; the file is deleted again. Run on demand:
    /// `cargo test -p terminal-manager live_release_installer_downloads_and_verifies -- --ignored --nocapture`
    #[test]
    #[ignore = "network: downloads the latest published installer"]
    fn live_release_installer_downloads_and_verifies() {
        let transport = Transport::new(false);
        let release =
            fetch_latest(&transport, crate::updater::DEFAULT_FEED_URL).expect("live feed");
        let asset = release
            .installer
            .as_ref()
            .expect("the latest release carries a *-setup.exe asset");
        eprintln!(
            "latest {} ({}): {} is {} bytes, digest {}",
            release.version,
            release.tag,
            asset.name,
            asset.size,
            if asset.sha256.is_some() {
                "published"
            } else {
                "absent"
            }
        );
        let dir = std::env::temp_dir().join(format!("tm-live-download-{}", std::process::id()));
        let mut last = (0u64, 0u64);
        let downloaded = download_installer(&transport, asset, &dir, &mut |done, total| {
            last = (done, total);
        })
        .expect("download");
        eprintln!(
            "downloaded {} in {} ms, digest verified: {}",
            downloaded.path.display(),
            downloaded.elapsed_ms,
            downloaded.verified_digest
        );
        assert!(downloaded.path.is_file());
        assert_eq!(downloaded.bytes, asset.size);
        assert_eq!(last, (asset.size, asset.size));
        assert_eq!(downloaded.verified_digest, asset.sha256.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
