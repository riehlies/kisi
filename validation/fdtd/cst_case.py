"""Build a kisi-fdtd reference case (via_cases/*.json) in CST Studio Suite: the case is turned into
a list of VBA history blocks (CST's macro language), which `run_cst.py` adds to a new CST project
through the `cst.interface` Python API, or which can be written to one `.bas` macro file.

Same model as kisi-fdtd and openEMS (`openems_case.py`):
* dielectric layers as bricks across the board; loss as a constant conductivity
  sigma = 2 pi f_loss eps0 eps_r tan(delta) (f_loss = case["f_loss"] or fmax / 2), so tan(delta) is exact at
  f_loss and falls as 1/f away from it, exactly as kisi-fdtd and openEMS treat it;
* copper as zero-thickness PEC sheets (traces, planes with round or slot-shaped antipads, pads,
  polygon copper with even-odd rings) and solid PEC cylinders (via barrels), cut out of the
  dielectric bricks (Solid.Insert);
* 50 ohm (G, H: 1 ohm) discrete ports along z at the ends of the feed traces. Across a trace the port
  is a discrete *face* port spanning the trace width (as kisi-fdtd's lumped port); a short piece of the
  reference plane under the port end is split off as its own sheet so that the face port has an edge
  to end on (the copper is unchanged). Ports of zero width (G, H) are discrete edge ports;
* lumped series R-L-C elements (CST LumpedElement "RLCSerial");
* open boundaries: in x and y at the board edge when the board fills the domain (CST extends touching
  structures into the PML, so planes and dielectric are infinite as in kisi-fdtd), elsewhere with
  background space up to the case's domain;
* time-domain solver (FIT, hexahedral mesh with PBA, i.e. conformal metal), steady-state energy
  criterion = case end_db (-50 dB), frequency range 0..fmax.

CST cannot mesh with kisi-fdtd's explicit lines; it uses its own automatic hexahedral mesh. The mesh
density is set by `mesh` options (lines per wavelength, lines per box, smallest step). The run
reports the mesh cells; a case that exceeds the available mesh size is reported,
not silently coarsened (run_cst.py).

usage (anywhere, no CST needed): python3 cst_case.py case.json out.bas [--port face|edge]
"""
import json, math, sys

EPS0 = 8.8541878128e-12


def q(x):
    """a number as a CST string argument"""
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


def boundary_loops(pts, tol=1e-6):
    """the boundary of an even-odd polygon ring as simple loops: edges are split where they
    overlap collinear edges, opposite edges cancel (zone-fill bridges and slits that run along
    an edge and back), and the remaining edges are chained into loops. The filled region is
    unchanged; CST cannot cover rings with repeated vertices or overlapping edges."""
    q = lambda p: (round(p[0] / tol), round(p[1] / tol))
    P = [list(map(float, p)) for p in pts]
    n = len(P)
    edges = [(P[i], P[(i + 1) % n]) for i in range(n)]
    cuts = [[] for _ in edges]
    boxes = [(min(a[0], b[0]) - tol, max(a[0], b[0]) + tol, min(a[1], b[1]) - tol, max(a[1], b[1]) + tol) for a, b in edges]
    for i, (a, b) in enumerate(edges):
        dx, dy = b[0] - a[0], b[1] - a[1]
        L2 = dx * dx + dy * dy
        if L2 < tol * tol:
            continue
        bi = boxes[i]
        for j, (c, d) in enumerate(edges):
            if j == i:
                continue
            bj = boxes[j]
            if bj[0] > bi[1] or bj[1] < bi[0] or bj[2] > bi[3] or bj[3] < bi[2]:
                continue
            for r in (c, d):  # an end of edge j inside edge i, on its line
                if abs(dx * (r[1] - a[1]) - dy * (r[0] - a[0])) / math.sqrt(L2) > tol:
                    continue
                t = ((r[0] - a[0]) * dx + (r[1] - a[1]) * dy) / L2
                if tol < t * math.sqrt(L2) < math.sqrt(L2) - tol:
                    cuts[i].append(t)
    segs = []
    for i, (a, b) in enumerate(edges):
        ts = [0.0] + sorted(set(round(t, 12) for t in cuts[i])) + [1.0]
        pts_i = [[a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])] for t in ts]
        for u, v in zip(pts_i, pts_i[1:]):
            if q(u) != q(v):
                segs.append((q(u), q(v), u, v))
    count = {}
    for ku, kv, u, v in segs:
        count[(ku, kv)] = count.get((ku, kv), 0) + 1
    out = {}  # start key -> list of (end key, start point, end point)
    used = {}
    for ku, kv, u, v in segs:
        k, r = (ku, kv), (kv, ku)
        c = min(count.get(k, 0), count.get(r, 0))
        if c and used.get(k, 0) < c:  # cancelled against an opposite edge
            used[k] = used.get(k, 0) + 1
            continue
        out.setdefault(ku, []).append((kv, u, v))
    loops = []
    while out:
        k0 = next(iter(out))
        loop, k = [], k0
        while True:
            lst = out.get(k)
            if not lst:
                break
            kv, u, v = lst.pop()
            if not lst:
                del out[k]
            loop.append(u)
            k = kv
            if k == k0:
                break
        if len(loop) >= 3:
            loops.append(loop)
    return [lp for loop in loops for lp in split_ring(loop, tol)]


