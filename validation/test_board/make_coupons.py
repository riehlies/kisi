#!/usr/bin/env python3
"""Measurement coupons for kisi, design v3 (validation/test_board/README.md).

Per board: a KiCad 8 board (boards/<board>.kicad_pcb) with its project (.kicad_pro, net classes 50Z
and 100Z_Diff), a kisi configuration (.kisi.json, stackup values with sources) and a manifest; per
coupon one or more kisi-fdtd cases (fdtd/<board>/<case>.json, the format of
validation/fdtd/via_cases.py, run with validation/fdtd/run_refs.py).

* Trace widths come from kisi itself: a small probe board per cross-section (the same writer as the
  coupons), solved with the 2D field solver of the Node CLI (kisi.mjs) when it is available, else
  with kisi's formulas (--solver formula); stored in boards/<stackup>.design.json.
* Every coupon is described once (traces, vias, antipads, pours, islands, parts, ports in coupon
  coordinates); the KiCad board and the kisi-fdtd cases are written from that one description.
* All connectors are 2.92 mm edge launches (Samtec 292-J-P-EP-ST-EL-01, to 40 GHz) on the top
  side; every board carries its own 2×thru (and a
  pair 2×thru when it has 4-port coupons), and the via coupons are via–line–via structures fed
  from L1 at both ends, so one fixture per board de-embeds them.
* Board sizes stay within the limits in connectors.json (4 layers 100 × 100 mm, 6 and 8 layers
  50 × 50 mm). Stackups (stackups.json) and connector data (connectors.json) carry their sources;
  the end-launch footprint is a DRAFT read from Samtec's drawing (connectors.json, footprint) and
  the launch on the board is parametric (connectors.json, launch) for the kisi-fdtd optimisation
  (--launch-cases writes its 3D cases).

Coordinates: mm, x along the coupons, y up (KiCad's y points down: converted on output). A board is
a stack of rows; a row is one coupon from the left to the right board edge, or two coupons side by
side that have their ports on the outer edges only. Rows are centred on x = 0 and sorted by length.

usage: python3 make_coupons.py [--boards jlc4_7628_a,...] [--kisi KISI] [--kisi-mjs KISI.MJS]
                               [--solver auto|field|formula] [--pair-pitch 12.7] [--reuse-design]
                               [--fdtd-exe KISI_FDTD] [--launch-cases STACKUP[,…]]
"""
import argparse, datetime, json, math, os, shutil, subprocess, sys, tempfile, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "validation", "fdtd"))
import via_cases as vc  # noqa: E402  (Builder, copper_z, ring: the 3D case format)
import kisi_board as kb  # noqa: E402  (layer names, circle and stadium outlines)

STK = json.load(open(os.path.join(HERE, "stackups.json")))
CON = json.load(open(os.path.join(HERE, "connectors.json")))
CONN = CON["connector"]
FP = CON["footprint"]
LNC = CON["launch"]             # the launch on the board (parametric, DRAFT)
LAY = CON["layout"]
PKG = CON["parts"]
FEED = LAY["feed"]              # board edge to the reference plane of a 2-port coupon
FEED4 = LAY["feed_4port"]       # the same for the 4-port pair coupons
ROW, ROW4 = LAY["row_pitch"], LAY["pair_row"]
LS_VIA, LS_RET = LAY["via_line"], LAY["return_line"]
# traces leave the launch straight until they are clear of the connector's mounting holes
LAUNCH_LEN = max(LNC["launch_length"], LNC["pad_len"] or 0.0, LNC["taper_len"])  # edge to the coupon's own line
X_CLEAR = max(LAUNCH_LEN + 0.3, max(h["x"] + h["d"] / 2 for h in FP["mount_holes"]) + 0.5)
DESIGN_REV = "3"
LIBNICK = "kisi_coupons"         # project-local footprint library: boards/kisi_coupons.pretty + boards/fp-lib-table
LIBRARY = {}                     # footprint name -> .kicad_mod text, collected while the boards are written
# the closest return via next to the 0.3 / 0.6 mm signal via with standard vias: 0.7 mm keeps 0.1 mm
# between the 0.6 mm pads (hole-to-hole 0.4 mm); the 3D reference has 0.5 mm, which overlaps the pads
D_CLOSE = 0.7
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
        self.pwr = (d.get("pdn") or {}).get("pwr")
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
        return abs(self.z[layer] - self.z[self.port_ref(layer)])

    def dw_thick(self, layer):
        """Wheeler's width increase that represents the copper thickness on a zero-thickness strip"""
        t, h = self.cu[layer], self.h_ref(layer)
        return t / math.pi * (1 + math.log(2 * h / t))


# --- coupon description ----------------------------------------------------------------------
class Coupon:
    """A coupon of length X (edge to edge) and height H. Geometry in local coordinates (x = 0 at the
    coupon centre); ports are end-launch connectors at the left (-X/2) or right (+X/2) edge."""

    def __init__(self, cid, group, title, X, H, validates, refs=(), params=None):
        self.cid, self.group, self.title, self.X, self.H = cid, group, title, X, H
        self.validates, self.refs, self.params = validates, list(refs), dict(params or {})
        self.traces, self.vias, self.holes, self.pours, self.islands, self.ports = [], [], [], [], [], []
        self.zones, self.parts, self.windows, self.nets, self.notches = [], [], [], [], []
        self.cx = self.cy = 0.0
        self.measure = ""

    def net(self, n):
        if n not in self.nets:
            self.nets.append(n)

    def trace(self, layer, pts, w, net):
        self.traces.append({"layer": layer, "pts": [tuple(p) for p in pts], "w": w, "net": net})
        self.net(net)

    def via(self, x, y, net, pads=None, role="sig", fence=False):
        v = LAY["signal_via"] if role == "sig" else LAY["gnd_via"]
        self.vias.append({"x": round(x, 4), "y": round(y, 4), "net": net, "pads": pads, "role": role,
                          "drill": v["drill"], "pad": v["pad"], "fence": fence})

    def hole(self, layers, shape, name=""):
        self.holes.append({"layers": list(layers), "shape": tuple(shape), "name": name})

    def pour(self, layer, rect, net="GND"):
        x0, y0, x1, y1 = rect
        if abs(x1 - x0) > 0.05 and abs(y1 - y0) > 0.05:
            self.pours.append({"layer": layer, "rect": (min(x0, x1), min(y0, y1), max(x0, x1), max(y0, y1)), "net": net})

    def port(self, side, y, layer, net, w, gap, name, geom=None):
        self.ports.append({"side": side, "y": y, "layer": layer, "net": net, "w": w, "gap": gap, "name": name, "geom": geom})

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

    def z(self, stk, layer, w, s=None, gap=None, field=None, cut=None):
        """impedance kisi reports for a 20 mm probe line: Z0 (50Z class) or Zdiff (pair, 100Z_Diff);
        cut: width of an opening in the first plane under the trace (the launch on a thin top
        dielectric), over the middle 19 mm of the line - the value of that section is returned"""
        c = Coupon("P", "probe", "probe", 20.0, 10.0, "")
        nets = ["SE_PROBE"] if s is None else ["DP_PROBE_P", "DP_PROBE_N"]
        ys = [0.0] if s is None else [(w + s) / 2, -(w + s) / 2]
        for net, y in zip(nets, ys):
            c.trace(layer, [(-10.0, y), (10.0, y)], w, net)
        if gap is not None:
            c.pour(layer, (-10.0, w / 2 + gap, 10.0, 4.7))
            c.pour(layer, (-10.0, -4.7, 10.0, -w / 2 - gap))
        if cut is not None:
            c.hole([stk.planes[0]], ("rect", -9.5, -cut / 2, 9.5, cut / 2), "launch cutout")
        for k, (net, y) in enumerate(zip(nets, ys)):
            for ref, x in ((f"U{k + 1}", -10.0), (f"J{k + 1}", 10.0)):
                c.parts.append({"prefix": ref, "x": x, "y": y, "value": "IC" if ref[0] == "U" else "conn", "props": {},
                                "pads": [{"num": "1", "x": 0, "y": 0, "sx": w, "sy": w, "shape": "rect", "net": net, "layer": layer}]})
        b = Board("probe", stk, [[c]], probe=True)
        b.design = {"w50": {str(layer): w}, "diff": {str(layer): {"w": w, "s": s or 0.2}}}
        path = os.path.join(self.tmp, "probe.kicad_pcb")
        b.write_kicad(path)
        d = self.run(["net", path, nets[0], "--no-config"], field=field)
        if field or (field is None and self.solver == "field"):
            if not (d.get("field_solver") or {}).get("cross_sections_field"):
                raise RuntimeError("field solver not used on the probe")
        return float(d["impedance"]["max" if cut is not None else "mean"])

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
    sym = stk.d["dielectrics"] == stk.d["dielectrics"][::-1] and stk.cu == stk.cu[::-1]
    out = {"solver": ks.solver, "kisi": os.path.basename(ks.mjs if ks.solver == "field" else ks.exe), "w50": {}, "diff": {}, "pair_via": {}, "notes": []}
    for layer in stk.signal:
        mirror = str(stk.n - 1 - layer)
        if sym and mirror in out["w50"]:
            out["w50"][str(layer)] = out["w50"][mirror]
            continue
        out["w50"][str(layer)] = r5(ks.solve(lambda x, fld: ks.z(stk, layer, x, field=fld), 50.0, 0.04, 2.0))
    # launch: grounded coplanar section at the connector (the connector ground contacts the top
    # copper): gap 0.3 mm, width for 50 Ω; if below the narrowest pad the pin may land on
    # (connectors.json launch.pad_w_min), that width and its gap
    g0 = 0.3
    wl = ks.solve(lambda x, fld: ks.z(stk, 0, x, gap=g0, field=fld), 50.0, 0.04, 2.0)
    out["launch"] = {"w": r5(wl), "gap": r5(g0)}
    if wl < LNC["pad_w_min"]:
        # thin top dielectric: the pad must carry the connector pin, so it is pin-wide; 50 Ω comes
        # from an opening in the first plane under the launch (the same width as the coplanar
        # opening on top, reference = the next plane) and the top gap is solved; JLC's minimum
        # spacing (0.1 mm) bounds the gap from below
        wl = LNC["pad_w_min"]
        g0 = max(0.1, ks.solve(lambda x, fld: ks.z(stk, 0, wl, gap=x, cut=wl + 2 * x, field=fld), 50.0, 0.06, 0.4, increasing=True))
        out["launch"] = {"w": r5(wl), "gap": r5(g0), "cut": r5(wl + 2 * r5(g0)), "cut_layer": stk.planes[0],
                         "note": "pin-wide launch with an opening in the first plane under it (thin top dielectric); DRAFT like the footprint"}
    # tightly coupled 100 Ω pair (outer layer and the first inner signal layer), spacing 0.2 mm
    for layer in [0] + [L for L in stk.signal if 0 < L < stk.n - 1][:1]:
        out["diff"][str(layer)] = {"w": r5(ks.solve(lambda x, fld: ks.z(stk, layer, x, s=0.2, field=fld), 100.0, 0.04, 1.5)), "s": 0.2}
    # pair at the via pitch (1.0 mm): width for 100 Ω differential, edge gap 1.0 - w, every signal layer
    for layer in stk.signal:
        mirror = str(stk.n - 1 - layer)
        if sym and mirror in out["pair_via"]:
            out["pair_via"][str(layer)] = out["pair_via"][mirror]
            continue
        w = r5(ks.solve(lambda x, fld: ks.z(stk, layer, x, s=1.0 - x, field=fld), 100.0, 0.08, 0.7))
        out["pair_via"][str(layer)] = {"w": w, "s": round(1.0 - w, 3), "pitch": 1.0}
    # CPWG coupon: the gap of the LibreVNA RF class (0.18 mm, 3D reference T_vna6_cpwg), width for 50 Ω
    out["cpwg"] = {"w": r5(ks.solve(lambda x, fld: ks.z(stk, 0, x, gap=0.18, field=fld), 50.0, 0.04, 1.0)), "gap": 0.18,
                   "note": "gap of the LibreVNA RF class (T_vna6_cpwg), width for 50 Ω"}
    out["pair_pitch"] = pair_pitch
    return out


def report_z(stk, ks, des):
    """impedance of every designed cross-section with both kisi solvers (for the README)"""
    rows = []
    have_field = ks.mjs and os.path.exists(ks.mjs) and shutil.which("node")

    def both(label, layer, w, s=None, gap=None, target=50.0, cut=None):
        zf = ks.z(stk, layer, w, s=s, gap=gap, field=False, cut=cut)
        zd = ks.z(stk, layer, w, s=s, gap=gap, field=True, cut=cut) if have_field else None
        rows.append({"what": label, "layer": stk.names[layer], "w_mm": w, "s_mm": s, "gap_mm": gap, "cut_mm": cut, "target_ohm": target,
                     "z_formula": round(zf, 2), "z_field": round(zd, 2) if zd else None,
                     "dev_formula_pct": round((zf / target - 1) * 100, 1), "dev_field_pct": round((zd / target - 1) * 100, 1) if zd else None})
    for L, w in des["w50"].items():
        both("50 Ω line" + (" (microstrip)" if int(L) in (0, stk.n - 1) else " (stripline)"), int(L), w)
    lc = des["launch"]
    both("launch (grounded coplanar" + (f", {stk.names[lc['cut_layer']]} cut out {lc['cut']:g} mm)" if lc.get("cut") else ")"), 0, lc["w"], gap=lc["gap"], cut=lc.get("cut"))
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


def shift_shape(s, dx, dy):
    if s[0] == "circle":
        return (s[0], s[1] + dx, s[2] + dy, s[3])
    if s[0] == "slot":
        return (s[0], s[1] + dx, s[2] + dy, s[3] + dx, s[4] + dy, s[5])
    return (s[0], s[1] + dx, s[2] + dy, s[3] + dx, s[4] + dy)


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
        out.append((a[1] if a[0] == "v" else b[1], a[1] if a[0] == "h" else b[1]))
    return out


def notch_outline(poly, notches, x_edge_tol=0.6):
    """cut rectangular notches into the left / right edges of a counter-clockwise rectilinear
    polygon (an opening in a plane under an end launch): notch = {side, y0, y1, x} with x the
    inner end of the notch; the edge is the vertical edge on that side that spans y0 … y1"""
    pts = list(poly)
    for nt in notches:
        for i in range(len(pts)):
            p, q = pts[i], pts[(i + 1) % len(pts)]
            if abs(p[0] - q[0]) > 1e-9:
                continue
            down = q[1] < p[1]
            if down != (nt["side"] < 0) or abs(abs(p[0]) - abs(nt["x_edge"])) > x_edge_tol:
                continue
            lo, hi = min(p[1], q[1]), max(p[1], q[1])
            if not (lo + 1e-6 < nt["y0"] and nt["y1"] < hi - 1e-6):
                continue
            x, xn = p[0], nt["x"]
            ins = [(x, nt["y1"]), (xn, nt["y1"]), (xn, nt["y0"]), (x, nt["y0"])] if down else [(x, nt["y0"]), (xn, nt["y0"]), (xn, nt["y1"]), (x, nt["y1"])]
            pts[i + 1:i + 1] = ins
            break
        else:
            raise ValueError(f"notch at y {nt['y0']:.2f} … {nt['y1']:.2f}: no board edge found")
    return pts


