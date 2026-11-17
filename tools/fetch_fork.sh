#!/bin/sh
# Fetch the fork riehlies/js_2d_fields (the field views, FORK.md there) at the commit pinned in
# third_party/js_2d_fields_fork.json and apply kisi's patches to it (web/advanced/fork-patches/*.patch, in name
# order; README.md there), for the field page kisi-advanced.html (tools/advanced_bundle.py).
# Not vendored: the fork carries the whole original app (about 2 MB of JS) plus Plotly (4.8 MB); kisi keeps only the
# pin, the patches and its own adapter (web/advanced/). Read-only: nothing is pushed anywhere.
# A patch that does not apply fails here, loudly: the pin moved past it (drop it, README.md) or the fork changed under
# it (refresh it).
#
# Usage: tools/fetch_fork.sh [DIR]      (default target/js2d-fork; the fetch is reused when it is at the pinned
#                                        commit, the patches are applied to a clean tree every time)
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
# a clean tree at the pin, then kisi's patches
git -C "$dir" reset -q --hard "$rev"
git -C "$dir" clean -q -fdx
n=0
for p in "$root"/web/advanced/fork-patches/*.patch; do
  [ -e "$p" ] || continue
  if ! git -C "$dir" apply --check --whitespace=nowarn "$p" 2>"$dir/.kisi-patch.err"; then
    echo "error: $(basename "$p") does not apply to the fork at $rev:" >&2
    cat "$dir/.kisi-patch.err" >&2
    echo "(web/advanced/fork-patches/README.md: drop a patch the pinned fork has taken over, refresh one it changed)" >&2
    exit 1
  fi
  git -C "$dir" apply --whitespace=nowarn "$p"
  n=$((n + 1))
done
rm -f "$dir/.kisi-patch.err"
echo "$dir: js_2d_fields fork at ${rev%"${rev#????????????}"} with $n patch(es) of kisi"
