"""Build validation/fdtd/references.html: a self-contained page with the geometry and the results
of every via reference case (validation/via_ref) and the solver checks (results/*.json).

usage: python3 make_viewer.py [via_ref_dir] [out.html] [--fragment]
"""
import base64, glob, json, math, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import sparam  # noqa: E402

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
REF = ARGS[0] if len(ARGS) > 0 else os.path.join(ROOT, "validation", "via_ref")
OUT = ARGS[1] if len(ARGS) > 1 else os.path.join(HERE, "references.html")
CASES = os.path.join(HERE, "via_cases")
STEP = 2  # every 2nd frequency point


def r2(a, d=2):
    return [None if not np.isfinite(x) else round(float(x), d) for x in a]


def load(name, tool):
    for n in (2, 4):
        p = os.path.join(REF, name, f"{tool}.s{n}p")
        if os.path.exists(p):
            return sparam.read_touchstone(p)
    return None, None


def curves(S, group="", f=None, z0=50.0):
    """dB curves to plot: through, reflection, coupling (by port count); Z for the PDN groups"""
    n = S.shape[1]
    if group in ("G", "H"):
        Z = sparam.s2z(S, z0)[::STEP]
        out = {"Z11": r2(np.abs(Z[:, 0, 0]), 4)}
        if n > 1:
            out["Z21"] = r2(np.abs(Z[:, 1, 0]), 4)
        if group == "H":
            out["L11"] = r2(Z[:, 0, 0].imag / (2 * np.pi * f[::STEP]) * 1e9, 4)
            out["R11"] = r2(Z[:, 0, 0].real * 1e3, 2)
        return out
    d = sparam.db(S[::STEP])
    if n == 2:
        return {"S21": r2(d[:, 1, 0]), "S11": r2(d[:, 0, 0]), "S22": r2(d[:, 1, 1])}
    mm = sparam.mixed_mode(S)
    dm = {k: sparam.db(v[::STEP]) for k, v in mm.items()}
    return {"S31": r2(d[:, 2, 0]), "S11": r2(d[:, 0, 0]), "S21": r2(d[:, 1, 0]), "S41": r2(d[:, 3, 0]),
            "Sdd21": r2(dm["dd"][:, 1, 0]), "Sdd11": r2(dm["dd"][:, 0, 0]), "Scd21": r2(dm["cd"][:, 1, 0]), "Scc21": r2(dm["cc"][:, 1, 0])}


def cavity_check(cv):
    rows = cv["rows"]
    tab = []
    for case in ("empty", "post"):
        for cell in sorted({r["cell"] for r in rows if r["case"] == case}, reverse=True):
            st = next((r for r in rows if r["case"] == case and r["cell"] == cell and r["metal"] == "staircase"), None)
            co = next((r for r in rows if r["case"] == case and r["cell"] == cell and r["metal"] == "conformal"), None)
            if not st or not co:
                continue
            tab.append([f"{'empty cavity' if case == 'empty' else 'with centre post'}", f"{cell:g}", f"{st['f_analytic_GHz']:.3f}",
                        f"{st['f_fdtd_GHz']:.3f}", f"{st['error_pct']:+.2f} %", f"{co['f_fdtd_GHz']:.3f}", f"{co['error_pct']:+.2f} %"])
    charts = []
    series = []
    for case, nm in (("empty", "empty"), ("post", "post")):
        for metal, dash in (("staircase", True), ("conformal", False)):
            rr = sorted((r for r in rows if r["case"] == case and r["metal"] == metal), key=lambda r: r["cell"])
            series.append({"name": f"{nm}, {metal}", "y": [abs(r["error_pct"]) for r in rr], "x": [r["cell"] for r in rr], "dash": dash, "marks": True,
                           "color": "var(--s1)" if case == "empty" else "var(--s2)"})
    cells = sorted({r["cell"] for r in rows})
    for s in series:  # align on the common cell list
        s["y"] = [next((y for x, y in zip(s["x"], s["y"]) if x == c), None) for c in cells]
        del s["x"]
    charts.append({"title": "Resonance error against cell size", "x": cells, "series": series, "xlabel": "cell mm", "ylabel": "|error| %",
                   "logx": True, "logy": True, "ymin": 0.01, "ymax": 5,
                   "note": "Staircase converges at first order (error halves with the cell), conformal metal is 7–13 times closer at the same cell."})
    return {"title": "Metal model: cylindrical cavity against the analytic resonance",
            "text": [f"Cylindrical cavity R = {cv['R_mm']} mm, h = {cv['h_mm']} mm, εr {cv['er']}, tanδ {cv['tand']}, without and with a centre post of radius "
                     f"{cv['post_mm']} mm (a via shorting both plates). TM010-type resonance: J0(kR) = 0, with the post J0(ka)Y0(kR) − J0(kR)Y0(ka) = 0. "
                     "Excited by a 50 Ω lumped port between the plates; the resonance is the peak of Re(Z_in). Uniform meshes; the circle is either "
                     "staircased or treated conformally (Dey–Mittra with a stable effective face area)."],
            "cols": ["case", "cell (mm)", "analytic (GHz)", "staircase (GHz)", "error", "conformal (GHz)", "error"], "rows": tab, "charts": charts}


