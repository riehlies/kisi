#!/usr/bin/env python3
"""The volume field output of kisi-fdtd (docs/research/fdtd-field-view.md): read the blob of
`kisi-fdtd case.json --volume out.bin` (or `fd_run_volume` in WebAssembly), check the power balance
and draw cut planes.

  python3 volume_fields.py check out.bin [--x1 -2 --zmin1 1.4 --x2 2 --zmax2 0.2]
  python3 volume_fields.py compare a.bin b.bin          # same case, two sampling settings
  python3 volume_fields.py plot out.bin outdir [--freq 2e10] [--y 0] [--z 0.8]
  python3 volume_fields.py vtk out.bin out.vtk [--freq 2e10]   # ParaView: legacy VTK rectilinear grid

Fields are peak phasors for 1 W incident at the excited port; S = ½ Re(E × H*) (W/m²), the
dielectric loss density ½ σ |E|² (W/m³) with the solver's σ (tanδ fixed at f_loss).
"""
import argparse
import json
import os
import struct
import sys

import numpy as np


def load(path):
    b = open(path, "rb").read()
    (hl,) = struct.unpack_from("<I", b, 0)
    h = json.loads(b[4:4 + hl].decode())
    base = 4 + hl
    shape = (len(h["x"]), len(h["y"]), len(h["z"]))
    a = {}
    for d in h["arrays"]:
        dt = np.float32 if d["dtype"] == "f32" else np.uint8
        a[d["name"]] = np.frombuffer(b, dtype=dt, count=d["len"], offset=base + d["offset"]).reshape(shape)
    for k in ("x", "y", "z", "x1", "y1", "z1"):
        h[k] = np.array(h[k]) * 1e-3  # m
    return h, a


def comps(a, fi):
    return {c: a[f"{c}_{fi}_re"].astype(np.float64) + 1j * a[f"{c}_{fi}_im"] for c in ("Ex", "Ey", "Ez", "Hx", "Hy", "Hz")}


def sparam(h, f):
    fr = np.array(h["extra"]["freq"])
    k = int(np.argmin(abs(fr - f)))
    s = [complex(*h["extra"]["s"][p][k]) for p in range(len(h["extra"]["s"]))]
    return fr[k], s


def spacing(h):
    """primary (to the next line) and dual spacings per axis, in m (stride 1)"""
    out = []
    for ax in "xyz":
        p = h[ax + "1"] - h[ax]
        d = np.empty_like(p)
        d[0] = p[0]
        d[1:] = 0.5 * (p[:-1] + p[1:])
        out.append((p, d))
    return out


def flux_x(h, F, i, zsel=None):
    """∫ S_x dA over the plane x = x_i (H averaged over the two half cells around it)"""
    (_, _), (py, dy), (pz, dz) = spacing(h)
    hz = 0.5 * (F["Hz"][i - 1] + F["Hz"][i])
    hy = 0.5 * (F["Hy"][i - 1] + F["Hy"][i])
    t1 = 0.5 * np.real(F["Ey"][i] * np.conj(hz)) * np.outer(py, dz)  # (j+½, k)
    t2 = -0.5 * np.real(F["Ez"][i] * np.conj(hy)) * np.outer(dy, pz)  # (j, k+½)
    if zsel is not None:
        t1 = t1 * zsel(h["z"])[None, :]
        t2 = t2 * zsel(0.5 * (h["z"] + h["z1"]))[None, :]
    return t1[:-1, 1:-1].sum() + t2[1:-1, :-1].sum()


def box_flux(h, F, m=2):
    """outward flux through a box m nodes inside the sampled domain (all six faces)"""
    (px, dx), (py, dy), (pz, dz) = spacing(h)
    nx, ny, nz = F["Ex"].shape
    i0, i1, j0, j1, k0, k1 = m, nx - 1 - m, m, ny - 1 - m, m, nz - 1 - m
    tot = 0.0
    # x faces
    for i, sg in ((i0, -1), (i1, 1)):
        hz = 0.5 * (F["Hz"][i - 1] + F["Hz"][i])
        hy = 0.5 * (F["Hy"][i - 1] + F["Hy"][i])
        a = 0.5 * np.real(F["Ey"][i] * np.conj(hz)) * np.outer(py, dz)
        b = -0.5 * np.real(F["Ez"][i] * np.conj(hy)) * np.outer(dy, pz)
        tot += sg * (a[j0:j1, k0:k1 + 1].sum() + b[j0:j1 + 1, k0:k1].sum())
    for j, sg in ((j0, -1), (j1, 1)):  # S_y = Ez Hx* − Ex Hz*
        hx = 0.5 * (F["Hx"][:, j - 1] + F["Hx"][:, j])
        hz = 0.5 * (F["Hz"][:, j - 1] + F["Hz"][:, j])
        a = 0.5 * np.real(F["Ez"][:, j] * np.conj(hx)) * np.outer(dx, pz)
        b = -0.5 * np.real(F["Ex"][:, j] * np.conj(hz)) * np.outer(px, dz)
        tot += sg * (a[i0:i1 + 1, k0:k1].sum() + b[i0:i1, k0:k1 + 1].sum())
    for k, sg in ((k0, -1), (k1, 1)):  # S_z = Ex Hy* − Ey Hx*
        hy = 0.5 * (F["Hy"][:, :, k - 1] + F["Hy"][:, :, k])
        hx = 0.5 * (F["Hx"][:, :, k - 1] + F["Hx"][:, :, k])
        a = 0.5 * np.real(F["Ex"][:, :, k] * np.conj(hy)) * np.outer(px, dy)
        b = -0.5 * np.real(F["Ey"][:, :, k] * np.conj(hx)) * np.outer(dx, py)
        tot += sg * (a[i0:i1, j0:j1 + 1].sum() + b[i0:i1 + 1, j0:j1].sum())
    return tot


