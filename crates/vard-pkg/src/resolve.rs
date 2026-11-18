//! Dependency resolution by minimal version selection (MVS, as in Go modules).
//!
//! Every requirement is a minimum on a compatibility line. Walk the requirement graph
//! from the app; for each package take the *largest of the minimums* anyone asked for.
//! No SAT solver, no backtracking, no "latest" — the result depends only on what the
//! manifests say, so it is the same today, tomorrow and in CI, with or without a
//! lockfile. Upgrades happen only when a manifest is edited (`vard update` does that and
//! shows the API diff). A Vard app links one version of each package, so all
//! requirements of one package must meet on one line (`P006` otherwise).

use crate::manifest::{DepSource, Manifest};
use crate::registry::{IndexEntry, Registry};
use crate::semver::{Req, Version};
use crate::{Problem, VARD_VERSION};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::path::{Path, PathBuf};

/// Registries visible to a project: its vard.toml [registries], then ~/.vard/config.toml,
/// then the public registry `vard` (the default; skipped with VARD_OFFLINE=1), then
/// `builtin`. A dependency comes from exactly one registry — its own `registry = ...`
/// or the default — never "whichever has it" (no dependency confusion).
pub struct Registries {
    list: RefCell<Vec<Rc<Registry>>>,
    pub default: String,
}

impl Registries {
    pub fn for_project(m: Option<&Manifest>, project_dir: &Path) -> Registries {
        let mut list: Vec<Rc<Registry>> = Vec::new();
        let mut default = None;
        let push = |name: &str, url: &str, list: &mut Vec<Rc<Registry>>| {
            if !list.iter().any(|r| r.name == name) {
                list.push(Rc::new(Registry::open(name, url)));
            }
        };
        if let Some(m) = m {
            for (n, u) in &m.registries {
                let url = if u.starts_with("http") || u == "builtin" || Path::new(u).is_absolute() { u.clone() } else { project_dir.join(u).to_string_lossy().to_string() };
                push(n, &url, &mut list);
            }
            default = m.default_registry.clone();
        }
        if let Ok(text) = std::fs::read_to_string(crate::net::home().join("config.toml")) {
            if let Ok(t) = crate::toml::parse(&text) {
                if let Some(r) = t.get("registries").and_then(|v| v.as_table()) {
                    for (n, v) in r {
                        if let Some(u) = v.as_str() {
                            if n == "default" {
                                default = default.or(Some(u.to_string()));
                            } else {
                                push(n, u, &mut list);
                            }
                        }
                    }
                }
            }
        }
        let offline = std::env::var_os("VARD_OFFLINE").is_some_and(|v| !v.is_empty() && v != "0");
        if !offline {
            push(crate::PUBLIC_REGISTRY_NAME, crate::PUBLIC_REGISTRY_URL, &mut list);
        }
        push("builtin", "builtin", &mut list);
        if let Ok(env) = std::env::var("VARD_REGISTRY") {
            // a known name or location becomes the default; anything else is registered as `env`
            match list.iter().find(|r| r.name == env || r.url == env) {
                Some(r) => default = Some(r.name.clone()),
                None => {
                    push("env", &env, &mut list);
                    default = Some("env".into());
                }
            }
        }
        let fallback = if offline { "builtin" } else { crate::PUBLIC_REGISTRY_NAME };
        Registries { list: RefCell::new(list), default: default.unwrap_or_else(|| fallback.into()) }
    }

    pub fn all(&self) -> Vec<Rc<Registry>> {
        self.list.borrow().clone()
    }

    /// By configured name, by location (URL / directory), or — for a location recorded in a
    /// package's signed index entry — opened on demand (its key is pinned on first use).
    pub fn get(&self, name: Option<&str>) -> Result<Rc<Registry>, Problem> {
        let n = name.unwrap_or(&self.default);
        if let Some(r) = self.list.borrow().iter().find(|r| r.name == n || r.same_location(n)) {
            return Ok(r.clone());
        }
        if n.starts_with("http://") || n.starts_with("https://") || Path::new(n).join("config.json").exists() || Path::new(n).join("public").exists() {
            let r = Rc::new(Registry::open(n, n));
            self.list.borrow_mut().push(r.clone());
            return Ok(r);
        }
        Err(Problem::new("P002", format!("unknown registry `{}`; known: {}", n, self.list.borrow().iter().map(|r| r.name.clone()).collect::<Vec<_>>().join(" ")))
            .fix(format!("add it to vard.toml: [registries] {} = \"https://...\"", n)))
    }
}

