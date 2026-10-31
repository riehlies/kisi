"""Run a kisi-fdtd case (JSON) in openEMS on a given mesh: the same geometry (zero-thickness sheets
with round or slot holes and polygon copper, solid via cylinders, pads), the same dielectric loss
(conductivity at f_loss), lumped ports along z (also across y-directed traces, `x1`), lumped series
R–L–C elements, the same Gaussian excitation and boundary (PML in the outer `pml_cells` cells of the
mesh, or Mur). openEMS has no conformal metal (staircase): compare like for like with kisi-fdtd
cases that set "conformal": false on the same mesh, or use a refined mesh (openems_refs.py).

The geometry goes to openEMS as CSXCAD primitives (boxes, polygons, cylinders), so openEMS decides
itself which mesh edges are metal. Heights with polygon copper are passed as the union of their sheets
(shapely: clean polygons with interiors, see sheet_union). Ports and lumped elements sit on the mesh
lines nearest to their coordinates, as in kisi-fdtd. Whatever touches the domain boundary continues
through the PML to the mesh edge, as in kisi-fdtd.

Writes the S-parameters as JSON (one run per excited port).

usage: python3 openems_case.py case.json mesh.json out.json [--threads N]
"""
import json, math, os, sys, tempfile, time
import numpy as np
from CSXCAD import ContinuousStructure
from openEMS import openEMS

EPS0 = 8.8541878128e-12

case = json.load(open(sys.argv[1]))
mesh = json.load(open(sys.argv[2]))
out = os.path.abspath(sys.argv[3])  # openEMS changes the working directory
threads = int(sys.argv[sys.argv.index("--threads") + 1]) if "--threads" in sys.argv else 1
CAPS = os.environ.get("OEMS_CAPS", "1") != "0"  # PEC end caps on the lumped elements (openEMS' default; its ports always have them)


def disc(x, y, r, n=64, grow=1.0005):
    # pads: circumscribed and slightly larger, so that edges on the circle count as metal as in
    # kisi-fdtd; holes: radius r
    rr = r * grow / (math.cos(math.pi / n) if grow > 1 else 1.0)
    return [[x + rr * math.cos(2 * math.pi * k / n) for k in range(n)], [y + rr * math.sin(2 * math.pi * k / n) for k in range(n)]]


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


