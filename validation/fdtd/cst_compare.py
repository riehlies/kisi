"""Compare CST results (via_ref/<case>/cst.sNp) with kisi-fdtd (kisi.sNp) and, where present,
openEMS (openems.sNp), and the former kisi-fdtd reference (kisi_base.sNp) where the reference was recomputed on a
refined mesh (TESTS.md section 17): raw S-parameters at the ports, the via de-embedded with the 2×thru of each
feed layer (same NZC method as via_pbm.py / kisi_deembedded.s2p, the thru taken from the same
solver), mixed mode for pairs, TDR. Prints a table and writes results/cst_compare.json.

usage: python3 cst_compare.py [case-id substrings ...] [--fmax-cmp 20e9] [--md FILE] [--openems DIR]

--openems DIR: another via_ref tree with openEMS results (openems.sNp); default: via_ref itself.
"""
import json, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import sparam  # noqa: E402

REF = os.path.join(HERE, "..", "via_ref")
OEMS = REF  # where openems.sNp is read from (--openems)
CASE_DIRS = ("via_cases", "board_cases", "resonance_cases", "return_cases")


def load(cid, tool):
    for n in (1, 2, 4):
        p = os.path.join(OEMS if tool.startswith("openems") else REF, cid, f"{tool}.s{n}p")
        if os.path.exists(p):
            return sparam.read_touchstone(p)
    return None


def thru_names(c):
    if c["meta"].get("thru"):  # board vias: their own 2x-thrus, in and out
        return list(c["meta"]["thru"])
    return [f"thru_{c['meta']['stackup']}_{pp['layer']}" for pp in c["meta"]["ports"]]


def case_files():
    out = {}
    for d in CASE_DIRS:
        dd = os.path.join(HERE, d)
        for fn in sorted(os.listdir(dd)):
            if fn.endswith(".json"):
                out.setdefault(fn[:-5], os.path.join(dd, fn))
    return out


def deembed(c, tool, f, S):
    """2-port vias: NZC 2×thru; pairs: first-order shift per line; None if the thrus are missing"""
    th = []
    for t in thru_names(c):
        r = load(t, "kisi" if tool == "kisi_base" else tool)  # the fixtures stayed on the base mesh (TESTS.md section 17)
        if r is None and tool == "cst_mid":  # no thrus on the intermediate mesh: the finest there is
            r = load(t, "cst_fine") or load(t, "cst")
        if r is None:
            return None
        th.append(r[1])
    if any(t.shape[1] != S.shape[1] for t in th):
        return None
    if any(t.shape[1] != S.shape[1] for t in th):
        return None
    if c["meta"].get("thru"):  # board vias: lines not 50 ohm, matched de-embedding (board_vias.py)
        if S.shape[1] == 4:
            S, th = sparam.mixed_mode(S)["dd"], [sparam.mixed_mode(t)["dd"] for t in th]
        return sparam.deembed_matched(f, S, th[0], th[1], 100.0 if len(c["ports"]) == 4 else 50.0)[0]
    if S.shape[1] == 2:
        FL, _ = sparam.fixture_2xthru(f, th[0])
        FR, _ = sparam.fixture_2xthru(f, th[1])
        return sparam.deembed2(S, FL, FR)
    return sparam.deembed_shift(S, [sparam.fixture_shift(f, t) for t in th])


def metrics(f, A, B, fcmp):
    """A vs B: max |Δ|S21|| dB, max |ΔS11| (complex, linear) and max Δ|S11| dB where above −30 dB,
    phase difference of S21 (deg), delay difference (ps) from the unwrapped phase, up to fcmp"""
    m = f <= fcmp
    if A.shape[1] == 4:
        A, B = sparam.mixed_mode(A)["dd"], sparam.mixed_mode(B)["dd"]
    a21, b21 = A[m, 1, 0], B[m, 1, 0]
    a11, b11 = A[m, 0, 0], B[m, 0, 0]
    d21 = np.abs(sparam.db(a21) - sparam.db(b21))
    big = np.maximum(sparam.db(a11), sparam.db(b11)) > -30
    d11 = np.abs(sparam.db(a11) - sparam.db(b11))[big] if big.any() else np.array([0.0])
    dph = np.degrees(np.angle(a21 * np.conj(b21)))
    pa, pb = np.unwrap(np.angle(a21)), np.unwrap(np.angle(b21))
    fm = f[m]
    tau = lambda p: -np.polyfit(2 * np.pi * fm, p, 1)[0]
    t_a, z_a = sparam.tdr(f, A[:, 0, 0], rise=25e-12, tmax=150e-12)
    t_b, z_b = sparam.tdr(f, B[:, 0, 0], rise=25e-12, tmax=150e-12)
    return {"dS21_max_db": round(float(d21.max()), 3), "dS21_mean_db": round(float(d21.mean()), 3),
            "dS11_max_db_above_-30": round(float(d11.max()), 2), "dS11_max_lin": round(float(np.abs(a11 - b11).max()), 4),
            "dphase21_max_deg": round(float(np.abs(dph).max()), 2), "ddelay_ps": round(float((tau(pa) - tau(pb)) * 1e12), 2),
            "tdr_zmin": [round(float(np.min(z_a)), 2), round(float(np.min(z_b)), 2)], "tdr_zmax": [round(float(np.max(z_a)), 2), round(float(np.max(z_b)), 2)]}