def interior_point(ring):
    """a point just inside a simple ring (next to the middle of its longest edge)"""
    area = sum(ring[k - 1][0] * ring[k][1] - ring[k][0] * ring[k - 1][1] for k in range(len(ring))) / 2
    k = max(range(len(ring)), key=lambda i: math.hypot(ring[i][0] - ring[i - 1][0], ring[i][1] - ring[i - 1][1]))
    a, b = ring[k - 1], ring[k]
    L = math.hypot(b[0] - a[0], b[1] - a[1]) or 1.0
    nx, ny = -(b[1] - a[1]) / L, (b[0] - a[0]) / L  # left normal: inside for a counter-clockwise ring
    if area < 0:
        nx, ny = -nx, -ny
    e = min(1e-4, 0.01 * L)
    return (a[0] + b[0]) / 2 + e * nx, (a[1] + b[1]) / 2 + e * ny


def split_ring(pts, tol=1e-6):
    """a ring that touches itself (zone fills: outline and holes joined by zero-width bridges
    that repeat vertices) split into simple loops at the repeated vertices"""
    key = lambda p: (round(p[0] / tol), round(p[1] / tol))
    path, seen, loops = [], {}, []
    for p in list(pts) + [pts[0]]:
        k = key(p)
        if k in seen:
            i = seen[k]
            loop = path[i:]
            for q in loop[1:]:
                seen.pop(key(q), None)
            path = path[:i + 1]
            if len(loop) >= 3:
                loops.append(loop)
        else:
            seen[k] = len(path)
            path.append(p)
    if len(path) >= 3:
        loops.append(path)
    return loops


def clean_ring(pts, tol=1e-6):
    """CST rejects polygons with repeated or collinear vertices: drop points closer than tol to
    the previous one (and a closing repeat of the first), then collinear points; None if fewer
    than 3 points or no area remain"""
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


