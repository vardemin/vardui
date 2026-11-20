# VardUI architecture

## Goals and their consequences

| Goal | Consequence in the design |
|---|---|
| AI writes the code right the first time | a small regular language; syntax close to Swift/Kotlin/TS, where models have strong priors; one form for each thing (`name: value`, no modifier chains) |
| errors get fixed without thinking | a diagnostic = `file:line` + the line of code + `fix:`; `vard check --fix` applies fixes to a fixed point; "habit" tables (`VStack→Column`, `onPress→onTap`, `var→state`, `"a"+n→"a{n}"`) |
| debugging without screenshots | a semantic tree with `@id`s, diffs after actions, `why` for sizes, `state`, lints with a ready recipe |
| minimal tokens | compact output formats; the reference is generated from the same registry as the checks; MCP tools are stateless — a "session" is a replayable list of actions |
| performance | fine-grained reactivity: a view body runs once, then exactly the prop whose dependencies changed is updated; single-pass layout |
| extensibility | design systems = plain Vard views (std is written that way); native functions = `extern fn`, platform events = `extern event`; elements = registry entries |
| an ecosystem for agents | packages with a contract computed by the compiler (API, capabilities), enforced semver, migrations as `fix:`es, installation by exact names only, a static signed registry ([packages](packages/)) |

## Layers

```
 .vard ──► vard-lang ──────────────────────────► vard-runtime ───────────────► platform
           lexer → parser (arena AST,             reactive graph (signals,      headless: PNG (tiny-skia)
           error recovery) → linker →             computed, effects, scopes)    Android (Kotlin host, Canvas)
           checker (bidirectional types,          retained Node tree            iOS / macOS (Swift host,
           fixes)                                 flex layout + why tracing     vard-gpu on Metal)
           registry.rs — elements, props,         inspect: tree/layout/state/lint   + native text fields
           tokens, habit aliases                  actions: tap/type/scroll/expect
                         vard-cli: check · run · test · doc · explain · new · mcp
                                   add · install · update · search · info · api
                                   pack · publish · bundle · registry (serve)
                         vard-pkg: vard.toml · semver · MVS · vard.lock · .vpk · registries + log
```

### vard-lang
- **Lexer** — line breaks are separators (except inside `(`/`[` and before continuation
  lines `.x`, `&&`, `?`, `:`). It catches `;`, `===`, `@State` and the like right away, with
  a fix.
- **Parser** — recursive descent, the AST in flat arenas (`ExprId`/`UiId` = `u32`), so the
  runtime keeps cheap indices in effects. Recovery at the line and declaration level: one
  run reports as many independent errors as possible; "unknown name" cascades after a
  syntax error are suppressed.
- **Checker** — the expected type flows down: `.center`, `.primary` and lambda parameters
  are inferred without annotations. The `Unknown` type after an error is compatible with
  everything — no avalanche of secondary errors. When the fix is obvious (an alias or an
  edit distance), checking continues as if it were already applied — one iteration instead
  of several.
- **registry.rs** — the single source of truth: elements, props and their types, enums,
  color tokens, the type scale, alias tables. `vard doc` is generated from it too.

### Packages: linking, API, capabilities (vard-lang)
- **link.rs** — namespaces without imports. Package files arrive with `@name/...` paths,
  the graph comes from `vard.lock` (or a `LinkInfo` from the CLI, with migrations).
  Resolution order: own → direct dependencies → std. Conflicting package declarations are
  renamed to `pkg.Name` *before* type checking, so the checker and the runtime work with
  unique names and know nothing about packages. Errors `E0230`–`E0235`; package migrations
  become `E0234` with a fix, and checking continues as if it were applied.
- **api.rs** — the public API (canonical items, a diff classified as major/minor/patch) and
  capabilities computed from the code with evidence (`secureGet @ file:line`).

### vard-pkg
The data and protocol layer of the package manager, with no TLS stack: a TOML subset with
diagnostics and fixes, semver, minimal version selection, `vard.lock`, deterministic `.vpk`,
registries (`builtin` — the embedded `packages/`, a directory, HTTP) with key pinning and
verification of the signed hash chain, and loading a project with its dependencies for the
compiler (self-healing: a stale lock is reinstalled).

