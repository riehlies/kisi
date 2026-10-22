# Measurement coupons (test boards)

Status: **design v2, prepared, not ordered.** 23 boards are generated (first order: JLC 4 / 6 / 8 layers with 7628
prepreg plus one Aisler board; second order: JLC 4 / 6 layers with 3313 prepreg). kisi reads all of them, their
impedances are on target with kisi's field solver, the PDN and capacitor coupons have a measurement method and a
component list, and the 3D cases are written (**not run**). **KiCad DRC has not been run** (the workspace has only
KiCad 7, which cannot read KiCad 8 boards): the maintainer runs it. The SMA end-launch footprint is a
**placeholder** (section 3). The plan behind this is `../test_board.md`.

Goal: close the loop simulation → measurement for kisi's fast models and the 3D references (`../via_ref`). The same
coupon geometry is measured, computed with kisi-fdtd (cases generated from the same description as the KiCad board)
and with kisi (the board file itself), and all three are de-embedded the same way (IEEE P370-style 2×thru,
`../fdtd/sparam.py`).

What changed from v1 (commit d7a7ec8): SMA connector and a 26.5 GHz frequency plan; boards split to the JLC price
brackets; every board has its own 2×thru and all connectors are on the top side (via coupons are via–line–via);
an 8-layer set; the thin-prepreg set; PDN coupons (plane cavity, pin field with capacitors) and capacitor coupons
with a component list; a pin-wide launch with a plane opening for the thin prepreg. The single v1 3D result
(`fdtd_ref/`) belonged to the old geometry and is removed.

## 1. Files

| Path | Content |
|---|---|
| `make_coupons.py` | generator: boards, kisi configuration, 3D cases; trace widths from kisi |
| `stackups.json` | stackups with values and sources (JLC 4 / 6 / 8 layers, Aisler assumed, thin prepreg) |
| `connectors.json` | SMA end launch, footprint (**placeholder**), layout pitches, via sizes, capacitor land patterns, size limits |
| `boards/<board>.kicad_pcb`, `.kicad_pro` | KiCad 8 boards (KiCad 9 opens them); net classes `50Z`, `100Z_Diff` |
| `boards/<board>.kisi.json` | kisi configuration: dielectric values with sources, class targets and profiles |
| `boards/<board>.manifest.json` | coupons, nets, connectors (J numbers, port order), parts (C/U refs), 3D cases and their cell counts |
| `boards/<stackup>.design.json` | trace widths and the impedance table (kisi formula and field solver) |
| `boards/<board>.kisi_check.json`, `kisi_check.md` | kisi run on every board (section 9) |
| `fdtd/<board key>/*.json` | kisi-fdtd cases, format of `../fdtd/via_cases.py`; run with `../fdtd/run_refs.py` |
| `check_coupons.py` | runs kisi on the boards → `kisi_check.md` |
| `compare_measured.py` | measured vs kisi vs kisi-fdtd (section 12) |

## 2. Connector and frequency plan

