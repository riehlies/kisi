#!/usr/bin/env python3
"""Screenshots for the README and the GitHub Pages landing page (docs/img/*.png).

Usage: screenshots.py <kisi-examples.html> [--out docs/img] [--theme light|dark] [--net NAME] [--rail NAME]
                      [--advanced kisi-advanced.html]

Opens the viewer with the built-in ThunderScope board (Playwright + Chromium, 1440×900 saved as 1600×1000) and saves
impedance.png (whole board coloured by deviation from the target), net.png (TDR of a net with vias),
channel.png (interfaces after the check), flow.png (power flow), power.png (voltage drop of the FPGA core rail,
its impedance seen from the ICs), temperature.png (temperature estimate, step 3), parts.png (the power tree's
components with their thermal columns), prepare.png (the Prepare PCB checklist of the bare board, Impedance
tab: a class for the QSPI nets applied, the laminate step open) and stackup.png (the Stackup view with the option
"Fab stackups & tolerances" on: the board's stackup next to a fab template). With --advanced (the field page next to
a copy of the viewer, as on a release): fields-2d.png ("Fields…" on a segment of a PCIe pair: |E| with its field lines,
a view of both builds) and fields-3d.png ("Fields…" on a via of REFINOUT2: the 3D run, |E| on the cut plane).
--only prepare,… makes only these.
"""
import argparse, os, shutil, tempfile
from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("html")
ap.add_argument("--out", default="docs/img")
ap.add_argument("--theme", default="light", choices=["light", "dark"])
ap.add_argument("--rail", default="+1V0", help="rail for the power screenshot")
ap.add_argument("--net", default="/TS-USB4 Components/M2_PER0_P", help="net for the TDR screenshot")
ap.add_argument("--only", default="", help="comma-separated subset of the screenshots (e.g. prepare)")
ap.add_argument("--advanced", help="kisi-advanced.html: also fields-2d.png and fields-3d.png")
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)
BUSY = "document.getElementById('busy').hidden === true"
FIELDS = {"fields-2d", "fields-3d"}
# a segment of a PCIe pair (net, layer, width in mm) and a via of the ThunderScope example (as tools/check_advanced.py)
PAIR = ("/TS-PCIe Components/PCIe_PET0_N", "F.Cu", 0.19079)
VIA_NET = "/TS-PCIe Components/REFINOUT2"
VIEW = {"width": 1440, "height": 900}


