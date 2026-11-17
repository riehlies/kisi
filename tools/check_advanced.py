#!/usr/bin/env python3
"""Headless check of the field views: kisi's "Fields…" and the field page next to the viewer (Playwright, Chromium).

Usage: check_advanced.py <kisi-examples.html> <kisi-advanced.html | kisi-advanced-lite.html>... [--out DIR]
                         [--themes light,dark] [--json FILE] [--md FILE] [--no-http]

For each field page given (a full build with every view, or the lite build with potential and |E|), in a folder of its
own next to the viewer (opened from disk, as a downloaded release is), and for each theme:
  * the viewer with the ThunderScope example (kisi-examples.html), the impedance estimate, the option "Advanced
    solvers: field views" on; a real click on a microstrip segment and on a segment of a differential pair; "Fields…":
    the field window opens, the page answers (its build), solves, shows the first view; the pair goes over by changing
    the fragment of the same window, in the odd mode (1 V between the conductors) and then in the common mode;
  * every view the build offers, switched inside the page (forkPlot.setView): the field drawn (no "Computing" or
    "not available" title); the power flow view's ∫S dA check and the page's Z0 from its log;
  * the page's theme is the viewer's (light / dark), no console errors in either page;
  * screenshots of every view at deviceScaleFactor 2 (--out).
With both builds given, the full page lies next to the lite one in its run (kisi.html must prefer it); the lite run has
the lite page alone (it opens when the full one is missing). Unless --no-http: the lite page (else the full one) once
over http from 127.0.0.1, where the viewer finds the page with a HEAD request.
Results: --json, --md (a table for a CI summary). Exit code 1 on problems.
"""
import argparse, functools, http.server, json, os, pathlib, re, shutil, socketserver, sys, tempfile, threading, time

from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("viewer")
ap.add_argument("pages", nargs="+")
ap.add_argument("--out")
ap.add_argument("--themes", default="light,dark")
ap.add_argument("--json")
ap.add_argument("--md")
ap.add_argument("--no-http", action="store_true")
a = ap.parse_args()
out = a.out or tempfile.mkdtemp(prefix="kisi-advanced-check-")
os.makedirs(out, exist_ok=True)
BUSY = "document.getElementById('busy').hidden === true"
# a microstrip and a pair of the ThunderScope example: net, layer, width (mm)
SEGMENTS = [("microstrip", "Net-(C1012_4-Pad1)", "F.Cu", 0.213), ("pair", "/TS-PCIe Components/PCIe_PET0_N", "F.Cu", 0.19079)]
VIEWS_FULL = ["geometry", "potential", "efield", "hfield", "jfield", "sfield", "lossfield"]
VIEWS_LITE = ["geometry", "potential", "efield"]
problems, rows, results = [], [], []


def check(ok, msg):
    if not ok:
        problems.append(msg)
        print("PROBLEM:", msg, flush=True)
    return ok


def build_of(path):
    head = open(path, encoding="utf-8").read(4000)
    m = re.search(r"kisi-advanced\.html \((full|lite) build\)", head)
    if not m:
        raise SystemExit(f"{path}: not a field page of tools/advanced_bundle.py")
    return m.group(1)


pages = {build_of(p): os.path.abspath(p) for p in a.pages}


def folder(build):
    """the viewer and the field page(s) of this run in a folder of their own"""
    d = tempfile.mkdtemp(prefix=f"kisi-adv-{build}-")
    shutil.copy(a.viewer, os.path.join(d, "kisi.html"))
    shutil.copy(pages[build], os.path.join(d, "kisi-advanced.html" if build == "full" else "kisi-advanced-lite.html"))
    if build == "full" and "lite" in pages:
        shutil.copy(pages["lite"], os.path.join(d, "kisi-advanced-lite.html"))
    return d


def watch(pg, tag, errs):
    pg.on("pageerror", lambda e: errs.append(f"{tag}: page error: {e}"))
    pg.on("console", lambda m: errs.append(f"{tag}: console {m.type}: {m.text[:300]}") if m.type == "error" else None)


