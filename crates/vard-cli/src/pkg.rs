//! Package commands: add remove install update search info api pack publish yank bundle
//! registry. Data and protocol live in `vard-pkg`; this module is the UI over it plus the
//! publish *gate* — the checks every registry runs before accepting a version.

use crate::{cmd_test, compile_loaded, load_project, user_diags};
use std::path::{Path, PathBuf};
use vard_lang::api::{self, ApiItem};
use vard_pkg::manifest::{self, DepSource};
use vard_pkg::project::{self, display, Project};
use vard_pkg::registry::{self, IndexEntry, Registry};
use vard_pkg::resolve::{self, Registries};
use vard_pkg::semver::{Req, Version};
use vard_pkg::{builtin, render_all, sha256_hex, vpk, Problem};

type Out = (String, bool);

fn fail(p: Problem, file: &str) -> Out {
    (p.render(file), false)
}

fn kb(n: usize) -> String {
    if n < 1024 {
        format!("{} B", n)
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}

/// "5 views, 2 structs, 1 enum, 247 tokens"
pub fn api_summary(items: &[ApiItem]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for i in items {
        let k = match i.kind.as_str() {
            "extern" => "extern fn".to_string(),
            "event" => "event".to_string(),
            k => k.to_string(),
        };
        match counts.iter_mut().find(|c| c.0 == k) {
            Some(c) => c.1 += 1,
            None => counts.push((k, 1)),
        }
    }
    if counts.is_empty() {
        return "no public API".into();
    }
    counts.iter().map(|(k, n)| format!("{} {}{}", n, k, if *n == 1 { "" } else { "s" })).collect::<Vec<_>>().join(", ")
}

fn caps_text(caps: &[String]) -> String {
    if caps.is_empty() {
        "none".into()
    } else {
        caps.join(", ")
    }
}

fn open_or_create(dir: &str) -> Result<Project, String> {
    let d = Path::new(dir);
    if !d.join("vard.toml").exists() {
        if !d.is_dir() {
            return Err(format!("{} is not a directory", dir));
        }
        let name = std::fs::canonicalize(d).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "app".into());
        std::fs::write(d.join("vard.toml"), format!("[app]\nname = {}\n", vard_pkg::toml::quote(&name))).map_err(|e| e.to_string())?;
    }
    Project::open(dir)
}

fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (spec, None),
    }
}

/// Persist approved capabilities into `[policy] allow` (the approval becomes a reviewable diff).
fn persist_allow(text: &str, current: &[String], extra: &[String]) -> String {
    let mut all: Vec<String> = current.to_vec();
    for c in extra {
        if !all.contains(c) {
            all.push(c.clone());
        }
    }
    if all.len() == current.len() {
        return text.to_string();
    }
    let list = format!("[{}]", all.iter().map(|c| vard_pkg::toml::quote(c)).collect::<Vec<_>>().join(", "));
    manifest::edit_section(text, "policy", "allow", Some(&list))
}

fn report_install(r: &vard_pkg::project::InstallReport) -> String {
    let mut s = String::new();
    for c in &r.changes {
        s.push_str(&format!("vard.lock: {}\n", c));
    }
    s
}

// ---------------- add / remove / install / update ----------------

pub fn cmd_add(spec: &str, dir: &str, path_dep: Option<&str>, registry: Option<&str>, allow: &[String]) -> Out {
    let (name, req) = split_spec(spec);
    if !manifest::valid_name(name) {
        return fail(Problem::new("P010", format!("`{}` is not a package name (lowercase identifier)", name)), "vard.toml");
    }
    let p = match open_or_create(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    let regs = p.registries();
    let value = match path_dep {
        Some(pd) => format!("{{ path = {} }}", vard_pkg::toml::quote(pd)),
        None => {
            let reg = match regs.get(registry) {
                Ok(r) => r,
                Err(e) => return fail(e, "vard.toml"),
            };
            let entries = match reg.index(name) {
                Ok(e) => e,
                Err(e) => return fail(e, "vard.toml"),
            };
            if entries.is_empty() {
                return fail(resolve::not_found(&reg, name), "vard.toml");
            }
            let req = match req {
                Some(r) => match Req::parse(r) {
                    Ok(r) => r.to_string(),
                    Err(e) => return (e, false),
                },
                None => {
                    let latest = entries.iter().filter(|e| !e.yanked).filter_map(|e| Version::parse(&e.version).ok()).filter(|v| v.pre.is_empty()).max();
                    match latest {
                        Some(v) if v.patch == 0 => format!("{}.{}", v.major, v.minor),
                        Some(v) => v.to_string(),
                        None => return fail(Problem::new("P003", format!("`{}` has no installable version", name)), "vard.toml"),
                    }
                }
            };
            match registry {
                Some(r) => format!("{{ version = {}, registry = {} }}", vard_pkg::toml::quote(&req), vard_pkg::toml::quote(r)),
                None => vard_pkg::toml::quote(&req),
            }
        }
    };
    let old = p.manifest_text.clone();
    let mut text = manifest::set_dependency(&old, name, &value);
    let mpath = p.dir.join("vard.toml");
    if let Err(e) = std::fs::write(&mpath, &text) {
        return (format!("cannot write vard.toml: {}", e), false);
    }
    let restore = || {
        let _ = std::fs::write(&mpath, &old);
    };
    let p2 = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => {
            restore();
            return (e, false);
        }
    };
    let r = match project::install(&p2, allow) {
        Ok(r) => r,
        Err(e) => {
            restore();
            return fail(e, "vard.toml");
        }
    };
    let current = p2.manifest.as_ref().map(|m| m.allow.clone()).unwrap_or_default();
    let used: Vec<String> = allow.iter().filter(|a| r.selected.iter().any(|s| s.caps.contains(a))).cloned().collect();
    let with_policy = persist_allow(&text, &current, &used);
    if with_policy != text {
        text = with_policy;
        let _ = std::fs::write(&mpath, &text);
    }
    let Some(sel) = r.selected.iter().find(|s| s.name == name) else { return (format!("added {}\n", name), true) };
    let src = match (&sel.src, path_dep) {
        (resolve::Src::Path(_), Some(pd)) => format!("path {}", pd),
        (resolve::Src::Path(d), None) => display(d),
        (resolve::Src::Registry(r), _) => r.clone(),
    };
    let mut s = format!("added {} {} ({})\n", sel.name, sel.version, src);
    s.push_str(&format!("  caps: {}{}\n", caps_text(&sel.caps), if used.is_empty() { String::new() } else { format!(" (approved: {} → [policy])", used.join(", ")) }));
    let api_line = match (&sel.entry, &sel.src) {
        (Some(e), _) => api_summary(&e.api),
        (None, resolve::Src::Path(d)) => project::package_files(d).ok().and_then(|f| builtin::entry_from_files(&f).ok()).map(|e| api_summary(&e.api)).unwrap_or_default(),
        _ => String::new(),
    };
    s.push_str(&format!("  api: {} — `vard doc {}`\n", api_line, name));
    s.push_str(&report_install(&r));
    s.push_str("next: vard check\n");
    (s, true)
}

