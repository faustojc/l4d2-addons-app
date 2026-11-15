#!/bin/sh
# Builds "L4D2 Addons Manager.app" (universal: Apple Silicon + Intel) and a .dmg. Run on macOS from the repo root:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   sh packaging/macos/bundle.sh 0.1.0
set -eu
VERSION="${1:-0.1.0}"
NAME="L4D2 Addons Manager"
BIN=l4d2-addons-manager

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

APP="dist/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/$BIN" \
    "target/aarch64-apple-darwin/release/$BIN" \
    "target/x86_64-apple-darwin/release/$BIN"
cp assets/icon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleDisplayName</key><string>$NAME</string>
    <key>CFBundleIdentifier</key><string>io.github.faustojc.l4d2-addons-manager</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>$BIN</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
# Ad-hoc signature: required to run on Apple Silicon. Not notarized, so Gatekeeper still asks on first launch.
codesign --force --deep --sign - "$APP"

STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO \
    "dist/L4D2-Addons-Manager-$VERSION-macos-universal.dmg"
rm -rf "$STAGE"
