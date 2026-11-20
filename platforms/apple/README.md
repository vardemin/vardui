# Vard Apple host (iOS, macOS)

Swift package `VardHost`: the Vard core and the GPU renderer (`crates/vard-gpu`) as
`VardFFI.xcframework`, a Metal-backed `VardView`, `VardViewController` for UIKit apps,
the std platform modules and `VardDemo`, a macOS window for any project.

```bash
./build-xcframework.sh --macos            # macOS slice only (fast); no flag = macOS + iOS + simulator
swift test                                # engine, frame decoding, animations, modules
swift run VardDemo ../../examples/todo    # draw a project in a window (Metal)
swift run VardDemo ../../examples/patterns Animated --check   # smoke test: draw, tap, animate, exit
./run-ios-sim.sh ../../examples/showcase "iPhone 15 Pro"      # an iOS app in the simulator, no Xcode project
```

Guide: https://vardemin.github.io/vardui/apple.html — C ABI: `crates/vard-ffi/include/vard.h`.
