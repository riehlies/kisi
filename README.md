# kisi – KiCad Signal-Integrity-Werkzeug

Extrahiert Leitungsimpedanzen aus KiCad-Layouts (`.kicad_pcb` + `.kicad_pro`) und stellt sie farbig dar.
Kern in Rust, läuft als WebAssembly im Browser und als native CLI. Lizenz: GPL-3.0-only.

## Ablauf im Viewer
1. Board laden (`.kicad_pcb`, optional `.kicad_pro`, optional `kipida_config.json`)
2. Schnelle Analyse läuft sofort: Impedanz per Formel, DC-Spannungsabfall aller konfigurierten Schienen
3. Feldlöser per Button für ausgewähltes Netz, eine Netzklasse, alle Netze mit Ziel oder alles.
   Ergebnisse werden im Browser (IndexedDB) gespeichert, boardübergreifend, Schlüssel = Solver-Version + Geometrie.

## DC-Spannungsabfall (Versorgung)
- Kupfer eines Netzes wird je Lage gerastert (Zonen, Leiterbahnen, Pads), Vias und Durchsteck-Pads verbinden die Lagen
  (Hülsenwiderstand mit 25 µm Kupfer), Quell-Pads liegen auf Nennspannung, Last-Pads ziehen ihren Strom.
- Ergebnis: Spannung und Stromdichte je Lage, Spannung an jeder Last, Ströme der Quellen und Vias, Verlustleistung.
- Lasten in KiCad: Feld am Bauteil (im Schaltplan anlegen, wird beim Aktualisieren in die Platine übernommen)
  - `PI_LOAD` = `+1V0=2.5A, +1V8=300mA` oder `+1V0=2W` (Leistung → Strom über die Nennspannung)
  - `PI_SOURCE` = `+3V3` oder `+3V3=3.3V` für Regler-Ausgänge und Einspeisungen
- Nennspannung sonst aus dem Netznamen (`+3V3`, `1V8`, `-5V`, `+1.2V`)
- KiPIDA: `kipida_config.json` wird gelesen, Regler werden als Last auf ihrer Eingangsschiene übernommen
- CLI: `kisi board.kicad_pcb --pi` (Felder), `--pi-config kipida_config.json` oder eigene JSON-Liste, `--rails` listet Versorgungsnetze
- Validierung: Kupferstreifen gegen R = ρL/(wt), Abweichung ≤ 2,5 % (Test `strip_resistance`)

## Versorgungsbaum und Leistungsbilanz (Beispielboards)
- `web/samples/ts_tree.json`, `web/samples/vna_tree.json`: Reglerketten von ThunderScope und LibreVNA, aus der Netzliste
  rekonstruiert. Elemente: `input`, `buck`, `boost`, `ldo` (Querstrom `iq`), `inverter`, `pass` (Shunt/Ferrit mit `r`);
  Lasten je Bauteil und Netz mit Strom und Grundlage (`Datenblatt` oder `Schätzung`).
- `crates/kisi-core/src/tree.rs`: rechnet von den Lasten zu den Eingängen, liefert Ströme, Verluste je Regler,
  Leistung je Bauteil (Bilanz exakt: Eingang = Verbrauch + Verluste) und die Quellen/Lasten jeder Schiene für den Spannungsabfall.
- CLI: `kisi board.kicad_pcb --pi-tree web/samples/ts_tree.json`
- Viewer: Versorgung → Leistungsbilanz, Ströme dort editierbar (im Browser gespeichert).

## HF: Kanal, Verluste, TDR, Übersprechen (M3) und Vias (M4)
- `crates/kisi-core/src/rf.rs`: frequenzabhängiges RLGC je Querschnitt, aus dem Feldlöser (R, L, G, C bei 1 GHz)
  oder aus Formeln. Skin-Effekt nach Wheeler (inkrementelle Induktivität), Rauheit nach Huray (glatt / HVLP / Standard-ED,
  Voreinstellungen geschätzt), Dielektrikum nach Djordjevic-Sarkar aus εr und tanδ des Lagenaufbaus, Microstrip-Dispersion.
- Kanal eines Netzes: längster Pfad durch den Leiterbahn-Graphen (Lagen nur über Vias / Durchsteck-Pads verbunden),
  ABCD-Kaskade → S21/S11 (bei Paaren ungerade Mode, Sdd), Touchstone `.s2p`, TDR aus S11 mit Gauß-Flanke.
- `crates/kisi-core/src/via.rs`: Via als Stapel koaxialer Abschnitte je Lage; Außenleiter = in den Zonenfüllungen
  gemessene Freistellung (Antipad), bei Paaren Wheeler-Formel für ein geschirmtes symmetrisches Paar. Stubs als offene
  Leitungen (Viertelwellen-Resonanz), Backdrill aus KiCad 10 (`backdrill`) oder global. Wechsel der Bezugsebene:
  Induktivität bis zum nächsten Stitching-Via, Warnung wenn keins in 5 mm oder Bezug auf ein anderes Netz.
- Übersprechen: Nachbarleitungen auf derselben Lage (parallel, bis 5 h Abstand), NEXT/FEXT aus der L/C-Kopplungsmatrix
  des Feldlösers oder Kirschning-Jansen/Cohn; Feldlöser-Umfang „ausgewähltes Netz“ rechnet die Kopplungsquerschnitte mit.
