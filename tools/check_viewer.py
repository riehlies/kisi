#!/usr/bin/env python3
"""Headless check of the bundled viewer (Playwright; Chromium, or Firefox/WebKit with --browser).

Usage: check_viewer.py <kisi.html> [--board board.kicad_pcb [board.kicad_pro]] [--config board.kisi.json]
                       [--out DIR] [--quick] [--browser chromium|firefox|webkit] [--layouts light,dark,phone]

* light, dark and phone layout; all three views (impedance, channel, power)
* impedance estimate, net detail tabs (a differential pair: 4-port loss, skew, common mode), interfaces overview,
  assembly variant (the prompt when the board needs one, switching to another variant and back), channel check,
  lane eye, voltage drop with ground return, impedance seen from the ICs, losses map, temperature and power flow
  (desktop layouts; --quick: only load and the three views), the Prepare PCB page (what a step needs marked on its
  inputs for every step type, the source only after a value, the focus on the first marked input; tabs, laminate,
  a dismissal, saving one category; with the example boards: the loaded kisi.json in the forms, saving it
  unchanged, keys without a form kept, the draft across a page reload, Discard, saving one category, the
  views' edits into the draft, a stale dismissal; light run: the acceptance run on the bare ThunderScope –
  its kisi.json entered through the forms, saved, reloaded)
* --accept BOARD REF [--accept-dismiss GAP=REASON …]: the acceptance run on another bare board (light run)
* the channel check in parallel (workers with engines of their own) gives the same rows as on one thread
  (light run; the timings are printed as a note)
* uses the embedded example boards if the build has them, else --board; on a board without impedance
  targets the steps that need such nets (net detail, couplings, field solver run) are skipped
* --config: a <board>.kisi.json, loaded with --board (else dropped at the end); expects "check passed"
* power views: no "0.000"-style values (texts, load fields, tooltips); the board labels of the voltage drop,
  current density, losses and temperature maps at a constant screen size without overlaps; in the light run
  also for the other example boards of the build
* board text: every text on the board canvas (labels, A/B marks, PDN parts) is drawn on a screen-space
  transform (drawBoardLabels), none in board units
* exit code 1 on page errors or console errors (failed loads of external resources
  such as web fonts are only reported); screenshots go to --out (default: a temp dir)
"""
import argparse, json, os, re, sys, tempfile, time
from playwright.sync_api import sync_playwright

ap = argparse.ArgumentParser()
ap.add_argument("html")
ap.add_argument("--board", nargs="+")
ap.add_argument("--config")
ap.add_argument("--out")
ap.add_argument("--quick", action="store_true")
ap.add_argument("--browser", default="chromium", choices=["chromium", "firefox", "webkit"])
ap.add_argument("--layouts", default="light,dark,phone", help="comma-separated subset of light,dark,phone")
ap.add_argument("--prepare-only", action="store_true", help="only the Prepare PCB checks")
ap.add_argument("--accept", nargs=2, metavar=("BOARD", "REF"), help="Prepare acceptance run on a bare board with a reference kisi.json (light run)")
ap.add_argument("--accept-dismiss", nargs="*", default=[], metavar="GAP=REASON", help="name-based gaps to dismiss in the acceptance run")
a = ap.parse_args()
out = a.out or tempfile.mkdtemp(prefix="kisi-check-")
os.makedirs(out, exist_ok=True)
url = "file://" + os.path.abspath(a.html)
BUSY = "document.getElementById('busy').hidden === true"
problems, notes = [], []
# records every text drawn on the board canvas with the transform's scale (window.__boardText)
TEXT_SPY = """(() => {
  window.__boardText = [];
  for (const k of ["fillText", "strokeText"]) {
    const fn = CanvasRenderingContext2D.prototype[k];
    CanvasRenderingContext2D.prototype[k] = function (t, ...rest) {
      if (this.canvas && this.canvas.id === "pcb") { const m = this.getTransform(); window.__boardText.push([String(t), m.a, m.d]); }
      return fn.call(this, t, ...rest);
    };
  }
})()"""
# draws the view once and returns the board texts drawn in board units (not on the screen transform)
BOARD_TEXT = """(() => { window.__boardText = []; drawNow(); const d = window.devicePixelRatio || 1, all = window.__boardText;
  return { n: all.length, bad: all.filter((x) => Math.abs(x[1] - d) > 1e-9 || Math.abs(x[2] - d) > 1e-9).map((x) => x[0]).slice(0, 5) }; })()"""


def board_text(pg, tag, where, expect=True):
    r = pg.evaluate(BOARD_TEXT)
    if r["bad"]:
        problems.append(f"{tag}: board text drawn in board units in {where}: {r['bad']}")
    elif expect and not r["n"]:
        problems.append(f"{tag}: no labels on the board in {where}")


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
    # the clicks toggled settings the viewer keeps (e.g. the coupling refinement): back to a fresh start
    pg.evaluate("localStorage.clear()")
    pg.reload()
    pg.wait_for_function("document.getElementById('drop').classList.contains('start')", timeout=90000)


# a value with fixed decimals instead of an SI prefix: "0.000 A", "0.0012 W", "0.001 mW/mm²", "0.000"
FIXED_ZERO = re.compile(r"(?<![\d.])0\.0\d*\s?[pnuµmkMG]?(?:A|W|V|Ω|F|H|Hz)(?![a-zA-Z])|(?<![\d.])0\.000\d*|(?<![A-Za-z\d.])\d+(?:\.\d+)?[eE][+-]?\d+(?![A-Za-z\d])")


def power_units(pg, tag, step):
    """Power views: no "0.000"-style values in the power tree, the components, the rail list and
    editor, the drop / losses panels, the legends and the hover tooltips (voltage drop, current
    density, losses); losses labels on the board at a constant screen size, without overlaps."""
    texts = pg.evaluate("""async () => {
      const wait = () => new Promise((r) => setTimeout(r, 120)), out = [], txt = (id) => { const e = $(id); return e && !e.hidden ? e.innerText : ""; };
      const tipAt = (fn, bx, by) => { fn(S.view.x + bx * S.view.s, S.view.y + by * S.view.s); const t = $("tip"); const s = t.hidden ? "" : t.innerText; t.hidden = true; return s; };
      setPiMode("b"); await wait();
      out.push(["power tree", txt("pi-balance") + " " + [...document.querySelectorAll("#pi-balance input[data-load]")].map((i) => i.value).join(" ")]);
      for (const m of ["v", "j"]) {
        setPiMode(m); await wait();
        out.push([m + " panels", txt("pi-detail") + " " + txt("rails") + " " + txt("rail-edit") + " " + txt("legend") + " " + txt("pi-step") + " " + txt("pi-origin")]);
        const r = piCurrent();
        if (r) {  // the tooltip over a cell with a result
          const li = piLayerIdx(r), g = r.res.layers[li], c = r.res.cell;
          for (let k = 0; k < g.nx * g.ny; k++) if (!isNaN(r.v[g.offset + k])) { out.push([m + " tooltip", tipAt(showPiTip, g.x0 + (k % g.nx + 0.5) * c, g.y0 + (Math.floor(k / g.nx) + 0.5) * c)]); break; }
        }
      }
      setPiMode("l"); await wait();
      out.push(["losses panels", txt("pi-detail") + " " + txt("legend")]);
      if (S.loss && S.loss.map) {  // tooltips at the hottest cell and at a cool one
        const m = S.loss.map, im = lossImage(); let hot = -1, cool = -1;
        for (let k = 0; k < im.vals.length; k++) { if (im.vals[k] > 0 && (hot < 0 || im.vals[k] > im.vals[hot])) hot = k; if (im.vals[k] > 0 && (cool < 0 || im.vals[k] < im.vals[cool])) cool = k; }
        for (const k of [hot, cool]) if (k >= 0) out.push(["losses tooltip", tipAt(showLossTip, m.x0 + (k % m.nx + 0.5) * m.cell, m.y0 + (Math.floor(k / m.nx) + 0.5) * m.cell)]);
      }
      return out;
    }""")
    for where, t in texts:
        bad = sorted(set(m.group(0) for m in FIXED_ZERO.finditer(t)))
        if bad:
            problems.append(f"{tag}: fixed-decimal values in {where}: {bad[:6]}")
    if not any(w == "power tree" and t.strip() for w, t in texts):
        notes.append(f"{tag}: no power tree to check for units")
    # board labels of the maps: constant screen size at two zoom levels (the second 8× around the first
    # label), no overlaps, the caller's transform restored, no text in board units
    lab = pg.evaluate("""async () => {
      const wait = () => new Promise((r) => setTimeout(r, 150)), out = {};
      if (typeof drawBoardLabels !== "function") return { missing: true };
      ctx.setTransform(2, 0, 0, 2, 5, 7); drawBoardLabels([{ x: 0, y: 0, text: "x" }]); const t = ctx.getTransform();
      out.restored = t.a === 2 && t.e === 5 && t.f === 7;
      const keep = { ...S.view }, d = window.devicePixelRatio || 1;
      const views = [["v", "piLabels"], ["j", "piLabels"], ["l", "lossLabels"]].concat(thermOk() ? [["t", "thermLabels"]] : []);
      const one = (key) => { window.__boardText = []; drawNow(); const b = S[key] || [];
        return { n: b.length, h: b.map((x) => Math.round((x[3] - x[1]) * 100) / 100), first: b[0] || null,
          overlap: b.some((p, i) => b.some((q, j) => j > i && p[0] < q[2] && p[2] > q[0] && p[1] < q[3] && p[3] > q[1])),
          bad: (window.__boardText || []).filter((x) => Math.abs(x[1] - d) > 1e-9 || Math.abs(x[2] - d) > 1e-9).map((x) => x[0]).slice(0, 3) }; };
      for (const [m, key] of views) {
        if ($("pim-" + m).disabled) continue;
        setPiMode(m); await wait(); Object.assign(S.view, keep);
        const a = one(key);
        let b = null;
        if (a.first) {
          const cx = ((a.first[0] + a.first[2]) / 2 - S.view.x) / S.view.s, cy = ((a.first[1] + a.first[3]) / 2 - S.view.y) / S.view.s, s2 = S.view.s * 8;
          S.view.x = cv.width / d / 2 - cx * s2; S.view.y = cv.height / d / 2 - cy * s2; S.view.s = s2;
          b = one(key);
        }
        out[m] = { a, b };
      }
      return { out, keep };
    }""")
    if lab.get("missing"):
        problems.append(f"{tag}: no shared label helper drawBoardLabels")
    else:
        step("labels_zoom")
        pg.evaluate(f"(() => {{ Object.assign(S.view, {json.dumps(lab['keep'])}); draw(); }})()")
        o = lab["out"]
        if not o.pop("restored"):
            problems.append(f"{tag}: drawBoardLabels does not restore the caller's transform")
        names = {"v": "voltage drop", "j": "current density", "l": "losses", "t": "temperature"}
        for m, r in o.items():
            a, b = r["a"], r["b"] or r["a"]
            if not a["n"]:
                problems.append(f"{tag}: no labels on the {names[m]} map")
            if a["overlap"] or b["overlap"] or len(set(a["h"] + b["h"])) > 1 or a["bad"] or b["bad"]:
                problems.append(f"{tag}: {names[m]} labels: overlap {a['overlap']}/{b['overlap']}, heights {sorted(set(a['h'] + b['h']))}, text in board units {a['bad'] + b['bad']}")
    pg.evaluate("setPiMode('b')"); pg.wait_for_timeout(200)


