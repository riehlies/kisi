#!/usr/bin/env python3
"""Headless check of the field page kisi-advanced.html and kisi's "Fields…" (Playwright, Chromium).

Usage: check_advanced.py <kisi-examples.html> <kisi-advanced.html> [--out DIR] [--themes light,dark] [--json FILE]
                         [--md FILE] [--no-http] [--no-3d] [--quick]

The field page lies next to the viewer in a folder of its own (opened from disk, as a downloaded release is). Its build
(full: every 2D view; quasi-static: potential and |E|, the build CI and local work make without the full-wave modules)
is read from the page. For each theme:
  * 2D: the viewer with the ThunderScope example (kisi-examples.html), the impedance estimate, the option "Advanced
    solvers: field views" on; a real click on a microstrip segment and on a segment of a differential pair; "Fields…":
    the field window opens, the page answers, solves, shows the first view; the pair goes over in the same window in
    the odd mode and then in the common mode. The page's Z0 (Zodd, Zeven) against kisi's own field solver on the same
    geometry text with kisi's settings (solverOnPage, FIELD_TOL, kisi's 1 GHz), reported at the hand-over frequency
    (the lane's Nyquist frequency: the difference there includes the dielectric dispersion of the causal materials);
    and the same geometry handed into the page's quasi-static solver at 1 GHz with the page's own adaptive settings and
    with kisi's (data/field_views.json "adaptive"), which must agree within 0.05 % (first theme).
    Every view the build offers, switched inside the page; no field-view warning; the theme is the viewer's.
  * 3D (unless --no-3d): "Fields…" on a via of the ThunderScope (REFINOUT2, its Vias tab): the same window switches to
    the 3D view, runs the case kisi's exporter cut out (kisi_via_case), shows it; the viewer's status line; the views
    |E|, |H|, S and loss, cut planes x / y / z, the plane slider and the jump list, the animation, the power balance
    (its closure within 1.5 % of the incident power) and the S-parameters; the switch to 2D and back. First theme
    only: the built-in examples (single via and differential pair, coarse mesh) with the pair's mixed-mode table, a
    cancelled run, the warning of a large run, a .kvf saved and loaded again.
  * no console errors in either page; screenshots at deviceScaleFactor 2 (--out).
Then the field page at phone width (390 px, light and dark) in 2D and 3D: no horizontal scroll. Unless --no-http:
the viewer over http from 127.0.0.1 once (it finds the page with a HEAD request), one segment.
Results: --json, --md (tables for a CI summary). Exit code 1 on problems.
"""
import argparse, functools, http.server, json, os, pathlib, re, shutil, socketserver, sys, tempfile, threading, time

from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("viewer")
ap.add_argument("page")
ap.add_argument("--out")
ap.add_argument("--themes", default="light,dark")
ap.add_argument("--json")
ap.add_argument("--md")
ap.add_argument("--no-http", action="store_true")
ap.add_argument("--no-3d", action="store_true")
ap.add_argument("--quick", action="store_true", help="one theme's 2D segment and the 3D board via only")
a = ap.parse_args()
out = a.out or tempfile.mkdtemp(prefix="kisi-advanced-check-")
os.makedirs(out, exist_ok=True)
BUSY = "document.getElementById('busy').hidden === true"
# a microstrip and a pair of the ThunderScope example: net, layer, width (mm)
SEGMENTS = [("microstrip", "Net-(C1012_4-Pad1)", "F.Cu", 0.213), ("pair", "/TS-PCIe Components/PCIe_PET0_N", "F.Cu", 0.19079)]
VIA_NET = "/TS-PCIe Components/REFINOUT2"
VIEWS_FULL = ["geometry", "potential", "efield", "hfield", "jfield", "sfield", "lossfield"]
VIEWS_QS = ["geometry", "potential", "efield"]
QS_TOL = 0.05  # %, the quasi-static page against kisi's field solver on the same geometry at 1 GHz (page: 0.01 Ω)
problems, rows, rows3, results = [], [], [], {"2d": [], "3d": [], "phone": [], "examples": {}}


def check(ok, msg):
    if not ok:
        problems.append(msg)
        print("PROBLEM:", msg, flush=True)
    return ok


head = open(a.page, encoding="utf-8").read(4000)
m = re.search(r"kisi-advanced\.html \((full|quasi-static) build\)", head)
if not m:
    raise SystemExit(f"{a.page}: not a field page of tools/advanced_bundle.py")
