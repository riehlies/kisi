# KiCad plugin: research and design

Design note for `kicad-plugin/` (2026-10-26). The user-facing description is in
[Getting started](../getting-started.md#from-kicad-the-plugin); the structure in `ARCHITECTURE.md`.

## Decisions (maintainer, 2026-10-26)
- One click in KiCad's PCB editor opens kisi's viewer in the browser with the open board and its
  `<board>.kisi.json`; the plugin creates that file (a minimal skeleton) when it does not exist.
- An IPC API plugin (KiCad 9/10, PCB editor only), not a SWIG action plugin (SWIG is removed in KiCad 11).
- No local server: the plugin writes a self-contained viewer page with the board embedded to a temporary folder and
  opens it (`file://`); the viewer ships inside the package (same version, offline).
- The plugin never writes KiCad's files and reads the saved board; unsaved changes are reported where KiCad can tell.
- Distribution through KiCad's Plugin and Content Manager (PCM) repository and as a zip ("Install from File…");
  identifier `com.github.riehlies.kisi`, GPL-3.0-only.

## Sources
The KiCad source was read on the GitHub mirror, branch 10.0 at `f4fb7776ff` and branch 9.0 at `ce707d0a6c`
([kicad-source-mirror](https://github.com/KiCad/kicad-source-mirror); gitlab.com was not reachable from the build
environment, so neither the metadata repository nor kicad-python's GitLab issues could be cloned).
- [S1] KiCad dev docs, IPC API for add-on developers: https://dev-docs.kicad.org/en/apis-and-binding/ipc-api/for-addon-developers/
  – plugin folders, `plugin.json` with the schema https://go.kicad.org/api/schemas/v1, Python runtime with a virtual
  environment per plugin in `${KICAD_CACHE_HOME}/python-environments/<identifier>`, "Recreate Plugin Environment",
  debugging (`KICAD_ALLOC_CONSOLE=1`, `KICAD_ENABLE_WXTRACE=1`, `WXTRACE=KICAD_API`, `EnableAPILogging=1` in
  `kicad_advanced` → `logs/api.log`), plugin output in the editor's messages from 10.0.1, KiCad 9/10: PCB editor only.
- [S2] KiCad dev docs, add-ons: https://dev-docs.kicad.org/en/addons/ – archive layout, metadata fields, schema
  v1/v2, licence rules (v1: Debian rules; v2: any string, the repository enforces open source; plugins
  GPL-compatible), identifier `com.github.<user>.<package>`, English metadata, submission as a merge request to
  gitlab.com/kicad/addons/metadata with `packages/<identifier>/metadata.json` (+ icon), download fields only there.
- [S3] `kicad/pcm/pcm_task_manager.cpp` (10.0), `PCM_TASK_MANAGER::extract`: every zip entry `<dir>/<rest>` with
  `<dir>` in `PCM_PACKAGE_DIRECTORIES` (`kicad/pcm/pcm.h`: plugins, footprints, 3dmodels, symbols, resources, colors,
  templates, scripts) goes to `<3RD_PARTY>/<dir>/<identifier with . → _>/<rest>`; root files and other folders are
  skipped. `InstallFromFile` reads `metadata.json` at the root, validates it (10.0: against the v2 schema; 9.0:
  v1) and needs exactly one version.
- [S4] `common/api/api_plugin_manager.cpp` (10.0): `ReloadPlugins` walks the stock plugin folder, the PCM folder
  (`3RD_PARTY`, default `Documents\KiCad\10.0\3rdparty` on Windows) and the user folder (`Documents\KiCad\10.0\plugins`)
  recursively for files named `plugin.json`. Python plugins: `python -m venv --system-site-packages`, `pip install
  --upgrade pip`, then `pip install --no-input --isolated --only-binary :all: --require-virtualenv -r
  requirements.txt` from the folder of `plugin.json` – a missing `requirements.txt` is an error and the plugin
  never becomes ready. Actions run as `<venv>\Scripts\pythonw.exe <entrypoint>` with `KICAD_API_SOCKET`,
  `KICAD_API_TOKEN`, cwd = the plugin folder, no arguments (`args` only for `exec` plugins); a non-zero exit code and
  stderr go to the editor's reporter (the alert triangle).
- [S5] `common/api/api_plugin.cpp` (10.0): schema check of `plugin.json`, `API_PLUGIN::IsValidIdentifier` (reverse
  DNS with at least three parts), entry point relative to `plugin.json`, icons loaded as PNG relative to it.
- [S6] `kicad/pcm/pcm.cpp` (10.0): a package version with `"runtime": "ipc"` makes the PCM ask "This plugin requires
  the KiCad API … enable it?" after installing (`ShowApiEnablePromptIfNeeded`); the API is off by default
  (`api.enable_server`, `common/settings/common_settings.cpp`), the switch is *Preferences → Plugins → Enable KiCad
  API* (`panel_plugin_settings_base.cpp`).
- [S7] `pcbnew/toolbars_pcb_editor.cpp` (10.0): the plugin buttons are the last group of the top toolbar (control
  `ipcScripting`, after "Switch to Schematic Editor" and the variant selector), shown when the API is enabled and
  the plugin is ready; *Preferences → PCB Editor → Plugins* lists the actions (`eda_base_frame.cpp`, right-click:
  "Recreate Plugin Environment", `panel_pcbnew_action_plugins.cpp`).
- [S8] `pcbnew/api/api_handler_pcb.cpp` (10.0): `GetOpenDocuments` returns the board's file name (no folder) and the
  project's name and folder; `SaveDocumentToString` returns the board as KiCad would save it. KiCad 10 has no call
  for the modified state; `GetDocumentModifiedState` exists on KiCad's development branch (`api/proto/common/commands/
  editor_commands.proto`, master, Oct 2026).
- [S9] kicad-python 0.8.0 (PyPI, built against KiCad 10.0.6; MIT): `KiCad(client_name=, timeout_ms=)`,
  `get_version()`, `get_board()` → `Board.document` (`board_filename`, `project.name`, `project.path`). Docs:
  https://docs.kicad.org/kicad-python-main/. Its dependencies have Windows wheels for CPython 3.11–3.14
  (`pip download --only-binary :all: --platform win_amd64`, checked 2026-10-26).
- [S10] OrthoRoute, an IPC plugin packaged for KiCad 10's PCM (https://github.com/bbenchoff/OrthoRoute at
  `2b2bc1cb83`, `docs/plugin_manager_integration.md`, `build.py`, `plugin.json`): zip = `metadata.json`,
  `resources/icon.png`, `plugins/` with `plugin.json`, `requirements.txt` and the code directly inside (no
  identifier folder), metadata v2 with one version and `"runtime": "ipc"`, no download fields in the zip.
- [S11] Open questions in KiCad's trackers: kicad-python issue 65 ("Add example packaging for use in Plugin and
  Content Manager", https://gitlab.com/kicad/code/kicad-python/-/issues/65) and issue 81 ("IPC plugins don't work
  with KiCad's PCM", KiCad 9.0.4: installed from file, no button; https://gitlab.com/kicad/code/kicad-python/-/issues/81,
  no answer visible); the forum thread https://forum.kicad.info/t/kicad-10-plugin-button/69728 (KiCad 10.0.3: "can't
  make the PCM read a zip file without crashing", no layout given; the reply points to the alert triangle and to
  `Documents/KiCad/10.0/plugins` for hand-installed plugins, `3rdparty` being the PCM's).

## Package layout
```
metadata.json               one version, "runtime": "ipc", no download fields   [S2, S3]
plugins/plugin.json         → 3rdparty/plugins/com_github_riehlies_kisi/plugin.json, found by [S4]
plugins/requirements.txt    kicad-python>=0.4.0,<1, next to plugin.json as [S4] requires
plugins/kisi_action.py      entry point; plugins/kisi_kicad.py (standard library only)
plugins/kisi_build.json     version, SHA-256 of the viewer
plugins/icons/kisi-{24,48}[-dark].png   toolbar (24 px; 48 px for high DPI) [S5]
plugins/viewer/kisi.html    the release's viewer with the hand-over marker
plugins/LICENSE             GPL-3.0
resources/icon.png          64×64 package icon [S2]
```
Why: [S3] and [S4] together fix it – only folders named in `PCM_PACKAGE_DIRECTORIES` are extracted, each below a
folder named after the identifier, and `plugin.json` is found anywhere below `3rdparty`, with `requirements.txt` and
all relative paths taken from its folder. OrthoRoute [S10] uses the same layout. The open reports [S11] do not show
their zips; the layout and the API switch are the likely causes, and the test plan checks both.
`kicad-plugin/tests/test_package.py` replays [S3]'s extraction on the built zip and checks what [S4] and [S5] need.

Metadata: `$schema` v2 (recommended for new packages [S2]); it also validates against KiCad 9's v1 schema, which
KiCad 9 uses for "Install from File" [S3]. Hence `"license": "GPL-3.0"`: v1 only accepts names from its list, and
under the Debian rules v1 follows [S2] `GPL-3.0` means version 3 only (`GPL-3.0+` would be "or later") – the same as
SPDX `GPL-3.0-only`, which v1 rejects. `kicad_version` 9.0: the code paths used are the same in 9.0 and 10.0 [S3,
S4, S5]; only 10 is tested. `tools/build_kicad_pcm.py` validates against the vendored schemas
(`kicad-plugin/tests/schemas/`).

## What the plugin does
1. KiCad starts `kisi_action.py` [S4]; it connects with kicad-python and calls `GetVersion` and
   `GetOpenDocuments` only (`get_board`) [S8, S9] – nothing that saves or changes the board.
2. Board path = project folder + board file name [S8]. A board never saved has no such file: the plugin says so.
3. Unsaved changes: KiCad 10 cannot report them [S8]. The plugin asks with `GetDocumentModifiedState` when the
   installed kicad-python has that message (KiCad 11) and otherwise says "unknown"; the viewer then notes that
   unsaved changes are not included. Comparing `SaveDocumentToString` with the file was considered and left out:
   the in-memory and on-disk formatting need not match byte for byte (a board from an older KiCad version), so it
   would report changes that are not there, and it transfers the whole board.
4. `kisi_kicad.py` reads `.kicad_pcb`, `.kicad_pro` (named after the board, else the project), `<board>.kisi.json`
   and the BOMs/Touchstone files it names; writes the skeleton `{"kisi_config": 1, "board": "<board>.kicad_pcb"}`
   (the viewer's own start, `config-check` ok) only when no file exists (exclusive create); a guard refuses any other
   write next to the board.
5. The files go gzip + base64 into the payload that replaces `null; /*@kicad-board*/` in `viewer/kisi.html`
   (`<` escaped); the page goes to `analysis/<board>.kisi.html` next to the board (round 2; before:
   `<temp>/kisi-kicad/<board>-<hash of the path>.html`, still the fallback) and opens with `webbrowser` (Windows:
   `os.startfile` as fallback).
6. In the viewer, `loadKicadBoard` turns the payload into `File` objects and calls `handleFiles` – the drop path.
   The "From KiCad" card names board, folder, the kisi.json's state and the unsaved-changes note.
7. Prepare PCB → "Save kisi.json": Chromium/Edge `showSaveFilePicker` with `suggestedName` `<board>.kisi.json` and
   `id` `kisi-<hash of the board folder>` (the browser remembers the folder per id; a page cannot start the dialog
   in a folder given as a path) and no `startIn` (MDN does not say whether a well-known `startIn` would override the
   remembered folder), the path shown with "Copy path" (Windows' save dialog takes a full path as file
   name); Firefox/Safari download, with the note to move the file next to the board.
8. Problems: one line on stderr (shown at the alert triangle from KiCad 10.0.1 [S1]), exit code 1, an error page
   in the browser, details in `<temp>/kisi-kicad/kisi-kicad.log`.

## Round 2 (2026-10-27, after the first test in KiCad 10 on Windows)
The first test passed in principle (install, button, viewer with the board, skeleton). The maintainer's decisions
for round 2 – **no local server** (a security risk) – and how they are built:
- **The page into `analysis/`:** `analysis/<board>.kisi.html` next to the board (the folder `kisi report` and
  `kisi export` use), written anew on each click; the temporary folder when `analysis/` cannot be written, said in
  the card. `Guard` allows exactly `<board>.kisi.json`, `analysis/<board>.kisi.html` and the backups
  `analysis/<board>.kisi.json.<YYYYmmdd-HHMMSS>[-n].bak` next to the board, and the plugin's temp folder.
- **Prepare PCB marks what a step needs:** before, `prepSrcCell` marked the source of every entry without a source –
  also entries without a value – and nothing marked the value inputs. Now an open step marks its empty value inputs
  (`prep-need`; per step type: εr/tanδ of the flagged layers, target Ω, profile, variant choice, the regulator
  chain's new entry and suggestions, currents per rail, rated V and dielectric, bead Z/R, θJB/θJC) and focuses the
  first; the source is marked once the entry has a value. `check_viewer.py` (`prepare_marks`) checks every step type
  on the bare example boards and ThunderScope with its kisi.json (thermal data).
- **The CLI in the package:** `plugins/cli/kisi.mjs` of the same version; `find_node` runs `node --version` (5 s,
  no console window; PATH, then `%ProgramFiles%\nodejs`, `%LOCALAPPDATA%\Programs\nodejs`, `NVM_SYMLINK`) and the
  page gets the CLI path and the Node.js status; the agent card shows `node "<plugin>\cli\kisi.mjs" gaps "<board>"
  --for <tab>` and the copied prompt names the CLI, the board and where its kisi.json goes (the agent writes it
  directly). The skill kisi-prepare names the plugin's folder as a place to find `kisi.mjs`.
- **Saving without File System Access:** (A) the save step and `docs/getting-started.md` give the browsers' "ask where
  to save" settings (Firefox: Settings → General → Downloads → "Always ask you where to save files"; Safari: Settings
  → General → File download location → "Ask for each download"). (B) The next click takes a downloaded
  `<board>.kisi.json` over (`import_download`): the Downloads folder from `SHGetKnownFolderPath(FOLDERID_Downloads)`
  ([S12]), else `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders` value
  `{374DE290-123F-4565-9164-39C4925E467B}` (REG_EXPAND_SZ), else `%USERPROFILE%\Downloads`; on Linux
  `xdg-user-dir DOWNLOAD`, else `user-dirs.dirs` ([S13]), else `~/Downloads`; macOS `~/Downloads`. Candidates:
  `<board>.kisi.json` and the browsers' duplicate names `<board>.kisi(1).json` (Firefox), `<board>.kisi (1).json`
  (Chrome, Edge), `<board>.kisi-1.json`; only those newer than the board's kisi.json (1 s slack). Exactly one, valid
  JSON with `kisi_config` 1 and `board` naming this board, and `kisi config-check` ok when Node.js and the CLI are
  there (a check that cannot run is reported, the format check stands); then the old file is copied to `analysis/`
  (exclusive create), the download's bytes replace the kisi.json (atomic), the download is removed. Several
  candidates, another board, invalid content or a failed backup: nothing changes, the card says why.
- [S12] FOLDERID_Downloads = `{374DE290-123F-4565-9164-39C4925E467B}` (KNOWNFOLDERID; e.g. Free Pascal's WinDirs
  reference, https://www.freepascal.org/docs-html/3.2.0/rtl/windirs/folderid_downloads.html).
- [S13] xdg-user-dirs (`xdg-user-dir`, `~/.config/user-dirs.dirs`), freedesktop.org.

## Checked here, and what only KiCad can show
Checked without KiCad: the unit tests (paths with spaces and umlauts, Windows path rules with `ntpath`, skeleton,
existing and invalid kisi.json, read-only folder, the KiCad files' hashes unchanged, error paths), the action with
kicad-python 0.8.0's own classes over a fake connection answering like [S8], the requirements installed with
KiCad's pip options [S4], the package against the schemas and [S3]'s extraction, the extracted plugin's page in
headless Chromium (`tools/check_kicad_plugin.py`: start, card, Prepare save via a stubbed File System Access dialog,
the download path, the second click, a full kisi.json, the unsaved-changes notice). Only KiCad can show: that the
PCM installs the zip and offers to enable the API, that the environment is built and the button appears, the
icons, the real `KICAD_API_SOCKET` connection, the browser start from `pythonw.exe`, the real save dialog and its
remembered folder (Chromium's handling of `id` is not documented in detail). The maintainer's test plan
covers these steps.