def fractured(outline, holes, xsplit=0.0):
    """polygon with holes as one outline (KiCad's fractured fill): each hole is joined by a
    horizontal cut to the left edge (hole centre x < xsplit) or the right edge. `outline`:
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
            if ((xc < xsplit) == down) and lo + 1e-6 < yc < hi - 1e-6:
                sel.append((yc, shape_ring(h)[:-1]))
        sel.sort(key=lambda t: -t[0] if down else t[0])
        for yc, ring in sel:
            # the ring point nearest to this edge, close to the hole's centre line
            k = min(range(len(ring)), key=lambda j: abs(ring[j][0] - p[0]) + 10 * abs(ring[j][1] - yc))
            r = ring[k:] + ring[:k]
            out += [(p[0], r[0][1]), r[0]] + r[:0:-1] + [r[0], (p[0], r[0][1])]  # the hole runs clockwise
    return out


def seg_dist(px, py, a, b):
    (x0, y0), (x1, y1) = a, b
    dx, dy = x1 - x0, y1 - y0
    l2 = dx * dx + dy * dy
    t = 0.0 if l2 == 0 else max(0.0, min(1.0, ((px - x0) * dx + (py - y0) * dy) / l2))
    return math.hypot(px - x0 - t * dx, py - y0 - t * dy)


def clear_of_traces(x, y, r, segs):
    for (L, pts, w) in segs:
        for a, b in zip(pts[:-1], pts[1:]):
            if seg_dist(x, y, a, b) < r + w / 2:
                return False
    return True


def gnd_pad_outline():
    """one Samtec ground pad in (u from the board edge, v across, v > 0 side), clipped at the
    edge pull-back; the other pad is its mirror (connectors.json footprint.gnd_pad)"""
    gp, pb = FP["gnd_pad"], FP["edge_pullback"]
    fing = [tuple(q) for q in gp["fingers"]]
    (u1, v1), (u0, v0) = fing[-2], fing[-1]
    v_pb = v0 + (v1 - v0) * (pb - u0) / (u1 - u0)
    return ([(pb, gp["half_width"]), (gp["chamfer_start"], gp["half_width"]), (gp["depth"], gp["chamfer_half_width"]),
             (gp["depth"], gp["inner_half"]), (fing[0][0], gp["inner_half"])] + [q for q in fing[:-1] if q[0] > pb] + [(pb, v_pb)])


def gnd_pad_polys(port):
    """the two ground pads of a connector at `port` (board coordinates)"""
    xe, side, y = port["x"], port["side"], port["y"]
    return [[(xe - side * u, y + sg * v) for u, v in gnd_pad_outline()] for sg in (1, -1)]


def in_poly(x, y, poly):
    inside = False
    for (x0, y0), (x1, y1) in zip(poly, poly[1:] + poly[:1]):
        if (y0 > y) != (y1 > y) and x < x0 + (y - y0) * (x1 - x0) / (y1 - y0):
            inside = not inside
    return inside


def rect_hits_polys(r, polys, step=0.05):
    nx, ny = max(2, int((r[2] - r[0]) / step) + 1), max(2, int((r[3] - r[1]) / step) + 1)
    return any(in_poly(r[0] + (r[2] - r[0]) * i / (nx - 1), r[1] + (r[3] - r[1]) * j / (ny - 1), q)
               for i in range(nx) for j in range(ny) for q in polys)


# --- board ---------------------------------------------------------------------------------------
class Board:
    def __init__(self, name, stk, rows, probe=False):
        self.name, self.stk, self.probe = name, stk, probe
        self.rows = [list(r) for r in rows]
        self.coupons = [c for r in self.rows for c in r]
        self.design = {}
        self.texts = []

    def place(self):
        y = 0.0
        for row in self.rows:
            W = sum(c.X for c in row)
            H = max(c.H for c in row)
            x = -W / 2
            for c in row:
                c.cx, c.cy = x + c.X / 2, y - H / 2
                x += c.X
                if len(row) > 1:  # side by side: ports on the outer edges only
                    outer = -1 if c is row[0] else 1
                    assert all(p["side"] == outer for p in c.ports), f"{c.cid}: port on an inner edge"
            y -= H
        self.bottom, self.top = y, 0.0

    def row_extents(self):
        out, y = [], 0.0
        for row in self.rows:
            H = max(c.H for c in row)
            out.append((y, y - H, sum(c.X for c in row) / 2))
            y -= H
        return out

    def outline(self):
        """counter-clockwise stepped outline from the rows"""
        rows = self.row_extents()
        pts = []
        for (yt, yb, hx) in rows:  # down the left side
            pts += [(-hx, yt), (-hx, yb)]
        for (yt, yb, hx) in reversed(rows):  # up the right side
            pts += [(hx, yb), (hx, yt)]
        out = []
        for p in pts:
            if not (out and abs(out[-1][0] - p[0]) < 1e-9 and abs(out[-1][1] - p[1]) < 1e-9):
                out.append(p)
        m = len(out)
        return [out[i] for i in range(m) if not ((abs(out[i - 1][0] - out[i][0]) < 1e-9 and abs(out[i][0] - out[(i + 1) % m][0]) < 1e-9) or
                                                 (abs(out[i - 1][1] - out[i][1]) < 1e-9 and abs(out[i][1] - out[(i + 1) % m][1]) < 1e-9))]

    def width(self):
        return max(sum(c.X for c in r) for r in self.rows)

    def height(self):
        return -self.bottom

    def gather(self):
        """all primitives in board coordinates, walls included"""
        G = {k: [] for k in ("traces", "vias", "holes", "pours", "islands", "ports", "zones", "parts", "notches")}
        for c in self.coupons:
            dx, dy = c.cx, c.cy
            for t in c.traces:
                G["traces"].append(dict(t, pts=[(x + dx, y + dy) for x, y in t["pts"]], cid=c.cid))
            for v in c.vias:
                G["vias"].append(dict(v, x=v["x"] + dx, y=v["y"] + dy, cid=c.cid))
            for h in c.holes:
                G["holes"].append(dict(h, shape=shift_shape(h["shape"], dx, dy), cid=c.cid))
            for p in c.pours:
                x0, y0, x1, y1 = p["rect"]
                G["pours"].append(dict(p, rect=(x0 + dx, y0 + dy, x1 + dx, y1 + dy), cid=c.cid))
            for p in c.islands + c.zones:
                x0, y0, x1, y1 = p["rect"]
                key = "islands" if p in c.islands else "zones"
                G[key].append(dict(p, rect=(x0 + dx, y0 + dy, x1 + dx, y1 + dy), holes=[shift_shape(s, dx, dy) for s in p.get("holes", [])], cid=c.cid))
            for p in c.ports:
                G["ports"].append(dict(p, x=dx + p["side"] * c.X / 2, y=p["y"] + dy, cid=c.cid))
            for nt in c.notches:
                xe = dx + nt["side"] * c.X / 2
                G["notches"].append(dict(nt, x_edge=xe, x=xe - nt["side"] * nt["depth"], y0=nt["y0"] + dy, y1=nt["y1"] + dy, cid=c.cid))
            for p in c.parts:
                G["parts"].append(dict(p, x=p["x"] + dx, y=p["y"] + dy, cid=c.cid))
        if not self.probe:
            G["vias"] += self.walls(G)
        return G

    def walls(self, G):
        """GND via walls along every row boundary (and the board's top and bottom), pitch
        wall_pitch, through all layers; they also stitch the launch pours"""
        out = []
        p = LAY["wall_pitch"]
        ext = self.row_extents()
        bounds = [(ext[0][0] - 0.75, ext[0][2])] + [(ext[i][0], min(ext[i][2], ext[i - 1][2])) for i in range(1, len(ext))] + [(ext[-1][1] + 0.75, ext[-1][2])]
        segs = [(t["layer"], t["pts"], t["w"]) for t in G["traces"]]
        isl = [i["rect"] for i in G["islands"]]
        pos = []
        for (y, hx) in bounds:
            n = int((2 * (hx - WALL_EDGE)) / p)
            pos += [(-n * p / 2 + k * p, y, hx) for k in range(n + 1)]
        for i in range(1, len(ext)):  # the exposed top edge of a row that is longer than the row above it
            if ext[i][2] > ext[i - 1][2] + 1.0:
                y, x0, x1 = ext[i][0] - 0.75, ext[i - 1][2] + 0.75, ext[i][2] - WALL_EDGE
                n = int((x1 - x0) / p)
                pos += [(sg * (x0 + k * p), y, ext[i][2]) for k in range(n + 1) for sg in (1, -1)]
        for (x, y, hx) in pos:
            if abs(x) > hx - WALL_EDGE + 1e-9 or not clear_of_traces(x, y, LAY["gnd_via"]["pad"] / 2 + 0.15, segs):
                continue
            if any(r[0] - 1.0 < x < r[2] + 1.0 and r[1] - 1.0 < y < r[3] + 1.0 for r in isl):
                continue
            out.append({"x": x, "y": y, "net": "GND", "pads": None, "role": "gnd", "fence": False,
                        "drill": LAY["gnd_via"]["drill"], "pad": LAY["gnd_via"]["pad"], "cid": "wall"})
        return out

    # --- KiCad output ------------------------------------------------------------------------
    def write_kicad(self, path):
        stk = self.stk
        self.place()
        G = self.gather()
        W = KicadText(self.name)
        W.collect = not self.probe
        outline = self.outline()
        hx = self.width() / 2
        W.ox, W.oy = 30.0 + hx, 30.0
        W.head(stk, stk.thickness())
        nets = ["", "GND"]
        for c in self.coupons:
            for n_ in c.nets:
                if n_ not in nets:
                    nets.append(n_)
        for p in G["parts"]:
            for pd in p["pads"]:
                if pd["net"] not in nets:
                    nets.append(pd["net"])
        W.nets(nets)
        # the connector's ground pads (top and bottom copper) and screw holes: nothing of another net there
        gp = FP["gnd_pad"]
        for p in G["ports"]:
            xe = p["x"]
            r = (min(xe, xe - p["side"] * (gp["depth"] + 0.3)), p["y"] - gp["half_width"] - 0.3, max(xe, xe - p["side"] * (gp["depth"] + 0.3)), p["y"] + gp["half_width"] + 0.3)
            for isl in G["islands"] + G["zones"]:
                q = isl["rect"]
                if isl["layer"] not in (0, self.stk.n - 1):  # inner layers: only the plated holes' round pads
                    rr = FP["hole_ring"] / 2 + isl.get("clearance", 0.2)
                    if any(q[0] - rr < xe - p["side"] * h["x"] < q[2] + rr and q[1] - rr < p["y"] + h["y"] < q[3] + rr for h in FP["mount_holes"]):
                        raise ValueError(f"{self.name}: {isl['net']} island of {isl['cid']} too close to a connector screw hole of {p['cid']}")
                    continue
                cl = isl.get("clearance", 0.2)
                if q[0] < r[2] and r[0] < q[2] and q[1] < r[3] and r[1] < q[3] and rect_hits_polys((q[0] - cl, q[1] - cl, q[2] + cl, q[3] + cl), gnd_pad_polys(p)):
                    raise ValueError(f"{self.name}: {isl['net']} {'island' if isl in G['islands'] else 'patch'} of {isl['cid']} under the connector ground pads of {p['cid']}")
            for t in G["traces"]:
                if t["net"] in (p["net"], "GND") or t["layer"] not in (0, self.stk.n - 1):
                    continue
                if any(r[0] < x < r[2] and r[1] < y < r[3] and rect_hits_polys((x - t["w"] / 2 - 0.2, y - t["w"] / 2 - 0.2, x + t["w"] / 2 + 0.2, y + t["w"] / 2 + 0.2), gnd_pad_polys(p)) for x, y in t["pts"]):
                    raise ValueError(f"{self.name}: trace {t['net']} under the connector ground pads of {p['cid']}")
        # connectors and parts
        self.jmap, self.refmap = {}, {}
        count = {}
        for p in G["ports"]:
            count["J"] = count.get("J", 0) + 1
            ref = f"J{count['J']}"
            self.jmap.setdefault(p["cid"], []).append({"ref": ref, "port": p["name"], "side": "left" if p["side"] < 0 else "right",
                                                       "layer": stk.names[p["layer"]], "net": p["net"]})
            W.endlaunch(ref, p)
        for p in G["parts"]:
            pre = p["prefix"]
            if pre in ("C", "U", "R"):
                count[pre] = count.get(pre, 0) + 1
                ref = f"{pre}{count[pre]}"
            else:
                ref = pre
            self.refmap.setdefault(p["cid"], []).append({"ref": ref, "role": p.get("role", ""), "value": p["value"], "package": p.get("package"),
                                                         "populate": p.get("populate")})
            W.part(ref, p, stk)
        W.gr_poly(outline, "Edge.Cuts", 0.1)
        for c in self.coupons:
            left = not any(pp["side"] > 0 for pp in c.ports) or any(pp["side"] < 0 for pp in c.ports)
            lx = c.cx - c.X / 2 + X_CLEAR + 0.3 if left else c.cx + c.X / 2 - X_CLEAR - 0.3
            W.text(c.cid, lx, c.cy + c.H / 2 - 1.2, "F.SilkS", 0.8, justify="left" if left else "right")
        for t in self.texts:
            W.text(*t)
        for t in G["traces"]:
            for a, b in zip(t["pts"][:-1], t["pts"][1:]):
                W.segment(a, b, t["w"], stk.names[t["layer"]], t["net"])
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
            nts = [nt for nt in G["notches"] if nt["layer"] == L]
            fill = notch_outline(inset, nts) if nts else inset
            W.zone("GND", stk.names[L], inset, [fractured(fill, holes)], name=f"GND plane {stk.names[L]}", clearance=0.2)
        # zone priorities: KiCad reports overlapping zones of equal priority on a layer, so every zone gets its
        # own: planes 0, the coupons' GND pours 1, 2, … (in the order written), islands and patches above them
        n_pour = len(G["pours"])
        for k, isl in enumerate(G["islands"] + G["zones"]):
            r = isl["rect"]
            W.zone(isl["net"], stk.names[isl["layer"]], rect_pts(r), [fractured(rect_pts(r), isl["holes"], (r[0] + r[2]) / 2)],
                   name=f"{isl['net']} {'island' if isl in G['islands'] else 'patch'}", clearance=0.1 if isl in G["zones"] else 0.2, priority=n_pour + 1 + k)
        # keepouts that reproduce the antipads and island clearances when KiCad refills the zones
        for h in G["holes"]:
            if h["shape"][0] != "rect":
                W.keepout([stk.names[L] for L in h["layers"]], shape_ring(h["shape"], 32)[:-1], h.get("name") or "antipad")
        for nt in G["notches"]:  # the launch openings, extending past the board edge
            xo = nt["x_edge"] + (0.5 if nt["side"] > 0 else -0.5)
            W.keepout([stk.names[nt["layer"]]], rect_pts((min(xo, nt["x"]), nt["y0"], max(xo, nt["x"]), nt["y1"])), "launch opening")
        for isl in G["islands"]:
            x0, y0, x1, y1 = isl["rect"]
            g = isl.get("clearance", 0.5)
            W.keepout_frame(stk.names[isl["layer"]], (x0 - g, y0 - g, x1 + g, y1 + g), isl["rect"], f"{isl['net']} clearance")
            for s in isl["holes"]:
                W.keepout([stk.names[isl["layer"]]], shape_ring(s, 24)[:-1], f"{isl['net']} via clearance")
        for k, p in enumerate(G["pours"]):
            W.zone(p["net"], stk.names[p["layer"]], rect_pts(p["rect"]), [rect_pts(p["rect"])], name=f"GND {p['cid']}", clearance=0.1, priority=1 + k)
        W.notes(self, -hx, self.bottom - 5.0)
        W.close()
        open(path, "w").write(W.text_out())
        self.write_pro(path[:-len(".kicad_pcb")] + ".kicad_pro")
        return G

    def write_pro(self, path):
        d = self.design
        w50 = d.get("w50", {}).get("0", 0.2)
        dp = d.get("diff", {}).get("0", {"w": 0.2, "s": 0.2})
        cls = lambda name, tw, dw, dg, pr: {"name": name, "bus_width": 12, "clearance": 0.1, "diff_pair_gap": dg, "diff_pair_via_gap": 0.25,
                                            "diff_pair_width": dw, "line_style": 0, "microvia_diameter": 0.3, "microvia_drill": 0.1,
                                            "pcb_color": "rgba(0, 0, 0, 0.000)", "priority": pr, "schematic_color": "rgba(0, 0, 0, 0.000)",
                                            "track_width": tw, "via_diameter": 0.6, "via_drill": 0.3, "wire_width": 6}
        classes = [cls("Default", 0.2, 0.2, 0.2, 2147483647), cls("50Z", w50, w50, w50, 0), cls("100Z_Diff", dp["w"], dp["w"], dp["s"], 1)]
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
            url = (STK["sources"].get(str(src.get("er", "")).split(" ")[0]) or {}).get("url")
            diel.append({"index": i, "material": d["material"], "epsilon_r": d["er"], "loss_tangent": d["tand"], "thickness": d["thickness"],
                         "source": {"kind": kind, "document": f"{stk.d['title']}: thickness {src.get('thickness')}, er {src.get('er')}, tan d {src.get('tand')}",
                                    "url": url, "note": "loss tangent assumed (not published by the manufacturer)"}})
        fields = []  # datasheet values of the populated capacitors go into PI_CAP fields (README, component list)
        cfg = {"kisi_config": 1, "board": os.path.basename(path).replace(".kisi.json", ".kicad_pcb"),
               "stackup": {"dielectrics": diel, "solder_mask": {"epsilon_r": stk.mask["er"], "source": {
                   "kind": "assumption" if "assumption" in stk.mask["src"]["er"] else "datasheet",
                   "document": "solder mask: " + stk.mask["src"]["er"], "url": STK["sources"]["jlc_impedance_guide"]["url"] if stk.d["manufacturer"] == "JLCPCB" else None}}},
               "net_classes": {
                   "50Z": {"target_ohm": 50, "diff": False, "profile": "se50", "source": {"kind": "standard", "note": "coupon design target (50 Ω VNA ports)"}},
                   "100Z_Diff": {"target_ohm": 100, "diff": True, "profile": "diff100", "source": {"kind": "standard", "note": "coupon design target (2 × 50 Ω VNA ports)"}}},
               "fields": fields,
               "notes": [f"kisi measurement coupons, generated by validation/test_board/make_coupons.py on {TODAY}", f"stackup: {stk.d['title']}",
                         "capacitors: replace the generic values by PI_CAP fields from the datasheets of the parts actually populated (README, component list)"]}
        json.dump(cfg, open(path, "w"), indent=1)


# --- KiCad text -----------------------------------------------------------------------------------
class KicadText:
    def __init__(self, board):
        self.board = board
        self.lines = []
        self.k = 0
        self.ox = self.oy = 0.0
        self.collect = True

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
        L.append('(kicad_pcb (version 20240108) (generator "kisi_make_coupons") (generator_version "8.0")')
        L.append(f"  (general (thickness {thick:.4f}) (legacy_teardrops no))")
        L.append('  (paper "A4")')
        L.append(f'  (title_block (title "{self.board}") (date "{TODAY}") (rev "{DESIGN_REV}") (company "kisi measurement coupons") '
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

    def footprint(self, name, ref, at, rot, layer, body):
        """one footprint on the board as `kisi_coupons:<name>`, and the same footprint (without
        position, nets and uuids) for the project-local library kisi_coupons.pretty: body(rot, n, u)
        returns its lines for a rotation `rot`, a net clause n(net) and a uuid clause u()"""
        L = self.lines
        L.append(f'  (footprint "{LIBNICK}:{name}" (layer "{layer}") (uuid "{self.uid()}") (at {self.X(at[0])} {self.Y(at[1])}{" " + str(rot) if rot else ""})')
        L += body(rot, lambda net: " " + self.n(net), lambda: f' (uuid "{self.uid()}")', ref, None)
        L.append("  )")
        if not self.collect:  # probe boards (trace widths) do not go into the library
            return
        lib = [f'(footprint "{name}" (version 20240108) (generator "kisi_make_coupons") (generator_version "8.0") (layer "{layer}")']
        lib += body(0, lambda net: "", lambda: "", "REF**", name)
        lib.append(")")
        text = "\n".join(lib) + "\n"
        old = LIBRARY.get(name)
        if old is not None and old != text:
            raise ValueError(f"footprint {name}: two different definitions (give them different names)")
        LIBRARY[name] = text

    def endlaunch(self, ref, p):
        """Samtec 292-J-P-EP-ST-EL-01 edge-launch footprint at the board edge, generated from
        connectors.json (footprint, launch). The geometry follows Samtec's drawing
        292-J-P-EP-ST-EL-01-X-MKT rev C, sheet 2 (recommended footprint): two plated screw holes
        Ø2.06 mm 2.79 mm from the edge, ±2.92 mm, with a round pad on every copper layer and the
        ground pads (6.48 mm deep, 10.52 mm overall, 45° chamfers, fingers towards the signal at
        the edge) on the top and bottom copper with the mask open; the signal pad's tip (tapering
        to the launch width). Copper pulled back from the routed edge by edge_pullback (JLC rule).
        One library footprint per launch geometry (name suffix _W<width>, _T<tip>_W…_L<taper>)."""
        top = p["layer"] == 0
        cu, mk, fab, crt = (("F.Cu", "F.Mask", "F.Fab", "F.CrtYd") if top else ("B.Cu", "B.Mask", "B.Fab", "B.CrtYd"))
        rot0 = 0 if p["side"] < 0 else 180
        mir = "" if top else " (justify mirror)"
        pb = FP["edge_pullback"]
        lg = p.get("geom") or {"tip_w": min(LNC["tip_w"] or p["w"], p["w"]), "taper_len": LNC["taper_len"], "pad_w": p["w"]}
        f = lambda v: f"{v:.4f}".rstrip("0").rstrip(".")
        tw, tl, pw = lg["tip_w"], lg["taper_len"], lg["pad_w"]
        name = FP["name"] + (f"_W{f(pw)}" if abs(tw - pw) < 1e-9 else f"_T{f(tw)}_W{f(pw)}_L{f(tl)}")

        def body(rot, n, u, rf, _val):
            L = []
            # KiCad footprint frame: x into the board from the edge, y down (our y up: mirrored, the footprint is symmetric)
            L.append(f'    (descr "Samtec {CONN["pn"]} 2.92 mm jack, edge launch, compression mount, to 40 GHz, PCB 1.02-2.54 mm. '
                     f'Generated by validation/test_board/make_coupons.py; geometry follows Samtec drawing 292-J-P-EP-ST-EL-01-X-MKT rev C (sheet 2). '
                     f'DRAFT: footprint to be checked; launch tip {f(tw)} to {f(pw)} mm over {f(tl)} mm, first kisi-fdtd optimisation step (board side)")')
            L.append(f'    (property "Reference" "{rf}" (at 3.0 {-round(FP["block_width"] / 2 - 0.6, 4)} {rot}) (layer "{fab}"){u()} (effects (font (size 0.8 0.8) (thickness 0.12)){mir}))')
            L.append(f'    (property "Value" "{CONN["pn"]}" (at -5.0 0 {rot}) (layer "{fab}"){u()} (effects (font (size 0.6 0.6) (thickness 0.1)){mir}))')
            L.append("    (attr smd exclude_from_pos_files exclude_from_bom)")
            cw, cd, co = FP["courtyard_half_width"], FP["courtyard_on_board"], FP["body_outside_board"] + 0.25
            L.append(f'    (fp_rect (start {-co} {-cw}) (end {cd} {cw}) (stroke (width 0.05) (type default)) (fill none) (layer "{crt}"){u()})')
            bw = FP["block_width"] / 2
            L.append(f'    (fp_rect (start {-FP["body_outside_board"]} {-bw}) (end {FP["body_on_board"]} {bw}) (stroke (width 0.1) (type default)) (fill none) (layer "{fab}"){u()})')
            L.append(f'    (fp_line (start 0 {-cw}) (end 0 {cw}) (stroke (width 0.1) (type dash)) (layer "{fab}"){u()})')
            L.append(f'    (fp_text user "DRAFT" (at 3.0 0 {rot}) (layer "{fab}"){u()} (effects (font (size 0.6 0.6) (thickness 0.1)){mir}))')
            outline = gnd_pad_outline()
            ring = FP["hole_ring"]
            for k, h in enumerate(FP["mount_holes"]):
                sg = 1 if h["y"] > 0 else -1
                hx, hy = h["x"], -h["y"]            # KiCad local y is down
                # plated hole with a round pad on every copper layer (inner layers: tied to the planes) ...
                L.append(f'    (pad "{2 + k}" thru_hole circle (at {f(hx)} {f(hy)} {rot}) (size {f(ring)} {f(ring)}) (drill {f(h["d"])}) '
                         f'(layers "*.Cu" "*.Mask") (solder_mask_margin {FP["mask_margin"]}){n("GND")}{u()})')
                # ... and the ground pad where the jaws clamp: top and bottom copper, mask open
                pts = " ".join(f"(xy {f(uu - hx)} {f(-sg * v - hy)})" for uu, v in outline)
                for lay, msk in (("F.Cu", "F.Mask"), ("B.Cu", "B.Mask")):
                    L.append(f'    (pad "{2 + k}" smd custom (at {f(hx)} {f(hy)} {rot}) (size 1 1) (layers "{lay}" "{msk}") (solder_mask_margin {FP["mask_margin"]})'
                             f'{n("GND")} (options (clearance outline) (anchor circle)) (primitives (gr_poly (pts {pts}) (width 0) (fill yes))){u()})')
            # signal pad: the tip the pin rests on, tapering to the launch width
            hw = lambda x: tw / 2 + (pw / 2 - tw / 2) * x / tl
            x0 = pb
            poly = [(x0, -hw(x0)), (tl, -pw / 2), (tl, pw / 2), (x0, hw(x0))]
            a0, xa = 0.1, (x0 + tl) / 2         # small anchor in the middle of the taper
            pts = " ".join(f"(xy {f(uu - xa)} {f(v)})" for uu, v in poly)
            L.append(f'    (pad "1" smd custom (at {f(xa)} 0 {rot}) (size {a0} {f(2 * hw(xa))}) (layers "{cu}" "{mk}") (solder_mask_margin {FP["mask_margin"]}){n(p["net"])} '
                     f'(options (clearance outline) (anchor rect)) (primitives (gr_poly (pts {pts}) (width 0) (fill yes))){u()})')
            return L
        self.footprint(name, ref, (p["x"], p["y"]), rot0, cu, body)

    def part(self, ref, p, stk):
        """SMD part (capacitor, IC pin field, probe pad): pads in board orientation (no rotation);
        library name from the KiCad name (capacitors) or the prefix, plus the rail of an IC pin field
        (its PI_* fields belong to the footprint), plus _V for a capacitor placed along y"""
        base = p.get("lib", f"x:{p['prefix']}").split(":")[-1]
        if p["prefix"] == "C" and len(p["pads"]) == 2 and abs(p["pads"][0]["x"] - p["pads"][1]["x"]) < 1e-9:
            base += "_V"
        if p.get("props"):
            base += "_" + "_".join(sorted({str(v).split("=")[0] for v in p["props"].values()}))

        def body(rot, n, u, rf, val):
            L = []
            L.append(f'    (property "Reference" "{rf}" (at 0 -1.2 0) (layer "F.Fab"){u()} (effects (font (size 0.5 0.5) (thickness 0.08))))')
            L.append(f'    (property "Value" "{val or p["value"]}" (at 0 1.2 0) (layer "F.Fab"){u()} (effects (font (size 0.5 0.5) (thickness 0.08))))')
            for k, v in p.get("props", {}).items():
                L.append(f'    (property "{k}" "{v}" (at 0 0 0) (layer "F.Fab") (hide yes){u()} (effects (font (size 0.5 0.5) (thickness 0.08))))')
            L.append("    (attr smd" + (" exclude_from_pos_files exclude_from_bom" if p.get("dnp_bom") else "") + ")")
            for pd in p["pads"]:
                lay = stk.names[pd.get("layer", 0)]
                extra = ' "F.Paste" "F.Mask"' if (lay == "F.Cu" and p["prefix"] in ("C", "R")) else (' "F.Mask"' if lay == "F.Cu" else "")
                L.append(f'    (pad "{pd["num"]}" smd {pd.get("shape", "rect")} (at {pd["x"]:.4f} {-pd["y"]:.4f}) (size {pd["sx"]:.4f} {pd["sy"]:.4f}) '
                         f'(layers "{lay}"{extra}){n(pd["net"])}{u()})')
            return L
        self.footprint(base, ref, (p["x"], p["y"]), 0, "F.Cu", body)

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
        self.lines.append(f'  (via (at {self.X(v["x"])} {self.Y(v["y"])}) (size {v["pad"]}) (drill {v["drill"]}) (layers "{stk.names[0]}" "{stk.names[-1]}") '
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
        lines = [f"{board.name}: kisi measurement coupons v{DESIGN_REV} ({TODAY})", f"Stackup: {stk.d['title']}",
                 f"Impedance control: 50 ohm single-ended (class 50Z) on L1, w = {d.get('w50', {}).get('0')} mm; 100 ohm differential (class 100Z_Diff).",
                 "Finish ENIG; green LPI mask both sides over all lines; through vias 0.3/0.6 mm, inner non-functional pads removed.",
                 f"Copper comes to {FP['edge_pullback']:g} mm from the left/right edges at the 2.92 mm edge launches (Samtec {CONN['pn']}, plated screw holes).",
                 "Connector footprint generated from Samtec's drawing (DRAFT, connectors.json); launch from a first kisi-fdtd optimisation step.",
                 "Zones are pre-filled by the generator: refill in KiCad (B) and run DRC before plotting. DRC has NOT been run."]
        for i, s in enumerate(lines):
            self.text(s, x, y - 1.6 * i, "Cmts.User", 1.0, justify="left")

    def close(self):
        self.lines.append(")")

    def text_out(self):
        return "\n".join(self.lines) + "\n"


# --- coupon builders --------------------------------------------------------------------------------
class Ctx:
    def __init__(self, stk, des, pair_pitch):
        self.stk, self.des, self.pp = stk, des, pair_pitch
        self.bot = stk.n - 1
        self.w = {int(k): v for k, v in des["w50"].items()}
        self.wlaunch, self.glaunch = des["launch"]["w"], des["launch"]["gap"]

    def wl(self, layer):
        return self.w[layer]


def edge_x(c, side):
    return side * (c.X / 2 - FP["edge_pullback"])


def launch_geometry(ctx, w=None, gap=None):
    """the launch of this stackup with the parametric values of connectors.json (launch): tip
    width and taper length, pad width / length / gap, coplanar width / gap and length, ground cut
    (width, length, layer) or None, fence. x from the board edge."""
    w = ctx.wlaunch if w is None else w
    gap = ctx.glaunch if gap is None else gap
    lc = ctx.des["launch"]
    std = w == ctx.wlaunch
    pick = lambda k, dflt: (LNC[k] if (std and LNC[k] is not None) else dflt)
    pw, pg = pick("pad_w", w), pick("pad_gap", gap)
    tl = LNC["taper_len"]
    pl = max(pick("pad_len", tl), tl)
    cut = None
    if std and LNC["gnd_cut"]["width"] is not None:
        cut = {"width": LNC["gnd_cut"]["width"], "length": LNC["gnd_cut"]["length"] or pl, "layer": ctx.stk.planes[0]}
    elif std and lc.get("cut"):  # the design's opening (thin top dielectric): along the whole launch
        cut = {"width": lc["cut"], "length": LNC["launch_length"], "layer": lc["cut_layer"]}
    return {"w": w, "gap": gap, "tip_w": min(LNC["tip_w"] or pw, pw), "taper_len": tl, "pad_w": pw, "pad_gap": pg, "pad_len": pl,
            "len": max(LNC["launch_length"], pl), "cut": cut, "fence": LNC["fence"]}


def launch(c, ctx, side, y, layer, net, name, w=None, gap=None, pour_lo=None, pour_hi=None):
    """end-launch port at the edge `side` (-1 left, +1 right), inside the Samtec ground pads
    (written with the connector footprint): the tip the pin rests on (part of the footprint's
    signal pad), the pad (pad_w, gap pad_gap) to pad_len, the grounded coplanar launch (w, gap) to
    launch_length, ground pours on both sides with rows of fence vias (kept clear of the screw
    holes) and the optional opening in the first plane under the pad (gnd_cut). Returns the x
    where the coupon's own trace starts."""
    lg = launch_geometry(ctx, w, gap)
    w, gap, pw, pg = lg["w"], lg["gap"], lg["pad_w"], lg["pad_gap"]
    c.port(side, y, layer, net, pw, pg, name, geom=lg)
    e = c.X / 2
    X = lambda u: side * (e - u)        # x of a point u mm from this edge
    x_in = X(lg["len"])
    x_tp = X(lg["taper_len"] - 0.05)    # the trace overlaps the footprint's tapered tip
    x_pad = X(lg["pad_len"])
    if lg["pad_len"] > lg["taper_len"] + 1e-9 and abs(pw - w) > 1e-9:
        c.trace(layer, [(x_tp, y), (x_pad, y)], pw, net)
        c.trace(layer, [(x_pad, y), (x_in, y)], w, net)
    else:
        c.trace(layer, [(x_tp, y), (x_in, y)], w if lg["pad_len"] <= lg["taper_len"] + 1e-9 else pw, net)
    if lg["cut"] and layer == 0:  # opening in the first plane under the pad / launch
        ct = lg["cut"]
        c.notches.append({"layer": ct["layer"], "side": side, "y0": y - ct["width"] / 2, "y1": y + ct["width"] / 2, "depth": ct["length"]})
    xa = X(ZONE_INSET)
    lo = -c.H / 2 + 0.3 if pour_lo is None else pour_lo
    hi = c.H / 2 - 0.3 if pour_hi is None else pour_hi
    fn = lg["fence"]
    holes = [(X(h["x"]), y + h["y"], h["d"]) for h in FP["mount_holes"]] if layer == 0 else []
    cols = []
    k = 0
    while fn["first_x"] + k * fn["pitch"] <= lg["len"] - 0.35:
        cols.append(X(fn["first_x"] + k * fn["pitch"]))
        k += 1
    g_pad, g_cp = pw / 2 + pg, w / 2 + gap
    g_f = max(g_pad, g_cp)  # the fence follows the wider of the two openings
    vd = LAY["gnd_via"]["drill"]
    for s, (y0, y1) in ((1, (y + g_cp, hi)), (-1, (lo, y - g_cp))):
        if y1 - y0 < 0.6:
            continue
        if abs(g_pad - g_cp) > 1e-9:
            yp0, yp1 = (y + g_pad, y1) if s > 0 else (y0, y - g_pad)
            c.pour(layer, (min(xa, x_pad), yp0, max(xa, x_pad), yp1))
            c.pour(layer, (min(x_pad, x_in), y0, max(x_pad, x_in), y1))
        else:
            c.pour(layer, (min(xa, x_in), y0, max(xa, x_in), y1))
        yf0, yf1 = (y + g_f, y1) if s > 0 else (y0, y - g_f)
        for row in range(fn["rows"]):
            yy = (yf0 + fn["offset"] + row * fn["row_pitch"]) if s > 0 else (yf1 - fn["offset"] - row * fn["row_pitch"])
            if yf0 + 0.3 <= yy <= yf1 - 0.3:
                for xx in cols:
                    if all(math.hypot(xx - hx, yy - hy) >= hd / 2 + vd / 2 + fn["hole_keep"] for hx, hy, hd in holes):
                        c.via(xx, yy, "GND", role="gnd", fence=True)
    return x_in


