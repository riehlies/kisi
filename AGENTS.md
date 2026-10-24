# kisi – notes for coding agents and contributors

* Structure, data flow and update rules: `ARCHITECTURE.md`. Field solver upstream: `UPSTREAM.md`.
  Planned work: `ROADMAP.md`. How to contribute: `CONTRIBUTING.md`.
* Language: GUI, documentation, code comments and tool output in English.

## Building
```
cargo build --release                                   # CLI (kisi, kisi-validate)
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
python3 tools/bundle.py web/index.html \
  target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
  target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.html
sh tools/fetch_examples.sh examples                     # example boards → dist/kisi-examples.html:
python3 tools/bundle.py web/index.html ... dist/kisi-examples.html \
  ts "ThunderScope Rev5.3" examples/Thunderscope_Rev5.3.kicad_pcb examples/Thunderscope_Rev5.3.kicad_pro \
  vna LibreVNA examples/VNA.kicad_pcb examples/VNA.kicad_pro
python3 tools/bundle_cli.py target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
  target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.mjs   # Node CLI with field solver
python3 validation/fdtd/make_viewer.py validation/via_ref dist/kisi-references.html  # 3D reference cases page
python3 tools/docs_pdf.py dist/kisi-docs.pdf             # docs/ as one PDF (pip: markdown playwright pypdf)
```
Releases: a tag `v*` runs `.github/workflows/release.yml`; it attaches `kisi.html`, `kisi-examples.html`,
`kisi.mjs`, `kisi-docs.pdf` and – when the reference cases changed since the previous release –
`kisi-references.html` (built, not kept in the repository) and publishes the live demo on GitHub Pages. The example boards (ThunderScope
CERN-OHL-P-2.0, LibreVNA GPL-3.0) are not in the repository; `tools/fetch_examples.sh` fetches them at fixed
commits and shrinks them; their power trees are `web/samples/{ts,vna}.kisi.json`.

## Checks
Locally, before pushing a branch, run the quick checks for what you changed:
* Rust: `cargo test --release -p <crate>` for the changed crates and the crates that use a changed API; the
  release builds for the bundles must compile.
* Viewer (`web/`, `kisi-wasm`): `python3 tools/check_viewer.py dist/kisi-examples.html --quick` (loads the
  boards, all three views, no console errors; `--board`/`--config` for builds without example boards), plus a
  short headless test of the controls you changed.
* CLI or `kisi_core::commands`: `python3 tools/compare_cli.py target/release/kisi dist/kisi.mjs examples`
  (Node CLI with `--formula` against the native `kisi` on both example boards).
* Field solver chain (`web/solver/`, the cross-section geometry, `tools/bundle.py`): `sh tools/validate_field.sh`
  (against HFSS/baseline, needs node).
* Documentation (`docs/`, `kisi help`, gap ids): `cargo test --release -p kisi-core --test docs` and
  `python3 tools/docs_pdf.py dist/kisi-docs.pdf` (link check).

The full suite runs in CI on every push and pull request (`.github/workflows/ci.yml`): `cargo test --release`,
`validate_field.sh`, `check_viewer.py` in light/dark/phone, `compare_cli.py`, the reference page and the docs
PDF. Hand a branch over for review only with a green CI run of its last commit; `main` changes only with green CI.

## Rules
* One verdict logic (`kisi_si::profile::judge`) for viewer and CLI – do not duplicate it; CLI commands live
  once in `kisi_core::commands` (native and Node CLI).
* Values that change (standards, component parasitics) belong in `data/*.json`, with a source.
* Never change Forstén's solver in `third_party/js_2d_fields`; only adapt `web/solver/backend_forsten.js`.
* Mark estimated values as estimates in the UI and in CLI output.
* User documentation lives in `docs/`, one fact in one place; a new command, option, `PI_*` field, kisi.json key
  or gap goes into `docs/cli.md`, `docs/reference.md` or `docs/preparing-a-board.md` in the same change.
* Model changes need a comparison against the references (`validation/`, `validation/fdtd/TESTS.md`).

## Commits
* Work on a branch and open a pull request; `main` is changed only through reviewed merges.
* Mark AI-assisted commits with a `Co-Authored-By:` trailer.
