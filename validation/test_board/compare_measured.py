#!/usr/bin/env python3
"""Compare measured coupons with kisi and kisi-fdtd (validation/test_board/README.md, "Comparison").

Every source is treated the same way: the coupon is de-embedded with the 2×thru coupons of the same
source (IEEE P370-style NZC 2×thru of validation/fdtd/sparam.py for 2-ports; for pairs a first-order
shift per port), then the metrics are computed and compared with the measurement.

  measured   MEASURED_DIR/<COUPON>.s2p | .s4p   (coupon ids from boards/<board>.manifest.json;
             2-port: 1 = left, 2 = right connector; 4-port: 1 = P/A left, 2 = N/V left, 3 = P/A right,
             4 = N/V right - the connector order of the manifest; linear sweep with f_k = k·Δf)
  kisi       `kisi net <board> <net> --s2p/--s4p` (formula impedances; kisi.mjs: field solver)
  kisi-fdtd  FDTD_REF/<case>/kisi.sNp from run_refs.py on fdtd/<board>/ (windows of the coupons)

Metrics (validation/via_references.md section 6): via TDR min / max (25 ps, 80 ps window around the
reference plane), |S21| up to 20 GHz (or 0.8 × the first notch), stub notch frequency and depth,
pairs Sdd21 / Scd21 / Zdiff, lines εeff, loss per mm and Zc (line–line), crosstalk NEXT / FEXT
(35 ps step), cavity resonance frequencies (|S21| peaks).

usage: python3 compare_measured.py boards/<board>.manifest.json MEASURED_DIR [--fdtd-ref DIR]
                                   [--kisi KISI | --kisi-mjs KISI.MJS] [--out compare.json] [--save-dir DIR]
"""
import argparse, json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "validation", "fdtd"))
import sparam  # noqa: E402



class Source:
    """S-parameters of the coupons from one source"""

    def __init__(self, name, loader):
        self.name, self.loader, self.cache = name, loader, {}

    def get(self, cid, window=None):
        k = (cid, window)
        if k not in self.cache:
            try:
                self.cache[k] = self.loader(cid, window)
            except Exception as e:  # noqa: BLE001 - a missing file just skips that comparison
                self.cache[k] = None
                self.cache[("err",) + k] = str(e)
        return self.cache[k]


def uniform(f, S):
    """keep the part of the sweep that is a uniform grid f_k = k·Δf (P370 needs it)"""
    df = f[1] - f[0]
    ok = np.abs(f / df - np.round(f / df)) < 1e-3
    return f[ok], S[ok]


def fixture(src, thru_id, window=None):
    t = src.get(thru_id, window)
    if t is None:
        return None
    f, T = t
    F, _ = sparam.fixture_2xthru(f, T)
    return f, F


def two_port_of(S4, a, b):
    """2-port (ports a, b) of a 4-port"""
    idx = [a, b]
    return S4[:, idx][:, :, idx]


def via_metrics(f, D, rise=25e-12):
    t, z = sparam.tdr(f, D[:, 0, 0], rise=rise, tmax=80e-12)
    d21 = sparam.db(D[:, 1, 0])
    k = int(np.argmin(d21))
    notch = (round(f[k] / 1e9, 2), round(float(d21[k]), 1)) if d21[k] < -10 and 0 < k < len(f) - 1 else None
    at = lambda g: round(float(np.interp(g, f, d21)), 3) if g <= f[-1] else None
    ph = np.unwrap(np.angle(D[:, 1, 0]))
    m = (f >= 1e9) & (f <= 10e9)
    delay = -np.polyfit(2 * np.pi * f[m], ph[m], 1)[0] * 1e12 if m.sum() > 3 else None
    return {"tdr_min": round(float(z.min()), 2), "tdr_max": round(float(z.max()), 2), "s21_db": {g / 1e9: at(g) for g in (5e9, 10e9, 15e9, 20e9, 25e9, 30e9, 40e9)},
            "notch_ghz_db": notch, "delay_ps": round(float(delay), 2) if delay is not None else None, "f_max_ghz": round(f[-1] / 1e9, 2)}


