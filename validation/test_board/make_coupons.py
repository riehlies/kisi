#!/usr/bin/env python3
"""Measurement coupons for kisi (validation/test_board/README.md).

Per board: a KiCad 8 board (boards/<board>.kicad_pcb) with its project (.kicad_pro, net classes 50Z
and 100Z_Diff) and kisi configuration (.kisi.json, stackup values with sources); per coupon one or
more kisi-fdtd cases (fdtd/<board>/<case>.json, the format of validation/fdtd/via_cases.py, run with
validation/fdtd/run_refs.py).

* Trace widths come from kisi itself: a small probe board per cross-section (the same writer as the
  coupons), solved with the 2D field solver of the Node CLI (kisi.mjs) when it is available, else
  with kisi's formulas (--solver formula). The result is stored in boards/<board>.design.json.
* Every coupon is described once (traces, vias, antipads, pours, ports in coupon coordinates); the
  KiCad board and the kisi-fdtd cases are written from that one description.
* Stackups (stackups.json) and connector data (connectors.json) carry their sources. The end-launch
  footprint is a PLACEHOLDER until checked against the vendor drawing (connectors.json).

Coordinates: mm, x along the coupons, y up (KiCad's y points down: converted on output). Coupons are
rows that run from the left to the right board edge; a board is a stack of rows, centred on x = 0,
with a stepped outline where rows of different length meet.

usage: python3 make_coupons.py [--boards jlc4_a,jlc4_b,jlc6_a,jlc6_b,aisler4_a]
                               [--kisi KISI] [--kisi-mjs KISI.MJS] [--solver auto|field|formula]
                               [--pair-pitch 12.7] [--check] [--fdtd-exe KISI_FDTD]
"""
import argparse, datetime, json, math, os, shutil, subprocess, sys, tempfile, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "validation", "fdtd"))
import via_cases as vc  # noqa: E402  (Builder, copper_z, ring: the 3D case format)
import kisi_board as kb  # noqa: E402  (layer names, circle and stadium outlines)

STK = json.load(open(os.path.join(HERE, "stackups.json")))
CON = json.load(open(os.path.join(HERE, "connectors.json")))
LAUNCH = CON["swm_narrow"]
FP = LAUNCH["footprint"]
LAY = CON["layout"]
FEED = LAY["feed"]              # board edge to the reference plane of a coupon
FEED4 = LAY["feed_4port"]       # the same for the 4-port pair coupons
# traces leave the launch straight until they are clear of the connector's mounting holes
X_CLEAR = max(FP["launch_length"] + 0.3, max(h["x"] + h["d"] / 2 for h in FP["mount_holes"]) + 0.5)
ZONE_INSET = 0.25               # zone outlines stay this far inside the board edge
FENCE_OFF = 0.4                 # fence via centre from the pour edge
WALL_EDGE = 0.9                 # no wall via closer than this to a left/right board edge
NS = uuid.UUID("6c1a0a52-5d8e-4f39-9a58-0d4c0d1e2b77")
TODAY = datetime.date.today().isoformat()


# --- stackup ------------------------------------------------------------------------------
class Stack:
    def __init__(self, key):
        d = STK["stackups"][key]
        self.key, self.d = key, d
        self.cu = d["copper"]
        self.diel = d["dielectrics"]
        self.n = len(self.cu)
        self.planes = d["planes"]
        self.signal = d["signal"]
        self.mask = d["mask"]
        self.names = kb.layer_names(self.n)
        self.vc_key = "tb_" + key
        vc.STACKUPS[self.vc_key] = {"title": d["title"], "diel": [(x["thickness"], x["er"], x["tand"]) for x in self.diel], "planes": list(self.planes)}
        self.z = vc.copper_z(vc.STACKUPS[self.vc_key])  # zero-thickness copper, as the 3D cases

    def thickness(self):
        return sum(self.cu) + sum(x["thickness"] for x in self.diel)

    def refs(self, layer):
        """reference planes (above, below) of a signal layer"""
        up = [p for p in self.planes if p < layer]
        dn = [p for p in self.planes if p > layer]
        return (max(up) if up else None), (min(dn) if dn else None)

    def port_ref(self, layer):
        """plane a lumped port of a trace on `layer` returns to: the nearer one"""
        a, b = self.refs(layer)
        if a is None or b is None:
            return a if b is None else b
        return a if self.z[a] - self.z[layer] <= self.z[layer] - self.z[b] else b

    def h_ref(self, layer):
        r = self.port_ref(layer)
        return abs(self.z[layer] - self.z[r])

    def dw_thick(self, layer):
        """Wheeler's width increase that represents the copper thickness on a zero-thickness strip"""
        t, h = self.cu[layer], self.h_ref(layer)
        return t / math.pi * (1 + math.log(2 * h / t))


# --- coupon description ----------------------------------------------------------------------
class Coupon:
    """One row of the board, from the left (x = -X/2) to the right board edge (x = +X/2),
    height H. Geometry in local coordinates; ports are end-launch connectors at the edges."""

    def __init__(self, cid, group, title, X, H, validates, refs=(), params=None):
        self.cid, self.group, self.title, self.X, self.H = cid, group, title, X, H
        self.validates, self.refs, self.params = validates, list(refs), dict(params or {})
        self.traces, self.vias, self.holes, self.pours, self.islands, self.ports = [], [], [], [], [], []
        self.windows = []
        self.cx = self.cy = 0.0
        self.nets = []

    # primitives
    def trace(self, layer, pts, w, net):
        self.traces.append({"layer": layer, "pts": [tuple(p) for p in pts], "w": w, "net": net})
        if net not in self.nets:
            self.nets.append(net)

    def via(self, x, y, l0, l1, net, pads=None, role="sig", drill=None, pad=None):
        v = LAY["signal_via"] if role == "sig" else LAY["gnd_via"]
        self.vias.append({"x": x, "y": y, "l0": l0, "l1": l1, "net": net, "pads": pads, "role": role,
                          "drill": drill or v["drill"], "pad": pad or v["pad"]})

    def hole(self, layers, shape, name=""):
        self.holes.append({"layers": list(layers), "shape": tuple(shape), "name": name})

    def pour(self, layer, rect, net="GND"):
        x0, y0, x1, y1 = rect
        if x1 - x0 > 0.05 and y1 - y0 > 0.05:
            self.pours.append({"layer": layer, "rect": (min(x0, x1), min(y0, y1), max(x0, x1), max(y0, y1)), "net": net})

    def port(self, side, y, layer, net, w, gap, name):
        """end-launch connector at the left (side -1) or right (+1) edge"""
        self.ports.append({"side": side, "y": y, "layer": layer, "net": net, "w": w, "gap": gap, "name": name})

    def window(self, name, x0, x1, half_y=None, excite=None, mirror=None, dfine=0.035, fmax=40e9, note="", group=None, extra=None):
        self.windows.append({"name": name, "x0": x0, "x1": x1, "half_y": half_y, "excite": excite, "mirror": mirror,
                             "dfine": dfine, "fmax": fmax, "note": note, "group": group or self.group, "extra": extra or {}})


# --- design (trace widths from kisi) ----------------------------------------------------------
class Kisi:
    def __init__(self, exe, mjs, solver):
        self.exe, self.mjs = exe, mjs
        if solver == "auto":
            solver = "field" if (mjs and os.path.exists(mjs) and shutil.which("node")) else "formula"
        self.solver = solver
        self.tmp = tempfile.mkdtemp(prefix="coupon_probe_")
        self.calls = 0

    def run(self, args, field=None, timeout=600):
        use_field = (self.solver == "field") if field is None else field
        cmd = (["node", self.mjs] if use_field else [self.exe]) + args
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        self.calls += 1
        if r.returncode != 0:
            raise RuntimeError(f"kisi failed: {' '.join(cmd)}\n{r.stderr[:500]}")
        return json.loads(r.stdout)

    def z(self, stk, layer, w, s=None, gap=None, field=None):
        """impedance kisi reports for a 20 mm probe line: Z0 (50Z class) or Zdiff (pair, 100Z_Diff)"""
        b = Board("probe", stk, [], probe=True)
        c = Coupon("P", "probe", "probe", 20.0, 10.0, "")
        nets = ["SE_PROBE"] if s is None else ["DP_PROBE_P", "DP_PROBE_N"]
        ys = [0.0] if s is None else [(w + s) / 2, -(w + s) / 2]
        for net, y in zip(nets, ys):
            c.trace(layer, [(-10.0, y), (10.0, y)], w, net)
        if gap is not None:
            c.pour(layer, (-10.0, w / 2 + gap, 10.0, 4.7))
            c.pour(layer, (-10.0, -4.7, 10.0, -w / 2 - gap))
        b.add(c)
        for k, (net, y) in enumerate(zip(nets, ys)):
            b.simple_fp(f"U{k + 1}", -10.0, y, layer, net, w)
            b.simple_fp(f"J{k + 1}", 10.0, y, layer, net, w)
        b.design = {"w50": {str(layer): w}, "diff": {str(layer): {"w": w, "s": s or 0.2}}}
        path = os.path.join(self.tmp, "probe.kicad_pcb")
        b.write_kicad(path)
        d = self.run(["net", path, nets[0], "--no-config"], field=field)
        if field or (field is None and self.solver == "field"):
            if not (d.get("field_solver") or {}).get("cross_sections_field"):
                raise RuntimeError("field solver not used on the probe")
        return float(d["impedance"]["mean"])

    def solve(self, f, target, lo, hi, increasing=False, tol=0.002, field=None):
        """x with f(x) = target, f monotone; geometric bisection on the formula, then secant
        steps with the solver in use"""
        g = lambda x: f(x, False) - target
        a, b = lo, hi
        for _ in range(40):
            m = math.sqrt(a * b)
            if (g(m) > 0) != increasing:
                a = m
            else:
                b = m
            if b / a < 1.0005:
                break
        x = math.sqrt(a * b)
        if (self.solver == "field") if field is None else field:
            x0, z0 = x, f(x, True) - target
            x1 = x * (1.03 if (z0 > 0) != increasing else 0.97)
            for _ in range(6):
                z1 = f(x1, True) - target
                if abs(z1) < tol * target or abs(z1 - z0) < 1e-6:
                    break
                x0, x1, z0 = x1, x1 - z1 * (x1 - x0) / (z1 - z0), z1
                x1 = min(max(x1, lo), hi)
            x = x1
        return x


