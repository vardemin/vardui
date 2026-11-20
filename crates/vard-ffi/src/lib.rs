//! Native host surface for the Vard runtime.
//!
//! A platform host (Android/Kotlin today, iOS/Swift later) links this crate as a `cdylib`
//! and drives one [`App`] per screen:
//!
//! 1. `create` with the project's `.vard` sources and the screen size in dp,
//! 2. `mount` the root view,
//! 3. after every event (`tap`, `input_text`, `submit`, `scroll`, `set_dark`, `resize`,
//!    `set_insets`, `back*`, `emit`, `callback`) fetch a `frame` — the laid-out tree as a
//!    flat, paint-ordered list of draw records (see [`frame`] for the byte layout) — and
//!    draw it with the platform canvas.
//!
//! Native modules: the host installs a [`NativeHost`] (`set_host`); every `extern fn` call
//! arrives there as `(name, JSON args)`. Function arguments are `{"$callback": id}` handles
//! the host answers later with `callback(id, json, keep)`. Platform events (push, deep
//! links, lifecycle) go in with `emit(name, json_args)`.
//!
//! Layout, text wrapping and hit-testing stay in the Rust core, so the native screen and
//! `vard run --do layout` agree on every rectangle. Two surfaces expose the same calls:
//! a C ABI (`vard_*`, for Swift/C/C++) and JNI (`dev.vard.host.VardNative`, see [`jni_api`]).

pub mod frame;
pub mod inspect;
mod jni_api;

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use vard_lang::compile;
use vard_runtime::layout::apply_xf;
pub use vard_runtime::native::NativeHost;
use vard_runtime::node::Kind;
use vard_runtime::rt::NodeKey;
use vard_runtime::Runtime;

/// Result of an event call. `Ignored` means the event reached nothing that reacts to it
/// (empty space, no `onTap` up the chain, a disabled element) — not an error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Handled = 1,
    Ignored = 0,
    Error = -1,
}

/// One running Vard screen.
pub struct App {
    rt: Option<Runtime>,
    root_view: Option<String>,
    mounted: bool,
    compile_diags: String,
    frame: Vec<u8>,
    diag_c: CString,
    /// scripts from `vard run --device` waiting for the UI thread (see [`App::inspect_listen`])
    inspector: Option<inspect::Inbox>,
}

impl App {
    /// Compile `files` (`(path, text)`, std is added automatically) for a `width`×`height` dp
    /// screen. On compile errors the app is inert and [`App::diagnostics`] explains why.
    pub fn new(files: &[(String, String)], width: f32, height: f32) -> App {
        let files: Vec<(String, String)> = files.iter().map(|(p, t)| (p.clone(), t.replace("\r\n", "\n"))).collect();
        let c = compile(&files);
        let compile_diags = c.render_diags();
        let failed = c.has_errors();
        let root_view = if c.program.view_index.contains_key("App") {
            Some("App".to_string())
        } else {
            // same fallback as `vard run`: the last view declared in user code
            c.program.views.iter().rev().find(|v| v.span.file != 0).map(|v| v.name.clone())
        };
        let rt = if failed { None } else { Some(Runtime::new(c, width.max(1.0), height.max(1.0))) };
        App { rt, root_view, mounted: false, compile_diags, frame: Vec::new(), diag_c: CString::default(), inspector: None }
    }

    pub fn runtime(&self) -> Option<&Runtime> {
        self.rt.as_ref()
    }

    /// Folder that `Image("assets/...")` paths are relative to (renderers in Rust load from it).
    pub fn set_asset_dir(&mut self, dir: &str) {
        if let Some(rt) = self.rt.as_mut() {
            rt.asset_dir = Some(std::path::PathBuf::from(dir));
        }
    }

