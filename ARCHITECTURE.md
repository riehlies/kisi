# kisi – architecture

Goal: parts can be updated one at a time without touching the rest. Every layer has a narrow
interface; data that changes (standards, component values) lives in files, not in code.

```
          ┌──────────────────── front ends ────────────────────┐
  people ►│ viewer kisi.html        CLI kisi.mjs (Node)  kisi  │◄─ agents via skills (claude-plugin/)
          │  └─ WASM: crates/kisi-wasm ─┘ + field solver  (native,│
          │     (one engine)                  workers     formula)│
          └──────────────┬─────────────────────────────────────┘
   <board>.kisi.json ───►│  kisi-core (facade; service: load project, JSON summaries;
   (values + sources)    │             config: board configuration; gaps: what is missing)
              ┌─────────────────┼──────────────────┬──────────────┐
           kisi-si           kisi-pi           kisi-board      kisi-ibis
  cross-sections, formulas, DC IR drop,        parser .kicad_pcb/.kicad_pro,  IBIS parser,
  field solver geometry,    power tree,        stackup (+ overrides),        checks, values
                                                                              derived from tables
  line models, channel,     PDN impedance      geometry, scene, num (Cx, constants)
  vias, eye, crosstalk,            │
  profiles/verdict            kisi-sparse (faer) ── also the field solver's linear solver
              │
  data/interfaces.json (interface profiles)     third_party/js_2d_fields (Forstén, unmodified)
                                                 └─ web/solver/backend_forsten.js (the only adapter)
```

