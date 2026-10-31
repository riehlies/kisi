"""Re-check of the junction model of kisi's via ladder (TESTS.md section 7) against the 3D references.

The junction model has two fitted choices (kisi_si::via_pbm, `ViaGeom::junction`):
* a series inductance k · μ0/(2π) · pad radius where a trace enters the via (k = 1);
* a stripline feed (planes above and below) is kept up to the via centre (cut = 0 mm), where a
  microstrip feed is removed up to the antipad edge.
The fit was made with experimental fields of `kisi-validate via-geom` that were removed afterwards.
This script needs no model change: it varies the two choices on kisi's ladder as stored
(rust_models.py's output or via_ref/<case>/ladder.s2p, k = 1, cut = 0) by cascading at each port a series inductance
(k − 1) · μ0/(2π) · pad radius and, on stripline sides, a line of −cut (matched, εeff of the layer)
– the junction sits next to the port plane, and the feeds are within a few ohms of 50 Ω, so the
cascade order does not matter at the scale looked at here – and scores every variant against the
de-embedded 3D result (kisi_deembedded.s2p; pairs: Sdd, first-order de-embedding as rust_models.py):
* Δτ: delay difference ladder − 3D from the S21 phase between 5 and 20 GHz (ps; positive: ladder slower);
* the TDR minimum / maximum error (25 ps, 80 ps window, %), max Δ|S21| to 20 GHz – as rust_models.py.

usage: python3 rust_models.py --out DIR && python3 junction_fit.py --ladders DIR [--out results/junction_fit.json]
(--ladders: kisi's ladder as the current code computes it, DIR/<case>/rust_pbm_graded.s2p of rust_models.py;
without it the stored via_ref/<case>/ladder.s2p, which is only as new as the last rust_models.py run)
"""
import argparse, json, math, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from rust_models import tdr_err  # noqa: E402

MU0 = 4e-7 * math.pi
C0 = 299792458.0
Z0 = 50.0
K_GRID = [round(0.4 + 0.1 * i, 1) for i in range(13)]          # 0.4 … 1.6
CUT_GRID = [0.0, 0.1, 0.2, 0.3, "0.5 pad", "0.75 pad", "pad"]     # stripline feed removed: mm, or × pad radius


def s2abcd(S):
    s11, s12, s21, s22 = S[:, 0, 0], S[:, 0, 1], S[:, 1, 0], S[:, 1, 1]
    A = np.empty_like(S)
    A[:, 0, 0] = ((1 + s11) * (1 - s22) + s12 * s21) / (2 * s21)
    A[:, 0, 1] = Z0 * ((1 + s11) * (1 + s22) - s12 * s21) / (2 * s21)
    A[:, 1, 0] = ((1 - s11) * (1 - s22) - s12 * s21) / (2 * s21) / Z0
    A[:, 1, 1] = ((1 - s11) * (1 + s22) + s12 * s21) / (2 * s21)
    return A


def abcd2s(A):
    a, b, c, d = A[:, 0, 0], A[:, 0, 1] / Z0, A[:, 1, 0] * Z0, A[:, 1, 1]
    den = a + b + c + d
    S = np.empty_like(A)
    S[:, 0, 0] = (a + b - c - d) / den
    S[:, 0, 1] = 2 * (a * d - b * c) / den
    S[:, 1, 0] = 2 / den
    S[:, 1, 1] = (-a + b - c + d) / den
    return S


def series_l(f, L):
    A = np.zeros((len(f), 2, 2), complex)
    A[:, 0, 0] = A[:, 1, 1] = 1
    A[:, 0, 1] = 2j * math.pi * f * L
    return A


def line(f, length_m, eeff):
    """matched Z0 line (negative length: removal)"""
    th = 2 * math.pi * f * length_m * math.sqrt(eeff) / C0
    A = np.zeros((len(f), 2, 2), complex)
    A[:, 0, 0] = A[:, 1, 1] = np.cos(th)
    A[:, 0, 1] = 1j * Z0 * np.sin(th)
    A[:, 1, 0] = 1j * np.sin(th) / Z0
    return A


def sides(c):
    """per port: (stripline?, εeff of a stripline feed) from the planes around the feed layer"""
    m = c["meta"]
    cz = m["copper_z"]
    pz = [cz[int(p[1:]) - 1] for p in m["planes"] + [q for q in m.get("pwr_planes", []) if q not in m["planes"]]]
    out = []
    for q in m["ports"][: 2 if len(m["ports"]) == 2 else 4: 1 if len(m["ports"]) == 2 else 2]:
        z = cz[int(q["layer"][1:]) - 1]
        strip = any(p > z + 1e-9 for p in pz) and any(p < z - 1e-9 for p in pz)
        er = [l["er"] for l in c["layers"] if l["z0"] < z + 0.3 and l["z1"] > z - 0.3 and (abs(l["z0"] - z) < 1e-6 or abs(l["z1"] - z) < 1e-6)]
        out.append((strip, float(np.mean(er)) if er else 4.0))
    return out


