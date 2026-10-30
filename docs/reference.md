# Field and `kisi.json` reference

Everything kisi reads besides the copper: names and fields in the KiCad project, the board configuration
`<board>.kisi.json`, and the other files the CLI and the viewer accept. How to fill these in step by step is in
[Preparing a board](preparing-a-board.md); the commands are in the [CLI reference](cli.md).

## What kisi reads from the KiCad project
- **Board file** `.kicad_pcb` (KiCad 8, 9 and 10): copper, zones, vias, footprints with their fields, the
  stackup, DNP and "exclude from BOM" attributes, the sheet of every part and – KiCad 10 – the assembly variants
  (variant list and per-part overrides: DNP, BOM, field values). kisi never writes into KiCad files.
  - **Copper layers** are those of the board's layer list (`(layers …)`), in the order of the stackup. A stackup
    that lists the layer stack more than once (seen in KiCad 8 and 10 files) is read up to the first repeated
    layer; a stackup copper layer the board does not have is left out (the dielectrics around it merged); when
    the stackup lacks one of the board's copper layers, kisi uses its default stackup. Each case is a warning in
    `kisi overview` (`warnings`) and in the viewer.
  - **Board outline** (for `thermal`): the Edge.Cuts drawings on the board and in footprints – lines, arcs
    (also KiCad 5 arcs), rectangles, circles, polygons, curves – joined where their ends meet (within
    0.01 mm, then 0.1 mm with a warning); a drawing repeated exactly is ignored. Nested loops are cut-outs, and a
    board inside a cut-out (a panel's frame) counts again. Several separate outlines – a panel, or boards drawn
    side by side – are computed together as one board area (warning); heat crosses between them only where
    they touch. Where the loops enclose no area or leave copper outside, the bounding box of the copper is the
    board (warning).
- **Project file** `.kicad_pro` next to the board (same stem): the net classes and which nets belong to them.
  Without it, no net class has a target impedance unless `<board>.kisi.json` gives one.

### Target impedance from net class names
Without a target in `<board>.kisi.json`, kisi reads one from the class name. A number between 20 and 200 counts
when
- a unit follows it: `Z`, `R`, `Ohm` or `Ω` – `50Z`, `FE_50Z_INPUT`, `USB_90R`, `Diff100ohm`;
- a prefix stands right before it (one `_`, `-`, `=` or space between allowed): single-ended `SE`, `S`, `ZSE`,
  `ZS`, `ZSINGLE`, `SINGLE` – `SE50`, `SE_50`, `S50`, `ZSINGLE=50`; differential `DIFF`, `DIF`, `DE`, `ZD`,
  `ZDIFF` – `Diff_100`, `diff90`, `ZDIFF=100`; or `Z` – `Z85_PCIe`. Prefixes of one or two letters must
  stand alone (`S50`, not `BUS50`);
- or, with neither, a kind word follows it: `100_DIFF`, `50_SE`.

With a unit or a `Z` prefix the target is differential when the name contains `Diff` or `DP` (`90Z_Diff`,
`FE_100Z_Diff`), or when it is 80 Ω or more and the name contains `USB`, `LVDS` or `PCI`. A name with a
single-ended and a differential value, such as `SE55DE100`, gives the differential target (100 Ω, what the
pairs are judged on) and names the single-ended value in the source; two different values of one kind
(`SE50_SE60`) give no target. Such targets show "class name" as their source (in `kisi overview` and the
viewer's class list): what the designer named the class, not an estimate. Without a number, interface keywords
give an assumed target, marked "interface assumption":

| Keyword in the class name | Target |
|---|---|
| `USB` | 90 Ω differential |
| `PCIE`, `PCI_E` | 85 Ω differential |
| `LVDS`, `HDMI`, `TMDS`, `SATA`, `MDI`, `ETH` | 100 Ω differential |
| `RF` | 50 Ω single-ended |

The prefixes, units, keywords and their values are in `data/netclass_names.json` (with sources); boards that
use other names can be added there. A class only gets a target for nets that belong to it: kisi reads the
assignments of the project file (`netclass_assignments`, `netclass_patterns`); a class whose nets are assigned
only in the schematic stays empty for kisi – `net_classes.<class>.nets` in `<board>.kisi.json` fills it.

`net_classes` in `<board>.kisi.json` gives or overrides the target of an existing class. With `nets` it also gives
nets to a class – names or patterns with `*` and `?`, matched against the full net name and its last part, like
KiCad's net class patterns – and defines the class if the project has none of that name: nets that sit in
"Default" get a target without a change in KiCad, e.g.
`"QSPI": {"nets": ["*QSPI_*"], "target_ohm": 50, "diff": false, "profile": "se50", "source": {…}}`. `config-check`
reports a pattern that matches no net and a net given to two classes.

### Interface profile
The channel check judges every lane against an interface profile (transmitter, receiver, limits) from
`data/interfaces.json`. kisi assigns one automatically – by net name first, then by class name, the first
matching rule of the file; a profile with `context` rules (the DDR3 ones: `DDR3` in the sheet path, net or class
name; DDR4 address/command: `DDR`, `DRAM` or `MEM`) only where its context matches, so that `/FPGA-DDR3L/A0` is
DDR3 address/command (`ddr3_ca`) and an address bus `/CPU/A0` no memory line at all – unless `net_classes.<class>.profile` in `<board>.kisi.json`, the viewer's choice or
`--class CLASS=PROFILE` sets one. `kisi profiles` lists the profiles; every value in the file carries its source
(standard, vendor guideline, or a kisi guide value).

### Part numbers
Where the Value field does not show the part number (`100n`, a manufacturer name), kisi takes the first non-empty
MPN field – `MPN`, `Manufacturer Part Number`, `MFR PN`, `Mfg Part Number`, … (case, spaces and punctuation
ignored) – and shows it in `kisi gaps`, `kisi interfaces` and the power tree, so that the datasheet can be found.

### Supply rails
A net is a supply rail when its name carries a voltage (`+3V3`, `1V8`, `-5V`; not signal names such as
`PG_1V0`), when a power field names it, or – for `kisi gaps` – when its name or a pin name on it looks like a
supply (`VBUS`, `VSYS`, `VBAT`, `VCC…`, `VDD…`, `VIN…`, `VIO`, …). A rail without a voltage in its name needs one:
`NAME@<V>V` in a `PI_REG` entry (below).

## Footprint fields
Fields on the symbol, copied to the footprint by "Update PCB from schematic" – or written into
`<board>.kisi.json` (`fields`, below) when the KiCad files should stay untouched. Field names are not
case-sensitive.

| Field | Example | Meaning |
|---|---|---|
| `PI_REG` | `+12V input` | supply input (a connector): source of the power tree and of the IR drop |
| | `+12V > +5V buck 90% 3A out=L3` | switching regulator: efficiency, rated output current, the part that carries the output |
| | `+5V > +3V3 ldo iq=2mA 1A` | linear regulator with its quiescent (ground) current |
| | `+3V3 > +3V3A pass r=0.05` | ferrite, 0 Ω link or shunt with its series resistance (Ω) |
| | `+5V > VCORE@0.9V buck 85% id=U4A` | rail without a voltage in its name; one IC with several channels: entries separated by `;`, each with its own `id=` |
| | `+3V3 > "/Sensors/IMU VDD" pass r=0.5` | net name with spaces (hierarchical name) in double quotes, also `"/Power/VDD CORE"@0.9V` |
| `PI_LOAD` | `+1V0=2.5A, +1V8=300mA` or `+1V0=2W` | load current (or power) per rail |
| | `+5V=60mA@L32/L33` | current that enters the board at other parts' pads (e.g. the inductors of an output stage); the IC still returns it into ground |
| | `"/Sensors/IMU VDD"=0.9mA` | net name with spaces in double quotes |
| `PI_SOURCE` | `+3V3` or `+3V3=3.3V` | source pads for the IR drop when there is no regulator chain (several separated by `,` or `;`) |
| `PI_CAP` | `esr=5m esl=0.45n rated=6.3V X5R c@3.3V=2.1u` | decoupling capacitor data for the PDN from its datasheet; replaces the package estimates and the DC-bias curve |
| `PI_SERIES` | `z100=600 rdc=0.15` · `l=2.2u dcr=50m` · `r=10m` | ferrite bead, inductor or resistor in a supply path, for the PDN per IC (replaces the typical values) |
| `PI_TARGET` | `VCCINT=10m fmax=50M` or `15mOhm` | target impedance of an IC for a rail and the band it applies to (default: from the IC's own current step) |
| `PI_THERMAL` | `theta_jb=17.4 theta_jc=53.6 psi_jt=1.1 tj_max=125` | thermal data from the datasheet's Thermal Information table (K/W, °C) for the temperature estimate |

### `PI_REG` in detail
One entry per regulator channel: `INPUT > OUTPUT kind [values]`, or `OUTPUT input` for a supply input. `>` may
also be written `->` or `→`.
- **kind:** `input`, `buck`, `boost`, `buck-boost`, `sepic`, `inverter`, `ldo`, `pass`. Without a kind, an entry
  with an input rail is a `buck`, one without is an `input`.
- **values:** efficiency `90%` (or `eff=0.9`) – without one, a switching regulator gets kisi's default of 85 %,
  marked as an estimate everywhere; rated output current `3A`, `500mA` (or `imax=`); `iq=1mA` quiescent current
  drawn from the input; `r=10m` series resistance of a pass element (Ω); `out=L3` / `in=…` the part whose pads
  carry the output / input current (an inductor or diode), if not the regulator itself; `id=U4A` a unique id when
  one part has several channels; `part=…` a part name for the lists.
- **voltages:** from the net name (`+3V3`, `1V8`, `-5V`), else `NAME@<V>V` wherever the rail appears.

### `PI_CAP`, `PI_SERIES`, `PI_TARGET`, `PI_THERMAL` keys
| Field | Keys |
|---|---|
| `PI_CAP` | `c=` capacitance, `esr=`, `esl=`, `rated=` (also `v=`, `vr=`) rated voltage, `c@<V>=` capacitance at a DC bias (several allowed), a dielectric word (`X5R`, `X7R`, `C0G`, …) |
| `PI_SERIES` | `r=` (also `dcr=`, `rdc=`) DC resistance, `l=` inductance, `z100=` bead impedance at 100 MHz, or the parallel model `rp=`, `lp=`, `cp=` |
| `PI_TARGET` | `RAIL=<Ω>` per rail (several separated by `;` or `,`), or a bare value for any rail; `fmax=` upper end of the band |
| `PI_THERMAL` | `theta_jb=`, `theta_jc=` (JEDEC two-resistor model), `psi_jb=` when there is no θJB, `psi_jt=` for the case top, `theta_ja=` only as a fallback (it describes the JEDEC test board), `tj_max=`, `p=0.2W` the heat that stays in the part (replaces the power tree's value, e.g. for a connector that passes power on) |

Values take SI prefixes (`5m`, `0.45n`, `2.2u`, `50M`); resistances also `4R7`, `2k2`, `10mR`.

### Capacitor ratings from the parts' fields
The DC-bias derating of a class II capacitor (X5R, X7R, …) needs its rated voltage. Many libraries carry it, and
the dielectric, in fields of their own; kisi reads these, so such capacitors need no `PI_CAP`. Each of the two
values comes from the first of:
1. `PI_CAP` (`rated=`, a dielectric word) – a `PI_CAP` in `<board>.kisi.json` replaces one in the board file;
2. a rating field of the part, in this order – rated voltage: `Voltage Rating`, `Rated Voltage`, `Voltage Rated`,
   `Voltage - Rated`, `Voltage Rating DC`, `Rated Voltage DC`, `Voltage`, `Characteristic`; dielectric:
   `Dielectric`, `Dielectric Material`, `Dielectric Type`, `Temperature Coefficient`, `Temp Coefficient`, `Tempco`,
   `TC`, `Characteristic`. Names compare without case, spaces and punctuation (`VOLTAGE RATING`, `Voltage_Rating`);
   the list, with the libraries it was taken from, is `decoupling.rating_fields` in `data/components.json`;
3. the Value text (`10u 6.3V X5R`, `100n/16V/X7R`, `4u7 6V3`);
4. the `Description` field (`SMD CAP Ceramic 100nF-10V-10%-X5R;0201`, `0.1 µF ±10% 16V Ceramic Capacitor X7R`);
5. none: for a class II (or unknown) dielectric the smallest standard rating ≥ 1.8 × the rail voltage is assumed –
   an estimate, marked as such; a capacitor of 100 nF or more is then listed in the `cap-ratings` gap.

A rated-voltage field may read `16V`, `16 V`, `6.3V`, `6V3`, `6,3 V`, `16VDC`, `1kV` or a bare `16` (1 V … 20 kV);
a dielectric field `X5R`, `C0G/NP0`, `NP0`, `X7R-25V`, `Polymer`, `Tantalum`. A field kisi cannot read (`n/a`, a
temperature range, two different values such as `X5R/X7R`) is passed over. The PDN results name where each value
came from (`rated_from`, `dielectric_from`: `PI_CAP`, `field <name>`, `value`, `assumed`); the viewer shows it on a
capacitor's row in the PDN table. A field of this list given in `<board>.kisi.json` `fields` is read the same way.

### Net names in fields
Net names are written as in KiCad, or as the last part of a hierarchical name (`IMU_VDD` for
`/Sensors/IMU_VDD`) when that is unique on the board. A name with spaces or one of `; , = @ >` goes in double
quotes (`PI_REG`, `PI_LOAD`, `PI_SOURCE`, `PI_TARGET`); inside `<board>.kisi.json` the quotes are escaped as `\"`.

## Board configuration `<board>.kisi.json`
Values the board file lacks or holds only as KiCad defaults, each with its source. The file sits next to the
board with the same stem (`board.kicad_pcb` → `board.kisi.json`); CLI and viewer lay it over the board when they
load it. `kisi config-template <board>` writes a skeleton with this board's dielectrics, net classes and assembly
variants; `kisi config-check <board>` checks it (the viewer runs the same check when the file is dropped).

```json
{
  "kisi_config": 1,
  "board": "board.kicad_pcb",
  "stackup": {
    "dielectrics": [{"index": 0, "epsilon_r": 3.65, "loss_tangent": 0.0095,
                     "source": {"kind": "datasheet", "document": "Isola FR408HR", "page": "2", "note": "10 GHz, typical"}}],
    "solder_mask": {"epsilon_r": 3.8, "source": {"kind": "assumption", "note": "typical LPI mask"}}
  },
  "net_classes": {"PCIe": {"target_ohm": 85, "diff": true, "profile": "pcie3", "source": {"kind": "standard", "document": "PCIe CEM 3.0"}}},
  "connector_ends": {"P1": {"model": "profile", "rest_loss_db": 6, "source": {"kind": "estimate", "note": "1 m cable"}}},
  "fields": [{"ref": "U5", "name": "PI_LOAD", "value": "+1V0=2.5A", "source": {"kind": "datasheet", "url": "…", "note": "maximum"}}],
  "variant": "TS-PCIe",
  "variants": {
    "TS-PCIe": {"description": "PCIe card", "not_fitted_sheets": ["/TS-USB4 Components/"], "fitted": ["C101", "C102", "C128"],
                "source": {"kind": "design", "document": "TS-PCIe_MP_BOM.csv"}},
    "TS-USB4": {"bom": "TS-USB4_MP_BOM.csv", "bom_exceptions": ["TP*"], "source": {"kind": "design", "document": "TS-USB4_MP_BOM.csv"}}
  },
  "notes": ["what was assumed and why"]
}
```

| Key | Content |
|---|---|
| `kisi_config` | format version, must be `1` |
| `board` | the board file it was written for; a different board is an error in `config-check` |
| `stackup` | per dielectric (`index` 0 = the layer below F.Cu): `material`, `epsilon_r` (1 … 20), `loss_tangent` (0 … 0.2), `thickness` (mm); `solder_mask.epsilon_r` |
| `net_classes` | per net class: `target_ohm` (10 … 200; differential when `diff` is true), `diff`, `profile` (an id from `kisi profiles`), `nets` (names or patterns given to this class; defines a class the project does not have) |
| `connector_ends` | per connector where lanes leave the board: what sits there and the channel behind it (below) |
| `fields` | footprint fields laid over the board: `ref`, `name` (`PI_*`, or a [capacitor rating field](#capacitor-ratings-from-the-parts-fields)), `value` |
| `variant` | the assembly variant to analyse: a KiCad 10 variant of the board, one of `variants`, or `default` (the board file as it is) |
| `variants` | assembly variants for boards without KiCad 10 variants, or on top of one of the same name (below) |
| `gaps` | `dismissed`: items of name-based gaps that do not apply, with a reason (below) |
| `notes` | free text: what was assumed and why |

Every value needs a `source`: `kind` plus at least one of `document`, `url`, `note` (`page` optional). Kinds:
`datasheet`, `standard`, `application_note`, `measurement`, `simulation`, `design` (the project's own BOM,
position or schematic files), `estimate`, `assumption`. Values of kind `estimate` or `assumption` are reported as
such. Say in the `note` whether a value is typical or maximum and at which condition.

**Precedence:** board file < `<board>.kisi.json` < edits in the viewer / `--class`, `--variant` and `--stackup`
on the command line.

### `connector_ends`
A lane from a device to a connector is judged on the board share – loss, return loss, skew (a lane from connector
to connector as well, without an eye). Its eye needs to know what sits at the connector:

```json
"connector_ends": {
  "P1": {"model": "profile" | "matched" | "high_impedance" | "custom",
         "r_ohm": 50, "c_pf": 1.5, "l_nh": 0.5,
         "rest_loss_db": 6, "rest_delay_ps": 500,
         "touchstone": {"file": "cable.s4p", "port_order": "1-3,2-4", "flip": false},
         "source": {"kind": "estimate", "note": "…"}}
}
```
- `model`: the termination when the connector receives, the driver when it sends – the profile's (`profile`,
  default), `matched`, `high_impedance` (with `c_pf`), or `custom` (`r_ohm` per line, 0 = high impedance, with
  `c_pf`, `l_nh`).
- The channel behind the connector: a lossy line with `rest_loss_db` (loss at the profile's Nyquist frequency,
  positive dB) and `rest_delay_ps` (default 500), or a Touchstone file. With either, the eye covers the whole
  channel and is judged; without, the eye covers the board share only and is information.
- `touchstone`: `file` (path relative to the kisi.json; in the viewer, drop the file with the board); `.s2p` for
  single-ended lanes or on each line of a pair, `.s4p` for a pair with its mode conversion (`port_order`
  `"1-3,2-4"` – P 1→3, N 2→4, the default – or `"1-2,3-4"`); port 1 at the connector unless `flip`.
- One file for all lanes of the connector (`.sNp`): `"ports": {"A16": [1, 9], "A17": [2, 10], …}` gives per
  connector pin the file's port at the connector and at the far end. The other lanes in the file are aggressors
  in each lane's eye – worst case, entering at the connector without their board traces (conservative); their
  directions are the guessed lane directions.

In the viewer the same settings are in the interface's card of the channel overview.

### `variants`
Which parts are fitted, per variant. Rules apply in this order, later ones win:
1. the board file (DNP, and the KiCad 10 variant of the same name),
2. `bom`: a CSV next to the kisi.json with a "Reference" or "Designator" column – parts not in it are not fitted,
   except parts marked "exclude from BOM" in KiCad and `bom_exceptions` (gold fingers, test points),
3. `fitted_sheets` / `not_fitted_sheets`: a sheet covers its sub-sheets, the most specific one wins,
4. `fitted` / `not_fitted`: references, `*` and `?` allowed.

A part that is not fitted connects nothing; its pads stay as copper and become stubs on the lanes, and nets that
only connect parts not fitted are not checked. When a board has variants – or forks where all alternative series
parts are fitted – and none is chosen, the lane commands stop with the list (CLI) or ask (viewer); see
[Getting started](getting-started.md#assembly-variants). In the viewer, drop the BOMs together with the board and
the kisi.json.

### `gaps.dismissed`
Some gaps are guesses from names: nets that look like an interface by their name (`nets-without-target`) and rails
found only by their name (`rails-without-voltage`, items with `"dismissable": true`). Items that are not what the
name suggests can be dismissed, with a reason:
```json
"gaps": {"dismissed": [
  {"gap": "nets-without-target", "items": ["/FPGA/QSPI_DQ0", "/FPGA/QSPI_DQ1"],
   "reason": "QSPI configuration flash at 50 MHz, short lines – no impedance control needed",
   "source": {"kind": "design", "note": "optional"}}
]}
```
`kisi gaps` leaves dismissed items out (a gap whose items are all dismissed is gone) and lists them under
`dismissed` with the reason, so a review still sees them. `config-check` requires one of the name-based gaps, a
reason and nets that exist; an item the gap reports but not as a name-based guess (a rail found by a pin name)
cannot be dismissed, and an item the gap no longer reports is a warning. Other gaps are filled or fixed, not
dismissed.

## Other input files
| File | Used by | Format |
|---|---|---|
| `--stackup FILE` | all commands | stackup changes as the viewer stores them: `{"dielectrics": [{"index", "epsilon_r", "loss_tangent", "thickness"}], "mask_epsilon_r"}` – the same as `stackup` in kisi.json, without sources |
| `--tree FILE` | `power`, `thermal` | a power tree as JSON instead of the `PI_REG`/`PI_LOAD` fields: `{"rails": [{"net", "voltage"}], "elements": [{"refdes", "id", "kind", "from", "to", "eff", "iq", "r", "imax", "in_ref", "out_ref"}], "loads": [{"refdes", "net", "current", "basis", "note", "at"}]}`; `tools/tree_to_fields.py` turns it into kisi.json fields |
| BOM (CSV) | `variants.*.bom` | a "Reference", "Designator" or "Ref" column (references separated by commas or spaces) |
| Touchstone (`.s2p`, `.s4p`, `.sNp`) | `connector_ends.*.touchstone` | Touchstone 1.x and 2.x |

## Output files
- `kisi report` writes everything into `analysis/kisi.json` next to the board (`--out` elsewhere), for
  file-based review tools and later sessions.
- `kisi net … --s2p FILE` / `--s4p FILE` write the channel's S-parameters as Touchstone.
- `kisi export` writes a lane or a bus into `analysis/export/<name>` next to the board (`--out` elsewhere): the
  subcircuit library, test benches and S-parameter decks for ngspice, Xyce and LTspice, Touchstone files, a KiCad
  project and a README; the viewer's "Export…" gives the same files as a zip, as its export settings choose
  ([CLI reference](cli.md#export)).
- The viewer exports Touchstone files per net and writes `<board>.kisi.json` from its Prepare PCB page
  ([Preparing a board](preparing-a-board.md#in-the-viewer-prepare-pcb)); its other edits stay in the browser.
