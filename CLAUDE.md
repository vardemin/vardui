# Working on the VardUI framework

Rust workspace: `crates/vard-lang` (front-end), `crates/vard-runtime` (reactive core, layout,
inspection, raster), `crates/vard-cli` (`vard` binary + MCP server), `crates/vard-ffi` (C ABI/JNI
for native hosts; Android host in `platforms/android`). Agent-facing knowledge lives
in `knowledge/` and is compiled into the binary (`include_str!`), so rebuild after editing it.

- Build/test: `cargo test --workspace`, then `./target/release/vard test examples/todo`.
- Elements, props, tokens and habit aliases are defined once in `crates/vard-lang/src/registry.rs`;
  runtime prop handling is `Node::apply` in `crates/vard-runtime/src/node.rs` — keep them in sync.
- Every new diagnostic needs a stable code, an entry in `explain.rs` (or `vard_runtime::EXPLAIN`),
  and a `fix` whenever the correct rewrite is knowable.
- The frame format (`crates/vard-ffi/src/frame.rs`) is decoded by hand in
  `platforms/android/.../Frame.kt`; bump `VERSION` and update both together. Android build:
  `platforms/android/README.md` (needs cargo-ndk + NDK; not part of `cargo test`).
- When language behaviour changes, update `knowledge/llms.txt` and the skills in the same change.
- Human docs (README, docs/) are in Russian; agent docs (knowledge/) in English.
