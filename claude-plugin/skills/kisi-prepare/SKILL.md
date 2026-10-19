---
name: kisi-prepare
description: Prepare a KiCad board for kisi's SI/PI analysis - find missing or default data (laminate εr/tanδ, impedance targets, interface standards, regulator chain, load currents), research it from datasheets with sources and write it to a board.kisi.json file next to the board. Use when the user wants a board "ready for simulation", asks what kisi is missing, or wants datasheet values filled in.
---

# Prepare a KiCad board for kisi

kisi analyses the saved `.kicad_pcb` (plus the `.kicad_pro` next to it for net classes). Many values a
good analysis needs are not in those files, or only as KiCad defaults. This skill finds them, looks
them up, and writes them **with their source** to `<board>.kisi.json` next to the board. kisi (CLI and
viewer) lays that file over the board when it loads it. **Never edit the KiCad files themselves.**

### Getting kisi
kisi is one file, `kisi.mjs` (Node.js ≥ 18; it includes the 2D field solver). Look for it in this order and
reuse it:
1. `$KISI_MJS` if set, else `~/.kisi/kisi.mjs`.
2. Otherwise download the latest release into `~/.kisi/`:
   - `gh release download --repo riehlies/kisi --pattern kisi.mjs --dir ~/.kisi --clobber`
     (works while the repository is private, needs `gh auth login` with access to it), or
   - once the repository is public: `https://github.com/riehlies/kisi/releases/latest/download/kisi.mjs`.
3. If Node.js is missing, ask the user to install it (nodejs.org, LTS). A native `kisi` binary
   (`cargo install --path crates/kisi-cli`) accepts the same commands but has no field solver.

Run it as `node ~/.kisi/kisi.mjs <command> <board.kicad_pcb> [options]` (below written as `kisi …`).
`kisi help` lists the commands. The field solver runs for `overview`, `net`, `check` and `report`:
the first run on a board can take minutes (it prints progress on stderr and stops new solves after
`--field-time` seconds, default 120); results are cached, so later runs are fast. `field_solver`
in the output shows the coverage; `--formula` skips the solver.

Every `kisi <command>` prints JSON. Preparing needs no field solver: `gaps`, `config-check`,
`config-template`, `interfaces` and `profiles` are fast.

## Steps

1. **Gap list:** `kisi gaps path/to/board.kicad_pcb`. Each gap has `priority`, `issue`, `affects`,
   `where_to_find`, `write` (where in kisi.json, with an example) and `items` (layers, classes, parts).
   Work through `high` first, then `medium`. Gaps whose `write.to` says "not kisi.json" are fixes in
   KiCad: list them for the user, do not try to fix them.
2. **Context before research:**
   - `kisi overview <board>`: stackup with materials, net classes and targets.
   - `kisi interfaces <board>`: which devices sit at the ends of each bus (refdes, value = part number).
   - `kisi profiles`: the interface profiles a net class can get (ids such as `pcie3`, `usb2`, `lvds`).
3. **Research**, per gap:
   - Laminate (εr, tanδ): the material name in the stackup → manufacturer datasheet. Take the values at
     a frequency near the signals (1–10 GHz), for the resin content / glass style of the layer if the
     datasheet lists them. Prefer the fabricator's stackup report when the user has one.
   - Interface standard / profile: device datasheets of the parts at the bus ends (interface type,
     data rate). Choose the matching profile id; if none fits, say so instead of forcing one.
   - Target impedance: the interface standard or the device datasheet / layout guide.
   - Load currents (`PI_LOAD`): device datasheet, supply current per rail in the expected operating
     mode. FPGAs and SoCs: the vendor's power estimator; ask the user for its result if needed.
     Current that enters the board at other parts (e.g. an output stage fed through inductors): keep it on
     the IC and name the load point, `+5V=60mA@L32/L33` – the IC then also returns it into ground.
   - Regulators (`PI_REG`): topology, efficiency at the expected load, rated output current; input and
     output rails from the schematic. Syntax: `+12V > +3V3 buck 90% 3A` (see the gap's example).
   - Supply inputs: `PI_REG` = `+5V input` on the connector (counts in the power budget and as the
     IR-drop source; `PI_SOURCE` only marks source pads when there is no regulator chain).
     Regulator outputs that leave through an inductor or diode: add `out=<part>` (e.g. `out=L5`).
     Ferrites/chokes between two rails: `PI_REG` = `+3V3 > 3V3_RF pass r=0.05` (state the assumed
     resistance). Rails without a voltage in their name: `NAME@1.2V`.
   Use the user's own documents first (datasheet folder, BOM, fabricator files), then web search.
4. **Rules for values:**
   - Every value needs a `source`: `kind` (datasheet, standard, application_note, measurement,
     simulation, estimate, assumption) plus `document` and/or `url`, `page`, and a `note`.
   - Say in the note whether a value is typical or maximum, and at which condition (frequency, load).
   - If you could not find a value, do not invent one. Either leave it out or, if the user agrees, use
     `kind: "estimate"` / `"assumption"` with the reasoning in the note.
   - Prefer typical values for loss and eye, maximum currents for IR drop and regulator loading, unless
     the user says otherwise; state the choice.
5. **Write** `<board>.kisi.json` next to the board (same stem: `board.kicad_pcb` → `board.kisi.json`).
   `kisi config-template <board>` prints a skeleton with this board's dielectric indices and net
   classes; keep only the entries you fill in. If the file exists, merge – keep the user's entries.
   Format:
   ```json
   {
     "kisi_config": 1,
     "board": "board.kicad_pcb",
     "stackup": {
       "dielectrics": [{"index": 0, "epsilon_r": 3.65, "loss_tangent": 0.0095,
                        "source": {"kind": "datasheet", "document": "Isola FR408HR", "url": "…", "page": "2", "note": "10 GHz, typical"}}],
       "solder_mask": {"epsilon_r": 3.8, "source": {"kind": "datasheet", "document": "…"}}
     },
     "net_classes": {"PCIe": {"target_ohm": 85, "diff": true, "profile": "pcie3", "source": {"kind": "standard", "document": "PCIe CEM 3.0"}}},
     "fields": [{"ref": "U5", "name": "PI_LOAD", "value": "+1V0=2.5A, +1V8=300mA",
                 "source": {"kind": "datasheet", "url": "…", "page": "14", "note": "maximum, all lanes active"}}],
     "notes": ["what was assumed and why"]
   }
   ```
   Dielectric `index` 0 is the layer below F.Cu. `net_classes` can only name classes that exist in the
   KiCad project; nets without a suitable class need one in KiCad first (tell the user).
6. **Check:** `kisi config-check <board>` – fix every error (references, ranges, missing sources,
   field syntax kisi cannot read) and look at the warnings. Then `kisi gaps <board>` again: the filled
   gaps should be gone.
7. **Report to the user:** what was filled (with sources), what was assumed, what is still open, and
   the KiCad fixes from step 1. Suggest the review next (skill kisi-si-pi-review) and that they can drop
   `<board>.kisi.json` into the kisi viewer together with the board.

## Notes
- kisi.json values override the board file; the viewer's own edits (stackup, profiles) override
  kisi.json. A value written here is used by `kisi check`, `kisi power`, `kisi report` and the viewer.
- If the user wants values permanently in KiCad (e.g. `PI_LOAD` as a symbol field), give them the
  list to paste; kisi itself does not write into the schematic.
- kisi does not use IBIS models (yet); transmitters and receivers come from the interface profiles.
