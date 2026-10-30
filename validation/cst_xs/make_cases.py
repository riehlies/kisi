#!/usr/bin/env python3
"""Generate the case matrix of the CST 2D port-mode comparison: validation/cst_xs/cases.json.

    python3 validation/cst_xs/make_cases.py [--kisi-validate target/release/kisi-validate]
        [--examples examples] [--test-boards DIR] [--out validation/cst_xs/cases.json]

Every case is a cross-section as kisi models it: a stackup and a cross-section description
(`spec`, the inputs of crates/kisi-si/src/xsgeom.rs and of the extraction's formula path,
kisi_si::extract::formula_xs). `kisi-validate xs-cases` turns the specs into kisi's geometry text
(`text`, the exact input of the field solver), which CST builds. A few cases kisi cannot express
(broadside-coupled pairs, coplanar lines without a plane, trapezoidal etch) carry a hand-written
text in the same syntax (`text_source: "hand"`) or a CST-side variant of kisi's text (`cst_text`).

Board cases come from `kisi-validate xs-board` on the example boards (sh tools/fetch_examples.sh
examples) and on the test-board coupons (branch feat/test-board, validation/test_board/boards: pass
the directory with --test-boards). Without them the board cases of the existing cases.json are kept.

Each case: {id, group, tier, title, params, spec?, text, text_source, cst_text?, domain, ref?,
dispersion?, cst?}. `tier` orders the runs: 1 = pilot and anchors (run first), 2 = the main
sweeps, 3 = extended sweeps and the remaining board cross-sections. `domain` is the solver box of
the port size rule (xsgeo.domain; README.md). See README.md for the groups.
"""
import argparse, json, math, os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import xsgeo  # noqa: E402

CU = 0.035  # reference plane copper (mm)


def tand_for(er):
    """a typical loss tangent for the permittivity (only the lossy dispersion runs use it)"""
    for lim, td in ((2.5, 0.0009), (3.1, 0.0013), (3.7, 0.0037), (4.7, 0.02), (6.5, 0.0027)):
        if er <= lim:
            return td
    return 0.0023


def stack(copper, diels, mask_top=None):
    """copper: thicknesses top→bottom; diels: [(kind, h, er)]"""
    return {
        "copper": [{"name": f"L{i + 1}", "thickness": t} for i, t in enumerate(copper)],
        "dielectrics": [{"kind": k, "thickness": h, "epsilon_r": er, "loss_tangent": tand_for(er)} for k, h, er in diels],
        "mask_top": {"thickness": mask_top[0], "epsilon_r": mask_top[1]} if mask_top else None,
        "mask_bottom": None,
    }


def desc(layer, w, ref_up=None, ref_dn=None, s=None, gap=None, mask=None, planes=None, wp=None):
    """s: pair gap (partner on the left, width wp or w); gap: coplanar gap from the outer edges"""
    partner = [s, wp or w, True] if s is not None else None
    s_left = None if gap is None else gap + ((s + (wp or w)) if s is not None else 0.0)
    return {"layer": layer, "w": w, "ref_up": ref_up, "ref_dn": ref_dn, "s_left": s_left, "s_right": gap,
            "partner": partner, "mask": list(mask) if mask else None, "planes": planes or [], "extra": [], "trace_gnd": False}


def spec_ms(w, h, t, er, mask=None, s=None, gap=None):
    return {"stackup": stack([t, CU], [("core", h, er)], mask), "desc": desc(0, w, ref_dn=1, s=s, gap=gap, mask=mask)}


def spec_embedded(w, h, t, er, d, er_cover=None):
    """trace on L2 under a cover of d (to the copper band of L1, resin-filled) – kisi's covered trace"""
    return {"stackup": stack([CU, t, CU], [("prepreg", d, er_cover or er), ("core", h, er)]),
            "desc": desc(1, w, ref_dn=2)}


def spec_sl(w, h1, h2, t, er1, er2=None, s=None, gap=None):
    """h1: dielectric below the trace, h2: dielectric between the trace top and the upper plane"""
    return {"stackup": stack([CU, t, CU], [("prepreg", h2, er2 or er1), ("core", h1, er1)]),
            "desc": desc(1, w, ref_up=0, ref_dn=2, s=s, gap=gap)}


def spec_dual(w, ha, hm, hb, t, er, s=None):
    """dual stripline: trace on L2 between planes L1 and L4, L3 a signal layer (empty here)"""
    return {"stackup": stack([CU, t, t, CU], [("prepreg", ha, er), ("core", hm, er), ("prepreg", hb, er)]),
            "desc": desc(1, w, ref_up=0, ref_dn=3, s=s)}