def zip_names(path):
    import zipfile
    try:
        with zipfile.ZipFile(path) as z:
            if z.testzip() is not None:
                return ["(bad CRC)"]
            return z.namelist()
    except zipfile.BadZipFile as e:
        return [f"(not a zip: {e})"]


def prep_change(pg, sel, value):
    """Fill a Prepare page field and commit it (input, then change – as leaving the field does)."""
    loc = pg.locator(sel + " >> visible=true").first
    loc.fill(value)
    loc.dispatch_event("change")
    prep_tick(pg)


def prep_tick(pg):
    """Let the page draw the forms again (a change redraws right after the event)."""
    pg.evaluate("() => new Promise((r) => setTimeout(r, 0))")


def prep_source(pg, spec, src):
    """The source of one entry in its row: kind, document; url, page and note behind "…"."""
    cell = f'[data-a="src"][data-e="{spec}"]'
    pg.locator(f'select{cell}[data-k="kind"] >> visible=true').first.select_option(src.get("kind", ""))
    prep_tick(pg)
    if src.get("document"):
        prep_change(pg, f'input{cell}[data-k="document"]', src["document"])
    more = [k for k in ("url", "page", "note") if src.get(k)]
    if more:
        # the "…" of the row that is shown (a gap card may hold a hidden copy of the same row)
        pg.locator(f'select{cell}[data-k="kind"] >> visible=true').first.evaluate("(e) => { const d = e.closest('.prep-srccell').querySelector('details'); if (!d.open) d.querySelector('summary').click(); }")
        for k in more:
            prep_change(pg, f'input{cell}[data-k="{k}"]', src[k])


def prep_open(pg, sel):
    """Open a collapsed card (gap or kisi.json section)."""
    if pg.locator(sel).count() and pg.get_attribute(sel, "aria-expanded") != "true":
        pg.click(f"{sel} > header")


def prep_dismiss(pg, gid, reason):
    """Dismiss every dismissable item of a gap with a reason (the step's form, opened if needed)."""
    if not pg.locator(f'article[data-gap="{gid}"]').count():
        return False
    prep_open(pg, f'article[data-gap="{gid}"]')
    if pg.locator(f'[data-b="dismiss"][data-g="{gid}"]').count():
        pg.click(f'[data-b="dismiss"][data-g="{gid}"]')
    if not pg.locator(f'[data-b="dis-all"][data-g="{gid}"]').count():
        return False
    pg.click(f'[data-b="dis-all"][data-g="{gid}"]'); prep_tick(pg)
    prep_change(pg, f'[data-a="dis-reason"][data-g="{gid}"]', reason)
    return True


def prep_apply(pg, gid):
    """Apply a step and wait for the gap list."""
    pg.click(f'article[data-gap="{gid}"] [data-b="apply"]')
    prep_idle(pg)


def prep_tab(pg, t):
    pg.click(f'#prep-tabs [data-prep-tab="{t}"]'); pg.wait_for_timeout(100)


def prep_idle(pg):
    pg.wait_for_function("!S.prep.busy && !S.prep.inflight && !S.prep.queued", timeout=120000); pg.wait_for_timeout(100)


def prep_load(pg, pcb_js, cfg_text, clear=True):
    """Load a board (JS expression giving [pcb, pro, name]) with a kisi.json text or none, like a dropped file
    (no example button pressed); clear its draft."""
    if clear:
        pg.evaluate("() => { for (const k of Object.keys(localStorage)) if (k.startsWith('kisi.prep.v1.')) localStorage.removeItem(k); S.prep.mem = {}; }")
    pg.evaluate("async ([cfg]) => { const [a, c, n] = await (%s); for (const k of ['ts', 'vna']) document.getElementById('sample-' + k).setAttribute('aria-pressed', 'false'); S.sample = null; await loadFiles(a, c, n, null, cfg ? { name: n.replace(/\\.kicad_pcb$/i, '') + '.kisi.json', text: cfg } : null); }" % pcb_js, [cfg_text])
    pg.wait_for_function(f"S.scene && {BUSY} && S.prep.base", timeout=120000)
    prep_idle(pg)


SAMPLE_TS = "(async () => { const s = SAMPLES.ts; const [a, c] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]); return [a, c, s.file || s.name]; })()"


def prep_download(pg):
    pg.evaluate("window.showSaveFilePicker = undefined")
    with pg.expect_download() as dl:
        pg.click("#prep-save")
    text = open(dl.value.path(), encoding="utf-8").read()
    try:
        pg.wait_for_function(f"{BUSY} && S.prep.base", timeout=120000)
    except Exception:
        pg.screenshot(path=f"{out}/prepare_save_failed.png")
        raise RuntimeError(f"saved, but not loaded again: {pg.evaluate('[S.prep.note, !!S.prep.base, document.getElementById(`busy`).hidden, document.getElementById(`busy`).innerText, S.prep.saved]')} {pg.inner_text('#warnings')[:300]}")
    prep_idle(pg)
    return text


def prepare_page(pg, tag, step):
    """The Prepare PCB page on the loaded board: readiness dots, tabs, laminate entry, a dismissal, saving
    one category (the saved text must pass the configuration check) and back to the views."""
    try:
        pg.wait_for_function("S.prep.base", timeout=90000)
    except Exception:
        problems.append(f"{tag}: prepare: no gap list ({pg.evaluate('S.prep.note')})")
        return
    pg.evaluate("() => { lsSet(PREP_KEY(), null); S.prep.mem = {}; S.prep.d = null; }")
    if pg.evaluate("document.querySelectorAll('#view-si .dot, #view-ch .dot, #view-pi .dot').length"):
        problems.append(f"{tag}: prepare: readiness dots on the view buttons")
    # no preparation card in the views: the assembly variant card is the first card of the side panel
    if pg.evaluate("!!document.getElementById('prep-status') || document.querySelector('.side > section') !== $('variant-card')"):
        problems.append(f"{tag}: prepare: a preparation card in the side panel, or the assembly variant card is not first")
    pg.click("#prep-btn"); pg.wait_for_timeout(300)
    if not pg.evaluate("S.prep.open && !document.getElementById('prep').hidden && getComputedStyle(document.querySelector('.stage')).display === 'none'"):
        problems.append(f"{tag}: prepare: the page did not open")
        return
    for t in ["channel", "power", "impedance"]:
        prep_tab(pg, t)
        if not pg.evaluate(f"document.querySelector('#prep-tabs [data-prep-tab=\"{t}\"]').getAttribute('aria-selected') === 'true'"):
            problems.append(f"{tag}: prepare: tab {t} not selected")
    step("prepare")
    entered = False
    sel = 'article[data-gap="stackup-defaults"]'
    if pg.locator(sel).count():
        prep_open(pg, sel)
        pg.locator(f'{sel} [data-a="shared"][data-k="kind"]').select_option("datasheet")
        prep_change(pg, f'{sel} [data-a="shared"][data-k="document"]', "laminate datasheet (viewer check)")
        prep_change(pg, f'{sel} input[data-a="diel"][data-k="loss_tangent"]', "0.0095")
        entered = True
    if prep_dismiss(pg, "nets-without-target", "not an interface (viewer check)"):
        gid = "nets-without-target"
        try:
            pg.wait_for_function(f"!S.prep.busy && S.prep.cur && !S.prep.cur.gaps.some((g) => g.id === '{gid}')", timeout=120000)
        except Exception:
            problems.append(f"{tag}: prepare: the dismissed gap is still listed")
        entered = True
    if entered:
        prep_idle(pg)
        step("prepare_filled")
        pg.select_option("#prep-scope", "impedance")
        if pg.locator("#prep-save").is_disabled():
            problems.append(f"{tag}: prepare: save disabled ({pg.inner_text('#prep-msgs')[:200]})")
        else:
            text = prep_download(pg)
            ck = pg.evaluate("(t) => S.engine.configCheck(prepBoardFile(), t)", text)
            if not ck["ok"]:
                problems.append(f"{tag}: prepare: the saved kisi.json fails the check: {ck['errors'][:3]}")
            if not pg.evaluate("S.config && S.config.check.ok"):
                problems.append(f"{tag}: prepare: the saved kisi.json was not loaded cleanly")
    else:
        notes.append(f"{tag}: prepare: nothing to enter on this board (no laminate or dismissable gap)")
    if pg.evaluate("document.documentElement.scrollWidth > window.innerWidth + 1"):
        problems.append(f"{tag}: horizontal scroll on the Prepare page")
    pg.click("#view-si"); pg.wait_for_timeout(300)
    if pg.evaluate("S.prep.open || getComputedStyle(document.querySelector('.stage')).display === 'none'"):
        problems.append(f"{tag}: prepare: back to the impedance view failed")


