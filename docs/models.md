# Models and limits

What kisi computes, how, how well it agrees with references, and which numbers are estimates. kisi is a quick
design check: its results are estimates without any warranty of correctness, not a replacement for sign-off
simulation or measurement.

## Impedance
- **Formulas:** Hammerstad-Jensen, Wheeler, Wadell (CPWG), IPC-2141 – in a second for every track. Microstrip
  within 1–2 %; coupled and coplanar lines 5–15 % off (asymmetric stripline +11 %). A formula value over a cut-out
  in a layer between trace and reference is flagged "rough" and can be 25 % off.
- **2D field solver:** js_2d_fields by Henrik Forstén (GPL-3.0), quasi-static, included unmodified (see
  [UPSTREAM.md](../UPSTREAM.md)); its linear solver runs in Rust (`crates/kisi-sparse`). Cross-sections are taken
  along every line with its reference planes (also across gaps), coplanar ground and the differential partner.
  Within 1.7 % of the HFSS/Ansys references in Forstén's test suite, εeff within 1 %.

Deviation on ThunderScope cross-sections (reference: Forstén's microstrip/stripline solver; the last row HFSS):

| Cross-section | Formula | Field solver |
|---|---|---|
| Microstrip 213 µm, with solder mask | +1.4 % | 0.0 % |
| Diff-microstrip LVDS (127/130 µm) | +7.8 % | −0.3 % |
| Diff-microstrip PCIe (191/127 µm) | +5.6 % | −0.3 % |
| Stripline In3, spacings 1:8 | +10.3 % | 0.0 % |
| Microstrip over a 3 mm cut-out (HFSS) | +17 % | −0.5 % |

## Lines
RLGC per cross-section: skin effect (Wheeler), copper roughness (Huray; the presets `smooth`, `hvlp`, `standard`
are estimates), dielectric loss (Djordjevic-Sarkar). RLGC within 2 % of HFSS.

A channel is a network of N-port elements solved by nodal analysis (`kisi_si::rf::network`): the line's ABCD
cascade, a differential pair as a 4-port (coupled sections from the odd and even mode, via pairs, stubs on both
lines paired up), receivers on side stubs, a Touchstone file behind a connector as an S-parameter block. For a
single line it equals the cascade to numerical precision. TDR comes from S11 (Sdd11 for pairs).

## Vias
A physics-based ladder per via (`kisi_si::via_pbm`): capacitances from an axisymmetric Laplace solve, the plane
pairs as radial lines with the nearby return posts (vias and plated pins of ground and supply planes, the nearest
12 within 3 mm), pads and annular rings, antipads from the zone fills (non-round ones as an equivalent circle),
pours on the via's signal layers as coplanar returns, stubs and back-drill, the junction to the traces; pairs
with their partner. Vias that do not cross a plane are coaxial sections. Compared with 3D results of kisi's own
FDTD solver: via impedance in the TDR median 2.3 %, 90th percentile 5.5 % (single vias); pairs within 3.5°
phase; stub notch within 3 %; the four board vias with a 3D reference within 0.03–0.44 dB up to 12 GHz. An
estimate, not a 3D result: above 12 GHz the ladder follows the 3D results only where its return posts close
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
- **DC IR drop:** copper rasterised per layer, vias as barrels (25 µm plating); per rail and for the ground
  return, which is solved like a rail with the supply input as 0 V. Every part returns what it draws from the
  supply rails minus what it feeds into them (a buck converter takes current from ground).
- **Power tree and losses:** from `PI_REG`/`PI_LOAD`. The power a load draws counts as heat in the part (an
  estimate), regulator losses on the regulator, copper loss is I²R from the DC solutions. A switching regulator
  without an efficiency gets 85 % (an estimate, marked).
- **PDN:** a lumped model of regulator, plane and capacitors with a plane model above 1 MHz; per IC the rail's
  copper on all layers as an AC grid with plane capacitance, capacitors with ESR/ESL and DC-bias derating
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
- **SPICE subcircuit:** line sections as RLGC ladders (20 sections per wavelength at the top frequency) with
  Foster/Debye networks fitted to R(f), L(f), G(f), C(f) – positive elements only, so passive and causal; vias as
  kisi's via ladder; parts with their estimated parasitics (e.g. an AC-coupling capacitor as R–L–C with its pads);
  open stubs; a pair with both lines and its coupled sections as even/odd chains between ideal mode converters.
- **Test benches** for ngspice and Xyce (PRBS7 between the profile's linear transmitter and receiver) and an
  S-parameter deck; the netlist is shared, only the decks differ (the two simulators call analyses differently).
- **Touchstone** per lane (`.s2p`, `.s4p`) and a bus as one N-port with its port map.
- **KiCad project** (KiCad 7 format, opens in KiCad 7 and later; checked with KiCad 9's ERC): one symbol per
  element, parts with their value as a parameter (change `220n` in KiCad and the parasitics stay), or the lane as one
  symbol.

The terminations next to the receiver act as the receiver's termination and are not in the subcircuit. Not
included: the channel behind a connector, crosstalk between lanes, IBIS ends. Above about 10 GHz kisi's own model is
not fully passive and causal (roughness, constant tanδ, via-pair spans); the export cannot follow that part. Report:
[docs/research/channel-export.md](research/channel-export.md).

## What is an estimate
kisi marks these as estimates in the viewer and in the CLI output:
- laminate εr/tanδ and thicknesses that are KiCad defaults (`assumed` in `kisi overview`), until
  `<board>.kisi.json` gives them;
- target impedances from interface keywords in class names ("interface assumption");
- copper roughness presets; package parasitics of series parts, terminations and unfitted pads;
- the transmit direction of a lane;
- regulator efficiency 85 % where none is given; the heat of a load (all drawn power stays in the part);
- capacitor ESR/ESL and DC-bias derating by package, with an assumed rated voltage where neither the Value text
  nor `PI_CAP` gives one; ferrite and inductor values without `PI_SERIES`;
- thermal resistances by package family and size, Tj max 125 °C, where `PI_THERMAL` gives none;
- values in `<board>.kisi.json` whose source kind is `estimate` or `assumption`.

## Validation
- **2D field solver and line models** against the HFSS/Ansys cases of Forstén's test suite: impedance within
  1.7 %, RLGC within 2 % (`tools/validate_field.sh`, which also compares against the stored baseline
  `validation/field_baseline.json`). Validation data: `validation/*.json` (cross-sections), `*.ref.json`
  (references from Forstén's solver, computed in Node with `tools/ref_forsten.mjs` and the solver shim
  `tools/forsten_sparse_shim.js`); `kisi-validate <cases.json> [<refs.json>]` compares the formulas, and the
  field-solver chain (`kisi-validate --geoms` → `tools/solve_xs.mjs` → `kisi-validate --field`) is what
  `validate_field.sh` runs.
- **Vias and planes** against 3D results of kisi's own FDTD solver (`crates/kisi-fdtd`, checked against openEMS
  and analytic cavities): 74 reference cases in `validation/via_ref`, the test log in
  [validation/fdtd/TESTS.md](../validation/fdtd/TESTS.md), and the reference page `kisi-references.html`
  (attached to the releases in which the cases changed) with every case and kisi's models against it. Useful for
  teaching as well: it shows why vias and planes behave as they do.
- **Temperature solver** against analytic cases (1D slab exact, fin within 0.2 %, radial spreading in a plane
  with convection within 0.13 %, energy balance) and, with the datasheets' θJB/θJC on a model of the JEDEC
  JESD51-7 board, against their θJA (QFN-16 +4 %, SOIC-8 −15 %, SOT-23-5 −20 %).
- **Channel export:** ngspice reproduces kisi's own evaluation of the exported circuit (ThunderScope's LVDS lanes and
  nine PCIe lanes, to 0.001) and kisi's S-parameters within 0.03 dB (LVDS) and 0.15–0.42 dB (PCIe, strongly
  reflecting) up to 1.5 × Nyquist; step responses within 2 % (`validation/channel_export/compare.py`, also in CI).
- **Still open:** validation against other solvers and against measurements. A measured test board is planned
  ([validation/test_board.md](../validation/test_board.md)). Until then, treat the numbers as plausible estimates,
  not as verified results.

## Limits
- The channel check is a first look: linear transmitter and receiver from the interface profile, no IBIS models,
  no equalisation, jitter or noise, no simulation of several aggressors at once (except the lanes of one N-port
  file behind a connector). A real channel validation needs IBIS (or AMI) models and more complex tests; kisi
  exports its linear channel models to tools that do that ([Channel export](#channel-export)).
- 2D cross-sections and reduced via and plane models instead of a full-board 3D solution; board edges, via-to-via
  coupling and resonances of the whole board are not included.
- Data the board file does not hold (laminate εr/tanδ, load currents, capacitor data) defaults to estimates;
  `kisi gaps` lists what is missing and `<board>.kisi.json` fills it in with sources
  ([Preparing a board](preparing-a-board.md)).
- One power tree per board: boards that run from USB or a battery have one tree per supply case, which kisi
  cannot represent yet.
