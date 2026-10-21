"""Compare kisi-fdtd and openEMS results of a case: |S11|, |S21| in dB, difference statistics,
run times; optional plot. usage: python3 compare.py rust.json openems.json [plot.png]"""
import json, math, sys

rs = json.load(open(sys.argv[1]))
oe = json.load(open(sys.argv[2]))
run = rs["runs"][0]
f = rs["freq"]
db = lambda c: 20 * math.log10(max(math.hypot(*c), 1e-12))
out = {"case": rs["name"], "cells": rs["cells"], "rust_s": run["seconds"], "rust_steps": run["steps"], "openems_s": oe["seconds"], "openems_threads": oe["threads"]}
for i, name in enumerate(["S11", "S21"][: len(run["s"])]):
    a = [db(x) for x in run["s"][i]]
    b = [db(x) for x in oe["s"][i]]
    d = [abs(x - y) for x, y in zip(a, b)]
    # S11 differences only where S11 is above -30 dB (deep nulls are not comparable)
    if name == "S11":
        d = [abs(x - y) for x, y in zip(a, b) if max(x, y) > -30]
    out[name] = {"max_diff_db": round(max(d), 2) if d else None, "mean_diff_db": round(sum(d) / len(d), 3) if d else None,
                 "rust_at": {f"{g/1e9:.0f}GHz": round(a[min(range(len(f)), key=lambda k: abs(f[k]-g))], 2) for g in (1e9, 5e9, 10e9, 15e9, 20e9)},
                 "openems_at": {f"{g/1e9:.0f}GHz": round(b[min(range(len(f)), key=lambda k: abs(f[k]-g))], 2) for g in (1e9, 5e9, 10e9, 15e9, 20e9)}}
print(json.dumps(out, indent=1))
if len(sys.argv) > 3:
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    fig, ax = plt.subplots(figsize=(8, 4.5))
    fg = [x / 1e9 for x in f]
    for i, (name, col) in enumerate([("S11", "C1"), ("S21", "C0")][: len(run["s"])]):
        ax.plot(fg, [db(x) for x in oe["s"][i]], col, lw=2.2, alpha=0.5, label=f"{name} openEMS")
        ax.plot(fg, [db(x) for x in run["s"][i]], col, lw=1, ls="--", label=f"{name} kisi-fdtd")
    ax.set_xlabel("GHz"); ax.set_ylabel("dB"); ax.set_ylim(-50, 2); ax.grid(alpha=0.3); ax.legend(); ax.set_title(rs["name"])
    fig.tight_layout(); fig.savefig(sys.argv[3], dpi=110)
