# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the plugin without KiCad: the page (kisi_kicad.py) and the action with a stand-in for kicad-python
(kisi_action.py); with the real kicad-python, when installed, against its classes and a fake API connection.

Run: python3 -m unittest discover -s kicad-plugin/tests -v
"""
import base64, pathlib, gzip, hashlib, importlib, io, json, ntpath, os, posixpath, re, shutil, sys, tempfile, time, types, unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import PureWindowsPath

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGIN = os.path.dirname(HERE)
sys.path.insert(0, PLUGIN)
import kisi_kicad as core  # noqa: E402

TEMPLATE = '<!doctype html><script>const SAMPLES = {};\nconst KICAD_BOARD = null; /*@kicad-board*/\n</script><span class="ver">v9.9.9</span>'
PCB = '(kicad_pcb (version 20241229) (generator "pcbnew")\n  (general (thickness 1.6))\n)\n'
PRO = '{"net_settings": {"classes": []}}\n'


def payload_of(html):
    m = re.search(r"const KICAD_BOARD = (\{.*?\}); /\*@kicad-board\*/", html, re.S)
    assert m, "no payload in the page"
    return json.loads(m.group(1))


def unpack(payload):
    return {f["name"]: gzip.decompress(base64.b64decode(f["gz"])).decode("utf-8") for f in payload["files"]}


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


class Board:
    """A board folder in a temp dir: a name with spaces and umlauts, as on a German Windows desktop."""

    def __init__(self, folder="Mein Prüf-Board ä ö ü ß", stem="Prüf Board", pro=True, cfg=None):
        self.root = tempfile.mkdtemp(prefix="kisi-plugin-test-")
        self.dir = os.path.join(self.root, folder)
        os.makedirs(self.dir)
        self.stem = stem
        self.pcb = os.path.join(self.dir, stem + ".kicad_pcb")
        pathlib.Path(self.pcb).write_text(encoding="utf-8", data=PCB)
        if pro:
            pathlib.Path(os.path.join(self.dir, stem + ".kicad_pro")).write_text(encoding="utf-8", data=PRO)
        self.cfg = os.path.join(self.dir, stem + ".kisi.json")
        if cfg is not None:
            pathlib.Path(self.cfg).write_text(encoding="utf-8", data=cfg)
        self.viewer = os.path.join(self.root, "kisi.html")
        pathlib.Path(self.viewer).write_text(encoding="utf-8", data=TEMPLATE)
        self.out = os.path.join(self.root, "out")

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


class PageTests(unittest.TestCase):
    def setUp(self):
        self.b = Board()

    def tearDown(self):
        self.b.close()

    def make(self, **kw):
        return core.make_page(self.b.stem + ".kicad_pcb", self.b.dir, self.b.stem, viewer=self.b.viewer, folder=self.b.out, **kw)

    def test_page_with_skeleton(self):
        before = {f: digest(os.path.join(self.b.dir, f)) for f in os.listdir(self.b.dir)}
        page, p = self.make(kicad_version="10.0.7")
        self.assertTrue(page.startswith(self.b.out) and page.endswith(".html"))
        html = pathlib.Path(page).read_text(encoding="utf-8")
        self.assertEqual(html.count("/*@kicad-board*/"), 1)
        q = payload_of(html)
        self.assertEqual(q, json.loads(json.dumps(p)))
        self.assertEqual((q["format"], q["source"], q["board"], q["config"], q["modified"], q["kicad"]), (1, "kicad", self.b.stem, "created", "unknown", "10.0.7"))
        self.assertEqual(q["dir"], self.b.dir)
        files = unpack(q)
        self.assertEqual(sorted(files), sorted([self.b.stem + ".kicad_pcb", self.b.stem + ".kicad_pro", self.b.stem + ".kisi.json"]))
        self.assertEqual(files[self.b.stem + ".kicad_pcb"], PCB)
        # the skeleton: next to the board, the same as the viewer's own (prepare.js prepSkeleton)
        self.assertEqual(json.loads(pathlib.Path(self.b.cfg).read_text(encoding="utf-8")), {"kisi_config": 1, "board": self.b.stem + ".kicad_pcb"})
        self.assertEqual(files[self.b.stem + ".kisi.json"], pathlib.Path(self.b.cfg).read_text(encoding="utf-8"))
        # KiCad's files untouched; the only new file next to the board is the kisi.json
        after = {f: digest(os.path.join(self.b.dir, f)) for f in os.listdir(self.b.dir)}
        self.assertEqual(set(after) - set(before), {self.b.stem + ".kisi.json"})
        for f, h in before.items():
            self.assertEqual(after[f], h, f)

    def test_existing_config_is_read_not_written(self):
        cfg = json.dumps({"kisi_config": 1, "board": self.b.stem + ".kicad_pcb",
                          "variants": {"A": {"bom": "bom A.csv"}, "B": {"bom": "missing.csv"}},
                          "connector_ends": {"J1": {"touchstone": {"file": "sub/cable.s4p"}}}}, indent=1)
        pathlib.Path(self.b.cfg).write_text(encoding="utf-8", data=cfg)
        pathlib.Path(os.path.join(self.b.dir, "bom A.csv")).write_text(encoding="utf-8", data="Reference\nR1\n")
        os.makedirs(os.path.join(self.b.dir, "sub"))
        pathlib.Path(os.path.join(self.b.dir, "sub", "cable.s4p")).write_text(encoding="utf-8", data="# GHz S MA R 50\n")
        t0 = os.path.getmtime(self.b.cfg)
        time.sleep(0.01)
        _, p = self.make()
        self.assertEqual(p["config"], "loaded")
        self.assertEqual(os.path.getmtime(self.b.cfg), t0)
        files = unpack(p)
        self.assertEqual(files[self.b.stem + ".kisi.json"], cfg)
        self.assertIn("bom A.csv", files)
        self.assertIn("cable.s4p", files)
        self.assertTrue(any("missing.csv" in n for n in p["notes"]), p["notes"])

    def test_invalid_config_is_passed_on(self):
        pathlib.Path(self.b.cfg).write_text(encoding="utf-8", data="{ not json")
        _, p = self.make()
        self.assertEqual((p["config"], unpack(p)[self.b.stem + ".kisi.json"]), ("loaded", "{ not json"))

    def test_modified_flag(self):
        for state, want in (("modified", "modified"), ("unmodified", "unmodified"), ("unknown", "unknown"), ("odd", "unknown")):
            _, p = self.make(modified=state)
            self.assertEqual(p["modified"], want)

    def test_no_project_file(self):
        os.remove(os.path.join(self.b.dir, self.b.stem + ".kicad_pro"))
        _, p = self.make()
        self.assertEqual(sorted(unpack(p)), sorted([self.b.stem + ".kicad_pcb", self.b.stem + ".kisi.json"]))
        self.assertTrue(any("project file" in n for n in p["notes"]))

    def test_project_named_differently(self):
        os.rename(os.path.join(self.b.dir, self.b.stem + ".kicad_pro"), os.path.join(self.b.dir, "proj.kicad_pro"))
        _, p = core.make_page(self.b.stem + ".kicad_pcb", self.b.dir, "proj", viewer=self.b.viewer, folder=self.b.out)
        self.assertIn(self.b.stem + ".kicad_pro", unpack(p))  # named after the board, as the viewer expects

    def test_unsaved_board(self):
        for name, folder in (("", self.b.dir), ("Untitled.kicad_pcb", self.b.dir), ("x.kicad_pcb", "")):
            with self.assertRaises(core.PluginError) as cm:
                core.make_page(name, folder, "", viewer=self.b.viewer, folder=self.b.out)
            self.assertIn("save the board", str(cm.exception))
        self.assertFalse(os.path.exists(os.path.join(self.b.dir, "Untitled.kisi.json")))

    def test_read_only_folder(self):
        if os.name == "nt" or os.geteuid() == 0:
            self.skipTest("permissions are not enforced here")
        os.chmod(self.b.dir, 0o555)
        try:
            _, p = self.make()
        finally:
            os.chmod(self.b.dir, 0o755)
        self.assertEqual(p["config"], "none")
        self.assertTrue(any("could not create" in n for n in p["notes"]))

    def test_second_click_replaces_the_page(self):
        p1, _ = self.make()
        p2, q = self.make()
        self.assertEqual(p1, p2)
        self.assertEqual(q["config"], "loaded")
        self.assertEqual([f for f in os.listdir(self.b.out) if f.endswith(".html")], [os.path.basename(p1)])

    def test_marker_problems(self):
        for t in ("<html>no marker</html>", TEMPLATE + TEMPLATE):
            with self.assertRaises(core.PluginError):
                core.embed(t, {"files": []})

    def test_script_cannot_be_closed(self):
        html = core.embed(TEMPLATE, {"notes": ["</script><script>alert(1)</script>"], "files": []})
        self.assertNotIn("</script><script>alert", html)
        self.assertEqual(payload_of(html)["notes"][0], "</script><script>alert(1)</script>")

    def test_guard_refuses_kicad_files(self):
        for name in ("x.kicad_pcb", "x.kicad_pro", "X.KICAD_PCB", "notes.txt"):
            with self.assertRaises(core.PluginError):
                core._guarded_write(os.path.join(self.b.dir, name), b"x")
        with self.assertRaises(core.PluginError):  # even inside the output folder
            core._guarded_write(os.path.join(self.b.out, "a.kicad_pcb"), b"x", allowed_dir=self.b.out)
        with self.assertRaises(FileExistsError):  # the skeleton never replaces a file
            pathlib.Path(self.b.cfg).write_text(encoding="utf-8", data="{}")
            core._guarded_write(self.b.cfg, b"{}", exclusive=True)

    def test_cleanup_old_pages(self):
        os.makedirs(self.b.out)
        old, new, other = (os.path.join(self.b.out, n) for n in ("old.html", "new.html", "kisi-kicad.log"))
        for p in (old, new, other):
            pathlib.Path(p).write_text(encoding="utf-8", data="x")
        t = time.time() - 30 * 86400
        os.utime(old, (t, t))
        os.utime(other, (t, t))
        self.assertEqual(core.cleanup(self.b.out), 1)
        self.assertEqual(sorted(os.listdir(self.b.out)), ["kisi-kicad.log", "new.html"])

    def test_error_page(self):
        page = core.error_page("the board <b>x</b> & more", "C:\\Temp\\kisi-kicad\\kisi-kicad.log", folder=self.b.out)
        html = pathlib.Path(page).read_text(encoding="utf-8")
        self.assertIn("the board &lt;b&gt;x&lt;/b&gt; &amp; more", html)
        self.assertIn("C:\\Temp\\kisi-kicad\\kisi-kicad.log", html)


class PathTests(unittest.TestCase):
    """Windows paths (the maintainer's platform) with the Windows path rules (ntpath), also on Linux CI."""

    def test_windows_paths(self):
        p = core.board_paths("Mein Board.kicad_pcb", "C:\\Users\\Jürgen\\Documents\\KiCad Projekte\\Mein Board", "Mein Board", pathmod=ntpath)
        self.assertEqual(p["pcb"], "C:\\Users\\Jürgen\\Documents\\KiCad Projekte\\Mein Board\\Mein Board.kicad_pcb")
        self.assertEqual(p["cfg"], "C:\\Users\\Jürgen\\Documents\\KiCad Projekte\\Mein Board\\Mein Board.kisi.json")
        self.assertEqual(p["dir"], "C:\\Users\\Jürgen\\Documents\\KiCad Projekte\\Mein Board")
        self.assertEqual(p["pro_candidates"], ["C:\\Users\\Jürgen\\Documents\\KiCad Projekte\\Mein Board\\Mein Board.kicad_pro"])
        # KiCad on Windows may send forward slashes in the project path
        q = core.board_paths("b.kicad_pcb", "C:/Users/Jürgen/Board Ü", "proj", pathmod=ntpath)
        self.assertEqual(q["pcb"], "C:\\Users\\Jürgen\\Board Ü\\b.kicad_pcb")
        self.assertEqual(q["pro_candidates"][1], "C:\\Users\\Jürgen\\Board Ü\\proj.kicad_pro")
        unc = core.board_paths("b.kicad_pcb", "\\\\server\\share\\Board", "b", pathmod=ntpath)
        self.assertEqual(unc["cfg"], "\\\\server\\share\\Board\\b.kisi.json")

    def test_posix_paths(self):
        p = core.board_paths("b.kicad_pcb", "/home/jürgen/My Boards/b/", "b", pathmod=posixpath)
        self.assertEqual(p["pcb"], "/home/jürgen/My Boards/b/b.kicad_pcb")

    def test_page_name_and_uri(self):
        n = core.page_name("C:\\Users\\Jürgen\\Mein Board\\Mein Board.kicad_pcb", "Mein Board")
        self.assertRegex(n, r"^Mein_Board-[0-9a-f]{8}\.html$")
        self.assertNotEqual(n, core.page_name("C:\\other\\Mein Board.kicad_pcb", "Mein Board"))
        self.assertRegex(core.page_name("/x/Prüf Böard.kicad_pcb", "Prüf Böard"), r"^Prüf_Böard-[0-9a-f]{8}\.html$")
        self.assertRegex(core.page_name("/x/...", "..."), r"^board-[0-9a-f]{8}\.html$")
        # the URL the browser gets for a page in a Windows temp folder with an umlaut in the user name
        uri = PureWindowsPath("C:\\Users\\Jürgen\\AppData\\Local\\Temp\\kisi-kicad\\Mein_Board-0123abcd.html").as_uri()
        self.assertEqual(uri, "file:///C:/Users/J%C3%BCrgen/AppData/Local/Temp/kisi-kicad/Mein_Board-0123abcd.html")

    def test_config_refs(self):
        self.assertEqual(core.config_refs('{"variants": {"a": {"bom": "x.csv"}, "b": {"bom": "x.csv"}}, "connector_ends": {"J": {"touchstone": {"file": "c.s2p"}}}}'), ["x.csv", "c.s2p"])
        for t in ("[]", "nope", '{"variants": []}', '{"variants": {"a": {"bom": "x.exe"}}}'):
            self.assertEqual(core.config_refs(t), [])


# ---- the action (kisi_action.py) with a stand-in for kicad-python ---------------------------------------------
def fake_kipy(board_filename, project_path, project_name, fail=None):
    """Modules kipy and kipy.proto.common.commands.editor_commands_pb2 with the attributes kisi_action.py uses
    (the same names as kicad-python 0.8.0: KiCad(client_name=, timeout_ms=), get_version(), get_board(),
    Board.document.{board_filename, project.name, project.path}, Board.client)."""
    mod = types.ModuleType("kipy")

    class KiCad:
        def __init__(self, socket_path=None, client_name=None, kicad_token=None, timeout_ms=2000):
            if fail == "connect":
                raise ConnectionError("Failed to connect to KiCad: no socket")
            self.client_name = client_name

        def get_version(self):
            return types.SimpleNamespace(major=10, minor=0, patch=7, full_version="10.0.7")

        def get_board(self):
            if fail == "noboard":
                raise RuntimeError("Expected to be able to retrieve at least one board")
            doc = types.SimpleNamespace(board_filename=board_filename, project=types.SimpleNamespace(name=project_name, path=project_path))
            return types.SimpleNamespace(document=doc, client=None, name=board_filename)

    mod.KiCad = KiCad
    return {"kipy": mod}


class ActionTests(unittest.TestCase):
    def setUp(self):
        self.b = Board()
        self.tmp = os.path.join(self.b.root, "tmp")
        os.makedirs(self.tmp)
        self.env = {k: os.environ.get(k) for k in ("TMPDIR", "TEMP", "TMP", "KISI_NO_BROWSER")}
        os.environ.update({"TMPDIR": self.tmp, "TEMP": self.tmp, "TMP": self.tmp, "KISI_NO_BROWSER": "1"})
        tempfile.tempdir = None
        self.saved_modules = {k: sys.modules.get(k) for k in ("kipy",)}
        import kisi_action
        self.action = importlib.reload(kisi_action)
        self.find_viewer = core.find_viewer
        core.find_viewer = lambda: self.b.viewer

    def tearDown(self):
        core.find_viewer = self.find_viewer
        for k, v in self.saved_modules.items():
            if v is None:
                sys.modules.pop(k, None)
            else:
                sys.modules[k] = v
        for k, v in self.env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        tempfile.tempdir = None
        self.b.close()

    def run_action(self, argv, modules=None):
        sys.modules.update(modules or {})
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            rc = self.action.main(argv)
        return rc, out.getvalue(), err.getvalue()

    def test_from_kicad(self):
        rc, out, err = self.run_action([], fake_kipy(self.b.stem + ".kicad_pcb", self.b.dir, self.b.stem))
        self.assertEqual((rc, err), (0, ""))
        page = out.strip()
        self.assertTrue(page.startswith(os.path.join(self.tmp, "kisi-kicad")), page)
        q = payload_of(pathlib.Path(page).read_text(encoding="utf-8"))
        self.assertEqual((q["board"], q["config"], q["modified"], q["kicad"]), (self.b.stem, "created", "unknown", "10.0.7 (10.0.7)"))
        log = pathlib.Path(os.path.join(self.tmp, "kisi-kicad", "kisi-kicad.log")).read_text(encoding="utf-8")
        self.assertIn("KiCad 10.0.7", log)
        self.assertIn("kisi.json created", log)

    def test_without_kicad(self):
        rc, out, err = self.run_action(["--pcb", self.b.pcb])
        self.assertEqual(rc, 0, err)
        self.assertEqual(payload_of(pathlib.Path(out.strip()).read_text(encoding="utf-8"))["kicad"], "")

    def test_errors_go_to_stderr_and_an_error_page(self):
        cases = [(fake_kipy("", self.b.dir, ""), "save the board"),
                 (fake_kipy("x.kicad_pcb", self.b.dir, "x", fail="noboard"), "is a board open"),
                 (fake_kipy("x.kicad_pcb", self.b.dir, "x", fail="connect"), "could not get the open board")]
        for mods, want in cases:
            rc, out, err = self.run_action([], mods)
            self.assertEqual(rc, 1)
            self.assertIn(want, err)
            self.assertTrue(err.startswith("kisi: ") and "kisi-kicad.log" in err, err)
            self.assertTrue(os.path.isfile(os.path.join(self.tmp, "kisi-kicad", "kisi-error.html")))

    def test_missing_kicad_python(self):
        sys.modules["kipy"] = None  # import fails
        rc, out, err = self.run_action([])
        self.assertEqual(rc, 1)
        self.assertIn("Recreate Plugin Environment", err)


# ---- with the real kicad-python (pip install kicad-python), against a fake API connection ------------------------
try:
    from kipy.proto.common.commands import base_commands_pb2, editor_commands_pb2
    from kipy.proto.common.types import DocumentType
    HAVE_KIPY = True
except Exception:
    HAVE_KIPY = False


@unittest.skipUnless(HAVE_KIPY, "kicad-python is not installed")
class RealKipyTests(unittest.TestCase):
    """kisi_action.board_from_kicad with kicad-python's own KiCad and Board classes; only the connection is fake:
    it answers GetVersion and GetOpenDocuments the way KiCad 10's PCB editor does (api_handler_pcb.cpp)."""

    def client(self, name, path, project):
        test = self

        class Client:
            sent = []

            def send(self, cmd, response_type):
                Client.sent.append(type(cmd).__name__)
                r = response_type()
                if isinstance(cmd, base_commands_pb2.GetVersion):
                    r.version.major, r.version.minor, r.version.patch, r.version.full_version = 10, 0, 7, "10.0.7-1"
                elif isinstance(cmd, editor_commands_pb2.GetOpenDocuments):
                    test.assertEqual(cmd.type, DocumentType.DOCTYPE_PCB)
                    d = r.documents.add()
                    d.type = DocumentType.DOCTYPE_PCB
                    d.board_filename = name
                    d.project.name, d.project.path = project, path
                else:
                    raise AssertionError(f"unexpected command {type(cmd).__name__}")
                return r

        return Client()

    def test_board_from_kicad(self):
        import kipy
        import kisi_action
        c = self.client("Mein Board.kicad_pcb", "C:\\Users\\Jürgen\\KiCad\\Mein Board", "Mein Board")
        real = kipy.KiCad
        kipy.KiCad = lambda **kw: real.from_client(c)
        try:
            got = kisi_action.board_from_kicad(lambda m: None)
        finally:
            kipy.KiCad = real
        self.assertEqual(got, ("Mein Board.kicad_pcb", "C:\\Users\\Jürgen\\KiCad\\Mein Board", "Mein Board", "unknown", "10.0.7 (10.0.7-1)"))
        self.assertEqual(c.sent, ["GetVersion", "GetOpenDocuments"])  # nothing that saves or changes the board

    def test_modified_state_where_the_api_has_it(self):
        import kisi_action
        if not hasattr(editor_commands_pb2, "GetDocumentModifiedState"):
            board = types.SimpleNamespace(document=None, client=self.client("", "", ""))
            self.assertEqual(kisi_action.modified_state(board), "unknown")  # KiCad 10's API: no such command
            return
        for state, want in ((1, "unmodified"), (2, "modified"), (0, "unknown")):
            class C:
                def send(self, cmd, rt):
                    r = rt()
                    r.state = state
                    return r
            from kipy.proto.common.types import DocumentSpecifier
            self.assertEqual(kisi_action.modified_state(types.SimpleNamespace(document=DocumentSpecifier(), client=C())), want)


if __name__ == "__main__":
    unittest.main()
