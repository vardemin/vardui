//! Package contracts computed by the compiler, never declared by hand:
//!
//! - [`api`]: the public API of a package (or the app) as canonical items. The registry
//!   stores it with every version; [`diff`] classifies a change as major/minor/patch, so
//!   semver is enforced instead of promised, and `vard update` can print exactly what
//!   changed in a dependency.
//! - [`capabilities`]: what the code can reach on the device (secure storage, push,
//!   permissions, native code, network...). The manifest must declare it, the consumer's
//!   policy must allow it, and the lockfile records it — a capability can never appear in
//!   a dependency without a reviewable diff.

use crate::ast::*;
use crate::link::package_of;
use crate::Compilation;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ApiParam {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
    pub bind: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApiItem {
    /// view struct enum fn extern event const token
    pub kind: String,
    pub name: String,
    pub params: Vec<ApiParam>,
    pub ret: Option<String>,
    pub variants: Vec<String>,
}

/// Which files belong to `pkg` (`None` = the app / the package being developed).
fn in_scope(c: &Compilation, file: u16, pkg: Option<&str>) -> bool {
    if file == 0 {
        return false;
    }
    let path = &c.sources.file(file).path;
    match pkg {
        None => package_of(path).is_none(),
        Some(p) => package_of(path) == Some(p),
    }
}

fn plain(name: &str, pkg: Option<&str>) -> String {
    match pkg {
        Some(p) => name.strip_prefix(&format!("{}.", p)).unwrap_or(name).to_string(),
        None => name.to_string(),
    }
}

pub fn type_text(t: &TypeRef, pkg: Option<&str>) -> String {
    match t {
        TypeRef::Named(n, _) => plain(n, pkg),
        TypeRef::List(t) => format!("[{}]", type_text(t, pkg)),
        TypeRef::Map(t) => format!("[String: {}]", type_text(t, pkg)),
        TypeRef::Opt(t) => format!("{}?", type_text(t, pkg)),
        TypeRef::Fn(ps, r) => {
            let ps: Vec<String> = ps.iter().map(|p| type_text(p, pkg)).collect();
            match r {
                Some(r) => format!("fn({}) -> {}", ps.join(", "), type_text(r, pkg)),
                None => format!("fn({})", ps.join(", ")),
            }
        }
    }
}

fn params(c: &Compilation, ps: &[Param], pkg: Option<&str>) -> Vec<ApiParam> {
    ps.iter()
        .map(|p| ApiParam {
            name: p.name.clone(),
            ty: type_text(&p.ty, pkg),
            default: p.default.map(|d| c.sources.text(c.program.span(d)).trim().to_string()),
            bind: p.bind,
        })
        .collect()
}

/// Public API of `pkg` (`None` = the root project), sorted by (kind, name).
pub fn api(c: &Compilation, pkg: Option<&str>) -> Vec<ApiItem> {
    let p = &c.program;
    let mut out = Vec::new();
    let public = |n: &str| !plain(n, pkg).starts_with('_');
    let item = |kind: &str, name: &str| ApiItem { kind: kind.into(), name: plain(name, pkg), params: Vec::new(), ret: None, variants: Vec::new() };
    for v in p.views.iter().filter(|v| in_scope(c, v.span.file, pkg) && public(&v.name)) {
        out.push(ApiItem { params: params(c, &v.params, pkg), ..item("view", &v.name) });
    }
    for s in p.structs.iter().filter(|s| in_scope(c, s.span.file, pkg) && public(&s.name)) {
        out.push(ApiItem { params: params(c, &s.fields, pkg), ..item("struct", &s.name) });
    }
    for e in p.enums.iter().filter(|e| in_scope(c, e.span.file, pkg) && public(&e.name)) {
        out.push(ApiItem { variants: e.variants.clone(), ..item("enum", &e.name) });
    }
    for f in p.fns.iter().filter(|f| in_scope(c, f.span.file, pkg) && public(&f.name)) {
        out.push(ApiItem { params: params(c, &f.params, pkg), ret: f.ret.as_ref().map(|t| type_text(t, pkg)), ..item("fn", &f.name) });
    }
    for f in p.externs.iter().filter(|f| in_scope(c, f.span.file, pkg) && public(&f.name)) {
        out.push(ApiItem { params: params(c, &f.params, pkg), ret: f.ret.as_ref().map(|t| type_text(t, pkg)), ..item("extern", &f.name) });
    }
    for ev in p.events.iter().filter(|e| in_scope(c, e.span.file, pkg) && public(&e.name)) {
        out.push(ApiItem { params: params(c, &ev.params, pkg), ..item("event", &ev.name) });
    }
    for k in p.consts.iter().filter(|k| in_scope(c, k.span.file, pkg) && public(&k.name)) {
        let ty = match &k.ty {
            Some(t) => type_text(t, pkg),
            None => match p.expr(k.value) {
                Expr::Int(_) => "Int".into(),
                Expr::Float(_) => "Float".into(),
                Expr::Str(_) => "String".into(),
                Expr::Bool(_) => "Bool".into(),
                Expr::Color(_) => "Color".into(),
                _ => "_".into(),
            },
        };
        out.push(ApiItem { ret: Some(ty), ..item("const", &k.name) });
    }
    let mut tokens = BTreeSet::new();
    for t in p.themes.iter().filter(|t| in_scope(c, t.span.file, pkg)) {
        for (n, _, _) in &t.entries {
            if n != "font" {
                tokens.insert(n.clone());
            }
        }
    }
    for t in tokens {
        out.push(item("token", &t));
    }
    let order = |k: &str| ["view", "struct", "enum", "fn", "extern", "event", "const", "token"].iter().position(|x| *x == k).unwrap_or(9);
    out.sort_by(|a, b| (order(&a.kind), &a.name).cmp(&(order(&b.kind), &b.name)));
    out
}

fn params_text(ps: &[ApiParam]) -> String {
    ps.iter()
        .map(|p| {
            let mut s = format!("{}{}: {}", if p.bind { "bind " } else { "" }, p.name, p.ty);
            if let Some(d) = &p.default {
                s.push_str(&format!(" = {}", d));
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl ApiItem {
    /// One canonical line in Vard syntax.
    pub fn line(&self) -> String {
        let ret = self.ret.as_ref().map(|r| format!(" -> {}", r)).unwrap_or_default();
        match self.kind.as_str() {
            "view" => format!("view {}({})", self.name, params_text(&self.params)),
            "struct" => format!("struct {} {{ {} }}", self.name, params_text(&self.params)),
            "enum" => format!("enum {} {{ {} }}", self.name, self.variants.join(", ")),
            "fn" => format!("fn {}({}){}", self.name, params_text(&self.params), ret),
            "extern" => format!("extern fn {}({}){}", self.name, params_text(&self.params), ret),
            "event" => format!("extern event {}({})", self.name, params_text(&self.params)),
            "const" => format!("const {}: {}", self.name, self.ret.as_deref().unwrap_or("_")),
            _ => format!("token .{}", self.name),
        }
    }
}

/// The API as compact text: one line per item; theme tokens collapsed into one line.
pub fn render(items: &[ApiItem]) -> String {
    let mut s = String::new();
    let tokens: Vec<&str> = items.iter().filter(|i| i.kind == "token").map(|i| i.name.as_str()).collect();
    for i in items.iter().filter(|i| i.kind != "token") {
        s.push_str(&i.line());
        s.push('\n');
    }
    if !tokens.is_empty() {
        if tokens.len() > 12 {
            s.push_str(&format!("theme tokens ({}): .{} … .{}\n", tokens.len(), tokens[0], tokens[tokens.len() - 1]));
        } else {
            s.push_str(&format!("theme tokens: {}\n", tokens.iter().map(|t| format!(".{}", t)).collect::<Vec<_>>().join(" ")));
        }
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn name(self) -> &'static str {
        match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

/// Smallest semver bump that `old → new` requires, plus one line per change
/// (`+` added, `-` removed, `~` changed; `!` marks a breaking change).
pub fn diff(old: &[ApiItem], new: &[ApiItem]) -> (Bump, Vec<String>) {
    let key = |i: &ApiItem| (i.kind.clone(), i.name.clone());
    let o: BTreeMap<_, _> = old.iter().map(|i| (key(i), i)).collect();
    let n: BTreeMap<_, _> = new.iter().map(|i| (key(i), i)).collect();
    let mut bump = Bump::Patch;
    let mut lines = Vec::new();
    let mut note = |b: Bump, line: String, bump: &mut Bump| {
        *bump = (*bump).max(b);
        lines.push(if b == Bump::Major { format!("! {}", line) } else { line });
    };
    for (k, oi) in &o {
        match n.get(k) {
            None => note(Bump::Major, format!("- {}", oi.line()), &mut bump),
            Some(ni) if ni != oi => {
                let mut changes: Vec<(Bump, String)> = Vec::new();
                for op in &oi.params {
                    match ni.params.iter().find(|p| p.name == op.name) {
                        None => changes.push((Bump::Major, format!("`{}` removed", op.name))),
                        Some(np) => {
                            if np.ty != op.ty {
                                changes.push((Bump::Major, format!("`{}`: {} -> {}", op.name, op.ty, np.ty)));
                            }
                            if np.bind != op.bind {
                                changes.push((Bump::Major, format!("`{}`: bind changed", op.name)));
                            }
                            if op.default.is_some() && np.default.is_none() {
                                changes.push((Bump::Major, format!("`{}` is now required", op.name)));
                            }
                            if op.default.is_none() && np.default.is_some() {
                                changes.push((Bump::Minor, format!("`{}` now has a default", op.name)));
                            }
                        }
                    }
                }
                for np in &ni.params {
                    if !oi.params.iter().any(|p| p.name == np.name) {
                        let b = if np.default.is_some() && oi.kind != "event" { Bump::Minor } else { Bump::Major };
                        changes.push((b, format!("`{}` added{}", np.name, if np.default.is_some() { "" } else { " (required)" })));
                    }
                }
                if oi.params.first().map(|p| &p.name) != ni.params.first().map(|p| &p.name) && oi.kind == "view" {
                    changes.push((Bump::Major, "positional (first) parameter changed".into()));
                }
                if oi.ret != ni.ret {
                    changes.push((Bump::Major, format!("returns {} -> {}", oi.ret.as_deref().unwrap_or("nothing"), ni.ret.as_deref().unwrap_or("nothing"))));
                }
                for v in &oi.variants {
                    if !ni.variants.contains(v) {
                        changes.push((Bump::Major, format!(".{} removed", v)));
                    }
                }
                for v in &ni.variants {
                    if !oi.variants.contains(v) {
                        changes.push((Bump::Minor, format!(".{} added", v)));
                    }
                }
                let b = changes.iter().map(|c| c.0).max().unwrap_or(Bump::Patch);
                let what: Vec<String> = changes.into_iter().map(|c| c.1).collect();
                let line = if what.is_empty() { format!("~ {} {}: defaults changed", ni.kind, ni.name) } else { format!("~ {} {}: {}", ni.kind, ni.name, what.join(", ")) };
                note(b, line, &mut bump);
            }
            _ => {}
        }
    }
    for (k, ni) in &n {
        if !o.contains_key(k) {
            note(Bump::Minor, format!("+ {}", ni.line()), &mut bump);
        }
    }
    (bump, lines)
}

// ---------------- capabilities ----------------

/// Every capability a package can need, with what it means for the person approving it.
pub const CAPABILITIES: &[(&str, &str)] = &[
    ("prefs", "reads/writes app preferences (`pref`, prefGet/prefSet)"),
    ("secure", "reads/writes the Keychain / Keystore (secureGet/secureSet)"),
    ("permissions", "asks for runtime permissions (camera, location, contacts...)"),
    ("push", "push token, notifications, push events"),
    ("links", "opens URLs in other apps (openUrl)"),
    ("share", "opens the system share sheet (shareText)"),
    ("deeplinks", "receives deep links (`on deepLink`)"),
    ("network", "loads remote resources (https:// URLs in code)"),
    ("native", "declares extern fn/event: runs native code shipped with the package"),
];

/// Capabilities a consumer must approve explicitly in `[policy] allow`.
pub const SENSITIVE: &[&str] = &["secure", "permissions", "push", "native", "network"];

const STD_CAPS: &[(&str, &str)] = &[
    ("prefGet", "prefs"),
    ("prefSet", "prefs"),
    ("prefRemove", "prefs"),
    ("secureGet", "secure"),
    ("secureSet", "secure"),
    ("secureRemove", "secure"),
    ("permissionStatus", "permissions"),
    ("requestPermission", "permissions"),
    ("openAppSettings", "permissions"),
    ("getPushToken", "push"),
    ("showNotification", "push"),
    ("openUrl", "links"),
    ("shareText", "share"),
];

const EVENT_CAPS: &[(&str, &str)] = &[("pushReceived", "push"), ("pushOpened", "push"), ("pushTokenChanged", "push"), ("deepLink", "deeplinks")];

/// Capabilities the code of `pkg` (`None` = root) actually uses: `cap -> first evidence`
/// (`secureGet @ path:line`). Computed from the checked program, so it cannot be forgotten.
pub fn capabilities(c: &Compilation, pkg: Option<&str>) -> BTreeMap<String, String> {
    let p = &c.program;
    let mut caps: BTreeMap<String, String> = BTreeMap::new();
    let mut add = |cap: &str, why: String| {
        caps.entry(cap.to_string()).or_insert(why);
    };
    for (i, e) in p.exprs.iter().enumerate() {
        let sp = p.expr_spans[i];
        if !in_scope(c, sp.file, pkg) {
            continue;
        }
        match e {
            Expr::Ident(n) => {
                if let Some((_, cap)) = STD_CAPS.iter().find(|(f, _)| f == n) {
                    add(cap, format!("{} @ {}", n, c.sources.loc(sp)));
                }
            }
            Expr::Str(parts) => {
                if let Some(StrPart::Lit(s)) = parts.first() {
                    if s.starts_with("https://") || s.starts_with("http://") {
                        add("network", format!("\"{}\" @ {}", s.chars().take(40).collect::<String>(), c.sources.loc(sp)));
                    }
                }
            }
            _ => {}
        }
    }
    for v in p.views.iter().filter(|v| in_scope(c, v.span.file, pkg)) {
        for m in &v.members {
            match m {
                ViewMember::Pref { name, span, .. } => add("prefs", format!("pref {} @ {}", name, c.sources.loc(*span))),
                ViewMember::On { event, event_span, .. } => {
                    if let Some((_, cap)) = EVENT_CAPS.iter().find(|(e, _)| e == event) {
                        add(cap, format!("on {} @ {}", event, c.sources.loc(*event_span)));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(f) = p.externs.iter().find(|f| in_scope(c, f.span.file, pkg)) {
        add("native", format!("extern fn {} @ {}", f.name, c.sources.loc(f.span)));
    } else if let Some(e) = p.events.iter().find(|e| in_scope(c, e.span.file, pkg)) {
        add("native", format!("extern event {} @ {}", e.name, c.sources.loc(e.span)));
    }
    caps
}