def design(stk, ks, pair_pitch):
    """trace widths and gaps for this stackup; rounded to 5 µm (what the CAM data resolve)"""
    r5 = lambda v: round(round(v / 0.005) * 0.005, 3)
    out = {"solver": ks.solver, "kisi": os.path.basename(ks.mjs if ks.solver == "field" else ks.exe), "w50": {}, "diff": {}, "notes": []}
    for layer in stk.signal:
        mirror = stk.n - 1 - layer
        if str(mirror) in out["w50"] and stk.d["dielectrics"] == stk.d["dielectrics"][::-1]:
            out["w50"][str(layer)] = out["w50"][str(mirror)]
            continue
        w = ks.solve(lambda x, fld: ks.z(stk, layer, x, field=fld), 50.0, 0.04, 2.0)
        out["w50"][str(layer)] = r5(w)
    # launch: grounded coplanar section at the connector (the connector ground contacts the top
    # copper): gap 0.3 mm, width for 50 Ω; if that width is below the pin diameter, the pin
    # diameter and the gap for 50 Ω
    g0 = 0.3
    wl = ks.solve(lambda x, fld: ks.z(stk, 0, x, gap=g0, field=fld), 50.0, 0.04, 2.0)
    if wl < LAUNCH["pin_diameter"]:
        wl = LAUNCH["pin_diameter"]
        g0 = ks.solve(lambda x, fld: ks.z(stk, 0, wl, gap=x, field=fld), 50.0, 0.06, 1.5, increasing=True)
    out["launch"] = {"w": r5(wl), "gap": r5(g0)}
    # tightly coupled 100 Ω pair (outer layer, and the first inner signal layer), spacing fixed
    for layer, s in [(0, 0.2)] + [(L, 0.2) for L in stk.signal if 0 < L < stk.n - 1][:1]:
        w = ks.solve(lambda x, fld: ks.z(stk, layer, x, s=s, field=fld), 100.0, 0.04, 1.5)
        out["diff"][str(layer)] = {"w": r5(w), "s": s}
    # pair at the via pitch (1.0 mm): width for 100 Ω differential with the edge gap 1.0 - w
    for layer in (0, stk.n - 1):
        if str(stk.n - 1 - layer) in out.get("pair_via", {}) and layer == stk.n - 1:
            out.setdefault("pair_via", {})[str(layer)] = out["pair_via"][str(stk.n - 1 - layer)]
            continue
        w = ks.solve(lambda x, fld: ks.z(stk, layer, x, s=1.0 - x, field=fld), 100.0, 0.08, 0.7)
        out.setdefault("pair_via", {})[str(layer)] = {"w": r5(w), "s": round(1.0 - r5(w), 3), "pitch": 1.0}
    # CPWG coupon: the gap of the LibreVNA RF class (0.18 mm, 3D reference T_vna6_cpwg on the same
    # 7628 top prepreg), width for 50 Ω (0.35 mm, the LibreVNA width, gives 45-48 Ω here with
    # mask and 35 µm copper)
    w = ks.solve(lambda x, fld: ks.z(stk, 0, x, gap=0.18, field=fld), 50.0, 0.04, 1.0)
    out["cpwg"] = {"w": r5(w), "gap": 0.18, "note": "gap of the LibreVNA RF class (T_vna6_cpwg), width for 50 Ω"}
    out["pair_pitch"] = pair_pitch
    return out


def report_z(stk, ks, des):
    """impedance of every designed cross-section with both kisi solvers (for the README)"""
    rows = []
    have_field = ks.mjs and os.path.exists(ks.mjs) and shutil.which("node")

    def both(label, layer, w, s=None, gap=None, target=50.0):
        zf = ks.z(stk, layer, w, s=s, gap=gap, field=False)
        zd = ks.z(stk, layer, w, s=s, gap=gap, field=True) if have_field else None
        rows.append({"what": label, "layer": stk.names[layer], "w_mm": w, "s_mm": s, "gap_mm": gap, "target_ohm": target,
                     "z_formula": round(zf, 2), "z_field": round(zd, 2) if zd else None,
                     "dev_formula_pct": round((zf / target - 1) * 100, 1), "dev_field_pct": round((zd / target - 1) * 100, 1) if zd else None})
    for L, w in des["w50"].items():
        both("50 Ω line" + (" (microstrip)" if int(L) in (0, stk.n - 1) else " (stripline)"), int(L), w)
    both("launch (grounded coplanar)", 0, des["launch"]["w"], gap=des["launch"]["gap"])
    both("CPWG coupon (LibreVNA gap 0.18 mm)", 0, des["cpwg"]["w"], gap=des["cpwg"]["gap"])
    for L, p in des["diff"].items():
        both("100 Ω coupled pair", int(L), p["w"], s=p["s"], target=100.0)
    for L, p in des["pair_via"].items():
        both("pair at the 1.0 mm via pitch", int(L), p["w"], s=p["s"], target=100.0)
    return rows


# --- polygons --------------------------------------------------------------------------------
def rect_pts(r):
    x0, y0, x1, y1 = r
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def shape_ring(shape, n=48):
    """closed outline of a hole (counter-clockwise, last point = first); rectangles carry the
    mid-points of their vertical sides, where the cut of a fractured fill attaches"""
    k = shape[0]
    if k == "circle":
        return kb.circle(shape[1], shape[2], shape[3], n)
    if k == "slot":
        return kb.stadium(list(shape[1:]))
    x0, y0, x1, y1 = shape[1:]
    ym = (y0 + y1) / 2
    return [(x0, y0), (x1, y0), (x1, ym), (x1, y1), (x0, y1), (x0, ym), (x0, y0)]


def shape_bbox(shape):
    k = shape[0]
    if k == "circle":
        return (shape[1] - shape[3], shape[2] - shape[3], shape[1] + shape[3], shape[2] + shape[3])
    if k == "slot":
        x0, y0, x1, y1, r = shape[1:]
        return (min(x0, x1) - r, min(y0, y1) - r, max(x0, x1) + r, max(y0, y1) + r)
    return tuple(shape[1:])


def inset_rectilinear(poly, d):
    """inset a counter-clockwise rectilinear polygon by d (every edge moves inwards)"""
    n = len(poly)
    lines = []
    for i in range(n):
        (x0, y0), (x1, y1) = poly[i], poly[(i + 1) % n]
        if abs(y1 - y0) < 1e-9:  # horizontal: inside is to the left of the direction
            lines.append(("h", y0 + (d if x1 > x0 else -d)))
        else:
            lines.append(("v", x0 - (d if y1 > y0 else -d)))
    out = []
    for i in range(n):
        a, b = lines[i - 1], lines[i]
        x = a[1] if a[0] == "v" else b[1]
        y = a[1] if a[0] == "h" else b[1]
        out.append((x, y))
    return out


def fractured(outline, holes):
    """polygon with holes as one outline (KiCad's fractured fill): each hole is joined by a
    horizontal cut to the left edge (hole centre x < 0) or the right edge. `outline`:
    counter-clockwise rectilinear polygon (down the left side, up the right side)."""
    out = []
    n = len(outline)
    for i in range(n):
        p, q = outline[i], outline[(i + 1) % n]
        out.append(p)
        if abs(p[0] - q[0]) > 1e-9:
            continue
        down = q[1] < p[1]
        lo, hi = min(p[1], q[1]), max(p[1], q[1])
        sel = []
        for h in holes:
            bb = shape_bbox(h)
            xc, yc = (bb[0] + bb[2]) / 2, (bb[1] + bb[3]) / 2
            if ((xc < 0) == down) and lo + 1e-6 < yc < hi - 1e-6:
                sel.append((yc, shape_ring(h)[:-1]))
        sel.sort(key=lambda t: -t[0] if down else t[0])
        for yc, ring in sel:
            # the ring point nearest to this edge, close to the hole's centre line
            k = min(range(len(ring)), key=lambda j: abs(ring[j][0] - p[0]) + 10 * abs(ring[j][1] - yc))
            r = ring[k:] + ring[:k]
            out += [(p[0], r[0][1]), r[0]] + r[:0:-1] + [r[0], (p[0], r[0][1])]  # the hole runs clockwise
    return out


