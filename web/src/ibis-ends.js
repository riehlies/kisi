"use strict";
// ---- IBIS models at the lane ends (interfaces overview, lane detail, channel check)
//
// Files:   .ibs files loaded in a bus card ("Load .ibs…") or dropped with the board / kisi.json; the
//          engine keeps them in memory (S.ibisFiles: name → {info} | {error}); never stored in the
//          browser, never uploaded.
// Choice:  per IC (refdes) {file, component, pin_map, corner, selectors, models (pad → model)}, stored
//          per board in localStorage – names and options only – over kisi.json "ibis".<ref>;
//          {off: true} = the profile's transmitter / receiver although kisi.json names a model.
// Engine:  kisi_ibis_ends resolves the ends for the overview; laneRf and the kisi_check items carry
//          "ibis" (the mapping of the lane's ends). Resolution and linearisation are Rust
//          (kisi_core::ibis), the same as in the CLI.

const ibisKey = () => "kisi.ibis.v1." + S.name;
function ibisChoices() {
  if (!S.ibc || S.ibc.board !== S.name) S.ibc = { board: S.name, map: lsGet(ibisKey()) || {} };
  return S.ibc.map;
}
// kisi.json "ibis" of the loaded configuration
function cfgIbisMap() {
  const t = S.lastFiles && S.lastFiles.cfg && S.lastFiles.cfg.text;
  if (!t) return {};
  if (S.cfgIbisMemo && S.cfgIbisMemo.t === t) return S.cfgIbisMemo.m;
  let m = {};
  try { m = JSON.parse(t).ibis || {}; } catch (e) { m = {}; }
  S.cfgIbisMemo = { t, m };
  return m;
}
const ibisBase = (p) => String(p || "").split(/[\\/]/).pop();
// the mapping of one IC: the viewer's choice over kisi.json; null = the profile's ends
function ibisEntryOf(ref) {
  const own = ibisChoices()[ref], cfg = cfgIbisMap()[ref];
  if (own && own.off) return null;
  if (own && own.file) return own;
  return cfg && cfg.file ? cfg : null;
}
// the mapping for the ends of a lane (the "ibis" of a request): {refdes: entry}
function ibisMapFor(l) {
  const out = {};
  for (const e of l.ends || []) {
    if (e.role !== "ic") continue;
    const m = ibisEntryOf(e.refdes);
    if (m) { const { source, ...r } = m; out[e.refdes] = r; } // the source is for kisi.json only
  }
  return out;
}
// the viewer's own entry of an IC, started from kisi.json's (to change one option)
function ibisEntryEdit(ref) {
  const m = ibisChoices();
  if (!m[ref] || m[ref].off || !m[ref].file) m[ref] = JSON.parse(JSON.stringify(cfgIbisMap()[ref] || { file: "" }));
  return m[ref];
}
function ibisSave() { lsSet(ibisKey(), ibisChoices()); S.ibisEnds = null; S.chCache = null; markCheckStale(); }

// register a model file in the engine (memory only)
async function loadIbisFile(f) {
  try { S.ibisFiles[f.name] = { info: S.engine.ibisLoad(f.name, await fileB64(f)) }; }
  catch (e) { S.ibisFiles[f.name] = { error: e.message }; }
  S.ibisEnds = null; S.chCache = null;
  return S.ibisFiles[f.name];
}

// the resolved IBIS ends of all lanes with a mapping: {laneKey: {tx, rx}} (cached)
function ibisEndsAll() {
  if (!S.buses || variantNeeded()) return {};
  const lanes = allLanes().filter((l) => CHECKABLE.has(l.kind) && l.ends.length === 2 && Object.keys(ibisMapFor(l)).length);
  if (!lanes.length) return {};
  const map = {};
  for (const l of lanes) Object.assign(map, ibisMapFor(l));
  const key = `${JSON.stringify(map)}|${S.boardKey}|${JSON.stringify(laneSwaps())}|${Object.keys(S.ibisFiles).join(",")}`;
  if (S.ibisEnds && S.ibisEnds.key === key && S.ibisEnds.buses === S.buses) return S.ibisEnds.res;
  let res = {};
  try { res = S.engine.ibisEnds({ ibis: map, lanes: lanes.map((l) => ({ key: laneKey(l), lane: l.nets[0], to: l.ends[l.ends.length - 1].refdes, reversed: laneReversed(l) })) }); }
  catch (e) { console.warn(e); }
  S.ibisEnds = { key, buses: S.buses, res };
  return res;
}

