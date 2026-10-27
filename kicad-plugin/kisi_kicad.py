# SPDX-License-Identifier: GPL-3.0-only
"""kisi's KiCad plugin, the part without KiCad (standard library only, tested in kicad-plugin/tests).

Given the open board's file on disk, it
* takes over a <board>.kisi.json that the viewer downloaded (browsers without a save dialog for a folder) from
  the user's Downloads folder, when exactly one such file is newer than the one next to the board and checks out
  (import_download; the previous file is kept in analysis/),
* reads <board>.kicad_pcb, the project's .kicad_pro and <board>.kisi.json next to the board – and writes that
  kisi.json as a minimal skeleton when there is none,
* embeds them, with the BOMs and Touchstone files the kisi.json names, the plugin's CLI path and the Node.js status,
  into a copy of the viewer page shipped with the plugin (viewer/kisi.html, which has the marker below), and
* writes that page to analysis/<board>.kisi.html next to the board (the folder `kisi report` and `kisi export`
  use), or to a folder in the system's temporary directory when analysis/ cannot be written.

Next to the board it writes exactly <board>.kisi.json and, in analysis/, <board>.kisi.html and the backups
<board>.kisi.json.<time>.bak – the Guard refuses anything else; KiCad's files are never written.
The viewer loads the embedded files the way it loads dropped files (web/src/main.js, loadKicadBoard).
"""
import base64
import datetime
import gzip
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import tempfile
import time

# in web/index.html: `const KICAD_BOARD = null; /*@kicad-board*/` – replaced by the payload
MARKER = "null; /*@kicad-board*/"
FORMAT = 1
HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR_NAME = "kisi-kicad"
LOG_NAME = "kisi-kicad.log"
ANALYSIS = "analysis"
KEEP_DAYS = 14  # pages in the temporary folder older than this are removed
MAX_EXTRA = 64 * 1024 * 1024  # largest BOM / Touchstone file that is embedded
NODE_MIN = 18
KICAD_FILES = (".kicad_pcb", ".kicad_pro", ".kicad_sch", ".kicad_prl", ".kicad_sym", ".kicad_mod")
CREATE_NO_WINDOW = 0x08000000  # Windows: no console window for node (KiCad runs the plugin with pythonw.exe)
DOWNLOADS_GUID = "{374DE290-123F-4565-9164-39C4925E467B}"  # FOLDERID_Downloads


class PluginError(Exception):
    """A problem the user can fix (shown in KiCad and on an error page), not a bug."""


