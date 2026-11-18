//! Semantic versions and requirements.
//!
//! A requirement names a *minimum*: `"1.4"` = `^1.4.0` = at least 1.4.0 within the 1.x
//! line. Minimal version selection (see `resolve`) picks the smallest version that
//! satisfies everyone, so the upper bound only decides compatibility, never which
//! version gets installed. Like Cargo, `0.x` lines are separate: `^0.3` is `0.3.*`.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// pre-release tag (`beta.1`); empty for releases
    pub pre: String,
}

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version { major, minor, patch, pre: String::new() }
    }

    pub fn parse(s: &str) -> Result<Version, String> {
        let s = s.trim().trim_start_matches('v');
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.to_string()),
            None => (s, String::new()),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("`{}` is not a version; write MAJOR.MINOR.PATCH, e.g. \"1.0.0\"", s));
        }
        let n = |x: &str| x.parse::<u64>().map_err(|_| format!("`{}` is not a version; write MAJOR.MINOR.PATCH, e.g. \"1.0.0\"", s));
        Ok(Version { major: n(parts[0])?, minor: n(parts[1])?, patch: n(parts[2])?, pre })
    }

    /// Versions on one compatibility line can replace each other: `1.*`, or `0.3.*`.
    pub fn line(&self) -> (u64, u64) {
        if self.major > 0 {
            (self.major, 0)
        } else {
            (0, self.minor)
        }
    }

    pub fn line_name(&self) -> String {
        if self.major > 0 {
            format!("{}.x", self.major)
        } else {
            format!("0.{}.x", self.minor)
        }
    }

    /// The next version for a bump of `kind` ("major" | "minor" | "patch"), `0.x`-aware.
    pub fn bumped(&self, kind: &str) -> Version {
        match (kind, self.major) {
            ("major", 0) => Version::new(0, self.minor + 1, 0),
            ("major", m) => Version::new(m + 1, 0, 0),
            ("minor", 0) => Version::new(0, self.minor, self.patch + 1),
            ("minor", m) => Version::new(m, self.minor + 1, 0),
            _ => Version::new(self.major, self.minor, self.patch + 1),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Version) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(o.major, o.minor, o.patch)).then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.pre.cmp(&o.pre),
        })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Version) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `^1.2` (default): >= 1.2.0 on the same compatibility line
    Caret,
    /// `=1.2.3`
    Exact,
    /// `>=0.2` (used for the `vard` core version)
    AtLeast,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Req {
    pub op: Op,
    pub min: Version,
}

impl Req {
    pub fn parse(s: &str) -> Result<Req, String> {
        let s = s.trim();
        let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
            (Op::AtLeast, r)
        } else if let Some(r) = s.strip_prefix('=') {
            (Op::Exact, r)
        } else if let Some(r) = s.strip_prefix('^') {
            (Op::Caret, r)
        } else {
            (Op::Caret, s)
        };
        let rest = rest.trim();
        let dots = rest.split('-').next().unwrap_or("").matches('.').count();
        let full = match dots {
            0 => format!("{}.0.0", rest),
            1 => format!("{}.0", rest),
            _ => rest.to_string(),
        };
        let min = Version::parse(&full).map_err(|_| format!("`{}` is not a version requirement; write e.g. \"1.4\" (= at least 1.4.0, below 2.0.0)", s))?;
        Ok(Req { op, min })
    }

    pub fn exact(v: &Version) -> Req {
        Req { op: Op::Exact, min: v.clone() }
    }

    pub fn matches(&self, v: &Version) -> bool {
        match self.op {
            Op::Exact => v == &self.min,
            Op::AtLeast => v >= &self.min,
            Op::Caret => v >= &self.min && v.line() == self.min.line() && (v.pre.is_empty() || !self.min.pre.is_empty()),
        }
    }
}

impl fmt::Display for Req {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = &self.min;
        match self.op {
            Op::Exact => write!(f, "={}", v),
            Op::AtLeast => write!(f, ">={}", v),
            Op::Caret if v.patch == 0 && v.pre.is_empty() => write!(f, "{}.{}", v.major, v.minor),
            Op::Caret => write!(f, "{}", v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_lines() {
        let r = Req::parse("1.4").unwrap();
        assert!(r.matches(&Version::parse("1.4.0").unwrap()));
        assert!(r.matches(&Version::parse("1.9.2").unwrap()));
        assert!(!r.matches(&Version::parse("2.0.0").unwrap()));
        assert!(!r.matches(&Version::parse("1.3.9").unwrap()));
        let z = Req::parse("0.3").unwrap();
        assert!(z.matches(&Version::parse("0.3.7").unwrap()));
        assert!(!z.matches(&Version::parse("0.4.0").unwrap()));
        assert!(Req::parse(">=0.1").unwrap().matches(&Version::parse("3.0.0").unwrap()));
        assert!(Version::parse("1.0.0-beta").unwrap() < Version::parse("1.0.0").unwrap());
        assert_eq!(Req::parse("1.4").unwrap().to_string(), "1.4");
        assert_eq!(Version::parse("0.3.1").unwrap().bumped("major").to_string(), "0.4.0");
    }
}
