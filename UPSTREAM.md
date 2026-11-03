# Field solver upstream: js_2d_fields

kisi uses the quasi-static 2D field solver of **Henrik Forstén**,
[github.com/Ttl/js_2d_fields](https://github.com/Ttl/js_2d_fields) (GPL-3.0),
**unmodified**. A copy of its files is vendored in `third_party/js_2d_fields` (no git submodule);
the upstream commit it was taken from is recorded in `third_party/UPSTREAM_REV`.

## How kisi talks to it

```
kisi-core (Rust)                         web/solver/backend_forsten.js        third_party/js_2d_fields
xsgeom.rs: cross-section -> text  ───►   solveGeometry(text, {tol})    ───►   CustomGeometrySolver(text)
extract.rs: Params.field <── results ◄── {z0, eeff, zodd, zeven, …}          .solve_adaptive()  (rectilinear FDM)
```

* **Interface = geometry text.** kisi writes each cross-section in upstream's custom
  geometry syntax (see `src/custom_geometry_text.js` upstream). This is the only coupling
  to upstream's input side.
* **One adapter file.** `web/solver/backend_forsten.js` is the only file that calls upstream
  code. Every backend implements the same contract:
  `solveGeometry(text, {tol}) -> {ok, z0?, eeff?, zodd?, zeven?, eeff_odd?, eeff_even?, ms, error?}`.
  Another solver (a Rust port, a BEM solver, …) is added as `web/solver/backend_<name>.js`
  and selected as `ENTRY` in `tools/solver_bundle.py` / imported in `tools/solve_xs.mjs`.
* **One replaced build artifact.** Upstream imports `src/wasm_solver/solver.js`, an emscripten
  build of Eigen that is not in its repository. kisi provides the same module shape in
  `web/solver/sparse_module.js`, backed by `crates/kisi-sparse` (Rust, faer). The import is
  redirected at bundle time (`tools/solver_bundle.py`) and in node (`tools/upstream_hooks.mjs`);
  no upstream file is changed.
* Only the rectilinear (quasi-static) backend is used in kisi. The triangular full-wave backend needs
  Gmsh/OpenCascade/Spectra builds and is not bundled; it runs for validation only (next section).

## Full-wave backend (validation only, not bundled)

Upstream's second backend, `src/tri_solver/` (triangular P2 FEM: static solve, Lee–Jin P2 Nédélec eigenmode above
100 MHz with a Robin ABC on radiating walls, MQS conductor loss, Djordjevic–Sarkar dielectrics), is run unmodified on
the CST cross-section cases (`validation/cst_xs/run_fullwave.py`, README there). It is not in kisi's bundles or CLI.

* **Adapter** `web/solver/backend_forsten_fw.js`: the same contract as `backend_forsten.js`
  (`solveGeometry(text, {tol, maxNodes, …})`, same geometry text), plus `solveSweep(text, freqs, opts)` (mesh at the
  highest frequency, then every frequency on the cached mesh: Z0, complex Zc, εeff, eigenmode εeff, R/L/G/C, αc, αd per
  mode; Zdiff/Zcomm and the 2×2 matrices of a pair). Z0 there is 1/(c0 C0 √εeff,mode) (dielectric dispersion, no
  conductor loss), εeff the phase εeff of the RLGC line.