def build_info():
    """{version, viewer_sha256, cli_sha256} written by tools/build_kicad_pcm.py; {} in the source tree."""
    try:
        with open(os.path.join(HERE, "kisi_build.json"), encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def version():
    return build_info().get("version", "dev")


def _first_file(*cands):
    return next((os.path.normpath(p) for p in cands if os.path.isfile(p)), None)


def find_viewer():
    """The viewer page: viewer/kisi.html in the package, else dist/kisi.html of the source tree."""
    p = _first_file(os.path.join(HERE, "viewer", "kisi.html"), os.path.join(HERE, "..", "dist", "kisi.html"))
    if not p:
        raise PluginError("the viewer page viewer/kisi.html is missing from the plugin – reinstall the kisi plugin")
    return p


def find_cli():
    """The command line shipped with the plugin (cli/kisi.mjs), else dist/kisi.mjs of the source tree; or None."""
    return _first_file(os.path.join(HERE, "cli", "kisi.mjs"), os.path.join(HERE, "..", "dist", "kisi.mjs"))


def temp_dir():
    return os.path.join(tempfile.gettempdir(), OUT_DIR_NAME)


def _quiet():
    """subprocess options: no console window on Windows."""
    return {"creationflags": CREATE_NO_WINDOW} if os.name == "nt" else {}


# ---- Node.js ---------------------------------------------------------------------------------------------------
def find_node(run=subprocess.run, which=shutil.which, env=None, system=None):
    """{ok, path, version, reason}: a Node.js ≥ 18 for the plugin's kisi.mjs (agents run it; the import checks a
    downloaded kisi.json with it). `run`, `which`, `env`, `system` are replaced in tests."""
    env = os.environ if env is None else env
    system = system or platform.system()
    cands = [which("node")]
    if system == "Windows":  # KiCad's environment may lack the user's PATH additions
        for var, sub in (("ProgramFiles", "nodejs"), ("LOCALAPPDATA", os.path.join("Programs", "nodejs")), ("NVM_SYMLINK", "")):
            if env.get(var):
                cands.append(os.path.join(env[var], sub, "node.exe") if sub else os.path.join(env[var], "node.exe"))
    else:
        cands += ["/usr/local/bin/node", "/opt/homebrew/bin/node", "/usr/bin/node"]
    seen, reasons = [], []
    for c in cands:
        if not c or c in seen or not os.path.isfile(c):
            continue
        seen.append(c)
        try:
            out = run([c, "--version"], capture_output=True, text=True, timeout=5, **_quiet())
        except (OSError, subprocess.SubprocessError) as e:
            reasons.append(f"{c} --version failed ({e})")
            continue
        m = re.match(r"\s*v?(\d+)\.(\d+)\.(\d+)", out.stdout or "")
        if not m:
            reasons.append(f"{c} --version printed {(out.stdout or out.stderr or '').strip()[:60]!r}")
            continue
        v = ".".join(m.groups())
        if int(m.group(1)) < NODE_MIN:
            reasons.append(f"Node.js {v} at {c} is older than {NODE_MIN}")
            continue
        return {"ok": True, "path": c, "version": v, "reason": ""}
    # the first problem: the node on the PATH, where there is one
    return {"ok": False, "path": seen[0] if seen else "", "version": "", "reason": reasons[0] if reasons else "Node.js was not found"}


def config_check(node, cli, pcb, cfg, run=subprocess.run, timeout=90):
    """kisi config-check of a kisi.json for the board with the plugin's CLI: (True/False, messages), or (None, why)
    when it could not run."""
    if not (node and node.get("ok") and cli):
        return None, "Node.js or the plugin's kisi.mjs is not available"
    try:
        out = run([node["path"], cli, "config-check", pcb, "--config", cfg], capture_output=True, text=True,
                  timeout=timeout, encoding="utf-8", errors="replace", **_quiet())
        r = json.loads(out.stdout)
    except (OSError, subprocess.SubprocessError, ValueError) as e:
        return None, f"kisi config-check did not run ({type(e).__name__}: {e})"
    return bool(r.get("ok")), [str(x) for x in (r.get("errors") or [])][:3]


# ---- paths ------------------------------------------------------------------------------------------------------
def board_paths(board_filename, project_dir, project_name, pathmod=os.path):
    """Where the files are: {pcb, pro_candidates, cfg, dir, stem, analysis, page}. KiCad's API names the board file
    and the project's folder (GetOpenDocuments); the board lies in that folder. `pathmod` is os.path, or
    ntpath/posixpath in tests of the other platform's paths."""
    name = (board_filename or "").strip()
    if not name or not project_dir:
        raise PluginError("the PCB editor has no saved board – save the board (File → Save), then click kisi again")
    pcb = name if pathmod.isabs(name) else pathmod.join(project_dir, name)
    pcb = pathmod.normpath(pcb)
    folder = pathmod.dirname(pcb)
    stem = re.sub(r"\.kicad_pcb$", "", pathmod.basename(pcb), flags=re.I)
    pros = [pathmod.join(folder, stem + ".kicad_pro")]
    if project_name and project_name != stem:
        pros.append(pathmod.join(pathmod.normpath(project_dir), project_name + ".kicad_pro"))
    analysis = pathmod.join(folder, ANALYSIS)
    return {"pcb": pcb, "pro_candidates": pros, "cfg": pathmod.join(folder, stem + ".kisi.json"), "dir": folder, "stem": stem,
            "analysis": analysis, "page": pathmod.join(analysis, stem + ".kisi.html")}


def page_name(pcb_path, stem):
    """The page's file name in the temporary folder: per board file, short and safe."""
    h = hashlib.sha1(os.path.normcase(pcb_path).encode("utf-8", "surrogatepass")).hexdigest()[:8]
    safe = re.sub(r"[^\w.-]+", "_", stem, flags=re.UNICODE).strip("._") or "board"
    return f"{safe[:60]}-{h}.html"


# ---- writing: exactly these files ---------------------------------------------------------------------------------
class Guard:
    """The only way the plugin writes: next to the board exactly <board>.kisi.json, in analysis/ exactly
    <board>.kisi.html and the backups <board>.kisi.json.<YYYYmmdd-HHMMSS>[-n].bak, and anything in its own folder
    in the temporary directory; never a KiCad file."""

    def __init__(self, paths, temp=None, pathmod=os.path):
        self.p, self.pm = paths, pathmod
        self.temp = temp or temp_dir()
        key = lambda x: pathmod.normcase(pathmod.abspath(x)) if pathmod is os.path else pathmod.normcase(x)
        self.key = key
        self.exact = {key(paths["cfg"]), key(paths["page"])}
        self.backup_re = re.compile(re.escape(paths["stem"]) + r"\.kisi\.json\.\d{8}-\d{6}(-\d+)?\.bak", re.I)

    def allowed(self, path):
        pm, k = self.pm, self.key(path)
        if path.lower().endswith(KICAD_FILES):
            return False
        if k in self.exact:
            return True
        if pm.dirname(k) == self.key(self.p["analysis"]) and self.backup_re.fullmatch(pm.basename(path)):
            return True
        t = self.key(self.temp)
        return k.startswith(t.rstrip("\\/") + pm.sep)

    def write(self, path, data, exclusive=False):
        if not self.allowed(path):
            raise PluginError(f"refusing to write {path}")  # a bug, never expected
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if exclusive:
            with open(path, "xb") as f:  # never replaces an existing file
                f.write(data)
            return
        tmp = path + ".kisi-tmp"
        try:
            with open(tmp, "wb") as f:
                f.write(data)
            os.replace(tmp, path)
        finally:
            if os.path.exists(tmp):
                os.remove(tmp)

    def backup_path(self, now=None):
        now = now or datetime.datetime.now()
        base = os.path.join(self.p["analysis"], f"{self.p['stem']}.kisi.json.{now:%Y%m%d-%H%M%S}")
        p, n = base + ".bak", 1
        while os.path.exists(p):
            p, n = f"{base}-{n}.bak", n + 1
        return p


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


# ---- the Downloads folder and the take-over of a downloaded kisi.json --------------------------------------------
def _win_known_downloads():
    """FOLDERID_Downloads through SHGetKnownFolderPath (Windows only)."""
    import ctypes
    from ctypes import wintypes

    class GUID(ctypes.Structure):
        _fields_ = [("Data1", wintypes.DWORD), ("Data2", wintypes.WORD), ("Data3", wintypes.WORD), ("Data4", ctypes.c_ubyte * 8)]

    g = GUID(0x374DE290, 0x123F, 0x4565, (ctypes.c_ubyte * 8)(0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B))
    out = ctypes.c_wchar_p()
    f = ctypes.windll.shell32.SHGetKnownFolderPath
    f.argtypes = [ctypes.POINTER(GUID), wintypes.DWORD, wintypes.HANDLE, ctypes.POINTER(ctypes.c_wchar_p)]
    try:
        return out.value if f(ctypes.byref(g), 0, None, ctypes.byref(out)) == 0 else None
    finally:
        ctypes.windll.ole32.CoTaskMemFree(out)


def _win_registry_downloads():
    """The Downloads folder from HKCU\\…\\Explorer\\User Shell Folders (REG_EXPAND_SZ), Windows only."""
    import winreg
    key = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders"
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key) as k:
        return winreg.QueryValueEx(k, DOWNLOADS_GUID)[0]