# --- board ---------------------------------------------------------------------------------------
class Board:
    def __init__(self, name, stk, coupons, title="", probe=False):
        self.name, self.stk, self.title, self.probe = name, stk, title, probe
        self.coupons = []
        self.fps = []        # simple footprints (probe)
        self.design = {}
        self.texts = []
        for c in coupons:
            self.add(c)

    def add(self, c):
        self.coupons.append(c)

    def simple_fp(self, ref, x, y, layer, net, w):
        self.fps.append({"ref": ref, "x": x, "y": y, "layer": layer, "net": net, "w": w})

    # layout
    def place(self, info_row=True):
        y = 0.0
        for c in self.coupons:
            c.cx, c.cy = 0.0, y - c.H / 2
            y -= c.H
        self.bottom = y
        self.top = 0.0

    def outline(self):
        """counter-clockwise stepped outline from the rows"""
        rows = [(c.cy + c.H / 2, c.cy - c.H / 2, c.X / 2) for c in self.coupons]
        left, right = [], []
        for (yt, yb, hx) in rows:  # down the left side
            left += [(-hx, yt), (-hx, yb)]
        for (yt, yb, hx) in reversed(rows):  # up the right side
            right += [(hx, yb), (hx, yt)]
        pts = left + right
        # merge collinear points
        out = []
        for p in pts:
            if out and abs(out[-1][0] - p[0]) < 1e-9 and abs(out[-1][1] - p[1]) < 1e-9:
                continue
            out.append(p)
        clean = []
        m = len(out)
        for i in range(m):
            a, b, c = out[i - 1], out[i], out[(i + 1) % m]
            if (abs(a[0] - b[0]) < 1e-9 and abs(b[0] - c[0]) < 1e-9) or (abs(a[1] - b[1]) < 1e-9 and abs(b[1] - c[1]) < 1e-9):
                continue
            clean.append(b)
        return clean

    def width(self):
        return max(c.X for c in self.coupons)

    # global geometry
    def gather(self):
        """all primitives in board coordinates, walls and launches included"""
        G = {"traces": [], "vias": [], "holes": [], "pours": [], "islands": [], "ports": []}
        for c in self.coupons:
            dx, dy = c.cx, c.cy
            for t in c.traces:
                G["traces"].append(dict(t, pts=[(x + dx, y + dy) for x, y in t["pts"]], cid=c.cid))
            for v in c.vias:
                G["vias"].append(dict(v, x=v["x"] + dx, y=v["y"] + dy, cid=c.cid))
            for h in c.holes:
                s = h["shape"]
                if s[0] == "circle":
                    s2 = (s[0], s[1] + dx, s[2] + dy, s[3])
                elif s[0] == "slot":
                    s2 = (s[0], s[1] + dx, s[2] + dy, s[3] + dx, s[4] + dy, s[5])
                else:
                    s2 = (s[0], s[1] + dx, s[2] + dy, s[3] + dx, s[4] + dy)
                G["holes"].append(dict(h, shape=s2, cid=c.cid))
            for p in c.pours:
                x0, y0, x1, y1 = p["rect"]
                G["pours"].append(dict(p, rect=(x0 + dx, y0 + dy, x1 + dx, y1 + dy), cid=c.cid))
            for p in c.islands:
                x0, y0, x1, y1 = p["rect"]
                G["islands"].append(dict(p, rect=(x0 + dx, y0 + dy, x1 + dx, y1 + dy), cid=c.cid))
            for p in c.ports:
                G["ports"].append(dict(p, x=dx + p["side"] * c.X / 2, y=p["y"] + dy, cid=c.cid))
        if not self.probe:
            G["vias"] += self.walls(G)
        return G

    def walls(self, G):
        """GND via walls along every row boundary (and the board's top and bottom), pitch
        wall_pitch, through all layers; they also stitch the launch pours"""
        out = []
        p = LAY["wall_pitch"]
        bounds = []
        cs = self.coupons
        for i in range(len(cs) + 1):
            if i == 0:
                y, hx, inset = cs[0].cy + cs[0].H / 2, cs[0].X / 2, -0.75
            elif i == len(cs):
                y, hx, inset = cs[-1].cy - cs[-1].H / 2, cs[-1].X / 2, 0.75
            else:
                y, hx, inset = cs[i].cy + cs[i].H / 2, min(cs[i].X, cs[i - 1].X) / 2, 0.0
            bounds.append((y + inset, hx))
        segs = [(t["layer"], t["pts"], t["w"]) for t in G["traces"]]
        for (y, hx) in bounds:
            n = int((2 * (hx - WALL_EDGE)) / p)
            x0 = -n * p / 2
            for k in range(n + 1):
                x = x0 + k * p
                if abs(x) > hx - WALL_EDGE + 1e-9:
                    continue
                if clear_of_traces(x, y, LAY["gnd_via"]["pad"] / 2 + 0.15, segs):
                    out.append({"x": x, "y": y, "l0": 0, "l1": self.stk.n - 1, "net": "GND", "pads": None, "role": "gnd",
                                "drill": LAY["gnd_via"]["drill"], "pad": LAY["gnd_via"]["pad"], "cid": "wall"})
        return out

    # --- KiCad output ------------------------------------------------------------------------
    def write_kicad(self, path):
        stk = self.stk
        self.place()
        G = self.gather()
        W = KicadText(self.name)
        outline = self.outline()
        hx = self.width() / 2
        W.ox, W.oy = 30.0 + hx, 30.0
        thick = stk.thickness()
        W.head(stk, thick)
        nets = ["", "GND"]
        for c in self.coupons:
            for n_ in c.nets:
                if n_ not in nets:
                    nets.append(n_)
        for f in self.fps:
            if f["net"] not in nets:
                nets.append(f["net"])
        for isl in G["islands"]:
            if isl["net"] not in nets:
                nets.append(isl["net"])
        W.nets(nets)
        # connectors
        jn = 0
        self.jmap = {}
        for p in G["ports"]:
            jn += 1
            ref = f"J{jn}"
            self.jmap.setdefault(p["cid"], []).append({"ref": ref, "port": p["name"], "side": "left" if p["side"] < 0 else "right",
                                                       "layer": stk.names[p["layer"]], "net": p["net"]})
            W.endlaunch(ref, p, stk)
        for f in self.fps:
            W.simple_pad(f, stk)
        # outline and texts
        W.gr_poly(outline, "Edge.Cuts", 0.1)
        for c in self.coupons:
            if c.group == "info":
                continue
            W.text(c.cid, -c.X / 2 + FP["launch_length"] + 0.8, c.cy + c.H / 2 - 1.2, "F.SilkS", 0.9, justify="left")
        for t in self.texts:
            W.text(*t)
        # tracks
        for t in G["traces"]:
            for a, b in zip(t["pts"][:-1], t["pts"][1:]):
                W.segment(a, b, t["w"], stk.names[t["layer"]], t["net"])
        # vias
        for v in G["vias"]:
            W.via(v, stk)
        # planes: one zone per plane layer, fractured fill with every hole of that layer
        inset = inset_rectilinear(outline, ZONE_INSET)
        for L in stk.planes:
            holes = [h["shape"] for h in G["holes"] if L in h["layers"]]
            for isl in G["islands"]:
                if isl["layer"] == L:
                    x0, y0, x1, y1 = isl["rect"]
                    g = isl.get("clearance", 0.5)
                    holes.append(("rect", x0 - g, y0 - g, x1 + g, y1 + g))
            fill = fractured(inset, holes)
            W.zone("GND", stk.names[L], inset, [fill], name=f"GND plane {stk.names[L]}", clearance=0.2)
        for isl in G["islands"]:
            W.zone(isl["net"], stk.names[isl["layer"]], rect_pts(isl["rect"]), [rect_pts(isl["rect"])], name=f"{isl['net']} island", clearance=0.2, priority=1)
        # keepouts that reproduce the antipads when KiCad refills the zones
        for h in G["holes"]:
            if h["shape"][0] == "rect":
                continue
            W.keepout([stk.names[L] for L in h["layers"]], shape_ring(h["shape"], 32)[:-1], h.get("name") or "antipad")
        for isl in G["islands"]:
            x0, y0, x1, y1 = isl["rect"]
            g = isl.get("clearance", 0.5)
            W.keepout_frame(stk.names[isl["layer"]], (x0 - g, y0 - g, x1 + g, y1 + g), isl["rect"], f"{isl['net']} clearance")
        # outer-layer pours (launches, coplanar grounds)
        for p in G["pours"]:
            W.zone(p["net"], stk.names[p["layer"]], rect_pts(p["rect"]), [rect_pts(p["rect"])], name=f"GND {p['cid']}", clearance=0.1)
        W.notes(self, -hx, self.bottom - 5.0)
        W.close()
        open(path, "w").write(W.text_out())
        self.write_pro(path[:-len(".kicad_pcb")] + ".kicad_pro")
        return G

    def write_pro(self, path):
        d = self.design
        w50 = d.get("w50", {}).get("0", 0.2)
        dp = d.get("diff", {}).get("0", {"w": 0.2, "s": 0.2})
        cls = lambda name, tw, dw, dg: {"name": name, "bus_width": 12, "clearance": 0.1, "diff_pair_gap": dg, "diff_pair_via_gap": 0.25,
                                        "diff_pair_width": dw, "line_style": 0, "microvia_diameter": 0.3, "microvia_drill": 0.1,
                                        "pcb_color": "rgba(0, 0, 0, 0.000)", "priority": 2147483647 if name == "Default" else 0,
                                        "schematic_color": "rgba(0, 0, 0, 0.000)", "track_width": tw, "via_diameter": 0.6, "via_drill": 0.3, "wire_width": 6}
        classes = [cls("Default", 0.2, 0.2, 0.2), cls("50Z", w50, w50, w50), cls("100Z_Diff", dp["w"], dp["w"], dp["s"])]
        classes[2]["priority"] = 1
        pro = {
            "board": {"3dviewports": [], "design_settings": {
                "defaults": {"board_outline_line_width": 0.1, "copper_line_width": 0.2, "copper_text_size_h": 1.5, "copper_text_size_v": 1.5,
                             "copper_text_thickness": 0.3, "silk_line_width": 0.12, "silk_text_size_h": 1.0, "silk_text_size_v": 1.0, "silk_text_thickness": 0.15},
                "diff_pair_dimensions": [{"gap": 0.0, "via_gap": 0.0, "width": 0.0}, {"gap": dp["s"], "via_gap": 0.25, "width": dp["w"]}],
                "rules": {"max_error": 0.005, "min_clearance": 0.1, "min_connection": 0.0, "min_copper_edge_clearance": 0.2, "min_hole_clearance": 0.2,
                          "min_hole_to_hole": 0.25, "min_microvia_diameter": 0.2, "min_microvia_drill": 0.1, "min_resolved_spokes": 1,
                          "min_silk_clearance": 0.0, "min_text_height": 0.8, "min_text_thickness": 0.08, "min_through_hole_diameter": 0.3,
                          "min_track_width": 0.09, "min_via_annular_width": 0.15, "min_via_diameter": 0.5, "solder_mask_to_copper_clearance": 0.0,
                          "use_height_for_length_calcs": True},
                "track_widths": [0.0, w50], "via_dimensions": [{"diameter": 0.0, "drill": 0.0}, {"diameter": 0.6, "drill": 0.3}]},
                "layer_presets": [], "viewports": []},
            "boards": [], "cvpcb": {"equivalence_files": []}, "libraries": {"pinned_footprint_libs": [], "pinned_symbol_libs": []},
            "meta": {"filename": os.path.basename(path), "version": 1},
            "net_settings": {"classes": classes, "meta": {"version": 3}, "net_colors": None, "netclass_assignments": None,
                             "netclass_patterns": [{"netclass": "50Z", "pattern": "SE_*"}, {"netclass": "100Z_Diff", "pattern": "DP_*"}]},
            "pcbnew": {"last_paths": {"gencad": "", "idf": "", "netlist": "", "plot": "", "pos_files": "", "specctra_dsn": "", "step": "", "svg": "", "vrml": ""},
                       "page_layout_descr_file": ""},
            "sheets": [], "text_variables": {}}
        json.dump(pro, open(path, "w"), indent=2)

    def write_kisi_config(self, path):
        stk = self.stk
        diel = []
        for i, d in enumerate(stk.diel):
            src = d["src"]
            kind = "assumption" if "assumption" in str(src.get("er", "")) else "datasheet"
            sid = str(src.get("er", "")).split(" ")[0]
            url = (STK["sources"].get(sid) or {}).get("url")
            diel.append({"index": i, "material": d["material"], "epsilon_r": d["er"], "loss_tangent": d["tand"], "thickness": d["thickness"],
                         "source": {"kind": kind, "document": f"{stk.d['title']}: thickness {src.get('thickness')}, er {src.get('er')}, tan d {src.get('tand')}",
                                    "url": url, "note": "loss tangent assumed (not published by the manufacturer)"}})
        cfg = {"kisi_config": 1, "board": os.path.basename(path).replace(".kisi.json", ".kicad_pcb"),
               "stackup": {"dielectrics": diel, "solder_mask": {"epsilon_r": stk.mask["er"], "source": {
                   "kind": "assumption" if "assumption" in stk.mask["src"]["er"] else "datasheet",
                   "document": "solder mask: " + stk.mask["src"]["er"], "url": STK["sources"]["jlc_impedance_guide"]["url"] if stk.d["manufacturer"] == "JLCPCB" else None}}},
               "net_classes": {
                   "50Z": {"target_ohm": 50, "diff": False, "profile": "se50", "source": {"kind": "standard", "note": "coupon design target (50 Ω VNA ports)"}},
                   "100Z_Diff": {"target_ohm": 100, "diff": True, "profile": "diff100", "source": {"kind": "standard", "note": "coupon design target (2 × 50 Ω VNA ports)"}}},
               "fields": [],
               "notes": [f"kisi measurement coupons, generated by validation/test_board/make_coupons.py on {TODAY}",
                         f"stackup: {stk.d['title']}"]}
        json.dump(cfg, open(path, "w"), indent=1)


def seg_dist(px, py, a, b):
    (x0, y0), (x1, y1) = a, b
    dx, dy = x1 - x0, y1 - y0
    l2 = dx * dx + dy * dy
    t = 0.0 if l2 == 0 else max(0.0, min(1.0, ((px - x0) * dx + (py - y0) * dy) / l2))
    return math.hypot(px - x0 - t * dx, py - y0 - t * dy)


def clear_of_traces(x, y, r, segs, layers=None):
    for (L, pts, w) in segs:
        if layers is not None and L not in layers:
            continue
        for a, b in zip(pts[:-1], pts[1:]):
            if seg_dist(x, y, a, b) < r + w / 2:
                return False
    return True


