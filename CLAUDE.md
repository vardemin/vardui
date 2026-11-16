# Working on the VardUI framework

Rust workspace: `crates/vard-lang` (front-end), `crates/vard-runtime` (reactive core, layout,
inspection, raster, native bridge + headless platform `Sim`), `crates/vard-cli` (`vard` binary
+ MCP server + `bindings`), `crates/vard-ffi` (C ABI/JNI for native hosts). Android host in
`platforms/android` (`:vard` library, `:app` shell, `:vard-firebase` module). Agent-facing
knowledge lives in `knowledge/` and is compiled into the binary (`include_str!`), so rebuild
after editing it.

- Build/test: `cargo test --workspace`, then `vard test` on every `examples/*` project.
- Elements, props, tokens, globals and habit aliases are defined once in
  `crates/vard-lang/src/registry.rs`; runtime prop handling is `Node::apply` in
  `crates/vard-runtime/src/node.rs` — keep them in sync. Paint-only props go in
  `node::affects_layout`.
- The platform API (permissions, prefs, secure, push, events) is declared in
  `crates/vard-lang/src/std.vard`; every std extern needs a headless implementation in
  `native::sim_call` and an Android one in `platforms/android/vard/.../CoreModules.kt`.
- Every new diagnostic needs a stable code, an entry in `explain.rs` (or `vard_runtime::EXPLAIN`),
  and a `fix` whenever the correct rewrite is knowable.
- The frame format (`crates/vard-ffi/src/frame.rs`) is decoded by hand in
  `platforms/android/vard/.../Frame.kt`; bump `VERSION` and update both together. JNI entry
  points (`jni_api.rs`) must match `VardNative.kt`; Rust calls `VardHost.call` and
  `VardHost.measure` by name. Android build: `platforms/android/README.md` (needs cargo-ndk +
  NDK; not part of `cargo test`).
- When language behaviour changes, update `knowledge/llms.txt`, `design.md`/`native.md` and
  the skills in the same change; keep skill snippets compiling in `examples/patterns`.
- Human docs (README, docs/) are in Russian; agent docs (knowledge/) in English.
