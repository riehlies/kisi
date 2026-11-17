#!/usr/bin/env python3
"""kisi-advanced.html: the field views of the fork riehlies/js_2d_fields (potential, |E|, |H|, J, power flow,
losses) as one standalone page that kisi.html hands a cross-section to (docs/reference.md, "Field views").

Usage: advanced_bundle.py <fork checkout> <kisi_sparse.wasm> <out.html> [--fw DIR]

  <fork checkout>  tools/fetch_fork.sh: the commit pinned in third_party/js_2d_fields_fork.json with kisi's patches
                   (web/advanced/fork-patches/) applied
  --fw DIR         the full-wave WebAssembly modules (eigen_solver.{js,wasm}, gmsh.{js,wasm}; fft.{js,wasm} if there)
                   and their BUILD_INFO.json from tools/build_forsten_fullwave.sh: kisi-advanced.html with every view.
                   Without it: the lite page (kisi-advanced-lite.html in a release) with the quasi-static backend only
                   (potential and |E|), Eigen's two linear solves from kisi-sparse (web/advanced/eigen_sparse_module.js),
                   the other Emscripten loaders replaced by web/advanced/fw_missing.js; the page says what it lacks.

What goes in (each gzip + base64, decoded in the browser on first use, web/advanced/runtime.js):
  * the fork's page src/field_solver.html with its modules (followed from the page's module tags, solve_worker.js and
    the two modules reached by a computed import(), as the fork's deploy/make_site.mjs does) as one classic script
    (tools/esm_classic.py), plus kisi's web/advanced/kisi_page.js; nothing of the fork is changed apart from
    import/export statements, `import.meta.url` (a fixed URL per module, served by the runtime) and dynamic import()
    (resolved in the bundle);
  * the original's wasm_solver/solver.js (Eigen, not in its git) replaced by kisi-sparse (web/solver/sparse_module.js),
    as in kisi.html;
  * Plotly, the fork's LICENSE and THIRD_PARTY_NOTICES.txt (the About tab links them) with the versions of the
    WebAssembly build.
The page's own HTML is changed only where it loads files: the stylesheets inlined (the fork's light theme included),
the theme script inlined, Plotly from the bundle, the module script tags replaced by the bundle's start, and a first
script that tells the page that opened it which build it is. Each such replacement must match exactly once and every
module tag of the page must be bundled (a changed fork fails here, not at run time). The output depends only on its
inputs (reproducible); its SHA-256 is printed and, with --fw, the modules' SHA-256 are checked against BUILD_INFO.json.
"""
import base64, gzip, hashlib, json, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from esm_classic import convert_module  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FAKE = "https://kisi-advanced.invalid/src/"
KISI_ENTRIES = ["kisi/kisi_page.js"]
WORKER_ENTRIES = ["solve_worker.js"]
EXTRA = ["tri_solver/tri_backend.js", "wasm_solver/gmsh.js"]  # reached by a computed import() only
MODULE_TAG_RE = re.compile(r'<script type="module" src="([^"]+)"></script>[^\n]*\n')

IMPORT_RE = re.compile(r"""((?:^|\n)\s*(?:import|export)\s[^'";]*?from\s*)(['"])(\.{1,2}/[^'"]+)\2""")
DYN_LIT_RE = re.compile(r"""\bimport\(\s*(['"])(\.{1,2}/[^'"]+)\1\s*\)""")
DYN_EXPR_RE = re.compile(r"""\bimport\(\s*(?=[A-Za-z_$])""")
EXPORT_DEFAULT_ID_RE = re.compile(r"^export\s+default\s+([A-Za-z_$][\w$]*)\s*;", re.M)
# Emscripten's loaders are minified: their `export default Module;` need not start a line
EMSCRIPTEN_EXPORT_RE = re.compile(r"(?<![\w$.])export\s+default\s+([A-Za-z_$][\w$]*)\s*;")
FW_MODULES = ("eigen_solver", "gmsh", "fft")


def gzb64(b):
    return base64.b64encode(gzip.compress(b, 9, mtime=0)).decode()


def check_syntax(bundle):
    """the bundle must parse as one classic script (node --check), else the page would not start at all"""
    node = shutil.which("node")
    if not node:
        print("advanced_bundle: node not found, syntax not checked")
        return
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as f:
        f.write("const __kisiBundle = (" + bundle + ");\n")
    r = subprocess.run([node, "--check", f.name], capture_output=True, text=True)
    os.unlink(f.name)
    if r.returncode:
        raise SystemExit("advanced_bundle: the bundle does not parse:\n" + r.stderr[:3000])


