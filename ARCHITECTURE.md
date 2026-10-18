# kisi – architecture

Goal: parts can be updated one at a time without touching the rest. Every layer has a narrow
interface; data that changes (standards, component values) lives in files, not in code.

```
                 ┌──────────────── front ends ────────────────┐
  KiCad plugin ─►│ viewer (web/, one HTML file)   CLI   MCP    │◄─ agents (Claude Code, Konnect, …)
  (kicad-plugin) │   └─ WASM: crates/kisi-wasm   kisi-cli kisi-mcp
                 └──────────────┬─────────────────────────────┘
                                │  kisi-core (facade + service: load project, JSON summaries)
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
| `kisi-si` | `extract`, `analytic`, `xsgeom`, `rf/{line,channel,crosstalk}`, `via`, `eye`, `profile`, `topology` (device-to-device lanes), `ibis` (IBIS → linear ends, experimental – not working yet) | kisi-board |
| `kisi-pi` | `pi` (DC), `tree` (regulator chains, `PI_REG`), `pdn` (Z(f), plane model) | kisi-board, kisi-sparse |
| `kisi-core` | facade with stable paths (`kisi_core::rf`, …) and `service` | the three above |
| `kisi-wasm` | C ABI for the browser | kisi-core |
| `kisi-cli` | `kisi`, `kisi-validate` | kisi-core |
| `kisi-mcp` | MCP server (stdio, JSON-RPC) | kisi-core |
| `kisi-sparse` | sparse solver, also built as a WASM module for the field solver | faer |

Rule: front ends only use `kisi-core`. Inside kisi-si/kisi-pi things can be restructured as long as
the facade stays the same. The channel verdict (`kisi_si::profile::judge`) exists exactly once;
viewer (via WASM), CLI and MCP show the same result.

## Viewer (`web/`)
* `index.html` – markup only; `css/kisi.css`; scripts in `src/` in the order of
  `src/manifest.json` (shared scope):
  `engine` (WASM) · `state` · `load` · `geometry` · `draw` · `charts` · `net-detail` · `si-tabs`
  (impedance view) · `channel` + `lanes` (channel view, interfaces overview) · `panels` · `interaction` · `field-pool` ·
  `field-cache` · `pi-dc` · `pi-pdn` · `pi-balance` · `views` · `main` (start-up, files, hand-over).
* `tools/bundle.py` turns this into one standalone file `dist/kisi.html` (WASM, field solver modules,
  `data/*.json` and example boards embedded).
* Hand-over from outside: `const PRELOAD = null; /*KISI_PRELOAD*/` is replaced by the KiCad plugin or by
  `kisi_open_viewer` – the viewer itself stays independent.
* Stored in the browser (per viewer): field solver results (IndexedDB), stackup changes, profile changes,
  net class → profile assignment, power settings, lane directions. Loaded IBIS files and the results of the
  last three boards stay in memory for the session.

## Data (`data/`)
* `components.json` – parasitics of passives in a channel (per package) and reference-designator roles.
* `interfaces.json` – interface profiles with sources (standard / guideline / kisi guide value),
  transmitter/receiver, limits, assignment rules. Built into viewer, CLI and MCP; the MCP server
  can load its own file via `KISI_PROFILES`.
* Example power trees: `web/samples/*_tree.json`.

## Field solver
See `UPSTREAM.md`: Forstén's js_2d_fields unmodified as a submodule, one adapter, updates via
`tools/upstream_update.sh` with a check against the baseline (`tools/validate_field.sh`).

## Integration
* **KiCad** (`kicad-plugin/`): IPC API action; saves a copy of the open board and opens the viewer;
  `tools/package_plugin.py` builds the zip.
* **Agents** (`crates/kisi-mcp`, `claude-plugin/`): MCP server with a few tools; reads board files from
  disk (path e.g. from Konnect) and on request writes `analysis/kisi.json` (same location as kicad-happy).
  Claude Code plugin with a skill for the review workflow.

## Checks after changes
`cargo test --release` · `tools/validate_field.sh` (field solver against HFSS/baseline) ·
headless tests of the views with Playwright (see `tools/shot*.py`).
