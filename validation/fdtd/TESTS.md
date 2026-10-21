# kisi-fdtd and reduced models: test log

What was tested, how, and the result. Numbers come from the JSON files in `results/` and from
`validation/via_ref/*/meta.json`. `references.html` shows the same tables (tab "Solver checks") and every
case with its curves. The scripts named here reproduce each result.

## 1. Solver (kisi-fdtd)

| # | Test | Script | Result | Status |
|---|---|---|---|---|
| S1 | Metal model: TM010-type resonance of a cylindrical cavity, empty and with a centre post, analytic | `cavity.py` | staircase −1.75 / −0.91 / −0.47 / −0.27 % (empty, 0.2–0.025 mm), conformal −0.21 / −0.09 / −0.04 / −0.02 %; post: staircase −3.26 … −0.89 %, conformal −1.53 … −0.13 % | conformal 7–13× closer |
| S2 | Mesh convergence of a via with antipads (A_s4_d300_ap800), staircase and conformal, 0.1–0.025 mm | `via_convergence.py` | conformal at h ≈ staircase at 0.7·h; at 0.035 mm \|S21\| within 0.07 dB of the finest run up to 30 GHz | references use 0.035 mm |
| S3 | CPML on a matched microstrip: 8 / 16 cells, larger box, openEMS PML_8 | `check_pml.py` | S11 identical for 8 and 16 cells (converged), openEMS 8–14 dB more reflection above 10 GHz, S21 within 0.01 dB | pass |
| S4 | Implementation against openEMS on the same mesh (staircase, PML), one case each of A, B, D, E | `openems_compare.py` | Δ\|S21\| mean 0.02–0.22 dB; larger maxima (up to 2.5 dB) only in deep nulls (stub notch at −25 dB) | pass |
| S5 | Lumped series R–L–C at the end of a 1 mm microstrip | unit test `lumped_rlc` | series resonance 4.35–4.40 GHz against 4.33 GHz from the circuit with the line's 0.35 nH, Re Z 9.6 Ω against 10 Ω | pass |
| S6 | Stability: lone conformal pad in a PEC box, 12 000 steps | unit test `conformal_pad_stable` | energy decays (found and fixed: every face touching a partially free edge must use its free length, otherwise the operator is not symmetric and the energy grows) | pass |
| S7 | Earlier feasibility: kisi-fdtd against openEMS with Mur, staircase | `compare.py` | thru 0.005 / 0.02 dB, via 0.10 / 0.67 dB (mean / max ΔS21) | pass |

Unit tests: `cargo test -p kisi-fdtd` (microstrip, conformal pad stability, cavity resonance within 0.2 %, lumped
R–L–C).

### Mesh rules found on the way
- With conformal metal, circle extremes are soft mesh lines: they merge into a neighbour within 0.9 cells, so trace
  edges and via circles no longer create sliver cells (one 0.019 mm cell had halved the time step).
- A gap remainder of at least half a cell becomes a full cell and the gap is rescaled, so no sliver cell remains.
  The smallest cell is now ≥ 0.84 · dfine.
- GND stitching vias get their own coarser cell (0.1 mm, `dmesh`), which conformal metal resolves.

## 2. Reference cases (`via_cases.py`, `run_refs.py`)

68 cases on a 0.035 mm mesh (G: 0.1 mm, H: 0.07 mm), λ/20 at 40 GHz elsewhere, CPML, planes into the PML,
field monitors on port 1:

| Group | Cases | What they check |
|---|---|---|
| A | 6 | coax section L1→L4 through one plane pair, drill 0.2 / 0.3, antipad 0.6 / 0.8 / 1.0, 4 GND vias |
| B | 8 | stubs: through via to L8, no stub, back-drilled to 0.2 mm; 8-layer and the ThunderScope stack |
| C | 8 | antipad 0.6–1.2 mm, non-functional pads kept / removed, offset antipad, mixed antipads |
| D | 5 + 3 | one GND via at 0.5 / 1 / 2 / 5 mm or none; GND → PWR through a capacitor at 2 / 10 mm or none |
| E | 12 | pairs: pitch 0.8–1.2 mm, round / oval antipads, 0 / 2 / 4 GND vias, L1→L8 and L1→L3 |
| G | 2 | plane pair 50 × 30 mm, d = 0.1 / 0.2 mm, 1 Ω ports at a corner and the centre |
| H | 4 | BGA pin field to a 0402 capacitor at 2 / 5 / 10 / 20 mm, loop inductance |
| T | 12 | lines and coupling: microstrip, CPWG (LibreVNA RF class), stripline (10 / 20 mm), edge-coupled microstrip and stripline pairs, single-ended aggressor/victim, broadside In2/In3 |
| thru | 8 | 2×thru per feed layer, for de-embedding |

