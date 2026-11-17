"""Build dist/kisi-references-3d.html (not in the repository; built locally and by the release workflow):
a self-contained page with the geometry and the results of every via reference case (validation/via_ref), the solver
checks (results/openems_refs.json, results/cst_compare.json) and the accuracy of kisi's models (results/kisi_models.json,
results/via_ladder.json, data/accuracy.json). Method: validation/vias.md.

usage: python3 make_viewer.py [via_ref_dir] [out.html] [--fragment]
"""
import base64, glob, json, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from cst_compare import CST_MESHES, CST_MESH_NAME, cst_fixture, cst_tools, result_file  # noqa: E402  (one mesh rule for page and comparison)

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
REF = ARGS[0] if len(ARGS) > 0 else os.path.join(ROOT, "validation", "via_ref")
OUT = ARGS[1] if len(ARGS) > 1 else os.path.join(ROOT, "dist", "kisi-references-3d.html")
CASES = os.path.join(HERE, "via_cases")
STEP = 2  # every 2nd frequency point


def r2(a, d=2):
    return [None if not np.isfinite(x) else round(float(x), d) for x in a]


def cst_mesh(name):
    """the CST result shown for a case: its finest mesh (CST_MESHES) → (file prefix, mesh key) or (None, None)"""
    t = cst_tools(REF, name)
    return (t[0], dict(CST_MESHES)[t[0]]) if t else (None, None)


def mesh_label(mesh):
    """'extra-fine mesh' for the key 'xfine' of results/cst_compare.json"""
    return CST_MESH_NAME.get(mesh, f"{mesh} mesh")


def lines_text(lines):
    """CST's mesh settings from the result's *_meta.json"""
    return f"{lines[0]:g} lines per wavelength at f max, {lines[1]:g} per model box"


def load(name, tool):
    if tool == "cst":
        tool = cst_mesh(name)[0] or "cst"
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


DEEMBED_MATCHED = ("2×thru per port, matched as in the accuracy scores (sparam.deembed_matched_nport; validation/vias.md): each port is "
                   "referred to its feed line's own impedance Zc(f) before the NZC fixture of its 2×thru is removed, because the 3D feed lines "
                   "are not 50 Ω at low frequency (their dielectric loss is a conductivity fixed at f_loss)")
DEEMBED_SHIFT = ("2×thru, first order (feed delay and loss per port, no reflection): the feeds of a coupled line are the coupled line "
                 "itself, which the matched per-port de-embedding does not model")
TDR_DE = dict(rise=25e-12, tmax=80e-12)  # the via alone: edge and window of the accuracy scores (via_ladder.py, compare_kisi.py)


def deembed(f, S, fixtures, group):
    """the case with each port's feed removed by its 2×thru → (D, label, Z∞ per port or None): the matched
    de-embedding of the accuracy scores (via_ladder.py, compare_kisi.py); the coupled lines of group T, whose feeds
    are coupled, with the first-order shift"""
    T = [x[: len(f)] for x in fixtures]
    if S.shape[1] == 4 and group == "T":
        return sparam.deembed_shift(S, [sparam.fixture_shift(f, x) for x in T]), DEEMBED_SHIFT, None
    D, zinf = sparam.deembed_matched_nport(f, S, T)
    return D, DEEMBED_MATCHED, zinf


def tdr_de(f, S, diff=False):
    """TDR of port 1 of a de-embedded result or of a via model, 25 ps edge, 80 ps: Z of a 2-port; of a 4-port, and of
    a pair's differential-mode 2-port (diff), Zdiff (100 Ω) → (t ps, Z)"""
    if S.shape[1] == 4:
        S, diff = sparam.mixed_mode(S)["dd"], True
    t, z = sparam.tdr(f, S[:, 0, 0], z0=100.0 if diff else 50.0, **TDR_DE)
    return t * 1e12, z


def fmt3(v):
    """[median, p90, max] → 'median / p90 / max'"""
    return " / ".join(f"{x:g}" for x in v) if v else "–"


