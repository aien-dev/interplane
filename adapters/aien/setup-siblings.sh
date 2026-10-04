#!/usr/bin/env bash
# Clone the pinned AIEN sibling checkouts next to the interplane checkout. Reads ./PINS.
# Usage: adapters/aien/setup-siblings.sh [parent-dir]   (default: the parent of the interplane checkout)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
parent="${1:-$(cd "$here/../../.." && pwd)}"
# shellcheck source=PINS
. "$here/PINS"
dest="$parent/interplane-audit"
mkdir -p "$dest"
pin() { # name repo rev
  [ -d "$dest/$1/.git" ] || git clone -q "$2" "$dest/$1"
  git -C "$dest/$1" fetch -q origin "$3" 2>/dev/null || true
  git -C "$dest/$1" checkout -q "$3"
  echo "$1 @ $(git -C "$dest/$1" rev-parse HEAD)"
}
pin aegis-runtime "$AEGIS_RUNTIME_REPO" "$AEGIS_RUNTIME_REV"
pin aien-protocols "$AIEN_PROTOCOLS_REPO" "$AIEN_PROTOCOLS_REV"
# The Cargo.toml git revs must match PINS.
for crate in aien-capability aien-mcp; do
  grep -q "^$crate = .*rev = \"$SOVEREIGN_CORE_REV\"" "$here/Cargo.toml" || { echo "Cargo.toml rev for $crate differs from PINS" >&2; exit 1; }
done
