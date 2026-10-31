"""CST Studio Suite model of the test-board connector launch: Samtec 292-J-P-EP-ST-EL-01 (2.92 mm jack,
edge launch, compression mount) on the coupons of make_coupons.py, with Samtec's STEP model imported
(README_cst_launch.md).

Models (--model):
  launch1       one connector + 12.5 mm feed (the coupons' feed: half of the 25 mm 2xthru) on jlc4-7628;
                port 1 a waveguide port on the connector's coax, referred to the 2.92 mm mating plane;
                port 2 a waveguide port on the line cross-section at the end of the feed (or --port2 face)
  thru2         the 2xthru as measured: two connectors, 25 mm board, both ports on the coax
  samtec_bench  two connectors (--connectors 1: one) + 25.4 mm (12.7 mm) 50 ohm grounded coplanar line on an
                ASSUMED 4-layer I-Tera MT40 stackup: a plausibility check against Samtec's RF report

The board side (Samtec ground pads, plated screw holes, launch, fence, planes) is the kisi-fdtd launch case of
make_coupons.launch_case(), so it follows connectors.json. The connector (--connector parametric, default) is built
from CST primitives directly in model coordinates with the dimensions measured from Samtec's STEP model
(connectors.json connector.step_model): air line, support bead with its holes, contact, a bore of assumed impedance
where the STEP is solid around the contact, lip, pin, jaw with its slot, lower plate. --connector step imports
Samtec's STEP instead (--step, not in the repository: its licence forbids redistribution; --step-scale 25.4 if CST
does not convert its inches), for comparison only. Same model conventions as validation/fdtd/cst_case.py (branch feat/cst-refs):
zero-thickness PEC copper widened by Wheeler's dw, dielectric loss as a constant conductivity at fmax/2
(--loss tand: CST's constant-tan(delta) model), time-domain solver, PBA, open (PML) boundaries.

usage:
  python3 cst_launch.py --model launch1 --dry-run --out OUT            # .bas + model JSON + geometry plot, no CST
  python cst_launch.py --model thru2 --out OUT                              # with CST 2025.5's python_cst_libraries
  python cst_launch.py --from-json launch1_default_jlc4_7628.model.json --out OUT   # outside the repository
Options: --variant (launch variants of make_coupons / connectors.json), --mesh fine|mid|base|coarse|auto
(estimated cells against --max-cells), --monitors (E-field at 10/20/30/40 GHz), --bead-er/--bead-tand,
--metal pec|lossy, --symmetry magnetic|none, --walls, --check-occ (OpenCASCADE check, pip gmsh),
--connector step [--step-scale, --part ROLE=PATTERN, --coax-port inner|ext].
"""
import argparse, datetime, json, math, os, re, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
C0 = 299792458.0
EPS0 = 8.8541878128e-12
STEP_TOKEN = "@@STEP_FILE@@"
MESH_PRESETS = {  # CST.md (feat/cst-refs): base 20/20, mid 25/25, fine 30/30; coarse only as a last resort
    "fine": {"lpw": 30, "lpb": 30}, "mid": {"lpw": 25, "lpb": 25}, "base": {"lpw": 20, "lpb": 20}, "coarse": {"lpw": 15, "lpb": 15},
    "rough": {"lpw": 12, "lpb": 12, "edge": 2, "ratio": 10}}  # last resort for the 1 M licence; check with a finer run
MESH_ORDER = ["fine", "mid", "base", "coarse", "rough"]
MONITOR_GHZ = [10, 20, 30, 40]
# launch variants: make_coupons.LAUNCH_VARIANTS (same names as the kisi-fdtd results in fdtd_ref/launch_<stackup>/)
# without the ones that approximate the connector (pin, body: here the STEP model is the connector), plus two
# with Samtec's footprint values and our fence
EXTRA_VARIANTS = {"tip023": "Samtec's 0.23 mm tip (drawing .0090 in), our fence 0.55 mm",
                  "samtec_pad": "Samtec's general signal pad 0.52 mm / gap 1.19 mm with the 0.23 mm tip, our fence 0.55 mm"}

for _s in (sys.stdout, sys.stderr):  # CST's console may not be UTF-8
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


# --- VBA helpers (patterns of validation/fdtd/cst_case.py, feat/cst-refs, proven on CST 2025.5) -------------
def q(x):
    """a number as a CST string argument (always a decimal point)"""
    if isinstance(x, str):
        return f'"{x}"'
    return f'"{float(x):.9g}"'


def sigma(er, tand, f_loss):
    return 2 * math.pi * f_loss * EPS0 * er * tand


def clip_ring(ring, x0, y0, x1, y1):
    """Sutherland-Hodgman: a ring [[x, y], ...] clipped to a rectangle"""
    def cut(pts, inside, inter):
        res = []
        for k in range(len(pts)):
            a, b = pts[k - 1], pts[k]
            if inside(b):
                if not inside(a):
                    res.append(inter(a, b))
                res.append(b)
            elif inside(a):
                res.append(inter(a, b))
        return res

    def ix(a, b, x):
        return [x, a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0])]

    def iy(a, b, y):
        return [a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]), y]

    pts = [list(p) for p in ring]
    for inside, inter in ((lambda p: p[0] >= x0, lambda a, b: ix(a, b, x0)), (lambda p: p[0] <= x1, lambda a, b: ix(a, b, x1)),
                          (lambda p: p[1] >= y0, lambda a, b: iy(a, b, y0)), (lambda p: p[1] <= y1, lambda a, b: iy(a, b, y1))):
        if not pts:
            break
        pts = cut(pts, inside, inter)
    return pts


def clean_ring(pts, tol=1e-6):
    """CST rejects polygons with repeated or collinear vertices; None if no area remains"""
    out = []
    for p in pts:
        if not out or abs(p[0] - out[-1][0]) > tol or abs(p[1] - out[-1][1]) > tol:
            out.append([p[0], p[1]])
    while len(out) > 1 and abs(out[0][0] - out[-1][0]) <= tol and abs(out[0][1] - out[-1][1]) <= tol:
        out.pop()
    changed = True
    while changed and len(out) >= 3:
        changed = False
        for k in range(len(out)):
            a, b, c = out[k - 1], out[k], out[(k + 1) % len(out)]
            if abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) < tol * tol:
                out.pop(k)
                changed = True
                break
    if len(out) < 3:
        return None
    area = sum(out[k - 1][0] * out[k][1] - out[k][0] * out[k - 1][1] for k in range(len(out))) / 2
    return out if abs(area) > 1e-8 else None


def inside_ring(x, y, ring):
    odd = False
    for k in range(len(ring)):
        (xa, ya), (xb, yb) = ring[k - 1], ring[k]
        if (ya > y) != (yb > y) and xa + (y - ya) * (xb - xa) / (yb - ya) > x:
            odd = not odd
    return odd


def copper_at(s, x, y):
    """is the sheet copper at (x, y) (rect, holes, even-odd polygon rings)?"""
    r = s["rect"]
    if not (r[0] - 1e-9 <= x <= r[2] + 1e-9 and r[1] - 1e-9 <= y <= r[3] + 1e-9):
        return False
    if s.get("polys") and sum(inside_ring(x, y, rg) for rg in s["polys"]) % 2 == 0:
        return False
    for h in s.get("holes") or []:
        if len(h) < 5 and math.hypot(x - h[0], y - h[1]) < h[2]:
            return False
    return True


class Blocks:
    """VBA history blocks [(caption, code, kind)]; kind geo | port | mesh | setup | connector"""

    def __init__(self):
        self.blocks = []
        self.n = 0

    def add(self, caption, code, kind="geo"):
        self.blocks.append({"caption": caption, "code": code.strip() + "\n", "kind": kind})

    def name(self, base):
        self.n += 1
        return f"{base}{self.n}"

    @staticmethod
    def wcs(z):
        return (f'WCS.ActivateWCS "local"\nWCS.SetNormal "0", "0", "1"\nWCS.SetOrigin "0", "0", {q(z)}\n'
                f'WCS.SetUVector "1", "0", "0"\n')

    @staticmethod
    def wcs_off():
        return 'WCS.ActivateWCS "global"\n'

    def cover(self, c, comp, name, material):
        return (f'With CoverCurve\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{material}"\n .Curve "{c}:{c}"\n'
                f' .DeleteCurve "True"\n .Create\nEnd With\n')

    def rect_sheet(self, comp, name, x0, y0, x1, y1, z, material="PEC"):
        c = self.name("c")
        return (self.wcs(z) + f'With Rectangle\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Xrange {q(x0)}, {q(x1)}\n .Yrange {q(y0)}, {q(y1)}\n'
                f' .Create\nEnd With\n' + self.cover(c, comp, name, material) + self.wcs_off())

    def circle_sheet(self, comp, name, x, y, r, z, material="PEC"):
        c = self.name("c")
        return (self.wcs(z) + f'With Circle\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Radius {q(r)}\n .Xcenter {q(x)}\n .Ycenter {q(y)}\n'
                f' .Segments "0"\n .Create\nEnd With\n' + self.cover(c, comp, name, material) + self.wcs_off())

    def poly_sheet(self, comp, name, pts, z, material="PEC"):
        c = self.name("c")
        s = f'With Polygon\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Point {q(pts[0][0])}, {q(pts[0][1])}\n'
        for p in pts[1:] + [pts[0]]:
            s += f' .LineTo {q(p[0])}, {q(p[1])}\n'
        s += ' .Create\nEnd With\n'
        return self.wcs(z) + s + self.cover(c, comp, name, material) + self.wcs_off()


def brick(name, comp, mat, x0, x1, y0, y1, z0, z1):
    return (f'With Brick\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{mat}"\n .Xrange {q(x0)}, {q(x1)}\n'
            f' .Yrange {q(y0)}, {q(y1)}\n .Zrange {q(z0)}, {q(z1)}\n .Create\nEnd With\n')


def cyl_x(name, comp, mat, x0, x1, r, ri=0.0):
    """a cylinder along x on the axis y = z = 0 (STEP coordinates of the connector)"""
    return (f'With Cylinder\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{mat}"\n .OuterRadius {q(r)}\n'
            f' .InnerRadius {q(ri)}\n .Axis "x"\n .Xrange {q(x0)}, {q(x1)}\n .Ycenter "0"\n .Zcenter "0"\n .Segments "0"\n .Create\nEnd With\n')


def cyl_z(name, comp, mat, x, y, r, z0, z1):
    return (f'With Cylinder\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{mat}"\n .OuterRadius {q(r)}\n'
            f' .InnerRadius "0"\n .Axis "z"\n .Zrange {q(z0)}, {q(z1)}\n .Xcenter {q(x)}\n .Ycenter {q(y)}\n .Segments "0"\n .Create\nEnd With\n')


# --- model ------------------------------------------------------------------------------------------------------
def load_connectors():
    return json.load(open(os.path.join(HERE, "connectors.json"), encoding="utf-8"))


def variants():
    import make_coupons as mc
    v = {k: dict(o) for k, o in mc.LAUNCH_VARIANTS.items() if not (o.get("pin") or o.get("body"))}
    sg = mc.LNC["samtec_general"]
    tip = 0.229  # drawing sheet 2: .0090 in
    v["tip023"] = {"tip_w": tip}
    v["samtec_pad"] = {"tip_w": tip, "pad_w": sg["pad_w"], "pad_len": sg["pad_len"], "pad_gap": sg["pad_gap"]}
    return v


def launch_len(lnc, over):
    lc = dict(lnc, **{k: v for k, v in over.items() if k in lnc})
    tl = lc["taper_len"]
    pl = max(lc["pad_len"] or tl, tl)
    return max(lc["launch_length"], pl)


def mirror_sheet(s, X):
    """sheet mirrored at x = X/2 (x -> X - x)"""
    t = dict(s)
    r = s["rect"]
    t["rect"] = [round(X - r[2], 6), r[1], round(X - r[0], 6), r[3]]
    if s.get("polys"):
        t["polys"] = [[[round(X - p[0], 6), p[1]] for p in rg][::-1] for rg in s["polys"]]
    if s.get("holes"):
        t["holes"] = [[round(X - h[0], 6)] + list(h[1:]) for h in s["holes"]]
    return t