BUILD = m.group(1)
FULL = BUILD == "full"


def folder():
    d = tempfile.mkdtemp(prefix="kisi-adv-")
    shutil.copy(a.viewer, os.path.join(d, "kisi.html"))
    shutil.copy(a.page, os.path.join(d, "kisi-advanced.html"))
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


# kisi's own field solver on the geometry of a cross-section (the viewer's backend on the page, kisi's tolerance)
KISI_Z = """async (xi) => { globalThis.__KISI_SPARSE_WASM__ = SPARSE_WASM; const be = solverOnPage();
  const r = await be.solveGeometry(S.an.geometries[S.an.cross_sections[xi].geom].text, { tol: FIELD_TOL });
  return { z0: r.z0 ?? null, zodd: r.zodd ?? null, zeven: r.zeven ?? null, ok: r.ok, error: r.error || null }; }"""
FIELD_DRAWN = """() => { const gd = document.getElementById('sim_canvas'), t = gd && gd.layout && gd.layout.title;
  const title = String(t && (t.text || t) || '');
  return !!gd && (gd.data || []).some((x) => ['heatmap', 'contour'].includes(x.type)) && !/^(Computing|.*not available)/.test(title); }"""
PAGE_INFO = """() => { const gd = document.getElementById('sim_canvas'), log = document.getElementById('console_out').textContent;
  const t = gd.layout && gd.layout.title; const last = (re) => { const m = [...log.matchAll(re)]; return m.length ? +m[m.length - 1][1] : null; };
  return { title: String(t && (t.text || t) || '').replace(/<[^>]+>/g, ' ').replace(/\\s+/g, ' ').slice(0, 300),
           view: window.forkPlot && window.forkPlot.view(), views: window.forkPlot ? window.forkPlot.views() : [],
           theme: document.documentElement.dataset.theme, mode: window.KVF_STATE && window.KVF_STATE.mode,
           handover_title: (document.getElementById('handover-title') || {}).textContent || '',
           param_tol: (document.getElementById('inp_param_tolerance') || {}).value, max_nodes: (document.getElementById('inp_max_nodes') || {}).value,
           field_warning: /Field views:/.test(log), z0: last(/Z0:\\s*([\\d.]+)/g), z0_odd: last(/Z0_odd:\\s*([\\d.]+)/g),
           z0_even: last(/Z0_even:\\s*([\\d.]+)/g) }; }"""


def fields(ctx, pg, popup, tag, errs):
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


def views(popup, tag, shot):
    """every view the build offers, inside the page; screenshots"""
    for v in VIEWS_FULL if FULL else VIEWS_QS:
        ok = popup.evaluate("(v) => window.forkPlot.setView(v)", v)
        if not check(ok, f"{tag}: the page does not offer the view {v} ({popup.evaluate(PAGE_INFO)['views']})"):
            continue
        if v != "geometry":
            try:
                popup.wait_for_function(FIELD_DRAWN, timeout=180000, polling=200)
            except Exception:
                check(False, f"{tag} {v}: no field drawn ({popup.evaluate(PAGE_INFO)['title']!r})")
        popup.wait_for_timeout(700)
        check(not popup.evaluate(PAGE_INFO)["field_warning"], f"{tag} {v}: the page logged a field view warning")
        popup.screenshot(path=os.path.join(out, f"{shot}_{v}.png"))


# a hand-over into the open field window at 1 GHz (kisi's frequency) with given solver settings: the page's Z0 values
LINK_1GHZ = r"""async ([text, extra]) => {
  const set = Object.assign({ tl_type: 'custom', custom_geom: text, mesh_backend: 'rectilinear', freq_start: 1, freq_stop: 1, freq_points: 1,
    custom_sigma: 5.8e7 }, extra);
  const b = new Uint8Array(await new Response(new Blob([JSON.stringify(set)]).stream().pipeThrough(new CompressionStream('deflate-raw'))).arrayBuffer());
  let s = ''; for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
  location.hash = '#params=z.' + btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '') + '&solve=1&view=efield&n=' + Date.now(); }"""
# the page's own defaults (before kisi handed its adaptive mesh over) and kisi's settings as field-views.js hands them over
PAGE_DEFAULTS = {"tolerance": 0.01, "param_tolerance": 0.05, "initial_grid": 30, "max_nodes": 20, "min_converged_passes": 2, "estimate_error": 1}