def straight(c, ctx, layer, y, net, x0, x1, w=None):
    c.trace(layer, [(x0, y), (x1, y)], w or ctx.wl(layer), net)


def ring_clear(c, xc, yc, n, r):
    """GND vias on a ring that keep clear of every trace of the coupon (through vias)"""
    segs = [(t["layer"], t["pts"], t["w"]) for t in c.traces]
    return [(xc + dx, yc + dy) for (dx, dy) in vc.ring(n, r) if clear_of_traces(xc + dx, yc + dy, LAY["gnd_via"]["pad"] / 2 + 0.12, segs)]


def fan(c, x_in, side, y_out, y_in, x_end, layer, net, w_out, w_in, x_min=0.3):
    """from the launch end (x_in, y_out): straight until clear of the connector, a 45° diagonal
    (steeper if there is no room) to y_in, then straight to x_end; width w_out up to the end of
    the diagonal, then w_in. Returns the x where the diagonal ends."""
    xs = side * (abs(x_in) + LAUNCH_LEN - X_CLEAR)
    xd = xs - side * abs(y_out - y_in)
    if side * (xd - x_end) < x_min:
        xd = x_end + side * x_min
    c.trace(layer, [(x_in, y_out), (xs, y_out), (xd, y_in)], w_out, net)
    if abs(xd - x_end) > 1e-9:
        c.trace(layer, [(xd, y_in), (x_end, y_in)], w_in, net)
    return xd


