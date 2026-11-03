#!/usr/bin/env python3
"""The fit of the two constants of kisi's coplanar formulas (crates/kisi-si/src/analytic.rs, section "Coplanar
lines with a ground plane"): the corner term of a slot between copper side walls, C/ε0 = t/s + a·(t/s)^b per slot
(code: SLOT_CORNER = 0.067, SLOT_CORNER_EXP = 0.090), and the width τ of the smooth maximum that joins the coplanar
capacitances to the plain line's (code: TAU = 0.045).

    python3 validation/cst_xs/fit_coplanar.py [--kisi-validate target/release/kisi-validate] [--jobs 2]
        [--tol 0.005] [--out validation/cst_xs/coplanar_fit.json]        # sweep, solve, store, fit
    python3 validation/cst_xs/fit_coplanar.py --from-json [validation/cst_xs/coplanar_fit.json]   # fit only
    python3 validation/cst_xs/fit_coplanar.py --formula-only     # kisi's formula values again (formulas changed), fit

The sweep (deterministic: fixed grids, seeded subsets) has 1 100 synthetic cross-sections built from stackups and
cross-section descriptions as make_cases.py builds them: grounded CPW (surface microstrip with coplanar ground on
one or both sides, with and without solder mask), stripline with coplanar ground, the coupled grounded CPW (pairs)
and the CPW without a plane. `kisi-validate xs-cases` gives kisi's geometry text and formula values for each (and
the plain line's formula, the same description without the coplanar copper); tools/solve_xs.mjs solves the texts
with kisi's field solver (js_2d_fields FDM, as run_kisi.py does; the no-plane CPW with make_cases.text_cpw, as in
the cpw_noplane group). The result is stored compactly in coplanar_fit.json, so the fit can be repeated without
solving (--from-json).

The fit evaluates the coplanar formulas in Python (a transcription of analytic.rs, checked against kisi's own
formula values for every case at the code's constants) with free constants:
  1. corner term: on the single lines with coplanar copper on both sides, without mask and with a gap of at most
     half the height to the nearest plane (where the smooth maximum hardly acts), the slot capacitance that makes
     kisi's formula equal to the field solution (bisection, τ → 0); a·(t/s)^b fitted by least squares to its
     excess over t/s (b on a 0.005 grid, a in closed form);
  2. τ: with that corner term, the τ (0.005 grid) with the smallest mean |error| of Z0 over all single lines with
     coplanar copper (grounded CPW with and without mask, stripline with coplanar ground).
It prints the fitted values against the code's, and the error statistics (Z0, Zdiff, Zeven against the field
solver) per family with the code's and with the fitted constants. The field solver itself is within 0.2–0.5 % of
CST (README.md); the coplanar formulas' check against CST is compare.py.
Run time: the sweep about 20 min on 2 cores (--jobs 2), the fit alone about a second.
"""
import argparse, json, math, os, random, subprocess, sys, tempfile, time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import make_cases as mc  # noqa: E402
import run_kisi  # noqa: E402

PI = math.pi
ETA0 = 376.730313668
CODE = {"corner": 0.067, "corner_exp": 0.090, "tau": 0.045}  # analytic.rs: SLOT_CORNER, SLOT_CORNER_EXP, TAU
OUT = os.path.join(HERE, "coplanar_fit.json")


# --- the sweep --------------------------------------------------------------------------------------------------

def _one_sided(spec):
    spec["desc"]["s_left"] = None
    return spec


def _plain(spec):
    s = json.loads(json.dumps(spec))
    s["desc"]["s_left"] = s["desc"]["s_right"] = None
    return s


