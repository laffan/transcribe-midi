//! Builds and links the Swift audio package on Apple targets.
//!
//! On every other host this is a no-op and the null backend is compiled instead, which
//! is what lets the workspace build and its tests run on a Linux CI box.
//!
//! Design note: a Swift build failure is a **hard error** here. The first version of this
//! script printed a `cargo:warning` and carried on, which produced a two-hundred-line
//! "undefined symbols" link failure that said nothing about the real cause. Failing loudly
//! at the point of failure, with Swift's own diagnostics inline, is far more useful.
//! Set `UNPLUGGED_SKIP_SWIFT=1` to build a deliberately silent app anyway.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" && target_os != "ios" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let package_dir = manifest_dir.join("../../swift/UnpluggedAudio");

    println!("cargo:rerun-if-changed={}", package_dir.join("Sources").display());
    println!("cargo:rerun-if-changed={}", package_dir.join("Package.swift").display());
    println!("cargo:rerun-if-env-changed=UNPLUGGED_SKIP_SWIFT");

    if env::var("UNPLUGGED_SKIP_SWIFT").is_ok() {
        println!("cargo:warning=UNPLUGGED_SKIP_SWIFT is set — building without audio.");
        println!("cargo:rustc-cfg=unplugged_no_swift");
        return;
    }

    // Probe for the toolchain before anything that needs `xcrun`: a host with no Swift
    // at all is a cross-`check` from a non-Mac (nothing links, skipping is sound), and
    // that must not depend on which Apple target is being checked. A *failing* build on
    // a host that has Swift stays a hard error below.
    if !host_has_swift() {
        println!(
            "cargo:warning=no `swift` toolchain on this host; skipping the Swift build \
             (fine for `cargo check`, the app cannot link here anyway)"
        );
        return;
    }

    let profile = if env::var("PROFILE").as_deref() == Ok("release") { "release" } else { "debug" };
    let target_triple = env::var("TARGET").unwrap_or_default();

    // Flags that differ between a native macOS build and an iOS cross-build.
    //
    // On macOS building for the host, SwiftPM already does the right thing — passing an
    // explicit `--triple` and `-sdk` is not just unnecessary, it is a good way to end up
    // with a mismatched SDK. So only iOS gets the extra arguments.
    let mut extra: Vec<String> = Vec::new();
    if target_os == "ios" {
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
        let swift_arch = if arch == "aarch64" { "arm64" } else { arch.as_str() };
        let simulator = target_triple.contains("sim");

        let sdk = if simulator { "iphonesimulator" } else { "iphoneos" };
        let triple = if simulator {
            format!("{swift_arch}-apple-ios15.0-simulator")
        } else {
            format!("{swift_arch}-apple-ios15.0")
        };

        let sdk_path = xcrun_sdk_path(sdk).unwrap_or_else(|| {
            panic!("could not locate the {sdk} SDK via `xcrun --sdk {sdk} --show-sdk-path`")
        });

        extra.extend(["--triple".into(), triple]);
        extra.extend(["-Xswiftc".into(), "-sdk".into(), "-Xswiftc".into(), sdk_path.clone()]);
        extra.extend(["-Xcc".into(), "-isysroot".into(), "-Xcc".into(), sdk_path]);
    }

    // ---- build -----------------------------------------------------------

    let output = run_swift(&package_dir, profile, &extra, false);

    if !output.status.success() {
        // Surface Swift's diagnostics through cargo's warning channel — otherwise they
        // are captured and invisible unless the user happens to run with `-vv`.
        emit("stderr", &String::from_utf8_lossy(&output.stderr));
        emit("stdout", &String::from_utf8_lossy(&output.stdout));
        panic!(
            "`swift build` failed in {}.\n\
             The Swift diagnostics are in the cargo:warning lines above.\n\
             To iterate on them directly:  cd swift/UnpluggedAudio && swift build\n\
             To build a silent app meanwhile:  UNPLUGGED_SKIP_SWIFT=1 cargo build",
            package_dir.display()
        );
    }

    // ---- locate the product ----------------------------------------------
    //
    // Ask SwiftPM rather than guessing at `.build/<triple>/<config>`: the layout differs
    // between host and cross builds and between SwiftPM versions.
    let bin_path = {
        let output = run_swift(&package_dir, profile, &extra, true);
        if !output.status.success() {
            emit("stderr", &String::from_utf8_lossy(&output.stderr));
            panic!("`swift build --show-bin-path` failed");
        }
        PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };

    if !bin_path.is_dir() {
        panic!("SwiftPM reported a bin path that does not exist: {}", bin_path.display());
    }

    println!("cargo:rustc-link-search=native={}", bin_path.display());

    // IMPORTANT — propagation. `cargo:rustc-link-arg` applies only to link targets of
    // the package that owns this build script, and the app binary lives in the
    // `unplugged` package — so a link-arg emitted here silently never reaches the final
    // link. (The first version of this script used `rustc-link-arg=-Wl,-force_load,…`
    // and the archive was simply absent from the link line.) `rustc-link-search` and
    // `rustc-link-lib` are the two directives cargo carries through to dependent
    // binaries, so everything below uses only those.
    //
    // Plain `static=` linkage is enough: Rust references every `unplugged_audio_*`
    // symbol directly, so the linker pulls the needed objects from the archive on
    // demand, and `-dead_strip` keeps referenced symbols. No force-load required.
    let swift_lib = bin_path.join("libUnpluggedAudio.a");
    if !swift_lib.is_file() {
        panic!(
            "expected {} to exist after a successful build — has the product name in \
             Package.swift changed?",
            swift_lib.display()
        );
    }
    println!("cargo:rustc-link-lib=static=UnpluggedAudio");

    // The C target carries only declarations, so it may or may not be emitted as its own
    // archive depending on SwiftPM version. Link it only if it is there.
    let c_lib = bin_path.join("libCUnpluggedFFI.a");
    if c_lib.is_file() {
        println!("cargo:rustc-link-lib=static=CUnpluggedFFI");
    }

    // ---- frameworks and the Swift runtime --------------------------------

    for framework in ["AVFoundation", "AudioToolbox", "CoreAudio", "CoreMIDI", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    if target_os == "ios" {
        println!("cargo:rustc-link-lib=framework=UIKit");
    } else {
        println!("cargo:rustc-link-lib=framework=AppKit");
    }

    // Search paths for the Swift runtime. The Swift objects carry autolink metadata
    // (LC_LINKER_OPTION) naming libswiftCore and friends; these `-L`s let the final
    // link resolve them. At load time the system runtime comes from the dyld shared
    // cache via absolute install names, so no rpath is needed.
    let swift_platform = if target_os == "ios" { "iphoneos" } else { "macosx" };
    if let Some(sdk_path) = xcrun_sdk_path(swift_platform) {
        println!("cargo:rustc-link-search=native={sdk_path}/usr/lib/swift");
    }
    println!("cargo:rustc-link-search=native=/usr/lib/swift");

    // Back-deployment archives — libswiftCompatibility56.a, libswiftCompatibilityPacks.a
    // and friends — live in the *toolchain*, not the SDK, and Swift objects whose
    // deployment target predates their language runtime's OS debut (macOS 12.0 < 12.3
    // for Swift 5.6) force-load them. Without this path the link dies on
    // `__swift_FORCE_LOAD_$_swiftCompatibility56`.
    if let Some(dir) = swift_toolchain_lib_dir(swift_platform) {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}

/// `…/XcodeDefault.xctoolchain/usr/lib/swift/<platform>`, derived from `swiftc`'s
/// location so it tracks whichever toolchain is selected.
fn swift_toolchain_lib_dir(platform: &str) -> Option<PathBuf> {
    let output = Command::new("xcrun").args(["--find", "swiftc"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let swiftc = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    // <toolchain>/usr/bin/swiftc -> <toolchain>/usr/lib/swift/<platform>
    let dir = swiftc.parent()?.parent()?.join("lib/swift").join(platform);
    dir.is_dir().then_some(dir)
}

fn host_has_swift() -> bool {
    Command::new("swift")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_swift(package_dir: &Path, profile: &str, extra: &[String], show_bin_path: bool) -> Output {
    let mut command = Command::new("swift");
    command.current_dir(package_dir).args(["build", "-c", profile]).args(extra);
    if show_bin_path {
        command.arg("--show-bin-path");
    }
    command
        .output()
        .unwrap_or_else(|e| panic!("could not run `swift build`: {e}"))
}

fn xcrun_sdk_path(sdk: &str) -> Option<String> {
    let output = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// Cargo only shows build-script output on `-vv`, so anything the user needs to read has
/// to go through `cargo:warning=`. Multi-line text must be emitted one line at a time.
fn emit(label: &str, text: &str) {
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        println!("cargo:warning=[swift {label}] {line}");
    }
}