# --- KiCad text -----------------------------------------------------------------------------------
class KicadText:
    def __init__(self, board):
        self.board = board
        self.lines = []
        self.k = 0
        self.ox = self.oy = 0.0

    def uid(self):
        self.k += 1
        return str(uuid.uuid5(NS, f"{self.board}/{self.k}"))

    def X(self, x):
        return round(self.ox + x, 4)

    def Y(self, y):
        return round(self.oy - y, 4)

    def xy(self, p):
        return f"(xy {self.X(p[0])} {self.Y(p[1])})"

    def head(self, stk, thick):
        L = self.lines
        L.append(f'(kicad_pcb (version 20240108) (generator "kisi_make_coupons") (generator_version "8.0")')
        L.append(f"  (general (thickness {thick:.4f}) (legacy_teardrops no))")
        L.append('  (paper "A3")')
        L.append(f'  (title_block (title "{self.board}") (date "{TODAY}") (rev "1") (company "kisi measurement coupons") '
                 f'(comment 1 "{stk.d["title"]}") (comment 2 "generated by validation/test_board/make_coupons.py - DRC not run"))')
        ids = kb.kicad_ids(stk.n)
        cu = " ".join(f'({i} "{nm}" signal)' for i, nm in zip(ids, stk.names))
        L.append(f"  (layers {cu} (32 \"B.Adhes\" user \"B.Adhesive\") (33 \"F.Adhes\" user \"F.Adhesive\") (34 \"B.Paste\" user) (35 \"F.Paste\" user) "
                 "(36 \"B.SilkS\" user \"B.Silkscreen\") (37 \"F.SilkS\" user \"F.Silkscreen\") (38 \"B.Mask\" user) (39 \"F.Mask\" user) "
                 "(40 \"Dwgs.User\" user \"User.Drawings\") (41 \"Cmts.User\" user \"User.Comments\") (42 \"Eco1.User\" user \"User.Eco1\") "
                 "(43 \"Eco2.User\" user \"User.Eco2\") (44 \"Edge.Cuts\" user) (45 \"Margin\" user) (46 \"B.CrtYd\" user \"B.Courtyard\") "
                 "(47 \"F.CrtYd\" user \"F.Courtyard\") (48 \"B.Fab\" user) (49 \"F.Fab\" user))")
        m = stk.mask
        s = ['    (layer "F.SilkS" (type "Top Silk Screen"))', '    (layer "F.Paste" (type "Top Solder Paste"))',
             f'    (layer "F.Mask" (type "Top Solder Mask") (color "Green") (thickness {m["thickness"]}) (material "LPI") (epsilon_r {m["er"]}) (loss_tangent {m["tand"]}))']
        for i in range(stk.n):
            s.append(f'    (layer "{stk.names[i]}" (type "copper") (thickness {stk.cu[i]}))')
            if i < stk.n - 1:
                d = stk.diel[i]
                s.append(f'    (layer "dielectric {i + 1}" (type "{d["type"]}") (thickness {d["thickness"]}) (material "{d["material"]}") '
                         f'(epsilon_r {d["er"]}) (loss_tangent {d["tand"]}))')
        s += [f'    (layer "B.Mask" (type "Bottom Solder Mask") (color "Green") (thickness {m["thickness"]}) (material "LPI") (epsilon_r {m["er"]}) (loss_tangent {m["tand"]}))',
              '    (layer "B.Paste" (type "Bottom Solder Paste"))', '    (layer "B.SilkS" (type "Bottom Silk Screen"))',
              '    (copper_finish "ENIG")', '    (dielectric_constraints yes)']
        L.append("  (setup\n   (stackup\n" + "\n".join(s) + "\n   )\n   (pad_to_mask_clearance 0)\n   (allow_soldermask_bridges_in_footprints no)\n  )")

    def nets(self, nets):
        self.netid = {n: i for i, n in enumerate(nets)}
        self.lines.append("  " + " ".join(f'(net {i} "{n}")' for i, n in enumerate(nets)))

    def n(self, net):
        return f'(net {self.netid[net]} "{net}")'

    def endlaunch(self, ref, p, stk):
        """end-launch connector footprint at the board edge: signal pad, mask opening of the ground
        contact, mounting holes (PLACEHOLDER positions, connectors.json), block outline"""
        top = p["layer"] == 0
        cu, mk, fab, crt, silk = (("F.Cu", "F.Mask", "F.Fab", "F.CrtYd", "F.SilkS") if top else ("B.Cu", "B.Mask", "B.Fab", "B.CrtYd", "B.SilkS"))
        rot = 0 if p["side"] < 0 else 180
        x, y = p["x"], p["y"]
        L = self.lines
        mir = "" if top else " (justify mirror)"
        L.append(f'  (footprint "kisi_coupons:SWM_EndLaunch_Narrow_PLACEHOLDER" (layer "{cu}") (uuid "{self.uid()}") (at {self.X(x)} {self.Y(y)} {rot})')
        L.append(f'    (descr "Southwest Microwave end launch, narrow block (.350): 292-04A-14 (SMA, 27 GHz) or 1092-01A-14 (2.92 mm, 40 GHz). '
                 f'Footprint PLACEHOLDER - check mounting holes and ground contact against the vendor drawing")')
        L.append(f'    (property "Reference" "{ref}" (at 2.0 {-(4.445 + 0.9)} {rot}) (layer "{silk}") (uuid "{self.uid()}") (effects (font (size 0.8 0.8) (thickness 0.12)){mir}))')
        L.append(f'    (property "Value" "292-04A-14 / 1092-01A-14" (at 2.0 {4.445 + 0.9} {rot}) (layer "{fab}") (uuid "{self.uid()}") (effects (font (size 0.6 0.6) (thickness 0.1)){mir}))')
        L.append(f'    (property "Port" "{p["cid"]} port {p["name"]}" (at 0 0 {rot}) (layer "{fab}") (hide yes) (uuid "{self.uid()}") (effects (font (size 0.6 0.6) (thickness 0.1))))')
        L.append("    (attr smd exclude_from_pos_files exclude_from_bom)")
        bw = LAUNCH["block_width"] / 2
        L.append(f'    (fp_rect (start {-FP["body_outside_board"]} {-bw}) (end {FP["block_depth_on_board"]} {bw}) (stroke (width 0.05) (type default)) (fill none) (layer "{crt}") (uuid "{self.uid()}"))')
        L.append(f'    (fp_rect (start {-FP["body_outside_board"] + 0.2} {-bw + 0.2}) (end {FP["block_depth_on_board"] - 0.2} {bw - 0.2}) (stroke (width 0.1) (type default)) (fill none) (layer "{fab}") (uuid "{self.uid()}"))')
        # mask opening over the ground contact and the signal pad
        g0 = p["w"] / 2 + p["gap"]
        for s in (1, -1):
            L.append(f'    (fp_rect (start {FP["edge_pullback"]} {s * g0}) (end {FP["launch_length"] - 0.3} {s * FP["gnd_contact_half_width"]}) '
                     f'(stroke (width 0) (type solid)) (fill solid) (layer "{mk}") (uuid "{self.uid()}"))')
        lp = FP["signal_pad_length"]
        L.append(f'    (pad "1" smd rect (at {FP["edge_pullback"] + lp / 2:.4f} 0 {rot}) (size {lp} {p["w"]}) (layers "{cu}" "{mk}") {self.n(p["net"])} (uuid "{self.uid()}"))')
        for h in FP["mount_holes"]:
            L.append(f'    (pad "" np_thru_hole circle (at {h["x"]} {h["y"]} {rot}) (size {h["d"]} {h["d"]}) (drill {h["d"]}) (layers "*.Cu" "*.Mask") (uuid "{self.uid()}"))')
        L.append("  )")

    def simple_pad(self, f, stk):
        nm = stk.names[f["layer"]]
        L = self.lines
        L.append(f'  (footprint "probe:pad" (layer "F.Cu") (uuid "{self.uid()}") (at {self.X(f["x"])} {self.Y(f["y"])})')
        L.append(f'    (property "Reference" "{f["ref"]}" (at 0 -1 0) (layer "F.Fab") (uuid "{self.uid()}") (effects (font (size 0.5 0.5) (thickness 0.08))))')
        L.append(f'    (property "Value" "{"IC" if f["ref"].startswith("U") else "conn"}" (at 0 1 0) (layer "F.Fab") (uuid "{self.uid()}") (effects (font (size 0.5 0.5) (thickness 0.08))))')
        L.append(f'    (pad "1" smd rect (at 0 0) (size {f["w"]:.4f} {f["w"]:.4f}) (layers "{nm}") {self.n(f["net"])} (uuid "{self.uid()}"))')
        L.append("  )")

    def gr_poly(self, pts, layer, w):
        self.lines.append(f'  (gr_poly (pts {" ".join(self.xy(p) for p in pts)}) (stroke (width {w}) (type solid)) (fill none) (layer "{layer}") (uuid "{self.uid()}"))')

    def text(self, s, x, y, layer, size, justify=None):
        j = f" (justify {justify})" if justify else ""
        if layer.startswith("B."):
            j = f" (justify {(justify + ' ') if justify else ''}mirror)"
        s = s.replace('"', "'")
        self.lines.append(f'  (gr_text "{s}" (at {self.X(x)} {self.Y(y)} 0) (layer "{layer}") (uuid "{self.uid()}") '
                          f'(effects (font (size {size} {size}) (thickness {round(size * 0.15, 3)})){j}))')

    def segment(self, a, b, w, layer, net):
        self.lines.append(f'  (segment (start {self.X(a[0])} {self.Y(a[1])}) (end {self.X(b[0])} {self.Y(b[1])}) (width {w}) (layer "{layer}") (net {self.netid[net]}) (uuid "{self.uid()}"))')

    def via(self, v, stk):
        la, lb = stk.names[v["l0"]], stk.names[v["l1"]]
        self.lines.append(f'  (via (at {self.X(v["x"])} {self.Y(v["y"])}) (size {v["pad"]}) (drill {v["drill"]}) (layers "{la}" "{lb}") '
                          f'(remove_unused_layers yes) (keep_end_layers no) (net {self.netid[v["net"]]}) (uuid "{self.uid()}"))')

    def zone(self, net, layer, outline, fills, name, clearance=0.2, priority=0):
        L = self.lines
        L.append(f'  (zone (net {self.netid[net]}) (net_name "{net}") (layer "{layer}") (uuid "{self.uid()}") (name "{name}") (hatch edge 0.5) (priority {priority})')
        L.append(f"    (connect_pads yes (clearance {clearance})) (min_thickness 0.15) (filled_areas_thickness no)")
        L.append("    (fill yes (thermal_gap 0.3) (thermal_bridge_width 0.3) (island_removal_mode 1))")
        L.append(f"    (polygon (pts {' '.join(self.xy(p) for p in outline)}))")
        for f in fills:
            L.append(f'    (filled_polygon (layer "{layer}") (pts {" ".join(self.xy(p) for p in f)}))')
        L.append("  )")

    def keepout(self, layers, pts, name):
        ls = " ".join(f'"{l}"' for l in layers)
        self.lines.append(f'  (zone (net 0) (net_name "") (layers {ls}) (uuid "{self.uid()}") (name "{name}") (hatch edge 0.5) '
                          f'(connect_pads (clearance 0)) (min_thickness 0.25) (filled_areas_thickness no) '
                          f'(keepout (tracks allowed) (vias allowed) (pads allowed) (copperpour not_allowed) (footprints allowed)) '
                          f'(fill (thermal_gap 0.5) (thermal_bridge_width 0.5)) (polygon (pts {" ".join(self.xy(p) for p in pts)})))')

    def keepout_frame(self, layer, outer, inner, name):
        """four keepout strips around an island (the clearance gap between island and plane)"""
        x0, y0, x1, y1 = outer
        a0, b0, a1, b1 = inner
        for r in ((x0, y0, x1, b0), (x0, b1, x1, y1), (x0, b0, a0, b1), (a1, b0, x1, b1)):
            self.keepout([layer], rect_pts(r), name)

    def notes(self, board, x, y):
        stk = board.stk
        d = board.design
        lines = [f"{board.name}: kisi measurement coupons ({TODAY})", f"Stackup: {stk.d['title']}",
                 f"Impedance control: 50 ohm single-ended (class 50Z) on outer layers w = {d.get('w50', {}).get('0')} mm; "
                 f"100 ohm differential (class 100Z_Diff); +/-10 % (JLC standard).",
                 "Finish ENIG (gold only on the launch openings); green LPI mask on both sides over all lines.",
                 "Via drill 0.3 mm / pad 0.6 mm, through vias only, inner non-functional pads removed (remove_unused_layers).",
                 "Copper may come to 0.2 mm from the left and right board edges at the launches (end-launch connectors).",
                 "Connector footprint (Southwest Microwave narrow block) is a PLACEHOLDER: check mounting holes before ordering.",
                 "Zones are pre-filled by the generator: refill in KiCad (B) and run DRC before plotting. DRC has NOT been run."]
        for i, s in enumerate(lines):
            self.text(s, x, y - 2.0 * i, "Cmts.User", 1.2, justify="left")

    def close(self):
        self.lines.append(")")

    def text_out(self):
        return "\n".join(self.lines) + "\n"


