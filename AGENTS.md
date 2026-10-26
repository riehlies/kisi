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
`kisi-references.html` (built, not kept in the repository) and publishes the live demo on GitHub Pages. The example
boards (ThunderScope CERN-OHL-P-2.0, LibreVNA GPL-3.0) are not in the repository; `tools/fetch_examples.sh` fetches
them at fixed commits and shrinks them; their power trees are `web/samples/{ts,vna}.kisi.json`.

## Checks
Locally, before pushing a branch, run the quick checks for what you changed:
* Rust: `cargo fmt --all` (settings in `rustfmt.toml`); `cargo clippy --release --workspace --all-targets -- -D
  warnings` and, for the wasm crates, `cargo clippy --release -p kisi-wasm -p kisi-sparse --target
  wasm32-unknown-unknown -- -D warnings` (deliberate exceptions: `[workspace.lints]` in `Cargo.toml`, or an `#[allow]`
  with its reason where it stands); `cargo test --release -p <crate>` for the changed crates and the crates that use
  a changed API; the release builds for the bundles must compile. CI fails on a formatting difference or a clippy
  warning. The Rust version is pinned in `rust-toolchain.toml` (rustup uses it locally and in CI); raising it is a
  change of its own, with the new lints fixed in the same commit.
* Viewer (`web/`, `kisi-wasm`): `python3 tools/check_viewer.py dist/kisi-examples.html --quick` (loads the
  boards, all three views, no console errors; `--board`/`--config` for builds without example boards), plus a
  short headless test of the controls you changed.
* CLI or `kisi_core::commands`: `python3 tools/compare_cli.py target/release/kisi dist/kisi.mjs examples`
  (Node CLI with `--formula` against the native `kisi` on both example boards).
* Field solver chain (`web/solver/`, the cross-section geometry, `tools/bundle.py`): `sh tools/validate_field.sh`
  (against HFSS/baseline, needs node).
* Documentation (`docs/`, `kisi help`, gap ids): `cargo test --release -p kisi-core --test docs` (the help text
  in `docs/cli.md`, every command, option and gap id) and `python3 tools/docs_pdf.py dist/kisi-docs.pdf` (the PDF;
  fails on links to missing files or headings in `docs/`).
* Channel export (`kisi_si::export`, `kisi export`): `python3 validation/channel_export/compare.py target/release/kisi
  examples/Thunderscope_Rev5.3.kicad_pcb --config web/samples/ts.kisi.json --out /tmp/export --lane SYNC_OUT_P --strict`
  (the exported netlist in ngspice against kisi's S-parameters; needs ngspice, numpy, optionally scikit-rf); add
  `--kicad` for the KiCad project (`validation/channel_export/kicad_sheet.py`: wiring, kicad-cli netlist and ERC).
* Model code (`crates/`, `data/`): `python3 tools/golden.py boards target/release/kisi examples` and
  `python3 tools/golden.py vias target/release/kisi` – the stored results of the example boards and kisi's models on
  the 3D via references. An intended change: run them with `--update` and commit `validation/golden/` and
  `validation/fdtd/results/kisi_models.json` with the change, so the review shows what moved.

CI (`.github/workflows/ci.yml`) runs the checks that a branch's changes since main can affect
(`tools/ci_checks.py changes`; a ROADMAP edit runs only the Markdown checks and the docs PDF); a push to main, a manual run and the
weekly run do everything, including the viewer in Firefox/WebKit and `kisi.mjs` on Windows. Before merging a branch
into main, run everything on it: `gh workflow run ci --ref <branch>` (or Actions → ci → Run workflow). Hand a branch
over for review only with a green CI run of its last commit; `main` changes only with green CI.

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
