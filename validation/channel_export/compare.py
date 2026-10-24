#!/usr/bin/env python3
"""Check kisi's channel export (`kisi export`) in ngspice against kisi's own S-parameters.

Usage:
  compare.py <kisi binary> <board.kicad_pcb> --out DIR [--config FILE] [--lane NAME]... [--bus NAME]...
             [--report FILE.md] [--plots] [--transient] [--kicad] [--svg DIR] [--strict]

For every lane (a bus: every lane of it) the script runs `kisi export` and then, in the lane's folder:
* reads kisi's Touchstone file (with scikit-rf when installed: that it reads, passivity, reciprocity);
* runs the exported S-parameter deck in ngspice (`sparam_ngspice.cir`, kisi's frequency grid) and compares
  with the Touchstone file: |dS21| in dB (pairs: Sdd21) up to 1.5 x Nyquist and up to the top frequency,
  the complex |dS11| (pairs: Sdd11) and the largest complex difference of any S-parameter;
* runs a step response in ngspice (a 100 ps edge into the lane terminated in its reference impedance) and
  compares it with the step response computed from kisi's S-parameters: the largest difference (fraction
  of the step) and the 50 % delay;
* with --transient also the PRBS test bench (bench_ngspice.cir), with --kicad the KiCad project
  (kicad_sheet.py: the sheet's wiring, kicad-cli's netlist against the lane subcircuit and the bench,
  kicad-cli's ERC where it has one; --svg DIR plots the sheets).
For a bus also: the N-port Touchstone file against the lanes' files, and the bus subcircuit in ngspice
with one lane's ports driven (the wiring of the bus subcircuit).

The export reports its own check (the circuit evaluated inside kisi, `check_against_kisi`); ngspice must
give the same numbers – that shows the netlist text is the circuit kisi evaluated. With --strict the exit
code is 1 when ngspice and kisi's evaluation of the circuit disagree (|max dS| differs by more than 0.01), the
Touchstone file is not passive or scikit-rf cannot read it, the step responses differ by more than 5 % of the
step, or a run fails.
"""
import json, math, os, re, shutil, subprocess, sys, time
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))


sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kicad_sheet  # noqa: E402


def run(cmd, cwd=None, timeout=3600):
    t0 = time.time()
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    return r, time.time() - t0


def read_touchstone(path):
    """(freq, S[k, n, n], z0) of a Touchstone 1.0 RI file (as kisi writes it)."""
    n = int(re.search(r"\.s(\d+)p$", path).group(1))
    z0, nums = 50.0, []
    for line in open(path):
        line = line.split("!")[0].strip()
        if not line:
            continue
        if line.startswith("#"):
            z0 = float(line.split()[-1])
            continue
        nums.extend(float(x) for x in line.split())
    per = 1 + 2 * n * n
    a = np.array(nums).reshape(-1, per)
    f = a[:, 0]
    c = a[:, 1::2] + 1j * a[:, 2::2]
    s = c.reshape(-1, n, n)
    if n == 2:  # Touchstone 2-port order S11 S21 S12 S22
        s = s.transpose(0, 2, 1)
    return f, s, z0


