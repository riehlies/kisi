#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Write synthetic_full.ibs: an IBIS file from analytic buffers, so that every value is known.

Drivers are linear (I = V/Ron per corner), clamps are ideal (no current between 0.6 V below
ground and 0.6 V above the supply, 5 ohm beyond), edges are raised cosines into the test
fixtures. Waveform end points are the exact DC solutions, [Ramp] is measured from the
waveforms the way kisi measures it (linear interpolation, 20-80 %). The file covers every
keyword kisi-ibis reads, several it only reports, typ/min/max with NA, all scaling factors,
keyword spellings with spaces/underscores and mixed case, a changed comment character,
multi-line text and matrix rows continued over lines.

Usage: python3 make_synthetic.py [outdir]   (default: the directory of this script)
"""
import math
import os
import sys

CORNERS = ("typ", "min", "max")
VCC = {"typ": 3.3, "min": 3.0, "max": 3.6}
T_EDGE = {"typ": 1.0e-9, "min": 1.4e-9, "max": 0.7e-9}  # raised-cosine duration
R_FIX = 50.0


def g(x):
    """Number with up to 6 significant digits, no exponent noise."""
    if x == 0:
        return "0"
    return f"{x:.6g}"


def eng(x, unit=""):
    """Engineering notation with an IBIS scaling factor."""
    if x == 0:
        return "0" + unit
    for f, s in ((1e9, "G"), (1e6, "M"), (1e3, "k"), (1, ""), (1e-3, "m"), (1e-6, "u"), (1e-9, "n"), (1e-12, "p"), (1e-15, "f")):
        if abs(x) >= f * 0.9999:
            return f"{x / f:.6g}{s}{unit}"
    return f"{x:.6g}{unit}"


def iv_x():
    # -Vcc(max) ... 2 Vcc(max)
    return [round(-3.6 + 0.3 * k, 6) for k in range(37)]


def clamp_x():
    return [round(-3.6 + 0.3 * k, 6) for k in range(25)]  # -3.6 ... 3.6


def table(rows, cols=3):
    out = ["|   Voltage     I(typ)          I(min)          I(max)"]
    for x, vals in rows:
        out.append(f"   {g(x):<10}  " + "  ".join(f"{('NA' if v is None else eng(v)):<14}" for v in vals[:cols]))
    return "\n".join(out)


def driver_tables(ron):
    pd = [(x, [x / ron[c] for c in CORNERS]) for x in iv_x()]
    pu = [(x, [-x / ron[c] for c in CORNERS]) for x in iv_x()]
    return pd, pu


def clamp_tables():
    # current into the pin; min/max columns NA (typ is used for them)
    gc = [(x, [((x + 0.6) / 5.0 if x < -0.6 else 0.0), None, None]) for x in clamp_x()]
    pc = [(x, [(-(x + 0.6) / 5.0 if x < -0.6 else 0.0), None, None]) for x in clamp_x()]
    return gc, pc


def s_curve(t, t0, dur):
    if t <= t0:
        return 0.0
    if t >= t0 + dur:
        return 1.0
    return (1 - math.cos(math.pi * (t - t0) / dur)) / 2


def times():
    return [round(0.1e-9 * k, 15) for k in range(36)]  # 0 ... 3.5 ns


def levels(ron, vcc, vf, has_pu=True, has_pd=True):
    """DC pin voltage low / high into R_FIX to vf (clamps do not conduct in 0 ... vcc)."""
    low = vf * ron / (ron + R_FIX) if has_pd else vf
    high = (vcc * R_FIX + vf * ron) / (R_FIX + ron) if has_pu else vf
    return low, high


def waveform(ron, rising, fixture, has_pu=True, has_pd=True):
    """fixture: 'gnd' or 'vcc'. Returns (header lines, rows, per-corner (v0, v1))."""
    vfix = {c: (0.0 if fixture == "gnd" else VCC[c]) for c in CORNERS}
    ends = {}
    for c in CORNERS:
        lo, hi = levels(ron[c], VCC[c], vfix[c], has_pu, has_pd)
        ends[c] = (lo, hi) if rising else (hi, lo)
    rows = []
    for t in times():
        rows.append((t, [ends[c][0] + (ends[c][1] - ends[c][0]) * s_curve(t, 0.5e-9, T_EDGE[c]) for c in CORNERS]))
    head = [f"R_fixture = {g(R_FIX)}", f"V_fixture = {g(vfix['typ'])}", f"V_fixture_min = {g(vfix['min'])}", f"V_fixture_max = {g(vfix['max'])}"]
    return head, rows, ends


def wave_text(kind, head, rows):
    out = [f"[{kind}]"] + head + ["|   Time        V(typ)          V(min)          V(max)"]
    for t, vals in rows:
        out.append(f"   {eng(t):<10}  " + "  ".join(f"{g(v):<14}" for v in vals))
    return "\n".join(out)


def edge_20_80(rows, k):
    pts = [(t, v[k]) for t, v in rows]
    v0, v1 = pts[0][1], pts[-1][1]
    sw = v1 - v0

    def cross(level):
        for (ta, va), (tb, vb) in zip(pts, pts[1:]):
            hit = (va < level <= vb) if sw > 0 else (va > level >= vb)
            if hit:
                return ta + (tb - ta) * (level - va) / (vb - va)
        raise ValueError("no crossing")

    return 0.6 * abs(sw), cross(v0 + 0.8 * sw) - cross(v0 + 0.2 * sw)


def ramp_text(rise_rows, fall_rows):
    def dvdt(rows):
        out = []
        for k in range(3):
            dv, dt = edge_20_80(rows, k)
            out.append(f"{g(dv)}/{eng(dt)}")
        return "  ".join(f"{x:<16}" for x in out)

    return "\n".join(["[Ramp]", "R_load = 50", "|               typ               min               max", f"dV/dt_r         {dvdt(rise_rows)}", f"dV/dt_f         {dvdt(fall_rows)}"])


def driver_model(name, mtype, ron, extra_params="", extra_kw=""):
    pd, pu = driver_tables(ron)
    gc, pc = clamp_tables()
    waves = []
    r_gnd = r_vcc = f_gnd = f_vcc = None
    for rising in (True, False):
        for fix in ("gnd", "vcc"):
            head, rows, _ = waveform(ron, rising, fix)
            waves.append(wave_text("Rising Waveform" if rising else "Falling Waveform", head, rows))
            if rising and fix == "gnd":
                r_gnd = rows
            if not rising and fix == "vcc":
                f_vcc = rows
    return f"""