def four_port_launches(c, ctx, nets, sides=(-1, 1)):
    """two connectors per edge at ±pair_pitch/2 on L1; returns {(side, k): x_in}"""
    hp = ctx.pp / 2
    out = {}
    for side in sides:
        for k, y in enumerate((hp, -hp)):
            nm = str(k + 1) if side < 0 else str(k + 3)
            lo, hi = (0.0, None) if y > 0 else (None, 0.0)
            out[(side, k)] = launch(c, ctx, side, y, 0, nets[k], nm, pour_lo=lo, pour_hi=hi)
    return out


def antipads(c, ctx, x, y, r, name, skip=()):
    for L in ctx.stk.planes:
        if L not in skip:
            c.hole([L], ("circle", x, y, r), name)


def c_thru(ctx, cid, X=25.0, lines=False):
    stk = ctx.stk
    if X == 25.0:
        title, val = "2×thru on L1, 25 mm edge to edge", "fixture of every coupon on this board (P370 2×thru); multiline TRL thru"
    else:
        title, val = f"Line on L1, {X:g} mm edge to edge (ΔL = {X - 2 * FEED:g} mm against the 2×thru)", "multiline TRL line / line–line: εeff, loss per mm, Zc of the 50 Ω microstrip"
    c = Coupon(cid, "thru" if X == 25.0 else "T", title, X, ROW, val, ["thru_s4_L1"] if X == 25.0 else ["T_ts6_ms_10", "T_ts6_ms_20"],
               {"layer": stk.names[0], "w_mm": ctx.wl(0), "length_mm": X})
    net = f"SE_{cid}"
    xl = launch(c, ctx, -1, 0.0, 0, net, "1")
    xr = launch(c, ctx, 1, 0.0, 0, net, "2")
    straight(c, ctx, 0, 0.0, net, xl, xr)
    c.measure = "VNA 2-port"
    if X == 25.0:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="thru", note="10 mm of the feed line: the 3D 2×thru of this board")
    elif lines:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note="line, 10 mm (line–line pair with w20)")
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note="line, 20 mm")
    return c


def c_reflect(ctx, cid, gap=5.0):
    c = Coupon(cid, "reflect", f"TRL reflect: two open line ends at the reference planes ({FEED:g} mm from each edge), {gap:g} mm apart",
               2 * FEED + gap, ROW, "reflect standard of the on-board multiline TRL calibration (with THRU and LINE_*)", [], {"open_ends": True})
    for side, nm in ((-1, "L"), (1, "R")):
        net = f"SE_{cid}_{nm}"
        x_in = launch(c, ctx, side, 0.0, 0, net, "1" if side < 0 else "2")
        straight(c, ctx, 0, 0.0, net, x_in, side * gap / 2)
    c.measure = "VNA 2-port"
    return c


def c_vlv(ctx, cid, group, title, refs, mid, validates, Ls=LS_VIA, antipad=0.8, gnd="ring", stitch=None, params=None, lines=False):
    """via–line–via: L1 feed (12.5 mm) -> via -> line on layer `mid` (length Ls) -> via -> L1 feed.
    gnd: "ring" (4 GND vias at 1 mm around each via), "one" (one GND via `stitch` mm away on +y
    from each via) or None (the row walls only)"""
    stk = ctx.stk
    c = Coupon(cid, group, title, 2 * FEED + Ls, ROW, validates, refs,
               dict({"mid_layer": stk.names[mid], "line_mm": Ls, "drill_mm": 0.3, "antipad_mm": antipad,
                     "stub": (f"{stk.names[mid]}→{stk.names[-1]}" if mid < stk.n - 1 else "none")}, **(params or {})))
    net = f"SE_{cid}"
    xa, xb = -Ls / 2, Ls / 2
    xl = launch(c, ctx, -1, 0.0, 0, net, "1")
    xr = launch(c, ctx, 1, 0.0, 0, net, "2")
    straight(c, ctx, 0, 0.0, net, xl, xa)
    straight(c, ctx, mid, 0.0, net, xa, xb)
    straight(c, ctx, 0, 0.0, net, xb, xr)
    for x in (xa, xb):
        c.via(x, 0.0, net, pads=[0, mid])
        antipads(c, ctx, x, 0.0, antipad / 2, f"antipad {cid}")
    for x in (xa, xb):
        if gnd == "ring":
            for (gx, gy) in ring_clear(c, x, 0.0, 4, 1.0):
                c.via(gx, gy, "GND", role="gnd")
        elif gnd == "one":
            c.via(x, stitch, "GND", role="gnd")
    c.measure = "VNA 2-port"
    if lines:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note=f"line on {stk.names[mid]}, 10 mm (line–line pair with w20)")
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note=f"line on {stk.names[mid]}, 20 mm")
    else:
        c.window("wv", xa - 5.0, xb + 5.0, excite=[0], mirror=[1, 0], note="via - line - via with 5 mm L1 feeds; de-embed with THRU w10")
    return c


def c_cpwg(ctx, cid, X):
    cp = ctx.des["cpwg"]
    w, g = cp["w"], cp["gap"]
    c = Coupon(cid, "T", f"Grounded coplanar waveguide on L1, w {w:g} / gap {g:g} mm, via fence 1.0 mm, {X:g} mm edge to edge", X, ROW,
               "CPWG Zc and εeff (kisi's CPWG formula: εeff +5 % against 3D); CPWG_25 is the CPWG 2×thru", ["T_vna6_cpwg_10", "T_vna6_cpwg_20"],
               {"w_mm": w, "gap_mm": g, "length_mm": X, "fence_pitch_mm": 1.0})
    net = f"SE_{cid}"
    if ctx.des["launch"].get("cut"):
        # thin top dielectric: the CPWG is narrower than the connector pin, so the standard
        # (pin-wide) launch comes first; CPWG_50 against CPWG_25 (line–line) still cancels it
        xl = launch(c, ctx, -1, 0.0, 0, net, "1")
        xr = launch(c, ctx, 1, 0.0, 0, net, "2")
        c.trace(0, [(xl, 0.0), (xr, 0.0)], w, net)
        xa = abs(xl)
        c.params["launch"] = "pin-wide launch with the plane opening, then the CPWG"
    else:
        xt = c.X / 2 - LNC["taper_len"] + 0.05   # from the footprint's tapered tip
        c.trace(0, [(-xt, 0.0), (xt, 0.0)], w, net)
        c.port(-1, 0.0, 0, net, w, g, "1")
        c.port(1, 0.0, 0, net, w, g, "2")
        xa = c.X / 2 - ZONE_INSET
    yh = min(abs(h["y"]) - h["d"] / 2 for h in FP["mount_holes"]) - 0.3   # clear of the connector's holes
    xh = c.X / 2 - X_CLEAR + 0.2
    for s in (1, -1):
        y0, y1 = w / 2 + g, c.H / 2 - 0.3
        c.pour(0, (-xa, s * y0, xa, s * yh))
        c.pour(0, (-xh, s * yh, xh, s * y1))
        n = int((2 * xa - 1.6) / 1.0)
        for k in range(n + 1):
            x = -xa + 0.8 + k * 1.0
            if x <= xa - 0.8 + 1e-9:
                c.via(x, s * (y0 + FENCE_OFF), "GND", role="gnd")
    c.measure = "VNA 2-port"
    if X == 25.0:
        c.window("w10", -5.0, 5.0, excite=[0], mirror=[1, 0], group="T", note="CPWG, 10 mm (line–line pair with CPWG_50 w20)")
    else:
        c.window("w20", -10.0, 10.0, excite=[0], mirror=[1, 0], group="T", note="CPWG, 20 mm")
    return c


def c_pair_thru(ctx, cid):
    pv = ctx.des["pair_via"]["0"]
    c = Coupon(cid, "thru", "pair 2×thru on L1 at the 1.0 mm via pitch (100 Ω differential), 30 mm edge to edge", 2 * FEED4, ROW4,
               "fixture of the pair coupons on this board", [], {"w_pair_mm": pv["w"], "pitch_mm": 1.0})
    P, N = f"DP_{cid}_P", f"DP_{cid}_N"
    hp = ctx.pp / 2
    xin = four_port_launches(c, ctx, (P, N))
    for k, (net, yo, yi) in enumerate(((P, hp, 0.5), (N, -hp, -0.5))):
        for side in (-1, 1):
            fan(c, xin[(side, k)], side, yo, yi, 0.0, 0, net, ctx.wl(0), pv["w"], x_min=3.3)
    c.measure = "VNA 4-port"
    c.window("w6", -3.0, 3.0, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="thru",
             note="pair at the 1.0 mm via pitch, 3 mm on each side; ports 1 P near, 2 N near, 3 P far, 4 N far")
    return c