class Model:
    """collects the VBA history blocks: [(caption, code, kind)], kind "geo" | "port" | "setup" """

    def __init__(self):
        self.blocks = []
        self.n = 0

    def add(self, caption, code, kind="geo"):
        self.blocks.append({"caption": caption, "code": code.strip() + "\n", "kind": kind})

    def name(self, base):
        self.n += 1
        return f"{base}{self.n}"

    # --- curves and sheets in the plane z = const ---------------------------------------------
    @staticmethod
    def wcs(z):
        return (f'WCS.ActivateWCS "local"\nWCS.SetNormal "0", "0", "1"\nWCS.SetOrigin "0", "0", {q(z)}\n'
                f'WCS.SetUVector "1", "0", "0"\n')

    @staticmethod
    def wcs_off():
        return 'WCS.ActivateWCS "global"\n'

    def rect_sheet(self, comp, name, x0, y0, x1, y1, z, material="PEC"):
        c = self.name("c")
        return (self.wcs(z) +
                f'With Rectangle\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Xrange {q(x0)}, {q(x1)}\n .Yrange {q(y0)}, {q(y1)}\n .Create\nEnd With\n'
                f'With CoverCurve\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{material}"\n .Curve "{c}:{c}"\n'
                f' .DeleteCurve "True"\n .Create\nEnd With\n' + self.wcs_off())

    def circle_sheet(self, comp, name, x, y, r, z, material="PEC"):
        c = self.name("c")
        return (self.wcs(z) +
                f'With Circle\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Radius {q(r)}\n .Xcenter {q(x)}\n .Ycenter {q(y)}\n'
                f' .Segments "0"\n .Create\nEnd With\n'
                f'With CoverCurve\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{material}"\n .Curve "{c}:{c}"\n'
                f' .DeleteCurve "True"\n .Create\nEnd With\n' + self.wcs_off())

    def poly_sheet(self, comp, name, pts, z, material="PEC"):
        c = self.name("c")
        s = f'With Polygon\n .Reset\n .Name "{c}"\n .Curve "{c}"\n .Point {q(pts[0][0])}, {q(pts[0][1])}\n'
        for p in pts[1:] + [pts[0]]:
            s += f' .LineTo {q(p[0])}, {q(p[1])}\n'
        s += ' .Create\nEnd With\n'
        return (self.wcs(z) + s +
                f'With CoverCurve\n .Reset\n .Name "{name}"\n .Component "{comp}"\n .Material "{material}"\n .Curve "{c}:{c}"\n'
                f' .DeleteCurve "True"\n .Create\nEnd With\n' + self.wcs_off())

    def hole_sheet(self, comp, name, h, z):
        """circle [cx, cy, r] or slot [x0, y0, x1, y1, r] (stadium) as a sheet to subtract"""
        if len(h) < 5:
            return self.circle_sheet(comp, name, h[0], h[1], h[2], z)
        x0, y0, x1, y1, r = h
        a = math.atan2(y1 - y0, x1 - x0)
        n = 48
        pts = []
        for k in range(n + 1):  # half circle around the end (x1, y1)
            t = a - math.pi / 2 + math.pi * k / n
            pts.append((x1 + r * math.cos(t), y1 + r * math.sin(t)))
        for k in range(n + 1):  # and around the start
            t = a + math.pi / 2 + math.pi * k / n
            pts.append((x0 + r * math.cos(t), y0 + r * math.sin(t)))
        return self.poly_sheet(comp, name, clean_ring(pts), z)


