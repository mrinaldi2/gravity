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

# hermesd recognises the owner's Mac app by the Team ID compiled in (H-110).
# A release with the placeholder would never recognise the signed app, so
# anything but a real Team ID stops here unless this is an explicit dev
# build (HERMES_DEV_BUILD=1, as scripts/dev.sh sets). Windows recognises the
# app by its Program Files folder and needs no Team ID (ARCH-R38).
case "$TRIPLE" in
  *-apple-darwin)
    if [ -z "${HERMES_DEV_BUILD:-}" ] && ! [[ "${HERMES_TEAM_ID:-}" =~ ^[A-Z0-9]{10}$ && "$HERMES_TEAM_ID" != 0000000000 ]]; then
      echo "prepare-sidecar: HERMES_TEAM_ID must be the owner's 10-character Apple Team ID for a release build (got '${HERMES_TEAM_ID:-}'); set HERMES_DEV_BUILD=1 for a dev build" >&2
      exit 1
    fi
    ;;
esac
cargo build --release -p hermesd --manifest-path "$ROOT/Cargo.toml"

# The gate asserts on the binary itself, not the build log (H-114): a macOS
# release must say it is signed with this Team ID.
IDENTITY="$("$ROOT/target/release/hermesd$EXT" --version | sed -n 's/^identity: //p')"
echo "identity: $IDENTITY"
case "$TRIPLE" in
  *-apple-darwin)
    if [ -z "${HERMES_DEV_BUILD:-}" ] && [ "$IDENTITY" != "signed $HERMES_TEAM_ID" ]; then
      echo "prepare-sidecar: the built hermesd reports '$IDENTITY', not 'signed $HERMES_TEAM_ID'" >&2
      exit 1
    fi
    ;;
esac

DEST="$ROOT/apps/desktop/src-tauri/binaries"
mkdir -p "$DEST"
cp "$ROOT/target/release/hermesd$EXT" "$DEST/hermesd-$TRIPLE$EXT"
echo "staged $DEST/hermesd-$TRIPLE$EXT"
