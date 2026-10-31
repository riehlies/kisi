# CST model of the connector launch (Samtec 292-J-P-EP-ST-EL-01)

Status: **first CST run 2026-10-31 with the STEP import failed (total reflection, see "History"); the
connector is now built from CST primitives (`--connector parametric`, default), checked without CST, to be run
again.** `cst_launch.py` builds the test-board connector launch in CST Studio Suite **2025.5**: Samtec's 2.92 mm
edge-launch jack with its coax, support bead, pin and jaw on the board launch, so the whole transition is computed,
not only the board side that kisi-fdtd covers (`README.md`, section 2). The model conventions are those of the CST
cross-check of the 3D references (`validation/fdtd/CST.md` on branch `feat/cst-refs`: zero-thickness PEC copper
widened by Wheeler's Δw, dielectric loss as a conductivity exact at fmax/2, time-domain solver with PBA, open
boundaries).

| File | What |
|---|---|
| `cst_launch.py` | builds a model: the board from `make_coupons.launch_case()` (so it follows `connectors.json`), the connector from primitives with the dimensions measured from Samtec's STEP; runs it through CST's Python API, or writes a VBA macro (`.bas`) and the model as JSON (`--dry-run`) |
| `compare_launch.py` | CST against kisi-fdtd's board-side launch cases (`fdtd_ref/launch_jlc4_7628/`) and Samtec's report |
| `connectors.json` | `connector.step_model`: what the STEP contains (measured; the STEP itself is not here); `connector.benchmark.cst_model`: the assumed MT40 benchmark stackup |

## Models

| `--model` | What | Ports |
|---|---|---|
| `launch1` | one connector + 12.5 mm feed (`layout.feed`: half of the 25 mm 2×thru) on JLC04161H-7628 (`--stackup`), launch from `connectors.json` (`--variant`) | 1: coax, 2: line end |
| `thru2` | the 2×thru as measured: two connectors, 25 mm board | 1, 2: coax |
| `samtec_bench` | two connectors + 25.4 mm (`--connectors 1`: one + 12.7 mm) 50 Ω grounded coplanar line on an **assumed** 4-layer I-Tera MT40 board: a plausibility check against Samtec's report, not their fixture | coax (and line end) |

`--variant`: `default` (`connectors.json` launch: the line width to the edge, fence 0.55 mm), the kisi-fdtd variants
with results in `fdtd_ref/` (`notip_f05`, `notip_f08`, `tip023_f08`, `samtec`, `samtec_cut`), and two with Samtec's
footprint values and our fence: `tip023` (0.23 mm tip) and `samtec_pad` (0.52 mm pad, gap 1.19 mm, 0.23 mm tip).
`--walls` adds the GND via walls between the coupon rows (y = ±6 mm; the kisi-fdtd launch cases have none).

## The connector

**`--connector parametric` (default):** the connector is built from CST primitives directly in model coordinates
(x from the board edge, z up, the coax axis 0.127 mm above the top copper): no import, no scaling, no rotation,
so a history rebuild always gives the same model. Dimensions from Samtec's STEP model (`connector.step_model`,
measured with OpenCASCADE; drawing values where they exist):

