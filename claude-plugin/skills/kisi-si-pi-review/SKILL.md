---
name: kisi-si-pi-review
description: Review signal and power integrity of a KiCad board with the kisi CLI (impedance per net class, channel checks against PCIe/USB/DDR/... profiles, vias, crosstalk, regulator chain, IR drop, PDN impedance). Use when the user asks about impedance, high-speed channels, eye/loss/skew, decoupling or voltage drop of a .kicad_pcb.
---

# SI/PI review with kisi

kisi reads the saved `.kicad_pcb` (and the `.kicad_pro` next to it for net classes), plus
`<board>.kisi.json` next to it if present (values with sources, see the skill kisi-prepare). It does
not talk to a running KiCad: make sure the board is **saved** before running kisi, and run it again
after changes. Every `kisi <command> <board>` prints JSON.

### Getting kisi
kisi is one file, `kisi.mjs` (Node.js ≥ 18; it includes the 2D field solver). Look for it in this order and
reuse it:
1. `$KISI_MJS` if set, else `~/.kisi/kisi.mjs`.
2. Otherwise download the latest release into `~/.kisi/`:
   - `curl -fsSL --create-dirs -o ~/.kisi/kisi.mjs https://github.com/riehlies/kisi/releases/latest/download/kisi.mjs`
     (PowerShell: `Invoke-WebRequest <same URL> -OutFile $HOME\.kisi\kisi.mjs`), or
   - `gh release download --repo riehlies/kisi --pattern kisi.mjs --dir ~/.kisi --clobber`.
3. If Node.js is missing, ask the user to install it (nodejs.org, LTS). A native `kisi` binary
   (`cargo install --path crates/kisi-cli`) accepts the same commands but has no field solver.

Run it as `node ~/.kisi/kisi.mjs <command> <board.kicad_pcb> [options]` (below written as `kisi …`).
`kisi help` lists the commands. The field solver runs for `overview`, `net`, `check`, `crosstalk` and `report`:
the first run on a board can take minutes (it prints progress on stderr and stops new solves after
`--field-time` seconds, default 120); results are cached, so later runs are fast. `field_solver`
in the output shows the coverage; `--formula` skips the solver.

## Order of work
1. `kisi gaps <board>` – if it reports high-priority gaps (no stackup, interfaces without a target),
   tell the user that results will be rough and offer to prepare the board first (kisi-prepare).
2. `kisi overview <board>` – stackup and, per net class with a target impedance, the share of routed
   length within ±5 % and the worst nets. Check that the stackup looks plausible (εr, tanδ;
   `assumed` lists values that are KiCad defaults). `config` says which kisi.json was used.
3. For classes or nets that look wrong: `kisi net <board> <NET>` – impedance segments along the path
   (layer, length, Z), losses, TDR extremes, vias (antipad, stub resonance, return path), crosstalk.
   Options `--rise-ps`, `--roughness smooth|hvlp|standard`.
4. High-speed interfaces: `kisi interfaces <board>` lists the buses and device-to-device lanes (pins,
   series parts, terminations, branches, direction guess, profile). Boards with assembly variants (or
   forks where all alternative parts are fitted) need a variant: `interfaces`, `check` and `report`
   stop with the list – ask the user which one, then use `--variant NAME` (or `"variant"` in kisi.json);
   `variant` in the output says which one was analysed, and `stubs` per lane names unused pads. Check that the end devices and
   profiles make sense; `kisi profiles` shows the available standards. Then `kisi check <board>`,
   with `--class CLASS=PROFILE` when the assignment is wrong (better: put it into kisi.json).
   Only yellow/red rows are printed by default (`--all` for green too, `--nets A,B` to restrict).
   Crosstalk: `kisi crosstalk <board>` lists the critical victims (NEXT or FEXT from 1 % of the aggressor
   swing, pairs as differential noise with the common-mode noise beside it) and their worst aggressor;
   `kisi net` then shows the aggressors of one net. `source: formula` values are estimates; broadside
   overlaps are counted but not computed.
   The first `check` on a board runs the field solver for all nets with a target (see Getting kisi).
5. Power: `kisi power <board>` – regulator chain balance (PI_REG/PI_LOAD fields or kisi.json, or
   `--tree tree.json`), IR drop per rail and in the ground return (`ground_return` rows: `max_offset_mv`
   adds to the drop of the parts' rails), `losses` (heat per part, copper loss per rail and layer) and
   `flow` (power from the input through the regulators to the loads); `--pdn RAIL` adds the impedance of one
   rail against its target. Losses are power, not temperature; part heat assumes all drawn power stays in the part.
6. Optionally `kisi report <board>` writes `analysis/kisi.json` next to the board, so file-based review
   tools (e.g. kicad-happy) and later sessions can reuse the results.
7. For the interactive view (2D field solver, charts, editable profiles), the user opens `kisi.html`
   and drops the board files (and kisi.json) into it.

## Reporting
- Impedances come from the 2D field solver where `field_solver.target_length_coverage_pct` says so,
  formulas elsewhere (differential pairs and coplanar lines can be off by 5–15 % there). If the
  coverage is low, say so and run the command again (cached results accumulate) or raise `--field-time`.
- Eyes are linear PRBS7 estimates with the profile's transmitter/receiver and no equalisation, jitter
  or noise. Limits marked "kisi" in the profile sources are guide values, not standards.
- Skew marked uncertain comes from branched paths; do not report it as a violation.
- Lanes ending at a connector (also connector to connector, e.g. test coupons) get their eye from the
  end set in `connector_ends` (termination when the connector receives, driver when it sends). Without
  a channel behind the connector (`rest_loss_db` or a Touchstone file) the eye covers the board share
  only (`eye_board_share`), is information and not judged – say that the rest of the channel is not
  included; loss, return loss and skew are judged on the board share.
- Formula impedances flagged "rough" (a launch or pad over a cut-out in a layer between trace and
  reference) can be 25 % off; prefer `kisi.mjs` (field solver) there.
- Lanes with an open branch (one IC feeding two connectors, both populated) show the stub's resonance;
  suggest marking the unused population option DNP in KiCad.
- Parasitics of series parts and terminations are estimates per package; the transmit direction is a
  guess (termination next to the receiver) and can be wrong.
- Values from kisi.json carry sources; when a result depends on an `estimate` or `assumption`, say so.
- Quote numbers with units and name the profile used for each interface.

## Working with other KiCad tools
- **Konnect** (live KiCad editing): use it to locate and change copper (widths, gaps, vias, stitching
  vias, back-drill), save, then re-run `kisi net` / `kisi check` to confirm.
- **kicad-happy** (file-based design review): run `kisi report` first; its JSON sits in the same
  `analysis/` folder as kicad-happy's outputs.
