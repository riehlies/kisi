"use strict";
// ============================================================== stackups: compare and tolerances
// The Stackup view (with the option "fab stackups & tolerances" on): the impedance of each net class's
// main cross-section per layer under the board's stackup and up to three others – fab templates
// (data/stackups.json) or edited copies – side by side, and the band for manufacturing tolerances under
// one of them (kisi_core::stackups::study_json through kisi_stackup_study: the same numbers as `kisi
// stackups` and `kisi overview --tolerance`). One column is the stackup all views use ("Use in all
// views": the board's by default; a template or copy is a what-if – kisi_core::stackups::active_stackup
// in the engine, as the CLI's --stackup-preset). Columns, the active one and the tolerances are kept per
// board in this browser.
const STK_MAX = 3;
const stkKey = () => "kisi.stk.v1." + S.name;
function stkState() {
  if (!S.stk || S.stk.board !== S.name) {
    const d = lsGet(stkKey()) || {};
    S.stk = { board: S.name, open: false, cols: d.cols || [], tolOn: d.tolOn ?? true, tolCol: d.tolCol || 0, edit: null, res: null, err: "", n: d.n || 0 };
  }
  return S.stk;
}
function stkSave() { const s = stkState(); lsSet(stkKey(), { cols: s.cols, tolOn: s.tolOn, tolCol: s.tolCol, n: s.n }); }
function stkBoardSource() {
  const parts = [S.scene.stackup.from_file ? "file" : "kisi's default stackup"];
  if (S.config && S.config.dielectrics) parts.push("kisi.json");
  if (Object.keys(stackOv().dielectrics || {}).length) parts.push("viewer edits");
  return parts.join(" + ");
}
// the class → profile choices of the channel view (they give a class's target window)
function stkAssign() {
  const m = { ...((S.config && S.config.profiles) || {}) };
  try { Object.assign(m, classAssign()); } catch (e) { /* no channel view state yet */ }
  return m;
}
function stkCompute() {
  const s = stkState();
  s.err = ""; s.res = null;
  if (!S.full) return;
  try {
    const req = { columns: s.cols.map((c) => ({ id: c.id, label: c.label, preset: c.preset || null, edits: c.edits || null })), tol_column: Math.min(s.tolCol, s.cols.length), assign: stkAssign(), board_source: stkBoardSource() };
    if (s.tolOn) req.tolerance = tolSet();
    s.res = S.engine.stackupStudy(req);
  } catch (e) { s.err = e.message; }
}
function stkPresets() {
  if (!S.stkPresets || S.stkPresets.k !== S.name + "|" + S.gen) {
    let r = null;
    try { r = S.engine.stackupPresets(); } catch (e) { console.warn(e); }
    S.stkPresets = { k: S.name + "|" + S.gen, r };
  }
  return S.stkPresets.r;
}
// Esc in the Stackup view: back to the view before it
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && S.tab === "stk" && !/^(INPUT|SELECT|TEXTAREA)$/.test((e.target && e.target.tagName) || "")) setTab(S.prevTab && S.prevTab !== "stk" ? S.prevTab : "si");
});
// called after every analysis (runAnalysis): new field results, edits of the board's stackup
function stkRefresh() {
  if (S.tab === "stk") stkRender();
}

