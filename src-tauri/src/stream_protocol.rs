//! `stream://` URI scheme: range-honouring file streaming for Quick View
//! media. The built-in asset protocol caps every 206 response at ~1 MB,
//! which starves AVFoundation with round trips on SMB/NFS mounts and makes
//! multi-GB videos unplayable there; this handler serves the requested
//! range up to [`CHUNK_MAX`] per response instead.
//!
//! Only files registered through [`url_for`] are served, each under an
//! unguessable token: the scheme names no path, so it cannot be used to
//! read the rest of the disk. The scheme itself is macOS-only (Linux has
//! the media server); the token registry and helpers are shared.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::UriSchemeResponder;

/// Upper bound per response. Big enough to keep the decoder fed across
/// network-mount latency, small enough that seeks stay snappy and memory
/// stays bounded (each response body is buffered before WebKit gets it).
const CHUNK_MAX: u64 = 16 * 1024 * 1024;

/// Files registered at once; older registrations age out so a long
/// session doesn't accumulate tokens (one preview plays at a time).
const MAX_TOKENS: usize = 16;

/// Served files by token, insertion-ordered for pruning. Shared with the
/// Linux media server.
pub(crate) struct Tokens(Mutex<Vec<(String, PathBuf)>>);

impl Tokens {
    pub(crate) const fn new() -> Self {
        Tokens(Mutex::new(Vec::new()))
    }

    /// The token `path` is served under, registering it first. The same
    /// file registers once.
    pub(crate) fn register(&self, path: &Path) -> Result<String, String> {
        crate::commands::require_absolute(path)?;
        let mut tokens = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((token, _)) = tokens.iter().find(|(_, p)| p == path) {
            return Ok(token.clone());
        }
        let mut bytes = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .map_err(|e| e.to_string())?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        tokens.push((token.clone(), path.to_path_buf()));
        if tokens.len() > MAX_TOKENS {
            tokens.remove(0);
        }
        Ok(token)
    }

    pub(crate) fn lookup(&self, token: &str) -> Option<PathBuf> {
        let tokens = self.0.lock().unwrap_or_else(|e| e.into_inner());
        tokens.iter().find(|(t, _)| t == token).map(|(_, p)| p.clone())
    }
}

static TOKENS: Tokens = Tokens::new();

/// stream://localhost/<token> serving `path`, registering it first.
pub fn url_for(path: &Path) -> Result<String, String> {
    Ok(format!("stream://localhost/{}", TOKENS.register(path)?))
}

/// Register `path` and return the URL a media element can range-request
/// it through.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn stream_url(path: PathBuf) -> Result<String, String> {
    url_for(&path)
}

pub fn handle(request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    // File I/O can touch a slow or dead network mount — off the main thread.
    tauri::async_runtime::spawn_blocking(move || responder.respond(build_response(&request)));
}

/// Content-Type from the file extension. AVFoundation picks a demuxer by
/// MIME type before it sees any bytes, so misreporting breaks playback.
/// (Shared with the Linux media server, which replaces this scheme there.)
pub(crate) fn mime_for(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("mpg" | "mpeg") => "video/mpeg",
        Some("wmv") => "video/x-ms-wmv",
        Some("flv") => "video/x-flv",
        Some("3gp") => "video/3gpp",
        Some("mts" | "m2ts") => "video/mp2t",
        Some("mp3") => "audio/mpeg",
        Some("m4a" | "m4b") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("wav") => "audio/wav",
        Some("flac") => "audio/flac",
        Some("aiff" | "aif") => "audio/aiff",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("caf") => "audio/x-caf",
        Some("wma") => "audio/x-ms-wma",
        _ => "application/octet-stream",
    }
}

/// First range of a `bytes=` header against a file of `len` bytes, as an
/// inclusive pair. None means unsatisfiable (→ 416). WebKit only ever
/// sends single ranges, so further ranges are ignored.
pub(crate) fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        // Suffix form: the last N bytes.
        let n: u64 = b.trim().parse().ok()?;
        if n == 0 || len == 0 {
            return None;
        }
        Some((len.saturating_sub(n), len - 1))
    } else {
        let start: u64 = a.trim().parse().ok()?;
        if start >= len {
            return None;
        }
        let end = match b.trim() {
            "" => len - 1,
            s => s.parse::<u64>().ok()?.min(len - 1),
        };
        if end < start {
            return None;
        }
        Some((start, end))
    }
}

