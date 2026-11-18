//! `.vpk` — the package archive. Deterministic by construction (sorted paths, no
//! timestamps, no permissions, no compression), so the same sources give the same SHA-256
//! on any machine and any CI. Transport compression is the HTTP layer's job.
//!
//! ```text
//! VPK1\n
//! F <path> <len>\n<bytes>\n     (one entry per file, paths sorted, `/`-separated, relative)
//! ```

pub fn safe_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && !p.contains(' ')
        && p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

pub fn pack(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut files: Vec<&(String, Vec<u8>)> = files.iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = b"VPK1\n".to_vec();
    for (p, data) in files {
        out.extend_from_slice(format!("F {} {}\n", p, data.len()).as_bytes());
        out.extend_from_slice(data);
        out.push(b'\n');
    }
    out
}

pub fn unpack(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut rest = data.strip_prefix(b"VPK1\n").ok_or("not a .vpk archive")?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let nl = rest.iter().position(|b| *b == b'\n').ok_or("truncated .vpk header")?;
        let header = std::str::from_utf8(&rest[..nl]).map_err(|_| "bad .vpk header")?;
        let mut parts = header.splitn(3, ' ');
        let (Some("F"), Some(path), Some(len)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("bad .vpk entry `{}`", header));
        };
        let len: usize = len.parse().map_err(|_| "bad .vpk length")?;
        if !safe_path(path) {
            return Err(format!("unsafe path in archive: `{}`", path));
        }
        let body = rest.get(nl + 1..nl + 1 + len).ok_or("truncated .vpk entry")?;
        out.push((path.to_string(), body.to_vec()));
        rest = rest.get(nl + 2 + len..).ok_or("truncated .vpk entry")?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_roundtrip() {
        let a = pack(&[("b.vard".into(), b"x\n".to_vec()), ("a/c.md".into(), vec![0, 1, 10])]);
        let b = pack(&[("a/c.md".into(), vec![0, 1, 10]), ("b.vard".into(), b"x\n".to_vec())]);
        assert_eq!(a, b);
        let files = unpack(&a).unwrap();
        assert_eq!(files[0].0, "a/c.md");
        assert_eq!(files[1].1, b"x\n");
        assert!(unpack(&pack(&[("../evil".into(), vec![])])).is_err());
    }
}
