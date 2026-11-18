//! `vard.toml` — one manifest format for apps and packages.
//!
//! ```toml
//! [package]                      # only in packages; an app may have [app] name = "..."
//! name = "charts"                # [a-z][a-z0-9_]*, the qualifier in code: charts.LineChart
//! version = "1.4.0"
//! summary = "Line, bar and pie charts drawn with Vard elements"   # one line, shown in search
//! keywords = ["chart", "graph"]
//! vard = ">=0.1"                 # core versions this package works with
//! capabilities = ["prefs"]       # must match what the compiler computes
//!
//! [dependencies]
//! tailwind = "1.0"               # registry dependency: minimum version 1.0.0, line 1.x
//! ui = { version = "2", registry = "acme" }
//! local = { path = "../local" }
//!
//! [policy]                       # app side: sensitive capabilities the app accepts
//! allow = ["native", "push"]
//!
//! [registries]
//! acme = "https://vard.acme.dev"
//! default = "acme"               # where unqualified dependencies come from (default: builtin)
//!
//! [sim]                          # headless answers for this package's externs (JSON)
//! authCurrentUser = 'null'
//!
//! [[migrate]]                    # renames → machine fixes in consumers (E0234)
//! since = "2.0.0"
//! from = "TwButton"
//! to = "Button"
//!
//! [native.android]
//! module = "dev.vard.firebase.FirebaseModule"
//! gradle = "native/android"      # Android library inside the package
//! ```

use crate::semver::{Req, Version};
use crate::toml::{self, Table, Value};
use crate::Problem;
use vard_lang::link::Migration;

