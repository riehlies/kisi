# Channel export to circuit and RF simulators – first increment and its check

Status: implemented (24 October 2026) – `kisi export`, the viewer's “Export…” in the lane detail and the bus card,
`kisi_si::export`, checked with `validation/channel_export/compare.py` (also in CI). ROADMAP block 3, steps 1 and 2;
step 3 (compact, vector-fitted models) as a design study at the end. Background and the KiCad facts:
`kicad-simulation.md` (option A of that note, with linear ends).

## Summary
- **What it does:** a lane – or every lane of a bus – becomes a SPICE subcircuit with ports at both ends (a pair:
  P and N at each end, with the coupling between them), plus ready test benches for ngspice and Xyce, an
  S-parameter deck, Touchstone files (a bus also as one N-port with its port map) and a KiCad project. The channel
  is exactly the one kisi's channel check computes for the eye (same sections, same models, same ends).
- **Netlist = kisi's circuit:** ngspice's S-parameter analysis of the exported subcircuit reproduces kisi's own
  evaluation of the circuit for every lane tried (LVDS lane, nine PCIe lanes; every S-parameter to 0.001), and the
  bus subcircuit gives the same lane response as the lane alone. The KiCad project shows the lane element by element
  (lines, coupled runs, vias, parts, pads, stubs); `kicad-cli`'s netlist of it is the lane subcircuit node for node,
  and its AC response in ngspice equals the bench's to 1e-8.
- **Circuit vs kisi's S-parameters:** up to 1.5 × Nyquist |ΔS21| ≤ 0.025 dB on the LVDS lanes and 0.15–0.42 dB on
  the strongly reflecting PCIe lanes (|Sdd11| up to −5 dB: the ripples shift with small phase errors); the largest
  complex difference of any S-parameter ≤ 0.07 in that band (0.3–0.5 up to 25 GHz, mostly above 15 GHz). Step
  responses (100 ps edge, every port terminated) agree within 1.8 % of the step, the 50 % delay within 1.4–2.3 ps
  (0.3–0.8 %).
- **Why not exact:** parts of kisi's model are not passive or not causal – the Huray roughness adds resistance
  without its internal inductance, the via models use a constant loss tangent and a barrel resistance without
  internal inductance, microstrip dispersion makes the capacitance rise with frequency, and the plane-pair spans of
  via pairs have a negative resistance from a few GHz (on ThunderScope's PCIe reference clock lane kisi's 4-port is
  not passive above 22 GHz, largest singular value 1.005). A circuit of positive R, L, C cannot follow those
  exactly; the export is passive and causal by construction and follows the loss closely and the reactance (the
  delay) more closely still. Above about 15 GHz the via-pair spans dominate the difference.
- **Sizes:** 12–25 k elements per PCIe lane, 42 k for the 85 mm LVDS pair; ngspice (one core, shared machine):
  S-parameters (500 points) in 1–66 s, a step response in 2–133 s, the PRBS7 bench (two periods) in 75–340 s.
- **Not included (open points):** IBIS (KIBIS) ends, the channel behind a connector, crosstalk between lanes (the
  bus N-port is block-diagonal), Xyce and Qucs-S runs (not available here), a compact model for long transients.

## 1. Interface
```
kisi export <board.kicad_pcb> --lane NAME | --bus NAME [--format spice,xyce,touchstone,kicad] [--out DIR]
            [--class CLASS=PROFILE] [--variant NAME] [--config FILE]
```
`NAME` is a lane as `kisi interfaces` lists it (or one of its nets, or the last part of a hierarchical name, e.g.
`SYNC_OUT_P`), or a bus name (`"P1 ↔ U18"`). The default folder is `analysis/export/<name>` next to the board. The
profile, direction, connector end models and assembly variant are the channel check's; the native CLI and
`kisi.mjs` give the same JSON and byte-identical files (`tools/compare_cli.py`). The viewer has one “Export…”
button in the lane detail and one in each bus card; it uses the profile, direction and connector ends set in the
viewer and downloads a zip of the same files. The JSON names every file, the ports, the bench ends, the element
counts and the check of the circuit against kisi's S-parameters (`check_against_kisi`).

