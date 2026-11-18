//! Type checker. Bidirectional: the expected type flows down so `.center` and `.primary`
//! resolve without qualification, and lambda parameters need no annotations.
//!
//! Every error tries to carry a machine-applicable fix or a concrete `help`.

use crate::ast::*;
use crate::diag::{closest, Diag, Sources, Span};
use crate::registry::{self, PTy};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Str,
    Bool,
    Color,
    Unit,
    Nil,
    List(Box<Ty>),
    /// `[String: T]`
    Map(Box<Ty>),
    Opt(Box<Ty>),
    Struct(String),
    Enum(String),
    Fn(Vec<Ty>, Box<Ty>),
    Slot,
    /// Poison type: produced after an error, compatible with everything (no cascades).
    Unknown,
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Int => write!(f, "Int"),
            Ty::Float => write!(f, "Float"),
            Ty::Str => write!(f, "String"),
            Ty::Bool => write!(f, "Bool"),
            Ty::Color => write!(f, "Color"),
            Ty::Unit => write!(f, "()"),
            Ty::Nil => write!(f, "nil"),
            Ty::List(t) => write!(f, "[{}]", t),
            Ty::Map(t) => write!(f, "[String: {}]", t),
            Ty::Opt(t) => write!(f, "{}?", t),
            Ty::Struct(n) | Ty::Enum(n) => write!(f, "{}", n),
            Ty::Fn(ps, r) => {
                write!(f, "fn(")?;
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ")")?;
                if **r != Ty::Unit {
                    write!(f, " -> {}", r)?;
                }
                Ok(())
            }
            Ty::Slot => write!(f, "slot"),
            Ty::Unknown => write!(f, "_"),
        }
    }
}

fn assignable(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        (a, b) if a == b => true,
        (Ty::Int, Ty::Float) => true,
        (Ty::Nil, Ty::Opt(_)) => true,
        (a, Ty::Opt(b)) => assignable(a, b),
        (Ty::List(a), Ty::List(b)) => assignable(a, b),
        (Ty::Map(a), Ty::Map(b)) => assignable(a, b),
        (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => {
            pa.len() == pb.len() && pa.iter().zip(pb).all(|(a, b)| assignable(b, a)) && (**rb == Ty::Unit || assignable(ra, rb))
        }
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum VarKind {
    State,
    Derived,
    Param,
    BindParam,
    Local { mutable: bool },
    Item { writable: bool },
    Fn,
    Slot,
}

#[derive(Clone, Debug)]
struct Var {
    ty: Ty,
    kind: VarKind,
}

#[derive(Clone)]
struct ParamSig {
    name: String,
    ty: Ty,
    has_default: bool,
    bind: bool,
}

#[derive(Clone)]
struct FnSig {
    params: Vec<ParamSig>,
    ret: Ty,
}

pub struct Checker<'a> {
    p: &'a Program,
    src: &'a Sources,
    pub diags: Vec<Diag>,
    pub dot_kinds: HashMap<ExprId, DotKind>,
    structs: HashMap<String, Vec<ParamSig>>,
    enums: HashMap<String, Vec<String>>,
    fns: HashMap<String, FnSig>,
    consts: HashMap<String, Ty>,
    views: HashMap<String, Vec<ParamSig>>,
    events: HashMap<String, Vec<ParamSig>>,
    /// `pref` names are app-wide: every declaration of one name must agree on the type.
    prefs: HashMap<String, Ty>,
    tokens: HashSet<String>,
    scopes: Vec<HashMap<String, Var>>,
    /// User code mentions `safeArea` (see `Program::uses_safe_area`).
    pub uses_safe_area: bool,
    /// Return type of the enclosing fn/lambda: `Some(Some(t))` fixed, `Some(None)` infer.
    ret: Option<Option<Ty>>,
    inferred_ret: Option<Ty>,
}

/// Handle to the diagnostic just pushed, for attaching help or a fix.
struct D<'d>(&'d mut Diag);
impl D<'_> {
    fn help(self, h: impl Into<String>) {
        self.0.help = Some(h.into());
    }
}
impl std::ops::Deref for D<'_> {
    type Target = Diag;
    fn deref(&self) -> &Diag {
        self.0
    }
}
impl std::ops::DerefMut for D<'_> {
    fn deref_mut(&mut self) -> &mut Diag {
        self.0
    }
}

/// Layout props that may be passed to any view use; they apply to the view's root element.
pub const VIEW_OUTER_PROPS: &[&str] =
    &["width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight", "aspectRatio", "grow", "alignSelf", "id", "opacity", "offsetX", "offsetY", "scale"];

const BUILTIN_FNS: &[&str] = &["print", "range", "min", "max"];

