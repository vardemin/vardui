#!/bin/sh
# Build a Vard project into an iOS app for the simulator, without an Xcode project:
#   ./run-ios-sim.sh PROJECT_DIR [DEVICE_NAME]      e.g. ./run-ios-sim.sh ../../examples/todo "iPhone 15 Pro"
# Needs VardFFI.xcframework with the ios-arm64-simulator slice (./build-xcframework.sh).
set -eu
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
PROJECT=$(cd "$1" && pwd)
DEVICE=${2:-}
OUT=.build/ios-sim
APP="$OUT/VardDemo.app"
SDK=$(xcrun --sdk iphonesimulator --show-sdk-path)
ARCH=$(uname -m | sed 's/x86_64/x86_64/;s/arm64/arm64/')
TARGET="$ARCH-apple-ios15.0-simulator"

echo "== VardHost for the simulator"
xcodebuild -scheme VardHost -destination 'generic/platform=iOS Simulator' -derivedDataPath .build/xcode -quiet build
P=.build/xcode/Build/Products/Debug-iphonesimulator

echo "== app binary"
rm -rf "$APP" && mkdir -p "$APP"
xcrun --sdk iphonesimulator swiftc -target "$TARGET" -sdk "$SDK" -I "$P" -I VardFFI.xcframework/ios-arm64-simulator/Headers -Onone \
  Examples/iOSDemo/main.swift "$P/VardHost.o" VardFFI.xcframework/ios-arm64-simulator/libvard_gpu.a \
  -framework UIKit -framework Metal -framework QuartzCore -framework Security -framework UserNotifications \
  -framework AVFoundation -framework Photos -framework Contacts -framework CoreLocation -lc++ \
  -Xlinker -rpath -Xlinker @executable_path/Frameworks -o "$APP/VardDemo"

echo "== bundle $PROJECT"
(cd "$ROOT" && cargo run -q -p vard-cli -- bundle "$PROJECT" --out "$(pwd)/platforms/apple/$APP/app")
cat > "$APP/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>dev.vard.demo</string>
  <key>CFBundleName</key><string>VardDemo</string>
  <key>CFBundleExecutable</key><string>VardDemo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UILaunchScreen</key><dict/>
  <key>UIRequiredDeviceCapabilities</key><array><string>metal</string></array>
  <key>CFBundleURLTypes</key><array><dict><key>CFBundleURLSchemes</key><array><string>vard</string></array></dict></array>
</dict></plist>
PLIST
codesign --force --sign - "$APP" >/dev/null

if [ -n "$DEVICE" ]; then
  xcrun simctl boot "$DEVICE" 2>/dev/null || true
  xcrun simctl install "$DEVICE" "$APP"
  xcrun simctl launch "$DEVICE" dev.vard.demo
fi
echo "built $(pwd)/$APP"