- CLI: `kisi board.kicad_pcb board.kicad_pro --rf <Netzende> [--s2p out_{}.s2p]`
- Viewer: Netz wählen → Tabs Impedanz / Verluste / TDR / Vias / Übersprechen, Karte „HF-Einstellungen“.
- Augendiagramm (M5, `crates/kisi-core/src/eye.rs`): PRBS7 NRZ über die Einzelbit-Antwort aus S21, Sender-Flanke =
  Anstiegszeit, ohne Entzerrung, Jitter und Rauschen. Datenrate aus der Netzklasse geschätzt oder fest gewählt.
  CLI: `--rf <Netz> --eye 16` (Gbit/s).
- Kanalprüfung: alle Netze mit Zielimpedanz (je Paar eine Zeile) mit Dämpfung bei Nyquist, Rückflussdämpfung,
  Augenhöhe/-breite, Skew im Paar und Via-Stub-Resonanz; Ampel nach Richtwerten (keine Spezifikation).
- Genauigkeit: Leitungs-RLGC gegen HFSS ≤ 2 %; Via-Impedanz ist ein Abschätzungsmodell (etwa ±15 %), kein 3D-Ergebnis.

## Impedanz der Versorgung Z(f) (PI-AC, erste Version)
- `crates/kisi-core/src/pdn.rs`: konzentriertes Modell je Schiene – Regler (R + L) ∥ Ebenenkapazität (Fläche der
  Zonen über der nächsten Masselage) ∥ alle Kondensatoren zwischen Schiene und Masse. Wert aus dem Bauteilwert
  (`100n`, `4u7`, `10µF/6.3V`), Bauform aus dem Padabstand, ESR/ESL geschätzt, Montage-Induktivität über die Vias bis
  zur Ebene, Ausbreitungsinduktivität bis zum Lastschwerpunkt.
- Zielimpedanz = 3 % der Spannung / (50 % des Laststroms); bewertet bis 100 MHz bzw. bis zur ersten Hohlraumresonanz.
- CLI: `kisi board.kicad_pcb --pi-tree web/samples/ts_tree.json --pdn +1V0`; Viewer: Versorgung → Schiene → „Impedanz Z(f)“.

## Stand: Meilenstein M2 (Feldlöser)
- Quasi-statischer 2D-Feldlöser: js_2d_fields von Henrik Forstén, unverändert eingebunden (siehe `UPSTREAM.md`)
- Jeder Querschnitt wird als Geometrie-Text erzeugt (`crates/kisi-core/src/xsgeom.rs`) und vom Backend gerechnet
- Gleichungslöser in Rust (`crates/kisi-sparse`, faer) ersetzt das nicht veröffentlichte Eigen/emscripten-Modul
- Viewer rechnet im Hintergrund mit mehreren Threads, wichtigste Querschnitte zuerst; Formelwerte als Startwert
- Validierung: ≤ 1,5 % gegen HFSS/Ansys-Fälle, ≤ 0,3 % gegen Forsténs Microstrip-/Stripline-Löser an ThunderScope-Querschnitten

## Meilenstein M1
- Parser für KiCad 9/10 (Stackup inkl. Teillagen, Leiterbahnen, Bögen, Vias, Pads, Zonenfüllungen, Netzklassen)
- Querschnitts-Erkennung entlang jeder Leitung: Referenzebenen (auch über Lücken/Freistellungen), koplanare Masse, Diff-Partner
- Analytische Impedanz: Hammerstad-Jensen, Wheeler, Wadell-CPWG, IPC-2141 (Diff)
- Zielimpedanz aus Netzklassennamen (`50Z`, `100Z_Diff`, …) bzw. Schnittstellen-Annahmen, im Viewer editierbar
- Viewer: Abweichung vom Ziel / Z₀ absolut / Leitungstyp, Tooltip, Netzprofil, Lagenaufbau

## Bauen
```sh
git submodule update --init                                              # js_2d_fields
cargo build --release -p kisi-cli                                        # CLI: target/release/kisi
cargo build --release -p kisi-wasm -p kisi-sparse --target wasm32-unknown-unknown
python3 tools/bundle.py web/index.html target/wasm32-unknown-unknown/release/kisi_wasm.wasm \
        target/wasm32-unknown-unknown/release/kisi_sparse.wasm dist/kisi.html \
        ts "ThunderScope Rev5.3" <ts.kicad_pcb> <ts.kicad_pro> vna LibreVNA <vna.kicad_pcb> <vna.kicad_pro>
```
Beispielboards werden vorher mit `tools/strip_pcb.py` verkleinert.

Feldlöser auf der Kommandozeile (braucht Node.js): `tools/kisi_field.sh board.kicad_pcb`

Solver-Update: `tools/upstream_update.sh` (Details in `UPSTREAM.md`), Prüfung allein: `tools/validate_field.sh`.

## Validierung
`validation/*.json` enthält Querschnitte, `*.ref.json` die Referenz aus Henrik Forsténs 2D-Feldlöser
(github.com/Ttl/js_2d_fields, GPL-3.0; in Node mit `tools/forsten_sparse_shim.js` als Ersatz für das WASM-Gleichungslöser-Modul).
```sh
target/release/kisi-validate validation/forsten_suite.json        # HFSS/Ansys-Referenzen aus Forsténs Testsuite
target/release/kisi-validate validation/thunderscope_xs.json validation/thunderscope_xs.ref.json
```
`forsten_selfcheck*.json`: Forsténs Löser in dieser Umgebung gegen dieselben HFSS-Fälle (Abweichung ≤ 1,4 %),
damit ist er als Referenz für die ThunderScope-Querschnitte abgesichert.

## Beispielboards
ThunderScope (EEVengers, MIT), LibreVNA (Jan Käberich, GPL-3.0).
