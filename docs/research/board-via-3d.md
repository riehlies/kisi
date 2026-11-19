# Board vias in 3D: the exporter, its cost and the integration into the viewer

Status: design note (November 2026). kisi's exporter cuts one via, or the via pair of a differential pair, out of
the real layout as a kisi-fdtd case (`kisi-validate via-case`); the 3D results of three critical vias of the example
boards against kisi's via ladder are in [`validation/vias.md`](../../validation/vias.md). This note keeps what the
exporter approximates, what a 3D run of a board via costs, and what a "click a via → 3D" check in the viewer needs.
The numbers were measured on 2026-11-13 on a 2-vCPU workspace; the hosts named below are as of that date (since
2026-11-19 the 3D view is planned for the separate field page `kisi-advanced.html`). Paths are relative to
`validation/fdtd/` unless they start with `crates/` or `web/`.

## Exporter: `kisi-validate via-case` (`crates/kisi-cli/src/viacase.rs`)

```
kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR
    [--at X,Y | --via K] [--layers IN,OUT] [--config F | --no-config] [--keep 1.2] [--keep-cell 0.05]
    [--rotate DEG] [--feed 4.0] [--margin 2.5] [--fmax 40e9] [--dfine 0.035] [--name ID] [--list] [--scan]
```

It loads the board with kisi's own parser and configuration (`Project::load_with`), so stackup overrides from
`<board>.kisi.json`, plane nets and the reference planes of the extraction are those kisi uses.

1. **Via and partner:** the via of the net (`--via`, `--at`; `--list` shows them, `--scan` lists every signal
   via of a board with the layers its tracks join). For a differential pair the partner via is found as in
   `kisi_si::via` (nearest via of the partner net within 2 mm). The origin is the via, or the middle of the pair.
2. **Feeds:** the tracks of the net (and of the partner) that leave the via on its two connected layers are
   followed outwards. Their exit direction decides the rotation of the cut-out: 0° or 45°, whichever brings both
   exits closer to the axes (45° routing then runs along the mesh). Inside the **keep box** (±1.2 mm) the traces
   are taken as routed (round-ended segments as polygons, axis-parallel pieces as rectangles with the thirds rule);
   at the box edge each continues straight along the nearest axis to a 50 Ω lumped port 4 mm from the origin.
3. **Copper:** on every layer without a feed, the zone fills of all other nets (KiCad's filled polygons, clipped to
   the window), pads (plated holes as barrels) and other tracks inside the window (keep box + 2.5 mm margin, 1.5 mm
   beyond the ports). On the two feed layers only the keep box is taken from the board.
4. **Coplanar copper:** if a pour runs closer than 3 h (at most 0.6 mm) beside a feed at the keep-box edge, it
   continues along the extension as a sheet with that gap and a via fence (r 0.15 mm, pitch 1 mm) to every plane of
   its net at the port (planes of other nets get a 0.35 mm clearance). The board's own vias in that corridor are
   left out. The fixture has the same sheet and fence, so it is the same line.
5. **Vias:** the signal via (and the partner): barrel of the drill radius, annular rings by KiCad's rules (every
   layer, or only the connected ones and with `keep_end_layers` the ends for `remove_unused_layers`), the
   back-drilled part removed. Every other via in the window: plane-net vias as return vias, other nets' vias with
   their rings (floating).
6. **Stackup:** kisi's copper heights (dielectric plus half the copper on each side, as the ladder), zero-thickness
   copper, air 1 mm above and below, dielectric loss as in the references.
7. **Fixtures:** the 2×thru of each side is the straight feed (single trace or pair with its pitch), 2 × 4 mm,
   with solid sheets on the layers that have copper at the port in the case, their nets, and the coplanar sheet and
   fence if any. Pairs: a 4-port thru, de-embedded in the differential mode.
