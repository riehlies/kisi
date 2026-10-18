# kisi – notes for working on the repository

* **The shared state is this GitHub repository** (`riehlies/kisi`, branch `main`).
  The folder `D:\Git\kisi` on the user's computer is only a local test folder for the integration
  with KiCad (plugin) and the MCP server – never treat it as the source.
* Structure and update rules: `ARCHITECTURE.md`. Field solver upstream: `UPSTREAM.md`.
* Language: GUI, documentation, code comments and tool output in English.
  Conversation with the user in German.

## Building
```
cargo build --release                                   # CLI (kisi, kisi-validate) and kisi-mcp
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
python3 tools/bundle.py web/index.html \
  target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
  target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.html \
  [ts "ThunderScope Rev5.3" ts.kicad_pcb ts.kicad_pro vna "LibreVNA" vna.kicad_pcb vna.kicad_pro]
python3 tools/package_plugin.py                         # dist/kisi-kicad-plugin.zip
```
The example boards (ThunderScope, LibreVNA) are not in the repository; they come from their
projects and are shrunk with `tools/strip_pcb.py`.

## Checks before every commit
* `cargo test --release`
* `sh tools/validate_field.sh` (field solver against HFSS/baseline, needs node)
* Viewer: open the bundled file headless in Chromium (Playwright), all three views,
  light/dark/mobile, no console errors.

## Rules
* One verdict logic (`kisi_si::profile::judge`) for viewer, CLI and MCP – do not duplicate it.
* Values that change (standards, component parasitics) belong in `data/*.json`, with a source.
* Never change Forstén's solver in `third_party/js_2d_fields`; only adapt `web/solver/backend_forsten.js`.
* Mark estimated values as estimates in the UI and in MCP output.

## Commits
* **Commit and push only after the user has checked and approved the change.** Work in progress
  stays local (uncommitted); test builds go to `D:\Git\kisi\dist\` and the artifact for review.
* Author: `riehlies <118917105+riehlies@users.noreply.github.com>` (set in `.git/config`).
* End commit messages with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
  no `Claude-Session` lines in commits or PRs.