def c_pair_vlv(ctx, cid, title, refs, mid, oval, ngnd, validates, Ls=LS_VIA):
    """pair via–line–via: L1 pair -> pair vias -> pair on `mid` (Ls) -> pair vias -> L1 pair"""
    stk = ctx.stk
    pv = ctx.des["pair_via"]
    c = Coupon(cid, "E", title, 2 * FEED4 + Ls, ROW4, validates, refs,
               {"mid_layer": stk.names[mid], "line_mm": Ls, "pitch_mm": 1.0, "antipad": "oval" if oval else "round", "gnd_vias": ngnd,
                "w_pair_mm": pv["0"]["w"], "stub": (f"{stk.names[mid]}→{stk.names[-1]}" if mid < stk.n - 1 else "none")})
    P, N = f"DP_{cid}_P", f"DP_{cid}_N"
    hp = ctx.pp / 2
    vx = Ls / 2
    xin = four_port_launches(c, ctx, (P, N))
    for k, (net, yo, yi) in enumerate(((P, hp, 0.5), (N, -hp, -0.5))):
        for side in (-1, 1):
            fan(c, xin[(side, k)], side, yo, yi, side * vx, 0, net, ctx.wl(0), pv["0"]["w"], x_min=3.3)
        c.trace(mid, [(-vx, yi), (vx, yi)], pv[str(mid)]["w"], net)
    for x in (-vx, vx):
        for net, y in ((P, 0.5), (N, -0.5)):
            c.via(x, y, net, pads=[0, mid])
        for L in stk.planes:
            if oval:
                c.hole([L], ("slot", x, 0.5, x, -0.5, 0.4), f"antipad {cid}")
            else:
                for y in (0.5, -0.5):
                    c.hole([L], ("circle", x, y, 0.4), f"antipad {cid}")
        if ngnd == 2:
            for y in (1.3, -1.3):
                c.via(x, y, "GND", role="gnd")
    c.measure = "VNA 4-port"
    c.window("wv", -vx - 3.0, vx + 3.0, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2],
             note="pair via - line - via with 3 mm L1 feeds; ports 1 P near, 2 N near, 3 P far, 4 N far; de-embed with PAIR w6")
    return c


def c_coupled(ctx, cid, kind, title, refs, validates, X=50.0):
    """XT: 50 Ω aggressor (A) and victim (V) with edge gap = w; DIFF: tightly coupled 100 Ω pair,
    reached by 50 Ω lines down to a 1.2 mm pitch and a short taper of the spacing"""
    c = Coupon(cid, "T", title, X, ROW4, validates, refs)
    hp = ctx.pp / 2
    if kind == "xt":
        w = ctx.wl(0)
        nets, yi, wi = ("SE_XT_A", "SE_XT_V"), w, w
        c.params.update(w_mm=w, gap_mm=w)
    else:
        p = ctx.des["diff"]["0"]
        nets, yi, wi = (f"DP_{cid}_P", f"DP_{cid}_N"), (p["w"] + p["s"]) / 2, p["w"]
        c.params.update(w_mm=p["w"], s_mm=p["s"])
    xin = four_port_launches(c, ctx, nets)
    xds = []
    for k, (net, sg) in enumerate(((nets[0], 1), (nets[1], -1))):
        for side in (-1, 1):
            if kind == "xt":
                xds.append(fan(c, xin[(side, k)], side, sg * hp, sg * yi, 0.0, 0, net, ctx.wl(0), wi))
            else:
                xm = fan(c, xin[(side, k)], side, sg * hp, sg * 0.6, side * 12.0, 0, net, ctx.wl(0), wi)
                xq = xm - side * (0.3 + 0.6 - yi)
                c.traces.pop()  # the straight part after the diagonal becomes the spacing taper
                c.trace(0, [(xm, sg * 0.6), (xm - side * 0.3, sg * 0.6), (xq, sg * yi), (0.0, sg * yi)], wi, net)
                xds.append(xq)
    half = min(abs(x) for x in xds)
    c.params["coupled_mm"] = round(2 * half, 2)
    c.title += f", coupled {2 * half:.1f} mm"
    c.measure = "VNA 4-port; scope TDR/TDT"
    h = round(half - 0.3, 2)
    c.window("wc", -h, h, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="T",
             note=f"the straight coupled section ({2 * h:g} mm); ports 1 A/P near, 2 V/N near, 3 A/P far, 4 V/N far")
    return c


def c_coupled_inner(ctx, cid, kind, title, refs, validates, X=50.0):
    """stripline pair on the first inner signal layer (diff), or broadside on the two inner signal
    layers (bs); fed from L1 through vias at both ends"""
    stk = ctx.stk
    inner = [L for L in stk.signal if 0 < L < stk.n - 1]
    c = Coupon(cid, "T", title, X, ROW4, validates, refs)
    hp = ctx.pp / 2
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
    xin = four_port_launches(c, ctx, nets)
    for k, (net, sg) in enumerate(((nets[0], 1), (nets[1], -1))):
        for side in (-1, 1):
            fan(c, xin[(side, k)], side, sg * hp, yv[k], side * xv, 0, net, ctx.wl(0), ctx.wl(0))
            xj = side * (xv - 0.9)
            c.trace(layers[k], [(side * xv, yv[k]), (xj, yv[k]), (side * (xv - 0.9 - abs(yv[k] - yc[k])), yc[k]), (0.0, yc[k])], wi[k], net)
            c.via(side * xv, yv[k], net, pads=[0, layers[k]])
            antipads(c, ctx, side * xv, yv[k], 0.4, f"antipad {cid}")
    for side in (-1, 1):
        for (gx, gy) in ring_clear(c, side * xv, 0.0, 8, 1.6):
            c.via(gx, gy, "GND", role="gnd")
    half = xv - 0.9 - max(abs(yv[k] - yc[k]) for k in (0, 1))
    c.params["coupled_mm"] = round(2 * half, 2)
    c.title += f", coupled {2 * half:.1f} mm"
    c.measure = "VNA 4-port; scope TDR/TDT"
    h = round(half - 0.3, 2)
    if kind == "diff":
        c.window("wc", -h, h, half_y=4.0, excite=[0, 2], mirror=[1, 0, 3, 2], group="T", note=f"the straight coupled stripline section ({2 * h:g} mm)")
    else:
        c.window("wc", -h, h, half_y=4.0, excite=[0, 1], mirror=[2, 3, 0, 1], group="T",
                 note=f"the straight broadside section ({2 * h:g} mm); ports 1/3 upper layer (A), 2/4 lower layer (V)")
    return c


def c_cavity(ctx, cid, a, b, X, H, p1, p2):
    """G: PWR island a × b on the PDN layer under the first GND plane; two ports through vias from L1
    microstrips: p1 = (x, y) from the left edge, p2 from the right edge"""
    stk = ctx.stk
    pw = stk.pwr
    gref = max(p for p in stk.planes if p < pw)
    d = stk.z[gref] - stk.z[pw]
    c = Coupon(cid, "G", f"Plane cavity {a:g} × {b:g} mm on {stk.names[pw]} under {stk.names[gref]} (d = {d:.3f} mm), ports at a corner and the centre",
               X, H, "cavity resonances (kisi `power --pdn`, cavity model ±0.8 % against 3D) and the εr of the dielectric between the planes",
               ["G_pp100", "G_pp200"], {"size_mm": f"{a:g} × {b:g}", "d_mm": round(d, 4)})
    rail = f"CAV_{cid}"
    holes = []
    xl = launch(c, ctx, -1, p1[1], 0, rail, "1")
    xr = launch(c, ctx, 1, p2[1], 0, rail, "2")
    straight(c, ctx, 0, p1[1], rail, xl, p1[0])
    straight(c, ctx, 0, p2[1], rail, p2[0], xr)
    for (x, y) in (p1, p2):
        c.via(x, y, rail, pads=[0, pw])
        antipads(c, ctx, x, y, 0.4, f"antipad {cid}", skip=(pw,))
    c.islands.append({"layer": pw, "rect": (-a / 2, -b / 2, a / 2, b / 2), "net": rail, "clearance": 0.5, "holes": holes})
    er = stk.diel[gref]["er"] if pw - gref == 1 else sum(stk.diel[i]["thickness"] for i in range(gref, pw)) / sum(stk.diel[i]["thickness"] / stk.diel[i]["er"] for i in range(gref, pw))
    modes = sorted((vc.C0 / (2 * math.sqrt(er)) * math.hypot(m / a, n / b) * 1e3, m, n) for m in range(5) for n in range(4) if (m, n) != (0, 0))
    c.params.update(modes_GHz=[round(f / 1e9, 3) for f, m, n in modes if f < 8e9][:8], modes_mn=[f"{m}{n}" for f, m, n in modes if f < 8e9][:8],
                    modes_note="magnetic-wall estimate with the nominal er; fringing lowers them")
    c.measure = "VNA 2-port (S21 peaks; Z21 by shunt-thru)"
    xw = min(max(a / 2, abs(p1[0]), abs(p2[0])) + 5.0, X / 2 - LAUNCH_LEN - 0.2)  # ports on the feed lines
    c.window("wg", -xw, xw, half_y=H / 2 + 0.6, dfine=0.1, fmax=8e9, note="the whole island with the L1 feeds up to the window",
             extra={"nf": 600, "f_loss": 2.5e9, "max_steps": 400000, "meta_modes": [round(f, -6) for f, m, n in modes]})
    return c


def cap_part(c, pkg, x, y, axis, nets, value, role, populate):
    """capacitor land pattern (KiCad standard geometry) at (x, y); axis 'x' or 'y'; nets = (pad 1, pad 2)"""
    p = PKG[pkg]
    along, across = p["pad"]
    pads = []
    for k, sgn in ((0, -1), (1, 1)):
        if axis == "x":
            pads.append({"num": str(k + 1), "x": sgn * p["pitch"] / 2, "y": 0.0, "sx": along, "sy": across, "net": nets[k]})
        else:
            pads.append({"num": str(k + 1), "x": 0.0, "y": -sgn * p["pitch"] / 2, "sx": across, "sy": along, "net": nets[k]})
    c.parts.append({"prefix": "C", "lib": p["kicad"], "x": x, "y": y, "value": value, "pads": pads, "role": role, "package": pkg, "populate": populate})
    for n_ in nets:
        c.net(n_)


def c_capz(ctx, cid, pkg):
    """capacitor shunt-thru: a 50 Ω L1 line with a capacitor footprint from the line to GND at x = 0"""
    p = PKG[pkg]
    c = Coupon(cid, "CAP", f"Shunt {pkg} capacitor on the 50 Ω line (2-port shunt-thru), 25 mm", 2 * FEED, ROW,
               f"impedance of a mounted {pkg} capacitor: C, ESR, ESL and mounting inductance (data/components.json esl_nh by package, ESR model, mount_nh)",
               [], {"package": pkg, "pitch_mm": p["pitch"], "kicad_footprint": p["kicad"]})
    net = f"SE_{cid}"
    xl = launch(c, ctx, -1, 0.0, 0, net, "1")
    xr = launch(c, ctx, 1, 0.0, 0, net, "2")
    straight(c, ctx, 0, 0.0, net, xl, xr)
    # pad 1 on the line, pad 2 below it to GND with two vias
    cap_part(c, pkg, 0.0, -p["pitch"] / 2, "y", (net, "GND"), "100nF 16V X7R", "shunt", "empty / 0 Ω / 100 nF X7R / others (component list)")
    yv = -p["pitch"] - p["pad"][0] / 2 - 0.25
    for xv in (-0.35, 0.35):
        c.via(xv, yv, "GND", role="gnd")
        c.trace(0, [(0.0, -p["pitch"]), (xv, yv)], 0.3, "GND")  # pad to via (KiCad and kisi connectivity)
    c.measure = "VNA 2-port shunt-thru: Z = 25·S21/(1 − S21)"
    c.window("w10", -5.0, 5.0, excite=[0, 1], dfine=0.035, fmax=20e9, note="the capacitor with 5 mm of line on both sides; modelled as populated with 0 Ω (5 mΩ)",
             extra={"capz": True})
    return c


