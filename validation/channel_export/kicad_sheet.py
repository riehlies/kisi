#!/usr/bin/env python3
"""Checks of a KiCad project written by `kisi export` (one lane folder).

Usage: kicad_sheet.py <lane folder> [--transient] [--svg DIR] [--png]

1. The sheet itself (no KiCad needed): every pin of a part visible (power symbols excepted), every
   wire end on a pin end, another wire end or a wire (then with a junction), no pin left open, a
   junction wherever three or more meet, everything on the 1.27 mm grid.
2. kicad-cli's SPICE netlist: no unconnected pin, no node with a single connection, and – for the
   detailed sheet – the same circuit as the lane subcircuit: every element of `<lane>_lane` once,
   connected as there (node for node, the reference on ground), the ports at the transmitter's and
   the receiver's die.
3. kicad-cli's ERC (KiCad 8 and later; KiCad 7's kicad-cli has none): its violations by type; any
   error, and any unconnected-pin, dangling-wire or off-grid report, fails.
4. The KiCad netlist in ngspice against the bench (bench_ngspice.cir), both with AC sources: the
   receiver's die voltage on 40 frequencies must agree to 1e-6 (exact, no time steps). With
   --transient also both transients, cut to an eighth of the pattern (for information: adaptive
   time steps differ with the element order, about 1e-4 of the swing).
5. With --svg DIR: the sheet plotted by kicad-cli (and as PNG with --png, needs cairosvg).

Prints a JSON summary; exit code 1 when a check fails.
"""
import glob, json, os, re, shutil, subprocess, sys

GRID = 1.27
BAD_ERC = ("pin_not_connected", "wire_dangling", "unconnected_wire_endpoint", "label_dangling", "endpoint_off_grid", "pin_not_driven", "power_pin_not_driven", "no_connect_dangling")


# --- s-expressions --------------------------------------------------------------------------------

def parse(text):
    tok = re.compile(r'\(|\)|"(?:[^"\\]|\\.)*"|[^\s()]+')
    stack, cur = [], []
    for m in tok.finditer(text):
        t = m.group(0)
        if t == "(":
            stack.append(cur)
            cur = []
        elif t == ")":
            done = cur
            cur = stack.pop()
            cur.append(done)
        elif t.startswith('"'):
            cur.append(bytes(t[1:-1], "utf-8").decode("unicode_escape").encode("latin-1").decode("utf-8"))
        else:
            cur.append(t)
    return cur[0]


def kids(node, name):
    return [c for c in node if isinstance(c, list) and c and c[0] == name]


def kid(node, name):
    k = kids(node, name)
    return k[0] if k else None


def on_grid(v):
    return abs(v / GRID - round(v / GRID)) < 1e-3


# --- 1. the sheet ---------------------------------------------------------------------------------

