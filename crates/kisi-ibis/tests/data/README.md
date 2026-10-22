# IBIS test fixtures

None of these files describes a real part. Vendor IBIS models are not stored in this repository
(their licences rarely allow redistribution); test them locally with `kisi ibis <file.ibs>`.

| File | Source | Licence | What it covers |
|---|---|---|---|
| `synthetic_full.ibs` | written by `make_synthetic.py` (kisi) from analytic buffers | GPL-3.0-only (as kisi) | every keyword kisi-ibis reads and several it only reports; typ/min/max with NA; all scaling factors (T G M k m u n p f); keyword spellings with spaces, underscores and mixed case; a changed and restored comment character; multi-line text; a full matrix row continued over two lines. Drivers are linear, clamps ideal, edges raised cosines, so Ron, Roff, DC levels and [Ramp] are known exactly and the file passes all checks. Regenerate with `python3 make_synthetic.py`. |
| `synthetic_errors.ibs` | hand-written (kisi) | GPL-3.0-only | one example of each finding of the parser and the checks; the expected finding is noted at the end of each line. |
| `kicad/ibis_v1_1.ibs` | KiCad QA data, `qa/data/eeschema/ibis_v1_1.ibs` | GPL-3.0-or-later (part of KiCad) | IBIS 1.1, [Comment char] `#`, NA in [Package] and [Pin], Input/Output models, [Ramp] |
| `kicad/ibis_v4_1_r_series.ibs` | KiCad QA data, `qa/data/eeschema/ibis_v4_1_r_series.ibs` | GPL-3.0-or-later | Series and Series_switch models ([R/L/C/Rl/Rc/Lc Series], [Series Current], [Series MOSFET], [On]/[Off]), [Series Pin Mapping] |
| `kicad/ibis_v4_1_series_pin_mapping.ibs` | KiCad QA data, `qa/data/eeschema/ibis_v4_1_series_pin_mapping.ibs` | GPL-3.0-or-later | [Series Pin Mapping] naming a model the file does not define |
| `kicad/ibis_v5_1.ibs` | KiCad QA data, `qa/data/eeschema/ibis_v5_1.ibs` | GPL-3.0-or-later | IBIS 5.1: [Package Model] with sparse matrices, [Pin Mapping], [Model Selector], C_comp_pullup/_pulldown, [Model Spec], [Receiver Thresholds], waveforms with V_fixture_min/max, [Composite Current], [ISSO PU/PD], [Algorithmic Model], [Add Submodel] / [Submodel], NA in table rows |

The KiCad files are copied unchanged from https://github.com/KiCad/kicad-source-mirror at commit
874060a22610f13e10838573cf0b82d0c8658b4d (22 Oct 2026). They are marked "Artificial data … NOT a valid
component" by their authors. KiCad is licensed GPL-3.0-or-later, which is compatible with kisi's
GPL-3.0-only.
