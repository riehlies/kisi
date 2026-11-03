#!/usr/bin/env python3
"""The full-wave (triangular FEM) backend of js_2d_fields on the cases of cases.json →
validation/cst_xs/fullwave/<id>.json (README.md, "Full-wave backend").

    python3 validation/cst_xs/run_fullwave.py [--ids a,b | --tier 1 | --group g | --plan fullwave_plan.json --shard K]
        [--kinds E,M,sweep] [--jobs N] [--wasm-dir target/forsten-fw/dist] [--out validation/cst_xs/fullwave]

Kinds (each a node process of its own, fullwave_driver.mjs → web/solver/backend_forsten_fw.js):
  E      lossless at 1 GHz in CST's box with electric walls (xsgeo.boxed_text "gnd", as kisi.json field_E):
         tanδ 0, static P2 FEM and full-wave eigenmode on the same mesh (Z0 and εeff with ideal conductors)
  M      the same with natural walls = CST's magnetic walls ("open", ground straps; as field_M; not for cases
         whose grounds touch nothing, see xsgeo.isolated_grounds)
  sweep  lossy, electric walls, CST's frequency grid of the lossy runs (cst_xs.sweep_freqs(10, 0.01, 40): 10 per
         decade 10 MHz-40 GHz plus 5, 20, 40 GHz): tanδ of the geometry with Djordjevic-Sarkar dispersion (εr and
         tanδ at 1 GHz, as CST's TanDFreq 1 GHz), copper σ 5.8e7 S/m, smooth, MQS conductor loss (upstream's
         default): Z0(f), complex Zc, εeff(f), R/L/G/C(f), αc, αd per mode, Zdiff/Zcomm and the 2×2 matrices of
         a pair
The geometry is the one CST solved: `cst_text` where a case has one (trapezoids of the etch group), else `text`.

Results already in --out or validation/cst_xs/fullwave/ for the same geometry and settings are skipped
(--force runs them again). --deadline-min stops starting new tasks (CI shards). The runner's CPU, node version,
the WASM build and the wall time of every task are stored in each result.
"""
import argparse, json, os, platform, re, subprocess, sys, tempfile, threading, time
from concurrent.futures import ThreadPoolExecutor, as_completed

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import xsgeo  # noqa: E402
from cst_xs import sweep_freqs, text_hash  # noqa: E402  (the CST runner's frequency grid and geometry hash)

DEFAULT_OUT = os.path.join(HERE, "fullwave")
SETTINGS = {  # changing one of these makes earlier results stale
    "point": {"f0": 1e9, "tol": 0.002, "maxNodes": 96000, "lossMethod": "static", "statics": True},
    "sweep": {"tol": 0.002, "maxNodes": 96000, "lossMethod": "auto", "causal": True, "sigma": 5.8e7, "rq": 0},
    "sweep_grid": {"per_decade": 10, "fmin_ghz": 0.01, "fmax_ghz": 40.0},
}


def host():
    cpu = None
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        cpu = platform.processor()
    return {"cpu": cpu, "cpus": os.cpu_count(), "machine": platform.machine(), "system": platform.system(),
            "runner": os.environ.get("RUNNER_NAME"), "run_id": os.environ.get("GITHUB_RUN_ID")}


def lossless(text):
    return re.sub(r"\btand=[^\s]+", "tand=0", text)


def tasks_of(case, kinds, settings):
    """[(kind, task dict)] of a case"""
    text = case.get("cst_text") or case["text"]
    box = case["domain"]["box"]
    out = []
    if "E" in kinds:
        out.append(("E", {"kind": "point", "text": lossless(xsgeo.boxed_text(text, box, "gnd")), "opts": settings["point"]}))
    if "M" in kinds and not xsgeo.isolated_grounds(xsgeo.parse(text), box):
        out.append(("M", {"kind": "point", "text": lossless(xsgeo.boxed_text(text, box, "open")), "opts": settings["point"]}))
    if "sweep" in kinds:
        g = settings["sweep_grid"]
        freqs = [f * 1e9 for f in sweep_freqs(g["per_decade"], g["fmin_ghz"], g["fmax_ghz"])]
        out.append(("sweep", {"kind": "sweep", "text": xsgeo.boxed_text(text, box, "gnd"), "freqs": freqs, "opts": settings["sweep"]}))
    return out