    /// Mount `view` (default: `App`, else the last view in user code) as the root.
    pub fn mount(&mut self, view: Option<&str>) -> Result<(), String> {
        let rt = self.rt.as_mut().ok_or("the project has compile errors; see diagnostics")?;
        if self.mounted {
            return Err("already mounted".into());
        }
        let name = view.map(str::to_string).or_else(|| self.root_view.clone()).ok_or("the project declares no views; add `view App { ... }`")?;
        rt.mount(&name)?;
        self.mounted = true;
        Ok(())
    }

    fn live(&mut self) -> Option<&mut Runtime> {
        if self.mounted {
            self.rt.as_mut()
        } else {
            None
        }
    }

    /// Deepest visible node under (x, y) in dp, in paint order (last painted wins).
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeKey> {
        let rt = self.rt.as_ref()?;
        fn walk(rt: &Runtime, k: NodeKey, x: f32, y: f32) -> Option<NodeKey> {
            let n = rt.nodes.get(k)?;
            if (n.kind == Kind::Scroll || n.clip) && !contains(apply_xf(n.rect, n.xf), x, y) {
                return None; // Scroll / clip: true clips its subtree
            }
            if let Some(hit) = n.children.iter().rev().find_map(|&c| walk(rt, c, x, y)) {
                return Some(hit);
            }
            let r = rt.visible_rect(k)?;
            contains(r, x, y).then_some(k)
        }
        walk(rt, rt.root, x, y)
    }

    /// Tap at (x, y) dp: hit-test, then bubble to the nearest `onTap` like `Runtime::tap`.
    pub fn tap(&mut self, x: f32, y: f32) -> Status {
        let Some(hit) = self.hit(x, y) else { return self.status_ignored() };
        let Some(rt) = self.live() else { return Status::Error };
        let mut cur = Some(hit);
        let mut target = None;
        while let Some(k) = cur {
            let n = &rt.nodes[k];
            if n.on_tap.is_some() {
                target = n.enabled.then_some(k);
                break;
            }
            cur = n.parent;
        }
        match target {
            Some(k) if rt.tap(k).is_ok() => Status::Handled,
            _ => Status::Ignored,
        }
    }

    fn status_ignored(&self) -> Status {
        if self.mounted {
            Status::Ignored
        } else {
            Status::Error
        }
    }

    fn by_uid(&self, uid: u32) -> Option<NodeKey> {
        let rt = self.rt.as_ref()?;
        rt.nodes.iter().find(|(_, n)| n.uid == uid).map(|(k, _)| k)
    }

    /// Replace the text of Input `uid` (fires its binding and `onChange`).
    pub fn input_text(&mut self, uid: u32, text: &str) -> Status {
        let Some(k) = self.by_uid(uid) else { return self.status_ignored() };
        let Some(rt) = self.live() else { return Status::Error };
        if rt.nodes[k].text.as_ref() == text {
            return Status::Ignored;
        }
        match rt.input_text(k, text) {
            Ok(()) => Status::Handled,
            Err(_) => Status::Ignored,
        }
    }

    /// Input `uid` got (or, with 0, everything lost) keyboard focus; the core keeps it
    /// scrolled above the keyboard.
    pub fn focus(&mut self, uid: u32) {
        let k = if uid == 0 { None } else { self.by_uid(uid) };
        if let Some(rt) = self.live() {
            rt.set_focus(k);
        }
    }

    /// Fire `onSubmit` of Input `uid` (the IME action key).
    pub fn submit(&mut self, uid: u32) -> Status {
        let Some(k) = self.by_uid(uid) else { return self.status_ignored() };
        let Some(rt) = self.live() else { return Status::Error };
        match rt.submit(k) {
            Ok(()) => Status::Handled,
            Err(_) => Status::Ignored,
        }
    }

    /// Scroll the nearest Scroll under (x, y) by `dy` dp (positive = content moves up).
    pub fn scroll(&mut self, x: f32, y: f32, dy: f32) -> Status {
        let Some(hit) = self.hit(x, y) else { return self.status_ignored() };
        let Some(rt) = self.live() else { return Status::Error };
        let before = scroll_pos(rt, hit);
        match rt.scroll_by(hit, dy) {
            Ok(()) if scroll_pos(rt, hit) != before => Status::Handled,
            _ => Status::Ignored,
        }
    }

