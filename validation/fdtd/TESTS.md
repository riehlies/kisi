# kisi-fdtd and reduced models: test log

What was tested, how, and the result. Numbers come from the JSON files in `results/` and from
`validation/via_ref/*/meta.json`. The reference page (`python3 make_viewer.py` → `dist/kisi-references.html`,
attached to the releases in which the reference cases changed) shows the same tables (tab "Solver checks") and every case with its curves.
The scripts named here reproduce each result.

## 1. Solver (kisi-fdtd)

| # | Test | Script | Result | Status |
|---|---|---|---|---|
| S1 | Metal model: TM010-type resonance of a cylindrical cavity, empty and with a centre post, analytic | `cavity.py` | staircase −1.75 / −0.91 / −0.47 / −0.27 % (empty, 0.2–0.025 mm), conformal −0.21 / −0.09 / −0.04 / −0.02 %; post: staircase −3.26 … −0.89 %, conformal −1.53 … −0.13 % | conformal 7–13× closer |
| S2 | Mesh convergence of a via with antipads (A_s4_d300_ap800), staircase and conformal, 0.1–0.025 mm | `via_convergence.py` | conformal at h ≈ staircase at 0.7·h; at 0.035 mm \|S21\| within 0.07 dB of the finest run up to 30 GHz (run with the mesh rules before soft circle lines and coarse GND-via cells) | references use 0.035 mm |
| S3 | CPML on a matched microstrip: 8 / 16 cells, larger box, openEMS PML_8 | `check_pml.py` | S11 identical for 8 and 16 cells (converged), openEMS 8–14 dB more reflection above 10 GHz, S21 within 0.01 dB | pass |
| S4 | Implementation against openEMS on the same mesh (staircase, PML), one case each of A, B, D, E | `openems_compare.py` | on the mesh of the reference runs: Δ\|S21\| mean 0.011–0.028 dB, max 0.04–0.22 dB (before, on a staircase-mode mesh with lines on every circle extreme: 0.02–0.22 / up to 2.5 dB) | pass |
| S15 | Every reference case (106) in openEMS, on kisi-fdtd's mesh and with the cell at metal halved | `openems_refs.py` | section 16 | pass, with the outliers explained there |
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

74 cases on a 0.035 mm mesh (G: 0.1 mm, H: 0.07 mm), λ/20 at 40 GHz elsewhere, CPML, planes into the PML,
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
| X | 2 | L1→L3 as in B without the L2 or the L4 plane (junction diagnosis, section 7) |
| thru | 12 | 2×thru per feed layer, for de-embedding |

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
The via findings below describe kisi's former coax-section via model; kisi now uses the ladder of section 5.
- **Via impedance (former coax model):** with the antipad as outer conductor, it puts the via too low. In thick
  plane pairs the field ends on the return vias and the planes, not on the antipad. Via alone (de-embedded, see
  section 5): TDR error median 8.5 %, 90th percentile 26 %.
- **Stub resonance (former coax model):** the quarter-wave estimate is 28.0 GHz against the 3D notch at
  21.4–21.7 GHz (+30 %) for the 8-layer stack, and 31.4 against 26.3 GHz (+19 %) for the ThunderScope stack; target
  ±10 %. Below 25 GHz, where kisi's S-parameters end, its S21 shows no notch at all, while 3D has −25 dB. Pads and
  antipads load the stub capacitively.
- **Return-path loss (former coax model):** without a stitching via, 3D loses energy into the plane pairs (S21
  −2.9 dB at 10 GHz). A series inductance cannot represent that radiation, so it stays 2 dB too optimistic up to
  20 GHz.
- **Full-channel TDR includes the ports:** the 3D lumped ports add a low-frequency offset, so a 49 Ω line reads
  43–45 Ω after 100 ps (T cases). The full-channel TDR minimum is therefore no via metric; vias are compared
  de-embedded (section 5).
- **Differential vias (E, former coax model):**
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

## 4. Reduced models (Python prototypes)

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
  90th percentile 7.5 %; the return-path peak in D is within 1 %.
- **Return-path loss:** the loss into the plane pairs is reproduced (D without a stitching via: S21 −2.43 against
  −2.25 dB at 10 GHz).
- **Stub notch:** +0.5 / −0.9 % for the 8-layer stack, −5.7 % for the ThunderScope stack (former coax model:
  +19 … +30 %).
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

## 5. Rust port of the reduced models, and the ladder in kisi

Rust modules:
- **`kisi_si::via_pbm`:** the via ladder.
- **`kisi_pi::cavity_modal`:** the plane cavity and the PDN loop inductance.
- **`kisi-validate models`:** runs them on the reference cases, with the same geometry reading as the prototype.
  `rust_models.py` compares them with Python and with 3D (`results/rust_models.json`).

### What changed against the prototype
- **r–z Laplace solve:**
  - **Sparse Cholesky** (`kisi-sparse`, faer) instead of scipy.
  - **Graded grid** as the default: 5 µm at the conductor edges, growing by 1.3 up to 0.1 mm radially and 0.05 mm
    axially. That is about 10 000 unknowns instead of 76 000.
  - **Charge assignment:** a grid row whose cell straddles the border between two ladder nodes is split by overlap.
  - **Cache:** the result is cached per via geometry, with antipads rounded to 0.01 mm, so a board solves once per
    via type.
- **Plane-pair impedance:**
  - Computed on a 0.25 GHz grid and interpolated linearly in Z/f.
  - The Hankel matrices are symmetric.
  - No `hypot` in the inner loops: in WebAssembly, `hypot` and its fused multiply-adds run in software, and the
    first build was 8× slower than native.
- **Bug fixed – odd-mode capacitance scaling of pairs:**
  - The prototype divided the barrel charge by the spread of the rows near a node. Around a plane node the rows
    fall into two groups, so it overstated the length.
  - With thin dielectrics (ThunderScope, 0.12–0.14 mm) the node capacitances grew up to ×10, and S21 of a pair via
    dropped by 15 dB at 12 GHz.
  - The port sums the rows' own cell heights instead. The reference cases do not change (their dielectrics are
    thicker).
- **Feed-line interface:** the feed is given as Zc and εeff with the length to remove, plus a switch for the
  trace-over-antipad line. That way kisi passes its own line models.

### Parity, accuracy and run time (`results/rust_models.json`)

Section 7 updates the "kisi" row: kisi's ladder now has the junction model and the equivalent antipad radius,
so the Rust graded-grid model no longer equals the prototype; parity is checked on the prototype model.


| | TDR error median / p90 (single vias) | max Δ\|S21\| median single / pairs (dB) | parity: max \|ΔS21\| against Python | time per via (400 frequencies) |
|---|---|---|---|---|
| Python prototype | 1.46 / 7.6 % | 0.144 / 0.156 | – | 2.6 s |
| Rust, uniform grid (as the prototype) | 1.46 / 7.6 % | 0.144 / 0.157 | 0.005 (linear), 0.005 dB | 240 ms + 1 ms |
| Rust, graded grid (kisi) | 1.47 / 7.4 % | 0.142 / 0.154 | 0.018 (linear), 0.2 dB | 17 ms + 1 ms |

- **Evaluation:** about 2.4 µs per frequency after the build.
- **Stub notch:** identical to Python in all 11 cases with a notch.
- **Uniform-grid residual:** it comes from the Z/f interpolation.
- **Cavity model:** Rust within 2·10⁻⁶ of Python; 600 frequencies × 9 600 modes in 52–69 ms (Python 200–244 ms).
- **PDN loop inductance:** identical to 0.001 nH, about 1 ms per configuration.

### kisi's channel with the ladder (`compare_kisi.py`, `results/kisi_models*.json`)

`kisi_si::via` builds the ladder geometry from the board:
- **Planes:** zone fills of plane nets with the hole around the via; the antipad is the distance to the nearest fill
  edge.
- **Annular rings:** on every layer of the barrel (KiCad default), or only on the connected layers with
  `remove_unused_layers` (now parsed, with `keep_end_layers`).
- **Return vias:** plane-net vias within 3 mm, the nearest 8.
- **Partner via:** for pairs.
- **Back-drilling** and the residual stub option.
- **Feeds:** the line models of the referenced traces on both sides. The unreferenced pieces over the antipad
  (≤ 1.5 mm) leave the cascade; the ladder models them.
