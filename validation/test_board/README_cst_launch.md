# CST model of the connector launch (Samtec 292-J-P-EP-ST-EL-01)

Status: **written and checked without CST (2026-10-31), not run in CST yet.** `cst_launch.py` builds the
test-board connector launch in CST Studio Suite **2025.5** with Samtec's STEP model of the 2.92 mm edge-launch jack,
so the whole transition – coax, support bead, pin, jaw, board launch – is computed, not only the board side that
kisi-fdtd covers (`README.md`, section 2). The model conventions are those of the CST cross-check of the 3D
references (`validation/fdtd/CST.md` on branch `feat/cst-refs`: zero-thickness PEC copper widened by Wheeler's Δw,
dielectric loss as a conductivity exact at fmax/2, time-domain solver with PBA, open boundaries).

| File | What |
|---|---|
| `cst_launch.py` | builds a model: the board from `make_coupons.launch_case()` (so it follows `connectors.json`), the connector from the STEP; runs it through CST's Python API, or writes a VBA macro (`.bas`) and the model as JSON (`--dry-run`) |
| `compare_launch.py` | CST against kisi-fdtd's board-side launch cases (`fdtd_ref/launch_jlc4_7628/`) and Samtec's report |
| `connectors.json` | `connector.step_model`: what the STEP contains (measured, the STEP itself is not here); `connector.benchmark.cst_model`: the assumed MT40 benchmark stackup |

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

## What the STEP contains and how it is used

Samtec's model (CADENAS, 7 solids in 5 parts, no materials; `connector.step_model` has the measured values) is
imported as it is and split by part name (`--part ROLE=PATTERN` overrides; CST's message window lists what was
found):

| STEP part | Role | In the model |
|---|---|---|
| `S-PRF92-J-P-EP-ST-EL-01` | body, bushing and both jaws – **one solid that also holds the support bead** | PEC (`--metal lossy`: stainless 1.4 MS/m) |
| `PRF92-J-P-2E-ST-EL-SK-01` | centre contact: socket, stepped body, launch pin Ø0.178 mm | PEC (lossy: gold 41 MS/m) |
| `PRF18-P-P-ST-EL-BP-01` | lower clamping plate | PEC, moved to the board's underside |
| `SHCS-1-72-06-X`, `WSHR-05-X` | screws, washers | dropped |

* **Support bead:** a disc Ø2.92 × 1.27 mm with six Ø0.70 mm holes and a Ø0.91 mm centre hole, 3.2–4.4 mm behind
  the mating plane. It is part of the body solid; the script splits it off (body ∩ cylinder) as a dielectric:
  `--bead-er` (default 2.1, PTFE – Samtec does not state the material), `--bead-tand` (0.0003).
* **The STEP is solid around the contact between the bead and the front lip** (STEP x 6.83–10.82 mm: the body
  overlaps the contact by 3.46 mm³, which would short it). The inside there is not modelled by Samtec, so the
  script cuts a coaxial bore of **assumed** impedance around each contact section (`--cavity-z`, default 50 Ω, air;
  `--cavity-er` for a dielectric fill). This is the largest assumption of the model: compare `--cavity-z 45 / 55`
  or `--cavity-er 2.1` to see how much it matters, and ask Samtec for the real section if it does.
* **Pin contact:** in the STEP the pin's underside floats 0.038 mm above the board (the real pin is pressed onto
  the pad); a PEC block from the copper to the pin axis, from the copper edge (0.2 mm) to the pin end, stands for
  the contact (`--no-pin-contact`: none).
