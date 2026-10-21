"""Mesh convergence of a via with antipads: case A_s4_d300_ap800 (L1→L4 through a plane pair, drill
Ø0.3, antipad Ø0.8, four GND vias), port 1 excited, the fine cell `dfine` near the via varied
(dmax fixed), staircase and conformal metal. Reference: conformal at the finest cell. Writes
results/via_convergence.json.

usage: python3 via_convergence.py kisi-fdtd [cells, default 0.1 0.07 0.05 0.035 0.025]
"""
import json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
exe = sys.argv[1]
cells = [float(x) for x in sys.argv[2:]] or [0.1, 0.07, 0.05, 0.035, 0.025]
base = json.load(open(os.path.join(HERE, "via_cases", "A_s4_d300_ap800.json")))
er = max(l["er"] for l in base["layers"])
dmax = round(299792458.0 / base["fmax"] / er ** 0.5 / 20 * 1e3, 4)
tmp = tempfile.mkdtemp()
runs = {}
for cell in cells:
    for conf in (False, True):
        c = json.loads(json.dumps(base))
        c.update(excite=[0], conformal=conf)
        c["mesh"] = {"dfine": cell, "dmax": dmax, "ratio": 1.4}
        p = os.path.join(tmp, f"v_{cell}_{conf}.json")
        json.dump(c, open(p, "w"))
        r = json.loads(subprocess.run([exe, p], capture_output=True, text=True, check=True).stdout)
        s = np.array([[complex(*x) for x in row] for row in r["runs"][0]["s"]])
        runs[(cell, conf)] = (np.array(r["freq"]), s, r["cells"], r["runs"][0]["steps"], r["runs"][0]["seconds"])
        print(f"{cell} {'conformal' if conf else 'staircase'}: {r['cells'] // 1000} k cells, {r['runs'][0]['steps']} steps, {r['runs'][0]['seconds']:.0f} s", flush=True)

f, sref = runs[(cells[-1], True)][0], runs[(cells[-1], True)][1]
fg = f / 1e9
band = fg <= 30
rows = []
err = {False: [], True: []}
for cell in cells:
    row = [f"{cell:g}"]
    for conf in (False, True):
        _, s, n, st, sec = runs[(cell, conf)]
        d11 = np.max(np.abs(20 * np.log10(np.abs(s[0][band])) - 20 * np.log10(np.abs(sref[0][band]))))
        # S11 difference as a complex error relative to the reference (dB of |ΔS11|)
        e11 = 20 * np.log10(np.max(np.abs(s[0][band] - sref[0][band])))
        d21 = np.max(np.abs(20 * np.log10(np.abs(s[1][band])) - 20 * np.log10(np.abs(sref[1][band]))))
        err[conf].append(None if (cell == cells[-1] and conf) else round(float(np.max(np.abs(s[0][band] - sref[0][band]))), 4))
        row += [f"{n // 1000} k", f"{st}", f"{sec:.0f} s", "–" if (cell == cells[-1] and conf) else f"{e11:.1f}", "–" if (cell == cells[-1] and conf) else f"{d21:.3f}"]
    rows.append(row)
step = 2
pick = [cells[0], cells[2] if len(cells) > 2 else cells[-1], cells[-1]]
series = []
for cell in pick:
    for conf in (False, True):
        if cell == cells[-1] and not conf:
            continue
        s = runs[(cell, conf)][1]
        series.append({"name": f"{cell:g} mm {'conformal' if conf else 'staircase'}", "y": [round(float(v), 2) for v in 20 * np.log10(np.abs(s[0][::step]))],
                       "dash": not conf, "color": {cells[0]: "var(--s2)", pick[1]: "var(--s3)", cells[-1]: "var(--s1)"}[cell]})
out = {
    "title": "Metal model: mesh convergence of a via with antipads",
    "text": ["Case A_s4_d300_ap800: L1→L4 through the L2/L3 plane pair of the 4-layer stack, drill Ø0.3 mm, pads Ø0.6 mm, antipads Ø0.8 mm, "
             f"four GND vias on a 1 mm circle; port 1 excited, cell near the via varied, elsewhere at most {dmax} mm (λ/20 at 40 GHz). "
             f"Reference: conformal metal at {cells[-1]:g} mm. Errors up to 30 GHz: max |ΔS11| (complex difference, in dB) and max difference of |S21|.",
             ],
    "cols": ["cell (mm)", "staircase cells", "steps", "time", "|ΔS11| dB", "Δ|S21| dB", "conformal cells", "steps", "time", "|ΔS11| dB", "Δ|S21| dB"],
    "rows": rows,
    "charts": [
        {"title": "Error of S11 against the cell size (up to 30 GHz)", "x": cells, "logx": True, "logy": True, "xlabel": "cell mm", "ylabel": "max |ΔS11|",
         "series": [{"name": "staircase", "y": err[False], "dash": True, "marks": True}, {"name": "conformal", "y": err[True], "marks": True}],
         "note": "Difference to the conformal run at the finest cell (so the conformal curve ends one point earlier)."},
        {"title": "Return loss |S11|", "x": [round(x, 3) for x in fg[::step]], "series": series, "xlabel": "frequency GHz", "ylabel": "dB", "ymin": -50, "ymax": 0},
    ],
}
json.dump(out, open(os.path.join(HERE, "results", "via_convergence.json"), "w"), indent=1)
print("results/via_convergence.json")