    /// The system light/dark setting (an app's `setColorScheme` overrides it).
    pub fn set_dark(&mut self, dark: bool) {
        if let Some(rt) = self.rt.as_mut() {
            rt.set_system_dark(dark);
        }
    }

    /// The user's language (BCP 47, e.g. `ru`, `pt-BR`): `t()` and `locale` follow it.
    pub fn set_locale(&mut self, tag: &str) {
        if let Some(rt) = self.rt.as_mut() {
            rt.set_locale(tag);
        }
    }

    /// Install the platform's native-module dispatcher. Call before `mount`.
    pub fn set_host(&mut self, host: Box<dyn NativeHost>, platform: &str) {
        if let Some(rt) = self.rt.as_mut() {
            rt.set_host(host, platform);
        }
    }

    /// Measure platform-font text with the host's shaper (see `TextMeasurer`).
    pub fn set_measurer(&mut self, m: Box<dyn vard_runtime::text::TextMeasurer>) {
        if let Some(rt) = self.rt.as_mut() {
            rt.text.set_measurer(m);
        }
    }

    /// Register a project font file (`fonts/Inter-SemiBold.ttf` -> family "Inter", 600).
    pub fn add_font(&mut self, file_name: &str, bytes: &[u8]) -> Result<(), String> {
        let rt = self.rt.as_mut().ok_or("the project has compile errors")?;
        let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
        let (family, weight, italic) = vard_runtime::text::parse_font_name(stem);
        rt.text.add_font(&family, weight, italic, bytes)
    }

    /// System bars + display cutout (top, bottom, left, right) and keyboard height, dp.
    pub fn set_insets(&mut self, top: f32, bottom: f32, left: f32, right: f32, keyboard: f32) {
        if let Some(rt) = self.rt.as_mut() {
            rt.set_insets(top, bottom, left, right, keyboard);
        }
    }

    /// Deliver a native result to callback `id`; `keep` for listeners that fire repeatedly.
    pub fn callback(&mut self, id: u64, json: &str, keep: bool) -> Status {
        match self.live().map(|rt| rt.invoke_callback(id, json, keep)) {
            Some(true) => Status::Handled,
            Some(false) => Status::Ignored,
            None => Status::Error,
        }
    }

    pub fn release_callback(&mut self, id: u64) {
        if let Some(rt) = self.rt.as_mut() {
            rt.release_callback(id);
        }
    }

    /// Platform event (`extern event name(...)`), `args` a JSON array.
    pub fn emit(&mut self, name: &str, args: &str) -> Status {
        match self.live().map(|rt| rt.emit(name, args)) {
            Some(Ok(n)) if n > 0 => Status::Handled,
            Some(Ok(_)) => Status::Ignored,
            Some(Err(e)) => {
                self.compile_diags.push_str(&format!("emit: {}\n", e));
                Status::Error
            }
            None => Status::Error,
        }
    }

    /// Whether the app takes the next back gesture (an enabled `on back` handler exists).
    pub fn back_enabled(&mut self) -> bool {
        self.live().is_some_and(|rt| rt.back_enabled())
    }

    /// Predictive back progress 0..1 while the gesture is in flight.
    pub fn back_progress(&mut self, progress: f32) {
        if let Some(rt) = self.live() {
            rt.back_progress(progress);
        }
    }

    /// Back committed. `Ignored` = the app did not handle it; the host does the system back.
    pub fn back(&mut self) -> Status {
        match self.live().map(|rt| rt.back()) {
            Some(true) => Status::Handled,
            Some(false) => Status::Ignored,
            None => Status::Error,
        }
    }

