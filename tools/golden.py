#!/usr/bin/env python3
"""Stored results: kisi's output on the example boards and on the 3D via references, kept in the repository.

  golden.py boards <kisi> <examples dir> [--update] [--only ts.check,vna.power]
      runs the native CLI on ThunderScope and LibreVNA (web/samples/{ts,vna}.kisi.json) and compares every
      command's JSON with validation/golden/<board>.<command>.json
  golden.py vias <kisi> [--update]
      runs validation/fdtd/compare_kisi.py (kisi's channel models on the 3D via reference cases) and compares
      its table with validation/fdtd/results/kisi_models.json
  golden.py diff <stored.json> <new.json>
      compares two JSON files with the same rules

Rules: strings, booleans and the structure (keys, list lengths) must be identical – verdicts, warnings and names;
numbers are rounded to 6 significant digits when stored and may differ by 1e-4 relative (other CPUs and maths
libraries). Keys that are not results (config echo, field solver status, timings) are left out.

A change in a model changes the stored files: run the same command with --update and commit validation/golden
(or results/kisi_models.json) together with the change – the review then shows which values moved and by how
much (model changes need a comparison against the references, AGENTS.md). Exit code 1 on any difference.
"""
import json, os, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GOLDEN = os.path.join(ROOT, "validation", "golden")
VIAS = os.path.join(ROOT, "validation", "fdtd", "results", "kisi_models.json")
# board key, file stem, rail for the PDN analysis
BOARDS = [("ts", "Thunderscope_Rev5.3", "+1V0"), ("vna", "VNA", "+3V3")]
COMMANDS = {
    "overview": [], "interfaces": [], "check": ["--all"], "crosstalk": [], "gaps": [],
    "power": ["--pdn", "{rail}"], "thermal": [],
}
IGNORE = {"field_solver", "field_solver_note", "timing_ms", "file"}
REL, ABS = 1e-4, 1e-12
MAX_LINES = 25


def norm(x):
    if isinstance(x, dict):
        return {k: norm(v) for k, v in x.items() if k not in IGNORE}
    if isinstance(x, list):
        return [norm(v) for v in x]
    if isinstance(x, float):
        return float(f"{x:.6g}")
    return x


def diffs(a, b, path="$", out=None):
    """All differences of stored a and new b (a list of 'path: old -> new')."""
    out = [] if out is None else out
    num = (int, float)
    if isinstance(a, num) and isinstance(b, num) and not isinstance(a, bool) and not isinstance(b, bool):
        if abs(a - b) > max(ABS, REL * max(abs(a), abs(b))):
            rel = f" ({(b - a) / abs(a) * 100:+.3g} %)" if a else ""
            out.append(f"{path}: {a!r} -> {b!r}{rel}")
    elif type(a) != type(b):
        out.append(f"{path}: {json.dumps(a)[:80]} -> {json.dumps(b)[:80]}")
    elif isinstance(a, dict):
        for k in a:
            if k not in b:
                out.append(f"{path}.{k}: removed")
            else:
                diffs(a[k], b[k], f"{path}.{k}", out)
        for k in b:
            if k not in a:
                out.append(f"{path}.{k}: new ({json.dumps(b[k])[:60]})")
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append(f"{path}: {len(a)} -> {len(b)} entries")
        for i, (x, y) in enumerate(zip(a, b)):
            diffs(x, y, f"{path}[{i}]", out)
    elif a != b:
        out.append(f"{path}: {json.dumps(a)[:80]} -> {json.dumps(b)[:80]}")
    return out


def dump(obj, path):
    with open(path, "w") as f:
        json.dump(obj, f, indent=1, ensure_ascii=False)
        f.write("\n")


def summary(lines):
    s = os.environ.get("GITHUB_STEP_SUMMARY")
    if s:
        with open(s, "a") as f:
            f.write("\n".join(lines) + "\n")


