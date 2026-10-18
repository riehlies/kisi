# kisi – signal and power integrity checks for KiCad

kisi extracts trace impedances from KiCad layouts (`.kicad_pcb` + `.kicad_pro`) and shows them as a colour map,
together with loss, TDR, vias, crosstalk, an eye diagram per interface profile, DC IR drop and PDN impedance.
The core is written in Rust and runs as WebAssembly in the browser and as a native CLI. Licence: GPL-3.0-only.

kisi is meant as a design check (target ±5 %), not as sign-off.

Structure and update rules: `ARCHITECTURE.md`. Integration: KiCad plugin (`kicad-plugin/`), MCP server for
AI agents (`crates/kisi-mcp`, Claude Code plugin in `claude-plugin/`).

## Workflow in the viewer
1. Load a board (`.kicad_pcb`, optionally `.kicad_pro`, `.ibs` IBIS models, `kipida_config.json`).
2. The fast analysis runs immediately: impedance by formula, DC IR drop of all configured rails.
3. Start the field solver for the selected net, one net class, all nets with a target, or everything.
4. **Impedance** view: line analysis (impedance, loss, TDR, vias, crosstalk) against a reference resistance.
   **Channel** view: “Interfaces” overview of all buses and device-to-device lanes (through series resistors
   and AC-coupling capacitors, with terminations), interface profile per net class (PCIe, USB, DDR, … from
   `data/interfaces.json`), transmitter and receiver (profile or linearised IBIS model), limits – eye from IC to IC,
   board share for lanes ending at a connector.
   **Power** view: IR drop, power budget, Z(f).
5. Stackup (εr, tanδ, thickness) and profiles can be edited in the viewer (stored in the browser; the file is not changed).
   Field solver results are cached in the browser (IndexedDB) across boards; key = solver version + geometry.

## DC IR drop (power)
- The copper of a net is rasterised per layer (zones, tracks, pads); vias and through-hole pads connect the layers
  (barrel resistance with 25 µm copper). Source pads sit at the nominal voltage, load pads draw their current.
- Result: voltage and current density per layer, voltage at every load, source and via currents, power loss.
- Loads in KiCad: a field on the footprint (add it in the schematic; it reaches the board on update)
  - `PI_LOAD` = `+1V0=2.5A, +1V8=300mA` or `+1V0=2W` (power → current via the nominal voltage)
  - `PI_SOURCE` = `+3V3` or `+3V3=3.3V` for regulator outputs and supply inputs
- Otherwise the nominal voltage comes from the net name (`+3V3`, `1V8`, `-5V`, `+1.2V`).
- KiPIDA: `kipida_config.json` is read; regulators become loads on their input rail.
- CLI: `kisi board.kicad_pcb --pi` (fields), `--pi-config kipida_config.json` or a JSON rail list; `--rails` lists supply nets.
- Validation: copper strip against R = ρL/(wt), deviation ≤ 2.5 % (test `strip_resistance`).

## Power tree and power budget
- `web/samples/ts_tree.json`, `web/samples/vna_tree.json`: regulator chains of ThunderScope and LibreVNA, reconstructed
  from the netlist. Elements: `input`, `buck`, `boost`, `ldo` (quiescent current `iq`), `inverter`, `pass` (shunt/ferrite
  with `r`); loads per component and net with current and basis (`datasheet` or `estimate`).
- `crates/kisi-pi/src/tree.rs`: works from the loads back to the inputs and returns currents, losses per regulator,
  power per component (exact balance: input = consumption + losses) and the sources/loads of every rail for the IR drop.
- CLI: `kisi board.kicad_pcb --pi-tree web/samples/ts_tree.json`
- Viewer: Power → Power budget; currents are editable there (stored in the browser).

### Regulator chains directly in KiCad: field `PI_REG`
Add a field `PI_REG` to the regulator, supply input or pass element (ferrite, shunt) in the schematic; it reaches
the board on update. Separate several channels of one IC with `;`:

| Example | Meaning |
|---|---|
| `+12V input` | supply input (connector) |
| `+12V > +5V buck 90% 3A out=L3` | switching regulator, efficiency, rated current, part carrying the output current |
| `+5V > +3V3 ldo iq=2mA 1A` | linear regulator with quiescent current |
| `+3V3 > +3V3A pass r=0.05` | ferrite / shunt with series resistance (Ω) |
| `+5V > -5V inverter 70%` | inverting converter |
| `+5V > VCORE@0.9V buck 85% id=U4A` | rail without a voltage in its name (`@`), own id per channel |

