# CST 2D port-mode check of kisi's impedance chain

kisi's line impedances come from two paths: closed-form formulas (`kisi_si::analytic`, chosen per cross-section by
`kisi_si::extract::formula_xs`) and the quasi-static 2D field solver of H. Forstén (`third_party/js_2d_fields`),
which gets kisi's own cross-section geometry (`crates/kisi-si/src/xsgeom.rs`, custom-geometry text; UPSTREAM.md).
Until now the field path was checked against 10 HFSS/Ansys cases of Forstén's suite and 13 cross-sections against
Forstén's own MS/SL solver (`tools/validate_field.sh`). This directory checks both paths broadly against a solver
of a different kind: **CST Studio Suite's 2D port-mode (eigenmode) solver**, on 620 cross-sections – sweeps of every
line type kisi models, the anchors above and the real cross-sections of the example boards and the test-board
coupons, each built in CST from exactly the geometry text kisi solves.

Status: **prepared, CST not run yet** (branch `feat/cst-xs`). kisi's side is in `kisi.json`; the CST results
go to `results/`.

| File | What |
|---|---|
| `make_cases.py` | generates `cases.json` (needs `target/release/kisi-validate`; boards: see below) |
| `cases.json` | the 620 cases: spec (stackup + cross-section description), kisi's geometry `text`, solver box |
| `xsgeo.py` | geometry text parser, port size rule, rectangle decomposition, wall variants (stdlib only) |
| `cst_xs.py` | the CST runner (CST's Python, stdlib only) → `results/<id>.json` |
| `run_kisi.py` | kisi's formula and field results on all cases → `kisi.json` |
| `compare.py` | CST against kisi field / formula (and HFSS where given), per group; works with any subset of results |

## Cases

`make_cases.py` builds each case as a stackup and a cross-section description – the inputs of `xsgeom::build`
and of the formula path – and `kisi-validate xs-cases` (crates/kisi-cli/src/xscases.rs) turns them into kisi's
geometry text and formula values, so CST builds exactly what kisi solves. Board cases come from `kisi-validate
xs-board` (the geometries kisi extracts on a board, with the description they were built from; the text is
reproduced from it before a case is kept). Units mm; `w/h` = width over the height to the reference.

| Group | n | tier 1 / 2 / 3 | Sweep |
|---|---:|---|---|
| `anchor` | 23 | 23 / 0 / 0 | Forstén's suite (10, HFSS/Ansys refs; the covered trace as kisi models it), ThunderScope (6) and asymmetric striplines (7) with refs of Forstén's MS/SL solver |
| `microstrip` | 66 | 2 / 40 / 24 | w/h 0.1…10 × εr 2.2, 3.0, 3.66, 4.4, 6.15, 10.2 (h 0.3, t 35 µm) |
| `microstrip_t` | 20 | 0 / 8 / 12 | t 17.5, 35, 70, 105 µm × w/h 0.25…4 |
| `microstrip_h` | 15 | 1 / 4 / 10 | h 0.07…1.6 mm × w/h 0.5, 1.9, 4 |
| `microstrip_mask` | 26 | 1 / 11 / 14 | solder mask 10, 20, 35 µm × εr 3.3, 4.0 × w/h 0.5…4; thick copper under mask |
| `embedded` | 24 | 1 / 11 / 12 | covered trace (inner layer, one plane): cover 0.05…0.5 mm × w/h × εr |
| `stripline` | 29 | 1 / 12 / 16 | symmetric: w/h 0.25…4 × h 0.1, 0.2 × εr 3.0, 4.2; t 17.5…70 µm |
| `stripline_asym` | 20 | 1 / 15 / 4 | offset h2/h1 1.5…8; different εr above / below (core / prepreg) |
| `stripline_dual` | 6 | 0 / 3 / 3 | trace between two planes with an empty signal layer in between |
| `diff_microstrip` | 34 | 2 / 19 / 13 | edge-coupled, s/h 0.25…4 × w/h 0.5…2, mask on/off, εr |
| `diff_stripline` | 20 | 1 / 19 / 0 | edge-coupled, s/h 0.25…4 × w/h × symmetric / offset |
| `broadside` | 8 | 0 / 4 / 4 | broadside-coupled stripline (no kisi model: solver check only, hand-written text) |
| `cpwg` | 30 | 1 / 23 / 6 | grounded coplanar gap/h 0.25…4 × w/h × mask |
| `diff_cpwg` | 6 | 0 / 2 / 4 | pair with coplanar ground |
| `stripline_cpw` | 3 | 0 / 3 / 0 | stripline with coplanar ground (the formula ignores it) |
| `cpw_noplane` | 8 | 0 / 4 / 4 | coplanar without plane: kisi's no-plane formula (an estimate) against the solvers; hand-written text (kisi has no field geometry for it) |
| `plane_cut` | 16 | 1 / 8 / 7 | trace over an opening in the next layer (1…8 × w), reference the layer below |
| `plane_edge` | 12 | 1 / 9 / 2 | reference plane ending 0.5…8 h beside the trace (single, pair) |
| `etch` | 21 | 0 / 7 / 14 | trapezoidal etch, factor 1, 2, 3: CST builds trapezoids (`cst_text`), kisi its rectangle – the error of kisi's rectangular model |
| `size_study` | 16 | 4 / 12 / 0 | 8 cases again with the solver box ×1.5 and ×2 (port size rule) |
| `board_target` | 120 | 0 / 60 / 60 | ThunderScope and LibreVNA: the cross-sections of nets with a target impedance, by routed length (30 each in tier 2: 81 % / 64 % of the target length) |
| `board_other` | 16 | 0 / 0 / 16 | the longest 8 other cross-sections per example board |
| `test_board` | 81 | 0 / 71 / 10 | all distinct cross-sections of the test-board coupons (branch `feat/test-board`: JLC 4/6/8 layers, 7628 and 3313, Aisler; MS, CPWG, stripline, pairs, launches over cut-outs) |

Tiers: **1** = anchors and one or two cases per group (40 cases, run first), **2** = main sweeps and the
target-impedance board cross-sections, **3** = the rest. 23 cases are marked `dispersion` for the lossy run.

Regenerating (needs the boards; without them the existing board cases are kept):

```sh
cargo build --release -p kisi-cli
sh tools/fetch_examples.sh examples
mkdir -p /tmp/tb && git archive origin/feat/test-board validation/test_board/boards | tar -x -C /tmp/tb
python3 validation/cst_xs/make_cases.py --test-boards /tmp/tb/validation/test_board/boards
```

## The CST model (`cst_xs.py`)

* **Geometry.** The case's text (`cst_text` for the etch group) is painted in js_2d_fields' order (later
  dielectrics over earlier, conductors over dielectrics) on the grid of all rectangle edges and merged into
  non-overlapping rectangles (`xsgeo.tiles`), extruded along z by 1/40 of the box size (the port modes do not
  depend on it; few cells along z): dielectric bricks (air = εr 1), PEC bricks for conductors (each conductor its
  own solid), trapezoids as extruded polygons cut into the bricks. Lossless: tanδ 0, PEC.
* **Box (port size rule, `xsgeo.domain`).** H = the largest distance from a signal conductor to a ground face
  above or below it that overlaps it (the farther plane of a stripline), Ws = width of the signal group. The box
  extends max(12 H, 4 Ws, 0.5 mm) beyond the outermost finite edge laterally and beyond the outermost face on an
  open top or bottom; a side whose outermost object is a full-width plane closes on that plane. `size_study`
  checks the rule (×1.5, ×2).
* **Walls.** Each case is solved twice: with **electric** side walls, and with **magnetic** side walls (where the
  box closes on a plane that side stays electric). Both bias the impedance of an open structure, in opposite
  directions and by about the same amount (images of opposite sign); their **mean is the open-boundary estimate**
  `open_est` (js_2d_fields on six cases: the mean changes by < 0.1 % between ×1 and ×2 margins, each wall kind
  alone by up to 1.7 % at ×1 and 1.1 % at ×2). kisi's solver holds every ground at 0 V. Electric walls join the grounds that reach a
  side wall; with magnetic walls a thin ground strap at each side wall joins the grounds touching it
  (`xsgeo.straps`), and a ground that touches nothing (two ThunderScope pairs whose reference is a narrow strip
  on the far layer) is tied to the nearer top/bottom wall (`xsgeo.ties`; such cases get electric walls only).
  `run_kisi.py` solves the very same boxes with js_2d_fields (`field_E`: `bounds gnd`, `field_M`: `bounds open`
  = natural boundary, with the same straps and ties), so CST and kisi's solver are compared on identical
  problems as well.
* **Port and impedance definition.** One waveguide port on the whole zmin face (`Coordinates "Full"`), multipin
  with potentials (`AddPotentialNumerically`, points inside the conductors):
  * single line: mode 1 = signal positive, ground negative → **Z0** = CST's line impedance, εeff = (β c/ω)²;
  * pair: mode 1 = sig+ positive, sig− negative (differential) → line impedance **Zdiff = 2 Zodd**;
    mode 2 = both signals positive, ground negative (common) → **Zcomm = Zeven / 2**. So **Zodd = Zdiff / 2,
    Zeven = 2 Zcomm**; εeff,odd / εeff,even from the β of the two modes. (With the voltage between the positive
    and negative pins and the power or the current of the positive pins, both definitions give exactly these
    relations for a symmetric pair.) The larger of the two impedances is taken as the differential mode
    (`meaning_by_value` in the result) in case CST orders the modes differently. If the port with potentials
    cannot be built, the runner falls back to plain eigenmodes and records `potentials: false`.
* **Solver.** Time-domain solver with `CalculateModesOnly` (no 3D solve), port modes at f0 = 1 GHz (frequency
  range 0.98…1.02 GHz; js_2d_fields is quasi-static, kisi evaluates its RLGC at 1 GHz; dispersion at 1 GHz is
  estimated at ≤ 0.3 % for the 1.6 mm substrates and far less for thin ones; `--freq` sets f0).
* **Mesh convergence.** Hexahedral PBA mesh, level k: lines per box 20·2^k, edge refinement 4+2k, ratio limit
  20·2^k. Levels 0, 1, 2… until Z and εeff of every mode change by less than **0.1 %** from the previous level
  (`--tol`), at most 5 levels or 3 M cells (`not_converged` otherwise). CST's adaptive port meshing
  (`AdaptivePortMeshing`, 6 passes) is switched on in addition where CST accepts it (the block is optional).
  The magnetic-wall solve runs on the converged level of the electric one. Where the changes of the last three
  levels shrink geometrically, the extrapolated value is recorded too (`values_extrap`, not used as the result).
* **Dispersion (`--lossy`, separate run).** On the converged level: dielectric tanδ of the geometry (constant
  tanδ; typical values per εr from `make_cases.py`, solder mask 0.025), copper σ = 5.8e7 S/m, port modes at
  0.1, 1, 5, 10, 20, 40 GHz, electric walls → `results/<id>.lossy.json` (complex line impedance, β, εeff per
  frequency). Only the cases marked `dispersion` unless `--all-cases`. At 40 GHz a large box carries box modes;
  the potentials pick the quasi-TEM modes.
* **Output** `results/<id>.json`: status (`ok`, `not_converged`, `no_values`, `failed`), CST version (from the
  solver log), date, f0, box, per wall kind every level (mesh settings, cells, port-mesh size where the log
  states it, seconds, per mode line impedance, β, εeff, accuracy, the line impedance printed in the log), the
  converged values, `result` = {electric, magnetic, open_est, open_spread_pct}, total run time, failed history
  blocks, the port-result tree items CST offered, and `text_hash` (a result whose geometry has changed since is
  skipped as stale). The solver logs go to `logs/` (not committed).

## Running it (CST session)

Requirements: the Python with CST's `cst` package, as for `validation/fdtd/run_cst.py` (CST's
`python_cst_libraries` on `PYTHONPATH`, or `--cst-lib <dir>`). Everything runs from `validation/cst_xs`. Each
case takes about 1 minute (two projects, 3–4 mesh levels each; CST start-up per project dominates): tier 1
(40) ≈ 40 min, tier 2 (345) ≈ 6 h, tier 3 (235) ≈ 4 h, the lossy run (23 cases × 6 frequencies) ≈ 30 min.

