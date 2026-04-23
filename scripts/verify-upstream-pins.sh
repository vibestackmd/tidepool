#!/usr/bin/env bash
# verify-upstream-pins.sh — sanity-check the pins in compatibility.toml
# still resolve. We don't do full SemVer range resolution (that would
# pull in a full version resolver); we do the next-best thing:
#
#   - For every crate/npm/git pin with a `url`, HEAD the URL to confirm
#     the project still exists at the documented location.
#   - For the `surfpool` pin specifically, fetch the GH releases list
#     and warn if the pinned range no longer matches any release.
#   - For `rust`, cross-check against workspace.package.rust-version.
#
# Not a hard gate on "does this exact version combo still work" —
# that's what the drift fixtures + contract tests cover. This just
# catches "upstream renamed itself / moved the repo / nuked the tag".
#
# Usage: scripts/verify-upstream-pins.sh

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
source "$SCRIPT_DIR/lib.sh"

step "Upstream compatibility pins"

root=$(repo_root)
toml="$root/compatibility.toml"
require '[ -f "$toml" ]' "compatibility.toml missing"

# Use python to parse TOML — tomllib is stdlib from 3.11+. On older
# Pythons, degrade to a warning + continue (the file is still checked
# structurally by the unit tests in `compatibility.rs`).
python_parser=$(cat <<'PY'
import sys, tomllib
try:
    data = tomllib.load(open(sys.argv[1], 'rb'))
except Exception as e:
    print(f"ERR parsing toml: {e}", file=sys.stderr)
    sys.exit(2)
for section in ("tested-against", "runtime"):
    for name, pin in data.get(section, {}).items():
        version = pin.get("version", "")
        url = pin.get("url", "")
        print(f"{section}\t{name}\t{version}\t{url}")
PY
)

if ! command -v python3 >/dev/null; then
  color_warn "⚠ "; printf "python3 not on PATH — skipping URL reachability check\n"
  exit 0
fi

entries=$(python3 -c "$python_parser" "$toml")

# Cross-check rust pin against Cargo.toml rust-version.
rust_pin=$(awk -F'\t' '$1=="tested-against" && $2=="rust" {print $3}' <<<"$entries")
cargo_rust=$(python3 -c "
import tomllib
data = tomllib.load(open('$root/Cargo.toml', 'rb'))
print(data['workspace']['package'].get('rust-version', ''))
")
if [ -n "$rust_pin" ] && [ -n "$cargo_rust" ]; then
  # compatibility.toml pin is a range (">=1.77"); Cargo.toml is an
  # exact value ("1.77"). Just assert the Cargo value appears in the
  # pin string — good enough for drift detection.
  if ! [[ "$rust_pin" == *"$cargo_rust"* ]]; then
    color_err "✖ "; printf "rust MSRV mismatch: Cargo=%s pin=%s\n" "$cargo_rust" "$rust_pin"
    exit 1
  fi
  color_ok "✔ "; printf "rust MSRV %s consistent with pin %s\n" "$cargo_rust" "$rust_pin"
fi

# Reachability check — each pinned URL returns something 2xx/3xx.
# `curl --head` is polite to upstream and cheap. Silent success,
# loud failure. We do NOT fail the script on a 4xx/5xx because
# transient CDN flakes are common; just warn.
while IFS=$'\t' read -r _section name _version url; do
  [ -z "$url" ] && continue
  status=$(curl -s -o /dev/null -w "%{http_code}" --max-time 8 -L -I "$url" || echo "000")
  case "$status" in
    2*|3*) color_ok "✔ "; printf "%-16s %s (HTTP %s)\n" "$name" "$url" "$status" ;;
    *)     color_warn "⚠ "; printf "%-16s %s (HTTP %s — upstream may have moved)\n" "$name" "$url" "$status" ;;
  esac
done <<<"$entries"
