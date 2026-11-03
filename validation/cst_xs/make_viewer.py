#!/usr/bin/env python3
"""Build dist/kisi-references-2d.html (not in the repository; built locally and by the release workflow): a
self-contained page with every cross-section case of validation/cst_xs – the geometry to scale, the four solvers
(CST, full-wave, kisi's FDM field solver, kisi's formulas) at 1 GHz and the loss sweep – and the per-group summary
of compare.py. Reads the committed JSON only (cases.json, results/, fullwave/, kisi.json, kisi_loss.json).

usage: python3 validation/cst_xs/make_viewer.py [cst_xs_dir] [out.html] [--fragment]
"""
import base64, json, math, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
SRC = os.path.abspath(ARGS[0]) if len(ARGS) > 0 else HERE
OUT = ARGS[1] if len(ARGS) > 1 else os.path.join(ROOT, "dist", "kisi-references-2d.html")
sys.path.insert(0, SRC)
import compare  # noqa: E402
import xsgeo  # noqa: E402

PER_DECADE = 5  # FW sweep points per decade on the page (the sweep has 10)
GROUPS = {
    "anchor": ("Anchors", "Forstén's suite (HFSS / Ansys references), ThunderScope and asymmetric striplines with references of Forstén's MS/SL solver."),
    "microstrip": ("Microstrip", "w/h 0.1…10 × εr 2.2…10.2, h 0.3 mm, 35 µm copper."),
    "microstrip_t": ("Microstrip, copper thickness", "17.5…105 µm × w/h 0.25…4."),
    "microstrip_h": ("Microstrip, substrate height", "h 0.07…1.6 mm × w/h 0.5, 1.9, 4."),
    "microstrip_mask": ("Microstrip under solder mask", "Mask 10…35 µm × εr 3.3, 4.0; thick copper under mask."),
    "embedded": ("Covered trace", "Inner layer with one plane: cover 0.05…0.5 mm."),
    "stripline": ("Stripline", "Symmetric: w/h 0.25…4 × h 0.1, 0.2 × εr 3.0, 4.2; copper 17.5…70 µm."),
    "stripline_asym": ("Offset stripline", "h2/h1 1.5…8; different εr above and below."),
    "stripline_dual": ("Dual stripline", "Trace between two planes with an empty signal layer in between."),
    "diff_microstrip": ("Edge-coupled microstrip pairs", "s/h 0.25…4 × w/h 0.5…2, mask on/off."),
    "diff_stripline": ("Edge-coupled stripline pairs", "s/h 0.25…4, symmetric and offset."),
    "broadside": ("Broadside-coupled stripline", "No kisi model: solver check only."),
    "cpwg": ("Grounded coplanar", "gap/h 0.25…4 × w/h, mask."),
    "diff_cpwg": ("Coplanar pairs", "Pair with coplanar ground."),
    "stripline_cpw": ("Stripline with coplanar ground", "The formula ignores the coplanar ground."),
    "cpw_noplane": ("Coplanar without plane", "kisi's no-plane formula (an estimate) against the solvers."),
    "plane_cut": ("Plane cut-outs", "Trace over an opening in the next layer (1…8 × w), reference the layer below."),
    "plane_edge": ("Plane edge", "Reference plane ending 0.5…8 h beside the trace."),
    "etch": ("Trapezoidal etch", "Etch factor 1, 2, 3: CST and FW solve the trapezoid, kisi's FDM and formula the rectangle."),
    "size_study": ("Box size", "8 cases with the solver box ×1.5 and ×2 (port size rule)."),
    "board_target": ("Example boards: target impedance", "ThunderScope and LibreVNA cross-sections of nets with a target impedance."),
    "board_other": ("Example boards: other", "The longest other cross-sections per example board."),
    "test_board": ("Test-board coupons", "All distinct cross-sections of the test-board coupons."),
}