def diel_loss(h, a, F):
    """∫ ½ σ |E|² dV, each E edge with its dual volume"""
    (px, dx), (py, dy), (pz, dz) = spacing(h)
    vol = [px[:, None, None] * dy[None, :, None] * dz[None, None, :],
           dx[:, None, None] * py[None, :, None] * dz[None, None, :],
           dx[:, None, None] * dy[None, :, None] * pz[None, None, :]]
    tot = 0.0
    for c, n in enumerate(("Ex", "Ey", "Ez")):
        p = 0.5 * a[f"sigma{c}"] * np.abs(F[n]) ** 2 * vol[c]
        tot += p[1:-1, 1:-1, 1:-1].sum()
    return tot


def nodal(F, h):
    """|E|², |H|² and S averaged to the cell corners (nodes), for pictures"""
    def avg(v, axes):
        out = v.copy()
        for ax in axes:
            sh = np.roll(out, 1, axis=ax)
            out = 0.5 * (out + sh)
        return out
    E = [avg(F["Ex"], [0]), avg(F["Ey"], [1]), avg(F["Ez"], [2])]
    H = [avg(F["Hx"], [1, 2]), avg(F["Hy"], [0, 2]), avg(F["Hz"], [0, 1])]
    S = [0.5 * np.real(E[1] * np.conj(H[2]) - E[2] * np.conj(H[1])),
         0.5 * np.real(E[2] * np.conj(H[0]) - E[0] * np.conj(H[2])),
         0.5 * np.real(E[0] * np.conj(H[1]) - E[1] * np.conj(H[0]))]
    return E, H, S


def cmd_check(args):
    h, a = load(args.blob)
    x = h["x"] * 1e3
    rows = []
    for fi, f in enumerate(h["freqs"]):
        F = comps(a, fi)
        fr, s = sparam(h, f)
        i1, i2 = int(np.argmin(abs(x - args.x1))), int(np.argmin(abs(x - args.x2)))
        p1 = flux_x(h, F, i1, lambda zc: zc * 1e3 > args.zmin1 + 1e-6)
        p2 = flux_x(h, F, i2, lambda zc: zc * 1e3 < args.zmax2 - 1e-6)
        pin = 1 - abs(s[0]) ** 2
        p21 = abs(s[1]) ** 2 if len(s) > 1 else float("nan")
        ld = diel_loss(h, a, F)
        out = box_flux(h, F)
        r = dict(f=f, f_s=fr, s11=abs(s[0]), s21=abs(s[1]) if len(s) > 1 else None, p_in_from_S=pin,
                 flux_port1_side=p1, p21_from_S=p21, flux_port2_side=p2, diel_loss=ld, box_outflow=out,
                 balance=1 - abs(s[0]) ** 2 - p21)
        rows.append(r)
        print(f"{f / 1e9:.1f} GHz: 1-|S11|² {pin:.4f} W, flux at x={args.x1} mm (z>{args.zmin1}) {p1:.4f} W; "
              f"|S21|² {p21:.4f} W, flux at x={args.x2} mm (z<{args.zmax2}) {p2:.4f} W; "
              f"1-|S11|²-|S21|² {r['balance']:.4f} W = dielectric loss {ld:.4f} W + out of the box {out:.4f} W "
              f"(sum {ld + out:.4f})")
    if args.json:
        json.dump(rows, open(args.json, "w"), indent=1)


