# Board preparation in the viewer – mockup and design note

Status: the third version was approved (24 Oct 2026) and is **implemented and functional** in branch
`feat/docs-prepare` – the core (`views` per gap and per item, `applicable`, `--for`, `gaps.dismissed`,
`config::subset`, check messages with categories, capacitor suggestions, tree/setups → fields), the WASM exports
`kisi_gaps`, `kisi_config_check`, `kisi_config_subset`, `kisi_edits_to_fields`, `kisi_board_sheets`, and the
viewer's Prepare PCB page; user documentation in [Preparing a board](../preparing-a-board.md#in-the-viewer-prepare-pcb).
What differs from this note and what is still open: see the end. The mockup below is kept as the design record.

History of the mockup: v1 was a fourth view "Prepare" with one gap list; v2 split the gaps by simulation category
into a card at the top of each analysis view; the maintainer's answers to v2 brought the page back, with the
categories as tabs, dismissable name-based gaps and "nothing to check" per category; a last change moved its button
("Prepare PCB") to the board actions of the top bar. This note describes v3.

[`prepare.html`](prepare.html) is a static, clickable mockup in the viewer's look (open it from the repository so
that it finds `web/fonts`). It embeds the real `kisi gaps` output of kisi 0.2.5 for three boards – ThunderScope
Rev5.3 without a kisi.json, ThunderScope with its example kisi.json (`web/samples/ts.kisi.json`), the XIAO
nRF54LM20A without one – with the proposed `views` per gap, and per category whether there is anything to check
(from `kisi overview` and `kisi interfaces`: ThunderScope 9 net classes with a target and 26 buses, the XIAO none).
The check imitates `kisi config-check` in a few lines of JavaScript. A file saved from the mockup – the ThunderScope
laminate plus the dismissed QSPI nets, "save only Impedance" – passes the real `kisi config-check`.

## The page
- **"Prepare PCB" in the top bar**, between the example boards and "Load files" – with the other board actions,
  not among the analysis views – with a badge counting the open high and medium gaps. The three analysis buttons
  (Impedance, Channel, Power) carry a **readiness dot**, on by default: green ready, orange or red open (medium /
  high), grey nothing to check.
- **Tabs Impedance, Channel, Power** (temperature under Power), each with its readiness ("ready", "2 open (1
  high)", "nothing to check"). A tab lists that category's gaps **by priority**, dismissed gaps last. Each gap card
  has *where to find it*, where the value goes, what it affects ("also Channel" where it serves another
  category), and the form for its values – laminate εr/tanδ, net class targets and profiles, the variant, the
  regulator chain (`PI_REG` with the suggestions of `rails-without-voltage`), loads, ferrite beads, thermal data –
  each block with a required source. Fixes in KiCad and research-heavy gaps are marked as such.
- **Side column:**
  - *Prepare the board* – the readiness of the three categories (a click opens the tab), the shared draft
    ("6 entries – Impedance 6 · Channel 6 · Power 0") and "Save kisi.json" with "save everything entered" or
    "save only <category>";
  - *Check* – config-check's messages for this tab's entries, and how many there are in the other categories;
  - *Agent path* (from v1, now per tab) – the gaps handed to the agent and the prompt for kisi-prepare limited to
    the category: "Start from `kisi gaps --for channel`, research the gaps … with a source for every value. Merge
    the results into … – keep my entries and the dismissed gaps – and run `kisi config-check`." "Copy prompt"
    copies it; a category with nothing to check has no prompt.
- **kisi.json preview** in the right column, for the chosen save scope.
- **One draft per board:** a value entered under one tab counts in the others (the laminate in Impedance and
  Channel; the variant in Channel and Power).
- **Dismissing a gap:** gaps whose items were found only by a name have "Dismiss…": choose the items (the four
  ThunderScope QSPI nets in class "Default", found by their names as `ddr4_dq`; the XIAO's VBAT rail, found by its
  name), give a required reason, and the items no longer count – on ThunderScope this makes Impedance "ready".
  "Undo" brings them back. The card says where the dismissal is stored: `gaps.dismissed` in `<board>.kisi.json`.