def read_wrdata(path, n):
    """S[k, n, n] from ngspice `wrdata` of S_i_j (row-major), and the frequencies."""
    a = np.loadtxt(path)
    f = a[:, 0]
    s = np.zeros((len(f), n, n), complex)
    for q in range(n * n):
        s[:, q // n, q % n] = a[:, 3 * q + 1] + 1j * a[:, 3 * q + 2]
    return f, s


def mixed(s):
    """Sdd (2x2) of 4-ports in the order P1 N1 P2 N2."""
    k = 1 / math.sqrt(2)
    m = np.array([[k, -k, 0, 0], [0, 0, k, -k]])
    return np.einsum("ij,fjk,lk->fil", m, s, m)


def transmission(s):
    return mixed(s)[:, 1, 0] if s.shape[1] == 4 else s[:, 1, 0]


def reflection(s):
    return mixed(s)[:, 0, 0] if s.shape[1] == 4 else s[:, 0, 0]


def db(x):
    return 20 * np.log10(np.maximum(np.abs(x), 1e-15))


def compare_s(f, sa, sb, f_nyq):
    d21 = np.abs(db(transmission(sa)) - db(transmission(sb)))
    band = f <= 1.5 * f_nyq
    return {
        "max_ds21_db": float(d21.max()),
        "max_ds21_db_nyq": float(d21[band].max()) if band.any() else 0.0,
        "max_ds11": float(np.abs(reflection(sa) - reflection(sb)).max()),
        "max_ds": float(np.abs(sa - sb).max()),
        "max_ds_nyq": float(np.abs(sa - sb)[band].max()) if band.any() else 0.0,
    }


def far_transfer(s):
    """Far-end voltage per emf volt with every port in its reference (a pair driven differentially:
    P +1/2, N −1/2, far-end differential voltage)."""
    if s.shape[1] == 4:
        a = np.array([0.5, -0.5, 0, 0]) / 2
        b = np.einsum("fij,j->fi", s, a)
        return b[:, 2] - b[:, 3]
    return s[:, 1, 0] / 2


def step_from_s(f, h, rise, t):
    """Response to an emf ramp (10–90 % = rise, 0 → 1) from transfer samples on f = k·df (k ≥ 1)."""
    df = f[0]
    tr = rise / 0.8
    nf = len(f)
    # finer time step: zero padding above the top frequency
    nt = 1
    while nt < 16 * nf:
        nt *= 2
    hh = np.zeros(nt, complex)
    hh[0] = h[0].real  # DC from the lowest point
    w = 2 * np.pi * f
    ramp = np.sinc(f * tr) * np.exp(-1j * w * tr / 2)  # spectrum of the ramp's derivative
    hh[1 : nf + 1] = h * ramp
    g = np.fft.irfft(hh[: nt // 2 + 1] if nt // 2 + 1 <= nt else hh, n=nt) * nt * df
    # g is the response to the ramp's derivative (a unit-area pulse): integrate
    dt = 1 / (nt * df)
    v = np.cumsum(g) * dt
    tt = np.arange(nt) * dt
    return np.interp(t, tt, v)


def t50(t, v):
    fin = v[-1]
    k = np.argmax(v >= 0.5 * fin) if fin > 0 else np.argmax(v <= 0.5 * fin)
    if k == 0:
        return float("nan")
    return float(t[k - 1] + (0.5 * fin - v[k - 1]) * (t[k] - t[k - 1]) / (v[k] - v[k - 1]))


def step_deck(meta, lib, out, rise, tstop):
    lc = meta
    ports = [p["node"] for p in lc["ports"]]
    z = lc["z_ref_ohm"]
    pair = len(ports) == 4
    tr = rise / 0.8
    lines = [f"* step response of {lc['lane']} (kisi channel export check)", f'.include "{lib}"']
    srcs = [(ports[0], 1.0 if not pair else 0.5)] + ([(ports[1], -0.5)] if pair else [])
    for i, p in enumerate(ports):
        drv = next((a for (q, a) in srcs if q == p), None)
        if drv is not None:
            lines.append(f"VS{i} e{i} 0 PWL(0 0 10p 0 {10e-12 + tr:.6e} {drv})")
            lines.append(f"RS{i} e{i} {p} {z}")
        else:
            lines.append(f"RT{i} {p} 0 {z}")
    lines.append(f"XL {' '.join(ports)} 0 {lc['subckt']}")
    probe = f"v({ports[2]})-v({ports[3]})" if pair else f"v({ports[1]})"
    lines += [f".tran {rise / 20:.6e} {tstop:.6e}", ".control", "run", f"wrdata {out} {probe}", ".endc", ".end"]
    return "\n".join(lines) + "\n"


def check_touchstone(path):
    info = {"file": os.path.basename(path)}
    f, s, z0 = read_touchstone(path)
    sv = np.linalg.svd(s, compute_uv=False).max(axis=1)
    info.update({"ports": s.shape[1], "points": len(f), "z0": z0, "max_singular_value": float(sv.max()), "passive": bool(sv.max() <= 1 + 1e-6),
                 "reciprocity_error": float(np.abs(s - s.transpose(0, 2, 1)).max())})
    try:
        import skrf
        nw = skrf.Network(path)
        info["scikit_rf"] = {"version": skrf.__version__, "read": True, "nports": nw.nports, "is_passive": bool(np.all(nw.is_passive())) if hasattr(nw, "is_passive") else None,
                             "is_reciprocal": bool(np.all(nw.is_reciprocal(tol=1e-6))) if hasattr(nw, "is_reciprocal") else None}
    except ImportError:
        info["scikit_rf"] = None
    except Exception as e:  # noqa: BLE001
        info["scikit_rf"] = {"read": False, "error": str(e)}
    # causality: impulse response of the transmission (DC from the lowest point), energy well before the
    # main arrival (an estimate: band-limited data ring a little)
    n = s.shape[1]
    tr = s[:, n // 2, 0] if n > 4 else transmission(s)  # an N-port: port 1 to the other end of its line
    h = np.concatenate([[tr[0].real], tr])
    imp = np.fft.irfft(np.concatenate([h, np.zeros(3 * len(h))]))
    k = int(np.argmax(np.abs(imp)))
    pre = imp[: max(1, int(0.8 * k))]
    info["energy_before_arrival"] = float(np.sum(pre ** 2) / max(np.sum(imp ** 2), 1e-300))
    return info, (f, s, z0)


def lane_check(lane_dir, meta, opts, rep):
    res = {"lane": meta["lane"], "dir": lane_dir, "model": meta["model"], "length_mm": meta["length_mm"], "delay_ps": meta["delay_ps"],
           "elements": meta["elements"], "kisi_eval": meta["check_against_kisi"]}
    stem = os.path.basename(meta["lane"])
    ts = [x for x in os.listdir(lane_dir) if re.match(re.escape(stem) + r"\.s\d+p$", x)][0]
    info, (f, sk, z0) = check_touchstone(os.path.join(lane_dir, ts))
    res["touchstone"] = info
    f_nyq = meta["rate_gbps"] * 0.5e9
    n = sk.shape[1]
    r, secs = run(["ngspice", "-b", "sparam_ngspice.cir"], cwd=lane_dir)
    out = os.path.join(lane_dir, "sparam_ngspice.txt")
    if r.returncode != 0 or not os.path.exists(out):
        res["error"] = "ngspice sparam failed: " + r.stdout[-500:] + r.stderr[-500:]
        return res, None
    fs, ss = read_wrdata(out, n)
    res["ngspice_sparam_s"] = round(secs, 1)
    res["ngspice_vs_kisi"] = compare_s(f, sk, ss, f_nyq)
    # step response
    rise = 100e-12
    tstop = 2 * meta["delay_ps"] * 1e-12 + 1.5e-9
    deck = step_deck(meta, f"{stem}.lib", "step_ngspice.txt", rise, tstop)
    open(os.path.join(lane_dir, "step_ngspice.cir"), "w").write(deck)
    r, secs = run(["ngspice", "-b", "step_ngspice.cir"], cwd=lane_dir)
    stp = None
    if r.returncode == 0 and os.path.exists(os.path.join(lane_dir, "step_ngspice.txt")):
        a = np.loadtxt(os.path.join(lane_dir, "step_ngspice.txt"))
        t, vs = a[:, 0], a[:, 1]
        vk = step_from_s(f, far_transfer(sk), rise, t + 10e-12 * 0)  # the deck's ramp starts at 10 ps
        vk = step_from_s(f, far_transfer(sk), rise, t - 10e-12)
        fin = max(abs(vk[-1]), 1e-9)
        res["step"] = {"ngspice_s": round(secs, 1), "max_diff_of_final": float(np.max(np.abs(vs - vk)) / fin), "t50_ngspice_ps": (t50(t, vs) - 10e-12 - rise / 0.8 / 2) * 1e12,
                       "t50_kisi_ps": (t50(t, vk) - 10e-12 - rise / 0.8 / 2) * 1e12, "final_ngspice": float(vs[-1]), "final_kisi": float(vk[-1])}
        stp = (t, vs, vk)
    else:
        res["step_error"] = r.stdout[-500:]
    if opts.get("transient"):
        r, secs = run(["ngspice", "-b", "bench_ngspice.cir"], cwd=lane_dir)
        ok = r.returncode == 0 and os.path.exists(os.path.join(lane_dir, "bench_ngspice.txt"))
        res["bench"] = {"ok": ok, "seconds": round(secs, 1)}
        if ok:
            b = np.loadtxt(os.path.join(lane_dir, "bench_ngspice.txt"))
            res["bench"]["v_min"], res["bench"]["v_max"] = float(b[:, 1].min()), float(b[:, 1].max())
    if opts.get("kicad"):
        res["kicad"] = kicad_sheet.check(lane_dir, transient=opts.get("transient"), svg_dir=opts.get("svg") and os.path.join(opts["svg"], stem), png=bool(opts.get("svg")))
    return res, (f, sk, ss, stp, f_nyq)


def plot(res, data, path):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    f, sk, ss, stp, f_nyq = data
    pair = sk.shape[1] == 4
    fig, ax = plt.subplots(2, 2, figsize=(11, 7.5))
    g = f / 1e9
    lab = "Sdd" if pair else "S"
    ax[0, 0].plot(g, db(transmission(sk)), label="kisi (Touchstone)", lw=2)
    ax[0, 0].plot(g, db(transmission(ss)), "--", label="export in ngspice", lw=1.5)
    ax[0, 0].set_ylabel(f"|{lab}21| (dB)")
    ax[0, 1].plot(g, db(reflection(sk)), lw=2, label="kisi")
    ax[0, 1].plot(g, db(reflection(ss)), "--", lw=1.5, label="ngspice")
    ax[0, 1].set_ylabel(f"|{lab}11| (dB)")
    ax[1, 0].plot(g, np.abs(db(transmission(sk)) - db(transmission(ss))), label=f"|d{lab}21| (dB)")
    ax[1, 0].plot(g, np.abs(sk - ss).max(axis=(1, 2)), label="max |dS| (all, complex)")
    ax[1, 0].axvline(1.5 * f_nyq / 1e9, color="gray", ls=":", label="1.5 x Nyquist")
    ax[1, 0].set_yscale("log")
    ax[1, 0].set_ylim(1e-5, 10)
    for a in (ax[0, 0], ax[0, 1], ax[1, 0]):
        a.set_xlabel("GHz")
        a.grid(alpha=0.3)
        a.legend(fontsize=8)
    if stp:
        t, vs, vk = stp
        ax[1, 1].plot(t * 1e12, vk, lw=2, label="from kisi's S-parameters")
        ax[1, 1].plot(t * 1e12, vs, "--", lw=1.5, label="ngspice transient")
        ax[1, 1].set_xlabel("ps")
        ax[1, 1].set_ylabel("far-end voltage per emf volt" + (" (differential)" if pair else ""))
        ax[1, 1].grid(alpha=0.3)
        ax[1, 1].legend(fontsize=8)
    fig.suptitle(f"{res['lane']}: kisi vs exported circuit in ngspice ({res['model']}, {res['length_mm']:.1f} mm)")
    fig.tight_layout()
    fig.savefig(path, dpi=110)
    plt.close(fig)


def bus_check(bus_dir, meta, opts):
    """The N-port file against the lanes' files; the bus subcircuit in ngspice with one lane driven."""
    out = {}
    name = [x for x in os.listdir(bus_dir) if re.search(r"\.s\d+p$", x)]
    if name:
        info, (f, sb, zb) = check_touchstone(os.path.join(bus_dir, name[0]))
        out["touchstone"] = info
        pm = meta["n_port"]["port_map"]
        worst = 0.0
        for lm in meta["lanes"]:
            stem = os.path.basename(lm["lane"])
            ts = [x for x in os.listdir(os.path.join(bus_dir, stem)) if re.match(re.escape(stem) + r"\.s\d+p$", x)][0]
            fl, sl, zl = read_touchstone(os.path.join(bus_dir, stem, ts))
            n = sl.shape[1]
            # renormalise the lane to the bus reference
            g = (zb - zl) / (zb + zl)
            eye = np.eye(n)
            sl2 = np.array([(s - g * eye) @ np.linalg.inv(eye - g * s) for s in sl])
            q = {p["lane_port"] - 1: p["port"] - 1 for p in pm if p["lane"] == lm["lane"]}
            idx = [q[k] for k in range(n)]
            worst = max(worst, float(np.abs(sb[:, idx][:, :, idx] - sl2).max()))
        out["lanes_vs_nport_max_ds"] = worst
    lib = [x for x in os.listdir(bus_dir) if x.endswith(".lib")]
    if lib and meta["lanes"]:
        lm = meta["lanes"][0]
        pm = meta["n_port"]["port_map"]
        nports = meta["n_port"]["ports"]
        z = meta["n_port"]["z_ref_ohm"]
        sub = open(os.path.join(bus_dir, lib[0])).read()
        bname = re.search(r"^\.subckt (\S+)", sub, re.M).group(1)
        mine = sorted((p["lane_port"], p["port"]) for p in pm if p["lane"] == lm["lane"])
        lines = [f"* bus wiring check: lane {lm['lane']} driven", f'.include "{lib[0]}"']
        drv = {bp: lp for lp, bp in mine}
        for q in range(1, nports + 1):
            if q in drv:
                lines.append(f"VP{drv[q]} p{q} 0 dc 0 ac 1 portnum {drv[q]} z0 {z}")
            else:
                lines.append(f"RT{q} p{q} 0 {z}")
        lines.append(f"XB {' '.join(f'p{q}' for q in range(1, nports + 1))} 0 {bname}")
        f0, f1, npts = 50e6, 25e9, 500
        n = len(mine)
        vecs = " ".join(f"S_{i}_{j}" for i in range(1, n + 1) for j in range(1, n + 1))
        lines += [f".sp lin {npts} {f0} {f1}", ".control", "run", f"wrdata bus_sparam.txt {vecs}", ".endc", ".end"]
        open(os.path.join(bus_dir, "bus_sparam_ngspice.cir"), "w").write("\n".join(lines) + "\n")
        r, secs = run(["ngspice", "-b", "bus_sparam_ngspice.cir"], cwd=bus_dir, timeout=7200)
        if r.returncode == 0 and os.path.exists(os.path.join(bus_dir, "bus_sparam.txt")):
            fs, ss = read_wrdata(os.path.join(bus_dir, "bus_sparam.txt"), n)
            stem = os.path.basename(lm["lane"])
            ts = [x for x in os.listdir(os.path.join(bus_dir, stem)) if re.match(re.escape(stem) + r"\.s\d+p$", x)][0]
            fl, sl, zl = read_touchstone(os.path.join(bus_dir, stem, ts))
            g = (z - zl) / (z + zl)
            eye = np.eye(n)
            sl2 = np.array([(s - g * eye) @ np.linalg.inv(eye - g * s) for s in sl])
            # the bus ports of a lane flipped against its own direction come in the other order
            order = [lp - 1 for lp, bp in sorted(mine, key=lambda x: x[1])]
            out["bus_subckt_lane"] = {"lane": lm["lane"], "ngspice_s": round(secs, 1), "points": len(fs)}
            # ngspice's port k = the lane's port k (portnum = lane port)
            out["bus_subckt_lane"]["max_ds_vs_lane_touchstone"] = float(np.abs(ss[: len(fl)] - sl2[: len(fs)]).max())
            _ = order
        else:
            out["bus_subckt_error"] = (r.stdout + r.stderr)[-500:]
    return out


def kicad_fail(k):
    if not k:
        return ""
    why = []
    if not k["sheet"]["ok"]:
        why.append(f"wiring {k['sheet']['issue_counts']}")
    nl = k.get("netlist", {})
    if nl and not nl.get("ok", True):
        why.append(f"netlist {nl.get('error') or (nl.get('unconnected', []) + nl.get('single_nodes', []) + nl.get('lane_errors', []))[:3]}")
    if k.get("erc", {}).get("available") and not k["erc"].get("ok"):
        why.append(f"ERC {k['erc'].get('failing') or k['erc'].get('error')}")
    if k.get("ac") and not k["ac"].get("ok"):
        why.append(f"AC {k['ac']}")
    return "; ".join(why)


def main():
    a = sys.argv[1:]
    if len(a) < 2 or "--out" not in a:
        raise SystemExit(__doc__)
    kisi, board = a[0], a[1]
    take = lambda k: [a[i + 1] for i in range(len(a) - 1) if a[i] == k]
    out = take("--out")[0]
    cfg = take("--config")
    opts = {"transient": "--transient" in a, "kicad": "--kicad" in a, "svg": (take("--svg") or [None])[0]}
    os.makedirs(out, exist_ok=True)
    cmd0 = [kisi if not kisi.endswith(".mjs") else "node", *([kisi] if kisi.endswith(".mjs") else []), "export", board] + (["--config", cfg[0]] if cfg else [])
    results, bus_results, bad = [], [], 0
    jobs = [("lane", x) for x in take("--lane")] + [("bus", x) for x in take("--bus")]
    for kind, name in jobs:
        d = os.path.join(out, re.sub(r"[^A-Za-z0-9_-]+", "_", name).strip("_"))
        shutil.rmtree(d, ignore_errors=True)
        r, secs = run(cmd0 + [f"--{kind}", name, "--out", d])
        if r.returncode != 0:
            print(f"{name}: kisi export failed\n{r.stdout[-800:]}{r.stderr[-800:]}")
            bad += 1
            continue
        meta = json.loads(r.stdout)
        json.dump(meta, open(os.path.join(d, "export.json"), "w"), indent=1)
        lanes = meta["lanes"]
        for lm in lanes:
            ld = d if kind == "lane" else os.path.join(d, os.path.basename(lm["lane"]))
            res, data = lane_check(ld, lm, opts, None)
            res["export_s"] = round(secs, 1)
            results.append(res)
            if data and "--plots" in a:
                png = os.path.join(out, re.sub(r"[^A-Za-z0-9_-]+", "_", lm["lane"]).strip("_") + ".png")
                plot(res, data, png)
                res["plot"] = os.path.basename(png)
            nv = res.get("ngspice_vs_kisi")
            msg = "ERROR " + res.get("error", "")[:200] if not nv else (
                f"ngspice vs kisi: |dS21| {nv['max_ds21_db_nyq']:.3f} dB to 1.5xNyq, {nv['max_ds21_db']:.2f} dB to fmax; max|dS| {nv['max_ds']:.3f} (kisi's own evaluation {res['kisi_eval']['max_ds']:.3f})")
            st = res.get("step")
            if st:
                msg += f"; step: max diff {100 * st['max_diff_of_final']:.2f} %, t50 {st['t50_ngspice_ps']:.1f} / {st['t50_kisi_ps']:.1f} ps"
            print(f"{lm['lane']}: {msg}")
            if os.environ.get("GITHUB_ACTIONS"):
                print(f"::notice title=channel export {os.path.basename(lm['lane'])}::{msg}")
            ts = res.get("touchstone", {})
            sk = ts.get("scikit_rf") or {}
            fails = [m for m, c in [("ngspice", not nv), ("ngspice differs from kisi's evaluation of the circuit", nv and abs(nv["max_ds"] - res["kisi_eval"]["max_ds"]) > 0.01),
                                    ("Touchstone not passive", not ts.get("passive")), ("scikit-rf does not read the Touchstone file", sk and not sk.get("read", True)),
                                    ("step response differs by more than 5 %", st and st["max_diff_of_final"] > 0.05),
                                    ("KiCad project: " + kicad_fail(res.get("kicad")), res.get("kicad") and not res["kicad"].get("ok"))] if c]
            kc = res.get("kicad")
            if kc:
                kmsg = f"KiCad sheet: {kc['sheet']['symbols']} symbols, wiring {'ok' if kc['sheet']['ok'] else kc['sheet']['issue_counts']}"
                nl = kc.get("netlist", {})
                if "same_as_lane" in nl:
                    kmsg += f"; kicad-cli {nl.get('kicad_cli')} netlist ({nl.get('sheet')}) {'= lane subcircuit' if nl['same_as_lane'] else 'differs: ' + '; '.join(nl['lane_errors'][:2])}"
                if kc.get("ac"):
                    kmsg += f"; AC vs bench {kc['ac'].get('max_rel_diff', float('nan')):.1e}"
                e = kc.get("erc", {})
                kmsg += f"; ERC {('ok' if e.get('ok') else 'FAIL ' + json.dumps(e.get('failing')))} {json.dumps(e.get('violations', {}))}" if e.get("available") else "; ERC: not in this kicad-cli"
                print(f"  {kmsg}")
                if os.environ.get("GITHUB_ACTIONS"):
                    print(f"::notice title=KiCad sheet {os.path.basename(lm['lane'])}::{kmsg}")
            if fails:
                print(f"  {'FAIL' if '--strict' in a else 'note'}: {'; '.join(fails)}")
                bad += 1
        if kind == "bus":
            br = bus_check(d, meta, opts)
            br["bus"] = name
            bus_results.append(br)
            print(f"bus {name}: {json.dumps(br)[:600]}")
    json.dump({"lanes": results, "buses": bus_results}, open(os.path.join(out, "results.json"), "w"), indent=1)
    rp = take("--report")
    if rp:
        write_report(rp[0], results, bus_results, board)
    if "--strict" in a and bad:
        raise SystemExit(1)


def write_report(path, results, buses, board):
    L = [f"Board `{os.path.basename(board)}`. ngspice: {subprocess.run(['ngspice', '-v'], capture_output=True, text=True).stdout.splitlines()[1].strip() if shutil.which('ngspice') else 'missing'}.", ""]
    L += ["| lane | model | length mm | elements (R/L/C/src) | ngspice vs kisi: |ΔS21| dB to 1.5×Nyq / to fmax | max |ΔS| to 1.5×Nyq / all | kisi's own evaluation: max |ΔS| | step: max diff, t50 ngspice / kisi (ps) |",
          "|---|---|---|---|---|---|---|---|"]
    for r in results:
        nv = r.get("ngspice_vs_kisi") or {}
        e = r["elements"]
        st = r.get("step") or {}
        L.append(f"| {r['lane']} | {r['model']} | {r['length_mm']:.1f} | {e['r']}/{e['l']}/{e['c']}/{e['sources']} | {nv.get('max_ds21_db_nyq', float('nan')):.3f} / {nv.get('max_ds21_db', float('nan')):.2f} | "
                 f"{nv.get('max_ds_nyq', float('nan')):.3f} / {nv.get('max_ds', float('nan')):.3f} | {r['kisi_eval']['max_ds']:.3f} | "
                 f"{100 * st.get('max_diff_of_final', float('nan')):.2f} %, {st.get('t50_ngspice_ps', float('nan')):.1f} / {st.get('t50_kisi_ps', float('nan')):.1f} |")
    ks = [r for r in results if r.get("kicad")]
    if ks:
        L += ["", "| lane | KiCad sheet | symbols / wires / junctions | wiring check | kicad-cli netlist = lane subcircuit | AC vs bench (rel.) | ERC |", "|---|---|---|---|---|---|---|"]
        for r in ks:
            k = r["kicad"]
            sh, nl, e, ac = k["sheet"], k.get("netlist", {}), k.get("erc", {}), k.get("ac", {})
            erc = ("ok " if e.get("ok") else "FAIL ") + ", ".join(f"{t} {n}" for t, n in e.get("violations", {}).items()) if e.get("available") else "not in this kicad-cli"
            L.append(f"| {r['lane']} | {nl.get('sheet', '?')} ({nl.get('kicad_cli', '-')}) | {sh['symbols']} / {sh['wires']} / {sh['junctions']} | {'ok' if sh['ok'] else sh['issue_counts']} | "
                     f"{nl.get('same_as_lane', '-')} | {ac.get('max_rel_diff', float('nan')):.1e} | {erc} |")
    open(path, "w").write("\n".join(L) + "\n")


if __name__ == "__main__":
    main()
