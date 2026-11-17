"use strict";
// ============================================================== export (the Export menu in the top bar)
// "Table (CSV)": the tables of the view shown, read from the page as drawn – the rows in the order,
// filter and sort shown there; one reader for every view (xpTables → xpCsv). Values with units become
// plain numbers in the column's most common prefix, the unit goes into the header; the marks the views
// set after a value (* estimate, ? uncertain, ⚠ warning) move into a column of their own.
// "Results (JSON)": `kisi report` on the loaded files (kisi_core::commands, as kisi.mjs) in Web
// Workers with engines of their own, with this browser's field-solver results; the check's rows in parts
// (kisi_cli_check_part), so its progress shows. Its result is kept for the next export of the same state.
const XP_UNIT = /^([+-]?(?:\d[\d,]*(?:\.\d+)?|\.\d+))\s?(%|dB|°C|K\/W|K|([fpnuµmkMG]?)(A\/mm²|W\/mm²|V|A|W|Hz|F|H|Ω|m|s))$/;
const XP_MARK = { "*": "estimate", "?": "uncertain", "⚠": "warning" };
// the view shown: its name in the file name and where its tables are, or why it has none
function xpView() {
  if (S.prep && S.prep.open) return { why: "the Prepare PCB page has no result table" };
  const t = S.tab, m = t === "pi" ? S.piMode : t === "ch" ? S.chMode || "o" : "";
  if (t === "si") return { key: "impedance", model: () => [netsTable()] };
  if (t === "stk") return { key: "stackup", box: $("stk-panel") };
  // what a view lacks and the viewer can compute first (compute, what): the channel check, the DC drop,
  // the temperature
  if (t === "ch") {
    if (m === "o") return { key: "interfaces", box: $("ch-overview") };
    const can = !S.check && S.an && !S.an.skeleton && !variantNeeded();
    return { key: "channel-check", box: $("ck-out"), why: S.an && S.an.skeleton ? "run the impedance estimate and the channel check first (card “Channel check”)" : "run the channel check first (card “Channel check”)",
      ...(can ? { compute: () => runCheck(), what: "the channel check" } : {}) };
  }
  const key = { b: "power-tree", v: "drop", j: "density", l: "losses", t: "temperature" }[m];
  if (m === "b") return { key, box: $("pi-balance") };
  const rails = S.piSetups && S.piSetups.size > 0;
  const dc = rails && !S.piSolved ? { compute: () => solveAllRails(), what: "the DC drop of every rail" } : {};
  const th = rails && m === "t" && !thermOk() ? { compute: async () => { if (!S.piSolved) await solveAllRails(); await solveThermal(); }, what: S.piSolved ? "the temperature" : "the DC drop and the temperature" } : {};
  return { key, box: $("pi-detail"), why: m === "v" || m === "j" ? "select a rail (card “Supply rails”)" : "compute this step first", ...dc, ...th };
}
// a heading's own text (without its buttons and notes)
function xpHead(h) { const c = h.cloneNode(true); c.querySelectorAll("button, .seg, em, small").forEach((x) => x.remove()); return c.textContent.replace(/\s+/g, " ").trim(); }
const xpShown = (el) => !!el && !el.closest("[hidden]") && !el.closest(".off");
// a table of the page → {title, group, head: [text], rows: [[text]]}
function xpRead(tb) {
  // the text as drawn (its lines joined by "; "), a form's value, else a status dot's label
  const dots = new Set();
  const txt = (el, i) => {
    const f = el.querySelector("input, select"), a = el.querySelector("[aria-label]");
    let s = f ? f.value : el.innerText.trim().split(/\s*\n\s*/).join("; ");
    if (!s && a) { s = a.getAttribute("aria-label"); dots.add(i); }
    return s.replace(/[\s\u00a0\u202f]+/g, " ").replace(/\u2212/g, "-").trim();
  };
  const row = (tr) => [...tr.children].flatMap((c, i) => [txt(c, i), ...Array(Math.max(0, (+c.colSpan || 1) - 1)).fill("")]);
  const hr = tb.tHead && tb.tHead.rows.length ? tb.tHead.rows[tb.tHead.rows.length - 1] : null;
  const rows = [...tb.tBodies].flatMap((b) => [...b.rows]).filter((tr) => !tr.closest("[hidden]")).map(row);
  const head = hr ? [...hr.children].map((c, i) => c.dataset.csv || txt(c, -1) || (dots.has(i) ? "Status" : "")) : (tb.dataset.head || "").split("|");
  // the heading above the table (h4 in a panel), the card's name as the group of merged tables
  let h = tb.closest(".tablewrap, .ck-scroll, .stk-scroll") || tb;
  while (h && !/^H[2-4]$/.test(h.tagName)) h = h.previousElementSibling;
  const card = tb.closest(".bal-card, .detail, .card");
  const top = card && card.querySelector("h3, h2");
  return { title: [top, h].filter(Boolean).map(xpHead).join(" – "),
    group: tb.dataset.group || "", groupHead: tb.dataset.groupHead || "", head, rows };
}
// the view's tables: [{title, head, rows}] – tables with the same header in one view merged, a column
// naming each one's group (a bus); or {why}
function xpTables() {
  const v = xpView();
  if (!v.key) return v;
  let ts = v.model ? v.model() : [...((xpShown(v.box) && v.box.querySelectorAll("table")) || [])].filter(xpShown).map(xpRead);
  ts = ts.filter((t) => t.rows.length);
  if (!ts.length) return { why: v.why || "this view shows no table", key: v.key, compute: v.compute, what: v.what };
  const out = [], h = v.box && v.box.querySelector("h3"), title = h ? xpHead(h) : "";
  for (const t of ts) {
    const sig = t.head.join("|"), m = t.groupHead && out.find((o) => o.sig === sig && o.groupHead);
    const rows = t.groupHead ? t.rows.map((r) => [t.group, ...r]) : t.rows;
    if (m) m.rows.push(...rows);
    else out.push(t.groupHead ? { ...t, sig, title, head: [t.groupHead, ...t.head], rows } : { ...t, sig });
  }
  return { key: v.key, tables: out };
}
// columns: values with one unit → numbers in the most common prefix (unit in the header), marks into a
// column of their own, empty columns without a header dropped
function xpColumns(t) {
  const cols = [];
  t.head.forEach((h, c) => {
    const marks = {}, cells = t.rows.map((r) => {
      let s = r[c] == null ? "" : String(r[c]);
      const mk = s.match(/\s*([*?⚠])$/);
      if (mk && s.length > 1) { s = s.slice(0, mk.index); marks[mk[1]] = 1; }
      return { s: s === "–" || s === "-" ? "" : s, mark: mk ? mk[1] : "" };
    });
    const full = cells.filter((x) => x.s);
    if (!h && !full.length) return;
    const p = full.map((x) => x.s.match(XP_UNIT));
    let head = h, vals = cells.map((x) => x.s);
    const unit = p.length && p.every((m) => m) && p.every((m) => (m[4] || m[2]) === (p[0][4] || p[0][2])) ? (p[0][4] || p[0][2]) : null;
    if (unit) {
      const n = {}; p.forEach((m) => { n[m[3] || ""] = (n[m[3] || ""] || 0) + 1; });
      const pre = p[0][4] ? Object.keys(n).sort((a, b) => n[b] - n[a])[0] : "", f = (q) => q ? SI_FACTOR[q] : 1;
      vals = cells.map((x) => { const m = x.s && x.s.match(XP_UNIT); return m ? String(+(Number(m[1].replace(/,/g, "")) * f(m[3] || "") / f(pre)).toPrecision(6)) : ""; });
      if (!(" " + h + " ").includes(` ${pre + unit} `) || h.startsWith(pre + unit + " ")) head = `${head} (${pre + unit})`;
    } else vals = vals.map((s) => /^[+-]?\d{1,3}(,\d{3})+(\.\d+)?$/.test(s) ? s.replace(/,/g, "") : s);
    cols.push({ head, vals });
    for (const k of Object.keys(marks)) cols.push({ head: `${head} ${XP_MARK[k]}`, vals: cells.map((x) => x.mark === k ? XP_MARK[k] : "") });
  });
  return cols;
}
// RFC 4180 with CRLF; a text that Excel would take for a formula (+5V, -5V, =…) starts with an apostrophe
function xpCsv(tables) {
  const q = (s) => { s = String(s); if (/^[=+\-@]/.test(s) && !isFinite(Number(s))) s = "'" + s; return /[",\r\n]|^\s|\s$/.test(s) ? `"${s.replace(/"/g, '""')}"` : s; };
  const out = [];
  for (const t of tables) {
    const cols = xpColumns(t);
    if (out.length) out.push("");
    out.push(q(t.title || ""), cols.map((c) => q(c.head)).join(","));
    for (let r = 0; r < t.rows.length; r++) out.push(cols.map((c) => q(c.vals[r])).join(","));
  }
  return "﻿" + out.join("\r\n") + "\r\n";
}
const xpDate = () => { const d = new Date(), z = (n) => String(n).padStart(2, "0"); return `${d.getFullYear()}-${z(d.getMonth() + 1)}-${z(d.getDate())}`; };
const xpFile = (view, ext) => `${String(S.name || "board").replace(/[^\w.-]+/g, "_")}_${view}_${xpDate()}.${ext}`;

// ---- kisi report in workers
const XP_WORKER_SRC = `
const enc = new TextEncoder(), dec = new TextDecoder();
let k = null;
self.onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.module) k = (await WebAssembly.instantiate(m.module, {})).exports;
    const b = enc.encode(m.text), p = k.kisi_alloc(b.length);
    new Uint8Array(k.memory.buffer, p, b.length).set(b);
    const rc = k[m.fn](p, b.length);
    k.kisi_free(p, b.length);
    const o = dec.decode(new Uint8Array(k.memory.buffer, k.kisi_out_ptr(), k.kisi_out_len()));
    self.postMessage(rc === 0 ? { id: m.id, text: o } : { id: m.id, error: o });
  } catch (err) { self.postMessage({ id: m.id, error: String(err && err.message || err) }); }
};`;
const XP = { run: null, done: null, url: null };
// the inputs of kisi_cli_project: the loaded files, the variant and stackup the views use
function xpInputs() {
  const l = S.lastFiles, cfg = l.cfg, opts = { cmd: "report", pos: [S.name + ".kicad_pcb"] };
  if (S.variant && S.variant.chosen && S.variant.chosen_by === "viewer") opts.variant = S.variant.chosen;
  const a = typeof activeStackup === "function" ? activeStackup() : null, by = (o) => new Map(((o && o.dielectrics) || []).map((d) => [d.index, d]));
  let su = stackParam();
  if (a && a.preset) { opts.stackup_preset = a.preset; su = a.edits; }
  else if (a && a.edits) { const m = by(su); for (const [i, d] of by(a.edits)) m.set(i, { ...m.get(i), ...d }); su = { ...a.edits, dielectrics: [...m.values()] }; }
  const t = fabOn() && tolEff();
  if (t) opts.tol = `thickness=${t.dielectric_thickness_pct},er=${t.er_pct},copper=${t.copper_pct},width=${t.width_pct != null ? t.width_pct + "%" : t.width_mm + "mm"}`;
  const files = {};
  for (const [k, v] of Object.entries(cfgFiles(cfg))) files[k] = { text: v };
  for (const [k, v] of Object.entries(S.tsFiles || {})) files[k] = { text: v.text };
  return { opts, pcb_path: opts.pos[0], pcb: l.pcbText, pro: l.proText, config: cfg ? { path: cfg.name, text: cfg.text } : null,
    stackup: su && su.dielectrics && su.dielectrics.length ? su : null, field: {}, files };
}
// the report as text (pretty JSON, as the CLI writes it); progress(text, done, n); kept while nothing changed
async function xpReport(progress) {
  const inputs = xpInputs(), key = JSON.stringify([S.engine.stateGen, S.field.size, inputs.opts, inputs.stackup]);
  if (XP.done && XP.done.key === key) return XP.done.text;
  if (XP.run) return XP.run.p;
  const ws = [], jobs = new Map(), cancel = { on: false };
  let id = 0;
  // the first request of a worker carries the compiled engine
  const call = (w, fn, v) => new Promise((ok, fail) => {
    const i = ++id; jobs.set(i, { ok, fail });
    w.postMessage({ module: w.fresh ? S.engine.mod : null, id: i, fn, text: JSON.stringify(v) }); w.fresh = false;
  });
  const p = (async () => {
    XP.url = XP.url || URL.createObjectURL(new Blob([XP_WORKER_SRC], { type: "text/javascript" }));
    const n = Math.max(1, Math.min(3, (navigator.hardwareConcurrency || 2) - 1));
    for (let i = 0; i < n; i++) {
      const w = new Worker(XP.url);
      w.fresh = true;
      w.onmessage = (e) => { const j = jobs.get(e.data.id); jobs.delete(e.data.id); if (j) e.data.error != null ? j.fail(new Error(e.data.error)) : j.ok(e.data.text); };
      w.onerror = (e) => { e.preventDefault(); for (const j of jobs.values()) j.fail(new Error(e.message || "worker failed")); jobs.clear(); };
      ws.push(w);
    }
    const stop = () => { if (cancel.on) throw new Error("cancelled"); };
    progress("reading the board", 0, 0);
    // the field plan of the board, then every engine with this browser's results for it
    const { plan } = JSON.parse(await call(ws[0], "kisi_cli_project", inputs));
    for (const [gid] of plan) { const r = S.field.get(gid); if (r && r.ok) inputs.field[gid] = r; }
    stop();
    await Promise.all(ws.map((w) => call(w, "kisi_cli_project", inputs)));
    const { count } = JSON.parse(await call(ws[0], "kisi_cli_check_part", { opts: inputs.opts, items: [] }));
    const rows = [], queue = Array.from({ length: count }, (_, i) => i);
    const feed = async (w) => {
      while (queue.length) {
        stop();
        rows.push(...JSON.parse(await call(w, "kisi_cli_check_part", { opts: inputs.opts, items: queue.splice(0, 2) })).rows);
        progress(`channel check ${rows.length}/${count} rows`, rows.length, count);
      }
    };
    progress(`channel check 0/${count} rows`, 0, count);
    await Promise.all(ws.map(feed));
    stop();
    progress("power: DC drop of every rail, losses, temperature", count, count);
    const res = JSON.parse(await call(ws[0], "kisi_cli_run", { opts: inputs.opts, tree: null, report_path: "analysis/kisi.json", check_rows: count >= 1 ? rows : null }));
    const text = res.files[0].content;
    // the findings by severity and the nets they are on, for the menu's descriptions
    let counts = null;
    try {
      const rep = JSON.parse(text), fs = rep.findings || [], n = (k) => fs.filter((f) => f.severity === k).length;
      counts = { n: fs.length, fail: n("fail"), warn: n("warn"), gap: n("gap"), groups: (rep.finding_groups || []).length,
        findings: fs, groups_list: rep.finding_groups || [] };
    } catch (e) { counts = null; }
    XP.done = { key, text, counts, board: S.name, fieldN: Object.keys(inputs.field).length, plan: plan.length };
    return text;
  })();
  XP.run = { p, cancel };
  try { return await p; } finally { ws.forEach((w) => w.terminate()); XP.run = null; }
}

// ---- the menu: each entry says in one line what it exports, in which format and how much – counted
// from the state shown (the report's findings once it has run for this state)
const xpPlural = (n, w) => `${n} ${w}${n === 1 ? "" : /s$/.test(w) ? "es" : "s"}`;
// the report's latest result for the state shown, else null
function xpLast() {
  if (!XP.done || !S.engine) return null;
  try {
    const inputs = xpInputs();
    return XP.done.key === JSON.stringify([S.engine.stateGen, S.field.size, inputs.opts, inputs.stackup]) ? XP.done : null;
  } catch (e) { return null; }
}
// what the report checks, counted from the views: net classes with a target, lanes, rails
function xpScope() {
  const cls = S.an ? S.an.classes.filter((c) => c.target && c.nets > 0).length : 0;
  const lanes = S.buses ? allLanes().filter(laneChecked).length : 0;
  const rails = S.piSetups ? S.piSetups.size : 0;
  return [cls && xpPlural(cls, "net class"), lanes && xpPlural(lanes, "lane"), rails && xpPlural(rails, "rail")].filter(Boolean).join(", ");
}
function xpSync() {
  const on = !!S.scene, t = on ? xpTables() : { why: "load a board first" };
  $("xp-btn").disabled = !on;
  $("xp-csv").disabled = !t.tables && !t.compute;
  if (t.tables) {
    const rows = t.tables.reduce((a, x) => a + x.rows.length, 0), cols = Math.max(...t.tables.map((x) => xpColumns(x).length));
    $("xp-csv-d").textContent = `the ${t.key.replace(/-/g, " ")} table${t.tables.length > 1 ? `s (${t.tables.length})` : ""} as shown: ${xpPlural(rows, "row")}, ${xpPlural(cols, "column")}`;
  } else if (t.compute) $("xp-csv-d").textContent = `the ${t.key.replace(/-/g, " ")} table – computes ${t.what} first (progress in the top bar)`;
  else $("xp-csv-d").textContent = `no table here: ${t.why}`;
  $("xp-csv").title = t.tables ? `the table${t.tables.length > 1 ? "s" : ""} of this view as shown (filter and sort), units in the headers, as ${xpFile(t.key, "csv")}`
    : t.compute ? `computes ${t.what}, then saves the table as ${xpFile(t.key, "csv")}` : `no table to export: ${t.why}`;
  $("xp-json").disabled = $("xp-html").disabled = !on || !!XP.run;
  const last = on ? xpLast() : null, c = last && last.counts;
  const found = c ? `${xpPlural(c.n, "finding")} (${c.fail} fail, ${xpPlural(c.warn, "warning")}, ${xpPlural(c.gap, "gap")})${c.groups ? ` on ${xpPlural(c.groups, "net")}, parts or gaps` : ""}` : "";
  const scope = on ? xpScope() : "";
  $("xp-html-d").textContent = !on ? "load a board first" : c ? `page to print or save as PDF: ${found}, each net with its place on the board and a sketch`
    : `page to print or save as PDF: every finding with its place on the board and a sketch – checks ${scope || "the board"} (computed first, about half a minute)`;
  $("xp-json-d").textContent = !on ? "load a board first" : `all results of kisi report as one file (as analysis/kisi.json)${c ? `: ${found}, ${Math.round(last.text.length / 1024)} KB` : ", each finding with its location"} – for scripts, review tools and agents`;
  $("xp-html").title = "in a new window: a summary, the findings per net with their place (layer, coordinates, part) and a board sketch, the tables per area – print it there or save it as PDF; “show in the viewer” at a finding jumps to it here";
  $("xp-json").title = `the same content as \`kisi report\` (analysis/kisi.json) for the loaded files – board, project, kisi.json, the assembly variant and stackup shown – with the field-solver results of this browser, as ${xpFile("report", "json")}. Not the board's settings: those are the Settings entry; edits kept only in this browser (targets, profiles, power settings) are not in it`;
  // the board's settings: the file Prepare PCB saves (prepSave – one path)
  const cfg = on ? xpCfgState() : { why: "load a board first" };
  $("xp-cfg").disabled = !cfg.ok;
  $("xp-cfg-d").textContent = cfg.ok ? cfg.text : cfg.why;
  $("xp-cfg").title = `${S.name || "<board>"}.kisi.json, as Prepare PCB saves it (the same save: next to the board, the KiCad plugin's folder, or a download), then loaded again`;
  xpFindings();
}
// the Settings entry: what the board's kisi.json would hold (Prepare PCB's draft – the loaded file with its
// edits), or why there is nothing to save
function xpCfgState() {
  try {
    if (S.prep.saving) return { ok: false, why: "saving …" };
    const n = prepEntries(prepW()).size, diff = prepDiff(), lc = prepLoaded();
    if (!n && !diff.n) return { ok: false, why: "nothing to save yet: no kisi.json loaded and nothing entered (Prepare PCB fills it in)" };
    const err = prepBlocked("all").length;
    if (err) return { ok: false, why: `${xpPlural(err, "error")} in the draft – fix them on the Prepare PCB page` };
    const parts = [diff.add && `${diff.add} added`, diff.chg && `${diff.chg} changed`, diff.del && `${diff.del} removed`].filter(Boolean).join(", ");
    return { ok: true, text: `${S.name}.kisi.json – the board's settings with their sources (stackup, net classes, power tree fields, variants): ${xpPlural(n, "entry").replace("entrys", "entries")}${parts ? `, ${parts} since ${lc ? "the loaded file" : "the start"}` : lc ? ", as loaded" : ""}; the file Prepare PCB saves` };
  } catch (e) { return { ok: false, why: "the Prepare PCB draft is not ready yet" }; }
}
function xpMenu(open) {
  if (open) {
    const bar = document.querySelector(".bar"), r = bar ? bar.getBoundingClientRect() : null;
    if (r) $("xp-menu").style.top = `${Math.max(8, Math.round(r.bottom + 6))}px`;
    xpSync();
  }
  $("xp-menu").hidden = !open;
  $("xp-btn").setAttribute("aria-expanded", String(open));
}
function xpStatus(html) { const s = $("xp-status"); s.hidden = !html; s.innerHTML = html || ""; }
$("xp-btn").onclick = (e) => { e.stopPropagation(); xpMenu($("xp-menu").hidden); };
$("xp-close").onclick = () => { xpMenu(false); $("xp-btn").focus(); };
document.addEventListener("click", (e) => { if (!e.target.closest("#xp-wrap")) xpMenu(false); });
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("xp-menu").hidden) { xpMenu(false); $("xp-btn").focus(); } });
$("xp-csv").onclick = async () => {
  xpMenu(false);
  let t = xpTables();
  if (!t.tables && t.compute) {
    xpStatus(`<span>Computing ${esc(t.what)} for the table …</span>`);
    try { await t.compute(); } catch (e) { xpStatus(`<span class="err">${esc(e.message)}</span>`); return; }
    xpStatus("");
    t = xpTables();
  }
  if (t.tables) saveText(xpFile(t.key, "csv"), xpCsv(t.tables), "text/csv;charset=utf-8");
  else xpStatus(`<span>No table to export: ${esc(t.why)}</span> <button class="linklike" id="xp-cancel">×</button>`), $("xp-cancel").onclick = () => xpStatus("");
};
// the board's settings: Prepare PCB's save of the whole draft
$("xp-cfg").onclick = async () => {
  xpMenu(false);
  const keep = S.prep.scope;
  S.prep.scope = "all";
  try { await prepSave(); } finally { if (S.prep.scope === "all") S.prep.scope = keep; }
  if (S.prep.saved) xpStatus(`<span>${esc(S.prep.saved)}</span> <button class="linklike" id="xp-cancel">×</button>`), $("xp-cancel").onclick = () => xpStatus("");
};
// run the report with its progress in the bar; then hand the text on
async function xpRun(then, failed, keepOpen) {
  if (!keepOpen) xpMenu(false);
  const t0 = performance.now();
  const show = (what, d, n) => xpStatus(`<span>kisi report: ${esc(what)}${n ? ` · ${Math.round(100 * d / n)} %` : ""} · ${Math.round((performance.now() - t0) / 1000)} s</span> <button class="linklike" id="xp-cancel">Cancel</button>`);
  try {
    if (keepOpen) xpFindings();
    const text = await xpReport((w, d, n) => { show(w, d, n); $("xp-cancel").onclick = () => { if (XP.run) XP.run.cancel.on = true; }; });
    xpStatus("");
    then(text);
  } catch (e) {
    if (failed) failed();
    xpStatus(e.message === "cancelled" ? "" : `<span class="err">kisi report: ${esc(e.message)}</span> <button class="linklike" id="xp-cancel">×</button>`);
    if ($("xp-cancel")) $("xp-cancel").onclick = () => xpStatus("");
  }
}
$("xp-json").onclick = () => xpRun((text) => saveText(xpFile("report", "json"), text, "application/json"));
// the window opens with the click (popup blockers allow that) and gets the page when the report is there;
// a blocked window: the page as a download
$("xp-html").onclick = () => {
  let w = null;
  try { w = window.open("", "_blank"); } catch (e) { w = null; }
  if (w) w.document.write(`<title>kisi report</title><p style="font:15px system-ui,sans-serif;margin:2em">kisi report: running – see the progress in the viewer's top bar.</p>`);
  xpRun((text) => {
    const html = S.engine.reportHtml(text, xpDate());
    if (w && !w.closed) { w.document.open(); w.document.write(html); w.document.close(); hookReportWindow(w); w.focus(); return; }
    saveText(xpFile("report", "html"), html, "text/html");
    xpStatus(`<span>The browser blocked the new window: the report was downloaded as ${esc(xpFile("report", "html"))} – open it and print.</span> <button class="linklike" id="xp-cancel">×</button>`);
    $("xp-cancel").onclick = () => xpStatus("");
  }, () => { if (w && !w.closed) w.close(); });
};
