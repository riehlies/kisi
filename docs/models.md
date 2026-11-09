# Models and limits

What kisi computes, how, how well it agrees with references, and which numbers are estimates. kisi is a quick
design check: its results are estimates without any warranty of correctness, not a replacement for sign-off
simulation or measurement.

## Scope: what kisi computes and what it leaves to other tools
kisi is a fast linear checker. It computes the board's lines, vias and channels as linear networks in the frequency
domain and judges them against interface profiles with a linear transmitter and receiver: impedance, loss, return
loss, skew, crosstalk estimates and a linear eye for every lane of a board in seconds, plus the DC and PDN analyses
of the power side.

Out of scope, on purpose: IBIS and IBIS-AMI buffer models, equalisers (CTLE, FFE, DFE) and transient simulations
with non-linear drivers (overshoot and ringback with real buffers, DDR timing flows, long patterns). For sign-off,
export the channel ([Channel export](#channel-export)) – as Touchstone files (a lane, or a whole bus as one N-port)
or as SPICE subcircuits – and simulate it with the buffers in a tool made for that: PyBERT, ngspice, Xyce or LTspice
with IBIS models, KiCad's simulator.

## Why kisi is small and fast
- **Linear networks in the frequency domain.** A line is a chain of 2×2 ABCD matrices per frequency (RLGC per
  cross-section), a via a small ladder, a part a few impedances; a lane is the product of these matrices – a few
  hundred multiplications per frequency point. Where a lane branches (a pair as a 4-port, receivers on a
  multi-drop line, an N-port file behind a connector) kisi solves one small linear system per frequency (nodal
  analysis with a handful of nodes). There is no Newton-Raphson iteration and no time stepping: the eye comes from
  the step response of the linear network.
- **Compared with a circuit simulator:** a lane takes kisi about 0.15 s (ThunderScope: 71 lanes in about 11 s on
  one thread, a few seconds on several cores). The same ThunderScope LVDS lane exported as a SPICE ladder has about
  42 000 elements and takes 6–15 minutes in ngspice or LTspice; the compact export (the default, lossless-line cells)
  brings it to about 8 000 elements and one minute, with the same accuracy in the band.
- **2D, not 3D.** Impedances come from formulas (a board in under a second) or from a quasi-static 2D field solver
  per cross-section – only for nets with a target, cached on disk and in the browser; vias, pads and planes use
  reduced models checked against 3D references instead of a full-board 3D solution.
- **Sparse and parallel where it counts.** The DC voltage drop, temperature and PDN grids use sparse solvers
  (`kisi-sparse` on faer); the channel check computes its lanes on several threads (native) or Web Workers
  (viewer) with results identical to one thread.
- **One engine, compiled once.** The models are Rust compiled to WebAssembly: the whole viewer including the field
  solver is one HTML file of about 3 MB that runs offline in a browser, and the same engine runs the Node CLI
  `kisi.mjs`.

## Impedance
kisi computes every track's impedance in two ways:
- **Formulas** – in a second for every track: Hammerstad-Jensen (microstrip), Wheeler (stripline), IPC-2141
  (pairs), and conformal mapping (Ghione & Naldi; Simons) for coplanar lines, i.e. copper of any net on the
  trace's layer within 3 h (at most 2 mm) beside it.
- **2D field solver** – js_2d_fields by Henrik Forstén (GPL-3.0), quasi-static, included unmodified (see
  [UPSTREAM.md](../UPSTREAM.md)), its linear solver in Rust (`crates/kisi-sparse`). Cross-sections are taken
  along every line with its reference planes (also across gaps), coplanar copper and the differential partner.

How close they are, against CST Studio Suite on 620 cross-sections ([Validation](#validation); median / 90th
percentile):

| Line type | Formula | Field solver |
|---|---|---|
| microstrip, symmetric stripline | 0.2 / 0.4 % | within 1 % |
| microstrip with solder mask | 1.2 / 1.8 % | within 1 % |
| with coplanar copper (CPWG, stripline with coplanar ground) | 0.5–0.9 / 1.3–1.7 % | within 1 % |
| offset stripline | 2 / 9 % | within 1 % |
| edge-coupled pair, Zdiff (microstrip / stripline) | 2.7 / 12 % · 4.3 / 33 % | within 1 % |
| over a cut-out in a layer between trace and plane | flagged "rough", 25 % and more | within 1 % |

**Known limits** (details and numbers in [validation/cst_xs](../validation/cst_xs/README.md)):
- **Trace shape:** every trace is a rectangle of the drawn width and full copper thickness; KiCad's stackup has no
  etch factor. Real traces are etched to a trapezoid, so kisi's Z0 comes out about 2–4 % low for 35 µm copper,
  less for thin inner layers.
- **Coplanar formulas:** a pair with coplanar ground on one side only is approximated as the mean of two symmetric
  pairs (Zeven typically 10 % off, up to −47 / +70 % for very narrow gaps under solder mask); the partner is taken
  as wide as the trace; stripline pairs and covered traces with one plane ignore their coplanar copper. The field
  solver is not affected.
- **Pads and gold fingers:** a track inside an SMD pad of its own net has the pad's cross-section, a cut-out in the
  plane included; the rest of a part's pad is a shunt capacitance (without a cross-section, an estimate per
  package from `data/components.json`). Copper with no plane above or below takes its return in the coplanar
  copper and a pair's partner (an estimate); with nothing within 0.5 mm beside it the channel uses 1.5 × the
  target impedance. Against 3D: [TESTS.md](../validation/fdtd/TESTS.md) section 15.

**Fab tolerance band** (the option "Fab stackups & tolerances", off by default). The fab builds every board with
some spread – dielectric thickness, εr, copper thickness, trace width –, and that usually moves the impedance more
than kisi's calculation is off (the [accuracy levels](#accuracy-levels): about 1 % with the field solver, 2–11 %
with formulas; the viewer gives a value's accuracy in its tooltip). With the option on kisi shows the band wherever
it shows an impedance (net, lane, tooltip, the board map's Tolerance mode, `net`/`check`/`overview`); the band and
the accuracy are not added. The band comes from one-at-a-time sensitivities: central
differences ΔZ = (Z(x + δ) − Z(x − δ)) / 2 for each dielectric's thickness and εr inside the cross-section, the trace
layer's copper thickness and the trace width, combined as worst case Σ|ΔZ| and as root sum of squares √ΣΔZ². The
width tolerance is symmetric – every copper edge moves by half of it, so gaps shrink as widths grow – with no etch
factor (trapezoid). Per cross-section kisi takes the formulas' relative differences and scales them to the
cross-section's impedance (field solver or formula), a net's band is the length-weighted mean (the tolerances move
all of its pieces the same way); the per-class table of `--tolerance`, `kisi stackups` and the viewer's “Compare &
tolerances” uses the field solver's differences where it has the perturbed cross-sections. The status compares the
nominal value and the RSS band with the class's window (its interface profile's tolerance, else the fab's ±10 %):
inside, nominal inside but the band leaves, nominal outside. It is a linear estimate around the nominal value, and
information only: no verdict uses it. The tolerances: kisi's defaults (`data/stackups.json`, mostly estimates) <
kisi.json `tolerances` < `--tol` or the viewer's edits ([reference](reference.md#board-configuration-boardkisijson));
the option is on with a kisi.json `tolerances` entry, `--tolerance` or `--tol`, or the viewer's switch.

**Other stackups** (`kisi stackups`, the viewer's “Compare & tolerances”): per net class and layer kisi takes the
cross-section with the most length (pads left out; a coupled one for a differential target) and evaluates the same
geometry – references, coplanar copper, partner – under other stackups with the formulas or, where it has the
result, the field solver.

## Lines
RLGC per cross-section, passive and causal by construction (the series impedance and the shunt admittance per
metre are positive-real functions of frequency):
- **Conductor:** the DC resistance turning into the skin effect (Wheeler's incremental inductance, or the field
  solve's resistance), R_dc·√(1 + jf/f_c) with its internal inductance, times the causal Huray roughness factor
  (Bracken, DesignCon 2012) – its loss part is Huray's, its inductance part what Kramers-Kronig requires. The
  roughness presets `smooth`, `hvlp`, `standard` are estimates.
- **Dielectric:** one Djordjevic-Sarkar model (a wideband Debye distribution fitted to εr and tanδ at 1 GHz) for
  ε′ and ε″ – in the lines, the via ladders and the coaxial via sections alike.
- **Microstrip dispersion:** a lossless Lorentz term that follows Getsinger's rising εeff below a resonance
  above the band. A passive line cannot follow a rise of εeff with frequency except below a resonance; a
  microstrip on a thick dielectric, whose Getsinger corner lies inside the band (at 50 Ω and 25 GHz from about
  0.4 mm), gets the rise at the top of the band and less below it.

RLGC within 2 % of HFSS.

A channel is a network of N-port elements solved by nodal analysis (`kisi_si::rf::network`): the line's ABCD
cascade, a differential pair as a 4-port (coupled sections from the odd and even mode, via pairs, stubs on both
lines paired up), receivers on side stubs, a Touchstone file behind a connector as an S-parameter block. For a
single line it equals the cascade to numerical precision. TDR comes from S11 (Sdd11 for pairs).

## Vias
A physics-based ladder per via (`kisi_si::via_pbm`): capacitances from an axisymmetric Laplace solve, the plane
pairs as radial lines with the nearby return posts (vias and plated pins of ground and supply planes, the nearest
12 within 3 mm), pads and annular rings, antipads from the zone fills (non-round ones as an equivalent circle),
pours on the via's signal layers as coplanar returns, stubs and back-drill, the junction to the traces; pairs
with their partner. Passive by construction: the plane-pair matrices treat every barrel as a cylinder of
uniform current, and a pair's modes are taken as (Z_PP + Z_NN)/2 ∓ Z_PN (the earlier Z_PP ∓ Z_PN gave via pairs
with returns on one side a negative resistance from a few GHz). Vias that do not cross a plane are coaxial sections. Compared with 3D results of kisi's own
FDTD solver ([Validation](#validation)): via impedance in the TDR median 2.5 %, 90th percentile 5.6 % (single
vias); pairs within 1.8° phase (median); stub notch within 1.5 % (`validation/fdtd/results/kisi_models.json`); the
four board vias with a 3D reference within 0.03–0.44 dB up to 12 GHz. An estimate, not a 3D result: above 12 GHz the ladder follows the 3D results only where its return posts close
the via's cage. A via whose path crosses a ground/supply plane pair that no return via bridges gets a warning.
`--via-cavity` (finite plane pairs) and `--via-returns` (return groups per plane pair) are experimental options.

## Lanes and the channel check
- **Lanes:** `kisi_si::topology` follows nets through series resistors and AC-coupling capacitors from device to
  device (or to a connector). Series parts and terminations carry package parasitics from
  `data/components.json` (estimates). The transmit direction is a guess (the termination sits next to the
  receiver) and can be swapped.
- **Stubs:** copper of a lane net that leaves the path is an open side stub – to unused pads and test points, to
  the pads of parts that are not fitted (with their pad capacitance, an estimate), and – when all alternatives of
  a fork are fitted – through the other part to the other device. A stub hangs on the side of a via that matches
  its layer.
- **Multi-drop:** on a line along one path the devices on side stubs are receivers with the profile's receiver
  model; the worst eye counts. Multi-drop trees through series parts are listed, not checked.
- **Eye:** PRBS7 NRZ with the profile's linear transmitter and receiver, from the lane's network (pairs:
  differential drive of the 4-port; ends per line to AC ground, so the common mode sees the same termination).
  No equalisation, jitter or noise. The step response has a DC point (the resistances of the signal band; where
  the channel changes below the lowest frequency – a small coupling capacitor, a choke – that frequency stands
  for DC) and the Gaussian edge as window. Warnings when the band ends inside the edge spectrum (above 35 % left:
  the edge rings by about 2 % and more), when there is response before the signal can arrive (non-causal data,
  time aliasing) or the step has not settled, and when S-parameters are not passive.
- **At a connector:** the eye uses the end set in `connector_ends` and the channel behind it; without a channel
  behind the connector it covers the board share only and is information, not judged. Lanes from connector to
  connector have no eye (transmitter and receiver are off the board).
- **Verdict:** one function (`kisi_si::profile::judge`) for viewer and CLI. Limits marked "kisi" in the profile
  sources are guide values, not standards. Mode conversion (Scd21) is information, not judged.

## Crosstalk
A coupling scan of every net with a target: NEXT and FEXT from the neighbouring tracks, critical from 1 % of the
aggressor swing (pairs as differential noise, with the common-mode noise beside it). The estimate uses formula
coupling; the field solver refines the critical couplings. Broadside overlaps between layers are detected and
counted, not computed. Several aggressors at once are only combined for the lanes of one N-port file behind a
connector.

## Power
- **DC IR drop:** copper rasterised per layer into square cells (0.05 … 1 mm, automatic) with the copper
  fraction of every cell, tracks narrower than three cells as line conductors along their centre line (their
  resistance at any angle), vias as barrels (25 µm plating) whose contact with a pour is corrected for the cell
  size (Peaceman: a barrel smaller than the cell acts as its true radius, a larger one as an equipotential disc);
  per rail and for the ground return, which is solved like a rail with the supply input as 0 V. Every part
  returns what it draws from the supply rails minus what it feeds into them (a buck converter takes current from
  ground). Largest drop within 1 % of a converged reference (median of 21 rails on 7 boards, at most 10 % on a
  motor supply of overlapping wide tracks); see [Validation](#validation).
- **Power tree and losses:** from `PI_REG`/`PI_LOAD`. The power a load draws counts as heat in the part (an
  estimate), regulator losses on the regulator, copper loss is I²R from the DC solutions. A switching regulator
  without an efficiency gets 85 % (an estimate, marked).
- **PDN:** a lumped model of regulator, plane and capacitors with a plane model above 1 MHz; per IC the rail's
  copper on all layers as an AC grid with plane capacitance (via contacts corrected as in the DC drop), capacitors with ESR/ESL and DC-bias derating
  (estimated per package unless `PI_CAP` gives the data), ferrites and series parts up to the regulators
  (typical values unless `PI_SERIES` gives them), the loop inductance of each capacitor to the IC,
  anti-resonances, and a target from the IC's current step (or `PI_TARGET`).

## Temperature (estimate)
Steady state of the board in air. One sheet per copper layer on a grid over the board outline (copper fill of
every net, laminate in plane 0.8 and through the thickness 0.3 W/m·K, open via barrels), natural or forced
convection (flat-plate correlations) and radiation from both sides; heat from the copper loss of every cell and
the parts. A part is a junction node with θJB to the board under its pads and θJC to its top (JEDEC two-resistor
model), with values from `PI_THERMAL` or package defaults by family and size (`data/components.json`,
estimates); the case top from ψJT or that model. Per via the heat through its barrel; per hot part the split of
its heat into vias, laminate and the pad layer. The IR drop at temperature solves the rails again with
ρ(T) = ρ₂₀(1 + 0.393 %/K·(T − 20 °C)).

Not included: heat sinks, conduction into mounting holes, connectors or card guides, the enclosure (a device in a
closed case runs hotter inside: emulate it with the case temperature as `--ambient` and the inner coupling as
`--h`), the split of a regulator's loss between IC and inductor, transients. Junction temperatures with package
defaults can be off by tens of percent of the rise. Concept, physics and validation:
[research/thermal.md](research/thermal.md).

## Channel export
`kisi export` and "Export…" in the viewer's lane detail and bus card write a lane's board channel – as the check
models it – for circuit and RF simulators:
- **SPICE subcircuit:** line sections fitted as Foster/Debye networks to R(f), L(f), G(f), C(f) – positive
  elements only, so passive and causal – and written as the compact model (default, below) or as RLGC ladders
  (`--model ladder`, 20 sections per wavelength at the top frequency, the fitted networks in every section); vias as
  kisi's via ladder; parts with their estimated parasitics (e.g. an AC-coupling capacitor as R–L–C with its pads);
  open stubs; a pair with both lines and its coupled sections as even/odd chains between ideal mode converters.
- **Compact model** (the default; "Lines in the SPICE netlist" in the viewer's export settings): the same
  fitted networks, but each line section as a few lossless-line cells – a lossless LC ladder (40 sections per
  wavelength, 2 elements each) carries the delay, the rest of the fitted networks is lumped once per cell (at least
  3 cells per delay × top frequency, more for microstrip, whose dispersion is lumped); consecutive sections with
  nearly the same model are merged. Passive and causal by construction, like the ladder, with 2–5 × fewer
  elements and PRBS benches 7–17 × faster in ngspice (23 × at 5 × Nyquist) and about 15 × in LTspice; it follows kisi's S-parameters as closely as the ladder up to the
  top frequency and is not meant for use above it (the cells' lumped losses reflect coherently from about 1.5 × the
  top frequency). The simulators' own line elements (`T`, LTRA) and vector-fitted line models were studied and not
  used: [research/vector-fitting.md](research/vector-fitting.md).
- **Test benches** for ngspice, Xyce and LTspice (PRBS7 between the profile's linear transmitter and receiver, the
  LTspice one with `.meas` results) and S-parameter decks; the netlist is shared, only the decks differ (the
  simulators call analyses differently; LTspice adds a series resistance (1 mΩ) and an implicit parallel damping
  to inductors by default, so the LTspice library sets `Rser=0 Rpar=1e30` on every inductor to match ngspice and
  Xyce).
- **Touchstone** per lane (`.s2p`, `.s4p`) and a bus as one N-port with its port map.
- **Frequency range:** the top frequency (default 25 GHz; `--fmax`, or from the profile: 5 × Nyquist, or 0.5 / t_r of
  its edge where that is higher) sets the cells and ladders and ends the Touchstone grid. A lower top frequency shortens the ladder's
  transients only so far: an 85 mm LVDS pair has 38 k elements at 25 GHz and 23 k at 2.5 GHz, because every
  section, via and part keeps its share. The compact model shrinks with the top frequency: 8.3 k and 3.9 k elements
  for that pair. For long transients `--fmax profile` is the fast choice.
- **KiCad project** (KiCad 7 format, opens in KiCad 7 and later; checked with KiCad 9's ERC): one symbol per
  element, parts with their value as a parameter (change `220n` in KiCad and the parasitics stay), or the lane as one
  symbol.

The terminations next to the receiver act as the receiver's termination and are not in the subcircuit. Not
included: the channel behind a connector, crosstalk between lanes, IBIS ends. kisi's own model is passive and
causal (checked over every lane of the example boards up to 25 GHz), so the fitted circuit can follow it across the
band. Report:
[docs/research/channel-export.md](research/channel-export.md).

## Accuracy levels
Every impedance and via result says which level it used and how far it typically is from kisi's validation
(`data/accuracy.json`, with the sources; design: [research/accuracy-levels.md](research/accuracy-levels.md)):

| Level | Typical deviation |
|---|---|
| 2D field solver | impedance 1 % |
| formula: single microstrip / with coplanar copper / pair / stripline / over a cut-out | 2 / 2 / 8 / 11 / 25 % |
| no plane (coplanar estimate) | 15 % |
| via ladder: single vias / pairs (TDR, median and 90th percentile) | 2.5 and 5.9 % / 12.6 and 14.4 % |
| coaxial via fallback | 8.0 and 25.5 % |

A net or lane takes the length-weighted mean of its cross-sections; the level is “field solver”, “formula” or
“mixed”. The CLI writes it as `accuracy` in every `check` row and in `net`, the viewer as a muted “±x %” next
to the impedance. Verdicts are judged on the values; the uncertainty is shown beside them. Loss, return loss
and the eye do not carry a number yet.

## What is an estimate
kisi marks these as estimates in the viewer and in the CLI output:
- laminate εr/tanδ and thicknesses that are KiCad defaults (`assumed` in `kisi overview`), until
  `<board>.kisi.json` gives them;
- target impedances from interface keywords in class names ("interface assumption");
- copper roughness presets; package parasitics of series parts and terminations; the pad capacitance per
  package where a pad has no cross-section of its own; the coplanar impedance of copper without a reference
  plane;
- the transmit direction of a lane;
- regulator efficiency 85 % where none is given; the heat of a load (all drawn power stays in the part);
- capacitor ESR/ESL and DC-bias derating by package, with an assumed rated voltage where neither the Value text
  nor `PI_CAP` gives one; ferrite and inductor values without `PI_SERIES`;
- thermal resistances by package family and size, Tj max 125 °C, where `PI_THERMAL` gives none;
- values in `<board>.kisi.json` whose source kind is `estimate` or `assumption`.

## Validation
Each model is checked against an independent reference; the details, tables and every case are in the linked
documents and on the two reference pages attached to the releases (`kisi-references-2d.html`,
`kisi-references-3d.html`).

| What | Reference | Result | Details |
|---|---|---|---|
| Impedance (formulas, field solver) | CST Studio Suite, 2D port modes, open boundary; 620 cross-sections of every line type, the example boards and the test-board coupons | field solver within 1 % (median 0.2 %); formulas see [Impedance](#impedance) | [validation/cst_xs](../validation/cst_xs/README.md) |
| Line loss | full-wave FEM with eddy-current conductor loss (js_2d_fields), checked by an independent finite-difference solution | kisi's line model R about 2 % low | [validation/cst_xs](../validation/cst_xs/README.md) |
| Vias and planes (fast models) | 3D FDTD (`crates/kisi-fdtd`) on 106 cases, itself checked against openEMS and CST | single vias TDR 2.5 / 5.6 %, pairs Zdiff 11 / 14 % (median / p90) | [TESTS.md](../validation/fdtd/TESTS.md), [CST.md](../validation/fdtd/CST.md) |
| Field solver chain | HFSS/Ansys cases of Forstén's test suite | impedance within 1.7 %, RLGC within 2 % | `tools/validate_field.sh` |
| Passivity and causality | unit tests and every lane of the example boards | passive to 25 GHz, causal | [TESTS.md](../validation/fdtd/TESTS.md) section 14 |
| DC IR drop | analytic cases (vias in a plane, tracks at any angle, necks, cut cells); kisi's own raster refined uniformly (cell/4 … cell/8, barrels as discs, line tracks) on 21 rails of 7 boards | analytic within 1.5 %; largest drop median 1.0 %, max 10 % (before the corrections 6.9 / 28 %) | [research/pi-quadtree.md](research/pi-quadtree.md) |
| Temperature | analytic cases; JEDEC JESD51-7 θJA | analytic within 0.2 %; θJA +4 … −20 % | – |
| Channel export | ngspice on the exported circuits | S-parameters within 0.03–0.34 dB up to 1.5 × Nyquist | [research/channel-export.md](research/channel-export.md) |

Two things the cross-checks taught: the result of a reference solver depends a lot on its setup (mesh at the metal
edges, boundaries, loss model), and a reference can be wrong too – kisi-fdtd's inner-layer via exits needed a finer
mesh, and CST's 2D lossy-metal loss is a surface-impedance approximation (4–23 % low), so the line loss is judged
against the full-wave result.

**Still open:** measurements. A test board with 2.92 mm connectors is planned (branch `feat/test-board`); until
then, treat the numbers as validated against solvers, not against hardware.

## Limits
- The channel check is a first look: linear transmitter and receiver from the interface profile, no jitter or
  noise, no simulation of several aggressors at once (except the lanes of one N-port file behind a connector). IBIS,
  IBIS-AMI and equalisers are out of kisi's scope ([Scope](#scope-what-kisi-computes-and-what-it-leaves-to-other-tools));
  a sign-off simulation uses the exported channel in a tool made for that ([Channel export](#channel-export)).
- 2D cross-sections and reduced via and plane models instead of a full-board 3D solution; board edges, via-to-via
  coupling and resonances of the whole board are not included.
- Data the board file does not hold (laminate εr/tanδ, load currents, capacitor data) defaults to estimates;
  `kisi gaps` lists what is missing and `<board>.kisi.json` fills it in with sources
  ([Preparing a board](preparing-a-board.md)).
- One power tree per board: boards that run from USB or a battery have one tree per supply case, which kisi
  cannot represent yet.
