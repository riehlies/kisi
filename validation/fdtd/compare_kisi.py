"""kisi's fast models against the 3D references: every case becomes a mini board (kisi_board.py),
kisi computes the channel (`kisi net … --s2p/--s4p`, formula impedances), the result is compared
with kisi-fdtd (validation/via_ref/<case>/kisi.sNp).

Writes validation/via_ref/<case>/kisi_model.sNp and results/kisi_models.json (table for the
viewer, metrics of validation/via_references.md section 6).

usage: python3 compare_kisi.py kisi-binary|kisi.mjs [--only SUBSTR,...] [--out results/kisi_models.json]
(kisi.mjs: the Node CLI with the 2D field solver; the native binary uses formula impedances)
"""
import json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402

exe = sys.argv[1]
CMD = ["node", exe] if exe.endswith(".mjs") else [exe]
OUT = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else os.path.join(HERE, "results", "kisi_models.json")
only = [k for k in (sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else []) if k]
tmp = tempfile.mkdtemp()


def interp(f_from, S, f_to):
    out = np.empty((len(f_to),) + S.shape[1:], complex)
    for i in range(S.shape[1]):
        for j in range(S.shape[2]):
            # beyond kisi's own range (25 GHz) no values
            out[:, i, j] = np.interp(f_to, f_from, S[:, i, j].real, right=np.nan) + 1j * np.interp(f_to, f_from, S[:, i, j].imag, right=np.nan)
    return out


def step(f, s, rise=35e-12):
    return sparam.step_response(f, s, rise, tmax=2e-9)


def notch(f, s21):
    d = sparam.db(s21)
    k = int(np.argmin(d))
    return (f[k], d[k]) if d[k] < -10 and 0 < k < len(f) - 1 else (None, None)


_thru = {}


def kisi_thru(key):
    """kisi's own 2×thru of a feed layer (thru_<stackup>_<layer> case), on kisi's grid"""
    if key not in _thru:
        cp = os.path.join(HERE, "via_cases", key + ".json")
        _thru[key] = None
        if os.path.exists(cp):
            pcb = os.path.join(tmp, key + ".kicad_pcb")
            subprocess.run([sys.executable, os.path.join(HERE, "kisi_board.py"), cp, pcb], check=True)
            ts = os.path.join(tmp, key + ".s2p")
            r = subprocess.run(CMD + ["net", pcb, "SIG", "--no-config", "--s2p", ts], capture_output=True, text=True, timeout=1800)
            if r.returncode == 0:
                _thru[key] = sparam.read_touchstone(ts)
    return _thru[key]


def tdr_errs(f, s3, sk):
    """relative errors of the TDR minimum and maximum (25 ps, 80 ps window), % – as the reduced
    via model in via_pbm / rust_models"""
    _, z3 = sparam.tdr(f, s3, rise=25e-12, tmax=80e-12)
    _, zk = sparam.tdr(f, sk, rise=25e-12, tmax=80e-12)
    return [round(abs(zk.min() / z3.min() - 1) * 100, 2), round(abs(zk.max() / z3.max() - 1) * 100, 2)]


