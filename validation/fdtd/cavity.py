"""Analytic check of the metal model: resonances of a cylindrical cavity (radius R, height h,
εr) without and with a centre post (radius a, a via shorting the two plates). TM0n0 modes:
  empty:  J0(k R) = 0
  post:   J0(k a) Y0(k R) − J0(k R) Y0(k a) = 0
k = 2π f √εr / c. The cavity is excited by a 50 Ω lumped port between the plates; the resonance is
the peak of Re(Z_in), Z_in = 50 (1 + S11) / (1 − S11) (a series inductance of the port does not
move it). kisi-fdtd runs on uniform meshes, staircase and conformal.

usage: python3 cavity.py kisi-fdtd-binary out.json [cell sizes, default 0.2 0.1 0.05]
"""
import json, math, os, subprocess, sys, tempfile
import numpy as np
from scipy.optimize import brentq
from scipy.special import j0, y0

C0 = 299792458.0
R, H, ER, TAND = 3.0, 0.5, 4.0, 0.03
A_POST = 0.3


def analytic(post):
    def f(k):
        if post:
            ka, kr = k * A_POST * 1e-3, k * R * 1e-3
            return j0(ka) * y0(kr) - j0(kr) * y0(ka)
        return j0(k * R * 1e-3)
    ks = np.linspace(10, 3000, 30000)
    v = [f(k) for k in ks]
    roots = [brentq(f, ks[i], ks[i + 1]) for i in range(len(ks) - 1) if v[i] * v[i + 1] < 0]
    return [k * C0 / (2 * math.pi * math.sqrt(ER)) for k in roots]


def case(cell, post, conformal):
    m = R + 0.4
    vias = [{"x": 0, "y": 0, "r": R, "z0": 0, "z1": H, "outer": True}]
    if post:
        vias.append({"x": 0, "y": 0, "r": A_POST, "z0": 0, "z1": H})
    return {
        "name": f"cavity_{'post' if post else 'empty'}_{cell}_{'conf' if conformal else 'stair'}",
        "domain": [-m, -m, 0, m, m, H], "board": [-m, -m, m, m],
        "layers": [{"z0": 0, "z1": H, "er": ER, "tand": TAND}],
        "sheets": [{"z": 0, "rect": [-m, -m, m, m]}, {"z": H, "rect": [-m, -m, m, m]}],
        "vias": vias,
        "ports": [{"x": 1.5, "y0": 0, "y1": 0, "z0": 0, "z1": H}],
        "fmax": 30e9, "mesh": {"dfine": cell, "dmax": cell}, "boundary": "pec", "conformal": conformal,
        "end_db": -60, "max_steps": 200000, "nf": 3000,
    }


def peaks(res):
    f = np.array(res["freq"])
    s = np.array([complex(*x) for x in res["runs"][0]["s"][0]])
    z = 50 * (1 + s) / (1 - s)
    re = z.real
    out = []
    for i in range(1, len(f) - 1):
        if re[i] > re[i - 1] and re[i] >= re[i + 1] and re[i] > 0.2 * re.max():
            # parabola through the three points
            d = (re[i - 1] - re[i + 1]) / (2 * (re[i - 1] - 2 * re[i] + re[i + 1]))
            out.append(f[i] + d * (f[1] - f[0]))
    return out


def main():
    exe, out = sys.argv[1], sys.argv[2]
    cells = [float(x) for x in sys.argv[3:]] or [0.2, 0.1, 0.05]
    rows = []
    tmp = tempfile.mkdtemp()
    for post in (False, True):
        ref = [f for f in analytic(post) if f < 28e9]
        for cell in cells:
            for conf in (False, True):
                c = case(cell, post, conf)
                p = os.path.join(tmp, c["name"] + ".json")
                json.dump(c, open(p, "w"))
                r = json.loads(subprocess.run([exe, p], capture_output=True, text=True, check=True).stdout)
                pk = peaks(r)
                f1 = min(pk, key=lambda x: abs(x - ref[0])) if pk else float("nan")
                err = (f1 / ref[0] - 1) * 100
                rows.append({"case": "post" if post else "empty", "cell": cell, "metal": "conformal" if conf else "staircase",
                             "f_analytic_GHz": ref[0] / 1e9, "f_fdtd_GHz": f1 / 1e9, "error_pct": err,
                             "cells": r["cells"], "steps": r["runs"][0]["steps"], "seconds": r["runs"][0]["seconds"]})
                print(f"{rows[-1]['case']:5} {cell:5} {rows[-1]['metal']:9}  analytic {ref[0]/1e9:.3f} GHz  fdtd {f1/1e9:.3f} GHz  {err:+.2f} %  ({r['runs'][0]['steps']} steps, {r['runs'][0]['seconds']:.0f} s)", flush=True)
    json.dump({"R_mm": R, "h_mm": H, "er": ER, "tand": TAND, "post_mm": A_POST, "rows": rows}, open(out, "w"), indent=1)


if __name__ == "__main__":
    main()
