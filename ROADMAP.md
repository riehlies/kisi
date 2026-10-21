# kisi – roadmap

Planned and possible work after 0.2.0, roughly in order of priority. Each item says why it matters;
"Decide first" marks items that need a decision before work starts.

## Next
- **First release and its workflow.** Check that `.github/workflows/release.yml` runs through (tests, field-solver
  validation, example boards, build) and attaches `kisi.html`, `kisi-examples.html` and `kisi.mjs`.
- **Checks on every push.** A CI workflow that runs `cargo test`, `tools/validate_field.sh` and
  `tools/check_viewer.py` on pushes and pull requests, not only on releases.
- **`kisi gaps`: supply rails without a voltage in the name.** Nets such as `Net-(IC22-VCC)` are not recognised as
  rails, so the ICs on them are missing from the load list. Use regulator outputs (`PI_REG … @V`) and the parts on
  the net to detect them.
- **Skills on real boards.** Run kisi-prepare and kisi-si-pi-review on a few real projects; sharpen the research
  guidance (datasheet sections, typical vs. maximum) and add a worked example `<board>.kisi.json`.

## Publishing
- **Decide first: make the repository public.** Then `kisi.mjs` downloads without a GitHub login and the
  Claude Code plugin installs for everyone.
- **GitHub Pages** for the viewer (needs a public repository on the free plan): publish `kisi.html` from the
  release workflow, so it opens without downloading.

## Models and accuracy
- **IBIS models as channel ends** (removed from 0.1.0; the experimental code is in commit `92b8105`). Open: the right
  model per pin with real vendor files ([Model Selector], [Diff Pin], package models, pin names vs. pads), how
  IBIS ends and interface profiles fit together (swing, terminations, current-mode drivers), validation with real
  files and a reference simulator.
- **Eye and channel check on the 4-port pair model** (the 4-port with mode conversion is shown in the net detail;
  eye and check still use the odd-mode 2-port).
- **Pads and gold fingers without a reference plane:** a proper model instead of the ~1.5 × estimate.
- **Via model against 3D results**: 68 kisi-fdtd reference cases in `validation/via_ref` (page
  `dist/kisi-references.html` from `validation/fdtd/make_viewer.py`, test log `validation/fdtd/TESTS.md`). The
  physics-based via ladder (`kisi_si::via_pbm`) replaced the coax-section model in the channel; the plane-cavity
  model is in `kisi_pi::cavity_modal`. Next:
  - exits on inner layers (about 3 ps of delay missing), conductor loss of the barrel, non-round antipads beyond
    the nearest-edge circle;
  - a 3D check of board vias with GND pour on the signal layers and a partner at < 0.6 mm (ThunderScope M.2 lanes:
    the ladder shows a dip near 16 GHz there);
  - the cavity model in the PDN (rectangular planes) and the vertical parts of the PDN loop;
  - the ladder in WebAssembly is about 5 × slower than native (Laplace solve per via type).
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
