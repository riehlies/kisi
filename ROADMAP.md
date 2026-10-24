# kisi – roadmap

Planned and possible work after 0.2.5, in three development blocks plus work across them. Within a block the items
are roughly in order of priority; each says why it matters, "Decide first" marks items that need a decision before
work starts. One rule holds for all of them: `main` always has a clean, mostly self-explanatory build – new features
come with a small, clear UI and without option combinatorics.

Block 1 comes first: better solvers and channel models help little while the inputs (stackup, targets, loads,
models) are missing or wrong, and the setup effort is the main risk of block 3. Block 2 runs alongside, because
validation depends on measurements that take time.

## 1. Usability: GUI, CLI, documentation
Goal: humans in the viewer and agents with the CLI can both use kisi easily. Well-prepared boards already display
well; the challenge is preparing a new design for a meaningful analysis – missing data, definitions and models.
- **Documentation structure** (first step, decided 2026-10-24) – **done in branch `feat/docs-prepare`:** a short
  README and the user documentation in `docs/` (getting started, preparing a board with the XIAO nRF54LM20A as the
  worked example, field and `kisi.json` reference, models and limits, CLI reference, agents), linked from the
  README and the landing page; `tools/docs_pdf.py` renders it into `kisi-docs.pdf`, which CI builds (and uses as a
  link check) and the release attaches; `crates/kisi-core/tests/docs.rs` keeps `docs/cli.md` (the `kisi help` text,
  every command and option) and the gap ids in step with the code. The repository was checked for stale
  statements and plans (the commit messages say what changed and why). Next: each feature updates its page in
  the same change (rule in `AGENTS.md`); the Prepare step gets its section in `docs/preparing-a-board.md`.
- **Board preparation in the viewer** (decided 2026-10-24: shared core plus a "Prepare" step; a mockup first). One
  core for humans and agents: `kisi gaps` already says what is missing, where to find it and where it goes in
  `kisi.json`. The "Prepare" step lists the gaps and lets the simple values be entered directly (laminate εr/tanδ,
  net class targets and profiles, regulators and loads, thermal data), checked by the same code as `config-check`,
  and exports `<board>.kisi.json`. Research-heavy values (datasheets) stay with the agent path (skill
  kisi-prepare). No second logic in the viewer. **Mockup and design note done** (`docs/mockups/prepare.html`,
  `prepare.md`: reuse of `kisi_core::gaps` and `config::check` through WASM, about 7–8 days, a first version 4–5).
  Next: the maintainer's review of the mockup (open questions in `prepare.md`), then the implementation.
- **Capacitor ratings from part fields.** Many libraries carry the rated voltage and the dielectric in fields
  (`Voltage Rating`, `Dielectric`; XIAO nRF54LM20A: on every capacitor), but kisi reads them only from the Value
  text or `PI_CAP`, so `kisi gaps` lists such capacitors under cap-ratings and the DC-bias derating uses an
  assumed rating. Read these fields too (their usual names belong in `data/components.json`).
- **Power tree per supply case.** Boards that run from USB or a battery (a PMIC power path) have one tree per case:
  on the XIAO, VSYS is VBUS on USB and VBAT on battery, and the debugger is supplied only on USB; while charging, the
  battery charge current is a load on VSYS. Today one tree has to stand for all cases.
- **Assembly variants, follow-ups:** variant-dependent `fields` in kisi.json (e.g. the TS-USB4 power tree from
  the M.2 connector); a via whose barrel continues into a stub beyond the path's layers is counted twice (its own
  stub and the side stub's via, ~0.3 mm on ThunderScope); side stubs in the net channel (`kisi net`) do not add
  the via they leave through yet; the transmit direction of lanes from an IC to a connector is a guess.
- **Skills on real boards.** Run kisi-prepare and kisi-si-pi-review on a few more real projects and sharpen the
  research guidance (datasheet sections, typical vs. maximum, efficiency read from curves).
- **Report export** from the viewer (HTML/PDF summary of checks and power results).
- **Smaller downloads:** separate WASM builds for viewer and CLI (the viewer does not need the CLI commands).
- **Faster field solver in the CLI:** better scheduling across threads, optional larger time budget presets.
- **Decide first: live link to KiCad** (cross-probing, reload on save). Needs a local helper process; only worth it if
  the manual “Reload” is not enough. KiCad 10 can place DRC markers from outside, and KiCad 10's tuning profiles
  (target impedance, delay per layer in the `.kicad_pro`) could be read as targets (`docs/research/kicad-simulation.md`
  in branch `research/kicad-sim`).

