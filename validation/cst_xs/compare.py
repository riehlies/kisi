#!/usr/bin/env python3
"""The cross-section comparison: CST, the full-wave backend (FW), kisi's FDM field solver and kisi's formulas, per
group, lossless at 1 GHz and the lossy sweep. Works with any subset of results.

    python3 validation/cst_xs/compare.py [--groups a,b] [--tier 1,2] [--ids a,b] [--quantity z|eeff|all]
        [--outlier 2] [--formula-outlier 5] [--show-text] [--no-loss] [--loss-table] [--per-case] [--sweep-table]
        [--fw-diag] [--json out.json] [--md out.md]
    python3 validation/cst_xs/compare.py --solve-loss [--jobs 2]    # kisi's line-model solves → kisi_loss.json

Sources: CST results/<id>.json (time domain port modes, PEC, tanδ 0, 1 GHz, electric and magnetic walls) and
results/<id>.fd.lossy.json (frequency domain lossy port modes, copper 5.8e7 S/m, tanδ, electric walls); FW
fullwave/<id>.json (run_fullwave.py); kisi.json (run_kisi.py: formula, field, field_E, field_M); kisi_loss.json
(--solve-loss: kisi's FDM solve with R, L, G, C at 1 GHz in CST's box, electric walls, the input of kisi's line model).

Error in % = (value / reference − 1) · 100, reference CST. Per group: n, bias (median signed error), median, 90th
percentile and max of |error| (with its case).

Definitions (one quantity, one definition in a comparison):
  Z0, Zdiff, Zeven  lossless line impedances: CST (PEC, tanδ 0), FW full-wave eigenmode (ideal conductors), FDM
                    1/(c·√(C·C0)) (static, conductor loss and internal inductance excluded), formula. Pairs: Zdiff
                    = 2 Zodd, Zeven = 2 Zcomm.
  εeff (PEC)        ideal conductors, tanδ 0: CST time domain (1 GHz), FW eigenmode (1 GHz); the static FW / FDM
                    values (C/C0) differ from these by the dispersion at 1 GHz (≤ 0.3 %, FW: "dispersion 1 GHz")
  εeff (phase)      (β/k0)² with copper and tanδ at 1 GHz, internal inductance included: CST frequency domain lossy,
                    FW sweep, FDM (kisi.json field_E eeff: how js_2d_fields reports εeff)
  loss sweep        per frequency of CST's lossy grid, electric walls: α (dB/m, total), R and G per metre from
                    γ and Zc (R = Re(γ·Zc), G = Re(γ/Zc): the conductor and the dielectric part apart), Re Zc and
                    the phase εeff. kisi = kisi's line model (crates/kisi-si/src/rf/line.rs, field path, smooth
                    copper) built from kisi_loss.json
Asymmetric pairs: both definitions use the pair's two eigenmodes. CST's line impedance is the power-current one
(Z = 2P/|I|², I = the larger conductor current of the differential mode, the sum of both for the common mode);
kisi's (FDM and FW: upstream's modal decomposition) is 1/(v_p·C_k) with C_k = ½·vᵀCv, |v|² = 2 (the ±1 V drive of
a symmetric pair). They agree for a symmetric pair and differ for an asymmetric one. FW's 2×2 L and C give the ratio:
"FW" is compared with CST in CST's definition, every other kisi value with CST transferred to kisi's definition. A
pair is asymmetric when the ratio differs from 1 by more than 0.2 %.
Box modes: above the lowest cutoff of the solver box (TE10 of the box filled with its largest εr) the electric
walls carry waveguide modes that both solvers can pick or mix with the quasi-TEM mode. A sweep point there is left
out (for every solver) when CST and FW differ by more than 3 % in Re Zc or the phase εeff, CST's line impedance is
off its lowest-frequency value by more than 2×, or FW's eigenmode εeff jumped by more than 2 % between
neighbouring points at or below it.
"""
import argparse, cmath, json, math, os, re, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)

C0 = 299792458.0
SIGMA, F_REF = 5.8e7, 1e9
DB = 20 / math.log(10)
ASYM_PCT = 0.2       # % between the two impedance definitions in FW → asymmetric pair
BOX_MISMATCH = 3.0   # % Re Zc / εeff between CST and FW above the box cutoff
JUMP_PCT = 2.0       # % FW eigenmode εeff between neighbouring sweep points
FW_NAME = {"line": "single", "differential": "odd", "common": "even"}
CST_K = {"line": 1.0, "differential": 0.5, "common": 2.0}  # CST line impedance → per-line modal (Zodd, Zeven)
ZQ = ("z0", "zdiff", "zeven")
EQ = ("eeff", "eeff_odd", "eeff_even")


# --- helpers -------------------------------------------------------------------------------------------

def load(p):
    try:
        return json.load(open(p, encoding="utf-8"))
    except (OSError, ValueError):
        return None


def fnv(t):
    h = 0xcbf29ce484222325
    for b in t.encode():
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def pct(v, p):
    v = sorted(v)
    if not v:
        return float("nan")
    i = (len(v) - 1) * p
    lo, hi = math.floor(i), math.ceil(i)
    return v[lo] + (v[hi] - v[lo]) * (i - lo)


def stats(rows):
    """rows: [(case id, err)] → dict"""
    if not rows:
        return None
    e = [x for _, x in rows]
    a = [abs(x) for x in e]
    k = max(range(len(a)), key=lambda i: a[i])
    return {"n": len(e), "bias": pct(e, 0.5), "med": pct(a, 0.5), "p90": pct(a, 0.9), "max": a[k], "max_case": rows[k][0],
            "max_signed": e[k]}


def err(x, ref, keys=None):
    return {k: (x[k] / ref[k] - 1) * 100 for k in (keys or ref) if x.get(k) is not None and ref.get(k)}


def mean(a, b):
    return {k: (a[k] + b[k]) / 2 for k in a if a.get(k) is not None and b.get(k) is not None}


# --- the values of each source -----------------------------------------------------------------------------

def cst_vals(v):
    if not v:
        return {}
    if v.get("zdiff") is not None:
        return {k: v.get(k) for k in ("zdiff", "zeven", "eeff_odd", "eeff_even")}
    return {k: v.get(k) for k in ("z0", "eeff")}


