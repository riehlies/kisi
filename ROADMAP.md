# kisi – roadmap

Planned and possible work after 0.2.3, roughly in order of priority. Each item says why it matters;
"Decide first" marks items that need a decision before work starts.

## Done since 0.2.3
- **Assembly variants (M0 of the channel-simulation concept, `docs/research/channel-simulation-ibis.md`
  sections 6–8):** KiCad 10 variants from the board file, `variants` / `variant` in `<board>.kisi.json` (by sheet,
  references or BOM), `--variant` and a selector in the viewer; parts that are not fitted leave their pads as stubs,
  side stubs and the other path of a fork in the lane cascade; `gaps` and the lane commands ask for a variant when
  a board has variants or forks with all alternatives fitted. ThunderScope's example config has TS-PCIe and TS-USB4.
- **Channel view and connector ends:** one "Channel check" card in steps (with the impedance view's field solver),
  the board map per variant (unused nets faint, click opens a channel), a Path tab for nets without devices, and an
  eye for every lane: at a connector with the end set per connector (termination or driver) and the channel behind
  it – a lossy line or a Touchstone file (`.s2p`, also on each line of a pair; `.s4p` of a pair) – judged when a
  channel behind it is given (`connector_ends` in kisi.json).
- **IBIS parser (M1 of the channel-simulation concept):** new crate `kisi-ibis` (IBIS 1.1–7.2 keywords for a
  physical channel, corners with NA, scaling factors, comment character, [Model Selector], [Diff Pin], [Pin
  Mapping], series and terminator models, [Define Package Model] and [Submodel] as data; unsupported keywords
  reported; IBISCHK-like checks; Ron/Roff and slew rate derived from the tables). `kisi ibis <file.ibs>`
  (native and Node). `ibis` mapping in `<board>.kisi.json` (file + sha256, component, `pin_map` by number or pin
  name, pad exceptions, selector choices), checked by `config-check` with the files; `gaps` lists lane-end ICs
  without a model (low priority). The viewer checks the mapping with `.ibs` files dropped with the board, kept in
  memory only. Fixtures: synthetic files from analytic buffers and KiCad's QA models.
- **IBIS models at the lane ends (linearised):** bus cards of the Interfaces overview choose per IC the model file
  (“Load IBIS…” in the overview's header, or dropped), component, pin map (by number, by name or manual: pad → IBIS
  pin per lane-end pad, `"pin_map": "manual"` with `pins`), corner and selector choices, with a per-lane model override
  and a “kisi.json entry” (with sha256); `check`/`interfaces` use the kisi.json mapping (`models`: pad → model) the same
  way in both CLIs. Driver and receiver become linear ends (Ron, C_comp + package R/L/C, edge from the waveforms or
  [Ramp], terminator / ODT), marked as estimates; limits and verdict stay the profile's.

## Next
Several items below come from running the kisi-prepare skill on the Seeed XIAO nRF54LM20A board (power tree only).
- **`kisi gaps`: supply rails without a voltage in the name.** Nets such as `Net-(IC22-VCC)` are not recognised as
  rails, so the ICs on them are missing from the load list. Use regulator outputs (`PI_REG … @V`) and the parts on
  the net to detect them. XIAO: VSYS, VBUS and VBAT of the nPM1300 PMIC are not found; `VSYS@5V` in the field works
  as a workaround.
- **PI_REG output rails with spaces in the net name.** Hierarchical names such as `/06 Peripherals/IMU_VDD` cannot be
  written, because the field parser splits at whitespace; allow quoting (`"/06 Peripherals/IMU_VDD"`). Workaround on
  the XIAO: a load point at the ferrite (`IMU&MIC_3V3=0.9mA@FB3`).
- **Default efficiency of switching regulators as an estimate.** Without a value kisi assumes 85 %; the viewer's power
  tree shows it as “η 85 %” without an estimate mark and the CLI output has no flag for it. Mark it as an estimate in
  both (AGENTS.md rule), so that a missing datasheet value is visible.
- **MPN in `kisi gaps` and part lists.** Footprints often carry a Manufacturer / MPN field while the Value field is not
  a part number; show the MPN when the value is not one, so that the user (and the skill) can look up the right
  datasheet. XIAO: U4 has the value “英联” (the manufacturer), the MPN is UM3204H.
- **`kisi config-template`: a load as the PI_LOAD example.** The example takes the first U part, which can be a
  regulator (XIAO: U1 TPS628438); pick a part that is a load (not a PI_REG part, not on a regulator's output only).
- **Skills on real boards.** Run kisi-prepare and kisi-si-pi-review on a few real projects; sharpen the research
  guidance (datasheet sections, typical vs. maximum) and add a worked example `<board>.kisi.json`.
- **kisi-prepare guidance from the XIAO test:** explain `NAME@<V>V` for rails without a voltage in the name, `id=` for
  PMICs with several channels and `NET=I@REF` load points; ask early for the user's datasheets – several manufacturer
  PDFs could not be read through web fetch, and efficiency is often only given as a curve.

## Publishing
- **Measured validation:** the test board (`validation/test_board.md`) and comparisons with other solvers.

## Models and accuracy
- **IBIS models as channel ends** (removed from 0.1.0; the experimental code is in commit `92b8105`). Open: the right
  model per pin with real vendor files ([Model Selector], [Diff Pin], package models, pin names vs. pads), how
  IBIS ends and interface profiles fit together (swing, terminations, current-mode drivers), validation with real
  files and a reference simulator. Research and concept (solvers, model sources, DDR test flows, assembly
  variants, staged plan): `docs/research/channel-simulation-ibis.md`; M0 (assembly variants) and M1 (IBIS parser,
  `kisi ibis`, mapping in kisi.json) are done, M2 (N-port network) is next. Open from M1: KiCad's `Sim.*` fields as
  a second mapping source; IBIS 8.0 keywords; package-model sections ([Pin Numbers] with Len/L/C); a check of the
  parser against IBISCHK on real vendor files; checks of the combined (pullup + clamp) I-V curves. Open from the
  linear IBIS ends: an on-die termination and a termination resistor next to the receiver are not combined (the
  resistor replaces the receiver's R); the edge from the waveforms already contains the C_comp load (counted twice
  with the C in the end model); open-drain/source drivers are linearised with their one transistor (the pull
  resistor is not modelled); the receiver's R_pin is left out; validation against an IBIS simulator.
- **Assembly variants, follow-ups:** variant-dependent `fields` in kisi.json (e.g. the TS-USB4 power tree from
  the M.2 connector); a via whose barrel continues into a stub beyond the path's layers is counted twice (its own
  stub and the side stub's via, ~0.3 mm on ThunderScope); side stubs in the net channel (`kisi net`) do not add
  the via they leave through yet; the transmit direction of lanes from an IC to a connector is a guess.
- **Eye and channel check on the 4-port pair model** (the 4-port with mode conversion is shown in the net detail;
  eye and check still use the odd-mode 2-port).
- **N-port files behind a connector with crosstalk between lanes** (one file for all lanes of a connector or cable):
  belongs to the multi-lane network (M2 of the channel-simulation concept); today a connector takes a 2-port or a
  pair's 4-port, without coupling to the other lanes.
- **Pads and gold fingers without a reference plane:** a proper model instead of the ~1.5 × estimate.
- **Via model against 3D results**: 74 kisi-fdtd reference cases in `validation/via_ref` (page
  `dist/kisi-references.html` from `validation/fdtd/make_viewer.py`, test log `validation/fdtd/TESTS.md`). The
  physics-based via ladder (`kisi_si::via_pbm`) replaced the coax-section model in the channel, with a junction
  model fitted to the 3D references, equivalent radii for non-round antipads and conductor loss; the plane-cavity
  model is in `kisi_pi::cavity_modal`. Next:
  - a 3D check of board vias from the layout (exporter board → kisi-fdtd case), first the ThunderScope M.2 vias
    (GND pour on the signal layers, partner at 0.53 mm: the ladder shows a dip near 16 GHz there);
  - remaining ladder errors: TDR maxima of long stubs, wide traces at the via, the ThunderScope through via (thick
    core, ~50 fF missing);
  - plane resonances of finite boards in the ladder (cavity model instead of infinite plates), via-to-via coupling
    through the cavity, the cavity model in the PDN and the vertical parts of the PDN loop;
  - the ladder in WebAssembly is about 2 × slower than native (Laplace solve and equivalent radius per via type).
- **Measured test board** (ToDo): coupons from the reference cases on JLCPCB / Aisler standard stackups with
  screw-on end-launch connectors, simulated in kisi-fdtd and CST, measured on Keysight ENA / PNA-X / PNA; plan in
  `validation/test_board.md`.
- **Crosstalk:** compute broadside coupling between layers (detected and listed so far), aggressors in the eye;
  crossings of plane splits and edges (return path), then via-to-via coupling through the cavity.
- **PDN:** per-IC grid for all rails at once, regulator models beyond R + L (control-loop output impedance),
  comparison with a measurement or a reference tool.
- **Power:** via current capacity; a temperature estimate from the losses map (2D heat spreading with
  conductivity of copper and laminate, convection; clearly an estimate); split of regulator losses between IC and
  inductor.
- **Power tree per supply case.** Boards that run from USB or a battery (a PMIC power path) have one tree per case:
  on the XIAO, VSYS is VBUS on USB and VBAT on battery, and the debugger is supplied only on USB; while charging, the
  battery charge current is a load on VSYS. Today one tree has to stand for all cases.
- **Whole-board validation** against a commercial tool (e.g. HyperLynx) on the example boards.
- **Decide first: equalisation** (CTLE/FFE) for profiles that require it (PCIe Gen3+). So far deliberately left out.

## Viewer and command line
- **Report export** from the viewer (HTML/PDF summary of checks and power results).
- **Faster field solver in the CLI:** better scheduling across threads, optional larger time budget presets.
- **Smaller downloads:** separate WASM builds for viewer and CLI (the viewer does not need the CLI commands).
- **Decide first: live link to KiCad** (cross-probing, reload on save). Needs a local helper process; only worth it if
  the manual “Reload” is not enough.

## Maintenance
- Solver updates with `tools/upstream_update.sh` when js_2d_fields changes (validation decides).
- `tools/check_viewer.py --board`: a board without impedance targets (no `.net` rows) stops the run at the net
  detail; skip that step when there are no nets (found with the XIAO board).
- Keep `data/interfaces.json` (standards) and `data/components.json` (package parasitics) current, with sources.