def accuracy_check(km, vl, acc):
    """the accuracy of kisi's models: data/accuracy.json's via numbers with the scores they come from
    (results/kisi_models.json: kisi's channel, compare_kisi.py), the via ladder alone against the three solvers
    (results/via_ladder.json, via_ladder.py), and kisi's channel per case"""
    sm, av = km.get("summary") or {}, acc.get("vias", {})
    rows = [["via ladder, single vias (" + str(sm.get("via_cases", "–")) + ")", "TDR error median / p90 (%)", f"{sm.get('via_tdr_err_pct_median')} / {sm.get('via_tdr_err_pct_p90')}",
             f"{av.get('ladder_median_pct')} / {av.get('ladder_p90_pct')}", "results/kisi_models.json"],
            ["", "max Δ|S21| to 20 GHz, median (dB)", sm.get("via_dS21_max_db_median"), "–", ""],
            ["", "|Δτ| median (ps)", sm.get("via_abs_dtau_ps_median"), "–", ""],
            ["", "stub notch", ", ".join(f"{k} {v:+.1f} %" for k, v in (sm.get("stub_err_pct") or {}).items()), "–", ""],
            ["via ladder, pairs (" + str(sm.get("pair_cases", "–")) + ")", "Zdiff TDR error median / p90 (%)", f"{sm.get('pair_zdiff_err_pct_median')} / {sm.get('pair_zdiff_err_pct_p90')}",
             f"{av.get('pair_median_pct')} / {av.get('pair_p90_pct')}", "results/kisi_models.json"],
            ["", "max ΔSdd21 to 20 GHz, median (dB) · phase (°)", f"{sm.get('pair_dSdd21_max_db_median')} · {sm.get('pair_dphase_max_deg_median')}", "–", ""],
            ["", "|Δτ| median (ps)", sm.get("pair_abs_dtau_ps_median"), "–", ""],
            ["coaxial fallback (vias without a plane)", "TDR error median / p90 (%)", "not scored on the current references",
             f"{av.get('coax_median_pct')} / {av.get('coax_p90_pct')}", "results/kisi_models_coax.json (the former references)"]]
    text = ["kisi's channel model as kisi runs it (each case as a mini KiCad board, kisi_board.py, run through kisi net with formula "
            "impedances) with the via alone cut out: kisi's channel de-embedded with kisi's own 2×thru, the 3D result (kisi-fdtd) with the 3D "
            "2×thru, both with the matched de-embedding of the case pages; TDR with a 25 ps edge in an 80 ps window, minimum and maximum "
            "pooled; |S21| and the phase up to 20 GHz. These scores are data/accuracy.json's via numbers (the column “data/accuracy.json”), "
            "the uncertainty kisi states for its via models. Script: compare_kisi.py (CI: tools/golden.py vias); method: validation/vias.md.",
            "The coaxial fallback is the former via model, now used only for vias that cross no plane; its numbers were scored against the "
            "references and the de-embedding of that time and are kept as data/accuracy.json's source until it is scored again."]
    out = [{"title": "Accuracy of kisi's via models", "text": text,
            "cols": ["model", "metric", "now (kisi's channel)", "data/accuracy.json", "stored in"], "rows": rows}]
    s = vl.get("summary") or {}
    name = {"kisi": "kisi-fdtd", "openems": "openEMS (cell at metal halved)", "cst": "CST (finest mesh)"}
    lrows = []
    for tool in ("kisi", "openems", "cst"):
        for st in ("single", "pairs"):
            x = (s.get(tool) or {}).get(st)
            if not x:
                continue
            lrows.append([name[tool], f"{'single vias' if st == 'single' else 'pairs (Sdd)'} ({x['cases']})", fmt3(x["tdr_err_pct"]), f"{x['tdr_min_err_pct_signed_median']:+g}",
                          fmt3(x["dS21_max_db"][:2]), fmt3(x["abs_dtau_ps"][:2])])
    cg = s.get("cavity_G") or {}
    ltext = ["The ladder alone (kisi_si::via_pbm as kisi builds it, ports at the via; `kisi-validate models`) against the de-embedded "
             "result of each 3D solver, each with its own 2×thru fixtures and the matched de-embedding (pairs: the differential mode). "
             "The spread between the three rows of a set is the uncertainty of the references themselves. Script: via_ladder.py."]
    if cg:
        ltext.append(f"kisi's plane-cavity model (kisi_pi::cavity_modal) on group G: the |Z11| resonances up to 4.5 GHz within "
                     f"{cg['resonance_err_pct_max']} % of kisi-fdtd, |Z21| below 1 GHz {' / '.join(f'{x:g}' for x in cg['Z21_below_1GHz_mean_ratio'])} "
                     "of the 3D value (curves on the G case pages).")
    out.append({"title": "The via ladder alone against the three 3D solvers", "text": ltext,
                "cols": ["reference", "set", "TDR error median / p90 / max (%)", "TDR minimum, signed median (%)", "max Δ|S21| to 20 GHz, median / p90 (dB)",
                         "|Δτ| median / p90 (ps)"], "rows": lrows})
    rows = []
    for v in km.get("rows", []):
        if "next_3d_pct" in v:
            rows.append([v["case"], "NEXT / FEXT 3D (%)", f"{v['next_3d_pct']} / {v['fext_3d_pct']}", "kisi (%)", f"{v['next_kisi_pct']} / {v['fext_kisi_pct']}", ""])
        elif "zc_3d_5ghz" in v:
            rows.append([v["case"], "Zc at 5 GHz 3D / kisi (Ω)", f"{v['zc_3d_5ghz']} / {v['zc_kisi']}", "εeff 3D / kisi", f"{v['eeff_3d_5ghz']} / {v['eeff_kisi']}", f"kisi: {v.get('kisi_source', '')}"])
        elif "dSdd21_max_db" in v:
            via = f"via pair alone: Zdiff TDR error {v['via_zdiff_err_pct'][0]} / {v['via_zdiff_err_pct'][1]} %" if v.get("via_zdiff_err_pct") else ""
            rows.append([v["case"], "max ΔSdd21 to 20 GHz", v["dSdd21_max_db"], "Zdiff min 3D / kisi", f"{v['zdiff_min_3d']} / {v['zdiff_min_kisi']}",
                         "; ".join(x for x in (via, f"Scd21 {v['Scd21_max_3d_db']} / {v['Scd21_max_kisi_db']} dB") if x)])
        else:
            via = f"via alone: TDR error {v['via_tdr_err_pct'][0]} / {v['via_tdr_err_pct'][1]} %, max Δ|S21| {v['via_dS21_max_db']} dB" if v.get("via_tdr_err_pct") else ""
            rows.append([v["case"], f"max Δ|S21| to {v['f_limit_ghz']} GHz", v["dS21_max_db"], "TDR Z min/max 3D | kisi", f"{v['z_min_3d']}/{v['z_max_3d']} | {v['z_min_kisi']}/{v['z_max_kisi']}",
                         "; ".join(x for x in (via, f"stub notch 3D {v['stub_3d_ghz']} GHz, kisi {v.get('stub_kisi_ghz') or '–'} GHz" if v.get("stub_3d_ghz") else "", v.get("kisi_return") or "") if x)])
    out.append({"title": "kisi's channel per case",
                "text": ["Every case through kisi's channel (formula impedances, kisi's via model), ports at the ends of the feed lines as in 3D, up to "
                         "kisi's 25 GHz. The full-channel TDR (35 ps edge) includes the low-frequency offset of the 3D lumped ports (a 49 Ω line reads "
                         "43–45 Ω after 100 ps), which is why the via is scored alone (above). Coupling: the peak of the 35 ps step response at the "
                         "victim's near and far end. Script: compare_kisi.py."],
                "cols": ["case", "metric", "value", "metric", "3D / kisi", "note"], "rows": rows})
    return out


