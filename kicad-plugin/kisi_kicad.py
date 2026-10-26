# SPDX-License-Identifier: GPL-3.0-only
"""kisi's KiCad plugin, the part without KiCad (standard library only, tested in kicad-plugin/tests).

Given the open board's file on disk, it
* reads <board>.kicad_pcb, the project's .kicad_pro and <board>.kisi.json next to the board – and writes that
  kisi.json as a minimal skeleton when there is none (the only file it ever writes next to the board; KiCad's
  files are never written),
* embeds them, with the BOMs and Touchstone files the kisi.json names, into a copy of the viewer page shipped
  with the plugin (viewer/kisi.html: the release's kisi.html, which has the marker below), and
* writes that page to a folder in the system's temporary directory, where the caller opens it in the browser.

The viewer loads the embedded files the way it loads dropped files (web/src/main.js, loadKicadBoard).
"""
import base64
import datetime
import gzip
import hashlib
import json
import os
import re
import tempfile
import time

# in web/index.html: `const KICAD_BOARD = null; /*@kicad-board*/` – replaced by the payload
MARKER = "null; /*@kicad-board*/"
FORMAT = 1
HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR_NAME = "kisi-kicad"
LOG_NAME = "kisi-kicad.log"
KEEP_DAYS = 14  # generated pages older than this are removed from the output folder
MAX_EXTRA = 64 * 1024 * 1024  # largest BOM / Touchstone file that is embedded


class PluginError(Exception):
    """A problem the user can fix (shown in KiCad and on an error page), not a bug."""


