# 3D FDTD: kisi-fdtd and the via references

`crates/kisi-fdtd` is a 3D FDTD solver (Yee scheme) for via transitions, plane cavities and PDN loops.
This folder holds its validation and the generator of the reference cases from `validation/via_references.md`.
The results are in `validation/via_ref/`. The reference page shows every case (geometry, S-parameters, TDR,
mixed mode, Z, field) and the solver checks. It is built, not kept in the repository:
`python3 validation/fdtd/make_viewer.py` → `dist/kisi-references.html` (self-contained; a release attaches it when the reference cases changed).

## Solver
- **Mesh:** rectilinear, non-uniform. Fixed lines at layer boundaries, sheet edges, vias, holes and ports, with
  spacing `dfine` there, growing by 1.4 up to λ/20 at fmax.
- **Metal:** zero-thickness sheets (traces, planes with round or slot-shaped antipads, pads, and polygon copper
  such as zone fills cut out of a board) and solid cylinders (via barrels), treated **conformally** (Dey–Mittra).
  - On every face that touches a partially free edge, the H update uses the free edge lengths. On faces cut by a
    round boundary it uses the free area.
  - The effective area A_eff = max(A_free, A · weighted free fraction of the edges). It keeps each face's local
    frequency at or below that of a full cell.
  - With every face treated this way, the update operator is similar to a symmetric positive semidefinite one, so
    it has real eigenvalues and runs stably at 0.95 of the Courant limit.
  - `"conformal": false` gives staircase metal, the same as openEMS.
  - Conformal metal only strictly inside the domain: in the PML and on its faces the geometry is the domain face
    extruded and stays staircased (the CPML terms use full-cell spacings; partial edges there made board cut-outs
    grow without bound). A Gershgorin bound of the operator guards the time step (never triggered so far).
- **Board cut-outs** (`BOARD_VIAS.md`): sheets with polygon rings (`polys`, even-odd, edges treated conformally),
  ports across traces that run along y (`x1`), mesh refinement boxes (`mesh.refine`), and no cell below
  0.7 · dfine (lines of nearby features merge; one sliver set the time step 7× too small).
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
| kisi-fdtd (staircase) against openEMS on the same mesh, one case each of A, B, D, E | `openems_compare.py` | Δ\|S21\| mean 0.011–0.028 dB |
| Every reference case (106) in openEMS: on kisi-fdtd's mesh (both staircase) and with the cell at metal halved | `openems_refs.py` | same mesh: Δ\|S21\| median 0.018 dB, \|Δτ\| 0.04 ps; references against openEMS refined: median 0.06 dB; outliers and their causes in `TESTS.md` section 16 |
| Every reference case (106) in CST Studio Suite (FIT, conformal metal) | `cst_case.py`, `run_cst.py`, `cst_compare.py` | lines, thrus, vias, pairs: \|S21\| median 0.03–0.09 dB per group, delay −0.42 … +0.26 ps against the current references; `CST.md` |
| Inner-layer exits on a finer mesh (21 references recomputed with the cell at metal halved, the four board vias on GitHub Actions) | `run_refs.py`, `fdtd_refs.py`, `openems_refs.py` | via cases: base → fine 1.1–2.3 ps faster on 14 of 17, afterwards kisi-fdtd − CST within ±0.07 ps; board vias −0.3 … +1.1 ps; `TESTS.md` section 17 |
| Lumped R–L–C | unit test `lumped_rlc` | series resonance within 2 % of the circuit |
| kisi's fast channel model on every case (mini KiCad boards) | `kisi_board.py`, `compare_kisi.py` | via ~15 % low, stub notch +17 %; a return-path bug found and fixed |
| Reduced via model (prototype) | `via_pbm.py` | phase ±5°, \|S21\| ±0.15 dB, TDR ±2–5 %, stub notch ±1–6 % up to 20 GHz |
| Plane-cavity model | `cavity_model.py` | resonances ±0.8 %, Z21 ±0.7 % |