- **Southwest Microwave `292-04A-14`**, SMA end launch jack, narrow block (.350 in), clamp-on, solderless, boards
  .040–.090 in (1.02–2.29 mm; the 2.05 mm 8-layer board fits). 2024 catalogue
  ([PDF](https://mpd.southwestmicrowave.com/wp-content/uploads/End-Launch-Connectors-2024.pdf)): SMA to 27 GHz, pin
  Ø.010 in (0.254 mm), dielectric Ø.0635 in, PCB fasteners #1-72 SHCS, connector fasteners #0-80. **Planned to
  26.5 GHz.** Price not published (no distributor price found on 2026-10-22): quote from Southwest. Needed: 4 (the
  4-port coupons), moved between coupons.
- **Footprint: PLACEHOLDER** (`connectors.json`, `footprint.verified: false`). Block and footprint dimensions are only
  images in the catalogue. Signal pad (1.5 mm), ground-contact mask opening, two NPTH Ø2.0 holes at 4.5 mm / ±3.0 mm
  and the block outline (8.89 × 6 mm on the board, 14 mm outside) are guesses inside the 8.89 mm block. **To update:**
  edit `connectors.json` → `connector.footprint` and run
  `make_coupons.py --reuse-design` (the widths stay; boards, manifests and 3D cases are rewritten in about a minute).
  The generator asserts that the PDN coupons' feed corners and PWR islands keep clear of the hole pattern, and all
  other traces leave the launch straight until they are past the holes (`X_CLEAR` follows the hole positions).
- **Launch:** a 3 mm grounded coplanar section at the edge with two rows of fence vias (0.8 mm) into the ground pour
  the block clamps on, then the 50 Ω line. 7628 boards: 0.315 mm / gap 0.3 mm (the 50 Ω microstrip is 0.345 mm, wider
  than the pin). **3313 boards** (0.099 mm top prepreg: the 50 Ω line is 0.15 mm, narrower than the pin): the launch
  pad is pin-wide (0.255 mm), the first plane (In1) has a 0.465 mm wide opening under it (the coplanar opening;
  reference is the next plane) and the top gap is 0.105 mm (kisi field solver 50.05 Ω; JLC minimum spacing
  0.1 mm). Copper ends 0.2 mm before the routed edge (JLC rule).
- **Frequency plan (SMA, 26.5 GHz).** kisi's stub resonance for the stubbed vias (`kisi_check.md`):

  | Coupon | Stub | kisi stub resonance | In the band? | Action |
  |---|---|---|---|---|
  | 6 layers VLV_L3, SL_L3 (L1 → In2, stub to L6) | 0.88 mm (3313: 0.82) | 32.4 GHz (34.5) | no | kept: the stub capacitance is measurable below the notch; the notch moves to the 8-layer board |
  | 6 layers DIFF_SL, BS_L3L4 (inner exits) | 0.65–0.88 mm | 24.5–38 GHz | partly | kept (coupling coupons; time domain) |
  | **8 layers VLV_L3, SL_L3** (L1 → In2, stub to L8, 2.05 mm board) | **1.42 mm** | **19.8 GHz** | **yes** | new: the stub notch inside the band (the plan's "notch below ~20 GHz") |
  | **8 layers E_L1L3** (pair, stub to L8) | 1.42 mm | 22.1 GHz | yes | new: differential stub notch |
  | 8 layers VLV_L6 (L1 → In5, stub to L8) | 0.60 mm | 38.5 GHz | no | short-stub reference next to VLV_L3 and B_L1L8 (none) |

  A 1.6 mm 8-layer board allows about 1.1 mm of stub (notch near 25 GHz), hence JLC's 2.0 mm 8-layer template.
  Everything else (lines, through vias, return path, pairs without stubs, coupling) has no resonance below 26.5 GHz;
  the plane cavities resonate at 1.4–3.2 GHz (measured to 8 GHz).

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
edge, 12.7 mm apart). GND via walls (2.54 mm, through all layers) run along every row boundary and along the
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
  compares with kisi (H: `power --pdn` with that part as a PI_CAP; CAPZ: `data/components.json` directly, because
  kisi's `net` does not model a shunt part on a signal net).

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
| package from the pad pitch | 0805 (pitch 1.9 mm) is taken as 0603 | CAPZ_0805 (section 9) |

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

**105 cases, 0.4–2.0 M cells each, 126 M cells in total** (`fdtd_cells` in the manifests). Not run here (maintainer's
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
`power --pdn` on every PDN rail, with the native `kisi` (formulas) and `kisi.mjs` (field solver). The `dist/kisi.mjs` in
the workspace was older (2026-10-20, without the per-IC PDN and the via junction model), so a `kisi.mjs` was bundled
from the current wasm build (`tools/bundle_cli.py`, version 0.2.3; its field-solver impedances equal the old build's,
and with `--formula` its `net` and `power --pdn` output equals the native CLI). Full tables: `kisi_check.md`.

- All 23 boards parse with the stackup from the file and no board warnings; `config-check` is ok; `gaps` reports only
  the missing regulator chain (irrelevant here).
- 50Z within ±5 % of the length: 92–100 % with the field solver, 63–83 % with the formulas; 100Z_Diff 92–97 % field,
  39–83 % formula. What falls out with the field solver: fan-in / taper pieces, traces over antipads, and kisi's
  sampling at the ends of the 3313 launch openings (next point).
- **kisi findings (reported, kisi's Rust code not changed):**
  1. **KiCad's 0805 capacitor is taken for an 0603**: kisi derives the package from the pad pitch
     (`crates/kisi-pi/src/pdn_ic.rs`, `package_of`: 0603 below 1.95 mm); `C_0805_2012Metric` has 1.9 mm. ESL 0.5
     instead of 0.6 nH. CAPZ_0805 measures which is right.
  2. **A shunt capacitor on a signal net is ignored by `net`** without a warning: the CAPZ nets give the same result as
     THRU. The CAPZ comparison therefore uses `data/components.json` directly.
  3. `check --all` returns no rows: kisi checks only ic–ic and ic–connector lanes (`kisi-core/src/service.rs`); every
     coupon lane is connector–connector, so the coupons are judged with `net` (as in v1).
  4. A plane opening that ends exactly where a track ends: kisi samples the last ~0.5 mm as if the plane were solid
     (3313 launch: 0.47 mm at 35 Ω instead of 50 Ω). Only the fixture is affected (de-embedded).
  5. The launch over a plane opening is 25 % high with the formulas (the field solver is right): a new formula weak spot.
  6. `power` without `--pdn` reports "no nominal voltage" for the VDD_* rails (no voltage in the name): expected, the
     PDN coupons are meant unbiased.
- Via results (ladder model): see `kisi_check.md` (A group Z rises with the antipad; D group "missing" return path for
  D_NONE; stub resonances in section 2).

## 10. Measurement plan

**Priority:** P1 THRU, LINE_50, LINE_100, A0800, A1200, D_S1p0, D_NONE, PAIR, E_R_G2, XT_MS (jlc4-7628); THRU, B_L1L6,
VLV_L3, SL_L3 (jlc6-7628); VLV_L3, E_L1L3 (jlc8, stub notch); CAPZ and H copies 1–3. P2 the rest of the first order,
G_CAV. P3 Aisler, the TRL set, a second copy of each board (build spread), the second (3313) order.

| Instrument | Coupons | Settings | Calibration |
|---|---|---|---|
| ENA, 4 ports, 20 GHz | all S-parameter coupons; 4-port coupons with all four ports | linear 10 MHz – 20 GHz, Δf 10 MHz (P370 and TDR need f_k = k·Δf), IFBW 300 Hz (100 Hz for XT and G), −10 … 0 dBm | ECal / SOLT at the cable ends |
| PNA-X, 4 ports | the same to 26.5 GHz (SMA); priority the stub coupons (8 layers VLV_L3, E_L1L3), A, D, E | 10 MHz – 26.5 GHz, Δf 10 MHz | ECal 3.5 mm / SMA at the cable ends |
| PNA, 2 ports | 2-port coupons; TRL set | 10 MHz – 26.5 GHz | SOLT, then on-board multiline TRL |
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
- **Connectors:** 4 × `292-04A-14`, quote from Southwest Microwave (*estimate 100–300 USD each, not quoted*); screws
  per Southwest; SMA torque wrench (8 in-lb).
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

1. **Connector footprint** (blocker): mounting holes, ground contact, block depth, and Southwest's launch
   recommendation for the 0.1 mm (3313) top dielectric (the plane opening here is a placeholder too); edit
   `connectors.json`, regenerate (`--reuse-design`).
2. **KiCad 8 or 9**: open each board, refill zones (B), run DRC, look at every layer (antipads, island clearances,
   launch openings, removed non-functional pads). Then rerun `check_coupons.py`: the numbers should not change.
3. JLC: the five templates still exist in the order form; impedance-control option and line specs accepted; decline
   copper thieving near the coupons; the 0.105 mm launch gap of the 3313 boards is accepted.
4. 4-port connector pitch 12.7 mm: torque-wrench access with the real cables.
5. Aisler: stackup data; regenerate with the real values.
6. Parts: pick from stock, enter the PI_CAP values (section 6).

## 14. Regenerate

```
python3 validation/test_board/make_coupons.py --kisi target/release/kisi --kisi-mjs dist/kisi.mjs \
    [--boards jlc4_7628_a,...] [--reuse-design] [--fdtd-exe target/release/kisi-fdtd]
python3 validation/test_board/check_coupons.py --kisi target/release/kisi --kisi-mjs dist/kisi.mjs
```
Board keys: `<stackup>_<part>` with stackups `jlc4_7628`, `jlc6_7628`, `jlc8_7628`, `aisler4_assumed`, `jlc4_3313`,
`jlc6_3313` and parts `a`–`d`, `p`. Without `--reuse-design` the widths are solved again (several minutes per stackup
with the field solver); with it, only boards, manifests and 3D cases are rewritten.
