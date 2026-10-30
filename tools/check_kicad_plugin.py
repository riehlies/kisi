#!/usr/bin/env python3
"""Headless check of the KiCad plugin package with the viewer and CLI it carries (Playwright; Chromium by default).

Usage: check_kicad_plugin.py <kisi-kicad-X.zip> <board.kicad_pcb> [--config ref.kisi.json] [--out DIR]
                             [--browser chromium|firefox|webkit]

Without KiCad (no KiCad runs in CI): the zip is extracted the way KiCad's Plugin and Content Manager does
(<zip>/plugins/… → 3rdparty/plugins/com_github_riehlies_kisi/…), the board is copied into a folder whose name has
spaces and umlauts, and the extracted kisi_action.py makes the page (--pcb, as KiCad's button does with the board's
path from the API), with a HOME of its own whose Downloads folder the plugin watches. Then, opened from disk:
1. no kisi.json next to the board: the plugin creates the skeleton and writes analysis/<board>.kisi.html; the page
   starts with the board from the embedded files, the "From KiCad" card (board, folder, kisi.json created, the
   page's place, the saved-file note), the agent card with the installed CLI's command and the board's paths
   (Node.js found when this machine has it), no console errors
2. Prepare PCB: the path of the board's kisi.json, saving through the File System Access dialog (stubbed: the
   options – suggested name, the remembered-folder id), the saved file loaded, the card says so
3. the download path (no File System Access): the note with the browsers' "ask where to save" settings, the
   download (put into the Downloads folder as a browser would), the card's note about the next click
4. the next click takes the download over: moved next to the board (checked with the CLI's config-check where Node.js
   is there), the previous kisi.json kept in analysis/, the card says so; two candidates: nothing is taken over
5. --config: a full kisi.json next to the board, checked without errors; KiCad's "unsaved changes" notice; the agent
   card without Node.js (the nodejs.org note)
Screenshots (kicad-start.png, kicad-prepare.png, kicad-saved.png, kicad-download.png, kicad-import.png, …) and a
generated page go to --out. Exit code 1 on problems.
"""
import argparse, json, os, pathlib, re, shutil, subprocess, sys, tempfile, time, zipfile
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
cli = os.path.join(plugin, "cli", "kisi.mjs")
check(os.path.isfile(os.path.join(plugin, "plugin.json")) and os.path.isfile(os.path.join(plugin, "requirements.txt")),
      "plugin.json or requirements.txt not where KiCad looks")
check(os.path.isfile(cli), "cli/kisi.mjs is not in the package")

stem = os.path.basename(a.board).removesuffix(".kicad_pcb")
bdir = os.path.join(root, "Projekte", "kisi Test äöü ß")
os.makedirs(bdir)
shutil.copy(a.board, bdir)
pro = a.board.removesuffix(".kicad_pcb") + ".kicad_pro"
if os.path.exists(pro):
    shutil.copy(pro, bdir)
pcb = os.path.join(bdir, stem + ".kicad_pcb")
cfg = os.path.join(bdir, stem + ".kisi.json")
page_want = os.path.join(bdir, "analysis", stem + ".kisi.html")
tmp, home = os.path.join(root, "temp"), os.path.join(root, "home")
downloads = os.path.join(home, "Downloads")
os.makedirs(tmp)
os.makedirs(downloads)
# the plugin's Downloads folder: ~/Downloads of this HOME (xdg-user-dir, where installed, gives the same)
# the plugin's own folder: the temporary folder (Windows, macOS) or $XDG_CACHE_HOME (Linux) – both point to tmp
ENV = dict(os.environ, TMPDIR=tmp, TEMP=tmp, TMP=tmp, XDG_CACHE_HOME=tmp, HOME=home, XDG_CONFIG_HOME=os.path.join(home, ".config"))
have_node = bool(shutil.which("node"))


