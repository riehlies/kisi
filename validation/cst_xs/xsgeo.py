"""Cross-section geometry text (js_2d_fields custom-geometry syntax, the subset kisi writes) for the
external 2D-solver comparison: parsing, the solver domain (port size rule) and a decomposition into
non-overlapping rectangles for a 3D modeller.

Standard library only: imported by cst_xs.py, which runs in CST's Python.

Syntax handled (crates/kisi-si/src/xsgeom.rs writes nothing else):
  units mm
  bounds open open open open                 (left right top bottom)
  diel x=X w=W y=Y h=H er=E tand=T [thin=1]
  gnd|sig+|sig- [rect|trap] x=X w=W y=Y h=H [angle=A angle2=A2]
x/y are the left/bottom edge; w=inf (with x=-inf) spans the domain, w=inf from x runs to the right
wall, w=-inf from x to the left wall. Later dielectrics override earlier ones, conductors override
dielectrics. `trap` (only in CST-side texts of the etch group): a trapezoid on the base x..x+w at y,
sides leaning in by angle / angle2 degrees from the vertical (the face at y+h is narrower).
"""
import math

INF = float("inf")


def _num(v):
    v = v.strip()
    if v in ("inf", "+inf"):
        return INF
    if v == "-inf":
        return -INF
    return float(v)


def parse(text):
    """-> {"units", "bounds", "rects": [{kind, shape, x0, x1, y0, y1, er, tand, thin, angle, angle2}]}
    x0/x1/y0/y1 may be +-inf (to the domain walls)"""
    g = {"units": "mm", "bounds": ["open"] * 4, "rects": []}
    for raw in text.replace(";", "\n").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        tok = line.split()
        if tok[0] == "units":
            g["units"] = tok[1]
            continue
        if tok[0] == "bounds":
            g["bounds"] = tok[1:5]
            continue
        if tok[0] == "domain":
            continue
        if tok[0] not in ("diel", "gnd", "sig+", "sig-"):
            raise ValueError(f"unknown statement: {line}")
        shape = "rect"
        kv = {}
        for t in tok[1:]:
            if "=" not in t:
                shape = t
                continue
            k, v = t.split("=", 1)
            kv[k] = v
        x, w = _num(kv["x"]), _num(kv["w"])
        y, h = _num(kv["y"]), _num(kv["h"])

        def span(p, s):
            if math.isinf(p) and p < 0 and math.isinf(s) and s > 0:
                return -INF, INF
            if math.isinf(s):
                return (p, INF) if s > 0 else (-INF, p)
            return (p, p + s) if s >= 0 else (p + s, p)

        x0, x1 = span(x, w)
        y0, y1 = span(y, h)
        g["rects"].append({
            "kind": tok[0], "shape": shape, "x0": x0, "x1": x1, "y0": y0, "y1": y1,
            "er": float(kv.get("er", 1.0)), "tand": float(kv.get("tand", 0.0)), "thin": kv.get("thin") == "1",
            "angle": float(kv.get("angle", 0.0)), "angle2": float(kv.get("angle2", kv.get("angle", 0.0))),
        })
    return g


def fmt(v):
    s = f"{v:.6f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def _fin(vals):
    return [v for v in vals if not math.isinf(v)]


# --- the solver domain (port size rule) -----------------------------------------------------------

# Port size rule (documented in README.md): margins beyond the finite structure, in units of the
# line's height scale H (largest vertical distance from a signal conductor to a reference plane that
# covers it) and the width Ws of the signal conductor group.
MARGIN_X = 12.0   # lateral: max(MARGIN_X * H, 4 * Ws) beyond the outermost finite edge
MARGIN_Y = 12.0   # open top/bottom: max(MARGIN_Y * H, 4 * Ws) beyond the outermost finite face
MARGIN_MIN = 0.5  # mm, at least