def at_1ghz(pg, popup, xi, tag, seg):
    """the page at 1 GHz (quasi-static) with its own defaults and with kisi's settings against kisi's field solver"""
    text = pg.evaluate("(xi) => S.an.geometries[S.an.cross_sections[xi].geom].text", xi)
    kisi_set = pg.evaluate("() => { const ad = DATA.field_views.adaptive; return { tolerance: FIELD_TOL, param_tolerance: FIELD_TOL, initial_grid: ad.initial_grid,"
                           " max_nodes: ad.max_nodes / 1000, min_converged_passes: ad.min_converged_passes, estimate_error: ad.estimate_error ? 1 : 0 }; }")
    kz = pg.evaluate(KISI_Z, xi)
    for label, extra in (("page's own settings", PAGE_DEFAULTS), ("kisi's settings", kisi_set)):
        n0 = popup.evaluate("window.__fieldHandover ? window.__fieldHandover.seq : 0")
        popup.evaluate(LINK_1GHZ, [text, extra])
        popup.wait_for_function("(n) => window.__fieldHandover && window.__fieldHandover.seq > n && ['shown', 'error'].includes(window.__fieldHandover.phase)",
                                arg=n0, timeout=600000, polling=200)
        info = popup.evaluate(PAGE_INFO)
        info["handover_title"] = "· 1 GHz ("
        agree(f"{tag}, 1 GHz, {label}", seg, info, kz, strict=label == "kisi's settings")


def agree(tag, seg, info, kz, strict=False):
    """the page's Z0 against kisi's field solver (1 GHz); a table row"""
    if not kz.get("ok"):
        check(False, f"{tag}: kisi's solver failed: {kz.get('error')}")
        return
    pairs = [("Z0", info["z0"], kz["z0"])] if seg == "microstrip" else [("Zodd", info["z0_odd"], kz["zodd"]), ("Zeven", info["z0_even"], kz["zeven"])]
    for name, page, kisi in pairs:
        if page is None or kisi is None:
            check(False, f"{tag}: {name} missing (page {page}, kisi {kisi})")
            continue
        d = 100 * (page - kisi) / kisi
        at = re.search(r"· ([\d.]+ [MG]Hz) \(", info["handover_title"])
        at = at.group(1) if at else "?"
        row = {"run": tag, "segment": seg, "quantity": name, "page": page, "kisi": round(kisi, 4), "diff_pct": round(d, 3), "build": BUILD, "page_at": at}
        rows.append(row)
        if strict and at == "1 GHz":
            check(abs(d) <= QS_TOL, f"{tag}: {name} of the quasi-static page {page} Ω vs kisi {kisi:.3f} Ω ({d:+.3f} %, tolerance {QS_TOL} %)")


def run_2d(ctx, pg, tag0, errs, quick):
    popup = None
    for seg in SEGMENTS[:1] if quick else SEGMENTS:
        hit = click_segment(pg, seg)
        if not hit:
            continue
        tag = f"{tag0} {seg[0]}"
        want = "sfield" if FULL else "efield"
        pg.select_option("#fv-view", want)
        popup, info = fields(ctx, pg, popup, tag, errs)
        check(info["view"] == want, f"{tag}: the page shows {info['view']}, not {want}")
        check(info["mode"] == "2d", f"{tag}: the page is not in its 2D view ({info['mode']})")
        theme = tag0.split()[0]
        check(info["theme"] == theme, f"{tag}: the field page's theme is {info['theme']}, the viewer's {theme}")
        check("kisi" in info["handover_title"] and "adaptive mesh" in info["handover_title"], f"{tag}: the title line {info['handover_title']!r}")
        check(info["param_tol"] == "1" and info["max_nodes"] == "60", f"{tag}: kisi's adaptive settings not in the page (parameter tolerance {info['param_tol']} %, max nodes {info['max_nodes']} k)")
        kz = pg.evaluate(KISI_Z, hit["xi"])
        agree(tag, seg[0], info, kz)
        if theme == a.themes.split(",")[0] and not tag0.endswith("http"):
            at_1ghz(pg, popup, hit["xi"], tag, seg[0])
        pg.screenshot(path=os.path.join(out, f"{theme}_{seg[0]}_viewer.png"))
        res = {"run": tag, "segment": seg[0], "real_click": hit["real_click"], **{k: info[k] for k in ("wall_s", "solve_ms", "shown_ms", "z0", "z0_odd", "z0_even")},
               "kisi": kz, "handover_title": info["handover_title"], "first_view_title": info["title"]}
        results["2d"].append(res)
        if quick:
            break
        n0 = popup.evaluate("window.__fieldHandover ? window.__fieldHandover.seq : 0")
        pg.click("#fv-go")   # the segment's hand-over again (at_1ghz replaced it) for the views
        popup.wait_for_function("(n) => window.__fieldHandover && window.__fieldHandover.seq > n && ['shown', 'error'].includes(window.__fieldHandover.phase)",
                                arg=n0, timeout=600000, polling=200)
        views(popup, tag, f"{theme}_2d_{seg[0]}")
        if seg[0] == "pair":
            pg.click("#detail [data-pm='c']")
            pg.wait_for_timeout(300)
            pg.select_option("#fv-view", "efield")
            popup, cm = fields(ctx, pg, popup, f"{tag} common mode", errs)
            check("common mode" in cm["handover_title"] and "Even" in popup.evaluate(
                "(document.getElementById('sim_canvas')._fullLayout.updatemenus || []).filter((m) => m.name === 'mode').map((m) => m.buttons[m.active].label).join()"),
                f"{tag} common mode: not in the even mode: {cm['handover_title']!r}")
            popup.screenshot(path=os.path.join(out, f"{theme}_2d_pair_common_efield.png"))
            pg.click("#detail [data-pm='d']")
    return popup