    pub fn back_cancel(&mut self) {
        if let Some(rt) = self.live() {
            rt.back_cancel();
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        if let Some(rt) = self.rt.as_mut() {
            rt.resize(width.max(1.0), height.max(1.0));
        }
    }

    /// Advance the animation clock by `dt_ms` (the time since the previous display frame).
    /// Returns whether animations are still running — keep requesting frames while true.
    pub fn advance(&mut self, dt_ms: f32) -> bool {
        match self.rt.as_mut().filter(|_| self.mounted) {
            // a stalled frame (app in background) must not jump into the middle of nowhere
            Some(rt) => rt.advance(dt_ms.clamp(0.0, 100.0) as f64),
            None => false,
        }
    }

    /// Whether animations are running (also bit 2 of the frame header).
    pub fn animating(&self) -> bool {
        self.rt.as_ref().is_some_and(|rt| rt.animating())
    }

    /// Encode the current screen (see [`frame`]). Empty when not mounted.
    pub fn frame(&mut self) -> &[u8] {
        self.frame.clear();
        if let Some(rt) = self.rt.as_mut().filter(|_| self.mounted) {
            rt.settle();
            let mut buf = std::mem::take(&mut self.frame);
            frame::encode(rt, &mut buf);
            self.frame = buf;
        }
        &self.frame
    }

    /// Compile diagnostics (errors and warnings) followed by runtime errors so far.
    /// Run an action script (the language of `vard run --do`) against the live app.
    pub fn run_actions(&mut self, script: &str) -> (String, bool) {
        match self.live() {
            Some(rt) => rt.run_script(script),
            None => ("the app is not running (compile errors?)
".into(), false),
        }
    }

    /// Debug builds: accept action scripts on `addr` (e.g. `127.0.0.1:8472`, reached with
    /// `adb forward tcp:8472 tcp:8472`). Scripts run on the UI thread in [`App::inspect_poll`].
    pub fn inspect_listen(&mut self, addr: &str) -> Result<(), String> {
        self.inspector = Some(inspect::listen(addr)?);
        Ok(())
    }

    /// Run the scripts that arrived since the last call; `true` if any ran (redraw).
    pub fn inspect_poll(&mut self) -> bool {
        let Some(inbox) = self.inspector.take() else { return false };
        let mut ran = false;
        while let Ok(req) = inbox.try_recv() {
            let (out, ok) = self.run_actions(&req.script);
            let _ = req.reply.send((out, ok));
            ran = true;
        }
        self.inspector = Some(inbox);
        ran
    }

    pub fn diagnostics(&self) -> String {
        let mut s = self.compile_diags.clone();
        if let Some(rt) = &self.rt {
            for e in &rt.errors {
                s.push_str(e);
                s.push('\n');
            }
        }
        s
    }

    pub fn has_errors(&self) -> bool {
        self.rt.is_none() || self.rt.as_ref().is_some_and(|rt| !rt.errors.is_empty())
    }
}

fn contains(r: vard_runtime::node::Rect, x: f32, y: f32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

fn scroll_pos(rt: &Runtime, mut k: NodeKey) -> f32 {
    loop {
        let n = &rt.nodes[k];
        if n.kind == Kind::Scroll {
            return n.scroll_y;
        }
        match n.parent {
            Some(p) => k = p,
            None => return f32::NAN,
        }
    }
}

// ---------------- C ABI ----------------

/// One source file for [`vard_create`]: NUL-terminated UTF-8 path and text.
#[repr(C)]
pub struct VardSource {
    pub path: *const c_char,
    pub text: *const c_char,
}

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

unsafe fn app<'a>(p: *mut App) -> Option<&'a mut App> {
    p.as_mut()
}

/// Create an app from `count` sources and a screen size in dp. Never returns null unless
/// the arguments are invalid; check `vard_mount`'s result and `vard_diagnostics`.
///
/// # Safety
/// `files` must point to `count` valid [`VardSource`] records.
#[no_mangle]
pub unsafe extern "C" fn vard_create(files: *const VardSource, count: usize, width: f32, height: f32) -> *mut App {
    guard(std::ptr::null_mut(), || {
        if files.is_null() && count > 0 {
            return std::ptr::null_mut();
        }
        let mut list = Vec::with_capacity(count);
        for i in 0..count {
            let f = &*files.add(i);
            match (cstr(f.path), cstr(f.text)) {
                (Some(p), Some(t)) => list.push((p.to_string(), t.to_string())),
                _ => return std::ptr::null_mut(),
            }
        }
        Box::into_raw(Box::new(App::new(&list, width, height)))
    })
}

/// # Safety
/// `app` must come from [`vard_create`] and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn vard_destroy(app: *mut App) {
    if !app.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(app))));
    }
}

