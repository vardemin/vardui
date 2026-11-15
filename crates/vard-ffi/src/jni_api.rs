//! JNI bindings for `dev.vard.host.VardNative` (Kotlin `object` with `@JvmStatic external`
//! functions). The handle is a `Box<App>` pointer carried as a `long`; every call happens on
//! the UI thread (the runtime is single-threaded by design).

use crate::App;
use jni::objects::{JClass, JObjectArray, JString};
use jni::sys::{jboolean, jbyteArray, jfloat, jint, jlong, jstring, JNI_TRUE};
use jni::JNIEnv;
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
