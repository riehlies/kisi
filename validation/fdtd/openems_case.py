"""Run a kisi-fdtd case (JSON) in openEMS on the same mesh: same metal (zero-thickness sheets with
holes, solid via cylinders, pads), same dielectric loss (conductivity at f_loss), same Gaussian
excitation and boundary (PML in the outer `pml_cells` cells of the mesh, or Mur). openEMS has no
conformal metal: compare with kisi-fdtd cases that set "conformal": false, or use a fine mesh. Writes the S-parameters as JSON.

usage: python3 openems_case.py case.json mesh.json out.json [--threads N]
"""
import json, math, os, sys, tempfile, time
import numpy as np
from CSXCAD import ContinuousStructure
from openEMS import openEMS

EPS0 = 8.8541878128e-12

case = json.load(open(sys.argv[1]))
mesh = json.load(open(sys.argv[2]))
out = sys.argv[3]
threads = int(sys.argv[sys.argv.index("--threads") + 1]) if "--threads" in sys.argv else 1

def run(ex):
    fmax = case["fmax"]
    f_loss = case.get("f_loss") or fmax / 2
    fdtd = openEMS(NrTS=case.get("max_steps", 400000), EndCriteria=10 ** (case.get("end_db", -50) / 10))
    fdtd.SetGaussExcite(fmax / 2, fmax / 2)
    bnd = case.get("boundary", "pml")
    npml = case.get("pml_cells", 8)
    fdtd.SetBoundaryCond([f"PML_{npml}" if bnd == "pml" else "MUR" if bnd == "mur" else "PEC"] * 6)
    csx = ContinuousStructure()
    fdtd.SetCSX(csx)
    grid = csx.GetGrid()
    grid.SetDeltaUnit(1e-3)
    for a in "xyz":
        grid.SetLines(a, mesh[a])

    def disc(x, y, r, n=64, grow=1.0005):
        # pads: circumscribed and slightly larger, so that edges on the circle count as metal as in
        # kisi-fdtd; holes: radius r
        rr = r * grow / (math.cos(math.pi / n) if grow > 1 else 1.0)
        pts = [[x + rr * math.cos(2 * math.pi * k / n) for k in range(n)], [y + rr * math.sin(2 * math.pi * k / n) for k in range(n)]]
        return pts

    def hole_poly(h, n=128):
        # circle [cx, cy, r] or slot [x0, y0, x1, y1, r] (stadium)
        if len(h) < 5:
            return disc(h[0], h[1], h[2], n, 1.0)
        x0, y0, x1, y1, r = h
        a = math.atan2(y1 - y0, x1 - x0)
        pts = []
        for k in range(n // 2 + 1):  # half circle around the end (x1, y1)
            t = a - math.pi / 2 + math.pi * k / (n // 2)
            pts.append((x1 + r * math.cos(t), y1 + r * math.sin(t)))
        for k in range(n // 2 + 1):  # and around the start
            t = a + math.pi / 2 + math.pi * k / (n // 2)
            pts.append((x0 + r * math.cos(t), y0 + r * math.sin(t)))
        return [[p[0] for p in pts], [p[1] for p in pts]]

    # whatever touches the domain boundary continues through the PML to the mesh edge (as in kisi-fdtd)
    dom = case["domain"]
    def ext(v, axis):
        lo, hi = dom[axis], dom[axis + 3]
        lines = mesh["xyz"[axis]]
        if abs(v - lo) < 1e-9 or v < lo:
            return lines[0]
        if abs(v - hi) < 1e-9 or v > hi:
            return lines[-1]
        return v
    def ext_rect(r):
        return [ext(r[0], 0), ext(r[1], 1), ext(r[2], 0), ext(r[3], 1)]
    b = ext_rect(case["board"])
    for n, l in enumerate(case["layers"]):
        kappa = 2 * math.pi * f_loss * EPS0 * l["er"] * l.get("tand", 0.0)
        m = csx.AddMaterial(f"layer{n}", epsilon=l["er"], kappa=kappa)
        m.AddBox([b[0], b[1], l["z0"]], [b[2], b[3], l["z1"]], priority=1)
    pec = csx.AddMetal("pec")
    hole_mat = csx.AddMaterial("hole", epsilon=1.0)
    for s in case.get("sheets", []):
        r = ext_rect(s["rect"])
        pec.AddBox([r[0], r[1], s["z"]], [r[2], r[3], s["z"]], priority=10)
        for h in s.get("holes", []):
            # planar polygon of a non-metal with higher priority: removes the sheet's edges inside
            # (zero-height cylinders are ignored by CSXCAD)
            hole_mat.AddPolygon(hole_poly(h), "z", s["z"], priority=20)
    for v in case.get("vias", []):
        assert not v.get("outer"), "outer walls: kisi-fdtd only"
        pec.AddCylinder([v["x"], v["y"], v["z0"]], [v["x"], v["y"], v["z1"]], v["r"], priority=30)
        for pz in v.get("pads", []):
            pec.AddPolygon(disc(v["x"], v["y"], v["pad_r"]), "z", pz, priority=30)
    ports = []
    for n, p in enumerate(case["ports"]):
        ports.append(fdtd.AddLumpedPort(n + 1, p.get("r", 50.0), [p["x"], p["y0"], p["z0"]], [p["x"], p["y1"], p["z1"]], "z", excite=1.0 if n == ex else 0, priority=5))

    sim = tempfile.mkdtemp(prefix="oems_")
    t0 = time.time()
    fdtd.Run(sim, cleanup=True, verbose=int(os.environ.get("OEMS_VERBOSE", "0")), numThreads=threads)
    secs = time.time() - t0
    nf = case.get("nf", 201)
    freq = np.array([fmax * (i + 1) / nf for i in range(nf)])
    # S-parameters from the port probes (sample times as written; openEMS samples sparsely):
    # a = (U + Z·I)/2, b = (U − Z·I)/2 with I positive from the port into the structure
    def probe(name):
        d = np.loadtxt(os.path.join(sim, name), comments="%")
        return d[:, 0], d[:, 1]
    def dft(t, x):
        return np.array([np.trapezoid(x * np.exp(-2j * np.pi * fk * t), t) for fk in freq])
    waves = []
    for n, p in enumerate(case["ports"]):
        tu, u = probe(f"port_ut_{n + 1}")
        ti, i = probe(f"port_it_{n + 1}")
        U, I = dft(tu, u), dft(ti, i)
        z = p.get("r", 50.0)
        waves.append(((U + z * I) / 2, (U - z * I) / 2))
    inc = waves[ex][0]
    s = [[(complex(x).real, complex(x).imag) for x in (w[1] / inc)] for w in waves]
    return {"excited": ex, "seconds": secs, "freq": freq.tolist(), "s": s}


excite = case.get("excite") or list(range(len(case["ports"])))
runs = [run(ex) for ex in excite]
cells = len(mesh["x"]) * len(mesh["y"]) * len(mesh["z"])
json.dump({"name": case["name"], "tool": "openEMS", "threads": threads, "cells": cells, "freq": runs[0]["freq"], "runs": runs,
           "seconds": sum(r["seconds"] for r in runs), "excited": runs[0]["excited"], "s": runs[0]["s"]}, open(out, "w"))
print(f"{case['name']}: {sum(r['seconds'] for r in runs):.1f} s, {cells} cells, {len(runs)} excitation(s)")