/// Mount `view` (null = default root). 0 on success, -1 on failure (see diagnostics).
///
/// # Safety
/// `app` from [`vard_create`]; `view` null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn vard_mount(app: *mut App, view: *const c_char) -> i32 {
    guard(-1, || match self::app(app) {
        Some(a) => match a.mount(cstr(view)) {
            Ok(()) => 0,
            Err(e) => {
                a.compile_diags.push_str(&e);
                a.compile_diags.push('\n');
                -1
            }
        },
        None => -1,
    })
}

/// Tap at (x, y) dp. Returns a [`Status`] as i32.
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_tap(app: *mut App, x: f32, y: f32) -> i32 {
    guard(-1, || self::app(app).map_or(-1, |a| a.tap(x, y) as i32))
}

/// # Safety
/// `app` from [`vard_create`]; `text` NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vard_input_text(app: *mut App, uid: u32, text: *const c_char) -> i32 {
    guard(-1, || match (self::app(app), cstr(text)) {
        (Some(a), Some(t)) => a.input_text(uid, t) as i32,
        _ => -1,
    })
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_submit(app: *mut App, uid: u32) -> i32 {
    guard(-1, || self::app(app).map_or(-1, |a| a.submit(uid) as i32))
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_scroll(app: *mut App, x: f32, y: f32, dy: f32) -> i32 {
    guard(-1, || self::app(app).map_or(-1, |a| a.scroll(x, y, dy) as i32))
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_set_dark(app: *mut App, dark: bool) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.set_dark(dark)
        }
    })
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_resize(app: *mut App, width: f32, height: f32) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.resize(width, height)
        }
    })
}

/// Input `uid` has keyboard focus (0 = none); the core keeps it above the keyboard.
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_focus(app: *mut App, uid: u32) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.focus(uid)
        }
    })
}

/// Register a project font file (`Inter-SemiBold.ttf`); the core measures and draws with it.
/// 0 on success, -1 on a bad font.
///
/// # Safety
/// `app` from [`vard_create`]; `file_name` NUL-terminated; `bytes` points to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn vard_add_font(app: *mut App, file_name: *const c_char, bytes: *const u8, len: usize) -> i32 {
    guard(-1, || {
        let (Some(a), Some(name)) = (self::app(app), cstr(file_name)) else { return -1 };
        if bytes.is_null() {
            return -1;
        }
        match a.add_font(name, std::slice::from_raw_parts(bytes, len)) {
            Ok(()) => 0,
            Err(_) => -1,
        }
    })
}

/// Folder that `Image("assets/...")` sources are relative to (for the Rust GPU renderer).
///
/// # Safety
/// `app` from [`vard_create`]; `dir` NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vard_set_asset_dir(app: *mut App, dir: *const c_char) {
    guard((), || {
        if let (Some(a), Some(d)) = (self::app(app), cstr(dir)) {
            a.set_asset_dir(d)
        }
    })
}

/// Drop a kept callback (a listener the native side will never call again).
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_release(app: *mut App, id: u64) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.release_callback(id)
        }
    })
}

/// Does an enabled `on back` handler take the next back gesture?
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_back_enabled(app: *mut App) -> bool {
    guard(false, || self::app(app).is_some_and(|a| a.back_enabled()))
}