## 2. What the files contain
Per lane (`<lane>/` in a bus export):

| File | Content |
|---|---|
| `<lane>.lib` | the lane as subcircuit `<lane>_lane a b ref` (pairs: `ap an bp bn ref`; transmitter end = a, receiver end = b); one subcircuit per line section, via, part and stub, the ladders as repeated section subcircuits. Header: ports, model, the check against kisi, element counts |
| `bench_ngspice.cir`, `bench_xyce.cir` | transient test bench: PRBS7 (two periods) at the profile's bit rate and edge, the profile's linear transmitter (emf → R → C at the die → L → pin) and receiver (pin → L → C ∥ R at the die) per line, scaled so that the receiver sees ±swing at DC; ngspice writes the receiver voltage with `wrdata`, Xyce with `.print tran format=csv` |
| `sparam_ngspice.cir`, `sparam_xyce.cir` | S-parameters on kisi's grid: ngspice `.sp` with `portnum` sources, Xyce `.ac` + `.lin` with port devices (Touchstone 2 output) |
| `<lane>.s2p` / `.s4p` | kisi's S-parameters of the same channel (Touchstone 1.0, RI), ports as above, the port list in the header |
| `<lane>.kicad_pro`, `.kicad_sch`, `<lane>_kicad.lib` | KiCad project (section 2a): one symbol per element of the lane subcircuit (or, with `--kicad-style compact`, the lane as one symbol), the PRBS7 sources as subcircuits, the ends as R, L, C symbols whose models KiCad infers from reference and value, ground and a power flag, a `.tran` directive, notes on what the ends stand for. KiCad 7 format (20230121) with kisi's own symbols embedded, so it opens without libraries. `<lane>_kicad.lib` includes `<lane>.lib` and holds the small wrapper subcircuits the symbols call: KiCad reads the models from the file a symbol names, and a short file loads at once |
| `README.md` | ports, files, bench ends, model, the check, what is not included, kisi's notes on the lane |

A bus adds `<bus>.s<N>p` (all lanes as one N-port: ports 1…N/2 at the first device, N/2+1…N at the other, port k
and k+N/2 the two ends of one line; every lane renormalised to the most common reference), `<bus>_ports.csv`,
`<bus>.lib` (subcircuit `<bus>_bus` with all lanes in the same port order) and a README with the port map. The
lanes are independent in kisi's network model, so the N-port is block-diagonal.

### 2a. The KiCad sheet
The sheet draws the lane subcircuit `<lane>_lane` element by element, in its order, with kisi's own symbols:
`TL` line sections (consecutive sections of one line on one layer as one element), `CPL` coupled runs drawn across
the P and N rows (consecutive runs on one layer as one), `VIA` and `VP` (via pair), the board's parts under their
reference designator (e.g. `C40`, a capacitor symbol with the value `220n`), `CP` pad capacitances (also those of
unfitted parts), `ST` and `SP` open stubs. A pair has two parallel rows; a long lane wraps into several bands
(P outside on the right, inside on the left, so the two lines never cross); the paper grows from A3 as needed.
The transmitter (PRBS7 source, R, C at the die) is on the left, the receiver (C ∥ R at the die) on the right; a note
on the sheet says what they stand for – profile, edge, emf, and for a high source resistance (LVDS: 5 kΩ against
50 Ω) “a current-mode driver as its Thevenin equivalent, I = emf/(R + R_rx)”.

- **Models:** each element symbol has `Sim.Device = SUBCKT`, `Sim.Library = <lane>_kicad.lib`, `Sim.Name =
  <element>_k` and `Sim.Pins`. The wrapper `<element>_k` calls the element's subcircuit in `<lane>.lib` with its
  reference node on ground (so lines, vias and coupled runs need no ground pin; pads and shunt parts keep theirs).