fn build_response(request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    // No Access-Control-Allow-Origin: the only consumers are <video> and
    // <audio> elements, whose loads need no CORS grant, and a wildcard
    // would hand the file to any origin that can name the scheme.
    let plain = |code: StatusCode| {
        Response::builder()
            .status(code)
            .body(Vec::new())
            .unwrap()
    };
    let not_satisfiable = |len: u64| {
        Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Vec::new())
            .unwrap()
    };

    if request.method() != Method::GET && request.method() != Method::HEAD {
        return plain(StatusCode::METHOD_NOT_ALLOWED);
    }
    let raw = request.uri().path();
    let Some(path) = TOKENS.lookup(raw.strip_prefix('/').unwrap_or(raw)) else {
        return plain(StatusCode::NOT_FOUND);
    };
    // Regular files only: a FIFO would hold this worker thread forever.
    let Ok((mut file, meta)) = crate::ops::safe_path::open_regular(&path) else {
        return plain(StatusCode::NOT_FOUND);
    };
    let len = meta.len();
    let mime = mime_for(&path);

    let head = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCEPT_RANGES, "bytes");
    if request.method() == Method::HEAD || len == 0 {
        return head
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, len)
            .body(Vec::new())
            .unwrap();
    }

    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let (start, end) = match range {
        Some(r) => match parse_range(r, len) {
            Some(pair) => pair,
            None => return not_satisfiable(len),
        },
        // A plain GET would otherwise buffer the whole file; answer with
        // the first chunk and let WebKit range through the rest.
        None => (0, len - 1),
    };
    let end = end.min(start.saturating_add(CHUNK_MAX - 1));

    let nbytes = end - start + 1;
    let mut buf = Vec::with_capacity(nbytes as usize);
    if file.seek(SeekFrom::Start(start)).is_err()
        || file.take(nbytes).read_to_end(&mut buf).is_err()
    {
        return plain(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if buf.is_empty() {
        // The file shrank between metadata() and read.
        return not_satisfiable(len);
    }
    let end = start + buf.len() as u64 - 1;

    let whole = range.is_none() && buf.len() as u64 == len;
    let resp = head.header(header::CONTENT_LENGTH, buf.len());
    if whole {
        resp.status(StatusCode::OK).body(buf).unwrap()
    } else {
        resp.status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
            .body(buf)
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_response, parse_range, url_for};
    use tauri::http::{header, Request, StatusCode};

    fn request(path: &std::path::Path, range: Option<&str>) -> Request<Vec<u8>> {
        let mut req = Request::builder().uri(url_for(path).unwrap());
        if let Some(r) = range {
            req = req.header(header::RANGE, r);
        }
        req.body(Vec::new()).unwrap()
    }

    fn temp_file(name: &str, len: usize) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, vec![7u8; len]).unwrap();
        path
    }

    #[test]
    fn serves_requested_range() {
        let path = temp_file("stream_proto_range.bin", 1024);
        let resp = build_response(&request(&path, Some("bytes=10-19")));
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers()[header::CONTENT_RANGE], "bytes 10-19/1024");
        assert_eq!(resp.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(resp.body().len(), 10);
    }

    #[test]
    fn open_ended_request_serves_to_eof() {
        let path = temp_file("stream_proto_open.bin", 1024);
        let resp = build_response(&request(&path, Some("bytes=1000-")));
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(resp.headers()[header::CONTENT_RANGE], "bytes 1000-1023/1024");
        assert_eq!(resp.body().len(), 24);
    }

    #[test]
    fn plain_get_returns_small_file_whole() {
        let path = temp_file("stream_proto_whole.bin", 1024);
        let resp = build_response(&request(&path, None));
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(resp.body().len(), 1024);
        assert!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
            "no cross-origin grant on streamed files"
        );
    }

    #[test]
    fn only_registered_files_are_served() {
        let path = temp_file("stream_proto_unregistered.bin", 16);
        // The path itself, as the scheme once took it, names nothing.
        let by_path = format!(
            "stream://localhost/{}",
            urlencoding::encode(path.to_str().unwrap())
        );
        let req = Request::builder().uri(by_path).body(Vec::new()).unwrap();
        assert_eq!(build_response(&req).status(), StatusCode::NOT_FOUND);
        assert!(url_for(std::path::Path::new("relative.mp4")).is_err());
    }

    #[test]
    fn other_methods_are_refused() {
        let path = temp_file("stream_proto_method.bin", 16);
        let req = Request::builder()
            .method("POST")
            .uri(url_for(&path).unwrap())
            .body(Vec::new())
            .unwrap();
        assert_eq!(build_response(&req).status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let path = std::env::temp_dir().join(format!("stream_proto_fifo_{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let resp = build_response(&request(&path, None));
        let _ = std::fs::remove_file(&path);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn missing_file_is_404() {
        let path = std::path::PathBuf::from("/nonexistent/stream_proto.mp4");
        let resp = build_response(&request(&path, None));
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn out_of_range_is_416() {
        let path = temp_file("stream_proto_416.bin", 100);
        let resp = build_response(&request(&path, Some("bytes=100-")));
        assert_eq!(resp.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(resp.headers()[header::CONTENT_RANGE], "bytes */100");
    }


    #[test]
    fn explicit_range() {
        assert_eq!(parse_range("bytes=0-1", 100), Some((0, 1)));
        assert_eq!(parse_range("bytes=10-200", 100), Some((10, 99)));
    }

    #[test]
    fn open_ended_range() {
        assert_eq!(parse_range("bytes=40-", 100), Some((40, 99)));
    }

    #[test]
    fn suffix_range() {
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-200", 100), Some((0, 99)));
    }

    #[test]
    fn unsatisfiable() {
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-4", 100), None);
        assert_eq!(parse_range("bytes=-0", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }

    #[test]
    fn first_of_multiple_ranges() {
        assert_eq!(parse_range("bytes=0-1, 30-40", 100), Some((0, 1)));
    }
}

#[cfg(test)]
mod large_file_tests {
    use super::build_response;
    use tauri::http::{header, Request, StatusCode};

    #[test]
    fn serves_ranges_beyond_2gb() {
        // Sparse file: 2.2 GB logical size, no disk cost on APFS.
        let path = std::env::temp_dir().join("stream_proto_2gb.bin");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(2_300_000_000).unwrap();
        let req = Request::builder()
            .uri(super::url_for(&path).unwrap())
            .header(header::RANGE, "bytes=2147483648-2147483657")
            .body(Vec::new())
            .unwrap();
        let resp = build_response(&req);
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            resp.headers()[header::CONTENT_RANGE],
            "bytes 2147483648-2147483657/2300000000"
        );
        assert_eq!(resp.body().len(), 10);
        let _ = std::fs::remove_file(&path);
    }
}
