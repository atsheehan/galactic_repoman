#!/usr/bin/env bash
# Render the plans/ markdown documents to styled HTML under target/plans/.
#
#   scripts/render-plans.sh                     # render every plans/**/*.md
#   scripts/render-plans.sh <file.md> [...]     # render just these
#   OPEN=1 scripts/render-plans.sh <file.md>    # ...and open the first one
#
# The renderer is tools/plan-render, a standalone crate — it is deliberately not a
# member of the game crate, so `cargo build` at the repo root never pulls it in.

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
renderer="$repo/tools/plan-render"
bin="$renderer/target/release/plan-render"

# Rebuild only when a source file is newer than the binary.
if [[ ! -x "$bin" ]] || [[ -n "$(find "$renderer/src" "$renderer/assets" "$renderer/Cargo.toml" -newer "$bin" 2>/dev/null)" ]]; then
  echo "building plan-render..." >&2
  cargo build --release --manifest-path "$renderer/Cargo.toml"
fi

if [[ $# -gt 0 ]]; then
  inputs=("$@")
else
  mapfile -t inputs < <(find "$repo/plans" -name '*.md' | sort)
fi

if [[ ${#inputs[@]} -eq 0 ]]; then
  echo "no markdown files to render" >&2
  exit 1
fi

outputs=()
for input in "${inputs[@]}"; do
  outputs+=("$("$bin" "$input")")
done

printf 'rendered %d file(s) to %s\n' "${#outputs[@]}" "$repo/target/plans" >&2

if [[ "${OPEN:-}" == "1" ]]; then
  xdg-open "${outputs[0]}" >/dev/null 2>&1 &
fi