def open_viewer(ctx, url, errs):
    pg = ctx.new_page()
    watch(pg, "viewer", errs)
    pg.goto(url)
    pg.wait_for_function(f"typeof S !== 'undefined' && S.scene && {BUSY}", timeout=180000)
    pg.click("#an-start")
    pg.wait_for_function(f"S.full && {BUSY}", timeout=180000)
    if not pg.evaluate("$('fs-opts').open"):
        pg.click("#fs-opts > summary")
    pg.check("#fv-on")
    return pg


def click_segment(pg, seg):
    """a real click on the segment's longest piece; returns {piece, xi, net, real_click}"""
    hit = pg.evaluate("""([net, layer, w]) => {
      const n = S.scene.nets.indexOf(net), pc = S.pieces; let best = null;
      for (let i = 0; i < pc.n; i++) {
        if (pc.net[i] !== n) continue;
        const xs = S.an.cross_sections[pc.xs[i]];
        if (xs.geom == null || S.scene.stackup.copper[xs.layer].name !== layer || Math.abs(xs.w - w) > 1e-4) continue;
        const len = Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
        if (!best || len > best.len) best = { piece: i, xi: pc.xs[i], net: n, len };
      }
      return best; }""", list(seg[1:]))
    if not check(hit is not None, f"segment {seg[0]} ({seg[1]}) not found on the board"):
        return None
    pg.evaluate("(n) => selectNet(n, true)", hit["net"])
    pg.wait_for_timeout(300)
    x, y = pg.evaluate("""(i) => { const pc = S.pieces, v = S.view, r = cv.getBoundingClientRect(); let p = null;
      for (const t of [0.5, 0.3, 0.7, 0.15, 0.85, 0.05, 0.95]) {
        const mx = (pc.ax[i] + t * (pc.bx[i] - pc.ax[i])) * v.s + v.x, my = (pc.ay[i] + t * (pc.by[i] - pc.ay[i])) * v.s + v.y;
        p = p || [mx, my];
        const k = pick(mx, my);
        if (k >= 0 && pc.xs[k] === pc.xs[i]) { p = [mx, my]; break; }
      }
      return [r.left + p[0], r.top + p[1]]; }""", hit["piece"])
    pg.mouse.click(x, y)
    pg.wait_for_timeout(300)
    hit["real_click"] = pg.evaluate("S.fvPiece >= 0 ? S.pieces.xs[S.fvPiece] : null") == hit["xi"]
    if not hit["real_click"]:  # another track on top at that point: as if chosen in the list
        pg.evaluate("([p, n]) => { S.fvPiece = p; selectNet(n, false); }", [hit["piece"], hit["net"]])
    row = pg.evaluate("({ row: !!document.getElementById('fv-go'), first: document.getElementById('fv-xs') && document.getElementById('fv-xs').value })")
    check(row["row"], f"{seg[0]}: no Fields row in the net detail")
    check(row["first"] == str(hit["xi"]), f"{seg[0]}: the clicked segment's cross-section is not the first in the list ({row['first']} != {hit['xi']})")
    return hit


FIELD_DRAWN = """() => { const gd = document.getElementById('sim_canvas'), t = gd && gd.layout && gd.layout.title;
  const title = String(t && (t.text || t) || '');
  return !!gd && (gd.data || []).some((x) => ['heatmap', 'contour'].includes(x.type)) && !/^(Computing|.*not available)/.test(title); }"""
PAGE_INFO = """() => { const gd = document.getElementById('sim_canvas'), log = document.getElementById('console_out').textContent;
  const t = gd.layout && gd.layout.title; const last = (re) => { const m = [...log.matchAll(re)]; return m.length ? +m[m.length - 1][1] : null; };
  return { title: String(t && (t.text || t) || '').replace(/<[^>]+>/g, ' ').replace(/\\s+/g, ' ').slice(0, 300),
           view: window.forkPlot && window.forkPlot.view(), views: window.forkPlot ? window.forkPlot.views() : [],
           theme: document.documentElement.dataset.theme, paper: gd._fullLayout && gd._fullLayout.paper_bgcolor,
           handover_title: (document.getElementById('handover-title') || {}).textContent || '',
           field_warning: /Field views:/.test(log), z0: last(/Z0:\\s*([\\d.]+)/g), z0_odd: last(/Z0_odd:\\s*([\\d.]+)/g),
           z0_even: last(/Z0_even:\\s*([\\d.]+)/g) }; }"""