def cmd_compare(args):
    ha, aa = load(args.a)
    hb, ab = load(args.b)
    for fi, f in enumerate(ha["freqs"]):
        fj = hb["freqs"].index(f)
        A, B = comps(aa, fi), comps(ab, fj)
        for grp in (("Ex", "Ey", "Ez"), ("Hx", "Hy", "Hz")):
            num = sum(np.sum(abs(A[c] - B[c]) ** 2) for c in grp)
            den = sum(np.sum(abs(A[c]) ** 2) for c in grp)
            mx = max(np.max(abs(A[c])) for c in grp)
            dmax = max(np.max(abs(A[c] - B[c])) for c in grp)
            print(f"{f / 1e9:.1f} GHz {grp[0][0]}: relative rms difference {np.sqrt(num / den):.2e}, "
                  f"max |Δ| {dmax / mx:.2e} of the max (every {ha['every']} vs {hb['every']})")


def cmd_plot(args):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.colors import LogNorm

    h, a = load(args.blob)
    fi = int(np.argmin([abs(f - args.freq) for f in h["freqs"]]))
    f = h["freqs"][fi]
    F = comps(a, fi)
    E, H, S = nodal(F, h)
    x, y, z = h["x"] * 1e3, h["y"] * 1e3, h["z"] * 1e3
    e2 = sum(abs(c) ** 2 for c in E)
    h2 = sum(abs(c) ** 2 for c in H)
    metal = [a[f"free{c}"] == 0 for c in range(3)]
    plt.rcParams.update({"font.size": 9, "figure.facecolor": "white", "axes.facecolor": "white"})
    os.makedirs(args.out, exist_ok=True)
    name = h["extra"]["name"]

    # section y = y0: |E| with S arrows and the metal edges
    j = int(np.argmin(abs(y - args.y)))
    fig, ax = plt.subplots(figsize=(9, 3.6))
    mag = np.sqrt(e2[:, j, :]).T
    pc = ax.pcolormesh(x, z, mag, shading="nearest", cmap="magma_r",
                       norm=LogNorm(vmin=mag.max() / 300, vmax=mag.max()))
    fig.colorbar(pc, ax=ax, label="|E| (V/m, peak, 1 W incident)")
    mx_ = np.argwhere(metal[0][:, j, :] | metal[2][:, j, :])
    ax.scatter(x[mx_[:, 0]], z[mx_[:, 1]], s=1.2, c="#1f77b4", marker="s", lw=0, label="metal edges")
    sx, sz = S[0][:, j, :].T, S[2][:, j, :].T
    st = 3
    X, Z = np.meshgrid(x, z)
    sm = np.hypot(sx, sz)
    keep = sm > sm.max() * 1e-3
    u, w = np.where(keep, sx / (sm + 1e-30), 0), np.where(keep, sz / (sm + 1e-30), 0)
    ax.quiver(X[::st, ::st], Z[::st, ::st], u[::st, ::st], w[::st, ::st], color="#2a9d8f", scale=60, width=0.0025,
              headwidth=4, alpha=0.9)
    ax.set_aspect("equal")
    ax.set_xlabel("x (mm)")
    ax.set_ylabel("z (mm)")
    ax.set_title(f"{name}: |E| and the direction of S = ½Re(E×H*) in the section y = {y[j]:.2f} mm, "
                 f"{f / 1e9:.0f} GHz (kisi-fdtd volume DFT, prototype)", fontsize=8.5)
    ax.legend(loc="lower left", fontsize=7, markerscale=4)
    fig.tight_layout()
    p1 = os.path.join(args.out, f"{name}_E_section_y{y[j]:.2f}_{f / 1e9:.0f}GHz.png")
    fig.savefig(p1, dpi=150)
    plt.close(fig)

    # top view z = z0: |H| with the metal (planes' antipad, vias) and the loss density
    k = int(np.argmin(abs(z - args.z)))
    fig, axs = plt.subplots(1, 2, figsize=(10, 3.4))
    hm = np.sqrt(h2[:, :, k]).T
    pc = axs[0].pcolormesh(x, y, hm, shading="nearest", cmap="viridis_r", norm=LogNorm(vmin=hm.max() / 300, vmax=hm.max()))
    fig.colorbar(pc, ax=axs[0], label="|H| (A/m)")
    mz = np.argwhere(metal[2][:, :, k])
    axs[0].scatter(x[mz[:, 0]], y[mz[:, 1]], s=1.5, c="#d62828", marker="s", lw=0)
    hx_, hy_ = np.real(H[0][:, :, k]).T, np.real(H[1][:, :, k]).T
    hn = np.hypot(hx_, hy_)
    Xg, Yg = np.meshgrid(x, y)
    sel = (np.abs(Xg) < 2) & (np.abs(Yg) < 2) & (hn > hn.max() * 1e-3)
    sel[::2, :] = False
    sel[:, ::2] = False
    axs[0].quiver(Xg[sel], Yg[sel], (hx_ / (hn + 1e-30))[sel], (hy_ / (hn + 1e-30))[sel], color="white", scale=60,
                  width=0.003, alpha=0.85)
    axs[0].set_title(f"|H| at z = {z[k]:.2f} mm; arrows Re H, metal red", fontsize=8.5)
    loss = 0.5 * (a["sigma0"] * abs(F["Ex"]) ** 2 + a["sigma1"] * abs(F["Ey"]) ** 2 + a["sigma2"] * abs(F["Ez"]) ** 2)
    lm = loss[:, :, k].T
    pc = axs[1].pcolormesh(x, y, np.maximum(lm, lm.max() / 1e4), shading="nearest", cmap="inferno_r",
                           norm=LogNorm(vmin=lm.max() / 1e3, vmax=lm.max()))
    fig.colorbar(pc, ax=axs[1], label="½σ|E|² (W/m³)")
    axs[1].set_title("dielectric loss density, same plane", fontsize=8.5)
    for ax_ in axs:
        ax_.set_aspect("equal")
        ax_.set_xlabel("x (mm)")
        ax_.set_ylabel("y (mm)")
    fig.suptitle(f"{name}, {f / 1e9:.0f} GHz, 1 W incident at port 1 (kisi-fdtd volume DFT, prototype)", fontsize=9)
    fig.tight_layout()
    p2 = os.path.join(args.out, f"{name}_H_loss_top_z{z[k]:.2f}_{f / 1e9:.0f}GHz.png")
    fig.savefig(p2, dpi=150)
    plt.close(fig)
    print(p1)
    print(p2)


