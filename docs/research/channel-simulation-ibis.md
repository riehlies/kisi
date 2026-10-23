# Channel simulation with IBIS models – research and concept

Status: research and concept (October 2026), no implementation. Scope: the physical channel only – IBIS
behavioural models of drivers and receivers at the ends of kisi's linear channel models. Equalisation (CTLE, FFE,
DFE), pre-/de-emphasis and IBIS-AMI are left out on purpose. Claims that could not be checked are marked
*(not verified)*; sources are linked in the text and listed at the end.

## Summary
- **Recommended method:** keep everything linear in kisi's frequency-domain models (lines, vias, passives,
  unpopulated pads, IBIS package R/L/C and C_comp), turn the lane or bus into an N-port, take its impulse
  responses, and solve only the nonlinear IBIS buffer currents at the die nodes in the time domain
  (convolution with Newton per time step, the method of Schutt-Ainé and Mittra [S20]). This reuses kisi's
  strongest part (skin effect, dielectric loss, via ladder) and runs in WebAssembly. Long patterns and eyes come
  from edge-response superposition on top.
- **Results move from "fraction of swing" to volts and picoseconds** (overshoot, ringback, slew rate, flight
  time, eye against thresholds). The verdict stays in `kisi_si::profile::judge`, extended by new metrics.
- **Reference simulator:** KiCad's own simulator (KIBIS → ngspice) on the same IBIS files. KIBIS is
  BSD-3-Clause licensed [S3], so its parser logic may be ported to Rust with attribution. Constant-RLGC lines in
  ngspice are not enough for kisi's lossy channels; a vector-fitted SPICE subcircuit from scikit-rf [S10] of
  kisi's Touchstone export closes that gap.
- **Models:** the repository ships only synthetic models and KiCad's artificial QA models. Vendor models stay in
  local folders that `<board>.kisi.json` names but never embeds. For the FPGA (AMD), the best model is Vivado's
  design-specific export with per-pin package parasitics [S24].
- **Assembly variants first** (small, high value): on ThunderScope Rev5.3 the board file holds both host paths
  populated, so kisi today attaches the unused path as a long open branch. Marking the other variant DNP changes
  insertion loss at 8 GHz from −14.1 dB to −2.5 dB on one lane (table in section 6). KiCad 10 stores variants in
  the board file [S29]; for KiCad 8/9 boards kisi.json needs its own variant list (by sheet, references or a BOM).
- **Test flows:** a DDR3/DDR4 "wizard-lite" (byte lanes, fly-by, ODT/drive settings, JEDEC signal-quality checks)
  is realistic; DDR5 without DFE stays indicative only. A generic single-ended flow (clocks, GPIO, RGMII) should
  come first because it needs the same engine with less setup.
- **Effort:** about 55–85 focused days in seven milestones (section 7.5) – months as a side project. Variants
  and the IBIS parser are useful on their own and can ship early.

## 1. Where kisi stands
**Channel today.** `kisi_si::rf::channel` cascades ABCD matrices along a path: RLGC line sections, vias as the
physics-based ladder (`via_pbm`), series parts and terminations with package parasitics from
`data/components.json`, branches as open stubs (nets only), pairs in odd mode. `rf::mixed` has the 4-port pair
model (shown in the net detail, not yet used by eye and check). `topology` follows nets through series resistors
and AC-coupling capacitors into device-to-device lanes; DNP parts are skipped.

**Eye today.** `eye.rs` takes the step response of the transfer function between linear end models
(`EndModel`: R, C, L per line), superposes a PRBS7 and reports height as a fraction of the transmitter swing. The
profiles in `data/interfaces.json` carry swing, R, C, rise time and limits; there is no DDR3 profile and no
absolute voltage anywhere. Lanes ending at a connector get no eye; multi-drop and open lanes are not checked.

**The 0.1.0 experiment** (commit `92b8105`, removed in `26c4bde`) parsed IBIS files (Component, Package, Pin,
Model Selector, I-V tables, Ramp, Rgnd/Rpower), found the model per pin from KiCad's `Sim.*` fields and
*linearised* it into `EndModel` (tangent of the I-V curves at the [Ramp] load; ODT from the clamp slope). Lessons:
- Linearising throws away what IBIS adds (clamps, nonlinear drivers, edge shape), so IBIS ends barely differed
  from profile ends – and it needed a swing conversion (`ibis_swing_mv`) to fit the profile's normalised eye.
- KiCad's `Sim.Device=IBIS` fields are set on simulation symbols, rarely on real ICs; pad numbers did not match
  [Pin] names in single-pin symbols; the first [Model Selector] entry was used, which can be the wrong ODT or
  drive setting.
- Current-mode drivers (LVDS, USB 2.0 HS, CML) do not linearise well around a resistive [Ramp] load.
- There was no reference simulator, so nothing was validated.

**Gaps that matter for IBIS:** voltages instead of fractions; N-port lanes (multi-drop, both lines of a pair);
side stubs on lane nets are not modelled ("side stubs … are not included"); no assembly variants; pad pin
functions (`pinfunction`) are neither parsed nor kept by `tools/strip_pcb.py`; no Touchstone import.

## 2. Concept: IBIS ends on a linear channel

### 2.1 What an IBIS file contributes
IBIS describes buffers by tables, not transistors [S1][S2]. Current specification: IBIS 8.0, ratified
5 December 2025; IBIS 7.2 was ratified 27 January 2023; IBIS 7.0 added Touchstone and IBIS-ISS interconnect
models [S1][S5]. Touchstone 2.1 dates from January 2024, IBIS-ISS 1.0 from October 2011 [S1].

