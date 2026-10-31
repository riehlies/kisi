"""Compare CST results of the connector launch (cst_launch.py) with kisi-fdtd's board-side launch cases and
Samtec's RF characterization report (README_cst_launch.md).

  python3 compare_launch.py --cst launch1=OUT/launch1_default_jlc4_7628/cst_mid.s2p \\
      [--cst thru2=OUT/thru2_.../cst_base.s2p] [--cst bench2=...] [--cascade launch1] \\
      [--kisi default] [--kisi-file LABEL=path.s2p] [--out compare_launch.png]

--cst LABEL=FILE    a CST Touchstone file (any number); port 1 = connector (coax, mating plane)
--cascade LABEL     also the 2xthru built from a one-connector result: S cascaded with itself reversed (exact for
                    the network when the line mode alone links the halves; coupling between the two launches across
                    the board is not in it) - the thru2 / bench2 result where the full model does not fit the mesh
--kisi VARIANT      kisi-fdtd result fdtd_ref/launch_jlc4_7628/launch_jlc4_7628_VARIANT/kisi.s2p (board side only:
                    lumped port at the board edge instead of the connector, 2.5 mm of line after the launch); compare it
                    1:1 with cst_launch.py --model board_only
--tdr PS[,PS]       also the TDR (step response of S11, Gaussian edge 10-90 % = PS ps; default 10,25) in a third panel;
                    time is the round trip from port 1 (connector models: the mating plane; the board edge then lies at
                    about 64 ps, --edge-ps); --shift LABEL=PS moves one curve (e.g. a board_only result to the edge)
Samtec's numbers (report 2023-06-01, connectors.json connector.benchmark) are drawn as markers: max |S11| (VSWR)
and |S21| at 40 GHz for one connector + 500 mil and two connectors + 1 in on their MT40 fixture.
Needs numpy and matplotlib (any Python, not CST's).
"""
import argparse, json, os
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
BENCH_DEFAULT = {  # connectors.json connector.benchmark (copy for use outside the repository)
    "one_connector_500mil": {"s11_max_db": [-25.4, -23.1], "at_ghz": [34.1, 37.4], "s21_db_at_40ghz": [-1.05, -0.94]},
    "two_connectors_1in": {"s11_max_db": [-20.5, -17.2], "at_ghz": [36.1, 36.9], "s21_db_at_40ghz": [-2.1, -1.88]}}


def read_touchstone(path):
    """(f in Hz, S[k, i, j]) of a .s1p / .s2p (RI, MA or DB; Hz / kHz / MHz / GHz)"""
    n = int(path.lower().rsplit(".s", 1)[1][0])
    unit, fmt = 1.0, "MA"
    vals = []
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.split("!")[0].strip()
        if not line:
            continue
        if line.startswith("#"):
            t = line[1:].upper().split()
            unit = {"HZ": 1.0, "KHZ": 1e3, "MHZ": 1e6, "GHZ": 1e9}.get(t[0], 1e9) if t else 1e9
            fmt = next((x for x in t if x in ("RI", "MA", "DB")), "MA")
            continue
        vals += [float(x) for x in line.split()]
    per = 1 + 2 * n * n
    a = np.array(vals).reshape(-1, per)
    f = a[:, 0] * unit
    p = a[:, 1:].reshape(-1, n * n, 2)
    if fmt == "RI":
        c = p[..., 0] + 1j * p[..., 1]
    elif fmt == "MA":
        c = p[..., 0] * np.exp(1j * np.radians(p[..., 1]))
    else:
        c = 10 ** (p[..., 0] / 20) * np.exp(1j * np.radians(p[..., 1]))
    S = c.reshape(-1, n, n)
    if n == 2:  # Touchstone 2-port order: S11 S21 S12 S22
        S = S.transpose(0, 2, 1)
    return f, S


def cascade_self(S):
    """S of the network followed by itself reversed (port 2 joined to port 2)"""
    R = S[:, ::-1, ::-1]
    s11, s12, s21, s22 = S[:, 0, 0], S[:, 0, 1], S[:, 1, 0], S[:, 1, 1]
    r11, r12, r21, r22 = R[:, 0, 0], R[:, 0, 1], R[:, 1, 0], R[:, 1, 1]
    d = 1 - s22 * r11
    out = np.empty_like(S)
    out[:, 0, 0] = s11 + s12 * r11 * s21 / d
    out[:, 0, 1] = s12 * r12 / d
    out[:, 1, 0] = r21 * s21 / d
    out[:, 1, 1] = r22 + r21 * s22 * r12 / d
    return out


def step_response(f, s, rise=25e-12, tmax=None):
    """response to a unit step with a Gaussian edge (10-90 % = rise) of S sampled on f_k = k df (as
    validation/fdtd/sparam.step_response); t = 0 at the middle of the edge"""
    if f[0] == 0:
        f, s = f[1:], s[1:]
    nf = len(f)
    df = f[1] - f[0]
    sig = rise / 2.563
    t0 = 3.0 * rise
    g = np.exp(-0.5 * (2 * np.pi * f * sig) ** 2) * np.exp(-2j * np.pi * f * t0)
    spec = np.concatenate([[s[0].real], s * g])
    n = 8 * nf
    v = np.cumsum(np.fft.irfft(spec, n))
    t = np.arange(n) / (n * df) - t0
    k = (t >= 0) & ((t <= tmax) if tmax else (t < n / (2 * n * df) - t0))
    return t[k], v[k]


