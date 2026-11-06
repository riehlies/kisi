"use strict";
// ============================================================== stackups: compare and tolerances
// A panel over the board (impedance view): the impedance of each net class's main cross-section per
// layer under the board's stackup and up to three others – fab templates (data/stackups.json) or
// edited copies – side by side, and the band for manufacturing tolerances under one of them
// (kisi_core::stackups::study_json through kisi_stackup_study: the same numbers as `kisi stackups`
// and `kisi overview --tolerance`). Columns and tolerances are kept per board in this browser.
const STK_MAX = 3;
const stkKey = () => "kisi.stk.v1." + S.name;
function stkState() {
  if (!S.stk || S.stk.board !== S.name) {
    const d = lsGet(stkKey()) || {};
    S.stk = { board: S.name, open: false, cols: d.cols || [], tolOn: !!d.tolOn, tol: d.tol || null, tolCol: d.tolCol || 0, edit: null, res: null, err: "", n: d.n || 0 };
  }
  return S.stk;
}
function stkSave() { const s = stkState(); lsSet(stkKey(), { cols: s.cols, tolOn: s.tolOn, tol: s.tol, tolCol: s.tolCol, n: s.n }); }
function stkBoardSource() {
  const parts = [S.scene.stackup.from_file ? "board file" : "kisi's default stackup"];
  if (S.config && S.config.dielectrics) parts.push("kisi.json");
  if (Object.keys(stackOv().dielectrics || {}).length) parts.push("viewer edits");
  return parts.join(" < ");
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
  const tol = s.tolOn ? (s.tol || null) : null;
  try {
    const req = { columns: s.cols.map((c) => ({ id: c.id, label: c.label, preset: c.preset || null, edits: c.edits || null })), tol_column: Math.min(s.tolCol, s.cols.length), assign: stkAssign(), board_source: stkBoardSource() };
    if (s.tolOn) req.tolerance = tol || stkDefaults();
    s.res = S.engine.stackupStudy(req);
    if (!s.tol) s.tol = stkDefaults();
  } catch (e) { s.err = e.message; }
}
// the default tolerances (data/stackups.json, through the presets call)
function stkDefaults() {
  const d = stkPresets().tolerance_defaults;
  return { dielectric_thickness_pct: d.dielectric_thickness_pct, er_pct: d.er_pct, copper_pct: d.copper_pct, width_mm: d.width_mm, width_pct: d.width_pct ?? null };
}
function stkPresets() {
  if (!S.stkPresets || S.stkPresets.k !== S.name + "|" + S.gen) {
    let r = null;
    try { r = S.engine.stackupPresets(); } catch (e) { console.warn(e); }
    S.stkPresets = { k: S.name + "|" + S.gen, r };
  }
  return S.stkPresets.r;
}
function stkOpen(on) {
  const s = stkState();
  s.open = on;
  $("stk-panel").hidden = !on;
  if (on) { $("detail").hidden = true; stkRender(); }
}
// called after every analysis (runAnalysis): new field results, edits of the board's stackup
function stkRefresh() {
  if ($("stk-panel").hidden) return;
  if (!S.stk || S.stk.board !== S.name) { stkOpen(false); return; }
  stkRender();
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
    box.innerHTML = `<div class="stk-head"><h2>Stackups and tolerances</h2><button class="btn" id="stk-close">Close</button></div>
      <section class="card"><p class="note">This needs the impedance estimate (step 2 in the “Signals” card).</p><button class="btn small primary" id="stk-est">Estimate impedances</button></section>`;
    $("stk-est").onclick = () => estimateImpedances();
    $("stk-close").onclick = () => stkOpen(false);
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
    ${other.length ? `<optgroup label="do not fit this board">${other.map((p) => `<option disabled title="${esc(p.fit.problem)}">${esc(p.template)} – ${p.layers} layers, ${fmtSig(p.nominal_mm, 2)} mm</option>`).join("")}</optgroup>` : ""}</select>`;
  const chips = [`<span class="stk-chip board" title="${esc(res ? res.columns[0].source : stkBoardSource())}"><b>Board</b> ${esc(stkBoardSource())}</span>`]
    .concat(s.cols.map((c, i) => `<span class="stk-chip"><b>${esc(c.label)}</b>${c.edits ? " (edited)" : ""} <button class="linklike" data-stk="edit" data-i="${i}">edit</button> <button class="linklike" data-stk="del" data-i="${i}" aria-label="remove ${esc(c.label)}">×</button></span>`)).join("");
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
  const t = s.tol || stkDefaults(), est = (res && res.tolerances && res.tolerances.estimates) || [];
  const estMark = (k) => est.includes(k) ? `<em class="est" title="kisi estimate (data/stackups.json)">estimate</em>` : "";
  const colOpts = [`<option value="0">the board</option>`].concat(s.cols.map((c, i) => `<option value="${i + 1}" ${s.tolCol === i + 1 ? "selected" : ""}>${esc(c.label)}</option>`)).join("");
  const wPct = t.width_pct != null;
  const inputs = `<div class="stk-tol-in">
    <label>Dielectric thickness ± <input id="stk-tol-thk" type="number" step="1" min="0" max="50" value="${t.dielectric_thickness_pct}"> % ${estMark("dielectric_thickness_pct")}</label>
    <label>εr ± <input id="stk-tol-er" type="number" step="0.5" min="0" max="50" value="${t.er_pct}"> % ${estMark("er_pct")}</label>
    <label>Copper ± <input id="stk-tol-cu" type="number" step="1" min="0" max="50" value="${t.copper_pct}"> % ${estMark("copper_pct")}</label>
    <label>Trace width ± <input id="stk-tol-width" type="number" step="${wPct ? 1 : 0.005}" min="0" value="${wPct ? t.width_pct : t.width_mm}"> <select id="stk-tol-wu" aria-label="width tolerance unit"><option value="mm" ${wPct ? "" : "selected"}>mm</option><option value="pct" ${wPct ? "selected" : ""}>%</option></select> ${estMark("width_mm")}</label>
    <button class="btn small" id="stk-tol-reset">Defaults</button></div>`;
  if (s.tolOn && res && res.rows.length) {
    const kinds = ["thickness", "er", "copper", "width"], kname = { thickness: "thickness", er: "εr", copper: "copper", width: "width" };
    const body = res.rows.filter((r) => r.tolerance).map((r) => {
      const b = r.tolerance, st = b.band_status;
      const cls = st === "inside" ? "in" : st === "worst_case_leaves" ? "near" : "out";
      return `<tr><td>${esc(r.class)}<span class="sub">${esc(r.layer_name)} · ${esc(stkLine(r))}</span></td>
        <td class="num">${fmt(r.window_ohm[0], 1)}–${fmt(r.window_ohm[1], 1)} Ω<span class="sub">±${fmt(r.window_pct, 0)} % · ${esc(r.window_source.startsWith("profile") ? r.window_source : "default")}</span></td>
        <td class="num">${fmt(b.nominal_ohm, 1)} Ω<span class="sub">${esc(b.level)}</span></td>
        ${kinds.map((k) => `<td class="num" title="${esc(b.sensitivities.filter((x) => x.kind === k).map((x) => `${x.what}: ${stkSign(x.dz_ohm, 2)} Ω`).join("\n"))}">${b.by_kind_ohm[k] != null ? "±" + fmt(b.by_kind_ohm[k], 2) : "–"}</td>`).join("")}
        <td class="num">${fmt(b.rss.min_ohm, 1)}–${fmt(b.rss.max_ohm, 1)}<span class="sub">±${fmt(b.rss.plus_minus_ohm, 1)} Ω</span></td>
        <td class="num">${fmt(b.worst_case.min_ohm, 1)}–${fmt(b.worst_case.max_ohm, 1)}<span class="sub">±${fmt(b.worst_case.plus_minus_ohm, 1)} Ω</span></td>
        <td class="band">${stkBandSvg(r, b)}<span class="hint ${cls}">${esc(b.hint)}</span></td></tr>`;
    }).join("");
    tolHtml = `<div class="stk-scroll"><table class="stk-tol"><thead><tr><th>Class · layer</th><th class="num">Window</th><th class="num">Nominal</th>${kinds.map((k) => `<th class="num" title="ΔZ for the ${kname[k]} tolerance, Ω (each layer's share summed)">Δ ${kname[k]}</th>`).join("")}<th class="num">RSS Ω</th><th class="num">Worst case Ω</th><th>Band <span class="sub">window · RSS bar · worst case line · nominal dot</span></th></tr></thead><tbody>${body}</tbody></table></div>
      <p class="note">${esc(res.method || "")} The hint is information: the channel check's verdict does not use the band.</p>`;
  }
  box.innerHTML = `<div class="stk-head"><h2>Stackups and tolerances</h2><span class="note">What a different prepreg or core does to each net class, and how far manufacturing tolerances move it.</span><button class="btn" id="stk-close">Close</button></div>
    <section class="card stk-card" aria-labelledby="stk-h1"><h3 id="stk-h1">Impedance per stackup</h3>
      <div class="stk-chips">${chips}${add}</div>${table}
      <p class="note">One row per net class and layer with at least 5 % of the class's length: its most used cross-section (pads left out; a coupled one for a differential target), the same references and neighbours under every stackup. Fab templates: kisi's library (data/stackups.json) – thicknesses and εr as the fab publishes them, tanδ assumed. The board column follows the Stackup card's values; “Prepare PCB → Use a fab stackup” makes a template the board's.</p></section>
    <section class="card stk-card" aria-labelledby="stk-h2"><h3 id="stk-h2">Manufacturing tolerances <label class="stk-on"><input type="checkbox" id="stk-tol-on" ${s.tolOn ? "checked" : ""}> show the band</label> <label class="stk-on">for <select id="stk-tol-col">${colOpts}</select></label></h3>
      ${inputs}${tolHtml || (s.tolOn ? "" : `<p class="note">The band for dielectric thickness, εr, copper thickness and trace width, one at a time (central differences), combined as worst case and as root sum of squares.</p>`)}</section>`;
  stkWire();
}
function stkWire() {
  const s = stkState();
  $("stk-close").onclick = () => stkOpen(false);
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
    if (a === "del") { s.cols.splice(i, 1); if (s.tolCol > s.cols.length) s.tolCol = 0; if (s.editing != null && s.editing >= s.cols.length) s.editing = null; }
    else if (a === "edit") s.editing = s.editing === i ? null : i;
    else if (a === "edit-done") s.editing = null;
    else if (a === "edit-reset") s.cols[i].edits = null;
    stkSave(); stkRender();
  });
  const edit = (i, fn) => {
    const c = s.cols[s.editing];
    if (!c) return;
    c.edits = c.edits || { dielectrics: [], copper: [] };
    fn(c.edits);
    stkSave(); stkRender();
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
  const tolIn = () => {
    const n = (id) => Math.max(0, parseFloat($(id).value) || 0), pct = $("stk-tol-wu").value === "pct";
    const cap = (v, hi) => Math.min(v, hi);
    s.tol = { dielectric_thickness_pct: cap(n("stk-tol-thk"), 50), er_pct: cap(n("stk-tol-er"), 50), copper_pct: cap(n("stk-tol-cu"), 50), width_mm: pct ? stkDefaults().width_mm : cap(n("stk-tol-width"), 0.5), width_pct: pct ? cap(n("stk-tol-width"), 50) : null };
    stkSave(); stkRender();
  };
  for (const id of ["stk-tol-thk", "stk-tol-er", "stk-tol-cu", "stk-tol-width"]) $(id).onchange = tolIn;
  $("stk-tol-wu").onchange = () => {
    const pct = $("stk-tol-wu").value === "pct";
    $("stk-tol-width").value = pct ? 20 : stkDefaults().width_mm;
    tolIn();
  };
  $("stk-tol-reset").onclick = () => { s.tol = stkDefaults(); stkSave(); stkRender(); };
}
