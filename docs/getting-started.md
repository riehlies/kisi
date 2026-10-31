# Getting started

kisi reads a KiCad layout and checks trace impedance, high-speed channels and power delivery. It runs in the
browser (the viewer) or on the command line; both use the same engine and give the same results.

## What you need
- The board file `.kicad_pcb` (KiCad 6 to 10), saved. A file from KiCad 5 or older is refused: open it in KiCad 6
  or newer and save it ([details](reference.md#what-kisi-reads-from-the-kicad-project)).
- The project file `.kicad_pro` next to it, for the net classes (target impedances and interface profiles come
  from them).
- Optionally `<board>.kisi.json`: the values the board file lacks (laminate data, load currents, …) with their
  sources – see [Preparing a board](preparing-a-board.md). Assembly variants may name BOM files, connector ends
  Touchstone files; drop or keep them next to it.

## The files of a release
Every [release](https://github.com/riehlies/kisi/releases/latest) carries:

| File | What it is |
|---|---|
| `kisi.html` | the viewer: one HTML file that runs offline in any current browser |
| `kisi-examples.html` | the same viewer with two example boards built in – ThunderScope Rev5.3 and LibreVNA – each with its power tree as a kisi.json |
| `kisi.mjs` | the command line for scripts, CI and agents (Node.js ≥ 18), with the 2D field solver |
| `kisi-docs.pdf` | this documentation as one PDF |
| `kisi-kicad-<version>.zip` | the KiCad plugin: a button in KiCad's PCB editor that opens the board in the viewer ([below](#from-kicad-the-plugin)) |
| `kisi-references.html` | 3D reference cases for vias, stubs, pairs, lines and plane cavities with field plots, and kisi's fast models against them (attached to the releases in which the cases changed, currently [v0.3.0](https://github.com/riehlies/kisi/releases/tag/v0.3.0)) |

The [live version](https://riehlies.github.io/kisi/) serves the viewer and the example viewer from GitHub Pages.

## The viewer
Open `kisi.html` (or the live version) and drop the `.kicad_pcb` with the `.kicad_pro` – and the board's
`kisi.json`, BOMs and Touchstone files if there are any – onto the page, or pick them with the file dialog.
Nothing is installed, the page loads nothing from the web, and the board never leaves your computer. In
Chromium-based browsers "Reload" reads the same files again after saving in KiCad.

The header switches between three views. Each starts with what the board file gives and computes more on demand,
in numbered steps.

### 1. Impedance
![The impedance view on the ThunderScope board: every track coloured by its deviation from the target](img/impedance.png)

Stackup and target impedances first. "Estimate impedances" computes formula values for every track in a second,
then the 2D field solver refines them – for the selected net, a class, all nets with a target, or everything;
results are cached in the browser. The map colours every track by its deviation from the target.

Per net: impedance along the path, loss, TDR, vias and crosstalk. The path runs from A to B (marked on the board
and under the charts; A is the transmitter side of the lane or the IC, ⇄ swaps it). Branches are marked where
they leave the path and included in loss and TDR as open stubs; hovering a chart shows the position on the board.

The estimate also gives NEXT/FEXT for every net with a target: the Couplings card lists the critical nets (from
1 %), the map mode "Crosstalk" marks the critical stretches, and per net a chart shows where along the path the
crosstalk comes from. The field solver refines only the critical couplings (on by default).

### 2. Channel
![The channel view: interfaces and lanes with their verdicts](img/channel.png)

The interfaces overview: buses and device-to-device lanes through series resistors and AC-coupling capacitors,
with terminations, branches and side stubs. Each net class gets an interface profile (PCIe, USB, DDR, LVDS,
Ethernet, HDMI, MIPI, … from `data/interfaces.json`) with transmitter, receiver and limits; the "Profile" card
shows and edits it. The check judges every lane on loss, return loss, skew, via stubs and a linear eye.

- A **differential pair** is a 4-port: both lines on their own paths with their own parts, stubs and vias, coupled
  where they run together. Loss and return loss are Sdd21/Sdd11, the eye drives P and N in opposite phase, and the
  mode conversion (Scd21) is shown.
- A **multi-drop line** along one path (one transmitter, more devices on side stubs) is checked at every receiver,
  all loading the line; the worst eye counts.
- **Lanes from a device to a connector** are judged on the board share (loss, return loss, skew). Their eye uses
  the end set in the interface's card – the termination when the connector receives, the driver when it sends –
  and, if given, the channel behind the connector as a lossy line or a Touchstone file (`.s2p`, `.s4p`). Without
  it the eye covers the board share only and is information; with it the eye is judged. Lanes from connector to
  connector (coupons, pass-through boards) get the board share only, without an eye.
- Parts from a lane's net to ground (a shunt capacitor) are in its channel at their pads.
- **Export…** in the lane detail and in the bus card (once the check has computed the lane) downloads the lane or
  the whole bus as SPICE subcircuits with test benches, Touchstone files and a KiCad project for circuit and RF
  simulators; the card "Export for simulators" sets what is written and the frequency range
  ([Channel export](models.md#channel-export)).

The board map shows the channels of the chosen variant – before the check in one colour, after it by result.
Nets that only connect parts not fitted are drawn faint and not checked. A click on a channel opens it.

### 3. Power
![Voltage drop of a rail and its impedance seen from an IC](img/power.png)

Power tree, budget and power flow (Sankey) first, from `PI_REG`/`PI_LOAD` fields or the board's kisi.json; part
references jump to the board. Then, in steps:
1. per rail and for the ground return, voltage drop and current density on every layer, the drop at each load
   and the vias with the highest currents;
2. a losses map: heat per part and copper loss per layer (mW/mm²);
3. the temperature (an estimate for an ambient temperature and still air or an air flow): a map per layer, the
   hottest parts with their junction temperature, the rails' IR drop at temperature;
4. Z(f): a lumped model of regulator, plane and capacitors with a plane model above 1 MHz, and the impedance seen
   from each IC – the rail domain with ferrites and series parts up to the regulator, the capacitors ranked by
   their loop inductance to the IC (outlined on the board), anti-resonances, a target from the IC's current step
   and recommendations (value, package, count, distance).

### Assembly variants
Boards with assembly variants (KiCad 10 variants or `variants` in kisi.json) – or with forks where all alternative
parts are fitted – ask "Which assembly variant?" before lanes are computed; the card "Assembly variant" switches
later, and the lane results name the variant. A lane is named after and made of its own path; where a line forks
with all alternatives fitted, each lane leaves the IC on its own branch and carries the other path as an open
branch (dashed on the board).

### Prepare PCB
"Prepare PCB" (between the example boards and "Load files") opens what the board file lacks, per category, with
forms for the simple values and for everything a loaded kisi.json holds, a draft kept in this browser until it is
saved, and the views' own edits taken into the file. It is a checklist: each step says what it needs and shows
how many of its items are open, "Apply" checks it with the core and gives it a check mark, and the next one opens;
the badge on the button counts the open points. The example boards come with a complete kisi.json: their
checklist is finished and shows the values in its forms. See
[Preparing a board](preparing-a-board.md#in-the-viewer-prepare-pcb).

### What the viewer stores
Stackup (εr, tanδ, thickness), profiles, the net class → profile assignment, power settings, lane directions and
the chosen variant can be edited; edits are stored in this browser per board and lie over the kisi.json (Prepare
PCB moves them into the kisi.json). The
KiCad files are never changed. Field-solver results are cached in the browser (IndexedDB); the results of the
last three boards stay in memory while switching. The "Accuracy" section of the impedance view summarises the
model accuracy ([Models and limits](models.md)).

## From KiCad: the plugin
A button in KiCad's PCB editor (KiCad 10) opens the open board in the viewer, in
your default browser, with its `<board>.kisi.json`. It has been tested in KiCad 10 on Windows; Linux and macOS
should work the same way but are not tested yet – a report of how it went helps (see *Reporting a problem* below).

**Install** – either way, in KiCad's project window: *Plugin and Content Manager*
- search for **kisi** and install it (once the package is in KiCad's repository), or
- download `kisi-kicad-<version>.zip` from the [latest release](https://github.com/riehlies/kisi/releases/latest)
  and choose *Install from File…* (bottom left).

The plugin uses the KiCad API: if KiCad asks whether to enable it, say yes – otherwise tick *Preferences →
Plugins → Enable KiCad API*. On the first start KiCad sets up the plugin's own Python environment and installs
`kicad-python` from the internet (a minute or so); the button appears when that is done. It sits at the right
end of the PCB editor's top toolbar (an eye-diagram icon, "Open in kisi"); the plugin's actions are also listed
in *Preferences → PCB Editor → Plugins* (KiCad 10), where its environment can be recreated (right-click).

**What the button does**
1. It asks KiCad which board is open and reads the saved `.kicad_pcb`, the `.kicad_pro` and the
   `<board>.kisi.json` next to the board. Changes not yet saved in KiCad are not included: save first.
2. If the viewer downloaded a `<board>.kisi.json` since the last click (browsers without a folder dialog, see
   below), it takes that file over from your downloads folder first – only when exactly one such file
   (`<board>.kisi.json`, also `<board>.kisi(1).json`, `<board>.kisi (1).json`) is newer than the one next to the
   board, is a kisi.json for this board and passes `kisi config-check` (when Node.js is installed). The previous
   file is kept as `analysis/<board>.kisi.json.<date-time>.bak`; anything doubtful (several files, another board,
   an error) leaves everything as it is, and the viewer says why.
3. If there is no `<board>.kisi.json`, it creates an empty one next to the board.
4. It writes a copy of the viewer with these files built in to `analysis/<board>.kisi.html` next to the board – the
   folder `kisi report` and `kisi export` use – and opens it; each click writes it anew. If `analysis/` cannot be
   written (a read-only folder), the page goes to the plugin's own folder instead (the one with the log, see
   below), and the viewer says so. The page works offline, like `kisi.html`; the card "From KiCad" names the board, its
   folder, the page and what happened to the kisi.json. In a project under version control, add `analysis/` to
   `.gitignore`.

Next to the board the plugin writes nothing but `<board>.kisi.json` and, in `analysis/`, the page and the backups;
KiCad's files are never written or saved.

**Saving kisi.json** – on the viewer's *Prepare PCB* page, "Save kisi.json" writes the file next to the board,
where the next click reads it:
- in Chrome and Edge: **Choose the board folder…** once per board (the page shows the folder's path; kisi checks
  that the chosen folder holds the board). From then on "Save kisi.json" writes the file straight into that folder,
  without a dialog; the browser asks once per session for permission to edit files there. Without a chosen folder
  a save dialog opens instead ("Copy path" copies the full file name, which can be pasted into the dialog's file
  name field).
- in Firefox and Safari the file goes to the downloads folder: then **click kisi in KiCad again** – the plugin moves
  it next to the board (step 2 above); the page says so after saving. To choose the folder yourself instead, let the
  browser ask where to save: Firefox: *Settings → General → Downloads → Always ask you where to save files* (Firefox
  then remembers the last folder); Safari: *Settings → General → File download location → Ask for each download*.

**With an agent** – the plugin carries kisi's command line (`cli/kisi.mjs` in its folder) and the
[agent guide](agents.md#any-assistant-the-agent-guide) for any assistant (`agent/KISI-AGENT-GUIDE.md`). On *Prepare
PCB* the agent card has "Copy guide" and "Copy prompt": give both to the assistant. The prompt names the guide, the
board, where its kisi.json belongs and the CLI. An agent that can run commands uses the ready command shown on the
card (with Node.js 18 or newer installed, [nodejs.org](https://nodejs.org), LTS), e.g.
`node "…\com_github_riehlies_kisi\cli\kisi.mjs" gaps "…\board.kicad_pcb" --for impedance`, and writes the
kisi.json there itself; a chat assistant asks for the commands' output and returns the complete kisi.json, which
you save next to the board. The next click in KiCad loads it. Without Node.js the card says where to get it.

**If nothing happens** – KiCad 10 shows the plugin's error at the warning triangle in the PCB editor's status bar
(bottom right), and the browser opens a page saying what went wrong. Details are in the plugin's log
`kisi-kicad.log`, in the plugin's own folder:
- Windows: `%TEMP%\kisi-kicad\kisi-kicad.log` (paste `%TEMP%\kisi-kicad` into the Explorer's address bar),
- macOS: `$TMPDIR/kisi-kicad/kisi-kicad.log` (in Terminal: `open "$TMPDIR/kisi-kicad"`),
- Linux: `~/.cache/kisi-kicad/kisi-kicad.log` (KiCad as a Flatpak: `~/.var/app/org.kicad.KiCad/cache/kisi-kicad/`).

If the button does not appear, check that the API is enabled and that *Preferences → Plugins* shows a Python
interpreter. To remove the plugin, uninstall it in the Plugin and Content Manager; the kisi.json files next to your
boards stay.

**Reporting a problem** – open an issue at [github.com/riehlies/kisi/issues](https://github.com/riehlies/kisi/issues)
with your system, the KiCad version (*Help → About KiCad*, "Copy Version Info"), the browser, the message KiCad or
the error page showed and the end of `kisi-kicad.log` (it holds folder and file names and kisi's messages, not the design
itself).

## The command line
```sh
curl -fsSLO https://github.com/riehlies/kisi/releases/latest/download/kisi.mjs
node kisi.mjs help
node kisi.mjs gaps     board.kicad_pcb   # what the analyses miss
node kisi.mjs overview board.kicad_pcb   # stackup and impedance per net class
node kisi.mjs check    board.kicad_pcb   # channel check of all interfaces
node kisi.mjs power    board.kicad_pcb   # regulator chain, IR drop, losses
node kisi.mjs export   board.kicad_pcb --lane NET   # a lane for SPICE, Touchstone and KiCad
```
Every command prints JSON and states the models and assumptions it used. The first run of a field-solver command
on a board can take a few minutes; results are cached. All commands and options: [CLI reference](cli.md).

## Next steps
- A board straight from KiCad gives a first picture; for meaningful numbers, fill the gaps:
  [Preparing a board](preparing-a-board.md).
- What the numbers mean and how far to trust them: [Models and limits](models.md).
- Let an agent do the research and the review: [Using kisi with agents](agents.md).
