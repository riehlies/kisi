# KiCad plugin: research and design

Design note for `kicad-plugin/` (2026-10-26). The user-facing description is in
[Getting started](../getting-started.md#from-kicad-the-plugin); the structure in `ARCHITECTURE.md`.

## Status (2026-10-30)
- Released in 0.3.1 (`kisi-kicad-0.3.1.zip`, release asset). The entry for KiCad's PCM repository (package
  `com.github.riehlies.kisi`, status `testing`, `kicad_version` 10.0) is a merge request to
  gitlab.com/kicad/addons/metadata, under review.
- Tested in KiCad 10 on Windows (from 2026-10-27: install, API prompt, button, viewer, skeleton, take-over of a
  download, CLI, agent card, paths with umlauts).
- Open: a test in KiCad 10 on Linux and on macOS ([checklist](#linux-and-macos-checklist)), then status `stable`;
  KiCad 9 (same code paths [S3–S5], untested); the unsaved-changes notice once KiCad's API reports it (step 3
  below).

## Decisions (maintainer, 2026-10-26)
- One click in KiCad's PCB editor opens kisi's viewer in the browser with the open board and its
  `<board>.kisi.json`; the plugin creates that file (a minimal skeleton) when it does not exist.
- An IPC API plugin (KiCad 9/10, PCB editor only), not a SWIG action plugin (SWIG is removed in KiCad 11).
- No local server: the plugin writes a self-contained viewer page with the board embedded and opens it (`file://`;
  first in a temporary folder, since the first test in `analysis/` next to the board); the viewer ships inside the
  package (same version, offline).
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
SPDX `GPL-3.0-only`, which v1 rejects. `kicad_version` 10.0 and status `testing` (maintainer's decision for the first
release: only KiCad 10 is tested; the code paths are the same in 9.0 [S3, S4, S5], so KiCad 9 can follow once it is
tested). `tools/build_kicad_pcm.py` validates against the vendored schemas
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
   (`<` escaped); the page goes to `analysis/<board>.kisi.html` next to the board – or, when `analysis/` cannot be
   written, to `<board>-<hash of the path>.html` in the plugin's own folder (`temp_dir`, step 8) – and opens in the
   default browser: Windows `os.startfile(<path>)`, macOS `open`, Linux `xdg-open`, `webbrowser` as the fallback.
6. In the viewer, `loadKicadBoard` turns the payload into `File` objects and calls `handleFiles` – the drop path.
   The "From KiCad" card names board, folder, the kisi.json's state and the unsaved-changes note.
7. Prepare PCB → "Save kisi.json": Chromium/Edge `showSaveFilePicker` with `suggestedName` `<board>.kisi.json` and
   `id` `kisi-<hash of the board folder>` (the browser remembers the folder per id; a page cannot start the dialog
   in a folder given as a path) and no `startIn` (MDN does not say whether a well-known `startIn` would override the
   remembered folder), the path shown with "Copy path" (Windows' save dialog takes a full path as file
   name); Firefox/Safari download, with the note to move the file next to the board.
8. Problems: one line on stderr (shown at the alert triangle from KiCad 10.0.1 [S1]), exit code 1, an error page
   in the browser, details in `kisi-kicad.log` in the plugin's own folder: `%TEMP%\kisi-kicad\` on Windows,
   `$TMPDIR/kisi-kicad/` on macOS (both per user), `~/.cache/kisi-kicad/` (`$XDG_CACHE_HOME`) on Linux.

## After the first test (2026-10-27, KiCad 10 on Windows)
The first test passed in principle (install, button, viewer with the board, skeleton). The maintainer's decisions
after it – **no local server** (a security risk) – and how they are built:
- **The page into `analysis/`:** `analysis/<board>.kisi.html` next to the board (the folder `kisi report` and
  `kisi export` use), written anew on each click; the plugin's own folder when `analysis/` cannot be written, said
  in the card. `Guard` allows exactly `<board>.kisi.json`, `analysis/<board>.kisi.html` and the backups
  `analysis/<board>.kisi.json.<YYYYmmdd-HHMMSS>[-n].bak` next to the board, and the plugin's own folder.
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

## After the second test (2026-10-27)
Re-test: the Downloads folder was found, the page in `analysis/` works, the marks help, the agent prompt worked with
ChatGPT. Changed:
- **Opening the page:** with the page in `analysis/` of a folder with umlauts (`kisi Test äöü`), `webbrowser.open`
  of the percent-encoded `file://` URL (Windows: `os.startfile(url)`) opened Edge's start page instead of the page.
  The plugin now passes the plain path to `os.startfile` (the `.html` association, i.e. the default browser);
  `open` on macOS, `xdg-open` on Linux, `webbrowser` as the fallback; the log names what was used.
- **Saving in Chromium:** the save dialog started in Downloads. Now the board's folder is chosen once per board
  (`showDirectoryPicker`, `startIn: "documents"`, an `id` per board), checked (its name is the board folder's and it
  holds `<board>.kicad_pcb`), its handle stored in IndexedDB per board path; "Save kisi.json" then asks for write
  permission once per session (`queryPermission`/`requestPermission`) and writes the file straight into the
  folder. Refused permission, a wrong folder or an error: the save dialog, starting in that folder. On `file://` in
  headless Chromium IndexedDB works and both pickers exist; the origin-private file system is refused, so the test
  uses a stand-in handle – whether Chromium stores a real handle of a `file://` page across sessions only the manual
  test shows (otherwise the card says "for this page only").
- **Saving in Firefox/Safari:** the download plus the take-over on the next click is the intended way; after saving
  the page says so in bold ("Saved to the downloads folder – click kisi in KiCad again …"), with the "ask where to
  save" setting as the alternative.
- **The agent guide for any assistant:** `kicad-plugin/agent/KISI-AGENT-GUIDE.md`, generated by
  `tools/agent_guide.py` from `KISI-AGENT-GUIDE.in.md` (how to run kisi; mode A with a shell, mode B in a chat
  without one – the user pastes `kisi gaps` and the kisi.json, the assistant returns the complete new kisi.json)
  and the kisi-prepare skill's steps and notes (blocks marked `agent-guide:steps`/`agent-guide:notes`, verbatim) – one
  source, checked in CI and by the package build, which also refuses assistant-specific words in the guide. The
  package ships it in `plugins/agent/`; the agent card shows its path and "Copy guide", and the prompt refers to it
  for both modes.

## After the release (2026-10-30)
- **Windows: the browser in front.** From the second click on, the new tab opened behind KiCad: a running browser
  may raise its window only when the process that asks may. The plugin – started by KiCad, which is in the
  foreground – hands that right on with `AllowSetForegroundWindow(ASFW_ANY)` before `os.startfile` (released in
  0.3.1).
- **`install_size`** in the PCM entry counts every file of the archive, `metadata.json` included, as the metadata
  repository's validator does (the 0.3.1 entry was 1636 bytes short; `tools/build_kicad_pcm.py`).
- **Linux and macOS, reviewed without KiCad** (none of these paths had run in KiCad yet):
  - The plugin's own folder (log, error page, fallback page) was `kisi-kicad` in the system's temporary folder. On
    Linux that is the shared `/tmp`: the folder could belong to another user (then neither log nor page could be
    written) and the pages, which hold the board, were readable by everyone; a Flatpak KiCad's `/tmp` is not
    visible outside its sandbox, so the log could not be found. Now `$XDG_CACHE_HOME/kisi-kicad`
    (`~/.cache/kisi-kicad`; a Flatpak's points into `~/.var/app/<id>/cache`), created readable only by the user;
    Windows and macOS keep their per-user temporary folder.
  - `open`/`xdg-open` were taken as success as soon as they started. They exit non-zero when no application takes
    the file (xdg-open 3: no tool, 4: failed); the plugin now waits up to 3 s and then falls back to `webbrowser`;
    an opener still running after that (xdg-open without a desktop environment runs the browser itself) counts as
    opened.
  - Node.js: KiCad started from the desktop (macOS: Finder or Dock) gets a short PATH, and a Flatpak KiCad sees
    the home folder but not the host's `/usr`. Besides the PATH, `/usr/local/bin`, `/opt/homebrew/bin`, `/usr/bin`,
    MacPorts' `/opt/local/bin` and `/snap/bin`, the plugin now looks for the version managers' Node.js in the home
    folder: Volta (`~/.volta/bin`), nvm (`$NVM_DIR/versions/node`, newest first), fnm's default alias, asdf's shim,
    `~/.local/bin`.
  - Downloads folder: a Flatpak KiCad sets `XDG_CONFIG_HOME` inside its sandbox folder, so `user-dirs.dirs` was not
    found when `xdg-user-dir` is missing; now `~/.config/user-dirs.dirs` is read too, with xdg-user-dirs' quoting
    and its "disabled" value (the home folder itself).
  - Names in two Unicode forms: macOS may list a file name decomposed (NFD), while the browser's download name and
    the kisi.json's `board` are composed (NFC) – a board named with umlauts would never have its download taken
    over. Names are now compared in NFC.

## Checked here, and what only KiCad can show
Checked without KiCad: the unit tests (paths with spaces and umlauts, Windows path rules with `ntpath`, skeleton,
existing and invalid kisi.json, read-only folder, the KiCad files' hashes unchanged, error paths), the action with
kicad-python 0.8.0's own classes over a fake connection answering like [S8], the requirements installed with
KiCad's pip options [S4], the package against the schemas and [S3]'s extraction, the extracted plugin's page in
headless Chromium (`tools/check_kicad_plugin.py`: start, card, Prepare save via a stubbed File System Access dialog,
the download path, the second click, a full kisi.json, the unsaved-changes notice). Only KiCad can show: that the
PCM installs the zip and offers to enable the API, that the environment is built and the button appears, the
icons, the real `KICAD_API_SOCKET` connection, the browser start from `pythonw.exe`, the real save dialog and its
remembered folder (Chromium's handling of `id` is not documented in detail).

A KiCad 10 run under Xvfb in the build container was tried on 2026-10-30 and is not possible there: the container
has KiCad 7 (no IPC API), and KiCad's PPA, Flathub, Docker Hub and gitlab.com are not reachable from it; building
KiCad 10 from source on two cores is out of proportion. Linux and macOS therefore wait for the checklist below.

## Linux and macOS checklist
For a test in KiCad 10 (10.0.1 or later, so that the plugin's messages show) on each system; the log is
`~/.cache/kisi-kicad/kisi-kicad.log` on Linux (Flatpak: `~/.var/app/org.kicad.KiCad/cache/kisi-kicad/`),
`$TMPDIR/kisi-kicad/kisi-kicad.log` on macOS (`open "$TMPDIR/kisi-kicad"`). Note the KiCad build (distribution
package, PPA, Flatpak, AppImage; macOS: Apple silicon or Intel), the browser and whether Node.js is installed and
how (package, Homebrew, nvm, Volta).
1. **Install** the release zip with *Install from File…*; KiCad offers to enable the API (else *Preferences →
   Plugins → Enable KiCad API*). The files are in `3rdparty/plugins/com_github_riehlies_kisi/` (Linux:
   `~/.local/share/kicad/10.0/3rdparty/`, Flatpak: `~/.var/app/org.kicad.KiCad/data/kicad/10.0/3rdparty/`, macOS:
   `~/Documents/KiCad/10.0/3rdparty/`).
2. **Environment:** the button appears after the first start (KiCad builds a venv and installs kicad-python; a
   missing `python3-venv` on Debian/Ubuntu or no network shows at the alert triangle). *Preferences → PCB Editor →
   Plugins* lists the action; "Recreate Plugin Environment" works.
3. **Button and icon:** the eye icon at the right end of the top toolbar, light and dark theme, high-DPI screen
   (48 px icon).
4. **First click** on a saved board in a folder with spaces and umlauts (`~/KiCad Tests/Prüf äöü/`): the default
   browser opens `analysis/<board>.kisi.html` in front, the card names the board; `<board>.kisi.json` is created;
   the log names the opener (`xdg-open` / `open`) and the Node.js found.
5. **Second click** while the browser is open: a new tab, the browser comes to the front (Wayland and X11 on Linux).
6. **Save in Chromium/Chrome:** *Choose the board folder…*, Save kisi.json writes next to the board; after a restart
   of the browser the folder is still known (or the card says "for this page only").
7. **Save in Firefox (and Safari on macOS):** the download, then a click in KiCad takes it over (the backup in
   `analysis/`); also with a board name with umlauts (macOS: NFD names) and with a localized Downloads folder
   (Linux: e.g. `~/Téléchargements`, from `xdg-user-dir`).
8. **Node.js:** the agent card shows the command with the installed `cli/kisi.mjs`; copy it into a terminal and run
   it. With Node.js only from nvm or Volta (not on KiCad's PATH) the plugin still finds it (log). Flatpak: Node.js
   from the host's `/usr` is invisible to the sandbox – the card says Node.js is missing, which is expected.
9. **Errors:** a never-saved board → the message at the alert triangle and an error page; the log has the details.
10. **Read-only board folder:** the page goes to the plugin's own folder and the card says so.
11. **Flatpak and AppImage:** the browser starts (Flatpak through the desktop portal); an AppImage's library path
    must not break the browser or `node` – if it does, the log shows the opener's exit code.
Then set `status` to `stable` in `kicad-plugin/metadata.json` (and the PCM entry) for the systems that passed.