OK_THRU_DB, OK_DELAY_PS = 0.1, 1.0  # per-case agreement shown as "ok": mean through-path difference and delay difference


def agreement(r):
    """the chip of a case: kisi-fdtd against openEMS on the refined mesh (openems_refs.py)"""
    if not r:
        return None
    s = r["kisi_vs_openems"]
    conv = r.get("openems_base_vs_fine", {}).get("dS_thru_mean_db")
    title = (f"kisi-fdtd (conformal) against openEMS (staircase, cell at metal halved) up to {r['fmax_ghz']:g} GHz: mean / max difference of the "
             f"through path, delay difference; openEMS reference mesh → refined mesh: {conv} dB mean (its remaining staircase error, about as large)")
    if r["group"] in ("G", "H"):
        if r["group"] == "G":
            pk, po = r["z_peaks_ghz"]["kisi"][:3], r["z_peaks_ghz"]["openems"][:3]
            dev = max((abs(a / b - 1) * 100 for a, b in zip(pk, po)), default=0)
            return {"ok": dev < 1.0, "title": title, "text": f"openEMS: first resonances {' / '.join(f'{x:g}' for x in po)} GHz (kisi-fdtd {' / '.join(f'{x:g}' for x in pk)}), max {dev:.1f} %"}
        lk, lo = r["L50_nH"]["kisi"], r["L50_nH"]["openems"]
        return {"ok": abs(lk / lo - 1) < 0.03, "title": title, "text": f"openEMS: loop L at 50 MHz {lo:.3f} nH (kisi-fdtd {lk:.3f}, {(lk / lo - 1) * 100:+.1f} %)"}
    key = "Sdd21" if r.get("pair") and "dSdd21_mean_db" in s else "thru"
    mean, mx = (s["dSdd21_mean_db"], s["dSdd21_max_db"]) if key == "Sdd21" else (s["dS_thru_mean_db"], s["dS_thru_max_db"])
    nm = "Sdd21" if key == "Sdd21" else "S21" if r["ports"] == 2 else "through"
    return {"ok": mean <= OK_THRU_DB and abs(s["dtau_ps"]) <= OK_DELAY_PS, "title": title,
            "text": f"openEMS: Δ|{nm}| {mean:.2f} / {mx:.2f} dB, Δτ {s['dtau_ps']:+.1f} ps"}


