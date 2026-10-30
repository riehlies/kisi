#!/usr/bin/env python3
"""kisi's results on the cases of cases.json → validation/cst_xs/kisi.json.

    python3 validation/cst_xs/run_kisi.py [--kisi-validate target/release/kisi-validate]
        [--ids a,b | --tier 1] [--jobs 4] [--out validation/cst_xs/kisi.json]

Per case:
  formula      the extraction's closed-form values (kisi-validate xs-cases: extract::formula_xs) –
               what kisi reports without the field solver (Z0 / Zdiff, Zeven, εeff)
  field        js_2d_fields as kisi runs it (tools/solve_xs.mjs, tol 0.01 as the viewer and kisi.mjs,
               open boundaries on its automatic domain) – what kisi reports with the field solver
  field_fine   the same at tol 0.002 (the solver's own discretisation error: field vs field_fine)
  field_E      tol 0.002 in CST's box with electric walls (xsgeo.boxed_text "gnd", ground ties)
  field_M      tol 0.002 in CST's box with natural walls = CST's magnetic walls ("open", ground straps)
field_E / field_M solve exactly the problem CST solves with electric / magnetic walls: CST against
them is the solver comparison without any boundary difference; their mean estimates the open
boundary (README.md). Cases with a hand-written text (no kisi model) get the field values only.
The node solver runs need the release builds (target/release/kisi-validate,
target/wasm32-unknown-unknown/release/kisi_sparse.wasm).
"""
import argparse, json, os, subprocess, sys, tempfile, time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import xsgeo  # noqa: E402

KEEP = ("ok", "z0", "eeff", "zodd", "zeven", "eeff_odd", "eeff_even", "ms", "error")


def solve(items, tol, tmp, tag):
    """[(key, text)] → {key: result} through tools/solve_xs.mjs"""
    pin = os.path.join(tmp, f"{tag}.in.json")
    pout = os.path.join(tmp, f"{tag}.out.json")
    json.dump([{"key": k, "text": t} for k, t in items], open(pin, "w"))
    r = subprocess.run(["node", os.path.join(ROOT, "tools/solve_xs.mjs"), pin, pout, "--tol", str(tol)],
                       capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-2000:])
    return {x["key"]: {k: x[k] for k in KEEP if k in x} for x in json.load(open(pout))}


def git_rev():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, cwd=ROOT).stdout.strip()
    except OSError:
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--kisi-validate", default=os.path.join(ROOT, "target/release/kisi-validate"))
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--out", default=os.path.join(HERE, "kisi.json"))
    ap.add_argument("--ids", default="")
    ap.add_argument("--tier", default="")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--chunk", type=int, default=20)
    a = ap.parse_args()
    cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
    if a.ids:
        want = set(a.ids.split(","))
        cases = [c for c in cases if c["id"] in want]
    if a.tier:
        cases = [c for c in cases if str(c["tier"]) in a.tier.split(",")]
    tmp = tempfile.mkdtemp(prefix="cstxs_kisi_")
    # formulas and a check that the geometry texts are current
    pin = os.path.join(tmp, "specs.json")
    pout = os.path.join(tmp, "formula.json")
    json.dump([{"id": c["id"], "spec": c["spec"]} for c in cases if c.get("spec")], open(pin, "w"))
    r = subprocess.run([a.kisi_validate, "xs-cases", pin, "--out", pout], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(r.stderr)
    fx = {x["key"]: x for x in json.load(open(pout))}
    stale = [c["id"] for c in cases if c.get("text_source") == "kisi" and fx.get(c["id"], {}).get("text") != c["text"]]
    if stale:
        print(f"warning: kisi's geometry text differs from cases.json for {len(stale)} cases (cases.json stale? "
              f"run make_cases.py): {', '.join(stale[:5])}", file=sys.stderr)
    jobs = {"field": [], "field_fine": [], "field_E": [], "field_M": []}
    for c in cases:
        text = c["text"]
        box = c["domain"]["box"]
        jobs["field"].append((c["id"], text))
        jobs["field_fine"].append((c["id"], text))
        jobs["field_E"].append((c["id"], xsgeo.boxed_text(text, box, "gnd")))
        if not xsgeo.isolated_grounds(xsgeo.parse(text), box):
            jobs["field_M"].append((c["id"], xsgeo.boxed_text(text, box, "open")))
    tols = {"field": 0.01, "field_fine": 0.002, "field_E": 0.002, "field_M": 0.002}
    work = []
    for kind, items in jobs.items():
        for i in range(0, len(items), a.chunk):
            work.append((kind, i // a.chunk, items[i:i + a.chunk]))
    t0 = time.time()
    res = {k: {} for k in jobs}
    with ThreadPoolExecutor(max_workers=a.jobs) as ex:
        futs = {ex.submit(solve, items, tols[kind], tmp, f"{kind}_{n}"): kind for kind, n, items in work}
        done = 0
        for f in futs:
            res[futs[f]].update(f.result())
            done += 1
            if done % 10 == 0 or done == len(futs):
                print(f"  {done}/{len(futs)} solver batches, {time.time() - t0:.0f} s", file=sys.stderr)
    out = {}
    for c in cases:
        e = {}
        f = fx.get(c["id"])
        if f and (c.get("text_source") == "kisi" or c["group"] == "cpw_noplane"):
            e["formula"] = {k: v for k, v in f["formula"].items() if v is not None}
        for kind in jobs:
            if c["id"] in res[kind]:
                e[kind] = res[kind][c["id"]]
        out[c["id"]] = e
    prev = {}
    if (a.ids or a.tier) and os.path.isfile(a.out):
        prev = json.load(open(a.out)).get("results", {})
    prev.update(out)
    doc = {"about": "kisi's results on validation/cst_xs/cases.json (run_kisi.py; README.md)", "kisi_rev": git_rev(),
           "date": time.strftime("%Y-%m-%d"), "tol": tols, "upstream_rev": open(os.path.join(ROOT, "third_party/UPSTREAM_REV")).read().strip(),
           "results": prev}
    with open(a.out, "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=1, ensure_ascii=False)
        fh.write("\n")
    fails = [(k, kind) for k, e in out.items() for kind in jobs if kind in e and not e[kind].get("ok")]
    print(f"{len(out)} cases → {os.path.relpath(a.out, ROOT)} in {time.time() - t0:.0f} s; solver failures: {len(fails)} {fails[:6]}")


if __name__ == "__main__":
    main()