def _expand_win(value, env):
    return re.sub(r"%([^%]+)%", lambda m: env.get(m.group(1), env.get(m.group(1).upper(), m.group(0))), value)


def downloads_dir(system=None, env=None, home=None, run=subprocess.run, known=None, registry=None, isdir=os.path.isdir):
    """The user's Downloads folder, or None. Windows: the known folder (SHGetKnownFolderPath), else the registry's
    User Shell Folders entry, else %USERPROFILE%\\Downloads; Linux: `xdg-user-dir DOWNLOAD`, else
    user-dirs.dirs, else ~/Downloads; macOS: ~/Downloads. The arguments are replaced in tests."""
    system = system or platform.system()
    env = os.environ if env is None else env
    home = home or os.path.expanduser("~")
    cands = []
    if system == "Windows":
        for getter in (known or _win_known_downloads, registry or _win_registry_downloads):
            try:
                v = getter()
            except Exception:
                v = None
            if v:
                cands.append(_expand_win(v, env))
        if env.get("USERPROFILE"):
            cands.append(env["USERPROFILE"] + "\\Downloads")
    elif system == "Darwin":
        cands.append(os.path.join(home, "Downloads"))
    else:
        try:
            out = run(["xdg-user-dir", "DOWNLOAD"], capture_output=True, text=True, timeout=5)
            v = (out.stdout or "").strip()
            if v and os.path.normpath(v) != os.path.normpath(home):
                cands.append(v)
        except (OSError, subprocess.SubprocessError):
            pass
        conf = os.path.join(env.get("XDG_CONFIG_HOME") or os.path.join(home, ".config"), "user-dirs.dirs")
        try:
            with open(conf, encoding="utf-8") as f:
                m = re.search(r'^XDG_DOWNLOAD_DIR="([^"]+)"', f.read(), re.M)
            if m:
                cands.append(m.group(1).replace("$HOME", home))
        except OSError:
            pass
        cands.append(os.path.join(home, "Downloads"))
    return next((c for c in cands if c and isdir(c)), None)


