#!/usr/bin/env python3
"""kisi-advanced.html: the field views of the fork riehlies/js_2d_fields (potential, |E|, |H|, J, power flow,
losses) as one standalone page that kisi.html hands a cross-section to (docs/research/advanced-solvers.md).

Usage: advanced_bundle.py <fork checkout> <kisi_sparse.wasm> <out.html> [--fw DIR]

  <fork checkout>  tools/fetch_fork.sh (the commit pinned in third_party/js_2d_fields_fork.json)
  --fw DIR         the full-wave WebAssembly modules (eigen_solver.{js,wasm}, gmsh.{js,wasm}; fft.{js,wasm} if there)
                   from tools/build_forsten_fullwave.sh; without it the page has the quasi-static backend only
                   (potential and |E|), the Emscripten loaders replaced by web/advanced/fw_missing.js

What goes in (each gzip + base64, decoded in the browser on first use, web/advanced/runtime.js):
  * the fork's page src/field_solver.html with its modules (followed from app_solver.js, fork/plot_fields.js and
    solve_worker.js as the fork's deploy/make_site.mjs does) as one classic script (tools/esm_classic.py), plus
    kisi's web/advanced/handover.js (link hand-over) and kisi_page.js; nothing of the fork is changed apart from
    import/export statements, `import.meta.url` (a fixed URL per module, served by the runtime) and dynamic import()
    (resolved in the bundle);
  * the original's wasm_solver/solver.js (Eigen, not in its git) replaced by kisi-sparse (web/solver/sparse_module.js),
    as in kisi.html;
  * Plotly, the fork's LICENSE and THIRD_PARTY_NOTICES.txt (the About tab links them).
The page's own HTML is changed only where it loads files: the stylesheet inlined, Plotly from the bundle, the module
script tags replaced by the bundle's start. Each such replacement must match exactly once (a changed fork fails here,
not at run time).
"""
import base64, gzip, hashlib, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from esm_classic import convert_module  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FAKE = "https://kisi-advanced.invalid/src/"
PAGE_ENTRIES = ["app_solver.js", "fork/plot_fields.js", "kisi/handover.js", "kisi/kisi_page.js"]
WORKER_ENTRIES = ["solve_worker.js"]
EXTRA = ["tri_solver/tri_backend.js", "wasm_solver/gmsh.js"]  # reached by a computed import() only

IMPORT_RE = re.compile(r"""((?:^|\n)\s*(?:import|export)\s[^'";]*?from\s*)(['"])(\.{1,2}/[^'"]+)\2""")
DYN_LIT_RE = re.compile(r"""\bimport\(\s*(['"])(\.{1,2}/[^'"]+)\1\s*\)""")
DYN_EXPR_RE = re.compile(r"""\bimport\(\s*(?=[A-Za-z_$])""")
EXPORT_DEFAULT_ID_RE = re.compile(r"^export\s+default\s+([A-Za-z_$][\w$]*)\s*;", re.M)


def gzb64(b):
    return base64.b64encode(gzip.compress(b, 9, mtime=0)).decode()


