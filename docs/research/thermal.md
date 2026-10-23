# Temperature estimate in the power analysis – concept and first increment

Status: concept (October 2026) and a first, steady-state implementation (`kisi_pi::thermal`, `kisi thermal`,
the viewer's "Temperature" step). Scope: a board-level temperature estimate from the losses kisi already
computes – how hot the copper, the laminate and the parts get in still air or a given air flow, which part is
closest to its limit, and how much the warmer copper adds to the IR drop. Not a replacement for a CFD or
detailed thermal simulation with package models. Claims that could not be checked are marked *(not verified)*;
sources are listed at the end ([T1] …).

## Summary
- **Physics covered now:** Joule heating of the copper (from the DC solution of every rail and the ground return),
  heat of the parts (load power V·I, regulator losses from the power balance, I²R of shunts and ferrites),
  in-plane conduction of every copper layer (by its copper fill), in-plane and through-plane conduction of the
  laminate (anisotropic), vias and plated holes as thermal conductors, natural or forced convection and
  linearised radiation on both outer surfaces, and per part a junction node with θJB to the board and θJC to the
  top surface (the JEDEC two-resistor model). One electro-thermal step: the copper resistance at the estimated
  temperature (+0.393 %/K) and the change of the IR drop it causes.
- **Model:** the board is a stack of 2D sheets – one per copper layer, on a uniform grid over the board outline –
  with the same building blocks as the DC voltage-drop solver (rasterisation, conductances between cells, vias
  between layers, a sparse Cholesky solve). The equation per layer is ∇·(k t ∇T) + q = 0, the layers are coupled
  through the dielectric and the vias, the outer layers lose heat to the air.
- **Data:** material constants and the convection correlations in `data/thermal.json`, package defaults (θJB,
  θJC by package family and size) in `data/components.json` → `thermal`, per-part values from a new footprint
  field `PI_THERMAL` (`theta_jb=17.4 theta_jc=53.6 tj_max=125`) with its source in `<board>.kisi.json`. Every
  default is marked as an estimate in the output.
- **Results:** temperature map per layer, hot spots, per part the board temperature under it and the junction
  temperature with the margin to Tj max, the copper temperature per rail, the IR drop at that temperature, an
  energy balance (heat in = convection + radiation out).
- **Validation:** analytic cases (1D slab, fin, radial spreading in a plane with convection, energy balance) as
  unit tests, the JEDEC θJA of three packages on a model of the JESD51-7 test board, and a plausibility check
  on ThunderScope and LibreVNA (section 7).
- **Later:** transient (thermal capacitance, time constants – the matrix is ready for implicit time steps),
  regulator losses split between IC and inductor, heat sinks and conduction into mounting holes, card guides and
  connectors, enclosure air, temperature-dependent IC power (leakage), adaptive grid refinement under small
  hot parts, filled vias and laminates by name.

## 1. Where kisi stands
`kisi_pi::pi` solves the DC voltage drop of a rail on rasterised copper: per copper layer a square grid over the
rail's copper, conductances between neighbouring cells from the sheet resistance (thin tracks with a fractional
weight), via barrels between layers, a sparse SPD solve (`kisi-sparse`, faer). `kisi_pi::losses` takes the
regulator chain (`tree`) and the DC results: the ground return as its own "rail", the heat per part (the power a
load draws, the loss of a regulator or pass element), the copper loss per cell, rail and layer, a loss map in
W/mm² and the power flow. Until now the output stopped at power – "Power, not temperature" (README).

## 2. Physical effects
### 2.1 Heat sources
| Source | How kisi gets it | Now |
|---|---|---|
| Joule heating of copper traces, planes and via barrels | I²R per cell from the DC solution of every rail and the ground return (`PiResult::loss`) | yes |
| Load ICs | P = Σ V·I of their `PI_LOAD` entries; all of it counts as heat in the part (power leaving the board – connectors, LEDs, RF output – can be set with `PI_THERMAL p=…`) | yes |
| Regulators | P_in − P_out from the power balance (efficiency of switchers, (V_in − V_out)·I + V_in·I_q of LDOs) | yes, all on the regulator |
| Shunts, ferrites, pass elements | I²R from their `r` | yes |
| Split of a switcher's loss between IC and inductor (DCR, core) | needs the inductor's DCR and a loss model | later (ROADMAP) |
| AC losses: ripple current in capacitor ESR, skin effect of switching currents, dielectric loss of RF lines | small on most boards | not modelled |

