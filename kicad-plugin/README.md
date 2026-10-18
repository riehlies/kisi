# kisi – KiCad plugin

Opens the standalone kisi viewer with the board that is open in the KiCad PCB editor
(KiCad 9/10, IPC API). The board is copied to a temporary folder with “save a copy” –
your own files are neither saved nor changed. If a track, via or pad is selected, its net is
pre-selected in the viewer.

## Installation
1. `python3 tools/package_plugin.py` → `dist/kisi-kicad-plugin.zip`
2. Unpack the folder `kisi` from the zip into KiCad's plugin folder:
   - Linux `~/.local/share/KiCad/9.0/plugins/`
   - Windows `%USERPROFILE%\Documents\KiCad\9.0\plugins\`
   - macOS `~/Documents/KiCad/9.0/plugins/`
3. In KiCad: Preferences → Plugins → “Enable API server”. On first use KiCad creates the
   Python environment with `kicad-python` (requirements.txt).

Actions: “kisi: Open viewer” (toolbar) and “kisi: Check channel” (starts in the channel view).

## Without KiCad
`python3 kisi_open.py --pcb board.kicad_pcb [--net NET] [--view si|ch|pi]`

## How it works
The viewer stays independent: in a copy of `kisi.html` the plugin only replaces the line
`const PRELOAD = null; /*KISI_PRELOAD*/` with the board (gzip + base64) and opens the file in the browser.
IBIS files named in the footprints' simulation fields (`Sim.Device` = IBIS, `Sim.Library`) are embedded as well
(resolved like KiCad: `${KIPRJMOD}`, relative to the project, else by file name in the project folder; up to 64 MB each).