- **Fallback:** without a plane, or with `KISI_VIA_MODEL=coax` (validation only), the coax-section model.

The via alone, de-embedded (kisi with kisi's own 2×thru, 3D with the 3D 2×thru, TDR 25 ps, up to 25 GHz):

| kisi via model | TDR error median / p90 | max Δ\|S21\| median | stub notch (8-layer, 2 cases) |
|---|---|---|---|
| former coax sections | 8.5 / 26 % | 0.28 dB | no notch below 25 GHz (λ/4 estimate +30 %) |
| ladder (native, formula impedances) | 2.4 / 10 % | 0.17 dB | −0.9 / −2.3 % |
| ladder (kisi.mjs, field solver) | 2.4 / 10 % | 0.17 dB | −0.9 / −2.3 % |

Pairs (E, full channel with the ports):
- **L1→L3:** max ΔSdd21 0.7–1.4 dB, against 1.7–4.9 dB with the coax model. Zdiff minimum within 1.2–3.8 %
  (coax 5–11 %).
- **L1→L8:** Sdd21 unchanged at 0.56 dB, dominated by the feeds. Zdiff minimum within 1.5–5.8 % (coax 0.2–4.1 %).
  The full-channel minimum includes the port offset.

Other changes in the output:
- **Via impedance:** √(ΣL/ΣC) of the ladder between entry and exit.
- **Stub resonance:** the first transmission dip of the ladder, searched up to 1.2 × the quarter-wave estimate.
- **Model per via:** `"model": "ladder" | "coax"`.
- **Warning:** "sections without reference plane" no longer appears for the pieces the ladder covers.

## 6. Practical cost on the example boards, and which solver where

`practice.py` → `results/practice.json`.

Measured in this workspace (2 cores, Xeon 2.8 GHz), best of 2 runs, with the ladder of section 7 (junction model
and equivalent antipad radii); an earlier run on a busier machine gave about 1.5 × these times.

| | ThunderScope Rev5.3 | LibreVNA |
|---|---|---|
| board file / vias / filled zones | 10.2 MB / 3741 / 824 | 4.6 MB / 2932 / 157 |
| `overview` (native) | 0.46 s, 63 MB | 0.18 s, 39 MB |
| `check --all` native, coax → ladder | 1.85 → 3.45 s | 0.68 → 0.97 s |
| ladder vias / Laplace solves (shared per via type) | 126 / 61, 0.71 s in total (12 ms each, ~11 000 unknowns); antipad equivalent radii 0.7 s | 21 / 18, 0.18 s |
| `net` with vias (native) | 0.54 → 0.53 s | 0.21 → 0.20 s |
| `check --all` Node (WASM, formulas), coax → ladder | 3.5 → 5.3 s, 229 MB | 1.4 → 2.0 s, 175 MB |
| `check --all` with field solver, empty cache (120 s budget) | 127 s: 62 of 923 new cross-sections, 91 % of the target length; all of them ≈ 30 min | 123 s: 209 of 836, 89 %; all ≈ 8 min |
| the same from the cache | 6.1 s (cache 33 kB) | 2.4 s (cache 54 kB) |
| channel verdicts changed by the ladder | none of 21 lanes | none of 30 lanes |

- **Spread of the metrics** over the lanes that do not reflect almost everything:
  - ThunderScope: RL −0.8 … 0 dB, IL at Nyquist within 0.01 dB.
  - LibreVNA: RL −0.2 … +5.1 dB, eye height ±0.004.
- **Reflective lanes left out:** 8 ThunderScope lanes have RL < 3 dB in both models. These are the M.2/PCIe lanes
  through 0 Ω links. There IL moves by several dB with any small change, because the standing waves shift.
- **ThunderScope M.2 vias:** on a via type with GND pour on the signal layers and the partner at 0.53 mm, the
  ladder shows a dip near 16 GHz; that needs a 3D check (ROADMAP).
- **Bundles:**
  - `kisi.html` 2275 → 2325 kB, `kisi.mjs` 1726 → 1775 kB (+50 kB for the ladder and the cavity model).
  - The WASM module is 3.4 MB before compression. The sparse solver was already in it for the PDN.
- **WebAssembly speed:** `hypot` and its fused multiply-adds run in software there, so the first build spent 70 %
  of its time in them. The inner loops avoid them now.
- **3D FDTD (one thread per run):**

  | Group | Cells (median) | Time per case (median / max) |
  |---|---|---|
  | via transition, A–E | 0.5–0.9 M | 2.6–4.5 min, at most 10 min |
  | lines T | 0.66 M | 3.9 min |
  | 2×thru | 0.37 M | 40 s |
  | plane cavity G | 0.32 M | 23 min (long ringing) |
  | PDN loop H | 0.59 M | 27 min |

  - Throughput is 25–45 M cell updates/s; memory 77 bytes per cell (thru: 27 MB).
  - All 68 references took 5.8 CPU hours. Stored data: 21 MB in `validation/via_ref`, of which 9 MB are field
    planes.
  - **Estimate for a whole channel:** a 30 mm lane with two vias at the same mesh is ≈ 3–5 M cells, ½–1 h.
  - **Estimate for a whole board:** the mesh lines of every trace give ~10⁹–10¹⁰ cells (≈ 0.1–1 TB). That is out
    of reach.


### Which solver where

| Solver | Cost | Accuracy (against 3D / HFSS) | Use it for |
|---|---|---|---|
| Formulas (native and WASM, always on) | µs per cross-section, a whole board in seconds | microstrip within 1–2 %; asymmetric stripline +11 %, CPWG εeff +5 % (marked as estimates) | screening every net, power and DC, quick what-ifs |
| 2D field solver (kisi.mjs, viewer) | about 1–2 s per new cross-section; cached on disk / in IndexedDB | Zc within 1.6 %, εeff within 1 % | sign-off of impedance-controlled nets, striplines, CPWG, coupled pairs |
| Via ladder (`via_pbm`, default for every via that crosses a plane) | ≈ 12 ms per via type (Laplace and equivalent antipad radius, cached), µs per frequency | via alone in kisi's channel: TDR median 1.4 %, p90 7.4 %; pairs: phase 3.5°; stub notch within 3 % | every channel with vias: return vias, antipads (also offset and oval), stubs, back-drilling, pairs, inner-layer exits |
| Cavity model (`cavity_modal`) | ms for a whole sweep | resonances within 0.8 %, transfer impedance within 0.7 % | rectangular plane pairs: resonances, spreading and loop inductance; kisi's PDN grid model stays the tool for real plane shapes |
| 3D FDTD (`kisi-fdtd`) | minutes per via transition (0.3–0.9 M cells), half an hour for a plane cavity; a whole board is out of reach | reference (CPML, conformal metal, checked against analytic cavities and, on every case, openEMS: section 16) | checking the reduced models, structures they do not cover (connector launches, non-round antipads, via fences, plane gaps, via-to-via coupling), frequencies above 25–40 GHz |

## 7. Via ladder: junction, non-round antipads, conductor loss (Block 1)

### Diagnosis of the inner-layer exits
- **Symptom:** the ladder was 2–3.5 ps too fast on L1→L3 (B s8, 20–27° at 20 GHz) and reflected 2–3× too
  much at 10–20 GHz.
- **Where it comes from:** comparing lumped L and C of model and 3D (de-embedded, 3–10 GHz) shows the model
  missing series inductance on *every* via:
  - about 50–100 pH on through vias;
  - 120–220 pH on inner exits;
  - while its capacitance agreed outside the inner exits.
- **Two extra 3D cases (group X)** separate the junction from the shared-cavity effect: L1→L3 without the L2
  plane (the exit above the top plane, no cavity) and without the L4 plane (the exit inside the thick L2–L5
  pair). Both still missed ~110–130 pH, so the cavity treatment was not the cause.
- **Variants tried** against all single vias (`kisi-validate via-geom`, experimental fields since removed):
  - The partial-height evanescent modes of the barrel add only ~6 pH.
  - Feeding the trace over the antipad from the barrel instead of the pad overshoots.
  - A series inductance at each port of ≈ μ0/(2π)·pad radius (50–60 pH) brought the through vias and both X
    cases to within ±0.4 ps.
  - What was left on inner exits came from the feed removal: a stripline between two planes is barely changed by
    the openings, and removing it up to the antipad edge cost 0.35 mm of line.

### The junction model (kisi's default; `ViaGeom::junction`)
- **Junction inductance:** μ0/(2π)·pad radius in series where a trace enters the via. This value is *fitted*
  to the 3D references; physically it is where the trace current turns into the pad and the barrel.
- **Trace over the antipad:** the feed's impedance, scaled by the microstrip ratio to the height
  h_ref + (antipad − pad) (the return runs around the edge of the opening), εeff of the feed. Before, it was a
  microstrip over the next plane beyond the reference, which was far too inductive (A, antipad Ø1.0: +170 pH).
- **Striplines (planes above and below):** the line is kept up to the via centre; nothing is removed.
- **The Python prototype stays available** (`junction: false`). `kisi-validate models` runs it on the uniform
  grid for the parity check.

### Non-round antipads
- **Equivalent radius:** the round opening with the same in-plane capacitance
  (`via_pbm::equivalent_radius`):
  - a 2D Laplace solve between the barrel and the opening in log-polar coordinates (Shortley–Weller at the edge);
  - the opening is taken from 96 rays: case geometry, or the fill edges of the plane on the board;
  - within 1 % of the eccentric coax;
  - cached; nearly round polygons use the log mean.
- **Results:**
  - C_s8_offset: phase 11° → 2°, max Δ|S21| 0.14 → 0.06 dB.
  - Oval antipads of the L1→L8 pairs: max ΔSdd21 0.11–0.13 → 0.06–0.07 dB.
  - L1→L3 oval pair: 1.22 → 0.73 dB.

### Conductor loss
- Skin-effect resistance and internal inductance of the barrel, and plates with surface impedance in the
  radial line (k² (1 + 2Rs/(ωμd) − j·2Rs/(ωμd))).
- kisi passes the copper conductivity; the 3D references use lossless metal, so their comparisons leave it off.
- The effect is negligible: 0.002 dB at 10 GHz, notch depth 0.2 dB.

### Results (`results/rust_models.json`, `results/kisi_models*.json`)

| Ladder against the de-embedded 3D result | before Block 1 | now |
|---|---|---|
| single vias (29 incl. X): TDR error median / p90 | 1.5 / 11.3 % (prototype) | 1.3 / 9.3 % |
| single vias: max Δ\|S21\| median | 0.16 dB | 0.09 dB |
| inner exits B L1→L3 s8: phase at 20 GHz | 16–27° | 3–13° |
| pairs (12, not used for the fit): max ΔSdd21 median | 0.16 dB | 0.11 dB |
| pairs L1→L3: phase | 22–23° | 3–6° |
| pairs L1→L8: phase | 4–6° | 1–3° |

| kisi channel, via alone (kisi's own 2×thru vs 3D 2×thru) | coax (former) | ladder before Block 1 | ladder now |
|---|---|---|---|
| single vias: TDR error median / p90 | 8.0 / 25.5 % | 2.4 / 10.1 % (27 cases) | 1.4 / 7.4 % (field solver: 1.3 / 5.6 %) |
| pairs: Zdiff error median / p90 | 17.8 / 29.1 % | – | 5.9 / 17.9 % |
| pairs: max ΔSdd21 median / phase | 0.39 dB / 53° | – | 0.16 dB / 3.5° |

Remaining:
- **TDR maxima of long stubs** (B L1→L3 stub: 11–16 %) and of the X cases with the 0.93 mm wide microstrip
  (10 %).
- **The ThunderScope through via** (L1→L6, thick core) misses ~50 fF.
- **The fit was made on the same single vias it is scored on.** The pairs and the board pipeline (kisi channel)
  were not used for the fit and improved as well.

## 8. Network assembly (M2): exactness, and pairs as 4-ports in the channel check

Not a new model of lines, vias or parts: the same elements assembled as a network of N-ports by modified nodal
analysis (`kisi_si::rf::network`, `kisi_si::lanenet`), with the time-domain step from `rf::timedomain`.

| Check (unit tests in `kisi-si`) | Result |
|---|---|
| a cascade of lines, series and shunt parts as one chain per element | equal to the ABCD cascade, 1e-11 relative |
| an open branch as a chain to its own node; a branch loaded at its end | equal to the shunt C/A, (C + D·Y)/(A + B·Y) |
| a coupled pair as one 4-port chain; two lines as a 4-port; an S block | equal to `rf::mixed::s_of`, two 2-ports, the chain |
| a single line between the ends through the network | equal to `eye::transfer` to 1e-10, DC point as well |
| a receiver on a side stub (multi-drop) | equal to the cascade with the loaded stub, voltage at the tap |
| a symmetric pair lane (`pair_lane_as_four_port`) | Sdd21 = odd-mode S21 (0.000 dB), the same eye; with a 2 mm jog Scd21 −6.5 dB |

On the ThunderScope PCIe lanes, P aligned with itself gives P's odd-mode S21 exactly; P and N of a lane differ
(PET2 at 8 GHz: P −3.79 dB, N −2.97 dB in the odd mode), so the 4-port gives −3.12 dB. The 3D pair references
(T_ts6_diff_ms/sl, group E) are symmetric pairs, where the 4-port and the odd mode agree; their comparisons in
sections 3, 5 and 7 used the net's 4-port (`rf::mixed`), which is unchanged. **Open:** a 3D reference of an
asymmetric pair (a jog, a stub on one line) to check the mode conversion.

## 9. Board vias in 3D (Block 2)

Full report: **`BOARD_VIAS.md`**; `board_vias.py` reproduces everything (cases `board_cases/`, results
`../via_ref/BV_*`, `results/board_vias*.json`).

### Solver
| # | Test | Where | Result |
|---|---|---|---|
| S9 | Polygon copper and ports across y-directed traces: the microstrip of `short_microstrip` turned by 90° with a polygon plane | unit test `polygon_plane_and_y_ports` | same mesh, S within 2·10⁻³ |
| S10 | No sliver cells: three soft refinement lines of the M.2 cut-out gave a 0.018 mm cell (0.005 mm in the whole case, time step 7× too small) | unit test `no_sliver_cells` | fails without the rule, passes with it; 74 reference meshes unchanged |
| S11 | Conformal edges only strictly inside the domain (the M.2 case and a coarse REFINOUT2 mesh grew without bound with partial edges in the PML and on its faces) | board cases, smoke test `trace_into_pml_stable` | stable; a reference via case (C_s8_offset) and the board 2×thru fixtures rerun bit-identical, the board cases within 0.07 dB |
| S12 | Time-step guard: Gershgorin bound of the operator next to conformal faces against the regular cells | `KISI_FDTD_DEBUG=1` | never triggered (74 references, 3 board cases) |

### De-embedding
| # | Test | Where | Result |
|---|---|---|---|
| S13 | 2×thru de-embedding with lines of Zc ≠ z0: a known π-section via between 30 pH ports and 4 mm lossy lines of 64/62, 53/51.5, 50/50 and 114/94 Ω | `board_vias.py deembed-check` | z0-referred NZC: C up to +82 fF, L up to −200 pH; `sparam.deembed_matched` (fixtures referred to their own line first): C within 5 fF, L within 4 pH, \|S21\| within 0.2 dB to 20 GHz |
| S14 | Block 1 single vias de-embedded both ways | `board_vias.py block1` | fixtures 49.4–50.7 Ω: C −1…−5 fF, L 0…+11 pH (B_s8 L1L3 stub / back-drill: C −8…−20 fF); 44–47 Ω fixtures change more (X_s8x2_L1L3_bd_d200: C 22 → 62 fF) |

### Board vias against kisi's ladder (matched de-embedding, ladder with the 3D loss model)
| | M.2 PCIe pair (Sdd) | ThunderScope through via | LibreVNA via with stub |
|---|---|---|---|
| max Δ\|S21\| to 16 / 20 GHz | 0.29 / 0.30 dB | 0.21 / 0.38 dB | 0.42 / 1.85 dB (notch shift; 0.06 dB to 6 GHz) |
| dip / notch, 3D / ladder | 16 / 17 GHz, −0.71 / −1.00 dB | 30.4 GHz notch in 3D only | 24.0 / 22.8 GHz (−5.0 %) |
| TDR min | −8.4 % | −0.6 % | −2.1 % |
| delay, 3D / ladder | 10.1 / 13.5 ps | 14.1 / 15.2 ps | 13.5 / 18.1 ps |
| C, L 3D / ladder | 158 / 217 fF, 673 / 855 pH (diff.) | 278 / 269 fF, 686 / 791 pH | 391 / 478 fF, 262 / 503 pH |
| ladder without the signal-layer pours: phase error at 20 GHz | 25° → 3.4° | 8° → 2.3° | 34° → 9° (notch > 25 GHz) |

- The M.2 16 GHz dip is real: the broad reflection maximum of the low-impedance pair via, not a resonance; the
  ladder makes it 0.29 dB too deep.
- Ladder findings for follow-up work: GND pours on the via's signal layers taken as planes (too slow by 1.1–4.6 ps,
  too capacitive on M.2 and LibreVNA); series inductance 105–241 pH too high (26–127 pH without the signal-layer
  pours); the REFINOUT2 notch at 30 GHz (likely its return-via cage) is missing.

## 10. Plane resonances of finite boards (Block 3)

Full report: **`BOARD_RESONANCES.md`**; `board_resonances.py` reproduces everything (cases `resonance_cases/`,
results `../via_ref/R_*`, `../via_ref/BV_vna_clk_w5`, `results/board_resonances*.json`, plots
`results/board_resonances/`). The model is the option `--via-cavity` (off by default).

### Model checks (unit tests in `kisi_si::via_cavity`, `kisi_si::via_pbm`)
| # | Test | Where | Result |
|---|---|---|---|
| M1 | Rectangular cavity, single series with Kummer acceleration, against the double modal series | `rect_green_matches_double_series` | within 0.2 % |
| M2 | A large cavity against the infinite plates (Hankel function), self and transfer terms | `rect_green_large_cavity_is_free_space` | within 0.1 % |
| M3 | Low-frequency limit: the plate capacitance εA/d; a short reduced as a port = as a load | `rect_green_low_frequency_capacitance`, `rect_short_load_reduces` | within 1 % / 10⁻⁹ |
| M4 | Grid (five-point, exact near field) against the rectangle, 4 ports and a short | `grid_matches_rect` | within 1 % |
| M5 | A grid window with absorbing sides against the infinite plates | `grid_window_is_open` | within 5 % |
| M6 | Edge radiation: the slot conductance's short and long limits ((w/λ)²/90, w/(120λ), halved for a board edge); the straight edges of an L shape | `slot_conductance_limits_and_edges` | within 2 % / 5 %; edges exact |
| M7 | Ladder with a large lossy cavity = the infinite ladder; one via in the coupled solver = its ABCD | `large_cavity_is_the_infinite_ladder`, `coupled_single_matches_abcd` | \|ΔS21\| < 2·10⁻³ / 10⁻⁹ |
| M8 | Decoupling-capacitor parasitics from `data/components.json` | `decap_parasitics_from_data` | 0402: 0.75 nH, 25 mΩ at 100 nF |

### Finite boards in 3D against the ladder (max / rms \|ΔS21\| below 12 GHz, dB)
| case | infinite plates (default) | cavity, rectangle | cavity, grid | resonances below 12 GHz found (grid) |
|---|---|---|---|---|
| one via, no return via | 4.90 / 1.13 | 1.92 / 0.47 | 1.75 / 0.46 | 6/6, \|Δf\| 2.0 %, \|Δdepth\| 0.18 dB |
| one return via 2 mm away | 2.41 / 0.75 | 1.76 / 0.44 | 1.71 / 0.43 | 8/8, 2.5 %, 0.23 dB |
| four return vias at 1 mm | 0.04 / 0.03 | 0.05 / 0.03 | 0.05 / 0.03 | none in 3D |
| L-shaped planes | 5.16 / 1.13 | – | 1.61 / 0.44 | 7/8, 1.8 %, 0.25 dB |
| capacitor in the cavity | 5.29 / 1.25 | 2.04 / 0.51 | 1.86 / 0.51 | 7/8, 1.6 %, 0.39 dB |
| two vias sharing the cavity (coupling S31 peak, 3D −12.2 dB) | 3.95 / 1.03, no coupling | 1.81 / 0.39, −12.1 dB | 1.67 / 0.39, −12.1 dB | – |

- Without edge radiation the resonances are 0.6–1.0 dB too deep and the error above 12 GHz is 2–3× larger.
- Above 12 GHz neither form follows the 3D (rms 1.7–2.2 dB, the infinite plates 1.5–1.9 dB).
- Default path unchanged: `compare_kisi.py` identical to `results/kisi_models.json`; `compare_cli.py` identical.
  With the option the Block 1 references get worse (their planes are infinite in 3D, the KiCad stand-in board
  ends at the window).

## 11. Return groups per plane pair (Block 4)

Full report: **`BOARD_RETURNS.md`**; `board_returns.py` reproduces everything (cases `return_cases/`,
results `../via_ref/RG_*`, `../via_ref/BV_ts_m2_per0_q050f25s_e35_in2gnd`, `results/board_returns*.json`,
plots `results/board_returns/`). The model is the option `--via-returns` (off by default).

### Model checks (unit tests in `kisi_si::via_pbm`)
| # | Test | Where | Result |
|---|---|---|---|
| M9 | The ladder solved as nodes and spans together (the core with return spans) without returns = the chain | `coupled_core_equals_chain` | 10⁻⁹ |
| M10 | A return joined to the outer of three planes in one dielectric (floating across the middle one) = shorts in both pairs; joined to all three = the ladder before return groups; joined to one plane (capacitive) = a poorer return than shorts | `floating_return_across_a_plane` | < 2·10⁻³·f/GHz in S21; 10⁻¹²; holds |
| M11 | A capacitor post without impedance = a short; the roles short / floating / capacitive / open; a 1 nH capacitor is a poorer return than a via at 10 GHz | `cap_post_and_roles` | 10⁻⁹; holds |
| M12 | A return's capacitance to a plane it passes (r–z Laplace with and without the post): between the coaxial stub value and 3× it, smaller for a larger opening or a shorter stub | `post_capacitance` | holds (119 fF for the GND vias of `RG_s4_pwr_n4`) |
| M13 | The returns reduced onto the via's current moments (W) against the full nodal solve | `board_returns.py`, `RG_s4_pwr_n4` | cavity 10⁻¹²; infinite plates (W interpolated on 0.25 GHz) ≤ 0.001 dB |

### 3D against the ladder (max / rms |ΔS21|, dB)
| case | before return groups (default) | return groups, infinite plates | return groups, cavities | |
|---|---|---|---|---|
| GND vias not joined to the PWR plane (`RG_s4_pwr_n4`), 0.5–12 / 12–20 GHz | 8.45 / 1.97, 6.65 / 1.69 | 6.15 / 1.36, 4.44 / 1.11 | 2.48 / 0.68, 3.17 / 0.85 | 7/7 dips, 2.1 %, 0.44 dB |
| the same with a capacitor 2 mm away (`RG_s4_pwr_n4_cap2`) | 9.02 / 2.10, 6.77 / 1.70 | 6.45 / 1.42, 4.33 / 1.12 | 2.94 / 0.77, 2.67 / 0.79 | 8/8 dips, 2.0 %, 0.44 dB |
| GND/GND (`R_s4_rect_n4`) | 0.04 / 0.03, 0.11 / 0.04 | identical | identical | – |
| ThunderScope stack, GND vias across a PWR In2 (`RG_ts6_sw_pwr`) | 0.10 / 0.04, 0.26 / 0.09 | 0.07 / 0.03, 0.19 / 0.07 | 0.11 / 0.03, 0.18 / 0.08 | – |
| ThunderScope REFINOUT2 (board via, 3D with In2 as it is) | 0.10 / 0.06, 0.38 / 0.23 | 0.16 / 0.08, 0.66 / 0.41 | – | – |
| ThunderScope M.2 pair (Sdd, quick mesh) | 0.27 / 0.15, 0.30 / 0.27 | 0.28 / 0.15, 0.31 / 0.28 | – | – |

- The floating supply plane (PWR − GND In2): 3D 0.06 / 0.14 dB at 16 / 20 GHz on the synthetic
  ThunderScope stack, ladder 0.07 / 0.13 dB; on REFINOUT2 3D 0.05 / 0.10 / 0.19 dB at 12 / 16 / 20 GHz,
  ladder 0.07 / 0.15 / 0.28 dB. The M.2 pair: no effect in 3D (≤ 0.002 dB to 20 GHz) nor in the
  ladder (≤ 0.03 dB).
- Default path unchanged: `compare_kisi.py` identical to `results/kisi_models.json` (56 rows), also
  with `KISI_VIA_RETURNS=1`; `check --all` of both boards byte-identical; with the option no verdict
  changes, ThunderScope 2.9 → 4.1 s, LibreVNA 1.0 → 1.15 s.

## 12. Block 5: the warning, the ladder fixes, the TDR of the validation scripts

### The warning in the default path
- Channel vias whose path crosses a GND/supply plane pair that no return via within 3 mm joins or
  bridges (`via::open_pairs`) get a warning in `net`, the viewer's lane/net detail and `check`
  (`via_warning`, `via_warnings`); the numbers do not change. Example boards: no such via; the old
  "reference changes … capacitors" warning dropped on two LibreVNA nets whose pairs are bridged.
  `RG_s4_pwr_n4` / `_cap2`: warned (with one capacitor); `RG_ts6_sw_pwr` (bridged), `RG_ts6_sw_gnd`,
  `R_s4_rect_n4`: not. Unit test `via::tests::open_plane_pair_warning`. BOARD_RETURNS.md section 8.

### The TDR of the validation scripts
- `sparam.step_response` summed the step from the middle of the Gaussian edge, so a reflection at the
  reference plane lost its first half and left an offset (a 60 fF capacitor read as a 56 Ω plateau).
  Now from three rise times before the edge, as kisi's `rf::tdr`. Re-scored: ladder on the single
  vias (`rust_models.py`) TDR error median / p90 1.3 / 9.3 → 1.1 / 4.3 %; kisi channel, single vias
  1.4 / 7.4 → 2.3 / 5.5 %, pair vias (Zdiff) 5.9 / 17.9 → 11.7 / 14.4 %; the long-stub maxima of
  section 7 (11–16 %) were this artefact (now 1–5 %). The numbers of sections 3, 5 and 7 are those of
  their time.

### Board vias at the via centre, and the pours on the signal layers
- The board-via comparisons of Blocks 2–4 carried the trace the channel leaves to the ladder
  (`Feed::dropped`): now at the via centre (`PbmVia::abcd_centre`).
- Pours on the via's signal layers as coplanar returns (`ViaGeom::coplanar`, default): M.2 pair phase
  23.6° → 2.2°, C 214 → 144 fF (3D 158); REFINOUT2 L 791 → 712 pH (686); LibreVNA RF via L 337 →
  225 pH (262), 6–12 GHz 0.20 → 0.44 dB; FPGA_CLK_R C/L/delay within 4 % of 3D. `compare_kisi.py`: all
  56 rows identical. Example boards: no verdict changes, 9 + 10 rows (BOARD_VIAS.md section 5).
- Series inductance after both: −14 … +12 % without a common sign; no change. Wide traces (X cases):
  the 44/47 Ω fixtures of their 3D de-embedding; no change. B_ts6_L1L6 still lacks 50 fF.
- `--via-cavity` / `--via-returns` stay off by default (BOARD_RETURNS.md section 7).

## 13. Block 6: the ladder's loss above 12 GHz, the cavity run time

### Loss above 12 GHz: plated pads as return posts
- Power balance 1 − |S11|² − |S21|² of REFINOUT2 (BOARD_VIAS.md section 6.1): at 20 GHz 3D 9.4 %, ladder
  16.3 %, of it 11.6 % radiation into the infinite plane pairs (the same ladder with a lossless dielectric).
  Not the dielectric, the conductor, the ports or the de-embedding (raw minus fixtures: 5.1 %, so the 3D
  number is good to about ±4 points). Cause: the return-via cage is open on one side, closed on the board
  by the plated GND pins of the coaxial connector (the nearest at 2.99 mm) and a ninth via beyond the cap
  of 8. Block 1 references: ladder minus 3D median +0.8 / +0.4 points at 12 / 20 GHz.
- Fix (default): `via::plane_posts` – vias of plane nets and plated holes of plane-net pads (`Pad::plated`) –
  and the nearest 12 within 3 mm (`via::RETURN_POSTS`). REFINOUT2: 12–20 GHz max |ΔS21| 0.41 → 0.02 dB,
  loss at 20 GHz 16.2 → 7.1 % (3D 9.4), 20–25 GHz rms 0.62 → 0.32 dB, phase at 20 GHz 2.4 → 3.8°; M.2
  pair, FPGA_CLK_R unchanged, LibreVNA RF via 1.63 → 1.64 dB. `compare_kisi.py` / `golden.py vias`: all 56
  rows unchanged. Example boards: no verdict changes, 5 + 5 rows (`golden.py boards`, ts.check and
  vna.check). Validation switches `KISI_RET_PADS=0`, `KISI_RET_CAP=8`; `board_vias.py fixes` keeps the
  three states (before Block 5, Block 5, Block 6). Unit test `via::tests::plated_pads_are_return_posts`.
- With return groups REFINOUT2 is 0.29 dB off at 12–20 GHz (Block 5: 0.67); they stay an option
  (BOARD_RETURNS.md section 7).

### The cavity run time
- 31 744 grid factorisations (79 of 105 s on ThunderScope) for environments that are mostly the same
  matrix. Shared factorisations per region (`via_cavity::GridGroup`), a registering pass, fixed-pattern
  assembly (`kisi_sparse::ComplexPattern`), reproducible order of the shorted cells: ThunderScope 102–109 →
  55–60 s on one thread, LibreVNA 14–15 → 11 s; one and four threads byte-identical
  (BOARD_RESONANCES.md section 9). Unit test `via_cavity::tests::shared_grid_groups`.

## 14. Causal and passive line, via and pair models (block 2, accuracy)

The channel export (`docs/research/channel-export.md` on `feat/channel-export`) found kisi's own channel model
not fully passive or causal above about 10 GHz. Causes and fixes:
- **Via pairs, negative resistance.** The pair's odd and even mode took Z_PP ∓ Z_PN of the plane-pair matrix
  reduced with the return posts. With returns that are not symmetric about the pair (one side of a fan-out),
  Z_PP ≠ Z_NN and that is not a quadratic form of the (passive) matrix: the spans of ThunderScope's ADC lane
  D2B came out with Re Z < 0 from about 8 GHz and its 4-port with a largest singular value of 1.286 at
  24.75 GHz. Now (Z_PP + Z_NN)/2 ∓ Z_PN, the mode voltage (V_P ∓ V_N)/2 at currents ±I. In addition the
  radial-line matrix treats every barrel as a cylinder of uniform current (Graf's addition theorem:
  Z_ii ∝ J0(ka)·H0⁽²⁾(ka), Z_ij ∝ J0(ka_i)·J0(ka_j)·H0⁽²⁾(kd)): its real part is then the radiated power,
  positive semi-definite; the line source's H0⁽²⁾(ka) on the diagonal left an indefinite matrix whose
  reductions go negative for a via in a close cage (4 returns at 0.5 mm: −0.002 Ω·m⁻¹ at 20 GHz). Unit test
  `via_pbm::tests::radial_reductions_passive` (both earlier forms fail it).
- **Dielectric.** The lines had Djordjevic-Sarkar in ε′ and ε″ already; the via ladder (node capacitances, the
  plane pairs' wavenumber), the coaxial via sections and the cavity option used constant εr and tanδ, which
  Kramers-Kronig does not allow. All use `rf::eps_c` now (the 3D references' conductivity model stays for the
  comparisons with kisi-fdtd, `f_loss`). The lines' ε′ follows the mode's own loss amplitude
  (A + B·k(f), B from the mode's tanδ at 1 GHz) instead of the material's ratio.
- **Roughness.** Huray was an R multiplier with the smooth internal inductance: now the causal Huray factor
  (Bracken, DesignCon 2012) multiplies the whole surface impedance R_dc·√(1 + jf/f_c) (DC to skin effect with
  its internal inductance; before √(R_dc² + R_ac²) and L_int = R_ac/ω, unbounded at low frequency).
- **Microstrip dispersion.** Getsinger's ε_eff(f) is a real function without a Kramers-Kronig partner, and a
  passive shunt admittance has C(0) ≥ C(∞): a rising capacitance is possible only below a resonance. Now a
  lossless Lorentz term with its resonance at max(fp/√G, 2·fmax), matched to Getsinger at fmax (within a few
  percent of the small rise on thin dielectrics; a thick one, fp inside the band, rises only near the top).
- Unit tests `rf::tests::line_models_positive_real` (Z(s), Y(s) with Re ≥ 0 on a grid of the right half-plane
  up to 10¹³ s⁻¹, for microstrip thin and thick, stripline, a field-solved mode, the no-plane fallback, all
  roughness presets), `lossy_line_causal` (200 mm, pulse response before the front < 1e-5 of its peak; the
  earlier model 3.7e-4), `causal_huray_is_huray`.

**The example boards** (`crates/kisi-core/tests/causality.rs`, every lane of ThunderScope and LibreVNA as the
check builds it, 25 GHz, 500 points; in CI with the models checks): largest singular value of the board's
S-parameters and the pulse response (Gaussian window leaving 1e-6 at 25 GHz) before 80 % of the static delay
(transfer, relative to its peak) or before the edge (reflection, relative to the incident pulse). Lanes still
ringing after 0.6/df (filters with resonances below the 50 MHz step: 18 of 98) are reported, not judged.

| | before | after | limit |
|---|---|---|---|
| largest singular value | 1.2861 (ts D2B_P, 24.75 GHz) | 1.0000000 | 1 + 1e-9 |
| transfer precursor, max (median) | 8.6e-4 (8.6e-5) – 45 lanes above the limit | 7.4e-5 (7.1e-6) | 3e-4 |
| reflection precursor, max | 2.1e-4 | 1.3e-4 | 1e-3 |

**References.** Field solver and HFSS (`validate_field.sh`): unchanged, all 15 values within 0.5 % of the
baseline, worst 1.7 %. RLGC of the HFSS microstrip at 1 GHz: R 3.33 / L 304.6 nH / G 14.0 mS / C 123.3 pF per
metre (HFSS 3.26 / 307 / 13.86 / 123; before 3.33 / 304.6 / 14.0 / 123.5). `compare_kisi.py` (all 56 rows in
`results/kisi_models.json`):

| | before | after | after, via ladder with constant εr (as the 3D) |
|---|---|---|---|
| via TDR error median / p90 | 2.33 / 5.52 % | 2.45 / 5.92 % | 1.94 / 5.13 % |
| via mean \|ΔS21\| median | 0.102 dB | 0.097 dB | 0.096 dB |
| pair Zdiff error median / p90 | 11.67 / 14.35 % | 12.57 / 14.42 % | 11.95 / 14.78 % |
| pair max phase error median | 3.5° | 1.8° | 3.5° |
| stub notch (d200 / d300) | −0.9 / −2.3 % | +1.4 / −0.5 % | −0.9 / −2.3 % |

The 3D references model the dielectric as a constant εr with a conductivity fixed at f_l; the causal ladder's
ε′ is 1.5–3 % lower than εr(1 GHz) at 5–15 GHz, which raises the via impedance in the 25 ps TDR by about 1 %
against them – the last column is the same ladder with the references' constant εr. The line model alone
(new roughness and dielectric, old via dielectric) brings the via TDR error from 2.82 to 2.45 %; the mode
symmetrisation changes no reference row (their returns are symmetric); the J0 factor moves the via |ΔS21|
by ≤ 0.02 dB. Lines (group T): |ΔS21| unchanged, TDR +0.3–0.5 Ω. Board vias (`board_vias.py ladder/compare`,
the ladder with the 3D loss model): every stored value unchanged.

**Example boards** (`golden.py boards`): no verdict changes; ts.check 187 and vna.check 171 values moved –
RL of well matched lanes (above 30 dB) by up to 7 dB, of the PCIe lanes by up to 0.2 dB, IL at Nyquist by at
most 0.08 dB, eye height by at most 0.015.

## 15. Pads and gold fingers without a reference plane (block 2, accuracy)

Before, a track inside an SMD pad of its own net (tracks run to the pad's centre; a card-edge finger) was
sampled with the track's width, a part's pads were a fixed capacitance per package (0402: 0.10 pF, whatever lay
below), and copper with no plane on any layer got 1.5 × the target impedance. Now:
- **Pad cross-sections** (`extract`): a track sample inside a same-net SMD pad has the pad's chord across the
  track as its width (`board::pad_chord`), the track is split exactly at the pad's edge
  (`board::pad_segment_span`); field solver or formula then see the pad with what lies below it (a cut-out in
  the plane included). These pieces stay out of the net's impedance statistics – the pad's synthetic track
  stands for the pad there – but keep their length.
- **Pad shunts** (`topology::pad_shunt_c`): the half of a part's pad that no track covers (towards the body) is
  a capacitance Σ C′·ℓ from the cross-sections of the pad's synthetic track; the package estimate stays as the
  fallback (no track enters the pad, no impedance). Also for the pads of parts that are not fitted.
- **No plane** (`analytic::no_plane`): copper with no plane above or below takes its return in coplanar copper
  within max(0.5 mm, 3 widths) and in a pair's partner – coplanar waveguide and coplanar strips without ground
  (Simons), the sides as parallel slots; otherwise still 1.5 × the target (an estimate, warned).
- Unit tests `topology::tests::pads_from_their_geometry`, `analytic::tests::coplanar_without_plane`,
  `board::tests::pad_chord_and_span`.

**3D references** (group P, `via_cases.py --groups P`, kisi-fdtd 0.46–1.49 M cells, 25–850 s on one shared
core): the pad or finger in the middle of an 8 mm 50 Ω microstrip on the ThunderScope stack (ports on the
microstrip; the TDR's line level of the 3D lumped ports sits a few ohms low). TDR with a 20 ps edge, the
section's extreme against each curve's own line level (Ω), and the worst |S11| up to 20 GHz
(rows in `results/kisi_models.json`):

| case | 3D | kisi before | kisi now, formula | kisi now, field solver |
|---|---|---|---|---|
| 0402 pad 0.56 × 0.62 mm, plane intact | −5.7 Ω, −11.9 dB | +0.1, −47.9 | −4.0, −13.2 | −4.0, −13.3 |
| the same, L2 cut out under it (next plane 1.14 mm) | −3.6, −18.7 | +17.6, −5.6 | +5.8, −12.4 | −0.8, −29.9 |
| finger 0.7 × 2.1 mm, GND fingers at 0.3 mm, L2 cut back | +11.8, −8.7 | +22.8, −5.2 | +7.0, −12.4 | +7.3, −11.2 |
| the same, every plane cut back (no plane) | +14.4, −7.7 | +9.4, −8.8 | +9.2, −9.3 | +9.3, −9.2 |
| finger pair, pitch 1 mm, no plane (Zdiff) | −5.6, −13.9 | −16.3, −11.3 | −13.3, −6.8 | −12.4, −7.4 |

The section's peak impedance (TDR maximum, 3D / now): pad over the cut-out 50.9 / 51.2 Ω with the field
solver (formula 56.9, flagged rough; before 68.2), finger with a far plane 61.5 / 60.1 (before 76.3), finger
without a plane 62.9 / 62.7, finger pair 115.3 / 114.7 Ω Zdiff (before 136.7). The TDR deltas of the single
cases are within about 4 Ω (8 %), the pair without any plane reads about 7 Ω low in its section (its feed of
two 0.19 mm traces at 1 mm pitch reads 107 against 117 Ω in 3D, the formula's diff microstrip): the no-plane
pair is an estimate to about 15 %.

**ThunderScope PCIe lanes** (check, formula impedances; RL limit 10 dB): the trace through each 4.2 mm
gold finger was a 0.19 mm microstrip over B.Cu 1.42 mm away (161 Ω diff, field solver 172 Ω); with the
finger's width it reads 93 Ω. Return loss PER0–3 3.9–5.0 → 9.2–9.5 dB, PET0–3 2.5–5.2 → 5.0–7.2 dB, IL at
Nyquist PER 2.4–3.4 → 1.6–1.8 dB, PET 2.8–4.6 → 2.9–4.4 dB; still yellow. What remains: the 0402 pads of the AC capacitors and 0 Ω
resistors over the plane 0.11 mm below (about 21 Ω over 0.45 mm per pad – the board has no cut-outs there),
the stubs to the unfitted alternatives, the vias. The 1.5 × fallback was not the cause: on these lanes it
covers only zero-length pieces next to vias.

**Example boards** (`golden.py boards --update`): 12 verdict changes, all on the impedance criterion (the
length-weighted mean of the lane's nets), from counting each pad once with its own cross-section:
ThunderScope /ADC/D3A_P yellow → green (110.2 → 109.8 Ω), /ADC Driver 2/OUT_P and Driver 3/OUT_P green →
yellow (92.0 → 86.9 / 85.5 Ω: short nets dominated by 0402 pads), PCIe_REFCLK_P green → yellow (90.7 → 87.4 Ω:
the 161 Ω finger trace had lifted the mean, the trace itself is 88 Ω); LibreVNA RF_HBLB_AMP, C270-Pad2,
T1-OUT+, T3-OUT- yellow → green (60–62 → 51–55 Ω: traces over openings under pads were counted as narrow
traces), LO1_RFOUTB_N_RC, C121-Pad2, C160-Pad2, C165-Pad2 green → yellow (46–48 → 42–45 Ω). Values moved in
overview (the classes' share within ±5 %, by up to 13 points), check, crosstalk (the coupling
scan sees the pads' width).

## 16. Every reference case in openEMS

All 106 cases in `validation/via_ref` (groups A–X with their 2×thru, the board vias, the finite boards and the
return-group cases) computed again with openEMS 0.37.0 (FDTD, staircase metal, UPML), the independent check that
`validation/via_references.md` planned; before, S4 compared one case each of A, B, D and E. Script:
`openems_refs.py` (`plan`, `run`, `merge`, `stair`, `compare`); results in `via_ref/<case>/openems.sNp` (refined
mesh), `openems_base.sNp` (kisi-fdtd's mesh), `meta.json` (`runs.openems`, `runs.openems_base`: cells, run time per
excited port, cell rate, runner) and `results/openems_refs.json`. The reference page shows the openEMS curves next to
kisi-fdtd (also de-embedded with openEMS' own 2×thru) and a per-case agreement chip.

### Method
- **The same case files.** `openems_case.py` builds each case from its kisi-fdtd JSON as CSXCAD primitives, so
  openEMS decides for itself which mesh edges are metal. Same dielectric loss (σ at `f_loss`), Gaussian pulse,
  lumped ports (50 / 1 Ω), lumped series R–L–C, PML in the outer 8 cells, end criterion.
- **Two meshes per case**, both from kisi-fdtd's mesher:
  - *base*: the lines of the kisi-fdtd reference run. There kisi-fdtd with staircase metal (`stair`) and openEMS
    must agree to the solver implementation – ports, PML, lumped elements, materials, geometry rules: the
    like-for-like check.
  - *fine*: the cell at metal edges, vias, ports, GND vias and refinement boxes halved (`dfine`, `dmesh`,
    `mesh.refine` ÷ 2), the largest cell kept at λ/20. Staircase metal converges at first order (S1, S2: at 0.035 mm
    0.18 dB off in \|S21\|, conformal 0.07 dB), so the halved cell halves openEMS' metal error and \|fine − base\|
    estimates what remains; 2·fine − base is the first-order extrapolation. kisi-fdtd's conformal reference is
    compared with *fine*. One very fine staircase mesh (a quarter of the cell, to fall below kisi-fdtd's own error)
    would cost ≈ 30× the cells × steps; the two meshes give the result and its error bar for ≈ 5× the base cost.
  - G and H have one-edge ports and an one-edge capacitor (`y0 = y1`): the refined mesh makes the port itself
    thinner, so they are judged on the base mesh.
- **kisi-fdtd on the fine mesh** for nine cases (conformal and staircase): its own mesh convergence, and the
  like-for-like check on the second mesh.
- **Metrics** up to each case's f max: the main through path (S21; S31 of a pair; S21 of two vias sharing a cavity)
  mean and max difference in dB, \|S11\| where either result is above −30 dB, the delay difference
  Δτ = τ_kisi-fdtd − τ_openEMS from the unwrapped phase (mean above f max / 4; positive: kisi-fdtd slower),
  the TDR extremes (25 ps), transmission dips (resonances, stub notches) matched within 3 %, G: the \|Z11\| peaks,
  H: the loop inductance at 50 MHz. A–E and X also de-embedded, each tool with its own 2×thru (NZC; pairs first
  order), to 20 GHz.
- **Where it ran:** GitHub Actions on the branch (`.github/workflows/openems-refs.yml`, `openems_plan.json`):
  openEMS built from the v0.37.0 tag and cached; the tasks (case × mesh × excited port, 358) balanced over 15
  shards of a 4-vCPU runner (one socket, two cores with two threads each, 15.6 GB; AMD EPYC 7763 / 9V45 / 9V74,
  Intel Xeon Platinum 8370C / 8573C, Xeon 6973P-C – recorded per result since the second run), two tasks with two
  threads each, or one large task with four. openEMS ran at 60–90 M cell updates/s per two-thread task; the full
  run took 5 h 23 min on 15 runners (40.8 runner-hours), the two re-runs after the fixes 1.9 h (17.6) and 1.0 h (5.5). openEMS needs about the physical
  time kisi-fdtd needs on ringing cases but at least ≈ 0.85 ns (its energy criterion weighs the cell volumes,
  kisi-fdtd's does not), up to 3× kisi-fdtd's steps on short cases. kisi-fdtd staircase ran locally (2 vCPUs).

### Fixes to the openEMS chain found on the way
- **Polygon copper:** CSXCAD counts a point on a polygon edge as inside. The zero-width slits and window cuts of
  KiCad's fractured fills therefore became metal lines wherever they fell on a mesh line – on the M.2 case a line
  across both L3 traces to the pour, S11 → −1 at DC. Heights with polygon copper now go to openEMS as the exact
  union of their sheets (shapely: rect ∩ even-odd rings − holes, with kisi-fdtd's choice on boundaries that fall on
  mesh lines). Checked edge by edge against openEMS' own PEC dump (`debug_pec`): 0–18 differing edges per case
  (single vertex ties), against up to 178 before.
- **Holes of one sheet** cut the other sheets at the same height in openEMS (kisi-fdtd: the union of the sheets,
  each minus its own holes) – two LibreVNA board cases.
- **Ports off their line:** the mesher can move a port's line by a micrometre (merged lines); openEMS then dropped
  the zero-width box (no excitation, NaN in the in2gnd case). Ports and lumped elements now sit on the nearest
  lines, as in kisi-fdtd.
- **Slot antipads:** the straight sides on mesh lines were hole in openEMS and metal in kisi-fdtd; the four oval
  cases were re-run. Round antipads only touch a line at their four extremes (one edge each); re-running an
  unchanged case with the final code reproduces it to \|ΔS\| ≤ 1·10⁻⁵.
- `openems_compare.py` and `run_refs.py --openems` now use the reference mesh (S4): on a staircase-mode mesh with a
  line on every circle extreme, tie decisions had put A_s4_d300_ap800 at 0.27 dB.

### Implementation: the same mesh, both staircase
102 cases (the four one-port H cases below): \|S21\| differs by 0.018 dB (median of the case means),
90th percentile 0.052 dB, largest 0.100 dB; \|Δτ\| median 0.04 ps, 90th percentile 0.10 ps. On the fine mesh
(seven cases with both): 0.007–0.031 dB and \|Δτ\| ≤ 0.07 ps, except X_s8x2 (0.10 dB, 0.27 ps). G resonances
identical to the 10 MHz grid. The largest same-mesh differences:
- **R (0.074 dB median):** the finite boards stop at −35 dB, and the two tools stop at different times (openEMS ≈ 1.4×
  later); kisi-fdtd itself moves by 0.07 dB mean on RG_s4_pwr_n4 when run to −50 dB instead.
- **thru_s8x2_L1 (0.10 dB, 0.48 ps):** the wide microstrip (0.93 mm) over a distant plane puts much field into the
  air box, where openEMS' UPML reflects more (S3).
- **H loop inductance (openEMS +3.5 … +7.6 %):** kisi-fdtd staircase 1.339 / 1.468 / 1.675 / 2.083 nH, openEMS 1.410 / 1.527 /
  1.734 / 2.241 nH at 2 / 5 / 10 / 20 mm, kisi-fdtd conformal 1.328 / 1.456 / 1.663 / 2.071 nH. Not the PEC end caps
  (openEMS without them: unchanged); the one-edge 1 Ω port is measured differently (openEMS: a probe line and an
  H loop at mid-height, kisi-fdtd: the edge sums). The CST session (branch `feat/cst-refs`) finds CST between the two.
  The loop inductance is therefore defined only to about ±4 % by the port model – well inside the ±15–25 % targets.

### kisi-fdtd's references against openEMS (refined mesh)
| group | cases | Δ\|S21\| kisi-fdtd vs openEMS fine: median of the case means / largest mean (dB) | largest max (dB) | Δτ median (ps) | same mesh, both staircase: median / largest mean (dB) | openEMS base → fine: median (dB) |
|---|---|---|---|---|---|---|
| A | 6 | 0.081 / 0.112 | 0.74 | -0.42 | 0.026 / 0.032 | 0.103 |
| B | 8 | 0.088 / 0.703 | 6.58 | 0.81 | 0.019 / 0.034 | 0.079 |
| C | 8 | 0.086 / 0.105 | 0.69 | -0.57 | 0.017 / 0.031 | 0.072 |
| D | 8 (2: base mesh only) | 0.053 / 0.090 | 0.25 | -0.46 | 0.023 / 0.028 | 0.025 |
| E | 12 | 0.101 / 0.268 | 0.61 | -0.45 | 0.024 / 0.057 | 0.057 |
| X | 2 | 0.059 / 0.107 | 0.33 | -0.38 | 0.046 / 0.085 | 0.019 |
| T | 12 | 0.011 / 0.037 | 0.30 | -0.06 | 0.013 / 0.021 | 0.004 |
| P | 5 | 0.060 / 0.062 | 0.47 | -0.03 | 0.016 / 0.024 | 0.043 |
| thru | 24 | 0.013 / 0.126 | 0.35 | -0.07 | 0.007 / 0.100 | 0.007 |
| board | 5 (2: base mesh only) | 0.406 / 1.675 | 17.32 | -1.02 | 0.022 / 0.054 | 0.349 |
| R | 6 | 0.100 / 0.113 | 0.83 | -0.32 | 0.074 / 0.089 | 0.077 |
| RG | 4 | 0.166 / 0.290 | 2.56 | 0.41 | 0.039 / 0.054 | 0.162 |
| G | 2 | 0.032 / 0.061 | 2.14 | 0.00 | 0.023 / 0.042 | – |
| H | 4 | loop inductance at 50 MHz, openEMS (base mesh) against kisi-fdtd: +6.2 / +4.8 / +4.3 / +8.2 % (2 / 5 / 10 / 20 mm) | – | – | openEMS against kisi-fdtd staircase: +3.5 … +7.6 % | – |

- **Through vias:** kisi-fdtd is 0.4–0.6 ps faster (A–E median −0.46 ps). Both staircase solvers agree with each
  other and keep this offset on the refined mesh (kisi-fdtd staircase against its conformal result on the fine mesh:
  0.5–0.75 ps), while kisi-fdtd's conformal result moves by 0.07–0.20 ps. Staircase metal makes vias, pads and
  antipads slow, and halving the cell barely changes it; the conformal result is the converged one here.
- **Inner-layer exits and the ThunderScope through via:** kisi-fdtd is 0.8–2.1 ps *slower* (B and E L1→L3,
  B_ts6_L1L6). Here kisi-fdtd's own reference is not converged: on the fine mesh it moves by −1.13 ps
  (B_s8_L1L3_stub_d300), −1.12 ps (E_s8_L1L3_p1000_round_g2) and −1.81 ps (B_ts6_L1L6), and then agrees with
  openEMS fine within 0.2–0.5 ps; X_s8x2 (no L2 plane) and the L1→L8 cases move by 0.04–0.12 ps only. The CST
  session finds kisi-fdtd 1.0–2.4 ps slow on L1→L3 / L1→L6 with openEMS within 0.1–0.5 ps of CST: the same finding
  from a third solver.
- **Stub notches:** the same mesh puts them at the same frequency in both tools (20.8 / 20.9, 21.0 / 21.0,
  25.4 / 25.4 GHz). openEMS refined moves up towards kisi-fdtd (21.0 → 21.4 GHz, 25.4 → 26.0 GHz; extrapolated
  21.8 against 21.5–21.8 GHz); kisi-fdtd refined moves up by 0.2 GHz. Matched dips 1.2–2.9 % apart, which is what
  gives the 3–7 dB maxima on the notch flanks (the CST session: CST and kisi-fdtd within 0.3–0.45 dB there).
- **Finite boards and return groups:** resonances matched within 0.0–0.4 % (R, 8/8 per case; R_s4_rect_2v 7/7) and
  1.0–1.2 % (RG), depths within 0.1–0.5 dB; RG's base → fine change (0.16 dB) is the largest of the plain
  cases – the floating GND vias across the PWR plane (narrow clearances) are where staircase metal matters most.
- **G:** the first six resonances identical to the 10 MHz grid (11/11 and 10/10 peaks matched, at most 0.23 %).
- **De-embedded via alone (A–E, X, each tool with its own 2×thru, to 20 GHz):** \|S21\| median difference
  0.007–0.025 dB; the via's TDR extremes median 0.2–1.6 %, at most 6.8 % (the long B stubs).
- **Board vias (BV):** the same mesh agrees within 0.010–0.054 dB, \|Δτ\| ≤ 0.11 ps, after the fixes below.
  Against kisi-fdtd's conformal result the differences are larger – REFINOUT2 0.37 dB, FPGA_CLK_R (vna_clk_w5)
  0.51 dB, the M.2 pair 0.41 dB, the LibreVNA RF via 1.68 dB with a 2.2 % notch shift at 24 GHz, kisi-fdtd 0.5–1.4 ps
  faster – and so is the staircase sensitivity (openEMS base → fine 0.35 dB on REFINOUT2, 1.02 dB on FPGA_CLK_R at
  0.07 → 0.035 mm): fill edges, clearances and pad rings of a real board are where conformal metal matters most.
  The M.2 pair and the LibreVNA RF via are compared on the base mesh only (their refined openEMS runs are invalid,
  below), which includes openEMS' staircase error there.

### Invalid openEMS runs
- **D_s8_cap_2 / D_s8_cap_10, refined mesh:** the run grows without bound (energy falls to −36 dB, then rises
  exponentially). The capacitor's series inductance (0.6 nH) in openEMS' explicit lumped-RLC update loses stability
  when it is split over the more and shorter edges of the 0.0175 mm mesh (without its C: the same; H's R-only shorts
  and the R/RG capacitors on 0.035 mm cells: stable). These two are judged on the base mesh, where openEMS agrees with
  kisi-fdtd staircase within 0.018 dB. kisi-fdtd's lumped elements are implicit and unconditionally stable.
- **BV_ts_m2_per0, BV_vna_rf_hblb1, BV_ts_m2_per0_…_in2gnd:** openEMS' UPML keeps the energy of these board
  cut-outs (copper into the PML on several layers) at −15 … −20 dB instead of letting it decay (with a Mur boundary
  the same case decays to −37 dB), and the long runs grow. They now stop at 1.2 × the physical time kisi-fdtd needed
  (`TCAP` in `openems_refs.py`), so both DFTs cover a comparable window: valid on the base mesh (M.2 against
  kisi-fdtd staircase 0.014 dB) and for in2gnd on both meshes, but the refined runs of the M.2 pair and the LibreVNA
  RF via (2–4 × the steps) still grow before that and are left out.

### kisi's fast models judged against openEMS
Scored against openEMS fine instead of kisi-fdtd (same metrics as `rust_models.py` and `compare_kisi.py`):

| | against kisi-fdtd | against openEMS |
|---|---|---|
| ladder, single vias (29): TDR error median / p90 | 1.07 / 4.265 % | 1.025 / 4.134 % |
| ladder, single vias: max Δ\|S21\| to 20 GHz, median | 0.093 dB | 0.071 dB |
| ladder, pairs (12): max ΔSdd21 median / phase median | 0.111 dB / 1.92° | 0.105 dB / 3.405° |
| kisi's channel (42 cases with the feeds): max Δ\|S21\| median / TDR error median, p90 (35 ps) | 0.581 dB / 8.98, 13.981 % | 0.573 dB / 7.345, 13.49 % |

The scores barely move: the fast models' errors are larger than the difference between the two 3D results, except
the pairs' phase (the through-via delay offset above). No change to kisi's models or to `data/accuracy.json` follows
from openEMS. What may follow from the inner-exit finding: the junction model (section 7) was fitted partly to the
L1→L3 references, which are ≈ 1.1 ps slow; recomputing those references on the refined mesh and re-checking the fit is
a separate change.
