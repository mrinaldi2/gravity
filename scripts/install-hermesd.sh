#!/bin/bash
set -euo pipefail

# Installs hermesd from an unpacked release tarball. Thin wrapper around
# the daemon's own installer, which stages the binary under ~/.thehermes/bin,
# writes a default config (kept if present), and starts the launchd user
# agent — all user-domain, no sudo.

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$DIR/hermesd" service install
