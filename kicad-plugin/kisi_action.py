#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""kisi's KiCad action (plugin.json): open the board of the PCB editor in kisi's viewer.

KiCad 9/10 start this file with the plugin's own Python environment (requirements.txt: kicad-python) and pass
KICAD_API_SOCKET and KICAD_API_TOKEN. It asks KiCad which board is open (file name, project folder, version and –
where KiCad can tell – unsaved changes) and hands that to kisi_kicad.py, which builds the viewer page; the page is
opened in the default browser. Nothing is saved in KiCad and no KiCad file is written.

Without KiCad (to test the rest): python kisi_action.py --pcb path/to/board.kicad_pcb [--no-browser]

The page goes to analysis/<board>.kisi.html next to the board (or the temporary folder when that cannot be written);
a kisi.json the viewer downloaded is taken over from the Downloads folder first (kisi_kicad.import_download).
Problems: one line on stderr (KiCad 10.0.1 and later show it at the alert triangle in the PCB editor's status bar),
an error page in the browser, and the details in kisi-kicad.log in the plugin's folder (kisi_kicad.temp_dir:
%TEMP%\\kisi-kicad on Windows, $TMPDIR/kisi-kicad on macOS, ~/.cache/kisi-kicad on Linux).
"""
import argparse
import os
import platform
import sys
import traceback
import webbrowser

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kisi_kicad as core  # noqa: E402


def modified_state(board):
    """'modified', 'unmodified' or 'unknown'. KiCad 10's API cannot tell; GetDocumentModifiedState (KiCad's
    development branch, the later 11) is used when the installed kicad-python has it."""
    try:
        from kipy.proto.common.commands import editor_commands_pb2 as ec
    except ImportError:
        return "unknown"
    cmd_type = getattr(ec, "GetDocumentModifiedState", None)
    resp_type = getattr(ec, "GetDocumentModifiedStateResponse", None)
    client = getattr(board, "client", None) or getattr(board, "_kicad", None)
    if cmd_type is None or resp_type is None or client is None:
        return "unknown"
    try:
        cmd = cmd_type()
        cmd.document.CopyFrom(board.document)
        state = client.send(cmd, resp_type).state
    except Exception:  # an older KiCad answers "unhandled"
        return "unknown"
    return {1: "unmodified", 2: "modified"}.get(int(state), "unknown")


def board_from_kicad(log):
    """(board file name, project folder, project name, modified state, KiCad version) of the open board."""
    try:
        from kipy import KiCad
    except ImportError as e:
        raise core.PluginError("kicad-python is missing from the plugin's Python environment – in KiCad, Preferences → "
                               "Plugins: right-click the kisi action and choose “Recreate Plugin Environment”") from e
    try:
        kicad = KiCad(client_name="kisi", timeout_ms=10000)
        try:
            v = kicad.get_version()
            kv = f"{v.major}.{v.minor}.{v.patch}" + (f" ({v.full_version})" if v.full_version else "")
        except Exception as e:  # not needed for the job
            kv = ""
            log(f"version: {e!r}")
        board = kicad.get_board()
    except core.PluginError:
        raise
    except Exception as e:
        log("KiCad API: " + traceback.format_exc())
        raise core.PluginError(f"could not get the open board from KiCad ({e}) – is a board open in the PCB editor?") from e
    doc = board.document
    state = modified_state(board)
    log(f"KiCad {kv or '?'}: board {doc.board_filename!r}, project {doc.project.name!r} in {doc.project.path!r}, unsaved changes: {state}")
    return doc.board_filename, doc.project.path, doc.project.name, state, kv


def say(msg):
    """One line for KiCad (stderr; shown at the PCB editor's alert triangle in KiCad 10.0.1 and later)."""
    try:
        if sys.stderr:
            print("kisi: " + msg, file=sys.stderr)
    except Exception:
        pass


def allow_foreground(log, user32=None):
    """Windows: let the browser bring its window to the front. A browser that is already running gets the page as a
    new tab, and Windows lets it raise its window only when the process asking may do so – this one may (KiCad, in
    the foreground, started it) and hands the right on (AllowSetForegroundWindow(ASFW_ANY)). Without it the tab
    opens behind KiCad from the second click on. `user32` is replaced in tests."""
    try:
        if user32 is None:
            import ctypes
            user32 = ctypes.windll.user32
        if not user32.AllowSetForegroundWindow(-1):  # ASFW_ANY
            log("AllowSetForegroundWindow: refused (the browser may open behind KiCad)")
    except Exception as e:
        log(f"AllowSetForegroundWindow: {e!r}")


OPENER_WAIT = 3  # seconds: how long `open`/`xdg-open` may take to report a failure


def open_page(path, log, system=None, startfile=None, popen=None, browser=None, user32=None):
    """Open the page in the default browser; returns how. Windows: os.startfile with the plain path – the .html
    file's default handler, i.e. the default browser (a file:// URL of a path with umlauts reached Edge as its start
    page in the KiCad 10 test); macOS: `open`; Linux: `xdg-open`; else, and as the fallback, webbrowser with the
    file:// URL. `open` and `xdg-open` hand the file on and exit, non-zero when no application took it (xdg-open:
    3 = no tool, 4 = the action failed) – then the fallback; one still running after OPENER_WAIT seconds (xdg-open
    without a desktop environment runs the browser itself) counts as opened. Windows lets the browser come to the
    front first (allow_foreground). The arguments are replaced in tests."""
    if os.environ.get("KISI_NO_BROWSER"):
        return "none (KISI_NO_BROWSER)"
    import subprocess
    from pathlib import Path
    system = system or platform.system()
    startfile = startfile if startfile is not None else getattr(os, "startfile", None)
    popen = popen or subprocess.Popen
    browser = browser or webbrowser
    path = os.path.normpath(os.path.abspath(path))
    tried = []
    if system == "Windows" and startfile:
        allow_foreground(log, user32)
        try:
            startfile(path)  # noqa: S606 (our own page)
            return f"os.startfile({path!r})"
        except OSError as e:
            tried.append(f"os.startfile: {e}")
    elif system in ("Darwin", "Linux") or system.endswith("BSD"):
        cmd = ["open", path] if system == "Darwin" else ["xdg-open", path]
        try:
            proc = popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            try:
                rc = proc.wait(timeout=OPENER_WAIT)
            except subprocess.TimeoutExpired:
                rc = 0
            if rc == 0:
                return f"{cmd[0]} {path!r}"
            tried.append(f"{cmd[0]} exited with {rc}")
        except OSError as e:
            tried.append(f"{cmd[0]}: {e}")
    uri = Path(path).as_uri()
    try:
        if browser.open(uri):
            return f"webbrowser.open({uri!r})" + (f" after {'; '.join(tried)}" if tried else "")
    except Exception as e:
        tried.append(f"webbrowser: {e!r}")
    raise core.PluginError(f"could not start a browser ({'; '.join(tried) or 'webbrowser refused'}) – open {path} yourself")


def main(argv=None):
    ap = argparse.ArgumentParser(description="Open the board in kisi's viewer.")
    ap.add_argument("--pcb", help="without KiCad: this board file")
    ap.add_argument("--no-browser", action="store_true", help="only write the page and print its path")
    a, _ = ap.parse_known_args(argv)
    if a.no_browser:
        os.environ["KISI_NO_BROWSER"] = "1"
    log = core.Log()
    log(f"---- kisi plugin {core.version()}, {platform.platform()}, Python {sys.version.split()[0]} ({sys.executable}), cwd {os.getcwd()}")
    try:
        if a.pcb:
            pcb = os.path.abspath(a.pcb)
            name, folder, project, state, kv = os.path.basename(pcb), os.path.dirname(pcb), os.path.splitext(os.path.basename(pcb))[0], "unknown", ""
        else:
            name, folder, project, state, kv = board_from_kicad(log)
        page, payload = core.make_page(name, folder, project, modified=state, kicad_version=kv)
        imp, cli = payload["import"], payload["cli"]
        log(f"page {page} ({os.path.getsize(page)} bytes, {payload['page']['where']}{': ' + payload['page']['reason'] if payload['page']['reason'] else ''}); "
            f"kisi.json {payload['config']}: {payload['config_path']}; files {[f['name'] for f in payload['files']]}; notes {payload['notes']}")
        log(f"downloads {imp['folder'] or '-'}: {imp['state']} {imp['from']} {imp['backup']} {imp['checked']} {imp['reasons']}; "
            f"cli {cli['path'] or '-'}; node {cli['node']}")
        log(f"browser: {open_page(page, log)}")
        try:  # for KiCad's messages and tests; the page is open already, so a console that cannot print it is no error
            print(page)
        except Exception as e:
            log(f"stdout: {e!r}")
        return 0
    except core.PluginError as e:
        msg = str(e)
    except Exception as e:  # a bug: the traceback goes to the log
        log(traceback.format_exc())
        msg = f"unexpected error {type(e).__name__}: {e}"
    log("error: " + msg)
    say(msg + f" (log: {log.path})")
    try:
        log(f"browser (error page): {open_page(core.error_page(msg, log.path), log)}")
    except Exception as e:
        log(f"error page: {e!r}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
