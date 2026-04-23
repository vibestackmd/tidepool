#!/usr/bin/env bash
# preflight.sh — every check that must pass before we publish.
# Identical flow in CI and locally (`make release-dry-run`); keeping
# the logic in one place prevents "works on my laptop" drift.
#
# Exit codes:
#   0  — all checks pass, safe to publish
#   1  — a check failed
#   2  — misuse (bad flags, missing tool)
#
# Usage:
#   scripts/preflight.sh                      # check Unreleased state
#   scripts/preflight.sh --release 1.0.0      # full check for tag v1.0.0

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "$SCRIPT_DIR/lib.sh"

release=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) release="$2"; shift 2 ;;
    -h|--help)
      echo "usage: $0 [--release <version>]"
      echo "  Without --release, runs the always-green checks."
      echo "  With --release, also enforces version-sync, changelog, and"
      echo "  crates-publish dry-runs for that version."
      exit 0 ;;
    *) color_err "✖ "; echo "unknown flag: $1"; exit 2 ;;
  esac
done

root=$(repo_root)
cd "$root"

# ──────────────────────────────────────────────────────────────────
# Always-green gates
# ──────────────────────────────────────────────────────────────────

step "Working tree clean"
if [ -n "$(git status --porcelain)" ]; then
  color_err "✖ "; printf "tree has uncommitted changes\n"
  git status --short
  exit 1
fi
color_ok "✔ "; printf "tree clean\n"

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy (workspace, -D warnings)"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace"
cargo test --workspace --locked

step "Contract tests (offline drift replay)"
cargo test --locked -p tidepool-rpc --test contracts

step "xtask check-drift (fixtures + schemas consistent with commit)"
cargo xtask check-drift

step "Upstream pins reachable"
bash "$SCRIPT_DIR/verify-upstream-pins.sh"

# ──────────────────────────────────────────────────────────────────
# Release-only gates
# ──────────────────────────────────────────────────────────────────

if [ -z "$release" ]; then
  step "Unreleased changelog entry"
  bash "$SCRIPT_DIR/verify-changelog.sh" --expected unreleased
  color_ok "✔ "; printf "preflight (no release) complete\n"
  exit 0
fi

step "Version sync"
bash "$SCRIPT_DIR/verify-version-sync.sh" --expected "$release"

step "Changelog entry for $release"
bash "$SCRIPT_DIR/verify-changelog.sh" --expected "$release"

step "compatibility.toml touched since last release"
# Heuristic: find the most recent `release: v*` commit and assert
# compatibility.toml was modified since then. Forces a
# "yes we re-verified" decision per release.
last_release_commit=$(git log --grep='^release: v' --format=%H -n1 || true)
if [ -n "$last_release_commit" ]; then
  if ! git diff --quiet "$last_release_commit"..HEAD -- compatibility.toml; then
    color_ok "✔ "; printf "compatibility.toml modified since %s\n" "${last_release_commit:0:7}"
  else
    color_err "✖ "; printf "compatibility.toml unchanged since last release — re-verify upstream pins or document why no change\n"
    printf "   to bypass (rare): git commit --allow-empty with 'compat-review: no change'\n"
    exit 1
  fi
else
  color_warn "⚠ "; printf "no prior 'release: v*' commit found — skipping diff check\n"
fi

step "Tag signability"
if ! git config --get user.signingkey >/dev/null; then
  color_err "✖ "; printf "no git signing key configured — release tags MUST be signed\n"
  printf "   configure with: git config --global user.signingkey <keyid>\n"
  exit 1
fi
color_ok "✔ "; printf "signing key configured\n"

step "cargo publish --dry-run (dep order)"
# Dep order: core → service → server → cli → node. Every crate must
# pass `--dry-run` individually — catches missing README/license/keyword
# errors that only surface at publish time otherwise.
for crate in tidepool-core tidepool-rpc tidepool-server tidepool-cli tidepool-node; do
  printf "  %s…\n" "$crate"
  cargo publish -p "$crate" --dry-run --allow-dirty 2>&1 | tail -3
done

step "npm publish --dry-run"
(cd crates/node && pnpm publish --dry-run --access public --no-git-checks 2>&1 | tail -6) || {
  color_err "✖ "; printf "npm dry-run failed\n"
  exit 1
}

color_ok "✔ "; printf "preflight for %s complete — ready to tag + publish\n" "$release"
