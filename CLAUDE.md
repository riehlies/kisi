# kisi – notes for working on the repository

* **The shared state is this GitHub repository** (`riehlies/kisi`, branch `main`).
  The folder `D:\Git\kisi` on the user's computer is only a local test folder (git clone; test builds
  in the gitignored `dist/`) – never treat it as the source.
* Structure and update rules: `ARCHITECTURE.md`. Field solver upstream: `UPSTREAM.md`.
* Language: GUI, documentation, code comments and tool output in English.
  Conversation with the user in German.

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
```
Releases: tag `v*` → `.github/workflows/release.yml` attaches `kisi.html`, `kisi-examples.html`, `kisi.mjs` and
`kisi-references.html` (built, not kept in the repository).
The example boards (ThunderScope CERN-OHL-P-2.0, LibreVNA GPL-3.0) are not in the repository;
`tools/fetch_examples.sh` fetches them at fixed commits and shrinks them; their power trees are
`web/samples/{ts,vna}.kisi.json`.

## Checks before every commit
* `cargo test --release`
* `sh tools/validate_field.sh` (field solver against HFSS/baseline, needs node)
* Viewer: `python3 tools/check_viewer.py dist/kisi.html` (headless Chromium: all three views,
  light/dark/mobile, no console errors; `--board`/`--config` for builds without example boards).
* CLI: `node dist/kisi.mjs` (e.g. `gaps`, `check`) on an example board; results match the native `kisi`
  with `--formula`.

## Rules
* One verdict logic (`kisi_si::profile::judge`) for viewer and CLI – do not duplicate it; CLI commands live
  once in `kisi_core::commands` (native and Node CLI).
* Values that change (standards, component parasitics) belong in `data/*.json`, with a source.
* Never change Forstén's solver in `third_party/js_2d_fields`; only adapt `web/solver/backend_forsten.js`.
* Mark estimated values as estimates in the UI and in CLI output.

## Commits
* **Commit and push only after the user has checked and approved the change.** Work in progress
  stays local (uncommitted); test builds go to `D:\Git\kisi\dist\` and the artifact for review.
* Author: `riehlies <118917105+riehlies@users.noreply.github.com>` (set in `.git/config`).
* End commit messages with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  no `Claude-Session` lines in commits or PRs.
