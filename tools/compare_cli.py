#!/usr/bin/env python3
"""Compare the Node CLI (dist/kisi.mjs, formula mode) with the native kisi on the example boards.

Usage: compare_cli.py <kisi native binary> <kisi.mjs> <examples dir> [--commands check,gaps]

Runs every command on ThunderScope and LibreVNA with their configs (web/samples/{ts,vna}.kisi.json) and compares
the JSON outputs. Numbers are compared after rounding to 6 significant digits; keys that differ by design
(field solver status, config echo, timings) are ignored. Exit code 1 on any difference.
"""
import json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOARDS = [("ts", "Thunderscope_Rev5.3"), ("vna", "VNA")]
IGNORE = {"field_solver", "field_solver_note", "config", "timing_ms"}
ARGS = {"check": ["--all"], "gaps": []}


def norm(x):
    if isinstance(x, dict):
        return {k: norm(v) for k, v in x.items() if k not in IGNORE}
    if isinstance(x, list):
        return [norm(v) for v in x]
    if isinstance(x, bool) or x is None or isinstance(x, str):
        return x
    if isinstance(x, (int, float)):
        return float(f"{float(x):.6g}")
    return x


def first_diff(a, b, path="$"):
    if type(a) != type(b):
        return f"{path}: {json.dumps(a)[:120]} != {json.dumps(b)[:120]}"
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}.{k}: only in {'native' if k in a else 'node'}"
            d = first_diff(a[k], b[k], f"{path}.{k}")
            if d:
                return d
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: {len(a)} != {len(b)} entries"
        for i, (x, y) in enumerate(zip(a, b)):
            d = first_diff(x, y, f"{path}[{i}]")
            if d:
                return d
        return None
    return None if a == b else f"{path}: {a!r} != {b!r}"


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
    if r.returncode != 0:
        raise SystemExit(f"failed ({r.returncode}): {' '.join(cmd)}\n{r.stderr[-2000:]}")
    return json.loads(r.stdout)


def main():
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    native, mjs, ex = sys.argv[1:4]
    cmds = ["check", "gaps"]
    if "--commands" in sys.argv:
        cmds = sys.argv[sys.argv.index("--commands") + 1].split(",")
    bad = 0
    for key, stem in BOARDS:
        pcb = os.path.join(ex, stem + ".kicad_pcb")
        cfg = os.path.join(ROOT, "web", "samples", key + ".kisi.json")
        for c in cmds:
            a = run([native, c, pcb, "--config", cfg] + ARGS.get(c, []))
            b = run(["node", mjs, c, pcb, "--config", cfg, "--formula"] + ARGS.get(c, []))
            d = first_diff(norm(a), norm(b))
            print(f"{stem} {c}: {'identical' if d is None else 'DIFFERENT – ' + d}")
            bad += d is not None
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
