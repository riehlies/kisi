"""Plane resonances of finite boards: via transitions through a finite plane cavity and via-to-via
coupling through it, in 3D (kisi-fdtd) against kisi's ladder with the cavity model
(validation/fdtd/BOARD_RESONANCES.md).

  python3 board_resonances.py cases                write resonance_cases/*.json (cases and 2×thru)
  python3 board_resonances.py mesh                 cells and time step of every case (no run)
  python3 board_resonances.py run [--only K]       kisi-fdtd, one case at a time → via_ref/<case>/
  python3 board_resonances.py model [--only K]     kisi's ladder (infinite planes and cavity) per case
  python3 board_resonances.py compare [--only K]   de-embedding, resonances, coupling, plots,
                                                   results/board_resonances.json
  python3 board_resonances.py block2               the cavity option on the Block 2 board vias
  python3 board_resonances.py summary              table of results/board_resonances.json

Cases (4-layer 1.6 mm FR-4 stack "s4" of via_cases.py: L1 / GND L2 / 1.2 mm core / GND L3 / L4,
the via L1 → L4 drill Ø0.3, antipad Ø0.8 as A_s4_d300_ap800; the board ends 3 mm before the PML,
air around it, so its edges are open):
  R_s4_rect_n0     24 × 16 mm board, one via, no return via
  R_s4_rect_n1     the same with one GND via 2 mm away
  R_s4_rect_n4     the same with four GND vias at 1 mm (as the A cases)
  R_s4_rect_2v     two vias 9.9 mm apart sharing the cavity, no return via (4 ports, port 1 excited)
  R_s4_lshape_n0   L-shaped planes (24 × 16 mm minus a 8 × 6 mm corner), one via, no return via
  R_s4_rect_cap5   R_s4_rect_n0 with a capacitor (100 nF, 10 mΩ, 0.6 nH) between the planes 5 mm away
  thru_R_s4_L1/L4  2×thru fixtures of the feeds on the same mesh
A real board via (not written by `cases`; exported with kisi-validate via-case, group "board"):
  BV_vna_clk_w5    LibreVNA /FPGA/FPGA_CLK_R via 0, F.Cu → In3.Cu, a 11.8 × 10.4 mm window of the board
                   with its stitching vias; `model` takes kisi's ladder from `kisi net` (KISI_DUMP_VIA)
    kisi-validate via-case examples/VNA.kicad_pcb --net /FPGA/FPGA_CLK_R --via 0 --keep 1.1 --feed 3.0 \\
      --config web/samples/vna.kisi.json --out validation/fdtd/resonance_cases --name BV_vna_clk_w5 \\
      --margin 5 --dfine 0.07 --keep-cell 0.07 --fmax 25e9      (then "nf": 1000 in the three files, and
                   "ref_refine": 2 in the meta of BV_vna_clk_w5: its reference has the cell at metal halved, TESTS.md
                   section 17)
"""
import argparse, glob, json, math, os, subprocess, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
CASES = os.path.join(HERE, "resonance_cases")
RES = os.path.join(HERE, "results")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
import via_cases as vc  # noqa: E402

C0 = 299792458.0
BOARD = [-12.0, -8.0, 12.0, 8.0]  # board outline (mm)
MARGIN = 3.0                      # air beside the board edges (mm)
AIR = 1.5                         # air above and below (mm)
VIA_A = (-3.3, -1.9)              # off the nodal lines of the low modes
VIA_B = (5.2, 3.1)
FMAX, DFINE = 25e9, 0.07          # the quick mesh of BOARD_VIAS.md for single vias (0.07 mm, 25 GHz)
DMAX = 0.35                       # largest cell: λ/16 in the core at 25 GHz
LSHAPE = [[-12.0, -8.0], [12.0, -8.0], [12.0, 2.0], [4.0, 2.0], [4.0, 8.0], [-12.0, 8.0]]


def builder():
    hx = (BOARD[2] - BOARD[0]) / 2 + MARGIN
    hy = (BOARD[3] - BOARD[1]) / 2 + MARGIN
    b = vc.Builder("s4", hx, hy, FMAX, DFINE, board=list(BOARD), air=AIR)
    b.case["mesh"]["dmax"] = DMAX
    return b


