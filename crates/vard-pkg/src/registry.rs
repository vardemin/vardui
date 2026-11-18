//! Registries: static, content-addressed, signed. The read protocol is plain files, so a
//! registry can be hosted by anything that serves files (S3/R2/GCS bucket, a CDN, GitHub
//! Pages, nginx, a directory on a share):
//!
//! ```text
//! config.json            {"vard_registry": 1, "name": "acme", "key": "<ed25519 public key hex>"}
//! catalog.json           one entry per package: name, latest, summary, keywords, caps, exports
//! index/<name>.json      JSON lines, one per published version (append-only; yank = new line)
//! blobs/<sha256>.vpk     immutable archives, cacheable forever
//! log.txt, log.head      the signed append-only publication log (see `log`)
//! ```
//! Writes (`publish`, `yank`) happen in one place — `vard publish --registry DIR`, CI on a
//! git repository holding the registry, or `vard registry serve` — and all of them run the
//! same gate before calling [`append`].

use crate::log::{self, LogEntry};
use crate::net;
use crate::{sha256_hex, Problem};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use vard_lang::api::{ApiItem, ApiParam};

#[derive(Clone, Debug, PartialEq)]
pub struct DepSpec {
    pub name: String,
    pub req: String,
    pub registry: Option<String>,
}

/// One published version, as stored in `index/<name>.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexEntry {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub vard: Option<String>,
    pub deps: Vec<DepSpec>,
    pub caps: Vec<String>,
    pub android: Option<String>,
    pub summary: String,
    pub keywords: Vec<String>,
    pub api: Vec<ApiItem>,
    pub doc_bytes: u64,
    /// (passed, failed) of the publish-time headless test run
    pub tests: Option<(u32, u32)>,
    pub tested_with: String,
    pub owner: String,
    pub yanked: bool,
}

pub fn api_to_json(items: &[ApiItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| {
                let mut o = json!({ "kind": i.kind, "name": i.name });
                if !i.params.is_empty() {
                    o["params"] = Value::Array(i.params.iter().map(|p| json!({ "name": p.name, "ty": p.ty, "default": p.default, "bind": p.bind })).collect());
                }
                if let Some(r) = &i.ret {
                    o["ret"] = json!(r);
                }
                if !i.variants.is_empty() {
                    o["variants"] = json!(i.variants);
                }
                o
            })
            .collect(),
    )
}

