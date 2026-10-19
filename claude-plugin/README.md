# kisi – Claude Code plugin (skills)

Two skills that use the `kisi` command line tool:

* **kisi-prepare** – finds what the analyses miss (`kisi gaps`), researches it from datasheets and
  writes it with sources to `<board>.kisi.json` (`kisi config-check` verifies the file).
* **kisi-si-pi-review** – SI/PI review: impedance per net class, channel checks against interface
  profiles, single nets, power (`kisi overview`, `interfaces`, `check`, `net`, `power`, `report`).

Setup:
1. Build the CLI and put it on the PATH: `cargo install --path crates/kisi-cli`
2. In Claude Code: `/plugin marketplace add <path or repo of kisi>` and `/plugin install kisi@kisi`

Every `kisi <command> <board.kicad_pcb>` prints JSON; `kisi help` lists the commands. Other agents
can use the same commands and the SKILL.md files as instructions.