def fields(ctx, pg, popup, tag):
    """Fields… for the selected cross-section: the field window, the page's answer and the view; returns (popup, info)"""
    t = time.time()
    if popup is None:
        with ctx.expect_page(timeout=60000) as pinfo:
            pg.click("#fv-go")
        popup = pinfo.value
        watch(popup, "field page", errs)
    else:
        n0 = popup.evaluate("window.__fieldHandover ? window.__fieldHandover.seq : 0")
        pg.click("#fv-go")
        popup.wait_for_function("(n) => window.__fieldHandover && window.__fieldHandover.seq > n", arg=n0, timeout=60000)
    popup.wait_for_function("window.__fieldHandover && ['shown', 'error'].includes(window.__fieldHandover.phase)", timeout=1800000, polling=200)
    h = popup.evaluate("({ ...window.__fieldHandover, request: undefined })")
    check(h.get("phase") == "shown", f"{tag}: hand-over ended in {h.get('phase')}: {h.get('error')}")
    try:
        pg.wait_for_function("/Shown in the field window/.test(document.getElementById('fv-msg').textContent)", timeout=20000)
    except Exception:
        check(False, f"{tag}: the viewer's status line: {pg.inner_text('#fv-msg')!r}")
    try:
        popup.wait_for_function(FIELD_DRAWN, timeout=120000, polling=200)
    except Exception:
        check(False, f"{tag}: no field drawn ({popup.evaluate(PAGE_INFO)['title']!r})")
    popup.wait_for_timeout(600)
    info = popup.evaluate(PAGE_INFO)
    info.update(wall_s=round(time.time() - t, 2), solve_ms=h.get("solveMs"), shown_ms=h.get("shownMs"), status=pg.inner_text("#fv-msg"))
    return popup, info


def views(popup, build, tag, shot):
    """every view the build offers, inside the page; screenshots"""
    got = {}
    for v in (VIEWS_FULL if build == "full" else VIEWS_LITE):
        ok = popup.evaluate("(v) => window.forkPlot.setView(v)", v)
        if not check(ok, f"{tag}: the page does not offer the view {v} ({popup.evaluate(PAGE_INFO)['views']})"):
            continue
        if v != "geometry":
            try:
                popup.wait_for_function(FIELD_DRAWN, timeout=180000, polling=200)
            except Exception:
                check(False, f"{tag} {v}: no field drawn ({popup.evaluate(PAGE_INFO)['title']!r})")
        popup.wait_for_timeout(700)
        info = popup.evaluate(PAGE_INFO)
        check(not info["field_warning"], f"{tag} {v}: the page logged a field view warning")
        got[v] = info["title"]
        popup.screenshot(path=os.path.join(out, f"{shot}_{v}.png"))
    return got


