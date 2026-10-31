"""Run the via reference cases (via_cases.py) and store the results per case:

  <out>/<case>/kisi.sNp        kisi-fdtd, conformal metal, base mesh
  <out>/<case>/kisi_fine.sNp   kisi-fdtd on a mesh refined by --fine (convergence check)
  <out>/<case>/openems.sNp     openEMS (staircase), if --openems
  <out>/<case>/meta.json       case meta, cells, steps, run times, convergence, versions, date

Symmetric structures are excited on half of their ports (2×thru: port 1; pairs: ports 1 and 3)
and the other columns are mirrored.

usage: python3 run_refs.py cases_dir out_dir [--only SUBSTR] [--fine 1.4] [--openems base|fine]
                           [--jobs 2] [--exe kisi-fdtd]
"""
import argparse, concurrent.futures as cf, datetime, glob, json, os, subprocess, sys, tempfile, time
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sparam  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
C0 = 299792458.0


def excitation(case):
    g = case["meta"]["group"]
    if "excite" in case["meta"]:
        return case["meta"]["excite"], case["meta"].get("mirror")
    n = len(case["ports"])
    if g == "thru":
        return [0], [1, 0]
    if g == "E":
        return [0, 2], [1, 0, 3, 2]
    if g == "H":
        return [0], None
    return list(range(n)), None


def refined(case, factor):
    c = json.loads(json.dumps(case))
    er = max(l["er"] for l in c["layers"])
    dmax = c["mesh"].get("dmax") or C0 / c["fmax"] / er ** 0.5 / 20 * 1e3
    c["mesh"]["dfine"] = round(c["mesh"]["dfine"] / factor, 4)
    c["mesh"]["dmax"] = round(dmax / factor, 4)
    return c


def field_raster(case, res, width=110):
    """fields of the first excitation on a uniform raster without the PML, int8 per component
    (re, im) with one scale per plane (99.5th percentile of |E|), base64"""
    import base64
    runs = [r for r in res["runs"] if r.get("fields")]
    if not runs:
        return None
    dom = case["domain"]
    zb = [min(l["z0"] for l in case["layers"]), max(l["z1"] for l in case["layers"])]
    groups = {}
    for fp in runs[0]["fields"]:
        groups.setdefault((fp["axis"], fp["pos"], fp["freq"]), []).append(fp)
    out = []
    for (axis, pos, freq), fps in groups.items():
        u, v = np.array(fps[0]["u"]), np.array(fps[0]["v"])
        if axis == "z":
            ext = [dom[0], dom[3], dom[1], dom[4]]
        elif axis == "y":
            ext = [dom[0], dom[3], zb[0] - 0.3, zb[1] + 0.3]
        else:
            ext = [dom[1], dom[4], zb[0] - 0.3, zb[1] + 0.3]
        W = width
        H = max(8, int(round(W * (ext[3] - ext[2]) / (ext[1] - ext[0]))))
        if axis != "z":  # sections: height exaggerated 2.5× like the cross-section drawing
            H = max(24, int(round(W * (ext[3] - ext[2]) / (ext[1] - ext[0]) * 2.5)))
        xs = ext[0] + (np.arange(W) + 0.5) * (ext[1] - ext[0]) / W
        ys = ext[2] + (np.arange(H) + 0.5) * (ext[3] - ext[2]) / H
        iu = np.clip(np.searchsorted(u, xs) - 1, 0, len(u) - 1)
        iv = np.clip(np.searchsorted(v, ys) - 1, 0, len(v) - 1)
        comps = {}
        mags = []
        for fp in fps:
            re = np.array(fp["re"], dtype=np.float32).reshape(len(u), len(v))[np.ix_(iu, iv)].T  # rows = v (y or z)
            im = np.array(fp["im"], dtype=np.float32).reshape(len(u), len(v))[np.ix_(iu, iv)].T
            comps[fp["comp"]] = (re, im)
            mags.append(re ** 2 + im ** 2)
        scale = float(np.percentile(np.sqrt(sum(mags)), 99.5)) or 1.0
        enc = {}
        for k, (re, im) in comps.items():
            q = lambda a: base64.b64encode(np.clip(np.round(a / scale * 127), -127, 127).astype(np.int8).tobytes()).decode()
            enc[str(k)] = {"re": q(re), "im": q(im)}
        out.append({"axis": axis, "pos": pos, "freq": freq, "extent": [round(x, 4) for x in ext], "w": W, "h": H, "scale": round(scale, 3), "comps": enc})
    return {"note": "E per volt of the incident wave at port 1 (V/m per V); int8 = value / scale · 127, rows bottom to top", "planes": out}


def run_kisi(exe, case, tmp, tag):
    ex, mirror = excitation(case)
    c = dict(case, excite=ex)
    p = os.path.join(tmp, f"{case['name']}_{tag}.json")
    json.dump(c, open(p, "w"))
    t0 = time.time()
    r = json.loads(subprocess.run([exe, p], capture_output=True, text=True, check=True).stdout)
    f, S = sparam.from_kisi(r, len(case["ports"]), mirror)
    run_kisi.fields = field_raster(case, r) if tag == "base" else None
    info = {"cells": r["cells"], "steps": [x["steps"] for x in r["runs"]], "dt": r["runs"][0]["dt"], "seconds": round(time.time() - t0, 1),
            "dfine": case["mesh"]["dfine"], "dmax": case["mesh"].get("dmax")}
    return f, S, info