`->` or `→` work instead of `>`. Loads come from `PI_LOAD` (`+1V0=2.5A, +1V8=300mA` or `+1V0=2W`).
If `PI_REG` fields exist, they replace the example tree in the viewer; “As KiCad fields” in the power view prints the
current tree in this form. CLI: `kisi board.kicad_pcb --pi-tree fields`.
`tools/tree_to_fields.py` writes a JSON tree as fields into a copy of the board (migration; test: both example boards
give the same budget with the fields as with the JSON tree).

## PDN impedance Z(f)
- `crates/kisi-pi/src/pdn.rs`: lumped model per rail – regulator (R + L) ∥ plane capacitance (zone area over the
  nearest ground layer) ∥ all capacitors between rail and ground. Value from the component value (`100n`, `4u7`,
  `10µF/6.3V`), package from the pad spacing, ESR/ESL estimated, mounting inductance through the vias to the plane,
  spreading inductance to the load centroid.
- Plane model from 1 MHz: the largest area of the rail as a grid (up to 2500 cells) over ideal ground, μ0·d per square,
  ε·A/d per cell, capacitors and regulator at their positions, solved with a complex band LU per frequency.
  Shows the influence of the shape and resonances (test: 50 × 30 mm rectangle, first resonance within 3 %).
- Target impedance = 3 % of the voltage / (50 % of the load current); evaluated up to 100 MHz or the first cavity resonance.
- CLI: `kisi board.kicad_pcb --pi-tree web/samples/ts_tree.json --pdn +1V0`; viewer: Power → rail → “Impedance Z(f)”.

## RF: channel, loss, TDR, crosstalk, vias, eye
- `crates/kisi-si/src/rf/`: frequency-dependent RLGC per cross-section, from the field solver (R, L, G, C at 1 GHz)
  or from formulas. Skin effect after Wheeler (incremental inductance), roughness after Huray (smooth / HVLP / standard ED,
  presets estimated), dielectric after Djordjevic-Sarkar from the stackup εr and tanδ, microstrip dispersion.
- Channel of a net: longest path through the track graph (layers joined only by vias / through-hole pads),
  ABCD cascade → S21/S11 (odd mode / Sdd for pairs), Touchstone `.s2p`, TDR from S11 with a Gaussian edge.
- `crates/kisi-si/src/via.rs`: a via as a stack of coaxial sections per layer; outer conductor = clearance (antipad)
  measured in the zone fills; for pairs the Wheeler formula for a shielded symmetric pair. Stubs as open lines
  (quarter-wave resonance), back-drill from KiCad 10 (`backdrill`) or global. Reference plane change: inductance to
  the nearest stitching via; warning if there is none within 5 mm or the reference changes to another net.
- Crosstalk: neighbouring lines on the same layer (parallel, up to 5 h away), NEXT/FEXT from the L/C coupling matrix
  of the field solver or Kirschning-Jansen/Cohn; field solver scope “selected net” includes the coupled cross-sections.
- Eye diagram (`crates/kisi-si/src/eye.rs`): PRBS7 NRZ from the single-bit response between the profile's linear
  transmitter and receiver (R, C, package L per line), no equalisation, jitter or noise. CLI: `--rf <net> --eye 16` (Gbit/s).
- Channel check: all nets with a target impedance (one row per pair) with loss at Nyquist, return loss, eye
  height/width, intra-pair skew and via stub resonance; traffic light against the profile limits. The verdict logic
  (`kisi_si::profile::judge`) is shared by viewer, CLI and MCP.
- CLI: `kisi board.kicad_pcb board.kicad_pro --rf <net suffix> [--s2p out_{}.s2p]`
- Accuracy: line RLGC against HFSS ≤ 2 %; via impedance is an estimation model (about ±15 %), not a 3D result.

## Device-to-device lanes (M7)
- `crates/kisi-si/src/topology.rs`: nets with a target impedance are followed through two-pin passives – series
  resistors up to 100 Ω, AC-coupling capacitors from 1 nF, small inductors – to the pins of ICs and connectors.
  Passives to ground/supply or across a pair are terminations; DNP parts are skipped. Lanes with the same end
  devices and net class form a bus. A line with one IC between two connectors (population options) gives one lane
  per connector with the other side as an open branch.
- Lane channel: copper of each net from pin to pad (path between those points), series parts with estimated
  package parasitics (`data/components.json`: ESL, pad capacitance, capacitor ESR), terminations, open branches.
  Loss, return loss and TDR describe the interconnect without terminations; the eye uses a termination next to
  the receiver as the receiver's termination. Direction: receiver = end next to a termination, else the second
  end (swappable in the viewer).
- Eye only from IC to IC; lanes ending at a connector get the board share (loss, return loss, skew).
- CLI: `kisi board.kicad_pcb --buses`; MCP: `kisi_interfaces`, `kisi_channel_check` (lane based).

## IBIS models as channel ends (M7b) – experimental, not working yet
> **Status: not working yet.** The parser and the linearisation are in place, but neither the selection of the
> right IBIS model per pin nor the interplay with the signal standards (profiles) is validated. Results with IBIS
> ends must not be relied on; this milestone is parked and will be reworked.

