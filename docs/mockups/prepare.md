# Board preparation in the viewer – mockup and design note

Status: the third version was approved (24 Oct 2026) and is **implemented** in branch `feat/docs-prepare` – the
core (`views`, `applicable`, `--for`, `gaps.dismissed`, `config::subset`, check messages with categories), the WASM
exports `kisi_gaps`, `kisi_config_check`, `kisi_config_subset`, and the viewer's Prepare PCB page with all forms of
this note; user documentation in [Preparing a board](../preparing-a-board.md#in-the-viewer-prepare-pcb). Still open
(see the end of this note). The mockup below is kept as the design record.

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
Implemented (branch `feat/docs-prepare`): everything in "The page" and "In the core" above, with these
differences – "save only <category>" applies `config::subset` to the draft and lays the result over the loaded
file (so the loaded file's other entries are kept); the gap list runs in a Web Worker that replays the engine's
state, so the page stays responsive (about a second per run on ThunderScope); `config::to_json` writes the subset
without empty entries.

Still open:
- the loaded kisi.json's values do not fill the forms yet (its dismissals do; its other entries already close their
  gaps, so the forms show what is still missing);
- the draft lives in memory only (lost on a page reload before saving);
- dismissals the core did not apply (`not_applied`, e.g. an item no longer reported) are shown as check warnings,
  not next to the gap;
- capacitor ratings (`PI_CAP`), connector ends and variant definitions by sheet or BOM have no form – they stay on
  the agent path;
- the viewer's own stackup and profile edits (stored in the browser) cannot be moved into the kisi.json yet;
- per-item categories (a default εr matters for Impedance, a default tanδ for Channel).

