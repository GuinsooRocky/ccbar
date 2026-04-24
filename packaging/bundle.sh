#!/usr/bin/env bash
# Build ccbar.app at the repo root from whichever profile you pass (debug|release).
# Usage: ./packaging/bundle.sh [debug|release]
set -euo pipefail

PROFILE="${1:-debug}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/$PROFILE/ccbar"
APP="$ROOT/ccbar.app"

if [[ ! -x "$BIN" ]]; then
    echo "Binary not found at $BIN — run 'cargo build${PROFILE/debug/}' first." >&2
    exit 1
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/ccbar"
cp "$ROOT/packaging/Info.plist" "$APP/Contents/Info.plist"

# Ad-hoc code signature so macOS Gatekeeper won't quarantine the launcher.
codesign --force --sign - "$APP" >/dev/null 2>&1 || true

echo "built: $APP (profile=$PROFILE, $(du -sh "$APP" | cut -f1))"