def spec_cut(w, h, t, er, cut, h2):
    """microstrip over an opening of width `cut` in L2; reference L3, h2 further down"""
    return {"stackup": stack([t, CU, CU], [("core", h, er), ("core", h2, er)]),
            "desc": desc(0, w, ref_dn=2, planes=[[1, cut / 2, cut / 2]])}


def spec_edge(w, h, t, er, r, s=None):
    """microstrip whose reference plane ends r right of the trace centre"""
    d = desc(0, w, ref_dn=1, s=s, planes=[[1, None, r]])
    return {"stackup": stack([t, CU], [("core", h, er)]), "desc": d}


def spec_cpw(w, h, t, er, gap):
    """coplanar line without a plane (kisi: formula only, no field geometry)"""
    return {"stackup": stack([t, CU], [("core", h, er)]), "desc": desc(0, w, gap=gap)}


f = xsgeo.fmt


def text_cpw(w, h, t, er, gap):
    return (f"units mm\nbounds open open open open\ndiel x=-inf w=inf y=0 h={f(h)} er={f(er)} tand=0\n"
            f"gnd x={f(-w / 2 - gap)} w=-inf y={f(h)} h={f(t)}\ngnd x={f(w / 2 + gap)} w=inf y={f(h)} h={f(t)}\n"
            f"sig+ x={f(-w / 2)} w={f(w)} y={f(h)} h={f(t)}\n")


def text_broadside(w, h1, hm, h2, t, er, off=0.0):
    """broadside-coupled stripline: traces on two layers hm apart (dielectric between the copper
    faces), planes h1 below the lower and h2 above the upper trace, lateral offset `off`"""
    y1 = CU + h1
    y2 = y1 + t + hm
    top = y2 + t + h2
    return (f"units mm\nbounds open open open open\ndiel x=-inf w=inf y={f(CU)} h={f(top - CU)} er={f(er)} tand=0\n"
            f"gnd x=-inf w=inf y=0 h={f(CU)}\ngnd x=-inf w=inf y={f(top)} h={f(CU)}\n"
            f"sig- x={f(-w / 2 - off / 2)} w={f(w)} y={f(y1)} h={f(t)}\nsig+ x={f(-w / 2 + off / 2)} w={f(w)} y={f(y2)} h={f(t)}\n")


def trap_variant(text, ef):
    """the CST-side text of the etch group: every signal as a trapezoid with etch factor ef
    (t / undercut per side; base = kisi's width at the bottom face, the face at y+h narrower)"""
    out = []
    ang = math.degrees(math.atan(1.0 / ef))
    for line in text.splitlines():
        if line.startswith("sig"):
            k, rest = line.split(" ", 1)
            line = f"{k} trap {rest} angle={f(ang)}"
        out.append(line)
    return "\n".join(out) + "\n"


class Cases:
    def __init__(self):
        self.list = []
        self.ids = set()

    def add(self, cid, group, tier, title, params, spec=None, text=None, ref=None, **kw):
        cid = cid.replace(" ", "")
        if cid in self.ids:
            return
        self.ids.add(cid)
        c = {"id": cid, "group": group, "tier": tier, "title": title, "params": params}
        if spec:
            c["spec"] = spec
        if text:
            c["text"] = text
            c["text_source"] = "hand"
        if ref:
            c["ref"] = ref
        c.update(kw)
        self.list.append(c)


def g(v):
    return f"{v:g}"


# --- groups ---------------------------------------------------------------------------------------

def anchors(cs):
    """Forstén's HFSS/Ansys cases, the ThunderScope cross-sections and the asymmetric striplines
    with references from Forstén's own MS/SL solver (validation/*.json), built as kisi-validate's
    case_geometry builds them (same text)"""
    def from_validation(name, reffile=None, tier=1, src=None):
        cases = json.load(open(os.path.join(ROOT, "validation", name)))
        refs = json.load(open(os.path.join(ROOT, "validation", reffile))) if reffile else [None] * len(cases)
        for k, (c, r) in enumerate(zip(cases, refs)):
            ty = c["type"]
            w, h, t, er = c["w"], c["h"], c["t"], c["er"]
            mask = (c["sm_t"], c.get("sm_er", 3.3)) if "sm_t" in c else None
            s = c.get("s")
            gap = c.get("gap")
            if ty in ("microstrip", "gcpw", "diff_microstrip", "diff_gcpw"):
                spec = {"stackup": stack([t, CU], [("core", h, er)], mask), "desc": desc(0, w, ref_dn=1, s=s, gap=gap, mask=mask)}
            elif ty in ("stripline", "diff_stripline"):
                spec = spec_sl(w, h, c["top_h"] - t, t, er, c.get("er_top", er), s=s)
            elif ty == "microstrip_cut":
                spec = spec_cut(w, h, t, er, c["cut"], c["cut_sub_h"])
            elif ty == "embedded_microstrip":
                # kisi has no geometry for a dielectric cover on a surface trace: the covered trace
                # as kisi models it (inner layer, cover to the resin-filled copper band above)
                spec = spec_embedded(w, h, t, er, c["cover"] - CU)
            else:
                continue
            for d in spec["stackup"]["dielectrics"]:
                d["loss_tangent"] = 0.0
            ref = dict(c.get("ref") or {})
            if r:
                ref.update({k2: v for k2, v in r.items() if k2 in ("z0", "zdiff", "zodd", "zeven", "eeff")})
            if ref:
                ref["source"] = c.get("source") or src
            cid = f"anchor_{name.split('.')[0]}_{k + 1}"
            cs.add(cid, "anchor", tier, c["name"], {"type": ty, **{k2: c[k2] for k2 in c if k2 not in ("name", "ref", "source", "note", "type")}},
                   spec=spec, ref=ref or None, dispersion=name == "forsten_suite.json")
    from_validation("forsten_suite.json")
    from_validation("thunderscope_xs.json", "thunderscope_xs.ref.json", src="Forstén MS/SL solver")
    from_validation("stripline_asym.json", "stripline_asym.ref.json", src="Forstén MS/SL solver")