def download_name_re(stem):
    """<board>.kisi.json as the browsers save it, duplicates included: "x.kisi(1).json" (Firefox), "x.kisi (1).json"
    (Chrome, Edge), "x.kisi-1.json" (Safari)."""
    return re.compile(re.escape(stem) + r"\.kisi(?:\s?\(\d+\)|-\d+)?\.json", re.I)


def download_candidates(folder, stem, listdir=os.listdir):
    rx = download_name_re(stem)
    try:
        names = listdir(folder)
    except OSError:
        return []
    return sorted(os.path.join(folder, n) for n in names if rx.fullmatch(n) and os.path.isfile(os.path.join(folder, n)))


def check_download(text, stem):
    """Why a downloaded kisi.json is not taken over, or None when it looks right: JSON, kisi_config 1, the board's name."""
    try:
        cfg = json.loads(text)
    except ValueError as e:
        return f"not valid JSON ({e.msg}, line {e.lineno})"
    if not isinstance(cfg, dict):
        return "not a kisi.json (no JSON object)"
    if cfg.get("kisi_config") != 1:
        return f"kisi_config is {cfg.get('kisi_config')!r}, not 1"
    want = stem + ".kicad_pcb"
    if str(cfg.get("board", "")).casefold() != want.casefold():
        return f"written for the board {cfg.get('board')!r}, not {want}"
    return None


def _when(t):
    return datetime.datetime.fromtimestamp(t).replace(microsecond=0).isoformat(sep=" ")


def import_download(paths, guard, downloads, node=None, cli=None, run=subprocess.run, now=None):
    """Take over a downloaded <board>.kisi.json: exactly one candidate in `downloads` newer than the board's
    kisi.json (or any, when there is none), valid, for this board, and accepted by `kisi config-check` when Node.js
    and the plugin's CLI are there. The old file goes to analysis/ as a backup, the download moves next to the
    board. Anything doubtful: nothing is touched, the reason is returned for the viewer's card."""
    info = {"state": "none", "folder": downloads or "", "from": "", "time": "", "backup": "", "reasons": [], "checked": ""}
    if not downloads:
        return info
    cfg = paths["cfg"]
    ref = os.path.getmtime(cfg) if os.path.isfile(cfg) else None
    own = os.path.normcase(os.path.abspath(cfg))  # a browser may have saved into the board's folder itself
    newer = [c for c in download_candidates(downloads, paths["stem"])
             if os.path.normcase(os.path.abspath(c)) != own and (ref is None or os.path.getmtime(c) > ref + 1)]
    if not newer:
        return info
    if len(newer) > 1:
        info.update(state="ambiguous", reasons=[f"{len(newer)} files in {downloads} are newer than the kisi.json next to the board: "
                                                + ", ".join(os.path.basename(c) for c in newer) + " – move the right one next to the board"])
        return info
    src = newer[0]
    info.update({"from": src, "time": _when(os.path.getmtime(src))})
    try:
        with open(src, "rb") as f:
            data = f.read()
        text = data.decode("utf-8")
    except (OSError, UnicodeDecodeError) as e:
        info.update(state="invalid", reasons=[f"could not read it ({e})"])
        return info
    why = check_download(text, paths["stem"])
    if why:
        info.update(state="invalid", reasons=[why])
        return info
    ok, msg = config_check(node, cli, paths["pcb"], src, run=run)
    if ok is False:
        info.update(state="invalid", reasons=["kisi config-check: " + ("; ".join(msg) or "not ok")])
        return info
    info["checked"] = "kisi config-check ok" if ok else f"format checked; {msg}"
    if os.path.isfile(cfg):
        bak = guard.backup_path(now)
        try:
            with open(cfg, "rb") as f:
                guard.write(bak, f.read(), exclusive=True)
        except OSError as e:
            info.update(state="invalid", reasons=[f"could not keep the previous kisi.json in {paths['analysis']} ({e.strerror or e}) – nothing changed"])
            return info
        info["backup"] = bak
    try:
        guard.write(cfg, data)
    except OSError as e:
        info.update(state="invalid", reasons=[f"could not write {cfg} ({e.strerror or e})"])
        return info
    try:
        os.remove(src)
    except OSError as e:
        info["reasons"].append(f"the download could not be removed ({e.strerror or e}); it is older than the kisi.json now and stays unused")
    info["state"] = "taken"
    return info


# ---- the page ---------------------------------------------------------------------------------------------------
def _gz(data):
    return base64.b64encode(gzip.compress(data, 6, mtime=0)).decode("ascii")