# --- coupon builders --------------------------------------------------------------------------------
class Ctx:
    def __init__(self, stk, des, pair_pitch):
        self.stk, self.des, self.pp = stk, des, pair_pitch
        self.rp = LAY["row_pitch"]
        n = stk.n
        self.bot = n - 1
        self.w = {int(k): v for k, v in des["w50"].items()}
        self.wlaunch, self.glaunch = des["launch"]["w"], des["launch"]["gap"]

    def wl(self, layer):
        return self.w[layer]


def launch(c, ctx, side, y, layer, net, name, w=None, gap=None, pour_lo=None, pour_hi=None):
    """end-launch port at the edge `side` (-1 left, +1 right): the launch trace (grounded coplanar,
    launch width) from the edge to `launch_length`, the ground pours on both sides of it with two
    rows of fence vias. Returns the x where the coupon's own trace starts."""
    w = ctx.wlaunch if w is None else w
    gap = ctx.glaunch if gap is None else gap
    c.port(side, y, layer, net, w, gap, name)
    xe = edge_x(c, side)
    x_in = side * (c.X / 2 - FP["launch_length"])
    c.trace(layer, [(xe, y), (x_in, y)], w, net)
    xa = side * (c.X / 2 - ZONE_INSET)
    lo = -c.H / 2 + 0.3 if pour_lo is None else pour_lo
    hi = c.H / 2 - 0.3 if pour_hi is None else pour_hi
    cols = []
    k = 0
    while 0.8 + k * LAY["fence_pitch_launch"] <= FP["launch_length"] - 0.35:
        cols.append(side * (c.X / 2 - 0.8 - k * LAY["fence_pitch_launch"]))
        k += 1
    for s, (y0, y1) in ((1, (y + w / 2 + gap, hi)), (-1, (lo, y - w / 2 - gap))):
        if y1 - y0 < 0.6:
            continue
        c.pour(layer, (min(xa, x_in), y0, max(xa, x_in), y1))
        for row in (0, 1):
            yy = (y0 + FENCE_OFF + row * 0.8) if s > 0 else (y1 - FENCE_OFF - row * 0.8)
            if y0 + 0.3 <= yy <= y1 - 0.3:
                for xx in cols:
                    c.via(round(xx, 4), round(yy, 4), 0, ctx.stk.n - 1, "GND", role="gnd")
                    c.vias[-1]["fence"] = True
    return x_in


def straight(c, ctx, layer, y, net, x0, x1, w=None):
    c.trace(layer, [(x0, y), (x1, y)], w or ctx.wl(layer), net)


def edge_x(c, side):
    return side * (c.X / 2 - FP["edge_pullback"])


def ring_clear(c, xc, yc, n, r):
    """GND vias on a ring that keep clear of every trace of the coupon (through vias)"""
    segs = [(t["layer"], t["pts"], t["w"]) for t in c.traces]
    return [(round(xc + dx, 4), round(yc + dy, 4)) for (dx, dy) in vc.ring(n, r)
            if clear_of_traces(xc + dx, yc + dy, LAY["gnd_via"]["pad"] / 2 + 0.12, segs)]


def fan(c, x_in, side, y_out, y_in, x_end, layer, net, w_out, w_in, x_min=0.3):
    """from the launch end (x_in, y_out): a short straight piece, a 45° diagonal (steeper if there
    is no room) to y_in, then straight to x_end; width w_out up to the end of the diagonal, then
    w_in. Returns the x where the diagonal ends."""
    xs = side * (abs(x_in) + FP["launch_length"] - X_CLEAR)
    dy = abs(y_out - y_in)
    xd = xs - side * dy
    if side * (xd - x_end) < x_min:
        xd = x_end + side * x_min
    c.trace(layer, [(x_in, y_out), (xs, y_out), (xd, y_in)], w_out, net)
    if abs(xd - x_end) > 1e-9:
        c.trace(layer, [(xd, y_in), (x_end, y_in)], w_in, net)
    return xd


def c_thru(ctx, cid, layer, X=25.0, lines=False):
    stk = ctx.stk
    side_name = "top" if layer == 0 else "bottom"
    if X == 25.0:
        title = f"2×thru on {stk.names[layer]} ({side_name} connectors), {X:g} mm edge to edge"
        val = "fixture for the P370 2×thru de-embedding of every coupon fed on this layer; multiline TRL 'thru'"
    else:
        title = f"Line on {stk.names[layer]}, {X:g} mm edge to edge (ΔL = {X - 2 * FEED:g} mm against the 2×thru)"
        val = "multiline TRL line / line–line method: εeff, loss per mm and Zc of the 50 Ω microstrip"
    c = Coupon(cid, "thru" if X == 25.0 else "T", title, X, ctx.rp, val, [f"thru_s4_L{1 if layer == 0 else 4}"] if X == 25.0 else ["T_ts6_ms_10", "T_ts6_ms_20"],
               {"layer": stk.names[layer], "w_mm": ctx.wl(layer), "length_mm": X})
    net = f"SE_{cid}"
    xl = launch(c, ctx, -1, 0.0, layer, net, "1")
    xr = launch(c, ctx, 1, 0.0, layer, net, "2")
    straight(c, ctx, layer, 0.0, net, xl, xr)
    if X == 25.0:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="thru", note="10 mm of the feed line: 3D 2×thru of the via coupons (window w10)")
        c.window("w18", -9.0, 9.0, excite=[0], mirror=[1, 0], group="thru", note="18 mm: 3D 2×thru of the wide D_NONE window (w18)")
    elif lines:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note="line, 10 mm (line–line pair with w20)")
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note="line, 20 mm")
    return c


def c_reflect(ctx, cid, gap=5.0):
    X = 2 * FEED + gap
    c = Coupon(cid, "reflect", f"TRL reflect: two open line ends at the reference planes ({FEED:g} mm from each edge), {gap:g} mm apart",
               X, ctx.rp, "reflect standard of an on-board multiline TRL calibration (with THRU_L1 and LINE_*)", [], {"open_ends": True})
    for side, nm in ((-1, "L"), (1, "R")):
        net = f"SE_{cid}_{nm}"
        x_in = launch(c, ctx, side, 0.0, 0, net, "1" if side < 0 else "2")
        straight(c, ctx, 0, 0.0, net, x_in, side * gap / 2)
    return c


def c_via(ctx, cid, group, title, refs, antipad, stitch, l_out=None, validates="", params=None, wide=False):
    """single via L1 -> l_out at x = 0: L1 feed from the left (top connector), l_out feed to the
    right (bottom connector)"""
    stk = ctx.stk
    l_out = ctx.bot if l_out is None else l_out
    c = Coupon(cid, group, title, 2 * FEED, ctx.rp, validates, refs, params)
    net = f"SE_{cid}"
    xl = launch(c, ctx, -1, 0.0, 0, net, "1")
    xr = launch(c, ctx, 1, 0.0, l_out, net, "2")
    straight(c, ctx, 0, 0.0, net, xl, 0.0)
    straight(c, ctx, l_out, 0.0, net, 0.0, xr)
    c.via(0.0, 0.0, 0, stk.n - 1, net, pads=[0, l_out])
    for L in stk.planes:
        c.hole([L], ("circle", 0.0, 0.0, antipad / 2), f"antipad {cid}")
    for (x, y) in stitch:
        c.via(round(x, 4), round(y, 4), 0, stk.n - 1, "GND", role="gnd")
    c.window("w10", -5.0, 5.0, note="via with 5 mm feeds on both sides; de-embed with the THRU w10 cases of both feed layers")
    if wide:
        c.window("w18", -9.0, 9.0, note="via with 9 mm feeds; the row walls at ±6.35 mm are the nearest return path (de-embed with THRU w18)")
    return c


def c_vlv(ctx, cid, Ls, inner, title, validates, refs=(), lines=False):
    """L1 -> inner stripline (length Ls) -> L1; through vias, so both have stubs to the bottom"""
    stk = ctx.stk
    X = 2 * FEED + Ls
    c = Coupon(cid, "B" if Ls <= 5 else "T", title, X, ctx.rp, validates, refs,
               {"inner": stk.names[inner], "stripline_mm": Ls, "w_stripline_mm": ctx.wl(inner), "stub": f"{stk.names[inner]}→{stk.names[-1]}"})
    net = f"SE_{cid}"
    xa, xb = -Ls / 2, Ls / 2
    xl = launch(c, ctx, -1, 0.0, 0, net, "1")
    xr = launch(c, ctx, 1, 0.0, 0, net, "2")
    straight(c, ctx, 0, 0.0, net, xl, xa)
    straight(c, ctx, inner, 0.0, net, xa, xb)
    straight(c, ctx, 0, 0.0, net, xb, xr)
    for x in (xa, xb):
        c.via(x, 0.0, 0, stk.n - 1, net, pads=[0, inner])
        for L in stk.planes:
            c.hole([L], ("circle", x, 0.0, 0.4), f"antipad {cid}")
    for x in (xa, xb):
        for (gx, gy) in ring_clear(c, x, 0.0, 4, 1.0):
            c.via(gx, gy, 0, stk.n - 1, "GND", role="gnd")
    if Ls <= 5:
        c.window("w15", xa - 5.0, xb + 5.0, excite=[0], mirror=[1, 0], note="via - stripline - via with 5 mm L1 feeds; de-embed with THRU_L1 w10")
    elif lines:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note="stripline, 10 mm (line–line pair with w20)")
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note="stripline, 20 mm")
    return c


