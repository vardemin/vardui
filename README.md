# VardUI

A cross-platform UI framework whose primary "developer" is an LLM agent. The `Vard`
language, a reactive runtime and tooling where every step of the
"write → check → run → understand what went wrong" loop costs as few tokens as possible
and needs no device.

**Documentation: https://vardemin.github.io/vardui/** (sources in [docs/](docs/README.md)).

```
view App {
  state count = 0
  Column(padding: 24, gap: 16, align: .center, justify: .center, grow: 1) {
    Text("Tapped {count} times", style: .title)
    Button("Tap me", onTap: { count += 1 })
  }
}
```

```
$ vard run --do 'tap "Tap me"'
> tap "Tap me"
~ Text "Tapped 1 times" @2
```

## What works today (v0.3, this repository)

| Component | What it does |
|---|---|
| `crates/vard-lang` | lexer, parser with error recovery, bidirectional type checker, diagnostics with machine-applicable fixes, element registry (single source of truth for compiler, runtime and docs), std written in Vard: views and the platform API (`extern fn` / `extern event`), package linker and compiler-computed package contracts |
| `crates/vard-runtime` | fine-grained reactivity (signals / computed / effects, no virtual DOM), keyed lists, flex/grid/wrap layout that explains sizes (`why`), semantic tree, diffs, UI lints, headless PNG rasterization (shadows, gradients, per-corner radii, project fonts), JSON bridge to native code, platform simulator (prefs, secure storage, permissions, push, back, insets) |
| `crates/vard-pkg` | packages: `vard.toml`, semver, minimal version selection, `vard.lock`, deterministic `.vpk` archives, static registries (builtin, directory, HTTP) with a signed log |
| `crates/vard-cli` | `vard check [--fix] [--json]`, `run --do`, `test` (`.vtest`), `doc`, `explain`, `new`, `bindings --kotlin`; packages: `search info add remove install update api pack publish yank bundle`; `registry init/token/serve/verify`; `mcp` (MCP server on stdio) |
| `crates/vard-ffi` | `cdylib` for native shells: C ABI (`vard_*`) and JNI — mount, touches, input, theme, insets, predictive back, events, native module callbacks, platform text measurement, frame (format v2) |
| `platforms/android` | `:vard` library (runtime + `VardActivity` + core modules), `:app` shell; packages' native modules are included automatically ([how to build](https://vardemin.github.io/vardui/android.html)) |
| `packages/` | packages bundled with the toolchain (the `builtin` registry): `tailwind`, `firebase` (with an Android native module) |
| `knowledge/` | `llms.txt` (complete language reference), `design.md`, `native.md`, `packages.md`, `AGENTS.md`, skills `vard-app`, `vard-debug`, `vard-native`, `vard-design`, `vard-packages` — `vard new` puts them into every project together with `.mcp.json` |
| `examples/` | `todo`, `patterns`, `native` (permissions, push, prefs, keychain, edge-to-edge, back), `showcase` (Tailwind-style design), `firebase` — all with `.vtest` files |

### Platform features

| Feature | In the language | On Android |
|---|---|---|
| Push and its handling | `on pushReceived(m)`, `on pushOpened(m)`, `getPushToken`, `showNotification` | FCM (`firebase` package), background notifications, open on tap (including cold start) |
| Permissions | `permissionStatus`, `requestPermission(.camera, s => ...)`, `.blocked` → `openAppSettings()` | runtime permissions, "don't ask again" handled |
| Preferences | `pref theme = "light"` — state kept across launches; `prefGet/prefSet` | SharedPreferences |
| Keychain | `secureGet/secureSet/secureRemove` | AES-256-GCM, key in the Android Keystore |
| Edge-to-edge | drawn under the system bars; `safeArea: .top/.bottom/...`, `safeArea.top`, `keyboardHeight` | bar, cutout and keyboard insets; the focused `Input` scrolls above the keyboard |
| Predictive back | `on back if stack.count > 1 { ... }`, `backProgress` for animation | `OnBackAnimationCallback` (API 34+), the system back-to-home animation when the app does not intercept |
| Any design | shadows, gradients, per-corner radii, `Grid`, `Row(wrap:)`, weights 100–900, project fonts, letter spacing, offset/scale, clip | the same frame drawn with Canvas; text measured by the platform shaper |
| Native SDKs | `extern fn` / `extern event`, data = structs/enums/lists/maps | `vard bindings --kotlin` → typed base class; module libraries register themselves (`dev.vard.module.*`) |

