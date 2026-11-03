# Cross-section validation of kisi's impedance and loss chain (CST, full-wave, FDM, formula)

kisi's line impedances come from two paths: closed-form formulas (`kisi_si::analytic`, chosen per cross-section by
`kisi_si::extract::formula_xs`) and the quasi-static 2D field solver of H. Forstén (`third_party/js_2d_fields`,
FDM backend), which gets kisi's own cross-section geometry (`crates/kisi-si/src/xsgeom.rs`, custom-geometry text;
UPSTREAM.md); kisi's line model (`crates/kisi-si/src/rf/line.rs`) takes R, L, G, C from the field solve at 1 GHz.
This directory checks both paths on 620 cross-sections – sweeps of every line type kisi models, the anchors of
`tools/validate_field.sh`, the real cross-sections of the example boards and the test-board coupons – against two
solvers of other kinds, each solving exactly the geometry text kisi solves: **CST Studio Suite's 2D port-mode
solver** (lossless at 1 GHz, and lossy port modes from 0.1 to 40 GHz) and the **full-wave triangular FEM backend**
of js_2d_fields (UPSTREAM.md, "Full-wave backend"; validation only, not in kisi's bundles). Results: section
[Results](#results); the page `kisi-references-2d.html` shows every case (section [The page](#the-page)).

| File | What |
|---|---|
| `make_cases.py` | generates `cases.json` (needs `target/release/kisi-validate`; boards: see below) |
| `cases.json` | the 620 cases: spec (stackup + cross-section description), kisi's geometry `text`, solver box |
| `xsgeo.py` | geometry text parser, port size rule, rectangle decomposition, wall variants (stdlib only) |
| `cst_xs.py` | the CST runner (CST's Python, stdlib only) → `results/<id>.json`, `results/<id>.fd.lossy.json` |
| `results/` | CST: lossless port modes (`<id>.json`, 620, electric walls; magnetic walls for 26), lossy port modes of the frequency domain solver (`<id>.fd.lossy.json`, 620), the earlier time-domain lossy runs (`<id>.lossy.json`, 23: Z and εeff, no α) |
| `run_kisi.py` | kisi's formula and field results on all cases → `kisi.json` |
| `run_fullwave.py`, `fullwave_driver.mjs`, `fullwave_plan.json` | the full-wave backend on all cases (GitHub Actions, workflow `fullwave-xs`) → `fullwave/<id>.json` |
| `compare.py` | **the comparison**: CST, FW, FDM, formula per group, lossless and the loss sweep; `--solve-loss` → `kisi_loss.json` |
| `kisi_loss.json` | kisi's FDM solve (R, L, G, C at 1 GHz) of every case in CST's box: the input of kisi's line model in the loss comparison |
| `fullwave_compare.py`, `compare_loss.py` | thin wrappers of `compare.py` (the workflow's summary; the loss section) |
| `mqs_check.py`, `mqs_check.json` | an independent finite-difference eddy-current (MQS) solution of the conductor loss on 9 cases |
| `uncertainty.py` | the accuracy of the CST results by source |
| `make_viewer.py`, `viewer_template.html` | the page `dist/kisi-references-2d.html` |
| `probes/` | small CST probes (frequency-domain pilot, the PCIe edge fingers) |

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
  tanδ, CST's wideband `ConstTanD` model; εr and tanδ hold at `--tand-freq`, default 1 GHz as in kisi's
  Djordjevic-Sarkar dielectric; typical values per εr from `make_cases.py`, solder mask 0.025), copper
  σ = 5.8e7 S/m, port modes at 0.1, 1, 5, 10, 20, 40 GHz, electric walls → `results/<id>.lossy.json` (line
  impedance as CST gives it, β, εeff and the attenuation α = Re γ, also in dB/m, per frequency; the port modes
  proper, CST's "Port Modes Base" only where those lack a quantity). Results written before `--tand-freq`
  (εr anchored at the first frequency, no α) do not count as done for `--skip-done`.
  The time domain solver computes lossless port modes (lossy metal as PEC, only the dispersion of ε′ enters;
  γ has no real part): its lossy runs give Z(f) and εeff(f) but no attenuation. `--lossy --fd --tag fd` runs
  the frequency domain solver instead (tetrahedral mesh, `FDSolver.ModesOnly`, `ConsiderPortLossesTet`, the
  same constant-tanδ fit via `TDCompatibleMaterials`; the port mode solver refines its own port mesh) →
  `results/<id>.fd.lossy.json` with α. Check its εeff and Z against the time domain runs first.
  `--sweep N`: N points per decade from `--sweep-min` to `--sweep-max` (default 0.01–40 GHz, whole decades,
  plus 5, 20 and 40 GHz) instead of `--freqs`, each frequency a project of its own as before: with
  `ModesOnly` the frequency domain solver computes the port modes only at the centre of the frequency range
  (one run with 37 single samples gave one result, at 20.4 GHz). With `--fd` about 10 s per frequency. Only the
  cases marked `dispersion` unless `--all-cases` (the stored `fd.lossy` results: all 620 cases, 0.1, 1, 5, 10, 20,
  40 GHz). Above the box's first waveguide cutoff the electric walls carry box modes, which the potentials do not
  always separate from the quasi-TEM mode (section [Results](#results), box modes).
  Options of the conductor-loss check (defaults = the stored runs): `--metal volume` (copper as a normal material
  with σ, its interior meshed, instead of CST's surface-impedance "Lossy metal"), `--tet-scale K` (tetrahedral mesh
  density ×K), `--no-td-compat` (`TDCompatibleMaterials` off); they are recorded as `loss_options` in the result.
* **Output** `results/<id>.json`: status (`ok`, `not_converged`, `no_values`, `failed`), CST version (from the
  solver log), date, f0, box, per wall kind every level (mesh settings, cells, port-mesh size where the log
  states it, seconds, per mode line impedance, β, εeff, accuracy, the line impedance printed in the log), the
  converged values, `result` = {electric, magnetic, open_est, open_spread_pct}, total run time, failed history
  blocks, the port-result tree items CST offered, and `text_hash` (a result whose geometry has changed since is
  skipped as stale). The solver logs go to `logs/` (not committed).

## Running it in CST

Requirements: the Python with CST's `cst` package, as for `validation/fdtd/run_cst.py` (CST's
`python_cst_libraries` on `PYTHONPATH`, or `--cst-lib <dir>`). Everything runs from `validation/cst_xs`. Each
case takes about 1 minute (two projects, 3–4 mesh levels each; CST start-up per project dominates): tier 1
(40) ≈ 40 min, tier 2 (345) ≈ 6 h, tier 3 (235) ≈ 4 h, the lossy run (23 cases × 6 frequencies) ≈ 30 min.

Long runs are started with `--detach` (the runner restarts itself in the background, writes `logs/run.pid`, `logs/run.out` and appends to `logs/run.log`) and watched with
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
3. Z0 within about 1 % of `field_E` of that case in `kisi.json`: `python compare.py --ids ms_u2_er4.4 --no-loss`
   (plain Python, reads `kisi.json` and `results/`; "FDM vs CST, E walls" is that comparison);
4. the pair: mode 1 ≈ 2× Zodd of kisi, mode 2 ≈ Zeven/2.

Results: `validation/cst_xs/results/*.json` are committed (not `work/` or `logs/`); runner fixes go into
`cst_xs.py` / `xsgeo.py` with the reason in the commit message. The CST version comes from the solver log and is in
every result.

## kisi's side, the full-wave backend and the comparison

```sh
cargo build --release && cargo build --release -p kisi-sparse --target wasm32-unknown-unknown
python3 validation/cst_xs/run_kisi.py              # → kisi.json (≈ 25 min on 2 cores; --ids / --tier for a subset)
python3 validation/cst_xs/compare.py --solve-loss  # → kisi_loss.json (kisi's line-model solves, ≈ 2 h on one core)
python3 validation/cst_xs/compare.py               # every comparison, all groups (--md / --json to save it)
python3 validation/cst_xs/compare.py --groups diff_stripline --groups-full --loss-table --show-text
python3 validation/cst_xs/mqs_check.py             # the independent eddy-current check (numpy, scipy)
python3 validation/cst_xs/uncertainty.py
```

`kisi.json` holds per case `formula` (the extraction's closed-form values), `field` (js_2d_fields as kisi runs it:
tol 0.01, open boundaries on its own domain), `field_fine` (tol 0.002), `field_E` / `field_M` (tol 0.002 in CST's box
with electric / natural walls).

**The full-wave backend** (`src/tri_solver/` of js_2d_fields, unmodified; UPSTREAM.md, "Full-wave backend") runs on
GitHub Actions (workflow `fullwave-xs`, manual only; the cases in `fullwave_plan.json`, results as artifacts:
`gh run download <run> -p 'fullwave-*' -D /tmp/fw && cp /tmp/fw/*/*.json validation/cst_xs/fullwave/`); its WASM
modules are built by `tools/build_forsten_fullwave.sh`.

| Kind | Problem | What it gives |
|---|---|---|
| `E` | CST's box, electric walls (`boxed_text "gnd"`, as `field_E`), tanδ 0, 1 GHz | static P2 FEM (`z0_static`, `eeff_static`) and full-wave eigenmode (`z0`, `eeff_mode`) on one mesh; Zodd/Zeven and the 2×2 L, C of pairs |
| `M` | the same with natural walls (`"open"` with straps = CST's magnetic walls, as `field_M`); not where a ground touches nothing | as E |
| `sweep` | electric walls, tanδ of the geometry with Djordjevic–Sarkar dispersion (εr and tanδ at 1 GHz, as CST's `TanDFreq`), copper 5.8e7 S/m smooth, MQS conductor loss, CST's sweep grid (`cst_xs.sweep_freqs(10, 0.01, 40)`, 37 frequencies) | per mode Z0, complex Zc, phase εeff, εeff of the eigenmode, R, L, G, C, internal L, αc, αd (dB/m); Zdiff, Zcomm and the 2×2 R/L/G/C of pairs |

The geometry is the one CST solved (`cst_text` where a case has one: the etch group's trapezoids, which the backend
meshes as they are). Both wall kinds are the backend's own (`gnd` = PEC, `open` = natural boundary). Below 100 MHz
the backend reports the static solution; above, the eigenmode εeff is scaled by its ratio to the static value at
100 MHz (upstream's `_eigenBias`). Mesh: adaptive to tol 0.002 on the static energies with upstream's error
certificate, triangle budget capped by upstream's eigensolve memory guard (about 24k triangles). The run of the 620
cases: 8 shards on 4-vCPU runners, 19 min wall time, 1858 tasks, none failed; per task on one vCPU median 6 s for E
and M, 13 s for a sweep, at most 6 min.

**`compare.py`** reports per group n, bias (median signed error), median, 90th percentile and max of |error|
against CST, for every comparison (its docstring lists them): the solver checks in CST's box (FW and FDM against
CST with the same walls), kisi's results as kisi reports them against CST's open-boundary estimate (the mean of the
electric and magnetic solve; where only the electric one exists, CST-E scaled by the FDM's open/electric ratio),
the anchors against their references, and the loss sweep. Each comparison uses one definition on both sides:

* **Z0, Zdiff, Zeven**: lossless line impedances – CST with PEC and tanδ 0, FW's full-wave eigenmode with ideal
  conductors, the FDM's 1/(c·√(C·C0)) (static, no conductor loss), the formula. Pairs: Zdiff = 2·Zodd,
  Zeven = 2·Zcomm.
* **εeff (PEC)**: ideal conductors – CST lossless and FW eigenmode at 1 GHz; FW static and FDM static (C/C0) differ
  from these by the dispersion at 1 GHz (≤ 0.3 %).
* **εeff (phase)**: (β/k0)² with copper and tanδ at 1 GHz, the conductors' internal inductance included (+1.2 %
  median, up to +6 % for narrow striplines over the static value) – how js_2d_fields reports εeff (kisi.json);
  compared only with CST's lossy result at the same frequency, never with its lossless one.
* **Loss sweep**: per frequency of CST's lossy grid, α (total, dB/m), R = Re(γ·Zc) and G = Re(γ/Zc) per metre
  (the conductor and the dielectric part apart, without assuming which is which), Re Zc and the phase εeff, for FW
  and for **kisi's line model** (the field path of `line_model_mode`, smooth copper, built from `kisi_loss.json`;
  its microstrip Lorentz term is left out – it shapes ε′ only and depends on the channel's fmax – so the line
  model's phase εeff above 5 GHz is not comparable; α, R and G are not affected). The formula path has no stored
  loss values and is not compared.
* **Asymmetric pairs**: CST's pair impedances belong to the eigenmodes with the power-current definition
  (Z = 2P/|I|², I = the larger conductor current of the differential mode, the sum of both for the common mode);
  kisi's (FDM and FW: upstream's modal decomposition) are 1/(v_p·½vᵀCv) with the voltage vector |v|² = 2 (the ±1 V
  drive of a symmetric pair). FW's 2×2 L and C give the ratio of the two definitions: FW is compared with CST in
  CST's definition, every other kisi value with CST transferred to kisi's. A pair counts as asymmetric when the
  ratio differs from 1 by more than 0.2 % (16 of 133 pairs).
* **Box modes**: above the box's first waveguide cutoff (TE10 of the box filled with its largest εr) the electric
  walls carry modes that either solver can pick or mix with the quasi-TEM mode. A sweep point there is left out for
  every solver when CST and FW differ by more than 3 % in Re Zc or the phase εeff (below the cutoff they agree within
  2.6 %), CST's line impedance is off its lowest-frequency value by more than 2×, or FW's eigenmode εeff jumped by
  more than 2 % between neighbouring sweep points at or below it.

## Results

620 cases, CST Studio Suite 2025.5. Errors in % against CST.

### Impedance at 1 GHz

Median / 90th percentile of |error|. FW and FDM in CST's box with electric walls; kisi's field result (as kisi
reports it) and formula against CST's open-boundary estimate; the etch group's FDM solved kisi's rectangle (left out
of "FDM vs CST", see Outliers).

| group | qty | n | FW vs CST | FDM vs CST | kisi field vs CST open | formula vs CST open |
|---|---|---:|---:|---:|---:|---:|
| `anchor` | z0 | 16 | 0.074 / 0.134 | 0.04 / 0.12 | 0.16 / 0.28 | 1.5 / 10.2 |
| `anchor` | zdiff | 7 | 0.051 / 0.097 | 0.34 / 0.74 | 0.50 / 0.74 | 5.6 / 9.7 |
| `anchor` | zeven | 7 | 0.051 / 0.081 | 0.20 / 0.37 | 0.21 / 0.34 | 4.7 / 19.8 |
| `microstrip` | z0 | 66 | 0.038 / 0.136 | 0.10 / 0.16 | 0.14 / 0.25 | 0.2 / 0.3 |
| `microstrip_h` | z0 | 15 | 0.053 / 0.116 | 0.05 / 0.10 | 0.11 / 0.16 | 0.2 / 0.4 |
| `microstrip_mask` | z0 | 26 | 0.025 / 0.035 | 0.02 / 0.04 | 0.07 / 0.20 | 1.2 / 1.8 |
| `embedded` | z0 | 24 | 0.005 / 0.010 | 0.06 / 0.09 | 0.25 / 0.49 | 0.8 / 2.3 |
| `stripline` | z0 | 29 | 0.028 / 0.041 | 0.03 / 0.06 | 0.28 / 0.37 | 0.2 / 0.4 |
| `stripline_asym` | z0 | 20 | 0.032 / 0.088 | 0.02 / 0.07 | 0.23 / 0.36 | 2.1 / 8.7 |
| `diff_microstrip` | zdiff | 34 | 0.062 / 0.116 | 0.45 / 0.65 | 0.51 / 0.80 | 2.7 / 11.6 |
| `diff_microstrip` | zeven | 34 | 0.043 / 0.080 | 0.25 / 0.38 | 0.09 / 0.27 | 6.0 / 21.7 |
| `diff_stripline` | zdiff | 20 | 0.039 / 0.065 | 0.42 / 0.68 | 0.47 / 0.57 | 4.3 / 32.7 |
| `diff_stripline` | zeven | 20 | 0.037 / 0.055 | 0.27 / 0.38 | 0.29 / 0.44 | 3.3 / 7.1 |
| `cpwg` | z0 | 30 | 0.044 / 0.113 | 0.03 / 0.09 | 0.23 / 0.35 | 4.7 / 18.7 |
| `plane_cut` | z0 | 16 | 0.113 / 0.195 | 0.03 / 0.06 | 0.08 / 0.18 | 17.6 / 60.0 |
| `plane_edge` | z0 | 10 | 0.024 / 0.035 | 0.05 / 0.07 | 0.30 / 0.63 | 0.4 / 3.1 |
| `plane_edge` | zdiff | 2 | 0.070 / 0.070 | 0.40 / 0.41 | 0.74 / 0.83 | 0.9 / 1.3 |
| `plane_edge` | zeven | 2 | 0.033 / 0.043 | 0.26 / 0.28 | 0.18 / 0.29 | 4.1 / 5.0 |
| `size_study` | z0 | 14 | 0.103 / 0.195 | 0.08 / 0.28 | 0.13 / 0.35 | 0.4 / 7.5 |
| `size_study` | zdiff | 2 | 0.125 / 0.164 | 0.53 / 0.60 | 0.52 / 0.56 | 2.1 / 2.2 |
| `size_study` | zeven | 2 | 0.128 / 0.164 | 0.48 / 0.55 | 0.39 / 0.43 | 1.4 / 1.4 |
| `microstrip_t` | z0 | 20 | 0.033 / 0.104 | 0.05 / 0.11 | 0.11 / 0.19 | 0.1 / 0.3 |
| `stripline_dual` | z0 | 6 | 0.053 / 0.103 | 0.02 / 0.02 | 0.30 / 0.45 | 3.5 / 6.6 |
| `broadside` | zdiff | 8 | 0.009 / 0.025 | 0.35 / 0.47 | 0.43 / 0.57 | – |
| `broadside` | zeven | 8 | 0.038 / 0.050 | 0.24 / 0.33 | 0.29 / 0.41 | – |
| `diff_cpwg` | zdiff | 6 | 0.062 / 0.101 | 0.28 / 0.34 | 0.46 / 0.56 | 0.7 / 1.6 |
| `diff_cpwg` | zeven | 6 | 0.045 / 0.077 | 0.17 / 0.26 | 0.14 / 0.28 | 14.9 / 21.3 |
| `stripline_cpw` | z0 | 3 | 0.019 / 0.020 | 0.02 / 0.05 | 0.35 / 0.39 | 2.4 / 10.9 |
| `cpw_noplane` | z0 | 8 | 0.077 / 0.094 | 0.19 / 0.26 | 0.44 / 0.59 | 10.0 / 14.0 |
| `etch` | z0 | 18 | 0.045 / 0.082 | – | 2.66 / 5.85 | 2.5 / 5.6 |
| `etch` | zdiff | 3 | 0.014 / 0.024 | – | 3.42 / 4.79 | 3.3 / 4.7 |
| `etch` | zeven | 3 | 0.007 / 0.008 | – | 1.48 / 2.12 | 4.3 / 4.6 |
| `board_target` | z0 | 88 | 0.021 / 0.073 | 0.01 / 0.11 | 0.14 / 0.31 | 3.8 / 37.2 |
| `board_target` | zdiff | 32 | 0.025 / 0.451 | 0.20 / 0.84 | 0.48 / 0.89 | 4.7 / 8.4 |
| `board_target` | zeven | 32 | 0.019 / 0.280 | 0.12 / 0.36 | 0.21 / 0.37 | 4.7 / 34.4 |
| `test_board` | z0 | 70 | 0.034 / 0.131 | 0.02 / 0.10 | 0.22 / 0.39 | 3.0 / 27.0 |
| `test_board` | zdiff | 11 | 0.054 / 0.077 | 0.29 / 0.67 | 0.40 / 0.51 | 4.1 / 10.0 |
| `test_board` | zeven | 11 | 0.048 / 0.072 | 0.25 / 0.45 | 0.27 / 0.41 | 0.3 / 1.4 |
| `board_other` | z0 | 16 | 0.046 / 0.075 | 0.04 / 0.07 | 0.26 / 0.43 | 7.4 / 16.8 |
| **all** | z0 | 495 | 0.031 / 0.112 | 0.04 / 0.13 | 0.18 / 0.41 | 1.6 / 12.2 |
| **all** | zdiff | 125 | 0.043 / 0.097 | 0.40 / 0.67 | 0.48 / 0.80 | 3.3 / 12.6 |
| **all** | zeven | 125 | 0.037 / 0.079 | 0.25 / 0.39 | 0.23 / 0.41 | 4.1 / 23.6 |

* **FW and CST agree to 0.03–0.04 %** (median; p90 ≤ 0.11 %, max 0.3 % for single lines and 1.1 % for the two PCIe
  finger pairs) with electric walls, the same with magnetic walls (26 cases), and 0.05–0.08 % on the open-boundary
  estimate: two solvers of different kinds (hexahedral port modes, triangular FEM) on the same problems.
* **kisi's field solver (FDM) in the same box**: Z0 −0.03 % (median signed, p90 0.13 %); pairs have a bias of
  −0.40 % (Zdiff) and −0.25 % (Zeven), against FW as well – a property of the rectilinear FDM at narrow gaps.
  As kisi runs it (tol 0.01, its own open domain): Z0 −0.18 %, Zdiff −0.48 %, Zeven −0.21 % (median signed);
  without the etch group and the asymmetric pairs p90 0.47 %, p99 0.78 %, max 1.03 % (`anchor_forsten_suite_6`).
* **Formulas**: plain microstrip, its thickness, height and solder-mask sweeps, covered traces and symmetric
  striplines within 0.2–1.2 % (median, p90 ≤ 2.3 %); offset and dual striplines 2–3.5 % (p90 6.6–8.7 %); edge-coupled
  pairs 3–6 % (p90 7–33 %); CPWG 4.7 % (p90 19 %) and coplanar pairs' Zeven 15 % (the formulas ignore or simplify
  the coplanar ground); plane cut-outs 18 % (p90 60 %; the formula takes the plane below the opening); no plane
  10 % (an estimate, as kisi marks it).
* **Anchors** (HFSS / Ansys / Forstén's MS/SL solver): CST −0.15 % (median signed), FW −0.10 %, kisi's field
  −0.37 %, all within 1.2 %.

### εeff at 1 GHz, per definition

Bias / p90 of the error, all groups.

| qty | FW eigenmode (PEC) | FW static | FDM static | formula | FW phase vs CST lossy | FDM phase vs CST lossy | FDM phase vs FDM static |
|---|---:|---:|---:|---:|---:|---:|---:|
| eeff | -0.000 / 0.070 | -0.006 / 0.101 | -0.001 / 0.090 | -0.16 / 4.50 | -0.50 / 0.87 | -0.54 / 0.95 | +1.24 / 2.45 |
| eeff_odd | -0.014 / 0.063 | -0.015 / 0.070 | +0.020 / 0.067 | +0.00 / 7.76 | -0.60 / 0.91 | -0.63 / 0.98 | +1.97 / 3.09 |
| eeff_even | -0.005 / 0.044 | -0.013 / 0.072 | +0.013 / 0.079 | -0.55 / 3.84 | -0.47 / 0.70 | -0.47 / 0.73 | +1.44 / 2.27 |

With ideal conductors on both sides the solvers agree to 0.01 % (FW) and 0.01–0.02 % (FDM static); homogeneous
striplines give εr exactly in CST (121 cases, |error| < 4·10⁻⁵ %). The phase εeff that js_2d_fields reports includes
the internal inductance (last column); compared with CST's lossy port modes, both kisi sources are about 0.5 % low.
That remainder is CST's dielectric model: with `TDCompatibleMaterials` (a Debye fit of constant tanδ) CST's ε′ is
0.2–0.5 % above the nominal εr and its tanδ ripples by ±3 % over frequency (homogeneous stripline
`sl_h0.1_u1_er4.2`: ε′ 4.211 at 1 GHz for εr 4.2; G of FW against CST −0.4, +1.9, −2.4, +1.8 % at 1, 5, 10, 20 GHz,
alternating), while L of the two agrees within 0.25 %.

### Outliers and their causes

* **Asymmetric pairs** (16: the ThunderScope PCIe pairs with a coplanar ground on one side, the edge fingers, two
  `plane_edge` pairs): CST's Zdiff is up to 51 % above, Zeven up to 15 % below kisi's, with FW and FDM agreeing with
  each other. The eigenmode εeff – independent of any impedance definition – agree within 0.04 % (FW) for all 16;
  CST's impedances reproduced from FW's L and C in CST's definition agree within 0.07 % for 12 pairs, 0.6 % for two
  and 1.0 % for the PCIe edge fingers (`board_ts_ea1df30155`, `board_ts_860e846530`; their odd mode with plane copper
  1.4 mm away, `probes/pcie_fingers`). The cause is the definition (CST: eigenmodes, power-current; kisi: modal,
  |v|² = 2), not a solver error; the comparison transfers CST to kisi's definition.
* **Trapezoidal etch** (`etch`, e.g. `etch_sl_ef1` −11 %): kisi's geometry text and its formula use the rectangle
  of the drawn width at full thickness (`xsgeom.rs` has no etch); CST and FW solve the trapezoid and agree within
  0.1 %. kisi's error in the same box: −1.0…−5.4 % for microstrips, −2.6…−5.1 % for the pair, −4.6…−10.8 % for the
  stripline (etch factor 3…1). Neither simple rectangle fits: the mean width w − t/ef gives +0.3…+11 %, a width
  w − t/(2·ef) within ±2.2 % for microstrips and the pair and −2…−4 % for the stripline (FDM in CST's box). kisi's
  FDM backend does not mesh trapezoids. A kisi geometry limitation, documented here; not changed.
* `anchor_forsten_suite_6` (3 mm wide microstrip pair): the largest field error without those (Zdiff −1.0 %), and
  phase εeff,odd −2.0 % against CST's lossy run.

### Loss

Bias (median signed) / p90 of the error against CST's lossy port modes, all groups; asymmetric pairs and box-mode
points left out (below).

| solver | qty | 0.1 GHz | 1 GHz | 5 GHz | 10 GHz | 20 GHz | 40 GHz |
|---|---|---:|---:|---:|---:|---:|---:|
| fw | α | +14.6 / 24.3 | +6.3 / 13.0 | +5.1 / 10.4 | +1.3 / 8.4 | +4.4 / 9.0 | +1.5 / 8.0 |
| fw | R | +20.4 / 30.4 | +14.3 / 18.9 | +12.5 / 16.3 | +11.8 / 15.6 | +11.4 / 15.8 | +10.8 / 15.3 |
| fw | G | -0.3 / 0.7 | -0.4 / 0.8 | +1.9 / 2.6 | -2.3 / 3.8 | +1.8 / 4.2 | -1.1 / 6.9 |
| fw | Re Zc | +0.28 / 0.48 | +0.05 / 0.15 | +0.06 / 0.22 | -0.06 / 0.49 | -0.04 / 1.25 | -0.12 / 1.86 |
| fw | εeff phase | -0.56 / 0.93 | -0.50 / 0.87 | -0.69 / 1.02 | -0.55 / 0.85 | -0.68 / 1.03 | -0.84 / 1.21 |
| kisi | α | +12.0 / 22.6 | +4.8 / 10.9 | +4.0 / 8.5 | +0.5 / 6.5 | +3.3 / 6.9 | +0.5 / 6.7 |
| kisi | R | +16.7 / 28.8 | +11.1 / 16.2 | +9.4 / 13.3 | +8.6 / 12.9 | +8.2 / 13.0 | +6.9 / 12.6 |
| kisi | G | -0.3 / 0.7 | -0.4 / 0.8 | +1.9 / 2.5 | -2.3 / 3.9 | +1.7 / 4.1 | -1.4 / 7.0 |
| kisi | Re Zc | +0.07 / 0.30 | -0.13 / 0.42 | -0.05 / 0.47 | -0.11 / 0.71 | -0.08 / 1.60 | -0.00 / 2.33 |
| kisi | εeff phase | -0.64 / 1.06 | -0.54 / 1.00 | -0.85 / 1.48 | -1.01 / 1.95 | -1.52 / 3.49 | -2.52 / 5.99 |
| kisi vs fw | α | -1.2 / 4.3 | -1.0 / 3.3 | -0.8 / 2.6 | -0.6 / 2.5 | -0.4 / 2.2 | -0.2 / 2.1 |
| kisi vs fw | R | -1.7 / 5.4 | -2.2 / 5.3 | -2.1 / 5.2 | -2.0 / 5.4 | -1.9 / 5.5 | -2.0 / 5.4 |
| kisi vs fw | G | +0.0 / 0.2 | -0.0 / 0.1 | -0.1 / 0.2 | -0.1 / 0.2 | -0.1 / 0.3 | -0.1 / 0.3 |
| kisi vs fw | Re Zc | -0.20 / 0.59 | -0.17 / 0.54 | -0.09 / 0.56 | -0.00 / 0.68 | +0.14 / 1.24 | +0.44 / 2.39 |
| kisi vs fw | εeff phase | -0.07 / 0.38 | -0.03 / 0.16 | -0.13 / 0.48 | -0.39 / 1.29 | -0.78 / 2.63 | -1.73 / 5.09 |

Per group (R = Re(γ·Zc), G = Re(γ/Zc), α total; bias / p90):

| group | n | FW R 1 GHz | FW R 10 GHz | kisi R 1 GHz | kisi R 10 GHz | FW G 1 GHz | FW α 1 GHz | kisi α 1 GHz |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `anchor` | 30 | +13.6 / 16.5 | +11.1 / 14.9 | +10.5 / 15.1 | +8.1 / 11.8 | +1.3 / 2.2 | +12.7 / 16.6 | +10.5 / 15.2 |
| `microstrip` | 66 | +12.9 / 18.9 | +11.1 / 16.1 | +11.8 / 13.4 | +9.8 / 11.6 | -0.5 / 0.9 | +9.5 / 16.6 | +8.9 / 11.9 |
| `microstrip_h` | 15 | +11.5 / 16.7 | +7.9 / 12.6 | +11.3 / 15.7 | +7.4 / 9.7 | -0.7 / 1.4 | +3.3 / 8.2 | +3.2 / 6.3 |
| `microstrip_mask` | 26 | +11.0 / 15.5 | +9.0 / 12.9 | +9.3 / 11.6 | +7.3 / 9.7 | -0.3 / 0.4 | +3.6 / 6.4 | +3.1 / 4.7 |
| `embedded` | 24 | +13.8 / 16.1 | +11.8 / 13.4 | +13.0 / 14.1 | +10.7 / 12.3 | -0.3 / 0.4 | +7.3 / 12.4 | +7.0 / 10.7 |
| `stripline` | 29 | +14.7 / 21.8 | +11.7 / 15.7 | +10.8 / 16.8 | +9.2 / 11.9 | -0.3 / 0.8 | +8.1 / 16.9 | +6.8 / 13.5 |
| `stripline_asym` | 20 | +15.8 / 19.7 | +12.2 / 14.5 | +14.1 / 16.9 | +11.6 / 14.5 | -0.2 / 0.4 | +9.2 / 11.3 | +8.1 / 10.3 |
| `diff_microstrip` | 68 | +14.3 / 18.0 | +12.0 / 15.3 | +10.7 / 12.1 | +8.7 / 9.7 | -0.6 / 0.8 | +5.6 / 10.9 | +4.3 / 8.4 |
| `diff_stripline` | 40 | +18.1 / 21.6 | +14.2 / 16.6 | +14.3 / 17.2 | +10.8 / 13.0 | -0.2 / 0.4 | +9.7 / 13.4 | +7.2 / 10.1 |
| `cpwg` | 30 | +15.4 / 17.6 | +13.8 / 14.9 | +10.5 / 12.1 | +8.0 / 9.3 | -0.5 / 0.8 | +6.8 / 10.2 | +4.4 / 7.6 |
| `plane_cut` | 16 | +16.0 / 17.7 | +12.1 / 13.8 | +11.6 / 12.3 | +8.8 / 9.6 | -0.7 / 0.8 | +4.7 / 5.5 | +3.2 / 4.0 |
| `plane_edge` | 10 | +11.9 / 12.8 | +8.1 / 9.5 | +9.8 / 10.6 | +5.8 / 8.1 | -0.5 / 0.8 | +4.1 / 4.7 | +3.2 / 3.8 |
| `size_study` | 18 | +13.6 / 16.1 | +10.4 / 13.6 | +10.9 / 17.2 | +7.7 / 14.5 | -0.5 / 1.2 | +4.6 / 6.2 | +3.0 / 6.1 |
| `microstrip_t` | 20 | +12.4 / 17.7 | +10.8 / 14.1 | +10.7 / 14.7 | +8.8 / 11.8 | -0.6 / 1.2 | +4.3 / 8.3 | +3.6 / 5.9 |
| `stripline_dual` | 6 | +16.0 / 17.9 | +13.0 / 14.0 | +11.5 / 13.2 | +8.9 / 10.3 | -0.3 / 0.4 | +8.2 / 9.6 | +5.9 / 6.9 |
| `broadside` | 16 | +16.1 / 20.0 | +14.3 / 17.9 | +13.4 / 15.7 | +11.6 / 15.0 | – | +16.0 / 20.1 | +13.7 / 16.8 |
| `diff_cpwg` | 12 | +16.8 / 18.7 | +15.2 / 17.3 | +9.5 / 11.3 | +7.0 / 9.5 | -0.9 / 1.1 | +7.3 / 9.1 | +4.2 / 5.2 |
| `stripline_cpw` | 3 | +16.4 / 17.7 | +12.6 / 13.4 | +11.0 / 13.2 | +7.7 / 8.9 | -0.4 / 0.4 | +9.5 / 11.0 | +6.4 / 8.2 |
| `cpw_noplane` | 8 | +18.5 / 18.6 | +14.4 / 14.9 | +11.9 / 12.9 | +8.0 / 8.6 | – | +18.4 / 18.5 | +12.1 / 13.1 |
| `etch` | 24 | +18.8 / 28.4 | +16.0 / 26.3 | +2.6 / 7.0 | +0.5 / 6.4 | -0.4 / 0.8 | +7.9 / 13.1 | +1.7 / 3.6 |
| `board_target` | 124 | +13.3 / 18.1 | +10.9 / 14.9 | +9.5 / 16.8 | +7.4 / 13.1 | -0.3 / 0.5 | +5.9 / 9.5 | +4.8 / 9.6 |
| `test_board` | 92 | +14.1 / 19.3 | +11.5 / 15.5 | +11.3 / 18.2 | +8.7 / 14.8 | -0.3 / 0.5 | +4.8 / 6.6 | +3.8 / 5.9 |
| `board_other` | 16 | +15.6 / 19.2 | +13.6 / 15.7 | +13.8 / 17.0 | +11.0 / 13.8 | -0.3 / 0.4 | +7.6 / 9.5 | +5.6 / 9.4 |
| **all** | 713 | +14.3 / 18.9 | +11.8 / 15.6 | +11.1 / 16.2 | +8.6 / 12.9 | -0.4 / 0.8 | +6.3 / 13.0 | +4.8 / 10.9 |

**CST's conductor loss is low.** FW's R is 14 % above CST's at 1 GHz (12 % at 10 GHz, 20 % at 0.1 GHz), in every
group, while G (dielectric) agrees within 0.4 % at 1 GHz and L within 0.25 %. An independent check settles which
side is right: `mqs_check.py` solves the same 2D eddy-current problem (finite volumes on a graded grid, δ/8 at every
conductor face, A = 0 on the box walls; grid error ≤ 0.9 % from δ/4 to δ/8) on 9 cases:

| case | R FD-MQS at 1 GHz (Ω/m) | FW vs FD-MQS at 0.1 / 1 / 10 GHz (%) | CST vs FD-MQS at 0.1 / 1 / 10 GHz (%) |
|---|---:|---:|---:|
| `sl_h0.1_u1_er4.2` | 60.37 | -0.1 / -0.2 / -0.3 | -20.1 / -13.4 / -10.7 |
| `ms_u0.2_er4.4` | 59.95 | -0.1 / -0.2 / -0.3 | -22.5 / -16.0 / -13.1 |
| `ms_u1_er4.4` | 22.18 | -0.0 / -0.3 / -0.2 | -14.9 / -11.4 / -9.9 |
| `ms_u10_er4.4` | 4.03 | -0.0 / -0.2 / -0.2 | -4.6 / -3.8 / +0.5 |
| `cpwg_u1_g0.5` | 37.87 | -0.3 / -0.4 / -0.6 | -19.6 / -14.6 / -12.8 |
| `emb_u1_d0.1_er4.4` | 30.65 | +0.1 / -0.1 / -0.1 | -16.3 / -12.9 / -11.5 |
| `cut_w0.4_c2_r1` | 19.22 | -0.2 / -0.4 / -1.0 | -17.5 / -14.2 / -9.8 |
| `dms_u1_s1` odd | 28.66 | -0.2 / -0.3 / -0.3 | -18.8 / -14.1 / -12.8 |
| `dms_u1_s1` even | 34.36 | +0.1 / -0.1 / -0.2 | -13.7 / -10.3 / -8.5 |
| `board_ts_01a9401349` odd | 37.85 | +0.0 / -0.2 / -0.3 | -18.5 / -13.2 / -11.7 |
| `board_ts_01a9401349` even | 48.89 | +0.3 / +0.1 / -0.1 | -13.4 / -9.6 / -8.4 |

FW and the finite-difference solution agree within 1 % (median 0.2 %); CST is 4–23 % below both. CST's deficit grows
with the share of the current at the conductor edges (microstrip group, w/h 0.1 → 10: −17 → −4 % at 1 GHz,
−15 → 0 % at 10 GHz) and towards low frequency, where the skin depth is no longer small against the copper
thickness (35 µm = 5 δ at 0.1 GHz). Both point to CST's lossy-metal treatment in the frequency-domain port-mode
solver: a surface impedance on a tetrahedral port mesh refined for β and Z, which resolves neither the edge
currents nor the finite thickness. The CST model itself is not the cause: every ground (planes, coplanar grounds)
is copper of its real thickness inside the box (`xsgeo.domain` closes the box on a plane's outer face), only the
box walls are PEC, and they carry no return current of a quasi-TEM line at 12 H; σ, thickness and smoothness are the
same in all three solvers, and the frequency grid is CST's. A CST re-run can confirm it (next section); until then
the loss comparison uses FW, checked against the independent solution, as the reference.

**kisi's line model against FW** (bias / p90 per group; the R of kisi's FDM solve at 1 GHz extended by
R_dc·√(1 + jω/ω_c)):

| group | n | R 0.1 GHz | R 1 GHz | R 10 GHz | α 1 GHz | α 10 GHz |
|---|---:|---:|---:|---:|---:|---:|
| `anchor` | 30 | -1.8 / 5.4 | -2.0 / 5.6 | -1.8 / 5.6 | -1.8 / 5.1 | -2.3 / 5.3 |
| `microstrip` | 66 | -1.1 / 5.7 | -0.8 / 5.3 | -1.0 / 4.4 | -0.5 / 4.4 | -0.5 / 3.1 |
| `microstrip_h` | 15 | -0.1 / 4.8 | -0.1 / 3.3 | -0.5 / 2.6 | -0.1 / 1.9 | -0.1 / 2.2 |
| `microstrip_mask` | 26 | -2.4 / 4.8 | -1.9 / 3.6 | -2.0 / 3.3 | -0.8 / 1.6 | -0.3 / 0.7 |
| `embedded` | 24 | -1.2 / 3.2 | -0.5 / 2.0 | -0.6 / 1.6 | -0.4 / 1.3 | -0.2 / 0.8 |
| `stripline` | 29 | -1.0 / 4.8 | -2.9 / 5.2 | -2.3 / 3.6 | -1.8 / 3.2 | -1.0 / 2.0 |
| `stripline_asym` | 20 | +0.9 / 2.4 | -0.5 / 2.7 | -0.3 / 1.9 | -0.3 / 1.8 | -0.2 / 0.8 |
| `diff_microstrip` | 68 | -2.8 / 5.8 | -3.0 / 5.6 | -2.8 / 5.7 | -1.2 / 3.1 | -0.7 / 2.4 |
| `diff_stripline` | 40 | -1.3 / 3.2 | -3.4 / 5.2 | -3.0 / 4.5 | -1.9 / 3.2 | -1.2 / 1.8 |
| `cpwg` | 30 | -3.7 / 6.1 | -3.9 / 5.6 | -4.5 / 5.6 | -2.1 / 3.2 | -1.1 / 2.0 |
| `plane_cut` | 16 | -4.1 / 5.2 | -4.1 / 4.8 | -2.9 / 3.7 | -1.4 / 1.7 | -0.6 / 0.7 |
| `plane_edge` | 10 | -1.9 / 3.1 | -1.7 / 2.4 | -1.6 / 2.3 | -0.7 / 1.1 | -0.0 / 0.2 |
| `size_study` | 18 | -1.9 / 4.4 | -1.4 / 4.5 | -0.8 / 3.7 | -0.6 / 1.7 | -0.1 / 2.9 |
| `microstrip_t` | 20 | -1.1 / 4.9 | -1.4 / 3.9 | -1.5 / 3.4 | -0.7 / 2.2 | -0.3 / 0.9 |
| `stripline_dual` | 6 | -1.9 / 2.7 | -3.6 / 4.3 | -3.2 / 3.7 | -2.1 / 2.5 | -1.0 / 1.2 |
| `broadside` | 16 | +0.2 / 2.5 | -1.6 / 3.8 | -1.5 / 3.4 | -1.3 / 3.3 | -1.2 / 3.0 |
| `diff_cpwg` | 12 | -5.2 / 6.7 | -5.5 / 7.2 | -5.8 / 7.7 | -2.7 / 4.0 | -1.4 / 2.5 |
| `stripline_cpw` | 3 | -3.0 / 3.8 | -4.6 / 5.4 | -4.2 / 4.9 | -2.9 / 3.4 | -1.6 / 1.8 |
| `cpw_noplane` | 8 | -5.0 / 6.1 | -5.7 / 6.6 | -5.9 / 6.8 | -5.4 / 6.3 | -5.8 / 6.7 |
| `etch` | 24 | -13.7 / 25.4 | -12.9 / 24.5 | -12.7 / 25.3 | -5.7 / 12.1 | -3.8 / 8.0 |
| `board_target` | 124 | -1.1 / 4.2 | -1.7 / 4.4 | -1.8 / 4.7 | -0.8 / 2.0 | -0.4 / 1.3 |
| `test_board` | 92 | -1.5 / 2.9 | -2.1 / 3.1 | -2.0 / 3.4 | -0.8 / 1.4 | -0.3 / 0.8 |
| `board_other` | 16 | -1.7 / 4.2 | -1.4 / 4.3 | -1.5 / 4.5 | -1.0 / 2.3 | -0.7 / 1.1 |
| **all** | 713 | -1.7 / 5.4 | -2.2 / 5.3 | -2.0 / 5.4 | -1.0 / 3.3 | -0.6 / 2.5 |

kisi's FDM gives 2 % less R than FW at 1 GHz (p90 5 %), more for coplanar structures (CPWG −4 %, coplanar pairs
−5.5 %, no plane −5.7 %); the frequency model keeps that offset constant from 0.1 to 40 GHz, so it extends the
1-GHz value correctly. α is 1 % low (p90 3 %) at 1 GHz and 0.6 % at 10 GHz, where the dielectric dominates. The etch
group's −13 % is the rectangle (wider, so less R). Roughness is not part of this check (smooth copper everywhere;
kisi's channel models multiply by the Huray factor).

**Box modes**: of 4278 sweep points (4470 with the asymmetric pairs), 496 are left out – 25 at 5 GHz, 46 at 10 GHz,
164 at 20 GHz, 261 at 40 GHz – all above the box's first cutoff; CST lost the quasi-TEM mode (impedance off by
more than 2×) in 247 of them, FW's eigenmode jumped in 252, and CST and FW differ by more than 3 % in 490. None below
the cutoff.

### Uncertainty of the CST results (`uncertainty.py`)

| Source | Z (median / p90 / max) | εeff (median / p90 / max) |
|---|---|---|
| change of the last mesh level (convergence criterion 0.1 %) | 0.07 / 0.14 / 0.30 % | 0.02 / 0.09 / 0.28 % |
| last level against its extrapolation | 0.03 / 0.05 / 0.15 % | 0.01 / 0.03 / 0.09 % |
| εeff = εr in a homogeneous dielectric (121 cases) | – | < 4·10⁻⁵ % |
| electric against magnetic walls (26 cases: the box's effect on the open estimate) | 0.20 / 1.23 / 1.50 % | |
| CST against FW in the same box (upper bound for both) | 0.03 / 0.10 / 1.07 % | 0.01 / 0.07 / 0.26 % |
| CST against the FDM in the same box (kisi's definition) | 0.08 / 0.40 / 1.44 % | |

CST's lossless impedances are good to about 0.1 % (p90); the open-boundary estimate adds the wall spread's
uncertainty for open structures (the two wall kinds bracket the open value; their mean changes by < 0.1 % with a
doubled margin). The lossy port modes' conductor loss is not covered by these numbers (above).

## CST re-run for the conductor loss

A frequency-domain re-run of a few cases tells where CST's low conductor loss comes from (options of `cst_xs.py`,
section "The CST model"; results go to `results/<id>.<tag>.lossy.json`, next to the stored ones):

```sh
cd validation/cst_xs
IDS=sl_h0.1_u1_er4.2,ms_u0.2_er4.4,ms_u1_er4.4,ms_u10_er4.4,cpwg_u1_g0.5,dms_u1_s1
python cst_xs.py --lossy --fd --all-cases --ids $IDS --freqs 0.1,1,10 --tag fdmesh2 --tet-scale 2 --detach
python cst_xs.py --lossy --fd --all-cases --ids $IDS --freqs 0.1,1,10 --tag fdmesh4 --tet-scale 4 --detach
python cst_xs.py --lossy --fd --all-cases --ids $IDS --freqs 0.1,1,10 --tag fdnotd --no-td-compat --detach
python cst_xs.py --lossy --fd --all-cases --ids $IDS --freqs 0.1,1,10 --tag fdvol --metal volume --tet-scale 4 --detach
```

Read R = Re(γ·Zc) from each result (`compare.cst_lossy`) and compare with `mqs_check.json` (FD-MQS and FW for these
cases): if R rises towards the MQS value with the mesh density, it is CST's port mesh at the edges (a finer port
mesh, or CST's local mesh refinement at the conductor edges, is then needed for loss); if only `--metal volume` gets
there, it is the surface impedance; `--no-td-compat` separates the material fit (it should also remove the
0.2–0.5 % ε′ offset and the tanδ ripple). The options are not tested in CST yet: run `--dry-run` first and check the
`.bas` (material type, mesh settings, `TDCompatibleMaterials`); with `--metal volume` the interior of the copper must
be meshed to the skin depth (0.66 µm at 10 GHz), which may need CST's own mesh refinement for lossy metal.

## The page

```sh
python3 validation/cst_xs/make_viewer.py validation/cst_xs dist/kisi-references-2d.html
```

`kisi-references-2d.html` (self-contained, about 3 MB; built, not kept in the repository; a release attaches it when
the cases, the results or the page builder changed): an overview with the per-group tables of `compare.py` and the
cases with a note, and per case the cross-section to scale (dielectrics with εr and thickness, conductors, etch
trapezoids with kisi's rectangle, the solver box), the parameters, the four solvers' values at 1 GHz with their
deviation from CST, agreement chips (the limits of `compare.py`: 2 % for the solvers and kisi's field result, 5 % for
the formula; the definitions in the tooltips), the explanation of known outliers, and the loss sweep (α, R, Re Zc,
phase εeff over frequency: CST, FW, kisi's line model; points left out as box modes hollow). The 3D reference
cases are on `kisi-references-3d.html` (`validation/fdtd/make_viewer.py`); the two pages link to each other.
