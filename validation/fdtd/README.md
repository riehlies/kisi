# 3D FDTD: kisi-fdtd and the via references

`crates/kisi-fdtd` is a 3D FDTD solver (Yee scheme) for via transitions, plane cavities and PDN loops.
This folder holds its validation and the generator of the reference cases from `validation/via_references.md`.
The results are in `validation/via_ref/`. The reference page shows every case (geometry, S-parameters, TDR,
mixed mode, Z, field) and the solver checks. It is built, not kept in the repository:
`python3 validation/fdtd/make_viewer.py` → `dist/kisi-references.html` (self-contained; every release attaches it).

## Solver
- **Mesh:** rectilinear, non-uniform. Fixed lines at layer boundaries, sheet edges, vias, holes and ports, with
  spacing `dfine` there, growing by 1.4 up to λ/20 at fmax.
- **Metal:** zero-thickness sheets (traces, planes with round or slot-shaped antipads, pads) and solid cylinders
  (via barrels), treated **conformally** (Dey–Mittra).
  - On every face that touches a partially free edge, the H update uses the free edge lengths. On faces cut by a
    round boundary it uses the free area.
  - The effective area A_eff = max(A_free, A · weighted free fraction of the edges). It keeps each face's local
    frequency at or below that of a full cell.
  - With every face treated this way, the update operator is similar to a symmetric positive semidefinite one, so
    it has real eigenvalues and runs stably at 0.95 of the Courant limit.
  - `"conformal": false` gives staircase metal, the same as openEMS.
- **Boundary:** convolutional PML (Roden & Gedney, CFS), 8 cells outside the domain, polynomial grading m = 3.
  Whatever touches the domain boundary (planes, dielectric) continues through the PML, so the planes are infinite
  and the board edges add no resonances. Mur and PEC are options.
- **Ports and parts:** resistive lumped ports along z. Lumped series R–L–C elements, trapezoidal and implicit in
  E, I and q, so they are unconditionally stable.
- **Loss:** dielectric loss is a conductivity fixed at `f_loss`, so tanδ is exact there and falls as 1/f away
  from it (no Djordjevic–Sarkar model yet).
- **Excitation:** Gaussian pulse, one run per excited port. The run stops when the field energy has fallen
  `end_db` below its maximum.

## Checks and models
The full test log, with the numbers, is in **`TESTS.md`**. In brief:

| Check | Script | Result |
|---|---|---|
| Metal model against the analytic resonance of a cylindrical cavity (empty, with centre post) | `cavity.py` | conformal 7–13× closer than staircase (0.05 mm: −0.04 % vs −0.47 %) |
| Mesh convergence of a via with antipads | `via_convergence.py` | conformal at h ≈ staircase at 0.7·h; 0.035 mm: \|S21\| within 0.07 dB to 30 GHz |
| CPML on a matched microstrip | `check_pml.py` | converged at 8 cells; openEMS PML_8 reflects 8–14 dB more above 10 GHz |
| kisi-fdtd (staircase) against openEMS on the same mesh | `openems_compare.py` | Δ\|S21\| mean 0.02–0.22 dB |
| Lumped R–L–C | unit test `lumped_rlc` | series resonance within 2 % of the circuit |
| kisi's fast channel model on every case (mini KiCad boards) | `kisi_board.py`, `compare_kisi.py` | via ~15 % low, stub notch +17 %; a return-path bug found and fixed |
| Reduced via model (prototype) | `via_pbm.py` | phase ±5°, \|S21\| ±0.15 dB, TDR ±2–5 %, stub notch ±1–6 % up to 20 GHz |
| Plane-cavity model | `cavity_model.py` | resonances ±0.8 %, Z21 ±0.7 % |

Unit tests (`cargo test -p kisi-fdtd`) cover a microstrip, the stability of a conformal pad, the cavity resonance
and the lumped R–L–C.

