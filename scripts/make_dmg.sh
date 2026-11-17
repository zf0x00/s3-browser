#!/usr/bin/env bash
# Build a release "S3 Browser.app" and pack it into a drag-to-Applications DMG.
#   scripts/make_dmg.sh            # dist/S3 Browser <version>.dmg, signed with your certificate
#   scripts/make_dmg.sh --public   # dist/S3-Browser-<version>-arm64.dmg for publishing:
#                                  # ad-hoc signed, no personal paths or names inside
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [ "${1:-}" = "--public" ]; then
  export S3B_PUBLIC=1
  export S3B_SIGN_IDENTITY=adhoc   # the DMG too, not just the app
  APP="target/public/S3 Browser.app"
  DMG="dist/S3-Browser-$VERSION-arm64.dmg"
else
  APP="target/S3 Browser.app"
  DMG="dist/S3 Browser $VERSION.dmg"
fi
scripts/bundle.sh release

STAGE="$(mktemp -d)/S3 Browser"
mkdir -p "$STAGE" dist
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create -quiet -volname "S3 Browser $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG"
scripts/sign.sh "$DMG" 2>/dev/null || true
rm -rf "$(dirname "$STAGE")"
echo "Built $DMG ($(du -h "$DMG" | cut -f1))"
