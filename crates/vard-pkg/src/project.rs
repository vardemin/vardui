//! A project on disk: its sources, its manifest, and its dependencies made ready for the
//! compiler. Package files enter compilation as `@name/<path>` (short, stable paths in
//! diagnostics: `@charts/line.vard:12`), the dependency graph and package migrations as a
//! `LinkInfo`.
//!
//! Loading is self-healing: when vard.toml and vard.lock disagree (an agent just added a
//! dependency by editing vard.toml), `load` resolves and installs first and says so in one
//! line — the same thing `cargo build` does — instead of failing with "run install".

use crate::lock::{Lock, LockPkg};
use crate::manifest::Manifest;
use crate::registry::api_hash;
use crate::resolve::{self, Registries, Selected, Src};
use crate::{net, vpk, Problem};
use std::path::{Path, PathBuf};
use vard_lang::link::{LinkInfo, PkgLink};

pub const AGENT_DOC: &str = "AGENT.md";
/// Top-level directories of a package that are not part of its code.
pub const NON_SOURCE_DIRS: &[&str] = &["example", "examples", "native", "skills", "tests"];

fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || name == "target" || name == "build" || name == "node_modules"
}

/// Is `rel` (a path inside a package) one of the package's `.vard` sources?
pub fn is_package_source(rel: &str) -> bool {
    let first = rel.split('/').next().unwrap_or("");
    rel.ends_with(".vard") && !(rel.contains('/') && NON_SOURCE_DIRS.contains(&first)) && !rel.split('/').any(skip_dir)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if p.is_dir() {
            if !skip_dir(&name) {
                walk(root, &p, out);
            }
        } else if !name.ends_with(".vpk") && name != "vard.lock" {
            out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Every file that goes into the package archive (relative paths, sorted).
pub fn package_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut rels = Vec::new();
    walk(dir, dir, &mut rels);
    rels.into_iter()
        .map(|r| {
            if !vpk::safe_path(&r) {
                return Err(format!("{}: file names in a package may not contain spaces or `:`", r));
            }
            std::fs::read(dir.join(&r)).map(|b| (r.clone(), b)).map_err(|e| format!("cannot read {}: {}", r, e))
        })
        .collect()
}

pub fn display(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

pub struct Project {
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub manifest_text: String,
    /// a single .vard file was given instead of a directory
    pub single_file: Option<PathBuf>,
}

impl Project {
    pub fn open(path: &str) -> Result<Project, String> {
        let p = Path::new(path);
        let (dir, single) = if p.is_file() {
            (p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf(), Some(p.to_path_buf()))
        } else if p.is_dir() {
            (p.to_path_buf(), None)
        } else {
            return Err(format!("{} does not exist", display(p)));
        };
        let mpath = dir.join("vard.toml");
        let (manifest, text) = match std::fs::read_to_string(&mpath) {
            Ok(t) => {
                let t = t.replace("\r\n", "\n");
                match Manifest::parse(&t) {
                    Ok(m) => (Some(m), t),
                    Err(ps) => return Err(crate::render_all(&ps, &display(&mpath))),
                }
            }
            Err(_) => (None, String::new()),
        };
        Ok(Project { dir, manifest, manifest_text: text, single_file: single })
    }

    pub fn is_package(&self) -> bool {
        self.manifest.as_ref().is_some_and(|m| m.package.is_some())
    }

    pub fn lock_path(&self) -> PathBuf {
        self.dir.join("vard.lock")
    }

    pub fn lock(&self) -> Option<Lock> {
        std::fs::read_to_string(self.lock_path()).ok().map(|t| Lock::parse(&t))
    }

    pub fn registries(&self) -> Registries {
        Registries::for_project(self.manifest.as_ref(), &self.dir)
    }

    /// The project's own `.vard` files as `(display path, text)`.
    pub fn sources(&self) -> Result<Vec<(String, String)>, String> {
        let mut rels = Vec::new();
        match &self.single_file {
            Some(f) => rels.push(f.strip_prefix(&self.dir).unwrap_or(f).to_string_lossy().replace('\\', "/")),
            None => walk(&self.dir, &self.dir, &mut rels),
        }
        let pkg = self.is_package();
        let mut out = Vec::new();
        for r in rels.into_iter().filter(|r| r.ends_with(".vard") && (!pkg || is_package_source(r))) {
            let p = self.dir.join(&r);
            let text = std::fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {}", display(&p), e))?;
            out.push((display(&p), text.replace("\r\n", "\n")));
        }
        if out.is_empty() {
            return Err(format!("no .vard files in {}", display(&self.dir)));
        }
        Ok(out)
    }
}

pub struct InstalledPkg {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub lock: LockPkg,
}

pub struct Loaded {
    pub files: Vec<(String, String)>,
    pub link: Option<LinkInfo>,
    /// default headless answers `(extern, json)` from packages' [sim]
    pub sims: Vec<(String, String)>,
    pub packages: Vec<InstalledPkg>,
    /// one-line notes for the user (e.g. "installed tailwind 1.0.0")
    pub notes: Vec<String>,
}

fn src_string(s: &Selected, project_dir: &Path, regs: &Registries) -> String {
    match &s.src {
        Src::Path(p) => {
            let rel = pathdiff(p, project_dir);
            format!("path:{}", rel)
        }
        Src::Git { url, commit, reference, dir, .. } => crate::git::src_string(url, commit, reference, dir.as_deref()),
        Src::Registry(r) => {
            if regs.get(Some(r)).is_ok_and(|x| x.is_builtin()) {
                "builtin".into()
            } else {
                format!("reg:{}", r)
            }
        }
    }
}

/// `to` relative to `from` (both directories), `/`-separated.
fn pathdiff(to: &Path, from: &Path) -> String {
    let a = std::fs::canonicalize(to).unwrap_or(to.to_path_buf());
    let b = std::fs::canonicalize(from).unwrap_or(from.to_path_buf());
    let ac: Vec<_> = a.components().collect();
    let bc: Vec<_> = b.components().collect();
    let common = ac.iter().zip(&bc).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return display(&a);
    }
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), bc.len() - common).collect();
    parts.extend(ac[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

fn cache_dir_for(name: &str, version: &str, sha: &str) -> PathBuf {
    net::cache().join("pkgs").join(format!("{}-{}-{}", name, version, &sha[..12.min(sha.len())]))
}

/// Make the archive `sha` available as an extracted directory in the cache.
fn extract(name: &str, version: &str, sha: &str, blob: Option<Vec<u8>>, fetch: impl FnOnce() -> Result<Vec<u8>, Problem>) -> Result<PathBuf, Problem> {
    let dir = cache_dir_for(name, version, sha);
    if dir.join(".complete").exists() {
        return Ok(dir);
    }
    let bpath = net::cache().join("blobs").join(format!("{}.vpk", sha));
    let data = match blob.or_else(|| std::fs::read(&bpath).ok().filter(|b| crate::sha256_hex(b) == sha)) {
        Some(b) => b,
        None => fetch()?,
    };
    if crate::sha256_hex(&data) != sha {
        return Err(Problem::new("P004", format!("{} {}: archive hash differs from vard.lock", name, version)));
    }
    let io = |e: std::io::Error| Problem::new("P022", format!("cannot write the package cache ({}): {}", net::cache().display(), e));
    std::fs::create_dir_all(bpath.parent().unwrap()).map_err(io)?;
    std::fs::write(&bpath, &data).map_err(io)?;
    let files = vpk::unpack(&data).map_err(|e| Problem::new("P004", format!("{} {}: {}", name, version, e)))?;
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, bytes) in files {
        let p = dir.join(&rel);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(io)?;
        std::fs::write(&p, bytes).map_err(io)?;
    }
    std::fs::write(dir.join(".complete"), sha).map_err(io)?;
    Ok(dir)
}

pub struct InstallReport {
    pub lock: Lock,
    pub changes: Vec<String>,
    pub selected: Vec<Selected>,
}

/// Resolve vard.toml, check the capability policy, fetch + verify every package, write
/// vard.lock, install package skills. `allow` adds capabilities for this run (`--allow`).
pub fn install(p: &Project, allow: &[String]) -> Result<InstallReport, Problem> {
    install_with(p, &InstallOpts { allow, ..Default::default() })
}

/// [`install`], re-resolving the git dependencies named in `refresh` instead of reusing the
/// commits vard.lock pins (`vard update`).
pub fn install_refreshing(p: &Project, allow: &[String], refresh: &[String]) -> Result<InstallReport, Problem> {
    install_with(p, &InstallOpts { allow, refresh, ..Default::default() })
}

#[derive(Default)]
pub struct InstallOpts<'a> {
    /// capabilities approved for this run (`--allow`)
    pub allow: &'a [String],
    /// git dependencies to re-resolve (`vard update`)
    pub refresh: &'a [String],
    /// install packages that are still in quarantine (`--allow-new`, a person's decision)
    pub allow_new: bool,
}

