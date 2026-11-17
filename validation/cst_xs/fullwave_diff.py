#!/usr/bin/env python3
"""Two sets of full-wave results against each other (validation/cst_xs/fullwave/<id>.json, run_fullwave.py): what an
upstream update, a new WASM build or other settings moved.

    python3 validation/cst_xs/fullwave_diff.py <old dir> <new dir> [--md out.md] [--max-pct 0.05]

Per kind (E, M, sweep) and quantity: cases compared, how many changed at all, max |relative change| in % and its case.
The point results (E, M) compare every number of the result except timings; the sweep compares every frequency of
every mode quantity and of Zdiff/Zcomm. Only cases with a valid result for the same geometry (text hash) on both sides
count. Exit code 1 when an impedance or εeff (z*, eeff*) moved by more than --max-pct (default: report only).
"""
import argparse, glob, json, math, os, sys

SKIP = {"ms", "wall_s", "task_s", "peak_rss_mb", "f0", "ms_point", "f"}


def load(p):
    try:
        return json.load(open(p, encoding="utf-8"))
    except (OSError, ValueError):
        return None


def values(kind, r):
    """{quantity: [numbers]} of one result"""
    out = {}
    if kind == "sweep":
        for mode, q in (r.get("modes") or {}).items():
            for k, v in q.items():
                if isinstance(v, list):
                    out[f"{k}_{mode}" if mode != "single" else k] = v
        for k in ("zdiff", "zcomm"):
            if isinstance(r.get(k), list):
                out[k] = r[k]
        return out
    for k, v in r.items():
        if k in SKIP or isinstance(v, bool):
            continue
        if isinstance(v, (int, float)):
            out[k] = [v]
        elif isinstance(v, list) and all(isinstance(x, (int, float)) for x in v):
            out[k] = v
    return out


def rel(a, b):
    if a == b:
        return 0.0
    if not (isinstance(a, (int, float)) and isinstance(b, (int, float))) or not (math.isfinite(a) and math.isfinite(b)):
        return math.inf
    return abs(b - a) / max(abs(a), abs(b), 1e-300)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--md")
    ap.add_argument("--max-pct", type=float)
    a = ap.parse_args()
    stats = {}  # (kind, quantity) -> [n, changed, max, case]
    revs, n_cases, missing = set(), 0, []
    for pn in sorted(glob.glob(os.path.join(a.new, "*.json"))):
        new, old = load(pn), load(os.path.join(a.old, os.path.basename(pn)))
        if not new or "results" not in new:
            continue
        if not old or old.get("text_hash") != new.get("text_hash"):
            missing.append(new.get("id", os.path.basename(pn)))
            continue
        n_cases += 1
        revs.add((old.get("upstream_rev", "?")[:7], new.get("upstream_rev", "?")[:7]))
        for kind, rn in new["results"].items():
            ro = old["results"].get(kind)
            if not (ro and ro.get("ok") and rn.get("ok")):
                continue
            vo, vn = values(kind, ro), values(kind, rn)
            for q in sorted(set(vo) & set(vn)):
                s = stats.setdefault((kind, q), [0, 0, 0.0, ""])
                s[0] += 1
                d = max((rel(x, y) for x, y in zip(vo[q], vn[q])), default=0.0)
                if len(vo[q]) != len(vn[q]):
                    d = math.inf
                if d > 0:
                    s[1] += 1
                if d > s[2]:
                    s[2], s[3] = d, new["id"]
    lines = [f"Full-wave results {a.new} against {a.old}: {n_cases} cases"
             + (f" (upstream {', '.join(f'{o} -> {n}' for o, n in sorted(revs))})" if revs else "")
             + (f"; {len(missing)} without a result of the same geometry on the old side" if missing else ""), "",
             "| kind | quantity | n | changed | max change % | case |", "|---|---|---:|---:|---:|---|"]
    worst = 0.0
    for (kind, q), (n, ch, mx, case) in sorted(stats.items()):
        lines.append(f"| {kind} | {q} | {n} | {ch} | {mx * 100:.3g} | {case if ch else ''} |")
        if q.startswith(("z", "eeff")):
            worst = max(worst, mx)
    lines += ["", f"largest change of an impedance or εeff: {worst * 100:.3g} %"]
    text = "\n".join(lines)
    print(text)
    if a.md:
        open(a.md, "w", encoding="utf-8").write(text + "\n")
    if a.max_pct is not None and worst * 100 > a.max_pct:
        print(f"moved by more than {a.max_pct} %", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