* **Placement:** STEP x (axis) → model x, STEP y (up) → model z (rotation about x by 90°; the block checks that
  the lower plate ends up below the axis and turns the parts by 180° if CST's rotation sense differs), the jaw's
  underside on the top copper, the plate under the bottom copper (the STEP shows a 1.905 mm board, ours is 1.59 mm).
* **Mating plane:** the outer-conductor face (STEP x = 1.930 mm; drawing: 1.91 mm, "REF PLANE").
* `--clip-body 3.0` (default) cuts the outside of the body more than 3 mm above / below the axis (bushing, hex,
  jaw top) to save mesh cells; `--clip-body 0` keeps it.

## Ports and de-embedding

* **Port 1 (and 2 of `thru2`): waveguide port on the connector's coax.** A waveguide port must lie on the boundary
  of the domain, so the connector is cut inside its uniform Ø2.92/Ø1.27 air line (STEP x = 4.4 mm, between the socket
  and the bead; `--coax-cut`) and the port's reference plane is moved 2.47 mm forward to the mating plane
  (`ReferencePlaneDistance`; exact for the lossless air line). The socket region (mated with the cable) is the
  ideal line in this model. `--coax-port ext` instead adds the mating plug as an ideal air coax (outer conductor in
  the Ø4.61 counterbore, inner conductor and pin into the socket) 1 mm ahead of the front face (`--ext-len`) and
  refers the port back to the mating plane: the socket with its fingers is then in the model (more cells).
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

## Mesh and the 1 M cell licence

Mesh presets as `CST.md`: `fine` 30 lines per wavelength / 30 per box, `mid` 25/25, `base` 20/20, `coarse` 15/15
(ratio limit 20, edge refinement 4). The magnetic symmetry plane y = 0 (`--symmetry magnetic`, default; the model is
symmetric across the trace without the screws) halves the cells. The script prints an estimate per preset and
`--mesh auto` (default) takes the finest whose estimate stays below 85 % of `--max-cells` (1 M). The estimate is
**rough** (fitted to CST.md's 101 fine-mesh counts: ±10 % there, up to ±40 %; here most of the domain is air or
metal, which it may overrate – a second number assumes CST meshes all of it at the board's wavelength):

| Model | fine | mid | base | coarse | `auto` |
|---|---|---|---|---|---|
| `launch1` | 2.1 M | 1.5 M | 0.95 M | 0.60 M | coarse |
| `thru2` | 4.1 M | 2.9 M | 1.9 M | 1.2 M | coarse (above 1 M) |
| `samtec_bench` (2 / 1 connectors) | 4.4 / 2.2 M | 3.2 / 1.6 M | 2.3 / 1.2 M | 1.0 / 0.53 M | coarse |

So check the real count before solving: build the model, then **Home > Mesh View** (or the solver's log) shows the
cells; the licence refuses a mesh above 1 M at the solver start ("… up to 1000000 mesh cells"). With the Python driver
`--mesh fine --fallback` tries fine, then mid, base and coarse until CST accepts one, and records which in
`cst_meta.json`. Other levers: `--lpw/--lpb` (own values), `--clip-body 2`, `--air 0.5`, `--coax-cut 4.9`.
Recommended order: `launch1` (finest mesh that fits) → `compare_launch.py --cascade launch1` → `thru2` at what
fits → `samtec_bench --connectors 1` and cascade.

## Running it

**Inside / next to CST 2025.5 with Python** (CST's Python 3.11 with `cst.interface`, as `run_cst.py` on
`feat/cst-refs`; numpy is not needed):

```
"C:\Program Files\CST Studio Suite 2025\AMD64\python\python.exe" cst_launch.py --model launch1 ^
    --step C:\samtec\292-J-P-EP-ST-EL-01.stp --out C:\kisi\cst_launch --mesh fine --fallback [--monitors]
```

It builds the project from the history blocks (a failing block is reported with its caption), saves
`OUT/<name>/<name>.cst`, solves (`--no-solve`: build only) and writes `OUT/<name>/cst_<mesh>.s2p` (CST's 1001
samples, 0–40 GHz, 50 Ω; `launch1` also `cst_<mesh>_s11.s1p`), `cst_meta.json`, the `.bas` and the model JSON.
From the repository it needs `make_coupons.py`, `connectors.json`, `stackups.json`, `boards/<stackup>.design.json`
and `validation/fdtd/{via_cases,kisi_board}.py`; outside it, rebuild from a dry-run JSON:
`cst_launch.py --from-json launch1_default_jlc4_7628.model.json --step …`.

**Without Python: the VBA macro.** `--dry-run` writes `<name>.bas`; in CST: new project (any template, the macro
sets units, background, boundaries, frequency), **Home > Macros > Run Macro**, pick the `.bas`; it asks for the STEP
(or set `stepFile` at its top). Every block goes into the history (**Modeling > History List**), so the model stays
parametric and can be edited and rebuilt. To solve and export from the macro set `RUN_SOLVER = True` and `outDir`;
otherwise start the solver by hand and export **Post-Processing > Import/Export > Touchstone** (50 Ω).

**Your own plots:** `--monitors` adds E-field monitors at 10, 20, 30 and 40 GHz (off by default: they cost memory
and time); the 2D/3D results then show the field in the bead, at the pin and in the launch. The port modes
(**2D/3D Results > Port Modes**) show the coax and line modes and the line impedance.

**Geometry check without CST:** `python3 cst_launch.py --model launch1 --dry-run --out OUT [--step …]` writes the
plot `<name>.png` (top view of the copper, vias, ports and the connector outline; section through the axis with the
bead, the assumed bore and the pin block; with `--step` and gmsh installed the STEP's own section is drawn).

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

## Checked without CST

* `--dry-run` for all three models and the variants: the VBA blocks and the macro pass a structural check (With /
  If / For blocks, quotes, ASCII; the patterns of `cst_case.py`); the model JSON round-trips (`--from-json`).
* The connector operations (bead split, bore, cut at the port, body clip, pin block, rotation and placement) were
  repeated with OpenCASCADE (gmsh 4.15) on Samtec's STEP: no overlap of contact and body or bead, the bead
  4.52 mm³ (disc minus holes and contact, as computed), the plate below the axis after the +90° rotation, the jaw on
  the top copper, the plate on the bottom copper, the pin block on the trace tip, no connector solid inside a board
  or mask brick.

## To check first in CST

1. The connector block: the STEP import (`With STEP`), the solid names CST gives the imported parts (the message
   window lists the classification; `--part` if a name differs), `Solid.GetNumberOfShapes` /
   `GetNameOfShapeFromIndex` / `GetLooseBoundingBoxOfShape` (`--no-orient-check` drops the last).
2. The orientation in the 3D view: jaw on top of the board, pin on the trace tip, plate underneath, the coax port
   inside the bore.
3. The port modes: one TEM mode on the coax (50.0 Ω), one quasi-TEM mode on the line (about 50 Ω); the line port
   should not show a second mode below 40 GHz.
4. The mesh cell count against the licence, then the finest preset that fits.
5. The assumed bore (`--cavity-z`): its effect on |S11|.
