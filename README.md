# kisi – signal and power integrity checks for KiCad

kisi reads a KiCad layout (`.kicad_pcb` + `.kicad_pro`) and checks trace impedance, high-speed channels
(loss, return loss, TDR, vias, crosstalk, skew, eye) and power delivery (regulator chain, DC IR drop,
PDN impedance). It is a design check (impedance target ±5 %), not sign-off. Licence: GPL-3.0-only.

## Quick start
Each release on GitHub has these files:

- **`kisi.html` – the viewer.** Open it in a browser and drop the `.kicad_pcb` (with the `.kicad_pro` for net
  classes, optionally `<board>.kisi.json`). Nothing is installed and nothing is loaded from the web; the board is
  processed in the browser and never uploaded. In Chromium-based browsers “Reload” reads the same files again after
  saving in KiCad.
- **`kisi-examples.html`** – the same viewer with two example boards built in (ThunderScope Rev5.3, LibreVNA), each
  with its power tree as a `kisi.json` (`web/samples/`).
- **`kisi.mjs` – the command line** for scripts and agents (Node.js ≥ 18):
  ```sh
  node kisi.mjs help
  node kisi.mjs gaps   board.kicad_pcb     # what the analyses miss
  node kisi.mjs check  board.kicad_pcb     # channel check of all interfaces
  ```
  Every command prints JSON and states the models and assumptions it used.
- **Agent skills** (`claude-plugin/`, Claude Code: `/plugin marketplace add riehlies/kisi`): *kisi-prepare*
  finds missing data and researches it from datasheets into `<board>.kisi.json`; *kisi-si-pi-review* runs the checks.

## Viewer
Three views, each starting with what the board file gives and computing more on demand:
1. **Impedance** – stackup and target impedances first; “Estimate impedances” (formulas), then the 2D field solver
   (selected net, a class, all nets with a target, or everything; results cached in the browser). Per net:
   impedance along the path, loss, TDR, vias, crosstalk. “Calculate couplings” estimates NEXT/FEXT for every net
   with a target (map mode “Crosstalk”, list of critical nets from 1 %) and sends only the critical couplings to the
   field solver; per net a chart shows where along the path the crosstalk comes from. The path runs from A to B (marked on the board and under
   the charts; A is the transmitter side of the lane or the IC, ⇄ swaps it); branches are marked where they leave
   the path and included in loss and TDR as open stubs; hovering a chart shows the position on the board.
2. **Channel** – interfaces overview: buses and device-to-device lanes through series resistors and AC-coupling
   capacitors, with terminations and branches; per net class an interface profile (PCIe, USB, DDR, LVDS, …
   from `data/interfaces.json`) with transmitter, receiver and limits. Eye only from IC to IC; lanes ending at a
   connector are judged on the board share (loss, return loss, skew).
3. **Power** – power tree, budget and power flow (Sankey) first; then voltage drop and current density per rail
   and of the ground return, a losses map (heat per part and copper loss per layer, mW/mm²), and Z(f).

Stackup (εr, tanδ, thickness) and profiles can be edited in the viewer; edits are stored in the browser, the
files are not changed. Results of the last three boards stay in memory while switching.

## Command line
`node kisi.mjs <command> <board.kicad_pcb> [options]`

| Command | Result |
|---|---|
| `overview` | stackup, impedance per net class (share within ±5 %, worst nets) |
| `interfaces` | buses and device-to-device lanes with their profiles |
| `net <NET>` | path (start, end, branches), impedance along it, loss, TDR, vias, crosstalk; `--s2p FILE` writes Touchstone |
| `check` | channel check against the profiles (`--class CLASS=PROFILE`, `--nets A,B`, `--all`) |
| `crosstalk` | coupling scan of all nets with a target, critical from 1 % (`--nets A,B`, `--all` for every row); the field solver refines the critical couplings |
| `power` | regulator chain, budget, DC IR drop per rail and in the ground return, losses per part / rail / layer, power flow (`--pdn RAIL` adds Z(f)) |
| `report` | everything into `analysis/kisi.json` next to the board |
| `gaps` | missing or default data, where to find it, how to write it into `<board>.kisi.json` |
| `config-check`, `config-template` | check / start a `<board>.kisi.json` |
| `profiles [ID]` | the interface profiles |