Unit tests (`cargo test -p kisi-fdtd`) cover a microstrip, the stability of a conformal pad, the cavity resonance
and the lumped R–L–C.

## Reference cases
- `via_cases.py` writes the cases to `via_cases/` (kisi-fdtd case JSON plus a `meta` block). openEMS reads the
  same files (`openems_case.py`).
  - **A** coax section: L1→L4 through a plane pair, 4 GND vias.
  - **B** stubs: through via, no stub, back-drilled; generic 8-layer and the ThunderScope stack.
  - **C** antipads, non-functional pads, an offset antipad.
  - **D** return path: one GND via at 0.5–5 mm or none; GND→PWR through a capacitor at 2 / 10 mm or none.
  - **E** differential pairs: pitch, round or oval antipads, 0 / 2 / 4 GND vias.
  - **G** plane cavity 50 × 30 mm.
  - **H** PDN loop: BGA pin field to a 0402 capacitor at 2–20 mm.
  - Plus a 2×thru per feed layer.
- `run_refs.py` runs them and writes `via_ref/<case>/`:
  - `kisi.sNp`: conformal, the case's mesh: dfine, or the cell at metal halved where the case's `meta.ref_refine` asks
    for it (every transition that ends on an inner layer, the ThunderScope through via, the RG_ts6 cases:
    `TESTS.md` section 17); `--refine F` overrides it for a convergence study.
  - `kisi_base.sNp` (the cases of section 17): the former reference on the base mesh, kept for the comparison.
  - `kisi_fine.sNp` with `--fine`: refined mesh for the convergence check.
  - `meta.json`: cells, steps, run time and convergence (`mesh_convergence`: base → fine, fine → extra-fine).
- `openems_refs.py` computes every case in openEMS and compares (`TESTS.md` section 16):
  - `openems_base.sNp`: openEMS (staircase) on the mesh of the kisi-fdtd run, `openems.sNp`: with the cell at metal
    halved; their run info (cells, time per excited port, cell rate, runner) in `meta.json`.
  - `plan` balances the tasks (case × mesh × excited port) over shards (`openems_plan.json`), `run` runs one shard
    (the workflow `openems-refs` on GitHub Actions commits the raw results to `openems_raw/` on its branch; they are
    removed once merged), `merge` writes the Touchstone files and the run info, `stair` runs kisi-fdtd staircase (or `--mesh fine --metal conformal`) for the like-for-like
    check, `compare` writes `results/openems_refs.json`.
- Ports are 50 Ω lumped ports at the ends of 4 mm feed traces (G and H use 1 Ω). Pairs use kisi's port order:
  1 = P near, 2 = N near, 3 = P far, 4 = N far.
- `sparam.py` provides Touchstone I/O, the full matrix from symmetric half-excitations, IEEE P370-style 2×thru
  de-embedding (NZC: fixture S11 from the gated 2×thru reflection, then S22 and S21 from the cascade), a
  first-order plane shift, mixed mode and TDR.