def sheet_check(path):
    root = parse(open(path, encoding="utf-8").read())
    lib = {}
    for s in kid(root, "lib_symbols")[1:]:
        if not (isinstance(s, list) and s[0] == "symbol"):
            continue
        power = any(c == ["power"] for c in s if isinstance(c, list))
        pins = []

        def walk(n):
            for c in n:
                if isinstance(c, list) and c and c[0] == "pin":
                    at = kid(c, "at")
                    pins.append((float(at[1]), float(at[2]), "hide" in c))
                elif isinstance(c, list) and c and c[0] == "symbol":
                    walk(c)
        walk(s)
        lib[s[1]] = (power, pins)
    issues = {}

    def add(kind, what):
        issues.setdefault(kind, []).append(what)

    pins = []
    nsym = 0
    for s in kids(root, "symbol"):
        lid = kid(s, "lib_id")[1]
        at = kid(s, "at")
        x, y, rot = float(at[1]), float(at[2]), float(at[3]) if len(at) > 3 else 0.0
        ref = next((p[2] for p in kids(s, "property") if p[1] == "Reference"), "?")
        nsym += 1
        if rot != 0:
            add("rotated_symbol", ref)
        power, lp = lib.get(lid, (False, []))
        for px, py, hide in lp:
            if hide and not power:
                add("hidden_pin", ref)
            pins.append(((round(x + px, 3), round(y - py, 3)), ref))
    wires = []
    for w in kids(root, "wire"):
        pts = kid(w, "pts")
        (a, b) = [(round(float(p[1]), 3), round(float(p[2]), 3)) for p in kids(pts, "xy")]
        wires.append((a, b))
    junctions = {(round(float(kid(j, "at")[1]), 3), round(float(kid(j, "at")[2]), 3)) for j in kids(root, "junction")}

    def through(p, a, b):
        if abs(a[0] - b[0]) < 1e-6 and abs(p[0] - a[0]) < 1e-6:
            return min(a[1], b[1]) + 1e-6 < p[1] < max(a[1], b[1]) - 1e-6
        if abs(a[1] - b[1]) < 1e-6 and abs(p[1] - a[1]) < 1e-6:
            return min(a[0], b[0]) + 1e-6 < p[0] < max(a[0], b[0]) - 1e-6
        return False

    def count(p):
        ends = sum((a == p) + (b == p) for a, b in wires)
        npin = sum(1 for q, _ in pins if q == p)
        thr = sum(1 for a, b in wires if through(p, a, b))
        return ends, npin, thr

    for p, ref in pins:
        if not (on_grid(p[0]) and on_grid(p[1])):
            add("off_grid", f"pin of {ref} at {p}")
        ends, npin, thr = count(p)
        if ends + thr == 0 and npin < 2:
            add("pin_not_connected", f"{ref} at {p}")
    points = {p for w in wires for p in w}
    for p in points:
        if not (on_grid(p[0]) and on_grid(p[1])):
            add("off_grid", f"wire end at {p}")
        ends, npin, thr = count(p)
        if ends == 1 and npin == 0 and thr == 0:
            add("wire_dangling", f"at {p}")
        n = ends + npin + 2 * thr
        if n >= 3 and p not in junctions:
            add("missing_junction", f"at {p}")
    for p in junctions:
        ends, npin, thr = count(p)
        if ends + npin + 2 * thr < 3:
            add("extra_junction", f"at {p}")
    hidden_ok = all(k not in issues for k in ("hidden_pin",))
    ok = not any(k in issues for k in ("hidden_pin", "off_grid", "pin_not_connected", "wire_dangling", "missing_junction", "rotated_symbol"))
    return {"ok": ok and hidden_ok, "symbols": nsym, "wires": len(wires), "junctions": len(junctions), "issues": {k: v[:8] for k, v in issues.items()}, "issue_counts": {k: len(v) for k, v in issues.items()}}


# --- 2. the netlist ---------------------------------------------------------------------------------

def run(cmd, cwd, timeout=3600):
    # kicad-cli writes model errors to the terminal only: run it under `script` when available
    if shutil.which("script") and cmd[0] == "kicad-cli":
        cmd = ["script", "-qec", " ".join(f"'{c}'" for c in cmd), "/dev/null"]
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)


def subckts(text):
    """{name: (ports, [lines])} of a SPICE library (continuation lines joined)."""
    out, cur = {}, None
    lines = []
    for raw in text.splitlines():
        if raw.startswith("+") and lines:
            lines[-1] += " " + raw[1:]
        else:
            lines.append(raw)
    for l in lines:
        t = l.split()
        if not t or l.startswith("*"):
            continue
        if t[0].lower() == ".subckt":
            ports = [p for p in t[2:] if "=" not in p and p.lower() != "params:"]
            cur = t[1]
            out[cur] = (ports, [])
        elif t[0].lower() == ".ends":
            cur = None
        elif cur:
            out[cur][1].append(t)
    return out


def elements(net):
    """Elements of a netlist: (name, nodes, model) – X lines with their subcircuit, R/L/C/V with value."""
    out = []
    for l in net.splitlines():
        t = l.split()
        if not t or t[0][0] in "*." or t[0][0] == "+":
            continue
        t = [x for x in t if "=" not in x]
        if t[0][0] in "Xx":
            out.append((t[0], t[1:-1], t[-1]))
        else:
            out.append((t[0], t[1:3], " ".join(t[3:])))
    return out


