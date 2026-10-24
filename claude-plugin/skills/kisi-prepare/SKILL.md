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

Every `kisi <command>` prints JSON. Preparing needs no field solver: `gaps`, `config-check`,
`config-template`, `interfaces` and `profiles` are fast.

## Steps

1. **Gap list:** `kisi gaps path/to/board.kicad_pcb`. Each gap has `priority`, `issue`, `affects`, `views`
   (impedance, channel, power), `where_to_find`, `write` (where in kisi.json, with an example) and `items`
   (layers, classes, parts). When the user wants one analysis ("ready for the impedance check"), use
   `kisi gaps <board> --for impedance` (or `channel`, `power`) and work only on those. `views` in the result
   says per category whether there is anything to check (`applicable`) and its `state` (`ready`, `open`,
   `nothing to check`) – do not research gaps for a category with nothing to check unless the user asks.
   Work through `high` first, then `medium`. Gaps whose `write.to` says "not kisi.json" are fixes in
   KiCad: list them for the user, do not try to fix them. Part items carry `mpn` (and `manufacturer`)
   when the footprint has a part number that its Value field does not show (Value "英联", MPN UM3204H):
   look up the datasheet of the MPN.
2. **Context before research:**
   - `kisi overview <board>`: stackup with materials, net classes and targets.
   - `kisi interfaces <board>`: which devices sit at the ends of each bus (refdes, value, and `mpns` where
     the value is not the part number).
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
     output rails from the schematic. Syntax: `+12V > +3V3 buck 90% 3A` (see the gap's example). Without
     an efficiency kisi assumes 85 % and marks it as an estimate (gap `regulator-efficiency`); when the
     datasheet has only a curve, read the value there and say so in the note.
   - Rails without a voltage in their name (gap `rails-without-voltage`: found by name or pin, e.g. VBUS,
     VSYS, VBAT, `Net-(U3-VCC)`): kisi needs their voltage. Write it where the rail appears in `PI_REG`, as
     the output of the part that feeds it (`VBUS > VSYS@5V pass`) or as an input (`VBAT@3.7V input`); a rail
     behind a ferrite or 0 Ω link takes the voltage through a pass element (`items[].write` suggests it).
     A net found by name that is not a supply: tell the user, do not invent a voltage.
   - Supply inputs: `PI_REG` = `+5V input` on the connector (counts in the power budget and as the
     IR-drop source; `PI_SOURCE` only marks source pads when there is no regulator chain).
     Regulator outputs that leave through an inductor or diode: add `out=<part>` (e.g. `out=L5`).
     Ferrites/chokes between two rails: `PI_REG` = `+3V3 > 3V3_RF pass r=0.05` (state the assumed
     resistance). Rails without a voltage in their name: `NAME@1.2V`. A PMIC with several outputs gets
     one entry per channel, separated by `;` and each with its own `id=` (`VSYS > +3V3 buck 200mA id=U2B;
     VSYS > +1V8 ldo 50mA id=U2C`). Net names with spaces (hierarchical names such as
     `/06 Peripherals/IMU_VDD`) go in double quotes, in every power field: `+3V3 > "/06 Peripherals/IMU_VDD"
     pass r=0.5` on the bead, `"/06 Peripherals/IMU_VDD"=0.9mA` on the load, `"/Power/VDD CORE"@0.9V` with a
     voltage. In kisi.json the quotes are escaped: `"value": "+3V3 > \"/06 Peripherals/IMU_VDD\" pass r=0.5"`.
     The last part of a hierarchical name (`IMU_VDD`) also works when it is unique on the board.
   - Ferrite beads and inductors in supply paths (`PI_SERIES`, for the PDN per IC): impedance at
     100 MHz and DC resistance of a bead (`z100=600 rdc=0.15`), inductance and DC resistance of an
     inductor (`l=2.2u dcr=50m`), from the part's datasheet (`kisi gaps`: bead-ratings).
   - Thermal data of hot parts (`PI_THERMAL`, gap `thermal-data`, for `kisi thermal`): the datasheet's
     Thermal Information table – RθJB and RθJC(top) (`theta_jb=17.4 theta_jc=53.6`), ψJT for the case top
     (`psi_jt=1.1`), or ψJB when RθJB is missing (`psi_jb=…`); θJA only as a last resort (`theta_ja=…`, it describes the JEDEC test board, not
     this one) – and the maximum junction temperature (`tj_max=125`; FPGAs: the speed/temperature grade).
     Parts whose power leaves the board (connectors, LEDs, RF output stages): the heat in the part, `p=…W`.
   - Assembly variants (gap `assembly-variant`): the project's BOM and position files per variant, the
     sheets that hold each option, KiCad 10 variants in the board file. Define `variants` (by sheet,
     references or BOM, `source` kind `design`) and ask the user which one to analyse – write it as
     `"variant"`; never pick one silently. `"default"` = the board file as it is.
   - Connector ends (`connector_ends`, lanes that end at a connector): what the connector meets – the
     termination when it receives, the driver when it sends (the profile's, matched, high impedance or
     custom R/C/L) – and the channel behind it: a cable or mating board as loss at the profile's Nyquist
     frequency and delay, or the vendor's Touchstone file (`.s2p`, `.s4p`). Without a channel behind the
     connector the eye covers the board share only and is not judged.
   Use the user's own documents first (datasheet folder, BOM, fabricator files), then web search. Ask for
   the datasheets early: many manufacturer PDFs cannot be read through a web fetch, and efficiencies are
   often only given as curves.
   - Name-based guesses (`nets-without-target`, and rails in `rails-without-voltage` with `"dismissable": true`):
     when the schematic or datasheet shows the name misleads (a QSPI bus that looks like DDR, a net called VBAT
     that is a sense line), dismiss the items instead of inventing values – `"gaps": {"dismissed": [{"gap":
     "nets-without-target", "items": ["/FPGA/QSPI_DQ0"], "reason": "QSPI flash at 50 MHz, no impedance control
     needed"}]}` – and tell the user. Keep dismissals that are already in the file.
4. **Rules for values:**
   - Every value needs a `source`: `kind` (datasheet, standard, application_note, measurement,
     simulation, design, estimate, assumption) plus `document` and/or `url`, `page`, and a `note`.
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
     "variant": "PCIe",
     "variants": {"PCIe": {"not_fitted_sheets": ["/USB Components/"], "fitted": ["C101"],
                           "source": {"kind": "design", "document": "board_PCIe_BOM.csv"}},
                  "USB": {"bom": "board_USB_BOM.csv", "bom_exceptions": ["P1"], "source": {"kind": "design", "document": "board_USB_BOM.csv"}}},
     "notes": ["what was assumed and why"]
   }
   ```
   Variants: rules apply in the order board file → `bom` (parts not in it are not fitted, except parts
   marked "exclude from BOM" in KiCad and `bom_exceptions`) → sheets (a sheet covers its sub-sheets, the
   most specific wins) → `fitted` / `not_fitted` (wildcards allowed). A BOM path is relative to the
   kisi.json.
   Dielectric `index` 0 is the layer below F.Cu. `net_classes` can only name classes that exist in the
   KiCad project; nets without a suitable class need one in KiCad first (tell the user).
6. **Check:** `kisi config-check <board>` – fix every error (references, ranges, missing sources,
   field syntax kisi cannot read) and look at the warnings. Then `kisi gaps <board>` again: the filled
   gaps should be gone.
7. **Report to the user:** what was filled (with sources), what was assumed, what is still open, and
   the KiCad fixes from step 1. Suggest the review next (skill kisi-si-pi-review) and that they can drop
   `<board>.kisi.json` into the kisi viewer together with the board.

## Notes
- A worked example – the power tree of the Seeed XIAO nRF54LM20A with sources, notes and the open gaps – is in
  the kisi repository: `docs/preparing-a-board.md` (the walk-through, also the gap list with every gap id) and
  `docs/examples/xiao-nrf54lm20a/` (the kisi.json). Field syntax and every kisi.json key: `docs/reference.md`.
- kisi.json values override the board file; the viewer's own edits (stackup, profiles) override
  kisi.json. A value written here is used by `kisi check`, `kisi power`, `kisi report` and the viewer.
- If the user wants values permanently in KiCad (e.g. `PI_LOAD` as a symbol field), give them the
  list to paste; kisi itself does not write into the schematic.
- kisi does not use IBIS models (yet); transmitters and receivers come from the interface profiles.
