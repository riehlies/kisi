# kisi documentation

| | |
|---|---|
| [Getting started](getting-started.md) | the release files, the viewer's three views, the first commands |
| [Preparing a board](preparing-a-board.md) | the gap list, filling `<board>.kisi.json`, the XIAO nRF54LM20A as a worked example |
| [Field and kisi.json reference](reference.md) | net class names, `PI_*` fields, every kisi.json key, other input files |
| [Models and limits](models.md) | how kisi computes, accuracy, validation, what is an estimate |
| [CLI reference](cli.md) | every command and option of `kisi.mjs` and the native `kisi` |
| [Using kisi with agents](agents.md) | the skills kisi-prepare and kisi-si-pi-review, setup |

The same pages as one PDF, `kisi-docs.pdf`, are attached to every
[release](https://github.com/riehlies/kisi/releases/latest) (built by `tools/docs_pdf.py`).

## Development documentation
Notes for those who work on kisi's models – why a model is as it is, what was tried, how it was checked. They are
not part of `kisi-docs.pdf`.

Research and concepts (`research/`):
- [accuracy-levels.md](research/accuracy-levels.md) – accuracy levels: which result gets a level and an
  uncertainty, where the numbers come from, how CLI and viewer show them.
- [thermal.md](research/thermal.md) – the temperature estimate: concept, physics and validation.
- [channel-export.md](research/channel-export.md) – the channel export to circuit and RF simulators: files, model,
  verification in ngspice, LTspice and KiCad.
- [vector-fitting.md](research/vector-fitting.md) – compact, causal, delay-preserving models for the SPICE export:
  vector fitting, delayed and characteristic line models, passivity, what the SPICE engines implement; why the
  export uses lossless-line cells.
- [kicad-simulation.md](research/kicad-simulation.md) – what KiCad's simulator (ngspice, KIBIS) can take from kisi.
- [channel-simulation-ibis.md](research/channel-simulation-ibis.md) – channel simulation with IBIS models: the
  staged concept behind assembly variants and the N-port channel (IBIS itself is out of scope since 0.3.0).
- [kicad-plugin.md](research/kicad-plugin.md) – the KiCad plugin: how KiCad installs and runs IPC plugins (with
  sources in KiCad's code), the package layout, what the plugin does, what only a test in KiCad can show.

Validation (`../validation/`):
- [fdtd/TESTS.md](../validation/fdtd/TESTS.md) – the 3D reference cases (kisi-fdtd) and kisi's models against
  them; [fdtd/BOARD_VIAS.md](../validation/fdtd/BOARD_VIAS.md), [BOARD_RESONANCES.md](../validation/fdtd/BOARD_RESONANCES.md),
  [BOARD_RETURNS.md](../validation/fdtd/BOARD_RETURNS.md) – vias, plane cavities and return paths of the example
  boards in 3D; [fdtd/README.md](../validation/fdtd/README.md) – the scripts.
- `channel_export/` – `compare.py` (the export in ngspice against kisi, also in CI), `kicad_sheet.py`,
  `ltspice.py`, and the studies behind the research notes (`compact_study.py`, `ltspice_microtests.py`).
- `golden/` – stored results of the example boards (`tools/golden.py`); the field-solver baselines
  (`field_baseline.json`, `*.ref.json`).

Board configurations: [examples/](examples/).

For contributors: [ARCHITECTURE.md](../ARCHITECTURE.md) (structure and rules), [CONTRIBUTING.md](../CONTRIBUTING.md),
[AGENTS.md](../AGENTS.md) (build and checks), [ROADMAP.md](../ROADMAP.md), [UPSTREAM.md](../UPSTREAM.md) (the field
solver), [validation/](../validation/) (reference data and test logs).
