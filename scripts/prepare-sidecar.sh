#!/bin/bash
set -euo pipefail

# Builds hermesd and stages it where Tauri expects external binaries
# (apps/desktop/src-tauri/binaries/hermesd-<target-triple>), so the
# desktop bundle ships the daemon as a signed sidecar. Run before
# `pnpm tauri build` or `pnpm tauri dev`.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
EXT=""
case "$TRIPLE" in
  *-windows-*) EXT=".exe" ;;
esac

# hermesd recognises the owner's app by the Team ID compiled in (H-110).
if [ -z "${HERMES_TEAM_ID:-}" ]; then
  echo "prepare-sidecar: HERMES_TEAM_ID is not set; hermesd gets the placeholder and won't recognise a signed app" >&2
fi
cargo build --release -p hermesd --manifest-path "$ROOT/Cargo.toml"

DEST="$ROOT/apps/desktop/src-tauri/binaries"
mkdir -p "$DEST"
cp "$ROOT/target/release/hermesd$EXT" "$DEST/hermesd-$TRIPLE$EXT"
echo "staged $DEST/hermesd-$TRIPLE$EXT"
