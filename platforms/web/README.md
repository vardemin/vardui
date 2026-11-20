# Vard web host

The core and the GPU renderer (`crates/vard-web`) as WebAssembly, drawn into a `<canvas>`
with WebGPU (WebGL2 where WebGPU is missing); `vard.js` is the host.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130   # = wasm-bindgen in Cargo.lock
./build.sh ../../examples/todo
python3 -m http.server -d . 8000                   # http://localhost:8000/?view=App
```

Guide: https://vardemin.github.io/vardui/web.html