CANVAS_HASH = """() => { const c = document.getElementById('k3-cv'); const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
  let h = 0; for (let i = 0; i < d.length; i += 97) h = (h * 31 + d[i]) >>> 0; return h; }"""


def k3_seg(p, sid, v):
    p.click(f"#{sid} button[data-v='{v}']")
    p.wait_for_timeout(250)


def k3_set(p, sid, v):
    p.evaluate("([id, v]) => { const e = document.getElementById(id); e.value = v; e.dispatchEvent(new Event(e.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true })); }", [sid, v])
    p.wait_for_timeout(250)


def balance(p):
    """the power balance's closure (Δ of the balance row, % of incident) and the totals"""
    txt = p.inner_text("#k3-totals")
    m = re.search(r"Δ (-?[\d.]+) %", txt)
    return (float(m.group(1)) if m else None), txt


def run_3d(ctx, pg, popup, theme, errs, first):
    """Fields… on a board via: the field window's 3D view runs the exported case"""
    n = pg.evaluate("(nm) => S.scene.nets.indexOf(nm)", VIA_NET)
    if not check(n >= 0, f"3D: net {VIA_NET} not on the board"):
        return popup
    pg.evaluate("(n) => selectNet(n, true)", n)
    pg.wait_for_timeout(400)
    pg.click("#detail [data-tab='via']")
    try:
        pg.wait_for_function("document.querySelector('button[data-fv3]')", timeout=180000)
    except Exception:
        check(False, f"3D {theme}: no Fields… button in the Vias tab")
        return popup
    t = time.time()
    if popup is None:
        with ctx.expect_page(timeout=60000) as pinfo:
            pg.click("button[data-fv3='0']")
        popup = pinfo.value
        watch(popup, "field page", errs)
    else:
        popup.evaluate("window.KVF_LAST_RUN = null")
        pg.click("button[data-fv3='0']")
    popup.wait_for_function("window.KVF_STATE && window.KVF_STATE.mode === '3d' && window.KVF_STATE.kind === 'board'", timeout=120000)
    popup.wait_for_function("window.KVF_LAST_RUN", timeout=1800000, polling=1000)
    lr = popup.evaluate("window.KVF_LAST_RUN")
    if not check(not lr.get("error"), f"3D {theme}: the run failed: {lr.get('error')}"):
        return popup
    try:
        pg.wait_for_function("/Shown in the field window/.test(document.getElementById('fv3-msg').textContent)", timeout=20000)
    except Exception:
        check(False, f"3D {theme}: the viewer's status line: {pg.inner_text('#fv3-msg')!r}")
    wall = round(time.time() - t, 1)
    pg.screenshot(path=os.path.join(out, f"{theme}_3d_viewer_vias.png"))
    popup.wait_for_timeout(500)
    d = popup.evaluate("({ title: document.getElementById('k3-dstitle').textContent, sub: document.getElementById('k3-dssub').textContent,"
                       " from: document.getElementById('k3-board-from').textContent, est: document.getElementById('k3-estimate').textContent,"
                       " theme: document.documentElement.dataset.theme, sp: document.getElementById('k3-sparams').innerText })")
    check(VIA_NET in d["title"], f"3D {theme}: the case title {d['title']!r}")
    check(d["theme"] == theme, f"3D {theme}: the field page's theme is {d['theme']}")
    clos, tot = balance(popup)
    check(clos is not None and abs(clos) <= 1.5, f"3D {theme}: the power balance closes within {clos} % (tolerance 1.5 %)")
    check("ESTIMATE" in tot.upper() and "conductor loss" in tot, f"3D {theme}: the conductor-loss estimate is not marked")
    shots = []
    h0 = popup.evaluate(CANVAS_HASH)
    popup.screenshot(path=os.path.join(out, f"{theme}_3d_board_E_y.png"))
    shots.append("E y")
    for v, label in (("S", "S"), ("loss", "loss"), ("H", "H")):
        k3_seg(popup, "k3-view", v)
        popup.screenshot(path=os.path.join(out, f"{theme}_3d_board_{label}_y.png"))
    check(popup.evaluate(CANVAS_HASH) != h0, f"3D {theme}: the canvas does not change with the view")
    k3_seg(popup, "k3-view", "E")
    k3_seg(popup, "k3-axis", "2")
    jumps = popup.evaluate("[...document.getElementById('k3-jump').options].map((o) => [o.value, o.textContent])")
    core = [j for j in jumps if "dielectric 3" in j[1]] or [j for j in jumps if j[0]]
    if check(bool(core), f"3D {theme}: no jump list for the z planes"):
        k3_set(popup, "k3-jump", core[0][0])
    p1 = popup.inner_text("#k3-planepos")
    popup.screenshot(path=os.path.join(out, f"{theme}_3d_board_E_z.png"))
    k3_seg(popup, "k3-view", "loss")
    popup.screenshot(path=os.path.join(out, f"{theme}_3d_board_loss_z.png"))
    k3_seg(popup, "k3-view", "E")
    k3_set(popup, "k3-plane", "3")
    check(popup.inner_text("#k3-planepos") != p1, f"3D {theme}: the slider does not move the plane")
    k3_seg(popup, "k3-axis", "0")
    popup.screenshot(path=os.path.join(out, f"{theme}_3d_board_E_x.png"))
    k3_seg(popup, "k3-axis", "1")
    # animation: the phase advances and the canvas changes
    hA = popup.evaluate(CANVAS_HASH)
    popup.click("#k3-play")
    popup.wait_for_timeout(1200)
    popup.click("#k3-play")
    check(popup.inner_text("#k3-phaselab") != "0°" and popup.evaluate(CANVAS_HASH) != hA, f"3D {theme}: the animation does not run")
    k3_seg(popup, "k3-tmode", "avg")
    # the switch to 2D and back
    popup.click("#kisi3d .k3-mode button[data-mode='2d']")
    popup.wait_for_timeout(500)
    check(popup.evaluate("document.getElementById('kisi3d').hidden && !!document.getElementById('kisi3d-switch')"), f"3D {theme}: the switch to 2D failed")
    popup.screenshot(path=os.path.join(out, f"{theme}_switch_2d.png"))
    popup.click("#kisi3d-switch")
    popup.wait_for_timeout(500)
    check(popup.evaluate("!document.getElementById('kisi3d').hidden"), f"3D {theme}: the switch to 3D failed")
    results["3d"].append({"theme": theme, "wall_s": wall, "run": lr, "title": d["title"], "estimate": d["est"], "closure_pct": clos, "sparams": d["sp"], "from": d["from"]})
    rows3.append({"run": f"{theme} board via", "cells": lr["runs"][0]["cells"], "steps": lr["runs"][0]["steps"], "seconds": round(lr["wall_s"], 1),
                  "wall_s": wall, "closure_pct": clos})
    if first:
        examples(popup, theme)
    return popup