/// A package from a remote registry whose first version is younger than the quarantine.
fn quarantined(reg: &crate::registry::Registry, name: &str) -> Option<String> {
    if !reg.is_remote() {
        return None;
    }
    let first = reg.index(name).ok()?.iter().filter_map(|e| crate::parse_rfc3339(&e.published)).min()?;
    let age = crate::now_secs().saturating_sub(first);
    (age < crate::QUARANTINE_SECS).then(|| crate::rfc3339(first))
}

pub fn install_with(p: &Project, o: &InstallOpts) -> Result<InstallReport, Problem> {
    let (allow, refresh) = (o.allow, o.refresh);
    let m = p.manifest.clone().unwrap_or_default();
    let regs = p.registries();
    let old = p.lock().unwrap_or_default();
    let selected = resolve::resolve(&p.dir, &m, &regs, &old, refresh)?;
    let mut allowed = m.allow.clone();
    allowed.extend(allow.iter().cloned());
    let bad = resolve::policy_violations(&selected, &allowed);
    if !bad.is_empty() {
        return Err(resolve::policy_problem(&bad, &m.allow));
    }
    let mut lock = Lock { root_deps: m.deps.iter().map(|d| d.name.clone()).collect(), pkgs: Vec::new() };
    for s in &selected {
        let mut lp = LockPkg { name: s.name.clone(), version: s.version.to_string(), src: src_string(s, &p.dir, &regs), caps: s.caps.clone(), deps: s.deps.clone(), ..Default::default() };
        if let (Some(e), Src::Registry(r)) = (&s.entry, &s.src) {
            let reg = regs.get(Some(r))?;
            if e.yanked {
                return Err(Problem::new("P009", format!("{} {} is yanked", e.name, e.version)));
            }
            if !o.allow_new && old.get(&e.name).is_none() {
                if let Some(since) = quarantined(&reg, &e.name) {
                    return Err(Problem::new("P024", format!("`{}` is new (first published {}, less than 72 hours ago) — fresh packages are confirmed by a person", e.name, since))
                        .fix(format!("check `vard info {}` (owner, provenance), then: vard add {} --allow-new", e.name, e.name)));
                }
            }
            reg.check_logged(e)?;
            extract(&e.name, &e.version, &e.sha256, None, || reg.blob(e))?;
            lp.sha256 = Some(e.sha256.clone());
            lp.api = Some(api_hash(&e.api));
        }
        if let Src::Git { path, .. } = &s.src {
            lp.sha256 = Some(crate::git::content_hash(path)?);
        }
        lock.pkgs.push(lp);
    }
    let changes = Lock::changes(&old, &lock);
    std::fs::write(p.lock_path(), lock.render()).map_err(|e| Problem::new("P022", format!("cannot write vard.lock: {}", e)))?;
    sync_agent_files(p, &lock, &old);
    Ok(InstallReport { lock, changes, selected })
}

