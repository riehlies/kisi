"use strict";
// ============================================================== prepare (board preparation)
// The "Prepare PCB" page: what the board file lacks, per analysis category (tabs Impedance, Channel,
// Power – the temperature is under Power), with forms for the simple values, one draft per board,
// the live check, the kisi.json preview and saving next to the board. Design: docs/mockups/prepare.md.
// Everything that decides comes from the core through the engine: the gap list with its categories,
// readiness and dismissals (kisi_gaps, run in a worker with the draft as configuration), the check
// (kisi_config_check) and one category's entries (kisi_config_subset). This file only collects values
// and merges the draft into a loaded kisi.json by keys.
const PREP_VIEWS = { impedance: "Impedance", channel: "Channel", power: "Power" };
const PREP_OF_TAB = { si: "impedance", ch: "channel", pi: "power" };
const PREP_TITLES = {
  "stackup-missing": "Stackup", "stackup-defaults": "Laminate data", "nets-without-target": "Nets without a target impedance",
  "classes-without-standard": "Net classes without an interface standard", "assembly-variant": "Assembly variant",
  "assembly-variant-forks": "Forks in the chosen variant", "channel-part-values": "Unreadable part values in channels",
  "power-tree": "Regulator chain", "rails-without-voltage": "Rails without a voltage", "rails-without-source": "Rails without a source",
  "regulator-efficiency": "Regulator efficiency", "ic-loads": "Load currents", "cap-ratings": "Capacitor ratings",
  "bead-ratings": "Ferrite beads", "cap-values": "Unreadable capacitor values", "thermal-data": "Thermal data (temperature)",
};
// which form a gap gets (presentation only): an editor, a fix in KiCad, or the agent path
const PREP_MODE = {
  "stackup-missing": "agent", "stackup-defaults": "stackup", "nets-without-target": "kicad", "classes-without-standard": "classes",
  "assembly-variant": "variant", "assembly-variant-forks": "agent", "channel-part-values": "kicad", "power-tree": "reg",
  "rails-without-voltage": "reg", "rails-without-source": "reg", "regulator-efficiency": "reg", "ic-loads": "loads",
  "cap-ratings": "agent", "bead-ratings": "series", "cap-values": "kicad", "thermal-data": "thermal",
};
const PREP_AGENT_ALSO = new Set(["ic-loads", "thermal-data", "regulator-efficiency", "assembly-variant", "bead-ratings"]);
const SRC_KINDS = ["datasheet", "standard", "application_note", "measurement", "simulation", "design", "estimate", "assumption"];
const PRIO_RANK = { high: 0, medium: 1, low: 2 };

S.prep = { open: false, tab: "impedance", drafts: {}, base: null, cur: null, check: null, scope: "all", agent: {}, cards: {}, dismissing: null, busy: false, note: "" };

const prepBoardFile = () => S.name + ".kicad_pcb";
function prepNewDraft() {
  return { stackup: {}, classes: {}, reg: [], loads: {}, series: {}, thermal: {}, variant: null, dismissed: {}, src: {} };
}
function prepDraft() { return S.prep.drafts[S.name] || (S.prep.drafts[S.name] = prepNewDraft()); }
function prepLoaded() {
  const c = S.lastFiles && S.lastFiles.cfg;
  if (!c) return null;
  try { return JSON.parse(c.text); } catch (e) { return null; }
}