## Crates (Rust, `crates/`)
| Crate | Contents | depends on |
|---|---|---|
| `kisi-board` | parser, stackup, `StackupOverride`, geometry, `package` (case size of chip parts: footprint name, else pad geometry), viewer scene, `num` | – |
| `kisi-si` | `extract`, `analytic`, `xsgeom`, `rf/{line,channel,mixed,crosstalk}` (`mixed`: 4-port pair model), `via` (board → via geometry, coax-section fallback), `via_pbm` (physics-based via ladder: r–z Laplace capacitances, parallel-plate impedance with return vias, stubs, pairs; checked against kisi-fdtd), `eye`, `profile`, `topology` (device-to-device lanes) | kisi-board, kisi-sparse |
| `kisi-pi` | `pi` (DC), `tree` (regulator chains, `PI_REG`), `losses` (ground return, losses, loss map, power flow), `pdn` (Z(f), plane model), `cavity_modal` (rectangular plane pair, double cosine series; loop inductance), `pdn_ic` (AC grid over all layers of the rail domain – the rail and the nets behind series resistors, inductors and ferrites up to the regulators –, PDN seen from every IC, filter resonances, recommendations) | kisi-board, kisi-sparse |
| `kisi-ibis` | IBIS parser (`parse`/`read`: bytes in, data plus findings with line numbers; structure after KiCad's KIBIS, BSD-3-Clause, `NOTICE`), `check` (IBISCHK-like checks), `derive` (Ron/Roff at Vcc/2, DC levels into fixtures, edge times), `sha256`; no file system access (also WASM) | serde |
| `kisi-core` | facade with stable paths (`kisi_core::rf`, …), `service` (Project from files or texts, JSON summaries), `commands` (arguments and results of `kisi <command>`, shared by both CLIs), `config` (`<board>.kisi.json`), `variant` (assembly variants: KiCad 10 overrides, kisi.json rules by sheet/references/BOM, the “must choose” state), `gaps`, `ibis` (`kisi ibis` summary, kisi.json IBIS mapping check with the model files) | the four above |
| `kisi-wasm` | C ABI for the browser and for `kisi.mjs` (`kisi_cli_*`) | kisi-core |
| `kisi-cli` | `kisi` (JSON commands in `agent.rs`, text output for people), `kisi-validate` (impedance cases; `models`: the reduced models on the 3D reference cases; `via-geom`: diagnostics of one via) | kisi-core |
| `kisi-sparse` | sparse solver (real Cholesky/LU, complex LU for the AC grid), also built as a WASM module for the field solver | faer |
| `kisi-fdtd` | 3D FDTD (Yee, non-uniform mesh, conformal metal, CPML, lumped ports, S-parameters) for via transitions; generates the via references (`validation/fdtd`, results in `validation/via_ref`); native binary and WASM (`fd_run`) | serde |

Rule: front ends only use `kisi-core`. Inside kisi-si/kisi-pi things can be restructured as long as
the facade stays the same. The channel verdict (`kisi_si::profile::judge`) exists exactly once;
viewer (via WASM) and CLI show the same result.

## Viewer (`web/`)
* `index.html` – markup only; `css/kisi.css`; scripts in `src/` in the order of
  `src/manifest.json` (shared scope):
  `engine` (WASM) · `state` · `load` · `geometry` · `draw` · `charts` · `net-detail` · `si-tabs`
  (impedance view) · `channel` + `lanes` (channel view, interfaces overview) · `variants` (assembly variant card and
  prompt) · `panels` · `interaction` · `field-pool` ·
  `field-cache` · `crosstalk` (coupling scan with step 2, Couplings card, critical stretches on the map) · `pi-dc` · `pi-pdn` · `pi-balance` · `pi-loss` (losses map, power flow) · `views` · `main` (start-up, files, hand-over).
* `tools/bundle.py` turns this into one standalone file `dist/kisi.html`: WASM modules and the field solver
  script (gzip + base64, unpacked at start), fonts (`web/fonts`, OFL), `data/*.json` and, optionally, example
  boards (`tools/fetch_examples.sh`) with their configuration `web/samples/<key>.kisi.json` → `kisi-examples.html`.
  The page loads nothing from the web.
* `web/cli/kisi.mjs` + `tools/bundle_cli.py` → `dist/kisi.mjs`: the same WASM engine and solver bundle
  (`tools/solver_bundle.py`, shared with the viewer) for Node; reads files, runs the field solver in worker
  threads with a disk cache, everything else through `kisi_core::commands`.
* GitHub Pages (release workflow): `web/pages/index.html` (landing page) with `kisi.html` and `kisi-examples.html`;
  the screenshots in `docs/img/` come from `tools/screenshots.py`.
* Files come in by drag and drop or the file dialog; with the File System Access API (Chromium) the viewer keeps
  the file handles and “Reload” reads them again after saving in KiCad.
* Stored in the browser (per viewer): field solver results (IndexedDB), stackup changes, profile changes,
  net class → profile assignment, power settings, lane directions, the chosen assembly variant. The results of the last three boards stay in memory for the session.
  IBIS model files dropped with a board are never stored: they live in the page's memory (and the WASM state) only.

## Data (`data/`)
* `<board>.kisi.json` (next to the board, not in this repository) – per-board values with sources, see above.
* `components.json` – parasitics of passives in a channel (per package), reference-designator roles, and decoupling
  data for the PDN (ESL/ESR by package, DC-bias derating, assumed ratings, candidate parts), with sources.
* `interfaces.json` – interface profiles with sources (standard / guideline / kisi guide value),
  transmitter/receiver, limits, assignment rules. Built into viewer and CLI.
* Example boards' configuration (power tree as PI_REG / PI_LOAD fields with sources): `web/samples/{ts,vna}.kisi.json`.
  `tools/tree_to_fields.py` converts a power tree JSON (`kisi power --tree`) into such fields.

## Field solver
See `UPSTREAM.md`: Forstén's js_2d_fields unmodified as a vendored copy (`third_party/UPSTREAM_REV`), one adapter, updates via
`tools/upstream_update.sh` with a check against the baseline (`tools/validate_field.sh`).

## Board configuration and agents
* `<board>.kisi.json` (`kisi_core::config`): values with sources that the board file lacks – stackup, net class
  targets and profiles, footprint fields (PI_LOAD/PI_SOURCE/PI_REG), assembly variants, IBIS model mapping. Laid over
  the board when loading (CLI and viewer); kisi never writes into KiCad files. Precedence: board file < kisi.json <
  viewer edits / CLI options.
* IBIS mapping (`ibis` in kisi.json, `kisi_core::ibis`): file path and sha256 only, never the content. `config::check`
  validates what needs no file; `ibis::check_files` runs with the files (`config-check` in the CLIs – the front ends
  read them, `commands::uses_ibis_files` – and the viewer with dropped files) on the analysed board: hash, component,
  selectors, an IBIS pin with a model for every pad on a net with a target or a lane. `kisi ibis <file>` goes through
  `commands::run_file` (the front ends read the file, `commands::needs_file`). Not used by the channel models yet.
* Assembly variants (`kisi_core::variant`): order when loading – board file → variant (KiCad 10 overrides, then the
  kisi.json rules of that name) → kisi.json fields. `kisi_si::topology::forks` finds forks where all alternative
  series parts are fitted. With variants or such forks and no choice, `Project::require_variant` stops the lane
  commands (`commands::needs_variant`) and the viewer asks before computing lanes. The WASM state keeps the board
  as read and rebuilds the analysed board when the viewer switches the variant.
* `kisi_core::gaps`: deterministic list of missing or default data, with where to find it and the kisi.json entry.
* Agents use the CLI (`node kisi.mjs <command> <board>` → JSON) following the skills in `claude-plugin/skills/`:
  `kisi-prepare` (gaps → research with sources → kisi.json → `kisi config-check`) and `kisi-si-pi-review`.
  An MCP server and a KiCad plugin existed until 2026-10 and were removed (git history) in favour of this.

## Releases
`.github/workflows/release.yml`: on a tag `v*` tests, field-solver validation, build of `kisi.html`,
`kisi-examples.html` and `kisi.mjs`, attached to the GitHub release (the skills download `kisi.mjs` from there).

## Checks after changes
`cargo test --release` · `tools/validate_field.sh` (field solver against HFSS/baseline) ·
`python3 tools/check_viewer.py dist/kisi-examples.html` (all views, light/dark/phone, no console errors) ·
`python3 tools/compare_cli.py` (Node CLI against the native CLI); all of them run on every push (`.github/workflows/ci.yml`).
