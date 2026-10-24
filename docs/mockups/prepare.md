# Board preparation in the viewer – mockup and design note

Status: mockup for review, second version (24 Oct 2026), nothing implemented. Decided by the maintainer: one core
for humans and agents, plus preparation in the viewer; mockup first ([ROADMAP](../../ROADMAP.md), block 1). The
first review asked to split the preparation by simulation category – Impedance, Channel, Power (with the
temperature under Power) – because they need different data and a user may want to simulate only one of them.

[`prepare.html`](prepare.html) is a static, clickable mockup in the viewer's look (open it from the repository so
that it finds `web/fonts`). It embeds the real `kisi gaps` output of kisi 0.2.5 for three boards: ThunderScope
Rev5.3 without a kisi.json, ThunderScope with its example kisi.json (`web/samples/ts.kisi.json`), and the XIAO
nRF54LM20A without one. Each gap carries the proposed `views` field (below), derived from its `affects`. The check
imitates `kisi config-check` in a few lines of JavaScript. Files saved from the mockup pass the real
`kisi config-check` (tested on ThunderScope: everything entered, and the Impedance entries alone).

## The design
- **A "Prepare" card at the top of each view** (Impedance, Channel, Power), above the view's own cards. It shows:
  - the readiness of all three views, the current one highlighted – e.g. "Impedance: ready · Channel: 3 open
    (1 high) · Power: 5 open"; a click switches the view. The same state is a coloured dot on each view's button
    in the header;
  - this view's gaps, by priority, with "also Channel" where a gap serves another view too, and "KiCad" or
    "agent" where it is not filled here;
  - the shared draft ("10 entries – Impedance 5 · Channel 8 · Power 3"), "Enter values" and "Save kisi.json".
- **"Enter values"** opens a panel over the board map (like the power tree in the Power view): this view's gap
  cards with *where to find it*, where the value goes, and the forms – laminate εr/tanδ, net class targets and
  profiles, the variant, the regulator chain (`PI_REG`, with the suggestions of `rails-without-voltage`), loads,
  ferrite beads, thermal data – each block with a required source. Beside them: the check of the draft, the
  kisi.json preview and the agent path for this view's research-heavy gaps ("Copy prompt" for kisi-prepare with
  `kisi gaps --for <view>`).
