// swift-tools-version:5.9
// The Apple host for Vard: a Metal-backed view that draws frames with the Rust GPU
// renderer, native text fields over Inputs, the std platform modules and the C ABI glue.
// Build the core first: ./build-xcframework.sh (or --macos for a quick local slice).
import PackageDescription

let package = Package(
    name: "VardHost",
    platforms: [.iOS(.v15), .macOS(.v14)],
    products: [
        .library(name: "VardHost", targets: ["VardHost"]),
        .executable(name: "VardDemo", targets: ["VardDemo"]),
    ],
    targets: [
        .binaryTarget(name: "VardFFI", path: "VardFFI.xcframework"),
        .target(
            name: "VardHost",
            dependencies: ["VardFFI"],
            linkerSettings: [
                .linkedFramework("Metal"),
                .linkedFramework("QuartzCore"),
                .linkedFramework("Security"),
                .linkedFramework("UserNotifications"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("Photos"),
                .linkedFramework("Contacts"),
                .linkedFramework("CoreLocation"),
            ]
        ),
        .executableTarget(name: "VardDemo", dependencies: ["VardHost"]),
        .testTarget(name: "VardHostTests", dependencies: ["VardHost"]),
    ]
)
