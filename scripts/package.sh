#!/usr/bin/env bash
# Shared staging script for both the CI/CD release workflow and the local testing
# flow. Builds the release binary, stages the depot content root, and renders the
# committed VDF templates into ci/steampipe/build/ with real AppID/DepotID/SETLIVE.
#
# Config (APPID / DEPOTID / SETLIVE) comes from ci/steampipe/depot.env if present,
# else from the already-exported environment (CI supplies these from repo
# variables / the workflow input). SETLIVE defaults to "internal" — never the
# default/public branch real buyers get.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
steampipe_dir="$repo_root/ci/steampipe"
build_dir="$steampipe_dir/build"
depot_env="$steampipe_dir/depot.env"

if [[ -f "$depot_env" ]]; then
	# shellcheck disable=SC1090
	set -a; source "$depot_env"; set +a
fi

: "${APPID:?APPID is required (set ci/steampipe/depot.env or export it)}"
: "${DEPOTID:?DEPOTID is required (set ci/steampipe/depot.env or export it)}"
# Only an entirely unset SETLIVE defaults to internal; an explicitly empty
# value is preserved to mean "upload without promoting" (no colon in the
# expansion, so "" stays "").
SETLIVE="${SETLIVE-internal}"

if [[ "$SETLIVE" == "default" || "$SETLIVE" == "public" ]]; then
	echo "ERROR: refusing to target SETLIVE='$SETLIVE' — that is the branch real buyers get." >&2
	exit 1
fi

echo "Building release binary..."
cargo build --release --manifest-path "$repo_root/Cargo.toml"

content_dir="$build_dir/content"
output_dir="$build_dir/output"
echo "Resetting staging dirs..."
rm -rf "$content_dir" "$output_dir"
mkdir -p "$content_dir" "$output_dir"

echo "Staging depot content..."
install -m 0755 "$repo_root/target/release/galactic_repoman" "$content_dir/galactic_repoman"

echo "Rendering VDFs (APPID=$APPID DEPOTID=$DEPOTID SETLIVE=$SETLIVE)..."
render() {
	sed -e "s/__APPID__/$APPID/g" \
	    -e "s/__DEPOTID__/$DEPOTID/g" \
	    -e "s/__SETLIVE__/$SETLIVE/g" \
	    "$1" > "$2"
}
render "$steampipe_dir/app_build.vdf.template"   "$build_dir/app_build_${APPID}.vdf"
render "$steampipe_dir/depot_build.vdf.template" "$build_dir/depot_build_${DEPOTID}.vdf"

echo "Done."
echo "  content : $content_dir/galactic_repoman"
echo "  appbuild: $build_dir/app_build_${APPID}.vdf"
echo "  depot   : $build_dir/depot_build_${DEPOTID}.vdf"
