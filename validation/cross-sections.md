# 2D references: cross-sections

kisi's line impedances come from two paths: closed-form formulas (`kisi_si::analytic`, chosen per cross-section by
`kisi_si::extract::formula_xs`) and the quasi-static 2D field solver of H. Forstén (`third_party/js_2d_fields`, FDM
backend), which solves kisi's own cross-section geometry (`kisi_si::xsgeom`, a geometry text). kisi's line model
(`kisi_si::rf::line`) takes R, L, G, C from the field solve at 1 GHz. Both paths are checked on 620 cross-sections
against two solvers of other kinds that solve exactly the geometry text kisi solves: CST Studio Suite's 2D port-mode
solver and the full-wave triangular FEM backend of js_2d_fields. The page `kisi-references-2d.html` (built by
`validation/cst_xs/make_viewer.py`, attached to the releases in which the cases or results changed) shows every
case. What the user sees of it: [docs/models.md](../docs/models.md), sections Impedance and Validation.

## Cases

`make_cases.py` builds each case as a stackup and a cross-section description – the inputs of `xsgeom::build` and of
the formula path – and `kisi-validate xs-cases` turns them into kisi's geometry text and formula values; the board
cases come from `kisi-validate xs-board` (the geometries kisi extracts on a board). Units mm; `w/h` = width over the
height to the reference.

