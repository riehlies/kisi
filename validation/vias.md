# 3D references: vias, lines and plane pairs

kisi's fast models of a channel – the via ladder (`kisi_si::via_pbm`), the coaxial fallback, the line and coupling
models, pads and gold fingers, the plane-cavity model of the PDN (`kisi_pi::cavity_modal`) – are checked against 3D
full-wave results. The references come from kisi's own 3D FDTD solver (`crates/kisi-fdtd`); every case was computed
again with openEMS and with CST Studio Suite. The page `kisi-references-3d.html` (built by
`validation/fdtd/make_viewer.py`, attached to the releases in which the cases changed) shows every case and the
tables of this document. What the user sees of it: [docs/models.md](../docs/models.md), sections Vias and Validation.

## Reference cases

106 cases, each a kisi-fdtd case JSON with a `meta` block (group, title, parameters, stackup, ports); openEMS
(`openems_case.py`) and CST (`cst_case.py`) build their models from the same files.

| Group | Cases | What it tests | Folder |
|---|---|---|---|
| A | 6 | coaxial section L1→L4 through one plane pair, 4 GND vias: drill 0.2 / 0.3 mm, antipad Ø 0.6 / 0.8 / 1.0 mm | `via_cases` |
| B | 8 | stubs: L1→L3 with the stub to L8, back-drilled to 0.2 mm, L1→L8 without stub; 8-layer and ThunderScope stack | `via_cases` |
| C | 8 | antipad Ø 0.6–1.2 mm, non-functional pads kept or removed, an offset antipad, mixed antipads | `via_cases` |
| D | 8 | return path: one GND via at 0.5 / 1 / 2 / 5 mm or none; GND → PWR plane through a capacitor at 2 / 10 mm or none | `via_cases` |
| E | 12 | via pairs: pitch 0.8–1.2 mm, round or oval antipads, 0 / 2 / 4 GND vias, L1→L8 and L1→L3 | `via_cases` |
| X | 2 | L1→L3 as in B without the L2 or the L4 plane (the junction of trace and via apart from the shared cavity) | `via_cases` |
| T | 12 | lines and coupling: microstrip, stripline, CPWG (10 and 20 mm, for Zc and εeff), coupled microstrip and stripline pairs, aggressor/victim, broadside | `via_cases` |
| P | 5 | a 0402 pad (plane intact or cut out) and card-edge gold fingers (far plane, no plane, a pair) in a 50 Ω microstrip | `via_cases` |
| G | 2 | plane pair 50 × 30 mm, d = 0.1 / 0.2 mm, 1 Ω ports at a corner and the centre | `via_cases` |
| H | 4 | BGA pin field to one 0402 capacitor at 2 / 5 / 10 / 20 mm: loop inductance | `via_cases` |
| board | 5 | vias cut out of the example boards (`kisi-validate via-case`): ThunderScope M.2 PCIe pair (also with In2 grounded), REFINOUT2 through via; LibreVNA RF via with stub, FPGA_CLK_R | `board_cases`, `resonance_cases`, `return_cases` |
| R | 6 | finite boards 24 × 16 mm with open edges: one via with 0 / 1 / 4 return vias, L-shaped planes, a capacitor, two vias sharing the cavity | `resonance_cases` |
| RG | 4 | return groups: GND vias that pass a supply plane (floating) or join it, with and without a capacitor; 4-layer and ThunderScope stack | `return_cases` |
| thru | 24 | 2×thru fixture per feed layer and stack (twice the feed length), for de-embedding | all four |

Stackups (`via_cases.py`): a 4-layer and a generic 8-layer 1.6 mm FR-4 stack (the latter also without its L2 or L4
plane for X), the ThunderScope Rev5.3 and LibreVNA stacks from their board files, and the plane pairs of G.
Feeds are 4 mm traces of about 50 Ω from 50 Ω lumped ports (G, H: 1 Ω). Pairs use kisi's port order: 1 = P near,
2 = N near, 3 = P far, 4 = N far. Planes run into the absorbing boundary, so they are infinite and the board edges
add no resonances – except in R, RG and the board cut-outs, whose boards end in air.

