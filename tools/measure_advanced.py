#!/usr/bin/env python3
"""Measure the delivery variants of the "Advanced solvers" prototype (docs/research/advanced-solvers.md) in headless
Chromium and check them end to end.

Usage: measure_advanced.py <dist dir> [--out results.json] [--md table.md] [--shots DIR]

<dist dir> holds the builds (missing ones are skipped):
  kisi.html, kisi-examples.html       the default bundles (as main)
  kisi-examples-adv.html               bundle.py --advanced, with the example boards
  kisi-examples-inline.html            bundle.py --advanced-inline kisi-advanced.html, with the example boards
  kisi-advanced.html                   tools/advanced_bundle.py (with --fw: the full-wave backend too)
  kisi.mjs                             the Node CLI (size only)

What it does:
  * sizes: raw and gzip -9 of every file
  * kisi-advanced.html alone: load time (navigation to Plotly and the app ready)
  * variant 1 (local, file://): the viewer with the ThunderScope example, the option on, a real click on a segment of
    a microstrip, a differential pair and a stripline, "Fields…": the new window, the solve, the view shown
    (__fieldHandover of web/advanced/handover.js); the first segment opens the window, the next ones change its
    fragment only. With each solver the build has (quasi-static; full-wave when the modules are in).
  * variant 2 (inline): load time of the viewer with the page built in, one segment opened from the blob: URL
  * variant 3 (link-out): kisi-advanced.html served over http on 127.0.0.1 stands in for the hosted page (another
    origin than the file:// viewer): the same hand-over across origins; the fragment lengths of the three segments
  * memory: resident size of all browser processes (sampled every 0.2 s, peak), a fresh browser per measurement
Exit code 1 when a hand-over fails or a page logs an error.
"""
import argparse, functools, gzip, http.server, json, os, shutil, socketserver, sys, tempfile, threading, time

from playwright.sync_api import sync_playwright

try:
    import psutil
except ImportError:  # memory is then not measured
    psutil = None

ap = argparse.ArgumentParser()
ap.add_argument("dist")
ap.add_argument("--out")
ap.add_argument("--md")
ap.add_argument("--shots")
a = ap.parse_args()
D = os.path.abspath(a.dist)
shots = a.shots or tempfile.mkdtemp(prefix="kisi-adv-shots-")
os.makedirs(shots, exist_ok=True)
BUSY = "document.getElementById('busy').hidden === true"
# the three segments of the ThunderScope example: net, layer, width (mm)
SEGMENTS = [("microstrip", "Net-(C1012_4-Pad1)", "F.Cu", 0.213),
            ("pair", "/TS-PCIe Components/PCIe_PET0_N", "F.Cu", 0.19079),
            ("stripline", "/FPGA/FPGA Config and Transceivers/DONE", "In3.Cu", 0.127)]
R = {"sizes": {}, "embedded": {}, "adv_alone": {}, "variant1": {}, "variant2": {}, "variant3": {}, "problems": []}
problems = R["problems"]


def path(n):
    p = os.path.join(D, n)
    return p if os.path.exists(p) else None


# ---- sizes ------------------------------------------------------------------------------------------------------
for n in sorted(os.listdir(D)):
    if n.endswith((".html", ".mjs")):
        b = open(os.path.join(D, n), "rb").read()
        R["sizes"][n] = {"raw": len(b), "gzip": len(gzip.compress(b, 9))}


# the files inside kisi-advanced.html (gzip as embedded, raw after decoding)
if path("kisi-advanced.html"):
    import base64, re
    h = open(path("kisi-advanced.html"), encoding="utf-8").read()
    files = json.loads(re.search(r'id="kisi-adv-files">(.*?)</script>', h, re.S).group(1))
    R["embedded"] = {k: {"gzip": len(base64.b64decode(v)), "raw": len(gzip.decompress(base64.b64decode(v))), "base64": len(v)}
                     for k, v in files.items()}