def netlist_check(lane_dir, stem):
    sch = f"{stem}.kicad_sch"
    r = run(["kicad-cli", "sch", "export", "netlist", "--format", "spice", sch, "-o", "kicad_netlist.cir"], lane_dir)
    msgs = [l.strip() for l in (r.stdout + r.stderr).splitlines() if "rror" in l or "arning" in l]
    path = os.path.join(lane_dir, "kicad_netlist.cir")
    if r.returncode != 0 or not os.path.exists(path):
        return {"ok": False, "error": (r.stdout + r.stderr)[-400:]}
    net = open(path).read()
    els = elements(net)
    out = {"kicad_cli": subprocess.run(["kicad-cli", "version"], capture_output=True, text=True).stdout.strip(), "elements": len(els), "messages": msgs[:5]}
    deg = {}
    for _, nodes, _ in els:
        for n in nodes:
            deg[n] = deg.get(n, 0) + 1
    gnd = {"0", "GND"}
    out["unconnected"] = sorted(n for n in deg if "unconnected" in n.lower())
    out["single_nodes"] = sorted(n for n, d in deg.items() if d < 2 and n not in gnd)
    ok = not out["unconnected"] and not out["single_nodes"]
    # the circuit against the lane subcircuit
    lib = subckts(open(os.path.join(lane_dir, f"{stem}.lib")).read())
    bench = subckts(open(os.path.join(lane_dir, f"{stem}_kicad.lib")).read())
    lane = next((k for k in lib if k.endswith("_lane")), None)
    lports, lbody = lib[lane]
    # the dies: the node of C_tx / C_rx and R_rx to ground (R_tx: its far end)
    die = {}
    for name, nodes, _ in els:
        m = re.match(r"^([CR])(TX|RX)([PN])1$", name)
        if m:
            k = (m.group(2), m.group(3))
            node = nodes[1] if m.group(1) == "R" and m.group(2) == "TX" else nodes[0]
            if m.group(1) == "C" or k not in die:
                die[k] = node
    want = {"ap": die.get(("TX", "P")), "an": die.get(("TX", "N")), "bp": die.get(("RX", "P")), "bn": die.get(("RX", "N"))}
    if len(lports) == 3:
        want = {"a": die.get(("TX", "P")), "b": die.get(("RX", "P"))}
    mapping = {"ref": None}
    rev = {}
    errors = []

    def bind(ln, kn):
        kn = "0" if kn in gnd else kn
        if ln == "ref":
            if kn != "0":
                errors.append(f"reference on {kn}")
            return
        if mapping.get(ln, kn) != kn or rev.get(kn, ln) != ln:
            errors.append(f"node {ln} on {kn} and {mapping.get(ln)} / {rev.get(kn)}")
        mapping[ln] = kn
        rev[kn] = ln

    kx = {}
    for name, nodes, model in els:
        if model.endswith("_k") or model.endswith("_kicad"):
            kx.setdefault(model, []).append(nodes)
    detailed = any(m.endswith("_k") for m in kx)
    if detailed:
        for t in lbody:
            nodes, sub = t[1:-1], t[-1]
            got = kx.pop(sub + "_k", [])
            if len(got) != 1:
                errors.append(f"{sub}: {len(got)} instance(s) in KiCad")
                continue
            wports, wbody = bench[sub + "_k"]
            inner = next((x[1:-1] for x in wbody if x[0].upper().startswith("X") and x[-1] == sub), None)
            if inner is None:
                inner = wports + ["0"] * (len(nodes) - len(wports))
            wmap = dict(zip(wports, got[0]))
            for ln, w in zip(nodes, inner):
                bind(ln, wmap.get(w, w))
        errors += [f"{m}: in KiCad, not in the lane" for m in kx]
    else:
        got = [v for k, v in kx.items() if k.endswith("_kicad")]
        if len(got) != 1 or len(got[0]) != 1:
            errors.append("compact: the lane symbol not found once")
        else:
            for ln, kn in zip(lports, got[0][0]):
                bind(ln, kn)
    for p, kn in want.items():
        if kn is not None and mapping.get(p) != kn:
            errors.append(f"port {p} on {mapping.get(p)}, the die is {kn}")
    out["same_as_lane"] = not errors
    out["lane_errors"] = errors[:8]
    out["sheet"] = "detailed" if detailed else "compact"
    out["ok"] = ok and not errors
    return out


# --- 3. ERC -------------------------------------------------------------------------------------------