#[derive(Clone, Debug)]
pub enum Src {
    Registry(String),
    Path(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Selected {
    pub name: String,
    pub version: Version,
    pub src: Src,
    pub deps: Vec<String>,
    pub caps: Vec<String>,
    /// registry packages
    pub entry: Option<IndexEntry>,
    /// path packages
    pub manifest: Option<Manifest>,
    /// who asked for it (for messages)
    pub wanted_by: Vec<String>,
}

struct Need {
    name: String,
    req: Option<Req>,
    source: DepSource,
    base: PathBuf,
    by: String,
}

/// Did-you-mean from a registry's catalog — never an automatic substitution.
pub fn suggest(reg: &Registry, name: &str) -> Vec<String> {
    let mut names: Vec<(usize, String)> = reg
        .catalog()
        .unwrap_or_default()
        .into_iter()
        .map(|c| (vard_lang::diag::edit_distance(name, &c.name), c.name))
        .filter(|(d, n)| *d <= 2.max(name.len() / 4) || n.contains(name) || name.contains(n.as_str()))
        .collect();
    names.sort();
    names.into_iter().take(3).map(|(_, n)| n).collect()
}

pub fn not_found(reg: &Registry, name: &str) -> Problem {
    let s = suggest(reg, name);
    let mut p = Problem::new("P001", format!("no package `{}` in registry `{}`", name, reg.name));
    if !s.is_empty() {
        p.msg.push_str(&format!("; did you mean: {}?", s.join(", ")));
    }
    p.fix(format!("search instead of guessing: vard search {}", name))
}

/// Resolve the dependency graph of the project in `dir`.
pub fn resolve(dir: &Path, root: &Manifest, regs: &Registries) -> Result<Vec<Selected>, Problem> {
    let mut queue: Vec<Need> = root.deps.iter().map(|d| Need { name: d.name.clone(), req: d.req.clone(), source: d.source.clone(), base: dir.to_path_buf(), by: root.name().to_string() }).collect();
    queue.reverse();
    // every (name, version) reached, with its data
    let mut reached: BTreeMap<String, Vec<Selected>> = BTreeMap::new();
    while let Some(n) = queue.pop() {
        let sel = match &n.source {
            DepSource::Path(p) => {
                let pdir = n.base.join(p);
                let text = std::fs::read_to_string(pdir.join("vard.toml")).map_err(|_| Problem::new("P022", format!("path dependency `{}`: no vard.toml in {}", n.name, pdir.display())))?;
                let m = Manifest::parse(&text).map_err(|ps| Problem::new("P021", format!("{}/vard.toml: {}", pdir.display(), ps[0].msg)))?;
                let pk = m.package.clone().ok_or_else(|| Problem::new("P021", format!("{} has no [package]", pdir.join("vard.toml").display())))?;
                if pk.name != n.name {
                    return Err(Problem::new("P021", format!("dependency `{}` points at package `{}` ({})", n.name, pk.name, pdir.display())));
                }
                let version = pk.version.clone().unwrap_or(Version::new(0, 0, 0));
                for d in &m.deps {
                    queue.push(Need { name: d.name.clone(), req: d.req.clone(), source: d.source.clone(), base: pdir.clone(), by: n.name.clone() });
                }
                Selected { name: n.name.clone(), version, src: Src::Path(pdir), deps: m.deps.iter().map(|d| d.name.clone()).collect(), caps: pk.capabilities.clone(), entry: None, manifest: Some(m), wanted_by: vec![n.by.clone()] }
            }
            DepSource::Registry(rname) => {
                let reg = regs.get(rname.as_deref())?;
                let entries = reg.index(&n.name)?;
                if entries.is_empty() {
                    return Err(not_found(&reg, &n.name));
                }
                let req = n.req.clone().unwrap_or(Req::parse("0.0.0").unwrap());
                let mut ok: Vec<(Version, &IndexEntry)> = entries.iter().filter_map(|e| Version::parse(&e.version).ok().map(|v| (v, e))).filter(|(v, e)| !e.yanked && req.matches(v)).collect();
                ok.sort_by(|a, b| a.0.cmp(&b.0));
                let Some((v, e)) = ok.first().cloned() else {
                    let avail: Vec<String> = entries.iter().map(|e| if e.yanked { format!("{} (yanked)", e.version) } else { e.version.clone() }).collect();
                    return Err(Problem::new("P003", format!("{} needs {} {}, but `{}` has: {}", n.by, n.name, req, reg.name, avail.join(", "))));
                };
                if let Some(core) = e.vard.as_ref().and_then(|r| Req::parse(r).ok()) {
                    let me = Version::parse(VARD_VERSION).unwrap();
                    if !core.matches(&me) {
                        return Err(Problem::new("P007", format!("{} {} needs vard {}, this is vard {}", e.name, e.version, core, VARD_VERSION)));
                    }
                }
                if !reached.get(&n.name).is_some_and(|r| r.iter().any(|s| s.version == v)) {
                    for d in &e.deps {
                        queue.push(Need { name: d.name.clone(), req: Req::parse(&d.req).ok(), source: DepSource::Registry(d.registry.clone().or(rname.clone())), base: n.base.clone(), by: format!("{} {}", n.name, v) });
                    }
                }
                Selected { name: n.name.clone(), version: v, src: Src::Registry(reg.name.clone()), deps: e.deps.iter().map(|d| d.name.clone()).collect(), caps: e.caps.clone(), entry: Some(e.clone()), manifest: None, wanted_by: vec![n.by.clone()] }
            }
        };
        let list = reached.entry(n.name.clone()).or_default();
        match list.iter_mut().find(|s| s.version == sel.version) {
            Some(s) => s.wanted_by.push(n.by.clone()),
            None => list.push(sel),
        }
    }
    let mut out = Vec::new();
    for (name, mut versions) in reached {
        versions.sort_by(|a, b| a.version.cmp(&b.version));
        let lines: Vec<(u64, u64)> = versions.iter().map(|s| s.version.line()).collect();
        let kinds = versions.iter().map(|s| matches!(s.src, Src::Path(_))).collect::<std::collections::BTreeSet<_>>();
        if lines.windows(2).any(|w| w[0] != w[1]) || kinds.len() > 1 {
            let who: Vec<String> = versions.iter().map(|s| format!("{} wants {} {}", s.wanted_by.join(" & "), name, s.version.line_name())).collect();
            return Err(Problem::new("P006", format!("incompatible requirements for `{}`: {}", name, who.join("; "))));
        }
        let mut best = versions.pop().unwrap();
        for v in versions {
            best.wanted_by.extend(v.wanted_by);
        }
        out.push(best);
    }
    Ok(out)
}

/// Sensitive capabilities of the selection that the app did not allow.
pub fn policy_violations(sel: &[Selected], allow: &[String]) -> Vec<(String, Vec<String>)> {
    let mut v = Vec::new();
    for s in sel {
        let bad: Vec<String> = s.caps.iter().filter(|c| vard_lang::api::SENSITIVE.contains(&c.as_str()) && !allow.contains(c)).cloned().collect();
        if !bad.is_empty() {
            v.push((format!("{} {}", s.name, s.version), bad));
        }
    }
    v
}

pub fn policy_problem(v: &[(String, Vec<String>)], allow: &[String]) -> Problem {
    let mut all: Vec<String> = allow.to_vec();
    for (_, caps) in v {
        for c in caps {
            if !all.contains(c) {
                all.push(c.clone());
            }
        }
    }
    let lines: Vec<String> = v
        .iter()
        .map(|(p, caps)| format!("{} needs {}", p, caps.iter().map(|c| format!("{} ({})", c, vard_lang::api::CAPABILITIES.iter().find(|x| x.0 == c).map(|x| x.1).unwrap_or(""))).collect::<Vec<_>>().join(", ")))
        .collect();
    Problem::new("P008", format!("not allowed by [policy]: {}", lines.join("; ")))
        .fix(format!("a person approves this: add to vard.toml `[policy] allow = [{}]` or rerun with --allow {}", all.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", "), all.join(",")))
}
