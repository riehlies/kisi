#!/bin/bash
# Build the WebAssembly modules of js_2d_fields' full-wave (triangular FEM) backend from upstream's
# sources at exactly the vendored revision (third_party/UPSTREAM_REV):
#   eigen_solver.{js,wasm}  Eigen + Spectra (upstream `make eigen_solver`)
#   gmsh.{js,wasm}          gmsh (upstream's submodule commit) + OpenCASCADE, cross-compiled
#                           (upstream `make gmsh`, link-occ-wasm.sh, gmsh.patch)
# These binaries are not in upstream's git nor in kisi; kisi uses them for validation only
# (UPSTREAM.md, "Full-wave backend"). Nothing under third_party/ is touched: upstream is cloned into
# the build directory, its Makefile and patches are used as they are.
#
# Usage: tools/build_forsten_fullwave.sh [BUILD_DIR]      (default target/forsten-fw)
# Result: BUILD_DIR/dist/{eigen_solver,gmsh}.{js,wasm} and BUILD_DIR/dist/BUILD_INFO.json
# Needs: git, python3, cmake, ninja, make; network access to github.com, gitlab.com (Eigen) and
# gitlab.onelab.info (gmsh). About 40 min on 4 cores (OCCT 31 min); each stage is skipped when its
# output for the same pinned versions is already there (CI caches emsdk/, occt-install/ and dist/).
# Pinned: EMSDK_VERSION, OCCT_TAG below (override from the environment only to try another version).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
B=$(mkdir -p "${1:-$ROOT/target/forsten-fw}" && cd "${1:-$ROOT/target/forsten-fw}" && pwd)
REV=$(cat "$ROOT/third_party/UPSTREAM_REV")
EMSDK_VER=${EMSDK_VERSION:-6.0.5}   # own name: emsdk_env.sh unsets EMSDK_VERSION
OCCT_TAG=${OCCT_TAG:-V7_5_1}
JOBS=${JOBS:-$(nproc)}
UP=$B/src/js_2d_fields
WS=$UP/src/wasm_solver
OCCT_INSTALL=$B/occt-install
DIST=$B/dist
mkdir -p "$B/src" "$DIST" "$B/logs"
T0=$(date +%s)
stamp() { echo "[$(date -u +%H:%M:%S)] $*"; }

# --- emsdk ------------------------------------------------------------------------------------------
if [ ! -f "$B/emsdk/.kisi-$EMSDK_VER" ]; then
  stamp "emsdk $EMSDK_VER"
  rm -rf "$B/emsdk"
  git clone -q https://github.com/emscripten-core/emsdk.git "$B/emsdk"
  "$B/emsdk/emsdk" install "$EMSDK_VER" > "$B/logs/emsdk.log"
  "$B/emsdk/emsdk" activate "$EMSDK_VER" >> "$B/logs/emsdk.log"
  touch "$B/emsdk/.kisi-$EMSDK_VER"
fi
# shellcheck disable=SC1091
EMSDK_QUIET=1 . "$B/emsdk/emsdk_env.sh"
EMCC_VERSION=$(emcc --version | head -1)
stamp "$EMCC_VERSION"

# --- upstream at the vendored revision, with its submodules -----------------------------------------
if [ "$(git -C "$UP" rev-parse HEAD 2>/dev/null || true)" != "$REV" ]; then
  stamp "js_2d_fields $REV"
  rm -rf "$UP"
  git clone -q https://github.com/Ttl/js_2d_fields.git "$UP"
  git -C "$UP" checkout -q "$REV"
fi
for sm in eigen spectra gmsh; do
  if [ ! -e "$WS/$sm/.git" ]; then
    stamp "submodule $sm"
    git -C "$UP" submodule update --init --depth 1 "src/wasm_solver/$sm" 2>/dev/null \
      || git -C "$UP" submodule update --init "src/wasm_solver/$sm"
  fi
done
# the build sources must be the vendored ones (third_party is a git archive of the same revision)
DIFF=$(diff -rq --exclude=eigen --exclude=spectra --exclude=gmsh "$ROOT/third_party/js_2d_fields/src" "$UP/src" \
  | grep -v "^Only in $WS" || true)   # build outputs appear in the checkout only
if [ -n "$DIFF" ]; then
  echo "error: the upstream checkout differs from third_party/js_2d_fields:" >&2; echo "$DIFF" >&2; exit 1
fi
SUBS=$(git -C "$UP" submodule status | awk '{gsub(/^[-+ ]/,"",$1); printf "%s\"%s\": \"%s\"", (n++?", ":""), $2, $1}')

# --- eigen_solver ----------------------------------------------------------------------------------
T1=$(date +%s)
if [ ! -f "$DIST/eigen_solver.wasm" ]; then
  stamp "make eigen_solver"
  make -C "$WS" eigen_solver > "$B/logs/eigen_solver.log" 2>&1 || { tail -50 "$B/logs/eigen_solver.log"; exit 1; }
  cp "$WS/eigen_solver.js" "$WS/eigen_solver.wasm" "$DIST/"
fi
T2=$(date +%s)