`overview`, `net`, `check`, `crosstalk` and `report` run the field solver for the cross-sections of nets with a target, the most
routed length first, within `--field-time` seconds (default 120). Results are cached
(`~/.cache/kisi`, Windows `%LOCALAPPDATA%\kisi`), so later runs are fast; `field_solver` in the output shows the
coverage, `--formula` skips the solver. A native build (`cargo install --path crates/kisi-cli`) accepts the same
commands with formula impedances only.

## Information kisi takes from KiCad
- **Target impedance** from net class names: `50Z`, `90Z_Diff`, `100Z_Diff`, … (or `<board>.kisi.json`).
- **Interface profile** automatically from net and class names (rules in `data/interfaces.json`), or per class.
- **Power** – fields on the symbol, copied to the footprint on “Update PCB from schematic”:

| Field | Example | Meaning |
|---|---|---|
| `PI_REG` | `+12V input` | supply input (connector) |
| | `+12V > +5V buck 90% 3A out=L3` | switching regulator: efficiency, rated current, part carrying the output |
| | `+5V > +3V3 ldo iq=2mA 1A` | linear regulator with quiescent current |
| | `+3V3 > +3V3A pass r=0.05` | ferrite or shunt with series resistance (Ω) |
| | `+5V > VCORE@0.9V buck 85% id=U4A` | rail without a voltage in its name; several channels of one IC with `;` |
| `PI_LOAD` | `+1V0=2.5A, +1V8=300mA` or `+1V0=2W` | load current per rail |
| | `+5V=60mA@L32/L33` | current entering at other parts' pads (e.g. the inductors of an output stage) |
| `PI_SOURCE` | `+3V3` | source pads for the IR drop when there is no regulator chain |

  The nominal voltage comes from the net name (`+3V3`, `1V8`, `-5V`). A `kipida_config.json` (KiPIDA) can be
  dropped into the viewer as well.

## Board configuration `<board>.kisi.json`
Values the board file lacks or holds only as KiCad defaults, each with its source. kisi never writes into the
KiCad files; CLI and viewer lay this file over the board.
```json
{
  "kisi_config": 1,
  "board": "board.kicad_pcb",
  "stackup": {"dielectrics": [{"index": 0, "epsilon_r": 3.65, "loss_tangent": 0.0095,
                               "source": {"kind": "datasheet", "document": "Isola FR408HR", "page": "2", "note": "10 GHz, typical"}}]},
  "net_classes": {"PCIe": {"target_ohm": 85, "diff": true, "profile": "pcie3", "source": {"kind": "standard", "document": "PCIe CEM 3.0"}}},
  "fields": [{"ref": "U5", "name": "PI_LOAD", "value": "+1V0=2.5A", "source": {"kind": "datasheet", "url": "…", "note": "maximum"}}]
}
```
- `stackup`: εr, tanδ, thickness, material per dielectric (index 0 = below F.Cu), solder mask εr.
- `net_classes`: target impedance and profile of existing net classes. `fields`: footprint fields (`PI_LOAD`, `PI_REG`, `PI_SOURCE`).
- Source kinds: datasheet, standard, application_note, measurement, simulation, estimate, assumption.
- Precedence: board file < kisi.json < edits in the viewer / `--class` on the command line.

## Models and accuracy
- **Impedance:** formulas (Hammerstad-Jensen, Wheeler, Wadell CPWG, IPC-2141) and the quasi-static 2D field solver
  js_2d_fields by Henrik Forstén (unmodified, see `UPSTREAM.md`; linear solver in Rust, `crates/kisi-sparse`).
  Cross-sections along every line with reference planes (also across gaps), coplanar ground and diff partner.
  Field solver against HFSS/Ansys: ≤ 1.7 %.