def report(name, stored, new, update, path):
    """Compare (or store) one result; returns the number of differences."""
    if update:
        dump(new, path)
        print(f"{name}: stored")
        return 0
    if stored is None:
        print(f"{name}: no stored result ({os.path.relpath(path, ROOT)}) – run with --update")
        return 1
    d = diffs(stored, new)
    print(f"{name}: {'unchanged' if not d else f'{len(d)} difference(s)'}")
    for line in d[:MAX_LINES]:
        print("   ", line)
    if len(d) > MAX_LINES:
        print(f"    … {len(d) - MAX_LINES} more")
    return len(d)


def load(path):
    if not os.path.exists(path):
        return None
    with open(path) as f:
        return json.load(f)


def boards(kisi, ex, update, only):
    os.makedirs(GOLDEN, exist_ok=True)
    bad, rows = 0, []
    for key, stem, rail in BOARDS:
        pcb = os.path.relpath(os.path.join(ex, stem + ".kicad_pcb"), ROOT)
        cfg = os.path.join("web", "samples", key + ".kisi.json")
        for cmd, args in COMMANDS.items():
            name = f"{key}.{cmd}"
            if only and name not in only:
                continue
            argv = [kisi, cmd, pcb, "--config", cfg] + [a.format(rail=rail) for a in args]
            t0 = time.time()
            r = subprocess.run(argv, capture_output=True, text=True, cwd=ROOT)
            secs = time.time() - t0
            if r.returncode != 0:
                print(f"{name}: failed ({r.returncode})\n{r.stderr[-1500:]}")
                bad += 1
                rows.append(f"| {name} | failed | {secs:.1f} s |")
                continue
            path = os.path.join(GOLDEN, name + ".json")
            n = report(name, load(path), norm(json.loads(r.stdout)), update, path)
            bad += n
            rows.append(f"| {name} | {'unchanged' if not n else f'{n} differences'} | {secs:.1f} s |")
    summary(["### Stored results of the example boards", "", "| result | state | time |", "|---|---|---|"] + rows)
    return bad


def vias(kisi, update):
    tmp = os.path.join(tempfile.mkdtemp(), "kisi_models.json")
    t0 = time.time()
    r = subprocess.run([sys.executable, os.path.join(ROOT, "validation", "fdtd", "compare_kisi.py"), kisi, "--out", tmp],
                       capture_output=True, text=True, cwd=ROOT)
    secs = time.time() - t0
    if r.returncode != 0:
        print(f"compare_kisi.py failed ({r.returncode})\n{r.stdout[-1500:]}\n{r.stderr[-1500:]}")
        return 1
    new, stored = norm(load(tmp)), load(VIAS)
    n = report("via references", norm(stored) if stored else None, new, update, VIAS)
    if update:
        dump(load(tmp), VIAS)
    # the headline numbers (validation/via_references.md section 6), old and new
    lines = ["### kisi's models on the 3D via references", "", f"{len(new['rows'])} cases, {secs:.1f} s", "",
             "| metric | stored | now |", "|---|---|---|"]
    for k, v in new["summary"].items():
        if not isinstance(v, dict):
            lines.append(f"| {k} | {(stored or {}).get('summary', {}).get(k)} | {v} |")
    summary(lines)
    print("\n".join(lines[2:3]))
    return n


def main():
    a = sys.argv[1:]
    update = "--update" in a
    only = set(a[a.index("--only") + 1].split(",")) if "--only" in a else set()
    pos = [x for i, x in enumerate(a) if not x.startswith("--") and (i == 0 or a[i - 1] != "--only")]
    if pos[:1] == ["boards"] and len(pos) == 3:
        bad = boards(os.path.abspath(pos[1]), pos[2], update, only)
    elif pos[:1] == ["vias"] and len(pos) == 2:
        bad = vias(os.path.abspath(pos[1]), update)
    elif pos[:1] == ["diff"] and len(pos) == 3:
        bad = report("diff", norm(load(pos[1])), norm(load(pos[2])), False, pos[1])
    else:
        raise SystemExit(__doc__)
    if bad and not update:
        print("\nIf the change is intended: run the same command with --update and commit the stored files with it.")
    sys.exit(1 if bad and not update else 0)


if __name__ == "__main__":
    main()
