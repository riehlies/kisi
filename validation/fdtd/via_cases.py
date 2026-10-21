"""Reference cases for vias (validation/via_references.md, groups A–E) as kisi-fdtd cases.

Each case is a kisi-fdtd case JSON (also read by openems_case.py) with an extra "meta" block for
the comparison and the viewer: group, title, parameters, stackup and the port list. Planes reach
the domain boundary and continue through the PML (an infinite plane pair: no board-edge
resonances), copper has zero thickness, the signal vias are solid cylinders. Feeds are 50 Ω-ish
traces (closed-form widths) from 50 Ω lumped ports; for each feed layer a 2×thru companion case
gives the fixture for de-embedding (sparam.py).

Groups G (plane cavity) and H (PDN loop inductance) use 1 Ω lumped ports.

usage: python3 via_cases.py out_dir [--groups A,B,C,D,Dcap,E,G,H] [--fmax 40e9] [--dfine 0.05]
"""
import argparse, json, math, os

# copper layers top (index 0, L1) to bottom; dielectrics between them: (thickness mm, εr, tanδ)
STACKUPS = {
    "s4": {"title": "4-layer 1.6 mm FR-4", "diel": [(0.2, 4.3, 0.02), (1.2, 4.5, 0.02), (0.2, 4.3, 0.02)], "planes": [1, 2]},
    "s8": {"title": "8-layer 1.6 mm FR-4 (generic)",
           "diel": [(0.1, 3.8, 0.02), (0.2, 4.2, 0.02), (0.15, 3.8, 0.02), (0.7, 4.2, 0.02), (0.15, 3.8, 0.02), (0.2, 4.2, 0.02), (0.1, 3.8, 0.02)],
           "planes": [1, 3, 4, 6]},
    "pp100": {"title": "plane pair, 0.1 mm FR-4", "diel": [(0.1, 4.3, 0.02)], "planes": [0, 1]},
    "pp200": {"title": "plane pair, 0.2 mm FR-4", "diel": [(0.2, 4.3, 0.02)], "planes": [0, 1]},
    "ts6": {"title": "ThunderScope Rev5.3 6-layer (FR408HR / FR-4, from the board file)",
            "diel": [(0.1107, 3.23, 0.02), (0.1016, 3.66, 0.02), (0.9256, 4.5, 0.02), (0.1107, 3.66, 0.02), (0.1, 3.23, 0.02)],
            "planes": [1, 4]},
}
C0 = 299792458.0
AIR = 1.0      # air above and below the board (mm), PML outside
FEED = 4.0     # trace length from the via to the port (mm)


def copper_z(st):
    t = sum(d[0] for d in st["diel"])
    z = [t]
    for d in st["diel"]:
        z.append(z[-1] - d[0])
    return [round(v, 6) for v in z]


# --- closed-form trace widths (zero thickness) -------------------------------------------
def z_microstrip(w, h, er):
    u = w / h
    a = 1 + math.log((u ** 4 + (u / 52) ** 2) / (u ** 4 + 0.432)) / 49 + math.log(1 + (u / 18.1) ** 3) / 18.7
    b = 0.564 * ((er - 0.9) / (er + 3)) ** 0.053
    ee = (er + 1) / 2 + (er - 1) / 2 * (1 + 10 / u) ** (-a * b)
    f = 6 + (2 * math.pi - 6) * math.exp(-((30.666 / u) ** 0.7528))
    return 60 / math.sqrt(ee) * math.log(f / u + math.sqrt(1 + (2 / u) ** 2))


def z_stripline_sym(w, b, er):
    # Cohn, zero thickness, via the exact elliptic form (k = sech(πw/2b))
    k = 1 / math.cosh(math.pi * w / (2 * b))
    kp = math.sqrt(1 - k * k)
    if k < 0.7071:
        r = math.pi / math.log(2 * (1 + math.sqrt(kp)) / (1 - math.sqrt(kp)))
    else:
        r = math.log(2 * (1 + math.sqrt(k)) / (1 - math.sqrt(k))) / math.pi
    return 30 * math.pi / math.sqrt(er) * r