def pdn_metrics(c, f, A, B):
    """G, H (1 ohm ports): G - |Z11| resonance frequencies (local maxima above 0.5 GHz) and the mean
    |Z11| ratio below 1 GHz; H - loop inductance Im(Z11)/omega at 50 and 100 MHz"""
    z0 = c["ports"][0].get("r", 1.0)
    ZA, ZB = sparam.s2z(A, z0), sparam.s2z(B, z0)
    if c["meta"]["group"] == "H":
        res = {}
        for fr in (50e6, 100e6):
            k = int(np.argmin(np.abs(f - fr)))
            la, lb = ZA[k, 0, 0].imag / (2 * np.pi * f[k]), ZB[k, 0, 0].imag / (2 * np.pi * f[k])
            res[f"L{int(fr / 1e6)}_nH"] = [round(float(la) * 1e9, 4), round(float(lb) * 1e9, 4)]
        return res

    def peaks(Z):
        a = np.abs(Z[:, 0, 0])
        k = [i for i in range(1, len(a) - 1) if a[i] > a[i - 1] and a[i] >= a[i + 1] and f[i] > 0.5e9 and a[i] > 2 * np.median(a)]
        return [round(float(f[i]) / 1e9, 3) for i in k][:6]

    m = f < 1e9
    return {"Z11_peaks_GHz": [peaks(ZA), peaks(ZB)], "Z11_below_1GHz_ratio": round(float(np.mean(np.abs(ZA[m, 0, 0]) / np.abs(ZB[m, 0, 0]))), 3)}


def write_md(path, out, fcmp):
    """one row per case: CST mesh cells and run time, against kisi-fdtd raw and de-embedded, and
    the fine-mesh change"""
    def g(v, k, fmt):
        x = v.get(k) if v else None
        return "–" if x is None or (isinstance(x, float) and x != x) else fmt.format(x)

    lines = ["| Case | Mesh | Cells | Time | Δ\\|S21\\| max / mean raw (dB) | Δ\\|S21\\| max de-emb. (dB) | \\|ΔS11\\| max | Δdelay (ps) | TDR min CST / kisi (Ω) | mesh change Δ\\|S21\\| max (dB) | openEMS: Δ\\|S21\\| max de-emb. or raw (dB), Δdelay (ps) |",
             "|---|---|---|---|---|---|---|---|---|---|---|"]
    for cid in sorted(out):
        row = out[cid]
        m = {}
        mp = os.path.join(REF, cid, {"fine": "cst_fine_meta.json", "mid": "cst_mid_meta.json"}.get(row.get("mesh"), "cst_meta.json"))
        if os.path.exists(mp):
            m = json.load(open(mp, encoding="utf-8"))
        cells = f"{m['cells'] // 1000} k" if m.get("cells") else "–"
        secs = f"{m['seconds']:.0f} s" if m.get("seconds") else "–"
        if "pdn_vs_kisi" in row:
            def pdn(v):
                if "L100_nH" in v:
                    return f"L(100 MHz) {v['L100_nH'][0]:.3f} / {v['L100_nH'][1]:.3f} nH"
                pa, pb = v["Z11_peaks_GHz"]
                return f"\\|Z11\\| peaks within {max(abs(x / y - 1) for x, y in zip(pa, pb)) * 100:.1f} %"
            o = pdn(row["pdn_vs_openems"]) + " (CST / openEMS)" if "pdn_vs_openems" in row else "–"
            lines.append(f"| {cid} | {row['mesh']} | {cells} | {secs} | {pdn(row['pdn_vs_kisi'])} (CST / kisi) | | | | | | {o} |")
            continue
        r, d = row.get("raw_vs_kisi"), row.get("deemb_vs_kisi")
        fn = row.get("cst_xfine_vs_cst_fine") or row.get("cst_fine_vs_cst") or row.get("cst_mid_vs_cst")
        tdr = f"{r['tdr_zmin'][0]} / {r['tdr_zmin'][1]}" if r and r["tdr_zmin"][1] == r["tdr_zmin"][1] else "–"
        o = row.get("deemb_vs_openems") or row.get("raw_vs_openems")
        oe = f"{g(o, 'dS21_max_db', '{:.3f}')}, {g(o, 'ddelay_ps', '{:+.2f}')}" if o else row.get("openems_note", "–").replace("|", "\\|")
        if not r and row.get("kisi_note"):
            lines.append(f"| {cid} | {row['mesh']} | {cells} | {secs} | kisi-fdtd {row['kisi_note']} | | | | | {g(fn, 'dS21_max_db', '{:.3f}')} | {oe} |")
            continue
        kb = row.get("deemb_vs_kisi_base") if d else row.get("raw_vs_kisi_base")
        dl = g(d or r, 'ddelay_ps', '{:+.2f}') if not kb else f"{kb['ddelay_ps']:+.2f} → {(d or r)['ddelay_ps']:+.2f}"
        lines.append(f"| {cid} | {row['mesh']} | {cells} | {secs} | {g(r, 'dS21_max_db', '{:.3f}')} / {g(r, 'dS21_mean_db', '{:.3f}')} | {g(d, 'dS21_max_db', '{:.3f}')} | "
                     f"{g(r, 'dS11_max_lin', '{:.3f}')} | {dl} | {tdr} | {g(fn, 'dS21_max_db', '{:.3f}')} | {oe} |")
    open(path, "w", encoding="utf-8").write(f"Up to {fcmp / 1e9:g} GHz; pairs as Sdd.\n\n" + group_table(out) + "\n\n" + "\n".join(lines) + "\n")