const ibisLinear = (e) => `${fmt(e.r_ohm, 1)} Ω${e.role === "receiver" && !(e.r_ohm > 0) ? " (high impedance)" : ""}, ${fmt(e.c_pf, 2)} pF, ${fmt(e.l_nh, 2)} nH${e.rise_ps ? `, edge ${fmt(e.rise_ps, 0)} ps 10–90 %` : ""}`;
const ibisTitle = (e) => e.error ? `IBIS not used: ${e.error} – the profile's ${e.role === "driver" ? "transmitter" : "receiver"} applies`
  : `${e.model} (${e.model_type || "?"}, ${e.model_from}) at IBIS pin ${e.ibis_pin} ${e.signal}: ${ibisLinear(e)} – linearised IBIS model (estimate)${(e.warnings || []).length ? " · " + e.warnings.join(" · ") : ""}`;

// per lane: the driver and receiver model (IBIS, connector or profile) with a model override
function ibisLaneCell(l, ends) {
  const r = ends[laneKey(l)] || {};
  const one = (e, role, pin) => {
    const lab = role === "tx" ? "Tx" : "Rx";
    if (!e) return `<div class="ie-row"><span class="muted">${lab} ${esc(pin ? pin.refdes : "")} ${pin && pin.role === "connector" ? "connector end" : "profile"}</span></div>`;
    const pads = [e.pad, ...(l.partner_ends || []).filter((p) => p.refdes === e.refdes).map((p) => p.pin)];
    const ovr = e.model_from === "override";
    const cands = e.candidates || [];
    const first = e.error && !ovr ? "profile" : ovr ? "from the pin" : `${e.model} (${e.selector ? "selector" : "pin"})`;
    const sel = cands.length ? `<select data-ib-ov="${esc(e.refdes)}" data-pads="${esc(pads.join(","))}" aria-label="${lab} model at ${esc(e.refdes)}" title="${esc(ibisTitle(e))}"><option value="">${esc(first)}</option>${cands.map((m) => `<option value="${esc(m)}" ${ovr && e.model === m ? "selected" : ""}>${esc(m)}</option>`).join("")}</select>`
      : `<span title="${esc(ibisTitle(e))}">${esc(e.error ? "profile" : e.model)}</span>`;
    return `<div class="ie-row"><span class="${e.error ? "err" : "muted"}" title="${esc(ibisTitle(e))}">${lab} ${esc(e.refdes)}</span> ${sel}</div>`;
  };
  return `<td class="ie">${one(r.tx, "tx", laneTx(l))}${one(r.rx, "rx", laneRx(l))}</td>`;
}
function busHasIbis(b, ends) { return b.lanes.some((l) => ends[laneKey(l)]); }

