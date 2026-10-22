#!/usr/bin/env python3
"""Compare measured coupons with kisi and kisi-fdtd (validation/test_board/README.md, "Comparison").

Every source is treated the same way: the coupon is de-embedded with the 2×thru coupon of the same
board and source (IEEE P370-style NZC 2×thru of validation/fdtd/sparam.py for 2-ports; for pairs a
first-order shift per port), then the metrics are computed and compared with the measurement.

  measured   MEASURED_DIR/<COUPON>.s2p | .s4p   (coupon ids from boards/<board>.manifest.json;
             2-port: 1 = left, 2 = right connector (H coupons: 1 = upper, 2 = lower connector);
             4-port: 1 = P/A left, 2 = N/V left, 3 = P/A right, 4 = N/V right - the connector order
             of the manifest; linear sweep with f_k = k·Δf)
             PDN and capacitor coupons (H, CAPZ): MEASURED_DIR/<COUPON>.<variant>.s2p, one file per
             populate variant (open, 0R, 100n, 1u, 10u, ...); MEASURED_DIR/populate.json maps a
             variant to the part and its datasheet values as a kisi PI_CAP value, e.g.
             {"100n": {"part": "GRM155R71C104KA88", "pi_cap": "c=100n esr=25m esl=0.35n"}}
  kisi       `kisi net <board> <net> --s2p/--s4p` (formula impedances; kisi.mjs: field solver);
             `kisi power <board> --pdn VDD_<COUPON>` with the variant's PI_CAP (H coupons);
             data/components.json (ESL by package, ESR model, mount_nh) for the CAPZ coupons, which
             kisi's `net` does not model (a shunt part on a signal net)
  kisi-fdtd  FDTD_REF/<case>/kisi.sNp from run_refs.py on fdtd/<board>/ (windows of the coupons)

Metrics (validation/via_references.md section 6): via TDR min / max (25 ps, 80 ps window around the
reference plane), |S21| up to 20 GHz (or 0.8 × the first notch), stub notch frequency and depth,
pairs Sdd21 / Scd21 / Zdiff, lines εeff, loss per mm and Zc (line–line), crosstalk NEXT / FEXT
(35 ps step), Zdiff plateau of coupled pairs, cavity resonance frequencies (|S21| peaks), shunt-thru
impedance Z = 25·S21 / (1 − S21): C, SRF, ESR, ESL / loop inductance, |Z| at 1 / 10 / 100 MHz / 1 GHz.

usage: python3 compare_measured.py boards/<board>.manifest.json MEASURED_DIR [--fdtd-ref DIR]
                                   [--kisi KISI | --kisi-mjs KISI.MJS] [--out compare.json] [--save-dir DIR]
"""
import argparse, glob, json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(REPO, "validation", "fdtd"))
import sparam  # noqa: E402

COMP = json.load(open(os.path.join(REPO, "data", "components.json")))
PARTS = json.load(open(os.path.join(HERE, "connectors.json")))["parts"]
# generic populate variants (README, component list); populate.json in the measurement folder overrides them
VARIANTS = {"open": {"part": "not populated", "pi_cap": None},
            "0R": {"part": "0 Ω jumper", "pi_cap": "c=1 esr=5m", "note": "modelled as a huge C with 5 mΩ: |Z| = loop inductance and ESL"},
            "100n": {"part": "100 nF X7R 16 V", "pi_cap": "c=100n"}, "1u": {"part": "1 µF X7R/X5R", "pi_cap": "c=1u"},
            "10u": {"part": "10 µF X5R", "pi_cap": "c=10u"}}


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
    """keep the part of the sweep that is a uniform grid f_k = k·Δf (P370 needs it); a sweep that is
    not one (a log sweep of the PDN coupons) is returned as it is"""
    df = f[1] - f[0]
    ok = np.abs(f / df - np.round(f / df)) < 1e-3
    return (f[ok], S[ok]) if ok.mean() > 0.9 else (f, S)


def fixture(src, thru_id, window=None):
    if thru_id is None:
        return None
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


def shunt_z(S):
    """impedance of a shunt element between two 50 Ω ports from S21 (2-port shunt-thru)"""
    s21 = S[:, 1, 0]
    return 25.0 * s21 / (1.0 - s21)


