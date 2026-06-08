#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Rusty"
BUNDLE_ID="com.rusty.terminal"
VERSION="0.1.4"
BINARY="target/release/rusty"
ICON="crates/rusty-ui/assets/icon.icns"
OUT="releases/$APP_NAME.app"

echo "Building release binary..."
cargo build --release -p rusty-app

echo "Assembling $OUT..."
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS"
mkdir -p "$OUT/Contents/Resources"

cp "$BINARY" "$OUT/Contents/MacOS/$APP_NAME"
cp "$ICON"   "$OUT/Contents/Resources/$APP_NAME.icns"

# Completion specs — copied into Resources so the bundled binary can find them.
mkdir -p "$OUT/Contents/Resources/completions"
cp completions-toml/*.toml "$OUT/Contents/Resources/completions/"

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundleIconFile</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>LSUIElement</key>
    <false/>
</dict>
</plist>
PLIST

echo "Done: $OUT"
echo ""
echo "To install:  cp -r $OUT /Applications/"
echo "To run now:  open $OUT"
