# kisi – agent skills (Claude Code plugin)

Two skills that use the kisi command line:

* **kisi-prepare** – finds what the analyses miss (`kisi gaps`), researches it from datasheets and
  writes it with sources to `<board>.kisi.json` (`kisi config-check` verifies the file).
* **kisi-si-pi-review** – SI/PI review: impedance per net class, channel checks against interface
  profiles, single nets, power (`kisi overview`, `interfaces`, `check`, `net`, `power`, `report`).

Setup:
1. Node.js ≥ 18 (nodejs.org, LTS).
2. In Claude Code: `/plugin marketplace add riehlies/kisi` and `/plugin install kisi@kisi`.

The command line itself is one file, `kisi.mjs`, attached to every GitHub release (with `kisi.html`,
the viewer). The skills download it on first use into `~/.kisi/`. It includes the engine of the viewer and
the 2D field solver; results of the field solver are cached in `~/.cache/kisi` (Windows:
`%LOCALAPPDATA%\kisi`). Every `node kisi.mjs <command> <board.kicad_pcb>` prints JSON;
`node kisi.mjs help` lists the commands. Other agents can use the same commands and the SKILL.md
files as instructions.
