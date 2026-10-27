# kisi agent guide: prepare a KiCad board for kisi

For any AI assistant – one that can run commands on the user's computer (a coding agent) or a chat assistant
without a shell (the user runs the commands and pastes the output). Give this file to the assistant together with
the prompt from kisi's viewer (Prepare PCB → agent card → "Copy prompt" and "Copy guide"), or attach it.

kisi checks signal and power integrity of a KiCad board: trace impedance per net class, high-speed channels,
vias, crosstalk, the regulator chain, DC voltage drop, PDN impedance and temperatures. It reads the saved
`.kicad_pcb` (and the `.kicad_pro` next to it for net classes). Many values a good analysis needs are not in those
files, or only as KiCad's defaults. Your job: find them, look them up, and write them **with their source** to
`<board>.kisi.json` next to the board (same name: `board.kicad_pcb` → `board.kisi.json`). kisi lays that file over
the board when it loads it. **Never edit the KiCad files themselves.**

## How to run kisi
kisi's command line is one file, `kisi.mjs`, run with Node.js 18 or newer: `node "<path>/kisi.mjs" <command>
"<board>.kicad_pcb" [options]`. Below it is written as `kisi <command> <board>`. Every command prints JSON.

- **The paths:** with kisi's KiCad plugin the prompt from the viewer names them – the plugin's `kisi.mjs`
  (`…/3rdparty/plugins/com_github_riehlies_kisi/cli/kisi.mjs` in KiCad's documents folder), the board file and where
  its `kisi.json` belongs. Without the plugin: `kisi.mjs` from https://github.com/riehlies/kisi/releases/latest.
  Node.js: https://nodejs.org (LTS).
- Preparing needs only fast commands: `gaps`, `config-check`, `config-template`, `overview --formula`,
  `interfaces` and `profiles`. `kisi help` lists all commands.

### Mode A – you can run commands
Run the commands yourself with the paths from the prompt. Write `<board>.kisi.json` next to the board directly
(merge into the existing file – keep the user's entries and the dismissed gaps), then run `kisi config-check
<board>` until it reports no errors. The user then clicks kisi in KiCad again (or reloads the viewer) to see it.

### Mode B – chat without a shell
Ask the user for what you need, as exact commands to copy (with the paths from the prompt), and work from the
pasted output:
1. the output of `kisi gaps <board>` (or `kisi gaps <board> --for impedance` – also `channel`, `power` – for one
   analysis),
2. the current `<board>.kisi.json` (its full text; the viewer's Prepare PCB page shows it on the right),
3. when needed, `kisi overview <board> --formula`, `kisi interfaces <board>`, `kisi profiles`, and the datasheets
   or the BOM (ask the user to paste or upload them; many manufacturer PDFs cannot be fetched).

Then return the **complete new `<board>.kisi.json`** in one JSON code block – the existing content merged with
your additions, nothing left out – and say where to save it (next to the board, the path from the prompt) and that
`kisi config-check <board>` checks it; ask for its output and fix what it reports. Never return only a fragment:
the user replaces the whole file.

In both modes `kisi …` below means: run it (mode A) or ask the user to run it and paste the output (mode B).

## Steps
{{steps}}
7. **Report to the user:** what was filled (with sources), what was assumed (estimates and assumptions, marked
   as such), what is still open, and the fixes that belong in KiCad (step 1). In mode B, the complete kisi.json
   comes with this report.

## Notes
{{notes}}
- The worked example and the reference live in the kisi repository: https://github.com/riehlies/kisi
  (`docs/preparing-a-board.md`, `docs/reference.md`, `docs/examples/`).
