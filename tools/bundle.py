#!/usr/bin/env python3
"""Bundle the viewer into one self-contained HTML file.

Usage: bundle.py <index.html> <kisi_wasm.wasm> <kisi_sparse.wasm> <out.html> [key name pcb pro]...

* kisi_wasm.wasm   extraction core, gzip + base64
* kisi_sparse.wasm sparse solver for the field solver backend, gzip + base64
* the solver backend (web/solver/backend_forsten.js and the vendored upstream
  js_2d_fields ES modules it imports) is converted into one classic script
  (tools/esm_classic.py) so it runs in workers also when the page is opened
  from disk (gzip + base64). Upstream code is unchanged apart from import/export statements.
* sample boards: gzip + base64, with web/samples/<key>.kisi.json (their board configuration) and
  <board>.notice.txt next to the board file (source and licence, tools/fetch_examples.sh)
* viewer: web/index.html (markup), web/fonts (embedded fonts), web/css/kisi.css, web/src/*.js in the order
  of web/src/manifest.json (one shared scope, concatenated), data/*.json as DATA (not corpus.json)
"""
import base64, gzip, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from solver_bundle import solver_bundle, upstream_rev as read_upstream_rev  # noqa: E402

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src, wasm, sparse, out = sys.argv[1:5]
rest = sys.argv[5:]

# --- solver backend as one classic script (tools/solver_bundle.py) -------------------
solver_src, n_modules = solver_bundle(root)

# gzip before base64: about a third of the size; the viewer unpacks with DecompressionStream.
# mtime=0: no timestamp in the gzip header, so two builds of the same commit are identical.
gzb64 = lambda b: base64.b64encode(gzip.compress(b, 9, mtime=0)).decode()

# --- samples ----------------------------------------------------------------------
samples = {}
for i in range(0, len(rest), 4):
    key, name, pcb, pro = rest[i:i + 4]
    gz = lambda p: gzb64(open(p, "rb").read())
    stem = os.path.basename(pcb).removesuffix(".kicad_pcb")
    samples[key] = {"name": name, "file": stem, "pcb": gz(pcb), "pro": gz(pro)}
    # the example's board configuration: power tree (PI_REG / PI_LOAD) with sources
    cfg = os.path.join(root, "web/samples", f"{key}.kisi.json")
    if os.path.exists(cfg):
        samples[key]["config"] = {"name": f"{stem}.kisi.json", "text": open(cfg, encoding="utf-8").read()}
    # source, licence and modification note (tools/fetch_examples.sh): embedded with the board
    notice = pcb.removesuffix(".kicad_pcb") + ".notice.txt"
    if os.path.exists(notice):
        samples[key]["notice"] = open(notice, encoding="utf-8").read()

upstream_rev = read_upstream_rev(root)

html = open(src, encoding="utf-8").read()
web = os.path.dirname(os.path.abspath(src))


# --- stylesheet and viewer scripts (web/css, web/src) -----------------------------
def inline_css(m):
    path = os.path.join(root, m.group(1))
    css = open(path, encoding="utf-8").read()
    # fonts (web/fonts): embedded as data URIs, so the page loads nothing from the web
    font = lambda u: "url(data:font/woff2;base64," + base64.b64encode(open(os.path.join(os.path.dirname(path), u.group(1)), "rb").read()).decode() + ")"
    css = re.sub(r"url\(([\w.-]+\.woff2)\)", font, css)
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
# (corpus.json is the board list of tools/corpus.py, not used by the viewer)
NOT_BUNDLED = {"corpus.json"}
data = {}
ddir = os.path.join(root, "data")
if os.path.isdir(ddir):
    for f in sorted(os.listdir(ddir)):
        if f.endswith(".json") and f not in NOT_BUNDLED:
            data[f[:-5]] = json.load(open(os.path.join(ddir, f), encoding="utf-8"))
html = html.replace("__DATA__", json.dumps(data, ensure_ascii=False).replace("</", "<\\/"))
html = html.replace("__WASM_GZ__", gzb64(open(wasm, "rb").read()))
html = html.replace("__SPARSE_GZ__", gzb64(open(sparse, "rb").read()))
html = html.replace("__SOLVER_GZ__", gzb64(solver_src.encode("utf-8")))
html = html.replace("__UPSTREAM_REV__", upstream_rev)
cargo = open(os.path.join(root, "Cargo.toml"), encoding="utf-8").read()
html = html.replace("__VERSION__", re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1))
html = html.replace("__SAMPLES__", json.dumps(samples))
os.makedirs(os.path.dirname(out) or ".", exist_ok=True)  # dist/ is not in the repository
# example boards: their credit lines at the top (full licence texts in SAMPLES[…].notice)
credits = [s["notice"].split("\n", 1)[0] for s in samples.values() if s.get("notice")]
if credits:
    html = html.replace("<!doctype html>\n", "<!doctype html>\n<!-- " + "\n     ".join(c.replace("--", "-") for c in credits) + " -->\n", 1)
open(out, "w", encoding="utf-8").write(html)
print(f"{out}: {len(html) / 1e6:.2f} MB, {n_modules} solver modules")