def sweep():
    """[{id, fam, p, spec, text?}] – fixed grids, seeded subsets"""
    cases = []
    g = mc.xsgeo.fmt
    # grounded CPW: h × εr × t × w/h × gap/h × mask × sides → 680 of them
    ms = []
    for h in (0.1, 0.2, 0.5, 1.0):
        for er in (3.0, 4.4, 10.0):
            for t in (0.0175, 0.035, 0.07):
                for u in (0.5, 1, 2, 4):
                    for gh in (0.1, 0.25, 0.5, 1, 2, 4):
                        for mask in (None, (0.015, 3.3), (0.03, 3.8)):
                            for side in ("both", "one"):
                                if er == 10.0 and (t != 0.035 or mask):
                                    continue
                                s = gh * h
                                if s < 0.025:
                                    continue
                                p = {"w": u * h, "h": h, "t": t, "er": er, "mask": mask, "sl": s if side == "both" else None, "sr": s}
                                spec = mc.spec_ms(u * h, h, t, er, mask=mask, gap=s)
                                if side == "one":
                                    _one_sided(spec)
                                ms.append({"id": f"ms_h{g(h)}_er{g(er)}_t{g(t)}_u{g(u)}_g{g(gh)}_m{mask[0] if mask else 0}_{side}",
                                           "fam": "cpwg" + ("_mask" if mask else ""), "p": p, "spec": spec})
    random.Random(1).shuffle(ms)
    cases += ms[:680]
    # stripline with coplanar ground: h (below) × ratio above/below × t × w/h × gap/h × sides × εr → 300
    sl = []
    for h1 in (0.1, 0.2):
        for r in (1, 2, 5):
            for t in (0.0175, 0.035):
                for u in (0.5, 1, 2):
                    for gh in (0.25, 0.5, 1, 2):
                        for side in ("both", "one"):
                            for er1, er2 in ((3.6, 3.6), (4.2, 4.6)):
                                s = gh * h1
                                if s < 0.025:
                                    continue
                                h2 = r * h1
                                p = {"w": u * h1, "h_up": h2, "h_dn": h1, "t": t, "e_up": er2, "e_dn": er1, "e_fill": er2,
                                     "sl": s if side == "both" else None, "sr": s}
                                spec = mc.spec_sl(u * h1, h1, h2, t, er1, er2, gap=s)
                                if side == "one":
                                    _one_sided(spec)
                                sl.append({"id": f"sl_h{g(h1)}_r{r}_t{g(t)}_u{g(u)}_g{g(gh)}_e{g(er2)}_{side}", "fam": "stripline_cpw",
                                           "p": p, "spec": spec})
    random.Random(2).shuffle(sl)
    cases += sl[:300]
    # coupled grounded CPW: h × w/h × gap/h × outer gap/h × mask, both outer gaps or only this trace's → 80
    pr = []
    for h in (0.1, 0.2):
        for u in (0.5, 1, 2):
            for sh in (0.25, 0.5, 1):
                for gh in (0.25, 0.5, 1, 2):
                    for mask in (None, (0.02, 3.3)):
                        for side in ("both", "one"):
                            w, gp, s = u * h, sh * h, gh * h
                            if gp < 0.025 or s < 0.025:
                                continue
                            spec = mc.spec_ms(w, h, 0.035, 4.4, mask=mask, s=gp, gap=s)
                            if side == "one":
                                _one_sided(spec)
                            p = {"w": w, "g": gp, "h": h, "t": 0.035, "er": 4.4, "mask": mask, "s_out": s,
                                 "s_partner": s if side == "both" else None}
                            pr.append({"id": f"pair_h{g(h)}_u{g(u)}_s{g(sh)}_g{g(gh)}_m{1 if mask else 0}_{side}", "fam": "cpwg_pair",
                                       "p": p, "spec": spec})
    random.Random(3).shuffle(pr)
    cases += pr[:80]
    # CPW without a plane: h × w × gap × t → 40
    npl = []
    for h in (0.8, 1.6):
        for w in (0.2, 0.3, 0.6, 1.0):
            for s in (0.1, 0.15, 0.25, 0.4, 0.6):
                for t in (0.035, 0.07):
                    npl.append({"id": f"np_h{g(h)}_w{g(w)}_g{g(s)}_t{g(t)}", "fam": "cpw_noplane",
                                "p": {"w": w, "h": h, "t": t, "er": 4.4, "s": s}, "spec": mc.spec_cpw(w, h, t, 4.4, s),
                                "text": mc.text_cpw(w, h, t, 4.4, s)})
    random.Random(4).shuffle(npl)
    cases += npl[:40]
    return cases


def rnd(x):
    return float(f"{x:.6g}") if isinstance(x, float) else x