### vard-runtime
- **Reactivity.** `state` → Signal, `let` and view parameters → Computed (lazy, cached,
  auto-tracked), every dynamic prop → an Effect writing one field of one node; `if`/`for` →
  structural effects. Ownership is a tree of `Scope`s: disposing a scope frees everything
  created inside (effects, nodes, signals). Effects run in creation order (parent before
  child), which rules out glitches.
- **Equality cut-offs** at every level: a signal does not notify for the same value, a prop
  does not touch the node for the same value, unchanged props do not mark layout dirty;
  static props (literals, enums, lambdas) create no effects at all.
- **Keyed `for`.** Reconciliation by the `by` key; each row has its own value signal, so
  changing one item of a 3000-item list re-runs one row's effects (measured: 7 actions over
  3000 rows — 35 effect runs).
- **Values** — `Rc` + copy-on-write (`Rc::make_mut`): copying is free, mutation never
  creates aliases. That makes the semantics predictable for a model.
- **Layout** — constraints down, sizes up, one pass. CSS flex semantics (`align`, `justify`,
  `grow`, `gap`). Every node remembers *why* it got its size on each axis — `why` prints the
  chain. Line wrapping is cached by (font, width).
- **Inspection** — the semantic tree (meaningless containers hidden, labels merged as in
  accessibility), diffs, selectors `@id` / `#id` / text, lints, `state`.
- **Runtime errors** do not crash the app: any problem → an `R` diagnostic with `file:line`,
  the value becomes `nil`. Nodes from std views point at the place where the app uses the view.

### The native bridge
- **The boundary** is two declarations: `extern fn` (app → platform) and `extern event`
  (platform → app; `on name(...) [if cond] { }` handlers live while the view is on screen).
  Only data crosses (structs, enums, lists, maps, optionals) and callbacks `fn(T)` for
  results that come "later"; the checker forbids everything else (`E0317`).
- **The format** is JSON. Encoding is untyped, decoding follows the type *declared* in Vard
  (struct fields by name, enums by variant name, defaults for missing fields). So one native
  function serves any app type: `firestoreGet*` in the `firebase` package returns a JSON
  document, and Vard turns it into a `Profile` or `Note` struct — whatever the developer
  declared.
- **Callbacks** — functions in arguments become `{"$callback": id}`; the host answers with
  `callback(id, json, keep)`. A closure created inside a handler pins the temporary scopes
  it captured — they outlive the call until the platform answers. A callback of a view that
  has already left the screen is dropped silently.
- **The std platform** (`std.vard`): permissions, `pref`/`prefGet`, `secure*`
  (Keychain/Keystore), push and notifications, `setColorScheme`, `openUrl`, `haptic`, events
  `appear`/`back`/`appResume`/`appPause`/`deepLink`/`push*`, globals `safeArea`,
  `keyboardHeight`, `backProgress`, `isDark`, `screenWidth`.
- **Headless** — `Sim` in the runtime plays the platform deterministically: real in-memory
  prefs/secure storage, the permission dialog answers what the test said, push/back/insets
  are injected by actions, `restart` models a cold start. User externs answer with a `mock`
  value or stay `pending` until `resolve`/`send`; packages' `[sim]` sets defaults.
- **`pref`** — state the runtime persists through the same boundary (`prefGet/prefSet` with
  the key `vard.pref.<name>`), one signal per name for the whole app.
- **Edge-to-edge and back** — the host always draws under the system bars and passes the
  insets; `safeArea` adds them to padding. If the code never mentions `safeArea`, the runtime
  pads the root itself (a static flag from the checker). `on back if cond` tells the host
  whether the app intercepts back (a flag in the frame header) — that decides whether Android
  shows the system predictive animation or hands the gesture progress to the app.
- **Text on devices** — the core measures the platform font through the host's
  `TextMeasurer` (Android `Paint.measureText`), project fonts (`fonts/`) with the same file
  through fontdue. Line breaks and alignment on screen match what the layout computed.

### vard-cli and MCP
Thin commands over the libraries. `pkg.rs` holds the package commands and the publish
*gate* (shared by `vard publish` and the registry server); `server.rs` is the registry's HTTP
hosting. The MCP server (JSON-RPC over stdio) is stateless: `vard_run` mounts the app from
scratch each time and replays the actions — 50–100 ms, deterministic, nothing to go stale
between calls.

## The debugging model for AI

