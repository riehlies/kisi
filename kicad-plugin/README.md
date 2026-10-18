# kisi – KiCad-Plugin

Öffnet den eigenständigen kisi-Viewer mit dem Board, das im KiCad-Leiterplatteneditor offen ist
(KiCad 9/10, IPC-API). Das Board wird dafür mit „Kopie speichern“ in einen temporären Ordner
kopiert – die eigenen Dateien werden weder gespeichert noch verändert. Ist eine Leiterbahn, ein Via
oder ein Pad ausgewählt, ist das Netz im Viewer vorausgewählt.

## Installation
1. `python3 tools/package_plugin.py` → `dist/kisi-kicad-plugin.zip`
2. Den Ordner `kisi` aus dem Zip in den Plugin-Ordner von KiCad entpacken:
   - Linux `~/.local/share/KiCad/9.0/plugins/`
   - Windows `%USERPROFILE%\Documents\KiCad\9.0\plugins\`
   - macOS `~/Documents/KiCad/9.0/plugins/`
3. In KiCad: Einstellungen → Plugins → „API-Server aktivieren“. Beim ersten Start legt KiCad die
   Python-Umgebung mit `kicad-python` an (requirements.txt).

Aktionen: „kisi: Viewer öffnen“ (Symbolleiste) und „kisi: Kanal prüfen“ (startet in der Kanal-Ansicht).

## Ohne KiCad
`python3 kisi_open.py --pcb board.kicad_pcb [--net NETZ] [--view si|ch|pi]`

## Aufbau
Der Viewer bleibt unabhängig: das Plugin ersetzt in einer Kopie von `kisi.html` nur die Zeile
`const PRELOAD = null; /*KISI_PRELOAD*/` durch das Board (gzip+base64) und öffnet die Datei im Browser.