// ---- the active stackup: the one all views use ("Use in all views"), the board's by default
const stkActKey = () => "kisi.stkact.v1." + S.name;
function stkActiveId() {
  if (!S.name || !fabOn()) return "board";
  const id = lsGet(stkActKey());
  return id && stkState().cols.some((c) => c.id === id) ? id : "board";
}
function stkActiveCol() { const id = stkActiveId(); return id === "board" ? null : stkState().cols.find((c) => c.id === id) || null; }
// for the engine (anParams): {preset, edits} over the board's stackup, null for the board's
function activeStackup() { const c = stkActiveCol(); return c ? { preset: c.preset || null, edits: c.edits || null } : null; }
function stkActiveLabel() { const c = stkActiveCol(); return c ? c.label + (c.edits ? " (edited)" : "") : null; }
function setActiveStackup(id) {
  if (id === stkActiveId()) return;
  vcSave();
  lsSet(stkActKey(), id === "board" ? null : id);
  stackupSwitched();
}
// ---- the other views' results per stackup: kept when the views leave a stackup, put back when they
// return to it and nothing else changed (the keys hold what each result depends on besides the
// board; a new board, kisi.json or variant starts afresh). The engine keeps its DC results alongside
// (put_stackup); a DC result is put back only when the engine holds the same rails.
// stackups besides the current one: the channel check and the PDN impedance are small, the DC maps and
// temperature planes about 100 MB per stackup on ThunderScope (and as much again in the engine,
// PI_STASH_MAX there)
const VIEW_CACHE_MAX = { check: 3, pdn: 3, dc: 2, therm: 2 };
function vcache() {
  const g = `${S.gen}|${S.engine.cacheGen || 0}`;
  if (!S.viewCache || S.viewCache.g !== g) S.viewCache = { g, check: new Map(), dc: new Map(), pdn: new Map(), therm: new Map() };
  return S.viewCache;
}
function vcPut(kind, k, v) { const m = vcache()[kind]; m.delete(k); m.set(k, v); while (m.size > VIEW_CACHE_MAX[kind]) m.delete(m.keys().next().value); }
const vcStk = () => JSON.stringify({ a: activeStackup(), s: stackParam() });
const vcDcKey = () => vcStk() + JSON.stringify({ t: S.piTree || null, s: S.piTree ? null : [...S.piSetups.values()], p: PI_PARAMS });
const vcThermKey = () => vcDcKey() + JSON.stringify(thermParams());
// the check does not use the tolerance band: its key leaves it out
const vcCheckKey = () => { const p = anParams(); return JSON.stringify({ g: S.gen, e: S.engine.cacheGen || 0, a: p.active_stackup, s: p.stackup, t: p.targets, st: p.step, m: p.solder_mask, f: Object.keys(p.field).length, i: checkItems(), rf: rfParams() }); };
// before the stackup changes: keep what is computed for the one the views leave
function vcSave() {
  if (!S.scene || !S.full) return;
  try {
    if (S.check && !S.check.stale && S.check.gen === S.anGen && S.laneRes) vcPut("check", vcCheckKey(), { check: S.check, laneRes: S.laneRes, chStatus: S.chStatus });
    if (S.piSolved) vcPut("dc", vcDcKey(), { piRes: new Map(S.piRes), piGround: new Map(S.piGround || []), groundWarn: S.groundWarn, loss: S.loss, piMs: S.piMs });
    if (S.pdnSolved) vcPut("pdn", vcDcKey(), { pdnRes: new Map(S.pdnRes), pdnMs: S.pdnMs });
    if (S.therm && !S.therm.error) vcPut("therm", vcThermKey(), { therm: S.therm, thermLayer: S.thermLayer });
  } catch (e) { console.warn(e); }
}
// the bytes the kept results hold (typed arrays of the DC maps, losses and temperature planes)
function vcBytes() {
  const c = S.viewCache; let n = 0;
  if (!c) return 0;
  for (const e of c.dc.values()) { for (const r of e.piRes.values()) n += (r.v ? r.v.byteLength : 0) + (r.j ? r.j.byteLength : 0); if (e.loss && e.loss.buf) n += e.loss.buf.byteLength; }
  for (const e of c.therm.values()) if (e.therm.t) n += e.therm.t.byteLength;
  return n;
}
// the stackup of all views changed: the impedance at once (cached per stackup), then what the other
// views had computed – the channel check, the IR drop, the PDN impedance, the temperature – from the
// cache where it still applies, else again
async function stackupSwitched() {
  if (S.switching) { S.switchAgain = true; return; }
  S.switching = true;
  const t0 = performance.now(), ms = {}, from = {};
  try {
    do {
      S.switchAgain = false;
      stkIndicator("impedance"); await frame();
      let t = performance.now(); runAnalysis(); ms.impedance = Math.round(performance.now() - t);
      const c = vcache(), therm = !!S.therm;
      if (S.laneRes && !variantNeeded()) {
        t = performance.now();
        const hit = c.check.get(vcCheckKey());
        if (hit) {
          S.check = { ...hit.check, gen: S.anGen, stale: false }; S.laneRes = hit.laneRes; S.chStatus = hit.chStatus; from.check = "cache";
          if (S.tab === "ch") { if (S.chMode === "o") renderChOverview(); else { buildPaths(); if (S.selNet >= 0) selectNet(S.selNet, false); renderLegend(); draw(); } }
          renderCheck();
        } else { stkIndicator("channel check"); await runCheck(); from.check = "computed"; }
        ms.check = Math.round(performance.now() - t);
      }
      let dcHit = null;
      if (S.piSolved) {
        t = performance.now();
        const hit = c.dc.get(vcDcKey());
        let held = null;
        try { held = S.engine.piStored(); } catch (e) { held = null; }
        const rails = hit ? [...hit.piRes].filter(([, r]) => r.res).map(([n]) => n).sort() : [];
        if (hit && held && JSON.stringify(rails) === JSON.stringify([...held.rails].sort())) {
          S.piRes = new Map(hit.piRes); S.piGround = new Map(hit.piGround); S.groundWarn = hit.groundWarn; S.loss = hit.loss; S.piMs = hit.piMs;
          S.piImg = null; S.lossImg = null; dcHit = held; from.dc = "cache";
          thermReset();
        } else { stkIndicator("IR drop"); await solveAllRails(); from.dc = "computed"; }
        ms.dc = Math.round(performance.now() - t);
      }
      if (S.pdnSolved) {
        t = performance.now();
        const hit = c.pdn.get(vcDcKey());
        if (hit) { S.pdnRes = new Map(hit.pdnRes); S.pdnMs = hit.pdnMs; from.pdn = "cache"; }
        else { stkIndicator("PDN impedance"); await solveAllPdn(); from.pdn = "computed"; }
        ms.pdn = Math.round(performance.now() - t);
      }
      if (therm && S.piSolved) {
        t = performance.now();
        const hit = c.therm.get(vcThermKey());
        if (hit && dcHit && dcHit.thermal) { S.therm = hit.therm; S.thermLayer = hit.thermLayer; S.thermImg = null; S.thermMark = null; from.thermal = "cache"; }
        else { stkIndicator("temperature"); await solveThermal(); from.thermal = "computed"; }
        ms.thermal = Math.round(performance.now() - t);
      }
      if (from.dc === "cache" || from.pdn === "cache" || from.thermal === "cache") renderPi();
    } while (S.switchAgain);
  } finally {
    S.switching = false;
    ms.total = Math.round(performance.now() - t0);
    S.switchMs = ms; S.switchFrom = from;
    const vals = Object.values(from);
    S.switchNote = vals.length ? { text: vals.every((v) => v === "cache") ? "results from cache" : vals.some((v) => v === "cache") ? "partly from cache" : "recomputed", title: Object.entries(from).map(([k, v]) => `${k}: ${v === "cache" ? "from cache" : "recomputed"}`).join(", "), until: performance.now() + 6000 } : null;
    if (S.switchNote) setTimeout(() => stkIndicator(), 6100);
    stkIndicator(null);
    renderSide();
    if (S.tab === "stk") stkRender();
    if (typeof exportHint === "function") exportHint();
  }
}
// the header indicator: a what-if stackup in every view (its name opens the Stackup view, × goes back
// to the board's), the progress while the views follow a switch and, for a moment after it, whether
// the results came from the cache. Short, so the header stays on one row on a desktop.
function stkIndicator(step) {
  const el = $("stk-act");
  if (step !== undefined) S.switchStep = step;
  const lab = S.scene ? stkActiveLabel() : null, busy = S.switchStep;
  const note = S.switchNote && performance.now() < S.switchNote.until ? S.switchNote : null;
  el.hidden = !lab && !busy && !note;
  el.classList.toggle("busy", !!busy);
  if (el.hidden) { el.innerHTML = ""; return; }
  el.title = lab ? `Stackup of all views: ${lab} (what-if) – the board file and kisi.json stay as they are` : "";
  el.innerHTML = (lab ? `<button class="linklike" id="stk-act-open" title="Stackup: ${esc(lab)} (what-if) – open the Stackup view"><b>${esc(lab)}</b></button><span>· what-if</span><button class="linklike stk-act-x" id="stk-act-back" title="Back to board stackup" aria-label="Back to board stackup">×</button>` : "<span>Stackup: the board's</span>")
    + (busy ? `<span class="stk-act-prog">· updating ${esc(busy)}…</span>` : note ? `<span class="stk-act-prog" title="${esc(note.title)}">· ${esc(note.text)}</span>` : "");
  if ($("stk-act-open")) $("stk-act-open").onclick = () => setTab("stk");
  if ($("stk-act-back")) $("stk-act-back").onclick = () => setActiveStackup("board");
}

