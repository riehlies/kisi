#!/usr/bin/env python3
"""Small LTspice test decks to find where LTspice and ngspice part on a pair's bench.

Usage: ltspice_microtests.py <lane folder of a pair export> <out folder>

From the lane's own files (bench_ltspice.cir for the ends, <lane>_ltspice.lib for the elements) it writes
decks of a few seconds each, all with the bench's ends (the profile's transmitter and receiver per line) and a
single step at 4 ns (P up, N down, the bench's emf and edge):
  1_ends.cir         the ends joined by ideal 0 V connections
  2_coupled.cir      the ends and the lane's first coupled block (mode converters, even and odd chains)
  3_viapair.cir      the ends and the lane's first via pair block
  4_line_p.cir       the ends and one line section on P (N joined ideally)
  5_lane.cir         the ends and the whole lane, with internal node voltages and the source currents
  5c_lane_trap.cir   as 5 with .options method=trap (LTspice's default integration is modified trapezoidal,
                     ngspice's trapezoidal)
These decks found the 1 % offset of the LVDS pair in LTspice: LTspice's implicit parallel damping of inductors
(now switched off with Rpar=1e30 in the LTspice library); the integration method made 0.2 mV.
Each measures the received differential voltage V(vrx) before the step (the DC operating point, at 3 ns) and at
4.5–20 ns, the transmitter's current, and in 5 a few internal nodes. ngspice runs the same decks (the shared
library, the LTspice-only options left out, `xlane:n4` written `xlane.n4`); expected.json holds its values.
"""
import json, os, re, shutil, subprocess, sys

# the step's middle at 4 ns (it starts 0.19 ns earlier for a 300 ps edge); the operating point at 3 ns
TIMES = [("dc", 3e-9), ("t4p5", 4.5e-9), ("t5", 5e-9), ("t6", 6e-9), ("t8", 8e-9), ("t12", 12e-9), ("t20", 19.9e-9)]


def lines_of(path):
    out = []
    for raw in open(path, encoding="ascii").read().splitlines():
        if raw.startswith("+") and out:
            out[-1] += " " + raw[1:]
        else:
            out.append(raw)
    return out


