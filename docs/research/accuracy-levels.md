# Accuracy levels – design note

Status: design and a first version (25 October 2026, branch `feat/accuracy`): the impedance and the via
models carry their level and uncertainty in the CLI (`check`, `net`) and the viewer; loss, return loss and
the eye follow (ROADMAP block 2).

## Goal
ROADMAP block 2: *accuracy levels formula → 2D field solver → 3D for selected structures, each with an error
from the validation, and every result says which level it used and how uncertain it is.* kisi already marks
estimates (`assumed`, “(estimate)”); what is missing is a number for the models themselves: how far a
formula impedance, a via ladder or a channel's return loss typically is from a reference.

## Levels
| Level | What | Where it runs |
|---|---|---|
| formula | closed-form impedance, Wheeler/Huray/Djordjevic-Sarkar line models | everywhere (native CLI, viewer step 2) |
| 2D field solver | Forstén's solver per cross-section | `kisi.mjs` (default for nets with a target), viewer step 3 |
| model of its own | the via ladder (physics-based), its coaxial fallback, pads from their cross-section, the no-plane estimate | everywhere |
| 3D | kisi-fdtd | validation only today; the planned “3D check in the viewer” (BOARD_VIAS.md section 5.6) would add it per selected structure |

## Quantities and where the numbers come from
Every number is a *typical* deviation from kisi's own validation, kept with its source in
`data/accuracy.json` (values that change belong in `data/`):

| Quantity | Level | Uncertainty | Source |
|---|---|---|---|
| impedance of a cross-section | field solver | 2 % | Forstén's suite vs HFSS/Ansys 1.7 % (`validate_field.sh`); ThunderScope cross-sections ≤ 0.5 % |
| | formula, single microstrip | 2 % | docs/models.md table (+1.4 %), HFSS microstrip −0.6 % |
| | formula, coupled or coplanar | 8 % | diff microstrip +5.6 / +7.8 % |
| | formula, stripline | 11 % | asymmetric stripline +10.3 %, 3D stripline +11 % (`T_ts6_sl`) |
| | formula over a cut-out (flag “rough”) | 25 % | the 3313 launch over a 0.465 mm opening |
| | no plane (coplanar estimate) | 15 % | gold-finger cases, TESTS.md section 15 |
| impedance of a net / lane | the cross-sections' values, weighted by length | | |
| via | ladder, single | TDR error median 2.5 %, p90 5.9 % | `compare_kisi.py` (`results/kisi_models.json`) |
| | ladder, pair | Zdiff median 12.6 %, p90 14.4 % | same |
| | coaxial fallback | 8.0 / 25.5 % | `results/kisi_models_coax.json` |
| loss (IL) | from lines and vias | lines: RLGC within 2 % (HFSS); per via \|ΔS21\| median 0.10 dB up to 20 GHz | planned |
| return loss | from the impedance uncertainty | Δ\|Γ\| ≈ ε/2 per discontinuity → an RL range | planned |
| eye | from the channel | not quantified; the loss and RL ranges propagated through the eye | later |

The uncertainty of a net is the length-weighted mean of its cross-sections' (a net that is 90 % field-solved
microstrip and 10 % formula stripline: 0.9·2 + 0.1·11 = 2.9 %). The level is “field solver”, “formula” or
“mixed” by the share of the length the field solver covers.

## Output, without clutter
- **CLI JSON:** one object `accuracy` per `check` row and in `net`:
  `{"impedance": {"level", "uncertainty_pct", "field_share"}, "vias": {"model", "tdr_error_pct": {"median",
  "p90"}} | null, "note"}`; per net in the analysis (`NetStat.z_unc_pct`, `field_share`, the viewer's data).
  Later in the same object: `loss_db` (± dB at Nyquist) and `return_loss_db` (a range).
- **Viewer:** a muted “±x %” after the impedance in the check's criteria and the net's statistics, with the
  level and the source in its tooltip – the way estimates are marked today; no new panel.
- **Verdicts stay as they are:** a criterion is judged on the value, the uncertainty is shown beside it. A
  “close to the limit within the uncertainty” mark (yellow by uncertainty) was considered and left for later:
  it would move verdicts with every validation update.

## Next steps
1. Loss: ±(2 % of the line loss + the via count × 0.1 dB) at Nyquist; per section from the RLGC fits.
2. Return loss: the worst |S11| with each section's impedance at ±ε (two extra evaluations, or a first-order
   sensitivity) → `return_loss_db: [lo, hi]`.
3. The vias' and pads' uncertainty above 12 GHz (where the ladder follows 3D only with a closed cage).
4. The 3D level for selected structures (the viewer's 3D check) replaces the model's uncertainty by the
   mesh-convergence error of kisi-fdtd.