### 2.2 Heat paths
- **Junction → package → board and top.** Datasheets give θJA, θJC(top), θJB and the characterisation parameters
  ψJT and ψJB [T1, T2]. θJB is measured with the board clamped to a ring cold plate and the package top
  insulated (JESD51-8), θJC with the top on a cold plate (JESD51-14) [T2, T3]. The JEDEC two-resistor compact
  model (JESD15-3) connects a junction node through θJB to the board under the part and through θJC to the case
  top [T3]. It is "suitable for roughly determining the circuit board size and the parts arrangement" – absolute
  junction temperatures carry the error of a two-node package model [T3]. ψJB is not a resistance: it is the
  ratio (T_J − T_B)/P with *all* heat paths active, and Tj = T_B + ψJB·P is the recommended estimate when the
  board temperature next to the part is known [T1, T2].
- **θJA only as a fallback.** θJA is measured or simulated on a JEDEC test board in a still-air enclosure
  (JESD51-2 with the JESD51-3 one-layer or JESD51-7 four-layer board, 76.2 × 114.3 mm) [T2]. It describes that
  board, not the user's: other parts' heat, planes and board size are not in it. kisi uses it only when nothing
  else is given, as Tj = T_amb + θJA·P, and flags it.
- **In-plane conduction of copper.** A layer conducts with k_Cu·t·(copper fill). 35 µm of copper (k ≈ 390 W/m·K)
  give 0.014 W/K per square – about 15 times a whole 1.6 mm FR-4 board in plane. Planes therefore set the
  spreading.