def c_cpwg(ctx, cid, X):
    cp = ctx.des["cpwg"]
    w, g = cp["w"], cp["gap"]
    c = Coupon(cid, "T", f"Grounded coplanar waveguide on L1, w {w:g} / gap {g:g} mm, via fence 1.0 mm, {X:g} mm edge to edge", X, ctx.rp,
               "CPWG Zc and εeff (kisi's CPWG formula: εeff +5 % against 3D); CPWG_25 is the CPWG 2×thru", ["T_vna6_cpwg_10", "T_vna6_cpwg_20"],
               {"w_mm": w, "gap_mm": g, "length_mm": X, "fence_pitch_mm": 1.0})
    net = f"SE_{cid}"
    c.trace(0, [(edge_x(c, -1), 0.0), (edge_x(c, 1), 0.0)], w, net)
    c.port(-1, 0.0, 0, net, w, g, "1")
    c.port(1, 0.0, 0, net, w, g, "2")
    xa = c.X / 2 - ZONE_INSET
    # the pour stays clear of the connector's mounting holes: a strip next to the gap over the whole
    # length, the full height only between the connector footprints
    yh = min(abs(h["y"]) - h["d"] / 2 for h in FP["mount_holes"]) - 0.3
    xh = c.X / 2 - X_CLEAR + 0.2
    for s in (1, -1):
        y0, y1 = w / 2 + g, c.H / 2 - 0.3
        c.pour(0, (-xa, min(s * y0, s * yh), xa, max(s * y0, s * yh)))
        c.pour(0, (-xh, min(s * yh, s * y1), xh, max(s * yh, s * y1)))
        yv = s * (y0 + FENCE_OFF)
        n = int((c.X - 1.6) / 1.0)
        for k in range(n + 1):
            x = -c.X / 2 + 0.8 + k * 1.0
            if x <= c.X / 2 - 0.8 + 1e-9:
                c.via(round(x, 4), round(yv, 4), 0, ctx.stk.n - 1, "GND", role="gnd")
    if X == 25.0:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note="CPWG, 10 mm (line–line pair with CPWG_50 w20)")
    else:
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note="CPWG, 20 mm")
    return c


def four_port_launches(c, ctx, layer_l, layer_r, nets):
    """two connectors per edge at ±pair_pitch/2; returns {(side, k): x_in}"""
    hp = ctx.pp / 2
    out = {}
    for side, layer in ((-1, layer_l), (1, layer_r)):
        for k, y in enumerate((hp, -hp)):
            nm = str(k + 1) if side < 0 else str(k + 3)
            lo, hi = (0.0, None) if y > 0 else (None, 0.0)
            out[(side, k)] = launch(c, ctx, side, y, layer, nets[k], nm, pour_lo=lo, pour_hi=hi)
    return out


def c_pair(ctx, cid, group, title, refs, l_out, vias, oval=False, ngnd=2, validates="", params=None):
    """differential pair at the 1.0 mm via pitch (100 Ω differential); with vias: L1 -> l_out at x = 0"""
    stk = ctx.stk
    c = Coupon(cid, group, title, 2 * FEED4, 2 * ctx.pp, validates, refs, params)
    P, N = f"DP_{cid}_P", f"DP_{cid}_N"
    pv = ctx.des["pair_via"]
    ys = (0.5, -0.5)
    hp = ctx.pp / 2
    l_l = 0 if vias else l_out      # 2×thru: both halves on l_out
    l_r = l_out
    xin = four_port_launches(c, ctx, l_l, l_r, (P, N))
    for k, (net, yo, yi) in enumerate(((P, hp, ys[0]), (N, -hp, ys[1]))):
        fan(c, xin[(-1, k)], -1, yo, yi, 0.0, l_l, net, ctx.wl(l_l), pv[str(l_l)]["w"], x_min=3.3)
        fan(c, xin[(1, k)], 1, yo, yi, 0.0, l_r, net, ctx.wl(l_r), pv[str(l_r)]["w"], x_min=3.3)
    c.params.update(w_pair_mm=pv[str(l_l)]["w"], pitch_mm=1.0)
    if vias:
        for net, y in ((P, ys[0]), (N, ys[1])):
            c.via(0.0, y, 0, stk.n - 1, net, pads=[0, l_out])
        for L in stk.planes:
            if oval:
                c.hole([L], ("slot", 0.0, ys[0], 0.0, ys[1], 0.4), f"antipad {cid}")
            else:
                for y in ys:
                    c.hole([L], ("circle", 0.0, y, 0.4), f"antipad {cid}")
        gy = 0.5 + 0.8
        for (x, y) in {0: [], 2: [(0, gy), (0, -gy)], 4: [(0.6, gy - 0.1), (-0.6, gy - 0.1), (0.6, -gy + 0.1), (-0.6, -gy + 0.1)]}[ngnd]:
            c.via(x, y, 0, stk.n - 1, "GND", role="gnd")
    c.window("w6", -3.0, 3.0, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="E" if vias else "thru",
             note="pair at the 1.0 mm via pitch, 3 mm on each side; ports 1 P near, 2 N near, 3 P far, 4 N far")
    return c


def c_coupled(ctx, cid, kind, title, refs, validates, X=50.0, layer=0):
    """XT: 50 Ω aggressor (A) and victim (V) with edge gap = w; DIFF: tightly coupled 100 Ω pair,
    reached by 50 Ω lines down to a 1.2 mm pitch and a short taper of the spacing"""
    c = Coupon(cid, "T", title, X, 2 * ctx.pp, validates, refs)
    hp = ctx.pp / 2
    if kind == "xt":
        w = ctx.wl(layer)
        nets, yi, wi = ("SE_XT_A", "SE_XT_V"), w, w
        c.params.update(w_mm=w, gap_mm=w)
    else:
        p = ctx.des["diff"][str(layer)]
        nets, yi, wi = (f"DP_{cid}_P", f"DP_{cid}_N"), (p["w"] + p["s"]) / 2, p["w"]
        c.params.update(w_mm=p["w"], s_mm=p["s"])
    xin = four_port_launches(c, ctx, layer, layer, nets)
    xds = []
    for k, (net, sg) in enumerate(((nets[0], 1), (nets[1], -1))):
        for side in (-1, 1):
            if kind == "xt":
                xds.append(fan(c, xin[(side, k)], side, sg * hp, sg * yi, 0.0, layer, net, ctx.wl(layer), wi))
            else:
                xm = fan(c, xin[(side, k)], side, sg * hp, sg * 0.6, side * 12.0, layer, net, ctx.wl(layer), wi)
                xq = xm - side * (0.3 + 0.6 - yi)
                c.traces.pop()  # the straight part after the diagonal is replaced by the taper
                c.trace(layer, [(xm, sg * 0.6), (xm - side * 0.3, sg * 0.6), (xq, sg * yi), (0.0, sg * yi)], wi, net)
                xds.append(xq)
    half = min(abs(x) for x in xds)
    c.params["coupled_mm"] = round(2 * half, 2)
    c.title += f", coupled {2 * half:.1f} mm"
    if layer == 0:
        h = round(half - 0.3, 2)
        c.window("wc", -h, h, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="T",
                 note=f"the straight coupled section ({2 * h:g} mm); ports 1 A/P near, 2 V/N near, 3 A/P far, 4 V/N far")
    return c


def c_coupled_inner(ctx, cid, kind, title, refs, validates, X=50.0):
    """stripline pair on the first inner signal layer (diff), or broadside on the two inner signal
    layers (bs); fed from L1 through vias at both ends"""
    stk = ctx.stk
    inner = [L for L in stk.signal if 0 < L < stk.n - 1]
    c = Coupon(cid, "T", title, X, 2 * ctx.pp, validates, refs)
    hp = ctx.pp / 2
    xv = None
    if kind == "diff":
        p = ctx.des["diff"][str(inner[0])]
        nets, layers = (f"DP_{cid}_P", f"DP_{cid}_N"), (inner[0], inner[0])
        yv, yc, wi = (0.5, -0.5), ((p["w"] + p["s"]) / 2, -(p["w"] + p["s"]) / 2), (p["w"], p["w"])
        c.params.update(layer=stk.names[inner[0]], w_mm=p["w"], s_mm=p["s"])
    else:
        nets, layers = ("SE_BS_A", "SE_BS_V"), (inner[0], inner[1])
        yv, yc, wi = (0.6, -0.6), (0.0, 0.0), (ctx.wl(inner[0]), ctx.wl(inner[1]))
        c.params.update(layers=f"{stk.names[inner[0]]} / {stk.names[inner[1]]}", w_mm=list(wi), offset_mm=0.0)
    xv = c.X / 2 - X_CLEAR - (hp - max(abs(y) for y in yv)) - 0.5   # via x: the L1 fan-in at 45°
    xin = four_port_launches(c, ctx, 0, 0, nets)
    for k, (net, sg) in enumerate(((nets[0], 1), (nets[1], -1))):
        for side in (-1, 1):
            fan(c, xin[(side, k)], side, sg * hp, yv[k], side * xv, 0, net, ctx.wl(0), ctx.wl(0))
            xj = side * (xv - 0.9)
            c.trace(layers[k], [(side * xv, yv[k]), (xj, yv[k]), (side * (xv - 0.9 - abs(yv[k] - yc[k])), yc[k]), (0.0, yc[k])], wi[k], net)
            c.via(side * xv, yv[k], 0, stk.n - 1, net, pads=[0, layers[k]])
            for L in stk.planes:
                c.hole([L], ("circle", side * xv, yv[k], 0.4), f"antipad {cid}")
    for side in (-1, 1):
        for (gx, gy) in ring_clear(c, side * xv, 0.0, 8, 1.6):
            c.via(gx, gy, 0, stk.n - 1, "GND", role="gnd")
    half = xv - 0.9 - max(abs(yv[k] - yc[k]) for k in (0, 1))
    c.params["coupled_mm"] = round(2 * half, 2)
    c.title += f", coupled {2 * half:.1f} mm"
    h = round(half - 0.3, 2)
    if kind == "diff":
        c.window("wc", -h, h, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="T", note=f"the straight coupled stripline section ({2 * h:g} mm)")
    else:
        c.window("wc", -h, h, half_y=4.0, excite=[0, 1], mirror=[2, 3, 0, 1], group="T",
                 note=f"the straight broadside section ({2 * h:g} mm); ports 1/3 upper layer (A), 2/4 lower layer (V)")
    return c


def c_cavity(ctx, cid):
    """G: plane island 50 × 30 mm on the first plane layer over the second (GND); two ports
    through vias from bottom-layer microstrips, at a corner (-22, -12) and the centre"""
    stk = ctx.stk
    a, b = 50.0, 30.0
    top_p, bot_p = stk.planes[0], stk.planes[1]
    d = stk.z[top_p] - stk.z[bot_p]
    c = Coupon(cid, "G", f"Plane cavity {a:g} × {b:g} mm on {stk.names[top_p]} over {stk.names[bot_p]} (d = {d:.3f} mm), ports at a corner and the centre",
               100.0, 3 * ctx.rp, "cavity resonances (kisi_pi::cavity_modal: ±0.8 % against 3D) and the εr of the core between the planes",
               ["G_pp100", "G_pp200"], {"size_mm": f"{a:g} × {b:g}", "d_mm": round(d, 4)})
    c.islands.append({"layer": top_p, "rect": (-a / 2, -b / 2, a / 2, b / 2), "net": "CAV", "clearance": 0.5})
    c.nets.append("CAV")
    bl = ctx.bot
    p1, p2 = (-22.0, -12.0), (0.0, 0.0)
    xl = launch(c, ctx, -1, p1[1], bl, "CAV", "1")
    xr = launch(c, ctx, 1, p2[1], bl, "CAV", "2")
    straight(c, ctx, bl, p1[1], "CAV", xl, p1[0])
    straight(c, ctx, bl, p2[1], "CAV", p2[0], xr)
    for (x, y) in (p1, p2):
        c.via(x, y, 0, stk.n - 1, "CAV", pads=[top_p, bl])
        c.hole([bot_p], ("circle", x, y, 0.4), f"antipad {cid}")
    er = stk.diel[top_p]["er"]
    modes = sorted((vc.C0 / (2 * math.sqrt(er)) * math.hypot(m / a, n / b) * 1e3, m, n) for m in range(5) for n in range(4) if (m, n) != (0, 0))
    c.params.update(modes_GHz=[round(f / 1e9, 3) for f, m, n in modes if f < 6e9][:8], modes_mn=[f"{m}{n}" for f, m, n in modes if f < 6e9][:8],
                    modes_note="magnetic-wall estimate with the nominal er of the core; fringing lowers them")
    c.window("w60", -30.0, 30.0, half_y=c.H / 2 + 0.6, dfine=0.1, fmax=6e9, note="whole island with 8 mm (port 1) and 30 mm (port 2) of the bottom feeds",
             extra={"nf": 600, "f_loss": 2.5e9, "max_steps": 400000, "meta_modes": [round(f, -6) for f, m, n in modes]})
    return c


