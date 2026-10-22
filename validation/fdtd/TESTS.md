# kisi-fdtd and reduced models: test log

What was tested, how, and the result. Numbers come from the JSON files in `results/` and from
`validation/via_ref/*/meta.json`. The reference page (`python3 make_viewer.py` → `dist/kisi-references.html`,
also attached to every release) shows the same tables (tab "Solver checks") and every case with its curves.
The scripts named here reproduce each result.

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
| 3D FDTD (`kisi-fdtd`) | minutes per via transition (0.3–0.9 M cells), half an hour for a plane cavity; a whole board is out of reach | reference (CPML, conformal metal, checked against analytic cavities and openEMS) | checking the reduced models, structures they do not cover (connector launches, non-round antipads, via fences, plane gaps, via-to-via coupling), frequencies above 25–40 GHz |

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