def z_metrics(f, Z, kind):
    """C, SRF, ESR, ESL (capacitor) or L, R (0 Ω / loop) from a shunt-thru impedance"""
    w = 2 * np.pi * f
    out = {"z_ohm": {lab: round(float(np.interp(g, f, np.abs(Z))), 5) for lab, g in (("1M", 1e6), ("10M", 1e7), ("100M", 1e8), ("1G", 1e9)) if f[0] <= g <= f[-1]}}
    if kind == "short":  # 0 Ω: R + jωL
        m = (f >= 10e6) & (f <= 100e6)
        if m.sum() > 3:
            out["l_nh"] = round(float(np.median(Z[m].imag / w[m])) * 1e9, 4)
            out["r_mohm_10mhz"] = round(float(np.interp(10e6, f, Z.real)) * 1e3, 3)
        return out
    if kind == "open":
        m = f <= 50e6
        if m.sum() > 3:  # pad / plane capacitance from the low-frequency reactance
            out["c_pf"] = round(float(np.median(-1 / (w[m] * Z[m].imag))) * 1e12, 2)
        return out
    a = np.abs(Z)
    k = int(np.argmin(a))
    if 0 < k < len(f) - 1:
        out["srf_mhz"] = round(f[k] / 1e6, 3)
        out["esr_mohm"] = round(float(Z[k].real) * 1e3, 3)
        lo = (f <= f[k] / 5)
        hi = (f >= 3 * f[k]) & (f <= min(10 * f[k], 300e6))
        if lo.sum() > 3:
            out["c_nf"] = round(float(np.median(-1 / (w[lo] * Z[lo].imag))) * 1e9, 3)
        if hi.sum() > 3:
            c = out.get("c_nf", 0) * 1e-9
            xl = Z[hi].imag + (1 / (w[hi] * c) if c else 0)
            out["esl_total_nh"] = round(float(np.median(xl / w[hi])) * 1e9, 4)
    return out


def model_cap_z(f, c, pkg, kisi_pkg):
    """the capacitor as kisi's data describe it (data/components.json): C (no DC bias: the VNA
    measurement is unbiased), ESR model, body ESL of the package kisi derives from the pad pitch,
    plus mount_nh"""
    dec = COMP["decoupling"]
    e = dec["esr"]
    esr = min(max(e["k_ohm"] * (e["c_ref_f"] / c) ** e["exp"], e["min_ohm"]), e["max_ohm"]) if c < 1e-3 else 0.005
    esl = dec["esl_nh"][kisi_pkg] * 1e-9
    lm = dec["mount_nh"]["value"] * 1e-9
    w = 2 * np.pi * f
    Z = esr + 1j * w * (esl + lm) + (1 / (1j * w * c) if c < 1e-3 else 0)
    return Z, {"esr_mohm": round(esr * 1e3, 2), "esl_nh": round(esl * 1e9, 3), "mount_nh": round(lm * 1e9, 3),
               "package": pkg, "package_kisi": kisi_pkg, "c_nf": round(c * 1e9, 3) if c < 1e-3 else None,
               "srf_mhz": round(1 / (2 * np.pi * np.sqrt((esl + lm) * c)) / 1e6, 3) if c < 1e-3 else None}


def kisi_package(pitch):
    """the package kisi derives from the pad pitch (crates/kisi-pi/src/pdn_ic.rs, package_of)"""
    for lim, p in ((0.65, "0201"), (1.25, "0402"), (1.95, "0603"), (2.7, "0805"), (3.8, "1206"), (4.6, "1210")):
        if pitch < lim:
            return p
    return "large"


