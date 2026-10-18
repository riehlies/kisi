#!/usr/bin/env python3
"""Bundle the viewer into one self-contained HTML file.

Usage: bundle.py <index.html> <kisi_wasm.wasm> <kisi_sparse.wasm> <out.html> [key name pcb pro]...

* kisi_wasm.wasm   extraction core, base64
* kisi_sparse.wasm sparse solver for the field solver backend, base64
* ES modules of the solver backend (web/solver/backend_forsten.js and the
  vendored upstream js_2d_fields modules it imports) are embedded as source
  text; the page turns them into blob: modules at runtime. Upstream files are
  embedded unmodified except for their import specifiers.
* sample boards: gzip + base64
* viewer: web/index.html (markup), web/css/kisi.css, web/src/*.js in the order
  of web/src/manifest.json (one shared scope, concatenated), data/*.json as DATA
"""
import base64, gzip, json, os, re, sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src, wasm, sparse, out = sys.argv[1:5]
rest = sys.argv[5:]

# --- module graph -------------------------------------------------------------
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

# --- samples ----------------------------------------------------------------------
samples = {}
for i in range(0, len(rest), 4):
    key, name, pcb, pro = rest[i:i + 4]
    gz = lambda p: base64.b64encode(gzip.compress(open(p, "rb").read(), 9)).decode()
    samples[key] = {"name": name, "pcb": gz(pcb), "pro": gz(pro)}
    # optional example supply configuration (loads are assumptions)
    # optional example power tree (regulator chains and loads, values are
    # datasheet typicals or estimates, marked per load)
    tree = os.path.join(root, "web/samples", f"{key}_tree.json")
    if os.path.exists(tree):
        samples[key]["pi"] = json.load(open(tree, encoding="utf-8"))

upstream_rev = open(os.path.join(root, "third_party/UPSTREAM_REV")).read().strip() if os.path.exists(os.path.join(root, "third_party/UPSTREAM_REV")) else "unknown"

html = open(src, encoding="utf-8").read()
web = os.path.dirname(os.path.abspath(src))


# --- stylesheet and viewer scripts (web/css, web/src) -----------------------------
def inline_css(m):
    css = open(os.path.join(root, m.group(1)), encoding="utf-8").read()
    return "<style>\n" + css + "</style>"


html = re.sub(r"<!-- @css (\S+) -->\n<link [^>]*>", inline_css, html)


def inline_scripts(m):
    manifest = json.load(open(os.path.join(root, m.group(1)), encoding="utf-8"))
    parts = []
    for f in manifest["scripts"]:
        code = open(os.path.join(web, "src", f), encoding="utf-8").read()
        code = code.removeprefix('"use strict";\n')
        parts.append(f"// ---- src/{f}\n" + code)
    return '<script>\n"use strict";\n' + "\n".join(parts).replace("</script", "<\\/script") + "</script>"


html = re.sub(r"<!-- @scripts (\S+) -->.*?<!-- @end-scripts -->", inline_scripts, html, flags=re.S)

# --- data files (data/*.json): presets that can be updated without touching code
data = {}
ddir = os.path.join(root, "data")
if os.path.isdir(ddir):
    for f in sorted(os.listdir(ddir)):
        if f.endswith(".json"):
            data[f[:-5]] = json.load(open(os.path.join(ddir, f), encoding="utf-8"))
html = html.replace("__DATA__", json.dumps(data, ensure_ascii=False).replace("</", "<\\/"))
b64 = lambda p: base64.b64encode(open(p, "rb").read()).decode()
html = html.replace("__WASM_B64__", b64(wasm))
html = html.replace("__SPARSE_B64__", b64(sparse))
# </script> inside module sources would end the script tag: escape
mods_json = json.dumps(modules).replace("</", "<\\/")
html = html.replace("__MODULES__", mods_json)
html = html.replace("__UPSTREAM_REV__", upstream_rev)
html = html.replace("__SAMPLES__", json.dumps(samples))
open(out, "w", encoding="utf-8").write(html)
print(f"{out}: {len(html) / 1e6:.2f} MB, {len(modules)} solver modules")
