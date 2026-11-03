#!/usr/bin/env python3
"""How accurate are the CST results of validation/cst_xs? Estimates per source, from the stored results only.

    python3 validation/cst_xs/uncertainty.py [--results DIR] [--kisi kisi.json] [--json out.json]

1. discretisation  every case: change of the last mesh level (the convergence criterion) and the distance of
                   the last level from its Richardson / geometric extrapolation; the anchors (3–5 levels) check
                   the estimator: Richardson from levels k-1, k against level k+1 and later
2. exact εeff      cases in one homogeneous dielectric (striplines): εeff must equal εr exactly
3. walls / box     electric against magnetic walls (anchors, both walls) – the domain's influence on the
                   open-boundary value; not part of the solver comparison (kisi field_E uses the same box)
4. solvers         CST against kisi's field solver in the same box (field_E): an upper bound for both
5. lossy runs      frequency domain (*.fd.lossy.json) against the time domain hexahedral results at 1 GHz,
                   with the internal inductance of the conductors (Wheeler) removed
Quantities: z (Z0, Zdiff, Zcomm / Zodd, Zeven) and eeff (εeff, odd, even); all in %.
"""
import argparse, glob, json, math, os, re, statistics as st

HERE = os.path.dirname(os.path.abspath(__file__))
ZQ = ("z0", "zdiff", "zcomm")
EQ = ("eeff", "eeff_odd", "eeff_even")


def pct(a, b):
    return (a / b - 1) * 100 if a is not None and b else None


def summary(xs):
    xs = sorted(abs(x) for x in xs if x is not None)
    if not xs:
        return {"n": 0}
    p = lambda q: xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]
    return {"n": len(xs), "median": round(st.median(xs), 4), "p90": round(p(0.9), 4), "max": round(xs[-1], 4)}


def richardson(va, vb, p=2):
    return {k: vb[k] + (vb[k] - va[k]) / (2 ** p - 1) for k in vb if va.get(k) is not None and vb.get(k) is not None}


