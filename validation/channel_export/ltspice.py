#!/usr/bin/env python3
"""LTspice files of a lane export: a consistency check without LTspice, and ngspice's results of the same bench.

Usage: ltspice.py check <lane folder>
       ltspice.py expected <lane folder> [--out expected.json]

check (also from compare.py, so in CI): the LTspice bench and library are ASCII, have no ngspice or Xyce constructs
(`.control`, `wrdata`, `portnum`, `.lin`, `.print`), every subcircuit instance names a subcircuit that exists with
as many ports as nodes given, every inductor has `Rser=0` (LTspice adds 1 mOhm otherwise) and no other element has
an LTspice parameter, the `.tran`, `.meas`, `B` and `.net` lines are well formed, and `<lane>_ltspice.lib` is
`<lane>.lib` except for `Rser=0` and ASCII comments; and no node of the flattened lane subcircuit is without a DC
path to ground (LTspice reports such nodes as floating and its results drift).

expected: runs bench_ltspice.cir in ngspice (with `<lane>.lib` instead of the LTspice library – the same circuit;
the `.meas` lines are read by both simulators) and writes the measurements, ngspice's run time and the element
counts; for a single-ended lane also S11 and S21 at a few frequencies from ngspice's S-parameter deck – the numbers
to compare with LTspice's `.log` and `.net` traces.
"""
import json, os, re, subprocess, sys, time

BANNED = [r"^\s*\.control\b", r"^\s*\.endc\b", r"\bwrdata\b", r"\bportnum\b", r"^\s*\.lin\b", r"^\s*\.print\b", r"^\s*\.sp\b"]
MEAS = re.compile(r"^\.meas TRAN \w+ (WHEN V\(\w+\)=[-+0-9.eE]+ (RISE|FALL|CROSS)=\d+|FIND V\(\w+\) AT=[-+0-9.eE]+|(MAX|MIN|AVG|PP|RMS) V\(\w+\) FROM=[-+0-9.eE]+ TO=[-+0-9.eE]+)$")
NUM = r"[-+]?[0-9.]+(e[-+]?\d+)?"


def joined(path):
    out = []
    for raw in open(path, encoding="ascii", errors="strict").read().splitlines():
        if raw.startswith("+") and out:
            out[-1] = (out[-1][0], out[-1][1] + " " + raw[1:])
        else:
            out.append((path, raw))
    return out


def stem_of(d):
    libs = [f for f in os.listdir(d) if f.endswith("_ltspice.lib")]
    return libs[0][: -len("_ltspice.lib")] if libs else None