def agreement_cst(r, oe):
    """the chip of a case: kisi-fdtd against CST on CST's finest mesh (results/cst_compare.json), with the metrics and
    thresholds of the openEMS chip"""
    if not r:
        return None
    mesh = mesh_label(r.get("mesh"))
    lines = f" ({lines_text(r['mesh_lines'])})" if r.get("mesh_lines") else ""
    if "pdn_vs_kisi" in r:
        p = r["pdn_vs_kisi"]  # [CST, kisi-fdtd]
        title = f"kisi-fdtd against CST Studio Suite (FIT, conformal metal) on its {mesh}{lines}; method: validation/vias.md"
        if "Z11_peaks_GHz" in p:
            pc, pk = p["Z11_peaks_GHz"][0][:3], p["Z11_peaks_GHz"][1][:3]
            dev = max((abs(a / b - 1) * 100 for a, b in zip(pk, pc)), default=0)
            return {"ok": dev < 1.0, "title": title, "text": f"CST ({mesh}): first resonances {' / '.join(f'{x:g}' for x in pc)} GHz, max {dev:.1f} %"}
        lc, lk = p["L50_nH"]
        return {"ok": abs(lk / lc - 1) < 0.03, "title": title, "text": f"CST ({mesh}): loop L at 50 MHz {lc:.3f} nH ({(lk / lc - 1) * 100:+.1f} %)"}
    s = r.get("chip_kisi_vs_cst")
    if not s:
        return None
    title = (f"kisi-fdtd (conformal) against CST Studio Suite (FIT, conformal metal) on its {mesh}{lines}, up to the case's f max: mean / max difference "
             f"of the through path, delay difference τ kisi-fdtd − τ CST; method: validation/vias.md")
    pair = (oe or {}).get("pair", False)
    key = "Sdd21" if pair and "dSdd21_mean_db" in s else "thru"
    mean, mx = (s["dSdd21_mean_db"], s["dSdd21_max_db"]) if key == "Sdd21" else (s["dS_thru_mean_db"], s["dS_thru_max_db"])
    nm = "Sdd21" if key == "Sdd21" else "S21" if (oe or {}).get("ports", 2) == 2 else "through"
    return {"ok": mean <= OK_THRU_DB and abs(s["dtau_ps"]) <= OK_DELAY_PS, "title": title,
            "text": f"CST ({mesh}): Δ|{nm}| {mean:.2f} / {mx:.2f} dB, Δτ {s['dtau_ps']:+.1f} ps"}


