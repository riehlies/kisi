#!/usr/bin/env python3
"""Compare CST's port-mode results (results/*.json, any subset) with kisi (kisi.json) per group.

    python3 validation/cst_xs/compare.py [--groups a,b] [--tier 1,2] [--ids a,b] [--outlier 2] [--formula-outlier 5]
        [--quantity z|eeff|all] [--json out.json] [--md out.md] [--show-text]

Error in % = (value / reference − 1) · 100. Comparisons (each per group: n, median signed error =
bias, median |error|, 90th percentile |error|, max |error| with its case):

  solver E / M     kisi's solver in CST's box (field_E / field_M) against CST with electric /
                   magnetic walls: the same problem in both, no boundary difference – the check of
                   the geometry chain and js_2d_fields' numerics (not for the etch group, where CST
                   builds trapezoids)
  field vs CST     kisi's field result as kisi reports it (field: tol 0.01, open boundaries) against
                   CST's open-boundary estimate (mean of electric and magnetic walls; with only the
                   electric solve: CST_E · (field_E + field_M) / (2 field_E), marked "~")
  formula vs CST   kisi's closed-form result against the same
  field vs ref     kisi's field result against the reference stored with the case (HFSS / Ansys /
  CST vs ref       Forstén's MS/SL solver: the anchors)
Without any CST result the kisi-side comparisons run against kisi's own open-boundary estimate
(mean of field_E and field_M, tol 0.002): formula vs field and the field's own tolerance and
domain error (field vs field_open).

Quantities: single lines Z0; pairs Zdiff (= 2 Zodd) and Zeven; with --quantity eeff / all also εeff
(εeff,odd / εeff,even of pairs). Cases with a stale CST result (geometry changed since) are skipped.
"""
import argparse, json, math, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)


def fnv(t):
    h = 0xcbf29ce484222325
    for b in t.encode():
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def field_vals(r):
    """a js_2d_fields result → {z0 | zdiff, zodd, zeven, eeff…}"""
    if not r or not r.get("ok"):
        return {}
    if r.get("zodd") is not None:
        return {"zdiff": 2 * r["zodd"], "zodd": r["zodd"], "zeven": r.get("zeven"), "eeff_odd": r.get("eeff_odd"), "eeff_even": r.get("eeff_even")}
    return {"z0": r.get("z0"), "eeff": r.get("eeff")}


def formula_vals(f, pair):
    if not f:
        return {}
    if pair:
        return {"zdiff": f.get("zdiff"), "zodd": f.get("zodd"), "zeven": f.get("zeven"), "eeff_odd": f.get("eeff_odd"), "eeff_even": f.get("eeff_even")}
    return {"z0": f.get("z0"), "eeff": f.get("eeff")}


def mean(a, b):
    return {k: (a[k] + b[k]) / 2 for k in a if a.get(k) is not None and b.get(k) is not None}


def err(x, ref):
    return {k: (x[k] / ref[k] - 1) * 100 for k in ref if x.get(k) is not None and ref.get(k)}


def pct(v, p):
    v = sorted(v)
    if not v:
        return float("nan")
    i = (len(v) - 1) * p
    lo, hi = math.floor(i), math.ceil(i)
    return v[lo] + (v[hi] - v[lo]) * (i - lo)


