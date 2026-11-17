# Validation

What kisi's results are checked against, how well they agree today, and how to rerun each check. The user-facing
summary – which model, how accurate, what is an estimate – is [docs/models.md](../docs/models.md) (sections Accuracy
levels and Validation); the uncertainties kisi reports come from [data/accuracy.json](../data/accuracy.json), whose
sources point here.

| Part | Reference | Document | In CI |
|---|---|---|---|
| Cross-sections (impedance, εeff, loss) | CST Studio Suite 2D port modes and the full-wave FEM backend of js_2d_fields, 620 cases | [cross-sections.md](cross-sections.md), page `kisi-references-2d.html` | the page builds; the field solver against HFSS / its baseline (`tools/validate_field.sh`) |
| Vias, lines, pads, plane pairs | 3D FDTD (kisi-fdtd), every case also in openEMS and CST, 106 cases | [vias.md](vias.md), page `kisi-references-3d.html` | `tools/golden.py vias`; the page builds |
| Example boards | kisi's own stored output of ThunderScope and LibreVNA | [golden/README.md](golden/README.md) | `tools/golden.py boards` |
| Robustness | kisi on 32 (weekly) or about 200 (monthly) open-source boards | [corpus/README.md](corpus/README.md) | weekly and monthly |
| Channel export | ngspice on the exported circuits against kisi's S-parameters | `channel_export/compare.py`; results in [docs/research/channel-export.md](../docs/research/channel-export.md) | on changes of the export |

## Current accuracy

From `data/accuracy.json` (typical deviations from the references, median / 90th percentile where two are given):

| Model | Uncertainty | From |
|---|---|---|
| Impedance, field solver | 1 % (median 0.18 %, p90 0.47 %, max 1.03 % against CST; pairs' Zdiff 0.48 / 0.80 %) | cross-sections |
| Impedance, microstrip formula | 2 % | cross-sections, HFSS |
| Impedance, microstrip with coplanar copper | 2 % (median 0.9 %, p90 1.7 %, 104 cases) | cross-sections |
| Impedance, coupled pairs (formula) | 8 % | cross-sections |
| Impedance, stripline formula | 11 % (3D stripline T_ts6_sl +11 %) | cross-sections, vias |
| Impedance over a cut-out (formula, flagged "rough") | 25 % | cross-sections |
| Copper without a reference plane (coplanar estimate) | 15 % | vias (group P) |
| Via ladder, single vias (TDR impedance) | 2.43 / 5.13 % (29 vias); delay 0.21 ps | vias |
| Via ladder, pairs (Zdiff) | 4.02 / 5.65 % (12 pairs); delay 0.18 ps | vias |
| Coaxial via fallback (vias without a plane) | 8.0 / 25.5 % (scored against the earlier references) | vias |

Other checks with their numbers: line loss – kisi's R about 2 % below the full-wave FEM, α 1 % (cross-sections);
kisi's plane-cavity model – resonances within 0.8 % (vias); the channel export – ngspice against kisi within
0.03–0.34 dB up to 1.5 × Nyquist (docs/research/channel-export.md); DC IR drop and temperature – analytic cases and a
refined raster ([docs/models.md](../docs/models.md#validation)).

## Known limits

* Traces are rectangles: real etched trapezoids make kisi's Z0 2–4 % low for 35 µm copper (cross-sections).
* Asymmetric pairs: CST's eigenmode impedances differ from kisi's modal ones by definition, up to 51 % in Zdiff;
  kisi's values are consistent with its own definition (cross-sections).
* CST's conductor loss is 4–23 % low (a surface-impedance approximation); loss is judged against the full-wave FEM,
  itself checked by an independent eddy-current solution (cross-sections).
* Via exits with one plane close and the other far or missing (X_s8x4, B_ts6_L1L3_stub, X_s8x2): the ladder is too
  capacitive, TDR minimum 7.5–11 % low; above 12 GHz the ladder follows 3D only where its return posts close the
  via's cage (vias).
* The board vias and finite boards are not fully converged in any of the three 3D solvers (vias).
* The coaxial fallback has not been scored against the current references (vias).
* No measurements yet: a test board with 2.92 mm connectors is planned (branch `feat/test-board`).

## Reproduce

```sh
cargo build --release                                     # kisi, kisi-validate, kisi-fdtd
sh tools/fetch_examples.sh examples                       # the example boards
python3 tools/golden.py boards target/release/kisi examples   # stored results of the example boards
python3 tools/golden.py vias target/release/kisi          # kisi's channel on the 3D references (validation/fdtd/results/kisi_models.json)
python3 validation/fdtd/via_ladder.py --exe target/release/kisi-validate   # the via ladder alone against the three 3D solvers
sh tools/validate_field.sh                                # the field solver against HFSS and its baseline (needs node)
python3 validation/cst_xs/compare.py                      # the cross-section comparison from the stored results
python3 tools/corpus.py fetch && python3 tools/corpus.py compare target/release/kisi
python3 validation/channel_export/compare.py target/release/kisi examples/Thunderscope_Rev5.3.kicad_pcb \
  --config web/samples/ts.kisi.json --out /tmp/export --lane SYNC_OUT_P --strict   # needs ngspice
python3 validation/fdtd/make_viewer.py validation/via_ref dist/kisi-references-3d.html
python3 validation/cst_xs/make_viewer.py validation/cst_xs dist/kisi-references-2d.html
```

The 3D and 2D references themselves (kisi-fdtd, openEMS, CST, the full-wave backend) take hours to days and run on
GitHub Actions or with a CST licence; their results are committed, so every comparison above runs from the
repository. How to recompute them: [vias.md](vias.md#reproduce-and-extend), [cross-sections.md](cross-sections.md#reproduce).

| Folder | What |
|---|---|
| `fdtd/` | the 3D cases, the scripts that run and compare them, `results/` |
| `via_ref/<case>/` | per 3D case: kisi-fdtd (`kisi.sNp`, `kisi_base.sNp`), openEMS, CST, kisi's models (`kisi_model.sNp`, `ladder.s2p`, `model.s2p`), `meta.json`, `fields.json` |
| `cst_xs/` | the cross-section cases, the CST, full-wave and kisi results, the comparison |
| `golden/`, `corpus/` | stored results of the example boards and of the corpus run |
| `channel_export/` | the export check in ngspice and LTspice, the KiCad sheet check |
| `*.json` (here) | the field solver's anchors for `tools/validate_field.sh`: Forstén's test suite, ThunderScope cross-sections, asymmetric striplines, the baseline |