## 3. kisi's fast models against the 3D references (`kisi_board.py`, `compare_kisi.py`)

Every case becomes a mini KiCad board; kisi computes it through its normal pipeline (`kisi net`).

**Findings**
- **Bug found and fixed** (`kisi-si/src/rf/channel.rs`): kisi took the reference planes of a via from the trace
  piece right at the via. That piece runs over the antipad and has no reference, so the return path of every via
  whose traces end over an antipad was reported as "none".
  - It now uses the last referenced piece before the via and the next one after it.
  - In D (L1 ref L2 → L8 ref L7) kisi now reports "stitch" or "missing" and matches the TDR peak: 55.2 against
    55.0 Ω at 2 mm, 62.8 against 57.0 Ω without a stitching via.
  - Regression test: `via_return_path_across_antipad`.
- **Via impedance:** kisi's coax-section model, with the antipad as outer conductor, puts the via too low. In A
  (0.8 mm antipad) the TDR minimum is 37.9 Ω against 43.8 Ω. In thick plane pairs the field ends on the return
  vias and the planes, not on the antipad.
- **Stub resonance:** kisi gives 25 GHz against 21.4 GHz in 3D (+17 %, target ±10 %) and the notch is too shallow
  (−13 against −25 dB). Pads and antipads load the stub capacitively.
- **Return-path loss:** without a stitching via, 3D loses energy into the plane pairs (S21 −2.9 dB at 10 GHz).
  kisi's series inductance cannot represent that radiation, so it stays 2 dB too optimistic up to 20 GHz.

## 4. Reduced models (prototypes)

### Via model (`via_pbm.py`)
A physics-based ladder along the barrel:
- **Nodes:** at the planes and at the signal layers; thick plane pairs are split into sections of ≤ 0.2 mm.
- **Shunt elements:** via–plate capacitances from an axisymmetric r–z Laplace solve (checked against the disc
  capacitor: 367 fF against 358 fF analytic).
- **Series elements:** between the planes, the parallel-plate impedance ωμd/4 · H0⁽²⁾(ka) with the GND vias as
  shorted ports. Above the outermost planes, a short coax to the antipad.
- **Stubs:** the open rest of the ladder.

It is compared with the 3D result de-embedded by the 2×thru of each feed layer.

Over the single-via cases (A–D), up to 20 GHz:
- median phase error 5.6°, |S21| within 0.15 dB, |ΔS11| 0.08;
- TDR via impedance within 2–5 %;
- return-path peak in D within 1 %, and the loss into the plane pairs reproduced (S21 −2.43 against −2.25 dB at
  10 GHz without a stitching via);
- stub notch −5 … −9 % (kisi's present model: +17 %).

**Open points**
- Exits on inner layers (stripline inside a plane pair) are the weak point: phase 17–19° too large.
- Next step: a single plane pair with weighted segment currents instead of independent sub-cavities.
- Pairs (E) still need the two-via cavity matrix and the mutual capacitance.

### Plane-cavity model (`cavity_model.py`)
- Rectangular plane pair with magnetic walls (double cosine series), same dielectric loss as in 3D.
- The resonances of G match within 0.8 % (one step of the 10 MHz frequency grid).
- |Z21| below 1 GHz agrees within 0.7 %; |Z11| is 7 % low (the effective size of the one-cell port).

### Not yet modelled
- H (PDN loop): the plane-pair loop model with all PWR and GND vias as ports.
- T: the coupling against kisi's crosstalk estimate; done by `compare_kisi.py` once the T results are in.