| Group | n | Sweep |
|---|---:|---|
| `anchor` | 23 | Forstén's test suite (10, HFSS/Ansys references), ThunderScope (6) and asymmetric striplines (7) with references of Forstén's MS/SL solver |
| `microstrip` | 66 | w/h 0.1…10 × εr 2.2…10.2 (h 0.3, t 35 µm) |
| `microstrip_t`, `_h`, `_mask` | 20, 15, 26 | copper 17.5…105 µm; height 0.07…1.6 mm; solder mask 10…35 µm, εr 3.3 / 4.0 |
| `embedded` | 24 | covered trace (inner layer, one plane), cover 0.05…0.5 mm |
| `stripline`, `stripline_asym`, `stripline_dual` | 29, 20, 6 | symmetric; offset h2/h1 1.5…8 and different εr above / below; an empty signal layer between the planes |
| `diff_microstrip`, `diff_stripline` | 34, 20 | edge-coupled pairs, s/h 0.25…4 × w/h, mask, symmetric / offset |
| `broadside` | 8 | broadside-coupled stripline (no kisi model: solver check only) |
| `cpwg`, `diff_cpwg`, `stripline_cpw` | 30, 6, 3 | coplanar ground: gap/h 0.25…4, mask; a pair; a stripline |
| `cpw_noplane` | 8 | coplanar without a plane (kisi's no-plane formula, an estimate) |
| `plane_cut`, `plane_edge` | 16, 12 | trace over an opening in the next layer (1…8 × w); a plane ending 0.5…8 h beside the trace |
| `etch` | 21 | trapezoidal etch, factor 1, 2, 3: CST and FW solve the trapezoid, kisi its rectangle |
| `size_study` | 16 | 8 cases with the solver box × 1.5 and × 2 (the box rule) |
| `board_target`, `board_other` | 120, 16 | ThunderScope and LibreVNA: the cross-sections of nets with a target impedance by routed length; the longest other ones |
| `test_board` | 81 | every distinct cross-section of the planned test-board coupons (branch `feat/test-board`) |

## Solvers and definitions

* **CST Studio Suite 2025.5, 2D port modes** (`cst_xs.py`, CST's Python): the geometry text painted into
  non-overlapping rectangles (`xsgeo.py`) and extruded; PEC conductors, lossless dielectrics; one multipin waveguide
  port with potentials on the zmin face, modes at 1 GHz. Mesh levels (20·2^k lines per box) until Z and εeff change
  by less than 0.1 %. Box rule: max(12 H, 4 Ws, 0.5 mm) beyond the outermost finite edge (H the farther reference of a
  signal, Ws the signal group's width); a side whose outermost object is a full plane closes on it. Each case with
  **electric** and with **magnetic** side walls: they bias an open structure in opposite directions by about the
  same amount, and their mean is the **open-boundary estimate** (the mean changes by < 0.1 % with a doubled margin).
  Lossy port modes (frequency-domain solver, copper 5.8e7 S/m, the tanδ of the geometry as CST's constant-tanδ fit)
  at 0.1, 1, 5, 10, 20, 40 GHz, electric walls.
* **Full-wave FEM (FW)** of js_2d_fields (`run_fullwave.py`, unmodified upstream code; validation only, not in kisi's
  bundles; GitHub Actions workflow `fullwave-xs`): in CST's box with both wall kinds, static P2 FEM and full-wave
  eigenmode at 1 GHz; a sweep 0.01–40 GHz with Djordjevic–Sarkar dielectrics and MQS conductor loss.
* **kisi's FDM** (`run_kisi.py` → `kisi.json`): `field` as kisi runs it (tol 0.01, its own open domain), `field_E` /
  `field_M` in CST's box with electric / natural walls (tol 0.002), and `formula`.
* **kisi's line model** (`compare.py --solve-loss` → `kisi_loss.json`): the FDM solve in CST's box at 1 GHz,
  extended over frequency as kisi does (R_dc·√(1 + jω/ω_c), smooth copper).

One quantity, one definition in a comparison:

* **Z0, Zdiff, Zeven:** lossless line impedances – CST with PEC and tanδ 0, FW's eigenmode with ideal conductors,
  the FDM's 1/(c·√(C·C0)), the formula. Pairs: Zdiff = 2·Zodd, Zeven = 2·Zcomm (CST's mode 1 = differential, mode
  2 = common).
* **εeff (PEC):** ideal conductors, 1 GHz. **εeff (phase):** (β/k0)² with copper and tanδ, the conductors' internal
  inductance included – how js_2d_fields reports εeff; compared only with CST's lossy result.
* **Loss:** per frequency α (dB/m), R = Re(γ·Zc) and G = Re(γ/Zc) per metre (conductor and dielectric part apart),
  Re Zc and the phase εeff.
* **Asymmetric pairs** (16 of 133: the ThunderScope PCIe pairs with coplanar ground on one side, the edge fingers,
  two `plane_edge` pairs): CST's pair impedances are the eigenmodes' power-current values, kisi's (FDM, FW) the
  modal values with |v|² = 2. FW's 2×2 L and C give the ratio; CST is transferred to kisi's definition for every
  kisi value.
* **Box modes:** above the box's first waveguide cutoff a sweep point is left out for every solver where CST and FW
  differ by more than 3 % or either loses the quasi-TEM mode (496 of 4278 points, all at 5–40 GHz).

Three comparisons: the **solver check** (CST, FW and kisi's FDM on the identical closed box: differences are
numerical only), **kisi's accuracy** (kisi as it reports – its field solver on its own domain, its formula –
against CST's open-boundary estimate) and **loss** (against FW's MQS conductor loss, see the known limits).

## Current results

Errors in % against CST; median / 90th percentile of |error|, all groups (`compare.py`):

| Quantity | n | FW vs CST (same box) | kisi's FDM vs CST (same box) | kisi field vs CST open | kisi formula vs CST open |
|---|---:|---:|---:|---:|---:|
| Z0 | 495 | 0.03 / 0.11 | 0.04 / 0.13 | 0.18 / 0.41 | 0.7 / 4.6 |
| Zdiff | 125 | 0.04 / 0.10 | 0.40 / 0.67 | 0.48 / 0.80 | 2.9 / 12.7 |
| Zeven | 125 | 0.04 / 0.08 | 0.25 / 0.39 | 0.23 / 0.41 | 2.9 / 21.8 |

* **The solvers agree:** FW and CST within 0.03–0.04 % (median, p90 ≤ 0.11 %) with either wall kind. kisi's FDM in
  the same box: Z0 −0.03 % (signed median); pairs −0.40 % (Zdiff) and −0.25 % (Zeven) against FW as well – the
  rectilinear FDM at narrow gaps.
* **kisi's field solver as it reports:** Z0, Zdiff, Zeven median 0.18 %, p90 0.47 %, p99 0.78 %, max 1.03 % without
  the etch group and the asymmetric pairs; pairs' Zdiff median 0.48 %, p90 0.80 % → `data/accuracy.json` `field_pct`
  1 %. The anchors (HFSS / Ansys / Forstén's MS/SL): all within 1.2 %.
* **Formulas** (median / p90): plain microstrip and its thickness, height and mask sweeps, covered traces and
  symmetric striplines 0.2–1.2 / ≤ 2.3 %; offset and dual striplines 2–3.5 / 6.6–8.7 %; edge-coupled pairs Zdiff
  2.7 / 11.6 % (microstrip) and 4.3 / 32.7 % (stripline); plane cut-outs 18 / 60 % (kisi flags them "rough");
  coplanar lines CPWG 0.5 / 1.3 %, the 104 single-ended cases with coplanar copper 0.9 / 1.7 % (max 2.7 %), stripline
  with coplanar ground 0.7 %, coplanar pairs Zdiff 2.2 / 3.6 %, coplanar without a plane 0.3 % (infinite grounds;
  on a board an estimate). These are `data/accuracy.json`'s formula levels; the table per line type is in
  [docs/models.md](../docs/models.md#impedance).
* **εeff:** with ideal conductors FW and FDM agree with CST to 0.01–0.02 % (bias), homogeneous striplines give εr in
  CST to 4·10⁻⁵ %. The phase εeff of both kisi sources is about 0.5 % below CST's lossy result: CST's constant-tanδ
  fit puts ε′ 0.2–0.5 % above the nominal εr.
* **Loss, kisi's line model against FW:** R 2 % low at 1 GHz (p90 5 %), more on coplanar structures (CPWG −4 %,
  coplanar pairs −5.5 %, no plane −5.7 %), the offset constant from 0.1 to 40 GHz; α 1 % low (p90 3 %) at 1 GHz,
  0.6 % at 10 GHz; G within 0.1 %. Roughness is not part of this check (smooth copper; kisi's channel multiplies by
  the Huray factor).

**Uncertainty of the CST results** (`uncertainty.py`): the last mesh level changes Z by 0.07 / 0.14 % (median / p90),
the extrapolation adds 0.03 / 0.05 %; electric against magnetic walls 0.20 / 1.23 % on the 26 cases with both (the
open estimate sits between them); CST against FW in the same box 0.03 / 0.10 %. CST's lossless impedances are good
to about 0.1 % (p90).

## Known limits

* **Trapezoidal etch:** kisi's geometry and formula use the rectangle of the drawn width at full thickness (KiCad's
  stackup has no etch factor); CST and FW solve the trapezoid and agree within 0.1 %. kisi's error: −1.0…−5.4 %
  (microstrip), −2.6…−5.1 % (pair), −4.6…−10.8 % (stripline) for etch factor 3…1. A width w − t/(2·etch factor)
  would come within ±2.2 % for microstrips and pairs, −2…−4 % for striplines; kisi's FDM backend does not mesh
  trapezoids.
* **Asymmetric pairs:** CST's Zdiff up to 51 % above, Zeven up to 15 % below kisi's – a matter of the impedance
  definition, not a solver error (the eigenmode εeff agree within 0.04 %; CST's impedances rebuilt from FW's L and C
  agree within 0.07 % for 12 of 16). The PCIe edge fingers, with plane copper 1.4 mm away under one finger, keep a
  1 % difference; `cst_xs/probes/pcie_fingers` made the pair symmetric in both solvers (then within 2–3 %).
* **CST's conductor loss is low:** FW's R is 14 % above CST's at 1 GHz (12 % at 10 GHz, 20 % at 0.1 GHz) in every
  group, while G agrees within 0.4 % and L within 0.25 %. An independent finite-difference eddy-current solution
  (`mqs_check.py`, 9 cases, δ/8 at every conductor face) agrees with FW within 1 % (median 0.2 %) and puts CST 4–23 %
  low, most for narrow conductors and at low frequency. A CST re-run of six cases (tetrahedral mesh × 2 and × 4, the
  port mesh following it, the dielectric fit switched off; `results/<id>.fd{mesh2,mesh4,match4,notd}.lossy.json`)
  moved R by at most 1.5 points; volume conductors cannot be solved by CST's port-mode solver with this licence
  (`fdvol`, `fdvolm`). The deficit is CST's surface-impedance treatment, so the loss is judged against FW.
* **Formula approximations:** a pair with coplanar ground on one side only is the mean of two symmetric pairs
  (Zeven typically 10 % off, up to −47 / +71 % for a 25 µm gap under mask); stripline pairs and covered traces with
  one plane ignore their coplanar copper. The field solver is not affected.
* **The largest field error** apart from those: `anchor_forsten_suite_6` (a 3 mm wide microstrip pair), Zdiff
  −1.0 %, phase εeff,odd −2.0 % against CST's lossy run.

## The coplanar formulas' constants

The formulas for lines with coplanar copper have two fitted constants: the corner term of a slot between copper side
walls, C/ε0 = t/s + a·(t/s)^b, and the width τ of the smooth maximum that joins the coplanar capacitances to the plain
line's. `fit_coplanar.py` fits them to kisi's field solver on 1 100 synthetic cross-sections (grounded CPW with and
without mask, stripline with coplanar ground, coupled grounded CPW, CPW without a plane; h 0.1–1.6 mm, εr 3–10, t
17.5–70 µm, w/h 0.5–4, gap/h 0.1–4), stored with their field solutions in `coplanar_fit.json`: a = 0.0673, b = 0.090,
τ = 0.045 (mean |error| of Z0 0.845 %); the code uses a = 0.067, b = 0.090, τ = 0.045. Errors against the field solver
(median / p90): grounded CPW 0.28 / 1.01 %, with mask 0.65 / 1.61 %, stripline with coplanar ground 0.61 / 3.5 %, CPW
without a plane 0.31 / 0.63 %, coupled grounded CPW with ground on both sides Zdiff 1.0 / 3.7 %.

## Reproduce

Python 3 (numpy, scipy for `mqs_check.py`), node for kisi's field solver. From the repository root:

```sh
cargo build --release && cargo build --release -p kisi-sparse --target wasm32-unknown-unknown
python3 validation/cst_xs/make_cases.py --test-boards DIR   # cases.json (needs the example boards; DIR: the test boards of feat/test-board)
python3 validation/cst_xs/run_kisi.py                       # kisi.json (≈ 25 min on 2 cores; --formula-only after a formula change)
python3 validation/cst_xs/compare.py --solve-loss           # kisi_loss.json (kisi's line-model solves, ≈ 2 h on one core)
python3 validation/cst_xs/compare.py                        # every comparison (--groups, --ids, --md, --json, --loss-table, --per-case)
python3 validation/cst_xs/mqs_check.py                      # the independent eddy-current check → mqs_check.json
python3 validation/cst_xs/uncertainty.py
python3 validation/cst_xs/fit_coplanar.py --from-json       # the formula constants from the stored sweep
python3 validation/cst_xs/make_viewer.py validation/cst_xs dist/kisi-references-2d.html
```

CST (in `validation/cst_xs`, with CST's Python libraries on `PYTHONPATH` or `--cst-lib DIR`; about a minute per case):
`python cst_xs.py --probe`, then `--tier 1|2|3 --skip-done --detach` (watch with `--status`), the lossy sweeps with
`--lossy --fd --tag fd --all-cases`; `--dry-run --ids ID` writes the VBA history without CST. Results go to
`results/<id>.json` and `results/<id>.fd.lossy.json` (committed; solver logs and projects are not). The full-wave
backend runs on GitHub Actions (workflow `fullwave-xs`, cases of `fullwave_plan.json`):
`gh run download <run> -p 'fullwave-*' -D /tmp/fw && cp /tmp/fw/*/*.json validation/cst_xs/fullwave/`;
`fullwave_compare.py` is the workflow's summary.

A formula change: `run_kisi.py --formula-only`, `compare.py`, and `data/accuracy.json` where the levels move. A new
case group: `make_cases.py` (and its `GROUPS` in `make_viewer.py`), then CST, FW and `run_kisi.py` for it.

| File | What |
|---|---|
| `cases.json`, `make_cases.py`, `xsgeo.py` | the 620 cases (spec, kisi's geometry text, solver box); geometry parser and box rule |
| `cst_xs.py`, `results/` | CST runner; lossless (`<id>.json`) and lossy (`<id>.fd.lossy.json`; earlier time-domain `<id>.lossy.json`, the loss re-run `<id>.fd<tag>.lossy.json`) port modes |
| `run_fullwave.py`, `fullwave_driver.mjs`, `fullwave_plan.json`, `fullwave/` | the full-wave backend |
| `run_kisi.py`, `kisi.json`, `kisi_loss.json` | kisi's formula, field and line-model results |
| `compare.py`, `fullwave_compare.py` | the comparison |
| `fullwave_diff.py` | two sets of full-wave results against each other (what an upstream update or a new build moved; the workflow's `collect` job against the committed results) |
| `mqs_check.py`, `mqs_check.json`, `uncertainty.py` | the independent conductor-loss check, the CST uncertainty |
| `fit_coplanar.py`, `coplanar_fit.json` | the coplanar formulas' constants |
| `probes/pcie_fingers/` | the asymmetric edge-finger pair made symmetric in CST and kisi |
| `make_viewer.py`, `viewer_template.html` | the page |

This document replaces `validation/cst_xs/README.md` and the full-wave run's summary (`fullwave/summary.md`); their
last state is commit `3e4d835` (`git show 3e4d835:validation/cst_xs/README.md`).