- **Parts with an editable value:** a part's wrapper is a copy of its subcircuit with the value as parameter, e.g.
  `.subckt …_C40_2_k a b params: c=220n` (ESR, ESL and the 1 GΩ DC path fixed); the symbol has `Sim.Params =
  c=${VALUE}`. KiCad substitutes the Value field (checked with kicad-cli 7: Value `100n` → `c=100n` in the netlist),
  so editing the value keeps the parasitics. The Value is in SPICE notation (`100n`, `4.7u`, `1Meg`); an inductor's
  parameter is its total inductance (kisi adds the ESL to the nominal value). Pads keep fixed values.
- **Drawing rules:** pins visible (names and numbers hidden), every wire ends on a pin end or another wire end, a
  junction wherever three or more items meet, everything on the 1.27 mm grid, a `PWR_FLAG` on ground (ERC: the
  ground net is driven). `validation/channel_export/kicad_sheet.py` checks this without KiCad, then kicad-cli's
  netlist (no unconnected pin, every element of `<lane>_lane` once and connected node for node as there, the ports at
  the transmitter's and the receiver's die), its AC response in ngspice against the bench (exact, no time steps) and,
  with kicad-cli 8 or later, KiCad's ERC.
![KiCad sheet of PCIe_PER0_P](../img/channel-export/kicad_pcie_per0_p.png)
![KiCad sheet of SYNC_OUT_P](../img/channel-export/kicad_sync_out_p.png)

- **Compact sheet** (`--kicad-style compact`): the lane as one symbol, as before. It stays as an option for users
  who want the lane as a black box; the detailed sheet is the default for lanes and buses (a PCIe lane is 48–55
  symbols, written in milliseconds, and opens and checks cleanly).
- **Bus:** every lane folder has its own KiCad project (simpler than hierarchical sheets and each opens on its
  own); the bus-level files stay as they are.
- **ngspice and Xyce decks:** the circuit (`<lane>.lib`) is shared; only the decks differ – ngspice writes results from a
  `.control` block (`wrdata`) and takes S-parameters from `.sp` with `portnum` sources, Xyce uses `.print` and port
  devices `P` with `.ac` + `.lin`. One deck with commented-out dialect lines would need hand editing (ngspice reads
  `P` as its coupled-line device, Xyce knows neither `portnum` nor `.control`), so each simulator has its own decks;
  the export README says so.

What the channel contains: the board between the two end pins as the eye's network uses it – lines, vias, series
parts, shunt parts, side stubs and forks, with the terminations that do not sit at the receiver. Terminations next
to the receiver act as the receiver's termination (bench and Touchstone leave them out, as kisi's eye does).