// ---- the draft as kisi.json entries
const prepNum = (v) => { const x = parseFloat(String(v).replace(",", ".")); return isFinite(x) ? x : null; };
const prepSrcOk = (s) => s && SRC_KINDS.includes(s.kind) && ((s.document || "").trim() || (s.note || "").trim());
function prepSource(s, extraNote) {
  if (!prepSrcOk(s)) return undefined;
  const o = { kind: s.kind };
  if ((s.document || "").trim()) o.document = s.document.trim();
  const note = [extraNote, (s.note || "").trim()].filter(Boolean).join(", ");
  if (note) o.note = note;
  return o;
}
// a current typed as "12 mA", "12m", "0.012" → "12mA" (the field syntax); unreadable text stays as typed
function prepAmps(t) { const v = parseSI(t, "A"); return isFinite(v) ? fieldSI(v, "A") : String(t).trim(); }
function prepDraftConfig() {
  const d = prepDraft(), out = {};
  const diel = Object.entries(d.stackup).map(([i, v]) => {
    const e = { index: +i };
    if ((v.material || "").trim()) e.material = v.material.trim();
    for (const k of ["epsilon_r", "loss_tangent"]) if ((v[k] ?? "") !== "") e[k] = prepNum(v[k]) ?? v[k];
    if (Object.keys(e).length === 1) return null;
    e.source = prepSource(d.src.stackup);
    return e;
  }).filter(Boolean);
  if (diel.length) out.stackup = { dielectrics: diel };
  const nc = {};
  for (const [k, v] of Object.entries(d.classes)) {
    const e = {};
    if ((v.target_ohm ?? "") !== "") e.target_ohm = prepNum(v.target_ohm) ?? v.target_ohm;
    if (v.diff !== undefined) e.diff = v.diff;
    if (v.profile) e.profile = v.profile;
    if (!Object.keys(e).length) continue;
    e.source = prepSource(d.src.classes);
    nc[k] = e;
  }
  if (Object.keys(nc).length) out.net_classes = nc;
  const fields = [];
  for (const r of d.reg) if ((r.ref || "").trim() && (r.value || "").trim()) fields.push({ ref: r.ref.trim(), name: "PI_REG", value: r.value.trim(), source: prepSource(r.src) });
  for (const [ref, v] of Object.entries(d.loads)) {
    const parts = Object.entries(v.rails || {}).filter(([, a]) => (a || "").trim()).map(([rail, a]) => `${fieldNet(rail)}=${prepAmps(a)}`);
    if (parts.length) fields.push({ ref, name: "PI_LOAD", value: parts.join(", "), source: prepSource(v.src, v.basis) });
  }
  for (const [ref, v] of Object.entries(d.series)) {
    const p = [];
    const z = parseSI(v.z100 || "", "Ω"), r = parseSI(v.rdc || "", "Ω");
    if ((v.z100 || "").trim()) p.push(`z100=${isFinite(z) ? fmtSig(z, 4) : v.z100.trim()}`);
    if ((v.rdc || "").trim()) p.push(`rdc=${isFinite(r) ? fmtSig(r, 4) : v.rdc.trim()}`);
    if (p.length) fields.push({ ref, name: "PI_SERIES", value: p.join(" "), source: prepSource(d.src.series) });
  }
  for (const [ref, v] of Object.entries(d.thermal)) {
    const p = ["theta_jb", "theta_jc", "psi_jt", "tj_max"].filter((k) => (v[k] || "").trim()).map((k) => `${k}=${v[k].trim()}`);
    if (p.length) fields.push({ ref, name: "PI_THERMAL", value: p.join(" "), source: prepSource(d.src.thermal) });
  }
  if (fields.length) out.fields = fields;
  if (d.variant) out.variant = d.variant;
  const dis = Object.entries(d.dismissed).filter(([, x]) => x.items.length).map(([gap, x]) => ({ gap, items: x.items, reason: (x.reason || "").trim() }));
  if (dis.length) out.gaps = { dismissed: dis };
  return out;
}
// the draft laid over a loaded kisi.json, entry by entry (dielectric index, class name, part + field,
// the dismissals of a gap); everything else of the loaded file stays
function prepMerge(base, add) {
  const out = JSON.parse(JSON.stringify(base || {}));
  out.kisi_config = out.kisi_config || 1;
  out.board = out.board || prepBoardFile();
  if (add.stackup) {
    const ds = (out.stackup = out.stackup || {}).dielectrics = (out.stackup.dielectrics || []);
    for (const e of add.stackup.dielectrics) { const i = ds.findIndex((x) => x.index === e.index); if (i >= 0) ds[i] = { ...ds[i], ...e }; else ds.push(e); }
    ds.sort((a, b) => a.index - b.index);
  }
  if (add.net_classes) { out.net_classes = out.net_classes || {}; for (const [k, e] of Object.entries(add.net_classes)) out.net_classes[k] = { ...(out.net_classes[k] || {}), ...e }; }
  if (add.fields) {
    const fs = out.fields = out.fields || [];
    for (const f of add.fields) { const i = fs.findIndex((x) => x.ref === f.ref && String(x.name).toUpperCase() === f.name); if (i >= 0) fs[i] = f; else fs.push(f); }
  }
  if (add.variant) out.variant = add.variant;
  if (add.gaps) {
    const ds = ((out.gaps = out.gaps || {}).dismissed = (out.gaps.dismissed || []).filter((x) => !add.gaps.dismissed.some((y) => y.gap === x.gap)));
    ds.push(...add.gaps.dismissed);
  }
  return out;
}
const prepHasDraft = () => Object.keys(prepDraftConfig()).length > 0;
// the draft's entries for a save scope: all, or one category's (kisi_config_subset)
function prepAdd(scope) {
  let add = prepDraftConfig();
  if (scope && scope !== "all" && Object.keys(add).length) {
    try { add = JSON.parse(S.engine.configSubset(JSON.stringify({ kisi_config: 1, ...add }), scope)); } catch (e) { console.warn(e); }
    delete add.kisi_config; delete add.board;
  }
  return add;
}
function prepCountEntries(c) {
  return ((c.stackup && c.stackup.dielectrics) || []).length + Object.keys(c.net_classes || {}).length + (c.fields || []).length + (c.variant ? 1 : 0) + ((c.gaps && c.gaps.dismissed) || []).length;
}
function prepText(scope) { return JSON.stringify(prepMerge(prepLoaded(), prepAdd(scope)), null, 2) + "\n"; }
// what the check and the gap list see: the loaded file with the whole draft (null: no configuration)
function prepConfigText() { return prepHasDraft() || prepLoaded() ? prepText("all") : null; }

// ---- the gap list in a worker (a new analysis of the board: about a second on large boards)
const PREP_WORKER_SRC = `
const enc = new TextEncoder(), dec = new TextDecoder();
let k = null, ready = null;
function call(name, text) {
  const b = enc.encode(text);
  const p = k.kisi_alloc(b.length);
  new Uint8Array(k.memory.buffer, p, b.length).set(b);
  const rc = k[name](p, b.length);
  k.kisi_free(p, b.length);
  const o = dec.decode(new Uint8Array(k.memory.buffer, k.kisi_out_ptr(), k.kisi_out_len()));
  if (rc !== 0) throw new Error(o);
  return o;
}
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    ready = (async () => { k = (await WebAssembly.instantiate(m.module, {})).exports; for (const [fn, text] of m.log) call(fn, text); })();
    try { await ready; } catch (err) { self.postMessage({ type: "dead", error: String(err && err.message || err) }); }
  } else if (m.type === "gaps") {
    try { await ready; self.postMessage({ type: "gaps", id: m.id, text: call("kisi_gaps", m.text) }); }
    catch (err) { self.postMessage({ type: "fail", id: m.id, error: String(err && err.message || err) }); }
  }
};`;
const PW = { w: null, gen: -1, id: 0, jobs: new Map(), url: null };
function prepWorker() {
  if (PW.w && PW.gen === S.engine.stateGen) return PW.w;
  // the engine's state changed (another board, a new analysis): a new worker replays it, and the
  // requests still open go to it
  if (PW.w) PW.w.terminate();
  PW.w = null;
  if (!S.engine.mod || typeof Worker === "undefined") return null;
  try {
    PW.url = PW.url || URL.createObjectURL(new Blob([PREP_WORKER_SRC], { type: "text/javascript" }));
    const w = new Worker(PW.url);
    w.onmessage = (e) => {
      const m = e.data, j = PW.jobs.get(m.id);
      if (m.type === "dead") { console.warn("prepare worker:", m.error); PW.w = null; for (const x of PW.jobs.values()) x.fail(m.error); PW.jobs.clear(); return; }
      if (!j) return;
      PW.jobs.delete(m.id);
      if (m.type === "gaps") j.ok(m.text); else j.fail(m.error);
    };
    w.onerror = (e) => { e.preventDefault(); PW.w = null; for (const x of PW.jobs.values()) x.fail(e.message); PW.jobs.clear(); };
    w.postMessage({ type: "init", module: S.engine.mod, log: S.engine.replay() });
    for (const [id, j] of PW.jobs) w.postMessage({ type: "gaps", id, text: j.text });
    PW.w = w; PW.gen = S.engine.stateGen;
  } catch (e) { console.warn(e); PW.w = null; }
  return PW.w;
}
function prepGaps(config) {
  const text = JSON.stringify({ board: prepBoardFile(), config: config ?? null, for: null });
  const w = prepWorker();
  if (!w) return Promise.resolve().then(() => S.engine.gaps(prepBoardFile(), config));
  return new Promise((ok, fail) => { const id = ++PW.id; PW.jobs.set(id, { text, ok: (t) => ok(JSON.parse(t)), fail: (e) => fail(new Error(e)) }); w.postMessage({ type: "gaps", id, text }); })
    .catch((e) => { console.warn("prepare worker failed, main thread:", e.message); return S.engine.gaps(prepBoardFile(), config); });
}

