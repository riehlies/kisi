# 3D FDTD: kisi-fdtd and the via references

`crates/kisi-fdtd` is a 3D FDTD solver (Yee scheme) for via transitions, plane cavities and PDN loops.
This folder holds its validation and the generator of the reference cases from `validation/via_references.md`.
The results are in `validation/via_ref/`. `references.html` shows every case (geometry, S-parameters, TDR,
mixed mode, Z) and the solver checks. It is a self-contained page; open it in a browser.

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

## Checks (`results/`, tab "Solver checks" in `references.html`)
| Check | Script | Result |
|---|---|---|
| Metal model against the analytic resonance of a cylindrical cavity (empty and with a centre post) | `cavity.py` | see `results/cavity.json` |
| Mesh convergence of a via with antipads, staircase and conformal | `via_convergence.py` | see `results/via_convergence.json` |
| CPML on a matched microstrip (8 / 16 cells, larger box, openEMS PML_8) | `check_pml.py` | converged at 8 cells; openEMS reflects 8–14 dB more above 10 GHz |
| Implementation: kisi-fdtd (staircase) against openEMS on the same mesh with PML | `openems_compare.py` | see `results/openems_compare.json` |
| Lumped R–L–C against a circuit model (series RLC at the end of a 1 mm microstrip) | unit test `lumped_rlc` | series resonance 4.35 GHz against 4.33 GHz, Re Z 9.6 Ω against 10 Ω |

Unit tests (`cargo test -p kisi-fdtd`) cover a microstrip (S11, S21), the stability of a conformal pad, the cavity
resonance at 0.1 mm cells (within 0.2 %) and the lumped R–L–C.

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
- `make_viewer.py` builds `references.html`.

```
cargo build --release -p kisi-fdtd
python3 validation/fdtd/via_cases.py validation/fdtd/via_cases
python3 validation/fdtd/run_refs.py validation/fdtd/via_cases validation/via_ref --exe target/release/kisi-fdtd --jobs 2
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