def build_info():
    """{version, viewer_sha256} written by tools/build_kicad_pcm.py; {} in the source tree."""
    try:
        with open(os.path.join(HERE, "kisi_build.json"), encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def version():
    return build_info().get("version", "dev")


def find_viewer():
    """The viewer page: viewer/kisi.html in the package, else dist/kisi.html of the source tree."""
    for p in (os.path.join(HERE, "viewer", "kisi.html"), os.path.join(HERE, "..", "dist", "kisi.html")):
        if os.path.isfile(p):
            return os.path.normpath(p)
    raise PluginError("the viewer page viewer/kisi.html is missing from the plugin – reinstall the kisi plugin")


def out_dir():
    return os.path.join(tempfile.gettempdir(), OUT_DIR_NAME)


# ---- paths ------------------------------------------------------------------------------------------------------
def board_paths(board_filename, project_dir, project_name, pathmod=os.path):
    """Where the files are: {pcb, pro_candidates, cfg, dir, stem}. KiCad's API names the board file and the
    project's folder (GetOpenDocuments); the board lies in that folder. `pathmod` is os.path, or ntpath/posixpath
    in tests of the other platform's paths."""
    name = (board_filename or "").strip()
    if not name or not project_dir:
        raise PluginError("the PCB editor has no saved board – save the board (File → Save), then click kisi again")
    pcb = name if pathmod.isabs(name) else pathmod.join(project_dir, name)
    pcb = pathmod.normpath(pcb)
    folder = pathmod.dirname(pcb)
    base = pathmod.basename(pcb)
    stem = re.sub(r"\.kicad_pcb$", "", base, flags=re.I)
    pros = [pathmod.join(folder, stem + ".kicad_pro")]
    if project_name and project_name != stem:
        pros.append(pathmod.join(pathmod.normpath(project_dir), project_name + ".kicad_pro"))
    return {"pcb": pcb, "pro_candidates": pros, "cfg": pathmod.join(folder, stem + ".kisi.json"), "dir": folder, "stem": stem}


def page_name(pcb_path, stem):
    """The generated page's file name: per board file (a second click replaces the page), short and safe."""
    h = hashlib.sha1(os.path.normcase(pcb_path).encode("utf-8", "surrogatepass")).hexdigest()[:8]
    safe = re.sub(r"[^\w.-]+", "_", stem, flags=re.UNICODE).strip("._") or "board"
    return f"{safe[:60]}-{h}.html"


# ---- writing: only the skeleton kisi.json next to the board, and files in our own output folder -----------------
def _guarded_write(path, data, allowed_dir=None, exclusive=False):
    """Write bytes, refusing anything that is not a .kisi.json or not inside allowed_dir: kisi never writes
    KiCad's files (.kicad_pcb, .kicad_pro, …)."""
    low = path.lower()
    inside = allowed_dir is not None and os.path.commonpath([os.path.abspath(allowed_dir), os.path.abspath(path)]) == os.path.abspath(allowed_dir)
    if low.endswith((".kicad_pcb", ".kicad_pro", ".kicad_sch", ".kicad_prl")) or not (low.endswith(".kisi.json") or inside):
        raise PluginError(f"refusing to write {path}")  # a bug, never expected
    if exclusive:
        with open(path, "xb") as f:  # never replaces an existing file
            f.write(data)
        return
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def skeleton(stem):
    """The minimal valid <board>.kisi.json – the same as the viewer's own start (web/src/prepare.js, prepSkeleton)."""
    return json.dumps({"kisi_config": 1, "board": stem + ".kicad_pcb"}, indent=2) + "\n"


def config_refs(text):
    """Files a kisi.json names relative to itself: BOMs of variants, Touchstone files of connector ends."""
    try:
        cfg = json.loads(text)
    except ValueError:
        return []
    out = []
    if not isinstance(cfg, dict):
        return out
    for v in (cfg.get("variants") or {}).values() if isinstance(cfg.get("variants"), dict) else []:
        if isinstance(v, dict) and isinstance(v.get("bom"), str):
            out.append(v["bom"])
    for e in (cfg.get("connector_ends") or {}).values() if isinstance(cfg.get("connector_ends"), dict) else []:
        ts = e.get("touchstone") if isinstance(e, dict) else None
        if isinstance(ts, dict) and isinstance(ts.get("file"), str):
            out.append(ts["file"])
    seen = []
    for r in out:
        if r and r not in seen and re.search(r"\.(csv|s\d+p)$", r, re.I):
            seen.append(r)
    return seen


def _gz(data):
    return base64.b64encode(gzip.compress(data, 6, mtime=0)).decode("ascii")


def _read(path):
    with open(path, "rb") as f:
        return f.read()


def _text(data):
    return data.decode("utf-8", errors="replace")


# ---- the page ---------------------------------------------------------------------------------------------------
def collect(paths, modified="unknown", kicad_version="", create_config=True, now=None):
    """Read the board's files and create the skeleton kisi.json if needed. Returns the payload for the viewer."""
    pcb = paths["pcb"]
    if not os.path.isfile(pcb):
        raise PluginError(f"the board file {pcb} does not exist – save the board (File → Save), then click kisi again")
    files, notes = [], []
    files.append({"name": os.path.basename(pcb), "gz": _gz(_read(pcb))})
    pro = next((p for p in paths["pro_candidates"] if os.path.isfile(p)), None)
    if pro:
        files.append({"name": os.path.basename(paths["stem"] + ".kicad_pro"), "gz": _gz(_read(pro))})
    else:
        notes.append("no project file (.kicad_pro) next to the board: net classes and their targets are missing")
    cfg_path, cfg_state, cfg_text = paths["cfg"], "none", None
    if os.path.isfile(cfg_path):
        cfg_text, cfg_state = _text(_read(cfg_path)), "loaded"
    elif create_config:
        try:
            _guarded_write(cfg_path, skeleton(paths["stem"]).encode("utf-8"), exclusive=True)
            cfg_text, cfg_state = skeleton(paths["stem"]), "created"
        except FileExistsError:  # appeared meanwhile: read it
            cfg_text, cfg_state = _text(_read(cfg_path)), "loaded"
        except OSError as e:
            notes.append(f"could not create {os.path.basename(cfg_path)} next to the board ({e.strerror or e}); the viewer starts without one")
    if cfg_text is not None:
        files.append({"name": os.path.basename(cfg_path), "gz": _gz(cfg_text.encode("utf-8"))})
        for ref in config_refs(cfg_text):
            p = os.path.normpath(os.path.join(paths["dir"], ref))
            if not os.path.isfile(p):
                notes.append(f"{ref}, named in the kisi.json, was not found next to it")
            elif os.path.getsize(p) > MAX_EXTRA:
                notes.append(f"{ref} is larger than 64 MB and was not embedded")
            else:
                files.append({"name": os.path.basename(p), "gz": _gz(_read(p))})
    saved = datetime.datetime.fromtimestamp(os.path.getmtime(pcb)).replace(microsecond=0).isoformat(sep=" ")
    return {
        "format": FORMAT,
        "source": "kicad",
        "board": paths["stem"],
        "dir": paths["dir"],
        "pcb_path": pcb,
        "config_path": cfg_path,
        "config": cfg_state,
        "saved": saved,
        "modified": modified if modified in ("modified", "unmodified") else "unknown",
        "kicad": kicad_version or "",
        "plugin": version(),
        "created": (now or datetime.datetime.now()).replace(microsecond=0).isoformat(sep=" "),
        "notes": notes,
        "files": files,
    }


def embed(template_text, payload):
    """The viewer page with the payload in place of the marker."""
    n = template_text.count(MARKER)
    if n != 1:
        raise PluginError(f"the viewer page has {n} hand-over markers instead of one – reinstall the kisi plugin")
    # JSON is valid JavaScript; "<" escaped so that nothing in it can end the script element
    js = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).replace("<", "\\u003c")
    return template_text.replace(MARKER, js + "; /*@kicad-board*/", 1)