| IBIS element | Describes | Role in a physical-channel flow |
|---|---|---|
| [Component], [Pin] | pin → signal name → model; R/L/C_pin per pin | map board pads to models; pin parasitics |
| [Package] | default R/L/C_pkg (typ/min/max) | fallback when [Pin] has no values |
| [Package Model], `.pkg` | package RLC matrices and sections | later; uncoupled (diagonal) first |
| [Pin Mapping] | supply/ground buses each buffer refers to | rail voltages; power-aware use left out |
| [Diff Pin] | pin pairs, Vdiff threshold, tdelay | drive P/N complementary with skew; receiver threshold |
| [Model Selector] | alternatives for a pin (drive strength, ODT, I/O standard) | must be chosen explicitly; default is the first entry *(per spec, not re-read in 8.0)* |
| [Model] `Model_type` | Input, Output, I/O, 3-state, open drain/sink/source, ECL types, Terminator, Series, Series_switch | which tables apply |
| C_comp | die capacitance | linear, part of the N-port |
| [Pullup], [Pulldown] | driver I-V tables | nonlinear current at the die |
| [GND Clamp], [POWER Clamp] | clamps; ODT often given as straight clamp curves | nonlinear at the die (overshoot) |
| [Rgnd], [Rpower], [Rac], [Cac] | terminator models | linear, part of the N-port |
| [Ramp] | 20–80 % dV/dt into R_load | fallback edge |
| [Rising/Falling Waveform] | V-t into fixtures (R/V_fixture) | switching coefficients Ku(t), Kd(t) |
| [Model Spec], [Receiver Thresholds], Vinh/Vinl, Vref/Rref/Cref/Vmeas | thresholds and timing test load | receiver thresholds; flight-time reference |
| [Series Pin Mapping], series models | on-die series elements, switches | linear R/L/C or I-V between two pins |
| [Driver Schedule], [Algorithmic Model] (AMI) | multi-stage drivers (pre-emphasis), equalisation | **out of scope** |
| [Composite Current], [ISSO PU/PD] | power-aware data | out of scope at first (ideal rails) |
| [Submodel] (dynamic clamp, bus hold) | extra behaviour | warn, later |
| EBD (`.ebd`) | board paths of modules such as DIMMs | later, for DIMM connectors |
| IBIS-ISS, [Interconnect Model Set] | SPICE-subset or Touchstone interconnect | Touchstone only; ISS needs a SPICE parser (out) |

Model quality varies. The IBIS Cookbook recommends four V-t waveforms per corner for push-pull outputs (50 Ω
to Vcc and to GND) and two for open-drain types, without reactive fixtures [S6]. ADI's IBIS article describes the
same structure and why loads should match the system impedance [S26]. Micron's DDR3/DDR4 talks show what good
memory models need: per-ball package models, proper ODT modelling and V-t windows long enough for all corners
[S35][S36].

### 2.2 How a simulator combines buffer and channel
The usual driver model is a current at the die node,

    i(v, t) = Ku(t)·I_pullup(Vcc − v) + Kd(t)·I_pulldown(v) + I_clamps(v) + C_comp·dv/dt,

with package R/L/C between die and pin. Ku and Kd are switching coefficients: with two V-t waveforms per edge,
each time point gives two equations for two unknowns; with one waveform an assumption fills the gap; with [Ramp]
only, they ramp linearly. KIBIS implements exactly these three levels (0, 1 or 2 waveform pairs, "poor accuracy"
warning for [Ramp]) and runs ngspice to obtain Ku/Kd [S3]. The receiver is C_comp, clamps and an optional
terminator. The channel is linear and time-invariant. (The IBIS-AMI flow makes the same LTI assumption for the
analog channel including the analog buffer models *(IBIS spec, AMI chapter; not re-read for this document)*.)

### 2.3 Methods and trade-offs
| Method | How | Nonlinear ends | Cost | Typical use |
|---|---|---|---|---|
| A. SPICE transient | buffers as B-sources with PWL Ku/Kd (KIBIS), channel as T/LTRA lines or a vector-fitted subcircuit | full | high; needs a SPICE engine; frequency-dependent loss needs a fitted model | reference, short patterns |
| B. Convolution with nonlinear ports | channel impulse responses from S(f); buffer equations solved per time step by Newton [S20] | full at the ports | medium: steps × taps | **recommended core** |
| C. Edge-response superposition | one rising and one falling response (from B), superposed for long patterns | approximate (LTI per edge, asymmetry kept) | low | long PRBS, eye diagrams |
| D. Statistical / peak distortion | pulse response → worst-case eye (PDA [S21]) or PDFs (statistical eye) | approximate | very low | worst case; BER contours need jitter/noise models |
| E. Linear ends (today) | Thevenin source and RC receiver | none | very low | first look, fallback |

B is accurate when the impulse responses are good: a DC point, a frequency range well above the edge bandwidth,
windowing and a causality/passivity check. C is accurate for buffers that are nearly linear during an edge
(ZQ-calibrated SSTL/POD drivers, CML) and less so while clamps conduct. Commercial DDR tools use D for low BER
contours – Keysight's DDR Bus Simulator builds the eye "from the transmitter, channel, and receiver impulse
responses" [S16].

### 2.4 What a physical-channel-only flow needs
- **Channel:** an N-port per lane or bus – lines, vias, passives, stubs of unpopulated pads, IBIS package and
  C_comp, optional Touchstone blocks (connectors); both lines of a pair as a 4-port.
- **Ends:** IBIS driver (I-V and V-t or [Ramp]), receiver (C_comp, clamps, ODT/terminator), corners typ/min/max,
  explicit [Model Selector] choice, [Diff Pin] pairs; profile ends remain the fallback.
- **Stimulus:** edges, clocks, PRBS, DDR-like bursts at a bit rate. **Probes:** pin and die of every receiver.
- **Measurements:** overshoot/undershoot (peak, area), ringback, slew rate, flight time (with the Vmeas/Rref/Cref
  reference load), skew, eye height/width in V/ps, mask margin.
- **Not needed:** AMI, CTLE/FFE/DFE, [Driver Schedule], jitter/noise models, power-aware SSN (all later or never).

## 3. Solvers and tools