def fw_vals(r, kind="mode"):
    """FW point result → {z0 | zdiff, zeven, eeff…}; kind: mode (eigenmode, ideal conductors), static"""
    if not r or not r.get("ok"):
        return {}
    s = kind == "static"
    if r.get("zodd") is not None:
        g = (lambda k: r.get(k + "_static")) if s else r.get
        return {"zdiff": 2 * g("zodd"), "zeven": g("zeven"),
                "eeff_odd": r.get("eeff_odd_static") if s else r.get("eeff_mode_odd"),
                "eeff_even": r.get("eeff_even_static") if s else r.get("eeff_mode_even")}
    return {"z0": r.get("z0_static") if s else r.get("z0"), "eeff": r.get("eeff_static") if s else r.get("eeff_mode")}


def fdm_vals(r):
    """js_2d_fields FDM result: Z lossless (static), εeff phase (copper)"""
    if not r or not r.get("ok"):
        return {}
    if r.get("zodd") is not None:
        return {"zdiff": 2 * r["zodd"], "zeven": r.get("zeven"), "eeff_odd": r.get("eeff_odd"), "eeff_even": r.get("eeff_even")}
    return {"z0": r.get("z0"), "eeff": r.get("eeff")}


def fdm_static_eeff(r):
    """static εeff of an FDM solve with C: Z0 = 1/(c √(C C0)) → εeff = C/C0 = (c Z0 C)²"""
    if not r or not r.get("ok"):
        return {}
    out = {}
    for sfx, q in (("", "eeff"), ("_odd", "eeff_odd"), ("_even", "eeff_even")):
        z = r.get("z0" if not sfx else "z" + sfx[1:])
        c = r.get("c" + sfx)
        if z and c:
            out[q] = (C0 * z * c) ** 2
    return out


def formula_vals(f, pair):
    if not f:
        return {}
    if pair:
        return {"zdiff": f.get("zdiff"), "zeven": f.get("zeven"), "eeff_odd": f.get("eeff_odd"), "eeff_even": f.get("eeff_even")}
    return {"z0": f.get("z0"), "eeff": f.get("eeff")}


def pair_defs(r):
    """FW pair → ({zdiff, zeven}: CST's definition / kisi's, {eeff_odd, eeff_even}: eigenmode εeff) from the 2×2 L
    and C. Both definitions use the eigenmodes of L·C (voltages v, currents I = v_p·C·v, power P = vᵀI): kisi's (FDM
    and FW, upstream's classifyModalDecomposition) Z = 2/(v_p·vᵀCv) with |v|² = 2 (Zdiff = 2 Zodd), CST's
    Zdiff = P/max|I_k|², Zcomm = P/(ΣI)² (Zeven = 2 Zcomm). The ratio does not depend on v_p or on the internal
    inductance in FW's L. Symmetric and degenerate pairs (voltage vectors (1, ∓1)): ratio 1."""
    if not r or not r.get("ok") or not r.get("mat_l") or not r.get("mat_c"):
        return None
    l11, l12, l21, l22 = r["mat_l"]
    c11, c12, c21, c22 = r["mat_c"]
    a, b, c, d = l11 * c11 + l12 * c21, l11 * c12 + l12 * c22, l21 * c11 + l22 * c21, l21 * c12 + l22 * c22
    tr, det = a + d, a * d - b * c
    disc = math.sqrt(max(tr * tr / 4 - det, 0.0))
    lam = (tr / 2 + disc, tr / 2 - disc)
    if abs(lam[0] / lam[1] - 1) < 1e-6:
        vecs = [(lam[1], (1.0, -1.0)), (lam[0], (1.0, 1.0))]
    else:
        vecs = []
        for x in lam:
            v = (b, x - a)
            if math.hypot(*v) < 1e-12 * (abs(a) + abs(b) + abs(c) + abs(d)):
                v = (x - d, c)
            n = math.hypot(*v) / math.sqrt(2)
            vecs.append((x, (v[0] / n, v[1] / n)))
    ratio, ee = {}, {}
    for x, v in vecs:
        cv = (c11 * v[0] + c12 * v[1], c21 * v[0] + c22 * v[1])
        q = v[0] * cv[0] + v[1] * cv[1]
        if v[0] * v[1] < 0:
            ratio["zdiff"] = q * q / (4 * max(abs(cv[0]), abs(cv[1])) ** 2)
            ee["eeff_odd"] = x * C0 * C0
        else:
            ratio["zeven"] = q * q / (cv[0] + cv[1]) ** 2
            ee["eeff_even"] = x * C0 * C0
    return ratio, ee


# --- kisi's line model (crates/kisi-si/src/rf/line.rs, line_model_mode, field path, smooth copper) ----------

M1, M2 = 4.0, 12.0


def ds_k(f):
    """Djordjevic-Sarkar kernel at f (DjordjevicSarkar::k)"""
    s = complex(0, 2 * math.pi * f)
    w1, w2 = 2 * math.pi * 10 ** M1, 2 * math.pi * 10 ** M2
    q = (s + w2) / (s + w1)
    return complex(math.log(abs(q)), cmath.phase(q)) / (math.log(10) * (M2 - M1))


def conductor_z(r_dc, k_ac, f):
    """smooth part of conductor_z: R_dc·√(1 + jω/ω_c)"""
    if k_ac > 0 and r_dc > 0:
        wc = math.pi * r_dc * r_dc / (k_ac * k_ac)
        return r_dc * cmath.sqrt(1 + complex(0, 2 * math.pi * f) / wc)
    return complex(r_dc, 0)


def k_ac_for(r_dc, r0, f0):
    rho = max(r0 / r_dc, 1 + 1e-9)
    x = math.sqrt((2 * rho * rho - 1) ** 2 - 1)
    return r_dc / math.sqrt(2 * (f0 / x))


