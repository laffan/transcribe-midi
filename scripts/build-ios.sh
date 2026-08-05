#!/usr/bin/env bash
#
# Build Unplugged for iOS.
#
# WHY THIS EXISTS
#
# The build itself is one command — `tauri ios build`. On a machine that has not built
# this app for iOS before, that command fails in five different ways, and all five look
# like a bug in Tauri rather than something you are missing:
#
#   1. The `ios` subcommand does not exist off macOS. The CLI compiles it out, so what
#      you get is "unrecognized subcommand 'ios'" — which reads as a broken install.
#   2. Xcode Command Line Tools are not Xcode. Everything the desktop build needs works
#      fine with them, so the first sign that the iOS toolchain is absent is a build
#      failure deep inside xcodebuild.
#   3. `tauri ios init` generates `src-tauri/gen/apple`, and this repository does not
#      track it — a pbxproj is unreviewable in a diff and the whole tree is regenerable.
#      So a fresh clone has no Xcode project and the build has nothing to open.
#   4. CocoaPods is a hard dependency of the generated project, not of Tauri, so nothing
#      in `npm install` or `cargo` mentions it.
#   5. The Rust iOS targets are separate rustup installs from the macOS one, and the
#      error for a missing one names a linker rather than a target.
#
# Each is one line to check and a sentence to explain. Working out which of them you hit
# from the raw failure is an afternoon.
#
# THE MICROPHONE STRING
#
# It also repairs one thing specific to this app. Listen — turning audio into notes — is
# half of what Unplugged is for, and on iOS a missing `NSMicrophoneUsageDescription` does
# not produce a permission denial you can debug. The process is killed the instant it
# touches the input device, and you get a crash report.
#
# The string lives in `src-tauri/Info.plist`, which Tauri merges into the *macOS* bundle.
# iOS reads the plist inside the generated Xcode project instead — a file that `tauri ios
# init` rewrites, so editing it by hand means editing it again after every regeneration.
# This script copies the string across on every build, from the one place it is written,
# which is also what keeps the two platforms from drifting to different wording.
#
# USAGE
#
# Everything after the checks is `tauri ios build`, and every argument is passed through:
#
#   npm run build:ios                             # release, for a device, signed
#   npm run build:ios -- --debug                  # faster, for iterating
#   npm run build:ios -- --no-sign                # no Apple Developer account
#   npm run build:ios -- --open                   # build, then open Xcode
#   npm run build:ios -- --export-method debugging
#
# `npx tauri ios build --help` lists the rest. To run on a simulator or a device rather
# than produce an IPA, that is `npx tauri ios dev` — a different command, not this one.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GEN_DIR="$REPO_ROOT/src-tauri/gen/apple"
MACOS_PLIST="$REPO_ROOT/src-tauri/Info.plist"
MIC_KEY="NSMicrophoneUsageDescription"

# The device target. The simulator ones are checked too but only warned about, because a
# release build for a device does not need them.
DEVICE_TARGET="aarch64-apple-ios"
SIM_TARGETS=("aarch64-apple-ios-sim" "x86_64-apple-ios")

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  sed -n '2,49p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 0
fi

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m warning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m error:\033[0m %s\n' "$*" >&2; exit 1; }

# ------------------------------------------------------------------ checks --

[[ "$(uname -s)" == "Darwin" ]] ||
  die "iOS builds need macOS with Xcode. The Tauri CLI does not even carry the \`ios\`
        subcommand on other platforms, which is why running it there says the
        subcommand does not exist rather than saying this."

command -v xcodebuild >/dev/null 2>&1 ||
  die "xcodebuild not found. Install Xcode from the App Store — the Command Line Tools
        alone are enough for the desktop build but carry no iOS SDK."

XCODE_PATH="$(xcode-select --print-path 2>/dev/null || true)"
if [[ "$XCODE_PATH" == *CommandLineTools* ]]; then
  die "xcode-select points at the Command Line Tools ($XCODE_PATH), which have no iOS
        SDK. Point it at Xcode:
          sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer"
fi

command -v pod >/dev/null 2>&1 ||
  die "CocoaPods not found. The Xcode project \`tauri ios init\` generates depends on it:
          brew install cocoapods"

INSTALLED_TARGETS="$(rustup target list --installed 2>/dev/null || true)"

grep -qx "$DEVICE_TARGET" <<<"$INSTALLED_TARGETS" ||
  die "the Rust target $DEVICE_TARGET is not installed. Add all three at once — the
        simulator needs the other two:
          rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim"

MISSING_SIM=()
for target in "${SIM_TARGETS[@]}"; do
  grep -qx "$target" <<<"$INSTALLED_TARGETS" || MISSING_SIM+=("$target")
done
if (( ${#MISSING_SIM[@]} > 0 )); then
  warn "missing simulator target(s): ${MISSING_SIM[*]}. This build does not need them;
          \`tauri ios dev\` against a simulator does.
            rustup target add ${MISSING_SIM[*]}"
fi

# ----------------------------------------------------------- Xcode project --

# Regenerable and untracked, so on a fresh clone this is the normal path rather than an
# error. `init` is idempotent, but it also rewrites the project — only run it when there
# is nothing there, so hand edits inside Xcode survive an ordinary build.
if [[ ! -d "$GEN_DIR" ]]; then
  say "No Xcode project yet — running \`tauri ios init\`"
  ( cd "$REPO_ROOT" && npx tauri ios init )
fi

[[ -d "$GEN_DIR" ]] || die "\`tauri ios init\` did not produce $GEN_DIR"

# ------------------------------------------------------ the microphone key --

# One source of truth. If the macOS plist ever loses the key, say so rather than quietly
# shipping an iOS build that dies the first time someone presses Listen.
MIC_STRING="$(/usr/libexec/PlistBuddy -c "Print :$MIC_KEY" "$MACOS_PLIST" 2>/dev/null || true)"
if [[ -z "$MIC_STRING" ]]; then
  warn "$MACOS_PLIST has no $MIC_KEY, so there is nothing to copy to the iOS project.
          Listen will kill the app on first use until it is set."
else
  # The generated project puts the app's plist in a directory named after the product.
  while IFS= read -r plist; do
    [[ -n "$plist" ]] || continue
    if /usr/libexec/PlistBuddy -c "Print :$MIC_KEY" "$plist" >/dev/null 2>&1; then
      /usr/libexec/PlistBuddy -c "Set :$MIC_KEY $MIC_STRING" "$plist"
    else
      say "Adding $MIC_KEY to $(basename "$(dirname "$plist")")/$(basename "$plist")"
      /usr/libexec/PlistBuddy -c "Add :$MIC_KEY string $MIC_STRING" "$plist"
    fi
  done < <(find "$GEN_DIR" -maxdepth 2 -name "Info.plist" -not -path "*/Pods/*" 2>/dev/null)
fi

# ------------------------------------------------------------------- build --

say "tauri ios build ${*:-}"
cd "$REPO_ROOT"
exec npx tauri ios build "$@"