fn lock_is_current(p: &Project, m: &Manifest, lock: &Lock) -> bool {
    let mut want: Vec<&str> = m.deps.iter().map(|d| d.name.as_str()).collect();
    let mut have: Vec<&str> = lock.root_deps.iter().map(|s| s.as_str()).collect();
    want.sort();
    have.sort();
    if want != have {
        return false;
    }
    let git_ok = |lp: &LockPkg| {
        crate::git::parse_src(&lp.src).is_some_and(|g| {
            let export = net::cache().join("git-src").join(format!("{}-{}", &crate::sha256_hex(g.url.as_bytes())[..16], &g.commit[..12.min(g.commit.len())]));
            export.join(".complete").exists()
        })
    };
    // a git dependency whose URL/reference changed in vard.toml must be re-resolved
    let git_same = m.deps.iter().all(|d| match &d.source {
        crate::manifest::DepSource::Git { url, reference, dir } => lock.get(&d.name).and_then(|lp| crate::git::parse_src(&lp.src)).is_some_and(|g| g.url == crate::git::absolute_url(url, &p.dir) && g.reference == reference.describe() && g.dir == *dir),
        _ => true,
    });
    if !git_same {
        return false;
    }
    lock.pkgs.iter().all(|lp| match lp.src.strip_prefix("path:") {
        // a path package: same version and dependencies as its manifest now says
        Some(rel) => Manifest::parse(&std::fs::read_to_string(p.dir.join(rel).join("vard.toml")).unwrap_or_default())
            .ok()
            .and_then(|pm| pm.package.map(|pk| (pk.version.map(|v| v.to_string()) == Some(lp.version.clone()), pm.deps)))
            .is_some_and(|(same, deps)| same && deps.len() == lp.deps.len() && deps.iter().all(|d| lp.deps.contains(&d.name))),
        None if lp.src.starts_with("git:") => git_ok(lp),
        None => lp.sha256.as_ref().is_some_and(|s| cache_dir_for(&lp.name, &lp.version, s).join(".complete").exists() || lp.src == "builtin"),
    })
}

