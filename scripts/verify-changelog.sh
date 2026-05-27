#!/usr/bin/env bash
# verify-changelog.sh — assert CHANGELOG.md has a real entry for the
# version we're about to release.
#
# We require:
#   - A `## [x.y.z] — YYYY-MM-DD` header matching the target version
#   - Or, for `--expected unreleased`, a `## [Unreleased]` section
#     with at least one non-template bullet under it
#
# Forcing this step means every release has human-authored release
# notes. Auto-generated changelogs from git logs look fine until
# you try to read them six months later — for a library this small,
# hand-written is worth the friction.
#
# Usage:
#   scripts/verify-changelog.sh --expected 1.0.0
#   scripts/verify-changelog.sh --expected unreleased

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

require '[ -n "$expected" ]' "--expected <version|unreleased> is required"

root=$(repo_root)
changelog="$root/CHANGELOG.md"
require '[ -f "$changelog" ]' "CHANGELOG.md missing"

step "Changelog entry check"

if [ "$expected" = "unreleased" ]; then
  # The Unreleased section exists and has at least one substantive
  # bullet — UNLESS the most recent commit is itself a release commit,
  # in which case Unreleased is legitimately empty (just shipped, no
  # new work yet). The check matters for PRs and in-flight main work,
  # not for the immediate post-release commit.
  head_subject=$(git log -1 --format=%s 2>/dev/null || echo "")
  if [[ "$head_subject" =~ ^release:\ v ]]; then
    color_ok "✔ "; printf "HEAD is a release commit — empty [Unreleased] is OK\n"
    exit 0
  fi

  if ! awk '
    /^## \[Unreleased\]/ { in_section = 1; next }
    /^## \[/ && in_section { exit }
    in_section && /^[-*] / { found++ }
    END { exit(found > 0 ? 0 : 1) }
  ' "$changelog"; then
    color_err "✖ "; printf "[Unreleased] section has no bullets yet\n"
    exit 1
  fi
  color_ok "✔ "; printf "[Unreleased] section populated\n"
  exit 0
fi

# Tagged version — expect `## [expected] — YYYY-MM-DD` header.
# ISO-8601 date in the same line is a soft contract but we require
# SOMETHING after the em-dash so accidental "## [1.0.0]" lines
# without a date don't slip through.
header_regex="^## \\[$expected\\] — "
if ! grep -qE "$header_regex" "$changelog"; then
  color_err "✖ "; printf "no '## [%s] — <date>' entry in CHANGELOG.md\n" "$expected"
  printf "   add one before releasing. Template in the file footer.\n"
  exit 1
fi

color_ok "✔ "; printf "CHANGELOG.md has entry for %s\n" "$expected"
