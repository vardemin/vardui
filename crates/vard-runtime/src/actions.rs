//! Events and the action language.
//!
//! One tiny line-based language drives the app everywhere: `vard run --do`, `.vtest`
//! files and the MCP tools. Because the runtime is deterministic and starts in
//! milliseconds, a "session" is just a replayable list of actions — no device, no
//! long-lived process, no flakiness.
//!
//! After every mutating action the output is a *diff* of the semantic tree, so the
//! effect of an interaction costs a few tokens to observe.

use crate::node::Kind;
use crate::rt::{NodeKey, Runtime};
use crate::value::Value;
use std::collections::HashMap;

pub const ACTIONS_HELP: &str = "\
actions (one per line, or separated by `;`):
  tree                 semantic tree: views, text, inputs, interactive elements, each with @id
  layout               every node with [x,y WxH], key style and source file:line
  tap SEL              tap an element (bubbles to the nearest onTap)
  type SEL \"text\"      replace an Input's text (fires value binding + onChange)
  submit SEL           fire an Input's onSubmit
  scroll SEL DY        scroll the nearest Scroll by DY dp
  why SEL              explain the element's size and position, up the parent chain
  find SEL             print the matching element's line
  state                dump all `state` variables by view
  expect SEL|\"text\"    fail unless present;   expect no \"text\"   fail if present
  expect SEL checked|unchecked|enabled|disabled|offscreen|visible
  theme light|dark     switch theme        size W H    resize the screen
  mock NAME VALUE      return VALUE from `extern fn NAME` (Int, Float, \"String\", true/false, nil)
  log                  print() output and native calls so far
  lint                 UI problems (overflow, tap targets, contrast, ...)
  shot FILE.png [S]    write a screenshot at scale S (default 1)
  stats                effect runs, layouts, node count
selectors: @12 (node id from tree) | #name (the `id:` prop) | \"label text\" (exact, else substring)";

impl Runtime {
    // ---------------- events ----------------
    pub fn tap(&mut self, key: NodeKey) -> Result<(), String> {
        let uid = self.nodes.get(key).map(|n| n.uid).unwrap_or(0);
        if self.visible_rect(key).is_none() {
            return Err(format!("@{} is offscreen and cannot be tapped; scroll it into view first (`scroll @{} DY`)", uid, uid));
        }
        let mut cur = Some(key);
        while let Some(k) = cur {
            let Some(n) = self.nodes.get(k) else { break };
            if let Some(h) = &n.on_tap {
                if !n.enabled {
                    return Err(format!("@{} is disabled (`enabled: false`), tap ignored", n.uid));
                }
                let (h, span, p) = (h.clone(), n.src, self.prog.clone());
                self.call_func(&p, &h, Vec::new(), span);
                self.settle();
                return Ok(());
            }
            cur = n.parent;
        }
        Err(format!("@{} has no onTap handler (and neither do its ancestors)", uid))
    }

    fn input_under(&self, key: NodeKey) -> Option<NodeKey> {
        let n = self.nodes.get(key)?;
        if n.kind == Kind::Input {
            return Some(key);
        }
        n.children.iter().find_map(|c| self.input_under(*c))
    }

    pub fn input_text(&mut self, key: NodeKey, text: &str) -> Result<(), String> {
        let uid = self.nodes.get(key).map(|n| n.uid).unwrap_or(0);
        let Some(input) = self.input_under(key) else {
            return Err(format!("@{} is not an Input and contains none", uid));
        };
        let p = self.prog.clone();
        let n = &self.nodes[input];
        let (bind, on_change, span) = (n.bind, n.on_change.clone(), n.src);
        match bind {
            Some((expr, sc)) => self.assign(&p, expr, sc, Value::str(text)),
            None => self.nodes[input].text = text.into(),
        }
        if let Some(h) = on_change {
            self.call_func(&p, &h, vec![Some(Value::str(text))], span);
        }
        self.settle();
        Ok(())
    }

    pub fn submit(&mut self, key: NodeKey) -> Result<(), String> {
        let uid = self.nodes.get(key).map(|n| n.uid).unwrap_or(0);
        let Some(input) = self.input_under(key) else {
            return Err(format!("@{} is not an Input and contains none", uid));
        };
        let n = &self.nodes[input];
        let Some(h) = n.on_submit.clone() else {
            return Err(format!("Input @{} has no onSubmit handler", n.uid));
        };
        let (span, p) = (n.src, self.prog.clone());
        self.call_func(&p, &h, Vec::new(), span);
        self.settle();
        Ok(())
    }

    pub fn scroll_by(&mut self, key: NodeKey, dy: f32) -> Result<(), String> {
        let mut cur = Some(key);
        while let Some(k) = cur {
            let Some(n) = self.nodes.get_mut(k) else { break };
            if n.kind == Kind::Scroll {
                let max = (n.content_h - n.rect.h).max(0.0);
                n.scroll_y = (n.scroll_y + dy).clamp(0.0, max);
                self.layout_dirty = true;
                self.layout();
                return Ok(());
            }
            cur = n.parent;
        }
        Err("no Scroll at or above this element".into())
    }

    pub fn resize(&mut self, w: f32, h: f32) {
        self.screen = (w, h);
        self.layout_dirty = true;
        self.layout();
    }

    // ---------------- action language ----------------
    fn snapshot(&self) -> Vec<(u32, String)> {
        self.semantic_nodes().into_iter().map(|n| (n.uid, n.line)).collect()
    }

    /// Run a script. Stops at the first failing action. Returns (output, ok).
    pub fn run_script(&mut self, script: &str) -> (String, bool) {
        let cmds = split_commands(script);
        let mut out = String::new();
        let mut before = self.snapshot();
        let (mut seen_err, mut seen_log, mut seen_native) = (self.errors.len(), self.log.len(), self.native_calls.len());
        for (i, cmd) in cmds.iter().enumerate() {
            out.push_str(&format!("> {}\n", cmd.trim()));
            let toks = tokenize(cmd);
            let res = self.run_action(&toks);
            let mut ok = true;
            match res {
                Ok(ActionOut::Text(t)) => {
                    out.push_str(&t);
                    if !t.ends_with('\n') && !t.is_empty() {
                        out.push('\n');
                    }
                }
                Ok(ActionOut::Mutated) => {
                    let after = self.snapshot();
                    out.push_str(&diff(&before, &after));
                    before = after;
                }
                Err(e) => {
                    out.push_str(&format!("FAIL: {}\n", e));
                    ok = false;
                }
            }
            for l in &self.log[seen_log..] {
                out.push_str(&format!("print: {}\n", l));
            }
            for l in &self.native_calls[seen_native..] {
                out.push_str(&format!("native: {}\n", l));
            }
            for e in &self.errors[seen_err..] {
                out.push_str(&format!("! {}\n", e));
                ok = false;
            }
            (seen_err, seen_log, seen_native) = (self.errors.len(), self.log.len(), self.native_calls.len());
            if !ok {
                let rest = cmds.len() - i - 1;
                if rest > 0 {
                    out.push_str(&format!("stopped: {} remaining action(s) skipped\n", rest));
                }
                return (out, false);
            }
        }
        (out, true)
    }

    fn sel(&self, toks: &[String], i: usize, what: &str) -> Result<NodeKey, String> {
        let s = toks.get(i).ok_or_else(|| format!("`{}` needs a selector (@id, #id or \"text\")", what))?;
        self.find(s)
    }

    fn run_action(&mut self, toks: &[String]) -> Result<ActionOut, String> {
        let cmd = toks.first().map(|s| s.as_str()).unwrap_or("");
        match cmd {
            "tree" if toks.get(1).map(|s| s.as_str()) == Some("layout") => Ok(ActionOut::Text(self.layout_text())),
            "tree" => Ok(ActionOut::Text(self.tree_text())),
            "layout" => Ok(ActionOut::Text(self.layout_text())),
            "tap" => {
                let k = self.sel(toks, 1, cmd)?;
                self.tap(k)?;
                Ok(ActionOut::Mutated)
            }
            "type" => {
                let k = self.sel(toks, 1, cmd)?;
                let text = toks.get(2).ok_or("`type` needs the text: type SEL \"text\"")?;
                self.input_text(k, text)?;
                Ok(ActionOut::Mutated)
            }
            "submit" => {
                let k = self.sel(toks, 1, cmd)?;
                self.submit(k)?;
                Ok(ActionOut::Mutated)
            }
            "scroll" => {
                let k = self.sel(toks, 1, cmd)?;
                let dy: f32 = toks.get(2).and_then(|s| s.parse().ok()).ok_or("`scroll` needs a distance: scroll SEL 300")?;
                self.scroll_by(k, dy)?;
                Ok(ActionOut::Mutated)
            }
            "theme" => {
                match toks.get(1).map(|s| s.as_str()) {
                    Some("dark") => self.set_dark(true),
                    Some("light") => self.set_dark(false),
                    _ => return Err("usage: theme light|dark".into()),
                }
                Ok(ActionOut::Text(String::new()))
            }
            "size" => {
                let w: f32 = toks.get(1).and_then(|s| s.parse().ok()).ok_or("usage: size W H")?;
                let h: f32 = toks.get(2).and_then(|s| s.parse().ok()).ok_or("usage: size W H")?;
                self.resize(w, h);
                Ok(ActionOut::Mutated)
            }
            "mock" => {
                let name = toks.get(1).ok_or("usage: mock NAME VALUE")?;
                let raw = toks.get(2).ok_or("usage: mock NAME VALUE")?;
                if !self.prog.extern_index.contains_key(name) {
                    return Err(format!("no `extern fn {}` in this project", name));
                }
                let v = match raw.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    "nil" => Value::Nil,
                    s => s.parse::<i64>().map(Value::Int).or_else(|_| s.parse::<f64>().map(Value::Float)).unwrap_or_else(|_| Value::str(s)),
                };
                self.mocks.insert(name.clone(), v);
                Ok(ActionOut::Text(String::new()))
            }
            "why" => {
                let k = self.sel(toks, 1, cmd)?;
                Ok(ActionOut::Text(self.why(k)))
            }
            "find" => {
                let k = self.sel(toks, 1, cmd)?;
                let n = &self.nodes[k];
                let line = self.semantic_nodes().into_iter().find(|s| s.key == k).map(|s| s.line);
                Ok(ActionOut::Text(format!("{}  {}\n", line.unwrap_or_else(|| format!("{} @{}", n.kind.name(), n.uid)), self.loc(n))))
            }
            "state" => Ok(ActionOut::Text(self.state_text())),
            "lint" => {
                let l = self.lints();
                Ok(ActionOut::Text(if l.is_empty() { "no UI problems\n".into() } else { l.join("\n") + "\n" }))
            }
            "log" => {
                let mut s = String::new();
                for l in &self.log {
                    s.push_str(&format!("print: {}\n", l));
                }
                for l in &self.native_calls {
                    s.push_str(&format!("native: {}\n", l));
                }
                Ok(ActionOut::Text(if s.is_empty() { "(empty)\n".into() } else { s }))
            }
            "stats" => Ok(ActionOut::Text(format!(
                "nodes={} effects={} effect_runs={} layouts={}\n",
                self.nodes.len(),
                self.effects.len(),
                self.stats.effects_run,
                self.stats.layouts
            ))),
            "shot" => {
                let path = toks.get(1).ok_or("usage: shot FILE.png [scale]")?;
                let scale: f32 = toks.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
                let png = self.render_png(scale)?;
                std::fs::write(path, &png).map_err(|e| format!("cannot write {}: {}", path, e))?;
                Ok(ActionOut::Text(format!("wrote {} ({}x{}, {} bytes)\n", path, (self.screen.0 * scale) as u32, (self.screen.1 * scale) as u32, png.len())))
            }
            "expect" => self.expect(toks),
            "" => Ok(ActionOut::Text(String::new())),
            other => Err(format!("unknown action `{}`\n{}", other, ACTIONS_HELP)),
        }
    }

    fn expect(&mut self, toks: &[String]) -> Result<ActionOut, String> {
        let a = toks.get(1).ok_or("usage: expect SEL|\"text\"  |  expect no \"text\"  |  expect SEL checked")?;
        if a == "no" {
            let t = toks.get(2).ok_or("usage: expect no \"text\"")?;
            if self.has_text(t) || ((t.starts_with('@') || t.starts_with('#')) && self.find(t).is_ok()) {
                return Err(format!("{:?} is present but should not be\n{}", t, self.tree_text()));
            }
            return Ok(ActionOut::Text(String::new()));
        }
        let Some(flag) = toks.get(2) else {
            let is_ref = a.starts_with('@') || a.starts_with('#');
            if (!is_ref && self.has_text(a)) || self.find(a).is_ok() {
                return Ok(ActionOut::Text(String::new()));
            }
            return Err(format!("{:?} not found\n{}", a, self.tree_text()));
        };
        let k = self.find(a)?;
        let n = &self.nodes[k];
        let visible = self.visible_rect(k).is_some();
        let ok = match flag.as_str() {
            "checked" => n.checked == Some(true),
            "unchecked" => n.checked == Some(false),
            "enabled" => n.enabled,
            "disabled" => !n.enabled,
            "offscreen" => !visible,
            "visible" => visible,
            other => return Err(format!("unknown expectation `{}`; use checked|unchecked|enabled|disabled|offscreen|visible", other)),
        };
        if ok {
            Ok(ActionOut::Text(String::new()))
        } else {
            let line = self.semantic_nodes().into_iter().find(|s| s.key == k).map(|s| s.line).unwrap_or_default();
            Err(format!("expected {} to be {}, but it is: {}", a, flag, line))
        }
    }
}

