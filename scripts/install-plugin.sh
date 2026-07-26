#!/usr/bin/env bash
#
# Build Unplugged and install it where macOS will find the AUv3 inside it.
#
# WHY THIS EXISTS
#
# An AUv3 is an app extension: it ships *inside* a container app and macOS discovers it by
# scanning the app, not by scanning a plugin folder. There is no `~/Library/Audio/
# Plug-Ins/Components/` to drop a file into — that is the AUv2 world. What actually
# happens is:
#
#   1. Launch Services notices an app bundle in a location it indexes.
#   2. `pluginkit` registers the extensions inside it.
#   3. Logic asks the system for Audio Units and gets yours back.
#
# Step 1 is where a build in ~/Documents falls over. Launch Services indexes /Applications
# and ~/Applications reliably; a Documents folder is scanned inconsistently, and a build
# directory is not scanned at all. So this script copies the built app to ~/Applications
# and launches it once, which is what actually registers the extension.
#
# It also kills the extension host. Logic does not reload an AUv3 that is already running
# in `AUHostingCompatibilityService`, so without that step you replace the binary and keep
# using the old one — the failure mode this whole script exists to prevent.
#
# Requires xcodegen (brew install xcodegen). The Xcode project is generated from
# plugin/project.yml rather than checked in — a pbxproj is unreviewable in a diff.
#
# SIGNING
#
# The default build is ad-hoc signed and needs no Apple Developer account. It is sandboxed,
# and reaches the app's projects through a read-only sandbox exception rather than an App
# Group — App Groups are provisioning-profile-backed, cannot be registered on a free Apple
# ID, and make `xcodebuild` refuse to build at all without a team.
#
# With a paid membership, `--signed` builds the shipping arrangement instead: the App
# Group, properly signed. It needs a team id, which is the ten-character string in
# Xcode > Settings > Accounts, or `security find-identity -v -p codesigning`.
#
# Usage:
#   scripts/install-plugin.sh            # release build, install, register, verify
#   scripts/install-plugin.sh --debug    # faster build, for iterating
#   scripts/install-plugin.sh --no-build # install what is already built
#   UNPLUGGED_TEAM_ID=XXXXXXXXXX scripts/install-plugin.sh --signed

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Unplugged AU.app"
INSTALL_DIR="$HOME/Applications"

CONFIGURATION="release"
DO_BUILD=1

for arg in "$@"; do
  case "$arg" in
    --debug) CONFIGURATION="debug" ;;
    --signed) CONFIGURATION="signed" ;;
    --no-build) DO_BUILD=0 ;;
    -h|--help) sed -n '2,43p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m warning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m error:\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "this only means anything on macOS"

# ---------------------------------------------------------------------------
# Identify what we are about to install, before we install it
# ---------------------------------------------------------------------------

cd "$REPO_ROOT"
COMMIT="$(git rev-parse --short=7 HEAD 2>/dev/null || echo unknown)"
if ! git diff --quiet HEAD -- 2>/dev/null; then
  COMMIT="${COMMIT}+"
  warn "the working tree has uncommitted changes; the build is stamped ${COMMIT}"
fi
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

say "Installing Unplugged ${VERSION} (${COMMIT}), ${CONFIGURATION}"

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

case "$CONFIGURATION" in
  debug) XCODE_CONFIG="Debug" ;;
  signed) XCODE_CONFIG="Signed" ;;
  *) XCODE_CONFIG="Release" ;;
esac

# Signing settings are passed on the command line rather than baked into project.yml,
# because a command-line build setting applies to every target — which is exactly right
# for the team id and exactly wrong for the entitlements file, since the two targets need
# different ones. The entitlements stay per-target in project.yml.
SIGNING_ARGS=(CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual)
if [[ "$CONFIGURATION" == "signed" ]]; then
  [[ -n "${UNPLUGGED_TEAM_ID:-}" ]] || die \
    "--signed needs a team id: UNPLUGGED_TEAM_ID=XXXXXXXXXX scripts/install-plugin.sh --signed"
  SIGNING_ARGS=("UNPLUGGED_TEAM_ID=$UNPLUGGED_TEAM_ID")
  say "Signing with team $UNPLUGGED_TEAM_ID (App Group build)"
else
  say "Ad-hoc signing (no App Group — see plugin/Support/UnpluggedAU.entitlements)"
fi

if [[ "$DO_BUILD" == "1" ]]; then
  command -v xcodegen >/dev/null 2>&1 || die "xcodegen is not installed — brew install xcodegen"

  say "Generating the Xcode project"
  (cd "$REPO_ROOT/plugin" && xcodegen generate)

  # The extension's pre-build script builds the Rust staticlib, so cargo is not run here.
  say "Building the Audio Unit ($XCODE_CONFIG)"
  xcodebuild \
    -project "$REPO_ROOT/plugin/UnpluggedAU.xcodeproj" \
    -scheme UnpluggedAUHost \
    -configuration "$XCODE_CONFIG" \
    -derivedDataPath "$REPO_ROOT/target/xcode" \
    "${SIGNING_ARGS[@]}" \
    build
fi

BUILT_APP="$REPO_ROOT/target/xcode/Build/Products/$XCODE_CONFIG/$APP_NAME"
[[ -d "$BUILT_APP" ]] || die "could not find $APP_NAME at $BUILT_APP — build first, or drop --no-build"
say "Built: $BUILT_APP"

# The extension inside is what actually matters; a host bundle without one installs
# cleanly and then does nothing, with no error anywhere to say why.
EXTENSION="$BUILT_APP/Contents/PlugIns/UnpluggedAU.appex"
[[ -d "$EXTENSION" ]] || die "no UnpluggedAU.appex inside the app — the extension was not embedded"
say "Extension: $(basename "$EXTENSION")"

# ---------------------------------------------------------------------------
# Stop anything still running the old copy
# ---------------------------------------------------------------------------
#
# This is the step people skip and then spend an hour confused. macOS keeps AUv3
# extensions alive in a hosting process between uses; replacing the bundle underneath one
# does not evict it.

say "Stopping the app and any running extension host"
osascript -e 'quit app "Unplugged AU"' 2>/dev/null || true
pkill -x "Unplugged AU" 2>/dev/null || true
# These come back on demand; killing them is how a replaced extension gets picked up.
killall -9 AUHostingCompatibilityService 2>/dev/null || true
killall -9 AudioComponentRegistrar 2>/dev/null || true
sleep 1

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

mkdir -p "$INSTALL_DIR"
TARGET="$INSTALL_DIR/$APP_NAME"

say "Copying to $TARGET"
rm -rf "$TARGET"
# `-c` keeps extended attributes and the bundle's structure intact; plain `cp -r` can
# break a code signature.
ditto "$BUILT_APP" "$TARGET"

# A build that never left this machine has no quarantine flag, but one that was zipped,
# downloaded or synced through iCloud does — and a quarantined bundle registers no
# extensions and gives no reason.
if xattr -p com.apple.quarantine "$TARGET" >/dev/null 2>&1; then
  warn "clearing the quarantine flag"
  xattr -dr com.apple.quarantine "$TARGET"
fi

# ---------------------------------------------------------------------------
# Register
# ---------------------------------------------------------------------------

say "Registering with Launch Services"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
[[ -x "$LSREGISTER" ]] && "$LSREGISTER" -f "$TARGET" || warn "lsregister not found; relying on the launch below"

# Launching once is what actually makes `pluginkit` see the extension. Headless
# registration alone is not reliable.
say "Launching once so the extension registers"
open -g "$TARGET"
sleep 3

exec "$REPO_ROOT/scripts/verify-plugin.sh"