def width_for(z0, fz):
    lo, hi = 0.01, 3.0
    for _ in range(60):
        mid = math.sqrt(lo * hi)
        if fz(mid) > z0:
            lo = mid
        else:
            hi = mid
    return round(math.sqrt(lo * hi), 3)


def feed(st, layer):
    """(width, reference layer) of a 50 Ω trace on `layer`."""
    zc, planes, n = copper_z(st), st["planes"], len(st["diel"]) + 1
    above = [p for p in planes if p < layer]
    below = [p for p in planes if p > layer]
    ra, rb = (max(above) if above else None), (min(below) if below else None)
    er = lambda i0, i1: sum(st["diel"][i][0] * st["diel"][i][1] for i in range(i0, i1)) / sum(st["diel"][i][0] for i in range(i0, i1))
    if ra is None or rb is None:  # microstrip
        ref = rb if ra is None else ra
        h = abs(zc[layer] - zc[ref])
        e = er(min(layer, ref), max(layer, ref))
        return width_for(50, lambda w: z_microstrip(w, h, e)), ref
    h1, h2 = zc[ra] - zc[layer], zc[layer] - zc[rb]
    e = er(ra, rb)
    # offset stripline: parallel combination of the two symmetric striplines (IPC-2141 style)
    fz = lambda w: 2 * z_stripline_sym(w, 2 * h1, e) * z_stripline_sym(w, 2 * h2, e) / (z_stripline_sym(w, 2 * h1, e) + z_stripline_sym(w, 2 * h2, e))
    return width_for(50, fz), (ra if h1 <= h2 else rb)


# --- case builder --------------------------------------------------------------------------
class Builder:
    def __init__(self, stackup, half_x, half_y, fmax, dfine, board=None, x0=None, air=AIR):
        self.sk, self.st = stackup, STACKUPS[stackup]
        self.zc = copper_z(self.st)
        t = self.zc[0]
        self.dom = [-half_x if x0 is None else x0, -half_y, -air, half_x, half_y, t + air]
        self.pwr = set()
        self.lumped_meta = []
        layers, z = [], t
        for d in self.st["diel"]:
            layers.append({"z0": round(z - d[0], 6), "z1": round(z, 6), "er": d[1], "tand": d[2]})
            z -= d[0]
        self.case = {"domain": self.dom, "board": board or [self.dom[0], -half_y, half_x, half_y], "layers": layers[::-1],
                     "sheets": [], "vias": [], "ports": [], "fmax": fmax, "mesh": {"dfine": dfine, "ratio": 1.4},
                     "boundary": "pml", "pml_cells": 8, "conformal": True, "end_db": -50, "max_steps": 150000, "nf": 400}
        self.holes = {p: [] for p in self.st["planes"]}
        self.ports = []

    def antipad(self, layer, x, y, r, x1=None, y1=None):
        if layer in self.holes:
            self.holes[layer].append([x, y, r] if x1 is None else [x, y, x1, y1, r])

    def via(self, x, y, r, pad_r, l0, l1, pads, z_bottom=None, net="SIG"):
        z0 = self.zc[l1] if z_bottom is None else z_bottom
        self.case["vias"].append({"x": x, "y": y, "r": r, "z0": round(min(z0, self.zc[l0]), 6), "z1": round(max(z0, self.zc[l0]), 6),
                                  "pad_r": pad_r, "pads": [self.zc[l] for l in pads], "net": net})

    def sheet(self, z, rect, holes=(), net="SIG"):
        self.case["sheets"].append({"z": round(z, 6), "rect": rect, "holes": list(holes), "net": net})

    def cyl(self, x, y, r, z0, z1, net="SIG"):
        self.case["vias"].append({"x": x, "y": y, "r": r, "z0": round(z0, 6), "z1": round(z1, 6), "pad_r": 0, "pads": [], "net": net})

    def lumped(self, x, y, z0, z1, r=0.0, l=0.0, c=0.0, w=0.0, name=""):
        self.case.setdefault("lumped", []).append({"x": x, "y0": round(y - w / 2, 6), "y1": round(y + w / 2, 6), "z0": round(z0, 6), "z1": round(z1, 6), "r": r, "l": l, "c": c})
        self.lumped_meta.append({"name": name, "r": r, "l": l, "c": c})

    def port_raw(self, x, y, z0, z1, r, name, layer="", ref="", w=0.0):
        self.case["ports"].append({"x": x, "y0": round(y - w / 2, 6), "y1": round(y + w / 2, 6), "z0": round(z0, 6), "z1": round(z1, 6), "r": r})
        self.ports.append({"name": name, "layer": layer, "ref": ref})

    def trace(self, layer, x0, x1, y, w):
        self.case["sheets"].append({"z": self.zc[layer], "rect": [min(x0, x1), y - w / 2, max(x0, x1), y + w / 2], "net": "SIG"})

    def port(self, x, y, w, layer, ref, name):
        self.case["ports"].append({"x": x, "y0": round(y - w / 2, 6), "y1": round(y + w / 2, 6), "z0": self.zc[ref], "z1": self.zc[layer], "r": 50.0})
        self.ports.append({"name": name, "layer": f"L{layer + 1}", "ref": f"L{ref + 1}"})

    def feed_line(self, side, layer, y, x_via, name):
        """trace from the via (x_via) to a port FEED mm away on side −1 (left) or +1 (right)"""
        w, ref = feed(self.st, layer)
        xp = x_via + side * FEED
        self.trace(layer, x_via, xp, y, w)
        self.port(xp, y, w, layer, ref, name)
        return w

    def finish(self, cid, group, title, params):
        for p in self.st["planes"]:
            self.case["sheets"].append({"z": self.zc[p], "rect": self.case["board"], "holes": self.holes[p], "net": "PWR" if p in self.pwr else "GND"})
        self.case["name"] = cid
        self.case["meta"] = {"group": group, "title": title, "params": params, "stackup": self.sk, "stackup_title": self.st["title"],
                             "copper_z": self.zc, "planes": [f"L{p + 1}" for p in self.st["planes"]], "pwr_planes": [f"L{p + 1}" for p in sorted(self.pwr)],
                             "ports": self.ports, "lumped": self.lumped_meta}
        return self.case


