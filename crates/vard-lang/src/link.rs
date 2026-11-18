//! Package linking: namespaces without imports.
//!
//! A project is the app (the *root*), the std library (file 0) and packages. Files of a
//! package arrive with paths `@name/...`; the dependency graph comes from `vard.lock`
//! (or a [`LinkInfo`] built by the CLI). Nothing in the language changes for code that
//! does not collide: a name written without qualification resolves, in order, to
//!
//! 1. a declaration of the same package (the app is its own package),
//! 2. the one direct dependency that exports it (two → `E0231`, qualify `pkg.Name`),
//! 3. std.
//!
//! Names starting with `_` are private to their package (`E0230`); a package that is not a
//! direct dependency is invisible (`E0232`) — so adding a package can never change what
//! another package's code means. Declarations whose names collide across packages are
//! renamed to `pkg.Name` before checking; the checker and the runtime only ever see
//! unique names. Native names (`extern fn` / `extern event`) are global by nature — the
//! platform calls them by name — so two packages declaring one is an error (`E0233`).
//!
//! Package migrations (`[[migrate]]` in a package's vard.toml) turn uses of renamed
//! names/params into `E0234` diagnostics with a machine fix: `vard check --fix` upgrades
//! the consumer's code after `vard update`.

use crate::ast::*;
use crate::diag::{closest, Diag, Sources, Span};
use crate::registry;
use std::collections::{HashMap, HashSet};

/// `from` → `to` rename shipped by a package; `View.param` form renames an argument.
#[derive(Clone, Debug, PartialEq)]
pub struct Migration {
    pub since: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Default)]
pub struct PkgLink {
    pub name: String,
    pub version: String,
    /// `None` = unknown (no lockfile): every other package is visible.
    pub deps: Option<Vec<String>>,
    pub migrations: Vec<Migration>,
}

#[derive(Clone, Debug, Default)]
pub struct LinkInfo {
    /// Direct dependencies of the app; `None` = every package is visible.
    pub root_deps: Option<Vec<String>>,
    pub packages: Vec<PkgLink>,
}

impl LinkInfo {
    /// Read the dependency graph from `vard.lock` (lines `root deps=a,b` and
    /// `pkg NAME VERSION ... deps=x,y`); everything else in the file is ignored here.
    pub fn from_lock(text: &str) -> LinkInfo {
        let mut info = LinkInfo::default();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let deps_of = |rest: &[&str]| -> Option<Vec<String>> {
                rest.iter().find_map(|kv| kv.strip_prefix("deps=")).map(|d| d.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
            };
            match it.next() {
                Some("root") => {
                    let rest: Vec<&str> = it.collect();
                    info.root_deps = Some(deps_of(&rest).unwrap_or_default());
                }
                Some("pkg") => {
                    let (Some(name), Some(version)) = (it.next(), it.next()) else { continue };
                    let rest: Vec<&str> = it.collect();
                    info.packages.push(PkgLink { name: name.into(), version: version.into(), deps: Some(deps_of(&rest).unwrap_or_default()), migrations: Vec::new() });
                }
                _ => {}
            }
        }
        info
    }
}