impl<'a> Checker<'a> {
    pub fn new(p: &'a Program, src: &'a Sources) -> Checker<'a> {
        Checker {
            p,
            src,
            diags: Vec::new(),
            dot_kinds: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            fns: HashMap::new(),
            consts: HashMap::new(),
            views: HashMap::new(),
            events: HashMap::new(),
            prefs: HashMap::new(),
            tokens: HashSet::new(),
            scopes: Vec::new(),
            uses_safe_area: false,
            ret: None,
            inferred_ret: None,
        }
    }

    fn err(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> D<'_> {
        self.diags.push(Diag::error(code, span, msg));
        D(self.diags.last_mut().unwrap())
    }
    fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> D<'_> {
        self.diags.push(Diag::warning(code, span, msg));
        D(self.diags.last_mut().unwrap())
    }

    // ---------------- declarations ----------------
    pub fn run(&mut self) {
        let p = self.p;
        for (n, vs) in registry::ENUMS {
            self.enums.insert(n.to_string(), vs.iter().map(|v| v.to_string()).collect());
        }
        for (n, _, _) in registry::COLOR_TOKENS {
            self.tokens.insert(n.to_string());
        }
        // 1. names of types (so they can reference each other in any order)
        for e in &p.enums {
            if self.enums.insert(e.name.clone(), e.variants.clone()).is_some() {
                self.err("E0220", e.span, format!("enum `{}` is already defined", e.name));
            }
        }
        for s in &p.structs {
            if self.structs.insert(s.name.clone(), Vec::new()).is_some() || self.enums.contains_key(&s.name) {
                self.err("E0220", s.span, format!("type `{}` is already defined", s.name));
            }
        }
        for t in &p.themes {
            for (name, e, sp) in &t.entries {
                if name == "font" {
                    if !matches!(p.expr(*e), Expr::Str(parts) if parts.iter().all(|s| matches!(s, StrPart::Lit(_)))) {
                        self.err("E0221", *sp, "`font` in a theme is a family name string, e.g. `font: \"Inter\"`");
                    }
                    continue;
                }
                if !matches!(p.expr(*e), Expr::Color(_)) {
                    self.err("E0221", *sp, "theme entries must be color literals, e.g. `primary: #2563EB` (or `font: \"Inter\"`)");
                }
                self.tokens.insert(name.clone());
            }
        }
        // 2. signatures
        for s in &p.structs {
            let fields = self.param_sigs(&s.fields);
            self.structs.insert(s.name.clone(), fields);
        }
        for f in &p.fns {
            let sig = FnSig { params: self.param_sigs(&f.params), ret: f.ret.as_ref().map(|t| self.resolve(t)).unwrap_or(Ty::Unit) };
            if self.fns.insert(f.name.clone(), sig).is_some() || BUILTIN_FNS.contains(&f.name.as_str()) {
                self.err("E0220", f.span, format!("function `{}` is already defined", f.name));
            }
        }
        for f in &p.externs {
            let sig = FnSig { params: self.param_sigs(&f.params), ret: f.ret.as_ref().map(|t| self.resolve(t)).unwrap_or(Ty::Unit) };
            for (pa, ps) in f.params.iter().zip(&sig.params) {
                let ok = match &ps.ty {
                    Ty::Fn(args, r) => args.iter().all(is_data) && **r == Ty::Unit,
                    t => is_data(t),
                };
                if !ok {
                    self.err("E0317", pa.span, format!("`{}: {}` cannot cross the native boundary", pa.name, ps.ty))
                        .help("extern parameters are data (Int Float String Bool Color, structs, enums, lists, maps, optionals) or callbacks `fn(Data)` that return nothing");
                }
                if pa.bind {
                    self.err("E0317", pa.span, "extern fn parameters cannot be `bind`; return the new value or pass a callback");
                }
            }
            if !is_data(&sig.ret) && sig.ret != Ty::Unit {
                self.err("E0317", f.span, format!("extern fn `{}` cannot return {}", f.name, sig.ret))
                    .help("return data; for results that arrive later take a callback parameter: `done: fn(Result)`");
            }
            if self.fns.insert(f.name.clone(), sig).is_some() {
                self.err("E0220", f.span, format!("function `{}` is already defined", f.name));
            }
        }
        for ev in &p.events {
            let sig = self.param_sigs(&ev.params);
            for (pa, ps) in ev.params.iter().zip(&sig) {
                if !is_data(&ps.ty) {
                    self.err("E0317", pa.span, format!("event parameter `{}: {}` must be data", pa.name, ps.ty));
                }
            }
            if self.events.insert(ev.name.clone(), sig).is_some() {
                self.err("E0220", ev.span, format!("event `{}` is already defined", ev.name));
            }
        }
        for v in &p.views {
            if registry::element(&v.name).is_some() {
                self.err("E0216", v.span, format!("`{}` is a built-in element; choose another view name", v.name));
            }
            // (`pkg.Name` = a package view renamed by the linker; `_Name` = package-private)
            let plain = v.name.rsplit('.').next().unwrap_or(&v.name).trim_start_matches('_');
            if !plain.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                self.err("E0211", v.span, format!("view names start with an uppercase letter: `{}`", v.name));
            }
            let sig = self.param_sigs(&v.params);
            if v.params.iter().filter(|p| is_slot(&p.ty)).count() > 1 {
                self.err("E0210", v.span, "a view can declare at most one `slot` parameter");
            }
            if self.views.insert(v.name.clone(), sig).is_some() {
                self.err("E0220", v.span, format!("view `{}` is already defined", v.name));
            }
        }
        // 3. bodies
        for s in &p.structs {
            self.scopes.push(HashMap::new());
            for f in &s.fields {
                if let Some(d) = f.default {
                    let ty = self.resolve(&f.ty);
                    self.expect(d, &ty);
                }
            }
            self.scopes.pop();
        }
        for c in &p.consts {
            self.scopes.push(HashMap::new());
            let ty = self.declared_or_inferred(c.ty.as_ref(), c.value, &c.name, c.span, "const");
            self.scopes.pop();
            self.consts.insert(c.name.clone(), ty);
        }
        for f in &p.fns {
            self.check_fn(f);
        }
        for v in &p.views {
            self.check_view(v);
        }
    }

    fn param_sigs(&mut self, params: &[Param]) -> Vec<ParamSig> {
        params
            .iter()
            .map(|p| ParamSig { name: p.name.clone(), ty: self.resolve(&p.ty), has_default: p.default.is_some(), bind: p.bind })
            .collect()
    }

    fn resolve(&mut self, t: &TypeRef) -> Ty {
        match t {
            TypeRef::Named(n, sp) => match n.as_str() {
                "Int" => Ty::Int,
                "Float" => Ty::Float,
                "String" => Ty::Str,
                "Bool" => Ty::Bool,
                "Color" => Ty::Color,
                "slot" => Ty::Slot,
                _ if self.structs.contains_key(n) => Ty::Struct(n.clone()),
                _ if self.enums.contains_key(n) => Ty::Enum(n.clone()),
                _ => {
                    let alias = registry::alias(registry::TYPE_ALIASES, n);
                    let names: Vec<&str> = ["Int", "Float", "String", "Bool", "Color"]
                        .into_iter()
                        .chain(self.structs.keys().map(|s| s.as_str()))
                        .chain(self.enums.keys().map(|s| s.as_str()))
                        .collect();
                    let sugg = alias.map(|s| s.to_string()).or_else(|| closest(n, names).map(|s| s.to_string()));
                    let mut d = self.err("E0204", *sp, format!("unknown type `{}`", n));
                    match sugg {
                        Some(s) => {
                            d.fix = Some(crate::diag::Fix { span: *sp, replacement: s });
                        }
                        None => {
                            d.help = Some("built-in types: Int Float String Bool Color, [T], T?, fn(T) -> R; declare your own with `struct` / `enum`".into());
                        }
                    }
                    Ty::Unknown
                }
            },
            TypeRef::List(t) => Ty::List(Box::new(self.resolve(t))),
            TypeRef::Map(t) => Ty::Map(Box::new(self.resolve(t))),
            TypeRef::Opt(t) => Ty::Opt(Box::new(self.resolve(t))),
            TypeRef::Fn(ps, r) => {
                let ps = ps.iter().map(|t| self.resolve(t)).collect();
                let r = r.as_ref().map(|t| self.resolve(t)).unwrap_or(Ty::Unit);
                Ty::Fn(ps, Box::new(r))
            }
        }
    }

    /// Type of a `state`/`let`/`const`/local: the annotation if present, else inferred.
    fn declared_or_inferred(&mut self, ann: Option<&TypeRef>, value: ExprId, name: &str, span: Span, kw: &str) -> Ty {
        if let Some(t) = ann {
            let ty = self.resolve(t);
            self.expect(value, &ty);
            return ty;
        }
        let ty = self.check(value, None);
        let vague = match &ty {
            Ty::Nil => Some("T?"),
            Ty::List(t) if **t == Ty::Unknown && matches!(self.p.expr(value), Expr::List(v) if v.is_empty()) => Some("[T]"),
            Ty::Map(t) if **t == Ty::Unknown && matches!(self.p.expr(value), Expr::Map(v) if v.is_empty()) => Some("[String: T]"),
            _ => None,
        };
        if let Some(shape) = vague {
            self.err("E0308", span, format!("cannot infer the type of `{}`", name))
                .help(format!("add an annotation: `{} {}: {} = ...`", kw, name, shape));
            return Ty::Unknown;
        }
        if ty == Ty::Unit {
            self.err("E0308", span, format!("`{}` has no value (the expression returns nothing)", name));
            return Ty::Unknown;
        }
        ty
    }

    fn check_fn(&mut self, f: &FnDecl) {
        self.scopes.push(HashMap::new());
        for p in &f.params {
            let ty = self.resolve_quiet(&p.ty);
            if let Some(d) = p.default {
                self.expect(d, &ty);
            }
            self.declare(&p.name, ty, VarKind::Param);
        }
        let ret = f.ret.as_ref().map(|t| self.resolve_quiet(t)).unwrap_or(Ty::Unit);
        let saved = self.ret.replace(Some(ret));
        self.block(&f.body);
        self.ret = saved;
        self.scopes.pop();
    }

    /// Resolve without re-reporting (signatures were already resolved once in `run`).
    fn resolve_quiet(&mut self, t: &TypeRef) -> Ty {
        let n = self.diags.len();
        let ty = self.resolve(t);
        self.diags.truncate(n);
        ty
    }

    fn declare(&mut self, name: &str, ty: Ty, kind: VarKind) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), Var { ty, kind });
    }

    fn lookup(&self, name: &str) -> Option<&Var> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn check_view(&mut self, v: &ViewDecl) {
        self.scopes.push(HashMap::new());
        for p in &v.params {
            let ty = self.resolve_quiet(&p.ty);
            if let Some(d) = p.default {
                self.expect(d, &ty);
            }
            let kind = if ty == Ty::Slot {
                VarKind::Slot
            } else if p.bind {
                VarKind::BindParam
            } else {
                VarKind::Param
            };
            self.declare(&p.name, ty, kind);
        }
        for m in &v.members {
            if let ViewMember::Fn(f) = m {
                let params: Vec<Ty> = f.params.iter().map(|p| self.resolve(&p.ty)).collect();
                let ret = f.ret.as_ref().map(|t| self.resolve(t)).unwrap_or(Ty::Unit);
                self.declare(&f.name, Ty::Fn(params, Box::new(ret)), VarKind::Fn);
            }
        }
        for m in &v.members {
            match m {
                ViewMember::State { name, ty, init, span } => {
                    let t = self.declared_or_inferred(ty.as_ref(), *init, name, *span, "state");
                    self.declare(name, t, VarKind::State);
                }
                ViewMember::Pref { name, ty, init, span } => {
                    let t = self.declared_or_inferred(ty.as_ref(), *init, name, *span, "pref");
                    if !is_data(&t) {
                        self.err("E0317", *span, format!("`pref {}` must hold data that can be stored, found {}", name, t));
                    }
                    match self.prefs.get(name) {
                        Some(prev) if *prev != t && t != Ty::Unknown && *prev != Ty::Unknown => {
                            let prev = prev.clone();
                            self.err("E0223", *span, format!("`pref {}` is declared elsewhere as {}; one name = one app-wide value", name, prev))
                                .help(format!("use the same type ({}) or another name", prev));
                        }
                        _ => {
                            self.prefs.insert(name.clone(), t.clone());
                        }
                    }
                    self.declare(name, t, VarKind::State);
                }
                ViewMember::Let { name, ty, value, span } => {
                    let t = self.declared_or_inferred(ty.as_ref(), *value, name, *span, "let");
                    self.declare(name, t, VarKind::Derived);
                }
                _ => {}
            }
        }
        for m in &v.members {
            match m {
                ViewMember::Fn(f) => self.check_fn(f),
                ViewMember::Node(n) => self.ui(*n),
                ViewMember::On { event, event_span, params, cond, body, span } => self.check_on(event, *event_span, params, *cond, body, *span),
                _ => {}
            }
        }
        if !v.members.iter().any(|m| matches!(m, ViewMember::Node(_))) {
            self.warn("W0402", v.span, format!("view `{}` renders nothing (no elements in its body)", v.name));
        }
        self.scopes.pop();
    }

    fn check_on(&mut self, event: &str, event_span: Span, params: &[String], cond: Option<ExprId>, body: &[Stmt], span: Span) {
        let sig = match self.events.get(event).cloned() {
            Some(s) => Some(s),
            None => {
                let alias = registry::alias(registry::EVENT_ALIASES, event).filter(|a| self.events.contains_key(*a));
                let sugg = alias.map(|s| s.to_string()).or_else(|| closest(event, self.events.keys().map(|s| s.as_str())).map(|s| s.to_string()));
                let mut names: Vec<&str> = self.events.keys().map(|s| s.as_str()).collect();
                names.sort();
                let list = names.join(" ");
                let mut d = self.err("E0222", event_span, format!("unknown event `{}`", event));
                match &sugg {
                    Some(s) => d.fix = Some(crate::diag::Fix { span: event_span, replacement: s.clone() }),
                    None => d.help = Some(format!("events: {}; declare your own with `extern event name(params)`", list)),
                }
                sugg.and_then(|s| self.events.get(&s).cloned())
            }
        };
        self.scopes.push(HashMap::new());
        if let Some(sig) = &sig {
            if params.len() > sig.len() {
                let shape = sig.iter().map(|p| format!("{}: {}", p.name, p.ty)).collect::<Vec<_>>().join(", ");
                self.err("E0310", span, format!("`{}` passes {} value(s) ({}), but the handler takes {}", event, sig.len(), shape, params.len()));
            }
        }
        for (i, name) in params.iter().enumerate() {
            let ty = sig.as_ref().and_then(|s| s.get(i)).map(|p| p.ty.clone()).unwrap_or(Ty::Unknown);
            self.declare(name, ty, VarKind::Param);
        }
        if let Some(c) = cond {
            self.expect(c, &Ty::Bool);
        }
        let saved = self.ret.replace(Some(Ty::Unit));
        self.block(body);
        self.ret = saved;
        self.scopes.pop();
    }

    // ---------------- UI nodes ----------------
    fn ui_list(&mut self, ids: &[UiId]) {
        for id in ids {
            self.ui(*id);
        }
    }

    fn ui(&mut self, id: UiId) {
        let p = self.p;
        match p.node(id) {
            UiNode::If { cond, then, els, .. } => {
                self.expect(*cond, &Ty::Bool);
                self.scopes.push(HashMap::new());
                self.ui_list(then);
                self.scopes.pop();
                self.scopes.push(HashMap::new());
                self.ui_list(els);
                self.scopes.pop();
            }
            UiNode::For { var, index_var, iter, key, body, .. } => {
                let it = self.check(*iter, None);
                let item = match it {
                    Ty::List(t) => *t,
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.err("E0302", p.span(*iter), format!("`for` iterates over a list, found {}", other))
                            .help("for a counted loop use `for i in range(0, n)`");
                        Ty::Unknown
                    }
                };
                let writable = self.place_error(*iter).is_none();
                self.scopes.push(HashMap::new());
                self.declare(var, item.clone(), VarKind::Item { writable });
                if let Some(iv) = index_var {
                    self.declare(iv, Ty::Int, VarKind::Local { mutable: false });
                }
                match key {
                    Some(k) => {
                        let kt = self.check(*k, None);
                        if !matches!(kt, Ty::Int | Ty::Str | Ty::Enum(_) | Ty::Unknown) {
                            self.err("E0311", p.span(*k), format!("`by` key must be Int, String or an enum, found {}", kt));
                        }
                    }
                    None => {
                        if let Ty::Struct(s) = &item {
                            if self.structs.get(s).is_some_and(|f| f.iter().any(|f| f.name == "id")) {
                                let end = p.span(*iter).end;
                                let at = Span::new(p.span(*iter).file, end, end);
                                self.warn(
                                    "W0401",
                                    p.span(*iter),
                                    "list items have an `id` but the loop has no key; rows will be matched by position (state and animations can jump)",
                                )
                                .fix = Some(crate::diag::Fix { span: at, replacement: format!(" by {}.id", var) });
                            }
                        }
                    }
                }
                self.ui_list(body);
                self.scopes.pop();
            }
            UiNode::Slot { name, span } => match self.lookup(name).map(|v| v.kind.clone()) {
                Some(VarKind::Slot) => {}
                Some(_) => {
                    self.err("E0209", *span, format!("`{}` is not a slot; to show a value use `Text(\"{{{}}}\")`", name, name))
                        .fix = Some(crate::diag::Fix { span: *span, replacement: format!("Text(\"{{{}}}\")", name) });
                }
                None => {
                    self.err("E0209", *span, format!("unknown slot `{}`; element names start with an uppercase letter", name));
                }
            },
            UiNode::Element { name, name_span, args, children, has_block, span } => {
                if let Some(def) = registry::element(name) {
                    self.primitive(def, args, *span);
                    if *has_block && !def.children {
                        self.err("E0205", *name_span, format!("`{}` cannot have children", name))
                            .help("wrap them in a Column/Row/Stack next to it");
                    }
                } else if let Some(sig) = self.views.get(name).cloned() {
                    self.view_use(name, &sig, args, *has_block, *name_span);
                } else {
                    let alias = registry::alias(registry::ELEM_ALIASES, name);
                    let names: Vec<&str> =
                        registry::ELEMENTS.iter().map(|e| e.name).chain(self.views.keys().map(|s| s.as_str())).collect();
                    let sugg = alias.map(|s| s.to_string()).or_else(|| closest(name, names).map(|s| s.to_string()));
                    let mut d = self.err("E0201", *name_span, format!("unknown element `{}`", name));
                    match sugg {
                        Some(s) => {
                            d.fix = Some(crate::diag::Fix { span: *name_span, replacement: s.clone() });
                            // Check the arguments as if the fix were applied, so one pass reports everything.
                            if let Some(def) = registry::element(&s) {
                                self.primitive(def, args, *span);
                            } else if let Some(sig) = self.views.get(&s).cloned() {
                                self.view_use(&s, &sig, args, *has_block, *name_span);
                            }
                        }
                        None => {
                            d.help = Some(
                                "built-ins: Column Row Grid Stack Scroll Text Input Image Spacer; std views: Button Checkbox Switch Card Divider; or declare `view Name { ... }`"
                                    .into(),
                            )
                        }
                    }
                }
                self.scopes.push(HashMap::new());
                self.ui_list(children);
                self.scopes.pop();
            }
        }
    }

    fn primitive(&mut self, def: &'static registry::ElemDef, args: &[Arg], span: Span) {
        let p = self.p;
        let mut seen: Vec<&str> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let prop = match &arg.name {
                None => match (i, def.positional) {
                    (0, Some(pos)) => registry::prop(def, pos).unwrap(),
                    _ => {
                        let what = match def.positional {
                            Some(pos) => format!("only its first argument (`{}`) may be positional", pos),
                            None => "has no positional arguments".to_string(),
                        };
                        self.err("E0207", p.span(arg.value), format!("`{}` {}; write `name: value`", def.name, what))
                            .help(format!("see `vard doc {}`", def.name));
                        continue;
                    }
                },
                Some(n) => match registry::prop(def, n) {
                    Some(pr) => pr,
                    None => {
                        let alias = registry::alias(registry::PROP_ALIASES, n).filter(|a| registry::prop(def, a).is_some());
                        let all = def.props.iter().chain(registry::COMMON.iter()).map(|p| p.name);
                        let sugg = alias.or_else(|| closest(n, all));
                        let own: Vec<&str> = def.props.iter().map(|p| p.name).collect();
                        let mut d = self.err("E0202", arg.name_span, format!("unknown prop `{}` on `{}`", n, def.name));
                        match sugg {
                            Some(s) => d.fix = Some(crate::diag::Fix { span: arg.name_span, replacement: s.to_string() }),
                            None => {
                                d.help = Some(format!(
                                    "`{}` props: {}; plus common props (`vard doc common`)",
                                    def.name,
                                    if own.is_empty() { "none".to_string() } else { own.join(" ") }
                                ))
                            }
                        }
                        if n == "disabled" {
                            self.diags.last_mut().unwrap().help = Some("use `enabled: !condition`".into());
                        }
                        // Keep checking the value as if the fix were applied: one pass reports everything.
                        match sugg.and_then(|s| registry::prop(def, s)) {
                            Some(pr) => pr,
                            None => continue,
                        }
                    }
                },
            };
            if seen.contains(&prop.name) {
                self.err("E0208", arg.name_span, format!("prop `{}` is given twice", prop.name));
            }
            seen.push(prop.name);
            if prop.name == "safeArea" && span.file != 0 {
                self.uses_safe_area = true;
            }
            match prop.ty {
                PTy::Colors => self.expect(arg.value, &Ty::List(Box::new(Ty::Color))),
                PTy::Float => self.expect(arg.value, &Ty::Float),
                PTy::Int => self.expect(arg.value, &Ty::Int),
                PTy::Bool => self.expect(arg.value, &Ty::Bool),
                PTy::Str => self.expect(arg.value, &Ty::Str),
                PTy::Color => self.expect(arg.value, &Ty::Color),
                PTy::Enum(e) => self.expect(arg.value, &Ty::Enum(e.to_string())),
                PTy::Fn0 => self.expect(arg.value, &Ty::Fn(vec![], Box::new(Ty::Unit))),
                PTy::FnStr => self.expect(arg.value, &Ty::Fn(vec![Ty::Str], Box::new(Ty::Unit))),
                PTy::BindStr => {
                    self.expect(arg.value, &Ty::Str);
                    self.require_place(arg.value, &format!("`{}` is a two-way binding", prop.name));
                }
            }
        }
        if def.name == "Input" && !seen.contains(&"value") {
            self.warn("W0403", span, "Input without `value:` cannot hold text").help("add `value: someState`");
        }
    }

    fn view_use(&mut self, name: &str, sig: &[ParamSig], args: &[Arg], has_block: bool, name_span: Span) {
        let p = self.p;
        let mut seen: Vec<String> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let param = match &arg.name {
                None => match sig.iter().find(|s| s.ty != Ty::Slot) {
                    Some(s) if i == 0 => s.clone(),
                    _ => {
                        self.err("E0207", p.span(arg.value), format!("only the first argument of `{}` may be positional; write `name: value`", name));
                        continue;
                    }
                },
                Some(n) => match sig.iter().find(|s| s.name == *n && s.ty != Ty::Slot) {
                    Some(s) => s.clone(),
                    None if VIEW_OUTER_PROPS.contains(&n.as_str()) => {
                        let ty = match registry::COMMON.iter().find(|c| c.name == n).map(|c| c.ty) {
                            Some(PTy::Enum(e)) => Ty::Enum(e.to_string()),
                            Some(PTy::Str) => Ty::Str,
                            _ => Ty::Float,
                        };
                        self.expect(arg.value, &ty);
                        continue;
                    }
                    None => {
                        let names: Vec<&str> = sig.iter().filter(|s| s.ty != Ty::Slot).map(|s| s.name.as_str()).collect();
                        let alias = registry::alias(registry::PROP_ALIASES, n).filter(|a| names.contains(a));
                        let sugg = alias.or_else(|| closest(n, names.iter().copied())).map(|s| s.to_string());
                        let is_box_prop = registry::COMMON.iter().any(|c| c.name == n);
                        let list = if names.is_empty() { "none".to_string() } else { names.join(" ") };
                        let mut d = self.err("E0202", arg.name_span, format!("view `{}` has no parameter `{}`", name, n));
                        match sugg {
                            Some(s) => d.fix = Some(crate::diag::Fix { span: arg.name_span, replacement: s }),
                            None if is_box_prop => {
                                d.help = Some(format!(
                                    "`{}` is a view, not a built-in element: it takes only its own parameters ({}). Wrap it in a Column/Row to apply `{}`",
                                    name, list, n
                                ))
                            }
                            None => d.help = Some(format!("parameters of `{}`: {}", name, list)),
                        }
                        continue;
                    }
                },
            };
            if seen.contains(&param.name) {
                self.err("E0208", arg.name_span, format!("argument `{}` is given twice", param.name));
            }
            seen.push(param.name.clone());
            self.expect(arg.value, &param.ty);
            if param.bind {
                self.require_place(arg.value, &format!("`{}` of `{}` is a `bind` parameter", param.name, name));
            }
        }
        let missing: Vec<&str> = sig
            .iter()
            .filter(|s| !s.has_default && s.ty != Ty::Slot && !seen.contains(&s.name))
            .map(|s| s.name.as_str())
            .collect();
        if !missing.is_empty() {
            self.err("E0206", name_span, format!("`{}` is missing required argument(s): {}", name, missing.join(", ")));
        }
        if has_block && !sig.iter().any(|s| s.ty == Ty::Slot) {
            self.err("E0205", name_span, format!("view `{}` does not accept children", name))
                .help(format!("declare a slot parameter: `view {}(content: slot)` and place `content` in its body", name));
        }
    }

    // ---------------- places ----------------
    /// `None` if `e` can be assigned to; otherwise the reason it cannot.
    fn place_error(&self, e: ExprId) -> Option<String> {
        match self.p.expr(e) {
            Expr::Ident(n) => match self.lookup(n).map(|v| &v.kind) {
                Some(VarKind::State) | Some(VarKind::BindParam) | Some(VarKind::Local { mutable: true }) | Some(VarKind::Item { writable: true }) => None,
                Some(VarKind::Local { mutable: false }) => Some(format!("`{}` is immutable; declare it with `var`", n)),
                Some(VarKind::Param) => {
                    Some(format!("`{}` is a read-only parameter; to let the callee write to the caller's state declare it `bind {}: T`", n, n))
                }
                Some(VarKind::Derived) => Some(format!("`{}` is a derived `let`; assign to the state it is computed from", n)),
                Some(VarKind::Item { writable: false }) => Some(format!(
                    "`{}` iterates over a computed list, so it is a read-only copy; iterate over the state list itself, or update the state by id",
                    n
                )),
                Some(VarKind::Fn) | Some(VarKind::Slot) => Some(format!("`{}` is not a variable", n)),
                None => Some(format!("`{}` is not a mutable variable", n)),
            },
            Expr::Member(b, _, _) | Expr::Index(b, _) | Expr::Unary(UnOp::Unwrap, b) => self.place_error(*b),
            Expr::Error => None,
            _ => Some("this expression is a temporary value, not a variable".into()),
        }
    }

    fn require_place(&mut self, e: ExprId, why: &str) {
        if let Some(reason) = self.place_error(e) {
            let sp = self.p.span(e);
            self.err("E0401", sp, format!("{} and needs a mutable variable: {}", why, reason));
        }
    }

    // ---------------- statements ----------------
    fn block(&mut self, stmts: &[Stmt]) {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        let p = self.p;
        match s {
            Stmt::Let { name, mutable, ty, value, span } => {
                let t = self.declared_or_inferred(ty.as_ref(), *value, name, *span, if *mutable { "var" } else { "let" });
                self.declare(name, t, VarKind::Local { mutable: *mutable });
            }
            Stmt::Assign { target, op, value, span } => {
                let tty = self.check(*target, None);
                if let Some(reason) = self.place_error(*target) {
                    self.err("E0401", p.span(*target), format!("cannot assign: {}", reason));
                }
                match op {
                    AssignOp::Set => self.expect(*value, &tty),
                    AssignOp::Add | AssignOp::Sub => {
                        let ok = matches!(tty, Ty::Int | Ty::Float | Ty::Unknown) || (*op == AssignOp::Add && tty == Ty::Str);
                        if !ok {
                            self.err("E0303", *span, format!("`+=`/`-=` need a number (or String for `+=`), found {}", tty));
                        }
                        self.expect(*value, &tty);
                    }
                }
            }
            Stmt::Expr(e) => {
                self.check(*e, None);
            }
            Stmt::If { cond, then, els } => {
                self.expect(*cond, &Ty::Bool);
                self.block(then);
                self.block(els);
            }
            Stmt::For { var, iter, body, .. } => {
                let it = self.check(*iter, None);
                let item = match it {
                    Ty::List(t) => *t,
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.err("E0302", p.span(*iter), format!("`for` iterates over a list, found {}", other))
                            .help("for a counted loop use `for i in range(0, n)`");
                        Ty::Unknown
                    }
                };
                self.scopes.push(HashMap::new());
                self.declare(var, item, VarKind::Local { mutable: false });
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Return(e, span) => match (self.ret.clone(), e) {
                (None, _) => {
                    self.err("E0312", *span, "`return` outside of a function");
                }
                (Some(Some(Ty::Unit)), Some(e)) => {
                    let t = self.check(*e, None);
                    if t != Ty::Unit && t != Ty::Unknown {
                        self.err("E0309", *span, format!("this function has no return type but returns {}", t))
                            .help(format!("declare it: `fn name(...) -> {} {{ ... }}`", t));
                    }
                }
                (Some(Some(t)), Some(e)) => self.expect(*e, &t),
                (Some(Some(t)), None) => {
                    if t != Ty::Unit {
                        self.err("E0309", *span, format!("this function must return {}", t));
                    }
                }
                (Some(None), Some(e)) => {
                    let t = self.check(*e, self.inferred_ret.clone().as_ref());
                    if self.inferred_ret.is_none() {
                        self.inferred_ret = Some(t);
                    }
                }
                (Some(None), None) => {}
            },
        }
    }

    // ---------------- expressions ----------------
    /// Check `e` against `want`, reporting a mismatch with a targeted fix where possible.
    fn expect(&mut self, e: ExprId, want: &Ty) {
        let got = self.check(e, Some(want));
        if assignable(&got, want) {
            return;
        }
        let sp = self.p.span(e);
        let text = self.src.text(sp).to_string();
        let tokens = self.token_list();
        let mut d = self.err("E0301", sp, format!("expected {}, found {}", want, got));
        match (want, &got) {
            (Ty::Str, Ty::Int | Ty::Float | Ty::Bool) => {
                d.fix = Some(crate::diag::Fix { span: sp, replacement: format!("\"{{{}}}\"", text) });
            }
            (Ty::Int, Ty::Float) => d.help = Some("convert with `.round()`, `.floor()` or `.toInt()`".into()),
            (Ty::Float | Ty::Int, Ty::Str) => d.help = Some("parse with `.toInt() ?? 0` / `.toFloat() ?? 0.0`".into()),
            (Ty::Bool, Ty::Opt(_)) => {
                d.fix = Some(crate::diag::Fix { span: sp, replacement: format!("{} != nil", text) });
            }
            (w, Ty::Opt(inner)) if assignable(inner, w) => {
                d.help = Some("the value may be nil: provide a fallback with `?? value`, or unwrap with `!` after checking `!= nil`".into())
            }
            (Ty::Fn(ps, _), g) if ps.is_empty() && !matches!(g, Ty::Fn(..)) => {
                d.help = Some("a handler is a function: pass a fn name or wrap statements in braces, e.g. `onTap: { count += 1 }`".into())
            }
            (Ty::Color, _) => d.help = Some(format!("use a color literal (#2563EB) or a token: {}", tokens)),
            _ => {}
        }
    }

    fn token_list(&self) -> String {
        let mut v: Vec<String> = self.tokens.iter().map(|t| format!(".{}", t)).collect();
        v.sort();
        v.join(" ")
    }

    fn check(&mut self, e: ExprId, want: Option<&Ty>) -> Ty {
        let p = self.p;
        let sp = p.span(e);
        match p.expr(e) {
            Expr::Error => Ty::Unknown,
            Expr::Int(_) => Ty::Int,
            Expr::Float(_) => Ty::Float,
            Expr::Bool(_) => Ty::Bool,
            Expr::Nil => Ty::Nil,
            Expr::Color(_) => Ty::Color,
            Expr::Str(parts) => {
                for part in parts {
                    if let StrPart::Expr(x) = part {
                        let t = self.check(*x, None);
                        if matches!(t, Ty::Fn(..) | Ty::Slot | Ty::Unit) {
                            self.err("E0313", p.span(*x), format!("cannot interpolate a value of type {}", t));
                        }
                    }
                }
                Ty::Str
            }
            Expr::Ident(n) => {
                if let Some(v) = self.lookup(n) {
                    return v.ty.clone();
                }
                if let Some(t) = self.consts.get(n) {
                    return t.clone();
                }
                if let Some(f) = self.fns.get(n) {
                    return Ty::Fn(f.params.iter().map(|p| p.ty.clone()).collect(), Box::new(f.ret.clone()));
                }
                if let Some((ty, _)) = registry::global(n) {
                    if n == "safeArea" && sp.file != 0 {
                        self.uses_safe_area = true;
                    }
                    return match ty {
                        "Float" => Ty::Float,
                        "Bool" => Ty::Bool,
                        "String" => Ty::Str,
                        other => Ty::Struct(other.to_string()),
                    };
                }
                let names: Vec<String> = self.scopes.iter().flat_map(|s| s.keys().cloned()).chain(self.consts.keys().cloned()).chain(self.fns.keys().cloned()).collect();
                let sugg = closest(n, names.iter().map(|s| s.as_str())).map(|s| s.to_string());
                let is_type = self.structs.contains_key(n) || self.enums.contains_key(n);
                let mut d = self.err("E0203", sp, format!("unknown name `{}`", n));
                if is_type {
                    d.help = Some(format!("`{}` is a type; construct a struct with `{}(field: value)`, write enum values as `.variant`", n, n));
                } else if n == "self" || n == "this" {
                    d.help = Some("Vard has no `self`/`this`; refer to state and params by name".into());
                } else if let Some(s) = sugg {
                    d.fix = Some(crate::diag::Fix { span: sp, replacement: s });
                }
                Ty::Unknown
            }
            Expr::Dot(name) => {
                let target = match want {
                    Some(Ty::Opt(inner)) => Some(&**inner),
                    other => other,
                };
                match target {
                    Some(Ty::Enum(en)) => {
                        let vs = self.enums.get(en).cloned().unwrap_or_default();
                        if vs.iter().any(|v| v == name) {
                            self.dot_kinds.insert(e, DotKind::Enum);
                        } else {
                            let sugg = closest(name, vs.iter().map(|s| s.as_str())).map(|s| s.to_string());
                            let list = vs.iter().map(|v| format!(".{}", v)).collect::<Vec<_>>().join(" ");
                            let mut d = self.err("E0304", sp, format!("`{}` has no variant `.{}`", en, name));
                            match sugg {
                                Some(s) => d.fix = Some(crate::diag::Fix { span: sp, replacement: format!(".{}", s) }),
                                None => d.help = Some(format!("variants: {}", list)),
                            }
                        }
                        Ty::Enum(en.clone())
                    }
                    Some(Ty::Color) => {
                        if self.tokens.contains(name) {
                            self.dot_kinds.insert(e, DotKind::Color);
                        } else {
                            let sugg = registry::alias(registry::COLOR_ALIASES, name)
                                .map(|s| s.to_string())
                                .or_else(|| closest(name, self.tokens.iter().map(|s| s.as_str())).map(|s| s.to_string()));
                            let list = self.token_list();
                            let mut d = self.err("E0305", sp, format!("unknown color token `.{}`", name));
                            match sugg {
                                Some(s) => d.fix = Some(crate::diag::Fix { span: sp, replacement: format!(".{}", s) }),
                                None => d.help = Some(format!("tokens: {}; add your own in a `theme {{ name: #RRGGBB }}` block", list)),
                            }
                        }
                        Ty::Color
                    }
                    Some(Ty::Unknown) => Ty::Unknown,
                    Some(other) => {
                        let other = other.clone();
                        self.err("E0306", sp, format!("`.{}` shorthand needs an enum or Color here, but {} is expected", name, other));
                        Ty::Unknown
                    }
                    None => {
                        self.err("E0306", sp, format!("cannot tell which type `.{}` belongs to", name))
                            .help("use it where the type is known (an argument, a typed variable, or the right side of `==`)");
                        Ty::Unknown
                    }
                }
            }
            Expr::List(items) => {
                let mut elem = match want {
                    Some(Ty::List(t)) => Some((**t).clone()),
                    _ => None,
                };
                for it in items {
                    match &elem {
                        Some(t) => {
                            let t = t.clone();
                            self.expect(*it, &t)
                        }
                        None => elem = Some(self.check(*it, None)),
                    }
                }
                Ty::List(Box::new(elem.unwrap_or(Ty::Unknown)))
            }
            Expr::Map(pairs) => {
                let mut elem = match want {
                    Some(Ty::Map(t)) => Some((**t).clone()),
                    _ => None,
                };
                for (k, v) in pairs {
                    self.expect(*k, &Ty::Str);
                    match &elem {
                        Some(t) => {
                            let t = t.clone();
                            self.expect(*v, &t)
                        }
                        None => elem = Some(self.check(*v, None)),
                    }
                }
                Ty::Map(Box::new(elem.unwrap_or(Ty::Unknown)))
            }
            Expr::Unary(op, x) => match op {
                UnOp::Not => {
                    self.expect(*x, &Ty::Bool);
                    Ty::Bool
                }
                UnOp::Neg => {
                    let t = self.check(*x, want);
                    if !matches!(t, Ty::Int | Ty::Float | Ty::Unknown) {
                        self.err("E0303", sp, format!("`-` needs a number, found {}", t));
                        return Ty::Unknown;
                    }
                    t
                }
                UnOp::Unwrap => match self.check(*x, None) {
                    Ty::Opt(t) => *t,
                    Ty::Unknown => Ty::Unknown,
                    t => {
                        self.warn("W0404", sp, format!("`!` is unnecessary: the value is {} and can never be nil", t));
                        t
                    }
                },
            },
            Expr::Binary(op, a, b) => self.binary(*op, *a, *b, sp, want),
            Expr::Ternary(c, a, b) => {
                self.expect(*c, &Ty::Bool);
                match want {
                    Some(w) if *w != Ty::Unknown => {
                        self.expect(*a, w);
                        self.expect(*b, w);
                        w.clone()
                    }
                    _ => {
                        let ta = self.check(*a, None);
                        let tb = self.check(*b, Some(&ta));
                        if assignable(&tb, &ta) {
                            ta
                        } else if assignable(&ta, &tb) {
                            tb
                        } else {
                            self.err("E0301", p.span(*b), format!("both branches of `? :` must have one type: {} vs {}", ta, tb));
                            Ty::Unknown
                        }
                    }
                }
            }
            Expr::Member(recv, name, nsp) => {
                let rt = self.check(*recv, None);
                self.member(&rt, name, *nsp)
            }
            Expr::Index(recv, idx) => {
                let rt = self.check(*recv, None);
                match rt {
                    Ty::List(t) => {
                        self.expect(*idx, &Ty::Int);
                        *t
                    }
                    Ty::Map(t) => {
                        self.expect(*idx, &Ty::Str);
                        Ty::Opt(t)
                    }
                    Ty::Unknown => {
                        self.check(*idx, None);
                        Ty::Unknown
                    }
                    other => {
                        self.check(*idx, None);
                        self.err("E0303", sp, format!("only lists and maps can be indexed, found {}", other));
                        Ty::Unknown
                    }
                }
            }
            Expr::Call { callee, args } => self.call(*callee, args, sp),
            Expr::Lambda { .. } => match want {
                Some(Ty::Fn(ps, r)) => {
                    let (ps, r) = (ps.clone(), (**r).clone());
                    let ret = self.lambda(e, &ps, Some(&r));
                    Ty::Fn(ps, Box::new(if r == Ty::Unit { Ty::Unit } else { ret }))
                }
                Some(Ty::Opt(inner)) if matches!(**inner, Ty::Fn(..)) => {
                    let inner = (**inner).clone();
                    self.check(e, Some(&inner))
                }
                _ => {
                    let Expr::Lambda { params, .. } = p.expr(e) else { unreachable!() };
                    if params.is_empty() {
                        self.lambda(e, &[], Some(&Ty::Unit));
                        Ty::Fn(vec![], Box::new(Ty::Unit))
                    } else {
                        self.err("E0307", sp, "cannot infer the parameter types of this lambda")
                            .help("pass it directly where a function is expected (e.g. `list.filter(x => ...)`) or declare a `fn`");
                        Ty::Unknown
                    }
                }
            },
        }
    }

    /// Check a lambda literal against parameter types; returns its result type.
    fn lambda(&mut self, e: ExprId, param_tys: &[Ty], ret: Option<&Ty>) -> Ty {
        let p = self.p;
        let Expr::Lambda { params, body } = p.expr(e) else {
            // a function reference instead of a literal
            let t = self.check(e, None);
            return match t {
                Ty::Fn(ps, r) => {
                    if ps.len() != param_tys.len() || !ps.iter().zip(param_tys).all(|(a, b)| assignable(b, a)) {
                        let want = Ty::Fn(param_tys.to_vec(), Box::new(ret.cloned().unwrap_or(Ty::Unknown)));
                        self.err("E0301", p.span(e), format!("expected {}, found {}", want, Ty::Fn(ps, r.clone())));
                    }
                    *r
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    self.err("E0301", p.span(e), format!("expected a function, found {}", other))
                        .help("write a lambda: `x => expression`");
                    Ty::Unknown
                }
            };
        };
        // A parameterless `{ ... }` block may be used where arguments are passed (they are ignored).
        if !params.is_empty() && params.len() != param_tys.len() {
            self.err("E0310", p.span(e), format!("this lambda takes {} parameter(s) but {} are passed", params.len(), param_tys.len()));
        }
        self.scopes.push(HashMap::new());
        for (i, name) in params.iter().enumerate() {
            self.declare(name, param_tys.get(i).cloned().unwrap_or(Ty::Unknown), VarKind::Param);
        }
        let out = match body {
            LambdaBody::Expr(x) => match ret {
                Some(r) if *r != Ty::Unit => {
                    self.expect(*x, r);
                    r.clone()
                }
                _ => self.check(*x, None),
            },
            LambdaBody::Block(stmts) => {
                let saved_ret = self.ret.replace(ret.cloned());
                let saved_inf = self.inferred_ret.take();
                self.block(stmts);
                let inferred = self.inferred_ret.take();
                self.ret = saved_ret;
                self.inferred_ret = saved_inf;
                ret.cloned().or(inferred).unwrap_or(Ty::Unit)
            }
        };
        self.scopes.pop();
        out
    }

    fn binary(&mut self, op: BinOp, a: ExprId, b: ExprId, sp: Span, want: Option<&Ty>) -> Ty {
        let p = self.p;
        match op {
            BinOp::And | BinOp::Or => {
                self.expect(a, &Ty::Bool);
                self.expect(b, &Ty::Bool);
                Ty::Bool
            }
            BinOp::Eq | BinOp::Ne => {
                let (first, second) = if matches!(p.expr(a), Expr::Dot(_) | Expr::Nil) { (b, a) } else { (a, b) };
                let t1 = self.check(first, None);
                let t2 = self.check(second, Some(&t1));
                if !assignable(&t2, &t1) && !assignable(&t1, &t2) {
                    self.err("E0301", sp, format!("cannot compare {} with {}", t1, t2));
                }
                Ty::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let ta = self.check(a, None);
                let tb = self.check(b, None);
                let num = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Unknown);
                let ok = (num(&ta) && num(&tb)) || (matches!(ta, Ty::Str | Ty::Unknown) && matches!(tb, Ty::Str | Ty::Unknown));
                if !ok {
                    self.err("E0303", sp, format!("cannot order {} and {} (numbers or strings only)", ta, tb));
                }
                Ty::Bool
            }
            BinOp::Coalesce => {
                let ta = self.check(a, want.map(|w| Ty::Opt(Box::new(w.clone()))).as_ref());
                match ta {
                    Ty::Opt(inner) => {
                        self.expect(b, &inner);
                        *inner
                    }
                    Ty::Nil | Ty::Unknown => self.check(b, want),
                    other => {
                        self.warn("W0404", sp, format!("`??` is unnecessary: the left side is {} and can never be nil", other));
                        self.check(b, Some(&other));
                        other
                    }
                }
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                let ta = self.check(a, None);
                let tb = self.check(b, None);
                match (&ta, &tb) {
                    (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                    (Ty::Int, Ty::Int) => Ty::Int,
                    (Ty::Int | Ty::Float, Ty::Int | Ty::Float) => Ty::Float,
                    (Ty::Str, Ty::Str) if op == BinOp::Add => Ty::Str,
                    (Ty::Str, _) | (_, Ty::Str) if op == BinOp::Add => {
                        let (ta_s, tb_s) = (self.src.text(p.span(a)).to_string(), self.src.text(p.span(b)).to_string());
                        let part = |t: &Ty, s: &str, id: ExprId| match (t, p.expr(id)) {
                            (Ty::Str, Expr::Str(_)) => s[1..s.len() - 1].to_string(),
                            _ => format!("{{{}}}", s),
                        };
                        let fixed = format!("\"{}{}\"", part(&ta, &ta_s, a), part(&tb, &tb_s, b));
                        self.err("E0303", sp, format!("cannot add {} and {}; build strings with interpolation", ta, tb))
                            .fix = Some(crate::diag::Fix { span: sp, replacement: fixed });
                        Ty::Str
                    }
                    _ => {
                        self.err("E0303", sp, format!("operator needs numbers, found {} and {}", ta, tb));
                        Ty::Unknown
                    }
                }
            }
        }
    }

    fn member(&mut self, rt: &Ty, name: &str, nsp: Span) -> Ty {
        let found = match (rt, name) {
            (Ty::Unknown, _) => Some(Ty::Unknown),
            (Ty::Str | Ty::List(_) | Ty::Map(_), "count") => Some(Ty::Int),
            (Ty::Str | Ty::List(_) | Ty::Map(_), "isEmpty") => Some(Ty::Bool),
            (Ty::List(t), "first" | "last") => Some(Ty::Opt(t.clone())),
            (Ty::Map(_), "keys") => Some(Ty::List(Box::new(Ty::Str))),
            (Ty::Map(t), "values") => Some(Ty::List(t.clone())),
            (Ty::Struct(s), _) => self.structs.get(s).and_then(|f| f.iter().find(|f| f.name == name)).map(|f| f.ty.clone()),
            _ => None,
        };
        if let Some(t) = found {
            return t;
        }
        if let Ty::Opt(inner) = rt {
            self.err("E0314", nsp, format!("the value is {}? and may be nil; cannot read `.{}`", inner, name))
                .help("give a fallback first: `(x ?? fallback).field`, or check `x != nil` and unwrap: `x!.field`");
            return Ty::Unknown;
        }
        let props = self.members_of(rt, false);
        let methods = self.members_of(rt, true);
        if methods.iter().any(|m| m == name) {
            self.err("E0215", nsp, format!("`{}` is a method; call it: `.{}(...)`", name, name));
            return Ty::Unknown;
        }
        let alias = registry::alias(registry::MEMBER_ALIASES, name).filter(|a| props.iter().any(|p| p == a));
        let sugg = alias.map(|s| s.to_string()).or_else(|| closest(name, props.iter().map(|s| s.as_str())).map(|s| s.to_string()));
        let mut d = self.err("E0214", nsp, format!("{} has no property `{}`", rt, name));
        match sugg {
            Some(s) => d.fix = Some(crate::diag::Fix { span: nsp, replacement: s }),
            None => d.help = Some(members_help(rt, &props, &methods)),
        }
        Ty::Unknown
    }

    fn members_of(&self, t: &Ty, methods: bool) -> Vec<String> {
        let v: &[&str] = match (t, methods) {
            (Ty::Str, false) => &["count", "isEmpty"],
            (Ty::Str, true) => &["trim", "upper", "lower", "contains", "startsWith", "endsWith", "split", "toInt", "toFloat"],
            (Ty::List(_), false) => &["count", "isEmpty", "first", "last"],
            (Ty::List(_), true) => &[
                "contains", "indexOf", "map", "filter", "find", "any", "all", "sum", "join", "sortedBy", "reversed", "append", "insert",
                "removeAt", "remove", "removeWhere", "clear",
            ],
            (Ty::Map(_), false) => &["count", "isEmpty", "keys", "values"],
            (Ty::Map(_), true) => &["contains", "remove", "clear"],
            (Ty::Int, true) => &["toFloat", "toString", "abs"],
            (Ty::Float, true) => &["toInt", "round", "floor", "ceil", "fixed", "toString", "abs"],
            (Ty::Struct(s), false) => {
                return self.structs.get(s).map(|f| f.iter().map(|f| f.name.clone()).collect()).unwrap_or_default();
            }
            _ => &[],
        };
        v.iter().map(|s| s.to_string()).collect()
    }

    fn call(&mut self, callee: ExprId, args: &[Arg], sp: Span) -> Ty {
        let p = self.p;
        match p.expr(callee) {
            Expr::Ident(name) => {
                if let Some(v) = self.lookup(name).cloned() {
                    return match v.ty {
                        Ty::Fn(ps, r) => {
                            let sig: Vec<ParamSig> =
                                ps.into_iter().map(|ty| ParamSig { name: String::new(), ty, has_default: false, bind: false }).collect();
                            self.call_args(name, &sig, args, sp);
                            *r
                        }
                        Ty::Unknown => Ty::Unknown,
                        other => {
                            self.err("E0315", sp, format!("`{}` is {}, not a function", name, other));
                            Ty::Unknown
                        }
                    };
                }
                if let Some(fields) = self.structs.get(name).cloned() {
                    self.call_args(name, &fields, args, sp);
                    return Ty::Struct(name.clone());
                }
                if let Some(sig) = self.fns.get(name).cloned() {
                    self.call_args(name, &sig.params, args, sp);
                    return sig.ret;
                }
                match name.as_str() {
                    "print" => {
                        for a in args {
                            self.check(a.value, None);
                        }
                        return Ty::Unit;
                    }
                    "range" => {
                        let sig = [Ty::Int, Ty::Int].map(|ty| ParamSig { name: String::new(), ty, has_default: false, bind: false });
                        self.call_args(name, &sig, args, sp);
                        return Ty::List(Box::new(Ty::Int));
                    }
                    "min" | "max" => {
                        if args.len() != 2 {
                            self.err("E0316", sp, format!("`{}` takes 2 arguments", name));
                            return Ty::Unknown;
                        }
                        let ta = self.check(args[0].value, None);
                        let tb = self.check(args[1].value, None);
                        return match (&ta, &tb) {
                            (Ty::Int, Ty::Int) => Ty::Int,
                            (Ty::Int | Ty::Float | Ty::Unknown, Ty::Int | Ty::Float | Ty::Unknown) => Ty::Float,
                            _ => {
                                self.err("E0303", sp, format!("`{}` needs numbers, found {} and {}", name, ta, tb));
                                Ty::Unknown
                            }
                        };
                    }
                    _ => {}
                }
                if self.views.contains_key(name) || registry::element(name).is_some() {
                    self.err("E0212", sp, format!("`{}` is a UI element; elements can only appear in a view body, not in expressions", name));
                    return Ty::Unknown;
                }
                let names: Vec<String> = self
                    .fns
                    .keys()
                    .cloned()
                    .chain(self.structs.keys().cloned())
                    .chain(self.scopes.iter().flat_map(|s| s.iter().filter(|(_, v)| v.kind == VarKind::Fn).map(|(k, _)| k.clone())))
                    .chain(BUILTIN_FNS.iter().map(|s| s.to_string()))
                    .collect();
                let sugg = closest(name, names.iter().map(|s| s.as_str())).map(|s| s.to_string());
                let mut d = self.err("E0203", p.span(callee), format!("unknown function `{}`", name));
                if let Some(s) = sugg {
                    d.fix = Some(crate::diag::Fix { span: p.span(callee), replacement: s });
                }
                for a in args {
                    if !matches!(p.expr(a.value), Expr::Dot(_) | Expr::Lambda { .. }) {
                        self.check(a.value, None);
                    }
                }
                Ty::Unknown
            }
            Expr::Member(recv, name, nsp) => {
                let rt = self.check(*recv, None);
                self.method(*recv, &rt, name, *nsp, args, sp)
            }
            _ => match self.check(callee, None) {
                Ty::Fn(ps, r) => {
                    let sig: Vec<ParamSig> = ps.into_iter().map(|ty| ParamSig { name: String::new(), ty, has_default: false, bind: false }).collect();
                    self.call_args("function", &sig, args, sp);
                    *r
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    self.err("E0315", sp, format!("cannot call a value of type {}", other));
                    Ty::Unknown
                }
            },
        }
    }

    /// Match call arguments (positional first, then named) against a signature.
    fn call_args(&mut self, what: &str, sig: &[ParamSig], args: &[Arg], sp: Span) {
        let p = self.p;
        let mut filled = vec![false; sig.len()];
        for (i, a) in args.iter().enumerate() {
            let idx = match &a.name {
                Some(n) => match sig.iter().position(|s| s.name == *n) {
                    Some(i) => i,
                    None => {
                        let names: Vec<&str> = sig.iter().map(|s| s.name.as_str()).filter(|s| !s.is_empty()).collect();
                        let sugg = closest(n, names.iter().copied()).map(|s| s.to_string());
                        let mut d = self.err("E0202", a.name_span, format!("`{}` has no parameter `{}`", what, n));
                        match sugg {
                            Some(s) => d.fix = Some(crate::diag::Fix { span: a.name_span, replacement: s }),
                            None => d.help = Some(format!("parameters: {}", names.join(", "))),
                        }
                        continue;
                    }
                },
                None => i,
            };
            if idx >= sig.len() {
                self.err("E0316", p.span(a.value), format!("`{}` takes {} argument(s), but {} are given", what, sig.len(), args.len()));
                break;
            }
            if filled[idx] {
                self.err("E0208", p.span(a.value), format!("argument `{}` is given twice", sig[idx].name));
            }
            filled[idx] = true;
            let ty = sig[idx].ty.clone();
            self.expect(a.value, &ty);
        }
        let missing: Vec<String> = sig
            .iter()
            .enumerate()
            .filter(|(i, s)| !filled[*i] && !s.has_default)
            .map(|(i, s)| if s.name.is_empty() { format!("#{}", i + 1) } else { format!("{}: {}", s.name, s.ty) })
            .collect();
        if !missing.is_empty() {
            self.err("E0206", sp, format!("`{}` is missing argument(s): {}", what, missing.join(", ")));
        }
    }

    fn method(&mut self, recv: ExprId, rt: &Ty, name: &str, nsp: Span, args: &[Arg], sp: Span) -> Ty {
        let p = self.p;
        let simple = |s: &mut Checker, params: &[Ty], ret: Ty| -> Ty {
            let sig: Vec<ParamSig> = params.iter().map(|ty| ParamSig { name: String::new(), ty: ty.clone(), has_default: false, bind: false }).collect();
            s.call_args(name, &sig, args, sp);
            ret
        };
        let one_lambda = |s: &mut Checker, param: &Ty, ret: Option<&Ty>| -> Ty {
            if args.len() != 1 {
                s.err("E0316", sp, format!("`{}` takes one function argument, e.g. `.{}(x => ...)`", name, name));
                return Ty::Unknown;
            }
            s.lambda(args[0].value, std::slice::from_ref(param), ret)
        };
        match (rt, name) {
            (Ty::Unknown, _) => {
                for a in args {
                    if !matches!(p.expr(a.value), Expr::Dot(_) | Expr::Lambda { .. }) {
                        self.check(a.value, None);
                    }
                }
                Ty::Unknown
            }
            (Ty::Str, "trim" | "upper" | "lower") => simple(self, &[], Ty::Str),
            (Ty::Str, "contains" | "startsWith" | "endsWith") => simple(self, &[Ty::Str], Ty::Bool),
            (Ty::Str, "split") => simple(self, &[Ty::Str], Ty::List(Box::new(Ty::Str))),
            (Ty::Str, "toInt") => simple(self, &[], Ty::Opt(Box::new(Ty::Int))),
            (Ty::Str, "toFloat") => simple(self, &[], Ty::Opt(Box::new(Ty::Float))),
            (Ty::Int, "toFloat") => simple(self, &[], Ty::Float),
            (Ty::Int, "abs") => simple(self, &[], Ty::Int),
            (Ty::Int | Ty::Float, "toString") => simple(self, &[], Ty::Str),
            (Ty::Float, "toInt" | "round" | "floor" | "ceil") => simple(self, &[], Ty::Int),
            (Ty::Float, "fixed") => simple(self, &[Ty::Int], Ty::Str),
            (Ty::Float, "abs") => simple(self, &[], Ty::Float),
            (Ty::List(t), _) => {
                let t = (**t).clone();
                let mutating = matches!(name, "append" | "insert" | "removeAt" | "remove" | "removeWhere" | "clear");
                if mutating {
                    if let Some(reason) = self.place_error(recv) {
                        self.err("E0402", nsp, format!("`.{}` changes the list in place and needs a mutable variable: {}", name, reason));
                    }
                }
                match name {
                    "contains" => simple(self, &[t], Ty::Bool),
                    "indexOf" => simple(self, &[t], Ty::Opt(Box::new(Ty::Int))),
                    "map" => {
                        let r = one_lambda(self, &t, None);
                        Ty::List(Box::new(r))
                    }
                    "filter" => {
                        one_lambda(self, &t, Some(&Ty::Bool));
                        Ty::List(Box::new(t))
                    }
                    "find" => {
                        one_lambda(self, &t, Some(&Ty::Bool));
                        Ty::Opt(Box::new(t))
                    }
                    "any" | "all" => {
                        one_lambda(self, &t, Some(&Ty::Bool));
                        Ty::Bool
                    }
                    "sortedBy" => {
                        let k = one_lambda(self, &t, None);
                        if !matches!(k, Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Unknown) {
                            self.err("E0311", sp, format!("`sortedBy` key must be a number, String or Bool, found {}", k));
                        }
                        Ty::List(Box::new(t))
                    }
                    "sum" => {
                        if !matches!(t, Ty::Int | Ty::Float | Ty::Unknown) {
                            self.err("E0303", sp, format!("`sum` needs a list of numbers, found [{}]", t)).help("map first: `.map(x => x.price).sum()`");
                        }
                        simple(self, &[], t)
                    }
                    "join" => {
                        if !matches!(t, Ty::Str | Ty::Unknown) {
                            self.err("E0303", sp, format!("`join` needs a list of String, found [{}]", t)).help("map first: `.map(x => \"{x}\").join(\", \")`");
                        }
                        simple(self, &[Ty::Str], Ty::Str)
                    }
                    "reversed" => simple(self, &[], Ty::List(Box::new(t))),
                    "append" | "remove" => simple(self, &[t], Ty::Unit),
                    "insert" => simple(self, &[Ty::Int, t], Ty::Unit),
                    "removeAt" => simple(self, &[Ty::Int], Ty::Unit),
                    "removeWhere" => {
                        one_lambda(self, &t, Some(&Ty::Bool));
                        Ty::Unit
                    }
                    "clear" => simple(self, &[], Ty::Unit),
                    _ => self.unknown_method(rt, name, nsp, args),
                }
            }
            (Ty::Map(_), "contains") => simple(self, &[Ty::Str], Ty::Bool),
            (Ty::Map(_), "remove" | "clear") => {
                if let Some(reason) = self.place_error(recv) {
                    self.err("E0402", nsp, format!("`.{}` changes the map in place and needs a mutable variable: {}", name, reason));
                }
                if name == "remove" {
                    simple(self, &[Ty::Str], Ty::Unit)
                } else {
                    simple(self, &[], Ty::Unit)
                }
            }
            (Ty::Struct(s), _) => {
                // calling a function-typed field
                match self.structs.get(s).and_then(|f| f.iter().find(|f| f.name == name)).map(|f| f.ty.clone()) {
                    Some(Ty::Fn(ps, r)) => simple(self, &ps, *r),
                    _ => self.unknown_method(rt, name, nsp, args),
                }
            }
            _ => self.unknown_method(rt, name, nsp, args),
        }
    }

    fn unknown_method(&mut self, rt: &Ty, name: &str, nsp: Span, args: &[Arg]) -> Ty {
        let p = self.p;
        for a in args {
            if !matches!(p.expr(a.value), Expr::Dot(_) | Expr::Lambda { .. }) {
                self.check(a.value, None);
            }
        }
        if let Ty::Opt(inner) = rt {
            self.err("E0314", nsp, format!("the value is {}? and may be nil; cannot call `.{}`", inner, name))
                .help("give a fallback first: `(x ?? fallback).method()`, or check `x != nil` and unwrap: `x!.method()`");
            return Ty::Unknown;
        }
        let props = self.members_of(rt, false);
        let methods = self.members_of(rt, true);
        if props.iter().any(|m| m == name) && args.is_empty() {
            // `list.count()` -> `list.count`
            let end = Span::new(nsp.file, nsp.end, nsp.end + 2);
            let has_parens = self.src.text(end) == "()";
            let mut d = self.err("E0215", nsp, format!("`{}` is a property, not a method; remove `()`", name));
            if has_parens {
                d.fix = Some(crate::diag::Fix { span: end, replacement: String::new() });
            }
            return self.member(rt, name, nsp);
        }
        let alias = registry::alias(registry::MEMBER_ALIASES, name);
        if let Some(a) = alias.filter(|a| props.iter().any(|p| p == a)) {
            // e.g. `.length()` -> `.count`
            let end = Span::new(nsp.file, nsp.start, nsp.end + 2);
            let has_parens = self.src.text(end).ends_with("()");
            let mut d = self.err("E0214", nsp, format!("{} has no method `{}`; use the property `{}`", rt, name, a));
            if has_parens {
                d.fix = Some(crate::diag::Fix { span: end, replacement: a.to_string() });
            }
            return Ty::Unknown;
        }
        let sugg = alias
            .filter(|a| methods.iter().any(|m| m == a))
            .map(|s| s.to_string())
            .or_else(|| closest(name, methods.iter().map(|s| s.as_str())).map(|s| s.to_string()));
        let mut d = self.err("E0214", nsp, format!("{} has no method `{}`", rt, name));
        match sugg {
            Some(s) => d.fix = Some(crate::diag::Fix { span: nsp, replacement: s }),
            None => {
                d.help = Some(members_help(rt, &props, &methods))
            }
        }
        Ty::Unknown
    }
}

fn members_help(rt: &Ty, props: &[String], methods: &[String]) -> String {
    match (props.is_empty(), methods.is_empty()) {
        (true, true) => format!("{} has no properties or methods", rt),
        (false, true) => format!("properties of {}: {}", rt, props.join(" ")),
        (true, false) => format!("methods of {}: {}", rt, methods.join(" ")),
        (false, false) => format!("{} has properties: {}; methods: {}", rt, props.join(" "), methods.join(" ")),
    }
}

fn is_slot(t: &TypeRef) -> bool {
    matches!(t, TypeRef::Named(n, _) if n == "slot")
}

/// Values that can cross the native boundary / be persisted (JSON-representable).
fn is_data(t: &Ty) -> bool {
    match t {
        Ty::Int | Ty::Float | Ty::Str | Ty::Bool | Ty::Color | Ty::Struct(_) | Ty::Enum(_) | Ty::Unknown => true,
        Ty::List(x) | Ty::Map(x) | Ty::Opt(x) => is_data(x),
        _ => false,
    }
}
