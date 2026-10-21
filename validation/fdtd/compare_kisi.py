"""kisi's fast models against the 3D references: every case becomes a mini board (kisi_board.py),
kisi computes the channel (`kisi net … --s2p/--s4p`, formula impedances), the result is compared
with kisi-fdtd (validation/via_ref/<case>/kisi.sNp).

Writes validation/via_ref/<case>/kisi_model.sNp and results/kisi_models.json (table for the
viewer, metrics of validation/via_references.md section 6).

usage: python3 compare_kisi.py kisi-binary [--only SUBSTR,...]
"""
import json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402

exe = sys.argv[1]
only = [k for k in (sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else []) if k]
tmp = tempfile.mkdtemp()


def interp(f_from, S, f_to):
    out = np.empty((len(f_to),) + S.shape[1:], complex)
    for i in range(S.shape[1]):
        for j in range(S.shape[2]):
            out[:, i, j] = np.interp(f_to, f_from, S[:, i, j].real) + 1j * np.interp(f_to, f_from, S[:, i, j].imag)
    return out


def step(f, s, rise=35e-12):
    return sparam.step_response(f, s, rise, tmax=2e-9)


def notch(f, s21):
    d = sparam.db(s21)
    k = int(np.argmin(d))
    return (f[k], d[k]) if d[k] < -10 and 0 < k < len(f) - 1 else (None, None)


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
    args = [exe, "net", pcb, net, "--no-config", "--s4p" if pair else "--s2p", ts]
    r = subprocess.run(args, capture_output=True, text=True)
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
    sparam.write_touchstone(os.path.join(REF, cid, f"kisi_model.s{n}p"), f, Sk, [f"kisi {cid}: fast channel model (formula impedances, via model)"])
    v = {"case": cid, "group": g}
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
        v["kisi_return"] = vd.get("return_path") if vd else None
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

json.dump({"rows": rows}, open(os.path.join(HERE, "results", "kisi_models.json"), "w"), indent=1)
print("results/kisi_models.json")