Shell calls of the session time out after 45 s, so every long run is started with `--detach` (it restarts itself
in the background, writes `logs/run.pid`, `logs/run.out` and appends to `logs/run.log`) and watched with
`--status` (same selection; done / failed / left, mean time, ETA, last log lines). `--skip-done` resumes after an
interruption (cases with a final result – `ok` or `not_converged` – for the same geometry are skipped; `failed`
ones run again).

```sh
cd validation/cst_xs
python cst_xs.py --probe                                          # CST found? writes logs/cst_api.txt
python cst_xs.py --dry-run --ids ms_u2_er4.4                      # no CST: work/<id>_electric.bas to inspect
# 1. pilot: three cases, one at a time, look at the results before going on
python cst_xs.py --ids ms_u2_er4.4 --keep-projects --detach
python cst_xs.py --ids ms_u2_er4.4 --status
python cst_xs.py --ids dms_u1_s1,cpwg_u1_g0.5 --keep-projects --detach
# 2. tier 1, then 2, then 3
python cst_xs.py --tier 1 --skip-done --detach
python cst_xs.py --tier 2 --skip-done --detach
python cst_xs.py --tier 3 --skip-done --detach
# 3. dispersion (after the lossless run of those cases: it reuses their converged mesh level)
python cst_xs.py --lossy --skip-done --detach
```

