# Working on the VardUI framework

Rust workspace: `crates/vard-lang` (front-end + package linker `link.rs` + API/capabilities
`api.rs`), `crates/vard-runtime` (reactive core, layout, inspection, raster, native bridge +
headless platform `Sim`), `crates/vard-pkg` (vard.toml, semver, MVS resolver, vard.lock, .vpk,
registries + signed log), `crates/vard-cli` (`vard` binary + MCP server + `bindings` + package
commands `pkg.rs` + registry server `server.rs`), `crates/vard-ffi` (C ABI/JNI for native hosts).
Android host in `platforms/android` (`:vard` library, `:app` shell; packages' native modules are
included by `settings.gradle.kts`). Agent-facing knowledge lives in `knowledge/` and is
compiled into the binary (`include_str!`), so rebuild after editing it. `packages/*` are the
packages bundled into the binary as the `builtin` registry (`vard-pkg/build.rs`).

- Build/test: `cargo test --workspace`, then `vard test` on every `examples/*` project and
  every `packages/*` (their `example/` apps are the packages' usage proofs).
- A bundled package must pass the publish gate (`crates/vard-cli/tests/packages.rs` checks it):
  capabilities equal to `vard api --caps`, AGENT.md ≤ 6 KB, tests green. Changing a package's
  public API means a semver bump in its vard.toml (and `[[migrate]]` for renames).
- Every package-manager error is a `Problem` with a stable `P0xx` code and an entry in
  `vard_pkg::EXPLAIN`; compiler package errors are `E023x` in `explain.rs`.
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
- When language behaviour changes, update `knowledge/llms.txt`, `design.md`/`native.md`/
  `packages.md` and the skills in the same change; keep skill snippets compiling in
  `examples/patterns`.
- All docs are in English. Human docs: README and `docs/` — an mdBook site (`book.toml`,
  `docs/SUMMARY.md`) published to https://vardemin.github.io/vardui/ by
  `.github/workflows/docs.yml`; add new pages to `docs/SUMMARY.md`, and link files outside
  `docs/` with absolute GitHub URLs. Agent docs: `knowledge/` (compiled into the binary).