The iOS shell, a GPU renderer, animations and bytecode compilation are on the
[roadmap](https://vardemin.github.io/vardui/architecture.html#roadmap).

## Quick start

Install a prebuilt `vard` ([releases](https://github.com/vardemin/vardui/releases)):

```bash
curl -fsSL https://raw.githubusercontent.com/vardemin/vardui/master/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/vardemin/vardui/master/install.ps1 | iex
```

Or build it: `cargo build --release` (the binary is `target/release/vard`). Then:

```bash
vard new myapp && cd myapp && vard run && vard test
```

For Claude Code / any MCP client: `vard new` creates `.mcp.json` with the `vard mcp` server
(also listed in the official MCP Registry as `io.github.vardemin/vardui`, with a one-click
`vard.mcpb` bundle on every release)
(tools `vard_check`, `vard_run`, `vard_screenshot`, `vard_test`, `vard_doc`, `vard_explain`,
`vard_search`, `vard_info`, `vard_add`, `vard_update`, `vard_api`, `vard_publish_check`,
`vard_bindings`).

"Make it look like Tailwind": `vard add tailwind` adds the palette, semantic tokens with a
dark theme and components; the class → prop mapping is `vard doc design`.
Firebase: `vard add firebase --allow native` plus `google-services.json` for Android.

## Packages built for agents

```bash
vard search bar chart       # search names, keywords, summaries and exported names
vard info charts            # versions, capabilities, API, tests — before installing
vard add charts             # exact names only; vard.toml + vard.lock
vard update --major         # API diff; then `vard check --fix` applies the package's migrations
# public registry (default): https://github.com/vardemin/vard-registry — publish = a pull request
vard registry init ./reg && vard registry serve ./reg   # or your own: static files + publish API
vard publish ./charts --registry ./reg --token …        # the gate: compile, tests, semver, capabilities
```

A package = code + **a contract the compiler computes** (public API and capabilities:
secure, push, native, network…) + AGENT.md for agents + tests run at publish time. Semver is
enforced (API diff), a person approves sensitive capabilities in `[policy]`, a guessed name
never installs, and a registry is static files with a signed log that any CDN can serve.
Details: [packages overview](https://vardemin.github.io/vardui/packages/).

## Why this suits LLMs — in numbers (examples/todo)

| Operation | Cost |
|---|---|
| the whole language reference (`vard doc`) | ~12 KB ≈ 3k tokens, once per session |
| the whole screen (`tree`) | ~400 bytes ≈ 110 tokens (a screenshot of the same screen ≈ 450–1600) |
| the result of an action (`tap`) | ~100 bytes: only the tree lines that changed |
| `vard check` | ~18 ms; errors with a ready `fix:` are applied by `--fix` without the model |
| headless run + script | ~50–100 ms, deterministic, no emulator |

Documentation — https://vardemin.github.io/vardui/: getting started, language, UI,
reactivity, platform, testing, tooling, diagnostics, packages and registries, architecture
and ADRs.

## Developing the framework itself

```bash
cargo test --workspace
```

```bash
for e in examples/* packages/*; do ./target/release/vard test $e; done
```

```bash
mdbook serve        # the documentation site, from docs/ (book.toml)
```

Rust 1.98 (`rust-toolchain.toml`). Runtime dependencies: `slotmap`, `fontdue`,
`tiny-skia`, `dejavu` (bundled font for deterministic layout), `serde_json` (native
bridge); FFI: `jni`; packages: `sha2`, `ed25519-dalek`, `getrandom` (HTTPS goes through the
system `curl`).