def _read(path):
    with open(path, "rb") as f:
        return f.read()


def _text(data):
    return data.decode("utf-8", errors="replace")


def collect(paths, guard, modified="unknown", kicad_version="", create_config=True, now=None, downloads=None, node=None, cli=None,
            run=subprocess.run):
    """Take over a downloaded kisi.json, read the board's files, create the skeleton kisi.json if needed.
    Returns the payload for the viewer (without the page's own location)."""
    pcb = paths["pcb"]
    if not os.path.isfile(pcb):
        raise PluginError(f"the board file {pcb} does not exist – save the board (File → Save), then click kisi again")
    imp = import_download(paths, guard, downloads, node=node, cli=cli, run=run, now=now)
    files, notes = [], []
    files.append({"name": os.path.basename(pcb), "gz": _gz(_read(pcb))})
    pro = next((p for p in paths["pro_candidates"] if os.path.isfile(p)), None)
    if pro:
        files.append({"name": os.path.basename(paths["stem"] + ".kicad_pro"), "gz": _gz(_read(pro))})
    else:
        notes.append("no project file (.kicad_pro) next to the board: net classes and their targets are missing")
    cfg_path, cfg_state, cfg_text = paths["cfg"], "none", None
    if os.path.isfile(cfg_path):
        cfg_text, cfg_state = _text(_read(cfg_path)), "imported" if imp["state"] == "taken" else "loaded"
    elif create_config:
        try:
            guard.write(cfg_path, skeleton(paths["stem"]).encode("utf-8"), exclusive=True)
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
    return {
        "format": FORMAT,
        "source": "kicad",
        "board": paths["stem"],
        "dir": paths["dir"],
        "pcb_path": pcb,
        "config_path": cfg_path,
        "config": cfg_state,
        "import": imp,
        "saved": _when(os.path.getmtime(pcb)),
        "modified": modified if modified in ("modified", "unmodified") else "unknown",
        "kicad": kicad_version or "",
        "plugin": version(),
        "created": (now or datetime.datetime.now()).replace(microsecond=0).isoformat(sep=" "),
        "cli": {"path": cli or "", "node": node or {"ok": False, "path": "", "version": "", "reason": "not checked"}},
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
    """Remove generated pages in the temporary folder older than keep_days (each is a few MB)."""
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


def make_page(board_filename, project_dir, project_name, modified="unknown", kicad_version="", viewer=None, temp=None,
              create_config=True, downloads="auto", node="auto", cli="auto", run=subprocess.run, now=None):
    """The whole job: returns (page path, payload). Raises PluginError for problems the user can fix.
    downloads/node/cli: "auto" finds them (the user's Downloads folder, Node.js, the plugin's kisi.mjs);
    tests pass their own or None."""
    paths = board_paths(board_filename, project_dir, project_name)
    temp = temp or temp_dir()
    guard = Guard(paths, temp)
    cli = find_cli() if cli == "auto" else cli
    node = find_node(run=run) if node == "auto" else node
    downloads = downloads_dir() if downloads == "auto" else downloads
    payload = collect(paths, guard, modified=modified, kicad_version=kicad_version, create_config=create_config, now=now,
                      downloads=downloads, node=node, cli=cli, run=run)
    viewer = viewer or find_viewer()
    with open(viewer, encoding="utf-8") as f:
        template = f.read()
    # analysis/<board>.kisi.html next to the board; the temporary folder when analysis/ cannot be written
    payload["page"] = {"path": paths["page"], "where": "analysis", "reason": ""}
    try:
        guard.write(paths["page"], embed(template, payload).encode("utf-8"))
        return paths["page"], payload
    except OSError as e:
        reason = f"{paths['analysis']} could not be written ({e.strerror or e})"
    os.makedirs(temp, exist_ok=True)
    cleanup(temp)
    page = os.path.join(temp, page_name(paths["pcb"], paths["stem"]))
    payload["page"] = {"path": page, "where": "temp", "reason": reason}
    guard.write(page, embed(template, payload).encode("utf-8"))
    return page, payload


def error_page(message, log_path, folder=None):
    """A small page that says what went wrong (opened instead of the viewer), in the temporary folder."""
    folder = folder or temp_dir()
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
    tmp = page + ".kisi-tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(html)
    os.replace(tmp, page)
    return page


# ---- log file (in the temporary folder) -------------------------------------------------------------------------
class Log:
    """Appends to <temp>/kisi-kicad/kisi-kicad.log (kept below ~512 kB: the older half goes to .log.1)."""

    def __init__(self, folder=None):
        self.path = os.path.join(folder or temp_dir(), LOG_NAME)
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