def single_via(cid, group, title, params, stackup, l_in, l_out, drill_r, antipad_r, fmax, dfine, *, stub_to=None, backdrill=None,
               nfp=False, stitch=(), half_y=3.0, antipad_off=0.0, antipad_layers=None, pwr=(), cap=None):
    """signal via from layer l_in (feed from the left) to l_out (feed to the right); the barrel
    runs to `stub_to` (default l_out) or ends `backdrill` mm below l_out; `stitch`: GND vias (x, y)"""
    b = Builder(stackup, FEED + 1.5, half_y, fmax, dfine)
    b.pwr = set(pwr)
    pad_r = round(drill_r + 0.15, 3)
    zc = b.zc
    end = l_out if stub_to is None else stub_to
    zb = None
    if backdrill is not None:
        zb = zc[l_out] - backdrill
    pads = [l_in, l_out] + ([p for p in b.st["planes"] if l_in < p < end] if nfp else [])
    b.via(0, 0, drill_r, pad_r, l_in, end, pads, z_bottom=zb)
    for p in b.st["planes"]:
        top, bot = min(l_in, end), max(l_in, end)
        if top < p < bot or zb is not None:  # back-drilled: the drill hole stays in every plane
            r = (antipad_layers or {}).get(p, antipad_r)
            b.antipad(p, antipad_off, 0, r)
    for (x, y) in stitch:
        b.via(x, y, 0.15, 0.3, 0, len(zc) - 1, [], net="GND")
        for p in b.pwr:
            b.antipad(p, x, y, 0.35)
    if cap is not None:  # decoupling capacitor between a GND and a PWR plane: (x, y, gnd layer, pwr layer)
        cx, cy, lg_, lp = cap
        b.lumped(cx, cy, b.zc[lp], b.zc[lg_], r=0.01, l=0.6e-9, c=100e-9, name="100 nF, ESR 10 mΩ, ESL 0.6 nH (with its vias)")
    b.feed_line(-1, l_in, 0, 0, "1")
    b.feed_line(+1, l_out, 0, 0, "2")
    return b.finish(cid, group, title, params)