[Model]         {name}
Model_type      {mtype}
Polarity        Non-Inverting
Enable          Active-High
Vinl = 0.8V
Vinh = 2.0V
Vmeas = 1.65V
Vref = 0
Rref = 50Ohm
Cref = 0pF
{extra_params}C_comp          2.0pF           1.8pF           2.2pF
|
[Temperature Range]     25C     85C     -40C
[Voltage range]         3.3V    3.0V    3.6V
[Pulldown]
{table(pd)}
[Pullup]
{table(pu)}
[GND_clamp]
{table(gc)}
[POWER clamp]
{table(pc)}
{ramp_text(r_gnd, f_vcc)}
{chr(10).join(waves)}
{extra_kw}"""


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    ron50 = {"typ": 50.0, "min": 60.0, "max": 40.0}
    ron34 = {"typ": 34.0, "min": 41.0, "max": 28.0}
    ron25 = {"typ": 25.0, "min": 30.0, "max": 20.0}
    gc, pc = clamp_tables()

    # open drain: pulldown only, waveforms into 50 ohm to the supply
    od_pd = [(x, [x / ron25[c] for c in CORNERS]) for x in iv_x()]
    od_waves = []
    od_rows = {}
    for rising in (True, False):
        head, rows, _ = waveform(ron25, rising, "vcc", has_pu=False)
        od_rows[rising] = rows
        od_waves.append(wave_text("Rising Waveform" if rising else "Falling Waveform", head, rows))

    # receiver with on-die termination 2 x 100 ohm (straight clamp lines): 50 ohm to Vcc/2
    odt_gc = [(x, [x / 100.0, None, None]) for x in clamp_x()]
    odt_pc = [(x, [-x / 100.0, None, None]) for x in clamp_x()]

    text = f"""| synthetic_full.ibs – written by make_synthetic.py (kisi, GPL-3.0-only).