def fw_build(fw):
    """the full-wave modules and what BUILD_INFO.json says about them (versions, SHA-256 checked)"""
    names = set(os.listdir(fw))
    if "BUILD_INFO.json" not in names:
        raise SystemExit(f"advanced_bundle: {fw} has no BUILD_INFO.json (tools/build_forsten_fullwave.sh writes it)")
    info = json.load(open(os.path.join(fw, "BUILD_INFO.json"), encoding="utf-8"))
    for m in ("eigen_solver", "gmsh"):
        if not {m + ".js", m + ".wasm"} <= names:
            raise SystemExit(f"advanced_bundle: {fw} lacks {m}.js / {m}.wasm")
    files = {}
    for f in sorted(names):
        if not f.endswith((".js", ".wasm")) or f.rsplit(".", 1)[0] not in FW_MODULES:
            continue
        data = open(os.path.join(fw, f), "rb").read()
        sha = hashlib.sha256(data).hexdigest()
        want = (info.get("files") or {}).get(f, {}).get("sha256")
        if want != sha:
            raise SystemExit(f"advanced_bundle: {f}: SHA-256 {sha[:12]}… is not the one in BUILD_INFO.json ({str(want)[:12]}…)")
        files[f] = sha
    keep = {k: info.get(k) for k in ("upstream", "upstream_rev", "submodules", "emsdk", "emcc", "occt", "built")}
    keep["files"] = files
    return keep


def notices_of_build(fw_info):
    if not fw_info:
        return ("This lite build contains no full-wave WebAssembly modules (no gmsh, OCCT, Eigen or Spectra code):\n"
                "the quasi-static solve runs on kisi-sparse.\n")
    subs = fw_info.get("submodules") or {}
    lines = [f"The full-wave WebAssembly modules in this page were built by kisi's tools/build_forsten_fullwave.sh from",
             f"  js_2d_fields (original) {fw_info.get('upstream', '')} at {fw_info.get('upstream_rev', '')}",
             *[f"  {k} at {v}" for k, v in sorted(subs.items())],
             f"  OpenCASCADE {fw_info.get('occt', '')} (https://github.com/Open-Cascade-SAS/OCCT)",
             f"  Emscripten SDK {fw_info.get('emsdk', '')} ({fw_info.get('emcc', '')})",
             f"  built {fw_info.get('built', '')}; SHA-256: " + ", ".join(f"{k} {v}" for k, v in fw_info["files"].items())]
    return "\n".join(lines) + "\n"


