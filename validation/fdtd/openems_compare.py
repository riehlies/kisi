"""Code-to-code check with PML: kisi-fdtd with staircase metal ("conformal": false) and openEMS on
the same mesh for a few reference cases (one per group). Writes results/openems_compare.json.

usage: python3 openems_compare.py kisi-fdtd [case ids]
"""
import json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from run_refs import excitation  # noqa: E402

exe = sys.argv[1]
ids = sys.argv[2:] or ["A_s4_d300_ap800", "B_s8_L1L3_stub_d300", "D_s8_stitch_none", "E_s8_L1L8_p1000_round_g2"]
tmp = tempfile.mkdtemp()
rows, charts = [], []
for cid in ids:
    case = json.load(open(os.path.join(HERE, "via_cases", cid + ".json")))
    ex, mirror = excitation(case)
    case.update(conformal=False, excite=ex[:1])
    p = os.path.join(tmp, cid + ".json")
    json.dump(case, open(p, "w"))
    mp = p.replace(".json", ".mesh.json")
    open(mp, "w").write(subprocess.run([exe, p, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
    rk = json.loads(subprocess.run([exe, p, "--mesh", mp], capture_output=True, text=True, check=True).stdout)
    out = p.replace(".json", ".oe.json")
    subprocess.run([sys.executable, os.path.join(HERE, "openems_case.py"), p, mp, out], check=True, capture_output=True)
    ro = json.load(open(out))
    f = np.array(rk["freq"]) / 1e9
    sk = np.array([[complex(*x) for x in row] for row in rk["runs"][0]["s"]])
    so = np.array([[complex(*x) for x in row] for row in ro["runs"][0]["s"]])
    dk, do = 20 * np.log10(np.abs(sk)), 20 * np.log10(np.abs(so))
    thr = 2 if len(case["ports"]) == 4 else 1  # through port (pair: 3rd port = P far)
    m = np.maximum(dk[0], do[0]) > -30
    rows.append([cid, f"{rk['cells'] // 1000} k", f"{rk['runs'][0]['steps']}", f"{rk['runs'][0]['seconds']:.0f} s", f"{ro['seconds']:.0f} s",
                 f"{np.mean(np.abs(dk[thr] - do[thr])):.3f} / {np.max(np.abs(dk[thr] - do[thr])):.3f}",
                 f"{np.mean(np.abs(dk[0][m] - do[0][m])):.2f} / {np.max(np.abs(dk[0][m] - do[0][m])):.2f}" if m.any() else "–"])
    st = 2
    charts.append({"title": f"{cid}: |S{thr + 1}1| and |S11|", "x": [round(x, 3) for x in f[::st]], "xlabel": "frequency GHz", "ylabel": "dB", "ymin": -50, "ymax": 1,
                   "series": [{"name": f"kisi-fdtd S{thr + 1}1", "y": [round(float(v), 2) for v in dk[thr][::st]], "color": "var(--s1)"},
                              {"name": f"openEMS S{thr + 1}1", "y": [round(float(v), 2) for v in do[thr][::st]], "color": "var(--s2)", "dash": True},
                              {"name": "kisi-fdtd S11", "y": [round(float(v), 2) for v in dk[0][::st]], "color": "var(--s3)"},
                              {"name": "openEMS S11", "y": [round(float(v), 2) for v in do[0][::st]], "color": "var(--s4)", "dash": True}]})
    print(rows[-1], flush=True)
out = {"title": "Implementation: kisi-fdtd against openEMS on the same mesh",
       "text": ["Both tools with staircase metal on the mesh kisi-fdtd generates, PML in the outer 8 cells, the same lumped ports, Gaussian pulse and "
                "dielectric loss; port 1 excited. This checks the field updates, ports, PML and the geometry rules, not the accuracy of the metal model "
                "(the conformal model is checked against the analytic cavity and by mesh convergence)."],
       "cols": ["case", "cells", "kisi steps", "kisi time", "openEMS time", "Δ|S21| mean / max (dB)", "Δ|S11| mean / max where > −30 dB"],
       "rows": rows, "charts": charts}
json.dump(out, open(os.path.join(HERE, "results", "openems_compare.json"), "w"), indent=1)
print("results/openems_compare.json")
