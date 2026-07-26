#!/usr/bin/env bash
#
# Answer one question: which build of Unplugged will Logic load right now?
#
# Worth its own script because the honest answer has three parts that can disagree:
#
#   - what is installed in ~/Applications,
#   - what `pluginkit` has registered,
#   - what the Audio Unit system will actually hand a host.
#
# When a fix "does not work", it is usually because those three are out of step — most
# often an old extension still registered from a copy that has since been deleted. This
# prints all three so the disagreement is visible instead of being guessed at.

set -uo pipefail

INSTALL_DIR="$HOME/Applications"
APP="$INSTALL_DIR/Unplugged.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m warning:\033[0m %s\n' "$*" >&2; }
ok() { printf '\033[32m  ok\033[0m %s\n' "$*"; }
bad() { printf '\033[31m  no\033[0m %s\n' "$*"; }

[[ "$(uname -s)" == "Darwin" ]] || { echo "macOS only"; exit 1; }

# ---------------------------------------------------------------------------
# 1. What is installed
# ---------------------------------------------------------------------------

say "Installed app"
if [[ -d "$APP" ]]; then
  PLIST="$APP/Contents/Info.plist"
  VERSION="$(defaults read "$PLIST" CFBundleShortVersionString 2>/dev/null || echo '?')"
  BUILD="$(defaults read "$PLIST" CFBundleVersion 2>/dev/null || echo '?')"
  MODIFIED="$(stat -f '%Sm' -t '%Y-%m-%d %H:%M:%S' "$APP")"
  ok "$APP"
  echo "     version $VERSION, build $BUILD, installed $MODIFIED"
else
  bad "nothing at $APP — run scripts/install-plugin.sh"
fi

# Copies elsewhere are the usual cause of "I fixed it and nothing changed": macOS may have
# registered a different one, and there is no indication in Logic which it picked.
say "Other copies on this machine"
OTHERS="$(mdfind -name 'Unplugged.app' 2>/dev/null | grep -v "^$APP$" || true)"
if [[ -n "$OTHERS" ]]; then
  warn "more than one copy exists; macOS may have registered one of these instead"
  echo "$OTHERS" | sed 's/^/     /'
else
  ok "only the installed copy"
fi

# ---------------------------------------------------------------------------
# 2. What the extension system knows about
# ---------------------------------------------------------------------------

say "Registered app extensions"
PK="$(pluginkit -mAv -p com.apple.AudioUnit-UI 2>/dev/null | grep -i unplugged || true)"
if [[ -n "$PK" ]]; then
  ok "pluginkit has it registered"
  echo "$PK" | sed 's/^/     /'
else
  bad "pluginkit does not list it"
  echo "     Launch the app once from $INSTALL_DIR, then run this again."
  echo "     An app outside /Applications or ~/Applications is often never scanned."
fi

# ---------------------------------------------------------------------------
# 3. What a host will actually load
# ---------------------------------------------------------------------------

say "Audio Units the system will offer a host"
if command -v auval >/dev/null 2>&1; then
  AUVAL="$(auval -a 2>/dev/null | grep -i unplugged || true)"
  if [[ -n "$AUVAL" ]]; then
    ok "visible to Audio Unit hosts"
    echo "$AUVAL" | sed 's/^/     /'
    echo
    echo "     Validate it fully with:  auval -v aumi Unpl Lffn"
  else
    bad "no Unplugged Audio Unit is registered"
    echo "     If pluginkit above found it, the extension is registered but its"
    echo "     AudioComponents entry is wrong — check the extension's Info.plist."
  fi
else
  warn "auval not found (it ships with Xcode's command line tools)"
fi

# ---------------------------------------------------------------------------
# The bit that actually settles arguments
# ---------------------------------------------------------------------------

cat <<'NOTE'

==> Confirming inside Logic

  The plugin shows its own build stamp — version, short commit, and a "+" when it was
  built from a modified working tree — in the top-right of its window. That is the only
  claim here that cannot be wrong: everything above describes what is *installed*, while
  the stamp describes what is *running*.

  If the stamp does not match what you just built:

    1. Quit Logic. It caches Audio Unit scans and will not re-scan while open.
    2. Re-run scripts/install-plugin.sh (it kills the extension host, which is the step
       that matters).
    3. Reopen Logic. If it still disagrees, open Plug-in Manager and Reset & Rescan.

  Repeated `git rev-parse --short=7 HEAD` here for comparison:
NOTE

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
  HEAD_SHA="$(git -C "$REPO_ROOT" rev-parse --short=7 HEAD)"
  git -C "$REPO_ROOT" diff --quiet HEAD -- 2>/dev/null || HEAD_SHA="${HEAD_SHA}+"
  echo "     this checkout is at $HEAD_SHA"
fi
