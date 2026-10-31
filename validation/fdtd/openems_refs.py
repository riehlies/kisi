"""openEMS for every 3D reference case (validation/via_ref): an independent check of kisi-fdtd.

kisi-fdtd computes the references with conformal metal on its base mesh. openEMS has staircase
metal only, so each case runs in openEMS twice:

  base   the mesh of the kisi-fdtd reference (the same lines): a like-for-like check of the solver
         implementation (ports, PML, lumped elements, materials, geometry rules), together with
         kisi-fdtd staircase on that mesh (`stair`);
  fine   the cell at metal edges, vias, ports, GND vias and refinement boxes halved (`FINE`), the
         largest cell (λ/20) kept: staircase converges at first order, so this halves openEMS'
         metal error, and |fine − base| estimates what remains of it (Richardson: 2·fine − base).

Subcommands:
  plan  [--shards N] [--out D]       cost-balanced list of the tasks (case × mesh × excited port) not yet in D
                                     → openems_plan.json
  run   --plan P --shard K --out D   run one shard's tasks (openEMS) → D/<case>/<mesh>_ex<j>.json
  merge D [D …]                      raw results → via_ref/<case>/openems.sNp (fine), openems_base.sNp, meta.json
  stair --out D [--jobs 2]           kisi-fdtd staircase on the base mesh → D/<case>/kisi_stair.sNp; with
        [--mesh fine] [--metal conformal] kisi-fdtd on the fine mesh → D/<case>/kisi_fine_<metal>.sNp
  compare [--stair D]                kisi-fdtd against openEMS per case and group → results/openems_refs.json

usage: python3 openems_refs.py plan|run|merge|stair|compare … [--exe kisi-fdtd] [--only SUBSTR,…]
"""
import argparse, concurrent.futures as cf, datetime, glob, json, os, subprocess, sys, tempfile, time
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
CASE_DIRS = ("via_cases", "board_cases", "resonance_cases", "return_cases")
PLAN = os.path.join(HERE, "openems_plan.json")
FINE = 2.0
# Cases on which openEMS' UPML lets the energy settle at −15 … −20 dB instead of decaying (board cut-outs with
# copper running into the PML on several layers): the run would end only at max_steps and grows late. openEMS
# stops these at TCAP × the physical time kisi-fdtd needed for its end criterion (same excited port), so both
# DFTs cover a comparable window. With a Mur boundary the same cases decay normally (TESTS.md section 16).
TCAP = {"BV_ts_m2_per0": 1.2, "BV_vna_rf_hblb1": 1.2, "BV_ts_m2_per0_q050f25s_e35_in2gnd": 1.2}
C0 = 299792458.0
sys.path.insert(0, HERE)
import sparam  # noqa: E402
from run_refs import excitation, metal_refined  # noqa: E402


# --- cases and meshes ------------------------------------------------------------------------
def all_cases(only=()):
    """reference cases with a stored kisi-fdtd result: name → case file"""
    out = {}
    for d in CASE_DIRS:
        for p in sorted(glob.glob(os.path.join(HERE, d, "*.json"))):
            n = os.path.splitext(os.path.basename(p))[0]
            if os.path.exists(os.path.join(REF, n, "meta.json")) and (not only or any(k in n for k in only)):
                out[n] = p
    return out


def mesh_case(case, mesh):
    """the case for a mesh: "base" as stored, "fine" with the cells at metal halved"""
    c = metal_refined(case, FINE if mesh == "fine" else 1)
    c.pop("monitors", None)
    return c


def make_mesh(exe, case, path):
    """mesh lines from kisi-fdtd's mesher (with its conformal-mode rules, as for the references)"""
    json.dump(case, open(path, "w"))
    out = subprocess.run([exe, path, "--mesh-only"], capture_output=True, text=True, check=True).stdout
    open(path.replace(".json", ".mesh.json"), "w").write(out)
    return json.loads(out)


def dt_of(mesh):
    d = [min(np.diff(mesh[a])) * 1e-3 for a in "xyz"]
    return 1 / (C0 * np.sqrt(sum(1 / x ** 2 for x in d)))


