# Delivery to devices

Vard code runs on a device in the runtime, from sources, so from Vard's point of view
"building the app" means a **bundle**: a deterministic set of files any host understands.

## vard bundle

```bash
vard bundle examples/firebase --out build/app          # a directory
vard bundle examples/firebase --out app.vpk            # a single archive
```

```text
build/app/
  app.vard                  the app's sources
  @firebase/firebase.vard   package sources (the @name/... path marks a package file for the linker)
  vard.lock                 the package graph — the compiler on the device reads it by itself
  fonts/, assets/           project resources
  bundle.json               metadata
```

```json
{
  "vard": "0.1.0",
  "sha256": "f626985f…",
  "native_fingerprint": "cffccd18…",
  "natives": ["android firebase dev.vard.firebase.FirebaseModule", "event authChanged", "fn analyticsLog", "…"],
  "caps": ["native"],
  "packages": [{ "name": "firebase", "version": "1.0.0", "sha256": null }]
}
```

- `sha256` — the hash of the bundle content (the same deterministic `.vpk` format).
- `native_fingerprint` — the hash of the sorted list of everything that must exist in the
  native part: `extern fn` and `extern event` names and the packages' native modules.
- `caps` — the resulting capabilities of the app and its packages.

The device needs no package manager: the host passes the runtime a list of `(path, text)`,
and `vard_lang::compile` recognizes `@name/...` files and `vard.lock` by itself. The FFI did
not change.

## Android

The `platforms/android` build does everything itself:

1. `settings.gradle.kts` runs `vard android-modules <vard.app>` — the list of packages with
   `[native.android]` — and includes each library as `:vard-<name>`; `:app` depends on them;
2. the `bundleVardApp` task runs `vard bundle <vard.app> --out build/generated/vardAssets/app`;
3. `VardActivity` reads `*.vard` and `vard.lock` from the assets; modules register through
   `dev.vard.module.*` meta-data in the libraries' manifests.

```bash
cd platforms/android
./gradlew installDebug -Pvard.app=examples/firebase
```

The `-Pvard.firebase=true` flag is gone: depending on the `firebase` package includes its
native module.

## Over-the-air updates (OTA)

An interpreted bundle can be replaced without rebuilding and reinstalling the app — on one
condition: **the native part has not changed**. That is exactly what the native fingerprint
checks.

| Change | Fingerprint | Deliverable OTA |
|---|---|---|
| layout, logic, styles, texts | same | yes |
| a new version of a package without native code | same | yes |
| a new `extern fn` / `extern event` or a new native module | different | no — needs a new app release |

```bash
vard ota init ./channel                       # once: prints the channel key to compile into the app
vard ota publish examples/todo --channel ./channel --version 1.0.1
```

```text
channel/
  public/config.json                    {"vard_ota": 1, "key": "<ed25519 public key>"}
  public/bundles/<sha256>.vpk           immutable bundles
  public/<native_fingerprint>/latest.json   {"sha256", "size", "version", "published", "sig"}
  private/ota.key                       the signing key — never publish it
```

`latest.json` is signed over `vard-ota/1 <fingerprint> <sha256> <version>`. A device:

1. downloads `<its own fingerprint>/latest.json` and the bundle it names (static hosting, any CDN);
2. calls `vard_ota_install(latest, bundle, len, key, fingerprint, dir)` (C ABI; Rust
   `vard_ffi::ota_install`) — it checks the signature with the key compiled into the app, the
   bundle hash, the fingerprint inside the bundle **and that the sources compile**, then
   writes the bundle to `dir` atomically; any failure leaves the current bundle in place;
3. loads `dir` instead of the built-in bundle on the next start.

Implemented: channels, publishing, verification and installation (tested in
`crates/vard-ffi/tests/ota.rs`). Not yet: the download-and-swap loop inside the Android and iOS
shells (a few dozen lines of platform code around `vard_ota_install`).

The channel is addressed by the fingerprint, so a device physically cannot receive a bundle
that needs native code it does not have. For AI-first development this closes the loop: an
agent fixed a bug → the tests passed headless → a bundle with the same fingerprint safely
goes to users; a change that needs new native code automatically becomes a store release.

## Reproducibility

- The same `vard.toml` gives the same version selection (MVS); `vard.lock` pins the hashes.
- `.vpk` archives are deterministic: sorted paths, no timestamps, no permissions.
- `vard bundle` gives the same `sha256` for the same inputs on any machine.
- Offline: packages from the cache (`~/.vard/cache/`), `builtin` and path dependencies need
  no network.