pub fn cmd_remove(name: &str, dir: &str) -> Out {
    let p = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    if !p.manifest.as_ref().is_some_and(|m| m.deps.iter().any(|d| d.name == name)) {
        return (format!("`{}` is not a dependency in {}/vard.toml", name, display(&p.dir)), false);
    }
    let text = manifest::remove_dependency(&p.manifest_text, name);
    if let Err(e) = std::fs::write(p.dir.join("vard.toml"), &text) {
        return (e.to_string(), false);
    }
    match Project::open(dir).map_err(|e| Problem::new("P021", e)).and_then(|p| project::install(&p, &[])) {
        Ok(r) => (format!("removed {}\n{}next: vard check\n", name, report_install(&r)), true),
        Err(e) => fail(e, "vard.toml"),
    }
}

pub fn cmd_install(dir: &str, allow: &[String]) -> Out {
    let p = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    if p.manifest.is_none() {
        return ("no vard.toml here; `vard add NAME` creates one".into(), false);
    }
    match project::install(&p, allow) {
        Ok(r) => {
            let mut s = report_install(&r);
            if s.is_empty() {
                s = format!("up to date: {} package(s)\n", r.lock.pkgs.len());
            }
            let current = p.manifest.as_ref().map(|m| m.allow.clone()).unwrap_or_default();
            let t = persist_allow(&p.manifest_text, &current, allow);
            if t != p.manifest_text {
                let _ = std::fs::write(p.dir.join("vard.toml"), t);
                s.push_str(&format!("[policy] allow += {}\n", allow.join(", ")));
            }
            (s, true)
        }
        Err(e) => fail(e, "vard.toml"),
    }
}

pub fn cmd_update(name: Option<&str>, dir: &str, major: bool) -> Out {
    let p = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    let Some(m) = p.manifest.clone() else { return ("no vard.toml here".into(), false) };
    let regs = p.registries();
    let old_lock = p.lock().unwrap_or_default();
    let mut text = p.manifest_text.clone();
    let mut out = String::new();
    let mut bumped: Vec<(String, String, String, Option<String>)> = Vec::new();
    for d in m.deps.iter().filter(|d| name.is_none_or(|n| n == d.name)) {
        let DepSource::Registry(rname) = &d.source else { continue };
        let Ok(reg) = regs.get(rname.as_deref()) else { continue };
        let entries = match reg.index(&d.name) {
            Ok(e) => e,
            Err(e) => return fail(e, "vard.toml"),
        };
        let cur = d.req.clone().map(|r| r.min).unwrap_or(Version::new(0, 0, 0));
        let best = entries
            .iter()
            .filter(|e| !e.yanked)
            .filter_map(|e| Version::parse(&e.version).ok())
            .filter(|v| v.pre.is_empty() && (major || v.line() == cur.line()))
            .max();
        if let Some(v) = best.filter(|v| *v > cur) {
            let req = if v.patch == 0 { format!("{}.{}", v.major, v.minor) } else { v.to_string() };
            let value = match rname {
                Some(r) => format!("{{ version = {}, registry = {} }}", vard_pkg::toml::quote(&req), vard_pkg::toml::quote(r)),
                None => vard_pkg::toml::quote(&req),
            };
            text = manifest::set_dependency(&text, &d.name, &value);
            bumped.push((d.name.clone(), old_lock.get(&d.name).map(|l| l.version.clone()).unwrap_or_default(), v.to_string(), rname.clone()));
        }
    }
    if bumped.is_empty() {
        let hint = if major { "" } else { " (new major versions: `vard update --major`)" };
        return (format!("everything is at the newest compatible version{}\n", hint), true);
    }
    let mpath = p.dir.join("vard.toml");
    let _ = std::fs::write(&mpath, &text);
    let r = match Project::open(dir).map_err(|e| Problem::new("P021", e)).and_then(|p| project::install(&p, &[])) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::write(&mpath, &p.manifest_text);
            return fail(e, "vard.toml");
        }
    };
    for (n, from, to, rname) in &bumped {
        out.push_str(&format!("~ {} {} -> {}\n", n, if from.is_empty() { "?" } else { from }, to));
        let Ok(reg) = regs.get(rname.as_deref()) else { continue };
        let entries = reg.index(n).unwrap_or_default();
        let api_of = |v: &str| entries.iter().find(|e| e.version == v).map(|e| e.api.clone());
        if let (Some(a), Some(b)) = (api_of(from), api_of(to)) {
            let (bump, lines) = api::diff(&a, &b);
            out.push_str(&format!("  API change: {}\n", bump.name()));
            for l in lines.iter().take(20) {
                out.push_str(&format!("  {}\n", l));
            }
        }
    }
    out.push_str(&report_install(&r));
    out.push_str("next: vard check --fix (applies the packages' migrations), then vard test\n");
    (out, true)
}

