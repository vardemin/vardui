//! Expression/statement interpreter (the reference "dev" execution engine).
//! The checker guarantees types, so this is dynamically typed and never panics: any
//! runtime fault becomes an `R` diagnostic with a source location and yields `nil`.

use crate::rt::{Binding, Runtime, ScopeId, SigId};
use crate::value::{Func, StructVal, Value};
use std::rc::Rc;
use vard_lang::ast::*;
use vard_lang::diag::Span;

pub(crate) enum Root {
    Sig(SigId),
    Local(ScopeId, Rc<str>),
}

pub(crate) enum Seg {
    Field(Rc<str>),
    Index(usize),
    /// map key; writing a missing key inserts it
    Key(Rc<str>),
}

/// An assignable location: a state signal or a local, plus a path into its value.
pub(crate) struct Place {
    root: Root,
    path: Vec<Seg>,
}

enum Flow {
    Next,
    Return(Value),
}

fn path_mut<'v>(v: &'v mut Value, path: &[Seg]) -> Option<&'v mut Value> {
    let Some((seg, rest)) = path.split_first() else { return Some(v) };
    match (seg, v) {
        (Seg::Field(f), Value::Struct(s)) => {
            let s = Rc::make_mut(s);
            let slot = s.fields.iter_mut().find(|(n, _)| n == f)?;
            path_mut(&mut slot.1, rest)
        }
        (Seg::Index(i), Value::List(l)) => {
            let l = Rc::make_mut(l);
            path_mut(l.get_mut(*i)?, rest)
        }
        (Seg::Key(k), Value::Map(m)) => {
            let m = Rc::make_mut(m);
            let i = match m.iter().position(|(n, _)| n == k) {
                Some(i) => i,
                None => {
                    m.push((k.clone(), Value::Nil));
                    m.len() - 1
                }
            };
            path_mut(&mut m[i].1, rest)
        }
        _ => None,
    }
}

/// After a write through a map key: a key set to `nil` is removed.
fn prune_nil_keys(v: &mut Value, path: &[Seg]) {
    let Some((seg, rest)) = path.split_first() else { return };
    match (seg, v) {
        (Seg::Field(f), Value::Struct(s)) => {
            if let Some(slot) = Rc::make_mut(s).fields.iter_mut().find(|(n, _)| n == f) {
                prune_nil_keys(&mut slot.1, rest);
            }
        }
        (Seg::Index(i), Value::List(l)) => {
            if let Some(x) = Rc::make_mut(l).get_mut(*i) {
                prune_nil_keys(x, rest);
            }
        }
        (Seg::Key(k), Value::Map(m)) => {
            let m = Rc::make_mut(m);
            if rest.is_empty() {
                m.retain(|(n, v)| n != k || !matches!(v, Value::Nil));
            } else if let Some(slot) = m.iter_mut().find(|(n, _)| n == k) {
                prune_nil_keys(&mut slot.1, rest);
            }
        }
        _ => {}
    }
}

