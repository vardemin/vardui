# Android shell

The same Rust core (`vard-runtime`) as in `vard run`, built as `libvard_ffi.so`, plus a
Kotlin host that draws the result and connects the platform. Sources:
[`platforms/android`](https://github.com/vardemin/vardui/tree/master/platforms/android).

```
 platforms/android
 ├── vard/            library: libvard_ffi.so + VardView, VardHost, VardActivity, core modules
 ├── app/             the shell app: bundles a Vard project (vard.app) and extends VardActivity
 (packages' native modules — `packages/*/native/android` — are included automatically)
```

```
 assets/app/**.vard, fonts/, assets/ ──► VardNative.create ──► Rust: compile → mount → layout
                                                     │  frame(): draw records (format v2)
 VardView.onDraw  ◄──────────── ByteArray ───────────┘  rect, background, gradient, shadow, radii, text, ...
 touch ─► VardNative.tap(x, y)            hit-test and bubbling to onTap — in the core
 EditText ─► inputText / submit / focus   a real EditText over each Input, the IME works
 extern fn ─► VardHost.call(name, json)  ─► VardModule (Kotlin) ─► callback(id, json) / emit(event)
 insets / back gesture / onResume / intents ─► setInsets / backProgress / back / emit(...)
```

## What `VardActivity` does

| Feature | How it works |
|---|---|
| Edge-to-edge | the window draws under the status bar, navigation and cutout on every API 26+; bar, cutout and keyboard insets go to the core (`safeArea`); bar icons follow the app theme; the focused `Input` scrolls above the keyboard |
| Predictive back | `android:enableOnBackInvokedCallback="true"`; while the app has an enabled `on back`, an `OnBackAnimationCallback` (API 34+) is registered and the gesture progress goes to `backProgress`; otherwise the system shows its back-to-home animation. API 33 — `OnBackInvokedCallback`, below — `onBackPressed` |
| Permissions | `PermissionsModule`: `requestPermissions` + `shouldShowRequestPermissionRationale` → `granted`/`denied`/`blocked`; a permission missing from the manifest is `denied` with a hint in logcat |
| Preferences | `PrefsModule` over SharedPreferences; `pref` variables are stored there too |
| Keychain | `SecureModule`: AES-256-GCM, the key never leaves the Android Keystore |
| Notifications and push | `VardPush`: in the foreground — the `pushReceived` event, in the background — a system notification; a tap opens the app and sends `pushOpened` (also on cold start). The token and delivery come from a provider (the `firebase` package) |
| Lifecycle, deep links | `appResume`/`appPause`, `deepLink(url)` for `vard://...` |
| Modules | the core ones + every library with `<meta-data android:name="dev.vard.module.<name>" android:value="<class>"/>` + the app's `modules()` |
| Fonts and images | the project's `fonts/*.ttf` are registered in the core and drawn with the same file; `Image("assets/x.png")` from assets, `https://` loaded in the background |

## Requirements

| Component | Version | Where to get it |
|---|---|---|
| Rust | from `rust-toolchain.toml` | `rustup target add aarch64-linux-android x86_64-linux-android` |
| cargo-ndk | 4.x | `cargo install cargo-ndk --locked` |
| Android SDK | platform 35, build-tools 35 | Android Studio → SDK Manager |
| NDK | 28.2.13676358 (`ndkVersion` in `vard/build.gradle.kts`) | SDK Manager → SDK Tools → NDK (Side by side) |
| JDK | 17–21 | e.g. the JBR from Android Studio |

The Gradle wrapper (Gradle 9.3.1, AGP 9.1.0) is in the repository. The SDK path goes into
`platforms/android/local.properties` (not committed):
`sdk.dir=C\:\\Users\\<you>\\AppData\\Local\\Android\\Sdk`.

## Build and run

From `platforms/android`:

```bash
./gradlew installDebug -Pvard.app=examples/native
```

```bash
adb shell am start -n dev.vard.host/dev.vard.app.MainActivity
```

| Property | Default | Meaning |
|---|---|---|
| `vard.app` | `examples/todo` | which Vard project to package (path from the repository root) |
| `vard.abis` | `arm64-v8a,x86_64` | ABIs for `cargo ndk -t`; for an emulator `-Pvard.abis=x86_64` is enough |
| `vard.skipCargo` | `false` | do not call cargo, take prebuilt `.so` files from `vard/src/main/jniLibs/<abi>/` |
| `vard.applicationId` | `dev.vard.host` | the app's applicationId |

Vard compile errors are shown on screen; runtime errors (`R…`) and module hints go to logcat:

```bash
adb logcat -s Vard VardFirebase
```

## Driving the real screen from `vard`

Debuggable builds listen for the action protocol on `127.0.0.1:8472`:

```bash
adb forward tcp:8472 tcp:8472
vard run --device 127.0.0.1:8472 --do 'tree; tap "Add"'
```

The locale follows the system language (`Configuration.locales[0]`, also on change), so
`t()` texts switch with it.

## Checking platform features on an emulator

A push through the same path as FCM (debug builds only, `DebugPushReceiver`):

```bash
adb shell am broadcast -a dev.vard.DEBUG_PUSH -p dev.vard.host --es title "Hi" --es body "Text" --es data '{"route":"inbox"}'
```

Predictive back on API 34 is enabled with a developer option (default on 35+):

```bash
adb shell settings put global enable_back_animation 1
```

A deep link:

```bash
adb shell am start -a android.intent.action.VIEW -d "vard://item/7" dev.vard.host
```

## Your own native module (any SDK)

1. Declare the API in `.vard`: `extern fn searchPlaces(query: String, done: fn([Place], String?))`,
   `extern event locationChanged(lat: Float, lng: Float)`.
2. Generate a typed base class:
   `vard bindings examples/myapp --kotlin --package com.example --class PlacesModule --out app/src/main/kotlin/PlacesModule.kt`.
3. Implement the abstract methods (callbacks may be called from any thread) and return the
   module from `MainActivity.modules()` — or move it into a library with
   `<meta-data android:name="dev.vard.module.places" .../>` (that is how the `firebase`
   package works), or better, ship it as a package with `[native.android]`
   (see [Writing a package](packages/authoring.md#native-code)): then the shell includes it
   by itself.

## Firebase

```bash
vard add firebase examples/myapp --allow native
```

Put `google-services.json` into `platforms/android/app/`. No separate flag is needed: the
package declares `[native.android] gradle = "native/android"`, and `settings.gradle.kts`
includes the module itself (`vard android-modules examples/myapp` shows what will be
included). Without the file the module starts, but every call answers
`FirebaseError(code: "not-configured")`. Firestore works with any app struct: declare
`firestoreGet<X>`/`firestoreList<X>`/... with your type — one native implementation serves
them all.

## How the app gets into the APK

The `bundleVardApp` task runs `vard bundle <vard.app> --out build/generated/vardAssets/app`:
the app's sources, the packages' sources (`@name/...`), `vard.lock` (the package graph for
the linker), `fonts/`, `assets/` and `bundle.json` with the content hash and the *native
fingerprint* (the list of externs and native modules). The fingerprint is the compatibility
key for delivering updates without rebuilding the APK
([Delivery to devices](packages/delivery.md)).

## Limitations

- Drawing with `Canvas`, a whole frame at a time; a GPU renderer and tree diffs are on the
  [roadmap](architecture.md#roadmap).
- Shadows through `Paint.setShadowLayer` (hardware-accelerated on API 28+).
- Accessibility (TalkBack) is not yet connected to the semantic tree; scrolling has no inertia.
- There is no iOS shell yet: the C ABI for it (`vard_set_host`, `vard_callback`, `vard_emit`,
  `vard_back*`, `vard_set_insets`) is ready.