def erc_check(lane_dir, stem):
    h = subprocess.run(["kicad-cli", "sch", "--help"], capture_output=True, text=True)
    if " erc" not in h.stdout + h.stderr:
        return {"available": False, "note": "this kicad-cli has no ERC (KiCad 7); KiCad 8 and later have `kicad-cli sch erc`"}
    r = run(["kicad-cli", "sch", "erc", "--format", "json", "--severity-all", "-o", "erc.json", f"{stem}.kicad_sch"], lane_dir)
    path = os.path.join(lane_dir, "erc.json")
    if not os.path.exists(path):
        return {"available": True, "ok": False, "error": (r.stdout + r.stderr)[-400:]}
    data = json.load(open(path))
    counts, examples = {}, {}
    for sh in data.get("sheets", []):
        for v in sh.get("violations", []):
            k = f"{v.get('severity')}:{v.get('type')}"
            counts[k] = counts.get(k, 0) + 1
            examples.setdefault(k, v.get("description"))
    bad = {k: n for k, n in counts.items() if k.startswith("error:") or k.split(":", 1)[1] in BAD_ERC}
    return {"available": True, "ok": not bad, "violations": counts, "examples": examples, "failing": bad}


# --- 4. AC and transient against the bench -------------------------------------------------------------

def _join(text):
    out = []
    for l in text.splitlines():
        if l.startswith("+") and out:
            out[-1] += " " + l[1:]
        else:
            out.append(l)
    return out


def ac_check(lane_dir, npts=40):
    """The KiCad netlist and the bench with the PRBS sources replaced by AC sources (P: +1, N: -1):
    the receiver's die voltage on 40 frequencies up to the bench's top frequency. Exact (no time
    steps), so the two must agree to rounding."""
    import numpy as np
    net = _join(open(os.path.join(lane_dir, "kicad_netlist.cir")).read())
    bench = _join(open(os.path.join(lane_dir, "bench_ngspice.cir")).read())
    fmax = 1.0 / float(re.search(r"^\.tran\s+(\S+)", "\n".join(bench), re.M).group(1)) / 20.0
    die = dict(re.findall(r"^[CR]RX([PN])1 (\S+) ", "\n".join(net), re.M))
    kprobe = f"v({die['P']})-v({die['N']})" if "N" in die else f"v({die['P']})"
    bprobe = re.search(r"^wrdata \S+ (.*)$", "\n".join(bench), re.M).group(1)
    ana = f".ac lin {npts} {fmax / npts:.6e} {fmax:.6e}"

    def fix(lines, src, probe, out):
        o = []
        for l in lines:
            m = src(l)
            if m:
                o.append(m)
            elif l.startswith(".tran"):
                o.append(ana)
            elif l.startswith("wrdata") or l.strip() in (".control", "run", ".endc"):
                continue
            elif l.strip().lower() == ".end":
                o.append(f".control\nrun\nwrdata {out} {probe}\n.endc\n.end")
            else:
                o.append(l)
        return "\n".join(o) + "\n"

    def ksrc(l):
        m = re.match(r"^XTX([PN])1 (\S+) (\S+) ", l + " ")
        return m and f"VTX{m.group(1)}1 {m.group(2)} 0 dc 0 ac {1 if m.group(1) == 'P' else -1}"

    def bsrc(l):
        m = re.match(r"^VTX_([pn]) (\S+) 0 PWL", l)
        return m and f"VTX_{m.group(1)} {m.group(2)} 0 dc 0 ac {1 if m.group(1) == 'p' else -1}"

    open(os.path.join(lane_dir, "kicad_ac.cir"), "w").write(fix(net, ksrc, kprobe, "kicad_ac.txt"))
    open(os.path.join(lane_dir, "bench_ac.cir"), "w").write(fix(bench, bsrc, bprobe, "bench_ac.txt"))
    for f in ("kicad_ac.cir", "bench_ac.cir"):
        r = subprocess.run(["ngspice", "-b", f], cwd=lane_dir, capture_output=True, text=True, timeout=7200)
        if r.returncode != 0 or not os.path.exists(os.path.join(lane_dir, f.replace(".cir", ".txt"))):
            return {"ok": False, "error": f"{f}: " + (r.stdout + r.stderr)[-300:]}
    a = np.loadtxt(os.path.join(lane_dir, "kicad_ac.txt"))
    b = np.loadtxt(os.path.join(lane_dir, "bench_ac.txt"))
    va, vb = a[:, 1] + 1j * a[:, 2], b[:, 1] + 1j * b[:, 2]
    rel = float(np.max(np.abs(va - vb)) / np.max(np.abs(vb)))
    return {"ok": rel < 1e-6, "points": len(va), "f_max_ghz": round(fmax / 1e9, 3), "max_rel_diff": rel}


