#!/bin/sh
# Fetch the fork riehlies/js_2d_fields (the field views, FORK.md there) at the commit pinned in
# third_party/js_2d_fields_fork.json, for the optional bundle kisi-advanced.html (tools/advanced_bundle.py).
# Not vendored: the fork carries the whole original app (about 2 MB of JS) plus Plotly (4.8 MB); kisi keeps only the
# pin and its own adapter (web/advanced/). Read-only: nothing is pushed anywhere.
#
# Usage: tools/fetch_fork.sh [DIR]      (default target/js2d-fork; reused when it is already at the pinned commit)
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
dir=${1:-$root/target/js2d-fork}
pin=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["repo"], d["commit"])' "$root/third_party/js_2d_fields_fork.json")
repo=${pin% *}; rev=${pin#* }
if [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null || true)" != "$rev" ]; then
  rm -rf "$dir"
  mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" remote add origin "$repo"
  # the pinned commit only, no submodules (the WebAssembly sources are built by tools/build_forsten_fullwave.sh)
  GIT_LFS_SKIP_SMUDGE=1 git -C "$dir" fetch -q --depth 1 origin "$rev"
  git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
fi
test "$(git -C "$dir" rev-parse HEAD)" = "$rev"
echo "$dir: js_2d_fields fork at ${rev%"${rev#????????????}"}"