def sig(x, n=5):
    if x is None or (isinstance(x, float) and not math.isfinite(x)):
        return None
    if x == 0:
        return 0
    return float(f"{x:.{n}g}")


def rnd(d, n=5):
    if isinstance(d, dict):
        return {k: rnd(v, n) for k, v in d.items() if v is not None}
    if isinstance(d, (list, tuple)):
        return [rnd(v, n) for v in d]
    if isinstance(d, float):
        return sig(d, n)
    return d


def geometry(c):
    """rects (clipped to the box), trapezoid polygons, wall extras, box"""
    text = c.get("cst_text") or c["text"]
    g = xsgeo.parse(text)
    box = c["domain"]["box"]
    X0, X1, Y0, Y1 = box
    out = []
    for r in g["rects"]:
        x0, x1 = max(r["x0"], X0), min(r["x1"], X1)
        e = {"k": r["kind"], "x": [x0, x1], "y": [r["y0"], r["y1"]]}
        if r["kind"] == "diel":
            e["er"], e["tand"] = r["er"], r["tand"]
        if r["shape"] == "trap":
            e["poly"] = xsgeo.trap_polygon(r)
        out.append(e)
    extras = xsgeo.wall_extras(g, box, "electric")
    ex = [{"x": [s["x0"], s["x1"]], "y": [s["y0"], s["y1"]]} for s in extras]
    rect_kisi = None
    if c.get("cst_text"):  # what kisi solves instead (etch group)
        rect_kisi = [{"k": r["kind"], "x": [r["x0"], r["x1"]], "y": [r["y0"], r["y1"]]} for r in xsgeo.parse(c["text"])["rects"] if r["kind"] in ("sig+", "sig-")]
    return rnd({"rects": out, "extras": ex, "box": box, "closed": [c["domain"].get("closed_bottom", False), c["domain"].get("closed_top", False)],
                "H": c["domain"].get("H"), "kisi_rect": rect_kisi}, 6)


def fw_curves(a, rec, models, text):
    """the FW sweep (PER_DECADE per decade) and kisi's line model on the same grid, per mode"""
    fw = compare.load(os.path.join(a.fullwave, rec["id"] + ".json")) or {}
    sw = (fw.get("results") or {}).get("sweep")
    if not sw or not sw.get("ok"):
        return None
    f = [x / 1e9 for x in sw["f"]]
    keep = [i for i, x in enumerate(f) if abs(PER_DECADE * math.log10(x) - round(PER_DECADE * math.log10(x))) < 1e-6 or x in (5.0, 20.0, 40.0)]
    out = {"f": [sig(f[i], 4) for i in keep], "modes": {}}
    for name, m in sw["modes"].items():
        d = {"alpha_db": [sig(m["alpha_c"][i] + m["alpha_d"][i], 4) for i in keep], "r": [sig(m["r"][i], 4) for i in keep],
             "zc_re": [sig(m["zc_re"][i], 5) for i in keep], "eeff": [sig(m["eeff"][i], 5) for i in keep]}
        k = {}
        if name in models:
            pts = [models[name].at(f[i] * 1e9) for i in keep]
            k = {q: [sig(p[q], 5 if q in ("zc_re", "eeff") else 4) for p in pts] for q in ("alpha_db", "r", "zc_re", "eeff")}
        out["modes"][name] = {"fw": d, "kisi": k}
    return out