#[derive(Clone, Debug, Default)]
pub struct PackageMeta {
    pub name: String,
    pub version: Option<Version>,
    pub summary: String,
    pub keywords: Vec<String>,
    pub license: String,
    pub repository: String,
    pub vard: Option<Req>,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DepSource {
    /// registry name (`None` = the project's default registry)
    Registry(Option<String>),
    Path(String),
    /// a git repository: `{ git = URL, tag | rev | branch = "...", dir = "sub/dir" }`
    Git { url: String, reference: GitRef, dir: Option<String> },
}

/// What to check out; the commit it resolves to is pinned in vard.lock.
#[derive(Clone, Debug, PartialEq)]
pub enum GitRef {
    Tag(String),
    Rev(String),
    Branch(String),
    /// the remote's default branch
    Head,
}

impl GitRef {
    /// The name git understands (`refs/tags/v1`, a commit, `origin/main`, `HEAD`).
    pub fn spec(&self) -> String {
        match self {
            GitRef::Tag(t) => format!("refs/tags/{}", t),
            GitRef::Rev(r) => r.clone(),
            GitRef::Branch(b) => format!("origin/{}", b),
            GitRef::Head => "origin/HEAD".into(),
        }
    }
    /// For messages and vard.lock (`tag=v1`).
    pub fn describe(&self) -> String {
        match self {
            GitRef::Tag(t) => format!("tag={}", t),
            GitRef::Rev(r) => format!("rev={}", r),
            GitRef::Branch(b) => format!("branch={}", b),
            GitRef::Head => "HEAD".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dep {
    pub name: String,
    pub req: Option<Req>,
    pub source: DepSource,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct NativeAndroid {
    pub module: String,
    pub gradle: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub package: Option<PackageMeta>,
    pub app_name: Option<String>,
    pub deps: Vec<Dep>,
    pub allow: Vec<String>,
    pub registries: Vec<(String, String)>,
    pub default_registry: Option<String>,
    pub sim: Vec<(String, String)>,
    pub migrations: Vec<Migration>,
    pub android: Option<NativeAndroid>,
}

/// Package names: lowercase identifiers, so `name.View` is valid Vard.
pub fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    matches!(c.next(), Some('a'..='z')) && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && n.len() <= 32
}

/// Names a package may not take: std, tooling words, and words a model would guess for core.
pub const RESERVED: &[&str] = &["std", "vard", "core", "app", "builtin", "default", "test", "tests", "self", "root", "native", "platform"];

fn line_of(text: &str, section: &str, key: &str) -> usize {
    let mut in_section = section.is_empty();
    for (i, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            in_section = t.trim_matches(['[', ']']).trim() == section;
            continue;
        }
        if in_section && t.split(['=', ' ']).next() == Some(key) {
            return i + 1;
        }
    }
    1
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, Vec<Problem>> {
        let t = toml::parse(text).map_err(|e| vec![Problem { code: "P020", line: e.line, msg: e.msg, fix: e.fix }])?;
        let mut m = Manifest::default();
        let mut probs = Vec::new();
        let bad = |section: &str, key: &str, msg: String, fix: Option<String>, probs: &mut Vec<Problem>| {
            probs.push(Problem { code: "P021", line: line_of(text, section, key), msg, fix });
        };
        let get_str = |tb: &Table, k: &str| tb.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        for k in t.keys() {
            if !["package", "app", "dependencies", "policy", "registries", "sim", "migrate", "native"].contains(&k.as_str()) {
                bad("", k, format!("unknown section `[{}]`", k), None, &mut probs);
            }
        }
        if let Some(app) = t.get("app").and_then(|v| v.as_table()) {
            m.app_name = get_str(app, "name");
        }
        if let Some(pk) = t.get("package").and_then(|v| v.as_table()) {
            let name = get_str(pk, "name").unwrap_or_default();
            if !valid_name(&name) {
                bad("package", "name", format!("package name `{}` must be a lowercase identifier ([a-z][a-z0-9_]*, at most 32 chars)", name), None, &mut probs);
            }
            let version = match get_str(pk, "version") {
                Some(v) => match Version::parse(&v) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        bad("package", "version", e, None, &mut probs);
                        None
                    }
                },
                None => {
                    bad("package", "name", "[package] needs `version = \"0.1.0\"`".into(), None, &mut probs);
                    None
                }
            };
            let vard = match get_str(pk, "vard") {
                Some(r) => match Req::parse(&r) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        bad("package", "vard", e, None, &mut probs);
                        None
                    }
                },
                None => None,
            };
            for k in pk.keys() {
                if !["name", "version", "summary", "keywords", "license", "repository", "vard", "capabilities"].contains(&k.as_str()) {
                    bad("package", k, format!("unknown key `{}` in [package]", k), None, &mut probs);
                }
            }
            let caps = pk.get("capabilities").and_then(|v| v.str_list()).unwrap_or_default();
            for c in &caps {
                if !vard_lang::api::CAPABILITIES.iter().any(|(n, _)| n == c) {
                    bad("package", "capabilities", format!("unknown capability `{}`; known: {}", c, cap_names()), None, &mut probs);
                }
            }
            m.package = Some(PackageMeta {
                name,
                version,
                summary: get_str(pk, "summary").unwrap_or_default(),
                keywords: pk.get("keywords").and_then(|v| v.str_list()).unwrap_or_default(),
                license: get_str(pk, "license").unwrap_or_default(),
                repository: get_str(pk, "repository").unwrap_or_default(),
                vard,
                capabilities: caps,
            });
        }
        if let Some(deps) = t.get("dependencies").and_then(|v| v.as_table()) {
            for (name, v) in deps {
                let line = line_of(text, "dependencies", name);
                if !valid_name(name) {
                    bad("dependencies", name, format!("`{}` is not a package name (lowercase identifier)", name), None, &mut probs);
                    continue;
                }
                let (req, source) = match v {
                    Value::Str(r) => (Some(r.clone()), DepSource::Registry(None)),
                    Value::Table(tb) => match (get_str(tb, "path"), get_str(tb, "git")) {
                        (Some(p), _) => (None, DepSource::Path(p)),
                        (None, Some(url)) => {
                            let reference = match (get_str(tb, "tag"), get_str(tb, "rev"), get_str(tb, "branch")) {
                                (Some(t), None, None) => GitRef::Tag(t),
                                (None, Some(r), None) => GitRef::Rev(r),
                                (None, None, Some(b)) => GitRef::Branch(b),
                                (None, None, None) => GitRef::Head,
                                _ => {
                                    bad("dependencies", name, format!("git dependency `{}`: use one of tag, rev or branch", name), None, &mut probs);
                                    continue;
                                }
                            };
                            (None, DepSource::Git { url, reference, dir: get_str(tb, "dir") })
                        }
                        (None, None) => (get_str(tb, "version"), DepSource::Registry(get_str(tb, "registry"))),
                    },
                    _ => {
                        bad("dependencies", name, format!("dependency `{}` must be a version string or a table", name), Some(format!("{} = \"1.0\"", name)), &mut probs);
                        continue;
                    }
                };
                let req = match req {
                    Some(r) => match Req::parse(&r) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            bad("dependencies", name, e, None, &mut probs);
                            continue;
                        }
                    },
                    None if matches!(source, DepSource::Registry(_)) => {
                        bad("dependencies", name, format!("dependency `{}` needs a version: `{} = \"1.0\"`", name, name), None, &mut probs);
                        continue;
                    }
                    None => None,
                };
                m.deps.push(Dep { name: name.clone(), req, source, line });
            }
        }
        if let Some(p) = t.get("policy").and_then(|v| v.as_table()) {
            m.allow = p.get("allow").and_then(|v| v.str_list()).unwrap_or_default();
        }
        if let Some(r) = t.get("registries").and_then(|v| v.as_table()) {
            for (k, v) in r {
                let Some(s) = v.as_str() else { continue };
                if k == "default" {
                    m.default_registry = Some(s.to_string());
                } else {
                    m.registries.push((k.clone(), s.to_string()));
                }
            }
        }
        if let Some(s) = t.get("sim").and_then(|v| v.as_table()) {
            for (k, v) in s {
                match v.as_str() {
                    Some(j) if serde_json::from_str::<serde_json::Value>(j).is_ok() => m.sim.push((k.clone(), j.to_string())),
                    _ => bad("sim", k, format!("[sim] {} must be a JSON value in a string, e.g. {} = '{{\"uid\": \"u1\"}}'", k, k), None, &mut probs),
                }
            }
        }
        if let Some(list) = t.get("migrate").and_then(|v| v.as_array()) {
            for v in list {
                let Some(tb) = v.as_table() else { continue };
                match (get_str(tb, "from"), get_str(tb, "to")) {
                    (Some(from), Some(to)) => m.migrations.push(Migration { since: get_str(tb, "since").unwrap_or_default(), from, to }),
                    _ => bad("migrate", "from", "[[migrate]] needs `from` and `to`".into(), None, &mut probs),
                }
            }
        }
        if let Some(a) = t.get("native").and_then(|v| v.as_table()).and_then(|n| n.get("android")).and_then(|v| v.as_table()) {
            m.android = Some(NativeAndroid { module: get_str(a, "module").unwrap_or_default(), gradle: get_str(a, "gradle") });
        }
        if probs.is_empty() {
            Ok(m)
        } else {
            Err(probs)
        }
    }

    pub fn name(&self) -> &str {
        self.package.as_ref().map(|p| p.name.as_str()).or(self.app_name.as_deref()).unwrap_or("app")
    }
}

