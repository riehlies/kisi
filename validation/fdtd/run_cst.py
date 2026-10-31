"""Run kisi reference cases (via_cases/*.json) in CST Studio Suite and store the results per case:

  <out>/<case>/cst.sNp        S-parameters on the case's frequency grid (f = fmax (i+1)/nf),
                              header with CST version, solver, mesh cells and run time
  <out>/<case>/cst_meta.json  settings, port types used, mesh cells, run time, CST log excerpts

Runs with a Python that can import CST's `cst` package (CST's python_cst_libraries on PYTHONPATH):

  python run_cst.py case.json [case.json ...] --out OUT --work WORK
        [--port face|edge] [--lpw 20] [--lpb 20] [--minstep 0] [--tag TAG] [--skip-done] [--probe] [--no-solve]

--tag names the outputs cst_TAG.sNp / cst_TAG_meta.json (e.g. a mesh study) instead of cst.sNp.

The model is built by cst_case.py as VBA history blocks, added one by one to a new CST project
(so a failing block is reported with its caption). A discrete face port that CST cannot build falls
back to a discrete edge port (recorded in cst_meta.json). Mesh settings are reported, never lowered
silently: a case that exceeds the available mesh size is reported as not run.
Only the standard library is used (CST's Python may lack numpy).
"""
import argparse, datetime, glob, json, os, re, sys, time, traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import cst_case  # noqa: E402


for _s in (sys.stdout, sys.stderr):  # the console may not be UTF-8; case titles hold arrows etc.
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def cst_import():
    import cst  # noqa: F401
    import cst.interface
    import cst.results
    return cst


def probe(cst, out):
    """write the API of the CST Python package (for the generator's author)"""
    import pydoc
    p = os.path.join(out, "cst_api.txt")
    with open(p, "w", encoding="utf-8") as fh:
        fh.write(f"python {sys.version}\ncst package: {getattr(cst, '__file__', '?')}\nversion: {getattr(cst, '__version__', '?')}\n\n")
        for mod in ("cst.interface", "cst.results"):
            try:
                fh.write(pydoc.render_doc(mod, renderer=pydoc.plaintext))
            except Exception as e:
                fh.write(f"{mod}: {e}\n")
    log("API written to", p)


def start_de(cst):
    """reuse a running CST design environment (a start can be held up by a modal start-up
    notice that waits for a click), else start one"""
    di = cst.interface.DesignEnvironment
    for how in ("connect_to_any", "new", "ctor"):
        try:
            de = di.connect_to_any() if how == "connect_to_any" else di.new() if how == "new" else di()
            log("design environment via", how)
            return de
        except Exception as e:
            log("design environment", how, "failed:", repr(e))
    raise RuntimeError("cannot start the CST design environment")


def interp(xs, ys, x):
    """linear interpolation of complex ys (xs ascending)"""
    if x <= xs[0]:
        return ys[0]
    if x >= xs[-1]:
        return ys[-1]
    lo, hi = 0, len(xs) - 1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if xs[mid] <= x:
            lo = mid
        else:
            hi = mid
    t = (x - xs[lo]) / (xs[hi] - xs[lo])
    return ys[lo] + t * (ys[hi] - ys[lo])


def write_touchstone(path, freq, S, comments, z0):
    """S[k][i][j]; same layout as sparam.write_touchstone"""
    n = len(S[0])
    with open(path, "w", encoding="utf-8") as fh:
        for c in comments:
            fh.write(f"! {c}\n")
        fh.write(f"# Hz S RI R {z0:g}\n")
        for k, f in enumerate(freq):
            if n == 2:
                vals = [S[k][0][0], S[k][1][0], S[k][0][1], S[k][1][1]]
                fh.write(f"{f:.6e} " + " ".join(f"{v.real:.6e} {v.imag:.6e}" for v in vals) + "\n")
            else:
                for i in range(n):
                    row = [S[k][i][j] for j in range(n)]
                    for li in range(0, n, 4):
                        head = f"{f:.6e} " if (i == 0 and li == 0) else "  "
                        fh.write(head + " ".join(f"{v.real:.6e} {v.imag:.6e}" for v in row[li:li + 4]) + "\n")


def solver_logs(prj_dir):
    """the CST log files of a project (text), newest first"""
    files = sorted(glob.glob(os.path.join(prj_dir, "**", "*.log"), recursive=True), key=os.path.getmtime, reverse=True)
    res = {}
    for f in files[:12]:
        try:
            res[os.path.relpath(f, prj_dir)] = open(f, encoding="utf-8", errors="replace").read()
        except OSError:
            pass
    return res