def sheet_union(sheets, dom):
    """the metal of all sheets at one height as kisi-fdtd defines it: the union over the sheets of
    rect ∩ even-odd(polys) − holes, clipped to the domain. Built with shapely, so the zero-width slits
    and cuts of KiCad's fractured fills vanish (CSXCAD counts a point exactly on a polygon edge as
    inside, which made such a slit on a mesh line a metal line across traces)."""
    from shapely.affinity import translate
    from shapely.geometry import Polygon, box, Point, LineString
    from shapely.ops import unary_union
    # kisi-fdtd's point tests on a boundary that falls on a mesh line: rectangles inclusive, polygon rings
    # half-open (left and bottom edges inside, right and top outside), holes strict. CSXCAD counts every
    # boundary point as inside; an offset far below any cell reproduces kisi-fdtd's choice.
    eps = 1e-6
    parts = []
    for s in sheets:
        r = s["rect"]
        g = box(r[0] - eps, r[1] - eps, r[2] + eps, r[3] + eps)
        if s.get("polys"):
            eo = None
            for ring in s["polys"]:
                q = Polygon(ring).buffer(0)
                eo = q if eo is None else eo.symmetric_difference(q)
            g = g.intersection(translate(eo, -eps, -eps))
        for h in s.get("holes", []):
            hole = Point(h[0], h[1]).buffer(h[2] - eps, quad_segs=32) if len(h) < 5 else LineString([(h[0], h[1]), (h[2], h[3])]).buffer(h[4] - eps, quad_segs=32)
            g = g.difference(hole)
        parts.append(g)
    u = unary_union(parts).intersection(box(dom[0], dom[1], dom[3], dom[4]))  # exact domain faces, for the extrusion
    return [g for g in getattr(u, "geoms", [u]) if g.geom_type == "Polygon" and not g.is_empty]


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

    # whatever touches the domain boundary continues through the PML to the mesh edge (as in kisi-fdtd)
    dom = case["domain"]
    edge = [mesh["x"][0], mesh["y"][0], mesh["x"][-1], mesh["y"][-1]]

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
    def extrude(pts):
        """a ring clipped to the domain: its parts on a domain face pushed out to the mesh edge"""
        def out_(p):
            q = list(p)
            for ax, lo, hi, elo, ehi in ((0, dom[0], dom[3], edge[0], edge[2]), (1, dom[1], dom[4], edge[1], edge[3])):
                if abs(p[ax] - lo) < 1e-9:
                    q[ax] = elo
                elif abs(p[ax] - hi) < 1e-9:
                    q[ax] = ehi
            return q

        def on_face(p, q):
            return any(abs(p[ax] - v) < 1e-9 and abs(q[ax] - v) < 1e-9 for ax, v in ((0, dom[0]), (0, dom[3]), (1, dom[1]), (1, dom[4])))

        res = []
        for k in range(len(pts)):
            p, q = pts[k], pts[(k + 1) % len(pts)]
            res.append(list(p))
            if on_face(p, q):  # detour p → p' → q' → q around the face segment
                res += [out_(p), out_(q)]
        return [[p[0] for p in res], [p[1] for p in res]]

    zs = sorted({round(s["z"], 9) for s in case.get("sheets", [])})
    for z in zs:
        at = [s for s in case["sheets"] if abs(s["z"] - z) < 1e-9]
        if any(s.get("polys") for s in at):
            # polygon copper at this height: the union of the sheets as clean polygons, their interiors as
            # non-metal of higher priority (nothing else but pads lies at this height)
            for g in sheet_union(at, dom):
                pec.AddPolygon(extrude(list(g.exterior.coords)[:-1]), "z", z, priority=12)
                for r in g.interiors:
                    pts = list(r.coords)[:-1]
                    hole_mat.AddPolygon([[p[0] for p in pts], [p[1] for p in pts]], "z", z, priority=13)
            continue
        for s in at:
            r = ext_rect(s["rect"])
            pec.AddBox([r[0], r[1], z], [r[2], r[3], z], priority=10)
            for h in s.get("holes", []):
                # planar polygon of a non-metal with higher priority: removes the sheet's edges inside
                # (zero-height cylinders are ignored by CSXCAD)
                # (kisi-fdtd keeps the boundary itself as metal: radius 1 nm smaller, see sheet_union)
                hole_mat.AddPolygon(hole_poly(list(h[:-1]) + [h[-1] - 1e-6]), "z", z, priority=20)
    for v in case.get("vias", []):
        assert not v.get("outer"), "outer walls: kisi-fdtd only"
        pec.AddCylinder([v["x"], v["y"], v["z0"]], [v["x"], v["y"], v["z1"]], v["r"], priority=30)
        for pz in v.get("pads", []):
            pec.AddPolygon(disc(v["x"], v["y"], v["pad_r"]), "z", pz, priority=30)

    def near(v, axis):
        lines = mesh["xyz"[axis]]
        return min(lines, key=lambda q: abs(q - v))

    def span(p):
        # across a trace along x (y0..y1 at x) or along y (x..x1 at y0), every coordinate on its nearest
        # mesh line as kisi-fdtd places ports and lumped elements: the mesher may move a line slightly
        # (merged lines), and openEMS drops a zero-width box that falls between two lines
        z0, z1 = near(p["z0"], 2), near(p["z1"], 2)
        if p.get("x1") is not None:
            y = near(p["y0"], 1)
            return [near(p["x"], 0), y, z0], [near(p["x1"], 0), y, z1]
        x = near(p["x"], 0)
        return [x, near(p["y0"], 1), z0], [x, near(p["y1"], 1), z1]

    for n, e in enumerate(case.get("lumped", [])):
        # series R–L–C; kisi-fdtd's zeros mean "absent" (l = 0: no inductance, c = 0: shorted capacitor)
        kw = {k.upper(): e[k] for k in ("r", "l", "c") if e.get(k, 0.0) > 0.0}
        if not kw:
            kw = {"R": 1e-6}
        el = csx.AddLumpedElement(f"lumped{n}", ny=2, caps=CAPS, LEtype=1, **kw)
        el.AddBox(*span(e), priority=5)
    ports = []
    for n, p in enumerate(case["ports"]):
        ports.append(fdtd.AddLumpedPort(n + 1, p.get("r", 50.0), *span(p), "z", excite=1.0 if n == ex else 0, priority=5))

    sim = tempfile.mkdtemp(prefix="oems_")
    t0 = time.time()
    fdtd.Run(sim, cleanup=True, verbose=int(os.environ.get("OEMS_VERBOSE", "0")), numThreads=threads, debug_pec=bool(os.environ.get("OEMS_DEBUG_PEC")))
    if os.environ.get("OEMS_KEEP"):  # keep the probe files for inspection
        print("simulation directory:", sim)
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
    t_end = 0.0
    for n, p in enumerate(case["ports"]):
        tu, u = probe(f"port_ut_{n + 1}")
        ti, i = probe(f"port_it_{n + 1}")
        t_end = max(t_end, float(tu[-1]))
        U, I = dft(tu, u), dft(ti, i)
        z = p.get("r", 50.0)
        waves.append(((U + z * I) / 2, (U - z * I) / 2))
    inc = waves[ex][0]
    s = [[(complex(x).real, complex(x).imag) for x in (w[1] / inc)] for w in waves]
    return {"excited": ex, "seconds": secs, "t_end": t_end, "freq": freq.tolist(), "s": s}


excite = case.get("excite") or list(range(len(case["ports"])))
runs = [run(ex) for ex in excite]
cells = len(mesh["x"]) * len(mesh["y"]) * len(mesh["z"])
json.dump({"name": case["name"], "tool": "openEMS", "threads": threads, "cells": cells, "freq": runs[0]["freq"], "runs": runs,
           "seconds": sum(r["seconds"] for r in runs), "excited": runs[0]["excited"], "s": runs[0]["s"]}, open(out, "w"))
print(f"{case['name']}: {sum(r['seconds'] for r in runs):.1f} s, {cells} cells, {len(runs)} excitation(s)")
