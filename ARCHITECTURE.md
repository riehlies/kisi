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
              ┌─────────────────┼──────────────────┐
           kisi-si           kisi-pi           kisi-board
  cross-sections, formulas, DC IR drop,        parser .kicad_pcb/.kicad_pro,
  field solver geometry,    power tree,        stackup (+ overrides),
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
| `kisi-board` | parser, stackup, `StackupOverride`, geometry, viewer scene, `num` | – |
| `kisi-si` | `extract`, `analytic`, `xsgeom`, `rf/{line,channel,crosstalk}`, `via`, `eye`, `profile`, `topology` (device-to-device lanes) | kisi-board |
| `kisi-pi` | `pi` (DC), `tree` (regulator chains, `PI_REG`), `losses` (ground return, losses, loss map, power flow), `pdn` (Z(f), plane model) | kisi-board, kisi-sparse |
| `kisi-core` | facade with stable paths (`kisi_core::rf`, …), `service` (Project from files or texts, JSON summaries), `commands` (arguments and results of `kisi <command>`, shared by both CLIs), `config` (`<board>.kisi.json`), `gaps` | the three above |
| `kisi-wasm` | C ABI for the browser and for `kisi.mjs` (`kisi_cli_*`) | kisi-core |
| `kisi-cli` | `kisi` (JSON commands in `agent.rs`, text output for people), `kisi-validate` | kisi-core |
| `kisi-sparse` | sparse solver, also built as a WASM module for the field solver | faer |

Rule: front ends only use `kisi-core`. Inside kisi-si/kisi-pi things can be restructured as long as
the facade stays the same. The channel verdict (`kisi_si::profile::judge`) exists exactly once;
viewer (via WASM) and CLI show the same result.

## Viewer (`web/`)
* `index.html` – markup only; `css/kisi.css`; scripts in `src/` in the order of
  `src/manifest.json` (shared scope):
  `engine` (WASM) · `state` · `load` · `geometry` · `draw` · `charts` · `net-detail` · `si-tabs`
  (impedance view) · `channel` + `lanes` (channel view, interfaces overview) · `panels` · `interaction` · `field-pool` ·
  `field-cache` · `pi-dc` · `pi-pdn` · `pi-balance` · `pi-loss` (losses map, power flow) · `views` · `main` (start-up, files, hand-over).
* `tools/bundle.py` turns this into one standalone file `dist/kisi.html`: WASM modules and the field solver
  script (gzip + base64, unpacked at start), fonts (`web/fonts`, OFL), `data/*.json` and, optionally, example
  boards (`tools/fetch_examples.sh`) with their configuration `web/samples/<key>.kisi.json` → `kisi-examples.html`.
  The page loads nothing from the web.
* `web/cli/kisi.mjs` + `tools/bundle_cli.py` → `dist/kisi.mjs`: the same WASM engine and solver bundle
  (`tools/solver_bundle.py`, shared with the viewer) for Node; reads files, runs the field solver in worker
  threads with a disk cache, everything else through `kisi_core::commands`.
* Files come in by drag and drop or the file dialog; with the File System Access API (Chromium) the viewer keeps
  the file handles and “Reload” reads them again after saving in KiCad.
* Stored in the browser (per viewer): field solver results (IndexedDB), stackup changes, profile changes,
  net class → profile assignment, power settings, lane directions. The results of the last three boards stay in memory for the session.

## Data (`data/`)
* `<board>.kisi.json` (next to the board, not in this repository) – per-board values with sources, see above.
* `components.json` – parasitics of passives in a channel (per package) and reference-designator roles.
* `interfaces.json` – interface profiles with sources (standard / guideline / kisi guide value),
  transmitter/receiver, limits, assignment rules. Built into viewer and CLI.
* Example boards' configuration (power tree as PI_REG / PI_LOAD fields with sources): `web/samples/{ts,vna}.kisi.json`.
  `tools/tree_to_fields.py` converts a power tree JSON (`kisi power --tree`) into such fields.

## Field solver
See `UPSTREAM.md`: Forstén's js_2d_fields unmodified as a vendored copy (`third_party/UPSTREAM_REV`), one adapter, updates via
`tools/upstream_update.sh` with a check against the baseline (`tools/validate_field.sh`).

## Board configuration and agents
* `<board>.kisi.json` (`kisi_core::config`): values with sources that the board file lacks – stackup, net class
  targets and profiles, footprint fields (PI_LOAD/PI_SOURCE/PI_REG). Laid over the board when loading (CLI and
  viewer); kisi never writes into KiCad files. Precedence: board file < kisi.json < viewer edits / CLI options.
* `kisi_core::gaps`: deterministic list of missing or default data, with where to find it and the kisi.json entry.
* Agents use the CLI (`node kisi.mjs <command> <board>` → JSON) following the skills in `claude-plugin/skills/`:
  `kisi-prepare` (gaps → research with sources → kisi.json → `kisi config-check`) and `kisi-si-pi-review`.
  An MCP server and a KiCad plugin existed until 2026-10 and were removed (git history) in favour of this.

## Releases
`.github/workflows/release.yml`: on a tag `v*` tests, field-solver validation, build of `kisi.html`,
`kisi-examples.html` and `kisi.mjs`, attached to the GitHub release (the skills download `kisi.mjs` from there).

## Checks after changes
`cargo test --release` · `tools/validate_field.sh` (field solver against HFSS/baseline) ·
`python3 tools/check_viewer.py dist/kisi.html` (all views, light/dark/phone, no console errors) · `node dist/kisi.mjs` on a board.