def build_model(a):
    """the model as plain data (written as <name>.model.json; --from-json rebuilds the VBA from it)"""
    sys.path.insert(0, HERE)
    import make_coupons as mc
    con = mc.CON["connector"]
    sm = con["step_model"]
    fp = mc.FP
    var_all = variants()
    bench = a.model == "samtec_bench"
    if bench:
        cm = con["benchmark"]["cst_model"]
        stk_key = "samtec_mt40_assumed"
        mc.STK["stackups"][stk_key] = dict(cm["stackup"])
        des = {"launch": {"w": cm["line"]["w"], "gap": cm["line"]["gap"]}, "w50": {"0": cm["line"]["w"]}}
        nconn = a.connectors or 2
        feed = a.feed or (cm["line_mm"] / 2 if nconn == 2 else cm["line_one_mm"])
        if a.variant not in var_all:
            raise SystemExit(f"unknown variant {a.variant}: {', '.join(var_all)}")
        over = dict(var_all[a.variant], launch_length=feed)  # the grounded coplanar line from the edge to the port
        walls = False
        title = f"Samtec benchmark: {nconn} connector(s) + {feed * nconn:g} mm 50 ohm GCPW on assumed I-Tera MT40"
    else:
        stk_key = a.stackup
        dp = os.path.join(HERE, "boards", f"{stk_key}.design.json")
        des = json.load(open(dp, encoding="utf-8"))
        nconn = 1 if a.model == "launch1" else 2
        feed = a.feed or mc.LAY["feed"]
        if a.variant not in var_all:
            raise SystemExit(f"unknown variant {a.variant}: {', '.join(var_all)}")
        over = var_all[a.variant]
        walls = a.walls
        title = (f"{'one connector + ' + format(feed, 'g') + ' mm feed' if nconn == 1 else '2xthru: two connectors, ' + format(2 * feed, 'g') + ' mm'}"
                 f", launch '{a.variant}', {mc.STK['stackups'][stk_key]['title']}")
    L = launch_len(mc.LNC, over)
    if L > feed + 1e-9:
        raise SystemExit(f"launch length {L} mm exceeds the feed {feed} mm")
    case = mc.launch_case(stk_key, des, a.variant, over, fmax=a.fmax, line=max(feed - L, 0.0))
    stk = mc.Stack(stk_key)
    z_top, z_bot = stk.z[0], stk.z[-1]
    z_ref = stk.z[stk.planes[0]]
    hy = case["board"][3]
    face2 = nconn == 1 and a.port2 == "face"
    x_end = feed + (1.5 if face2 else 0.0)  # with a face port the board runs on 1.5 mm into the PML, as in kisi-fdtd
    zi = mc.ZONE_INSET
    sheets = []
    for s in case["sheets"]:
        r = list(s["rect"])
        inner = abs(s["z"] - z_top) > 1e-9 and abs(s["z"] - z_bot) > 1e-9
        x0 = max(r[0], zi) if inner else r[0]   # planes end 0.25 mm inside the edge (zones of the boards), not at the edge
        x1 = min(r[2], x_end)
        if x1 - x0 < 1e-6 or r[3] - r[1] < 1e-6:
            continue
        t = dict(s, rect=[round(x0, 6), r[1], round(x1, 6), r[3]])
        if s.get("polys"):
            rings = []
            for k, rg in enumerate(s["polys"]):
                c = clip_ring(rg, x0, r[1] - 1, x1, r[3] + 1) if k == 0 or inner else rg
                if len(c) >= 3:
                    rings.append([[round(p[0], 6), round(p[1], 6)] for p in c])
            t["polys"] = rings
        sheets.append(t)
    vias = [dict(v) for v in case["vias"] if v["x"] <= x_end]
    wall_vias = []
    if walls:
        p, X = mc.LAY["wall_pitch"], 2 * mc.LAY["feed"]
        n = int((2 * (X / 2 - mc.WALL_EDGE)) / p)
        for k in range(n + 1):
            x = X / 2 - n * p / 2 + k * p
            if x <= feed - 0.3:
                for y in (mc.ROW / 2, -mc.ROW / 2):
                    wall_vias.append({"x": round(x, 4), "y": y, "r": mc.LAY["gnd_via"]["drill"] / 2, "z0": round(z_bot, 6), "z1": round(z_top, 6),
                                      "pad_r": mc.LAY["gnd_via"]["pad"] / 2, "pads": [], "net": "GND", "wall": True})
    vias += wall_vias
    Lb = feed * nconn if nconn == 2 else x_end
    if nconn == 2:  # mirror the half at x = feed; sheets that end on the cut become one sheet
        full = []
        for s in sheets:
            if abs(s["rect"][2] - feed) < 1e-9 and not s.get("polys") and not s.get("holes"):
                full.append(dict(s, rect=[s["rect"][0], s["rect"][1], round(Lb - s["rect"][0], 6), s["rect"][3]]))
            else:
                full += [s, mirror_sheet(s, Lb)]
        sheets = full
        vias = vias + [dict(v, x=round(Lb - v["x"], 6)) for v in vias]
    diel = [l for l in case["layers"] if l["z0"] >= z_bot - 1e-9 and l["z1"] <= z_top + 1e-9 and l["z1"] - l["z0"] > 1e-6]
    bricks = [{"name": f"layer{k}", "x": [0.0, Lb], "y": [-hy, hy], "z": [l["z0"], l["z1"]], "er": l["er"], "tand": l["tand"]} for k, l in enumerate(diel)]
    mask = stk.mask
    x_open = fp["gnd_pad"]["depth"] + fp["mask_margin"]
    y_open = fp["gnd_pad"]["half_width"] + fp["mask_margin"]
    if mask["thickness"] > 0:  # solder mask, open over the connector footprint (kisi-fdtd: everywhere)
        t = mask["thickness"]
        for side, (za, zb) in (("top", (z_top, z_top + t)), ("bot", (z_bot - t, z_bot))):
            xa, xb = x_open, (Lb - x_open if nconn == 2 else Lb)
            rects = [(xa, xb, -hy, hy), (0.0, x_open, y_open, hy), (0.0, x_open, -hy, -y_open)]
            if nconn == 2:
                rects += [(Lb - x_open, Lb, y_open, hy), (Lb - x_open, Lb, -hy, -y_open)]
            for k, (x0, x1, y0, y1) in enumerate(rects):
                bricks.append({"name": f"mask_{side}{k}", "x": [x0, x1], "y": [y0, y1], "z": [za, zb], "er": mask["er"], "tand": mask["tand"]})
    # connectors
    e = sm["board_edge_x"]
    conns = [{"id": 1, "edge_x": 0.0, "mirror": False}] + ([{"id": 2, "edge_x": Lb, "mirror": True}] if nconn == 2 else [])
    for c in conns:
        c["dz_main"] = round(z_top - sm["board_top_y"], 6)       # the jaw rests on the top copper
        c["dz_plate"] = round(z_bot - sm["board_bottom_y"], 6)   # the plate under the bottom copper
        c["dx"] = round(c["edge_x"] + e if c["mirror"] else c["edge_x"] - e, 6)
    z_axis = z_top - sm["board_top_y"]
    # ports
    ports = []
    if a.coax_port == "inner":
        x_cut = a.coax_cut
        ref = x_cut - sm["mating_plane_x"]   # positive: the reference plane moves out of the model, to the mating plane
        what = f"coax waveguide port inside the connector's Ø2.92/Ø1.27 air line (STEP x = {x_cut:g}), referred {ref:.4f} mm forward to the mating plane"
    else:
        x_cut = -a.ext_len
        ref = -(sm["mating_plane_x"] + a.ext_len)
        what = f"coax waveguide port on a {sm['mating_plane_x'] + a.ext_len:.3f} mm ideal air coax (Ø2.92/Ø1.27, the mating plug) ahead of the mating plane, referred to it"
    if a.connector == "parametric" and a.coax_port != "inner":
        raise SystemExit("--coax-port ext: only with --connector step")
    hp = a.coax_port_half or sm["oc_bore"]["r"]  # the port square circumscribes the outer conductor: one TEM mode
    for c in conns:
        xs = (c["edge_x"] + (e - x_cut)) if c["mirror"] else (c["edge_x"] - (e - x_cut))
        ports.append({"num": c["id"], "type": "waveguide", "orientation": "xmax" if c["mirror"] else "xmin", "x": round(xs, 6),
                      "y": [-hp, hp], "z": [round(z_axis - hp, 6), round(z_axis + hp, 6)], "ref_shift": round(ref, 6), "what": what})
    if nconn == 1:
        wl = des["w50"]["0"] if not bench else des["launch"]["w"]
        if face2:
            p2 = case["ports"][1]
            ports.append({"num": 2, "type": "face", "x": p2["x"], "y": [p2["y0"], p2["y1"]], "z": [p2["z0"], p2["z1"]], "r": 50.0,
                          "what": "discrete face port 50 ohm from In1 to the line end (as kisi-fdtd); the board runs on 1.5 mm into the PML"})
        else:
            ports.append({"num": 2, "type": "waveguide", "orientation": "xmax", "x": round(feed, 6), "y": [-a.line_port_half, a.line_port_half],
                          "z": [round(z_ref, 6), round(z_top + a.line_port_height, 6)], "ref_shift": 0.0,
                          "what": f"waveguide port on the line cross-section (w {wl:g} mm on L1 over In1) at the end of the {feed:g} mm feed"})
    step = dict(sm)
    step["parts_map"] = part_map(sm, a.part)
    cav = []
    if a.cavity_z > 0:
        x0, x1 = sm["void_behind_bead"]["x"][0], sm["lip_opening"]["x"][0]
        for (u0, u1, r, _) in sm["contact"]:
            lo, hi = max(u0, x0), min(u1, x1)
            if hi - lo > 1e-6:
                cav.append([round(lo, 4), round(hi, 4), round(r * math.exp(a.cavity_z * math.sqrt(a.cavity_er) / 60.0), 4), r])
        # where the bore narrows at a contact shoulder (x 10.693: Ø1.016 -> Ø0.254, bore Ø2.34 -> Ø0.58) the shoulder face
        # would lie in the plane of the bore's step and touch it (a short): the wider bore runs on 0.1 mm past the shoulder
        for k in range(1, len(cav)):
            if cav[k][2] < cav[k - 1][3] + 0.05:
                u = round(min(cav[k][0] + 0.1, (cav[k][0] + cav[k][1]) / 2), 4)
                cav[k - 1][1], cav[k][0] = u, u
    step["cavity"] = {"sections": cav, "z": a.cavity_z, "er": a.cavity_er,
                      "note": "ASSUMED: the STEP has no bore between the bead and the front lip; a coaxial bore of this impedance (fill εr) is cut around each contact section"}
    step["cut_x"] = x_cut
    step["coax_port"] = a.coax_port
    step["connector"] = a.connector
    step["pin_contact"] = a.pin_contact
    step["clip_body"] = a.clip_body
    mats = {"bead": {"er": a.bead_er, "tand": a.bead_tand}, "metal": a.metal}
    name = a.name or (f"{a.model}_{a.variant}_{stk_key}" if not bench else f"samtec_bench_{nconn}c_{a.variant}")
    hb = a.clip_body if a.clip_body > 0 else sm["body_box"]["y"][1]
    zlo = min(z_axis - hb, z_bot - (sm["board_bottom_y"] - sm["plate"]["y"][0]))
    zhi = z_axis + hb
    model = {"name": name, "title": title, "date": datetime.date.today().isoformat(), "model": a.model, "variant": a.variant,
             "variant_over": over, "stackup": stk_key, "stackup_title": stk.d["title"], "fmax": a.fmax, "f_loss": a.fmax / 2, "loss": a.loss,
             "feed": feed, "launch_length": L, "board_x": [0.0, Lb], "board_y": [-hy, hy], "z_top": z_top, "z_bot": z_bot, "z_ref": z_ref,
             "z_axis": round(z_axis, 6), "z_span": [round(zlo, 6), round(zhi, 6)], "connectors": conns, "bricks": bricks, "sheets": sheets,
             "vias": vias, "ports": ports, "step": step, "materials": mats, "symmetry": a.symmetry, "air": a.air,
             "monitors": MONITOR_GHZ if a.monitors else [], "orient_check": not a.no_orient_check, "end_db": a.end_db,
             "edge_pullback": fp["edge_pullback"], "walls": bool(wall_vias), "launch_params": case["meta"]["params"],
             "notes": model_notes(a, sm, bench)}
    est = {k: estimate_cells(model, v["lpw"]) for k, v in MESH_PRESETS.items()}
    model["cells_estimate"] = est
    model["max_cells"] = a.max_cells
    preset = a.mesh
    if preset == "auto":
        preset = next((k for k in MESH_ORDER if est[k]["local"] <= a.max_cells * 0.85), "coarse")
    model["mesh"] = dict({"ratio": 20, "minstep": 0.0}, **MESH_PRESETS[preset], preset=preset)
    if a.lpw or a.lpb:
        model["mesh"].update(preset="custom", lpw=a.lpw or model["mesh"]["lpw"], lpb=a.lpb or model["mesh"]["lpb"])
        model["cells_estimate"]["custom"] = estimate_cells(model, model["mesh"]["lpw"])
    return model


def part_map(sm, overrides):
    """[(pattern, role)], checked in this order: drop, contact, body, plate (the body's name contains no contact name)"""
    pm = [(p["name"], p["role"]) for p in sm["parts"]]
    for o in overrides or []:
        role, _, pat = o.partition("=")
        if role not in ("body", "contact", "plate", "drop", "bead") or not pat:
            raise SystemExit(f"--part {o}: ROLE=PATTERN with ROLE body|contact|plate|drop|bead")
        pm = [x for x in pm if x[0] != pat] + [(pat, role)]
    order = {"drop": 0, "contact": 1, "bead": 2, "body": 3, "plate": 4}
    return sorted(pm, key=lambda x: order[x[1]])


def model_notes(a, sm, bench):
    n = ["Samtec's STEP model: the body solid also holds the support bead and is solid around the contact between the bead and the front lip "
         f"(x {sm['filled']['x'][0]}-{sm['filled']['x'][1]} mm): the bead is split off as a dielectric (εr {a.bead_er:g}, tanδ {a.bead_tand:g}); "
         + (f"a bore of {a.cavity_z:g} ohm (εr {a.cavity_er:g}) is ASSUMED there" if a.cavity_z > 0 else "NO bore cut: the contact is shorted to the body"),
         "the pin floats 0.038 mm above the board in the STEP: " + ("a PEC block from the copper to the pin axis stands for the pressed contact" if a.pin_contact else "no contact block (capacitive only)"),
         "copper: zero-thickness PEC sheets, widths widened by Wheeler's dw (kisi-fdtd convention); planes start 0.25 mm inside the board edge (zone inset), pads and tip 0.2 mm (edge pull-back)",
         "dielectric loss: " + ("constant conductivity, tanδ exact at fmax/2 (kisi-fdtd / CST.md convention)" if a.loss == "sigma" else "CST constant-tanδ model at fmax/2"),
         "connector metal: " + ("PEC" if a.metal == "pec" else "lossy metal: body / plate stainless 1.4e6 S/m, contact gold-plated 4.1e7 S/m")]
    if a.clip_body > 0:
        n.append(f"connector body clipped to ±{a.clip_body:g} mm about the axis (outside of the bushing, hex and jaw top only; --clip-body 0 keeps it whole)")
    if a.symmetry != "none":
        n.append("magnetic symmetry plane y = 0 (the structure is symmetric across the trace without the screws): half the cells")
    if bench:
        n.append("samtec_bench: ASSUMED stackup and line (connectors.json benchmark.cst_model), not Samtec's fixture")
    return n


# --- mesh estimate ---------------------------------------------------------------------------------------------
def _axis(lo, hi, dense, edges, dd, da, kappa):
    n, cov = 0.0, 0.0
    for a_, b_ in dense:
        a_, b_ = max(a_, lo), min(b_, hi)
        if b_ > a_:
            n += (b_ - a_) / dd
            cov += b_ - a_
    n += (hi - lo - cov) / da
    groups = []
    for e in sorted(x for x in edges if lo - 1e-9 <= x <= hi + 1e-9):
        if not groups or e - groups[-1] > dd:
            groups.append(e)
    return n + kappa * len(groups)