- `make_viewer.py` builds `dist/kisi-references.html`: geometry, S-parameters (at the ports and de-embedded, with
  kisi's fast model and the reduced via model), TDR, mixed mode, line parameters (T), Z (G, H), and the electric
  field of port 1 (top view in the thickest dielectric, animated; section through the via; collapsed by default).
- `rust_models.py` runs the Rust port of the reduced models (`kisi-validate models`) against the Python prototypes
  and the 3D results, with run times → `results/rust_models.json`; kisi's ladder per case → `via_ref/<case>/ladder.s2p`.
  `kisi-validate via-geom geom.json [--s2p f.s2p]` evaluates one via geometry (the JSON that `KISI_DEBUG_VIA=1`
  prints from `kisi` or `kisi-validate models`).
- `fdtd_refs.py` runs kisi-fdtd references too large for one machine on GitHub Actions runners (workflow
  `fdtd-refs`, manual, never on main): `plan` cuts the cases into tasks (case × excited port) and shards
  (`fdtd_plan.json`), `run` runs a shard, `merge --keep-base` writes `via_ref/<case>/` from the raw results the
  workflow commits to its branch (`fdtd_raw/`, removed again after the merge). The board vias with an inner exit were
  computed this way (`TESTS.md` section 17).
- **CST** (`CST.md`): `cst_case.py` writes a case as CST VBA history blocks, `run_cst.py` runs cases in CST Studio
  Suite (with CST's Python libraries) → `via_ref/<case>/cst[_fine|_xfine|_mid].sNp` and `*_meta.json`,
  `cst_compare.py` compares with kisi-fdtd (also its former `kisi_base.sNp`) and openEMS → `results/cst_compare.json`
  and the tables of `CST.md` (`--md`). The reference page draws CST's finest mesh.
- `junction_fit.py` re-checks the two fitted choices of the ladder's junction model (`TESTS.md` sections 7 and 17)
  against the de-embedded references, on the ladder as `rust_models.py --out DIR` computes it.
- `compare_kisi.py` runs kisi's channel model on every case (`KISI_VIA_MODEL=coax` for the former via model) and
  compares the via alone de-embedded → `results/kisi_models.json`.
- `practice.py` measures run time, memory and bundle size on the example boards → `results/practice.json`.
- `wasm_speed.mjs` runs one case in the WebAssembly build of kisi-fdtd (`fd_run`, the engine a browser would use)
  and prints its run time and cell rate, for the comparison with the native solver.
- The planned measured test board is in branch `feat/test-board` (`validation/test_board.md` there).
- **Board vias** (`BOARD_VIAS.md`): `kisi-validate via-case` cuts a via (or the via pair of a differential pair)
  out of a KiCad board as a case with its 2×thru fixtures; `board_vias.py` exports, runs and compares three
  critical vias of the example boards with kisi's ladder (`board_cases/`, results in `via_ref/BV_*`,
  `results/board_vias*.json`, plots in `results/board_vias/`). Their lines are not 50 Ω in 3D, so the 2×thru
  de-embedding first refers each fixture to its own line impedance (`sparam.deembed_matched`; checked on a known
  network with `board_vias.py deembed-check`).
- **Plane resonances of finite boards** (`BOARD_RESONANCES.md`): `board_resonances.py` (cases `resonance_cases/`,
  results `via_ref/R_*`) against the ladder with finite cavities (`kisi-validate cavity-model`, option `--via-cavity`).
- **Return groups per plane pair** (`BOARD_RETURNS.md`): `board_returns.py` (cases `return_cases/`, results
  `via_ref/RG_*`, board vias with In2 grounded) against the ladder with return groups (`kisi-validate cavity-model
  --returns`, option `--via-returns`).

```
cargo build --release -p kisi-fdtd
python3 validation/fdtd/via_cases.py validation/fdtd/via_cases
python3 validation/fdtd/run_refs.py validation/fdtd/via_cases validation/via_ref --exe target/release/kisi-fdtd --jobs 2
cargo build --release -p kisi-cli
python3 validation/fdtd/rust_models.py --exe target/release/kisi-validate
python3 validation/fdtd/compare_kisi.py target/release/kisi
# openEMS (needs openEMS with its Python bindings, and shapely): on GitHub Actions, or locally per shard
python3 validation/fdtd/openems_refs.py plan --exe target/release/kisi-fdtd
python3 validation/fdtd/openems_refs.py run --shard 0 --exe target/release/kisi-fdtd
python3 validation/fdtd/openems_refs.py merge validation/fdtd/openems_raw
python3 validation/fdtd/openems_refs.py stair --exe target/release/kisi-fdtd --out /tmp/stair --jobs 2
python3 validation/fdtd/openems_refs.py compare --stair /tmp/stair
# kisi-fdtd on runners (board vias): plan, then Actions → fdtd-refs → Run workflow on the branch, then
python3 validation/fdtd/fdtd_refs.py plan BV_vna_rf_hblb1,BV_vna_clk_w5 --refine 2
python3 validation/fdtd/fdtd_refs.py merge --keep-base
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