def board_box(pg):
    """The board's bounding box on the canvas: centre offset (share of the canvas) and the share it fills."""
    return pg.evaluate("""() => { const r = cv.getBoundingClientRect(), b = S.scene.bbox, v = S.view;
      const x0 = b[0] * v.s + v.x, x1 = b[2] * v.s + v.x, y0 = b[1] * v.s + v.y, y1 = b[3] * v.s + v.y;
      return { dx: ((x0 + x1) / 2 - r.width / 2) / r.width, dy: ((y0 + y1) / 2 - r.height / 2) / r.height, fill: Math.max((x1 - x0) / r.width, (y1 - y0) / r.height), w: r.width }; }""")


def apply_visible(pg, gid):
    return pg.evaluate("(g) => { const b = document.querySelector(`article[data-gap='${g}'] [data-b=apply]`); return !!b && !b.disabled && b.offsetParent !== null; }", gid)


def prepare_checklist(pg, tag, step):
    """The checklist on the bare ThunderScope: a step's marker counts its open items and does not shift when
    another step is done; a class for the QSPI nets in "Default" (kisi.json net_classes.<name>.nets) closes
    the high step with Apply and a check mark, the next step opens; a partly filled step says what is missing;
    Apply stays while typing, after leaving the field and after a redraw; the capacitor group form; the tab ends
    ready; the board is where it was when coming back from the page (also after a reload while it was open)."""
    prep_load(pg, SAMPLE_TS, None)
    pg.click("#view-si"); pg.wait_for_timeout(300)
    box0 = board_box(pg)
    n0 = pg.evaluate("prepCount()")
    pg.click("#prep-btn"); pg.wait_for_timeout(300)
    prep_tab(pg, "impedance")
    step("checklist_open")
    marks = lambda: pg.evaluate("[...document.querySelectorAll('#prep-gaps .prep-step')].map((a) => [a.dataset.gap, a.querySelector('.mk').textContent])")
    m0 = dict(marks())
    want = pg.evaluate("prepCountItems(prepGapsNow().find((g) => g.id === 'nets-without-target'), 'impedance')")
    if m0.get("nets-without-target") != str(want):
        problems.append(f"{tag}: checklist: the marker of nets-without-target shows {m0.get('nets-without-target')}, not its {want} open items")
    sel = 'article[data-gap="nets-without-target"]'
    if pg.get_attribute(sel, "aria-expanded") != "true":
        problems.append(f"{tag}: checklist: the first open step is not open")
    # a class of their own for the QSPI nets, with a target, a profile and a source
    prep_change(pg, f'{sel} input[data-a="nc"][data-k="name"]', "QSPI")
    prep_change(pg, f'{sel} input[data-a="nc"][data-k="extra"]', "QSPI_*")
    pg.locator(f'{sel} [data-a="shared"][data-k="kind"]').select_option("assumption"); prep_tick(pg)
    prep_change(pg, f'{sel} [data-a="shared"][data-k="note"]', "single-ended 50 Ω, common practice for QSPI flash traces (viewer check)")
    # Apply stays while typing, after leaving the field, after a redraw
    pg.click(f'{sel} input[data-a="nc"][data-k="target_ohm"]'); pg.keyboard.type("50", delay=40)
    ok = [apply_visible(pg, "nets-without-target")]
    pg.keyboard.press("Tab"); ok.append(apply_visible(pg, "nets-without-target"))
    prep_idle(pg); ok.append(apply_visible(pg, "nets-without-target"))
    if not all(ok):
        problems.append(f"{tag}: checklist: Apply not there while typing / after leaving the field / after a redraw: {ok}")
    step("checklist_apply_kept")
    pg.locator(f'{sel} select[data-a="nc"][data-k="profile"]').select_option("se50"); prep_tick(pg)
    prep_apply(pg, "nets-without-target")
    if not pg.locator(f"{sel} .mk.done").count():
        problems.append(f"{tag}: checklist: the QSPI class did not close the step ({pg.inner_text(sel + ' .prep-res')!r}, {pg.evaluate('JSON.stringify(prepW().net_classes)')})")
    if pg.get_attribute('article[data-gap="stackup-defaults"]', "aria-expanded") != "true":
        problems.append(f"{tag}: checklist: the next step did not open after Apply")
    m1 = dict(marks())
    moved = {g: (m0[g], m1.get(g)) for g in m0 if g != "nets-without-target" and m0[g] != m1.get(g)}
    if moved:
        problems.append(f"{tag}: checklist: other steps' markers changed when one was done: {moved}")
    step("checklist_applied")
    # a step applied before it is complete says what is missing (Power: one load of many); the marker counts down
    prep_tab(pg, "power")
    sel = 'article[data-gap="ic-loads"]'
    prep_open(pg, sel)
    before = pg.inner_text(f"{sel} .mk")
    ic = pg.get_attribute(f'{sel} input[data-a="load"][data-rail]', "data-r")
    pg.click(f'{sel} input[data-a="load"][data-r="{ic}"][data-rail]'); pg.keyboard.type("5 mA", delay=30)
    if not apply_visible(pg, "ic-loads"):
        problems.append(f"{tag}: checklist: Apply gone while typing a load")
    prep_apply(pg, "ic-loads")
    res, after = pg.inner_text(f"{sel} .prep-res"), pg.inner_text(f"{sel} .mk")
    if "still missing" not in res or pg.get_attribute(sel, "aria-expanded") != "true" or not (after.isdigit() and before.isdigit() and int(after) == int(before) - 1):
        problems.append(f"{tag}: checklist: a partly filled step: {res!r}, marker {before} → {after}")
    step("checklist_missing")
    prep_change(pg, f'{sel} input[data-a="load"][data-r="{ic}"][data-rail]', "")
    # the capacitor group form writes PI_CAP on every part of the group
    sel = 'article[data-gap="cap-ratings"]'
    if pg.locator(sel).count():
        prep_open(pg, sel)
        grp = pg.get_attribute(f'{sel} input[data-a="capgrp"][data-k="rated"]', "data-g")
        prep_change(pg, f'{sel} input[data-a="capgrp"][data-g="{grp}"][data-k="rated"]', "6.3V")
        pg.locator(f'{sel} select[data-a="capgrp"][data-g="{grp}"]').first.select_option("X5R"); prep_tick(pg)
        caps = pg.evaluate(f"(S.prep.capGroups[{json.dumps(grp)}] || []).map((r) => (prepFld(r, 'PI_CAP') || {{}}).value)")
        if not caps or any(c != "rated=6.3V X5R" for c in caps):
            problems.append(f"{tag}: checklist: the capacitor group form wrote {caps[:3]}")
        pg.evaluate(f"(S.prep.capGroups[{json.dumps(grp)}] || []).forEach((r) => prepSetFld(r, 'PI_CAP', ''))"); prep_idle(pg)
    # the laminate step, then the tab is ready
    prep_tab(pg, "impedance")
    sel = 'article[data-gap="stackup-defaults"]'
    prep_open(pg, sel)
    pg.locator(f'{sel} [data-a="shared"][data-k="kind"]').select_option("datasheet"); prep_tick(pg)
    prep_change(pg, f'{sel} [data-a="shared"][data-k="document"]', "Isola FR408HR data sheet (viewer check)")
    for i in pg.evaluate("prepItems(prepSteps('impedance').find((g) => g.id === 'stackup-defaults'), 'impedance').map((it) => it.index)"):
        prep_change(pg, f'{sel} input[data-a="diel"][data-i="{i}"][data-k="epsilon_r"]', "3.68")
    prep_apply(pg, "stackup-defaults")
    if not pg.locator(f"{sel} .mk.done").count():
        problems.append(f"{tag}: checklist: the laminate step has no check mark ({pg.inner_text(sel + ' .prep-res')!r})")
    if not pg.locator(".prep-finish [data-b=goview]").count():
        problems.append(f"{tag}: checklist: the Impedance tab does not end with 'Ready for the impedance check'")
    step("checklist_ready")
    n1 = pg.evaluate("prepCount()")
    if not (n1 < n0) or pg.inner_text("#prep-count") != str(n1):
        problems.append(f"{tag}: checklist: the badge did not follow ({n0} → {n1})")
    # back to the views: the board where it was, also after loading the board again while the page was open
    pg.evaluate("async () => { const l = S.lastFiles; await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, l.cfg); }")
    pg.wait_for_function(f"S.scene && {BUSY} && S.prep.base", timeout=120000); prep_idle(pg)
    pg.click(".prep-finish [data-b=goview]") if pg.locator(".prep-finish [data-b=goview]").count() else pg.click("#view-si")
    for v in ["si", "ch", "pi"]:
        pg.click(f"#view-{v}"); pg.wait_for_timeout(400)
        if v == "pi":
            continue  # the power view opens on the power tree, not the board
        b = board_box(pg)
        if abs(b["dx"]) > 0.08 or abs(b["dy"]) > 0.1 or not (0.7 * box0["fill"] <= b["fill"] <= 1.3 * box0["fill"]):
            problems.append(f"{tag}: checklist: the board is off after Prepare in view {v}: {b} (before {box0})")
    pg.click("#view-si"); pg.wait_for_timeout(400)
    step("checklist_board_back")
    pg.evaluate("() => { prepDiscard(); }"); prep_idle(pg)