8. **Ladder match:** the case's `meta.kisi_returns` lists the return vias as kisi's via model selects them (plane-net
   vias within 3 mm, nearest 8); `board_vias.py ladder` takes the ladder geometry that kisi's channel prints with
   `KISI_DEBUG_VIA=1` for that via (for a pair the differential channel's odd-mode ladder).

### What the exporter approximates
- Copper has zero thickness; the heights are kisi's (dielectric plus half the copper), as in the ladder. Together
  with the missing solder mask this makes the lines of the cut-out 5–26 % higher in impedance than kisi's 2D
  solution of the same trace: REFINOUT2 64 / 62 Ω against 51 / 48 Ω, M.2 115 / 94 Ω differential against 91 / 85 Ω,
  LibreVNA 53 / 52 Ω against 49 / 50 Ω (in / out side; 3D from the 2×thru, `line_z_3d_ohm` in
  `results/board_vias.json`). The matched de-embedding takes care of the lines themselves (`validation/vias.md`); the trace pieces
  at the via (over the antipad, into the pad) keep the same height convention in both models.
- Outside the window nothing is modelled; copper that reaches the window edge continues through the PML, so planes
  are infinite there and fills cut by the window do not end.
- On the feed layers only the keep box comes from the board. Outside it the feed is a straight trace (or pair)
  along the nearest axis, with a coplanar pour only if one runs within 3 h at the box edge. A route that bends
  outside the box, a neck-down or a different pair pitch further out are not in the case. A bend at the box edge
  (for routing that is neither 0°/90° nor 45° to the other feed) stays in the de-embedded result.
- Traces of other nets on the feed layers are cut at the keep box (floating ends).
- Other vias and tracks of the own net and the partner net are left out (a second via of the lane close by is not
  in the case).
- The barrel is a solid cylinder of the drill radius (no plating thickness, no hole wall); solder mask is ignored.
- Dielectric loss is a conductivity fixed at fmax/2: tanδ is exact at 20 GHz and grows towards low frequencies
  (at 0.1 GHz the de-embedded |S21| is 0.2 dB low). The comparison uses the ladder with the same loss model
  (`ladder_3dloss.s2p`); copper is lossless in both.
- Zone fills are taken as filled in the file (no refill); thermal reliefs and islands are in the polygons.


## Resource estimate

### Measured on the board cases
One excitation, one thread, the case's own mesh; setup = geometry, conformal faces and coefficients (one time
step), step rate from 1 and 4001 steps, peak memory of the process (`board_vias.py speed --threads 2`, a command the validation clean-up of 2026-11-17 removed; in the git history before
1c5575f). Workspace: 2 vCPU (Xeon, 2.1 GHz nominal) shared with another agent; native and
WebAssembly (node 22, V8, SIMD build) back to back.

| case | cells | native setup | native, 1 thread | native, 2 threads | WASM setup | WASM step rate | memory native / WASM |
|---|---|---|---|---|---|---|---|
| M.2 pair | 1.03 M | 6.1 s | 80 Mcells/s | 86 Mcells/s | 13.3 s | 67 Mcells/s | 82 / 79 MB |
| REFINOUT2 | 1.19 M | 4.7 s | 80 Mcells/s | (68)* | 10.0 s | 69 Mcells/s | 88 / 85 MB |
| LibreVNA | 1.53 M | 11.0 s | (75)* | (56)* | 23.8 s | (66)* | 113 / 110 MB |

\* another job compiled on the second vCPU during these measurements.

- **WebAssembly runs at 0.84–0.88 of native** with SIMD (`-C target-feature=+simd128`); without SIMD it was 21 against
  50 Mcells/s (M.2, earlier measurement under load). The SIMD build is the one to ship (all current browsers run
  SIMD128). The module is 370 kB.
- **Memory 77–84 bytes per cell**, as for the references (77 B/cell): about 80–120 MB per worker for these cases.
- **Two threads gave only +8 %** (M.2, idle machine): the field update is bound by memory bandwidth on this VM.
- **The setup is large**: 4.7–11 s natively and 10–24 s in WebAssembly, against 0.7 s for a reference case of similar
  size (the 0.77 M-cell pair E_s8_L1L8_p1000_round_g2). The conformal faces sample the geometry 576 times per face
  and every sample tests the polygons; worth optimising before an integration (analytic free fractions for
  straight polygon edges, fewer samples where only one boundary crosses a face).
- **Steps:** 29–43 k per excitation to −50 dB of the peak energy at the 0.035 mm mesh (time step 50–54 fs,
  1.5–2.3 ns); the references need 4–40 k. The board vias ring longer (weakly damped plane cavities, resonances
  above 25 GHz). Stopping at −35 dB halves that (15–24 k) and changes |S21| by at most 0.004 / 0.012 / 0.03 dB up
  to 20 GHz (M.2 / REFINOUT2 / LibreVNA; the deep LibreVNA notch moves by 0.1 GHz). The small step of the M.2
  |Sdd21| at 20.5 GHz disappears with it: a late-time artefact.
- **Whole cases, native, one thread, −50 dB** (as computed here, the workspace shared): M.2 pair 1737 s
  (4 excitations), REFINOUT2 1125 s, LibreVNA 1119 s (2 each), i.e. 69–105 Mcells/s averaged over the run; the
  2×thru fixtures 15–39 s each (3.4–4.2 k steps).