The case files: `via_cases.py` writes `via_cases/`, `board_vias.py export` writes `board_cases/`. The files in
`resonance_cases/` and `return_cases/` are kept as they were written by the studies of the experimental options
(below); those scripts are in the git history.

## Solvers and meshes

**kisi-fdtd** (`crates/kisi-fdtd`): Yee FDTD on a rectilinear non-uniform mesh. Copper is zero-thickness sheets
(traces, planes with round or slot antipads, polygon copper of board cut-outs) and solid via cylinders, treated
conformally (Dey–Mittra with a stable effective face area, so it runs at 0.95 of the Courant limit); convolutional
PML of 8 cells; lumped ports and series R–L–C elements, implicit and unconditionally stable; dielectric loss as a
conductivity fixed at `f_loss` (tanδ exact there). Gaussian pulse, one run per excited port, stop at −50 dB energy.
Mesh: fixed lines at layer boundaries, sheet edges, vias and ports with the spacing `dfine` there (thirds rule at
trace edges), growing by 1.4 up to λ/20 at f max; `dfine` 0.035 mm (G 0.1, H 0.07 mm). The unit tests
(`cargo test -p kisi-fdtd`) check a cylindrical cavity against its analytic resonance, a microstrip, the stability of
conformal pads and the lumped R–L–C.

The transitions that end on an inner layer, the ThunderScope through via B_ts6_L1L6, RG_ts6 and the board vias with
an inner exit are computed with the cell at metal halved (`meta.ref_refine: 2`): on 0.035 mm their delay was
1.1–2.3 ps slow, which both other solvers showed. Refined, a further halving moves the delay by 0.03–0.11 ps and
|S21| by 0.003–0.034 dB (mean) on the via cases (`meta.json` → `mesh_convergence`); the base-mesh results are kept
as `kisi_base.sNp` (the dashed curve on the page). The board vias are not converged to the same degree: their
copper (45° polygons, fills, narrow clearances) moves FPGA_CLK_R's notches by 0.07–0.12 GHz and the M.2 pair's delay
by about 0.7 ps per halving.

**openEMS 0.37.0** (staircase metal, UPML), every case on two meshes: kisi-fdtd's own (*base*, together with
kisi-fdtd in staircase mode: the like-for-like check of the implementation) and with the cell at metal halved
(*fine*: staircase converges at first order, so base → fine estimates its remaining error). kisi-fdtd's references
are compared with *fine*; G and H (one-edge ports) on *base*. Invalid and left out on *fine*: D_s8_cap_2 / _10 (the
explicit lumped RLC update loses stability on the finer mesh), BV_ts_m2_per0 and BV_vna_rf_hblb1 (UPML keeps their
energy). Runs on GitHub Actions (`openems-refs` workflow, `openems_plan.json`).

**CST Studio Suite 2025.5** (time domain, FIT with PBA conformal metal, discrete face ports, open boundaries), CST's
own mesh: base 20 / 20 lines per wavelength / per model box, fine 30 / 30, extra-fine 40 / 40, intermediate 25 / 25,
at most 1 M cells. Every case is shown and compared on its finest mesh: extra-fine on 5 (CST's mesh check: fine →
extra-fine ≤ 0.06 dB, ≤ 0.2 ps), fine on 96, intermediate on BV_vna_clk_w5, base on D_s8_cap_10 and three board vias.
A CST case is de-embedded with the 2×thru on its own mesh, or on the thru's finest where the thru lacks that mesh.

## De-embedding