def prepare_handover(pg, tag, step):
    """Single items handed to the agent appear in the agent card's prompt (per category and for all categories),
    an item taken back disappears, the marks survive a reload; nothing handed over: the default prompt."""
    prep_load(pg, SAMPLE_TS, None)
    if not pg.evaluate("S.prep.open"):
        pg.click("#prep-btn"); pg.wait_for_timeout(300)
    prep_tab(pg, "power")
    sel = 'article[data-gap="ic-loads"]'
    prompt = lambda: pg.inner_text("#prep-prompt")
    if "research this category's open gaps" not in prompt():
        problems.append(f"{tag}: handover: without anything handed over the prompt is not the default one")
    prep_open(pg, sel)
    pg.locator(f"{sel} details.prep-hand > summary").click(); pg.wait_for_timeout(100)
    boxes = pg.locator(f'{sel} input[data-a="agentitem"]')
    keys = [boxes.nth(i).get_attribute("data-k") for i in range(2)]
    for k in keys:
        pg.check(f'{sel} input[data-a="agentitem"][data-k="{k}"]'); prep_idle(pg)
    p1 = prompt()
    if not all(k in p1 for k in keys) or "ic-loads" not in p1 or "--for power" not in p1 or "PI_LOAD" not in p1:
        problems.append(f"{tag}: handover: the prompt lacks the two items handed over {keys}: {p1[:300]}")
    step("handover_prompt")
    pg.uncheck(f'{sel} input[data-a="agentitem"][data-k="{keys[1]}"]'); prep_idle(pg)
    p2 = prompt()
    if keys[0] not in p2 or re.search(r"\b%s\b" % re.escape(keys[1]), p2):
        problems.append(f"{tag}: handover: after taking {keys[1]} back the prompt is {p2[:300]}")
    # the marks are kept in the draft: loading the board again keeps them
    pg.evaluate("async () => { const l = S.lastFiles; await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, l.cfg); }")
    pg.wait_for_function(f"S.scene && {BUSY} && S.prep.base", timeout=120000); prep_idle(pg)
    if not pg.evaluate("S.prep.open"):
        pg.click("#prep-btn"); pg.wait_for_timeout(300)
    prep_tab(pg, "power")
    if keys[0] not in prompt():
        problems.append(f"{tag}: handover: the item handed over is gone after loading the board again")
    pg.select_option("#prep-agent-scope", "all"); pg.wait_for_timeout(100)
    pa = prompt()
    if "Power (--for power):" not in pa or keys[0] not in pa or "`kisi gaps`" not in pa:
        problems.append(f"{tag}: handover: the all-categories prompt is {pa[:300]}")
    pg.select_option("#prep-agent-scope", "tab")
    pg.evaluate("() => { const d = prepD(); d.agent = {}; d.agentItems = {}; prepPersist(); prepDiscard(); }"); prep_idle(pg)
    pg.click("#view-si"); pg.wait_for_timeout(300)


def prepare_stale_draft(pg, tag, step):
    """A stored draft belongs to the kisi.json it was made on: the same board with a changed kisi.json shows the
    notice and the loaded file (no stale gaps), Restore lays the draft over it, Discard removes it; for a bundled
    example whose kisi.json changed the draft is dropped without a notice."""
    text0 = pg.evaluate("SAMPLES.ts.config.text")
    mark = "draft over the old file (viewer check)"
    def make_draft(load):
        load()
        if not pg.evaluate("S.prep.open"):
            pg.click("#prep-btn"); pg.wait_for_timeout(300)
        pg.evaluate("(m) => { const w = prepW(); w.notes = (w.notes || []).concat([m]); prepEdited(false); }", mark); prep_idle(pg)
        return pg.evaluate("(k) => !!localStorage.getItem(k)", "kisi.prep.v1." + pg.evaluate("S.name"))
    text1 = json.dumps(dict(json.loads(text0), notes=json.loads(text0).get("notes", []) + ["edited elsewhere (viewer check)"]), indent=1)
    if not make_draft(lambda: prep_load(pg, SAMPLE_TS, text0)):
        problems.append(f"{tag}: stale draft: the draft was not stored")
    prep_load(pg, SAMPLE_TS, text1, clear=False)
    if not pg.evaluate("S.prep.open"):
        pg.click("#prep-btn"); pg.wait_for_timeout(300)
    st = pg.evaluate("() => ({ shown: !$('prep-stale').hidden, text: $('prep-stale').textContent, dirty: prepDirty(), count: prepCount() })")
    if not st["shown"] or "earlier version of this kisi.json" not in st["text"] or st["dirty"] or st["count"] != 0:
        problems.append(f"{tag}: stale draft: after the kisi.json changed: {st}")
    step("stale_draft_notice")
    pg.click('#prep-stale [data-b="stale-restore"]'); prep_idle(pg)
    if not pg.evaluate("(m) => prepDirty() && (prepW().notes || []).includes(m) && $('prep-stale').hidden && prepD().foreign", mark):
        problems.append(f"{tag}: stale draft: Restore did not lay the draft over the loaded file")
    pg.evaluate("() => prepDiscard()"); prep_idle(pg)
    make_draft(lambda: prep_load(pg, SAMPLE_TS, text0))
    prep_load(pg, SAMPLE_TS, text1, clear=False)
    pg.click('#prep-stale [data-b="stale-discard"]'); prep_idle(pg)
    if pg.evaluate("(k) => !!localStorage.getItem(k) || !$('prep-stale').hidden || prepDirty()", "kisi.prep.v1." + pg.evaluate("S.name")):
        problems.append(f"{tag}: stale draft: Discard left the old draft")
    # a bundled example whose kisi.json changed (a new build): the draft goes without a notice
    def load_example():
        pg.evaluate("async () => { await loadSample('ts'); }")
        pg.wait_for_function(f"S.scene && {BUSY} && S.prep.base", timeout=120000); prep_idle(pg)
    make_draft(load_example)
    pg.evaluate("(t) => { SAMPLES.ts.config.text = t; }", text1)
    load_example()
    if pg.evaluate("(k) => !!localStorage.getItem(k) || !$('prep-stale').hidden || prepDirty() || prepCount() !== 0", "kisi.prep.v1." + pg.evaluate("S.name")):
        problems.append(f"{tag}: stale draft: the example kept a draft made on its earlier kisi.json")
    pg.evaluate("(t) => { SAMPLES.ts.config.text = t; }", text0)
    pg.click("#view-si"); pg.wait_for_timeout(300)


def prepare_examples_clean(pg, tag, step):
    """The example boards with their kisi.json: every category ready (or nothing to check), no required step
    open, the check without errors or warnings – a finished checklist with the values in the forms."""
    for key in pg.evaluate("Object.keys(SAMPLES)"):
        js = "(async () => { const s = SAMPLES[%s]; const [a, c] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]); return [a, c, s.file || s.name]; })()" % json.dumps(key)
        prep_load(pg, js, pg.evaluate(f"SAMPLES[{json.dumps(key)}].config && SAMPLES[{json.dumps(key)}].config.text"))
        pg.click("#prep-btn"); pg.wait_for_timeout(300)
        ck = pg.evaluate("S.prep.check")
        if not ck or ck["errors"] or ck["warnings"]:
            problems.append(f"{tag}: example {key}: the kisi.json check reports {ck and (ck['errors'] + ck['warnings'])[:3]}")
        for t in ["impedance", "channel", "power"]:
            prep_tab(pg, t)
            v = pg.evaluate(f"prepViewsNow()['{t}']")
            left = pg.evaluate(f"prepSteps('{t}').filter((g) => prepRequired(g) && prepStepState(g, '{t}').k === 'open').map((g) => g.id)")
            if v["state"] not in ("ready", "nothing to check") or left:
                problems.append(f"{tag}: example {key}: {t} is {v['state']}, open steps {left}")
            step(f"example_{key}_{t}")
        if pg.inner_text("#prep-count") != "0":
            problems.append(f"{tag}: example {key}: the badge shows {pg.inner_text('#prep-count')} open points")
        pg.click("#view-si"); pg.wait_for_timeout(200)


# per step type: the gap, the value typed into its first marked input (None: a select or radio, chosen instead), and
# whether the entry then gets a source cell of its own (caps and the regulator chain share one source)
MARK_STEPS = {"nets-without-target": "50", "stackup-defaults": "0.012", "classes-without-standard": None, "assembly-variant": None,
              "power-tree": None, "ic-loads": "120", "cap-ratings": "6.3V", "bead-ratings": "220", "thermal-data": "20"}
MARK_SOURCE = {"nets-without-target", "stackup-defaults", "classes-without-standard", "ic-loads", "bead-ratings", "thermal-data"}


def prepare_marks(pg, tag, step):
    """What a step needs is marked where it is entered: on the bare example boards (and ThunderScope with its kisi.json
    for the thermal data) every open step with a form marks its empty value inputs (prep-need), no source cell is
    marked before its entry has a value, opening the step focuses the first marked input; a value typed there drops
    the mark and marks the entry's source instead."""
    seen = set()
    for key, cfg in (("ts", False), ("vna", False), ("ts", True)):
        if not pg.evaluate(f"!!SAMPLES[{json.dumps(key)}]"):
            continue
        js = "(async () => { const s = SAMPLES[%s]; const [a, c] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]); return [a, c, s.file || s.name]; })()" % json.dumps(key)
        prep_load(pg, js, pg.evaluate(f"SAMPLES[{json.dumps(key)}].config.text") if cfg else None)
        if not pg.evaluate("S.prep.open"):
            pg.click("#prep-btn")
        for t in ["impedance", "channel", "power"]:
            prep_tab(pg, t)
            for g in pg.evaluate("[...document.querySelectorAll('#prep-gaps .prep-step.k-open[data-gap]')].map((e) => e.dataset.gap).filter((g) => g !== 'save')"):
                if g in seen or g not in MARK_STEPS:
                    continue
                seen.add(g)
                pg.evaluate(f"S.prep.cards[S.name + {json.dumps(g)}] = true; S.prep.scrollTo = {json.dumps(g)}; prepRender(true)")
                pg.wait_for_timeout(250)
                r = pg.evaluate("""(g) => { const el = document.getElementById('prep-gap-' + g), f = document.activeElement;
                  return { need: el.querySelectorAll('.body .prep-need').length, miss: el.querySelectorAll('.body .prep-srccell.miss').length,
                           focus: !!f && el.contains(f) && (f.classList.contains('prep-need') || !!f.closest('.prep-need')) }; }""", g)
                where = f"{tag}: prepare marks ({key}{' with kisi.json' if cfg else ''}, {t}) {g}"
                if not r["need"]:
                    problems.append(f"{where}: no input marked as needed")
                    continue
                if r["miss"]:
                    problems.append(f"{where}: {r['miss']} source cell(s) marked before any value")
                if not r["focus"]:
                    problems.append(f"{where}: the focus is not on a marked input")
                step(f"marks_{g}")
                # enter the first value: the mark goes, the entry's source is marked (until a source is given)
                sel = pg.evaluate("""(g) => { const el = document.querySelector('#prep-gap-' + g + ' .body .prep-need');
                  const x = el.matches('input, select') ? el : el.querySelector('input'); return prepSel(x); }""", g)
                full = f"#prep-gap-{g} {sel}"
                loc = pg.locator(full).first
                tagname = loc.evaluate("(e) => e.tagName + ':' + e.type")
                if tagname == "SELECT:select-one":
                    loc.select_option(index=1)
                elif tagname.endswith(":radio"):
                    pg.locator(f"#prep-gap-{g} [role=radiogroup] input").first.check()
                elif MARK_STEPS[g] is not None:
                    loc.fill(MARK_STEPS[g]); loc.press("Tab")
                else:
                    continue
                prep_idle(pg)
                after = pg.evaluate("""([g, sel]) => { const x = document.querySelector('#prep-gap-' + g + ' ' + sel);
                  const box = x && (x.closest('tr') || x.closest('.prep-nc') || x.closest('.prep-conn'));
                  return { gone: !x || !x.classList.contains('prep-need'), src: !!box && !!box.querySelector('.prep-srccell.miss'), step: !!document.getElementById('prep-gap-' + g) }; }""", [g, sel])
                if not after["gone"]:
                    problems.append(f"{where}: the input stays marked after a value was entered")
                if g in MARK_SOURCE and after["step"] and not after["src"]:
                    problems.append(f"{where}: the source of the entry is not marked after its value")
    want = {"nets-without-target", "stackup-defaults", "classes-without-standard", "assembly-variant", "power-tree", "ic-loads", "cap-ratings"}
    if pg.evaluate("!!SAMPLES.vna"):
        want.add("bead-ratings")
    if pg.evaluate("!!(SAMPLES.ts && SAMPLES.ts.config)"):
        want.add("thermal-data")
    if want - seen:
        problems.append(f"{tag}: prepare marks: step types not checked: {sorted(want - seen)}")
    pg.evaluate("() => { for (const k of Object.keys(localStorage)) if (k.startsWith('kisi.prep.v1.')) localStorage.removeItem(k); S.prep.mem = {}; }")
    if pg.evaluate("S.prep.open"):
        pg.click("#view-si"); pg.wait_for_timeout(200)


