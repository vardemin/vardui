//! The TOML subset used by `vard.toml`: `[table]`, `[a.b]`, `[[array]]`, `key = value`
//! with strings ("..." / '...'), integers, booleans, arrays and inline tables.
//!
//! Own parser instead of a crate for one reason: errors in the manifest get the same
//! treatment as errors in code — `vard.toml:LINE`, the line, and a fix when it is
//! knowable (`version = 1.2` → `version = "1.2"`).

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
    Table(Table),
}

pub type Table = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct TomlError {
    pub line: usize,
    pub msg: String,
    /// replacement for the whole line, when the intended text is clear
    pub fix: Option<String>,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Array of strings (a single string is accepted as a one-element list).
    pub fn str_list(&self) -> Option<Vec<String>> {
        match self {
            Value::Str(s) => Some(vec![s.clone()]),
            Value::Array(a) => a.iter().map(|v| v.as_str().map(|s| s.to_string())).collect(),
            _ => None,
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl P<'_> {
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, TomlError> {
        Err(TomlError { line: self.line, msg: msg.into(), fix: None })
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.i += 1;
        }
    }
    /// whitespace, newlines and comments (inside arrays / inline tables)
    fn ws_nl(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r') => self.i += 1,
                Some(b'\n') => {
                    self.i += 1;
                    self.line += 1;
                }
                Some(b'#') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }
    fn key(&mut self) -> Result<String, TomlError> {
        self.ws();
        match self.peek() {
            Some(b'"') | Some(b'\'') => self.string(),
            _ => {
                let start = self.i;
                while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
                    self.i += 1;
                }
                if start == self.i {
                    return self.err("expected a key");
                }
                Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }
    fn dotted_key(&mut self) -> Result<Vec<String>, TomlError> {
        let mut parts = vec![self.key()?];
        loop {
            self.ws();
            if self.peek() == Some(b'.') {
                self.i += 1;
                parts.push(self.key()?);
            } else {
                return Ok(parts);
            }
        }
    }
    fn string(&mut self) -> Result<String, TomlError> {
        let q = self.peek().unwrap();
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => return self.err("unterminated string"),
                Some(c) if c == q => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') if q == b'"' => {
                    self.i += 1;
                    let c = self.peek().ok_or(TomlError { line: self.line, msg: "bad escape".into(), fix: None })?;
                    self.i += 1;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        _ => return self.err(format!("unknown escape `\\{}`", c as char)),
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        String::from_utf8(out).or_else(|_| self.err("invalid UTF-8 in string"))
    }
    fn value(&mut self) -> Result<Value, TomlError> {
        self.ws();
        match self.peek() {
            Some(b'"') | Some(b'\'') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws_nl();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        return Ok(Value::Array(items));
                    }
                    items.push(self.value()?);
                    self.ws_nl();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return self.err("expected `,` or `]` in array"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut t = Table::new();
                loop {
                    self.ws();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        return Ok(Value::Table(t));
                    }
                    let k = self.dotted_key()?;
                    self.ws();
                    if self.peek() != Some(b'=') {
                        return self.err("expected `=` in inline table");
                    }
                    self.i += 1;
                    let v = self.value()?;
                    insert(&mut t, &k, v).map_err(|m| TomlError { line: self.line, msg: m, fix: None })?;
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return self.err("expected `,` or `}` in inline table"),
                    }
                }
            }
            _ => {
                let start = self.i;
                while matches!(self.peek(), Some(c) if !matches!(c, b',' | b']' | b'}' | b'\n' | b'#' | b'\r')) {
                    self.i += 1;
                }
                let word = String::from_utf8_lossy(&self.s[start..self.i]).trim().to_string();
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "" => self.err("expected a value"),
                    w => match w.replace('_', "").parse::<i64>() {
                        Ok(n) => Ok(Value::Int(n)),
                        Err(_) => self.err(format!("`{}` is not a TOML value; strings need quotes", w)),
                    },
                }
            }
        }
    }
}

fn insert(t: &mut Table, path: &[String], v: Value) -> Result<(), String> {
    let (last, prefix) = path.split_last().unwrap();
    let mut cur = t;
    for k in prefix {
        let e = cur.entry(k.clone()).or_insert_with(|| Value::Table(Table::new()));
        cur = match e {
            Value::Table(t) => t,
            _ => return Err(format!("`{}` is not a table", k)),
        };
    }
    if cur.contains_key(last) {
        return Err(format!("`{}` is defined twice", path.join(".")));
    }
    cur.insert(last.clone(), v);
    Ok(())
}

