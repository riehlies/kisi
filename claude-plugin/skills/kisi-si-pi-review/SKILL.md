---
name: kisi-si-pi-review
description: Review signal and power integrity of a KiCad board with the kisi CLI (impedance per net class, channel checks against PCIe/USB/DDR/... profiles, vias, crosstalk, regulator chain, IR drop, PDN impedance). Use when the user asks about impedance, high-speed channels, eye/loss/skew, decoupling or voltage drop of a .kicad_pcb.
---

# SI/PI review with kisi

kisi reads the saved `.kicad_pcb` (and the `.kicad_pro` next to it for net classes), plus
`<board>.kisi.json` next to it if present (values with sources, see the skill kisi-prepare). It does
not talk to a running KiCad: make sure the board is **saved** before running kisi, and run it again
after changes. Needs `kisi` on the PATH (`cargo install --path crates/kisi-cli`). Every
`kisi <command> <board>` prints JSON; `kisi help` lists the commands and options.

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
   series parts, terminations, branches, direction guess, profile). Check that the end devices and
   profiles make sense; `kisi profiles` shows the available standards. Then `kisi check <board>`,
   with `--class CLASS=PROFILE` when the assignment is wrong (better: put it into kisi.json).
   Only yellow/red rows are printed by default (`--all` for green too, `--nets A,B` to restrict).
5. Power: `kisi power <board>` – regulator chain balance (PI_REG/PI_LOAD fields or kisi.json, or
   `--tree tree.json`), IR drop per rail; `--pdn RAIL` adds the impedance of one rail against its target.
6. Optionally `kisi report <board>` writes `analysis/kisi.json` next to the board, so file-based review
   tools (e.g. kicad-happy) and later sessions can reuse the results.
7. For the interactive view (2D field solver, charts, editable profiles), the user opens `kisi.html`
   and drops the board files (and kisi.json) into it.

## Reporting
- Impedances from the CLI are closed-form formulas (field solver only in the viewer): differential
  pairs and coplanar lines can be off by 5–15 %. Say so, and suggest the viewer's field solver for
  nets close to a limit.
- Eyes are linear PRBS7 estimates with the profile's transmitter/receiver and no equalisation, jitter
  or noise. Limits marked "kisi" in the profile sources are guide values, not standards.
- Skew marked uncertain comes from branched paths; do not report it as a violation.
- Eyes exist only for lanes from IC to IC. Lanes ending at a connector are judged on the board share
  (loss, return loss, skew); say that the rest of the channel is not included.
- Lanes with an open branch (one IC feeding two connectors, both populated) show the stub's resonance;
  suggest marking the unused population option DNP in KiCad.
- Parasitics of series parts and terminations are estimates per package; the transmit direction is a
  guess (termination next to the receiver) and can be wrong.
- Values from kisi.json carry sources; when a result depends on an `estimate` or `assumption`, say so.
- IBIS ends (`end_models`, `ibis_files`) are experimental and not working yet: report such results as
  unvalidated.
- Quote numbers with units and name the profile used for each interface.

## Working with other KiCad tools
- **Konnect** (live KiCad editing): use it to locate and change copper (widths, gaps, vias, stitching
  vias, back-drill), save, then re-run `kisi net` / `kisi check` to confirm.
- **kicad-happy** (file-based design review): run `kisi report` first; its JSON sits in the same
  `analysis/` folder as kicad-happy's outputs.