def c_hpdn(ctx, cid, d, side, X, island_len, pkg="0402"):
    """PDN loop: two shunt-thru ports at the `side` edge (connectors at ±pair_pitch/2), traces to an IC
    pin field (4 × 4, 0.8 mm, checkerboard PWR/GND; PWR pins tied by a copper patch on L1), one
    capacitor footprint whose GND pad is d mm from the pin field's edge; PWR island on the PDN layer"""
    stk = ctx.stk
    pw = stk.pwr
    p = PKG[pkg]
    rail = f"VDD_{cid}"
    c = Coupon(cid, "H", f"PDN loop: IC pin field to a {pkg} capacitor {d:g} mm away, PWR island on {stk.names[pw]}, shunt-thru ports at the {'left' if side < 0 else 'right'} edge",
               X, ROW4, "capacitor loop inductance and Z(f) at the IC (kisi `power --pdn`: pdn_ic, cavity_modal loop inductance; package ESL and mount_nh of data/components.json)",
               [f"H_s8_cap_{int(d)}"] if (d in (2, 5, 10, 20) and pkg == "0402") else [], {"distance_mm": d, "package": pkg, "pins": "4 × 4, 0.8 mm", "pwr_layer": stk.names[pw]})
    hp = ctx.pp / 2
    # pin-field centre: each feed (launch, along x, then along y to the patch edge at ±1.6 mm) is
    # FEED long, as one half of the board's 2×thru, so the THRU de-embeds it
    ye = 1.65                                            # feed end, 0.05 mm over the patch edge
    xe = FEED - (hp - ye)
    assert xe >= X_CLEAR + 0.5, "H coupon: the feed corner is too close to the connector"
    xf = side * (X / 2 - xe)
    for k, y in enumerate((hp, -hp)):
        lo, hi = (0.0, None) if y > 0 else (None, 0.0)
        x_in = launch(c, ctx, side, y, 0, rail, str(k + 1), pour_lo=lo, pour_hi=hi)
        c.trace(0, [(x_in, y), (xf, y), (xf, math.copysign(ye, y))], ctx.wl(0), rail)
    pins = [(xf - 1.2 + 0.8 * i, -1.2 + 0.8 * j, (i + j) % 2 == 0) for i in range(4) for j in range(4)]
    island_holes = []
    pads = []
    for k, (x, y, is_pwr) in enumerate(pins):
        net = rail if is_pwr else "GND"
        pads.append({"num": f"{'ABCD'[k // 4]}{k % 4 + 1}", "x": x - xf, "y": y, "sx": 0.6, "sy": 0.6, "shape": "circle", "net": net})
        if is_pwr:
            c.via(x, y, rail, pads=[0, pw])
            antipads(c, ctx, x, y, 0.35, f"antipad {cid}", skip=(pw,))
        else:
            c.via(x, y, "GND", pads=None, role="gnd")
            island_holes.append(("circle", x, y, 0.35))
    c.parts.append({"prefix": "U", "x": xf, "y": 0.0, "value": "pin field (no part)", "pads": pads, "role": "ic", "dnp_bom": True,
                    "props": {"PI_LOAD": f"{rail}=0.5A", "PI_TARGET": f"{rail}=10m fmax=1G"}})
    c.zones.append({"layer": 0, "rect": (xf - 1.6, -1.6, xf + 1.6, 1.6), "net": rail, "holes": [("circle", x, y, 0.4) for (x, y, q) in pins if not q]})
    # capacitor: pads along x, GND pad nearer the pin field, a via just outside each pad
    s = -side
    xg = xf + s * (1.6 + d)
    xc = xg + s * p["pitch"] / 2
    nets = ("GND", rail) if s > 0 else (rail, "GND")
    cap_part(c, pkg, xc, 0.0, "x", nets, "100nF 16V X7R", "decap", "empty / 0 Ω / 100 nF X7R / others (component list)")
    off = p["pad"][0] / 2 + 0.25
    c.via(xg - s * off, 0.0, "GND", role="gnd")
    island_holes.append(("circle", xg - s * off, 0.0, 0.35))
    xpv = xg + s * (p["pitch"] + off)
    c.via(xpv, 0.0, rail, pads=[0, pw])
    c.trace(0, [(xg, 0.0), (xg - s * off, 0.0)], 0.3, "GND")  # pads to their vias (KiCad and kisi connectivity)
    c.trace(0, [(xg + s * p["pitch"], 0.0), (xpv, 0.0)], 0.3, rail)
    antipads(c, ctx, xpv, 0.0, 0.35, f"antipad {cid}", skip=(pw,))
    # PWR island (the same length for every coupon of a board) with clearance around the GND vias
    xi0, xi1 = xf - s * 1.7, xf + s * island_len
    assert max(h["x"] + h["d"] / 2 for h in FP["mount_holes"]) + 0.3 <= xe - 1.7, "H coupon: PWR island too close to the mounting holes"
    c.islands.append({"layer": pw, "rect": (min(xi0, xi1), -(ROW4 / 2 - 1.2), max(xi0, xi1), ROW4 / 2 - 1.2), "net": rail, "clearance": 0.5, "holes": island_holes})
    c.params.update(island_mm=[round(abs(xi1 - xi0), 2), round(ROW4 - 2.4, 2)], gnd_pad_centre_from_field_edge_mm=d)
    c.measure = "VNA 2-port shunt-thru: Z = 25·S21/(1 − S21), 100 kHz – 1 GHz; unpopulated, 0 Ω, capacitor"
    w0, w1 = sorted((side * (X / 2 - X_CLEAR), xf + s * (island_len + 1.0)))
    c.window("wh", w0, w1, half_y=ROW4 / 2 + 0.6, excite=[0, 1], dfine=0.07, fmax=2e9, group="H",
             note="ports, pin field, capacitor (modelled as populated with 0 Ω: 5 mΩ) and the PWR island",
             extra={"nf": 200, "f_loss": 1e9, "end_db": -60, "max_steps": 300000, "hpdn": True})
    return c


# --- boards -------------------------------------------------------------------------------------
def board_specs(ctx):
    """rows per board part; every row list is sorted by length later"""
    stk = ctx.stk
    sn = stk.names
    bot = ctx.bot
    inner = [L for L in stk.signal if 0 < L < stk.n - 1]
    ap = lambda d: int(round(d * 1000))
    T = lambda: [c_thru(ctx, "THRU")]
    if stk.n == 4:
        a_ = lambda d: c_vlv(ctx, f"A{ap(d):04d}", "A", f"via–line–via L1→L4→L1 through the L2/L3 plane pair, drill 0.3, antipad Ø{d:.1f}, 4 GND vias at 1 mm",
                             [f"A_s4_d300_ap{ap(d)}"] if d <= 1.0 else [], bot, "via ladder / coax section: antipad dependence of Z and delay (A group)", antipad=d)
        d_ = lambda s: c_vlv(ctx, f"D_S{str(s).replace('.', 'p')}", "D", f"L1 (ref L2) → L4 (ref L3) → L1, one GND via {s:g} mm from each via, antipad Ø0.8",
                             [f"D_s8_stitch_{str(s).replace('.', 'p')}"] if s in (1.0, 2.0) else (["D_s8_stitch_0p5"] if s < 1.0 else []), bot,
                             "return path of a reference change L2 → L3 (kisi: GND vias within 3 mm enter the ladder; up to 5 mm an inductance)",
                             Ls=LS_RET, gnd="one", stitch=s, params={"stitch_mm": s})
        parts = {
            "a": [T(), [c_thru(ctx, "LINE_27", 27.0)], [c_reflect(ctx, "REFLECT")], [c_thru(ctx, "LINE_30", 30.0)], [c_thru(ctx, "LINE_50", 50.0, lines=True)],
                  [c_thru(ctx, "LINE_100", 100.0)], [c_cpwg(ctx, "CPWG_25", 25.0)], [c_cpwg(ctx, "CPWG_50", 50.0)]],
            "b": [T(), [a_(0.8)], [a_(1.0)], [a_(1.2)], [d_(1.0)], [d_(2.0)], [d_(3.5)],
                  [c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L4 (ref L3) → L1, no return via (the row walls at ±6 mm only)", ["D_s8_stitch_none"], bot,
                         "return path without a nearby GND via: the plane-pair cavity between the row walls", Ls=LS_RET, gnd=None)]],
            "c": [[c_pair_thru(ctx, "PAIR")],
                  [c_pair_vlv(ctx, "E_R_G2", "pair via–line–via L1→L4→L1, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, False, 2,
                              "differential via ladder: Sdd21, Zdiff, Scd21")],
                  [c_pair_vlv(ctx, "E_O_G2", "pair via–line–via L1→L4→L1, pitch 1.0, one oval antipad, 2 GND vias", ["E_s8_L1L8_p1000_oval_g2"], bot, True, 2,
                              "oval (non-round) antipad: kisi's equivalent radius")],
                  [c_pair_vlv(ctx, "E_R_G0", "pair via–line–via L1→L4→L1, pitch 1.0, round antipads, no GND vias", ["E_s8_L1L8_p1000_round_g0"], bot, False, 0,
                              "pair via without return vias")]],
            "d": [T(), [c_capz(ctx, "CAPZ_0402", "0402")], [c_capz(ctx, "CAPZ_0603", "0603")], [c_capz(ctx, "CAPZ_0805", "0805")],
                  [c_coupled(ctx, "XT_MS", "xt", "Microstrip aggressor and victim on L1, edge gap = w", ["T_ts6_xt_ms_s10"], "NEXT / FEXT (kisi's NEXT is 70–80 % high against 3D)")],
                  [c_coupled(ctx, "DIFF_MS", "diff", "Edge-coupled 100 Ω microstrip pair on L1", ["T_ts6_diff_ms"], "Zdiff (TDR plateau), odd-mode εeff, Sdd21 per mm")]],
        }
        if stk.key == "jlc4_7628":  # first order (maintainer, 2026-10-31): two 4-layer boards
            dnone = lambda: c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L4 (ref L3) → L1, no return via (the row walls at ±6 mm only)", ["D_s8_stitch_none"], bot,
                                  "return path without a nearby GND via: the plane-pair cavity between the row walls", Ls=LS_RET, gnd=None)
            # f1 (100 × 96): the lines, the coupling coupon and the cavity (the 100 mm row sets the width anyway);
            # f2 (35 × 156): the vias. Edge launches take 12 mm of edge per connector and side, so 16 positions per
            # 96 mm board: the 36 positions of the first order cannot share one board within 100 × 100 mm.
            parts["f1"] = [T(), [c_cpwg(ctx, "CPWG_25", 25.0)], [c_cpwg(ctx, "CPWG_50", 50.0)],
                           [c_coupled(ctx, "XT_MS", "xt", "Microstrip aggressor and victim on L1, edge gap = w", ["T_ts6_xt_ms_s10"], "NEXT / FEXT (kisi's NEXT is 70–80 % high against 3D)")],
                           [c_cavity(ctx, "G_CAV", 50.0, 30.0, 100.0, 3 * ROW, (-22.0, -12.0), (0.0, 0.0))]]
            parts["f2"] = [T(), [a_(0.8)], [a_(1.0)], [a_(1.2)], [d_(D_CLOSE)], [d_(2.0)], [dnone()], [c_pair_thru(ctx, "PAIR")],
                           [c_pair_vlv(ctx, "E_R_G2", "pair via–line–via L1→L4→L1, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, False, 2,
                                       "differential via ladder: Sdd21, Zdiff, Scd21")],
                           [c_pair_vlv(ctx, "E_R_G0", "pair via–line–via L1→L4→L1, pitch 1.0, round antipads, no GND vias", ["E_s8_L1L8_p1000_round_g0"], bot, False, 0,
                                       "pair via without return vias")]]
        if stk.pwr is not None and stk.key == "jlc4_7628":
            parts["p"] = [T(), [c_cavity(ctx, "G_CAV", 50.0, 30.0, 100.0, 3 * ROW, (-22.0, -12.0), (0.0, 0.0))],
                          [c_hpdn(ctx, "H_D2", 2.0, -1, 50.0, 26.0), c_hpdn(ctx, "H_D5", 5.0, 1, 50.0, 26.0)],
                          [c_hpdn(ctx, "H_D10", 10.0, -1, 50.0, 26.0), c_hpdn(ctx, "H_D20", 20.0, 1, 50.0, 26.0)]]
        if stk.key.startswith("aisler"):
            g = c_cavity(ctx, "G_SMALL", 25.0, 18.0, 35.0, ROW4, (-10.0, -6.0), (0.0, 0.0))
            parts = {"a": [T(), [c_thru(ctx, "LINE_27", 27.0)], [c_thru(ctx, "LINE_30", 30.0)], [a_(0.8)], [a_(1.0)], [a_(1.2)], [d_(1.0)], [d_(2.0)],
                           [c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L4 (ref L3) → L1, no return via (row walls only)", ["D_s8_stitch_none"], bot,
                                  "return path without a nearby GND via", Ls=LS_RET, gnd=None)],
                           [c_thru(ctx, "LINE_35", 35.0)], [g]]}
        return parts
    if stk.n == 6:
        d6 = lambda st: c_vlv(ctx, f"D_S{str(st).replace('.', 'p')}", "D", f"L1 (ref L2) → L6 (ref L5) → L1, one GND via {st:g} mm from each via",
                              [f"D_s8_stitch_{str(st).replace('.', 'p')}"] if st in (1.0, 2.0) else (["D_s8_stitch_0p5"] if st < 1.0 else []), bot,
                              "return path L2 → L5", Ls=LS_RET, gnd="one", stitch=st, params={"stitch_mm": st})
        first = {}
        if stk.key == "jlc6_7628":  # first order (maintainer, 2026-10-31): one 6-layer board
            first = {"f1": [T(),  # one board with everything (50 × 132 mm, above the 50 × 50 mm bracket: maintainer, 2026-10-31)
                            [c_vlv(ctx, "SL_L3", "T", f"via–line–via with 25 mm stripline on {sn[inner[0]]}", ["T_ts6_sl_10", "T_ts6_sl_20"], inner[0],
                                   "stripline γ (line–line with VLV_L3: the vias cancel), Zc from the TDR plateau", Ls=25.0, lines=True)],
                            [c_vlv(ctx, "VLV_L3", "B", f"via–line–via L1→{sn[inner[0]]}→L1 (through vias, stubs to L6), 5 mm stripline", ["B_ts6_L1L3_stub", "B_s8_L1L3_stub_d300"],
                                   inner[0], "inner-layer exit with an open stub (junction model); stub notch inside the 40 GHz band (kisi: 32 GHz)")],
                            [c_vlv(ctx, "B_L1L6", "B", "via–line–via L1→L6→L1 (through vias, no stub), antipad Ø0.8 on L2/L5, 4 GND vias at 1 mm", ["B_ts6_L1L6", "B_s8_L1L8_d300"], bot,
                                   "through via without stub (via ladder)")],
                            [c_pair_thru(ctx, "PAIR")],
                            [c_pair_vlv(ctx, "E_R_G2", "pair via–line–via L1→L6→L1, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, False, 2,
                                        "differential via ladder through two plane layers")],
                            [d6(D_CLOSE)], [d6(2.0)],
                            [c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L6 (ref L5) → L1, no return via (row walls only)", ["D_s8_stitch_none"], bot,
                                   "return path without a nearby GND via", Ls=LS_RET, gnd=None)]]}
        return dict(first, **{
            "a": [T(), [c_vlv(ctx, "B_L1L6", "B", "via–line–via L1→L6→L1 (through vias, no stub), antipad Ø0.8 on L2/L5, 4 GND vias at 1 mm", ["B_ts6_L1L6", "B_s8_L1L8_d300"], bot,
                                "through via without stub (via ladder)")],
                  [c_vlv(ctx, "D_S1", "D", "L1 (ref L2) → L6 (ref L5) → L1, one GND via 1 mm from each via", ["D_s8_stitch_1p0"], bot, "return path L2 → L5",
                         Ls=LS_RET, gnd="one", stitch=1.0, params={"stitch_mm": 1.0})],
                  [c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L6 (ref L5) → L1, no return via (row walls only)", ["D_s8_stitch_none"], bot,
                         "return path without a nearby GND via", Ls=LS_RET, gnd=None)]],
            "b": [T(), [c_vlv(ctx, "VLV_L3", "B", f"via–line–via L1→{sn[inner[0]]}→L1 (through vias, stubs to L6), 5 mm stripline", ["B_ts6_L1L3_stub", "B_s8_L1L3_stub_d300"],
                                inner[0], "inner-layer exit with an open stub (junction model); stub resonance inside the 40 GHz band (kisi: 32 GHz)")],
                  [c_vlv(ctx, "SL_L3", "T", f"via–line–via with 25 mm stripline on {sn[inner[0]]}", ["T_ts6_sl_10", "T_ts6_sl_20"], inner[0],
                         "stripline γ (line–line with VLV_L3: the vias cancel), Zc from the TDR plateau; asymmetric stripline (formula +11 % on the ThunderScope stack)",
                         Ls=25.0, lines=True)],
                  [c_thru(ctx, "LINE_50", 50.0, lines=True)]],
            "c": [[c_pair_thru(ctx, "PAIR")],
                  [c_pair_vlv(ctx, "E_R_G2", "pair via–line–via L1→L6→L1, pitch 1.0, round antipads Ø0.8, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, False, 2,
                              "differential via ladder through two plane layers")]],
            "d": [[c_coupled_inner(ctx, "BS_L3L4", "bs", f"Broadside: {sn[inner[0]]} over {sn[inner[1]]}, offset 0", ["T_ts6_broadside_o0"],
                                   "broadside NEXT / FEXT (kisi lists it but does not compute it yet)")],
                  [c_coupled_inner(ctx, "DIFF_SL", "diff", f"Edge-coupled 100 Ω stripline pair on {sn[inner[0]]}", ["T_ts6_diff_sl"],
                                   "stripline Zdiff plateau (open point in TESTS.md)")]],
        })
    # 8 layers
    return {
        "a": [T(), [c_vlv(ctx, "VLV_L3", "B", f"via–line–via L1→{sn[inner[0]]}→L1 (through vias, 1.4 mm stubs to L8), 5 mm stripline",
                          ["B_s8_L1L3_stub_d300", "X_s8x2_L1L3_bd_d200"], inner[0], "long open stub: notch near 20 GHz, inside the band of every VNA (B group)")],
              [c_vlv(ctx, "B_L1L8", "B", "via–line–via L1→L8→L1 (through vias, no stub), 4 GND vias at 1 mm", ["B_s8_L1L8_d300"], bot, "through via without stub")],
              [c_vlv(ctx, "SL_L3", "T", f"via–line–via with 25 mm stripline on {sn[inner[0]]}", ["T_ts6_sl_10", "T_ts6_sl_20"], inner[0],
                     "stripline γ (line–line with VLV_L3), Zc from the TDR plateau", Ls=25.0, lines=True)]],
        "b": [T(), [c_vlv(ctx, "VLV_L6", "B", f"via–line–via L1→{sn[inner[1]]}→L1 (through vias, short stubs to L8), 5 mm stripline", [], inner[1],
                          "short stub: compare with VLV_L3 (long stub) and B_L1L8 (none)")],
              [c_vlv(ctx, "D_S1", "D", "L1 (ref L2) → L8 (ref L7) → L1, one GND via 1 mm from each via", ["D_s8_stitch_1p0"], bot, "return path L2 → L7 across four planes",
                     Ls=LS_RET, gnd="one", stitch=1.0, params={"stitch_mm": 1.0})],
              [c_vlv(ctx, "D_NONE", "D", "L1 (ref L2) → L8 (ref L7) → L1, no return via (row walls only)", ["D_s8_stitch_none"], bot,
                     "return path without a nearby GND via", Ls=LS_RET, gnd=None)]],
        "c": [[c_pair_thru(ctx, "PAIR")],
              [c_pair_vlv(ctx, "E_L1L8", "pair via–line–via L1→L8→L1, pitch 1.0, round antipads, 2 GND vias", ["E_s8_L1L8_p1000_round_g2"], bot, False, 2,
                          "pair through four planes")]],
        "d": [[c_pair_thru(ctx, "PAIR")],
              [c_pair_vlv(ctx, "E_L1L3", f"pair via–line–via L1→{sn[inner[0]]}→L1 (stubs to L8), pitch 1.0, round antipads, 2 GND vias",
                          ["E_s8_L1L3_p1000_round_g2"], inner[0], False, 2, "pair inner-layer exit with stubs")]],
        "p": [[c_hpdn(ctx, "H_D2", 2.0, -1, 25.0, 16.0), c_hpdn(ctx, "H_D5", 5.0, 1, 25.0, 16.0)],
              [c_hpdn(ctx, "H_D10", 10.0, -1, 25.0, 16.0), c_hpdn(ctx, "H_D2_0603", 2.0, 1, 25.0, 16.0, pkg="0603")]],
    }


