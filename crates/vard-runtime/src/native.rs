//! The native boundary: `extern fn` calls, callbacks, platform events, `pref` storage,
//! predictive back and system insets.
//!
//! On a device a [`NativeHost`] (the Kotlin/Swift shell) answers calls with JSON. Headless
//! (`vard run`, tests, MCP) the built-in [`Sim`] plays the platform: preferences and secure
//! storage are real in-memory stores, the permission dialog answers what the test says,
//! notifications are recorded, push messages and back gestures are injected by actions.
//! User-declared externs answer from `mock` values or stay *pending* until `resolve`.

use crate::json::{data_to_json, to_json, Json};
use crate::rt::{NodeKey, Runtime, ScopeId};
use crate::value::{Func, StructVal, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::Rc;
use vard_lang::ast::{Program, TypeRef, ViewMember};
use vard_lang::diag::Span;

/// Implemented by a platform shell. Every call happens on the UI thread.
pub trait NativeHost {
    /// Call native function `name` with a JSON array of arguments. Function arguments arrive
    /// as `{"$callback": id}`; answer them later with [`Runtime::invoke_callback`].
    /// Return the JSON result (`"null"` for none), or `None` if this host does not implement
    /// `name` (the runtime then reports R0010 and uses a zero value).
    fn call(&mut self, name: &str, args_json: &str) -> Option<String>;
}

/// Headless stand-in for the platform.
#[derive(Default, Clone)]
pub struct Sim {
    pub prefs: BTreeMap<String, String>,
    pub secure: BTreeMap<String, String>,
    /// permission -> current status
    pub permissions: BTreeMap<String, String>,
    /// permission -> what the system dialog will answer (default: granted)
    pub answers: BTreeMap<String, String>,
    pub notifications: Vec<String>,
    pub push_token: String,
}

pub(crate) struct Callback {
    pub func: Value,
    pub params: Vec<TypeRef>,
}

/// A headless call to a user extern that takes callbacks and has no mock: it stays pending
/// until `resolve NAME VALUE`.
pub(crate) struct Pending {
    pub name: Rc<str>,
    pub callbacks: Vec<u64>,
    pub shown: String,
}

pub(crate) struct Handler {
    pub event: Rc<str>,
    pub view: usize,
    pub member: usize,
    pub scope: ScopeId,
}

/// Insets edges bit mask used by `safeArea`.
pub const SAFE_TOP: u8 = 1;
pub const SAFE_BOTTOM: u8 = 2;
pub const SAFE_LEFT: u8 = 4;
pub const SAFE_RIGHT: u8 = 8;
pub const SAFE_ALL: u8 = 15;

pub fn safe_mask(variant: &str) -> u8 {
    match variant {
        "all" => SAFE_ALL,
        "top" => SAFE_TOP,
        "bottom" => SAFE_BOTTOM,
        "horizontal" => SAFE_LEFT | SAFE_RIGHT,
        "vertical" => SAFE_TOP | SAFE_BOTTOM,
        _ => 0,
    }
}

const PREF_PREFIX: &str = "vard.pref.";

impl Runtime {
    // ---------------- host ----------------
    pub fn set_host(&mut self, host: Box<dyn NativeHost>, platform: &str) {
        self.host = Some(host);
        self.platform = Rc::from(platform);
    }

    fn register_callback(&mut self, func: &Value, params: Vec<TypeRef>) -> u64 {
        self.next_callback += 1;
        let id = self.next_callback;
        self.pin(func, true);
        self.callbacks.insert(id, Callback { func: func.clone(), params });
        id
    }

    fn drop_callback(&mut self, id: u64) -> Option<Callback> {
        let cb = self.callbacks.remove(&id)?;
        self.pin(&cb.func, false);
        Some(cb)
    }

    pub(crate) fn defer(&mut self, f: Value, args: Vec<Option<Value>>) {
        self.pin(&f, true);
        self.deferred.push_back((f, args));
    }

    /// Keep (or release) the temporary call scopes a closure captured: a callback created
    /// inside a handler outlives the handler's own execution.
    pub(crate) fn pin(&mut self, f: &Value, on: bool) {
        let Value::Func(func) = f else { return };
        let mut cur = match &**func {
            Func::Lambda { scope, .. } | Func::ViewFn { scope, .. } => Some(*scope),
            _ => None,
        };
        while let Some(id) = cur {
            let Some(s) = self.scopes.get_mut(id) else { break };
            cur = s.lex;
            if s.owner.is_some() {
                continue; // owned by the view tree: lives as long as its view
            }
            if on {
                s.pins += 1;
            } else {
                s.pins = s.pins.saturating_sub(1);
                if s.pins == 0 && s.zombie {
                    s.zombie = false;
                    self.dispose_scope(id);
                }
            }
        }
    }

    /// Native call boundary for `extern fn` number `i`.
    pub(crate) fn call_extern(&mut self, p: &Program, i: usize, mut argv: Vec<Option<Value>>) -> Value {
        let d = &p.externs[i];
        let name: Rc<str> = Rc::from(d.name.as_str());
        // defaults for omitted parameters
        argv.resize(d.params.len().max(argv.len()), None);
        let root = self.root_scope;
        let args: Vec<Value> = d
            .params
            .iter()
            .zip(argv)
            .map(|(pa, a)| match (a, pa.default) {
                (Some(v), _) => v,
                (None, Some(def)) => self.untracked(|rt| rt.eval(p, def, root)),
                (None, None) => Value::Nil,
            })
            .collect();
        let shown: Vec<String> = args.iter().map(|v| v.repr()).collect();
        let shown = format!("{}({})", d.name, shown.join(", "));
        // intrinsics: the runtime itself implements these on every platform
        if &*name == "setColorScheme" {
            self.scheme_override = match args.first() {
                Some(Value::Enum(e)) if &**e == "dark" => Some(true),
                Some(Value::Enum(e)) if &**e == "light" => Some(false),
                _ => None,
            };
            self.native_calls.push(shown);
            self.apply_scheme();
            return Value::Unit;
        }
        self.native_calls.push(shown.clone());
        let ret_ty = d.ret.clone();
        let cb_params: Vec<(usize, Vec<TypeRef>)> = d
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, pa)| match &pa.ty {
                TypeRef::Fn(ps, _) => Some((i, ps.clone())),
                _ => None,
            })
            .collect();

        // 1. mocks (tests): the return value, and every callback receives the mock
        if let Some(mock) = self.mocks.get(&*name).cloned() {
            for (ai, ps) in &cb_params {
                if let Some(f @ Value::Func(_)) = args.get(*ai) {
                    let vals = self.decode_cb_args(p, &mock, ps);
                    self.defer(f.clone(), vals);
                }
            }
            return match &ret_ty {
                Some(t) => self.decode_json(p, &mock, t),
                None => Value::Unit,
            };
        }
        // 2. a real platform
        if self.host.is_some() {
            // encode with placeholder ids, then register (the encoder cannot borrow `self`)
            let mut funcs: Vec<(usize, Value)> = Vec::new();
            let encoded: Vec<Json> = args.iter().enumerate().map(|(ai, a)| to_json(a, &mut |f| {
                funcs.push((ai, f.clone()));
                0
            })).collect();
            let ids: Vec<u64> = funcs
                .into_iter()
                .map(|(ai, f)| {
                    let ps = cb_params.iter().find(|(i, _)| *i == ai).map(|(_, ps)| ps.clone()).unwrap_or_default();
                    self.register_callback(&f, ps)
                })
                .collect();
            let mut encoded = Json::Array(encoded);
            fix_callback_ids(&mut encoded, &mut ids.into_iter());
            let args_json = encoded.to_string();
            let answer = self.host.as_mut().and_then(|h| h.call(&name, &args_json));
            return match answer {
                Some(r) => {
                    let j: Json = serde_json::from_str(&r).unwrap_or(Json::Null);
                    match &ret_ty {
                        Some(t) => self.decode_json(p, &j, t),
                        None => Value::Unit,
                    }
                }
                None => {
                    self.error("R0010", d.span, format!("native function `{}` is not implemented by the {} host", name, self.platform));
                    zero(ret_ty.as_ref())
                }
            };
        }
        // 3. headless platform simulator for std functions
        if d.span.file == 0 {
            if let Some(v) = self.sim_call(p, &name, &args, &cb_params) {
                return v;
            }
        }
        // 4. user extern without a mock: callbacks stay pending until `resolve`
        if !cb_params.is_empty() {
            let mut ids = Vec::new();
            for (ai, ps) in &cb_params {
                if let Some(f @ Value::Func(_)) = args.get(*ai) {
                    ids.push(self.register_callback(f, ps.clone()));
                }
            }
            self.pending.push(Pending { name: name.clone(), callbacks: ids, shown });
        }
        zero(ret_ty.as_ref())
    }

    fn decode_cb_args(&mut self, p: &Program, j: &Json, params: &[TypeRef]) -> Vec<Option<Value>> {
        match params.len() {
            0 => Vec::new(),
            1 => vec![Some(self.decode_json(p, j, &params[0]))],
            _ => {
                // several parameters: the value is a JSON array of arguments
                let items: Vec<Json> = match j {
                    Json::Array(a) => a.clone(),
                    other => vec![other.clone()],
                };
                params.iter().enumerate().map(|(i, t)| Some(self.decode_json(p, items.get(i).unwrap_or(&Json::Null), t))).collect()
            }
        }
    }

    /// Deliver a native result to callback `id` (from [`NativeHost::call`]). `keep` leaves the
    /// callback registered for further deliveries (listeners). Returns false if the id is
    /// unknown or its view is gone.
    pub fn invoke_callback(&mut self, id: u64, json: &str, keep: bool) -> bool {
        let Some((func, params)) = self.callbacks.get(&id).map(|c| (c.func.clone(), c.params.clone())) else {
            return false;
        };
        if !self.func_alive(&func) {
            self.drop_callback(id);
            return false;
        }
        let p = self.prog.clone();
        let j: Json = serde_json::from_str(json).unwrap_or(Json::Null);
        let args = self.decode_cb_args(&p, &j, &params);
        self.call_func(&p, &func, args, Span::default());
        if !keep {
            self.drop_callback(id);
        }
        self.settle();
        true
    }

    /// Forget a kept callback (the host's listener was removed).
    pub fn release_callback(&mut self, id: u64) {
        self.drop_callback(id);
    }

    /// The closure's scope chain still exists (its view was not removed meanwhile).
    pub(crate) fn func_alive(&self, f: &Value) -> bool {
        let Value::Func(func) = f else { return false };
        let mut cur = match &**func {
            Func::Lambda { scope, .. } | Func::ViewFn { scope, .. } => Some(*scope),
            _ => return true,
        };
        while let Some(id) = cur {
            match self.scopes.get(id) {
                Some(s) => cur = s.lex,
                None => return false,
            }
        }
        true
    }

    // ---------------- events ----------------
    /// Deliver platform event `name`; `args_json` is a JSON array with one item per event
    /// parameter (a single non-array value is accepted for one-parameter events).
    /// Returns the number of handlers that ran.
    pub fn emit(&mut self, name: &str, args_json: &str) -> Result<usize, String> {
        let p = self.prog.clone();
        let Some(&ei) = p.event_index.get(name) else {
            let names: Vec<&str> = p.events.iter().map(|e| e.name.as_str()).collect();
            return Err(format!("no event `{}`; events: {}", name, names.join(" ")));
        };
        let j: Json = serde_json::from_str(args_json).unwrap_or(Json::Null);
        let items: Vec<Json> = match j {
            Json::Array(a) => a,
            Json::Null => Vec::new(),
            other => vec![other],
        };
        let params: Vec<TypeRef> = p.events[ei].params.iter().map(|pa| pa.ty.clone()).collect();
        let args: Vec<Value> = params.iter().enumerate().map(|(i, t)| self.decode_json(&p, items.get(i).unwrap_or(&Json::Null), t)).collect();
        Ok(self.dispatch(name, args))
    }

    pub(crate) fn dispatch(&mut self, name: &str, args: Vec<Value>) -> usize {
        let p = self.prog.clone();
        self.handlers.retain(|h| self.scopes.contains_key(h.scope));
        let targets: Vec<(usize, usize, ScopeId)> =
            self.handlers.iter().filter(|h| &*h.event == name).map(|h| (h.view, h.member, h.scope)).collect();
        let mut ran = 0;
        for (v, m, sc) in targets {
            if !self.scopes.contains_key(sc) || !self.handler_enabled(&p, v, m, sc) {
                continue;
            }
            self.run_handler(&p, v, m, sc, &args);
            ran += 1;
        }
        self.settle();
        ran
    }

    fn handler_enabled(&mut self, p: &Program, v: usize, m: usize, sc: ScopeId) -> bool {
        match &p.views[v].members[m] {
            ViewMember::On { cond: Some(c), .. } => {
                let c = *c;
                self.untracked(|rt| rt.eval(p, c, sc)).truthy()
            }
            _ => true,
        }
    }

    /// Whether an `on back` handler would take the next back gesture. Hosts read this to
    /// decide between in-app back (with predictive progress) and the system's own animation.
    pub fn back_enabled(&mut self) -> bool {
        let p = self.prog.clone();
        let hs: Vec<(usize, usize, ScopeId)> =
            self.handlers.iter().filter(|h| &*h.event == "back" && self.scopes.contains_key(h.scope)).map(|h| (h.view, h.member, h.scope)).collect();
        hs.into_iter().any(|(v, m, sc)| self.handler_enabled(&p, v, m, sc))
    }

    /// Predictive back progress (0..1) while the gesture is in flight.
    pub fn back_progress(&mut self, progress: f32) {
        let v = Value::Float(progress.clamp(0.0, 1.0) as f64);
        self.sig_set(self.back_sig, v);
        self.settle();
    }

    /// The back gesture was released (commit). Runs the innermost enabled `on back` handler.
    /// Returns false when nothing handled it: the host should perform the system back.
    pub fn back(&mut self) -> bool {
        let p = self.prog.clone();
        self.sig_set(self.back_sig, Value::Float(0.0));
        self.handlers.retain(|h| self.scopes.contains_key(h.scope));
        let hs: Vec<(usize, usize, ScopeId)> =
            self.handlers.iter().filter(|h| &*h.event == "back").map(|h| (h.view, h.member, h.scope)).collect();
        for (v, m, sc) in hs.into_iter().rev() {
            if self.handler_enabled(&p, v, m, sc) {
                self.run_handler(&p, v, m, sc, &[]);
                self.settle();
                return true;
            }
        }
        self.settle();
        false
    }

    pub fn back_cancel(&mut self) {
        self.sig_set(self.back_sig, Value::Float(0.0));
        self.settle();
    }

    // ---------------- insets & scheme ----------------
    /// System bar + cutout insets in dp (top, bottom, left, right) and the keyboard height.
    pub fn set_insets(&mut self, top: f32, bottom: f32, left: f32, right: f32, keyboard: f32) {
        self.insets = [top.max(0.0), bottom.max(0.0), left.max(0.0), right.max(0.0)];
        self.keyboard = keyboard.max(0.0);
        let s = self.safe_insets();
        let v = Value::Struct(Rc::new(StructVal {
            name: Rc::from("Insets"),
            fields: vec![
                (Rc::from("top"), Value::Float(s[0] as f64)),
                (Rc::from("bottom"), Value::Float(s[1] as f64)),
                (Rc::from("left"), Value::Float(s[2] as f64)),
                (Rc::from("right"), Value::Float(s[3] as f64)),
            ],
        }));
        self.sig_set(self.insets_sig, v);
        self.sig_set(self.keyboard_sig, Value::Float(self.keyboard as f64));
        self.layout_dirty = true;
        self.settle();
        if self.keyboard > 0.0 {
            if let Some(f) = self.focused {
                self.reveal(f);
            }
        }
    }

    /// The Input with keyboard focus (hosts report it; `focus SEL` headless).
    pub fn set_focus(&mut self, key: Option<NodeKey>) {
        self.focused = key;
        if let Some(k) = key {
            self.reveal(k);
        }
    }

    /// Scroll ancestors so `key` is fully visible above the keyboard (16dp margin).
    pub fn reveal(&mut self, key: NodeKey) {
        let Some(target) = self.visual_rect(key) else { return };
        let bottom_limit = self.screen.1 - self.keyboard;
        let mut cur = self.nodes.get(key).and_then(|n| n.parent);
        while let Some(k) = cur {
            let Some(n) = self.nodes.get(k) else { break };
            cur = n.parent;
            if n.kind != crate::node::Kind::Scroll {
                continue;
            }
            let pad = n.pad();
            let top = n.rect.y + pad[1] - n.safe_pad[1];
            let bottom = (n.rect.y + n.rect.h).min(bottom_limit);
            let delta = if target.y + target.h + 16.0 > bottom {
                target.y + target.h + 16.0 - bottom
            } else if target.y - 16.0 < top {
                target.y - 16.0 - top
            } else {
                0.0
            };
            if delta != 0.0 {
                let _ = self.scroll_by(k, delta);
            }
            break;
        }
    }

    /// Effective safe-area insets (top, bottom, left, right): bottom includes the keyboard.
    pub fn safe_insets(&self) -> [f32; 4] {
        [self.insets[0], self.insets[1].max(self.keyboard), self.insets[2], self.insets[3]]
    }

    /// The platform's light/dark setting; an app's `setColorScheme(.light/.dark)` overrides it.
    pub fn set_system_dark(&mut self, dark: bool) {
        self.system_dark = dark;
        self.apply_scheme();
    }

    pub(crate) fn apply_scheme(&mut self) {
        let dark = self.scheme_override.unwrap_or(self.system_dark);
        if self.is_dark() != dark {
            self.set_dark(dark);
        }
    }

    // ---------------- pref ----------------
    pub(crate) fn pref_load(&mut self, name: &str) -> Option<Json> {
        let key = format!("{}{}", PREF_PREFIX, name);
        let raw = match self.host.as_mut() {
            Some(h) => {
                let args = Json::Array(vec![Json::String(key)]).to_string();
                h.call("prefGet", &args).and_then(|r| serde_json::from_str::<Json>(&r).ok()).and_then(|j| j.as_str().map(str::to_string))
            }
            None => self.sim.prefs.get(&key).cloned(),
        }?;
        serde_json::from_str(&raw).ok()
    }

    pub(crate) fn persist_prefs(&mut self) {
        let dirty = std::mem::take(&mut self.dirty_prefs);
        for sig in dirty {
            let Some(s) = self.signals.get(sig) else { continue };
            let Some(name) = s.persist.clone() else { continue };
            let raw = data_to_json(&s.value).to_string();
            let key = format!("{}{}", PREF_PREFIX, name);
            match self.host.as_mut() {
                Some(h) => {
                    let args = Json::Array(vec![Json::String(key), Json::String(raw)]).to_string();
                    h.call("prefSet", &args);
                }
                None => {
                    self.sim.prefs.insert(key, raw);
                }
            }
        }
    }

    // ---------------- headless platform ----------------
    fn sim_call(&mut self, p: &Program, name: &str, args: &[Value], cbs: &[(usize, Vec<TypeRef>)]) -> Option<Value> {
        let s = |i: usize| match args.get(i) {
            Some(Value::Enum(e)) => e.to_string(),
            Some(v) => v.display(),
            None => String::new(),
        };
        let deliver = |rt: &mut Runtime, value: Value| {
            if let Some((ai, _)) = cbs.first() {
                if let Some(f @ Value::Func(_)) = args.get(*ai) {
                    rt.defer(f.clone(), vec![Some(value)]);
                }
            }
        };
        let _ = p;
        Some(match name {
            "permissionStatus" => Value::Enum(Rc::from(self.sim.permissions.get(&s(0)).map(|s| s.as_str()).unwrap_or("notDetermined"))),
            "requestPermission" => {
                let perm = s(0);
                let cur = self.sim.permissions.get(&perm).cloned().unwrap_or_else(|| "notDetermined".into());
                let next = match cur.as_str() {
                    "granted" | "blocked" => cur.clone(),
                    _ => {
                        let answer = self.sim.answers.get(&perm).cloned().unwrap_or_else(|| "granted".into());
                        // a second refusal blocks further dialogs (Android/iOS behaviour)
                        let next = if answer == "denied" && cur == "denied" { "blocked".to_string() } else { answer };
                        self.log.push(format!("permission dialog: {} -> {}", perm, next));
                        next
                    }
                };
                self.sim.permissions.insert(perm, next.clone());
                deliver(self, Value::Enum(Rc::from(next.as_str())));
                Value::Unit
            }
            "openAppSettings" | "openUrl" | "shareText" | "haptic" => Value::Unit,
            "prefGet" => self.sim.prefs.get(&s(0)).map(|v| Value::str(v)).unwrap_or(Value::Nil),
            "prefSet" => {
                self.sim.prefs.insert(s(0), s(1));
                Value::Unit
            }
            "prefRemove" => {
                self.sim.prefs.remove(&s(0));
                Value::Unit
            }
            "secureGet" => self.sim.secure.get(&s(0)).map(|v| Value::str(v)).unwrap_or(Value::Nil),
            "secureSet" => {
                self.sim.secure.insert(s(0), s(1));
                Value::Bool(true)
            }
            "secureRemove" => {
                self.sim.secure.remove(&s(0));
                Value::Unit
            }
            "getPushToken" => {
                let t = if self.sim.push_token.is_empty() { "headless-push-token".to_string() } else { self.sim.push_token.clone() };
                deliver(self, Value::str(&t));
                Value::Unit
            }
            "showNotification" => {
                let shown = format!("{}: {}", s(0), s(1));
                self.sim.notifications.push(shown);
                Value::Unit
            }
            _ => return None,
        })
    }

    /// `resolve NAME VALUE`: complete the oldest pending call of extern NAME.
    pub fn resolve_pending(&mut self, name: &str, value: &Json) -> Result<(), String> {
        self.answer_pending(name, value, false)
    }

    /// `send NAME VALUE`: deliver to the oldest pending call of NAME and keep it pending
    /// (listeners such as `firestoreListen*` fire many times).
    pub fn send_pending(&mut self, name: &str, value: &Json) -> Result<(), String> {
        self.answer_pending(name, value, true)
    }

    fn answer_pending(&mut self, name: &str, value: &Json, keep: bool) -> Result<(), String> {
        let Some(i) = self.pending.iter().position(|pc| &*pc.name == name) else {
            let open: Vec<&str> = self.pending.iter().map(|pc| pc.shown.as_str()).collect();
            return Err(if open.is_empty() {
                format!("no pending call of `{}` (calls with a `mock` complete immediately)", name)
            } else {
                format!("no pending call of `{}`; pending: {}", name, open.join(", "))
            });
        };
        let ids = if keep { self.pending[i].callbacks.clone() } else { self.pending.remove(i).callbacks };
        let p = self.prog.clone();
        for id in ids {
            if let Some((func, params)) = self.callbacks.get(&id).map(|c| (c.func.clone(), c.params.clone())) {
                if self.func_alive(&func) {
                    let args = self.decode_cb_args(&p, value, &params);
                    self.call_func(&p, &func, args, Span::default());
                }
                if !keep {
                    self.drop_callback(id);
                }
            }
        }
        self.settle();
        Ok(())
    }

    pub fn pending_text(&self) -> String {
        if self.pending.is_empty() {
            return "no pending native calls\n".into();
        }
        self.pending.iter().map(|pc| format!("pending: {}\n", pc.shown)).collect()
    }

    pub fn storage_text(&self) -> String {
        let mut s = String::new();
        let user_prefs: Vec<_> = self.sim.prefs.iter().filter(|(k, _)| !k.starts_with(PREF_PREFIX)).collect();
        let app_prefs: Vec<_> = self.sim.prefs.iter().filter_map(|(k, v)| k.strip_prefix(PREF_PREFIX).map(|k| (k, v))).collect();
        for (k, v) in app_prefs {
            s.push_str(&format!("pref {} = {}\n", k, v));
        }
        for (k, v) in user_prefs {
            s.push_str(&format!("prefGet({:?}) = {:?}\n", k, v));
        }
        for (k, v) in &self.sim.secure {
            s.push_str(&format!("secure {:?} = {:?}\n", k, v));
        }
        for (k, v) in &self.sim.permissions {
            s.push_str(&format!("permission {} = {}\n", k, v));
        }
        for n in &self.sim.notifications {
            s.push_str(&format!("notification: {}\n", n));
        }
        if s.is_empty() {
            s.push_str("(empty: no prefs, secure values, permissions or notifications)\n");
        }
        s
    }

    /// Run queued work that must not happen during rendering: async results delivered
    /// "later" headless, and `on appear` handlers of freshly built views.
    pub(crate) fn run_deferred(&mut self) -> bool {
        let mut did = false;
        let p = self.prog.clone();
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 1000 {
                self.error("R0006", Span::default(), "deferred native results kept re-triggering each other");
                self.deferred.clear();
                self.appear.clear();
                break;
            }
            if let Some((f, args)) = self.deferred.pop_front() {
                if self.func_alive(&f) {
                    self.call_func(&p, &f, args, Span::default());
                }
                self.pin(&f, false);
                did = true;
                continue;
            }
            if let Some(h) = self.appear.pop_front() {
                if self.scopes.contains_key(h.scope) {
                    self.run_handler(&p, h.view, h.member, h.scope, &[]);
                }
                did = true;
                let pp = self.prog.clone();
                self.flush(&pp);
                continue;
            }
            break;
        }
        did
    }
}