// ---------------- discovery ----------------

pub fn cmd_search(query: &str, dir: &str) -> Out {
    let p = Project::open(dir).ok();
    let regs = match &p {
        Some(p) => p.registries(),
        None => Registries::for_project(None, Path::new(dir)),
    };
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    let mut rows: Vec<(i32, String)> = Vec::new();
    let mut errors = String::new();
    for reg in regs.all() {
        let cat = match reg.catalog() {
            Ok(c) => c,
            Err(e) => {
                errors.push_str(&format!("({})\n", e.msg));
                continue;
            }
        };
        for c in cat {
            let hay_name = c.name.to_lowercase();
            let mut score = 0;
            for w in &words {
                if hay_name == *w {
                    score += 10;
                } else if hay_name.contains(w.as_str()) {
                    score += 5;
                }
                if c.keywords.iter().any(|k| k.to_lowercase() == *w) {
                    score += 4;
                }
                if c.summary.to_lowercase().contains(w.as_str()) {
                    score += 2;
                }
                if c.exports.iter().any(|e| e.to_lowercase().contains(w.as_str())) {
                    score += 2;
                }
            }
            if score == 0 && !words.is_empty() {
                continue;
            }
            let exports: Vec<&str> = c.exports.iter().take(6).map(|s| s.as_str()).collect();
            let more = if c.exports.len() > 6 { "…" } else { "" };
            let tests = match c.tests {
                Some((p, 0)) => format!(" · tests {} ✓", p),
                Some((_, f)) => format!(" · {} failing tests", f),
                None => String::new(),
            };
            rows.push((score, format!("{} {} [{}] — {}\n    caps: {} · exports: {}{}{}\n", c.name, c.latest, reg.name, c.summary, caps_text(&c.caps), exports.join(" "), more, tests)));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    if rows.is_empty() {
        return (format!("no packages match `{}`{}\n{}", query, if regs.all().len() == 1 { " (only the builtin registry is configured; add one in vard.toml [registries])" } else { "" }, errors), true);
    }
    let mut s: String = rows.into_iter().take(10).map(|r| r.1).collect();
    s.push_str(&errors);
    s.push_str("details: vard info NAME · install: vard add NAME\n");
    (s, true)
}

fn find_entries(name: &str, dir: &str) -> Result<(String, Vec<IndexEntry>), Problem> {
    let p = Project::open(dir).ok();
    let regs = match &p {
        Some(p) => p.registries(),
        None => Registries::for_project(None, Path::new(dir)),
    };
    let reg = regs.get(None)?;
    let e = reg.index(name)?;
    if !e.is_empty() {
        return Ok((reg.name.clone(), e));
    }
    for r in regs.all() {
        let e = r.index(name).unwrap_or_default();
        if !e.is_empty() {
            return Ok((r.name.clone(), e));
        }
    }
    Err(resolve::not_found(&reg, name))
}

pub fn cmd_info(name: &str, dir: &str) -> Out {
    let (reg, entries) = match find_entries(name, dir) {
        Ok(x) => x,
        Err(e) => return fail(e, "vard.toml"),
    };
    let latest = entries.iter().filter(|e| !e.yanked).max_by(|a, b| Version::parse(&a.version).ok().cmp(&Version::parse(&b.version).ok())).unwrap_or(&entries[entries.len() - 1]);
    let mut s = format!("{} {} [{}] — {}\n", latest.name, latest.version, reg, latest.summary);
    let versions: Vec<String> = entries.iter().map(|e| if e.yanked { format!("{} (yanked)", e.version) } else { e.version.clone() }).collect();
    s.push_str(&format!("versions: {}\n", versions.join(" ")));
    s.push_str(&format!("caps: {}\n", caps_text(&latest.caps)));
    for c in &latest.caps {
        if let Some((_, d)) = api::CAPABILITIES.iter().find(|x| x.0 == c) {
            s.push_str(&format!("  {}: {}\n", c, d));
        }
    }
    if !latest.deps.is_empty() {
        s.push_str(&format!("deps: {}\n", latest.deps.iter().map(|d| format!("{} {}", d.name, d.req)).collect::<Vec<_>>().join(", ")));
    }
    if let Some(v) = &latest.vard {
        s.push_str(&format!("vard: {}\n", v));
    }
    if let Some(a) = &latest.android {
        s.push_str(&format!("android module: {}\n", a));
    }
    match latest.tests {
        Some((p, f)) => s.push_str(&format!("tests at publish: {} passed, {} failed (vard {})\n", p, f, latest.tested_with)),
        None => s.push_str("tests: bundled with the toolchain (verified in its CI)\n"),
    }
    s.push_str(&format!("archive: {} sha256 {} · AGENT.md {}\n", kb(latest.size as usize), &latest.sha256[..16], kb(latest.doc_bytes as usize)));
    s.push_str(&format!("api ({}):\n", api_summary(&latest.api)));
    for l in api::render(&latest.api).lines() {
        s.push_str(&format!("  {}\n", l));
    }
    s.push_str(&format!("install: vard add {}\n", name));
    (s, true)
}

/// `vard doc NAME` for a package: AGENT.md + generated API (+ capabilities).
pub fn package_doc(name: &str, dir: &str) -> Option<String> {
    let p = Project::open(dir).ok();
    // 1. installed in this project
    if let Some(p) = &p {
        if p.manifest.as_ref().is_some_and(|m| m.deps.iter().any(|d| d.name == name)) {
            if let Ok(l) = project::load(p) {
                if let Some(ip) = l.packages.iter().find(|ip| ip.name == name) {
                    let c = compile_loaded(&l);
                    let items = api::api(&c, Some(name));
                    let doc = std::fs::read_to_string(ip.dir.join(project::AGENT_DOC)).unwrap_or_default();
                    return Some(format!(
                        "{}\n## API ({} {}, generated)\n{}caps: {}\n",
                        doc.trim_end(),
                        name,
                        ip.version,
                        api::render(&items),
                        caps_text(&ip.lock.caps)
                    ));
                }
            }
        }
    }
    // 2. bundled with the toolchain
    let files = builtin::files(name);
    if files.is_empty() {
        return None;
    }
    let e = builtin::entry_from_files(&files).ok()?;
    let doc = files.iter().find(|f| f.0 == project::AGENT_DOC).map(|f| String::from_utf8_lossy(&f.1).to_string()).unwrap_or_default();
    Some(format!("{}\n## API ({} {}, generated)\n{}caps: {}\n(not installed here: `vard add {}`)\n", doc.trim_end(), name, e.version, api::render(&e.api), caps_text(&e.caps), name))
}

/// "## Packages in this project" for the cheat sheet.
pub fn packages_section(dir: &str) -> String {
    let Ok(p) = Project::open(dir) else { return String::new() };
    let Some(lock) = p.lock() else { return String::new() };
    if lock.pkgs.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n## Packages in this project (vard.toml; details: `vard doc NAME`)\n");
    let l = project::load(&p).ok();
    for lp in &lock.pkgs {
        let summary = l.as_ref().and_then(|l| l.packages.iter().find(|ip| ip.name == lp.name)).and_then(|ip| ip.manifest.package.as_ref().map(|pk| pk.summary.clone())).unwrap_or_default();
        let direct = if lock.root_deps.contains(&lp.name) { "" } else { " (indirect: not visible to app code)" };
        s.push_str(&format!("{} {} — {}{}\n", lp.name, lp.version, summary, direct));
    }
    s
}

pub fn cmd_api(target: &str, caps_only: bool) -> Out {
    // a package name (installed or builtin) or a project path
    if manifest::valid_name(target) && !Path::new(target).exists() {
        return match package_doc(target, ".") {
            Some(d) => (d, true),
            None => (format!("no package or path `{}`", target), false),
        };
    }
    let l = match load_project(target) {
        Ok(l) => l,
        Err(e) => return (e, false),
    };
    let c = compile_loaded(&l);
    if c.has_errors() {
        return (user_diags(&c), false);
    }
    let caps = api::capabilities(&c, None);
    let mut s = String::new();
    if !caps_only {
        s.push_str(&api::render(&api::api(&c, None)));
    }
    s.push_str(&format!("capabilities: {}\n", if caps.is_empty() { "none".into() } else { caps.keys().cloned().collect::<Vec<_>>().join(", ") }));
    for (cap, why) in &caps {
        s.push_str(&format!("  {}: {}\n", cap, why));
    }
    (s, true)
}

// ---------------- the gate ----------------

pub struct Gate {
    pub entry: Option<IndexEntry>,
    pub blob: Vec<u8>,
    pub report: String,
    pub ok: bool,
}

/// Everything a registry checks before accepting `dir` as a new version.
/// `prior`: what the target registry already has (index of this name, all names with owners).
pub fn gate(dir: &Path, prior: &[IndexEntry], names: &[(String, String)], owner: &str) -> Gate {
    let mut rep = String::new();
    let mut probs: Vec<Problem> = Vec::new();
    let mut warn: Vec<String> = Vec::new();
    let okl = |s: String, rep: &mut String| rep.push_str(&format!("  ok    {}\n", s));
    let dir_s = display(dir);
    let p = match Project::open(&dir_s) {
        Ok(p) => p,
        Err(e) => return Gate { entry: None, blob: vec![], report: e, ok: false },
    };
    let Some(pk) = p.manifest.as_ref().and_then(|m| m.package.clone()) else {
        return Gate { entry: None, blob: vec![], report: format!("{}/vard.toml has no [package] section\n", dir_s), ok: false };
    };
    let m = p.manifest.clone().unwrap();
    let version = pk.version.clone().unwrap_or(Version::new(0, 0, 0));
    rep.push_str(&format!("gate: {} {}\n", pk.name, version));
    // name
    if manifest::RESERVED.contains(&pk.name.as_str()) {
        probs.push(Problem::new("P010", format!("`{}` is a reserved name", pk.name)));
    }
    if let Some((other, o)) = names.iter().find(|(n, o)| *n != pk.name && o != owner && vard_lang::diag::edit_distance(n, &pk.name) <= 1) {
        probs.push(Problem::new("P010", format!("`{}` is one edit away from `{}` (owner {}); pick a distinct name", pk.name, other, o)));
    }
    if let Some((_, o)) = names.iter().find(|(n, o)| *n == pk.name && o != owner) {
        probs.push(Problem::new("P018", format!("`{}` belongs to {}", pk.name, o)));
    }
    for d in m.deps.iter().filter(|d| matches!(d.source, DepSource::Path(_))) {
        probs.push(Problem { code: "P021", line: d.line, msg: format!("path dependency `{}` cannot be published: publish it first, then depend on it by version", d.name), fix: Some(format!("{} = \"1.0\"", d.name)) });
    }
    if pk.summary.trim().is_empty() {
        probs.push(Problem::new("P021", "[package] needs a one-line `summary` (shown in search results)").fix("summary = \"What it is, in one line\""));
    }
    // files
    let files = match project::package_files(dir) {
        Ok(f) => f,
        Err(e) => return Gate { entry: None, blob: vec![], report: format!("{}{}\n", rep, e), ok: false },
    };
    let blob = vpk::pack(&files);
    if blob.len() > vard_pkg::MAX_PACKAGE_BYTES {
        probs.push(Problem::new("P019", format!("archive is {} (limit 8 MB)", kb(blob.len()))));
    }
    match files.iter().find(|f| f.0 == project::AGENT_DOC) {
        None => probs.push(Problem::new("P011", "no AGENT.md (what an agent reads to use the package)")),
        Some(f) if f.1.len() > vard_pkg::AGENT_DOC_BUDGET => probs.push(Problem::new("P011", format!("AGENT.md is {} (budget {})", kb(f.1.len()), kb(vard_pkg::AGENT_DOC_BUDGET)))),
        Some(f) => okl(format!("AGENT.md: {} of {}", kb(f.1.len()), kb(vard_pkg::AGENT_DOC_BUDGET)), &mut rep),
    }
    // compile with dependencies
    let l = match project::load(&p) {
        Ok(l) => l,
        Err(e) => return Gate { entry: None, blob, report: format!("{}{}", rep, e), ok: false },
    };
    let c = compile_loaded(&l);
    let own_files = l.files.iter().filter(|f| !f.0.starts_with('@')).count();
    if c.has_errors() {
        probs.push(Problem::new("P012", format!("does not compile:\n{}", user_diags(&c).trim_end())));
    } else {
        okl(format!("compiles ({} file(s), {} dependenc{})", own_files, l.packages.len(), if l.packages.len() == 1 { "y" } else { "ies" }), &mut rep);
    }
    let items = api::api(&c, None);
    if items.is_empty() {
        warn.push("no public API (names starting with `_` are private)".into());
    }
    // capabilities
    let computed = api::capabilities(&c, None);
    let mut declared = pk.capabilities.clone();
    declared.sort();
    let need: Vec<String> = computed.keys().cloned().collect();
    let missing: Vec<&String> = need.iter().filter(|c| !declared.contains(c)).collect();
    if !missing.is_empty() {
        let ev: Vec<String> = missing.iter().map(|c| format!("{} ({})", c, computed[*c])).collect();
        probs.push(Problem::new("P013", format!("code uses undeclared capabilities: {}", ev.join("; "))).fix(format!("[package] capabilities = [{}]", need.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", "))));
    } else {
        okl(format!("capabilities: {}", caps_text(&need)), &mut rep);
    }
    for d in declared.iter().filter(|d| !need.contains(d)) {
        warn.push(format!("capability `{}` is declared but unused; consumers would have to approve it for nothing", d));
    }
    if need.contains(&"native".to_string()) && m.android.is_none() {
        warn.push("declares externs but has no [native.android]: on Android every call is R0010".into());
    }
    // sim answers
    let externs: Vec<&str> = items.iter().filter(|i| i.kind == "extern").map(|i| i.name.as_str()).collect();
    for (k, _) in &m.sim {
        if !externs.contains(&k.as_str()) {
            probs.push(Problem::new("P017", format!("[sim] {}: the package declares no `extern fn {}`", k, k)));
        }
    }
    for mg in &m.migrations {
        let to = mg.to.split('.').next().unwrap_or(&mg.to);
        if !items.iter().any(|i| i.name == to) {
            warn.push(format!("[[migrate]] {} -> {}: `{}` is not in the API", mg.from, mg.to, to));
        }
    }
    // tests
    let has_tests = files.iter().any(|f| f.0.ends_with(".vtest"));
    let mut tests = None;
    if has_tests {
        let (out, ok) = cmd_test(&dir_s);
        let last = out.lines().last().unwrap_or("").to_string();
        let nums: Vec<u32> = last.split_whitespace().filter_map(|w| w.parse().ok()).collect();
        let (passed, failed) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0));
        tests = Some((passed, failed));
        if ok {
            okl(format!("tests: {} passed (headless)", passed), &mut rep);
        } else {
            probs.push(Problem::new("P014", format!("tests failed:\n{}", out.trim_end())));
        }
    } else {
        warn.push("no .vtest files: consumers will see the package as untested (add example/ with tests)".into());
    }
    // versions and semver
    let mut prev: Vec<(Version, &IndexEntry)> = prior.iter().filter_map(|e| Version::parse(&e.version).ok().map(|v| (v, e))).collect();
    prev.sort_by(|a, b| a.0.cmp(&b.0));
    if prev.iter().any(|(v, _)| *v == version) {
        probs.push(Problem::new("P016", format!("{} {} is already published (versions are immutable)", pk.name, version)));
    } else if let Some((lv, _)) = prev.iter().rev().find(|(v, _)| v.line() == version.line()) {
        if *lv > version {
            probs.push(Problem::new("P016", format!("{} is older than the published {} on the same line", version, lv)));
        }
    }
    if let Some((pv, pe)) = prev.iter().rev().find(|(v, _)| *v < version) {
        let (bump, lines) = api::diff(&pe.api, &items);
        let min = pv.bumped(bump.name());
        let same_line_ok = version >= min || (version.line() != pv.line() && bump == api::Bump::Major) || version.line() > pv.line();
        if !same_line_ok {
            probs.push(Problem::new("P015", format!("API change since {} is {}: {} — the version must be at least {}", pv, bump.name(), lines.join("; "), min)).fix(format!("version = \"{}\"", min)));
        } else {
            let detail = if lines.is_empty() { String::new() } else { format!(": {}", lines.iter().take(6).cloned().collect::<Vec<_>>().join("; ")) };
            okl(format!("semver: {} change vs {}{}", bump.name(), pv, detail), &mut rep);
        }
    } else {
        okl(format!("api: {} (first version)", api_summary(&items)), &mut rep);
    }
    let mut entry = builtin::entry_from_files(&files).ok();
    if let Some(e) = &mut entry {
        // record where each dependency resolved, so consumers fetch it from the same place
        let regs = p.registries();
        for d in e.deps.iter_mut() {
            if let Some(ip) = l.packages.iter().find(|ip| ip.name == d.name) {
                let loc = match ip.lock.src.as_str() {
                    "builtin" => Some("builtin".to_string()),
                    s => s.strip_prefix("reg:").and_then(|r| regs.get(Some(r)).ok()).map(|r| r.location()),
                };
                d.registry = loc.or(d.registry.clone());
            }
        }
        e.api = items;
        e.caps = need;
        e.tests = tests;
        e.owner = owner.to_string();
        e.tested_with = vard_pkg::VARD_VERSION.to_string();
        okl(format!("archive: {} files, {}, sha256 {}", files.len(), kb(blob.len()), &e.sha256[..16]), &mut rep);
    }
    for w in &warn {
        rep.push_str(&format!("  warn  {}\n", w));
    }
    rep.push_str(&render_all(&probs, "vard.toml"));
    let ok = probs.is_empty() && entry.is_some();
    Gate { entry, blob, report: rep, ok }
}