def explain(rec, c):
    """known causes of a case's outliers (README, Results)"""
    v = rec["values"]
    notes = []
    if rec["asym"]:
        raw = compare.err(v.get("fw_E") or {}, v.get("cst_E") or {}, ("zdiff", "zeven"))
        notes.append("Asymmetric pair: CST's impedances are the eigenmodes' power-current values, kisi's (FDM, FW) the modal "
                     "values with |v|² = 2; they differ here by " + ", ".join(f"{q} {x:+.1f} %" for q, x in raw.items()) +
                     ". The comparison transfers CST to kisi's definition through FW's L and C; the eigenmode εeff agree.")
    if rec["group"] == "etch":
        e = compare.err(v.get("fdm_E") or {}, v.get("cst_E") or {}, ("z0", "zdiff", "zeven"))
        ef = compare.err(v.get("formula") or {}, v.get("cst_E_k") or {}, ("z0", "zdiff", "zeven"))
        notes.append("kisi's field geometry and its formula use the rectangle of the drawn width at full thickness; CST "
                     "and FW solve the trapezoid. kisi's error from it (same box): field " +
                     ", ".join(f"{q} {x:+.1f} %" for q, x in e.items()) + "; formula " + ", ".join(f"{q} {x:+.1f} %" for q, x in ef.items()) +
                     ". A rectangle of width w − t/(2·etch factor) comes within 2 % for microstrips, −2…−4 % for striplines (README).")
    f = (rec.get("formula_meta") or {})
    e = compare.err(v.get("formula") or {}, v.get("cst_open_k") or {}, ("z0", "zdiff", "zeven"))
    big = {q: x for q, x in e.items() if abs(x) > 5} if rec["group"] != "etch" else {}
    if big:
        why = {"plane_cut": "the formula takes the plane below the cut-out, the field sees the opening's edges",
               "cpwg": "coplanar formula outside its range (wide gap or narrow trace)",
               "cpw_noplane": "kisi's no-plane formula is an estimate",
               "diff_cpwg": "the pair formula ignores the coplanar ground",
               "stripline_cpw": "the stripline formula ignores the coplanar ground"}.get(rec["group"], "outside the formula's range of validity or a structure it does not model")
        notes.append(f"Formula ({f.get('kind') or '–'}) off by " + ", ".join(f"{q} {x:+.1f} %" for q, x in big.items()) + f": {why}.")
    return notes


