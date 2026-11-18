//! On-device inspection: the action protocol of `vard run` over TCP, so the same `tree`,
//! `tap`, `why`, `expect` (and the MCP tools on top of them) work against a real device.
//!
//! Framing is line-based text, SMTP-style: the client sends script lines and then a line
//! `.`; the server answers with the output lines, a status line `ok` or `fail`, and `.`.
//! A line that starts with `.` is sent with one more `.` in front. The listener thread
//! only moves strings; scripts run on the UI thread when the host calls `inspect_poll`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

pub struct Request {
    pub script: String,
    pub reply: mpsc::Sender<(String, bool)>,
}

pub type Inbox = mpsc::Receiver<Request>;

pub fn listen(addr: &str) -> Result<Inbox, String> {
    let listener = TcpListener::bind(addr).map_err(|e| format!("cannot listen on {}: {}", addr, e))?;
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::Builder::new()
        .name("vard-inspect".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                let _ = std::thread::Builder::new().name("vard-inspect-conn".into()).spawn(move || serve(stream, tx));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

fn serve(stream: TcpStream, tx: mpsc::Sender<Request>) {
    let Ok(mut out) = stream.try_clone() else { return };
    let mut lines = BufReader::new(stream).lines();
    loop {
        let mut script = String::new();
        loop {
            match lines.next() {
                Some(Ok(l)) if l == "." => break,
                Some(Ok(l)) => {
                    script.push_str(l.strip_prefix('.').filter(|_| l.starts_with("..")).unwrap_or(&l));
                    script.push('\n');
                }
                _ => return,
            }
        }
        let (rtx, rrx) = mpsc::channel();
        if tx.send(Request { script, reply: rtx }).is_err() {
            return;
        }
        let (text, ok) = rrx.recv_timeout(Duration::from_secs(30)).unwrap_or_else(|_| ("timed out: the app's UI thread did not poll (is it in the foreground?)\n".into(), false));
        let mut resp = String::new();
        for l in text.lines() {
            if l.starts_with('.') {
                resp.push('.');
            }
            resp.push_str(l);
            resp.push('\n');
        }
        resp.push_str(if ok { "ok\n.\n" } else { "fail\n.\n" });
        if out.write_all(resp.as_bytes()).is_err() {
            return;
        }
    }
}

/// Client side (`vard run --device ADDR`): send one script, return (output, ok).
pub fn request(addr: &str, script: &str) -> Result<(String, bool), String> {
    let mut s = TcpStream::connect(addr).map_err(|e| format!("cannot reach the app at {} ({}); on Android: adb forward tcp:8472 tcp:8472", addr, e))?;
    s.set_read_timeout(Some(Duration::from_secs(40))).ok();
    let mut msg = String::new();
    for l in script.lines() {
        if l.starts_with('.') {
            msg.push('.');
        }
        msg.push_str(l);
        msg.push('\n');
    }
    msg.push_str(".\n");
    s.write_all(msg.as_bytes()).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = Vec::new();
    for l in BufReader::new(s).lines() {
        let l = l.map_err(|e| e.to_string())?;
        if l == "." {
            break;
        }
        lines.push(l.strip_prefix('.').filter(|_| l.starts_with("..")).unwrap_or(&l).to_string());
    }
    let ok = lines.pop().as_deref() == Some("ok");
    let mut out = lines.join("\n");
    out.push('\n');
    Ok((out, ok))
}