- Source: KiCad's simulation fields of a symbol, copied to the footprint on “Update PCB”: `Sim.Device` = `IBIS`,
  `Sim.Library` (the .ibs file), `Sim.Name` (component in the file), optionally `Sim.Ibis.Model` (entry of a
  [Model Selector], e.g. the ODT variant). The pad number of a lane end is looked up in the component's [Pin] table.
- `crates/kisi-si/src/ibis.rs` reduces a pin to the linear end model of the eye (typical corner):
  transmitter = pull-up/pull-down I-V curves linearised at the [Ramp] load (Thevenin resistance and swing),
  edge from [Ramp] (20–80 % → 10–90 %); receiver = conductance of the clamp curves and [Rgnd]/[Rpower] at mid supply
  (on-die termination or high impedance); C_comp + C_pin/C_pkg, L_pin/L_pkg. Waveform tables, IBIS-AMI and package
  models are not evaluated – the result is marked as an estimate.
- Ends with a model replace the profile's transmitter/receiver; an on-die termination stays in parallel with a
  termination resistor at the receiver. IBIS model types (Output/Input) also set the lane direction.
- Files: the KiCad plugin and `kisi_open_viewer` embed the referenced files; in the viewer they can also be dropped
  or loaded with “Load .ibs files…” (Interfaces overview). CLI and MCP find them like KiCad (`${KIPRJMOD}`,
  relative to the board, else by file name below the board folder). `kisi file.ibs` lists the linear model of
  every signal pin; `kisi board.kicad_pcb --buses --ibis extra.ibs` adds files.

## Field solver
- Quasi-static 2D field solver: js_2d_fields by Henrik Forstén, included unmodified (see `UPSTREAM.md`).
- Every cross-section is written as a geometry text (`crates/kisi-si/src/xsgeom.rs`) and solved by the backend.
- Linear solver in Rust (`crates/kisi-sparse`, faer) replaces the unpublished Eigen/emscripten module.
- The viewer solves in the background on several threads, most important cross-sections first; formula values as start values.
- Validation: ≤ 1.5 % against HFSS/Ansys cases, ≤ 0.3 % against Forstén's microstrip/stripline solver on ThunderScope cross-sections.

## Board model
- Parser for KiCad 9/10 (stackup incl. sub-layers, tracks, arcs, vias, pads, zone fills, net classes).
- Cross-section detection along every line: reference planes (also across gaps/clearances), coplanar ground, diff partner.
- Analytic impedance: Hammerstad-Jensen, Wheeler, Wadell CPWG, IPC-2141 (diff).
- Target impedance from net class names (`50Z`, `100Z_Diff`, …) or interface assumptions, editable in the viewer.

## MCP server
`kisi-mcp` speaks MCP over stdio (tools `kisi_overview`, `kisi_interfaces`, `kisi_net`, `kisi_channel_check`, `kisi_profiles`,
`kisi_power`, `kisi_report`, `kisi_open_viewer`) and works next to KiCad's own MCP integrations and kicad-happy.
See `claude-plugin/README.md`.

## Building
```sh
git submodule update --init                                              # js_2d_fields
cargo build --release                                                    # CLI (target/release/kisi) and kisi-mcp
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
python3 tools/bundle.py web/index.html target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
        target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.html \
        ts "ThunderScope Rev5.3" <ts.kicad_pcb> <ts.kicad_pro> vna LibreVNA <vna.kicad_pcb> <vna.kicad_pro>
python3 tools/package_plugin.py                                          # dist/kisi-kicad-plugin.zip
```
Example boards are shrunk beforehand with `tools/strip_pcb.py`.

Field solver on the command line (needs Node.js): `tools/kisi_field.sh board.kicad_pcb`

Solver update: `tools/upstream_update.sh` (details in `UPSTREAM.md`); validation only: `tools/validate_field.sh`.

## Validation
`validation/*.json` holds cross-sections, `*.ref.json` the reference from Henrik Forstén's 2D field solver
(github.com/Ttl/js_2d_fields, GPL-3.0; run in Node with `tools/forsten_sparse_shim.js` replacing the WASM linear solver module).
```sh
target/release/kisi-validate validation/forsten_suite.json        # HFSS/Ansys references from Forstén's test suite
target/release/kisi-validate validation/thunderscope_xs.json validation/thunderscope_xs.ref.json
```
`forsten_selfcheck*.json`: Forstén's solver in this environment against the same HFSS cases (deviation ≤ 1.4 %),
which qualifies it as the reference for the ThunderScope cross-sections.

## Example boards
ThunderScope (EEVengers, MIT), LibreVNA (Jan Käberich, GPL-3.0).
