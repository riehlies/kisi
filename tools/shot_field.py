"""Headless check of the field solver in the bundled viewer.
python3 shot_field.py <kisi.html> <outdir> [seconds]"""
import sys, time
from playwright.sync_api import sync_playwright
src, outdir = sys.argv[1], sys.argv[2]
secs = float(sys.argv[3]) if len(sys.argv) > 3 else 40
page_html = "<!doctype html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><style>[hidden]{display:none!important}body{margin:0}</style></head><body>" + open(src, encoding="utf-8").read() + "</body></html>"
open(outdir + "/wrapped.html", "w", encoding="utf-8").write(page_html)
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 1440, "height": 900})
    logs = []
    pg.on("console", lambda m: logs.append(f"{m.type}: {m.text}"))
    pg.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
    pg.goto("file://" + outdir + "/wrapped.html")
    pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=60000)
    t0 = time.time()
    while time.time() - t0 < secs:
        time.sleep(5)
        print(round(time.time() - t0), pg.inner_text("#fs").replace("\n", " | "))
    pg.screenshot(path=f"{outdir}/shot_field.png")
    print("console:", [l for l in logs if "ERR_TUNNEL" not in l][:10])
    b.close()