| Tool | Licence | IBIS | Channel | Use for kisi |
|---|---|---|---|---|
| **KiCad simulator + KIBIS** (`eeschema/sim/kibis`, files from 2022; in KiCad since 7.0 *(not verified)*) | KIBIS files BSD-3-Clause [S3]; KiCad GPL-3.0+ | parser covers most keywords incl. [Package Model], [Diff Pin], [Receiver Thresholds]; SPICE output uses one lumped R/L/C per pin ([Pin] or [Package]), not [Package Model]; drivers DC, rectangular, PRBS; typ/min/max; differential via `Sim.Ibis.Diff`; Series/Series_switch; no Terminator device | ideal T and LTRA with constant RLGC [S4] | **reference simulator**; parser logic to port; QA models as fixtures |
| **ngspice** | BSD-3-Clause [S8] | none built in (KiCad generates B-source models) | T, LTRA, TXL, CPL; .SP analysis [S7] | engine behind KiCad; runs kisi-exported decks |
| **scikit-rf** | BSD-3-Clause | – | Touchstone I/O, time domain, vector fitting with passivity enforcement, `write_spice_subcircuit_s` [S10] | turns kisi's `.s2p/.s4p` into a SPICE subcircuit for validation |
| **PyBERT / PyIBIS-AMI** | BSD-style [S11][S12] | IBIS parser; Tx/Rx analog as series/shunt resistance from zout/zin [S13] | Touchstone, LTI | cross-check for the LTI eye only |
| **SignalIntegrity** (Pupalaikis) | GPL-3.0+ [S14] | no IBIS reader found in the repository | S-parameter networks, simulator, eye | network/eye cross-checks |
| **Xyce** | open source *(licence not re-checked)* | – | YLIN Touchstone device, documented mainly for harmonic balance [S15] | not needed |
| **Qucs-S** | GPL | via engines | Xyce Touchstone device not implemented (2023) [S15] | not needed |
| **IBISCHK** (golden parser) | free executables; source terms not stated [S9] | full syntax check | – | local check of vendor files |
| **S2IBIS3** (NCSU) | free, unsupported [S9] | SPICE → IBIS 3.2 | – | possible source of shippable test models *(not tried)* |
| **Rust** | – | no IBIS or Touchstone crate found (crates.io unreachable from here; web search found none) *(not verified)* | – | write own |

**Commercial references.** HyperLynx DDR automates post-layout verification of whole DDR interfaces: topology
extraction, a wizard for controller capabilities (1T/2T, read/write leveling, dynamic ODT, deskew), checks
against JEDEC and controller parameters in a pass/fail report, DDR2–5 and LPDDR2–5, eye margins at 1e-16 for
DDR5 [S17]. Keysight's DDR Bus Simulator is statistical (impulse responses, BER contours down to 1e-16, margins
against the DDR4 receive mask; IBIS, built-in and circuit models) [S16]. Cadence Sigrity SystemSI has
parallel-bus compliance checks for DDR2/3/4 and LPDDR3/4 with SSO analysis [S18]; Cadence's DDR guide lists eye
width/height, overshoot, ringback, DQ-to-DQS skew and Rx mask margins and states that DDR5 needs DFE/CTLE via AMI
[S19]. Ansys SIwave has a DDR wizard that builds a schematic and compliance toolkits per standard *(only a user
forum was found)* [S19b]. Common to all: the tool, not the user, assembles the test bench from the layout.

## 4. IBIS models: sources, licences, private folders

### 4.1 Where to get them
| Source | Access (as found) | Notes |
|---|---|---|
| IBIS Open Forum model list [S25] | links only; last checked by the forum 27 Jan 2022 | notes that Broadcom needs a login, Marvell an NDA, Sony models go through sales, Cirrus needs an agreement; AMD, Hynix and others "on request" |
| Micron (DDR3/4/5 DRAM) | from the part page, e.g. a zip with `.ibs` and readme for MT41K512M16HA-125 [S27] | some models only on request (reported for LPDDR4) [S28]; login *(not verified)* |
| Intel/Altera FPGAs | per-family zip on intel.com, no login mentioned; "for modeling purposes only" [S23] | |
| ADI | list with direct `.ibs` links; some AMI kits gated; no licence statement [S26b] | |
| TI | per product page *(not verified per part)* | |
| Microchip | product pages, e.g. SmartFusion2/IGLOO2 [S30] | |
| NXP, ST, Lattice, Infineon, Renesas, onsemi | product pages; listed by the IBIS Open Forum [S25] | not checked individually |
| **AMD/Xilinx** | Vivado "File > Export > Export IBIS Model": pins used by the design, model per I/O standard, per-pin package path [S24]; generic models ship with Vivado; 7-series transceiver IBIS-AMI behind an AMD account sign-in [S22] | private; design-specific export preferred |

For the example boards: ThunderScope uses an Artix-7 (AMD), HMCAD1520 ADC, FIN1019 LVDS buffer and ZL30260 clock
generator; LibreVNA a Spartan-6 (AMD) with an SI5351C clock driving the FPGA clock net. Models for these parts
were not downloaded. Neither board has DDR (ThunderScope Rev5 dropped DDR3 [S31]).

### 4.2 Licence situation
Vendor IBIS files carry [Copyright] and [Disclaimer] keywords; the pages checked (ADI, Intel) state no licence
for redistribution. Treat every vendor file as **not redistributable** unless the file or page says otherwise.
- **In the repository:** synthetic models written by a kisi script from analytic buffers (exact expected
  results, GPL-3.0); KiCad's artificial QA models (`qa/data/eeschema/ibis_v1_1.ibs`, `ibis_v4_1_*.ibs`,
  `ibis_v5_1.ibs` – the latter with [Package Model] and [Model Selector]), marked "Artificial data … NOT a valid
  component", part of KiCad (GPL-3.0-or-later, compatible with GPL-3.0-only); possibly models generated from
  open SPICE models with S2IBIS3 *(not tried)*.
- **Local only:** all vendor models, including the AMD models the maintainer downloads or exports. CI runs only
  the shippable fixtures; tests with private models are opt-in (`#[ignore]` plus an environment variable with the
  folder).
- **JEDEC standards** are paid documents (JESD79-3F $247, JESD79-4D $284, JESD79-5D $423; addenda free with
  registration) [S32]. Limit values can be cited with sources in a data file, like `interfaces.json` does;
  vendor datasheets (e.g. Micron) repeat many of them and are free.

### 4.3 Private model folders
- `<board>.kisi.json` gets a `models` section: search folders (relative to the board, `~` expanded by the CLI)
  and per file the name, SHA-256, `[File Rev]`, a source entry and `"shareable": false`. Never the content.
- **CLI:** folders from kisi.json, `--models DIR` and an environment variable; `config-check` warns when a
  non-shareable file lies in a folder tracked by git *(option)*.
- **Viewer:** files come in by drag and drop or a folder picker (File System Access API in Chromium), are parsed
  locally and kept in memory; storing them in IndexedDB only on request ("remember on this computer"). The page
  already loads nothing from the web.
- **Outputs** (`report`, JSON, screenshots of waveforms) name file, hash, component, model, corner – never tables
  from the file.

## 5. Test automation and bus test suites