def load(path):
    try:
        return json.load(open(path, encoding="utf-8"))
    except (OSError, ValueError):
        return None


def done_kinds(case, dirs, settings):
    """kinds with a valid result for this geometry and settings in one of dirs → (doc, set)"""
    th = text_hash(case.get("cst_text") or case["text"])
    for d in dirs:
        doc = load(os.path.join(d, case["id"] + ".json"))
        if doc and doc.get("text_hash") == th and doc.get("settings") == settings:
            return doc, {k for k, v in doc.get("results", {}).items() if v.get("ok")}
    return None, set()


def run_task(task, wasm_dir, timeout_s, tmp):
    fd, pin = tempfile.mkstemp(suffix=".json", dir=tmp)
    with os.fdopen(fd, "w") as fh:
        json.dump(dict(task, wasm_dir=wasm_dir), fh)
    pout = pin[:-5] + ".out.json"
    t0 = time.time()
    try:
        p = subprocess.run(["node", "--max-old-space-size=4096", os.path.join(HERE, "fullwave_driver.mjs"), pin, pout],
                           capture_output=True, text=True, timeout=timeout_s, cwd=ROOT)
        r = load(pout)
        if r is None:
            r = {"ok": False, "error": f"driver exit {p.returncode}: {(p.stderr or p.stdout)[-1500:]}"}
    except subprocess.TimeoutExpired:
        r = {"ok": False, "error": f"timeout after {timeout_s} s"}
    r["task_s"] = round(time.time() - t0, 1)
    for f in (pin, pout):
        if os.path.exists(f):
            os.remove(f)
    return r


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--ids", default="")
    ap.add_argument("--tier", default="")
    ap.add_argument("--group", default="")
    ap.add_argument("--plan", help="fullwave_plan.json: ids (or groups / all) and the number of shards")
    ap.add_argument("--shard", type=int, help="with --plan: run shard K of the plan's shards")
    ap.add_argument("--kinds", default="", help="E,M,sweep (default: the plan's, else all three)")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1, help="parallel node processes (default: one per CPU)")
    ap.add_argument("--wasm-dir", default=os.environ.get("KISI_FW_WASM_DIR", os.path.join(ROOT, "target/forsten-fw/dist")))
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--timeout-min", type=float, default=120)
    ap.add_argument("--deadline-min", type=float, default=0, help="start no new task after this many minutes")
    ap.add_argument("--tol", type=float, help="override the mesh tolerance (results then carry other settings)")
    ap.add_argument("--max-nodes", type=int)
    a = ap.parse_args()
    t_start = time.time()
    settings = json.loads(json.dumps(SETTINGS))
    for k in ("point", "sweep"):
        if a.tol:
            settings[k]["tol"] = a.tol
        if a.max_nodes:
            settings[k]["maxNodes"] = a.max_nodes
    cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
    plan = load(a.plan) if a.plan else None
    kinds = [k for k in (a.kinds or ",".join((plan or {}).get("kinds", ["E", "M", "sweep"]))).split(",") if k]
    if plan:
        sel = set(plan.get("ids", []))
        groups = set(plan.get("groups", []))
        cases = [c for c in cases if plan.get("all") or c["id"] in sel or c["group"] in groups]
        if a.shard is not None:
            n = plan["shards"]
            # round robin over the cases ordered by expected cost (pairs and wide boxes are slower), so every
            # shard gets a similar mix
            cases.sort(key=lambda c: (c.get("n_signals", 1), (c["domain"]["box"][1] - c["domain"]["box"][0]), c["id"]))
            cases = cases[a.shard::n]
    if a.ids:
        want = set(a.ids.split(","))
        cases = [c for c in cases if c["id"] in want]
    if a.tier:
        cases = [c for c in cases if str(c["tier"]) in a.tier.split(",")]
    if a.group:
        cases = [c for c in cases if c["group"] in a.group.split(",")]
    os.makedirs(a.out, exist_ok=True)
    info_p = os.path.join(a.wasm_dir, "BUILD_INFO.json")
    build = load(info_p) or {}
    build = {k: build.get(k) for k in ("upstream_rev", "emsdk", "emcc", "occt", "submodules")} | {
        "sha256": {f: v["sha256"][:16] for f, v in (build.get("files") or {}).items()}}
    hst = host()
    node_v = subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip()
    upstream_rev = open(os.path.join(ROOT, "third_party/UPSTREAM_REV")).read().strip()

    docs, todo = {}, []
    for c in cases:
        prev, have = (None, set()) if a.force else done_kinds(c, [a.out, DEFAULT_OUT], settings)
        th = text_hash(c.get("cst_text") or c["text"])
        doc = prev if prev else {
            "id": c["id"], "group": c["group"], "tier": c["tier"], "n_signals": c.get("n_signals", 1),
            "geometry": "cst_text" if c.get("cst_text") else "text", "text_hash": th, "box": c["domain"]["box"],
            "backend": "js_2d_fields triangular FEM (web/solver/backend_forsten_fw.js)", "upstream_rev": upstream_rev,
            "settings": settings, "results": {}, "seconds": {}}
        docs[c["id"]] = doc
        for kind, t in tasks_of(c, kinds, settings):
            if kind not in have:
                todo.append((c["id"], kind, t))
    print(f"{len(cases)} cases, {len(todo)} tasks ({','.join(kinds)}), {a.jobs} jobs, wasm {a.wasm_dir}", flush=True)
    if not todo:
        return
    left = {cid: sum(1 for x in todo if x[0] == cid) for cid in docs}
    lock = threading.Lock()
    tmp = tempfile.mkdtemp(prefix="fullwave_")
    timeout_s = int(a.timeout_min * 60)

    def write(cid):
        d = docs[cid]
        d["date"] = time.strftime("%Y-%m-%d")
        d["build"], d["host"], d["node"] = build, hst, node_v
        d["status"] = "ok" if d["results"] and all(v.get("ok") for v in d["results"].values()) else "failed"
        with open(os.path.join(a.out, cid + ".json"), "w", encoding="utf-8") as fh:
            json.dump(d, fh, indent=1)
            fh.write("\n")

    def work(cid, kind, t):
        if a.deadline_min and time.time() - t_start > a.deadline_min * 60:
            return cid, kind, None
        return cid, kind, run_task(t, a.wasm_dir, timeout_s, tmp)

    n_done = n_fail = 0
    with ThreadPoolExecutor(max_workers=a.jobs) as ex:
        futs = [ex.submit(work, *x) for x in todo]
        for f in as_completed(futs):
            cid, kind, r = f.result()
            with lock:
                if r is None:
                    continue
                docs[cid]["results"][kind] = r
                docs[cid]["seconds"][kind] = r.get("task_s")
                left[cid] -= 1
                n_done += 1
                n_fail += 0 if r.get("ok") else 1
                write(cid)
                z = r.get("z0") or r.get("zodd")
                msg = (f"Z {z:.3f}" if isinstance(z, (int, float)) else "") if kind != "sweep" else \
                    (f"{len(r.get('f', []))} freqs" if r.get("ok") else "")
                print(f"[{time.time() - t_start:7.0f} s] {n_done}/{len(todo)} {cid} {kind}: "
                      f"{'ok' if r.get('ok') else 'FAILED ' + str(r.get('error'))[:200]} {msg} ({r.get('task_s')} s)", flush=True)
    skipped = len(todo) - n_done
    print(f"done: {n_done} tasks, {n_fail} failed, {skipped} not started (deadline), {time.time() - t_start:.0f} s")


if __name__ == "__main__":
    main()
