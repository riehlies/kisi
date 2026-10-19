#!/usr/bin/env python3
"""Bundle the Node command line into one file.

Usage: bundle_cli.py <kisi_wasm.wasm> <kisi_sparse.wasm> <out.mjs>

* web/cli/kisi.mjs with the engine (kisi_wasm.wasm) and the field solver's sparse
  module (kisi_sparse.wasm) as base64 and the solver backend as one classic script
  (tools/solver_bundle.py, the same as in the viewer)
* run: node kisi.mjs <command> <board.kicad_pcb> [options]   (Node.js >= 18)
"""
import base64, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from solver_bundle import solver_bundle, upstream_rev  # noqa: E402

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
wasm, sparse, out = sys.argv[1:4]
src = open(os.path.join(root, "web/cli/kisi.mjs"), encoding="utf-8").read()
cargo = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
version = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)
solver_src, n = solver_bundle(root)
b64 = lambda p: base64.b64encode(open(p, "rb").read()).decode()
for k, v in [("__VERSION__", version), ("__UPSTREAM_REV__", upstream_rev(root)), ("__WASM_B64__", b64(wasm)), ("__SPARSE_B64__", b64(sparse))]:
    assert k in src, k
    src = src.replace(k, v)
assert "__SOLVER_SRC__" in src
src = src.replace("__SOLVER_SRC__", json.dumps(solver_src))
os.makedirs(os.path.dirname(out) or ".", exist_ok=True)  # dist/ is not in the repository
open(out, "w", encoding="utf-8", newline="\n").write(src)
print(f"{out}: {len(src) / 1e6:.2f} MB, {n} solver modules, version {version}")
