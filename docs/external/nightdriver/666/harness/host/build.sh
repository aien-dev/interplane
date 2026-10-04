#!/bin/sh
# Usage: build.sh <variant> <source-dir> <out> [extra g++ flags, e.g. -DENABLE_AUDIO=1]
# variant: modern | legacy
# source-dir: an UNMODIFIED NightDriverStrip tree at the SHA under test (needs include/ and src/socketserver.cpp)
# Platform headers are shadowed by stubs in an overlay copy of include/; socketserver.cpp is compiled from source-dir as-is.
set -e
H=$(cd "$(dirname "$0")/.." && pwd)
V=$1; SRC=$2; OUT=$3; shift 3
OV=$(mktemp -d)
cp -r "$SRC/include" "$OV/include"
cp -r "$H/stubs/$V/." "$OV/include/"
g++ -std=gnu++20 -O1 -g -w "$@" -I"$OV/include" \
  "$SRC/src/socketserver.cpp" "$H/host/ndhost_$V.cpp" -o "$OUT" -lpthread
rm -rf "$OV"