def cvalue(pi_cap):
    for tok in (pi_cap or "").split():
        if tok.startswith("c="):
            v = tok[2:].lower().rstrip("f")
            mult = {"p": 1e-12, "n": 1e-9, "u": 1e-6, "µ": 1e-6, "m": 1e-3}
            return float(v[:-1]) * mult[v[-1]] if v[-1] in mult else float(v)
    return None


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
    kisi_native = [a.kisi]  # `power` does not use the field solver
    pop = dict(VARIANTS)
    pj = os.path.join(a.measured, "populate.json")
    if os.path.exists(pj):
        pop.update(json.load(open(pj)))

    def load_meas(cid, window):
        c = by_id[cid]
        n = 4 if len(c["connectors"]) == 4 else 2
        name = f"{cid}.s{n}p" if window is None else f"{cid}.{window}.s{n}p"  # window = populate variant here
        return uniform(*sparam.read_touchstone(os.path.join(a.measured, name)))

    def load_kisi(cid, window):
        c = by_id[cid]
        net = [x for x in c["nets"] if not x.endswith("_N") and x != "GND"][0]
        four = len(c["connectors"]) == 4
        if c["group"] in ("H", "G", "CAP"):
            raise ValueError("PDN / capacitor coupon: no `net` S-parameters (see the PDN comparison)")
        if cid.startswith(("XT", "BS")):
            raise ValueError("crosstalk: kisi reports NEXT / FEXT in `net`, no 4-port")
        p = os.path.join(tmp, f"{cid}.s{4 if four else 2}p")
        r = subprocess.run(kcmd + ["net", board, net, "--s4p" if four else "--s2p", p], capture_output=True, text=True, timeout=1800)
        if r.returncode:
            raise RuntimeError(r.stderr[:300])
        f, S = sparam.read_touchstone(p)
        # kisi numbers the ports from the start of its path; reorder to the manifest's connectors
        first = [ln for ln in open(p) if ln.startswith("! Port 1")]
        left = c["connectors"][0]["ref"]
        if first and f" {left}." not in first[0]:
            perm = [1, 0] if S.shape[1] == 2 else [2, 3, 0, 1]
            S = S[:, perm][:, :, perm]
        return f, S

    def load_fdtd(cid, window):
        c = by_id[cid]
        cases = [x for x in c["fdtd"] if window is None or x.endswith("__" + window)]
        if not cases:
            raise FileNotFoundError(f"no 3D window {window} for {cid}")
        d = os.path.join(a.fdtd_ref, cases[0])
        n = [x for x in os.listdir(d) if x.startswith("kisi.s")][0]
        return sparam.read_touchstone(os.path.join(d, n))

    srcs = {"measured": Source("measured", load_meas), "kisi": Source("kisi", load_kisi), "fdtd": Source("kisi-fdtd", load_fdtd)}
    thru = next((c["id"] for c in man["coupons"] if c["group"] == "thru" and len(c["connectors"]) == 2), None)
    pthru = next((c["id"] for c in man["coupons"] if c["group"] == "thru" and len(c["connectors"]) == 4), None)
    res = {"board": man["board"], "sources": {"kisi": " ".join(os.path.basename(x) for x in kcmd), "fdtd_ref": a.fdtd_ref},
           "fixtures": {"2-port": thru, "4-port": pthru}, "coupons": {}}
    if thru is None:
        res["fixtures"]["note"] = "no 2×thru on this board: 2-port results are raw (connector and feed included); see README, de-embedding"

    def save(cid, sname, f, D):
        if a.save_dir:
            os.makedirs(a.save_dir, exist_ok=True)
            sparam.write_touchstone(os.path.join(a.save_dir, f"{cid}.{sname}.deembedded.s{D.shape[1]}p"), f, D, [f"{cid} de-embedded ({sname})"])

    def deembed_2port(src, sname, cid, window):
        """the coupon de-embedded with the board's 2×thru (both feeds are L1, 12.5 mm)"""
        s = src.get(cid, window)
        fx = fixture(src, thru, "w10" if sname == "fdtd" else None)
        if s is None or fx is None:
            return None
        f, S = s
        n = min(len(f), len(fx[0]))
        if not np.allclose(f[:n], fx[0][:n], rtol=1e-6):  # e.g. a log sweep: no P370 de-embedding
            return None
        return f[:n], sparam.deembed2(S[:n], fx[1][:n], fx[1][:n])

    def variants(cid):
        vs = sorted({os.path.basename(p)[len(cid) + 1:-4] for p in glob.glob(os.path.join(a.measured, f"{cid}.*.s2p"))})
        return vs or [None]

    for c in man["coupons"]:
        cid, g = c["id"], c["group"]
        out = {}
        if g in ("A", "B", "D"):
            # via - line - via fed from L1 at both ends: the board's 2×thru on both sides
            per = {}
            for sname, src in srcs.items():
                r = deembed_2port(src, sname, cid, "wv" if sname == "fdtd" else None)
                if r is None:
                    continue
                per[sname] = r
                save(cid, sname, *r)
                out[sname] = via_metrics(*r)
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
            out["note"] = "two vias 5 mm apart (D group 10 mm): the TDR separates them (about 60 / 120 ps round trip)"
        elif cid.startswith(("LINE_", "SL_")) or cid == "CPWG_50":
            # line: line–line against the shorter structure of the set (the vias / launches cancel)
            short = {"LINE": thru, "CPWG": "CPWG_25", "SL": "VLV_L3"}[cid.split("_")[0]]
            dl = c["length_mm"] - by_id[short]["length_mm"] if short in by_id else None
            for sname, src in srcs.items():
                if sname == "fdtd":
                    s20 = src.get(cid, "w20")
                    s10 = src.get(cid, "w10") if any(x.endswith("__w10") for x in c["fdtd"]) else (src.get(short, "w10") if short in by_id else None)
                    if s20 and s10:
                        n = min(len(s20[0]), len(s10[0]))
                        tl = sparam.line_params(s20[0][:n], s20[1][:n], s10[1][:n], 20.0, 10.0)
                        out[sname] = summarize_line(s20[0][:n], tl, "3D line–line 10 / 20 mm")
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
                fT = fixture(src, thru) if cid.startswith("SL") else None
                if fT is not None:  # stripline plateau in the TDR, after the L1 feed and the first via
                    D = sparam.deembed2(s[1][:n], fT[1][:n], fT[1][:n])
                    t, z = sparam.tdr(s[0][:n], D[:, 0, 0], rise=35e-12, tmax=600e-12)
                    sel = (t > 80e-12) & (t < 2 * c["params"]["line_mm"] * 7.0e-12 - 40e-12)
                    out[sname]["tdr_plateau_ohm"] = round(float(np.median(z[sel])), 2) if sel.any() else None
            if "measured" in out:
                for sname in ("kisi", "fdtd"):
                    if sname in out:
                        out[sname]["vs_measured"] = {k: (round((out[sname][k] / out["measured"][k] - 1) * 100, 2) if out[sname].get(k) and out["measured"].get(k) else None)
                                                     for k in ("eeff_5ghz", "zc_5ghz", "loss_db_per_mm_10ghz")}
        elif g == "E":
            # pair via - line - via: per-port shift with the board's pair 2×thru (15 mm per side; 3D: wv with PAIR w6)
            for sname, src in srcs.items():
                w = ("wv", "w6") if sname == "fdtd" else (None, None)
                s, t_ = src.get(cid, w[0]), src.get(pthru, w[1]) if pthru else None
                if s is None or t_ is None:
                    continue
                f, S = s
                n = min(len(f), len(t_[0]))
                fx = [sparam.fixture_shift(f[:n], two_port_of(t_[1][:n], 0, 2)), sparam.fixture_shift(f[:n], two_port_of(t_[1][:n], 1, 3)),
                      sparam.fixture_shift(f[:n], two_port_of(t_[1][:n], 0, 2)), sparam.fixture_shift(f[:n], two_port_of(t_[1][:n], 1, 3))]
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
            # time domain: the fan-out is gated out by position, no 2×thru needed
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
            kj = (load_kisi_json(kcmd, board, c["nets"][0]) or {}).get("crosstalk") or {}
            ag = (kj.get("aggressors") or [{}])[0]
            out["kisi"] = {"next_pct": round(100 * ag.get("next", 0.0), 2) if ag else None, "fext_pct": round(100 * ag.get("fext", 0.0), 2) if ag else None,
                           "broadside_listed": len(kj.get("broadside") or []), "rise_ps": kj.get("rise_ps")}
        elif cid.startswith("DIFF_"):
            # Zdiff plateau of the coupled section in the differential TDR (35 ps), time-gated by the
            # coupon's one-way delay T (the section is centred: TDR times 0.8 T … 1.2 T)
            for sname, src in srcs.items():
                s = src.get(cid, "wc" if sname == "fdtd" else None)
                if s is None:
                    continue
                f, S = s
                mm = sparam.mixed_mode(S)
                t, z = sparam.tdr(f, mm["dd"][:, 0, 0], rise=35e-12, tmax=1.5e-9, z0=100.0)
                tt, st = sparam.step_response(f, mm["dd"][:, 1, 0], 35e-12, tmax=1.5e-9)
                T = float(tt[np.argmax(st > 0.5 * st.max())])
                sel = (t > 0.8 * T) & (t < 1.2 * T) if sname != "fdtd" else (t > 0.3 * 2 * T) & (t < 0.7 * 2 * T)
                out[sname] = {"zdiff_plateau_ohm": round(float(np.median(z[sel])), 2) if sel.any() else None, "one_way_ps": round(T * 1e12, 1)}
            if "measured" in out and out["measured"]["zdiff_plateau_ohm"]:
                for sname in ("kisi", "fdtd"):
                    if out.get(sname, {}).get("zdiff_plateau_ohm"):
                        out[sname]["vs_measured_pct"] = round((out[sname]["zdiff_plateau_ohm"] / out["measured"]["zdiff_plateau_ohm"] - 1) * 100, 2)
        elif g == "G":
            # cavity: |S21| peaks (and the shunt-thru transfer impedance), analytic and 3D modes
            for sname, src in srcs.items():
                if sname == "kisi":
                    continue
                s = src.get(cid, "wg" if sname == "fdtd" else None)
                if s is None:
                    continue
                f, S = s
                d = sparam.db(S[:, 1, 0])
                pk = [k for k in range(1, len(f) - 1) if d[k] > d[k - 1] and d[k] >= d[k + 1] and d[k] > np.median(d) + 6 and f[k] < 8e9]
                Z = shunt_z(S)
                out[sname] = {"peaks_ghz": [round(f[k] / 1e9, 3) for k in pk][:10],
                              "z21_low_c_nf": round(float(np.median(-1 / (2 * np.pi * f[f < 100e6] * Z[f < 100e6].imag))) * 1e9, 4) if (f < 100e6).sum() > 3 else None}
            out["analytic_modes_ghz"] = c["params"].get("modes_GHz")
            kp = load_kisi_power(kisi_native, board, c["nets"][0])
            out["kisi"] = {"cavity_first_resonance_ghz": round(((kp or {}).get("pdn") or {}).get("cavity_first_resonance_hz", 0) / 1e9, 3) or None,
                           "c_plane_nf": round(((kp or {}).get("pdn") or {}).get("c_plane_f", 0) * 1e9, 4) or None}
        elif g == "CAP":
            # capacitor shunt-thru: the board's 2×thru, then Z = 25·S21/(1−S21) per populate variant
            pkg = c["params"]["package"]
            kp = kisi_package(c["params"]["pitch_mm"])
            for v in variants(cid):
                o = {}
                r = deembed_2port(srcs["measured"], "measured", cid, v)
                if r is None:
                    s = srcs["measured"].get(cid, v)
                    if s is not None:
                        r = s
                        o["note"] = "raw (no 2×thru)"
                if r is not None:
                    f, D = r
                    Z = shunt_z(D)
                    kind = "open" if v == "open" else ("short" if v == "0R" else "cap")
                    o["measured"] = z_metrics(f, Z, kind)
                    pc = (pop.get(v) or {}).get("pi_cap")
                    cv = cvalue(pc) if pc else None
                    if kind != "open" and cv:
                        Zk, info = model_cap_z(f, cv, pkg, kp)
                        o["kisi_data"] = dict(info, **z_metrics(f, Zk, "short" if kind == "short" else "cap"))
                        o["kisi_data"]["note"] = "data/components.json (kisi does not model a shunt part on a signal net in `net`)"
                out[v or "as_built"] = o
            s3 = srcs["fdtd"].get(cid, "w10")
            if s3 is not None:
                out["fdtd_0R"] = z_metrics(s3[0], shunt_z(s3[1]), "short")
                out["fdtd_0R"]["note"] = "3D: the footprint with a 5 mΩ short on a 0.5 mm body: mounting loop"
        elif g == "H":
            # PDN loop: Z at the pin field per populate variant, kisi `power --pdn` with that part
            rail = [n for n in c["nets"] if n.startswith("VDD_")][0]
            cref = next((p["ref"] for p in c.get("parts", []) if p.get("role") == "decap"), None)
            for v in variants(cid):
                o = {}
                r = deembed_2port(srcs["measured"], "measured", cid, v)
                if r is None:
                    s = srcs["measured"].get(cid, v)
                    if s is not None:
                        r = s
                        o["note"] = "raw: feeds not de-embedded (|Z| and the ESR at the SRF are not affected; Re Z above ~100 MHz is)"
                if r is not None:
                    f, D = r
                    kind = "open" if v == "open" else ("short" if v == "0R" else "cap")
                    o["measured"] = z_metrics(f, shunt_z(D), kind)
                pc = (pop.get(v) or {}).get("pi_cap") if v else None
                if v != "open" and cref:
                    kp = load_kisi_power(kisi_native, board, rail, cref, pc, tmp)
                    ics = (((kp or {}).get("pdn") or {}).get("per_ic") or {}).get("ics") or []
                    if ics:
                        ce = (ics[0].get("capacitors_most_effective") or [{}])[0]
                        o["kisi"] = {"z_ohm": {k: round(x, 5) for k, x in ics[0]["z_ohm"].items()}, "l_path_nh": ce.get("l_path_nh"),
                                     "l_total_nh": ce.get("l_total_nh"), "srf_mhz": ce.get("srf_mhz"), "pi_cap": pc}
                        if "measured" in o:
                            o["vs_measured_pct"] = {k: round((o["kisi"]["z_ohm"][k] / o["measured"]["z_ohm"][k] - 1) * 100, 1)
                                                    for k in o["measured"]["z_ohm"] if k in o["kisi"]["z_ohm"] and o["measured"]["z_ohm"][k]}
                out[v or "as_built"] = o
            s3 = srcs["fdtd"].get(cid, "wh")
            if s3 is not None:
                out["fdtd_0R"] = z_metrics(s3[0], shunt_z(s3[1]), "short")
                out["fdtd_0R"]["note"] = "3D: capacitor as a 5 mΩ short: the loop inductance pin field - planes - capacitor"
        if out:
            res["coupons"][cid] = out
    res["missing"] = sorted({f"{src.name}: {k[1]}{'/' + k[2] if k[2] else ''}: {v[:160]}" for src in srcs.values() for k, v in src.cache.items() if k[0] == "err"})
    js = json.dumps(res, indent=1)
    if a.out:
        open(a.out, "w").write(js)
    print(js)