// a board was loaded (or the configuration changed): the gap list for the readiness dots
let prepSeq = 0;
function prepOnBoard() {
  const d = prepDraft(), lc = prepLoaded();
  // dismissals of the loaded file fill the draft, so that they can be undone here
  for (const x of (lc && lc.gaps && lc.gaps.dismissed) || []) if (!d.dismissed[x.gap]) d.dismissed[x.gap] = { items: [...(x.items || [])], reason: x.reason || "" };
  S.prep.base = null; S.prep.cur = null; S.prep.note = ""; S.prep.saved = ""; S.prep.dismissing = null;
  const seq = ++prepSeq, name = S.name, base = lc ? S.lastFiles.cfg.text : null;
  prepGaps(base).then((g) => {
    if (seq !== prepSeq || name !== S.name) return;
    S.prep.base = g;
    if (prepHasDraft()) prepUpdate(true); else { S.prep.cur = g; prepRender(); }
  }).catch((e) => { console.warn(e); S.prep.note = "The gap list could not be computed: " + e.message; prepRender(); });
  prepCheck();
  prepRender();
}
// after an edit: the check at once, the gap list with the draft a little later
let prepTimer = null;
function prepUpdate(now) {
  prepCheck();
  clearTimeout(prepTimer);
  const run = () => {
    const seq = ++prepSeq, name = S.name;
    S.prep.busy = true; prepRenderStatus();
    prepGaps(prepConfigText()).then((g) => { if (seq === prepSeq && name === S.name) { S.prep.cur = g; S.prep.busy = false; prepRender(); } })
      .catch((e) => { console.warn(e); S.prep.busy = false; prepRender(); });
  };
  if (now) run(); else prepTimer = setTimeout(run, 700);
}
function prepCheck() {
  const t = prepConfigText();
  if (!t || !S.scene) { S.prep.check = null; return; }
  try { S.prep.check = S.engine.configCheck(prepBoardFile(), t); } catch (e) { S.prep.check = { ok: false, entries: 0, errors: [e.message], warnings: [], messages: [{ level: "error", text: e.message, views: Object.keys(PREP_VIEWS) }] }; }
}

// ---- state of a gap and of a category
const prepGapsNow = () => (S.prep.cur || S.prep.base || { gaps: [] }).gaps;
const prepViewsNow = () => (S.prep.cur || S.prep.base || {}).views || null;
function prepGapState(g) {
  const now = prepGapsNow().find((x) => x.id === g.id), dis = ((S.prep.cur || {}).dismissed || []).filter((x) => x.gap === g.id && x.items.length);
  if (!now) return dis.length ? { cls: "dis", text: "dismissed" } : { cls: "done", text: "✓ filled" };
  const n = (x) => x.id === "nets-without-target" ? x.items.reduce((s, i) => s + (i.nets || []).length, 0) : x.items.length;
  const left = n(now), all = n(g);
  if (left < all) return { cls: "", text: `${left} of ${all} left` };
  return { cls: "", text: all ? `${all} item${all === 1 ? "" : "s"}` : "open" };
}
function prepDot(v) {
  const x = prepViewsNow() && prepViewsNow()[v];
  if (!x) return "";
  return x.state === "nothing to check" ? "na" : x.state === "ready" ? "ok" : x.high ? "h" : "m";
}
function prepStateText(v) {
  const x = prepViewsNow() && prepViewsNow()[v];
  if (!x) return "…";
  if (x.state !== "open") return x.state + (x.state === "ready" && x.low ? ` · ${x.low} refinement${x.low > 1 ? "s" : ""}` : "");
  return `${x.high + x.medium} open${x.high ? ` (${x.high} high)` : ""}${x.low ? ` · ${x.low} low` : ""}`;
}
// the badge: open high and medium gaps of the categories with something to check
function prepCount() {
  const v = prepViewsNow();
  if (!v) return null;
  const ok = Object.keys(PREP_VIEWS).filter((k) => v[k] && v[k].applicable);
  return prepGapsNow().filter((g) => g.priority !== "low" && g.views.some((x) => ok.includes(x))).length;
}
function prepRenderStatus() {
  for (const [t, v] of Object.entries(PREP_OF_TAB)) {
    const el = $("dot-" + t), c = prepDot(v);
    if (!el) continue;
    el.className = "dot " + c; el.hidden = !c;
    $("view-" + t).title = c ? `${PREP_VIEWS[v]}: ${prepStateText(v)} (Prepare PCB)` : "";
  }
  const n = prepCount(), b = $("prep-count");
  if (b) { b.hidden = n == null; b.textContent = n ?? ""; const v = prepViewsNow() || {}; b.className = "count " + (Object.values(v).some((x) => x.applicable && x.high) ? "h" : n ? "m" : "z"); }
  const st = $("prep-busy"); if (st) st.hidden = !S.prep.busy;
}