def build(case, port_mode="face", mesh=None, simple_clip=False):
    """VBA history blocks for one case. mesh: dict of mesh options (see MESH_DEFAULT).
    Polygon copper reaching beyond the domain is cut to it by a Boolean intersection in CST
    (simple_clip: by Sutherland-Hodgman here instead, as in the first runs; it leaves overlapping
    edges along the cut for concave rings, which CST cannot cover)."""
    mesh = dict(MESH_DEFAULT, **(mesh or {}))
    m = Model()
    fmax = case["fmax"]
    f_loss = case.get("f_loss") or fmax / 2
    dom = case["domain"]
    board = case["board"]
    zlo = min(l["z0"] for l in case["layers"])
    zhi = max(l["z1"] for l in case["layers"])
    for s in case.get("sheets", []):
        zlo, zhi = min(zlo, s["z"]), max(zhi, s["z"])
    for v in case.get("vias", []):
        zlo, zhi = min(zlo, v["z0"]), max(zhi, v["z1"])

    m.add("units", 'With Units\n .Geometry "mm"\n .Frequency "GHz"\n .Time "ns"\n .TemperatureUnit "Kelvin"\nEnd With', "setup")
    # background: vacuum, space up to the case's domain (0 where the board touches the domain face)
    sp = [max(0.0, board[0] - dom[0]), max(0.0, dom[3] - board[2]), max(0.0, board[1] - dom[1]), max(0.0, dom[4] - board[3]),
          max(0.0, zlo - dom[2]), max(0.0, dom[5] - zhi)]
    m.add("background", f"""
With Background
 .ResetBackground
 .Type "Normal"
 .Epsilon "1.0"
 .Mu "1.0"
 .XminSpace {q(sp[0])}
 .XmaxSpace {q(sp[1])}
 .YminSpace {q(sp[2])}
 .YmaxSpace {q(sp[3])}
 .ZminSpace {q(sp[4])}
 .ZmaxSpace {q(sp[5])}
 .ApplyInAllDirections "False"
End With""", "setup")
    m.add("boundaries", """
With Boundary
 .Xmin "open"
 .Xmax "open"
 .Ymin "open"
 .Ymax "open"
 .Zmin "open"
 .Zmax "open"
 .Xsymmetry "none"
 .Ysymmetry "none"
 .Zsymmetry "none"
 .ApplyInAllDirections "False"
End With""", "setup")
    m.add("frequency", f'Solver.FrequencyRange "0", {q(fmax / 1e9)}', "setup")

    # dielectric layers
    mats = {}
    for n, l in enumerate(case["layers"]):
        key = (l["er"], l.get("tand", 0.0))
        if key not in mats:
            mname = f"diel_er{l['er']:g}_tand{key[1]:g}".replace(".", "p")
            mats[key] = mname
            m.add(f"material {mname}", f"""
With Material
 .Reset
 .Name "{mname}"
 .Folder ""
 .Type "Normal"
 .Epsilon {q(l['er'])}
 .Mu "1.0"
 .Sigma {q(sigma(l['er'], key[1], f_loss))}
 .TanDGiven "False"
 .TanD "0.0"
 .SigmaM "0"
 .Colour "0.2", "0.6", "0.2"
 .Transparency "70"
 .Create
End With""")
        m.add(f"layer {n}", f"""
With Brick
 .Reset
 .Name "layer{n}"
 .Component "board"
 .Material "{mats[key]}"
 .Xrange {q(board[0])}, {q(board[2])}
 .Yrange {q(board[1])}, {q(board[3])}
 .Zrange {q(l['z0'])}, {q(l['z1'])}
 .Create
End With""")

    # face ports: the trace sheet at z1 that ends on the port line, and a short piece of the
    # reference sheet at z0 split off under the port, on the side away from the trace, so that the
    # face port has an edge to end on there (the copper is unchanged)
    ports = case["ports"]
    faces = {}  # port index -> {"trace": sheet index, "plane": sheet index, "patch": rect, "mid": (x, y)}
    if port_mode == "face":
        for k, p in enumerate(ports):
            f = face_geometry(case, p)
            if f:
                faces[k] = f
    patches = {}  # sheet index -> [(port index, rect)]
    for k, f in faces.items():
        patches.setdefault(f["plane"], []).append((k, f["patch"]))

    sheet_names = []
    for si, s in enumerate(case.get("sheets", [])):
        nm = f"sheet{si}"
        sheet_names.append(nm)
        r = s["rect"]
        b = [max(r[0], dom[0]), max(r[1], dom[1]), min(r[2], dom[3]), min(r[3], dom[4])]
        if b[2] - b[0] <= 1e-9 or b[3] - b[1] <= 1e-9:  # entirely outside the domain
            continue
        rings = [lp for rg in s.get("polys", []) for lp in (split_ring(rg) if simple_clip else boundary_loops(rg))]
        outside = False
        if rings:
            if simple_clip:
                depth = [sum(inside_ring(rg[0][0], rg[0][1], o) for o in rings if o is not rg) for rg in rings]
            else:
                ips = [interior_point(rg) for rg in rings]
                depth = [sum(inside_ring(ip[0], ip[1], o) for o in rings if o is not rg) for ip, rg in zip(ips, rings)]
            first = True
            for d in sorted(set(depth)):
                for rg, dd in zip(rings, depth):
                    if dd != d:
                        continue
                    inside = all(b[0] - 1e-9 <= x <= b[2] + 1e-9 and b[1] - 1e-9 <= y <= b[3] + 1e-9 for x, y in rg)
                    xs, ys = [p[0] for p in rg], [p[1] for p in rg]
                    if not simple_clip and (min(xs) >= b[2] or max(xs) <= b[0] or min(ys) >= b[3] or max(ys) <= b[1]):
                        continue  # entirely outside the domain
                    if inside:  # the ring as it is
                        subs = [rg]
                    elif simple_clip:
                        # clipping at the board edge can make a ring touch itself again: split once more
                        clipped = clip_ring(rg, *b)
                        subs = split_ring(clipped) if len(clipped) >= 3 else []
                    else:  # unclipped; the sheet is cut to the domain by a Boolean intersection below
                        subs = [rg]
                        outside = True
                    for sub in subs:
                        pts = clean_ring(sub)
                        if pts is None:
                            continue
                        part = m.name(f"{nm}_r")
                        code = m.poly_sheet("metal", nm if first else part, pts, s["z"])
                        if not first:
                            code += (f'Solid.Subtract "metal:{nm}", "metal:{part}"\n' if d % 2 else f'Solid.Add "metal:{nm}", "metal:{part}"\n')
                        first = False
                        m.add(f"{nm} ring", code)
            if outside and not first:
                cl = m.name(f"{nm}_clip")
                m.add(f"{nm} clip to the domain", m.rect_sheet("metal", cl, *b, s["z"]) + f'Solid.Intersect "metal:{nm}", "metal:{cl}"\n')
            if first:  # no copper of this sheet inside the domain
                continue
        else:
            m.add(f"{nm}", m.rect_sheet("metal", nm, *b, s["z"]))
        for h in s.get("holes", []):
            hn = m.name(f"{nm}_h")
            m.add(f"{nm} hole", m.hole_sheet("metal", hn, h, s["z"]) + f'Solid.Subtract "metal:{nm}", "metal:{hn}"\n')
        for k, rr in patches.get(si, []):
            pn = f"portref{k + 1}"
            cut = m.name(f"{nm}_p")
            m.add(f"{nm} port patch {k + 1}", m.rect_sheet("metal", cut, *rr, s["z"]) + f'Solid.Subtract "metal:{nm}", "metal:{cut}"\n'
                  + m.rect_sheet("metal", pn, *rr, s["z"]))

    for vi, v in enumerate(case.get("vias", [])):
        assert not v.get("outer"), "outer walls: kisi-fdtd only"
        vn = f"via{vi}"
        m.add(vn, f"""
With Cylinder
 .Reset
 .Name "{vn}"
 .Component "metal"
 .Material "PEC"
 .OuterRadius {q(v['r'])}
 .InnerRadius "0"
 .Axis "z"
 .Zrange {q(v['z0'])}, {q(v['z1'])}
 .Xcenter {q(v['x'])}
 .Ycenter {q(v['y'])}
 .Segments "0"
 .Create
End With""")
        # cut the barrel out of the dielectric it crosses
        ins = "".join(f'Solid.Insert "board:layer{n}", "metal:{vn}"\n' for n, l in enumerate(case["layers"])
                      if l["z0"] < v["z1"] - 1e-9 and l["z1"] > v["z0"] + 1e-9)
        if ins:
            m.add(f"{vn} insert", ins)
        for pz in v.get("pads", []):
            pn = m.name(f"{vn}_pad")
            m.add(f"{vn} pad", m.circle_sheet("metal", pn, v["x"], v["y"], v["pad_r"], pz))

    # lumped series R-L-C; kisi-fdtd's zeros mean "absent" (l = 0: no inductance, c = 0: shorted capacitor)
    for n, e in enumerate(case.get("lumped", [])):
        p1, p2 = span_points(e)
        m.add(f"lumped {n}", f"""
With LumpedElement
 .Reset
 .SetName "lumped{n}"
 .Folder ""
 .SetType "RLCSerial"
 .SetR {q(max(e.get('r', 0.0), 1e-6))}
 .SetL {q(e.get('l', 0.0))}
 .SetC {q(e.get('c', 0.0))}
 .SetGs "0"
 .SetI0 "1e-14"
 .SetT "300"
 .SetP1 "False", {q(p1[0])}, {q(p1[1])}, {q(p1[2])}
 .SetP2 "False", {q(p2[0])}, {q(p2[1])}, {q(p2[2])}
 .SetInvert "False"
 .SetMonitor "True"
 .SetRadius "0.0"
 .Wire ""
 .Position "end1"
 .Create
End With""")

    # ports along z from z0 (reference) to z1 (trace)
    for k, p in enumerate(ports):
        z0 = p.get("r", 50.0)
        if k in faces:
            xm, ym = faces[k]["mid"]
            trace = sheet_names[faces[k]["trace"]]
            m.add(f"port {k + 1} (face)", f"""
Pick.PickEdgeFromPoint "metal:{trace}", {q(xm)}, {q(ym)}, {q(p['z1'])}
Pick.PickEdgeFromPoint "metal:portref{k + 1}", {q(xm)}, {q(ym)}, {q(p['z0'])}
With DiscreteFacePort
 .Reset
 .PortNumber "{k + 1}"
 .Type "SParameter"
 .Label ""
 .Folder ""
 .Impedance {q(z0)}
 .VoltageAmplitude "1.0"
 .CurrentAmplitude "1.0"
 .Monitor "True"
 .CenterEdge "True"
 .SetP1 "True", {q(xm)}, {q(ym)}, {q(p['z0'])}
 .SetP2 "True", {q(xm)}, {q(ym)}, {q(p['z1'])}
 .LocalCoordinates "False"
 .InvertDirection "False"
 .UseProjection "False"
 .ReverseProjection "False"
 .Create
End With""", "port")
        else:
            p1, p2 = span_points(p)
            m.add(f"port {k + 1} (edge)", discrete_edge_port(k + 1, z0, p1, p2), "port")

    # mesh and solver
    m.add("mesh", f"""
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
 .Set "EdgeRefinementRatio", "4"
 .Set "EdgeRefinementBufferLines", "3"
End With""", "mesh")
    m.add("solver", f"""
With Solver
 .Method "Hexahedral"
 .CalculationType "TD-S"
 .StimulationPort "All"
 .StimulationMode "All"
 .SteadyStateLimit {q(case.get('end_db', -50))}
 .MeshAdaption "False"
 .AutoNormImpedance "True"
 .NormingImpedance {q(ports[0].get('r', 50.0))}
 .CalculateModesOnly "False"
 .SParaSymmetry "False"
 .StoreTDResultsInCache "False"
 .FullDeembedding "False"
 .SuperimposePLWExcitation "False"
 .UseSensitivityAnalysis "False"
End With""", "setup")
    return m.blocks