// ---------------- pack / publish / yank ----------------

pub fn cmd_pack(dir: &str, out: Option<&str>) -> Out {
    let p = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    let Some(pk) = p.manifest.as_ref().and_then(|m| m.package.clone()) else { return (format!("{}/vard.toml has no [package]", dir), false) };
    let files = match project::package_files(&p.dir) {
        Ok(f) => f,
        Err(e) => return (e, false),
    };
    let blob = vpk::pack(&files);
    let file = out.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}-{}.vpk", pk.name, pk.version.map(|v| v.to_string()).unwrap_or_default())));
    if let Err(e) = std::fs::write(&file, &blob) {
        return (e.to_string(), false);
    }
    (format!("wrote {}: {} files, {}, sha256 {}\n", display(&file), files.len(), kb(blob.len()), sha256_hex(&blob)), true)
}

fn token(arg: Option<&str>, reg_name: &str) -> Option<String> {
    if let Some(t) = arg {
        return Some(t.to_string());
    }
    if let Ok(t) = std::env::var("VARD_TOKEN") {
        return Some(t);
    }
    let text = std::fs::read_to_string(vard_pkg::net::home().join("credentials.toml")).ok()?;
    let t = vard_pkg::toml::parse(&text).ok()?;
    t.get("tokens")?.as_table()?.get(reg_name)?.as_str().map(String::from)
}

