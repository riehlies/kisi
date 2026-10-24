"""3D check of critical vias taken from the example boards (validation/fdtd/BOARD_VIAS.md).

  python3 board_vias.py export   [--only KEY]   board → kisi-fdtd cases (kisi-validate via-case)
  python3 board_vias.py run      [--only KEY]   kisi-fdtd on the cases and their 2×thru fixtures
  python3 board_vias.py ladder   [--only KEY]   kisi's ladder for the same via (KISI_DEBUG_VIA)
  python3 board_vias.py compare  [--only KEY]   de-embed, compare with the ladder, plots, results
  python3 board_vias.py deembed-check            the de-embedding on a known network (lines of Zc ≠ z0)
  python3 board_vias.py block1                   Block 1 references de-embedded both ways
  python3 board_vias.py variants [--only KEY]    ladder with the signal-layer pours as planes (before Block 5),
                                                without them, without the junction model
  python3 board_vias.py diag                     REFINOUT2 with In2 grounded (board_cases/diag)
  python3 board_vias.py fixes    [--only KEY]   Block 5: the ladder with the pours on the signal layers as
                                                planes and as coplanar returns, all board vias → results/board_vias_fixes.json
  python3 board_vias.py quick    [--only KEY]   coarser meshes against the base mesh (resource estimate)
  python3 board_vias.py speed    [--wasm F] [--threads N]   setup time, step rate and memory,
                                                native (1 and N threads) and WebAssembly
  python3 board_vias.py plot CASE.json OUT.png  top views of a case (one panel per copper layer)

Cases go to validation/fdtd/board_cases/, results to validation/via_ref/<case>/ (kisi.sNp,
meta.json, fields.json, ladder.s2p, deembedded.s2p), the summary to results/board_vias.json and
the plots to results/board_vias/*.png.

Options: --kisi, --validate, --fdtd (binaries), --boards (folder with the example boards).
"""
import argparse, glob, json, math, os, subprocess, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
CASES = os.path.join(HERE, "board_cases")
RES = os.path.join(HERE, "results")
sys.path.insert(0, HERE)
import sparam  # noqa: E402

# the vias: case name → board, net, via and exporter options
VIAS = {
    "BV_ts_m2_per0": {"board": "Thunderscope_Rev5.3", "config": "ts", "net": "/TS-USB4 Components/M2_PER0_P", "via": 0, "opts": ["--keep", "1.2"],
                      "title": "ThunderScope M.2 PCIe RX lane 0 (PER0_P/N), pair via F.Cu → In3.Cu, pitch 0.53 mm"},
    "BV_ts_refinout2": {"board": "Thunderscope_Rev5.3", "config": "ts", "net": "/TS-PCIe Components/REFINOUT2", "via": 0, "opts": ["--keep", "1.2"],
                        "title": "ThunderScope 50 Ω REFINOUT2, through via F.Cu → B.Cu (thick core)"},
    "BV_vna_rf_hblb1": {"board": "VNA", "config": "vna", "net": "/Attenuator Amp Switches/RF_HBLB_PORT1", "via": 0, "opts": ["--keep", "1.2"],
                        "title": "LibreVNA RF_HBLB_PORT1, via F.Cu → In2.Cu with a 0.87 mm stub to B.Cu (CPWG feeds)"},
}


def tool(a, name):
    """binary: --kisi / --validate / --fdtd, else target/release in the repository"""
    exe = {"kisi": "kisi", "validate": "kisi-validate", "fdtd": "kisi-fdtd"}[name]
    return getattr(a, name) or os.path.join(ROOT, "target", "release", exe)


def selected(a):
    keys = [k for k in a.only.split(",") if k]
    return [k for k in VIAS if not keys or any(x in k for x in keys)]


def board_files(a, v):
    pcb = os.path.join(a.boards, v["board"] + ".kicad_pcb")
    cfg = os.path.join(ROOT, "web", "samples", v["config"] + ".kisi.json")
    return pcb, cfg


# --- export, run --------------------------------------------------------------------------
def export(a):
    os.makedirs(CASES, exist_ok=True)
    for k in selected(a):
        v = VIAS[k]
        pcb, cfg = board_files(a, v)
        cmd = [tool(a, "validate"), "via-case", pcb, "--net", v["net"], "--config", cfg, "--via", str(v["via"]), "--name", k, "--out", CASES] + v["opts"]
        r = subprocess.run(cmd, capture_output=True, text=True)
        print(r.stderr.strip().splitlines()[-1] if r.stderr.strip() else "", flush=True)
        if r.returncode:
            raise SystemExit(r.stderr)
        c = json.load(open(os.path.join(CASES, k + ".json")))
        c["meta"]["title"] = v["title"]
        json.dump(c, open(os.path.join(CASES, k + ".json"), "w"))
        os.makedirs(os.path.join(RES, "board_vias"), exist_ok=True)
        plot_case(c, os.path.join(RES, "board_vias", k + "_case.png"))
        plot_case(c, os.path.join(RES, "board_vias", k + "_case_zoom.png"), zoom=1.6)


def run(a):
    import run_refs
    ra = argparse.Namespace(exe=tool(a, "fdtd"), fine=0.0, openems=None)
    for k in selected(a):
        c = json.load(open(os.path.join(CASES, k + ".json")))
        for name in c["meta"]["thru"] + [k]:
            n = len(json.load(open(os.path.join(CASES, name + ".json")))["ports"])
            if os.path.exists(os.path.join(REF, name, f"kisi.s{n}p")) and not a.force:
                print(f"{name}: done", flush=True)
                continue
            name_, meta = run_refs.one(os.path.join(CASES, name + ".json"), REF, ra)
            r = meta["runs"]["kisi"]
            print(f"{name}: {r['cells'] / 1e6:.2f} M cells, steps {r['steps']}, {r['seconds']} s", flush=True)


