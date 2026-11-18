//! Git sources: `pkg = { git = URL, tag | rev | branch = "...", dir = "sub/dir" }`.
//!
//! The system `git` does the transport (any URL git understands: https, ssh, a local path).
//! A repository is cloned once into `cache/git/<hash of URL>`; the commit a reference
//! resolves to is pinned in vard.lock together with the content hash of the package files,
//! so a branch never moves under a project until `vard update` — the same reproducibility
//! a registry version gives, without a registry.

use crate::manifest::GitRef;
use crate::{net, sha256_hex, Problem};
use std::path::{Path, PathBuf};
use std::process::Command;

fn problem(msg: impl Into<String>) -> Problem {
    Problem::new("P025", msg)
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut c = Command::new("git");
    c.args(args).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let out = c.output().map_err(|e| format!("git dependencies need `git` on PATH ({})", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("git failed").to_string())
    }
}

fn key(url: &str) -> String {
    sha256_hex(url.as_bytes())[..16].to_string()
}

/// The local clone of `url` (created on first use).
fn clone(url: &str) -> Result<PathBuf, Problem> {
    let repo = net::cache().join("git").join(key(url));
    if !repo.join(".git").exists() {
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.parent().unwrap()).map_err(|e| problem(e.to_string()))?;
        git(&["clone", "--quiet", "--no-checkout", url, &repo.to_string_lossy()], None).map_err(|e| problem(format!("cannot clone {}: {}", url, e)))?;
    }
    Ok(repo)
}

fn resolve_in(repo: &Path, spec: &str) -> Option<String> {
    git(&["rev-parse", "--verify", "--quiet", &format!("{}^{{commit}}", spec)], Some(repo)).ok().filter(|s| !s.is_empty())
}

/// Resolve `reference` (or reuse `locked`, the commit vard.lock already pins) and export that
/// commit. Returns `(commit, directory with the checked-out tree)`.
pub fn checkout(url: &str, reference: &GitRef, locked: Option<&str>) -> Result<(String, PathBuf), Problem> {
    let repo = clone(url)?;
    let want = locked.map(|c| c.to_string()).unwrap_or_else(|| reference.spec());
    let commit = match resolve_in(&repo, &want) {
        Some(c) if locked.is_some() => c,
        _ => {
            git(&["fetch", "--quiet", "--tags", "--force", "origin"], Some(&repo)).map_err(|e| problem(format!("cannot fetch {}: {}", url, e)))?;
            let _ = git(&["remote", "set-head", "origin", "--auto"], Some(&repo));
            resolve_in(&repo, &want).ok_or_else(|| problem(format!("{}: no {} in {}", url, locked.map(|c| format!("commit {}", c)).unwrap_or_else(|| reference.describe()), url)))?
        }
    };
    let export = net::cache().join("git-src").join(format!("{}-{}", key(url), &commit[..12]));
    if !export.join(".complete").exists() {
        let _ = std::fs::remove_dir_all(&export);
        std::fs::create_dir_all(&export).map_err(|e| problem(e.to_string()))?;
        let git_dir = repo.join(".git");
        git(&["--git-dir", &git_dir.to_string_lossy(), "--work-tree", &export.to_string_lossy(), "checkout", "--quiet", "-f", &commit, "--", "."], None)
            .map_err(|e| problem(format!("cannot check out {} of {}: {}", &commit[..12], url, e)))?;
        std::fs::write(export.join(".complete"), &commit).map_err(|e| problem(e.to_string()))?;
    }
    Ok((commit, export))
}

/// `git:URL#COMMIT?ref=tag=v1&dir=sub` (the vard.lock `src=` of a git package).
pub fn src_string(url: &str, commit: &str, reference: &GitRef, dir: Option<&str>) -> String {
    let mut s = format!("git:{}#{}?ref={}", url, commit, reference.describe());
    if let Some(d) = dir {
        s.push_str(&format!("&dir={}", d));
    }
    s
}

pub struct GitSrc {
    pub url: String,
    pub commit: String,
    pub reference: String,
    pub dir: Option<String>,
}

pub fn parse_src(src: &str) -> Option<GitSrc> {
    let rest = src.strip_prefix("git:")?;
    let (url, tail) = rest.rsplit_once('#')?;
    let (commit, query) = tail.split_once('?').unwrap_or((tail, ""));
    let mut reference = String::new();
    let mut dir = None;
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        if let Some(r) = kv.strip_prefix("ref=") {
            reference = r.to_string();
        } else if let Some(d) = kv.strip_prefix("dir=") {
            dir = Some(d.to_string());
        }
    }
    Some(GitSrc { url: url.to_string(), commit: commit.to_string(), reference, dir })
}

/// A relative local path in `git = "../repo"` is relative to the manifest's directory.
pub fn absolute_url(url: &str, base: &Path) -> String {
    let remote = url.contains("://") || url.starts_with("git@") || Path::new(url).is_absolute();
    if remote {
        url.to_string()
    } else {
        let p = base.join(url);
        let s = std::fs::canonicalize(&p).unwrap_or(p).to_string_lossy().replace('\\', "/");
        s.strip_prefix("//?/").unwrap_or(&s).to_string()
    }
}

/// Content hash of a package directory (what vard.lock pins next to the commit).
pub fn content_hash(dir: &Path) -> Result<String, Problem> {
    let files = crate::project::package_files(dir).map_err(problem)?;
    Ok(sha256_hex(&crate::vpk::pack(&files)))
}