def estimate_cells(m, lpw):
    """rough CST hexahedral cell count: lines per wavelength in the board (εr max) inside the board's extent and
    in air elsewhere, plus kappa lines per group of geometric edges; C and kappa fitted to the 101 fine-mesh counts
    of CST.md (feat/cst-refs): 'local' (sd 9 %, max 36 %) assumes CST refines by the local material wavelength,
    'global' (sd 8.5 %) by the largest εr everywhere - an upper bound here, where much of the model is air"""
    sm = m["step"]
    er = max(b["er"] for b in m["bricks"])
    dd = C0 / m["fmax"] / math.sqrt(er) / lpw * 1e3
    da = C0 / m["fmax"] / lpw * 1e3
    xs, ys, zs = [], [], []
    for s in m["sheets"]:
        r = s["rect"]
        xs += [r[0], r[2]]
        ys += [r[1], r[3]]
        zs.append(s["z"])
        for rg in s.get("polys") or []:
            xs += [p[0] for p in rg]
            ys += [p[1] for p in rg]
    for v in m["vias"]:
        xs += [v["x"] - v["r"], v["x"] + v["r"]]
        ys += [v["y"] - v["r"], v["y"] + v["r"]]
    for b in m["bricks"]:
        zs += b["z"]
    za = m["z_axis"]
    radii = sorted({sm["oc_bore"]["r"], sm["lip_opening"]["r"], sm["bead"]["centre_hole_r"]} | {c[2] for c in sm["contact"]})
    cx = sorted({u for c in sm["contact"] for u in c[:2]} | set(sm["bead"]["x"]) | set(sm["lip_opening"]["x"]) | set(sm["plate"]["x"])
                | {sm["jaw"]["lip_x"], sm["jaw"]["slot_x"], sm["body_box"]["x"][1], sm["cut_x"]})
    for c in m["connectors"]:
        xs += [(c["dx"] - u) if c["mirror"] else (c["dx"] + u) for u in cx if u >= sm["cut_x"] - 1e-9]
    ys += [s * r for r in radii + [sm["jaw"]["underside_inner_half"], sm["plate"]["z_half"], sm["body_box"]["z_half"]] + sm["jaw"]["slot_half"] for s in (1, -1)]
    zs += [za + s * r for r in radii for s in (1, -1)] + [za + y for y in sm["jaw"]["top_y"]] + [m["z_bot"] - (sm["board_bottom_y"] - sm["plate"]["y"][0])] + m["z_span"]
    xlo = min(p["x"] for p in m["ports"] if p["type"] == "waveguide" and p.get("orientation") == "xmin")
    xhi = max([p["x"] for p in m["ports"] if p.get("orientation") == "xmax"] + [m["board_x"][1]])
    ylo, yhi = m["board_y"]
    if m["symmetry"] != "none":
        ylo = 0.0
    zlo, zhi = m["z_span"][0] - m["air"], m["z_span"][1] + m["air"]
    zb = (min(b["z"][0] for b in m["bricks"]), max(b["z"][1] for b in m["bricks"]))
    res = {}
    for key, (cc, kappa, glob) in {"local": (0.96, 3, False), "global": (0.57, 4, True)}.items():
        d_air = dd if glob else da
        nx = _axis(xlo, xhi, [tuple(m["board_x"])], xs, dd, d_air, kappa)
        ny = _axis(ylo, yhi, [tuple(m["board_y"])], ys, dd, d_air, kappa)
        nz = _axis(zlo, zhi, [zb], zs, dd, d_air, kappa)
        res[key] = int(cc * nx * ny * nz)
    return res


# --- VBA ---------------------------------------------------------------------------------------------------------
def material_block(name, er, tand, m, colour="0.2\", \"0.6\", \"0.2"):
    if m["loss"] == "tand" and tand > 0:
        loss = (f' .Sigma "0"\n .TanDGiven "True"\n .TanD {q(tand)}\n .TanDFreq {q(m["f_loss"] / 1e9)}\n .TanDModel "ConstTanD"\n')
    else:
        loss = f' .Sigma {q(sigma(er, tand, m["f_loss"]))}\n .TanDGiven "False"\n .TanD "0.0"\n'
    return (f'With Material\n .Reset\n .Name "{name}"\n .Folder ""\n .Type "Normal"\n .Epsilon {q(er)}\n .Mu "1.0"\n{loss}'
            f' .SigmaM "0"\n .Colour "{colour}"\n .Transparency "60"\n .Create\nEnd With\n')


def lossy_metal(name, s):
    return (f'With Material\n .Reset\n .Name "{name}"\n .Folder ""\n .FrqType "all"\n .Type "Lossy metal"\n .Mu "1.0"\n'
            f' .Sigma {q(s)}\n .Colour "0.7", "0.7", "0.75"\n .Create\nEnd With\n')


def transform(name_expr, kind, args):
    """Transform of one shape; name_expr is a VBA string expression (a variable or a quoted name)"""
    if kind == "Rotate":
        geo = f' .Origin "Free"\n .Center "0", "0", "0"\n .Angle {q(args[0])}, {q(args[1])}, {q(args[2])}\n'
    else:
        geo = f' .Vector {q(args[0])}, {q(args[1])}, {q(args[2])}\n .UsePickedPoints "False"\n .InvertPickedPoints "False"\n'
    return (f'With Transform\n .Reset\n .Name {name_expr}\n{geo} .MultipleObjects "False"\n .GroupObjects "False"\n .Repetitions "1"\n'
            f' .MultipleSelection "False"\n .Transform "Shape", "{kind}"\nEnd With\n')


def connector_vba(m, c):
    """one history block: STEP import, part classification (printed to CST's message window), the bead split off as
    a dielectric, the assumed bore, the cut at the coax port (or the plug extension), the pin's contact block, the
    materials and the placement (rotation x 90 deg: STEP y (up) -> model z; mirrored for the far connector)"""
    sm = m["step"]
    k = c["id"]
    comp = f"conn{k}"
    tag = f"connector {k}"
    mt = m["materials"]["metal"]
    mat_body, mat_cont = ("PEC", "PEC") if mt == "pec" else ("stainless", "gold")
    o = []
    A = o.append
    A(f"' {tag}: Samtec STEP model, parts by name (cst_launch.py --part ROLE=PATTERN overrides)")
    A("Dim i As Long, k As Long, nn As Long, ns As Long, nm As String, before As String, others As String")
    A("Dim sBody As String, sCont As String, sPlate As String, sBead As String")
    A("Dim xa As Double, xb As Double, ya As Double, yb As Double, za As Double, zb As Double")
    A("Dim names(200) As String, shp(200) As String")
    A(f'Component.New "{comp}"')
    A('before = "|"')
    A("For i = 0 To Solid.GetNumberOfShapes - 1")
    A(' before = before & Solid.GetNameOfShapeFromIndex(i) & "|"')
    A("Next i")
    A(f'With STEP\n .Reset\n .FileName "{STEP_TOKEN}"\n .Id "{100 + k}"\n .ScaleToUnit "True"\n .ImportAttributes "True"\n .ImportCurves "False"\n .Read\nEnd With')
    A("nn = 0")
    A("For i = 0 To Solid.GetNumberOfShapes - 1")
    A(" nm = Solid.GetNameOfShapeFromIndex(i)")
    A(' If InStr(before, "|" & nm & "|") = 0 Then')
    A("  names(nn) = nm")
    A("  nn = nn + 1")
    A(" End If")
    A("Next i")
    sc = float(sm.get("step_scale", 1.0) or 1.0)
    if sc != 1.0:
        A(f"' the STEP file is in inches; scale the imported solids by {sc:g} about the origin (cst_launch.py --step-scale)")
        A("For k = 0 To nn - 1")
        A(f'With Transform\n .Reset\n .Name names(k)\n .Origin "Free"\n .Center "0", "0", "0"\n .ScaleFactor {q(sc)}, {q(sc)}, {q(sc)}\n'
          f' .MultipleObjects "False"\n .GroupObjects "False"\n .Repetitions "1"\n .MultipleSelection "False"\n .Transform "Shape", "Scale"\nEnd With')
        A("Next k")
    A('sBody = ""\nsCont = ""\nsPlate = ""\nsBead = ""\nothers = ""')
    A("For k = 0 To nn - 1")
    A(" nm = names(k)")
    for j, (pat, role) in enumerate(sm["parts_map"]):
        A(f' {"If" if j == 0 else "ElseIf"} InStr(nm, "{pat}") > 0 Then')
        if role == "drop":
            A("  Solid.Delete nm")
            A(f'  ReportInformationToWindow "{tag}: dropped " & nm')
        else:
            var = {"body": "sBody", "contact": "sCont", "plate": "sPlate", "bead": "sBead"}[role]
            A(f'  If {var} <> "" Then ReportError "{tag}: two STEP solids match {role} ({pat}): " & {var} & ", " & nm')
            A(f"  {var} = nm")
            A(f'  ReportInformationToWindow "{tag}: {role} = " & nm')
    A(" Else")
    A('  others = others & nm & "|"')
    A(f'  ReportWarningToWindow "{tag}: STEP solid " & nm & " matches no part pattern: kept as body metal"')
    A(" End If")
    A("Next k")
    A(f'If sBody = "" Or sCont = "" Then ReportError "{tag}: body or contact not found in the STEP (names: see the message window; --part ROLE=PATTERN)"')
    b = sm["bead"]
    A(f"' the support bead is part of the body solid: bead = body inside the bead cylinder (x {b['x'][0]}-{b['x'][1]}, r {b['r']})")
    A("Dim found As Boolean")
    A("' where the body lies (diagnostic: the bead cylinder below assumes the coax axis at y = z = 0)")
    A("On Error Resume Next")
    A("Solid.GetLooseBoundingBoxOfShape sBody, xa, xb, ya, yb, za, zb")
    A(f'ReportInformationToWindow "{tag}: body box x " & xa & " .. " & xb & ", y " & ya & " .. " & yb & ", z " & za & " .. " & zb')
    A("On Error GoTo 0")
    A('If sBead = "" Then')
    A(cyl_x("beadA", comp, "Vacuum", b["x"][0] - 0.05, b["x"][1], b["r"]).rstrip())
    A(f'Solid.Insert "{comp}:beadA", sBody')
    A(cyl_x("bead", comp, "bead", b["x"][0] - 0.05, b["x"][1], b["r"]).rstrip())
    A(f'Solid.Subtract "{comp}:bead", "{comp}:beadA"')
    A("' an empty result (the body does not overlap the cylinder as expected) deletes the shape: then a plain disc")
    A("found = False")
    A("For i = 0 To Solid.GetNumberOfShapes - 1")
    A(f' If Solid.GetNameOfShapeFromIndex(i) = "{comp}:bead" Then found = True')
    A("Next i")
    A("If Not found Then")
    A(f'  ReportWarningToWindow "{tag}: splitting the bead off the body gave nothing (the body is not where the STEP '
      f'measurement expects it, see the body box above): a plain disc without its six holes is used instead"')
    A(cyl_x("bead", comp, "bead", b["x"][0], b["x"][1], b["r"], b.get("centre_hole_r", 0.0)).rstrip())
    A("End If")
    A(f'Solid.Insert sBody, "{comp}:bead"')
    A(f'sBead = "{comp}:bead"')
    A("Else")
    A('Solid.ChangeMaterial sBead, "bead"')
    A("End If")
    A("Solid.Insert sBead, sCont")
    cav = sm["cavity"]
    if cav["sections"]:
        A(f"' ASSUMED bore between the bead and the front lip ({cav['z']:g} ohm, fill er {cav['er']:g}): the STEP is solid there")
        fill = "Vacuum" if cav["er"] <= 1.0 else "cavity_fill"
        for j, (x0, x1, r, _) in enumerate(cav["sections"]):
            A(cyl_x(f"cav{j + 1}", comp, fill, x0, x1, r).rstrip())
            if j:
                A(f'Solid.Add "{comp}:cav1", "{comp}:cav{j + 1}"')
        if fill == "Vacuum":
            A(f'Solid.Subtract sBody, "{comp}:cav1"')
        else:
            A(f'Solid.Insert sBody, "{comp}:cav1"')
            A(f'Solid.Insert "{comp}:cav1", sCont')
    if sm["coax_port"] == "inner":
        x = sm["cut_x"]
        A(f"' cut at the coax port plane (STEP x = {x:g}): the thread, the socket and the mating plane lie ahead of it")
        A(brick("cutA", comp, "Vacuum", -2.0, x, -8, 8, -8, 8).rstrip())
        A(f'Solid.Subtract sBody, "{comp}:cutA"')
        A(brick("cutB", comp, "Vacuum", -2.0, x, -8, 8, -8, 8).rstrip())
        A(f'Solid.Subtract sCont, "{comp}:cutB"')
        A("' the socket's bore (D0.95, to x = 5.04) is open at the port plane: the contact would be a tube there and the port")
        A("' would see a second, decoupled region inside it; fill it (as the mated pin does)")
        A(cyl_x("sockfill", comp, mat_cont, x - 0.01, 5.04, 0.4765).rstrip())
        A(f'Solid.Add sCont, "{comp}:sockfill"')
    else:
        x0, xm = sm["cut_x"], sm["mating_plane_x"]
        A("' the mating plug as an ideal air coax: outer conductor in the counterbore, inner conductor to the socket, pin in the socket bore")
        A(cyl_x("ext_outer", comp, mat_body, x0, xm, sm["counterbore"]["r"], sm["oc_bore"]["r"]).rstrip())
        A(f'Solid.Add sBody, "{comp}:ext_outer"')
        A(cyl_x("ext_inner", comp, mat_cont, x0, sm["contact"][0][0] + 0.01, 0.635).rstrip())
        A(f'Solid.Add sCont, "{comp}:ext_inner"')
        A(cyl_x("ext_pin", comp, mat_cont, xm, sm["contact"][0][1], 0.476).rstrip())
        A(f'Solid.Add sCont, "{comp}:ext_pin"')
    if sm.get("clip_body", 0) > 0:
        h = sm["clip_body"]
        A(f"' body clipped to {h:g} mm above / below the axis (its outside only: bushing, hex, jaw top) to save mesh cells")
        A(brick("clipA", comp, "Vacuum", -2.0, 20.0, h, 8, -8, 8).rstrip())
        A(f'Solid.Subtract sBody, "{comp}:clipA"')
        A(brick("clipB", comp, "Vacuum", -2.0, 20.0, -8, -h, -8, 8).rstrip())
        A(f'Solid.Subtract sBody, "{comp}:clipB"')
    if sm["pin_contact"]:
        pin = sm["contact"][-1]
        e = sm["board_edge_x"]
        A("' the pin floats 0.038 mm over the board in the STEP; this block stands for the pressed contact on the pad")
        A(brick("pinblk", comp, mat_cont, e + m["edge_pullback"], pin[1], sm["board_top_y"], 0.0, -pin[2], pin[2]).rstrip())
        A(f'Solid.Add sCont, "{comp}:pinblk"')
    A(f'Solid.ChangeMaterial sBody, "{mat_body}"')
    A(f'Solid.ChangeMaterial sCont, "{mat_cont}"')
    A(f'If sPlate <> "" Then Solid.ChangeMaterial sPlate, "{mat_body}"')
    A("' placement: all parts rotated about x (STEP y up -> model z), the plate last")
    A("ns = 0")
    shapes = ["sBody", "sCont", "sBead"] + (["\"%s:cav1\"" % comp] if cav["sections"] and cav["er"] > 1.0 else [])
    for s_ in shapes:
        A(f"shp(ns) = {s_}\nns = ns + 1")
    A('If others <> "" Then')
    A(' Dim oth() As String')
    A(' oth = Split(others, "|")')
    A(" For i = 0 To UBound(oth)")
    A('  If oth(i) <> "" Then')
    A(f'   Solid.ChangeMaterial oth(i), "{mat_body}"')
    A("   shp(ns) = oth(i)")
    A("   ns = ns + 1")
    A("  End If")
    A(" Next i")
    A("End If")
    A('If sPlate <> "" Then\n shp(ns) = sPlate\n ns = ns + 1\nEnd If')
    A("For i = 0 To ns - 1")
    A(transform("shp(i)", "Rotate", (90, 0, 0)).rstrip())
    A("Next i")
    if m["orient_check"]:
        A("' check: the lower plate must lie below the axis (else the rotation sense differs: turn by 180 deg)")
        A('If sPlate <> "" Then')
        A(" Solid.GetLooseBoundingBoxOfShape sPlate, xa, xb, ya, yb, za, zb")
        A(" If za + zb > 0 Then")
        A(f'  ReportWarningToWindow "{tag}: plate above the axis after the rotation: turned by 180 deg about x"')
        A("  For i = 0 To ns - 1")
        A(transform("shp(i)", "Rotate", (180, 0, 0)).rstrip())
        A("  Next i")
        A(" End If")
        A("End If")
    if c["mirror"]:
        A("For i = 0 To ns - 1")
        A(transform("shp(i)", "Rotate", (0, 0, 180)).rstrip())
        A("Next i")
    A("For i = 0 To ns - 1")
    A(" If shp(i) = sPlate Then")
    A(transform("shp(i)", "Translate", (c["dx"], 0, c["dz_plate"])).rstrip())
    A(" Else")
    A(transform("shp(i)", "Translate", (c["dx"], 0, c["dz_main"])).rstrip())
    A(" End If")
    A("Next i")
    return "\n".join(o)


