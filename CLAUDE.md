# kisi – Hinweise für die Arbeit am Repository

* **Gemeinsamer Stand ist dieses GitHub-Repository** (`riehlies/kisi`, Branch `main`).
  Der Ordner `D:\Git\kisi` auf dem Rechner des Nutzers ist nur ein lokaler Testordner für die
  Integration mit KiCad (Plugin) und dem MCP-Server – dort nichts als „Quelle“ betrachten.
* Aufbau und Update-Regeln: `ARCHITECTURE.md`. Feldlöser-Upstream: `UPSTREAM.md`.
* Texte in der Oberfläche und Antworten an den Nutzer auf Deutsch; Code-Kommentare auf Englisch.

## Bauen
```
cargo build --release                                   # CLI (kisi, kisi-validate) und kisi-mcp
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
python3 tools/bundle.py web/index.html \
  target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
  target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.html \
  [ts "ThunderScope Rev5.3" ts.kicad_pcb ts.kicad_pro vna "LibreVNA" vna.kicad_pcb vna.kicad_pro]
python3 tools/package_plugin.py                         # dist/kisi-kicad-plugin.zip
```
Die Beispielboards (ThunderScope, LibreVNA) liegen nicht im Repository; sie stammen aus den
jeweiligen Projekten und werden mit `tools/strip_pcb.py` verkleinert.

## Prüfen vor jedem Commit
* `cargo test --release`
* `sh tools/validate_field.sh` (Feldlöser gegen HFSS/Baseline, braucht node)
* Viewer: gebündelte Datei headless in Chromium öffnen (Playwright), alle drei Ansichten,
  hell/dunkel/mobil, keine Konsolenfehler.

## Regeln
* Eine Bewertungslogik (`kisi_si::profile::judge`) für Viewer, CLI und MCP – nicht duplizieren.
* Veränderliche Werte (Normen, Bauteil-Parasitics) gehören in `data/*.json`, mit Quelle.
* Forsténs Löser in `third_party/js_2d_fields` nie ändern; nur `web/solver/backend_forsten.js` anpassen.
* Geschätzte Werte im UI und in MCP-Ausgaben als Schätzung kennzeichnen.

## Commits
* Autor: `riehlies <118917105+riehlies@users.noreply.github.com>` (in `.git/config` gesetzt).
* Keine `Co-Authored-By`-, `Claude-Session`- oder ähnlichen Zeilen in Commit-Nachrichten oder PRs.
