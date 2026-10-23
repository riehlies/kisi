# kisi – roadmap

Planned and possible work after 0.2.4, roughly in order of priority. Each item says why it matters;
"Decide first" marks items that need a decision before work starts.

## Next
- **Capacitor ratings from part fields.** Many libraries carry the rated voltage and the dielectric in fields
  (`Voltage Rating`, `Dielectric`; XIAO nRF54LM20A: on every capacitor), but kisi reads them only from the Value
  text or `PI_CAP`, so `kisi gaps` lists such capacitors under cap-ratings and the DC-bias derating uses an
  assumed rating. Read these fields too (their usual names belong in `data/components.json`).
- **Skills on real boards.** Run kisi-prepare and kisi-si-pi-review on a few more real projects and sharpen the
  research guidance (datasheet sections, typical vs. maximum, efficiency read from curves). The first worked
  example is the XIAO nRF54LM20A (`docs/examples/xiao-nrf54lm20a/`).

## Publishing
- **Measured validation:** the test board (`validation/test_board.md`) and comparisons with other solvers.

## Models and accuracy
- **IBIS models as channel ends – on hold.** Decision of the maintainer (2026-10): branch `feat/ibis-parser` is
  kept as preliminary work but not merged. Its viewer integration became too complicated – too many
  combinations and error messages – and `main` must always have a clean, self-explanatory build. Before this
  continues, look at what KiCad already brings for simulation: ngspice, IBIS through KiCad's KIBIS and the
  `Sim.*` fields. Then decide between handing kisi's channel models (lines, vias, series parts) back to KiCad
  for its simulator, and configuring the IBIS models in KiCad (in the schematic) or in `kisi.json`, so that
  kisi does not grow a second model setup. Background: removed from 0.1.0 (experimental code in commit
  `92b8105`); research and concept (solvers, model sources, DDR test flows, assembly variants, staged plan) in
  `docs/research/channel-simulation-ibis.md`; M0 (assembly variants) is in 0.2.4. The branch holds M1 (IBIS
  parser, `kisi ibis`, mapping in kisi.json with `config-check`) and, on top of it, linearised IBIS ends in check
  and eye with the model choice per IC and lane in the interfaces view (one “Load IBIS…”, “as the profile” as
  default, manual pin mapping), as preliminary work for M3 (transient with the buffers). Open there: KiCad's
  `Sim.*` fields as a second mapping source; IBIS 8.0 keywords; package-model sections; the parser against
  IBISCHK on real vendor files; an on-die termination and a termination resistor next to the receiver are not
  combined; the edge from the waveforms already contains the C_comp load; open-drain drivers without the pull
  resistor; the receiver's R_pin; validation against an IBIS simulator.
- **Assembly variants, follow-ups:** variant-dependent `fields` in kisi.json (e.g. the TS-USB4 power tree from
  the M.2 connector); a via whose barrel continues into a stub beyond the path's layers is counted twice (its own
  stub and the side stub's via, ~0.3 mm on ThunderScope); side stubs in the net channel (`kisi net`) do not add
  the via they leave through yet; the transmit direction of lanes from an IC to a connector is a guess.
- **Channel network, M2 follow-ups.** M2's first increment (`kisi_si::rf::network`, `rf::timedomain`, `lanenet`)
  checks pairs as 4-ports with mode conversion and multi-drop lines along one path per receiver. Open:
  - N-port files behind a connector: one file for all lanes with a port map per pin works, the other lanes are
    worst-case aggressors entering at the connector; open: the aggressors' own board traces and transmitters
    (today the victim's profile, without their traces – conservative), synchronous aggressors with a known phase
    instead of the worst phase, crosstalk on the board between the lanes (the coupling scan) in the same eye;
  - multi-drop trees through series parts (DDR fly-by with VTT terminations), pairs on multi-drop lines;
  - a pair's fork branches (board as it is, variant `default`) and terminations across a pair as coupled 4-port
    elements (today per line; the common mode is terminated per line, centre-tapped);
  - judge the mode conversion (Scd21 is information; limits per profile need sources) and a receiver setting
    "across" vs. "centre-tapped" termination for the common mode.
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
- **Power:** via current capacity; split of regulator losses between IC and inductor (also a second heat source
  in the temperature estimate).
- **Temperature, follow-ups** (the steady state is in `kisi thermal` and the viewer, `docs/research/thermal.md`):
  - **Heat sinks**: a heat sink on a part (`theta_sa`, area, or a sink touching several parts), thermal pads to a
    chassis, conduction into mounting holes, card guides and connectors (often a large share on PCIe/M.2 cards).
  - **More convection settings**: a closed enclosure (case surface and material, the coupling of the board to it
    through air gaps and radiation – the LibreVNA's case runs near the touch limit, section 7.4), a vertical board in a
    rack, a fan with a given air speed per side, the ambient inside a device.
  - **Transient simulation**: thermal capacitance on the same matrix (implicit time steps), power profiles and duty
    cycles, the time to reach a temperature.
  - **Export to a 3D thermal solver** (e.g. the board's layers, copper fill, vias and the parts' power as a model for
    an open CFD/FEM tool) to design the heat sink there.
  - Smaller: local grid refinement under small hot parts; filled vias and laminate conductivity by material name;
    iterating the electro-thermal step to a fixed point and temperature-dependent IC power (leakage); a comparison
    with IPC-2152 trace heating and a measurement (IR camera) on the test board.
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
- Keep `data/interfaces.json` (standards) and `data/components.json` (package parasitics) current, with sources.