### Quick mesh
Coarser variants of each case (`board_vias.py quick`, removed with `speed`: the exporter with `--dfine`, `--keep-cell`, `--fmax`, shorter
feeds and margin, and the stop level), de-embedded the same way and compared with the standard result up to 20 GHz.
Time = all excitations and both fixtures, native, one thread, on the shared workspace (the load varied, so the
ratios are approximate; the step counts are exact).

| case | mesh, fmax, stop | cells | steps | time | vs standard | max Δ\|S21\| | Δphase | max \|ΔS11\| | ΔC | |
|---|---|---|---|---|---|---|---|---|---|---|
| M.2 pair | 0.035 mm, 40 GHz, −35 dB | 1.03 M | 18.4–21.0 k | 902 s | 2.0× | 0.00 dB | 0.0° | 0.001 | 0 fF | |
| M.2 pair | 0.05 mm, 25 GHz, −50 dB | 0.52 M | 33.0–39.8 k | 807 s | 2.2× | 0.11 dB | 6.7° | 0.026 | +15 fF | |
| M.2 pair | **0.05 mm, 25 GHz, −35 dB** | 0.52 M | 8.0–13.0 k | 239 s | 7.6× | 0.11 dB | 6.7° | 0.026 | +15 fF | |
| M.2 pair | 0.07 mm, 25 GHz, −50 dB | 0.34 M | 24.4–29.2 k | 381 s | 4.7× | 0.23 dB | 11.8° | 0.157 | +24 fF | too coarse for the 0.13 mm gap |
| M.2 pair | 0.07 mm, 25 GHz, −35 dB | 0.34 M | 5.6–9.6 k | 124 s | 15× | 0.23 dB | 11.6° | 0.151 | +24 fF | |
| REFINOUT2 | 0.035 mm, 40 GHz, −35 dB | 1.19 M | 15.0–19.4 k | 479 s | 2.5× | 0.01 dB | 0.1° | 0.003 | 0 fF | |
| REFINOUT2 | 0.05 mm, 40 GHz, −50 dB | 0.83 M | 22.6–27.2 k | 440 s | 2.7× | 0.01 dB | 1.3° | 0.030 | +12 fF | |
| REFINOUT2 | 0.05 mm, 25 GHz, −50 dB | 0.61 M | 5.0–8.0 k | 108 s | 11× | 0.10 dB | 1.3° | 0.032 | +12 fF | |
| REFINOUT2 | 0.05 mm, 25 GHz, −35 dB | 0.61 M | 3.8 k | 77 s | 15× | 0.13 dB | 1.4° | 0.033 | +10 fF | |
| REFINOUT2 | **0.07 mm, 25 GHz, −50 dB** | 0.35 M | 3.4–6.8 k | 51 s | 23× | 0.07 dB | 3.2° | 0.021 | +14 fF | |
| REFINOUT2 | 0.07 mm, 25 GHz, −35 dB | 0.35 M | 2.6 k | 34 s | 35× | 0.17 dB | 2.9° | 0.020 | +12 fF | |
| LibreVNA | 0.035 mm, 40 GHz, −35 dB | 1.53 M | 14.2–24.0 k | 813 s | 1.4× | 0.03 dB | 0.2° | 0.002 | 0 fF | notch 24.1 GHz |
| LibreVNA | 0.05 mm, 25 GHz, −50 dB | 0.71 M | 4.6–10.0 k | 134 s | 8.6× | 0.17 dB | 4.1° | 0.167 | −9 fF | notch 23.8 GHz |
| LibreVNA | 0.05 mm, 25 GHz, −35 dB | 0.71 M | 3.6 k | 87 s | 13× | 0.15 dB | 5.3° | 0.161 | −10 fF | notch 24.1 GHz |
| LibreVNA | **0.07 mm, 25 GHz, −50 dB** | 0.39 M | 5.4–10.6 k | 84 s | 14× | 0.20 dB | 2.1° | 0.158 | −5 fF | notch 23.6 GHz |
| LibreVNA | 0.07 mm, 25 GHz, −35 dB | 0.39 M | 2.6 k | 43 s | 27× | 0.30 dB | 2.2° | 0.157 | −6 fF | notch 23.9 GHz |

(Standard: 0.035 mm, 40 GHz, −50 dB, notch 24.0 GHz. The large \|ΔS11\| of LibreVNA sits at the stub notch, where
\|S11\| is close to 1 and a small shift of the notch is a large linear difference.)

