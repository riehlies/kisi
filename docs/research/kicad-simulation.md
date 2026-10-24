# KiCad's simulator, KIBIS and kisi – research note

Status: research note (23 October 2026), no implementation. Question: instead of building IBIS simulation into
kisi's viewer (branch `feat/ibis-parser`, on hold), can kisi use what KiCad already has – hand its channel models
to KiCad's simulator, or configure the IBIS models in KiCad? Builds on `channel-simulation-ibis.md` (stages M0–M6).
KiCad facts come mainly from the KiCad source (GitHub mirror of the GitLab repository, branches 7.0–10.0 and
master at the commits in [K0]); they are **code reading, not tried in a running KiCad** (no KiCad binary and no
builds in this session). Claims that could not be checked are marked *(not verified)*. Sources at the end.

## Summary
- **KiCad's simulator** is ngspice with a model layer in symbol fields (`Sim.Device`, `Sim.Type`, `Sim.Pins`,
  `Sim.Params`, `Sim.Library`, `Sim.Name`, plus `Sim.Ibis.*`). Analyses: OP, DC, AC, transient, noise,
  pole-zero, S-parameters (KiCad 8+, with `portnum`/`z0` on voltage sources), FFT. Text directives on the sheet
  (`.include`, `.tran`, `.control … wrdata …`) go into the netlist; `kicad-cli sch export netlist --format spice`
  produces the netlist without the GUI. No eye diagram, no SI-specific measurements.