def microstrip(cs):
    # w/h × εr at t = 35 µm, h = 0.3 mm
    h, t = 0.3, 0.035
    for u in (0.1, 0.2, 0.35, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0):
        for er in (2.2, 3.0, 3.66, 4.4, 6.15, 10.2):
            tier = 1 if (er == 4.4 and u in (0.5, 2.0)) else 2 if (u in (0.2, 0.5, 1.0, 2.0, 5.0) or er in (3.66, 4.4)) else 3
            cs.add(f"ms_u{g(u)}_er{g(er)}", "microstrip", tier, f"microstrip w/h {g(u)}, εr {g(er)}, h 0.3 mm, t 35 µm",
                   {"w": u * h, "h": h, "t": t, "er": er}, spec=spec_ms(u * h, h, t, er), dispersion=(er == 4.4 and u in (0.5, 2.0)) or (er == 3.66 and u == 2.0))
    # copper thickness × w/h (εr 4.4, h 0.2 mm)
    h = 0.2
    for t in (0.0175, 0.035, 0.07, 0.105):
        for u in (0.25, 0.5, 1.0, 2.0, 4.0):
            cs.add(f"ms_t{g(t * 1000)}_u{g(u)}", "microstrip_t", 2 if u in (0.5, 2.0) else 3,
                   f"microstrip t {g(t * 1000)} µm, w/h {g(u)}, h 0.2 mm, εr 4.4", {"w": u * h, "h": h, "t": t, "er": 4.4}, spec=spec_ms(u * h, h, t, 4.4))
    # substrate height (thin cores to 1.6 mm), w/h for ~50 Ω and two others
    for h in (0.07, 0.1, 0.2, 0.8, 1.6):
        for u in (0.5, 1.9, 4.0):
            cs.add(f"ms_h{g(h)}_u{g(u)}", "microstrip_h", 2 if u == 1.9 else 3, f"microstrip h {g(h)} mm, w/h {g(u)}, t 35 µm, εr 4.4",
                   {"w": u * h, "h": h, "t": 0.035, "er": 4.4}, spec=spec_ms(u * h, h, 0.035, 4.4), dispersion=(u == 1.9 and h in (0.2, 1.6)))


def masked(cs):
    h, t, er = 0.2, 0.035, 4.4
    for u in (0.5, 1.0, 2.0, 4.0):
        for tm in (0.01, 0.02, 0.035):
            for em in (3.3, 4.0):
                tier = 1 if (u == 2.0 and tm == 0.02 and em == 3.3) else 2 if em == 3.3 else 3
                cs.add(f"msm_u{g(u)}_m{g(tm * 1000)}_er{g(em)}", "microstrip_mask", tier,
                       f"microstrip w/h {g(u)} with mask {g(tm * 1000)} µm εr {g(em)} (h 0.2 mm, εr 4.4)",
                       {"w": u * h, "h": h, "t": t, "er": er, "sm_t": tm, "sm_er": em}, spec=spec_ms(u * h, h, t, er, mask=(tm, em)))
    # thick copper under mask
    for t2 in (0.07, 0.105):
        cs.add(f"msm_t{g(t2 * 1000)}_u2", "microstrip_mask", 3, f"microstrip t {g(t2 * 1000)} µm, w/h 2, mask 20 µm εr 3.3",
               {"w": 0.4, "h": h, "t": t2, "er": er, "sm_t": 0.02, "sm_er": 3.3}, spec=spec_ms(0.4, h, t2, er, mask=(0.02, 3.3)))