def signal_via(b, at, name0):
    """via L1 → L4 at `at` with feeds to the left on L1 and to the right on L4"""
    x, y = at
    b.via(x, y, 0.15, 0.3, 0, 3, [0, 3])
    for p in b.st["planes"]:
        b.antipad(p, x, y, 0.4)
    b.feed_line(-1, 0, y, x, f"{name0} L1")
    b.feed_line(+1, 3, y, x, f"{name0 + 1} L4")


def gnd_via(b, x, y):
    b.via(round(x, 4), round(y, 4), 0.15, 0.3, 0, 3, [], net="GND")


def finish(b, cid, title, params, excite=None, polys=None):
    c = b.finish(cid, "R", title, params)
    if polys is not None:  # plane outline as a polygon (the board stays rectangular)
        for s in c["sheets"]:
            if s.get("net") in ("GND", "PWR") and s["rect"] == c["board"]:
                s["polys"] = [polys]
    f10 = C0 / (2 * (BOARD[2] - BOARD[0]) * 1e-3 * math.sqrt(4.5))
    c.update(end_db=-35, max_steps=300000, nf=1000)
    # the cavity field: top view in the middle of the core at three frequencies
    c["monitors"] = [{"axis": "z", "pos": 0.8, "freqs": [round(f10 * k / 1e6) * 1e6 for k in (1.0, 1.5, 1.8)], "comps": [2]}]
    c["meta"].update(board=list(BOARD), via=list(VIA_A), thru=["thru_R_s4_L1", "thru_R_s4_L4"], cavity={"layers": ["L2", "L3"], "d_mm": 1.2, "er": 4.5, "tand": 0.02})
    if excite is not None:
        c["meta"]["excite"] = excite
    return c


def cases():
    out = []
    # one via, 0 / 1 / 4 return vias
    for n, rets in ((0, []), (1, [(VIA_A[0], VIA_A[1] + 2.0)]), (4, [(VIA_A[0] + dx, VIA_A[1] + dy) for dx, dy in vc.ring(4, 1.0)])):
        b = builder()
        signal_via(b, VIA_A, 1)
        for (x, y) in rets:
            gnd_via(b, x, y)
        what = {0: "no return via", 1: "one GND via 2 mm away", 4: "four GND vias at 1 mm"}[n]
        out.append(finish(b, f"R_s4_rect_n{n}", f"Finite board 24 × 16 mm, via L1 → L4 through the L2/L3 cavity, {what}",
                          {"board_mm": "24 × 16", "returns": n, "via_mm": list(VIA_A)}))
    # two vias sharing the cavity
    b = builder()
    signal_via(b, VIA_A, 1)
    signal_via(b, VIA_B, 3)
    c = finish(b, "R_s4_rect_2v", "Finite board 24 × 16 mm, two vias L1 → L4 9.9 mm apart sharing the L2/L3 cavity, no return via",
               {"board_mm": "24 × 16", "returns": 0, "via_mm": list(VIA_A), "via_b_mm": list(VIA_B)}, excite=[0])
    c["meta"]["via_b"] = list(VIA_B)
    out.append(c)
    # L-shaped planes
    b = builder()
    signal_via(b, VIA_A, 1)
    c = finish(b, "R_s4_lshape_n0", "L-shaped planes (24 × 16 mm minus an 8 × 6 mm corner), via L1 → L4, no return via",
               {"board_mm": "24 × 16 minus 8 × 6", "returns": 0, "via_mm": list(VIA_A)}, polys=LSHAPE)
    c["meta"]["plane_outline"] = LSHAPE
    out.append(c)
    # a capacitor between the planes
    b = builder()
    signal_via(b, VIA_A, 1)
    cap = (VIA_A[0], VIA_A[1] + 5.0)
    b.lumped(cap[0], cap[1], b.zc[2], b.zc[1], r=0.01, l=0.6e-9, c=100e-9, name="100 nF, ESR 10 mΩ, ESL 0.6 nH")
    c = finish(b, "R_s4_rect_cap5", "Finite board 24 × 16 mm, via L1 → L4, a capacitor between the planes 5 mm away, no return via",
               {"board_mm": "24 × 16", "returns": 0, "via_mm": list(VIA_A), "cap_mm": list(cap)})
    c["meta"]["caps"] = [{"at": list(cap), "r": 0.01, "l": 0.6e-9, "c": 100e-9}]
    out.append(c)
    # 2×thru of both feeds on the same mesh (infinite planes)
    for layer in (0, 3):
        t = vc.thru("s4", layer, FMAX, DFINE)
        t["name"] = f"thru_R_s4_L{layer + 1}"
        t["mesh"]["dmax"] = DMAX
        t["nf"] = 1000
        t["monitors"] = []
        out.append(t)
    return out


