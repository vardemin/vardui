//! Paths under `VARD_HOME` and the HTTP transport.
//!
//! `http://` is spoken directly (std `TcpStream`, enough for `vard registry serve` on a LAN
//! or in CI); `https://` goes through the system `curl` (present on Windows 10+, macOS
//! and Linux), so the binary carries no TLS stack. Integrity never depends on the
//! transport: blobs are content-addressed and the registry log is signed.

use std::io::{Read, Write};
use std::path::PathBuf;

/// `$VARD_HOME`, else `~/.vard`: cache, keys, credentials, pinned registry keys.
pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("VARD_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join(".vard")
}

pub fn cache() -> PathBuf {
    home().join("cache")
}

/// GET. `Ok(None)` = 404 (e.g. an index file of a package that does not exist).
pub fn get(url: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(rest) = url.strip_prefix("http://") {
        let (status, body) = http_plain("GET", rest, None, None)?;
        return match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            s => Err(format!("GET {}: HTTP {}", url, s)),
        };
    }
    if url.starts_with("https://") {
        let out = std::process::Command::new("curl")
            .args(["-sSL", "--max-time", "60", "-w", "\n%{http_code}", url])
            .output()
            .map_err(|e| format!("https needs `curl` on PATH ({}); or use an http:// or file registry", e))?;
        let (body, status) = split_status(out.stdout)?;
        if !out.status.success() {
            return Err(format!("GET {}: {}", url, String::from_utf8_lossy(&out.stderr).trim()));
        }
        return match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            s => Err(format!("GET {}: HTTP {}", url, s)),
        };
    }
    Err(format!("unsupported URL `{}` (http://, https:// or a directory)", url))
}

/// POST with a bearer token; returns (status, body).
pub fn post(url: &str, body: &[u8], token: Option<&str>) -> Result<(u16, Vec<u8>), String> {
    if let Some(rest) = url.strip_prefix("http://") {
        return http_plain("POST", rest, Some(body), token);
    }
    if url.starts_with("https://") {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-sS", "--max-time", "300", "-X", "POST", "--data-binary", "@-", "-H", "Content-Type: application/octet-stream", "-w", "\n%{http_code}"]);
        if let Some(t) = token {
            cmd.args(["-H", &format!("Authorization: Bearer {}", t)]);
        }
        cmd.arg(url).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("https needs `curl` on PATH ({})", e))?;
        child.stdin.take().unwrap().write_all(body).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let (b, s) = split_status(out.stdout)?;
        return Ok((s, b));
    }
    Err(format!("unsupported URL `{}`", url))
}

fn split_status(mut out: Vec<u8>) -> Result<(Vec<u8>, u16), String> {
    let nl = out.iter().rposition(|b| *b == b'\n').ok_or("no HTTP status from curl")?;
    let status: u16 = String::from_utf8_lossy(&out[nl + 1..]).trim().parse().map_err(|_| "no HTTP status from curl")?;
    out.truncate(nl);
    Ok((out, status))
}

fn http_plain(method: &str, rest: &str, body: Option<&[u8]>, token: Option<&str>) -> Result<(u16, Vec<u8>), String> {
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addr = if hostport.contains(':') { hostport.to_string() } else { format!("{}:80", hostport) };
    let mut s = std::net::TcpStream::connect(&addr).map_err(|e| format!("cannot connect to {}: {}", addr, e))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(300))).ok();
    let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: vard/{}\r\n", method, path, hostport, env!("CARGO_PKG_VERSION"));
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {}\r\n", t));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\nContent-Type: application/octet-stream\r\n", b.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(b) = body {
        s.write_all(b).map_err(|e| e.to_string())?;
    }
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).map_err(|e| e.to_string())?;
    let end = resp.windows(4).position(|w| w == b"\r\n\r\n").ok_or("bad HTTP response")?;
    let head = String::from_utf8_lossy(&resp[..end]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("bad HTTP status line")?;
    let mut body = resp[end + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        body = dechunk(&body)?;
    }
    Ok((status, body))
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let nl = b.windows(2).position(|w| w == b"\r\n").ok_or("bad chunked body")?;
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).split(';').next().unwrap_or("").trim(), 16).map_err(|_| "bad chunk size")?;
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(b.get(nl + 2..nl + 2 + size).ok_or("truncated chunk")?);
        b = b.get(nl + 4 + size..).ok_or("truncated chunk")?;
    }
}
