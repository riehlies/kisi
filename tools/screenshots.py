#!/usr/bin/env python3
"""Screenshots for the README and the GitHub Pages landing page (docs/img/*.png).

Usage: screenshots.py <kisi-examples.html> [--out docs/img] [--theme light|dark] [--net NAME] [--rail NAME]

Opens the viewer with the built-in ThunderScope board (Playwright + Chromium, 1440×900 saved as 1600×1000) and saves
impedance.png (whole board coloured by deviation from the target), net.png (TDR of a net with vias),
channel.png (interfaces after the check), flow.png (power flow), power.png (voltage drop of the FPGA core rail,
its impedance seen from the ICs), temperature.png (temperature estimate, step 3), parts.png (the power tree's
components with their thermal columns) and prepare.png (the Prepare PCB page, Impedance tab, with the laminate
and a dismissal being entered). --only prepare,… makes only these.
"""
import argparse, os
from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("html")
ap.add_argument("--out", default="docs/img")
ap.add_argument("--theme", default="light", choices=["light", "dark"])
ap.add_argument("--rail", default="+1V0", help="rail for the power screenshot")
ap.add_argument("--net", default="/TS-USB4 Components/M2_PER0_P", help="net for the TDR screenshot")
ap.add_argument("--only", default="", help="comma-separated subset of the screenshots (e.g. prepare)")
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)
BUSY = "document.getElementById('busy').hidden === true"

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1440, "height": 900}, device_scale_factor=1600 / 1440, color_scheme=a.theme)
    pg.goto("file://" + os.path.abspath(a.html))
    pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    if pg.get_attribute("#sample-ts", "aria-pressed") != "true":
        pg.click("#sample-ts"); pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=90000)
    only = set(filter(None, a.only.split(",")))
    shot = lambda name: (not only or name in only) and pg.screenshot(path=os.path.join(a.out, name + ".png"))

    def prepare():
        # the Prepare PCB page on the Impedance tab: the laminate partly entered, the QSPI nets being dismissed
        pg.wait_for_function("S.prep.base", timeout=90000)
        pg.click("#prep-btn"); pg.click('#prep-tabs [data-prep-tab="impedance"]'); pg.wait_for_timeout(300)
        if pg.locator('[data-dismiss="nets-without-target"]').count():
            pg.click('[data-dismiss="nets-without-target"]'); pg.click('[data-dis-all="nets-without-target"]')
            pg.fill('[data-dis-reason="nets-without-target"]', "QSPI configuration flash at 50 MHz, short lines – no impedance control needed")
        if pg.locator('[data-src="stackup"]').count():
            pg.select_option('[data-src="stackup"] [data-k="kind"]', "datasheet")
            pg.fill('[data-src="stackup"] [data-k="document"]', "Isola FR408HR data sheet")
            pg.fill('[data-src="stackup"] [data-k="note"]', "10 GHz, typical")
            for i in (0, 1):
                pg.fill(f'input[data-ed="stackup"][data-i="{i}"][data-k="loss_tangent"]', "0.0095")
        pg.wait_for_timeout(1200); pg.wait_for_function("!S.prep.busy", timeout=90000); pg.wait_for_timeout(300)
        pg.evaluate("document.getElementById('prep').scrollTop = 0")
        shot("prepare")
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
    b.close()
print("screenshots in", a.out)
