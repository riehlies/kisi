# kisi – roadmap

Planned and possible work after 0.3.1, in three development blocks plus work across them. Within a block the items
are roughly in order of priority; each says why it matters, "Decide first" marks items that need a decision before
work starts. One rule holds for all of them: `main` always has a clean, mostly self-explanatory build – new features
come with a small, clear UI and without option combinatorics.

Block 1 comes first: better solvers and channel models help little while the inputs (stackup, targets, loads,
models) are missing or wrong, and the setup effort is the main risk of block 3. Block 2 runs alongside, because
validation depends on measurements that take time.

## 1. Usability: GUI, CLI, documentation
Goal: humans in the viewer and agents with the CLI can both use kisi easily. Well-prepared boards already display
well; the challenge is preparing a new design for a meaningful analysis – missing data, definitions and models.
- **Documentation structure** (first step, decided 2026-10-24) – **done:** a short README and the user
  documentation in `docs/` (getting started, preparing a board with the XIAO nRF54LM20A as the worked example, field
  and `kisi.json` reference, models and limits, CLI reference, agents), linked from the README and the landing page;
  `tools/docs_pdf.py` renders it into `kisi-docs.pdf`, which CI builds (and uses as a link check) and the release
  attaches; `crates/kisi-core/tests/docs.rs` keeps `docs/cli.md` (the `kisi help` text, every command and option)
  and the gap ids in step with the code. The repository was checked for stale statements and plans. Next: each
  feature updates its page in the same change (rule in `AGENTS.md`); the maintainer's coordinating session keeps
  the overview of the docs when branches are merged.