# --- OpenCASCADE (upstream README, "gmsh (OCC-enabled)") -------------------------------------------
if [ ! -f "$OCCT_INSTALL/.kisi-$OCCT_TAG-$EMSDK_VER" ]; then
  stamp "OCCT $OCCT_TAG (about 30 min on 4 cores)"
  rm -rf "$B/occt" "$OCCT_INSTALL"
  git clone -q --depth 1 --branch "$OCCT_TAG" https://github.com/Open-Cascade-SAS/OCCT.git "$B/occt"
  sed -i 's/^DataExchange .*/DataExchange TKXSBase TKSTEPBase TKSTEPAttr TKSTEP209 TKSTEP TKIGES/' "$B/occt/adm/MODULES"
  emcmake cmake -S "$B/occt" -B "$B/occt/build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_LIBRARY_TYPE=Static \
    -DBUILD_MODULE_FoundationClasses=ON -DBUILD_MODULE_ModelingData=ON \
    -DBUILD_MODULE_ModelingAlgorithms=ON -DBUILD_MODULE_DataExchange=ON \
    -DBUILD_MODULE_Visualization=OFF -DBUILD_MODULE_ApplicationFramework=OFF -DBUILD_MODULE_Draw=OFF \
    -DUSE_FREETYPE=OFF -DUSE_TK=OFF -DUSE_TBB=OFF -DUSE_FREEIMAGE=OFF -DUSE_RAPIDJSON=OFF \
    -DUSE_OPENGL=OFF -DUSE_GLES2=OFF -DBUILD_DOC_Overview=OFF \
    -DINSTALL_DIR="$OCCT_INSTALL" -DINSTALL_DIR_LAYOUT=Unix \
    -DCMAKE_CXX_FLAGS=-fwasm-exceptions -DCMAKE_C_FLAGS=-fwasm-exceptions \
    -DCMAKE_CXX_FLAGS_RELEASE="-O2 -DNDEBUG" -DCMAKE_C_FLAGS_RELEASE="-O2 -DNDEBUG" > "$B/logs/occt-configure.log" 2>&1 \
    || { tail -50 "$B/logs/occt-configure.log"; exit 1; }
  ninja -C "$B/occt/build" -j"$JOBS" install > "$B/logs/occt-build.log" 2>&1 || { grep -E -B2 -A8 "error|FAILED" "$B/logs/occt-build.log" | head -120; exit 1; }
  rm -rf "$B/occt"   # sources and objects: 2-3 GB, not needed any more
  touch "$OCCT_INSTALL/.kisi-$OCCT_TAG-$EMSDK_VER"
fi
T3=$(date +%s)

# --- gmsh with OCC ---------------------------------------------------------------------------------
if [ ! -f "$DIST/gmsh.wasm" ]; then
  stamp "make gmsh"
  rm -rf "$WS/gmsh/build-wasm-occ"
  OCCT_WASM="$OCCT_INSTALL" make -C "$WS" gmsh > "$B/logs/gmsh.log" 2>&1 \
    || { grep -E -B2 -A8 "error|Error" "$B/logs/gmsh.log" | head -120; tail -30 "$B/logs/gmsh.log"; exit 1; }
  cp "$WS/gmsh.js" "$WS/gmsh.wasm" "$DIST/"
fi
T4=$(date +%s)

# --- record what was built -------------------------------------------------------------------------
python3 - "$DIST" "$REV" "$EMSDK_VER" "$EMCC_VERSION" "$OCCT_TAG" "{$SUBS}" "$((T2 - T1))" "$((T3 - T2))" "$((T4 - T3))" <<'PY'
import hashlib, json, os, platform, sys, time
dist, rev, emsdk, emcc, occt, subs, t_eig, t_occt, t_gmsh = sys.argv[1:]
p = os.path.join(dist, "BUILD_INFO.json")
old = json.load(open(p)) if os.path.exists(p) else {}
files = {f: {"bytes": os.path.getsize(os.path.join(dist, f)),
             "sha256": hashlib.sha256(open(os.path.join(dist, f), "rb").read()).hexdigest()}
         for f in sorted(os.listdir(dist)) if f.endswith((".js", ".wasm"))}
secs = {"eigen_solver": int(t_eig), "occt": int(t_occt), "gmsh": int(t_gmsh)}
info = {"about": "js_2d_fields full-wave backend WASM modules (tools/build_forsten_fullwave.sh)",
        "upstream": "https://github.com/Ttl/js_2d_fields", "upstream_rev": rev, "submodules": json.loads(subs),
        "emsdk": emsdk, "emcc": emcc, "occt": occt, "built": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {"machine": platform.machine(), "cpus": os.cpu_count()},
        # a stage skipped this time (already built) keeps the time of the run that built it
        "seconds": {k: (v if v > 5 or k not in old.get("seconds", {}) else old["seconds"][k]) for k, v in secs.items()},
        "files": files}
json.dump(info, open(p, "w"), indent=1)
print(json.dumps(info, indent=1))
PY
stamp "done in $(( $(date +%s) - T0 )) s → $DIST"
