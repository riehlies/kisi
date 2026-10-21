"""Reduced plane-cavity model against the 3D references of group G.

Rectangular plane pair a × b, height d, magnetic walls at the edges (open board edge), square
ports of side w (the 3D lumped port: one cell):
  Z_ij = jωμd/(ab) · Σ_m Σ_n χm² χn² cos(kxm xi) cos(kyn yi) cos(kxm xj) cos(kyn yj)
                       · sinc²(kxm w/2) sinc²(kyn w/2) / (kxm² + kyn² − k²)
with kxm = mπ/a, kyn = nπ/b, χ0 = 1, χ = √2 otherwise, k² = ω²με(1 − j tanδ_eff); the 3D
references fix σ at f_loss, so tanδ_eff = tanδ · f_loss / f. Optional fringing: a + 2Δ, b + 2Δ.

usage: python3 cavity_model.py   → results/cavity_model.json, via_ref/G_*/model.s2p
"""
import json, math, os, sys
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402

EPS0, MU0 = 8.8541878128e-12, 4e-7 * math.pi


def zmatrix(f, a, b, d, er, tand, f_loss, ports, w, M=120, N=80, fringe=0.0):
    a2, b2 = a + 2 * fringe, b + 2 * fringe
    m = np.arange(M)[:, None]
    n = np.arange(N)[None, :]
    kx, ky = m * math.pi / a2, n * math.pi / b2
    chi = np.where(m == 0, 1.0, math.sqrt(2)) * np.where(n == 0, 1.0, math.sqrt(2))
    sx = np.sinc(kx * w / 2 / math.pi) ** 2 * np.sinc(ky * w / 2 / math.pi) ** 2
    cosf = [np.cos(kx * (x + fringe)) * np.cos(ky * (y + fringe)) for (x, y) in ports]
    Z = np.empty((len(f), len(ports), len(ports)), complex)
    for q, fk in enumerate(f):
        w0 = 2 * math.pi * fk
        k2 = w0 ** 2 * MU0 * EPS0 * er * (1 - 1j * tand * (f_loss / fk))
        den = kx ** 2 + ky ** 2 - k2
        base = 1j * w0 * MU0 * d / (a2 * b2) * chi ** 2 * sx / den
        for i in range(len(ports)):
            for j in range(len(ports)):
                Z[q, i, j] = np.sum(base * cosf[i] * cosf[j])
    return Z


def main():
    out = {}
    for name in ("G_pp100", "G_pp200"):
        c = json.load(open(os.path.join(HERE, "via_cases", name + ".json")))
        p = os.path.join(REF, name, "kisi.s2p")
        if not os.path.exists(p):
            continue
        f, S = sparam.read_touchstone(p)
        Z3 = sparam.s2z(S, 1.0)
        bx = c["board"]
        a, b = (bx[2] - bx[0]) * 1e-3, (bx[3] - bx[1]) * 1e-3
        lay = c["layers"][0]
        d = (lay["z1"] - lay["z0"]) * 1e-3
        ports = [((pp["x"] - bx[0]) * 1e-3, ((pp["y0"] + pp["y1"]) / 2 - bx[1]) * 1e-3) for pp in c["ports"]]
        w = c["mesh"]["dfine"] * 1e-3
        fl = c.get("f_loss") or c["fmax"] / 2
        res = {}
        for tag, fr in (("model", 0.0), ("model_fringe", d * math.log(2) / math.pi)):
            Zm = zmatrix(f, a, b, d, lay["er"], lay.get("tand", 0), fl, ports, w, fringe=fr)
            res[tag] = Zm
        Zm = res["model"]
        I = np.eye(2)
        Sm = (Zm - I) @ np.linalg.inv(Zm + I)  # 1 Ω reference
        sparam.write_touchstone(os.path.join(REF, name, "model.s2p"), f, Sm, [f"{name}: cavity model (double cosine series), 1 ohm"], 1.0)

        def peaks(z):
            m = np.abs(z)
            return [float(f[i]) for i in range(1, len(f) - 1) if m[i] > m[i - 1] and m[i] >= m[i + 1] and m[i] > 3 * np.median(m)]
        p3, pm = peaks(Z3[:, 0, 0]), peaks(Zm[:, 0, 0])
        pairs = []
        for x in p3[:6]:
            y = min(pm, key=lambda v: abs(v - x)) if pm else None
            pairs.append([round(x / 1e9, 3), round(y / 1e9, 3) if y else None, round((y / x - 1) * 100, 2) if y else None])
        band = (f > 0.05e9) & (f < 1.0e9)
        out[name] = {"d_mm": d * 1e3, "resonances_GHz_3d_model_errpct": pairs,
                     "Z11_below_1GHz_mean_ratio": round(float(np.mean(np.abs(Zm[band, 0, 0]) / np.abs(Z3[band, 0, 0]))), 3),
                     "Z21_below_1GHz_mean_ratio": round(float(np.mean(np.abs(Zm[band, 1, 0]) / np.abs(Z3[band, 1, 0]))), 3)}
        print(name, out[name], flush=True)
    json.dump(out, open(os.path.join(HERE, "results", "cavity_model.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