def prepare_roundtrip(pg, tag, step):
    """ThunderScope with its kisi.json (plus keys the page has no form for): the forms show the loaded values,
    saving unchanged gives the same file, an edit keeps the other keys, the draft survives a page reload and
    can be discarded, saving one category keeps the rest in the draft, the views' edits move into the draft,
    and a stale dismissal shows next to its gap with remove."""
    ref = json.loads(pg.evaluate("SAMPLES.ts.config.text"))
    ref["x_custom"] = {"keep": True}
    ref["connector_ends"] = {"P1": {"model": "matched", "rest_loss_db": 3, "source": {"kind": "assumption", "note": "PCIe slot", "url": "https://example.com/slot", "page": "4"}}}
    ref.setdefault("gaps", {})["dismissed"] = [{"gap": "rails-without-voltage", "items": ["+1V0"], "reason": "stale on purpose (viewer check)"}]
    text0 = json.dumps(ref, indent=1)
    prep_load(pg, SAMPLE_TS, text0)
    pg.click("#prep-btn"); pg.wait_for_timeout(300)
    if pg.evaluate("prepDirty()") or not pg.evaluate("(t) => prepSame(JSON.parse(prepText('all')), JSON.parse(t))", text0):
        problems.append(f"{tag}: prepare: the loaded kisi.json does not come back unchanged")
    saved = prep_download(pg)
    if json.loads(saved) != ref:
        problems.append(f"{tag}: prepare: saving the loaded kisi.json unchanged changed it")
    pg.click("#prep-btn") if not pg.evaluate("S.prep.open") else None
    # the loaded values in the forms
    prep_tab(pg, "channel")
    if pg.evaluate("(document.querySelector('[data-a=\"conn\"][data-r=\"P1\"][data-k=\"model\"]') || {}).value") != "matched":
        problems.append(f"{tag}: prepare: the loaded connector end P1 is not in the Channel form")
    if pg.evaluate("(document.querySelector('input[data-a=\"variant\"][value=\"TS-PCIe\"]') || {}).checked") is not True:
        problems.append(f"{tag}: prepare: the loaded variant is not chosen in the form")
    prep_tab(pg, "power")
    n_reg = pg.evaluate("document.querySelectorAll('input[data-a=\"fld\"][data-n=\"PI_REG\"]').length")
    if n_reg != sum(1 for f in ref["fields"] if f["name"] == "PI_REG"):
        problems.append(f"{tag}: prepare: {n_reg} PI_REG rows in the form, the file has {sum(1 for f in ref['fields'] if f['name'] == 'PI_REG')}")
    if not pg.locator('.prep-stale [data-b="dis-drop"][data-item="+1V0"]').count():
        problems.append(f"{tag}: prepare: the stale dismissal is not shown with remove")
    else:
        pg.click('.prep-stale [data-b="dis-drop"][data-item="+1V0"]'); prep_idle(pg)
        if pg.evaluate("!!prepDis('rails-without-voltage', false)"):
            problems.append(f"{tag}: prepare: removing the stale dismissal left it in the draft")
    step("prepare_loaded")
    # an edit: the other keys stay
    load = next(f for f in ref["fields"] if f["name"] == "PI_LOAD")
    prep_open(pg, 'article[data-sec="f-PI_LOAD"]')
    prep_change(pg, f'input[data-a="fld"][data-r="{load["ref"]}"][data-n="PI_LOAD"]', load["value"] + ", +3V3=1mA" if "+3V3" not in load["value"] else load["value"].replace("+3V3=", "+3V3=9"))
    prep_idle(pg)
    w = json.loads(pg.evaluate("prepText('all')"))
    if w.get("x_custom") != {"keep": True} or w["connector_ends"]["P1"]["source"].get("page") != "4" or w.get("notes") != ref["notes"]:
        problems.append(f"{tag}: prepare: an edit lost keys the page has no form for")
    if pg.evaluate("prepDiff().n") != 2:  # the PI_LOAD and the removed dismissal
        problems.append(f"{tag}: prepare: the draft differs in {pg.evaluate('prepDiff().n')} entries, expected 2")
    # the draft survives a reload of the page
    pg.reload(); pg.wait_for_function("document.getElementById('busy').hidden === true || document.getElementById('drop').classList.contains('start')", timeout=90000)
    prep_load(pg, SAMPLE_TS, text0, clear=False)
    if not pg.evaluate("prepDirty() && prepDiff().n === 2 && !prepD().foreign"):
        problems.append(f"{tag}: prepare: the draft did not survive a page reload")
    pg.click("#prep-btn"); pg.wait_for_timeout(200)
    if pg.locator("#prep-discard").is_hidden():
        problems.append(f"{tag}: prepare: no Discard for a changed draft")
    pg.click("#prep-discard"); prep_idle(pg)
    if pg.evaluate("prepDirty() || lsGet(PREP_KEY()) !== null"):
        problems.append(f"{tag}: prepare: Discard did not reset the draft")
    # the connector end form
    prep_tab(pg, "channel")
    prep_open(pg, 'article[data-sec="conn"]')
    prep_change(pg, 'input[data-a="conn"][data-r="P1"][data-k="rest_loss_db"]', "4.5")
    if pg.evaluate("prepConn('P1', false).rest_loss_db") != 4.5:
        problems.append(f"{tag}: prepare: the connector end form did not change P1")
    prep_idle(pg); step("prepare_forms")
    pg.evaluate("prepDiscard()"); prep_idle(pg)
    # save one category: the laminate (impedance) goes, the load (power) stays in the draft
    prep_tab(pg, "impedance")
    prep_open(pg, 'article[data-sec="stackup"]')
    pg.locator('article[data-sec="stackup"] [data-a="shared"][data-k="kind"]').select_option("datasheet")
    prep_change(pg, 'article[data-sec="stackup"] [data-a="shared"][data-k="document"]', "laminate datasheet (viewer check)")
    prep_change(pg, 'article[data-sec="stackup"] input[data-a="diel"][data-i="0"][data-k="loss_tangent"]', "0.0091")
    prep_tab(pg, "power")
    prep_open(pg, 'article[data-sec="f-PI_LOAD"]')
    prep_change(pg, f'input[data-a="fld"][data-r="{load["ref"]}"][data-n="PI_LOAD"]', load["value"].split(",")[0].split("=")[0] + "=77mA")
    prep_tab(pg, "impedance"); prep_idle(pg)
    pg.select_option("#prep-scope", "impedance")
    if pg.locator("#prep-save").is_disabled():
        problems.append(f"{tag}: prepare: saving only Impedance is disabled ({pg.inner_text('#prep-msgs')[:200]})")
    else:
        part = json.loads(prep_download(pg))
        f0 = next(f for f in part["fields"] if f["ref"] == load["ref"] and f["name"] == "PI_LOAD")
        if part["stackup"]["dielectrics"][0].get("loss_tangent") != 0.0091 or f0["value"] != load["value"]:
            problems.append(f"{tag}: prepare: saving only Impedance did not take just the impedance entries")
        if not pg.evaluate(f"prepDirty() && prepFld({json.dumps(load['ref'])}, 'PI_LOAD').value.endsWith('=77mA')"):
            problems.append(f"{tag}: prepare: after saving Impedance the power change left the draft")
    pg.evaluate("prepDiscard()"); prep_idle(pg)
    # an edit in the Impedance view (stackup) moves into the draft with a source
    pg.evaluate("() => { lsSet(stackKey(), { dielectrics: { 1: { loss_tangent: 0.0088 } } }); S.stackOv = null; }")
    pg.click("#prep-btn"); pg.wait_for_timeout(100); pg.click("#prep-btn"); pg.wait_for_timeout(200)
    prep_tab(pg, "impedance")
    if not pg.locator('.prep-take [data-b="take"]').count():
        problems.append(f"{tag}: prepare: the stackup edit of the Impedance view is not offered")
    else:
        pg.locator('.prep-take select[data-a="take"]').select_option("measurement")
        prep_change(pg, '.prep-take input[data-a="take"][data-k="document"]', "fabricator's stackup report (viewer check)")
        pg.click('.prep-take [data-b="take"]'); prep_idle(pg)
        e = pg.evaluate("prepDiel(1, false)")
        if not e or e.get("loss_tangent") != 0.0088 or (e.get("source") or {}).get("kind") != "measurement":
            problems.append(f"{tag}: prepare: taking the views' edits did not fill the draft ({e})")
        step("prepare_take")
    pg.evaluate("() => { lsSet(stackKey(), null); S.stackOv = null; prepDiscard(); }")
    pg.click("#view-si"); pg.wait_for_timeout(200)


