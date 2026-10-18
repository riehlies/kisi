#!/usr/bin/env python3
"""Package the KiCad IPC-API plugin: dist/kisi-kicad-plugin.zip

Contents (folder "kisi"): plugin.json, kisi_open.py, requirements.txt,
resources/*.png and the bundled viewer dist/kisi.html.
Install: unzip into the KiCad plugins folder, e.g.
  Linux   ~/.local/share/KiCad/9.0/plugins/
  Windows %USERPROFILE%\\Documents\\KiCad\\9.0\\plugins\\
  macOS   ~/Documents/KiCad/9.0/plugins/
KiCad creates the Python environment (kicad-python) on first use.
"""
import os, sys, zipfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src = os.path.join(root, "kicad-plugin")
viewer = os.path.join(root, "dist", "kisi.html")
out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(root, "dist", "kisi-kicad-plugin.zip")
if not os.path.exists(viewer):
    sys.exit("dist/kisi.html fehlt – erst bündeln (tools/bundle.py)")
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for dp, _, fs in os.walk(src):
        for f in fs:
            if f.endswith((".pyc",)) or "__pycache__" in dp:
                continue
            p = os.path.join(dp, f)
            z.write(p, os.path.join("kisi", os.path.relpath(p, src)))
    z.write(viewer, "kisi/kisi.html")
print(out, os.path.getsize(out) // 1024, "kB")
