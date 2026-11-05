# SPDX-License-Identifier: GPL-3.0-only
"""Board-scoped, temporary loopback transport for the existing viewer (standard library only).

Only the generated viewer and this board's configuration are exposed. The HTML entry URL is a secret;
API requests additionally require the session token. No directory is served and no client-supplied path
is opened. This is not a general-purpose web server or a sandbox against other code running as this user.
"""
import hashlib
import json
import math
import os
import secrets
import threading
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import kisi_kicad as core

MAX_CONFIG = 2 * 1024 * 1024
MAX_REQUEST = MAX_CONFIG * 6 + 4096  # JSON string escaping can expand UTF-8 text
IDLE_TIMEOUT = 30 * 60  # suspended/closed tabs eventually release the process


def revision(data):
    return hashlib.sha256(data).hexdigest()


@contextmanager
def config_lock(folder, path):
    """Coordinate independent plugin processes, including two windows of the same board."""
    core.make_own_dir(folder)
    key = revision(os.path.normcase(os.path.realpath(path)).encode("utf-8"))
    with open(os.path.join(folder, "config-" + key + ".lock"), "a+b") as f:
        if os.name == "nt":
            import msvcrt
            if not os.path.getsize(f.name):
                f.write(b"\0")
                f.flush()
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_LOCK, 1)
            try:
                yield
            finally:
                f.seek(0)
                msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(f, fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(f, fcntl.LOCK_UN)


class Session:
    def __init__(self, payload, template=None, idle_timeout=IDLE_TIMEOUT, log=None, temp=None):
        if not math.isfinite(idle_timeout) or idle_timeout <= 0:
            raise ValueError("idle timeout must be positive")
        self.paths = core.board_paths(os.path.basename(payload["pcb_path"]), payload["dir"], "")
        self.guard = core.Guard(self.paths, temp=temp)
        self.token = secrets.token_urlsafe(32)
        self.entry = "/open/" + self.token
        self.idle_timeout = idle_timeout
        self.last_seen = time.monotonic()
        self.lock = threading.Lock()
        self.log = log or (lambda message: None)
        self.closed = threading.Event()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.server.session = self
        self.server.timeout = 0.25
        self.origin = f"http://127.0.0.1:{self.server.server_port}"
        self.url = self.origin + self.entry
        try:
            with open(template or core.find_viewer(), encoding="utf-8") as f:
                self.template = f.read()
            self.payload = payload
        except BaseException:
            self.server.server_close()
            raise

    def html(self):
        # Refreshing this URL must also refresh the saved configuration and its revision.
        data = self.read_config()
        name = self.paths["stem"] + ".kisi.json"
        files = [f for f in self.payload["files"] if f["name"] != name]
        if data:
            files.append({"name": name, "gz": core._gz(data)})
        live = dict(self.payload, files=files, bridge={"token": self.token, "revision": revision(data)})
        previous = next((f for f in self.payload["files"] if f["name"] == name), None)
        if data and (not previous or previous["gz"] != core._gz(data)):
            live["config"] = "loaded"
        return core.embed(self.template, live).encode("utf-8")

    def read_config(self):
        try:
            with open(self.paths["cfg"], "rb") as f:
                data = f.read(MAX_CONFIG + 1)
            if len(data) > MAX_CONFIG:
                raise ValueError("the board configuration exceeds 2 MB")
            return data
        except FileNotFoundError:
            return b""

    def save(self, body):
        if not isinstance(body, dict) or set(body) != {"text", "revision"}:
            return 400, {"error": "expected configuration text and revision"}
        text, expected = body["text"], body["revision"]
        if not isinstance(text, str) or not isinstance(expected, str):
            return 400, {"error": "invalid configuration request"}
        data = text.encode("utf-8")
        if len(data) > MAX_CONFIG:
            return 413, {"error": "the configuration exceeds 2 MB"}
        # The browser also runs the Rust schema validation before saving. The host independently
        # checks JSON, format and board identity; none of the configuration's paths are followed here.
        def reject_constant(value):
            raise ValueError(value)
        try:
            json.loads(text, parse_constant=reject_constant)
        except (ValueError, RecursionError):
            return 400, {"error": "invalid configuration JSON"}
        why = core.check_download(text, self.paths["stem"])
        if why:
            return 400, {"error": why}
        with self.lock, config_lock(self.guard.temp, self.paths["cfg"]):
            current = self.read_config()
            if not secrets.compare_digest(revision(current).encode(), expected.encode("utf-8")):
                return 409, {"error": "The configuration was changed outside this window. Reload this page to load it; your draft is kept."}
            if current != data:
                if current:
                    self.guard.write(self.guard.backup_path(), current, exclusive=True)
                self.guard.write(self.paths["cfg"], data)
            self.log("local viewer: board configuration saved")
            return 200, {"revision": revision(data)}

    def run(self):
        try:
            while not self.closed.is_set() and time.monotonic() - self.last_seen < self.idle_timeout:
                self.server.handle_request()
        finally:
            self.close()

    def close(self):
        self.closed.set()
        self.server.server_close()


class Handler(BaseHTTPRequestHandler):
    server_version = "kisi"

    def setup(self):
        super().setup()
        self.connection.settimeout(10)

    def log_message(self, *args):
        pass  # never put the secret entry URL, token or board data in the access log

    def reply(self, status, data, content_type="application/json"):
        if not isinstance(data, bytes):
            data = json.dumps(data).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        self.wfile.write(data)

    def authorized(self, api=False):
        s = self.server.session
        if self.headers.get_all("Host", []) != [s.origin.removeprefix("http://")]:
            return False
        origins = self.headers.get_all("Origin", [])
        if origins and origins != [s.origin]:
            return False
        if self.headers.get("Sec-Fetch-Site") == "cross-site":
            return False
        if api:
            auth = self.headers.get_all("Authorization", [])
            return len(auth) == 1 and secrets.compare_digest(auth[0].encode("utf-8"), ("Bearer " + s.token).encode())
        return True

    def do_GET(self):
        s = self.server.session
        if not self.authorized():
            self.reply(403, {"error": "forbidden"})
        elif secrets.compare_digest(self.path.encode("utf-8"), s.entry.encode()):
            try:
                html = s.html()
                s.last_seen = time.monotonic()
                self.reply(200, html, "text/html; charset=utf-8")
            except (OSError, ValueError):
                self.reply(500, {"error": "the board configuration could not be read"})
        elif self.path == "/api/config" and self.authorized(api=True):
            try:
                data = s.read_config()
                s.last_seen = time.monotonic()
                self.reply(200, {"text": data.decode("utf-8"), "revision": revision(data)})
            except (OSError, ValueError):
                self.reply(500, {"error": "the board configuration could not be read"})
        else:
            self.reply(404, {"error": "not found"})

    def do_POST(self):
        s = self.server.session
        if not self.authorized(api=True):
            self.reply(403, {"error": "forbidden"})
            return
        if self.path not in ("/api/config", "/api/ping"):
            self.reply(404, {"error": "not found"})
            return
        lengths = self.headers.get_all("Content-Length", [])
        if self.headers.get("Transfer-Encoding") or len(lengths) != 1 or not lengths[0].isascii() or not lengths[0].isdigit():
            self.reply(400, {"error": "a single content length is required"})
            return
        if len(lengths[0]) > 10:
            self.reply(413, {"error": "request too large"})
            return
        length = int(lengths[0])
        if length > MAX_REQUEST:
            self.reply(413, {"error": "request too large"})
            return
        if self.headers.get("Content-Type", "").split(";")[0].strip().lower() != "application/json":
            self.reply(415, {"error": "JSON is required"})
            return
        try:
            raw = self.rfile.read(length)
            if len(raw) != length:
                self.reply(400, {"error": "incomplete request"})
                return
            body = json.loads(raw)
            s.last_seen = time.monotonic()
            status, result = (200, {"ok": True}) if self.path == "/api/ping" else s.save(body)
            self.reply(status, result)
        except (ValueError, RecursionError):
            self.reply(400, {"error": "invalid JSON"})
        except OSError:
            self.reply(500, {"error": "The configuration could not be saved. Check folder permissions and try again."})
