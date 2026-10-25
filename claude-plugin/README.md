# kisi – agent skills (Claude Code plugin)

Two skills that use the kisi command line `kisi.mjs` (Node.js ≥ 18):

* **kisi-prepare** – finds what the analyses miss (`kisi gaps`), researches it from datasheets and
  writes it with sources to `<board>.kisi.json` (`kisi config-check` verifies the file).
* **kisi-si-pi-review** – SI/PI review: impedance, channels, crosstalk, power, temperature.

Install in Claude Code: `/plugin marketplace add riehlies/kisi`, then `/plugin install kisi@kisi`. The skills
download `kisi.mjs` from the latest release on first use. Setup, how the skills find kisi and notes for other
agents: [docs/agents.md](../docs/agents.md).