def connector_ops(m, c, grow=0.0):
    """the connector from primitives in model coordinates (no import, no rotation: replay-safe), dimensions of
    connectors.json step_model (measured from Samtec's STEP): [(op, ...)] with op brick | cylx | cylz (create: name,
    material, geometry) and add | subtract | insert (a, b). STEP x (axis) -> model x from the board edge, STEP y (up) ->
    model z (axis at z_axis), STEP z (across) -> model -y. The outside of the body is a box clipped to clip_body about
    the axis (bushing, hex, thread are outside the coax and the launch: not modelled)"""
    sm = m["step"]
    comp = f"conn{c['id']}"
    sg = -1.0 if c["mirror"] else 1.0
    e, xe = sm["board_edge_x"], c["edge_x"]
    za = m["z_axis"]
    X = lambda u: round(xe + sg * (u - e), 6)
    Zm = lambda y: round(za + y, 6)
    Zp = lambda y: round(m["z_bot"] + (y - sm["board_bottom_y"]), 6)
    mt = m["materials"]["metal"]
    mat_body, mat_cont = ("PEC", "PEC") if mt == "pec" else ("stainless", "gold")
    ops = []
    n = [0]

    def nm(base):
        n[0] += 1
        return f"{comp}:{base}{n[0]}"

    def box(name, mat, u0, u1, y0, y1, z0, z1, plate=False):
        x0, x1 = sorted((X(u0), X(u1)))
        Z = Zp if plate else Zm
        ops.append(("brick", name, mat, x0, x1, round(-z1, 6), round(-z0, 6), Z(y0), Z(y1)))
        return name

    def cylx(name, mat, u0, u1, r, yc=0.0, zc=0.0):
        x0, x1 = sorted((X(u0), X(u1)))
        ops.append(("cylx", name, mat, x0, x1, round(-zc, 6), Zm(yc), r))
        return name

    def cylv(name, mat, u, zc, r, y0, y1):  # along STEP y = model z
        ops.append(("cylz", name, mat, X(u), round(-zc, 6), r, Zm(y0), Zm(y1)))
        return name

    def cut(a, b):
        ops.append(("subtract", a, b))

    def add(a, b):
        ops.append(("add", a, b))

    up = sm["cut_x"]                 # the coax port plane
    h = sm.get("clip_body") or 3.0
    jw = sm["body_box"]["z_half"]
    ty = sm["board_top_y"]
    jaw = sm["jaw"]
    lip0 = jaw["lip_x"]
    jtop = jaw["top_y"][0]
    ob, bd, lo = sm["oc_bore"], sm["bead"], sm["lip_opening"]
    body = f"{comp}:body"
    # body: round (Ø6.35 thread bushing ahead of the hex, then Ø9.53) and cut flat above / below: the front, the tongue that
    # meets the board edge, the jaw over the board (STEP: the body's sides are the Ø9.53 cylinder)
    def round_part(name, u0, u1, r, y0, y1):
        cylx(name, mat_body, u0, u1, r)
        cut(name, box(nm("flat"), "Vacuum", u0 - 0.1, u1 + 0.1, y1, r + 1, -r - 1, r + 1))
        cut(name, box(nm("flat"), "Vacuum", u0 - 0.1, u1 + 0.1, -r - 1, y0, -r - 1, r + 1))
        return name
    hexu = sm.get("hex_x", 5.6301)
    round_part(body, up, hexu, sm.get("bushing_r", 3.175), -h, h)
    add(body, round_part(nm("hex"), hexu, lip0, jw, -h, h))
    add(body, box(nm("tongue"), mat_body, lip0, e, -h, ty, -1.27, 1.27))
    add(body, round_part(nm("front"), lip0, e, jw, ty, h))
    add(body, round_part(nm("jaw"), e, sm["body_box"]["x"][1], jw, ty, jtop))
    # inside: the air line, the bead seat, the (ASSUMED) bore, the lip opening for the pin
    cut(body, cylx(nm("bore"), "Vacuum", up - 0.1, bd["x"][0], ob["r"]))
    cut(body, cylx(nm("seat"), "Vacuum", bd["x"][0], bd["x"][1], bd["r"]))
    cav = sm["cavity"]
    fill = cav["sections"] and cav["er"] > 1.0
    fills = []
    for (u0, u1, r, _) in cav["sections"]:
        if fill:
            fills.append(cylx(nm("fill"), "cavity_fill", u0, u1, r))
        else:
            cut(body, cylx(nm("cav"), "Vacuum", u0, u1, r))
    if not cav["sections"]:  # the STEP's void behind the bead only (the contact is then shorted to the body)
        cut(body, cylx(nm("void"), "Vacuum", *sm["void_behind_bead"]["x"], sm["void_behind_bead"]["r"]))
    cut(body, cylx(nm("lip"), "Vacuum", lo["x"][0] - 0.01, lo["x"][1] + 0.05, lo["r"]))
    # the slot over the trace (open to the top), the screw slots
    sx = jaw["slot_x"]
    s1 = 12.167  # STEP: the slot widens from 2.67 to 3.18 mm here
    cut(body, box(nm("slot"), "Vacuum", sx, s1, ty - 0.01, jtop + 0.01, -jaw["slot_half"][0], jaw["slot_half"][0]))
    cut(body, box(nm("slot"), "Vacuum", s1, sm["body_box"]["x"][1] + 0.1, ty - 0.01, jtop + 0.01, -jaw["slot_half"][1], jaw["slot_half"][1]))
    for zs in (1, -1):
        zc = zs * 2.921
        cut(body, box(nm("screw"), "Vacuum", 13.208, 15.24, ty - 0.01, jtop + 0.01, zc - 1.016, zc + 1.016))
        for u in (13.208, 15.24):
            cut(body, cylv(nm("screw"), "Vacuum", u, zc, 1.016, ty - 0.01, jtop + 0.01))
    # lower plate (with its groove under the trace)
    pl = sm["plate"]
    plate = f"{comp}:plate"
    box(plate, mat_body, pl["x"][0], pl["x"][1], pl["y"][0], pl["y"][1], -pl["z_half"], pl["z_half"], plate=True)
    cut(plate, box(nm("groove"), "Vacuum", pl["x"][0] - 0.1, pl["x"][1] + 0.1, pl["y"][1] - 0.0762, pl["y"][1] + 0.01, -0.254, 0.254, plate=True))
    # centre contact from the port plane to the pin end (the socket lies ahead of the port plane)
    cont = f"{comp}:contact"
    first = True
    for (u0, u1, r, _) in sm["contact"]:
        if u1 <= up + 1e-9:
            continue
        if first:
            cylx(cont, mat_cont, max(u0, up), u1 + grow, r + grow)
            first = False
        else:
            add(cont, cylx(nm("ct"), mat_cont, u0, u1 + grow, r + grow))
    if sm["pin_contact"]:
        pin = sm["contact"][-1]
        add(cont, box(nm("pinblk"), mat_cont, e + m["edge_pullback"], pin[1] + grow, ty + grow, 0.0, -pin[2] - grow, pin[2] + grow))
    # the support bead: disc with six holes and the centre hole, minus the contact's shoulder
    bead = f"{comp}:bead"
    cylx(bead, "bead", bd["x"][0], bd["x"][1], bd["r"])
    hl = bd["holes"]
    for k in range(hl["n"]):
        t = 2 * math.pi * k / hl["n"]
        cut(bead, cylx(nm("hole"), "Vacuum", bd["x"][0] - 0.01, bd["x"][1] + 0.01, hl["r"], hl["pitch_r"] * math.cos(t), hl["pitch_r"] * math.sin(t)))
    cut(bead, cylx(nm("hole"), "Vacuum", bd["x"][0] - 0.01, bd["x"][1] + 0.01, bd["centre_hole_r"]))
    ops.append(("insert", bead, cont))
    if fill:
        for f_ in fills[1:]:
            add(fills[0], f_)
        ops.append(("insert", body, fills[0]))
        ops.append(("insert", fills[0], cont))
    return ops


def ops_vba(ops):
    o = ["' connector from primitives (cst_launch.py connector_ops): dimensions measured from Samtec's STEP, placed directly"]
    for op in ops:
        k = op[0]
        if k in ("brick", "cylx", "cylz"):
            comp, name = op[1].split(":")
            if k == "brick":
                o.append(brick(name, comp, op[2], *op[3:]).rstrip())
            elif k == "cylx":
                x0, x1, yc, zc, r = op[3:]
                o.append(f'With Cylinder\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{op[2]}"\n .OuterRadius {q(r)}\n'
                         f' .InnerRadius "0"\n .Axis "x"\n .Xrange {q(x0)}, {q(x1)}\n .Ycenter {q(yc)}\n .Zcenter {q(zc)}\n .Segments "0"\n .Create\nEnd With')
            else:
                x, y, r, z0, z1 = op[3:]
                o.append(cyl_z(name, comp, op[2], x, y, r, z0, z1).rstrip())
        else:
            o.append(f'Solid.{ {"add": "Add", "subtract": "Subtract", "insert": "Insert"}[k] } "{op[1]}", "{op[2]}"')
    return "\n".join(o)


def ops_occ(ops, occ):
    """the same operations with OpenCASCADE (gmsh): {name: [tags]} (checks without CST)"""
    tags = {}
    for op in ops:
        k = op[0]
        if k == "brick":
            x0, x1, y0, y1, z0, z1 = op[3:]
            tags[op[1]] = [occ.addBox(x0, y0, z0, x1 - x0, y1 - y0, z1 - z0)]
        elif k == "cylx":
            x0, x1, yc, zc, r = op[3:]
            tags[op[1]] = [occ.addCylinder(x0, yc, zc, x1 - x0, 0, 0, r)]
        elif k == "cylz":
            x, y, r, z0, z1 = op[3:]
            tags[op[1]] = [occ.addCylinder(x, y, z0, 0, 0, z1 - z0, r)]
        else:
            a, b = [(3, t) for t in tags[op[1]]], [(3, t) for t in tags[op[2]]]
            if k == "add":
                res, _ = occ.fuse(a, b)
            else:
                res, _ = occ.cut(a, b, removeObject=True, removeTool=(k == "subtract"))
            tags[op[1]] = [t for d, t in res if d == 3]
            if k == "subtract":
                tags.pop(op[2])
    return tags


def port_conductor_check(m):
    """history block: the conductors at every coax port plane must be the contact (centred, its radius) inside the
    body (outer conductor); a misplaced connector (e.g. a STEP placed differently on replay) stops the build here"""
    sm = m["step"]
    o = ["' the coax ports: contact centred in the outer conductor at the port plane (else: stop)", "Dim xa As Double, xb As Double, ya As Double, yb As Double, za As Double, zb As Double", "Dim bad As String"]
    par = sm.get("connector", "step") == "parametric"
    r0 = next(r for (u0, u1, r, _) in sm["contact"] if u0 <= sm["cut_x"] < u1)
    # every waveguide port must lie on the structure's bounding box (CST's domain boundary): a part beyond it moves the
    # boundary and leaves the port inside the model
    o += ["Dim i As Long, gx0 As Double, gx1 As Double", "gx0 = 1e30", "gx1 = -1e30",
          "For i = 0 To Solid.GetNumberOfShapes - 1",
          " Solid.GetLooseBoundingBoxOfShape Solid.GetNameOfShapeFromIndex(i), xa, xb, ya, yb, za, zb",
          " If xa < gx0 Then gx0 = xa", " If xb > gx1 Then gx1 = xb", "Next i",
          'ReportInformationToWindow "model x extent " & gx0 & " .. " & gx1']
    for p in [p for p in m["ports"] if p["type"] == "waveguide"]:
        g = "gx0" if p["orientation"] == "xmin" else "gx1"
        o.append(f'If Abs({g} - ({p["x"]:.6f})) > 0.02 Then ReportError "port {p["num"]} (x = {p["x"]:.4f}) is not on the model boundary (" & {g} & "): a part reaches beyond it"')
    for p in [p for p in m["ports"] if p["type"] == "waveguide" and abs(p["ref_shift"]) > 0]:
        k = p["num"]
        za_ = m["z_axis"]
        if par:
            o.append(f'Solid.GetLooseBoundingBoxOfShape "conn{k}:contact", xa, xb, ya, yb, za, zb')
            o.append(f'ReportInformationToWindow "port {k}: contact box x " & xa & " .. " & xb & ", y " & ya & " .. " & yb & ", z " & za & " .. " & zb')
            o.append('bad = ""')
            xe = "xa" if p["orientation"] == "xmin" else "xb"
            o.append(f'If Abs({xe} - ({p["x"]:.6f})) > 0.01 Then bad = bad & " contact does not reach the port plane;"')
            o.append(f'If Abs((ya + yb) / 2) > 0.01 Or Abs((za + zb) / 2 - ({za_:.6f})) > 0.08 Then bad = bad & " contact not on the axis;"')
            o.append(f'If yb - ya > {2 * r0 + 0.1:.4f} Then bad = bad & " contact wider than its {2 * r0:.3f} mm;"')
            o.append(f'Solid.GetLooseBoundingBoxOfShape "conn{k}:body", xa, xb, ya, yb, za, zb')
            o.append(f'ReportInformationToWindow "port {k}: outer conductor (body) box x " & xa & " .. " & xb & ", z " & za & " .. " & zb')
            o.append(f'If Abs({xe} - ({p["x"]:.6f})) > 0.01 Then bad = bad & " outer conductor does not reach the port plane;"')
            o.append(f'If za > {za_ - sm["oc_bore"]["r"]:.6f} Or zb < {za_ + sm["oc_bore"]["r"]:.6f} Then bad = bad & " outer conductor does not enclose the bore;"')
            o.append(f'If bad <> "" Then ReportError "port {k}:" & bad & " (expected: contact D{2 * r0:.3f} on the axis z = {za_:.4f} in the D{2 * sm["oc_bore"]["r"]:.3f} bore at x = {p["x"]:.4f})"')
        o.append(f'ReportInformationToWindow "port {k}: square {2 * (p["y"][1]):.4f} mm at x = {p["x"]:.4f}, centre z = {(p["z"][0] + p["z"][1]) / 2:.4f} (axis {za_:.4f}): one TEM mode expected, Z = 60 ln(D/d) = {60 * math.log(sm["oc_bore"]["r"] / r0):.2f} ohm"')
    return "\n".join(o)