/// Table at `path`, creating it (an `[[array]]` path selects its last element).
fn table_at<'a>(root: &'a mut Table, path: &[String]) -> Result<&'a mut Table, String> {
    let mut cur = root;
    for k in path {
        let e = cur.entry(k.clone()).or_insert_with(|| Value::Table(Table::new()));
        cur = match e {
            Value::Table(t) => t,
            Value::Array(a) => match a.last_mut() {
                Some(Value::Table(t)) => t,
                _ => return Err(format!("`{}` is not a table", k)),
            },
            _ => return Err(format!("`{}` is not a table", k)),
        };
    }
    Ok(cur)
}

pub fn parse(text: &str) -> Result<Table, TomlError> {
    let mut root = Table::new();
    let mut current: Vec<String> = Vec::new();
    let mut p = P { s: text.as_bytes(), i: 0, line: 1 };
    loop {
        p.ws_nl();
        let Some(c) = p.peek() else { break };
        let line_start = p.line;
        let line_text = text.lines().nth(line_start - 1).unwrap_or("").to_string();
        let with_line = |mut e: TomlError| {
            e.line = line_start;
            if e.fix.is_none() {
                e.fix = suggest(&line_text);
            }
            e
        };
        if c == b'[' {
            p.i += 1;
            let array = p.peek() == Some(b'[');
            if array {
                p.i += 1;
            }
            let path = p.dotted_key().map_err(with_line)?;
            p.ws();
            let close: &[u8] = if array { b"]]" } else { b"]" };
            if !p.s[p.i..].starts_with(close) {
                return Err(with_line(TomlError { line: 0, msg: "unclosed table header".into(), fix: None }));
            }
            p.i += close.len();
            if array {
                let (last, prefix) = path.split_last().unwrap();
                let parent = table_at(&mut root, prefix).map_err(|m| with_line(TomlError { line: 0, msg: m, fix: None }))?;
                match parent.entry(last.clone()).or_insert_with(|| Value::Array(Vec::new())) {
                    Value::Array(a) => a.push(Value::Table(Table::new())),
                    _ => return Err(with_line(TomlError { line: 0, msg: format!("`{}` is not an array of tables", last), fix: None })),
                }
            } else {
                table_at(&mut root, &path).map_err(|m| with_line(TomlError { line: 0, msg: m, fix: None }))?;
            }
            current = path;
        } else {
            let key = p.dotted_key().map_err(with_line)?;
            p.ws();
            if p.peek() != Some(b'=') {
                return Err(with_line(TomlError { line: 0, msg: format!("expected `=` after `{}`", key.join(".")), fix: None }));
            }
            p.i += 1;
            let v = p.value().map_err(with_line)?;
            let t = table_at(&mut root, &current).map_err(|m| with_line(TomlError { line: 0, msg: m, fix: None }))?;
            insert(t, &key, v).map_err(|m| with_line(TomlError { line: 0, msg: m, fix: None }))?;
        }
        p.ws();
        if p.peek() == Some(b'#') {
            p.ws_nl();
            continue;
        }
        match p.peek() {
            None | Some(b'\n') | Some(b'\r') => {}
            _ => return Err(with_line(TomlError { line: 0, msg: "expected end of line".into(), fix: None })),
        }
    }
    Ok(root)
}

/// A likely intended line for the most common manifest mistake: an unquoted string.
fn suggest(line: &str) -> Option<String> {
    let (k, v) = line.split_once('=')?;
    let v = v.trim();
    if v.is_empty() || v.starts_with(['"', '\'', '[', '{']) || v == "true" || v == "false" || v.parse::<i64>().is_ok() {
        return None;
    }
    Some(format!("{}= \"{}\"", k, v))
}

/// Quote a string for writing TOML.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_shapes() {
        let t = parse(
            r#"
# comment
[package]
name = "ui"          # trailing
version = "1.2.0"
keywords = ["a", 'b',
  "c"]
[dependencies]
charts = "1.4"
local = { path = "../local" }
[native.android]
module = "dev.x.M"
[[migrate]]
from = "A"
[[migrate]]
from = "B"
"#,
        )
        .unwrap();
        assert_eq!(t["package"].as_table().unwrap()["name"], Value::Str("ui".into()));
        assert_eq!(t["package"].as_table().unwrap()["keywords"].str_list().unwrap(), vec!["a", "b", "c"]);
        assert_eq!(t["dependencies"].as_table().unwrap()["local"].as_table().unwrap()["path"], Value::Str("../local".into()));
        assert!(t["native"].as_table().unwrap()["android"].as_table().is_some());
        assert_eq!(t["migrate"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn errors_have_lines_and_fixes() {
        let e = parse("[package]\nversion = 1.2\n").unwrap_err();
        assert_eq!(e.line, 2);
        assert_eq!(e.fix.as_deref(), Some("version = \"1.2\""));
        assert!(parse("a = 1\na = 2").unwrap_err().msg.contains("twice"));
    }
}