def tran_check(lane_dir):
    import numpy as np
    net = open(os.path.join(lane_dir, "kicad_netlist.cir")).read()
    m = re.search(r"^\.tran\s+(\S+)\s+(\S+)", net, re.M)
    tstep, tstop = float(m.group(1)), float(m.group(2))
    short = tstop / 8
    die = dict(re.findall(r"^[CR]RX([PN])1 (\S+) ", net, re.M))
    probe = f"v({die['P']})-v({die['N']})" if "N" in die else f"v({die['P']})"
    net = re.sub(r"^\.tran.*$", f".tran {tstep:.6e} {short:.6e}", net, flags=re.M)
    net = net.replace(".end", f".control\nrun\nwrdata kicad_bench.txt {probe}\n.endc\n.end")
    open(os.path.join(lane_dir, "kicad_run.cir"), "w").write(net)
    bench = open(os.path.join(lane_dir, "bench_ngspice.cir")).read()
    bench = re.sub(r"^\.tran.*$", f".tran {tstep:.6e} {short:.6e}", bench, flags=re.M).replace("bench_ngspice.txt", "bench_short.txt")
    open(os.path.join(lane_dir, "bench_short.cir"), "w").write(bench)
    for f in ("kicad_run.cir", "bench_short.cir"):
        r = subprocess.run(["ngspice", "-b", f], cwd=lane_dir, capture_output=True, text=True, timeout=7200)
        if r.returncode != 0:
            return {"ok": False, "error": f"{f}: " + (r.stdout + r.stderr)[-300:]}
    a = np.loadtxt(os.path.join(lane_dir, "kicad_bench.txt"))
    b = np.loadtxt(os.path.join(lane_dir, "bench_short.txt"))
    vb = np.interp(a[:, 0], b[:, 0], b[:, 1])
    d = float(np.max(np.abs(a[:, 1] - vb)))
    swing = float(np.max(np.abs(b[:, 1])))
    return {"t_stop_ns": round(short * 1e9, 3), "max_diff_v": d, "swing_v": swing}


def svg(lane_dir, stem, out, png):
    os.makedirs(out, exist_ok=True)
    r = run(["kicad-cli", "sch", "export", "svg", f"{stem}.kicad_sch", "-o", os.path.abspath(out)], lane_dir)
    files = glob.glob(os.path.join(out, f"{stem}*.svg"))
    res = {"ok": bool(files), "files": [os.path.basename(f) for f in files]}
    if png and files:
        try:
            import cairosvg
            for f in files:
                cairosvg.svg2png(url=f, write_to=f[:-4] + ".png", output_width=3000, background_color="white")
        except ImportError:
            res["png"] = "cairosvg not installed"
    if not files:
        res["error"] = (r.stdout + r.stderr)[-300:]
    return res


def check(lane_dir, transient=False, svg_dir=None, png=False):
    sch = glob.glob(os.path.join(lane_dir, "*.kicad_sch"))
    if not sch:
        return {"ok": False, "error": "no .kicad_sch"}
    stem = os.path.basename(sch[0])[:-10]
    res = {"sheet": sheet_check(sch[0])}
    if shutil.which("kicad-cli"):
        res["netlist"] = netlist_check(lane_dir, stem)
        res["erc"] = erc_check(lane_dir, stem)
        if res["netlist"].get("ok") and shutil.which("ngspice") and os.path.exists(os.path.join(lane_dir, "bench_ngspice.cir")):
            res["ac"] = ac_check(lane_dir)
            if transient:
                res["transient"] = tran_check(lane_dir)
        if svg_dir:
            res["svg"] = svg(lane_dir, stem, svg_dir, png)
    else:
        res["netlist"] = {"skipped": "kicad-cli not found"}
    res["ok"] = res["sheet"]["ok"] and res["netlist"].get("ok", True) and res.get("erc", {}).get("ok", True) and res.get("ac", {}).get("ok", True)
    return res


def main():
    a = sys.argv[1:]
    if not a:
        raise SystemExit(__doc__)
    svg_dir = a[a.index("--svg") + 1] if "--svg" in a else None
    res = check(a[0], "--transient" in a, svg_dir, "--png" in a)
    print(json.dumps(res, indent=1))
    sys.exit(0 if res["ok"] else 1)


if __name__ == "__main__":
    main()
