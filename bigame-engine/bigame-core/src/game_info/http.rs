//! One HTTPS GET for the game information providers.
//!
//! `curl`, already a dependency (AI Graphics fetches `OptiScaler` with it),
//! started with an argument vector — never a shell — and none of the
//! user's `~/.curlrc` (`--disable` first). HTTPS only, redirects included;
//! a time limit and a size cap on every request; the status code, the
//! `Retry-After` header and the body read back separately, so a 404, a 429
//! and a 500 are told from each other and from a network that is down.
//! Nothing but the URL leaves the machine, and the only identifier in a URL
//! is a game's public Steam `AppID`.

use std::path::Path;
use std::time::Duration;

/// What the server said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The body, at most the size asked for.
    pub body: Vec<u8>,
    /// How long the server asked to wait (`Retry-After`), on a 429 or 503.
    pub retry_after: Option<Duration>,
}

/// Why no answer came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// No network, no name resolution, or the connection was refused.
    Offline,
    /// The server took longer than the time limit.
    Timeout,
    /// The answer was larger than any real one.
    TooLarge,
    /// Anything else curl reports, in its own words.
    Other(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => f.write_str("offline"),
            Self::Timeout => f.write_str("timed out"),
            Self::TooLarge => f.write_str("answer too large"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

/// The time one request may take, from connecting to the last byte.
const TIME_LIMIT: Duration = Duration::from_secs(15);

/// `GET url`, keeping at most `max_bytes` of the body. With `etag`, the
/// file holding the last `ETag` is sent as `If-None-Match` and replaced
/// with the new one: an unchanged answer is a 304 with no body.
///
/// # Errors
/// Returns why no answer came; any status the server sends is an answer.
pub fn get(url: &str, max_bytes: u64, etag: Option<&Path>) -> Result<Response, Failure> {
    if !url.starts_with("https://") {
        return Err(Failure::Other(format!("not an HTTPS address: {url}")));
    }
    let dir = Scratch::new().map_err(|e| Failure::Other(e.to_string()))?;
    let body_path = dir.0.join("body");
    let header_path = dir.0.join("headers");
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("--disable")
        .args(["--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--max-redirs", "3", "--connect-timeout", "8"])
        .args(["--max-time", &TIME_LIMIT.as_secs().to_string()])
        .args(["--max-filesize", &max_bytes.to_string()])
        .args(["--user-agent", &user_agent()])
        .args(["--header", "Accept: application/json"])
        .arg("--dump-header")
        .arg(&header_path)
        .arg("--output")
        .arg(&body_path)
        .args(["--write-out", "%{http_code}"]);
    if let Some(etag) = etag {
        if etag.is_file() {
            cmd.arg("--etag-compare").arg(etag);
        }
        cmd.arg("--etag-save").arg(etag);
    }
    let out = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Failure::Other(format!("curl: {e}")))?;
    if !out.status.success() {
        return Err(failure(
            out.status.code(),
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    let status: u16 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| Failure::Other("no HTTP status".into()))?;
    let body = read_capped(&body_path, max_bytes)?;
    let headers = std::fs::read_to_string(&header_path).unwrap_or_default();
    Ok(Response {
        status,
        body,
        retry_after: retry_after(&headers),
    })
}

/// A folder of its own for one request's body and headers, removed with
/// everything in it when the request is done.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Self> {
        static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = crate::paths::cache_home()
            .join("bigame-mode/game-info/tmp")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Who asks: the application, its version and where it lives, as the
/// providers ask clients to say.
fn user_agent() -> String {
    format!(
        "BigGameMode/{} (+https://github.com/biglinux/bigamemode)",
        env!("CARGO_PKG_VERSION")
    )
}

/// curl's exit code as a [`Failure`].
fn failure(code: Option<i32>, stderr: &str) -> Failure {
    match code {
        // Could not resolve, connect, or reach the proxy.
        Some(5..=7) => Failure::Offline,
        Some(28) => Failure::Timeout,
        Some(63) => Failure::TooLarge,
        _ => Failure::Other(stderr.trim().chars().take(200).collect()),
    }
}

/// The body file, refused when it outgrew `max_bytes` (a server that does
/// not say the length up front is not stopped by curl's cap).
fn read_capped(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Failure> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > max_bytes => Err(Failure::TooLarge),
        Ok(_) => std::fs::read(path).map_err(|e| Failure::Other(e.to_string())),
        // A 304, or a HEAD-like empty answer, writes no file.
        Err(_) => Ok(Vec::new()),
    }
}

/// `Retry-After` in seconds from the last response's headers (after any
/// redirect). An HTTP date is read as an hour: rarer, and never shorter.
fn retry_after(headers: &str) -> Option<Duration> {
    let last = headers.rsplit("\r\n\r\n").find(|b| !b.trim().is_empty())?;
    last.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("retry-after") {
            return None;
        }
        let value = value.trim();
        Some(
            value
                .parse::<u64>()
                .map_or(Duration::from_secs(3600), |secs| {
                    Duration::from_secs(secs.min(86_400))
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_read_from_the_last_response() {
        let headers =
            "HTTP/2 301\r\nlocation: x\r\n\r\nHTTP/2 429\r\nRetry-After: 120\r\nserver: x\r\n\r\n";
        assert_eq!(retry_after(headers), Some(Duration::from_secs(120)));
        assert_eq!(
            retry_after("HTTP/2 503\r\nretry-after: Wed, 21 Oct 2026 07:28:00 GMT\r\n\r\n"),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(retry_after("HTTP/2 200\r\n\r\n"), None);
        // Never more than a day, whatever is asked.
        assert_eq!(
            retry_after("HTTP/2 429\r\nRetry-After: 99999999\r\n\r\n"),
            Some(Duration::from_secs(86_400))
        );
    }

    #[test]
    fn curls_failures_are_told_apart() {
        assert_eq!(failure(Some(6), "Could not resolve host"), Failure::Offline);
        assert_eq!(failure(Some(7), ""), Failure::Offline);
        assert_eq!(failure(Some(28), ""), Failure::Timeout);
        assert_eq!(failure(Some(63), ""), Failure::TooLarge);
        assert_eq!(failure(Some(35), "SSL"), Failure::Other("SSL".into()));
    }

    #[test]
    fn only_https_is_asked() {
        assert!(matches!(
            get("http://example.com", 10, None),
            Err(Failure::Other(_))
        ));
    }

    #[test]
    fn a_body_larger_than_asked_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("b");
        std::fs::write(&p, vec![0u8; 20]).unwrap();
        assert_eq!(read_capped(&p, 10), Err(Failure::TooLarge));
        assert_eq!(read_capped(&p, 20).unwrap().len(), 20);
        assert_eq!(
            read_capped(&d.path().join("none"), 10).unwrap(),
            Vec::<u8>::new()
        );
    }
}
