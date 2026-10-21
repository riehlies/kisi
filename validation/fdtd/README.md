# 3D FDTD feasibility: kisi-fdtd against openEMS

Status: feasibility test (21.10.2026) for the branch `feat/3d-vias-resonances`. Question: can an own
Rust FDTD (`crates/kisi-fdtd`) reproduce openEMS on via transitions, and how fast is it natively and
as WebAssembly (the viewer)?

## Method
- **Cases** (`cases/*.json`, 20 × 10 mm board, 4 layers FR-4, 0.2 / 1.2 / 0.2 mm, εr 4.3 / 4.5,
  tanδ 0.02, 0.35 mm microstrips, 50 Ω lumped ports at both ends, 0–20 GHz):
  - `thru_ms`: microstrip on L1 over the GND plane L2, 19 mm.
  - `via_l1_l4`: microstrip L1 → via (Ø 0.3 mm, pads Ø 0.6 mm, antipads Ø 0.9 mm in L2 and L3) →
    microstrip L4 over L3; no stitching via, so the return current crosses the L2–L3 cavity.
  - `via_l1_l4_stitched`: the same with four GND vias at 1 mm from the signal via.
- **Same model in both tools:** the mesh comes from `kisi-fdtd --mesh-only` and is passed to
  openEMS; zero-thickness copper, staircased vias, solid cylinders, dielectric loss as a
  conductivity at fmax/2, openEMS' Gaussian excitation, Mur boundaries on all six faces, one
  excitation (port 1). openEMS 0.0.36 (SSE engine), built from source in the cloud workspace;
  S-parameters of both from the port voltages and currents, a = (U + Z·I)/2, b = (U − Z·I)/2.
- `openems_case.py` runs a case in openEMS, `compare.py` compares and plots, `wasm_speed.mjs` runs
  the WebAssembly build in Node (V8, the engine of Chrome).

## Results

| Case | Cells | kisi-fdtd | openEMS (1 thread) | ΔS21 mean / max | ΔS11 mean / max (where > −30 dB) |
|---|---|---|---|---|---|
| thru_ms | 191 k | 4 201 steps, 11.7 s | 30 000 steps, 64 s | 0.005 / 0.02 dB | 0.12 / 0.3 dB |
| via_l1_l4 | 276 k | 14 401 steps, 50 s | 60 000 steps, 176 s | 0.10 / 0.67 dB | 0.59 / 3.4 dB |
| via_l1_l4_stitched | 426 k | 9 401 steps, 56 s | 60 000 steps, 299 s | 0.02 / 0.27 dB | 1.1 / 4.6 dB |

Plots: `results/*.png`. Both tools show the same cavity resonances of the unstitched via
(S21 dips at 7.2, 14.5 and 16 GHz); with four stitching vias S21 stays flat (−0.6 … −0.8 dB) and
S11 below −21 dB. The larger S11 differences are at deep nulls and around −25 dB, where the
staircased geometry near the via and the different stop criteria matter.

**Speed per time step** (cell updates per second, this workspace: Xeon 2.8 GHz, 2 vCPUs):

| | Mcells/s |
|---|---|
| openEMS, SSE engine, 1 thread | 94–101 |
| kisi-fdtd native, 1 thread | 80 |
| kisi-fdtd WebAssembly with SIMD128 (Node / V8) | 42 |
| kisi-fdtd WebAssembly without SIMD | 18 |

Two threads gave no gain for either tool here (openEMS 94 → 94, kisi-fdtd 82 → 80): the two vCPUs
of this workspace share the memory bandwidth. Scaling has to be measured on a desktop.

**Stop criterion:** kisi-fdtd stops when its field energy (Σ εE² + μH²) is 40 dB below the
maximum; openEMS' energy measure stayed at about −35 dB in all cases and ran to the step limit. The
agreement above holds for both; the step counts are therefore not comparable, the speed per step is.

## Conclusions
- The Rust FDTD reproduces openEMS on the same mesh within a few hundredths of a dB in S21 and
  within about 1 dB in S11 above −30 dB, including the cavity resonances.
- Per step it is about 20 % slower than openEMS' hand-vectorised SSE engine natively; as
  WebAssembly with SIMD it reaches half the native speed. A via case of about 300 k cells
  (10–15 k steps) takes 1.5–2 minutes per excitation in one browser worker; the excitations of a
  2- or 4-port run in parallel workers.

## Next steps
1. CPML instead of Mur (fewer reflections, smaller air box), port de-embedding (2×thru).
2. Conformal metal for round vias and antipads (Dey–Mittra or CST-like PBA): coarser cells near
   the via, larger time step – the main lever on run time.
3. Several workers per excitation in the browser (slabs along x, halo exchange by messages).
4. "3D check of this via" in the viewer: geometry from the board (stackup, via, pads, antipads,
   nearby GND vias and planes), 2- and 4-port, results next to the fast model.
5. The reference cases of `validation/via_references.md` with openEMS on finer meshes; the fast
   hybrid model (r–z via solver, cavity model) is calibrated against them.