def stats(rows):
    """rows: [(case id, err)] → dict"""
    if not rows:
        return None
    e = [x for _, x in rows]
    a = [abs(x) for x in e]
    k = max(range(len(a)), key=lambda i: a[i])
    return {"n": len(e), "bias": pct(e, 0.5), "med": pct(a, 0.5), "p90": pct(a, 0.9), "max": a[k], "max_case": rows[k][0], "max_signed": e[k]}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--kisi", default=os.path.join(HERE, "kisi.json"))
    ap.add_argument("--results", default=os.path.join(HERE, "results"))
    ap.add_argument("--tag", default="", help="compare results/<id>.<tag>.json")
    ap.add_argument("--groups", default="")
    ap.add_argument("--ids", default="")
    ap.add_argument("--tier", default="")
    ap.add_argument("--quantity", choices=["z", "eeff", "all"], default="z")
    ap.add_argument("--outlier", type=float, default=2.0, help="list field / solver errors above this, %%")
    ap.add_argument("--formula-outlier", type=float, default=5.0, help="list formula errors above this, %%")
    ap.add_argument("--show-text", action="store_true", help="print the geometry text of outliers")
    ap.add_argument("--json", default="")
    ap.add_argument("--md", default="")
    a = ap.parse_args()

    cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
    if a.groups:
        cases = [c for c in cases if c["group"] in a.groups.split(",")]
    if a.tier:
        cases = [c for c in cases if str(c["tier"]) in a.tier.split(",")]
    if a.ids:
        cases = [c for c in cases if c["id"] in a.ids.split(",")]
    kisi = json.load(open(a.kisi, encoding="utf-8"))["results"] if os.path.isfile(a.kisi) else {}
    qz = {"z": ("z0", "zdiff", "zeven"), "eeff": ("eeff", "eeff_odd", "eeff_even"),
          "all": ("z0", "zdiff", "zeven", "eeff", "eeff_odd", "eeff_even")}[a.quantity]

    comps = {}  # (comparison, group, quantity) -> [(id, err)]
    outl = []
    n_cst = n_stale = 0
    per_case = {}

    def add(comp, c, e, lim):
        for q, v in e.items():
            if q not in qz:
                continue
            comps.setdefault((comp, c["group"], q), []).append((c["id"], v))
            if lim is not None and abs(v) > lim:
                outl.append((comp, c, q, v))

    for c in cases:
        k = kisi.get(c["id"], {})
        pair = c.get("n_signals", 1) == 2
        fld = field_vals(k.get("field"))
        fE, fM = field_vals(k.get("field_E")), field_vals(k.get("field_M"))
        fopen = mean(fE, fM) if fM else {}
        frm = formula_vals(k.get("formula"), pair)
        stem = c["id"] + (f".{a.tag}" if a.tag else "")
        cst = None
        p = os.path.join(a.results, stem + ".json")
        if os.path.isfile(p):
            m = json.load(open(p, encoding="utf-8"))
            if m.get("text_hash") and m["text_hash"] != fnv(c.get("cst_text") or c["text"]):
                n_stale += 1
            elif m.get("status") in ("ok", "not_converged") and m.get("result"):
                cst = m
                n_cst += 1
        rec = {"group": c["group"], "field": fld, "formula": frm, "field_open": fopen}
        etch = c["group"] == "etch"
        if cst:
            r = cst["result"]
            cE, cM = r.get("electric") or {}, r.get("magnetic") or {}
            if r.get("open_est"):
                copen, approx = r["open_est"], False
            elif cE and fE and fM:
                copen = {q: cE[q] * fopen[q] / fE[q] for q in cE if cE.get(q) and fopen.get(q) and fE.get(q)}
                approx = True
            else:
                copen, approx = cE, True
            rec.update(cst_E=cE, cst_M=cM, cst_open=copen, cst_open_approx=approx, cst_status=cst["status"])
            if not etch:
                add("solver E", c, err(fE, cE), a.outlier)
                if cM and fM:
                    add("solver M", c, err(fM, cM), a.outlier)
            add("field vs CST", c, err(fld, copen), a.outlier)
            if frm:
                add("formula vs CST", c, err(frm, copen), a.formula_outlier)
            if c.get("ref"):
                add("CST vs ref", c, err(copen, c["ref"]), a.outlier)
        else:
            # kisi side only
            if fopen:
                add("field vs field_open", c, err(fld, fopen), a.outlier)
                if frm:
                    add("formula vs field_open", c, err(frm, fopen), a.formula_outlier)
        if c.get("ref"):
            add("field vs ref", c, err(fld, c["ref"]), a.outlier)
            if frm:
                add("formula vs ref", c, err(frm, c["ref"]), a.formula_outlier)
        per_case[c["id"]] = rec

    order = ["solver E", "solver M", "field vs CST", "formula vs CST", "CST vs ref", "field vs ref", "formula vs ref",
             "field vs field_open", "formula vs field_open"]
    groups = []
    for c in cases:
        if c["group"] not in groups:
            groups.append(c["group"])
    lines = [f"cases {len(cases)}, with CST result {n_cst}" + (f", stale CST results skipped {n_stale}" if n_stale else "")]
    summary = {}
    for comp in order:
        rows = [(g, q, stats(comps[(comp, g, q)])) for g in groups for q in qz if (comp, g, q) in comps]
        if not rows:
            continue
        lines.append("")
        lines.append(f"== {comp}  (error %: bias = median signed, med / p90 / max of |error|)")
        lines.append(f"{'group':20s} {'qty':9s} {'n':>4s} {'bias':>7s} {'med':>6s} {'p90':>6s} {'max':>7s}  worst case")
        allrows = {}
        for g, q, s in rows:
            lines.append(f"{g:20s} {q:9s} {s['n']:4d} {s['bias']:+7.2f} {s['med']:6.2f} {s['p90']:6.2f} {s['max_signed']:+7.2f}  {s['max_case']}")
            summary.setdefault(comp, {}).setdefault(g, {})[q] = s
            allrows.setdefault(q, []).extend(comps[(comp, g, q)])
        for q, rr in allrows.items():
            s = stats(rr)
            lines.append(f"{'ALL':20s} {q:9s} {s['n']:4d} {s['bias']:+7.2f} {s['med']:6.2f} {s['p90']:6.2f} {s['max_signed']:+7.2f}  {s['max_case']}")
            summary.setdefault(comp, {}).setdefault("ALL", {})[q] = s
    if outl:
        lines.append("")
        lines.append(f"== outliers (solver / field above {a.outlier} %, formula above {a.formula_outlier} %)")
        outl.sort(key=lambda t: (order.index(t[0]), -abs(t[3])))
        for comp, c, q, v in outl:
            lines.append(f"{comp:22s} {q:6s} {v:+7.2f} %  {c['id']}: {c['title']}")
            if a.show_text:
                lines.extend("      " + ln for ln in (c.get("cst_text") or c["text"]).strip().splitlines())
    rep = "\n".join(lines)
    print(rep)
    if a.json:
        json.dump({"summary": summary, "cases": per_case, "outliers": [{"comparison": o[0], "id": o[1]["id"], "quantity": o[2], "err_pct": o[3]} for o in outl]},
                  open(a.json, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    if a.md:
        open(a.md, "w", encoding="utf-8").write("```\n" + rep + "\n```\n")


if __name__ == "__main__":
    main()
