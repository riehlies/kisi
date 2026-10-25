"""Practical cost of kisi's models on the example boards (ThunderScope, LibreVNA): run time and
memory of the commands with the via ladder and with the old coax via model, the field solver
(kisi.mjs) cold and cached, the bundle sizes, how many vias the ladder handles and how often the
Laplace solve is shared, and whether the channel verdicts change. The 3D FDTD cost comes from
the reference runs (validation/via_ref/*/meta.json) and one measured run.

usage: python3 practice.py --kisi target/release/kisi --mjs dist/kisi.mjs [--mjs-old OLD/kisi.mjs]
                           [--examples examples] [--fdtd target/release/kisi-fdtd]
       → results/practice.json
"""
import argparse, glob, json, os, re, shutil, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
BOARDS = [("ts", "Thunderscope_Rev5.3"), ("vna", "VNA")]


def run(cmd, env=None, cwd=None):
    """wall time (s), peak RSS of the child (MB), stdout, stderr"""
    probe = ("import resource, subprocess, sys, time, json\n"
             "t = time.perf_counter(); r = subprocess.run(sys.argv[1:], capture_output=True, text=True)\n"
             "dt = time.perf_counter() - t\n"
             "sys.stderr.write(r.stderr); sys.stdout.write(r.stdout)\n"
             "open(__import__('os').environ['PROBE_OUT'], 'w').write(json.dumps([dt, resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss / 1024, r.returncode]))\n")
    fd, po = tempfile.mkstemp()
    os.close(fd)
    e = dict(os.environ, **(env or {}), PROBE_OUT=po)
    r = subprocess.run([sys.executable, "-c", probe] + cmd, capture_output=True, text=True, env=e, cwd=cwd)
    dt, rss, rc = json.load(open(po))
    os.unlink(po)
    if rc != 0:
        raise RuntimeError(f"{cmd} failed: {r.stderr[-400:]}")
    return dt, rss, r.stdout, r.stderr


def best(n, cmd, env=None, cwd=None):
    """fastest of n runs (wall time, RSS, stdout, stderr of that run)"""
    rs = [run(cmd, env, cwd) for _ in range(n)]
    return min(rs, key=lambda x: x[0])


def board_stats(pcb):
    t = open(pcb).read()
    return {"file_mb": round(os.path.getsize(pcb) / 1e6, 2), "vias": len(re.findall(r"\(via[ \n]", t)),
            "segments": len(re.findall(r"\(segment[ \n]", t)), "zones_filled": len(re.findall(r"\(filled_polygon", t)),
            "footprints": len(re.findall(r"\(footprint[ \n]", t))}


def ladder_stats(stderr):
    builds = [l for l in stderr.splitlines() if l.startswith("# ladder build")]
    ms = [float(l.split()[3]) for l in builds]
    solved = [m for m in ms if m > 1.0]  # the cached ones take microseconds
    unk = [int(l.split()[5]) for l in builds]
    return {"ladder_vias": len(builds), "laplace_solves": len(solved), "laplace_ms_total": round(sum(solved), 1),
            "laplace_ms_median": round(statistics.median(solved), 1) if solved else None,
            "laplace_unknowns_median": int(statistics.median(unk)) if unk else None}