- **Readiness of a view:** no open high or medium gap for it; low gaps are shown as refinements ("ready ·
  2 refinements"). Fixes in KiCad keep a view open until the board is reloaded.
- **Gaps in several views are filled once.** The draft belongs to the board, not to a view: the laminate entered
  in Impedance shows as filled in Channel; the variant chosen in Channel closes the gap in Power.
- **Partial preparation.** "Save" offers "everything entered" or "only <view> entries". The second writes only the
  entries that view uses – valid on its own, the other views keep their gaps – and can be saved while entries of
  another view still have errors (the mockup shows it: a `PI_REG` typo in Power, the Impedance entries saved).
  Which entry serves which view: stackup → Impedance and Channel; net class target → Impedance and Channel, its
  profile → Channel; variant → Channel and Power; `PI_*` fields → Power.
- **A loaded kisi.json** (the example boards, a user's file) is the base of the draft: saving merges into it – its
  entries stay, the draft adds entries or replaces those with the same key – and in the implementation its
  entries also fill the forms (not in the mockup).

### Why cards in the views, not a fourth view
The mockup's first version had a fourth view "Prepare". Cards are better here: the gaps appear where the user is
about to simulate, a user who only wants impedances never sees the power gaps, and the readiness of the other
views stays one click away in the same card. A fourth view would need category tabs anyway and would separate the
data from the results it changes. So only the card variant is shown.

## Working assumptions (proposed, to confirm)
- **Saving is blocked by errors in what is saved; warnings are fine.** With errors only in another view's entries,
  that view's own entries can still be saved alone.
- **Where the file goes:** in Chromium, when the board was opened through the File System Access API, the
  kisi.json is written next to the board (where "Reload" already reads); otherwise it is downloaded.
- **The gap list also runs on the example boards with their kisi.json**, so a prepared board shows what is still
  open (the mockup's "ThunderScope + kisi.json": Impedance 2 open – the KiCad fix and the laminate –, Channel 3,
  Power 2 and 2 low).

## The category in the core – no second logic
Everything that decides stays in `kisi_core`; the viewer shows and collects.

**`views` per gap, from `affects`.** Each gap in `gaps.rs` already lists what it affects (`"impedance (εr)"`,
`"loss and eye (tanδ)"`, `"DC IR drop"`, …). A small table in `gaps.rs` gives each of these words its view, and
`gap()` writes `"views"` from it, so a new gap gets its views from the words it lists anyway:

| affects | view |
|---|---|
| impedance, impedance (εr), impedance check, crosstalk | `impedance` |
| loss, eye, loss and eye (tanδ), channel check, lanes | `channel` |
| power budget, regulator loading, regulator losses, input current, DC IR drop, PDN, PDN (fitted capacitors), PDN target impedance, PDN impedance, PDN impedance per IC, decoupling recommendations, filter resonances, temperature estimate, junction temperature | `power` |

A test fails for a word without a view. With today's gaps this gives: `stackup-missing`, `stackup-defaults`,
`nets-without-target` → Impedance and Channel; `classes-without-standard`, `assembly-variant-forks`,
`channel-part-values` → Channel; `assembly-variant` → Channel and Power; all power gaps and `thermal-data` → Power.
A later refinement can put views on items (in `stackup-defaults`, a default εr matters for Impedance, a default
tanδ for Channel).

**Readiness in the summary.** `kisi gaps` adds per view `{"impedance": {"ready": true, "high": 0, "medium": 0,
"low": 1}, …}` next to today's `summary`, with the rule above – so viewer, CLI and agent count the same way.

**`kisi gaps --for impedance|channel|power`** keeps the gaps of that view only, and the summary of that view. The
skill kisi-prepare uses it when the user wants one analysis ("get the board ready for the impedance check"); the
viewer's "Copy prompt" names it.

**Which entry serves which view** (for "save only <view> entries" and for sorting check messages) also belongs in
the core, next to `config::check`:
- `config::check` returns each message with the path of its entry (`stackup.dielectrics[index 2]`,
  `net_classes.50Z`, `fields[J10 PI_REG]`) and that entry's views, besides today's `errors`/`warnings` strings;
- `config::subset(cfg, view)` returns the configuration with only the entries of that view.

The JavaScript keeps only a table from gap id to form (presentation, like the views' labels) and the merge of the
draft into a loaded file by keys; field syntax, ranges, sources, gaps, views and the subset stay in Rust.

| Need | Rust | WASM |
|---|---|---|
| gaps with views and readiness | `gaps::gaps` (+ `views`, per-view summary) | a `kisi_gaps(config?)` export on the viewer's own state; until then `kisi_cli_project` + `kisi_cli_run` with `{"cmd": "gaps"}` |
| does the draft close a gap? | `gaps::gaps` with the draft as configuration | the same call with the draft text (the mockup guesses in JavaScript – the real step asks the core) |
| check of the draft, per view | `config::check` (+ message paths and views) | a pure `kisi_config_check(text)` export – check without applying |
| save one view | `config::subset` | `kisi_config_subset(text, view)` |
| use the draft in the views | `config` apply | `kisi_config` (exists) |

The stored results of `kisi gaps` (`validation/golden/*.gaps.json`) change with the new fields and are updated in
the same commit.

What the mockup taught:
- Choosing the variant `default` closes `assembly-variant`, but then `kisi gaps` reports `assembly-variant-forks` –
  the step must ask the core again after each change instead of predicting.
- On ThunderScope, `nets-without-target` (four QSPI nets in the class "Default", found by their names) keeps
  Impedance open, and only a fix in KiCad closes it. Should a name-based gap that the user has looked at be
  dismissable ("not an interface"), stored in kisi.json?
- The XIAO has no net with a target impedance, yet Impedance shows the laminate gap. The per-view summary could say
  "nothing to check" for a view without targets (Impedance) or without lanes (Channel).

## Effort estimate
| Part | Days |
|---|---|
| Core: `views` from `affects` (table, test), per-view summary, `kisi gaps --for`, docs/cli.md, golden update | 1–1.5 |
| Core: check messages with path and views, `config::subset`, tests | 1 |
| WASM: `kisi_gaps`, `kisi_config_check`, `kisi_config_subset` | 0.5–1 |
| Viewer: the card in the three views and the header dots, the panel with the forms (stackup, classes, variant, PI_REG with suggestions, PI_LOAD, PI_SERIES, PI_THERMAL), shared draft, save per view, File System Access with download fallback, prefill from and merge into a loaded kisi.json | 5–6 |
| Tests: `check_viewer.py` per view (enter, save, the file through the native `config-check`), light/dark/phone | 1.5 |
| Docs (viewer section in `docs/preparing-a-board.md`, skill kisi-prepare with `--for`), screenshots | 0.5 |
| **Total** | **about 10 focused days (9.5–11.5)** |

A first version – the core's `views` and `--for`, the card and panel in Impedance and Power with laminate, net
classes, PI_REG and PI_LOAD, download only, no merge – about 5–6 days. The split by views adds about 2–3 days to the
first estimate (7–8 days): the core fields, the subset, and the card in three places.

## Open questions
- Dismissing a name-based gap ("these nets are not an interface"), and where to store that.
- "Nothing to check" for a view without targets or lanes, instead of showing its gaps.
- Whether the readiness dots in the header should be on by default or only after the first "Prepare" interaction.