- **Laminate.** Glass-epoxy is anisotropic: in plane 0.81–1.06 W/m·K, through the thickness 0.29–0.34 W/m·K
  (measured, Azar & Graebner 1996, quoted in [T5]). Thin dielectrics between planes conduct well through the
  thickness (0.1 mm: 3000 W/m²K, far above the surface's 10–20 W/m²K); the thick core of a 6-layer board
  (0.9 mm: 330 W/m²K) is the main through-plane resistance.
- **Vias.** A plated barrel (25 µm in a 0.3 mm hole: 2.2·10⁻⁸ m²) conducts 390·2.2·10⁻⁸/L – 0.04 W/K over
  0.2 mm, about 100 times the laminate under a 0.5 mm cell. Thermal vias under exposed pads are what makes
  QFN/BGA θJB values work [T2]. Fill (epoxy, copper) is not known from the board file; kisi assumes open
  barrels (conservative).
- **Solder joints and solder mask.** Their resistance per area (mask 25 µm / 0.2 W/m·K ≈ 1.3·10⁻⁴ m²K/W) is
  small against the air (≈ 0.07 m²K/W); the mask is kept in series with the surface coefficient.
- **Convection.** Natural convection from horizontal plates: Nu = 0.54 Ra^¼ for the hot side facing up,
  0.27 Ra^¼ facing down, with L = area/perimeter; vertical plates: Churchill–Chu [T4]. Forced convection:
  laminar flat plate, Nu = 0.664 Re^½ Pr^⅓ [T4]. On a 100 mm board 20 K above the air this gives 5–7 W/m²K
  (up), 2–3 W/m²K (down), 4–5 W/m²K (vertical) and 12 W/m²K at 1 m/s.
- **Radiation.** Linearised, h_rad = εσ(T_s² + T_a²)(T_s + T_a) ≈ 6–7 W/m²K for ε = 0.9 (solder mask, an estimate)
  near room temperature – as large as natural convection, so it cannot be left out.
- **Not modelled now:** conduction into screws, standoffs, card edge and bracket, the PCIe or M.2 connector
  (often a large share on add-in cards), heat sinks, the enclosure (its air warms up: use that air as ambient),
  shadowing between parts, the side faces of packages, board edges (≈ 3 % of the surface on a 100 mm board).

### 2.3 Thermal capacitance and time constants
Volumetric heat capacity: copper 3.45 MJ/m³K, FR-4 about 2 MJ/m³K *(typical, not verified per laminate)*. A
1.6 mm board stores ≈ 3.3 kJ/m²K; with 2 × 12 W/m²K to the air its time constant is τ = C/(hA) ≈ 2–3 minutes.
Packages and dies react in milliseconds to seconds. Consequences: the steady state is the right answer for
continuous operation; loads that change faster than about a minute (bursts, duty cycles) see the board at the
average power plus a package-level transient. A transient solver would reuse the same conductance matrix G with
the capacitances C: implicit Euler (C/Δt + G)·T_{n+1} = C/Δt·T_n + q needs one factorisation per time step size.
Out of scope for this increment.

### 2.4 Electro-thermal coupling
Copper resistivity rises with temperature: ρ(T) = ρ₂₀[1 + α(T − 20 °C)], α = 0.00393 /K (IEC 60028 annealed
copper). 30 K above 20 °C means +12 % resistance, IR drop and copper loss. kisi's DC solver works at 20 °C
(ρ = 1.72·10⁻⁸ Ω·m). The coupled solution is a fixed point (DC → losses → temperature → resistance → DC …).
Copper loss is a small part of the heat on most boards (ThunderScope: 0.06 of 12.8 W), so one step is enough:
the IR drop is recomputed at the estimated temperature and the change reported; the second pass of the
temperature with the higher copper loss is left out (its effect is α·ΔT on the copper share only). Other
temperature effects – regulator efficiency, IC leakage power (it can double per 10–20 K in large FPGAs
*(not verified)*), LDO dropout – are not modelled.

### 2.5 Limits
- **Junction temperature:** Tj max from the datasheet (absolute maximum or recommended operating range;
  commercial FPGAs often 85 °C, industrial 100 °C, analogue and power ICs 125–150 °C). Without a value kisi
  assumes 125 °C and says so.
- **Laminate:** maximum operating temperature (MOT, UL 796) of FR-4 typically 130 °C *(typical, not verified per
  material)*; kisi warns above it.
- **Not checked:** capacitor temperature ratings (X5R 85 °C, X7R 125 °C), electrolytic lifetime (Arrhenius,
  halves per 10 K), solder fatigue.

## 3. Discretisation
- **Grid.** One uniform square grid over the board outline (Edge.Cuts chained into closed loops, holes and
  cut-outs by even-odd; the bounding box if no closed outline exists), one node per cell and copper layer. The
  cell size follows the board: about 200 000 nodes over all layers (0.6–0.8 mm on the example boards), at least
  0.25 mm. Board-level spreading lengths are 10–30 mm (fin parameter 1/m = √(K/2h)), so this resolves the board
  field; it does not resolve the spreading right under a part smaller than a few cells (an error of up to
  ln(c/a)/(2πK) for a source of radius a in a cell of size c – a few K/W on a single 35 µm plane, less with
  several planes; θJB already contains the local spreading under the part on the JEDEC board).
- **Copper fill.** Each copper layer is rasterised on a finer sub-grid (≈ 0.1 mm, the DC solver's routines:
  zones of every net, tracks, pads) and averaged into the fill fraction f of each cell.
- **In-plane conductance per layer** (W/K per square): K = k_Cu·t_Cu·f + k_xy·(t_Cu·(1 − f) + ½(h_above + h_below));
  each copper layer node carries half of the dielectric above and below. Between two cells the harmonic mean.
- **Between layers:** k_z·c²/h per cell for the dielectric between them, plus k_Cu·A_barrel/Δz for every via and
  plated hole in that cell (between the layers it spans).
- **To the air:** (1/h + t_mask/k_mask)⁻¹·c² per cell on the top and bottom layer, h = h_conv + h_rad of that side.
  h depends on the temperature (Ra^¼, T³): kisi takes the board's mean surface rise from the energy balance
  P = (h_top + h_bot)·A·ΔT and iterates h(ΔT) on that scalar before the solve; local h variations are second
  order *(an estimate)*.
- **Sources:** copper loss per DC cell into the thermal cell under its centre on the same layer; parts into a
  junction node (two-resistor model) connected through θJB to the cells under their pads (weighted by pad
  area, on the layer the part sits on) and through θJC + 1/(h·A_top) to the air (A_top = area of the pads'
  bounding box, a stand-in for the body). With only ψJB the heat goes into the pad cells and Tj = T_B + ψJB·P;
  with only θJA the heat goes into the pad cells and Tj = T_amb + θJA·P (flagged).
- **Solve:** one sparse SPD system for the temperature rise over ambient (Cholesky, `kisi-sparse`), the same
  path as the DC solver. ThunderScope: ≈ 200 000 nodes.

## 4. Data
### 4.1 `data/thermal.json`
Copper (k, α, ρ₂₀), laminate (k in plane and through the thickness), solder mask (k, thickness, emissivity),
air (properties at 300 K for the correlations), the correlation constants, the laminate MOT warning level and the
defaults (ambient 25 °C, horizontal board in still air), each with its source.

### 4.2 Package defaults (`data/components.json` → `thermal`)
Families recognised from the footprint name (in this order; the first pattern that matches wins), values for a
reference size and an exponent for the size (pad bounding box area A): θ = θ_ref·(A_ref/A)^n.

| Family | Footprint names | Reference (datasheet) | θJB K/W | θJC(top) K/W |
|---|---|---|---|---|
| power | SOT-223, SOT-89, TO-252/DPAK, TO-263/D2PAK, TO-220, PowerPAK, LFPAK | TLV1117LV, DCY (SOT-223) [T6] | 12 | 47 |
| bga | BGA, CSP, FBGA | TMS570LC4357-EP, 337-ball BGA (16 × 16 mm) [T7] | 5.5 | 5.0 |
| qfn | QFN, DFN, SON, LGA, LFCSP, MLF, exposed pad (EP, PowerPAD) | TPS6215x, RGT 3 × 3 mm [T8]; CC1352P, RGZ 7 × 7 mm [T9] | 17.4 (3 × 3) … 8.0 (7 × 7) | 53.6 … 13.3 |
| qfp | LQFP, TQFP, QFP | MSP430F6723A, PN (LQFP-80) [T10] | 21.9 | 11.5 |
| so | SOIC, SOP, SSOP, TSSOP, MSOP, VSSOP | LM358, D (SOIC-8), PW (TSSOP-8) [T11] | 68 … 99 | 67 … 69 |
| sot | SOT-23, SC-70, SOT-353/363, TSOT | TLV704, DBV (SOT-23-5) [T12] | 77–97 | 69–111 |
| chip | two-pad chip parts (resistors, ferrites, …) | kisi estimate from the rating convention (155 °C film at 70 °C ambient and rated power, IEC 60115) | 150 (0603) | – |
| generic | everything else | kisi estimate | 40 (10 mm²) | 50 |

All of them are estimates for parts without `PI_THERMAL`: real values depend on the die size, lead frame and the
number of thermal vias.

### 4.3 The footprint field `PI_THERMAL`
Same form as the other PI_* fields: `key=value` tokens separated by spaces, values with or without units.
```
PI_THERMAL = theta_jb=17.4 theta_jc=53.6 tj_max=125
PI_THERMAL = psi_jb=11.9 tj_max=150
PI_THERMAL = theta_ja=45 tj_max=125            (fallback: JEDEC board, flagged)
PI_THERMAL = p=0                               (heat in the part: a connector that passes power on)
```
| Key | Meaning |
|---|---|
| `theta_jb` | junction-to-board thermal resistance, K/W (datasheet RθJB) |
| `theta_jc` | junction-to-case (top) thermal resistance, K/W (RθJC(top)); without it the top path is left out |
| `psi_jb` | junction-to-board characterisation parameter, K/W; used when `theta_jb` is not given |
| `theta_ja` | junction-to-ambient on the JEDEC board, K/W; only used when neither `theta_jb` nor `psi_jb` is given |
| `tj_max` | maximum junction temperature, °C |
| `p` | heat dissipated in the part, W (replaces the power from the power tree: V·I of a load, loss of a regulator) |

In `<board>.kisi.json` it is an ordinary `fields` entry with a source (datasheet, page, "Thermal Information"
table). `kisi config-check` reads it.

## 5. Results and front ends
- `kisi thermal <board> [--ambient °C] [--h W/m²K] [--airflow m/s] [--vertical]` (native and Node CLI, from
  `kisi_core::commands`): the power analysis, then the temperature: ambient and surface coefficients (marked as
  estimates unless given), totals and the energy balance, maximum board temperature and hot spots with the
  nearest part, per part P, the board temperature under it, Tj, Tj max and its source, the θ values and their
  source (`PI_THERMAL` or the package default), per rail the copper temperature (maximum and loss-weighted
  mean) and the IR drop at 20 °C and at that temperature. `report` includes it.
- Viewer, Power view: step 3 "Temperature (estimate)" with the ambient temperature and the cooling (still air
  horizontal or vertical, air flow 1 or 3 m/s), then the "Temperature" map per layer (°C, the hottest parts
  labelled with Tj), a side panel with the same tables as the CLI and the note that the values are estimates.

## 6. Scope
| Now (this increment) | Later |
|---|---|
| steady state, one board in air at ambient | transient (C and Δt on the same matrix), duty cycles |
| copper loss, part heat, θJB/θJC/ψJB/θJA, package defaults | heat sinks (`theta_sa`), thermal pads to a chassis, conduction into mounting holes and connectors |
| natural convection (horizontal, vertical), forced convection, radiation | enclosure air temperature, CFD-like local h |
| one electro-thermal step (copper resistance) | iteration to a fixed point; temperature-dependent IC power |
| uniform grid ≈ 200 000 nodes | local refinement under small hot parts |
| open via barrels, FR-4 values | filled vias, laminate data by material name |

## 7. Validation
### 7.1 Analytic cases (unit tests, `crates/kisi-pi/src/thermal.rs`)
*(results filled in below after the implementation)*

### 7.2 JEDEC θJA on a model of the JESD51-7 board

### 7.3 Example boards

## Sources
- [T1] Texas Instruments, SPRA953, Semiconductor and IC Package Thermal Metrics – https://www.ti.com/lit/an/spra953c/spra953c.pdf *(cited from earlier knowledge; definitions confirmed by [T2])*
- [T2] Analog Devices, Thermal Characterization of IC Packages (θJA, θJC, θJB, ψJB, ψJT, JESD51-x test boards) –
  https://www.analog.com/en/resources/technical-articles/thermal-characterization-of-ic-packages.html
- [T3] ROHM, Two-Resistor Model for Thermal Simulation (JESD15-3) –
  https://fscdn.rohm.com/en/products/databook/applinote/common/two_resistor_model_for_thermal_simulation-e.pdf
- [T4] F. P. Incropera, D. P. DeWitt, T. L. Bergman, A. S. Lavine, Fundamentals of Heat and Mass Transfer, 6th ed.,
  Wiley 2007: horizontal plates eq. 9.30–9.32, vertical plate (Churchill–Chu) eq. 9.26/9.27, laminar flat plate
  eq. 7.30, copper Table A.1, air Table A.4 *(textbook, cited from earlier knowledge)*
- [T5] FR-4 thermal conductivity in plane and through the thickness (measurements of Azar & Graebner, SEMI-THERM
  1996, and Sarvar et al., J. Electron. Mater. 1990) – https://en.wikipedia.org/wiki/FR-4
- [T6] TI, TLV1117LV datasheet SBVS160C, Thermal Information, DCY (SOT-223) –
  https://www.ti.com/document-viewer/lit/html/SBVS160C/GUID-8709A32C-BEF6-422B-A5D9-5BCC979D7382 (the table's row
  labels are swapped there: 12 K/W is the junction-to-board value, 47.2 K/W junction-to-case top)