def examples(p, theme):
    """the built-in examples, cancel, the large-run warning, .kvf save and load (first theme)"""
    ex = results["examples"]
    for kind in ("single", "pair"):
        k3_seg(p, "k3-kind", kind)
        k3_set(p, "k3-mesh", "coarse")
        p.wait_for_function("!/Estimating/.test(document.getElementById('k3-estimate').textContent)", timeout=60000)
        p.evaluate("window.KVF_LAST_RUN = null")
        t = time.time()
        p.click("#k3-run")
        p.wait_for_function("window.KVF_LAST_RUN", timeout=900000, polling=500)
        lr = p.evaluate("window.KVF_LAST_RUN")
        check(not lr.get("error"), f"example {kind}: {lr.get('error')}")
        clos, _ = balance(p)
        check(clos is not None and abs(clos) <= 1.5, f"example {kind}: the power balance closes within {clos} %")
        ex[kind] = {"wall_s": round(time.time() - t, 1), "run": lr, "closure_pct": clos}
        rows3.append({"run": f"{theme} example {kind}", "cells": lr.get("runs", [{}])[0].get("cells"), "steps": lr.get("runs", [{}])[0].get("steps"),
                      "seconds": round(lr.get("wall_s", 0), 1), "wall_s": ex[kind]["wall_s"], "closure_pct": clos})
        if kind == "pair":
            sp = p.inner_text("#k3-sparams")
            check("Sdd21" in sp and "Scd21" in sp, "example pair: no mixed-mode table")
            k3_set(p, "k3-exc", "0")
            p.screenshot(path=os.path.join(out, f"{theme}_3d_example_pair_odd.png"))
    # a cancelled run
    p.evaluate("window.KVF_LAST_RUN = null")
    k3_set(p, "k3-mesh", "standard")
    p.wait_for_timeout(600)
    p.click("#k3-run")
    p.wait_for_timeout(1500)
    p.click("#k3-cancel")
    p.wait_for_function("window.KVF_LAST_RUN", timeout=60000)
    check(p.evaluate("window.KVF_LAST_RUN.error") == "cancelled", "cancel: the run was not cancelled")
    # the warning of a large run (no limit)
    k3_set(p, "k3-mesh", "fine")
    k3_set(p, "k3-fother", "7, 13, 17, 23, 27, 33, 37, 40")
    p.wait_for_function("!/Estimating/.test(document.getElementById('k3-estimate').textContent)", timeout=60000)
    p.wait_for_timeout(800)
    warn = p.evaluate("document.getElementById('k3-warn').classList.contains('hidden') ? '' : document.getElementById('k3-warn').textContent")
    check("Large run" in warn and not p.evaluate("document.getElementById('k3-run').disabled"), f"large run: no warning, or the run is blocked ({warn!r})")
    k3_set(p, "k3-fother", "")
    k3_set(p, "k3-mesh", "coarse")
    # .kvf: save the pair, load it again
    with p.expect_download() as dl:
        p.click("#k3-save")
    path = os.path.join(out, "saved.kvf")
    dl.value.save_as(path)
    n0 = p.evaluate("window.KVF_STATE.ds.n")
    p.set_input_files("#k3-file", path)
    p.wait_for_function("/loaded from/.test(document.getElementById('k3-dssub').textContent)", timeout=60000)
    check(p.evaluate("window.KVF_STATE.ds.n") == n0, ".kvf: the loaded data differs")
    ex["kvf_bytes"] = os.path.getsize(path)
    os.unlink(path)


