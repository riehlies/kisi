#!/usr/bin/env python3
"""Compact line models for the SPICE export (docs/research/vector-fitting.md): a study on kisi's line models.

Usage: compact_study.py [line_models.json] [--ngspice]

line_models.json: per-metre R(f), L(f), G(f), C(f) of four kisi line models (ThunderScope Rev5.3, formula impedances:
SYNC_OUT_P's stripline even and odd mode, a PCIe microstrip, REFINOUT2's microstrip), sampled from 100 Hz to 200 GHz.

A: method of characteristics (Branin, Marti, the universal line model): the characteristic admittance Yc(s) fitted
   in passive RC form (non-negative least squares), the propagation function H(s) = e^{-gamma l} = e^{-s tau} Hm(s)
   with tau from L and C at the top of the data and Hm fitted by (relaxed) vector fitting; the line's 2-port
   Y11 = Yc (1 + H^2)/(1 - H^2), Y12 = -2 Yc H/(1 - H^2). Reported: max |dS| to 25 GHz, the largest singular value
   of S from 1 Hz to 250 GHz (passive: <= 1) and the DC resistance the model implies.
B: lossless-line cells: N ideal lossless lines (L and C at the top of the data) with the rest of Z(s) and Y(s) as
   exact lumped elements in the middle of each cell (T/2 - z/2 - y - z/2 - T/2; "ends": y/2 z/2 - T - z/2 y/2),
   N = ceil(c * tau * fmax). The LC ladder of today's export (20 sections per wavelength) for comparison.
--ngspice: the delay element in ngspice – a 40-cell chain with lumped losses over a 20 ns PRBS transient as
   lossless T lines (default and relaxed breakpoint control) and as LTRA lines in the lossless case.
"""
import json, sys
import numpy as np
from scipy.optimize import nnls

import os, subprocess, time

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
M = json.load(open(ARGS[0] if ARGS else os.path.join(os.path.dirname(os.path.abspath(__file__)), "line_models.json")))["models"]


def rlgc(m, f):
    a = np.array(m["rows"])
    lf = np.log(a[:, 0])
    return [np.interp(np.log(f), lf, a[:, k]) for k in range(1, 5)]


def zy(m, f):
    R, L, G, C = rlgc(m, f)
    w = 2 * np.pi * f
    return R + 1j * w * L, G + 1j * w * C


def abcd_line(Z, Y, l):
    g = np.sqrt(Z * Y)
    zc = np.sqrt(Z / Y)
    ch, sh = np.cosh(g * l), np.sinh(g * l)
    return np.array([[ch, zc * sh], [sh / zc, ch]])


def mul(a, b):
    return np.einsum("ijf,jkf->ikf", a, b)


def ser(z):
    o, n = np.ones_like(z), np.zeros_like(z)
    return np.array([[o, z], [n, o]])


def sh(y):
    o, n = np.ones_like(y), np.zeros_like(y)
    return np.array([[o, n], [y, o]])


def tl(z0, td, w):
    th = w * td
    return np.array([[np.cos(th) + 0j, 1j * z0 * np.sin(th)], [1j * np.sin(th) / z0, np.cos(th) + 0j]])


def s_of(m, z0=50.0):
    a, b, c, d = m[0, 0], m[0, 1] / z0, m[1, 0] * z0, m[1, 1]
    den = a + b + c + d
    return np.array([[(a + b - c - d) / den, 2 / den], [2 / den, (-a + b - c + d) / den]])


def cells(m, f, l, n, arr="mid"):
    w = 2 * np.pi * f
    Z, Y = zy(m, f)
    fh = np.logspace(2, 11.3, 400)
    R, L, G, C = rlgc(m, fh)
    LT, CT = L.min(), C.min()
    z = (Z - 1j * w * LT) * l / n
    y = (Y - 1j * w * CT) * l / n
    zt, tc = np.sqrt(LT / CT), l / n * np.sqrt(LT * CT)
    if arr == "mid":
        cell = mul(mul(mul(mul(tl(zt, tc / 2, w), ser(z / 2)), sh(y)), ser(z / 2)), tl(zt, tc / 2, w))
    else:  # loss at the cell ends
        cell = mul(mul(mul(mul(sh(y / 2), ser(z / 2)), tl(zt, tc, w)), ser(z / 2)), sh(y / 2))
    t = cell
    for _ in range(n - 1):
        t = mul(t, cell)
    return t