| Part | Model | Material |
|---|---|---|
| body (outer conductor) | Ø6.35 bushing to x = 5.63, then the Ø9.53 cylinder, flat at ±3 mm about the axis (`--clip-body`); ahead of the board edge the tongue (2.54 mm wide) that meets it; over the board the jaw (underside on the top copper, top 1.78 mm above the axis) with the slot over the trace (2.67 / 3.18 mm wide, open to the top) and the two screw slots | PEC (`--metal lossy`: stainless 1.4 MS/m) |
| inside the body | Ø2.921 air line from the port plane to the bead; the bead's seat; the **assumed** bore (below); the Ø0.70 opening of the front lip for the pin | air |
| support bead | disc Ø2.921 × 1.27 mm, six Ø0.70 mm holes on Ø2.01 mm, Ø0.91 mm centre hole, minus the contact's shoulder | `--bead-er` 2.1 (PTFE assumed: Samtec does not state it), `--bead-tand` 0.0003 |
| contact | Ø1.27 / 0.914 (groove in the bead) / 1.27 / 1.016 / 0.254, the launch pin Ø0.178 to 0.686 mm beyond the board edge, solid; a PEC block from the copper to the pin axis (pin width, from the copper edge at 0.2 mm to the pin end) for the pressed contact (`--no-pin-contact`: none) | PEC (lossy: gold 41 MS/m) |
| lower plate | 5.33 × 8.89 × 1.37 mm under the bottom copper, with its 0.51 × 0.08 mm groove under the trace | as the body |

* **The assumed bore:** the STEP is solid around the contact between the bead and the front lip (STEP x
  6.83–10.82 mm: the body overlaps the contact by 3.46 mm³); Samtec does not model the inside there. The script puts
  a coaxial bore of **assumed** impedance around each contact section (`--cavity-z`, default 50 Ω, air;
  `--cavity-er` for a dielectric fill). Where the bore narrows at the contact's last shoulder (Ø1.016 → Ø0.254 at
  STEP x 10.69) the wider bore runs on 0.06 mm past it – otherwise the shoulder face lies in the plane of the bore's
  step and touches it (a short; the first STEP-based model had this). This is the largest assumption: compare
  `--cavity-z 45 / 55` or `--cavity-er 2.1`, and ask Samtec for the real section if it matters.
* **Not modelled:** the thread, the hex flats, chamfers and roundings outside the coax, the screws (they press the
  jaw and the plate onto the ground pads; the plated holes are in the board model).
* **Compared with Samtec's STEP** (`--check-occ --step …`, OpenCASCADE; the STEP cut and clipped the same way):
  contact 5.773 / 5.773 mm³ (differing volume 0.0002), bead 4.523 / 4.524 (0.0008), body 347.3 / 329.2 mm³ (34 mm³
  differ: thread, hex, chamfers; inside the coax region they agree to 0.001 mm³), plate 64.8 / 58.4 (its tapped
  holes). The plot `<name>.png` overlays both sections (STEP dashed).

**`--connector step`:** Samtec's STEP imported as before, for comparison only (`--step`; `--step-scale 25.4`:
the file is in inches and CST 2025.5 imported it without converting; parts by name, `--part ROLE=PATTERN`; the
bead split off the body; the socket's bore at the port plane filled, as the mated pin does – open, it is a tube
with a decoupled region inside, which the port solver showed as a second mode). The import, rotation and the
orientation check are not replay-safe (after a reload the connector sat on the other side of the board).

## Ports and de-embedding

* **Port 1 (and 2 of `thru2`): waveguide port on the connector's coax.** A waveguide port must lie on the boundary
  of the domain, so the connector begins inside its uniform Ø2.921/Ø1.27 air line (STEP x = 4.4 mm, between the
  socket and the bead; `--coax-cut`) and the port's reference plane is moved 2.47 mm forward to the mating plane
  (`ReferencePlaneDistance`; exact for the lossless air line). The port is the square that circumscribes the outer
  conductor (2.921 mm, centred on the axis; `--coax-port-half`): its corners are metal, the only open region is the
  annulus, so there is exactly one TEM mode (60 ln(2.921/1.27) = 49.97 Ω) and nothing else below the TE11 cutoff
  (about 46 GHz). One mode per port (`NumberOfModes 1`). The socket region (mated with the cable) is the ideal line in
  this model.
