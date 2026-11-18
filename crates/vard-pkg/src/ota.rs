//! Over-the-air update channels: the same static, signed, content-addressed idea as a
//! registry, for app bundles (`vard bundle`).
//!
//! ```text
//! channel/
//!   public/config.json                 {"vard_ota": 1, "key": "<ed25519 public key>"}
//!   public/bundles/<sha256>.vpk        immutable bundles
//!   public/<fingerprint>/latest.json   {"sha256", "size", "version", "published", "sig"}
//!   private/ota.key                    the signing key (keep it out of the hosting)
//! ```
//! A channel is addressed by the **native fingerprint** (`bundle.json`): a device asks only
//! for bundles built against exactly its native code, so an update can never need a native
//! function the installed binary lacks. `latest.json` is signed over
//! `vard-ota/1 <fingerprint> <sha256> <version>`; the device checks that signature with the
//! key compiled into the app, the bundle hash, and that the bundle compiles — else it keeps
//! the bundle it shipped with.

use crate::{log, sha256_hex, vpk};
use serde_json::{json, Value};
use std::path::Path;

pub fn message(fingerprint: &str, sha256: &str, version: &str) -> String {
    format!("vard-ota/1 {} {} {}", fingerprint, sha256, version)
}

/// Create a channel in `root`; returns the public key to compile into the app.
pub fn init(root: &Path) -> Result<String, String> {
    if root.join("public").join("config.json").exists() {
        return Err(format!("{} is already an OTA channel", root.display()));
    }
    for d in [root.join("public").join("bundles"), root.join("private")] {
        std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {}", d.display(), e))?;
    }
    let (secret, key) = log::keygen()?;
    std::fs::write(root.join("private").join("ota.key"), secret).map_err(|e| e.to_string())?;
    std::fs::write(root.join("public").join("config.json"), format!("{}\n", json!({ "vard_ota": 1, "key": key }))).map_err(|e| e.to_string())?;
    Ok(key)
}

/// Publish a bundle archive (`.vpk` from `vard bundle`) to its fingerprint's channel.
/// Returns `(fingerprint, sha256)`.
pub fn publish(root: &Path, bundle: &[u8], version: &str, now: &str) -> Result<(String, String), String> {
    let files = vpk::unpack(bundle)?;
    let meta: Value = files
        .iter()
        .find(|f| f.0 == "bundle.json")
        .and_then(|f| serde_json::from_slice(&f.1).ok())
        .ok_or("the archive has no bundle.json: build it with `vard bundle PATH --out FILE.vpk`")?;
    let fp = meta.get("native_fingerprint").and_then(|v| v.as_str()).ok_or("bundle.json has no native_fingerprint")?.to_string();
    let secret = std::fs::read_to_string(root.join("private").join("ota.key")).map_err(|_| format!("{}: no private/ota.key (create a channel: vard ota init DIR)", root.display()))?;
    let sha = sha256_hex(bundle);
    let public = root.join("public");
    std::fs::write(public.join("bundles").join(format!("{}.vpk", sha)), bundle).map_err(|e| e.to_string())?;
    let sig = log::sign(&secret, &message(&fp, &sha, version))?;
    let latest = json!({ "sha256": sha, "size": bundle.len(), "version": version, "published": now, "sig": sig });
    std::fs::create_dir_all(public.join(&fp)).map_err(|e| e.to_string())?;
    std::fs::write(public.join(&fp).join("latest.json"), format!("{:#}\n", latest)).map_err(|e| e.to_string())?;
    Ok((fp, sha))
}

/// Check an update the device downloaded: `latest` (the channel's latest.json), the bundle
/// bytes, the app's compiled-in channel key and its own native fingerprint. Returns the
/// bundle's files on success; the caller still compiles them before switching.
/// `(path, bytes)` of every file in a bundle.
pub type Files = Vec<(String, Vec<u8>)>;

pub fn verify(latest: &str, bundle: &[u8], key: &str, fingerprint: &str) -> Result<(String, Files), String> {
    let l: Value = serde_json::from_str(latest).map_err(|_| "latest.json is not JSON")?;
    let field = |k: &str| l.get(k).and_then(|v| v.as_str()).map(String::from).ok_or(format!("latest.json has no `{}`", k));
    let (sha, version, sig) = (field("sha256")?, field("version")?, field("sig")?);
    if !log::verify(key, &message(fingerprint, &sha, &version), &sig) {
        return Err("signature does not match this app's channel key and native fingerprint".into());
    }
    if sha256_hex(bundle) != sha {
        return Err("bundle hash differs from latest.json".into());
    }
    let files = vpk::unpack(bundle)?;
    let meta: Value = files.iter().find(|f| f.0 == "bundle.json").and_then(|f| serde_json::from_slice(&f.1).ok()).ok_or("the bundle has no bundle.json")?;
    if meta.get("native_fingerprint").and_then(|v| v.as_str()) != Some(fingerprint) {
        return Err("the bundle was built for different native code".into());
    }
    Ok((version, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_roundtrip() {
        let dir = std::env::temp_dir().join(format!("vard-ota-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = init(&dir).unwrap();
        let bundle = vpk::pack(&[("app.vard".into(), b"view App { Text(\"v2\") }\n".to_vec()), ("bundle.json".into(), br#"{"native_fingerprint":"fp1"}"#.to_vec())]);
        let (fp, sha) = publish(&dir, &bundle, "2", "2026-11-19T00:00:00Z").unwrap();
        assert_eq!(fp, "fp1");
        let latest = std::fs::read_to_string(dir.join("public/fp1/latest.json")).unwrap();
        let (v, files) = verify(&latest, &bundle, &key, "fp1").unwrap();
        assert_eq!((v.as_str(), files.len()), ("2", 2));
        assert!(verify(&latest, &bundle, &key, "fp2").is_err(), "another app's native code");
        let mut tampered = bundle.clone();
        tampered.push(b'x');
        assert!(verify(&latest, &tampered, &key, "fp1").is_err());
        let (_, other) = log::keygen().unwrap();
        assert!(verify(&latest, &bundle, &other, "fp1").is_err());
        assert_eq!(sha.len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