def main():
    thru = {}
    for p in glob.glob(os.path.join(REF, "thru_*")):
        nm = os.path.basename(p)
        f, T = load(nm, "kisi")
        if T is not None:
            thru[nm] = (f, T)
    cases = []
    for cp in sorted(glob.glob(os.path.join(CASES, "*.json"))):
        c = json.load(open(cp))
        nm = c["name"]
        f, S = load(nm, "kisi")
        entry = {"id": nm, "case": {k: c[k] for k in ("domain", "board", "layers", "sheets", "vias", "ports", "fmax")}, "meta": c["meta"]}
        mp = os.path.join(REF, nm, "meta.json")
        if os.path.exists(mp):
            entry["run"] = json.load(open(mp))
        if S is not None:
            entry["f"] = r2(f[::STEP] / 1e9, 3)
            grp = c["meta"]["group"]
            z0 = c["ports"][0].get("r", 50.0)
            res = {"kisi": curves(S, grp, f, z0)}
            for tool in ("kisi_fine", "openems"):
                f2, S2 = load(nm, tool)
                if S2 is not None:
                    res[tool] = curves(S2, grp, f2, z0)
            # de-embedded with the 2×thru of each port's feed layer
            if grp not in ("thru", "G", "H"):
                fx = []
                for p in c["meta"]["ports"]:
                    key = f"thru_{c['meta']['stackup']}_{p['layer']}"
                    fx.append(thru.get(key))
                if all(fx):
                    if S.shape[1] == 2:
                        FL, _ = sparam.fixture_2xthru(f, fx[0][1])
                        FR, _ = sparam.fixture_2xthru(f, fx[1][1])
                        D = sparam.deembed2(S, FL, FR)
                        res["deembedded"] = curves(D)
                        entry["deembed"] = "2×thru (IEEE P370-style, NZC)"
                    else:
                        D = sparam.deembed_shift(S, [sparam.fixture_shift(f, x[1]) for x in fx])
                        res["deembedded"] = curves(D)
                        entry["deembed"] = "2×thru, first order (feed delay and loss per port)"
            if grp in ("G", "H"):
                entry["res"] = res
                cases.append(entry)
                continue
            # TDR of port 1 (raw: feed line, via, feed line), 25 ps edge
            t, z = sparam.tdr(f, S[:, 0, 0], rise=25e-12, tmax=180e-12)
            k = np.linspace(0, len(t) - 1, 240).astype(int)
            entry["tdr"] = {"t": r2(t[k] * 1e12, 1), "z": r2(z[k], 2)}
            if S.shape[1] == 4:
                mm = sparam.mixed_mode(S)
                t, z = sparam.tdr(f, mm["dd"][:, 0, 0], rise=25e-12, tmax=180e-12, z0=100.0)
                entry["tdr"]["zdd"] = r2(z[k], 2)
            entry["res"] = res
        cases.append(entry)
    checks = {}
    for nm in ("cavity", "pml_thru", "via_convergence", "openems_compare"):
        p = os.path.join(HERE, "results", f"{nm}.json")
        if os.path.exists(p):
            checks[nm] = json.load(open(p))
    if "cavity" in checks and "rows" in checks["cavity"] and "cols" not in checks["cavity"]:
        checks["cavity"] = cavity_check(checks["cavity"])
    data = json.dumps({"cases": cases, "checks": checks}, separators=(",", ":"))
    fonts = []
    for fam, file, w in (("Barlow", "barlow-latin-400-normal.woff2", 400), ("Barlow", "barlow-latin-600-normal.woff2", 600),
                         ("Barlow Semi Condensed", "barlow-semi-condensed-latin-600-normal.woff2", 600),
                         ("Barlow Semi Condensed", "barlow-semi-condensed-latin-700-normal.woff2", 700),
                         ("JetBrains Mono", "jetbrains-mono-latin-400-normal.woff2", 400)):
        b = base64.b64encode(open(os.path.join(ROOT, "web", "fonts", file), "rb").read()).decode()
        fonts.append(f'@font-face{{font-family:"{fam}";font-weight:{w};font-display:swap;src:url(data:font/woff2;base64,{b}) format("woff2")}}')
    tpl = open(os.path.join(HERE, "viewer_template.html")).read()
    html = tpl.replace("/*FONTS*/", "\n".join(fonts)).replace("__DATA__", data)
    if "--fragment" not in sys.argv:  # standalone file; the artifact host adds this skeleton itself
        html = ('<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">'
                "</head><body>\n" + html + "\n</body></html>\n")
    open(OUT, "w").write(html)
    print(f"{OUT}: {len(cases)} cases, {sum(1 for c in cases if 'res' in c)} with results, {len(html) // 1024} KB")


if __name__ == "__main__":
    main()
