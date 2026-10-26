#!/usr/bin/env python3
"""Headless check of the KiCad plugin package with the viewer it carries (Playwright; Chromium by default).

Usage: check_kicad_plugin.py <kisi-kicad-X.zip> <board.kicad_pcb> [--config ref.kisi.json] [--out DIR]
                             [--browser chromium|firefox|webkit]

Without KiCad (no KiCad runs in CI): the zip is extracted the way KiCad's Plugin and Content Manager does
(<zip>/plugins/… → 3rdparty/plugins/com_github_riehlies_kisi/…), the board is copied into a folder whose name has
spaces and umlauts, and the extracted kisi_action.py makes the page (--pcb, as KiCad's button does with the board's
path from the API). Then, opened from disk (file://):
1. no kisi.json next to the board: the plugin creates the skeleton; the page starts with the board loaded from the
   embedded files, the "From KiCad" card (board, folder, kisi.json created, the saved-file note), no console errors
2. Prepare PCB: the path of the board's kisi.json, saving through the File System Access dialog (stubbed: the
   options – suggested name, the remembered-folder id), the saved file loaded, the card says so
3. the download path (no File System Access): the download and the note where to put the file
4. a second click with the saved kisi.json next to the board: loaded from there
5. --config: a full kisi.json next to the board, checked without errors; KiCad's "unsaved changes" notice
Screenshots (kicad-start.png, kicad-prepare.png, kicad-saved.png, …) go to --out. Exit code 1 on problems.
"""
import argparse, json, os, pathlib, re, shutil, subprocess, sys, tempfile, zipfile
from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("zip")
ap.add_argument("board")
ap.add_argument("--config", help="a <board>.kisi.json to put next to the board for the last run")
ap.add_argument("--out")
ap.add_argument("--browser", default="chromium", choices=["chromium", "firefox", "webkit"])
a = ap.parse_args()
out = a.out or tempfile.mkdtemp(prefix="kisi-kicad-check-")
os.makedirs(out, exist_ok=True)
root = tempfile.mkdtemp(prefix="kisi-kicad-plugin-")
problems, notes = [], []
BUSY = "document.getElementById('busy').hidden === true"


def check(ok, msg):
    if not ok:
        problems.append(msg)
        print("PROBLEM:", msg)
    return ok


# ---- install as KiCad's PCM does (kicad/pcm/pcm_task_manager.cpp) -------------------------------------------------
with zipfile.ZipFile(a.zip) as z:
    md = json.loads(z.read("metadata.json"))
    for e in z.infolist():
        parts = e.filename.split("/")
        if e.is_dir() or len(parts) < 2 or parts[0] not in ("plugins", "resources"):
            continue
        t = os.path.join(root, "3rdparty", parts[0], md["identifier"].replace(".", "_"), *parts[1:])
        os.makedirs(os.path.dirname(t), exist_ok=True)
        pathlib.Path(t).write_bytes(z.read(e))
plugin = os.path.join(root, "3rdparty", "plugins", md["identifier"].replace(".", "_"))
action = os.path.join(plugin, "kisi_action.py")
check(os.path.isfile(os.path.join(plugin, "plugin.json")) and os.path.isfile(os.path.join(plugin, "requirements.txt")),
      "plugin.json or requirements.txt not where KiCad looks")

stem = os.path.basename(a.board).removesuffix(".kicad_pcb")
bdir = os.path.join(root, "Projekte", "kisi Test äöü ß")
os.makedirs(bdir)
shutil.copy(a.board, bdir)
pro = a.board.removesuffix(".kicad_pcb") + ".kicad_pro"
if os.path.exists(pro):
    shutil.copy(pro, bdir)
pcb = os.path.join(bdir, stem + ".kicad_pcb")
cfg = os.path.join(bdir, stem + ".kisi.json")
tmp = os.path.join(root, "temp")
os.makedirs(tmp)
ENV = dict(os.environ, TMPDIR=tmp, TEMP=tmp, TMP=tmp)


