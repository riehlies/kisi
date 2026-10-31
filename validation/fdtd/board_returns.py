"""Return groups per plane pair: which vias carry a via's return current through each plane pair it
crosses, in 3D (kisi-fdtd) against kisi's ladder before and with return groups
(validation/fdtd/BOARD_RETURNS.md).

  python3 board_returns.py cases                write return_cases/*.json (cases and 2×thru)
  python3 board_returns.py mesh                 cells and smallest cell of every case (no run)
  python3 board_returns.py run [--only K]       kisi-fdtd, one case at a time → via_ref/<case>/
  python3 board_returns.py model [--only K]     kisi's ladder per case: before return groups (shorts in
                                                every pair a GND via spans) and with them, each on
                                                infinite plates and with the finite cavities
  python3 board_returns.py compare [--only K]   de-embedding, resonances, plots,
                                                results/board_returns.json
  python3 board_returns.py board [--only K]     ThunderScope board vias: 3D with In2 as it is (a supply
                                                plane) and with In2 grounded, against the ladders
  python3 board_returns.py summary              table of results/board_returns.json

Synthetic cases (finite boards 24 × 16 mm with open edges in air, as board_resonances.py):
  RG_s4_pwr_n4       4-layer s4 (L1 / GND L2 / 1.2 mm core / PWR L3 / L4), via L1 → L4 (the L4 trace
                     refers to the PWR plane), four GND vias at 1 mm joined to L2 only (openings in
                     L3): the GND vias do not short the GND/PWR pair (the R_s4_rect_n4 case of Block 3
                     with L3 a supply plane)
  RG_s4_pwr_n4_cap2  the same with a capacitor (100 nF, 10 mΩ, 0.6 nH) between L2 and L3 2 mm away
  R_s4_rect_n4       (Block 3) the GND/GND reference: the same GND vias join both planes
  RG_ts6_sw_pwr      ThunderScope stack (F / In1 GND / In2 PWR / 0.93 mm core / In3 / In4 GND / B),
                     via F → In3 with the stub to B (the common ThunderScope channel via), four GND
                     vias at 1 mm joined to In1 and In4 that pass the PWR plane In2 through openings:
                     floating returns across In2
  RG_ts6_sw_gnd      the same with In2 a GND plane joined to the GND vias (the reference: the ladder
                     before return groups shorts both pairs)
  thru_RG_ts6p_L1/L4 2×thru fixtures of both feeds of the ts6 cases (the s4 cases use thru_R_s4_L1/L4)
Board vias (`board`): BV_ts_refinout2 (Block 2) and its In2-grounded diagnosis (Block 2), the M.2
pair BV_ts_m2_per0 on the quick mesh and an In2-grounded copy of it (written by `board --write`).
"""
import argparse, glob, json, math, os, subprocess, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
CASES = os.path.join(HERE, "return_cases")
RES = os.path.join(HERE, "results")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
import via_cases as vc  # noqa: E402
import board_resonances as br  # noqa: E402

C0 = 299792458.0
BOARD = br.BOARD          # 24 × 16 mm
VIA_A = br.VIA_A
# the ThunderScope stack with the supply plane In2 (planes In1, In2, In4)
vc.STACKUPS["ts6p"] = dict(vc.STACKUPS["ts6"], title="ThunderScope Rev5.3 6-layer with the In2 plane", planes=[1, 2, 4])
TS_DRILL_R, TS_PAD_R, TS_AP = 0.1016, 0.2032, 0.35   # ThunderScope via: drill Ø0.2032, pad Ø0.4064


def builder(stackup):
    hx = (BOARD[2] - BOARD[0]) / 2 + br.MARGIN
    hy = (BOARD[3] - BOARD[1]) / 2 + br.MARGIN
    b = vc.Builder(stackup, hx, hy, br.FMAX, br.DFINE, board=list(BOARD), air=br.AIR)
    b.case["mesh"]["dmax"] = br.DMAX
    return b


