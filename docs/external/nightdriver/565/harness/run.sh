#!/bin/sh
# Usage: run.sh <fork-checkout-with-include/heapbudget.h>   (default: ~/workspace/external/nd565-fix)
# Logic-level test against a MODEL of ESP-IDF heap semantics. Not a device run.
set -e
H=$(cd "$(dirname "$0")" && pwd)
FIX=${1:-$HOME/workspace/external/nd565-fix}
OUT=$(mktemp)
g++ -std=gnu++17 -O1 -Wall -I"$H/stubs" -I"$FIX/include" "$H/host/test_budget.cpp" -o "$OUT"
"$OUT"; rc=$?; rm -f "$OUT"; exit $rc