fn path_get(mut v: Value, path: &[Seg]) -> Value {
    for seg in path {
        v = match (seg, &v) {
            (Seg::Field(f), Value::Struct(s)) => s.fields.iter().find(|(n, _)| n == f).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
            (Seg::Index(i), Value::List(l)) => l.get(*i).cloned().unwrap_or(Value::Nil),
            (Seg::Key(k), Value::Map(m)) => m.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
    }
    v
}

impl Runtime {
    // ---------------- expressions ----------------
    pub(crate) fn eval(&mut self, p: &Program, e: ExprId, sc: ScopeId) -> Value {
        match p.expr(e) {
            Expr::Int(v) => Value::Int(*v),
            Expr::Float(v) => Value::Float(*v),
            Expr::Bool(v) => Value::Bool(*v),
            Expr::Nil | Expr::Error => Value::Nil,
            Expr::Color(c) => Value::Color(*c),
            Expr::Str(parts) => {
                if let [StrPart::Lit(s)] = parts.as_slice() {
                    return Value::str(s);
                }
                let mut out = String::new();
                for part in parts {
                    match part {
                        StrPart::Lit(s) => out.push_str(s),
                        StrPart::Expr(x) => out.push_str(&self.eval(p, *x, sc).display()),
                    }
                }
                Value::str(&out)
            }
            Expr::Ident(name) => self.read_var(p, name, sc, e),
            Expr::Dot(name) => match p.dot_kinds.get(&e) {
                Some(DotKind::Color) => self.token_tracked(name),
                _ => Value::Enum(Rc::from(name.as_str())),
            },
            Expr::List(items) => Value::List(Rc::new(items.iter().map(|i| self.eval(p, *i, sc)).collect())),
            Expr::Map(pairs) => {
                let mut out: Vec<(Rc<str>, Value)> = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let k: Rc<str> = Rc::from(self.eval(p, *k, sc).display().as_str());
                    let v = self.eval(p, *v, sc);
                    match out.iter_mut().find(|(n, _)| *n == k) {
                        Some(slot) => slot.1 = v,
                        None => out.push((k, v)),
                    }
                }
                Value::Map(Rc::new(out))
            }
            Expr::Unary(op, x) => {
                let v = self.eval(p, *x, sc);
                match (op, v) {
                    (UnOp::Not, v) => Value::Bool(!v.truthy()),
                    (UnOp::Neg, Value::Int(i)) => Value::Int(i.wrapping_neg()),
                    (UnOp::Neg, Value::Float(f)) => Value::Float(-f),
                    (UnOp::Unwrap, Value::Nil) => {
                        self.error("R0001", p.span(e), "`!` unwrapped nil; check `!= nil` first or use `?? fallback`");
                        Value::Nil
                    }
                    (_, v) => v,
                }
            }
            Expr::Binary(op, a, b) => self.binary(p, *op, *a, *b, sc, e),
            Expr::Ternary(c, a, b) => {
                if self.eval(p, *c, sc).truthy() {
                    self.eval(p, *a, sc)
                } else {
                    self.eval(p, *b, sc)
                }
            }
            Expr::Member(recv, name, _) => {
                let r = self.eval(p, *recv, sc);
                match (&r, name.as_str()) {
                    (Value::Str(s), "count") => Value::Int(s.chars().count() as i64),
                    (Value::Str(s), "isEmpty") => Value::Bool(s.is_empty()),
                    (Value::List(l), "count") => Value::Int(l.len() as i64),
                    (Value::List(l), "isEmpty") => Value::Bool(l.is_empty()),
                    (Value::List(l), "first") => l.first().cloned().unwrap_or(Value::Nil),
                    (Value::List(l), "last") => l.last().cloned().unwrap_or(Value::Nil),
                    (Value::Map(m), "count") => Value::Int(m.len() as i64),
                    (Value::Map(m), "isEmpty") => Value::Bool(m.is_empty()),
                    (Value::Map(m), "keys") => Value::List(Rc::new(m.iter().map(|(k, _)| Value::Str(k.clone())).collect())),
                    (Value::Map(m), "values") => Value::List(Rc::new(m.iter().map(|(_, v)| v.clone()).collect())),
                    (Value::Struct(s), f) => s.fields.iter().find(|(n, _)| &**n == f).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
                    _ => Value::Nil,
                }
            }
            Expr::Index(recv, idx) => {
                let r = self.eval(p, *recv, sc);
                let i = self.eval(p, *idx, sc);
                match (&r, &i) {
                    (Value::Map(m), Value::Str(k)) => m.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or(Value::Nil),
                    (Value::List(l), Value::Int(i)) => match l.get(*i as usize) {
                        Some(v) if *i >= 0 => v.clone(),
                        _ => {
                            self.error("R0001", p.span(e), format!("index {} is out of range (list has {} items)", i, l.len()));
                            Value::Nil
                        }
                    },
                    _ => Value::Nil,
                }
            }
            Expr::Call { callee, args } => self.call(p, e, *callee, args, sc),
            Expr::Lambda { .. } => Value::Func(Rc::new(Func::Lambda { expr: e, scope: sc })),
        }
    }

    fn read_var(&mut self, p: &Program, name: &str, sc: ScopeId, e: ExprId) -> Value {
        if let Some((_, b)) = self.lookup(sc, name) {
            return self.read_binding(p, b);
        }
        if let Some(&i) = p.const_index.get(name) {
            if let Some(v) = self.consts.get(&i) {
                return v.clone();
            }
            let root = self.root_scope;
            let v = self.untracked(|rt| rt.eval(p, p.consts[i].value, root));
            self.consts.insert(i, v.clone());
            return v;
        }
        if let Some(&i) = p.fn_index.get(name) {
            return Value::Func(Rc::new(Func::Global(i)));
        }
        if let Some(&i) = p.extern_index.get(name) {
            return Value::Func(Rc::new(Func::Extern(i)));
        }
        match name {
            "safeArea" => return self.sig_get(self.insets_sig),
            "keyboardHeight" => return self.sig_get(self.keyboard_sig),
            "backProgress" => return self.sig_get(self.back_sig),
            "screenWidth" | "screenHeight" => {
                let s = self.sig_get(self.screen_sig);
                let i = if name == "screenWidth" { 0 } else { 1 };
                return match s {
                    Value::List(l) => l.get(i).cloned().unwrap_or(Value::Float(0.0)),
                    _ => Value::Float(0.0),
                };
            }
            "isDark" => {
                let t = self.theme_sig;
                return self.sig_get(t);
            }
            "platform" => return Value::Str(self.platform.clone()),
            _ => {}
        }
        self.error("R0007", p.span(e), format!("`{}` is not available here", name));
        Value::Nil
    }

    fn read_binding(&mut self, p: &Program, b: Binding) -> Value {
        match b {
            Binding::Sig(s) => self.sig_get(s),
            Binding::Comp(c) => self.comp_get(p, c),
            Binding::Val(v) => v,
            Binding::Bind { expr, scope } => self.eval(p, expr, scope),
            Binding::Item { value, .. } => self.sig_get(value),
            Binding::Slot { .. } => Value::Nil,
        }
    }

    fn binary(&mut self, p: &Program, op: BinOp, a: ExprId, b: ExprId, sc: ScopeId, e: ExprId) -> Value {
        match op {
            BinOp::And => return Value::Bool(self.eval(p, a, sc).truthy() && self.eval(p, b, sc).truthy()),
            BinOp::Or => return Value::Bool(self.eval(p, a, sc).truthy() || self.eval(p, b, sc).truthy()),
            BinOp::Coalesce => {
                let l = self.eval(p, a, sc);
                return if matches!(l, Value::Nil) { self.eval(p, b, sc) } else { l };
            }
            _ => {}
        }
        let l = self.eval(p, a, sc);
        let r = self.eval(p, b, sc);
        self.arith(op, l, r, p.span(e))
    }

    pub(crate) fn arith(&mut self, op: BinOp, l: Value, r: Value, span: Span) -> Value {
        use std::cmp::Ordering;
        let ord = |l: &Value, r: &Value| -> Option<Ordering> {
            match (l, r) {
                (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
                (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
                (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
                _ => l.as_f64()?.partial_cmp(&r.as_f64()?),
            }
        };
        match op {
            BinOp::Eq => Value::Bool(l == r),
            BinOp::Ne => Value::Bool(l != r),
            BinOp::Lt => Value::Bool(ord(&l, &r) == Some(Ordering::Less)),
            BinOp::Le => Value::Bool(matches!(ord(&l, &r), Some(Ordering::Less | Ordering::Equal))),
            BinOp::Gt => Value::Bool(ord(&l, &r) == Some(Ordering::Greater)),
            BinOp::Ge => Value::Bool(matches!(ord(&l, &r), Some(Ordering::Greater | Ordering::Equal))),
            _ => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => match op {
                    BinOp::Add => Value::Int(a.wrapping_add(*b)),
                    BinOp::Sub => Value::Int(a.wrapping_sub(*b)),
                    BinOp::Mul => Value::Int(a.wrapping_mul(*b)),
                    BinOp::Div | BinOp::Rem if *b == 0 => {
                        self.error("R0008", span, "division by zero");
                        Value::Int(0)
                    }
                    BinOp::Div => Value::Int(a.wrapping_div(*b)),
                    _ => Value::Int(a.wrapping_rem(*b)),
                },
                (Value::Str(a), Value::Str(b)) => Value::str(&format!("{}{}", a, b)),
                _ => match (l.as_f64(), r.as_f64()) {
                    (Some(a), Some(b)) => Value::Float(match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => a / b,
                        _ => a % b,
                    }),
                    _ => Value::Nil,
                },
            },
        }
    }

    // ---------------- calls ----------------
    fn ordered_args(&mut self, p: &Program, params: &[Param], args: &[Arg], sc: ScopeId) -> Vec<Option<Value>> {
        let mut out: Vec<Option<Value>> = vec![None; params.len().max(args.len())];
        for (i, a) in args.iter().enumerate() {
            let idx = match &a.name {
                Some(n) => params.iter().position(|pa| pa.name == *n).unwrap_or(i),
                None => i,
            };
            out[idx] = Some(self.eval(p, a.value, sc));
        }
        out
    }

    fn call(&mut self, p: &Program, e: ExprId, callee: ExprId, args: &[Arg], sc: ScopeId) -> Value {
        match p.expr(callee) {
            Expr::Ident(name) => {
                if let Some((_, b)) = self.lookup(sc, name) {
                    let f = self.read_binding(p, b);
                    let argv = self.args_for(p, &f, args, sc);
                    return self.call_func(p, &f, argv, p.span(e));
                }
                if let Some(&si) = p.struct_index.get(name) {
                    let decl = &p.structs[si];
                    let mut argv = self.ordered_args(p, &decl.fields, args, sc);
                    let root = self.root_scope;
                    let fields = decl
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let v = match (argv.get_mut(i).and_then(|a| a.take()), f.default) {
                                (Some(v), _) => v,
                                (None, Some(d)) => self.eval(p, d, root),
                                (None, None) => Value::Nil,
                            };
                            (Rc::from(f.name.as_str()), v)
                        })
                        .collect();
                    return Value::Struct(Rc::new(StructVal { name: Rc::from(name.as_str()), fields }));
                }
                if let Some(&fi) = p.fn_index.get(name) {
                    let argv = self.ordered_args(p, &p.fns[fi].params, args, sc);
                    return self.call_decl(p, &p.fns[fi], None, argv, p.span(e));
                }
                if let Some(&xi) = p.extern_index.get(name) {
                    let argv = self.ordered_args(p, &p.externs[xi].params, args, sc);
                    return self.call_extern(p, xi, argv);
                }
                let argv: Vec<Value> = args.iter().map(|a| self.eval(p, a.value, sc)).collect();
                match (name.as_str(), argv.as_slice()) {
                    ("print", vals) => {
                        let line = vals.iter().map(|v| v.display()).collect::<Vec<_>>().join(" ");
                        self.log.push(line);
                        Value::Unit
                    }
                    ("range", [Value::Int(a), Value::Int(b)]) => Value::List(Rc::new((*a..*b).map(Value::Int).collect())),
                    ("min", [a, b]) => {
                        if self.arith(BinOp::Le, a.clone(), b.clone(), p.span(e)).truthy() {
                            a.clone()
                        } else {
                            b.clone()
                        }
                    }
                    ("max", [a, b]) => {
                        if self.arith(BinOp::Ge, a.clone(), b.clone(), p.span(e)).truthy() {
                            a.clone()
                        } else {
                            b.clone()
                        }
                    }
                    _ => Value::Nil,
                }
            }
            Expr::Member(recv, name, _) => self.method(p, e, *recv, name, args, sc),
            _ => {
                let f = self.eval(p, callee, sc);
                let argv = self.args_for(p, &f, args, sc);
                self.call_func(p, &f, argv, p.span(e))
            }
        }
    }

    /// Evaluate arguments for a function value, honouring named arguments when the
    /// target is a declared fn.
    fn args_for(&mut self, p: &Program, f: &Value, args: &[Arg], sc: ScopeId) -> Vec<Option<Value>> {
        let params: &[Param] = match f {
            Value::Func(func) => match &**func {
                Func::Global(i) => &p.fns[*i].params,
                Func::ViewFn { view, member, .. } => match &p.views[*view].members[*member] {
                    ViewMember::Fn(d) => &d.params,
                    _ => &[],
                },
                Func::Extern(i) => &p.externs[*i].params,
                Func::Lambda { .. } => &[],
            },
            _ => &[],
        };
        self.ordered_args(p, params, args, sc)
    }

    pub(crate) fn call_func(&mut self, p: &Program, f: &Value, argv: Vec<Option<Value>>, span: Span) -> Value {
        let Value::Func(func) = f else { return Value::Nil };
        match &**func {
            Func::Lambda { expr, scope } => {
                let Expr::Lambda { params, body } = p.expr(*expr) else { return Value::Nil };
                if !self.enter(span) {
                    return Value::Nil;
                }
                let sc = self.new_scope(Some(*scope), None, None);
                for (name, v) in params.iter().zip(argv) {
                    self.declare(sc, name, Binding::Val(v.unwrap_or(Value::Nil)));
                }
                let out = match body {
                    LambdaBody::Expr(x) => self.eval(p, *x, sc),
                    LambdaBody::Block(stmts) => match self.exec(p, stmts, sc) {
                        Flow::Return(v) => v,
                        Flow::Next => Value::Unit,
                    },
                };
                self.dispose_scope(sc);
                self.depth -= 1;
                out
            }
            Func::Global(i) => self.call_decl(p, &p.fns[*i], None, argv, span),
            Func::ViewFn { view, member, scope } => match &p.views[*view].members[*member] {
                ViewMember::Fn(d) => self.call_decl(p, d, Some(*scope), argv, span),
                _ => Value::Nil,
            },
            Func::Extern(i) => self.call_extern(p, *i, argv),
        }
    }

    fn enter(&mut self, span: Span) -> bool {
        if self.depth >= 200 {
            self.error("R0005", span, "call depth exceeded 200 (infinite recursion?)");
            return false;
        }
        self.depth += 1;
        true
    }

    fn call_decl(&mut self, p: &Program, d: &FnDecl, lex: Option<ScopeId>, mut argv: Vec<Option<Value>>, span: Span) -> Value {
        if !self.enter(span) {
            return Value::Nil;
        }
        let sc = self.new_scope(lex, None, None);
        for (i, param) in d.params.iter().enumerate() {
            let v = match (argv.get_mut(i).and_then(|a| a.take()), param.default) {
                (Some(v), _) => v,
                (None, Some(def)) => self.eval(p, def, sc),
                (None, None) => Value::Nil,
            };
            self.declare(sc, &param.name, Binding::Val(v));
        }
        let out = match self.exec(p, &d.body, sc) {
            Flow::Return(v) => v,
            Flow::Next => Value::Unit,
        };
        self.dispose_scope(sc);
        self.depth -= 1;
        out
    }

    /// Run the body of `on event(params) { ... }` (member `member` of view `view`).
    pub(crate) fn run_handler(&mut self, p: &Program, view: usize, member: usize, scope: ScopeId, args: &[Value]) {
        let ViewMember::On { params, body, span, .. } = &p.views[view].members[member] else { return };
        if !self.enter(*span) {
            return;
        }
        let sc = self.new_scope(Some(scope), None, None);
        for (i, name) in params.iter().enumerate() {
            self.declare(sc, name, Binding::Val(args.get(i).cloned().unwrap_or(Value::Nil)));
        }
        self.exec(p, body, sc);
        self.dispose_scope(sc);
        self.depth -= 1;
    }

    fn call1(&mut self, p: &Program, f: &Value, arg: Value, span: Span) -> Value {
        self.call_func(p, f, vec![Some(arg)], span)
    }

    fn method(&mut self, p: &Program, e: ExprId, recv: ExprId, name: &str, args: &[Arg], sc: ScopeId) -> Value {
        let span = p.span(e);
        let argv: Vec<Value> = args.iter().map(|a| self.eval(p, a.value, sc)).collect();
        let arg = |i: usize| argv.get(i).cloned().unwrap_or(Value::Nil);
        // in-place list / map mutation
        if matches!(name, "append" | "insert" | "removeAt" | "remove" | "removeWhere" | "clear") {
            let Some(place) = self.place(p, recv, sc) else {
                self.error("R0009", span, format!("`.{}` needs a mutable list or map", name));
                return Value::Unit;
            };
            let cur = self.place_get(&place);
            if let Value::Map(_) = cur {
                drop(cur);
                let key = arg(0).display();
                self.place_update(&place, span, |v| {
                    if let Value::Map(m) = v {
                        let m = Rc::make_mut(m);
                        if name == "clear" {
                            m.clear();
                        } else {
                            m.retain(|(k, _)| **k != *key);
                        }
                    }
                });
                return Value::Unit;
            }
            let Value::List(cur) = cur else { return Value::Unit };
            let drop_mask: Vec<bool> = match name {
                "removeWhere" => cur.iter().map(|it| self.call1(p, &arg(0), it.clone(), span).truthy()).collect(),
                _ => Vec::new(),
            };
            let len = cur.len();
            drop(cur);
            let mut fault: Option<String> = None;
            self.place_update(&place, span, |v| {
                let Value::List(l) = v else { return };
                let l = Rc::make_mut(l);
                match name {
                    "append" => l.push(arg(0)),
                    "insert" => match arg(0) {
                        Value::Int(i) if i >= 0 && i as usize <= len => l.insert(i as usize, arg(1)),
                        other => fault = Some(format!("insert index {} is out of range (list has {} items)", other.repr(), len)),
                    },
                    "removeAt" => match arg(0) {
                        Value::Int(i) if i >= 0 && (i as usize) < len => {
                            l.remove(i as usize);
                        }
                        other => fault = Some(format!("removeAt index {} is out of range (list has {} items)", other.repr(), len)),
                    },
                    "remove" => {
                        let x = arg(0);
                        if let Some(i) = l.iter().position(|it| *it == x) {
                            l.remove(i);
                        }
                    }
                    "removeWhere" => {
                        let mut i = 0;
                        l.retain(|_| {
                            i += 1;
                            !drop_mask[i - 1]
                        });
                    }
                    _ => l.clear(),
                }
            });
            if let Some(f) = fault {
                self.error("R0001", span, f);
            }
            return Value::Unit;
        }
        let r = self.eval(p, recv, sc);
        match (&r, name) {
            (Value::Map(m), "contains") => Value::Bool(m.iter().any(|(k, _)| **k == *arg(0).display())),
            (Value::Str(s), "trim") => Value::str(s.trim()),
            (Value::Str(s), "upper") => Value::str(&s.to_uppercase()),
            (Value::Str(s), "lower") => Value::str(&s.to_lowercase()),
            (Value::Str(s), "contains") => Value::Bool(matches!(arg(0), Value::Str(x) if s.contains(&*x))),
            (Value::Str(s), "startsWith") => Value::Bool(matches!(arg(0), Value::Str(x) if s.starts_with(&*x))),
            (Value::Str(s), "endsWith") => Value::Bool(matches!(arg(0), Value::Str(x) if s.ends_with(&*x))),
            (Value::Str(s), "split") => match arg(0) {
                Value::Str(sep) if !sep.is_empty() => Value::List(Rc::new(s.split(&*sep).map(Value::str).collect())),
                _ => Value::List(Rc::new(s.chars().map(|c| Value::str(&c.to_string())).collect())),
            },
            (Value::Str(s), "toInt") => s.trim().parse::<i64>().map(Value::Int).unwrap_or(Value::Nil),
            (Value::Str(s), "toFloat") => s.trim().parse::<f64>().map(Value::Float).unwrap_or(Value::Nil),
            (Value::Int(i), "toFloat") => Value::Float(*i as f64),
            (Value::Int(i), "abs") => Value::Int(i.wrapping_abs()),
            (Value::Int(_) | Value::Float(_), "toString") => Value::str(&r.display()),
            (Value::Int(i), "toInt" | "round" | "floor" | "ceil") => Value::Int(*i),
            (Value::Float(f), "toInt") => Value::Int(f.trunc() as i64),
            (Value::Float(f), "round") => Value::Int(f.round() as i64),
            (Value::Float(f), "floor") => Value::Int(f.floor() as i64),
            (Value::Float(f), "ceil") => Value::Int(f.ceil() as i64),
            (Value::Float(f), "abs") => Value::Float(f.abs()),
            (Value::Int(_) | Value::Float(_), "fixed") => {
                let digits = match arg(0) {
                    Value::Int(d) => d.clamp(0, 12) as usize,
                    _ => 0,
                };
                Value::str(&format!("{:.*}", digits, r.as_f64().unwrap_or(0.0)))
            }
            (Value::List(l), _) => {
                let l = l.clone();
                match name {
                    "contains" => Value::Bool(l.contains(&arg(0))),
                    "indexOf" => l.iter().position(|x| *x == arg(0)).map(|i| Value::Int(i as i64)).unwrap_or(Value::Nil),
                    "map" => Value::List(Rc::new(l.iter().map(|x| self.call1(p, &arg(0), x.clone(), span)).collect())),
                    "filter" => Value::List(Rc::new(l.iter().filter(|x| self.call1(p, &arg(0), (*x).clone(), span).truthy()).cloned().collect())),
                    "find" => l.iter().find(|x| self.call1(p, &arg(0), (*x).clone(), span).truthy()).cloned().unwrap_or(Value::Nil),
                    "any" => Value::Bool(l.iter().any(|x| self.call1(p, &arg(0), x.clone(), span).truthy())),
                    "all" => Value::Bool(l.iter().all(|x| self.call1(p, &arg(0), x.clone(), span).truthy())),
                    "sum" => l.iter().fold(Value::Int(0), |acc, x| self.arith(BinOp::Add, acc, x.clone(), span)),
                    "join" => {
                        let sep = arg(0).display();
                        Value::str(&l.iter().map(|x| x.display()).collect::<Vec<_>>().join(&sep))
                    }
                    "reversed" => Value::List(Rc::new(l.iter().rev().cloned().collect())),
                    "sortedBy" => {
                        let mut keyed: Vec<(Value, Value)> = l.iter().map(|x| (self.call1(p, &arg(0), x.clone(), span), x.clone())).collect();
                        keyed.sort_by(|a, b| {
                            if self.arith(BinOp::Lt, a.0.clone(), b.0.clone(), span).truthy() {
                                std::cmp::Ordering::Less
                            } else if a.0 == b.0 {
                                std::cmp::Ordering::Equal
                            } else {
                                std::cmp::Ordering::Greater
                            }
                        });
                        Value::List(Rc::new(keyed.into_iter().map(|(_, v)| v).collect()))
                    }
                    _ => Value::Nil,
                }
            }
            (Value::Struct(s), f) => {
                // a function stored in a struct field
                let fv = s.fields.iter().find(|(n, _)| &**n == f).map(|(_, v)| v.clone()).unwrap_or(Value::Nil);
                self.call_func(p, &fv, argv.into_iter().map(Some).collect(), span)
            }
            _ => Value::Nil,
        }
    }

    // ---------------- places ----------------
    pub(crate) fn place(&mut self, p: &Program, e: ExprId, sc: ScopeId) -> Option<Place> {
        match p.expr(e) {
            Expr::Ident(name) => {
                let (owner, b) = self.lookup(sc, name)?;
                match b {
                    Binding::Sig(s) => Some(Place { root: Root::Sig(s), path: Vec::new() }),
                    Binding::Val(_) => Some(Place { root: Root::Local(owner, Rc::from(name.as_str())), path: Vec::new() }),
                    Binding::Bind { expr, scope } => self.place(p, expr, scope),
                    Binding::Item { src, scope, index, .. } => {
                        let mut pl = self.place(p, src, scope)?;
                        if let Value::Int(i) = self.sig_peek(index) {
                            pl.path.push(Seg::Index(i as usize));
                        }
                        Some(pl)
                    }
                    _ => None,
                }
            }
            Expr::Member(b, name, _) => {
                let mut pl = self.place(p, *b, sc)?;
                pl.path.push(Seg::Field(Rc::from(name.as_str())));
                Some(pl)
            }
            Expr::Index(b, i) => {
                let mut pl = self.place(p, *b, sc)?;
                match self.eval(p, *i, sc) {
                    Value::Int(i) if i >= 0 => pl.path.push(Seg::Index(i as usize)),
                    Value::Str(k) => pl.path.push(Seg::Key(k)),
                    _ => return None,
                }
                Some(pl)
            }
            Expr::Unary(UnOp::Unwrap, b) => self.place(p, *b, sc),
            _ => None,
        }
    }

    fn place_get(&mut self, pl: &Place) -> Value {
        let root = match &pl.root {
            Root::Sig(s) => self.sig_get(*s),
            Root::Local(sc, name) => self
                .scopes
                .get(*sc)
                .and_then(|s| s.vars.iter().rev().find(|(n, _)| n == name))
                .and_then(|(_, b)| if let Binding::Val(v) = b { Some(v.clone()) } else { None })
                .unwrap_or(Value::Nil),
        };
        path_get(root, &pl.path)
    }

    /// Mutate the value at `pl` in place (copy-on-write only where shared), then notify.
    fn place_update(&mut self, pl: &Place, span: Span, f: impl FnOnce(&mut Value)) {
        match &pl.root {
            Root::Sig(s) => {
                if matches!(self.observers.last(), Some(Some(_))) {
                    self.error(
                        "R0003",
                        span,
                        "state was modified while the UI was being computed; mutate state only in event handlers or fns called from them",
                    );
                    return;
                }
                let Some(sig) = self.signals.get_mut(*s) else { return };
                // Take the value out so the Rc is unique and `make_mut` does not copy.
                let mut v = std::mem::replace(&mut sig.value, Value::Nil);
                let ok = match path_mut(&mut v, &pl.path) {
                    Some(target) => {
                        f(target);
                        true
                    }
                    None => false,
                };
                if pl.path.iter().any(|s| matches!(s, Seg::Key(_))) {
                    prune_nil_keys(&mut v, &pl.path);
                }
                self.signals[*s].value = v;
                if ok {
                    self.notify(*s);
                } else {
                    self.error("R0001", span, "assignment target no longer exists (index out of range or nil)");
                }
            }
            Root::Local(sc, name) => {
                let slot = self.scopes.get_mut(*sc).and_then(|s| s.vars.iter_mut().rev().find(|(n, _)| n == name));
                if let Some((_, Binding::Val(v))) = slot {
                    if let Some(target) = path_mut(v, &pl.path) {
                        f(target);
                    }
                    if pl.path.iter().any(|s| matches!(s, Seg::Key(_))) {
                        prune_nil_keys(v, &pl.path);
                    }
                }
            }
        }
    }

    pub(crate) fn assign(&mut self, p: &Program, target: ExprId, sc: ScopeId, v: Value) {
        let span = p.span(target);
        let Some(pl) = self.place(p, target, sc) else {
            self.error("R0009", span, "cannot assign: not a mutable variable");
            return;
        };
        if let (Root::Sig(s), true) = (&pl.root, pl.path.is_empty()) {
            if matches!(self.observers.last(), Some(Some(_))) {
                self.error("R0003", span, "state was modified while the UI was being computed; mutate state only in event handlers or fns called from them");
                return;
            }
            self.sig_set(*s, v); // equality cut-off: no notification if unchanged
            return;
        }
        if self.untracked(|rt| rt.place_get(&pl)) == v {
            return;
        }
        self.place_update(&pl, span, |slot| *slot = v);
    }

    // ---------------- statements ----------------
    /// A nested block (`if`/`for` body) runs in its own scope, so its locals end with it —
    /// unless a closure captured them (the scope is then pinned, see `native::pin`).
    fn block(&mut self, p: &Program, stmts: &[Stmt], sc: ScopeId, bind: Option<(&str, Value)>) -> Flow {
        let inner = self.new_scope(Some(sc), None, None);
        if let Some((name, v)) = bind {
            self.declare(inner, name, Binding::Val(v));
        }
        let f = self.exec(p, stmts, inner);
        self.dispose_scope(inner);
        f
    }

    fn exec(&mut self, p: &Program, stmts: &[Stmt], sc: ScopeId) -> Flow {
        let mut flow = Flow::Next;
        for s in stmts {
            match s {
                Stmt::Let { name, value, .. } => {
                    let v = self.eval(p, *value, sc);
                    self.declare(sc, name, Binding::Val(v));
                }
                Stmt::Assign { target, op, value, span } => {
                    let mut v = self.eval(p, *value, sc);
                    if *op != AssignOp::Set {
                        let cur = self.untracked(|rt| rt.eval(p, *target, sc));
                        let bop = if *op == AssignOp::Add { BinOp::Add } else { BinOp::Sub };
                        v = self.arith(bop, cur, v, *span);
                    }
                    self.assign(p, *target, sc, v);
                }
                Stmt::Expr(e) => {
                    self.eval(p, *e, sc);
                }
                Stmt::If { cond, then, els } => {
                    let branch = if self.eval(p, *cond, sc).truthy() { then } else { els };
                    if let Flow::Return(v) = self.block(p, branch, sc, None) {
                        flow = Flow::Return(v);
                        break;
                    }
                }
                Stmt::For { var, iter, body, .. } => {
                    let Value::List(items) = self.eval(p, *iter, sc) else { continue };
                    let mut ret = None;
                    for it in items.iter() {
                        if let Flow::Return(v) = self.block(p, body, sc, Some((var, it.clone()))) {
                            ret = Some(v);
                            break;
                        }
                    }
                    if let Some(v) = ret {
                        flow = Flow::Return(v);
                        break;
                    }
                }
                Stmt::Return(e, _) => {
                    flow = Flow::Return(e.map(|e| self.eval(p, e, sc)).unwrap_or(Value::Unit));
                    break;
                }
            }
        }
        flow
    }
}
