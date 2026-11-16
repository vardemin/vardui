//! Minimal MCP server (JSON-RPC 2.0 over stdio, newline-delimited).
//!
//! The tools are thin wrappers over the CLI commands, and they are stateless: a
//! "session" is the list of actions passed to `vard_run`, replayed deterministically
//! from a fresh mount in a few milliseconds. No process to keep alive, nothing to leak.

use crate::{cmd_add, cmd_bindings, cmd_check, cmd_doc, cmd_explain, cmd_run, cmd_test, RunOpts};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn tools() -> Value {
    json!([
        {
            "name": "vard_check",
            "description": "Type-check a Vard project (a .vard file or a directory). Returns compact diagnostics with file:line, the source line and a fix. Set fix=true to apply all machine-applicable fixes to the files first.",
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "file or project directory (default: .)" },
                "fix": { "type": "boolean", "description": "apply auto-fixes and re-check" }
            } }
        },
        {
            "name": "vard_run",
            "description": "Run the app headless and drive it with actions; returns text only. Actions (one per line or `;`-separated): tree | layout | tap SEL | type SEL \"text\" | submit SEL | focus SEL | scroll SEL DY | why SEL | find SEL | state | expect SEL|\"text\" | expect no \"text\" | theme light|dark | size W H | log | lint | stats. Platform: mock NAME JSON | resolve NAME JSON | send NAME JSON | pending | emit EVENT JSON.. | push received|opened \"T\" \"B\" {data} | back [0..1|cancel] | insets T B | keyboard H | permission NAME STATUS | permission NAME answer STATUS | storage | restart. Selectors: @12 (id from tree), #id, \"label\". Mutating actions print a diff of the semantic tree. Each call starts from a fresh app, so pass the whole action sequence.",
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "file or project directory (default: .)" },
                "actions": { "type": "string", "description": "action script (default: tree)" },
                "view": { "type": "string", "description": "root view (default: App)" },
                "width": { "type": "number" }, "height": { "type": "number" },
                "dark": { "type": "boolean" }
            } }
        },
        {
            "name": "vard_screenshot",
            "description": "Run actions, then return a PNG screenshot. Costly in tokens: prefer vard_run (`tree`, `layout`, `why`, `lint`) and use this only to judge visual design.",
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string" },
                "actions": { "type": "string", "description": "actions to run before the screenshot" },
                "view": { "type": "string" },
                "width": { "type": "number" }, "height": { "type": "number" },
                "dark": { "type": "boolean" },
                "scale": { "type": "number", "description": "device pixels per dp (default 1)" }
            } }
        },
        {
            "name": "vard_test",
            "description": "Run the project's *.vtest files (action scripts with expectations). Prints PASS/FAIL per file; for a failure, only the failing step.",
            "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } } }
        },
        {
            "name": "vard_doc",
            "description": "Reference on demand. No topic: the full cheat sheet. Topics: elements, common, actions, std, errors, design (styling; Tailwind/Material/iOS mapping), native (push, permissions, storage, back, insets, native modules), tailwind, firebase, an element name (Column, Text, ...), an enum name, or a view name.",
            "inputSchema": { "type": "object", "properties": { "topic": { "type": "string" }, "path": { "type": "string" } } }
        },
        {
            "name": "vard_add",
            "description": "Add a kit or module to the project: `tailwind` (palette, dark tokens, components) or `firebase` (Auth, Firestore, Analytics externs; push via std events).",
            "inputSchema": { "type": "object", "properties": { "name": { "type": "string" }, "path": { "type": "string", "description": "project directory (default: .)" } }, "required": ["name"] }
        },
        {
            "name": "vard_bindings",
            "description": "Generate a typed Kotlin base class (data classes, enums, abstract functions, event emitters) for the project's extern fn / extern event declarations. Implement it in Kotlin and return it from VardActivity.modules().",
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string" }, "package": { "type": "string" }, "class": { "type": "string" },
                "out": { "type": "string", "description": "write to this file instead of returning the code" }
            } }
        },
        {
            "name": "vard_explain",
            "description": "Explain a diagnostic code (E#### compile error, W#### warning, R#### runtime, L### UI lint).",
            "inputSchema": { "type": "object", "properties": { "code": { "type": "string" } }, "required": ["code"] }
        }
    ])
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn text_result(text: String, ok: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": !ok })
}

fn call(name: &str, a: &Value) -> Value {
    let s = |k: &str| a.get(k).and_then(|v| v.as_str());
    let path = s("path").unwrap_or(".");
    let size = (
        a.get("width").and_then(|v| v.as_f64()).unwrap_or(390.0) as f32,
        a.get("height").and_then(|v| v.as_f64()).unwrap_or(844.0) as f32,
    );
    let dark = a.get("dark").and_then(|v| v.as_bool()).unwrap_or(false);
    match name {
        "vard_check" => {
            let (t, ok) = cmd_check(path, a.get("fix").and_then(|v| v.as_bool()).unwrap_or(false), false);
            text_result(t, ok)
        }
        "vard_run" => {
            let (t, ok) = cmd_run(&RunOpts { path, view: s("view"), size, dark, script: s("actions").unwrap_or("tree"), lint: true });
            text_result(t, ok)
        }
        "vard_screenshot" => {
            let tmp = std::env::temp_dir().join(format!("vard-shot-{}.png", std::process::id()));
            let tmp_s = tmp.to_string_lossy().replace('\\', "/");
            let scale = a.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let script = format!("{}\nshot \"{}\" {}", s("actions").unwrap_or(""), tmp_s, scale);
            let (t, ok) = cmd_run(&RunOpts { path, view: s("view"), size, dark, script: &script, lint: false });
            let png = std::fs::read(&tmp);
            let _ = std::fs::remove_file(&tmp);
            match (ok, png) {
                (true, Ok(bytes)) => json!({ "content": [{ "type": "image", "data": base64(&bytes), "mimeType": "image/png" }], "isError": false }),
                _ => text_result(t, false),
            }
        }
        "vard_test" => {
            let (t, ok) = cmd_test(path);
            text_result(t, ok)
        }
        "vard_doc" => {
            let (t, ok) = cmd_doc(s("topic"), path);
            text_result(t, ok)
        }
        "vard_add" => {
            let (t, ok) = cmd_add(s("name").unwrap_or(""), path, false);
            text_result(t, ok)
        }
        "vard_bindings" => {
            let (t, ok) = cmd_bindings(path, s("package").unwrap_or("dev.vard.generated"), s("class").unwrap_or("AppModule"), s("out"));
            text_result(t, ok)
        }
        "vard_explain" => {
            let (t, ok) = cmd_explain(s("code").unwrap_or(""));
            text_result(t, ok)
        }
        other => text_result(format!("unknown tool `{}`", other), false),
    }
}

pub fn serve() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {}", e) } });
                let _ = writeln!(stdout, "{}", err);
                let _ = stdout.flush();
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "vard", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Vard UI framework tools. Loop: edit .vard -> vard_check (fix=true) -> vard_run with actions -> read the tree diff. Call vard_doc once for the language cheat sheet."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(call(name, &args))
            }
            m if m.starts_with("notifications/") => continue,
            other => Err((-32601, format!("method not found: {}", other))),
        };
        // notifications (no id) get no response
        let Some(id) = id else { continue };
        let resp = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        let _ = writeln!(stdout, "{}", resp);
        let _ = stdout.flush();
    }
}