def phone(pw, d):
    """the field page at phone width, 2D and 3D, light and dark: no horizontal scroll"""
    for theme in ("light", "dark"):
        b = pw.chromium.launch()
        ctx = b.new_context(viewport={"width": 390, "height": 844}, device_scale_factor=2, color_scheme=theme, is_mobile=True, has_touch=True)
        errs = []
        p = ctx.new_page()
        watch(p, "field page (phone)", errs)
        p.goto(pathlib.Path(d, "kisi-advanced.html").as_uri())
        p.wait_for_function("window.forkPlot && document.getElementById('kisi3d-switch')", timeout=120000)
        p.wait_for_timeout(800)
        w2 = p.evaluate("[document.documentElement.scrollWidth, window.innerWidth]")
        p.screenshot(path=os.path.join(out, f"phone_{theme}_2d.png"))
        p.click("#kisi3d-switch")
        p.wait_for_timeout(600)
        k3_set(p, "k3-mesh", "coarse")
        p.wait_for_function("!/Estimating/.test(document.getElementById('k3-estimate').textContent)", timeout=60000)
        p.click("#k3-run")
        p.wait_for_function("window.KVF_LAST_RUN", timeout=600000, polling=500)
        p.wait_for_timeout(500)
        w3 = p.evaluate("[document.getElementById('kisi3d').scrollWidth, document.getElementById('kisi3d').clientWidth]")
        warn = p.evaluate("document.getElementById('k3-warn').classList.contains('hidden') ? '' : document.getElementById('k3-warn').textContent")
        p.screenshot(path=os.path.join(out, f"phone_{theme}_3d.png"), full_page=False)
        p.evaluate("document.getElementById('k3-cvwrap').scrollIntoView()")
        p.screenshot(path=os.path.join(out, f"phone_{theme}_3d_plane.png"), full_page=False)
        check(w2[0] <= w2[1] + 1, f"phone {theme}: the 2D page scrolls sideways ({w2[0]} > {w2[1]} px)")
        check(w3[0] <= w3[1] + 1, f"phone {theme}: the 3D view scrolls sideways ({w3[0]} > {w3[1]} px)")
        check("phone or tablet" in warn or "Large run" in warn, f"phone {theme}: no phone warning ({warn!r})")
        results["phone"].append({"theme": theme, "2d": w2, "3d": w3, "warning": warn[:120]})
        for e in errs:
            check(False, e)
        b.close()