enum ActionOut {
    Text(String),
    Mutated,
}

/// Line diff of two semantic snapshots, keyed by node id.
fn diff(before: &[(u32, String)], after: &[(u32, String)]) -> String {
    let old: HashMap<u32, &String> = before.iter().map(|(u, l)| (*u, l)).collect();
    let new: HashMap<u32, &String> = after.iter().map(|(u, l)| (*u, l)).collect();
    let mut s = String::new();
    for (u, l) in before {
        if !new.contains_key(u) {
            s.push_str(&format!("- {}\n", l));
        }
    }
    for (u, l) in after {
        match old.get(u) {
            None => s.push_str(&format!("+ {}\n", l)),
            Some(o) if *o != l => s.push_str(&format!("~ {}\n", l)),
            _ => {}
        }
    }
    if s.is_empty() {
        s.push_str("(no visible change)\n");
    }
    s
}

/// Split on newlines and `;` outside quotes; drop blanks and `#`-comment lines.
fn split_commands(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut prev = ' ';
    for c in script.chars() {
        match c {
            '"' if prev != '\\' => {
                in_q = !in_q;
                cur.push(c);
            }
            '\n' | ';' if !in_q => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
        prev = c;
    }
    out.push(cur);
    out.into_iter().filter(|c| !c.trim().is_empty() && !c.trim_start().starts_with("# ") && c.trim() != "#").collect()
}

fn tokenize(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = cmd.trim().chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(n) = chars.next() {
                            s.push(match n {
                                'n' => '\n',
                                other => other,
                            });
                        }
                    }
                    '"' => break,
                    c => s.push(c),
                }
            }
            out.push(s);
        } else {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                s.push(c);
                chars.next();
            }
            out.push(s);
        }
    }
    out
}
