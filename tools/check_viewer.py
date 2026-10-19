#!/usr/bin/env python3
"""Headless check of the bundled viewer (Playwright + Chromium).

Usage: check_viewer.py <kisi.html> [--board board.kicad_pcb [board.kicad_pro]] [--config board.kisi.json]
                       [--out DIR] [--quick]

* light, dark and phone layout; all three views (impedance, channel, power)
* impedance estimate, net detail tabs (a differential pair: 4-port loss, skew, common mode), interfaces overview, channel check, lane eye,
  voltage drop with ground return, impedance seen from the ICs, losses map and power flow (desktop layouts; --quick: only
  load and the three views)
* uses the embedded example boards if the build has them, else --board
* --config: drops a <board>.kisi.json and expects "check passed"
* exit code 1 on page errors or console errors (failed loads of external resources
  such as web fonts are only reported); screenshots go to --out (default: a temp dir)
"""
import argparse, os, sys, tempfile
from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("html")
ap.add_argument("--board", nargs="+")
ap.add_argument("--config")
ap.add_argument("--out")
ap.add_argument("--quick", action="store_true")
a = ap.parse_args()
out = a.out or tempfile.mkdtemp(prefix="kisi-check-")
os.makedirs(out, exist_ok=True)
url = "file://" + os.path.abspath(a.html)
BUSY = "document.getElementById('busy').hidden === true"
problems, notes = [], []


def start_screen(pg):
    """Trigger every control on the start screen (also hidden ones, as keyboard or scripts
    could): without a board none of them may fail."""
    pg.evaluate("""async () => {
      for (const e of document.querySelectorAll('button, select, input, [data-tab]')) {
        if (['load-files', 'file'].includes(e.id) || e.closest('.samples')) continue;
        e.dispatchEvent(new MouseEvent('click', {bubbles: true}));
        if (e.tagName === 'SELECT' || e.tagName === 'INPUT') ['change', 'input'].forEach((t) => e.dispatchEvent(new Event(t, {bubbles: true})));
        await new Promise((r) => setTimeout(r, 5));
      }
    }""")
    pg.wait_for_timeout(300)