def cst_check(d, oe):
    """the table of the solver checks: kisi-fdtd against CST per group, and openEMS against CST"""
    def grp(cid):
        return "thru" if cid.startswith("thru") else "board" if cid.startswith("BV") else (oe.get("cases", {}).get(cid) or {}).get("group") or cid.split("_")[0]

    def med(v):
        return round(float(np.median(v)), 3) if v else "–"
    groups = {}
    for cid, r in d.items():
        if cid == "note" or not isinstance(r, dict):
            continue
        groups.setdefault(grp(cid), []).append((cid, r))
    order = list(oe.get("summary", {}).keys()) or sorted(groups)
    rows = []
    for g in order + [g for g in sorted(groups) if g not in order]:
        rr = groups.get(g)
        if not rr:
            continue
        meshes = ", ".join(f"{sum(1 for _, r in rr if r.get('mesh') == m)} {CST_MESH_NAME[m].replace(' mesh', '')}" for _, m in CST_MESHES
                           if any(r.get("mesh") == m for _, r in rr))
        if g in ("G", "H"):
            if g == "G":
                dev = []
                for _, r in rr:
                    pc, pk = r["pdn_vs_kisi"]["Z11_peaks_GHz"]
                    dev.append(max(abs(a / b - 1) * 100 for a, b in zip(pk[:3], pc[:3])))
                cell = f"first three |Z11| resonances within {max(dev):.1f} %"
            else:
                dl = [(r["pdn_vs_kisi"]["L50_nH"][1] / r["pdn_vs_kisi"]["L50_nH"][0] - 1) * 100 for _, r in rr]
                cell = f"loop L at 50 MHz {min(dl):+.1f} … {max(dl):+.1f} %"
            rows.append([g, len(rr), meshes, cell, "–", "–", "–"])
            continue
        k = [r["chip_kisi_vs_cst"] for _, r in rr if "chip_kisi_vs_cst" in r]
        o = [r["chip_openems_vs_cst"] for _, r in rr if "chip_openems_vs_cst" in r]
        rows.append([g, len(rr), meshes,
                     f"{med([x['dS_thru_mean_db'] for x in k])} / {max(x['dS_thru_mean_db'] for x in k):.3f} / {max(x['dS_thru_max_db'] for x in k):.3f}" if k else "–",
                     med([x["dtau_ps"] for x in k]) if k else "–",
                     f"{med([x['dS_thru_mean_db'] for x in o])} / {max(x['dS_thru_mean_db'] for x in o):.3f}" if o else "–",
                     med([x["dtau_ps"] for x in o]) if o else "–"])
    # the meshes shown, finest first, with their settings from the *_meta.json files (cst_compare.py: "mesh_lines")
    used = [(m, sorted({tuple(r["mesh_lines"]) for r in d.values() if isinstance(r, dict) and r.get("mesh") == m and r.get("mesh_lines")}),
             sum(1 for r in d.values() if isinstance(r, dict) and r.get("mesh") == m)) for _, m in CST_MESHES]
    used = [(m, ls, n) for m, ls, n in used if n]
    role = {"xfine": "where computed (CST's mesh check)", "fine": "where it stays within the 1 M cell limit",
            "mid": "where the fine mesh does not fit", "base": "where neither does"}
    meshes = "; ".join(f"{CST_MESH_NAME[m].replace(' mesh', '')} ({' or '.join(f'{a:g} / {b:g}' for a, b in ls) or 'settings not recorded'}) "
                       f"{role.get(m, '')}: {n} case{'s' if n != 1 else ''}" for m, ls, n in used)
    text = ["Every reference case computed a third time with CST Studio Suite 2025.5 (time domain, FIT with PBA conformal metal, discrete face "
            "ports, open boundaries) from the same case files, on CST's own mesh, the finest computed for each case (lines per wavelength at f max / "
            f"per model box): {meshes}. A case is de-embedded with the 2×thru on its own mesh, or on the thru's finest where the thru "
            "lacks that mesh. Compared with kisi-fdtd's references as they are since "
            "the refined inner-layer exits, with the metrics of the openEMS table (through path S21, of a pair S31; Δτ = τ kisi-fdtd (or openEMS) − τ CST "
            "from the phase above f max / 4; G: |Z11| resonances, H: loop inductance at 50 MHz). Script: cst_compare.py (results/cst_compare.json); "
            "setup, meshes and findings: validation/vias.md."]
    return {"title": "Every reference case in CST", "text": text,
            "cols": ["group", "cases", "CST mesh (cases)", "Δ|S21| kisi-fdtd vs CST: median of the case means / largest mean / largest max (dB); G, H: as noted",
                     "Δτ kisi-fdtd − CST, median (ps)", "Δ|S21| openEMS fine vs CST: median of the case means / largest mean (dB)", "Δτ openEMS − CST, median (ps)"],
            "rows": rows}


MESH_NAME = {"openems": "refined mesh", "openems_base": "base mesh"}