### 5.1 What DDR test suites check
The standards are JESD79-3F (DDR3, July 2012), JESD79-4D (DDR4, July 2021) and JESD79-5D (DDR5, November 2025)
[S32]. The commercial suites in section 3 check signal quality and timing against them; vendor notes add design
practice: Micron TN-41-13 asks to simulate the chosen termination at both corners (slow corner at maximum ODT
tolerance), uses simulated slew rates for DRAM setup/hold derating, terminates the last device of a fly-by to
VTT, and notes that fly-by creates clock-to-data skew, compensated by write leveling or by fixed skews [S33]. TI's KeyStone
DDR3 guide terminates fly-by address/command through 39.2 Ω to VTT and clocks through 39.2 Ω to a capacitor node
[S34]; AMD UG586 covers the same for 7-series memory interfaces [S37].

| Check | DDR3 | DDR4 | DDR5 | In kisi without EQ |
|---|---|---|---|---|
| Overshoot/undershoot (peak, area in V·ns) | yes | yes | yes | yes – needs clamps, i.e. IBIS |
| Ringback into the threshold region | AC/DC levels | yes | yes | yes |
| Input levels around Vref | SSTL-15, external Vref | POD12, trained internal VrefDQ | POD11 | yes, levels from a data file |
| Slew rate and setup/hold derating | tables for tIS/tIH, tDS/tDH | CA *(details not checked)* | – | measure yes; derating tables in a data file |
| DQ receiver mask (window at BER) | – | yes | yes, receiver with DFE | DDR4 yes; DDR5 indicative only |
| Differential crosspoint (VIX), time above AC level | CK, DQS | yes | yes | yes |
| Skew DQ–DQS within a byte lane | yes | yes | yes | yes |
| CK–DQS at each DRAM (write-leveling range) | fly-by | fly-by | fly-by | yes, flight times against the controller's range |
| Flight time and timing budget | yes | yes | yes | board share; controller timing stays with its vendor tool |
| ODT and drive-strength sweeps | yes | yes | yes | yes, as [Model Selector] sweeps |
| Crosstalk, SSN | yes | yes | yes | later (multi-aggressor, power-aware) |

The level families (SSTL-15, POD12, POD11), the DDR4 CA details and the DDR5 receiver DFE are from secondary
sources, not checked in the standards *(not verified)*. The "1e-16" BER contours of DDR4/DDR5 [S16][S17] need
jitter and noise models; without them kisi can give the deterministic worst case (PDA) and should say so.

### 5.2 Automating the setup from the board
The setup is the laborious part, so kisi should derive it and ask only for what the board cannot tell:
1. **Find the bus:** memory parts from pad pin functions (KiCad stores the symbol pin name per pad, e.g. DQ0,
   DQSL, A0 – to be parsed and kept by `strip_pcb.py`), net names (existing regex rules) and the IBIS [Pin] signal
   names. The controller is the other end of the DQ nets.
