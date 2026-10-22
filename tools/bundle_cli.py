#!/usr/bin/env python3
"""Bundle the Node command line into one file.

Usage: bundle_cli.py <kisi_wasm.wasm> <kisi_sparse.wasm> <out.mjs>

* web/cli/kisi.mjs with the engine (kisi_wasm.wasm) and the field solver's sparse
  module (kisi_sparse.wasm) and the solver backend as one classic script
  (tools/solver_bundle.py, the same as in the viewer), each gzip + base64
* run: node kisi.mjs <command> <board.kicad_pcb> [options]   (Node.js >= 18)
"""
import base64, gzip, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from solver_bundle import engine_notices, solver_bundle, upstream_rev  # noqa: E402

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
wasm, sparse, out = sys.argv[1:4]
src = open(os.path.join(root, "web/cli/kisi.mjs"), encoding="utf-8").read()
cargo = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
version = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)
solver_src, n = solver_bundle(root)
# gzip before base64: about a third of the size; kisi.mjs unpacks with zlib
gzb64 = lambda b: base64.b64encode(gzip.compress(b, 9, mtime=0)).decode()
for k, v in [("__VERSION__", version), ("__UPSTREAM_REV__", upstream_rev(root)), ("__WASM_GZ__", gzb64(open(wasm, "rb").read())),
             ("__SPARSE_GZ__", gzb64(open(sparse, "rb").read())), ("__SOLVER_GZ__", gzb64(solver_src.encode("utf-8")))]:
    assert k in src, k
    src = src.replace(k, v)
# third-party code in the engine whose licence asks binary redistributions to carry its notice (the IBIS
# parser after KiCad's KIBIS, BSD-3-Clause): the full notice as a comment after the SPDX line
head, rest = src.split("\n", 2)[:2], src.split("\n", 2)[2]
src = "\n".join(head) + "\n/*\n" + engine_notices(root).replace("*/", "* /") + "*/\n" + rest
os.makedirs(os.path.dirname(out) or ".", exist_ok=True)  # dist/ is not in the repository
open(out, "w", encoding="utf-8", newline="\n").write(src)
print(f"{out}: {len(src) / 1e6:.2f} MB, {n} solver modules, version {version}")
