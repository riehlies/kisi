"""kisi's via ladder alone and its plane-cavity model on the 3D reference cases (validation/vias.md, sections Scoring
and Current results).

Runs `kisi-validate models … --grid graded` (crates/kisi-cli/src/models.rs: the ladder kisi uses – kisi_si::via_pbm,
graded r–z grid, junction model, equivalent antipad radius – with its ports at the via, and kisi_pi::cavity_modal),
then
* writes validation/via_ref/<case>/ladder.s2p (single vias A–D, X; pairs E in the differential mode, 50 Ω per line)
  and validation/via_ref/<case>/model.s2p (group G, 1 Ω) for the reference page;
* scores the ladder against the 3D result of each solver – kisi-fdtd (kisi.sNp), openEMS (openems.sNp, cell at metal
  halved) and CST (its finest mesh, cst_compare.CST_MESHES) –, each de-embedded with its own 2×thru fixtures, every
  fixture referred to its own line impedance first (sparam.deembed_matched_nport), pairs in the differential mode;
* compares the plane-cavity model with the |Z11| resonances of group G.

usage: python3 via_ladder.py [--exe kisi-validate] [--out DIR] → results/via_ladder.json
       (the raw output of kisi-validate goes to DIR, not to the repository)
"""
import argparse, json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from cst_compare import cst_fixture, cst_tools, result_file  # noqa: E402  (CST: the finest mesh, as the page)

TDR = dict(rise=25e-12, tmax=80e-12)  # the via alone: edge and window of the scores (compare_kisi.py, the page)
SOLVERS = ("kisi", "openems", "cst")  # kisi-fdtd, openEMS (refined mesh), CST (finest mesh)


def tdr_err(f, s3, sm):
    """errors of the TDR minimum and maximum (25 ps, 80 ps window), % signed (model / 3D − 1)"""
    _, z3 = sparam.tdr(f, s3, **TDR)
    _, zm = sparam.tdr(f, sm, **TDR)
    return [round((zm.min() / z3.min() - 1) * 100, 2), round((zm.max() / z3.max() - 1) * 100, 2)]


def dtau_ps(f, sm, s3):
    """delay difference model − 3D from the S21 phase between 5 and 20 GHz (ps; positive: the model slower)"""
    m = f <= 20e9
    ph = np.unwrap(np.angle(sm[m])) - np.unwrap(np.angle(s3[m]))
    sel = f[m] >= 5e9
    return round(float(-np.mean(ph[sel] / (2 * np.pi * f[m][sel])) * 1e12), 3)


def notch(f, s21):
    n = sparam.db(s21)
    return float(f[np.argmin(n)] / 1e9) if n.min() < -10 else None


def stats(v):
    v = [abs(x) for x in v if x is not None]
    if not v:
        return None
    return [round(float(np.median(v)), 3), round(float(np.percentile(v, 90)), 3), round(float(np.max(v)), 3)]


def deembedded(c, cid, tool):
    """the 3D result of one solver de-embedded with that solver's 2×thru fixtures (matched), pairs → Sdd; None if a
    result or a fixture is missing or not finite"""
    if tool == "cst":
        mesh = (cst_tools(REF, cid) or [None])[0]
        if not mesh:
            return None
        p = result_file(REF, cid, mesh)
        fx = [result_file(REF, t, cst_fixture(REF, t, mesh) or mesh) for t in thrus(c)]
    else:
        p = result_file(REF, cid, tool)
        fx = [result_file(REF, t, tool) for t in thrus(c)]
    if not p or not all(fx):
        return None
    f, S = sparam.read_touchstone(p)
    T = [sparam.read_touchstone(x)[1] for x in fx]
    if not np.all(np.isfinite(S)) or any(len(t) < len(f) or not np.all(np.isfinite(t)) for t in T):
        return None
    D = sparam.deembed_matched_nport(f, S, [t[: len(f)] for t in T])[0]
    return f, (sparam.mixed_mode(D)["dd"] if S.shape[1] == 4 else D)


def thrus(c):
    return [f"thru_{c['meta']['stackup']}_{q['layer']}" for q in c["meta"]["ports"]]