def make_page(modified=None):
    """The extracted plugin makes the page: kisi_action.py --pcb (KiCad's way minus the API), or kisi_kicad with a
    modified state (what KiCad's API would report)."""
    if modified:
        code = ("import sys; sys.path.insert(0, sys.argv[1]); import kisi_kicad as k, os; "
                "print(k.make_page(os.path.basename(sys.argv[2]), os.path.dirname(sys.argv[2]), '', modified=sys.argv[3], kicad_version='10.0.7')[0])")
        cmd = [sys.executable, "-I", "-c", code, plugin, pcb, modified]
    else:
        cmd = [sys.executable, "-I", action, "--pcb", pcb, "--no-browser"]
    p = subprocess.run(cmd, capture_output=True, text=True, env=ENV, cwd=plugin)
    if not check(p.returncode == 0, f"plugin failed: {p.stderr.strip()}"):
        raise SystemExit(1)
    return p.stdout.strip().splitlines()[-1]


def open_page(pg, page, tag):
    errs = []
    pg.on("pageerror", lambda e: errs.append(f"page error: {e}"))
    pg.on("console", lambda m: errs.append(f"console {m.type}: {m.text}") if m.type == "error" else None)
    pg.goto(pathlib.Path(page).as_uri())
    try:
        pg.wait_for_function(f"typeof S !== 'undefined' && S.scene && {BUSY} && S.prep && S.prep.base !== undefined", timeout=180000)
        pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)
    except Exception:
        pg.screenshot(path=f"{out}/{tag}_failed.png")
        check(False, f"{tag}: the board did not load: {pg.inner_text('#busy')[:200]} {pg.inner_text('#warnings')[:200]} {errs[:3]}")
        raise SystemExit(1)
    pg.wait_for_timeout(300)
    return errs


def card(pg):
    return pg.evaluate("document.getElementById('kicad-card').classList.contains('off') ? null : document.getElementById('kicad-card').innerText")


