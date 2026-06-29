#!/usr/bin/env bash
# Download steamcmd directly from Valve's official CDN into ./tools/steamcmd/.
# Used by both CI and the local testing flow so the binary always comes from
# Valve, never a third party. Idempotent — re-running is a no-op once present.
#
# Note: Valve publishes no SHA256/GPG checksum for this tarball, and steamcmd is
# a self-updating bootstrapper, so trust is HTTPS-to-Valve's-CDN (see
# ci/steampipe/README.md / the plan's integrity note).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
steamcmd_dir="$repo_root/tools/steamcmd"
url="https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz"

if [[ -x "$steamcmd_dir/steamcmd.sh" ]]; then
	echo "steamcmd already present at $steamcmd_dir"
	exit 0
fi

echo "Downloading steamcmd from Valve CDN..."
mkdir -p "$steamcmd_dir"
curl -sqL "$url" | tar -xzf - -C "$steamcmd_dir"

echo "steamcmd installed at $steamcmd_dir"
