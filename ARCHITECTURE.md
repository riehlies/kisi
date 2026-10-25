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
| `kisi-board` | parser, stackup, `StackupOverride`, geometry, `package` (case size of chip parts: footprint name, else pad geometry), viewer scene, `num` | – |
| `kisi-si` | `extract`, `analytic`, `xsgeom`, `rf/{line,channel,mixed,crosstalk,network,timedomain,touchstone_in}` (`mixed`: 4-port pair model and the alignment of a pair's two lines; `network`: N-port networks by modified nodal analysis – chains of 2-ports or coupled pairs, admittances, S-parameter blocks, sources; `timedomain`: step responses with DC point, Gaussian window and causality/settling checks), `lanenet` (a lane as a row of network blocks with the ends: transfer to every receiver, the pair's 4-port, the channel behind a connector – also one N-port file for all lanes of a connector, the other lanes as aggressors), `via` (board → via geometry, coax-section fallback; the return posts – vias and plated pads of plane nets, `plane_posts`; the GND/supply plane pairs in a via's path that no return via bridges, `open_pairs`, for the warning in `check`/`net`), `via_pbm` (physics-based via ladder: r–z Laplace capacitances, parallel-plate impedance with return vias, stubs, pairs, pours on the via's signal layers as coplanar returns; checked against kisi-fdtd; vias coupled through a shared cavity; return groups, option `--via-returns`: a return via shorts only the plane pairs whose two planes it joins, elsewhere a small ladder of its own – floating across a supply plane, capacitive where it ends at one –, capacitors between supply and ground planes nearby; `validation/fdtd/BOARD_RETURNS.md`), `via_cavity` (finite plane-pair cavities for the ladder, option `--via-cavity`: region of a plane pair on the board, rectangular modal series, grid of any shape with the exact near field of the vias, edge radiation; identical grids of several vias share one factorisation per frequency, `GridGroup`, filled by a registering pass in `kisi_core::service`; `validation/fdtd/BOARD_RESONANCES.md`), `eye`, `profile`, `topology` (device-to-device lanes, multi-drop taps) | kisi-board, kisi-sparse |
| `kisi-pi` | `pi` (DC), `tree` (regulator chains, `PI_REG`), `field_syntax` (net names in the power fields, quoting), `losses` (ground return, losses, loss map, power flow), `thermal` (steady-state temperature of the board and the parts from the losses on the DC solver's building blocks, `PI_THERMAL`, the IR drop at temperature), `pdn` (Z(f), plane model), `cavity_modal` (rectangular plane pair, double cosine series; loop inductance), `pdn_ic` (AC grid over all layers of the rail domain – the rail and the nets behind series resistors, inductors and ferrites up to the regulators –, PDN seen from every IC, filter resonances, recommendations) | kisi-board, kisi-sparse |
| `kisi-core` | facade with stable paths (`kisi_core::rf`, …), `service` (Project from files or texts, JSON summaries), `commands` (arguments and results of `kisi <command>`, shared by both CLIs), `par` (the check's rows on scoped threads), `config` (`<board>.kisi.json`), `variant` (assembly variants: KiCad 10 overrides, kisi.json rules by sheet/references/BOM, the “must choose” state), `gaps` | the three above |
| `kisi-wasm` | C ABI for the browser and for `kisi.mjs` (`kisi_cli_*`) | kisi-core |
| `kisi-cli` | `kisi` (JSON commands in `agent.rs`, text output for people), `kisi-validate` (impedance cases; `models`: the reduced models on the 3D reference cases; `via-geom`: diagnostics of one via; `via-case`: a board via as a kisi-fdtd case with its 2×thru fixtures, `validation/fdtd/BOARD_VIAS.md`; `cavities`: the plane pairs the channel vias of a board cross and their return groups; `cavity-model`: the ladder with a finite cavity per plane pair on a 3D case, `--returns` with return groups, `validation/fdtd/BOARD_RESONANCES.md`, `BOARD_RETURNS.md`) | kisi-core |
| `kisi-sparse` | sparse solver (real Cholesky/LU, complex LU for the AC grid), also built as a WASM module for the field solver | faer |
| `kisi-fdtd` | 3D FDTD (Yee, non-uniform mesh, conformal metal incl. polygon copper, CPML, lumped ports, S-parameters) for via transitions and board cut-outs; generates the via references (`validation/fdtd`, results in `validation/via_ref`); native binary and WASM (`fd_run`) | serde |

Rule: front ends only use `kisi-core`. Inside kisi-si/kisi-pi things can be restructured as long as
the facade stays the same. The channel verdict (`kisi_si::profile::judge`) exists exactly once;
viewer (via WASM) and CLI show the same result.

Channel data flow (`kisi_si::topology::lane_eval`, M2): lane hops → per line a cascade (`rf::Build`: lines, vias,
parts, side stubs; stubs that end at a receiver stay separate as taps) → a pair's two lines aligned into a 4-port
(`rf::mixed::align_pair`) → blocks (`lanenet::Block`: the board's cascade, 4-port or tapped line; the channel behind a
connector as a cascade or an S block, with the connector's other lanes as aggressor sources in an N-port file) →
`rf::network` with the ends → transfer to every receiver (and the crosstalk into it) with its DC point →
`rf::timedomain::Step` → eye (crosstalk as worst-case peak distortion). Loss and return loss come from the board's S-parameters (Sdd for pairs).

Parallel check: `service::check_plan` lists the rows (lanes, nets without devices) in a fixed order; each row is a
pure function of the project, computed on several engines and put back in that order – native: threads
(`kisi_core::par`, 4 by default, at most the cores, `KISI_THREADS`); Node CLI: worker threads, each with its own WASM
instance and project (`kisi_cli_check_part` → rows by index → `kisi_cli_run` with `check_rows`, the main thread
computes rows too); viewer: Web Workers that replay the engine's state calls (`Engine.replay`, `check-pool.js`) and
run `kisi_check`. Rule for model code: nothing a row computes may depend on what was computed before – caches are
memos of a function of their key (the via model computes from the key-quantised geometry; its caches are shared by
the native threads). `compare_cli.py` (both CLIs) and `check_viewer.py` require the parallel output to be identical
to one thread.

## Viewer (`web/`)
* `index.html` – markup only; `css/kisi.css`; scripts in `src/` in the order of
  `src/manifest.json` (shared scope):
  `engine` (WASM) · `units` (values with SI prefixes for every view: `fmtSI`, `parseSI`, `fieldSI`; tests
  `tools/test_units.mjs`) · `state` · `load` · `geometry` · `draw` (also `drawBoardLabels`: every text on the board – part
  labels of all maps, A/B marks – at a constant screen size, with the overlap rule; checked by `check_viewer.py`) · `charts` · `net-detail` · `si-tabs`
  (impedance view) · `channel` + `lanes` (channel view, interfaces overview) · `variants` (assembly variant card and
  prompt) · `panels` · `interaction` · `field-pool` ·
  `field-cache` · `check-pool` (the channel check's rows in Web Workers with engines of their own) · `crosstalk` (coupling scan with step 2, Couplings card, critical stretches on the map) · `pi-dc` · `pi-pdn` · `pi-balance` · `pi-loss` (losses map, power flow) · `pi-thermal` (temperature step and map) · `views` · `main` (start-up, files, hand-over).
* `tools/bundle.py` turns this into one standalone file `dist/kisi.html`: WASM modules and the field solver
  script (gzip + base64, unpacked at start), fonts (`web/fonts`, OFL), `data/*.json` and, optionally, example
  boards (`tools/fetch_examples.sh`) with their configuration `web/samples/<key>.kisi.json` → `kisi-examples.html`.
  The page loads nothing from the web.
* `web/cli/kisi.mjs` + `tools/bundle_cli.py` → `dist/kisi.mjs`: the same WASM engine and solver bundle
  (`tools/solver_bundle.py`, shared with the viewer) for Node; reads files, runs the field solver in worker
  threads with a disk cache and the rows of `check` in worker threads with engines of their own, everything else
  through `kisi_core::commands`.
* GitHub Pages (release workflow): `web/pages/index.html` (landing page) with `kisi.html`, `kisi-examples.html` and
  `kisi-docs.pdf`; the screenshots in `docs/img/` come from `tools/screenshots.py`.
* Files come in by drag and drop or the file dialog; with the File System Access API (Chromium) the viewer keeps
  the file handles and “Reload” reads them again after saving in KiCad.
* Stored in the browser (per viewer): field solver results (IndexedDB), stackup changes, profile changes,
  net class → profile assignment, power settings, lane directions, the chosen assembly variant. The results of the last three boards stay in memory for the session.

## Data (`data/`)
* `<board>.kisi.json` (next to the board, not in this repository) – per-board values with sources, see above.
* `components.json` – parasitics of passives in a channel (per package), reference-designator roles, decoupling
  data for the PDN (ESL/ESR by package, DC-bias derating, assumed ratings, candidate parts) and package thermal
  resistances (θJB, θJC per family and size) for the temperature estimate, with sources.
* `thermal.json` – material constants (copper, laminate, solder mask, air), convection correlations and defaults of
  the temperature estimate, with sources.
* `interfaces.json` – interface profiles with sources (standard / guideline / kisi guide value),
  transmitter/receiver, limits, assignment rules. Built into viewer and CLI.
* Example boards' configuration (power tree as PI_REG / PI_LOAD fields with sources): `web/samples/{ts,vna}.kisi.json`.
  `tools/tree_to_fields.py` converts a power tree JSON (`kisi power --tree`) into such fields.

## Field solver
See `UPSTREAM.md`: Forstén's js_2d_fields unmodified as a vendored copy (`third_party/UPSTREAM_REV`), one adapter, updates via
`tools/upstream_update.sh` with a check against the baseline (`tools/validate_field.sh`).

## Board configuration and agents
* `<board>.kisi.json` (`kisi_core::config`): values with sources that the board file lacks – stackup, net class
  targets and profiles, footprint fields (PI_LOAD/PI_SOURCE/PI_REG/PI_THERMAL/…), assembly variants. Laid over the board when
  loading (CLI and viewer); kisi never writes into KiCad files. Precedence: board file < kisi.json < viewer edits /
  CLI options.
* Assembly variants (`kisi_core::variant`): order when loading – board file → variant (KiCad 10 overrides, then the
  kisi.json rules of that name) → kisi.json fields. `kisi_si::topology::forks` finds forks where all alternative
  series parts are fitted. With variants or such forks and no choice, `Project::require_variant` stops the lane
  commands (`commands::needs_variant`) and the viewer asks before computing lanes. The WASM state keeps the board
  as read and rebuilds the analysed board when the viewer switches the variant.
* `kisi_core::gaps`: deterministic list of missing or default data, with where to find it and the kisi.json entry
  (supply rails also from net and pin names; the MPN where the Value field is not a part number).
* Worked example of a `<board>.kisi.json` (power tree with sources, the open gaps): `docs/preparing-a-board.md` with
  the file in `docs/examples/xiao-nrf54lm20a/`.
* Agents use the CLI (`node kisi.mjs <command> <board>` → JSON) following the skills in `claude-plugin/skills/`:
  `kisi-prepare` (gaps → research with sources → kisi.json → `kisi config-check`) and `kisi-si-pi-review`.
  An MCP server and a KiCad plugin existed until 2026-10 and were removed (git history) in favour of this.

## Documentation
* User documentation in `docs/` (Markdown, rendered by GitHub, linked from the README and the landing page):
  `getting-started.md`, `preparing-a-board.md`, `reference.md` (fields, kisi.json), `models.md` (models, accuracy,
  validation, estimates), `cli.md`, `agents.md`; `docs/README.md` is the index. One fact in one place: the README
  is a short entry page, the contributor files (`ARCHITECTURE.md`, `AGENTS.md`, `CONTRIBUTING.md`, `ROADMAP.md`,
  `UPSTREAM.md`) stay at the top level. The skills in `claude-plugin/skills/` repeat what an agent needs, because
  they are read without the repository.
* `docs/research/` – background research and concepts; `docs/examples/` – prepared board configurations.
* Kept in step with the program by `crates/kisi-core/tests/docs.rs` (part of `cargo test`): `docs/cli.md` carries
  `kisi help` verbatim (`KISI_BLESS_DOCS=1 cargo test -p kisi-core --test docs` rewrites it) and names every command
  and option; `docs/preparing-a-board.md` names every gap id of `kisi gaps`.
* `tools/docs_pdf.py` renders the pages (its `ORDER`) into `kisi-docs.pdf` with a table of contents – python-markdown
  and the headless Chromium of Playwright – and fails on links to missing files or headings.

## Releases
`.github/workflows/release.yml`: on a tag `v*` tests, field-solver validation, build of `kisi.html`,
`kisi-examples.html`, `kisi.mjs` and `kisi-docs.pdf`, attached to the GitHub release (the skills download `kisi.mjs`
from there), plus `kisi-references.html` when the reference cases changed.

## Checks after changes
`cargo test --release` · `tools/validate_field.sh` (field solver against HFSS/baseline) ·
`python3 tools/check_viewer.py dist/kisi-examples.html` (all views, light/dark/phone, no console errors) ·
`python3 tools/compare_cli.py` (Node CLI against the native CLI) · `python3 tools/docs_pdf.py` (documentation PDF
and link check of `docs/`) · `tools/golden.py` (stored results of the example
boards in `validation/golden/` and of the via models on the 3D references) · `tools/ci_checks.py` (values with sources,
Markdown links, CLI options named in the docs, Forstén's solver unchanged, reproducible bundles, bundle sizes).
`.github/workflows/ci.yml` runs them by what changed (`tools/ci_checks.py changes`), everything on main, by hand and
weekly (then also dependency audit and licences, `deny.toml`). Locally only the quick checks for what changed
(`AGENTS.md`); a branch goes to review with a green CI run.
