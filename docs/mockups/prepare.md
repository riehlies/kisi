# "Prepare" step in the viewer – mockup and design note

Status: mockup for review (24 Oct 2026), nothing implemented. Decided by the maintainer: one core for humans and
agents, plus a "Prepare" step in the viewer; mockup first ([ROADMAP](../../ROADMAP.md), block 1).

[`prepare.html`](prepare.html) is a static, clickable mockup in the viewer's look (open it from the repository so
that it finds `web/fonts`). It embeds the real `kisi gaps` output of kisi 0.2.5 for ThunderScope Rev5.3 and the
XIAO nRF54LM20A (both without a kisi.json; long item lists shortened); the board switch in the header changes
between them. The check is imitated in a few lines of JavaScript with the messages of `kisi config-check`. A file
exported from the mockup passes the real `kisi config-check` (tested on ThunderScope).

## What the step does
- **Where:** a fourth view "Prepare" before Impedance, Channel and Power, with a badge counting the open high and
  medium gaps. It opens with the board and stays optional – a well-prepared board shows 0.
- **Gap list** (main column): every gap of `kisi gaps` as a card, by priority, with the issue, *where to find it*,
  where it goes in kisi.json and what it affects. A filter shows high / medium / low / open gaps.
- **Inline entry** of the simple values, each block with a required source (kind, document or url, note):
  - laminate εr, tanδ and material per dielectric (`stackup-defaults`);
  - net class target, differential flag and interface profile (`classes-without-standard`);
  - the regulator chain as `PI_REG` rows with suggestions from `rails-without-voltage` (e.g. "PI_REG on FB3:
    IMU&MIC_3V3 > "/06 Peripherals/IMU_VDD" pass") that add a row in one click;
  - load currents per IC and rail (`ic-loads`, with the MPN where the Value field is not the part number),
    typical or maximum;
  - ferrite beads (`PI_SERIES`), thermal data (`PI_THERMAL`), the assembly variant (`default` or a defined one).
- **Fixes in KiCad** (`nets-without-target` in the class "Default", unreadable Value fields) are shown as such,
  without a form.
- **Agent path:** research-heavy gaps (capacitor ratings of 256 parts, variants from BOMs, efficiencies read from
  curves) are listed in the "Agent path" card; any other gap can be handed over too. "Copy prompt" gives a prompt
  for the skill kisi-prepare, which merges its results into the same kisi.json (keeping the user's entries); loaded
  again, the file fills the step.
- **Check** (side card): the result of `config-check` on the draft, live; errors block the export.
- **Export:** the kisi.json preview (right column) and "Download kisi.json" – saved next to the board, read by
  the CLI and the viewer from there.

## Reusing kisi_core – no second logic
The rule of the ROADMAP is that the viewer gets no logic of its own. The step only collects values and builds
JSON; everything that decides is Rust:

| Need | Rust (exists) | WASM today | Change |
|---|---|---|---|
| gap list | `kisi_core::gaps::gaps(&Project, &Profiles)` | `kisi_cli_project` + `kisi_cli_run` with `{"cmd": "gaps"}` (the CLI exports are in the same module) | none for a first version; later a `kisi_gaps()` export on the viewer's own state, so the board is not parsed twice |
| check of the draft | `kisi_core::config::check` (format, ranges, references, sources, field syntax by parsing like the analyses) | `kisi_config` runs it, but also applies the file | a pure `kisi_config_check(text)` export (about 10 lines): check without applying, for every edit (debounced) |
| does an entry close its gap? | `gaps` with the draft as configuration | the same `kisi_cli_project` call with the draft text | none – the mockup's JavaScript guess is replaced by running `gaps` again |
| errors per row | the check's messages carry the reference (`U6: PI_REG '…': 'pas' not understood`, `fields[U6 PI_REG]: …`) | – | map messages to rows by their prefix (presentation only) |
| use the draft in the views | `kisi_config` (apply) | exists | "Use" applies the draft like a dropped kisi.json; the views recompute |

The JavaScript keeps a table from gap id to editor (which form to show) – presentation, like the labels of the
views. Field syntax, ranges, sources and the gap list stay in `kisi_core`, so CLI, agents and viewer agree. What
the mockup taught: choosing the variant `default` closes `assembly-variant`, but then `kisi gaps` reports
`assembly-variant-forks` – exactly why the step must ask the core again after each change instead of predicting.

**Merging with an existing kisi.json:** a dropped file prefills the editors; the export keeps every key the step
does not edit (`notes`, `variants`, `connector_ends`, sources) and replaces entries by their key (dielectric index,
class name, `ref` + field name). That merge is plain JSON; if the agent path should use the same rule, it moves
into `kisi_core::config` as `merge(base, patch)` with a WASM export.

**Edits that already exist** (stackup and profiles edited in the viewer, stored in the browser) can be offered as
entries ("put into kisi.json", source required), so they are no longer lost in one browser.

**Smaller downloads** (ROADMAP: separate WASM builds for viewer and CLI): the viewer then keeps `gaps` and
`config::check` but not the command layer – with the `kisi_gaps` export of the second step that costs nothing.

## Effort estimate
| Part | Days |
|---|---|
| WASM: `kisi_config_check(text)`, `kisi_gaps()` on the viewer state, gaps with a draft configuration | 1 |
| Viewer: the view, cards, editors (stackup, classes, variant, PI_REG with suggestions, PI_LOAD, PI_SERIES, PI_THERMAL), source blocks, preview, export, prefill and merge with a loaded kisi.json | 4–5 |
| Tests: `check_viewer.py` (enter values, export, the export through the native `config-check`), light/dark/phone | 1–1.5 |
| Docs (viewer section in `docs/preparing-a-board.md`), screenshots | 0.5 |
| **Total** | **about 7–8 focused days** |

A first version with laminate, net classes, PI_REG and PI_LOAD, without merge (export of a new file only) and
through the existing CLI exports: about 4–5 days.

## Open questions
- A fourth view (as in the mockup) or a card in the existing views?
- Export only without errors (mockup), or with a confirmation?
- With the File System Access API (Chromium), save the kisi.json directly next to the board, where "Reload"
  already reads from?
- Should the gap list also run on the example boards with their kisi.json (a "what is still open" view for
  prepared boards)?
