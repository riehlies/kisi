# kisi-fdtd and reduced models: test log

What was tested, how, and the result. Numbers come from the JSON files in `results/` and from
`validation/via_ref/*/meta.json`. `references.html` shows the same tables (tab "Solver checks") and every
case with its curves. The scripts named here reproduce each result.

## 1. Solver (kisi-fdtd)

| # | Test | Script | Result | Status |
|---|---|---|---|---|
| S1 | Metal model: TM010-type resonance of a cylindrical cavity, empty and with a centre post, analytic | `cavity.py` | staircase −1.75 / −0.91 / −0.47 / −0.27 % (empty, 0.2–0.025 mm), conformal −0.21 / −0.09 / −0.04 / −0.02 %; post: staircase −3.26 … −0.89 %, conformal −1.53 … −0.13 % | conformal 7–13× closer |
| S2 | Mesh convergence of a via with antipads (A_s4_d300_ap800), staircase and conformal, 0.1–0.025 mm | `via_convergence.py` | conformal at h ≈ staircase at 0.7·h; at 0.035 mm \|S21\| within 0.07 dB of the finest run up to 30 GHz (run with the mesh rules before soft circle lines and coarse GND-via cells) | references use 0.035 mm |
| S3 | CPML on a matched microstrip: 8 / 16 cells, larger box, openEMS PML_8 | `check_pml.py` | S11 identical for 8 and 16 cells (converged), openEMS 8–14 dB more reflection above 10 GHz, S21 within 0.01 dB | pass |
| S4 | Implementation against openEMS on the same mesh (staircase, PML), one case each of A, B, D, E | `openems_compare.py` | Δ\|S21\| mean 0.02–0.22 dB; larger maxima (up to 2.5 dB) only in deep nulls (stub notch at −25 dB) | pass |
| S5 | Lumped series R–L–C at the end of a 1 mm microstrip | unit test `lumped_rlc` | series resonance 4.35–4.40 GHz against 4.33 GHz from the circuit with the line's 0.35 nH, Re Z 9.6 Ω against 10 Ω | pass |
| S6 | Stability: lone conformal pad in a PEC box, 12 000 steps | unit test `conformal_pad_stable` | energy decays (found and fixed: every face touching a partially free edge must use its free length, otherwise the operator is not symmetric and the energy grows) | pass |
| S8 | Line impedance and the edge effect of zero-thickness strips: Zc and εeff of 20 mm microstrip / stripline (ThunderScope stack; γ from 10 / 20 mm) against kisi's 2D field solver (≤ 1.7 % from HFSS) | `compare_kisi.py … kisi.mjs` | mesh lines on the strip edges: Zc 6 % low (46.1 / 43.3 Ω against 49.1 / 46.1 Ω), εeff within 1 %; thirds rule (lines Δ/3 inside, 2Δ/3 outside the long edges): Zc 48.6 / 46.5 Ω (−1.0 / +0.8 %), εeff 2.580 / 3.920 (+0.5 / +0.4 %) | thirds rule on for all cases with traces; references recomputed |
| S7 | Earlier feasibility: kisi-fdtd against openEMS with Mur, staircase | `compare.py` | thru 0.005 / 0.02 dB, via 0.10 / 0.67 dB (mean / max ΔS21) | pass |

Unit tests: `cargo test -p kisi-fdtd` (microstrip, conformal pad stability, cavity resonance within 0.2 %, lumped
R–L–C).

### Mesh rules found on the way
- Thirds rule at the long edges of traces (`"thirds": true`): with the mesh line on the edge, the staircased
  zero-thickness strip acts wider and Zc comes out 6 % low.
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
  - In D (L1 ref L2 → L8 ref L7) kisi now reports "stitch" or "missing" and follows the TDR peak:
    - 54.8 against 54.5 Ω at 2 mm;
    - 62.1 against 56.8 Ω beyond 5 mm (kisi caps the return distance at 5 mm and models it as an inductance).
  - Regression test: `via_return_path_across_antipad`.