# --- kisi's ladder for the same via ---------------------------------------------------------
def ladder(a):
    """kisi's channel builds the ladder of every via of the net (KISI_DEBUG_VIA prints each
    geometry); the one whose return vias match the exported via is evaluated with
    `kisi-validate via-geom` → via_ref/<case>/ladder.s2p (odd mode for a pair: 50 Ω per line =
    Sdd at 100 Ω)"""
    for k in selected(a):
        v = VIAS[k]
        c = json.load(open(os.path.join(CASES, k + ".json")))
        pcb, cfg = board_files(a, v)
        env = dict(os.environ, KISI_DEBUG_VIA="1")
        r = subprocess.run([tool(a, "kisi"), "net", pcb, v["net"], "--config", cfg], capture_output=True, text=True, env=env)
        geoms = [json.loads(l) for l in r.stderr.splitlines() if l.startswith("{")]
        want = c["meta"]["kisi_returns"]
        pair = c["meta"]["params"]["partner"] is not None

        def match(g):
            # every return the case lists (cases exported before Block 6 list the nearest 8 vias,
            # the ladder now also takes plated pads and up to 12)
            rr = [[q["dx"], q["dy"]] for q in g["returns"]]
            return all(any(abs(p[0] - q[0]) < 2e-3 and abs(p[1] - q[1]) < 2e-3 for p in rr) for q in want) and ((g["partner"] is not None) == pair)
        hits = [g for g in geoms if match(g)]
        if not hits:
            raise SystemExit(f"{k}: no ladder geometry of kisi matches the via ({len(geoms)} printed)")
        g = hits[0]  # a pair: the first is the differential channel's (odd-mode feeds)
        # the via between lines that run to its centre, as the de-embedded 3D result: without the trace
        # the channel left out for the ladder (`dropped`, which the ladder puts back in the channel;
        # Block 2 compared with it in: LibreVNA 0.15 / 0.34 mm, M.2 0.03 mm of extra line)
        for fd in g["feeds"]:
            fd["dropped"] = 0.0
        d = os.path.join(REF, k)
        os.makedirs(d, exist_ok=True)
        json.dump(g, open(os.path.join(d, "ladder_geom.json"), "w"))
        out = subprocess.run([tool(a, "validate"), "via-geom", os.path.join(d, "ladder_geom.json"), "--s2p", os.path.join(d, "ladder.s2p")], capture_output=True, text=True, check=True).stdout
        # the same ladder with the 3D loss model (dielectric conductivity fixed at fmax/2, lossless
        # metal) for the comparison with kisi-fdtd, as validation/fdtd/rust_models.py does
        g3 = dict(g, f_loss=c["fmax"] / 2, sigma=None)
        json.dump(g3, open(os.path.join(d, "ladder3d_geom.json"), "w"))
        subprocess.run([tool(a, "validate"), "via-geom", os.path.join(d, "ladder3d_geom.json"), "--s2p", os.path.join(d, "ladder_3dloss.s2p")], capture_output=True, text=True, check=True)
        print(f"{k}: {len(hits)} of {len(geoms)} ladder geometries match; feeds Zc {[round(f['zc'], 1) for f in g['feeds']]}\n{out}", flush=True)


# --- comparison ----------------------------------------------------------------------------
def _interp_db(f, x, f0):
    return float(np.interp(f0, f, sparam.db(x)))


def deembedded(k, ref=REF, cases=CASES):
    """3D result of the via alone: 2-port (single via) or the differential 2-port Sdd (pair),
    de-embedded with the 2×thru of each side, the outer ports referred to each line's own
    impedance first (sparam.deembed_matched); also the raw mixed-mode blocks of a pair and, as
    `extra`, the line impedances and the plain z0-referred NZC result for comparison"""
    c = json.load(open(os.path.join(cases, k + ".json")))
    n = len(c["ports"])
    f, S = sparam.read_touchstone(os.path.join(ref, k, f"kisi.s{n}p"))
    th = []
    tref = os.path.join(REF, c["meta"]["thru_dir"]) if "thru_dir" in c["meta"] else ref
    for name in c["meta"]["thru"]:
        ft, T = sparam.read_touchstone(os.path.join(tref, name, f"kisi.s{n}p"))
        th.append(sparam.mixed_mode(T)["dd"] if n == 4 else T)
    raw = sparam.mixed_mode(S) if n == 4 else {"dd": S}
    z0 = 100.0 if n == 4 else 50.0
    FL, tauL = sparam.fixture_2xthru(f, th[0])
    FR, tauR = sparam.fixture_2xthru(f, th[1])
    D, zc = sparam.deembed_matched(f, raw["dd"], th[0], th[1], z0)
    extra = {"zc": zc, "D_nzc_z0": sparam.deembed2(raw["dd"], FL, FR)}
    return f, D, raw, (tauL, tauR), c, extra


def renorm(S, z_old, z_new):
    """S of a 2-port at reference z_old → reference z_new"""
    Z = sparam.s2z(S, z_old)
    I = np.eye(2)
    return (Z - z_new * I) @ np.linalg.inv(Z + z_new * I)


def metrics(f, D, L, z0):
    """3D (de-embedded) against the ladder: transmission, reflection, phase, TDR, dip"""
    m = {}
    for lim in (20e9, 25e9):
        sel = f <= lim + 1
        m[f"dS21_max_db_{int(lim / 1e9)}"] = round(float(np.max(np.abs(sparam.db(D[sel, 1, 0]) - sparam.db(L[sel, 1, 0])))), 3)
        m[f"dphase_max_deg_{int(lim / 1e9)}"] = round(float(np.max(np.abs(np.degrees(np.angle(D[sel, 1, 0] * np.conj(L[sel, 1, 0])))))), 1)
    for fx in (4e9, 8e9, 12e9, 16e9, 20e9, 25e9):
        g = int(fx / 1e9)
        m[f"S21_3d_{g}"] = round(_interp_db(f, D[:, 1, 0], fx), 3)
        m[f"S21_ladder_{g}"] = round(_interp_db(f, L[:, 1, 0], fx), 3)
        m[f"S11_3d_{g}"] = round(_interp_db(f, D[:, 0, 0], fx), 2)
        m[f"S11_ladder_{g}"] = round(_interp_db(f, L[:, 0, 0], fx), 2)
    for tag, X in (("3d", D), ("ladder", L)):
        # the via region only (reference plane at the via; later times hold the DC conductance of
        # the 3D loss model and the band edge)
        t, z = sparam.tdr(f, X[:, 0, 0], rise=25e-12, z0=z0, tmax=40e-12)
        m[f"tdr_min_{tag}"], m[f"tdr_max_{tag}"] = round(float(z.min()), 1), round(float(z.max()), 1)
        # least |S21| between 8 and 25 GHz (a dip or a notch) and the delay at 5–15 GHz
        sel = (f >= 8e9) & (f <= 25e9)
        i = int(np.argmin(sparam.db(X[sel, 1, 0])))
        m[f"S21_min_{tag}_ghz"] = round(float(f[sel][i] / 1e9), 2)
        m[f"S21_min_{tag}_db"] = round(float(sparam.db(X[sel, 1, 0])[i]), 2)
        sel = (f >= 25e9) & (f <= 40e9)
        if sel.sum() > 1:
            i = int(np.argmin(sparam.db(X[sel, 1, 0])))
            m[f"S21_min_25_40_{tag}_ghz"], m[f"S21_min_25_40_{tag}_db"] = round(float(f[sel][i] / 1e9), 2), round(float(sparam.db(X[sel, 1, 0])[i]), 2)
        sel = (f >= 5e9) & (f <= 15e9)
        ph = np.unwrap(np.angle(X[:, 1, 0]))
        m[f"delay_5_15ghz_{tag}_ps"] = round(float(-np.polyfit(2 * np.pi * f[sel], ph[sel], 1)[0] * 1e12), 2)
    # lumped totals at 3–8 GHz: shunt C = Im(ΣY)/ω, series L = Im(Z11 + Z22 − Z12 − Z21)/ω
    sel = (f >= 3e9) & (f <= 8e9)
    w = 2 * np.pi * f[sel]
    for tag, X in (("3d", D), ("ladder", L)):
        Y = np.linalg.inv(sparam.s2z(X[sel], z0))
        Z = sparam.s2z(X[sel], z0)
        m[f"C_total_fF_{tag}"] = round(float(np.mean(np.imag(Y.sum(axis=(1, 2))) / w)) * 1e15, 1)
        m[f"L_total_pH_{tag}"] = round(float(np.mean(np.imag(Z[:, 0, 0] + Z[:, 1, 1] - Z[:, 0, 1] - Z[:, 1, 0]) / w)) * 1e12, 1)
    m["tdr_min_err_pct"] = round((m["tdr_min_ladder"] / m["tdr_min_3d"] - 1) * 100, 1)
    m["tdr_max_err_pct"] = round((m["tdr_max_ladder"] / m["tdr_max_3d"] - 1) * 100, 1)
    return m