/// The checked-out package directory of a git package, verified against vard.lock.
fn git_dir(lp: &LockPkg) -> Result<PathBuf, Problem> {
    let g = crate::git::parse_src(&lp.src).ok_or_else(|| Problem::new("P022", format!("vard.lock: bad git source for {}", lp.name)))?;
    let (_, export) = crate::git::checkout(&g.url, &crate::manifest::GitRef::Rev(g.commit.clone()), Some(&g.commit))?;
    let dir = match &g.dir {
        Some(d) => export.join(d),
        None => export,
    };
    if let Some(want) = &lp.sha256 {
        let got = crate::git::content_hash(&dir)?;
        if &got != want {
            return Err(Problem::new("P004", format!("{} {}: the files of commit {} differ from vard.lock", lp.name, lp.version, &g.commit[..12.min(g.commit.len())])));
        }
    }
    Ok(dir)
}

/// The project's sources plus its packages, ready for `vard_lang::compile_linked`.
pub fn load(p: &Project) -> Result<Loaded, String> {
    let mut files = p.sources()?;
    let Some(m) = p.manifest.clone().filter(|m| !m.deps.is_empty()) else {
        return Ok(Loaded { files, link: None, sims: Vec::new(), packages: Vec::new(), notes: Vec::new() });
    };
    let mut notes = Vec::new();
    let lock = match p.lock() {
        Some(l) if lock_is_current(p, &m, &l) => l,
        _ => {
            let r = install(p, &[]).map_err(|e| e.render("vard.toml"))?;
            for c in &r.changes {
                notes.push(format!("vard.lock: {}", c));
            }
            r.lock
        }
    };
    let regs = p.registries();
    let mut packages = Vec::new();
    for lp in &lock.pkgs {
        let dir = match lp.src.strip_prefix("path:") {
            Some(rel) => p.dir.join(rel),
            None if lp.src.starts_with("git:") => git_dir(lp).map_err(|e| e.render("vard.lock"))?,
            None => {
                let sha = lp.sha256.clone().ok_or_else(|| format!("vard.lock: {} has no sha256; run `vard install`", lp.name))?;
                let reg_name = lp.src.strip_prefix("reg:").unwrap_or("builtin");
                extract(&lp.name, &lp.version, &sha, None, || {
                    let reg = regs.get(Some(reg_name))?;
                    let e = reg.index(&lp.name)?.into_iter().find(|e| e.version == lp.version).ok_or_else(|| Problem::new("P003", format!("{} {} is no longer in registry `{}`", lp.name, lp.version, reg_name)))?;
                    reg.blob(&e)
                })
                .map_err(|e| e.render("vard.lock"))?
            }
        };
        let mtext = std::fs::read_to_string(dir.join("vard.toml")).map_err(|_| format!("package {}: no vard.toml in {}", lp.name, display(&dir)))?;
        let pm = Manifest::parse(&mtext.replace("\r\n", "\n")).map_err(|ps| crate::render_all(&ps, &format!("@{}/vard.toml", lp.name)))?;
        let mut rels = Vec::new();
        walk(&dir, &dir, &mut rels);
        for r in rels.into_iter().filter(|r| is_package_source(r)) {
            let text = std::fs::read_to_string(dir.join(&r)).map_err(|e| format!("cannot read @{}/{}: {}", lp.name, r, e))?;
            files.push((format!("@{}/{}", lp.name, r), text.replace("\r\n", "\n")));
        }
        packages.push(InstalledPkg { name: lp.name.clone(), version: lp.version.clone(), dir, manifest: pm, lock: lp.clone() });
    }
    let link = LinkInfo {
        root_deps: Some(lock.root_deps.clone()),
        packages: packages
            .iter()
            .map(|ip| PkgLink { name: ip.name.clone(), version: ip.version.clone(), deps: Some(ip.lock.deps.clone()), migrations: ip.manifest.migrations.clone() })
            .collect(),
    };
    let sims = packages.iter().flat_map(|ip| ip.manifest.sim.clone()).collect();
    Ok(Loaded { files, link: Some(link), sims, packages, notes })
}