def thru(stackup, layer, fmax, dfine, half_y=3.0):
    b = Builder(stackup, FEED + 1.5, half_y, fmax, dfine)
    w, ref = feed(b.st, layer)
    b.trace(layer, -FEED, FEED, 0, w)
    b.port(-FEED, 0, w, layer, ref, "1")
    b.port(FEED, 0, w, layer, ref, "2")
    return b.finish(f"thru_{stackup}_L{layer + 1}", "thru", f"2×thru, L{layer + 1}, {2 * FEED:g} mm",
                    {"layer": f"L{layer + 1}", "width_mm": w, "length_mm": 2 * FEED})


def pair(cid, title, params, stackup, l_in, l_out, pitch, oval, ngnd, fmax, dfine, drill_r=0.15, antipad_r=0.4):
    b = Builder(stackup, FEED + 1.5, max(3.0, pitch / 2 + 2.5), fmax, dfine)
    pad_r = drill_r + 0.15
    ys = (pitch / 2, -pitch / 2)
    for y in ys:
        b.via(0, y, drill_r, pad_r, l_in, len(b.zc) - 1, [l_in, l_out])
    for p in b.st["planes"]:
        if l_in < p:
            if oval:
                b.antipad(p, 0, ys[0], antipad_r, 0, ys[1])
            else:
                for y in ys:
                    b.antipad(p, 0, y, antipad_r)
    gy = pitch / 2 + 0.8
    gnd = {0: [], 2: [(0, gy), (0, -gy)], 4: [(0.6, gy - 0.1), (-0.6, gy - 0.1), (0.6, -gy + 0.1), (-0.6, -gy + 0.1)]}[ngnd]
    for (x, y) in gnd:
        b.via(x, y, 0.15, 0.3, 0, len(b.zc) - 1, [], net="GND")
    # port order as kisi's .s4p: 1 = P near, 2 = N near, 3 = P far, 4 = N far
    b.feed_line(-1, l_in, ys[0], 0, "1 P near")
    b.feed_line(-1, l_in, ys[1], 0, "2 N near")
    b.feed_line(+1, l_out, ys[0], 0, "3 P far")
    b.feed_line(+1, l_out, ys[1], 0, "4 N far")
    return b.finish(cid, "E", title, params)


def plane_cavity(cid, stackup, fmax, dfine):
    """G: rectangular plane pair 50 × 30 mm (PWR on top, GND below), ports at a corner and the centre"""
    a, bb = 50.0, 30.0
    b = Builder(stackup, a / 2 + 3, bb / 2 + 3, fmax, max(dfine, 0.1), board=[-a / 2, -bb / 2, a / 2, bb / 2], air=3.0)
    b.pwr = {0}
    zc = b.zc
    b.port_raw(-22.0, -12.0, zc[1], zc[0], 1.0, "1 corner", "PWR", "GND")
    b.port_raw(0.0, 0.0, zc[1], zc[0], 1.0, "2 centre", "PWR", "GND")
    b.case.update(nf=600, f_loss=2.5e9, end_db=-50, max_steps=400000)
    d = zc[0] - zc[1]
    er = STACKUPS[stackup]["diel"][0][1]
    modes = sorted((C0 / (2 * math.sqrt(er)) * math.hypot(m / a, n / bb) * 1e3, m, n) for m in range(5) for n in range(4) if (m, n) != (0, 0))
    return b.finish(cid, "G", f"Plane pair 50 × 30 mm, d = {d:g} mm, ports at a corner and the centre",
                    {"size_mm": "50 × 30", "d_mm": d, "port_ohm": 1.0, "modes_GHz": [round(f / 1e9, 3) for f, m, n in modes if f < fmax][:8],
                     "modes_mn": [f"{m}{n}" for f, m, n in modes if f < fmax][:8]})


