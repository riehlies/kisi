# CST Studio Suite cross-check of the 3D via references

The reference cases of `via_cases/`, `board_cases/`, `resonance_cases/` and `return_cases/` (kisi-fdtd,
`README.md`) computed a second time with CST Studio Suite, a commercial solver of a different implementation
(FIT time domain with PBA conformal meshing, CST's own mesher and port models), as an independent check of
kisi-fdtd next to openEMS (branch `feat/openems-refs`).

Status: **done** (2026-10-31). All 106 cases computed; 101 on the fine mesh, 5 only on coarser meshes because
the fine mesh exceeds the available mesh size (> 1 M cells): BV_vna_clk_w5 on an intermediate mesh, four on
the base mesh. Extra-fine mesh check on five cases.

## Setup

| Item | Value |
|---|---|
| CST | CST Studio Suite **2025.5** (Release 2025-05-30) |
| Computer | Intel Core i7-10700K; time-domain solver with 8 threads, GPU acceleration off |
| Driver | `run_cst.py` under Python 3.11 with CST's Python libraries (`cst.interface`, `cst.results`); the model is built from VBA history blocks written by `cst_case.py` |
| Solver | time domain (FIT), hexahedral mesh with PBA (conformal metal), steady-state energy criterion = the case's `end_db` (−50 dB), all ports excited |
| Mesh | CST's automatic mesh, ratio limit 20, edge refinement 4. **base**: 20 lines per wavelength at fmax, 20 lines per model box; **fine** (`--tag fine`): 30 / 30; **extra-fine** (`--tag xfine`): 40 / 40; **intermediate** (`--tag mid`, only where the fine mesh does not fit): 25 / 25 |
| Mesh size | at most 1 M cells; a case that exceeds it is reported as not run, never coarsened |
| Frequency | 0 … fmax (40 GHz for via cases, 6 GHz G, 2 GHz H); CST's 1001 samples interpolated (linear, complex) to the case grid f = fmax (i+1)/nf |
| Run time | fine mesh: 74 k … 988 k cells (median 386 k), 15 … 490 s per case, 2.5 h for all |

## Model (same as kisi-fdtd and openEMS)

* **Dielectric:** one brick per layer across the board. Loss as a constant conductivity
  σ = 2π f_loss ε0 εr tanδ (f_loss = `f_loss` or fmax/2), exactly as kisi-fdtd and openEMS: tanδ is right at
  f_loss and falls as 1/f away from it (CST material type "Normal" with `Sigma`, `TanDGiven False`).
* **Copper:** zero-thickness PEC sheets (traces, planes, pads), round or slot-shaped antipads cut out of the
  planes, via barrels as solid PEC cylinders cut out of the dielectric (`Solid.Insert`).
* **Zone fills** (board cases): the even-odd rings of the case are first resolved into simple boundary loops
  (KiCad's fills join outline and holes by zero-width bridges and run slits along the outline and back; the
  overlapping edges are cancelled), then covered and combined by nesting depth with Boolean add/subtract. Copper
  that reaches beyond the domain is cut to it by a Boolean intersection in CST. Checked against the case by
  rasterising: the copper area agrees to the raster step (10 µm) on every sheet.
* **Ports:** 50 Ω (G, H: 1 Ω) **discrete face ports** from the reference plane to the trace end, across the full
  trace width, like kisi-fdtd's lumped port. To give the face port an edge on the plane, a 0.05 mm long piece of
  the plane under the port is split off as its own sheet (same copper). Ports of zero width (G, H) are discrete
  edge ports. Port order as kisi: pairs 1 = P near, 2 = N near, 3 = P far, 4 = N far.
* **Lumped elements:** CST `LumpedElement` RLCSerial between the same points.
* **Boundaries:** open (PML) on all sides. Where the board fills the domain (all via cases), the background space
  in x and y is 0 and CST extends the touching planes and dielectric into the PML, so they are infinite as in
  kisi-fdtd; above and below the board, vacuum up to the case's domain. G and H (finite boards) get vacuum up to
  the domain on all sides.

With discrete *edge* ports (one wire across the middle of the trace, the first try) |S11| of the thru was up to
10 dB higher above 10 GHz: the port model matters for raw S11, the face port fixes it.

## Comparison

`python3 cst_compare.py --fmax-cmp 20e9 --openems <via_ref of the openEMS branch> --md table.md` →
`results/cst_compare.json`. Up to 20 GHz: raw S-parameters at the ports and, for vias, de-embedded with the
2×thru of the same solver (NZC as `kisi_deembedded.s2p`; board vias: matched de-embedding as `board_vias.py`);
pairs as Sdd. Δdelay from a linear fit of the unwrapped S21 phase. CST is taken on the fine mesh where it fits,
else on the base mesh. kisi-fdtd: `kisi.sNp` (conformal). openEMS: `openems.sNp` on its fine mesh (staircase),
merged from `openems_raw` of `feat/openems-refs` at 0efc987 (`openems_refs.py merge openems_raw`, not committed
here).

### By group (median over the cases of each group; Δ|S21| max up to 20 GHz, raw)

| Group | n | CST – kisi-fdtd | CST – openEMS | openEMS – kisi-fdtd |
|---|---|---|---|---|
| thru | 24 | 0.026 dB (0.007…0.27), Δdelay −0.56…+0.09 ps | 0.028 dB (0.006…0.25), −0.88…+0.03 ps | 0.020 dB (0.005…0.10), −0.11…+0.61 ps |
| A (4-layer via, drill/antipad) | 6 | 0.034 dB (0.014…0.060), −0.32…−0.07 ps | 0.085 dB (0.055…0.11), −0.60…−0.30 ps | 0.051 dB (0.043…0.064), −0.02…+0.47 ps |
| B (8- / 6-layer transitions, stubs) | 8 | 0.077 dB (0.025…0.46), **−2.43…+0.16 ps** | 0.072 dB (0.019…3.7), −0.49…−0.11 ps | 0.063 dB (0.031…3.5), **−2.09…+0.50 ps** |
| C (antipad variants) | 8 | 0.036 dB (0.022…0.095), +0.06…+0.18 ps | 0.037 dB (0.026…0.094), −0.53…−0.27 ps | 0.043 dB (0.031…0.070), +0.40…+0.60 ps |
| D (return vias, caps) | 8 | 0.085 dB (0.050…0.23), −1.05…+0.26 ps | 0.083 dB (0.033…0.12)¹ | 0.066 dB (0.040…0.25)¹ |
| E (pairs, Sdd) | 12 | 0.060 dB (0.038…0.42), −1.43…+0.03 ps | 0.061 dB (0.022…1.5), −0.81…−0.45 ps | 0.018 dB (0.009…1.6), −0.66…+0.55 ps |
| P (pads, fingers) | 5 | 0.14 dB (0.033…0.15), −0.32…−0.02 ps | 0.12 dB (0.048…0.20) | 0.037 dB (0.028…0.047) |
| T (lines, coupling) | 12 | 0.041 dB (0.012…0.13), −0.10…+0.50 ps | 0.042 dB (0.010…0.89) | 0.017 dB (0.007…0.86) |
| X (via arrays) | 2 | 0.030 dB, −0.07…+0.02 ps | 0.074 dB | 0.053 dB |
| R (plane resonances) | 5² | 0.92 dB (0.093…1.6), mean Δ 0.19 dB | 1.1 dB (0.096…1.2) | 0.58 dB (0.18…0.72) |
| RG (return gaps) | 4 | 0.39 dB (0.19…1.2) | 1.5 dB (0.19…2.5) | 1.5 dB (0.16…2.6) |
| board vias (BV) | 5 | see below | ¹ | ¹ |

¹ openEMS results that are not passive (|S| > 1: `D_s8_cap_2`, `D_s8_cap_10`, `BV_ts_m2_per0`,
`BV_vna_rf_hblb1`) or contain NaN (`BV_ts_m2_per0_q050f25s_e35_in2gnd`) are left out; the cases with lumped
capacitors and these board cases need a look on the openEMS side. CST and kisi-fdtd agree on them (table below).
² `R_s4_rect_2v`: kisi.s4p and openems.s4p contain NaN (excitations missing); CST only.

**PDN cases (G, H, 1 Ω ports)**

| Case | CST | kisi-fdtd | openEMS |
|---|---|---|---|
| G_pp100 \|Z11\| peaks (GHz) | 1.45, 2.42, 2.83, 2.91, 3.79, 4.35 | 1.45, 2.40, 2.81, 2.90, 3.78, 4.34 | 1.45, 2.40, 2.81, 2.90, 3.77, 4.33 |
| G_pp200 \|Z11\| peaks (GHz) | 1.46, 2.42, 2.83, 2.91, 3.81, 4.36 | 1.45, 2.40, 2.82, 2.90, 3.79, 4.34 | 1.45, 2.40, 2.82, 2.90, 3.78, 4.33 |
| H_s8_cap_2 loop L at 100 MHz | 1.417 nH | 1.344 nH | 1.481 nH |
| H_s8_cap_5 | 1.493 nH | 1.475 nH | 1.611 nH |
| H_s8_cap_10 | 1.694 nH | 1.686 nH | 1.823 nH |
| H_s8_cap_20 | 2.099 nH | 2.101 nH | 2.254 nH |

G: all resonances within 1 % in all three solvers; mean |Z11| below 1 GHz CST/kisi-fdtd 0.998 and 0.963.
H: CST −0.1 … +1.2 % against kisi-fdtd (+5.4 % at 2 mm cap spacing), openEMS 7 … 10 % above kisi-fdtd and
4.5 … 8 % above CST.

## Mesh convergence

* base → fine: thrus Δ|S21| ≤ 0.07 dB (output thru of BV_vna_clk_w5 0.27 dB); vias mostly ≤ 0.5 dB; up to
  0.6 … 0.9 dB on the stub notches (B stub, E L1→L3) and P fingers, 1.5 … 2.1 dB on RG_s4 (resonances).
  On the base mesh the 50 Ω feeds of the drill-0.3 / 8-layer cases came out at 53.9 Ω (kisi-fdtd 49.3 Ω); on
  the fine mesh 49.6 Ω. All results below are on the fine mesh where it fits.
* fine → extra-fine (40 / 40) on thru_s8_L1, thru_s8_L3, C_s8_ap800, B_s8_L1L8_d300, B_s8_L1L3_bd_d200:
  Δ|S21| ≤ 0.06 dB, Δdelay ≤ 0.2 ps, |ΔS11| ≤ 0.04 (thru_s8_L3 0.10). The fine mesh is converged for these.
* Where the fine mesh exceeds the available mesh size: `BV_vna_clk_w5` on the intermediate mesh (25 / 25,
  726 k cells; base → intermediate 0.69 dB up to 15 GHz); `D_s8_cap_10`, `BV_ts_m2_per0`, `BV_ts_refinout2`,
  `BV_vna_rf_hblb1` on the base mesh (the intermediate mesh takes about 2.5 times the base cells, more than
  1 M for these; for comparison D_s8_cap_2 moves by 0.22 dB base → fine).
  `BV_ts_m2_per0`, `BV_ts_refinout2` and `BV_vna_rf_hblb1` were run with an earlier construction of the zone-fill
  polygons that gives the same copper.

## Findings

1. **Lines, thrus, single vias, antipads, via arrays, pairs (A, C, E L1→L8, T, X, thrus): the three solvers
   agree** to 0.01 … 0.1 dB in |S21| up to 20 GHz and to ≤ 0.6 ps in delay. TDR minima within ~0.3 Ω (except
   striplines, where the TDR start differs by the port model).
2. **Delay of L1→L3 and L1→L6 transitions: kisi-fdtd is the outlier.** CST is 1.0 … 2.4 ps faster than kisi-fdtd
   (B_s8_L1L3_bd_d200 −2.20 ps de-embedded, unchanged on the extra-fine mesh; B_ts6_L1L6 −1.64 ps;
   E_s8_L1L3 −1.4 ps; RG_ts6_sw −1.4 ps). On the B cases openEMS agrees with CST within 0.1 … 0.5 ps
   (openEMS – kisi-fdtd −2.09 ps on B_s8_L1L3_bd_d200, −1.40 ps on B_ts6_L1L6); on E_s8_L1L3 and RG_ts6_sw it
   sits between the two (−0.6 … −1.0 ps against kisi-fdtd). The thrus of the same layers (thru_s8_L1, _L3,
   thru_ts6_L1, _L6) agree within 0.3 ps in all three, and the L1→L8 transitions agree too, so the extra
   delay sits in kisi-fdtd's via of a transition that ends on an inner layer. |S21| is not affected (≤ 0.09 dB).
3. **Stub notches** (B_s8_L1L3_stub, E_s8_L1L3): CST and kisi-fdtd agree to 0.3 … 0.45 dB; openEMS is
   0.8 … 3.7 dB off on the flank of the notch (B_ts6_L1L3_stub: all three within 0.3 dB).
4. **Plane-resonance cases R, RG:** up to 0.9 … 1.6 dB Δ|S21| near the cavity resonances between any two
   solvers (mean 0.1 … 0.3 dB); the resonances are sharp and their exact frequencies depend on the mesh of the
   open plane edges. R: kisi-fdtd and openEMS are closer to each other (0.2 … 0.7 dB) than to CST
   (0.9 … 1.6 dB); RG_s4: CST and kisi-fdtd closer (0.5 … 1.2 dB) than openEMS (2.4 dB). Delays agree within
   0.3 ps.
5. **Board vias of the VNA board (BV_vna_clk_w5, BV_vna_rf_hblb1): CST's base mesh is too coarse.** On the base
   mesh CST has more reflection than kisi-fdtd and openEMS, which agree with each other (|S11| −4.3 vs −5.4 dB at
   10 GHz) and so 0.6 … 0.7 dB less |S21| up to 15 GHz; the dissipated power is about the same. On the
   intermediate mesh (25 / 25) BV_vna_clk_w5 moves onto the other two: Δ|S21| up to 15 GHz 0.19 dB against
   kisi-fdtd and 0.16 dB against openEMS (base: 0.72 dB). Above 15 GHz all three differ by several dB at a
   notch near 17 GHz (openEMS – kisi-fdtd 4.4 dB as well); the max values in the table come from there. The
   geometry agrees with the case (zone fills checked by rasterising; thrus of the same layers within
   0.1 … 0.27 dB). BV_vna_rf_hblb1 (805 k cells on the base mesh) does not fit a finer mesh, so its 0.85 dB
   is a mesh effect to be expected, not a solver difference. The TS board vias agree with kisi-fdtd to
   0.15 … 0.27 dB on the base mesh; the pair BV_ts_m2_per0_q050f25s_e35_in2gnd (Sdd) 0.10 dB on the base and
   0.41 dB on the fine mesh (Δdelay −0.9 ps).
6. **H loop inductance:** CST within 1.2 % of kisi-fdtd, 5.4 % at 2 mm cap spacing (H_s8_cap_2); openEMS
   7 … 10 % higher. G resonances: all three within 1 %.

## Per case (CST on the finest mesh that fits)

Columns: CST mesh cells and run time; CST – kisi-fdtd raw Δ|S21| max / mean, de-embedded Δ|S21| max, raw |ΔS11|
max (linear), Δdelay (de-embedded where available, negative: CST faster), TDR minimum (25 ps rise); mesh change
(fine → extra-fine where computed, else base → fine or base → intermediate); CST – openEMS (de-embedded, else raw) Δ|S21| max and Δdelay.

| Case | Mesh | Cells | Time | Δ\|S21\| max / mean raw (dB) | Δ\|S21\| max de-emb. (dB) | \|ΔS11\| max | Δdelay (ps) | TDR min CST / kisi (Ω) | mesh change Δ\|S21\| max (dB) | openEMS: Δ\|S21\| max de-emb. or raw (dB), Δdelay (ps) |
|---|---|---|---|---|---|---|---|---|---|---|
| A_s4_d200_ap1000 | fine | 339 k | 45 s | 0.021 / 0.009 | 0.033 | 0.008 | -0.22 | 43.96 / 43.95 | 0.032 | 0.072, -0.34 |
| A_s4_d200_ap600 | fine | 339 k | 45 s | 0.060 / 0.019 | 0.066 | 0.017 | -0.32 | 43.55 / 43.58 | 0.015 | 0.096, -0.30 |
| A_s4_d200_ap800 | fine | 339 k | 47 s | 0.041 / 0.011 | 0.051 | 0.013 | -0.23 | 43.77 / 43.93 | 0.025 | 0.091, -0.31 |
| A_s4_d300_ap1000 | fine | 339 k | 45 s | 0.014 / 0.007 | 0.022 | 0.011 | -0.07 | 43.62 / 43.64 | 0.006 | 0.024, -0.52 |
| A_s4_d300_ap600 | fine | 339 k | 46 s | 0.044 / 0.017 | 0.053 | 0.015 | -0.14 | 39.86 / 40.0 | 0.021 | 0.052, -0.60 |
| A_s4_d300_ap800 | fine | 339 k | 46 s | 0.027 / 0.012 | 0.032 | 0.010 | -0.09 | 42.96 / 43.19 | 0.014 | 0.033, -0.56 |
| BV_ts_m2_per0 | base | 799 k | 613 s | 0.270 / 0.111 | 0.202 | 0.053 | +0.51 | 34.88 / 36.05 | – | not passive (\|S\| > 1) |
| BV_ts_m2_per0_q050f25s_e35_in2gnd | fine | 673 k | 225 s | 0.410 / 0.165 | – | 0.100 | -0.90 | 35.03 / 35.97 | 0.472 | contains NaN |
| BV_ts_refinout2 | base | 519 k | 220 s | 0.146 / 0.062 | 0.036 | 0.039 | +0.84 | 44.9 / 45.09 | – | 0.081, +0.31 |
| BV_vna_clk_w5 | mid | 726 k | 445 s | 6.165 / 0.412 | 5.713 | 0.657 | +0.98 | 35.32 / 37.65 | 3.054 | 4.191, +0.42 |
| BV_vna_rf_hblb1 | base | 805 k | 331 s | 0.854 / 0.363 | 0.931 | 0.134 | -0.54 | 27.78 / 29.02 | – | not passive (\|S\| > 1) |
| B_s8_L1L3_bd_d200 | fine | 386 k | 72 s | 0.069 / 0.029 | 0.064 | 0.113 | -2.20 | 42.29 / 40.59 | 0.041 | 0.045, -0.11 |
| B_s8_L1L3_bd_d300 | fine | 386 k | 72 s | 0.086 / 0.045 | 0.086 | 0.086 | -1.04 | 39.67 / 39.07 | 0.267 | 0.044, -0.15 |
| B_s8_L1L3_stub_d200 | fine | 386 k | 76 s | 0.455 / 0.181 | 0.446 | 0.501 | -2.43 | 31.01 / 29.8 | 0.643 | 1.559, -0.49 |
| B_s8_L1L3_stub_d300 | fine | 386 k | 74 s | 0.397 / 0.152 | 0.367 | 0.279 | -1.34 | 28.26 / 27.8 | 0.817 | 3.594, -0.44 |
| B_s8_L1L8_d200 | fine | 386 k | 72 s | 0.025 / 0.014 | 0.044 | 0.046 | +0.11 | 44.11 / 44.08 | 0.050 | 0.077, -0.15 |
| B_s8_L1L8_d300 | fine | 386 k | 72 s | 0.038 / 0.017 | 0.054 | 0.034 | +0.06 | 42.89 / 43.12 | 0.046 | 0.034, -0.44 |
| B_ts6_L1L3_stub | fine | 312 k | 50 s | 0.134 / 0.058 | 0.265 | 0.040 | +0.16 | 31.79 / 32.49 | 0.301 | 0.210, -0.15 |
| B_ts6_L1L6 | fine | 303 k | 48 s | 0.035 / 0.018 | 0.030 | 0.095 | -1.64 | 44.57 / 43.25 | 0.023 | 0.029, -0.24 |
| C_s8_ap1000 | fine | 386 k | 71 s | 0.022 / 0.007 | 0.063 | 0.033 | +0.09 | 44.1 / 44.18 | 0.202 | 0.025, -0.34 |
| C_s8_ap1200 | fine | 386 k | 72 s | 0.048 / 0.015 | 0.067 | 0.033 | +0.09 | 44.42 / 44.36 | 0.114 | 0.029, -0.31 |
| C_s8_ap600 | fine | 386 k | 73 s | 0.063 / 0.033 | 0.068 | 0.034 | +0.08 | 37.89 / 37.98 | 0.506 | 0.066, -0.53 |
| C_s8_ap800 | fine | 386 k | 72 s | 0.038 / 0.017 | 0.054 | 0.034 | +0.06 | 42.89 / 43.12 | 0.046 | 0.034, -0.44 |
| C_s8_mixed | fine | 386 k | 72 s | 0.032 / 0.014 | 0.121 | 0.033 | +0.09 | 44.11 / 44.05 | 0.283 | 0.070, -0.33 |
| C_s8_nfp | fine | 386 k | 72 s | 0.095 / 0.044 | 0.126 | 0.040 | +0.18 | 38.69 / 39.24 | 0.554 | 0.117, -0.27 |
| C_s8_nonfp | fine | 386 k | 72 s | 0.028 / 0.011 | 0.056 | 0.034 | +0.08 | 43.94 / 44.02 | 0.259 | 0.029, -0.38 |
| C_s8_offset | fine | 386 k | 72 s | 0.033 / 0.014 | 0.064 | 0.034 | +0.09 | 43.84 / 43.9 | 0.291 | 0.046, -0.32 |
| D_s8_cap_10 | base | 468 k | 83 s | 0.301 / 0.146 | 0.265 | 0.174 | -1.57 | 40.87 / 39.02 | – | not passive (\|S\| > 1) |
| D_s8_cap_2 | fine | 474 k | 82 s | 0.081 / 0.042 | 0.091 | 0.082 | -1.05 | 39.73 / 39.02 | 0.220 | not passive (\|S\| > 1) |
| D_s8_cap_none | fine | 447 k | 79 s | 0.081 / 0.043 | 0.090 | 0.083 | -1.05 | 39.73 / 39.02 | 0.223 | 0.038, -0.08 |
| D_s8_stitch_0p5 | fine | 601 k | 88 s | 0.070 / 0.031 | 0.088 | 0.035 | +0.18 | 40.78 / 41.08 | 0.400 | 0.055, -0.36 |
| D_s8_stitch_1p0 | fine | 616 k | 91 s | 0.050 / 0.021 | 0.027 | 0.032 | +0.24 | 44.2 / 44.14 | 0.174 | 0.049, -0.12 |
| D_s8_stitch_2p0 | fine | 620 k | 91 s | 0.099 / 0.036 | 0.059 | 0.029 | +0.26 | 44.54 / 44.42 | 0.250 | 0.059, -0.06 |
| D_s8_stitch_5p0 | fine | 616 k | 92 s | 0.090 / 0.050 | 0.067 | 0.030 | +0.22 | 45.36 / 45.33 | 0.254 | 0.092, -0.11 |
| D_s8_stitch_none | fine | 581 k | 92 s | 0.228 / 0.066 | 0.195 | 0.029 | +0.16 | 46.24 / 46.31 | 0.262 | 0.074, -0.11 |
| E_s8_L1L3_p1000_oval_g2 | fine | 404 k | 94 s | 0.334 / 0.130 | – | 0.253 | -1.40 | 28.84 / 28.47 | 0.617 | 0.972, -0.81 |
| E_s8_L1L3_p1000_round_g2 | fine | 404 k | 95 s | 0.408 / 0.170 | – | 0.258 | -1.42 | 27.67 / 27.38 | 0.730 | 1.080, -0.78 |
| E_s8_L1L3_p1200_round_g2 | fine | 418 k | 96 s | 0.417 / 0.172 | – | 0.261 | -1.43 | 27.88 / 27.62 | 0.723 | 1.473, -0.78 |
| E_s8_L1L3_p800_round_g2 | fine | 394 k | 93 s | 0.416 / 0.177 | – | 0.255 | -1.40 | 26.93 / 26.57 | 0.737 | 0.835, -0.74 |
| E_s8_L1L8_p1000_oval_g0 | fine | 344 k | 76 s | 0.038 / 0.016 | – | 0.030 | -0.01 | 43.13 / 43.25 | 0.262 | 0.022, -0.47 |
| E_s8_L1L8_p1000_oval_g2 | fine | 404 k | 90 s | 0.049 / 0.021 | – | 0.032 | -0.04 | 40.88 / 41.0 | 0.367 | 0.046, -0.55 |
| E_s8_L1L8_p1000_oval_g4 | fine | 432 k | 94 s | 0.046 / 0.019 | – | 0.029 | -0.10 | 40.77 / 40.75 | 0.399 | 0.047, -0.64 |
| E_s8_L1L8_p1000_round_g0 | fine | 344 k | 75 s | 0.047 / 0.022 | – | 0.034 | +0.03 | 41.67 / 41.94 | 0.326 | 0.034, -0.45 |
| E_s8_L1L8_p1000_round_g2 | fine | 404 k | 90 s | 0.063 / 0.031 | – | 0.033 | -0.02 | 39.56 / 39.84 | 0.440 | 0.060, -0.53 |
| E_s8_L1L8_p1000_round_g4 | fine | 432 k | 91 s | 0.058 / 0.032 | – | 0.034 | -0.05 | 39.4 / 39.59 | 0.452 | 0.063, -0.60 |
| E_s8_L1L8_p1200_round_g2 | fine | 418 k | 89 s | 0.054 / 0.027 | – | 0.033 | +0.00 | 40.84 / 41.14 | 0.391 | 0.046, -0.51 |
| E_s8_L1L8_p800_round_g2 | fine | 394 k | 88 s | 0.078 / 0.039 | – | 0.033 | -0.02 | 37.39 / 37.7 | 0.519 | 0.083, -0.55 |
| G_pp100 | fine | 111 k | 105 s | \|Z11\| peaks within 0.8 % (CST / kisi) | | | | | | \|Z11\| peaks within 0.8 % (CST / openEMS) |
| G_pp200 | fine | 85 k | 51 s | \|Z11\| peaks within 0.8 % (CST / kisi) | | | | | | \|Z11\| peaks within 0.8 % (CST / openEMS) |
| H_s8_cap_10 | fine | 196 k | 197 s | L(100 MHz) 1.694 / 1.685 nH (CST / kisi) | | | | | | L(100 MHz) 1.694 / 1.823 nH (CST / openEMS) |
| H_s8_cap_2 | fine | 491 k | 489 s | L(100 MHz) 1.417 / 1.344 nH (CST / kisi) | | | | | | L(100 MHz) 1.417 / 1.481 nH (CST / openEMS) |
| H_s8_cap_20 | fine | 155 k | 173 s | L(100 MHz) 2.099 / 2.101 nH (CST / kisi) | | | | | | L(100 MHz) 2.099 / 2.254 nH (CST / openEMS) |
| H_s8_cap_5 | fine | 290 k | 246 s | L(100 MHz) 1.493 / 1.475 nH (CST / kisi) | | | | | | L(100 MHz) 1.493 / 1.611 nH (CST / openEMS) |
| P_ts6_finger_diff | fine | 390 k | 118 s | 0.033 / 0.012 | – | 0.021 | -0.02 | 46.37 / 46.37 | 0.268 | 0.048, -0.06 |
| P_ts6_finger_far | fine | 325 k | 49 s | 0.146 / 0.061 | 0.078 | 0.032 | -0.18 | 45.08 / 45.45 | 0.767 | 0.080, -0.33 |
| P_ts6_finger_noplane | fine | 325 k | 54 s | 0.152 / 0.066 | 0.082 | 0.032 | -0.20 | 45.48 / 45.69 | 0.903 | 0.105, -0.33 |
| P_ts6_pad | fine | 212 k | 39 s | 0.143 / 0.048 | 0.124 | 0.065 | -0.32 | 40.58 / 39.91 | 0.023 | 0.098, -0.26 |
| P_ts6_pad_cut | fine | 223 k | 40 s | 0.114 / 0.050 | 0.113 | 0.082 | -0.12 | 44.84 / 45.55 | 0.175 | 0.070, -0.10 |
| RG_s4_pwr_n4 | fine | 716 k | 218 s | 0.543 / 0.101 | 0.701 | 0.062 | +0.22 | 44.3 / 45.55 | 1.507 | 2.414, -0.23 |
| RG_s4_pwr_n4_cap2 | fine | 738 k | 223 s | 1.228 / 0.134 | 1.408 | 0.085 | +0.24 | 44.3 / 45.55 | 2.110 | 2.468, -0.36 |
| RG_ts6_sw_gnd | fine | 772 k | 213 s | 0.240 / 0.094 | 0.176 | 0.225 | -1.40 | 40.31 / 42.61 | 0.055 | 0.507, -0.42 |
| RG_ts6_sw_pwr | fine | 793 k | 230 s | 0.192 / 0.094 | 0.118 | 0.227 | -1.37 | 40.34 / 42.63 | 0.046 | 0.132, -0.25 |
| R_s4_lshape_n0 | fine | 652 k | 190 s | 1.600 / 0.242 | 1.550 | 0.168 | +0.27 | 47.95 / 46.28 | 0.191 | 0.945, +0.00 |
| R_s4_rect_2v | fine | 721 k | 349 s | kisi-fdtd contains NaN | | | | | 0.951 | contains NaN |
| R_s4_rect_cap5 | fine | 646 k | 199 s | 0.918 / 0.197 | 0.852 | 0.164 | +0.22 | 47.95 / 46.28 | 0.404 | 0.918, -0.05 |
| R_s4_rect_n0 | fine | 625 k | 191 s | 1.051 / 0.189 | 1.014 | 0.162 | +0.20 | 47.95 / 46.28 | 0.155 | 1.057, -0.06 |
| R_s4_rect_n1 | fine | 656 k | 202 s | 0.769 / 0.162 | 0.709 | 0.169 | +0.24 | 47.97 / 46.26 | 0.214 | 1.005, -0.00 |
| R_s4_rect_n4 | fine | 698 k | 130 s | 0.093 / 0.019 | 0.093 | 0.037 | +0.26 | 43.73 / 44.77 | 0.275 | 0.090, +0.01 |
| T_ts6_broadside_o0 | fine | 388 k | 91 s | 0.101 / 0.017 | – | 0.053 | +0.01 | 40.55 / 38.11 | 0.063 | 0.890, -3.31 |
| T_ts6_broadside_o5 | fine | 416 k | 94 s | 0.051 / 0.014 | – | 0.041 | +0.03 | 40.46 / 38.57 | 0.011 | 0.052, -0.01 |
| T_ts6_diff_ms | fine | 390 k | 106 s | 0.067 / 0.023 | – | 0.043 | -0.10 | 54.98 / 56.06 | 0.269 | 0.091, -0.19 |
| T_ts6_diff_sl | fine | 383 k | 109 s | 0.133 / 0.073 | – | 0.102 | -0.03 | 51.34 / 45.57 | 0.083 | 0.129, -0.06 |
| T_ts6_ms_10 | fine | 223 k | 41 s | 0.014 / 0.005 | 0.004 | 0.014 | +0.00 | 43.85 / 44.3 | 0.011 | 0.002, -0.00 |
| T_ts6_ms_20 | fine | 383 k | 78 s | 0.016 / 0.006 | 0.006 | 0.014 | +0.01 | 41.7 / 42.29 | 0.023 | 0.007, +0.00 |
| T_ts6_sl_10 | fine | 219 k | 40 s | 0.066 / 0.013 | 0.038 | 0.109 | +0.02 | 43.18 / 40.55 | 0.078 | 0.031, +0.01 |
| T_ts6_sl_20 | fine | 376 k | 52 s | 0.101 / 0.015 | 0.071 | 0.102 | +0.12 | 44.21 / 39.25 | 0.113 | 0.043, +0.10 |
| T_ts6_xt_ms_s10 | fine | 410 k | 99 s | 0.012 / 0.004 | – | 0.023 | +0.11 | 41.3 / 40.29 | 0.038 | 0.017, +0.04 |
| T_ts6_xt_ms_s20 | fine | 417 k | 99 s | 0.019 / 0.003 | – | 0.026 | -0.01 | 42.7 / 41.51 | 0.020 | 0.021, -0.07 |
| T_vna6_cpwg_10 | fine | 539 k | 95 s | 0.020 / 0.005 | – | 0.030 | -0.03 | 44.06 / 44.23 | 0.051 | 0.033, -0.23 |
| T_vna6_cpwg_20 | fine | 988 k | 151 s | 0.032 / 0.005 | – | 0.026 | +0.50 | 42.35 / 43.46 | 0.097 | 0.029, +0.29 |
| X_s8x2_L1L3_bd_d200 | fine | 386 k | 72 s | 0.036 / 0.016 | 0.025 | 0.018 | -0.07 | 40.46 / 40.22 | 0.018 | 0.040, -0.26 |
| X_s8x4_L1L3_bd_d200 | fine | 386 k | 71 s | 0.025 / 0.012 | 0.032 | 0.047 | +0.02 | 43.09 / 43.35 | 0.014 | 0.027, -0.07 |
| thru_BV_ts_m2_per0_in | fine | 227 k | 52 s | 0.013 / 0.005 | – | 0.013 | -0.25 | 46.69 / 46.65 | 0.025 | 0.034, -0.30 |
| thru_BV_ts_m2_per0_out | fine | 214 k | 49 s | 0.013 / 0.007 | – | 0.038 | +0.09 | 42.95 / 42.21 | 0.055 | 0.010, +0.01 |
| thru_BV_ts_refinout2_in | fine | 492 k | 81 s | 0.106 / 0.057 | – | 0.041 | -0.20 | 47.09 / 46.91 | 0.018 | 0.085, -0.25 |
| thru_BV_ts_refinout2_out | fine | 226 k | 41 s | 0.053 / 0.026 | – | 0.031 | -0.35 | 46.79 / 46.67 | 0.044 | 0.042, -0.39 |
| thru_BV_vna_clk_w5_in | fine | 252 k | 41 s | 0.114 / 0.055 | – | 0.061 | -0.22 | 48.08 / 47.93 | 0.026 | 0.096, -0.36 |
| thru_BV_vna_clk_w5_out | fine | 216 k | 43 s | 0.265 / 0.137 | – | 0.119 | -0.25 | 47.73 / 47.18 | 0.272 | 0.251, -0.33 |
| thru_BV_vna_rf_hblb1_in | fine | 543 k | 84 s | 0.041 / 0.019 | – | 0.047 | -0.20 | 45.74 / 45.59 | 0.016 | 0.035, -0.50 |
| thru_BV_vna_rf_hblb1_out | fine | 503 k | 81 s | 0.007 / 0.004 | – | 0.015 | -0.19 | 43.89 / 43.86 | 0.004 | 0.026, -0.39 |
| thru_RG_ts6p_L1 | fine | 78 k | 18 s | 0.088 / 0.040 | – | 0.137 | -0.56 | 47.48 / 46.7 | 0.076 | 0.084, -0.45 |
| thru_RG_ts6p_L4 | fine | 74 k | 18 s | 0.022 / 0.006 | – | 0.009 | -0.12 | 44.01 / 44.04 | 0.016 | 0.009, -0.05 |
| thru_R_s4_L1 | fine | 76 k | 15 s | 0.010 / 0.002 | – | 0.017 | -0.20 | 46.49 / 46.2 | 0.013 | 0.031, -0.34 |
| thru_R_s4_L4 | fine | 76 k | 16 s | 0.010 / 0.002 | – | 0.017 | -0.20 | 46.49 / 46.2 | 0.013 | 0.006, -0.13 |
| thru_s4_L1 | fine | 212 k | 37 s | 0.009 / 0.004 | – | 0.017 | -0.16 | 44.84 / 44.49 | 0.006 | 0.027, -0.30 |
| thru_s4_L4 | fine | 212 k | 37 s | 0.009 / 0.004 | – | 0.017 | -0.16 | 44.84 / 44.49 | 0.006 | 0.027, -0.30 |
| thru_s8_L1 | fine | 236 k | 41 s | 0.013 / 0.007 | – | 0.056 | -0.09 | 45.5 / 45.11 | 0.006 | 0.021, -0.22 |
| thru_s8_L3 | fine | 231 k | 42 s | 0.044 / 0.013 | – | 0.032 | -0.06 | 42.81 / 44.02 | 0.058 | 0.026, -0.10 |
| thru_s8_L8 | fine | 236 k | 42 s | 0.013 / 0.007 | – | 0.056 | -0.09 | 45.5 / 45.11 | 0.010 | 0.019, -0.13 |
| thru_s8x2_L1 | fine | 239 k | 40 s | 0.071 / 0.027 | – | 0.028 | -0.26 | 44.66 / 44.21 | 0.008 | 0.169, -0.88 |
| thru_s8x2_L3 | fine | 224 k | 41 s | 0.044 / 0.014 | – | 0.009 | -0.06 | 39.96 / 39.66 | 0.018 | 0.012, -0.10 |
| thru_s8x4_L1 | fine | 220 k | 40 s | 0.013 / 0.007 | – | 0.056 | -0.09 | 45.5 / 45.11 | 0.010 | 0.021, -0.22 |
| thru_s8x4_L3 | fine | 229 k | 41 s | 0.042 / 0.016 | – | 0.014 | -0.12 | 42.45 / 41.97 | 0.013 | 0.028, -0.30 |
| thru_ts6_L1 | fine | 191 k | 39 s | 0.013 / 0.005 | – | 0.015 | -0.00 | 44.8 / 45.28 | 0.009 | 0.008, -0.05 |
| thru_ts6_L3 | fine | 188 k | 31 s | 0.090 / 0.020 | – | 0.111 | +0.04 | 44.3 / 41.75 | 0.070 | 0.059, +0.03 |
| thru_ts6_L6 | fine | 191 k | 38 s | 0.031 / 0.015 | – | 0.088 | -0.24 | 45.98 / 45.4 | 0.037 | 0.038, -0.28 |

## Files

* `cst_case.py`: case JSON → VBA history blocks (or one `.bas` macro: `python3 cst_case.py case.json out.bas`).
* `run_cst.py`: runs cases in CST (`--tag`, `--lpw`, `--lpb`, `--port face|edge`, `--skip-done`), writes
  `via_ref/<case>/cst[_tag].sNp` (header: CST version, solver, mesh cells, run time, mesh and port settings) and
  `cst[_tag]_meta.json` (status `not run` where the mesh exceeds the available mesh size).
* `cst_compare.py`: CST against kisi-fdtd and openEMS → `results/cst_compare.json`, `--md` for the table above.
* `via_ref/<case>/cst.sNp` (base mesh), `cst_fine.sNp` (fine), `cst_xfine.sNp` (extra-fine, five cases),
  `cst_mid.sNp` (intermediate, BV_vna_clk_w5), with their `*_meta.json`.