def run_openems(exe, case, tmp, tag):
    ex, mirror = excitation(case)
    c = dict(case, excite=ex)
    p = os.path.join(tmp, f"{case['name']}_{tag}_oe.json")
    json.dump(c, open(p, "w"))
    mp = p.replace(".json", ".mesh.json")
    # kisi-fdtd's mesh of the case (conformal-mode lines, as for the reference); openEMS staircases on it
    open(mp, "w").write(subprocess.run([exe, p, "--mesh-only"], capture_output=True, text=True, check=True).stdout)
    out = p.replace(".json", ".out.json")
    subprocess.run([sys.executable, os.path.join(HERE, "openems_case.py"), p, mp, out], check=True, capture_output=True)
    r = json.load(open(out))
    f, S = sparam.from_openems([r])
    if mirror is not None:
        n = S.shape[1]
        for j in range(n):
            if np.isnan(S[0, 0, j].real):
                for i in range(n):
                    S[:, i, j] = S[:, mirror[i], mirror[j]]
    return f, S, {"cells": r["cells"], "seconds": round(r["seconds"], 1), "mesh": tag}


def convergence(S1, S2):
    """max |ΔS21|-type (transmission) in dB and max |ΔS11|-type (reflection, where above −20 dB)"""
    n = S1.shape[1]
    trans, refl = 0.0, 0.0
    d1, d2 = sparam.db(S1), sparam.db(S2)
    for i in range(n):
        for j in range(n):
            if i == j:
                m = np.maximum(d1[:, i, i], d2[:, i, i]) > -20
                if m.any():
                    refl = max(refl, float(np.max(np.abs(d1[m, i, i] - d2[m, i, i]))))
            elif np.max(d1[:, i, j]) > -6:  # through paths only
                trans = max(trans, float(np.max(np.abs(d1[:, i, j] - d2[:, i, j]))))
    return {"max_dS_through_db": round(trans, 3), "max_dS_refl_db_above_-20": round(refl, 2)}


def one(path, out, a):
    case = json.load(open(path))
    name = case["name"]
    d = os.path.join(out, name)
    os.makedirs(d, exist_ok=True)
    n = len(case["ports"])
    tmp = tempfile.mkdtemp(prefix="ref_")
    meta = {"case": name, "meta": case["meta"], "date": datetime.date.today().isoformat(), "fmax": case["fmax"],
            "tool": "kisi-fdtd (conformal metal, CPML)", "runs": {}}
    hdr = [f"{name}: {case['meta']['title']}", f"stackup: {case['meta']['stackup_title']}", "ports: " + ", ".join(p["name"] for p in case["meta"]["ports"])]
    z0 = case["ports"][0].get("r", 50.0)
    f, S, info = run_kisi(a.exe, case, tmp, "base")
    sparam.write_touchstone(os.path.join(d, f"kisi.s{n}p"), f, S, hdr + [f"kisi-fdtd, conformal, dfine {info['dfine']} mm, {info['cells']} cells"], z0)
    meta["runs"]["kisi"] = info
    if run_kisi.fields:
        json.dump(run_kisi.fields, open(os.path.join(d, "fields.json"), "w"), separators=(",", ":"))
    if a.fine and case["meta"]["group"] != "thru":
        cf_ = refined(case, a.fine)
        f2, S2, info2 = run_kisi(a.exe, cf_, tmp, "fine")
        sparam.write_touchstone(os.path.join(d, f"kisi_fine.s{n}p"), f2, S2, hdr + [f"kisi-fdtd, conformal, dfine {info2['dfine']} mm, {info2['cells']} cells"], z0)
        meta["runs"]["kisi_fine"] = info2
        meta["convergence"] = convergence(S, S2)
    if a.openems:
        oc = refined(case, a.fine) if a.openems == "fine" else case
        f3, S3, info3 = run_openems(a.exe, oc, tmp, a.openems)
        sparam.write_touchstone(os.path.join(d, f"openems.s{n}p"), f3, S3, hdr + [f"openEMS, staircase, {info3['cells']} cells"], z0)
        meta["runs"]["openems"] = info3
    json.dump(meta, open(os.path.join(d, "meta.json"), "w"), indent=1)
    return name, meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cases")
    ap.add_argument("out")
    ap.add_argument("--only", default="")
    ap.add_argument("--fine", type=float, default=0.0)
    ap.add_argument("--openems", choices=["base", "fine"], default=None)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--exe", default="kisi-fdtd")
    a = ap.parse_args()
    keys = [k for k in a.only.split(",") if k]
    paths = sorted(p for p in glob.glob(os.path.join(a.cases, "*.json")) if not keys or any(k in os.path.basename(p) for k in keys))
    with cf.ProcessPoolExecutor(a.jobs) as pool:
        futs = {pool.submit(one, p, a.out, a): p for p in paths}
        for fu in cf.as_completed(futs):
            try:
                name, meta = fu.result()
                k = meta["runs"]["kisi"]
                print(f"{name}: {k['cells'] // 1000} k cells, steps {k['steps']}, {k['seconds']} s" + (f", convergence {meta['convergence']}" if "convergence" in meta else ""), flush=True)
            except Exception as e:  # keep going
                print(f"{futs[fu]}: FAILED {e}", flush=True)


if __name__ == "__main__":
    main()