def homogeneous_er(text):
    ers = {float(m) for m in re.findall(r"^diel .* er=([\d.]+)", text, re.M)}
    closed = re.search(r"^gnd x=-inf w=inf y=0 ", text, re.M) and len(re.findall(r"^gnd x=-inf w=inf", text, re.M)) >= 2
    return ers.pop() if len(ers) == 1 and closed else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--results", default=os.path.join(HERE, "results"))
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--kisi", default=os.path.join(HERE, "kisi.json"))
    ap.add_argument("--json", default="")
    a = ap.parse_args()
    cases = {c["id"]: c for c in json.load(open(a.cases))["cases"]}
    res = {}
    for f in glob.glob(os.path.join(a.results, "*.json")):
        if f.endswith(".lossy.json"):
            continue
        m = json.load(open(f))
        if m.get("status") == "ok":
            res[m["id"]] = m
    out = {}

    # 1 discretisation
    last_change = {"z": [], "eeff": []}
    to_extrap = {"z": [], "eeff": []}
    check = {"z": [], "eeff": []}  # Richardson(k-1, k) against the finest level, where ≥ 3 levels
    for cid, m in res.items():
        w = m["walls"].get("electric", {})
        lv = [l["values"] for l in w.get("levels", [])]
        if len(lv) >= 2:
            for k in set(ZQ + EQ) & set(lv[-1]):
                last_change["z" if k in ZQ else "eeff"].append(pct(lv[-1][k], lv[-2].get(k)))
            ex = (w.get("values_richardson") or {}).get("values") or w.get("values_extrap") or richardson(lv[-2], lv[-1])
            for k in set(ZQ + EQ) & set(ex):
                to_extrap["z" if k in ZQ else "eeff"].append(pct(lv[-1][k], ex[k]))
        if len(lv) >= 3:
            r = richardson(lv[-3], lv[-2])
            for k in set(ZQ + EQ) & set(r):
                check["z" if k in ZQ else "eeff"].append(pct(r[k], lv[-1][k]))
    out["discretisation"] = {
        "last_level_change": {q: summary(v) for q, v in last_change.items()},
        "last_level_vs_extrapolation": {q: summary(v) for q, v in to_extrap.items()},
        "richardson_check_on_finer_level": {q: summary(v) for q, v in check.items()},
    }

    # 2 exact εeff in a homogeneous dielectric
    ex = []
    for cid, m in res.items():
        er = homogeneous_er(cases.get(cid, {}).get("cst_text") or cases.get(cid, {}).get("text", ""))
        if er:
            v = m["result"].get("electric", {})
            for k in EQ:
                if v.get(k):
                    ex.append((cid, k, pct(v[k], er)))
    out["exact_eeff_homogeneous"] = summary([x[2] for x in ex]) | {"worst": sorted(ex, key=lambda x: -abs(x[2]))[:3]}

    # 3 walls
    sp = [m["result"].get("open_spread_pct") for m in res.values() if m["result"].get("open_spread_pct") is not None]
    out["walls_electric_vs_magnetic"] = summary(sp)

    # 4 CST against the other solvers in the same box (compare.py's values: like with like – Z lossless, εeff with
    # ideal conductors, pairs in CST's impedance definition for FW and transferred to kisi's for the FDM)
    import compare
    ca = compare.parser().parse_args(["--cases", a.cases, "--kisi", a.kisi, "--results", a.results, "--no-loss"])
    recs, _ = compare.collect(ca)
    sol = {"fw_z": [], "fw_eeff": [], "fdm_z": []}
    for r in recs:
        v = r["values"]
        if r["id"] not in res or not v.get("cst_E"):
            continue
        for k, x in compare.err(v.get("fw_E_cstdef") or {}, v["cst_E"]).items():
            sol["fw_eeff" if k in EQ else "fw_z"].append(x)
        if r["group"] != "etch":
            sol["fdm_z"] += [x for k, x in compare.err(v.get("fdm_E") or {}, v.get("cst_E_k") or {}).items() if k not in EQ]
    out["cst_vs_solvers_same_box"] = {q: summary(x) for q, x in sol.items()}

    # 5 lossy: frequency domain against the time domain at 1 GHz
    fdz, fde, fdc, fdl = [], [], [], []
    for f in glob.glob(os.path.join(a.results, "*.fd.lossy.json")):
        m = json.load(open(f))
        if m.get("status") != "ok" or m["id"] not in res:
            continue
        p1 = [p for p in m["walls"]["electric"]["freqs"] if abs(p["f_ghz"] - 1) < 1e-6]
        if not p1:
            continue
        v, b = p1[0]["values"], res[m["id"]]["result"]["electric"]
        k = "z0" if "z0" in b else "zdiff"
        e = "eeff" if "eeff" in b else "eeff_odd"
        fdz.append(pct(v.get(k), b.get(k)))
        fde.append(pct(v.get(e), b.get(e)))
        # Z² ∝ L/C, εeff ∝ L·C: split the difference into capacitance (port mesh, dielectric) and inductance
        # (the conductors' internal inductance, absent with the lossless port modes of the time domain)
        if v.get(k) and v.get(e) and b.get(k) and b.get(e):
            zr, er = v[k] / b[k], v[e] / b[e]
            fdc.append((math.sqrt(er / zr ** 2) - 1) * 100)
            fdl.append((math.sqrt(er * zr ** 2) - 1) * 100)
    out["fd_vs_td_1GHz_raw"] = {"z": summary(fdz), "eeff": summary(fde)}
    out["fd_vs_td_1GHz_split"] = {"capacitance": summary(fdc), "inductance": summary(fdl),
                                  "note": "asymmetric pairs (CST's modal definitions) inflate the maxima"}

    print(json.dumps(out, indent=1, ensure_ascii=False))
    if a.json:
        json.dump(out, open(a.json, "w"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