/// Replace the placeholder callback ids (0) in encoding order.
fn fix_callback_ids(j: &mut Json, ids: &mut impl Iterator<Item = u64>) {
    match j {
        Json::Object(o) => {
            if o.contains_key("$callback") {
                if let Some(id) = ids.next() {
                    o.insert("$callback".into(), Json::Number(id.into()));
                }
                return;
            }
            for v in o.values_mut() {
                fix_callback_ids(v, ids);
            }
        }
        Json::Array(a) => {
            for v in a {
                fix_callback_ids(v, ids);
            }
        }
        _ => {}
    }
}

pub(crate) fn zero(t: Option<&TypeRef>) -> Value {
    match t {
        None => Value::Unit,
        Some(TypeRef::Named(n, _)) => match n.as_str() {
            "Int" => Value::Int(0),
            "Float" => Value::Float(0.0),
            "String" => Value::str(""),
            "Bool" => Value::Bool(false),
            _ => Value::Nil,
        },
        Some(TypeRef::List(_)) => Value::List(Rc::new(Vec::new())),
        Some(TypeRef::Map(_)) => Value::Map(Rc::new(Vec::new())),
        _ => Value::Nil,
    }
}

pub(crate) type Deferred = VecDeque<(Value, Vec<Option<Value>>)>;
pub(crate) type Callbacks = HashMap<u64, Callback>;