GROUPS = [("thru", "thru"), ("A", "A (4-layer via, drill/antipad)"), ("B", "B (8- / 6-layer transitions, stubs)"), ("C", "C (antipad variants)"),
          ("D", "D (return vias, caps)"), ("E", "E (pairs, Sdd)"), ("P", "P (pads, fingers)"), ("T", "T (lines, coupling)"), ("X", "X (one plane removed)"),
          ("R", "R (plane resonances)"), ("RG", "RG (return groups)"), ("board", "board vias (BV)")]


def group_table(out):
    """per group: median (range) of max Δ|S21| raw and the range of Δdelay (de-embedded where available) for
    CST − kisi-fdtd, CST − openEMS and openEMS − kisi-fdtd"""
    def grp(cid):
        if cid.startswith("thru"):
            return "thru"
        if cid.startswith("BV"):
            return "board"
        if cid.startswith("RG"):
            return "RG"
        return cid.split("_")[0]

    def cell(rows, raw, de):
        v = [r[raw] for r in rows if raw in r]
        if not v:
            return "–"
        dm = [x["dS21_max_db"] for x in v]
        dl = [(r.get(de) or r[raw])["ddelay_ps"] for r in rows if raw in r]
        fmt = lambda x: f"{x:.3f}" if x < 1 else f"{x:.2g}"  # noqa: E731
        return f"{fmt(float(np.median(dm)))} dB ({fmt(min(dm))}…{fmt(max(dm))}), {min(dl):+.2f}…{max(dl):+.2f} ps"
    lines = ["| Group | n | CST – kisi-fdtd | CST – openEMS | openEMS – kisi-fdtd |", "|---|---|---|---|---|"]
    for g_, title in GROUPS:
        rows = [r for c, r in out.items() if grp(c) == g_ and "pdn_vs_kisi" not in r]
        if rows:
            lines.append(f"| {title} | {len(rows)} | {cell(rows, 'raw_vs_kisi', 'deemb_vs_kisi')} | {cell(rows, 'raw_vs_openems', 'deemb_vs_openems')} | "
                         f"{cell(rows, 'raw_openems_vs_kisi', 'deemb_openems_vs_kisi')} |")
    return "\n".join(lines)