// per IC at the ends of the bus's lanes: model file, component, pins, corner, selector choices
function ibisControls(b, ends) {
  if (!CHECKABLE.has(b.kind)) return "";
  const ics = new Map();
  for (const l of b.lanes) for (const e of l.ends) if (e.role === "ic" && !ics.has(e.refdes)) ics.set(e.refdes, e.value);
  if (!ics.size) return "";
  const loadBtn = (ref) => `<label class="btn small ce-load">Load .ibs…<input type="file" accept=".ibs" data-ib-load="${esc(ref)}" hidden></label>`;
  const fileSel = (ref, cur, cfg, own) => {
    const name = cur ? ibisBase(cur.file) : "", usingCfg = !(own && (own.off || own.file)) && cfg && cfg.file;
    const names = [...new Set([...Object.keys(S.ibisFiles), ...(name && !usingCfg ? [name] : [])])];
    return `<select data-ib-file="${esc(ref)}" aria-label="IBIS model of ${esc(ref)}"><option value="" ${!cur ? "selected" : ""}>none – profile</option>${cfg && cfg.file ? `<option value="@cfg" ${usingCfg ? "selected" : ""}>kisi.json (${esc(ibisBase(cfg.file))})</option>` : ""}${names.map((n) => `<option value="${esc(n)}" ${cur && !usingCfg && n === name ? "selected" : ""}>${esc(n)}</option>`).join("")}</select>`;
  };
  const plain = [], boxes = [];
  for (const [ref, val] of ics) {
    const own = ibisChoices()[ref], cfg = cfgIbisMap()[ref], cur = ibisEntryOf(ref);
    if (!cur) { plain.push(`<span class="ib-ic">${esc(ref)} <span class="muted">${esc(val || "")}</span> ${fileSel(ref, cur, cfg, own)} ${loadBtn(ref)}</span>`); continue; }
    const name = ibisBase(cur.file), f = S.ibisFiles[name];
    const info = f && f.info, err = !f ? `${name} not loaded – load it here or drop it with the board` : f.error;
    const comps = info ? info.components : [];
    const compSel = comps.length > 1 ? `<label>component <select data-ib-comp="${esc(ref)}">${comps.map((c) => `<option value="${esc(c.name)}" ${c.name === (cur.component || "") ? "selected" : ""}>${esc(c.name)}</option>`).join("")}${!cur.component ? '<option value="" selected>– choose –</option>' : ""}</select></label>` : "";
    const pm = `<label>pins <select data-ib-pm="${esc(ref)}" title="by number: pad number = IBIS pin name; by name: the pad's symbol pin name = IBIS signal name"><option value="by-number">by number</option><option value="by-name" ${cur.pin_map === "by-name" ? "selected" : ""}>by name</option></select></label>`;
    const cn = `<label>corner <select data-ib-corner="${esc(ref)}">${["typ", "min", "max"].map((c) => `<option ${(cur.corner || "typ") === c ? "selected" : ""}>${c}</option>`).join("")}</select></label>`;
    // [Model Selector]s the IC's lane ends in this bus use
    const sels = new Map();
    let ok = 0, bad = 0, why = "";
    for (const l of b.lanes) {
      const r = ends[laneKey(l)] || {};
      for (const e of [r.tx, r.rx]) {
        if (!e || e.refdes !== ref) continue;
        if (e.selector) sels.set(e.selector, e.selector_models || []);
        if (e.error) { bad++; why = why || e.error; } else ok++;
      }
    }
    const selCtl = [...sels].map(([s, ms]) => `<label title="[Model Selector] ${esc(s)}: drive strength, ODT or I/O standard">${esc(s)} <select data-ib-sel="${esc(ref)}" data-sel="${esc(s)}"><option value="">first entry (${esc(ms[0] || "")})</option>${ms.map((m) => `<option value="${esc(m)}" ${(cur.selectors || {})[s] === m ? "selected" : ""}>${esc(m)}</option>`).join("")}</select></label>`).join("");
    const st = info ? `<span class="${bad && !ok ? "err" : "muted"}">${ok} lane end${ok === 1 ? "" : "s"} with IBIS${bad ? `, ${bad} on the profile (${esc(why)})` : ""}</span>` : "";
    const entry = info ? `<details class="ib-entry"><summary>kisi.json entry</summary><pre>${esc(ibisEntryJson(ref))}</pre><button class="btn small" data-ib-copy="${esc(ref)}">Copy</button> <span class="muted">for the "ibis" section; "file" is a path relative to the kisi.json</span></details>` : "";
    boxes.push(`<div class="ce-box ib-box"><span class="ce-h">IC <b>${esc(ref)}</b> ${esc(val || "")} · IBIS</span>${fileSel(ref, cur, cfg, own)}${loadBtn(ref)}${err ? `<span class="err">${esc(err)}</span>` : ""}${info ? compSel + pm + cn + selCtl : ""}${st}${entry}</div>`);
  }
  const p = plain.length ? `<div class="ce-box ib-box"><span class="ce-h" title="IBIS models at the IC ends: used in the check and the eye as linearised ends (estimate); the files stay in this page's memory">IBIS</span>${plain.join("")}</div>` : "";
  return `<div class="ce-list">${boxes.join("")}${p}</div>`;
}

// the kisi.json "ibis" entry of the current choice (names, options and the file's sha256)
function ibisEntryJson(ref) {
  const cur = ibisEntryOf(ref);
  if (!cur) return "";
  const f = S.ibisFiles[ibisBase(cur.file)], info = f && f.info;
  const comps = info ? info.components : [];
  const e = { file: cur.file, component: cur.component || (comps.length === 1 ? comps[0].name : undefined), sha256: info ? info.sha256 : cur.sha256, pin_map: cur.pin_map || "by-number" };
  for (const k of ["pins", "selectors", "models"]) if (cur[k] && Object.keys(cur[k]).length) e[k] = cur[k];
  if (cur.corner && cur.corner !== "typ") e.corner = cur.corner;
  e.source = cur.source || { kind: "simulation", note: "IBIS model chosen in the kisi viewer – say where the file comes from (vendor page, design tool export)" };
  return JSON.stringify({ [ref]: e }, null, 2);
}