def run_sweep(a):
    t0 = time.time()
    cases = sweep()
    tmp = tempfile.mkdtemp(prefix="coplanar_fit_")
    fx = formulas(a, cases, tmp)
    items = []
    for c in cases:
        text = c.get("text") or fx[c["id"]].get("text")
        if not text:
            raise SystemExit(f"no geometry for {c['id']}")
        items.append((c["id"], text))
    work = [items[i:i + a.chunk] for i in range(0, len(items), a.chunk)]
    res = {}
    with ThreadPoolExecutor(max_workers=a.jobs) as ex:
        futs = [ex.submit(run_kisi.solve, w, a.tol, tmp, f"fit_{n}") for n, w in enumerate(work)]
        for n, f in enumerate(futs):
            res.update(f.result())
            print(f"  {n + 1}/{len(futs)} solver batches, {time.time() - t0:.0f} s", file=sys.stderr)
    out = []
    for c in cases:
        fr = res.get(c["id"], {})
        if not fr.get("ok"):
            print(f"solver failed: {c['id']} {fr.get('error')}", file=sys.stderr)
            continue
        field = {"zodd": rnd(fr["zodd"]), "zeven": rnd(fr["zeven"])} if fr.get("zodd") else {"z0": rnd(fr["z0"])}
        e = {"id": c["id"], "fam": c["fam"], "p": {k: (list(v) if isinstance(v, tuple) else rnd(v)) for k, v in c["p"].items()},
             "field": field}
        e.update(formula_entry(c, fx))
        out.append(e)
    doc = {"about": "Synthetic cross-sections for the fit of kisi's coplanar-formula constants (fit_coplanar.py; README.md): "
                    "per case the parameters p (mm), kisi's field solution (tools/solve_xs.mjs), kisi's formula (kisi) and the "
                    "plain line's formula without the coplanar copper (plain), both from kisi-validate xs-cases, at the "
                    "constants `constants`.",
           "kisi_rev": git_rev(), "constants": CODE, "date": time.strftime("%Y-%m-%d"), "tol": a.tol,
           "solve_seconds": round(time.time() - t0), "cases": out}
    write(doc, a.out)
    print(f"{len(out)} cases → {os.path.relpath(a.out, ROOT)} ({time.time() - t0:.0f} s)")
    return doc


def git_rev():
    return subprocess.run(["git", "describe", "--always", "--dirty", "--abbrev=7"], capture_output=True, text=True, cwd=ROOT).stdout.strip()