Pilot checklist (open `work/ms_u2_er4.4/*.cst` with `--keep-projects`):
1. the port covers the whole zmin face and shows the potential pins on the signal(s) and the ground plane
   (if `AddPotentialNumerically` reads (u, v) not as (x, y) on a zmin port, fix the coordinates in
   `potential_sets`);
2. `results/ms_u2_er4.4.json` has values at every level (`z`, `eeff`) and `blocks_failed` is empty or holds only
   the optional adaptive-port-meshing block; if the values are missing, the result-tree names differ: the
   `cst_raw_last.items` list in the result shows what CST offers, adjust `read_port_results`;
3. Z0 within about 1 % of `field_E` of that case in `kisi.json`: `python compare.py --ids ms_u2_er4.4 --quantity
   all` (plain Python, reads `kisi.json` and `results/`; "solver E" is that comparison);
4. the pair: mode 1 ≈ 2× Zodd of kisi, mode 2 ≈ Zeven/2.
Fixes of the runner go into `cst_xs.py` / `xsgeo.py` on this branch, with the reason in the commit message.

**Handing results back:** commit `validation/cst_xs/results/*.json` on `feat/cst-xs` (results only, plus runner
fixes if any; not `work/` or `logs/`), in batches (e.g. after the pilot, after each tier). The CST version comes
from the solver log and is in every result.