* **WASM modules.** It imports `src/wasm_solver/eigen_solver.js` (Eigen + Spectra) and `gmsh.js` (gmsh + OpenCASCADE),
  which are not in upstream's git. `tools/build_forsten_fullwave.sh [BUILD_DIR]` builds both from upstream's sources
  at `third_party/UPSTREAM_REV` (fresh clone with its submodules, upstream's `Makefile`, `eigen.patch`, `gmsh.patch`,
  `link-occ-wasm.sh` as they are; it checks that the clone's `src/` equals `third_party/js_2d_fields/src`) into
  `BUILD_DIR/dist` (default `target/forsten-fw/dist`, with `BUILD_INFO.json`: versions, submodule commits, sha256,
  build times). Nothing under `third_party/` changes. The workflow `fullwave-xs` builds them on GitHub Actions with
  caches keyed on the pinned versions (without the cache 40 min on a 4-vCPU runner: OCCT 31 min, gmsh 7 min, eigen_solver
  30 s); the binaries are kept as workflow artifacts only, never committed.
* **Load path.** In node, `tools/upstream_hooks_fw.mjs` redirects the two imports to the build directory (registered
  next to `tools/upstream_hooks.mjs`, which keeps routing `wasm_solver/solver.js` of the FDM path to kisi-sparse;
  `tools/fullwave_node.mjs` registers both). emscripten loads each `.wasm` next to its `.js`.

| Component | Version (pinned in the build script) | Licence |
|---|---|---|
| js_2d_fields `src/tri_solver/` | `third_party/UPSTREAM_REV` | GPL-3.0 |
| Eigen (submodule `src/wasm_solver/eigen`) | upstream's submodule commit (fdfdd4c9 at this revision), `eigen.patch` | MPL-2.0 |
| Spectra (submodule `src/wasm_solver/spectra`) | upstream's submodule commit (3133be5c) | MPL-2.0 |
| gmsh (submodule `src/wasm_solver/gmsh`) | upstream's submodule commit (097c1518, 4.15 development), `gmsh.patch` | GPL-2.0-or-later (with gmsh's linking exceptions) |
| OpenCASCADE Technology | tag `V7_5_1`, modules trimmed as in upstream's README | LGPL-2.1 with the OCCT exception |
| emsdk / emscripten | 6.0.5 (6.0.6 requires `em++` for C++ links; upstream links with `emcc`) | MIT / UIUC |

All are compatible with kisi's GPL-3.0-only (gmsh "or later"; MPL-2.0 and LGPL-2.1 code may be combined with GPL
code); a bundled build would have to ship their licence texts and offer the corresponding sources (the build
script reproduces them from the pinned versions).

**Integrating it into kisi later** (not done): a backend `forsten-fem-fw` next to `forsten-fdm` in
`tools/solver_bundle.py` (`ENTRY`, two more `REDIRECT` entries) and the Node CLI; the two modules would be loaded
lazily, only when the full-wave backend is chosen, because of their size (this build: `gmsh.wasm` 12.0 MB,
`eigen_solver.wasm` 0.48 MB, the two JS loaders 0.1 MB, uncompressed; the whole viewer `kisi.html` is about 3.4 MB –
`tools/size_budget.json`). Options:
a separate file next to `kisi.html` / `kisi.mjs` fetched on demand, or a second bundle `kisi-fullwave.html`.
The geometry text and the result contract need no change; sweeps (RLGC(f), α) would replace kisi's own
frequency models of the line where the backend is chosen, which is a model change with its own validation
(`validation/cst_xs/compare.py`, loss sweep, with the independent MQS check `mqs_check.py`, is the start of it).

## Updating to a new upstream version

```sh
tools/upstream_update.sh            # latest upstream main
tools/upstream_update.sh <commit>   # a specific commit
```

The script copies the files of that upstream revision (`git archive`, unchanged) into
`third_party/js_2d_fields`, rebuilds, runs the field-solver validation
(HFSS/Ansys cases from upstream's own test suite and the ThunderScope cross-sections) and
compares against `validation/field_baseline.json`. It stops on any failure; commit
`third_party/js_2d_fields` and `third_party/UPSTREAM_REV` only when it passes.

What an update can break, and where to fix it:

| Upstream change | Fix in |
|---|---|
| `CustomGeometrySolver` options, `solve_adaptive` options or result fields | `web/solver/backend_forsten.js` |
| geometry text syntax | `crates/kisi-si/src/xsgeom.rs` |
| a new import of a build artifact (like `wasm_solver/solver.js`) | `REDIRECT` in `tools/solver_bundle.py`, `tools/upstream_hooks.mjs` |
| `solve_sparse_multi` signature | `crates/kisi-sparse`, `web/solver/sparse_module.js` |
| anything (full-wave, validation only) | rebuild with `tools/build_forsten_fullwave.sh` (the workflow's cache key follows `UPSTREAM_REV`); `web/solver/backend_forsten_fw.js` for its options/result fields; a new submodule or build flag in upstream's `src/wasm_solver/` goes into the build script |

## License

js_2d_fields is GPL-3.0; kisi is GPL-3.0-only so the combined work is consistent.
Copyright of the vendored code stays with Henrik Forstén.