def vba_blocks(m):
    B = Blocks()
    mesh = m["mesh"]
    B.add("units", 'With Units\n .Geometry "mm"\n .Frequency "GHz"\n .Time "ns"\n .TemperatureUnit "Kelvin"\nEnd With', "setup")
    air = m["air"]
    B.add("background", f"""
With Background
 .ResetBackground
 .Type "Normal"
 .Epsilon "1.0"
 .Mu "1.0"
 .XminSpace "0"
 .XmaxSpace "0"
 .YminSpace "0"
 .YmaxSpace "0"
 .ZminSpace {q(air)}
 .ZmaxSpace {q(air)}
 .ApplyInAllDirections "False"
End With""", "setup")
    sym = {"none": "none", "magnetic": "magnetic"}[m["symmetry"]]
    B.add("boundaries", f"""
With Boundary
 .Xmin "open"
 .Xmax "open"
 .Ymin "open"
 .Ymax "open"
 .Zmin "open"
 .Zmax "open"
 .Xsymmetry "none"
 .Ysymmetry "{sym}"
 .Zsymmetry "none"
 .ApplyInAllDirections "False"
End With""", "setup")
    B.add("frequency", f'Solver.FrequencyRange "0", {q(m["fmax"] / 1e9)}', "setup")
    # materials
    mats = {}
    for b in m["bricks"]:
        key = (b["er"], b["tand"])
        if key not in mats:
            mats[key] = f"diel_er{b['er']:g}_tand{b['tand']:g}".replace(".", "p")
            B.add(f"material {mats[key]}", material_block(mats[key], b["er"], b["tand"], m))
    bm = m["materials"]["bead"]
    B.add("material bead", material_block("bead", bm["er"], bm["tand"], m, "0.9\", \"0.9\", \"0.9"))
    cav = m["step"]["cavity"]
    if cav["sections"] and cav["er"] > 1.0:
        B.add("material cavity_fill", material_block("cavity_fill", cav["er"], bm["tand"], m, "0.9\", \"0.8\", \"0.6"))
    if m["materials"]["metal"] == "lossy":
        B.add("material stainless", lossy_metal("stainless", 1.4e6))
        B.add("material gold", lossy_metal("gold", 4.1e7))
    for b in m["bricks"]:
        B.add(b["name"], brick(b["name"], "board", mats[(b["er"], b["tand"])], b["x"][0], b["x"][1], b["y"][0], b["y"][1], b["z"][0], b["z"][1]))
    # face port patch (launch1 --port2 face): a piece of the plane under the port split off as its own sheet
    patches = {}
    for p in m["ports"]:
        if p["type"] != "face":
            continue
        for si, s in enumerate(m["sheets"]):
            rr = [p["x"], p["y"][0], p["x"] + 0.05, p["y"][1]]
            if abs(s["z"] - p["z"][0]) < 1e-9 and all(copper_at(s, x, y) for x, y in ((rr[0], rr[1]), (rr[2], rr[3]), (rr[0], rr[3]), (rr[2], rr[1]))):
                patches[si] = (p["num"], rr)
                p["patch_sheet"] = si
                break
        else:
            raise SystemExit("face port: no plane under the port end")
    for si, s in enumerate(m["sheets"]):
        nm = f"sheet{si}"
        rings = [cr for cr in (clean_ring(rg) for rg in s.get("polys") or []) if cr]
        if rings:
            ips = [(sum(p[0] for p in rg) / len(rg), sum(p[1] for p in rg) / len(rg)) for rg in rings]
            depth = [sum(inside_ring(ip[0], ip[1], o) for o in rings if o is not rg) for ip, rg in zip(ips, rings)]
            first = True
            for d in sorted(set(depth)):
                for rg, dd_ in zip(rings, depth):
                    if dd_ != d:
                        continue
                    part = B.name(f"{nm}_r")
                    code = B.poly_sheet("metal", nm if first else part, rg, s["z"])
                    if not first:
                        code += (f'Solid.Subtract "metal:{nm}", "metal:{part}"\n' if d % 2 else f'Solid.Add "metal:{nm}", "metal:{part}"\n')
                    first = False
                    B.add(f"{nm} ring", code)
            if first:
                continue
        else:
            r = s["rect"]
            B.add(nm, B.rect_sheet("metal", nm, r[0], r[1], r[2], r[3], s["z"]))
        for h in s.get("holes") or []:
            hn = B.name(f"{nm}_h")
            B.add(f"{nm} hole", B.circle_sheet("metal", hn, h[0], h[1], h[2], s["z"]) + f'Solid.Subtract "metal:{nm}", "metal:{hn}"\n')
        if si in patches:
            k, rr = patches[si]
            cut = B.name(f"{nm}_p")
            B.add(f"{nm} port patch {k}", B.rect_sheet("metal", cut, *rr, s["z"]) + f'Solid.Subtract "metal:{nm}", "metal:{cut}"\n'
                  + B.rect_sheet("metal", f"portref{k}", *rr, s["z"]))
    for vi, v in enumerate(m["vias"]):
        vn = f"via{vi}"
        B.add(vn, cyl_z(vn, "metal", "PEC", v["x"], v["y"], v["r"], v["z0"], v["z1"]))
        ins = "".join(f'Solid.Insert "board:{b["name"]}", "metal:{vn}"\n' for b in m["bricks"]
                      if b["z"][0] < v["z1"] - 1e-9 and b["z"][1] > v["z0"] + 1e-9 and b["x"][0] - v["r"] < v["x"] < b["x"][1] + v["r"]
                      and b["y"][0] - v["r"] < v["y"] < b["y"][1] + v["r"])
        if ins:
            B.add(f"{vn} insert", ins)
        for pz in v.get("pads") or []:
            pn = B.name(f"{vn}_pad")
            B.add(f"{vn} pad", B.circle_sheet("metal", pn, v["x"], v["y"], v["pad_r"], pz))
    for c in m["connectors"]:
        if m["step"].get("connector", "step") == "parametric":
            B.add(f"connector {c['id']} (parametric)", ops_vba(connector_ops(m, c)), "connector")
        else:
            B.add(f"connector {c['id']} (Samtec STEP)", connector_vba(m, c), "connector")
    B.add("check: conductors at the coax ports", port_conductor_check(m), "check")
    for p in m["ports"]:
        if p["type"] == "waveguide":
            B.add(f"port {p['num']} (waveguide)", f"""
With Port
 .Reset
 .PortNumber "{p['num']}"
 .Label ""
 .Folder ""
 .NumberOfModes "1"
 .AdjustPolarization "False"
 .PolarizationAngle "0.0"
 .ReferencePlaneDistance {q(p['ref_shift'])}
 .TextSize "50"
 .TextMaxLimit "0"
 .Coordinates "Free"
 .Orientation "{p['orientation']}"
 .PortOnBound "True"
 .ClipPickedPortToBound "False"
 .Xrange {q(p['x'])}, {q(p['x'])}
 .Yrange {q(p['y'][0])}, {q(p['y'][1])}
 .Zrange {q(p['z'][0])}, {q(p['z'][1])}
 .XrangeAdd "0.0", "0.0"
 .YrangeAdd "0.0", "0.0"
 .ZrangeAdd "0.0", "0.0"
 .SingleEnded "False"
 .WaveguideMonitor "False"
 .Create
End With""", "port")
        else:
            xm, ym = p["x"], (p["y"][0] + p["y"][1]) / 2
            tr = next(si for si, s in enumerate(m["sheets"]) if abs(s["z"] - p["z"][1]) < 1e-9 and copper_at(s, xm - 0.02, ym) and not copper_at(s, xm + 0.02, ym))
            B.add(f"port {p['num']} (face)", f"""
Pick.PickEdgeFromPoint "metal:sheet{tr}", {q(xm)}, {q(ym)}, {q(p['z'][1])}
Pick.PickEdgeFromPoint "metal:portref{p['num']}", {q(xm)}, {q(ym)}, {q(p['z'][0])}
With DiscreteFacePort
 .Reset
 .PortNumber "{p['num']}"
 .Type "SParameter"
 .Label ""
 .Folder ""
 .Impedance {q(p['r'])}
 .VoltageAmplitude "1.0"
 .CurrentAmplitude "1.0"
 .Monitor "True"
 .CenterEdge "True"
 .SetP1 "True", {q(xm)}, {q(ym)}, {q(p['z'][0])}
 .SetP2 "True", {q(xm)}, {q(ym)}, {q(p['z'][1])}
 .LocalCoordinates "False"
 .InvertDirection "False"
 .UseProjection "False"
 .ReverseProjection "False"
 .Create
End With""", "port")
    for f in m["monitors"]:
        B.add(f"monitor e-field {f} GHz", f"""
With Monitor
 .Reset
 .Name "e-field (f={f})"
 .Dimension "Volume"
 .Domain "Frequency"
 .FieldType "Efield"
 .MonitorValue {q(f)}
 .UseSubvolume "False"
 .Create
End With""", "setup")
    B.add("mesh", mesh_vba(mesh), "mesh")
    B.add("solver", f"""
With Solver
 .Method "Hexahedral"
 .CalculationType "TD-S"
 .StimulationPort "All"
 .StimulationMode "All"
 .SteadyStateLimit {q(m['end_db'])}
 .MeshAdaption "False"
 .AutoNormImpedance "True"
 .NormingImpedance "50"
 .CalculateModesOnly "False"
 .SParaSymmetry "False"
 .StoreTDResultsInCache "False"
 .FullDeembedding "False"
 .SuperimposePLWExcitation "False"
 .UseSensitivityAnalysis "False"
End With""", "setup")
    return B.blocks


def mesh_vba(mesh):
    return f"""
With Mesh
 .MeshType "PBA"
 .SetCreator "High Frequency"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "Version", 1%
 .Set "StepsPerWaveNear", {q(mesh['lpw'])}
 .Set "StepsPerWaveFar", {q(mesh['lpw'])}
 .Set "WavelengthRefinementSameAsNear", "1"
 .Set "StepsPerBoxNear", {q(mesh['lpb'])}
 .Set "StepsPerBoxFar", "1"
 .Set "MaxStepNear", "0"
 .Set "MaxStepFar", "0"
 .Set "ModelBoxDescrNear", "maxedge"
 .Set "ModelBoxDescrFar", "maxedge"
 .Set "UseMaxStepAbsolute", "0"
 .Set "GeometryRefinementSameAsNear", "0"
 .Set "UseRatioLimitGeometry", "1"
 .Set "RatioLimitGeometry", {q(mesh['ratio'])}
 .Set "MinStepGeometryX", {q(mesh['minstep'])}
 .Set "MinStepGeometryY", {q(mesh['minstep'])}
 .Set "MinStepGeometryZ", {q(mesh['minstep'])}
 .Set "UseSameMinStepGeometryXYZ", "1"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "FaceRefinementOn", "0"
 .Set "EdgeRefinementOn", "1"
 .Set "EdgeRefinementRatio", {q(mesh.get('edge', 4))}
 .Set "EdgeRefinementBufferLines", "3"
End With"""


def with_step(blocks, step_path):
    p = step_path.replace("\\", "/") if step_path else ""
    return [dict(b, code=b["code"].replace(STEP_TOKEN, p)) for b in blocks]


def to_bas(blocks, m, step_path=""):
    """one VBA macro (CST: Home > Macros > Run Macro): every block goes into the project history with
    AddToHistory, so the model stays parametric and editable; set stepFile (or pick it when asked) and, to solve
    and export Touchstone from the macro, RUN_SOLVER and outDir"""
    n = len([p for p in m["ports"]])
    s = [f"' {m['name']}: {ascii_only(m['title'])}",
         "' generated by kisi validation/test_board/cst_launch.py " + m["date"] + " for CST Studio Suite 2025.5",
         f"' mesh '{m['mesh']['preset']}' ({m['mesh']['lpw']} lines/wavelength, {m['mesh']['lpb']} per box); estimated cells "
         + ", ".join(f"{k} {v['local'] / 1e6:.2f} M (up to {v['global'] / 1e6:.2f} M)" for k, v in m["cells_estimate"].items()),
         ("' connector from primitives with the dimensions measured from Samtec's STEP (no STEP needed)" if m["step"].get("connector") == "parametric"
          else "' Samtec's STEP model is NOT part of kisi (licence): set stepFile below or pick the file when asked."),
         "Option Explicit", "", "Sub Main", " Dim stepFile As String, outDir As String, h As String, RUN_SOLVER As Boolean, PORT_CHECK As Boolean",
         " Dim p As Long, mt As String, zl As Double, msg As String, bad As Boolean",
         f' stepFile = "{step_path}"',
         ' outDir = ""            \' folder for the Touchstone file (RUN_SOLVER = True)',
         " RUN_SOLVER = False",
         " PORT_CHECK = True      ' modes-only solver run: every port must have the expected mode type and impedance",
         ]
    if any(STEP_TOKEN in b["code"] for b in blocks):
        s += [' If stepFile = "" Then stepFile = GetFilePath("292-J-P-EP-ST-EL-01.stp", "STEP|*.stp;*.step", , "Samtec STEP model 292-J-P-EP-ST-EL-01", 0)',
              ' If stepFile = "" Then',
              '  MsgBox "No STEP file: stopped."',
              "  Exit Sub",
              " End If",
              ' stepFile = Replace(stepFile, "\\", "/")']
    for b in blocks:
        s.append(f" ' --- {b['caption']}")
        s.append(' h = ""')
        for line in b["code"].rstrip("\n").split("\n"):
            lit = '"' + ascii_only(line).replace('"', '""') + '"'
            lit = lit.replace(STEP_TOKEN, '" & stepFile & "')
            s.append(f" h = h & {lit} & vbLf")
        s.append(f' AddToHistory "{ascii_only(b["caption"])}", h')
    s += [" If PORT_CHECK Then",
          '  Solver.CalculateModesOnly "True"',
          "  Solver.Start",
          '  msg = ""',
          "  bad = False"]
    for p in m["ports"]:
        if p["type"] != "waveguide":
            continue
        coax = abs(p["ref_shift"]) > 0 or p["orientation"] == "xmin"
        lo, hi = (47.0, 53.0) if coax else (40.0, 60.0)
        s += [f"  mt = Port.GetModeType({p['num']}, 1)",
              f"  zl = Port.GetLineImpedance({p['num']}, 1)",
              f'  msg = msg & "port {p["num"]}: mode 1 " & mt & ", Z = " & Format(zl, "0.00") & " ohm" & vbLf',
              f'  If zl < {lo} Or zl > {hi} Or mt = "" Or InStr("TEM|QTEM", mt) = 0 Then bad = True']
    s += ['  Solver.CalculateModesOnly "False"',
          '  ReportInformationToWindow "port modes:" & vbLf & msg',
          "  If bad Then",
          '   MsgBox "Port check FAILED (expected one TEM mode of about 50 ohm on the coax, a (quasi-)TEM line mode):" & vbLf & msg, vbCritical',
          "   Exit Sub",
          "  End If",
          " End If",
          " If RUN_SOLVER Then",
          "  Solver.Start",
          '  If outDir <> "" Then',
          "   With TOUCHSTONE",
          "    .Reset",
          f'    .FileName outDir & "\\{m["name"]}"',
          "    .Impedance 50",
          '    .FrequencyRange "Full"',
          "    .Renormalize True",
          "    .UseARResults False",
          "    .SetNSamples 1001",
          "    .Write",
          "   End With",
          "  End If",
          " End If",
          f' ReportInformationToWindow "{m["name"]}: model built ({len(blocks)} history blocks, {n} ports)"',
          "End Sub", ""]
    return "\r\n".join(s)