pub fn api_from_json(v: &Value) -> Vec<ApiItem> {
    let s = |o: &Value, k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|o| ApiItem {
                    kind: s(o, "kind"),
                    name: s(o, "name"),
                    params: o
                        .get("params")
                        .and_then(|p| p.as_array())
                        .map(|ps| {
                            ps.iter()
                                .map(|p| ApiParam {
                                    name: s(p, "name"),
                                    ty: s(p, "ty"),
                                    default: p.get("default").and_then(|d| d.as_str()).map(String::from),
                                    bind: p.get("bind").and_then(|b| b.as_bool()).unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    ret: o.get("ret").and_then(|r| r.as_str()).map(String::from),
                    variants: o.get("variants").and_then(|v| v.as_array()).map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Short hash of an API (lockfile `api=`): equal hashes = identical public API.
pub fn api_hash(items: &[ApiItem]) -> String {
    sha256_hex(api_to_json(items).to_string().as_bytes())[..12].to_string()
}

fn strs(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

impl IndexEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "version": self.version, "sha256": self.sha256, "size": self.size,
            "vard": self.vard,
            "deps": self.deps.iter().map(|d| json!({ "name": d.name, "req": d.req, "registry": d.registry })).collect::<Vec<_>>(),
            "caps": self.caps, "android": self.android, "summary": self.summary, "keywords": self.keywords,
            "api": api_to_json(&self.api), "doc_bytes": self.doc_bytes,
            "tests": self.tests.map(|(p, f)| json!({ "passed": p, "failed": f })),
            "tested_with": self.tested_with, "owner": self.owner, "yanked": self.yanked,
        })
    }

    pub fn from_json(v: &Value) -> Option<IndexEntry> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        Some(IndexEntry {
            name: s("name")?,
            version: s("version")?,
            sha256: s("sha256")?,
            size: v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            vard: s("vard"),
            deps: v
                .get("deps")
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|d| {
                            Some(DepSpec {
                                name: d.get("name")?.as_str()?.to_string(),
                                req: d.get("req")?.as_str()?.to_string(),
                                registry: d.get("registry").and_then(|r| r.as_str()).map(String::from),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            caps: strs(v, "caps"),
            android: s("android"),
            summary: s("summary").unwrap_or_default(),
            keywords: strs(v, "keywords"),
            api: v.get("api").map(api_from_json).unwrap_or_default(),
            doc_bytes: v.get("doc_bytes").and_then(|x| x.as_u64()).unwrap_or(0),
            tests: v.get("tests").and_then(|t| Some((t.get("passed")?.as_u64()? as u32, t.get("failed")?.as_u64()? as u32))),
            tested_with: s("tested_with").unwrap_or_default(),
            owner: s("owner").unwrap_or_default(),
            yanked: v.get("yanked").and_then(|x| x.as_bool()).unwrap_or(false),
        })
    }
}

/// `catalog.json` row: enough to search and to decide, without fetching anything else.
#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub name: String,
    pub latest: String,
    pub summary: String,
    pub keywords: Vec<String>,
    pub caps: Vec<String>,
    pub exports: Vec<String>,
    pub tests: Option<(u32, u32)>,
}

impl CatalogEntry {
    pub fn of(e: &IndexEntry) -> CatalogEntry {
        CatalogEntry {
            name: e.name.clone(),
            latest: e.version.clone(),
            summary: e.summary.clone(),
            keywords: e.keywords.clone(),
            caps: e.caps.clone(),
            exports: e.api.iter().filter(|i| i.kind != "token").map(|i| i.name.clone()).collect(),
            tests: e.tests,
        }
    }
    fn to_json(&self) -> Value {
        json!({ "name": self.name, "latest": self.latest, "summary": self.summary, "keywords": self.keywords, "caps": self.caps, "exports": self.exports,
                "tests": self.tests.map(|(p, f)| json!({ "passed": p, "failed": f })) })
    }
    fn from_json(v: &Value) -> Option<CatalogEntry> {
        Some(CatalogEntry {
            name: v.get("name")?.as_str()?.to_string(),
            latest: v.get("latest")?.as_str()?.to_string(),
            summary: v.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            keywords: strs(v, "keywords"),
            caps: strs(v, "caps"),
            exports: strs(v, "exports"),
            tests: v.get("tests").and_then(|t| Some((t.get("passed")?.as_u64()? as u32, t.get("failed")?.as_u64()? as u32))),
        })
    }
}

enum Kind {
    Builtin,
    Dir(PathBuf),
    Http(String),
}

#[derive(Default)]
struct State {
    log: Option<Vec<LogEntry>>,
    index: HashMap<String, Vec<IndexEntry>>,
}

pub struct Registry {
    pub name: String,
    /// `builtin`, a directory, or an http(s) base URL
    pub url: String,
    kind: Kind,
    state: RefCell<State>,
}

/// Public root of a directory registry: `DIR/public` when it exists (a registry created
/// by `vard registry init`), else `DIR` itself (a synced copy of `public/`).
pub fn public_dir(dir: &Path) -> PathBuf {
    if dir.join("public").join("config.json").exists() {
        dir.join("public")
    } else {
        dir.to_path_buf()
    }
}

impl Registry {
    pub fn open(name: &str, url: &str) -> Registry {
        let kind = if url == "builtin" {
            Kind::Builtin
        } else if url.starts_with("http://") || url.starts_with("https://") {
            Kind::Http(url.trim_end_matches('/').to_string())
        } else {
            Kind::Dir(public_dir(Path::new(url.strip_prefix("file://").unwrap_or(url))))
        };
        Registry { name: name.to_string(), url: url.to_string(), kind, state: RefCell::new(State::default()) }
    }

    /// `s` names this registry's location (same URL, or the same directory).
    pub fn same_location(&self, s: &str) -> bool {
        match &self.kind {
            Kind::Builtin => s == "builtin",
            Kind::Http(u) => u == s.trim_end_matches('/'),
            Kind::Dir(d) => {
                let other = public_dir(Path::new(s.strip_prefix("file://").unwrap_or(s)));
                std::fs::canonicalize(d).ok().is_some_and(|a| std::fs::canonicalize(&other).ok() == Some(a))
            }
        }
    }

    /// Location to record in other packages' index entries: `builtin`, a URL, or an
    /// absolute directory.
    pub fn location(&self) -> String {
        self.identity()
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self.kind, Kind::Builtin)
    }

    fn fetch(&self, rel: &str) -> Result<Option<Vec<u8>>, String> {
        match &self.kind {
            Kind::Builtin => Ok(None),
            Kind::Dir(d) => {
                let p = d.join(rel);
                match std::fs::read(&p) {
                    Ok(b) => Ok(Some(b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if !d.join("config.json").exists() {
                            return Err(format!("{} is not a Vard registry (no config.json)", d.display()));
                        }
                        Ok(None)
                    }
                    Err(e) => Err(format!("cannot read {}: {}", p.display(), e)),
                }
            }
            Kind::Http(base) => net::get(&format!("{}/{}", base, rel)),
        }
    }

    fn problem(&self, msg: impl Into<String>) -> Problem {
        Problem::new("P002", format!("registry `{}` ({}): {}", self.name, self.url, msg.into()))
    }

    /// Every version of `name` (oldest first); empty if the registry has no such package.
    pub fn index(&self, name: &str) -> Result<Vec<IndexEntry>, Problem> {
        if let Some(v) = self.state.borrow().index.get(name) {
            return Ok(v.clone());
        }
        let entries = match &self.kind {
            Kind::Builtin => crate::builtin::entries().into_iter().filter(|e| e.name == name).collect(),
            _ => {
                let raw = self.fetch(&format!("index/{}.json", name)).map_err(|e| self.problem(e))?;
                parse_index(&String::from_utf8_lossy(&raw.unwrap_or_default()))
            }
        };
        self.state.borrow_mut().index.insert(name.to_string(), entries.clone());
        Ok(entries)
    }

    pub fn catalog(&self) -> Result<Vec<CatalogEntry>, Problem> {
        match &self.kind {
            Kind::Builtin => Ok(crate::builtin::entries().iter().map(CatalogEntry::of).collect()),
            _ => {
                let raw = self.fetch("catalog.json").map_err(|e| self.problem(e))?.unwrap_or_default();
                let v: Value = serde_json::from_slice(&raw).unwrap_or(Value::Array(vec![]));
                Ok(v.as_array().map(|a| a.iter().filter_map(CatalogEntry::from_json).collect()).unwrap_or_default())
            }
        }
    }

    /// The archive of `e`, verified against its hash and the registry's signed log.
    pub fn blob(&self, e: &IndexEntry) -> Result<Vec<u8>, Problem> {
        let data = match &self.kind {
            Kind::Builtin => crate::builtin::blob(&e.name).ok_or_else(|| self.problem(format!("no builtin package `{}`", e.name)))?,
            _ => {
                self.check_logged(e)?;
                self.fetch(&format!("blobs/{}.vpk", e.sha256)).map_err(|x| self.problem(x))?.ok_or_else(|| self.problem(format!("blob of {} {} is missing", e.name, e.version)))?
            }
        };
        let got = sha256_hex(&data);
        if got != e.sha256 {
            return Err(Problem::new("P004", format!("{} {}: archive hash {} differs from the index ({})", e.name, e.version, &got[..12], &e.sha256[..12.min(e.sha256.len())])));
        }
        Ok(data)
    }

    /// The entry must be in the signed log under the same hash.
    pub fn check_logged(&self, e: &IndexEntry) -> Result<(), Problem> {
        if self.is_builtin() {
            return Ok(());
        }
        let log = self.verified_log()?;
        match log.iter().find(|l| l.name == e.name && l.version == e.version) {
            Some(l) if l.sha256 == e.sha256 => Ok(()),
            Some(_) => Err(Problem::new("P004", format!("{} {}: the index hash differs from the signed log — the index was altered", e.name, e.version))),
            None => Err(Problem::new("P004", format!("{} {} is not in the registry's signed log", e.name, e.version))),
        }
    }

    fn verified_log(&self) -> Result<Vec<LogEntry>, Problem> {
        if let Some(l) = &self.state.borrow().log {
            return Ok(l.clone());
        }
        let cfg = self.fetch("config.json").map_err(|e| self.problem(e))?.ok_or_else(|| self.problem("no config.json"))?;
        let cfg: Value = serde_json::from_slice(&cfg).map_err(|_| self.problem("config.json is not JSON"))?;
        let key = cfg.get("key").and_then(|k| k.as_str()).ok_or_else(|| self.problem("config.json has no signing key"))?.to_string();
        let id = self.identity();
        pin_key(&id, &key)?;
        let log_text = String::from_utf8_lossy(&self.fetch("log.txt").map_err(|e| self.problem(e))?.unwrap_or_default()).to_string();
        let head = String::from_utf8_lossy(&self.fetch("log.head").map_err(|e| self.problem(e))?.unwrap_or_default()).to_string();
        let entries = log::verify_log(&log_text, &head, &key).map_err(|e| Problem::new("P005", format!("registry `{}`: {}", self.name, e)))?;
        check_prefix(&id, &entries)?;
        self.state.borrow_mut().log = Some(entries.clone());
        Ok(entries)
    }

    fn identity(&self) -> String {
        match &self.kind {
            Kind::Dir(d) => {
                let s = std::fs::canonicalize(d).unwrap_or(d.clone()).to_string_lossy().replace('\\', "/");
                s.strip_prefix("//?/").unwrap_or(&s).to_string()
            }
            Kind::Http(u) => u.clone(),
            Kind::Builtin => "builtin".into(),
        }
    }
}

pub fn parse_index(text: &str) -> Vec<IndexEntry> {
    let mut out: Vec<IndexEntry> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("sha256").is_none() {
            // a status line: {"name", "version", "yanked": bool}
            let (Some(ver), Some(y)) = (v.get("version").and_then(|x| x.as_str()), v.get("yanked").and_then(|x| x.as_bool())) else { continue };
            if let Some(e) = out.iter_mut().find(|e| e.version == ver) {
                e.yanked = y;
            }
            continue;
        }
        if let Some(e) = IndexEntry::from_json(&v) {
            out.push(e);
        }
    }
    out
}

/// Trust on first use, then never silently: a changed key is an error (like SSH).
fn pin_key(id: &str, key: &str) -> Result<(), Problem> {
    let path = net::home().join("known_registries");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    for line in text.lines() {
        if let Some((u, k)) = line.rsplit_once(' ') {
            if u == id {
                return if k == key {
                    Ok(())
                } else {
                    Err(Problem::new("P005", format!("the signing key of registry {} changed (pinned {}…, now {}…)", id, &k[..8.min(k.len())], &key[..8.min(key.len())]))
                        .fix(format!("if the rotation is confirmed by the registry owner, delete its line from {}", path.display())))
                };
            }
        }
    }
    let _ = std::fs::create_dir_all(net::home());
    let _ = std::fs::write(&path, format!("{}{} {}\n", text, id, key));
    Ok(())
}

/// The log seen last time must be a prefix of this one (history is never rewritten).
fn check_prefix(id: &str, entries: &[LogEntry]) -> Result<(), Problem> {
    let dir = net::cache().join("logs");
    let path = dir.join(&sha256_hex(id.as_bytes())[..16]);
    if let Ok(prev) = std::fs::read_to_string(&path) {
        if let Some((n, chain)) = prev.trim().split_once(' ') {
            let n: usize = n.parse().unwrap_or(0);
            let ok = n == 0 || entries.get(n - 1).is_some_and(|e| e.chain == chain);
            if !ok {
                return Err(Problem::new("P005", format!("registry {} rewrote its history: the log no longer contains the {} entries seen before", id, n)));
            }
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    let last = entries.last().map(|e| e.chain.as_str()).unwrap_or(log::ZERO);
    let _ = std::fs::write(&path, format!("{} {}\n", entries.len(), last));
    Ok(())
}

// ---------------- the write side (directory registries) ----------------

/// Create a registry in `root`: `public/` (serve or sync this anywhere) and `private/`
/// (the signing key and owner tokens — never publish it). Returns the public key.
pub fn init(root: &Path, name: &str) -> Result<String, String> {
    let public = root.join("public");
    if public.join("config.json").exists() {
        return Err(format!("{} is already a registry", root.display()));
    }
    for d in [public.join("index"), public.join("blobs"), root.join("private")] {
        std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {}", d.display(), e))?;
    }
    let (secret, key) = log::keygen()?;
    let w = |p: PathBuf, t: String| std::fs::write(&p, t).map_err(|e| format!("cannot write {}: {}", p.display(), e));
    w(root.join("private").join("registry.key"), secret.clone())?;
    w(root.join("private").join("tokens"), String::new())?;
    w(public.join("config.json"), format!("{}\n", json!({ "vard_registry": 1, "name": name, "key": key })))?;
    w(public.join("catalog.json"), "[]\n".into())?;
    w(public.join("log.txt"), String::new())?;
    w(public.join("log.head"), format!("0 {} {}\n", log::ZERO, log::sign(&secret, &log::head_message(0, log::ZERO))?))?;
    Ok(key)
}

/// A new publish token for `owner` (printed once; only its hash is stored).
pub fn add_token(root: &Path, owner: &str) -> Result<String, String> {
    let mut raw = [0u8; 24];
    getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
    let token = format!("vard_{}", log::hex(&raw));
    let path = root.join("private").join("tokens");
    let mut text = std::fs::read_to_string(&path).map_err(|_| format!("{} is not a registry root (no private/tokens)", root.display()))?;
    text.push_str(&format!("{} {}\n", sha256_hex(token.as_bytes()), owner));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(token)
}

pub fn token_owner(root: &Path, token: &str) -> Option<String> {
    let h = sha256_hex(token.trim().as_bytes());
    std::fs::read_to_string(root.join("private").join("tokens")).ok()?.lines().find_map(|l| l.split_once(' ').filter(|(k, _)| *k == h).map(|(_, o)| o.to_string()))
}

pub fn dir_index(root: &Path, name: &str) -> Vec<IndexEntry> {
    parse_index(&std::fs::read_to_string(public_dir(root).join("index").join(format!("{}.json", name))).unwrap_or_default())
}

pub fn dir_names(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(public_dir(root).join("index"))
        .map(|rd| rd.filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(String::from)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Store a gated package: blob, index line, signed log entry, catalog.
pub fn append(root: &Path, entry: &IndexEntry, blob: &[u8]) -> Result<LogEntry, String> {
    let public = public_dir(root);
    if sha256_hex(blob) != entry.sha256 {
        return Err("blob hash does not match the entry".into());
    }
    let secret = std::fs::read_to_string(root.join("private").join("registry.key")).map_err(|_| format!("{}: no private/registry.key; publishing needs the registry root, not a copy of public/", root.display()))?;
    let log_text = std::fs::read_to_string(public.join("log.txt")).unwrap_or_default();
    let entries = log::parse(&log_text)?;
    let prev = entries.last().map(|e| e.chain.clone()).unwrap_or_else(|| log::ZERO.to_string());
    let seq = entries.len() as u64 + 1;
    let le = LogEntry { seq, name: entry.name.clone(), version: entry.version.clone(), sha256: entry.sha256.clone(), chain: log::chain_next(&prev, seq, &entry.name, &entry.version, &entry.sha256) };
    std::fs::write(public.join("blobs").join(format!("{}.vpk", entry.sha256)), blob).map_err(|e| e.to_string())?;
    let ip = public.join("index").join(format!("{}.json", entry.name));
    let mut idx = std::fs::read_to_string(&ip).unwrap_or_default();
    idx.push_str(&entry.to_json().to_string());
    idx.push('\n');
    std::fs::write(&ip, idx).map_err(|e| e.to_string())?;
    std::fs::write(public.join("log.txt"), format!("{}{}", log_text, log::line(&le))).map_err(|e| e.to_string())?;
    std::fs::write(public.join("log.head"), format!("{} {} {}\n", seq, le.chain, log::sign(&secret, &log::head_message(seq, &le.chain))?)).map_err(|e| e.to_string())?;
    rebuild_catalog(root)?;
    Ok(le)
}

pub fn yank(root: &Path, name: &str, version: &str, yanked: bool) -> Result<(), String> {
    let ip = public_dir(root).join("index").join(format!("{}.json", name));
    let mut idx = std::fs::read_to_string(&ip).map_err(|_| format!("no package `{}`", name))?;
    if !parse_index(&idx).iter().any(|e| e.version == version) {
        return Err(format!("no version {} of `{}`", version, name));
    }
    idx.push_str(&json!({ "name": name, "version": version, "yanked": yanked }).to_string());
    idx.push('\n');
    std::fs::write(&ip, idx).map_err(|e| e.to_string())?;
    rebuild_catalog(root)
}

fn rebuild_catalog(root: &Path) -> Result<(), String> {
    let mut rows = Vec::new();
    for n in dir_names(root) {
        let entries = dir_index(root, &n);
        let latest = entries
            .iter()
            .filter(|e| !e.yanked)
            .max_by(|a, b| crate::semver::Version::parse(&a.version).ok().cmp(&crate::semver::Version::parse(&b.version).ok()));
        if let Some(e) = latest {
            rows.push(CatalogEntry::of(e).to_json());
        }
    }
    let text = format!("[\n{}\n]\n", rows.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(",\n"));
    std::fs::write(public_dir(root).join("catalog.json"), text).map_err(|e| e.to_string())
}
