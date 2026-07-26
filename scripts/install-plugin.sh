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
# Usage:
#   scripts/install-plugin.sh            # release build, install, register, verify
#   scripts/install-plugin.sh --debug    # faster build, for iterating
#   scripts/install-plugin.sh --no-build # install what is already built

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Unplugged.app"
INSTALL_DIR="$HOME/Applications"

CONFIGURATION="release"
DO_BUILD=1

for arg in "$@"; do
  case "$arg" in
    --debug) CONFIGURATION="debug" ;;
    --no-build) DO_BUILD=0 ;;
    -h|--help) sed -n '2,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
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

if [[ "$DO_BUILD" == "1" ]]; then
  say "Building the frontend"
  npm run build

  say "Building the app"
  if [[ "$CONFIGURATION" == "debug" ]]; then
    npm run tauri build -- --debug
  else
    npm run tauri build
  fi
fi

BUILT_APP="$(find "$REPO_ROOT/target" -maxdepth 4 -name "$APP_NAME" -type d 2>/dev/null | head -1)"
[[ -n "$BUILT_APP" ]] || die "could not find $APP_NAME under target/ — build first, or drop --no-build"
say "Built: $BUILT_APP"

# ---------------------------------------------------------------------------
# Stop anything still running the old copy
# ---------------------------------------------------------------------------
#
# This is the step people skip and then spend an hour confused. macOS keeps AUv3
# extensions alive in a hosting process between uses; replacing the bundle underneath one
# does not evict it.

say "Stopping the app and any running extension host"
osascript -e 'quit app "Unplugged"' 2>/dev/null || true
pkill -x Unplugged 2>/dev/null || true
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