## kisi's side and the comparison

```sh
cargo build --release && cargo build --release -p kisi-sparse --target wasm32-unknown-unknown
python3 validation/cst_xs/run_kisi.py            # → kisi.json (≈ 25 min on 2 cores; --ids / --tier for a subset)
python3 validation/cst_xs/compare.py             # all groups, any subset of CST results
python3 validation/cst_xs/compare.py --tier 1 --quantity all --outlier 1 --show-text
```

`kisi.json` holds per case `formula` (the extraction's closed-form values), `field` (js_2d_fields as kisi runs it:
tol 0.01, open boundaries on its own domain), `field_fine` (tol 0.002), `field_E` / `field_M` (tol 0.002 in
CST's box with electric / natural walls). `compare.py` reports per group n, bias (median signed error), median,
90th percentile and max of |error| for: **solver E / M** (field_E / field_M against CST with the same walls: the
geometry chain and the solver numerics alone), **field vs CST** and **formula vs CST** (what kisi reports, against
CST's open-boundary estimate), **CST vs ref** and **field vs ref** (anchors), and lists the outliers with their
geometry. Without CST results it compares kisi with itself: formula and production field against the solver's
open-boundary estimate (mean of field_E and field_M).

What changes once the results are in and reviewed (not before): `data/accuracy.json` (`field_pct`, today 2.0 from
the HFSS cases; the formula uncertainties per line type) and the validation section of `docs/models.md`;
`validation/field_baseline.json` stays the regression baseline of `tools/validate_field.sh`.
