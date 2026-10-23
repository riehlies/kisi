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
- **Finite plane cavities in the via ladder (experimental, `--via-cavity`, off by default):** board edges,
  stitching vias and capacitors of the plane pairs a via crosses, validated against seven new 3D cases
  (`validation/fdtd/BOARD_RESONANCES.md`); `kisi-validate cavities` lists the plane pairs of a board's channel vias.

## Next
- **`kisi gaps`: supply rails without a voltage in the name.** Nets such as `Net-(IC22-VCC)` are not recognised as
  rails, so the ICs on them are missing from the load list. Use regulator outputs (`PI_REG … @V`) and the parts on
  the net to detect them.
- **Skills on real boards.** Run kisi-prepare and kisi-si-pi-review on a few real projects; sharpen the research
  guidance (datasheet sections, typical vs. maximum) and add a worked example `<board>.kisi.json`.

## Publishing
- **Measured validation:** the test board (`validation/test_board.md`) and comparisons with other solvers.

## Models and accuracy
- **IBIS models as channel ends** (removed from 0.1.0; the experimental code is in commit `92b8105`). Open: the right
  model per pin with real vendor files ([Model Selector], [Diff Pin], package models, pin names vs. pads), how
  IBIS ends and interface profiles fit together (swing, terminations, current-mode drivers), validation with real
  files and a reference simulator. Research and concept (solvers, model sources, DDR test flows, assembly
  variants, staged plan): `docs/research/channel-simulation-ibis.md`; M0 (assembly variants) is done, M1 (IBIS
  parser) is next.
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
  - done: a 3D check of board vias from the layout (`kisi-validate via-case`, `validation/fdtd/BOARD_VIAS.md`).
    The ThunderScope M.2 dip near 16 GHz is real (a broad reflection maximum of the pair via, not a resonance;
    the ladder makes it 0.3 dB too deep);
  - ladder errors found on the board vias: GND pours on the via's own signal layers enter as planes (1.1–4.6 ps too
    slow, too capacitive on the M.2 pair and the LibreVNA via; dropping them fixes the delay but loses the stub
    notch, so they need a coplanar-ground treatment); series inductance 105–241 pH too high; a 30 GHz notch of the
    ThunderScope through via (likely its return-via cage) is missing; remaining from Block 1: TDR maxima of long
    stubs, wide traces at the via, the thick-core through via of the references (~50 fF missing);
  - re-score the Block 1 cases with 44–47 Ω stripline fixtures with the matched de-embedding
    (`sparam.deembed_matched`) when the ladder is next fitted;
  - "3D check" of a via or a track point in the viewer (exporter in WebAssembly, one worker per excitation,
    fixture cache, plots against the ladder; estimate 8 days, minimal version 4; `BOARD_VIAS.md` section 4);
  - done (experimental, off by default): plane resonances of finite boards in the ladder, `--via-cavity`
    (`kisi_si::via_cavity`: rectangle and grid of any shape with stitching vias, capacitors and edge radiation;
    `validation/fdtd/BOARD_RESONANCES.md`). Against 3D it finds every resonance below 12 GHz (1.6–2.5 % in
    frequency) and the coupling of two vias in one cavity; on the example boards it changes no verdict (most
    channel vias have close return vias) at 2–4× the run time. Next: return groups per plane pair (a GND via does
    not short a GND/supply pair; there the cavity with its capacitors is the return), via-to-via coupling in the
    channel and crosstalk, the grid as the PDN cavity model for irregular planes, the vertical parts of the PDN
    loop;
  - the ladder in WebAssembly is about 2 × slower than native (Laplace solve and equivalent radius per via type).
- **Measured test board** (ToDo): coupons from the reference cases on JLCPCB / Aisler standard stackups with
  screw-on end-launch connectors, simulated in kisi-fdtd and CST, measured on Keysight ENA / PNA-X / PNA; plan in
  `validation/test_board.md`.
- **Crosstalk:** compute broadside coupling between layers (detected and listed so far), aggressors in the eye;
  crossings of plane splits and edges (return path), then via-to-via coupling through the cavity (the coupled
  ladders are in `via_pbm::coupled_s`, validated in `BOARD_RESONANCES.md`).
- **PDN:** per-IC grid for all rails at once, regulator models beyond R + L (control-loop output impedance),
  comparison with a measurement or a reference tool.
- **Power:** via current capacity; a temperature estimate from the losses map (2D heat spreading with
  conductivity of copper and laminate, convection; clearly an estimate); split of regulator losses between IC and
  inductor.
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