pub fn cap_names() -> String {
    vard_lang::api::CAPABILITIES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" ")
}

/// Set (insert or replace) `name = value` in `[dependencies]`, keeping the rest of the file.
pub fn set_dependency(text: &str, name: &str, value: &str) -> String {
    edit_section(text, "dependencies", name, Some(value))
}

pub fn remove_dependency(text: &str, name: &str) -> String {
    edit_section(text, "dependencies", name, None)
}

/// Line-level edit of `key = ...` inside `[section]` (created at the end when missing).
pub fn edit_section(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let header = lines.iter().position(|l| l.trim() == format!("[{}]", section));
    let new_line = value.map(|v| format!("{} = {}", key, v));
    match header {
        Some(h) => {
            let end = lines[h + 1..].iter().position(|l| l.trim_start().starts_with('[')).map(|e| h + 1 + e).unwrap_or(lines.len());
            let existing = (h + 1..end).find(|&i| lines[i].trim().split(['=', ' ']).next() == Some(key));
            match (existing, new_line) {
                (Some(i), Some(nl)) => lines[i] = nl,
                (Some(i), None) => {
                    lines.remove(i);
                }
                (None, Some(nl)) => {
                    // after the last non-empty line of the section
                    let mut at = end;
                    while at > h + 1 && lines[at - 1].trim().is_empty() {
                        at -= 1;
                    }
                    lines.insert(at, nl);
                }
                (None, None) => {}
            }
        }
        None => {
            if let Some(nl) = new_line {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("[{}]", section));
                lines.push(nl);
            }
        }
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_edit() {
        let text = "[app]\nname = \"demo\"\n\n[dependencies]\ncharts = \"1.4\"\nlocal = { path = \"../l\" }\n\n[policy]\nallow = [\"native\"]\n";
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.deps.len(), 2);
        assert_eq!(m.deps[1].source, DepSource::Path("../l".into()));
        assert_eq!(m.allow, vec!["native"]);
        let e = set_dependency(text, "tailwind", "\"1.0\"");
        assert!(e.contains("local = { path = \"../l\" }\ntailwind = \"1.0\"\n\n[policy]"), "{}", e);
        let e = set_dependency(&e, "charts", "\"2.0\"");
        assert!(e.contains("charts = \"2.0\""));
        let e = remove_dependency(&e, "charts");
        assert!(!e.contains("charts"));
        let e = set_dependency("[app]\nname = \"x\"\n", "ui2", "\"1.0\"");
        assert!(e.ends_with("[dependencies]\nui2 = \"1.0\"\n"), "{}", e);
    }

    #[test]
    fn problems_point_at_lines() {
        let p = Manifest::parse("[package]\nname = \"Charts\"\nversion = \"1.0\"\n").unwrap_err();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].line, 2);
        let p = Manifest::parse("[dependencies]\nx = 1\n").unwrap_err();
        assert_eq!(p[0].line, 2);
    }
}