def dtau_ps(f, A, B):
    m = (f >= 5e9) & (f <= 20e9)
    ph = np.unwrap(np.angle(A[f <= 20e9, 1, 0])) - np.unwrap(np.angle(B[f <= 20e9, 1, 0]))
    fm = f[f <= 20e9]
    sel = m[f <= 20e9]
    return float(-np.mean(ph[sel] / (2 * math.pi * fm[sel])) * 1e12)


def load_cases(ladders=None):
    out = []
    for fn in sorted(os.listdir(os.path.join(HERE, "via_cases"))):
        cid = fn[:-5]
        c = json.load(open(os.path.join(HERE, "via_cases", fn)))
        g = c["meta"]["group"]
        lp = os.path.join(ladders, cid, "rust_pbm_graded.s2p") if ladders else os.path.join(REF, cid, "ladder.s2p")
        if g not in "ABCDEX" or len(g) != 1 or not os.path.exists(lp):
            continue
        f, L = sparam.read_touchstone(lp)
        if g == "E":
            f3, S3 = sparam.read_touchstone(os.path.join(REF, cid, "kisi.s4p"))
            th = [os.path.join(REF, f"thru_{c['meta']['stackup']}_{q['layer']}", "kisi.s2p") for q in c["meta"]["ports"]]
            D = sparam.mixed_mode(sparam.deembed_shift(S3, [sparam.fixture_shift(f3, sparam.read_touchstone(t)[1]) for t in th]))["dd"]
        else:
            dp = os.path.join(REF, cid, "kisi_deembedded.s2p")
            if not os.path.exists(dp):
                continue
            _, D = sparam.read_touchstone(dp)
        n = min(len(f), len(D))
        pad = next(v["pad_r"] for v in c["vias"] if v.get("net") not in ("GND", "PWR"))
        out.append({"id": cid, "group": g, "f": f[:n], "L": L[:n], "D": D[:n], "pad_r": pad, "sides": sides(c)})
    return out


def variant(cs, k_ms, k_sl, cut):
    f = cs["f"]
    A = s2abcd(cs["L"])
    lj = MU0 / (2 * math.pi) * cs["pad_r"] * 1e-3
    left, right = [], []
    for i, (strip, er) in enumerate(cs["sides"]):
        k = k_sl if strip else k_ms
        X = series_l(f, (k - 1) * lj)
        cm = cut if not isinstance(cut, str) else float(cut.split()[0] if " " in cut else 1.0) * cs["pad_r"]
        if strip and cm:
            X = line(f, -cm * 1e-3, er) @ X if i == 0 else X @ line(f, -cm * 1e-3, er)
        (left if i == 0 else right).append(X)
    for X in left:
        A = X @ A
    for X in right:
        A = A @ X
    return abcd2s(A)


def score(cases, k_ms, k_sl, cut):
    rows = {}
    for cs in cases:
        S = variant(cs, k_ms, k_sl, cut)
        f, D = cs["f"], cs["D"]
        m = f <= 20e9
        r = {"dtau_ps": round(dtau_ps(f, S, D), 3),
             "dS21_max_db": round(float(np.max(np.abs(sparam.db(D[m, 1, 0]) - sparam.db(S[m, 1, 0])))), 3),
             "dphase_20GHz_deg": round(float(np.degrees(np.angle(S[m, 1, 0][-1] * np.conj(D[m, 1, 0][-1])))), 2)}
        if cs["group"] != "E":
            r["tdr_err_pct"] = tdr_err(f, D[:, 0, 0], S[:, 0, 0])
        rows[cs["id"]] = r
    return rows


def summary(cases, rows):
    def pick(sel):
        return [rows[c["id"]] for c in cases if sel(c)]
    inner = lambda c: c["group"] != "E" and any(s[0] for s in c["sides"])  # noqa: E731
    thru = lambda c: c["group"] != "E" and not any(s[0] for s in c["sides"])  # noqa: E731
    single = pick(lambda c: c["group"] != "E")
    pairs = pick(lambda c: c["group"] == "E")
    tdr = [e for r in single for e in r["tdr_err_pct"]]
    med = lambda v: round(float(np.median(v)), 3) if v else None  # noqa: E731
    return {"single_tdr_err_pct_median": med(tdr), "single_tdr_err_pct_p90": round(float(np.percentile(tdr, 90)), 3),
            "single_dS21_max_db_median": med([r["dS21_max_db"] for r in single]),
            "single_abs_dtau_ps_median": med([abs(r["dtau_ps"]) for r in single]),
            "strip_exit_dtau_ps": [r["dtau_ps"] for r in pick(inner)], "strip_exit_dtau_ps_median": med([r["dtau_ps"] for r in pick(inner)]),
            "microstrip_only_dtau_ps_median": med([r["dtau_ps"] for r in pick(thru)]),
            "pairs_dS21_max_db_median": med([r["dS21_max_db"] for r in pairs]),
            "pairs_abs_dtau_ps_median": med([abs(r["dtau_ps"]) for r in pairs]),
            "pairs_strip_dtau_ps_median": med([rows[c["id"]]["dtau_ps"] for c in cases if c["group"] == "E" and any(s[0] for s in c["sides"])])}