def domain(g, scale=1.0):
    """The rectangular solver domain [x0, x1, y0, y1] for a geometry, and how it was sized.
    A side where a full-width ground plane (x=-inf w=inf) is the outermost object closes on that
    plane's outer face (the plane is a wall there); every other side gets the margin."""
    rects = g["rects"]
    sig = [r for r in rects if r["kind"] in ("sig+", "sig-")]
    gnd = [r for r in rects if r["kind"] == "gnd"]
    if not sig:
        raise ValueError("no signal conductor")
    sx0, sx1 = min(r["x0"] for r in sig), max(r["x1"] for r in sig)
    ws = sx1 - sx0
    # height scale: per signal, the distance to the nearest ground face below / above it that
    # overlaps it in x; H = the largest of these (the farther reference of a stripline)
    hs = []
    for s in sig:
        below = [s["y0"] - r["y1"] for r in gnd if r["y1"] <= s["y0"] + 1e-12 and r["x0"] < s["x1"] and r["x1"] > s["x0"]]
        above = [r["y0"] - s["y1"] for r in gnd if r["y0"] >= s["y1"] - 1e-12 and r["x0"] < s["x1"] and r["x1"] > s["x0"]]
        if below:
            hs.append(min(below))
        if above:
            hs.append(min(above))
    if not hs:  # no plane over or under the signal (coplanar only): the nearest ground in x
        hs = [min(abs(r["x0"] - s["x1"]) if r["x0"] >= s["x1"] else abs(s["x0"] - r["x1"]) for r in gnd for s in sig)]
    H = max(hs)
    mx = scale * max(MARGIN_X * H, 4 * ws, MARGIN_MIN)
    my = scale * max(MARGIN_Y * H, 4 * ws, MARGIN_MIN)
    fx = _fin([r[k] for r in rects for k in ("x0", "x1")])
    fy = _fin([r[k] for r in rects for k in ("y0", "y1")])
    x0, x1 = min(fx) - mx, max(fx) + mx
    full = [r for r in gnd if math.isinf(r["x0"]) and math.isinf(r["x1"])]
    ylo, yhi = min(fy), max(fy)
    # closes on a full plane if that plane is the lowest / highest object
    bot = [r for r in full if r["y0"] <= ylo + 1e-12]
    top = [r for r in full if r["y1"] >= yhi - 1e-12]
    y0 = ylo if bot else ylo - my
    y1 = yhi if top else yhi + my
    return {"box": [x0, x1, y0, y1], "H": H, "Ws": ws, "margin_x": mx, "margin_y": my,
            "closed_bottom": bool(bot), "closed_top": bool(top), "scale": scale}


def boxed_text(text, box, walls="gnd"):
    """The same geometry in the domain box with walls of one kind on all four sides: "gnd"
    (electric walls, as CST's electric boundaries) or "open" (js_2d_fields' natural boundary,
    zero normal flux = CST's magnetic boundaries). A side closed by a full ground plane is that
    plane either way."""
    out = []
    for line in text.splitlines():
        if line.startswith("bounds") or line.startswith("domain"):
            continue
        out.append(line)
    i = 1 if out and out[0].startswith("units") else 0
    out.insert(i, "bounds " + " ".join([walls] * 4))
    out.insert(i + 1, "domain " + " ".join(fmt(v) for v in box))
    for st in wall_extras(parse(text), box, walls):
        out.append(f"gnd x={fmt(st['x0'])} w={fmt(st['x1'] - st['x0'])} y={fmt(st['y0'])} h={fmt(st['y1'] - st['y0'])}")
    return "\n".join(out) + "\n"


# --- decomposition into non-overlapping rectangles -------------------------------------------------