def run_json(args, timeout):
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        print(f"kisi not run: {e}", file=sys.stderr)
        return None
    return json.loads(r.stdout) if r.returncode == 0 else None


def load_kisi_json(kcmd, board, net):
    return run_json(kcmd + ["net", board, net], 1800)


def load_kisi_power(kcmd, board, rail, cref=None, pi_cap=None, tmp=None):
    """`kisi power --pdn RAIL`; with a PI_CAP value for the capacitor `cref` in a temporary copy of the
    board's kisi.json (the populate variant)"""
    args = kcmd + ["power", board, "--pdn", rail]
    if cref and pi_cap:
        cfg = json.load(open(board[:-len(".kicad_pcb")] + ".kisi.json"))
        cfg["fields"] = [f for f in cfg.get("fields", []) if not (f.get("ref") == cref and f.get("name") == "PI_CAP")]
        cfg["fields"].append({"ref": cref, "name": "PI_CAP", "value": pi_cap, "source": {"kind": "datasheet", "note": "populate variant (compare_measured.py)"}})
        p = os.path.join(tmp, f"{rail}.kisi.json")
        json.dump(cfg, open(p, "w"))
        args += ["--config", p]
    return run_json(args, 600)


def summarize_line(f, tl, how):
    pick = lambda arr, g: round(float(np.interp(g, f, arr)), 4)
    return {"method": how, "eeff_5ghz": pick(tl["eeff"], 5e9), "eeff_20ghz": pick(tl["eeff"], 20e9) if f[-1] >= 20e9 else None,
            "zc_5ghz": pick(tl["zc"], 5e9), "loss_db_per_mm_10ghz": pick(tl["loss_db_per_mm"], 10e9),
            "loss_db_per_mm_20ghz": pick(tl["loss_db_per_mm"], 20e9) if f[-1] >= 20e9 else None,
            "zc_note": "Zc from the ABCD matrix of the longer structure: with launches or vias in it, use the TDR plateau instead"}


if __name__ == "__main__":
    main()
