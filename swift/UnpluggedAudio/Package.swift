// swift-tools-version:5.9
import PackageDescription

// One Swift package, linked into BOTH the macOS and iOS binaries.
//
// Tauri's Swift plugin mechanism is iOS-only, but it exists so JavaScript can call
// Swift — and nothing in this app does. Rust owns the engine and the webview only talks
// to Rust, so we bypass it entirely and use a plain C ABI on both platforms. That gives
// one binding path instead of two divergent ones. See DECISIONS.md.
let package = Package(
    name: "UnpluggedAudio",
    platforms: [
        .macOS(.v12),
        .iOS(.v15),
    ],
    products: [
        .library(name: "UnpluggedAudio", type: .static, targets: ["UnpluggedAudio"]),
        .library(name: "UnpluggedPlatform", type: .static, targets: ["UnpluggedPlatform"]),
    ],
    targets: [
        .target(name: "CUnpluggedFFI"),
        .target(
            name: "UnpluggedAudio",
            dependencies: ["CUnpluggedFFI"]
        ),
        // Phase 5: share sheet, pasteboard and drag-out. No dependency on the audio
        // target — it is called from Rust the same way, but the two share nothing.
        .target(name: "UnpluggedPlatform"),
    ]
)