* **Port 2 of `launch1`: waveguide port on the line cross-section** at the end of the 12.5 mm feed (2.8 mm wide,
  from In1 to 1.2 mm above L1; `--line-port-half`, `--line-port-height`), the board and planes ending there. All
  S-parameters are renormalised to 50 Ω (`AutoNormImpedance`). `launch1` is then "one connector + 12.5 mm", the
  structure of Samtec's "one connector + 500 mil" (12.7 mm). `--port2 face` uses a 50 Ω discrete face port from In1
  to the line end as kisi-fdtd and `cst_case.py` do (the board runs on 1.5 mm into the PML).
* **De-embedding the feed:** for |S11| nothing is needed (the line is matched to within 0.3 Ω). To refer port 2 to
  the end of the launch (6.48 mm) set its `ReferencePlaneDistance` to −6.02 mm in CST (Simulation > Ports, it uses
  the line's own propagation constant) – the reflection of the connector and launch alone, comparable with
  kisi-fdtd's edge port. **2×thru from `launch1`:** `compare_launch.py --cascade launch1` cascades the result with
  itself reversed (exact for the network when only the line mode links the halves) – the 2×thru where the full
  `thru2` does not fit the mesh; `thru2` at a coarser mesh checks it.

## Checks in the model

* **History block "check: conductors at the coax ports"** (runs on every rebuild): every waveguide port must lie on
  the model's x extent (a part beyond it would move the boundary and leave the port inside), the contact must reach
  the port plane on the axis with its 1.27 mm, the outer conductor must reach it and enclose the bore; otherwise the
  build stops with `ReportError` and the reason; the message window lists the boxes.
* **Port-mode check before the solve:** the macro (`PORT_CHECK = True`) and the Python driver (unless
  `--no-port-check`) run the solver for the port modes only, then require mode 1 of every port to be (quasi-)TEM with
  47–53 Ω on the coax and 40–60 Ω on the line; the macro stops with a message box, the driver with an error, and both
  report the impedances (the Python driver reads them from the results; if a CST version names them differently it
  says so and goes on).
* **Results:** the driver reads S-parameters only by their fundamental-mode names (`S2(1),1(1)` or `S2,1`) and stops
  if one is missing, if a higher mode (`S1(2),1(1)`) is present, or if a transmission is identically zero; the
  solver excites all ports (`StimulationPort "All"`). The mesh cells (from the solver log) go into `cst_meta.json`
  and the Touchstone header.
* **Without CST** (`--check-occ`, pip `gmsh`): the connector built with OpenCASCADE from the same operation list:
  no overlap of contact, body, bead and plate, at least 0.02 mm between the contact and the body or plate (the
  contact grown by 0.02 mm still touches neither), the jaw on the top copper and not below it, no body copper on the
  signal trace, the pin block on the trace, nothing inside a board or mask brick.

## Mesh and the 1 M cell licence

Mesh presets as `CST.md`: `fine` 30 lines per wavelength / 30 per box, `mid` 25/25, `base` 20/20, `coarse` 15/15
(ratio limit 20, edge refinement 4) and `rough` 12/12 (edge refinement 2, ratio limit 10) as a last resort;
`--edge-ratio`, `--ratio-limit`, `--lpw`, `--lpb` set them by hand (also with `--from-json`). The magnetic symmetry
plane y = 0 (`--symmetry magnetic`, default; the model is symmetric across the trace without the screws) halves the
cells. The script prints an estimate per preset and `--mesh auto` (default) takes the finest whose estimate stays
below 85 % of `--max-cells` (1 M). The estimate is **rough** (fitted to CST.md's 101 fine-mesh counts with edge
refinement 4: ±10 % there, up to ±40 %; a second number assumes CST meshes all of it at the board's wavelength; edge
refinement 2 needs fewer cells than it says):

| Model | fine | mid | base | coarse | rough | `auto` |
|---|---|---|---|---|---|---|
| `launch1` | 2.1 M | 1.5 M | 0.95 M | 0.60 M | 0.30 M | coarse |
| `thru2` | 4.1 M | 2.9 M | 1.9 M | 1.2 M | 0.58 M | rough |
| `samtec_bench` (2 / 1 connectors) | 4.2 / 2.1 M | 3.0 / 1.5 M | 2.2 / 1.1 M | 0.97 / 0.50 M | 0.53 / 0.27 M | rough / coarse |

The first run (STEP connector, launch1, coarse with edge refinement 2 and ratio limit 10) fitted the licence and
took 122 s. Check the real count before solving: **Home > Mesh View** (or the solver's log) shows the cells; the
licence refuses a mesh above 1 M at the solver start ("… up to 1000000 mesh cells"). With the Python driver
`--mesh fine --fallback` tries fine, mid, base, coarse and rough until CST accepts one and records which in
`cst_meta.json`. Other levers: `--clip-body 2`, `--air 0.5`, `--coax-cut 4.9`. Recommended order: `launch1`
(finest mesh that fits) → `compare_launch.py --cascade launch1` → `thru2` at what fits → `samtec_bench --connectors
1` and cascade.

## Running it

**With Python and CST's library.** CST 2025.5 brings no Python of its own for the API: use your own Python
(3.9–3.12; 3.12.1 works with CST 2025.5, tested 2026-10-31) and put CST's `python_cst_libraries` on its path.
Outside the repository the model comes from the package's `.model.json` (`--from-json`). In PowerShell, one line
per command:

```powershell
$env:PYTHONPATH = "C:\Program Files (x86)\CST Studio Suite 2025\AMD64\python_cst_libraries"
python cst_launch.py --from-json launch1_default_jlc4_7628.model.json --out "C:\kisi\cst_launch" --mesh fine --fallback
python cst_launch.py --from-json launch1_default_jlc4_7628.model.json --out "C:\kisi\cst_launch" --mesh coarse --edge-ratio 2 --ratio-limit 10
```

The parametric connector needs no STEP. For the STEP comparison model (from the repository): `python cst_launch.py
--model launch1 --connector step --step "D:\path\to\292-J-P-EP-ST-EL-01.stp" --step-scale 25.4 --out …`.

(cmd: `set PYTHONPATH=…` and the same `python …` line; `--monitors` at the end for field plots.) From the
repository, `--model launch1` instead of `--from-json …` builds the model from `make_coupons.py`.
Find the library with `Get-ChildItem "C:\Program Files*\CST Studio Suite 2025" -Directory -Recurse -Filter
python_cst_libraries`.

It builds the project from the history blocks (a failing block is reported with its caption), saves
`OUT/<name>/<name>.cst`, solves (`--no-solve`: build only) and writes `OUT/<name>/cst_<mesh>.s2p` (CST's 1001
samples, 0–40 GHz, 50 Ω; `launch1` also `cst_<mesh>_s11.s1p`), `cst_meta.json`, the `.bas` and the model JSON.
From the repository it needs `make_coupons.py`, `connectors.json`, `stackups.json`, `boards/<stackup>.design.json`
and `validation/fdtd/{via_cases,kisi_board}.py`; outside it, rebuild from a dry-run JSON (`--from-json`; the mesh
options apply to it). A failed run leaves the project open in CST and saves it as `<name>_failed.cst`
(`--keep-open` also after a successful run).

**Without Python: the VBA macro.** `--dry-run` writes `<name>.bas`; in CST: new project (any template, the macro
sets units, background, boundaries, frequency), **Home > Macros > Run Macro**, pick the `.bas` (a `--connector
step` macro asks for the STEP, or set `stepFile` at its top). After building it runs the port-mode check
(`PORT_CHECK`). Every block goes into the history (**Modeling > History List**), so the model stays
parametric and can be edited and rebuilt. To solve and export from the macro set `RUN_SOLVER = True` and `outDir`;
otherwise start the solver by hand and export **Post-Processing > Import/Export > Touchstone** (50 Ω).

**Your own plots:** `--monitors` adds E-field monitors at 10, 20, 30 and 40 GHz (off by default: they cost memory
and time); the 2D/3D results then show the field in the bead, at the pin and in the launch. The port modes
(**2D/3D Results > Port Modes**) show the coax and line modes and the line impedance.

**Geometry check without CST:** `python3 cst_launch.py --model launch1 --dry-run --check-occ --out OUT [--step …]`
writes the plot `<name>.png` (top view of the copper, vias, ports and the connector outline; section through the
axis: with gmsh the parametric connector's own section, with `--step` Samtec's model dashed over it; the bead, the
assumed bore, the pin block) and `<name>.occ_check.json`.

## Comparing

```
python3 compare_launch.py --cst launch1=OUT/launch1_default_jlc4_7628/cst_base.s2p --cascade launch1 \
    --cst thru2=OUT/thru2_default_jlc4_7628/cst_coarse.s2p --kisi default --out compare_launch.png
```

prints max |S11| (0.5–40 GHz) with its frequency and VSWR, |S11| at 10/20/30/40 GHz and |S21| at 40 GHz, and plots
|S11| and |S21| with Samtec's numbers (connectors.json `benchmark`: one connector + 500 mil VSWR ≤ 1.11–1.15,
|S21| −0.94…−1.05 dB at 40 GHz; two connectors + 1 in VSWR ≤ 1.21–1.32 at 36–37 GHz, −1.9…−2.1 dB). What to
expect: kisi-fdtd's `default` case is −21.0 dB worst to 40 GHz on the board side; with the connector the launch adds
the pin, the lip and the jaw, so CST's |S11| is the number for the target (−20 dB for one launch). Our line loses more
than Samtec's (FR-4 tan δ 0.02 against MT40 0.003): compare |S21| with `samtec_bench`, not with the report directly.

## History

* 2026-10-31, first CST runs (maintainer, CST 2025.5, `--connector step` as it was then): the STEP is in inches and
  was imported unscaled (`--step-scale` added); with the scale the model built and ran (launch1, coarse, 122 s), but
  CST listed only S1,1 with |S11| −0.00…−0.03 dB over 0–40 GHz (a short at port 1; the Touchstone file had S21 = 0;
  the reader now stops on a missing or all-zero entry) and warned of two decoupled modes at port 1: the port plane cut through the socket's open bore (the
  contact a tube there) and the bore's step touched the contact's last shoulder (both fixed); after a crash and a
  reload the connector sat on the other side of the board (import / rotation / orientation check not replay-safe).
  Hence the parametric connector, the conductor and port-mode checks and the strict result reading.

