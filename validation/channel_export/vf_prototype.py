#!/usr/bin/env python3
"""Prototype for a compact lane model (ROADMAP block 3, step 3): delay + vector-fitted loss.

Usage: vf_prototype.py <lane.s2p | lane.s4p> [--poles N] [--out DIR]

kisi's ladder export has 10–40 k elements per lane – exact enough, but long transients get slow. A compact
model separates the delay and fits the rest:
  1. the lane's S-parameters, a pair reduced to its differential mode (Sdd, reference 2·z0);
  2. the propagation delay from the phase of Sdd21 (group delay at low frequency) is taken out of the
     transmission terms: S21·e^{+jωτ}, so what remains is smooth (the loss and the reflections);
  3. scikit-rf's VectorFitting fits the 2-port with N real/complex pole pairs, with a passivity check (and
     enforcement when it is violated);
  4. the error against the original is reported, and a good fit of the whole lane (with its delay) written as a
     SPICE subcircuit by scikit-rf (`write_spice_subcircuit_s`, vf_model.sp).
Result (docs/research/channel-export.md): fitting the lane with its delay needs about two poles per ripple of the
response (delay × top frequency); taking the delay out of S21 alone does not help a reflecting lane (the delay sits
in S11 too) – a compact model needs the delay as ideal lines at the ports and the fit of what is between them, and
passivity enforcement is the hard part (scikit-rf's diverges on the 60-pole fit of a 574 ps lane).

This is a design study, not part of the export: it shows the orders and errors to expect.
"""
import os, sys
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from compare import read_touchstone, mixed, db  # noqa: E402


def main():
    a = sys.argv[1:]
    if not a:
        raise SystemExit(__doc__)
    path = a[0]
    npoles = int(a[a.index("--poles") + 1]) if "--poles" in a else 24
    out = a[a.index("--out") + 1] if "--out" in a else os.path.dirname(os.path.abspath(path))
    import skrf
    from skrf.vectorFitting import VectorFitting
    f, s, z0 = read_touchstone(path)
    if s.shape[1] == 4:
        s = mixed(s)
        z0 = 2 * z0
    w = 2 * np.pi * f
    # group delay of S21 at low frequency (phase slope over the first 5 % of the band)
    k = max(3, len(f) // 20)
    ph = np.unwrap(np.angle(s[:k, 1, 0]))
    tau = -np.polyfit(w[:k], ph, 1)[0]
    rot = np.exp(1j * w * tau)
    sd = s.copy()
    sd[:, 1, 0] *= rot
    sd[:, 0, 1] *= rot
    res = {}
    for name, data in (("with the delay", s), ("delay removed", sd)):
        nw = skrf.Network(frequency=skrf.Frequency.from_f(f, unit="hz"), s=data, z0=z0)
        vf = VectorFitting(nw)
        vf.vector_fit(n_poles_real=2, n_poles_cmplx=npoles // 2, fit_proportional=False, fit_constant=True)
        model = np.array([[vf.get_model_response(i, j, f) for j in range(2)] for i in range(2)]).transpose(2, 0, 1)
        err = np.abs(model - data).max()
        passive = vf.is_passive()
        if not passive:
            vf.passivity_enforce()
            model = np.array([[vf.get_model_response(i, j, f) for j in range(2)] for i in range(2)]).transpose(2, 0, 1)
        err2 = np.abs(model - data).max()
        back = model.copy()
        if name == "delay removed":
            back[:, 1, 0] /= rot
            back[:, 0, 1] /= rot
        d21 = np.abs(db(back[:, 1, 0]) - db(s[:, 1, 0])).max()
        res[name] = {"poles": len(vf.poles), "max_abs_err": float(err), "passive_before": bool(passive), "max_abs_err_after_enforcement": float(err2), "max_dS21_db": float(d21)}
        if name == "with the delay" and err2 < 0.05:
            vf.write_spice_subcircuit_s(os.path.join(out, "vf_model.sp"))
    print(f"{os.path.basename(path)}: delay {tau * 1e12:.1f} ps, {len(f)} points up to {f[-1] / 1e9:.1f} GHz")
    for k2, v in res.items():
        print(f"  {k2}: {v}")


if __name__ == "__main__":
    main()