def finish(b, cid, title, params, thru, cavity):
    c = b.finish(cid, "RG", title, params)
    c.update(end_db=-35, max_steps=300000, nf=1000)
    # the field of the pair the via crosses first: top view in its middle at three frequencies
    f10 = C0 / (2 * (BOARD[2] - BOARD[0]) * 1e-3 * math.sqrt(cavity["er"]))
    c["monitors"] = [{"axis": "z", "pos": cavity["z_mid"], "freqs": [round(f10 * k / 1e6) * 1e6 for k in (1.0, 1.5, 1.8)], "comps": [2]}]
    c["meta"].update(board=list(BOARD), via=list(VIA_A), thru=thru, cavity={k: v for k, v in cavity.items() if k != "z_mid"})
    return c


def s4_case(cid, cap):
    b = builder("s4")
    b.pwr = {2}  # L3 is the supply plane
    x, y = VIA_A
    b.via(x, y, 0.15, 0.3, 0, 3, [0, 3])
    for p in b.st["planes"]:
        b.antipad(p, x, y, 0.4)
    for dx, dy in vc.ring(4, 1.0):
        gx, gy = round(x + dx, 4), round(y + dy, 4)
        b.via(gx, gy, 0.15, 0.3, 0, 3, [], net="GND")
        b.antipad(2, gx, gy, 0.35)  # the GND via passes the PWR plane through an opening
    b.feed_line(-1, 0, y, x, "1 L1")
    b.feed_line(+1, 3, y, x, "2 L4")
    what = "four GND vias at 1 mm joined to L2 only (openings in the PWR plane L3)"
    params = {"board_mm": "24 × 16", "returns": 4, "via_mm": list(VIA_A), "pwr_plane": "L3"}
    if cap:
        at = (x, y + 2.0)
        b.lumped(at[0], at[1], b.zc[2], b.zc[1], r=0.01, l=0.6e-9, c=100e-9, name="100 nF, ESR 10 mΩ, ESL 0.6 nH")
        what += ", a capacitor between L2 and L3 2 mm away"
        params["cap_mm"] = list(at)
    c = finish(b, cid, f"Finite board 24 × 16 mm, via L1 → L4 through the GND/PWR pair L2/L3, {what}", params,
               ["thru_R_s4_L1", "thru_R_s4_L4"], {"layers": ["L2", "L3"], "d_mm": 1.2, "er": 4.5, "tand": 0.02, "z_mid": 0.8})
    if cap:
        c["meta"]["caps"] = [{"at": list(at), "r": 0.01, "l": 0.6e-9, "c": 100e-9}]
    return c


def ts6_case(cid, in2):
    b = builder("ts6p")
    if in2 == "PWR":
        b.pwr = {2}
    x, y = VIA_A
    zc = b.zc
    b.via(x, y, TS_DRILL_R, TS_PAD_R, 0, 5, [0, 3])
    for p in b.st["planes"]:
        b.antipad(p, x, y, TS_AP)
    for dx, dy in vc.ring(4, 1.0):
        gx, gy = round(x + dx, 4), round(y + dy, 4)
        b.via(gx, gy, TS_DRILL_R, TS_PAD_R, 0, 5, [], net="GND")
        if in2 == "PWR":
            b.antipad(2, gx, gy, 0.3)
    b.feed_line(-1, 0, y, x, "1 F")
    b.feed_line(+1, 3, y, x, "2 In3")
    title = ("Finite board 24 × 16 mm, ThunderScope stack, via F → In3 (stub to B) through In1 GND / In2 {} / In4 GND, "
             "four GND vias at 1 mm joined to In1 and In4{}").format("PWR" if in2 == "PWR" else "GND", " (openings in In2)" if in2 == "PWR" else " and In2")
    zm = round((zc[2] + zc[3]) / 2, 6)
    c = finish(b, cid, title, {"board_mm": "24 × 16", "returns": 4, "via_mm": list(VIA_A), "in2": in2},
               ["thru_RG_ts6p_L1", "thru_RG_ts6p_L4"], {"layers": ["In1", "In2", "In4"], "er": 4.5, "tand": 0.02, "z_mid": zm})
    # the reference with the cell at metal halved (0.035 mm; run_refs.py, TESTS.md section 17): at 0.07 mm the
    # exit on In3 came out 1.8 ps slow
    c["meta"]["ref_refine"] = 2
    return c