def open_prepare(pg):
    if not pg.evaluate("S.prep.open"):
        pg.click("#prep-btn")
    pg.wait_for_function("S.prep.open && !S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000)
    pg.wait_for_timeout(200)


FSA_STUB = """() => { window.__fsa = null; window.__saved = null;
  window.showSaveFilePicker = async (o) => { window.__fsa = o; return { kind: "file", name: o.suggestedName,
    createWritable: async () => ({ write: async (t) => { window.__saved = t; }, close: async () => {} }) }; }; }"""

with sync_playwright() as p:
    b = getattr(p, a.browser).launch()
    ctx = b.new_context(viewport={"width": 1440, "height": 900}, accept_downloads=True)
    # 1. first click: no kisi.json next to the board
    page1 = make_page()
    shutil.copy(page1, os.path.join(out, f"{stem}-from-kicad.html"))  # a sample of the generated page
    check(os.path.isfile(cfg), "the skeleton kisi.json was not created next to the board")
    check(json.loads(pathlib.Path(cfg).read_text(encoding="utf-8")) == {"kisi_config": 1, "board": stem + ".kicad_pcb"}, "skeleton content")
    pg = ctx.new_page()
    errs = open_page(pg, page1, "start")
    st = pg.evaluate("({ name: S.name, cfg: S.lastFiles && S.lastFiles.cfg && S.lastFiles.cfg.name, ok: !!(S.config && S.config.check && S.config.check.ok),"
                     " fsa: typeof window.showSaveFilePicker, secure: window.isSecureContext, sample: S.sample })")
    check(st["name"] == stem, f"start: board name {st['name']!r}")
    check(st["cfg"] == stem + ".kisi.json" and st["ok"], f"start: kisi.json not loaded or not ok: {st}")
    c = card(pg) or ""
    check("from kicad" in c.lower() and stem + ".kicad_pcb" in c and bdir in c, f"start: the KiCad card: {c!r}")
    check("empty one was created next to the board" in c and "not yet saved in KiCad" in c, f"start: card text: {c!r}")
    notes.append(f"{a.browser} on file://: showSaveFilePicker {st['fsa']}, secure context {st['secure']}")
    pg.screenshot(path=f"{out}/kicad-start.png")
    # 2. Prepare PCB: save through the File System Access dialog (stubbed)
    open_prepare(pg)
    vis = pg.evaluate("[!document.getElementById('prep-kicad').hidden, document.getElementById('prep-kicad-path').textContent, document.getElementById('prep-save').disabled, document.getElementById('prep-save-note').textContent]")
    check(vis[0] and vis[1] == cfg, f"prepare: path of the kisi.json: {vis[:2]}")
    pg.evaluate(FSA_STUB)
    pg.evaluate("prepRender(true)")
    note = pg.inner_text("#prep-save-note")
    check("choose the board's folder" in note, f"prepare: save note: {note!r}")
    check(not pg.evaluate("document.getElementById('prep-save').disabled"), "prepare: Save kisi.json is disabled")
    pg.locator("#prep-kicad").scroll_into_view_if_needed()
    pg.screenshot(path=f"{out}/kicad-prepare.png")
    pg.click("#prep-save")
    pg.wait_for_function(f"window.__saved && {BUSY} && S.prep.base && /saved and loaded/.test(document.getElementById('prep-save-note').textContent)", timeout=120000)
    o = pg.evaluate("({ id: window.__fsa.id, name: window.__fsa.suggestedName, startIn: window.__fsa.startIn ?? null, saved: window.__saved })")
    check(o["name"] == stem + ".kisi.json", f"save: suggested name {o['name']!r}")
    check(bool(re.fullmatch(r"kisi-[0-9a-f]{1,8}", o["id"] or "")) and len(o["id"]) <= 32, f"save: id {o['id']!r} (File System Access: ≤ 32 of [A-Za-z0-9_-])")
    check(o["startIn"] is None, f"save: startIn {o['startIn']!r} (none: the id's remembered folder decides)")
    saved = json.loads(o["saved"])
    check(saved.get("board") == stem + ".kicad_pcb" and saved.get("kisi_config") == 1, f"save: content {o['saved'][:200]}")
    c = card(pg) or ""
    check("saved from the" in c, f"save: card after saving: {c!r}")
    pg.screenshot(path=f"{out}/kicad-saved.png")
    # 3. the download path (Firefox, Safari)
    pg.evaluate("window.showSaveFilePicker = undefined; S.prep.saved = ''; prepRender(true)")  # as after the next edit
    note = pg.inner_text("#prep-save-note")
    check("move it from the downloads folder" in note, f"download: note before saving: {note!r}")
    with pg.expect_download() as dl:
        pg.click("#prep-save")
    check(dl.value.suggested_filename == stem + ".kisi.json", f"download: file name {dl.value.suggested_filename!r}")
    pg.wait_for_function(f"{BUSY} && /downloaded and loaded/.test(document.getElementById('prep-save-note').textContent)", timeout=120000)
    check("Move it from the downloads folder" in pg.inner_text("#prep-save-note"), "download: note after saving")
    check("downloaded: move it" in (card(pg) or ""), f"download: card: {card(pg)!r}")
    pg.screenshot(path=f"{out}/kicad-download.png")
    for e in errs:
        check(False, f"first page: {e}")
    pg.close()
    # 4. second click: the saved kisi.json next to the board (as the dialog wrote it)
    pathlib.Path(cfg).write_text(o["saved"], encoding="utf-8")
    page2 = make_page()
    check(page2 == page1, "second click: a new page instead of replacing the first")
    pg = ctx.new_page()
    errs = open_page(pg, page2, "second")
    c = card(pg) or ""
    check("loaded from next to the board" in c, f"second click: card: {c!r}")
    for e in errs:
        check(False, f"second page: {e}")
    pg.close()
    # 5. a full kisi.json, and KiCad reporting unsaved changes
    if a.config:
        shutil.copy(a.config, cfg)
    page3 = make_page(modified="modified")
    pg = ctx.new_page()
    errs = open_page(pg, page3, "modified")
    c = card(pg) or ""
    check("KiCad reports unsaved changes" in c, f"modified: card: {c!r}")
    if a.config:
        r = pg.evaluate("({ ok: S.config.check.ok, entries: S.config.check.entries, errors: S.config.check.errors })")
        check(r["ok"] and r["entries"] > 0, f"--config: {r}")
    pg.screenshot(path=f"{out}/kicad-modified.png")
    for e in errs:
        check(False, f"modified page: {e}")
    pg.close()
    b.close()

for n in notes:
    print("note:", n)
print(f"screenshots and the generated page: {out}")
print("kicad plugin check:", "ok" if not problems else f"{len(problems)} problem(s)")
shutil.rmtree(root, ignore_errors=True)
sys.exit(1 if problems else 0)
