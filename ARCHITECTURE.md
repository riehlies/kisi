# kisi – Aufbau

Ziel: Teile einzeln aktualisieren können, ohne den Rest anzufassen. Jede Schicht hat eine
schmale Schnittstelle; Daten, die sich ändern (Normen, Bauteilwerte), liegen in Dateien statt im Code.

```
                 ┌─────────────── Front-Ends ────────────────┐
  KiCad-Plugin ─►│ Viewer (web/, eine HTML-Datei)  CLI  MCP  │◄─ Agenten (Claude Code, Konnect, …)
  (kicad-plugin) │   └─ WASM: crates/kisi-wasm   kisi-cli  kisi-mcp
                 └──────────────┬────────────────────────────┘
                                │  kisi-core (Fassade + service: Projekt laden, JSON-Zusammenfassungen)
              ┌─────────────────┼──────────────────┐
           kisi-si           kisi-pi           kisi-board
  Querschnitte, Formeln,   DC-Spannungsabfall,  Parser .kicad_pcb/.kicad_pro,
  Feldlöser-Geometrie,     Versorgungsbaum,     Lagenaufbau (+Änderungen),
  Leitungsmodelle, Kanal,  PDN-Impedanz         Geometrie, Szene, num (Cx, Konstanten)
  Vias, Auge, Übersprechen,       │
  Profile/Bewertung          kisi-sparse (faer) ── auch Gleichungslöser des Feldlösers
              │
  data/interfaces.json (Schnittstellen-Profile)     third_party/js_2d_fields (Forstén, unverändert)
                                                     └─ web/solver/backend_forsten.js (einziger Adapter)
```

## Crates (Rust, `crates/`)
| Crate | Inhalt | hängt ab von |
|---|---|---|
| `kisi-board` | Parser, Lagenaufbau, `StackupOverride`, Geometrie, Viewer-Szene, `num` | – |
| `kisi-si` | `extract`, `analytic`, `xsgeom`, `rf/{line,channel,crosstalk}`, `via`, `eye`, `profile` | kisi-board |
| `kisi-pi` | `pi` (DC), `tree` (Reglerketten, `PI_REG`), `pdn` (Z(f), Ebenenmodell) | kisi-board, kisi-sparse |
| `kisi-core` | Fassade mit stabilen Pfaden (`kisi_core::rf`, …) und `service` | die drei oben |
| `kisi-wasm` | C-ABI für den Browser | kisi-core |
| `kisi-cli` | `kisi`, `kisi-validate` | kisi-core |
| `kisi-mcp` | MCP-Server (stdio, JSON-RPC) | kisi-core |
| `kisi-sparse` | dünnbesetzter Löser, auch als WASM-Modul für den Feldlöser | faer |

Regel: Front-Ends benutzen nur `kisi-core`. Innerhalb von kisi-si/kisi-pi kann umgebaut werden,
solange die Fassade gleich bleibt. Die Bewertung eines Kanals (`kisi_si::profile::judge`) gibt es nur
einmal; Viewer (über WASM), CLI und MCP zeigen dasselbe Ergebnis.

## Viewer (`web/`)
* `index.html` – nur Markup; `css/kisi.css`; Skripte in `src/` in der Reihenfolge von
  `src/manifest.json` (gemeinsamer Gültigkeitsbereich):
  `engine` (WASM) · `state` · `load` · `geometry` · `draw` · `charts` · `net-detail` · `si-tabs`
  (Impedanz-Ansicht) · `channel` (Kanal-Ansicht) · `panels` · `interaction` · `field-pool` ·
  `field-cache` · `pi-dc` · `pi-pdn` · `pi-balance` · `views` · `main` (Start, Dateien, Übergabe).
* `tools/bundle.py` macht daraus eine eigenständige Datei `dist/kisi.html` (WASM, Feldlöser-Module,
  `data/*.json`, Beispielboards eingebettet).
* Übergabe von außen: `const PRELOAD = null; /*KISI_PRELOAD*/` wird vom KiCad-Plugin bzw. von
  `kisi_open_viewer` ersetzt – der Viewer selbst bleibt unabhängig.
* Im Browser gespeichert (je Betrachter): Feldlöser-Ergebnisse (IndexedDB), Lagenaufbau-Änderungen,
  Profil-Änderungen, Zuordnung Netzklasse → Profil, Versorgungseinstellungen.

## Daten (`data/`)
* `interfaces.json` – Schnittstellen-Profile mit Quellen (Norm / Richtlinie / kisi-Richtwert),
  Sender/Empfänger, Grenzen, Zuordnungsregeln. Wird in Viewer, CLI und MCP eingebaut; der MCP-Server
  kann mit `KISI_PROFILES` eine eigene Datei laden.
* Beispiel-Versorgungsbäume: `web/samples/*_tree.json`.

## Feldlöser
Siehe `UPSTREAM.md`: Forsténs js_2d_fields unverändert als Submodul, ein Adapter, Update per
`tools/upstream_update.sh` mit Prüfung gegen die Baseline (`tools/validate_field.sh`).

## Integration
* **KiCad** (`kicad-plugin/`): IPC-API-Aktion, kopiert das offene Board („Kopie speichern“) und öffnet
  den Viewer; `tools/package_plugin.py` baut das Zip.
* **Agenten** (`crates/kisi-mcp`, `claude-plugin/`): MCP-Server mit wenigen Werkzeugen, liest Board-
  Dateien vom Datenträger (Pfad z. B. von Konnect), schreibt auf Wunsch `analysis/kisi.json`
  (Ablage wie bei kicad-happy). Claude-Code-Plugin mit Skill für den Prüfablauf.

## Prüfen nach Änderungen
`cargo test --release` · `tools/validate_field.sh` (Feldlöser gegen HFSS/Baseline) ·
Headless-Tests der Ansichten mit Playwright (siehe `tools/shot*.py`).