def ladder(m, f, l, n):
    Z, Y = zy(m, f)
    z, y = Z * l / n, Y * l / n
    sec = mul(ser(z), sh(y))
    t = sh(y / 2)
    for _ in range(n - 1):
        t = mul(t, sec)
    return mul(mul(t, ser(z)), sh(y / 2))


# --- vector fitting (scalar, relaxed, real + complex poles) -------------------------------------
def vf(s, h, n, it=10, d=True, w=None, f_lo=None, f_hi=None):
    f_lo = f_lo or abs(s[0]) / 2 / np.pi
    f_hi = f_hi or abs(s[-1]) / 2 / np.pi
    beta = np.logspace(np.log10(f_lo), np.log10(f_hi), n // 2) * 2 * np.pi
    p = np.concatenate([[-b / 100 + 1j * b, -b / 100 - 1j * b] for b in beta])
    if n % 2:
        p = np.concatenate([p, [-f_hi * 2 * np.pi]])
    w = np.ones(len(s)) if w is None else w
    for _ in range(it):
        A = basis(s, p)
        nb = A.shape[1]
        cols = [A] + ([np.ones((len(s), 1))] if d else []) + [-h[:, None] * A, -h[:, None]]
        X = np.hstack(cols) * w[:, None]
        Xr = np.vstack([X.real, X.imag])
        # relaxation row: sum of Re(sigma) = len(s)
        rel = np.zeros(Xr.shape[1])
        sig = np.hstack([A, np.ones((len(s), 1))])
        rel[-nb - 1:] = np.sum(sig.real, axis=0)
        Xr = np.vstack([Xr, rel * np.linalg.norm(h * w) / len(s)])
        b = np.zeros(Xr.shape[0])
        b[-1] = np.linalg.norm(h * w)
        x = np.linalg.lstsq(Xr, b, rcond=None)[0]
        ct, dt = x[-nb - 1:-1], x[-1]
        if abs(dt) < 1e-8:
            dt = 1e-8
        # zeros of sigma: eig(Aq - bq ct / dt)
        Aq, bq = statespace(p)
        z = np.linalg.eigvals(Aq - np.outer(bq, ct) / dt)
        z = np.where(z.real > 0, -z.real + 1j * z.imag, z)
        p = sort_poles(z)
    A = basis(s, p)
    cols = [A] + ([np.ones((len(s), 1))] if d else [])
    X = np.hstack(cols) * w[:, None]
    x = np.linalg.lstsq(np.vstack([X.real, X.imag]), np.concatenate([(h * w).real, (h * w).imag]), rcond=None)[0]
    return p, x


def sort_poles(z):
    re = z[np.abs(z.imag) < 1e-9 * np.abs(z)].real
    cp = z[z.imag > 1e-9 * np.abs(z)]
    return np.concatenate([re + 0j] + [[c, np.conj(c)] for c in cp])


def basis(s, p):
    cols, k = [], 0
    while k < len(p):
        if abs(p[k].imag) < 1e-12 * abs(p[k]) + 1e-300:
            cols.append(1 / (s - p[k]))
            k += 1
        else:
            a, b = 1 / (s - p[k]), 1 / (s - np.conj(p[k]))
            cols += [a + b, 1j * a - 1j * b]
            k += 2
    return np.array(cols).T


def statespace(p):
    n = len(p)
    A, b = np.zeros((n, n)), np.zeros(n)
    k = 0
    while k < n:
        if abs(p[k].imag) < 1e-12 * abs(p[k]) + 1e-300:
            A[k, k], b[k] = p[k].real, 1
            k += 1
        else:
            A[k:k + 2, k:k + 2] = [[p[k].real, p[k].imag], [-p[k].imag, p[k].real]]
            b[k:k + 2] = [2, 0]
            k += 2
    return A, b


def evalvf(s, p, x, d=True):
    A = basis(s, p)
    return A @ x[:A.shape[1]] + (x[A.shape[1]] if d else 0)


def debye_fit(f, y, n, f_lo, f_hi, wt=None):
    """Y ~ G0 + sum k_i s/(s+q_i) (+ s C) with non-negative coefficients (passive RC admittance)."""
    s = 2j * np.pi * f
    q = 2 * np.pi * np.logspace(np.log10(f_lo / 10), np.log10(f_hi * 10), n)
    A = np.hstack([np.ones((len(f), 1)), s[:, None], s[:, None] / (s[:, None] + q)])
    wv = 1 / np.abs(y) if wt is None else wt
    Aw = A * wv[:, None]
    x, _ = nnls(np.vstack([Aw.real, Aw.imag]), np.concatenate([(y * wv).real, (y * wv).imag]))
    return lambda ss: x[0] + ss * x[1] + (ss[:, None] / (ss[:, None] + q)) @ x[2:], int(np.sum(x > 0))


def moc(m, l, fmax, nh, ny, f_lo):
    """Method of characteristics: (S on f, max sigma beyond the band, R_dc model vs exact, counts)."""
    fit_f = np.logspace(np.log10(f_lo), np.log10(fmax), 200)
    s = 2j * np.pi * fit_f
    Z, Y = zy(m, fit_f)
    yc = np.sqrt(Y / Z)
    g = np.sqrt(Z * Y)
    fh = np.logspace(2, 11.3, 400)
    R, L, G, C = rlgc(m, fh)
    tau = l * np.sqrt(L.min() * C.min())
    H = np.exp(-g * l)
    hm = H * np.exp(s * tau)
    ycf, nyk = debye_fit(fit_f, yc, ny, f_lo, fmax)
    p, x = vf(s, hm, nh, d=False)
    return ycf, (p, x), tau, nyk


def moc_s(ycf, hfit, tau, f, z0=50.0):
    s = 2j * np.pi * f
    yc = ycf(s)
    H = evalvf(s, *hfit, d=False) * np.exp(-s * tau)
    y11 = yc * (1 + H ** 2) / (1 - H ** 2)
    y12 = -2 * yc * H / (1 - H ** 2)
    Yn = np.array([[y11, y12], [y12, y11]]) * z0
    I = np.eye(2)[:, :, None] * np.ones(len(f))
    S = np.einsum("ijf,jkf->ikf", I - Yn, np.linalg.inv((I + Yn).transpose(2, 0, 1)).transpose(1, 2, 0))
    return S


def sig_max(S):
    return np.linalg.svd(S.transpose(2, 0, 1), compute_uv=False).max(axis=1)


def ngspice_delay_element(out):
    """Time a 40-cell chain (11 ps cells, lumped R, R||L, R-C) over a 20 ns PRBS transient in ngspice."""
    rng = np.random.default_rng(1)
    bits = rng.random(20) < 0.5
    ui, tr = 1e-9, 300e-12 / 0.8
    pts, prev = [(0, -0.2)], False
    for i, b in enumerate(bits):
        if b != prev:
            t = (i + 1) * ui
            v0, v1 = (-0.2, 0.2) if b else (0.2, -0.2)
            pts += [(t - tr / 2, v0), (t + tr / 2, v1)]
            prev = b
    pwl = " ".join(f"{t:.6e} {v:.6e}" for t, v in pts)
    res = {}
    for name, el in (("T (default breakpoint control)", "T{k} {a} 0 {b} 0 Z0=50 TD=11p"), ("T REL=100 ABS=100", "T{k} {a} 0 {b} 0 Z0=50 TD=11p REL=100 ABS=100"),
                     ("LTRA lossless (O)", "O{k} {a} 0 {b} 0 LC")):
        L = ["* delay element test", f"V1 e 0 PWL({pwl})", "R1 e a 5000", ".model LC LTRA(R=0 L=330n G=0 C=132p LEN=0.002)"]
        n = "a"
        for k in range(40):
            L += [el.format(k=k, a=n, b=f"m{k}"), f"Ra{k} m{k} x{k} 0.02", f"Rb{k} x{k} c{k} 0.05", f"Lb{k} x{k} c{k} 1e-12", f"Rc{k} c{k} y{k} 2000", f"Cc{k} y{k} 0 1e-15"]
            n = f"c{k}"
        L += [f"Rload {n} 0 50", ".tran 3e-11 2e-8", ".control", "run", f"wrdata delay_test.txt v({n})", ".endc", ".end"]
        deck = os.path.join(out, "delay_test.cir")
        open(deck, "w").write("\n".join(L) + "\n")
        txt = os.path.join(out, "delay_test.txt")
        if os.path.exists(txt):
            os.remove(txt)
        t0 = time.time()
        try:
            subprocess.run(["ngspice", "-b", deck], cwd=out, capture_output=True, timeout=120)
            dt = time.time() - t0
            a = np.loadtxt(txt)
            res[name] = f"{dt:.2f} s, {len(a)} time points, max |v| {np.abs(a[:, 1]).max():.3g} V (expected about 2 mV)"
        except subprocess.TimeoutExpired:
            res[name] = "not finished after 120 s"
    return res


def main():
    fmax = 25e9
    f = np.arange(1, 501) * fmax / 500
    print("B: lossless-line cells with exact lumped losses: max |dS| to 25 GHz (50 ohm), cells in the middle / at the ends;")
    print("   the LC ladder of today's export (20 sections per wavelength) for comparison")
    for m in M:
        for l in (0.01, 0.07):
            Z, Y = zy(m, f)
            ex = s_of(abcd_line(Z, Y, l))
            fh = np.logspace(2, 11.3, 400)
            R, L, G, C = rlgc(m, fh)
            tau = l * np.sqrt(L.min() * C.min())
            row = []
            for c in (1, 2, 3, 4):
                n = max(1, int(np.ceil(c * tau * fmax)))
                e = np.abs(s_of(cells(m, f, l, n)) - ex).max()
                e2 = np.abs(s_of(cells(m, f, l, n, "ends")) - ex).max()
                row.append(f"c={c} (N={n}): {e:.4f} / {e2:.4f}")
            nl = int(np.ceil(20 * tau * fmax))
            el = np.abs(s_of(ladder(m, f, l, nl)) - ex).max()
            print(f"  {m['name']}, {l*1e3:.0f} mm ({tau*1e12:.0f} ps): " + "; ".join(row) + f"; ladder (N={nl}): {el:.4f}")
    print("A: method of characteristics: Yc in passive RC form (16 poles), Hm by vector fitting; fitted from f_lo to 25 GHz")
    for m in M:
        for l in (0.01, 0.07):
            Z, Y = zy(m, f)
            ex = s_of(abcd_line(Z, Y, l))
            for nh in (6, 10):
                for f_lo in (1e6, 1e3):
                    ycf, hf, tau, nyk = moc(m, l, fmax, nh, 16, f_lo)
                    e = np.abs(moc_s(ycf, hf, tau, f) - ex).max()
                    fd = np.logspace(0, np.log10(2.5e11), 4000)
                    sm = sig_max(moc_s(ycf, hf, tau, fd))
                    s0 = np.array([2j * np.pi * 1e-3])
                    yc0, h0 = ycf(s0)[0], evalvf(s0, *hf, d=False)[0]
                    rdc = (2 / (yc0 * (1 + h0) / (1 - h0))).real
                    R0 = rlgc(m, np.array([1e2]))[0][0] * l
                    print(f"  {m['name']}, {l*1e3:.0f} mm, Hm {nh} poles, Yc {nyk} terms, from {f_lo:.0e} Hz: max |dS| {e:.4f}, "
                          f"largest singular value {sm.max():.4g} at {fd[sm.argmax()]:.3g} Hz, R_dc {rdc:.3g} ohm (line: {R0:.3g} ohm)")
    if "--ngspice" in sys.argv:
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            print("ngspice: 40 cells with lumped losses, 20 ns PRBS transient (30 ps step)")
            for k, v in ngspice_delay_element(d).items():
                print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