// ---- forms
function prepSrcBlock(key, label) {
  const s = prepDraft().src[key] || {};
  return `<div class="prep-src" data-src="${key}"><b>${esc(label || "Source of these values")} – required</b>
    <select data-k="kind" aria-label="source kind"><option value="">kind…</option>${SRC_KINDS.map((k) => `<option ${s.kind === k ? "selected" : ""}>${k}</option>`).join("")}</select>
    <input data-k="document" placeholder="document or url" value="${esc(s.document || "")}" aria-label="document">
    <input data-k="note" placeholder="note: typical / maximum, condition" value="${esc(s.note || "")}" aria-label="note"></div>`;
}
const prepKindSel = (attrs, cur) => `<select ${attrs} aria-label="source kind"><option value="">kind…</option>${SRC_KINDS.map((k) => `<option ${cur === k ? "selected" : ""}>${k}</option>`).join("")}</select>`;
function prepRegSuggestions(g) {
  return g.items.filter((it) => it.write && it.write.startsWith("PI_REG on ")).map((it) => {
    const m = it.write.match(/^PI_REG on (\S+): (.*)$/);
    return m ? `<button class="prep-sugg" data-add-reg="${esc(m[1])}" data-val="${esc(m[2] + " r=")}">+ ${esc(m[1])}: ${esc(m[2])}</button>` : "";
  }).join("");
}
function prepEditor(g, gs) {
  const d = prepDraft(), mode = PREP_MODE[g.id];
  if (mode === "stackup") {
    return `<table class="prep-ed"><thead><tr><th>#</th><th>Between</th><th>Material</th><th>εr</th><th>tanδ</th><th>Thickness</th></tr></thead><tbody>${g.items.map((it) => {
      const e = d.stackup[it.index] || {}, defEr = it.why.some((w) => w.startsWith("εr")), defTd = it.why.some((w) => w.startsWith("tan"));
      const why = [defEr && "εr", defTd && "tanδ"].filter(Boolean).join(", ");
      return `<tr><td class="m" data-l="index">${it.index}</td><td data-l="between">${esc(it.between)}<span class="sub">${esc(it.kind)}${why ? ` · ${why}: KiCad default` : ""}</span></td>
        <td data-l="material"><input data-ed="stackup" data-i="${it.index}" data-k="material" value="${esc(e.material ?? "")}" placeholder="${esc(it.material)}"></td>
        <td data-l="εr"><input class="num" data-ed="stackup" data-i="${it.index}" data-k="epsilon_r" value="${esc(e.epsilon_r ?? "")}" placeholder="${fmtSig(it.epsilon_r, 3)}"></td>
        <td data-l="tanδ"><input class="num" data-ed="stackup" data-i="${it.index}" data-k="loss_tangent" value="${esc(e.loss_tangent ?? "")}" placeholder="${fmtSig(it.loss_tangent, 3)}"></td>
        <td class="m" data-l="thickness">${fmtSI(it.thickness_mm * 1e-3, "m")}</td></tr>`;
    }).join("")}</tbody></table>${prepSrcBlock("stackup")}`;
  }
  if (mode === "classes") {
    return `<table class="prep-ed"><thead><tr><th>Class</th><th>Nets</th><th>Target Ω</th><th>Diff</th><th>Profile</th></tr></thead><tbody>${g.items.map((it) => {
      const e = d.classes[it.class] || {}, diff = e.diff ?? it.diff;
      const opts = IFC.profiles.filter((p) => !["se50", "diff100"].includes(p.id)).map((p) => `<option value="${p.id}" ${e.profile === p.id ? "selected" : ""}>${esc(p.name)}${(p.signal === "diff") !== diff ? " – " + p.signal : ""}</option>`).join("");
      return `<tr><td class="m" data-l="class">${esc(it.class)}</td><td data-l="nets">${it.nets}<span class="sub">${esc((it.examples || []).slice(0, 2).map(netShort).join(", "))}</span></td>
        <td data-l="target Ω"><input class="num" data-ed="classes" data-c="${esc(it.class)}" data-k="target_ohm" value="${esc(e.target_ohm ?? "")}" placeholder="${fmtSig(it.target_ohm, 3)}"></td>
        <td data-l="differential"><input type="checkbox" data-ed="classes" data-c="${esc(it.class)}" data-k="diff" ${diff ? "checked" : ""} aria-label="differential"></td>
        <td data-l="profile"><select data-ed="classes" data-c="${esc(it.class)}" data-k="profile"><option value="">${esc(it.profile_now || "")} (generic, now)</option>${opts}</select></td></tr>`;
    }).join("")}</tbody></table>${prepSrcBlock("classes")}`;
  }
  if (mode === "variant") {
    const names = ((S.variant && S.variant.variants) || []).map((v) => v.name || v);
    const forks = g.items.filter((it) => it.fork);
    return `${forks.length ? `<p class="note">Forks where all alternative parts are fitted (${forks.length}): ${esc(forks.slice(0, 4).map((f) => f.fork).join("; "))}${forks.length > 4 ? " …" : ""}</p>` : ""}
      <div class="prep-row">${["default", ...names].map((n) => `<label class="prep-chk"><input type="radio" name="prep-var" value="${esc(n)}" data-ed="variant" ${d.variant === n ? "checked" : ""}> <b>${esc(n)}</b>${n === "default" ? " – the board as it is, other paths as open branches" : ""}</label>`).join("")}</div>
      <p class="note">Variants by sheet, references or BOM are defined in kisi.json (agent path, or drop the BOMs with the board).</p>`;
  }
  if (mode === "reg") {
    const first = gs.find((x) => PREP_MODE[x.id] === "reg"), sugg = prepRegSuggestions(g);
    const suggHtml = sugg ? `<div class="prep-row">${sugg}</div><p class="note">Suggested by <code>kisi gaps</code> (a rail behind a ferrite or link takes the voltage before it); a click adds the row to the regulator chain – then give r= and a source.</p>` : "";
    if (first && first.id !== g.id) {
      const list = g.items.slice(0, 12).map((it) => esc(netShort(it.rail) + (it.voltage ? ` (${fmtSI(it.voltage, "V")})` : ""))).join(", ");
      return `<p class="note">${list}${g.items.length > 12 ? ` … ${g.items.length - 12} more` : ""}. Entered in the regulator chain above${g.id === "rails-without-voltage" ? " as <code>NAME@&lt;V&gt;V</code> where the rail appears" : g.id === "regulator-efficiency" ? " (the efficiency after the type, e.g. buck 88%)" : " (an input or a regulator output)"}.</p>${suggHtml}`;
    }
    const rows = d.reg.map((r, i) => `<tr><td data-l="part"><input class="ref" data-ed="reg" data-i="${i}" data-k="ref" value="${esc(r.ref)}" placeholder="U3"></td>
      <td data-l="PI_REG"><input class="wide" data-ed="reg" data-i="${i}" data-k="value" value="${esc(r.value)}" placeholder="+12V > +3V3 buck 90% 3A"></td>
      <td data-l="source">${prepKindSel(`data-ed="reg" data-i="${i}" data-k="kind"`, r.src && r.src.kind)}</td>
      <td data-l="document / note"><input data-ed="reg" data-i="${i}" data-k="document" value="${esc((r.src && r.src.document) || "")}" placeholder="datasheet, page, note"></td>
      <td><button class="btn small" data-del-reg="${i}" title="remove">×</button></td></tr>`).join("");
    return `<table class="prep-ed"><thead><tr><th>Part</th><th>PI_REG (input &gt; output kind values)</th><th>Source</th><th>Document / note</th><th></th></tr></thead><tbody>${rows || `<tr><td colspan="5" class="note">No regulator entered yet.</td></tr>`}</tbody></table>
      <div class="prep-row"><button class="btn small" data-add-reg="" data-val="">+ Regulator or input</button><span class="note">kinds: input, buck, boost, buck-boost, sepic, inverter, ldo, pass · values: 90%, 3A, iq=, r=, out=, id= (one PMIC channel per <code>;</code>)</span></div>${suggHtml}`;
  }
  if (mode === "loads") {
    const rows = g.items.map((it) => {
      const e = d.loads[it.ref] || {};
      return `<tr><td class="m" data-l="IC">${esc(it.ref)}<span class="sub">${esc(it.mpn ? `${it.mpn} (${it.manufacturer || it.value})` : it.value || "")}</span></td>
        <td data-l="current per rail">${(it.rails || []).map((r) => `<div class="prep-rail"><span class="m">${esc(netShort(r))}</span><input class="num" data-ed="loads" data-r="${esc(it.ref)}" data-rail="${esc(r)}" value="${esc((e.rails || {})[r] ?? "")}" placeholder="mA"></div>`).join("")}</td>
        <td data-l="basis"><select data-ed="loads" data-r="${esc(it.ref)}" data-k="basis"><option value="">–</option><option ${e.basis === "typical" ? "selected" : ""}>typical</option><option ${e.basis === "maximum" ? "selected" : ""}>maximum</option></select></td>
        <td data-l="source">${prepKindSel(`data-ed="loads" data-r="${esc(it.ref)}" data-k="kind"`, e.src && e.src.kind)}<input data-ed="loads" data-r="${esc(it.ref)}" data-k="document" value="${esc((e.src && e.src.document) || "")}" placeholder="datasheet, page"></td></tr>`;
    }).join("");
    return `<table class="prep-ed"><thead><tr><th>IC</th><th>Current per rail</th><th>Basis</th><th>Source</th></tr></thead><tbody>${rows}</tbody></table>`;
  }
  if (mode === "series") {
    const rows = g.items.map((it) => {
      const e = d.series[it.ref] || {}, z = (String(it.value).match(/^(\d+)R/) || [])[1] || "";
      return `<tr><td class="m" data-l="bead">${esc(it.ref)}<span class="sub">${esc(it.mpn || it.value)} · ${esc(it.rail)}</span></td>
        <td data-l="Z at 100 MHz (Ω)"><input class="num" data-ed="series" data-r="${esc(it.ref)}" data-k="z100" value="${esc(e.z100 ?? "")}" placeholder="${z || "220"}"></td>
        <td data-l="DC resistance (Ω)"><input class="num" data-ed="series" data-r="${esc(it.ref)}" data-k="rdc" value="${esc(e.rdc ?? "")}" placeholder="0.1"></td></tr>`;
    }).join("");
    return `<table class="prep-ed"><thead><tr><th>Bead</th><th>Z at 100 MHz (Ω)</th><th>DC resistance (Ω)</th></tr></thead><tbody>${rows}</tbody></table>${prepSrcBlock("series")}`;
  }
  if (mode === "thermal") {
    const rows = g.items.map((it) => {
      const e = d.thermal[it.ref] || {};
      return `<tr><td class="m" data-l="part">${esc(it.ref)}<span class="sub">${esc(it.value || "")}${it.heat_w ? " · " + fmtSI(it.heat_w, "W") : ""}</span></td>${["theta_jb", "theta_jc", "psi_jt", "tj_max"].map((k) =>
        `<td data-l="${k}"><input class="num" data-ed="thermal" data-r="${esc(it.ref)}" data-k="${k}" value="${esc(e[k] ?? "")}" placeholder="${k === "tj_max" ? "125" : "K/W"}"></td>`).join("")}</tr>`;
    }).join("");
    return `<table class="prep-ed"><thead><tr><th>Part</th><th>θJB</th><th>θJC</th><th>ψJT</th><th>Tj max</th></tr></thead><tbody>${rows}</tbody></table>${prepSrcBlock("thermal", "Source of these values (the parts' Thermal Information tables)")}`;
  }
  if (mode === "kicad") {
    const it = g.items[0] || {};
    const nets = (it.nets || []).slice(0, 4).map((n) => esc(netShort(n.net) + (n.looks_like ? ` (${n.looks_like}?)` : ""))).join(", ");
    return `<p class="note">${it.class ? `Class <code>${esc(it.class)}</code>: ${nets}. ` : ""}A fix in KiCad, not in kisi.json: ${esc((g.write && g.write.to) || "")}. Reload the board after saving.</p>`;
  }
  const list = g.items.slice(0, 6).map((it) => esc([it.ref, it.mpn || it.value, it.rail].filter(Boolean).join(" "))).join(" · ");
  return `<p class="note">${list}${g.items.length > 6 ? ` … ${g.items.length} in all` : ""}</p>`;
}
// dismissable items (the core marks them): nets of nets-without-target, rails found only by name
function prepDismissable(g) {
  if (g.id === "nets-without-target") return g.items.flatMap((it) => (it.nets || []).filter((n) => n.dismissable).map((n) => n.net));
  return g.items.filter((it) => it.dismissable).map((it) => it.rail);
}
function prepDismissBlock(g) {
  const all = [...new Set([...prepDismissable(g), ...((prepDraft().dismissed[g.id] || {}).items || [])])];
  if (!all.length) return "";
  const x = prepDraft().dismissed[g.id] || { items: [], reason: "" };
  if (S.prep.dismissing !== g.id && !x.items.length) return `<div class="prep-row"><button class="btn small" data-dismiss="${g.id}">Dismiss…</button><span class="note">${all.length} item${all.length > 1 ? "s" : ""} found only by ${g.id === "nets-without-target" ? "their net names" : "their names"} – dismiss what is not ${g.id === "nets-without-target" ? "an interface" : "a supply"}.</span></div>`;
  return `<div class="prep-dismiss"><b>Dismiss – stored in <code>${esc(S.name)}.kisi.json</code> → <code>gaps.dismissed</code> with your reason; <code>kisi gaps</code>, the CLI and agents skip these items</b>
    <div class="prep-row">${all.map((n) => `<label class="prep-chk"><input type="checkbox" data-dis-item="${esc(n)}" data-gap-id="${g.id}" ${x.items.includes(n) ? "checked" : ""}> <code>${esc(netShort(n))}</code></label>`).join("")}</div>
    <input data-dis-reason="${g.id}" value="${esc(x.reason || "")}" placeholder="reason (required), e.g. QSPI flash at 50 MHz – no impedance control needed" aria-label="reason">
    <div class="prep-row"><button class="btn small" data-dis-all="${g.id}">Select all</button><button class="btn small" data-undo="${g.id}">Undo</button></div></div>`;
}
function prepCard(g, gs, na) {
  const st = prepGapState(g), mode = PREP_MODE[g.id], key = S.name + g.id;
  const open = S.prep.cards[key] ?? (!na && st.cls !== "dis" && st.cls !== "done" && g.priority !== "low");
  const others = g.views.filter((v) => v !== S.prep.tab).map((v) => `<span class="prep-tag view" title="the same entries serve this category too – filled once">also ${PREP_VIEWS[v]}</span>`).join("");
  const tags = [mode === "kicad" ? `<span class="prep-tag kicad">fix in KiCad</span>` : mode === "agent" ? `<span class="prep-tag agent">agent path</span>` : `<span class="prep-tag">enter here</span>`]
    .concat(g.affects.map((a) => `<span class="prep-tag">${esc(a)}</span>`)).join("");
  const agent = (mode === "agent" || PREP_AGENT_ALSO.has(g.id)) ? `<div class="prep-agentbox"><p>${mode === "agent" ? "Many parts or research-heavy values: let an agent read the datasheets (skill kisi-prepare), with a source for every value." : g.id === "assembly-variant" ? "Variants from the project's BOM and position files: choose here, or let an agent define them." : "Values to look up in datasheets: enter them here, or hand the gap to the agent."}</p>
      <label class="prep-chk"><input type="checkbox" data-agent="${g.id}" ${S.prep.agent[key] ? "checked" : ""}> hand to the agent</label></div>` : "";
  return `<article class="prep-gap ${st.cls === "dis" ? "isdis" : ""} ${na ? "later" : ""}" id="prep-gap-${g.id}" data-gap="${g.id}" aria-expanded="${open}">
    <header data-toggle="${g.id}"><span class="prio ${g.priority}">${g.priority}</span><h3>${esc(PREP_TITLES[g.id] || g.id)}<code>${g.id}</code></h3>
      <span class="state ${st.cls}">${na && st.cls !== "done" && st.cls !== "dis" ? "not blocking" : esc(st.text)}</span><p>${esc(g.issue)}</p></header>
    <div class="body">
      <dl class="prep-meta"><dt>Where to find it</dt><dd>${esc(g.where_to_find)}</dd><dt>Goes to</dt><dd><code>${esc((g.write && g.write.to) || "")}</code></dd><dt>Affects</dt><dd>${others}${tags}</dd></dl>
      ${st.cls === "dis" ? "" : prepEditor(g, gs)}${prepDismissBlock(g)}${st.cls === "dis" ? "" : agent}
    </div></article>`;
}
function prepPrompt(gs) {
  const ids = gs.filter((g) => prepGapState(g).cls === "" && (PREP_MODE[g.id] === "agent" || S.prep.agent[S.name + g.id])).map((g) => g.id);
  return `Prepare ${prepBoardFile()} for kisi's ${PREP_VIEWS[S.prep.tab].toLowerCase()} analysis with the skill kisi-prepare. Start from \`kisi gaps --for ${S.prep.tab}\`${ids.length ? `, research the gaps ${ids.join(", ")}` : ", research this category's open gaps"} from datasheets, BOM and position files, with a source for every value. Merge the results into ${S.name}.kisi.json next to the board – keep my entries and the dismissed gaps – and run \`kisi config-check\`.`;
}