def embedded(cs):
    h, t = 0.2, 0.035
    for u in (0.5, 1.0, 2.0):
        for d in (0.05, 0.1, 0.2, 0.5):
            for er in (3.66, 4.4):
                tier = 1 if (u == 1.0 and d == 0.1 and er == 4.4) else 2 if er == 4.4 else 3
                cs.add(f"emb_u{g(u)}_d{g(d)}_er{g(er)}", "embedded", tier,
                       f"covered trace w/h {g(u)}, cover {g(d)} mm + copper band, εr {g(er)}",
                       {"w": u * h, "h": h, "t": t, "er": er, "cover": d}, spec=spec_embedded(u * h, h, t, er, d))


def stripline(cs):
    t = 0.0175
    # symmetric: w/h × h × εr
    for h1 in (0.1, 0.2):
        for u in (0.25, 0.5, 1.0, 2.0, 4.0):
            for er in (3.0, 4.2):
                tier = 1 if (h1 == 0.1 and u == 1.0 and er == 4.2) else 2 if er == 4.2 else 3
                cs.add(f"sl_h{g(h1)}_u{g(u)}_er{g(er)}", "stripline", tier, f"symmetric stripline w/h {g(u)}, h {g(h1)} mm each side, εr {g(er)}",
                       {"w": u * h1, "h1": h1, "h2": h1, "t": t, "er": er}, spec=spec_sl(u * h1, h1, h1 - t, t, er),
                       dispersion=(h1 == 0.1 and u == 1.0 and er == 4.2))
    # copper thickness
    for tt in (0.0175, 0.035, 0.07):
        for u in (0.5, 1.0, 2.0):
            cs.add(f"sl_t{g(tt * 1000)}_u{g(u)}", "stripline", 2 if u == 1.0 else 3, f"symmetric stripline t {g(tt * 1000)} µm, w/h {g(u)}, h 0.15 mm, εr 4.2",
                   {"w": u * 0.15, "h1": 0.15, "h2": 0.15, "t": tt, "er": 4.2}, spec=spec_sl(u * 0.15, 0.15, 0.15 - tt, tt, 4.2))
    # asymmetric (offset): h2/h1 × w/h1, one εr
    for r in (1.5, 2.0, 4.0, 8.0):
        for u in (0.5, 1.0, 2.0):
            cs.add(f"sla_r{g(r)}_u{g(u)}", "stripline_asym", 1 if (r == 4.0 and u == 1.0) else 2,
                   f"offset stripline h2/h1 {g(r)}, w/h1 {g(u)}, h1 0.1 mm, εr 4.0", {"w": u * 0.1, "h1": 0.1, "h2": r * 0.1, "t": t, "er": 4.0},
                   spec=spec_sl(u * 0.1, 0.1, r * 0.1, t, 4.0))
    # different εr above / below (core and prepreg)
    for (e1, e2) in ((4.5, 3.66), (3.66, 4.5), (3.0, 4.4), (4.4, 2.2)):
        for r in (1.0, 3.0):
            cs.add(f"sle_er{g(e1)}-{g(e2)}_r{g(r)}", "stripline_asym", 2 if r == 3.0 else 3,
                   f"stripline εr {g(e1)} below / {g(e2)} above, h2/h1 {g(r)}", {"w": 0.12, "h1": 0.12, "h2": r * 0.12, "t": t, "er1": e1, "er2": e2},
                   spec=spec_sl(0.12, 0.12, r * 0.12, t, e1, e2))
    # dual stripline (second signal layer empty)
    for hm in (0.1, 0.2, 0.4):
        for u in (0.75, 1.5):
            cs.add(f"dsl_hm{g(hm)}_u{g(u)}", "stripline_dual", 2 if u == 1.5 else 3,
                   f"dual stripline: trace on L2, L3 empty (resin band), core {g(hm)} mm, w/h {g(u)}", {"w": u * 0.1, "ha": 0.1, "hm": hm, "hb": 0.1, "t": t, "er": 4.2},
                   spec=spec_dual(u * 0.1, 0.1, hm, 0.1, t, 4.2))


