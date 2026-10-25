"""A reference case (via_cases.py) as a minimal .kicad_pcb, so that kisi itself (its via and line
models, through the normal board pipeline) can be compared with the 3D result.

Same stackup (copper 5 µm, the 3D model has zero thickness), planes as filled zones with the
antipads cut in (fractured polygons, the cut runs radially from each hole to the board edge),
signal vias with pads only where the 3D model has them (remove_unused_layers, no end layers), back-drilling as a KiCad 10 `backdrill`,
GND vias, traces and one pad at every port: U1 at port 1 (start), J1 at port 2. Pairs: nets D0_P /
D0_N, U1 / J1 carry both. Capacitors (group D) are not modelled by kisi's via model and are left
out; capacitors between two planes (`meta.caps`, the finite-board cases) become 0402 parts on F.Cu.

usage: python3 kisi_board.py case.json out.kicad_pcb
"""
import json, math, sys

CU_T = 0.005


def layer_names(n):
    return ["F.Cu"] + [f"In{k}.Cu" for k in range(1, n - 1)] + ["B.Cu"]


def kicad_ids(n):
    return [0] + list(range(1, n - 1)) + [31]


def circle(cx, cy, r, n=48, start=-math.pi / 2):
    return [(cx + r * math.cos(start + 2 * math.pi * k / n), cy + r * math.sin(start + 2 * math.pi * k / n)) for k in range(n + 1)]


def stadium(h, n=24):
    x0, y0, x1, y1, r = h
    a = math.atan2(y1 - y0, x1 - x0)
    pts = [(x1 + r * math.cos(a - math.pi / 2 + math.pi * k / n), y1 + r * math.sin(a - math.pi / 2 + math.pi * k / n)) for k in range(n + 1)]
    pts += [(x0 + r * math.cos(a + math.pi / 2 + math.pi * k / n), y0 + r * math.sin(a + math.pi / 2 + math.pi * k / n)) for k in range(n + 1)]
    return pts + [pts[0]]


def hole_ring(h, top):
    """closed outline of a hole starting at its lowest (top=False) or highest point"""
    if h[0] == "rect":
        pts = [tuple(p) for p in h[1]]
    else:
        pts = (circle(h[0], h[1], h[2]) if len(h) < 5 else stadium(h))[:-1]
    k = max(range(len(pts)), key=lambda i: pts[i][1]) if top else min(range(len(pts)), key=lambda i: pts[i][1])
    pts = pts[k:] + pts[:k]
    return pts + [pts[0]]


def fractured(rect, holes):
    """outer rectangle with holes; each hole joined by a vertical cut to the bottom edge (holes
    at y ≤ 0) or the top edge"""
    x0, y0, x1, y1 = rect
    cx = lambda h: (sum(p[0] for p in h[1]) / len(h[1]), sum(p[1] for p in h[1]) / len(h[1])) if h[0] == "rect" else (h[0], h[1])
    low = sorted([h for h in holes if cx(h)[1] <= 0], key=lambda h: cx(h)[0])
    high = sorted([h for h in holes if cx(h)[1] > 0], key=lambda h: -cx(h)[0])
    pts = [(x0, y0)]
    for h in low:  # along the bottom edge, left to right
        r = hole_ring(h, False)
        pts += [(r[0][0], y0)] + r[::-1] + [(r[0][0], y0)]
    pts += [(x1, y0), (x1, y1)]
    for h in high:  # along the top edge, right to left
        r = hole_ring(h, True)
        pts += [(r[0][0], y1)] + r[::-1] + [(r[0][0], y1)]
    pts += [(x0, y1)]
    return pts