def cmd_vtk(args):
    """legacy binary VTK (RECTILINEAR_GRID, point data at the mesh nodes): |E|, |H|, S, loss density,
    Re E and Re H at phase 0, the metal mask; opens in ParaView (slices, glyphs, streamlines, volume)"""
    h, a = load(args.blob)
    fi = int(np.argmin([abs(f - args.freq) for f in h["freqs"]]))
    F = comps(a, fi)
    E, H, S = nodal(F, h)
    loss = 0.5 * (a["sigma0"] * abs(F["Ex"]) ** 2 + a["sigma1"] * abs(F["Ey"]) ** 2 + a["sigma2"] * abs(F["Ez"]) ** 2)
    metal = ((a["free0"] == 0) | (a["free1"] == 0) | (a["free2"] == 0)).astype(np.float32)
    nx, ny, nz = F["Ex"].shape
    # VTK point order: x fastest → transpose (i, j, k) arrays to (k, j, i)
    fl = lambda v: np.ascontiguousarray(np.transpose(v, (2, 1, 0))).astype(">f4").ravel()
    with open(args.out, "wb") as o:
        w = lambda t: o.write(t.encode())
        w(f"# vtk DataFile Version 3.0\nkisi-fdtd {h['extra']['name']} {h['freqs'][fi] / 1e9:g} GHz, 1 W incident, mm\n")
        w(f"BINARY\nDATASET RECTILINEAR_GRID\nDIMENSIONS {nx} {ny} {nz}\n")
        for ax, n in (("X", nx), ("Y", ny), ("Z", nz)):
            w(f"{ax}_COORDINATES {n} float\n")
            o.write((h[ax.lower()] * 1e3).astype(">f4").tobytes())
            w("\n")
        w(f"POINT_DATA {nx * ny * nz}\n")
        for name, v in (("E_abs", np.sqrt(sum(abs(c) ** 2 for c in E))), ("H_abs", np.sqrt(sum(abs(c) ** 2 for c in H))),
                        ("loss_density", loss), ("metal", metal)):
            w(f"SCALARS {name} float 1\nLOOKUP_TABLE default\n")
            o.write(fl(v).tobytes())
            w("\n")
        for name, vv in (("S", S), ("E_re", [np.real(c) for c in E]), ("H_re", [np.real(c) for c in H])):
            w(f"VECTORS {name} float\n")
            o.write(np.stack([fl(c) for c in vv], axis=1).astype(">f4").tobytes())
            w("\n")
    print(args.out, os.path.getsize(args.out), "bytes")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    c = sp.add_parser("check")
    c.add_argument("blob")
    c.add_argument("--x1", type=float, default=-2.0)
    c.add_argument("--zmin1", type=float, default=1.4)
    c.add_argument("--x2", type=float, default=2.0)
    c.add_argument("--zmax2", type=float, default=0.2)
    c.add_argument("--json")
    c = sp.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c = sp.add_parser("plot")
    c.add_argument("blob")
    c.add_argument("out")
    c.add_argument("--freq", type=float, default=20e9)
    c.add_argument("--y", type=float, default=0.0)
    c.add_argument("--z", type=float, default=0.8)
    c = sp.add_parser("vtk")
    c.add_argument("blob")
    c.add_argument("out")
    c.add_argument("--freq", type=float, default=20e9)
    args = ap.parse_args()
    {"check": cmd_check, "compare": cmd_compare, "plot": cmd_plot, "vtk": cmd_vtk}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
