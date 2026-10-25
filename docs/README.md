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

Background:
- [research/channel-simulation-ibis.md](research/channel-simulation-ibis.md) – channel simulation with IBIS
  models: research and the staged concept behind assembly variants and the N-port channel.
- [research/thermal.md](research/thermal.md) – the temperature estimate: concept, physics and validation.
- [research/accuracy-levels.md](research/accuracy-levels.md) – accuracy levels: which result gets a level and an
  uncertainty, where the numbers come from, how CLI and viewer show them.
- [research/channel-export.md](research/channel-export.md) – the channel export to circuit and RF simulators:
  files, model, verification.
- [research/vector-fitting.md](research/vector-fitting.md) – compact, causal, delay-preserving models for the SPICE
  export: vector fitting, delayed and characteristic line models, passivity, what the SPICE engines implement.
- [examples/](examples/) – prepared board configurations; `mockups/` – design mockups of planned features.

For contributors: [ARCHITECTURE.md](../ARCHITECTURE.md) (structure and rules), [CONTRIBUTING.md](../CONTRIBUTING.md),
[AGENTS.md](../AGENTS.md) (build and checks), [ROADMAP.md](../ROADMAP.md), [UPSTREAM.md](../UPSTREAM.md) (the field
solver), [validation/](../validation/) (reference data and test logs).