/// Advance animations by `dt_ms` since the previous display frame; true while animations
/// run (then draw a new `vard_frame` and call again on the next vsync).
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_advance(app: *mut App, dt_ms: f32) -> bool {
    guard(false, || self::app(app).is_some_and(|a| a.advance(dt_ms)))
}

/// Encode the current frame. The buffer is owned by `app` and valid until the next call
/// on it. `*out_len` receives its length (0 when not mounted).
///
/// # Safety
/// `app` from [`vard_create`]; `out_len` writable.
#[no_mangle]
pub unsafe extern "C" fn vard_frame(app: *mut App, out_len: *mut usize) -> *const u8 {
    guard(std::ptr::null(), || {
        let Some(a) = self::app(app) else { return std::ptr::null() };
        let buf = a.frame();
        if !out_len.is_null() {
            *out_len = buf.len();
        }
        buf.as_ptr()
    })
}

/// Diagnostics as a NUL-terminated string owned by `app`, valid until the next call.
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_diagnostics(app: *mut App) -> *const c_char {
    guard(std::ptr::null(), || {
        let Some(a) = self::app(app) else { return std::ptr::null() };
        a.diag_c = CString::new(a.diagnostics().replace('\0', " ")).unwrap_or_default();
        a.diag_c.as_ptr()
    })
}

/// Native-module callback for C/Swift hosts: answer `extern fn name(args_json)` with a
/// NUL-terminated JSON string that stays valid until the next call (host-owned), or null
/// when the function is not implemented.
pub type VardNativeFn = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, name: *const c_char, args_json: *const c_char) -> *const c_char;

struct CHost {
    f: VardNativeFn,
    ctx: *mut std::ffi::c_void,
}

impl NativeHost for CHost {
    fn call(&mut self, name: &str, args_json: &str) -> Option<String> {
        let (n, a) = (CString::new(name).ok()?, CString::new(args_json).ok()?);
        // SAFETY: the host promised a valid callback and a string valid until the next call.
        let r = unsafe { (self.f)(self.ctx, n.as_ptr(), a.as_ptr()) };
        if r.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(r) }.to_str().ok().map(str::to_string)
        }
    }
}

/// Install the native-module dispatcher (before `vard_mount`).
///
/// # Safety
/// `app` from [`vard_create`]; `f` callable with `ctx` on the UI thread for the app's life.
#[no_mangle]
pub unsafe extern "C" fn vard_set_host(app: *mut App, f: VardNativeFn, ctx: *mut std::ffi::c_void, platform: *const c_char) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.set_host(Box::new(CHost { f, ctx }), cstr(platform).unwrap_or("native"));
        }
    })
}

/// # Safety
/// `app` from [`vard_create`]; `json` NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vard_callback(app: *mut App, id: u64, json: *const c_char, keep: bool) -> i32 {
    guard(-1, || match (self::app(app), cstr(json)) {
        (Some(a), Some(j)) => a.callback(id, j, keep) as i32,
        _ => -1,
    })
}

/// # Safety
/// `app` from [`vard_create`]; strings NUL-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vard_emit(app: *mut App, name: *const c_char, args_json: *const c_char) -> i32 {
    guard(-1, || match (self::app(app), cstr(name), cstr(args_json)) {
        (Some(a), Some(n), Some(j)) => a.emit(n, j) as i32,
        _ => -1,
    })
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_set_insets(app: *mut App, top: f32, bottom: f32, left: f32, right: f32, keyboard: f32) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.set_insets(top, bottom, left, right, keyboard)
        }
    })
}

/// 1 = the app handled back, 0 = do the system back.
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_back(app: *mut App) -> i32 {
    guard(-1, || self::app(app).map_or(-1, |a| a.back() as i32))
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_back_progress(app: *mut App, progress: f32) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.back_progress(progress)
        }
    })
}

/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_back_cancel(app: *mut App) {
    guard((), || {
        if let Some(a) = self::app(app) {
            a.back_cancel()
        }
    })
}