def c_info(ctx, X, text):
    c = Coupon("INFO", "info", text, X, 8.0, "")
    return c


# --- boards -------------------------------------------------------------------------------------
def board_specs(ctx, stk_key):
    stk = ctx.stk
    bot = ctx.bot
    sn = stk.names
    ap = lambda d: int(round(d * 1000))
    single = lambda: [
        c_thru(ctx, "THRU_L1", 0),
        c_thru(ctx, "LINE_27", 0, 27.0), c_reflect(ctx, "REFLECT"), c_thru(ctx, "LINE_30", 0, 30.0),
        c_thru(ctx, "LINE_50", 0, 50.0, lines=True), c_thru(ctx, "LINE_100", 0, 100.0),
    ]
    if stk.n == 4:
        a_cases = [c_via(ctx, f"A{ap(d):04d}", "A", f"via L1→L4 through the L2/L3 plane pair, drill 0.3, antipad Ø{d:.1f}, 4 GND vias at 1 mm",
                         [f"A_s4_d300_ap{ap(d)}"] if d <= 1.0 else [], d, vc.ring(4, 1.0),
                         validates="via ladder / coax section: antipad dependence of Z and delay (A group)", params={"drill_mm": 0.3, "antipad_mm": d, "stitch": "4 × 1 mm"})
                   for d in (0.8, 1.0, 1.2)]
        d_cases = [c_via(ctx, f"D_S{str(s).replace('.', 'p')}", "D", f"L1 (ref L2) → L4 (ref L3), one GND via at {s:g} mm, antipad Ø0.8",
                         [f"D_s8_stitch_{str(s).replace('.', 'p')}"] if s in (1.0, 2.0) else [], 0.8, [(0.0, s)],
                         validates="return path of a reference change L2 → L3 (kisi: return via within 3 mm, else inductance up to 5 mm)", params={"stitch_mm": s})
                   for s in (1.0, 2.0, 3.5)]
        d_cases.append(c_via(ctx, "D_NONE", "D", "L1 (ref L2) → L4 (ref L3), no return via near the via (the row walls at ±6.35 mm only)",
                             ["D_s8_stitch_none"], 0.8, [], validates="return path without a nearby GND via: the plane-pair cavity between the row walls",
                             params={"stitch_mm": None}, wide=True))
        a = [*single(), c_thru(ctx, "THRU_L4", bot), *a_cases, *d_cases, c_cavity(ctx, "G_CAV")]
        b = [c_cpwg(ctx, "CPWG_25", 25.0), c_cpwg(ctx, "CPWG_50", 50.0),
             c_pair(ctx, "PAIR_L1", "thru", "pair 2×thru on L1 at the 1.0 mm via pitch (top connectors)", [], 0, False, validates="fixture of the pair via coupons (L1 side)"),
             c_pair(ctx, "PAIR_L4", "thru", "pair 2×thru on L4 at the 1.0 mm via pitch (bottom connectors)", [], bot, False, validates="fixture of the pair via coupons (L4 side)"),
             c_pair(ctx, "E_R_G2", "E", "pair vias L1→L4, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, True, False, 2,
                    validates="differential via ladder: Sdd21, Zdiff, Scd21", params={"pitch_mm": 1.0, "antipad": "round", "gnd_vias": 2}),
             c_pair(ctx, "E_O_G2", "E", "pair vias L1→L4, pitch 1.0, one oval antipad, 2 GND vias", ["E_s8_L1L8_p1000_oval_g2"], bot, True, True, 2,
                    validates="oval (non-round) antipad: kisi's equivalent radius", params={"pitch_mm": 1.0, "antipad": "oval", "gnd_vias": 2}),
             c_pair(ctx, "E_R_G0", "E", "pair vias L1→L4, pitch 1.0, round antipads, no GND vias", ["E_s8_L1L8_p1000_round_g0"], bot, True, False, 0,
                    validates="pair via without return vias", params={"pitch_mm": 1.0, "antipad": "round", "gnd_vias": 0}),
             c_coupled(ctx, "XT_MS", "xt", "Microstrip aggressor and victim on L1, edge gap = w", ["T_ts6_xt_ms_s10"],
                       "NEXT / FEXT (kisi's NEXT is 70–80 % high against 3D, FEXT +6 %)"),
             c_coupled(ctx, "DIFF_MS", "diff", "Edge-coupled 100 Ω microstrip pair on L1", ["T_ts6_diff_ms"],
                       "Zdiff (TDR plateau), εeff of the odd mode, Sdd21 per mm")]
        return {"a": a, "b": b}
    inner = [L for L in stk.signal if 0 < L < stk.n - 1]
    a = [c_thru(ctx, "THRU_L1", 0), c_thru(ctx, f"THRU_L{bot + 1}", bot),
         c_via(ctx, "B_L1L6", "B", "through via L1→L6 (no stub), drill 0.3, antipad Ø0.8 on L2/L5, 4 GND vias at 1 mm", ["B_ts6_L1L6", "B_s8_L1L8_d300"], 0.8,
               vc.ring(4, 1.0), validates="through via without stub (via ladder)", params={"drill_mm": 0.3, "antipad_mm": 0.8}),
         c_via(ctx, "D_S1", "D", "L1 (ref L2) → L6 (ref L5), one GND via at 1 mm", ["D_s8_stitch_1p0"], 0.8, [(0.0, 1.0)],
               validates="return path L2 → L5", params={"stitch_mm": 1.0}),
         c_via(ctx, "D_NONE", "D", "L1 (ref L2) → L6 (ref L5), no return via near the via (row walls only)", ["D_s8_stitch_none"], 0.8, [],
               validates="return path without a nearby GND via", params={"stitch_mm": None}, wide=True),
         c_vlv(ctx, "VLV_5", 5.0, inner[0], f"L1 → {sn[inner[0]]} → L1 (through vias with stubs to L6), 5 mm stripline",
               "inner-layer exit with an open stub (B group, junction model): the via–line–via as a whole", ["B_ts6_L1L3_stub", "B_s8_L1L3_stub_d300"]),
         c_vlv(ctx, "SL_25", 25.0, inner[0], f"Stripline {sn[inner[0]]}, 25 mm between the via transitions", "stripline γ by line–line (with VLV_5 and SL_50), Zc from the TDR plateau"),
         c_vlv(ctx, "SL_50", 50.0, inner[0], f"Stripline {sn[inner[0]]}, 50 mm between the via transitions",
               "asymmetric stripline Zc / εeff (kisi formula +11 % on the ThunderScope stack)", ["T_ts6_sl_10", "T_ts6_sl_20"], lines=True),
         c_thru(ctx, "LINE_50", 0, 50.0, lines=True)]
    b = [c_pair(ctx, "PAIR_L1", "thru", "pair 2×thru on L1 at the 1.0 mm via pitch", [], 0, False, validates="fixture of the pair via coupons (L1 side)"),
         c_pair(ctx, "PAIR_L6", "thru", "pair 2×thru on L6 at the 1.0 mm via pitch (bottom connectors)", [], bot, False, validates="fixture of the pair via coupons (L6 side)"),
         c_pair(ctx, "E_R_G2", "E", "pair vias L1→L6, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, True, False, 2,
                validates="differential via ladder through two plane layers", params={"pitch_mm": 1.0, "antipad": "round", "gnd_vias": 2}),
         c_pair(ctx, "E_O_G2", "E", "pair vias L1→L6, pitch 1.0, one oval antipad, 2 GND vias", ["E_s8_L1L8_p1000_oval_g2"], bot, True, True, 2,
                validates="oval antipad", params={"pitch_mm": 1.0, "antipad": "oval", "gnd_vias": 2}),
         c_coupled_inner(ctx, "BS_L3L4", "bs", f"Broadside: {sn[inner[0]]} over {sn[inner[1]]} across the 0.21 mm prepreg, offset 0",
                         ["T_ts6_broadside_o0"], "broadside NEXT / FEXT (kisi detects it, does not compute it yet)"),
         c_coupled_inner(ctx, "DIFF_SL", "diff", f"Edge-coupled 100 Ω stripline pair on {sn[inner[0]]}", ["T_ts6_diff_sl"],
                         "stripline Zdiff (needs a plateau value: open point in TESTS.md)")]
    return {"a": a, "b": b}


BOARDS = {"jlc4_a": ("jlc4_7628", "a"), "jlc4_b": ("jlc4_7628", "b"), "jlc6_a": ("jlc6_7628", "a"), "jlc6_b": ("jlc6_7628", "b"),
          "aisler4_a": ("aisler4_assumed", "a")}


# --- kisi-fdtd cases ------------------------------------------------------------------------------
def in_rect(x, y, r, m=0.0):
    return r[0] - m <= x <= r[2] + m and r[1] - m <= y <= r[3] + m