// ---- the page
function prepRender() {
  prepRenderStatus();
  if (!S.prep.open || !S.scene) return;
  const base = S.prep.base, tab = S.prep.tab, vnow = prepViewsNow() || {};
  $("prep-ready").innerHTML = Object.entries(PREP_VIEWS).map(([v, name]) => `<button data-prep-tab="${v}" aria-current="${v === tab}"><span class="dot ${prepDot(v)}"></span><b>${name}</b><span>${esc(prepStateText(v))}</span></button>`).join("");
  $("prep-tabs").innerHTML = Object.entries(PREP_VIEWS).map(([v, name]) => `<button role="tab" aria-selected="${v === tab}" data-prep-tab="${v}"><span class="dot ${prepDot(v)}"></span>${name}<small>${esc(prepStateText(v))}</small></button>`).join("");
  const lc = prepLoaded();
  $("prep-cfg").textContent = lc ? `${S.lastFiles.cfg.name} loaded` : "no kisi.json";
  if (!base) {
    $("prep-head").innerHTML = `<p class="note">${esc(S.prep.note || "Computing the gap list (kisi gaps)…")}</p>`;
    $("prep-gaps").innerHTML = ""; prepRenderSide([]); return;
  }
  const x = vnow[tab] || base.views[tab], na = x && !x.applicable;
  const gs = base.gaps.filter((g) => g.views.includes(tab)).sort((a, b) => ((prepGapState(a).cls === "dis") - (prepGapState(b).cls === "dis")) || PRIO_RANK[a.priority] - PRIO_RANK[b.priority]);
  $("prep-head").innerHTML = na
    ? `<div class="prep-na"><b>Nothing to check for ${PREP_VIEWS[tab]}:</b> ${esc(x.why)}. ${tab === "impedance" ? "The laminate data below still matters once net classes get a target impedance (and for loss in a channel check); it blocks nothing now." : "The gaps below matter only once there is something to check; they block nothing now."}</div>`
    : `<p class="note">${gs.length} gap${gs.length === 1 ? "" : "s"} for ${PREP_VIEWS[tab]}, by priority · ${esc(prepStateText(tab))}${x && x.why ? " · " + esc(x.why) : ""}. Gaps marked “also …” serve other categories too and are filled once.</p>`;
  const focus = document.activeElement && document.activeElement.closest && document.activeElement.closest("#prep-gaps [data-gap]") ? { gap: document.activeElement.closest("[data-gap]").dataset.gap, sel: prepSel(document.activeElement) } : null;
  $("prep-gaps").innerHTML = gs.length ? gs.map((g) => prepCard(g, gs, na)).join("") : `<div class="prep-empty">kisi gaps reports nothing missing for ${PREP_VIEWS[tab]}.</div>`;
  if (focus) { const el = document.querySelector(`#prep-gaps [data-gap="${focus.gap}"] ${focus.sel}`); if (el) { el.focus(); if (el.setSelectionRange && el.type === "text") { const n = el.value.length; el.setSelectionRange(n, n); } } }
  prepRenderSide(gs, na);
}
function prepSel(el) { return Object.entries(el.dataset).map(([k, v]) => `[data-${k.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase())}="${CSS.escape(v)}"]`).join(""); }
function prepMsgs() {
  // the live check of the draft, plus what only the analysed board can tell (dismissals) from the gap list
  const ck = S.prep.check, msgs = ck ? [...ck.messages] : [];
  const cc = S.prep.cur && S.prep.cur.config;
  if (ck && cc && prepHasDraft()) for (const [level, list] of [["error", cc.errors || []], ["warning", cc.warnings || []]]) for (const t of list) if (t.startsWith("gaps.dismissed") && !msgs.some((m) => m.text === t)) msgs.push({ level, text: t, views: (((S.prep.base || { gaps: [] }).gaps.find((g) => t.includes(`[${g.id}]`)) || {}).views) || Object.keys(PREP_VIEWS) });
  return msgs;
}
function prepRenderSide(gs, na) {
  const tab = S.prep.tab, entries = (S.prep.check && S.prep.check.entries) || 0;
  const n = prepCountEntries(prepDraftConfig());
  $("prep-draft").textContent = n ? `Draft: ${n} entr${n === 1 ? "y" : "ies"} entered${prepLoaded() ? `, laid over the loaded file (${entries} in all)` : ""}` : "Draft: nothing entered";
  const scopes = [["all", "save everything entered"], [tab, `save only ${PREP_VIEWS[tab]}`]];
  if (!scopes.some((s) => s[0] === S.prep.scope)) S.prep.scope = "all";
  $("prep-scope").innerHTML = scopes.map(([k, t]) => `<option value="${k}" ${S.prep.scope === k ? "selected" : ""}>${t}</option>`).join("");
  const blocked = prepBlocked(S.prep.scope);
  const ns = prepHasScope(S.prep.scope);
  $("prep-save").disabled = !ns || blocked.length > 0;
  $("prep-save").title = !ns ? "nothing entered for this" : blocked.length ? "fix the errors first (warnings are fine)" : "";
  const fsa = typeof window.showSaveFilePicker === "function";
  $("prep-save-note").textContent = S.prep.saved ? S.prep.saved : !n ? "" : blocked.length ? `${blocked.length} error${blocked.length > 1 ? "s" : ""} in ${S.prep.scope === "all" ? "the draft" : `the ${PREP_VIEWS[S.prep.scope]} entries`}${S.prep.scope === "all" && !prepBlocked(tab).length && prepHasScope(tab) ? ` – “save only ${PREP_VIEWS[tab]}” works` : ""}.` : `${fsa ? "Saves next to the board (you choose the place)" : "Downloads"} ${S.name}.kisi.json${prepLoaded() ? ", the loaded file with your entries" : ""}, then loads it.`;
  const msgs = prepMsgs(), mine = msgs.filter((m) => m.views.includes(tab)), other = msgs.filter((m) => !m.views.includes(tab));
  const nerr = msgs.filter((m) => m.level === "error").length;
  $("prep-check-sum").textContent = n ? (nerr ? `${nerr} error${nerr === 1 ? "" : "s"}` : "ok") : "";
  $("prep-msgs").innerHTML = !n ? `<li class="o">Nothing entered yet.</li>` : mine.map((m) => `<li class="${m.level === "warning" ? "w" : ""}">${esc(m.text)}</li>`).join("") +
    (other.length ? `<li class="o">${other.length} more in entries of other categories</li>` : "") + (!nerr ? `<li class="ok">ok – ${entries} entr${entries === 1 ? "y" : "ies"}</li>` : "");
  const ag = (gs || []).filter((g) => prepGapState(g).cls === "" && (PREP_MODE[g.id] === "agent" || S.prep.agent[S.name + g.id]));
  $("prep-agent-for").textContent = `--for ${tab}`;
  $("prep-agent-list").innerHTML = ag.length ? ag.map((g) => `<li class="w"><b>${esc(PREP_TITLES[g.id] || g.id)}</b> <code>${g.id}</code></li>`).join("") : `<li class="o">nothing handed over – the prompt asks for this category's open gaps</li>`;
  $("prep-prompt").textContent = na ? `Nothing to check for ${PREP_VIEWS[tab]} on this board – no prompt needed.` : prepPrompt(gs || []);
  $("prep-copy").disabled = !!na;
  $("prep-json-name").textContent = S.name + ".kisi.json";
  $("prep-json").textContent = n ? prepText(S.prep.scope) : (prepLoaded() ? S.lastFiles.cfg.text : "(nothing entered yet – the file appears here as you fill in values)");
  $("prep-json-note").textContent = S.prep.scope === "all" ? (prepLoaded() ? "The loaded file with your entries." : "Everything entered.") : `Only what ${PREP_VIEWS[S.prep.scope]} uses from your entries${prepLoaded() ? ", laid over the loaded file" : ""}.`;
}
// errors in what would be saved (warnings do not block)
function prepHasScope(scope) { return prepCountEntries(prepAdd(scope)) > 0; }
function prepBlocked(scope) {
  return prepMsgs().filter((m) => m.level === "error" && (scope === "all" || m.views.includes(scope)));
}
async function prepSave() {
  const scope = S.prep.scope, text = prepText(scope), name = `${S.name}.kisi.json`;
  let ck;
  try { ck = S.engine.configCheck(prepBoardFile(), text); } catch (e) { ck = { ok: false, errors: [e.message] }; }
  if (!ck.ok) { S.prep.saved = "Not saved: " + ck.errors.slice(0, 2).join("; "); prepRender(); return; }
  let how = "downloaded";
  if (typeof window.showSaveFilePicker === "function") {
    try {
      const start = (S.fileHandles || []).find((h) => /\.kicad_pcb$/i.test(h.name));
      const h = await window.showSaveFilePicker({ suggestedName: name, startIn: start || "documents", types: [{ description: "kisi board configuration", accept: { "application/json": [".json"] } }] });
      const w = await h.createWritable(); await w.write(text); await w.close();
      if (S.fileHandles) S.fileHandles = S.fileHandles.filter((x) => !/\.kisi\.json$/i.test(x.name)).concat([h]);
      how = "saved";
    } catch (e) { if (e && e.name === "AbortError") return; console.warn(e); prepDownload(name, text); }
  } else prepDownload(name, text);
  S.prep.lastSaved = text;
  S.prep.drafts[S.name] = prepNewDraft();
  const l = S.lastFiles;
  await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, { name, text, csv: (l.cfg && l.cfg.csv) || {} });
  S.prep.saved = `${name} ${how} and loaded${scope === "all" ? "" : ` (${PREP_VIEWS[scope]} entries)`}.`;
  prepRender();
}
function prepDownload(name, text) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  a.download = name; document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}
function prepOpen(on) {
  S.prep.open = on;
  $("app").classList.toggle("prep-open", on);
  $("prep").hidden = !on;
  $("prep-btn").setAttribute("aria-pressed", String(on));
  for (const k of ["si", "ch", "pi"]) $("view-" + k).setAttribute("aria-pressed", String(!on && S.tab === k));
  if (on) { if (PREP_OF_TAB[S.tab] && !S.prep.touched) S.prep.tab = PREP_OF_TAB[S.tab]; S.prep.saved = ""; prepCheck(); prepRender(); window.scrollTo(0, 0); }
}

