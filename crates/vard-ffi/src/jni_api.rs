//! JNI bindings for `dev.vard.host.VardNative` (Kotlin `object` with `@JvmStatic external`
//! functions). The handle is a `Box<App>` pointer carried as a `long`; every call happens on
//! the UI thread (the runtime is single-threaded by design).

use crate::{App, NativeHost};
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jbyteArray, jfloat, jint, jlong, jstring, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

fn app<'a>(h: jlong) -> Option<&'a mut App> {
    // SAFETY: `h` is 0 or a pointer returned by `create` and not yet destroyed.
    unsafe { (h as *mut App).as_mut() }
}

fn string(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(Into::into)
}

fn strings(env: &mut JNIEnv, arr: &JObjectArray) -> Option<Vec<String>> {
    let n = env.get_array_length(arr).ok()?;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let o = env.get_object_array_element(arr, i).ok()?;
        let s = JString::from(o);
        out.push(string(env, &s)?);
        env.delete_local_ref(s).ok()?;
    }
    Some(out)
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_create<'l>(
    mut env: JNIEnv<'l>,
    _: JClass<'l>,
    paths: JObjectArray<'l>,
    texts: JObjectArray<'l>,
    width: jfloat,
    height: jfloat,
) -> jlong {
    guard(0, || {
        let (Some(p), Some(t)) = (strings(&mut env, &paths), strings(&mut env, &texts)) else { return 0 };
        let files: Vec<(String, String)> = p.into_iter().zip(t).collect();
        Box::into_raw(Box::new(App::new(&files, width, height))) as jlong
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_destroy(_: JNIEnv, _: JClass, h: jlong) {
    if h != 0 {
        // SAFETY: see `app`; the Kotlin side zeroes its handle after destroy.
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(h as *mut App) })));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_mount<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, view: JString<'l>) -> jint {
    let view = string(&mut env, &view);
    guard(-1, || match app(h) {
        Some(a) => match a.mount(view.as_deref()) {
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

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_tap(_: JNIEnv, _: JClass, h: jlong, x: jfloat, y: jfloat) -> jint {
    guard(-1, || app(h).map_or(-1, |a| a.tap(x, y) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_inputText<'l>(
    mut env: JNIEnv<'l>,
    _: JClass<'l>,
    h: jlong,
    uid: jint,
    text: JString<'l>,
) -> jint {
    let Some(text) = string(&mut env, &text) else { return -1 };
    guard(-1, || app(h).map_or(-1, |a| a.input_text(uid as u32, &text) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_submit(_: JNIEnv, _: JClass, h: jlong, uid: jint) -> jint {
    guard(-1, || app(h).map_or(-1, |a| a.submit(uid as u32) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_scroll(_: JNIEnv, _: JClass, h: jlong, x: jfloat, y: jfloat, dy: jfloat) -> jint {
    guard(-1, || app(h).map_or(-1, |a| a.scroll(x, y, dy) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_setDark(_: JNIEnv, _: JClass, h: jlong, dark: jboolean) {
    guard((), || {
        if let Some(a) = app(h) {
            a.set_dark(dark == JNI_TRUE)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_advance(_: JNIEnv, _: JClass, h: jlong, dt_ms: jfloat) -> jboolean {
    guard(0, || app(h).map_or(0, |a| a.advance(dt_ms) as jboolean))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_resize(_: JNIEnv, _: JClass, h: jlong, width: jfloat, height: jfloat) {
    guard((), || {
        if let Some(a) = app(h) {
            a.resize(width, height)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_frame<'l>(env: JNIEnv<'l>, _: JClass<'l>, h: jlong) -> jbyteArray {
    let buf = guard(Vec::new(), || app(h).map(|a| a.frame().to_vec()).unwrap_or_default());
    env.byte_array_from_slice(&buf).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_diagnostics<'l>(env: JNIEnv<'l>, _: JClass<'l>, h: jlong) -> jstring {
    let s = guard(String::new(), || app(h).map(|a| a.diagnostics()).unwrap_or_default());
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ---------------- native modules ----------------

/// Forwards `extern fn` calls to the Kotlin `VardHost.call(name, argsJson): String?`.
/// The Kotlin side must not call back into `VardNative` synchronously from `call`
/// (it posts callbacks to the UI thread's queue instead).
struct JniHost {
    vm: JavaVM,
    obj: GlobalRef,
}

impl NativeHost for JniHost {
    fn call(&mut self, name: &str, args_json: &str) -> Option<String> {
        let mut env = self.vm.attach_current_thread_permanently().ok()?;
        let n = env.new_string(name).ok()?;
        let a = env.new_string(args_json).ok()?;
        let res = env.call_method(&self.obj, "call", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&n), JValue::Object(&a)]);
        let _ = env.delete_local_ref(n);
        let _ = env.delete_local_ref(a);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            return Some("null".into());
        }
        let obj = res.ok()?.l().ok()?;
        if obj.is_null() {
            return None;
        }
        let js = JString::from(obj);
        let out: Option<String> = env.get_string(&js).ok().map(Into::into);
        let _ = env.delete_local_ref(js);
        out
    }
}

/// Measures text with the platform shaper: Kotlin `VardHost.measure(text, family, weight, italic, px): Float`.
struct JniMeasurer {
    vm: JavaVM,
    obj: GlobalRef,
}

impl vard_runtime::text::TextMeasurer for JniMeasurer {
    fn measure(&mut self, text: &str, family: &str, weight: u16, italic: bool, px: f32) -> f32 {
        let Ok(mut env) = self.vm.attach_current_thread_permanently() else { return 0.0 };
        let (Ok(t), Ok(f)) = (env.new_string(text), env.new_string(family)) else { return 0.0 };
        let r = env.call_method(
            &self.obj,
            "measure",
            "(Ljava/lang/String;Ljava/lang/String;IZF)F",
            &[JValue::Object(&t), JValue::Object(&f), JValue::Int(weight as i32), JValue::Bool(italic as u8), JValue::Float(px)],
        );
        let _ = env.delete_local_ref(t);
        let _ = env.delete_local_ref(f);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
            return 0.0;
        }
        r.and_then(|v| v.f()).unwrap_or(0.0)
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_setHost<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, host: JObject<'l>, platform: JString<'l>, platform_text: jboolean) {
    let platform = string(&mut env, &platform).unwrap_or_else(|| "android".into());
    let (Ok(vm), Ok(obj)) = (env.get_java_vm(), env.new_global_ref(&host)) else { return };
    let (Ok(vm2), Ok(obj2)) = (env.get_java_vm(), env.new_global_ref(&host)) else { return };
    guard((), || {
        if let Some(a) = app(h) {
            a.set_host(Box::new(JniHost { vm, obj }), &platform);
            // the GPU renderer draws with the core's fonts: then the core measures with them too
            if platform_text != 0 {
                a.set_measurer(Box::new(JniMeasurer { vm: vm2, obj: obj2 }));
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_callback<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, id: jlong, json: JString<'l>, keep: jboolean) -> jint {
    let json = string(&mut env, &json).unwrap_or_else(|| "null".into());
    guard(-1, || app(h).map_or(-1, |a| a.callback(id as u64, &json, keep == JNI_TRUE) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_release(_: JNIEnv, _: JClass, h: jlong, id: jlong) {
    guard((), || {
        if let Some(a) = app(h) {
            a.release_callback(id as u64)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_emit<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, name: JString<'l>, json: JString<'l>) -> jint {
    let (Some(name), Some(json)) = (string(&mut env, &name), string(&mut env, &json)) else { return -1 };
    guard(-1, || app(h).map_or(-1, |a| a.emit(&name, &json) as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_setInsets(_: JNIEnv, _: JClass, h: jlong, top: jfloat, bottom: jfloat, left: jfloat, right: jfloat, keyboard: jfloat) {
    guard((), || {
        if let Some(a) = app(h) {
            a.set_insets(top, bottom, left, right, keyboard)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_back(_: JNIEnv, _: JClass, h: jlong) -> jint {
    guard(-1, || app(h).map_or(-1, |a| a.back() as jint))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_backEnabled(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    guard(0, || app(h).map_or(0, |a| a.back_enabled() as jboolean))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_backProgress(_: JNIEnv, _: JClass, h: jlong, progress: jfloat) {
    guard((), || {
        if let Some(a) = app(h) {
            a.back_progress(progress)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_backCancel(_: JNIEnv, _: JClass, h: jlong) {
    guard((), || {
        if let Some(a) = app(h) {
            a.back_cancel()
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_addFont<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, file: JString<'l>, bytes: JByteArray<'l>) -> jint {
    let Some(file) = string(&mut env, &file) else { return -1 };
    let Ok(data) = env.convert_byte_array(&bytes) else { return -1 };
    guard(-1, || match app(h).map(|a| a.add_font(&file, &data)) {
        Some(Ok(())) => 0,
        _ => -1,
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_focus(_: JNIEnv, _: JClass, h: jlong, uid: jint) {
    guard((), || {
        if let Some(a) = app(h) {
            a.focus(uid as u32)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_inspectListen<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, addr: JString<'l>) -> jint {
    guard(-1, || {
        let Some(addr) = string(&mut env, &addr) else { return -1 };
        match app(h).map(|a| a.inspect_listen(&addr)) {
            Some(Ok(())) => 0,
            _ => -1,
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_inspectPoll(_: JNIEnv, _: JClass, h: jlong) -> jboolean {
    guard(0, || app(h).map(|a| a.inspect_poll() as jboolean).unwrap_or(0))
}

#[no_mangle]
pub extern "system" fn Java_dev_vard_host_VardNative_setLocale<'l>(mut env: JNIEnv<'l>, _: JClass<'l>, h: jlong, tag: JString<'l>) {
    guard((), || {
        if let (Some(t), Some(a)) = (string(&mut env, &tag), app(h)) {
            a.set_locale(&t)
        }
    })
}