def verdicts(out):
    d = json.loads(out)
    key = lambda i, r: r.get("lane") or "+".join(r.get("nets") or []) or f"row{i}"
    return {key(i, r): (r.get("status"), r.get("metrics") or {}) for i, r in enumerate(d["rows"])}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kisi", default=os.path.join(ROOT, "target", "release", "kisi"))
    ap.add_argument("--mjs", default=os.path.join(ROOT, "dist", "kisi.mjs"))
    ap.add_argument("--mjs-old", default=None, help="kisi.mjs of the previous version (coax via model)")
    ap.add_argument("--html", default=os.path.join(ROOT, "dist", "kisi.html"))
    ap.add_argument("--html-old", default=None)
    ap.add_argument("--examples", default=os.path.join(ROOT, "examples"))
    ap.add_argument("--fdtd", default=os.path.join(ROOT, "target", "release", "kisi-fdtd"))
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--wasm-dir", default=os.path.join(ROOT, "target", "wasm32-unknown-unknown", "release"))
    a = ap.parse_args()
    out = {"machine": {"cpus": os.cpu_count(), "cpu": next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name")), "?")}, "boards": {}}
    for key, name in BOARDS:
        pcb = os.path.join(a.examples, name + ".kicad_pcb")
        cfg = os.path.join(ROOT, "web", "samples", key + ".kisi.json")
        if not os.path.exists(pcb):
            continue
        b = {"board": board_stats(pcb)}
        common = ["--config", cfg]
        t, rss, o, _ = best(a.runs, [a.kisi, "overview", pcb] + common)
        b["native_overview"] = {"s": round(t, 2), "rss_mb": round(rss)}
        t, rss, ol, _ = best(a.runs, [a.kisi, "check", pcb, "--all"] + common)
        tc, rssc, oc, _ = best(a.runs, [a.kisi, "check", pcb, "--all"] + common, env={"KISI_VIA_MODEL": "coax"})
        _, _, _, err = run([a.kisi, "check", pcb, "--all"] + common, env={"KISI_DEBUG_VIA": "1"})
        b["native_check_all"] = {"ladder_s": round(t, 2), "coax_s": round(tc, 2), "rss_mb": [round(rss), round(rssc)], **ladder_stats(err)}
        # verdicts: does the via model change the channel check?
        vl, vc = verdicts(ol), verdicts(oc)
        changed = {k: [vc[k][0], vl[k][0]] for k in vl if k in vc and vl[k][0] != vc[k][0]}
        # lanes that reflect almost everything in both models (RL < 3 dB: an unmodelled part or an
        # open end) are left out of the metric spread: there a small change moves the standing waves
        both = [k for k in vl if k in vc]
        refl = [k for k in both if min(vl[k][1].get("rl_db") or 99, vc[k][1].get("rl_db") or 99) < 3]
        ok = [k for k in both if k not in refl]
        dd = lambda m: [vl[k][1][m] - vc[k][1][m] for k in ok if vl[k][1].get(m) is not None and vc[k][1].get(m) is not None]
        drl, dil, deye = dd("rl_db"), dd("il_nyq_db"), dd("eye_h")
        b["verdicts"] = {"lanes": len(vl), "status_changed": changed, "lanes_reflective": len(refl),
                         "d_rl_db": [round(min(drl), 2), round(max(drl), 2)] if drl else None,
                         "d_il_nyq_db": [round(min(dil), 3), round(max(dil), 3)] if dil else None,
                         "d_eye_h": [round(min(deye), 3), round(max(deye), 3)] if deye else None}
        # one channel with vias: the lane with the most vias in the ladder log is not known here,
        # so take the first differential lane of the check
        lane = next((r["lane"] for r in json.loads(ol)["rows"] if r.get("diff") and r.get("lane")), None)
        if lane:
            t, _, _, _ = best(a.runs, [a.kisi, "net", pcb, lane] + common)
            tc, _, _, _ = best(a.runs, [a.kisi, "net", pcb, lane] + common, env={"KISI_VIA_MODEL": "coax"})
            b["native_net"] = {"net": lane, "ladder_s": round(t, 3), "coax_s": round(tc, 3)}
        # Node CLI (WASM): formula, field solver cold (fresh copy of the board, no cache) and cached
        if os.path.exists(a.mjs):
            t, rss, _, _ = best(a.runs, ["node", a.mjs, "check", pcb, "--all", "--formula"] + common)
            b["node_check_all_formula"] = {"s": round(t, 2), "rss_mb": round(rss)}
            if a.mjs_old and os.path.exists(a.mjs_old):
                t, rss, _, _ = best(a.runs, ["node", a.mjs_old, "check", pcb, "--all", "--formula"] + common)
                b["node_check_all_formula_old"] = {"s": round(t, 2), "rss_mb": round(rss)}
            # field solver: empty cache (KISI_CACHE), the default time budget (120 s of new solves), then
            # the same command without new solves (--field-time 0) from the cache; the rate of the first run
            # gives the time for all planned cross-sections
            tmp = tempfile.mkdtemp()
            env = {"KISI_CACHE": tmp}
            t, rss, o, err = run(["node", a.mjs, "check", pcb, "--all"] + common, env=env)
            fs = json.loads(o).get("field_solver") or {}
            prog = re.findall(r"field solver (\d+)/(\d+) new cross-sections, (\d+) % of the length covered, (\d+) s", err)
            t2, _, o2, _ = run(["node", a.mjs, "check", pcb, "--all", "--field-time", "0"] + common, env=env)
            fs2 = json.loads(o2).get("field_solver") or {}
            cf = os.path.join(tmp, "field-cache.json")
            solved, planned, cov, secs = (int(x) for x in prog[-1]) if prog else (0, 0, 0, 0)
            rate = solved / secs if secs else None
            b["node_check_all_field"] = {"cold_s": round(t, 1), "cached_s": round(t2, 1), "rss_mb": round(rss),
                                         "planned_new": planned, "solved_in_budget": solved, "target_length_covered_pct": cov,
                                         "cross_sections_field": fs.get("cross_sections_field"), "cross_sections": fs.get("cross_sections"),
                                         "coverage_cached_pct": fs2.get("target_length_coverage_pct"),
                                         "solves_per_min": round(rate * 60, 1) if rate else None,
                                         "all_planned_min_estimate": round(planned / rate / 60, 1) if rate else None,
                                         "cache_file_kb": round(os.path.getsize(cf) / 1e3, 1) if os.path.exists(cf) else None}
            shutil.rmtree(tmp)
        out["boards"][key] = b
        print(key, json.dumps(b, indent=1), flush=True)
    # bundle sizes
    sz = lambda p: round(os.path.getsize(p) / 1e3) if p and os.path.exists(p) else None
    out["bundle_kb"] = {"kisi.html": sz(a.html), "kisi.html_old": sz(a.html_old), "kisi.mjs": sz(a.mjs), "kisi.mjs_old": sz(a.mjs_old)}
    for w in ("kisi_wasm", "kisi_sparse"):
        p = os.path.join(a.wasm_dir, w + ".wasm")
        out["bundle_kb"][w + ".wasm"] = sz(p)
    # 3D FDTD: reference runs (one thread each, two runs in parallel on the machine above)
    rows = []
    for mp in glob.glob(os.path.join(os.path.dirname(HERE), "via_ref", "*", "meta.json")):
        m = json.load(open(mp))
        r = m["runs"]["kisi"]
        steps = sum(r["steps"]) if isinstance(r["steps"], list) else r["steps"]
        rows.append({"case": m["case"], "group": m["case"].split("_")[0], "cells": r["cells"], "s": r["seconds"], "runs": len(r["steps"]) if isinstance(r["steps"], list) else 1,
                     "mcells_per_s": r["cells"] * steps / r["seconds"] / 1e6})
    grp = {}
    for g in sorted({r["group"] for r in rows}):
        v = [r for r in rows if r["group"] == g]
        grp[g] = {"cases": len(v), "cells_median": int(statistics.median(r["cells"] for r in v)), "s_median": round(statistics.median(r["s"] for r in v)),
                  "s_max": round(max(r["s"] for r in v)), "mcells_per_s_median": round(statistics.median(r["mcells_per_s"] for r in v), 1)}
    out["fdtd"] = {"groups": grp, "total_cpu_h": round(sum(r["s"] for r in rows) / 3600, 2)}
    # one measured run for memory: the smallest thru case
    if os.path.exists(a.fdtd):
        case = os.path.join(HERE, "via_cases", "thru_s4_L1.json")
        t, rss, o, _ = run([a.fdtd, case])
        cells = json.loads(o).get("cells")
        out["fdtd"]["measured"] = {"case": "thru_s4_L1", "s": round(t, 1), "rss_mb": round(rss), "cells": cells,
                                   "bytes_per_cell": round(rss * 1e6 / cells) if cells else None}
    # reference data on disk
    vr = os.path.join(os.path.dirname(HERE), "via_ref")
    tot = sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fn in os.walk(vr) for f in fn)
    fields = sum(os.path.getsize(p) for p in glob.glob(os.path.join(vr, "*", "fields.json")))
    out["reference_data_mb"] = {"via_ref_total": round(tot / 1e6, 1), "fields_json": round(fields / 1e6, 1)}
    json.dump(out, open(os.path.join(HERE, "results", "practice.json"), "w"), indent=1)
    print(json.dumps({k: out[k] for k in ("bundle_kb", "fdtd", "reference_data_mb")}, indent=1))


if __name__ == "__main__":
    main()
