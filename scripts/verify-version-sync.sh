#!/usr/bin/env bash
# verify-version-sync.sh — assert every publishable unit reports the
# same version. Run locally and in release preflight; blocks a
# publish when anything has drifted.
#
# Source-of-truth order:
#   1. [workspace.package].version in Cargo.toml
#   2. crates/node/package.json version
#
# Every tidepool-* crate inherits (1) via `version.workspace = true`,
# so the only real mismatch risk is between (1) and (2). This script
# enforces they match before any publish runs.
#
# Usage:
#   scripts/verify-version-sync.sh                # assert sync
#   scripts/verify-version-sync.sh --expected V   # also assert == V

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "$SCRIPT_DIR/lib.sh"

expected=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --expected) expected="$2"; shift 2 ;;
    *) color_err "✖ "; echo "unknown flag: $1"; exit 2 ;;
  esac
done

step "Version sync check"

ws=$(workspace_version)
npm=$(npm_version)

printf "  workspace (Cargo.toml):       %s\n" "$ws"
printf "  npm (crates/node/package.json): %s\n" "$npm"

require '[ -n "$ws" ]'  "workspace version not readable"
require '[ -n "$npm" ]' "npm version not readable"
require '[ "$ws" = "$npm" ]' "workspace ($ws) != npm ($npm) — run scripts/bump-version.sh to re-sync"

# Leaf crates inherit via workspace; double-check nobody reintroduced
# a hard-coded `version = "..."` line in a leaf Cargo.toml.
root=$(repo_root)
offenders=$(grep -lE '^version\s*=\s*"[0-9]' "$root"/crates/*/Cargo.toml || true)
require '[ -z "$offenders" ]' "these crates override workspace version: $offenders"

if [ -n "$expected" ]; then
  require '[ "$ws" = "$expected" ]' "expected $expected, got workspace=$ws"
fi

color_ok "✔ "; printf "versions in sync at %s\n" "$ws"