## 3. The model
- **Line sections** become RLGC ladders of π sections, at least 20 per wavelength at the top frequency (kisi's
  check frequency, 25 GHz or 1.25 × the bit rate). Per section: a Foster network R0 + sL + Σ R_k ∥ L_k for the series
  impedance R(f) + jωL(f) and a Debye network G0 + sC + Σ (R_k + 1/sC_k)⁻¹ for the shunt admittance G(f) + jωC(f),
  fitted to kisi's `LineModel::rlgc` from fmax/10⁴ to fmax with poles on a logarithmic grid and non-negative
  coefficients (non-negative least squares, `export::fit`). Microstrip gets series R–L–C branches with resonances
  above the band, so that the capacitance can rise with frequency. The fit weights the loss (R, G) at 1 and the
  reactance and susceptance at 10 to 30 (graded with frequency): a reactance error turns into a phase error along
  the line. Errors of the main sections on the example boards: R 7–8 %, X 0.7–0.9 %, G 0.7–3.2 %, B ≤ 0.3 % (the resistance gives way to the reactance where kisi's roughness has no matching inductance).
- **Vias:** the physics-based ladder (`via_pbm`) node by node – node capacitances with their dielectric loss,
  span impedances (plane-pair impedance with the return vias, barrel resistance; fitted like the lines, with parallel
  R–L–C tanks for resonances above the band), the open barrel stubs, the junction inductances and the trace over the
  antipad. The ladder runs from via centre to via centre; feed trace it adds is a lossless line inside the via,
  feed trace it removes is taken from the nearest line section (both lines of a coupled run together). Vias without
  the ladder: kisi's coaxial section stack with its stubs.
- **Parts:** R, L, C with the parasitics kisi uses (estimates from `data/components.json`); series capacitors get
  1 GΩ across them (a DC path for the simulator).
- **Stubs:** their own chain of sections, open at the end.
- **Pairs:** both lines on their own paths as `rf::mixed::align_pair` aligns them. Coupled runs, via pairs and stub
  pairs are modal blocks: the even- and the odd-mode chain between ideal mode converters – E sources set V_e =
  (V_P + V_N)/2 and V_o = (V_P − V_N)/2, F sources make I_P = I_e + I_o and I_N = I_e − I_o – exactly
  `T = P·blockdiag(T_e, T_o)·P⁻¹` of `rf::mixed::coupled`. Each coupled run (consecutive runs on one layer
  joined), via pair and stub pair is its own block – back-to-back mode converters are exact, and each block is
  one element on the KiCad sheet. This keeps
  the mode-dependent conductor loss and the even mode of the via pairs, which a K/C_m ladder would not.
- **The check inside kisi:** the circuit is evaluated element by element (cascade matrices of exactly the values
  written) and compared with kisi's cascade of the same build (`check_against_kisi`); ngspice must give the same.

## 4. Verification
`validation/channel_export/compare.py` runs `kisi export`, then ngspice on the S-parameter deck and on a step
response (100 ps edge, every port in its reference), and compares with kisi's Touchstone file and with the step
response computed from it; scikit-rf reads every Touchstone file (passivity, reciprocity). ngspice 42, scikit-rf
2.2.0, kicad-cli 7.0.11; ThunderScope Rev5.3 with `web/samples/ts.kisi.json`, formula impedances (native CLI).

| Lane | model | mm | elements R/L/C/sources | ngspice vs kisi up to 1.5×Nyquist: \|ΔS21\| dB, max \|ΔS\| | up to 25 GHz: \|ΔS21\| dB, max \|ΔS\| | kisi's evaluation: max \|ΔS\| | step: max diff, t50 ngspice / kisi (ps) | ngspice s (S-par. / step) |
|---|---|---|---|---|---|---|---|---|
| SYNC_OUT_P | pair | 85.4 | 20745/9091/11762/200 | 0.016, 0.008 | 10.73, 0.293 | 0.293 | 1.8 %, 576.6 / 574.3 | 5 / 8 |
| REFINOUT2 | single | 70.2 | 5573/2745/3101/0 | 0.024, 0.005 | 1.18, 0.136 | 0.136 | 1.7 %, 404.3 / 402.5 | 1 / 2 |
| PCIe_PER0_P | pair | 34.7 | 6092/3066/3535/170 | 0.424, 0.069 | 1.43, 0.157 | 0.157 | 1.4 %, 207.4 / 205.9 | 25 / 42 |
| PCIe_PER1_P | pair | 32.9 | 6096/3104/3516/170 | 0.383, 0.056 | 2.08, 0.179 | 0.179 | 1.5 %, 196.4 / 194.6 | 66 / 133 |
| PCIe_PER2_P | pair | 31.1 | 5815/2934/3367/170 | 0.242, 0.049 | 2.33, 0.171 | 0.171 | 1.3 %, 188.6 / 187.0 | 56 / 54 |
| PCIe_PER3_P | pair | 30.5 | 5687/2804/3360/210 | 0.401, 0.068 | 2.08, 0.140 | 0.140 | 1.3 %, 183.0 / 181.4 | 32 / 36 |
| PCIe_PET0_P | pair | 59.7 | 12521/5804/7275/190 | 0.155, 0.064 | 2.80, 0.182 | 0.182 | 1.5 %, 382.3 / 380.4 | 17 / 30 |
| PCIe_PET1_P | pair | 42.1 | 8399/4013/4930/150 | 0.149, 0.070 | 0.99, 0.219 | 0.219 | 1.3 %, 270.4 / 269.0 | 17 / 16 |
| PCIe_PET2_P | pair | 45.1 | 9239/4504/5268/70 | 0.178, 0.053 | 1.24, 0.485 | 0.485 | 1.5 %, 302.4 / 300.5 | 17 / 39 |
| PCIe_PET3_P | pair | 39.1 | 8145/3929/4728/150 | 0.157, 0.067 | 1.62, 0.244 | 0.244 | 1.5 %, 261.5 / 259.8 | 19 / 27 |
| PCIe_REFCLK_P | pair | 37.6 | 7705/3594/4511/170 | 0.003, 0.001 | 26.60, 0.305 | 0.305 | 1.3 %, 252.3 / 250.7 | 12 / 14 |

LVDS lanes (profile lvds, 1.5 × Nyquist = 0.75 GHz) and the PCIe lanes (pcie4, 16 GT/s, 1.5 × Nyquist = 12 GHz), single-ended references 50 Ω (LVDS) and 42.5 Ω (PCIe) per port. Fits of the main line sections: R 7–8 %, X 0.7–0.9 %, G 0.7–3.2 %, B ≤ 0.3 % (worst over all sections, short ones included: G 5 %, B 0.9 %; a microstrip piece over a distant plane on the PCIe receive lanes: G 96 %, B 19 %, a few tenths of a millimetre).

Plots (|S21|/|Sdd21|, |S11|/|Sdd11|, the differences, the step responses):

![SYNC_OUT](../img/channel-export/sync_out_p.png)
![PCIe PET1](../img/channel-export/pcie_pet1_p.png)

- **ngspice = kisi's evaluation of the circuit:** for all lanes the largest S-parameter difference against kisi
  is the same to three digits in ngspice and in kisi's own evaluation – the netlist text is the circuit.
- **Bus:** the N-port's blocks equal the lanes' Touchstone files (difference 0); the bus subcircuit in ngspice with
  one lane's four ports driven and all other ports terminated gives that lane's response (same difference to kisi
  as the lane alone). For the PCIe bus (9 lanes, 36 ports) the bus subcircuit's S-parameter run took 9–11 minutes in ngspice (4 ports driven, 32 terminated, 500 points).
- **Transient:** the PRBS7 benches run in ngspice (about 100 s per lane on one core).
- **KiCad** (`--kicad`, `kicad_sheet.py`): every lane's sheet passes the wiring check (visible pins, no open pin or
  dangling wire, junctions, grid); kicad-cli's netlist equals the lane subcircuit node for node, and its AC response
  in ngspice equals the bench's (relative difference 1e-10 to 5e-8, rounding). With kicad-cli 7.0.11 here for the LVDS
  lanes and all nine PCIe lanes (27–55 symbols per sheet); with kicad-cli 9.0.9 in CI (KiCad 9 from the KiCad PPA)
  for SYNC_OUT_P, PCIe_PER0_P and PCIe_PET1_P, where KiCad's ERC reports no error and no unconnected pin, dangling
  wire or off-grid item – only `lib_symbol_issues` warnings, one per symbol, because the sheet's library `kisi` was
  not in a library table; the export now ships it as a project library (`kisi.kicad_sym`, `sym-lib-table`). KiCad 9
  thus reads the KiCad 7 file, its models and `Sim.Params` with `${VALUE}`. The sheets as kicad-cli renders them:
  `docs/img/channel-export/kicad_*.png` (kicad-cli 7) and the CI artifact `kicad-sheets` (kicad-cli 9). Transients of
  the KiCad netlist and the bench differ by about 1e-4 of the swing: ngspice's time steps depend on the element order.
  Not tested: the simulator dialog of the KiCad GUI.