class LineModel:
    """kisi's LineModel from a field solve's R, L, G, C at f0: the conductor R_dc·√(1 + jω/ω_c) through R(f0), its
    internal inductance taken out of L(f0); the dielectric A + B·k(f) (Djordjevic-Sarkar) through G/(ωC) at f0; the
    microstrip Lorentz term is left out (it changes ε′ only)"""

    def __init__(self, r0, l0, g0, c0f, r_dc, f0=F_REF):
        w0 = 2 * math.pi * f0
        self.r_dc = r_dc
        self.k_ac = k_ac_for(r_dc, r0, f0) if r0 > r_dc else 0.0
        l_int0 = conductor_z(r_dc, self.k_ac, f0).imag / w0
        self.l_ext = max(l0 - l_int0, 0.5 * l0)
        self.c0 = c0f
        self.eeff0 = self.l_ext * c0f * C0 * C0
        t0 = max(g0 / (w0 * c0f), 0.0) if g0 is not None else 0.0
        k0 = ds_k(F_REF)
        self.b = -self.eeff0 * t0 / k0.imag if k0.imag < 0 else 0.0
        self.a = self.eeff0 - self.b * k0.real

    def at(self, f):
        w = 2 * math.pi * f
        zc = conductor_z(self.r_dc, self.k_ac, f)
        e = self.a + self.b * ds_k(f)
        c_air = self.c0 / self.eeff0
        r, l = zc.real, self.l_ext + zc.imag / w
        g, c = -w * c_air * e.imag, c_air * e.real
        gam = cmath.sqrt(complex(r, w * l) * complex(g, w * c))
        z = cmath.sqrt(complex(r, w * l) / complex(g, w * c))
        return {"alpha_db": gam.real * DB, "r": r, "g": g, "zc_re": z.real, "eeff": (gam.imag / (w / C0)) ** 2}


def sig_rect(text):
    for ln in text.splitlines():
        m = re.match(r"sig\+ (?:rect )?x=(\S+) w=(\S+) y=(\S+) h=(\S+)", ln)
        if m:
            return abs(float(m.group(2))), float(m.group(4))
    return None


def kisi_models(k, text):
    """{fw mode name: LineModel} from a kisi_loss.json solve"""
    wt = sig_rect(text)
    if not k or not k.get("ok") or not wt:
        return {}
    r_dc = 1 / (SIGMA * wt[0] * 1e-3 * wt[1] * 1e-3)
    out = {}
    for name, s in (("single", ""), ("odd", "_odd"), ("even", "_even")):
        if all(k.get(x + s) for x in ("r", "l", "c")):
            out[name] = LineModel(k["r" + s], k["l" + s], k.get("g" + s), k["c" + s], r_dc)
    return out


def solve_loss(cases, res_dir, cache_path, jobs, chunk=15):
    """kisi's FDM solve (tools/solve_xs.mjs, tol 0.005) of each case in CST's box with electric walls (the field_E
    geometry, ground ties included) → kisi_loss.json"""
    import xsgeo
    cache = load(cache_path) or {"about": "kisi's FDM solve (R, L, G, C at 1 GHz) in CST's box, electric walls: "
                                          "compare.py --solve-loss", "results": {}}
    todo = []
    for c in cases:
        fd = load(os.path.join(res_dir, c["id"] + ".fd.lossy.json"))
        if fd and fd.get("status") == "ok" and c["id"] not in cache["results"]:
            todo.append((c["id"], xsgeo.boxed_text(c["text"], fd["box"], "gnd")))
    print(f"{len(todo)} field solves", file=sys.stderr)
    tmp = tempfile.mkdtemp(prefix="kloss_")
    parts = [todo[i:i + chunk] for i in range(0, len(todo), chunk)]

    def run(k):
        pin, pout = os.path.join(tmp, f"{k}.in.json"), os.path.join(tmp, f"{k}.out.json")
        json.dump([{"key": a, "text": b} for a, b in parts[k]], open(pin, "w"))
        r = subprocess.run(["node", os.path.join(ROOT, "tools/solve_xs.mjs"), pin, pout, "--tol", "0.005"],
                           capture_output=True, text=True, cwd=ROOT)
        if r.returncode != 0:
            return {}
        print(f"  {k + 1}/{len(parts)}", file=sys.stderr, flush=True)
        return {x["key"]: x for x in json.load(open(pout))}

    with ThreadPoolExecutor(jobs) as ex:
        for d in ex.map(run, range(len(parts))):
            cache["results"].update(d)
            json.dump(cache, open(cache_path, "w"), indent=0)
    json.dump(cache, open(cache_path, "w"), indent=0)


# --- CST lossy results and box modes ---------------------------------------------------------------------------

def cst_lossy(d):
    """CST frequency-domain lossy result → {f_ghz: {fw mode name: {alpha_db, r, g, zc_re, eeff, z_line}}}"""
    out = {}
    if not d or d.get("status") != "ok":
        return out
    for fr in (d.get("walls", {}).get("electric", {}) or {}).get("freqs", []):
        modes = [x for x in fr.get("modes", []) if x.get("z") is not None and x.get("beta") is not None]
        if len(modes) == 2 and not all(x.get("meaning_by_value") for x in modes):
            hi = max(modes, key=lambda x: x["z"])  # the larger line impedance is the differential mode
            names = {id(x): ("differential" if x is hi else "common") for x in modes}
        else:
            names = {id(x): x.get("meaning_by_value", x.get("meaning")) for x in modes}
        m = {}
        for x in modes:
            mean_ = names[id(x)]
            if mean_ not in CST_K:
                continue
            k = CST_K[mean_]
            zc = complex(x["z"] * k, (x.get("z_im") or 0) * k)
            g = complex(x.get("alpha") or 0, x["beta"])
            m[FW_NAME[mean_]] = {"alpha_db": x.get("alpha_db_m"), "r": (g * zc).real, "g": (g / zc).real, "wc": (g / zc).imag,
                                 "zc_re": zc.real, "eeff": x.get("eeff"), "z_line": x["z"]}
        if m:
            out[round(fr["f_ghz"], 6)] = m
    return out


def box_cutoff_ghz(case):
    X0, X1, Y0, Y1 = case["domain"]["box"]
    t = case.get("cst_text") or case["text"]
    er = max([float(x) for x in re.findall(r"\ber=([\d.]+)", t)] + [1.0])
    return C0 / (2 * max(X1 - X0, Y1 - Y0) * 1e-3 * math.sqrt(er)) / 1e9