def openems_check(d):
    """the table of the solver checks: kisi-fdtd against openEMS per group"""
    rows = []
    for g, s in d["summary"].items():
        a, b, c_, x = (s.get(k, {}) for k in ("kisi_vs_openems", "openems_base_vs_fine", "stair_vs_openems_base", "kisi_vs_openems_extrap"))
        rows.append([g, s["cases"], f"{a.get('dS_thru_mean_db')} / {a.get('dS_thru_mean_db_max')} / {a.get('dS_thru_max_db')}" if a else "–",
                     a.get("dtau_ps_median", "–"), f"{c_.get('dS_thru_mean_db')} / {c_.get('dS_thru_mean_db_max')}" if c_ else "–",
                     b.get("dS_thru_mean_db", "–") if b else "–", x.get("dS_thru_mean_db", "–") if x else "–"])
    text = ["Every reference case computed again with openEMS 0.37.0 (staircase metal, UPML) from the same case files: on the mesh of the "
            "kisi-fdtd run (“base”, together with kisi-fdtd staircase on it: the like-for-like check of the implementation) and with the cell at "
            "metal halved (“fine”: staircase converges at first order, so this halves openEMS' metal error and base → fine estimates what remains). "
            "kisi-fdtd's conformal reference is compared with openEMS fine; G and H (one-edge ports) on the base mesh. Through path: S21, of a pair S31; "
            "Δτ = τ kisi-fdtd − τ openEMS from the phase above f max / 4. Script: openems_refs.py (results/openems_refs.json); method, outliers and causes: validation/vias.md."]
    inv = d.get("invalid") or {}
    if inv:
        text.append("Invalid openEMS runs (left out of the table, judged on the other mesh where it is valid): " +
                    "; ".join(f"{n} ({', '.join(f'{MESH_NAME.get(t, t)}: {w}' for t, w in v.items())})" for n, v in sorted(inv.items())) + ".")
    return {"title": "Every reference case in openEMS", "text": text,
            "cols": ["group", "cases", "Δ|S21| kisi-fdtd vs openEMS fine: median of the case means / largest mean / largest max (dB)", "Δτ median (ps)",
                     "same mesh, both staircase: median / largest mean (dB)", "openEMS base → fine: median (dB)", "against openEMS extrapolated (2·fine − base): median (dB)"],
            "rows": rows}