rows, summary = [], {}
cases = sorted(p for p in os.listdir(os.path.join(HERE, "via_cases")) if p.endswith(".json"))
for name in cases:
    cid = name[:-5]
    if only and not any(k in cid for k in only):
        continue
    c = json.load(open(os.path.join(HERE, "via_cases", name)))
    g = c["meta"]["group"]
    if g in ("G", "H", "thru"):
        continue
    n = len(c["ports"])
    ref = os.path.join(REF, cid, f"kisi.s{n}p")
    if not os.path.exists(ref):
        continue
    f, S3 = sparam.read_touchstone(ref)
    pcb = os.path.join(tmp, cid + ".kicad_pcb")
    subprocess.run([sys.executable, os.path.join(HERE, "kisi_board.py"), os.path.join(HERE, "via_cases", name), pcb], check=True)
    four = n == 4
    pair = g == "E" or "diff" in cid
    net = "D0_P" if pair else ("V1" if four else "SIG")
    ts = os.path.join(tmp, cid + (".s4p" if pair else ".s2p"))
    args = CMD + ["net", pcb, net, "--no-config", "--s4p" if pair else "--s2p", ts]
    r = subprocess.run(args, capture_output=True, text=True, timeout=1800)
    if r.returncode != 0:
        print(cid, "kisi failed:", r.stderr.strip()[:200], flush=True)
        continue
    kj = json.loads(r.stdout)
    fk, Sk = sparam.read_touchstone(ts)
    if four and not pair:
        # victim path only (2-port of V1); kisi reports the coupling separately
        Sk2 = interp(fk, Sk, f)
        v = {"case": cid, "group": g}
        ag = (kj.get("crosstalk") or {}).get("aggressors") or []
        # 3D: aggressor = line A (port 1 driven), victim near = port 2, far = port 4
        t, nx = step(f, S3[:, 1, 0])
        _, fx = step(f, S3[:, 3, 0])
        v["next_3d_pct"] = round(100 * float(np.max(np.abs(nx))), 2)
        v["fext_3d_pct"] = round(100 * float(np.max(np.abs(fx))), 2)
        a0 = ag[0] if ag else {}
        v["next_kisi_pct"] = round(100 * a0.get("next", 0.0), 2) if ag else None
        v["fext_kisi_pct"] = round(100 * a0.get("fext", 0.0), 2) if ag else None
        v["broadside"] = len((kj.get("crosstalk") or {}).get("broadside") or [])
        summary[cid] = v
        rows.append(v)
        print(cid, v, flush=True)
        sparam.write_touchstone(os.path.join(REF, cid, "kisi_model.s2p"), f, Sk2, [f"kisi {cid}: victim path V1 (formula impedances)"])
        continue
    Sk = interp(fk, Sk, f)
    ok = f <= fk.max() + 1
    sparam.write_touchstone(os.path.join(REF, cid, f"kisi_model.s{n}p"), f[ok], Sk[ok], [f"kisi {cid}: fast channel model (formula impedances, via model), up to {fk.max() / 1e9:g} GHz"])
    f, S3, Sk = f[ok], S3[ok], Sk[ok]
    v = {"case": cid, "group": g}
    if g == "T" and cid.endswith("_20") and not four:
        # line parameters: 3D (ABCD of the 20 mm line, γ from 10 / 20 mm) against kisi's segment impedance and delay
        p10 = os.path.join(REF, cid[:-3] + "_10", "kisi.s2p")
        S10 = sparam.read_touchstone(p10)[1] if os.path.exists(p10) else None
        f_full, S3_full = sparam.read_touchstone(ref)
        tl = sparam.line_params(f_full, S3_full, S10)
        k5 = int(np.argmin(np.abs(f_full - 5e9)))
        segs = [x for x in kj.get("segments", []) if x.get("z_ohm_per_line")]
        zk = float(np.mean([x["z_ohm_per_line"] for x in segs])) if segs else None
        ek = (kj["delay_ps"] * 1e-12 * 299792458.0 / (kj["length_mm"] * 1e-3)) ** 2 if kj.get("length_mm") else None
        v.update(zc_3d_5ghz=round(float(tl["zc"][k5]), 2), eeff_3d_5ghz=round(float(tl["eeff"][k5]), 3), zc_kisi=round(zk, 2) if zk else None,
                 eeff_kisi=round(ek, 3) if ek else None, kisi_source=(kj.get("field_solver") or {}).get("cross_sections_field", 0) and "field solver" or "formula")
    if not pair:
        t3, z3 = sparam.tdr(f, S3[:, 0, 0], rise=35e-12, tmax=150e-12)
        tk, zk = sparam.tdr(f, Sk[:, 0, 0], rise=35e-12, tmax=150e-12)
        f3n, d3n = notch(f, S3[:, 1, 0])
        fkn, dkn = notch(f, Sk[:, 1, 0])
        flim = min(20e9, 0.8 * min(x for x in (f3n, fkn, 1e12) if x))
        m = f <= flim
        v["dS21_max_db"] = round(float(np.max(np.abs(sparam.db(S3[m, 1, 0]) - sparam.db(Sk[m, 1, 0])))), 2)
        v["f_limit_ghz"] = round(flim / 1e9, 1)
        v["z_min_3d"], v["z_min_kisi"] = round(float(z3.min()), 1), round(float(zk.min()), 1)
        v["z_max_3d"], v["z_max_kisi"] = round(float(z3.max()), 1), round(float(zk.max()), 1)
        v["stub_3d_ghz"] = round(f3n / 1e9, 2) if f3n else None
        v["stub_kisi_ghz"] = round(fkn / 1e9, 2) if fkn else None
        v["stub_depth_3d_db"] = round(float(d3n), 1) if f3n else None
        v["stub_depth_kisi_db"] = round(float(dkn), 1) if fkn else None
        vd = (kj.get("via_details") or [{}])[0] if kj.get("via_details") else {}
        v["kisi_via_z"] = round(vd.get("z_ohm", 0) or 0, 1) if vd else None
        v["kisi_stub_quarter_wave_ghz"] = round(vd["stub_resonance_ghz"], 2) if vd and vd.get("stub_resonance_ghz") else None
        v["f_max_kisi_ghz"] = round(fk.max() / 1e9, 1)
        v["kisi_return"] = vd.get("return_path") if vd else None
        v["kisi_via_model"] = vd.get("model") if vd else None
        # the via alone: kisi's channel de-embedded with kisi's own 2×thru against the 3D result
        # de-embedded with the 3D 2×thru (the full-channel TDR also holds the low-frequency offset
        # of the 3D lumped ports), both up to kisi's 25 GHz
        dp = os.path.join(REF, cid, "kisi_deembedded.s2p")
        th = [kisi_thru(f"thru_{c['meta']['stackup']}_{q['layer']}") for q in c["meta"].get("ports", [])]
        if g in "ABCD" and os.path.exists(dp) and len(th) == 2 and all(th):
            fk0, Sk0 = sparam.read_touchstone(ts)
            FL, _ = sparam.fixture_2xthru(fk0, th[0][1])
            FR, _ = sparam.fixture_2xthru(fk0, th[1][1])
            Dk = interp(fk0, sparam.deembed2(Sk0, FL, FR), f)
            fd, D3 = sparam.read_touchstone(dp)
            D3 = D3[: len(f)]
            v["via_tdr_err_pct"] = tdr_errs(f, D3[:, 0, 0], Dk[:, 0, 0])
            m2 = f <= 20e9
            v["via_dS21_max_db"] = round(float(np.max(np.abs(sparam.db(D3[m2, 1, 0]) - sparam.db(Dk[m2, 1, 0])))), 3)
            v["via_dS21_mean_db"] = round(float(np.mean(np.abs(sparam.db(D3[m2, 1, 0]) - sparam.db(Dk[m2, 1, 0])))), 3)
    else:
        mm3, mmk = sparam.mixed_mode(S3), sparam.mixed_mode(Sk)
        m = f <= 20e9
        v["dSdd21_max_db"] = round(float(np.max(np.abs(sparam.db(mm3["dd"][m, 1, 0]) - sparam.db(mmk["dd"][m, 1, 0])))), 2)
        v["Scd21_max_3d_db"] = round(float(np.max(sparam.db(mm3["cd"][m, 1, 0]))), 1)
        v["Scd21_max_kisi_db"] = round(float(np.max(sparam.db(mmk["cd"][m, 1, 0]))), 1)
        t3, z3 = sparam.tdr(f, mm3["dd"][:, 0, 0], rise=35e-12, tmax=150e-12, z0=100.0)
        tk, zk = sparam.tdr(f, mmk["dd"][:, 0, 0], rise=35e-12, tmax=150e-12, z0=100.0)
        v["zdiff_min_3d"], v["zdiff_min_kisi"] = round(float(z3.min()), 1), round(float(zk.min()), 1)
        v["zdiff_max_3d"], v["zdiff_max_kisi"] = round(float(z3.max()), 1), round(float(zk.max()), 1)
    summary[cid] = v
    rows.append(v)
    print(cid, v, flush=True)

vias = [r for r in rows if r.get("via_tdr_err_pct")]
if vias:
    e = [x for r in vias for x in r["via_tdr_err_pct"]]
    summ = {"via_cases": len(vias), "via_tdr_err_pct_median": round(float(np.median(e)), 2), "via_tdr_err_pct_p90": round(float(np.percentile(e, 90)), 2),
            "via_dS21_mean_db_median": round(float(np.median([r["via_dS21_mean_db"] for r in vias])), 3),
            "via_dS21_max_db_median": round(float(np.median([r["via_dS21_max_db"] for r in vias])), 3)}
    stubs = [r for r in rows if r.get("stub_3d_ghz") and r.get("stub_kisi_ghz")]
    summ["stub_err_pct"] = {r["case"]: round((r["stub_kisi_ghz"] / r["stub_3d_ghz"] - 1) * 100, 1) for r in stubs}
    print(json.dumps(summ, indent=1))
else:
    summ = {}
json.dump({"tool": " ".join(CMD), "summary": summ, "rows": rows}, open(OUT, "w"), indent=1)
print(OUT)