STACKS_GEN = ["jlc4_7628", "jlc6_7628", "jlc8_7628", "aisler4_assumed", "jlc4_3313", "jlc6_3313"]


FIRST_ORDER = {"jlc4_7628": ("f1", "f2"), "jlc6_7628": ("f1",)}
# first-order boards above the price bracket, accepted by the maintainer (2026-10-31): their own limits (mm)
OVERSIZE_OK = {"jlc4_7628_f2": (50.0, 160.0), "jlc6_7628_f1": (50.0, 140.0)}


def all_boards():
    """board key -> (stackup, part); parts follow board_specs; the first order first"""
    parts = {4: "abcd", 6: "abcd", 8: "abcdp"}
    out = {f"{sk}_{p}": (sk, p) for sk, ps in FIRST_ORDER.items() for p in ps}
    for sk in STACKS_GEN:
        n = len(STK["stackups"][sk]["copper"])
        ps = "a" if sk.startswith("aisler") else parts[n] + ("p" if sk == "jlc4_7628" else "")
        for p in ps:
            out[f"{sk}_{p}"] = (sk, p)
    return out


BOARDS = all_boards()
PART_TITLE = {"a": "lines / vias", "b": "vias / return path", "c": "pairs", "d": "coupling / capacitors", "p": "PDN",
              "f1": "first order 1", "f2": "first order 2"}


# --- kisi-fdtd cases ------------------------------------------------------------------------------
def in_rect(x, y, r, m=0.0):
    return r[0] - m <= x <= r[2] + m and r[1] - m <= y <= r[3] + m


def fdtd_case(board, G, c, win, mask=True):
    """the coupon inside the window as a kisi-fdtd case (via_cases.Builder): planes, traces (zero
    thickness, widened for the copper thickness), vias, antipads, pours, islands and patches, the
    solder mask as a dielectric layer, capacitors as a 5 mΩ lumped short on a 0.5 mm body loop (as
    via_cases.pdn_loop); lumped 50 Ω ports where traces leave the window"""
    stk = board.stk
    x0, x1 = c.cx + win["x0"], c.cx + win["x1"]
    hy = win["half_y"] or min(c.H / 2 + 0.65, 7.0)
    xc, yc = (x0 + x1) / 2, c.cy
    W = (x0, yc - hy, x1, yc + hy)
    half_x = (x1 - x0) / 2 + 1.5
    isl_here = [isl for isl in G["islands"] if isl["cid"] == c.cid]
    key = stk.vc_key
    if isl_here:  # the island layer is not a full plane in this window
        key = stk.vc_key + "_isl"
        st = dict(vc.STACKUPS[stk.vc_key])
        st["planes"] = [p for p in stk.planes if p not in [isl["layer"] for isl in isl_here]]
        vc.STACKUPS[key] = st
    b = vc.Builder(key, half_x, hy, win["fmax"], win["dfine"])
    b.case["mesh"]["thirds"] = True
    loc = lambda x, y: (round(x - xc, 5), round(y - yc, 5))
    ports = []
    for t in G["traces"]:
        if t["cid"] != c.cid or t["net"] == "GND":  # GND: pad-to-via links, covered by the via pads
            continue
        L = t["layer"]
        wf = round(t["w"] + stk.dw_thick(L), 4)
        for a, q in zip(t["pts"][:-1], t["pts"][1:]):
            if abs(a[1] - q[1]) > 1e-6:  # y-directed or diagonal piece
                inside = in_rect(a[0], a[1], W) or in_rect(q[0], q[1], W)
                if inside and abs(a[0] - q[0]) > 1e-6:
                    raise ValueError(f"{c.cid}/{win['name']}: diagonal trace inside the 3D window")
                if inside:
                    ya, yb = max(min(a[1], q[1]), W[1]), min(max(a[1], q[1]), W[3])
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
    clip = lambda r: (max(r[0], W[0]), max(r[1], W[1]), min(r[2], W[2]), min(r[3], W[3]))
    for p in G["pours"]:  # this coupon's pours, ending at the port planes
        if p["cid"] == c.cid:
            rr = clip(p["rect"])
            if rr[2] - rr[0] > 0.05 and rr[3] - rr[1] > 0.05:
                a_, b_ = loc(rr[0], rr[1]), loc(rr[2], rr[3])
                b.sheet(stk.z[p["layer"]], [a_[0], a_[1], b_[0], b_[1]], net="GND")
    hole_loc = lambda s: [*loc(s[1], s[2]), s[3]]
    for z_ in [z for z in G["zones"] if z["cid"] == c.cid]:  # copper patches on signal layers
        a_, b_ = loc(z_["rect"][0], z_["rect"][1]), loc(z_["rect"][2], z_["rect"][3])
        b.sheet(stk.z[z_["layer"]], [a_[0], a_[1], b_[0], b_[1]], [hole_loc(s) for s in z_["holes"]], net="PWR")
    dom = [-half_x, -hy, half_x, hy]
    for isl in isl_here:
        r, g, z = isl["rect"], isl.get("clearance", 0.5), stk.z[isl["layer"]]
        a_, b_ = loc(r[0], r[1]), loc(r[2], r[3])
        b.sheet(z, [a_[0], a_[1], b_[0], b_[1]], [hole_loc(s) for s in isl["holes"]], net="PWR")
        for rr in ((dom[0], dom[1], dom[2], a_[1] - g), (dom[0], b_[1] + g, dom[2], dom[3]), (dom[0], a_[1] - g, a_[0] - g, b_[1] + g), (b_[0] + g, a_[1] - g, dom[2], b_[1] + g)):
            if rr[2] - rr[0] > 0.05 and rr[3] - rr[1] > 0.05:
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
                b.antipad(L, *loc(s[1], s[2]), s[3])
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
            b.via(p_[0], p_[1], v["drill"] / 2, v["pad"] / 2, 0, stk.n - 1, v["pads"] if v["pads"] is not None else list(range(stk.n)), net="SIG")
    top = stk.z[0]
    for part in [p for p in G["parts"] if p["cid"] == c.cid]:
        pads = [(part["x"] + pd["x"], part["y"] + pd["y"], pd) for pd in part["pads"]]
        for (x, y, pd) in pads:  # pads as sheets (circles as squares of the same area)
            if not in_rect(x, y, W):
                continue
            sx, sy = (pd["sx"] * 0.886, pd["sy"] * 0.886) if pd.get("shape") == "circle" else (pd["sx"], pd["sy"])
            p_ = loc(x, y)
            b.sheet(top, [round(p_[0] - sx / 2, 5), round(p_[1] - sy / 2, 5), round(p_[0] + sx / 2, 5), round(p_[1] + sy / 2, 5)], net="PAD")
        if part["prefix"] == "C" and all(in_rect(x, y, W) for x, y, _ in pads):
            (xa, ya, pa), (xb, yb, pb) = pads
            gnd_first = pa["net"] == "GND"
            (xg, yg), (xs, ys) = ((xa, ya), (xb, yb)) if gnd_first else ((xb, yb), (xa, ya))
            g_, s_ = loc(xg, yg), loc(xs, ys)
            hb = 0.5
            b.cyl(s_[0], s_[1], 0.1, top, top + hb, net="PWR")
            lo_ = (min(g_[0], s_[0]) - 0.15, min(g_[1], s_[1]) - 0.15, max(g_[0], s_[0]) + 0.15, max(g_[1], s_[1]) + 0.15)
            b.sheet(top + hb, [round(v, 5) for v in lo_], net="PWR")
            # the short: a z-directed lumped element at the GND pad, between the pad and the body sheet
            b.lumped(g_[0], g_[1], top, top + hb, r=0.005, w=0.3, name=f"{part['package']} populated with 0 Ω (5 mΩ)")
    # ports: near (left) ports first, upper y first; 4-port order 1 P near, 2 N near, 3 P far, 4 N far
    ports.sort(key=lambda t: (t[0][0] > 0, -t[0][1]))
    for k, ((x, y), wf, L) in enumerate(ports):
        b.port(x, y, wf, L, stk.port_ref(L), str(k + 1))
    for k_, v_ in win["extra"].items():
        if k_ == "meta_modes":
            b.case["meta_modes"] = v_
        elif k_ not in ("capz", "hpdn"):
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


# --- launch optimisation cases (kisi-fdtd) ------------------------------------------------------------
F08 = {"pitch": 0.8, "first_x": 0.8, "rows": 2, "row_pitch": 0.8, "offset": 0.4, "hole_keep": 0.45}   # design v3 first draft
LAUNCH_VARIANTS = {
    # name: overrides of connectors.json 'launch' ('pin': the connector's pin as a strip on the tip; 'body': its top jaw)
    "default": {},
    "default_body": {"body": True},
    "notip_f05": {"fence": dict(F08, pitch=0.5, first_x=0.5)},
    "notip_f08": {"fence": F08},
    "tip023_f08": {"tip_w": 0.229, "fence": F08},
    "tip023_pin_f08": {"tip_w": 0.229, "pin": True, "fence": F08},
    "samtec": {"tip_w": 0.229, "pad_w": 0.521, "pad_len": 6.477, "pad_gap": 1.188, "fence": F08},
    "samtec_cut": {"tip_w": 0.229, "pad_w": 0.521, "pad_len": 6.477, "pad_gap": 1.188, "fence": F08, "gnd_cut": {"width": 0.521, "length": 6.477}},
}


