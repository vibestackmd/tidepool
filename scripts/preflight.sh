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
# Heuristic: find the *prior* `release: v*` commit (not the one we're
# currently cutting) and assert compatibility.toml was modified
# between then and HEAD. Forces a "yes we re-verified" decision per
# release.
#
# If the most recent release commit's subject matches the release
# we're cutting, skip it and look for the one before. Without this,
# the diff is trivially empty (the current release commit is being
# diffed against itself or against descendants that don't touch
# compatibility.toml).
expected_subject="release: v$release"
most_recent_subject=$(git log --grep='^release: v' -n1 --format=%s 2>/dev/null || true)
if [ "$most_recent_subject" = "$expected_subject" ]; then
  last_release_commit=$(git log --grep='^release: v' --format=%H --skip=1 -n1 2>/dev/null || true)
else
  last_release_commit=$(git log --grep='^release: v' --format=%H -n1 2>/dev/null || true)
fi
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
# Soft locally, hard in CI: the workflow's `git verify-tag` step
# refuses unsigned tag pushes, so an unsigned release can't ship
# through the OIDC pipeline even if this warns through.
if ! git config --get user.signingkey >/dev/null; then
  color_warn "⚠ "; printf "no git signing key configured — local publish OK, CI tag-push will refuse unsigned tags\n"
  printf "   configure with: git config --global user.signingkey <keyid>\n"
else
  color_ok "✔ "; printf "signing key configured\n"
fi

step "cargo publish --dry-run (dep order)"
# Dep order: core → service → server → cli → node. Every crate must
# pass `--dry-run` individually — catches missing README/license/keyword
# errors that only surface at publish time otherwise.
#
# First-release exception: dry-run for downstream crates resolves
# workspace-sibling deps via crates.io, not via the local path —
# so on a true first publish (no crate exists on crates.io yet) the
# dependent dry-runs fail with "no matching package named tidepool-*".
# In that case we only dry-run the leaf (`tidepool-core`).
if curl -sfH "User-Agent: tidepool-preflight" "https://crates.io/api/v1/crates/tidepool-core" >/dev/null 2>&1; then
  for crate in tidepool-core tidepool-rpc tidepool-server tidepool-cli tidepool-node; do
    printf "  %s…\n" "$crate"
    cargo publish -p "$crate" --dry-run --allow-dirty 2>&1 | tail -3
  done
else
  color_warn "⚠ "; printf "tidepool-core not yet on crates.io — first release; only dry-running the leaf\n"
  cargo publish -p tidepool-core --dry-run --allow-dirty 2>&1 | tail -3
fi

step "npm publish --dry-run"
(cd crates/node && pnpm publish --dry-run --access public --no-git-checks 2>&1 | tail -6) || {
  color_err "✖ "; printf "npm dry-run failed\n"
  exit 1
}

color_ok "✔ "; printf "preflight for %s complete — ready to tag + publish\n" "$release"