def ascii_only(t):
    rep = {"Ø": "D", "ε": "e", "δ": "d", "Ω": "ohm", "×": "x", "°": " deg", "–": "-", "—": "-", "→": "->", "±": "+-", "µ": "u", "³": "^3"}
    return "".join(rep.get(ch, ch if ord(ch) < 128 else "?") for ch in t)


def lint_vba(code):
    """structural checks of VBA code (the patterns CST's history interpreter needs): With/End With, block If/End
    If, For/Next balanced and nested, even quotes per line, ASCII only. Returns a list of problems."""
    probs, stack = [], []
    for n, raw in enumerate(code.split("\n"), 1):
        line = raw.strip()
        if any(ord(ch) > 127 for ch in raw):
            probs.append(f"line {n}: non-ASCII")
        if raw.count('"') % 2:
            probs.append(f"line {n}: odd number of quotes: {raw}")
        if not line or line.startswith("'"):
            continue
        code_part = re.sub(r'"[^"]*"', '""', line)
        w = code_part.split()
        if w[0] == "With":
            stack.append("With")
        elif code_part.startswith("End With"):
            if not stack or stack.pop() != "With":
                probs.append(f"line {n}: End With without With")
        elif w[0] == "If" and code_part.endswith("Then"):
            stack.append("If")
        elif w[0] in ("ElseIf", "Else"):
            if not stack or stack[-1] != "If":
                probs.append(f"line {n}: {w[0]} outside If")
        elif code_part.startswith("End If"):
            if not stack or stack.pop() != "If":
                probs.append(f"line {n}: End If without If")
        elif w[0] == "For":
            stack.append("For")
        elif w[0] == "Next":
            if not stack or stack.pop() != "For":
                probs.append(f"line {n}: Next without For")
        elif w[0] == "Sub":
            stack.append("Sub")
        elif code_part.startswith("End Sub"):
            if not stack or stack.pop() != "Sub":
                probs.append(f"line {n}: End Sub without Sub")
        elif w[0].startswith(".") and (not stack or "With" not in stack):
            probs.append(f"line {n}: member call outside With: {line}")
    if stack:
        probs.append(f"unclosed blocks: {stack}")
    return probs


# --- checks with OpenCASCADE (gmsh, optional) ----------------------------------------------------------------------
def _occ_vol(occ, tags):
    return sum(occ.getMass(3, t) for t in tags)


def _occ_common(occ, a, b):
    if not a or not b:
        return 0.0
    r, _ = occ.intersect([(3, t) for t in a], [(3, t) for t in b], removeObject=False, removeTool=False)
    occ.synchronize()
    v = sum(occ.getMass(3, t) for d, t in r if d == 3)
    occ.remove(r, recursive=True)
    occ.synchronize()
    return v


def step_parts_model(m, occ, gmsh, step_path, c):
    """Samtec's STEP placed as the --connector step mode does (gmsh converts its inch units to mm): {role: [tags]}"""
    sm = m["step"]
    before = set(t for _, t in gmsh.model.getEntities(3))
    occ.importShapes(step_path)
    occ.synchronize()
    role = {}
    for d, t in gmsh.model.getEntities(3):
        if t in before:
            continue
        nm = gmsh.model.getEntityName(d, t)
        role.setdefault(next((r for pat, r in sm["parts_map"] if pat in nm), "other"), []).append(t)
    occ.remove([(3, t) for t in role.pop("drop", [])], recursive=True)
    body, cont, plate = role["body"], role["contact"], role["plate"]
    b = sm["bead"]
    cyl = lambda x0, x1, r: [(3, occ.addCylinder(x0, 0, 0, x1 - x0, 0, 0, r))]
    holes, _ = occ.cut(cyl(b["x"][0] - 0.05, b["x"][1], b["r"]), [(3, t) for t in body], removeTool=False)
    bead, _ = occ.cut(cyl(b["x"][0] - 0.05, b["x"][1], b["r"]), holes)
    body2, _ = occ.cut([(3, t) for t in body], bead, removeTool=False)
    bead2, _ = occ.cut(bead, [(3, t) for t in cont], removeTool=False)
    cav = [cyl(x0, x1, r)[0] for x0, x1, r, _ in sm["cavity"]["sections"]]
    if cav:
        body2, _ = occ.cut(body2, cav)
    x = sm["cut_x"]
    body2, _ = occ.cut(body2, [(3, occ.addBox(-2, -8, -8, x + 2, 16, 16))])
    cont2, _ = occ.cut([(3, t) for t in cont], [(3, occ.addBox(-2, -8, -8, x + 2, 16, 16))])
    cont2, _ = occ.fuse(cont2, cyl(x - 0.01, 5.04, 0.4765))
    cont2, _ = occ.cut(cont2, [(3, occ.addBox(-2, -8, -8, x + 2, 16, 16))])
    h = sm.get("clip_body") or 0
    if h:
        body2, _ = occ.cut(body2, [(3, occ.addBox(-2, h, -8, 22, 8, 16)), (3, occ.addBox(-2, -8 - h, -8, 22, 8, 16))])
    if sm["pin_contact"]:
        pin, e = sm["contact"][-1], sm["board_edge_x"]
        cont2, _ = occ.fuse(cont2, [(3, occ.addBox(e + m["edge_pullback"], sm["board_top_y"], -pin[2], pin[1] - e - m["edge_pullback"], -sm["board_top_y"], 2 * pin[2]))])
    out = {"body": [t for _, t in body2], "contact": [t for _, t in cont2], "bead": [t for _, t in bead2], "plate": plate}
    main = [(3, t) for k in ("body", "contact", "bead") for t in out[k]]
    occ.rotate(main + [(3, t) for t in plate], 0, 0, 0, 1, 0, 0, math.pi / 2)
    if c["mirror"]:
        occ.rotate(main + [(3, t) for t in plate], 0, 0, 0, 0, 0, 1, math.pi)
    occ.translate(main, c["dx"], 0, c["dz_main"])
    occ.translate([(3, t) for t in plate], c["dx"], 0, c["dz_plate"])
    occ.synchronize()
    return out


def check_occ(m, step_path=None):
    """the parametric connector built with OpenCASCADE: volumes, no overlap between contact, body and bead, the jaw
    on the top copper, the pin block on the trace, nothing inside a board brick; with the STEP: the same parts of
    Samtec's model placed as --connector step does, volumes and the volume where the two differ"""
    import gmsh
    gmsh.initialize()
    gmsh.option.setNumber("General.Terminal", 0)
    occ = gmsh.model.occ
    res = {"connectors": []}
    try:
        for c in m["connectors"]:
            k = c["id"]
            t = ops_occ(connector_ops(m, c), occ)
            occ.synchronize()
            par = {r: t.get(f"conn{k}:{r}", []) for r in ("body", "contact", "bead", "plate")}
            r = {"id": k, "volume_mm3": {x: round(_occ_vol(occ, v), 4) for x, v in par.items()}}
            r["overlap_mm3"] = {"contact_body": round(_occ_common(occ, par["contact"], par["body"]), 6),
                                "contact_bead": round(_occ_common(occ, par["contact"], par["bead"]), 6),
                                "bead_body": round(_occ_common(occ, par["bead"], par["body"]), 6),
                                "contact_plate": round(_occ_common(occ, par["contact"], par["plate"]), 6)}
            zt, sg = m["z_top"], (-1 if c["mirror"] else 1)
            x0, x1 = sorted((c["edge_x"] + sg * 0.05, c["edge_x"] + sg * 5.0))
            slab = lambda za_, zb_, y0=-6, y1=6, xa=x0, xb=x1: [occ.addBox(xa, y0, za_, xb - xa, y1 - y0, zb_ - za_)]
            r["body_below_top_copper_mm3"] = round(_occ_common(occ, par["body"], slab(zt - 1.0, zt)), 6)
            r["body_on_top_copper_mm3"] = round(_occ_common(occ, par["body"], slab(zt, zt + 0.01)), 4)
            px0, px1 = sorted((c["edge_x"] + sg * 0.25, c["edge_x"] + sg * 0.6))
            r["pin_on_trace_mm3"] = round(_occ_common(occ, par["contact"], slab(zt, zt + 0.01, -0.1, 0.1, px0, px1)), 6)
            trace = []  # the signal copper as thin prisms (polygons as drawn)
            for sh in m["sheets"]:
                if sh["net"] != "SIG" or abs(sh["z"] - zt) > 1e-9:
                    continue
                rings = sh.get("polys") or [[[sh["rect"][0], sh["rect"][1]], [sh["rect"][2], sh["rect"][1]], [sh["rect"][2], sh["rect"][3]], [sh["rect"][0], sh["rect"][3]]]]
                for rg in rings:
                    rg = clean_ring(rg)
                    if not rg:
                        continue
                    pts = [occ.addPoint(x_, y_, zt - 0.005) for x_, y_ in rg]
                    lines = [occ.addLine(pts[i_], pts[(i_ + 1) % len(pts)]) for i_ in range(len(pts))]
                    srf = occ.addPlaneSurface([occ.addCurveLoop(lines)])
                    ex = occ.extrude([(2, srf)], 0, 0, 0.015)
                    trace += [t_ for d_, t_ in ex if d_ == 3]
            r["body_on_trace_mm3"] = round(_occ_common(occ, par["body"], trace), 6)
            g = ops_occ([op for op in connector_ops(m, c, grow=0.02) if op[1].startswith(f"conn{k}:c") or op[0] in ("add",) and op[1] == f"conn{k}:contact"
                         or (op[0] in ("brick",) and "pinblk" in op[1])], occ)
            occ.synchronize()
            r["clearance_0p02mm"] = {"grown_contact_body": round(_occ_common(occ, g.get(f"conn{k}:contact", []), par["body"]), 6),
                                     "grown_contact_plate": round(_occ_common(occ, g.get(f"conn{k}:contact", []), par["plate"]), 6)}
            hit = []
            for bk in m["bricks"]:
                v = _occ_common(occ, par["body"] + par["contact"] + par["plate"] + par["bead"],
                                [occ.addBox(bk["x"][0], bk["y"][0], bk["z"][0], bk["x"][1] - bk["x"][0], bk["y"][1] - bk["y"][0], bk["z"][1] - bk["z"][0])])
                if v > 1e-9:
                    hit.append([bk["name"], round(v, 6)])
            r["inside_board_bricks"] = hit
            if step_path and k == 1:
                st = step_parts_model(m, occ, gmsh, step_path, c)
                cmp_ = {}
                for x in ("body", "contact", "bead", "plate"):
                    va, vb = _occ_vol(occ, par[x]), _occ_vol(occ, st[x])
                    common = _occ_common(occ, par[x], st[x])
                    cmp_[x] = {"parametric": round(va, 4), "step": round(vb, 4), "differ": round(va + vb - 2 * common, 4)}
                r["vs_step_mm3"] = cmp_
            res["connectors"].append(r)
        return res
    finally:
        gmsh.finalize()


def occ_sections(m, step_path=None):
    """outlines in the section y = 0 (model coordinates) of the parametric connector and, with the STEP, of
    Samtec's model: {"parametric": {role: [curves]}, "step": {...}} or None without gmsh"""
    try:
        import gmsh
    except ImportError:
        return None
    gmsh.initialize()
    gmsh.option.setNumber("General.Terminal", 0)
    occ = gmsh.model.occ
    out = {"parametric": {}, "step": {}}
    try:
        groups = []
        for c in m["connectors"]:
            t = ops_occ(connector_ops(m, c), occ)
            occ.synchronize()
            groups.append(("parametric", {r: t.get(f"conn{c['id']}:{r}", []) for r in ("body", "contact", "bead", "plate")}))
            if step_path:
                groups.append(("step", step_parts_model(m, occ, gmsh, step_path, c)))
        x0 = min(p["x"] for p in m["ports"]) - 1
        x1 = max(max(p["x"] for p in m["ports"]), m["board_x"][1]) + 1
        z0, z1 = m["z_span"][0] - 1, m["z_span"][1] + 1
        for kind, parts in groups:
            for role, tags in parts.items():
                for tg in tags:
                    rect = occ.addRectangle(x0, z0, 0, x1 - x0, z1 - z0)
                    occ.rotate([(2, rect)], 0, 0, 0, 1, 0, 0, math.pi / 2)  # the plane y = 0 (x, z)
                    sec, _ = occ.intersect([(2, rect)], [(3, tg)], removeObject=True, removeTool=False)
                    occ.synchronize()
                    for dd, ss in sec:
                        if dd != 2:
                            continue
                        for _, cv in gmsh.model.getBoundary([(2, ss)], oriented=False):
                            lo, hi = gmsh.model.getParametrizationBounds(1, abs(cv))
                            us = [lo[0] + (hi[0] - lo[0]) * i / 24 for i in range(25)]
                            xyz = gmsh.model.getValue(1, abs(cv), us)
                            out[kind].setdefault(role, []).append([(xyz[3 * i], xyz[3 * i + 2]) for i in range(25)])
        return out
    finally:
        gmsh.finalize()