def cases():
    out = [s4_case("RG_s4_pwr_n4", False), s4_case("RG_s4_pwr_n4_cap2", True), ts6_case("RG_ts6_sw_pwr", "PWR"), ts6_case("RG_ts6_sw_gnd", "GND")]
    for layer in (0, 3):
        t = vc.thru("ts6p", layer, br.FMAX, br.DFINE)
        t["name"] = f"thru_RG_ts6p_L{layer + 1}"
        t["mesh"]["dmax"] = br.DMAX
        t["nf"] = 1000
        t["monitors"] = []
        out.append(t)
    return out


def tool(a, name):
    exe = {"kisi": "kisi", "validate": "kisi-validate", "fdtd": "kisi-fdtd"}[name]
    return getattr(a, name, None) or os.path.join(os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target"), "release", exe)


SYNTH = ["RG_s4_pwr_n4", "RG_s4_pwr_n4_cap2", "R_s4_rect_n4", "RG_ts6_sw_pwr", "RG_ts6_sw_gnd"]


def case_path(n):
    p = os.path.join(CASES, n + ".json")
    return p if os.path.exists(p) else os.path.join(HERE, "resonance_cases", n + ".json")


def selected(a, with_thru=True):
    keys = [k for k in (a.only or "").split(",") if k]
    names = sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(CASES, "*.json")))
    names.sort(key=lambda n: (not n.startswith("thru"), n))
    names = [n for n in names if with_thru or not n.startswith("thru")]
    return [n for n in names if not keys or any(k in n for k in keys)]


def cmd_cases(a):
    os.makedirs(CASES, exist_ok=True)
    cs = cases()
    for c in cs:
        json.dump(c, open(os.path.join(CASES, c["name"] + ".json"), "w"), indent=1)
    print(f"{len(cs)} cases in {CASES}")


def cmd_mesh(a):
    for n in selected(a):
        m = json.loads(subprocess.run([tool(a, "fdtd"), os.path.join(CASES, n + ".json"), "--mesh-only"], capture_output=True, text=True, check=True).stdout)
        nx, ny, nz = len(m["x"]), len(m["y"]), len(m["z"])
        dmin = min(min(np.diff(m["x"])), min(np.diff(m["y"])), min(np.diff(m["z"])))
        print(f"{n}: {nx} × {ny} × {nz} = {nx * ny * nz / 1e6:.2f} M cells, smallest cell {dmin:.4f} mm")


def cmd_run(a):
    import run_refs
    ra = argparse.Namespace(exe=tool(a, "fdtd"), fine=0.0, openems=None)
    for n in selected(a):
        c = json.load(open(os.path.join(CASES, n + ".json")))
        k = len(c["ports"])
        if os.path.exists(os.path.join(REF, n, f"kisi.s{k}p")) and not a.force:
            print(f"{n}: done", flush=True)
            continue
        _, meta = run_refs.one(os.path.join(CASES, n + ".json"), REF, ra)
        r = meta["runs"]["kisi"]
        print(f"{n}: {r['cells'] / 1e6:.2f} M cells, steps {r['steps']}, {r['seconds']} s", flush=True)


# --- kisi's ladders on the synthetic cases ---------------------------------------------------
def cap_radius(a, n):
    """Peaceman radius of the cell of the lumped capacitor in the 3D mesh (as board_resonances.py)"""
    c = json.load(open(case_path(n)))
    if not c.get("lumped"):
        return None
    m = json.loads(subprocess.run([tool(a, "fdtd"), case_path(n), "--mesh-only"], capture_output=True, text=True, check=True).stdout)
    x, y = np.array(m["x"]), np.array(m["y"])
    el = c["lumped"][0]
    i, j = int(np.argmin(abs(x - el["x"]))), int(np.argmin(abs(y - el["y0"])))
    dx, dy = (x[i + 1] - x[i - 1]) / 2, (y[j + 1] - y[j - 1]) / 2
    return float(0.14 * math.hypot(dx, dy))


