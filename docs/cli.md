# CLI reference

kisi has two command lines with the same commands, arguments and JSON results (both run
`kisi_core::commands`):

- **`kisi.mjs`** – one file for Node.js ≥ 18, attached to every
  [release](https://github.com/riehlies/kisi/releases/latest). It contains the viewer's engine and the 2D field
  solver.
- **`kisi`** – a native build (`cargo install --path crates/kisi-cli`) with formula impedances only (no field
  solver); useful for CI and development.

```sh
curl -fsSLO https://github.com/riehlies/kisi/releases/latest/download/kisi.mjs
node kisi.mjs help
```

`node kisi.mjs <command> <board.kicad_pcb> [options]` prints JSON on stdout. Every result states the models and
assumptions it used; progress (field solver, check rows) goes to stderr. An error prints `{"error": "…"}` and
exits with code 1. The board's `.kicad_pro` and `<board>.kisi.json` next to it are read automatically.

## `kisi help`
The text below is the output of `kisi help`. A test (`cargo test -p kisi-core`) fails when it differs from the
program; `KISI_BLESS_DOCS=1 cargo test -p kisi-core docs_cli` writes the current text here.

<!-- BEGIN kisi help (generated from kisi_core::commands::HELP) -->
```text
kisi <command> <board.kicad_pcb> [options] – JSON on stdout

  overview         stackup, impedance per net class (share within ±5 %, worst nets)
  interfaces       buses and device-to-device lanes with profiles   [--class CLASS=PROFILE]…
  net <NET>        one net: impedance along the path, loss, TDR, vias, crosstalk;
                   differential pairs also as 4-port (mixed mode, skew)
                   [--rise-ps 35] [--roughness smooth|hvlp|standard] [--s2p FILE | --s4p FILE (Touchstone)]
  check            channel check of all lanes against interface profiles
                   [--class CLASS=PROFILE]… [--nets A,B] [--all] [--max 30]
  crosstalk        coupling scan (NEXT/FEXT from neighbouring tracks) of all nets with a target,
                   critical from 1 %; kisi.mjs refines the critical couplings with the field solver
                   [--nets A,B] [--all (all rows, not only critical)] [--max 30] [--rise-ps 35]
  profiles [ID]    interface profiles (no board needed)
  power            regulator chain, DC IR drop per rail and ground return, losses, power flow
                                                          [--tree tree.json] [--pdn RAIL]
  thermal          temperature estimate from the power analysis: board temperature per layer, hot
                   spots, junction temperature per part (PI_THERMAL or package defaults), copper
                   temperature and IR drop at temperature per rail
                   [--tree tree.json] [--ambient °C (25)] [--h W/m²K per side] [--airflow m/s] [--vertical]
  report           write analysis/kisi.json next to the board   [--out FILE]
  gaps             what the analyses miss, where to find it, how to write it into <board>.kisi.json
  config-check     check <board>.kisi.json (format, ranges, references, sources, variants)
  config-template  a <board>.kisi.json skeleton with this board's dielectrics, net classes and
                   assembly variants

common options: --pro FILE (default: board.kicad_pro), --config FILE (default: board.kisi.json
next to the board), --no-config, --stackup FILE (stackup changes, as in the viewer),
--variant NAME (assembly variant: a KiCad 10 variant of the board or one of the kisi.json
"variants"; "default" = the board file as it is). Boards with variants – or with forks where
all alternative parts are fitted – need one for interfaces, check and report (else they stop and
list the variants); "variant" in <board>.kisi.json chooses one for every run.
field solver (kisi.mjs only; overview, net, check, crosstalk, report): used by default for the
cross-sections of nets with a target, results cached on disk; --formula (no field solver),
--field-time S (time budget for new solves, default 120), --field-all (all cross-sections, not
only targets), --field-xt (also the coupling cross-sections of critical crosstalk victims, NEXT or FEXT from 1 %;
net: always all of the net's)

kisi – a side project by David Riehl, started as a visualisation tool for the course Advanced PCB Design
at TU Darmstadt. GPL-3.0-only, https://github.com/riehlies/kisi
A quick design check: results are estimates, without any warranty of correctness.
```
<!-- END kisi help -->

## Commands

### `overview`
Stackup (with `assumed` for values that are KiCad defaults) and, per net class with a target impedance, the share
of routed length within ±5 % and the worst nets. `config` says which kisi.json was used. Runs the field solver.

### `interfaces`
Buses and device-to-device lanes: pins, series parts, terminations, branches, side stubs (`stubs`: unused pads),
the guessed transmit direction and the interface profile of each class (`--class CLASS=PROFILE` to try another).
Needs an assembly variant on boards that have them (see below).

### `net`
One net (`kisi net <board> <NET>`): its path (start, end, branches), impedance segments along it (layer, length,
Z), loss, TDR extremes, vias (antipad, stub resonance, return path) and crosstalk from its neighbours;
differential pairs also as a 4-port (mixed mode, skew). `--rise-ps` sets the edge (default 35 ps),
`--roughness smooth|hvlp|standard` the copper roughness preset; `--s2p FILE` writes the channel's S-parameters,
`--s4p FILE` the pair's single-ended 4-port as Touchstone. Runs the field solver for the net's cross-sections.

### `check`
Channel check of all lanes against their interface profiles: loss, return loss, skew, via stubs and the eye;
nets with a target but no devices on their own. Only yellow and red rows are printed (`--all` for green too,
`--nets A,B` to restrict, `--max N` rows, default 30). `--class CLASS=PROFILE` overrides a profile (better: put it
into kisi.json). Rows are computed on several threads (below); the result is the same as on one thread. Needs an
assembly variant on boards that have them. Runs the field solver.

### `crosstalk`
Coupling scan of all nets with a target: NEXT and FEXT from the neighbouring tracks, critical from 1 % of the
aggressor swing, with the worst aggressor per victim (`--nets A,B`, `--all` for every row, `--max`, `--rise-ps`).
`kisi.mjs` refines the critical couplings with the field solver; `source: formula` values are estimates.

### `power`
Regulator chain and budget (from `PI_REG`/`PI_LOAD` fields, kisi.json or `--tree FILE`), DC IR drop per rail and
in the ground return (`ground_return`: `max_offset_mv` adds to the drop of the parts' rails), `losses` (heat per
part, copper loss per rail and layer) and `flow` (power from the inputs through the regulators to the loads).
`--pdn RAIL` adds the impedance Z(f) of one rail against its target. Regulators flagged `efficiency_estimate` use
kisi's default of 85 %.

### `thermal`
Steady-state temperature estimate from the losses of `power`: board temperature per layer and hot spots; per part
(`parts[]`) the junction temperature against Tj max and the case top, with `data_source` (PI_THERMAL or package
defaults); heat through the vias and under each hot part, with `parts_hints` where thermal vias look short; per
rail the copper temperature and the IR drop at that temperature. `--ambient` (°C, default 25), `--airflow` (m/s),
`--vertical` (a vertical board), `--h` (surface coefficient per side in W/m²K, replaces the convection
correlations, e.g. for the inside of a closed case). See [Models and limits](models.md#temperature-estimate).

### `report`
Everything into `analysis/kisi.json` next to the board (`--out FILE` elsewhere); prints a summary. For file-based
review tools and later sessions. Needs an assembly variant on boards that have them. Runs the field solver.

### `gaps`
Missing or default data: each gap has `id`, `priority` (high, medium, low), `area`, `issue`, `affects`,
`where_to_find`, `write` (where it goes in `<board>.kisi.json`, with an example) and `items` (layers, classes,
parts with their MPN). See [Preparing a board](preparing-a-board.md#the-gap-list). Fast, no field solver.

### `config-check`
Checks `<board>.kisi.json` against the board: format, ranges, references, sources, field syntax kisi cannot read,
variants and the files they name. Prints `ok`, `errors`, `warnings` and the number of entries.

### `config-template`
A `<board>.kisi.json` skeleton with this board's dielectric indices, net classes and suggested assembly variants
(from forks where all alternative parts are fitted). Keep only the entries you fill in.

### `profiles`
The interface profiles (`kisi profiles`), or one in full (`kisi profiles pcie3`). No board needed.

### `help`
The text above. `kisi.mjs` also accepts `--version`.

## Options
| Option | Commands | Meaning |
|---|---|---|
| `--pro FILE` | all with a board | the project file (default: `<board>.kicad_pro`) |
| `--config FILE` | all with a board | the board configuration (default: `<board>.kisi.json` next to the board) |
| `--no-config` | all with a board | ignore `<board>.kisi.json` |
| `--stackup FILE` | all with a board | stackup changes as the viewer stores them ([format](reference.md#other-input-files)) |
| `--variant NAME` | all with a board | assembly variant: a KiCad 10 variant, one of the kisi.json `variants`, or `default` |
| `--class CLASS=PROFILE` | `interfaces`, `check` | interface profile of a net class (repeatable) |
| `--nets A,B` | `check`, `crosstalk` | only these nets |
| `--all` | `check`, `crosstalk` | all rows, not only the yellow/red or critical ones |
| `--max N` | `check`, `crosstalk` | at most N rows (default 30) |
| `--rise-ps PS` | `net`, `check`, `crosstalk` | edge rise time (default 35 ps) |
| `--roughness R` | `net`, `check`, `crosstalk` | copper roughness preset: `smooth`, `hvlp`, `standard` |
| `--s2p FILE` | `net` | write the channel's S-parameters (Touchstone) |
| `--s4p FILE` | `net` | write a pair's single-ended 4-port (Touchstone) |
| `--tree FILE` | `power`, `thermal` | power tree as JSON instead of the fields ([format](reference.md#other-input-files)) |
| `--pdn RAIL` | `power` | add Z(f) of this rail |
| `--ambient °C` | `thermal` | ambient temperature (default 25) |
| `--airflow m/s` | `thermal` | forced air flow over the board |
| `--vertical` | `thermal` | the board stands vertically (natural convection) |
| `--h W/m²K` | `thermal` | surface coefficient per side instead of the convection correlations |
| `--out FILE` | `report` | where to write the report (default `analysis/kisi.json` next to the board) |
| `--formula` | field-solver commands | no field solver (formula impedances) |
| `--field-time S` | field-solver commands | time budget for new solves, default 120 s |
| `--field-all` | field-solver commands | all cross-sections, not only those of nets with a target |
| `--field-xt` | field-solver commands | also the coupling cross-sections of critical crosstalk victims |

## Assembly variants
`--variant NAME` (over `"variant"` in `<board>.kisi.json`) chooses which parts are fitted. When a board has
variants – or forks where all alternative series parts are fitted – and none is chosen, `interfaces`, `check` and
`report` stop with the list of variants (exit code 1) and `gaps` reports it; `power` and `thermal` warn and count
the parts as in the board file; the other commands do not depend on the fitted parts. `variant` in the output
says which one was analysed. How variants are defined: [reference](reference.md#variants).

## Field solver
`overview`, `net`, `check`, `crosstalk` and `report` run the 2D field solver (`kisi.mjs` only) for the
cross-sections of nets with a target, the most routed length first, in worker threads, and stop starting new
solves after `--field-time` seconds (default 120). The first run on a board can take minutes; results are cached
in `~/.cache/kisi/field-cache.json` (`$XDG_CACHE_HOME/kisi`, Windows `%LOCALAPPDATA%\kisi`, or `$KISI_CACHE`), so
later runs are fast and the coverage grows. `field_solver` in the output shows the coverage
(`target_length_coverage_pct`); formula values fill the rest. `--formula` skips the solver; the native `kisi`
always uses formulas and says so in `field_solver_note`.

## Threads
`check` (and the check in `report` of the native build) computes its rows on up to 4 threads, fewer on machines
with fewer cores; `KISI_THREADS=n` sets the number (1: no extra threads). The viewer's check uses Web Workers the
same way. The result is identical to one thread.

## Exit codes
`0` success; `1` an error (unknown command or option, unreadable file, a lane command without a chosen variant),
with `{"error": …}` on stdout; the native `kisi` without arguments prints the help to stderr and exits with `2`.
