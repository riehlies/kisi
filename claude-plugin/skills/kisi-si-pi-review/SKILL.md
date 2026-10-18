---
name: kisi-si-pi-review
description: Review signal and power integrity of a KiCad board with the kisi MCP tools (impedance per net class, channel checks against PCIe/USB/DDR/... profiles, vias, crosstalk, regulator chain, IR drop, PDN impedance). Use when the user asks about impedance, high-speed channels, eye/loss/skew, decoupling or voltage drop of a .kicad_pcb.
---

# SI/PI review with kisi

kisi reads the saved `.kicad_pcb` (and the `.kicad_pro` next to it for net classes). It does not
talk to a running KiCad. If the board is being edited live (for example through Konnect or the
KiCad IPC API), make sure it is **saved** before calling kisi, and call kisi again after changes.

## Order of work
1. `kisi_overview` – stackup and, per net class with a target impedance, the share of routed
   length within ±5 % and the worst nets. Check that the stackup looks plausible (εr, tanδ);
   unrealistic values can be corrected per call with the `stackup` argument.
2. For classes or nets that look wrong: `kisi_net` – impedance segments along the path (layer,
   length, Z), losses, TDR extremes, vias (antipad, stub resonance, return path), crosstalk.
3. High-speed interfaces: `kisi_interfaces` lists the buses and device-to-device lanes (pins,
   series parts, terminations, branches, direction guess, profile). Check that the end devices and
   profiles make sense; `kisi_profiles` shows the available standards. Then `kisi_channel_check`,
   with a class → profile mapping when the automatic assignment is wrong (e.g. `{"PCIe": "pcie3"}`).
   Only yellow/red rows are returned by default. Devices whose symbols carry an IBIS model (KiCad
   simulation fields Sim.Device=IBIS, Sim.Library, Sim.Name) get linearised IBIS ends in the eye
   (`end_models`; `ibis_files` says whether each .ibs file was found). This is experimental and not
   working yet: report such results as unvalidated;
   if a file is missing, say so and suggest placing it next to the board.
4. Power: `kisi_power` – regulator chain balance (PI_REG/PI_LOAD fields or a tree JSON), IR drop
   per rail; `pdn_rail` adds the impedance of one rail against its target.
5. Optionally `kisi_report` writes `analysis/kisi.json` next to the board, so file-based review
   tools (e.g. kicad-happy) and later sessions can reuse the results.
6. `kisi_open_viewer` opens the interactive viewer (2D field solver, charts, editable profiles)
   for the user, optionally at a net.

## Reporting
- Impedances from the MCP tools are closed-form formulas (field solver only in the viewer):
  differential pairs and coplanar lines can be off by 5–15 %. Say so, and suggest the viewer's
  field solver for nets close to a limit.
- Eyes are linear PRBS7 estimates with the profile's transmitter/receiver and no equalisation,
  jitter or noise. Limits marked "kisi" in the profile sources are guide values, not standards.
- Skew marked uncertain comes from branched paths; do not report it as a violation.
- Eyes exist only for lanes from IC to IC. Lanes ending at a connector are judged on the board
  share (loss, return loss, skew); say that the rest of the channel is not included.
- Lanes with an open branch (one IC feeding two connectors, both populated) show the stub's
  resonance; suggest marking the unused population option DNP in KiCad.
- Parasitics of series parts and terminations are estimates per package; the transmit direction
  is a guess (termination next to the receiver) and can be wrong.
- Quote numbers with units and name the profile used for each interface.

## Working with other KiCad tools
- **Konnect** (live KiCad editing): use it to locate and change copper (widths, gaps, vias,
  stitching vias, back-drill), save, then re-run `kisi_net` / `kisi_channel_check` to confirm.
- **kicad-happy** (file-based design review): run `kisi_report` first; its JSON sits in the same
  `analysis/` folder as kicad-happy's outputs.