- **Lines:** RLGC per cross-section; skin effect (Wheeler), roughness (Huray, presets estimated), dielectric loss
  (Djordjevic-Sarkar); RLGC against HFSS ≤ 2 %. Channel as ABCD cascade (odd mode for pairs), TDR from S11.
- **Vias:** stack of coaxial sections (antipads from the zone fills), stubs, back-drill, return path; an estimate
  (about ±15 %), not a 3D result.
- **Lanes:** series parts and terminations with package parasitics from `data/components.json` (estimates); the
  transmit direction is a guess (termination next to the receiver) and can be swapped.
- **Eye:** PRBS7 NRZ with the profile's linear transmitter/receiver; no equalisation, jitter or noise.
- **Power:** copper rasterised per layer, vias as barrels (25 µm); PDN as lumped model plus a grid model of the
  largest plane; capacitor ESR/ESL estimated per package.
- **Ground return and losses:** every part returns what it draws from the supply rails minus what it feeds into
  them (a buck converter takes current from ground); ground is solved like a rail with the supply input as 0 V.
  The power a load draws is counted as heat in the part (estimate), regulator losses on the regulator; copper
  loss is I²R from the DC solutions. Power, not temperature.

## Development
```sh
cargo build --release                                                    # native CLI, kisi-validate
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
W=target/wasm32-unknown-unknown/release
python3 tools/bundle.py web/index.html $W/kisi_wasm.wasm $W/kisi_sparse.wasm dist/kisi.html
sh tools/fetch_examples.sh examples                                     # example boards (optional)
python3 tools/bundle.py web/index.html $W/kisi_wasm.wasm $W/kisi_sparse.wasm dist/kisi-examples.html \
        ts "ThunderScope Rev5.3" examples/Thunderscope_Rev5.3.kicad_pcb examples/Thunderscope_Rev5.3.kicad_pro \
        vna LibreVNA examples/VNA.kicad_pcb examples/VNA.kicad_pro
python3 tools/bundle_cli.py $W/kisi_wasm.wasm $W/kisi_sparse.wasm dist/kisi.mjs
```
- Checks: `cargo test --release`, `sh tools/validate_field.sh` (field solver against HFSS and the stored baseline),
  `python3 tools/check_viewer.py dist/kisi.html` (all views, light/dark/phone, no console errors).
- Example boards (not in the repository): `sh tools/fetch_examples.sh examples` fetches them at fixed commits, shrinks
  them with `tools/strip_pcb.py` and writes their licence notices; their power trees are `web/samples/{ts,vna}.kisi.json`.
- Releases: a tag `v*` runs `.github/workflows/release.yml` and attaches `kisi.html`, `kisi-examples.html` and `kisi.mjs`.
- Solver update: `tools/upstream_update.sh` (see `UPSTREAM.md`). Structure and rules: `ARCHITECTURE.md`.
  Planned work: `ROADMAP.md`.
- Validation data: `validation/*.json` (cross-sections) and `*.ref.json` (references from Forstén's solver, run in
  Node with `tools/forsten_sparse_shim.js`); `kisi-validate validation/forsten_suite.json` compares against the
  HFSS/Ansys cases of Forstén's test suite.

## Credits
Field solver: [js_2d_fields](https://github.com/Ttl/js_2d_fields) by Henrik Forstén (GPL-3.0).
Example boards in `kisi-examples.html`: [ThunderScope](https://github.com/EEVengers/ThunderScope) Rev5.3 (EEVengers,
hardware under CERN-OHL-P-2.0) and [LibreVNA](https://github.com/jankae/LibreVNA) (Jan Käberich, GPL-3.0), shrunk to
what kisi reads; the licence texts are embedded with them.
Fonts: Barlow, Barlow Semi Condensed and JetBrains Mono (SIL Open Font License 1.1, `web/fonts/`), embedded in the viewer.