# ---- browser with memory sampling ---------------------------------------------------------------------------------
class Browser:
    def __init__(self, pw):
        self.dir = tempfile.mkdtemp(prefix="kisi-adv-prof-")
        self.ctx = pw.chromium.launch_persistent_context(self.dir, headless=True, viewport={"width": 1400, "height": 900},
                                                         color_scheme="light")
        self.errors, self.peak, self.stop = [], 0, False
        self.ctx.on("page", self.watch)
        for p in self.ctx.pages:
            self.watch(p)
        self.t = threading.Thread(target=self.sample, daemon=True)
        self.t.start()

    def watch(self, pg):
        pg.on("pageerror", lambda e: self.errors.append(f"pageerror: {e}"))
        pg.on("console", lambda m: self.errors.append(f"console {m.type}: {m.text[:300]}") if m.type == "error" else None)

    def rss(self):
        if not psutil:
            return 0
        # the browser process (its command line names the profile folder) and every process below it (renderers,
        # GPU, utility); PSS (shared pages split between the processes that map them), RSS where PSS is not readable
        procs = set()
        for p in psutil.process_iter(["cmdline"]):
            try:
                if any(self.dir in c for c in (p.info["cmdline"] or [])):
                    procs.add(p)
                    procs.update(p.children(recursive=True))
            except (psutil.NoSuchProcess, psutil.AccessDenied, TypeError):
                pass
        tot = 0
        for p in procs:
            try:
                tot += p.memory_full_info().pss
            except (psutil.NoSuchProcess, psutil.AccessDenied, AttributeError):
                try:
                    tot += p.memory_info().rss
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
        return tot

    def sample(self):
        while not self.stop:
            self.peak = max(self.peak, self.rss())
            time.sleep(0.2)

    def close(self):
        self.stop = True
        self.t.join()
        self.ctx.close()
        shutil.rmtree(self.dir, ignore_errors=True)


def mb(x):
    return round(x / 1e6, 1)


def open_viewer(br, html):
    pg = br.ctx.pages[0] if br.ctx.pages else br.ctx.new_page()
    t = time.time()
    pg.goto("file://" + html)
    pg.wait_for_function(f"typeof S !== 'undefined' && S.scene && {BUSY}", timeout=180000)
    load = time.time() - t
    pg.click("#view-si")
    t = time.time()
    pg.click("#an-start")
    pg.wait_for_function(f"S.full && {BUSY}", timeout=180000)
    return pg, load, time.time() - t


def find_xs(pg, net, layer, w):
    return pg.evaluate("""([net, layer, w]) => {
      const n = S.scene.nets.indexOf(net), pc = S.pieces; let best = null;
      for (let i = 0; i < pc.n; i++) {
        if (pc.net[i] !== n) continue;
        const xs = S.an.cross_sections[pc.xs[i]];
        if (xs.geom == null || S.scene.stackup.copper[xs.layer].name !== layer || Math.abs(xs.w - w) > 1e-4) continue;
        const len = Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
        if (!best || len > best.len) best = { piece: i, xi: pc.xs[i], net: n, len };
      }
      return best; }""", [net, layer, w])


def click_segment(pg, seg):
    """a real click on the segment's longest piece; returns its net and cross-section"""
    hit = find_xs(pg, *seg[1:])
    if not hit:
        raise RuntimeError(f"segment {seg[0]} not found")
    pg.evaluate("(n) => selectNet(n, true)", hit["net"])
    pg.wait_for_timeout(300)
    # a point on the piece where the board's own pick finds this cross-section (another track may lie on top)
    x, y = pg.evaluate("""(i) => { const pc = S.pieces, v = S.view, r = cv.getBoundingClientRect();
      let p = null;
      for (const t of [0.5, 0.3, 0.7, 0.15, 0.85, 0.05, 0.95]) {
        const mx = (pc.ax[i] + t * (pc.bx[i] - pc.ax[i])) * v.s + v.x, my = (pc.ay[i] + t * (pc.by[i] - pc.ay[i])) * v.s + v.y;
        p = p || [mx, my];
        const k = pick(mx, my);
        if (k >= 0 && pc.xs[k] === pc.xs[i]) { p = [mx, my]; break; }
      }
      return [r.left + p[0], r.top + p[1]]; }""", hit["piece"])
    pg.mouse.click(x, y)
    pg.wait_for_timeout(300)
    got = pg.evaluate("ADV.piece >= 0 ? S.pieces.xs[ADV.piece] : null")
    hit["real_click"] = got == hit["xi"]
    if not hit["real_click"]:  # another piece on top at that point: as if the user had chosen it in the list
        pg.evaluate("([p, n]) => { ADV.piece = p; selectNet(n, false); }", [hit["piece"], hit["net"]])
    pg.select_option("#adv-xs", str(hit["xi"]))
    return hit