def pdn_loop(cid, dist, fmax, dfine):
    """H: BGA pin field (4 × 4, 0.8 mm, checkerboard PWR/GND, pins shorted per net by two die
    sheets above the board) and one 0402 capacitor `dist` mm from the field's edge, both on L1;
    GND L2/L5/L7, PWR L4 (s8). The capacitor is a short with 5 mΩ on a 0.5 mm high body loop, so
    Im(Z11)/ω is the loop inductance of the mounting and the planes."""
    xc = 1.2 + 0.4 + dist + 0.5
    b = Builder("s8", xc + 3.5, 4.0, fmax, dfine, x0=-4.0, air=1.5)
    b.pwr = {3}
    zc, top = b.zc, b.zc[0]
    gnd_planes = [p for p in b.st["planes"] if p not in b.pwr]
    zg, zp = top + 0.05, top + 0.10
    pins = [(-1.2 + 0.8 * i, -1.2 + 0.8 * j, (i + j) % 2 == 0) for i in range(4) for j in range(4)]
    g_holes, p_holes = [], []
    for (x, y, is_pwr) in pins:
        if is_pwr:
            b.cyl(x, y, 0.1, zc[3], zp, net="PWR")
            for p in gnd_planes:
                if zc[p] > zc[3]:
                    b.antipad(p, x, y, 0.3)
            g_holes.append([x, y, 0.2])
        else:
            b.cyl(x, y, 0.1, zc[6], zg, net="GND")
            for p in b.pwr:
                b.antipad(p, x, y, 0.3)
    b.sheet(zg, [-1.6, -1.6, 1.6, 1.6], g_holes, net="GND")
    b.sheet(zp, [-1.6, -1.6, 1.6, 1.6], [], net="PWR")
    b.port_raw(0.0, 0.0, zg, zp, 1.0, "1 IC pins", "PWR die", "GND die")
    # 0402: GND pad (x − 0.5) with a via to L2…L7, PWR pad (x + 0.5) with a via to L4, body loop
    xg, xp = xc - 0.5, xc + 0.5
    b.cyl(xg, 0.0, 0.1, zc[6], top, net="GND")
    b.cyl(xp, 0.0, 0.1, zc[3], top + 0.5, net="PWR")
    for p in gnd_planes:
        if zc[p] > zc[3]:
            b.antipad(p, xp, 0.0, 0.3)
    for p in b.pwr:
        b.antipad(p, xg, 0.0, 0.3)
    b.sheet(top, [xg - 0.25, -0.25, xg + 0.25, 0.25], net="GND")
    b.sheet(top, [xp - 0.25, -0.25, xp + 0.25, 0.25], net="PWR")
    b.sheet(top + 0.5, [xg - 0.15, -0.25, xp + 0.15, 0.25], net="PWR")
    b.lumped(xg, 0.0, top, top + 0.5, r=0.005, name="capacitor as a short, 5 mΩ")
    b.case.update(nf=200, f_loss=1e9, end_db=-60, max_steps=300000)
    return b.finish(cid, "H", f"PDN loop: BGA pin field to a 0402 capacitor {dist:g} mm away", {"distance_mm": dist, "pins": "4 × 4, 0.8 mm", "port_ohm": 1.0})


def ring(n, r):
    return [(round(r * math.cos(2 * math.pi * (k + 0.5) / n), 4), round(r * math.sin(2 * math.pi * (k + 0.5) / n), 4)) for k in range(n)]