def tool(a, name):
    exe = {"kisi": "kisi", "validate": "kisi-validate", "fdtd": "kisi-fdtd"}[name]
    return getattr(a, name, None) or os.path.join(ROOT, "target", "release", exe)



def selected(a):
    keys = [k for k in (a.only or "").split(",") if k]
    names = sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(CASES, "*.json")))
    # fixtures first
    names.sort(key=lambda n: (not n.startswith("thru"), n))
    return [n for n in names if not keys or any(k in n for k in keys)]


def cmd_cases(a):
    os.makedirs(CASES, exist_ok=True)
    for c in cases():
        json.dump(c, open(os.path.join(CASES, c["name"] + ".json"), "w"), indent=1)
    print(f"{len(cases())} cases in {CASES}")


def cmd_mesh(a):
    for n in selected(a):
        p = os.path.join(CASES, n + ".json")
        m = json.loads(subprocess.run([tool(a, "fdtd"), p, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
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


# --- kisi's ladder on the cases -------------------------------------------------------------
def cap_radius(a, n):
    """Peaceman radius of the cell of a lumped capacitor in the 3D mesh (the lumped element is one
    z-edge column at the nearest node): 0.14·√(dx² + dy²) of the node's dual cell"""
    c = json.load(open(os.path.join(CASES, n + ".json")))
    if not c.get("lumped"):
        return None
    m = json.loads(subprocess.run([tool(a, "fdtd"), os.path.join(CASES, n + ".json"), "--mesh-only"], capture_output=True, text=True, check=True).stdout)
    x, y = np.array(m["x"]), np.array(m["y"])
    el = c["lumped"][0]
    i, j = int(np.argmin(abs(x - el["x"]))), int(np.argmin(abs(y - el["y0"])))
    dx, dy = (x[i + 1] - x[i - 1]) / 2, (y[j + 1] - y[j - 1]) / 2
    return float(0.14 * math.hypot(dx, dy))


def board_ladders(a, n, c):
    """a via cut out of a board (kisi-validate via-case): kisi's ladder as the channel builds it,
    infinite plates and with the cavities (KISI_VIA_CAVITY=1), with the 3D loss model
    (KISI_VIA_3DLOSS) → <case>/ladder_inf.s2p, ladder_grid.s2p (via KISI_DUMP_VIA into a temporary
    folder, ports in the case's order; no ladder_grid.s2p when no plane pair of the via gets a cavity)"""
    import tempfile
    p = c["meta"]["params"]
    d = os.path.join(REF, n)
    tmp = tempfile.TemporaryDirectory(prefix="kisi_dump_")
    dump = tmp.name  # every via of the net is dumped; only this one is kept
    pcb = os.path.join(ROOT, "examples", p["board"])
    cfg = os.path.join(ROOT, "web", "samples", {"VNA.kicad_pcb": "vna", "Thunderscope_Rev5.3.kicad_pcb": "ts"}[p["board"]] + ".kisi.json")
    for extra in ({}, {"KISI_VIA_CAVITY": "1"}):
        env = dict(os.environ, KISI_VIA_3DLOSS=str(c.get("f_loss") or c["fmax"] / 2), KISI_DUMP_VIA=dump, **extra)
        subprocess.run([tool(a, "kisi"), "net", pcb, p["net"], "--config", cfg], capture_output=True, text=True, env=env, check=True)
    names = ["F.Cu"] + [f"In{k}.Cu" for k in range(1, len(c["meta"]["copper_z"]) - 1)] + ["B.Cu"]
    li, lo = names.index(p["l_in"]), names.index(p["l_out"])
    x, y = p["via_at"]
    for tag, suf in (("inf", ""), ("grid", "_cav")):
        for (a_, b_, swap) in ((li, lo, False), (lo, li, True)):
            q = os.path.join(dump, f"via_{x:.4f}_{y:.4f}_{a_}{b_}{suf}.s2p")
            if os.path.exists(q):
                f, S = sparam.read_touchstone(q)
                if swap:
                    S = S[:, ::-1, ::-1]
                sparam.write_touchstone(os.path.join(d, f"ladder_{tag}.s2p"), f, S, [f"{n}: kisi's ladder as in the channel ({'with the cavities' if suf else 'infinite plates'}), 3D loss model"])
                break
    tmp.cleanup()


def cmd_model(a):
    out = {}
    for n in selected(a):
        if n.startswith("thru"):
            continue
        c = json.load(open(os.path.join(CASES, n + ".json")))
        if c["meta"].get("group") == "board":
            board_ladders(a, n, c)
            print(n, "board via: ladders from kisi's channel", flush=True)
            continue
        cmd = [tool(a, "validate"), "cavity-model", os.path.join(CASES, n + ".json"), os.path.join(REF, n)]
        r = cap_radius(a, n)
        if r:
            cmd += ["--cap-r", f"{r:.4f}"]
        res = json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout)
        out[n] = res["timing"]
        print(n, {k: round(v.get("ladder_ms", v.get("ms", 0))) for k, v in res["timing"].items()}, flush=True)
    os.makedirs(RES, exist_ok=True)
    p = os.path.join(RES, "board_resonances_model_timing.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(out)
    json.dump(old, open(p, "w"), indent=1)


# --- comparison ----------------------------------------------------------------------------
def thru_on(f, name):
    """2×thru on the case's frequency grid"""
    ft, T = sparam.read_touchstone(os.path.join(REF, name, "kisi.s2p"))
    if len(ft) == len(f) and np.allclose(ft, f):
        return T
    out = np.empty((len(f), 2, 2), complex)
    for i in range(2):
        for j in range(2):
            out[:, i, j] = np.interp(f, ft, T[:, i, j].real) + 1j * np.interp(f, ft, T[:, i, j].imag)
    return out


def embed(S, F):
    """N-port S with a 2-port fixture F[i] (port 1 outside, port 2 at the DUT) on every port"""
    nf, n = S.shape[0], S.shape[1]
    out = np.empty_like(S)
    I = np.eye(n)
    for k in range(nf):
        f11, f12, f21, f22 = (np.diag([F[i][k, p, q] for i in range(n)]) for p, q in ((0, 0), (0, 1), (1, 0), (1, 1)))
        out[k] = f11 + f12 @ S[k] @ np.linalg.solve(I - f22 @ S[k], f21)
    return out


def dips(f, x, lo=1e9, hi=25e9, prom=0.5):
    """local minima of |x| (dB) at least `prom` dB below the larger of the neighbouring maxima"""
    d = sparam.db(x)
    out = []
    for i in range(1, len(f) - 1):
        if not (lo <= f[i] <= hi) or not (d[i] < d[i - 1] and d[i] <= d[i + 1]):
            continue
        l = i
        while l > 0 and d[l - 1] >= d[l]:
            l -= 1
        r = i
        while r < len(f) - 1 and d[r + 1] >= d[r]:
            r += 1
        if min(d[l], d[r]) - d[i] >= prom:
            out.append((float(f[i]), float(d[i])))
    return out


def match_dips(ref, mod, tol=0.08):
    rows = []
    for (fr, dr) in ref:
        cand = [(abs(fm / fr - 1), fm, dm) for fm, dm in mod if abs(fm / fr - 1) < tol]
        if cand:
            _, fm, dm = min(cand)
            rows.append([round(fr / 1e9, 3), round(fm / 1e9, 3), round((fm / fr - 1) * 100, 1), round(dr, 2), round(dm, 2)])
        else:
            rows.append([round(fr / 1e9, 3), None, None, round(dr, 2), None])
    return rows


def band_err(f, A, B, lo, hi):
    sel = (f >= lo) & (f <= hi)
    d = np.abs(sparam.db(A[sel]) - sparam.db(B[sel]))
    return round(float(d.max()), 2), round(float(np.sqrt(np.mean(d ** 2))), 2)


def plot_case(n, f, ref, models, out, title, kind="s21", extra=None):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    styles = {"3D": dict(color="k", lw=1.6), "infinite plates (kisi now)": dict(color="#888", lw=1.0, ls="--"),
              "cavity, magnetic walls only": dict(color="#e59866", lw=0.8), "cavity, magnetic walls only (grid)": dict(color="#e59866", lw=0.8),
              "cavity, rectangle": dict(color="#c0392b", lw=1.1), "cavity, grid": dict(color="#2471a3", lw=1.0, ls=":")}
    rows = [("|S21| (dB)", lambda S: S[:, 1, 0]), ("|S11| (dB)", lambda S: S[:, 0, 0])]
    if extra:
        rows = extra
    import textwrap
    fig, axs = plt.subplots(len(rows), 1, figsize=(9, 2.6 * len(rows) + 0.8), sharex=True)
    for ax, (lab, get) in zip(np.atleast_1d(axs), rows):
        r3 = sparam.db(get(ref))
        ax.plot(f / 1e9, r3, label="3D", **styles["3D"])
        lo = float(np.min(r3))
        for k, S in models.items():
            m = sparam.db(get(S))
            ax.plot(f / 1e9, m, label=k, **styles.get(k, {}))
            lo = min(lo, float(np.min(m)))
        # a model without the path (infinite plates: no via-to-via coupling) stays off the scale
        ax.set_ylim(bottom=max(lo, float(np.min(r3)) - 25.0) - 1.0)
        ax.set_ylabel(lab)
        ax.grid(alpha=0.3)
    np.atleast_1d(axs)[0].legend(fontsize=8, loc="lower left")
    np.atleast_1d(axs)[0].set_title("\n".join(textwrap.wrap(title, 110)), fontsize=9)
    np.atleast_1d(axs)[-1].set_xlabel("frequency (GHz)")
    np.atleast_1d(axs)[-1].set_xlim(0, 25)
    fig.tight_layout()
    fig.savefig(out, dpi=110)
    plt.close(fig)


def cmd_compare(a):
    res = {}
    pdir = os.path.join(RES, "board_resonances")
    os.makedirs(pdir, exist_ok=True)
    for n in selected(a):
        if n.startswith("thru"):
            continue
        c = json.load(open(os.path.join(CASES, n + ".json")))
        k = len(c["ports"])
        d = os.path.join(REF, n)
        if not os.path.exists(os.path.join(d, f"kisi.s{k}p")):
            continue
        f, S = sparam.read_touchstone(os.path.join(d, f"kisi.s{k}p"))
        T = [thru_on(f, t) for t in c["meta"]["thru"]]
        names = {"inf": "infinite plates (kisi now)", "rect_norad": "cavity, magnetic walls only", "grid_norad": "cavity, magnetic walls only (grid)",
                 "rect": "cavity, rectangle", "grid": "cavity, grid"}
        r = {"title": c["meta"]["title"], "cells": json.load(open(os.path.join(d, "meta.json")))["runs"]["kisi"]}
        if c["meta"].get("group") == "board":
            r["leakage_note"] = "plane pairs with a cavity: see BOARD_RESONANCES.md"
        if k == 2:
            D, zc = sparam.deembed_matched(f, S, T[0], T[1], 50.0)
            sparam.write_touchstone(os.path.join(d, "deembedded.s2p"), f, D, [f"{n}: 3D, de-embedded (matched 2x-thru)"])
            models = {}
            for tag, lab in names.items():
                p = os.path.join(d, f"ladder_{tag}.s2p")
                if os.path.exists(p):
                    models[lab] = sparam.read_touchstone(p)[1]
            dref = dips(f, D[:, 1, 0])
            r["line_z_ohm"] = [round(z, 1) for z in zc]
            r["dips_3d"] = [[round(x / 1e9, 3), round(y, 2)] for x, y in dref]
            for tag, lab in names.items():
                if lab not in models:
                    continue
                L = models[lab]
                m = {"dips": match_dips([x for x in dref if x[0] < 12e9], dips(f, L[:, 1, 0]))}
                for lo, hi in ((0.5e9, 6e9), (0.5e9, 12e9), (12e9, 20e9), (20e9, 25e9)):
                    m[f"dS21_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = band_err(f, D[:, 1, 0], L[:, 1, 0], lo, hi)
                    m[f"dS11_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = band_err(f, D[:, 0, 0], L[:, 0, 0], lo, hi)
                r[tag] = m
            plot_case(n, f, D, models, os.path.join(pdir, n + ".png"), f"{n}: {c['meta']['title']}")
        else:
            # coupled vias: kisi's 4-port embedded in the 3D fixtures (port 1 excited in 3D)
            FL, _ = sparam.fixture_2xthru(f, T[0])
            FR, _ = sparam.fixture_2xthru(f, T[1])
            Fs = [FL, FR, FL, FR]
            models = {}
            for tag, lab in names.items():
                p = os.path.join(d, f"ladder_{tag}.s4p")
                if os.path.exists(p):
                    models[lab] = embed(sparam.read_touchstone(p)[1], Fs)
                elif tag == "inf":
                    L2 = sparam.read_touchstone(os.path.join(d, "ladder_inf.s2p"))[1]
                    S4 = np.zeros((len(f), 4, 4), complex)
                    S4[:, :2, :2] = L2
                    S4[:, 2:, 2:] = L2
                    models[lab] = embed(S4, Fs)
            for tag, lab in names.items():
                if lab not in models:
                    continue
                L = models[lab]
                m = {}
                for (i, j, key) in ((1, 0, "S21"), (2, 0, "S31"), (3, 0, "S41"), (0, 0, "S11")):
                    for lo, hi in ((0.5e9, 6e9), (0.5e9, 12e9), (12e9, 25e9)):
                        if tag == "inf" and key in ("S31", "S41"):
                            continue
                        m[f"d{key}_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = band_err(f, S[:, i, j], L[:, i, j], lo, hi)
                for key, (i, j) in (("S31", (2, 0)), ("S41", (3, 0))):
                    if tag != "inf":
                        sel = f <= 12e9
                        m[f"{key}_peak_db_below_12"] = [round(float(sparam.db(S[sel, i, j]).max()), 1), round(float(sparam.db(L[sel, i, j]).max()), 1)]
                        m[f"{key}_mean_db_1_12"] = [round(float(np.mean(sparam.db(S[(f > 1e9) & sel, i, j]))), 1), round(float(np.mean(sparam.db(L[(f > 1e9) & sel, i, j]))), 1)]
                r[tag] = m
            r["dips_3d_S21_raw"] = [[round(x / 1e9, 3), round(y, 2)] for x, y in dips(f, S[:, 1, 0])]
            rows = [("|S21| A (dB)", lambda X: X[:, 1, 0]), ("|S31| A→B in (dB)", lambda X: X[:, 2, 0]), ("|S41| A→B out (dB)", lambda X: X[:, 3, 0])]
            plot_case(n, f, S, models, os.path.join(pdir, n + ".png"), f"{n}: {c['meta']['title']} (at the ports, model in the 3D fixtures)", extra=rows)
        res[n] = r
        print(n, json.dumps({t: r[t].get("dips", r[t]) if t in r else None for t in ("rect", "grid")})[:600], flush=True)
    p = os.path.join(RES, "board_resonances.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(res)
    json.dump(old, open(p, "w"), indent=1)


def cmd_block2(a):
    """the two single board vias of Block 2 (BOARD_VIAS.md, de-embedded there with the matched
    2×thru): kisi's ladder from the channel with infinite plates and with the cavities against the
    3D result, to 25 GHz (the ladder dump's range) → results/board_resonances.json"""
    res = {}
    pdir = os.path.join(RES, "board_resonances")
    os.makedirs(pdir, exist_ok=True)
    for n in ("BV_ts_refinout2", "BV_vna_rf_hblb1"):
        if a.only and not any(k in n for k in a.only.split(",")):
            continue
        c = json.load(open(os.path.join(HERE, "board_cases", n + ".json")))
        board_ladders(a, n, c)
        d = os.path.join(REF, n)
        f3, D = sparam.read_touchstone(os.path.join(d, "deembedded.s2p"))
        models = {}
        for tag, lab in (("inf", "infinite plates (kisi now)"), ("grid", "cavity, grid")):
            p_ = os.path.join(d, f"ladder_{tag}.s2p")
            if not os.path.exists(p_):
                continue
            fl, L = sparam.read_touchstone(p_)
            sel = f3 <= fl[-1] + 1.0
            Li = np.empty((sel.sum(), 2, 2), complex)
            for i in range(2):
                for j in range(2):
                    Li[:, i, j] = np.interp(f3[sel], fl, L[:, i, j].real) + 1j * np.interp(f3[sel], fl, L[:, i, j].imag)
            models[lab] = Li
        sel = f3 <= 25e9 + 1.0
        f, Dr = f3[sel], D[sel]
        r = {"title": c["meta"]["title"] + " (Block 2 reference)", "cells": json.load(open(os.path.join(d, "meta.json")))["runs"]["kisi"]}
        for tag, lab in (("inf", "infinite plates (kisi now)"), ("grid", "cavity, grid")):
            if lab not in models:
                continue
            L = models[lab]
            m = {}
            for lo, hi in ((0.5e9, 6e9), (0.5e9, 12e9), (12e9, 20e9), (20e9, 25e9)):
                m[f"dS21_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = band_err(f, Dr[:, 1, 0], L[:, 1, 0], lo, hi)
                m[f"dS11_max_rms_db_{lo / 1e9:g}_{hi / 1e9:g}"] = band_err(f, Dr[:, 0, 0], L[:, 0, 0], lo, hi)
            r[tag] = m
        if len(models) == 2:
            A, B = models.values()
            r["inf_vs_grid_dS21_max_db_to_25"] = round(float(np.max(np.abs(sparam.db(A[:, 1, 0]) - sparam.db(B[:, 1, 0])))), 3)
        plot_case(n, f, Dr, models, os.path.join(pdir, n + ".png"), f"{n}: {r['title']}")
        res[n] = r
        print(n, json.dumps({t: r.get(t) for t in ("inf", "grid")}), r.get("inf_vs_grid_dS21_max_db_to_25"), flush=True)
    p = os.path.join(RES, "board_resonances.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(res)
    json.dump(old, open(p, "w"), indent=1)


def cmd_summary(a):
    """table of the comparison (results/board_resonances.json)"""
    r = json.load(open(os.path.join(RES, "board_resonances.json")))
    for n, d in sorted(r.items()):
        print(f"\n{n}: {d['title']}")
        for tag in ("inf", "rect_norad", "grid_norad", "rect", "grid"):
            if tag not in d:
                continue
            m = d[tag]
            bands = [k for k in m if k.startswith("dS21_max_rms")]
            line = "  ".join(f"{k.replace('dS21_max_rms_db_', '')}: {m[k][0]:.2f}/{m[k][1]:.2f}" for k in bands)
            dd = m.get("dips")
            if dd:
                ok = [x for x in dd if x[1] is not None]
                ferr = np.mean([abs(x[2]) for x in ok]) if ok else float("nan")
                derr = np.mean([abs(x[4] - x[3]) for x in ok]) if ok else float("nan")
                line += f"  | dips found {len(ok)}/{len(dd)}, |Δf| {ferr:.1f} %, |Δdepth| {derr:.2f} dB"
            for key in ("S31_peak_db_below_12", "S31_mean_db_1_12"):
                if key in m:
                    line += f"  {key} 3D/model {m[key]}"
            print(f"   {tag:11s} {line}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["cases", "mesh", "run", "model", "compare", "block2", "summary"])
    ap.add_argument("--only", default="")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--fdtd")
    ap.add_argument("--validate")
    ap.add_argument("--kisi")
    a = ap.parse_args()
    {"cases": cmd_cases, "mesh": cmd_mesh, "run": cmd_run, "model": cmd_model, "compare": cmd_compare, "block2": cmd_block2, "summary": cmd_summary}[a.cmd](a)


if __name__ == "__main__":
    main()
