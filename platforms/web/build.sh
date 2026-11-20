#!/bin/sh
# Build the web host: the core + GPU renderer as WebAssembly (pkg/) and a bundled project (app/).
#   ./build.sh [PROJECT_DIR]        default: examples/todo
#   python3 -m http.server -d .     then open http://localhost:8000
# Needs: rustup target add wasm32-unknown-unknown; cargo install wasm-bindgen-cli --version <the
# wasm-bindgen version in Cargo.lock>.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PROJECT=$(cd "${1:-$ROOT/examples/todo}" && pwd)
cd "$HERE"

echo "== wasm"
(cd "$ROOT" && cargo build -p vard-web --release --target wasm32-unknown-unknown)
wasm-bindgen --target web --no-typescript --out-dir pkg "$ROOT/target/wasm32-unknown-unknown/release/vard_web.wasm"
if command -v wasm-opt >/dev/null 2>&1; then wasm-opt -Oz -o pkg/vard_web_bg.wasm pkg/vard_web_bg.wasm; fi

echo "== bundle $PROJECT"
rm -rf app
(cd "$ROOT" && cargo run -q -p vard-cli -- bundle "$PROJECT" --out "$(pwd)/platforms/web/app")
# files.json: what vard.js fetches
(cd app && {
  printf '{"sources":['
  find . -type f \( -name '*.vard' -o -name 'vard.lock' \) | sed 's|^\./||' | sort | awk '{printf "%s\"%s\"", (NR>1?",":""), $0}'
  printf '],"fonts":['
  find . -type f \( -iname '*.ttf' -o -iname '*.otf' \) | sed 's|^\./||' | sort | awk '{printf "%s\"%s\"", (NR>1?",":""), $0}'
  printf ']}\n'
} > files.json)
echo "built $(pwd): serve this folder (python3 -m http.server -d $(pwd))"
