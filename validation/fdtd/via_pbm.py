"""Physics-based reduced via model (prototype), compared with the de-embedded 3D references.

A via is a ladder along its barrel:
* nodes: the planes it crosses and the signal layers where traces join it (entry, exit);
* shunt at every node: the via–plate capacitance, from an axisymmetric r–z Laplace solve
  (barrel and pads at 1 V, planes with their antipads at 0 V, layered εr); the charge on the
  barrel is split between the nodes at the midpoints;
* series between two nodes inside a plane pair: the parallel-plate (radial TEM) impedance of the
  barrel, Z = ωμd/4 · H0⁽²⁾(k a), with the GND vias of that plane pair as shorted ports
  (Z_in = Z00 − z0r Zrr⁻¹ zr0). Without a nearby return via the real part is the radial wave
  that leaves into the plane pair;
* above the top plane and below the bottom plane: no series element (part of the trace);
* stubs: the ladder below the exit node ends open;
* the trace over the antipad (from the pad to the plane opening) as a short line over the next
  plane.
Ports at the via: compared with the 3D result de-embedded by the 2×thru of each feed layer.

usage: python3 via_pbm.py [case ids]   → results/via_pbm.json, via_ref/<case>/pbm.s2p
"""
import json, math, os, sys
import numpy as np
import scipy.sparse as sp
import scipy.sparse.linalg as spla
from scipy.special import hankel2

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from via_cases import z_microstrip, z_stripline_sym  # noqa: E402

EPS0, MU0, C0 = 8.8541878128e-12, 4e-7 * math.pi, 299792458.0
SHARED = True  # trace joining inside a plane pair: weighted cavity groups
OUTER_L = True  # barrel inductance between a surface layer and the outermost plane
SUB = 0.2  # largest span between ladder nodes inside a plane pair (mm)
F_LOSS = None  # set per case: the 3D references fix σ at f_loss, so tanδ_eff = tanδ · f_loss / f


def loss_tan(tand, f):
    return tand * (F_LOSS / f if F_LOSS else 1.0)


def er_at(case, z):
    for l in case["layers"]:
        if l["z0"] - 1e-9 <= z <= l["z1"] + 1e-9:
            return l["er"], l.get("tand", 0.0)
    return 1.0, 0.0


def geometry(case, via):
    """planes the barrel region sees: (z, antipad radius) around this via; pads"""
    planes = []
    for s in case["sheets"]:
        if s.get("net") not in ("GND", "PWR"):
            continue
        b = None
        for h in s.get("holes", []):
            if len(h) < 5 and math.hypot(h[0] - via["x"], h[1] - via["y"]) < 1e-6:
                b = h[2]
            elif len(h) >= 5:
                # slot (one antipad for a pair): the via lies on its axis, the nearest edge is r away
                x0, y0, x1, y1, r = h
                dx, dy = x1 - x0, y1 - y0
                l2 = dx * dx + dy * dy
                t = max(0.0, min(1.0, ((via["x"] - x0) * dx + (via["y"] - y0) * dy) / l2)) if l2 else 0.0
                if math.hypot(via["x"] - x0 - t * dx, via["y"] - y0 - t * dy) < 1e-6:
                    b = r
        if b is None:
            # offset or slot antipads: radius of the circle centred on the via that fits inside
            for h in s.get("holes", []):
                if len(h) < 5:
                    d = math.hypot(h[0] - via["x"], h[1] - via["y"])
                    if d < h[2]:
                        b = h[2] - d  # conservative: nearest edge
        planes.append((s["z"], b))
    return sorted(planes, key=lambda p: -p[0])