| Analytic buffers: every value is known. NOT a model of a real part.
|
[IBIS Ver]      5.1
[Comment Char]  !_char   ! from here on '!' starts a comment, '|' is an ordinary character
[File Name]     synthetic_full.ibs
[File Rev]      1.0 ! a comment with the new character
[Date]          22/10/2026
[Source]        kisi make_synthetic.py:
                linear drivers, ideal clamps, raised-cosine edges
[Notes]         Covers the keywords kisi-ibis reads.
[Notes]         A second [Notes] keyword is appended.
[Disclaimer]    Synthetic data | not a real component.
[Copyright]     Copyright 2026 kisi contributors, GPL-3.0-only
[Comment Char]  |_char   ! back to the default comment character
|
[Component]     KISI_SYNTH
[Manufacturer]  kisi (synthetic)
[Package]
|               typ         min         max
R_pkg           100m        80m         120m
L_pkg           2nH         1.5nH       2.5nH
C_pkg           0.5pF       NA          NA
|
[Pin]   signal_name     model_name      R_pin   L_pin   C_pin
1       VCC             POWER           NA      NA      NA
2       GND             GND             NA      NA      NA
3       NC              NC              NA      NA      NA
4       IO0             IO_SEL          80m     1.8nH   0.45pF
5       TX_P            IO_SEL          90m     1.9nH   0.46pF
6       TX_N            IO_SEL          91m     1.95nH  0.46pF
7       S1A             NC              NA      NA      NA
8       S1B             NC              NA      NA      NA
9       S2A             NC              NA      NA      NA
10      S2B             NC              NA      NA      NA
11      RX_ODT          IN_ODT          70m     1.2nH   0.40pF
12      OD0             OD_OUT          NA      2.1nH   NA
13      TRM             TERM            60m     1.1nH   0.35pF
14      DRV0            DRV             85m     1.7nH   0.44pF
15      IN1             IN_ODT
16      IO1             IO_50           NA      NA      NA
|
[Package Model] PKG_SYNTH
|
[Pin Mapping]   pulldown_ref    pullup_ref      gnd_clamp_ref   power_clamp_ref
1               NC              VCCBUS
2               GNDBUS          NC
4               GNDBUS          VCCBUS          GNDBUS          VCCBUS
5               GNDBUS          VCCBUS          GNDBUS          VCCBUS
6               GNDBUS          VCCBUS          GNDBUS          VCCBUS
11              GNDBUS          VCCBUS          GNDBUS          VCCBUS
|
[Diff Pin]      inv_pin     vdiff       tdelay_typ  tdelay_min  tdelay_max
5               6           100mV       0           NA          NA
|
[Series Pin Mapping]  pin_2   model_name   function_table_group
7                     8       SER
9                     10      SW           group_a
|
[Series Switch Groups]
On group_a /
|
[Model Selector] IO_SEL
IO_50           50 ohm drive (default, first entry)
IO_34           34 ohm drive
|
{driver_model("IO_50", "I/O", ron50)}
{driver_model("IO_34", "I/O", ron34, extra_kw=chr(10).join(["[Add Submodel]", "DYN_CLAMP   Non-Driving"]))}
{driver_model("DRV", "3-state", ron50, extra_kw=chr(10).join(["[Driver Schedule]   Rise_on_dly   Rise_off_dly   Fall_on_dly   Fall_off_dly", "IO_34               0.1ns         NA             0.1ns          NA"]))}
[Model]         IN_ODT
Model_type      input
Vinl = 0.8
Vinh = 2.0
C_comp          1.5p    1.4p    1.6p
C_comp_gnd_clamp  300f  NA  NA
Rref_diff = 0.1k
Cref_diff = 0
[Model Spec]
|               typ     min     max
Vinh            2.0     1.9     2.1
Vinl            0.8     0.75    0.85
[Receiver Thresholds]
Vth = 1.65V
Vth_min = 1.6V
Vth_max = 1.7V
Reference_supply Pullup_ref
[Voltage Range]         3.3     3.0     3.6
[Pullup Reference]      3.3     3.0     3.6
[Pulldown Reference]    0       0       0
[GND Clamp Reference]   0       0       0
[POWER Clamp Reference] 3.3     3.0     3.6
[GND Clamp]
{table(odt_gc)}
[POWER Clamp]
{table(odt_pc)}
|
[Model]         OD_OUT
Model_type      Open_drain
C_comp          1.2pF   1.1pF   1.3pF
[Voltage Range] 3.3     3.0     3.6
[Pulldown]
{table(od_pd)}
[GND Clamp]
{table(gc)}
[POWER Clamp]
{table(pc)}
[Ramp]
R_load = 50
dV/dt_r   {"  ".join(f"{g(edge_20_80(od_rows[True], k)[0])}/{eng(edge_20_80(od_rows[True], k)[1])}" for k in range(3))}
dV/dt_f   {"  ".join(f"{g(edge_20_80(od_rows[False], k)[0])}/{eng(edge_20_80(od_rows[False], k)[1])}" for k in range(3))}
{chr(10).join(od_waves)}
|
[Model]         TERM
Model_type      Terminator
C_comp          0.8p    NA      NA
[Voltage Range] 3.3     3.0     3.6
[Rgnd]          100     90      110
[Rpower]        1T      NA      NA
[Rac]           50      45      55
[Cac]           10p     9p      11p
|
[Model]         SER
Model_type      Series
C_comp          0p      0p      0p
[Voltage Range] 3.3     3.0     3.6
[R Series]      25      22      28
[L Series]      1n      0.8n    1.2n
[Series Current]
   -1.0     -40m     -36m     -44m
    0.0      0        0        0
    1.0      40m      36m      44m