def build(fork, sparse, out, fw=None):
    src_dir = os.path.join(fork, "src")
    fw = os.path.abspath(fw) if fw else None
    fw_info = fw_build(fw) if fw else None
    fw_files = set(fw_info["files"]) if fw_info else set()
    html = open(os.path.join(src_dir, "field_solver.html"), encoding="utf-8").read()
    page_modules = MODULE_TAG_RE.findall(html)
    if not page_modules or page_modules[0] != "app_solver.js":
        raise SystemExit(f"advanced_bundle: field_solver.html: module tags {page_modules}, expected app_solver.js first")
    page_entries = page_modules + KISI_ENTRIES

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

    for e in page_entries + WORKER_ENTRIES + EXTRA:
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
    for logical in order:
        text = raw[logical]
        text = IMPORT_RE.sub(lambda m: f"{m.group(1)}{m.group(2)}__KISI_MOD_{index[resolve(logical, m.group(3))]}__{m.group(2)}", text)
        text = DYN_LIT_RE.sub(lambda m: f"__kisiDyn({index[resolve(logical, m.group(2))]})", text)
        text = DYN_EXPR_RE.sub("__kisiImport(", text)
        text = text.replace("import.meta.url", json.dumps(FAKE + logical))
        if "import.meta" in text:
            raise SystemExit(f"advanced_bundle: {logical} uses import.meta beyond import.meta.url")
        text = EXPORT_DEFAULT_ID_RE.sub(r"export { \1 as default };", text)
        if fw and real(logical).startswith(fw):
            text = EMSCRIPTEN_EXPORT_RE.sub(lambda m: f"\nexport {{ {m.group(1)} as default }};\n", text)
        try:
            conv = convert_module(text)
        except ValueError as e:
            raise SystemExit(f"advanced_bundle: {logical}: {e}")
        parts.append(f"// ---- {logical}" + ("" if real(logical).startswith(src_dir) else f" ({os.path.relpath(real(logical), ROOT)})"))
        parts.append(f"__kisiMods[{index[logical]}] = {{ f: function (__exports) {{\n{conv}}} }};")
    parts.append("for (const e of __kisiEntries) __kisiReq(e);")
    parts.append("}")
    bundle = "\n".join(parts)
    check_syntax(bundle)
    entries = {"#page": [index[e] for e in page_entries]}
    entries.update({e: index[e] for e in WORKER_ENTRIES})

    pin = json.load(open(os.path.join(ROOT, "third_party/js_2d_fields_fork.json"), encoding="utf-8"))
    patches = sorted(f for f in os.listdir(os.path.join(ROOT, "web/advanced/fork-patches")) if f.endswith(".patch"))
    notices = open(os.path.join(fork, "THIRD_PARTY_NOTICES.txt"), encoding="utf-8").read()
    notices += ("\n\n" + "=" * 78 + "\nkisi-advanced.html\n" + "=" * 78 + "\n"
                "kisi (https://github.com/riehlies/kisi, GPL-3.0-only) bundles this page as one file with\n"
                "kisi-sparse (kisi's sparse solver, Rust; faer MIT/Apache-2.0) in place of the original's solver.wasm\n"
                "and kisi's scripts web/advanced/ (tools/advanced_bundle.py). The fork " + pin["repo"] + "\n"
                "at " + pin["commit"] + " with kisi's patches web/advanced/fork-patches/ (" + ", ".join(patches) + ")\n"
                "is the source of the page; the release notes of kisi name the corresponding source.\n"
                + notices_of_build(fw_info))
    files = {
        "kisi/bundle.js": bundle.encode(),
        "kisi/bundle.json": json.dumps({"entries": entries}).encode(),
        "plotly-3.3.0.min.js": open(os.path.join(src_dir, "plotly-3.3.0.min.js"), "rb").read(),
        "kisi_sparse.wasm": open(sparse, "rb").read(),
        "kisi/LICENSE.txt": open(os.path.join(fork, "LICENSE"), "rb").read(),
        "kisi/THIRD_PARTY_NOTICES.txt": notices.encode(),
    }
    for name in sorted(fw_files):
        if name.endswith(".wasm") and name.replace(".wasm", ".js") in fw_files:
            files["wasm_solver/" + name] = open(os.path.join(fw, name), "rb").read()

    def once(old, new):
        nonlocal html
        n = html.count(old)
        if n != 1:
            raise SystemExit(f"advanced_bundle: expected one {old!r} in field_solver.html, found {n}")
        html = html.replace(old, new)

    def text_of(rel):
        text = open(os.path.join(src_dir, rel), encoding="utf-8").read()
        if rel.endswith(".css") and "</style" in text.lower():
            raise SystemExit(f"advanced_bundle: {rel} contains </style")
        return text.replace("</script", "<\\/script")

    for css in ("solver-style.css", "fork/theme.css"):
        once(f'<link rel="stylesheet" href="{css}">', "<style>\n" + text_of(css) + "\n</style>")
    once('<script src="fork/theme.js"></script>', "<script>\n" + text_of("fork/theme.js") + "</script>")
    once("const PRIMARY_PLOTLY_SRC = 'plotly-3.3.0.min.js';", "const PRIMARY_PLOTLY_SRC = window.__kisiPlotlyURL;")
    once("const FALLBACK_PLOTLY_SRC = 'plotly-3.3.0.min.js';", "const FALLBACK_PLOTLY_SRC = window.__kisiPlotlyURL;")
    once("requestIdleCallback(loadPlotly);", "window.__kisiReady.then(loadPlotly);")
    once("setTimeout(loadPlotly, 1);", "window.__kisiReady.then(loadPlotly);")
    html = MODULE_TAG_RE.sub("", html)
    once("</body>", "<script>window.__kisiReady.then(() => window.__kisiStartPage()).catch((e) => { document.body.prepend("
         "Object.assign(document.createElement('pre'), { textContent: 'kisi-advanced: ' + e })); });</script>\n</body>")
    html = html.replace("BUILD_VERSION", f"fork {pin['commit'][:7]} in kisi-advanced.html")
    build_kind = "full" if fw_info else "lite"
    info = {"repo": pin["repo"], "commit": pin["commit"], "upstream_base": pin["upstream_base"], "patches": patches,
            "build": build_kind, "fullwave": fw_info}
    # first: tell the page that opened this one (kisi's "Fields…") that it is here and which build it is
    hello = ("<script>try { if (window.opener) window.opener.postMessage({ type: 'kisi-advanced', build: "
             + json.dumps(build_kind) + ", commit: " + json.dumps(pin["commit"][:7]) + " }, '*'); } catch (e) { /* no opener */ }</script>\n")
    head = (hello + '<script type="application/json" id="kisi-adv-files">' + json.dumps({k: gzb64(v) for k, v in files.items()}) +
            "</script>\n<script>window.__kisiAdvInfo = " + json.dumps(info) + ";</script>\n<script>\n" +
            open(os.path.join(ROOT, "web/advanced/runtime.js"), encoding="utf-8").read().replace("</script", "<\\/script") +
            "</script>\n")
    once("<head>\n", "<head>\n" + head)
    head_note = (f"<!-- kisi-advanced.html ({build_kind} build): the field views of js_2d_fields (fork {pin['repo']} at "
                 f"{pin['commit']} with kisi's patches, original by Henrik Forstén), GPL-3.0; built by kisi's "
                 "tools/advanced_bundle.py. -->\n")
    if not html.startswith("<!DOCTYPE html>\n"):
        raise SystemExit("advanced_bundle: field_solver.html does not start with <!DOCTYPE html>")
    html = html.replace("<!DOCTYPE html>\n", "<!DOCTYPE html>\n" + head_note, 1)
    data = html.encode("utf-8")
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    open(out, "wb").write(data)
    sizes = {k: len(v) for k, v in files.items()}
    sha = hashlib.sha256(data).hexdigest()
    print(f"{out}: {len(data) / 1e6:.2f} MB, {build_kind} build, {len(order)} modules, sha256 {sha}; "
          + ", ".join(f"{k} {v / 1e6:.2f} MB" for k, v in sizes.items()))
    return {"bytes": len(data), "files": sizes, "modules": len(order), "build": build_kind, "sha256": sha}


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
