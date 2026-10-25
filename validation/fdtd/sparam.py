"""S-parameter helpers for the 3D references: full matrices from kisi-fdtd / openEMS results,
Touchstone files, 2×thru de-embedding, mixed-mode conversion and TDR.

S arrays have the shape (nf, N, N), S[k, i, j] = b_i / a_j at freq[k].
"""
import math
import numpy as np


# --- results -----------------------------------------------------------------------------
def from_kisi(res, nports=None, mirror=None):
    """Full matrix from a kisi-fdtd result (one run per excited port). `mirror`: port
    permutation of a symmetric structure (e.g. [1, 0] for a symmetric 2-port) to fill the columns
    of ports that were not excited."""
    f = np.array(res["freq"])
    n = nports or len(res["runs"][0]["s"])
    S = np.full((len(f), n, n), np.nan, complex)
    for run in res["runs"]:
        j = run["excited"]
        for i, row in enumerate(run["s"]):
            S[:, i, j] = [complex(*x) for x in row]
    if mirror is not None:
        for j in range(n):
            if np.isnan(S[0, 0, j].real):
                jm = mirror[j]
                for i in range(n):
                    S[:, i, j] = S[:, mirror[i], jm]
    return f, S


def from_openems(results):
    """Full matrix from openems_case.py outputs (one per excitation, or one with "runs")."""
    runs = []
    for r in results:
        runs += r["runs"] if "runs" in r else [r]
    f = np.array(runs[0]["freq"])
    n = len(runs[0]["s"])
    S = np.full((len(f), n, n), np.nan, complex)
    for r in runs:
        for i, row in enumerate(r["s"]):
            S[:, i, r["excited"]] = [complex(*x) for x in row]
    return f, S


# --- Touchstone ----------------------------------------------------------------------------
def write_touchstone(path, f, S, comments=(), z0=50.0):
    n = S.shape[1]
    with open(path, "w") as fh:
        for c in comments:
            fh.write(f"! {c}\n")
        fh.write(f"# Hz S RI R {z0:g}\n")
        for k in range(len(f)):
            if n == 2:  # 2-port order: S11 S21 S12 S22
                vals = [S[k, 0, 0], S[k, 1, 0], S[k, 0, 1], S[k, 1, 1]]
                fh.write(f"{f[k]:.6e} " + " ".join(f"{v.real:.6e} {v.imag:.6e}" for v in vals) + "\n")
            else:
                for i in range(n):
                    row = [S[k, i, j] for j in range(n)]
                    lines = [row[q:q + 4] for q in range(0, n, 4)]
                    for li, part in enumerate(lines):
                        head = f"{f[k]:.6e} " if (i == 0 and li == 0) else "  "
                        fh.write(head + " ".join(f"{v.real:.6e} {v.imag:.6e}" for v in part) + "\n")


def s2z(S, z0):
    """Z = z0 (I + S)(I − S)⁻¹"""
    n = S.shape[1]
    I = np.eye(n)
    return z0 * (I + S) @ np.linalg.inv(I - S)


def read_touchstone(path):
    n = int(path.rsplit(".s", 1)[1].rstrip("pP"))
    nums, unit, fmt, z0 = [], 1.0, "MA", 50.0
    for line in open(path):
        line = line.split("!")[0].strip()
        if not line:
            continue
        if line.startswith("#"):
            t = line[1:].upper().split()
            unit = {"HZ": 1, "KHZ": 1e3, "MHZ": 1e6, "GHZ": 1e9}[t[0]]
            fmt = t[2]
            if "R" in t:
                z0 = float(t[t.index("R") + 1])
            continue
        nums += [float(x) for x in line.split()]
    per = 1 + 2 * n * n
    a = np.array(nums).reshape(-1, per)
    f = a[:, 0] * unit
    v = a[:, 1::2] + 0j
    w = a[:, 2::2]
    if fmt == "RI":
        c = v.real + 1j * w
    elif fmt == "MA":
        c = v.real * np.exp(1j * np.radians(w))
    else:
        c = 10 ** (v.real / 20) * np.exp(1j * np.radians(w))
    S = c.reshape(-1, n, n)
    if n == 2:
        S = S.transpose(0, 2, 1)  # file order S11 S21 S12 S22
    read_touchstone.z0 = z0
    return f, S


