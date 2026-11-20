#!/bin/sh
# Build VardFFI.xcframework: crates/vard-ffi as a staticlib with the `gpu` feature (the core,
# the C ABI and the wgpu renderer) for macOS, iOS and the iOS simulator,
# with include/vard.h exposed to Swift as the `VardFFI` module.
#
#   ./build-xcframework.sh            release build of every slice
#   ./build-xcframework.sh --macos    only the macOS slice (fast local loop, `swift run`)
#   PROFILE=debug ./build-xcframework.sh
set -eu
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
PROFILE=${PROFILE:-release}
FLAG=$([ "$PROFILE" = release ] && echo --release || echo)
OUT=VardFFI.xcframework
TARGETS="aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim"
[ "${1:-}" = "--macos" ] && TARGETS="aarch64-apple-darwin"

INSTALLED=$(rustup target list --installed 2>/dev/null || true)
for t in $TARGETS; do
  if [ -n "$INSTALLED" ] && ! echo "$INSTALLED" | grep -qx "$t"; then
    if [ "$t" = x86_64-apple-darwin ]; then echo "== skip $t (not installed): arm64-only macOS slice"; TARGETS=$(echo "$TARGETS" | sed 's/x86_64-apple-darwin//'); continue; fi
    echo "missing Rust target $t: rustup target add $t" >&2; exit 1
  fi
  echo "== cargo build $t"
  (cd "$ROOT" && cargo build -p vard-ffi --features gpu $FLAG --target "$t")
done
lib() { echo "$ROOT/target/$1/$PROFILE/libvard_ffi.a"; }

HEADERS=$(mktemp -d)
cp "$ROOT/crates/vard-ffi/include/vard.h" "$HEADERS/"
cat > "$HEADERS/module.modulemap" <<'MAP'
module VardFFI {
    header "vard.h"
    link "c++"
    export *
}
MAP

ARGS=""
MAC="$(lib aarch64-apple-darwin)"
if [ -f "$(lib x86_64-apple-darwin)" ] && echo "$TARGETS" | grep -q x86_64; then
  mkdir -p "$ROOT/target/universal-macos/$PROFILE"
  lipo -create "$(lib aarch64-apple-darwin)" "$(lib x86_64-apple-darwin)" -output "$ROOT/target/universal-macos/$PROFILE/libvard_ffi.a"
  MAC="$ROOT/target/universal-macos/$PROFILE/libvard_ffi.a"
fi
ARGS="$ARGS -library $MAC -headers $HEADERS"
for t in aarch64-apple-ios aarch64-apple-ios-sim; do
  if echo "$TARGETS" | grep -q "$t\( \|$\)"; then ARGS="$ARGS -library $(lib $t) -headers $HEADERS"; fi
done
rm -rf "$OUT"
# shellcheck disable=SC2086
xcodebuild -create-xcframework $ARGS -output "$OUT"
rm -rf "$HEADERS"
echo "built $(pwd)/$OUT"
