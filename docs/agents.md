# Using kisi with agents

Coding agents use kisi through its command line: `node kisi.mjs <command> <board>` prints JSON that states the
models and assumptions behind every number. Two skills describe the work for agents; Claude Code installs them as
a plugin, other agents can follow the same `SKILL.md` files.

## The skills
| Skill | What it does |
|---|---|
| [kisi-prepare](../claude-plugin/skills/kisi-prepare/SKILL.md) | runs `kisi gaps`, researches the missing values from datasheets (laminate εr/tanδ, impedance targets, interface standards, regulator chain, load currents, thermal data, assembly variants, connector ends), writes them with sources to `<board>.kisi.json` and checks the file with `kisi config-check` |
| [kisi-si-pi-review](../claude-plugin/skills/kisi-si-pi-review/SKILL.md) | the SI/PI review: impedance per net class, channel checks against interface profiles, single nets, crosstalk, power tree, IR drop, PDN impedance and the temperature estimate (`overview`, `interfaces`, `check`, `net`, `crosstalk`, `power`, `thermal`, `report`), reported with the models' limits |

A typical session: "prepare this board for kisi" (kisi-prepare), then "review its signal and power integrity"
(kisi-si-pi-review). The worked example of a prepared board – the Seeed XIAO nRF54LM20A – is in
[Preparing a board](preparing-a-board.md#worked-example-seeed-studio-xiao-nrf54lm20a).

## Any assistant: the agent guide
[KISI-AGENT-GUIDE.md](../kicad-plugin/agent/KISI-AGENT-GUIDE.md) is the same preparation as kisi-prepare, written for
any assistant – ChatGPT, Claude, Gemini, local models in LM Studio or Ollama, coding agents. Give it to the assistant
together with your request (attach or paste it). It works in two modes: an agent that can run commands calls
`kisi.mjs` itself and writes `<board>.kisi.json`; a chat assistant without a shell asks you for the output of
`kisi gaps` and the current kisi.json and returns the complete new kisi.json for you to save next to the board.
The KiCad plugin carries the guide (`agent/KISI-AGENT-GUIDE.md` in its folder); on the viewer's *Prepare PCB* page
the agent card has "Copy guide", and its prompt names the guide, the plugin's `kisi.mjs`, the board and where the
kisi.json goes ([Getting started](getting-started.md#from-kicad-the-plugin)). The guide is generated from the
kisi-prepare skill (`tools/agent_guide.py`), so both say the same.

## Setup
1. Node.js ≥ 18 ([nodejs.org](https://nodejs.org), LTS).
2. Claude Code: `/plugin marketplace add riehlies/kisi`, then `/plugin install kisi@kisi`.

The skills look for `kisi.mjs` in `$KISI_MJS`, then `~/.kisi/kisi.mjs`, and otherwise download the latest release
into `~/.kisi/`:
```sh
curl -fsSL --create-dirs -o ~/.kisi/kisi.mjs https://github.com/riehlies/kisi/releases/latest/download/kisi.mjs
# or
gh release download --repo riehlies/kisi --pattern kisi.mjs --dir ~/.kisi --clobber
```
(PowerShell: `Invoke-WebRequest <same URL> -OutFile $HOME\.kisi\kisi.mjs`.) A native `kisi` binary
(`cargo install --path crates/kisi-cli`) accepts the same commands, without the field solver.

## Notes for agents
- kisi reads the **saved** files; it does not talk to a running KiCad. Run it again after changes.
- Preparing needs no field solver: `gaps`, `config-check`, `config-template`, `interfaces` and `profiles` are
  fast. `overview`, `net`, `check`, `crosstalk` and `report` run the field solver; the first run on a board can
  take minutes (progress on stderr), later runs use the cache. `field_solver` in the output shows the coverage.
- Boards with assembly variants need one (`--variant` or `"variant"` in kisi.json); the lane commands stop with
  the list otherwise. Ask the user which one – never pick one silently.
- Every value written to kisi.json needs a source; estimates and assumptions are marked as such
  ([reference](reference.md#board-configuration-boardkisijson)).
- `kisi report` writes `analysis/kisi.json` next to the board, for file-based review tools (e.g. kicad-happy) and
  later sessions.
- With a tool that edits KiCad live (e.g. Konnect): change the copper, save, then run `kisi net` / `kisi check`
  again to confirm.

An MCP server existed until October 2026 and was removed in favour of the CLI and the skills (it remains in the git
history).