|
[Model]         SW
Model_type      Series_switch
C_comp          0p      0p      0p
[Voltage Range] 3.3     3.0     3.6
[On]
[R Series]      5       4.5     5.5
[Off]
[R Series]      1M      0.9M    1.1M
|
[Submodel]      DYN_CLAMP
Submodel_type   Dynamic_clamp
[Submodel Spec]
Off_delay       1n      NA      NA
[POWER Clamp]
   -3.3     0.5      NA       NA
    0       0        NA       NA
|
[Define Package Model]  PKG_SYNTH
[Manufacturer]          kisi
[OEM]                   kisi (synthetic)
[Description]           three coupled pins
[Number Of Pins]        3
[Pin Numbers]
4
5
6
[Model Data]
[Inductance Matrix]     Full_matrix
[Row]   4
2.0nH   0.3nH
0.1nH
[Row]   5
2.1nH   0.4nH
[Row]   6
2.2nH
[Capacitance Matrix]    Banded_matrix
[Bandwidth]             1
[Row]   4
0.45pF  -0.05pF
[Row]   5
0.46pF  -0.06pF
[Row]   6
0.46pF
[Resistance Matrix]     Sparse_matrix
[Row]   4
4       80m
[Row]   5
5       90m
[Row]   6
6       91m
[End Model Data]
[End Package Model]
|
[END]
text after [END] is ignored
"""
    with open(os.path.join(out_dir, "synthetic_full.ibs"), "w", newline="\n") as f:
        f.write(text)


if __name__ == "__main__":
    main()
