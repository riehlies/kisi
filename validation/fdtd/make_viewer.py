"""Build dist/kisi-references.html (not in the repository; built locally and by the release workflow):
a self-contained page with the geometry and the results
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
OUT = ARGS[1] if len(ARGS) > 1 else os.path.join(ROOT, "dist", "kisi-references.html")
CASES = os.path.join(HERE, "via_cases")
STEP = 2  # every 2nd frequency point


def r2(a, d=2):
    return [None if not np.isfinite(x) else round(float(x), d) for x in a]


def load(name, tool):
    for n in (1, 2, 4):
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


def model_check(nm, d):
    """tables of the reduced-model comparisons"""
    if nm == "via_pbm":
        rows = []
        for cid, v in sorted(d.items()):
            if v.get("mode") == "odd":
                rows.append([cid + " (differential)", v["dSdd21_max_db_20GHz"], "–", f"phase error max {v['dphase_max_deg_20GHz']}°", "–", "–"])
                continue
            rows.append([cid, v["dS21_max_db_20GHz"], v["dS11_max_20GHz"], f"{v['z_min_3d']} / {v['z_min_pbm']}", f"{v['z_max_3d']} / {v['z_max_pbm']}",
                         f"{v['notch_3d_ghz'] or '–'} / {v['notch_pbm_ghz'] or '–'}" if (v["notch_3d_ghz"] or 0) < 39 else "–"])
        return {"title": "Reduced via model (prototype) against the de-embedded 3D result",
                "text": ["Physics-based ladder along the barrel: via–plate capacitances from an axisymmetric r–z Laplace solve (pads, antipads, "
                         "layered εr), between the planes the parallel-plate impedance ωμd/4 · H0⁽²⁾(ka) with the GND vias as shorted ports, "
                         "plane pairs split into ≤ 0.2 mm sections, the barrel above the outermost planes as a short coax to the antipad, stubs as "
                         "the open rest of the ladder. Ports at the via; the 3D result is de-embedded with the 2×thru of each feed layer. "
                         "TDR with a 25 ps edge. Script: via_pbm.py.",
                         "kisi now uses this ladder (Rust port kisi_si::via_pbm, see the tables “Rust port” and “kisi's fast channel model”); "
                         "the former coax-section model is the fallback for vias without a plane."],
                "cols": ["case", "max Δ|S21| to 20 GHz (dB)", "max |ΔS11| to 20 GHz", "TDR Z min 3D / model (Ω)", "TDR Z max 3D / model (Ω)", "stub notch 3D / model (GHz)"],
                "rows": rows}
    if nm == "pdn_loop":
        rows = [[r["case"], r["dist_mm"], r["D_mm"], r["L50_nH"], r["R50_mOhm"], r["dL_3d_nH"], r.get("dL_model_nH", "–"), r["dL_formula_nH"]] for r in d["rows"]]
        return {"title": "PDN loop (group H): loop inductance against the plane-spreading formula",
                "text": [f"BGA pin field (4 × 4, 0.8 mm, pins shorted per net by two sheets above the board) to one 0402 capacitor (a short with 5 mΩ "
                         f"on a 0.5 mm body loop), finite 8-layer board, 1 Ω port. Loop inductance Im(Z11)/ω at 50 MHz. The change with the distance "
                         f"is compared with the finite-board plane-cavity model (double cosine series, height d_eff, 2.4 mm port at the pin field, the capacitor vias as a "
                         f"shorted 0.2 mm port) and with the radial formula ΔL = μ0·d_eff/π·ln(D2/D1), d_eff = {d['d_eff_mm']} mm (PWR plane between two GND planes). "
                         "The board is 7 mm wide, so beyond a few mm the current runs as in a strip and the radial formula underestimates ΔL. "
                         "An earlier run with the planes reaching into the PML was wrong: the infinite plane pair bypassed the capacitor. Script: pdn_loop.py."],
                "cols": ["case", "distance (mm)", "D centre (mm)", "L at 50 MHz (nH)", "R at 50 MHz (mΩ)", "ΔL 3D (nH)", "ΔL plane-cavity model (nH)", "ΔL radial formula (nH)"], "rows": rows}
    if nm == "cavity_model":
        rows = []
        for cid, v in d.items():
            for r in v["resonances_GHz_3d_model_errpct"]:
                rows.append([cid, r[0], r[1], f"{r[2]:+.2f} %" if r[2] is not None else "–"])
            rows.append([cid, "|Z11| / |Z21| model ÷ 3D below 1 GHz", f"{v['Z11_below_1GHz_mean_ratio']} / {v['Z21_below_1GHz_mean_ratio']}", ""])
        return {"title": "Plane-cavity model against group G",
                "text": ["Rectangular plane pair with magnetic walls (double cosine series, Okoshi), square ports of one cell, the same dielectric "
                         "loss as in 3D (σ at f_loss). Resonances of |Z11| at the corner port. The frequency grid is 10 MHz (0.7 % at 1.45 GHz). "
                         "Script: cavity_model.py."],
                "cols": ["case", "resonance 3D (GHz)", "model (GHz)", "error"], "rows": rows}
    if nm == "rust_models":
        sm = d["summary"]
        t = sm["via_time_ms_median"]
        rows = [["Python prototype (via_pbm.py)", sm["python"]["single_tdr_err_pct_median"], sm["python"]["single_tdr_err_pct_p90"], sm["python"]["single_dS21_max_db_median"], sm["python"]["pairs_dS21_max_db_median"], "–", f"{t['python']} ms" if t.get("python") else "–"],
                ["Rust, prototype model (uniform grid)", sm["rust"]["single_tdr_err_pct_median"], sm["rust"]["single_tdr_err_pct_p90"], sm["rust"]["single_dS21_max_db_median"], sm["rust"]["pairs_dS21_max_db_median"], f"parity {sm['parity_rust']['dS21_lin_max_max']}", f"{t['rust_build']} + {t['rust_eval']} ms"],
                ["Rust, kisi's ladder (graded grid, equivalent antipad, junction)", sm["rust_graded"]["single_tdr_err_pct_median"], sm["rust_graded"]["single_tdr_err_pct_p90"], sm["rust_graded"]["single_dS21_max_db_median"], sm["rust_graded"]["pairs_dS21_max_db_median"], "model differs", f"{t['rust_graded_build']} + {t['rust_eval']} ms"]]
        cv = sm.get("cavity", {})
        text = ["The reduced models in Rust: the via ladder (kisi_si::via_pbm), the plane cavity and the PDN loop (kisi_pi::cavity_modal). "
                "Same metrics as the prototype: TDR minimum and maximum (25 ps, 80 ps window) against the de-embedded 3D result, both values "
                "of every single-via case pooled; |S21| up to 20 GHz. Parity: largest |ΔS21| (linear) against the Python result over all via cases. "
                "kisi's ladder adds to the prototype the junction model fitted to the 3D references (series inductance μ0/(2π)·pad radius where a trace "
                "enters the via, the trace over the antipad scaled from its own line, striplines kept up to the via centre) and the equivalent radius of "
                "non-round antipads. Time per via for 400 frequencies: build (r–z Laplace solve) + evaluation; the Laplace result is cached per via "
                "geometry, so a board solves it once per via type. Script: rust_models.py."]
        if cv:
            text.append(f"Cavity model: Rust within {cv['dZ_rel_max']:.1e} of Python; {cv['nf']} frequencies × {cv['modes']} modes in "
                        f"{' / '.join(str(x) for x in cv['rust_eval_ms'])} ms (Python {' / '.join(str(x) for x in cv['python_ms'])} ms). "
                        "PDN loop inductance: identical to 0.001 nH, about 1 ms per configuration.")
        return {"title": "Rust port of the reduced models", "text": text,
                "cols": ["model", "TDR error median (%)", "TDR error p90 (%)", "max Δ|S21| median, single (dB)", "max Δ|S21| median, pairs (dB)", "against Python", "time per via"],
                "rows": rows}
    if nm == "practice":
        rows = []
        for key, b in d["boards"].items():
            bd, nc = b["board"], b["native_check_all"]
            nd = b.get("node_check_all_field", {})
            rows.append([key, f"{bd['file_mb']} MB", bd["vias"], f"{b['native_overview']['s']} s", f"{nc['coax_s']} → {nc['ladder_s']} s",
                         f"{nc['ladder_vias']} / {nc['laplace_solves']}", f"{b.get('node_check_all_formula_old', {}).get('s', '–')} → {b.get('node_check_all_formula', {}).get('s', '–')} s",
                         f"{nd.get('cold_s', '–')} / {nd.get('cached_s', '–')} s", len(b["verdicts"]["status_changed"])])
        bk = d["bundle_kb"]
        fd = d["fdtd"]
        text = [f"Example boards on {d['machine']['cpus']} cores ({d['machine']['cpu']}). check --all: every lane of the board with eye, coax via model → "
                "via ladder. Ladder vias / Laplace solves: vias of one type share the solve. The Node CLI runs the same Rust code as WebAssembly; "
                "with the field solver cold (no cache) and cached. Script: practice.py.",
                f"Bundles: kisi.html {bk.get('kisi.html_old')} → {bk.get('kisi.html')} kB, kisi.mjs {bk.get('kisi.mjs_old')} → {bk.get('kisi.mjs')} kB.",
                "3D FDTD (kisi-fdtd, one thread per run): " + ", ".join(f"{g} {v['s_median']} s for {v['cells_median'] / 1e6:.2f} M cells" for g, v in fd["groups"].items()) +
                (f"; memory {fd['measured']['bytes_per_cell']} bytes per cell." if fd.get("measured") else ".")]
        return {"title": "Practical cost on the example boards", "text": text,
                "cols": ["board", "file", "vias", "overview", "check --all (native)", "ladder vias / Laplace solves", "check --all (Node, formula)", "check --all with field solver: 120 s budget / from the cache", "verdicts changed"],
                "rows": rows}
    rows = []
    for v in d.get("rows", []):
        if "next_3d_pct" in v:
            rows.append([v["case"], "NEXT / FEXT 3D (%)", f"{v['next_3d_pct']} / {v['fext_3d_pct']}", "kisi (%)", f"{v['next_kisi_pct']} / {v['fext_kisi_pct']}", ""])
        elif "zc_3d_5ghz" in v:
            rows.append([v["case"], "Zc at 5 GHz 3D / kisi (Ω)", f"{v['zc_3d_5ghz']} / {v['zc_kisi']}", "εeff 3D / kisi", f"{v['eeff_3d_5ghz']} / {v['eeff_kisi']}", f"kisi: {v.get('kisi_source', '')}"])
        elif "dSdd21_max_db" in v:
            rows.append([v["case"], "max ΔSdd21 to 20 GHz", v["dSdd21_max_db"], "Zdiff min 3D / kisi", f"{v['zdiff_min_3d']} / {v['zdiff_min_kisi']}", f"Scd21 {v['Scd21_max_3d_db']} / {v['Scd21_max_kisi_db']} dB"])
        else:
            via = f"via alone: TDR error {v['via_tdr_err_pct'][0]} / {v['via_tdr_err_pct'][1]} %, max Δ|S21| {v['via_dS21_max_db']} dB" if v.get("via_tdr_err_pct") else ""
            rows.append([v["case"], f"max Δ|S21| to {v['f_limit_ghz']} GHz", v["dS21_max_db"], "TDR Z min/max 3D | kisi", f"{v['z_min_3d']}/{v['z_max_3d']} | {v['z_min_kisi']}/{v['z_max_kisi']}",
                         "; ".join(x for x in (via, f"stub notch 3D {v['stub_3d_ghz']} GHz, kisi {v.get('stub_kisi_ghz') or '–'} GHz" if v.get("stub_3d_ghz") else "", v.get("kisi_return") or "") if x)])
    title = "kisi's fast channel model against the 3D result" + (" (Node CLI, 2D field solver)" if nm == "kisi_models_field" else " (formula impedances)")
    text = ["Each case as a mini KiCad board (kisi_board.py), computed by `kisi net … --s2p/--s4p` (formula impedances, kisi's via "
            "model), ports at the ends of the feed lines as in 3D. TDR with a 35 ps edge; coupling as the peak of the step response "
            "(35 ps) at the victim's near and far end. kisi exports S-parameters up to 25 GHz; the comparison stops there. Script: compare_kisi.py.",
            "The full-channel TDR minimum also holds the low-frequency offset of the 3D lumped ports (a 49 Ω line shows 43–45 Ω after 100 ps), "
            "so the via itself is compared de-embedded: kisi's channel with kisi's own 2×thru, 3D with the 3D 2×thru (TDR 25 ps, 80 ps window, up to 25 GHz)."]
    sm = d.get("summary") or {}
    if sm.get("via_cases"):
        old = d.get("_coax") or {}
        text.append(f"Via alone, {sm['via_cases']} single vias: TDR error median {sm['via_tdr_err_pct_median']} %, 90th percentile {sm['via_tdr_err_pct_p90']} %, "
                    f"max Δ|S21| median {sm['via_dS21_max_db_median']} dB" + (f" (former coax model: {old['via_tdr_err_pct_median']} / {old['via_tdr_err_pct_p90']} %, {old['via_dS21_max_db_median']} dB)" if old else "") +
                    ". Stub notch: " + ", ".join(f"{k} {v:+.1f} %" for k, v in (sm.get("stub_err_pct") or {}).items()) + ".")
        if sm.get("pair_cases"):
            text.append(f"Via pairs alone (differential mode, {sm['pair_cases']} cases): Zdiff TDR error median {sm['pair_zdiff_err_pct_median']} %, 90th percentile {sm['pair_zdiff_err_pct_p90']} %, "
                        f"max ΔSdd21 median {sm['pair_dSdd21_max_db_median']} dB, phase {sm['pair_dphase_max_deg_median']}°" +
                        (f" (former coax model: {old.get('pair_zdiff_err_pct_median')} / {old.get('pair_zdiff_err_pct_p90')} %, {old.get('pair_dSdd21_max_db_median')} dB, {old.get('pair_dphase_max_deg_median')}°)" if old.get("pair_cases") else "") + ".")
    return {"title": title,
            "text": text,
            "cols": ["case", "metric", "value", "metric", "3D / kisi", "note"], "rows": rows}


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
        fp = os.path.join(REF, nm, "fields.json")
        if os.path.exists(fp):
            entry["fields"] = json.load(open(fp))["planes"]
        mp = os.path.join(REF, nm, "meta.json")
        if os.path.exists(mp):
            entry["run"] = json.load(open(mp))
        if S is not None:
            entry["f"] = r2(f[::STEP] / 1e9, 3)
            grp = c["meta"]["group"]
            z0 = c["ports"][0].get("r", 50.0)
            res = {"kisi": curves(S, grp, f, z0)}
            for tool in ("kisi_fine", "openems", "kisi_model", "pbm", "ladder", "model"):
                f2, S2 = load(nm, tool)
                if S2 is None or (S2.shape[1] != S.shape[1] and tool not in ("pbm", "ladder")):
                    continue
                if tool in ("pbm", "ladder") and S.shape[1] == 4:  # differential-mode model of a pair
                    res[tool] = {"Sdd21": r2(sparam.db(S2[::STEP, 1, 0])), "Sdd11": r2(sparam.db(S2[::STEP, 0, 0]))}
                    continue
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
            if grp == "T" and nm.endswith("_20") and S.shape[1] == 2:
                f10, S10 = load(nm[:-3] + "_10", "kisi")
                tl = sparam.line_params(f, S, S10 if S10 is not None and len(f10) == len(f) else None)
                entry["tl"] = {"source": tl["source"], "zc": r2(tl["zc"][::STEP], 2), "eeff": r2(tl["eeff"][::STEP], 3), "loss_db_per_mm": r2(tl["loss_db_per_mm"][::STEP], 4)}
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
    for nm in ("via_pbm", "cavity_model", "rust_models", "kisi_models", "kisi_models_field", "pdn_loop", "practice"):
        p = os.path.join(HERE, "results", f"{nm}.json")
        if os.path.exists(p):
            d = json.load(open(p))
            pc = os.path.join(HERE, "results", "kisi_models_coax.json")
            if nm == "kisi_models" and os.path.exists(pc):
                d["_coax"] = json.load(open(pc)).get("summary") or {}
            checks[nm] = model_check(nm, d)
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
    if "--fragment" in sys.argv:  # page fragment for a host that adds the skeleton (side gutter uses safe-area insets)
        html = '<meta name="claude:viewport-fit-sides" content="cover">\n' + html
    else:  # standalone file
        html = ('<!doctype html>\n<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">'
                "</head><body>\n" + html + "\n</body></html>\n")
    os.makedirs(os.path.dirname(os.path.abspath(OUT)), exist_ok=True)
    open(OUT, "w").write(html)
    print(f"{OUT}: {len(cases)} cases, {sum(1 for c in cases if 'res' in c)} with results, {len(html) // 1024} KB")


if __name__ == "__main__":
    main()
