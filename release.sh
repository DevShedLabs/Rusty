#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: ./release.sh <version>   (e.g. ./release.sh 0.2.0)"
    exit 1
fi

VERSION="$1"
APP_NAME="Rusty"
BUNDLE_ID="com.rusty.terminal"
BINARY="target/release/rusty"
ICON="crates/rusty-ui/assets/icon.icns"
APP_OUT="$APP_NAME.app"
ZIP_OUT="Rusty-macos-$VERSION.zip"

echo "==> Bumping version to $VERSION"
# Update the single workspace version — all crates inherit it.
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

echo "==> Building release binary"
cargo build --release -p rusty-app

echo "==> Assembling $APP_OUT"
rm -rf "$APP_OUT"
mkdir -p "$APP_OUT/Contents/MacOS"
mkdir -p "$APP_OUT/Contents/Resources"

cp "$BINARY" "$APP_OUT/Contents/MacOS/$APP_NAME"
cp "$ICON"   "$APP_OUT/Contents/Resources/$APP_NAME.icns"

cat > "$APP_OUT/Contents/Info.plist" <<PLIST
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

echo "==> Creating $ZIP_OUT"
rm -f "$ZIP_OUT"
zip -r "$ZIP_OUT" "$APP_OUT"

echo "==> Tagging git commit"
git add Cargo.toml Cargo.lock
git commit -m "release v$VERSION"
git tag "v$VERSION"

echo ""
echo "Done!"
echo ""
echo "  Bundle: $APP_OUT"
echo "  Zip:    $ZIP_OUT  ← upload this to GitHub Releases"
echo ""
echo "Push and publish:"
echo "  git push && git push --tags"
echo "  gh release create v$VERSION $ZIP_OUT --title \"v$VERSION\" --generate-notes"
