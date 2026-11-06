#!/usr/bin/env python3
"""Figures and tables of docs/research/pi-quadtree.md from `kisi-validate pi-refine` results.

  plot.py tables RESULTS.json…                    Markdown tables (drop error, cost) per rail
  plot.py summary OUT.png RESULTS.json…           |drop error| against unknowns, every rail and variant
  plot.py maps OUT.png DUMP_DIR [VARIANT…]        error maps of one rail (the `--dump` of pi-refine)

Needs numpy and matplotlib.
"""
import json, os, sys

import numpy as np

# reference palette (dataviz skill): categorical slots in fixed order, blue↔red diverging with a gray midpoint
CAT = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"]
SURF, INK, MUTED, GRID = "#fcfcfb", "#0b0b0b", "#6b6a63", "#e1e0d9"
SUMMARY = ["uniform", "uniform_x2", "peaceman", "lines_pm", "lines_pm_cut4", "qt2", "qt2_lines_pm", "lines_pm_eq"]
LABEL = {
    "uniform": "uniform (today)",
    "uniform_x2": "uniform, cell/2",
    "peaceman": "Peaceman",
    "cut4": "cut-cell",
    "lines": "line tracks",
    "lines_pm": "lines + Peaceman",
    "lines_pm_cut4": "lines + Peaceman + cut-cell",
    "lines_pm_x2": "lines + Peaceman, cell/2",
    "qt2": "quadtree (2 levels)",
    "qt2_lines_pm": "quadtree + lines + Peaceman",
    "uniform_eq": "uniform, as many unknowns as quadtree",
    "lines_pm_eq": "lines + Peaceman, as many unknowns as quadtree",
}


def load(paths):
    out = []
    for p in paths:
        d = json.load(open(p))
        for r in d["rails"]:
            r["board"] = d["board"].replace(".kicad_pcb", "")
            out.append(r)
    return out


def tables(paths):
    rails = load(paths)
    names = [x["variant"] for x in rails[0]["rows"] if not x["variant"].startswith("ref_")]
    print("| board | rail | cell mm | ref | ref − next coarser | " + " | ".join(names) + " |")
    print("|---|---|---|---|---|" + "---|" * len(names))
    for r in rails:
        rows = {x["variant"]: x for x in r["rows"]}
        conv = [x for x in r["rows"] if x["variant"].startswith("ref_")]
        cv = f"{-conv[0]['drop_err_pct']:+.2f} %" if conv else "–"
        cells = []
        for n in names:
            x = rows.get(n)
            if x is None:
                cells.append("–")
                continue
            s = f"{x['drop_err_pct']:+.1f}"
            if x.get("unconnected_loads"):
                s += f" ({x['unconnected_loads']} open)"
            cells.append(s)
        u = rows["uniform"]
        print(f"| {r['board']} | {r['rail']} | {u['cell_mm']:.3f} | {r['reference']} ({r['ref_unknowns'] / 1e6:.2f} M) | {cv} | " + " | ".join(cells) + " |")
    print()
    print("| board | rail | " + " | ".join(f"{n} N / t" for n in names) + " |")
    print("|---|---|" + "---|" * len(names))
    for r in rails:
        rows = {x["variant"]: x for x in r["rows"]}
        u = rows["uniform"]
        cells = []
        for n in names:
            x = rows.get(n)
            cells.append("–" if x is None else f"{x['unknowns'] / u['unknowns']:.1f}× / {x['time_s'] / max(u['time_s'], 1e-3):.1f}×")
        print(f"| {r['board']} | {r['rail']} | " + " | ".join(cells) + " |")
    # medians over the rails
    print()
    print("| variant | median abs drop error | max abs drop error | median abs error at the loads | median unknowns | median time |")
    print("|---|---|---|---|---|---|")
    for n in names:
        e, l, nu, t = [], [], [], []
        for r in rails:
            rows = {x["variant"]: x for x in r["rows"]}
            if n not in rows:
                continue
            x, u = rows[n], rows["uniform"]
            e.append(abs(x["drop_err_pct"]))
            l.append(x["load_max_pct"])
            nu.append(x["unknowns"] / u["unknowns"])
            t.append(x["time_s"] / max(u["time_s"], 1e-3))
        print(f"| {LABEL.get(n, n)} | {np.median(e):.2f} % | {max(e):.1f} % | {np.median(l):.2f} % | {np.median(nu):.1f}× | {np.median(t):.1f}× |")


def style(ax):
    ax.set_facecolor(SURF)
    for s in ax.spines.values():
        s.set_color("#c3c2b7")
    ax.tick_params(colors=MUTED, labelsize=8)
    ax.grid(True, color=GRID, linewidth=0.6)
    ax.set_axisbelow(True)