// ---- events
$("prep-btn").onclick = () => { if (S.scene) prepOpen(!S.prep.open); };
for (const k of ["si", "ch", "pi"]) $("view-" + k).addEventListener("click", () => { if (S.prep.open) prepOpen(false); });
$("prep").addEventListener("input", (ev) => {
  const t = ev.target, d = prepDraft(), ed = t.dataset.ed, val = t.type === "checkbox" ? t.checked : t.value;
  const srcEl = t.closest("[data-src]");
  S.prep.saved = "";
  if (t.id === "prep-scope") { S.prep.scope = t.value; prepRender(); return; }
  if (t.dataset.disItem) { const x = (d.dismissed[t.dataset.gapId] ||= { items: [], reason: "" }); x.items = t.checked ? [...new Set([...x.items, t.dataset.disItem])] : x.items.filter((i) => i !== t.dataset.disItem); }
  else if (t.dataset.disReason) { (d.dismissed[t.dataset.disReason] ||= { items: [], reason: "" }).reason = t.value; }
  else if (srcEl) { const k = srcEl.dataset.src; d.src[k] = { ...(d.src[k] || {}), [t.dataset.k]: t.value }; }
  else if (ed === "stackup") { (d.stackup[t.dataset.i] ||= {})[t.dataset.k] = val; }
  else if (ed === "classes") { (d.classes[t.dataset.c] ||= {})[t.dataset.k] = val; }
  else if (ed === "reg") { const r = d.reg[+t.dataset.i]; if (["kind", "document"].includes(t.dataset.k)) r.src = { ...(r.src || {}), [t.dataset.k]: val }; else r[t.dataset.k] = val; }
  else if (ed === "loads") { const e = (d.loads[t.dataset.r] ||= { rails: {} }); if (t.dataset.rail) e.rails[t.dataset.rail] = val; else if (t.dataset.k === "basis") e.basis = val; else e.src = { ...(e.src || {}), [t.dataset.k]: val }; }
  else if (ed === "series") { (d.series[t.dataset.r] ||= {})[t.dataset.k] = val; }
  else if (ed === "thermal") { (d.thermal[t.dataset.r] ||= {})[t.dataset.k] = val; }
  else if (ed === "variant") { d.variant = val; }
  else if (t.dataset.agent) { S.prep.agent[S.name + t.dataset.agent] = t.checked; prepRender(); return; }
  else return;
  prepUpdate(false);
  prepRender();
});
$("prep").addEventListener("change", (ev) => { if (ev.target.matches("select, input[type=checkbox], input[type=radio]")) ev.target.dispatchEvent(new Event("input", { bubbles: true })); });
$("prep").addEventListener("click", async (ev) => {
  const t = ev.target.closest("button, header[data-toggle]");
  if (!t) return;
  const d = prepDraft();
  if (t.dataset.toggle) { const g = t.closest(".prep-gap"); S.prep.cards[S.name + t.dataset.toggle] = g.getAttribute("aria-expanded") !== "true"; prepRender(); return; }
  if (t.dataset.prepTab) { S.prep.tab = t.dataset.prepTab; S.prep.touched = true; prepRender(); return; }
  if (t.dataset.dismiss) { S.prep.dismissing = t.dataset.dismiss; prepRender(); return; }
  if (t.dataset.disAll) { const g = S.prep.base.gaps.find((x) => x.id === t.dataset.disAll); (d.dismissed[g.id] ||= { items: [], reason: "" }).items = prepDismissable(g); prepUpdate(false); prepRender(); return; }
  if (t.dataset.undo) { delete d.dismissed[t.dataset.undo]; S.prep.dismissing = null; S.prep.cards[S.name + t.dataset.undo] = true; prepUpdate(false); prepRender(); return; }
  if (t.dataset.delReg != null) { d.reg.splice(+t.dataset.delReg, 1); prepUpdate(false); prepRender(); return; }
  if ("addReg" in t.dataset) {
    const first = S.prep.base.gaps.filter((x) => x.views.includes(S.prep.tab)).find((x) => PREP_MODE[x.id] === "reg");
    d.reg.push({ ref: t.dataset.addReg, value: t.dataset.val, src: {} });
    if (first) S.prep.cards[S.name + first.id] = true;
    prepRender();
    const ins = document.querySelectorAll(`#prep-gap-${first ? first.id : ""} input[data-k="value"]`); if (ins.length) ins[ins.length - 1].focus();
    return;
  }
  if (t.id === "prep-save") { await prepSave(); return; }
  if (t.id === "prep-copy") { try { await navigator.clipboard.writeText($("prep-prompt").textContent); } catch (e) { /* no clipboard (insecure context) */ } $("prep-copied").hidden = false; setTimeout(() => ($("prep-copied").hidden = true), 1500); }
});
