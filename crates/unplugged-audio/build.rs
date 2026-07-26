//! Builds and links the Swift audio package on Apple targets.
//!
//! On every other host this is a no-op and the null backend is compiled instead, which
//! is what lets the workspace build and its tests run on a Linux CI box.
//!
//! UNVERIFIED: this script has never been executed — it requires a Swift toolchain.
//! Treat it as a starting point rather than known-good. In particular, for iOS device
//! and simulator builds the `-sdk` / `-target` pair below may need adjusting to match
//! what `tauri ios build` passes.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" && target_os != "ios" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let package_dir = manifest_dir.join("../../swift/UnpluggedAudio");

    println!("cargo:rerun-if-changed={}", package_dir.display());

    let profile = if env::var("PROFILE").as_deref() == Ok("release") {
        "release"
    } else {
        "debug"
    };

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let swift_arch = if arch == "aarch64" { "arm64" } else { arch.as_str() };

    // Simulator and device are different triples with the same arch, so the distinction
    // has to come from the Cargo target rather than the arch alone.
    let target_triple = env::var("TARGET").unwrap_or_default();
    let (sdk, swift_target) = match target_os.as_str() {
        "ios" if target_triple.contains("sim") => ("iphonesimulator", format!("{swift_arch}-apple-ios15.0-simulator")),
        "ios" => ("iphoneos", format!("{swift_arch}-apple-ios15.0")),
        _ => ("macosx", format!("{swift_arch}-apple-macosx12.0")),
    };

    let sdk_path = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let Some(sdk_path) = sdk_path else {
        println!("cargo:warning=could not locate the {sdk} SDK; skipping the Swift build");
        return;
    };

    let mut command = Command::new("swift");
    command
        .current_dir(&package_dir)
        .args(["build", "-c", profile])
        .args(["--triple", &swift_target])
        .args(["-Xswiftc", "-sdk", "-Xswiftc", &sdk_path])
        .args(["-Xcc", "-isysroot", "-Xcc", &sdk_path]);

    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!("cargo:warning=`swift build` failed with {status}; the app will not produce sound");
            return;
        }
        Err(error) => {
            println!("cargo:warning=could not run `swift build`: {error}");
            return;
        }
    }

    let build_dir = package_dir.join(".build").join(&swift_target).join(profile);
    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=UnpluggedAudio");
    println!("cargo:rustc-link-lib=static=CUnpluggedFFI");

    // Frameworks the Swift code links against.
    for framework in ["AVFoundation", "AudioToolbox", "CoreAudio", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    if target_os == "ios" {
        println!("cargo:rustc-link-lib=framework=UIKit");
    }

    // The Swift runtime itself.
    println!("cargo:rustc-link-search=native={sdk_path}/usr/lib/swift");
}
