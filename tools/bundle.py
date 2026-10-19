#!/usr/bin/env python3
"""Bundle the viewer into one self-contained HTML file.

Usage: bundle.py <index.html> <kisi_wasm.wasm> <kisi_sparse.wasm> <out.html> [key name pcb pro]...

* kisi_wasm.wasm   extraction core, base64
* kisi_sparse.wasm sparse solver for the field solver backend, base64
* the solver backend (web/solver/backend_forsten.js and the vendored upstream
  js_2d_fields ES modules it imports) is converted into one classic script
  (tools/esm_classic.py) so it runs in workers also when the page is opened
  from disk. Upstream code is unchanged apart from import/export statements.
* sample boards: gzip + base64
* viewer: web/index.html (markup), web/css/kisi.css, web/src/*.js in the order
  of web/src/manifest.json (one shared scope, concatenated), data/*.json as DATA
"""
import base64, gzip, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from solver_bundle import solver_bundle, upstream_rev as read_upstream_rev  # noqa: E402

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src, wasm, sparse, out = sys.argv[1:5]
rest = sys.argv[5:]

# --- solver backend as one classic script (tools/solver_bundle.py) -------------------
solver_src, n_modules = solver_bundle(root)

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

upstream_rev = read_upstream_rev(root)

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
html = html.replace("__SOLVER_SRC__", json.dumps(solver_src).replace("</", "<\\/"))
html = html.replace("__UPSTREAM_REV__", upstream_rev)
cargo = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
html = html.replace("__VERSION__", re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1))
html = html.replace("__SAMPLES__", json.dumps(samples))
os.makedirs(os.path.dirname(out) or ".", exist_ok=True)  # dist/ is not in the repository
open(out, "w", encoding="utf-8").write(html)
print(f"{out}: {len(html) / 1e6:.2f} MB, {n_modules} solver modules")