def tiles(g, box, extra_gnd=()):
    """Paint the rectangles in order on the grid of all their edges (clipped to the box) and merge
    equal cells into maximal rectangles: -> [{"mat": ("diel", er, tand) | (kind, index), "x0", "x1",
    "y0", "y1", "cond": index into g["rects"] (len(g["rects"]) + k for extra_gnd[k]) or None}].
    Air is a dielectric of εr 1, so the tiles cover the box. Trapezoids are not painted (they are
    built separately and cut into the tiles). Later dielectrics override earlier ones, conductors
    override dielectrics, as in js_2d_fields."""
    X0, X1, Y0, Y1 = box
    n0 = len(g["rects"])
    rects = [(k, r) for k, r in enumerate(g["rects"]) if r["shape"] != "trap"]
    rects += [(n0 + k, dict(r, kind="gnd", shape="rect")) for k, r in enumerate(extra_gnd)]
    clip = lambda v, a, b: min(max(v, a), b)
    xs = _dedup(sorted({clip(v, X0, X1) for _, r in rects for v in (r["x0"], r["x1"])} | {X0, X1}))
    ys = _dedup(sorted({clip(v, Y0, Y1) for _, r in rects for v in (r["y0"], r["y1"])} | {Y0, Y1}))
    nx, ny = len(xs) - 1, len(ys) - 1
    air = ("diel", 1.0, 0.0)
    cell = [[air] * nx for _ in range(ny)]
    for pass_cond in (False, True):
        for k, r in rects:
            if (r["kind"] != "diel") != pass_cond:
                continue
            mat = ("diel", r["er"], r["tand"]) if r["kind"] == "diel" else (r["kind"], k)
            for j in range(ny):
                if not (r["y0"] < 0.5 * (ys[j] + ys[j + 1]) < r["y1"]):
                    continue
                for i in range(nx):
                    if r["x0"] < 0.5 * (xs[i] + xs[i + 1]) < r["x1"]:
                        cell[j][i] = mat
    # merge: runs in x per row, then stack equal runs in y
    runs = {}
    for j in range(ny):
        i = 0
        while i < nx:
            m = cell[j][i]
            k = i
            while k + 1 < nx and cell[j][k + 1] == m:
                k += 1
            runs.setdefault((i, k, m), []).append(j)
            i = k + 1
    out = []
    for (i, k, m), js in sorted(runs.items(), key=lambda t: (t[1][0], t[0][0])):
        start = prev = js[0]
        for j in js[1:] + [None]:
            if j is not None and j == prev + 1:
                prev = j
                continue
            out.append({"mat": m, "x0": xs[i], "x1": xs[k + 1], "y0": ys[start], "y1": ys[prev + 1],
                        "cond": m[1] if m[0] != "diel" else None})
            if j is not None:
                start = prev = j
    return out


def _dedup(v, tol=1e-9):
    out = []
    for x in v:
        if not out or x - out[-1] > tol:
            out.append(x)
    return out


def trap_polygon(r):
    """corner points of a trapezoid rect (base at y0)"""
    t = r["y1"] - r["y0"]
    dl = t * math.tan(math.radians(r["angle"]))
    dr = t * math.tan(math.radians(r["angle2"]))
    return [(r["x0"], r["y0"]), (r["x1"], r["y0"]), (r["x1"] - dr, r["y1"]), (r["x0"] + dl, r["y1"])]


def conductors(g):
    """signal and ground conductors with an interior point each (for port potentials)"""
    out = []
    for k, r in enumerate(g["rects"]):
        if r["kind"] == "diel":
            continue
        out.append({"index": k, "kind": r["kind"], "rect": r})
    return out


def interior_point(r, box):
    X0, X1, Y0, Y1 = box
    x0, x1 = max(r["x0"], X0), min(r["x1"], X1)
    y0, y1 = max(r["y0"], Y0), min(r["y1"], Y1)
    if r["shape"] == "trap":
        return (0.5 * (x0 + x1), y0 + 0.25 * (y1 - y0))
    # a ground spanning the box: a point near the middle of the structure, not at the wall
    xm = 0.0 if x0 < 0.0 < x1 else 0.5 * (x0 + x1)
    if math.isinf(r["x0"]) and not math.isinf(r["x1"]):
        xm = x1 - min(0.1, 0.25 * (x1 - x0))
    if math.isinf(r["x1"]) and not math.isinf(r["x0"]):
        xm = x0 + min(0.1, 0.25 * (x1 - x0))
    return (xm, 0.5 * (y0 + y1))