def fdtd_case(board, G, c, win, mask=True):
    """the coupon inside the window as a kisi-fdtd case (via_cases.Builder): planes, traces
    (zero thickness, widened for the copper thickness), vias, antipads, pours, the solder mask as
    a dielectric layer; lumped 50 Ω ports where traces leave the window"""
    stk = board.stk
    x0, x1 = c.cx + win["x0"], c.cx + win["x1"]
    hy = win["half_y"] or min(c.H / 2 + 0.65, 7.0)
    xc, yc = (x0 + x1) / 2, c.cy
    W = (x0, yc - hy, x1, yc + hy)
    half_x = (x1 - x0) / 2 + 1.5
    has_island = any(isl["cid"] == c.cid for isl in G["islands"])
    key = stk.vc_key
    if has_island:  # the island layer is not a full plane in this window
        key = stk.vc_key + "_g"
        st = dict(vc.STACKUPS[stk.vc_key])
        st["planes"] = [p for p in stk.planes if p not in [isl["layer"] for isl in G["islands"]]]
        vc.STACKUPS[key] = st
    b = vc.Builder(key, half_x, hy, win["fmax"], win["dfine"])
    b.case["mesh"]["thirds"] = True
    loc = lambda x, y: (round(x - xc, 5), round(y - yc, 5))
    # traces: x-directed pieces inside the window
    ports = []
    for t in G["traces"]:
        if t["cid"] != c.cid:
            continue
        L = t["layer"]
        wf = round(t["w"] + stk.dw_thick(L), 4)
        for a, q in zip(t["pts"][:-1], t["pts"][1:]):
            if abs(a[1] - q[1]) > 1e-6:
                if in_rect(a[0], a[1], W) or in_rect(q[0], q[1], W):
                    if abs(a[0] - q[0]) > 1e-6:
                        raise ValueError(f"{c.cid}/{win['name']}: diagonal trace inside the 3D window")
                    ya, yb = sorted((a[1], q[1]))
                    xa_, _ = loc(a[0], 0)
                    b.case["sheets"].append({"z": stk.z[L], "rect": [round(xa_ - wf / 2, 5), round(ya - yc, 5), round(xa_ + wf / 2, 5), round(yb - yc, 5)], "net": "SIG"})
                continue
            if not (W[1] <= a[1] <= W[3]):
                continue
            lo, hi = sorted((a[0], q[0]))
            if hi < x0 - 1e-9 or lo > x1 + 1e-9:
                continue
            ca, cb = max(lo, x0), min(hi, x1)
            if cb - ca < 1e-6:
                continue
            xa_, ya_ = loc(ca, a[1])
            xb_, _ = loc(cb, a[1])
            b.trace(L, xa_, xb_, ya_, wf)
            for xe, edge in ((ca, x0), (cb, x1)):
                if abs(xe - edge) < 1e-9 and (hi > edge + 1e-9 if edge == x1 else lo < edge - 1e-9):
                    ports.append((loc(xe, a[1]), wf, L))
    # pours, island, holes, vias
    for p in G["pours"]:  # this coupon's pours, ending at the port planes
        if p["cid"] != c.cid:
            continue
        r = p["rect"]
        rr = (max(r[0], W[0]), max(r[1], W[1]), min(r[2], W[2]), min(r[3], W[3]))
        if rr[2] - rr[0] > 0.05 and rr[3] - rr[1] > 0.05:
            a_ = loc(rr[0], rr[1])
            b_ = loc(rr[2], rr[3])
            b.sheet(stk.z[p["layer"]], [a_[0], a_[1], b_[0], b_[1]], net="GND")
    for isl in G["islands"]:
        if isl["cid"] != c.cid:
            continue
        r = isl["rect"]
        g = isl.get("clearance", 0.5)
        z = stk.z[isl["layer"]]
        a_, b_ = loc(r[0], r[1]), loc(r[2], r[3])
        b.sheet(z, [a_[0], a_[1], b_[0], b_[1]], net="PWR")
        dom = [-half_x, -hy, half_x, hy]
        for rr in ((dom[0], dom[1], dom[2], a_[1] - g), (dom[0], b_[1] + g, dom[2], dom[3]), (dom[0], a_[1] - g, a_[0] - g, b_[1] + g), (b_[0] + g, a_[1] - g, dom[2], b_[1] + g)):
            b.sheet(z, list(rr), net="GND")
    for h in G["holes"]:
        if h["cid"] != c.cid:
            continue
        bb = shape_bbox(h["shape"])
        if not in_rect((bb[0] + bb[2]) / 2, (bb[1] + bb[3]) / 2, (x0 - 1.5, W[1], x1 + 1.5, W[3])):
            continue
        s = h["shape"]
        for L in h["layers"]:
            if L not in vc.STACKUPS[key]["planes"]:
                continue
            if s[0] == "circle":
                p_ = loc(s[1], s[2])
                b.antipad(L, p_[0], p_[1], s[3])
            elif s[0] == "slot":
                p_, q_ = loc(s[1], s[2]), loc(s[3], s[4])
                b.antipad(L, p_[0], p_[1], s[5], q_[0], q_[1])
    for v in G["vias"]:  # this coupon's vias (launch fences only inside the window) and the row walls
        if v["cid"] not in (c.cid, "wall"):
            continue
        m = 0.0 if v.get("fence") else 1.5
        if not in_rect(v["x"], v["y"], (x0 - m, W[1], x1 + m, W[3])):
            continue
        p_ = loc(v["x"], v["y"])
        if v["role"] == "gnd":
            b.via(p_[0], p_[1], v["drill"] / 2, v["pad"] / 2, 0, stk.n - 1, [], net="GND")
        else:
            pads = v["pads"] if v["pads"] is not None else list(range(stk.n))
            b.via(p_[0], p_[1], v["drill"] / 2, v["pad"] / 2, 0, stk.n - 1, pads, net="SIG")
    # ports: near (left) ports first, upper y first; 4-port order 1 P near, 2 N near, 3 P far, 4 N far
    ports.sort(key=lambda t: (t[0][0] > 0, -t[0][1]))
    for k, ((x, y), wf, L) in enumerate(ports):
        b.port(x, y, wf, L, stk.port_ref(L), str(k + 1))
    for k_, v_ in win["extra"].items():
        if k_ == "meta_modes":
            b.case["meta_modes"] = v_
        else:
            b.case[k_] = v_
    cid = f"{board.name}__{c.cid}__{win['name']}"
    case = b.finish(cid, win["group"], c.title + f" [{win['name']}: {win['note']}]", dict(c.params, window_mm=[win["x0"], win["x1"]]))
    if mask:  # solder mask as a dielectric layer on both sides (zero-thickness copper inside it)
        t = stk.mask["thickness"]
        zt, zb = stk.z[0], stk.z[-1]
        case["layers"].append({"z0": round(zt, 6), "z1": round(zt + t, 6), "er": stk.mask["er"], "tand": stk.mask["tand"]})
        case["layers"].insert(0, {"z0": round(zb - t, 6), "z1": round(zb, 6), "er": stk.mask["er"], "tand": stk.mask["tand"]})
    if win["excite"] is not None:
        case["meta"]["excite"], case["meta"]["mirror"] = win["excite"], win["mirror"]
    case["meta"].update({"board": board.name, "coupon": c.cid, "window": win["name"], "stackup_source": stk.d["title"],
                         "validates": c.validates, "references": c.refs, "trace_width_note": "zero-thickness strips widened by Wheeler's Δw for the copper thickness",
                         "mask": bool(mask)})
    return case


# --- main -------------------------------------------------------------------------------------------
def build(board_name, ks, pair_pitch, outdir, design_cache, fdtd_exe=None):
    stk_key, part = BOARDS[board_name]
    stk = Stack(stk_key)
    if stk_key not in design_cache:
        cpath = os.path.join(outdir, "boards", f"{stk_key}.design.json")
        des = design(stk, ks, pair_pitch)
        des["z_table"] = report_z(stk, ks, des)
        des["stackup"] = stk_key
        des["date"] = TODAY
        json.dump(des, open(cpath, "w"), indent=1)
        design_cache[stk_key] = des
    des = design_cache[stk_key]
    ctx = Ctx(stk, des, pair_pitch)
    coupons = board_specs(ctx, stk_key)[part]
    # rows sorted by length (stable): every row rests on a row at least as long (a stepped pyramid)
    coupons.sort(key=lambda c: c.X)
    X = max(c.X for c in coupons)
    coupons.append(c_info(ctx, X, ""))
    name = f"kisi-coupons-{board_name.replace('_', '-')}"
    b = Board(name, stk, coupons, title=stk.d["title"])
    b.design = des
    info = coupons[-1]
    b.place()
    yi = info.cy
    b.texts.append((f"{name}  -  {stk.d['template'] or stk.d['title'][:40]}  -  {TODAY}", -X / 2 + 2.0, yi + 1.6, "F.SilkS", 1.2, "left"))
    b.texts.append(("kisi measurement coupons - github.com/riehlies/kisi - validation/test_board", -X / 2 + 2.0, yi - 1.0, "F.SilkS", 0.9, "left"))
    b.texts.append((f"{name} (bottom)", X / 2 - 2.0, yi, "B.SilkS", 1.0, "left"))
    if stk.d["manufacturer"] == "JLCPCB":  # JLC prints its order number where this text is
        b.texts.append(("JLCJLCJLCJLC", X / 2 - 2.0, yi + 2.0, "B.SilkS", 1.0, "left"))
    path = os.path.join(outdir, "boards", name + ".kicad_pcb")
    G = b.write_kicad(path)
    b.write_kisi_config(path[:-len(".kicad_pcb")] + ".kisi.json")
    # 3D cases
    fdir = os.path.join(outdir, "fdtd", board_name)
    os.makedirs(fdir, exist_ok=True)
    for f in os.listdir(fdir):
        if f.endswith(".json"):
            os.remove(os.path.join(fdir, f))
    cases, cells = [], {}
    for c in coupons:
        for win in c.windows:
            case = fdtd_case(b, G, c, win)
            cp = os.path.join(fdir, case["name"] + ".json")
            json.dump(case, open(cp, "w"), indent=1)
            cases.append(case["name"])
            if fdtd_exe:  # mesh only: cell count of the case
                m = json.loads(subprocess.run([fdtd_exe, cp, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
                cells[case["name"]] = len(m["x"]) * len(m["y"]) * len(m["z"])
    manifest = {"board": name, "file": os.path.basename(path), "stackup": stk_key, "stackup_title": stk.d["title"], "date": TODAY,
                "size_mm": [round(X, 2), round(-b.bottom, 2)], "thickness_mm": round(stk.thickness(), 4),
                "coupons": [{"id": c.cid, "group": c.group, "title": c.title, "length_mm": c.X, "height_mm": c.H, "validates": c.validates,
                             "references": c.refs, "params": c.params, "nets": c.nets, "connectors": b.jmap.get(c.cid, []),
                             "fdtd": [f"{name}__{c.cid}__{w['name']}" for w in c.windows]} for c in coupons if c.group != "info"],
                "fdtd_cases": cases, "fdtd_cells": cells}
    json.dump(manifest, open(os.path.join(outdir, "boards", name + ".manifest.json"), "w"), indent=1)
    print(f"{name}: {len(coupons) - 1} coupons, {X:g} × {-b.bottom:g} mm, {len(cases)} 3D cases, {len(G['vias'])} vias", flush=True)
    return name, path, manifest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--boards", default="jlc4_a,jlc4_b,jlc6_a,jlc6_b,aisler4_a")
    ap.add_argument("--out", default=HERE)
    ap.add_argument("--kisi", default=os.path.join(REPO, "target", "release", "kisi"))
    ap.add_argument("--kisi-mjs", default=os.path.join(REPO, "dist", "kisi.mjs"))
    ap.add_argument("--solver", default="auto", choices=["auto", "field", "formula"])
    ap.add_argument("--pair-pitch", type=float, default=LAY["pair_pitch"])
    ap.add_argument("--reuse-design", action="store_true", help="use boards/<stackup>.design.json if present (keeps the trace widths; "
                    "stackup values may have changed, e.g. after the extraction)")
    ap.add_argument("--fdtd-exe", default=None, help="kisi-fdtd binary: record the cell count of every 3D case (mesh only)")
    a = ap.parse_args()
    os.makedirs(os.path.join(a.out, "boards"), exist_ok=True)
    ks = Kisi(a.kisi, a.kisi_mjs, a.solver)
    cache = {}
    if a.reuse_design:
        for f in os.listdir(os.path.join(a.out, "boards")):
            if f.endswith(".design.json"):
                d = json.load(open(os.path.join(a.out, "boards", f)))
                if d.get("pair_pitch") == a.pair_pitch:
                    cache[d["stackup"]] = d
    need = {BOARDS[bn][0] for bn in a.boards.split(",")} - set(cache)
    if need:
        print(f"trace widths for {', '.join(sorted(need))} from kisi ({ks.solver}: {a.kisi_mjs if ks.solver == 'field' else a.kisi})", flush=True)
    for bn in a.boards.split(","):
        build(bn, ks, a.pair_pitch, a.out, cache, a.fdtd_exe)
    print(f"kisi probe runs: {ks.calls}")


if __name__ == "__main__":
    main()