# --- one case ----------------------------------------------------------------------------------------------------

def collect_case(c, a, kisi, kloss):
    """all values of a case: {lossless: {source: {q: v}}, loss: {mode: {f: {source: {...}}}}, flags…}"""
    pair = c.get("n_signals", 1) == 2
    k = kisi.get(c["id"], {})
    rec = {"id": c["id"], "group": c["group"], "tier": c.get("tier"), "title": c.get("title"), "pair": pair,
           "params": c.get("params"), "ref": c.get("ref"), "flags": []}
    v = rec["values"] = {}
    v["formula"] = formula_vals(k.get("formula"), pair)
    v["field"] = fdm_vals(k.get("field"))
    v["fdm_E"], v["fdm_M"] = fdm_vals(k.get("field_E")), fdm_vals(k.get("field_M"))
    v["fdm_open"] = mean(v["fdm_E"], v["fdm_M"]) if v["fdm_M"] else {}
    v["fdm_static_E"] = fdm_static_eeff(kloss.get(c["id"]))
    fw = load(os.path.join(a.fullwave, c["id"] + ".json")) or {}
    rec["fw_geometry"] = fw.get("geometry")
    R = fw.get("results", {})
    v["fw_E"], v["fw_M"] = fw_vals(R.get("E")), fw_vals(R.get("M"))
    v["fw_static_E"] = fw_vals(R.get("E"), "static")
    v["fw_open"] = mean(v["fw_E"], v["fw_M"]) if v["fw_M"] else {}
    rec["fw_seconds"] = fw.get("seconds")
    rec["fw_warnings"] = (((R.get("sweep") or {}).get("info") or {}).get("warnings")) or []
    stem = c["id"] + (f".{a.tag}" if a.tag else "")
    m = load(os.path.join(a.results, stem + ".json"))
    cst = None
    if m:
        if m.get("text_hash") and m["text_hash"] != fnv(c.get("cst_text") or c["text"]):
            rec["flags"].append("stale CST result")
        elif m.get("status") in ("ok", "not_converged") and m.get("result"):
            cst = m
    if cst:
        r = cst["result"]
        v["cst_E"], v["cst_M"] = cst_vals(r.get("electric")), cst_vals(r.get("magnetic"))
        if r.get("open_est"):
            v["cst_open"], rec["cst_open_approx"] = cst_vals(r["open_est"]), False
        elif v["cst_E"] and v["fdm_E"] and v["fdm_M"]:
            v["cst_open"] = {q: v["cst_E"][q] * v["fdm_open"][q] / v["fdm_E"][q] for q in v["cst_E"]
                             if v["cst_E"].get(q) and v["fdm_open"].get(q) and v["fdm_E"].get(q)}
            rec["cst_open_approx"] = True
        else:
            v["cst_open"], rec["cst_open_approx"] = dict(v["cst_E"]), True
        rec["cst_status"] = cst["status"]
        rec["cst_version"] = cst.get("cst_version")
    # pairs: the two impedance definitions (FW matrices)
    rec["asym"] = False
    if not pair:
        for w in ("E", "M"):
            if v[f"fw_{w}"]:
                v[f"fw_{w}_cstdef"] = v[f"fw_{w}"]
    if pair:
        for w in ("E", "M"):
            d = pair_defs(R.get(w))
            if not d or not v[f"fw_{w}"]:
                continue
            rec.setdefault("def_ratio", {})[w] = d[0]
            v[f"fw_{w}_cstdef"] = {q: x * d[0].get(q, 1.0) if x is not None else None for q, x in v[f"fw_{w}"].items()}
        if any(abs(x - 1) * 100 > ASYM_PCT for x in (rec.get("def_ratio") or {}).get("E", {}).values()):
            rec["asym"] = True
            rec["flags"].append("asymmetric pair: CST's impedances (eigenmodes, power-current) transferred to kisi's "
                                "definition through FW's L and C")
    # the CST references of kisi's definition (identical to CST's own for a symmetric pair)
    for w in ("E", "M", "open"):
        ref = v.get(f"cst_{w}")
        if not ref:
            continue
        ratio = (rec.get("def_ratio") or {}).get("M" if w == "M" else "E") if rec["asym"] else None
        v[f"cst_{w}_k"] = {q: x / ratio.get(q, 1.0) if ratio and x is not None else x for q, x in ref.items()}
    # lossy: the sweep at CST's frequencies
    rec["loss"] = loss_case(c, a, R.get("sweep"), kisi_models(kloss.get(c["id"]), c["text"]), rec) if not a.no_loss else {}
    return rec


def loss_case(c, a, sw, models, rec):
    d = load(os.path.join(a.results, c["id"] + ".fd.lossy.json"))
    cl = cst_lossy(d)
    if not cl:
        return {}
    fb = box_cutoff_ghz(c)
    rec["box_cutoff_ghz"] = fb
    fl = [round(f / 1e9, 6) for f in sw["f"]] if sw and sw.get("ok") else []
    jump = {}
    if fl:
        for name, m in sw["modes"].items():
            e = m.get("eeff_mode") or []
            for i in range(1, len(e)):
                if e[i] and e[i - 1] and abs(e[i] / e[i - 1] - 1) * 100 > JUMP_PCT:
                    jump[name] = fl[i]
                    break
    low = {}
    for fg in sorted(cl):
        for name, x in cl[fg].items():
            low.setdefault(name, x["z_line"])
    out = {}
    for fg in sorted(cl):
        for name, x in cl[fg].items():
            p = {"cst": {q: x[q] for q in ("alpha_db", "r", "g", "zc_re", "eeff")}}
            if x["g"] < 1e-5 * x["wc"]:  # tanδ 0 (CST's G is numerical noise there)
                p["cst"]["g"] = None
            if fg in fl:
                i = fl.index(fg)
                m = sw["modes"].get(name)
                if m:
                    p["fw"] = {"alpha_db": m["alpha_c"][i] + m["alpha_d"][i], "r": m["r"][i], "g": m["g"][i],
                               "zc_re": m["zc_re"][i], "eeff": m["eeff"][i]}
            if name in models:
                p["kisi"] = models[name].at(fg * 1e9)
            why = []
            if not 0.5 < x["z_line"] / low[name] < 2:
                why.append("CST impedance off 2x")
            if name in jump and fg >= jump[name]:
                why.append("FW eigenmode jump")
            if fg >= fb and p.get("fw"):
                dz = abs(p["fw"]["zc_re"] / x["zc_re"] - 1) * 100
                de = abs(p["fw"]["eeff"] / x["eeff"] - 1) * 100 if x.get("eeff") else 0
                if max(dz, de) > BOX_MISMATCH:
                    why.append("CST/FW mismatch above the box cutoff")
            if why:
                p["excluded"] = why if fg >= fb else why + ["(below the box cutoff)"]
            out.setdefault(name, {})[fg] = p
    return out


