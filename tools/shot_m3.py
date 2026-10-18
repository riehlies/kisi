"""Headless check: quick analysis, supply view, on-demand solver, cache.
python3 shot_m3.py <kisi.html> <outdir>"""
import sys, time
from playwright.sync_api import sync_playwright
src, out = sys.argv[1], sys.argv[2]
open(out + "/wrapped.html", "w", encoding="utf-8").write("<!doctype html><html><head><meta charset='utf-8'><style>[hidden]{display:none!important}body{margin:0}</style></head><body>" + open(src, encoding="utf-8").read() + "</body></html>")
with sync_playwright() as p:
    b = p.chromium.launch()
    ctx = b.new_context(viewport={"width": 1440, "height": 900})
    pg = ctx.new_page()
    errs = []
    pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.on("console", lambda m: errs.append(m.text) if m.type == "error" and "TUNNEL" not in m.text else None)
    t = time.time()
    pg.goto("http://127.0.0.1:8765/wrapped.html")
    pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=90000)
    print("loaded", round(time.time() - t, 1), "s;", pg.inner_text("#si-source"), "|", pg.inner_text("#pi-timing"))
    pg.screenshot(path=f"{out}/m3_si.png")
    pg.click("#view-pi"); time.sleep(0.8)
    pg.screenshot(path=f"{out}/m3_pi.png")
    print("PI:", pg.inner_text("#pi-origin"))
    pg.click("#pim-j"); time.sleep(0.5); pg.screenshot(path=f"{out}/m3_pi_j.png")
    pg.click("#view-si"); time.sleep(0.3)
    # field solver for one class
    pg.select_option("#fs-scope", label="class LVDS_SYNC")
    print("button:", pg.inner_text("#fs-start"))
    pg.click("#fs-start")
    pg.wait_for_function("document.getElementById('fs-start').textContent === 'computed'", timeout=240000)
    print("after solve:", pg.inner_text("#si-source"), "|", pg.inner_text("#fs-cache"))
    # switch board and back: results must come from cache
    pg.click("#sample-vna"); pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=90000)
    pg.click("#sample-ts"); pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=90000)
    pg.select_option("#fs-scope", label="class LVDS_SYNC")
    print("after switch:", pg.inner_text("#fs-start"), "|", pg.inner_text("#si-source"))
    # reload page: IndexedDB cache
    pg.reload(); pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=90000)
    pg.select_option("#fs-scope", label="class LVDS_SYNC")
    print("after reload:", pg.inner_text("#fs-start"), "|", pg.inner_text("#fs-cache"))
    print("errors:", errs[:6])
    b.close()