- **Single vias: 0.07 mm, 25 GHz** keeps |S21| within 0.07–0.20 dB up to 20 GHz, the phase within 3.2° and the stub
  notch within 0.4 GHz, at 1/14–1/23 of the cost. Their runs are short already; stopping at −35 dB saves little
  and costs accuracy at 20–25 GHz.
- **Pairs: 0.05 mm, 25 GHz, −35 dB.** 0.07 mm cells do not resolve the 0.13 mm gap of the M.2 pair (Sdd11 off by
  0.15, C +24 fF); at 0.05 mm the pair keeps ringing (plane cavities), so the −35 dB stop is what makes it quick.
- The quick meshes shift the lumped C by 5–15 fF and keep the TDR minimum within 0.4–0.9 Ω (single) and 0.4 Ω (pair);
  that is below the ladder errors found on these vias (`validation/vias.md`), so the quick result is enough to judge the ladder.

### What a user would wait (estimate)
From the measured setup and step rates and the step counts above: WebAssembly with SIMD on this workspace, one
worker per excitation at 60 % of the single-worker rate each (an assumption: bandwidth-bound, see above), fixtures
afterwards in parallel (cached after the first via of a line type). Other machines were not measured; the gain
from parallel workers depends on their memory bandwidth.

| case | quick (1 worker / one per excitation) | standard, −35 dB stop (1 worker / one per excitation) | fixtures, first time (quick / standard) |
|---|---|---|---|
| M.2 pair (4 excitations) | 0.05 mm, −35 dB: 5.2 / 2.9 min | 21 / 9 min | +0.6 / +1.4 min |
| REFINOUT2 (2) | 0.07 mm: 1.0 / 1.0 min | 10 / 9.5 min | +0.3 / +1.5 min |
| LibreVNA (2) | 0.07 mm: 1.8 / 1.9 min | 16 / 16 min | +0.2 / +0.9 min |

With the −50 dB stop the standard mesh takes about twice as long (M.2: 41 / 18 min). Memory is no limit
(80–120 MB per worker). A quick check of a via is a 1–3 minute wait, a standard one 10–20 minutes; the setup
(10–24 s of it) is the first thing to optimise, the stop criterion the second (see Integration).


## Integration: click a via or a segment → 3D in the viewer

### What it needs
1. **Exporter as a library.** Move `viacase.rs` from `kisi-cli` into `kisi-core` (`kisi_core::viacase`, the same
   code for CLI and viewer) and export it through `kisi-wasm`; the viewer already holds the loaded project in
   WebAssembly. A **segment mode** uses the same machinery with a clicked point on a track instead of a via: keep
   box around the point, the track's two directions as feeds, the 2×thru of that line (neck-downs, plane gaps,
   pads, connector launches).
2. **Solver in workers.** `kisi_fdtd.wasm` (370 kB with SIMD, separate from the viewer's module, loaded on demand).
   `simulate` already reports progress natively; `fd_run` ignores it and runs to the end. It needs a JS import
   called every few hundred steps (step, energy level) for the progress bar, the ETA and an early stop. Cancel =
   terminate the worker. One worker per excitation (2 for a single via, 4 for a pair) and the fixtures as further
   jobs, through a small pool like the field solver's (`web/src/field-pool.js`, classic-script workers from blob
   URLs, which also work when kisi.html is opened from disk). Threads inside one run (the native `threads`
   option) would need `SharedArrayBuffer`, i.e. a cross-origin-isolated page (COOP/COEP headers), which neither
   the single-file kisi.html nor GitHub Pages provides; separate runs per worker need no shared memory. How much
   the parallel workers gain depends on the memory bandwidth: on the 2-vCPU workspace two native threads gave
   only +8 % (M.2), a desktop with more channels should do better (not measured).
3. **Stop criterion.** The runs stop when the field energy has fallen 50 dB below its peak. Weakly damped plane
   cavities and resonances above the band keep energy in the window long after the port signals have settled;
   stopping at −35 dB changed |S21| by at most 0.03 dB up to 20 GHz on the standard meshes and cut the steps
   1.8–2× there and 3–4× on the quick pair mesh (see Resource estimate).
   A criterion on the port waves (the DFT of b/a no longer changing) would be the robust version.
4. **Post-processing in Rust.** The matched 2×thru de-embedding (line impedance fit, NZC split), mixed mode, TDR and
   the metrics of `sparam.py` and `board_vias.py` (about 350 lines) belong in `kisi-core`, so CLI and viewer compute
   the same numbers; kisi's ladder for the same via is already in WebAssembly.
5. **Fixture cache.** A 2×thru depends only on the stackup, layer, width (and pitch, coplanar gap) of the feed, so
   most vias of a board share a few fixtures; cache them per board (memory, optionally IndexedDB) and run them only
   once.
