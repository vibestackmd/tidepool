#!/usr/bin/env bash
# Common helpers for the release scripts. Sourced, not executed.
#
# All release scripts run under `set -euo pipefail`, so helpers here
# are written to fail loudly — unexpected git state, missing tools,
# manifest drift — rather than silently returning empty strings.

set -euo pipefail

# Absolute path to the repo root, regardless of where the caller cd'd.
repo_root() {
  git rev-parse --show-toplevel
}

# Read the single-source workspace version from Cargo.toml. Bails if
# the key is missing or malformed.
#
# Implemented via Python (tomllib is stdlib from 3.11+) because macOS
# BSD awk doesn't support `match()` capture arrays and we want the
# scripts to work identically on laptops and Linux CI.
workspace_version() {
  local root
  root=$(repo_root)
  python3 - "$root/Cargo.toml" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as f:
    data = tomllib.load(f)
print(data["workspace"]["package"]["version"])
PY
}

# Read the npm package version for crates/node/package.json.
npm_version() {
  local root
  root=$(repo_root)
  node -p "require('$root/crates/node/package.json').version"
}

# Stdout color helpers — safe to use in CI (they only emit when
# stdout is a TTY).
color_ok()   { if [ -t 1 ]; then printf "\033[32m%s\033[0m" "$*"; else printf "%s" "$*"; fi }
color_err()  { if [ -t 1 ]; then printf "\033[31m%s\033[0m" "$*"; else printf "%s" "$*"; fi }
color_warn() { if [ -t 1 ]; then printf "\033[33m%s\033[0m" "$*"; else printf "%s" "$*"; fi }

# Print a structured header before each preflight step so the log
# reads clearly in CI.
step() {
  printf "\n── %s ──\n" "$*"
}

# Assert a shell invariant; exit 1 with a red message on failure.
# Intentionally minimal — full structured error reporting would be
# nicer but is out of scope for shell glue.
require() {
  local cond="$1"
  local msg="$2"
  if ! eval "$cond"; then
    color_err "✖ "
    printf "%s\n" "$msg"
    exit 1
  fi
}