# --- de-embedding --------------------------------------------------------------------------
def _s2t(s):
    s11, s12, s21, s22 = s[..., 0, 0], s[..., 0, 1], s[..., 1, 0], s[..., 1, 1]
    t = np.empty_like(s)
    t[..., 0, 0] = -(s11 * s22 - s12 * s21) / s21
    t[..., 0, 1] = s11 / s21
    t[..., 1, 0] = -s22 / s21
    t[..., 1, 1] = 1 / s21
    return t


def _t2s(t):
    t11, t12, t21, t22 = t[..., 0, 0], t[..., 0, 1], t[..., 1, 0], t[..., 1, 1]
    s = np.empty_like(t)
    s[..., 0, 0] = t12 / t22
    s[..., 0, 1] = (t11 * t22 - t12 * t21) / t22
    s[..., 1, 0] = 1 / t22
    s[..., 1, 1] = -t21 / t22
    return s


def _sqrt_cont(x):
    """square root with a continuous phase starting near the positive real axis"""
    ph = np.unwrap(np.angle(x))
    return np.sqrt(np.abs(x)) * np.exp(0.5j * ph)


def fixture_2xthru(f, T):
    """Half of a symmetric 2×thru (IEEE P370 NZC style): s11 of the fixture from the first half of
    the 2×thru reflection impulse response, then s22 and s21 from the cascade
    T11 = s11 + s21² s22 / (1 − s22²), T21 = s21² / (1 − s22²). Returns the fixture S (port 1 =
    outer port, port 2 = towards the DUT). Needs a uniform grid f_k = k·Δf."""
    s11t, s21t = T[:, 0, 0], T[:, 1, 0]
    nf = len(f)
    df = f[1] - f[0]
    dc11 = s11t[0].real - (s11t[1].real - s11t[0].real)  # linear extrapolation of Re to DC
    spec = np.concatenate([[dc11], s11t])
    h = np.fft.irfft(spec, 2 * nf)
    dt = 1 / (2 * nf * df)
    # thru delay from the phase of S21 (low half of the band)
    ph = np.unwrap(np.angle(s21t))
    sel = slice(0, max(3, nf // 2))
    tau = -np.polyfit(2 * np.pi * f[sel], ph[sel], 1)[0]
    th = tau / 2
    t = np.arange(2 * nf) * dt
    w = np.where(t < th - dt / 2, 1.0, np.where(t < th + dt / 2, 0.5, 0.0))
    w[nf:] = 0  # negative times
    s11 = np.fft.rfft(h * w)[1:nf + 1]
    s22 = (s11t - s11) / s21t
    s21sq = s21t * (1 - s22 ** 2)
    s21 = _sqrt_cont(s21sq)
    F = np.empty((nf, 2, 2), complex)
    F[:, 0, 0], F[:, 0, 1], F[:, 1, 0], F[:, 1, 1] = s11, s21, s21, s22
    return F, tau


def fixture_shift(f, T):
    """First-order fixture: half the 2×thru propagation (delay and loss), no reflection."""
    h = _sqrt_cont(T[:, 1, 0])
    F = np.zeros((len(f), 2, 2), complex)
    F[:, 0, 1] = F[:, 1, 0] = h
    return F


def deembed2(S, FL, FR):
    """2-port DUT from S = FL · DUT · mirror(FR); FL, FR: fixture S with port 1 outside."""
    FRm = FR[:, ::-1, ::-1]  # mirrored: port 1 towards the DUT
    tL, tR, tS = _s2t(FL), _s2t(FRm), _s2t(S)
    tD = np.linalg.inv(tL) @ tS @ np.linalg.inv(tR)
    return _t2s(tD)


def renorm(S, z_old, z_new):
    """2-port S from per-port references z_old to z_new (each a scalar, a pair, or an (nf, 2)
    array; complex allowed). Voltage waves a = (V + Zr I)/2, b = (V − Zr I)/2: S = (Z − R)(Z + R)⁻¹.
    T-matrix cascading stays valid where both sides of a plane use the same reference; with real,
    equal references it is the usual S."""
    nf = S.shape[0]
    zo = np.broadcast_to(np.asarray(z_old, complex), (nf, 2)) if np.ndim(z_old) < 2 else np.asarray(z_old, complex)
    zn = np.broadcast_to(np.asarray(z_new, complex), (nf, 2)) if np.ndim(z_new) < 2 else np.asarray(z_new, complex)
    I = np.eye(2)
    Ro = zo[:, :, None] * I
    Rn = zn[:, :, None] * I
    Z = np.linalg.inv(I - S) @ (I + S) @ Ro
    return (Z - Rn) @ np.linalg.inv(Z + Rn)


def line_impedance(f, T, z0=50.0, band=(2e9, 30e9)):
    """Characteristic impedance Zc(f) of the line of a symmetric 2×thru (lumped ports at both ends):
    the image impedance sqrt(B/C) of the thru, fitted with Zc² = Z∞² · jω / (jω + g) (lossless
    metal, a dielectric conductivity fixed in frequency: g = G/C). The fit avoids the frequencies
    where the thru is a multiple of half a wavelength (B, C → 0). Returns Zc(f), Z∞, g."""
    s11, s12, s21, s22 = T[:, 0, 0], T[:, 0, 1], T[:, 1, 0], T[:, 1, 1]
    B = z0 * ((1 + s11) * (1 + s22) - s12 * s21) / (2 * s21)
    C = ((1 - s11) * (1 - s22) - s12 * s21) / (2 * z0 * s21)
    y = C / B  # 1 / Zc² = 1/Z∞² − j g / (ω Z∞²)
    w = 2 * np.pi * f
    sel = (f >= band[0]) & (f <= band[1])
    a = float(np.median(y.real[sel]))
    b = float(np.median(-(w * y.imag)[sel]))
    zinf = 1 / math.sqrt(a)
    g = b / a
    return zinf * np.sqrt(1j * w / (1j * w + g)), zinf, g


def deembed_matched(f, S, T_in, T_out, z0=50.0):
    """2×thru de-embedding with the outer ports first referred to each line's own impedance, so
    that each 2×thru is nearly matched and the NZC split only has to place the small port
    parasitics; the DUT then comes out referred to the line impedances at its planes and is
    converted to z0 at the end (the via between ideal z0 lines, as kisi's ladder gives it).
    With z0-referred NZC (fixture_2xthru on the raw data) a line of Zc ≠ z0 leaves an impedance
    step of (Zc − z0)/(Zc + z0) in the fixture model at each plane. Returns D, (Zc_in, Zc_out)."""
    zi, zinf_i, _ = line_impedance(f, T_in, z0)
    zo, zinf_o, _ = line_impedance(f, T_out, z0)
    Ti = renorm(T_in, z0, np.stack([zi, zi], 1))
    To = renorm(T_out, z0, np.stack([zo, zo], 1))
    Sc = renorm(S, z0, np.stack([zi, zo], 1))
    FL, _ = fixture_2xthru(f, Ti)
    FR, _ = fixture_2xthru(f, To)
    Dc = deembed2(Sc, FL, FR)
    return renorm(Dc, np.stack([zi, zo], 1), z0), (zinf_i, zinf_o)


def deembed_shift(S, F_per_port):
    """N-port plane shift: S'_ij = S_ij / (h_i h_j) with h the fixture transmission per port."""
    out = S.copy()
    n = S.shape[1]
    for i in range(n):
        for j in range(n):
            out[:, i, j] = S[:, i, j] / (F_per_port[i][:, 1, 0] * F_per_port[j][:, 1, 0])
    return out


# --- mixed mode and TDR --------------------------------------------------------------------
def mixed_mode(S):
    """4-port with 1 = P near, 2 = N near, 3 = P far, 4 = N far → dict of 2×2 blocks dd, cc, dc, cd
    (port 1 near, 2 far)."""
    M = np.array([[1, -1, 0, 0], [0, 0, 1, -1], [1, 1, 0, 0], [0, 0, 1, 1]]) / math.sqrt(2)
    Smm = M @ S @ M.T
    return {"dd": Smm[:, :2, :2], "dc": Smm[:, :2, 2:], "cd": Smm[:, 2:, :2], "cc": Smm[:, 2:, 2:]}


def step_response(f, s, rise=25e-12, tmax=None):
    """response to a unit step with a Gaussian edge (10–90 % = `rise`) of a transfer function
    sampled on a uniform grid f_k = k·Δf (DC from the real part of the first point). t = 0 is the
    middle of the edge; the response is integrated from 3·rise before it, as kisi's own TDR
    (`rf::tdr`). (Before Block 5 the sum started at the middle of the edge: the half of a
    reflection at the reference plane that lies before t = 0 was lost and its integral stayed as a
    constant offset – a 60 fF shunt capacitor at the reference plane read as a 56 Ω plateau.)"""
    nf = len(f)
    df = f[1] - f[0]
    sig = rise / 2.563
    t0 = 3.0 * rise
    g = np.exp(-0.5 * (2 * np.pi * f * sig) ** 2) * np.exp(-2j * np.pi * f * t0)
    spec = np.concatenate([[s[0].real], s * g])
    n = 8 * nf
    v = np.cumsum(np.fft.irfft(spec, n))
    t = np.arange(n) / (n * df) - t0
    k = (t >= 0) & ((t <= tmax) if tmax else (t < n / (2 * n * df) - t0))
    return t[k], v[k]


def tdr(f, s11, rise=25e-12, z0=50.0, tmax=None):
    """step-response TDR from S11: t, Z(t)"""
    t, rho = step_response(f, s11, rise, tmax)
    return t, z0 * (1 + rho) / (1 - rho)


def line_params(f, S20, S10=None, l20=20.0, l10=10.0):
    """Zc and εeff of a uniform line from its 2-port (50 Ω ports, ABCD of the 20 mm line); γ also
    from the line–line method (T20 · T10⁻¹ is a line of l20 − l10, port effects cancel)"""
    s11, s12, s21, s22 = S20[:, 0, 0], S20[:, 0, 1], S20[:, 1, 0], S20[:, 1, 1]
    B = 50 * ((1 + s11) * (1 + s22) - s12 * s21) / (2 * s21)
    C = ((1 - s11) * (1 - s22) - s12 * s21) / (2 * 50 * s21)
    zc = np.sqrt(B / C)
    zc = np.where(zc.real < 0, -zc, zc)
    w = 2 * np.pi * f
    out = {"zc": zc.real}

    def beta_unwrap(ph, length):
        return np.unwrap(ph) / (length * 1e-3)
    if S10 is not None:
        T20, T10 = _s2t(S20), _s2t(S10)
        M = T20 @ np.linalg.inv(T10)
        ev = np.linalg.eigvals(M)
        # e^{−γΔl}: the eigenvalue with |λ| < 1 (or the one with negative phase slope)
        lam = np.where(np.abs(ev[:, 0]) < np.abs(ev[:, 1]), ev[:, 0], ev[:, 1])
        dl = l20 - l10
        beta = -beta_unwrap(np.angle(lam), dl)
        alpha = -np.log(np.abs(lam)) / (dl * 1e-3)
        out["source"] = "line–line (10 / 20 mm)"
    else:
        beta = -beta_unwrap(np.angle(s21), l20)
        alpha = -np.log(np.abs(s21)) / (l20 * 1e-3)
        out["source"] = "S21 of the 20 mm line"
    eeff = (beta * 299792458.0 / w) ** 2
    out["eeff"] = eeff
    out["loss_db_per_mm"] = alpha * 8.686e-3
    return out


def db(x):
    return 20 * np.log10(np.maximum(np.abs(x), 1e-12))
