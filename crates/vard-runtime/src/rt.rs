//! Reactive core and tree construction.
//!
//! Model (fine-grained, no virtual DOM, no re-rendering of views):
//!   * `state`            -> Signal
//!   * `let`, view params -> Computed (lazy, cached, auto-tracked)
//!   * each dynamic prop  -> Effect that writes one field of one Node
//!   * `if` / `for`       -> structural Effects that add/remove child scopes
//!
//! A view body runs exactly once per instance. Afterwards only the effects whose
//! dependencies changed run, each touching exactly the nodes it owns.
//!
//! Ownership is a tree of `Scope`s; disposing a scope frees everything created under it.

use crate::json::Json;
use crate::native::{Callbacks, Deferred, Handler, NativeHost, Pending, Sim, SAFE_ALL};
use crate::node::{Kind, Node};
use crate::text::TextEngine;
use crate::value::{Func, StructVal, Value};
use slotmap::{new_key_type, SlotMap};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;
use vard_lang::ast::*;
use vard_lang::check::VIEW_OUTER_PROPS;
use vard_lang::diag::{Sources, Span};
use vard_lang::registry;
use vard_lang::Compilation;

new_key_type! {
    pub struct SigId;
    pub struct CompId;
    pub struct EffId;
    pub struct ScopeId;
    pub struct NodeKey;
    pub struct FragId;
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Obs {
    Eff(EffId),
    Comp(CompId),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Src {
    Sig(SigId),
    Comp(CompId),
}

pub(crate) struct Signal {
    pub value: Value,
    pub subs: Vec<Obs>,
    pub name: Option<Rc<str>>,
    /// `pref` name: writes are persisted to the platform preferences.
    pub persist: Option<Rc<str>>,
}

pub(crate) struct Computed {
    pub expr: ExprId,
    pub scope: ScopeId,
    pub value: Option<Value>,
    pub deps: Vec<Src>,
    pub subs: Vec<Obs>,
    pub computing: bool,
}

pub(crate) struct ItemRec {
    pub key: String,
    pub scope: ScopeId,
    pub frag: FragId,
    pub index: SigId,
    /// The item's current value. Updated with an equality cut-off on every reconcile, so
    /// changing one element of a 10k list re-runs only that row's effects.
    pub value: SigId,
}

pub(crate) enum EffKind {
    Prop { node: NodeKey, name: &'static str, expr: ExprId, scope: ScopeId, last: Option<Value> },
    If { ui: UiId, scope: ScopeId, frag: FragId, parent: NodeKey, current: Option<(bool, ScopeId)> },
    For { ui: UiId, list: CompId, scope: ScopeId, frag: FragId, parent: NodeKey, items: Vec<ItemRec> },
    Running,
}

pub(crate) struct Effect {
    pub kind: EffKind,
    pub deps: Vec<Src>,
    pub seq: u64,
}

#[derive(Clone)]
pub(crate) enum Binding {
    Sig(SigId),
    Comp(CompId),
    Val(Value),
    /// `bind` parameter: an assignable expression in the caller's scope.
    Bind { expr: ExprId, scope: ScopeId },
    /// `for` item: element `index` of the loop's list; writes go through `src`.
    Item { value: SigId, src: ExprId, scope: ScopeId, index: SigId },
    Slot { nodes: Rc<[UiId]>, scope: ScopeId },
}

#[derive(Default)]
pub(crate) struct Scope {
    pub lex: Option<ScopeId>,
    pub owner: Option<ScopeId>,
    pub vars: Vec<(Rc<str>, Binding)>,
    pub kids: Vec<ScopeId>,
    pub effs: Vec<EffId>,
    pub comps: Vec<CompId>,
    pub sigs: Vec<SigId>,
    pub nodes: Vec<NodeKey>,
    pub frags: Vec<FragId>,
    pub view: Option<Rc<str>>,
    /// A temporary call scope captured by a callback still waiting for a native result.
    /// Disposal is postponed until the last pin goes (`zombie`).
    pub pins: u32,
    pub zombie: bool,
}

pub(crate) enum FItem {
    Node(NodeKey),
    Frag(FragId),
}

#[derive(Default)]
pub(crate) struct Frag {
    pub items: Vec<FItem>,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub effects_run: u64,
    pub layouts: u64,
    pub nodes_created: u64,
}

pub struct Runtime {
    pub prog: Rc<Program>,
    pub sources: Rc<Sources>,
    pub(crate) signals: SlotMap<SigId, Signal>,
    pub(crate) comps: SlotMap<CompId, Computed>,
    pub(crate) effects: SlotMap<EffId, Effect>,
    pub(crate) scopes: SlotMap<ScopeId, Scope>,
    pub nodes: SlotMap<NodeKey, Node>,
    pub(crate) frags: SlotMap<FragId, Frag>,
    pub(crate) observers: Vec<Option<Obs>>,
    queue: BTreeMap<u64, EffId>,
    seq: u64,
    next_uid: u32,
    pub root: NodeKey,
    pub(crate) root_scope: ScopeId,
    pub(crate) theme_sig: SigId,
    /// BCP 47 language tag; t() and the `locale` global follow it
    pub(crate) locale_sig: SigId,
    pub tokens: HashMap<String, (u32, u32)>,
    pub(crate) consts: HashMap<usize, Value>,
    pub log: Vec<String>,
    pub errors: Vec<String>,
    pub native_calls: Vec<String>,
    pub mocks: HashMap<String, Json>,
    pub text: TextEngine,
    pub screen: (f32, f32),
    pub layout_dirty: bool,
    pub(crate) depth: u32,
    pub stats: Stats,
    // ---- platform ----
    pub(crate) host: Option<Box<dyn NativeHost>>,
    pub platform: Rc<str>,
    pub sim: Sim,
    pub(crate) callbacks: Callbacks,
    pub(crate) next_callback: u64,
    pub(crate) pending: Vec<Pending>,
    pub(crate) deferred: Deferred,
    pub(crate) handlers: Vec<Handler>,
    pub(crate) appear: VecDeque<Handler>,
    /// system bars + cutout: top, bottom, left, right (dp)
    pub insets: [f32; 4],
    pub keyboard: f32,
    pub(crate) insets_sig: SigId,
    pub(crate) keyboard_sig: SigId,
    pub(crate) back_sig: SigId,
    /// [width, height]
    pub(crate) screen_sig: SigId,
    pub(crate) system_dark: bool,
    pub(crate) scheme_override: Option<bool>,
    pub(crate) prefs: HashMap<String, SigId>,
    pub(crate) dirty_prefs: Vec<SigId>,
    /// theme `font: "..."` — default family for Text/Input
    pub default_font: Option<Rc<str>>,
    /// project directory for `Image("assets/...")` in the headless rasterizer
    pub asset_dir: Option<std::path::PathBuf>,
    pub(crate) root_view: Option<String>,
    pub(crate) next_push: u64,
    /// Input with keyboard focus
    pub focused: Option<NodeKey>,
    // ---- animation ----
    /// animation clock, ms; moved only by `advance` (host frames, `wait` in tests)
    pub clock: f64,
    pub(crate) tweens: Vec<crate::anim::Tween>,
}

impl Runtime {
    pub fn new(c: Compilation, width: f32, height: f32) -> Runtime {
        let mut tokens: HashMap<String, (u32, u32)> = HashMap::new();
        for (n, l, d) in registry::COLOR_TOKENS {
            tokens.insert(n.to_string(), (*l, *d));
        }
        for dark in [false, true] {
            for t in c.program.themes.iter().filter(|t| t.dark == dark) {
                for (name, e, _) in &t.entries {
                    if let Expr::Color(col) = c.program.expr(*e) {
                        let entry = tokens.entry(name.clone()).or_insert((*col, *col));
                        if dark {
                            entry.1 = *col;
                        } else {
                            // a light override of a custom token also seeds dark until `theme dark` overrides it
                            let custom = !registry::COLOR_TOKENS.iter().any(|(n, _, _)| n == name);
                            entry.0 = *col;
                            if custom {
                                entry.1 = *col;
                            }
                        }
                    }
                }
            }
        }
        let default_font = c.program.themes.iter().flat_map(|t| t.entries.iter()).find(|(n, _, _)| n == "font").and_then(|(_, e, _)| {
            match c.program.expr(*e) {
                Expr::Str(parts) => match parts.as_slice() {
                    [StrPart::Lit(s)] => Some(Rc::from(s.as_str())),
                    _ => None,
                },
                _ => None,
            }
        });
        let mut signals: SlotMap<SigId, Signal> = SlotMap::with_key();
        let mut scopes: SlotMap<ScopeId, Scope> = SlotMap::with_key();
        let mut frags: SlotMap<FragId, Frag> = SlotMap::with_key();
        let mut nodes: SlotMap<NodeKey, Node> = SlotMap::with_key();
        let root_scope = scopes.insert(Scope::default());
        let mut sys = |v: Value| signals.insert(Signal { value: v, subs: Vec::new(), name: None, persist: None });
        let theme_sig = sys(Value::Bool(false));
        let locale_sig = sys(Value::Str(Rc::from("en")));
        let zero_insets = Value::Struct(Rc::new(StructVal {
            name: Rc::from("Insets"),
            fields: ["top", "bottom", "left", "right"].iter().map(|f| (Rc::from(*f), Value::Float(0.0))).collect(),
        }));
        let insets_sig = sys(zero_insets);
        let keyboard_sig = sys(Value::Float(0.0));
        let back_sig = sys(Value::Float(0.0));
        let screen_sig = sys(Value::List(Rc::new(vec![Value::Float(width as f64), Value::Float(height as f64)])));
        let root_frag = frags.insert(Frag::default());
        let mut root_node = Node::new(0, Kind::Column, root_frag, Span::default());
        if !c.program.uses_safe_area {
            // The app does not manage insets: keep its content out of the system bars.
            root_node.safe = SAFE_ALL;
        }
        let root = nodes.insert(root_node);
        Runtime {
            prog: Rc::new(c.program),
            sources: Rc::new(c.sources),
            signals,
            comps: SlotMap::with_key(),
            effects: SlotMap::with_key(),
            scopes,
            nodes,
            frags,
            observers: Vec::new(),
            queue: BTreeMap::new(),
            seq: 0,
            next_uid: 1,
            root,
            root_scope,
            theme_sig,
            tokens,
            consts: HashMap::new(),
            log: Vec::new(),
            errors: Vec::new(),
            native_calls: Vec::new(),
            mocks: HashMap::new(),
            text: TextEngine::new(),
            screen: (width, height),
            layout_dirty: true,
            depth: 0,
            stats: Stats::default(),
            host: None,
            platform: Rc::from("headless"),
            sim: Sim::default(),
            callbacks: HashMap::new(),
            next_callback: 0,
            pending: Vec::new(),
            deferred: VecDeque::new(),
            handlers: Vec::new(),
            appear: VecDeque::new(),
            insets: [0.0; 4],
            keyboard: 0.0,
            insets_sig,
            keyboard_sig,
            back_sig,
            screen_sig,
            locale_sig,
            system_dark: false,
            scheme_override: None,
            prefs: HashMap::new(),
            dirty_prefs: Vec::new(),
            default_font,
            asset_dir: None,
            root_view: None,
            next_push: 0,
            focused: None,
            clock: 0.0,
            tweens: Vec::new(),
        }
    }

    /// Tear the UI down and mount the root view again, keeping platform storage (prefs,
    /// secure values, permissions) — what a cold app restart looks like to the app.
    pub fn restart(&mut self) -> Result<(), String> {
        let view = self.root_view.clone().ok_or("nothing is mounted")?;
        let kids = self.scopes[self.root_scope].kids.clone();
        for k in kids {
            self.dispose_scope(k);
        }
        // app-wide pref signals live in the root scope; drop them too
        let sigs = std::mem::take(&mut self.scopes[self.root_scope].sigs);
        for s in sigs {
            self.signals.remove(s);
        }
        let frag = self.nodes[self.root].frag;
        self.frags[frag].items.clear();
        self.nodes[self.root].children.clear();
        self.handlers.clear();
        self.appear.clear();
        self.tweens.clear();
        self.callbacks.clear();
        self.pending.clear();
        self.deferred.clear();
        self.prefs.clear();
        self.dirty_prefs.clear();
        self.consts.clear();
        self.scheme_override = None;
        self.apply_scheme();
        self.mount(&view)
    }

    /// Instantiate `view` as the app root.
    pub fn mount(&mut self, view: &str) -> Result<(), String> {
        let p = self.prog.clone();
        let Some(&vi) = p.view_index.get(view) else {
            let names: Vec<&str> = p.views.iter().skip_while(|v| v.span.file == 0).map(|v| v.name.as_str()).collect();
            return Err(format!("no view named `{}`; views in this project: {}", view, names.join(" ")));
        };
        let required: Vec<&str> =
            p.views[vi].params.iter().filter(|pa| pa.default.is_none() && !is_slot(pa)).map(|pa| pa.name.as_str()).collect();
        if !required.is_empty() {
            return Err(format!("view `{}` cannot be the root: it has required parameters ({})", view, required.join(", ")));
        }
        let (frag, root, sc) = (self.nodes[self.root].frag, self.root, self.root_scope);
        self.root_view = Some(view.to_string());
        self.instantiate_view(&p, vi, &[], &[], sc, frag, root);
        self.reflatten(root);
        self.settle();
        Ok(())
    }

    /// Run pending effects, deferred native results and `on appear` handlers, persist
    /// changed `pref`s, then layout. Call after any event.
    pub fn settle(&mut self) {
        let p = self.prog.clone();
        for _ in 0..100 {
            self.flush(&p);
            if !self.run_deferred() {
                break;
            }
        }
        if !self.dirty_prefs.is_empty() {
            self.persist_prefs();
        }
        self.layout();
    }

    /// The user's language (`en`, `ru`, `pt-BR`); the host sets it at start and on change.
    pub fn set_locale(&mut self, tag: &str) {
        self.sig_set(self.locale_sig, Value::Str(Rc::from(tag)));
        self.layout_dirty = true;
        self.settle();
    }

    pub fn locale(&self) -> String {
        match &self.signals[self.locale_sig].value {
            Value::Str(s) => s.to_string(),
            _ => "en".into(),
        }
    }

    pub fn is_dark(&self) -> bool {
        self.signals[self.theme_sig].value.truthy()
    }
    pub fn set_dark(&mut self, dark: bool) {
        self.sig_set(self.theme_sig, Value::Bool(dark));
        self.layout_dirty = true;
        self.settle();
    }
    /// Current value of a color token (untracked).
    pub fn token(&self, name: &str) -> u32 {
        let (l, d) = self.tokens.get(name).copied().unwrap_or((0x000000FF, 0xFFFFFFFF));
        if self.is_dark() {
            d
        } else {
            l
        }
    }
    pub(crate) fn token_tracked(&mut self, name: &str) -> Value {
        let dark = self.sig_get(self.theme_sig).truthy();
        let (l, d) = self.tokens.get(name).copied().unwrap_or((0x000000FF, 0xFFFFFFFF));
        Value::Color(if dark { d } else { l })
    }

    pub fn error(&mut self, code: &str, span: Span, msg: impl AsRef<str>) {
        let line = format!("{} {} ({})", code, msg.as_ref(), self.sources.loc(span));
        if self.errors.last() != Some(&line) && self.errors.len() < 50 {
            self.errors.push(line);
        }
    }

    // ---------------- reactive graph ----------------
    pub(crate) fn new_signal(&mut self, owner: ScopeId, value: Value, name: Option<&str>) -> SigId {
        let id = self.signals.insert(Signal { value, subs: Vec::new(), name: name.map(Rc::from), persist: None });
        self.scopes[owner].sigs.push(id);
        id
    }

    fn track(&mut self, src: Src) {
        let Some(Some(obs)) = self.observers.last().copied() else { return };
        let subs = match src {
            Src::Sig(s) => self.signals.get_mut(s).map(|s| &mut s.subs),
            Src::Comp(c) => self.comps.get_mut(c).map(|c| &mut c.subs),
        };
        let Some(subs) = subs else { return };
        if !subs.contains(&obs) {
            subs.push(obs);
        }
        let deps = match obs {
            Obs::Eff(e) => self.effects.get_mut(e).map(|e| &mut e.deps),
            Obs::Comp(c) => self.comps.get_mut(c).map(|c| &mut c.deps),
        };
        if let Some(deps) = deps {
            if !deps.contains(&src) {
                deps.push(src);
            }
        }
    }

    pub(crate) fn sig_get(&mut self, id: SigId) -> Value {
        self.track(Src::Sig(id));
        self.signals.get(id).map(|s| s.value.clone()).unwrap_or(Value::Nil)
    }
    pub(crate) fn sig_peek(&self, id: SigId) -> Value {
        self.signals.get(id).map(|s| s.value.clone()).unwrap_or(Value::Nil)
    }
    pub(crate) fn sig_set(&mut self, id: SigId, v: Value) {
        let Some(s) = self.signals.get_mut(id) else { return };
        if s.value == v {
            return;
        }
        s.value = v;
        self.notify(id);
    }
    pub(crate) fn notify(&mut self, id: SigId) {
        let Some(s) = self.signals.get(id) else { return };
        if s.persist.is_some() && !self.dirty_prefs.contains(&id) {
            self.dirty_prefs.push(id);
        }
        let subs = s.subs.clone();
        for o in subs {
            self.mark(o);
        }
    }
    fn mark(&mut self, obs: Obs) {
        match obs {
            Obs::Eff(e) => {
                if let Some(eff) = self.effects.get(e) {
                    self.queue.insert(eff.seq, e);
                }
            }
            Obs::Comp(c) => {
                let Some(comp) = self.comps.get_mut(c) else { return };
                if comp.value.take().is_some() {
                    let subs = comp.subs.clone();
                    for s in subs {
                        self.mark(s);
                    }
                }
            }
        }
    }
    fn unsubscribe(&mut self, obs: Obs, deps: &[Src]) {
        for d in deps {
            let subs = match d {
                Src::Sig(s) => self.signals.get_mut(*s).map(|s| &mut s.subs),
                Src::Comp(c) => self.comps.get_mut(*c).map(|c| &mut c.subs),
            };
            if let Some(subs) = subs {
                subs.retain(|o| *o != obs);
            }
        }
    }

    pub(crate) fn new_comp(&mut self, owner: ScopeId, expr: ExprId, scope: ScopeId) -> CompId {
        let id = self.comps.insert(Computed { expr, scope, value: None, deps: Vec::new(), subs: Vec::new(), computing: false });
        self.scopes[owner].comps.push(id);
        id
    }

    pub(crate) fn comp_get(&mut self, p: &Program, id: CompId) -> Value {
        self.track(Src::Comp(id));
        let Some(c) = self.comps.get_mut(id) else { return Value::Nil };
        if let Some(v) = &c.value {
            return v.clone();
        }
        if c.computing {
            let span = p.span(c.expr);
            self.error("R0004", span, "a derived value depends on itself (cycle)");
            return Value::Nil;
        }
        c.computing = true;
        let (expr, scope) = (c.expr, c.scope);
        let deps = std::mem::take(&mut c.deps);
        self.unsubscribe(Obs::Comp(id), &deps);
        self.observers.push(Some(Obs::Comp(id)));
        let v = self.eval(p, expr, scope);
        self.observers.pop();
        if let Some(c) = self.comps.get_mut(id) {
            c.computing = false;
            c.value = Some(v.clone());
        }
        v
    }

    pub(crate) fn untracked<T>(&mut self, f: impl FnOnce(&mut Runtime) -> T) -> T {
        self.observers.push(None);
        let r = f(self);
        self.observers.pop();
        r
    }

    fn create_effect(&mut self, p: &Program, kind: EffKind, owner: ScopeId) -> EffId {
        self.seq += 1;
        let id = self.effects.insert(Effect { kind, deps: Vec::new(), seq: self.seq });
        self.scopes[owner].effs.push(id);
        self.run_effect(p, id);
        id
    }

    fn run_effect(&mut self, p: &Program, id: EffId) {
        let Some(eff) = self.effects.get_mut(id) else { return };
        let deps = std::mem::take(&mut eff.deps);
        let mut kind = std::mem::replace(&mut eff.kind, EffKind::Running);
        self.unsubscribe(Obs::Eff(id), &deps);
        self.stats.effects_run += 1;
        self.observers.push(Some(Obs::Eff(id)));
        match &mut kind {
            EffKind::Prop { node, name, expr, scope, last } => {
                let v = self.eval(p, *expr, *scope);
                // equality cut-off: an unchanged value touches nothing
                if last.as_ref() != Some(&v) {
                    if let Some(n) = self.nodes.get_mut(*node) {
                        let old = if n.shown && n.animate != 0 { crate::anim::get(n, name) } else { None };
                        n.apply(name, &v);
                        self.layout_dirty |= crate::node::affects_layout(name);
                        if let Some(old) = old {
                            self.start_tween(*node, name, old, v.clone());
                        }
                    }
                    *last = Some(v);
                }
            }
            EffKind::If { ui, scope, frag, parent, current } => self.run_if(p, *ui, *scope, *frag, *parent, current),
            EffKind::For { ui, list, scope, frag, parent, items } => self.run_for(p, *ui, *list, *scope, *frag, *parent, items),
            EffKind::Running => {}
        }
        self.observers.pop();
        if let Some(eff) = self.effects.get_mut(id) {
            eff.kind = kind;
        }
    }

    pub(crate) fn flush(&mut self, p: &Program) {
        let mut budget = 100_000u32;
        while let Some((&seq, &id)) = self.queue.iter().next() {
            self.queue.remove(&seq);
            self.run_effect(p, id);
            budget -= 1;
            if budget == 0 {
                self.queue.clear();
                self.error("R0006", Span::default(), "reactive loop: effects kept re-triggering each other");
                break;
            }
        }
    }

    // ---------------- scopes ----------------
    pub(crate) fn new_scope(&mut self, lex: Option<ScopeId>, owner: Option<ScopeId>, view: Option<Rc<str>>) -> ScopeId {
        let id = self.scopes.insert(Scope { lex, owner, view, ..Default::default() });
        if let Some(o) = owner {
            if let Some(os) = self.scopes.get_mut(o) {
                os.kids.push(id);
            }
        }
        id
    }

    pub(crate) fn dispose_scope(&mut self, id: ScopeId) {
        if let Some(s) = self.scopes.get_mut(id) {
            if s.pins > 0 {
                s.zombie = true;
                return;
            }
        }
        let Some(sc) = self.scopes.remove(id) else { return };
        if let Some(o) = sc.owner {
            if let Some(os) = self.scopes.get_mut(o) {
                os.kids.retain(|k| *k != id);
            }
        }
        self.dispose_contents(sc);
    }

    fn dispose_contents(&mut self, sc: Scope) {
        for k in sc.kids {
            if let Some(child) = self.scopes.remove(k) {
                self.dispose_contents(child);
            }
        }
        for e in sc.effs {
            if let Some(eff) = self.effects.remove(e) {
                self.unsubscribe(Obs::Eff(e), &eff.deps);
            }
        }
        for c in sc.comps {
            if let Some(comp) = self.comps.remove(c) {
                self.unsubscribe(Obs::Comp(c), &comp.deps);
            }
        }
        for s in sc.sigs {
            self.signals.remove(s);
        }
        for n in sc.nodes {
            self.nodes.remove(n);
        }
        for f in sc.frags {
            self.frags.remove(f);
        }
    }

    pub(crate) fn lookup(&self, mut sc: ScopeId, name: &str) -> Option<(ScopeId, Binding)> {
        loop {
            let scope = self.scopes.get(sc)?;
            if let Some((_, b)) = scope.vars.iter().rev().find(|(n, _)| &**n == name) {
                return Some((sc, b.clone()));
            }
            sc = scope.lex?;
        }
    }

    pub(crate) fn declare(&mut self, sc: ScopeId, name: &str, b: Binding) {
        if let Some(s) = self.scopes.get_mut(sc) {
            s.vars.push((Rc::from(name), b));
        }
    }

    // ---------------- tree construction ----------------
    fn new_node(&mut self, kind: Kind, src: Span, owner: ScopeId) -> NodeKey {
        let frag = self.frags.insert(Frag::default());
        let uid = self.next_uid;
        self.next_uid += 1;
        self.stats.nodes_created += 1;
        let key = self.nodes.insert(Node::new(uid, kind, frag, src));
        let s = &mut self.scopes[owner];
        s.nodes.push(key);
        s.frags.push(frag);
        key
    }

    fn new_frag(&mut self, owner: ScopeId) -> FragId {
        let f = self.frags.insert(Frag::default());
        self.scopes[owner].frags.push(f);
        f
    }

    /// Recompute `parent.children` from its fragment tree (after structural changes).
    pub(crate) fn reflatten(&mut self, parent: NodeKey) {
        let Some(node) = self.nodes.get(parent) else { return };
        let mut out = Vec::new();
        self.collect(node.frag, &mut out);
        for k in &out {
            self.nodes[*k].parent = Some(parent);
        }
        self.nodes[parent].children = out;
        self.layout_dirty = true;
    }

    fn collect(&self, f: FragId, out: &mut Vec<NodeKey>) {
        let Some(fr) = self.frags.get(f) else { return };
        for it in &fr.items {
            match it {
                FItem::Node(k) => {
                    if self.nodes.contains_key(*k) {
                        out.push(*k)
                    }
                }
                FItem::Frag(g) => self.collect(*g, out),
            }
        }
    }

    pub(crate) fn build(&mut self, p: &Program, ids: &[UiId], sc: ScopeId, frag: FragId, parent: NodeKey) {
        for &id in ids {
            match p.node(id) {
                UiNode::Element { name, args, children, span, .. } => {
                    if let Some(kind) = Kind::from_name(name) {
                        let node = self.new_node(kind, *span, sc);
                        self.frags[frag].items.push(FItem::Node(node));
                        let def = registry::element(name).unwrap();
                        for (i, a) in args.iter().enumerate() {
                            let pname = match &a.name {
                                Some(n) => n.as_str(),
                                None if i == 0 => def.positional.unwrap_or(""),
                                None => "",
                            };
                            if let Some(pd) = registry::prop(def, pname) {
                                self.bind_prop(p, node, pd.name, a.value, sc);
                            }
                        }
                        let nf = self.nodes[node].frag;
                        self.build(p, children, sc, nf, node);
                        self.reflatten(node);
                    } else if let Some(&vi) = p.view_index.get(name) {
                        let start = self.frags[frag].items.len();
                        self.instantiate_view(p, vi, args, children, sc, frag, parent);
                        if span.file != 0 {
                            for it in &self.frags[frag].items[start..] {
                                if let FItem::Node(k) = it {
                                    self.nodes[*k].use_site = Some(*span);
                                }
                            }
                        }
                    }
                }
                UiNode::If { .. } => {
                    let f = self.new_frag(sc);
                    self.frags[frag].items.push(FItem::Frag(f));
                    self.create_effect(p, EffKind::If { ui: id, scope: sc, frag: f, parent, current: None }, sc);
                }
                UiNode::For { iter, .. } => {
                    let f = self.new_frag(sc);
                    self.frags[frag].items.push(FItem::Frag(f));
                    let list = self.new_comp(sc, *iter, sc);
                    self.create_effect(p, EffKind::For { ui: id, list, scope: sc, frag: f, parent, items: Vec::new() }, sc);
                }
                UiNode::Slot { name, .. } => {
                    if let Some((_, Binding::Slot { nodes, scope })) = self.lookup(sc, name) {
                        let ssc = self.new_scope(Some(scope), Some(sc), None);
                        self.build(p, &nodes, ssc, frag, parent);
                    }
                }
            }
        }
    }

    fn bind_prop(&mut self, p: &Program, node: NodeKey, name: &'static str, expr: ExprId, sc: ScopeId) {
        if name == "value" {
            if let Some(n) = self.nodes.get_mut(node) {
                n.bind = Some((expr, sc));
            }
        }
        // Literals, enum variants and handler closures read no reactive source: apply once, no effect.
        if is_static(p, expr) {
            let v = self.eval(p, expr, sc);
            if let Some(n) = self.nodes.get_mut(node) {
                n.apply(name, &v);
            }
            return;
        }
        let id = self.create_effect(p, EffKind::Prop { node, name, expr, scope: sc, last: None }, sc);
        // A prop that read no reactive source is static: drop its effect.
        if self.effects.get(id).is_some_and(|e| e.deps.is_empty()) {
            self.effects.remove(id);
            if let Some(s) = self.scopes.get_mut(sc) {
                if s.effs.last() == Some(&id) {
                    s.effs.pop();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn instantiate_view(&mut self, p: &Program, vi: usize, args: &[Arg], block: &[UiId], sc: ScopeId, frag: FragId, parent: NodeKey) {
        let view = &p.views[vi];
        let vname: Rc<str> = Rc::from(view.name.as_str());
        let vsc = self.new_scope(None, Some(sc), Some(vname.clone()));
        let first_param = view.params.iter().position(|pa| !is_slot(pa));
        let mut outer: Vec<(&'static str, ExprId)> = Vec::new();
        let mut given: Vec<Option<ExprId>> = vec![None; view.params.len()];
        for (i, a) in args.iter().enumerate() {
            match &a.name {
                None => {
                    if let (0, Some(fp)) = (i, first_param) {
                        given[fp] = Some(a.value);
                    }
                }
                Some(n) => {
                    if let Some(pi) = view.params.iter().position(|pa| pa.name == *n) {
                        given[pi] = Some(a.value);
                    } else if let Some(op) = VIEW_OUTER_PROPS.iter().find(|o| **o == n.as_str()) {
                        outer.push((op, a.value));
                    }
                }
            }
        }
        for (pi, param) in view.params.iter().enumerate() {
            let b = if is_slot(param) {
                Binding::Slot { nodes: Rc::from(block), scope: sc }
            } else {
                match (given[pi], param.default) {
                    (Some(e), _) if param.bind => Binding::Bind { expr: e, scope: sc },
                    (Some(e), _) => Binding::Comp(self.new_comp(vsc, e, sc)),
                    (None, Some(d)) => Binding::Comp(self.new_comp(vsc, d, vsc)),
                    (None, None) => Binding::Val(Value::Nil),
                }
            };
            self.declare(vsc, &param.name, b);
        }
        for (mi, m) in view.members.iter().enumerate() {
            if let ViewMember::Fn(f) = m {
                let func = Value::Func(Rc::new(Func::ViewFn { view: vi, member: mi, scope: vsc }));
                self.declare(vsc, &f.name, Binding::Val(func));
            }
        }
        for (mi, m) in view.members.iter().enumerate() {
            match m {
                ViewMember::State { name, init, .. } => {
                    let v = self.untracked(|rt| rt.eval(p, *init, vsc));
                    let s = self.new_signal(vsc, v, Some(name));
                    self.declare(vsc, name, Binding::Sig(s));
                }
                ViewMember::Pref { name, ty, init, .. } => {
                    let s = match self.prefs.get(name.as_str()) {
                        Some(&s) if self.signals.contains_key(s) => s,
                        _ => {
                            let init_v = self.untracked(|rt| rt.eval(p, *init, vsc));
                            let v = match self.pref_load(name) {
                                Some(j) => match ty {
                                    Some(t) => self.decode_json(p, &j, t),
                                    None => self.decode_like(&j, &init_v),
                                },
                                None => init_v,
                            };
                            let root = self.root_scope;
                            let s = self.new_signal(root, v, Some(name));
                            self.signals[s].persist = Some(Rc::from(name.as_str()));
                            self.prefs.insert(name.clone(), s);
                            s
                        }
                    };
                    self.declare(vsc, name, Binding::Sig(s));
                }
                ViewMember::On { event, .. } => {
                    let h = crate::native::Handler { event: Rc::from(event.as_str()), view: vi, member: mi, scope: vsc };
                    if event == "appear" {
                        self.appear.push_back(h);
                    } else {
                        self.handlers.push(h);
                    }
                }
                ViewMember::Let { name, value, .. } => {
                    let c = self.new_comp(vsc, *value, vsc);
                    self.declare(vsc, name, Binding::Comp(c));
                }
                _ => {}
            }
        }
        let start = self.frags[frag].items.len();
        let nodes: Vec<UiId> = view.members.iter().filter_map(|m| if let ViewMember::Node(n) = m { Some(*n) } else { None }).collect();
        self.untracked(|rt| rt.build(p, &nodes, vsc, frag, parent));
        let roots: Vec<NodeKey> =
            self.frags[frag].items[start..].iter().filter_map(|it| if let FItem::Node(k) = it { Some(*k) } else { None }).collect();
        for k in roots {
            self.nodes[k].components.insert(0, vname.clone());
            for (op, e) in &outer {
                self.bind_prop(p, k, op, *e, sc);
            }
        }
    }

    fn run_if(&mut self, p: &Program, ui: UiId, scope: ScopeId, frag: FragId, parent: NodeKey, current: &mut Option<(bool, ScopeId)>) {
        let UiNode::If { cond, then, els, .. } = p.node(ui) else { return };
        let c = self.eval(p, *cond, scope).truthy();
        if current.map(|(b, _)| b) == Some(c) {
            return;
        }
        if let Some((_, old)) = current.take() {
            self.dispose_scope(old);
        }
        if let Some(f) = self.frags.get_mut(frag) {
            f.items.clear();
        }
        let branch = self.new_scope(Some(scope), Some(scope), None);
        let nodes = if c { then } else { els };
        self.untracked(|rt| rt.build(p, nodes, branch, frag, parent));
        *current = Some((c, branch));
        self.reflatten(parent);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_for(&mut self, p: &Program, ui: UiId, list: CompId, scope: ScopeId, frag: FragId, parent: NodeKey, items: &mut Vec<ItemRec>) {
        let UiNode::For { var, index_var, iter, key, body, span } = p.node(ui) else { return };
        let values = match self.comp_get(p, list) {
            Value::List(l) => l,
            _ => Rc::new(Vec::new()),
        };
        // 1. keys of the new list
        let mut keys: Vec<String> = Vec::with_capacity(values.len());
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (i, item) in values.iter().enumerate() {
            let mut k = match key {
                Some(k) if fast_key(p, *k, var).is_some() => {
                    // `by item.field`: read the field directly, no scope allocation
                    let field = fast_key(p, *k, var).unwrap();
                    match item {
                        Value::Struct(s) => s.fields.iter().find(|(n, _)| &**n == field).map(|(_, v)| v.repr()).unwrap_or_default(),
                        _ => String::new(),
                    }
                }
                Some(k) => {
                    let tmp = self.new_scope(Some(scope), None, None);
                    self.declare(tmp, var, Binding::Val(item.clone()));
                    if let Some(iv) = index_var {
                        self.declare(tmp, iv, Binding::Val(Value::Int(i as i64)));
                    }
                    let kv = self.untracked(|rt| rt.eval(p, *k, tmp));
                    self.dispose_scope(tmp);
                    kv.repr()
                }
                None => i.to_string(),
            };
            if let Some(first) = seen.get(&k) {
                self.error("R0002", *span, format!("duplicate `by` key {} (items {} and {}); keys must be unique", k, first, i));
                k = format!("{}#dup{}", k, i);
            }
            seen.insert(k.clone(), i);
            keys.push(k);
        }
        // 2. reconcile: reuse scopes by key, create missing, dispose leftovers
        let mut old: HashMap<String, ItemRec> = std::mem::take(items).into_iter().map(|r| (r.key.clone(), r)).collect();
        let mut next: Vec<ItemRec> = Vec::with_capacity(keys.len());
        for (i, k) in keys.into_iter().enumerate() {
            if let Some(rec) = old.remove(&k) {
                self.sig_set(rec.index, Value::Int(i as i64));
                self.sig_set(rec.value, values[i].clone());
                next.push(rec);
                continue;
            }
            let isc = self.new_scope(Some(scope), Some(scope), None);
            let index = self.new_signal(isc, Value::Int(i as i64), None);
            let value = self.new_signal(isc, values[i].clone(), None);
            self.declare(isc, var, Binding::Item { value, src: *iter, scope, index });
            if let Some(iv) = index_var {
                self.declare(isc, iv, Binding::Sig(index));
            }
            let ifrag = self.new_frag(isc);
            self.untracked(|rt| rt.build(p, body, isc, ifrag, parent));
            next.push(ItemRec { key: k, scope: isc, frag: ifrag, index, value });
        }
        for (_, rec) in old {
            self.dispose_scope(rec.scope);
        }
        if let Some(f) = self.frags.get_mut(frag) {
            f.items = next.iter().map(|r| FItem::Frag(r.frag)).collect();
        }
        *items = next;
        self.reflatten(parent);
    }
}

/// `by var.field` -> Some("field")
fn fast_key<'p>(p: &'p Program, k: ExprId, var: &str) -> Option<&'p str> {
    match p.expr(k) {
        Expr::Member(b, f, _) if matches!(p.expr(*b), Expr::Ident(v) if v == var) => Some(f.as_str()),
        _ => None,
    }
}

fn is_static(p: &Program, e: ExprId) -> bool {
    match p.expr(e) {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Nil | Expr::Color(_) | Expr::Lambda { .. } => true,
        Expr::Str(parts) => parts.iter().all(|s| matches!(s, StrPart::Lit(_))),
        Expr::Dot(_) => p.dot_kinds.get(&e) == Some(&DotKind::Enum),
        Expr::Unary(UnOp::Neg, x) => is_static(p, *x),
        _ => false,
    }
}

pub(crate) fn is_slot(p: &Param) -> bool {
    matches!(&p.ty, TypeRef::Named(n, _) if n == "slot")
}