- **KIBIS** (KiCad's IBIS layer, BSD-3, since KiCad 7) models **one buffer per symbol**: a 2-pin (GND, IN/OUT) or
  3-pin (GND, +, −) behavioural subcircuit with an ideal internal supply, lumped R/L/C_pin, C_comp, clamps and
  I-V tables, and Ku/Kd switching coefficients that KIBIS computes with ngspice from 0, 1 or 2 waveform pairs.
  Drivers: DC (hi-Z/low/high), rectangular, PRBS7. One corner switch (`vcc`) selects typ/min/max for the supply and
  all tables; package R/L/C have their own corner switches; C_comp is always typ.
- **KIBIS limits that matter for real vendor files:** files with `[IBIS Ver]` above 7.0 are rejected; an unknown
  keyword (e.g. `[Driver Schedule]`, `[Die Supply Pads]`) or one invalid model invalidates the whole file;
  `[Package Model]` is parsed but not used; `[Diff Pin]` only marks a pin as differential (no Vdiff, no tdelay; the
  N leg reuses the P pin's parasitics); `[Rgnd]/[Rpower]/[Rac]/[Cac]` terminators are not emitted; Series models
  only in master (after 10.0). A real 600-ball FPGA symbol cannot carry per-pin IBIS models – KiCad's IBIS
  workflow is a separate simulation schematic with driver, interconnect and receiver symbols.
- **Transmission lines:** KiCad's `TLINE` device becomes an ngspice LTRA line with constant RLGC (or Z0/td).
  ngspice has T, LTRA, URC, TXL and CPL (coupled lines) – all with frequency-independent per-unit-length
  parameters – and **no Touchstone/N-port element and no W-element** (in ngspice `W` is a current-controlled
  switch). kisi's frequency-dependent channels must therefore be exported as SPICE subcircuits: either a physical
  lumped ladder (L, C with small R(f)/G(f) networks per section, vias and parts as in kisi's cascade) or a
  vector-fitted macromodel of the lane's S-parameters (scikit-rf can write one).
- **KiCad 9/10 for SI:** no SI simulation, but KiCad 10 has **tuning profiles** in the `.kicad_pro` (target
  impedance, per-layer width/gap/delay from analytic microstrip/stripline formulas, via delays) referenced by net
  classes, time-domain tuning, delay columns in the net inspector and pad die delays. kisi does not read them yet.
  Master (KiCad 11 development) adds a frequency-dependent dielectric model (Djordjevic-Sarkar) to the stackup.
- **API:** the IPC API covers only the PCB editor in KiCad 9/10 (KiCad 10 can inject DRC markers); the schematic
  API, schematic plugins and a headless `kicad-cli api-server` are KiCad 11. Writing Sim fields into the user's
  schematic files is possible but invasive, and kisi's rule is to never write KiCad files.
- **Recommendation (section 7):** keep IBIS out of kisi's viewer. Add a **"simulate this lane in KiCad"
  export** (option A, configured by B in kisi.json): a lane model as a SPICE subcircuit plus a small, separate,
  ready-to-run KiCad simulation project – with the interface profile's linear ends by default (always runs) and
  KIBIS drivers/receivers where kisi.json names IBIS models. Read KiCad 10 tuning profiles (D). Bring KiCad's
  waveforms back into kisi's one verdict function later if needed (F). kisi's own transient (C = M3) only if the
  KiCad path proves too limited; the export then serves as its reference harness. About 12–20 focused days for
  A+B (CLI first, one download button in the viewer), 2–4 for D.

## 1. KiCad's simulator (question 1)

### 1.1 Engine and analyses
KiCad runs ngspice as a shared library. Analyses offered by the simulation command dialog per release branch
(`dialog_sim_command.cpp`, `sim_types.h` [K6]):

| KiCad | Analyses in the dialog | Notes |
|---|---|---|
| 7.0 | OP, DC sweep, AC, transient | new model editor and `Sim.*` fields (format 20221206, [K8]); KIBIS present [K3] |
| 8.0 | + pole-zero, noise, S-parameters (`.sp`), FFT | release notes: "four new simulation types … (pole-zero, noise, S-parameter, and FFT)", new plot UI, cursors and measurements [K17]; transfer function, sensitivity and distortion exist only as enum values |
| 9.0 | same | sheets can be excluded from simulation (sheet attributes, format 20240602 [K8]); embedded files (models can live in the schematic) [K18][K10] |
| 10.0 | same | "improvements to the simulator plotting tools" [K19] |
| master (11 dev) | same | Smith chart for S-parameters; `Sim.Decomposition` for multi-unit symbols; IBIS Series models [K1][K3] |

S-parameter ports are voltage sources with the parameters `portnum` and `z0` [K9]; ngspice has had `.sp` since
ngspice-37 (May 2022) [N2]. The simulator panel offers min/max/avg/RMS/peak-to-peak measurements on traces
[K9], CSV export of plots, and waits for `wrdata` output files to be written [K9]. There is no eye diagram
(no "eye" in the plot code of 9.0, 10.0 or master) and nothing SI-specific (overshoot, ringback, masks).

### 1.2 Simulation model fields
Defined in `sim_model.h` [K1]; IBIS fields in `sim_library_ibis.h` [K2]. Stored as ordinary symbol properties.

| Field | Content | Example |
|---|---|---|
| `Sim.Device` | device: R, C, L, K, TLINE, SW, D, NPN/PNP, NJFET/PJFET, NMOS/PMOS, NMES/PMES, V, I, E/F/G/H, **IBIS**, SUBCKT, XSPICE, SPICE (raw) | `TLINE` |
| `Sim.Type` | subtype: PULSE, PWL, SIN, … for sources; `RLGC` for TLINE; `DEVICE`, `DCDRIVER`, `RECTDRIVER`, `PRBSDRIVER` for IBIS; model levels (BSIM4, …) | `RLGC` |
| `Sim.Pins` | `symbolPin=modelPin` pairs, space separated [K1] | `1=GND 2=IN/OUT` |
| `Sim.Params` | `name=value` pairs | `len=1 r=0 l=1.25m g=0 c=500n` |
| `Sim.Library` | model file (`.lib`, `.sub`, `.ibs`, …), resolved against the project, environment variables, `SPICE_LIB_DIR` and embedded files [K10] | `models/fpga.ibs` |
| `Sim.Name` | model or subcircuit name; for IBIS the `[Component]` | `XC7A100T` *(illustrative)* |
| `Sim.Ibis.Pin`, `Sim.Ibis.Model`, `Sim.Ibis.Diff` | IBIS pin, model (any entry of the pin's `[Model Selector]`), `1` for differential | `AB12`, `LVDS_25`, `1` |
| `Sim.NodesFormat` (9.0+) | node format for XSPICE code models | |
| `Sim.Decomposition` (master) | whole device or repeat per unit, for multi-unit symbols [K1][K5] | |

Legacy fields (`Spice_Primitive`, `Spice_Model`, `Spice_Node_Sequence`, `Spice_Lib_File`) are converted, and
`Sim.Enable` became the `exclude_from_sim` attribute in format 20230808 [K1][K8]. Without fields, R, C, L and
sources are inferred from reference prefix and value [K1]. In the file (KiCad's own QA schematic [K7]):
```
(property "Sim.Device" "TLINE" (at 146.05 114.3 0) (effects (font (size 1.27 1.27)) hide))
(property "Sim.Type" "RLGC" (at 146.05 114.3 0) (effects (font (size 1.27 1.27)) hide))
(property "Sim.Params" "len=1 r=0 l=1.25m g=0 c=500n" (at 146.05 114.3 0) (effects (font (size 1.27 1.27)) hide))
```
Symbol fields are copied to the footprint on "Update PCB from Schematic" when field updates are enabled [K16], so
`Sim.*` fields can appear in the `.kicad_pcb` that kisi reads (how the 0.1.0 experiment found them).

### 1.3 Directives, libraries, headless use
- Text items on the sheet whose lines start with `.AC`, `.CONTROL`, `.INCLUDE`, `.LIB`, `.MEAS`, `.MODEL`,
  `.PARAM`, `.SAVE`, `.PROBE`, `.SP`, `.SUBCKT`, `.TRAN`, … are passed to the netlist [K5]; an analysis command on
  the sheet becomes the simulator tab's command (KiCad asks when it changes) [K9]. Nets named `0` or `GND` are
  SPICE ground [K5].
- Each model contributes its `.include` files: SPICE libraries as they are, IBIS models as a generated file in
  the user cache (`ibis/<ref>.cache`) [K2][K5].
- `kicad-cli sch export netlist --format spice|spicemodel` (10.0) writes the SPICE netlist from a schematic
  [K11]; with `ngspice -b` this gives a GUI-free pipeline *(not tried; whether the CLI path also writes the KIBIS
  cache files is not verified)*.
- Schematic format versions: 7.0 = 20230121, 8.0 = 20231120, 9.0 = 20250114, 10.0 = 20260306 [K8]. Older KiCad
  cannot open newer files, so a generated file should use the oldest format that has what it needs.

## 2. IBIS in KiCad: KIBIS (question 2)

### 2.1 Attaching a model
KIBIS (`eeschema/sim/kibis`, © 2022 Fabien Corona, BSD-3-Clause [K3]; presented at FOSDEM 2022 [K24]) is in
the 7.0 branch and all later ones. The `Simulation_SPICE` library has the symbols IBIS_DEVICE, IBIS_DEVICE_DIFF,
IBIS_DRIVER and IBIS_DRIVER_DIFF, "intended to be used with an external IBIS model" [K22]. The model dialog
writes [K1][K2]:

| What | Field / parameter | Values |
|---|---|---|
| file, component, pin, model | `Sim.Library`, `Sim.Name`, `Sim.Ibis.Pin`, `Sim.Ibis.Model` | the pin's model list includes all entries of its `[Model Selector]`; the choice is explicit |
| kind | `Sim.Device=IBIS`, `Sim.Type` | `DEVICE` (receiver: C_comp, clamps), `DCDRIVER`, `RECTDRIVER`, `PRBSDRIVER` |
| differential | `Sim.Ibis.Diff=1` | only offered when the pin is in `[Diff Pin]`; pins become GND, +, − |
| corner | `Sim.Params`: `vcc` | `typ`/`min`/`max` – selects `[Voltage Range]` **and** the corner of all I-V and V-t tables |
| package corner | `rpin`, `lpin`, `cpin` | `typ`/`min`/`max` each; C_comp is fixed at typ (the `ccomp` switch is commented out) |
| DC driver | `dc` | `hi-Z`, `low`, `high` |
| rectangular driver | `ton`, `toff`, `td`, `n` | on/off time, delay, cycles |
| PRBS driver | `f0`, `td`, `n` | bit rate, delay, bits; PRBS7 (x⁷+x⁶+1, fixed seed). `td` is read but not used by the PRBS generator *(code reading)* |

### 2.2 What ngspice receives
Per symbol a `.SUBCKT` with ports `GND PIN` (or `GND PIN_P PIN_N`) [K3]:
- `RPIN`, `LPIN`, `CPIN` from `[Pin]`, else `[Package]` – one lumped section;
- an ideal internal supply `VPWR POWER GND <[Voltage Range]>` – no power pins, no SSN;
- `C_comp`, GND/POWER clamps and pull-up/pull-down as I-V tables; submodel clamps;
- for drivers, PWL sources `KU` and `KD` and B-sources that scale the pull-up and pull-down currents. Ku/Kd come
  from a separate ngspice run against the `[Rising/Falling Waveform]` fixtures: 2 waveform pairs → solved from both
  (two equations, two unknowns), 1 pair → warning "reduced accuracy", none → `[Ramp]` with the 20–80 % time stretched by 1/0.6 ("poor
  accuracy"); more than 2 pairs → the first two; `R/L/C_dut` → warning "not supported" [K3];
- the differential driver is two single-ended drivers of the **same** pin with inverted stimulus [K3].

### 2.3 Limits
| IBIS feature | KIBIS (10.0 and master) [K3] |
|---|---|
| `[IBIS Ver]` | > 7.0 rejected ("The parser does not handle this IBIS version"), all releases 7.0–10.0 |
| unknown keywords | error, parsing stops, file invalid; not handled e.g. `[Driver Schedule]`, `[Die Supply Pads]` (commented "Not supported yet") |
| invalid model or component | the whole file is invalid (`m_valid` is the AND of all) |
| `[Package Model]` / `.pkg` | parsed (matrices), not used for SPICE |
| `[Diff Pin]` | marks diff-capable pins; Vdiff and tdelay unused; N leg uses the P pin's R/L/C |
| `[Model Selector]` | all entries offered; user picks one model |
| Model types | drivers: Output, I/O, 3-state, open drain/sink/source, Output/I/O/3-state ECL; devices (receivers): Input, I/O variants, I/O_ECL; Terminator and Input_ECL rejected ("Invalid model type for a device"); Series/Series_switch only in master |
| `[Rgnd]`, `[Rpower]`, `[Rac]`, `[Cac]` | parsed, not emitted – ODT given this way is lost (ODT as clamp curves works) |
| `[ISSO PU/PD]`, `[Composite Current]`, `[GND/POWER Pulse]` | parsed, not used |
| `[Algorithmic Model]` (AMI) | parsed, not used |
| `[Receiver Thresholds]`, Vinh/Vinl | read into the model; not used in the SPICE output (no threshold measurements) |

### 2.4 What users run into
The forum shows the expectation gap: users try to put an IBIS file on a vendor's IC symbol and get "there is no
simulation available in the library"; the answer is that IBIS needs "a driver, the interconnect, a receiver",
with driver/receiver symbols from `Simulation_SPICE` and the interconnect supplied by the user [K23]. That matches
the code: one IBIS pin per symbol, and the IC's other pins would be unconnected in SPICE. **Configuring IBIS on
the real ICs of a design schematic is therefore not a workable path in KiCad 7–10**; a separate simulation sheet
or project is.

## 3. Transmission lines and S-parameters (question 3)

### 3.1 In KiCad
`Sim.Device=TLINE` with either `z0`, `td` or `RLGC` (`r`, `l`, `g`, `c`, `len`, per unit length) – **both** are
written as ngspice `O` elements with an `LTRA` model (for Z0/td: l = td·z0, c = td/z0, len = 1) in 8.0, 9.0, 10.0
and master; the old `T` mapping is commented out [K1][K4]. Four pins (in+, in−, out+, out−). No coupled-line
device; coupled lines need a raw SPICE element or a subcircuit.

### 3.2 In ngspice (manual 47+, March 2026; ngspice 47 released 11 August 2026 [N1][N3])
| Element | Model | Frequency dependence | Use for kisi |
|---|---|---|---|
| `T` | lossless line (Z0, TD) | none | ideal reference only |
| `O` | `LTRA` lossy line | constant R, L, G, C per length | what KiCad's TLINE gives; wrong loss away from one frequency |
| `U` | `URC` distributed RC line | – | no |
| `Y` | `TXL` single lossy line (KSPICE, recursive convolution from Padé approximations [N4]) | constant RLGC *(manual section 6.4.1 not re-read)* | faster LTRA |
| `P` | `CPL` coupled multiconductor line (KSPICE) | constant RLGC matrices *(manual section 6.4.2 not re-read)* | pairs with coupling, constant loss |
| `X` subcircuit | anything built from R, L, C, K, controlled sources | as synthesised | **the way to export kisi's channels** |
| `A` XSPICE | code models; Laplace/`xfer` examples since ngspice-39 [N2] | tabulated or rational H(s) | one-way transfer functions, not an N-port; loading an `.s2p` this way has DC-operating-point trouble (xschem issue, 2026) [N6] |

There is **no W-element** (ngspice's `W` is a current-controlled switch) and **no element that reads Touchstone
files** in the element table [N1]. `ngmultidec` builds coupled lines from uncoupled lossy lines, but only for
symmetric, nearest-neighbour coupling [N5]. Elsewhere: Xyce has a YLIN Touchstone device "mostly for Harmonic
Balance", and Qucs-S does not implement it (January 2024) [T4].

### 3.3 What a kisi lane model for KiCad would look like
kisi's channel is a cascade of segments – lines with R(f), L, G(f), C per cross-section (skin effect, roughness,
Djordjevic-Sarkar), vias with (L, C) or the physics-based ladder, series and shunt parts – evaluated as ABCD
matrices per frequency (`kisi_si::rf::channel`). It already writes `.s2p` and `.s4p` per net. Options for SPICE:

| Model | How | Pros | Cons |
|---|---|---|---|
| a. LTRA/TXL per segment | constant RLGC at one frequency (e.g. Nyquist), KiCad TLINE symbols | readable in KiCad, few elements | loss only right near that frequency; DC and edge shape wrong |
| b. **Physical lumped ladder** | each segment cut into sections ≤ λ/20 at f_max (≈ 0.4 mm at 20 GHz for ε_eff 3.5); per section L, C, a small R–L network for R(f) and R–C branches for G(f), fitted once per cross-section over the band; vias as their ladders; parts with their parasitics; pairs as coupled sections (L, C per line, K and C_m between them) | passive and causal by construction, follows kisi's own structure (a user can add a termination at a via), no fitting failures, one generator for all lanes | ~1000–2000 elements per 100 mm lane *(estimate)*; ngspice step size follows section delays (picoseconds); coupling in pairs needs care |
| c. Vector-fitted macromodel | rational fit of the lane's S-parameters, realised as R, C and controlled sources (scikit-rf `write_spice_subcircuit_s`, "compatible with LTspice, ngspice, Xyce" [T1]; snp2le does the same for ngspice [T2]) | compact, exact within the band, also for Touchstone blocks (connectors, cables behind a connector) | passivity enforcement and model order to manage, opaque, a Rust implementation is real work (or Python in `tools/`) |
| d. CPL for pairs | constant RLGC matrices | coupling in one element | same loss problem as a |

Whatever the model, kisi can check its own export: run ngspice `.sp` on the subcircuit and compare S21/S11 with
kisi's ABCD result (a KiCad user can do the same with the S-parameter analysis). In KiCad, the model is a symbol
with `Sim.Device=SUBCKT`, `Sim.Library=<lane>.lib`, `Sim.Name=<subckt>`, `Sim.Pins=1=a 2=b …`.

## 4. KiCad 9/10 news for SI, the API and file editing (question 4)

### 4.1 SI-related features
- **No built-in SI simulation** in 9.0 or 10.0 (no channel, eye or IBIS beyond KIBIS) [K18][K19].
- **KiCad 10 time-domain tuning:** "support for defining time-domain constraints instead of just length
  constraints" and "Tuning Profiles, which let users define per-layer routing parameters" [K19]. In the source:
  `tuning_profiles_impedance_geometric` in the project file, each profile with name, single/differential,
  `target_impedance`, `enable_time_domain_tuning`, per signal layer the reference layers, `width`,
  `diff_pair_gap`, `delay`, plus `via_prop_delay` and via overrides; width, gap or delay are calculated in the
  dialog with analytic (coupled) microstrip/stripline formulas from the stackup; net classes carry a
  `tuning_profile` (net settings schema 5) [K12].
- **Net inspector (10.0):** columns Total, Track, Via and Die Delay [K13]; pads carry `die_length` (9.0) and
  `die_delay` (10.0) [K13].
- **Master (KiCad 11 development):** stackup dielectrics get `spec_frequency` and
  `dielectric_model djordjevic_sarkar`, used by the tuning-profile calculations (forum, May 2026) [K15][K21].
- **PCB Calculator:** transmission-line panel (microstrip, coupled microstrip, stripline, coplanar, coax, …) [K13];
  9.0 added tolerance calculations in the regulator panel [K18].
- Earlier: KiCad 8 overhauled the length-tuning tools [K17]; 9.0 added diff-pair skew DRC and the net inspector
  panel [K18].

### 4.2 IPC API and kicad-python
- KiCad 9 introduced an IPC (protobuf) API for the board editor that "will eventually replace the current
  Python wrapper interface" and "will be extended to support the schematic editor during version 10
  development" [K18]. In fact 9.0 and 10.0 ship only a few schematic types (lines, text, labels) and no schematic
  commands; master has
  `GetSchematicHierarchy`, `GetSchematicNetlist`, `PlaceSymbolFromLibrary` and symbol instances with
  `user_fields`, used with the generic create/update/delete item commands [K14].
- The developer docs: KiCad 9 and 10 support IPC plugins only in the PCB editor; schematic plugins and headless
  operation (`kicad-cli api-server`) come with KiCad 11; in 9/10 the API needs a running GUI; the official Python
  wrapper is kicad-python [K20][K21].
- Board API in 10.0: stackup get/update, pads with `pad_to_die_length`/`pad_to_die_delay`, items by net or net
  class, and `InjectDrcError` (severity, message, position, items) – an external tool can place DRC markers on
  the board [K14]. Not in 9.0.
- **So:** an external tool cannot write `Sim.*` fields through the API before KiCad 11; KiCad 11 is not released
  (KiCad releases came in February/March in 2024–2026 [K17][K18][K19], so early 2027 is likely *(inference)*).

### 4.3 Editing `.kicad_sch` files directly (fallback)
Possible – the format is plain S-expressions and `Sim.*` fields are ordinary properties – but risky in the user's
design: format versions (a file written for 10.0 does not open in 8.0); instance data per sheet path in
hierarchies; KiCad 10 variants (fields can differ per variant); the cached `lib_symbols` must match the symbols;
an open editor holding the old file; noisy diffs in the user's version control; and `Sim.Device=IBIS` on a real IC
changes what every simulation of the design netlists for it (section 2.4). It also breaks kisi's rule "kisi never
writes into the KiCad files" (README). Writing a **new, separate** simulation project avoids all of these.

## 5. Other open tools (question 5)
| Tool | Licence | What | Relation to kisi |
|---|---|---|---|
| ngspice | BSD-3 [N3] | the engine in KiCad; `.sp`, OSDI/Verilog-A, XSPICE | runs kisi-exported subcircuits headless (`ngspice -b`) |
| KIBIS (in KiCad) | BSD-3 [K3] | IBIS → behavioural SPICE | the IBIS half of option A; logic portable to Rust with attribution |
| scikit-rf | BSD-3 | Touchstone I/O, vector fitting with passivity test/enforcement, SPICE subcircuit export [T1] | validation of kisi's Touchstone and of option b/c exports; model c for Touchstone blocks |
| snp2le | Apache-2.0, 0.1.14 (12 Oct 2026) | Touchstone → lumped netlists for ngspice and VACASK via scikit-rf VF [T2] | shows the VF route works with ngspice |
| PyBERT / PyIBIS-AMI | BSD-style | LTI serial-link simulator with GUI, IBIS-AMI, Touchstone channels [T3] | cross-check of kisi's linear eye |
| SignalIntegrity (Pupalaikis) | GPL-3.0+ | S-parameter networks, simulator, eyes ([S14] in the concept) | network/eye cross-checks |
| Xyce | open source | YLIN Touchstone device, mainly for harmonic balance [T4] | not needed |
| Qucs-S | GPL | GUI for ngspice/Xyce/Qucsator; Xyce Touchstone device not implemented (2024) [T4] | not needed |
| openEMS | GPL | 3D FDTD, Touchstone output | references for vias/connectors (already used for kisi-fdtd checks) |
| IBISCHK | free binaries ([S9] in the concept) | golden IBIS parser | check vendor files before KIBIS/kisi |

## 6. Integration options for kisi
Effort in focused days; M-stages as in `channel-simulation-ibis.md` 7.5.

### A. Export a lane as a ready KiCad simulation (kisi → KiCad)
**What.** `kisi export-sim <board> --lane NAME [--out DIR]` (and one "Simulate in KiCad…" download in the
viewer's lane detail) writes a folder:
- `<lane>.lib` – the lane as a subcircuit (model b; pairs as a coupled 4-port), with a header naming board,
  variant, nets, ends, frequency range, model and kisi version;
- `<lane>.kicad_pro` / `<lane>.kicad_sch` – a one-sheet project: driver, channel symbol (`SUBCKT`), receiver,
  ground, net labels, a `.tran` directive sized to the profile (e.g. PRBS7 at its bit rate), `.save` of pin
  nodes and a `wrdata` of the receiver waveform; kisi's own minimal symbols embedded in `lib_symbols` (no
  dependency on, or licence question about, KiCad's library); written in the KiCad 8 format so that 8, 9, 10 and
  11 open it;
- ends: by default the **interface profile's linear ends** (V source with a PWL PRBS7 at the profile's swing, rise
  time and R; RC receiver) – runs everywhere and reproduces kisi's linear eye; where kisi.json maps an IC to an
  IBIS model (option B), **KIBIS symbols** with `Sim.Library/Name/Ibis.Pin/Ibis.Model/Params` filled in; connector
  ends as their termination (R/C) – a Touchstone channel behind the connector only via model c;
- `README.txt` – three steps (open, check models, run) and what the model contains.

**User experience.** In kisi nothing about IBIS needs to be understood or fixed: the export always succeeds. In
KiCad the user sees waveforms at the receiver pin, can swap ends or corners in KiCad's own dialog (where KIBIS
errors appear, next to the choice that caused them), add a termination, or run the S-parameter analysis on the
channel to see that it matches kisi. No eye diagram and no verdict in KiCad.

**Effort.** Lane → subcircuit 4–6, pairs 2–3, self-check (`.sp` vs kisi S-parameters, script in `tools/`, needs
ngspice) 1–2, project writer 3–5 (with manual tests in KiCad 8/9/10), CLI, viewer download (a store-only zip) and
docs 2–4: **12–20**.

**Risks.** KIBIS limits (section 2.3) – many vendor files may fail in KiCad, and kisi's parser can only warn
beforehand; ngspice run time and convergence with ~2000 elements plus B-sources (to measure); keeping up with
KiCad's file format; results stay in KiCad unless F is added; the browser does not know where the user's IBIS files
are (relative paths and README; the CLI knows); multi-drop needs the N-port of M2.

**M-stages.** A new stage between M1 and M2. It replaces the manual set-up of validation step 4 in the concept
(7.6) and becomes the reference harness for M3. It does not need M2–M4 for point-to-point lanes.

### B. kisi writes or proposes the IBIS fields (configuration)
**What.** The `ibis` section of kisi.json (from M1) in KiCad's vocabulary – file, `[Component]`, pin, model,
corner, differential – so that A fills the KIBIS symbols; `kisi ibis <file>` and `config-check` report whether
KiCad will accept a file (version ≤ 7.0, no unsupported keywords, Terminator or Series models, `[Package Model]`
ignored, ODT as Rgnd/Rpower ignored); `Sim.*` fields already present on footprints are read as a second source.
Writing fields into the user's design schematic is **not** recommended (sections 2.4 and 4.3). On its own, B gives
kisi nothing: the linear eye does not use IBIS data – B is the configuration half of A.

**User experience.** One place for model choices (kisi.json, editable by the kisi-prepare skill); the viewer
shows only "IBIS: from kisi.json" in the export, no loader or pin-mapping UI. **Effort** 2–4 (mostly in
`feat/ibis-parser` already). **Risks** small; pin names vs pads (from M1). **M-stages** part of M1.

### C. kisi's own transient with IBIS buffers (M3, as in the concept)
**What.** N-port (M2), convolution plus Newton at the die nodes (M3), measurements and `judge` (M4), in WASM.
**User experience** best when it works – verdicts in volts and picoseconds in the viewer, no KiCad needed – but
it brings back the viewer complexity that stopped `feat/ibis-parser` (model loading, pin mapping, selector and
corner choice, model errors). **Effort** 26–39 (M2–M4) plus viewer work. **Risks** as in the concept (7.7).
**M-stages** M2–M4.

### D. Read KiCad 10 tuning profiles, give back delays (better, small)
**What.** Read `tuning_profiles_impedance_geometric` and the net classes' `tuning_profile` from the `.kicad_pro`:
target impedance and single/diff per net class, without naming conventions (`50Z`) – precedence: profile >
class name, below kisi.json. Report kisi's field-solver impedance and delay per layer and width next to the
profile's analytic values ("profile says 6.6 ps/mm, field solver 6.9 ps/mm") so the user can correct the profile
in KiCad; later read master's `spec_frequency`/`dielectric_model`. **User experience** zero set-up on KiCad 10
boards that use profiles. **Effort** 2–4. **Risks** profile schema changes between 10 and 11. **M-stages**
independent (impedance part).

### E. GUI-free round trip for validation (tooling)
**What.** A's project → `kicad-cli sch export netlist --format spice` → `ngspice -b` → waveforms, in `tools/`
and optionally CI with a KiCad container; plain ngspice decks for the linear-ends case need no KiCad. **Effort**
2–3. **Risks** KiCad in CI, KIBIS cache paths in the CLI netlist *(not verified)*. **M-stages** validation of A,
later of C (concept 7.6 step 4).

### F. Bring KiCad's waveforms back into kisi's verdict
**What.** The generated project writes the receiver waveform (`wrdata`, or KiCad's CSV export); dropping that file
on the lane in kisi (or `kisi measure <lane> --waveform FILE`) runs the M4 measurements – overshoot, ringback,
slew, eye height/width in V/ps against the profile's levels – in `kisi_si::profile::judge`. The nonlinear
simulation stays in KiCad, the one verdict logic in kisi. **Effort** 5–8 (subset of M4). **Risks** matching file
to lane (the header carries a lane id and hash), PRBS length for eyes. **M-stages** M4 without M3.

### G. kisi results as DRC markers in KiCad 10 (optional)
**What.** A small helper (Python with kicad-python, or a PCB-editor plugin) reads kisi's report JSON and calls
`InjectDrcError` with the lane's items, so failing lanes show up in KiCad's DRC panel. **Effort** 2–4. **Risks**
running KiCad needed, Python on the user's side, wrapper support for this call not checked. Fits the roadmap item
"live link to KiCad".

### Comparison
| Option | Effort (d) | kisi viewer complexity | Nonlinear IBIS | Verdict in kisi | Needs KiCad |
|---|---|---|---|---|---|
| A export (+B config) | 12–20 (+2–4) | one download button | yes, in KiCad (KIBIS limits) | linear only (F adds nonlinear) | to simulate |
| C own transient | 26–39 + UI | high | yes, in kisi | yes | no |
| D tuning profiles | 2–4 | none | – | – | no |
| E round trip | 2–3 | none | validation | – | CI |
| F waveform import | 5–8 | one drop target | from KiCad | yes | to simulate |
| G DRC markers | 2–4 | none | – | shown in KiCad | yes (10.0) |

## 7. Recommendation
A clean, self-explanatory path keeps two clearly separated layers – **kisi: fast linear board checks with one
verdict; KiCad: nonlinear simulation with IBIS** – and a single bridge between them:
1. **Main stays IBIS-free in the viewer.** Do not merge the viewer part of `feat/ibis-parser`. The parser,
   `kisi ibis` (model inspector plus a "will KiCad accept this file?" check) and the kisi.json `ibis` mapping can
   be merged as CLI-only features once they use KiCad's vocabulary (B).
2. **Build A as `kisi export-sim` in the CLI first**, with the physical lumped model (b), profile ends by default
   and KIBIS ends where kisi.json maps them; validate each export with ngspice `.sp` against kisi's S-parameters
   (E). Then one "Simulate in KiCad…" download in the lane detail. kisi's result rows keep saying "linear ends
   from the interface profile"; the export's README says what KiCad adds.
3. **Add D** (KiCad 10 tuning profiles) – small, useful on every KiCad 10 board, and the most direct "give back
   to KiCad" today.
4. **Decide on F after using A on real boards** (the ThunderScope LVDS/clock nets, a DDR test board): if
   waveforms in KiCad are enough, stop; if verdicts in volts are wanted, add F (no own transient needed).
5. **C only if KIBIS proves too limited** (IBIS > 7.0, `[Package Model]`, ODT terminators, DDR flows), with A/E
   as its reference. Write-back into KiCad (fields via the KiCad 11 schematic API, DRC markers via G) can wait.

This answers the maintainer's idea: **give the channel models to KiCad – yes (A); configure IBIS in the JSON –
yes (B, feeding A); configure IBIS in the design schematic – no** (one buffer per symbol, invasive edits); a
generated simulation schematic is what KiCad's IBIS workflow expects anyway.

## 8. Open questions for the maintainer
1. Is "waveforms in KiCad, verdicts in kisi only for the linear check" acceptable for a first version, or must
   IBIS results reach `judge` from the start (then F, or C)?
2. Lane model: physical lumped ladder (readable, editable, larger) or vector-fitted macromodel (compact, opaque)
   – or the ladder plus a scikit-rf script for Touchstone blocks?
3. Oldest KiCad the generated project must open: 8 (format 20231120) or 9?
4. `feat/ibis-parser`: merge parser, `kisi ibis` and the kisi.json mapping as CLI-only, or keep everything parked?
5. KiCad 10 tuning profiles as impedance targets: above net-class names and below kisi.json – agreed?
6. Is a KiCad container in CI acceptable for the round trip (E), or ngspice only?
7. Which first test case: a ThunderScope LVDS or clock lane with public IBIS models (FIN1019, HMCAD1520), or wait
   for the DDR board?
8. Write-back into KiCad (DRC markers in 10.0, fields via the KiCad 11 schematic API): wanted at all?

## Sources
KiCad source (GitHub mirror `KiCad/kicad-source-mirror` of `gitlab.com/kicad/code/kicad`), read with git on
23 Oct 2026:
- [K0] commits: master `ba5ed30c1bee` (22 Oct 2026), 10.0 `99157a13013a` (21 Oct 2026), 9.0 `ce707d0a6c87`,
  8.0 `d7388e2a70fc`, 7.0 `8ad2755ae319`; latest tags 10.0.7 and 10.0.8-rc1 –
  https://github.com/KiCad/kicad-source-mirror
- [K1] `eeschema/sim/sim_model.h`, `sim_model.cpp` (fields, device and type tables, SPICE mapping),
  `sim_model_serializer.cpp`, `sim_model_multiunit.h` –
  https://github.com/KiCad/kicad-source-mirror/blob/ba5ed30c1bee8bcfa7db6dc423f1d20224c4e3a5/eeschema/sim/sim_model.cpp
- [K2] `eeschema/sim/sim_model_ibis.cpp`, `sim_library_ibis.h/.cpp` –
  https://github.com/KiCad/kicad-source-mirror/blob/ba5ed30c1bee8bcfa7db6dc423f1d20224c4e3a5/eeschema/sim/sim_model_ibis.cpp
- [K3] `eeschema/sim/kibis/kibis.cpp`, `kibis.h`, `ibis_parser.h/.cpp` (BSD-3-Clause, © 2022 Fabien Corona), in
  master and 10.0; 7.0 has `sim_model_kibis.cpp` –
  https://github.com/KiCad/kicad-source-mirror/blob/99157a13013a402402c69274e4ce122605750427/eeschema/sim/kibis/kibis.cpp
- [K4] `eeschema/sim/sim_model_tline.cpp` –
  https://github.com/KiCad/kicad-source-mirror/blob/ba5ed30c1bee8bcfa7db6dc423f1d20224c4e3a5/eeschema/sim/sim_model_tline.cpp
- [K5] `eeschema/netlist_exporters/netlist_exporter_spice.cpp` (directives, includes, ground, decomposition) –
  https://github.com/KiCad/kicad-source-mirror/blob/ba5ed30c1bee8bcfa7db6dc423f1d20224c4e3a5/eeschema/netlist_exporters/netlist_exporter_spice.cpp
- [K6] `eeschema/sim/sim_types.h` and `eeschema/dialogs/dialog_sim_command.cpp` in 7.0, 8.0, 9.0, 10.0 (commits in
  [K0])
- [K7] `qa/data/eeschema/spice_netlists/tlines/tlines.kicad_sch` and `tlines.spice` (master)
- [K8] `eeschema/sch_file_versions.h` in 7.0–10.0
- [K9] `eeschema/sim/sim_model_source.cpp` (`portnum`, `z0`), `simulator_frame.cpp` (sheet analysis command,
  `wrdata`, CSV export), `simulator_frame_ui.cpp` (measurements, `.save`/`.probe`), `sim_plot_tab.cpp` (master)
- [K10] `eeschema/sim/sim_lib_mgr.cpp` (library path resolution, embedded files)
- [K11] `kicad/cli/command_sch_export_netlist.cpp` (10.0) –
  https://github.com/KiCad/kicad-source-mirror/blob/99157a13013a402402c69274e4ce122605750427/kicad/cli/command_sch_export_netlist.cpp
- [K12] `include/project/tuning_profiles.h`, `common/project/tuning_profiles.cpp`, `common/project/net_settings.cpp`,
  `pcbnew/dialogs/panel_setup_tuning_profile_info.cpp` (10.0) –
  https://github.com/KiCad/kicad-source-mirror/blob/99157a13013a402402c69274e4ce122605750427/common/project/tuning_profiles.cpp
- [K13] `pcbnew/widgets/pcb_net_inspector_panel.cpp`, `pcbnew/pcb_io/kicad_sexpr/pcb_io_kicad_sexpr.cpp` (9.0, 10.0),
  `pcb_calculator/transline/` (10.0: microstrip, coupled microstrip, stripline, coupled stripline, coplanar, coax,
  twisted pair, rectangular waveguide)
- [K14] `api/proto/schematic/*.proto`, `api/proto/board/board_types.proto`, `board_commands.proto`,
  `api/proto/common/commands/editor_commands.proto` (9.0, 10.0, master) –
  https://github.com/KiCad/kicad-source-mirror/tree/ba5ed30c1bee8bcfa7db6dc423f1d20224c4e3a5/api/proto
- [K15] `pcbnew/board_stackup_manager/board_stackup.h/.cpp` (master: `spec_frequency`, `dielectric_model`)
- [K16] `pcbnew/netlist_reader/board_netlist_updater.cpp` (10.0: footprint fields from symbol fields)

KiCad project pages:
- [K17] KiCad 8.0.0 release (Feb 2024) – https://www.kicad.org/blog/2024/02/Version-8.0.0-Released/
- [K18] KiCad 9.0.0 release (Feb 2025) – https://www.kicad.org/blog/2025/02/Version-9.0.0-Released/
- [K19] KiCad 10.0.0 release (20 Mar 2026) – https://www.kicad.org/blog/2026/03/Version-10.0.0-Released/
- [K20] KiCad developer docs, IPC API for add-on developers –
  https://dev-docs.kicad.org/en/apis-and-binding/ipc-api/for-addon-developers/
- [K21] KiCad forum, "Post-V10 New Features and Development News" (headless API server 21 Mar 2026,
  frequency-dependent dielectric 11 May 2026) – https://forum.kicad.info/t/post-v10-new-features-and-development-news/66611
- [K22] KiCad 9.0 Schematic Editor manual, `Simulation_SPICE` symbol table – https://docs.kicad.org/9.0/en/eeschema/eeschema.html
- [K23] KiCad forum, "Using IBIS Model in KiCAD" (31 May – 1 Jun 2024) – https://forum.kicad.info/t/using-ibis-model-in-kicad/51299
- [K24] FOSDEM 2022, F. Corona, "Advanced PCB simulation with KiCad – Introduction to IBIS and FEM-based
  simulations" – https://archive.fosdem.org/2022/schedule/speaker/fabien_corona/

ngspice:
- [N1] ngspice User's Manual, version 47+ (preface March 2026): element table 2.2, model table 2.3, chapter list –
  https://ngspice.sourceforge.io/docs/ngspice-html-manual/manual.xhtml
- [N2] ngspice change log as packaged by pkgsrc (ngspice-37: `.sp`; ngspice-39: OSDI, Laplace `x_fer` examples) –
  https://pkgsrc.se/cad/ng-spice
- [N3] Wikipedia, Ngspice (licence, analyses, version 47 of 11 Aug 2026) – https://en.wikipedia.org/wiki/Ngspice
- [N4] UC Berkeley, KSPICE (TXL, CPL, recursive convolution) –
  https://ptolemy.berkeley.edu/projects/embedded/Software/spice/kspice.html
- [N5] `ngmultidec(1)` – https://dyn.manpages.debian.org/testing/ngspice/ngmultidec.1
- [N6] xschem issue #377 "Feature request: S to spice" (Jul 2025 – Jan 2026) – https://zff.dev/stef_xschem/xschem/issues/377

Other tools:
- [T1] scikit-rf `VectorFitting` and `write_spice_subcircuit_s` –
  https://scikit-rf-official.readthedocs.io/en/latest/api/generated/skrf.vectorFitting.VectorFitting.html ;
  https://scikit-rf-official.readthedocs.io/en/latest/api/generated/skrf.vectorFitting.VectorFitting.write_spice_subcircuit_s.html
- [T2] snp2le on PyPI – https://pypi.org/project/snp2le/
- [T3] PyBERT documentation – https://pybert.readthedocs.io/en/latest/intro.html ; licence, IBIS-AMI and Touchstone
  details as in [S11]–[S13] of `channel-simulation-ibis.md`
- [T4] Qucs-S discussion #450 (Xyce YLIN, Jan 2024) – https://github.com/ra3xdh/qucs_s/discussions/450

**Not read or blocked in this session:** the KiCad documentation repository on gitlab.com and the ngspice git and
web servers on sourceforge.net were refused by the network policy for git/curl (not worked around), so the bodies of
the ngspice manual sections 6.2–6.4 (LTRA, TXL, CPL parameters) and 11.3.8 were not re-read; the simulator chapter of
the KiCad manual was truncated by the fetch tool (the source code was read instead); sources.debian.org answered with
a bot challenge; the PyBERT GitHub page and several forum threads (only metadata) could not be read. Nothing was run
in KiCad or ngspice.
