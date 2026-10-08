#!/usr/bin/env bash
# One-time operator setup for the VAC M5 slice: give the judge account its own P-256 signing key
# and pin the matching public key where the operator keeps trust inputs.
#
# The private key is created by the judge account inside its own home (mode 700, file mode 600)
# and never leaves it. The proposing account (RSI_USER) cannot read it. The operator account can,
# through sudo; that is the operator's authority and is stated as a limit in every M5 receipt.
#
# Usage: JUDGE_BIN=<spark-rsi-judge> m5-setup-judge.sh <pinned-public-key-file>
# An existing key is kept (never overwritten); the pin file is rewritten from it.
set -euo pipefail
die() { echo "M5 SETUP FAIL: $*" >&2; exit 1; }
pin="${1:?usage: m5-setup-judge.sh <pinned-public-key-file>}"
: "${JUDGE_BIN:?JUDGE_BIN must name the spark-rsi-judge binary}"
judge_user="${JUDGE_USER:-aien-judge}"
rsi_user="${RSI_USER:-aien-rsi}"
judge_home="${JUDGE_HOME:-/var/lib/aien-judge}"
key="$judge_home/judge.key"
as_judge() { sudo -n -u "$judge_user" "$@"; }

[ "$(as_judge stat -c %a "$judge_home")" = 700 ] || die "$judge_home must be mode 700"
if ! as_judge test -f "$key"; then
  as_judge sh -c 'umask 077; head -c 32 /dev/urandom | od -An -tx1 | tr -d " \n" > "$1"' _ "$key"
fi
[ "$(as_judge stat -c %a "$key")" = 600 ] || die "$key must be mode 600"
# The proposer must not be able to read the key.
if sudo -n -u "$rsi_user" cat "$key" >/dev/null 2>&1; then die "$rsi_user can read $key"; fi
bin_copy="$(as_judge mktemp "$judge_home/judge-bin.XXXXXX")"
as_judge sh -c 'cat > "$1" && chmod 700 "$1"' _ "$bin_copy" <"$JUDGE_BIN"
mkdir -p "$(dirname "$pin")"
as_judge "$bin_copy" --signing-key-file "$key" --public-key >"$pin.new"
as_judge rm -f "$bin_copy"
grep -Eq '^04[0-9a-f]{128}$' "$pin.new" || die "unexpected public key output"
mv "$pin.new" "$pin"
echo "judge public key pinned at $pin"