## 2. Solvers, accuracy, validation
Goal: fast models whose accuracy is known – checked against other solvers and against measured test designs, with a
3D re-check of single parts where it matters. The trade-off accuracy vs. run time becomes a choice per net, not a
principle: accuracy levels formula → 2D field solver → 3D for selected structures, each with an error from the
validation, and every result says which level it used and how uncertain it is.
- **Measured test board.** Coupons from the reference cases on JLCPCB / Aisler standard stackups with screw-on
  end-launch connectors, simulated in kisi-fdtd and CST, measured on Keysight ENA / PNA-X / PNA; plan in
  `validation/test_board.md`, coupons v2 in branch `feat/test-board` (waiting for the RF group's advice on the
  measurement).
- **Vias and planes in 3D** (preliminary work in branches, not merged; one topic – the work continues on
  `feat/return-groups`, which contains Blocks 2 and 3, and goes to `main` as one merge once the default path is
  sound): Block 2 `feat/via-3d-board` (board via →
  kisi-fdtd exporter, matched de-embedding, `BOARD_VIAS.md`), Block 3 `feat/board-resonances` (finite plane cavities
  and via-to-via coupling, option `--via-cavity`, `BOARD_RESONANCES.md`), Block 4 `feat/return-groups` (return vias
  per plane pair and decoupling capacitors as returns, option `--via-returns`, `BOARD_RETURNS.md`). Next:
  - fix the ladder errors found in Block 2 – pours on the signal layers as a coplanar return, the series
    inductance, TDR maxima of long stubs, wide traces at the via, the ThunderScope through via (~50 fF missing) –
    before cavities and return groups can become the default;
  - a warning in the default path for channel vias that cross a GND/supply pair no via bridges (the cavity survey
    finds them already);
  - above 12 GHz neither ladder follows the 3D results, and the first board mode comes out 1.6–2.5 % low;
  - **a 3D check in the viewer** (export a selected via or structure, run kisi-fdtd in a worker, compare with the
    ladder; about 8 days, a minimal version 4) – after the warning and the ladder fixes (order decided 2026-10-24:
    warning, ladder fixes, 3D check);
  - the ladder in WebAssembly is about 2 × slower than native.
- **Pads and gold fingers without a reference plane:** a proper model instead of the ~1.5 × estimate.
- **Crosstalk:** broadside coupling between layers (detected and listed so far), crossings of plane splits and
  edges (return path), via-to-via coupling through the cavity.
- **PDN:** per-IC grid for all rails at once, regulator models beyond R + L (control-loop output impedance), the
  cavity model in the PDN, comparison with a measurement or a reference tool.
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
  - **Export to a 3D thermal solver** (the board's layers, copper fill, vias and the parts' power as a model for an
    open CFD/FEM tool) to design the heat sink there.
  - Smaller: local grid refinement under small hot parts; filled vias and laminate conductivity by material name;
    iterating the electro-thermal step to a fixed point and temperature-dependent IC power (leakage); a comparison
    with IPC-2152 trace heating and a measurement (IR camera) on the test board.
- **Whole-board validation** against a commercial tool (e.g. HyperLynx) on the example boards.