- **Via impedance:** kisi's coax-section model, with the antipad as outer conductor, puts the via too low. The TDR
  extremes (35 ps, with the feeds) deviate by a median of 4.4 %, at most 20 %. In thick plane pairs the field ends
  on the return vias and the planes, not on the antipad.
- **Stub resonance:** kisi's quarter-wave estimate is 28.0 GHz against the 3D notch at 21.4–21.7 GHz (+30 %) for
  the 8-layer stack, and 31.4 against 26.3 GHz (+19 %) for the ThunderScope stack; target ±10 %. Below 25 GHz,
  where kisi's S-parameters end, its S21 shows no notch at all, while 3D has −25 dB. Pads and antipads load the
  stub capacitively.
- **Return-path loss:** without a stitching via, 3D loses energy into the plane pairs (S21 −2.9 dB at 10 GHz).
  kisi's series inductance cannot represent that radiation, so it stays 2 dB too optimistic up to 20 GHz.
- **Differential vias (E):**
  - kisi puts the pair via too high: TDR minimum Zdiff 87.9–94.6 against 79.8–88.8 Ω for L1→L8 (+4 … +11 %).
  - With the stub (L1→L3) it gives 73–77 against 63–67 Ω (+15 … +18 %), because the pair model has no stub.
  - It does not distinguish round from oval antipads or 0 / 2 / 4 GND vias (3D: 82.6 Ω with 4 GND vias, 86.5 Ω
    with none).
- **Lines (T, 20 mm, 3D with the thirds rule, Zc from the ABCD matrix, εeff from 10 / 20 mm, 5 GHz):**

  | Line | 3D | kisi field solver (kisi.mjs) | kisi formula (native) |
  |---|---|---|---|
  | microstrip L1 (ThunderScope) | 48.6 Ω, εeff 2.580 | 49.1 Ω, 2.568 | 49.2 Ω, 2.554 |
  | stripline In2 (ThunderScope) | 46.5 Ω, 3.920 | 46.1 Ω, 3.904 | 51.6 Ω (+11 %), 3.725 (−5 %) |
  | CPWG (LibreVNA RF class) | 51.1 Ω, 3.116 | 50.3 Ω, 3.084 | 51.1 Ω, 3.264 (+5 %) |

  - The field solver is within 1.6 % in Zc and 1 % in εeff.
  - The formulas miss the asymmetric stripline by 11 % and the CPWG εeff by 5 %. The formula fallback is marked
    as an estimate in kisi.
- **Coupling (T, microstrip aggressor/victim, 20 mm, 35 ps):**
  - FEXT: 3D 6.64 / 3.26 %, kisi formula 7.01 / 3.48 % (+6 %), field solver 7.28 / 3.61 % (+10 %).
  - NEXT: 3D 1.41 / 0.58 %, kisi 2.36–2.59 / 0.86–1.06 %, i.e. 70–80 % high.
  - Next step: the saturated-NEXT definition against the 3D near-end peak.
- **Broadside In2/In3** (0.93 mm core): 3D NEXT 1.1 / 0.7 %, FEXT 1.3 / 0.9 % (0 / 0.5 mm offset). kisi detects
  it but does not compute it yet; these are the references for that.
- **Differential lines:** the TDR minimum includes the lumped-port transition. The ms pair is 100.7 Ω in 3D
  against 103.6 Ω in kisi; for the stripline pair (88.4 against 101 Ω) the port dip dominates, so it needs a
  plateau value (open).

## 4. Reduced models (prototypes)

### Via model (`via_pbm.py`)
A physics-based ladder along the barrel:
- **Nodes:** at the planes and at the signal layers; thick plane pairs are split into sections of ≤ 0.2 mm.
- **Shunt elements:** via–plate capacitances from an axisymmetric r–z Laplace solve (checked against a disc over a
  plane: 367 fF against 358 fF analytic).