def handover(br, pg, popup, seg, tag, view_shot=True):
    """Fields… for the selected segment; waits for the view in the field page"""
    t = time.time()
    if popup is None:
        with br.ctx.expect_page(timeout=60000) as pinfo:
            pg.click("#adv-go")
        popup = pinfo.value
    else:
        n0 = popup.evaluate("window.__fieldHandover ? window.__fieldHandover.seq : 0")
        pg.click("#adv-go")
        popup.wait_for_function("(n) => window.__fieldHandover && window.__fieldHandover.seq > n", arg=n0, timeout=60000)
    popup.wait_for_function("window.__fieldHandover && ['shown', 'error'].includes(window.__fieldHandover.phase)",
                            timeout=1800000, polling=200)
    wall = time.time() - t
    h = popup.evaluate("({ ...window.__fieldHandover, request: undefined })")
    # the field drawn (a heatmap / contour trace; H, J, S and losses come from a worker job after the view switch)
    try:
        popup.wait_for_function("(document.getElementById('sim_canvas').data || []).some((t) => ['heatmap', 'contour'].includes(t.type))",
                                timeout=60000, polling=200)
    except Exception:
        problems.append(f"{tag} {seg[0]}: no field drawn")
    popup.wait_for_timeout(1000)
    info = popup.evaluate("""() => { const gd = document.getElementById('sim_canvas'), log = document.getElementById('console_out').textContent;
      const t = gd.layout && gd.layout.title; const last = (re) => { const m = [...log.matchAll(re)]; return m.length ? +m[m.length - 1][1] : null; };
      return { title: String(t && (t.text || t) || '').replace(/<[^>]+>/g, ' ').slice(0, 240), field_warning: /Field views:/.test(log),
               z0: last(/Z0:\s*([\d.]+)/g), z0_odd: last(/Z0_odd:\s*([\d.]+)/g), z0_even: last(/Z0_even:\s*([\d.]+)/g) }; }""")
    if info["field_warning"]:
        problems.append(f"{tag} {seg[0]}: the page logged a field view warning")
    if view_shot:
        popup.screenshot(path=os.path.join(shots, f"{tag}_{seg[0]}.png"))
    if h.get("phase") != "shown":
        problems.append(f"{tag} {seg[0]}: hand-over ended in {h.get('phase')}: {h.get('error')}")
    return popup, {"wall_s": round(wall, 2), "solve_ms": h.get("solveMs"), "shown_ms": h.get("shownMs"), **info}


def set_option(pg, target, backend, view, freq=1):
    if not pg.evaluate("$('fs-opts').open"):
        pg.click("#fs-opts > summary")
    if not pg.is_checked("#adv-on"):
        pg.click("#adv-on")
    pg.select_option("#adv-target", target)
    pg.select_option("#adv-backend", backend)
    pg.select_option("#adv-view", view)
    pg.select_option("#adv-freq", str(freq))


