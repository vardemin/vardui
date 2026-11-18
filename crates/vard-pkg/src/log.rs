//! The registry's append-only, signed publication log — trust comes from math, not from
//! the host, so a registry can be served from any CDN, bucket or mirror.
//!
//! ```text
//! log.txt    1 tailwind 1.0.0 <blob sha256> <chain>
//!            2 charts 0.3.0 <blob sha256> <chain>       chain_n = sha256(chain_{n-1} " " seq name version sha)
//! log.head   <size> <chain_size> <ed25519 signature of "vard-log/1 <size> <chain_size>">
//! ```
//! A client (1) pins the registry key on first contact (`known_registries`, like SSH),
//! (2) checks the head signature, (3) recomputes the chain, (4) checks that the head it saw
//! last time is a prefix of the new log (history was not rewritten) and (5) only installs
//! a blob whose `(name, version, sha256)` is in the log. A compromised mirror can withhold
//! updates but cannot change or fork what was published.

use crate::sha256_hex;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

pub const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub seq: u64,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub chain: String,
}

pub fn chain_next(prev: &str, seq: u64, name: &str, version: &str, sha: &str) -> String {
    sha256_hex(format!("{} {} {} {} {}", prev, seq, name, version, sha).as_bytes())
}

/// Parse and verify the hash chain; returns the entries.
pub fn parse(text: &str) -> Result<Vec<LogEntry>, String> {
    let mut out = Vec::new();
    let mut prev = ZERO.to_string();
    for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 5 {
            return Err(format!("log line {} is malformed", i + 1));
        }
        let seq: u64 = f[0].parse().map_err(|_| format!("log line {}: bad sequence number", i + 1))?;
        if seq != i as u64 + 1 {
            return Err(format!("log line {}: sequence {} out of order", i + 1, seq));
        }
        let want = chain_next(&prev, seq, f[1], f[2], f[3]);
        if want != f[4] {
            return Err(format!("log line {} ({} {}): hash chain broken — the log was altered", seq, f[1], f[2]));
        }
        prev = want.clone();
        out.push(LogEntry { seq, name: f[1].into(), version: f[2].into(), sha256: f[3].into(), chain: want });
    }
    Ok(out)
}

pub fn line(e: &LogEntry) -> String {
    format!("{} {} {} {} {}\n", e.seq, e.name, e.version, e.sha256, e.chain)
}

pub fn head_message(size: u64, chain: &str) -> String {
    format!("vard-log/1 {} {}", size, chain)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// New registry key: (secret seed hex, public key hex).
pub fn keygen() -> Result<(String, String), String> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("no OS randomness: {}", e))?;
    let sk = SigningKey::from_bytes(&seed);
    Ok((hex(&seed), hex(sk.verifying_key().as_bytes())))
}

/// The public key of a secret seed.
pub fn public_of(secret_hex: &str) -> Result<String, String> {
    let seed: [u8; 32] = unhex(secret_hex.trim()).and_then(|v| v.try_into().ok()).ok_or("bad registry key file")?;
    Ok(hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()))
}

/// What the old key signs to hand the registry over to the new key.
pub fn rotation_message(old: &str, new: &str) -> String {
    format!("vard-key-rotation/1 {} {}", old, new)
}

/// Follow signed rotations (`(from, to, sig)`, each signed by `from`) from `trusted` to
/// `current`. `true` when `current` is `trusted` or reachable through valid signatures.
pub fn rotation_chain_ok(trusted: &str, current: &str, rotations: &[(String, String, String)]) -> bool {
    let mut cur = trusted.to_string();
    for _ in 0..=rotations.len() {
        if cur == current {
            return true;
        }
        match rotations.iter().find(|(f, t, s)| *f == cur && verify(f, &rotation_message(f, t), s)) {
            Some((_, t, _)) => cur = t.clone(),
            None => return false,
        }
    }
    false
}

pub fn sign(secret_hex: &str, msg: &str) -> Result<String, String> {
    let seed: [u8; 32] = unhex(secret_hex.trim()).and_then(|v| v.try_into().ok()).ok_or("bad registry key file")?;
    Ok(hex(&SigningKey::from_bytes(&seed).sign(msg.as_bytes()).to_bytes()))
}

pub fn verify(pub_hex: &str, msg: &str, sig_hex: &str) -> bool {
    let (Some(pk), Some(sig)) = (unhex(pub_hex).and_then(|v| <[u8; 32]>::try_from(v).ok()), unhex(sig_hex).and_then(|v| <[u8; 64]>::try_from(v).ok())) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else { return false };
    vk.verify(msg.as_bytes(), &Signature::from_bytes(&sig)).is_ok()
}

/// Check a log against its signed head. Returns the entries.
pub fn verify_log(log_text: &str, head_text: &str, pub_hex: &str) -> Result<Vec<LogEntry>, String> {
    let entries = parse(log_text)?;
    let h: Vec<&str> = head_text.split_whitespace().collect();
    if h.len() != 3 {
        return Err("log.head is malformed".into());
    }
    let size: u64 = h[0].parse().map_err(|_| "log.head: bad size")?;
    if !verify(pub_hex, &head_message(size, h[1]), h[2]) {
        return Err("log.head signature does not match the registry key".into());
    }
    if size as usize != entries.len() {
        return Err(format!("log.head says {} entries, log.txt has {}", size, entries.len()));
    }
    let last = entries.last().map(|e| e.chain.as_str()).unwrap_or(ZERO);
    if last != h[1] {
        return Err("log.head does not match the end of log.txt".into());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_rotation_chain() {
        let (s1, p1) = keygen().unwrap();
        let (s2, p2) = keygen().unwrap();
        let (_, p3) = keygen().unwrap();
        let r12 = (p1.clone(), p2.clone(), sign(&s1, &rotation_message(&p1, &p2)).unwrap());
        let r23 = (p2.clone(), p3.clone(), sign(&s2, &rotation_message(&p2, &p3)).unwrap());
        assert!(rotation_chain_ok(&p1, &p3, &[r23.clone(), r12.clone()]));
        assert!(rotation_chain_ok(&p2, &p3, &[r12.clone(), r23.clone()]));
        // a rotation signed by the wrong key does not count
        let forged = (p1.clone(), p3.clone(), sign(&s2, &rotation_message(&p1, &p3)).unwrap());
        assert!(!rotation_chain_ok(&p1, &p3, &[forged]));
        assert!(!rotation_chain_ok(&p3, &p1, &[r12, r23]));
    }

    #[test]
    fn signed_chain() {
        let (sk, pk) = keygen().unwrap();
        let e1 = LogEntry { seq: 1, name: "a".into(), version: "1.0.0".into(), sha256: "aa".into(), chain: chain_next(ZERO, 1, "a", "1.0.0", "aa") };
        let e2 = LogEntry { seq: 2, name: "b".into(), version: "0.1.0".into(), sha256: "bb".into(), chain: chain_next(&e1.chain, 2, "b", "0.1.0", "bb") };
        let log = format!("{}{}", line(&e1), line(&e2));
        let head = format!("2 {} {}", e2.chain, sign(&sk, &head_message(2, &e2.chain)).unwrap());
        assert_eq!(verify_log(&log, &head, &pk).unwrap().len(), 2);
        // tampering with a published hash breaks the chain
        assert!(verify_log(&log.replace(" aa ", " ab "), &head, &pk).is_err());
        // another key cannot sign heads
        let (_, other) = keygen().unwrap();
        assert!(verify_log(&log, &head, &other).is_err());
    }
}
