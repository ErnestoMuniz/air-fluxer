#!/bin/sh
# Builds the macOS app bundle and wraps it in a compressed DMG for releases.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

scripts/macos-bundle.sh
DMG="$ROOT/target/release/AirFluxer.dmg"
rm -f "$DMG"
hdiutil create -volname "AirFluxer" -srcfolder "$ROOT/target/release/AirFluxer.app" \
    -ov -format UDZO "$DMG" >/dev/null
echo "built $DMG"