## Reference cases
- `via_cases.py` writes the cases to `via_cases/` (kisi-fdtd case JSON plus a `meta` block). openEMS reads the
  same files.
  - **A** coax section: L1→L4 through a plane pair, 4 GND vias.
  - **B** stubs: through via, no stub, back-drilled; generic 8-layer and the ThunderScope stack.
  - **C** antipads, non-functional pads, an offset antipad.
  - **D** return path: one GND via at 0.5–5 mm or none; GND→PWR through a capacitor at 2 / 10 mm or none.
  - **E** differential pairs: pitch, round or oval antipads, 0 / 2 / 4 GND vias.
  - **G** plane cavity 50 × 30 mm.
  - **H** PDN loop: BGA pin field to a 0402 capacitor at 2–20 mm.
  - Plus a 2×thru per feed layer.
- `run_refs.py` runs them and writes `via_ref/<case>/`:
  - `kisi.sNp`: conformal, base mesh.
  - `kisi_fine.sNp` with `--fine`: refined mesh for the convergence check.
  - `openems.sNp` with `--openems`.
  - `meta.json`: cells, steps, run time and convergence.
- Ports are 50 Ω lumped ports at the ends of 4 mm feed traces (G and H use 1 Ω). Pairs use kisi's port order:
  1 = P near, 2 = N near, 3 = P far, 4 = N far.
- `sparam.py` provides Touchstone I/O, the full matrix from symmetric half-excitations, IEEE P370-style 2×thru
  de-embedding (NZC: fixture S11 from the gated 2×thru reflection, then S22 and S21 from the cascade), a
  first-order plane shift, mixed mode and TDR.
- `make_viewer.py` builds `dist/kisi-references.html`: geometry, S-parameters (at the ports and de-embedded, with
  kisi's fast model and the reduced via model), TDR, mixed mode, line parameters (T), Z (G, H), and the electric
  field of port 1 (top view in the thickest dielectric, animated; section through the via; collapsed by default).
- `rust_models.py` runs the Rust port of the reduced models (`kisi-validate models`) against the Python prototypes
  and the 3D results, with run times → `results/rust_models.json`.
- `compare_kisi.py` runs kisi's channel model on every case (`KISI_VIA_MODEL=coax` for the former via model) and
  compares the via alone de-embedded → `results/kisi_models.json`.
- `practice.py` measures run time, memory and bundle size on the example boards → `results/practice.json`.
- The planned measured test board is in `../test_board.md`.

```
cargo build --release -p kisi-fdtd
python3 validation/fdtd/via_cases.py validation/fdtd/via_cases
python3 validation/fdtd/run_refs.py validation/fdtd/via_cases validation/via_ref --exe target/release/kisi-fdtd --jobs 2
cargo build --release -p kisi-cli
python3 validation/fdtd/rust_models.py --exe target/release/kisi-validate
python3 validation/fdtd/compare_kisi.py target/release/kisi
python3 validation/fdtd/make_viewer.py
```

## Earlier feasibility test (Mur boundary, staircase, same mesh as openEMS)
Run on 21.10.2026, in `cases/` and `compare.py`:

| Case | ΔS21 mean / max | ΔS11 mean / max (where above −30 dB) |
|---|---|---|
| `thru_ms` | 0.005 / 0.02 dB | 0.12 / 0.3 dB |
| `via_l1_l4` | 0.10 / 0.67 dB | 0.59 / 3.4 dB |
| `via_l1_l4_stitched` | 0.02 / 0.27 dB | 1.1 / 4.6 dB |

Speed per time step in this workspace (Xeon 2.8 GHz, 2 vCPUs):

| Solver | Mcells/s |
|---|---|
| openEMS SSE, 1 thread | 94–101 |
| kisi-fdtd native | 80 |
| kisi-fdtd WebAssembly with SIMD128 | 42 |
| kisi-fdtd WebAssembly without SIMD | 18 |
