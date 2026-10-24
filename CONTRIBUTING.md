# Contributing to kisi

kisi is a small side project, maintained when time allows. Reports and pull requests are welcome; answers
may take a while.

## Reporting a problem
Open an issue with:
- the kisi version (shown in the viewer's header, or `node kisi.mjs --version`) and where you ran it
  (viewer in which browser, or the command line);
- the KiCad version that saved the board;
- what you expected and what kisi showed. For results that look wrong, say what you compare against
  (fabricator stackup report, field solver, measurement, datasheet).
- If you can share it, the board (or a cut-down copy with the nets in question). The viewer never uploads
  your board; attaching it to an issue makes it public.

## Data
Interface profiles (`data/interfaces.json`) and component data (`data/components.json`) are the easiest
place to help: every value carries its source. Corrections with a reference (standard, datasheet,
application note) are very welcome.

## Documentation
The user documentation is in [`docs/`](docs/README.md). Corrections and clearer explanations are welcome; each fact
has one place there, and `python3 tools/docs_pdf.py` checks the links.

## Code
- Build and checks: see `AGENTS.md` (the same notes are used by coding agents).
- Structure and rules: `ARCHITECTURE.md`. Changes to the models need a comparison against the references in
  `validation/` (see `validation/fdtd/TESTS.md` for how the via and plane models were checked).
- The field solver in `third_party/js_2d_fields` is a vendored, unmodified copy of Henrik Forstén's
  js_2d_fields; changes to it belong upstream.
- By contributing you agree that your contribution is licensed under GPL-3.0-only, like the rest of kisi.