def prepare_acceptance(pg, tag, step, board_js, ref_text, dismiss, label):
    """The acceptance run: a bare board, every value of a reference kisi.json entered through the forms
    (with its source), the name-based gaps that do not apply dismissed, what needs research handed to the
    agent; then each applicable category must be ready or have only agent items, the saved file must pass
    the check and hold the reference's entries, and after a page reload board + saved file must give the
    same state."""
    ref = json.loads(ref_text)
    t0 = time.time()
    prep_load(pg, board_js, None)
    pg.click("#prep-btn"); pg.wait_for_timeout(300)
    # the variants first (the lanes and the channel gaps depend on them)
    if ref.get("variants"):
        prep_tab(pg, "channel")
        card = 'article[data-gap="assembly-variant"]' if pg.locator('article[data-gap="assembly-variant"]').count() else 'article[data-sec="variants"]'
        prep_open(pg, card)
        for name, v in ref["variants"].items():
            pg.fill("#prep-newvar", name); pg.click('[data-b="add-var"]')
            for k in ("description", "not_fitted_sheets", "fitted_sheets", "not_fitted", "fitted", "bom", "bom_exceptions"):
                if k in v:
                    prep_change(pg, f'input[data-a="vdef"][data-v="{name}"][data-k="{k}"]', v[k] if isinstance(v[k], str) else ", ".join(v[k]))
            prep_source(pg, f"var:{name}", v.get("source", {}))
        if ref.get("variant"):
            pg.locator(f'input[data-a="variant"][value="{ref["variant"]}"]').first.check()
        prep_idle(pg)
    # the laminate, per dielectric with its own source (Impedance: In <board>.kisi.json)
    if (ref.get("stackup") or {}).get("dielectrics"):
        prep_tab(pg, "impedance"); prep_open(pg, 'article[data-sec="stackup"]')
        for d in ref["stackup"]["dielectrics"]:
            for k in ("material", "epsilon_r", "loss_tangent", "thickness"):
                if k in d:
                    prep_change(pg, f'article[data-sec="stackup"] input[data-a="diel"][data-i="{d["index"]}"][data-k="{k}"]', str(d[k]))
            prep_source(pg, f"diel:{d['index']}", d.get("source", {}))
    # net classes: those with nets through the step for nets in "Default", the others' profiles in the table
    for c, e in (ref.get("net_classes") or {}).items():
        if e.get("nets"):
            prep_tab(pg, "impedance"); sel = 'article[data-gap="nets-without-target"]'; prep_open(pg, sel)
            prep_change(pg, f'{sel} input[data-a="nc"][data-k="name"]', c)
            prep_change(pg, f'{sel} input[data-a="nc"][data-k="extra"]', ", ".join(e["nets"]))
            if "target_ohm" in e:
                prep_change(pg, f'{sel} input[data-a="nc"][data-k="target_ohm"]', str(e["target_ohm"]))
            box = pg.locator(f'{sel} input[data-a="nc"][data-k="diff"]').first
            if box.is_checked() != bool(e.get("diff")):
                box.click(); prep_tick(pg)
            if e.get("profile"):
                pg.locator(f'{sel} select[data-a="nc"][data-k="profile"]').first.select_option(e["profile"]); prep_tick(pg)
            prep_source(pg, f"cls:{c}", e.get("source", {}))
            prep_apply(pg, "nets-without-target")
        else:
            prep_tab(pg, "channel"); prep_open(pg, 'article[data-sec="profiles"]')
            if e.get("profile"):
                pg.locator(f'article[data-sec="profiles"] select[data-a="cls"][data-c="{c}"][data-k="profile"]').first.select_option(e["profile"]); prep_tick(pg)
            prep_source(pg, f"cls:{c}", e.get("source", {}))
    prep_tab(pg, "power")
    for f in ref.get("fields", []):
        if f["name"].upper() == "PI_CAP":
            continue  # through the capacitor groups below
        name = f["name"].upper()
        sec = f'article[data-sec="f-{name}"]'
        if pg.locator(sec).count():
            prep_open(pg, sec)
        else:
            prep_open(pg, 'article[data-gap="power-tree"]')
        pg.locator(f'input[data-a="new"][data-n="{name}"][data-k="ref"] >> visible=true').first.fill(f["ref"])
        prep_change(pg, f'input[data-a="new"][data-n="{name}"][data-k="value"]', f["value"])
        prep_source(pg, f"fld:{f['ref']}|{name}", f.get("source", {}))
    # capacitor data: the suggestion from the parts' own fields, per group
    if any(f["name"].upper() == "PI_CAP" for f in ref.get("fields", [])):
        prep_idle(pg); sel = 'article[data-gap="cap-ratings"]'
        if pg.locator(sel).count():
            prep_open(pg, sel)
            for i in range(pg.locator(f'{sel} [data-b="cap-sugg"]').count()):
                pg.locator(f'{sel} [data-b="cap-sugg"]').nth(i).click(); prep_tick(pg)
    if ref.get("notes"):
        pg.evaluate("document.querySelector('.prep-notes').open = true")
        pg.fill("#prep-notes", "\n".join(ref["notes"]))
    prep_idle(pg)
    for gid, reason in dismiss.items():
        for t in ["impedance", "channel", "power"]:
            prep_tab(pg, t)
            if prep_dismiss(pg, gid, reason):
                prep_apply(pg, gid)
                break
    prep_idle(pg)
    # what is left of high and medium priority goes to the agent (research: laminate, profiles, loads)
    for t in ["impedance", "channel", "power"]:
        prep_tab(pg, t)
        for g in pg.evaluate(f"prepSteps('{t}').filter((g) => prepRequired(g) && prepStepState(g, '{t}').k === 'open').map((g) => g.id)"):
            box = f'input[data-a="agent"][data-g="{g}"]'
            if pg.locator(box).count():
                prep_open(pg, f'article[data-gap="{g}"]')
                pg.locator(box).first.check(); pg.wait_for_timeout(50)
    prep_idle(pg)
    views = pg.evaluate("prepViewsNow()")
    for t, v in views.items():
        left = pg.evaluate(f"prepSteps('{t}').filter((g) => prepRequired(g) && prepStepState(g, '{t}').k === 'open').map((g) => g.id)")
        if v["applicable"] and v["state"] != "ready" and left:
            problems.append(f"{tag}: acceptance {label}: {t} is {v['state']} with gaps not handed over: {left}")
    step(f"accept_{label}")
    pg.select_option("#prep-scope", "all")
    if pg.locator("#prep-save").is_disabled():
        problems.append(f"{tag}: acceptance {label}: save disabled ({pg.inner_text('#prep-msgs')[:300]})")
        return
    text = prep_download(pg)
    open(f"{out}/accept_{label}.kisi.json", "w", encoding="utf-8").write(text)
    ck = pg.evaluate("(t) => S.engine.configCheck(prepBoardFile(), t)", text)
    if not ck["ok"]:
        problems.append(f"{tag}: acceptance {label}: the saved file fails the check: {ck['errors'][:3]}")
    got = json.loads(text)
    key = lambda f: (f["ref"], f["name"].upper())
    # capacitor data compares by value (its source names the parts' fields in the page's words)
    caps = lambda c: {key(f): f["value"] for f in c.get("fields", []) if f["name"].upper() == "PI_CAP"}
    if caps(got) != caps(ref):
        problems.append(f"{tag}: acceptance {label}: PI_CAP: {len(caps(got))} saved, {len(caps(ref))} in the reference, {sum(1 for k in caps(ref) if caps(got).get(k) != caps(ref)[k])} differ")
    nocap = lambda c: [f for f in c.get("fields", []) if f["name"].upper() != "PI_CAP"]
    if (ref.get("stackup") or {}) != (got.get("stackup") or {}):
        problems.append(f"{tag}: acceptance {label}: the saved stackup differs from the reference")
    nets_of = lambda pats: pg.evaluate("(p) => S.scene.nets.filter((n, i) => i > 0 && net_matches_js(p, n))", pats)
    for c, e in (ref.get("net_classes") or {}).items():
        g_ = (got.get("net_classes") or {}).get(c)
        if not g_ or {k: v for k, v in g_.items() if k != "nets"} != {k: v for k, v in e.items() if k != "nets"} or sorted(nets_of(g_.get("nets", []))) != sorted(nets_of(e.get("nets", []))):
            problems.append(f"{tag}: acceptance {label}: net class {c} saved as {g_}, reference {e}")
    if sorted(nocap(got), key=key) != sorted(nocap(ref), key=key):
        a_, b_ = {key(f): f for f in nocap(got)}, {key(f): f for f in nocap(ref)}
        diff = [k for k in set(a_) | set(b_) if a_.get(k) != b_.get(k)]
        k0 = sorted(diff)[0]
        problems.append(f"{tag}: acceptance {label}: {len(diff)} saved fields differ from the reference: {sorted(diff)[:6]}; first: {a_.get(k0)} – reference {b_.get(k0)}")
    for k in ("variants", "variant", "notes"):
        if got.get(k) != ref.get(k):
            problems.append(f"{tag}: acceptance {label}: the saved {k} differ from the reference")
    state = pg.evaluate("JSON.stringify([prepViewsNow(), prepGapsNow().map((g) => [g.id, g.items.length]), prepEntries(prepW()).size])")
    # a new page: the board and the saved file give the same state
    pg.reload(); pg.wait_for_function("document.getElementById('busy').hidden === true || document.getElementById('drop').classList.contains('start')", timeout=90000)
    prep_load(pg, board_js, text)
    again = pg.evaluate("JSON.stringify([prepViewsNow(), prepGapsNow().map((g) => [g.id, g.items.length]), prepEntries(prepW()).size])")
    if again != state or pg.evaluate("prepDirty()"):
        problems.append(f"{tag}: acceptance {label}: board + saved file after a reload give another state")
    notes.append(f"{tag}: acceptance {label}: {len(ref.get('fields', []))} fields, {len(ref.get('variants', {}))} variants entered and saved in {time.time() - t0:.0f} s; "
                 + ", ".join(f"{t} {v['state']}" for t, v in views.items()))
    pg.evaluate("() => { lsSet(PREP_KEY(), null); S.prep.mem = {}; }")