## Checked without CST

* `--dry-run` for all three models, the variants and the options: the VBA blocks and the macro pass a structural
  check (With / If / For blocks, quotes, ASCII; the patterns of `cst_case.py`); the model JSON round-trips.
* `--check-occ` for launch1, thru2 (both connectors), samtec_bench (1 and 2) and launch1 `samtec_pad` with a filled
  bore: all checks above pass; compared with the STEP as listed under "The connector".
* The S-parameter picking (`pick_sparam_items`) on the name forms `S2(1),1(1)` and `S2,1`, a missing entry and a
  second mode.

## To check first in CST

1. The 3D view: jaw on the board's top copper, pin on the trace tip, plate underneath, the port squares at both ends
   (the message window: "model x extent", the contact and body boxes).
2. The port-mode check: coax mode 1 TEM, about 50.0 Ω; line mode 1 quasi-TEM, about 50 Ω (2D/3D Results > Port Modes:
   the radial coax field, no second mode). `Port.GetModeType` / `GetLineImpedance` are used by the macro; if a CST
   version lacks them, set `PORT_CHECK = False` and look at the port modes by hand.
3. S1,1, S2,1, S1,2, S2,2 all in 1D Results > S-Parameters; |S21| near 0 dB at low frequency.
4. The mesh cell count against the licence, then the finest preset that fits.
5. The assumed bore (`--cavity-z`): its effect on |S11|.