# --- the report ----------------------------------------------------------------------------------------------------

LOSSLESS = [
    # (comparison, source key, reference key, quantities, note)
    ("FW vs CST, E walls", "fw_E_cstdef|fw_E", "cst_E", ZQ + EQ, "same box; εeff (PEC); pairs: FW in CST's impedance definition"),
    ("FW vs CST, M walls", "fw_M_cstdef|fw_M", "cst_M", ZQ + EQ, "same box, magnetic walls"),
    ("FW static vs CST, E walls", "fw_static_E", "cst_E_k", ZQ + EQ, "static P2 FEM on the full-wave mesh; εeff static"),
    ("FDM vs CST, E walls", "fdm_E", "cst_E_k", ZQ, "same box (kisi.json field_E, tol 0.002); Z lossless"),
    ("FDM vs CST, M walls", "fdm_M", "cst_M_k", ZQ, "same box, natural walls (field_M)"),
    ("FDM static vs CST, E walls", "fdm_static_E", "cst_E_k", EQ, "εeff = (c·Z0·C)² of kisi_loss.json (tol 0.005)"),
    ("FDM vs FW, E walls", "fdm_E", "fw_E", ZQ, "the two js_2d_fields backends, kisi's definition"),
    ("FW open vs CST open", "fw_open", "cst_open_k", ZQ + EQ, "mean of E and M walls on both sides"),
    ("field vs CST open", "field", "cst_open_k", ZQ, "kisi's field result as reported (tol 0.01, its own open domain)"),
    ("formula vs CST open", "formula", "cst_open_k", ZQ + EQ, "kisi's closed-form result; εeff quasi-static"),
    ("CST vs ref", "cst_open", "ref", ZQ, "anchors: HFSS / Ansys / Forstén's MS/SL solver"),
    ("FW open vs ref", "fw_open", "ref", ZQ, ""),
    ("field vs ref", "field", "ref", ZQ, ""),
    ("formula vs ref", "formula", "ref", ZQ, ""),
]
PHASE = ("εeff (phase) 1 GHz", "copper + tanδ: FW sweep and FDM field_E against CST's lossy result")
LOSS_Q = ("alpha_db", "r", "g", "zc_re", "eeff")
LOSS_LABEL = {"alpha_db": "α", "r": "R", "g": "G", "zc_re": "Re Zc", "eeff": "εeff phase"}


def compare_lossless(recs, quants, a):
    comps, outl = {}, []
    lim = {"formula vs CST open": a.formula_outlier, "formula vs ref": a.formula_outlier}

    def add(comp, r, e):
        for q, x in e.items():
            if q not in quants:
                continue
            for g in (r["group"], "ALL"):
                comps.setdefault((comp, g, q), []).append((r["id"], x))
            if abs(x) > lim.get(comp, a.outlier) and "ref" not in comp and comp != "FDM phase vs FDM static":
                outl.append((comp, r, q, x))

    for r in recs:
        v = r["values"]
        etch = r["group"] == "etch"
        for comp, src, ref, qs, _ in LOSSLESS:
            s = next((v.get(k) for k in src.split("|") if v.get(k)), None)
            rf = r.get("ref") if ref == "ref" else v.get(ref)
            if not s or not rf:
                continue
            if etch and src.startswith("fdm") and "CST" in comp or (etch and comp == "FDM vs FW, E walls"):
                continue  # the FDM solved kisi's rectangle, CST and FW the trapezoid
            if src == "fw_static_E" or comp.startswith("formula"):
                s = dict(s)
            e = err(s, rf, [q for q in qs if q in rf])
            add(comp, r, e)
        # phase εeff at 1 GHz: CST lossy vs FW sweep and the FDM
        cl = {name: pts.get(1.0) for name, pts in (r.get("loss") or {}).items()}
        for name, p in cl.items():
            if not p or "excluded" in p:
                continue
            q = {"single": "eeff", "odd": "eeff_odd", "even": "eeff_even"}[name]
            ce = p["cst"].get("eeff")
            if p.get("fw") and ce and not r["asym"]:
                add("FW phase vs CST lossy", r, {q: (p["fw"]["eeff"] / ce - 1) * 100})
            fe = v["fdm_E"].get(q)
            if fe and ce and not r["asym"] and not etch:
                add("FDM phase vs CST lossy", r, {q: (fe / ce - 1) * 100})
            if fe and v["fdm_static_E"].get(q):
                add("FDM phase vs FDM static", r, {q: (fe / v["fdm_static_E"][q] - 1) * 100})
    return comps, outl