2. **Group:** byte lanes (DQ0–7 with DQS0, DM0) from the DRAM pin functions rather than net names; address,
   command, control and clock as multi-drop trees ordered along the route; VTT terminations at the end (net name,
   or the power tree's `PI_REG`).
3. **Settings:** ODT (RTT_NOM/RTT_WR/RTT_PARK), drive strength, controller output impedance and input
   termination as [Model Selector] choices in kisi.json, with sources (e.g. the memory controller configuration).
   Defaults from the standard's typical setups, marked as assumptions.
4. **Test matrix:** write (controller → DRAM) and read per byte lane, address/command (1T/2T) and clocks; corners
   min/typ/max; selector sweeps.
5. **Run and judge:** the checks of 5.1 per pin, worst value per check, verdict colours from `judge`, a report
   with the model sources and the words "physical channel only, no equalisation, jitter or SSN".

The test data per standard (levels, overshoot limits, derating tables, masks, write-leveling ranges of common
controllers) belongs in `data/ddr.json`, each value with its source.

### 5.3 Other buses
- **Generic single-ended** (clocks, GPIO, SPI, RGMII): the first flow – overshoot, ringing, slew, flight time,
  monotonic edges at the threshold. LibreVNA's SI5351C → Spartan-6 clock net and ThunderScope's SYNC lines are
  candidates.
- **LVDS:** current-mode drivers as [Diff Pin] pairs; differential swing and common mode at the receiver, eye.
- **USB 2.0 HS:** current-mode, eye templates from the USB 2.0 specification; mostly board share (connector,
  cable off board).
- **Ethernet:** RGMII as above; 1000BASE-T MDI through magnetics stays a linear board-share check.
- **SerDes up to PCIe:** LTI is accurate; IBIS adds the analog end (termination, C_comp, package) instead of the
  profile values. Without de-emphasis and EQ the eye is pessimistic for PCIe Gen3+, as kisi already says.
  ThunderScope's lanes are auto-assigned `pcie4`; whether the Artix-7 runs Gen2 there is worth checking in
  `ts.kisi.json`.

## 6. Assembly variants

### 6.1 How KiCad represents population
- **DNP attribute** (KiCad 8+): `(attr … dnp)` on the footprint; kisi already reads it (`Footprint::dnp`) and
  skips such parts in lanes and the PDN.
- **KiCad 10 design variants** (10.0.0 released 20 March 2026 [S38]): variants live in the schematic and are
  written into the board file. Since format 20260101 the board header has
  `(variants (variant (name "…") (description "…")))` and each footprint may carry
  `(variant (name "…") (dnp …) (exclude_from_bom …) (exclude_from_pos_files …) (field (name "…") (value "…")))`
  with only the overrides [S29]. Variants can also change values and footprints; changing the footprint puts both
  footprints on the board [S39]. kisi can read all of this from the `.kicad_pcb`.
- **Plugins:** KiVar (MIT) keeps variation rules in footprint fields and applies them in place, with several
  independent "aspects" [S40].
- **Outside KiCad:** separate BOM and position files per variant – what ThunderScope Rev5.3 does.

### 6.2 ThunderScope Rev5.3
The repository has only the branches `master` and `gh-pages` (`git ls-remote`); the GitHub web pages could not be
fetched from here. The "branch" is therefore read as the fork in the routing: Rev5.3 ships two assemblies,
TS-PCIe and TS-USB4 (M.2 to a Thunderbolt adapter), with separate BOMs and position files [S41]. Findings from the
board and BOM files (KiCad 9 board):
- The variants map to hierarchical sheets: parts in `/TS-PCIe Components/` vs `/TS-USB4 Components/`, with three
  capacitors per sheet fitted in both. A `VARIANT` pin strap tells the firmware which one is built.
- The PCIe lanes leave the FPGA on one net and fork into AC capacitors (the FPGA's transmit lanes, `PERx` at the
  connectors, e.g. C41 for P1 and C43 for J8) or 0-Ω resistors (its receive lanes, `PETx`, e.g. R38/R40), placed
  **back to back** at the same position on F.Cu and B.Cu and joined by a through via about 1.25 mm away. In either
  variant the unused side leaves a via and an unpopulated pad as a stub.
- Nothing is DNP in the board file, so kisi computes each lane with the other path attached as an open branch
  through the other capacitor/resistor up to the other connector.

kisi 0.2.3 native, formula impedances, `ts.kisi.json`; variants made by marking the other assembly's parts DNP
from the MP BOMs (scratch copies, not committed). IL at 8 GHz (Nyquist of the auto-assigned `pcie4`), RL = worst
return loss up to 12 GHz:

| Lane (FPGA U18 – connector, series part) | Board file as is: IL / RL (dB) | TS-PCIe: IL / RL | TS-USB4: IL / RL |
|---|---|---|---|
| P1 PER0 (C41, FPGA transmit) | −2.37 / 0.60 | −1.50 / 7.26 | – |
| P1 PET3 (R26, FPGA receive) | −14.07 / 0.47 | −2.49 / 4.97 | – |
| J8 PER3 (C31, FPGA transmit) | −7.10 / 0.51 | – | −0.94 / 8.95 |
| J8 PET0 (R40, FPGA receive) | −4.45 / 0.65 | – | −3.32 / 6.67 |

With DNP set the lanes become clean point-to-point paths, but the via and pad stub of the unused footprint are
then missing (lanes ignore side stubs), so the truth lies between the columns, close to the variant columns.

**Update (M0 implemented, branch `feat/variants`):** the variants are defined in `web/samples/ts.kisi.json` by
sheet (TS-PCIe: `/TS-USB4 Components/` not fitted except C101, C102, C128; TS-USB4: `/TS-PCIe Components/` not
fitted except C22, C83, C84 – the parts of the MP BOMs; parts that no BOM lists, such as test points and the edge
fingers P1, follow their sheet). Lanes now carry side stubs: the
unused pad (0402 pad capacitance, estimate) at the end of its trace – and, in TS-USB4, the whole F.Cu–B.Cu via
barrel, because the fitted part sits on F.Cu next to the FPGA trace. The board as it is (`--variant default`)
attaches the other path at the fork instead of at the FPGA pin. Hops now end on the pad's layer: 0.2.3 could end
on the other part's pad (same position, other side) and skip the through via. Same lanes, same settings
(native, formula, `pcie4` at 8 GHz; RL up to 12 GHz, seen from the first end – the FPGA for the board as it is,
the connector for the variants):

| Lane | Board as it is: IL / RL (dB) | TS-PCIe: IL / RL | TS-USB4: IL / RL | at 2.5 GHz (`pcie2`), variant: IL / RL |
|---|---|---|---|---|
| P1 PER0 (C41) | −2.19 / 0.69 | −3.12 / 4.05 | – | −0.55 / 16.9 |
| P1 PET3 (R26) | −3.32 / 1.03 | −4.37 / 2.44 | – | −0.78 / 9.9 |
| J8 PER3 (C31) | −3.60 / 0.58 | – | −3.44 / 1.84 | −0.40 / 14.4 |
| J8 PET0 (R40) | −4.10 / 1.03 | – | −5.71 / 3.64 | −1.16 / 10.6 |

The variant numbers are worse than the DNP estimates above because the through via and the stubs are now in the
path; in TS-USB4 the ~2.2 mm stub (via barrel, 0.7 mm on B.Cu, pad) resonates near 12.5 GHz and dominates above
6 GHz. At PCIe Gen2 rates the variants are clean, the board as it is is not (e.g. P1 PER0 −1.41 / 0.69 dB).

### 6.3 Proposal
- **Variant selection:** KiCad 10 variants from the board file; for older boards a `variants` section in
  kisi.json – by sheet path, by reference list, or from a BOM CSV ("everything not in this BOM is DNP", except
  parts excluded from the BOM such as gold fingers and test points). `"variant": "<name>"` picks one; `--variant`
  on the CLI and a selector in the viewer override it. Precedence as today: board < kisi.json < viewer/CLI.
- **Electrical effect:** a DNP part disappears as a link but its pads stay: the copper of the lane net up to the
  unpopulated pads becomes an open stub with the pad capacitance from `components.json`; a fitted 0-Ω or AC
  capacitor stays a series part. This needs side stubs in lanes (they exist for nets already).
- **Variant-dependent values:** fields that differ per variant (values, `PI_*` fields) follow the variant – the
  power tree of `ts.kisi.json` already says "PCIe variant" in a note; the M.2 supply path (J8, R54) could then be
  a second variant instead of being left out.
- **Report:** every result names the variant; `gaps` warns when parts in a fork are all fitted (as on
  ThunderScope) and no variant is chosen.

## 7. Proposal for kisi

### 7.1 Architecture
```
 .kicad_pcb/.kicad_pro ─┐
 <board>.kisi.json ─────┼─► variant → fitted parts, field values           (kisi-board, kisi-core::config)
 (variant, models,      │
  settings, sources)    ▼
               lanes, buses, multi-drop trees                               (kisi_si::topology)
                        ▼
               linear N-port: lines, vias, passives, unpopulated pads,      (kisi_si::rf::network, new)
               IBIS package R/L/C and C_comp, Touchstone blocks
                        ▼  S(f) on the uniform grid → impulse responses
 local .ibs ──► kisi-ibis: parse, pick model per pin (selector, corner),    (new crate)
                I-V tables, Ku/Kd(t), thresholds, diff pairs
                        ▼
               time-domain solver: convolution + Newton at the die nodes;   (kisi_si::tran, new)
               edge-response superposition for long patterns
                        ▼  waveforms at pins and dies
               measurements: overshoot, ringback, slew, flight time, skew,  (kisi_si::measure, new)
               eye in V/ps, mask margin
                        ▼
               kisi_si::profile::judge (one verdict) → viewer (WASM) / CLI (JSON)
```
- **`kisi-ibis` (new crate):** parser for IBIS 1.1–8.0 syntax (unknown keywords kept as warnings), model selection,
  corners, Ku/Kd extraction (solved directly, no SPICE), buffer equations with derivatives. No dependencies
  beyond serde; testable alone. The KIBIS parser is the reference for keyword coverage; ported logic keeps its
  BSD-3 notice.
- **`kisi_si::rf::network`:** nodal assembly of a net tree into an N-port (generalises today's ABCD cascade;
  complex LU from `kisi-sparse`); ports at IC pins; pairs as true 4-ports, which also settles the roadmap item
  "eye on the 4-port model".
- **`kisi_si::tran`:** ports implement one trait – IBIS buffer, profile end, ideal source – so the linear fallback
  and the IBIS path share the solver.
- **`judge`:** new `Metrics`/`Limits` fields (overshoot V and V·ns, ringback, slew V/ns, flight time, eye in mV/ps,
  mask margin); profiles gain absolute levels or point to `data/ddr.json`. Still one verdict function.
- **`kisi-core`:** config sections `models`, `ibis` (part → file/component/selector/corner), `variants`,
  `variant`; commands in `kisi_core::commands` as today.

### 7.2 WebAssembly and CLI
Everything numeric runs in WebAssembly: parsing, Ku/Kd, N-port, impulse responses, the solver (in a worker).
Rough cost: a 2-port lane with 2000-tap responses and 125 000 time steps is about 10⁹ multiply-adds – seconds in
WASM; edge-response superposition and recursive convolution (vector fitting) reduce it when needed. The CLI adds
model folders on disk, batch sweeps (corners, selectors, variants), CSV/JSON waveforms and an optional ngspice
deck export for validation.

### 7.3 kisi.json additions (sketch)
```json
{
  "variant": "TS-PCIe",
  "variants": {
    "TS-PCIe": {"dnp_sheets": ["/TS-USB4 Components/"], "fit": ["C101", "C102", "C128"],
                "source": {"kind": "design", "document": "Thunderscope_Rev5.3_TS-PCIe_MP_BOM.csv"}},
    "TS-USB4": {"bom": "Thunderscope_Rev5.3_TS-USB4_MP_BOM.csv"}
  },
  "models": {"folders": ["ibis-private/"],
             "files": [{"file": "<design export>.ibs", "sha256": "…", "shareable": false,
                        "source": {"kind": "simulation", "note": "Vivado Export IBIS Model, private"}}]},
  "ibis": {
    "U18": {"file": "<design export>.ibs", "component": "<from file>", "corner": "typ",
            "selectors": {"<selector name>": "<model name>"}, "source": {"kind": "assumption"}}
  }
}
```
`design` would be a new source kind (the project's own BOM or design files). KiCad's `Sim.Device=IBIS`,
`Sim.Library`, `Sim.Name`, `Sim.Ibis.Pin`, `Sim.Ibis.Model`, `Sim.Ibis.Diff` fields [S3] are read as a second
source, below kisi.json.

### 7.4 Viewer and CLI outline
- **Viewer:** a "Models" box (local files, never uploaded; which parts and pins are covered, missing models,
  [Copyright]/[Disclaimer] shown); per lane an "Ends" panel (profile or IBIS: file, pin, model, selector, corner);
  a waveform chart at pin and die with thresholds; the eye in volts with a mask; a "Test flow" panel per bus
  (detected roles, byte lanes, settings table, run, result table). A variant selector in the header.
- **CLI:** `kisi ibis <file.ibs> [--component C] [--pin P]` (summary, warnings, per-pin models);
  `kisi sim <board> --net NET [--pattern edge|clock|prbs7] [--corner min|typ|max] [--csv]`;
  `kisi bus <board> --flow se|ddr3|ddr4 [--variant V]`; `check` uses IBIS ends where kisi.json assigns them and
  states the model source per row; `--models DIR`, `--variant NAME` for all commands.

### 7.5 Staged plan
| Milestone | Content | Focused days |
|---|---|---|
| M0 Variants | KiCad 10 variants and kisi.json variants (sheets, refs, BOM); side stubs and unpopulated pads in lanes; `gaps` warning; ThunderScope config with both variants | 4–6 |
| M1 IBIS parser | `kisi-ibis` covering the in-scope keywords of the table in section 2.1, corners, selectors, diff pins; `kisi ibis`; model mapping in kisi.json; fixtures (synthetic, KiCad QA) | 6–10 |
| M2 N-port | network assembly incl. multi-drop and 4-port pairs; impulse responses (DC point, window, causality/passivity checks); compare with today's cascade (first increment in branch `feat/nport`, see below) | 8–12 |
| M3 Transient | buffers (Ku/Kd from 0/1/2 waveforms, clamps, terminators, series), Newton solver, waveforms; self-consistency and ngspice comparison | 10–15 |
| M4 Measurements | overshoot, ringback, slew, flight time, eye in V/ps; `judge` extension; waveform and eye views; `kisi sim`; generic single-ended flow | 8–12 |
| M5 DDR3/DDR4 flow | detection, byte lanes, fly-by, settings, `data/ddr.json`, report | 12–20 |
| M6 Long patterns | edge-response superposition, PDA worst-case eye; optional aggressors from the crosstalk scan | 6–10 |

**M2, first increment (October 2026, branch `feat/nport`):** `kisi_si::rf::network` (modified nodal analysis
of chains, admittances, S blocks and sources; equal to the cascade to 1e-11 in tests), `rf::timedomain` (DC point,
Gaussian window, band-edge, precursor and settling checks, passivity of S-matrices), `kisi_si::lanenet` (a lane's
blocks with the ends). Pairs are 4-ports in the check and the eye (lanes and nets without devices), with mode
conversion; a 4-port Touchstone file behind a connector enters as an S block with mode conversion; multi-drop lines
along one path are checked at every receiver. On the example boards no verdict changed; loss at Nyquist of the
ThunderScope PCIe lanes moves by up to ±0.7 dB because P and N differ, mode conversion reaches −6.4 dB where the
intra-pair skew is 11 ps. Open: N-port files for several lanes with crosstalk, multi-drop trees through series
parts, pairs on multi-drop lines, coupled fork branches of a pair, terminations across a pair as a 4-port element.

About 55–85 focused days in total. M0 and M1 are useful alone (correct ThunderScope lanes; `kisi ibis` as a model
inspector). A DDR test board is needed from M5 on: Antmicro's open KiCad boards (Apache-2.0) with a Kintex-7 and a
DDR4 RDIMM slot or LPDDR4 are candidates [S42][S43]; a board with discrete DDR3/DDR4 chips would be better *(none
selected yet)*.

### 7.6 Validation plan
1. **Parser:** KiCad QA models and synthetic files with known content; vendor files locally against IBISCHK
   warnings [S9].
2. **Self-consistency (no external tool):** every driver into its own V-t fixtures must reproduce its [Rising/
   Falling Waveform] tables (target: within a few percent of the swing). This checks Ku/Kd and the solver.
3. **Old vs new engine:** with linear I-V tables the new solver must match today's `eye::transfer` on the same
   channel.
4. **Reference simulator:** the same IBIS file in KiCad 10/ngspice (KIBIS), with (a) an ideal T line, (b) a
   constant-RLGC LTRA line, (c) kisi's `.s2p/.s4p` export vector-fitted by scikit-rf into a SPICE subcircuit.
   Compare delay at Vmeas, overshoot, slew rate and settling at the receiver die (proposed tolerances: 2 ps or
   1 %, 20 mV, 5 %). KIBIS and kisi read the same file, so a shared misreading would go unnoticed – IBISCHK and
   the self-consistency test cover that.
5. **LTI eye:** PyBERT with the same Touchstone and linear ends.
6. **Measurement:** add a digital section to the planned test board (`validation/test_board.md`): a GPIO with an
   IBIS model driving lines with stubs and termination options, scope and TDR.
7. **Commercial:** HyperLynx on the example boards if a licence becomes available (already in the roadmap).

### 7.7 Risks
| Risk | Mitigation |
|---|---|
| Poor or inconsistent vendor models | IBISCHK, self-consistency test, show model warnings with results |
| Pin names vs pads, wrong selector (ODT/drive) | explicit mapping in kisi.json, per-pin coverage report, no silent defaults |
| Impulse-response artefacts (DC, causality, band limit) | DC point, window, checks; vector fitting with passivity later |
| Newton convergence with non-monotonic tables | damping, step halving, table cleaning with warnings |
| WASM run time on whole buses | workers, superposition, fewer taps, run per byte lane |
| DDR5 without DFE, no jitter/SSN | state it in every result; keep DDR5 indicative |
| Timing closure needs controller data | report the board share; leave timing to the controller vendor's tools |
| Licences of models and standards | local folders, hashes only, shippable fixtures in CI |
| Scope creep for a side project | milestones that stand alone; stop after M4 if DDR is not needed |

## 8. Decisions (maintainer, 22 Oct 2026)
1. **Variants:** the variant must be chosen in `<board>.kisi.json` (KiCad 10 variants from the board file, for older
   boards a `variants` section by sheet, references or BOM). If a board has variants – or forks in which all
   alternative parts are fitted – and none is chosen, kisi asks the user: the viewer shows a selector before it
   computes lanes, the CLI stops with the list of variants and asks for `--variant` or a kisi.json entry.
2. **Crates:** a separate `kisi-ibis` crate for the parser and buffer models; solver and network in `kisi-si`.
3. **First flow:** a generic single-ended flow (clocks, GPIO, RGMII) before DDR3/DDR4.
4. **DDR board:** Antmicro's open boards are fine. The DDR4 RDIMM tester has a DIMM socket, so a connector model and
   a module model (DIMM routing, EBD) would be needed as well; a board with discrete DRAM chips avoids that and is
   preferred as the first case.
5. **Standards:** limits from application notes, datasheets and other public sources with references. JEDEC
   documents the maintainer can download may be used locally for checking, never published.
6. **Private models in the browser:** kept in memory only.
7. **ngspice export:** first only as a validation script in `tools/`; a user-facing export (e.g. to debug a channel
   by adding a termination in SPICE) stays in mind for later.
8. **BER:** possibly for the simpler protocols without equalisation, with jitter as a user setting (it depends on
   the ICs); not for DDR5 or SerDes with equalisation.

## Sources
- [S1] IBIS Open Forum, specifications: IBIS 8.0 (ratified 5 Dec 2025), Touchstone 2.1, IBIS-ISS 1.0 –
  https://ibis.org/specs/ , https://ibis.org/ver8.0/
- [S2] Wikipedia, Input/output Buffer Information Specification (version history) –
  https://en.wikipedia.org/wiki/Input/output_Buffer_Information_Specification
- [S3] KiCad source, `eeschema/sim/kibis/kibis.cpp`, `ibis_parser.cpp` (BSD-3-Clause), `sim_library_ibis.h`,
  `sim_model_ibis.cpp`, QA models in `qa/data/eeschema/` – https://github.com/KiCad/kicad-source-mirror (read at
  commit 874060a, 22 Oct 2026); https://docs.kicad.org/doxygen/sim__model__ibis_8cpp_source.html
- [S4] KiCad source, `eeschema/sim/sim_model_tline.cpp` (T line, LTRA with constant RLGC), same repository
- [S5] IBIS 7.2 ratification – https://www.ibis.org/ver7.2
- [S6] IBIS Modeling Cookbook for IBIS 4.0 (2005) – https://www.ibis.org/cookbook/cookbook-v4.pdf
- [S7] ngspice user manual (version 47+) – https://ngspice.sourceforge.io/docs/ngspice-html-manual/manual.xhtml
- [S8] Wikipedia, Ngspice (licence) – https://en.wikipedia.org/wiki/Ngspice
- [S9] IBIS Open Forum, free tools (IBISCHK, TSCHK, S2IBIS) – https://ibis.org/tools/tools.htm ,
  https://ibis.org/ibischk8/
- [S10] scikit-rf, `skrf/vectorFitting.py` (`write_spice_subcircuit_s`, passivity) –
  https://github.com/scikit-rf/scikit-rf
- [S11] PyBERT (licence file) – https://github.com/capn-freako/PyBERT
- [S12] PyIBIS-AMI on PyPI (BSD) – https://pypi.org/project/PyIBIS-AMI/6.1.1 ; https://github.com/capn-freako/PyAMI
- [S13] PyBERT `utility/ibisami.py` – https://pybert.readthedocs.io/en/latest/_modules/pybert/utility/ibisami.html
- [S14] SignalIntegrity (GPL-3.0+) – https://github.com/Nubis-Communications/SignalIntegrity
- [S15] Qucs-S discussion on Xyce YLIN/Touchstone – https://github.com/ra3xdh/qucs_s/discussions/450
- [S16] Keysight W2309EP/ET DDR Bus Simulator data sheet –
  https://www.keysight.com/zz/en/assets/7018-04610/data-sheets/5992-0215.pdf
- [S17] Siemens HyperLynx DDR –
  https://www.siemens.com/en-us/products/pcb/hyperlynx/signal-integrity/ddrx-interface-design-hyperlynx-ddr/
- [S18] Cadence Sigrity SystemSI technology (EMA) –
  https://www.ema-eda.com/wp-content/uploads/2024/04/sigrity-systemsi-technology.pdf
- [S19] Cadence, Field Guide to DDR Signal Integrity Analysis (Semiconductor Engineering, 8 Jul 2026) –
  https://semiengineering.com/field-guide-to-ddr-signal-integrity-analysis/
- [S19b] Ansys forum, DDR4 compliance toolkit – https://innovationspace.ansys.com/forum/forums/topic/ansys-ddr4-compliance-toolkit
- [S20] J. E. Schutt-Ainé, R. Mittra, "Scattering Parameter Transient Analysis of Transmission Lines Loaded with
  Nonlinear Terminations", Univ. of Illinois report 87-2, March 1987 – https://apps.dtic.mil/sti/pdfs/ADA183718.pdf
- [S21] B. K. Casper, M. Haycock, R. Mooney, "An accurate and efficient analysis method for multi-Gb/s chip-to-chip
  signaling schemes", VLSI Circuits 2002 –
  https://people.engr.tamu.edu/spalermo/ecen689/link_modeling_pda_casper_vlsi_2002.pdf
- [S22] AMD, simulation models download page –
  https://www.amd.com/de/support/downloads/adaptive-socs-and-fpgas/device-models/simulation-models.html
- [S23] Intel, IBIS models for FPGA devices –
  https://intel.com/content/www/us/en/support/programmable/support-resources/board-layout/ibs-ibis-index.html
- [S24] AMD UG899, Exporting IBIS Models –
  https://docs.amd.com/r/2023.2-English/ug899-vivado-io-clock-planning/Exporting-IBIS-Models ;
  https://docs.amd.com/r/en-US/ug899-vivado-io-clock-planning/Generating-IBIS-Models
- [S25] IBIS Open Forum, IBIS model links – https://ibis.org/models/
- [S26] ADI, IBIS Modeling Part 1 –
  https://www.analog.com/en/resources/analog-dialogue/articles/ibis-modeling-part-1-why-ibis-modeling-is-critical-to-the-success-of-your-design.html
- [S26b] ADI, IBIS models – https://www.analog.com/en/design-center/simulation-models/ibis-models.html
- [S27] Download of a Micron DDR3 IBIS model (walk-through) –
  https://cadenhance.atlassian.net/wiki/spaces/PD/pages/617349138/Download+the+IBIS+Model+from+Micron+Website
- [S28] DigiKey forum, Micron LPDDR4 IBIS request – https://forum.digikey.com/t/ibis-model-for-lpddr4-from-micron/26268
- [S29] KiCad source, `pcbnew/pcb_io/kicad_sexpr/writers/pcb_writer_v10.cpp` (`formatVariants`, footprint
  variants), `pcb_io_kicad_sexpr.h` (format 20260101 "PCB variants with per-footprint overrides"), same repository
  as [S3]
- [S30] Microchip, SmartFusion2/IGLOO2 board design guidelines, IBIS models –
  https://onlinedocs.microchip.com/oxy/GUID-2952C8AA-A592-489E-8058-3FD06065EDDB-en-US-7/GUID-D8FFC717-CA45-48F6-966B-514BDFEA6162.html
- [S31] Crowd Supply, ThunderScope update (Rev 5 drops DDR3, M.2 Thunderbolt adapter) –
  https://crowdsupply.com/eevengers/thunderscope/updates/all-the-latest-progress-on-our-software-and-hardware
- [S32] JEDEC, JESD79 documents (revisions, dates, prices) – https://www.jedec.org/standards-documents/results/JESD79 ;
  https://www.jedec.org/standards-documents/results/jesd79-3 ; https://www.jedec.org/standards-documents/results/ddr5
- [S33] Micron TN-41-13, DDR3 Point-to-Point Design Support, Rev. B 08/13 –
  https://cdn.hackaday.io/files/19354828041536/tn4113_ddr3_point_to_point_design.pdf
- [S34] TI SPRABI1D, DDR3 Design Requirements for KeyStone Devices, fly-by signals –
  https://ti.com/document-viewer/lit/html/SPRABI1D/GUID-D68B2334-EBBE-474C-9063-8A4D39B03BA6
- [S35] R. Wolff (Micron), Modeling DDR3 with IBIS, IBIS Summit 2008 – https://www.ibis.org/summits/feb08/wolff.pdf
- [S36] R. Wolff (Micron), DDR4 IBIS Power Integrity Simulation, European IBIS Summit 2012 –
  https://www.ibis.org/summits/may12/wolff.pdf
- [S37] AMD UG586, 7 Series FPGAs Memory Interface Solutions – https://docs.amd.com/r/en-US/ug586_7Series_MIS
  *(not read in detail)*
- [S38] KiCad 10.0.0 release (20 Mar 2026) – https://www.kicad.org/blog/2026/03/Version-10.0.0-Released/
- [S39] KiCad forum, KiCad 10 variants – https://forum.kicad.info/t/new-kicad-10-variants/66223
- [S40] KiVar (MIT) – https://github.com/markh-de/KiVar
- [S41] ThunderScope Rev5.3 fabrication readme and BOMs (`Hardware/Thunderscope_Rev5.3/Outputs/`) –
  https://hub.allspice.io/AllSpiceMirrors/ThunderScope/src/branch/master/Hardware/Thunderscope_Rev5.3/Outputs/Fab_Readme.txt ;
  repository https://github.com/EEVengers/ThunderScope (read with git at `df2b845`; web pages not fetched)
- [S42] Antmicro, LPDDR4 test board (Kintex-7, KiCad, Apache-2.0) – https://github.com/antmicro/lpddr4-test-board
- [S43] Antmicro, Data Center RDIMM DDR4 Tester (Kintex-7, KiCad, Apache-2.0) –
  https://github.com/antmicro/data-center-dram-tester

**Not read for this document:** the IBIS 8.0 specification text (the PDF could not be fetched; statements about
keyword semantics rely on the KiCad parser [S3], the Cookbook [S6] and earlier knowledge), the GitHub web pages
of ThunderScope (repository read with git instead), the KiCad 7 release notes, and the JEDEC standards themselves.