def run(pg, tag, desktop):
    errs = []
    pg.add_init_script(TEXT_SPY)
    pg.on("pageerror", lambda e: errs.append(f"pageerror: {e}"))
    pg.on("console", lambda m: errs.append(f"console {m.type}: {m.text}") if m.type == "error" else None)
    pg.goto(url)
    pg.wait_for_function("document.getElementById('busy').hidden === true || document.getElementById('drop').classList.contains('start')", timeout=90000)
    if not pg.evaluate("!!S.scene"):  # start screen: no control may fail without a board
        start_screen(pg)
    if a.board:
        # with --config the board configuration comes with the board, so every view uses it
        pg.set_input_files("#file", [os.path.abspath(f) for f in a.board + ([a.config] if a.config else [])])
        pg.wait_for_function(f"S.scene && {BUSY}" + (" && !!S.config" if a.config else ""), timeout=90000)
        if a.config and not pg.evaluate("S.config.check.ok"):
            problems.append(f"{tag}: kisi.json check failed: {pg.evaluate('S.config.check.errors')[:3]}")
    elif not pg.evaluate("!!S.scene"):
        raise SystemExit("the build has no example boards: pass --board")
    step = lambda name: pg.screenshot(path=f"{out}/{tag}_{name}.png")
    if a.prepare_only:
        return prepare_all(pg, tag, step, errs)
    for v in ["si", "ch", "pi"]:
        pg.click(f"#view-{v}"); pg.wait_for_timeout(300); step(f"start_{v}")
    pg.click("#view-si")
    pg.click("#an-start"); pg.wait_for_function(f"S.full && {BUSY}", timeout=90000); step("estimate")
    # a board without impedance targets (no net rows): the steps that need such nets are skipped
    targets = pg.evaluate("S.an.nets.filter((n) => n.target).length")
    if not targets:
        notes.append(f"{tag}: no nets with a target impedance – net detail, couplings and field solver steps skipped")
    if desktop and not a.quick and targets:
        pg.click(".net >> nth=0"); pg.wait_for_timeout(400)
        for t in ["loss", "tdr", "via", "xt"]:
            pg.click(f"#detail [data-tab={t}]"); pg.wait_for_timeout(300)
        step("net")
        board_text(pg, tag, "the net view (A/B marks)")
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
    if targets:
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
    # the export settings (card "Export for simulators"): collapsed by default, remembered when opened
    if not a.quick and pg.locator("#view-ch").is_visible():
        pg.click("#view-ch"); pg.wait_for_timeout(400)
        pg.evaluate("document.getElementById('h-exp').scrollIntoView()"); pg.wait_for_timeout(200)
        if pg.evaluate("$('ex-opts').open"):
            problems.append(f"{tag}: export settings open by default")
        step("export_settings")
        pg.click("#ex-opts summary"); pg.wait_for_timeout(200)
        pg.evaluate("document.getElementById('h-exp').scrollIntoView()"); pg.wait_for_timeout(200)
        step("export_settings_open")
        if not pg.evaluate("(lsGet('kisi.export.v1') || {}).open === true && $('ex-lib').disabled"):
            problems.append(f"{tag}: export settings not remembered, or the netlist not tied to the benches")
        pg.click("#ex-opts summary"); pg.wait_for_timeout(200)
    if desktop and not a.quick:
        pg.click("#view-ch"); pg.wait_for_timeout(500)
        # a board with assembly variants and none chosen: the channel view asks before computing lanes
        if pg.evaluate("variantNeeded()"):
            if not pg.locator("#ch-overview [data-variant]").count() or pg.evaluate("!!S.laneRes"):
                problems.append(f"{tag}: assembly variant needed, but no prompt in the channel view")
            else:
                step("variant_prompt")
                pg.click("#ch-overview [data-variant] >> nth=0"); pg.wait_for_function(f"!variantNeeded() && {BUSY}", timeout=90000)
                pg.wait_for_timeout(300)
        # "Export…" only after the check: greyed with a hint in the bus cards and the lane detail
        if pg.locator("#ch-overview [data-export]").count() and not pg.evaluate("[...document.querySelectorAll('#ch-overview [data-export]')].every((b) => b.disabled && b.title === 'run the channel check first')"):
            problems.append(f"{tag}: bus export enabled before the channel check")
        if pg.locator("#ch-overview tr[data-lane]").count():
            pg.click("#ch-overview tr[data-lane] >> nth=0"); pg.wait_for_timeout(600)
            if pg.locator("#ch-export").count() and not pg.evaluate("$('ch-export').disabled && $('ch-export').title === 'run the channel check first'"):
                problems.append(f"{tag}: lane export enabled before the channel check")
            step("export_disabled")
            pg.click("#chm-o"); pg.wait_for_timeout(300)
        t0 = time.time()
        pg.click("#ck-start"); pg.wait_for_function("!S.checkRunning && S.laneRes", timeout=300000)
        t_first = time.time() - t0
        pg.wait_for_timeout(300); step("check")
        # the check in parallel (check-pool.js) and on one thread: the same rows
        if tag == "light":
            rows = "JSON.stringify(S.check.rows.map((r) => { const { lane, ...x } = r; return x; }))"
            timed = "async (n) => { if (n) S.checkThreads = n; else delete S.checkThreads; const t0 = performance.now(); await runCheck(); delete S.checkThreads; return [S.checkWorkers || 0, Math.round(performance.now() - t0), %s]; }" % rows
            first = pg.evaluate(rows)
            w, t_par, par = pg.evaluate(timed, 0)
            _, t_one, one = pg.evaluate(timed, 1)
            if not (first == par == one):
                problems.append(f"{tag}: the parallel check ({w + 1} threads) gives other rows than on one thread")
            notes.append(f"{tag}: channel check {pg.evaluate('S.name')} ({pg.evaluate('S.check.rows.length')} channels, {pg.evaluate('navigator.hardwareConcurrency')} cores): "
                         f"{w + 1} threads {t_first:.1f} s with setting the workers up, {t_par / 1000:.1f} s again; one thread {t_one / 1000:.1f} s; rows {'identical' if first == par == one else 'DIFFERENT'}")
        if pg.locator("#ch-overview [data-export]").count() and not pg.evaluate("[...document.querySelectorAll('#ch-overview [data-export]')].some((b) => !b.disabled)"):
            problems.append(f"{tag}: bus export still greyed after the channel check")
        if pg.locator("#ch-overview tr[data-lane]").count():
            pg.click("#ch-overview tr[data-lane] >> nth=0"); pg.wait_for_timeout(600)
            if pg.locator("#ch-export").count() and pg.evaluate("$('ch-export').disabled"):
                problems.append(f"{tag}: lane export still greyed after the channel check")
            if pg.locator("#ch-detail [data-ct=eye]").count():
                pg.click("#ch-detail [data-ct=eye]"); pg.wait_for_timeout(600)
            step("lane")
            # the lane's export (zip): the subcircuit library, the benches, Touchstone, KiCad project, README
            if tag == "light" and pg.locator("#ch-export").count():
                with pg.expect_download(timeout=120000) as dl:
                    pg.click("#ch-export")
                names = zip_names(dl.value.path())
                if not (any(n.endswith(".lib") for n in names) and "README.md" in names and any(n.endswith(".kicad_sch") for n in names)):
                    problems.append(f"{tag}: lane export zip incomplete: {names}")
                else:
                    notes.append(f"{tag}: lane export {dl.value.suggested_filename}: {len(names)} files")
        # switch to another variant in the side card (lanes computed again) and back
        if pg.evaluate("!!S.variant && S.variant.variants.length > 1"):
            pg.click("#chm-o"); pg.wait_for_timeout(300)
            other = pg.evaluate("S.variant.variants.map((v) => v.name).find((n) => n !== S.variant.chosen)")
            pg.select_option("#variant-sel", other); pg.wait_for_function(f"S.variant.chosen === {json.dumps(other)} && {BUSY}", timeout=90000)
            pg.wait_for_timeout(400); step("variant_other")
            if not pg.evaluate("S.buses && S.buses.length > 0 && !S.laneRes") or other not in pg.inner_text("#ch-overview .bal-head"):
                problems.append(f"{tag}: lanes not recomputed or variant not shown after switching to {other}")
            pg.evaluate("setVariant(null)"); pg.wait_for_function(f"S.variant.chosen_by !== 'viewer' && {BUSY}", timeout=90000)
            pg.wait_for_timeout(300)
        pg.click("#view-pi"); pg.wait_for_timeout(400)
        if not pg.locator("#pi-run").is_disabled():
            pg.click("#pi-run"); pg.wait_for_function(f"S.piSolved && !S.piRunning && {BUSY}", timeout=300000); pg.wait_for_timeout(300)
        step("power")
        # impedance of the selected rail seen from its ICs
        pg.evaluate("if (S.piRail) { S.piTab = 'z'; renderPiDetail(); }"); pg.wait_for_timeout(500)
        if pg.locator("#pdn-ic-run").count():
            pg.click("#pdn-ic-run"); pg.wait_for_function("document.getElementById('ch-pdn-ic') || document.querySelector('#pdn-ic .err')", timeout=300000)
            pg.wait_for_timeout(300); step("pdn_ic")
            board_text(pg, tag, "the PDN view (parts seen from the IC)", expect=pg.evaluate("!!document.getElementById('ch-pdn-ic')"))
            if pg.locator("#pdn-ic details.acc").count():
                pg.evaluate("document.querySelector('#pdn-ic details.acc').open = true"); pg.wait_for_timeout(200)
                pg.evaluate("document.querySelector('#pdn-ic details.acc').scrollIntoView()"); pg.wait_for_timeout(200); step("pdn_domain")
            pg.evaluate("S.piTab = 'dc'; renderPiDetail()")
        if not pg.locator("#pim-l").is_disabled():
            pg.click("#pim-l"); pg.wait_for_timeout(500); step("losses")
            pg.click("#pim-b"); pg.wait_for_timeout(500); step("flow")
            # power flow: a load jumps to its part on the board
            lid = pg.evaluate("(() => { const g = [...document.querySelectorAll('.sankey [data-node]')].find((x) => x.dataset.node.startsWith('load:')); return g ? g.dataset.node : null; })()")
            if lid:
                pg.click(f".sankey [data-node='{lid}']"); pg.wait_for_timeout(500)
                if not pg.evaluate("S.piMode === 'v' && !!S.piMark"):
                    problems.append(f"{tag}: click on a load in the power flow did not show the part")
                board_text(pg, tag, "the voltage drop view (part from the power flow)")
                step("flow_jump"); pg.click("#pim-b"); pg.wait_for_timeout(300)
            # before step 3 the components table offers "Thermal" only as a disabled choice
            if not pg.evaluate("thermOk() || (!!document.querySelector('.bal-comp [data-cm=t]') && document.querySelector('.bal-comp [data-cm=t]').disabled)"):
                problems.append(f"{tag}: components table: the Thermal columns are not disabled before step 3")
            # power tree and component list: part references jump to the board as well
            for sel in [".tree [data-part]", ".ctab [data-part]"]:
                if pg.locator(sel).count():
                    pg.click(f"{sel} >> nth=0"); pg.wait_for_timeout(400)
                    if not pg.evaluate("S.piMode === 'v' && !!S.piMark"):
                        problems.append(f"{tag}: click on a part in {sel} did not show it")
                    pg.click("#pim-b"); pg.wait_for_timeout(300)
        # temperature (step 3): map per layer, junction temperatures, energy balance; a layer and the
        # losses view in between, back to the power tree
        if not pg.locator("#therm-run").is_disabled():
            pg.click("#therm-run"); pg.wait_for_function(f"S.therm && !S.thermRunning && {BUSY}", timeout=300000); pg.wait_for_timeout(400)
            if pg.evaluate("!!S.therm.error"):
                problems.append(f"{tag}: temperature: {pg.evaluate('S.therm.error')}")
            elif not pg.evaluate("S.piMode === 't' && S.therm.parts.length > 0 && S.therm.balance.error_rel < 1e-6 && S.therm.rails.length > 0 && !S.therm.rails.some((r) => r.error)"):
                problems.append(f"{tag}: temperature view not shown, without parts or with failed rails")
            step("temperature")
            # units in the panel, the legend and the tooltips (a via and the board); a rail and a part in the
            # tables jump to the map; the vias and the parts' heat paths are there
            t = pg.evaluate("""async () => {
              const wait = () => new Promise((r) => setTimeout(r, 150)), out = [];
              const txt = (id) => { const e = $(id); return e && !e.hidden ? e.innerText : ""; };
              const tip = (bx, by) => { showThermTip(S.view.x + bx * S.view.s, S.view.y + by * S.view.s); const e = $("tip"); const s = e.hidden ? "" : e.innerText; e.hidden = true; return s; };
              out.push(txt("pi-detail") + " " + txt("legend") + " " + txt("therm-step"));
              const v = thermVias().list[0];
              if (v) out.push(tip(v.at[0], v.at[1]));
              const p = S.therm.parts[0];
              if (p) out.push(tip(p.at[0], p.at[1]));
              const r = document.querySelector("#pi-detail [data-trail]"); if (r) { r.click(); await wait(); }
              const railOk = !r || (!!S.thermMark && S.thermLayer >= 0);
              const q = document.querySelector("#pi-detail [data-tpart]"); if (q) { q.click(); await wait(); }
              const partOk = !q || (!!S.thermMark && S.thermMark.label.startsWith(q.dataset.tpart));
              setPiMode("b"); await wait(); out.push(txt("pi-balance"));
              // components table: "Thermal" columns behind the toggle, remembered, "Power" again
              const cm = (k) => { const b = document.querySelector(`.bal-comp [data-cm=${k}]`); if (b && !b.disabled) b.click(); };
              cm("t"); await wait(); out.push(txt("pi-balance"));
              const head = () => (document.querySelector(".bal-comp thead") || { innerText: "" }).innerText;
              const thermHead = head();
              setPiMode("v"); await wait(); setPiMode("b"); await wait();
              const kept = head() === thermHead;
              cm("p"); await wait();
              const comp = thermHead.includes("Tj") && thermHead.includes("θ") && thermHead.includes("source") && kept && head().includes("consumption") && !head().includes("Tj");
              setPiMode("t"); await wait();
              return { texts: out, railOk, partOk, comp, vias: S.therm.vias.length, under: S.therm.parts.filter((x) => x.vias).length };
            }""")
            for tx in t["texts"]:
                bad = sorted(set(m.group(0) for m in FIXED_ZERO.finditer(tx)))
                if bad:
                    problems.append(f"{tag}: fixed-decimal values in the temperature view: {bad[:6]}")
            if not (t["railOk"] and t["partOk"]):
                problems.append(f"{tag}: a rail or a part in the temperature tables did not jump to the map")
            if not t["comp"]:
                problems.append(f"{tag}: components table: no Thermal columns (Tj, θ, source) behind the toggle, choice not kept, or Power columns not back")
            if not (t["vias"] > 0 and t["under"] > 0):
                problems.append(f"{tag}: no via heat or heat paths under the parts")
            step("temperature_jump")
            pg.click("#layers .chip >> nth=-1"); pg.wait_for_timeout(300)
            # tooltip over the board, left of the detail panel
            bx = pg.locator("#pcb").bounding_box()
            pg.mouse.move(bx["x"] + 0.25 * bx["width"], bx["y"] + 0.5 * bx["height"]); pg.wait_for_timeout(200); step("temperature_bottom")
            pg.click("#pim-b"); pg.wait_for_timeout(300)
        power_units(pg, tag, step)
        if a.config and not a.board:
            pg.click("#view-si")
            pg.set_input_files("#file", [os.path.abspath(a.config)]); pg.wait_for_function(f"S.config && {BUSY}", timeout=90000)
            ck = pg.evaluate("S.config.check")
            if not ck["ok"]:
                problems.append(f"{tag}: kisi.json check failed: {ck['errors'][:3]}")
            step("config")
    # the other example boards of the build: power views with the voltage drop, units and labels
    if desktop and not a.quick and not a.board and tag == "light":
        for key in pg.evaluate("Object.keys(SAMPLES).filter((k) => k !== 'ts')"):
            pg.evaluate(f"loadSample({json.dumps(key)})"); pg.wait_for_function(f"S.scene && {BUSY}", timeout=90000)
            pg.click("#view-pi"); pg.wait_for_timeout(400)
            if not pg.locator("#pi-run").is_disabled():
                pg.click("#pi-run"); pg.wait_for_function(f"S.piSolved && !S.piRunning && {BUSY}", timeout=300000); pg.wait_for_timeout(300)
            step(f"power_{key}")
            power_units(pg, f"{tag} ({key})", lambda n, k=key: step(f"{n}_{k}"))
    if pg.evaluate("document.documentElement.scrollWidth > window.innerWidth + 1"):
        problems.append(f"{tag}: horizontal scroll")
    prepare_all(pg, tag, step, errs)