// ---------------- over-the-air updates ----------------

/// Verify a downloaded update and install it into `dir` (the app's update directory).
///
/// `latest` is the channel's `<fingerprint>/latest.json`, `bundle` the `.vpk` it names,
/// `key` the channel's public key compiled into the app and `fingerprint` the app's own
/// native fingerprint (from the `bundle.json` it shipped with). The bundle is accepted only
/// if the signature, the hash and the fingerprint match **and its sources compile**; then it
/// is written to `dir` (old files replaced) and its version is returned. On the next start
/// the host loads `dir` instead of the built-in bundle; on any error it keeps the old one.
pub fn ota_install(latest: &str, bundle: &[u8], key: &str, fingerprint: &str, dir: &std::path::Path) -> Result<String, String> {
    let (version, files) = vard_pkg::ota::verify(latest, bundle, key, fingerprint)?;
    let sources: Vec<(String, String)> = files
        .iter()
        .filter(|f| f.0.ends_with(".vard") || f.0 == "vard.lock")
        .map(|f| (f.0.clone(), String::from_utf8_lossy(&f.1).replace("\r\n", "\n")))
        .collect();
    let c = compile(&sources);
    if c.has_errors() {
        return Err(format!("the update does not compile; keeping the current bundle:
{}", c.render_diags()));
    }
    let staging = dir.with_extension("incoming");
    let _ = std::fs::remove_dir_all(&staging);
    for (rel, bytes) in &files {
        let p = staging.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&staging, dir).map_err(|e| format!("cannot install the update: {}", e))?;
    Ok(version)
}

/// C ABI for [`ota_install`]: returns NULL on success, else an error message the caller
/// frees with [`vard_string_free`].
///
/// # Safety
/// String arguments are NUL-terminated UTF-8; `bundle` points to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn vard_ota_install(latest: *const c_char, bundle: *const u8, len: usize, key: *const c_char, fingerprint: *const c_char, dir: *const c_char) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (Some(l), Some(k), Some(f), Some(d)) = (cstr(latest), cstr(key), cstr(fingerprint), cstr(dir)) else {
            return CString::new("invalid arguments").unwrap().into_raw();
        };
        let bytes = if bundle.is_null() { &[][..] } else { std::slice::from_raw_parts(bundle, len) };
        match ota_install(l, bytes, k, f, std::path::Path::new(d)) {
            Ok(_) => std::ptr::null_mut(),
            Err(e) => CString::new(e.replace('\0', " ")).unwrap_or_default().into_raw(),
        }
    })
}

/// Free a string returned by [`vard_ota_install`].
///
/// # Safety
/// `s` must come from this library (or be NULL).
#[no_mangle]
pub unsafe extern "C" fn vard_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// C ABI for [`App::inspect_listen`]: 0 on success, -1 on error.
///
/// # Safety
/// `app` from [`vard_create`]; `addr` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn vard_inspect_listen(app: *mut App, addr: *const c_char) -> i32 {
    guard(-1, || match (self::app(app), cstr(addr)) {
        (Some(a), Some(addr)) => a.inspect_listen(addr).map(|_| 0).unwrap_or(-1),
        _ => -1,
    })
}

/// C ABI for [`App::inspect_poll`]: 1 if scripts ran (pull a frame), else 0.
///
/// # Safety
/// `app` from [`vard_create`].
#[no_mangle]
pub unsafe extern "C" fn vard_inspect_poll(app: *mut App) -> i32 {
    guard(0, || self::app(app).map(|a| a.inspect_poll() as i32).unwrap_or(0))
}

/// C ABI for [`App::set_locale`].
///
/// # Safety
/// `app` from [`vard_create`]; `tag` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn vard_set_locale(app: *mut App, tag: *const c_char) {
    guard((), || {
        if let (Some(a), Some(t)) = (self::app(app), cstr(tag)) {
            a.set_locale(t)
        }
    })
}