/// The registry to publish to: a local registry root (has private/) or an http(s) URL.
fn publish_target(registry: Option<&str>, p: &Project) -> Result<(String, String), String> {
    let regs = p.registries();
    let r = match registry {
        Some(r) if r.starts_with("http") || Path::new(r).exists() => return Ok((r.to_string(), r.to_string())),
        Some(r) => regs.get(Some(r)).map_err(|e| e.msg)?,
        None => regs.get(None).map_err(|e| e.msg)?,
    };
    if r.is_builtin() {
        return Err("the builtin registry is read-only; publish with --registry DIR|URL (create one: vard registry init DIR)".into());
    }
    Ok((r.name.clone(), r.url.clone()))
}

pub fn cmd_publish(dir: &str, registry: Option<&str>, token_arg: Option<&str>, dry_run: bool) -> Out {
    let p = match Project::open(dir) {
        Ok(p) => p,
        Err(e) => return (e, false),
    };
    let (rname, url) = match publish_target(registry, &p) {
        Ok(x) => x,
        Err(e) => return (e, false),
    };
    let name = p.manifest.as_ref().and_then(|m| m.package.as_ref()).map(|pk| pk.name.clone()).unwrap_or_default();
    if url.starts_with("http") {
        // the server runs the same gate; a dry run gates locally against the server's index
        let reg = Registry::open(&rname, &url);
        let prior = reg.index(&name).unwrap_or_default();
        let names: Vec<(String, String)> = reg.catalog().unwrap_or_default().into_iter().map(|c| (c.name, String::new())).filter(|(n, _)| *n == name).collect();
        let g = gate(&p.dir, &prior, &names, "");
        if !g.ok || dry_run {
            return (format!("{}{}", g.report, if g.ok { "dry run: not published\n" } else { "" }), g.ok);
        }
        let Some(tok) = token(token_arg, &rname) else {
            return fail(Problem::new("P018", "publishing needs a token").fix("--token T, VARD_TOKEN, or ~/.vard/credentials.toml [tokens]"), "vard.toml");
        };
        return match vard_pkg::net::post(&format!("{}/api/v1/publish", url.trim_end_matches('/')), &g.blob, Some(&tok)) {
            Ok((200, body)) => (String::from_utf8_lossy(&body).to_string(), true),
            Ok((s, body)) => (format!("registry answered HTTP {}:\n{}", s, String::from_utf8_lossy(&body)), false),
            Err(e) => fail(Problem::new("P002", e), "vard.toml"),
        };
    }
    let root = PathBuf::from(&url);
    let prior = registry::dir_index(&root, &name);
    let names: Vec<(String, String)> = registry::dir_names(&root).into_iter().map(|n| {
        let o = registry::dir_index(&root, &n).first().map(|e| e.owner.clone()).unwrap_or_default();
        (n, o)
    }).collect();
    let owner = match token(token_arg, &rname) {
        Some(t) => match registry::token_owner(&root, &t) {
            Some(o) => o,
            None => return fail(Problem::new("P018", "the token is not valid for this registry"), "vard.toml"),
        },
        None => prior.first().map(|e| e.owner.clone()).unwrap_or_else(|| "local".into()),
    };
    let g = gate(&p.dir, &prior, &names, &owner);
    if !g.ok || dry_run {
        return (format!("{}{}", g.report, if g.ok { "dry run: not published\n" } else { "" }), g.ok);
    }
    let e = g.entry.unwrap();
    match registry::append(&root, &e, &g.blob) {
        Ok(le) => (format!("{}published {} {} to {} (log #{})\n", g.report, e.name, e.version, rname, le.seq), true),
        Err(x) => (format!("{}{}\n", g.report, x), false),
    }
}