def run(pg, tag, desktop):
    errs = []
    pg.on("pageerror", lambda e: errs.append(f"pageerror: {e}"))
    pg.on("console", lambda m: errs.append(f"console {m.type}: {m.text}") if m.type == "error" else None)
    pg.goto(url)
    pg.wait_for_function("document.getElementById('busy').hidden === true || document.getElementById('drop').classList.contains('start')", timeout=90000)
    if not pg.evaluate("!!S.scene"):  # start screen: no control may fail without a board
        start_screen(pg)
    if a.board:
        pg.set_input_files("#file", [os.path.abspath(f) for f in a.board])
        pg.wait_for_function(f"S.scene && {BUSY}", timeout=90000)
    elif not pg.evaluate("!!S.scene"):
        raise SystemExit("the build has no example boards: pass --board")
    step = lambda name: pg.screenshot(path=f"{out}/{tag}_{name}.png")
    for v in ["si", "ch", "pi"]:
        pg.click(f"#view-{v}"); pg.wait_for_timeout(300); step(f"start_{v}")
    pg.click("#view-si")
    pg.click("#an-start"); pg.wait_for_function(f"S.full && {BUSY}", timeout=90000); step("estimate")
    if desktop and not a.quick:
        pg.click(".net >> nth=0"); pg.wait_for_timeout(400)
        for t in ["loss", "tdr", "via", "xt"]:
            pg.click(f"#detail [data-tab={t}]"); pg.wait_for_timeout(300)
        step("net")
        # a differential pair: 4-port loss view, differential and common-mode TDR
        dn = pg.evaluate("(() => { for (const n of S.an.nets) { const t = classTarget(n.net); if (t && t.diff) return n.net; } return -1; })()")
        if dn >= 0:
            pg.evaluate(f"selectNet({dn})"); pg.wait_for_timeout(400)
            pg.click("#detail [data-tab=loss]"); pg.wait_for_timeout(500)
            if not pg.locator("#ts-export [data-dl=s4p]").count():
                problems.append(f"{tag}: differential net without the 4-port export")
            else:
                pg.click("#ts-export summary"); pg.wait_for_timeout(300)
            step("pair_loss")
            pg.click("#detail [data-tab=skew]"); pg.wait_for_timeout(500); step("pair_skew")
            pg.click("#detail [data-sv=tdr]"); pg.wait_for_timeout(400); step("pair_skew_tdr"); pg.click("#detail [data-sv=scd]"); pg.wait_for_timeout(200)
            # common mode for Impedance, Loss and TDR (the switch is greyed out in the other tabs)
            if not pg.evaluate("document.querySelector('#detail [data-pm=c]').disabled"):
                problems.append(f"{tag}: mode switch active in the Skew tab")
            pg.click("#detail [data-tab=z]"); pg.wait_for_timeout(300)
            pg.click("#detail [data-pm=c]"); pg.wait_for_timeout(400)
            for t in ["z", "loss", "tdr"]:
                pg.click(f"#detail [data-tab={t}]"); pg.wait_for_timeout(400)
            step("pair_common")
            pg.click("#detail [data-pm=d]"); pg.wait_for_timeout(300)
    # couplings: scan with step 2, Couplings card, crosstalk map of the critical stretches,
    # the net's Crosstalk tab; refinement of the critical couplings planned in step 3 (no solve)
    pg.wait_for_function("S.xtScan", timeout=60000)
    if pg.evaluate("S.xtScan.error || !S.xtScan.rows.length"):
        problems.append(f"{tag}: coupling scan empty or failed: {pg.evaluate('S.xtScan.error')}")
    if pg.evaluate("S.xtScan.rows.some((r) => r.critical) && (!S.xtScan.map.length || !document.querySelector('#xt-rows tr[data-net]'))"):
        problems.append(f"{tag}: critical nets without stretches on the map or rows in the Couplings card")
    pg.evaluate("setMode('xt')"); pg.wait_for_timeout(300)
    if pg.get_attribute("#mode-xt", "aria-pressed") != "true":
        problems.append(f"{tag}: crosstalk map mode not active")
    step("couplings")
    if not pg.evaluate("$('fs-xt').checked"):
        problems.append(f"{tag}: coupling refinement not on by default")
    if pg.evaluate("S.xtScan.critical_geoms.length > 0 && !scopeGeoms('target').some((x) => x.xt)"):
        problems.append(f"{tag}: critical couplings not planned for the field solver")
    # follow-up round: couplings that turn critical after a run are queued without a click
    if pg.evaluate("S.xtScan.critical_geoms.length > 0") and not pg.evaluate("(() => { const p = new FieldPool(); S.pool = p; p.scope = 'target'; p.round = 1; p.followUp(); const ok = p.round === 2 && p.total > 0; p.stop(); return ok; })()"):
        problems.append(f"{tag}: no follow-up round for new critical couplings")
    # a short real run (3 cross-sections): results applied, then the idle threads are ended
    # (here after 0.5 s instead of a minute)
    if tag == "light":
        pg.evaluate("(() => { S.poolIdleMs = 500; S.pool = new FieldPool(); S.pool.start(scopeGeoms('target').filter((x) => !S.field.has(x.id)).slice(0, 3)); })()")
        pg.wait_for_function("S.pool === null || (!S.pool.running && S.fieldPending.size === 0)", timeout=300000)
        pg.wait_for_timeout(1500)
        if pg.evaluate("S.field.size") < 3 or not pg.evaluate("S.pool === null"):
            problems.append(f"{tag}: short field solver run: {pg.evaluate('S.field.size')} results, threads ended: {pg.evaluate('S.pool === null')}")
        pg.evaluate("delete S.poolIdleMs")
    if desktop and not a.quick and pg.locator("#xt-rows tr[data-net]").count():
        pg.click("#xt-rows tr[data-net] >> nth=0"); pg.wait_for_timeout(600)
        if pg.evaluate("S.detailTab") != "xt" or not pg.locator("#ch-xt").count():
            problems.append(f"{tag}: critical net without the crosstalk chart")
        if pg.locator("#tab-body tr[data-net]").count():
            pg.hover("#tab-body tr[data-net] >> nth=0"); pg.wait_for_timeout(300)
        step("net_xt")
        if pg.locator("#detail [data-pm=c]").count():
            if pg.evaluate("document.querySelector('#detail [data-pm=c]').disabled"):
                problems.append(f"{tag}: mode switch greyed out in the Crosstalk tab")
            pg.click("#detail [data-pm=c]"); pg.wait_for_timeout(500); step("net_xt_common")
            pg.click("#detail [data-pm=d]"); pg.wait_for_timeout(300)
    pg.evaluate("setMode('dev')"); pg.wait_for_timeout(200)
    if desktop and not a.quick:
        pg.click("#view-ch"); pg.wait_for_timeout(500)
        pg.click("#ov-check"); pg.wait_for_function("!S.checkRunning && S.laneRes", timeout=300000); pg.wait_for_timeout(300); step("check")
        if pg.locator("#ch-overview tr[data-lane]").count():
            pg.click("#ch-overview tr[data-lane] >> nth=0"); pg.wait_for_timeout(600)
            if pg.locator("#ch-detail [data-ct=eye]").count():
                pg.click("#ch-detail [data-ct=eye]"); pg.wait_for_timeout(600)
            step("lane")
        pg.click("#view-pi"); pg.wait_for_timeout(400)
        if not pg.locator("#pi-run").is_disabled():
            pg.click("#pi-run"); pg.wait_for_function(f"S.piSolved && !S.piRunning && {BUSY}", timeout=300000); pg.wait_for_timeout(300)
        step("power")
        # impedance of the selected rail seen from its ICs
        pg.evaluate("if (S.piRail) { S.piTab = 'z'; renderPiDetail(); }"); pg.wait_for_timeout(500)
        if pg.locator("#pdn-ic-run").count():
            pg.click("#pdn-ic-run"); pg.wait_for_function("document.getElementById('ch-pdn-ic') || document.querySelector('#pdn-ic .err')", timeout=300000)
            pg.wait_for_timeout(300); step("pdn_ic")
            pg.evaluate("S.piTab = 'dc'; renderPiDetail()")
        if not pg.locator("#pim-l").is_disabled():
            pg.click("#pim-l"); pg.wait_for_timeout(500); step("losses")
            pg.click("#pim-b"); pg.wait_for_timeout(500); step("flow")
        if a.config:
            pg.click("#view-si")
            pg.set_input_files("#file", [os.path.abspath(a.config)]); pg.wait_for_function(f"S.config && {BUSY}", timeout=90000)
            ck = pg.evaluate("S.config.check")
            if not ck["ok"]:
                problems.append(f"{tag}: kisi.json check failed: {ck['errors'][:3]}")
            step("config")
    if pg.evaluate("document.documentElement.scrollWidth > window.innerWidth + 1"):
        problems.append(f"{tag}: horizontal scroll")
    for e in errs:
        (notes if "Failed to load resource" in e else problems).append(f"{tag}: {e}")


with sync_playwright() as p:
    b = p.chromium.launch()
    for scheme, w, h, tag in [("light", 1440, 900, "light"), ("dark", 1440, 900, "dark"), ("light", 400, 860, "phone")]:
        pg = b.new_page(viewport={"width": w, "height": h}, color_scheme=scheme)
        run(pg, tag, w > 800)
        pg.close()
        print(f"{tag}: done")
    b.close()
for n in sorted(set(notes)):
    print("note:", n)
for pr in problems:
    print("PROBLEM:", pr)
print(f"screenshots: {out}")
sys.exit(1 if problems else 0)