def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("ids", nargs="*")
    ap.add_argument("--fmax-cmp", type=float, default=20e9)
    ap.add_argument("--md", help="write the per-case table (Markdown) to this file")
    ap.add_argument("--openems", help="via_ref tree with openems.sNp (default: via_ref)")
    a = ap.parse_args()
    global OEMS
    if a.openems:
        OEMS = a.openems
    keys, fcmp = a.ids, a.fmax_cmp
    out = {}
    for cid, cpath in case_files().items():
        if keys and not any(k in cid for k in keys):
            continue
        base = load(cid, "cst")
        fine = load(cid, "cst_fine")
        mid = load(cid, "cst_mid")
        if base is None and fine is None:
            continue
        c = json.load(open(cpath))
        # results are reported on the fine mesh where it exists, else on the intermediate mesh
        # (25 / 25, where the fine mesh exceeds the available mesh size), else on the base mesh;
        # base -> fine (or -> intermediate) and fine -> extra-fine are the mesh checks
        prim, (f, Sc) = next((t, r) for t, r in (("cst_fine", fine), ("cst_mid", mid), ("cst", base)) if r is not None)
        row = {"mesh": {"cst_fine": "fine", "cst_mid": "mid", "cst": "base"}[prim]}
        if c["meta"]["group"] in ("G", "H"):
            r = load(cid, "kisi")
            if r is not None and len(r[0]) == len(f):
                row["pdn_vs_kisi"] = pdn_metrics(c, f, Sc, r[1])
                o = load(cid, "openems")
                if o is not None and len(o[0]) == len(f):
                    row["pdn_vs_openems"] = pdn_metrics(c, f, Sc, o[1])
                    row["pdn_openems_vs_kisi"] = pdn_metrics(c, f, o[1], r[1])
                if base is not None and fine is not None:
                    row["pdn_base_vs_kisi"] = pdn_metrics(c, f, base[1], r[1])
                out[cid] = row
                print(f"{cid:28s} pdn_vs_kisi        {row['pdn_vs_kisi']}")
            continue
        if base is not None and fine is not None:
            row["cst_fine_vs_cst"] = metrics(f, fine[1], base[1], fcmp)
        if base is not None and prim == "cst_mid":
            row["cst_mid_vs_cst"] = metrics(f, mid[1], base[1], fcmp)
        xf = load(cid, "cst_xfine")
        if xf is not None and fine is not None:
            row["cst_xfine_vs_cst_fine"] = metrics(f, xf[1], fine[1], fcmp)
        k_ref = None  # kisi-fdtd's reference, for openEMS against kisi-fdtd
        rk = load(cid, "kisi")
        if rk is not None and not np.isnan(rk[1]).any() and len(rk[0]) == len(f):
            k_ref = rk[1]
        # kisi_base: the former kisi-fdtd reference of the cases recomputed on a refined mesh (TESTS.md section 17)
        for tool in ("kisi", "kisi_base", "openems"):
            r = load(cid, tool)
            if r is None:
                continue
            if np.isnan(r[1]).any():
                row[f"{tool}_note"] = "contains NaN"
                continue
            if np.abs(r[1]).max() > 1.05:
                row[f"{tool}_note"] = "not passive (|S| > 1)"
                continue
            f2, St = r
            if len(f2) != len(f) or np.max(np.abs(f2 - f)) > 1:
                print(cid, tool, "frequency grids differ, skipped")
                continue
            row[f"raw_vs_{tool}"] = metrics(f, Sc, St, fcmp)
            if tool == "openems" and k_ref is not None:
                row["raw_openems_vs_kisi"] = metrics(f, St, k_ref, fcmp)
            if tool == "kisi" and base is not None and prim != "cst":
                row["base_raw_vs_kisi"] = metrics(f, base[1], St, fcmp)
            if c["meta"]["group"] != "thru":
                Dc, Dt = deembed(c, prim, f, Sc), deembed(c, tool, f, St)
                if Dc is not None and Dt is not None:
                    row[f"deemb_vs_{tool}"] = metrics(f, Dc, Dt, fcmp)
                    if tool == "openems" and k_ref is not None:
                        Dk = deembed(c, "kisi", f, k_ref)
                        if Dk is not None:
                            row["deemb_openems_vs_kisi"] = metrics(f, Dt, Dk, fcmp)
        out[cid] = row
        for k, v in row.items():
            if not isinstance(v, dict) or "dS21_max_db" not in v:
                continue
            print(f"{cid:28s} {k:18s} dS21 max {v['dS21_max_db']:6.3f} dB  mean {v['dS21_mean_db']:6.3f}  |dS11| max {v['dS11_max_lin']:.3f}"
                  f"  dphase {v['dphase21_max_deg']:6.2f}°  ddelay {v['ddelay_ps']:6.2f} ps  TDR min {v['tdr_zmin']} max {v['tdr_zmax']}")
    if a.md:
        write_md(a.md, out, fcmp)
    os.makedirs(os.path.join(HERE, "results"), exist_ok=True)
    p = os.path.join(HERE, "results", "cst_compare.json")
    old = json.load(open(p)) if os.path.exists(p) else {}
    old.update(out)
    json.dump({"note": f"CST vs kisi-fdtd / openEMS up to {fcmp / 1e9:g} GHz (pairs: Sdd)", **{k: v for k, v in old.items() if k != "note"}}, open(p, "w"), indent=1)


if __name__ == "__main__":
    main()