- **Nothing to check:** the XIAO has no net class with a target impedance and no lanes between devices, so
  Impedance and Channel say "nothing to check"; the laminate gap is listed below as "not blocking", with the note
  that the laminate matters once net classes get a target impedance.

## Working assumptions (proposed, to confirm – unchanged since v2)
- **Saving is blocked by errors in what is saved; warnings are fine.** With errors only in another category's
  entries, this category's entries can still be saved alone.
- **Where the file goes:** in Chromium, when the board was opened through the File System Access API, the
  kisi.json is written next to the board (where "Reload" already reads); otherwise it is downloaded.
- **The gap list also runs on the example boards with their kisi.json**, so a prepared board shows what is still
  open (the mockup's "ThunderScope + kisi.json").

## In the core – one logic for viewer, CLI and agents
Everything that decides stays in `kisi_core`; the viewer shows and collects.

### Categories: `views` per gap
Each gap in `gaps.rs` lists what it affects. A table in `gaps.rs` gives each of these words its category and
`gap()` writes `"views"` from it; a test fails for a word without one:

| affects | view |
|---|---|
| impedance, impedance (εr), impedance check, crosstalk | `impedance` |
| loss, eye, loss and eye (tanδ), channel check, lanes | `channel` |
| power budget, regulator loading, regulator losses, input current, DC IR drop, PDN, PDN (fitted capacitors), PDN target impedance, PDN impedance, PDN impedance per IC, decoupling recommendations, filter resonances, temperature estimate, junction temperature | `power` |

With today's gaps: `stackup-missing`, `stackup-defaults`, `nets-without-target` → Impedance and Channel;
`classes-without-standard`, `assembly-variant-forks`, `channel-part-values` → Channel; `assembly-variant` →
Channel and Power; the power gaps and `thermal-data` → Power.

### Readiness and "nothing to check"
`kisi gaps` adds per category:
```json
"views": {
  "impedance": {"applicable": false, "why": "no net class with a target impedance", "ready": true, "high": 0, "medium": 0, "low": 0},
  "channel":   {"applicable": false, "why": "no buses or lanes between devices", …},
  "power":     {"applicable": true, "ready": false, "high": 0, "medium": 4, "low": 2}
}
```
- `applicable` comes from the same project the analyses use: Impedance needs a net (or class) with a target
  impedance, Channel a bus or lane (`topology`), Power a supply rail. Gaps of a category that is not applicable
  stay in the list with `"blocking": false`, so the laminate gap is still visible for later.
- `ready`: applicable and no open high or medium gap of that category (low gaps are refinements).
- **`kisi gaps --for impedance|channel|power`** keeps that category's gaps and summary; the skill kisi-prepare uses
  it when the user wants one analysis, and the viewer's prompt names it.

### Dismissed gaps
Stored in `<board>.kisi.json`, so the viewer, the CLI and agents see the same:
```json
"gaps": {"dismissed": [
  {"gap": "nets-without-target",
   "items": ["/FPGA/FPGA IO Banks/QSPI_DQ0", "/FPGA/FPGA IO Banks/QSPI_DQ1", "/FPGA/FPGA IO Banks/QSPI_DQ2", "/FPGA/FPGA IO Banks/QSPI_DQ3"],
   "reason": "QSPI configuration flash at 50 MHz, short lines – no impedance control needed",
   "source": {"kind": "design", "note": "optional, like every kisi.json value"}}
]}
```
- **Which items can be dismissed** is decided in `gaps.rs`: items found only by a name heuristic get
  `"dismissable": true` (`nets-without-target`: `looks_like` from the net name; `rails-without-voltage`: rails
  found by name, not by a pin or a field). Items that carry a fact (a default εr in the board file, a missing
  regulator) cannot be dismissed – they are filled or fixed.
- **`config`**: a `gaps.dismissed` section in `BoardConfig`; `config::check` requires a known gap id, items that
  this gap can dismiss on this board, and a non-empty reason (error otherwise); an item no longer reported is a
  warning ("dismissal no longer needed").
- **`gaps`** leaves dismissed items out, closes a gap whose every item is dismissed, and lists them under
  `"dismissed"` with their reason, so a review still sees what was waved through. The skills read the same list
  and keep it when they merge their results.
- kisi 0.2.5 already ignores unknown kisi.json keys, so a file with `gaps.dismissed` stays readable by older
  versions (without the effect).

### Check, subset, WASM
- `config::check` returns each message with the path of its entry (`stackup.dielectrics[index 2]`,
  `net_classes.50Z`, `fields[J10 PI_REG]`, `gaps.dismissed[nets-without-target]`) and that entry's categories,
  besides today's `errors`/`warnings` strings.
- `config::subset(cfg, view)` returns only the entries of one category (stackup and net class targets: Impedance
  and Channel; net class profiles: Channel; variant: Channel and Power; `PI_*` fields: Power; a dismissal: its
  gap's categories) – "save only <category>".
- WASM: `kisi_gaps(config?)` on the viewer's own state (with views, applicability and dismissals),
  `kisi_config_check(text)` (check without applying), `kisi_config_subset(text, view)`; `kisi_config` applies the
  draft as today. Whether an entry closes a gap is answered by running `gaps` again on the draft – the mockup
  guesses in JavaScript, the real page asks the core (choosing the variant `default` closes `assembly-variant` but
  makes `kisi gaps` report `assembly-variant-forks`).
- The JavaScript keeps a table from gap id to form (presentation only) and the merge of the draft into a loaded
  kisi.json by keys.
- The stored results of `kisi gaps` (`validation/golden/*.gaps.json`) change with the new fields and are updated in
  the same commit.

## Effort estimate
| Part | Days |
|---|---|
| Core: `views` from `affects` (table, test), per-category readiness and `applicable`, `kisi gaps --for`, docs, golden update | 1.5 |
| Core: dismissals – `dismissable` items, `gaps.dismissed` in config with its check, gaps leaving them out and listing them, skill update | 1–1.5 |
| Core: check messages with path and categories, `config::subset`, tests | 1 |
| WASM: `kisi_gaps`, `kisi_config_check`, `kisi_config_subset` | 0.5–1 |
| Viewer: the Prepare page (tabs, gap cards by priority, forms, dismiss, nothing to check), readiness dots, shared draft, save per category, File System Access with download fallback, agent card per tab, prefill from and merge into a loaded kisi.json | 5–6 |
| Tests: `check_viewer.py` (each tab, enter, dismiss, save, the file through the native `config-check`), light/dark/phone | 1.5 |
| Docs (`docs/preparing-a-board.md`, `docs/reference.md` for `gaps.dismissed`, `docs/cli.md` for `--for`), screenshots | 0.5 |
| **Total** | **about 11–13 focused days** |

A first version – `views`, `applicable` and `--for` in the core, the page with the tabs and forms for laminate, net
classes, PI_REG and PI_LOAD, download only, no dismissals and no merge – about 6 days. Compared with v2 (about
10 days) the page costs about the same as the cards; the dismissals and "nothing to check" add about 1.5–2 days.

## Implemented, and what is still open
Implemented (branch `feat/docs-prepare`): everything in "The page" and "In the core" above, and then:
- **The draft is the kisi.json:** the loaded file (or a skeleton) with the edits. Every entry of it is in a form
  under "In `<board>`.kisi.json" on its tab – laminate and solder mask, class targets, profiles, connector ends,
  the variant and the variant definitions, one table per field name (`PI_REG`, `PI_LOAD`, `PI_SOURCE`,
  `PI_SERIES`, `PI_CAP`, `PI_THERMAL`, `PI_TARGET`, others), notes. Forms change entries by key; keys without a
  form (and source keys such as `url`, `page`) stay. Loaded and saved unchanged, the file is the same
  (`check_viewer.py` compares).
- **Kept per board** in the browser (`kisi.prep.v1.<board>`, like the stackup and profile edits) until saved or
  discarded; the side card counts what is added, changed and removed against the loaded file; a draft made over
  another kisi.json is kept and marked.
- **Save:** everything, or the loaded file with one category's changes – the entries of `config::subset` of the
  loaded file are replaced by those of the draft, entry by entry, so removals in that category count too; the
  rest stays in the draft.
- **Forms added:** capacitor data per part number (or value and package) for the whole group, with the
  suggestion the core reads from the parts' own fields (`Voltage Rating`, `Dielectric`, `Description`; source kind
  `design`); connector ends with the Touchstone file, port order and ports per pin; variant definitions by sheets
  (the board's sheets as suggestions), parts, BOM and BOM exceptions.
- **The views' edits** (stackup, targets, profiles, connector ends, the power tree or rail setups) are listed on
  their tab and move into the draft with one source; power settings go through `tree::spec_to_fields` /
  `pi::setups_to_fields` in the core (round-trip tests), and only the parts whose fields change are written.
- **Categories per item** from the core: dielectrics with only a default tanδ are Channel items, forks are Channel
  items, defined variants Channel and Power; a tab shows a gap's items for it.
- **Stale dismissals** (the core's `not_applied`) next to their gap, or as "Stale dismissals" on the tab, with remove.
- "hand to the agent" on any gap with a form; a category whose open gaps are all handed over says so.
- The gap list runs in a worker, one run at a time (edits during a run ask for one more afterwards).

**Checklist** (after the maintainer's review of the functional version: "what do I really have to enter, and where
do I confirm?"):
- each gap is a step with a marker – a red or amber number while open (high or medium; low-priority steps are
  optional), a green check mark when done, an arrow when with the agent, a dash when dismissed – ordered by
  priority; the first open step of a tab is open when the tab is first shown;
- one line says what the step needs (which fields, how many items); only those inputs are shown; *more* holds the
  explanation and optional fields (material and thickness, ψJT and Tj max, ESR and ESL, the load basis);
- **Apply** commits a filled new row, runs the gap list at once and then gives the step its check mark (the core
  no longer reports it for this tab, or only dismissed items) and opens the next open step – or keeps it open
  with "N of M … still missing: …" and the check's errors for its entries; "hand to the agent" settles a step too;
- progress per tab ("1 of 2 required steps done", a bar), "Ready for the impedance check" (channel check, power
  analysis) with a button to the view at the end, and saving the kisi.json as the last step;
- the readiness dots on the view buttons are gone; the badge on "Prepare PCB" counts the open points. (A
  "Preparation" card in the views' side panel was tried and removed again after the review of 25 Oct: one entry
  point, the button; the Assembly variant card stays where it was.)
- review of 25 Oct: the marker of an open step shows the number of its items still open (not a running ordinal
  that shifted when a step was done); Apply stays in every step with a form (it vanished when the core reported a
  step done or dismissed while typing, and a redraw between press and release swallowed the click – full redraws
  now wait for the release); the board keeps its place when coming back from the page (a fit asked for while the
  canvas was hidden happens once it is shown); nets in "Default" get a class of their own
  (`net_classes.<name>.nets`, a small core addition); the example boards come with a complete kisi.json.

Still open, with the reason:
- Interface profile parameters edited in the Channel view (limits, data rate) are stored per browser for all
  boards; kisi.json has no key for them (profiles are `data/interfaces.json`), so they cannot move into it.
- Lane direction swaps and path flips are viewer display choices, not data of the board – no kisi.json key.
- Pads chosen for a source or load in rail setups: fields name parts, not pads – the page says which pad choices
  are lost when the setups are taken over.
- A part removed from the power tree whose `PI_REG` / `PI_LOAD` comes from the KiCad file cannot be removed by
  kisi.json – the page names it (change it in KiCad).
- Capacitor ratings from the parts' fields are a suggestion on the page, not read by the analyses (ROADMAP:
  capacitor ratings from part fields – a model change with its own comparison).
- The forms do not parse field values themselves; the core's check reports what it cannot read.
- Undo is per gap for dismissals and "Discard draft" for the whole draft; there is no step-by-step undo.
- Values that need datasheets or curves stay on the agent path (by design).
