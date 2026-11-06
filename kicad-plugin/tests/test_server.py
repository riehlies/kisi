# SPDX-License-Identifier: GPL-3.0-only
"""The real loopback transport and disk writes, without KiCad or a browser."""
import concurrent.futures
import http.client
import json
import os
import pathlib
import sys
import threading
import time
import unittest
from urllib.parse import quote
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import kisi_kicad as core
import kisi_server as server
from test_kisi_kicad import Board, NO_NODE, digest, payload_of, unpack


class ServerTests(unittest.TestCase):
    def setUp(self):
        self.b = Board()
        self.addCleanup(self.b.close)
        self.page, self.payload = core.make_page(self.b.stem + ".kicad_pcb", self.b.dir, "", viewer=self.b.viewer,
                                                temp=self.b.out, downloads=None, node=NO_NODE, cli=None)
        self.s = self.start()
        self.initial = pathlib.Path(self.b.cfg).read_bytes()
        self.pcb_hash = digest(self.b.pcb)

    def start(self, payload=None, idle_timeout=30):
        s = server.Session(payload or self.payload, template=self.b.viewer, temp=self.b.out, idle_timeout=idle_timeout)
        thread = threading.Thread(target=s.run, daemon=True)
        thread.start()
        def stop():
            s.closed.set()
            thread.join(timeout=3)
            self.assertFalse(thread.is_alive(), "session did not stop")
        self.addCleanup(stop)
        s.thread = thread
        return s

    def request(self, method, path, body=None, headers=None, session=None, auth=True):
        s = session or self.s
        hs = {"Content-Type": "application/json", "Origin": s.origin}
        if auth:
            hs["Authorization"] = "Bearer " + s.token
        hs.update(headers or {})
        conn = http.client.HTTPConnection("127.0.0.1", s.server.server_port, timeout=5)
        try:
            conn.request(method, path, body=body if isinstance(body, (str, bytes)) else json.dumps(body or {}), headers=hs)
            r = conn.getresponse()
            data = r.read()
            return r.status, data, dict(r.getheaders())
        finally:
            conn.close()

    def config(self, **extra):
        return json.dumps(dict(kisi_config=1, board=self.b.stem + ".kicad_pcb", **extra), ensure_ascii=False) + "\n"

    def save(self, text=None, expected=None, **kwargs):
        return self.request("POST", "/api/config", {"text": text or self.config(notes=["saved äöü"]),
                            "revision": expected or server.revision(self.initial)}, **kwargs)

    def test_real_save_backup_and_reload(self):
        status, data, headers = self.save()
        self.assertEqual(status, 200, data)
        saved = pathlib.Path(self.b.cfg).read_bytes()
        self.assertEqual(json.loads(data)["revision"], server.revision(saved))
        self.assertEqual(digest(self.b.pcb), self.pcb_hash)
        backups = list(pathlib.Path(self.b.dir, "analysis").glob("*.bak"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), self.initial)
        status, html, headers = self.request("GET", self.s.entry, auth=False)
        self.assertEqual(status, 200)
        live = payload_of(html.decode())
        self.assertEqual(live["bridge"]["revision"], server.revision(saved))
        self.assertEqual(unpack(live)[self.b.stem + ".kisi.json"], saved.decode())
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(headers["Referrer-Policy"], "no-referrer")
        csp = headers["Content-Security-Policy"]
        self.assertIn("default-src 'none'", csp)
        self.assertIn("connect-src 'self'", csp)
        self.assertNotIn("http", csp)  # no other host is allowed
        self.assertNotIn(self.s.token, pathlib.Path(self.page).read_text(encoding="utf-8"))

    def test_backups_are_limited(self):
        analysis = pathlib.Path(self.b.dir, "analysis")
        expected = server.revision(self.initial)
        for i in range(server.KEEP_BACKUPS + 3):
            status, data, _ = self.save(text=self.config(notes=[f"save {i}"]), expected=expected)
            self.assertEqual(status, 200, data)
            expected = json.loads(data)["revision"]
        backups = sorted(analysis.glob(self.b.stem + ".kisi.json.*.bak"))
        self.assertEqual(len(backups), server.KEEP_BACKUPS)
        newest = max(backups, key=lambda p: (p.stat().st_mtime, p.name))
        self.assertIn("save " + str(server.KEEP_BACKUPS + 1), newest.read_text(encoding="utf-8"))
        other = analysis / "other.kisi.json.20260101-000000.bak"
        other.write_text("{}", encoding="utf-8")
        server.prune_backups(str(analysis), self.b.stem, keep=1)
        self.assertTrue(other.exists(), "backups of other boards are not touched")

    def test_authentication_origin_and_host(self):
        for kwargs in [dict(auth=False), dict(headers={"Authorization": "Bearer wrong"}),
                       dict(headers={"Origin": "https://attacker.invalid"}),
                       dict(headers={"Origin": "null"}), dict(headers={"Host": "attacker.invalid"}),
                       dict(headers={"Sec-Fetch-Site": "cross-site"})]:
            with self.subTest(kwargs=kwargs):
                self.assertEqual(self.save(**kwargs)[0], 403)
        self.assertEqual(pathlib.Path(self.b.cfg).read_bytes(), self.initial)
        self.assertEqual(self.request("GET", "/")[0], 404)
        self.assertEqual(self.request("GET", "/api/config", auth=False)[0], 404)
        self.assertEqual(self.request("GET", self.s.entry, headers={"Host": "attacker.invalid"})[0], 403)

    def test_no_file_or_command_api(self):
        for path in ["/../" + quote(os.path.basename(self.b.pcb)), "/api/run", "/api/config?path=other.json", "/favicon.ico"]:
            self.assertEqual(self.request("GET", path)[0], 404)
        self.assertEqual(self.request("POST", "/api/config", {"text": self.config(), "revision": server.revision(self.initial),
                                                              "path": self.b.pcb})[0], 400)
        self.assertEqual(digest(self.b.pcb), self.pcb_hash)

    def test_invalid_board_and_requests(self):
        for text in ["{", "[]", '{"kisi_config":1,"board":"another.kicad_pcb"}',
                     self.config().replace('"kisi_config": 1', '"kisi_config": NaN')]:
            with self.subTest(text=text):
                self.assertEqual(self.save(text=text)[0], 400)
        self.assertEqual(self.request("POST", "/api/config", "{bad")[0], 400)
        self.assertEqual(self.save(headers={"Content-Type": "text/plain"})[0], 415)
        self.assertEqual(self.save(headers={"Content-Length": str(server.MAX_REQUEST + 1)})[0], 413)
        self.assertEqual(self.save(headers={"Transfer-Encoding": "chunked"})[0], 400)
        self.assertEqual(pathlib.Path(self.b.cfg).read_bytes(), self.initial)

    def test_external_edit_and_other_window_conflict(self):
        second = self.start()
        text = self.config(notes=["external edit"])
        pathlib.Path(self.b.cfg).write_text(text, encoding="utf-8")
        self.assertEqual(self.save()[0], 409)
        self.assertEqual(self.save(session=second)[0], 409)
        self.assertEqual(pathlib.Path(self.b.cfg).read_text(encoding="utf-8"), text)

    def test_write_failure_and_size_limit_keep_the_file(self):
        with patch.object(self.s.guard, "write", side_effect=OSError("read-only folder")):
            self.assertEqual(self.save()[0], 500)
        self.assertEqual(self.save(text=self.config(notes=["x" * server.MAX_CONFIG]))[0], 413)
        self.assertEqual(pathlib.Path(self.b.cfg).read_bytes(), self.initial)
        self.assertEqual(digest(self.b.pcb), self.pcb_hash)

    def test_concurrent_sessions_do_not_lose_updates(self):
        other = self.start()
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            jobs = [pool.submit(self.save, text=self.config(notes=[str(i)]), session=s) for i, s in enumerate([self.s, other])]
            self.assertEqual(sorted(job.result()[0] for job in jobs), [200, 409])

    def test_sessions_are_isolated(self):
        other = self.start()
        self.assertNotEqual(self.s.token, other.token)
        self.assertEqual(self.save(session=other, headers={"Authorization": "Bearer " + self.s.token})[0], 403)
        self.assertEqual(self.request("GET", self.s.entry, session=other)[0], 404)

    def test_startup_does_not_need_dns(self):
        with patch("socket.getfqdn", side_effect=AssertionError("local startup must not resolve hostnames")):
            s = self.start()
            self.assertEqual(self.request("GET", s.entry, session=s)[0], 200)

    def test_idle_shutdown_and_heartbeat(self):
        s = self.start(idle_timeout=0.8)
        time.sleep(0.5)
        self.assertEqual(self.request("POST", "/api/ping", session=s)[0], 200)
        time.sleep(0.5)
        self.assertTrue(s.thread.is_alive())
        s.thread.join(timeout=2)
        self.assertFalse(s.thread.is_alive())