def main():
    global REF
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(HERE, "results", "junction_fit.json"))
    ap.add_argument("--ladders", default=None)
    ap.add_argument("--ref", default=REF, help="the 3D references (default validation/via_ref)")
    a = ap.parse_args()
    REF = a.ref
    cases = load_cases(a.ladders)
    strip_ids = [c["id"] for c in cases if any(s[0] for s in c["sides"])]
    now = score(cases, 1.0, 1.0, 0.0)
    out = {"note": "kisi's ladder (k = 1, stripline cut 0) and variants against the de-embedded 3D references; see the docstring",
           "ladders": "kisi-validate models (rust_models.py --out), the current code" if a.ladders else "via_ref/<case>/ladder.s2p as stored",
           "cases": len(cases), "stripline_exits": strip_ids, "current": {"summary": summary(cases, now), "cases": now}}
    # 1) one k for every junction (the fitted coefficient), stripline handling unchanged
    grid1 = []
    for k in K_GRID:
        grid1.append({"k": k, **summary(cases, score(cases, k, k, 0.0))})
    out["common_k"] = grid1
    # 2) microstrip and stripline junctions separately, with a stripline cut
    grid2 = []
    for cut in CUT_GRID:
        for k_sl in K_GRID:
            s = summary(cases, score(cases, 1.0, k_sl, cut))
            grid2.append({"k_ms": 1.0, "k_sl": k_sl, "cut_mm": cut, **{x: s[x] for x in ("single_tdr_err_pct_median", "single_tdr_err_pct_p90", "single_dS21_max_db_median",
                                                                                           "single_abs_dtau_ps_median", "strip_exit_dtau_ps_median", "pairs_strip_dtau_ps_median")}})
    out["strip_k_cut"] = grid2
    best1 = min(grid1, key=lambda r: r["single_abs_dtau_ps_median"])
    # the stripline handling with the junction coefficient kept (k = 1): the cut that brings the stripline exits'
    # median delay error closest to zero (single vias and pairs together)
    best2 = min((r for r in grid2 if r["k_sl"] == 1.0), key=lambda r: abs(r["strip_exit_dtau_ps_median"]) + abs(r["pairs_strip_dtau_ps_median"]))
    out["best"] = {"common_k_by_abs_dtau": best1["k"], "strip_cut_with_k1": best2["cut_mm"],
                   "strip_cut_with_k1_dtau_ps": [best2["strip_exit_dtau_ps_median"], best2["pairs_strip_dtau_ps_median"]]}
    json.dump(out, open(a.out, "w"), indent=1)
    s = out["current"]["summary"]
    print(f"{len(cases)} cases; current: TDR {s['single_tdr_err_pct_median']} / {s['single_tdr_err_pct_p90']} %, |Δτ| median {s['single_abs_dtau_ps_median']} ps, "
          f"stripline exits Δτ median {s['strip_exit_dtau_ps_median']} ps, microstrip-only {s['microstrip_only_dtau_ps_median']} ps, pairs |Δτ| {s['pairs_abs_dtau_ps_median']} ps")
    for r in grid1:
        print(f"  k {r['k']:.1f}: TDR {r['single_tdr_err_pct_median']:.2f} / {r['single_tdr_err_pct_p90']:.2f} %, |Δτ| {r['single_abs_dtau_ps_median']:.2f} ps, "
              f"strip exits {r['strip_exit_dtau_ps_median']:+.2f}, microstrip-only {r['microstrip_only_dtau_ps_median']:+.2f}, pairs |Δτ| {r['pairs_abs_dtau_ps_median']:.2f} ps, "
              f"max Δ|S21| {r['single_dS21_max_db_median']:.3f} dB")
    for r in grid2:
        if r["k_sl"] == 1.0:
            print(f"  k 1, stripline cut {r['cut_mm']}: stripline exits Δτ {r['strip_exit_dtau_ps_median']:+.2f} ps (pairs {r['pairs_strip_dtau_ps_median']:+.2f}), "
                  f"TDR {r['single_tdr_err_pct_median']:.2f} / {r['single_tdr_err_pct_p90']:.2f} %")
    print("best:", out["best"])


if __name__ == "__main__":
    main()