with sync_playwright() as pw:
    adv = path("kisi-advanced.html")
    fw = False
    # ---- kisi-advanced.html alone ----------------------------------------------------------------------------------
    if adv:
        br = Browser(pw)
        pg = br.ctx.pages[0]
        t = time.time()
        pg.goto("file://" + adv)
        try:
            pg.wait_for_function("typeof Plotly === 'object' && window.__kisiAdv && window.__kisiAdv.tStarted && "
                                 "/Ready/.test(document.getElementById('console_out').textContent)", timeout=120000)
        except Exception:
            print("kisi-advanced.html did not start:\n" + "\n".join(br.errors[:20]))
            raise
        fw = pg.evaluate("window.__kisiAdv.has('wasm_solver/gmsh.wasm')")
        R["adv_alone"] = {"load_s": round(time.time() - t, 2), "decoded_ms": round(pg.evaluate("window.__kisiAdv.tDecoded")),
                          "rss_mb": mb(br.rss()), "fullwave_in_build": fw}
        pg.wait_for_timeout(500)
        R["adv_alone"]["errors"] = list(br.errors)
        problems.extend(f"kisi-advanced.html: {e}" for e in br.errors)
        br.close()

    # ---- variant 1: local file next to the viewer -------------------------------------------------------------------
    viewer = path("kisi-examples-adv.html")
    if viewer and adv:
        for backend, view in [("rectilinear", "efield")] + ([("fullwave_mqs", "sfield")] if fw else []):
            br = Browser(pw)
            pg, load, est = open_viewer(br, viewer)
            pg.screenshot(path=os.path.join(shots, "viewer_estimate.png"))
            set_option(pg, "local", backend, view)
            res, popup = {"viewer_load_s": round(load, 2), "estimate_s": round(est, 2)}, None
            for k, seg in enumerate(SEGMENTS):
                hit = click_segment(pg, seg)
                if k == 0:
                    pg.screenshot(path=os.path.join(shots, f"viewer_fields_button_{backend}.png"))
                popup, r = handover(br, pg, popup, seg, f"v1_{backend}")
                r["real_click"] = hit["real_click"]
                r["fragment_chars"] = len(pg.evaluate("([n, x]) => advFragment(n, x)", [hit["net"], hit["xi"]]))
                res[seg[0]] = r
            # the other views of the pair (full-wave: all six; quasi-static: potential and |E|)
            hit = click_segment(pg, SEGMENTS[1])
            for v in (["potential", "hfield", "jfield", "lossfield"] if fw and backend == "fullwave_mqs" else ["potential"]):
                pg.select_option("#adv-view", v)
                popup, r = handover(br, pg, popup, SEGMENTS[1], f"v1_{backend}_{v}")
                res[f"pair_{v}"] = r
            res["rss_peak_mb"] = mb(br.peak)
            res["errors"] = list(br.errors)
            problems.extend(f"variant 1 {backend}: {e}" for e in br.errors)
            R["variant1"][backend] = res
            br.close()

    # ---- variant 2: the page built into kisi.html --------------------------------------------------------------------
    inline, plain = path("kisi-examples-inline.html"), path("kisi-examples.html")
    if inline:
        for name, html in [("default", plain), ("inline", inline)]:
            if not html:
                continue
            br = Browser(pw)
            pg, load, est = open_viewer(br, html)
            r = {"viewer_load_s": round(load, 2), "estimate_s": round(est, 2), "rss_mb": mb(br.rss())}
            if name == "inline":
                backend = "fullwave_mqs" if fw else "rectilinear"
                set_option(pg, "inline", backend, "sfield" if fw else "efield")
                click_segment(pg, SEGMENTS[0])
                _, r["microstrip"] = handover(br, pg, None, SEGMENTS[0], "v2_inline")
                r["rss_peak_mb"] = mb(br.peak)
            r["errors"] = list(br.errors)
            problems.extend(f"variant 2 {name}: {e}" for e in br.errors)
            R["variant2"][name] = r
            br.close()

    # ---- variant 3: link-out to another origin (the hosted page's stand-in) -----------------------------------------
    if viewer and adv:
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=D)
        handler.log_message = lambda *x: None
        srv = socketserver.TCPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        host = f"http://127.0.0.1:{srv.server_address[1]}/kisi-advanced.html"
        br = Browser(pw)
        pg, load, est = open_viewer(br, viewer)
        set_option(pg, "hosted", "rectilinear", "efield")
        pg.evaluate("(u) => { ADV.hostedURL = u; }", host)
        res, popup = {"origin": "http://127.0.0.1 (stand-in for the hosted page)"}, None
        for seg in SEGMENTS[:2]:
            click_segment(pg, seg)
            popup, res[seg[0]] = handover(br, pg, popup, seg, "v3_http", view_shot=False)
        res["popup_url_origin"] = popup.evaluate("location.origin")
        res["errors"] = list(br.errors)
        problems.extend(f"variant 3: {e}" for e in br.errors)
        R["variant3"] = res
        br.close()
        srv.shutdown()

if a.out:
    json.dump(R, open(a.out, "w"), indent=1)
print(json.dumps(R, indent=1))
if a.md:
    s = R["sizes"]
    lines = ["| File | raw MB | gzip MB |", "|---|---:|---:|"]
    lines += [f"| {n} | {v['raw'] / 1e6:.2f} | {v['gzip'] / 1e6:.2f} |" for n, v in s.items()]
    open(a.md, "w").write("\n".join(lines) + "\n")
print(f"screenshots: {shots}")
if problems:
    print("PROBLEMS:\n" + "\n".join(problems))
    sys.exit(1)