def compare_loss(recs):
    rows = {}  # (solver, q, f, group) -> [(id:mode, err)]
    excl = {"points": 0, "excluded": 0, "by_reason": {}, "by_f": {}, "below_cutoff": [], "asym_left_out": 0}
    for r in recs:
        for name, pts in (r.get("loss") or {}).items():
            for fg, p in pts.items():
                excl["points"] += 1
                if r["asym"]:
                    excl["asym_left_out"] += 1
                    continue
                if "excluded" in p:
                    excl["excluded"] += 1
                    for why in p["excluded"]:
                        excl["by_reason"][why] = excl["by_reason"].get(why, 0) + 1
                    excl["by_f"][fg] = excl["by_f"].get(fg, 0) + 1
                    if "(below the box cutoff)" in p["excluded"]:
                        excl["below_cutoff"].append(f"{r['id']}:{name}@{fg:g}")
                    continue
                for sv in ("fw", "kisi"):
                    if not p.get(sv):
                        continue
                    for q in LOSS_Q:
                        x, ref = p[sv].get(q), p["cst"].get(q)
                        if x is None or not ref or (q == "g" and (ref <= 0 or x <= 0)):
                            continue
                        for g in (r["group"], "ALL"):
                            rows.setdefault((sv, q, fg, g), []).append((f"{r['id']}:{name}", (x / ref - 1) * 100))
                if p.get("fw") and p.get("kisi"):
                    for q in LOSS_Q:
                        x, ref = p["kisi"].get(q), p["fw"].get(q)
                        if x is not None and ref and not (q == "g" and (ref <= 0 or x <= 0)):
                            for g in (r["group"], "ALL"):
                                rows.setdefault(("kisi vs fw", q, fg, g), []).append((f"{r['id']}:{name}", (x / ref - 1) * 100))
    return rows, excl


def fmt_s(s, d=2):
    if not s:
        return "–"
    return f"{s['bias']:+.{d}f} / {s['med']:.{d}f} / {s['p90']:.{d}f}"


