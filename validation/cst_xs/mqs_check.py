#!/usr/bin/env python3
"""Independent check of the conductor loss: a 2D magneto-quasi-static (eddy-current) solution by finite volumes on
a graded rectilinear grid, against the full-wave backend's MQS loss (fullwave/<id>.json sweep) and CST's lossy port
modes (results/<id>.fd.lossy.json), per metre: R = Re(Z′), L = Im(Z′)/ω.

    python3 validation/cst_xs/mqs_check.py [--ids a,b] [--freqs 0.1,1,10] [--json mqs_check.json]

  ∇²A − jωμ0σA = −μ0σV   in the conductors (V = 1 on sig+, −1 or +1 on sig− for the odd or even mode, 0 on the
                         grounds), ∇²A = 0 elsewhere; A = 0 on the box walls (electric walls, as CST and FW)
  Z′ = V / I(sig+)       per line (odd / even mode of a pair: Zodd, Zeven per line)
The grid lines are spaced δ/8 at every conductor face (both sides) and grow by 1.15 up to box/60; each value is
also solved with δ/4 and growth 1.3, the change is the grid error estimate. numpy and scipy.
"""
import argparse, json, math, os, sys
import numpy as np
import scipy.sparse as sp
import scipy.sparse.linalg as spl

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
MU0, SIGMA = 4e-7 * math.pi, 5.8e7
DEFAULT_IDS = ["sl_h0.1_u1_er4.2", "ms_u0.2_er4.4", "ms_u1_er4.4", "ms_u10_er4.4", "cpwg_u1_g0.5", "emb_u1_d0.1_er4.4",
               "cut_w0.4_c2_r1", "dms_u1_s1", "board_ts_01a9401349"]


def rects_of(text):
    """conductor rectangles (kind, x0, x1, y0, y1) of a geometry text (xsgeo.parse; rectangles only)"""
    import xsgeo
    return [(r["kind"], r["x0"], r["x1"], r["y0"], r["y1"]) for r in xsgeo.parse(text)["rects"] if r["kind"] != "diel"]


def axis(lo, hi, edges, hmin, g, hmax):
    """grid lines: spacing hmin at every edge (both sides), growing by g up to hmax"""
    edges = sorted({e for e in edges if lo < e < hi} | {lo, hi})
    pts = [lo]
    for a, b in zip(edges[:-1], edges[1:]):
        span, half, h, s = b - a, [], hmin, 0.0
        while s + h < span / 2:
            half.append(h)
            s += h
            h = min(hmax, h * g)
        mid = span - 2 * sum(half)
        if not half or mid < 0:
            k = max(2, int(math.ceil(span / hmin)))
            steps = [span / k] * k
        else:
            km = max(1, int(math.ceil(mid / half[-1] - 1e-9)))
            steps = half + [mid / km] * km + half[::-1]
        x = a
        for d in steps[:-1]:
            x += d
            pts.append(x)
        pts.append(b)
    return np.array(pts)


