//! `vard.lock` — line-oriented, sorted, one package per line, so a diff reads like a
//! review: a new `caps=` value is a new permission, a new `sha256=` is new code.
//!
//! ```text
//! # vard.lock — written by `vard install`. Commit it; review caps= changes like code.
//! lock 1
//! root deps=firebase,tailwind
//! pkg firebase 1.0.0 src=builtin sha256=9f2c… api=41ab09c2 caps=native,push deps=
//! pkg tailwind 1.0.0 src=path:../../packages/tailwind caps= deps=
//! ```
//! The lock never decides versions (minimal version selection already makes resolution
//! reproducible from vard.toml alone); it pins content hashes and records what each
//! package can do. The compiler reads `root`/`pkg ... deps=` to link packages.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LockPkg {
    pub name: String,
    pub version: String,
    /// `builtin` | `reg:NAME` | `path:DIR`
    pub src: String,
    pub sha256: Option<String>,
    pub api: Option<String>,
    pub caps: Vec<String>,
    pub deps: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lock {
    pub root_deps: Vec<String>,
    pub pkgs: Vec<LockPkg>,
}

fn esc(s: &str) -> String {
    s.replace('%', "%25").replace(' ', "%20")
}
fn unesc(s: &str) -> String {
    s.replace("%20", " ").replace("%25", "%")
}

impl Lock {
    pub fn parse(text: &str) -> Lock {
        let mut l = Lock::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            match it.next() {
                Some("root") => {
                    for kv in it {
                        if let Some(d) = kv.strip_prefix("deps=") {
                            l.root_deps = d.split(',').filter(|s| !s.is_empty()).map(String::from).collect();
                        }
                    }
                }
                Some("pkg") => {
                    let (Some(name), Some(version)) = (it.next(), it.next()) else { continue };
                    let mut p = LockPkg { name: name.into(), version: version.into(), ..Default::default() };
                    for kv in it {
                        let Some((k, v)) = kv.split_once('=') else { continue };
                        let list = || v.split(',').filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>();
                        match k {
                            "src" => p.src = unesc(v),
                            "sha256" => p.sha256 = Some(v.into()),
                            "api" => p.api = Some(v.into()),
                            "caps" => p.caps = list(),
                            "deps" => p.deps = list(),
                            _ => {}
                        }
                    }
                    l.pkgs.push(p);
                }
                _ => {}
            }
        }
        l
    }

    pub fn render(&self) -> String {
        let mut s = String::from("# vard.lock — written by `vard install`. Commit it; review caps= changes like code.\nlock 1\n");
        let mut root = self.root_deps.clone();
        root.sort();
        s.push_str(&format!("root deps={}\n", root.join(",")));
        let mut pkgs = self.pkgs.clone();
        pkgs.sort_by(|a, b| a.name.cmp(&b.name));
        for p in pkgs {
            s.push_str(&format!("pkg {} {} src={}", p.name, p.version, esc(&p.src)));
            if let Some(h) = &p.sha256 {
                s.push_str(&format!(" sha256={}", h));
            }
            if let Some(a) = &p.api {
                s.push_str(&format!(" api={}", a));
            }
            let mut caps = p.caps.clone();
            caps.sort();
            let mut deps = p.deps.clone();
            deps.sort();
            s.push_str(&format!(" caps={} deps={}\n", caps.join(","), deps.join(",")));
        }
        s
    }

    pub fn get(&self, name: &str) -> Option<&LockPkg> {
        self.pkgs.iter().find(|p| p.name == name)
    }

    /// What changed between two locks, one line each; `!` marks what a reviewer must look at
    /// (a new capability, or new content under an old version).
    pub fn changes(old: &Lock, new: &Lock) -> Vec<String> {
        let mut out = Vec::new();
        for p in &new.pkgs {
            match old.get(&p.name) {
                None => out.push(format!("+ {} {}{}", p.name, p.version, caps_note(&p.caps))),
                Some(o) => {
                    let added: Vec<String> = p.caps.iter().filter(|c| !o.caps.contains(c)).cloned().collect();
                    if !added.is_empty() {
                        out.push(format!("! {} now also needs: {}", p.name, added.join(", ")));
                    }
                    if o.version != p.version {
                        out.push(format!("~ {} {} -> {}", p.name, o.version, p.version));
                    } else if o.sha256 != p.sha256 && o.sha256.is_some() && p.sha256.is_some() {
                        out.push(format!("! {} {}: content hash changed under the same version", p.name, p.version));
                    }
                }
            }
        }
        for o in &old.pkgs {
            if new.get(&o.name).is_none() {
                out.push(format!("- {} {}", o.name, o.version));
            }
        }
        out
    }
}

fn caps_note(caps: &[String]) -> String {
    if caps.is_empty() {
        String::new()
    } else {
        format!(" (caps: {})", caps.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let l = Lock {
            root_deps: vec!["b".into(), "a".into()],
            pkgs: vec![LockPkg { name: "a".into(), version: "1.0.0".into(), src: "path:../my dir".into(), caps: vec!["push".into()], ..Default::default() }],
        };
        let t = l.render();
        assert!(t.contains("root deps=a,b"));
        assert!(t.contains("src=path:../my%20dir"));
        let back = Lock::parse(&t);
        assert_eq!(back.pkgs[0].src, "path:../my dir");
        assert_eq!(back.root_deps, vec!["a", "b"]);
        // the compiler reads the same file
        let info = vard_lang::link::LinkInfo::from_lock(&t);
        assert_eq!(info.root_deps.unwrap(), vec!["a", "b"]);
    }
}
