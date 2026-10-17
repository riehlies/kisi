"""Headless check of the bundled viewer: python3 shot.py <kisi.html> <outdir>"""
import sys, time
from playwright.sync_api import sync_playwright
src, outdir = sys.argv[1], sys.argv[2]
page_html = "<!doctype html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1,viewport-fit=cover'><style>[hidden]{display:none!important}body{margin:0;font:14px system-ui}img{max-width:100%}</style></head><body>" + open(src, encoding="utf-8").read() + "</body></html>"
open(outdir + "/wrapped.html", "w", encoding="utf-8").write(page_html)
with sync_playwright() as p:
    b = p.chromium.launch()
    for scheme, w, h, tag in [("light", 1440, 900, "light"), ("dark", 1440, 900, "dark"), ("light", 400, 860, "phone")]:
        pg = b.new_page(viewport={"width": w, "height": h}, color_scheme=scheme)
        logs = []
        pg.on("console", lambda m: logs.append(f"{m.type}: {m.text}"))
        pg.on("pageerror", lambda e: logs.append(f"pageerror: {e}"))
        t = time.time()
        pg.goto("file://" + outdir + "/wrapped.html")
        pg.wait_for_function("document.getElementById('busy').hidden === true", timeout=60000)
        print(tag, "loaded in", round(time.time() - t, 2), "s")
        pg.wait_for_timeout(300)
        pg.screenshot(path=f"{outdir}/shot_{tag}.png", full_page=(tag == "phone"))
        if tag == "light":
            # click worst net, hover a trace
            pg.click(".net >> nth=0")
            pg.wait_for_timeout(400)
            pg.screenshot(path=f"{outdir}/shot_{tag}_net.png")
            box = pg.locator("#pcb").bounding_box()
            pg.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
            pg.wait_for_timeout(200)
            pg.click("#mode-abs"); pg.wait_for_timeout(300)
            pg.screenshot(path=f"{outdir}/shot_{tag}_abs.png")
        print(tag, "console:", logs[:10])
        pg.close()
    b.close()