def make_page(modified=None, node_off=False):
    """The extracted plugin makes the page: kisi_action.py --pcb (KiCad's way minus the API), or kisi_kicad with a
    modified state (what KiCad's API would report) and, with node_off, as on a machine without Node.js."""
    if modified or node_off:
        code = ("import sys, os; sys.path.insert(0, sys.argv[1]); import kisi_kicad as k; "
                "node = {'ok': False, 'path': '', 'version': '', 'reason': 'Node.js was not found'} if sys.argv[4] == '1' else 'auto'; "
                "print(k.make_page(os.path.basename(sys.argv[2]), os.path.dirname(sys.argv[2]), '', modified=sys.argv[3], kicad_version='10.0.7', node=node)[0])")
        cmd = [sys.executable, "-I", "-c", code, plugin, pcb, modified or "unknown", "1" if node_off else "0"]
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
    return pg.evaluate("document.getElementById('kicad-card').classList.contains('off') ? null : document.getElementById('kicad-card').innerText") or ""


def open_prepare(pg):
    if not pg.evaluate("S.prep.open"):
        pg.click("#prep-btn")
    pg.wait_for_function("S.prep.open && !S.prep.busy && !S.prep.inflight && !S.prep.queued && !!document.querySelector('#prep-gaps .prep-step, #prep-gaps .prep-empty')", timeout=120000)
    pg.wait_for_timeout(200)


def agent(pg):
    return pg.evaluate("({ shown: !document.getElementById('prep-agent-kicad').hidden, text: document.getElementById('prep-agent-kicad').innerText,"
                       " cmd: (document.getElementById('prep-agent-cmd') || {}).textContent || '', prompt: document.getElementById('prep-prompt').textContent })")


def age(path, seconds):
    t = time.time() - seconds
    os.utime(path, (t, t))


# a directory handle as showDirectoryPicker gives it (the picker itself cannot run headless): name, the files in it,
# permission "prompt" until requested; records what is written and every permission request
DIR_STUB = """([name, files, grant]) => { const h = { kind: "directory", name, written: {}, asked: 0,
    queryPermission: async () => (h.asked ? (grant ? "granted" : "denied") : "prompt"),
    requestPermission: async () => { h.asked++; return grant ? "granted" : "denied"; },
    getFileHandle: async (n, o) => { if (!files.includes(n) && !(o && o.create)) throw new DOMException("not found", "NotFoundError");
      return { kind: "file", name: n, createWritable: async () => ({ write: async (t) => { h.written[n] = t; }, close: async () => {} }) }; } };
  window.__dir = h; window.showDirectoryPicker = async (o) => { window.__dirOpts = o; return h; }; }"""
FSA_STUB = """() => { window.__fsa = null; window.__saved = null;
  window.showSaveFilePicker = async (o) => { window.__fsa = o; return { kind: "file", name: o.suggestedName,
    createWritable: async () => ({ write: async (t) => { window.__saved = t; }, close: async () => {} }) }; }; }"""