pub fn cmd_yank(spec: &str, registry: Option<&str>, undo: bool) -> Out {
    let (name, Some(ver)) = split_spec(spec) else { return ("usage: vard yank NAME@VERSION --registry DIR [--undo]".into(), false) };
    let Some(r) = registry else { return ("--registry DIR is required (on an http registry: POST /api/v1/yank)".into(), false) };
    match registry::yank(Path::new(r), name, ver, !undo) {
        Ok(()) => (format!("{} {} {}\n", if undo { "restored" } else { "yanked" }, name, ver), true),
        Err(e) => (e, false),
    }
}

// ---------------- bundle (delivery to devices) ----------------

/// Everything a device needs, flattened: app sources, `@pkg/...` sources, vard.lock,
/// fonts/ and assets/, plus bundle.json with a content hash and the *native fingerprint*
/// (externs + native modules). A host may apply an over-the-air bundle only when its
/// fingerprint equals the one the installed binary was built with.
pub fn cmd_bundle(dir: &str, out: &str) -> Out {
    let l = match load_project(dir) {
        Ok(l) => l,
        Err(e) => return (e, false),
    };
    let c = compile_loaded(&l);
    if c.has_errors() {
        return (format!("{}cannot bundle a project with errors\n", user_diags(&c)), false);
    }
    let p = Project::open(dir).unwrap();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let pdir = display(&p.dir);
    for (path, text) in &l.files {
        let rel = if path.starts_with('@') { path.clone() } else { path.strip_prefix(&format!("{}/", pdir)).unwrap_or(path).to_string() };
        files.push((rel, text.as_bytes().to_vec()));
    }
    if let Ok(t) = std::fs::read(p.lock_path()) {
        files.push(("vard.lock".into(), t));
    }
    for sub in ["fonts", "assets"] {
        if let Ok(fs) = project::package_files(&p.dir.join(sub)) {
            for (r, b) in fs {
                files.push((format!("{}/{}", sub, r), b));
            }
        }
    }
    let mut natives: Vec<String> = c.program.externs.iter().filter(|e| e.span.file != 0).map(|e| format!("fn {}", e.name)).collect();
    natives.extend(c.program.events.iter().filter(|e| e.span.file != 0).map(|e| format!("event {}", e.name)));
    for ip in &l.packages {
        if let Some(a) = &ip.manifest.android {
            natives.push(format!("android {} {}", ip.name, a.module));
        }
    }
    natives.sort();
    let fingerprint = sha256_hex(natives.join("\n").as_bytes());
    let content = sha256_hex(&vpk::pack(&files));
    let mut caps: Vec<String> = l.packages.iter().flat_map(|ip| ip.lock.caps.clone()).chain(api::capabilities(&c, None).into_keys()).collect();
    caps.sort();
    caps.dedup();
    let meta = serde_json::json!({
        "vard": vard_pkg::VARD_VERSION, "sha256": content, "native_fingerprint": fingerprint, "natives": natives, "caps": caps,
        "packages": l.packages.iter().map(|ip| serde_json::json!({ "name": ip.name, "version": ip.version, "sha256": ip.lock.sha256 })).collect::<Vec<_>>(),
    });
    let outp = Path::new(out);
    if out.ends_with(".vpk") {
        files.push(("bundle.json".into(), format!("{:#}\n", meta).into_bytes()));
        if let Err(e) = std::fs::write(outp, vpk::pack(&files)) {
            return (e.to_string(), false);
        }
    } else {
        for (rel, b) in &files {
            let fp = outp.join(rel);
            if let Some(d) = fp.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&fp, b) {
                return (format!("cannot write {}: {}", display(&fp), e), false);
            }
        }
        let _ = std::fs::write(outp.join("bundle.json"), format!("{:#}\n", meta));
    }
    (format!("bundled {} files into {} (sha256 {}, native fingerprint {})\n", files.len(), out, &content[..16], &fingerprint[..16]), true)
}

