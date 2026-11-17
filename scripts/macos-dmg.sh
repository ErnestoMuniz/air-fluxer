#!/bin/sh
# Builds the macOS app bundle and wraps it in a compressed DMG for releases.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

scripts/macos-bundle.sh
DMG="$ROOT/target/release/Fluxer.dmg"
rm -f "$DMG"
hdiutil create -volname "Fluxer" -srcfolder "$ROOT/target/release/Fluxer.app" \
    -ov -format UDZO "$DMG" >/dev/null
echo "built $DMG"
