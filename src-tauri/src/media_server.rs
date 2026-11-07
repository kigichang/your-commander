//! Localhost HTTP range server for Quick View media on Linux. WebKitGTK
//! never issues media range requests against custom URI schemes (the
//! GStreamer source only speaks HTTP), so where macOS uses stream://,
//! Linux serves the file from 127.0.0.1 on an ephemeral port instead.
//! Each served file gets an unguessable token; nothing else is reachable.
//! Bodies stream straight to the socket, so memory stays flat however big
//! the video is, and a seek simply drops the old connection.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::stream_protocol::{mime_for, parse_range};
use crate::commands::blocking;

/// Served files kept registered; older registrations age out so a long
/// session doesn't accumulate tokens (one preview plays at a time).
const MAX_TOKENS: usize = 16;

struct MediaServer {
    port: u16,
    /// token → path, insertion-ordered for pruning.
    tokens: Mutex<Vec<(String, PathBuf)>>,
}

static SERVER: OnceLock<Result<MediaServer, String>> = OnceLock::new();

fn server() -> Result<&'static MediaServer, String> {
    SERVER
        .get_or_init(|| {
            let listener =
                TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    std::thread::spawn(move || {
                        let _ = handle(stream);
                    });
                }
            });
            Ok(MediaServer { port, tokens: Mutex::new(Vec::new()) })
        })
        .as_ref()
        .map_err(|e| e.clone())
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// http://127.0.0.1:<port>/<token> serving `path`, registering it first.
pub fn url_for(path: &Path) -> Result<String, String> {
    let server = server()?;
    let mut tokens = server.tokens.lock().unwrap();
    let token = match tokens.iter().find(|(_, p)| p == path) {
        Some((t, _)) => t.clone(),
        None => {
            let token = random_token()?;
            tokens.push((token.clone(), path.to_path_buf()));
            if tokens.len() > MAX_TOKENS {
                tokens.remove(0);
            }
            token
        }
    };
    Ok(format!("http://127.0.0.1:{}/{token}", server.port))
}

fn lookup(token: &str) -> Option<PathBuf> {
    let server = server().ok()?;
    let tokens = server.tokens.lock().unwrap();
    tokens.iter().find(|(t, _)| t == token).map(|(_, p)| p.clone())
}

/// One request per connection (`Connection: close`); GStreamer reconnects
/// for every range it wants, which keeps the loop trivial.
fn handle(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    let mut range: Option<String> = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_string());
        }
    }

    let mut out = stream;
    let head_only = method == "HEAD";
    if !head_only && method != "GET" {
        return status_line(&mut out, 405, "Method Not Allowed", &[]);
    }
    let Some(path) = target.strip_prefix('/').and_then(lookup) else {
        return status_line(&mut out, 404, "Not Found", &[]);
    };

    let Ok(mut file) = std::fs::File::open(&path) else {
        return status_line(&mut out, 404, "Not Found", &[]);
    };
    let len = match file.metadata() {
        Ok(m) if m.is_file() => m.len(),
        _ => return status_line(&mut out, 404, "Not Found", &[]),
    };
    let mime = mime_for(&path);
    let common = [
        ("Content-Type", mime.to_string()),
        ("Accept-Ranges", "bytes".to_string()),
        ("Connection", "close".to_string()),
    ];

    let (status, start, end) = match range.as_deref() {
        None => (200, 0, len.saturating_sub(1)),
        Some(r) => match parse_range(r, len) {
            Some((s, e)) => (206, s, e),
            None => {
                let hdrs =
                    [("Content-Range", format!("bytes */{len}")), common[2].clone()];
                return status_line(&mut out, 416, "Range Not Satisfiable", &hdrs);
            }
        },
    };

    let body_len = if len == 0 { 0 } else { end + 1 - start };
    let mut headers: Vec<(&str, String)> = common.to_vec();
    headers.push(("Content-Length", body_len.to_string()));
    if status == 206 {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
    }
    let reason = if status == 206 { "Partial Content" } else { "OK" };
    write!(out, "HTTP/1.1 {status} {reason}\r\n")?;
    for (name, value) in &headers {
        write!(out, "{name}: {value}\r\n")?;
    }
    write!(out, "\r\n")?;
    if head_only || body_len == 0 {
        return Ok(());
    }

    // Stream the body; a dropped connection (seek, cursor move) just ends
    // the loop via the write error.
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = body_len;
    let mut buf = vec![0u8; 256 * 1024];
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break; // file shrank underneath us
        }
        out.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

fn status_line(
    out: &mut TcpStream,
    code: u16,
    reason: &str,
    headers: &[(&str, String)],
) -> std::io::Result<()> {
    write!(out, "HTTP/1.1 {code} {reason}\r\n")?;
    for (name, value) in headers {
        write!(out, "{name}: {value}\r\n")?;
    }
    write!(out, "Content-Length: 0\r\nConnection: close\r\n\r\n")
}

/// Register `path` and return a localhost URL that streams it with Range
/// support. Used by Quick View on Linux in place of stream://.
#[tauri::command]
pub async fn media_url(path: PathBuf) -> Result<String, String> {
    blocking(move || url_for(&path)).await
}

#[cfg(test)]
mod tests {
    use super::url_for;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn fetch(url: &str, range: Option<&str>) -> (String, Vec<u8>) {
        let rest = url.strip_prefix("http://").unwrap();
        let (addr, token) = rest.split_once('/').unwrap();
        let mut sock = TcpStream::connect(addr).unwrap();
        let mut req = format!("GET /{token} HTTP/1.1\r\nHost: {addr}\r\n");
        if let Some(r) = range {
            req.push_str(&format!("Range: {r}\r\n"));
        }
        req.push_str("\r\n");
        sock.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        sock.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        (head, raw[split + 4..].to_vec())
    }

    #[test]
    fn serves_ranges_over_localhost() {
        let path = std::env::temp_dir().join("media_server_test.mp4");
        std::fs::write(&path, (0u16..1000).map(|i| (i % 251) as u8).collect::<Vec<_>>())
            .unwrap();
        let url = url_for(&path).unwrap();
        // Same file registers once.
        assert_eq!(url, url_for(&path).unwrap());

        let (head, body) = fetch(&url, None);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: video/mp4"), "{head}");
        assert_eq!(body.len(), 1000);

        let (head, body) = fetch(&url, Some("bytes=10-19"));
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 10-19/1000"), "{head}");
        assert_eq!(body, (10u16..20).map(|i| (i % 251) as u8).collect::<Vec<_>>());

        let (head, _) = fetch(&url, Some("bytes=2000-"));
        assert!(head.starts_with("HTTP/1.1 416"), "{head}");

        // Unknown tokens stay unreachable.
        let bogus = url.rsplit_once('/').unwrap().0.to_string() + "/deadbeef";
        let (head, _) = fetch(&bogus, None);
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
    }
}