def run(pw, theme, first, base_url=None, quick=False, d=None):
    errs = []
    own = d is None
    d = d or folder()
    url = (base_url or pathlib.Path(d).as_uri()) + "/kisi.html"
    b = pw.chromium.launch()
    ctx = b.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=2, color_scheme=theme, accept_downloads=True)
    tag0 = f"{theme}{' http' if base_url else ''}"
    try:
        pg = open_viewer(ctx, url, errs)
        popup = run_2d(ctx, pg, tag0, errs, quick or base_url is not None)
        if not a.no_3d and base_url is None:
            run_3d(ctx, pg, popup, theme, errs, first and not quick)
    except Exception as e:
        check(False, f"{tag0}: {type(e).__name__}: {str(e)[:400]}")
        try:
            ctx.pages[-1].screenshot(path=os.path.join(out, f"{theme}_failed.png"))
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
    themes = a.themes.split(",")[:1] if a.quick else a.themes.split(",")
    for i, theme in enumerate(themes):
        t = time.time()
        run(pw, theme, i == 0, quick=a.quick)
        print(f"{theme}: {time.time() - t:.0f} s", flush=True)
    d = folder()
    if not a.quick:
        t = time.time()
        phone(pw, d)
        print(f"phone: {time.time() - t:.0f} s", flush=True)
    if not a.no_http:
        srv = socketserver.TCPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=d))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        run(pw, themes[0], False, base_url=f"http://127.0.0.1:{srv.server_address[1]}", quick=True, d=d)
        srv.shutdown()
    shutil.rmtree(d, ignore_errors=True)

print(json.dumps({"build": BUILD, "agreement": rows, "3d": rows3}, ensure_ascii=False, indent=1))
if a.json:
    json.dump({"build": BUILD, "agreement": rows, "runs_3d": rows3, "results": results, "problems": problems}, open(a.json, "w"), indent=1, ensure_ascii=False)
if a.md:
    lines = [f"Field page: {BUILD} build.", "", "**2D: the page's impedance against kisi's field solver on the same geometry** "
             f"(at the hand-over frequency with the {'full-wave' if FULL else 'quasi-static'} solver: reported; quasi-static at 1 GHz with "
             f"kisi's adaptive mesh: within {QS_TOL} %)", "",
             "| run | segment | quantity | page Ω (at) | kisi Ω (1 GHz) | Δ % |", "|---|---|---|---:|---:|---:|"]
    lines += [f"| {r['run']} | {r['segment']} | {r['quantity']} | {r['page']} ({r['page_at']}) | {r['kisi']} | {r['diff_pct']:+.3f} |" for r in rows]
    lines += ["", "**3D: runs in the browser**", "", "| run | cells | steps | run s | wall s | balance closure % |", "|---|---:|---:|---:|---:|---:|"]
    lines += [f"| {r['run']} | {r['cells']} | {r['steps']} | {r['seconds']} | {r['wall_s']} | {r['closure_pct']} |" for r in rows3]
    lines += ["", f"Problems: {len(problems)}"] + [f"* {p}" for p in problems]
    open(a.md, "w").write("\n".join(lines) + "\n")
print(f"screenshots: {out}")
print("field page check:", "ok" if not problems else f"{len(problems)} problem(s)")
sys.exit(1 if problems else 0)
