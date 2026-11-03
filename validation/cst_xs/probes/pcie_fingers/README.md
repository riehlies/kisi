# Probe: PCIe edge fingers (board_ts_ea1df30155)

`board_ts_ea1df30155` (ThunderScope, B.Cu) is the PCIe edge connector: pads 0.7 × 4.2 mm at 1.0 mm pitch over
cut-out inner planes; the only plane copper under the pair is the opposite finger on F.Cu (0.71 mm, 1.4 mm
away), under one finger only. kisi's field solver (±1 V excitation) and CST's port eigenmodes disagree there
(Zdiff +50 %, Zeven −15 %). The same pair, made symmetric, in both solvers (CST 2025.5, electric walls,
levels 1–2, tol 0.3 %; kisi `field_E` in CST's box):

| variant | CST Zodd / Zeven | kisi Zodd / Zeven | Zodd | Zeven |
|---|---|---|---|---|
| `orig` (strip under sig+ only) | 37.0 / 113.5 | 55.7 / 96.5 | +50.7 % | −15.0 % |
| `sym` (strip under both fingers, tied to the wall) | 48.8 / 99.5 | 49.7 / 98.5 | +1.8 % | −1.0 % |
| `noplane` (no F.Cu copper) | 50.2 / 112.8 | 50.1 / 112.3 | −0.3 % | −0.5 % |
| `plane` (full F.Cu plane) | 48.6 / 98.2 | 49.9 / 98.0 | +2.6 % | −0.3 % |

The asymmetry causes the outlier (modal vs ±1 V definitions). Open: with plane copper 1.4 mm away the odd
mode differs by 2–3 % (CST lowers Zodd by 3.3 % from `noplane` to `plane`, kisi by 0.5 %); the ground ties
are not the cause (`plane` has none). Files: `cases.json` (the probe cases, `--cases`), `kisi.json`
(`run_kisi.py`), `probe_fing_*.json` (CST).