def main():
    a = compare.parser().parse_args(["--cases", os.path.join(SRC, "cases.json"), "--kisi", os.path.join(SRC, "kisi.json"),
                                     "--results", os.path.join(SRC, "results"), "--fullwave", os.path.join(SRC, "fullwave"),
                                     "--loss-cache", os.path.join(SRC, "kisi_loss.json")])
    recs, cases = compare.collect(a)
    byid = {c["id"]: c for c in cases}
    kisi = (compare.load(a.kisi) or {}).get("results", {})
    kloss = (compare.load(a.loss_cache) or {}).get("results", {})
    quants = compare.ZQ + compare.EQ
    _, comps, outl = compare.report(recs, a, quants)
    loss_rows, excl = compare.compare_loss(recs)
    # per-group summary (the four-way tables)
    groups = []
    for r in recs:
        if r["group"] not in groups:
            groups.append(r["group"])
    summ = {}
    for g in groups + ["ALL"]:
        row = {}
        for cp in ("FW vs CST, E walls", "FDM vs CST, E walls", "field vs CST open", "formula vs CST open", "FW static vs CST, E walls",
                   "FDM static vs CST, E walls", "FW phase vs CST lossy", "FDM phase vs CST lossy"):
            for q in quants:
                s = compare.stats(comps.get((cp, g, q), []))
                if s:
                    row.setdefault(cp, {})[q] = rnd({k: s[k] for k in ("n", "bias", "med", "p90", "max_signed", "max_case")}, 3)
        for sv in ("fw", "kisi"):
            for q in ("alpha_db", "r", "g"):
                for fg in (1.0, 10.0):
                    s = compare.stats(loss_rows.get((sv, q, fg, g), []))
                    if s:
                        row.setdefault(f"loss {sv}", {})[f"{q}@{fg:g}"] = rnd({k: s[k] for k in ("n", "bias", "med", "p90")}, 3)
        summ[g] = row
    out_cases = []
    for r in recs:
        c = byid[r["id"]]
        r["formula_meta"] = {k: (kisi.get(r["id"], {}).get("formula") or {}).get(k) for k in ("kind", "flags")}
        v = r["values"]
        keep = {k: rnd(v.get(k) or {}) for k in ("cst_E", "cst_M", "cst_open", "cst_E_k", "cst_open_k", "fw_E", "fw_E_cstdef", "fw_M",
                                                  "fw_static_E", "fdm_E", "fdm_M", "fdm_static_E", "field", "formula") if v.get(k)}
        loss = {}
        for name, pts in (r.get("loss") or {}).items():
            loss[name] = [rnd({"f": fg, "cst": p["cst"], "fw": p.get("fw"), "kisi": p.get("kisi"), "x": p.get("excluded")}, 5) for fg, p in sorted(pts.items())]
        e = {"id": r["id"], "g": r["group"], "t": r["title"], "tier": r["tier"], "pair": r["pair"], "asym": r["asym"],
             "params": rnd(c.get("params"), 6), "ref": c.get("ref"), "geo": geometry(c), "v": keep, "loss": loss,
             "fw_geometry": r.get("fw_geometry"), "box_cutoff": sig(r.get("box_cutoff_ghz"), 3),
             "formula": r["formula_meta"], "cst_version": r.get("cst_version"), "notes": explain(r, c),
             "M": bool(v.get("cst_M"))}
        cur = fw_curves(a, r, compare.kisi_models(kloss.get(r["id"]), c["text"]), c["text"])
        if cur:
            e["sweep"] = cur
        out_cases.append(e)
    data = {"cases": out_cases, "groups": {g: list(GROUPS.get(g, (g, ""))) for g in groups}, "order": groups, "summary": summ,
            "box_modes": {k: excl[k] for k in ("points", "excluded", "by_reason", "asym_left_out")} | {"by_f": {f"{k:g}": n for k, n in excl["by_f"].items()}},
            "outliers": [{"c": o[0], "id": o[1]["id"], "q": o[2], "e": sig(o[3], 3)} for o in outl],
            "limits": {"solver": a.outlier, "formula": a.formula_outlier},
            "cst_version": next((r.get("cst_version") for r in recs if r.get("cst_version")), None)}
    js = json.dumps(data, separators=(",", ":"), ensure_ascii=False).replace("</", "<\\/")
    fonts = []
    for fam, file, w in (("Barlow", "barlow-latin-400-normal.woff2", 400), ("Barlow", "barlow-latin-600-normal.woff2", 600),
                         ("Barlow Semi Condensed", "barlow-semi-condensed-latin-600-normal.woff2", 600),
                         ("Barlow Semi Condensed", "barlow-semi-condensed-latin-700-normal.woff2", 700),
                         ("JetBrains Mono", "jetbrains-mono-latin-400-normal.woff2", 400)):
        b = base64.b64encode(open(os.path.join(ROOT, "web", "fonts", file), "rb").read()).decode()
        fonts.append(f'@font-face{{font-family:"{fam}";font-weight:{w};font-display:swap;src:url(data:font/woff2;base64,{b}) format("woff2")}}')
    tpl = open(os.path.join(HERE, "viewer_template.html"), encoding="utf-8").read()
    html = tpl.replace("/*FONTS*/", "\n".join(fonts)).replace("__DATA__", js)
    if "--fragment" in sys.argv:  # page fragment for a host that adds the skeleton
        html = '<meta name="claude:viewport-fit-sides" content="cover">\n' + html
    else:
        html = ('<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">'
                "</head><body>\n" + html + "\n</body></html>\n")
    os.makedirs(os.path.dirname(os.path.abspath(OUT)), exist_ok=True)
    open(OUT, "w", encoding="utf-8").write(html)
    print(f"{OUT}: {len(out_cases)} cases, {sum(1 for c in out_cases if c['loss'])} with a loss sweep, {len(html) // 1024} KB")


if __name__ == "__main__":
    main()