# --- geometry plot -------------------------------------------------------------------------------------------------
def plot_model(m, path, step_path=None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.patches import Polygon, Rectangle, Circle
    sm = m["step"]
    zt = m["z_top"]
    za = m["z_axis"]
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(13, 9.5), gridspec_kw={"height_ratios": [1.15, 1]})
    # top view
    bx, by = m["board_x"], m["board_y"]
    ax1.add_patch(Rectangle((bx[0], by[0]), bx[1] - bx[0], by[1] - by[0], fc="#e8f0e0", ec="#6a8a5a", lw=0.8))
    for s in m["sheets"]:
        if abs(s["z"] - zt) > 1e-9:
            continue
        col = "#c8973a" if s["net"] == "SIG" else "#d9b46a"
        if s.get("polys"):
            for rg in s["polys"]:
                ax1.add_patch(Polygon(rg, closed=True, fc=col, ec="#8a6420", lw=0.3, alpha=0.85))
        else:
            r = s["rect"]
            ax1.add_patch(Rectangle((r[0], r[1]), r[2] - r[0], r[3] - r[1], fc=col, ec="#8a6420", lw=0.3, alpha=0.85))
    for s in m["sheets"]:
        if abs(s["z"] - zt) < 1e-9 and s["net"] == "SIG":
            r = s["rect"]
            ax1.add_patch(Rectangle((r[0], r[1]), r[2] - r[0], r[3] - r[1], fc="#b0531e", ec="none"))
    for v in m["vias"]:
        ax1.add_patch(Circle((v["x"], v["y"]), v["r"], fc="#555" if not v.get("wall") else "#888", ec="none"))
    jw = sm["body_box"]["z_half"]
    for c in m["connectors"]:
        X = (lambda u, c=c: c["dx"] - u if c["mirror"] else c["dx"] + u)
        e = sm["board_edge_x"]
        xa, xb = sorted((X(sm["cut_x"]), X(sm["body_box"]["x"][1])))
        ax1.add_patch(Rectangle((xa, -jw), xb - xa, 2 * jw, fc="none", ec="#3060a0", lw=1.2, ls="--"))
        for hs in sm["jaw"]["slot_half"][1:]:
            xs0, xs1 = sorted((X(sm["jaw"]["slot_x"]), X(sm["body_box"]["x"][1])))
            ax1.add_patch(Rectangle((xs0, -hs), xs1 - xs0, 2 * hs, fc="none", ec="#3060a0", lw=0.8, ls=":"))
        pin = sm["contact"][-1]
        p0, p1 = sorted((X(pin[0]), X(pin[1])))
        ax1.add_patch(Rectangle((p0, -pin[2]), p1 - p0, 2 * pin[2], fc="#2050c0", ec="none"))
        ax1.annotate(f"connector {c['id']}\n(jaw, slot dashed;\n pin blue)", (X(e - 3.0), jw + 0.3), fontsize=7, color="#3060a0", ha="center")
    for p in m["ports"]:
        ax1.plot([p["x"], p["x"]], p["y"], color="#c02020", lw=2.5)
        ax1.annotate(f"P{p['num']}", (p["x"], p["y"][1] + 0.2), color="#c02020", fontsize=9, ha="center")
    ax1.set_aspect("equal")
    xlo = min(p["x"] for p in m["ports"]) - 0.8
    xhi = max(max(p["x"] for p in m["ports"]), bx[1]) + 0.8
    ax1.set_xlim(xlo, xhi)
    ax1.set_ylim(by[0] - 0.5, by[1] + 1.6)
    ax1.set_title(f"{m['name']}: top view (L1 copper, vias; connector outline from the STEP measurements)", fontsize=10)
    ax1.set_xlabel("x / mm (board edge of connector 1 at 0)")
    ax1.set_ylabel("y / mm")
    # side section y = 0
    for b in m["bricks"]:
        if b["name"].startswith("layer"):
            ax2.add_patch(Rectangle((b["x"][0], b["z"][0]), b["x"][1] - b["x"][0], b["z"][1] - b["z"][0], fc="#cfe3c0", ec="#8aa878", lw=0.4))
        elif b["y"][0] < 0 < b["y"][1]:
            ax2.add_patch(Rectangle((b["x"][0], b["z"][0]), b["x"][1] - b["x"][0], b["z"][1] - b["z"][0], fc="#3c8a3c", ec="none"))
    for s in m["sheets"]:
        if s["rect"][1] <= 0 <= s["rect"][3]:
            xs = [s["rect"][0], s["rect"][2]]
            if s.get("polys"):
                pts = [p for rg in s["polys"] for p in rg]
                if not any(abs(p[1]) < 1e-3 for p in pts) and not copper_at(s, (xs[0] + xs[1]) / 2, 0.0):
                    continue
            ax2.plot(xs, [s["z"]] * 2, color="#b0531e" if s["net"] == "SIG" else "#a07820", lw=2)
    secs = occ_sections(m, step_path)
    if secs:
        colr = {"body": "#3060a0", "contact": "#c02060", "plate": "#3060a0", "bead": "#555555"}
        for kind, lw, ls in (("parametric", 0.9, "-"), ("step", 0.6, "--")):
            for role, curves in secs[kind].items():
                for cv in curves:
                    ax2.plot([p_[0] for p_ in cv], [p_[1] for p_ in cv], ls, color="k" if kind == "step" else colr.get(role, "#999"), lw=lw)
    for c in m["connectors"]:
        X = (lambda u, c=c: c["dx"] - u if c["mirror"] else c["dx"] + u)
        if not secs:  # schematic from the measurements
            ob = sm["oc_bore"]
            for sg in (1, -1):
                ax2.plot([X(ob["x"][0]), X(ob["x"][1])], [za + sg * ob["r"]] * 2, color="#3060a0", lw=1)
                ax2.plot([X(sm["lip_opening"]["x"][0]), X(sm["lip_opening"]["x"][1])], [za + sg * sm["lip_opening"]["r"]] * 2, color="#3060a0", lw=1)
            pl = sm["plate"]
            x0, x1 = sorted((X(pl["x"][0]), X(pl["x"][1])))
            ax2.add_patch(Rectangle((x0, pl["y"][0] + c["dz_plate"]), x1 - x0, pl["y"][1] - pl["y"][0], fc="none", ec="#3060a0"))
            jx0, jx1 = sorted((X(sm["jaw"]["lip_x"]), X(sm["body_box"]["x"][1])))
            ax2.plot([jx0, jx1], [zt, zt], color="#3060a0", lw=1.5)
        bd = sm["bead"]
        x0, x1 = sorted((X(bd["x"][0]), X(bd["x"][1])))
        ax2.add_patch(Rectangle((x0, za - bd["r"]), x1 - x0, 2 * bd["r"], fc="#dddddd", ec="#777", lw=0.6, alpha=0.8))
        for (u0, u1, r, _) in sm["cavity"]["sections"]:
            x0, x1 = sorted((X(u0), X(u1)))
            ax2.add_patch(Rectangle((x0, za - r), x1 - x0, 2 * r, fc="#fff3c0", ec="#c09020", lw=0.6, ls="--", alpha=0.7))
        for (u0, u1, r, _) in sm["contact"]:
            if u1 <= sm["cut_x"]:
                continue
            u0 = max(u0, sm["cut_x"])
            x0, x1 = sorted((X(u0), X(u1)))
            ax2.add_patch(Rectangle((x0, za - r), x1 - x0, 2 * r, fc="#e04080", ec="none", alpha=0.85))
        if sm["pin_contact"]:
            pin = sm["contact"][-1]
            x0, x1 = sorted((X(sm["board_edge_x"] + m["edge_pullback"]), X(pin[1])))
            ax2.add_patch(Rectangle((x0, zt), x1 - x0, za - zt, fc="#e04080", ec="none"))
        xm = X(sm["mating_plane_x"])
        ax2.axvline(xm, color="#c02020", ls=":", lw=0.8)
        ax2.annotate("mating plane", (xm, za + 2.0), color="#c02020", fontsize=7, rotation=90, ha="right")
    for p in m["ports"]:
        ax2.plot([p["x"], p["x"]], p["z"], color="#c02020", lw=2.5)
        ax2.annotate(f"P{p['num']}", (p["x"], p["z"][1] + 0.15), color="#c02020", fontsize=9, ha="center")
    ax2.set_aspect("equal")
    ax2.set_xlim(xlo, xhi)
    ax2.set_ylim(m["z_span"][0] - 0.3, m["z_span"][1] + 0.3)
    ax2.set_title("section y = 0: board (green), copper, connector " + ("(parametric model: blue body / plate, pink contact"
                  + ("; Samtec's STEP dashed black)" if step_path else ")") if secs else "(schematic)")
                  + "; bead grey, ASSUMED bore yellow, pin contact block", fontsize=9)
    ax2.set_xlabel("x / mm")
    ax2.set_ylabel("z / mm")
    fig.tight_layout()
    fig.savefig(path, dpi=130)
    plt.close(fig)


# --- run in CST -----------------------------------------------------------------------------------------------------
def start_de(cst):
    di = cst.interface.DesignEnvironment
    for how in ("connect_to_any", "new", "ctor"):
        try:
            de = di.connect_to_any() if how == "connect_to_any" else di.new() if how == "new" else di()
            log("design environment via", how)
            return de
        except Exception as e:
            log("design environment", how, "failed:", repr(e))
    raise RuntimeError("cannot start the CST design environment")


def pick_sparam_items(items, nports):
    """{(i, j): tree item} of the S-parameters between the FUNDAMENTAL modes: 'S2(1),1(1)' (multi-mode ports) or 'S2,1',
    only directly in a '...\\S-Parameters' folder; anything else (a second mode 'S1(2),1(1)', a missing entry) fails"""
    found, other = {}, []
    for t in items:
        parts = t.split("\\")
        if len(parts) < 2 or parts[-2] != "S-Parameters":
            continue
        mm = re.fullmatch(r"S(\d+)(?:\((\d+)\))?,(\d+)(?:\((\d+)\))?", parts[-1])
        if not mm:
            continue
        i, mi, j, mj = int(mm.group(1)), mm.group(2), int(mm.group(3)), mm.group(4)
        if (mi or "1") != "1" or (mj or "1") != "1":
            other.append(parts[-1])
            continue
        key = (i - 1, j - 1)
        if key in found and found[key] != t:  # prefer the explicit mode form
            if "(" in parts[-1]:
                found[key] = t
            continue
        found[key] = t
    missing = [(i + 1, j + 1) for i in range(nports) for j in range(nports) if (i, j) not in found]
    if missing:
        raise RuntimeError(f"S-parameters missing for {missing}; S-parameter items: {[t.split(chr(92))[-1] for t in items][:40]}")
    if other:
        raise RuntimeError(f"higher port modes in the results ({other[:8]}): a port has more than one mode - check the port modes "
                           "(the coax port must have one TEM mode)")
    return found


def read_sparams(cst, prj_path, nports):
    pf = cst.results.ProjectFile(prj_path, allow_interactive=True)
    r3 = pf.get_3d()
    items = [t for t in r3.get_tree_items() if "S-Parameters" in t]
    found = pick_sparam_items(items, nports)
    data = {}
    for (i, j), t in found.items():
        it = r3.get_result_item(t)
        data[(i, j)] = ([float(x) * 1e9 for x in it.get_xdata()], [complex(y) for y in it.get_ydata()])
        if all(abs(v) == 0 for v in data[(i, j)][1]):
            raise RuntimeError(f"{t} is identically zero: no coupling between the ports - check the port modes and the geometry")
    log("   S-parameters read from:", ", ".join(f"S{i + 1}{j + 1} = {t.split(chr(92))[-1]}" for (i, j), t in sorted(found.items())))
    return data, {f"S{i + 1}{j + 1}": t for (i, j), t in found.items()}


def port_mode_report(cst, prj_path, m):
    """port line impedances from the results of a modes-only run: [(port, mode, Z)] (best effort: the tree names
    differ between versions; nothing found -> [])"""
    pf = cst.results.ProjectFile(prj_path, allow_interactive=True)
    r3 = pf.get_3d()
    rows = []
    for t in r3.get_tree_items():
        if "Line Impedance" not in t and "Port Impedance" not in t:
            continue
        mm = re.search(r"Port ?(\d+)\D*(?:\[(\d+)\])?$", t.split("\\")[-1])
        if not mm:
            continue
        try:
            ys = [abs(complex(y)) for y in r3.get_result_item(t).get_ydata()]
        except Exception:
            continue
        if ys:
            rows.append((int(mm.group(1)), int(mm.group(2) or 1), sum(ys) / len(ys), t))
    return rows