def launch_case(stk_key, des, var, over, fmax=40e9, dfine=0.035, line=2.5):
    """one launch as a kisi-fdtd case: the board from its edge (x = 0) to launch_length + `line`
    mm of the 50 Ω microstrip, the Samtec ground pads (top and bottom), the plated screw holes, the
    tip, pad and coplanar launch with its fence, the optional ground cut, and (body) the top jaw of
    the connector as metal sheets with its slot and posts along the slot walls (from Samtec's 3D
    model: jaw 5.08 mm over the board, slot 2.67 / 3.18 mm wide from 0.38 / 0.64 mm). Port 1 is a
    lumped 50 Ω port at the edge (tip to the first plane, which keeps a 0.25 mm strip at the edge
    when it is cut out): the coax and the pin of the connector are NOT modelled (CST with the STEP
    model does that). Port 2 at the end of the microstrip, 1.5 mm from the domain boundary."""
    stk = Stack(stk_key)
    lc = dict(LNC, **{k: v for k, v in over.items() if k in LNC})
    w, gap, wm = des["launch"]["w"], des["launch"]["gap"], des["w50"]["0"]
    tip = lc["tip_w"] if lc["tip_w"] is not None else (lc["pad_w"] or w)
    pw = lc["pad_w"] or w
    pg = lc["pad_gap"] if lc["pad_gap"] is not None else gap
    tl = lc["taper_len"]
    pl = max(lc["pad_len"] or tl, tl)
    L = max(lc["launch_length"], pl)
    cut = lc["gnd_cut"] if lc["gnd_cut"].get("width") else (({"width": des["launch"]["cut"], "length": L}) if des["launch"].get("cut") else None)
    pb = FP["edge_pullback"]
    xp = L + line                # port 2: the line ends there (a line running on into the PML would load the port)
    x1 = xp + 1.5
    hy = 6.0
    b = vc.Builder(stk.vc_key, x1, hy + 0.5, fmax, dfine, board=[0.0, -(hy + 0.5), x1, hy + 0.5], x0=-0.6)
    zt, zb = stk.z[0], stk.z[-1]
    sig = lambda poly: b.case["sheets"].append({"z": zt, "rect": [min(p[0] for p in poly), min(p[1] for p in poly), max(p[0] for p in poly), max(p[1] for p in poly)],
                                                "polys": [[[round(x, 5), round(y, 5)] for x, y in poly]], "net": "SIG"})
    dw = stk.dw_thick(0)
    hwt = lambda u: (tip + (pw - tip) * min(u, tl) / tl) / 2 + dw / 2
    sig([(pb, -hwt(pb)), (tl, -hwt(tl)), (tl, hwt(tl)), (pb, hwt(pb))])
    if pl > tl:
        b.case["sheets"].append({"z": zt, "rect": [tl, -(pw + dw) / 2, pl, (pw + dw) / 2], "net": "SIG"})
    b.case["sheets"].append({"z": zt, "rect": [pl, -(w + dw) / 2, L, (w + dw) / 2], "net": "SIG"})
    b.case["sheets"].append({"z": zt, "rect": [L, -(wm + dw) / 2, xp, (wm + dw) / 2], "net": "SIG"})
    # ground: Samtec pads top and bottom (polygons), coplanar pours on top
    for sg in (1, -1):
        poly = [[round(u, 5), round(sg * v, 5)] for u, v in gnd_pad_outline()]
        for z in (zt, zb):
            b.case["sheets"].append({"z": z, "rect": [pb, -hy, FP["gnd_pad"]["depth"], hy], "polys": [poly], "net": "GND"})
        g_pad, g_cp = pw / 2 + pg, w / 2 + gap
        if pl > tl + 1e-9:
            b.sheet(zt, [ZONE_INSET, min(sg * g_pad, sg * hy), pl, max(sg * g_pad, sg * hy)], net="GND")
            b.sheet(zt, [pl, min(sg * g_cp, sg * hy), L, max(sg * g_cp, sg * hy)], net="GND")
        else:
            b.sheet(zt, [ZONE_INSET, min(sg * g_cp, sg * hy), L, max(sg * g_cp, sg * hy)], net="GND")
    # fence and screw holes
    fn = lc["fence"]
    g_f = max(pw / 2 + pg, w / 2 + gap)
    holes = [(h["x"], h["y"], h["d"]) for h in FP["mount_holes"]]
    k = 0
    while fn["first_x"] + k * fn["pitch"] <= L - 0.35:
        xx = fn["first_x"] + k * fn["pitch"]
        for sg in (1, -1):
            for row in range(fn["rows"]):
                yy = sg * (g_f + fn["offset"] + row * fn["row_pitch"])
                if all(math.hypot(xx - hx, yy - hy_) >= hd / 2 + LAY["gnd_via"]["drill"] / 2 + fn["hole_keep"] for hx, hy_, hd in holes):
                    b.via(round(xx, 4), round(yy, 4), LAY["gnd_via"]["drill"] / 2, LAY["gnd_via"]["pad"] / 2, 0, stk.n - 1, [], net="GND")
        k += 1
    body = over.get("body")
    top_m = zt + (0.9 if body else 0.0)
    for hx, hy_, hd in holes:  # plated holes (and the screw through the jaw)
        b.case["vias"].append({"x": hx, "y": hy_, "r": hd / 2, "z0": round(zb, 6), "z1": round(top_m if body else zt, 6), "pad_r": FP["hole_ring"] / 2,
                               "pads": [stk.z[i] for i in range(stk.n)], "net": "GND", "dmesh": 0.1})
    if cut:
        b.rect_cut(stk.planes[0], 0.25, -cut["width"] / 2, cut["length"], cut["width"] / 2)
    if over.get("pin"):  # the launch pin (Ø0.18 mm, 0.74 mm onto the board) as a strip at its axis height, tied to the tip
        r_pin = CONN["pin_diameter"] / 2
        b.sheet(zt + r_pin, [round(pb, 4), -r_pin, CONN["pin_length"], r_pin], net="SIG")
        for xx in (pb + 0.1, (pb + CONN["pin_length"]) / 2, CONN["pin_length"] - 0.05):
            b.cyl(round(xx, 4), 0.0, 0.05, zt, zt + r_pin, net="SIG")
    if body:  # the top jaw: metal sheets at four heights with the slot, posts along the slot walls and the lip
        jb = FP["block_width"] / 2
        for dz in (0.0, 0.25, 0.5, 0.9):   # the jaw's underside at the copper height (it rests on the ground pads)
            z = zt + dz
            for sg in (1, -1):
                b.sheet(z, [0.38, min(sg * 1.335, sg * jb), 0.64, max(sg * 1.335, sg * jb)], net="GND")
                b.sheet(z, [0.64, min(sg * 1.59, sg * jb), FP["body_on_board"], max(sg * 1.59, sg * jb)], net="GND")
            if dz > 0.4:  # the lip above the pin opening (Ø0.70 at 0.13 mm above the board: top at +0.48)
                b.sheet(z, [0.0, -jb, 0.38, jb], net="GND")
        for sg in (1, -1):
            b.sheet(zt, [0.0, min(sg * 0.33, sg * jb), 0.38, max(sg * 0.33, sg * jb)], net="GND")
            ys = [(0.38, 0.64, 1.335), (0.64, FP["body_on_board"], 1.59)]
            for xa, xb_, yw in ys:
                n = max(2, int((xb_ - xa) / 0.25) + 1)
                for i in range(n):
                    b.cyl(round(xa + (xb_ - xa) * i / (n - 1), 4), sg * yw, 0.05, zt, zt + 0.9, net="GND")
            for i in range(3):  # sides of the pin opening
                b.cyl(round(0.38 * i / 2, 4), sg * 0.38, 0.05, zt, zt + 0.48, net="GND")
    # ports
    b.port_raw(pb, 0.0, stk.z[stk.planes[0]], zt, 50.0, "1 edge (launch)", "L1", stk.names[stk.planes[0]], w=round(2 * hwt(pb), 4))
    b.port(xp, 0.0, round(wm + dw, 4), 0, stk.planes[0], "2 line")
    title = f"Samtec 292-J-P-EP-ST-EL-01 launch on {stk.d['title']}: {var}"
    params = {"variant": var, "tip_w": tip, "taper_len": tl, "pad_w": pw, "pad_len": pl, "pad_gap": pg, "w": w, "gap": gap, "launch_length": L,
              "gnd_cut": cut, "fence": fn, "body": bool(body), "pin": bool(over.get("pin")), "line_w": wm}
    case = b.finish(f"launch_{stk_key}_{var}", "launch", title, params)
    t = stk.mask["thickness"]
    case["layers"].append({"z0": round(zt, 6), "z1": round(zt + t, 6), "er": stk.mask["er"], "tand": stk.mask["tand"]})
    case["layers"].insert(0, {"z0": round(zb - t, 6), "z1": round(zb, 6), "er": stk.mask["er"], "tand": stk.mask["tand"]})
    case["meta"].update(excite=[0], mirror=None, target="|S11| < -20 dB to 40 GHz at the edge port (Samtec's own fixture: -23 to -25 dB, report 2023-06-01)",
                        limits="connector coax and pin not modelled; port 1 is a lumped port at the edge; the jaw is approximated (body)")
    return case


def write_launch_cases(stk_keys, outdir, fdtd_exe=None):
    for sk in stk_keys:
        des = json.load(open(os.path.join(outdir, "boards", f"{sk}.design.json")))
        d = os.path.join(outdir, "fdtd", f"launch_{sk}")
        os.makedirs(d, exist_ok=True)
        for var, over in LAUNCH_VARIANTS.items():
            if des["launch"].get("cut") and var.startswith("samtec"):
                continue
            case = launch_case(sk, des, var, over)
            cp = os.path.join(d, case["name"] + ".json")
            json.dump(case, open(cp, "w"), indent=1)
            cells = ""
            if fdtd_exe:
                m = json.loads(subprocess.run([fdtd_exe, cp, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
                cells = f", {len(m['x']) * len(m['y']) * len(m['z']) / 1e6:.2f} M cells"
            print(f"{case['name']}{cells}", flush=True)


# --- main -------------------------------------------------------------------------------------------
def build(board_key, ks, pair_pitch, outdir, design_cache, fdtd_exe=None):
    stk_key, part = BOARDS[board_key]
    stk = Stack(stk_key)
    if stk_key not in design_cache:
        des = design(stk, ks, pair_pitch)
        des["z_table"] = report_z(stk, ks, des)
        des.update(stackup=stk_key, date=TODAY)
        json.dump(des, open(os.path.join(outdir, "boards", f"{stk_key}.design.json"), "w"), indent=1)
        design_cache[stk_key] = des
    des = design_cache[stk_key]
    ctx = Ctx(stk, des, pair_pitch)
    rows = board_specs(ctx)[part]
    # rows sorted by length (stable): every row rests on a row at least as long (a stepped pyramid)
    rows.sort(key=lambda r: sum(c.X for c in r))
    name = f"kisi-coupons-{board_key.replace('_', '-')}"
    b = Board(name, stk, rows)
    b.design = des
    b.place()
    Wd, Hd = b.width(), b.height()
    lim = OVERSIZE_OK.get(board_key) or LAY["board_limits"][str(stk.n)]
    if stk.d["manufacturer"] == "JLCPCB" and (Wd > lim[0] + 1e-6 or Hd > lim[1] + 1e-6):
        raise ValueError(f"{name}: {Wd:g} × {Hd:g} mm exceeds the {lim[0]:g} × {lim[1]:g} mm limit for {stk.n} layers")
    # board name and order-number placeholder on the bottom silk, in the lower half of the bottom
    # (longest) row, away from the end launches that clamp the board edges
    yb = b.bottom
    b.texts.append((name, 0.0, yb + 4.8, "B.SilkS", 0.8, None))
    b.texts.append((f"{stk.d['template'] or 'assumed stackup'}  v{DESIGN_REV}", 0.0, yb + 3.6, "B.SilkS", 0.8, None))
    b.texts.append((TODAY, 0.0, yb + 2.4, "B.SilkS", 0.8, None))
    if stk.d["manufacturer"] == "JLCPCB":  # JLC prints its order number where this text is
        b.texts.append(("JLCJLCJLCJLC", 0.0, yb + 1.2, "B.SilkS", 0.8, None))
    path = os.path.join(outdir, "boards", name + ".kicad_pcb")
    G = b.write_kicad(path)
    b.write_kisi_config(path[:-len(".kicad_pcb")] + ".kisi.json")
    fdir = os.path.join(outdir, "fdtd", board_key)
    os.makedirs(fdir, exist_ok=True)
    for f in os.listdir(fdir):
        if f.endswith(".json"):
            os.remove(os.path.join(fdir, f))
    cases, cells = [], {}
    for c in b.coupons:
        for win in c.windows:
            case = fdtd_case(b, G, c, win)
            cp = os.path.join(fdir, case["name"] + ".json")
            json.dump(case, open(cp, "w"), indent=1)
            cases.append(case["name"])
            if fdtd_exe:  # mesh only: cell count of the case
                m = json.loads(subprocess.run([fdtd_exe, cp, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
                cells[case["name"]] = len(m["x"]) * len(m["y"]) * len(m["z"])
    manifest = {"board": name, "key": board_key, "part": PART_TITLE.get(part, part), "file": os.path.basename(path), "stackup": stk_key,
                "stackup_title": stk.d["title"], "layers": stk.n, "manufacturer": stk.d["manufacturer"], "template": stk.d.get("template"), "date": TODAY,
                "size_mm": [round(Wd, 2), round(Hd, 2)], "thickness_mm": round(stk.thickness(), 4),
                "coupons": [{"id": c.cid, "group": c.group, "title": c.title, "length_mm": c.X, "height_mm": c.H, "validates": c.validates,
                             "references": c.refs, "params": c.params, "nets": c.nets, "measure": c.measure, "connectors": b.jmap.get(c.cid, []),
                             "parts": b.refmap.get(c.cid, []), "fdtd": [f"{name}__{c.cid}__{w['name']}" for w in c.windows]} for c in b.coupons],
                "fdtd_cases": cases, "fdtd_cells": cells}
    json.dump(manifest, open(os.path.join(outdir, "boards", name + ".manifest.json"), "w"), indent=1)
    print(f"{name}: {len(b.coupons)} coupons, {Wd:g} × {Hd:g} mm, {len(cases)} 3D cases", flush=True)
    return manifest


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--boards", default=",".join(BOARDS), help="board keys: " + ", ".join(BOARDS))
    ap.add_argument("--first-order", action="store_true", help="only the first-order boards: " + ", ".join(k for k in BOARDS if BOARDS[k][1].startswith("f")))
    ap.add_argument("--out", default=HERE)
    ap.add_argument("--kisi", default=os.path.join(REPO, "target", "release", "kisi"))
    ap.add_argument("--kisi-mjs", default=os.path.join(REPO, "dist", "kisi.mjs"))
    ap.add_argument("--solver", default="auto", choices=["auto", "field", "formula"])
    ap.add_argument("--pair-pitch", type=float, default=LAY["pair_pitch"])
    ap.add_argument("--reuse-design", action="store_true", help="use boards/<stackup>.design.json if present (keeps the trace widths; "
                    "stackup values may have changed, e.g. after the extraction)")
    ap.add_argument("--fdtd-exe", default=None, help="kisi-fdtd binary: record the cell count of every 3D case (mesh only)")
    ap.add_argument("--launch-cases", default=None, help="stackup keys: write only the launch optimisation cases (fdtd/launch_<stackup>/) and stop")
    a = ap.parse_args()
    if a.launch_cases:
        write_launch_cases([k for k in a.launch_cases.split(",") if k], a.out, a.fdtd_exe)
        return
    os.makedirs(os.path.join(a.out, "boards"), exist_ok=True)
    ks = Kisi(a.kisi, a.kisi_mjs, a.solver)
    cache = {}
    if a.reuse_design:
        for f in os.listdir(os.path.join(a.out, "boards")):
            if f.endswith(".design.json"):
                d = json.load(open(os.path.join(a.out, "boards", f)))
                if d.get("pair_pitch") == a.pair_pitch and "pair_via" in d and len(d["pair_via"]) == len(STK["stackups"][d["stackup"]]["signal"]):
                    cache[d["stackup"]] = d
    keys = [k for k in a.boards.split(",") if k]
    if a.first_order:
        keys = [k for k in keys if BOARDS[k][1].startswith("f")]
    need = {BOARDS[k][0] for k in keys} - set(cache)
    if need:
        print(f"trace widths for {', '.join(sorted(need))} from kisi ({ks.solver}: {a.kisi_mjs if ks.solver == 'field' else a.kisi})", flush=True)
    for k in keys:
        build(k, ks, a.pair_pitch, a.out, cache, a.fdtd_exe)
    write_library(os.path.join(a.out, "boards"), prune=set(keys) == set(BOARDS))
    print(f"kisi probe runs: {ks.calls}")


def write_library(bdir, prune=False):
    """the project-local footprint library next to the boards: kisi_coupons.pretty/<name>.kicad_mod
    (every footprint the boards use, generated here; the Samtec one from connectors.json) and the
    fp-lib-table that points KiCad to it (${KIPRJMOD}); prune: remove footprints no board uses"""
    lib = os.path.join(bdir, f"{LIBNICK}.pretty")
    os.makedirs(lib, exist_ok=True)
    for name, text in sorted(LIBRARY.items()):
        open(os.path.join(lib, name + ".kicad_mod"), "w").write(text)
    if prune:
        for fn in os.listdir(lib):
            if fn.endswith(".kicad_mod") and fn[:-len(".kicad_mod")] not in LIBRARY:
                os.remove(os.path.join(lib, fn))
    open(os.path.join(bdir, "fp-lib-table"), "w").write(
        "(fp_lib_table\n  (version 7)\n"
        f'  (lib (name "{LIBNICK}") (type "KiCad") (uri "${{KIPRJMOD}}/{LIBNICK}.pretty") (options "") '
        '(descr "kisi measurement coupons: footprints generated by validation/test_board/make_coupons.py"))\n)\n')


if __name__ == "__main__":
    main()