def rz_charges(case, via, dr=0.01, rmax=3.0):
    """r–z Laplace solve; returns z of the conductor-1 nodes and their charges (F per V)"""
    planes = geometry(case, via)
    zc = sorted({round(p[0], 6) for p in planes} | {round(z, 6) for z in via["pads"]} | {round(via["z0"], 6), round(via["z1"], 6)} |
                {round(l["z0"], 6) for l in case["layers"]} | {round(l["z1"], 6) for l in case["layers"]})
    zlo, zhi = min(zc) - 0.5, max(zc) + 0.5
    fixed = sorted(set(zc) | {zlo, zhi})
    zs = [fixed[0]]
    for a, b in zip(fixed, fixed[1:]):
        n = max(1, int(math.ceil((b - a) / dr - 1e-9)))
        zs += [a + (b - a) * k / n for k in range(1, n + 1)]
    z = np.array(zs)
    r = np.arange(0, rmax + dr / 2, dr)
    nr, nz = len(r), len(z)
    idx = lambda i, j: j * nr + i
    # conductors: 1 = via (barrel, pads), 0 = planes
    cond = np.full(nr * nz, -1, int)
    a = via["r"]
    for j, zz in enumerate(z):
        for i, rr in enumerate(r):
            if via["z0"] - 1e-9 <= zz <= via["z1"] + 1e-9 and rr <= a + 1e-9:
                cond[idx(i, j)] = 1
            if any(abs(zz - pz) < 1e-6 for pz in via["pads"]) and rr <= via["pad_r"] + 1e-9:
                cond[idx(i, j)] = 1
            for (pz, b) in planes:
                if abs(zz - pz) < 1e-6 and (b is None or rr >= b - 1e-9):
                    cond[idx(i, j)] = 0
    # permittivity of the cell between rows j and j+1
    ecell = np.array([er_at(case, 0.5 * (z[j] + z[j + 1]))[0] for j in range(nz - 1)])
    rows, cols, vals = [], [], []
    links = []  # (p, q, G)
    for j in range(nz):
        for i in range(nr):
            p = idx(i, j)
            if i + 1 < nr:  # radial link on row j
                e = np.mean([ecell[jj] for jj in (j - 1, j) if 0 <= jj < nz - 1])
                dzh = 0.5 * ((z[j + 1] - z[j]) if j + 1 < nz else 0) + 0.5 * ((z[j] - z[j - 1]) if j > 0 else 0)
                G = EPS0 * e * 2 * math.pi * (r[i] + dr / 2) * dzh / dr * 1e-3
                links.append((p, idx(i + 1, j), G))
            if j + 1 < nz:  # axial link in column i
                area = math.pi * ((r[i] + dr / 2) ** 2 - max(r[i] - dr / 2, 0) ** 2) if i > 0 else math.pi * (dr / 2) ** 2
                if i == nr - 1:
                    area = math.pi * (r[i] ** 2 - (r[i] - dr / 2) ** 2)
                G = EPS0 * ecell[j] * area / (z[j + 1] - z[j]) * 1e-3
                links.append((p, idx(i, j + 1), G))
    free = np.where(cond < 0)[0]
    fmap = -np.ones(nr * nz, int)
    fmap[free] = np.arange(len(free))
    phi_fix = np.where(cond == 1, 1.0, 0.0)
    rhs = np.zeros(len(free))
    diag = np.zeros(len(free))
    for (p, q, G) in links:
        for (u, w) in ((p, q), (q, p)):
            if fmap[u] >= 0:
                diag[fmap[u]] += G
                if fmap[w] >= 0:
                    rows.append(fmap[u]); cols.append(fmap[w]); vals.append(-G)
                else:
                    rhs[fmap[u]] += G * phi_fix[w]
    A = sp.csr_matrix((np.concatenate([vals, diag]), (np.concatenate([rows, np.arange(len(free))]), np.concatenate([cols, np.arange(len(free))]))), shape=(len(free), len(free)))
    phi = phi_fix.copy()
    phi[free] = spla.spsolve(A.tocsc(), rhs)
    q = np.zeros(nr * nz)
    for (p, q_, G) in links:
        d = G * (phi[p] - phi[q_])
        q[p] += d
        q[q_] -= d
    on = np.where(cond == 1)[0]
    return np.array([z[k // nr] for k in on]), q[on]


def feed_line(case, planes, w, zt, zref):
    """impedance and εeff of the feed trace: offset stripline (IPC-2141 style, as the case
    generator) between the nearest planes above and below, else microstrip over the reference"""
    pz = [p[0] for p in planes]
    up = [z for z in pz if z > zt + 1e-9]
    dn = [z for z in pz if z < zt - 1e-9]
    num = den = 0.0
    if up and dn:
        za, zb = min(up), max(dn)
        for l in case["layers"]:
            o = max(0.0, min(l["z1"], za) - max(l["z0"], zb))
            num += o * l["er"]; den += o
        er = num / den
        h1, h2 = za - zt, zt - zb
        z1, z2 = z_stripline_sym(w, 2 * h1, er), z_stripline_sym(w, 2 * h2, er)
        return 2 * z1 * z2 / (z1 + z2), er
    h = abs(zt - zref)
    er, _ = er_at(case, (zt + zref) / 2)
    return z_microstrip(w, h, er), feed_eeff(w, h, er, case, zt, zref)


def feed_eeff(w, h, er, case, zt, zref):
    """effective εr of the feed: microstrip (Hammerstad) when nothing is above the trace, else
    the εr of the dielectric (stripline)"""
    above = [l for l in case["layers"] if l["z0"] >= max(zt, zref) - 1e-9]
    if above and zt < zref or (zt > zref and [l for l in case["layers"] if l["z0"] >= zt - 1e-9]):
        return er
    u = w / h
    a = 1 + math.log((u ** 4 + (u / 52) ** 2) / (u ** 4 + 0.432)) / 49 + math.log(1 + (u / 18.1) ** 3) / 18.7
    b = 0.564 * ((er - 0.9) / (er + 3)) ** 0.053
    return (er + 1) / 2 + (er - 1) / 2 * (1 + 10 / u) ** (-a * b)


def zpp(f, d, er, tand, a, returns):
    """parallel-plate impedance of a barrel (radius a, m) in a plane pair of height d (m) with
    shorted return vias [(distance m, radius m)]"""
    w = 2 * math.pi * f
    k = w * math.sqrt(MU0 * EPS0 * er) * np.sqrt(1 - 1j * tand)
    pre = w * MU0 * d / 4
    pts = [(0.0, a)] + returns
    n = len(pts)
    Z = np.empty((n, n), complex)
    # positions: signal at origin, returns on a ray each (only distances to the signal and between
    # returns matter; returns are passed with their coordinates)
    for i in range(n):
        for j in range(n):
            if i == j:
                Z[i, j] = pre * hankel2(0, k * pts[i][1])
            else:
                Z[i, j] = pre * hankel2(0, k * dist(pts[i], pts[j]))
    if n == 1:
        return Z[0, 0]
    z0r = Z[0, 1:]
    Zrr = Z[1:, 1:]
    return Z[0, 0] - z0r @ np.linalg.solve(Zrr, Z[1:, 0])


def dist(p, q):
    return math.hypot(p[2] - q[2], p[3] - q[3]) if len(p) > 2 and len(q) > 2 else abs(p[0] - q[0])


def ladder(case, f, via_index=0, loss_as_3d=True, mode=None):
    """mode None: single via; "odd" / "even": the pair (vias 0 and 1), per line"""
    global F_LOSS
    F_LOSS = (case.get("f_loss") or case["fmax"] / 2) if loss_as_3d else None
    via = [v for v in case["vias"] if v.get("net", "SIG") == "SIG"][via_index]
    planes = geometry(case, via)
    ports = case["ports"]
    if len(ports) == 4:  # pair: P near, N near, P far, N far
        ports = [ports[0], ports[2]]
    z_in, z_out = ports[0]["z1"], ports[1]["z1"]
    sigs = [v for v in case["vias"] if v.get("net", "SIG") == "SIG"]
    partner = sigs[1] if mode and len(sigs) > 1 else None
    crossed = [p for p in planes if via["z0"] - 1e-9 <= p[0] <= via["z1"] + 1e-9]
    nodes = sorted({round(p[0], 6) for p in crossed if p[1] is not None} | {round(z_in, 6), round(z_out, 6)}, reverse=True)
    # thick plane pairs: intermediate nodes every ≤ 0.2 mm (distributed instead of one Pi section)
    plane_zs = [p[0] for p in planes]
    extra = []
    for za, zb in zip(nodes, nodes[1:]):
        if any(pz >= za - 1e-9 for pz in plane_zs) and any(pz <= zb + 1e-9 for pz in plane_zs):
            n = int(math.ceil((za - zb) / SUB - 1e-9))
            extra += [round(za - (za - zb) * k / n, 6) for k in range(1, n)]
    nodes = sorted(set(nodes) | set(extra), reverse=True)
    zq, qq = rz_charges(case, via)
    # node charges: nearest node in z, split at midpoints
    C = np.zeros(len(nodes))
    for zz, q in zip(zq, qq):
        k = int(np.argmin([abs(zz - n) for n in nodes]))
        C[k] += q
    if partner and mode == "odd":
        # odd mode: the partner barrel at the opposite potential adds capacitance. Two wires with a
        # far return: C_self' = 2πε/ln(R/a), C_odd' = 2πε/ln(p/a), so inside the plane pairs the
        # barrel charge is scaled by ln(R/a)/ln(p/a), R from the solved charge per length there
        pitch = math.hypot(partner["x"] - via["x"], partner["y"] - via["y"])
        rows = {}
        for zz, q in zip(zq, qq):
            rows[round(zz, 5)] = rows.get(round(zz, 5), 0.0) + q
        zr = sorted(rows)
        a_m = via["r"]
        for k in range(len(nodes)):
            zk = nodes[k]
            inside = any(p_[0] > zk + 1e-9 for p_ in crossed) and any(p_[0] < zk - 1e-9 for p_ in crossed)
            if not inside:
                continue
            # charge per length on the barrel around this node, away from pads and planes
            near = [z for z in zr if abs(z - zk) < SUB and all(abs(z - p_[0]) > 0.05 for p_ in planes)]
            if len(near) < 3:
                continue
            dz = (max(near) - min(near)) / (len(near) - 1) * 1e-3
            cpl = np.mean([rows[z] for z in near]) / dz
            er_, _ = er_at(case, zk)
            reff = a_m * math.exp(2 * math.pi * EPS0 * er_ / cpl)
            if pitch < reff:
                C[k] *= math.log(reff / a_m) / math.log(pitch / a_m)
    # return vias (GND) with coordinates in m
    rets = [(0.0, v["r"] * 1e-3, (v["x"] - via["x"]) * 1e-3, (v["y"] - via["y"]) * 1e-3) for v in case["vias"] if v.get("net") == "GND"]
    sig = (0.0, via["r"] * 1e-3, 0.0, 0.0)
    plane_z = [p[0] for p in planes]

    def series(za, zb, fk):
        """series element between nodes za > zb: plane-pair impedance if inside a plane pair"""
        above = [pz for pz in plane_z if pz >= za - 1e-9]
        below = [pz for pz in plane_z if pz <= zb + 1e-9]
        if not above or not below:
            if not OUTER_L:
                return 0.0
            # outside the plane pairs (between a surface layer and the outermost plane): the barrel
            # and the return current wrapping into the antipad, a short coax with the antipad as
            # outer radius
            pl = min(below) if below else max(above)
            b = next((p[1] for p in planes if abs(p[0] - pl) < 1e-9 and p[1]), None) or 2 * via["r"]
            return 1j * 2 * math.pi * fk * MU0 * (za - zb) * 1e-3 / (2 * math.pi) * math.log(max(b / via["r"], 1.01))
        d = (za - zb) * 1e-3
        er, tand = er_at(case, 0.5 * (za + zb))
        # thickness-weighted εr of the span
        num, den = 0.0, 0.0
        for l in case["layers"]:
            o = max(0.0, min(l["z1"], za) - max(l["z0"], zb))
            num += o * l["er"]; den += o
        er = num / den if den else er
        w = 2 * math.pi * fk
        k = w * math.sqrt(MU0 * EPS0 * er) * np.sqrt(1 - 1j * loss_tan(tand, fk))
        pre = w * MU0 * d / 4  # Z = ωμd/4 · H0⁽²⁾(kρ) (e^{jωt})
        pts = [sig] + ([(0.0, partner["r"] * 1e-3, (partner["x"] - via["x"]) * 1e-3, (partner["y"] - via["y"]) * 1e-3)] if partner else []) + rets
        n = len(pts)
        Z = np.empty((n, n), complex)
        for i in range(n):
            for j in range(n):
                rr = pts[i][1] if i == j else math.hypot(pts[i][2] - pts[j][2], pts[i][3] - pts[j][3])
                Z[i, j] = pre * hankel2(0, k * rr)
        if partner:
            # reduce the shorted returns, then the odd / even mode of the pair
            k2 = 2
            if n > k2:
                Zr = Z[:k2, :k2] - Z[:k2, k2:] @ np.linalg.solve(Z[k2:, k2:], Z[k2:, :k2])
            else:
                Zr = Z[:k2, :k2]
            zs = Zr[0, 0] - Zr[0, 1] if mode == "odd" else Zr[0, 0] + Zr[0, 1]
        else:
            zs = Z[0, 0] if n == 1 else Z[0, 0] - Z[0, 1:] @ np.linalg.solve(Z[1:, 1:], Z[1:, 0])
        if SHARED:
            # a trace joining inside this plane pair splits it into two groups of the same cavity:
            # the radial mode sees each group weighted by its height, Z(d)·(d_g/d)² = Z(d_g)·d_g/d
            pa, pb = min(above), max(below)
            inner = [zj for zj in (z_in, z_out) if pb + 1e-9 < zj < pa - 1e-9]
            if inner:
                zj = inner[0]
                dg = (pa - zj) if za <= pa + 1e-9 and zb >= zj - 1e-9 else (zj - pb)
                zs *= dg / (pa - pb)
        return zs

    i_in, i_out = nodes.index(round(z_in, 6)), nodes.index(round(z_out, 6))
    S = []
    for fk in f:
        w = 2 * math.pi * fk
        # node admittance: via–plate capacitance with the dielectric loss of its surroundings
        def Yc(k):
            er_, td = er_at(case, nodes[k] - 1e-4) if k < len(nodes) - 1 else er_at(case, nodes[k] + 1e-4)
            return 1j * w * C[k] * (1 - 1j * loss_tan(td, fk))

        # chain from the entry node to the exit node, open stubs beyond on both sides
        def chain(a, b):
            M = np.eye(2, dtype=complex)
            step = 1 if b > a else -1
            for k in range(a, b, step):
                M = M @ np.array([[1, 0], [Yc(k), 1]]) @ np.array([[1, series(max(nodes[k], nodes[k + step]), min(nodes[k], nodes[k + step]), fk)], [0, 1]])
            return M @ np.array([[1, 0], [Yc(b), 1]])

        def stub_y(a, end):
            if a == end:
                return 0.0
            step = 1 if end > a else -1
            # admittance looking from node a (exclusive) towards the open end
            Y = Yc(end)
            for k in range(end, a, -step):
                Zs = series(max(nodes[k], nodes[k - step]), min(nodes[k], nodes[k - step]), fk)
                Y = 1 / (Zs + 1 / Y) if abs(Y) > 0 else 0.0
                if k - step != a:
                    Y = Y + Yc(k - step)
            return Y

        M = chain(i_in, i_out)
        # stubs: above the entry and below the exit (beyond the chain)
        top_end, bot_end = 0, len(nodes) - 1
        y_top = stub_y(min(i_in, i_out), top_end) if min(i_in, i_out) != top_end else 0.0
        y_bot = stub_y(max(i_in, i_out), bot_end) if max(i_in, i_out) != bot_end else 0.0
        if i_in < i_out:
            M = np.array([[1, 0], [y_top, 1]]) @ M @ np.array([[1, 0], [y_bot, 1]])
        else:
            M = np.array([[1, 0], [y_bot, 1]]) @ M @ np.array([[1, 0], [y_top, 1]])
        # traces over the antipads at both ends
        for side, zt in ((0, z_in), (1, z_out)):
            p = ports[side]
            wt = p["y1"] - p["y0"]
            ap = [pl for pl in planes if pl[1] is not None]
            # first plane on the other side of the reference
            ref_z = p["z0"]
            hole = next((pl for pl in ap if abs(pl[0] - ref_z) < 1e-6), None)
            if hole is None or hole[1] <= via["pad_r"]:
                continue
            ln = (hole[1] - via["pad_r"]) * 1e-3
            nxt = [pl[0] for pl in planes if (pl[0] < ref_z - 1e-6 if ref_z < zt else pl[0] > ref_z + 1e-6)]
            h2 = abs(zt - (max(nxt) if ref_z < zt and nxt else min(nxt) if nxt else ref_z)) if nxt else abs(zt - ref_z) * 3
            er_, _ = er_at(case, (zt + ref_z) / 2)
            zt0 = z_microstrip(wt, h2, er_)
            beta = w * math.sqrt(er_) / C0
            T = np.array([[math.cos(beta * ln), 1j * zt0 * math.sin(beta * ln)], [1j * math.sin(beta * ln) / zt0, math.cos(beta * ln)]])
            M = T @ M if side == 0 else M @ T
        # the 2×thru fixture runs to the via centre: remove the feed line over the pad radius
        for side in (0, 1):
            p = ports[side]
            wt = p["y1"] - p["y0"]
            zt = p["z1"]
            er_, _ = er_at(case, (zt + p["z0"]) / 2)
            zf, ee = feed_line(case, planes, wt, zt, p["z0"])
            beta = w * math.sqrt(ee) / C0
            # the fixture is a full trace up to the via centre: over the pad and over the antipad
            # opening of the reference plane, both of which the model has in its own elements
            hole = next((pl for pl in planes if abs(pl[0] - p["z0"]) < 1e-6 and pl[1]), None)
            ln = -max(via["pad_r"], hole[1] if hole else 0.0) * 1e-3
            T = np.array([[math.cos(beta * ln), 1j * zf * math.sin(beta * ln)], [1j * math.sin(beta * ln) / zf, math.cos(beta * ln)]])
            M = T @ M if side == 0 else M @ T
        A, B, Cc, D = M[0, 0], M[0, 1], M[1, 0], M[1, 1]
        z0 = 50.0
        den = A + B / z0 + Cc * z0 + D
        S.append([[(A + B / z0 - Cc * z0 - D) / den, 2 * (A * D - B * Cc) / den], [2 / den, (-A + B / z0 - Cc * z0 + D) / den]])
    return np.array(S), {"nodes_z": nodes, "C_fF": [round(c * 1e15, 1) for c in C]}


def pair_case(cid, c):
    """pairs: odd-mode ladder (per line, 50 Ω) against Sdd of the 3D result de-embedded to first
    order (feed delay and loss per line)"""
    p = os.path.join(REF, cid, "kisi.s4p")
    if not os.path.exists(p):
        return None
    f, S3 = sparam.read_touchstone(p)
    th = [os.path.join(REF, f"thru_{c['meta']['stackup']}_{pp['layer']}", "kisi.s2p") for pp in c["meta"]["ports"]]
    if not all(os.path.exists(t) for t in th):
        return None
    D = sparam.deembed_shift(S3, [sparam.fixture_shift(f, sparam.read_touchstone(t)[1]) for t in th])
    dd = sparam.mixed_mode(D)["dd"]
    So, info = ladder(c, f, mode="odd")
    sparam.write_touchstone(os.path.join(REF, cid, "pbm.s2p"), f, So, [f"{cid}: reduced via model, differential mode (per line 50 ohm)"])
    m = f <= 20e9
    dph = np.max(np.abs(np.degrees(np.angle(dd[m, 1, 0] * np.conj(So[m, 1, 0])))))
    r = {"mode": "odd", "dSdd21_max_db_20GHz": round(float(np.max(np.abs(sparam.db(dd[m, 1, 0]) - sparam.db(So[m, 1, 0])))), 2),
         "dphase_max_deg_20GHz": round(float(dph), 1)}
    print(cid, r, flush=True)
    return {cid: r}


def main():
    ids = sys.argv[1:]
    out = {}
    for fn in sorted(os.listdir(os.path.join(HERE, "via_cases"))):
        cid = fn[:-5]
        if ids and not any(k in cid for k in ids):
            continue
        c = json.load(open(os.path.join(HERE, "via_cases", fn)))
        if c["meta"]["group"] == "E":
            out.update(pair_case(cid, c) or {})
            continue
        if c["meta"]["group"] not in ("A", "B", "C", "D", "X") or len(c["ports"]) != 2 or "cap" in cid:
            continue
        p = os.path.join(REF, cid, "kisi.s2p")
        if not os.path.exists(p):
            continue
        f, S3 = sparam.read_touchstone(p)
        th = [os.path.join(REF, f"thru_{c['meta']['stackup']}_{pp['layer']}", "kisi.s2p") for pp in c["meta"]["ports"]]
        if not all(os.path.exists(t) for t in th):
            continue
        FL, _ = sparam.fixture_2xthru(f, sparam.read_touchstone(th[0])[1])
        FR, _ = sparam.fixture_2xthru(f, sparam.read_touchstone(th[1])[1])
        D = sparam.deembed2(S3, FL, FR)
        Sm, info = ladder(c, f)
        sparam.write_touchstone(os.path.join(REF, cid, "pbm.s2p"), f, Sm, [f"{cid}: physics-based via model (prototype), ports at the via"])
        sparam.write_touchstone(os.path.join(REF, cid, "kisi_deembedded.s2p"), f, D, [f"{cid}: kisi-fdtd, de-embedded with the 2×thru"])
        m = f <= 20e9
        d21 = np.abs(sparam.db(D[m, 1, 0]) - sparam.db(Sm[m, 1, 0]))
        e11 = np.abs(D[m, 0, 0] - Sm[m, 0, 0])
        t3, z3 = sparam.tdr(f, D[:, 0, 0], rise=25e-12, tmax=80e-12)
        tm, zm = sparam.tdr(f, Sm[:, 0, 0], rise=25e-12, tmax=80e-12)
        n3 = sparam.db(D[:, 1, 0]); nm = sparam.db(Sm[:, 1, 0])
        out[cid] = {"dS21_max_db_20GHz": round(float(d21.max()), 2), "dS11_max_20GHz": round(float(e11.max()), 3),
                    "z_min_3d": round(float(z3.min()), 1), "z_min_pbm": round(float(zm.min()), 1),
                    "z_max_3d": round(float(z3.max()), 1), "z_max_pbm": round(float(zm.max()), 1),
                    "notch_3d_ghz": round(float(f[np.argmin(n3)] / 1e9), 2) if n3.min() < -10 else None,
                    "notch_pbm_ghz": round(float(f[np.argmin(nm)] / 1e9), 2) if nm.min() < -10 else None, **info}
        print(cid, out[cid], flush=True)
    old = {}
    p = os.path.join(HERE, "results", "via_pbm.json")
    if os.path.exists(p) and ids:
        old = json.load(open(p))
    old.update(out)
    json.dump(old, open(p, "w"), indent=1)


if __name__ == "__main__":
    main()
