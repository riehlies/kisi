# Field solver upstream: js_2d_fields

kisi uses the quasi-static 2D field solver of **Henrik Forstén**,
[github.com/Ttl/js_2d_fields](https://github.com/Ttl/js_2d_fields) (GPL-3.0),
**unmodified**. It is vendored as a git submodule in `third_party/js_2d_fields`;
the commit in use is the submodule pointer and is mirrored in `third_party/UPSTREAM_REV`.

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
  and selected as `ENTRY` in `tools/bundle.py` / imported in `tools/solve_xs.mjs`.
* **One replaced build artifact.** Upstream imports `src/wasm_solver/solver.js`, an emscripten
  build of Eigen that is not in its repository. kisi provides the same module shape in
  `web/solver/sparse_module.js`, backed by `crates/kisi-sparse` (Rust, faer). The import is
  redirected at bundle time (`tools/bundle.py`) and in node (`tools/upstream_hooks.mjs`);
  no upstream file is changed.
* Only the rectilinear (quasi-static) backend is used. The triangular full-wave backend needs
  Gmsh/OpenCascade/Spectra builds and is not bundled.

## Updating to a new upstream version

```sh
tools/upstream_update.sh            # latest upstream main
tools/upstream_update.sh <commit>   # a specific commit
```

The script checks out the revision, rebuilds, runs the field-solver validation
(HFSS/Ansys cases from upstream's own test suite and the ThunderScope cross-sections) and
compares against `validation/field_baseline.json`. It stops on any failure; commit the
submodule pointer and `third_party/UPSTREAM_REV` only when it passes.

What an update can break, and where to fix it:

| Upstream change | Fix in |
|---|---|
| `CustomGeometrySolver` options, `solve_adaptive` options or result fields | `web/solver/backend_forsten.js` |
| geometry text syntax | `crates/kisi-core/src/xsgeom.rs` |
| a new import of a build artifact (like `wasm_solver/solver.js`) | `REDIRECT` in `tools/bundle.py`, `tools/upstream_hooks.mjs` |
| `solve_sparse_multi` signature | `crates/kisi-sparse`, `web/solver/sparse_module.js` |

## License

js_2d_fields is GPL-3.0; kisi is GPL-3.0-only so the combined work is consistent.
Copyright of the vendored code stays with Henrik Forstén.