A via is compared alone: each port's feed is removed with the 2×thru of its feed layer, from the same solver (and,
for CST, the same mesh). The method is the NZC 2×thru (IEEE P370 style: fixture S11 from the gated 2×thru
reflection, then S22 and S21 from the cascade) in its **matched** form (`sparam.deembed_matched`,
`deembed_matched_nport`): each port is first referred to its own feed line's impedance Zc(f), the fixture removed,
and the result referred back to 50 Ω. The reason: with the dielectric loss as a conductivity, the 3D lines are not
50 Ω at low frequency (Zc² = Z∞²·jω/(jω + g); the 8-layer L3 stripline is 10 % low at 1 GHz), and a 50 Ω-referred
NZC leaves an impedance step at each reference plane in the via's TDR. On a known network – a π-section via between
lossy lines of 50–114 Ω – the 50 Ω-referred NZC is off by up to +82 fF / −200 pH, the matched form within 5 fF /
4 pH and 0.2 dB (`board_vias.py deembed-check`, `results/board_vias_deembed_check.json`). Pairs: each of the four
ports with its own fixture (the feeds are uncoupled single-ended lines), then the differential mode. The board vias
have their own in/out fixtures, de-embedded the same way.

**Exception, group T:** the 4-ports of coupled lines (the pairs and the aggressor/victim cases) keep a first-order
shift (each port's feed delay and loss, no reflection), because their feeds are the coupled line itself, which a
per-port single-line fixture does not model.

What the de-embedding cannot settle: a nearly lossless via has |S11| = |S22|, but the de-embedded 8-layer L1→L3 vias
keep a difference up to 0.045 (pairs 0.07) with either method – the fixture error of the lumped port between the L3
stripline and its planes. The TDR minimum of those exits is known to about ±4 % only.

## Scoring

Two scripts score kisi against the de-embedded 3D result, on the same set: the single vias of A–D and X without the
GND → PWR cases D_s8_cap_\* (29) and the pairs of E (12).

* **kisi's channel** (`compare_kisi.py`): each case as a mini KiCad board (`kisi_board.py`), computed by
  `kisi net … --s2p/--s4p` with formula impedances and kisi's via model, ports at the ends of the feeds as in 3D, up
  to kisi's 25 GHz. The via alone: kisi's channel de-embedded with kisi's own 2×thru, the 3D result with the 3D
  2×thru, both matched. These scores are `data/accuracy.json`'s via numbers; `tools/golden.py vias` reruns the
  script in CI and compares with `results/kisi_models.json`. The full channel (feeds included) is in the same file
  for every case, also the lines, coupling and pads of T and P.
* **The ladder alone** (`via_ladder.py`): `kisi-validate models --grid graded`, kisi's ladder with its ports at the
  via, against each solver's de-embedded result → `results/via_ladder.json` and the curves `via_ref/<case>/ladder.s2p`;
  and the plane-cavity model on G → `via_ref/<case>/model.s2p`.

Metrics: the TDR of the via alone (Gaussian edge 25 ps 10–90 %, 80 ps window; pairs Zdiff) – the relative error of its
minimum and of its maximum, both pooled for the median and the 90th percentile; the largest Δ|S21| (pairs ΔSdd21) up
to 20 GHz; the delay difference Δτ from the S21 phase between 5 and 20 GHz; the stub notch frequency.

## Current results

**Accuracy of kisi's models** (`data/accuracy.json`, from `results/kisi_models.json`):

| Model | Set | TDR error median / p90 | Other |
|---|---|---|---|
| via ladder, single vias | 29 | 2.43 / 5.13 % | max Δ\|S21\| median 0.17 dB; \|Δτ\| median 0.21 ps; stub notch +0.9 / −1.4 % |
| via ladder, pairs | 12 | Zdiff 4.02 / 5.65 % | max ΔSdd21 median 0.17 dB, phase 1.8°; \|Δτ\| median 0.18 ps |
| coaxial fallback | – | 8.0 / 25.5 % | scored against the references and de-embedding before the ladder (`results/kisi_models_coax.json`); not rescored |

**The ladder alone against the three solvers** (`results/via_ladder.json`; median / p90 / max):

| Reference | Set | TDR error % | TDR minimum, signed median | max Δ\|S21\| dB (median / p90) | \|Δτ\| ps (median / p90) |
|---|---|---|---|---|---|
| kisi-fdtd | single (29) | 1.19 / 3.45 / 11.3 | −1.4 % | 0.09 / 0.83 | 0.12 / 0.39 |
| openEMS (fine) | single | 1.31 / 3.97 / 10.5 | −1.3 % | 0.08 / 1.03 | 0.41 / 0.56 |
| CST (finest) | single | 1.80 / 3.34 / 8.7 | +1.6 % | 0.11 / 0.80 | 0.18 / 0.41 |
| kisi-fdtd | pairs (12) | 1.48 / 3.30 / 3.8 | +1.8 % | 0.08 / 0.58 | 0.07 / 0.35 |
| openEMS (fine) | pairs | 2.38 / 4.28 / 6.0 | +3.4 % | 0.07 / 1.20 | 0.41 / 0.70 |
| CST (finest) | pairs | 1.49 / 5.18 / 6.4 | +4.0 % | 0.13 / 0.46 | 0.06 / 0.38 |

The spread between the three references of a set is about the size of the ladder's own error: the fast model is
judged at the limit of what the references settle. openEMS' |Δτ| carries its staircase offset (below).

**Board vias** (hold-outs, not used for any fit; `board_vias.py`, `results/board_vias.json`; the ladder with the 3D
loss model against kisi-fdtd, matched): the ThunderScope M.2 pair (Sdd) max Δ|S21| 0.18 dB up to 12 GHz, 0.19 dB to
20 GHz, TDR minimum +6.2 %; REFINOUT2 0.04 / 0.04 dB, −0.4 %; the LibreVNA RF via with its 0.87 mm stub 0.37 dB to
12 GHz, 2.2 dB to 20 GHz (its stub notch: 24.4 GHz in 3D, 22.8 GHz in the ladder, −7 %), TDR +1.7 / +3.3 %.

**Lines, coupling, pads** (kisi's channel, formula impedances, `results/kisi_models.json`): Zc / εeff at 5 GHz of the
20 mm lines – microstrip 48.6 Ω / 2.58 in 3D, kisi 49.2 Ω / 2.55; stripline 46.5 / 3.92, kisi 51.6 (+11 %) / 3.73;
CPWG 51.1 / 3.12, kisi 49.9 / 3.10 (the formulas; kisi's field solver on cross-sections:
[cross-sections.md](cross-sections.md)). Coupling of the microstrip aggressor/victim
(35 ps step, near / far end): 3D 2.46 / 6.63 % and 0.98 / 3.26 %, kisi 2.36 / 7.01 % and 0.86 / 3.48 %; broadside
coupling has no kisi model yet. Pads and fingers (TDR with a 20 ps edge, the section's extreme against the line level): single pads and
fingers within about 4 Ω (8 %); the finger pair without any plane reads about 15 % low – an estimate
(`data/accuracy.json`, `no_plane_pct`).

**Plane cavity and PDN loop**: kisi's plane-cavity model on G – the |Z11| resonances up to 4.5 GHz within 0.8 % of
kisi-fdtd, |Z21| below 1 GHz within 0.7 %, |Z11| about 7 % low (the size of the one-cell port). The loop inductance
of H is defined by the port model to about ±4 %: kisi-fdtd 1.33 / 1.46 / 1.66 / 2.07 nH at 2 / 5 / 10 / 20 mm (50 MHz),
CST 0.3–6 % and openEMS 4.3–8.2 % above it.

**The solvers against each other** (group medians of the per-case mean |S21| difference; Δτ = τ kisi-fdtd − τ other):

| Check | Vias, lines, pads (A–E, P, T, X, thru) | Finite boards (R, RG) | Board vias |
|---|---|---|---|
| kisi-fdtd vs openEMS, same mesh, both staircase (102 cases) | 0.007–0.046 dB | 0.04–0.07 dB | 0.02 dB |
| kisi-fdtd vs openEMS fine | 0.01–0.10 dB, Δτ −0.03 … −0.57 ps | 0.10–0.18 dB | 0.47 dB, −1.1 ps |
| kisi-fdtd vs CST (finest) | 0.02–0.08 dB, Δτ −0.11 … +0.36 ps | 0.09–0.23 dB | 0.24 dB, −0.6 ps |

openEMS is slower on every via by 0.4–0.6 ps: staircase metal makes vias, pads and antipads slow and halving the cell
barely changes it; kisi-fdtd's conformal result is the converged one there, and CST, conformal as well, agrees with
it within 0.4 ps (group medians). Stub notches: CST and kisi-fdtd agree within 0.2–0.6 dB, openEMS is off by up to 4 dB on the
notch flank. The finite boards' sharp resonances depend on the mesh of the open edges in all three. G: the first
resonances agree within 1 % in all three. Per group: the page's tab "Solver checks and accuracy"
(`results/openems_refs.json`, `results/cst_compare.json`).

## Known limits

* **Inner-layer exits with one plane close and the other far or missing** (X_s8x4, B_ts6_L1L3_stub, X_s8x2): the
  ladder is too capacitive, the TDR minimum 7.5–11 % low against kisi-fdtd and openEMS; the via's loss can be
  overestimated (X_s8x4 −0.4 against −0.1 dB at 20 GHz). None of the model changes tried fixed them without moving
  the other vias. The 8-layer L1→L3 exits with both planes close are within −4.0 … +0.5 %.
* **Pairs:** the remaining TDR error (+1 … +4 %, L1→L8) grows as the pitch shrinks and with oval antipads.
* **Above 12 GHz** the ladder follows 3D only where its return posts close the via's cage; a stub notch above
  20 GHz can be off by several percent (the LibreVNA RF via: −7 %), which gives dB-level errors near the notch.
* **Board vias** and the finite boards are mesh-limited in all three solvers; their 3D results carry an uncertainty of
  about 0.5 ps and a few tenths of a dB.
* **The coaxial fallback** is scored only against the former references (above).
* **Not covered:** asymmetric pairs (a jog, a stub on one line) in 3D, broadside coupling in kisi, the vertical part
  of the PDN loop (pins, vias, capacitor body), measurements (a test board is planned).

## Experimental options

`--via-cavity` (finite plane-pair cavities, `kisi_si::via_cavity`) and `--via-returns` (return groups per plane
pair) were studied on the R, RG and board cases and are off by default. The last results: on one via through a
finite 4-layer board without return vias, max / rms Δ|S21| below 12 GHz 4.9 / 1.1 dB with infinite plates, 1.8 /
0.5 dB with the cavity (resonances found 6 of 6, frequency within 2 %); with four return vias at 1 mm both 0.04 dB.
Return groups on GND vias that do not join a PWR plane: 8.5 / 2.0 dB before, 2.5 / 0.7 dB with return groups and
cavities. Above 12 GHz neither option follows 3D, and on the board vias they bring no gain, so they stay options.
The cavity option shares its grid factorisations per board region (ThunderScope `check --all` 105 → 57 s on one
thread). Those studies (scripts, tables, plots) are in the git history (below). The warning for a via that crosses
a GND/supply plane pair no return via bridges (`via::open_pairs`) is in the default path: RG_s4_pwr_n4 and its
capacitor variant are warned, the bridged and GND/GND cases not.

## Reproduce and extend

Python 3 with numpy (matplotlib for the plots of `board_vias.py`). From the repository root:

```sh
cargo build --release -p kisi-fdtd -p kisi-cli
python3 validation/fdtd/via_cases.py validation/fdtd/via_cases                       # the case files
python3 validation/fdtd/run_refs.py validation/fdtd/via_cases validation/via_ref --exe target/release/kisi-fdtd --jobs 2
python3 validation/fdtd/compare_kisi.py target/release/kisi                          # kisi's channel → results/kisi_models.json
python3 tools/golden.py vias target/release/kisi                                     # the same, compared with the stored file
python3 validation/fdtd/via_ladder.py --exe target/release/kisi-validate             # the ladder alone → results/via_ladder.json
python3 validation/fdtd/make_viewer.py validation/via_ref dist/kisi-references-3d.html
# board vias (the example boards: sh tools/fetch_examples.sh examples)
python3 validation/fdtd/board_vias.py export|run|ladder --stored-geom|compare|deembed-check
# openEMS (with its Python bindings and shapely; on GitHub Actions: workflow openems-refs)
python3 validation/fdtd/openems_refs.py plan --exe target/release/kisi-fdtd
python3 validation/fdtd/openems_refs.py run --shard 0 --exe target/release/kisi-fdtd
python3 validation/fdtd/openems_refs.py merge validation/fdtd/openems_raw
python3 validation/fdtd/openems_refs.py stair --exe target/release/kisi-fdtd --out /tmp/stair --jobs 2
python3 validation/fdtd/openems_refs.py compare --stair /tmp/stair                   # → results/openems_refs.json
# kisi-fdtd cases too large for one machine: fdtd_refs.py plan, workflow fdtd-refs, fdtd_refs.py merge --keep-base
# CST (CST's Python libraries on PYTHONPATH): run_cst.py runs cases (cst_case.py writes the VBA history),
python3 validation/fdtd/cst_compare.py --fmax-cmp 20e9                               # → results/cst_compare.json
```

A model change: `tools/golden.py vias … --update` and `via_ladder.py`, commit `results/kisi_models.json`,
`results/via_ladder.json`, the `ladder.s2p` files and, where the numbers move, `data/accuracy.json` – the review
shows what moved. A new case: add it to `via_cases.py` (a new group also to `GROUPS` of `viewer_template.html`),
run `run_refs.py` with `--only`, then openEMS and CST for it, and the scripts above. `kisi-validate via-geom
geom.json [--s2p f.s2p]` evaluates one via geometry as `KISI_DEBUG_VIA=1` prints it from `kisi`.

| File | What |
|---|---|
| `fdtd/via_cases.py`, `run_refs.py`, `fdtd_refs.py` (+ `fdtd_plan.json`) | cases, kisi-fdtd runs (`via_ref/<case>/kisi.sNp`, `meta.json`, `fields.json`) |
| `fdtd/openems_case.py`, `openems_refs.py` (+ `openems_plan.json`) | openEMS (`openems.sNp`, `openems_base.sNp`), `results/openems_refs.json` |
| `fdtd/cst_case.py`, `run_cst.py`, `cst_compare.py` | CST (`cst*.sNp`, `cst*_meta.json`), `results/cst_compare.json` |
| `fdtd/sparam.py` | Touchstone I/O, de-embedding, mixed mode, TDR |
| `fdtd/kisi_board.py`, `compare_kisi.py` | kisi's channel (`kisi_model.sNp`), `results/kisi_models.json` |
| `fdtd/via_ladder.py` | the ladder alone (`ladder.s2p`), the cavity model (`model.s2p`), `results/via_ladder.json` |
| `fdtd/board_vias.py`, `board_cases/` | board vias (`ladder_geom.json`, `ladder*.s2p`, `deembedded.s2p`), `results/board_vias*.json` |
| `fdtd/make_viewer.py`, `viewer_template.html` | the page |

## History

This document replaces a test log and the study reports of the development (the last state of all of them is
commit `3e4d835`, e.g. `git show 3e4d835:validation/fdtd/TESTS.md`): `validation/fdtd/TESTS.md` (sections 1–19),
`BOARD_VIAS.md`, `BOARD_RESONANCES.md`, `BOARD_RETURNS.md`, `CST.md`, `README.md`, `validation/via_references.md`,
and the scripts and results that only served them – the Python prototype of the ladder (`via_pbm.py`, `pbm.s2p`) and
its Rust parity tables (`rust_models.py`), the solver studies (`cavity.py`, `via_convergence.py`, `check_pml.py`,
`openems_compare.py`), the junction fit, the PDN-loop and cavity prototypes, the cost measurements (`practice.py`)
and the studies of the experimental options (`board_resonances.py`, `board_returns.py`). Code comments that name
a finding of those studies refer to this history.