6. **Panel.** From the via list of the net detail and from a click on a via in the board view: the case preview
   (layer by layer), cells and an estimated time from the mesh (milliseconds to
   compute), quick / standard; progress per excitation (energy against the stop level, ETA) with cancel; results:
   |S21| / |S11| (Sdd for pairs) against the ladder, TDR, dip / notch frequency, downloads (Touchstone, case JSON
   for `kisi-fdtd`). Estimated values marked as such, as everywhere in kisi; the quick mesh marked as such.
7. **Field view (optional).** The monitor planes kisi-fdtd already records (top view in the thickest dielectric,
   section through port 1, 10 / 20 / 30 GHz) as 2D heat maps with the phase animated, as on the reference page
   (`make_viewer.py`).
8. **Solver robustness and setup time.** Board geometry exercised paths the references did not (three solver bugs, since fixed
   in `crates/kisi-fdtd`). The time-step guard now checks the operator bound; a run whose energy grows after the pulse should
   stop with an error (or repeat with `"conformal_polys": false`) instead of returning NaN. The setup (10–24 s in
   WebAssembly, see Resource estimate) should come down before an integration.

### Hosts for a 3D view
- **Forstén's js_2d_fields** (vendored unmodified in `third_party/js_2d_fields`, upstream revision `3d5009c`): a
  2D cross-section solver (quasi-static and full-wave) with Plotly plots of the cross-section. No 3D view, and the
  vendored copy must not be changed.
- **The maintainer's fork `riehlies/js_2d_fields`** (based on upstream `318300a`): the full-wave solver as default
  and 2D field views of the cross-section — |E|, |H|, current density J, power flow S and loss density, E/H/S
  arrows, flux-weighted field lines, equipotentials, marching-squares contours, perceptual colour maps — scaled by
  one excitation, with a solve worker and a hosted teaching page. Also no 3D view: everything is the transverse
  plane of a line. It ships the full Plotly 3.3 bundle (4.8 MB), whose WebGL traces (surface, mesh3d) it does not
  use.
- Neither is a host for a 3D via view. What carries over from the fork is the way fields are shown (log scales,
  colour maps, arrows), which fits the 2D field-plane view of point 7.

### Recommendation and effort
Build the 3D check into kisi's own viewer with 2D views only (layer preview, plots, field planes) and keep a WebGL
3D view out of the first version (three.js would add about 150 kB compressed; only if users ask for it). Offer the
quick mesh by default (single vias 0.07 mm, pairs 0.05 mm with the −35 dB stop, 25 GHz: about 1–3 minutes in
WebAssembly on the measured cases) and the standard mesh with the −35 dB stop as an explicit "accurate" run
(10–20 minutes); estimates, see Resource estimate.

| step | days (estimate) |
|---|---|
| exporter as a library in kisi-core + WebAssembly binding | 1 |
| segment mode (a clicked track point instead of a via) | 1 |
| matched de-embedding, mixed mode, TDR, metrics in Rust (shared with the CLI) | 1 |
| solver: progress callback, stop criterion on the port waves, stop on growth, faster setup | 1 |
| worker pool for excitations and fixtures, fixture cache | 1 |
| panel: preview, estimate, progress / cancel, plots against the ladder, downloads | 1.5 |
| field-plane view | 1 |
| tests (`check_viewer.py`, viewer against `board_vias.py` on one case), docs | 0.5–1 |
| **total** | **8–8.5**; minimal version (vias only, quick and standard mesh, no field view, no segments): **3.5–4** |


## Design note: a 3D check in the viewer
The integration above stands, with what the later ladder work added: the comparison has to be made at the via centre
(`abcd_centre`), with the corrected TDR and the matched de-embedding in Rust. Minimal prototype, in this
order: (1) `viacase.rs` into `kisi_core::viacase` and a `kisi_cli_via_case` export in kisi-wasm (0.5 d);
(2) kisi-fdtd as a second WebAssembly module with a progress import, run in a classic worker from a
blob URL, one worker per excitation (1 d); (3) the matched de-embedding and the metrics of
`board_vias.py` in Rust (1 d); (4) a "3D check" button on a row of the via table: case preview, quick
mesh, progress/cancel, |S21|/|S11|/TDR against `abcd_centre` (1 d); (5) a `check_viewer.py` test with a
tiny case (0.5 d) – 4 days, as estimated. Not built then: a prototype that runs 1–3 minutes per via needs the solver's setup time
(10–24 s in WebAssembly) reduced first.
