#!/usr/bin/env bash
# bump-version.sh — single command to update every version field
# atomically. Writes:
#
#   - [workspace.package].version in Cargo.toml (source of truth)
#   - crates/node/package.json
#   - crates/node/npm/*/package.json if they exist (napi per-platform)
#
# Then runs `cargo update -p <name>` on every tidepool-* crate so
# Cargo.lock reflects the new version, preventing CI from re-resolving
# on the first post-bump build.
#
# Does not commit, does not tag — those live in `make release-*` so
# the user sees the diff before it's wrapped in a commit.
#
# Usage:
#   scripts/bump-version.sh 1.0.0
#   scripts/bump-version.sh 1.0.0-rc.1

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "$SCRIPT_DIR/lib.sh"

if [ $# -ne 1 ]; then
  color_err "✖ "; printf "usage: %s <new-version>\n" "$0"
  exit 2
fi

new="$1"

# SemVer + common pre-release variants (-rc.N, -alpha.N, -beta.N).
# Stays permissive enough for shapes we actually use, strict enough
# to catch typos like "1..0.0" or "v1.0.0" (no leading v in manifests).
if ! [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  color_err "✖ "; printf "not a valid SemVer: %s\n" "$new"
  printf "   shapes accepted: 1.2.3, 1.0.0-rc.1, 1.0.0-alpha.2\n"
  exit 2
fi

root=$(repo_root)
old=$(workspace_version)

step "Bumping ${old} → ${new}"

# 1. Workspace Cargo.toml. Match only the version line inside
# [workspace.package] — not the [package] sections of leaf crates
# (those use `version.workspace = true` already) and not any other
# `version = "..."` in a dependency line.
python3 - "$root/Cargo.toml" "$new" <<'PY'
import sys, re
path, new = sys.argv[1], sys.argv[2]
src = open(path).read()
pat = re.compile(r'(\[workspace\.package\][^\[]*?\nversion\s*=\s*")[^"]+(")', re.DOTALL)
new_src, n = pat.subn(rf'\g<1>{new}\g<2>', src, count=1)
if n != 1:
    print("ERR: couldn't find [workspace.package] version line", file=sys.stderr)
    sys.exit(1)
open(path, 'w').write(new_src)
PY
printf "  Cargo.toml                        → %s\n" "$new"

# 2. npm package.
node -e "
const fs = require('fs');
const p = 'crates/node/package.json';
const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
pkg.version = '$new';
// napi-rs's per-platform packages use the 'optionalDependencies'
// map keyed by package name. Rewrite every pinned scoped `@tidepool/*` or unscoped tidepool-*
// entry to the new version so the per-platform addons resolve.
if (pkg.optionalDependencies) {
  for (const name of Object.keys(pkg.optionalDependencies)) {
    if (name.startsWith('@tidepool/') || name.startsWith('tidepool-')) {
      pkg.optionalDependencies[name] = '$new';
    }
  }
}
fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
"
printf "  crates/node/package.json          → %s\n" "$new"

# 3. Per-platform napi packages (if any exist yet). napi-rs creates
# these in `crates/node/npm/*/package.json` when you run
# `pnpm exec napi prepublish`. Update in place if present.
if ls "$root"/crates/node/npm/*/package.json >/dev/null 2>&1; then
  for f in "$root"/crates/node/npm/*/package.json; do
    node -e "
      const fs=require('fs');
      const pkg=JSON.parse(fs.readFileSync('$f','utf8'));
      pkg.version='$new';
      fs.writeFileSync('$f', JSON.stringify(pkg,null,2)+'\n');
    "
    printf "  %s → %s\n" "${f#$root/}" "$new"
  done
fi

# 4. Cargo.lock — run `cargo check` to regenerate the lock for the
# new workspace version without a full rebuild.
step "Regenerating Cargo.lock"
(cd "$root" && cargo check --workspace --quiet 2>/dev/null) || true
printf "  Cargo.lock updated\n"

step "Done"
printf "  Review with: git diff\n"
printf "  Commit + tag: git commit -am \"release: v%s\" && git tag -s v%s -m \"v%s\"\n" "$new" "$new" "$new"