with sync_playwright() as p:
    b = getattr(p, a.browser).launch()
    ctx = b.new_context(viewport={"width": 1440, "height": 900}, accept_downloads=True)
    # 1. first click: no kisi.json next to the board
    page1 = make_page()
    check(page1 == page_want, f"start: the page is {page1}, not analysis/{stem}.kisi.html next to the board")
    shutil.copy(page1, os.path.join(out, f"{stem}-from-kicad.html"))  # a sample of the generated page
    check(os.path.isfile(cfg), "the skeleton kisi.json was not created next to the board")
    check(json.loads(pathlib.Path(cfg).read_text(encoding="utf-8")) == {"kisi_config": 1, "board": stem + ".kicad_pcb"}, "skeleton content")
    check(sorted(os.listdir(bdir)) == sorted([stem + ".kicad_pcb", stem + ".kisi.json", "analysis"] + ([stem + ".kicad_pro"] if os.path.exists(pro) else [])),
          f"start: files next to the board: {sorted(os.listdir(bdir))}")
    pg = ctx.new_page()
    errs = open_page(pg, page1, "start")
    st = pg.evaluate("({ name: S.name, cfg: S.lastFiles && S.lastFiles.cfg && S.lastFiles.cfg.name, ok: !!(S.config && S.config.check && S.config.check.ok),"
                     " fsa: typeof window.showSaveFilePicker, secure: window.isSecureContext })")
    check(st["name"] == stem, f"start: board name {st['name']!r}")
    check(st["cfg"] == stem + ".kisi.json" and st["ok"], f"start: kisi.json not loaded or not ok: {st}")
    c = card(pg)
    check("from kicad" in c.lower() and stem + ".kicad_pcb" in c and bdir in c, f"start: the KiCad card: {c!r}")
    check("empty one was created next to the board" in c and "not yet saved in KiCad" in c, f"start: card text: {c!r}")
    check(f"This page: {page_want}" in c, f"start: the card does not name the page in analysis/: {c!r}")
    notes.append(f"{a.browser} on file://: showSaveFilePicker {st['fsa']}, secure context {st['secure']}; node on this machine: {have_node}")
    pg.screenshot(path=f"{out}/kicad-start.png")
    # 2. Prepare PCB: the agent card with the installed CLI, then saving through the File System Access dialog (stubbed)
    open_prepare(pg)
    ag = agent(pg)
    want_cmd = f'node "{cli}" gaps "{pcb}" --for impedance'
    check(ag["shown"] and ag["cmd"] == want_cmd, f"agent card: command {ag['cmd']!r}, expected {want_cmd!r}")
    pg.evaluate("S.prep.agentScope = 'all'; prepRender(true)")  # a prompt also where this tab has nothing to check
    ag_all = agent(pg)
    check(ag_all["cmd"] == f'node "{cli}" gaps "{pcb}"', f"agent card (all categories): command {ag_all['cmd']!r}")
    guide = os.path.join(plugin, "agent", "KISI-AGENT-GUIDE.md")
    check(f'its kisi.json belongs at "{cfg}"' in ag_all["prompt"] and f'node "{cli}"' in ag_all["prompt"], f"agent card: the prompt lacks the paths: {ag_all['prompt'][-500:]!r}")
    check('Follow the kisi agent guide (KISI-AGENT-GUIDE.md' in ag_all["prompt"] and guide in ag_all["prompt"] and "return the complete new kisi.json" in ag_all["prompt"],
          f"agent card: the prompt does not refer to the guide and both modes: {ag_all['prompt'][-600:]!r}")
    check(guide in ag["text"] and pg.evaluate("!!document.getElementById('prep-guide-copy') && S.kicad.guide.text === %s" % json.dumps(pathlib.Path(guide).read_text(encoding="utf-8"))),
          "agent card: the guide's path or the Copy guide button / text")
    pg.evaluate("S.prep.agentScope = 'tab'; prepRender(true)")
    check(("Node.js" in ag["text"] and "found" in ag["text"]) if have_node else "nodejs.org" in ag["text"], f"agent card: Node.js status: {ag['text']!r}")
    pg.locator("#prep-agent-kicad").scroll_into_view_if_needed()
    pg.screenshot(path=f"{out}/kicad-agent.png")
    vis = pg.evaluate("[!document.getElementById('prep-kicad').hidden, document.getElementById('prep-kicad-path').textContent]")
    check(vis[0] and vis[1] == cfg, f"prepare: path of the kisi.json: {vis}")
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
    check("saved from the" in card(pg), f"save: card after saving: {card(pg)!r}")
    pg.screenshot(path=f"{out}/kicad-saved.png")
    # 2b. Chromium: the board's folder chosen once (showDirectoryPicker), then saving writes there without a dialog
    check(pg.evaluate("kdirSet('probe', { a: 1 }).then(async (ok) => ok && (await kdirGet('probe')).a === 1)"), "IndexedDB does not work on file://")
    check(pg.evaluate("!document.getElementById('prep-kicad-dir').hidden") and "Choose the board folder" in pg.inner_text("#prep-kicad-dir"),
          f"folder: no 'Choose the board folder' step: {pg.inner_text('#prep-kicad-dir')!r}")
    base = os.path.basename(bdir)
    for name, files, want in (("Downloads", [stem + ".kicad_pcb"], "is not the board's folder"), (base, [], "has no " + stem + ".kicad_pcb")):
        pg.evaluate(DIR_STUB, [name, files, True])
        pg.click("#prep-kicad-choose"); pg.wait_for_timeout(300)
        check(want in pg.inner_text("#prep-kicad-dir") and pg.evaluate("!S.kicad.dirHandle"), f"folder {name!r}: {pg.inner_text('#prep-kicad-dir')!r}")
    pg.evaluate(DIR_STUB, [base, [stem + ".kicad_pcb", stem + ".kisi.json"], True])
    pg.click("#prep-kicad-choose"); pg.wait_for_timeout(300)
    opts = pg.evaluate("({ id: window.__dirOpts.id, mode: window.__dirOpts.mode, startIn: typeof window.__dirOpts.startIn === 'string' ? window.__dirOpts.startIn : 'handle' })")
    check(opts["mode"] == "readwrite" and opts["startIn"] == "documents" and re.fullmatch(r"kisi-dir-[0-9a-f]{1,8}", opts["id"] or ""), f"folder: picker options {opts}")
    check(f"Saves straight into the board's folder {base}" in pg.inner_text("#prep-kicad-dir"), f"folder: chosen: {pg.inner_text('#prep-kicad-dir')!r}")
    pg.evaluate(FSA_STUB)  # records a dialog, which must not open now
    pg.evaluate("""() => { const w = prepW(); w.notes = ["written into the chosen folder"]; prepEdited(true); }""")
    pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=60000)
    check("Writes straight into the board's folder" in pg.inner_text("#prep-save-note"), f"folder: save note {pg.inner_text('#prep-save-note')!r}")
    pg.click("#prep-save")
    pg.wait_for_function(f"{BUSY} && /saved into the board's folder and loaded/.test(document.getElementById('prep-save-note').textContent)", timeout=120000)
    r = pg.evaluate("({ text: window.__dir.written[%s], asked: window.__dir.asked, dialog: !!window.__fsa })" % json.dumps(stem + ".kisi.json"))
    check(r["text"] and "written into the chosen folder" in r["text"] and r["asked"] == 1 and not r["dialog"], f"folder: direct save {r}")
    pg.locator("#prep-kicad-dir").scroll_into_view_if_needed()
    pg.screenshot(path=f"{out}/kicad-folder-saved.png")
    # permission refused: the save dialog instead, starting in the chosen folder
    pg.evaluate(DIR_STUB, [base, [stem + ".kicad_pcb"], False])
    pg.evaluate("S.kicad.dirHandle = window.__dir")
    pg.evaluate(FSA_STUB)
    pg.click("#prep-save")
    pg.wait_for_function(f"window.__saved && {BUSY} && /saved and loaded/.test(document.getElementById('prep-save-note').textContent)", timeout=120000)
    check(pg.evaluate("window.__fsa.startIn === window.__dir") and "no permission" in pg.inner_text("#prep-kicad-dir"),
          f"folder: refused permission → dialog in the chosen folder: {pg.inner_text('#prep-kicad-dir')!r}")
    pg.evaluate("S.kicad.dirHandle = null; window.showDirectoryPicker = undefined; S.kicad.dirNote = ''")  # as in Firefox and Safari
    # 3. the download path (Firefox, Safari): the browsers' setting, else the take-over on the next click
    pg.evaluate("window.showSaveFilePicker = undefined; S.prep.saved = ''; prepRender(true)")  # as after the next edit
    note = pg.inner_text("#prep-save-note")
    check("Always ask you where to save files" in note and "Ask for each download" in note and "takes the file over" in note, f"download: note before saving: {note!r}")
    pg.evaluate("""() => { const w = prepW(); w.notes = ["entered in the browser, downloaded"]; prepEdited(true); }""")
    pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=60000)
    with pg.expect_download() as dl:
        pg.click("#prep-save")
    check(dl.value.suggested_filename == stem + ".kisi.json", f"download: file name {dl.value.suggested_filename!r}")
    got = os.path.join(downloads, dl.value.suggested_filename)
    dl.value.save_as(got)  # where the browser puts it by default
    pg.wait_for_function(f"{BUSY} && /Saved to the downloads folder/.test(document.getElementById('prep-save-note').textContent)", timeout=120000)
    check("click kisi in KiCad again to move it next to the board" in pg.inner_text("#prep-save-note") and "Always ask you where to save files" in pg.inner_text("#prep-save-note")
          and pg.evaluate("document.getElementById('prep-save-note').classList.contains('prep-dl')"), f"download: note after saving: {pg.inner_text('#prep-save-note')!r}")
    check("Click kisi in KiCad again" in card(pg), f"download: card: {card(pg)!r}")
    pg.screenshot(path=f"{out}/kicad-download.png")
    for e in errs:
        check(False, f"first page: {e}")
    pg.close()
    # 4. the next click takes the download over (the skeleton next to the board is older)
    age(cfg, 120)
    old = pathlib.Path(cfg).read_text(encoding="utf-8")
    want = pathlib.Path(got).read_text(encoding="utf-8")
    page2 = make_page()
    check(page2 == page1, "second click: another page instead of analysis/<board>.kisi.html again")
    check(pathlib.Path(cfg).read_text(encoding="utf-8") == want and not os.path.exists(got), "import: the download was not moved next to the board")
    baks = [f for f in os.listdir(os.path.join(bdir, "analysis")) if f.endswith(".bak")]
    check(len(baks) == 1 and pathlib.Path(bdir, "analysis", baks[0]).read_text(encoding="utf-8") == old, f"import: backup in analysis/: {baks}")
    pg = ctx.new_page()
    errs = open_page(pg, page2, "import")
    c = card(pg)
    check("Taken over from the downloads folder" in c and got in c and "previous file is kept" in c, f"import: card: {c!r}")
    check(("kisi config-check ok" in c) if have_node else ("format checked" in c), f"import: card names the check: {c!r}")
    check(pg.evaluate("S.lastFiles.cfg.text") == want, "import: the page did not load the taken-over kisi.json")
    pg.screenshot(path=f"{out}/kicad-import.png")
    for e in errs:
        check(False, f"import page: {e}")
    pg.close()
    # two candidates: nothing is taken over
    age(cfg, 120)
    for n in (stem + ".kisi.json", stem + ".kisi (1).json"):
        pathlib.Path(downloads, n).write_text(want, encoding="utf-8")
    before = pathlib.Path(cfg).read_text(encoding="utf-8")
    page3 = make_page()
    check(pathlib.Path(cfg).read_text(encoding="utf-8") == before and len(os.listdir(downloads)) == 2, "ambiguous: something was taken over")
    pg = ctx.new_page()
    errs = open_page(pg, page3, "ambiguous")
    check("Not taken over from the downloads folder" in card(pg) and "2 files" in card(pg), f"ambiguous: card: {card(pg)!r}")
    pg.screenshot(path=f"{out}/kicad-ambiguous.png")
    for e in errs:
        check(False, f"ambiguous page: {e}")
    pg.close()
    for n in os.listdir(downloads):
        os.remove(os.path.join(downloads, n))
    # 5. a full kisi.json, KiCad reporting unsaved changes, no Node.js
    if a.config:
        shutil.copy(a.config, cfg)
    page4 = make_page(modified="modified", node_off=True)
    pg = ctx.new_page()
    errs = open_page(pg, page4, "modified")
    check("KiCad reports unsaved changes" in card(pg), f"modified: card: {card(pg)!r}")
    if a.config:
        r = pg.evaluate("({ ok: S.config.check.ok, entries: S.config.check.entries, errors: S.config.check.errors })")
        check(r["ok"] and r["entries"] > 0, f"--config: {r}")
    pg.screenshot(path=f"{out}/kicad-modified.png")
    open_prepare(pg)
    pg.evaluate("S.prep.agentScope = 'all'; prepRender(true)")
    ag = agent(pg)
    check(ag["shown"] and "nodejs.org" in ag["text"] and "Node.js ≥ 18 is needed" in ag["prompt"], f"no Node.js: agent card {ag['text']!r}")
    pg.locator("#prep-agent-kicad").scroll_into_view_if_needed()
    pg.screenshot(path=f"{out}/kicad-agent-no-node.png")
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
