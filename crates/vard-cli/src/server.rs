//! `vard registry serve DIR` — a complete registry host in the `vard` binary.
//!
//! - `GET /...` serves `DIR/public` as static files (the whole read protocol).
//! - `POST /api/v1/publish` (body: a .vpk, `Authorization: Bearer TOKEN`) unpacks the
//!   package into a scratch directory, runs the same gate as `vard publish`, and appends
//!   it to the signed log.
//! - `POST /api/v1/yank` (body: `NAME VERSION [undo]`).
//!
//! Plain HTTP on purpose: put it behind any TLS terminator, or skip it entirely and sync
//! `public/` to a CDN — clients verify signatures and hashes, not the transport.

use crate::pkg::gate;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use vard_pkg::{registry, vpk};

pub fn serve(root: &Path, addr: &str) -> Result<(), String> {
    if !root.join("public").join("config.json").exists() {
        return Err(format!("{} is not a registry root (create one: vard registry init {})", root.display(), root.display()));
    }
    let listener = TcpListener::bind(addr).map_err(|e| format!("cannot listen on {}: {}", addr, e))?;
    eprintln!("vard registry: serving {} on http://{} (publish: POST /api/v1/publish)", root.display(), addr);
    // one request at a time: publishing appends to a single log, so this is also the lock
    for stream in listener.incoming().flatten() {
        if let Err(e) = handle(root, stream) {
            eprintln!("request failed: {}", e);
        }
    }
    Ok(())
}

fn respond(s: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Unprocessable Entity",
    };
    let cache = if ctype == "application/octet-stream" { "public, max-age=31536000, immutable" } else { "no-cache" };
    write!(s, "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nConnection: close\r\n\r\n", status, reason, ctype, body.len(), cache)?;
    s.write_all(body)?;
    s.flush()
}

fn handle(root: &Path, mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(60)))?;
    let mut r = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let mut len = 0usize;
    let mut auth = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        let (k, v) = h.split_once(':').unwrap_or((&h, ""));
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => len = v.trim().parse().unwrap_or(0),
            "authorization" => auth = v.trim().strip_prefix("Bearer ").map(|t| t.trim().to_string()),
            _ => {}
        }
    }
    if len > vard_pkg::MAX_PACKAGE_BYTES {
        return respond(&mut stream, 413, "text/plain", b"error P019: package too large\n");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let path = path.split('?').next().unwrap_or("/").to_string();
    match (method.as_str(), path.as_str()) {
        ("GET", p) | ("HEAD", p) => {
            let rel = p.trim_start_matches('/');
            // `/` is the registry's web page (older registries without one: config.json)
            let rel = if !rel.is_empty() {
                rel
            } else if root.join("public").join("index.html").exists() {
                "index.html"
            } else {
                "config.json"
            };
            if !vpk::safe_path(rel) {
                return respond(&mut stream, 404, "text/plain", b"not found\n");
            }
            match std::fs::read(root.join("public").join(rel)) {
                Ok(b) => {
                    let ct = if rel.ends_with(".json") {
                        "application/json"
                    } else if rel.ends_with(".vpk") {
                        "application/octet-stream"
                    } else if rel.ends_with(".html") {
                        "text/html; charset=utf-8"
                    } else {
                        "text/plain; charset=utf-8"
                    };
                    respond(&mut stream, 200, ct, if method == "HEAD" { &[] } else { &b })
                }
                Err(_) => respond(&mut stream, 404, "text/plain", b"not found\n"),
            }
        }
        ("POST", "/api/v1/publish") => {
            let Some(owner) = auth.as_deref().and_then(|t| registry::token_owner(root, t)) else {
                return respond(&mut stream, 401, "text/plain", b"error P018: missing or unknown token\n");
            };
            let (status, text) = publish(root, &body, &owner);
            respond(&mut stream, status, "text/plain; charset=utf-8", text.as_bytes())
        }
        ("POST", "/api/v1/yank") => {
            let Some(owner) = auth.as_deref().and_then(|t| registry::token_owner(root, t)) else {
                return respond(&mut stream, 401, "text/plain", b"error P018: missing or unknown token\n");
            };
            let b = String::from_utf8_lossy(&body).to_string();
            let f: Vec<&str> = b.split_whitespace().collect();
            let (Some(name), Some(ver)) = (f.first(), f.get(1)) else { return respond(&mut stream, 400, "text/plain", b"body: NAME VERSION [undo]\n") };
            if registry::dir_index(root, name).first().map(|e| e.owner.as_str()) != Some(owner.as_str()) {
                return respond(&mut stream, 403, "text/plain", b"error P018: not the owner\n");
            }
            match registry::yank(root, name, ver, f.get(2) != Some(&"undo")) {
                Ok(()) => respond(&mut stream, 200, "text/plain", format!("ok {} {}\n", name, ver).as_bytes()),
                Err(e) => respond(&mut stream, 422, "text/plain", e.as_bytes()),
            }
        }
        _ => respond(&mut stream, 405, "text/plain", b"GET static files; POST /api/v1/publish, /api/v1/yank\n"),
    }
}

fn publish(root: &Path, blob: &[u8], owner: &str) -> (u16, String) {
    let files = match vpk::unpack(blob) {
        Ok(f) => f,
        Err(e) => return (400, format!("{}\n", e)),
    };
    let scratch = root.join("private").join("incoming").join(&vard_pkg::sha256_hex(blob)[..16]);
    let _ = std::fs::remove_dir_all(&scratch);
    for (rel, b) in &files {
        let p = scratch.join(rel);
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        if std::fs::write(&p, b).is_err() {
            return (500, "cannot write scratch directory\n".into());
        }
    }
    let name = files
        .iter()
        .find(|f| f.0 == "vard.toml")
        .and_then(|f| vard_pkg::manifest::Manifest::parse(&String::from_utf8_lossy(&f.1)).ok())
        .and_then(|m| m.package.map(|p| p.name))
        .unwrap_or_default();
    let prior = registry::dir_index(root, &name);
    let names: Vec<(String, String)> = registry::dir_names(root).into_iter().map(|n| {
        let o = registry::dir_index(root, &n).first().map(|e| e.owner.clone()).unwrap_or_default();
        (n, o)
    }).collect();
    let target = vard_pkg::registry::Registry::open("self", &root.to_string_lossy());
    let g = gate(&scratch, &prior, &names, owner, Some(&target));
    let _ = std::fs::remove_dir_all(&scratch);
    if !g.ok {
        return (422, g.report);
    }
    let e = g.entry.unwrap();
    // the archive is rebuilt from the same files, so its hash equals the uploaded one
    match registry::append(root, &e, &g.blob) {
        Ok(le) => (200, format!("{}published {} {} (log #{})\n", g.report, e.name, e.version, le.seq)),
        Err(x) => (500, format!("{}{}\n", g.report, x)),
    }
}
