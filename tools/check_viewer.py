#!/usr/bin/env python3
"""Headless check of the bundled viewer (Playwright + Chromium).

Usage: check_viewer.py <kisi.html> [--board board.kicad_pcb [board.kicad_pro]] [--config board.kisi.json]
                       [--out DIR] [--quick]

* light, dark and phone layout; all three views (impedance, channel, power)
* impedance estimate, net detail tabs (a differential pair: 4-port loss, skew, common mode), interfaces overview,
  assembly variant (the prompt when the board needs one, switching to another variant and back), channel check,
  lane eye, voltage drop with ground return, impedance seen from the ICs, losses map, temperature and power flow
  (desktop layouts; --quick: only load and the three views)
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
import argparse, json, os, re, sys, tempfile
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
        pg.click("#ck-start"); pg.wait_for_function("!S.checkRunning && S.laneRes", timeout=300000); pg.wait_for_timeout(300); step("check")
        if pg.locator("#ch-overview tr[data-lane]").count():
            pg.click("#ch-overview tr[data-lane] >> nth=0"); pg.wait_for_timeout(600)
            if pg.locator("#ch-detail [data-ct=eye]").count():
                pg.click("#ch-detail [data-ct=eye]"); pg.wait_for_timeout(600)
            step("lane")
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
              const comp = !!document.querySelector(".bal-comp thead") && document.querySelector(".bal-comp thead").innerText.includes("Tj");
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
                problems.append(f"{tag}: no temperature in the power tree's components table")
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