def ds21(fm, Dm, fo, Do, fmax):
    """max | |S21|dB(other) - |S21|dB(measured) | on the measured grid up to fmax"""
    m = (fm <= min(fmax, fo[-1])) & (fm >= fo[0])
    if m.sum() < 3:
        return None
    o = np.interp(fm[m], fo, sparam.db(Do[:, 1, 0]))
    return round(float(np.max(np.abs(o - sparam.db(Dm[m, 1, 0])))), 3)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("manifest")
    ap.add_argument("measured")
    ap.add_argument("--fdtd-ref", default=os.path.join(HERE, "fdtd_ref"))
    ap.add_argument("--kisi", default=os.path.join(REPO, "target", "release", "kisi"))
    ap.add_argument("--kisi-mjs", default=None)
    ap.add_argument("--out", default=None)
    ap.add_argument("--save-dir", default=None, help="write the de-embedded Touchstone files here")
    a = ap.parse_args()
    man = json.load(open(a.manifest))
    board = os.path.join(os.path.dirname(os.path.abspath(a.manifest)), man["file"])
    by_id = {c["id"]: c for c in man["coupons"]}
    tmp = tempfile.mkdtemp(prefix="cmp_")
    kcmd = ["node", a.kisi_mjs] if a.kisi_mjs else [a.kisi]

    def load_meas(cid, window):
        c = by_id[cid]
        n = 4 if len(c["connectors"]) == 4 else 2
        return uniform(*sparam.read_touchstone(os.path.join(a.measured, f"{cid}.s{n}p")))

    def load_kisi(cid, window):
        c = by_id[cid]
        net = [x for x in c["nets"] if not x.endswith("_N")][0]
        four = net.startswith("DP_") or cid.startswith("XT")
        if cid.startswith("XT"):
            raise ValueError("crosstalk: kisi reports NEXT / FEXT in `net`, no 4-port")
        p = os.path.join(tmp, f"{cid}.s{4 if four else 2}p")
        r = subprocess.run(kcmd + ["net", board, net, "--s4p" if four else "--s2p", p], capture_output=True, text=True, timeout=1800)
        if r.returncode:
            raise RuntimeError(r.stderr[:300])
        load_kisi.json[cid] = json.loads(r.stdout)
        f, S = sparam.read_touchstone(p)
        # kisi numbers the ports from the start of its path; reorder to the manifest's connectors
        first = [ln for ln in open(p) if ln.startswith("! Port 1")]
        left = c["connectors"][0]["ref"]
        if first and f" {left}." not in first[0]:
            perm = [1, 0] if S.shape[1] == 2 else [2, 3, 0, 1]
            S = S[:, perm][:, :, perm]
        return f, S
    load_kisi.json = {}

    def load_fdtd(cid, window):
        c = by_id[cid]
        cases = [x for x in c["fdtd"] if window is None or x.endswith("__" + window)]
        if not cases:
            raise FileNotFoundError(f"no 3D window {window} for {cid}")
        d = os.path.join(a.fdtd_ref, cases[0])
        n = [x for x in os.listdir(d) if x.startswith("kisi.s")][0]
        return sparam.read_touchstone(os.path.join(d, n))

    srcs = {"measured": Source("measured", load_meas), "kisi": Source("kisi", load_kisi), "fdtd": Source("kisi-fdtd", load_fdtd)}
    thru_of = {}  # feed layer -> 2×thru coupon
    for c in man["coupons"]:
        if c["group"] == "thru" and len(c["connectors"]) == 2:
            thru_of[c["connectors"][0]["layer"]] = c["id"]
    pthru_of = {c["connectors"][0]["layer"]: c["id"] for c in man["coupons"] if c["group"] == "thru" and len(c["connectors"]) == 4}
    res = {"board": man["board"], "sources": {"kisi": " ".join(os.path.basename(x) for x in kcmd), "fdtd_ref": a.fdtd_ref}, "coupons": {}}

    def save(cid, sname, f, D):
        if a.save_dir:
            os.makedirs(a.save_dir, exist_ok=True)
            sparam.write_touchstone(os.path.join(a.save_dir, f"{cid}.{sname}.deembedded.s{D.shape[1]}p"), f, D, [f"{cid} de-embedded ({sname})"])

    for c in man["coupons"]:
        cid, g = c["id"], c["group"]
        out = {}
        conn = c["connectors"]
        if g in ("A", "B", "D") or cid.startswith("VLV"):
            # single via (or via - line - via): P370 2×thru of both feed layers
            lt, rt = thru_of.get(conn[0]["layer"]), thru_of.get(conn[1]["layer"])
            win = "w18" if "w18" in " ".join(c["fdtd"]) else "w10"
            per = {}
            for sname, src in srcs.items():
                s = src.get(cid, None if sname != "fdtd" else c["fdtd"][0].rsplit("__", 1)[1])
                fl = fixture(src, lt, "w10" if sname == "fdtd" else None)
                fr = fixture(src, rt, "w10" if sname == "fdtd" else None)
                if s is None or fl is None or fr is None:
                    continue
                f, S = s
                n = min(len(f), len(fl[0]), len(fr[0]))
                D = sparam.deembed2(S[:n], fl[1][:n], fr[1][:n])
                per[sname] = (f[:n], D)
                save(cid, sname, f[:n], D)
                out[sname] = via_metrics(f[:n], D)
            if "measured" in per:
                fm, Dm = per["measured"]
                mm = out["measured"]
                flim = 20e9 if not mm["notch_ghz_db"] else min(20e9, 0.8 * mm["notch_ghz_db"][0] * 1e9)
                for sname in ("kisi", "fdtd"):
                    if sname in per:
                        fo, Do = per[sname]
                        o = out[sname]
                        o["vs_measured"] = {"dS21_max_db": ds21(fm, Dm, fo, Do, flim), "f_limit_ghz": round(flim / 1e9, 1),
                                            "tdr_min_err_pct": round((o["tdr_min"] / mm["tdr_min"] - 1) * 100, 1),
                                            "tdr_max_err_pct": round((o["tdr_max"] / mm["tdr_max"] - 1) * 100, 1),
                                            "notch_err_pct": round((o["notch_ghz_db"][0] / mm["notch_ghz_db"][0] - 1) * 100, 1) if o.get("notch_ghz_db") and mm["notch_ghz_db"] else None}
            if win == "w18":
                out["note"] = "the 3D comparison uses window w10 here; w18 (with THRU w18) is the wider variant"
        elif cid.startswith(("LINE_", "SL_")) or cid == "CPWG_50":
            # line: de-embed with the 2×thru (or line–line against the short structure of the set)
            short = {"LINE": thru_of.get(conn[0]["layer"]), "CPWG": "CPWG_25", "SL": "VLV_5"}[cid.split("_")[0]]
            dl = c["length_mm"] - by_id[short]["length_mm"] if short in by_id else None
            for sname, src in srcs.items():
                if sname == "fdtd":
                    lw = [w for w in ("w20", "w10") if any(x.endswith("__" + w) for x in c["fdtd"])]
                    if len(lw) == 2:
                        s20, s10 = src.get(cid, "w20"), src.get(cid, "w10")
                        if s20 and s10:
                            tl = sparam.line_params(s20[0], s20[1], s10[1], 20.0, 10.0)
                            out[sname] = summarize_line(s20[0], tl, "3D line–line 10 / 20 mm")
                    continue
                s, sh = src.get(cid), src.get(short) if short else None
                if s is None or sh is None or dl is None:
                    continue
                n = min(len(s[0]), len(sh[0]))
                tl = sparam.line_params(s[0][:n], s[1][:n], sh[1][:n], c["length_mm"], by_id[short]["length_mm"])
                out[sname] = summarize_line(s[0][:n], tl, f"line–line {c['length_mm']:g} / {by_id[short]['length_mm']:g} mm (ΔL {dl:g} mm)")
                if not cid.startswith("SL"):  # Zc from the ABCD matrix of the line alone (2×thru de-embedded)
                    fx = fixture(src, short)
                    if fx is not None:
                        D = sparam.deembed2(s[1][:n], fx[1][:n], fx[1][:n])
                        zc = sparam.line_params(s[0][:n], D, None, dl, 1.0)["zc"]
                        out[sname]["zc_5ghz"] = round(float(np.interp(5e9, s[0][:n], zc)), 3)
                        out[sname]["zc_note"] = f"Zc from the ABCD matrix of the {dl:g} mm line after the 2×thru de-embedding"
                fT = fixture(src, thru_of.get(conn[0]["layer"])) if cid.startswith("SL") else None
                if fT is not None:  # stripline plateau in the TDR, after the L1 feed
                    D = sparam.deembed2(s[1][:n], fT[1][:n], fT[1][:n])
                    t, z = sparam.tdr(s[0][:n], D[:, 0, 0], rise=35e-12, tmax=600e-12)
                    out[sname]["tdr_plateau_ohm"] = round(float(np.median(z[(t > 80e-12) & (t < 2 * c["params"]["stripline_mm"] * 7.0e-12 - 40e-12)])), 2)
            if "measured" in out:
                for sname in ("kisi", "fdtd"):
                    if sname in out:
                        out[sname]["vs_measured"] = {k: (round((out[sname][k] / out["measured"][k] - 1) * 100, 2) if out[sname].get(k) and out["measured"].get(k) else None)
                                                     for k in ("eeff_5ghz", "zc_5ghz", "loss_db_per_mm_10ghz")}
        elif g in ("E",):
            lt, rt = pthru_of.get(conn[0]["layer"]), pthru_of.get(conn[2]["layer"])
            for sname, src in srcs.items():
                w = "w6" if sname == "fdtd" else None
                s, tl_, tr_ = src.get(cid, w), src.get(lt, w), src.get(rt, w)
                if s is None or tl_ is None or tr_ is None:
                    continue
                f, S = s
                n = min(len(f), len(tl_[0]), len(tr_[0]))
                # first-order shift per port: the half 2×thru transmission of the same line
                fx = [sparam.fixture_shift(f[:n], two_port_of(tl_[1][:n], 0, 2)), sparam.fixture_shift(f[:n], two_port_of(tl_[1][:n], 1, 3)),
                      sparam.fixture_shift(f[:n], two_port_of(tr_[1][:n], 0, 2)), sparam.fixture_shift(f[:n], two_port_of(tr_[1][:n], 1, 3))]
                D = sparam.deembed_shift(S[:n], fx)
                save(cid, sname, f[:n], D)
                mm = sparam.mixed_mode(D)
                t, z = sparam.tdr(f[:n], mm["dd"][:, 0, 0], rise=25e-12, tmax=80e-12, z0=100.0)
                at = lambda x, gg: round(float(np.interp(gg, f[:n], sparam.db(x))), 3) if gg <= f[n - 1] else None
                out[sname] = {"zdiff_min": round(float(z.min()), 2), "zdiff_max": round(float(z.max()), 2),
                              "sdd21_db": {gg / 1e9: at(mm["dd"][:, 1, 0], gg) for gg in (5e9, 10e9, 20e9, 30e9)},
                              "scd21_max_db_20ghz": round(float(np.max(sparam.db(mm["cd"][f[:n] <= 20e9, 1, 0]))), 1)}
            if "measured" in out:
                for sname in ("kisi", "fdtd"):
                    if sname in out:
                        out[sname]["vs_measured"] = {"zdiff_min_err_pct": round((out[sname]["zdiff_min"] / out["measured"]["zdiff_min"] - 1) * 100, 1),
                                                     "dSdd21_10ghz_db": round(out[sname]["sdd21_db"][10.0] - out["measured"]["sdd21_db"][10.0], 3)
                                                     if out[sname]["sdd21_db"].get(10.0) is not None and out["measured"]["sdd21_db"].get(10.0) is not None else None}
        elif cid.startswith(("XT", "BS")):
            for sname, src in srcs.items():
                if sname == "kisi":
                    continue
                s = src.get(cid, "wc" if sname == "fdtd" else None)
                if s is None:
                    continue
                f, S = s
                nx = sparam.step_response(f, S[:, 1, 0], 35e-12, tmax=2e-9)[1]
                fx = sparam.step_response(f, S[:, 3, 0], 35e-12, tmax=2e-9)[1]
                out[sname] = {"next_pct": round(100 * float(np.max(np.abs(nx))), 2), "fext_pct": round(100 * float(np.max(np.abs(fx))), 2),
                              "note": "measured: whole coupon incl. fan-out; 3D: the straight coupled section"}
            kj = (load_kisi_json(kcmd, board, c["nets"][0], tmp) or {}).get("crosstalk") or {}
            ag = (kj.get("aggressors") or [{}])[0]
            out["kisi"] = {"next_pct": round(100 * ag.get("next", 0.0), 2) if ag else None, "fext_pct": round(100 * ag.get("fext", 0.0), 2) if ag else None,
                           "broadside_listed": len(kj.get("broadside") or []), "rise_ps": kj.get("rise_ps")}
        elif g == "G":
            for sname, src in srcs.items():
                if sname == "kisi":
                    continue
                s = src.get(cid, "w60" if sname == "fdtd" else None)
                if s is None:
                    continue
                f, S = s
                d = sparam.db(S[:, 1, 0])
                pk = [k for k in range(1, len(f) - 1) if d[k] > d[k - 1] and d[k] >= d[k + 1] and d[k] > np.median(d) + 6 and f[k] < 6e9]
                out[sname] = {"peaks_ghz": [round(f[k] / 1e9, 3) for k in pk][:10]}
            out["analytic_modes_ghz"] = c["params"].get("modes_GHz")
        if out:
            res["coupons"][cid] = out
    res["missing"] = sorted({f"{src.name}: {k[1]}{'/' + k[2] if k[2] else ''}: {v[:160]}" for src in srcs.values() for k, v in src.cache.items() if k[0] == "err"})
    js = json.dumps(res, indent=1)
    if a.out:
        open(a.out, "w").write(js)
    print(js)


def load_kisi_json(kcmd, board, net, tmp):
    r = subprocess.run(kcmd + ["net", board, net], capture_output=True, text=True, timeout=1800)
    return json.loads(r.stdout) if r.returncode == 0 else None


def summarize_line(f, tl, how):
    pick = lambda arr, g: round(float(np.interp(g, f, arr)), 4)
    return {"method": how, "eeff_5ghz": pick(tl["eeff"], 5e9), "eeff_20ghz": pick(tl["eeff"], 20e9) if f[-1] >= 20e9 else None,
            "zc_5ghz": pick(tl["zc"], 5e9), "loss_db_per_mm_10ghz": pick(tl["loss_db_per_mm"], 10e9),
            "loss_db_per_mm_20ghz": pick(tl["loss_db_per_mm"], 20e9) if f[-1] >= 20e9 else None,
            "zc_note": "Zc from the ABCD matrix of the longer structure: with launches or vias in it, use the TDR plateau instead"}


if __name__ == "__main__":
    main()