/// Package name of a source path (`@tailwind/tailwind.vard` → `tailwind`).
pub fn package_of(path: &str) -> Option<&str> {
    path.strip_prefix('@').map(|r| r.split('/').next().unwrap_or(r))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Owner {
    Std,
    Root,
    Pkg(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DK {
    View,
    Struct,
    Enum,
    Fn,
    Extern,
    Event,
    Const,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Type,
    View,
    Value,
}

impl Class {
    fn accepts(self, k: DK) -> bool {
        match self {
            Class::Type => matches!(k, DK::Struct | DK::Enum),
            Class::View => k == DK::View,
            Class::Value => matches!(k, DK::Fn | DK::Extern | DK::Const | DK::Struct),
        }
    }
}

struct Decl {
    kind: DK,
    owner: Owner,
    span: Span,
}

struct Linker<'a> {
    pkgs: Vec<PkgLink>,
    file_owner: Vec<Owner>,
    root_vis: Vec<usize>,
    pkg_vis: Vec<Vec<usize>>,
    decls: HashMap<String, Vec<Decl>>,
    internal: HashMap<(Owner, String), String>,
    scopes: Vec<HashSet<String>>,
    diags: &'a mut Vec<Diag>,
}

const BUILTIN_TYPES: &[&str] = &["Int", "Float", "String", "Bool", "Color", "slot"];

/// Owner of every file id: 0 = std, `@name/...` = that package, anything else = the app.
pub(crate) fn owners(sources: &Sources, pkgs: &mut Vec<PkgLink>) -> Vec<Owner> {
    (0..sources.files.len())
        .map(|i| {
            if i == 0 {
                return Owner::Std;
            }
            match package_of(&sources.files[i].path) {
                Some(name) => {
                    let idx = pkgs.iter().position(|p| p.name == name).unwrap_or_else(|| {
                        pkgs.push(PkgLink { name: name.to_string(), ..Default::default() });
                        pkgs.len() - 1
                    });
                    Owner::Pkg(idx)
                }
                None => Owner::Root,
            }
        })
        .collect()
}

pub fn link(prog: &mut Program, sources: &Sources, info: &LinkInfo, diags: &mut Vec<Diag>) {
    let mut pkgs = info.packages.clone();
    let file_owner = owners(sources, &mut pkgs);
    let idx_of = |names: &[String], pkgs: &[PkgLink]| -> Vec<usize> { names.iter().filter_map(|n| pkgs.iter().position(|p| &p.name == n)).collect() };
    let root_vis = match &info.root_deps {
        Some(d) => idx_of(d, &pkgs),
        None => (0..pkgs.len()).collect(),
    };
    let pkg_vis = (0..pkgs.len())
        .map(|i| match &pkgs[i].deps {
            Some(d) => idx_of(d, &pkgs),
            None => (0..pkgs.len()).filter(|j| *j != i).collect(),
        })
        .collect();
    let mut l = Linker { pkgs, file_owner, root_vis, pkg_vis, decls: HashMap::new(), internal: HashMap::new(), scopes: Vec::new(), diags };
    l.collect(prog);
    l.rename_decls(prog);
    l.walk(prog);
}

impl Linker<'_> {
    fn owner(&self, sp: Span) -> Owner {
        self.file_owner.get(sp.file as usize).copied().unwrap_or(Owner::Root)
    }
    fn pkg_name(&self, o: Owner) -> String {
        match o {
            Owner::Std => "std".into(),
            Owner::Root => "the app".into(),
            Owner::Pkg(i) => format!("package `{}`", self.pkgs[i].name),
        }
    }
    fn vis(&self, o: Owner) -> &[usize] {
        match o {
            Owner::Root => &self.root_vis,
            Owner::Pkg(i) => &self.pkg_vis[i],
            Owner::Std => &[],
        }
    }

    // ---------------- declarations ----------------
    fn collect(&mut self, p: &Program) {
        let add = |name: &str, kind: DK, span: Span, s: &mut Self| {
            let owner = s.owner(span);
            s.decls.entry(name.to_string()).or_default().push(Decl { kind, owner, span });
        };
        for v in &p.views {
            add(&v.name, DK::View, v.span, self);
        }
        for d in &p.structs {
            add(&d.name, DK::Struct, d.span, self);
        }
        for d in &p.enums {
            add(&d.name, DK::Enum, d.span, self);
        }
        for d in &p.fns {
            add(&d.name, DK::Fn, d.span, self);
        }
        for d in &p.externs {
            add(&d.name, DK::Extern, d.span, self);
        }
        for d in &p.events {
            add(&d.name, DK::Event, d.span, self);
        }
        for d in &p.consts {
            add(&d.name, DK::Const, d.span, self);
        }
        // native names are global: report each later declaration from another owner
        let mut native_clash: Vec<(Span, String, String)> = Vec::new();
        for (name, ds) in &self.decls {
            for k in [DK::Extern, DK::Event] {
                let nat: Vec<&Decl> = ds.iter().filter(|d| d.kind == k).collect();
                if let Some(first) = nat.first() {
                    for d in nat.iter().skip(1).filter(|d| d.owner != first.owner) {
                        native_clash.push((d.span, name.clone(), format!("{} and {}", self.pkg_name(first.owner), self.pkg_name(d.owner))));
                    }
                }
            }
        }
        native_clash.sort_by_key(|c| (c.0.file, c.0.start));
        for (sp, name, who) in native_clash {
            self.diags.push(
                Diag::error("E0233", sp, format!("native name `{}` is declared by {}", name, who))
                    .help("the platform calls extern fn/event by name, so a native name exists once per app; remove the duplicate declaration or rename one side (both in Vard and in the native module)"),
            );
        }
        // internal names: package declarations that collide with any other owner get `pkg.Name`
        for (name, ds) in &self.decls {
            for d in ds {
                let mut iname = name.clone();
                if let Owner::Pkg(i) = d.owner {
                    let native = matches!(d.kind, DK::Extern | DK::Event);
                    if !native && ds.iter().any(|o| o.owner != d.owner) {
                        iname = format!("{}.{}", self.pkgs[i].name, name);
                    }
                }
                self.internal.insert((d.owner, name.clone()), iname);
            }
        }
    }

    fn iname(&self, owner: Owner, name: &str) -> String {
        self.internal.get(&(owner, name.to_string())).cloned().unwrap_or_else(|| name.to_string())
    }

    fn rename_decls(&mut self, p: &mut Program) {
        macro_rules! ren {
            ($list:expr) => {
                for d in $list.iter_mut() {
                    let o = self.owner(d.span);
                    if let Owner::Pkg(_) = o {
                        d.name = self.iname(o, &d.name);
                    }
                }
            };
        }
        ren!(p.views);
        ren!(p.structs);
        ren!(p.enums);
        ren!(p.fns);
        ren!(p.consts);
    }

    // ---------------- resolution ----------------
    /// `(internal name, owner of the target, declared name)` of an unqualified reference.
    fn resolve(&mut self, owner: Owner, name: &str, class: Class, span: Span) -> Option<(String, Owner, String)> {
        if owner == Owner::Std {
            return None;
        }
        let found = |s: &Self, o: Owner| s.decls.get(name).is_some_and(|ds| ds.iter().any(|d| d.owner == o && class.accepts(d.kind)));
        if found(self, owner) {
            return Some((self.iname(owner, name), owner, name.to_string()));
        }
        let vis = self.vis(owner).to_vec();
        let private = name.starts_with('_');
        let cands: Vec<usize> = vis.iter().copied().filter(|q| !private && found(self, Owner::Pkg(*q))).collect();
        if let Some(&q) = cands.first() {
            if cands.len() > 1 {
                let names: Vec<String> = cands.iter().map(|c| format!("`{}.{}`", self.pkgs[*c].name, name)).collect();
                let who: Vec<String> = cands.iter().map(|c| format!("`{}`", self.pkgs[*c].name)).collect();
                self.diags.push(
                    Diag::error("E0231", span, format!("`{}` is exported by {}", name, who.join(" and ")))
                        .help(format!("qualify it with the package: {}", names.join(" or "))),
                );
            }
            return Some((self.iname(Owner::Pkg(q), name), Owner::Pkg(q), name.to_string()));
        }
        if found(self, Owner::Std) {
            return None;
        }
        // declared, but not visible from here
        let hidden: Option<usize> = (0..self.pkgs.len()).find(|q| Owner::Pkg(*q) != owner && found(self, Owner::Pkg(*q)));
        if let Some(q) = hidden {
            let qn = self.pkgs[q].name.clone();
            if private && vis.contains(&q) {
                self.diags.push(
                    Diag::error("E0230", span, format!("`{}` is private to package `{}`", name, qn))
                        .help("names starting with `_` are package-internal; use the package's public API (`vard doc PACKAGE`)"),
                );
            } else {
                let help = match owner {
                    Owner::Root => format!("add the dependency: `vard add {}`", qn),
                    _ => format!("add `{}` to [dependencies] of {}", qn, self.pkg_name(owner)),
                };
                self.diags.push(Diag::error("E0232", span, format!("`{}` comes from package `{}`, which {} does not depend on", name, qn, self.pkg_name(owner))).help(help));
            }
            return Some((self.iname(Owner::Pkg(q), name), Owner::Pkg(q), name.to_string()));
        }
        // a name a dependency renamed in a newer version
        for q in vis {
            if let Some(m) = self.pkgs[q].migrations.iter().find(|m| m.from == name && !m.to.contains('.')).cloned() {
                self.diags.push(
                    Diag::error("E0234", span, format!("`{}` was renamed to `{}` in {} {}", name, m.to, self.pkgs[q].name, m.since)).fix(span, m.to.clone()),
                );
                if found(self, Owner::Pkg(q)) || self.decls.get(&m.to).is_some_and(|ds| ds.iter().any(|d| d.owner == Owner::Pkg(q))) {
                    return Some((self.iname(Owner::Pkg(q), &m.to), Owner::Pkg(q), m.to.clone()));
                }
            }
        }
        None
    }

    /// `pkg.Name`. Returns `None` when `pkg` is not a package (an ordinary member access).
    fn resolve_q(&mut self, owner: Owner, pkg: &str, name: &str, class: Class, span: Span) -> Option<Option<(String, Owner, String)>> {
        let q = if pkg == "std" { None } else { Some(self.pkgs.iter().position(|p| p.name == pkg)?) };
        let Some(q) = q else {
            // `std.Button`: always the std declaration
            return Some(None);
        };
        let target = Owner::Pkg(q);
        if target != owner && !self.vis(owner).contains(&q) {
            let help = match owner {
                Owner::Root => format!("add the dependency: `vard add {}`", pkg),
                _ => format!("add `{}` to [dependencies] of {}", pkg, self.pkg_name(owner)),
            };
            self.diags.push(Diag::error("E0232", span, format!("{} does not depend on package `{}`", self.pkg_name(owner), pkg)).help(help));
        } else if name.starts_with('_') && target != owner {
            self.diags.push(
                Diag::error("E0230", span, format!("`{}` is private to package `{}`", name, pkg))
                    .help("names starting with `_` are package-internal; use the package's public API (`vard doc PACKAGE`)"),
            );
        }
        let has = self.decls.get(name).is_some_and(|ds| ds.iter().any(|d| d.owner == target && class.accepts(d.kind)));
        if has {
            return Some(Some((self.iname(target, name), target, name.to_string())));
        }
        if let Some(m) = self.pkgs[q].migrations.iter().find(|m| m.from == name && !m.to.contains('.')).cloned() {
            let name_sp = Span::new(span.file, span.end - name.len() as u32, span.end);
            self.diags.push(Diag::error("E0234", name_sp, format!("`{}` was renamed to `{}` in {} {}", name, m.to, pkg, m.since)).fix(name_sp, m.to.clone()));
            return Some(Some((self.iname(target, &m.to), target, m.to.clone())));
        }
        let exports: Vec<String> = self
            .decls
            .iter()
            .filter(|(n, ds)| !n.starts_with('_') && ds.iter().any(|d| d.owner == target && class.accepts(d.kind)))
            .map(|(n, _)| n.clone())
            .collect();
        let d = Diag::error("E0235", span, format!("package `{}` has no {} `{}`", pkg, match class {
            Class::Type => "type",
            Class::View => "view",
            Class::Value => "function or value",
        }, name));
        match closest(name, exports.iter().map(|s| s.as_str())).map(|s| s.to_string()) {
            Some(s) => {
                // continue as if the fix were applied: no cascade of "unknown element" errors
                let name_sp = Span::new(span.file, span.end - name.len() as u32, span.end);
                self.diags.push(d.fix(name_sp, s.clone()));
                Some(Some((self.iname(target, &s), target, s)))
            }
            None => {
                self.diags.push(d.help(format!("its API: `vard doc {}`", pkg)));
                Some(None)
            }
        }
    }

    fn prop_migration(&self, q: usize, view: &str, arg: &str) -> Option<Migration> {
        let key = format!("{}.{}", view, arg);
        self.pkgs[q].migrations.iter().find(|m| m.from == key).cloned()
    }

    // ---------------- walking ----------------
    fn is_local(&self, n: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }
    fn push(&mut self, names: impl IntoIterator<Item = String>) {
        self.scopes.push(names.into_iter().collect());
    }

    fn walk(&mut self, p: &mut Program) {
        for i in 0..p.consts.len() {
            let (v, sp) = (p.consts[i].value, p.consts[i].span);
            let o = self.owner(sp);
            if let Some(mut t) = p.consts[i].ty.take() {
                self.ty(&mut t, o);
                p.consts[i].ty = Some(t);
            }
            self.expr(p, v, o);
        }
        let mut structs = std::mem::take(&mut p.structs);
        for s in &mut structs {
            let o = self.owner(s.span);
            self.params(p, &mut s.fields, o);
        }
        p.structs = structs;
        let mut fns = std::mem::take(&mut p.fns);
        for f in &mut fns {
            let o = self.owner(f.span);
            self.fn_decl(p, f, o);
        }
        p.fns = fns;
        let mut externs = std::mem::take(&mut p.externs);
        for f in &mut externs {
            let o = self.owner(f.span);
            self.params(p, &mut f.params, o);
            if let Some(t) = &mut f.ret {
                self.ty(t, o);
            }
        }
        p.externs = externs;
        let mut events = std::mem::take(&mut p.events);
        for ev in &mut events {
            let o = self.owner(ev.span);
            self.params(p, &mut ev.params, o);
        }
        p.events = events;
        let mut views = std::mem::take(&mut p.views);
        for v in &mut views {
            let o = self.owner(v.span);
            self.view(p, v, o);
        }
        p.views = views;
    }

    fn params(&mut self, p: &mut Program, ps: &mut [Param], o: Owner) {
        for pa in ps.iter_mut() {
            self.ty(&mut pa.ty, o);
            if let Some(d) = pa.default {
                self.expr(p, d, o);
            }
        }
    }

    fn fn_decl(&mut self, p: &mut Program, f: &mut FnDecl, o: Owner) {
        self.params(p, &mut f.params, o);
        if let Some(t) = &mut f.ret {
            self.ty(t, o);
        }
        self.push(f.params.iter().map(|x| x.name.clone()));
        self.stmts(p, &mut f.body, o);
        self.scopes.pop();
    }

    fn view(&mut self, p: &mut Program, v: &mut ViewDecl, o: Owner) {
        self.params(p, &mut v.params, o);
        let mut names: Vec<String> = v.params.iter().map(|x| x.name.clone()).collect();
        for m in &v.members {
            match m {
                ViewMember::State { name, .. } | ViewMember::Pref { name, .. } | ViewMember::Let { name, .. } => names.push(name.clone()),
                ViewMember::Fn(f) => names.push(f.name.clone()),
                _ => {}
            }
        }
        self.push(names);
        for m in &mut v.members {
            match m {
                ViewMember::State { ty, init, .. } | ViewMember::Pref { ty, init, .. } => {
                    if let Some(t) = ty {
                        self.ty(t, o);
                    }
                    self.expr(p, *init, o);
                }
                ViewMember::Let { ty, value, .. } => {
                    if let Some(t) = ty {
                        self.ty(t, o);
                    }
                    self.expr(p, *value, o);
                }
                ViewMember::Fn(f) => self.fn_decl(p, f, o),
                ViewMember::On { params, cond, body, .. } => {
                    self.push(params.iter().cloned());
                    if let Some(c) = cond {
                        self.expr(p, *c, o);
                    }
                    self.stmts(p, body, o);
                    self.scopes.pop();
                }
                ViewMember::Node(id) => self.ui(p, *id, o),
            }
        }
        self.scopes.pop();
    }

    fn ui(&mut self, p: &mut Program, id: UiId, o: Owner) {
        match p.nodes[id as usize].clone() {
            UiNode::Element { name, name_span, mut args, children, has_block, span } => {
                let mut new_name = name.clone();
                let mut target: Option<(Owner, String)> = None;
                if registry::element(&name).is_none() {
                    let r = match name.split_once('.') {
                        Some(("std", v)) => {
                            new_name = v.to_string();
                            None
                        }
                        Some((q, v)) => match self.resolve_q(o, q, v, Class::View, name_span) {
                            Some(r) => r,
                            None => {
                                self.diags.push(Diag::error("E0235", name_span, format!("unknown package `{}`", q)).help(format!(
                                    "`pkg.View` names a view of a dependency; this project's packages: {}",
                                    if self.pkgs.is_empty() { "none (`vard add NAME`)".to_string() } else { self.pkgs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(" ") }
                                )));
                                None
                            }
                        },
                        None => self.resolve(o, &name, Class::View, name_span),
                    };
                    if let Some((iname, towner, decl)) = r {
                        new_name = iname;
                        target = Some((towner, decl));
                    }
                }
                if let Some((Owner::Pkg(q), vname)) = &target {
                    if Owner::Pkg(*q) != o {
                        for a in args.iter_mut() {
                            let Some(an) = a.name.clone() else { continue };
                            if let Some(m) = self.prop_migration(*q, vname, &an) {
                                let to = m.to.rsplit('.').next().unwrap_or(&m.to).to_string();
                                self.diags.push(
                                    Diag::error("E0234", a.name_span, format!("`{}` of `{}` was renamed to `{}` in {} {}", an, vname, to, self.pkgs[*q].name, m.since))
                                        .fix(a.name_span, to.clone()),
                                );
                                a.name = Some(to);
                            }
                        }
                    }
                }
                for a in &args {
                    self.expr(p, a.value, o);
                }
                for c in &children {
                    self.ui(p, *c, o);
                }
                p.nodes[id as usize] = UiNode::Element { name: new_name, name_span, args, children, has_block, span };
            }
            UiNode::If { cond, then, els, .. } => {
                self.expr(p, cond, o);
                for c in then.iter().chain(&els) {
                    self.ui(p, *c, o);
                }
            }
            UiNode::For { var, index_var, iter, key, body, .. } => {
                self.expr(p, iter, o);
                self.push(std::iter::once(var).chain(index_var));
                if let Some(k) = key {
                    self.expr(p, k, o);
                }
                for c in &body {
                    self.ui(p, *c, o);
                }
                self.scopes.pop();
            }
            UiNode::Slot { .. } => {}
        }
    }

    fn stmts(&mut self, p: &mut Program, ss: &mut [Stmt], o: Owner) {
        self.scopes.push(HashSet::new());
        for s in ss.iter_mut() {
            match s {
                Stmt::Let { name, ty, value, .. } => {
                    self.expr(p, *value, o);
                    if let Some(t) = ty {
                        self.ty(t, o);
                    }
                    self.scopes.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Assign { target, value, .. } => {
                    self.expr(p, *target, o);
                    self.expr(p, *value, o);
                }
                Stmt::Expr(x) => self.expr(p, *x, o),
                Stmt::If { cond, then, els } => {
                    self.expr(p, *cond, o);
                    self.stmts(p, then, o);
                    self.stmts(p, els, o);
                }
                Stmt::For { var, iter, body, .. } => {
                    self.expr(p, *iter, o);
                    self.push([var.clone()]);
                    self.stmts(p, body, o);
                    self.scopes.pop();
                }
                Stmt::Return(Some(x), _) => self.expr(p, *x, o),
                Stmt::Return(None, _) => {}
            }
        }
        self.scopes.pop();
    }

    fn ty(&mut self, t: &mut TypeRef, o: Owner) {
        match t {
            TypeRef::Named(n, sp) => {
                if BUILTIN_TYPES.contains(&n.as_str()) {
                    return;
                }
                let r = match n.split_once('.') {
                    Some(("std", name)) => Some((name.to_string(), Owner::Std, name.to_string())),
                    Some((q, name)) => match self.resolve_q(o, q, name, Class::Type, *sp) {
                        Some(r) => r,
                        None => {
                            self.diags.push(Diag::error("E0235", *sp, format!("unknown package `{}`", q)));
                            None
                        }
                    },
                    None => self.resolve(o, n, Class::Type, *sp),
                };
                if let Some((iname, _, _)) = r {
                    *n = iname;
                }
            }
            TypeRef::List(t) | TypeRef::Map(t) | TypeRef::Opt(t) => self.ty(t, o),
            TypeRef::Fn(ps, r) => {
                for x in ps.iter_mut() {
                    self.ty(x, o);
                }
                if let Some(r) = r {
                    self.ty(r, o);
                }
            }
        }
    }

    fn expr(&mut self, p: &mut Program, e: ExprId, o: Owner) {
        match p.exprs[e as usize].clone() {
            Expr::Ident(n) => {
                if self.is_local(&n) || registry::global(&n).is_some() {
                    return;
                }
                let sp = p.span(e);
                if let Some((iname, _, _)) = self.resolve(o, &n, Class::Value, sp) {
                    if iname != n {
                        p.exprs[e as usize] = Expr::Ident(iname);
                    }
                }
            }
            Expr::Member(b, f, _) => {
                if let Expr::Ident(q) = p.expr(b).clone() {
                    if !self.is_local(&q) && registry::global(&q).is_none() && (q == "std" || self.pkgs.iter().any(|x| x.name == q)) {
                        let sp = p.span(e);
                        match self.resolve_q(o, &q, &f, Class::Value, sp) {
                            Some(Some((iname, _, _))) => p.exprs[e as usize] = Expr::Ident(iname),
                            Some(None) if q == "std" => p.exprs[e as usize] = Expr::Ident(f.clone()),
                            _ => {}
                        }
                        return;
                    }
                }
                self.expr(p, b, o);
            }
            Expr::Str(parts) => {
                for part in parts {
                    if let StrPart::Expr(x) = part {
                        self.expr(p, x, o);
                    }
                }
            }
            Expr::List(xs) => {
                for x in xs {
                    self.expr(p, x, o);
                }
            }
            Expr::Map(kv) => {
                for (k, v) in kv {
                    self.expr(p, k, o);
                    self.expr(p, v, o);
                }
            }
            Expr::Unary(_, a) => self.expr(p, a, o),
            Expr::Binary(_, a, b) | Expr::Index(a, b) => {
                self.expr(p, a, o);
                self.expr(p, b, o);
            }
            Expr::Ternary(a, b, c) => {
                self.expr(p, a, o);
                self.expr(p, b, o);
                self.expr(p, c, o);
            }
            Expr::Call { callee, args } => {
                self.expr(p, callee, o);
                for a in args {
                    self.expr(p, a.value, o);
                }
            }
            Expr::Lambda { params, body } => {
                self.push(params.iter().cloned());
                match body {
                    LambdaBody::Expr(x) => self.expr(p, x, o),
                    LambdaBody::Block(mut ss) => {
                        self.stmts(p, &mut ss, o);
                        p.exprs[e as usize] = Expr::Lambda { params, body: LambdaBody::Block(ss) };
                    }
                }
                self.scopes.pop();
            }
            _ => {}
        }
    }
}