- **scikit-rf** reads every file (2-, 4- and 36-port). The lanes' Touchstone files are passive and reciprocal
  except the PCIe reference clock lane (kisi's model, see the summary); the bus N-port inherits that.
- **Not run here:** Xyce (no package), Qucs-S/Qucsator, PyBERT – the decks follow the Xyce reference guide
  (`.lin sparcalc=1`, port devices `P`, `.print tran format=csv`) but are untested.

Where the differences come from (diagnosed element by element while building the export): line ladders against
kisi's line model ≤ 0.005 per element; the via ladders ≤ 0.03 up to 12 GHz, more above where kisi's via-pair spans
turn active; the remaining in-band differences of the PCIe lanes come from small phase errors in channels with
large reflections. The fallback model of sections without a reference plane is the one kisi's 4-port uses.

## 5. Choices and why
- **Physical ladder, not a macromodel**, as the default: passive and causal by construction, readable (a user can add
  a termination at a via), no fitting failures, one generator for all lanes. Its size (10–40 k elements) is fine for
  AC and for transients of a few hundred unit intervals.
- **Modal blocks** for pairs (controlled sources) instead of coupled inductors: exact for kisi's model, including the
  common-mode loss; every SPICE reads E and F elements.
- **Terminations at the receiver** belong to the receiver (as in kisi's eye), so the Touchstone file and the
  subcircuit are the interconnect and the bench is the profile's ends plus that termination.
- **KiCad 7 format** for the project: the oldest format with the simulation model fields; KiCad 8 and 9 read it
  (and convert it when saving), and it can be tested with kicad-cli 7 here and kicad-cli 9 in CI.
- **One symbol per element, through wrappers:** the sheet is the netlist a user can edit; the wrappers keep the
  file KiCad parses short (with the 7 k-line lane library as `Sim.Library`, kicad-cli 7 sometimes did not find the
  model) and the elements' own subcircuits stay in one library shared with ngspice and Xyce.
- **20 sections per wavelength:** 40 or 80 change nothing measurable against kisi's model (the fits dominate).

## 6. Step 3 – compact models (design study)
`validation/channel_export/vf_prototype.py` fits a lane's differential 2-port with scikit-rf's vector fitting:

| Lane | delay | poles | max |ΔS| | passive |
|---|---|---|---|---|
| PCIe PET1 (42 mm) | 270 ps | 17 (32 requested) | 0.021 | yes |
| PCIe PET1, delay taken out of S21 only | 270 ps | 19 | 0.37 | no |
| LVDS SYNC_OUT (85 mm) | 574 ps | 13 | 0.145 | yes |
| LVDS SYNC_OUT | 574 ps | 32 (60 requested) | 0.014 | no – scikit-rf's enforcement diverges |

A lane with its delay needs about two poles per ripple (delay × top frequency); taking the delay out of S21 alone
does not help a reflecting lane, because the delay sits in S11 as well. A compact model therefore needs the delay as
ideal lines (T elements) at the ports and a fit of what lies between them (delayed rational fitting), with a robust
passivity enforcement – the hard part. Plan: per long line segment (not per lane) a constant-delay T line plus a
fitted loss network (the line's attenuation is smooth and minimum-phase), the vias and parts stay ladders; this
shrinks a 100 mm lane from ~40 k to a few hundred elements and keeps the export's structure.

## Sources
- ngspice manual (S-parameter analysis `.sp`, `portnum`; E/F sources) – https://ngspice.sourceforge.io/docs.html
- Xyce Reference Guide (`.LIN`, port device) – https://xyce.sandia.gov/documentation-tutorials/
- KiCad source, simulation model fields and inference (`eeschema/sim/sim_model.cpp`) – see `kicad-simulation.md` [K1]
- scikit-rf `VectorFitting` – https://scikit-rf.readthedocs.io/en/latest/api/vectorFitting.html
- A. Djordjevic et al., “Wideband frequency-domain characterization of FR-4 and time-domain causality”, IEEE Trans.
  EMC 43 (2001) – the dielectric model the Debye fits follow
- B. Gustavsen, A. Semlyen, “Rational approximation of frequency domain responses by vector fitting”, IEEE Trans.
  Power Delivery 14 (1999)