def report(recs, a, quants):
    L = []
    n_cst = sum(1 for r in recs if r["values"].get("cst_E"))
    n_fw = sum(1 for r in recs if r["values"].get("fw_E"))
    n_loss = sum(1 for r in recs if r.get("loss"))
    n_asym = sum(1 for r in recs if r["asym"])
    L += ["# Cross-section comparison: CST, full-wave (FW), FDM, formula", "",
          f"{len(recs)} cases: CST lossless {n_cst}, FW {n_fw}, CST lossy {n_loss}, asymmetric pairs {n_asym}. "
          "Error % = value / CST − 1; cells: bias (median signed) / median |e| / p90 |e|.", ""]
    comps, outl = compare_lossless(recs, quants, a)
    groups = []
    for r in recs:
        if r["group"] not in groups:
            groups.append(r["group"])
    # four-way table per group
    qs_z = [q for q in ZQ if q in quants]
    qs_e = [q for q in EQ if q in quants]
    if qs_z:
        L += ["## Impedance, 1 GHz lossless: four-way per group", "",
              "FW and FDM in CST's box with electric walls (the same problem); formula against CST's open-boundary "
              "estimate. Pairs: kisi's impedance definition, CST transferred for asymmetric pairs; FW in CST's definition.", "",
              "| group | qty | n | CST (median) | FW vs CST | FDM vs CST | formula vs CST open |", "|---|---|---:|---:|---|---|---|"]
        for g in groups + ["ALL"]:
            for q in qs_z:
                s = [stats(comps.get((cp, g, q), [])) for cp in ("FW vs CST, E walls", "FDM vs CST, E walls", "formula vs CST open")]
                if not any(s):
                    continue
                vals = [r["values"]["cst_E"][q] for r in recs if (g == "ALL" or r["group"] == g) and r["values"].get("cst_E", {}).get(q)]
                n = max(x["n"] for x in s if x)
                L.append(f"| {g} | {q} | {n} | {pct(vals, 0.5):.2f} | {fmt_s(s[0], 3)} | {fmt_s(s[1])} | {fmt_s(s[2])} |")
        L.append("")
    if qs_e:
        L += ["## εeff, 1 GHz: per definition", "",
              "PEC: CST lossless (time domain) against FW eigenmode, FW static, FDM static (C/C0) and the formula. "
              "Phase: CST lossy (copper, tanδ) against the FW sweep and kisi's FDM (field_E, how it reports εeff).", "",
              "| group | qty | FW (PEC) | FW static | FDM static | formula | FW phase vs CST lossy | FDM phase vs CST lossy |",
              "|---|---|---|---|---|---|---|---|"]
        for g in groups + ["ALL"]:
            for q in qs_e:
                cps = ("FW vs CST, E walls", "FW static vs CST, E walls", "FDM static vs CST, E walls", "formula vs CST open",
                       "FW phase vs CST lossy", "FDM phase vs CST lossy")
                s = [stats(comps.get((cp, g, q), [])) for cp in cps]
                if not any(s):
                    continue
                L.append(f"| {g} | {q} | " + " | ".join(fmt_s(x, 3 if i < 3 else 2) for i, x in enumerate(s)) + " |")
        L.append("")
    # every comparison in full
    L += ["## All lossless comparisons (ALL groups)", "", "| comparison | qty | n | bias % | med % | p90 % | max % | max case | note |",
          "|---|---|---:|---:|---:|---:|---:|---|---|"]
    notes = {cp: nt for cp, _, _, _, nt in LOSSLESS}
    notes.update({"FW phase vs CST lossy": "copper + tanδ, 1 GHz", "FDM phase vs CST lossy": "field_E εeff vs CST lossy, 1 GHz",
                  "FDM phase vs FDM static": "the internal inductance in the FDM's reported εeff"})
    for cp in [x[0] for x in LOSSLESS] + ["FW phase vs CST lossy", "FDM phase vs CST lossy", "FDM phase vs FDM static"]:
        for q in quants:
            s = stats(comps.get((cp, "ALL", q), []))
            if s:
                L.append(f"| {cp} | {q} | {s['n']} | {s['bias']:+.3f} | {s['med']:.3f} | {s['p90']:.3f} | {s['max_signed']:+.2f} | "
                         f"{s['max_case']} | {notes.get(cp, '')} |")
    L.append("")
    if a.groups_full:
        for cp in [x[0] for x in LOSSLESS]:
            keys = [(g, q) for g in groups for q in quants if (cp, g, q) in comps]
            if not keys:
                continue
            L += [f"### {cp}", "", "| group | qty | n | bias % | med % | p90 % | max % | max case |", "|---|---|---:|---:|---:|---:|---:|---|"]
            for g, q in keys:
                s = stats(comps[(cp, g, q)])
                L.append(f"| {g} | {q} | {s['n']} | {s['bias']:+.3f} | {s['med']:.3f} | {s['p90']:.3f} | {s['max_signed']:+.2f} | {s['max_case']} |")
            L.append("")
    # asymmetric pairs
    asym = [r for r in recs if r["asym"]]
    if asym:
        L += ["## Asymmetric pairs: the two impedance definitions", "",
              "CST (eigenmodes, power-current) against FW in the same definition; FW and FDM (kisi's definition) "
              "against CST raw and transferred; the eigenmode εeff needs no definition.", "",
              "| case | CST Zdiff / Zeven | FW CST-def. vs CST | FW kisi-def. vs CST raw | FDM vs CST transferred | εeff odd / even FW vs CST |",
              "|---|---|---|---|---|---|"]
        for r in asym:
            v = r["values"]
            ce, fc, fp, fd, ct = v.get("cst_E", {}), v.get("fw_E_cstdef", {}), v.get("fw_E", {}), v.get("fdm_E", {}), v.get("cst_E_k", {})
            e1, e2, e3 = err(fc, ce, ("zdiff", "zeven")), err(fp, ce, ("zdiff", "zeven")), err(fd, ct, ("zdiff", "zeven"))
            e4 = err(fp, ce, ("eeff_odd", "eeff_even"))
            f2 = lambda e, k1, k2: f"{e.get(k1, float('nan')):+.2f} / {e.get(k2, float('nan')):+.2f}"
            L.append(f"| {r['id']} | {ce.get('zdiff', 0):.2f} / {ce.get('zeven', 0):.2f} | {f2(e1, 'zdiff', 'zeven')} | {f2(e2, 'zdiff', 'zeven')} | "
                     f"{f2(e3, 'zdiff', 'zeven')} | {f2(e4, 'eeff_odd', 'eeff_even')} |")
        L.append("")
    # loss
    if not a.no_loss:
        rows, excl = compare_loss(recs)
        if rows:
            fs = sorted({k[2] for k in rows})
            L += ["## Loss sweep against CST's lossy results (electric walls)", "",
                  f"Points {excl['points']}; asymmetric pairs left out {excl['asym_left_out']}; box-mode points left out "
                  f"{excl['excluded']} (by frequency: " + ", ".join(f"{f:g} GHz {n}" for f, n in sorted(excl["by_f"].items())) +
                  "; reasons: " + ", ".join(f"{k} {n}" for k, n in sorted(excl["by_reason"].items())) + ")" +
                  (f"; below the box cutoff: {', '.join(excl['below_cutoff'])}" if excl["below_cutoff"] else "; none below the box cutoff") + ".", "",
                  "FW: the sweep (MQS conductor loss, Djordjevic-Sarkar). kisi: kisi's line model from its FDM solve at 1 GHz.", "",
                  "| solver | qty | " + " | ".join(f"{f:g} GHz" for f in fs) + " |", "|---|---|" + "---|" * len(fs)]
            for sv in ("fw", "kisi", "kisi vs fw"):
                for q in LOSS_Q:
                    cells = [fmt_s(stats(rows.get((sv, q, f, "ALL"), [])), 1 if q in ("alpha_db", "r", "g") else 2) for f in fs]
                    if any(c != "–" for c in cells):
                        L.append(f"| {sv} | {LOSS_LABEL[q]} | " + " | ".join(cells) + " |")
            L += ["", "Per group, R (conductor) and G (dielectric) at 1 and 10 GHz, bias / med / p90:", "",
                  "| group | n | FW R 1 GHz | FW R 10 GHz | kisi R 1 GHz | kisi R 10 GHz | FW G 1 GHz | kisi G 1 GHz | FW α 1 GHz | kisi α 1 GHz |",
                  "|---|---:|---|---|---|---|---|---|---|---|"]
            for g in groups + ["ALL"]:
                cell = lambda sv, q, f: fmt_s(stats(rows.get((sv, q, f, g), [])), 1)
                n = len(rows.get(("fw", "r", 1.0, g), []))
                if n:
                    L.append(f"| {g} | {n} | {cell('fw', 'r', 1.0)} | {cell('fw', 'r', 10.0)} | {cell('kisi', 'r', 1.0)} | {cell('kisi', 'r', 10.0)} | "
                             f"{cell('fw', 'g', 1.0)} | {cell('kisi', 'g', 1.0)} | {cell('fw', 'alpha_db', 1.0)} | {cell('kisi', 'alpha_db', 1.0)} |")
            L.append("")
            if a.loss_table:
                L += ["### Loss per case (error % against CST)", "", "| case:mode | f GHz | FW α | FW R | FW G | FW Re Zc | FW εeff | kisi α | kisi R | kisi G | note |",
                      "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|"]
                for r in recs:
                    for name, pts in (r.get("loss") or {}).items():
                        for fg, p in sorted(pts.items()):
                            def e(sv, q):
                                x, ref = (p.get(sv) or {}).get(q), p["cst"].get(q)
                                return "–" if x is None or not ref else f"{(x / ref - 1) * 100:+.1f}"
                            note = "; ".join(p.get("excluded", [])) + (" asymmetric" if r["asym"] else "")
                            L.append(f"| {r['id']}:{name} | {fg:g} | " + " | ".join(e("fw", q) for q in LOSS_Q) + " | " +
                                     " | ".join(e("kisi", q) for q in ("alpha_db", "r", "g")) + f" | {note} |")
                L.append("")
    # outliers
    if outl:
        L += [f"## Outliers (solver / field above {a.outlier} %, formula above {a.formula_outlier} %)", ""]
        order = [x[0] for x in LOSSLESS] + ["FW phase vs CST lossy", "FDM phase vs CST lossy", "FDM phase vs FDM static"]
        outl.sort(key=lambda t: (order.index(t[0]), -abs(t[3])))
        for comp, r, q, x in outl:
            L.append(f"* {comp} {q} {x:+.2f} %: {r['id']} ({r['title']})" + (f" [{'; '.join(r['flags'])}]" if r["flags"] else ""))
            if a.show_text:
                c = a._cases[r["id"]]
                L.extend("      " + ln for ln in (c.get("cst_text") or c["text"]).strip().splitlines())
        L.append("")
    if a.per_case:
        f = lambda x, d=4: "–" if x is None else f"{x:.{d}f}"
        L += ["## Per case (1 GHz, lossless, E walls)", "", "| case | q | CST-E | FW-E | FW static | FDM-E | CST-M | FW-M | FDM-M | formula |",
              "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
        for r in recs:
            v = r["values"]
            for q in [q for q in quants if q in (v.get("fw_E") or v.get("cst_E") or {})]:
                L.append(f"| {r['id']} | {q} | " + " | ".join(f((v.get(s) or {}).get(q)) for s in ("cst_E", "fw_E", "fw_static_E", "fdm_E", "cst_M", "fw_M", "fdm_M", "formula")) + " |")
        L.append("")
    if a.fw_diag or a.sweep_table:
        L += fw_diagnostics(recs, a)
    return "\n".join(L), comps, outl


def fw_diagnostics(recs, a):
    L, times, warn = [], {}, {}
    for r in recs:
        for k, s in (r.get("fw_seconds") or {}).items():
            if s is not None:
                times.setdefault(k, []).append((r["id"], s))
        for w in r.get("fw_warnings") or []:
            warn.setdefault(w.split(":")[0], []).append(r["id"])
    if times:
        L += ["## FW run time per task (s, one node process on one vCPU)", "", "| kind | n | median | p90 | max | max case |", "|---|---:|---:|---:|---:|---|"]
        for k, rows in sorted(times.items()):
            v = [s for _, s in rows]
            j = max(range(len(v)), key=lambda i: v[i])
            L.append(f"| {k} | {len(v)} | {pct(v, 0.5):.0f} | {pct(v, 0.9):.0f} | {v[j]:.0f} | {rows[j][0]} |")
        L.append("")
    if warn:
        L += ["FW sweep warnings (type: number of cases): " + ", ".join(f"{k} {len(v)}" for k, v in sorted(warn.items())), ""]
    if a.sweep_table:
        for r in recs:
            fw = load(os.path.join(a.fullwave, r["id"] + ".json")) or {}
            sw = (fw.get("results") or {}).get("sweep")
            if not sw or not sw.get("ok"):
                continue
            L += [f"### FW sweep {r['id']}", "", "| mode | f GHz | Z0 | Re Zc | εeff | εeff,mode | αc dB/m | αd dB/m | R Ω/m | G S/m |",
                  "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
            for name, m in sw["modes"].items():
                for i, fr in enumerate(sw["f"]):
                    g = lambda k: (m.get(k) or [None] * len(sw["f"]))[i]
                    cells = [g(k) for k in ("z0", "zc_re", "eeff", "eeff_mode", "alpha_c", "alpha_d", "r", "g")]
                    L.append(f"| {name} | {fr / 1e9:.4g} | " + " | ".join("–" if x is None else f"{x:.4g}" for x in cells) + " |")
            L.append("")
    return L


def collect(a):
    cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
    if a.groups:
        cases = [c for c in cases if c["group"] in a.groups.split(",")]
    if a.tier:
        cases = [c for c in cases if str(c["tier"]) in a.tier.split(",")]
    if a.ids:
        cases = [c for c in cases if c["id"] in a.ids.split(",")]
    a._cases = {c["id"]: c for c in cases}
    kisi = (load(a.kisi) or {}).get("results", {})
    kloss = (load(a.loss_cache) or {}).get("results", {})
    return [collect_case(c, a, kisi, kloss) for c in cases], cases


def parser():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--kisi", default=os.path.join(HERE, "kisi.json"))
    ap.add_argument("--results", default=os.path.join(HERE, "results"))
    ap.add_argument("--fullwave", default=os.path.join(HERE, "fullwave"))
    ap.add_argument("--loss-cache", default=os.path.join(HERE, "kisi_loss.json"))
    ap.add_argument("--tag", default="", help="compare results/<id>.<tag>.json (lossless)")
    ap.add_argument("--groups", default="")
    ap.add_argument("--ids", default="")
    ap.add_argument("--tier", default="")
    ap.add_argument("--quantity", choices=["z", "eeff", "all"], default="all")
    ap.add_argument("--outlier", type=float, default=2.0, help="list solver / field errors above this, %%")
    ap.add_argument("--formula-outlier", type=float, default=5.0, help="list formula errors above this, %%")
    ap.add_argument("--show-text", action="store_true", help="print the geometry text of outliers")
    ap.add_argument("--groups-full", action="store_true", help="every lossless comparison per group")
    ap.add_argument("--no-loss", action="store_true", help="lossless only")
    ap.add_argument("--loss-table", action="store_true", help="the loss comparison per case and frequency")
    ap.add_argument("--per-case", action="store_true", help="the lossless values per case")
    ap.add_argument("--sweep-table", action="store_true", help="every FW sweep point of every case")
    ap.add_argument("--fw-diag", action="store_true", help="FW run times and sweep warnings")
    ap.add_argument("--solve-loss", action="store_true", help="run kisi's FDM solves for the line model (node) → --loss-cache")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--json", default="")
    ap.add_argument("--md", default="")
    return ap


def main(argv=None):
    a = parser().parse_args(argv)
    if a.solve_loss:
        cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
        if a.ids:
            cases = [c for c in cases if c["id"] in a.ids.split(",")]
        solve_loss(cases, a.results, a.loss_cache, a.jobs)
        return
    recs, _ = collect(a)
    quants = {"z": ZQ, "eeff": EQ, "all": ZQ + EQ}[a.quantity]
    rep, comps, outl = report(recs, a, quants)
    print(rep)
    if a.md:
        open(a.md, "w", encoding="utf-8").write(rep + "\n")
    if a.json:
        summary = {}
        for (cp, g, q), rows in comps.items():
            summary.setdefault(cp, {}).setdefault(g, {})[q] = stats(rows)
        loss_rows, excl = compare_loss(recs) if not a.no_loss else ({}, {})
        loss = {}
        for (sv, q, fg, g), rows in loss_rows.items():
            loss.setdefault(sv, {}).setdefault(q, {}).setdefault(f"{fg:g}", {})[g] = stats(rows)
        json.dump({"summary": summary, "loss": loss, "box_modes": excl, "cases": {r["id"]: r for r in recs},
                   "outliers": [{"comparison": o[0], "id": o[1]["id"], "quantity": o[2], "err_pct": o[3]} for o in outl]},
                  open(a.json, "w", encoding="utf-8"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