def summary(out, paths):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    rails = load(paths)
    fig, ax = plt.subplots(figsize=(8.0, 4.8), dpi=150)
    fig.patch.set_facecolor(SURF)
    style(ax)
    for i, n in enumerate(SUMMARY):
        xs, ys = [], []
        for r in rails:
            rows = {x["variant"]: x for x in r["rows"]}
            if n in rows:
                xs.append(rows[n]["unknowns"] / rows["uniform"]["unknowns"])
                ys.append(max(abs(rows[n]["drop_err_pct"]), 0.01))
        ax.scatter(xs, ys, s=30, color=CAT[i], edgecolor=SURF, linewidth=1.0, label=LABEL[n], zorder=3)
        if ys:
            ax.scatter([np.median(xs)], [np.median(ys)], s=140, marker="_", color=CAT[i], linewidth=2.5, zorder=4)
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("unknowns relative to today's raster", color=INK, fontsize=9)
    ax.set_ylabel("|error of the largest drop| vs reference, %", color=INK, fontsize=9)
    ax.set_title("DC IR drop: accuracy against cost, %d rails on 7 boards (bar: median)" % len(rails), color=INK, fontsize=10, loc="left")
    leg = ax.legend(fontsize=7.5, frameon=False, loc="upper left", bbox_to_anchor=(1.01, 1.0))
    for t in leg.get_texts():
        t.set_color(INK)
    fig.tight_layout()
    fig.savefig(out, facecolor=SURF)
    print("wrote", out)


def maps(out, d, variants):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.colors import LinearSegmentedColormap

    idx = json.load(open(os.path.join(d, "index.json")))
    nx, ny, c = idx["nx"], idx["ny"], idx["cell"]
    ext = [idx["x0"], idx["x0"] + nx * c, idx["y0"] + ny * c, idx["y0"]]
    drop = np.fromfile(os.path.join(d, "reference_drop_mv.f32"), dtype="<f4").reshape(ny, nx)
    ref = idx["drop_ref_mv"]
    have = {m["variant"]: m for m in idx["maps"]}
    variants = [v for v in (variants or list(have)) if v in have]
    div = LinearSegmentedColormap.from_list("div", ["#184f95", "#6da7ec", "#f0efec", "#f08c8b", "#a32c2b"])
    seq = LinearSegmentedColormap.from_list("seq", ["#cde2fb", "#6da7ec", "#256abf", "#0d366b"])
    errs = {v: np.fromfile(os.path.join(d, have[v]["file"]), dtype="<f4").reshape(ny, nx) / (ref * 1e3) * 100 for v in variants}
    lim = max(np.nanpercentile(np.abs(errs[variants[0]]), 99.5), 0.5)
    n = len(variants) + 1
    cols = min(n, 3)
    rows = (n + cols - 1) // cols
    aspect = (ny / nx)
    fig, axs = plt.subplots(rows, cols, figsize=(4.2 * cols, 4.2 * rows * aspect + 0.9), dpi=150, squeeze=False)
    fig.patch.set_facecolor(SURF)
    for a in axs.flat:
        a.set_visible(False)
    a = axs.flat[0]
    a.set_visible(True)
    im = a.imshow(np.ma.masked_invalid(drop), extent=ext, cmap=seq, interpolation="nearest")
    a.set_title(f"reference drop, mV (max {ref:.2f})", fontsize=8.5, color=INK, loc="left")
    cb = fig.colorbar(im, ax=a, fraction=0.046, pad=0.02)
    cb.ax.tick_params(labelsize=7, colors=MUTED)
    for k, v in enumerate(variants):
        a = axs.flat[k + 1]
        a.set_visible(True)
        im = a.imshow(np.ma.masked_invalid(errs[v]), extent=ext, cmap=div, vmin=-lim, vmax=lim, interpolation="nearest")
        a.set_title(f"{LABEL.get(v, v)}: {have[v]['drop_err_pct']:+.1f} %", fontsize=8.5, color=INK, loc="left")
        cb = fig.colorbar(im, ax=a, fraction=0.046, pad=0.02)
        cb.ax.tick_params(labelsize=7, colors=MUTED)
        cb.set_label("error, % of max drop", fontsize=7, color=MUTED)
    for a in axs.flat:
        a.tick_params(labelsize=6, colors=MUTED)
        a.set_facecolor("#ffffff")
    fig.suptitle(f"{idx['rail']}, copper layer {idx['layer']}: error of the voltage against the reference (largest |error| per block of {c:.2f} mm)",
                 fontsize=9, color=INK, x=0.01, ha="left")
    fig.tight_layout()
    fig.savefig(out, facecolor=SURF)
    print("wrote", out)


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    cmd = sys.argv[1]
    if cmd == "tables":
        tables(sys.argv[2:])
    elif cmd == "summary":
        summary(sys.argv[2], sys.argv[3:])
    elif cmd == "maps":
        maps(sys.argv[2], sys.argv[3], sys.argv[4:])
    else:
        print(__doc__)
        sys.exit(2)
