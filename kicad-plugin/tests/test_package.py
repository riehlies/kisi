# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the package (tools/build_kicad_pcm.py) with a stand-in viewer: the zip as KiCad's Plugin and Content
Manager extracts it (kicad/pcm/pcm_task_manager.cpp, PCM_TASK_MANAGER::extract), what KiCad's plugin manager then
finds (common/api/api_plugin_manager.cpp: plugin.json anywhere below 3rdparty/, requirements.txt next to it),
the schemas, reproducibility, and the extracted plugin making a page.

Needs jsonschema (pip install jsonschema). Run: python3 -m unittest discover -s kicad-plugin/tests -v
"""
import hashlib, json, os, pathlib, re, shutil, subprocess, sys, tempfile, unittest, zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import build_kicad_pcm as pcm  # noqa: E402

VIEWER = '<!doctype html><script>const KICAD_BOARD = null; /*@kicad-board*/</script><span class="ver">v0.0.0</span>'
CLI = '#!/usr/bin/env node\nconst VERSION = "0.0.0";\nconsole.log(JSON.stringify({ok: true, errors: []}));\n'
# KiCad's package folders (kicad/pcm/pcm.h, PCM_PACKAGE_DIRECTORIES)
PCM_DIRS = {"plugins", "footprints", "3dmodels", "symbols", "resources", "colors", "templates", "scripts"}


def pcm_extract(zpath, identifier, third_party):
    """What KiCad's PCM writes: <folder>/<rest> → 3rdparty/<folder>/<identifier with . → _>/<rest>, only for the
    package folders; entries at the root (metadata.json) and directories are skipped."""
    out = []
    with zipfile.ZipFile(zpath) as z:
        for e in z.infolist():
            parts = e.filename.split("/")
            if e.is_dir() or len(parts) < 2 or parts[0] not in PCM_DIRS:
                continue
            assert ".." not in parts and not e.filename.startswith("/"), e.filename
            target = os.path.join(third_party, parts[0], identifier.replace(".", "_"), *parts[1:])
            os.makedirs(os.path.dirname(target), exist_ok=True)
            with open(target, "wb") as f:
                f.write(z.read(e))
            out.append(target)
    return out


class PackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="kisi-pcm-test-")
        cls.viewer = os.path.join(cls.tmp, "kisi.html")
        pathlib.Path(cls.viewer).write_text(encoding="utf-8", data=VIEWER)
        cls.cli = os.path.join(cls.tmp, "kisi.mjs")
        pathlib.Path(cls.cli).write_text(encoding="utf-8", data=CLI)
        cls.r = pcm.build(cls.viewer, cls.cli, os.path.join(cls.tmp, "dist"), version="0.0.0", test_viewer=True)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_layout(self):
        names = zipfile.ZipFile(self.r["zip"]).namelist()
        self.assertEqual(names[0], "metadata.json")
        for n in ["plugins/plugin.json", "plugins/requirements.txt", "plugins/kisi_action.py", "plugins/kisi_kicad.py",
                  "plugins/kisi_build.json", "plugins/viewer/kisi.html", "plugins/cli/kisi.mjs", "plugins/LICENSE", "resources/icon.png"]:
            self.assertIn(n, names)
        # nothing else at the root, no nested plugin folder, no tests or generator scripts
        self.assertEqual({n.split("/")[0] for n in names}, {"metadata.json", "plugins", "resources"})
        self.assertFalse([n for n in names if "tests/" in n or n.endswith(("make_icons.py", ".pyc"))])

    def test_as_kicad_installs_it(self):
        third = os.path.join(self.tmp, "3rdparty")
        md = json.loads(zipfile.ZipFile(self.r["zip"]).read("metadata.json"))
        files = pcm_extract(self.r["zip"], md["identifier"], third)
        base = os.path.join(third, "plugins", "com_github_riehlies_kisi")
        found = [os.path.join(dp, f) for dp, _, fs in os.walk(third) for f in fs if f == "plugin.json"]
        self.assertEqual(found, [os.path.join(base, "plugin.json")])
        self.assertTrue(os.path.isfile(os.path.join(base, "requirements.txt")))  # KiCad's pip install -r needs it here
        pj = json.loads(pathlib.Path(os.path.join(base, "plugin.json")).read_text(encoding="utf-8"))
        for a in pj["actions"]:
            for f in [a["entrypoint"]] + a["icons-light"] + a["icons-dark"]:
                self.assertTrue(os.path.isfile(os.path.join(base, f)), f)  # relative to plugin.json, as KiCad resolves
        self.assertTrue(os.path.isfile(os.path.join(third, "resources", "com_github_riehlies_kisi", "icon.png")))
        self.assertEqual(sum(os.path.getsize(f) for f in files), self.r["install_size"])
        # the extracted plugin makes a page (without KiCad: --pcb)
        board = os.path.join(self.tmp, "Board Ä", "b.kicad_pcb")
        os.makedirs(os.path.dirname(board))
        pathlib.Path(board).write_text(encoding="utf-8", data="(kicad_pcb (version 20241229))\n")
        env = dict(os.environ, TMPDIR=os.path.join(self.tmp, "t"), TEMP=os.path.join(self.tmp, "t"), TMP=os.path.join(self.tmp, "t"))
        os.makedirs(env["TMPDIR"], exist_ok=True)
        p = subprocess.run([sys.executable, "-I", os.path.join(base, "kisi_action.py"), "--pcb", board, "--no-browser"],
                           capture_output=True, text=True, env=env, cwd=base)
        self.assertEqual(p.returncode, 0, p.stderr)
        page = p.stdout.strip().splitlines()[-1]
        html = pathlib.Path(page).read_text(encoding="utf-8")
        self.assertIn('"plugin":"0.0.0"', html)
        self.assertEqual(page, os.path.join(os.path.dirname(board), "analysis", "b.kisi.html"))
        self.assertIn(json.dumps(os.path.join(base, "cli", "kisi.mjs"))[1:-1], html)  # the installed CLI path
        self.assertTrue(os.path.isfile(os.path.join(os.path.dirname(board), "b.kisi.json")))

    def test_metadata(self):
        md = json.loads(zipfile.ZipFile(self.r["zip"]).read("metadata.json"))
        self.assertEqual(pcm.check_metadata(md), [])
        self.assertEqual(md["versions"][0]["version"], "0.0.0")
        self.assertEqual(md["versions"][0]["runtime"], "ipc")
        self.assertNotIn("download_url", md["versions"][0])
        sub = json.loads(pathlib.Path(self.r["metadata"]).read_text(encoding="utf-8"))
        self.assertEqual(pcm.check_metadata(sub, submission=True), [])
        v = sub["versions"][0]
        self.assertEqual(v["download_url"], "https://github.com/riehlies/kisi/releases/download/v0.0.0/kisi-kicad-0.0.0.zip")
        self.assertEqual(v["download_sha256"], hashlib.sha256(pathlib.Path(self.r["zip"]).read_bytes()).hexdigest())
        self.assertEqual(v["download_size"], os.path.getsize(self.r["zip"]))
        mr = os.path.join(os.path.dirname(self.r["zip"]), "pcm", "com.github.riehlies.kisi")
        self.assertEqual(sorted(os.listdir(mr)), ["icon.png", "metadata.json"])
        # English metadata, identifier as KiCad's add-on rules want it for a GitHub project
        self.assertRegex(md["identifier"], r"^com\.github\.[a-z0-9-]+\.[a-z0-9-]+$")
        self.assertTrue(all(ord(c) < 0x2100 for c in md["description_full"]))

    def test_schemas_reject_mistakes(self):
        md = json.loads(zipfile.ZipFile(self.r["zip"]).read("metadata.json"))
        bad = json.loads(json.dumps(md))
        bad["license"] = "GPL-3.0-only"  # valid for KiCad 10, rejected by KiCad 9's Install from File
        self.assertTrue(any("KiCad 9" in p for p in pcm.check_metadata(bad)))
        bad = json.loads(json.dumps(md))
        bad["versions"][0]["download_sha256"] = "0" * 64
        self.assertTrue(pcm.check_metadata(bad))
        pj = json.loads(pathlib.Path(os.path.join(ROOT, "kicad-plugin", "plugin.json")).read_text(encoding="utf-8"))
        self.assertEqual(pcm.check_plugin_json(pj), [])
        for k, v in (("identifier", "kisi"), ("runtime", {"type": "ipc"})):
            self.assertTrue(pcm.check_plugin_json(dict(pj, **{k: v})), k)

    def test_icons(self):
        z = zipfile.ZipFile(self.r["zip"])
        for n, size in (("plugins/icons/kisi-24.png", 24), ("plugins/icons/kisi-48.png", 48), ("resources/icon.png", 64)):
            d = z.read(n)
            self.assertEqual(d[:8], b"\x89PNG\r\n\x1a\n")
            self.assertEqual((int.from_bytes(d[16:20], "big"), int.from_bytes(d[20:24], "big")), (size, size), n)

    def test_reproducible(self):
        r2 = pcm.build(self.viewer, self.cli, os.path.join(self.tmp, "dist2"), version="0.0.0", test_viewer=True)
        self.assertEqual(r2["sha256"], self.r["sha256"])
        for zi in zipfile.ZipFile(self.r["zip"]).infolist():
            self.assertEqual(zi.date_time, (1980, 1, 1, 0, 0, 0))

    def test_viewer_checks(self):
        self.assertTrue(pcm.check_viewer("<html></html>", "0.0.0"))
        self.assertTrue(pcm.check_viewer(VIEWER, "1.2.3"))  # another version
        self.assertEqual(pcm.check_viewer(VIEWER, "0.0.0"), [])
        self.assertTrue(re.search("hand-over", pcm.check_viewer(VIEWER + VIEWER, "0.0.0")[0]))
        self.assertEqual(pcm.check_cli(CLI, "0.0.0"), [])
        self.assertTrue(pcm.check_cli(CLI, "1.2.3"))


if __name__ == "__main__":
    unittest.main()