def cases(groups, fmax, dfine):
    out = []
    if "A" in groups:  # coax section through a plane pair, clean return path
        for dr in (0.1, 0.15):
            for ap in (0.3, 0.4, 0.5):
                if ap <= dr + 0.1:
                    continue
                out.append(single_via(f"A_s4_d{int(dr * 2000)}_ap{int(ap * 2000)}", "A", f"L1→L4 through L2/L3, drill Ø{dr * 2:g}, antipad Ø{ap * 2:g}, 4 GND vias at 1 mm",
                                      {"drill_mm": dr * 2, "antipad_mm": ap * 2, "stitch": "4 × 1 mm"}, "s4", 0, 3, dr, ap, fmax, dfine, stitch=ring(4, 1.0)))
    if "B" in groups:  # stubs
        for dr in (0.1, 0.15):
            ap = dr + 0.25
            g = {"drill_mm": dr * 2, "antipad_mm": ap * 2, "stitch": "4 × 1 mm"}
            out.append(single_via(f"B_s8_L1L3_stub_d{int(dr * 2000)}", "B", f"L1→L3, through via (stub to L8), drill Ø{dr * 2:g}",
                                  {**g, "stub": "L3→L8"}, "s8", 0, 2, dr, ap, fmax, dfine, stub_to=7, stitch=ring(4, 1.0)))
            out.append(single_via(f"B_s8_L1L8_d{int(dr * 2000)}", "B", f"L1→L8, no stub, drill Ø{dr * 2:g}",
                                  {**g, "stub": "none"}, "s8", 0, 7, dr, ap, fmax, dfine, stitch=ring(4, 1.0)))
            out.append(single_via(f"B_s8_L1L3_bd_d{int(dr * 2000)}", "B", f"L1→L3, back-drilled to 0.2 mm below L3, drill Ø{dr * 2:g}",
                                  {**g, "stub": "0.2 mm residual"}, "s8", 0, 2, dr, ap, fmax, dfine, backdrill=0.2, stitch=ring(4, 1.0)))
        g = {"drill_mm": 0.3, "antipad_mm": 0.8, "stitch": "4 × 1 mm"}
        out.append(single_via("B_ts6_L1L3_stub", "B", "ThunderScope stack: L1→In2, through via (stub to B.Cu)", {**g, "stub": "In2→B.Cu"},
                              "ts6", 0, 2, 0.15, 0.4, fmax, dfine, stub_to=5, stitch=ring(4, 1.0)))
        out.append(single_via("B_ts6_L1L6", "B", "ThunderScope stack: L1→B.Cu, no stub", {**g, "stub": "none"},
                              "ts6", 0, 5, 0.15, 0.4, fmax, dfine, stitch=ring(4, 1.0)))
    if "C" in groups:  # antipads and pads, s8 L1→L8, drill Ø0.3
        g = {"drill_mm": 0.3, "stitch": "4 × 1 mm"}
        for ap in (0.3, 0.4, 0.5, 0.6):
            out.append(single_via(f"C_s8_ap{int(ap * 2000)}", "C", f"L1→L8, antipad Ø{ap * 2:g} on every plane", {**g, "antipad_mm": ap * 2},
                                  "s8", 0, 7, 0.15, ap, fmax, dfine, stitch=ring(4, 1.0)))
        out.append(single_via("C_s8_nfp", "C", "L1→L8, antipad Ø0.9, non-functional pads kept", {**g, "antipad_mm": 0.9, "nfp": "kept"},
                              "s8", 0, 7, 0.15, 0.45, fmax, dfine, nfp=True, stitch=ring(4, 1.0)))
        out.append(single_via("C_s8_nonfp", "C", "L1→L8, antipad Ø0.9, non-functional pads removed", {**g, "antipad_mm": 0.9, "nfp": "removed"},
                              "s8", 0, 7, 0.15, 0.45, fmax, dfine, stitch=ring(4, 1.0)))
        out.append(single_via("C_s8_offset", "C", "L1→L8, antipad Ø0.9 offset 0.1 mm from the via", {**g, "antipad_mm": 0.9, "offset_mm": 0.1},
                              "s8", 0, 7, 0.15, 0.45, fmax, dfine, antipad_off=0.1, stitch=ring(4, 1.0)))
        out.append(single_via("C_s8_mixed", "C", "L1→L8, antipad Ø1.2 on L2/L7, Ø0.8 on L4/L5", {**g, "antipad_mm": "1.2 / 0.8"},
                              "s8", 0, 7, 0.15, 0.4, fmax, dfine, antipad_layers={1: 0.6, 6: 0.6}, stitch=ring(4, 1.0)))
    if "D" in groups:  # return path: one GND stitching via at a distance, or none
        for d in (0.5, 1.0, 2.0, 5.0, None):
            st = [] if d is None else [(0.0, d)]
            out.append(single_via(f"D_s8_stitch_{'none' if d is None else str(d).replace('.', 'p')}", "D",
                                  f"L1 (ref L2) → L8 (ref L7), {'no stitching via' if d is None else f'one GND via at {d:g} mm'}",
                                  {"drill_mm": 0.3, "antipad_mm": 0.8, "stitch_mm": d}, "s8", 0, 7, 0.15, 0.4, fmax, dfine, stitch=st, half_y=6.5))
    if "E" in groups:  # differential pairs
        combos = [(0, 7, 0.8, False, 2), (0, 7, 1.0, False, 2), (0, 7, 1.2, False, 2), (0, 7, 1.0, True, 2), (0, 7, 1.0, False, 0),
                  (0, 7, 1.0, False, 4), (0, 7, 1.0, True, 4), (0, 7, 1.0, True, 0), (0, 2, 1.0, False, 2), (0, 2, 1.0, True, 2),
                  (0, 2, 0.8, False, 2), (0, 2, 1.2, False, 2)]
        for (li, lo, p, ov, ng) in combos:
            cid = f"E_s8_L1L{lo + 1}_p{int(p * 1000)}_{'oval' if ov else 'round'}_g{ng}"
            out.append(pair(cid, f"pair L1→L{lo + 1}, pitch {p:g} mm, {'one oval' if ov else 'two round'} antipad{'s' if not ov else ''}, {ng} GND vias",
                            {"pitch_mm": p, "antipad": "oval" if ov else "round", "gnd_vias": ng, "to": f"L{lo + 1}"}, "s8", li, lo, p, ov, ng, fmax, dfine))
    if "Dcap" in groups:  # return path from GND (L2) to PWR (L4) through a capacitor
        for d in (2.0, 10.0, None):
            out.append(single_via(f"D_s8_cap_{'none' if d is None else str(int(d))}", "D",
                                  f"L1 (ref L2 GND) → L3 (ref L4 PWR), {'no capacitor' if d is None else f'capacitor GND–PWR at {d:g} mm'}",
                                  {"drill_mm": 0.3, "antipad_mm": 0.8, "cap_mm": d, "stitch": "4 × 1 mm (GND only)"}, "s8", 0, 2, 0.15, 0.4, fmax, dfine,
                                  stitch=ring(4, 1.0), pwr=(3,), cap=None if d is None else (0.0, d, 1, 3), half_y=(d or 2.0) + 1.5))
    if "G" in groups:
        for sk in ("pp100", "pp200"):
            out.append(plane_cavity(f"G_{sk}", sk, 6e9, dfine))
    if "H" in groups:
        for d in (2.0, 5.0, 10.0, 20.0):
            out.append(pdn_loop(f"H_s8_cap_{int(d)}", d, 2e9, max(dfine, 0.07)))
    # 2×thru companions of every feed layer used
    feeds = {}
    for c in out:
        for p in c["meta"]["ports"]:
            if not p["layer"].startswith("L"):
                continue
            feeds[(c["meta"]["stackup"], int(p["layer"][1:]) - 1)] = c["domain"][4]
    for (sk, layer), hy in sorted(feeds.items()):
        out.append(thru(sk, layer, fmax, dfine, 3.0))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--groups", default="A,B,C,D,Dcap,E,G,H")
    ap.add_argument("--fmax", type=float, default=40e9)
    ap.add_argument("--dfine", type=float, default=0.05)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    cs = cases(set(a.groups.split(",")), a.fmax, a.dfine)
    for c in cs:
        json.dump(c, open(os.path.join(a.out, c["name"] + ".json"), "w"), indent=1)
    print(f"{len(cs)} cases in {a.out}")


if __name__ == "__main__":
    main()
