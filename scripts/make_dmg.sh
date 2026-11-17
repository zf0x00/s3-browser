#!/usr/bin/env bash
# Build a release "S3 Browser.app" and pack it into dist/S3 Browser <version>.dmg
# (drag-to-Applications layout, compressed, signed with the same identity).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
scripts/bundle.sh release

APP="target/S3 Browser.app"
STAGE="$(mktemp -d)/S3 Browser"
mkdir -p "$STAGE" dist
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

DMG="dist/S3 Browser $VERSION.dmg"
rm -f "$DMG"
hdiutil create -quiet -volname "S3 Browser $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG"
scripts/sign.sh "$DMG" 2>/dev/null || true
rm -rf "$(dirname "$STAGE")"
echo "Built $DMG ($(du -h "$DMG" | cut -f1))"