def diff_pairs(cs):
    h, t, er = 0.2, 0.035, 4.4
    for u in (0.5, 1.0, 2.0):
        for sh in (0.25, 0.5, 1.0, 2.0, 4.0):
            for m in (None, (0.02, 3.3)):
                tier = 1 if (u == 1.0 and sh == 1.0) else 2 if (m is None or sh in (0.5, 1.0)) else 3
                tag = "_mask" if m else ""
                cs.add(f"dms_u{g(u)}_s{g(sh)}{tag}", "diff_microstrip", tier,
                       f"edge-coupled microstrip w/h {g(u)}, s/h {g(sh)}{' with mask 20 µm' if m else ''} (h 0.2 mm, εr 4.4)",
                       {"w": u * h, "s": sh * h, "h": h, "t": t, "er": er, **({"sm_t": m[0], "sm_er": m[1]} if m else {})},
                       spec=spec_ms(u * h, h, t, er, mask=m, s=sh * h), dispersion=(u == 1.0 and sh == 1.0 and m is None))
    for er2 in (3.0, 3.66):
        for sh in (0.5, 2.0):
            cs.add(f"dms_er{g(er2)}_s{g(sh)}", "diff_microstrip", 3, f"edge-coupled microstrip εr {g(er2)}, w/h 1, s/h {g(sh)}",
                   {"w": 0.2, "s": sh * 0.2, "h": 0.2, "t": t, "er": er2}, spec=spec_ms(0.2, 0.2, t, er2, s=sh * 0.2))
    t = 0.0175
    for u in (0.5, 1.0):
        for sh in (0.25, 0.5, 1.0, 2.0, 4.0):
            for r in (1.0, 3.0):
                tier = 1 if (u == 1.0 and sh == 1.0 and r == 1.0) else 2
                cs.add(f"dsl_u{g(u)}_s{g(sh)}_r{g(r)}", "diff_stripline", tier,
                       f"edge-coupled stripline w/h1 {g(u)}, s/h1 {g(sh)}, h2/h1 {g(r)} (h1 0.12 mm, εr 4.2)",
                       {"w": u * 0.12, "s": sh * 0.12, "h1": 0.12, "h2": r * 0.12, "t": t, "er": 4.2},
                       spec=spec_sl(u * 0.12, 0.12, r * 0.12, t, 4.2, s=sh * 0.12), dispersion=(u == 1.0 and sh == 1.0 and r == 1.0))
    # broadside-coupled stripline (kisi models pairs on one layer only: solver check, no kisi model)
    for hm in (0.1, 0.2):
        for off in (0.0, 0.15):
            for w in (0.1, 0.2):
                cs.add(f"bsl_hm{g(hm)}_off{g(off)}_w{g(w)}", "broadside", 2 if off == 0.0 else 3,
                       f"broadside-coupled stripline w {g(w)}, layers {g(hm)} mm apart, offset {g(off)} (planes 0.15 mm away, εr 4.2)",
                       {"w": w, "hm": hm, "off": off, "h1": 0.15, "h2": 0.15, "t": t, "er": 4.2},
                       text=text_broadside(w, 0.15, hm, 0.15, t, 4.2, off), kisi_model=False)


def coplanar(cs):
    h, t, er = 0.2, 0.035, 4.4
    for u in (0.5, 1.0, 2.0):
        for gh in (0.25, 0.5, 1.0, 2.0, 4.0):
            for m in (None, (0.02, 3.3)):
                tier = 1 if (u == 1.0 and gh == 0.5 and m is None) else 2 if m is None or gh <= 1.0 else 3
                tag = "_mask" if m else ""
                cs.add(f"cpwg_u{g(u)}_g{g(gh)}{tag}", "cpwg", tier, f"grounded coplanar w/h {g(u)}, gap/h {g(gh)}{' with mask 20 µm' if m else ''} (h 0.2 mm, εr 4.4)",
                       {"w": u * h, "gap": gh * h, "h": h, "t": t, "er": er, **({"sm_t": m[0], "sm_er": m[1]} if m else {})},
                       spec=spec_ms(u * h, h, t, er, mask=m, gap=gh * h), dispersion=(u == 1.0 and gh == 0.5 and m is None))
    for sh in (0.5, 1.0):
        for gh in (0.5, 1.0, 2.0):
            cs.add(f"dcpwg_s{g(sh)}_g{g(gh)}", "diff_cpwg", 2 if gh == 1.0 else 3, f"coupled pair with coplanar ground w/h 1, s/h {g(sh)}, gap/h {g(gh)}",
                   {"w": 0.2, "s": sh * h, "gap": gh * h, "h": h, "t": t, "er": er}, spec=spec_ms(0.2, h, t, er, s=sh * h, gap=gh * h))
    # stripline with coplanar ground on the trace layer (the formula ignores it: F_COPLANAR_IGNORED)
    for gh in (0.5, 1.0, 2.0):
        cs.add(f"slcpw_g{g(gh)}", "stripline_cpw", 2, f"stripline with coplanar ground, gap/h {g(gh)} (w 0.1, h 0.1 mm each side, εr 4.2)",
               {"w": 0.1, "gap": gh * 0.1, "h1": 0.1, "h2": 0.1, "t": 0.0175, "er": 4.2}, spec=spec_sl(0.1, 0.1, 0.1 - 0.0175, 0.0175, 4.2, gap=gh * 0.1))
    # coplanar without a plane: kisi's no-plane formula (an estimate) against the solvers
    for gap in (0.1, 0.15, 0.25, 0.4):
        for w in (0.3, 0.6):
            cs.add(f"cpw_w{g(w)}_g{g(gap)}", "cpw_noplane", 2 if w == 0.3 else 3, f"coplanar without plane w {g(w)}, gap {g(gap)} on 1.6 mm, εr 4.4",
                   {"w": w, "gap": gap, "h": 1.6, "t": 0.035, "er": 4.4}, spec=spec_cpw(w, 1.6, 0.035, 4.4, gap),
                   text=text_cpw(w, 1.6, 0.035, 4.4, gap))