def touches_wall(r, box, tol=1e-9):
    X0, X1, Y0, Y1 = box
    return r["x0"] <= X0 + tol or r["x1"] >= X1 - tol or r["y0"] <= Y0 + tol or r["y1"] >= Y1 - tol


def straps(g, box, tol=1e-9):
    """Ground straps of the magnetic-wall variant: with electric walls every ground that reaches a
    side wall is connected through the wall; with magnetic (natural) walls it is not, while kisi's
    solver holds every ground at 0 V. A thin ground strip at each side wall from the lowest to the
    highest ground touching that wall connects them in both solvers alike (CST and js_2d_fields
    solve the same geometry). Returns gnd rects [{x0, x1, y0, y1}]."""
    X0, X1, Y0, Y1 = box
    d = max(1e-4, 1e-3 * (X1 - X0))
    out = []
    for side in ("left", "right"):
        touch = [r for r in g["rects"] if r["kind"] == "gnd" and (r["x0"] <= X0 + tol if side == "left" else r["x1"] >= X1 - tol)]
        if len(touch) < 2:
            continue
        y0 = max(Y0, min(r["y0"] for r in touch))
        y1 = min(Y1, max(r["y1"] for r in touch))
        out.append({"x0": X0, "x1": X0 + d, "y0": y0, "y1": y1} if side == "left" else {"x0": X1 - d, "x1": X1, "y0": y0, "y1": y1})
    return out


def isolated_grounds(g, box, tol=1e-9):
    """grounds that touch neither a wall nor a ground connected to one (kisi's solver holds every
    ground at 0 V; in a 3D modeller they float unless given a potential): indices into g["rects"]"""
    R = g["rects"]
    gnd = [k for k, r in enumerate(R) if r["kind"] == "gnd"]

    def touch(a, b):
        return a["x0"] <= b["x1"] + tol and b["x0"] <= a["x1"] + tol and a["y0"] <= b["y1"] + tol and b["y0"] <= a["y1"] + tol
    linked = {k for k in gnd if touches_wall(R[k], box, tol)}
    grow = True
    while grow:
        grow = False
        for k in gnd:
            if k not in linked and any(touch(R[k], R[j]) for j in linked):
                linked.add(k)
                grow = True
    return [k for k in gnd if k not in linked]


def ties(g, box):
    """Ground ties of the electric-wall variant: each isolated ground (isolated_grounds) joined to
    the nearer of the bottom and top walls by a thin vertical ground strip under (over) its middle,
    in both solvers alike, so it is held at 0 V as in kisi's solver. Returns gnd rects. The
    magnetic-wall variant is not built for such cases."""
    X0, X1, Y0, Y1 = box
    d = max(1e-4, 1e-3 * (X1 - X0))
    out = []
    for k in isolated_grounds(g, box):
        r = g["rects"][k]
        xm = 0.5 * (max(r["x0"], X0) + min(r["x1"], X1))
        if r["y0"] - Y0 <= Y1 - r["y1"]:
            out.append({"x0": xm - d / 2, "x1": xm + d / 2, "y0": Y0, "y1": r["y0"]})
        else:
            out.append({"x0": xm - d / 2, "x1": xm + d / 2, "y0": r["y1"], "y1": Y1})
    return out


def wall_extras(g, box, walls):
    """the ground strips added for a wall kind: straps ("magnetic"/"open") or ties ("electric"/"gnd")"""
    return straps(g, box) if walls in ("magnetic", "open") else ties(g, box)