def tdr(f, s11, rise=25e-12, z0=50.0, tmax=None):
    t, rho = step_response(f, s11, rise, tmax)
    return t, z0 * (1 + rho) / (1 - rho)


def db(x):
    return 20 * np.log10(np.maximum(np.abs(x), 1e-12))


def vswr(s11_db):
    g = 10 ** (np.asarray(s11_db) / 20)
    return (1 + g) / (1 - g)


def summary(label, f, S, band=(0.5e9, 40e9)):
    m = (f >= band[0]) & (f <= band[1] + 1)
    s11 = db(S[m, 0, 0])
    k = int(np.argmax(s11))
    at = lambda g: db(S[np.argmin(abs(f - g * 1e9)), 0, 0]) if f[-1] >= g * 1e9 * 0.999 else float("nan")
    row = {"label": label, "s11_max_db": float(s11[k]), "at_ghz": float(f[m][k] / 1e9), "vswr_max": float(vswr(s11[k])),
           "s11_10_20_30_40": [float(at(g)) for g in (10, 20, 30, 40)]}
    if S.shape[1] == 2 and np.all(np.isfinite(S[:, 1, 0])):
        i40 = np.argmin(abs(f - 40e9))
        row["s21_db_40"] = float(db(S[i40, 1, 0])) if f[-1] >= 39.9e9 else float("nan")
    return row


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--cst", action="append", default=[], help="LABEL=FILE")
    ap.add_argument("--cascade", action="append", default=[], help="LABEL of a one-connector --cst result")
    ap.add_argument("--kisi", action="append", default=[], help="kisi-fdtd launch variant (fdtd_ref/launch_jlc4_7628/)")
    ap.add_argument("--kisi-file", action="append", default=[], help="LABEL=FILE: other kisi-fdtd result")
    ap.add_argument("--stackup", default="jlc4_7628")
    ap.add_argument("--fmax", type=float, default=40e9)
    ap.add_argument("--out", default="compare_launch.png")
    ap.add_argument("--json", default="", help="also write the summary table as JSON")
    ap.add_argument("--tdr", default="10,25", help="TDR edge(s) in ps (empty: no TDR panel)")
    ap.add_argument("--tmax", type=float, default=200.0, help="TDR time range (ps)")
    ap.add_argument("--edge-ps", type=float, default=64.0, help="round trip mating plane -> board edge (9.6 mm of air line)")
    ap.add_argument("--shift", action="append", default=[], help="LABEL=PS: shift the TDR of a curve by PS ps")
    a = ap.parse_args()
    curves = []  # (label, f, S, style)
    notes = []
    for item in a.cst:
        lab, _, p = item.partition("=")
        f, S = read_touchstone(p)
        curves.append((f"CST {lab}", f, S, "-"))
        head = "".join(line for line in open(p, encoding="utf-8", errors="replace") if line.startswith("!"))
        if "constant conductivity" in head and lab not in notes:
            notes.append(lab)
    if notes:
        print("note: " + ", ".join(notes) + ": dielectric loss as a constant conductivity (exact at fmax/2, as kisi-fdtd): a shunt "
              "conductance at every frequency, so about -20 dB |S11| and -1 dB |S21| at low frequency on 12.5 mm of FR-4 and a TDR that "
              "drifts below 50 ohm along the line - a convention for the comparison with kisi-fdtd, not the board (cst_launch.py --loss tand)")
    for lab in a.cascade:
        src = next((c for c in curves if c[0] == f"CST {lab}"), None)
        if src is None or src[2].shape[1] != 2:
            raise SystemExit(f"--cascade {lab}: no 2-port --cst result with that label")
        curves.append((f"CST {lab} cascaded (2xthru)", src[1], cascade_self(src[2]), "-."))
    kis = [(v, os.path.join(HERE, "fdtd_ref", f"launch_{a.stackup}", f"launch_{a.stackup}_{v}", "kisi.s2p")) for v in a.kisi]
    kis += [tuple(x.partition("=")[::2]) for x in a.kisi_file]
    for lab, p in kis:
        f, S = read_touchstone(p)
        curves.append((f"kisi-fdtd {lab} (board side, edge port)", f, S, "--"))
    if not curves:
        raise SystemExit("nothing to compare: --cst / --kisi")
    try:
        bench = json.load(open(os.path.join(HERE, "connectors.json"), encoding="utf-8"))["connector"]["benchmark"]
    except (OSError, KeyError, ValueError):
        bench = BENCH_DEFAULT
    rows = [summary(lab, f, S) for lab, f, S, _ in curves]
    print(f"{'result':58s} {'max|S11| 0.5-40':>16s} {'at GHz':>7s} {'VSWR':>6s}  {'|S11| 10/20/30/40 GHz':>24s} {'|S21| 40 GHz':>12s}")
    for r in rows:
        print(f"{r['label'][:58]:58s} {r['s11_max_db']:13.1f} dB {r['at_ghz']:7.1f} {r['vswr_max']:6.2f}  "
              + " / ".join(f"{v:5.1f}" for v in r["s11_10_20_30_40"]) + f" {r.get('s21_db_40', float('nan')):9.2f} dB")
    for key, txt in (("one_connector_500mil", "Samtec, 1 connector + 500 mil"), ("two_connectors_1in", "Samtec, 2 connectors + 1 in")):
        b = bench[key]
        print(f"{txt:58s} {max(b['s11_max_db']):13.1f} dB {b['at_ghz'][int(np.argmax(b['s11_max_db']))]:7.1f} "
              f"{vswr(max(b['s11_max_db'])):6.2f}  (MT40 fixture, report 2023-06-01)       {min(b['s21_db_at_40ghz']):9.2f} dB")
    if a.json:
        json.dump({"rows": rows, "samtec": bench}, open(a.json, "w", encoding="utf-8"), indent=1)
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    rises = [float(x) for x in a.tdr.split(",") if x.strip()]
    if rises:
        fig, axs = plt.subplots(3, 1, figsize=(10, 11.5))
        ax1, ax2, ax3 = axs
    else:
        fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 8), sharex=True)
    shift = {k: float(v) for k, _, v in (x.partition("=") for x in a.shift)}
    for ci, (lab, f, S, ls) in enumerate(curves):
        m = f <= a.fmax * 1.001
        col = f"C{ci % 10}"
        ax1.plot(f[m] / 1e9, db(S[m, 0, 0]), ls, color=col, lw=1.3, label=lab + " S11")
        if S.shape[1] == 2 and np.all(np.isfinite(S[m, 1, 1])) and np.any(S[m, 1, 1] != 0) and not lab.startswith("kisi"):
            ax1.plot(f[m] / 1e9, db(S[m, 1, 1]), ":", color=col, lw=0.9, label=lab + " S22")
        if S.shape[1] == 2 and np.all(np.isfinite(S[m, 1, 0])):
            ax2.plot(f[m] / 1e9, db(S[m, 1, 0]), ls, color=col, lw=1.3, label=lab)
        if rises:
            uniform = np.allclose(np.diff(f), f[1] - f[0], rtol=1e-6) and abs(f[0] - (f[1] - f[0])) < 1e-3 * f[1] or f[0] == 0
            if not uniform:
                continue
            sh = shift.get(lab.split(" ", 1)[1].split(" (")[0] if " " in lab else lab, 0.0)
            for ri, r in enumerate(rises):
                t, z = tdr(f, S[:, 0, 0], r * 1e-12, tmax=a.tmax * 1e-12)
                ax3.plot(t * 1e12 + sh, z, ["-", "--", ":"][ri % 3], color=col, lw=1.2 if ri == 0 else 0.9,
                         label=f"{lab}, {r:g} ps" + (f" (shifted {sh:+g} ps)" if sh else ""))
    if rises:
        ax3.axhline(50, color="#888", lw=0.6)
        ax3.axvline(a.edge_ps, color="#c02020", lw=0.8, ls=":")
        ax3.annotate("board edge (connector models)", (a.edge_ps, ax3.get_ylim()[1]), fontsize=7, color="#c02020", va="top", rotation=90, ha="right")
        ax3.set_xlabel("round trip from port 1 / ps")
        ax3.set_ylabel("TDR Z / ohm")
        ax3.grid(alpha=0.3)
        ax3.legend(fontsize=6, loc="lower right")
        ax2.set_xlabel("f / GHz")
    for key, mk, txt in (("one_connector_500mil", "v", "Samtec 1 conn. + 500 mil"), ("two_connectors_1in", "^", "Samtec 2 conn. + 1 in")):
        b = bench[key]
        ax1.plot(b["at_ghz"], b["s11_max_db"], mk, color="k", ms=7, ls="none", label=f"{txt}: max |S11| (2 samples)")
        ax2.plot([40, 40], b["s21_db_at_40ghz"], mk, color="k", ms=7, ls="none", label=f"{txt}: |S21| at 40 GHz")
    ax1.axhline(-20, color="#c02020", lw=0.8, ls=":", label="target one launch: -20 dB")
    ax1.set_ylabel("|S11| / dB (connector side)")
    ax1.set_ylim(-50, 0)
    ax2.set_ylabel("|S21| / dB")
    ax1.set_xlabel("f / GHz")
    ax2.set_xlabel("f / GHz")
    for ax in (ax1, ax2):
        ax.grid(alpha=0.3)
        ax.legend(fontsize=7, loc="lower left" if ax is ax1 else "lower left")
    ax1.set_title("Samtec 292-J-P-EP-ST-EL-01 launch: CST (connector modelled) vs kisi-fdtd (board side) vs Samtec's report", fontsize=10)
    fig.tight_layout()
    fig.savefig(a.out, dpi=130)
    print("plot:", a.out)


if __name__ == "__main__":
    main()