def run(pw, build, theme, base_url=None, quick=False, d=None):
    global errs
    errs = []
    own = d is None
    d = d or folder(build)
    url = (base_url or pathlib.Path(d).as_uri()) + "/kisi.html"
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=2, color_scheme=theme)
    tag0 = f"{build} {theme}{' http' if base_url else ''}"
    try:
        pg = open_viewer(ctx, url, errs)
        popup = None
        for seg in SEGMENTS[:1] if quick else SEGMENTS:
            hit = click_segment(pg, seg)
            if not hit:
                continue
            tag = f"{tag0} {seg[0]}"
            pg.select_option("#fv-view", "sfield" if build == "full" else "efield")
            popup, info = fields(ctx, pg, popup, tag)
            want = "sfield" if build == "full" else "efield"
            check(info["view"] == want, f"{tag}: the page shows {info['view']}, not {want}")
            check(info["theme"] == theme, f"{tag}: the field page's theme is {info['theme']}, the viewer's {theme}")
            check("kisi" in info["handover_title"] and ("Nyquist" in info["handover_title"] or "no interface profile" in info["handover_title"]),
                  f"{tag}: the title line {info['handover_title']!r}")
            build_seen = pg.evaluate("FV.build")
            check(build_seen == build, f"{tag}: the viewer took the {build_seen} page, expected {build}")
            pg.screenshot(path=os.path.join(out, f"{build}_{theme}_{seg[0]}_viewer.png"))
            res = {"run": tag0, "segment": seg[0], "real_click": hit["real_click"], **{k: info[k] for k in ("wall_s", "solve_ms", "shown_ms", "z0", "z0_odd", "z0_even")},
                   "handover_title": info["handover_title"], "first_view_title": info["title"]}
            if quick:
                results.append(res)
                break
            res["views"] = views(popup, build, tag, f"{build}_{theme}_{seg[0]}")
            if seg[0] == "pair":
                # the common mode: kisi's mode switch, the same window
                pg.click("#detail [data-pm='c']")
                pg.wait_for_timeout(300)
                pg.select_option("#fv-view", "efield")
                popup, cm = fields(ctx, pg, popup, f"{tag} common mode")
                check("common mode" in cm["handover_title"] and "Even" in popup.evaluate(
                    "(document.getElementById('sim_canvas')._fullLayout.updatemenus || []).filter((m) => m.name === 'mode').map((m) => m.buttons[m.active].label).join()"),
                    f"{tag} common mode: not in the even mode: {cm['handover_title']!r}")
                popup.screenshot(path=os.path.join(out, f"{build}_{theme}_pair_common_efield.png"))
                res["common_mode_title"] = cm["title"]
                pg.click("#detail [data-pm='d']")
            results.append(res)
        if popup:
            popup.wait_for_timeout(500)
    except Exception as e:
        check(False, f"{tag0}: {type(e).__name__}: {str(e)[:400]}")
        try:
            ctx.pages[-1].screenshot(path=os.path.join(out, f"{build}_{theme}_failed.png"))
        except Exception:
            pass
    for e in errs:
        # over http the viewer's HEAD request for a page that is not there is logged by the browser (expected)
        if not (base_url and e.startswith("viewer:") and "status of 404" in e):
            check(False, e)
    b.close()
    if own:
        shutil.rmtree(d, ignore_errors=True)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


with sync_playwright() as pw:
    for build in [b for b in ("full", "lite") if b in pages]:
        for theme in a.themes.split(","):
            t = time.time()
            run(pw, build, theme)
            print(f"{build} {theme}: {time.time() - t:.0f} s", flush=True)
    if not a.no_http:
        # over http: the viewer asks with HEAD which page is there (the lite one alone, or the full one)
        build = "lite" if "lite" in pages else "full"
        d = folder(build)
        srv = socketserver.TCPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=d))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        run(pw, build, a.themes.split(",")[0], base_url=f"http://127.0.0.1:{srv.server_address[1]}", quick=True, d=d)
        srv.shutdown()
        shutil.rmtree(d, ignore_errors=True)

for r in results:
    print(json.dumps(r, ensure_ascii=False))
if a.json:
    json.dump({"results": results, "problems": problems}, open(a.json, "w"), indent=1, ensure_ascii=False)
if a.md:
    lines = ["| run | segment | solve s | shown s | wall s | Z0 / Zodd, Zeven Ω | first view |", "|---|---|---:|---:|---:|---|---|"]
    for r in results:
        z = f"{r['z0']}" if r.get("z0_odd") is None else f"{r['z0_odd']}, {r['z0_even']}"
        lines.append(f"| {r['run']} | {r['segment']} | {(r['solve_ms'] or 0) / 1000:.2f} | {(r['shown_ms'] or 0) / 1000:.2f} | {r['wall_s']} | {z} | "
                     f"{r['first_view_title'][:120]} |")
    lines += ["", f"Problems: {len(problems)}"] + [f"* {p}" for p in problems]
    open(a.md, "w").write("\n".join(lines) + "\n")
print(f"screenshots: {out}")
print("field views check:", "ok" if not problems else f"{len(problems)} problem(s)")
sys.exit(1 if problems else 0)