def plane_openings(cs):
    h, t, er = 0.2, 0.035, 4.4
    for w in (0.2, 0.4):
        for cw in (1.0, 2.0, 4.0, 8.0):
            for r in (1.0, 3.0):
                cut = cw * w
                tier = 1 if (w == 0.4 and cw == 2.0 and r == 1.0) else 2 if cw in (1.0, 4.0) else 3
                cs.add(f"cut_w{g(w)}_c{g(cw)}_r{g(r)}", "plane_cut", tier,
                       f"microstrip over a {g(cut)} mm opening in L2, reference L3 {g(r * h)} mm further down (w {g(w)}, h 0.2 mm)",
                       {"w": w, "cut": cut, "h": h, "h2": r * h, "t": t, "er": er}, spec=spec_cut(w, h, t, er, cut, r * h))
    for w in (0.2, 0.4):
        for rr in (0.5, 1.0, 2.0, 4.0, 8.0):
            r = w / 2 + rr * h
            cs.add(f"edge_w{g(w)}_r{g(rr)}", "plane_edge", 1 if (w == 0.4 and rr == 1.0) else 2,
                   f"microstrip, reference plane ends {g(rr)}·h beyond the trace edge (w {g(w)}, h 0.2 mm)",
                   {"w": w, "edge_from_trace_edge": rr * h, "h": h, "t": t, "er": er}, spec=spec_edge(w, h, t, er, r))
    for rr in (0.5, 2.0):
        r = 0.2 / 2 + rr * h
        cs.add(f"edge_pair_r{g(rr)}", "plane_edge", 3, f"pair w 0.2 s 0.2, plane ends {g(rr)}·h beyond the right trace edge",
               {"w": 0.2, "s": 0.2, "edge_from_trace_edge": rr * h, "h": h, "t": t, "er": er}, spec=spec_edge(0.2, h, t, er, r, s=0.2))


def etch(cs):
    """trapezoidal etch: CST builds trapezoids (etch factor EF = t / undercut per side, base at the
    bottom face = kisi's width), kisi solves its rectangle – the error of kisi's rectangular model"""
    for t in (0.035, 0.07):
        for u in (0.5, 1.0, 2.0):
            for ef in (1.0, 2.0, 3.0):
                if t == 0.07 and u == 0.5:
                    continue
                cs.add(f"etch_ms_t{g(t * 1000)}_u{g(u)}_ef{g(ef)}", "etch", 2 if ef == 2.0 else 3,
                       f"microstrip t {g(t * 1000)} µm, w/h {g(u)}, etch factor {g(ef)} (CST trapezoid, kisi rectangle)",
                       {"w": u * 0.2, "h": 0.2, "t": t, "er": 4.4, "etch_factor": ef}, spec=spec_ms(u * 0.2, 0.2, t, 4.4), etch_factor=ef)
    for ef in (1.0, 2.0, 3.0):
        cs.add(f"etch_sl_ef{g(ef)}", "etch", 2 if ef == 2.0 else 3, f"stripline t 35 µm, w 0.12, etch factor {g(ef)}",
               {"w": 0.12, "h1": 0.12, "h2": 0.12, "t": 0.035, "er": 4.2, "etch_factor": ef},
               spec=spec_sl(0.12, 0.12, 0.12 - 0.035, 0.035, 4.2), etch_factor=ef)
        cs.add(f"etch_dms_ef{g(ef)}", "etch", 2 if ef == 2.0 else 3, f"pair w 0.2 s 0.2 h 0.2, t 35 µm, etch factor {g(ef)}",
               {"w": 0.2, "s": 0.2, "h": 0.2, "t": 0.035, "er": 4.4, "etch_factor": ef}, spec=spec_ms(0.2, 0.2, 0.035, 4.4, s=0.2), etch_factor=ef)


SIZE_STUDY = ["ms_u0.5_er4.4", "ms_h1.6_u1.9", "ms_u10_er4.4", "sla_r8_u1", "dms_u1_s4", "cpwg_u1_g4", "edge_w0.4_r8", "cut_w0.4_c8_r3"]