def peaks(f, z, fmax):
    """frequencies of the local maxima of |Z| up to fmax (GHz)"""
    a = np.abs(z)
    k = [i for i in range(1, len(f) - 1) if a[i] > a[i - 1] and a[i] >= a[i + 1] and f[i] <= fmax]
    return [round(float(f[i] / 1e9), 3) for i in k]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=os.environ.get("KISI_VALIDATE", "kisi-validate"))
    ap.add_argument("--out", default=os.path.join(tempfile.gettempdir(), "kisi_via_ladder"))
    a = ap.parse_args()
    res = json.loads(subprocess.run([a.exe, "models", os.path.join(HERE, "via_cases"), a.out, "--grid", "graded"],
                                    capture_output=True, text=True, check=True).stdout)
    rows = {}
    for cid, r in sorted(res.items()):
        c = json.load(open(os.path.join(HERE, "via_cases", cid + ".json")))
        g = c["meta"]["group"]
        if "rust_pbm_graded" in r:
            f, L = sparam.read_touchstone(os.path.join(a.out, cid, "rust_pbm_graded.s2p"))
            sparam.write_touchstone(os.path.join(REF, cid, "ladder.s2p"), f, L, [f"{cid}: via ladder as kisi uses it (kisi_si::via_pbm, graded grid, junction model)" +
                                                                                (", differential mode (per line 50 ohm)" if g == "E" else ", ports at the via")])
            if g not in "ABCDXE" or cid.startswith("D_s8_cap"):  # the set of the scores (compare_kisi.py)
                continue
            row = {"group": g}
            for tool in SOLVERS:
                d = deembedded(c, cid, tool)
                if d is None:
                    continue
                f3, D = d
                n = min(len(f3), len(f))
                if not np.allclose(f3[:n], f[:n]):
                    continue
                f3, D, Lm = f3[:n], D[:n], L[:n]
                m = f3 <= 20e9
                row[tool] = {"tdr_err_pct": tdr_err(f3, D[:, 0, 0], Lm[:, 0, 0]),
                             "dS21_max_db": round(float(np.max(np.abs(sparam.db(D[m, 1, 0]) - sparam.db(Lm[m, 1, 0])))), 3),
                             "dS11_max": round(float(np.max(np.abs(D[m, 0, 0] - Lm[m, 0, 0]))), 4),
                             "dtau_ps": dtau_ps(f3, Lm[:, 1, 0], D[:, 1, 0]),
                             "notch_3d_ghz": notch(f3, D[:, 1, 0]), "notch_ladder_ghz": notch(f3, Lm[:, 1, 0])}
            rows[cid] = row
        elif "cavity" in r:
            f, M = sparam.read_touchstone(os.path.join(a.out, cid, "rust_model.s2p"))
            sparam.write_touchstone(os.path.join(REF, cid, "model.s2p"), f, M, [f"{cid}: plane-cavity model as kisi uses it (kisi_pi::cavity_modal), 1 ohm ports"], 1.0)
            f3, S3 = sparam.read_touchstone(os.path.join(REF, cid, "kisi.s2p"))
            fmax = 4.5e9  # the first resonances, below the box modes of the finite mesh
            p3, pm = peaks(f3, sparam.s2z(S3, 1.0)[:, 0, 0], fmax), peaks(f, sparam.s2z(M, 1.0)[:, 0, 0], fmax)
            pairs = [(x, min(pm, key=lambda y: abs(y - x))) for x in p3 if pm]
            lo = (f3 <= 1e9) & (f3 > 0)
            z3, zm = sparam.s2z(S3, 1.0)[lo], sparam.s2z(M[: len(f3)], 1.0)[lo]
            rows[cid] = {"group": g, "resonances_ghz_3d_model": pairs,
                         "resonance_err_pct_max": round(max(abs(m_ / x - 1) * 100 for x, m_ in pairs), 2) if pairs else None,
                         "Z11_below_1GHz_mean_ratio": round(float(np.mean(np.abs(zm[:, 0, 0]) / np.abs(z3[:, 0, 0]))), 3),
                         "Z21_below_1GHz_mean_ratio": round(float(np.mean(np.abs(zm[:, 1, 0]) / np.abs(z3[:, 1, 0]))), 3)}
    single = [v for v in rows.values() if v["group"] in "ABCDX"]
    pairs_ = [v for v in rows.values() if v["group"] == "E"]
    summ = {}
    for tool in SOLVERS:
        for nm, vs in (("single", single), ("pairs", pairs_)):
            vs = [v[tool] for v in vs if tool in v]
            if not vs:
                continue
            summ.setdefault(tool, {})[nm] = {
                "cases": len(vs),
                "tdr_err_pct": stats([e for v in vs for e in v["tdr_err_pct"]]),
                "tdr_min_err_pct_signed_median": round(float(np.median([v["tdr_err_pct"][0] for v in vs])), 2),
                "dS21_max_db": stats([v["dS21_max_db"] for v in vs]),
                "dS11_max": stats([v["dS11_max"] for v in vs]),
                "abs_dtau_ps": stats([v["dtau_ps"] for v in vs]),
            }
    cav = [v for v in rows.values() if v["group"] == "G"]
    if cav:
        summ["cavity_G"] = {"resonance_err_pct_max": max(v["resonance_err_pct_max"] or 0 for v in cav),
                            "Z21_below_1GHz_mean_ratio": [v["Z21_below_1GHz_mean_ratio"] for v in cav]}
    out = {"about": "kisi's via ladder alone (ports at the via) against the de-embedded 3D result of each solver (matched 2×thru "
                    "de-embedding per port, pairs Sdd); [median, p90, max] of |error|: TDR minimum and maximum pooled (25 ps, "
                    "80 ps window), max Δ|S21| (dB) and max |ΔS11| (linear) to 20 GHz, |Δτ| from the phase 5–20 GHz; "
                    "group G: kisi's plane-cavity model against the |Z11| resonances",
           "summary": summ, "cases": rows}
    with open(os.path.join(HERE, "results", "via_ladder.json"), "w") as fh:
        json.dump(out, fh, indent=1)
        fh.write("\n")
    print(json.dumps(summ, indent=1))


if __name__ == "__main__":
    main()
