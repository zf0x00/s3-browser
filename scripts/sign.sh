#!/usr/bin/env bash
# Sign a binary or .app so the macOS Keychain sees every rebuild as the same app.
#
# Identity, in order:
#   1. $S3B_SIGN_IDENTITY (a certificate name or SHA-1 hash), if set;
#   2. the first "Apple Development" certificate in your keychain
#      (free, created by Xcode → Settings → Accounts → Manage Certificates);
#   3. ad-hoc with a fixed designated requirement (fallback: macOS still asks
#      once per rebuild, because ad-hoc builds can't carry a team ID).
set -euo pipefail
ID="dev.s3browser.app"
TARGET="$1"

IDENTITY="${S3B_SIGN_IDENTITY:-}"
# "adhoc" forces anonymous signing (public builds: no certificate name inside).
if [ "$IDENTITY" = "adhoc" ] || [ "$IDENTITY" = "-" ]; then
  IDENTITY=""
elif [ -z "$IDENTITY" ]; then
  IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Apple Development:/ { print $2; exit }')"
fi

if [ -n "$IDENTITY" ]; then
  # A real certificate: codesign's default requirement is
  # identifier + Apple anchor + your team ID — stable across rebuilds.
  codesign --force --sign "$IDENTITY" --identifier "$ID" --timestamp=none "$TARGET" >/dev/null 2>&1
  echo "Signed with: $IDENTITY" >&2
else
  codesign --force --sign - --identifier "$ID" \
    --requirements "=designated => identifier \"$ID\"" "$TARGET" >/dev/null 2>&1
fi