def copper_at(s, x, y):
    """is the sheet copper at (x, y) (rect, holes, even-odd polygon rings)?"""
    r = s["rect"]
    if not (r[0] - 1e-9 <= x <= r[2] + 1e-9 and r[1] - 1e-9 <= y <= r[3] + 1e-9):
        return False
    if s.get("polys"):
        if sum(inside_ring(x, y, rg) for rg in s["polys"]) % 2 == 0:
            return False
    for h in s.get("holes", []):
        if len(h) < 5:
            if math.hypot(x - h[0], y - h[1]) < h[2]:
                return False
        else:
            x0, y0, x1, y1, rr = h
            dx, dy = x1 - x0, y1 - y0
            t = max(0.0, min(1.0, ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy or 1.0)))
            if math.hypot(x - x0 - t * dx, y - y0 - t * dy) < rr:
                return False
    return True


def face_geometry(case, p, w=0.05):
    """trace sheet, reference sheet and patch rectangle of a face port, or None (edge port)"""
    if p.get("x1") is not None:  # across a trace along y: the port line runs in x at y0
        a0, a1, c, axis = min(p["x"], p["x1"]), max(p["x"], p["x1"]), p["y0"], 1
    else:  # across a trace along x: the port line runs in y at x
        a0, a1, c, axis = p["y0"], p["y1"], p["x"], 0
    if a1 - a0 < 1e-9:
        return None
    am = (a0 + a1) / 2
    pt = (lambda along, normal: (normal, along)) if axis == 0 else (lambda along, normal: (along, normal))
    trace = side = None
    for si, s in enumerate(case["sheets"]):
        if abs(s["z"] - p["z1"]) > 1e-9:
            continue
        for sg in (1.0, -1.0):  # copper on one side of the port line only: the trace ends there
            if copper_at(s, *pt(am, c + sg * 0.02)) and not copper_at(s, *pt(am, c - sg * 0.02)):
                trace, side = si, sg
                break
        if trace is not None:
            break
    if trace is None:
        return None
    lo, hi = sorted((c, c - side * w))
    patch = [lo, a0, hi, a1] if axis == 0 else [a0, lo, a1, hi]
    for si, s in enumerate(case["sheets"]):
        if abs(s["z"] - p["z0"]) > 1e-9:
            continue
        corners = [(patch[0], patch[1]), (patch[2], patch[3]), (patch[0], patch[3]), (patch[2], patch[1]),
                   ((patch[0] + patch[2]) / 2, (patch[1] + patch[3]) / 2)]
        if all(copper_at(s, x, y) for x, y in corners):
            return {"trace": trace, "plane": si, "patch": patch, "mid": pt(am, c)}
    return None