/// For the Android shell: `name<TAB>gradle dir<TAB>module` of packages with native code.
pub fn android_modules(dir: &str) -> Out {
    let l = match load_project(dir) {
        Ok(l) => l,
        Err(e) => return (e, false),
    };
    let mut s = String::new();
    for ip in &l.packages {
        if let Some(a) = &ip.manifest.android {
            if let Some(g) = &a.gradle {
                let d = std::fs::canonicalize(ip.dir.join(g)).unwrap_or(ip.dir.join(g));
                s.push_str(&format!("{}\t{}\t{}\n", ip.name, d.to_string_lossy().replace('\\', "/").trim_start_matches("//?/"), a.module));
            }
        }
    }
    (s, true)
}

// ---------------- registry admin ----------------

pub fn cmd_registry(args: &[String]) -> Out {
    let pos: Vec<&str> = args.iter().map(|s| s.as_str()).filter(|a| !a.starts_with("--")).collect();
    let opt = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).map(|s| s.as_str());
    match pos.first().copied() {
        Some("init") => {
            let Some(dir) = pos.get(1) else { return ("usage: vard registry init DIR [--name NAME]".into(), false) };
            let name = opt("--name").unwrap_or("local");
            match registry::init(Path::new(dir), name) {
                Ok(key) => (
                    format!(
                        "created registry `{}` in {}\n  public/   serve or sync anywhere (static files): vard registry serve {} | any CDN/bucket\n  private/  signing key + tokens — never publish this\n  key: {}\nnext: vard registry token {} OWNER, then vard publish --registry {}\n",
                        name, dir, dir, key, dir, dir
                    ),
                    true,
                ),
                Err(e) => (e, false),
            }
        }
        Some("token") => {
            let (Some(dir), Some(owner)) = (pos.get(1), pos.get(2)) else { return ("usage: vard registry token DIR OWNER".into(), false) };
            match registry::add_token(Path::new(dir), owner) {
                Ok(t) => (format!("token for {} (shown once; store it in ~/.vard/credentials.toml [tokens] or VARD_TOKEN):\n{}\n", owner, t), true),
                Err(e) => (e, false),
            }
        }
        Some("serve") => {
            let Some(dir) = pos.get(1) else { return ("usage: vard registry serve DIR [--addr 127.0.0.1:8470]".into(), false) };
            let addr = opt("--addr").unwrap_or("127.0.0.1:8470");
            match crate::server::serve(Path::new(dir), addr) {
                Ok(()) => (String::new(), true),
                Err(e) => (e, false),
            }
        }
        Some("verify") => {
            let Some(url) = pos.get(1) else { return ("usage: vard registry verify DIR|URL".into(), false) };
            let reg = Registry::open("verify", url);
            let cat = match reg.catalog() {
                Ok(c) => c,
                Err(e) => return fail(e, ""),
            };
            let mut n = 0;
            for c in &cat {
                for e in reg.index(&c.name).unwrap_or_default() {
                    if let Err(p) = reg.blob(&e) {
                        return fail(p, "");
                    }
                    n += 1;
                }
            }
            (format!("ok: {} package(s), {} version(s): signatures, hash chain and every blob verified\n", cat.len(), n), true)
        }
        _ => ("usage: vard registry init DIR [--name N] | token DIR OWNER | serve DIR [--addr HOST:PORT] | verify DIR|URL".into(), false),
    }
}