1. `vard check` → errors with fixes. 2. `vard run --do "tree"` → the screen as text.
3. Actions print only the changes. 4. `why`/`layout` give the numbers and the rule that made
the size. 5. Lints describe the problem and the recipe. 6. A screenshot — only to judge visual
design.

## Roadmap

This version is a reference implementation of the core and tooling. For production on
devices:

1. **Native shells.** The Rust core builds as a `cdylib`/`staticlib`; Android — a Kotlin host
   (JNI), iOS — a Swift host (C ABI). The runtime emits a buffer of tree mutation commands
   (create/set/insert/remove), the host applies them per frame.
   *Done:* `crates/vard-ffi` (C ABI + JNI) and `platforms/android` (the `:vard` library with
   `VardActivity`, the `:app` shell, the `firebase` package's native module). The host gets
   the whole frame — a flat list of draw records (format v3: shadows, gradients, per-corner
   radii, fonts, transforms, `crates/vard-ffi/src/frame.rs`); hit-testing and `onTap`
   bubbling stay in the core, `Input` is a native `EditText` over the Canvas;
   `platforms/apple` — the Swift package `VardHost` for iOS and macOS (`VardView` on a
   `CAMetalLayer`, `VardViewController`, the std modules over UserDefaults, Keychain,
   UserNotifications and the permission frameworks; `include/vard.h` is checked against the
   exported symbols). A mutation buffer instead of the full frame is the next step.
2. **Rendering.** The main path is our own GPU renderer over the existing tree and layout:
   pixel-identical arbitrary designs and predictable performance. Native platform views
   are embedded where integration matters: text input with IME, maps, WebView, camera,
   video. *Done:* `crates/vard-gpu` — wgpu, one SDF pipeline (rounded boxes, borders,
   gradients, shadows, rounded clips), a glyph atlas from the core's fonts, images; parity
   with the headless rasterizer is a test; the Apple host draws with it on Metal, native
   text fields float over Inputs ([Rendering](rendering.md)). Every host now draws with
   it: Android (`SurfaceView`, Vulkan/GLES, Canvas fallback), iOS/macOS (Metal), desktop
   (`vard-desktop`: winit + DX12/Vulkan/GL/Metal) and the web (`vard-web`: WebAssembly +
   WebGPU/WebGL2). Next: HarfBuzz-class shaping with system fonts.
3. **Execution.** The AST interpreter is the dev engine (instant start, hot reload). For
   release — compilation to a compact register bytecode or AOT to Rust; type checks are
   already static, so runtime checks go away.
4. **Native modules.** *Done:* `extern fn` with callbacks, `extern event` + `on`, the std
   platform (permissions, push, prefs, secure storage, back, insets), `vard bindings --kotlin`
   and `--swift` (the Swift output is type-checked by `swiftc` in CI), self-registering module
   libraries, Firebase as a package with a native module, automatic inclusion of packages'
   native modules in the Android build. Next: `async/await` sugar over callbacks.
5. **Language.** *Done:* `Map`, `pref`, `Grid`, wrap, shadows/gradients/fonts, localized text
   (`strings` tables, `t()`, the `locale` global — checked at compile time). Animations: `animate:` interpolates
   prop changes in the effect layer without re-running logic, `enter:` transitions, a frame
   clock from the host (`vard_advance`, the `animating` frame flag) and `wait MS` for
   deterministic tests. Next: navigation as a built-in element (today a stack + `on back`
   pattern), exit transitions, gestures, virtualized long lists, image caching.
6. **Accessibility.** The semantic tree exists — mapping it to TalkBack/VoiceOver.
7. **On-device inspection.** *Done:* the action protocol over TCP (`vard run --device`), a
   listener in `vard-ffi` that runs scripts on the UI thread, enabled in debuggable Android
   builds. Next: a `device` parameter for the MCP tools.
8. **Packages.** *Done:* vard.toml/vard.lock, MVS, the namespace linker, API and capabilities
   from the compiler, enforced semver, migrations, the publish gate, a static signed registry,
   `vard registry serve`, `vard bundle` with the native fingerprint, git sources, enum/field
   migrations, behavioral checks with the previous version's tests, quarantine and provenance,
   signed key rotation, a web page per registry, the public registry, signed OTA channels with
   on-device verification (`vard ota`). Next: the OTA download loop in the hosts, Merkle
   inclusion proofs, Sigstore attestations (see
   [the package overview](packages/#what-is-implemented-and-what-is-on-the-roadmap)).