def main():
    # 2×thru fixtures per tool: each tool's result is de-embedded with its own fixtures
    thrus = {"kisi": {}, "openems": {}}  # CST: the fixture on the case's own mesh, below
    for p in glob.glob(os.path.join(REF, "thru_*")):
        nm = os.path.basename(p)
        for tool, d in thrus.items():
            f, T = load(nm, tool)
            if T is not None:
                d[nm] = (f, T)
    thru = thrus["kisi"]
    po = os.path.join(HERE, "results", "openems_refs.json")
    oe_refs = json.load(open(po)) if os.path.exists(po) else {}
    pc = os.path.join(HERE, "results", "cst_compare.json")
    cst_refs = json.load(open(pc)) if os.path.exists(pc) else {}
    cases = []
    for cp in sorted(glob.glob(os.path.join(CASES, "*.json"))):
        c = json.load(open(cp))
        nm = c["name"]
        f, S = load(nm, "kisi")
        entry = {"id": nm, "case": {k: c[k] for k in ("domain", "board", "layers", "sheets", "vias", "ports", "fmax")}, "meta": c["meta"]}
        ctool, cmesh = cst_mesh(nm)
        if cmesh:
            entry["cst_mesh"] = mesh_label(cmesh)
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
            for tool in ("kisi_base", "openems", "openems_base", "cst", "kisi_model", "ladder", "model"):
                f2, S2 = load(nm, tool)
                if S2 is None or (S2.shape[1] != S.shape[1] and tool != "ladder"):
                    continue
                if tool == "ladder" and S.shape[1] == 4:  # differential-mode model of a pair
                    res[tool] = {"Sdd21": r2(sparam.db(S2[::STEP, 1, 0])), "Sdd11": r2(sparam.db(S2[::STEP, 0, 0]))}
                    continue
                res[tool] = curves(S2, grp, f2, z0)
            # de-embedded with the 2×thru of each port's feed layer
            if grp not in ("thru", "G", "H"):
                fx = []
                for p in c["meta"]["ports"]:
                    key = f"thru_{c['meta']['stackup']}_{p['layer']}"
                    fx.append(thru.get(key))
                other = [("openems_de", load(nm, "openems")[1], [thrus["openems"].get(f"thru_{c['meta']['stackup']}_{p['layer']}") for p in c["meta"]["ports"]])]
                if ctool:  # CST: each 2×thru on the case's mesh where it has it (cst_compare.cst_fixture), as in results/cst_compare.json
                    names = [f"thru_{c['meta']['stackup']}_{p['layer']}" for p in c["meta"]["ports"]]
                    fmesh = [cst_fixture(REF, t, ctool) for t in names]
                    fxc = [sparam.read_touchstone(result_file(REF, t, m)) if m else None for t, m in zip(names, fmesh)]
                    other.append(("cst_de", sparam.read_touchstone(result_file(REF, nm, ctool))[1], fxc))
                    off = sorted({f"{t.split('_')[-1]} on the {CST_MESH_NAME[dict(CST_MESHES)[m]]}" for t, m in zip(names, fmesh) if m and m != ctool})
                    if off:
                        entry["cst_de_note"] = f"2×thru {', '.join(off)}"
                tdr = {}
                for tag, SS, fxs in [("deembedded", S, fx)] + other:
                    if SS is None or not all(fxs) or SS.shape[1] != S.shape[1] or len(SS) < len(f):
                        continue
                    D, entry["deembed"], zinf = deembed(f, SS[: len(f)], [x[1] for x in fxs], grp)
                    res[tag] = curves(D)
                    tdr[tag] = tdr_de(f, D)
                    if tag == "deembedded" and zinf:
                        entry["deembed_zc"] = [round(z, 1) for z in zinf]  # kisi-fdtd's feed lines, Z∞ of the Zc(f) fit
                if tdr:
                    # the via models next to it (ports at the via; a pair's model is its differential mode)
                    for tool in ("ladder",):
                        f2, S2 = load(nm, tool)
                        if S2 is not None and len(f2) == len(f) and np.allclose(f2, f):
                            tdr[tool] = tdr_de(f, S2, diff=S.shape[1] == 4)
                    t = next(iter(tdr.values()))[0]
                    entry["tdr_de"] = {"t": r2(t, 1), "diff": S.shape[1] == 4, **{k: r2(v[1], 2) for k, v in tdr.items()}}
            if grp == "T" and nm.endswith("_20") and S.shape[1] == 2:
                f10, S10 = load(nm[:-3] + "_10", "kisi")
                tl = sparam.line_params(f, S, S10 if S10 is not None and len(f10) == len(f) else None)
                entry["tl"] = {"source": tl["source"], "zc": r2(tl["zc"][::STEP], 2), "eeff": r2(tl["eeff"][::STEP], 3), "loss_db_per_mm": r2(tl["loss_db_per_mm"][::STEP], 4)}
            ag = agreement(oe_refs.get("cases", {}).get(nm))
            if ag:
                entry["agree"] = ag
            agc = agreement_cst(cst_refs.get(nm), oe_refs.get("cases", {}).get(nm))
            if agc:
                entry["agree_cst"] = agc
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
            _, So = load(nm, "openems")
            if So is not None and So.shape == S.shape:
                entry["tdr"]["z_oe"] = r2(sparam.tdr(f, So[:, 0, 0], rise=25e-12, tmax=180e-12)[1][k], 2)
                if S.shape[1] == 4:
                    entry["tdr"]["zdd_oe"] = r2(sparam.tdr(f, sparam.mixed_mode(So)["dd"][:, 0, 0], rise=25e-12, tmax=180e-12, z0=100.0)[1][k], 2)
            entry["res"] = res
        cases.append(entry)
    checks = {}
    if oe_refs:
        checks["openems_refs"] = openems_check(oe_refs)
    if cst_refs:
        checks["cst_refs"] = cst_check(cst_refs, oe_refs)

    def res_json(nm):
        p = os.path.join(HERE, "results", f"{nm}.json")
        return json.load(open(p)) if os.path.exists(p) else {}
    km = res_json("kisi_models")
    if km:
        checks["accuracy"] = accuracy_check(km, res_json("via_ladder"), json.load(open(os.path.join(ROOT, "data", "accuracy.json"))))
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