def size_study(cs):
    """the domain (port size) rule checked: a few cases again with 1.5× and 2× the margins"""
    base = {c["id"]: c for c in cs.list}
    for cid in SIZE_STUDY:
        c = base.get(cid)
        if not c:
            continue
        if cid in SIZE_STUDY[:2]:
            c["tier"] = 1  # the base case runs with its tier-1 variants
        for sc in (1.5, 2.0):
            cs.add(f"{cid}__m{g(sc)}", "size_study", 1 if cid in SIZE_STUDY[:2] else 2, f"{c['title']} – domain margins × {g(sc)}",
                   dict(c["params"], margin_scale=sc), spec=c.get("spec"), text=c.get("text") if c.get("text_source") == "hand" else None,
                   margin_scale=sc, base=cid)


# --- boards ---------------------------------------------------------------------------------------

def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)}: {r.stderr or r.stdout}")
    return r.stdout


def board_cases(cs, kv, boards, old):
    """impedance cross-sections kisi extracts on real boards: the geometries of nets with a target
    impedance by routed length (the first N_TARGET in tier 2, the next N_TARGET in tier 3; test
    boards: all) and the longest others up to N_OTHER per board (tier 3)"""
    N_TARGET, N_OTHER = 30, 8
    seen_text = {}
    have_any = False
    tmp = tempfile.mkdtemp(prefix="cstxs_")
    for tag, pcb, cfg, label in boards:
        if not os.path.isfile(pcb):
            print(f"  {label}: {pcb} not found, board cases kept from the existing cases.json", file=sys.stderr)
            for c in old:
                if c.get("board") == label:
                    cs.add(c["id"], c["group"], c["tier"], c["title"], c["params"], spec=c.get("spec"), ref=c.get("ref"),
                           board=label, nets=c.get("nets"), classes=c.get("classes"), length_mm=c.get("length_mm"),
                           target_length_mm=c.get("target_length_mm"), geom_id=c.get("geom_id"), dispersion=c.get("dispersion", False))
            continue
        have_any = True
        out = os.path.join(tmp, tag + ".json")
        cmd = [kv, "xs-board", pcb, "--out", out] + (["--config", cfg] if cfg else ["--no-config"])
        print(" ", run(cmd).strip())
        geoms = json.load(open(out))
        tgt = sorted([x for x in geoms if x["target_length"] > 0], key=lambda x: -x["target_length"])
        test_board = tag.startswith("tb")
        if not test_board:
            tgt = tgt[:2 * N_TARGET]
        oth = sorted([x for x in geoms if x["target_length"] <= 0], key=lambda x: -x["length"])[:N_OTHER]
        for rank, x in enumerate(tgt + oth):
            if not x.get("spec"):
                print(f"  {label}: geometry {x['id']} not reproducible from its description, skipped", file=sys.stderr)
                continue
            if x["text"] in seen_text:
                seen_text[x["text"]]["boards_also"] = seen_text[x["text"]].get("boards_also", []) + [label]
                continue
            d = x["spec"]["desc"]
            kind = x["formula"]["kind"]
            what = ("pair " if d["partner"] else "") + kind.replace("_", " ") + (" cpw" if d["s_left"] or d["s_right"] else "")
            tier = 2 if x["target_length"] > 0 and (test_board or rank < N_TARGET) else 3
            c = {"board": label, "nets": x["nets"], "classes": x["classes"], "length_mm": round(x["length"], 2),
                 "target_length_mm": round(x["target_length"], 2), "geom_id": x["id"], "dispersion": rank < 2 and tag in ("ts", "vna")}
            group = "test_board" if test_board else "board_target" if x["target_length"] > 0 else "board_other"
            cs.add(f"board_{tag}_{x['id'][:10]}", group, tier,
                   f"{label} {x['layer_name']}: {what} w {g(round(d['w'], 4))} ({', '.join(x['classes'][:2])}; {', '.join(x['nets'][:2])})",
                   {"layer": x["layer_name"], "w": d["w"], "pair_gap": d["partner"][0] if d["partner"] else None,
                    "s_left": d["s_left"], "s_right": d["s_right"], "flags": x["flags"]}, spec=x["spec"], **c)
            seen_text[x["text"]] = cs.list[-1]
    return have_any


def boards_list(examples, test_boards):
    b = [("ts", os.path.join(examples, "Thunderscope_Rev5.3.kicad_pcb"), os.path.join(ROOT, "web/samples/ts.kisi.json"), "ThunderScope Rev5.3"),
         ("vna", os.path.join(examples, "VNA.kicad_pcb"), os.path.join(ROOT, "web/samples/vna.kisi.json"), "LibreVNA")]
    tb = test_boards or ""
    names = sorted(n for n in os.listdir(tb) if n.endswith(".kicad_pcb")) if tb and os.path.isdir(tb) else []
    for n in names:
        stem = n[:-len(".kicad_pcb")]
        cfg = os.path.join(tb, stem + ".kisi.json")
        tag = "tb_" + stem.replace("kisi-coupons-", "").replace("-", "_")
        b.append((tag, os.path.join(tb, n), cfg if os.path.isfile(cfg) else None, "test board " + stem.replace("kisi-coupons-", "")))
    if not names:
        b.append(("tb", "", None, None))  # marker: keep the old test-board cases
    return b