- [T7] TI, TMS570LC4357-EP datasheet, thermal resistance of the 337-ball BGA –
  https://www.ti.com.cn/document-viewer/cn/TMS570LC4357-EP/datasheet/thermal-resistance-characteristics-for-the-bga-package-gwt-thermal-char-zwt337
- [T8] TI, TPS6215x datasheet SLVSAL5E, Thermal Information, RGT (VQFN-16, 3 × 3 mm) –
  https://www.ti.com/document-viewer/lit/html/SLVSAL5E/GUID-A4C7E975-BB33-458C-97B8-6CC60403872A
- [T9] TI, CC1352P datasheet, Thermal Resistance Characteristics, RGZ (VQFN-48, 7 × 7 mm) –
  https://www.ti.com/document-viewer/CC1352P/datasheet/GUID-XXXXXXXX-SF0T-XXXX-XXXX-000000175171
- [T10] TI, MSP430F6723A datasheet, thermal resistance, PN (LQFP-80) and PZ (LQFP-100) –
  https://www.ti.com.cn/document-viewer/cn/MSP430F6723A/datasheet/23-ZHCSDG4A
- [T11] TI, LM358 datasheet, Thermal Information, D (SOIC-8), PW (TSSOP-8), DGK (VSSOP-8) –
  https://ti.com/document-viewer/LM358/datasheet/GUID-6D406977-775A-4C11-AB6B-61A1BCD385E8
- [T12] TI, TLV704 datasheet, Thermal Information, DBV (SOT-23-5) –
  https://ti.com/document-viewer/TLV704/datasheet/GUID-XXXXXXXX-SF0T-XXXX-XXXX-000000316708
- [T13] IEC 60028, International standard of resistance for copper (ρ₂₀ = 1/58 Ω·mm²/m, α₂₀ = 0.00393 /K)
  *(standard, cited from earlier knowledge)*
- [T14] IEC 60115-1, fixed resistors: rated dissipation at 70 °C, upper category temperature 155 °C
  *(standard, cited from earlier knowledge)*

**Not read for this document:** the JEDEC standards themselves (JESD51-x, JESD15-3), the IPC-2152 charts (a
comparison of trace temperature rise against IPC-2152 would be a useful later check), AMD's package thermal data
for the Artix-7 CSG325.