def solve(text, box, f, mode="single", hmin_frac=0.125, growth=1.15, sigma=SIGMA):
    X0, X1, Y0, Y1 = box
    rects = [(k, max(a, X0), min(b, X1), max(c, Y0), min(d, Y1)) for k, a, b, c, d in rects_of(text)]
    w = 2 * math.pi * f
    delta = math.sqrt(2 / (w * MU0 * sigma)) * 1e3  # mm
    hmax = max(X1 - X0, Y1 - Y0) / 60
    xs = axis(X0, X1, [v for r in rects for v in r[1:3]], hmin_frac * delta, growth, hmax)
    ys = axis(Y0, Y1, [v for r in rects for v in r[3:5]], hmin_frac * delta, growth, hmax)
    nx, ny = len(xs), len(ys)
    xc, yc = (xs[:-1] + xs[1:]) / 2, (ys[:-1] + ys[1:]) / 2
    sig = np.zeros((nx - 1, ny - 1))
    drv = np.zeros((nx - 1, ny - 1))
    for k, a, b, c, d in rects:
        ix, iy = (xc > a) & (xc < b), (yc > c) & (yc < d)
        sig[np.ix_(ix, iy)] = sigma
        drv[np.ix_(ix, iy)] = {"sig+": 1.0, "sig-": -1.0 if mode == "odd" else 1.0}.get(k, 0.0)
    hx, hy = np.diff(xs) * 1e-3, np.diff(ys) * 1e-3
    N = nx * ny
    wq, wv, wp = np.zeros(N), np.zeros(N), np.zeros(N)  # ∫σ, ∫σV, ∫σ over sig+ on each node's dual cell
    I, J = np.meshgrid(np.arange(nx - 1), np.arange(ny - 1), indexing="ij")
    area = np.outer(hx, hy) / 4
    for di in (0, 1):
        for dj in (0, 1):
            n = ((I + di) * ny + (J + dj)).ravel()
            np.add.at(wq, n, (sig * area).ravel())
            np.add.at(wv, n, (sig * drv * area).ravel())
            np.add.at(wp, n, (sig * (drv > 0) * (drv == 1) * area).ravel() if mode != "even" else (sig * (drv > 0) * area).ravel() / 2)
    rows, cols, vals = [], [], []
    diag = np.zeros(N, complex)
    jj = np.arange(ny)
    for i in range(nx - 1):  # fluxes between nodes (i, j) and (i+1, j)
        dl = np.zeros(ny)
        dl[:-1] += hy / 2
        dl[1:] += hy / 2
        c = dl / hx[i]
        a, b = i * ny + jj, (i + 1) * ny + jj
        rows += [a, b]; cols += [b, a]; vals += [c, c]
        diag[a] -= c; diag[b] -= c
    ii = np.arange(nx)
    for j in range(ny - 1):
        dl = np.zeros(nx)
        dl[:-1] += hx / 2
        dl[1:] += hx / 2
        c = dl / hy[j]
        a, b = ii * ny + j, ii * ny + j + 1
        rows += [a, b]; cols += [b, a]; vals += [c, c]
        diag[a] -= c; diag[b] -= c
    diag -= 1j * w * MU0 * wq
    K = sp.csr_matrix((np.concatenate(vals + [diag]), (np.concatenate(rows + [np.arange(N)]), np.concatenate(cols + [np.arange(N)]))), shape=(N, N))
    inner = np.zeros((nx, ny), bool)
    inner[1:-1, 1:-1] = True
    keep = np.flatnonzero(inner.ravel())
    A = np.zeros(N, complex)
    A[keep] = spl.spsolve(K[keep][:, keep].tocsc(), -MU0 * wv[keep])
    ip = np.sum(wp * (1.0 - 1j * w * A))  # current of sig+ (even mode: half the pair's current)
    z = 1.0 / ip
    return {"r": z.real, "l": z.imag / w, "grid": [nx, ny], "delta_um": delta * 1e3}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ids", default=",".join(DEFAULT_IDS))
    ap.add_argument("--freqs", default="0.1,1,10")
    ap.add_argument("--json", default="")
    a = ap.parse_args()
    import compare
    cases = {c["id"]: c for c in json.load(open(os.path.join(HERE, "cases.json"), encoding="utf-8"))["cases"]}
    out = {"about": __doc__.splitlines()[0], "rows": []}
    print(f"{'case:mode':30s} {'f GHz':>6s} {'R FD-MQS':>9s} {'grid err':>8s} {'R FW':>9s} {'FW/FD':>7s} {'R CST':>9s} {'CST/FD':>7s}")
    for cid in a.ids.split(","):
        c = cases[cid]
        text = c.get("cst_text") or c["text"]
        fw = (compare.load(os.path.join(HERE, "fullwave", cid + ".json")) or {}).get("results", {}).get("sweep") or {}
        cl = compare.cst_lossy(compare.load(os.path.join(HERE, "results", cid + ".fd.lossy.json")))
        fl = [round(x / 1e9, 6) for x in fw.get("f", [])]
        modes = ["odd", "even"] if c.get("n_signals", 1) == 2 else ["single"]
        for m in modes:
            for fg in (float(x) for x in a.freqs.split(",")):
                fine = solve(text, c["domain"]["box"], fg * 1e9, m)
                coarse = solve(text, c["domain"]["box"], fg * 1e9, m, hmin_frac=0.25, growth=1.3)
                rf = fw["modes"][m]["r"][fl.index(fg)] if fg in fl and m in fw.get("modes", {}) else None
                rc = (cl.get(fg) or {}).get(m, {}).get("r")
                row = {"id": cid, "mode": m, "f_ghz": fg, "r_fd": fine["r"], "l_fd": fine["l"], "grid": fine["grid"],
                       "grid_err_pct": (coarse["r"] / fine["r"] - 1) * 100, "r_fw": rf, "r_cst": rc}
                out["rows"].append(row)
                q = lambda x: "–" if x is None else f"{x:9.3f}"
                rr = lambda x: "–" if x is None else f"{(x / fine['r'] - 1) * 100:+6.2f}%"
                print(f"{cid + ':' + m:30s} {fg:6g} {fine['r']:9.3f} {row['grid_err_pct']:+7.2f}% {q(rf)} {rr(rf):>7s} {q(rc)} {rr(rc):>7s}", flush=True)
    if a.json:
        json.dump(out, open(a.json, "w"), indent=1)


if __name__ == "__main__":
    main()