# --- plan --------------------------------------------------------------------------------------
def plan(a):
    tmp = tempfile.mkdtemp(prefix="oeplan_")
    tasks = []
    rate = 65e6  # openEMS cell updates per s per task with 2 threads, two tasks on a 4-vCPU runner (measured 60–88 M)
    for n, p in all_cases(a.only).items():
        case = json.load(open(p))
        meta = json.load(open(os.path.join(REF, n, "meta.json")))
        ex, _ = excitation(case)
        k = meta["runs"]["kisi"]
        dt_k = k["dt"]
        for mesh in a.meshes.split(","):
            c = mesh_case(case, mesh)
            m = make_mesh(a.exe, c, os.path.join(tmp, f"{n}_{mesh}.json"))
            cells = len(m["x"]) * len(m["y"]) * len(m["z"])
            for i, j in enumerate(ex):
                if os.path.exists(os.path.join(a.out, n, f"{mesh}_ex{j}.json")):
                    continue  # done
                # openEMS' end criterion: about kisi-fdtd's physical time on ringing cases, at least ≈ 0.85 ns (measured)
                steps = max(1.15 * k["steps"][i] * dt_k, 0.85e-9) / dt_of(m)
                tasks.append({"case": n, "mesh": mesh, "ex": j, "cells": cells, "est_s": round(cells * steps / rate)})
    # big tasks run alone with 4 threads; the rest in pairs with 2 threads each, greedy by cost
    big = [t for t in tasks if t["est_s"] > a.big * 3600]
    rest = sorted((t for t in tasks if t["est_s"] <= a.big * 3600), key=lambda t: -t["est_s"])
    shards = [{"threads": 4, "jobs": 1, "tasks": [t], "est_s": round(t["est_s"] / 1.6)} for t in big]
    nrest = max(1, a.shards - len(shards))
    reg = [{"threads": 2, "jobs": 2, "tasks": [], "load": 0.0} for _ in range(nrest)]
    for t in rest:
        s = min(reg, key=lambda s: s["load"])
        s["tasks"].append(t)
        s["load"] += t["est_s"] / 2
    for s in reg:
        s["est_s"] = round(s.pop("load"))
    shards += [s for s in reg if s["tasks"]]
    out = {"note": "generated by openems_refs.py plan; est_s are rough planning estimates", "fine_factor": FINE, "shards": shards}
    json.dump(out, open(a.plan, "w"), indent=1)
    print(f"{len(tasks)} tasks in {len(shards)} shards; largest shard ≈ {max(s['est_s'] for s in shards) / 3600:.1f} h, "
          f"total ≈ {sum(t['est_s'] for t in tasks) / 3600:.0f} task-hours")


# --- run ---------------------------------------------------------------------------------------
def host_info():
    """the machine a result was computed on: CPU model, sockets × cores × threads, clock, memory"""
    h = {"cpus": os.cpu_count()}
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                h["model"] = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    try:
        lc = subprocess.run(["lscpu"], capture_output=True, text=True, timeout=10).stdout
        keys = {"Socket(s)": "sockets", "Core(s) per socket": "cores_per_socket", "Thread(s) per core": "threads_per_core",
                "CPU MHz": "mhz", "CPU max MHz": "max_mhz", "L3 cache": "l3"}
        for line in lc.splitlines():
            k, _, v = line.partition(":")
            if k.strip() in keys:
                h[keys[k.strip()]] = v.strip()
    except (OSError, subprocess.SubprocessError):
        pass
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemTotal"):
                h["mem_gb"] = round(int(line.split()[1]) / 2 ** 20, 1)
                break
    except OSError:
        pass
    return h


HOST = None