def build(fork, sparse, out, fw=None):
    src_dir = os.path.join(fork, "src")
    fw_files = set(os.listdir(fw)) if fw else set()

    def real(logical):
        """the file a module path of the page stands for"""
        if logical == "wasm_solver/solver.js":
            return os.path.join(ROOT, "web/solver/sparse_module.js")
        if logical.startswith("wasm_solver/") and logical.count("/") == 1 and logical.endswith(".js"):
            name = logical.split("/")[1]
            if name in fw_files:
                return os.path.join(fw, name)
            # without the full-wave build: Eigen's two linear solves from kisi-sparse, the mesher and FFT missing
            stand_in = "eigen_sparse_module.js" if name == "eigen_solver.js" else "fw_missing.js"
            return os.path.join(ROOT, "web/advanced", stand_in)
        if logical.startswith("kisi/"):
            return os.path.join(ROOT, "web/advanced", logical[5:])
        return os.path.join(src_dir, logical)

    index, order, raw = {}, [], {}

    def resolve(importer, spec):
        return os.path.normpath(os.path.join(os.path.dirname(importer), spec)).replace(os.sep, "/")

    def visit(logical):
        if logical in index:
            return index[logical]
        path = real(logical)
        if not os.path.exists(path):
            raise SystemExit(f"advanced_bundle: missing {logical} ({path})")
        index[logical] = len(order)
        order.append(logical)
        text = open(path, encoding="utf-8").read()
        raw[logical] = text
        for m in IMPORT_RE.finditer(text):
            visit(resolve(logical, m.group(3)))
        for m in DYN_LIT_RE.finditer(text):
            visit(resolve(logical, m.group(2)))
        return index[logical]

    for e in PAGE_ENTRIES + WORKER_ENTRIES + EXTRA:
        visit(e)

    parts = ["function (__kisiEntries) {", '"use strict";',
             "const __kisiMods = [];",
             "const __kisiReq = (i) => { const m = __kisiMods[i]; if (!m.e) { m.e = {}; m.f(m.e); } return m.e; };",
             "const __kisiDyn = (i) => Promise.resolve().then(() => __kisiReq(i));",
             f"const __kisiFake = {json.dumps(FAKE)};",
             "const __kisiPaths = " + json.dumps({p: i for p, i in index.items()}) + ";",
             # a computed import(): a module of the bundle by its URL, anything else as the browser does
             "const __kisiImport = (u) => { const s = String(u); const p = s.startsWith(__kisiFake) ? s.slice(__kisiFake.length) : null;"
             " return p !== null && p in __kisiPaths ? __kisiDyn(__kisiPaths[p]) : import(s); };"]
    sources = {}
    for logical in order:
        text = raw[logical]
        text = IMPORT_RE.sub(lambda m: f"{m.group(1)}{m.group(2)}__KISI_MOD_{index[resolve(logical, m.group(3))]}__{m.group(2)}", text)
        text = DYN_LIT_RE.sub(lambda m: f"__kisiDyn({index[resolve(logical, m.group(2))]})", text)
        text = DYN_EXPR_RE.sub("__kisiImport(", text)
        text = text.replace("import.meta.url", json.dumps(FAKE + logical))
        if "import.meta" in text:
            raise SystemExit(f"advanced_bundle: {logical} uses import.meta beyond import.meta.url")
        text = EXPORT_DEFAULT_ID_RE.sub(r"export { \1 as default };", text)
        try:
            conv = convert_module(text)
        except ValueError as e:
            raise SystemExit(f"advanced_bundle: {logical}: {e}")
        sources[logical] = conv
        parts.append(f"// ---- {logical}" + ("" if real(logical).startswith(src_dir) else f" ({os.path.relpath(real(logical), ROOT)})"))
        parts.append(f"__kisiMods[{index[logical]}] = {{ f: function (__exports) {{\n{conv}}} }};")
    parts.append("for (const e of __kisiEntries) __kisiReq(e);")
    parts.append("}")
    bundle = "\n".join(parts)
    entries = {"#page": [index[e] for e in PAGE_ENTRIES]}
    entries.update({e: index[e] for e in WORKER_ENTRIES})

    pin = json.load(open(os.path.join(ROOT, "third_party/js_2d_fields_fork.json")))
    notices = open(os.path.join(fork, "THIRD_PARTY_NOTICES.txt"), encoding="utf-8").read()
    notices += ("\n\nkisi-advanced.html (kisi, https://github.com/riehlies/kisi, GPL-3.0-only) bundles this page with\n"
                "kisi-sparse (kisi's sparse solver, Rust, faer MIT/Apache-2.0) in place of the original's\n"
                "solver.wasm, and kisi's hand-over scripts (web/advanced/). Built by tools/advanced_bundle.py.\n")
    files = {
        "kisi/bundle.js": bundle.encode(),
        "kisi/bundle.json": json.dumps({"entries": entries}).encode(),
        "plotly-3.3.0.min.js": open(os.path.join(src_dir, "plotly-3.3.0.min.js"), "rb").read(),
        "kisi_sparse.wasm": open(sparse, "rb").read(),
        "kisi/LICENSE.txt": open(os.path.join(fork, "LICENSE"), "rb").read(),
        "kisi/THIRD_PARTY_NOTICES.txt": notices.encode(),
    }
    fw_info = None
    for name in ("eigen_solver.wasm", "gmsh.wasm", "fft.wasm"):
        if name in fw_files and name.replace(".wasm", ".js") in fw_files:
            files["wasm_solver/" + name] = open(os.path.join(fw, name), "rb").read()
    if fw and "BUILD_INFO.json" in fw_files:
        fw_info = json.load(open(os.path.join(fw, "BUILD_INFO.json")))
        fw_info = {k: fw_info.get(k) for k in ("upstream_rev", "emsdk", "occt", "submodules", "built")}

    html = open(os.path.join(src_dir, "field_solver.html"), encoding="utf-8").read()

    def once(old, new):
        nonlocal html
        n = html.count(old)
        if n != 1:
            raise SystemExit(f"advanced_bundle: expected one {old!r} in field_solver.html, found {n}")
        html = html.replace(old, new)

    css = open(os.path.join(src_dir, "solver-style.css"), encoding="utf-8").read()
    once('<link rel="stylesheet" href="solver-style.css">', "<style>\n" + css + "\n</style>")
    once("const PRIMARY_PLOTLY_SRC = 'plotly-3.3.0.min.js';", "const PRIMARY_PLOTLY_SRC = window.__kisiPlotlyURL;")
    once("const FALLBACK_PLOTLY_SRC = 'plotly-3.3.0.min.js';", "const FALLBACK_PLOTLY_SRC = window.__kisiPlotlyURL;")
    once("requestIdleCallback(loadPlotly);", "window.__kisiReady.then(loadPlotly);")
    once("setTimeout(loadPlotly, 1);", "window.__kisiReady.then(loadPlotly);")
    once('<script type="module" src="app_solver.js"></script>',
         "<script>window.__kisiReady.then(() => window.__kisiStartPage(), (e) => { document.body.prepend("
         "Object.assign(document.createElement('pre'), { textContent: 'kisi-advanced: ' + e })); });</script>")
    html = re.sub(r'<script type="module" src="fork/plot_fields.js"></script>[^\n]*', "", html)
    html = html.replace("BUILD_VERSION", f"fork {pin['commit'][:7]} in kisi-advanced.html")
    info = {"repo": pin["repo"], "commit": pin["commit"], "upstream_base": pin["upstream_base"], "fullwave": fw_info}
    head = ('<script type="application/json" id="kisi-adv-files">' + json.dumps({k: gzb64(v) for k, v in files.items()}) +
            "</script>\n<script>window.__kisiAdvInfo = " + json.dumps(info) + ";</script>\n<script>\n" +
            open(os.path.join(ROOT, "web/advanced/runtime.js"), encoding="utf-8").read().replace("</script", "<\\/script") +
            "</script>\n")
    once("<head>\n", "<head>\n" + head)
    head_note = ("<!-- kisi-advanced.html: the field views of js_2d_fields (fork " + pin["repo"] + " at " + pin["commit"] +
                 ", original by Henrik Forstén), GPL-3.0; built by kisi's tools/advanced_bundle.py. -->\n")
    html = html.replace("<!DOCTYPE html>\n", "<!DOCTYPE html>\n" + head_note, 1)
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    open(out, "w", encoding="utf-8").write(html)
    sizes = {k: len(v) for k, v in files.items()}
    print(f"{out}: {len(html.encode()) / 1e6:.2f} MB, {len(order)} modules, full-wave: {'yes' if fw_files else 'no'}; "
          + ", ".join(f"{k} {v / 1e6:.2f} MB" for k, v in sizes.items()))
    return {"bytes": len(html.encode()), "files": sizes, "modules": len(order),
            "sha256": hashlib.sha256(html.encode()).hexdigest()}


if __name__ == "__main__":
    a = sys.argv[1:]
    fw = None
    if "--fw" in a:
        i = a.index("--fw")
        fw = a[i + 1]
        del a[i:i + 2]
    if len(a) != 3:
        raise SystemExit(__doc__)
    build(*a, fw=fw)