def find_int(texts, patterns):
    for t in texts:
        for p in patterns:
            mm = re.search(p, t, re.I)
            if mm:
                return int(mm.group(1).replace(",", "").replace(".", "").replace(" ", ""))
    return None


def read_sparams(cst, prj_path, nports):
    pf = cst.results.ProjectFile(prj_path, allow_interactive=True)
    r3 = pf.get_3d()
    items = [t for t in r3.get_tree_items() if "S-Parameters" in t]
    found = {}
    for t in items:
        leaf = t.split("\\")[-1]
        mm = re.fullmatch(r"S(\d+)(?:\(1\))?,(\d+)(?:\(1\))?", leaf)
        if mm:
            found[(int(mm.group(1)) - 1, int(mm.group(2)) - 1)] = t
    if len(found) < nports * nports:
        raise RuntimeError(f"S-parameters missing: found {sorted(found)} in {items[:40]}")
    data = {}
    for (i, j), t in found.items():
        it = r3.get_result_item(t)
        data[(i, j)] = ([float(x) for x in it.get_xdata()], [complex(y) for y in it.get_ydata()])
    return data, items


def run_case(cst, de, path, a):
    case = json.load(open(path, encoding="utf-8"))
    name = case["name"]
    n = len(case["ports"])
    z0 = case["ports"][0].get("r", 50.0)
    od = os.path.join(a.out, name)
    os.makedirs(od, exist_ok=True)
    stem = f"cst_{a.tag}" if a.tag else "cst"
    wd = os.path.abspath(os.path.join(a.work, name + (f"_{a.tag}" if a.tag else "")))
    os.makedirs(wd, exist_ok=True)
    mesh = {"lpw": a.lpw, "lpb": a.lpb, "minstep": a.minstep, "ratio": a.ratio}
    blocks = cst_case.build(case, a.port, mesh)
    open(os.path.join(wd, name + ".bas"), "w", encoding="utf-8").write(cst_case.to_bas(blocks))
    meta = {"case": name, "tag": a.tag, "date": datetime.date.today().isoformat(), "tool": "CST Studio Suite", "solver": "time domain (FIT, hexahedral, PBA)",
            "mesh_settings": mesh, "port_request": a.port, "ports_used": [], "blocks_failed": [], "status": "started"}
    log(f"== {name}: {len(blocks)} history blocks")
    prj = de.new_mws()
    try:
        for b in blocks:
            try:
                prj.model3d.add_to_history(b["caption"], b["code"])
                if b["kind"] == "port":
                    meta["ports_used"].append(b["caption"])
            except Exception as e:
                if b["kind"] == "port" and "(face)" in b["caption"]:
                    k = int(b["caption"].split()[1]) - 1
                    log(f"   face port {k + 1} failed ({e}); edge port instead")
                    prj.model3d.add_to_history(f"port {k + 1} (edge)", cst_case.edge_fallback(case, k))
                    meta["ports_used"].append(f"port {k + 1} (edge, face failed)")
                    meta["blocks_failed"].append({"caption": b["caption"], "error": str(e)})
                elif b["kind"] == "mesh":
                    log(f"   mesh block failed ({e}); minimal mesh block instead")
                    prj.model3d.add_to_history("mesh (minimal)", cst_case.mesh_minimal(mesh))
                    meta["blocks_failed"].append({"caption": b["caption"], "error": str(e)})
                    meta["mesh_block"] = "minimal"
                else:
                    meta["blocks_failed"].append({"caption": b["caption"], "error": str(e), "code": b["code"]})
                    raise
        cp = os.path.join(wd, name + ".cst")
        try:
            prj.save(cp, allow_overwrite=True)
        except TypeError:
            prj.save(cp)
        log("   saved", cp)
        if a.no_solve:
            meta["status"] = "built"
            return meta
        t0 = time.time()
        prj.model3d.run_solver()
        secs = time.time() - t0
        prj.save()
        logs = solver_logs(os.path.join(wd, name))
        texts = list(logs.values())
        cells = find_int(texts, [r"Number of mesh cells[^\d]*([\d,\. ]+\d)", r"mesh cells[^\d]*:\s*([\d,\. ]+\d)", r"Total number of mesh cells[^\d]*([\d,\. ]+\d)"])
        ver = None
        for t in texts:
            mm = re.search(r"Version:\s*(20\d\d\.\d+[^\n\r(]*)", t)
            if mm:
                ver = "CST Studio Suite " + mm.group(1).strip()
                break
        meta.update(seconds=round(secs, 1), cells=cells, cst_version=ver or a.cst_version)
        with open(os.path.join(a.logs, f"{name}_{stem}_logs.txt"), "w", encoding="utf-8") as fh:
            for k, t in logs.items():
                fh.write(f"==== {k}\n{t}\n")
        data, items = read_sparams(cst, cp, n)
        nf, fmax = case.get("nf", 400), case["fmax"]
        freq = [fmax * (i + 1) / nf for i in range(nf)]
        S = [[[0j] * n for _ in range(n)] for _ in freq]
        for (i, j), (xs, ys) in data.items():
            xs = [x * 1e9 for x in xs]  # project unit GHz
            for k, f in enumerate(freq):
                S[k][i][j] = interp(xs, ys, f)
        hdr = [f"{name}: {case['meta']['title']}", f"stackup: {case['meta']['stackup_title']}",
               "ports: " + ", ".join(p["name"] for p in case["meta"]["ports"]),
               f"{meta['cst_version']}, {meta['solver']}, {cells} mesh cells, {secs:.0f} s",
               f"mesh: {mesh}; ports: {'; '.join(meta['ports_used'])}",
               f"interpolated from CST's {len(next(iter(data.values()))[0])} samples to the case grid"]
        write_touchstone(os.path.join(od, f"{stem}.s{n}p"), freq, S, hdr, z0)
        meta["status"] = "ok"
        log(f"   done: {secs:.0f} s, {cells} cells")
        return meta
    except Exception as e:
        meta["status"] = "failed"
        meta["error"] = f"{e}"
        m = re.search(r"up to (\d+) mesh cells", str(e))
        if m:  # the mesh needs more cells than are available: not run, nothing coarsened
            meta["status"] = "not run"
            meta["error"] = f"exceeds the available mesh size (> {int(m.group(1)) / 1e6:g} M cells)"
        log("   FAILED:", repr(e))
        traceback.print_exc()
        try:
            for k, t in solver_logs(wd).items():
                with open(os.path.join(a.logs, f"{name}_{stem}_logs.txt"), "a", encoding="utf-8") as fh:
                    fh.write(f"==== {k}\n{t}\n")
        except Exception:
            pass
        return meta
    finally:
        json.dump(meta, open(os.path.join(od, f"{stem}_meta.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
        try:
            prj.close()
        except Exception:
            pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cases", nargs="*")
    ap.add_argument("--out", default="results")
    ap.add_argument("--work", default="projects")
    ap.add_argument("--logs", default="logs")
    ap.add_argument("--port", choices=["face", "edge"], default="face")
    ap.add_argument("--lpw", type=float, default=20)
    ap.add_argument("--lpb", type=float, default=20)
    ap.add_argument("--ratio", type=float, default=20)
    ap.add_argument("--minstep", type=float, default=0.0)
    ap.add_argument("--tag", default="")
    ap.add_argument("--skip-done", action="store_true", help="skip cases whose meta says ok (resume)")
    ap.add_argument("--close-de", action="store_true", help="close the design environment at the end")
    ap.add_argument("--probe", action="store_true")
    ap.add_argument("--no-solve", action="store_true")
    a = ap.parse_args()
    for d in (a.out, a.work, a.logs):
        os.makedirs(d, exist_ok=True)
    log("python", sys.version)
    cst = cst_import()
    if a.probe:
        probe(cst, a.logs)
    if not a.cases:
        return
    de = start_de(cst)
    a.cst_version = None
    try:
        de.set_quiet_mode(True)  # no modal dialogs in unattended runs
    except Exception as e:
        log("quiet mode n/a", repr(e))
    for m in ("version", "list_open_projects"):
        try:
            v = getattr(de, m)()
            log(m, v)
            if m == "version":
                mm = re.search(r"(20\d\d\.\d+[^\n(]*)", str(v))
                a.cst_version = f"CST Studio Suite {mm.group(1).strip()}" if mm else None
        except Exception as e:
            log(m, "n/a", repr(e))
    summary = []
    for p in a.cases:
        if a.skip_done:
            name = os.path.basename(p)[:-5]
            mp = os.path.join(a.out, name, (f"cst_{a.tag}" if a.tag else "cst") + "_meta.json")
            try:
                if json.load(open(mp, encoding="utf-8")).get("status") == "ok":
                    log(f"== {name}: done before, skipped")
                    continue
            except (OSError, ValueError):
                pass
        summary.append(run_case(cst, de, p, a))
    json.dump(summary, open(os.path.join(a.logs, "summary.json"), "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    if a.close_de:
        try:
            de.close()
        except Exception:
            pass
    log("all done:", [(m["case"], m["status"], m.get("seconds"), m.get("cells")) for m in summary])


if __name__ == "__main__":
    main()
