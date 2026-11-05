#!/usr/bin/env python3
"""Check the packaged local viewer: real browser click -> HTTP -> board-side JSON, on every OS.

Usage: check_kicad_local.py <package.zip> <board.kicad_pcb> [--browser chromium|firefox|webkit] [--out DIR]
KiCad's board discovery is replaced by --pcb; the installed action, server, bundle and disk writes are real.
Playwright is a test dependency only. No file picker or download is stubbed into a successful save.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import zipfile

from playwright.sync_api import sync_playwright


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("package")
    ap.add_argument("board")
    ap.add_argument("--browser", choices=["chromium", "firefox", "webkit"], default="chromium")
    ap.add_argument("--out", default="local-viewer-results")
    ap.add_argument("--executable", help="optional local browser executable for development checks")
    a = ap.parse_args()
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="kisi-local-check-") as td:
        root = Path(td)
        plugin = root / "3rdparty" / "plugins" / "com_github_riehlies_kisi"
        with zipfile.ZipFile(a.package) as z:
            for entry in z.infolist():
                if entry.filename.startswith("plugins/") and not entry.is_dir():
                    dest = plugin.joinpath(*entry.filename.split("/")[1:])
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(z.read(entry))
        folder = root / "Boards äöü" / "Prüf Board"
        folder.mkdir(parents=True)
        source = Path(a.board)
        pcb = folder / "Prüf Board.kicad_pcb"
        shutil.copyfile(source, pcb)
        pro = source.with_suffix(".kicad_pro")
        if pro.exists():
            shutil.copyfile(pro, pcb.with_suffix(".kicad_pro"))
        cfg = pcb.with_suffix(".kisi.json")
        pcb_hash = hashlib.sha256(pcb.read_bytes()).hexdigest()
        temp = root / "cache"
        temp.mkdir()
        env = dict(os.environ, TMPDIR=str(temp), TEMP=str(temp), TMP=str(temp), XDG_CACHE_HOME=str(temp), KISI_NO_BROWSER="1")
        proc = subprocess.Popen([sys.executable, "-I", str(plugin / "kisi_action.py"), "--pcb", str(pcb),
                                 "--no-browser", "--serve", "--idle-timeout", "300"],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", env=env)
        try:
            lines = queue.Queue()
            threading.Thread(target=lambda: lines.put(proc.stdout.readline()), daemon=True).start()
            url = lines.get(timeout=30).strip()
            assert url.startswith("http://127.0.0.1:"), "the packaged action did not start its local viewer"
            with sync_playwright() as pw:
                browser = getattr(pw, a.browser).launch(executable_path=a.executable)
                context = browser.new_context(viewport={"width": 1440, "height": 1000}, accept_downloads=True)
                pg = context.new_page()
                errors, downloads = [], []
                pg.on("pageerror", lambda e: errors.append(str(e)))
                pg.on("download", lambda d: downloads.append(d.suggested_filename))
                # A save must not silently fall back to a browser file picker, either.
                pg.add_init_script("window.showSaveFilePicker = async () => { throw new Error('unexpected save picker'); };"
                                   "window.showDirectoryPicker = async () => { throw new Error('unexpected directory picker'); };")
                pg.goto(url)
                pg.wait_for_function("typeof S !== 'undefined' && S.scene && document.getElementById('busy').hidden && S.prep.base !== undefined", timeout=180000)
                pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)
                pg.click("#prep-btn")
                pg.wait_for_function("S.prep.open && !S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)
                assert not pg.is_visible("#prep-kicad-dir"), "folder chooser shown in the plugin"
                assert not pg.is_visible("#prep-kicad-copy"), "save-dialog path helper shown in the plugin"

                def edit(note):
                    if pg.locator("details.prep-notes").get_attribute("open") is None:
                        pg.click("details.prep-notes > summary")
                    pg.fill("#prep-notes", note)
                    pg.locator("#prep-notes").dispatch_event("change")
                    pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)

                edit("Local save äöü")
                pg.screenshot(path=str(out / "before-save.png"), full_page=True)
                with pg.expect_response(lambda r: r.url.endswith("/api/config") and r.request.method == "POST") as reply:
                    pg.click("#prep-save")
                assert reply.value.status == 200, "configuration POST failed"
                pg.wait_for_function("!S.prep.saving && S.prep.saved.includes('saved into the board')", timeout=120000)
                saved = cfg.read_bytes()
                assert json.loads(saved)["notes"] == ["Local save äöü"]
                assert not downloads, "save produced a download"
                assert hashlib.sha256(pcb.read_bytes()).hexdigest() == pcb_hash, "the board changed"
                assert list((folder / "analysis").glob("*.bak")), "no backup of the previous configuration"
                pg.screenshot(path=str(out / "saved.png"), full_page=True)

                # A page reload reads the new file and revision from this same session.
                pg.reload()
                pg.wait_for_function("typeof S !== 'undefined' && S.scene && document.getElementById('busy').hidden && S.prep.base !== undefined", timeout=180000)
                pg.click("#prep-btn")
                pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)
                assert pg.input_value("#prep-notes") == "Local save äöü"

                external = json.loads(cfg.read_text(encoding="utf-8"))
                external["notes"] = ["external edit"]
                cfg.write_text(json.dumps(external), encoding="utf-8")
                edit("keep this unsaved draft")
                pg.click("#prep-save")
                pg.wait_for_function("!S.prep.saving && S.prep.saved.includes('changed outside')")
                assert json.loads(cfg.read_bytes())["notes"] == ["external edit"], "external change overwritten"
                assert pg.input_value("#prep-notes") == "keep this unsaved draft", "draft lost after a conflict"

                # Invalid credentials are rejected without a download or a filesystem change.
                pg.evaluate("S.kicad.bridge.token = 'wrong'")
                pg.click("#prep-save")
                pg.wait_for_function("!S.prep.saving && S.prep.saved.includes('forbidden')")
                assert not downloads
                assert json.loads(cfg.read_bytes())["notes"] == ["external edit"]
                pg.screenshot(path=str(out / "save-error.png"), full_page=True)

                # A disconnected session reports an error and keeps the form, without downloading.
                proc.terminate()
                proc.wait(timeout=10)
                pg.click("#prep-save")
                pg.wait_for_function("!S.prep.saving && S.prep.saved.includes('connection')", timeout=20000)
                assert pg.input_value("#prep-notes") == "keep this unsaved draft"
                assert not downloads
                assert not errors, "JavaScript errors: " + "; ".join(errors)
                context.close()
                browser.close()
            print(f"local viewer: {a.browser} on {sys.platform}: save, reload, conflicts and errors passed")
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=10)
            stderr = proc.stderr.read()
            # Never persist stdout: it contains the secret entry URL.
            if stderr:
                (out / "plugin-stderr.txt").write_text(stderr, encoding="utf-8")
            proc.stdout.close()
            proc.stderr.close()


if __name__ == "__main__":
    main()