# --- main -----------------------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--kisi-validate", default=os.path.join(ROOT, "target/release/kisi-validate"))
    ap.add_argument("--examples", default=os.path.join(ROOT, "examples"))
    ap.add_argument("--test-boards", default="")
    ap.add_argument("--out", default=os.path.join(HERE, "cases.json"))
    a = ap.parse_args()
    old = []
    if os.path.isfile(a.out):
        old = json.load(open(a.out)).get("cases", [])
    cs = Cases()
    for fn in (anchors, microstrip, masked, embedded, stripline, diff_pairs, coplanar, plane_openings, etch):
        fn(cs)
    size_study(cs)
    blist = boards_list(a.examples, a.test_boards)
    if blist[-1][0] == "tb":  # no test boards given: keep the old test-board cases
        blist.pop()
        for c in old:
            if str(c.get("board", "")).startswith("test board"):
                cs.add(c["id"], c["group"], c["tier"], c["title"], c["params"], spec=c.get("spec"), board=c["board"],
                       nets=c.get("nets"), classes=c.get("classes"), length_mm=c.get("length_mm"),
                       target_length_mm=c.get("target_length_mm"), geom_id=c.get("geom_id"), dispersion=c.get("dispersion", False))
    board_cases(cs, a.kisi_validate, blist, old)

    # kisi's geometry text for every spec
    tmp = tempfile.mkdtemp(prefix="cstxs_")
    pin = os.path.join(tmp, "specs.json")
    pout = os.path.join(tmp, "texts.json")
    json.dump([{"id": c["id"], "spec": c["spec"]} for c in cs.list if c.get("spec")], open(pin, "w"))
    print(" ", run([a.kisi_validate, "xs-cases", pin, "--out", pout]).strip())
    texts = {r["key"]: r for r in json.load(open(pout))}
    out = []
    for c in cs.list:
        r = texts.get(c["id"])
        if r and r.get("error"):
            raise SystemExit(f"{c['id']}: {r['error']}")
        if c.get("text_source") != "hand":
            if not r or not r.get("text"):
                print(f"  {c['id']}: kisi writes no geometry, dropped", file=sys.stderr)
                continue
            c["text"] = r["text"]
            c["text_source"] = "kisi"
        if c.get("etch_factor"):
            c["cst_text"] = trap_variant(c["text"], c["etch_factor"])
        geo = xsgeo.parse(c.get("cst_text") or c["text"])
        dom = xsgeo.domain(geo, c.get("margin_scale", 1.0))
        c["domain"] = {k: (round(v, 6) if isinstance(v, float) else v) for k, v in dom.items()}
        c["domain"]["box"] = [round(v, 6) for v in dom["box"]]
        c["n_signals"] = sum(1 for r2 in geo["rects"] if r2["kind"] in ("sig+", "sig-"))
        out.append(c)
    # stable order: tier, then group order of generation
    out.sort(key=lambda c: c["tier"])
    groups = {}
    for c in out:
        gk = groups.setdefault(c["group"], {"n": 0, "tier": {}})
        gk["n"] += 1
        gk["tier"][str(c["tier"])] = gk["tier"].get(str(c["tier"]), 0) + 1
    doc = {
        "about": "CST 2D port-mode comparison cases, generated by validation/cst_xs/make_cases.py (README.md there). "
                 "text = kisi's geometry (js_2d_fields custom-geometry syntax, mm); cst_text = what CST builds instead "
                 "(etch group: trapezoids); domain.box = solver box [x0, x1, y0, y1] by the port size rule.",
        "port_rule": {"margin_x": xsgeo.MARGIN_X, "margin_y": xsgeo.MARGIN_Y, "margin_min_mm": xsgeo.MARGIN_MIN},
        "counts": {"total": len(out), "tier": {t: sum(1 for c in out if c["tier"] == t) for t in (1, 2, 3)},
                   "dispersion": sum(1 for c in out if c.get("dispersion")), "groups": groups},
        "cases": out,
    }
    with open(a.out, "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=1, ensure_ascii=False)
        fh.write("\n")
    print(f"{len(out)} cases → {os.path.relpath(a.out, ROOT)}; tiers {doc['counts']['tier']}, dispersion {doc['counts']['dispersion']}")
    for k, v in groups.items():
        print(f"  {k:18s} {v['n']:4d}  {v['tier']}")


if __name__ == "__main__":
    main()
