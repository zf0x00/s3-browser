#!/usr/bin/env bash
# Cargo runner (see .cargo/config.toml): sign, then run. Makes `cargo run`
# share the bundle's Keychain access.
set -euo pipefail
BIN="$1"; shift
case "$(basename "$BIN")" in
  s3-browser) "$(dirname "$0")/sign.sh" "$BIN" ;;
esac
exec "$BIN" "$@"