def main():
    c = json.load(open(sys.argv[1]))
    m = c["meta"]
    zc = m["copper_z"]
    n = len(zc)
    names, ids = layer_names(n), kicad_ids(n)
    zl = lambda z: min(range(n), key=lambda i: abs(zc[i] - z))
    lay = sorted(c["layers"], key=lambda l: -l["z1"])
    out = ['(kicad_pcb (version 20240108) (generator "kisi-validation")', f"  (general (thickness {zc[0] + n * CU_T:.4f}))",
           "  (layers " + " ".join(f'({i} "{nm}" signal)' for i, nm in zip(ids, names)) + ")", "  (setup (stackup"]
    for i in range(n):
        out.append(f'    (layer "{names[i]}" (type "copper") (thickness {CU_T}))')
        if i < n - 1:
            l = lay[i]
            out.append(f'    (layer "dielectric {i + 1}" (type "core") (thickness {l["z1"] - l["z0"]:.4f}) (material "FR4") (epsilon_r {l["er"]}) (loss_tangent {l.get("tand", 0)}))')
    out.append("  ))")
    four = len(c["ports"]) == 4
    pair = m["group"] == "E" or (m["group"] in ("T", "P") and four and "diff" in c["name"])
    # differential pairs: D0_P above, D0_N below; coupled single lines: aggressor A1 (port 1/3), victim V1
    sig = ["D0_P", "D0_N"] if pair else (["A1", "V1"] if four else ["SIG"])
    nets = [""] + sig + ["GND", "PWR"]
    out.append("  " + " ".join(f'(net {i} "{nm}")' for i, nm in enumerate(nets)))
    nid = {nm: i for i, nm in enumerate(nets)}
    # which signal net a y position belongs to (pairs: P above, N below)
    # net of a trace or via: from the port on the same line (port k → line k mod 2), else nearest y
    table = {}
    for k, p in enumerate(c["ports"]):
        table[(round((p["y0"] + p["y1"]) / 2, 4), zl(p["z1"]))] = sig[k % len(sig)]

    def snet(y, L=None):
        if len(sig) == 1:
            return sig[0]
        if L is not None and (round(y, 4), L) in table:
            return table[(round(y, 4), L)]
        best = min(table.items(), key=lambda kv: abs(kv[0][0] - y))
        return best[1]
    # ports → pads: port 1 (and 2 for pairs) at the start (U1), the others at the end (J1)
    nports = len(c["ports"])
    start = [0, 1] if nports == 4 else [0]
    fps = {"U1": [], "J1": []}
    for k, p in enumerate(c["ports"]):
        y = (p["y0"] + p["y1"]) / 2
        L = zl(p["z1"])
        fps["U1" if k in start else "J1"].append((p["x"], y, L, snet(y, L), max(p["y1"] - p["y0"], 0.1)))
    for ref, pads in fps.items():
        for j, (x, y, L, net, w) in enumerate(pads):
            out.append(f'  (footprint "{ref}" (layer "F.Cu") (at {x:.4f} {-y:.4f}) (property "Reference" "{ref}") (property "Value" "{"IC" if ref == "U1" else "conn"}")')
            out.append(f'    (pad "{j + 1}" smd rect (at 0 0) (size {w:.4f} {w:.4f}) (layers "{names[L]}") (net {nid[net]} "{net}")))')
    # pads and gold fingers (group P: `"pad": true`): SMD pads of a footprint, the traces end at their centre
    for i, s in enumerate(x for x in c["sheets"] if x.get("pad")):
        x0, y0, x1, y1 = s["rect"]
        xc, yc = (x0 + x1) / 2, (y0 + y1) / 2
        L = zl(s["z"])
        out.append(f'  (footprint "TP" (layer "F.Cu") (at {xc:.4f} {-yc:.4f}) (property "Reference" "TP{i + 1}") (property "Value" "pad")')
        out.append(f'    (pad "1" smd rect (at 0 0) (size {x1 - x0:.4f} {y1 - y0:.4f}) (layers "{names[L]}") (net {nid[snet(yc, L)]} "{snet(yc, L)}")))')
    # traces (KiCad y points down: mirror)
    for s in c["sheets"]:
        if s.get("net") in ("GND", "PWR") or s.get("pad"):
            continue
        x0, y0, x1, y1 = s["rect"]
        L = zl(s["z"])
        w = y1 - y0
        if w <= 0 or x1 - x0 <= 0:
            continue
        yc = (y0 + y1) / 2
        out.append(f'  (segment (start {x0:.4f} {-yc:.4f}) (end {x1:.4f} {-yc:.4f}) (width {w:.4f}) (layer "{names[L]}") (net {nid[snet(yc, L)]}))')
    # coplanar pours and planes as zones
    for s in c["sheets"]:
        net = s.get("net")
        if net not in ("GND", "PWR"):
            continue
        L = zl(s["z"])
        r = s["rect"]
        # rectangular cut-outs (`polys`: the outline, then the cuts) as holes of the fill
        cuts = [q for q in s.get("polys", [])[1:]]
        pts = fractured(r, s.get("holes", []) + [["rect", q] for q in cuts])
        out.append(f'  (zone (net {nid[net]}) (net_name "{net}") (layer "{names[L]}") (filled_polygon (layer "{names[L]}") (pts ' +
                   " ".join(f"(xy {x:.4f} {-y:.4f})" for x, y in pts) + ")))")
    # vias
    for v in c["vias"]:
        if v.get("outer"):
            continue
        net = v.get("net", "SIG")
        net = snet(v["y"]) if net == "SIG" else net
        la, lb = zl(v["z1"]), zl(v["z0"])
        bd = ""
        if net in sig and abs(zc[lb] - v["z0"]) > 0.02:  # barrel ends between layers: back-drilled from the bottom
            lb_keep = min(i for i in range(n) if zc[i] < v["z0"] - 1e-9)  # first copper below the stub end
            bd = f' (backdrill (size {2 * v["r"] + 0.2:.3f}) (layers "{names[n - 1]}" "{names[lb_keep]}"))'
            lb = n - 1
        size = 2 * max(v.get("pad_r", 0), v["r"] + 0.05)
        out.append(f'  (via (at {v["x"]:.4f} {-v["y"]:.4f}) (size {size:.4f}) (drill {2 * v["r"]:.4f}) (layers "{names[la]}" "{names[lb]}") (remove_unused_layers yes) (keep_end_layers no) (net {nid[net]}){bd})')
    # capacitors between two planes (meta.caps of the resonance and return cases): a 0402 part on
    # F.Cu with one pad on each plane net (the via model counts them; their parasitics come from
    # data/components.json, not from the 3D case)
    na, nb = ("GND", "PWR") if any(s.get("net") == "PWR" for s in c["sheets"]) else ("GND", "GND")
    for i, cp in enumerate(m.get("caps", [])):
        x, y = cp["at"]
        out.append(f'  (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "F.Cu") (at {x:.4f} {-y:.4f}) (property "Reference" "C{i + 1}") (property "Value" "{cp["c"] * 1e9:g}n")')
        out.append(f'    (pad "1" smd rect (at -0.48 0) (size 0.56 0.62) (layers "F.Cu") (net {nid[na]} "{na}"))')
        out.append(f'    (pad "2" smd rect (at 0.48 0) (size 0.56 0.62) (layers "F.Cu") (net {nid[nb]} "{nb}")))')
    out.append(")")
    open(sys.argv[2], "w").write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
