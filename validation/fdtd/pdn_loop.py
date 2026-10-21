"""Group H (PDN loop): loop inductance Im(Z11)/ω of the pin field to one capacitor at 2–20 mm, and
the change with distance against the plane-pair spreading inductance (radial, valid for D ≪ board
width) and against the finite-board cavity model (cavity_model.zmatrix),
  ΔL = μ0·d_eff/π · ln(D2/D1),
d_eff = the PWR plane's distance to the GND planes above and below in parallel (0.35 and 0.7 mm in
the 8-layer stack: 0.233 mm), D = distance between the pin-field centre and the capacitor.

usage: python3 pdn_loop.py   → results/pdn_loop.json
"""
import glob, json, math, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from cavity_model import zmatrix  # noqa: E402

MU0 = 4e-7 * math.pi
rows = []
for cp in sorted(glob.glob(os.path.join(HERE, "via_cases", "H_*.json"))):
    c = json.load(open(cp))
    p = os.path.join(REF, c["name"], "kisi.s1p")
    if not os.path.exists(p):
        continue
    f, S = sparam.read_touchstone(p)
    Z = 1.0 * (1 + S[:, 0, 0]) / (1 - S[:, 0, 0])
    L = Z.imag / (2 * math.pi * f)
    k = int(np.argmin(np.abs(f - 50e6)))
    k2 = int(np.argmin(np.abs(f - 100e6)))
    lump = c.get("lumped", [{}])[0]
    D = lump.get("x", 0) + 0.5  # capacitor centre (the GND pad is 0.5 mm left of it)
    rows.append({"case": c["name"], "dist_mm": c["meta"]["params"]["distance_mm"], "D_mm": round(D, 2),
                 "L50_nH": round(float(L[k]) * 1e9, 3), "L100_nH": round(float(L[k2]) * 1e9, 3), "R50_mOhm": round(float(Z[k].real) * 1e3, 1)})
rows.sort(key=lambda r: r["D_mm"])
zc = json.load(open(os.path.join(HERE, "via_cases", "H_s8_cap_2.json")))["meta"]["copper_z"]
d1, d2 = zc[1] - zc[3], zc[3] - zc[4]
d_eff = d1 * d2 / (d1 + d2)
# reduced model: finite rectangular plane pair (double cosine series, height d_eff) with a square
# port at the pin field (2.4 mm, the pin pitch area) and the capacitor's vias as a shorted port
# (0.2 mm); the vertical parts (pins, capacitor body) are the same for every distance and drop out of ΔL
f50 = np.array([50e6])
for r in rows:
    c = json.load(open(os.path.join(HERE, "via_cases", r["case"] + ".json")))
    bx = c["board"]
    a, b = (bx[2] - bx[0]) * 1e-3, (bx[3] - bx[1]) * 1e-3
    lump = c["lumped"][0]
    ports = [((0.0 - bx[0]) * 1e-3, (0.0 - bx[1]) * 1e-3), ((lump["x"] + 0.5 - bx[0]) * 1e-3, (0.0 - bx[1]) * 1e-3)]
    Z = zmatrix(f50, a, b, d_eff * 1e-3, 4.0, 0.0, 50e6, ports, 2.4e-3, M=200, N=60)
    Z2 = zmatrix(f50, a, b, d_eff * 1e-3, 4.0, 0.0, 50e6, ports, 0.2e-3, M=200, N=60)
    zin = Z[0, 0, 0] - Z[0, 0, 1] * Z[0, 1, 0] / Z2[0, 1, 1]
    r["L_plane_model_nH"] = zin.imag / (2 * math.pi * 50e6) * 1e9
for r in rows:
    r["dL_3d_nH"] = round(r["L50_nH"] - rows[0]["L50_nH"], 3)
    r["dL_formula_nH"] = round(MU0 * d_eff * 1e-3 / math.pi * math.log(r["D_mm"] / rows[0]["D_mm"]) * 1e9, 3)
    r["dL_model_nH"] = round(r["L_plane_model_nH"] - rows[0]["L_plane_model_nH"], 3)
    r["L_plane_model_nH"] = round(r["L_plane_model_nH"], 3)
out = {"d_eff_mm": round(d_eff, 4), "rows": rows}
json.dump(out, open(os.path.join(HERE, "results", "pdn_loop.json"), "w"), indent=1)
for r in rows:
    print(r)
