# VardUI on Android

The Kotlin host (`:vard` library + `:app` shell) for the Vard runtime. Full documentation:
**https://vardemin.github.io/vardui/android.html** (source: [docs/android.md](../../docs/android.md)).

Quick start, from this directory (needs the Android SDK, NDK 28.2.13676358, `cargo-ndk` and
`rustup target add aarch64-linux-android x86_64-linux-android`):

```bash
./gradlew installDebug -Pvard.app=examples/native
```

```bash
adb shell am start -n dev.vard.host/dev.vard.app.MainActivity
```

The build bundles the Vard project with `vard bundle` and includes the native modules of its
packages automatically (`vard android-modules <app>` shows which).
