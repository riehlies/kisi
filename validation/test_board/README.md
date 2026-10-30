# Measurement coupons (test boards)

Status: **design v3, prepared, not ordered.** 23 boards are generated (first order: JLC 4 / 6 / 8 layers with 7628
prepreg plus one Aisler board; second order: JLC 4 / 6 layers with 3313 prepreg). kisi reads all of them, their
impedances are on target with kisi's field solver, the PDN and capacitor coupons have a measurement method and a
component list, and the 3D cases are written (**not run**). The connector is the 2.92 mm Samtec
`292-J-P-EP-ST-EL-01` (40 GHz); its footprint is generated from Samtec's drawing and is a **DRAFT**, and the launch
inside it is parametric and **not yet optimised** (section 2). **DRC:** KiCad 7 (the workspace has no KiCad 8) on
copies converted to KiCad 7 syntax, zones refilled: no errors, no unconnected items (section 13); the maintainer
reruns it in KiCad 8 / 9. The plan behind this is `../test_board.md`.

Goal: close the loop simulation → measurement for kisi's fast models and the 3D references (`../via_ref`). The same
coupon geometry is measured, computed with kisi-fdtd (cases generated from the same description as the KiCad board)
and with kisi (the board file itself), and all three are de-embedded the same way (IEEE P370-style 2×thru,
`../fdtd/sparam.py`).

What changed in v3 (2026-10-30): the 2.92 mm Samtec connector instead of SMA and a 40 GHz frequency plan
(section 2); the footprint generated from Samtec's drawing with plated screw holes and ground pads on top and bottom;
a parametric launch (tip, taper, pad, coplanar section, ground cut, fence) and its kisi-fdtd cases
(`fdtd/launch_<stackup>/`, `make_coupons.py --launch-cases`); the GND via walls between rows at 1.27 mm instead of
2.54 mm (closed to 40 GHz). The coupons, widths and stackups are unchanged.

What changed in v2 from v1 (commit d7a7ec8): SMA connector and a 26.5 GHz frequency plan; boards split to the JLC price
brackets; every board has its own 2×thru and all connectors are on the top side (via coupons are via–line–via);
an 8-layer set; the thin-prepreg set; PDN coupons (plane cavity, pin field with capacitors) and capacitor coupons
with a component list; a pin-wide launch with a plane opening for the thin prepreg. The single v1 3D result
(`fdtd_ref/`) belonged to the old geometry and is removed.

## 1. Files

| Path | Content |
|---|---|
| `make_coupons.py` | generator: boards, kisi configuration, 3D cases; trace widths from kisi |
| `stackups.json` | stackups with values and sources (JLC 4 / 6 / 8 layers, Aisler assumed, thin prepreg) |
| `connectors.json` | Samtec 2.92 mm edge launch with sources and the drawing's dimensions, footprint parameters (**DRAFT**), launch parameters (to be optimised), layout pitches, via sizes, capacitor land patterns, size limits |
| `boards/<board>.kicad_pcb`, `.kicad_pro` | KiCad 8 boards (KiCad 9 opens them); net classes `50Z`, `100Z_Diff` |
| `boards/<board>.kisi.json` | kisi configuration: dielectric values with sources, class targets and profiles |
| `boards/<board>.manifest.json` | coupons, nets, connectors (J numbers, port order), parts (C/U refs), 3D cases and their cell counts |
| `boards/<stackup>.design.json` | trace widths and the impedance table (kisi formula and field solver) |
| `boards/<board>.kisi_check.json`, `kisi_check.md` | kisi run on every board (section 9) |
| `fdtd/<board key>/*.json` | kisi-fdtd cases, format of `../fdtd/via_cases.py`; run with `../fdtd/run_refs.py` |
| `fdtd/launch_<stackup>/*.json` | launch optimisation cases (section 2) |
| `fdtd_ref/launch_<stackup>/<case>/` | kisi-fdtd results of the launch cases (`kisi.s2p`, `meta.json`; section 2) |
| `check_coupons.py` | runs kisi on the boards → `kisi_check.md` |
| `compare_measured.py` | measured vs kisi vs kisi-fdtd (section 12) |

## 2. Connector, launch and frequency plan

### Connector