- **Series elements:** between the planes, the parallel-plate impedance ωμd/4 · H0⁽²⁾(ka) with the GND vias as
  shorted ports. Above the outermost planes, a short coax to the antipad.
- **Trace joining inside a plane pair:** the two parts of that cavity are weighted by their height,
  Z(d)·(d_g/d)².
- **Stubs:** the open rest of the ladder.
- **Pairs:** the 2 × 2 cavity matrix (returns shorted), odd mode Z_PP − Z_PN.
- **Feed line removal:** the de-embedding fixture runs to the via centre, so the model subtracts the feed over the
  pad or the antipad opening (stripline feeds with the offset-stripline formula).

It is compared with the 3D result de-embedded by the 2×thru of each feed layer (pairs: first-order shift).

Results up to 20 GHz (`results/via_pbm.json`):
- **Single vias (A–D, 27 cases):** |S21| median deviation 0.14 dB. TDR via impedance (25 ps) median 1.4 %,
  90th percentile 7.5 %; the return-path peak in D is within 1 %. kisi's present model (35 ps, with the feeds):
  median 4.0 %, 90th percentile 16 %.
- **Return-path loss:** the loss into the plane pairs is reproduced (D without a stitching via: S21 −2.43 against
  −2.25 dB at 10 GHz).
- **Stub notch:** +0.5 / −0.9 % for the 8-layer stack, −5.7 % for the ThunderScope stack. kisi's present model:
  +19 … +30 %.
- **Pairs L1→L8 (8 cases):** Sdd21 within 0.12–0.16 dB, phase within 3.9–5.9°.

**Open points**
- **Exits on inner layers** (stripline inside a plane pair: B L1→L3, E L1→L3): the magnitude is good for single
  vias (|ΔS21| 0.2–0.4 dB), pairs 0.7–1.3 dB. About 3 ps of delay are missing (20–23° at 20 GHz).
- **Pairs:** the mutual capacitance of the two barrels is included only where the pitch is below the field radius
  (not the case in these stacks).

### Plane-cavity model (`cavity_model.py`)
- Rectangular plane pair with magnetic walls (double cosine series), same dielectric loss as in 3D.
- **G:** resonances within 0.8 % (one step of the 10 MHz frequency grid). |Z21| below 1 GHz within 0.7 %, |Z11|
  7 % low (the effective size of the one-cell port).
- **H (`pdn_loop.py`):** the change of the loop inductance with the capacitor distance, from the same series with
  a port at the pin field and the capacitor vias shorted:
  - Model against 3D, from 2 mm to 5 / 10 / 20 mm: 0.128 against 0.128 nH, 0.339 against 0.335 nH, 0.758 against
    0.743 nH.
  - The radial formula μ0·d_eff/π·ln(D2/D1) gives only 0.05 / 0.10 / 0.16 nH, because the 7 mm wide board makes the
    current run as in a strip.
  - The total loop inductance in 3D at 50 MHz: 1.33 / 1.46 / 1.66 / 2.07 nH at 2 / 5 / 10 / 20 mm.
  - The vertical parts (pins, vias, capacitor body) are equal for every distance and are not modelled yet.
- **H setup error, found and fixed:** the first H runs had the planes reaching into the PML. The infinite PWR/GND
  plane pair was itself a ~0.2 Ω path at 100 MHz and bypassed the capacitor (same impedance with and without it).
  H now uses a finite board.

### Field view
Every case has the electric field of port 1 (`fields.json`).
- **Top view:** in the thickest dielectric the via crosses (lines: between the trace and its reference).
  Animated, so the phase runs.
- **Section:** through port 1.
- **What it shows:**
  - D without a stitching via: the radial wave in the plane pair, at 20 GHz rings with λ/2 ≈ 3.6 mm.
  - G: the standing cavity modes, e.g. TM01 at 2.41 GHz, positive in one half of the board and negative in the
    other.
