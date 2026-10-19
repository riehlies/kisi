#!/usr/bin/env python3
"""Headless check of the bundled viewer (Playwright + Chromium).

Usage: check_viewer.py <kisi.html> [--board board.kicad_pcb [board.kicad_pro]] [--config board.kisi.json]
                       [--out DIR] [--quick]

* light, dark and phone layout; all three views (impedance, channel, power)
* impedance estimate, net detail tabs (a differential pair: 4-port loss, common-mode TDR), interfaces overview, channel check, lane eye,
  voltage drop with ground return, losses map and power flow (desktop layouts; --quick: only
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
            if pg.locator("#loss-cm").count():
                pg.click("#loss-cm"); pg.wait_for_timeout(300)
            else:
                problems.append(f"{tag}: differential net without the 4-port loss view")
            step("pair_loss")
            pg.click("#detail [data-tab=tdr]"); pg.wait_for_timeout(400)
            if pg.locator("#tdr-c").count():
                pg.click("#tdr-c"); pg.wait_for_timeout(300); step("pair_tdr_common"); pg.click("#tdr-d"); pg.wait_for_timeout(200)
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