- **Samtec `292-J-P-EP-ST-EL-01`** (decided 2026-10-30, on the RF group's advice): 50 Ω 2.92 mm (K) jack, edge
  launch, solderless compression mount (field replaceable), performance to **40 GHz**, PCB thickness .040–.100 in
  (1.02–2.54 mm, option `-01`: every board fits, the 2.05 mm 8-layer board included), two #1-72 socket-head screws
  with #1 split washers (1/16 in hex key), **0.5–0.8 in-lb**; intermates with 3.5 mm and SMA. Why: orderable at
  DigiKey (about 6 weeks) and Mouser, solderless and reusable. Price 52.65 USD for one (Samtec, 2026-10-30). Needed:
  **4** (2 per 2-port coupon, 4 for the 4-port coupons; they move between coupons), a fifth as a spare. Sources and
  the drawing's dimensions: `connectors.json` (`connector`).
- Data from Samtec (supplied by the maintainer 2026-10-30, **not in the repository**: the terms of the SnapMagic
  footprint and the CADENAS STEP model forbid redistributing the models; the drawing is proprietary): drawing
  292-J-P-EP-ST-EL-01-X-MKT rev C, SnapMagic KiCad footprint and symbol, CADENAS STEP model, RF Characterization
  Report (2023-06-01). Samtec has no CST / HFSS model of the part.
- From the drawing and the STEP model: launch pin Ø0.18 mm (.007 in), 0.74 mm beyond the reference plane (the board
  edge), its axis 0.13 mm above the board, in a Ø0.70 mm opening of the body's front lip; the top jaw covers the
  board 5.08 mm deep and 9.53 mm wide, with a slot over the trace (2.67 mm wide from 0.38 mm, 3.18 mm from 0.64 mm,
  open to the top); the lower plate clamps the bottom side; screws 2.79 mm from the edge, ±2.92 mm.

### Footprint (generated, DRAFT)

`make_coupons.py` writes the footprint `kisi_coupons:Samtec_292-J-P-EP-ST-EL-01_EdgeLaunch` from the parameters in
`connectors.json` (`footprint`); its geometry follows Samtec's drawing (sheet 2, "recommended footprint"):

| Feature | Drawing | Footprint |
|---|---|---|
| Screw holes | 2X Ø.0810 in, 2X .1100 in from the edge, 2X .230 in apart | Ø2.06 mm plated, GND, 2.79 mm from the edge, ±2.92 mm; Ø2.7 mm pad on every copper layer |
| Ground pads | .2550 in deep, .414 in overall, inner edges .114 in apart, 45° chamfers from .205 in to .314 in, fingers .025 in deep, .0220 in apart at the edge | the same, on the top and bottom copper (where the jaws clamp; Samtec's model has them on every layer: our planes do that inside, and PWR islands stay clear), mask open both sides (+0.05 mm) |
| Signal pad | .0205 in wide (0.52 mm), tip .0090 in (0.23 mm) | our launch (below): tip 0.23 mm tapering to the launch width over 0.85 mm (taper length from the SnapMagic model) |
| Edge | copper to the edge | pulled back 0.2 mm (JLC rule): the pin still rests 0.54 mm on the tip, the lip 0.18 mm on the fingers |
| Body, courtyard | body .375 in wide | Fab: body 9.53 mm wide, 5.08 mm on the board, 11.57 mm outside; courtyard ±5.51 mm, 6.73 mm on the board |

The generator checks that no copper of another net lies under the ground pads (top / bottom) and that PWR islands
keep clear of the screw holes. The finger contour (0.84 mm half-width at 0.64 mm, 0.35 mm at 0.45 mm) and the taper
length come from the SnapMagic footprint, not from dimensions on the drawing.

### Launch (parametric, not yet optimised)

Samtec's general signal pad (0.52 mm wide, 1.19 mm from the ground pads, 6.48 mm long) is 43.5 Ω on the 0.21 mm 7628
prepreg (kisi field solver): Samtec states the footprint is not optimised for electrical performance. Our launch
inside the same ground pads (`connectors.json` `launch`, `null` = the stackup's designed launch):

| Parameter | Default (7628 / 3313) | Meaning |
|---|---|---|
| `tip_w`, `taper_len` | 0.23 mm, 0.85 mm | the tip at the edge (the pin rests on it) and its taper to the pad width |
| `pad_w`, `pad_len`, `pad_gap` | = launch: 0.315 mm, no extra pad, gap 0.3 mm / 0.255 mm, gap 0.105 mm | an optional wider pad after the taper (Samtec: 0.52 mm × 6.48 mm, gap 1.19 mm) |
| `w`, `gap` (design) | 0.315 / 0.3 mm (50.0 Ω) / 0.255 / 0.105 mm with the In1 opening (50.05 Ω) | the grounded coplanar launch |
| `launch_length` | 6.48 mm | to the end of the ground pads, then the coupon's 50 Ω line |
| `gnd_cut` | none / the design's In1 opening (0.465 mm) over the whole launch | an opening in the first plane under the tip / pad |
| `fence` | 0.8 mm pitch from 0.8 mm, two rows 0.4 / 1.2 mm off the gap, none within 0.45 mm of a screw hole | GND vias into all planes |

The top ground is the union of the Samtec ground pads and the coplanar pours. The outer fence row (1.66 mm off the
centre) lies in the ground pads, where the mask is open and the jaw edge (slot wall at 1.59 mm) presses: open via
holes in the contact area are harmless electrically; if the jaw should sit on flat copper, order plugged vias or move
that row inside 1.45 mm (a fence parameter). Change the values and run
`make_coupons.py --reuse-design` (the boards, manifests and 3D cases are rewritten in seconds).

### Launch optimisation (kisi-fdtd)

`make_coupons.py --launch-cases jlc4_7628 [--fdtd-exe …]` writes `fdtd/launch_jlc4_7628/`: the board from its edge to
the end of the launch plus 2.5 mm of the 50 Ω microstrip, the ground pads top and bottom, the plated screw holes,
the fence, the mask, port 1 a lumped 50 Ω port at the edge (tip to In1; under a ground cut In1 keeps a 0.25 mm strip
at the edge), port 2 on the microstrip. The 7628 top prepreg is the same on the 4-, 6- and 8-layer boards, so one
stackup serves the first order (the planes below In1 differ; check the result on the 6- and 8-layer stacks).

| Case | What | Cells |
|---|---|---|
| `ours` | the default launch | 0.82 M |
| `ours_body` | the same with the connector's top jaw (metal sheets at four heights with its slot, posts along the slot walls and the lip, the screws through it) | 2.12 M |
| `notaper` | no tip: the launch width to the edge | 0.82 M |
| `samtec` | Samtec's general pad (0.52 mm, gap 1.19 mm, 6.48 mm) | 0.89 M |
| `samtec_cut` | the same with In1 cut out under the pad (0.52 mm) | 2.18 M |

- **Not modelled:** the connector's coax and the pin (kisi-fdtd has z-directed cylinders and horizontal sheets only):
  the edge port stands in for them, so the cases optimise the board side. CST with Samtec's STEP model and a
  waveguide port on the 2.92 mm interface checks the whole transition (open point).
- **Target:** |S11| < −20 dB to 40 GHz for one launch (Samtec's own optimised fixture on MT40: −23 to −25 dB for one
  connector with 500 mil of trace, report 2023-06-01).
- **Sweep plan** (after the first runs): one parameter at a time around the best case, about 20 runs –
  `tip_w` 0.18–0.35 mm, `taper_len` 0.4–1.5 mm, the coplanar `gap` 0.2–0.4 mm with `w` re-solved for 50 Ω, a cut in
  In1 under the tip (0.3–0.8 mm wide, 0.3–1.0 mm long), the first fence column 0.4–0.8 mm and pitch 0.5–0.8 mm, then a
  2-D refinement of the two most sensitive parameters, with and without the jaw. Time-gated S11 (TDR) locates the
  discontinuity. The thin-prepreg (3313) launch follows the same way (its own cases: `--launch-cases jlc4_3313`).
- **Run time:** 0.9 M cells, 23 000–42 000 steps: 280–680 s per case on this workspace (2 vCPUs shared with other
  jobs, about half a core each); the jaw cases (2.1–2.2 M cells) about three times that. A sweep of 20 cases: 2–4 h.
- **First runs (2026-10-30, `fdtd_ref/launch_jlc4_7628/`):**

  | Case | max \|S11\| 0.5–40 GHz | \|S11\| at 10 / 20 / 30 / 40 GHz | \|S21\| at 40 GHz (≈ 9 mm) |
  |---|---|---|---|
  | `notaper` (0.315 / 0.3 mm to the edge) | −18.9 dB at 36.8 GHz | −29.9 / −28.4 / −29.1 / −22.2 dB | −0.84 dB |
  | `ours` (the default: 0.23 mm tip) | −15.7 dB at 37.2 GHz | −38.0 / −30.5 / −19.9 / −15.7 dB | −0.93 dB |
  | `samtec` (0.52 mm pad, 1.19 mm gap) | −14.6 dB at 24.8 GHz | −26.6 / −19.7 / −19.5 / −22.0 dB | −1.19 dB |

  Reading: our 50 Ω coplanar launch inside the Samtec pads is close to the target (−20 dB to 35 GHz, −18.9 dB at the
  top); Samtec's general pad is not usable on the 0.21 mm prepreg (−14.6 dB, and its sparse fence – the rows move out
  to the screw holes – lets the plane-pair mode carry power away: \|S21\| −2 dB already at 1 GHz). Without the pin in
  the model the 0.23 mm tip only adds inductance (worse above 30 GHz); the pin over the tip adds the capacitance the tip
  compensates, so the tip stays the default until a case with the pin decides. All three show narrow \|S21\| dips at
  25.5, 27, 35.5 and 37.5 GHz: resonances of the fence / screw-hole region, the first thing to sweep (fence pitch,
  vias between the holes). Values below about 2 GHz are limited by the edge port (−24 dB at 1 GHz on every case).
  `ours_body` (the jaw) ran twice and is **not valid yet**: port 1 sees a near short (\|S11\| about 0 dB at low
  frequency, \|S21\| −30 dB at 1 GHz rising to −2.7 dB at 40 GHz), also after moving the jaw's underside to the
  copper height and its top off the domain boundary. The jaw approximation (sheets with the slot, the lip next to the
  edge port, posts) needs debugging before its numbers mean anything. Next: fix that case, the pin as a strip over the
  tip, then the sweep.

### Frequency plan (2.92 mm, 40 GHz)

kisi's stub resonance for the stubbed vias (`kisi_check.md`):

| Coupon | Stub | kisi stub resonance | In the 40 GHz band? | v3 |
|---|---|---|---|---|
| 6 layers VLV_L3, SL_L3 (L1 → In2, stub to L6) | 0.88 mm (3313: 0.82) | 32.4 GHz (34.5) | **yes** (v2: no) | the notch is now measured on the 6-layer board too (PNA-X, PNA) |
| 6 layers DIFF_SL, BS_L3L4 (inner exits) | 0.65–0.88 mm | 24.5–38 GHz | yes (v2: partly) | coupling coupons, read in the time domain |
| 8 layers VLV_L3, SL_L3 (L1 → In2, stub to L8, 2.05 mm board) | 1.42 mm | 19.8 GHz | yes | inside the band of every VNA, the 20 GHz ENA included |
| 8 layers E_L1L3 (pair, stub to L8) | 1.42 mm | 22.1 GHz | yes | differential stub notch |
| 8 layers VLV_L6 (L1 → In5, stub to L8) | 0.60 mm | 38.5 GHz | **yes**, at the top (v2: no) | short-stub reference; its notch now just inside the band |

- **2.0 mm 8-layer board:** with 40 GHz, a 1.6 mm 8-layer board (about 1.1 mm of stub, notch near 25 GHz) would now be
  inside the band too; the 2.0 mm board stays (a lower, deeper notch that the 20 GHz ENA sees as well; within the
  connector's 2.54 mm limit). Not changed.
- **Row walls:** at 2.54 mm the GND via walls between rows open near 28 GHz (pitch = λ/2 in FR-4); v3 uses 1.27 mm
  (λ/2 near 56 GHz). The launch fence (0.8 mm, below λ/4 at 40 GHz) and the CPWG fence (1.0 mm, below λ/2) stay.
- **TRL:** LINE_27 (ΔL 2 mm) reaches 160° near 37 GHz; the multiline set (ΔL 2 / 5 / 10 / 25 / 75 mm) stays usable to
  40 GHz with LINE_30 and LINE_50.
- **3D cases:** already computed to 40 GHz (section 7), unchanged.
- Everything else (lines, through vias, return path, pairs without stubs, coupling) has no stub resonance below
  40 GHz; the plane cavities resonate at 1.4–3.2 GHz (measured to 8 GHz).

## 3. Stackups

Values fetched on 2026-10-22; sources in `stackups.json` (key `sources`): thicknesses from JLC's impedance-template
data ([gsuberland/jlcpcb_autogenerated_stackups](https://github.com/gsuberland/jlcpcb_autogenerated_stackups), commit
9c8b685), 7628 / 3313 / 2116 εr from [JLC capabilities](https://jlcpcb.com/capabilities/pcb-capabilities), core εr
4.6 from [JITX's JLC materials](https://docs.jitx.com/en/latest/_modules/jitxlib/jlcpcb/materials.html), tan δ 0.02
assumed (not published), mask εr 3.8 and 1.2 mil from the
[JLC impedance calculator guide](https://jlcpcb.com/help/article/user-guide-to-the-jlcpcb-impedance-calculator).

| Stackup | Order | Dielectrics L1 → bottom (mm) | Copper | Planes / signal layers |
|---|---|---|---|---|
| **JLC04161H-7628** | first | 7628 0.2104 · core 1.065 · 7628 0.2104; 1.586 total | 1 oz outer, 0.5 oz inner | GND In1, In2 (PDN: PWR islands on In2) / L1, L4 |
| **JLC06161H-7628** | first | 7628 0.2104 · core 0.4 · 7628 0.2104 · core 0.4 · 7628 0.2104; 1.562 | same | In1, In4 / L1, In2, In3, L6 |
| **JLC08201H-7628** | first | 7628 0.2104 · core 0.35 · 7628 · core 0.35 · 7628 · core 0.35 · 7628; 2.053 | same | In1, In3, In4, In6 (PDN: PWR islands on In3 over In4, 0.35 mm) / L1, In2, In5, L8 |
| Aisler 4 layers 1.6 mm (**assumed**) | first | prepreg 0.2 · core 1.2 · prepreg 0.2 (εr 4.3 / 4.5 / 4.3) | 35 µm | as JLC 4 layers |
| **JLC04161H-3313** | second | 3313 0.0994 · core 1.265 · 3313 0.0994; 1.564 | 1 oz / 0.5 oz | as JLC 4 layers |
| **JLC06161H-3313** | second | 3313 0.0994 · core 0.55 · 2116 0.1164 · core 0.55 · 3313 0.0994; 1.546 | same | as JLC 6 layers |
| JLC08161H-2116 | – | in `stackups.json`, not generated (1.6 mm 8 layers: stub only about 1.1 mm) | | |

The 7628 top prepreg is the same on the 4-, 6- and 8-layer boards (and on LibreVNA), so the launch and the L1 lines
are identical on all first-order JLC boards: a build-to-build check. The Aisler stack is the generic `s4` of the
3D references until Aisler sales give the real one; its line and cavity coupons extract it.

## 4. Boards

Limits (maintainer, 2026-10-22, JLC price brackets): 4 layers ≤ 100 × 100 mm, 6 and 8 layers ≤ 50 × 50 mm; the
generator refuses a JLC board that exceeds them. Aisler charges by area: one dense board of 35 mm rows.
Rows are sorted by length (a stepped outline); a row is one coupon from edge to edge, or two coupons side by side
with their ports on the outer edges (PDN coupons). 2-port rows are 12 mm, 4-port rows 24 mm (two connectors per
edge, 12.7 mm apart). GND via walls (1.27 mm, through all layers; v2: 2.54 mm) run along every row boundary and along the
exposed step edges. Through vias only, drill 0.3 / pad 0.6 mm, inner non-functional pads removed. Antipads, island
clearances and the launch openings are KiCad keep-out rule areas, so a refill reproduces them. Bottom silk: board
name, template, date and JLC's order-number placeholder `JLCJLCJLCJLC`.

| Board | Layers | Size (mm) | Coupons |
|---|---|---|---|
| **kisi-coupons-jlc4-7628-a** | 4 | 100 × 96 | THRU, LINE_27, LINE_30, LINE_50, LINE_100, REFLECT, CPWG_25, CPWG_50 |
| **kisi-coupons-jlc4-7628-b** | 4 | 35 × 96 | THRU, A0800, A1000, A1200, D_S1p0, D_S2p0, D_S3p5, D_NONE |
| **kisi-coupons-jlc4-7628-c** | 4 | 35 × 96 | PAIR, E_R_G2, E_O_G2, E_R_G0 |
| **kisi-coupons-jlc4-7628-d** | 4 | 50 × 96 | THRU, CAPZ_0402, CAPZ_0603, CAPZ_0805, XT_MS, DIFF_MS |
| **kisi-coupons-jlc4-7628-p** | 4 | 100 × 96 | THRU, G_CAV, H_D2, H_D5, H_D10, H_D20 |
| **kisi-coupons-jlc6-7628-a** | 6 | 35 × 48 | THRU, B_L1L6, D_S1, D_NONE |
| **kisi-coupons-jlc6-7628-b** | 6 | 50 × 48 | THRU, VLV_L3, SL_L3, LINE_50 |
| **kisi-coupons-jlc6-7628-c** | 6 | 35 × 48 | PAIR, E_R_G2 |
| **kisi-coupons-jlc6-7628-d** | 6 | 50 × 48 | BS_L3L4, DIFF_SL (no 2×thru: section 5) |
| **kisi-coupons-jlc8-7628-a** | 8 | 50 × 48 | THRU, VLV_L3, B_L1L8, SL_L3 |
| **kisi-coupons-jlc8-7628-b** | 8 | 35 × 48 | THRU, VLV_L6, D_S1, D_NONE |
| **kisi-coupons-jlc8-7628-c** | 8 | 35 × 48 | PAIR, E_L1L8 |
| **kisi-coupons-jlc8-7628-d** | 8 | 35 × 48 | PAIR, E_L1L3 |
| **kisi-coupons-jlc8-7628-p** | 8 | 50 × 48 | H_D2, H_D5, H_D10, H_D2_0603 (no 2×thru: section 5) |
| **kisi-coupons-aisler4-assumed-a** | 4 | 35 × 144 | THRU, LINE_27, LINE_30, LINE_35, A0800, A1000, A1200, D_S1p0, D_S2p0, D_NONE, G_SMALL |
| kisi-coupons-jlc4-3313-a … -d | 4 | as jlc4-7628-a … -d | the same coupons on the thin prepreg (CAPZ: the mounting inductance with 0.1 mm to the plane) |
| kisi-coupons-jlc6-3313-a … -d | 6 | as jlc6-7628-a … -d | the same coupons on the thin prepreg |

First order: 15 boards (14 JLC + 1 Aisler); second order: 8 JLC boards.

### Coupons

| Coupon | Length (mm) | What | Validates in kisi | 3D reference | 3D window |
|---|---|---|---|---|---|
| THRU | 25 | 2×thru on L1, reference planes 12.5 mm from each edge | fixture of every 2-port coupon on the board; TRL thru | thru_s4_L1 | w10 |
| LINE_27 / 30 / 35 / 50 / 100 | 27–100 | 50 Ω microstrip, ΔL 2 / 5 / 10 / 25 / 75 mm against THRU | εeff, loss per mm, Zc (line–line); multiline TRL | T_ts6_ms | LINE_50 w10 / w20 |
| REFLECT | 30 | two open ends at the reference planes | TRL reflect | – | – |
| CPWG_25, CPWG_50 | 25, 50 | grounded CPW on L1, gap 0.18 mm (LibreVNA), fence 1.0 mm | CPWG Zc and εeff (formula εeff +5 % vs 3D) | T_vna6_cpwg | w10, w20 |
| A0800 / A1000 / A1200 | 30 | via–line–via L1 → L4 → L1 through the plane pair, antipad Ø0.8 / 1.0 / 1.2, 4 GND vias at 1 mm | via ladder: antipad dependence of Z and delay | A_s4_d300_ap800 / ap1000 | wv |
| D_S1p0 / S2p0 / S3p5 / NONE | 35 | L1 (ref In1) → L4 (ref In2) → L1, one GND via at 1 / 2 / 3.5 mm from each via, none | return path of a reference change (GND vias within 3 mm enter the ladder, up to 5 mm an inductance, beyond "missing") | D_s8_stitch_* | wv |
| B_L1L6, B_L1L8 | 30 | through via, no stub (via–line–via on the bottom layer) | via ladder through several planes | B_ts6_L1L6, B_s8_L1L8_d300 | wv |
| VLV_L3 (6, 8 layers), VLV_L6 (8) | 30 | via–line–via to an inner layer, stub to the bottom | junction model, stub capacitance / notch (section 2) | B_*_L1L3_stub, X_s8x2_L1L3_bd | wv |
| SL_L3 | 50 | the same with 25 mm stripline | stripline γ (line–line with VLV_L3: the vias cancel), Zc from the TDR plateau | T_ts6_sl | w10, w20 |
| PAIR | 30 | pair 2×thru on L1 at the 1.0 mm via pitch (100 Ω diff.), 15 mm per side | fixture of the pair coupons on the board | – | w6 |
| E_R_G2 / E_O_G2 / E_R_G0 / E_L1L8 / E_L1L3 | 35 | pair via–line–via, pitch 1.0, round / oval antipads, 2 / 0 GND vias; E_L1L3 with stubs | pair ladder: Sdd21, Zdiff, Scd21, oval antipad, differential stub | E_s8_L1L8_p1000_*, E_s8_L1L3 | wv |
| XT_MS | 50 | 50 Ω aggressor and victim on L1, edge gap = w | NEXT / FEXT (kisi's NEXT 70–80 % high vs 3D) | T_ts6_xt_ms_s10 | wc |
| DIFF_MS, DIFF_SL | 50 | 100 Ω edge-coupled pair on L1 / In2 | Zdiff (TDR plateau), odd-mode εeff | T_ts6_diff_ms / _sl | wc |
| BS_L3L4 | 50 | broadside In2 over In3 | broadside NEXT / FEXT (kisi lists, does not compute yet) | T_ts6_broadside_o0 | wc |
| CAPZ_0402 / 0603 / 0805 | 25 | capacitor footprint in shunt on the 50 Ω line, two GND vias | mounted C, ESR, ESL, mounting inductance, pad capacitance (section 6) | – | w10 (0 Ω) |
| G_CAV, G_SMALL | 100, 35 | PWR island 50 × 30 (Aisler 25 × 18) mm on In2 under In1 (JLC d = 1.065 mm, Aisler assumed 1.2 mm), ports at a corner and the centre | cavity resonances (kisi cavity model ±0.8 % vs 3D), core εr, plane capacitance | G_pp100/200 | wg |
| H_D2 / D5 / D10 / D20, H_D2_0603 | 50 (4 layers), 25 (8) | pin field + one capacitor at 2 / 5 / 10 / 20 mm, PWR island, two shunt-thru ports (section 6) | capacitor loop inductance and Z(f) at the IC (`power --pdn`) | H_s8_cap_* | wh |

The trace widths of every coupon come from `boards/<stackup>.design.json` (section 8). The G coupon's first modes
(magnetic walls, εr 4.6): 1.40 (10), 2.33 (01), 2.72 (11), 2.80 (20) GHz; kisi's cavity model gives 1.398 GHz.

## 5. De-embedding across boards

Every separately ordered board is a separate panel and process run: εr, pressed prepreg thickness, etched width and
mask thickness differ from run to run (JLC: impedance ±10 %, thickness ±10 %, track width ±20 %). A 2×thru taken from
another board carries that difference into the de-embedded coupon: a 5 % impedance difference of the 12.5 mm feeds
leaves a reflection of about −32 dB and a delay error of a few ps per side, which is the size of the effects the line
and via comparisons look for. **Decision: every board carries its own 2×thru** (THRU; PAIR on boards with 4-port
coupons), and all via coupons are **via–line–via** structures fed from L1 at both ends, so that one L1 fixture per
board serves them all and every connector sits on the top side (no bottom-side 2×thru, no connectors mounted upside
down).

Trade-off of via–line–via: each coupon has two vias. The line between them (5 mm, D group 10 mm) separates them in
the TDR (about 60 / 120 ps round trip against a 25–35 ps edge), kisi and the 3D cases compute the same two-via
structure, and the first via alone is read from the TDR. The cost is a second via in the S-parameters (|S21|
dips add up) and two more connector positions on the 4-port boards.

Exceptions, justified:
- **jlc6-7628-d / jlc6-3313-d** (BS_L3L4, DIFF_SL): no room for a THRU within 50 × 50 mm (two 24 mm rows). Both are read
  in the time domain (TDR plateau of the coupled section, NEXT / FEXT peaks), where the fan-out is separated by time;
  a THRU would not de-embed them anyway (their fixture includes the via to the inner layer).
- **jlc8-7628-p** (PDN): the shunt-thru quantities |Z|, the ESR at the series resonance and the inductance from |Z|
  do not depend on the feed delay; above about 100 MHz, Re Z does. The feeds (12.5 mm, as one half of a THRU) can be
  removed with the VNA's port extension set to the delay of the THRU of jlc8-7628-a / -b (same template, different
  panel: a few ps difference, uncritical here); `compare_measured.py` reports these coupons from the raw data.
- The coupled coupons XT_MS and DIFF_MS on the 4-layer D boards are also read in the time domain; their board has a
  THRU for the CAPZ coupons.

Within one board, all coupons share the panel, so the remaining uncertainty is the connector remount (measure THRU
three times). The 4-layer A board carries an independent check: multiline TRL with THRU, LINE_27/30/50/100 and
REFLECT on the same reference planes.

## 6. PDN and capacitor coupons

### Structures

- **H (PDN loop):** a 4 × 4 pin field at 0.8 mm (BGA-like, checkerboard PWR / GND, 0.6 mm pads with a via in every
  pad; the PWR pins are tied by a 3.2 × 3.2 mm copper patch on L1, the U footprint carries `PI_LOAD` and `PI_TARGET`
  so kisi analyses it as an IC; it stays unpopulated, so the vias in its pads need no filling), one capacitor footprint whose GND pad centre is d = 2 / 5 / 10 / 20 mm from the edge
  of the pin field, a via next to each capacitor pad, and a PWR island on the PDN layer (4 layers: In2, under the GND
  plane In1, 1.065 mm; 8 layers: In3, over the GND plane In4, 0.35 mm). The islands have the same size within a board
  (4 layers 27.7 × 21.6 mm, 8 layers 17.7 × 21.6 mm), so only the distance changes. H_D2_0603 (8 layers) is H_D2 with an
  0603 footprint. Rails `VDD_<coupon>` (0 V: no DC-bias derating, as in the unbiased VNA measurement).
- **G (cavity):** PWR island on In2 under In1, fed at a corner and at the centre through vias from L1 lines.
- **CAPZ:** a capacitor footprint (KiCad standard land pattern) from the 50 Ω line to GND with two GND vias, in 0402,
  0603 and 0805.

### Measurement method: 2-port shunt-thru

Z = 25 · S21 / (1 − S21) (Z ≪ 25 Ω; 1 mΩ gives S21 ≈ −88 dB, inside a VNA's dynamic range at IFBW 10–100 Hz).
What the structures provide:
- **Two ports per DUT, Kelvin-connected:** the H coupons have both connectors on one edge (12.7 mm apart), each with
  its own 50 Ω trace to the pin-field patch, so the only shared impedance is the DUT (patch, vias, planes, capacitor).
  The ground of both ports is the board ground (launch pours, fence vias, planes). CAPZ and G have one port per edge.
- **Feeds of 12.5 mm** per port (the H feed is 3 mm launch + 4.8 mm + 4.7 mm, the same as half of THRU), so the
  board's THRU de-embeds them in a linear sweep.
- **Settings:** two sweeps per DUT: log 100 kHz – 100 MHz (801 points, IFBW 10–30 Hz, 0 dBm) where the feeds do not
  matter, and linear 10 MHz – 2 GHz with 10 MHz steps (P370 grid, de-embedded with THRU). Calibration: SOLT / ECal at
  the cable ends.
- **Ground loop below about 1 MHz:** the shield of the port-2 cable is in parallel with the measured milliohms.
  Put a coaxial ground-loop isolator / common-mode transformer on port 2 (e.g. Picotest J2102B) or many turns of the
  cable through a high-permeability ferrite core, or use a VNA gain-phase port with an isolated receiver; check the
  floor with the 0 Ω population (the reading at 100 kHz must stay at the jumper's resistance).
- **Populate variants:** each board comes as 5 copies, so the same footprint is measured with several parts (table
  below). File names `<COUPON>.<variant>.s2p`; `compare_measured.py` extracts C, SRF, ESR, ESL / loop inductance and
  compares with kisi (H: `power --pdn` with that part as a PI_CAP; CAPZ: `data/components.json` directly – written
  when kisi's `net` ignored shunt parts; kisi 0.3.1 models them at their pads, so `net` could be compared too).

### Component list (generic; the maintainer picks parts from stock)

| Where | Copy 1 | Copy 2 | Copy 3 | Copy 4 | Copy 5 |
|---|---|---|---|---|---|
| H_D2 … H_D20 (0402) | empty | 0 Ω jumper 0402 | 100 nF X7R 16 V 0402 | 1 µF X5R/X7R 10 V 0402 | 10 nF X7R 50 V 0402 |
| H_D2_0603 (0603) | empty | 0 Ω jumper 0603 | 100 nF X7R 16 V 0603 | 1 µF X7R 16 V 0603 | 10 nF X7R 50 V 0603 |
| CAPZ_0402 | empty | 0 Ω 0402 | 100 nF X7R 16 V 0402 | 1 µF X5R 10 V 0402 | 10 µF X5R 6.3 V 0402 |
| CAPZ_0603 | empty | 0 Ω 0603 | 100 nF X7R 16 V 0603 | 1 µF X7R 16 V 0603 | 10 µF X5R 10 V 0603 |
| CAPZ_0805 | empty | 0 Ω 0805 | 100 nF X7R 50 V 0805 | 1 µF X7R 25 V 0805 | 10 µF X5R 16 V 0805 |
| U (pin field) | not a part: the pads stay bare (no BOM entry) | | | | |

Alternatives: any MLCC of the same case size (Murata GRM, Samsung CL, TDK C, Yageo CC); voltage and dielectric are
free because the measurement is unbiased (they only matter for the DC-bias part of kisi's model, which is off here);
0 Ω thick-film jumpers of any make. Quantities per order: 5 boards × (4 H or 3 CAPZ) per board type, plus spares.

What they validate in kisi (`data/components.json`, `decoupling` / `packages`):

| Data | Value now | Measured by |
|---|---|---|
| `decoupling.esl_nh` by package | 0402 0.35, 0603 0.5, 0805 0.6 nH | CAPZ (copies 3–5: ESL above the SRF, minus the 0 Ω mounting loop), H_D2 vs H_D2_0603 |
| `decoupling.esr` model k·(c_ref/C)^0.4 | 100 nF 25 mΩ, 1 µF 10 mΩ, 10 µF 4 mΩ, 10 nF 63 mΩ | CAPZ and H, ESR at the SRF |
| `decoupling.mount_nh` | 0.4 nH | CAPZ copy 2 (0 Ω): footprint + vias; 4 layers 7628 vs 3313 (0.21 vs 0.1 mm to the plane) |
| `packages.pad_c_pf` | 0402 0.1, 0603 0.15, 0805 0.2 pF | CAPZ copy 1 (empty) |
| kisi PDN: plane spreading and via inductance (`l_path`), cavity model | 4 layers: L path 2.43 → 3.70 nH (d = 2 → 20 mm); 8 layers: 0.97 → 1.11 nH (2 → 10 mm) | H copy 2 (0 Ω): Im Z / ω; G: resonances, plane capacitance |
| package from the footprint | 0805 read from the footprint name (v2: from the pad pitch, taken as 0603) | CAPZ_0805 (section 9) |

**After populating, please enter the datasheet values** of the parts actually used. For the H coupons, as PI_CAP
fields in the board's `.kisi.json` (refs in the manifest, e.g. C1 = H_D2 on jlc4-7628-p):
```json
"fields": [{"ref": "C1", "name": "PI_CAP", "value": "c=100n esr=22m esl=0.33n X7R rated=16V",
            "source": {"kind": "datasheet", "document": "GRM155R71C104KA88 data sheet / SimSurfing", "note": "0 V, 25 °C"}}]
```
For the comparison, the same values go into `MEASURED_DIR/populate.json`, one entry per variant
(`{"100n": {"part": "GRM155R71C104KA88", "pi_cap": "c=100n esr=22m esl=0.33n"}}`); without it the generic values
above are used (0 Ω as `c=1 esr=5m`, i.e. inductance only).

### kisi's prediction (generic 100 nF 0402, `kisi_check.md`)

| Board | Coupon | L path / L total (nH) | SRF (MHz) | Magnitude of Z at 100 MHz (Ω) |
|---|---|---|---|---|
| jlc4-7628-p | H_D2 / D5 / D10 / D20 | 2.43 / 3.07 · 2.70 / 3.34 · 3.04 / 3.68 · 3.70 / 4.34 | 9.1 · 8.7 · 8.3 · 7.6 | 1.95 · 2.13 · 2.35 · 2.79 |
| jlc8-7628-p | H_D2 / D5 / D10 · D2_0603 | 0.97 / 1.61 · 1.02 / 1.66 · 1.11 / 1.75 · 1.18 / 2.06 | 12.6 · 12.4 · 12.0 · 11.1 | 1.03 · 1.07 · 1.13 · 1.34 |

The thick 4-layer core makes the distance visible (+1.3 nH from 2 to 20 mm); on the 8-layer board the thin cavity
keeps it small (+0.15 nH from 2 to 10 mm), which tests the other end of the model.

## 7. 3D cases (kisi-fdtd), written, not run

Each coupon has one or more **windows**: the coupon between two port planes on the real stackup, with lumped 50 Ω ports
where traces cross the window edge, planes into the PML, the solder mask as a 0.0305 mm dielectric layer, zero-thickness
copper widened by Wheeler's Δw, thirds rule, 0.035 mm fine mesh to 40 GHz (H: 0.07 mm, 2 GHz; G: 0.1 mm, 8 GHz;
CAPZ: 20 GHz). Windows: `w10` / `w20` lines and THRU (10 / 20 mm), `wv` via–line–via with 5 mm L1 feeds (pairs 3 mm),
`w6` PAIR, `wc` coupled section, `wg` cavity, `wh` PDN loop, CAPZ `w10`. Capacitors are modelled populated with 0 Ω
(5 mΩ lumped short on a 0.5 mm body loop, as `via_cases.pdn_loop`), so the 3D result is the mounting / loop
inductance. The fixtures `THRU w10` and `PAIR w6` de-embed the 3D results with the same geometry as the measurement.

**105 cases, 0.4–2.4 M cells each, 131 M cells in total** (`fdtd_cells` in the manifests; v2: 126 M, the denser row walls add the rest), plus the launch cases (section 2). Not run here (maintainer's
decision); v1 proved the format with one case (0.75 M cells, 36–85 s on one shared core). *Estimate: 1–5 min per via
case, longer for G and H (low-frequency ringing); several CPU hours in total.*
```
cargo build --release -p kisi-fdtd
python3 validation/fdtd/run_refs.py validation/test_board/fdtd/jlc4_7628_b validation/test_board/fdtd_ref --exe target/release/kisi-fdtd --jobs 2
```
Known model limits (`../fdtd/README.md`): lossless metal, dielectric loss fixed at `f_loss`; compare losses with kisi.

## 8. Trace widths (from kisi)

Widths are solved with kisi on a probe board per cross-section (the same KiCad writer as the coupons): bisection with
the native formulas, then secant steps with the 2D field solver (`kisi.mjs`), rounded to 5 µm
(`boards/<stackup>.design.json`). Field solver values; the formula deviation in brackets:

| Cross-section | 7628 (4 / 6 / 8 layers) | 3313 (4 / 6 layers) |
|---|---|---|
| 50 Ω microstrip L1 / bottom | 0.345 mm: 49.8 Ω (+1.8 %) | 0.150 mm: 50.2 Ω (+2.8 %) |
| launch | 0.315 / gap 0.3: 50.0 Ω (+5.7 %) | 0.255 / gap 0.105, In1 opening 0.465: 50.05 Ω (**+25 %**) |
| CPWG, gap 0.18 | 0.280: 49.7 Ω (+6.9 %) | 0.140: 50.2 Ω (+6.0 %) |
| 100 Ω microstrip pair, s 0.2 | 0.205: 100.1 Ω (+4.1 %) | 0.115: 99.9 Ω (+6.8 %) |
| pair at the 1.0 mm via pitch, L1 | 0.315: 99.8 Ω (+4.3 %) | 0.150: 100.4 Ω (+2.8 %) |
| 50 Ω stripline | 6 layers In2/In3 0.405: 49.9 Ω; 8 layers In2/In5 0.210: 49.9 Ω | 6 layers 0.515: 50.1 Ω |
| 100 Ω stripline pair, s 0.2 | 6 layers 0.190: 99.8 Ω (+10.3 %); 8 layers 0.140: 100.4 Ω (+4.8 %) | 0.210: 99.9 Ω (+13.1 %) |
| Aisler (assumed) | microstrip 0.335, launch 0.305 / 0.3, CPWG 0.275, pair 0.21 / 0.2 | |

The field solver is within 1.6 % of HFSS (`../fdtd/TESTS.md`), so it sets the widths; the formula deviations are
kisi's known weak spots (coplanar, stripline pairs, and new: the launch over a plane opening) and are themselves a
target of the measurement.

## 9. kisi check (native CLI and kisi.mjs)

`check_coupons.py` ran `overview`, `config-check`, `check --all`, `interfaces`, `gaps`, `net` on every signal net and
`power --pdn` on every PDN rail, with the native `kisi` (formulas) and `kisi.mjs` (field solver), both kisi 0.3.1
(v3 run 2026-10-30 on the regenerated boards; the v2 run used 0.2.3). Full tables: `kisi_check.md`.

- All 23 boards parse with the stackup from the file and no board warnings; `config-check` is ok; `gaps` reports only
  the missing regulator chain (and on the PDN boards the rails without a voltage, finding 4).
- 50Z within ±5 % of the length: 93–100 % with the field solver, 54–76 % with the formulas; 100Z_Diff 92–97 % field,
  31–76 % formula. The formula share dropped from v2 (63–83 %) because the grounded coplanar launch is now 6.48 mm per
  side instead of 3 mm, and kisi's coplanar formula is +5.7 % high there. What falls out with the field solver:
  fan-in / taper pieces, traces over antipads, the short piece where the trace meets the connector's tip.
- `check --all` now returns rows for the connector–connector lanes as well (kisi 0.3.1; v2: none); the coupons are
  still judged with `net`.
- **kisi findings of v2, rechecked with kisi 0.3.1:**
  1. KiCad's 0805 capacitor taken for an 0603 (package from the pad pitch): **resolved** – kisi now reads the case size
     from the footprint name (`kisi_si::package::chip_package`); CAPZ_0805 is an 0805.
  2. A shunt capacitor on a signal net ignored by `net`: **resolved** – `net` models shunt parts at their pads (with a
     note that the package parasitics are estimates); the CAPZ nets now show the capacitor.
  3. A plane opening that ends where a track ends sampled as solid plane (3313 launch at 35 Ω): **resolved** – the
     3313 launch is 50 Ω with the field solver along its whole length, and kisi warns that the path lies over a
     cut-out in a layer between the trace and its reference.
  4. `power` without `--pdn` reports "no nominal voltage" for the VDD_* rails (`gaps`: rails-without-voltage):
     expected, the PDN coupons are meant unbiased.
  5. The launch over a plane opening is 25 % high with the formulas (the field solver is right): still a formula weak
     spot.
- Via results (ladder model): see `kisi_check.md` (A group Z rises with the antipad; D group "missing" return path for
  D_NONE; stub resonances in section 2).

## 10. Measurement plan

**Priority:** P1 THRU, LINE_50, LINE_100, A0800, A1200, D_S1p0, D_NONE, PAIR, E_R_G2, XT_MS (jlc4-7628); THRU, B_L1L6,
VLV_L3, SL_L3 (jlc6-7628); VLV_L3, E_L1L3 (jlc8, stub notch); CAPZ and H copies 1–3. P2 the rest of the first order,
G_CAV. P3 Aisler, the TRL set, a second copy of each board (build spread), the second (3313) order.

| Instrument | Coupons | Settings | Calibration |
|---|---|---|---|
| ENA, 4 ports, 20 GHz | all S-parameter coupons; 4-port coupons with all four ports | linear 10 MHz – 20 GHz, Δf 10 MHz (P370 and TDR need f_k = k·Δf), IFBW 300 Hz (100 Hz for XT and G), −10 … 0 dBm | ECal / SOLT at the cable ends |
| PNA-X, 4 ports | the same to 40 GHz (2.92 mm); priority the stub coupons (8 layers VLV_L3, E_L1L3, VLV_L6; 6 layers VLV_L3), A, D, E | 10 MHz – 40 GHz, Δf 10 MHz (4000 points) | 2.92 mm ECal or mechanical kit at the cable ends (Samtec used a Maury 8770CK) |
| PNA, 2 ports | 2-port coupons; TRL set | 10 MHz – 40 GHz | 2.92 mm SOLT, then on-board multiline TRL |
| VNA 2-port with low-frequency range (e.g. ENA E5061B LF-RF) | H, CAPZ, G (shunt-thru) | log 100 kHz – 100 MHz + linear 10 MHz – 2 GHz (section 6); ground-loop isolator on port 2 | SOLT at the cable ends |
| Scope + step source (TDR/TDT) | THRU (Zc plateau), A / D vias, E (differential), XT / BS / DIFF (NEXT, FEXT, Zdiff plateau) | normalised to a 35 ps Gaussian edge | open / short / load at the cable end |

- **De-embedding:** P370 2×thru with the board's own THRU / PAIR (`sparam.fixture_2xthru`, `deembed2`; pairs: per-port
  shift, or scikit-rf `IEEEP370_MM_NZC_2xThru`). Self-check: LINE_50 de-embedded with THRU is a clean 25 mm line.
- **Repeatability:** remount the connectors on THRU three times; two copies of each board.
- **Stackup extraction first:** εeff and tan δ from LINE_* / SL_L3 (line–line), core εr from G_CAV, a micro-section for
  thicknesses; put the values into `stackups.json`, regenerate with `--reuse-design` and rerun kisi and the 3D cases.

## 11. Ordering checklist

| Item | JLCPCB, first order | JLCPCB, second order | Aisler |
|---|---|---|---|
| Boards | 14 designs: 5 × 4 layers (100 × 96, 35 × 96, 35 × 96, 50 × 96, 100 × 96), 4 × 6 layers (35/50 × 48), 5 × 8 layers (35/50 × 48) | 8 designs: 4 × 4 layers, 4 × 6 layers (sizes as the first order) | 1: 35 × 144 mm |
| Files | Gerbers + drill from KiCad after refill (B) and DRC, one order line per board | same | KiCad upload |
| Stackup | impedance control **on**; templates **JLC04161H-7628**, **JLC06161H-7628**, **JLC08201H-7628** (2.0 mm), FR-4 TG155 | **JLC04161H-3313**, **JLC06161H-3313** | ask sales for the stackup first; 35 µm outer if offered |
| Impedance specs | 50 Ω L1 0.345 mm; 100 Ω diff. L1 0.205/0.2; 6 layers In2 stripline 0.405, pair 0.19/0.2; 8 layers In2 stripline 0.21, pair 0.14/0.2 | 50 Ω L1 0.15; diff. 0.115/0.2; 6 layers In2 0.515, pair 0.21/0.2 | – |
| Copper / finish / mask | 1 oz outer, 0.5 oz inner; **ENIG**; green mask both sides | same | ENIG, green |
| Outline | single boards, stepped outline, no panel | same | – |
| Quantity | 5 each | 5 each | 3 (minimum) |
| Order number | at `JLCJLCJLCJLC` (bottom silk) | same | – |
| Assembly | none ordered: the maintainer populates P and D boards from stock (section 6) | D boards (CAPZ) | – |

**Price estimates** (fetched 2026-10-22; the JLC online quote could not be run from the workspace):
- **Aisler**, "4 Layer 1.6mm ENIG Budget": job fee €14.00 + area × €0.13/cm² × quantity, sets of 3, net
  ([Aisler pricing](https://community.aisler.net/t/our-simple-pricing/102), page dated 2023-08-27). 35 × 144 mm =
  50.4 cm² (bounding box) → **€33.66 net for 3 boards** (€31.88 if the 45.8 cm² outline is billed), plus VAT and shipping.
- **JLCPCB**: JLC publishes "5pcs 6-layer PCBs starts from $2" with ENIG at $0
  ([6-layer page](https://jlcpcb.com/resources/6-layer-pcbs)) and has offered one $2 coupon per month for a 6–8-layer
  board ≤ 50 × 50 mm with default options ([elektroda, 2023-08-08](https://www.elektroda.com/news/news3994646.html); may
  have changed); the 1–4-layer prices were adjusted in August 2026 without published numbers
  ([JLC news, 2026-08-03](https://jlcpcb.com/news/jlcpcb-fr4-pcb-price-adjustment-august2026)).
  *Estimate, not a quote: 4 layers ≤ 100 × 100 mm, 5 pcs, ENIG, impedance template about $10–30 per design;
  6 layers ≤ 50 × 50 mm $2 with the coupon, else about $20–40; 8 layers ≤ 50 × 50 mm $2 with the coupon, else about
  $40–80; shipping about $15–30 per order. First order about $330–710 + shipping; second order about $120–280 +
  shipping.* Take the online quote before ordering; dropping jlc8-7628-d (E_L1L3) or merging the two 8-layer pair
  boards (50 × 72 mm, outside the bracket) are the levers if the 8-layer price is high.
- **Connectors:** 4 (+1 spare) × Samtec `292-J-P-EP-ST-EL-01` from DigiKey or Mouser, about 52.65 USD each (Samtec's
  price for one, 2026-10-30) = about 211 USD for 4; **lead time about 6 weeks: order them first**. The #1-72 screws
  and split washers are on Samtec's drawing (whether they come with the part: check on delivery; spares of both).
  Tools: a torque screwdriver for 0.5–0.8 in-lb (0.06–0.09 N·m) with a 1/16 in hex bit for the board screws, an
  8 in-lb 5/16 in torque wrench for the 2.92 mm cables, a 5/16 in open-end wrench to hold the jack.
- **Parts:** from stock (section 6).

## 12. Comparison

`compare_measured.py boards/<board>.manifest.json MEASURED_DIR [--fdtd-ref fdtd_ref] [--kisi KISI | --kisi-mjs kisi.mjs]`
reads `MEASURED_DIR/<COUPON>.s2p/.s4p` (port order = the connector order in the manifest: 2-port 1 = left; H 1 = upper,
2 = lower; 4-port 1 = P/A left, 2 = N/V left, 3 = P/A right, 4 = N/V right) and `<COUPON>.<variant>.s2p` for H / CAPZ,
runs kisi (`net … --s2p/--s4p`, `power --pdn` with the variant's PI_CAP), reads the kisi-fdtd results, de-embeds all with
their own 2×thrus and prints per coupon: via TDR min / max, |S21|, notch, delay (A, B, D); εeff, loss, Zc (lines);
Zdiff, Sdd21, Scd21 (E); NEXT / FEXT (XT, BS); Zdiff plateau (DIFF); cavity peaks (G); C, SRF, ESR, ESL / loop
inductance and |Z| at 1 / 10 / 100 MHz / 1 GHz (CAPZ, H). Tested with kisi's own S-parameters standing in for the
measurement (differences 0) and with model shunt-thru data for CAPZ / H. Acceptance bands: `../via_references.md`
section 6, with the measured connector repeatability as the floor.

## 13. To check by a human before ordering

1. **Connector footprint and launch** (blocker): compare the generated footprint with Samtec's (drawing sheet 2,
   SnapMagic footprint, STEP model in KiCad's 3D view: screws, ground pads, pin on the tip, jaw over the launch);
   run the launch optimisation (section 2) and put the result into `connectors.json` (`launch`); cross-check the
   launch with the STEP model in CST; optionally ask SIG@samtec.com for their launch on a 0.2 mm (7628) and a 0.1 mm
   (3313) top dielectric. Regenerate (`--reuse-design`).
2. **Copper to the edge:** Samtec's footprint runs the ground fingers and the tip to the board edge; the boards keep
   JLC's 0.2 mm. Ask JLC whether 0.1 mm (or copper to the edge) is possible on these edges, or accept 0.2 mm (the pin
   still rests 0.54 mm on the tip).
3. **KiCad 8 or 9**: open each board, refill zones (B), run DRC, look at every layer (antipads, island clearances,
   launch openings, removed non-functional pads). Then rerun `check_coupons.py`: the numbers should not change.
   Done in the workspace with KiCad 7 on converted copies (pcbnew's DRC after a zone refill): no errors and no
   unconnected items on any of the 23 boards; warnings only for the missing footprint library `kisi_coupons`, the open
   ends of REFLECT and four isolated fill pieces of the G_SMALL island (Aisler). (v2 in the same check: 22 copper-to-edge
   errors at the old launch, 46 isolated-fill warnings.) The script: `k8to7.py` + pcbnew's `WriteDRCReport`, not in the
   repository (a workaround for the missing KiCad 8).
4. JLC: the five templates still exist in the order form; impedance-control option and line specs accepted; decline
   copper thieving near the coupons; the 0.105 mm launch gap of the 3313 boards is accepted.
5. 4-port connector pitch 12.7 mm: torque-wrench access with the real cables (3.2 mm between the 9.53 mm bodies).
6. Aisler: stackup data; regenerate with the real values.
7. Parts: pick from stock, enter the PI_CAP values (section 6).

## 14. Regenerate

```
python3 validation/test_board/make_coupons.py --kisi target/release/kisi --kisi-mjs dist/kisi.mjs \
    [--boards jlc4_7628_a,...] [--reuse-design] [--fdtd-exe target/release/kisi-fdtd]
python3 validation/test_board/check_coupons.py --kisi target/release/kisi --kisi-mjs dist/kisi.mjs
python3 validation/test_board/make_coupons.py --launch-cases jlc4_7628 --fdtd-exe target/release/kisi-fdtd
python3 validation/fdtd/run_refs.py validation/test_board/fdtd/launch_jlc4_7628 OUT --exe target/release/kisi-fdtd --jobs 2
```
Board keys: `<stackup>_<part>` with stackups `jlc4_7628`, `jlc6_7628`, `jlc8_7628`, `aisler4_assumed`, `jlc4_3313`,
`jlc6_3313` and parts `a`–`d`, `p`. Without `--reuse-design` the widths are solved again (several minutes per stackup
with the field solver); with it, only boards, manifests and 3D cases are rewritten.
