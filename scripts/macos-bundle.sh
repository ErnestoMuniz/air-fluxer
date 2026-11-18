#!/bin/sh
# Builds the macOS app bundle: target/release/AirFluxer.app.
#
# The bundle is ad-hoc signed (or signed with $CODESIGN_IDENTITY), which is enough to run it on
# this Mac and to keep the microphone permission across launches. Shipping it to other people
# also needs a Developer ID signature and notarisation.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

APP="$ROOT/target/release/AirFluxer.app"
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' app/Cargo.toml | head -n 1)

# UNIVERSAL=1 builds both Apple architectures and joins them with lipo (needs
# `rustup target add x86_64-apple-darwin` and an Intel libwebrtc build the first time).
if [ "${UNIVERSAL:-0}" = "1" ]; then
    cargo build --release -p fluxer-app --target aarch64-apple-darwin
    cargo build --release -p fluxer-app --target x86_64-apple-darwin
    mkdir -p target/release
    lipo -create target/aarch64-apple-darwin/release/fluxer-app target/x86_64-apple-darwin/release/fluxer-app \
        -output target/release/fluxer-app
else
    cargo build --release -p fluxer-app
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/fluxer-app "$APP/Contents/MacOS/fluxer-app"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# The icon: AppKit reads the SVG (macOS 13+), so render it once at 1024 and cut every size the
# iconset wants. This needs the Swift toolchain that comes with the command line tools.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/svg2png.swift" <<'SWIFT'
import AppKit

let args = CommandLine.arguments
guard args.count == 4, let image = NSImage(contentsOfFile: args[1]), let size = Int(args[2]) else {
    FileHandle.standardError.write("svg2png: cannot read the SVG\n".data(using: .utf8)!)
    exit(1)
}
guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                                 bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                 colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { exit(1) }
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor.clear.setFill()
NSRect(x: 0, y: 0, width: size, height: size).fill()
// The macOS tile: a rounded square in the brand colour with the waves (the same-colour circle
// in the SVG disappears into it) inset so the glyph keeps Apple's margins.
let d = CGFloat(size)
let tile = NSRect(x: d * 0.04, y: d * 0.04, width: d * 0.92, height: d * 0.92)
NSColor(srgbRed: 0x46 / 255.0, green: 0x41 / 255.0, blue: 0xD9 / 255.0, alpha: 1).setFill()
NSBezierPath(roundedRect: tile, xRadius: d * 0.185, yRadius: d * 0.185).fill()
let inner = d * 0.68
image.draw(in: NSRect(x: (d - inner) / 2, y: (d - inner) / 2, width: inner, height: inner))
NSGraphicsContext.restoreGraphicsState()
guard let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
try! png.write(to: URL(fileURLWithPath: args[3]))
SWIFT
swift "$TMP/svg2png.swift" app/assets/logo.svg 1024 "$TMP/icon-1024.png"

ICONSET="$TMP/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$TMP/icon-1024.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$TMP/icon-1024.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

IDENTITY=${CODESIGN_IDENTITY:-}
if [ -z "$IDENTITY" ] && security find-certificate -c "Fluxer Dev" >/dev/null 2>&1; then
    IDENTITY="Fluxer Dev"
fi

codesign --force --options runtime \
    --entitlements packaging/macos/Fluxer.entitlements \
    --sign "${IDENTITY:--}" \
    "$APP"
echo "built $APP"