def cmd_model(a):
    out = {}
    keys = [k for k in (a.only or "").split(",") if k]
    for n in SYNTH:
        if keys and not any(k in n for k in keys):
            continue
        cp = case_path(n)
        res = {}
        for extra in ([], ["--returns", "--suffix", "ret"]):
            cmd = [tool(a, "validate"), "cavity-model", cp, os.path.join(REF, n)] + extra
            r = cap_radius(a, n) if "cap" in n else None
            if r:
                cmd += ["--cap-r", f"{r:.4f}"]
            res["ret" if extra else "old"] = json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout)["timing"]
        out[n] = res
        print(n, json.dumps({k: {t: round(v.get("ladder_ms", v.get("ms", 0))) for t, v in r.items()} for k, r in res.items()}), flush=True)
    os.makedirs(RES, exist_ok=True)
    p = os.path.join(RES, "board_returns_model_timing.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(out)
    json.dump(old, open(p, "w"), indent=1)


# --- comparison ----------------------------------------------------------------------------
LABELS = {"inf": "before return groups, infinite plates (kisi now)", "grid": "before return groups, cavities",
          "inf_ret": "return groups, infinite plates", "grid_ret": "return groups, cavities"}
STYLES = {"3D": dict(color="k", lw=1.6), LABELS["inf"]: dict(color="#888", lw=1.0, ls="--"), LABELS["grid"]: dict(color="#e59866", lw=0.9, ls="--"),
          LABELS["inf_ret"]: dict(color="#2471a3", lw=1.0), LABELS["grid_ret"]: dict(color="#c0392b", lw=1.1)}
BANDS = ((0.5e9, 6e9), (0.5e9, 12e9), (12e9, 20e9), (20e9, 25e9))


def thru_on(f, name):
    d = os.path.join(REF, name, "kisi.s2p")
    return br.thru_on(f, name) if os.path.exists(d) else None


def plot(f, ref, models, out, title, extra_ref=None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    import textwrap
    rows = [("|S21| (dB)", lambda S: S[:, 1, 0]), ("|S11| (dB)", lambda S: S[:, 0, 0])]
    fig, axs = plt.subplots(2, 1, figsize=(9, 6.0), sharex=True)
    for ax, (lab, get) in zip(axs, rows):
        r3 = sparam.db(get(ref))
        ax.plot(f / 1e9, r3, label="3D", **STYLES["3D"])
        if extra_ref is not None:
            ax.plot(extra_ref[0] / 1e9, sparam.db(get(extra_ref[1])), label=extra_ref[2], color="#7d3c98", lw=1.0, ls="-.")
        for k, S in models.items():
            ax.plot(f / 1e9, sparam.db(get(S)), label=k, **STYLES.get(k, {}))
        ax.set_ylabel(lab)
        ax.grid(alpha=0.3)
    axs[0].legend(fontsize=7.5, loc="lower left")
    axs[0].set_title("\n".join(textwrap.wrap(title, 110)), fontsize=9)
    axs[-1].set_xlabel("frequency (GHz)")
    axs[-1].set_xlim(0, 25)
    fig.tight_layout()
    fig.savefig(out, dpi=110)
    plt.close(fig)


def deembedded_case(n):
    c = json.load(open(case_path(n)))
    d = os.path.join(REF, n)
    p = os.path.join(d, "kisi.s2p")
    if not os.path.exists(p):
        return None
    f, S = sparam.read_touchstone(p)
    T = [br.thru_on(f, t) for t in c["meta"]["thru"]]
    D, zc = sparam.deembed_matched(f, S, T[0], T[1], 50.0)
    sparam.write_touchstone(os.path.join(d, "deembedded.s2p"), f, D, [f"{n}: 3D, de-embedded (matched 2x-thru)"])
    return c, f, D, zc


def models_of(n):
    d = os.path.join(REF, n)
    out = {}
    for tag in LABELS:
        p = os.path.join(d, f"ladder_{tag}.s2p")
        if os.path.exists(p):
            out[tag] = sparam.read_touchstone(p)[1]
    return out


def cmd_compare(a):
    res = {}
    pdir = os.path.join(RES, "board_returns")
    os.makedirs(pdir, exist_ok=True)
    keys = [k for k in (a.only or "").split(",") if k]
    got = {}
    for n in SYNTH:
        if keys and not any(k in n for k in keys):
            continue
        x = deembedded_case(n)
        if x is None:
            continue
        c, f, D, zc = x
        got[n] = (f, D)
        mods = models_of(n)
        r = {"title": c["meta"]["title"], "cells": json.load(open(os.path.join(REF, n, "meta.json")))["runs"]["kisi"], "line_z_ohm": [round(z, 1) for z in zc]}
        dref = br.dips(f, D[:, 1, 0])
        r["dips_3d"] = [[round(x / 1e9, 3), round(y, 2)] for x, y in dref]
        for tag, L in mods.items():
            m = {"dips": br.match_dips([x for x in dref if x[0] < 12e9], br.dips(f, L[:, 1, 0]))}
            for lo, hi in BANDS:
                m[f"dS21_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = br.band_err(f, D[:, 1, 0], L[:, 1, 0], lo, hi)
                m[f"dS11_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = br.band_err(f, D[:, 0, 0], L[:, 0, 0], lo, hi)
            r[tag] = m
        plot(f, D, {LABELS[t]: L for t, L in mods.items()}, os.path.join(pdir, n + ".png"), f"{n}: {c['meta']['title']}")
        res[n] = r
        print(n, json.dumps({t: r[t]["dS21_max_rms_db_0.5_12"] for t in LABELS if t in r}), flush=True)
    # the effect of the supply plane: 3D (PWR − GND) against the model (with − before return groups)
    for pw, gd in (("RG_ts6_sw_pwr", "RG_ts6_sw_gnd"), ("RG_s4_pwr_n4", "R_s4_rect_n4")):
        if pw in got and gd in got:
            f, Dp = got[pw]
            _, Dg = got[gd]
            mp, mg = models_of(pw), models_of(gd)
            e = {}
            d3 = sparam.db(Dp[:, 1, 0]) - sparam.db(Dg[:, 1, 0])
            for tag in ("inf", "grid"):
                if tag + "_ret" in mp and tag in mg:
                    dm = sparam.db(mp[tag + "_ret"][:, 1, 0]) - sparam.db(mg[tag][:, 1, 0])
                    for lo, hi in BANDS:
                        sel = (f >= lo) & (f <= hi)
                        e[f"{tag}_{lo / 1e9:g}_{hi / 1e9:g}"] = {"3d_max": round(float(np.max(np.abs(d3[sel]))), 2), "model_max": round(float(np.max(np.abs(dm[sel]))), 2),
                                                                  "diff_max": round(float(np.max(np.abs(d3[sel] - dm[sel]))), 2)}
            res.setdefault(pw, {})["supply_effect_vs_" + gd] = e
            for fx in (2e9, 6e9, 12e9, 16e9, 20e9):
                k = int(np.argmin(abs(f - fx)))
                res[pw].setdefault("supply_effect_points", {})[f"{fx / 1e9:g}"] = [round(float(d3[k]), 3)] + [round(float(sparam.db(mp[t + "_ret"][k, 1, 0]) - sparam.db(mg[t][k, 1, 0])), 3) for t in ("inf", "grid") if t + "_ret" in mp and t in mg]
            plot(f, Dp, {LABELS[t]: L for t, L in mp.items()}, os.path.join(pdir, pw + "_vs_gnd.png"),
                 f"{pw} against {gd} (dash-dot: 3D of {gd})", extra_ref=(f, Dg, f"3D, {gd}"))
    p = os.path.join(RES, "board_returns.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(res)
    json.dump(old, open(p, "w"), indent=1)


def cmd_summary(a):
    r = json.load(open(os.path.join(RES, "board_returns.json")))
    for n, d in sorted(r.items()):
        print(f"\n{n}: {d.get('title', '')}")
        for tag in LABELS:
            if tag not in d:
                continue
            m = d[tag]
            line = "  ".join(f"{k.replace('dS21_max_rms_db_', '')}: {m[k][0]:.2f}/{m[k][1]:.2f}" for k in m if k.startswith("dS21_max_rms"))
            dd = m.get("dips")
            if dd:
                ok = [x for x in dd if x[1] is not None]
                line += f"  | dips {len(ok)}/{len(dd)}"
                if ok:
                    line += f", |Δf| {np.mean([abs(x[2]) for x in ok]):.1f} %, |Δdepth| {np.mean([abs(x[4] - x[3]) for x in ok]):.2f} dB"
            print(f"   {tag:9s} {line}")
        for k in d:
            if k.startswith("supply_effect"):
                print(f"   {k}: {json.dumps(d[k])}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["cases", "mesh", "run", "model", "compare", "board", "summary"])
    ap.add_argument("--only", default="")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--fdtd")
    ap.add_argument("--validate")
    ap.add_argument("--kisi")
    a = ap.parse_args()
    {"cases": cmd_cases, "mesh": cmd_mesh, "run": cmd_run, "model": cmd_model, "compare": cmd_compare, "board": lambda a: cmd_board(a), "summary": cmd_summary}[a.cmd](a)


# --- ThunderScope board vias: In2 as it is (a supply plane) against In2 grounded -----------------
import board_vias as bv  # noqa: E402

BOARD_VIAS = {
    # case, its folders (cases, results), the In2-grounded copy and its folders, net, pair
    "BV_ts_refinout2": dict(cases=bv.CASES, ref=REF, gnd="BV_ts_refinout2_in2gnd", gcases=os.path.join(bv.CASES, "diag"), gref=os.path.join(REF, "diag"),
                            net="/TS-PCIe Components/REFINOUT2", title="ThunderScope REFINOUT2, through via F.Cu → B.Cu across In2 (+VUSB)"),
    "BV_ts_m2_per0_q050f25s_e35": dict(cases=os.path.join(bv.CASES, "quick"), ref=os.path.join(REF, "quick"), gnd="BV_ts_m2_per0_q050f25s_e35_in2gnd", gcases=CASES, gref=REF,
                                       net="/TS-USB4 Components/M2_PER0_P", title="ThunderScope M.2 PCIe pair via F.Cu → In3.Cu across In2 (+VUSB / +1V2_MGT), quick mesh"),
}


def write_m2_in2gnd():
    """the M.2 quick case with In2 joined to the GND vias: a GND disk on In2 over each GND via's
    opening (radius: the distance to the nearest In2 copper + 0.05 mm), except where the disk would
    come within 0.1 mm of a via of another net; the signal vias' openings stay as they are"""
    k = "BV_ts_m2_per0_q050f25s_e35"
    c = json.load(open(os.path.join(BOARD_VIAS[k]["cases"], k + ".json")))
    z2 = 1.20165
    rings = [r for s in c["sheets"] if abs(s["z"] - z2) < 1e-6 for r in s.get("polys", [])]

    def dist(x, y):
        d = 1e9
        for r in rings:
            for (p, q) in zip(r, r[1:] + r[:1]):
                ex, ey = q[0] - p[0], q[1] - p[1]
                l2 = ex * ex + ey * ey
                t = 0.0 if l2 < 1e-18 else max(0.0, min(1.0, ((x - p[0]) * ex + (y - p[1]) * ey) / l2))
                d = min(d, math.hypot(x - p[0] - t * ex, y - p[1] - t * ey))
        return d
    n = 0
    others = [w for w in c["vias"] if w.get("net") != "GND"]
    for v in c["vias"]:
        if v.get("net") != "GND":
            continue
        rr = dist(v["x"], v["y"]) + 0.05
        # a GND via that shares an opening with a via of another net (the pair's own GND vias) cannot
        # be bridged to In2 without touching it: left as it is
        if any(math.hypot(w["x"] - v["x"], w["y"] - v["y"]) - w["r"] - 0.1 < rr for w in others):
            continue
        disk = [[round(v["x"] + rr * math.cos(2 * math.pi * i / 32), 6), round(v["y"] + rr * math.sin(2 * math.pi * i / 32), 6)] for i in range(32)]
        c["sheets"].append({"z": z2, "rect": [v["x"] - rr, v["y"] - rr, v["x"] + rr, v["y"] + rr], "holes": [], "net": "GND", "polys": [disk]})
        n += 1
    c["name"] = BOARD_VIAS[k]["gnd"]
    c["meta"]["title"] = c["meta"]["title"] + " – diagnosis: In2 joined to every GND via"
    c["meta"]["thru_dir"] = "quick"
    c["meta"]["ref_refine"] = 2  # the reference with the cell at metal halved (TESTS.md section 17)
    os.makedirs(CASES, exist_ok=True)
    json.dump(c, open(os.path.join(CASES, c["name"] + ".json"), "w"), indent=1)
    print(f"{c['name']}: {n} GND vias joined to In2")


def board_ladders(a, k, v, c):
    """kisi's ladder for the exported via before and with return groups: the geometry kisi's
    channel prints (KISI_DEBUG_VIA) for this via – found by its return vias in the run before return
    groups, the same build in the run with them – with the 3D loss model, evaluated by
    `kisi-validate via-geom` (a pair: the odd mode, 50 Ω per line = Sdd at 100 Ω)"""
    pcb = os.path.join(ROOT, "examples", "Thunderscope_Rev5.3.kicad_pcb")
    cfg = os.path.join(ROOT, "web", "samples", "ts.kisi.json")
    out = {}
    runs = {}
    for tag, extra in (("old", {}), ("ret", {"KISI_VIA_RETURNS": "1"})):
        env = dict(os.environ, KISI_DEBUG_VIA="1", **extra)
        r = subprocess.run([tool(a, "kisi"), "net", pcb, v["net"], "--config", cfg], capture_output=True, text=True, env=env)
        runs[tag] = [json.loads(l) for l in r.stderr.splitlines() if l.startswith("{")]
    want = c["meta"]["kisi_returns"]
    pair = c["meta"]["params"]["partner"] is not None

    def match(g):
        # every return the case lists (cases exported before Block 6 list the nearest 8 vias, the
        # ladder now also takes plated pads and up to 12)
        rr = [[q["dx"], q["dy"]] for q in g["returns"]]
        return all(any(abs(p[0] - q[0]) < 2e-3 and abs(p[1] - q[1]) < 2e-3 for p in rr) for q in want) and ((g["partner"] is not None) == pair)
    idx = [i for i, g in enumerate(runs["old"]) if match(g)]
    if not idx:
        raise SystemExit(f"{k}: no ladder geometry matches")
    d = os.path.join(REF, "board_returns", k)
    os.makedirs(d, exist_ok=True)
    for tag in ("old", "ret"):
        g = dict(runs[tag][idx[0]], f_loss=c["fmax"] / 2, sigma=None)
        # the via between lines to its centre (without the trace the channel left out, board_vias.py)
        g["feeds"] = [dict(fd, dropped=0.0) for fd in g["feeds"]]
        gp = os.path.join(d, f"ladder3d_{tag}.json")
        json.dump(g, open(gp, "w"))
        subprocess.run([tool(a, "validate"), "via-geom", gp, "--s2p", os.path.join(d, f"ladder3d_{tag}.s2p")], capture_output=True, text=True, check=True)
        out[tag] = sparam.read_touchstone(os.path.join(d, f"ladder3d_{tag}.s2p"))
    return out


def cmd_board(a):
    import run_refs
    if a.write:
        write_m2_in2gnd()
    res = {}
    pdir = os.path.join(RES, "board_returns")
    os.makedirs(pdir, exist_ok=True)
    keys = [k for k in (a.only or "").split(",") if k]
    for k, v in BOARD_VIAS.items():
        if keys and not any(x in k for x in keys):
            continue
        gp = os.path.join(v["gref"], v["gnd"])
        gcase = os.path.join(v["gcases"], v["gnd"] + ".json")
        if not os.path.exists(gcase):
            continue
        n = len(json.load(open(gcase))["ports"])
        if not os.path.exists(os.path.join(gp, f"kisi.s{n}p")):
            ra = argparse.Namespace(exe=tool(a, "fdtd"), fine=0.0, openems=None)
            _, meta = run_refs.one(gcase, v["gref"], ra)
            print(f"{v['gnd']}: {meta['runs']['kisi']['cells'] / 1e6:.2f} M cells, {meta['runs']['kisi']['seconds']} s", flush=True)
        f, Df, _, _, c, _ = bv.deembedded(k, v["ref"], v["cases"])
        fg, Dg, *_ = bv.deembedded(v["gnd"], v["gref"], v["gcases"])
        lad = board_ladders(a, k, v, c)
        fmax = min(25e9, f[-1], fg[-1])
        fq = np.arange(0.5e9, fmax + 1, 0.05e9)
        at = lambda F, S: np.stack([np.stack([np.interp(fq, F, S[:, i, j].real) + 1j * np.interp(fq, F, S[:, i, j].imag) for j in range(2)], -1) for i in range(2)], -2)
        D3 = {"3D, In2 as it is": at(f, Df), "3D, In2 grounded": at(fg, Dg)}
        L = {"before return groups": at(*lad["old"]), "return groups": at(*lad["ret"])}
        r = {"title": v["title"]}
        for dn, D in D3.items():
            for ln, M in L.items():
                e = {}
                for lo, hi in ((0.5e9, 12e9), (12e9, 20e9), (20e9, 25e9)):
                    sel = (fq >= lo) & (fq <= hi)
                    if sel.any():
                        dd = np.abs(sparam.db(D[sel, 1, 0]) - sparam.db(M[sel, 1, 0]))
                        e[f"{lo / 1e9:g}_{hi / 1e9:g}"] = [round(float(dd.max()), 3), round(float(np.sqrt(np.mean(dd ** 2))), 3)]
                r[f"{dn} vs {ln}"] = e
        pts = {}
        for fx in (4e9, 8e9, 12e9, 16e9, 20e9, 24e9):
            if fx > fmax:
                continue
            j = int(np.argmin(abs(fq - fx)))
            pts[f"{fx / 1e9:g}"] = {name: round(float(sparam.db(S[j, 1, 0])), 3) for name, S in {**D3, **L}.items()}
        r["S21_db"] = pts
        res[k] = r
        # plot
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
        fig, axs = plt.subplots(2, 1, figsize=(9, 6.0), sharex=True)
        sty = {"3D, In2 as it is": dict(color="k", lw=1.6), "3D, In2 grounded": dict(color="#7d3c98", lw=1.2, ls="-."),
               "before return groups": dict(color="#888", lw=1.0, ls="--"), "return groups": dict(color="#2471a3", lw=1.1)}
        lab = "|Sdd21|" if len(c["ports"]) == 4 else "|S21|"
        for name, S in {**D3, **L}.items():
            axs[0].plot(fq / 1e9, sparam.db(S[:, 1, 0]), label=name + ("" if name.startswith("3D") else " (ladder, 3D loss model)"), **sty[name])
            axs[1].plot(fq / 1e9, sparam.db(S[:, 0, 0]), **sty[name])
        axs[0].set_ylabel(lab + " (dB)")
        axs[1].set_ylabel(lab.replace("21", "11") + " (dB)")
        for ax in axs:
            ax.grid(alpha=0.3)
        axs[0].legend(fontsize=8, loc="lower left")
        axs[0].set_title(f"{k}: {v['title']}", fontsize=9)
        axs[1].set_xlabel("frequency (GHz)")
        axs[1].set_xlim(0, fmax / 1e9)
        fig.tight_layout()
        fig.savefig(os.path.join(pdir, k + ".png"), dpi=110)
        plt.close(fig)
        print(k, json.dumps(pts), flush=True)
    p = os.path.join(RES, "board_returns.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(res)
    json.dump(old, open(p, "w"), indent=1)


if __name__ == "__main__":
    main()