const stkDevClass = (dev, win) => dev == null ? "" : Math.abs(dev) <= win ? "in" : Math.abs(dev) <= 2 * win ? "near" : "out";
const stkSign = (v, d = 1) => (v > 0 ? "+" : v < 0 ? "−" : "±") + fmt(Math.abs(v), d);
function stkLine(r) {
  const k = r.kind === "Stripline" ? "stripline" : r.kind === "EmbeddedMicrostrip" ? "inner, one plane" : "microstrip";
  return `${k}${r.coplanar ? " + coplanar" : ""} · w ${fmt(r.w_mm * 1000, 0)} µm${r.gap_mm != null ? ` · gap ${fmt(r.gap_mm * 1000, 0)} µm` : ""}`;
}
// the band against the target window as a small bar: window (green), worst case (line), RSS (bar), nominal (tick)
function stkBandSvg(r, t) {
  const W = 168, H = 22, z0 = r.target_ohm, span = Math.max(2 * r.window_pct / 100 * z0, 1.15 * Math.max(Math.abs(t.worst_case.max_ohm - z0), Math.abs(t.worst_case.min_ohm - z0)));
  const x = (z) => Math.max(1, Math.min(W - 1, W / 2 + (z - z0) / span * (W / 2 - 4)));
  const [lo, hi] = r.window_ohm, wc = t.worst_case, rs = t.rss;
  return `<svg class="stk-bar" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="window ${fmt(lo, 1)}–${fmt(hi, 1)} Ω, nominal ${fmt(t.nominal_ohm, 1)} Ω, worst case ${fmt(wc.min_ohm, 1)}–${fmt(wc.max_ohm, 1)} Ω, RSS ${fmt(rs.min_ohm, 1)}–${fmt(rs.max_ohm, 1)} Ω">
    <rect class="win" x="${x(lo)}" y="3" width="${Math.max(1, x(hi) - x(lo))}" height="${H - 6}" rx="2"></rect>
    <line class="tgt" x1="${x(z0)}" x2="${x(z0)}" y1="1" y2="${H - 1}"></line>
    <line class="wc" x1="${x(wc.min_ohm)}" x2="${x(wc.max_ohm)}" y1="${H / 2}" y2="${H / 2}"></line>
    <line class="wc" x1="${x(wc.min_ohm)}" x2="${x(wc.min_ohm)}" y1="${H / 2 - 4}" y2="${H / 2 + 4}"></line>
    <line class="wc" x1="${x(wc.max_ohm)}" x2="${x(wc.max_ohm)}" y1="${H / 2 - 4}" y2="${H / 2 + 4}"></line>
    <rect class="rss ${rs.in_window ? "in" : "out"}" x="${x(rs.min_ohm)}" y="${H / 2 - 3}" width="${Math.max(1, x(rs.max_ohm) - x(rs.min_ohm))}" height="6" rx="1"></rect>
    <circle class="nom" cx="${x(t.nominal_ohm)}" cy="${H / 2}" r="3"></circle></svg>`;
}
function stkColumnEditor(ci) {
  const s = stkState(), res = s.res, col = res && res.columns[ci + 1], c = s.cols[ci];
  if (!col || !c) return "";
  const L = S.scene.stackup.copper;
  const rows = col.dielectrics.map((d, i) => `<tr><td>${esc(L[i].name)} / ${esc(L[i + 1].name)}<span class="sub">${esc(d.kind)} · ${esc(d.material)}</span></td>
      <td><input type="number" step="1" min="5" data-stk-edit="${i}" data-k="thickness" value="${Math.round(d.thickness_mm * 1e4) / 10}" aria-label="thickness µm, dielectric ${i}"></td>
      <td><input type="number" step="0.01" min="1" data-stk-edit="${i}" data-k="epsilon_r" value="${+d.er.toFixed(4)}" aria-label="εr, dielectric ${i}"></td></tr>`).join("");
  const cu = col.copper_um.map((t, i) => `<label>${esc(L[i].name)} <input type="number" step="0.1" min="2" data-stk-cu="${i}" value="${t}" aria-label="copper µm ${esc(L[i].name)}"></label>`).join("");
  return `<div class="stk-editor"><b>Edit “${esc(c.label)}”</b> <span class="note">${c.preset ? "the template with your changes" : "a copy of the board's stackup"}; values as used</span>
    <table class="stk-ed"><thead><tr><th>Dielectric</th><th>Thickness µm</th><th>εr</th></tr></thead><tbody>${rows}</tbody></table>
    <div class="stk-cu"><span class="note">Copper µm</span>${cu}</div>
    <div class="prep-row"><button class="btn small" data-stk="edit-reset" data-i="${ci}">Undo the changes</button><button class="btn small" data-stk="edit-done">Done</button></div></div>`;
}
function stkRender() {
  const s = stkState(), box = $("stk-panel");
  if (!S.full) {
    box.innerHTML = `<div class="stk-head"><h2>Stackups and tolerances</h2><label class="prep-chk" for="stk-fab-on" title="${esc(FAB_TITLE)}"><input type="checkbox" id="stk-fab-on" checked> Fab stackups &amp; tolerances</label><span class="note">The stackup all views use, what a different prepreg or core does to each net class, and how far manufacturing tolerances move it.</span></div>
      <section class="card"><p class="note">This needs the impedance estimate (step 2 in the “Signals” card).</p><button class="btn small primary" id="stk-est">Estimate impedances</button></section>`;
    $("stk-est").onclick = () => estimateImpedances();
    $("stk-fab-on").onchange = () => setFabOn($("stk-fab-on").checked);
    return;
  }
  stkCompute();
  const res = s.res, pr = stkPresets();
  const used = new Set(s.cols.map((c) => c.preset).filter(Boolean));
  const fits = pr ? pr.presets.filter((p) => p.fit && p.fit.ok) : [];
  const other = pr ? pr.presets.filter((p) => !(p.fit && p.fit.ok)) : [];
  const full = s.cols.length >= STK_MAX;
  const add = `<select id="stk-add" aria-label="add a stackup to compare" ${full ? "disabled" : ""}><option value="">${full ? `at most ${STK_MAX} to compare` : "+ add a stackup…"}</option>
    ${fits.length ? `<optgroup label="fab templates for this board">${fits.map((p) => `<option value="preset:${esc(p.id)}" ${used.has(p.id) ? "disabled" : ""}>${esc(p.template)} – ${esc(p.fab)}, ${p.layers} layers, ${fmtSig(p.nominal_mm, 2)} mm</option>`).join("")}</optgroup>` : ""}
    <option value="copy">a copy of the board's stackup, to edit</option>
    ${other.length ? `<optgroup label="do not fit this board">${other.map((p) => `<option disabled title="${esc(p.fit.problem)}">${esc(p.template)} – ${p.layers} layers, ${fmtSig(p.nominal_mm, 2)} mm: ${p.layers !== p.fit.board_layers ? `the board has ${p.fit.board_layers} copper layers` : "another board thickness"}</option>`).join("")}</optgroup>` : ""}</select>`;
  // one column is the stackup of all views ("Use in all views"); the board's by default
  const act = stkActiveId();
  const useBtn = (id, label) => id === act
    ? `<span class="stk-in-use" title="all views use this stackup">● in all views</span>`
    : `<button class="btn small" data-stk-act="${esc(id)}" title="all views – impedance, channel, power, exports – use ${esc(label)}${id === "board" ? "" : " (what-if; the board file and kisi.json stay as they are)"}">Use in all views</button>`;
  const chips = [`<span class="stk-chip board${act === "board" ? " active" : ""}" title="${esc(res ? res.columns[0].source : stkBoardSource())}"><b>Board</b> (${esc(stkBoardSource())}) ${useBtn("board", "the board's stackup")}</span>`]
    .concat(s.cols.map((c, i) => `<span class="stk-chip${act === c.id ? " active" : ""}"><b>${esc(c.label)}</b>${c.edits ? " (edited)" : ""} ${useBtn(c.id, c.label)} ${c.preset ? `<button class="linklike" data-stk="use" data-i="${i}" title="write this template into the board's kisi.json draft (Prepare PCB), with its sources">use for the board</button> ` : ""}<button class="linklike" data-stk="edit" data-i="${i}">edit</button> <button class="linklike" data-stk="del" data-i="${i}" aria-label="remove ${esc(c.label)}">×</button></span>`)).join("");
  let table = "", tolHtml = "";
  if (s.err) table = `<p class="err">${esc(s.err)}</p>`;
  else if (res && !res.rows.length) table = `<p class="note">No net class with a target impedance has routed length here: give the classes a target (the “Target impedance per net class” card or kisi.json).</p>`;
  else if (res) {
    const cols = res.columns;
    const head = `<tr><th>Class</th><th>Layer · line</th><th class="num">Target</th>${cols.map((c, i) => `<th class="stk-col num" title="${esc(c.source)}">${i === 0 ? "Board" : esc(c.label)}<span class="sub">${fmt(c.thickness_mm, 2)} mm</span></th>`).join("")}</tr>`;
    const body = res.rows.map((r) => `<tr><td>${esc(r.class)}<span class="sub">${Math.round(r.share_of_class * 100)} % of its length on the layer</span></td>
      <td>${esc(r.layer_name)}<span class="sub">${esc(stkLine(r))}</span></td>
      <td class="num">${fmt(r.target_ohm, 0)} Ω${r.diff ? " diff" : ""}<span class="sub">±${fmt(r.window_pct, 0)} %</span></td>
      ${r.cells.map((c, i) => c.z_ohm == null ? `<td class="z num">–</td>` : `<td class="z num ${stkDevClass(c.dev_pct, r.window_pct)}" title="${esc(c.level)}, typically within ±${fmt(c.uncertainty_pct, 1)} %${i > 0 ? `; ${stkSign(c.z_ohm - r.cells[0].z_ohm)} Ω against the board` : ""}">${fmt(c.z_ohm, 1)} Ω<span class="sub">${stkSign(c.dev_pct)} %${c.level === "formula" ? " · formula" : " · field"}</span></td>`).join("")}</tr>`).join("");
    const nf = (res.field_needed || []).length, pool = S.pool;
    const busy = pool && (pool.running || S.fieldPending.size > 0);
    const fsBtn = nf ? `<button class="btn small" id="stk-fs" ${busy || (pool && pool.failed) ? "disabled" : ""}>Field solver for ${nf} cross-section${nf > 1 ? "s" : ""}</button>` : `<span class="note">Every value here from the field solver.</span>`;
    table = `<div class="stk-scroll"><table class="stk-cmp"><thead>${head}</thead><tbody>${body}</tbody></table></div>
      <div class="prep-row">${fsBtn}<span class="note">${busy ? `Field solver running: ${pool.done_n} of ${pool.total}.` : "Cells say formula or field solver (the accuracy levels of the Signals card; hover for the typical uncertainty). Green: within the class's window, amber: within twice of it, red: beyond."}</span></div>`;
    if (s.editing != null && s.cols[s.editing]) table += stkColumnEditor(s.editing);
  }
  // tolerance band
  const t = tolEff();
  const estMark = (k) => { const f = t.from[k]; return `<em class="est" title="${esc(tolFromText(k))}">${f === "kisi.json" ? "kisi.json" : f === "viewer" ? "edited" : "default, estimate"}</em>`; };
  const colOpts = [`<option value="0">the board</option>`].concat(s.cols.map((c, i) => `<option value="${i + 1}" ${s.tolCol === i + 1 ? "selected" : ""}>${esc(c.label)}</option>`)).join("");
  const wPct = t.width_pct != null;
  const inputs = `<div class="stk-tol-in">
    <label>Dielectric thickness ± <input id="stk-tol-thk" type="number" step="1" min="0" max="50" value="${t.dielectric_thickness_pct}"> % ${estMark("dielectric_thickness_pct")}</label>
    <label>εr ± <input id="stk-tol-er" type="number" step="0.5" min="0" max="50" value="${t.er_pct}"> % ${estMark("er_pct")}</label>
    <label>Copper ± <input id="stk-tol-cu" type="number" step="1" min="0" max="50" value="${t.copper_pct}"> % ${estMark("copper_pct")}</label>
    <label>Trace width ± <input id="stk-tol-width" type="number" step="${wPct ? 1 : 0.005}" min="0" value="${wPct ? t.width_pct : t.width_mm}"> <select id="stk-tol-wu" aria-label="width tolerance unit"><option value="mm" ${wPct ? "" : "selected"}>mm</option><option value="pct" ${wPct ? "selected" : ""}>%</option></select> ${estMark("width")}</label>
    <button class="btn small" id="stk-tol-reset" ${Object.keys(tolEdits()).length ? "" : "disabled"} title="drop the edits made here: the kisi.json values or kisi's defaults">Undo edits</button></div>
    <p class="note">These tolerances apply everywhere – the net detail, the channel view, the board map's “Tolerance” mode, the exports. Edits here stay in this browser; Prepare PCB moves them into the board's kisi.json (<code>tolerances</code>). Channel exports use the stackup active in all views and name it with these tolerances in their header; the tolerance corners are a choice in the export settings.</p>`;
  if (s.tolOn && res && res.rows.length) {
    const kinds = ["thickness", "er", "copper", "width"], kname = { thickness: "thickness", er: "εr", copper: "copper", width: "width" };
    const body = res.rows.filter((r) => r.tolerance).map((r) => {
      const b = r.tolerance, st = b.band_status;
      const cls = st === "inside" ? "in" : st === "band_leaves" ? "near" : "out", mark = st === "inside" ? "✓" : st === "band_leaves" ? "!" : "✕";
      return `<tr><td>${esc(r.class)}<span class="sub">${esc(r.layer_name)} · ${esc(stkLine(r))}</span></td>
        <td class="num">${fmt(r.window_ohm[0], 1)}–${fmt(r.window_ohm[1], 1)} Ω<span class="sub">±${fmt(r.window_pct, 0)} % · ${esc(r.window_source.startsWith("profile") ? r.window_source : "default")}</span></td>
        <td class="num">${fmt(b.nominal_ohm, 1)} Ω<span class="sub">${esc(b.level)}</span></td>
        ${kinds.map((k) => `<td class="num" title="${esc(b.sensitivities.filter((x) => x.kind === k).map((x) => `${x.what}: ${stkSign(x.dz_ohm, 2)} Ω`).join("\n"))}">${b.by_kind_ohm[k] != null ? "±" + fmt(b.by_kind_ohm[k], 2) : "–"}</td>`).join("")}
        <td class="num">${fmt(b.rss.min_ohm, 1)}–${fmt(b.rss.max_ohm, 1)}<span class="sub">±${fmt(b.rss.plus_minus_ohm, 1)} Ω</span></td>
        <td class="num">${fmt(b.worst_case.min_ohm, 1)}–${fmt(b.worst_case.max_ohm, 1)}<span class="sub">±${fmt(b.worst_case.plus_minus_ohm, 1)} Ω</span></td>
        <td class="band"><span class="stk-mark ${cls}" title="${esc(b.hint)}">${mark}</span>${stkBandSvg(r, b)}</td></tr>`;
    }).join("");
    const rows = res.rows.filter((r) => r.tolerance), cnt = (k) => rows.filter((r) => r.tolerance.band_status === k).length;
    const sum = `<p class="stk-sum"><b>${rows.length} rows:</b> <span class="zb inside">${cnt("inside")} band inside the window</span> <span class="zb band_leaves">${cnt("band_leaves")} nominal inside, band leaves</span> <span class="zb nominal_outside">${cnt("nominal_outside")} nominal outside</span> <span class="note">(RSS band; marks per row: ✓ ! ✕, the bar shows window, RSS and worst case)</span></p>`;
    tolHtml = sum + `<div class="stk-scroll"><table class="stk-tol"><thead><tr><th>Class · layer</th><th class="num">Window</th><th class="num">Nominal</th>${kinds.map((k) => `<th class="num" title="ΔZ for the ${kname[k]} tolerance, Ω (each layer's share summed)">Δ ${kname[k]}</th>`).join("")}<th class="num">RSS Ω</th><th class="num">Worst case Ω</th><th>Band <span class="sub">window · RSS bar · worst case line · nominal dot</span></th></tr></thead><tbody>${body}</tbody></table></div>
      <p class="note">${esc(res.method || "")}</p>`;
  }
  box.innerHTML = `<div class="stk-head"><h2>Stackups and tolerances</h2><label class="prep-chk" for="stk-fab-on" title="${esc(FAB_TITLE)}"><input type="checkbox" id="stk-fab-on" checked> Fab stackups &amp; tolerances</label><span class="note">The stackup all views use, what a different prepreg or core does to each net class, and how far manufacturing tolerances move it.</span></div>
    <section class="card stk-card" aria-labelledby="stk-h1"><h3 id="stk-h1">Impedance per stackup</h3>
      <div class="stk-chips">${chips}${add}</div>${s.note ? `<p class="note">${esc(s.note)}</p>` : ""}${table}
      <p class="note">One row per net class and layer with at least 5 % of the class's length: its most used cross-section (pads left out; a coupled one for a differential target), the same references and neighbours under every stackup. Fab templates: kisi's library (data/stackups.json) – thicknesses and εr as the fab publishes them, tanδ assumed. The board column follows the Stackup card's values; “use for the board” on a template writes it into the Prepare PCB draft.</p></section>
    <section class="card stk-card" aria-labelledby="stk-h2"><h3 id="stk-h2">Manufacturing tolerances <label class="stk-on"><input type="checkbox" id="stk-tol-on" ${s.tolOn ? "checked" : ""}> show the band</label> <label class="stk-on">stackup <select id="stk-tol-col" title="the stackup of the band">${colOpts}</select></label></h3>
      ${inputs}${tolHtml || (s.tolOn ? "" : `<p class="note">The band for dielectric thickness, εr, copper thickness and trace width, one at a time (central differences), combined as worst case and as root sum of squares.</p>`)}</section>`;
  stkWire();
}
function stkWire() {
  const s = stkState();
  $("stk-fab-on").onchange = () => setFabOn($("stk-fab-on").checked);
  $("stk-panel").querySelectorAll("[data-stk-act]").forEach((b) => b.onclick = () => setActiveStackup(b.dataset.stkAct));
  const add = $("stk-add");
  if (add) add.onchange = () => {
    const v = add.value;
    if (!v || s.cols.length >= STK_MAX) return;
    s.n = (s.n || 0) + 1;
    if (v.startsWith("preset:")) {
      const id = v.slice(7), p = (stkPresets().presets || []).find((x) => x.id === id);
      s.cols.push({ id: "c" + s.n, label: p ? p.template : id, preset: id, edits: null });
    } else {
      let k = 1;
      while (s.cols.some((c) => c.label === `Copy ${k}`)) k++;
      s.cols.push({ id: "c" + s.n, label: `Copy ${k}`, preset: null, edits: null });
    }
    if (v === "copy") s.editing = s.cols.length - 1;
    stkSave(); stkRender();
  };
  $("stk-panel").querySelectorAll("[data-stk]").forEach((b) => b.onclick = () => {
    const i = +b.dataset.i, a = b.dataset.stk;
    if (a === "del") {
      const wasActive = stkActiveId() === s.cols[i].id;
      if (wasActive) vcSave();
      s.cols.splice(i, 1); if (s.tolCol > s.cols.length) s.tolCol = 0; if (s.editing != null && s.editing >= s.cols.length) s.editing = null;
      if (wasActive) { lsSet(stkActKey(), null); stkSave(); stackupSwitched(); return; }
    }
    else if (a === "edit") s.editing = s.editing === i ? null : i;
    else if (a === "edit-done") s.editing = null;
    else if (a === "edit-reset") { if (stkActiveId() === s.cols[i].id) vcSave(); s.cols[i].edits = null; if (stkActiveId() === s.cols[i].id) { stkSave(); stackupSwitched(); return; } }
    else if (a === "use") {
      // the template becomes the board's stackup in the Prepare PCB draft (saved there)
      const p = (stkPresets().presets || []).find((x) => x.id === s.cols[i].preset);
      if (!p || !p.kisi_json) return;
      prepW().stackup = prepClone(p.kisi_json);
      s.note = `${p.template} written into the Prepare PCB draft as the board's stackup (dielectrics, copper, solder mask, with sources; tanδ assumed) – save it there to use it.`;
      prepEdited(true);
    }
    stkSave(); stkRender();
  });
  const edit = (i, fn) => {
    const c = s.cols[s.editing];
    if (!c) return;
    if (stkActiveId() === c.id) vcSave(); // before the active copy changes: its results stay with its old values
    c.edits = c.edits || { dielectrics: [], copper: [] };
    fn(c.edits);
    stkSave(); stkRender();
    if (stkActiveId() === c.id) stackupSwitched(); // the views follow the active copy's edits
  };
  $("stk-panel").querySelectorAll("[data-stk-edit]").forEach((el) => el.onchange = () => {
    const i = +el.dataset.stkEdit, k = el.dataset.k; let v = parseFloat(el.value);
    if (!isFinite(v) || v <= 0) return;
    if (k === "thickness") v /= 1000;
    edit(i, (e) => { let d = e.dielectrics.find((x) => x.index === i); if (!d) e.dielectrics.push(d = { index: i }); d[k] = v; });
  });
  $("stk-panel").querySelectorAll("[data-stk-cu]").forEach((el) => el.onchange = () => {
    const i = +el.dataset.stkCu, v = parseFloat(el.value) / 1000;
    if (!isFinite(v) || v <= 0) return;
    edit(i, (e) => { let d = e.copper.find((x) => x.index === i); if (!d) e.copper.push(d = { index: i }); d.thickness = v; });
  });
  const fs = $("stk-fs");
  if (fs) fs.onclick = () => {
    const items = ((s.res && s.res.field_needed) || []).map(([id, text, w]) => ({ id, text, prio: 1e5 + w }));
    if (!items.length) return;
    if (S.pool && !S.pool.running && !S.pool.mainThread && S.pool.n !== threadCount()) { S.pool.terminate(); S.pool = null; }
    if (!S.pool) S.pool = new FieldPool();
    S.pool.scope = null; S.pool.round = 1;
    S.pool.start(items);
    stkRender();
  };
  $("stk-tol-on").onchange = () => { s.tolOn = $("stk-tol-on").checked; stkSave(); stkRender(); };
  $("stk-tol-col").onchange = () => { s.tolCol = +$("stk-tol-col").value; stkSave(); stkRender(); };
  // an edit keeps only the values that differ from kisi.json / the defaults (their origin stays visible)
  const tolIn = () => {
    const n = (id) => Math.max(0, parseFloat($(id).value) || 0), pct = $("stk-tol-wu").value === "pct";
    const cap = (v, hi) => Math.min(v, hi), base = tolEffBase(), e = {};
    const v = { dielectric_thickness_pct: cap(n("stk-tol-thk"), 50), er_pct: cap(n("stk-tol-er"), 50), copper_pct: cap(n("stk-tol-cu"), 50) };
    for (const k of Object.keys(v)) if (Math.abs(v[k] - base[k]) > 1e-9) e[k] = v[k];
    if (pct) { const w = cap(n("stk-tol-width"), 50); if (base.width_pct == null || Math.abs(w - base.width_pct) > 1e-9) e.width_pct = w; }
    else { const w = cap(n("stk-tol-width"), 0.5); if (base.width_pct != null || Math.abs(w - base.width_mm) > 1e-9) e.width_mm = w; }
    setTolEdits(e); runAnalysis();
  };
  for (const id of ["stk-tol-thk", "stk-tol-er", "stk-tol-cu", "stk-tol-width"]) $(id).onchange = tolIn;
  $("stk-tol-wu").onchange = () => {
    const pct = $("stk-tol-wu").value === "pct", base = tolEffBase();
    $("stk-tol-width").value = pct ? (base.width_pct ?? 20) : base.width_mm;
    tolIn();
  };
  $("stk-tol-reset").onclick = () => { setTolEdits({}); runAnalysis(); };
}
// the tolerances for an export's comment block and the viewer's Touchstone files: with the option on
function stkExportTol() { return fabOn() ? tolSet() : null; }
// the tolerances without the edits made here (kisi.json over the defaults)
function tolEffBase() { const e = S.tolEd; S.tolEd = { board: S.name, v: {} }; const t = tolEff(); S.tolEd = e; return t; }