def solver_cells(od, name):
    """mesh cells from CST's solver log (None if not found)"""
    import glob
    for f in sorted(glob.glob(os.path.join(od, name, "**", "*.log"), recursive=True), key=os.path.getmtime, reverse=True)[:12]:
        try:
            t = open(f, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        mm = re.search(r"(?:Total number of mesh cells|Number of mesh cells|mesh cells)[^\d]*([\d][\d,\. ]*\d)", t, re.I)
        if mm:
            return int(re.sub(r"[,\. ]", "", mm.group(1)))
    return None


def write_touchstone(path, freq, S, comments, z0=50.0):
    n = len(S[0])
    with open(path, "w", encoding="utf-8") as fh:
        for c in comments:
            fh.write(f"! {c}\n")
        fh.write(f"# Hz S RI R {z0:g}\n")
        for k, f in enumerate(freq):
            vals = [S[k][0][0]] if n == 1 else [S[k][0][0], S[k][1][0], S[k][0][1], S[k][1][1]]
            fh.write(f"{f:.6e} " + " ".join(f"{v.real:.6e} {v.imag:.6e}" for v in vals) + "\n")


def run_in_cst(m, blocks, a, od):
    import cst  # noqa: F401
    import cst.interface
    import cst.results
    de = start_de(cst)
    try:
        de.set_quiet_mode(True)
    except Exception as e:
        log("quiet mode n/a", repr(e))
    meta = {"model": m["name"], "date": datetime.date.today().isoformat(), "mesh": m["mesh"]["preset"], "status": "started", "blocks_failed": []}
    prj = de.new_mws()
    try:
        for b in with_step(blocks, a.step):
            try:
                prj.model3d.add_to_history(b["caption"], b["code"])
            except Exception as e:
                meta["blocks_failed"].append({"caption": b["caption"], "error": str(e)})
                log("   history block failed:", b["caption"], repr(e))
                raise
        cp = os.path.join(od, m["name"] + ".cst")
        try:
            prj.save(cp, allow_overwrite=True)
        except TypeError:
            prj.save(cp)
        log("saved", cp)
        if a.no_solve:
            meta["status"] = "built"
            return meta
        if not a.no_port_check:  # modes only: one TEM mode of about 50 ohm on every coax port
            prj.model3d.add_to_history("port check: modes only", 'Solver.CalculateModesOnly "True"')
            prj.model3d.run_solver()
            prj.save()
            rows = port_mode_report(cst, cp, m)
            meta["port_modes"] = [{"port": r[0], "mode": r[1], "z_ohm": round(r[2], 3), "item": r[3]} for r in rows]
            for r in rows:
                log(f"   port {r[0]} mode {r[1]}: Z = {r[2]:.2f} ohm  ({r[3]})")
            if not rows:
                log("   port check: no line impedance found in the results; look at 2D/3D Results > Port Modes")
            coax = [p["num"] for p in m["ports"] if p["type"] == "waveguide" and abs(p["ref_shift"]) > 0]
            bad = [r for r in rows if r[1] > 1] + [r for r in rows if r[0] in coax and r[1] == 1 and not 47.0 < r[2] < 53.0]
            if bad:
                raise RuntimeError(f"port check failed: {[(r[0], r[1], round(r[2], 2)) for r in bad]} (expected one mode per port, "
                                   "about 50 ohm on the coax); see 2D/3D Results > Port Modes")
            prj.model3d.add_to_history("port check done: full solve", 'Solver.CalculateModesOnly "False"')
        presets = MESH_ORDER[MESH_ORDER.index(m["mesh"]["preset"]):] if m["mesh"]["preset"] in MESH_ORDER else [m["mesh"]["preset"]]
        for k, pr in enumerate(presets):
            if k:
                log(f"   mesh '{presets[k - 1]}' exceeds the licence's mesh size: '{pr}' instead (--fallback)")
                prj.model3d.add_to_history(f"mesh (fallback {pr})", mesh_vba(dict({"ratio": 20, "minstep": 0.0}, **MESH_PRESETS[pr])))
            t0 = time.time()
            try:
                prj.model3d.run_solver()
                meta.update(mesh=pr, seconds=round(time.time() - t0, 1))
                break
            except Exception as e:
                mm = re.search(r"up to (\d+) mesh cells", str(e))
                if mm and a.fallback and k + 1 < len(presets):
                    meta.setdefault("mesh_refused", []).append({"preset": pr, "error": str(e)})
                    continue
                raise
        prj.save()
        n = len(m["ports"])
        meta["cells"] = solver_cells(od, m["name"])
        data, meta["sparam_items"] = read_sparams(cst, cp, n)
        freq = data[(0, 0)][0]
        S = [[[data[(i, j)][1][kk] for j in range(n)] for i in range(n)] for kk in range(len(freq))]
        hdr = [f"{m['name']}: {m['title']}", f"CST Studio Suite, time domain, PBA, mesh '{meta['mesh']}' {m['mesh']}, {meta.get('cells')} cells, {meta.get('seconds')} s",
               "ports: " + "; ".join(f"{p['num']} {p['what']}" for p in m["ports"]), "S normalised to 50 ohm"] + m["notes"]
        tp = os.path.join(od, f"cst_{meta['mesh']}.s{n}p")
        write_touchstone(tp, freq, S, [ascii_only(h) for h in hdr])
        if n == 2 and m["model"] != "thru2":
            write_touchstone(os.path.join(od, f"cst_{meta['mesh']}_s11.s1p"), freq, [[[s[0][0]]] for s in S], [ascii_only(h) for h in hdr] + ["S11 only"])
        meta.update(status="ok", touchstone=os.path.basename(tp))
        log("done:", tp)
        return meta
    except Exception as e:
        meta.update(status="failed", error=str(e))
        log("FAILED:", repr(e))
        # keep the project open in CST (message window, 3D view, history list) and save it for later
        fp = os.path.join(od, m["name"] + "_failed.cst")
        try:
            try:
                prj.save(fp, allow_overwrite=True)
            except TypeError:
                prj.save(fp)
            log("project left open in CST and saved as", fp, "- see the message window (e.g. 'body box') and the history list")
        except Exception as e2:
            log("could not save the failed project:", repr(e2))
        return meta
    finally:
        json.dump(meta, open(os.path.join(od, "cst_meta.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
        if meta["status"] != "failed" and not a.keep_open:
            try:
                prj.close()
            except Exception:
                pass


# --- main -----------------------------------------------------------------------------------------------------------
def print_summary(m):
    log(f"{m['name']}: {m['title']}")
    log(f"  board {m['board_x'][1]:g} x {m['board_y'][1] - m['board_y'][0]:g} mm, {len(m['sheets'])} copper sheets, {len(m['vias'])} vias"
        + (" (with row walls)" if m["walls"] else "") + f", {len(m['connectors'])} connector(s)")
    for p in m["ports"]:
        log(f"  port {p['num']}: {p['what']}")
    for pat, role in m["step"]["parts_map"]:
        log(f"  STEP part '{pat}' -> {role}")
    sm = m["step"]
    log(f"  bead x {sm['bead']['x'][0]}-{sm['bead']['x'][1]} mm, Ø{2 * sm['bead']['r']:.3f} mm, εr {m['materials']['bead']['er']:g}")
    for s in sm["cavity"]["sections"]:
        log(f"  ASSUMED bore x {s[0]}-{s[1]}: Ø{2 * s[2]:.3f} around the Ø{2 * s[3]:.3f} contact ({sm['cavity']['z']:g} ohm, εr {sm['cavity']['er']:g})")
    log(f"  estimated cells, ROUGH (±40 %; local / global-λ model; limit {m['max_cells'] / 1e6:g} M; CST's mesh view shows the real count):")
    for k in [x for x in MESH_ORDER + ["custom"] if x in m["cells_estimate"]]:
        v = m["cells_estimate"][k]
        fit = "fits" if v["local"] <= m["max_cells"] * 0.85 else ("borderline" if v["local"] <= m["max_cells"] else "too many")
        mp = MESH_PRESETS.get(k, m["mesh"])
        log(f"    {k:6s} {mp['lpw']:g}/{mp['lpb']:g}: {v['local'] / 1e6:.2f} M ({v['global'] / 1e6:.2f} M)  {fit}"
            + ("   <- used" if k == m["mesh"]["preset"] else ""))
    for n_ in m["notes"]:
        log("  note: " + n_)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model", choices=["launch1", "thru2", "samtec_bench"], default="launch1")
    ap.add_argument("--variant", default="default", help="launch variant: make_coupons.LAUNCH_VARIANTS without pin/body, tip023, samtec_pad")
    ap.add_argument("--stackup", default="jlc4_7628", help="stackup key with boards/<key>.design.json (launch1, thru2)")
    ap.add_argument("--connectors", type=int, choices=[1, 2], default=None, help="samtec_bench: 1 (12.7 mm) or 2 (25.4 mm)")
    ap.add_argument("--feed", type=float, default=None, help="board edge to port 2 / half the thru (default: connectors.json layout.feed 12.5 mm)")
    ap.add_argument("--step", default="", help="Samtec's STEP file 292-J-P-EP-ST-EL-01.stp (not in the repository)")
    ap.add_argument("--out", default="cst_launch_out")
    ap.add_argument("--name", default="")
    ap.add_argument("--dry-run", action="store_true", help="write the .bas macro, the model JSON and the geometry plot; no CST")
    ap.add_argument("--from-json", default="", help="rebuild from a model JSON written by --dry-run (no repository files needed)")
    ap.add_argument("--no-solve", action="store_true", help="build and save the CST project only")
    ap.add_argument("--fallback", action="store_true", help="when CST refuses the mesh (licence), retry with the next coarser preset")
    ap.add_argument("--mesh", choices=["auto"] + MESH_ORDER, default="auto", help="auto: the finest preset whose estimate fits under --max-cells (85 %%)")
    ap.add_argument("--max-cells", type=float, default=1e6)
    ap.add_argument("--lpw", type=float, default=None, help="own lines per wavelength (overrides the preset)")
    ap.add_argument("--lpb", type=float, default=None, help="own lines per model box (overrides the preset)")
    ap.add_argument("--fmax", type=float, default=40e9)
    ap.add_argument("--end-db", type=float, default=-50)
    ap.add_argument("--loss", choices=["sigma", "tand"], default="sigma")
    ap.add_argument("--metal", choices=["pec", "lossy"], default="pec")
    ap.add_argument("--bead-er", type=float, default=2.1, help="support bead εr (PTFE 2.1)")
    ap.add_argument("--bead-tand", type=float, default=0.0003)
    ap.add_argument("--cavity-z", type=float, default=50.0, help="impedance of the ASSUMED bore between the bead and the lip; 0: none (shorts the contact)")
    ap.add_argument("--cavity-er", type=float, default=1.0, help="its fill (1: air)")
    ap.add_argument("--coax-port", choices=["inner", "ext"], default="inner")
    ap.add_argument("--coax-cut", type=float, default=4.4, help="inner: STEP x of the coax port (uniform air line 4.0-5.1 mm)")
    ap.add_argument("--ext-len", type=float, default=1.0, help="ext: plug extension ahead of the connector's front face (mm)")
    ap.add_argument("--coax-port-half", type=float, default=None, help="half side of the square coax port (default: the outer conductor's radius)")
    ap.add_argument("--connector", choices=["parametric", "step"], default="parametric",
                    help="parametric (default): the connector from CST primitives with the dimensions measured from Samtec's STEP "
                         "(connectors.json step_model), placed directly; step: Samtec's STEP imported (for comparison; --step, --step-scale)")
    ap.add_argument("--no-port-check", action="store_true", help="skip the port-mode check (modes-only solver run) before the solve")
    ap.add_argument("--port2", choices=["wg", "face"], default="wg", help="launch1 / samtec_bench --connectors 1: port at the line end")
    ap.add_argument("--line-port-half", type=float, default=1.4)
    ap.add_argument("--line-port-height", type=float, default=1.2, help="waveguide port on the line: height above L1 (mm)")
    ap.add_argument("--no-pin-contact", dest="pin_contact", action="store_false")
    ap.add_argument("--part", action="append", default=[], help="ROLE=PATTERN: STEP solid names containing PATTERN get ROLE (body, contact, plate, drop, bead)")
    ap.add_argument("--symmetry", choices=["magnetic", "none"], default="magnetic")
    ap.add_argument("--walls", action="store_true", help="add the GND via walls between the coupon rows (y = ±6 mm, 1.27 mm pitch; the kisi-fdtd launch cases have none)")
    ap.add_argument("--monitors", action="store_true", help="E-field monitors at 10/20/30/40 GHz")
    ap.add_argument("--air", type=float, default=1.0, help="space above / below the connector (mm)")
    ap.add_argument("--clip-body", type=float, default=3.0, help="cut the connector body's outside more than this above / below the axis (mm; 0: the full body)")
    ap.add_argument("--no-orient-check", action="store_true")
    ap.add_argument("--step-scale", type=float, default=None, help="scale the imported STEP solids (25.4 if CST imports the inch STEP without converting it to mm)")
    ap.add_argument("--edge-ratio", type=float, default=None, help="CST edge refinement ratio (default 4; 2 saves cells)")
    ap.add_argument("--ratio-limit", type=float, default=None, help="CST mesh ratio limit (default 20; 10 saves cells)")
    ap.add_argument("--keep-open", action="store_true", help="leave the project open in CST after a successful run (a failed run always stays open)")
    ap.add_argument("--plot", action="store_true", help="geometry plot also without --dry-run")
    ap.add_argument("--check-occ", action="store_true", help="build the parametric connector with OpenCASCADE (pip: gmsh) and check it; "
                    "with --step also against Samtec's model")
    a = ap.parse_args()
    if a.from_json:
        m = json.load(open(a.from_json, encoding="utf-8"))
    else:
        m = build_model(a)
    if a.step_scale:
        m["step"]["step_scale"] = a.step_scale
    if a.from_json:  # mesh choices also apply to a model rebuilt from its JSON
        if a.mesh != "auto":
            m["mesh"] = dict({"ratio": 20, "minstep": 0.0}, **MESH_PRESETS[a.mesh], preset=a.mesh)
        if a.lpw or a.lpb:
            m["mesh"].update(preset="custom", lpw=a.lpw or m["mesh"]["lpw"], lpb=a.lpb or m["mesh"]["lpb"])
    if a.edge_ratio:
        m["mesh"]["edge"] = a.edge_ratio
    if a.ratio_limit:
        m["mesh"]["ratio"] = a.ratio_limit
    print_summary(m)
    blocks = vba_blocks(m)
    probs = [f"{b['caption']}: {p}" for b in blocks for p in lint_vba(b["code"])]
    od = os.path.join(a.out, m["name"])
    os.makedirs(od, exist_ok=True)
    bas = to_bas(blocks, m, (a.step or "").replace("\\", "/"))
    probs += [f"macro: {p}" for p in lint_vba(bas.replace("\r\n", "\n"))]
    with open(os.path.join(od, m["name"] + ".bas"), "w", encoding="ascii", newline="") as fh:
        fh.write(bas)
    json.dump(m, open(os.path.join(od, m["name"] + ".model.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    log(f"  {len(blocks)} history blocks -> {os.path.join(od, m['name'] + '.bas')}, model JSON next to it")
    if probs:
        for p in probs[:20]:
            log("  VBA lint:", p)
        raise SystemExit(f"{len(probs)} VBA lint problems")
    if a.check_occ:
        res = check_occ(m, a.step or None)
        json.dump(res, open(os.path.join(od, m["name"] + ".occ_check.json"), "w", encoding="utf-8"), indent=1)
        for r in res["connectors"]:
            log(f"  OCC check connector {r['id']}: volumes {r['volume_mm3']}")
            log(f"    overlaps {r['overlap_mm3']} (all 0), clearance 0.02 mm {r['clearance_0p02mm']} (all 0)")
            log(f"    body below the top copper {r['body_below_top_copper_mm3']} (0), on it {r['body_on_top_copper_mm3']} (> 0), "
                f"on the trace {r['body_on_trace_mm3']} (0), pin block on the trace {r['pin_on_trace_mm3']} (> 0), inside board bricks {r['inside_board_bricks']}")
            if "vs_step_mm3" in r:
                log(f"    parametric vs Samtec's STEP: {r['vs_step_mm3']}")
            ok = (all(v < 1e-6 for v in r["overlap_mm3"].values()) and all(v < 1e-6 for v in r["clearance_0p02mm"].values())
                  and r["body_below_top_copper_mm3"] < 1e-6 and r["body_on_top_copper_mm3"] > 0 and r["body_on_trace_mm3"] < 1e-6
                  and (r["pin_on_trace_mm3"] > 0 or not m["step"]["pin_contact"]) and not r["inside_board_bricks"])
            if not ok:
                raise SystemExit(f"OCC check of connector {r['id']} FAILED")
        log("  OCC check passed")
    if a.dry_run or a.plot:
        try:
            plot_model(m, os.path.join(od, m["name"] + ".png"), a.step or None)
            log("  geometry plot", os.path.join(od, m["name"] + ".png"))
        except ImportError as e:
            log("  no plot (matplotlib missing):", e)
    if a.dry_run:
        return
    if m["step"].get("connector", "step") == "step" and (not a.step or not os.path.exists(a.step)):
        raise SystemExit("--connector step: Samtec's STEP file (--step) is needed")
    meta = run_in_cst(m, blocks, a, os.path.abspath(od))
    log("status:", meta["status"], meta.get("touchstone", meta.get("error", "")))


if __name__ == "__main__":
    main()
