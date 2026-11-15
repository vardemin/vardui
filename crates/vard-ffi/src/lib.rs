//! Native host surface for the Vard runtime.
//!
//! A platform host (Android/Kotlin today, iOS/Swift later) links this crate as a `cdylib`
//! and drives one [`App`] per screen:
//!
//! 1. `create` with the project's `.vard` sources and the screen size in dp,
//! 2. `mount` the root view,
//! 3. after every event (`tap`, `input_text`, `submit`, `scroll`, `set_dark`, `resize`)
//!    fetch a `frame` — the laid-out tree as a flat, paint-ordered list of draw records
//!    (see [`frame`] for the byte layout) — and draw it with the platform canvas.
//!
//! Layout, text wrapping and hit-testing stay in the Rust core, so the native screen and
//! `vard run --do layout` agree on every rectangle. Two surfaces expose the same calls:
//! a C ABI (`vard_*`, for Swift/C/C++) and JNI (`dev.vard.host.VardNative`, see [`jni_api`]).

pub mod frame;
mod jni_api;

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use vard_lang::compile;
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
        App { rt, root_view, mounted: false, compile_diags, frame: Vec::new(), diag_c: CString::default() }
    }

    pub fn runtime(&self) -> Option<&Runtime> {
        self.rt.as_ref()
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
            if n.kind == Kind::Scroll && !contains(n.rect, x, y) {
                return None; // Scroll clips its subtree
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

    pub fn set_dark(&mut self, dark: bool) {
        if let Some(rt) = self.rt.as_mut() {
            rt.set_dark(dark);
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        if let Some(rt) = self.rt.as_mut() {
            rt.resize(width.max(1.0), height.max(1.0));
        }
    }

    /// Encode the current screen (see [`frame`]). Empty when not mounted.
    pub fn frame(&mut self) -> &[u8] {
        self.frame.clear();
        if let Some(rt) = self.rt.as_mut().filter(|_| self.mounted) {
            rt.settle();
            frame::encode(rt, &mut self.frame);
        }
        &self.frame
    }

    /// Compile diagnostics (errors and warnings) followed by runtime errors so far.
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
