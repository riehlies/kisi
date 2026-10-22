#!/usr/bin/env python3
"""The field solver backend (web/solver/backend_forsten.js and the vendored upstream
js_2d_fields ES modules it imports) as one classic script, shared by the viewer
bundle (tools/bundle.py) and the Node CLI bundle (tools/bundle_cli.py).
Upstream code is unchanged apart from import/export statements (tools/esm_classic.py)."""
import os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from esm_classic import classic_bundle  # noqa: E402


def engine_notices(root):
    """Licence notices of third-party code compiled into the engine (kisi_wasm.wasm) that its licence asks
    binary redistributions to carry: the IBIS parser after KiCad's KIBIS (BSD-3-Clause,
    crates/kisi-ibis/NOTICE). The bundlers write it as a comment; the page and the CLI do not show it."""
    return open(os.path.join(root, "crates/kisi-ibis/NOTICE"), encoding="utf-8").read()


def upstream_rev(root):
    p = os.path.join(root, "third_party/UPSTREAM_REV")
    return open(p).read().strip() if os.path.exists(p) else "unknown"


def solver_bundle(root):
    """(classic script source, number of modules)"""
    ENTRY = os.path.join(root, "web/solver/backend_forsten.js")
    REDIRECT = {  # upstream build artifact -> kisi replacement
        "wasm_solver/solver.js": os.path.join(root, "web/solver/sparse_module.js"),
    }
    IMPORT_RE = re.compile(r"""((?:^|\n)\s*(?:import|export)\s[^'";]*?from\s*)(['"])(\.{1,2}/[^'"]+)\2""")
    mods, order = {}, []

    def resolve(base, spec):
        for k, v in REDIRECT.items():
            if spec.endswith(k):
                return v
        return os.path.normpath(os.path.join(os.path.dirname(base), spec))

    def visit(path):
        if path in mods:
            return
        text = open(path, encoding="utf-8").read()
        mods[path] = None  # mark (cycle guard)
        deps = []

        def sub(m):
            dep = resolve(path, m.group(3))
            visit(dep)
            deps.append(dep)
            return f"{m.group(1)}{m.group(2)}__KISI_MOD_{order_index(dep)}__{m.group(2)}"

        # visit deps first so indices exist
        for m in IMPORT_RE.finditer(text):
            visit(resolve(path, m.group(3)))
        text = IMPORT_RE.sub(sub, text)
        mods[path] = {"src": text, "deps": sorted(set(order_index(d) for d in deps))}
        order.append(path)

    def order_index(p):
        return order.index(p)

    visit(ENTRY)
    modules = [{"name": os.path.relpath(p, root), "src": mods[p]["src"], "deps": mods[p]["deps"]} for p in order]
    return classic_bundle(modules), len(modules)