def fields(b, only, shoot):
    """"Fields…" in the viewer next to the field page (a folder of its own, as a downloaded release): a pair's
    cross-section in 2D, a via in 3D"""
    d = tempfile.mkdtemp(prefix="kisi-shots-")
    shutil.copy(a.html, os.path.join(d, "kisi.html"))
    shutil.copy(a.advanced, os.path.join(d, "kisi-advanced.html"))
    ctx = b.new_context(viewport=VIEW, device_scale_factor=1600 / 1440, color_scheme=a.theme)
    pg = ctx.new_page()
    pg.goto("file://" + os.path.join(d, "kisi.html"))
    pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    if pg.get_attribute("#sample-ts", "aria-pressed") != "true":
        pg.click("#sample-ts"); pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    pg.click("#view-si")
    pg.click("#an-start"); pg.wait_for_function(f"S.full && {BUSY}", timeout=90000)
    if not pg.evaluate("$('fv-on').checked"):
        if not pg.evaluate("$('fs-opts').open"):
            pg.click("#fs-opts > summary")
        pg.check("#fv-on")
    # 2D: the pair's longest piece as the clicked segment, its row's "Fields…" in the Impedance tab
    hit = pg.evaluate("""([net, layer, w]) => {
      const n = S.scene.nets.indexOf(net), pc = S.pieces; let best = null;
      for (let i = 0; i < pc.n; i++) {
        if (pc.net[i] !== n) continue;
        const xs = S.an.cross_sections[pc.xs[i]];
        if (xs.geom == null || S.scene.stackup.copper[xs.layer].name !== layer || Math.abs(xs.w - w) > 1e-4) continue;
        const len = Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
        if (!best || len > best.len) best = { piece: i, net: n, len };
      }
      return best; }""", list(PAIR))
    if not hit:
        raise SystemExit(f"fields: segment of {PAIR[0]} not found")
    pg.evaluate("([p, n]) => { S.detailTab = 'z'; S.fvPiece = p; selectNet(n, true); }", [hit["piece"], hit["net"]])
    pg.wait_for_timeout(500)
    with ctx.expect_page(timeout=60000) as pinfo:
        pg.click("#tab-body tr.clicked [data-fv2]")
    pop = pinfo.value
    pop.wait_for_function("window.__fieldHandover && window.__fieldHandover.phase === 'ready'", timeout=120000, polling=200)
    pop.click("#btn_solve")
    pop.wait_for_function("window.__fieldHandover && ['shown', 'error'].includes(window.__fieldHandover.phase)", timeout=1800000, polling=200)
    if pop.evaluate("window.__fieldHandover.phase") != "shown":
        raise SystemExit(f"fields: 2D hand-over failed: {pop.evaluate('window.__fieldHandover.error')}")
    pop.evaluate("window.forkPlot.setView('efield')")
    pop.wait_for_function("""() => { const gd = document.getElementById('sim_canvas'), t = gd && gd.layout && gd.layout.title;
      return !!gd && (gd.data || []).some((x) => ['heatmap', 'contour'].includes(x.type)) && !/^(Computing|.*not available)/.test(String(t && (t.text || t) || '')); }""",
                          timeout=180000, polling=200)
    pop.wait_for_timeout(800)
    shoot(pop, "fields-2d")
    # 3D: "Fields…" on the first via of REFINOUT2 (its Vias tab) into the same window, then Run
    if not only or "fields-3d" in only:
        pg.evaluate("(nm) => selectNet(S.scene.nets.indexOf(nm), true)", VIA_NET)
        pg.wait_for_timeout(400)
        pg.click("#detail [data-tab='via']")
        pg.wait_for_function("document.querySelector('button[data-fv3]')", timeout=180000)
        pg.click("button[data-fv3='0']")
        pop.wait_for_function("window.KVF_STATE && window.KVF_STATE.mode === '3d' && window.KVF_STATE.kind === 'board'", timeout=120000)
        pop.wait_for_function("!/Estimating/.test(document.getElementById('k3-estimate').textContent) && !document.getElementById('k3-run').disabled", timeout=120000)
        pop.evaluate("window.KVF_LAST_RUN = null")
        pop.click("#k3-run")
        pop.wait_for_function("window.KVF_LAST_RUN", timeout=1800000, polling=1000)
        err = pop.evaluate("window.KVF_LAST_RUN.error")
        if err:
            raise SystemExit(f"fields: 3D run failed: {err}")
        pop.wait_for_timeout(800)
        shoot(pop, "fields-3d")
    ctx.close()
    shutil.rmtree(d, ignore_errors=True)

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1440, "height": 900}, device_scale_factor=1600 / 1440, color_scheme=a.theme)
    pg.goto("file://" + os.path.abspath(a.html))
    pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    if pg.get_attribute("#sample-ts", "aria-pressed") != "true":
        pg.click("#sample-ts"); pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    only = set(filter(None, a.only.split(",")))
    shot = lambda name: (not only or name in only) and pg.screenshot(path=os.path.join(a.out, name + ".png"))
    if a.advanced and (not only or only & FIELDS):
        fields(b, only, lambda page, name: (not only or name in only) and page.screenshot(path=os.path.join(a.out, name + ".png")))
    if only and only <= FIELDS:
        b.close(); print("screenshots in", a.out); raise SystemExit(0)

    def prepare():
        # the Prepare PCB checklist on the bare ThunderScope (without its kisi.json), Impedance tab: the QSPI nets
        # given a class of their own and applied (check mark), the laminate step open with a value and its source
        pg.wait_for_function("S.prep.base", timeout=90000)
        idle = lambda: (pg.wait_for_timeout(300), pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=90000))
        tick = lambda: pg.evaluate("() => new Promise((r) => setTimeout(r, 0))")
        def commit(sel, v):
            pg.fill(sel, v); pg.dispatch_event(sel, "change"); tick()
        pg.evaluate("""async () => { for (const k of Object.keys(localStorage)) if (k.startsWith('kisi.prep.v1.')) localStorage.removeItem(k);
          S.prep.mem = {}; S.sample = null; for (const k of ['ts', 'vna']) document.getElementById('sample-' + k).setAttribute('aria-pressed', 'false');
          const s = SAMPLES.ts; const [a, c] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]); await loadFiles(a, c, s.file || s.name, null, null); }""")
        pg.wait_for_function(f"S.scene && {BUSY} && S.prep.base", timeout=120000); idle()
        pg.click("#prep-btn"); pg.click('#prep-tabs [data-prep-tab="impedance"]'); pg.wait_for_timeout(300)
        g = 'article[data-gap="nets-without-target"]'
        if pg.locator(g).count():
            commit(f'{g} input[data-a="nc"][data-k="name"]', "QSPI")
            commit(f'{g} input[data-a="nc"][data-k="extra"]', "QSPI_*")
            commit(f'{g} input[data-a="nc"][data-k="target_ohm"]', "50")
            pg.select_option(f'{g} select[data-a="nc"][data-k="profile"]', "se50"); tick()
            pg.select_option(f'{g} [data-a="shared"][data-k="kind"]', "assumption"); tick()
            commit(f'{g} [data-a="shared"][data-k="note"]', "single-ended 50 Ω, common practice for QSPI flash traces")
            pg.click(f'{g} [data-b="apply"]'); idle()
        sel = 'article[data-gap="stackup-defaults"]'
        if pg.locator(sel).count():
            if pg.get_attribute(sel, "aria-expanded") != "true":
                pg.click(f"{sel} > header")
            pg.select_option(f'{sel} [data-a="shared"][data-k="kind"]', "datasheet"); tick()
            commit(f'{sel} [data-a="shared"][data-k="document"]', "Isola FR408HR data sheet, Rev. F")
            commit(f'{sel} [data-a="shared"][data-k="note"]', "Dk at 2 GHz, typical")
            commit(f'{sel} input[data-a="diel"][data-i="2"][data-k="epsilon_r"]', "3.68")
        idle()
        pg.evaluate("document.getElementById('prep').scrollTop = 0")
        shot("prepare")
        pg.evaluate("() => { lsSet(PREP_KEY(), null); S.prep.mem = {}; }")
        pg.evaluate("loadSample('ts')"); pg.wait_for_function(f"S.scene && {BUSY}", timeout=120000)
        pg.click("#view-si"); pg.wait_for_timeout(300)

    if only and only <= {"prepare"}:
        prepare(); b.close(); print("screenshots in", a.out); raise SystemExit(0)

    pg.click("#view-si")
    pg.click("#an-start"); pg.wait_for_function(f"S.full && {BUSY}", timeout=90000)
    pg.evaluate("setMode('dev')"); pg.wait_for_timeout(300)
    shot("impedance")
    # one net with vias: TDR in the detail panel (the view zooms to the net)
    net = pg.evaluate(f"(() => {{ const n = S.an.nets.find((x) => x.name === {a.net!r}); return n ? n.net : -1; }})()")
    if net >= 0:
        pg.evaluate(f"selectNet({net})"); pg.wait_for_timeout(500)
        pg.click("#detail [data-tab=tdr]"); pg.wait_for_timeout(600)
        shot("net")

    pg.click("#view-ch"); pg.wait_for_timeout(500)
    pg.click("#ck-start"); pg.wait_for_function("!S.checkRunning && S.laneRes", timeout=300000); pg.wait_for_timeout(400)
    shot("channel")

    pg.click("#view-pi"); pg.wait_for_timeout(400)
    if not pg.locator("#pi-run").is_disabled():
        pg.click("#pi-run"); pg.wait_for_function(f"S.piSolved && !S.piRunning && {BUSY}", timeout=300000)
    pg.click("#pim-b"); pg.wait_for_timeout(600)
    shot("flow")
    # voltage drop of the FPGA core rail and its impedance seen from the ICs
    pg.evaluate("(() => { const RAIL = '" + a.rail + "'; const r = [...S.piSetups.keys()].find((n) => n === RAIL) || S.piRail; S.piRail = r; S.piLayer = -1; S.piImg = null;"
                " setPiMode('v'); fitRail(); S.piTab = 'z'; renderPi(); renderPiDetail(); })()")
    pg.wait_for_timeout(500)
    if pg.locator("#pdn-ic-run").count():
        pg.click("#pdn-ic-run")
        pg.wait_for_function("document.getElementById('ch-pdn-ic') || document.querySelector('#pdn-ic .err')", timeout=300000)
        pg.wait_for_timeout(600)
        # the whole rail on the map, the chart seen from the IC in the panel
        pg.evaluate("""(() => { fitRail(); const c = document.getElementById('ch-pdn-ic'), p = document.getElementById('pi-detail');
          if (c && p) p.scrollTop += c.getBoundingClientRect().top - p.getBoundingClientRect().top - 160; })()""")
        pg.wait_for_timeout(600)
    shot("power")
    # temperature (step 3): the board's temperature map with the hottest parts and the panel with parts and rails
    if pg.locator("#therm-run").count() and not pg.locator("#therm-run").is_disabled():
        pg.click("#therm-run"); pg.wait_for_function(f"typeof thermOk === 'function' && thermOk() && {BUSY}", timeout=300000)
        pg.click("#pim-t"); pg.wait_for_timeout(800)
        shot("temperature")
        # the power tree's components with their thermal columns (Tj, case, θ and where they come from)
        pg.click("#pim-b"); pg.wait_for_timeout(600)
        if pg.locator("[data-cm=t]").count():
            pg.click("[data-cm=t]"); pg.wait_for_timeout(500)
            # scroll the power view (#pi-balance) so that the power tree and the components card fill the screen
            pg.evaluate("""(() => { const v = document.getElementById('pi-balance'), c = document.querySelector('.bal-comp');
              if (v && c) v.scrollTop += c.getBoundingClientRect().top - v.getBoundingClientRect().top - 16; })()""")
            pg.wait_for_timeout(400)
            shot("parts")
    prepare()
    # the Stackup view (option "Fab stackups & tolerances" on): the board's stackup next to the first fitting fab
    # template, the impedance per net class and layer
    if not only or "stackup" in only:
        if pg.is_visible("#an-start"):
            pg.click("#an-start")
        pg.wait_for_function(f"S.full && {BUSY}", timeout=90000)
        pg.check("#fab-on"); pg.wait_for_timeout(500)
        pg.click("#view-stk"); pg.wait_for_timeout(300)
        fit = pg.evaluate("(stkPresets().fitting || [])[0] || null")
        if fit:
            pg.select_option("#stk-add", f"preset:{fit}"); pg.wait_for_timeout(400)
        pg.evaluate("document.getElementById('stk-panel').scrollTop = 0")
        shot("stackup")
    b.close()
print("screenshots in", a.out)