## 3. Channel and lane simulation
Goal: simulate channels as well as possible, including the dependencies between lanes (crosstalk, shared
references). The staged concept is in `docs/research/channel-simulation-ibis.md` (M0 assembly variants: 0.2.4; M2
N-port networks, pairs as 4-ports, multi-drop: 0.2.5). IBIS-AMI and equalisers make the later stages large, and the
setup (models, pin mapping, terminations, interface settings) is the main effort – it depends on block 1.
- **Export of device-to-device connections to circuit and RF simulators** (first step, decided 2026-10-24). kisi
  stays a fast linear solver that provides good linear channel models; non-linear analysis (IBIS buffers, AMI,
  equalisers) is done in other tools. Steps:
  1. the channel as a physical RLGC ladder – a SPICE subcircuit per line segment and per element (discrete R, L, C,
     vias, stubs), composed into the lane – for SPICE simulators (ngspice, Xyce), with a ready test bench using the
     profile's linear transmitter and receiver; a simulation project that opens in KiCad 9 and later (IBIS ends
     through KiCad's KIBIS where `kisi.json` names models);
  2. a Touchstone export of the full channel (per lane and as an N-port of a bus) for RF tools (PyBERT,
     scikit-rf, Qucsator);
  3. a vector-fitting reduction of long RLGC segments that keeps the delay (delay lines) and the frequency-dependent
     loss.
  Test cases: ThunderScope LVDS (SYNC_OUT U18 → U10), then a PCIe pair; goal: the whole PCIe bus. Each export is
  checked against kisi's own S-parameters (e.g. ngspice's AC/S-parameter analysis of the exported ladder). Research
  note: `docs/research/kicad-simulation.md` in branch `research/kicad-sim`; KIBIS models one buffer per symbol and
  rejects files above IBIS 7.0 or with unknown keywords, so IBIS models live in a separate simulation project, not on
  the ICs of the design schematic.
- **IBIS models – on hold.** Branch `feat/ibis-parser` (IBIS parser, `kisi ibis`, mapping in kisi.json with
  `config-check`, linearised IBIS ends with the model choice in the interfaces view) is kept as preliminary work but
  not merged: its viewer integration became too complicated (too many combinations and error messages). Its parser
  and checks can serve the export (e.g. "will KiCad accept this file?"). Open there: KiCad's `Sim.*` fields as a
  mapping source; IBIS 8.0 keywords; package-model sections; the parser against IBISCHK on real vendor files;
  on-die and external terminations combined; the C_comp load in the waveform edge; open-drain pull resistors; the
  receiver's R_pin; validation against an IBIS simulator.
- **Channel network, M2 follow-ups** (`kisi_si::rf::network`, `rf::timedomain`, `lanenet`):
  - N-port files behind a connector: the aggressors' own board traces and transmitters (today the victim's
    profile, without their traces – conservative), synchronous aggressors with a known phase instead of the worst
    phase, crosstalk on the board between the lanes (the coupling scan) in the same eye;
  - multi-drop trees through series parts (DDR fly-by with VTT terminations), pairs on multi-drop lines;
  - a pair's fork branches (board as it is, variant `default`) and terminations across a pair as coupled 4-port
    elements (today per line; the common mode is terminated per line, centre-tapped);
  - judge the mode conversion (Scd21 is information; limits per profile need sources) and a receiver setting
    "across" vs. "centre-tapped" termination for the common mode.
- **Decide first: equalisation and IBIS-AMI** (CTLE/FFE/DFE for PCIe Gen3+, DDR5, multi-gigabit Ethernet). Weigh
  an own implementation against handing the channel to existing tools (e.g. PyBERT, a simulator with AMI support);
  kisi's strength may be providing the channel quickly and correctly rather than the whole link simulation.
- **Interface simulations** (later: DDR5, PCIe, Gigabit Ethernet) on top of the export and the setup work of
  block 1 – the concept's M3–M6 (transient with buffers, measurements such as overshoot and ringback, DDR flows,
  long patterns).

## Across the blocks
- **Reference boards** with a prepared `kisi.json`: tutorial (block 1), regression test (all blocks) and
  validation (block 2). Today: ThunderScope, LibreVNA, XIAO nRF54LM20A (`docs/examples/`).

## Maintenance
- **Issues and small fixes:** check the reported GitHub issues regularly, answer or label them, and keep a list of
  small, independent fixes ("Small fixes" below) that can be done in parallel to the blocks.
- **Branches:** keep the number of open branches small – one branch per topic, merged as a whole when the topic is
  ready; preliminary work for one topic stays on one branch.
- Solver updates with `tools/upstream_update.sh` when js_2d_fields changes (validation decides).
- Keep `data/interfaces.json` (standards) and `data/components.json` (package parasitics) current, with sources.

### Small fixes (independent, can be done in parallel)
- `tools/check_viewer.py dist/kisi.html --board examples/VNA.kicad_pcb …` reports three “coupling refinement”
  problems on LibreVNA (the field-solver refinement of critical crosstalk couplings); CI runs the check on
  `kisi-examples.html` and does not see it. Found during the 0.2.5 work, older than it.