MESH_DEFAULT = {"lpw": 20, "lpb": 20, "ratio": 20, "minstep": 0.0}


def mesh_minimal(mesh=None):
    """the mesh block reduced to the settings every CST version knows (used if the full one fails)"""
    mesh = dict(MESH_DEFAULT, **(mesh or {}))
    return f"""
With Mesh
 .MeshType "PBA"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "StepsPerWaveNear", {q(mesh['lpw'])}
 .Set "StepsPerBoxNear", {q(mesh['lpb'])}
 .Set "RatioLimitGeometry", {q(mesh['ratio'])}
End With"""


def span_points(p):
    """the two ends of a port / lumped element along z, in the middle of its width"""
    if p.get("x1") is not None:
        xc, yc = (p["x"] + p["x1"]) / 2, p["y0"]
    else:
        xc, yc = p["x"], (p["y0"] + p["y1"]) / 2
    return (xc, yc, p["z0"]), (xc, yc, p["z1"])


def discrete_edge_port(num, z0, p1, p2):
    return f"""
With DiscretePort
 .Reset
 .PortNumber "{num}"
 .Type "SParameter"
 .Label ""
 .Folder ""
 .Impedance {q(z0)}
 .VoltagePortImpedance "0.0"
 .Voltage "1.0"
 .Current "1.0"
 .Monitor "True"
 .Radius "0.0"
 .SetP1 "False", {q(p1[0])}, {q(p1[1])}, {q(p1[2])}
 .SetP2 "False", {q(p2[0])}, {q(p2[1])}, {q(p2[2])}
 .InvertDirection "False"
 .LocalCoordinates "False"
 .Wire ""
 .Position "end1"
 .Create
End With"""


def edge_fallback(case, k):
    """the discrete edge port block for port k (used when the face port cannot be built)"""
    p = case["ports"][k]
    p1, p2 = span_points(p)
    return discrete_edge_port(k + 1, p.get("r", 50.0), p1, p2)


def to_bas(blocks):
    """one VBA macro file (for running by hand in CST: Home > Macros > Run Macro)"""
    s = "' generated by kisi validation/fdtd/cst_case.py\nSub Main\n"
    for b in blocks:
        s += f"' --- {b['caption']}\n" + b["code"]
    return s + "End Sub\n"


if __name__ == "__main__":
    c = json.load(open(sys.argv[1]))
    mode = sys.argv[sys.argv.index("--port") + 1] if "--port" in sys.argv else "face"
    open(sys.argv[2], "w").write(to_bas(build(c, mode)))
    print(f"{c['name']}: {len(build(c, mode))} history blocks")