def plot_compare(k, f, D, L, raw, z0, out, title, Lk=None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    pair = "cd" in raw
    lab = "Sdd" if pair else "S"
    fg = f / 1e9
    fig, axs = plt.subplots(2, 2, figsize=(11, 7.5))
    ax = axs[0][0]
    ax.plot(fg, sparam.db(D[:, 1, 0]), "C0", label="3D (kisi-fdtd), de-embedded")
    ax.plot(fg, sparam.db(L[:, 1, 0]), "C1--", label="kisi ladder, 3D loss model")
    if Lk is not None:
        ax.plot(fg, sparam.db(Lk[:, 1, 0]), "C1:", lw=1, label="kisi ladder as in kisi (tanδ const., Cu)")
    ax.set_ylabel(f"|{lab}21| (dB)")
    lo = min(-3.0, float(np.min(sparam.db(D[fg <= 30, 1, 0]))) - 0.5, float(np.min(sparam.db(L[fg <= 30, 1, 0]))) - 0.5)
    ax.set_ylim(max(lo, -30), 0.3)
    ax.legend(fontsize=8)
    ax = axs[0][1]
    ax.plot(fg, sparam.db(D[:, 0, 0]), "C0", label=f"{lab}11 3D")
    ax.plot(fg, sparam.db(L[:, 0, 0]), "C1--", label=f"{lab}11 ladder")
    ax.plot(fg, sparam.db(D[:, 1, 1]), "C2", lw=0.8, label=f"{lab}22 3D")
    ax.plot(fg, sparam.db(L[:, 1, 1]), "C3--", lw=0.8, label=f"{lab}22 ladder")
    ax.set_ylabel("reflection (dB)")
    ax.set_ylim(-50, 0)
    ax.legend(fontsize=8)
    ax = axs[1][0]
    dph = np.degrees(np.angle(D[:, 1, 0] * np.conj(L[:, 1, 0])))
    ax.plot(fg, dph, "C0", label=f"∠{lab}21: 3D − ladder")
    if pair:
        ax2 = ax.twinx()
        ax2.plot(fg, sparam.db(raw["cd"][:, 1, 0]), "C4", lw=0.8, label="Scd21 3D (raw, with the feeds)")
        ax2.plot(fg, sparam.db(raw["cd"][:, 0, 0]), "C5", lw=0.8, label="Scd11 3D (raw)")
        ax2.set_ylabel("mode conversion (dB)")
        ax2.set_ylim(-60, 0)
        ax2.legend(fontsize=7, loc="lower right")
    ax.set_ylabel("phase difference (°)")
    ax.set_ylim(-60, 60)
    ax.legend(fontsize=8, loc="upper left")
    for a_ in (axs[0][0], axs[0][1], axs[1][0]):
        a_.set_xlabel("f (GHz)")
        a_.set_xlim(0, 40)
        a_.grid(alpha=0.3)
    ax = axs[1][1]
    for X, st, nm in ((D, "C0", "3D"), (L, "C1--", "ladder")):
        t, z = sparam.tdr(f, X[:, 0, 0], rise=25e-12, z0=z0, tmax=60e-12)
        ax.plot(t * 1e12, z, st, label=f"{nm}, port 1")
        t, z = sparam.tdr(f, X[:, 1, 1], rise=25e-12, z0=z0, tmax=60e-12)
        ax.plot(t * 1e12, z, st, lw=0.8, alpha=0.6, label=f"{nm}, port 2")
    ax.set_xlabel("t (ps), TDR 25 ps, reference plane at the via")
    ax.set_ylabel("Zdiff (Ω)" if pair else "Z (Ω)")
    ax.grid(alpha=0.3)
    ax.legend(fontsize=8)
    fig.suptitle(title, fontsize=10)
    fig.tight_layout()
    fig.savefig(out, dpi=110)
    plt.close(fig)


def compare(a):
    os.makedirs(os.path.join(RES, "board_vias"), exist_ok=True)
    summ = {}
    rp = os.path.join(RES, "board_vias.json")
    if os.path.exists(rp):
        summ = json.load(open(rp)).get("cases", {})
    for k in selected(a):
        if not os.path.exists(os.path.join(REF, k, "ladder.s2p")):
            print(f"{k}: no ladder.s2p (run `ladder` first)")
            continue
        try:
            f, D, raw, taus, c, extra = deembedded(k)
        except FileNotFoundError as e:
            print(f"{k}: missing 3D result ({e.filename})")
            continue
        fl, L = sparam.read_touchstone(os.path.join(REF, k, "ladder_3dloss.s2p"))
        L = L[: len(f)]
        _, Lk = sparam.read_touchstone(os.path.join(REF, k, "ladder.s2p"))
        Lk = Lk[: len(f)]
        pair = "cd" in raw
        z0 = 100.0 if pair else 50.0
        zc = extra["zc"]
        sparam.write_touchstone(os.path.join(REF, k, "deembedded.s2p"), f, D,
                                [f"{k}: kisi-fdtd, " + ("Sdd (100 Ω)" if pair else "2-port") + ", de-embedded with the 2×thru of each side (NZC with the outer ports",
                                 f"referred to the line impedances {zc[0]:.1f} / {zc[1]:.1f} Ω, sparam.deembed_matched), the via between ideal {z0:g} Ω lines"], z0)
        m = metrics(f, D, L, z0)
        m["fixture_delay_ps"] = [round(t * 1e12, 2) for t in taus]
        m["line_z_3d_ohm"] = [round(z, 1) for z in zc]
        # the plain z0-referred NZC (the first evaluation): lines of Zc ≠ z0 leave impedance steps
        mo = metrics(f, extra["D_nzc_z0"], L, z0)
        for key in ("C_total_fF_3d", "L_total_pH_3d", "tdr_min_3d", "tdr_max_3d", "S21_min_3d_ghz", "S21_min_3d_db", "dS21_max_db_20"):
            m[f"nzc_z0_{key}"] = mo[key]
        lg = json.load(open(os.path.join(REF, k, "ladder_geom.json")))
        m["line_z_kisi_ohm"] = [round(fd["zc"] * (2 if pair else 1), 1) for fd in lg["feeds"]]
        for fx in (8e9, 12e9, 16e9, 20e9):
            m[f"S21_kisi_ladder_{int(fx / 1e9)}"] = round(_interp_db(f, Lk[:, 1, 0], fx), 3)
        sel = (f >= 8e9) & (f <= 25e9)
        i = int(np.argmin(sparam.db(Lk[sel, 1, 0])))
        m["S21_min_kisi_ladder_ghz"], m["S21_min_kisi_ladder_db"] = round(float(f[sel][i] / 1e9), 2), round(float(sparam.db(Lk[sel, 1, 0])[i]), 2)
        if pair:
            # in the lane's own reference (PCIe: 85 Ω differential)
            for tag, X in (("3d", D), ("ladder", L)):
                X85 = renorm(X, z0, 85.0)
                sel = (f >= 8e9) & (f <= 25e9)
                i = int(np.argmin(sparam.db(X85[sel, 1, 0])))
                m[f"S21_85ohm_min_{tag}_ghz"], m[f"S21_85ohm_min_{tag}_db"] = round(float(f[sel][i] / 1e9), 2), round(float(sparam.db(X85[sel, 1, 0])[i]), 2)
                m[f"S11_85ohm_max_{tag}_db"] = round(float(np.max(sparam.db(X85[f <= 20e9 + 1, 0, 0]))), 2)
            sel = f <= 20e9 + 1
            m["Scd21_raw_max_db_20"] = round(float(np.max(sparam.db(raw["cd"][sel, 1, 0]))), 1)
            m["Scd11_raw_max_db_20"] = round(float(np.max(sparam.db(raw["cd"][sel, 0, 0]))), 1)
            m["Sdd21_raw_16_db"] = round(_interp_db(f, raw["dd"][:, 1, 0], 16e9), 2)
        meta = json.load(open(os.path.join(REF, k, "meta.json")))
        m["cells"] = meta["runs"]["kisi"]["cells"]
        m["seconds"] = meta["runs"]["kisi"]["seconds"]
        m["steps"] = meta["runs"]["kisi"]["steps"]
        summ[k] = {"title": VIAS[k]["title"], "metrics": m}
        plot_compare(k, f, D, L, raw, z0, os.path.join(RES, "board_vias", k + ".png"), VIAS[k]["title"], Lk)
        print(k, json.dumps(m), flush=True)
    json.dump({"note": "3D (kisi-fdtd, de-embedded with the 2×thru of each side) against kisi's ladder for the same via; pairs: Sdd at 100 Ω", "cases": summ},
              open(rp, "w"), indent=1)


def deembed_check(a):
    """the de-embedding on a known network: a π-section via (C/2 – L – C/2) between lumped ports
    with 30 pH and 4 mm lines of Zc ≠ z0 (dielectric conductivity fixed at 20 GHz, tanδ 0.02),
    as in the board cases → results/board_vias_deembed_check.json"""
    f = np.arange(1, 401) * 0.1e9
    w = 2 * np.pi * f
    c0 = 299792458.0
    g = 2 * np.pi * 20e9 * 0.02

    def mat(a11, a12, a21, a22):
        M = np.empty((len(f), 2, 2), complex)
        M[:, 0, 0], M[:, 0, 1], M[:, 1, 0], M[:, 1, 1] = a11, a12, a21, a22
        return M

    def line(zhf, eeff, ln):
        zc = zhf * np.sqrt(1j * w / (1j * w + g))
        gl = 1j * w * np.sqrt(eeff) / c0 * np.sqrt((1j * w + g) / (1j * w)) * ln
        return mat(np.cosh(gl), zc * np.sinh(gl), np.sinh(gl) / zc, np.cosh(gl))

    one, zero = np.ones(len(f)), np.zeros(len(f))
    mirror = lambda M: M[:, ::-1, ::-1].transpose(0, 2, 1)

    def a2s(M, z0):
        A, B, C, D = M[:, 0, 0], M[:, 0, 1], M[:, 1, 0], M[:, 1, 1]
        den = A + B / z0 + C * z0 + D
        return mat((A + B / z0 - C * z0 - D) / den, 2 * (A * D - B * C) / den, 2 / den, (-A + B / z0 - C * z0 + D) / den)
    out = {}
    for zin, zout, z0, cv, lv in ((64.0, 62.0, 50.0, 270e-15, 790e-12), (53.0, 51.5, 50.0, 480e-15, 500e-12), (50.0, 50.0, 50.0, 270e-15, 790e-12), (114.0, 94.0, 100.0, 217e-15, 855e-12)):
        half = mat(one, zero, 0.5j * w * cv * (1 - 0.02j * 20e9 / f), one)
        dut = half @ mat(one, 1j * w * lv, zero, one) @ half
        port = mat(one, 30e-12j * w, zero, one)
        fi, fo = port @ line(zin, 3.0, 4e-3), port @ line(zout, 3.5, 4e-3)
        case = a2s(fi @ dut @ mirror(fo), z0)
        ti, to = a2s(fi @ mirror(fi), z0), a2s(fo @ mirror(fo), z0)
        truth = a2s(dut, z0)
        res = {}
        for tag, D in (("nzc_z0", sparam.deembed2(case, sparam.fixture_2xthru(f, ti)[0], sparam.fixture_2xthru(f, to)[0])),
                       ("matched", sparam.deembed_matched(f, case, ti, to, z0)[0])):
            m = metrics(f, D, truth, z0)
            res[tag] = {"max_dS_lin_25": round(float(np.abs(D - truth)[f <= 25e9 + 1].max()), 4), "dS21_max_db_20": m["dS21_max_db_20"],
                        "C_fF": m["C_total_fF_3d"], "L_pH": m["L_total_pH_3d"], "tdr_min": m["tdr_min_3d"], "S11_0.1GHz": round(float(D[0, 0, 0].real), 4)}
        res["truth"] = {"C_fF": m["C_total_fF_ladder"], "L_pH": m["L_total_pH_ladder"], "tdr_min": m["tdr_min_ladder"], "S11_0.1GHz": round(float(truth[0, 0, 0].real), 4)}
        key = f"lines {zin:g}/{zout:g} ohm, ref {z0:g} ohm"
        out[key] = res
        print(key, json.dumps(res), flush=True)
    json.dump({"note": "known π-section via between 30 pH ports and 4 mm lossy lines of Zc ≠ z0: the plain z0-referred NZC against the matched de-embedding", "cases": out},
              open(os.path.join(RES, "board_vias_deembed_check.json"), "w"), indent=1)


def block1(a):
    """the Block 1 single-via references de-embedded both ways (z0-referred NZC as in via_pbm.py,
    and matched) against kisi's ladder → results/board_vias_block1_deembed.json"""
    out = {}
    for p in sorted(glob.glob(os.path.join(HERE, "via_cases", "*.json"))):
        cid = os.path.basename(p)[:-5]
        c = json.load(open(p))
        if c["meta"]["group"] not in "ABCDX" or not all(os.path.exists(os.path.join(REF, cid, x)) for x in ("kisi.s2p", "kisi_deembedded.s2p", "ladder.s2p")):
            continue
        th = [os.path.join(REF, f"thru_{c['meta']['stackup']}_{q['layer']}", "kisi.s2p") for q in c["meta"]["ports"]]
        f, S = sparam.read_touchstone(os.path.join(REF, cid, "kisi.s2p"))
        T = [sparam.read_touchstone(t)[1][: len(f)] for t in th]
        _, Dold = sparam.read_touchstone(os.path.join(REF, cid, "kisi_deembedded.s2p"))
        Dm, zc = sparam.deembed_matched(f, S, T[0], T[1], 50.0)
        _, L = sparam.read_touchstone(os.path.join(REF, cid, "ladder.s2p"))
        mo, mm = metrics(f, Dold[: len(f)], L[: len(f)], 50.0), metrics(f, Dm, L[: len(f)], 50.0)
        row = {"line_z_3d_ohm": [round(z, 1) for z in zc]}
        for key in ("C_total_fF", "L_total_pH", "tdr_min", "tdr_max"):
            row[key] = {"nzc_z0": mo[key + "_3d"], "matched": mm[key + "_3d"], "ladder": mo[key + "_ladder"]}
        row["dS21_max_db_20"] = {"nzc_z0": mo["dS21_max_db_20"], "matched": mm["dS21_max_db_20"]}
        out[cid] = row
        print(cid, json.dumps(row), flush=True)
    json.dump({"note": "Block 1 single vias: 3D de-embedded with the z0-referred NZC (kisi_deembedded.s2p) and with deembed_matched, against kisi's ladder (ladder.s2p)", "cases": out},
              open(os.path.join(RES, "board_vias_block1_deembed.json"), "w"), indent=1)


def variants(a):
    """kisi's ladder for the same via without the planes on the via's signal layers (the GND
    pours there) and without the junction model, against the 3D result →
    results/board_vias_variants.json"""
    import copy
    import tempfile
    tmp = tempfile.mkdtemp(prefix="bvvar_")
    out = {}
    for k in selected(a):
        f, D, raw, _, c, _ = deembedded(k)
        z0 = 100.0 if "cd" in raw else 50.0
        g = json.load(open(os.path.join(REF, k, "ladder3d_geom.json")))
        # the via's signal layers (entry and exit), where the "planes" are GND pours around the feeds
        zs = [g["feeds"][0]["z"], g["feeds"][1]["z"]]
        nop = copy.deepcopy(g)
        nop["planes"] = [p for p in g["planes"] if not any(abs(p["z"] - z) < 1e-6 for z in zs)]
        nop["coplanar"] = []
        # before Block 5 the pours on the signal layers were planes (no `coplanar` list)
        old = dict(copy.deepcopy(g), coplanar=[])
        vs = {"as is": g, "pours as planes (before Block 5)": old, "no planes on the signal layers": nop, "no junction model": dict(copy.deepcopy(g), junction=False),
              "neither": dict(copy.deepcopy(nop), junction=False)}
        out[k] = {"removed_plane_z_mm": [round(z, 4) for z in zs]}
        for nm, gg in vs.items():
            p = os.path.join(tmp, "g.json")
            json.dump(gg, open(p, "w"))
            subprocess.run([tool(a, "validate"), "via-geom", p, "--s2p", p[:-5] + ".s2p"], capture_output=True, text=True, check=True)
            _, L = sparam.read_touchstone(p[:-5] + ".s2p")
            m = metrics(f, D, L[: len(f)], z0)
            out[k][nm] = {"C_total_fF": m["C_total_fF_ladder"], "L_total_pH": m["L_total_pH_ladder"], "delay_ps": m["delay_5_15ghz_ladder_ps"], "tdr_min": m["tdr_min_ladder"],
                          "S21_min_ghz": m["S21_min_ladder_ghz"], "S21_min_db": m["S21_min_ladder_db"], "dS21_max_db_20": m["dS21_max_db_20"], "dphase_max_deg_20": m["dphase_max_deg_20"]}
        out[k]["3d"] = {"C_total_fF": m["C_total_fF_3d"], "L_total_pH": m["L_total_pH_3d"], "delay_ps": m["delay_5_15ghz_3d_ps"], "tdr_min": m["tdr_min_3d"],
                        "S21_min_ghz": m["S21_min_3d_ghz"], "S21_min_db": m["S21_min_3d_db"]}
        print(k, json.dumps(out[k]), flush=True)
    json.dump({"note": "kisi's ladder (3D loss model) for the matched via geometry, variants, against the de-embedded 3D result", "cases": out},
              open(os.path.join(RES, "board_vias_variants.json"), "w"), indent=1)


# the board vias with a 3D reference: (case, folder of the case, folder of the result, board, config)
FIX_VIAS = [
    ("BV_ts_m2_per0", CASES, REF, "Thunderscope_Rev5.3", "ts"),
    ("BV_ts_refinout2", CASES, REF, "Thunderscope_Rev5.3", "ts"),
    ("BV_vna_rf_hblb1", CASES, REF, "VNA", "vna"),
    ("BV_vna_clk_w5", os.path.join(HERE, "resonance_cases"), REF, "VNA", "vna"),
]


def board_ladder_geom(a, case, board, cfg, env_extra=None):
    """kisi's ladder geometry for the via of a board case (KISI_DEBUG_VIA, matched by its return
    vias), with the 3D loss model and between lines to the via centre (dropped = 0)"""
    c = json.load(open(case))
    p = c["meta"]["params"]
    pcb = os.path.join(a.boards, board + ".kicad_pcb")
    env = dict(os.environ, KISI_DEBUG_VIA="1", **(env_extra or {}))
    r = subprocess.run([tool(a, "kisi"), "net", pcb, p["net"], "--config", os.path.join(ROOT, "web", "samples", cfg + ".kisi.json")], capture_output=True, text=True, env=env)
    want = c["meta"]["kisi_returns"]
    pair = p["partner"] is not None
    for line in r.stderr.splitlines():
        if not line.startswith("{"):
            continue
        g = json.loads(line)
        rr = [[q["dx"], q["dy"]] for q in g["returns"]]
        if all(any(abs(x[0] - y[0]) < 2e-3 and abs(x[1] - y[1]) < 2e-3 for x in rr) for y in want) and ((g["partner"] is not None) == pair):
            g = dict(g, f_loss=c.get("f_loss") or c["fmax"] / 2, sigma=None)
            g["feeds"] = [dict(fd, dropped=0.0) for fd in g["feeds"]]
            return g
    raise SystemExit(f"{case}: no ladder geometry matches")


def fixes(a):
    """Blocks 5 and 6: the ladder before (pours on the signal layers as planes, KISI_VIA_POURS=planes),
    after the coplanar treatment (Block 5, the return posts as before: vias only, nearest 8) and
    with plated pads as return posts, the nearest 12 (Block 6), all between lines to the via
    centre, against the de-embedded 3D result of every board via → results/board_vias_fixes.json"""
    import tempfile
    tmp = tempfile.mkdtemp(prefix="bvfix_")
    out = {}
    for k, cdir, rdir, board, cfg in FIX_VIAS:
        if a.only and not any(x in k for x in a.only.split(",")):
            continue
        case = os.path.join(cdir, k + ".json")
        if os.path.exists(os.path.join(rdir, k, "deembedded.s2p")) and k == "BV_vna_clk_w5":
            f, D = sparam.read_touchstone(os.path.join(rdir, k, "deembedded.s2p"))
            z0 = 50.0
        else:
            f, D, raw, _, _, _ = deembedded(k)
            z0 = 100.0 if "cd" in raw else 50.0
        row = {}
        old_posts = {"KISI_RET_PADS": "0", "KISI_RET_CAP": "8"}
        for tag, env in (("before", dict(old_posts, KISI_VIA_POURS="planes")), ("after", old_posts), ("block6", {})):
            g = board_ladder_geom(a, case, board, cfg, env)
            gp = os.path.join(tmp, f"{k}_{tag}.json")
            json.dump(g, open(gp, "w"))
            subprocess.run([tool(a, "validate"), "via-geom", gp, "--s2p", gp[:-5] + ".s2p"], capture_output=True, text=True, check=True)
            fl, L = sparam.read_touchstone(gp[:-5] + ".s2p")
            sel = f <= fl[-1] + 1
            ff, DD = f[sel], D[sel]
            LL = np.stack([np.stack([np.interp(ff, fl, L[:, i, j].real) + 1j * np.interp(ff, fl, L[:, i, j].imag) for j in range(2)], -1) for i in range(2)], -2)
            m = metrics(ff, DD, LL, z0)
            e = {}
            for lo, hi in ((0.5e9, 6e9), (6e9, 12e9), (12e9, 20e9), (20e9, 25e9)):
                sel = (ff >= lo) & (ff <= hi + 1)
                dd = np.abs(sparam.db(DD[sel, 1, 0]) - sparam.db(LL[sel, 1, 0]))
                e[f"{lo / 1e9:g}-{hi / 1e9:g}"] = [round(float(dd.max()), 3), round(float(np.sqrt(np.mean(dd ** 2))), 3)]
            i20 = int(np.argmin(abs(ff - 20e9)))
            lossf = lambda X: round(float(100 * (1 - abs(X[i20, 0, 0]) ** 2 - abs(X[i20, 1, 0]) ** 2)), 2)
            row["3d_loss_pct_20"] = lossf(DD)
            row[tag] = {"loss_pct_20": lossf(LL), "returns": len(g["returns"]), "dS21_max_rms_db": e, "dphase_max_deg_20": m["dphase_max_deg_20"], "C_fF": m["C_total_fF_ladder"], "L_pH": m["L_total_pH_ladder"],
                        "delay_ps": m["delay_5_15ghz_ladder_ps"], "tdr_min": m["tdr_min_ladder"], "tdr_max": m["tdr_max_ladder"], "S21_min_ghz": m["S21_min_ladder_ghz"],
                        "S21_min_db": m["S21_min_ladder_db"], "coplanar_mm": g.get("coplanar", [])}
        row["3d"] = {"C_fF": m["C_total_fF_3d"], "L_pH": m["L_total_pH_3d"], "delay_ps": m["delay_5_15ghz_3d_ps"], "tdr_min": m["tdr_min_3d"], "tdr_max": m["tdr_max_3d"],
                     "S21_min_ghz": m["S21_min_3d_ghz"], "S21_min_db": m["S21_min_3d_db"]}
        out[k] = row
        print(k, json.dumps(row), flush=True)
    json.dump({"note": "kisi's ladder (3D loss model, between lines to the via centre) with the pours on the via's signal layers as planes (before), as coplanar returns (after, Block 5) and with plated pads among the return posts, the nearest 12 (block6), against the de-embedded 3D result; pairs Sdd at 100 Ω",
               "cases": out}, open(os.path.join(RES, "board_vias_fixes.json"), "w"), indent=1)


def diag(a):
    """the diagnostic case of REFINOUT2 with In2 as a solid GND plane joined to every GND via
    (board_cases/diag) → ../via_ref/diag, compared with the board case"""
    import run_refs
    ra = argparse.Namespace(exe=tool(a, "fdtd"), fine=0.0, openems=None)
    dd, rd = os.path.join(CASES, "diag"), os.path.join(REF, "diag")
    name = "BV_ts_refinout2_in2gnd"
    c = json.load(open(os.path.join(dd, name + ".json")))
    for nm in c["meta"]["thru"] + [name]:
        if not os.path.exists(os.path.join(rd, nm, "kisi.s2p")) or a.force:
            run_refs.one(os.path.join(dd, nm + ".json"), rd, ra)
    f, D, *_ = deembedded(name, rd, dd)
    fb, Db, *_ = deembedded("BV_ts_refinout2")
    _, L = sparam.read_touchstone(os.path.join(REF, "BV_ts_refinout2", "ladder_3dloss.s2p"))
    m, mb = metrics(f, D, L[: len(f)], 50.0), metrics(fb, Db, L[: len(fb)], 50.0)
    for key in ("S21_min_25_40_3d_ghz", "S21_min_25_40_3d_db", "C_total_fF_3d", "L_total_pH_3d", "delay_5_15ghz_3d_ps", "tdr_min_3d"):
        print(f"{key}: In2 grounded {m[key]}, board {mb[key]}")


# --- quick meshes (resource estimate) ------------------------------------------------------
QUICK = {  # variant → exporter options (on top of the case's own), and changes to the exported case
    "q050": (["--dfine", "0.05", "--keep-cell", "0.07"], {}),
    "q050f25s": (["--dfine", "0.05", "--keep-cell", "0.07", "--fmax", "25e9", "--feed", "3", "--margin", "1.8"], {}),
    "q070f25s": (["--dfine", "0.07", "--keep-cell", "0.1", "--fmax", "25e9", "--feed", "3", "--margin", "1.8"], {}),
    # the same, stopped at −35 dB of the peak energy instead of −50 dB (plane cavities and resonances
    # above the band ring long but hardly change the S-parameters); the fixtures are those of the
    # variant named last ("": the standard case), which they equal bit for bit
    "q050f25s_e35": (["--dfine", "0.05", "--keep-cell", "0.07", "--fmax", "25e9", "--feed", "3", "--margin", "1.8"], {"end_db": -35}, "q050f25s"),
    "q070f25s_e35": (["--dfine", "0.07", "--keep-cell", "0.1", "--fmax", "25e9", "--feed", "3", "--margin", "1.8"], {"end_db": -35}, "q070f25s"),
    "std_e35": ([], {"end_db": -35}, ""),  # the standard mesh, only the end criterion changed
}


def quick(a):
    """coarser meshes of a case against its base mesh: export, run, de-embed, compare up to the
    variant's fmax → results/board_vias_quick.json"""
    import run_refs
    ra = argparse.Namespace(exe=tool(a, "fdtd"), fine=0.0, openems=None)
    qdir = os.path.join(CASES, "quick")
    os.makedirs(qdir, exist_ok=True)
    rp = os.path.join(RES, "board_vias_quick.json")
    out = json.load(open(rp)) if os.path.exists(rp) else {}
    variants = [q for q in QUICK if not a.variants or q in a.variants.split(",")]
    for k in selected(a):
        v = VIAS[k]
        pcb, cfg = board_files(a, v)
        fb, Db, rawb, _, cb, _ = deembedded(k)
        z0 = 100.0 if "cd" in rawb else 50.0
        for q in variants:
            name = f"{k}_{q}"
            opts = list(v["opts"]) + QUICK[q][0]
            r = subprocess.run([tool(a, "validate"), "via-case", pcb, "--net", v["net"], "--config", cfg, "--via", str(v["via"]), "--name", name, "--out", qdir] + opts, capture_output=True, text=True)
            if r.returncode:
                raise SystemExit(r.stderr)
            c = json.load(open(os.path.join(qdir, name + ".json")))
            c.update(QUICK[q][1])
            if len(QUICK[q]) > 2:
                # the fixtures of another variant (or of the standard case)
                src = f"{k}_{QUICK[q][2]}" if QUICK[q][2] else k
                for t in c["meta"]["thru"]:
                    os.remove(os.path.join(qdir, t + ".json"))
                c["meta"]["thru"] = json.load(open(os.path.join(qdir if QUICK[q][2] else CASES, src + ".json")))["meta"]["thru"]
                c["meta"]["thru_dir"] = "quick" if QUICK[q][2] else "."
            json.dump(c, open(os.path.join(qdir, name + ".json"), "w"))
            secs, cells, steps = 0.0, 0, []
            for nm in c["meta"]["thru"] + [name]:
                rd, cd = os.path.join(REF, "quick"), qdir
                if nm != name and "thru_dir" in c["meta"]:
                    rd, cd = os.path.join(REF, c["meta"]["thru_dir"]), (qdir if c["meta"]["thru_dir"] == "quick" else CASES)
                n = len(json.load(open(os.path.join(cd, nm + ".json")))["ports"])
                if not os.path.exists(os.path.join(rd, nm, f"kisi.s{n}p")) or (a.force and (nm == name or "thru_dir" not in c["meta"])):
                    run_refs.one(os.path.join(cd, nm + ".json"), rd, ra)
                mt = json.load(open(os.path.join(rd, nm, "meta.json")))["runs"]["kisi"]
                secs += mt["seconds"]
                if nm == name:
                    cells, steps = mt["cells"], mt["steps"]
            f, D, _, _, _, _ = deembedded(name, os.path.join(REF, "quick"), qdir)
            # on the base grid up to the variant's fmax (0.1 GHz steps in both)
            fm = min(f.max(), 25e9)
            kb, kq = fb <= fm + 1, f <= fm + 1
            Dq = np.empty((kb.sum(), 2, 2), complex)
            for i in range(2):
                for j in range(2):
                    Dq[:, i, j] = np.interp(fb[kb], f[kq], D[kq, i, j].real) + 1j * np.interp(fb[kb], f[kq], D[kq, i, j].imag)
            mb = metrics(fb[kb], Db[kb], Dq, z0)
            res = {"cells": cells, "steps": steps, "seconds_total_incl_thru": round(secs, 1), "dfine": c["mesh"]["dfine"], "fmax": c["fmax"], "end_db": c["end_db"],
                   "dS21_max_db_20": mb["dS21_max_db_20"], "dS21_max_db_25": mb["dS21_max_db_25"], "dphase_max_deg_20": mb["dphase_max_deg_20"],
                   "tdr_min_base": mb["tdr_min_3d"], "tdr_min_quick": mb["tdr_min_ladder"], "tdr_max_base": mb["tdr_max_3d"], "tdr_max_quick": mb["tdr_max_ladder"],
                   "S21_min_base_ghz": mb["S21_min_3d_ghz"], "S21_min_quick_ghz": mb["S21_min_ladder_ghz"],
                   "dS11_max_lin_20": round(float(np.max(np.abs(Dq[:, 0, 0] - Db[kb, 0, 0])[fb[kb] <= 20e9 + 1])), 4),
                   "C_total_fF_base": mb["C_total_fF_3d"], "C_total_fF_quick": mb["C_total_fF_ladder"],
                   "L_total_pH_base": mb["L_total_pH_3d"], "L_total_pH_quick": mb["L_total_pH_ladder"]}
            out.setdefault(k, {})[q] = res
            print(name, json.dumps(res), flush=True)
            json.dump(out, open(rp, "w"), indent=1)


# --- speed: setup and step rate natively and in WebAssembly ---------------------------------
def speed(a):
    """one excitation, 1 and `--steps` time steps (setup = the first, step rate from the
    difference), native and with the wasm build (--wasm, node) → results/board_vias_speed.json"""
    import tempfile

    def run_rss(cmd):
        """stdout and peak resident memory (MB) of one child process"""
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        out = p.stdout.read()
        _, status, ru = os.wait4(p.pid, 0)
        p.returncode = os.waitstatus_to_exitcode(status)
        if p.returncode:
            raise SystemExit(f"{cmd[0]} failed ({p.returncode})")
        return out, ru.ru_maxrss / 1024
    tmp = tempfile.mkdtemp(prefix="bvspeed_")
    rp = os.path.join(RES, "board_vias_speed.json")
    out = json.load(open(rp)) if os.path.exists(rp) else {}
    names = [x for x in a.args] or selected(a)
    for k in names:
        cp = k if k.endswith(".json") else os.path.join(CASES, k + ".json")
        c = json.load(open(cp))
        c.update(excite=[0], monitors=[], end_db=-300)
        mp = os.path.join(tmp, "mesh.json")
        open(mp, "w").write(subprocess.run([tool(a, "fdtd"), cp, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
        row = {}
        for ns in (1, a.steps):
            c["max_steps"] = ns
            p = os.path.join(tmp, f"c{ns}.json")
            json.dump(c, open(p, "w"))
            o, rss = run_rss([tool(a, "fdtd"), p, "--mesh", mp])
            r = json.loads(o)
            row[f"native_{ns}"] = round(r["runs"][0]["seconds"], 2)
            row["cells"] = r["cells"]
            row["native_max_rss_mb"] = round(max(rss, row.get("native_max_rss_mb", 0)))
            if a.wasm:
                o = subprocess.run(["node", os.path.join(HERE, "wasm_speed.mjs"), a.wasm, p, mp, os.path.join(tmp, "w.json")], capture_output=True, text=True, check=True).stdout
                row[f"wasm_{ns}"] = float(o.split("wasm ")[1].split(" s")[0])
                # the module's linear memory (the field arrays and the case live there)
                row["wasm_memory_mb"] = int(o.split("memory ")[1].split(" MB")[0])
        n = a.steps - 1
        row["native_setup_s"] = round(row["native_1"], 2)
        row["native_mcells_s"] = round(row["cells"] * n / (row[f"native_{a.steps}"] - row["native_1"]) / 1e6, 1)
        if a.wasm:
            row["wasm_setup_s"] = round(row["wasm_1"], 2)
            row["wasm_mcells_s"] = round(row["cells"] * n / (row[f"wasm_{a.steps}"] - row["wasm_1"]) / 1e6, 1)
        row["bytes_per_cell"] = round(row["native_max_rss_mb"] * 2 ** 20 / row["cells"], 1)
        if a.threads > 1:
            # native field updates on several threads (the browser build runs one thread per run)
            c.update(max_steps=a.steps, threads=a.threads)
            p = os.path.join(tmp, "ct.json")
            json.dump(c, open(p, "w"))
            r = json.loads(run_rss([tool(a, "fdtd"), p, "--mesh", mp])[0])
            row[f"native_{a.threads}threads_mcells_s"] = round(row["cells"] * n / (r["runs"][0]["seconds"] - row["native_1"]) / 1e6, 1)
        out[os.path.basename(cp)[:-5]] = row
        print(os.path.basename(cp), json.dumps(row), flush=True)
        json.dump(out, open(rp, "w"), indent=1)


# --- plots -------------------------------------------------------------------------------
def plot_case(case, out, mesh=None, zoom=None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.patches import Polygon, Rectangle, Circle
    zs = sorted({round(s["z"], 6) for s in case["sheets"]} | {round(z, 6) for z in case["meta"].get("copper_z", [])}, reverse=True)
    names = {}
    cz = case["meta"].get("copper_z", [])
    for i, z in enumerate(cz):
        names[round(z, 6)] = f"L{i + 1}"
    n = len(zs)
    cols = min(n, 3)
    rows = math.ceil(n / cols)
    fig, axs = plt.subplots(rows, cols, figsize=(4.2 * cols, 4.2 * rows), squeeze=False)
    dom = case["domain"]
    netcol = {"SIG": "#c0392b"}
    for k, z in enumerate(zs):
        ax = axs[k // cols][k % cols]
        ax.set_title(f"{names.get(z, '')} z = {z:.3f} mm", fontsize=9)
        for s in case["sheets"]:
            if abs(s["z"] - z) > 1e-6:
                continue
            col = netcol.get(s.get("net"), "#b9770e" if s.get("net") in ("GND", "/GND") or "GND" in str(s.get("net")) else "#7f8c8d")
            if s.get("polys"):
                for ring in s["polys"]:
                    ax.add_patch(Polygon(ring, closed=True, fc=col, ec="none", alpha=0.55))
            else:
                r = s["rect"]
                ax.add_patch(Rectangle((r[0], r[1]), r[2] - r[0], r[3] - r[1], fc=col, ec="none", alpha=0.55))
            for h in s.get("holes", []):
                if len(h) == 3:
                    ax.add_patch(Circle((h[0], h[1]), h[2], fc="white", ec="none"))
        for v in case["vias"]:
            if v["z0"] - 1e-6 <= z <= v["z1"] + 1e-6:
                sig = v.get("net") == "SIG"
                if sig and any(abs(p - z) < 1e-6 for p in v.get("pads", [])):
                    ax.add_patch(Circle((v["x"], v["y"]), v["pad_r"], fc="#e74c3c", ec="k", lw=0.3))
                ax.add_patch(Circle((v["x"], v["y"]), v["r"], fc="k" if sig else "#5d4037", ec="none"))
        for i, p in enumerate(case["ports"]):
            if min(p["z0"], p["z1"]) - 1e-6 <= z <= max(p["z0"], p["z1"]) + 1e-6:
                if "x1" in p:
                    ax.plot([p["x"], p["x1"]], [p["y0"], p["y0"]], "b-", lw=2)
                    ax.text(p["x"], p["y0"], str(i + 1), color="b", fontsize=8, clip_on=True)
                else:
                    ax.plot([p["x"], p["x"]], [p["y0"], p["y1"]], "b-", lw=2)
                    ax.text(p["x"], p["y1"], str(i + 1), color="b", fontsize=8, clip_on=True)
        if mesh is not None:
            for x in mesh["x"]:
                ax.axvline(x, color="#2980b9", lw=0.2 if zoom else 0.1, alpha=0.6)
            for y in mesh["y"]:
                ax.axhline(y, color="#2980b9", lw=0.2 if zoom else 0.1, alpha=0.6)
        if zoom:
            ax.set_xlim(-zoom, zoom)
            ax.set_ylim(-zoom, zoom)
        else:
            ax.set_xlim(dom[0], dom[3])
            ax.set_ylim(dom[1], dom[4])
        ax.set_aspect("equal")
        ax.tick_params(labelsize=7)
    for k in range(n, rows * cols):
        axs[k // cols][k % cols].axis("off")
    fig.suptitle(case["meta"].get("title", case["name"]), fontsize=9)
    fig.tight_layout()
    fig.savefig(out, dpi=110)
    plt.close(fig)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd")
    ap.add_argument("args", nargs="*")
    ap.add_argument("--only", default="")
    ap.add_argument("--kisi")
    ap.add_argument("--validate")
    ap.add_argument("--fdtd")
    ap.add_argument("--boards", default=os.path.join(ROOT, "examples"))
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--variants", default="")
    ap.add_argument("--wasm", default="")
    ap.add_argument("--steps", type=int, default=2001)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--mesh", action="store_true")
    ap.add_argument("--zoom", type=float, default=None)
    a = ap.parse_args()
    if a.cmd == "plot":
        case = json.load(open(a.args[0]))
        mesh = None
        if a.mesh:
            mesh = json.loads(subprocess.run([tool(a, "fdtd"), a.args[0], "--mesh-only"], capture_output=True, text=True, check=True).stdout)
            print(f"mesh {len(mesh['x'])} × {len(mesh['y'])} × {len(mesh['z'])} = {len(mesh['x']) * len(mesh['y']) * len(mesh['z']) / 1e6:.2f} M cells")
        plot_case(case, a.args[1], mesh, a.zoom)
        return
    cmds = {"export": export, "run": run, "ladder": ladder, "compare": compare, "quick": quick, "speed": speed, "deembed-check": deembed_check,
            "block1": block1, "variants": variants, "diag": diag, "fixes": fixes}
    if a.cmd not in cmds:
        raise SystemExit(f"unknown command {a.cmd}")
    cmds[a.cmd](a)


if __name__ == "__main__":
    main()
