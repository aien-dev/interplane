#!/bin/sh
# Reproduce the NightDriver 666 matrix. Needs: git, g++ (C++20), cargo, and a clone of
# https://github.com/PlummersSoftwareLLC/NightDriverStrip (set ND_REPO). No NightDriver source is stored in this repo:
# each snapshot is extracted unmodified with `git archive` at the SHA under test.
#
#   ND_REPO=~/workspace/external/NightDriverStrip ./run-matrix.sh > results.tsv
set -e
H=$(cd "$(dirname "$0")" && pwd)
: "${ND_REPO:?set ND_REPO to a NightDriverStrip clone}"
W=$(mktemp -d)
(cd "$H/torture" && cargo build --release -q)
T="$H/torture/target/release/nd666-torture"

# label  variant  sha  audio    (variant: which stub set matches that snapshot's platform API)
# s0 = 15221f20 last socket-server commit before the issue was filed (2024-11-09)
# s1 = parent of the fix commit,  s2 = the fix commit d73d18db (2025-08-27),  main = pinned upstream main
while read -r label variant sha audio; do
  git -C "$ND_REPO" archive "$sha" include src/socketserver.cpp | (mkdir -p "$W/$label" && tar -x -C "$W/$label")
  "$H/host/build.sh" "$variant" "$W/$label" "$W/$label.bin" -DENABLE_AUDIO="$audio"
  "$T" "$W/$label.bin" --label "${label}_audio${audio}" --audio "$audio"
done <<LIST
s0   legacy 15221f20  0
s1   legacy d73d18db~1 0
s2   legacy d73d18db  0
s2   legacy d73d18db  1
main modern 4d79c290  0
main modern 4d79c290  1
LIST
rm -rf "$W"
