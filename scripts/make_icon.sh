#!/usr/bin/env bash
# Regenerate assets/icon/AppIcon.icns from assets/icon/AppIcon.svg.
set -euo pipefail
cd "$(dirname "$0")/.."
SET="$(mktemp -d)/AppIcon.iconset"
cargo run -q --example render_icon -- assets/icon/AppIcon.svg "$SET"
iconutil -c icns "$SET" -o assets/icon/AppIcon.icns
cp "$SET/icon_512x512@2x.png" assets/icon/AppIcon.png
echo "Wrote assets/icon/AppIcon.icns"