def run_task(exe, t, out, threads):
    n, mesh, j = t["case"], t["mesh"], t["ex"]
    case = json.load(open(all_cases()[n]))
    c = mesh_case(case, mesh)
    c["excite"] = [j]
    d = os.path.join(out, n)
    os.makedirs(d, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix=f"oe_{n}_")
    cp = os.path.join(tmp, "case.json")
    m = make_mesh(exe, c, cp)
    t_cap = None
    if n in TCAP:
        k = json.load(open(os.path.join(REF, n, "meta.json")))["runs"]["kisi"]
        ex, _ = excitation(case)
        t_cap = TCAP[n] * k["steps"][ex.index(j)] * k["dt"]
        c["max_steps"] = int(t_cap / dt_of(m)) + 1
        json.dump(c, open(cp, "w"))
    res = os.path.join(tmp, "out.json")
    t0 = time.time()
    p = subprocess.run([sys.executable, os.path.join(HERE, "openems_case.py"), cp, cp.replace(".json", ".mesh.json"), res, "--threads", str(threads)],
                       capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError(p.stderr[-2000:])
    r = json.load(open(res))
    r.update(mesh=mesh, dfine=c["mesh"]["dfine"], cells=len(m["x"]) * len(m["y"]) * len(m["z"]), dt=dt_of(m), wall_s=round(time.time() - t0, 1), host=HOST)
    if t_cap:
        r["t_cap_ns"] = round(t_cap * 1e9, 3)
    json.dump(r, open(os.path.join(d, f"{mesh}_ex{j}.json"), "w"), separators=(",", ":"))
    return f"{n} {mesh} ex{j}: {r['seconds']:.0f} s, {r['cells']} cells"


def run(a):
    sh = json.load(open(a.plan))["shards"][a.shard]
    todo = [t for t in sh["tasks"] if not os.path.exists(os.path.join(a.out, t["case"], f"{t['mesh']}_ex{t['ex']}.json"))]
    os.makedirs(os.path.join(a.out, "_logs"), exist_ok=True)
    log = open(os.path.join(a.out, "_logs", f"shard{a.shard}.txt"), "a")

    def say(msg):  # the job logs are not always reachable: the log goes with the results
        print(msg, flush=True)
        log.write(f"{datetime.datetime.now().isoformat(timespec='seconds')} {msg}\n")
        log.flush()

    global HOST
    HOST = host_info()
    json.dump(HOST, open(os.path.join(a.out, "_logs", f"shard{a.shard}_host.json"), "w"), indent=1)
    say(f"shard {a.shard}: {len(todo)} tasks, {sh['jobs']} × {sh['threads']} threads, host {HOST}")
    with cf.ThreadPoolExecutor(sh["jobs"]) as pool:
        futs = {pool.submit(run_task, a.exe, t, a.out, sh["threads"]): t for t in todo}
        for fu in cf.as_completed(futs):
            try:
                say(fu.result())
            except Exception as e:  # keep going; the merge reports what is missing
                say(f"{futs[fu]}: FAILED {e}")


# --- merge -------------------------------------------------------------------------------------
def merge(a):
    version = a.version
    for n, p in all_cases(a.only).items():
        case = json.load(open(p))
        ex, mirror = excitation(case)
        npt = len(case["ports"])
        mp = os.path.join(REF, n, "meta.json")
        meta = json.load(open(mp))
        hdr = [f"{n}: {case['meta']['title']}", f"stackup: {case['meta']['stackup_title']}", "ports: " + ", ".join(q["name"] for q in case["meta"]["ports"])]
        for mesh, tag in (("base", "openems_base"), ("fine", "openems")):
            raws = {}
            for d in a.dirs:
                for j in ex:
                    q = os.path.join(d, n, f"{mesh}_ex{j}.json")
                    if os.path.exists(q):
                        raws[j] = json.load(open(q))
            if len(raws) < len(ex):
                if raws or tag not in meta.get("runs", {}):
                    print(f"{n} {mesh}: {len(raws)} of {len(ex)} excitations")
                continue
            rs = [raws[j] for j in ex]
            f, S = sparam.from_openems(rs)
            if mirror is not None:
                for j in range(npt):
                    if np.isnan(S[0, 0, j].real):
                        for i in range(npt):
                            S[:, i, j] = S[:, mirror[i], mirror[j]]
            r0 = rs[0]
            sparam.write_touchstone(os.path.join(REF, n, f"{tag}.s{npt}p"), f, S,
                                    hdr + [f"openEMS {version}, staircase, {mesh} mesh (dfine {r0['dfine']} mm), {r0['cells']} cells"], case["ports"][0].get("r", 50.0))
            info = {"tool": f"openEMS {version} (staircase, PML_{case.get('pml_cells', 8)})", "mesh": mesh, "dfine": r0["dfine"], "cells": r0["cells"],
                    "dt": r0["dt"], "seconds": [round(r["seconds"], 1) for r in rs], "threads": [r["threads"] for r in rs],
                    "t_end_ns": [round(r["runs"][0]["t_end"] * 1e9, 3) for r in rs],
                    "rate_mcells_s": [round(r["cells"] * r["runs"][0]["t_end"] / r["dt"] / r["seconds"] / 1e6, 1) for r in rs], "date": a.date}
            if any(r.get("t_cap_ns") for r in rs):
                info["t_cap_ns"] = [r.get("t_cap_ns") for r in rs]
            hosts = [r["host"] for r in rs if r.get("host")]
            if hosts:
                info["hosts"] = [dict(t) for t in {tuple(sorted(h.items())) for h in hosts}]
            if mesh == "base" and r0["cells"] != meta["runs"]["kisi"]["cells"]:
                info["warning"] = f"mesh differs from the kisi-fdtd run ({meta['runs']['kisi']['cells']} cells)"
                print(f"{n}: {info['warning']}")
            meta["runs"][tag] = info
        json.dump(meta, open(mp, "w"), indent=1)


# --- kisi-fdtd staircase on the base mesh --------------------------------------------------------
def stair_one(exe, n, p, out, mesh="base", metal="stair"):
    case = json.load(open(p))
    c = mesh_case(case, mesh)
    ex, mirror = excitation(case)
    c["excite"] = ex
    tmp = tempfile.mkdtemp(prefix=f"st_{n}_")
    cp = os.path.join(tmp, "case.json")
    make_mesh(exe, c, cp)  # the conformal-mode mesh (base: the reference mesh)
    c["conformal"] = metal == "conformal"
    tag = "kisi_stair" if (mesh, metal) == ("base", "stair") else f"kisi_{mesh}_{metal}"
    sp = os.path.join(tmp, "stair.json")
    json.dump(c, open(sp, "w"))
    t0 = time.time()
    r = json.loads(subprocess.run([exe, sp, "--mesh", cp.replace(".json", ".mesh.json")], capture_output=True, text=True, check=True).stdout)
    f, S = sparam.from_kisi(r, len(case["ports"]), mirror)
    os.makedirs(os.path.join(out, n), exist_ok=True)
    sparam.write_touchstone(os.path.join(out, n, f"{tag}.s{len(case['ports'])}p"), f, S,
                            [f"{n}: kisi-fdtd, {metal}, {mesh} mesh, {r['cells']} cells"], case["ports"][0].get("r", 50.0))
    json.dump({"cells": r["cells"], "steps": [x["steps"] for x in r["runs"]], "seconds": round(time.time() - t0, 1)}, open(os.path.join(out, n, f"{tag}.json"), "w"))
    return f"{n} {mesh} {metal}: {r['cells']} cells, {time.time() - t0:.0f} s"


def stair(a):
    """kisi-fdtd on a mesh of openems_refs.py: staircase on the base mesh (default), or --mesh fine and
    --metal conformal|stair (the convergence of kisi-fdtd itself, and the like-for-like check on the fine mesh)"""
    tag = "kisi_stair" if (a.mesh, a.metal) == ("base", "stair") else f"kisi_{a.mesh}_{a.metal}"
    cases = {n: p for n, p in all_cases(a.only).items() if not glob.glob(os.path.join(a.out, n, f"{tag}.s*p"))}
    with cf.ProcessPoolExecutor(a.jobs) as pool:
        futs = {pool.submit(stair_one, a.exe, n, p, a.out, a.mesh, a.metal): n for n, p in cases.items()}
        for fu in cf.as_completed(futs):
            try:
                print(fu.result(), flush=True)
            except Exception as e:
                print(f"{futs[fu]}: FAILED {e}", flush=True)


# --- compare -----------------------------------------------------------------------------------
def load(d, n, tool):
    for k in (1, 2, 4):
        p = os.path.join(d, n, f"{tool}.s{k}p")
        if os.path.exists(p):
            return sparam.read_touchstone(p)
    return None, None


def through_port(S):
    """the main through path from port 1: the largest mean |S_j1| (S21 of a 2-port, S31 = P near → P far of a
    pair, S21 of two single vias sharing a cavity)"""
    if S.shape[1] == 2:
        return 1
    m = [np.nanmean(np.abs(S[:, j, 0])) for j in range(1, S.shape[1])]
    return 1 + int(np.nanargmax(m))


def stats(f, A, B, fmax):
    """|ΔS| statistics of two results (dB): through path (mean / max), reflection where either is
    above −30 dB, the through phase at fmax and the delay difference τ_A − τ_B (ps; positive: A slower)"""
    m = f <= fmax * 1.0001
    da, db_ = sparam.db(A[m]), sparam.db(B[m])
    if A.shape[1] == 1:  # one port (H): the reflection only, as complex difference in dB
        d = np.abs(da[:, 0, 0] - db_[:, 0, 0])
        return {"dS11_all_mean_db": round(float(d.mean()), 3), "dS11_all_max_db": round(float(d.max()), 3)}
    i = through_port(A)
    dt = np.abs(da[:, i, 0] - db_[:, i, 0])
    mr = np.maximum(da[:, 0, 0], db_[:, 0, 0]) > -30
    dr = np.abs(da[mr, 0, 0] - db_[mr, 0, 0])
    ph = np.unwrap(np.angle(A[m, i, 0])) - np.unwrap(np.angle(B[m, i, 0]))
    fm = f[m]
    out = {"dS_thru_mean_db": round(float(dt.mean()), 3), "dS_thru_max_db": round(float(dt.max()), 3),
           "dS11_mean_db": round(float(dr.mean()), 2) if mr.any() else None, "dS11_max_db": round(float(dr.max()), 2) if mr.any() else None,
           "dphase_fmax_deg": round(float(np.degrees(ph[-1])), 2), "dtau_ps": round(float(-np.mean(ph[fm > fm[-1] / 4] / (2 * np.pi * fm[fm > fm[-1] / 4])) * 1e12), 2)}
    if A.shape[1] == 4:
        ma, mb = sparam.mixed_mode(A[m])["dd"], sparam.mixed_mode(B[m])["dd"]
        d = np.abs(sparam.db(ma[:, 1, 0]) - sparam.db(mb[:, 1, 0]))
        out.update(dSdd21_mean_db=round(float(d.mean()), 3), dSdd21_max_db=round(float(d.max()), 3))
    return out


def dips(f, d, fmax, prom=3.0):
    """transmission dips (resonances, notches): local minima at least `prom` dB below the highest
    value within ±5 % in frequency, up to fmax → [(GHz, dB)]"""
    out = []
    for k in range(1, len(f) - 1):
        if f[k] > fmax or not (d[k] < d[k - 1] and d[k] <= d[k + 1]):
            continue
        w = (f > f[k] * 0.95) & (f < f[k] * 1.05)
        if d[w].max() - d[k] >= prom:
            out.append((float(f[k] / 1e9), float(d[k])))
    return out


def match_dips(a, b, tol=0.03):
    """dips of a matched to the nearest dip of b within tol (relative): count, median |Δf| %, median |Δdepth| dB"""
    df, dd, n = [], [], 0
    for fa, da in a:
        c = min(b, key=lambda x: abs(x[0] - fa), default=None)
        if c and abs(c[0] / fa - 1) <= tol:
            n += 1
            df.append(abs(c[0] / fa - 1) * 100)
            dd.append(abs(c[1] - da))
    return {"dips_ref": len(a), "dips_other": len(b), "matched": n, "df_pct_median": round(float(np.median(df)), 2) if df else None,
            "df_pct_max": round(float(max(df)), 2) if df else None, "ddepth_db_median": round(float(np.median(dd)), 2) if dd else None}


def invalid(S):
    """why an openEMS result cannot be used: NaN (a port without excitation) or not passive"""
    cols = [j for j in range(S.shape[2]) if not np.isnan(S[:, :, j]).all()]  # excited (or mirrored) columns
    S = S[:, :, cols]
    if np.isnan(S).any():
        return "NaN (port without excitation)"
    w = np.abs(S) ** 2
    pw = float(w.sum(axis=1).max())
    return f"not passive (largest column power {pw:.3g}: the run grew without bound)" if pw > 1.02 else None


def tdr_minmax(f, s11, z0, tmax=180e-12):
    _, z = sparam.tdr(f, s11, rise=25e-12, z0=z0, tmax=tmax)
    return float(z.min()), float(z.max())


def z_peaks(f, S, z0):
    """frequencies of the |Z11| maxima (G) and the loop inductance at 50 MHz (H)"""
    Z = sparam.s2z(S, z0)[:, 0, 0]
    a = np.abs(Z)
    pk = [float(f[k] / 1e9) for k in range(1, len(f) - 1) if a[k] > a[k - 1] and a[k] > a[k + 1] and a[k] > 2 * np.median(a)]
    k50 = int(np.argmin(np.abs(f - 50e6)))
    return pk, float(Z[k50].imag / (2 * np.pi * f[k50]) * 1e9)


def deembed(f, S, case, tool, ref):
    """the via alone with the 2×thru of each feed layer from the same tool (as via_pbm.py: NZC for
    2-ports, first-order shift for pairs)"""
    keys = [f"thru_{case['meta']['stackup']}_{q['layer']}" for q in case["meta"]["ports"]]
    th = [load(ref, k, tool)[1] for k in keys]
    if any(t is None for t in th):
        return None
    if S.shape[1] == 2:
        FL, _ = sparam.fixture_2xthru(f, th[0])
        FR, _ = sparam.fixture_2xthru(f, th[1])
        return sparam.deembed2(S, FL, FR)
    D = sparam.deembed_shift(S, [sparam.fixture_shift(f, t) for t in th])
    return sparam.mixed_mode(D)["dd"]


def pct(v, q):
    v = [x for x in v if x is not None]
    return round(float(np.percentile(v, q)), 3) if v else None


def compare(a):
    rows = {}
    for n, p in all_cases(a.only).items():
        case = json.load(open(p))
        g = case["meta"]["group"]
        fmax = case["fmax"]
        z0 = case["ports"][0].get("r", 50.0)
        f, K = load(REF, n, "kisi")
        _, O = load(REF, n, "openems")
        _, Ob = load(REF, n, "openems_base")
        if K is None or O is None:
            continue
        row = {"group": g, "fmax_ghz": fmax / 1e9, "ports": K.shape[1], "pair": K.shape[1] == 4 and (g == "E" or "diff" in n or "_m2_" in n)}
        bad = {tag: why for tag, X in (("openems", O), ("openems_base", Ob)) if X is not None and (why := invalid(X))}
        if bad:
            row["invalid"] = bad
            if "openems" in bad and Ob is not None and "openems_base" not in bad:
                O = None  # judge against the base mesh only
            if "openems_base" in bad:
                Ob = None
            if O is None and Ob is None:
                rows[n] = row
                continue
        if O is None:
            O, row["judged_on"] = Ob, "base mesh (the refined openEMS run is invalid)"
        if g in ("G", "H"):
            # one-edge ports (y0 = y1): the refined mesh makes the port itself thinner, so the like-for-like
            # comparison of G and H is the base mesh
            O, row["judged_on"] = (Ob, "base mesh (one-edge ports)") if Ob is not None else (O, "fine mesh")
        row["kisi_vs_openems"] = stats(f, K, O, fmax)
        if Ob is not None and O is not Ob and "openems" not in bad:
            row["kisi_vs_openems_base"] = stats(f, K, Ob, fmax)
            _, Of = load(REF, n, "openems")
            row["openems_base_vs_fine"] = stats(f, Ob, Of, fmax)
            X = 2 * Of - Ob  # first-order extrapolation of the staircase result to zero cell
            row["kisi_vs_openems_extrap"] = stats(f, K, X, fmax)
        # references recomputed on the refined mesh (TESTS.md section 17): the former base-mesh result is kept as
        # kisi_base.sNp – kisi-fdtd's own convergence, and the former reference against openEMS
        _, Kb = load(REF, n, "kisi_base")
        if Kb is not None:
            row["kisi_base_vs_kisi_fine"] = stats(f, Kb, K, fmax)
            row["kisi_base_vs_openems"] = stats(f, Kb, O, fmax)
            if "openems" not in bad:
                row["kisi_fine_vs_openems"] = row["kisi_vs_openems"]
        if a.stair:
            # kisi-fdtd itself on the fine mesh (subset): its own convergence, and like for like with openEMS fine
            _, Kf = load(a.stair, n, "kisi_fine_conformal")
            _, Sf = load(a.stair, n, "kisi_fine_stair")
            _, Of = load(REF, n, "openems")
            if Kf is not None and Kb is None:
                row["kisi_base_vs_kisi_fine"] = stats(f, K, Kf, fmax)
                if Of is not None and not invalid(Of):
                    row["kisi_fine_vs_openems"] = stats(f, Kf, Of, fmax)
            if Sf is not None and Of is not None and not invalid(Of):
                row["stair_fine_vs_openems"] = stats(f, Sf, Of, fmax)
        St = None
        if a.stair:
            _, St = load(a.stair, n, "kisi_stair")
            if St is not None and Ob is not None:
                row["stair_vs_openems_base"] = stats(f, St, Ob, fmax)
                row["stair_vs_kisi"] = stats(f, St, K if Kb is None else Kb, fmax)  # both on the base mesh
        if g in ("G", "H"):
            pk, pko = z_peaks(f, K, z0), z_peaks(f, O, z0)
            row["z_peaks_ghz"] = {"kisi": [round(x, 3) for x in pk[0][:6]], "openems": [round(x, 3) for x in pko[0][:6]]}
            row["L50_nH"] = {"kisi": round(pk[1], 4), "openems": round(pko[1], 4)}
            _, Of = load(REF, n, "openems")
            for tag, X in (("kisi_stair", St), ("openems_fine", Of)):
                if X is not None:
                    q = z_peaks(f, X, z0)
                    row["z_peaks_ghz"][tag] = [round(x, 3) for x in q[0][:6]]
                    row["L50_nH"][tag] = round(q[1], 4)
            if g == "G":
                row["z_peaks_match"] = match_dips([(x, 0.0) for x in pko[0]], [(x, 0.0) for x in pk[0]])
        else:
            i = through_port(K)
            dk_, do_ = dips(f, sparam.db(K[:, i, 0]), fmax), dips(f, sparam.db(O[:, i, 0]), fmax)
            if do_ or dk_:
                row["dips"] = match_dips(do_, dk_)  # openEMS' dips found in kisi-fdtd
                row["dips"]["openems_ghz_db"] = [[round(x, 3), round(y, 2)] for x, y in do_[:12]]
                row["dips"]["kisi_ghz_db"] = [[round(x, 3), round(y, 2)] for x, y in dk_[:12]]
            zk, zo = tdr_minmax(f, K[:, 0, 0], z0), tdr_minmax(f, O[:, 0, 0], z0)
            row["tdr_raw_ohm"] = {"kisi": [round(x, 2) for x in zk], "openems": [round(x, 2) for x in zo]}
        if g in "ABCDEX" and g != "thru":
            Dk, Do = deembed(f, K, case, "kisi", REF), deembed(f, O, case, "openems", REF)
            if Dk is not None and Do is not None:
                m = f <= min(20e9, fmax)
                d21 = np.abs(sparam.db(Dk[m, 1, 0]) - sparam.db(Do[m, 1, 0]))
                ph = np.degrees(np.angle(Dk[m, 1, 0] * np.conj(Do[m, 1, 0])))
                de = {"dS21_mean_db_20GHz": round(float(d21.mean()), 3), "dS21_max_db_20GHz": round(float(d21.max()), 3), "dphase_20GHz_deg": round(float(ph[-1]), 2)}
                if g != "E":
                    zk, zo = (sparam.tdr(f, D[:, 0, 0], rise=25e-12, tmax=80e-12)[1] for D in (Dk, Do))
                    de["tdr_via_ohm"] = {"kisi": [round(float(zk.min()), 2), round(float(zk.max()), 2)], "openems": [round(float(zo.min()), 2), round(float(zo.max()), 2)]}
                    de["tdr_via_err_pct"] = [round(abs(zk.min() / zo.min() - 1) * 100, 2), round(abs(zk.max() / zo.max() - 1) * 100, 2)]
                nk, no = (sparam.db(D[:, 1, 0]) for D in (Dk, Do))
                if no.min() < -10:
                    de["notch_ghz"] = {"kisi": round(float(f[np.argmin(nk)] / 1e9), 2), "openems": round(float(f[np.argmin(no)] / 1e9), 2)}
                # kisi's ladder against both 3D results (ports at the via)
                fl, L = load(REF, n, "ladder")
                if L is not None and len(fl) == len(f):
                    for tag, D in (("kisi", Dk), ("openems", Do)):
                        dl = np.abs(sparam.db(D[m, 1, 0]) - sparam.db(L[m, 1, 0]))
                        e = {"dS21_max_db": round(float(dl.max()), 3), "dphase_max_deg": round(float(np.max(np.abs(np.degrees(np.angle(D[m, 1, 0] * np.conj(L[m, 1, 0])))))), 2)}
                        if g != "E":
                            _, z3 = sparam.tdr(f, D[:, 0, 0], rise=25e-12, tmax=80e-12)
                            _, zm = sparam.tdr(f, L[:, 0, 0], rise=25e-12, tmax=80e-12)
                            e["tdr_err_pct"] = [round(abs(zm.min() / z3.min() - 1) * 100, 2), round(abs(zm.max() / z3.max() - 1) * 100, 2)]
                        de[f"ladder_vs_{tag}"] = e
                row["deembedded"] = de
        # kisi's fast channel model (kisi_model.sNp, full channel with the feeds) against both: as compare_kisi.py,
        # |S21| up to 20 GHz (or 0.8 × the first notch), TDR minimum and maximum with a 35 ps edge
        fm, M = load(REF, n, "kisi_model")
        if M is not None and M.shape[1] == 2 and K.shape[1] == 2 and g not in ("G", "H", "thru"):
            km = {}
            nf_ = len(fm)
            for tag, R in (("kisi", K[:nf_]), ("openems", O[:nf_])):
                notches = [x[0] * 1e9 for x in dips(fm, sparam.db(R[:, 1, 0]), fmax, prom=10.0)]
                flim = min([20e9] + [0.8 * x for x in notches])
                mm_ = fm <= flim
                _, z3 = sparam.tdr(fm, R[:, 0, 0], rise=35e-12, tmax=150e-12)
                _, zk = sparam.tdr(fm, M[:, 0, 0], rise=35e-12, tmax=150e-12)
                km[tag] = {"dS21_max_db": round(float(np.max(np.abs(sparam.db(R[mm_, 1, 0]) - sparam.db(M[mm_, 1, 0])))), 3),
                           "tdr_err_pct": [round(abs(zk.min() / z3.min() - 1) * 100, 2), round(abs(zk.max() / z3.max() - 1) * 100, 2)]}
            row["kisi_model"] = km
        meta = json.load(open(os.path.join(REF, n, "meta.json")))["runs"]
        row["runs"] = {k: {"cells": v["cells"], "seconds": v["seconds"] if isinstance(v["seconds"], (int, float)) else round(sum(v["seconds"]), 1)}
                       for k, v in meta.items() if k in ("kisi", "openems", "openems_base")}
        rows[n] = row
    # per group
    groups = {}
    for n, r in rows.items():
        groups.setdefault(r["group"], []).append(r)
    summ = {}
    for g, rr in sorted(groups.items()):
        s = {"cases": len(rr)}
        for key in ("kisi_vs_openems", "kisi_vs_openems_base", "openems_base_vs_fine", "kisi_vs_openems_extrap", "stair_vs_openems_base",
                    "kisi_base_vs_kisi_fine", "kisi_fine_vs_openems", "stair_fine_vs_openems"):
            v = [r[key] for r in rr if key in r and "dS_thru_mean_db" in r[key]]
            if v:
                s[key] = {"dS_thru_mean_db": pct([x["dS_thru_mean_db"] for x in v], 50), "dS_thru_max_db": round(max(x["dS_thru_max_db"] for x in v), 3),
                          "dS_thru_mean_db_max": round(max(x["dS_thru_mean_db"] for x in v), 3),
                          "dS11_mean_db": pct([x["dS11_mean_db"] for x in v], 50), "dtau_ps_median": pct([x["dtau_ps"] for x in v], 50),
                          "dphase_fmax_deg_median": pct([x["dphase_fmax_deg"] for x in v], 50)}
        de = [r["deembedded"] for r in rr if "deembedded" in r]
        if de:
            s["deembedded"] = {"dS21_mean_db_median": pct([x["dS21_mean_db_20GHz"] for x in de], 50), "dS21_max_db_max": round(max(x["dS21_max_db_20GHz"] for x in de), 3),
                               "tdr_via_err_pct_median": pct([e for x in de for e in x.get("tdr_via_err_pct", [])], 50),
                               "tdr_via_err_pct_max": round(max([e for x in de for e in x.get("tdr_via_err_pct", [])] or [0]), 2)}
        summ[g] = s
    # kisi's ladder judged against either reference (single vias: TDR error, max Δ|S21| to 20 GHz)
    lad = {}
    for tag in ("kisi", "openems"):
        sg = [r["deembedded"][f"ladder_vs_{tag}"] for r in rows.values() if r["group"] in "ABCDX" and f"ladder_vs_{tag}" in r.get("deembedded", {})]
        pr = [r["deembedded"][f"ladder_vs_{tag}"] for r in rows.values() if r["group"] == "E" and f"ladder_vs_{tag}" in r.get("deembedded", {})]
        lad[tag] = {"single_cases": len(sg), "single_tdr_err_pct_median": pct([e for x in sg for e in x["tdr_err_pct"]], 50),
                    "single_tdr_err_pct_p90": pct([e for x in sg for e in x["tdr_err_pct"]], 90),
                    "single_dS21_max_db_median": pct([x["dS21_max_db"] for x in sg], 50), "pairs_dS21_max_db_median": pct([x["dS21_max_db"] for x in pr], 50),
                    "pairs_dphase_max_deg_median": pct([x["dphase_max_deg"] for x in pr], 50)}
    for tag in ("kisi", "openems"):
        km = [r["kisi_model"][tag] for r in rows.values() if "kisi_model" in r]
        lad[f"channel_vs_{tag}"] = {"cases": len(km), "dS21_max_db_median": pct([x["dS21_max_db"] for x in km], 50),
                                    "tdr_err_pct_median": pct([e for x in km for e in x["tdr_err_pct"]], 50),
                                    "tdr_err_pct_p90": pct([e for x in km for e in x["tdr_err_pct"]], 90)}
    inval = {n: r["invalid"] for n, r in rows.items() if r.get("invalid")}
    out = {"fine_factor": FINE, "summary": summ, "ladder": lad, "invalid": inval, "cases": rows}
    json.dump(out, open(os.path.join(HERE, "results", "openems_refs.json"), "w"), indent=1)
    print(json.dumps({"summary": summ, "ladder": lad}, indent=1))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["plan", "run", "merge", "stair", "compare"])
    ap.add_argument("dirs", nargs="*")
    ap.add_argument("--exe", default=os.environ.get("KISI_FDTD", "kisi-fdtd"))
    ap.add_argument("--only", default="")
    ap.add_argument("--plan", default=PLAN)
    ap.add_argument("--shards", type=int, default=40)
    ap.add_argument("--meshes", default="base,fine")
    ap.add_argument("--big", type=float, default=2.5, help="tasks estimated above this many hours run alone with 4 threads")
    ap.add_argument("--shard", type=int, default=0)
    ap.add_argument("--out", default=os.path.join(HERE, "openems_raw"))
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--stair", default=None)
    ap.add_argument("--mesh", choices=["base", "fine"], default="base")
    ap.add_argument("--metal", choices=["stair", "conformal"], default="stair")
    ap.add_argument("--version", default="0.37.0")
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    a = ap.parse_args()
    a.only = [k for k in a.only.split(",") if k]
    {"plan": plan, "run": run, "merge": merge, "stair": stair, "compare": compare}[a.cmd](a)


if __name__ == "__main__":
    main()
