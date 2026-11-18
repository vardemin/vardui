//! The `builtin` registry: packages shipped inside the toolchain (`packages/` in the
//! framework repository). Works offline and needs no signature — the bytes are part of
//! the `vard` binary. Its index is computed the way a registry gate computes one: the
//! API and capabilities come from compiling the package.

use crate::manifest::Manifest;
use crate::project::{is_package_source, AGENT_DOC};
use crate::registry::{DepSpec, IndexEntry};
use crate::{sha256_hex, vpk, VARD_VERSION};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/builtin_files.rs"));

pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&str> = FILES.iter().map(|f| f.0).collect();
    v.dedup();
    v
}

pub fn files(pkg: &str) -> Vec<(String, Vec<u8>)> {
    FILES.iter().filter(|f| f.0 == pkg).map(|f| (f.1.to_string(), f.2.to_vec())).collect()
}

pub fn blob(pkg: &str) -> Option<Vec<u8>> {
    let f = files(pkg);
    (!f.is_empty()).then(|| vpk::pack(&f))
}

/// Index entry computed from files (used for builtin packages and by the publish gate).
pub fn entry_from_files(files: &[(String, Vec<u8>)]) -> Result<IndexEntry, String> {
    let text = |p: &str| files.iter().find(|f| f.0 == p).map(|f| String::from_utf8_lossy(&f.1).replace("\r\n", "\n"));
    let m = Manifest::parse(&text("vard.toml").ok_or("no vard.toml")?).map_err(|p| crate::render_all(&p, "vard.toml"))?;
    let pk = m.package.clone().ok_or("vard.toml has no [package]")?;
    let sources: Vec<(String, String)> =
        files.iter().filter(|f| is_package_source(&f.0)).map(|f| (f.0.clone(), String::from_utf8_lossy(&f.1).replace("\r\n", "\n"))).collect();
    let c = vard_lang::compile(&sources);
    let api = vard_lang::api::api(&c, None);
    let caps: Vec<String> = vard_lang::api::capabilities(&c, None).into_keys().collect();
    let blob = vpk::pack(files);
    Ok(IndexEntry {
        name: pk.name.clone(),
        version: pk.version.map(|v| v.to_string()).unwrap_or_default(),
        sha256: sha256_hex(&blob),
        size: blob.len() as u64,
        vard: pk.vard.map(|r| r.to_string()),
        deps: m
            .deps
            .iter()
            .filter_map(|d| match &d.source {
                crate::manifest::DepSource::Registry(r) => Some(DepSpec { name: d.name.clone(), req: d.req.as_ref().map(|r| r.to_string()).unwrap_or_default(), registry: r.clone() }),
                crate::manifest::DepSource::Path(_) | crate::manifest::DepSource::Git { .. } => None,
            })
            .collect(),
        caps,
        android: m.android.map(|a| a.module),
        summary: pk.summary,
        keywords: pk.keywords,
        api,
        doc_bytes: text(AGENT_DOC).map(|t| t.len() as u64).unwrap_or(0),
        tests: None,
        tested_with: VARD_VERSION.to_string(),
        owner: "vard".into(),
        yanked: false,
        published: String::new(),
        provenance: None,
    })
}

pub fn entries() -> Vec<IndexEntry> {
    static E: OnceLock<Vec<IndexEntry>> = OnceLock::new();
    E.get_or_init(|| names().into_iter().filter_map(|n| entry_from_files(&files(n)).ok()).collect()).clone()
}
