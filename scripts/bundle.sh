#!/usr/bin/env bash
# Build "S3 Browser.app" from the cargo binary.
#   scripts/bundle.sh            # release build
#   scripts/bundle.sh debug      # debug build (faster to compile)
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${1:-release}"
if [ "$PROFILE" = "release" ]; then cargo build --release; else cargo build; fi
APP="target/S3 Browser.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$PROFILE/s3-browser" "$APP/Contents/MacOS/s3-browser"
cp assets/icon/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>S3 Browser</string>
  <key>CFBundleDisplayName</key><string>S3 Browser</string>
  <key>CFBundleIdentifier</key><string>dev.s3browser.app</string>
  <key>CFBundleExecutable</key><string>s3-browser</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# A fixed designated requirement (identifier, not the binary's hash) keeps the
# Keychain's "Always Allow" valid across rebuilds — otherwise every build is a
# "new app" and macOS asks for the saved keys again.
scripts/sign.sh "$APP"
echo "Built $APP"