def check(d):
    errs = []
    st = stem_of(d)
    if st is None:
        return {"ok": False, "errors": ["no <lane>_ltspice.lib"]}
    files = ["bench_ltspice.cir", f"{st}_ltspice.lib"] + (["sparam_ltspice.cir"] if os.path.exists(os.path.join(d, "sparam_ltspice.cir")) else [])
    lines = []
    for f in files:
        p = os.path.join(d, f)
        if not os.path.exists(p):
            errs.append(f"{f} missing")
            continue
        try:
            lines += joined(p)
        except UnicodeDecodeError as e:
            errs.append(f"{f}: not ASCII ({e})")
    subs, insts, cur = {}, [], None
    nodes = {}
    for f, l in lines:
        t = l.split()
        if t and t[0][0].lower() in "rlcvefbx" and not l.startswith("*"):
            n = {"r": 2, "l": 2, "c": 2, "v": 2, "b": 2, "e": 4, "f": 2, "x": len(t) - 2}[t[0][0].lower()]
            for nd in t[1:1 + n]:
                nodes.setdefault((f, cur), {}).setdefault(nd.lower(), set()).add(nd)
            # values in e-notation only (LTspice: 1M is milli, letters after a number are dropped)
            vals = t[3:4] if t[0][0].lower() in "rlc" else []
            if t[0][0].lower() == "v" and len(t) == 4:
                vals = t[3:]
            for v in vals:
                if not re.fullmatch(NUM, v, re.I):
                    errs.append(f"{f}: value not in e-notation: {l[:60]}")
            for v in re.findall(r"PWL\((.*)\)", l):
                if not all(re.fullmatch(NUM, x, re.I) for x in v.split()):
                    errs.append(f"{f}: PWL value not in e-notation")
        if t and t[0].lower() == ".subckt":
            cur = t[1].lower()
        elif t and t[0].lower() == ".ends":
            cur = None
    for (f, sub), m in nodes.items():
        for k, v in m.items():
            if len(v) > 1:
                errs.append(f"{f}: nodes {sorted(v)} differ only in case ({sub})")
    if not any(l.startswith(".options") and "reject_number_tails" in l for f, l in lines if f.endswith("bench_ltspice.cir")):
        errs.append("bench: .options reject_number_tails missing")
    cur = None
    for f, l in lines:
        t = l.split()
        if not t or l.startswith("*"):
            continue
        for b in BANNED:
            if re.search(b, l, re.I):
                errs.append(f"{f}: {l[:60]} (not LTspice)")
        k = t[0].lower()
        if k == ".subckt":
            cur = t[1].lower()
            subs[cur] = len([x for x in t[2:] if "=" not in x and x.lower() != "params:"])
        elif k == ".ends":
            cur = None
        elif k[0] == "x":
            insts.append((f, t[0], len(t) - 2, t[-1].lower()))
        elif k[0] == "l":
            if not (len(t) == 5 and t[4] == "Rser=0" and re.fullmatch(NUM, t[3], re.I)):
                errs.append(f"{f}: inductor without Rser=0: {l[:60]}")
        elif k[0] in "rc" and len(t) != 4:
            errs.append(f"{f}: unexpected parameters: {l[:60]}")
        elif k == ".tran" and not (len(t) == 5 and all(re.fullmatch(NUM, x, re.I) for x in t[1:])):
            errs.append(f"{f}: .tran needs Tprint Tstop Tstart Tmaxstep: {l}")
        elif k == ".meas" and not MEAS.match(l):
            errs.append(f"{f}: .meas not understood: {l}")
        elif k[0] == "b" and not re.fullmatch(r"B\w+ \w+ 0 V=V\(\w+\)(-V\(\w+\))?", l):
            errs.append(f"{f}: B source: {l}")
        elif k == ".net" and l.strip() != ".net I(Rout) Vin":
            errs.append(f"{f}: .net: {l}")
    for f, name, n, sub in insts:
        if sub not in subs:
            errs.append(f"{f}: {name} calls {sub}, not defined")
        elif subs[sub] != n:
            errs.append(f"{f}: {name} gives {n} nodes, {sub} has {subs[sub]} ports")
    bench = [l for f, l in lines if f.endswith("bench_ltspice.cir")]
    if not any(l.startswith(".meas") for l in bench):
        errs.append("bench: no .meas")
    if not any(l.startswith(f'.include "{st}_ltspice.lib"') for l in bench):
        errs.append("bench: does not include the LTspice library")
    sp = [l for f, l in lines if f.endswith("sparam_ltspice.cir")]
    if sp and not (any(re.match(r"^Vin a 0 AC 1 Rser=", l) for l in sp) and any(l.startswith("Rout b 0 ") for l in sp)):
        errs.append("sparam: Vin with Rser and Rout expected")
    # the LTspice library is the shared one, with Rser=0 and ASCII comments
    lib = os.path.join(d, f"{st}.lib")
    if os.path.exists(lib):
        a = [l for l in open(lib, encoding="utf-8").read().splitlines() if not l.startswith("*")]
        b = [l.replace(" Rser=0", "") for l in open(os.path.join(d, f"{st}_ltspice.lib"), encoding="ascii").read().splitlines() if not l.startswith("*")]
        if a != b:
            errs.append(f"{st}_ltspice.lib differs from {st}.lib by more than Rser=0")
    try:
        fl = floating_nodes(d)
    except (StopIteration, KeyError) as e:
        fl = []
        errs.append(f"floating-node check failed: {e}")
    if fl:
        errs.append(f"{len(fl)} node(s) without a DC path to ground, e.g. {fl[:3]}")
    return {"ok": not errs, "subckts": len(subs), "instances": len(insts), "floating_nodes": len(fl), "errors": errs[:10]}


def floating_nodes(d):
    """Nodes of the lane subcircuit without a DC path to ground (flattened): connected only through current
    sources, capacitors or controlled-source inputs. The lane's ports count as driven (the bench's ends)."""
    st = stem_of(d)
    subs = {}
    cur = None
    for _, l in joined(os.path.join(d, f"{st}_ltspice.lib")):
        t = l.split()
        if not t or l.startswith("*"):
            continue
        if t[0].lower() == ".subckt":
            cur = t[1].lower()
            subs[cur] = ([x.lower() for x in t[2:] if "=" not in x and x.lower() != "params:"], [])
        elif t[0].lower() == ".ends":
            cur = None
        elif cur:
            subs[cur][1].append([x.lower() for x in t])
    lane = next(k for k in subs if k.endswith("_lane"))
    parent = {}

    def find(a):
        while parent.setdefault(a, a) != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def join(a, b):
        parent[find(a)] = find(b)

    nodes = set()
    stack = [(lane, {p: (p if p != "ref" else "0") for p in subs[lane][0]}, "")]
    while stack:
        name, mp, pre = stack.pop()
        ports, body = subs[name]
        node = lambda x: "0" if x == "0" else mp.get(x, pre + x)
        for t in body:
            k = t[0][0]
            if k == "x":
                sub = t[-1]
                stack.append((sub, dict(zip(subs[sub][0], [node(x) for x in t[1:-1]])), pre + t[0] + "."))
                continue
            ns = [node(x) for x in t[1:3]]
            nodes.update(ns)
            if k in "rlvbe":
                join(ns[0], ns[1])
            if k in "ef":
                nodes.update(node(x) for x in t[3:5] if k == "e")
    for p in subs[lane][0]:
        join(p if p != "ref" else "0", "0")
    g = find("0")
    return sorted(n for n in nodes if find(n) != g)