def cleanup(folder, keep_days=KEEP_DAYS, now=None):
    """Remove generated pages older than keep_days (each is a few MB)."""
    now = now or time.time()
    removed = 0
    try:
        names = os.listdir(folder)
    except OSError:
        return 0
    for n in names:
        p = os.path.join(folder, n)
        if n.endswith(".html") and os.path.isfile(p) and now - os.path.getmtime(p) > keep_days * 86400:
            try:
                os.remove(p)
                removed += 1
            except OSError:
                pass
    return removed


def make_page(board_filename, project_dir, project_name, modified="unknown", kicad_version="", viewer=None, folder=None, create_config=True):
    """The whole job: returns (page path, payload). Raises PluginError for problems the user can fix."""
    paths = board_paths(board_filename, project_dir, project_name)
    payload = collect(paths, modified=modified, kicad_version=kicad_version, create_config=create_config)
    viewer = viewer or find_viewer()
    with open(viewer, encoding="utf-8") as f:
        html = embed(f.read(), payload)
    folder = folder or out_dir()
    os.makedirs(folder, exist_ok=True)
    cleanup(folder)
    page = os.path.join(folder, page_name(paths["pcb"], paths["stem"]))
    _guarded_write(page, html.encode("utf-8"), allowed_dir=folder)
    return page, payload


def error_page(message, log_path, folder=None):
    """A small page that says what went wrong (opened instead of the viewer)."""
    folder = folder or out_dir()
    os.makedirs(folder, exist_ok=True)
    esc = lambda s: str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    html = f"""<!doctype html>
<meta charset="utf-8"><title>kisi – could not open the board</title>
<style>body{{font:15px/1.5 system-ui,sans-serif;max-width:46em;margin:3em auto;padding:0 1em;color:#222}}
code{{background:#f2f2f2;padding:1px 4px;border-radius:3px}}@media(prefers-color-scheme:dark){{body{{background:#1b1b1b;color:#ddd}}code{{background:#333}}}}</style>
<h1>kisi could not open the board</h1>
<p>{esc(message)}</p>
<p>Details are in the plugin's log file: <code>{esc(log_path)}</code></p>
<p>kisi plugin {esc(version())} · the KiCad files were not changed.</p>
"""
    page = os.path.join(folder, "kisi-error.html")
    _guarded_write(page, html.encode("utf-8"), allowed_dir=folder)
    return page


# ---- log file (in the output folder) ----------------------------------------------------------------------------
class Log:
    """Appends to <temp>/kisi-kicad/kisi-kicad.log (kept below ~512 kB: the older half goes to .log.1)."""

    def __init__(self, folder=None):
        self.path = os.path.join(folder or out_dir(), LOG_NAME)
        try:
            os.makedirs(os.path.dirname(self.path), exist_ok=True)
            if os.path.isfile(self.path) and os.path.getsize(self.path) > 512 * 1024:
                os.replace(self.path, self.path + ".1")
        except OSError:
            pass

    def __call__(self, msg):
        line = f"{datetime.datetime.now().replace(microsecond=0).isoformat(sep=' ')} {msg}\n"
        try:
            with open(self.path, "a", encoding="utf-8") as f:
                f.write(line)
        except OSError:
            pass