def formulas(a, cases, tmp):
    """kisi-validate xs-cases on the cases and on their plain lines (the description without coplanar copper)"""
    specs = [{"id": c["id"], "spec": c["spec"]} for c in cases] + \
            [{"id": c["id"] + "#plain", "spec": _plain(c["spec"])} for c in cases if c["fam"] != "cpw_noplane"]
    pin, pout = os.path.join(tmp, "specs.json"), os.path.join(tmp, "formula.json")
    json.dump(specs, open(pin, "w"))
    r = subprocess.run([a.kisi_validate, "xs-cases", pin, "--out", pout], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(r.stderr)
    return {x["key"]: x for x in json.load(open(pout))}


def formula_entry(c, fx):
    """what the fit needs from kisi: its impedances (the transcription check) and the plain line's modes"""
    pair = c["fam"] == "cpwg_pair"
    e = {"kisi": {k: rnd(fx[c["id"]]["formula"][k]) for k in (("zodd", "zeven") if pair else ("z0",))}}
    if c["fam"] != "cpw_noplane":
        pf = fx[c["id"] + "#plain"]["formula"]
        e["plain"] = {k: rnd(pf[k]) for k in (("zdiff", "zeven", "eeff_odd", "eeff_even") if pair else ("z0", "eeff"))}
    return e


def write(doc, path):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("{\n" + ",\n".join(f' {json.dumps(k)}: {json.dumps(v, ensure_ascii=False)}' for k, v in doc.items() if k != "cases"))
        fh.write(',\n "cases": [\n' + ",\n".join("  " + json.dumps(c, ensure_ascii=False, separators=(",", ":"))
                                                  for c in doc["cases"]) + "\n ]\n}\n")


def refresh_formulas(a):
    """--formula-only: kisi's formula values of the stored sweep again (after a change of the formulas or their
    constants), the field solutions kept"""
    doc = json.load(open(a.out, encoding="utf-8"))
    by_id = {c["id"]: c for c in sweep()}
    cases = [by_id[c["id"]] for c in doc["cases"]]
    fx = formulas(a, cases, tempfile.mkdtemp(prefix="coplanar_fit_"))
    for c in doc["cases"]:
        c.update(formula_entry(by_id[c["id"]], fx))
    rest = {k: v for k, v in doc.items() if k != "cases"}
    rest.update({"formula_rev": git_rev(), "constants": CODE})
    doc = dict(rest, cases=doc["cases"])
    write(doc, a.out)
    print(f"{len(cases)} cases: kisi's formula values → {os.path.relpath(a.out, ROOT)} (field solutions kept)")
    return doc


# --- kisi's coplanar formulas in Python (analytic.rs) ------------------------------------------------------------

def ellip_k(k):
    kp = math.sqrt(max(1.0 - k * k, 0.0))
    a, b = 1.0, kp
    for _ in range(40):
        a, b = 0.5 * (a + b), math.sqrt(a * b)
        if abs(a - b) < 1e-15:
            break
    return PI / (2.0 * a)


def k_ratio(k):
    return ellip_k(k) / ellip_k(math.sqrt(max(1.0 - k * k, 0.0)))


def slot_cap(t, s, c):
    if t <= 0 or s <= 0:
        return 0.0
    r = t / s
    return r + c["corner"] * r ** c["corner_exp"]


def z_of(c, ca):
    return ETA0 / math.sqrt(c * ca), c / ca


def caps_of(z, e):
    ca = ETA0 / (z * math.sqrt(e))
    return e * ca, ca


def smooth_max(a, b, tau):
    if tau <= 0:
        return max(a, b)
    x = (a / b - 1.0) / tau
    sp = x + math.log1p(math.exp(-x)) if x > 30 else math.log1p(math.exp(x))
    return b * (1.0 + tau * sp)


def open_cap(w, s):
    return 2.0 * k_ratio(w / (w + 2.0 * s))


def slab_cap(w, s, h):
    return 2.0 * k_ratio(math.tanh(PI * w / (4 * h)) / math.tanh(PI * (w + 2 * s) / (4 * h)))


def cover_cap(w, s, d):
    if d <= 0:
        return 0.0
    a, b = PI * w / (4 * d), PI * (w + 2 * s) / (4 * d)
    k = math.exp(a - b) if b > 300 else math.sinh(a) / math.sinh(b)
    return 2.0 * k_ratio(k)


def mask_on_slot(w, s, t, mask):
    if not mask or mask[0] <= 0 or mask[1] <= 1:
        return 1.0, 1.0
    tm, em = mask
    if tm >= s or t <= 0:
        e_slot = em
    else:
        e_mix = 1.0 + (em - 1.0) * min(tm / t, 1.0)
        e_slot = 1.0 / (tm / s / em + (1.0 - tm / s) / e_mix)
    cu = open_cap(w, s)
    cu_m = min(cu + (em - 1.0) * (0.5 * cover_cap(w, s, tm) + 0.5 * cover_cap(w, s, tm - t)), em * cu)
    return cu_m / cu, e_slot


def combine(plain, sides, tau):
    c = ca = 0.0
    for s in sides:
        sc, sca = plain if s is None else (smooth_max(s[0], plain[0], tau), smooth_max(s[1], plain[1], tau))
        c += 0.5 * sc
        ca += 0.5 * sca
    return z_of(c, ca)


def cpwg(p, plain, c, slot=None):
    """grounded CPW (microstrip_cpwg): (Z0, εeff); slot: a fixed slot capacitance in place of slot_cap"""
    w, h, t, er, mask = p["w"], p["h"], p["t"], p["er"], p["mask"]

    def side(s):
        if s is None:
            return None
        cl, cu = slab_cap(w, s, h), open_cap(w, s)
        cs = 2.0 * (slot if slot is not None else slot_cap(t, s, c))
        f_up, e_slot = mask_on_slot(w, s, t, mask)
        return er * cl + f_up * cu + e_slot * cs, cl + cu + cs
    return combine(caps_of(plain["z0"], plain["eeff"]), [side(p["sl"]), side(p["sr"])], c["tau"])


def stripline_cpw(p, plain, c, slot=None):
    w, t = p["w"], p["t"]

    def side(s):
        if s is None:
            return None
        a, b = slab_cap(w, s, p["h_up"]), slab_cap(w, s, p["h_dn"])
        cs = 2.0 * (slot if slot is not None else slot_cap(t, s, c))
        return p["e_up"] * a + p["e_dn"] * b + p["e_fill"] * cs, a + b + cs
    return combine(caps_of(plain["z0"], plain["eeff"]), [side(p["sl"]), side(p["sr"])], c["tau"])


def coupled_caps(w, g, gout, h, t, er, mask, c):
    def ke(a, b, cc):
        return math.sqrt((b * b - a * a) / (cc * cc - a * a)) if math.isfinite(cc) else 0.0

    def ko(a, b, cc):
        r = cc * cc / (cc * cc - a * a) if math.isfinite(cc) else 1.0
        return math.sqrt((b * b - a * a) / (b * b) * r)
    x1, x2 = g / 2.0, g / 2.0 + w
    x3 = math.inf if gout is None else x2 + gout
    m = (lambda x: math.tanh(PI * x / (2 * h)) if math.isfinite(x) else 1.0)
    m1, m2, m3 = m(x1), m(x2), m(x3)
    ue, de = k_ratio(ke(x1, x2, x3)), k_ratio(ke(m1, m2, m3))
    uo, do = k_ratio(ko(x1, x2, x3)), k_ratio(ko(m1, m2, m3))
    if gout is not None:
        f_out, e_out = mask_on_slot(w, gout, t, mask)
        s_out = slot_cap(t, gout, c)
    else:
        f_out, e_out, s_out = 1.0, 1.0, 0.0
    f_in, e_in = mask_on_slot(w, g, t, mask)
    s_in = 2.0 * slot_cap(t, g, c)
    return ((f_out * ue + er * de + e_out * s_out, ue + de + s_out),
            (f_in * uo + er * do + e_out * s_out + e_in * s_in, uo + do + s_out + s_in))


def cpwg_pair(p, plain, c):
    """coupled grounded CPW (diff_microstrip_cpwg): ((Zodd, εodd), (Zeven, εeven))"""
    lo = (plain["zdiff"] / 2.0, plain["eeff_odd"])
    le = (plain["zeven"], plain["eeff_even"])
    args = (p["w"], p["g"])
    e1, o1 = coupled_caps(*args, p["s_out"], p["h"], p["t"], p["er"], p["mask"], c)
    e2, o2 = coupled_caps(*args, p["s_partner"], p["h"], p["t"], p["er"], p["mask"], c)

    def join(x, y, lim):
        l = caps_of(*lim)
        return z_of(smooth_max(0.5 * (x[0] + y[0]), l[0], c["tau"]), smooth_max(0.5 * (x[1] + y[1]), l[1], c["tau"]))
    return join(o1, o2, lo), join(e1, e2, le)


def cpw_noplane(p, c):
    """CPW without a plane on a slab (cpw; both gaps equal: no_plane's Z0 is the CPW's)"""
    w, s, h, t, er = p["w"], p["s"], p["h"], p["t"], p["er"]
    k0 = w / (w + 2 * s)
    k1 = math.sinh(PI * w / (4 * h)) / math.sinh(PI * (w + 2 * s) / (4 * h))
    ca0 = 4.0 * k_ratio(k0)
    c0 = ca0 + 2.0 * (er - 1.0) * k_ratio(k1)
    cs = 2.0 * slot_cap(t, s, c)
    return z_of(c0 + cs, ca0 + cs)


def errors(case, c):
    """{quantity: % error of the formula with constants c against the field solution}"""
    p, f, fam = case["p"], case["field"], case["fam"]
    if fam == "cpw_noplane":
        return {"z0": (cpw_noplane(p, c)[0] / f["z0"] - 1) * 100}
    if fam == "cpwg_pair":
        (zo, _), (ze, _) = cpwg_pair(p, case["plain"], c)
        return {"zdiff": (zo / f["zodd"] - 1) * 100, "zeven": (ze / f["zeven"] - 1) * 100}
    fn = stripline_cpw if fam == "stripline_cpw" else cpwg
    return {"z0": (fn(p, case["plain"], c)[0] / f["z0"] - 1) * 100}


# --- the fit --------------------------------------------------------------------------------------------------

def pct(v, q):
    v = sorted(v)
    i = (len(v) - 1) * q
    lo, hi = math.floor(i), math.ceil(i)
    return v[lo] + (v[hi] - v[lo]) * (i - lo)


def stats(v):
    a = [abs(x) for x in v]
    return f"n {len(v):4d}  bias {pct(v, 0.5):+6.2f}  median {pct(a, 0.5):5.2f}  p90 {pct(a, 0.9):5.2f}  max {max(a):6.2f}  mean {sum(a) / len(a):5.2f}"


def mirror_check(cases):
    """the Python formulas at the code's constants against kisi's own formula values"""
    worst = 0.0
    for case in cases:
        p, k, fam = case["p"], case["kisi"], case["fam"]
        if fam == "cpw_noplane":
            got = {"z0": cpw_noplane(p, CODE)[0]}
        elif fam == "cpwg_pair":
            (zo, _), (ze, _) = cpwg_pair(p, case["plain"], CODE)
            got = {"zodd": zo, "zeven": ze}
        else:
            got = {"z0": (stripline_cpw if fam == "stripline_cpw" else cpwg)(p, case["plain"], CODE)[0]}
        for q, v in got.items():
            worst = max(worst, abs(v / k[q] - 1))
    return worst


def fit_corner(cases):
    """per case the slot capacitance that matches the field solution (single lines, both sides, no mask, gap ≤ h/2,
    τ → 0); a·r^b fitted to its excess over r = t/s"""
    pts = []
    c0 = dict(CODE, tau=1e-4)
    for case in cases:
        p, fam = case["p"], case["fam"]
        if fam not in ("cpwg", "stripline_cpw") or p["sl"] is None or p["sl"] != p["sr"]:
            continue
        hmin = p["h"] if fam == "cpwg" else min(p["h_up"], p["h_dn"])
        if p["sl"] > 0.5 * hmin:
            continue
        fn = stripline_cpw if fam == "stripline_cpw" else cpwg
        z = case["field"]["z0"]
        g = (lambda x: fn(p, case["plain"], c0, slot=x)[0] - z)
        lo, hi = 0.0, 50.0
        if g(lo) < 0 or g(hi) > 0:
            continue
        for _ in range(60):
            mid = 0.5 * (lo + hi)
            lo, hi = (mid, hi) if g(mid) > 0 else (lo, mid)
        r = p["t"] / p["sl"]
        pts.append((r, 0.5 * (lo + hi) - r))
    best = None
    for i in range(0, 61):
        b = 0.005 * i
        num = sum(y * r ** b for r, y in pts)
        den = sum(r ** (2 * b) for r, _ in pts)
        a = num / den
        rms = math.sqrt(sum((y - a * r ** b) ** 2 for r, y in pts) / len(pts))
        if best is None or rms < best[2]:
            best = (a, b, rms)
    a, b, rms = best
    code_rms = math.sqrt(sum((y - CODE["corner"] * r ** CODE["corner_exp"]) ** 2 for r, y in pts) / len(pts))
    return a, b, rms, code_rms, pts


def fit_tau(cases, corner, exp):
    single = [c for c in cases if c["fam"] in ("cpwg", "cpwg_mask", "stripline_cpw")]
    rows = []
    for i in range(1, 21):
        tau = 0.005 * i
        c = {"corner": corner, "corner_exp": exp, "tau": tau}
        e = [abs(errors(x, c)["z0"]) for x in single]
        rows.append((tau, sum(e) / len(e), pct(e, 0.5), pct(e, 0.9)))
    return min(rows, key=lambda r: r[1]), rows


def report(doc):
    cases = doc["cases"]
    print(f"{len(cases)} cases (kisi {doc.get('kisi_rev')}, field tol {doc.get('tol')}, {doc.get('date')})")
    if doc.get("constants") and any(abs(doc["constants"][k] - CODE[k]) > 1e-12 for k in CODE):
        print(f"note: the stored formula values are at {doc['constants']}, not the code's {CODE} – run --formula-only")
    m = mirror_check(cases)
    print(f"Python transcription vs kisi's formula values at the code's constants: max |Δ| {m * 100:.2g} %"
          + (" (the stored values have 6 digits)" if m < 2e-5 else "  ← the transcription differs from the code"))
    a, b, rms, code_rms, pts = fit_corner(cases)
    print(f"\n1. corner term a·(t/s)^b per slot, {len(pts)} cases, t/s {min(r for r, _ in pts):.3f} … {max(r for r, _ in pts):.2f}:")
    print(f"   fitted a = {a:.4f}, b = {b:.3f} (rms {rms:.4f});  code a = {CODE['corner']}, b = {CODE['corner_exp']} (rms {code_rms:.4f})")
    (tau, mean, med, p90), rows = fit_tau(cases, CODE["corner"], CODE["corner_exp"])
    print("\n2. τ (code's corner term): mean / median / p90 |error| of Z0, single lines with coplanar copper")
    for r in rows:
        mark = "  ← best" if r[0] == tau else ("  ← code" if abs(r[0] - CODE["tau"]) < 1e-9 else "")
        print(f"   τ {r[0]:.3f}: {r[1]:.3f} / {r[2]:.3f} / {r[3]:.3f}{mark}")
    (tau2, _, _, _), _ = fit_tau(cases, a, b)
    print(f"   fitted τ = {tau:.3f} (with the fitted corner term: {tau2:.3f});  code τ = {CODE['tau']}")
    fitted = {"corner": a, "corner_exp": b, "tau": tau2}
    rounded = {"corner": round(a, 3), "corner_exp": round(b, 3), "tau": round(tau2, 3)}
    same = all(abs(rounded[k] - CODE[k]) < 1e-9 for k in CODE)
    print(f"\n   fitted, rounded as in the code: a {rounded['corner']}, b {rounded['corner_exp']}, τ {rounded['tau']} – "
          + ("the code's constants" if same else f"the code has a {CODE['corner']}, b {CODE['corner_exp']}, τ {CODE['tau']}"))
    single = [c for c in cases if c["fam"] in ("cpwg", "cpwg_mask", "stripline_cpw")]
    mean = (lambda cc: sum(abs(errors(x, cc)["z0"]) for x in single) / len(single))
    print(f"\n   single lines with coplanar copper, mean |error| of Z0: code's constants {mean(CODE):.3f} %, "
          f"fitted ({a:.4f}, {b:.3f}, {tau2:.3f}) {mean(fitted):.3f} %")
    print("\n3. error against kisi's field solver per family (%): code's constants | fitted constants")
    fams = {}
    for c in cases:
        fam = c["fam"] + (("_one_side" if c["p"]["s_partner"] is None else "_both_sides") if c["fam"] == "cpwg_pair" else "")
        for q in errors(c, CODE):
            fams.setdefault((fam, q), []).append(c)
    for (fam, q), cs in sorted(fams.items()):
        e1 = [errors(c, CODE)[q] for c in cs]
        e2 = [errors(c, fitted)[q] for c in cs]
        print(f"   {fam:20s} {q:6s} code:   {stats(e1)}")
        print(f"   {'':20s} {'':6s} fitted: {stats(e2)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--kisi-validate", default=os.path.join(ROOT, "target/release/kisi-validate"))
    ap.add_argument("--from-json", nargs="?", const=OUT, default=None, help="fit the stored sweep (no solving)")
    ap.add_argument("--out", default=OUT)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--chunk", type=int, default=25)
    ap.add_argument("--tol", type=float, default=0.005)
    ap.add_argument("--formula-only", action="store_true", help="recompute kisi's formula values of the stored sweep (--out), "
                    "keep the field solutions, then fit")
    a = ap.parse_args()
    if a.formula_only:
        doc = refresh_formulas(a)
    elif a.from_json:
        doc = json.load(open(a.from_json, encoding="utf-8"))
    else:
        doc = run_sweep(a)
    report(doc)


if __name__ == "__main__":
    main()