- **Board preparation in the viewer** (decided 2026-10-24: shared core plus a "Prepare" step; a mockup first). One
  core for humans and agents: `kisi gaps` already says what is missing, where to find it and where it goes in
  `kisi.json`. The "Prepare" step lists the gaps and lets the simple values be entered directly (laminate εr/tanδ,
  net class targets and profiles, regulators and loads, thermal data), checked by the same code as `config-check`,
  and exports `<board>.kisi.json`. Research-heavy values (datasheets) stay with the agent path (skill
  kisi-prepare). No second logic in the viewer. **Done, released in 0.3.0** (mockup v3 approved, then implemented): the core
  (gap categories from `affects`, "nothing to check" per category, readiness, `kisi gaps --for`, `gaps.dismissed`
  for name-based gaps, `config::subset`, check messages with categories), the WASM exports, and the viewer's
  "Prepare PCB" page (tabs, forms for laminate, net classes, variant, regulators, loads, beads and thermal data,
  dismissals, one draft, live check, save everything or one category next to the board, the agent prompt per tab,
  readiness dots), then made fully functional: every entry of a loaded kisi.json in editable forms (keys without a
  form kept, load and save unchanged gives the same file), the draft per board kept in the browser with Discard,
  forms for capacitor data (by part number, with the parts' own rating fields as a suggestion), connector ends
  (Touchstone file and ports) and variant definitions, the views' stackup, target, profile, connector-end and
  power edits moved into the draft (power through the core's tree/setup → fields conversion), categories per
  gap item from the core, stale dismissals with remove; `check_viewer.py` runs the acceptance on the bare
  ThunderScope (its kisi.json entered through the forms, saved, reloaded). After the maintainer's review the page
  became a checklist (a marker per step, one line of what it needs, Apply through the core with a check mark or
  what is still missing, the next step opens, progress and "Ready for …" per tab, saving as the last step; the
  marker counts a step's open items), the readiness dots on the view buttons are gone, kisi.json can give nets a
  class of their own (`net_classes.<name>.nets`), and the example boards carry a complete kisi.json.
  Still open, with the reason: profile parameters edited in the Channel view (limits, data rate) are stored per
  browser – kisi.json has no key for them; lane direction swaps and path flips are display choices, not board data;
  pads chosen for a source or load in rail setups are lost when the setups move into fields (fields name parts, not
  pads – the page says so); a `PI_REG`/`PI_LOAD` from the KiCad file cannot be removed by kisi.json (the page names
  it); the forms do not parse field values themselves (the core's check reports what it cannot read); no
  step-by-step undo (per gap for dismissals, "Discard draft" for all); values from datasheets or curves stay on the
  agent path, by design.
- **Capacitor ratings from part fields** – **done:** the rated voltage and dielectric come from PI_CAP, then the
  parts' rating fields (`Voltage Rating`, `Dielectric`, `Temperature Coefficient`, … – the names, surveyed on the
  example boards, the XIAO and the JLC/LCSC library, in `data/components.json`), then the Value text, then the
  description; the PDN names the source of each value, `kisi gaps` lists only capacitors still without a rating,
  and the viewer's capacitor form suggests what the fields give in part. The XIAO and ThunderScope need no
  `PI_CAP` for it any more (ThunderScope's 256 entries removed from its kisi.json, results unchanged).
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
- **KiCad plugin** (decided 2026-10-26: IPC API plugin, no local server) – **released in 0.3.1:** a toolbar button
  in KiCad 10's PCB editor opens the saved board with its `<board>.kisi.json` in a copy of the viewer
  (`analysis/<board>.kisi.html`), the package carries the CLI and the agent guide (`docs/getting-started.md`,
  design note `docs/research/kicad-plugin.md`). Tested in KiCad 10 on Windows; the entry for KiCad's Plugin and
  Content Manager (status `testing`, KiCad 10.0) is under review. Open: a test in KiCad 10 on Linux and macOS
  (checklist in the design note; their code paths are reviewed and unit-tested), then status `stable`; KiCad 9
  (untested); the "unsaved changes" notice once KiCad's API reports them (`GetDocumentModifiedState`, KiCad's
  development branch – the plugin uses it when kicad-python has it).
- **Decide first: live link to KiCad** (cross-probing, reload on save). Needs a local helper process; only worth it if
  the manual “Reload” is not enough. KiCad 10 can place DRC markers from outside, and KiCad 10's tuning profiles
  (target impedance, delay per layer in the `.kicad_pro`) could be read as targets (`docs/research/kicad-simulation.md`).

## 2. Solvers, accuracy, validation
Goal: fast models whose accuracy is known – checked against other solvers and against measured test designs, with a
3D re-check of single parts where it matters. The trade-off accuracy vs. run time becomes a choice per net, not a
principle: accuracy levels formula → 2D field solver → 3D for selected structures, each with an error from the
validation, and every result says which level it used and how uncertain it is.
- **Measured test board.** Coupons from the reference cases on JLCPCB / Aisler standard stackups with screw-on
  end-launch connectors, simulated in kisi-fdtd and CST, measured on Keysight ENA / PNA-X / PNA; plan
  (`validation/test_board.md`) and coupons v2 in branch `feat/test-board` (waiting for the RF group's advice on the
  measurement).
- **Vias and planes in 3D** (merged into `main` at 98af58d with Blocks 1–6): Block 2 `feat/via-3d-board` (board via →
  kisi-fdtd exporter, matched de-embedding, `BOARD_VIAS.md`), Block 3 `feat/board-resonances` (finite plane cavities
  and via-to-via coupling, option `--via-cavity`, `BOARD_RESONANCES.md`), Block 4 `feat/return-groups` (return vias
  per plane pair and decoupling capacitors as returns, option `--via-returns`, `BOARD_RETURNS.md`). Block 5 on the
  same branch: done – a warning in the default path for channel vias across a GND/supply pair no via bridges
  (`check`, `net`, the viewer's details; none on the example boards); the pours on the via's signal layers as
  coplanar returns in the ladder (the default; M.2 pair phase 24° → 2°); two corrections of the comparisons (board
  vias at the via centre, the TDR of the validation scripts – the long-stub TDR maxima were this);
  `--via-cavity` / `--via-returns` reassessed and kept off (`BOARD_VIAS.md` section 5, `BOARD_RETURNS.md`
  sections 7–8). Block 6: done – the REFINOUT2 excess loss above 12 GHz was an open return cage: plated pads of
  plane nets (connector pins) are return posts now, the nearest 12 (12–20 GHz 0.41 → 0.02 dB against 3D, loss at
  20 GHz 16 → 7 %, 3D 9 %; `BOARD_VIAS.md` section 6); the cavity option shares its factorisations per region
  (ThunderScope 105 → 57 s on one thread, results reproducible; `BOARD_RESONANCES.md` section 9). Next:
  - the return groups' floating-plane loss is 1.6× the 3D on REFINOUT2 (0.29 dB off at 12–20 GHz) – what keeps
    them from the default; the cavity option is still 8× the default (per-via fill masks);
  - pair vias read 8–11 % high in Zdiff minimum with the corrected TDR (L1→L8 references); the X exits without
    a plane on one side (+56 fF on X_s8x4); the thick-core through via B_ts6_L1L6 (−50 fF); re-score the
    Block 1 references with the matched de-embedding;
  - above 12 GHz neither ladder follows the 3D results, and the first board mode comes out 1.6–2.5 % low;
  - **a 3D check in the viewer** (export a selected via or structure, run kisi-fdtd in a worker, compare with the
    ladder at the via centre; about 8 days, a minimal version 4; design note `BOARD_VIAS.md` section 5.6);
  - the ladder in WebAssembly is about 2 × slower than native.
- **Causal and passive models** (`feat/accuracy`, done): the via pairs' modes and the plane-pair matrix passive by
  construction, Djordjevic-Sarkar everywhere, the causal Huray roughness with its inductance, microstrip
  dispersion below a resonance; every example lane passive and causal in a test (`validation/fdtd/TESTS.md`
  section 14). Next: a microstrip on a thick dielectric (Getsinger corner inside the band) loses its mid-band
  dispersion – a lossy Lorentz term (surface-wave leakage) would follow it.
- **Pads and gold fingers without a reference plane** (`feat/accuracy`, done): pads and fingers from their own
  cross-section, a part's pads as shunts from it, copper without a plane on its coplanar return; five 3D
  cases (TESTS.md section 15). Next: the finger pair without any plane reads ~15 % low (the coplanar estimate);
  the finger's tip beyond the contact as a stub; a 3D case with the 0402 body (the ESL estimate).
- **Accuracy levels** (`feat/accuracy`, first version): the impedance (formula / field solver, with the
  validated uncertainty) and the via models carry their level in `check`, `net` and the viewer
  (`data/accuracy.json`, design note `docs/research/accuracy-levels.md`). Next: loss (± dB at Nyquist from
  lines and vias) and return loss (a range from the impedance uncertainty), then the eye; the 3D level per
  selected structure with the viewer's 3D check.
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
Goal: fast linear channel models of the whole board – lines, vias, parts and lanes as ABCD/S networks in the
frequency domain – for quick checks with the profiles' linear ends, and as good channel models for sign-off in
other tools. Decided 2026-10-25: **IBIS, IBIS-AMI, equalisers (CTLE/FFE/DFE) and transient interface simulations
(DDR flows, overshoot/ringback with buffers, long patterns) are out of kisi's scope.** Non-linear and time-domain
sign-off happens in tools made for it (PyBERT, ngspice/Xyce/LTspice with IBIS, KiCad's simulator) with kisi's
exported channel. Why: kisi's speed comes from linear frequency-domain networks (a lane in about 0.15 s); modified
nodal analysis with Newton-Raphson over the whole channel means large matrices and minutes per lane (the SPICE
export shows it). Background: `docs/models.md` (scope, why kisi is fast); the earlier staged concept
`docs/research/channel-simulation-ibis.md` is kept as background (M0 and M2 shipped; M1, M3–M6 not pursued).
- **Export of device-to-device connections to circuit and RF simulators** (decided 2026-10-24). Steps 1 and 2 are
  done (`kisi export`, “Export…” in the viewer's lane detail and bus card with the export settings – files and
  frequency range; `kisi_si::export`; report `docs/research/channel-export.md`): the lane's board channel as SPICE
  subcircuits (RLGC ladders with fitted R(f)/G(f) networks, kisi's via model, parts, stubs; pairs with modal coupled
  blocks), test benches for ngspice, Xyce and LTspice with the profile's linear ends, a KiCad project (one symbol per
  lane element, KiCad 7 format, checked with kicad-cli and its ERC; IBIS ends can be added there by the user), and
  Touchstone per lane and as one N-port of a bus. Checked in ngspice against kisi's S-parameters on ThunderScope's
  LVDS lane and the whole PCIe bus (`validation/channel_export/compare.py`, also in CI), and in LTspice on the
  maintainer's PC (single line within 0.07 %, LVDS pair within 0.33 %). Next:
  - **reduced models** (step 3, branch `feat/vector-fit`): researched (`docs/research/vector-fitting.md`: vector
    fitting and its variants, delayed and characteristic line models, passivity enforcement, what ngspice, Xyce and
    LTspice implement) and implemented as `--model compact` and "Lines in the SPICE netlist" in the viewer's export
    settings: lossless-line cells – LC lines carry the delay, the ladder's fitted loss networks are lumped once per
    cell, equal consecutive sections merged; passive and causal by construction (checked to 10 × f max). Results
    in `docs/research/channel-export.md` section 7. LTspice 26 on the maintainer's PC follows ngspice
    within 1 mV (0.3 %) in 61 s (ladder 938 s). The default since 2026-10-25 (maintainer's decision; `--model
    ladder` stays as an option). Next: the vias (now about a third of a compact lane's elements) could get a reduced ladder too; vector
    fitting with delays and passivity enforcement stays for tabulated data (a Touchstone block behind a
    connector);
  - the top frequency from the interface profile (`--fmax profile`: 5 × Nyquist, or 0.5 / t_r of the profile's edge
    where that is higher) stays an option; the default remains 25 GHz (decided 2026-10-25: a file valid over the
    whole band);
  - crosstalk between the lanes of a bus in the N-port (the coupling scan's sections as coupled blocks), the
    channel behind a connector (a Touchstone block as a subcircuit);
  - Xyce and Qucs-S/Qucsator runs of the exported files (not available in the development environment so far).
- **Channel network, M2 follow-ups** (`kisi_si::rf::network`, `rf::timedomain`, `lanenet`):
  - N-port files behind a connector: the aggressors' own board traces and transmitters (today the victim's
    profile, without their traces – conservative), synchronous aggressors with a known phase instead of the worst
    phase, crosstalk on the board between the lanes (the coupling scan) in the same eye;
  - multi-drop trees through series parts (DDR fly-by with VTT terminations), pairs on multi-drop lines;
  - a pair's fork branches (board as it is, variant `default`) and terminations across a pair as coupled 4-port
    elements (today per line; the common mode is terminated per line, centre-tapped);
  - judge the mode conversion (Scd21 is information; limits per profile need sources) and a receiver setting
    "across" vs. "centre-tapped" termination for the common mode.
- Branch `feat/ibis-parser` (IBIS parser, linearised IBIS ends) is not merged and not continued (out of scope).

## Across the blocks
- **Reference boards** with a prepared `kisi.json`: tutorial (block 1), regression test (all blocks) and
  validation (block 2). Today: ThunderScope, LibreVNA, XIAO nRF54LM20A (`docs/examples/`).

## Maintenance
- **Issues and small fixes:** check the reported GitHub issues regularly, answer or label them, and keep a list of
  small, independent fixes ("Small fixes" below) that can be done in parallel to the blocks.
- **Branches:** keep the number of open branches small – one branch per topic, merged as a whole when the topic is
  ready; preliminary work for one topic stays on one branch.
- Solver updates with `tools/upstream_update.sh` when js_2d_fields changes (validation decides; the weekly CI run
  reports a newer upstream revision).
- **CI:** the checks run by what changed (`tools/ci_checks.py changes`); stored results of the example boards and the
  via references (`tools/golden.py`) catch unintended model changes; the docs PDF and the channel export in ngspice
  run in their groups (the KiCad project check `compare.py --kicad` locally only: the KiCad PPA download stalled
  too often in CI; package installs go through `tools/ci_install.sh` with retries, a mirror fallback and a cache); the Rust code is formatted (`cargo fmt --check`) and clippy-clean
  (`-D warnings`, native and wasm) on every push that touches it, with the Rust version pinned in
  `rust-toolchain.toml` (raised on purpose, not by a new stable release). Next: parse a KiCad 10 test board.
- Keep `data/interfaces.json` (standards) and `data/components.json` (package parasitics) current, with sources.

### Small fixes (independent, can be done in parallel)
- (none open)