def prepare_all(pg, tag, step, errs):
    prepare_page(pg, tag, step)
    if pg.evaluate("typeof SAMPLES !== 'undefined' && !!SAMPLES.ts && !!SAMPLES.ts.config"):
        prepare_examples_clean(pg, tag, step)
        if tag != "phone":
            prepare_marks(pg, tag, step)
        prepare_checklist(pg, tag, step)
        prepare_handover(pg, tag, step)
        prepare_stale_draft(pg, tag, step)
        prepare_roundtrip(pg, tag, step)
        if tag == "light" and not a.quick:
            prepare_acceptance(pg, tag, step, SAMPLE_TS, pg.evaluate("SAMPLES.ts.config.text"), {}, "ts")
    if a.accept and tag == "light":
        pcb = os.path.abspath(a.accept[0]); pro = re.sub(r"\.kicad_pcb$", ".kicad_pro", pcb)
        js = "Promise.resolve([%s, %s, %s])" % (json.dumps(open(pcb, encoding="utf-8").read()), json.dumps(open(pro, encoding="utf-8").read() if os.path.exists(pro) else None), json.dumps(re.sub(r"\.kicad_pcb$", "", os.path.basename(pcb))))
        prepare_acceptance(pg, tag, step, js, open(a.accept[1], encoding="utf-8").read(), dict(x.split("=", 1) for x in a.accept_dismiss), os.path.basename(pcb).split(".")[0].split(" ")[0].lower())
    for e in errs:
        (notes if "Failed to load resource" in e else problems).append(f"{tag}: {e}")


with sync_playwright() as p:
    b = getattr(p, a.browser).launch()
    layouts = a.layouts.split(",")
    for scheme, w, h, tag in [("light", 1440, 900, "light"), ("dark", 1440, 900, "dark"), ("light", 400, 860, "phone")]:
        if tag not in layouts:
            continue
        pg = b.new_page(viewport={"width": w, "height": h}, color_scheme=scheme)
        run(pg, tag, w > 800)
        pg.close()
        print(f"{tag}: done ({a.browser})")
    b.close()
for n in sorted(set(notes)):
    print("note:", n)
    if os.environ.get("GITHUB_ACTIONS") and "channel check" in n:
        print(f"::notice title=viewer check timing::{n}")
for pr in problems:
    print("PROBLEM:", pr)
print(f"screenshots: {out}")
sys.exit(1 if problems else 0)