def expected(d):
    st = stem_of(d)
    src = open(os.path.join(d, "bench_ltspice.cir"), encoding="ascii").read()
    src = src.replace(f'.include "{st}_ltspice.lib"', f'.include "{st}.lib"')
    # LTspice-only options out (ngspice does not know them)
    src = "\n".join(l for l in src.splitlines() if not l.startswith(".options")) + "\n"
    src = re.sub(r"( Rser=0)", "", src)
    open(os.path.join(d, "bench_ltspice_in_ngspice.cir"), "w").write(src)
    t0 = time.time()
    r = subprocess.run(["ngspice", "-b", "bench_ltspice_in_ngspice.cir"], cwd=d, capture_output=True, text=True, timeout=7200)
    secs = time.time() - t0
    meas = {}
    for m in re.finditer(r"^(\w+)\s*=\s*([-+0-9.eE]+)", r.stdout, re.M):
        meas[m.group(1).lower()] = float(m.group(2))
    names = re.findall(r"^\.meas TRAN (\w+)", src, re.M)
    out = {"lane_folder": os.path.basename(os.path.abspath(d)), "simulator": "ngspice " + (subprocess.run(["ngspice", "-v"], capture_output=True, text=True).stdout.split("ngspice-")[1].split()[0] if True else ""),
           "bench": "bench_ltspice.cir (ngspice read it with the shared .lib instead of the LTspice library)",
           "meas": {n: meas.get(n.lower()) for n in names}, "units": "t50 in s, voltages in V (the received voltage; a pair: differential)",
           "ngspice_transient_s": round(secs, 1), "ngspice_ok": r.returncode == 0}
    hdr = open(os.path.join(d, f"{st}.lib"), encoding="utf-8").read()
    m = re.search(r"elements after flattening: (\d+) R, (\d+) L, (\d+) C, (\d+) sources; (\d+) RLGC sections", hdr)
    if m:
        out["elements"] = {"R": int(m.group(1)), "L": int(m.group(2)), "C": int(m.group(3)), "sources": int(m.group(4)), "rlgc_sections": int(m.group(5))}
    m = re.search(r"\(([\d.]+) sections per wavelength at ([\d.]+) GHz\)", hdr)
    if m:
        out["ladder"] = {"sections_per_wavelength": float(m.group(1)), "f_max_ghz": float(m.group(2))}
    tr = re.search(r"^\.tran (\S+) (\S+)", src, re.M)
    out["tran"] = {"t_step_s": float(tr.group(1)), "t_stop_s": float(tr.group(2))}
    if os.path.exists(os.path.join(d, "sparam_ltspice.cir")):
        import numpy as np
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from compare import read_wrdata
        if not os.path.exists(os.path.join(d, "sparam_ngspice.txt")):
            subprocess.run(["ngspice", "-b", "sparam_ngspice.cir"], cwd=d, capture_output=True, text=True, timeout=7200)
        f, s = read_wrdata(os.path.join(d, "sparam_ngspice.txt"), 2)
        rows = []
        for fg in (0.1, 0.5, 1, 2, 5, 10, 15, 20, 25):
            k = int(np.argmin(np.abs(f - fg * 1e9)))
            row = {"f_ghz": round(float(f[k]) / 1e9, 4)}
            for nm, (i, j) in (("S11", (0, 0)), ("S21", (1, 0))):
                v = s[k, i, j]
                row[nm + "_db"] = round(float(20 * np.log10(abs(v))), 4)
                row[nm + "_deg"] = round(float(np.degrees(np.angle(v))), 3)
            rows.append(row)
        out["sparams"] = {"from": "sparam_ngspice.cir (ngspice .sp, reference as in sparam_ltspice.cir)", "points": rows,
                          "ltspice_traces": "S11(Vin), S21(Vin) of sparam_ltspice.cir"}
    return out


def main():
    a = sys.argv[1:]
    if len(a) < 2 or a[0] not in ("check", "expected"):
        raise SystemExit(__doc__)
    if a[0] == "check":
        res = check(a[1])
        print(json.dumps(res, indent=1))
        sys.exit(0 if res["ok"] else 1)
    res = expected(a[1])
    out = a[a.index("--out") + 1] if "--out" in a else os.path.join(a[1], "expected.json")
    json.dump(res, open(out, "w"), indent=1)
    print(json.dumps(res, indent=1))


if __name__ == "__main__":
    main()