def main():
    a = sys.argv[1:]
    if len(a) < 2:
        raise SystemExit(__doc__)
    src, out = a[0], a[1]
    os.makedirs(out, exist_ok=True)
    st = [f for f in os.listdir(src) if f.endswith("_ltspice.lib")][0][: -len("_ltspice.lib")]
    bench = lines_of(os.path.join(src, "bench_ltspice.cir"))
    lib_lt = open(os.path.join(src, f"{st}_ltspice.lib"), encoding="ascii").read()
    shutil.copy(os.path.join(src, f"{st}_ltspice.lib"), out)
    shutil.copy(os.path.join(src, f"{st}.lib"), out)
    # the ends and the emf from the bench
    emf = abs(float(re.search(r"^VTX_p \S+ 0 PWL\(\S+ (\S+)", "\n".join(bench), re.M).group(1)))
    rise = float(re.search(r"edge (\S+) ps", "\n".join(bench)).group(1)) * 1e-12
    ends = [l for l in bench if re.match(r"^(RTX|CTX|LTX|VTXS|VTXL|LRX|VRXL|CRX|RRX)_[pn] ", l)]
    lane = [l for l in bench if l.startswith("XLANE ")][0]
    subs = re.findall(r"^\.subckt (\S+) (.*)$", lib_lt, re.M)
    lane_name = lane.split()[-1]
    body = re.search(r"^\.subckt %s .*?\n(.*?)^\.ends" % re.escape(lane_name), lib_lt, re.M | re.S).group(1).splitlines()
    first = lambda pat: next(l.split()[-1] for l in body if re.search(pat, l.split()[-1]))
    cpl, vp = first(r"_CPL\d+$"), first(r"_VIAPAIR\d+")
    tl_p = next(l.split()[-1] for l in body if len(l.split()) == 5 and l.split()[1] != "an")
    t0, tr = 4e-9, rise / 0.8
    srcs = [f"VTX_p txe_p 0 PWL(0 {-emf:.6e} {t0 - tr / 2:.6e} {-emf:.6e} {t0 + tr / 2:.6e} {emf:.6e})",
            f"VTX_n txe_n 0 PWL(0 {emf:.6e} {t0 - tr / 2:.6e} {emf:.6e} {t0 + tr / 2:.6e} {-emf:.6e})"]
    dut = {
        "1_ends": ["VC_p ap bp 0", "VC_n an bn 0"],
        "2_coupled": [f"XB ap an bp bn 0 {cpl}"],
        "3_viapair": [f"XB ap an bp bn 0 {vp}"],
        "4_line_p": [f"XB ap bp 0 {tl_p}", "VC_n an bn 0"],
        "5_lane": [lane],
        "5c_lane_trap": [lane],
    }
    internal = ["n4", "n7", "n27", "n29"]
    results = {}
    for name, d in dut.items():
        lib = f"{st}_ltspice.lib"
        opts = ".options plotwinsize=0 numdgt=16 reject_number_tails" + (" method=trap" if "trap" in name else "")
        txt = [f"* kisi LTspice micro-test {name} of lane {st}: the bench's ends, {' '.join(x.split()[-1] for x in d if x.startswith('X')) or 'ideal connections'}",
               f'.include "{lib}"'] + srcs + ends + d + ["Bvrx vrx 0 V=V(rxd_p)-V(rxd_n)", ".tran 1e-11 2e-8 0 1e-11", ".save V(vrx) I(VTX_p)" + ("".join(f" V(xlane:{n})" for n in internal) + " V(rxd_p) V(rxd_n)" if name.startswith("5") else ""), opts]
        for k, t in TIMES:
            txt.append(f".meas TRAN v_{k} FIND V(vrx) AT={t:.6e}")
        txt += [f".meas TRAN i_tx_dc FIND I(VTX_p) AT={TIMES[0][1]:.6e}", f".meas TRAN i_tx_t20 FIND I(VTX_p) AT={TIMES[-1][1]:.6e}"]
        if name.startswith("5"):
            for n in internal:
                for k, t in (TIMES[0], TIMES[-1]):
                    txt.append(f".meas TRAN v_{n}_{k} FIND V(xlane:{n}) AT={t:.6e}")
            txt += [f".meas TRAN v_rxp_dc FIND V(rxd_p) AT={TIMES[0][1]:.6e}", f".meas TRAN v_rxn_dc FIND V(rxd_n) AT={TIMES[0][1]:.6e}"]
        txt.append(".end")
        open(os.path.join(out, f"{name}.cir"), "w").write("\n".join(txt) + "\n")
        if name == "5c_lane_trap":
            continue  # LTspice-only variant; ngspice's values are those of 5
        # ngspice: the shared library, no LTspice options, subcircuit paths with '.'
        ng = [l for l in txt if not l.startswith(".options")]
        ng = [l.replace(lib, f"{st}.lib") for l in ng]
        ng = [re.sub(r"V\(xlane:(\w+)\)", r"V(xlane.\1)", l) for l in ng]
        p = os.path.join(out, f"_ngspice_{name}.cir")
        open(p, "w").write("\n".join(ng) + "\n")
        r = subprocess.run(["ngspice", "-b", os.path.basename(p)], cwd=out, capture_output=True, text=True, timeout=3600)
        m = {k.lower(): float(v) for k, v in re.findall(r"^(\w+)\s*=\s*([-+0-9.eE]+)", r.stdout, re.M)}
        names = re.findall(r"^\.meas TRAN (\w+)", "\n".join(txt), re.M)
        results[name] = {n: m.get(n.lower()) for n in names}
        os.remove(p)
        print(name, "ok" if r.returncode == 0 else "FAILED", {k: v for k, v in results[name].items() if k in ("v_dc", "v_t20")})
    info = {"lane": st, "emf_v": emf, "edge_s": rise, "blocks": {"coupled": cpl, "viapair": vp, "line_p": tl_p},
            "simulator": "ngspice 42 (the same decks with the shared library; for 5c the values of 5: ngspice integrates trapezoidally)",
            "units": "V and A; v_* the received differential voltage V(vrx), v_dc before the step (operating point), i_tx_* the current of VTX_p",
            "expected": results}
    results["5c_lane_trap"] = results.get("5_lane")
    json.dump(info, open(os.path.join(out, "expected.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