function bindIbis(box, after) {
  const done = () => { ibisSave(); after(); };
  box.querySelectorAll("select[data-ib-file]").forEach((s) => s.onchange = () => {
    const ref = s.dataset.ibFile, m = ibisChoices();
    if (s.value === "@cfg") delete m[ref];
    else if (s.value === "") { if (cfgIbisMap()[ref]) m[ref] = { off: true }; else delete m[ref]; }
    else {
      const e = ibisEntryEdit(ref);
      if (ibisBase(e.file) !== s.value) { e.file = s.value; delete e.sha256; }
      const info = (S.ibisFiles[s.value] || {}).info;
      if (info && e.component && !info.components.some((c) => c.name === e.component)) delete e.component;
    }
    done();
  });
  box.querySelectorAll("input[data-ib-load]").forEach((inp) => inp.onchange = async () => {
    const f = inp.files && inp.files[0];
    if (!f) return;
    const r = await loadIbisFile(f);
    const e = ibisEntryEdit(inp.dataset.ibLoad);
    e.file = f.name; delete e.sha256;
    if (r.info && e.component && !r.info.components.some((c) => c.name === e.component)) delete e.component;
    done();
  });
  const set = (attr, key) => box.querySelectorAll(`select[data-${attr}]`).forEach((s) => s.onchange = () => {
    const e = ibisEntryEdit(s.getAttribute(`data-${attr}`));
    if (s.value) e[key] = s.value; else delete e[key];
    done();
  });
  set("ib-comp", "component"); set("ib-pm", "pin_map"); set("ib-corner", "corner");
  box.querySelectorAll("select[data-ib-sel]").forEach((s) => s.onchange = () => {
    const e = ibisEntryEdit(s.dataset.ibSel);
    e.selectors = { ...(e.selectors || {}) };
    if (s.value) e.selectors[s.dataset.sel] = s.value; else delete e.selectors[s.dataset.sel];
    done();
  });
  box.querySelectorAll("select[data-ib-ov]").forEach((s) => {
    s.onclick = (ev) => ev.stopPropagation(); // the row opens the lane
    s.onchange = () => {
      const e = ibisEntryEdit(s.dataset.ibOv);
      e.models = { ...(e.models || {}) };
      for (const pad of s.dataset.pads.split(",").filter(Boolean)) { if (s.value) e.models[pad] = s.value; else delete e.models[pad]; }
      done();
    };
  });
  box.querySelectorAll("button[data-ib-copy]").forEach((bt) => bt.onclick = () => {
    const t = ibisEntryJson(bt.dataset.ibCopy);
    if (navigator.clipboard) navigator.clipboard.writeText(t).then(() => { bt.textContent = "Copied"; }, () => { bt.textContent = "Select the text above"; });
  });
}

// an end of a lane result (laneRf "ibis".tx / .rx, check row "ends"): the IBIS end, or null for the
// profile's; {model: "profile", ibis: end} = a mapping that could not be used (end.error says why)
const ibisInfoEnd = (x) => !x ? null : x.role ? x : x.ibis || null;
// lane detail: one line per IBIS end (file, component, model, linear values)
function ibisEndLine(x, what) {
  const e = ibisInfoEnd(x);
  if (!e) return "";
  if (e.error) return `<span class="err">${what} ${esc(e.refdes)}: IBIS not used – ${esc(e.error)}; the profile's ${what.toLowerCase()} applies</span>`;
  const w = (e.warnings || []).length ? ` title="${esc(e.warnings.join(" · "))}"` : "";
  return `<span${w}>${what} ${esc(e.refdes)}.${esc(e.pad)}: IBIS ${esc(e.file)} · ${esc(e.component || "")} · pin ${esc(e.ibis_pin || "")} · <b>${esc(e.model)}</b> (${esc(e.model_type || "?")}${e.model_from && e.model_from !== "pin" ? `, ${esc(e.model_from)}` : ""}, ${esc(e.corner)}) → ${ibisLinear(e)}${e.termination && e.role === "receiver" ? ` · ${esc(e.termination)}` : ""}${e.diff_pin ? ` · [Diff Pin] ${esc(e.diff_pin)}` : ""}${w ? " ⚠" : ""}</span>`;
}
// the profile as shown with an IBIS driver: its edge and its swing at the receiver
function ibisView(p, r) {
  const ib = r && r.ibis, t = ib && ibisInfoEnd(ib.tx);
  if (!ib || !ib.used || !t || t.error) return p;
  return { ...p, tx: { ...p.tx, rise_ps: ib.eye_rise_ps ?? p.tx.rise_ps, swing_mv: ib.swing_mv ?? p.tx.swing_mv } };
}
