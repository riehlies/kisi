// Prototype via field viewer: headless Chromium check of dist/kisi-via-field-proto.html (playwright).
//   node check_page.cjs <page.html> <out dir> [--quick]
// Loads the page (embedded example renders), moves the cut plane, plays the animation, saves and reloads a .kvf,
// cancels a run, runs a small case and the default single via and pair in the worker (run times and memory),
// checks a phone-width layout, fails on console errors; screenshots at deviceScaleFactor 2.
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const [page_, outDir] = process.argv.slice(2);
const quick = process.argv.includes("--quick");
fs.mkdirSync(outDir, { recursive: true });
const url = "file://" + path.resolve(page_);
const results = { checks: [], runs: {}, screenshots: [] };
let failed = 0;
const ok = (name, cond, info) => { results.checks.push({ name, ok: !!cond, info }); if (!cond) failed++; console.log(`${cond ? "ok  " : "FAIL"} ${name}${info != null ? ": " + JSON.stringify(info) : ""}`); };

(async () => {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1480, height: 1000 }, deviceScaleFactor: 2, acceptDownloads: true });
  const page = await ctx.newPage();
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
  const t0 = Date.now();
  await page.goto(url);
  await page.waitForFunction(() => window.KVF_STATE && window.KVF_STATE.ds, null, { timeout: 60000 });
  ok("embedded example renders", true, { load_s: (Date.now() - t0) / 1000 });
  const canvasHash = () => page.evaluate(() => { const c = document.getElementById("cv"); const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data; let h = 0; for (let i = 0; i < d.length; i += 97) h = (h * 31 + d[i]) >>> 0; return h; });
  const set = (id, v) => page.evaluate(([id, v]) => { const e = document.getElementById(id); e.value = v; e.dispatchEvent(new Event(e.tagName === "SELECT" ? "change" : "input", { bubbles: true })); }, [id, v]);
  const seg = (id, v) => page.click(`#${id} button[data-v="${v}"]`);
  const shot = async (name, sel) => {
    const p = path.join(outDir, name);
    if (sel) await page.locator(sel).screenshot({ path: p });
    else await page.screenshot({ path: p, fullPage: true });
    results.screenshots.push(name);
  };
  const viewerShot = (name) => shot(name, "section.viewer");

  // 1. single via |E| in the section y = 0 at 20 GHz
  await set("freq", "1");
  await page.mouse.move(700, 560);
  const h0 = await canvasHash();
  ok("plane drawn (non-uniform canvas)", h0 !== 0);
  await shot("1-single-via-E-section.png");

  // 2. cut plane slider and axis change
  await seg("axis", "2");
  const p1 = await page.textContent("#planePos");
  await set("planeSlider", "10");
  const p2 = await page.textContent("#planePos");
  ok("z slider moves the plane", p1 !== p2, [p1, p2]);
  await set("jump", String(0.8));
  ok("jump to the middle of the core", (await page.textContent("#planePos")).includes("0.8"), await page.textContent("#planePos"));
  const h1 = await canvasHash();
  ok("canvas changes with the plane", h1 !== h0);
  await seg("view", "H");
  await viewerShot("extra-single-via-H-top-view.png");

  // 3. loss density with the conductor estimate (section y = 0)
  await seg("axis", "1");
  await set("jump", "0");
  await seg("view", "loss");
  const tot = await page.textContent("#totals");
  ok("totals show the conductor estimate", /conductor loss/.test(tot) && /estimate/i.test(tot));
  await viewerShot("3-single-via-loss-with-conductor-estimate.png");
  await shot("3b-single-via-loss-full-page.png");

  // 4. animation: Ez instantaneous in the section y = 0
  await seg("view", "E");
  await seg("comp", "2");
  const ph0 = await page.evaluate(() => window.KVF_STATE.phase);
  await page.click("#play");
  await page.waitForTimeout(900);
  const ph1 = await page.evaluate(() => window.KVF_STATE.phase);
  const hA = await canvasHash();
  await page.waitForTimeout(400);
  const hB = await canvasHash();
  await page.click("#play");
  ok("animation advances the phase and redraws", ph1 !== ph0 && hA !== hB && (await page.evaluate(() => window.KVF_STATE.tmode)) === "inst", { ph0, ph1 });
  await set("phase", "90");
  await viewerShot("4-animation-frame-Ez-90deg.png");
  await seg("comp", "mag");
  await seg("tmode", "avg");

  // 5. save .kvf and load it back
  const [dl] = await Promise.all([page.waitForEvent("download"), page.click("#save")]);
  const kvf = path.join(outDir, "..", "saved-example.kvf");
  await dl.saveAs(kvf);
  const size = fs.statSync(kvf).size;
  await page.setInputFiles("#file", kvf);
  await page.waitForFunction(() => /saved-example|kvf/.test(document.getElementById("dsSub").textContent), null, { timeout: 30000 });
  ok("save .kvf and load it back", size > 1e5, { bytes: size });
  fs.unlinkSync(kvf);

  // 6. cancel a run
  await seg("kind", "pair");
  await set("mesh", "fine");
  await page.waitForFunction(() => !document.getElementById("run").disabled && /cells/.test(document.getElementById("estimate").textContent), null, { timeout: 30000 });
  await page.click("#run");
  await page.waitForTimeout(2500);
  await page.click("#cancel");
  await page.waitForFunction(() => /cancelled/i.test(document.getElementById("progress").textContent) && !document.getElementById("run").disabled, null, { timeout: 10000 });
  ok("cancel stops the run", true);

  // 7. a small run: single via, coarse mesh, 20 GHz only
  const runCase = async (name, kind, mesh, freqsOn, extra) => {
    await seg("kind", kind);
    await set("mesh", mesh);
    await page.evaluate((on) => { for (const b of document.querySelectorAll("#fchips .chip")) b.classList.toggle("on", on.includes(+b.dataset.f)); document.getElementById("fother").value = ""; document.getElementById("fother").dispatchEvent(new Event("input")); }, freqsOn);
    if (extra) await extra();
    await page.evaluate(() => window.KVF_APP.estimateNow()); // the estimate of exactly these settings
    await page.waitForFunction(() => !document.getElementById("run").disabled && /cells/.test(document.getElementById("estimate").textContent), null, { timeout: 30000 });
    const est = await page.textContent("#estimate");
    const warn = await page.evaluate(() => !document.getElementById("warn").classList.contains("hidden"));
    await page.evaluate(() => { window.KVF_LAST_RUN = null; });
    const t = Date.now();
    await page.click("#run");
    await page.waitForFunction(() => window.KVF_LAST_RUN, null, { timeout: 900000 });
    const r = await page.evaluate(() => window.KVF_LAST_RUN);
    r.page_wall_s = (Date.now() - t) / 1000;
    r.estimate = est.replace(/\s+/g, " ");
    r.warning_shown = warn;
    results.runs[name] = r;
    ok(`run ${name} completes in the worker`, !r.error, r.error ? r.error : { wall_s: r.wall_s, runs: r.runs });
    return r;
  };
  await runCase("single coarse, 20 GHz", "single", "coarse", [20]);
  ok("the viewer shows the new run", /run in this browser/.test(await page.textContent("#dsSub")));
  if (!quick) {
    await runCase("single standard, 10/20/30 GHz (default)", "single", "standard", [10, 20, 30]);
    await runCase("pair standard, 10/20/30 GHz, ports 1+2 (default)", "pair", "standard", [10, 20, 30]);
    // 8. the pair, odd mode, S with arrows: section through the P via and the top view in the core
    const odd = await page.evaluate(() => [...document.getElementById("exc").options].findIndex((o) => /odd/.test(o.text)));
    ok("odd mode offered for the pair", odd >= 0);
    await set("exc", String(odd));
    await set("freq", "1");
    await seg("view", "S");
    await seg("axis", "2");
    await set("jump", "0.8");
    await viewerShot("2-pair-odd-mode-S-top-view.png");
    await seg("axis", "1");
    await set("jump", "0.5");
    await viewerShot("2b-pair-odd-mode-S-section-P-via.png");
    await shot("2c-pair-odd-full-page.png");
    const even = await page.evaluate(() => [...document.getElementById("exc").options].findIndex((o) => /even/.test(o.text)));
    await set("exc", String(even));
    await seg("axis", "2");
    await set("jump", "0.8");
    await viewerShot("extra-pair-even-mode-S-top-view.png");
  }

  // 9. the large-run warning (no cap): pair, fine mesh, 5 mm feeds, ten frequencies
  await seg("kind", "pair");
  await set("mesh", "fine");
  await set("feed", "5");
  await page.evaluate(() => { for (const b of document.querySelectorAll("#fchips .chip")) b.classList.add("on"); const o = document.getElementById("fother"); o.value = "12, 15, 25, 35, 40, 45"; o.dispatchEvent(new Event("input")); });
  await page.evaluate(() => window.KVF_APP.estimateNow());
  const warnOn = await page.evaluate(() => !document.getElementById("warn").classList.contains("hidden"));
  const estBig = (await page.textContent("#estimate")).replace(/\s+/g, " ");
  ok("large-run warning shown above 300 MB per worker, Run stays enabled", warnOn && !(await page.isDisabled("#run")), estBig);
  results.large_estimate = estBig;
  await shot("extra-large-run-warning.png", "section.setup");

  // 10. phone width (dark theme): no horizontal scroll, the viewer still draws
  const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, colorScheme: "dark" });
  const pp = await phone.newPage();
  pp.on("pageerror", (e) => errors.push("phone pageerror: " + e.message));
  pp.on("console", (m) => { if (m.type() === "error") errors.push("phone: " + m.text()); });
  await pp.goto(url);
  await pp.waitForFunction(() => window.KVF_STATE && window.KVF_STATE.ds, null, { timeout: 60000 });
  const sw = await pp.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
  ok("phone width: no horizontal scroll", sw[0] <= sw[1] + 1, sw);
  await pp.locator("section.viewer").screenshot({ path: path.join(outDir, "extra-phone-viewer-dark.png") });
  results.screenshots.push("extra-phone-viewer-dark.png");
  await phone.close();

  ok("no console errors", errors.length === 0, errors);
  fs.writeFileSync(path.join(outDir, "..", "browser-check.json"), JSON.stringify(results, null, 1));
  await browser.close();
  console.log(`${results.checks.length - failed} of ${results.checks.length} browser checks passed`);
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