/// Package skills → `.claude/skills/<pkg>-<skill>/`, and the `<!-- vard:packages -->` block
/// of AGENTS.md: an agent opening the project sees its packages without a tool call.
fn sync_agent_files(p: &Project, lock: &Lock, old: &Lock) {
    let skills_root = p.dir.join(".claude").join("skills");
    for o in &old.pkgs {
        if lock.get(&o.name).is_none() {
            if let Ok(rd) = std::fs::read_dir(&skills_root) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with(&format!("{}-", o.name)) {
                        let _ = std::fs::remove_dir_all(e.path());
                    }
                }
            }
        }
    }
    let mut lines = Vec::new();
    for lp in &lock.pkgs {
        let dir = match lp.src.strip_prefix("path:") {
            Some(rel) => p.dir.join(rel),
            None if lp.src.starts_with("git:") => match git_dir(lp) {
                Ok(d) => d,
                Err(_) => continue,
            },
            None => match &lp.sha256 {
                Some(s) => cache_dir_for(&lp.name, &lp.version, s),
                None => continue,
            },
        };
        let summary = Manifest::parse(&std::fs::read_to_string(dir.join("vard.toml")).unwrap_or_default().replace("\r\n", "\n"))
            .ok()
            .and_then(|m| m.package.map(|pk| pk.summary))
            .unwrap_or_default();
        lines.push(format!("- `{}` {} — {} (`vard doc {}`)", lp.name, lp.version, summary, lp.name));
        if let Ok(rd) = std::fs::read_dir(dir.join("skills")) {
            for e in rd.flatten() {
                let skill = e.path().join("SKILL.md");
                if let Ok(text) = std::fs::read_to_string(&skill) {
                    let to = skills_root.join(format!("{}-{}", lp.name, e.file_name().to_string_lossy()));
                    let _ = std::fs::create_dir_all(&to);
                    let _ = std::fs::write(to.join("SKILL.md"), text);
                }
            }
        }
    }
    let agents = p.dir.join("AGENTS.md");
    if let Ok(text) = std::fs::read_to_string(&agents) {
        const OPEN: &str = "<!-- vard:packages -->";
        const CLOSE: &str = "<!-- /vard:packages -->";
        if let (Some(a), Some(b)) = (text.find(OPEN), text.find(CLOSE)) {
            if a < b {
                let body = if lines.is_empty() { "No packages yet: `vard search WORDS`, then `vard add NAME`.".to_string() } else { lines.join("\n") };
                let new = format!("{}{}\n{}\n{}", &text[..a], OPEN, body, &text[b..]);
                if new != text {
                    let _ = std::fs::write(&agents, new);
                }
            }
        }
    }
}
