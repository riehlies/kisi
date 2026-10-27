"use strict";
// ============================================================== prepare (board preparation)
// The "Prepare PCB" page: what the board file lacks, per analysis category (tabs Impedance, Channel,
// Power – the temperature is under Power), with forms for the simple values, the live check, the
// kisi.json preview and saving next to the board. User documentation: docs/preparing-a-board.md.
//
// Data:   the draft is the board's kisi.json itself (S.prep.d.work: the loaded file, or a skeleton,
//         with the edits), kept per board in this browser (PREP_KEY) until it is saved or discarded.
//         Forms read and write its entries by key (dielectric index, class, part + field, connector,
//         variant, dismissed gap); keys the page has no form for stay as they are.
// Engine: everything that decides comes from the core: the gap list with its categories (per item
//         where a gap mixes them), readiness and dismissals (kisi_gaps, in a worker), the check
//         (kisi_config_check), one category's entries (kisi_config_subset) and the viewer's power
//         settings as fields (kisi_edits_to_fields). This file collects values and merges by keys.
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
  "stackup-missing": "agent", "stackup-defaults": "stackup", "nets-without-target": "nettarget", "classes-without-standard": "classes",
  "assembly-variant": "variant", "assembly-variant-forks": "variant", "channel-part-values": "kicad", "power-tree": "reg",
  "rails-without-voltage": "reg", "rails-without-source": "reg", "regulator-efficiency": "reg", "ic-loads": "loads",
  "cap-ratings": "caps", "bead-ratings": "series", "cap-values": "kicad", "thermal-data": "thermal",
};
const SRC_KINDS = ["datasheet", "standard", "application_note", "measurement", "simulation", "design", "estimate", "assumption"];
const PRIO_RANK = { high: 0, medium: 1, low: 2 };
const PREP_TOP = ["kisi_config", "board", "stackup", "net_classes", "connector_ends", "fields", "variant", "variants", "gaps", "notes"];
// the field tables of the Power tab (entries of kisi.json "fields", by name)
const PREP_FIELDS = [
  ["PI_REG", "Regulator chain", "+12V > +3V3 buck 90% 3A"], ["PI_LOAD", "Load currents", "+3V3=120mA, +1V8=20mA"],
  ["PI_SOURCE", "Supply sources (without a power tree)", "+5V or VIN=12V"], ["PI_SERIES", "Ferrite beads and pass parts", "z100=220 rdc=0.1"],
  ["PI_CAP", "Capacitor data", "rated=6.3V X5R c@3.3V=4.1u esl=0.45n"], ["PI_THERMAL", "Thermal data", "theta_jb=17 theta_jc=50 tj_max=125"],
  ["PI_TARGET", "PDN target impedance", "+1V0=0.01"],
];
const PREP_KV = { PI_SERIES: ["z100", "rdc"], PI_THERMAL: ["theta_jb", "theta_jc", "psi_jt", "tj_max"], PI_CAP: ["rated", "esr", "esl"] };
const PREP_DIEL = /^(X5R|X7R|X6S|X7S|X8R|X7T|X6T|Y5V|Z5U|C0G|NP0|COG|NPO|POLYMER|TANTALUM)$/i;
const PREP_KEY = (name) => "kisi.prep.v1." + (name ?? S.name);

S.prep = { handOpen: {}, agentScope: "tab", open: false, tab: "impedance", scope: "all", cards: {}, dismissing: null, busy: false, note: "", saved: "", base: null, cur: null, check: null, d: null, mem: {}, pend: {}, takeNote: "",
  opt: {}, applied: {}, started: {}, moreOpen: {}, applying: null, scrollTo: null, ncName: {}, ncNets: {}, pending: {} };

const prepBoardFile = () => String(S.name).replace(/\.kicad_pcb$/i, "") + ".kicad_pcb";
const prepClone = (o) => JSON.parse(JSON.stringify(o));
// JSON with sorted keys: two objects with the same content compare equal
function prepCanon(o) {
  if (Array.isArray(o)) return "[" + o.map(prepCanon).join(",") + "]";
  if (o && typeof o === "object") return "{" + Object.keys(o).sort().map((k) => JSON.stringify(k) + ":" + prepCanon(o[k])).join(",") + "}";
  return JSON.stringify(o);
}
const prepSame = (a, b) => prepCanon(a) === prepCanon(b);
const prepLoadedText = () => (S.lastFiles && S.lastFiles.cfg ? S.lastFiles.cfg.text : null);
function prepLoaded() {
  const t = prepLoadedText();
  if (t == null) return null;
  try { const o = JSON.parse(t); return o && typeof o === "object" && !Array.isArray(o) ? o : null; } catch (e) { return null; }
}
const prepSkeleton = () => ({ kisi_config: 1, board: prepBoardFile() });
const prepBaseObj = () => prepLoaded() || prepSkeleton();

// ---- the draft per board: restored from this browser, else the loaded file
function prepD() { return S.prep.d && S.prep.d.name === S.name ? S.prep.d : prepRestore(); }
const prepW = () => prepD().work;
// a stored draft belongs to the kisi.json it was made on (hash of the loaded text) and to the draft format:
// over another base (new example data, a file edited elsewhere) or an older format it is not applied – the
// page offers Restore / Discard; a draft made on a bundled example is dropped when the example's kisi.json
// changed (a new build)
const PREP_DRAFT_V = 2;
// of the content: keys sorted, formatting ignored (a file saved here and read back has the same hash)
function prepHash(t) {
  if (t == null) return "none";
  const canon = (v) => Array.isArray(v) ? v.map(canon) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v;
  try { t = JSON.stringify(canon(JSON.parse(t))); } catch (e) { /* not JSON: the text itself */ }
  let h = 0x811c9dc5;
  for (let i = 0; i < t.length; i++) { h ^= t.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
  return h.toString(16).padStart(8, "0") + ":" + t.length;
}
const prepRec = (d, baseText, work) => ({ v: PREP_DRAFT_V, baseHash: prepHash(baseText), sample: S.sample || null, baseText, work, agent: d.agent, agentItems: d.agentItems, shared: d.shared });
function prepRestore() {
  const lt = prepLoadedText(), base = prepBaseObj();
  const d = { name: S.name, baseText: lt, work: prepClone(base), agent: {}, agentItems: {}, shared: {}, foreign: false, stale: null };
  const st = lsGet(PREP_KEY()) || S.prep.mem[S.name] || null;
  if (st && st.work && typeof st.work === "object" && !Array.isArray(st.work)) {
    if (st.v === PREP_DRAFT_V && st.baseHash === prepHash(lt)) prepTakeDraft(d, st, false);
    else if (S.sample && st.sample === S.sample) { lsSet(PREP_KEY(), null); delete S.prep.mem[S.name]; }
    else d.stale = st;
  }
  S.prep.d = d;
  return d;
}
function prepTakeDraft(d, st, foreign) {
  d.work = st.work; d.agent = st.agent || {}; d.agentItems = st.agentItems || {}; d.shared = st.shared || {}; d.foreign = foreign; d.stale = null;
}
// the notice's answers: the old draft over the loaded file, or away
function prepStaleRestore() { const d = prepD(); if (!d.stale) return; prepTakeDraft(d, d.stale, true); prepPersist(); prepUpdate(true); prepRender(true); }
function prepStaleDiscard() { const d = prepD(); if (!d.stale) return; d.stale = null; delete S.prep.mem[d.name]; lsSet(PREP_KEY(d.name), null); prepPersist(); prepRender(true); }
function prepPersist() {
  const d = prepD();
  const rec = prepRec(d, d.baseText, d.work);
  const keep = prepDirty() || Object.values(d.agent).some(Boolean) || Object.values(d.agentItems).some((x) => x.length) || Object.keys(d.shared).length;
  // an old draft waiting for Restore / Discard stays stored until the page has something of its own
  if (d.stale && !keep) return;
  d.stale = null;
  S.prep.mem[d.name] = rec;
  lsSet(PREP_KEY(d.name), keep ? rec : null);
}
const prepDirty = () => !prepSame(prepW(), prepBaseObj());
function prepDiscard() {
  const d = prepD();
  d.work = prepClone(prepBaseObj()); d.foreign = false; S.prep.pend = {};
  prepPersist();
  prepUpdate(true);
  prepRender(true);
}

// the entries of a configuration by key, for the "differs from the loaded file" count
function prepEntries(o) {
  const m = new Map(), put = (k, v) => m.set(k, prepCanon(v));
  o = o || {};
  for (const d of (o.stackup && o.stackup.dielectrics) || []) put(`dielectric ${d.index}`, d);
  if (o.stackup && o.stackup.solder_mask) put("solder mask", o.stackup.solder_mask);
  for (const [k, v] of Object.entries(o.net_classes || {})) put(`net class ${k}`, v);
  for (const [k, v] of Object.entries(o.connector_ends || {})) put(`connector end ${k}`, v);
  for (const f of o.fields || []) put(`${f.ref} ${String(f.name).toUpperCase()}`, f);
  if (o.variant != null) put("variant", o.variant);
  for (const [k, v] of Object.entries(o.variants || {})) put(`variant ${k}`, v);
  for (const x of (o.gaps && o.gaps.dismissed) || []) put(`dismissed ${x.gap}`, x);
  if (o.notes) put("notes", o.notes);
  for (const k of Object.keys(o)) if (!PREP_TOP.includes(k)) put(k, o[k]);
  return m;
}
function prepDiff() {
  const a = prepEntries(prepBaseObj()), b = prepEntries(prepW());
  let add = 0, chg = 0, del = 0;
  for (const [k, v] of b) { if (!a.has(k)) add++; else if (a.get(k) !== v) chg++; }
  for (const k of a.keys()) if (!b.has(k)) del++;
  return { add, chg, del, n: add + chg + del };
}

// ---- editing the draft
const prepNum = (v) => { const x = parseFloat(String(v).replace(",", ".")); return isFinite(x) && /^\s*[-+]?[\d.,]+(e[-+]?\d+)?\s*$/i.test(String(v)) ? x : null; };
// a typed value: a number where the key is numeric (unreadable text stays, the check reports it)
function prepVal(v, num) { const t = String(v ?? "").trim(); if (!t) return undefined; return num ? (prepNum(t) ?? t) : t; }
function prepSet(o, k, v) { if (v === undefined || v === null || v === "") delete o[k]; else o[k] = v; }
const prepEmpty = (x) => x == null || (Array.isArray(x) ? !x.length : typeof x === "object" && !Object.keys(x).length);
// containers the page emptied go (unless the loaded file had them)
function prepTidy() {
  const w = prepW(), b = prepBaseObj();
  if (w.stackup) {
    if (prepEmpty(w.stackup.dielectrics) && !(b.stackup && "dielectrics" in b.stackup)) delete w.stackup.dielectrics;
    if (prepEmpty(w.stackup) && !("stackup" in b)) delete w.stackup;
  }
  for (const k of ["net_classes", "connector_ends", "fields", "variants", "notes"]) if (k in w && prepEmpty(w[k]) && !(k in b)) delete w[k];
  if (w.gaps) {
    if (prepEmpty(w.gaps.dismissed) && !(b.gaps && "dismissed" in b.gaps)) delete w.gaps.dismissed;
    if (prepEmpty(w.gaps) && !("gaps" in b)) delete w.gaps;
  }
}
function prepDiel(i, make) {
  const w = prepW();
  let e = ((w.stackup && w.stackup.dielectrics) || []).find((x) => x.index === i);
  if (!e && make) {
    w.stackup = w.stackup || {}; w.stackup.dielectrics = w.stackup.dielectrics || [];
    e = { index: i }; w.stackup.dielectrics.push(e); w.stackup.dielectrics.sort((a, b) => a.index - b.index);
  }
  return e || null;
}
function prepMask(make) { const w = prepW(); if (!(w.stackup && w.stackup.solder_mask) && make) { w.stackup = w.stackup || {}; w.stackup.solder_mask = {}; } return (w.stackup && w.stackup.solder_mask) || null; }
function prepCls(c, make) { const w = prepW(); if (!(w.net_classes && w.net_classes[c]) && make) { (w.net_classes = w.net_classes || {})[c] = {}; } return (w.net_classes && w.net_classes[c]) || null; }
function prepConn(r, make) { const w = prepW(); if (!(w.connector_ends && w.connector_ends[r]) && make) { (w.connector_ends = w.connector_ends || {})[r] = {}; } return (w.connector_ends && w.connector_ends[r]) || null; }
function prepVar(n, make) { const w = prepW(); if (!(w.variants && w.variants[n]) && make) { (w.variants = w.variants || {})[n] = {}; } return (w.variants && w.variants[n]) || null; }
const prepFld = (ref, name) => (prepW().fields || []).find((f) => f.ref === ref && String(f.name).toUpperCase() === name) || null;
function prepDis(gap, make) {
  const w = prepW();
  let x = ((w.gaps && w.gaps.dismissed) || []).find((d) => d.gap === gap);
  if (!x && make) { w.gaps = w.gaps || {}; w.gaps.dismissed = w.gaps.dismissed || []; x = { gap, items: [], reason: "" }; w.gaps.dismissed.push(x); }
  return x || null;
}
function prepDropDis(gap) { const w = prepW(); if (w.gaps && w.gaps.dismissed) w.gaps.dismissed = w.gaps.dismissed.filter((d) => d.gap !== gap); prepTidy(); }
// a field entry: an empty value removes it
function prepSetFld(ref, name, value, srcKey) {
  const w = prepW();
  let f = prepFld(ref, name);
  if (!String(value ?? "").trim()) { if (f) { w.fields = w.fields.filter((x) => x !== f); prepTidy(); } return null; }
  if (!f) { f = { ref, name, value }; (w.fields = w.fields || []).push(f); } else f.value = value;
  if (srcKey) prepAutoSrc(f, srcKey);
  return f;
}
// an entry whose values are all gone goes (a dielectric, a class, a connector end, a variant)
const PREP_VALUES = {
  diel: ["material", "epsilon_r", "loss_tangent", "thickness"], mask: ["epsilon_r"], cls: ["target_ohm", "diff", "profile"],
  conn: ["model", "r_ohm", "c_pf", "l_nh", "rest_loss_db", "rest_delay_ps", "touchstone"],
};
function prepPrune(kind, key) {
  const w = prepW(), e = prepEntry(`${kind}:${key}`);
  if (!e || PREP_VALUES[kind].some((k) => k in e)) return;
  if (kind === "diel") w.stackup.dielectrics = w.stackup.dielectrics.filter((x) => x !== e);
  else if (kind === "mask") delete w.stackup.solder_mask;
  else if (kind === "cls") delete w.net_classes[key];
  else if (kind === "conn") delete w.connector_ends[key];
  prepTidy();
}
// an entry by its key in the forms: "diel:3", "mask", "cls:PCIe", "fld:U3|PI_REG", "conn:P1", "var:TS-PCIe", "dis:<gap>"
function prepEntry(spec, make) {
  const i = spec.indexOf(":"), kind = i < 0 ? spec : spec.slice(0, i), key = i < 0 ? "" : spec.slice(i + 1);
  if (kind === "diel") return prepDiel(+key, make);
  if (kind === "mask") return prepMask(make);
  if (kind === "cls") return prepCls(key, make);
  if (kind === "conn") return prepConn(key, make);
  if (kind === "var") return prepVar(key, make);
  if (kind === "dis") return prepDis(key, make);
  if (kind === "fld") { const [r, n] = key.split("|"); return prepFld(r, n); }
  return null;
}

// ---- sources: one per entry; a table's shared source fills the entries entered without one
const prepSrcOk = (s) => !!s && SRC_KINDS.includes(s.kind) && ["document", "url", "note"].some((k) => String(s[k] || "").trim());
function prepSetSrc(e, k, v) {
  const s = { ...(e.source || {}) };
  if (String(v ?? "").trim()) s[k] = v; else delete s[k];
  if (Object.keys(s).length) e.source = s; else delete e.source;
}
function prepAutoSrc(e, key) { const sh = prepD().shared[key]; if (e && !prepSrcOk(e.source) && prepSrcOk(sh)) e.source = prepClone(sh); }
// the entries a table's shared source applies to
function prepTableEntries(key) {
  const w = prepW();
  if (key === "stackup") return [...((w.stackup && w.stackup.dielectrics) || []), ...(w.stackup && w.stackup.solder_mask ? [w.stackup.solder_mask] : [])];
  if (key === "classes") return Object.values(w.net_classes || {});
  if (key === "conn") return Object.values(w.connector_ends || {});
  if (key === "variants") return Object.values(w.variants || {});
  if (key.startsWith("PI_")) return (w.fields || []).filter((f) => String(f.name).toUpperCase() === key);
  return [];
}
function prepSetShared(key, k, v) {
  const d = prepD(), old = d.shared[key] ? prepClone(d.shared[key]) : null;
  const s = { ...(d.shared[key] || {}) };
  if (String(v ?? "").trim()) s[k] = v; else delete s[k];
  if (Object.keys(s).length) d.shared[key] = s; else delete d.shared[key];
  // entries without a source, and those that had the old shared one, follow
  if (prepSrcOk(s)) for (const e of prepTableEntries(key)) if (!prepSrcOk(e.source) || (old && prepSame(e.source, old))) e.source = prepClone(s);
}

// ---- value helpers: PI_SERIES / PI_THERMAL / PI_CAP as key=value tokens, PI_LOAD as rail=current parts
function prepKV(v) {
  const toks = String(v || "").trim().split(/\s+/).filter(Boolean), m = {};
  for (const t of toks) { const i = t.indexOf("="); if (i > 0) m[t.slice(0, i).toLowerCase()] = t.slice(i + 1); }
  return { toks, m };
}
function prepKVSet(v, k, val) {
  const { toks } = prepKV(v), keys = k === "rated" ? ["rated", "v", "vr"] : [k];
  const i = toks.findIndex((t) => keys.some((x) => t.toLowerCase().startsWith(x + "=")));
  const t = String(val ?? "").trim() ? `${k}=${String(val).trim()}` : null;
  if (i >= 0) { if (t) toks[i] = t; else toks.splice(i, 1); } else if (t) toks.push(t);
  return toks.join(" ");
}
const prepKVGet = (v, k) => { const m = prepKV(v).m; return k === "rated" ? (m.rated ?? m.v ?? m.vr ?? "") : (m[k] ?? ""); };
const prepCapDiel = (v) => prepKV(v).toks.find((t) => PREP_DIEL.test(t)) || "";
function prepCapDielSet(v, d) {
  const toks = prepKV(v).toks.filter((t) => !PREP_DIEL.test(t));
  if (String(d || "").trim()) toks.splice(toks.findIndex((t) => t.toLowerCase().startsWith("rated=")) + 1, 0, d.trim());
  return toks.join(" ");
}
function prepSplitQ(v, seps) {
  const out = []; let cur = "", q = false;
  for (const ch of String(v || "")) { if (ch === '"') q = !q; if (!q && seps.includes(ch)) { out.push(cur); cur = ""; } else cur += ch; }
  out.push(cur);
  return out.map((s) => s.trim()).filter(Boolean);
}
// "+3V3=12mA, \"/A B/VDD\"=1mA" → [[net, amount, rest]]
function prepLoadParts(v) {
  return prepSplitQ(v, ",;").map((p) => {
    const m = p.match(/^("([^"]*)"|[^=]+)=(.*)$/);
    return m ? [m[2] ?? m[1].trim(), m[3].trim()] : [p, null];
  });
}
const prepAmps = (t) => { const v = parseSI(t, "A"); return isFinite(v) ? fieldSI(v, "A") : String(t).trim(); };
function prepLoadSet(value, rail, amount) {
  const name = netLabel(rail), parts = prepLoadParts(value);
  const i = parts.findIndex(([n]) => n === name || netShort(n) === netShort(name));
  const t = String(amount ?? "").trim() ? prepAmps(amount) : null;
  if (i >= 0) { if (t) parts[i][1] = t; else parts.splice(i, 1); } else if (t) parts.push([name, t]);
  return parts.map(([n, a]) => a == null ? n : `${fieldNet(n)}=${a}`).join(", ");
}
const prepLoadGet = (value, rail) => { const name = netLabel(rail); const p = prepLoadParts(value).find(([n]) => n === name || netShort(n) === netShort(name)); return p ? p[1] ?? "" : ""; };
const prepList = (t) => String(t || "").split(/[,;\n]/).map((s) => s.trim()).filter(Boolean);

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
// what the check and the gap list see: the draft (null: no configuration at all)
function prepConfigText() { return prepLoadedText() != null || !prepSame(prepW(), prepSkeleton()) ? JSON.stringify(prepW(), null, 2) + "\n" : null; }

// a board was loaded (or the configuration changed): the draft, the gap list for the readiness dots
let prepSeq = 0, prepBaseSeq = 0;
function prepOnBoard() {
  S.prep.d = null; prepD();
  // a board loaded anew starts its checklist at the first open step (not after our own save)
  if (!S.prep.savedNext) for (const t of Object.keys(PREP_VIEWS)) delete S.prep.started[S.name + t];
  S.prep.base = null; S.prep.cur = null; S.prep.note = ""; S.prep.saved = S.prep.savedNext || ""; S.prep.savedNext = ""; S.prep.dismissing = null; S.prep.pend = {}; S.prep.takeNote = "";
  S.prep.sheets = null; S.prep.inflight = false; S.prep.again = false; S.prep.busy = false;
  clearTimeout(prepTimer); S.prep.queued = false;
  const seq = ++prepBaseSeq, name = S.name;
  prepSeq++; // results of the board before are dropped
  prepGaps(prepLoadedText()).then((g) => {
    if (seq !== prepBaseSeq || name !== S.name) return;
    S.prep.base = g;
    if (prepDirty()) prepUpdate(true); else { S.prep.cur = g; prepRender(true); }
  }).catch((e) => { console.warn(e); S.prep.note = "The gap list could not be computed: " + e.message; prepRender(true); });
  prepCheck();
  prepRender(true);
}
// after an edit: the check at once, the gap list with the draft a little later
let prepTimer = null;
function prepUpdate(now) {
  prepCheck();
  clearTimeout(prepTimer); S.prep.queued = false;
  // one gap list at a time: edits while it runs ask for one more afterwards
  const run = () => {
    if (S.prep.inflight) { S.prep.again = true; return; }
    const seq = ++prepSeq, name = S.name;
    S.prep.busy = true; S.prep.inflight = true; S.prep.again = false; prepRenderStatus();
    const done = () => { S.prep.inflight = false; if (S.prep.again && name === S.name) { S.prep.again = false; run(); } };
    prepGaps(prepConfigText()).then((g) => { done(); if (seq === prepSeq && name === S.name && !S.prep.inflight) { S.prep.cur = g; S.prep.busy = false; prepAfterApply(); prepRender(true); } })
      .catch((e) => { console.warn(e); done(); if (!S.prep.inflight) { S.prep.busy = false; prepRender(true); } });
  };
  if (now) run(); else { S.prep.queued = true; prepTimer = setTimeout(() => { S.prep.queued = false; run(); }, 700); }
}
function prepCheck() {
  const t = prepConfigText();
  if (!t || !S.scene) { S.prep.check = null; return; }
  try { S.prep.check = S.engine.configCheck(prepBoardFile(), t); } catch (e) { S.prep.check = { ok: false, entries: 0, errors: [e.message], warnings: [], messages: [{ level: "error", text: e.message, views: Object.keys(PREP_VIEWS) }] }; }
}
// an edit happened: store, check, and the gap list later; the forms are drawn again only when
// `full` (a change, not every key stroke)
// (a change redraws after the event, so that a click into another field has moved the focus there first)
let prepDrawTimer = null;
function prepEdited(full) {
  S.prep.saved = ""; prepPersist(); prepUpdate(false);
  prepRender(false);
  if (full) { clearTimeout(prepDrawTimer); prepDrawTimer = setTimeout(() => prepRender(true), 0); }
}

// ---- state of a gap and of a category
const prepGapsNow = () => (S.prep.cur || S.prep.base || { gaps: [] }).gaps;
const prepViewsNow = () => (S.prep.cur || S.prep.base || {}).views || null;
// items of a gap for a tab: those with their own categories only where they include it
const prepItems = (g, tab) => g.items.filter((it) => !Array.isArray(it.views) || it.views.includes(tab));
function prepGapState(g, tab) {
  const now = prepGapsNow().find((x) => x.id === g.id), dis = ((S.prep.cur || {}).dismissed || []).filter((x) => x.gap === g.id && x.items.length);
  const n = (x) => { const its = tab ? prepItems(x, tab) : x.items; return x.id === "nets-without-target" ? its.reduce((s, i) => s + (i.nets || []).length, 0) : its.length; };
  if (!now || (tab && g.items.length && !n(now))) return dis.length ? { cls: "dis", text: "dismissed" } : { cls: "done", text: "✓ filled" };
  const left = n(now), all = n(g);
  if (left < all) return { cls: "", text: `${left} of ${all} left` };
  return { cls: "", text: all ? `${all} item${all === 1 ? "" : "s"}` : "open" };
}
const prepAgentWhole = (g) => PREP_MODE[g.id] === "agent" || !!prepD().agent[g.id];
// the items of a gap one by one, as the agent gets them: a key, a label (refdes, net, rail, class, layer)
// and what identifies it (MPN or value, rails); nets without a target count per net
function prepUnits(g, tab) {
  const its = tab ? prepItems(g, tab) : g.items;
  if (g.id === "nets-without-target") return its.flatMap((i) => (i.nets || []).map((x) => ({ key: x.net, label: netShort(x.net), detail: x.looks_like ? `looks like ${x.looks_like}` : "" })));
  return its.map((it) => {
    const label = it.ref || (it.index != null ? `#${it.index} ${it.between || ""}`.trim() : "") || it.class || (it.rail ? netShort(it.rail) : "") || it.fork || it.variant || "";
    const rails = it.rails ? it.rails.map(netShort).join(", ") : it.rail && it.ref ? netShort(it.rail) : "";
    const what = it.mpn || it.value || (it.why ? it.why.map((w) => w.split(" ")[0]).join(", ") : "");
    return { key: label, label, detail: [what, rails && `on ${rails}`].filter(Boolean).join(" ") };
  }).filter((u) => u.key);
}
const prepHandedKeys = (g) => prepD().agentItems[g.id] || [];
// the items of a gap with the agent now: all (the step handed over) or those handed one by one, among the
// items the core still reports
function prepHanded(g, tab) {
  const now = prepGapsNow().find((x) => x.id === g.id);
  if (!now) return [];
  const us = prepUnits(now, tab);
  if (prepAgentWhole(g)) return us;
  const ks = new Set(prepHandedKeys(g));
  return us.filter((u) => ks.has(u.key));
}
// a step is with the agent when handed over as a whole, or when every item still open is handed over
const prepAgentOf = (g, tab) => {
  if (prepAgentWhole(g)) return true;
  if (!prepHandedKeys(g).length) return false;
  const now = prepGapsNow().find((x) => x.id === g.id), us = now ? prepUnits(now, tab ?? S.prep.tab) : [];
  return us.length > 0 && prepHanded(g, tab ?? S.prep.tab).length === us.length;
};
// the steps of a tab: the gaps of the loaded file, then gaps the draft brought up (a power tree brings the
// efficiency and thermal gaps), by priority, otherwise in the core's order
function prepSteps(tab) {
  const base = (S.prep.base || { gaps: [] }).gaps, extra = prepGapsNow().filter((g) => !base.some((x) => x.id === g.id));
  return base.concat(extra).filter((g) => g.views.includes(tab) && prepItems(g, tab).length + (g.items.length ? 0 : 1) > 0)
    .map((g, i) => [g, i]).sort((a, b) => PRIO_RANK[a[0].priority] - PRIO_RANK[b[0].priority] || a[1] - b[1]).map((x) => x[0]);
}
// a step: open, done (the core no longer reports it for this tab), dismissed, or with the agent
function prepStepState(g, tab) {
  const st = prepGapState(g, tab);
  if (st.cls === "done") return { k: "done", text: "done" };
  if (st.cls === "dis") return { k: "dis", text: "dismissed" };
  if (prepAgentOf(g, tab)) return { k: "agent", text: "with the agent" };
  return { k: "open", text: st.text };
}
const prepRequired = (g) => g.priority !== "low";
// per tab: required steps (high, medium) done / with the agent / open, and the optional ones
function prepProgress(tab) {
  const x = prepViewsNow() && prepViewsNow()[tab];
  const steps = prepSteps(tab), req = steps.filter(prepRequired);
  const ks = req.map((g) => prepStepState(g, tab).k);
  return { na: !!x && !x.applicable, ready: !!x && x.state === "ready", req: req.length, done: ks.filter((k) => k === "done" || k === "dis").length,
    agent: ks.filter((k) => k === "agent").length, open: ks.filter((k) => k === "open").length, opt: steps.length - req.length, known: !!x };
}
function prepStateText(v) {
  const p = prepProgress(v);
  if (!p.known) return "…";
  if (p.na) return "nothing to check";
  if (!p.req) return "ready";
  return `${p.done} of ${p.req} done${p.agent ? ` · ${p.agent} with the agent` : ""}`;
}
// the marker of a category: a check mark when nothing required is open (the agent's items aside)
function prepMark(v) {
  const p = prepProgress(v);
  if (!p.known) return { cls: "", t: "…" };
  if (p.na) return { cls: "na", t: "–" };
  if (!p.open) return { cls: p.agent ? "agent" : "done", t: p.agent ? "↗" : "✓" };
  const hi = prepSteps(v).some((g) => g.priority === "high" && prepStepState(g, v).k === "open");
  return { cls: "open " + (hi ? "h" : "m"), t: String(p.open) };
}
// the badge and the status card: open required steps of the categories with something to check
function prepCount() {
  if (!prepViewsNow()) return null;
  const ids = new Set();
  for (const v of Object.keys(PREP_VIEWS)) { const p = prepProgress(v); if (!p.na) for (const g of prepSteps(v)) if (prepRequired(g) && prepStepState(g, v).k === "open") ids.add(g.id); }
  return ids.size;
}
function prepRenderStatus() {
  for (const [t, v] of Object.entries(PREP_OF_TAB)) $("view-" + t).title = prepViewsNow() ? `${PREP_VIEWS[v]}: ${prepStateText(v)} (Prepare PCB)` : "";
  const n = prepCount(), b = $("prep-count");
  if (b) { b.hidden = n == null; b.textContent = n ?? ""; const hi = Object.keys(PREP_VIEWS).some((v) => prepMark(v).cls === "open h"); b.className = "count " + (hi ? "h" : n ? "m" : "z"); }
  const st = $("prep-busy"); if (st) st.hidden = !S.prep.busy;
}
// ---- form pieces
// what a step still needs is marked on its inputs (prep-need: the open values of an open step – εr/tanδ, target Ω,
// currents, ratings …); the source of an entry only once the entry has a value (prepSrcCell). PREP_MARKING is set
// while an open step's form is drawn (prepCard), so the forms of the "In kisi.json" section stay unmarked.
let PREP_MARKING = false;
const prepBlank = (v) => v == null || String(v).trim() === "";
const prepNeed = (empty) => (PREP_MARKING && empty ? " prep-need" : "");
// an entry with a value of its own (not only its source or its name)
const PREP_NOT_VALUES = ["source", "ref", "name", "nets", "diff", "index"];
const prepHasValue = (e) => !!e && Object.entries(e).some(([k, v]) => !PREP_NOT_VALUES.includes(k) && (v && typeof v === "object" ? Object.keys(v).length > 0 : !prepBlank(v)));
const prepKindOpts = (cur) => `<option value="">kind…</option>${SRC_KINDS.map((k) => `<option ${cur === k ? "selected" : ""}>${k}</option>`).join("")}`;
// the source of one entry: kind, document; url, page and note behind "…"
function prepSrcCell(spec, e) {
  const s = (e && e.source) || {}, ok = prepSrcOk(s) || !prepHasValue(e);
  const a = `data-a="src" data-e="${esc(spec)}"`;
  const more = ["url", "page", "note"].filter((k) => s[k]).length;
  return `<div class="prep-srccell${ok ? "" : " miss"}"><select ${a} data-k="kind" aria-label="source kind" ${e ? "" : "disabled"}>${prepKindOpts(s.kind)}</select><input ${a} data-k="document" value="${esc(s.document || "")}" placeholder="${e ? "document" : "source after the value"}" aria-label="source document" ${e ? "" : "disabled"}>
    <details class="prep-srcmore"${S.prep.openSrc === spec ? " open" : ""}><summary title="url, page, note">…${more ? `<b>${more}</b>` : ""}</summary><div>${["url", "page", "note"].map((k) => `<input ${a} data-k="${k}" value="${esc(s[k] || "")}" placeholder="${k === "note" ? "note: typical / maximum, condition" : k}" aria-label="source ${k}" ${e ? "" : "disabled"}>`).join("")}</div></details></div>`;
}
function prepSharedBlock(key, label) {
  const s = prepD().shared[key] || {};
  const a = `data-a="shared" data-t="${esc(key)}"`;
  return `<div class="prep-src" data-shared="${esc(key)}"><b>${esc(label || "Source for the rows you fill in here")} – each value needs one (rows with their own keep it)</b>
    <select ${a} data-k="kind" aria-label="shared source kind">${prepKindOpts(s.kind)}</select>
    <input ${a} data-k="document" placeholder="document or url" value="${esc(s.document || "")}" aria-label="shared source document">
    <input ${a} data-k="note" placeholder="note: typical / maximum, condition" value="${esc(s.note || "")}" aria-label="shared source note"></div>`;
}
const prepTd = (label, html, cls) => `<td data-l="${esc(label)}"${cls ? ` class="${cls}"` : ""}>${html}</td>`;
const prepIn = (attrs, v, ph, cls) => `<input ${attrs} value="${esc(v ?? "")}" placeholder="${esc(ph ?? "")}"${cls ? ` class="${cls}"` : ""}>`;

// the dielectrics of the board (all, or the gap's), with the draft's values
function prepStackTable(idx, flagged, key) {
  const st = S.scene.stackup, w = prepW();
  const rows = idx.map((i) => {
    const d = st.dielectrics[i] || {}, e = prepDiel(i, false) || {}, why = (flagged && flagged[i]) || [];
    const wEr = why.some((x) => /εr|epsilon/i.test(x)), wTan = why.some((x) => /tanδ|loss/i.test(x));
    const a = (k) => `data-a="diel" data-i="${i}" data-k="${k}"`;
    const between = `${(st.copper[i] || {}).name || "?"} / ${(st.copper[i + 1] || {}).name || "?"}`;
    return `<tr class="${why.length ? "flag" : ""}"><td class="m" data-l="index">${i}</td>${prepTd("between", `${esc(between)}<span class="sub">${esc(d.kind || "")}${why.length ? ` · ${esc(why.join(", "))}` : ""}</span>`)}
      ${prepTd("material", prepIn(a("material"), e.material, d.material || ""), "opt")}
      ${prepTd("εr", prepIn(a("epsilon_r"), e.epsilon_r, fmtSig(d.epsilon_r, 3), "num" + prepNeed(wEr && prepBlank(e.epsilon_r))))}
      ${prepTd("tanδ", prepIn(a("loss_tangent"), e.loss_tangent, fmtSig(d.loss_tangent, 3), "num" + prepNeed(wTan && prepBlank(e.loss_tangent))))}
      ${prepTd("thickness mm", prepIn(a("thickness"), e.thickness, fmtSig(d.thickness, 3), "num"), "opt")}
      ${prepTd("source", prepSrcCell(`diel:${i}`, prepDiel(i, false)))}</tr>`;
  });
  if (!flagged) {
    const m = (w.stackup && w.stackup.solder_mask) || {};
    rows.push(`<tr><td class="m" data-l="index">mask</td>${prepTd("between", `solder mask<span class="sub">both sides</span>`)}<td class="opt"></td>${prepTd("εr", prepIn(`data-a="mask" data-k="epsilon_r"`, m.epsilon_r, "3.8", "num"))}<td></td><td class="opt"></td>${prepTd("source", prepSrcCell("mask", w.stackup && w.stackup.solder_mask))}</tr>`);
  }
  return `<table class="prep-ed"><thead><tr><th>#</th><th>Between</th><th class="opt">Material</th><th>εr</th><th>tanδ</th><th class="opt">Thickness mm</th><th>Source</th></tr></thead><tbody>${rows.join("")}</tbody></table>${prepSharedBlock(key || "stackup", "Source for the laminate values you enter (datasheet, fabricator's stackup)")}`;
}
// net classes: targets (impedance) or profiles (channel)
function prepClassTable(names, tab, hint) {
  const cls = (n) => (S.an.classes || []).find((c) => c.name === n) || {};
  const rows = names.map((n) => {
    const c = cls(n), e = prepCls(n, false) || {}, t = c.target, a = (k) => `data-a="cls" data-c="${esc(n)}" data-k="${k}"`;
    const h = (hint && hint[n]) || {};
    const diff = e.diff ?? (t ? t.diff : h.diff ?? false);
    const head = prepTd("class", `${esc(n)}<span class="sub">${c.nets ?? h.nets ?? 0} nets${h.examples ? " · " + esc(h.examples.slice(0, 2).map(netShort).join(", ")) : ""}</span>`, "m");
    if (tab === "impedance") {
      return `<tr>${head}${prepTd("target Ω", prepIn(a("target_ohm"), e.target_ohm, t ? fmtSig(t.z, 3) : "–", "num" + prepNeed(!t && prepBlank(e.target_ohm))))}
        ${prepTd("differential", `<input type="checkbox" ${a("diff")} ${diff ? "checked" : ""} aria-label="differential">`)}${prepTd("source", prepSrcCell(`cls:${n}`, prepCls(n, false)))}</tr>`;
    }
    // the generic profiles last: chosen on purpose for signals without an interface standard (analog, RF)
    const ord = [...IFC.profiles.filter((p) => !["se50", "diff100"].includes(p.id)), ...IFC.profiles.filter((p) => ["se50", "diff100"].includes(p.id))];
    const opts = ord.map((p) => `<option value="${p.id}" ${e.profile === p.id ? "selected" : ""}>${esc(p.name)}${["se50", "diff100"].includes(p.id) ? " (on purpose)" : ""}${(p.signal === "diff") !== diff ? " – " + p.signal : ""}</option>`).join("");
    const now = h.profile_now || (typeof cfgProfile === "function" && cfgProfile(n)) || "generic";
    return `<tr>${head}${prepTd("profile", `<select ${a("profile")} class="${prepNeed(!e.profile).trim()}"><option value="">${esc(e.profile && !IFC.profiles.some((p) => p.id === e.profile) ? e.profile : now)} (now)</option>${opts}</select>`)}
      ${prepTd("differential", diff ? "diff" : "single-ended")}${prepTd("source", prepSrcCell(`cls:${n}`, prepCls(n, false)))}</tr>`;
  });
  return `<table class="prep-ed"><thead><tr><th>Class</th>${tab === "impedance" ? "<th>Target Ω</th><th>Diff</th>" : "<th>Profile</th><th></th>"}<th>Source</th></tr></thead><tbody>${rows.join("")}</tbody></table>${prepSharedBlock("classes", "Source for the class values you enter (standard, datasheet)")}`;
}
// nets in "Default" that look like an interface: a class of their own in kisi.json (net_classes.<name>.nets)
function prepNetClassOf(g, nets) {
  const w = prepW();
  return Object.keys(w.net_classes || {}).find((k) => (w.net_classes[k].nets || []).some((p) => nets.includes(p))) || null;
}
function prepSuggestClass(nets) {
  const short = nets.map((n) => netShort(n));
  let pre = short[0] || "";
  for (const x of short) while (pre && !x.startsWith(pre)) pre = pre.slice(0, -1);
  pre = pre.replace(/[_\-\d]+$/, "").replace(/_?D[PNQ]?$/, "") || "Interface";
  return pre.replace(/[^A-Za-z0-9_+-]/g, "_");
}
function prepNetTargetEditor(g, items) {
  const groups = items.map((it) => (it.nets || []).map((n) => n.net)).filter((x) => x.length);
  return groups.map((nets, gi) => {
    const key = `${g.id}#${gi}`, cls = prepNetClassOf(g, nets), e = cls ? prepW().net_classes[cls] : null;
    const name = cls || S.prep.ncName[S.name + key] || prepSuggestClass(nets);
    const chosen = e ? nets.filter((n) => (e.nets || []).includes(n)) : (S.prep.ncNets[S.name + key] || nets);
    const extra = e ? (e.nets || []).filter((x) => !nets.includes(x)) : [];
    const looks = ((items[gi].nets || [])[0] || {}).looks_like, lp = IFC.profiles.find((p) => p.id === looks);
    const diff = e && e.diff != null ? e.diff : !!(lp && lp.signal === "diff");
    const a = (k) => `data-a="nc" data-g="${esc(key)}" data-n="${esc(name)}" data-k="${k}"`;
    const opts = IFC.profiles.map((p) => `<option value="${p.id}" ${(e && e.profile) === p.id ? "selected" : ""}>${esc(p.name)}</option>`).join("");
    return `<div class="prep-nc"><div class="prep-row"><label>New class <input ${a("name")} value="${esc(name)}" aria-label="class name" class="ref"></label>
        <span class="note">in “${esc(items[gi].class)}” on the board; kisi.json gives these nets to it (<code>net_classes.${esc(name)}.nets</code>), KiCad stays unchanged</span></div>
      <div class="prep-row">${nets.map((n) => `<label class="prep-chk"><input type="checkbox" data-a="ncnet" data-g="${esc(key)}" data-n="${esc(name)}" data-net="${esc(n)}" ${chosen.includes(n) ? "checked" : ""}> <code>${esc(netShort(n))}</code></label>`).join("")}
        <label>more nets or patterns ${prepIn(a("extra"), extra.join(", "), "QSPI_*, USB_D?", "grow")}</label></div>
      <div class="prep-row"><label>target Ω ${prepIn(a("target_ohm"), e ? e.target_ohm : "", lp && lp.signal === "diff" ? "90" : "50", "num" + prepNeed(!e || prepBlank(e.target_ohm)))}</label>
        <label class="prep-chk"><input type="checkbox" ${a("diff")} ${diff ? "checked" : ""}> differential</label>
        <label>profile <select ${a("profile")}><option value="">${esc(looks ? `${looks}? (from the names)` : "generic")}</option>${opts}</select></label></div>
      <div class="src">${prepSrcCell(`cls:${name}`, e)}</div></div>`;
  }).join("") + prepSharedBlock("classes", "Source for the class targets (interface standard, datasheet, layout guide)");
}
// the core's net matching for the forms (full name or its last part; * and ?)
function net_matches_js(pats, name) {
  const short = String(name).split("/").pop();
  const wc = (p, x) => new RegExp("^" + p.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$").test(x);
  return (pats || []).some((p) => p === name || wc(p, name) || wc(p, short));
}

// fields of one name: every entry, and an empty row for a new one
function prepFieldTable(name, ph, refs) {
  const fs = (prepW().fields || []).filter((f) => String(f.name).toUpperCase() === name && (!refs || refs.includes(f.ref)));
  const rows = fs.map((f) => {
    const a = `data-r="${esc(f.ref)}" data-n="${name}"`;
    return `<tr>${prepTd("part", `<input class="ref" data-a="fldref" ${a} value="${esc(f.ref)}" aria-label="part">`)}
      ${prepTd(name, `<input class="wide" data-a="fld" ${a} data-k="value" value="${esc(f.value)}" aria-label="${name} value">`)}
      ${prepTd("source", prepSrcCell(`fld:${f.ref}|${name}`, f))}<td><button class="btn small" data-b="del-fld" ${a} title="remove">×</button></td></tr>`;
  });
  const p = S.prep.pend[name] || {};
  rows.push(`<tr class="new">${prepTd("part", `<input class="ref" data-a="new" data-n="${name}" data-k="ref" value="${esc(p.ref || "")}" placeholder="new: U3" aria-label="new part">`)}
    ${prepTd(name, `<input class="wide${prepNeed(prepBlank(p.value))}" data-a="new" data-n="${name}" data-k="value" value="${esc(p.value || "")}" placeholder="${esc(ph)}" aria-label="new ${name} value">`)}
    ${prepTd("source", `<span class="note">after the value</span>`)}<td></td></tr>`);
  return `<table class="prep-ed fields"><thead><tr><th>Part</th><th>${name}</th><th>Source</th><th></th></tr></thead><tbody>${rows.join("")}</tbody></table>${prepSharedBlock(name)}`;
}
function prepRegSuggestions(g) {
  return g.items.filter((it) => it.write && it.write.startsWith("PI_REG on ")).map((it) => {
    const m = it.write.match(/^PI_REG on (\S+): (.*)$/);
    return m ? `<button class="prep-sugg${prepNeed(true)}" data-b="sugg-reg" data-r="${esc(m[1])}" data-val="${esc(m[2])}">+ ${esc(m[1])}: ${esc(m[2])}</button>` : "";
  }).join("");
}
// capacitors grouped by MPN (or value and package): one row writes PI_CAP on all of them
function prepCapGroups(items) {
  const gs = new Map();
  for (const it of items) {
    const k = it.mpn ? `${it.mpn}` : `${it.value} ${it.package || ""}`.trim();
    if (!gs.has(k)) gs.set(k, { key: k, items: [], mpn: it.mpn, value: it.value, package: it.package, manufacturer: it.manufacturer, suggest: it.suggest });
    gs.get(k).items.push(it);
  }
  return [...gs.values()].sort((a, b) => b.items.length - a.items.length);
}
function prepCapsEditor(g, tab) {
  // the capacitors the core reports now (after a variant choice, the fitted ones), and those filled in already
  const now = S.prep.cur ? prepGapsNow().find((x) => x.id === g.id) : g;
  const filled = new Set((prepW().fields || []).filter((f) => String(f.name).toUpperCase() === "PI_CAP").map((f) => f.ref));
  const seen = new Set(), pool = [...(now ? prepItems(now, tab) : []), ...prepItems(g, tab).filter((it) => filled.has(it.ref))].filter((it) => !seen.has(it.ref) && seen.add(it.ref));
  const groups = prepCapGroups(pool);
  S.prep.capGroups = Object.fromEntries(groups.map((x) => [x.key, x.items.map((i) => i.ref)]));
  const rows = groups.map((x) => {
    const refs = x.items.map((i) => i.ref), f = prepFld(refs[0], "PI_CAP"), v = f ? f.value : "";
    const a = (k) => `data-a="capgrp" data-g="${esc(x.key)}" data-k="${k}"`;
    const rails = [...new Set(x.items.map((i) => i.rail))].slice(0, 3).map(netShort).join(", ");
    const sugg = x.suggest ? `<button class="prep-sugg" data-b="cap-sugg" data-g="${esc(x.key)}" data-val="${esc(x.suggest.value)}" data-from="${esc((x.suggest.from || []).join(", "))}" title="from the parts' own fields: ${esc((x.suggest.from || []).join(", "))}">${esc(x.suggest.value)}</button>` : "";
    return `<tr>${prepTd("parts", `${esc(x.mpn || x.value)}<span class="sub">${refs.length} × ${esc(x.value)}${x.package ? " " + esc(x.package) : ""} · ${esc(rails)}</span><details class="prep-refs"><summary>${esc(refs.slice(0, 4).join(" "))}${refs.length > 4 ? " …" : ""}</summary>${esc(refs.join(" "))}</details>`, "m")}
      ${prepTd("rated V", prepIn(a("rated"), prepKVGet(v, "rated"), "6.3V", "num" + prepNeed(prepBlank(prepKVGet(v, "rated")))))}
      ${prepTd("dielectric", `<select ${a("diel")} class="${prepNeed(!prepCapDiel(v)).trim()}"><option value=""></option>${["X5R", "X7R", "X6S", "X7S", "X8R", "C0G", "Y5V", "POLYMER", "TANTALUM"].map((d) => `<option ${prepCapDiel(v).toUpperCase() === d ? "selected" : ""}>${d}</option>`).join("")}</select>`)}
      ${prepTd("ESR Ω", prepIn(a("esr"), prepKVGet(v, "esr"), "5m", "num"), "opt")}${prepTd("ESL H", prepIn(a("esl"), prepKVGet(v, "esl"), "0.45n", "num"), "opt")}
      ${prepTd("from the parts' fields", sugg || `<span class="note">–</span>`)}</tr>`;
  });
  return `<p class="note">One row per part number (or value and package): the values go as PI_CAP on every capacitor of the group. A suggestion comes from the parts' own fields (Voltage Rating, Dielectric) – kind <code>design</code>.</p>
    <table class="prep-ed"><thead><tr><th>Parts</th><th>Rated V</th><th>Dielectric</th><th class="opt">ESR</th><th class="opt">ESL</th><th>Suggested</th></tr></thead><tbody>${rows.join("")}</tbody></table>${prepSharedBlock("PI_CAP", "Source for the capacitor values (datasheet, vendor DC-bias curve, the BOM)")}`;
}
// connectors where lanes leave the board (and those in the draft)
function prepConnectorRefs() {
  const refs = new Set(Object.keys(prepW().connector_ends || {}));
  for (const l of (typeof allLanes === "function" ? allLanes() : [])) { const r = laneConnRef(l); if (r) refs.add(r); }
  return [...refs].sort();
}
function prepConnEditor() {
  const refs = prepConnectorRefs();
  if (!refs.length) return `<p class="note">No lane ends at a connector on this board (or the lanes are not computed yet – choose an assembly variant first).</p>`;
  const tsNames = Object.keys(S.tsFiles || {});
  const rows = refs.map((r) => {
    const e = prepConn(r, false) || {}, t = e.touchstone || {}, a = (k) => `data-a="conn" data-r="${esc(r)}" data-k="${k}"`;
    const ports = Object.entries(t.ports || {}).map(([p, [x, y]]) => `${p}:${x}/${y}`).join(", ");
    return `<div class="prep-conn"><b class="m">${esc(r)}</b>
      <label>model <select ${a("model")}><option value=""></option>${["profile", "matched", "high_impedance", "custom"].map((m) => `<option ${e.model === m ? "selected" : ""}>${m}</option>`).join("")}</select></label>
      <label>R Ω ${prepIn(a("r_ohm"), e.r_ohm, "50", "num")}</label><label>C pF ${prepIn(a("c_pf"), e.c_pf, "1", "num")}</label><label>L nH ${prepIn(a("l_nh"), e.l_nh, "", "num")}</label>
      <label>channel behind: loss dB ${prepIn(a("rest_loss_db"), e.rest_loss_db, "3", "num")}</label><label>delay ps ${prepIn(a("rest_delay_ps"), e.rest_delay_ps, "500", "num")}</label>
      <label>Touchstone ${prepIn(a("ts_file") + ` list="prep-ts-files"`, t.file, "cable.s4p")}</label>
      <label>port order <select ${a("ts_order")}><option value=""></option>${["1-3,2-4", "1-2,3-4"].map((o) => `<option ${t.port_order === o ? "selected" : ""}>${o}</option>`).join("")}</select></label>
      <label>ports per pin ${prepIn(a("ts_ports"), ports, "A2:1/2, A3:3/4")}</label>
      <div class="src">${prepSrcCell(`conn:${r}`, prepConn(r, false))}</div></div>`;
  });
  return `<datalist id="prep-ts-files">${tsNames.map((n) => `<option value="${esc(n)}">`).join("")}</datalist>${rows.join("")}
    <p class="note">The end at each connector (termination when it receives, driver when it sends) and the channel behind it – a lossy line or a Touchstone file (loaded in a bus card of the Channel view or dropped with the board; the path is relative to the kisi.json). Ports: connector pin → file port at the connector / at the far end.</p>${prepSharedBlock("conn", "Source for the connector ends")}`;
}
function prepSheets() {
  if (!S.prep.sheets) { try { S.prep.sheets = S.engine.boardSheets(); } catch (e) { S.prep.sheets = []; } }
  return S.prep.sheets;
}
// the variant to analyse and the variants defined in kisi.json
function prepVariantEditor(g) {
  const w = prepW(), board = ((S.variant && S.variant.variants) || []).filter((v) => v.defined_in !== "kisi.json").map((v) => v.name);
  const names = [...new Set([...board, ...Object.keys(w.variants || {})])];
  const forks = g ? g.items.filter((it) => it.fork) : [];
  const sheets = prepSheets();
  const defs = Object.entries(w.variants || {}).map(([n, v]) => {
    const a = (k) => `data-a="vdef" data-v="${esc(n)}" data-k="${k}"`, l = (k) => (v[k] || []).join(", ");
    return `<div class="prep-vdef"><div class="prep-row"><b>${esc(n)}</b>${prepIn(a("description"), v.description, "description", "grow")}<button class="btn small" data-b="del-var" data-v="${esc(n)}" title="remove">×</button></div>
      <label>not fitted: sheets ${prepIn(a("not_fitted_sheets") + ` list="prep-sheets"`, l("not_fitted_sheets"), "/Option B/")}</label>
      <label>fitted: sheets ${prepIn(a("fitted_sheets") + ` list="prep-sheets"`, l("fitted_sheets"), "")}</label>
      <label>not fitted: parts ${prepIn(a("not_fitted"), l("not_fitted"), "R26, R28")}</label><label>fitted: parts ${prepIn(a("fitted"), l("fitted"), "C101")}</label>
      <label>BOM file ${prepIn(a("bom"), v.bom, "board_A_BOM.csv")}</label><label>BOM exceptions ${prepIn(a("bom_exceptions"), l("bom_exceptions"), "P1")}</label>
      <div class="src">${prepSrcCell(`var:${n}`, v)}</div></div>`;
  }).join("");
  return `${forks.length ? `<p class="note">Forks where all alternative parts are fitted (${forks.length}): ${esc(forks.slice(0, 4).map((f) => f.fork).join("; "))}${forks.length > 4 ? " …" : ""}</p>` : ""}
    <div class="prep-row${prepNeed(!w.variant)}" role="radiogroup" aria-label="variant to analyse">${["default", ...names].map((n) => `<label class="prep-chk"><input type="radio" name="prep-var" value="${esc(n)}" data-a="variant" ${(w.variant || "") === n ? "checked" : ""}> <b>${esc(n)}</b>${n === "default" ? " – the board file as it is" : ""}</label>`).join("")}
      ${w.variant ? `<button class="btn small" data-b="no-variant" title="no variant in kisi.json">clear</button>` : ""}</div>
    <datalist id="prep-sheets">${sheets.map((s) => `<option value="${esc(s.sheet)}">${s.parts} parts</option>`).join("")}</datalist>
    ${defs}<div class="prep-row"><input id="prep-newvar" placeholder="new variant name" aria-label="new variant name"><button class="btn small" data-b="add-var">+ Define a variant</button>
    <span class="note">by sheets (${sheets.length} on this board), parts or a BOM; lists comma-separated</span></div>${Object.keys(w.variants || {}).length ? prepSharedBlock("variants", "Source for the variant definitions (BOM, position files)") : ""}`;
}

// the form of an open step marks what it needs (PREP_MARKING)
function prepEditorMarked(g, gs, tab, open) {
  PREP_MARKING = open;
  try { return prepEditor(g, gs, tab); } finally { PREP_MARKING = false; }
}
// the form of a gap (its items)
function prepEditor(g, gs, tab) {
  const mode = PREP_MODE[g.id], items = prepItems(g, tab);
  if (mode === "stackup") {
    const flag = {}; for (const it of items) flag[it.index] = it.why.map((x) => x.replace(" is KiCad's default value", ": default"));
    return prepStackTable(items.map((it) => it.index), flag);
  }
  if (mode === "nettarget") return prepNetTargetEditor(g, items);
  if (mode === "classes") return prepClassTable(items.map((it) => it.class), "channel", Object.fromEntries(items.map((it) => [it.class, it])));
  if (mode === "variant") return prepVariantEditor(g);
  if (mode === "reg") {
    const first = gs.find((x) => PREP_MODE[x.id] === "reg"), sugg = prepRegSuggestions(g);
    const suggHtml = sugg ? `<div class="prep-row">${sugg}</div><p class="note">Suggested by <code>kisi gaps</code> (a rail behind a ferrite or link takes the voltage before it); a click adds the entry – then give r= and a source.</p>` : "";
    const list = items.slice(0, 12).map((it) => esc(it.ref ? `${it.ref} ${it.from || ""} → ${it.to || ""}` : netShort(it.rail) + (it.voltage ? ` (${fmtSI(it.voltage, "V")})` : ""))).join(", ");
    const more = items.length > 12 ? ` … ${items.length - 12} more` : "";
    if ((first && first.id !== g.id) || g.priority === "low") {
      if (g.priority === "low" && (!first || first.id === g.id)) return `<p class="note">${list}${more}. Entered in the regulator chain – the table “Regulator chain (PI_REG)” under “In ${esc(S.name)}.kisi.json” below (the efficiency after the type, e.g. buck 88%).</p>${suggHtml}`;
      return `<p class="note">${list}${more}. Entered in the regulator chain ${g.id === "rails-without-voltage" ? "as <code>NAME@&lt;V&gt;V</code> where the rail appears" : g.id === "regulator-efficiency" ? "(the efficiency after the type, e.g. buck 88%)" : "(an input or a regulator output)"} – the table in “${esc(PREP_TITLES[first.id])}” above.</p>${suggHtml}`;
    }
    return `${list ? `<p class="note">${list}${more}</p>` : ""}${prepFieldTable("PI_REG", "+12V > +3V3 buck 90% 3A")}
      <p class="note">PI_REG: <code>input &gt; output kind values</code> – kinds input, buck, boost, buck-boost, sepic, inverter, ldo, pass; values 90%, 3A, iq=, r=, out=, id=; one PMIC channel per <code>;</code>; a rail without a voltage in its name as <code>NAME@3.3V</code>.</p>${suggHtml}`;
  }
  if (mode === "loads") {
    const rows = items.map((it) => {
      const f = prepFld(it.ref, "PI_LOAD"), v = f ? f.value : "", basis = f && f.source && /\bmaximum\b/.test(f.source.note || "") ? "maximum" : f && f.source && /\btypical\b/.test(f.source.note || "") ? "typical" : "";
      return `<tr>${prepTd("IC", `${esc(it.ref)}<span class="sub">${esc(it.mpn ? `${it.mpn} (${it.manufacturer || it.value})` : it.value || "")}</span>`, "m")}
        ${prepTd("current per rail", (it.rails || []).map((r) => `<div class="prep-rail"><span class="m">${esc(netShort(r))}</span><input class="num${prepNeed(prepBlank(prepLoadGet(v, r)))}" data-a="load" data-r="${esc(it.ref)}" data-rail="${esc(r)}" value="${esc(prepLoadGet(v, r))}" placeholder="mA"></div>`).join(""))}
        ${prepTd("basis", `<select data-a="load" data-r="${esc(it.ref)}" data-k="basis"><option value="">–</option><option ${basis === "typical" ? "selected" : ""}>typical</option><option ${basis === "maximum" ? "selected" : ""}>maximum</option></select>`, "opt")}
        ${prepTd("source", prepSrcCell(`fld:${it.ref}|PI_LOAD`, f))}</tr>`;
    }).join("");
    return `<table class="prep-ed"><thead><tr><th>IC</th><th>Current per rail</th><th class="opt">Basis</th><th>Source</th></tr></thead><tbody>${rows}</tbody></table>${prepSharedBlock("PI_LOAD", "Source for the load currents")}`;
  }
  if (mode === "series" || mode === "thermal") {
    const name = mode === "series" ? "PI_SERIES" : "PI_THERMAL", keys = PREP_KV[name];
    const heads = mode === "series" ? ["Z at 100 MHz (Ω)", "DC resistance (Ω)"] : ["θJB K/W", "θJC K/W", "ψJT K/W", "Tj max °C"];
    const rows = items.map((it) => {
      const f = prepFld(it.ref, name), v = f ? f.value : "", z = (String(it.value).match(/^(\d+)R/) || [])[1] || "";
      return `<tr>${prepTd(mode === "series" ? "bead" : "part", `${esc(it.ref)}<span class="sub">${esc(it.mpn || it.value || "")}${it.rail ? " · " + esc(netShort(it.rail)) : ""}${it.heat_w ? " · " + fmtSI(it.heat_w, "W") : ""}</span>`, "m")}
        ${keys.map((k, i) => prepTd(heads[i], prepIn(`data-a="kv" data-r="${esc(it.ref)}" data-n="${name}" data-k="${k}"`, prepKVGet(v, k), k === "z100" ? z || "220" : k === "rdc" ? "0.1" : k === "tj_max" ? "125" : "", "num" + prepNeed(!["psi_jt", "tj_max"].includes(k) && prepBlank(prepKVGet(v, k)))), ["psi_jt", "tj_max"].includes(k) ? "opt" : "")).join("")}
        ${prepTd("source", prepSrcCell(`fld:${it.ref}|${name}`, f))}</tr>`;
    }).join("");
    return `<table class="prep-ed"><thead><tr><th>${mode === "series" ? "Bead" : "Part"}</th>${heads.map((h, i) => `<th${["psi_jt", "tj_max"].includes(keys[i]) ? ' class="opt"' : ""}>${h}</th>`).join("")}<th>Source</th></tr></thead><tbody>${rows}</tbody></table>${prepSharedBlock(name, mode === "series" ? "Source for the bead data" : "Source for the thermal data (the parts' Thermal Information tables)")}`;
  }
  if (mode === "caps") return prepCapsEditor(g, tab);
  if (mode === "kicad") {
    const it = items[0] || {};
    const nets = (it.nets || []).slice(0, 4).map((n) => esc(netShort(n.net) + (n.looks_like ? ` (${n.looks_like}?)` : ""))).join(", ");
    return `<p class="note">${it.class ? `Class <code>${esc(it.class)}</code>: ${nets}. ` : ""}A fix in KiCad, not in kisi.json: ${esc((g.write && g.write.to) || "")}. Reload the board after saving.</p>`;
  }
  const list = items.slice(0, 6).map((it) => esc([it.ref, it.mpn || it.value, it.rail].filter(Boolean).join(" "))).join(" · ");
  return `<p class="note">${list}${items.length > 6 ? ` … ${items.length} in all` : ""}</p>`;
}
// dismissable items (the core marks them): nets of nets-without-target, rails found only by name
function prepDismissable(g) {
  if (g.id === "nets-without-target") return g.items.flatMap((it) => (it.nets || []).filter((n) => n.dismissable).map((n) => n.net));
  return g.items.filter((it) => it.dismissable).map((it) => it.rail);
}
// dismissals that do not apply on this board (the core's not_applied)
function prepStale(gap) { return ((S.prep.cur || {}).dismissed || []).filter((x) => (!gap || x.gap === gap) && (x.not_applied || []).length); }
function prepStaleList(gap) {
  const st = prepStale(gap);
  if (!st.length) return "";
  return st.map((x) => `<div class="prep-stale"><b>Dismissed, but not reported on this board${gap ? "" : ` (<code>${esc(x.gap)}</code>)`}:</b> ${x.not_applied.map((n) => `<span class="prep-tag"><code>${esc(netShort(n))}</code> <button class="linklike" data-b="dis-drop" data-g="${esc(x.gap)}" data-item="${esc(n)}" title="remove from gaps.dismissed">remove</button></span>`).join("")}</div>`).join("");
}
function prepDismissBlock(g) {
  const x = prepDis(g.id, false) || { items: [], reason: "" };
  const all = [...new Set([...prepDismissable(g), ...x.items])];
  if (!all.length) return "";
  // where dismissing is the only way to close the step here (a fix otherwise belongs in KiCad), the form is open
  if (S.prep.dismissing !== g.id && !x.items.length && PREP_MODE[g.id] !== "kicad") return `<div class="prep-row"><button class="btn small" data-b="dismiss" data-g="${g.id}">Dismiss…</button><span class="note">${all.length} item${all.length > 1 ? "s" : ""} found only by ${g.id === "nets-without-target" ? "their net names" : "their names"} – dismiss what is not ${g.id === "nets-without-target" ? "an interface" : "a supply"}.</span></div>`;
  return `<div class="prep-dismiss"><b>Dismiss – stored in <code>${esc(S.name)}.kisi.json</code> → <code>gaps.dismissed</code> with your reason; <code>kisi gaps</code>, the CLI and agents skip these items</b>
    <div class="prep-row">${all.map((n) => `<label class="prep-chk"><input type="checkbox" data-a="dis-item" data-item="${esc(n)}" data-g="${g.id}" ${x.items.includes(n) ? "checked" : ""}> <code>${esc(netShort(n))}</code></label>`).join("")}</div>
    <input data-a="dis-reason" data-g="${g.id}" value="${esc(x.reason || "")}" placeholder="reason (required), e.g. QSPI flash at 50 MHz – no impedance control needed" aria-label="reason">
    <div class="prep-row"><button class="btn small" data-b="dis-all" data-g="${g.id}">Select all</button><button class="btn small" data-b="undo" data-g="${g.id}">Undo</button></div></div>`;
}
// what a step needs, in one line (the inputs below are exactly these)
const PREP_NOUN = {
  "stackup-defaults": ["dielectric layer", "dielectric layers"], "nets-without-target": ["net", "nets"], "classes-without-standard": ["net class", "net classes"],
  "assembly-variant": ["fork", "forks"], "assembly-variant-forks": ["fork", "forks"], "channel-part-values": ["part", "parts"], "rails-without-voltage": ["rail", "rails"],
  "rails-without-source": ["rail", "rails"], "regulator-efficiency": ["regulator", "regulators"], "ic-loads": ["IC", "ICs"], "cap-ratings": ["capacitor", "capacitors"],
  "bead-ratings": ["bead", "beads"], "cap-values": ["capacitor", "capacitors"], "thermal-data": ["part", "parts"],
};
const prepPl = (n, id) => `${n} ${(PREP_NOUN[id] || ["item", "items"])[n === 1 ? 0 : 1]}`;
function prepCountItems(g, tab) { const its = prepItems(g, tab); return g.id === "nets-without-target" ? its.reduce((s, i) => s + (i.nets || []).length, 0) : its.length; }
function prepRequiredLine(g, tab) {
  const its = prepItems(g, tab), n = prepCountItems(g, tab), N = prepPl(n, g.id);
  switch (g.id) {
    case "stackup-defaults": {
      const er = its.filter((i) => i.why.some((w) => w.startsWith("εr"))).length, td = its.filter((i) => i.why.some((w) => w.startsWith("tan"))).length;
      return `Enter ${[er && `εr for ${prepPl(er, g.id)}`, td && `tanδ for ${prepPl(td, g.id)}`].filter(Boolean).join(" and ")} (marked rows), with a source.`;
    }
    case "nets-without-target": return `${N} look like interfaces by their names: give them a class with a target impedance (and profile), with a source – or dismiss those that are not interfaces.`;
    case "classes-without-standard": return `Choose an interface profile for ${N}, with a source.`;
    case "assembly-variant": return its.some((i) => i.variant) ? "Choose the assembly variant to analyse." : `Choose the variant to analyse: “default” (the board as it is) or define variants for the ${N}.`;
    case "assembly-variant-forks": return `List the unused alternative of ${N} as not fitted in the variant.`;
    case "power-tree": return "Enter the regulator chain: a PI_REG entry for each supply input, regulator and pass part, with a source.";
    case "rails-without-voltage": return `Give ${N} a voltage (NAME@<V>V in a PI_REG entry), or dismiss those that are not supplies.`;
    case "rails-without-source": return `Name what feeds ${N}: a supply input or a regulator output in PI_REG.`;
    case "regulator-efficiency": return `Add the efficiency of ${N} to their PI_REG (e.g. buck 88%) – optional, 85 % is assumed.`;
    case "ic-loads": return `Enter the current per rail for ${N}, with a source.`;
    case "cap-ratings": return `Enter rated voltage and dielectric for ${N} (${prepCapGroups(its).length} groups) – optional refinement.`;
    case "bead-ratings": return `Enter Z at 100 MHz and DC resistance for ${N}, with a source – optional refinement.`;
    case "thermal-data": return `Enter θJB and θJC (or ψJT) for ${N}, with a source – optional refinement.`;
    case "channel-part-values": case "cap-values": return `Fix the Value field of ${N} in KiCad, then reload the board.`;
    case "stackup-missing": return "The board file has no stackup: enter it in KiCad, or hand it to the agent.";
    default: return g.issue;
  }
}
// what the core still reports for a step, after Apply or while it is partly filled
function prepMissingLine(g, tab) {
  const now = prepGapsNow().find((x) => x.id === g.id);
  if (!now) return "";
  const left = prepCountItems(now, tab), all = prepCountItems(g, tab);
  const its = prepItems(now, tab);
  const label = (it) => it.ref || (it.index != null ? `#${it.index}` : "") || it.class || (it.rail ? netShort(it.rail) : "") || it.variant || it.fork || "";
  const names = g.id === "nets-without-target" ? its.flatMap((i) => (i.nets || []).map((x) => netShort(x.net))) : its.map(label);
  const list = names.filter(Boolean).slice(0, 6).join(", ") + (names.length > 6 ? " …" : "");
  if (!all) return "Still missing – the core reports it as before.";
  return `${left} of ${prepPl(all, g.id)} still missing${list ? `: ${list}` : ""}.`;
}
// errors of the check in this step's entries
const PREP_ERR = { stackup: /^stackup\./, classes: /^net_classes\./, nettarget: /^net_classes\./, variant: /^variant/, reg: /PI_REG/, loads: /PI_LOAD/, caps: /PI_CAP/, series: /PI_SERIES/, thermal: /PI_THERMAL/ };
function prepStepErrors(g) {
  const re = PREP_ERR[PREP_MODE[g.id]];
  return prepMsgs().filter((m) => m.level === "error" && ((re && re.test(m.text)) || m.text.startsWith(`gaps.dismissed[${g.id}]`)));
}
// commit the new-entry rows of a step that were filled but not left yet
function prepCommitRows(card) {
  for (const row of card ? card.querySelectorAll("tr.new") : []) {
    const r = row.querySelector('input[data-k="ref"]'), v = row.querySelector('input[data-k="value"]');
    if (r && v && r.value.trim() && v.value.trim()) prepOnInput(v, true);
  }
}
// the marker of a step: how many items are still open (coloured by priority), a check mark when done
function prepStepMarker(k, g, tab) {
  if (k === "done") return `<span class="mk done" title="done">✓</span>`;
  if (k === "dis") return `<span class="mk dis" title="dismissed">–</span>`;
  if (k === "agent") return `<span class="mk agent" title="with the agent">↗</span>`;
  const now = prepGapsNow().find((x) => x.id === g.id) || g, n = g.items.length ? Math.max(0, prepCountItems(now, tab) - prepHanded(g, tab).length) : 0;
  return `<span class="mk open ${g.priority === "high" ? "h" : g.priority === "medium" ? "m" : "l"}" title="${n ? `${prepPl(n, g.id)} open` : "open"}, ${g.priority} priority">${n || "!"}</span>`;
}
function prepCard(g, gs, na, tab, n) {
  const st = prepStepState(g, tab), mode = PREP_MODE[g.id], key = S.name + g.id;
  const open = S.prep.cards[key] ??= false;
  const others = g.views.filter((v) => v !== tab).map((v) => `<span class="prep-tag view" title="the same entries serve this category too – filled once">also ${PREP_VIEWS[v]}</span>`).join("");
  const tags = [mode === "kicad" ? `<span class="prep-tag kicad">fix in KiCad</span>` : mode === "agent" ? `<span class="prep-tag agent">agent path</span>` : `<span class="prep-tag">enter here</span>`]
    .concat(g.affects.map((a) => `<span class="prep-tag">${esc(a)}</span>`)).join("");
  const handed = !!prepD().agent[g.id];
  const ns = prepItems(g, tab).length, nall = g.items.length;
  const opt = !!S.prep.opt[key], hasOpt = ["stackup", "thermal", "caps", "loads"].includes(mode);
  const applied = !!S.prep.applied[key], partial = st.k === "open" && /left/.test(prepGapState(g, tab).text);
  const missing = st.k === "open" && (applied || partial) ? prepMissingLine(g, tab) : "";
  const errs = st.k === "open" || applied ? prepStepErrors(g) : [];
  const nh = st.k === "open" ? prepHanded(g, tab).length : 0;
  const stateText = (na && st.k === "open" ? "not blocking" : st.k === "open" ? (prepRequired(g) ? esc(prepGapState(g, tab).text) : "optional") : st.text) + (nh ? ` · ${nh} with the agent` : "");
  const more = `<details class="prep-more"${S.prep.moreOpen[key] ? " open" : ""}><summary>more – where to find it, where it goes${hasOpt ? ", optional fields" : ""}</summary>
      <dl class="prep-meta"><dt>Why</dt><dd>${esc(g.issue)}${ns < nall ? ` Here: the ${ns} of ${nall} items that matter for ${PREP_VIEWS[tab]}.` : ""}</dd><dt>Where to find it</dt><dd>${esc(g.where_to_find)}</dd><dt>Goes to</dt><dd><code>${esc((g.write && g.write.to) || "")}</code></dd><dt>Affects</dt><dd>${others}${tags}</dd></dl>
      ${hasOpt ? `<label class="prep-chk"><input type="checkbox" data-a="opt" data-g="${g.id}" ${opt ? "checked" : ""}> show the optional fields (${mode === "stackup" ? "material, thickness" : mode === "thermal" ? "ψJT, Tj max" : mode === "caps" ? "ESR, ESL" : "basis: typical / maximum"})</label>` : ""}</details>`;
  // Apply stays in every step with a form (also once the core reports it done or dismissed while typing),
  // highlighted while the step has entries not applied yet
  const canApply = mode !== "agent", pending = !!S.prep.pending[key];
  const label = pending || st.k === "open" ? (mode === "kicad" && !prepDismissable(g).length ? "Check again" : "Apply") : "Apply again";
  const foot = `<div class="prep-foot">${canApply ? `<button class="btn small ${pending || st.k === "open" ? "primary" : ""}" data-b="apply" data-g="${g.id}">${label}</button>` : ""}
      ${mode === "kicad" ? "" : `<label class="prep-chk"><input type="checkbox" data-a="agent" data-g="${g.id}" ${handed || mode === "agent" ? "checked" : ""} ${mode === "agent" ? "disabled" : ""}> hand to the agent</label>`}
      <span class="prep-res ${st.k}">${st.k === "done" ? "✓ Done – the core no longer reports it." : st.k === "dis" ? "Dismissed with your reason." : st.k === "agent" ? "With the agent: in the prompt of the agent card." : esc(missing)}</span></div>
      ${prepHandBlock(g, tab, st, mode)}
      ${errs.length ? `<ul class="prep-msgs">${errs.slice(0, 4).map((m) => `<li>${esc(m.text)}</li>`).join("")}${errs.length > 4 ? `<li class="o">${errs.length - 4} more errors</li>` : ""}</ul>` : ""}`;
  return `<article class="prep-gap prep-step k-${st.k} ${na ? "later" : ""} ${opt ? "showopt" : ""}" id="prep-gap-${g.id}" data-gap="${g.id}" aria-expanded="${open}">
    <header data-toggle="${g.id}">${prepStepMarker(st.k, g, tab)}<h3>${esc(PREP_TITLES[g.id] || g.id)}<code>${g.id}</code></h3>
      <span class="state ${st.k}">${stateText}</span><p>${esc(prepRequiredLine(g, tab))}</p></header>
    <div class="body">
      ${st.k === "dis" ? "" : prepEditorMarked(g, gs, tab, st.k === "open")}${prepDismissBlock(g)}${prepStaleList(g.id)}
      ${foot}${more}
    </div></article>`;
}
// single items to the agent: one checkbox per item still open (the step as a whole: "hand to the agent")
function prepHandBlock(g, tab, st, mode) {
  if (st.k !== "open" || mode === "kicad" || mode === "agent" || prepAgentWhole(g)) return "";
  const now = prepGapsNow().find((x) => x.id === g.id), us = now ? prepUnits(now, tab) : [];
  if (us.length < 2) return "";
  const ks = new Set(prepHandedKeys(g)), nh = us.filter((u) => ks.has(u.key)).length, key = S.name + g.id;
  return `<details class="prep-hand" data-hand="${g.id}"${S.prep.handOpen[key] ? " open" : ""}><summary>hand single items to the agent${nh ? ` <b>(${nh} handed over)</b>` : ""}</summary>
    <div class="prep-hand-list">${us.map((u) => `<label class="prep-chk" title="${esc(u.detail)}"><input type="checkbox" data-a="agentitem" data-g="${g.id}" data-k="${esc(u.key)}" ${ks.has(u.key) ? "checked" : ""}> ${esc(u.label)}</label>`).join("")}</div></details>`;
}
// the last step: save the file
function prepSaveStep(n, tab) {
  const diff = prepDiff(), lc = prepLoaded(), done = !diff.n && !!lc;
  const blocked = prepBlocked(S.prep.scope);
  const k = done ? "done" : "open";
  const open = S.prep.cards[S.name + "save:" + tab] ?? false;
  return `<article class="prep-gap prep-step prep-savestep k-${k}" id="prep-step-save" data-gap="save" aria-expanded="${open}">
    <header data-toggle="save:${tab}">${done ? `<span class="mk done">✓</span>` : `<span class="mk open s" title="the last step">⤓</span>`}<h3>Save ${esc(S.name)}.kisi.json</h3>
      <span class="state ${k}">${done ? "saved" : diff.n ? `${diff.n} change${diff.n === 1 ? "" : "s"}` : "nothing entered"}</span>
      <p>${done ? "The file holds everything entered; the analyses use it." : "Write the draft next to the board – everything, or only this category's changes (choice on the left) – and load it."}</p></header>
    <div class="body"><div class="prep-foot"><button class="btn small primary" data-b="save" ${!diff.n && !lc || blocked.length ? "disabled" : ""}>Save kisi.json</button>
      <span class="prep-res">${blocked.length ? `${blocked.length} error${blocked.length > 1 ? "s" : ""} first (see Check on the left).` : esc(S.prep.saved || "")}</span></div></div></article>`;
}
// the end of a tab: ready for its analysis (or only the agent's items left)
function prepFinish(tab) {
  const p = prepProgress(tab);
  if (!p.known || p.na || p.open) return "";
  const view = { impedance: "si", channel: "ch", power: "pi" }[tab];
  const what = { impedance: "the impedance check", channel: "the channel check", power: "the power analysis" }[tab];
  return `<div class="prep-finish ${p.agent ? "agent" : ""}"><span class="mk ${p.agent ? "agent" : "done"}">${p.agent ? "↗" : "✓"}</span><div><b>${p.agent ? `Ready for ${what} – ${p.agent} step${p.agent > 1 ? "s" : ""} with the agent` : `Ready for ${what}`}</b>
    <span>${p.agent ? "The open values use kisi's defaults (marked as estimates) until the agent fills them." : p.opt ? `${p.opt} optional refinement${p.opt > 1 ? "s" : ""} below.` : "Nothing more to enter."}${prepDiff().n ? " Save the kisi.json (last step) so that the analyses use it." : ""}</span></div>
    <button class="btn small primary" data-b="goview" data-view="${view}">Open the ${PREP_VIEWS[tab]} view</button></div>`;
}
// after Apply: the step is done (closed, the next open step opens) or shows what is still missing
function prepAfterApply() {
  const a = S.prep.applying;
  if (!a || a.name !== S.name) return;
  S.prep.applying = null;
  const steps = prepSteps(a.tab), g = steps.find((x) => x.id === a.gap);
  if (!g) return;
  if (prepStepState(g, a.tab).k === "open") { S.prep.cards[S.name + g.id] = true; S.prep.scrollTo = g.id; return; }
  prepAdvance(a.tab, g.id);
}
function prepAdvance(tab, from) {
  const steps = prepSteps(tab);
  if (from) S.prep.cards[S.name + from] = false;
  const open = steps.filter((x) => prepStepState(x, tab).k === "open");
  const next = open.find(prepRequired) || open[0];
  if (next) { S.prep.cards[S.name + next.id] = true; S.prep.scrollTo = next.id; }
  else { S.prep.cards[S.name + "save:" + tab] = true; S.prep.scrollTo = "save"; }
}

// ---- the entries of kisi.json per category, and the viewer's own edits
function prepSection(id, title, sub, body, open) {
  const key = S.name + "sec:" + id, o = S.prep.cards[key] ?? open;
  return `<article class="prep-gap prep-sec" data-sec="${id}" aria-expanded="${o}"><header data-toggle="sec:${id}"><span class="prio file">file</span><h3>${esc(title)}</h3><span class="state">${esc(sub)}</span></header><div class="body">${body}</div></article>`;
}
function prepEntriesHtml(tab, gs) {
  const w = prepW(), out = [];
  const nd = ((w.stackup && w.stackup.dielectrics) || []).length + (w.stackup && w.stackup.solder_mask ? 1 : 0);
  const allIdx = S.scene.stackup.dielectrics.map((_, i) => i);
  if (tab !== "power") out.push(prepSection("stackup", "Laminate (stackup)", `${nd} in the file`, prepStackTable(allIdx, null), false));
  if (tab === "impedance") {
    const names = (S.an.classes || []).filter((c) => c.nets || c.target || (w.net_classes && w.net_classes[c.name])).map((c) => c.name);
    const n = Object.values(w.net_classes || {}).filter((c) => "target_ohm" in c || "diff" in c).length;
    out.push(prepSection("targets", "Net class targets", `${n} in the file`, prepClassTable(names, "impedance"), false));
  }
  if (tab === "channel") {
    const names = (S.an.classes || []).filter((c) => (c.target && c.nets) || (w.net_classes && w.net_classes[c.name] && w.net_classes[c.name].profile)).map((c) => c.name);
    const n = Object.values(w.net_classes || {}).filter((c) => "profile" in c).length;
    out.push(prepSection("profiles", "Net class profiles", `${n} in the file`, names.length ? prepClassTable(names, "channel") : `<p class="note">No net class with a target impedance.</p>`, false));
    out.push(prepSection("conn", "Connector ends", `${Object.keys(w.connector_ends || {}).length} in the file`, prepConnEditor(), false));
    const hasVarGap = prepGapsNow().concat((S.prep.base || { gaps: [] }).gaps).some((g) => PREP_MODE[g.id] === "variant");
    if (!hasVarGap) out.push(prepSection("variants", "Assembly variants", w.variant ? `“${w.variant}” chosen` : `${Object.keys(w.variants || {}).length} defined`, prepVariantEditor(null), false));
  }
  if (tab === "power") {
    const fs = w.fields || [];
    // the regulator table is in the first regulator gap's card when there is one
    const firstReg = (gs || []).find((g) => PREP_MODE[g.id] === "reg"), regCard = !!firstReg && firstReg.priority !== "low";
    for (const [name, title, ph] of PREP_FIELDS) {
      const n = fs.filter((f) => String(f.name).toUpperCase() === name).length;
      if (name === "PI_REG" && regCard) continue;
      out.push(prepSection("f-" + name, `${title} (${name})`, `${n} in the file`, prepFieldTable(name, ph), false));
    }
    const other = [...new Set(fs.map((f) => String(f.name).toUpperCase()).filter((n) => !PREP_FIELDS.some((x) => x[0] === n)))];
    for (const name of other) out.push(prepSection("f-" + name, `Field ${name}`, "kisi does not evaluate it", prepFieldTable(name, ""), false));
  }
  return `<h2 class="prep-h">In ${esc(S.name)}.kisi.json <em>– everything ${PREP_VIEWS[tab]} uses, loaded and entered</em></h2>${prepTakeCard(tab)}${out.join("")}`;
}
// edits made in the views (stored in this browser), as kisi.json entries
function prepViewerEdits(tab) {
  const out = [];
  if (tab !== "power" && typeof stackOv === "function") {
    const so = stackOv().dielectrics || {}, ids = Object.keys(so);
    if (ids.length) out.push({ label: `Stackup: ${ids.length} dielectric layer${ids.length > 1 ? "s" : ""} edited in the Impedance view`, apply: (src) => {
      for (const [i, o] of Object.entries(so)) {
        const e = prepDiel(+i, true);
        for (const k of ["epsilon_r", "loss_tangent", "thickness"]) if (o[k] != null) e[k] = Math.round(o[k] * 1e6) / 1e6;
        e.source = prepClone(src);
      }
    } });
  }
  if (tab === "impedance") {
    const ov = Object.entries(S.overrides || {});
    if (ov.length) out.push({ label: `Targets: ${ov.length} net class${ov.length > 1 ? "es" : ""} set in the Impedance view`, apply: (src) => {
      for (const [c, t] of ov) { const e = prepCls(c, true); e.target_ohm = t.z; e.diff = !!t.diff; e.source = prepClone(src); }
    } });
  }
  if (tab === "channel") {
    const m = Object.entries((typeof classAssign === "function" && classAssign()) || {}).filter(([, p]) => p);
    if (m.length) out.push({ label: `Profiles: ${m.length} net class${m.length > 1 ? "es" : ""} assigned in the Channel view`, apply: (src) => {
      for (const [c, p] of m) { const e = prepCls(c, true); e.profile = p; e.source = prepClone(src); }
    } });
    const ce = Object.entries((typeof connEnds === "function" && connEnds()) || {}).filter(([, e]) => e);
    if (ce.length) out.push({ label: `Connector ends: ${ce.map(([r]) => r).join(", ")} set in the Channel view`, apply: (src) => {
      for (const [r, e] of ce) { const w = prepW(); (w.connector_ends = w.connector_ends || {})[r] = { ...prepClone(e), source: prepClone(src) }; }
    } });
  }
  if (tab === "power") {
    let ed = [], df = [], lost = [], what = "";
    try {
      if (S.piTree && S.piTree.elements) {
        ed = S.engine.editsToFields(S.piTree, null).fields;
        df = S.piDefaults && S.piDefaults.elements ? S.engine.editsToFields(S.piDefaults, null).fields : [];
        what = "power tree";
      } else if (S.piSetups && S.piSetups.size) {
        const r = S.engine.editsToFields(null, [...S.piSetups.values()]);
        ed = r.fields; lost = r.lost;
        let base = [];
        try { base = S.engine.piFields().rails || []; } catch (e) { base = []; }
        df = S.engine.editsToFields(null, base).fields;
        what = "rail setups";
      }
    } catch (e) { console.warn(e); }
    const changed = ed.filter((f) => { const d = df.find((x) => x.ref === f.ref && x.name === f.name); return !d || d.value !== f.value; });
    const removed = df.filter((d) => !ed.some((f) => f.ref === d.ref && f.name === d.name));
    if (changed.length || removed.length) out.push({ label: `Power: ${changed.length + removed.length} field${changed.length + removed.length > 1 ? "s" : ""} from the ${what} edited in the Power view (${[...new Set(changed.concat(removed).map((f) => f.ref))].slice(0, 6).join(", ")})`, lost, apply: (src) => {
      const kept = [];
      for (const f of changed) { const e = prepSetFld(f.ref, f.name, f.value); e.source = prepClone(src); }
      for (const d of removed) { if (prepFld(d.ref, d.name)) prepSetFld(d.ref, d.name, ""); else kept.push(`${d.ref} ${d.name}`); }
      return kept.length ? `In the board file, not removed (change it in KiCad): ${kept.join(", ")}.` : "";
    } });
  }
  return out;
}
function prepTakeCard(tab) {
  const eds = prepViewerEdits(tab);
  if (!eds.length && !S.prep.takeNote) return "";
  const s = S.prep.takeSrc || {};
  return `<div class="prep-take"><b>Edits made in the views (stored only in this browser)</b><ul>${eds.map((e) => `<li>${esc(e.label)}${e.lost && e.lost.length ? `<span class="sub">not in fields: ${esc(e.lost.slice(0, 3).join("; "))}</span>` : ""}</li>`).join("")}</ul>
    ${eds.length ? `<div class="prep-src"><b>Source for these values – required</b><select data-a="take" data-k="kind" aria-label="source kind">${prepKindOpts(s.kind)}</select><input data-a="take" data-k="document" value="${esc(s.document || "")}" placeholder="document or url" aria-label="document"><input data-a="take" data-k="note" value="${esc(s.note || "")}" placeholder="note" aria-label="note"></div>
    <div class="prep-row"><button class="btn small primary" data-b="take" ${prepSrcOk(s) ? "" : "disabled"}>Take into the draft</button><span class="note">The views keep their own edits (they win over kisi.json); reset them there once the file is saved.</span></div>` : ""}
    ${S.prep.takeNote ? `<p class="note">${esc(S.prep.takeNote)}</p>` : ""}</div>`;
}

// what was handed to the agent: per gap the items still open (all of them when the step went as a whole),
// for one category or all of them (a gap shared by categories once, under the first)
const PREP_NEED = {
  "stackup-missing": "the stackup (layers, thicknesses, εr, tanδ)", "stackup-defaults": "εr and tanδ per dielectric layer",
  "nets-without-target": "a class with target impedance and profile (or a dismissal if not an interface)",
  "classes-without-standard": "the interface profile per class", "assembly-variant": "the assembly variant to analyse",
  "assembly-variant-forks": "the unused alternative per fork, as not fitted", "power-tree": "the regulator chain (PI_REG)",
  "rails-without-voltage": "the rail's voltage", "rails-without-source": "the rail's source", "ic-loads": "the current per rail",
  "cap-ratings": "rated voltage and dielectric", "thermal-data": "θJB and θJC (or ψJT)", "regulator-efficiency": "efficiency or dropout",
  "bead-ratings": "impedance at 100 MHz and DC resistance", "channel-part-values": "the part's value", "cap-values": "the capacitance",
};
function prepHandover(scope) {
  const tabs = scope === "all" ? Object.keys(PREP_VIEWS) : [scope], seen = new Set(), out = [];
  for (const tab of tabs) {
    const vn = (prepViewsNow() || {})[tab];
    if (vn && !vn.applicable) continue;
    for (const g of prepSteps(tab)) {
      if (seen.has(g.id)) continue;
      const st = prepStepState(g, tab);
      if (st.k === "done" || st.k === "dis") continue;
      const units = prepHanded(g, tab), whole = prepAgentWhole(g);
      if (!whole && !units.length) continue;
      seen.add(g.id);
      const now = prepGapsNow().find((x) => x.id === g.id);
      out.push({ g, tab, whole, units, total: now ? prepUnits(now, tab).length : 0 });
    }
  }
  return out;
}
function prepHandLine(h) {
  const { g, whole, units, total } = h, cap = whole ? 12 : units.length;
  const list = units.slice(0, cap).map((u) => u.detail ? `${u.label} (${u.detail})` : u.label).join("; ");
  const rest = units.length > cap ? `; … +${units.length - cap} more – all ${total} items of the gap` : "";
  const what = PREP_NEED[g.id] || g.issue.split(/[.:]/)[0];
  const to = (g.write && g.write.to) || "";
  return `- ${g.id}: ${what}${to ? ` → ${to}` : ""}${units.length ? ` – ${whole && units.length === total ? "all: " : ""}${list}${rest}` : ""}`;
}
// a board from the KiCad plugin: the agent gets the paths – the CLI shipped with the plugin, the board, its kisi.json
const prepQ = (p) => `"${p}"`;
function prepKicadCmd(scope) {
  const c = kicadCli();
  return c ? `node ${prepQ(c.path)} gaps ${prepQ(S.kicad.pcb_path)}${scope === "all" ? "" : ` --for ${scope}`}` : "";
}
function prepKicadPromptTail() {
  const c = kicadCli();
  if (!c) return "";
  const node = c.node || {};
  return `\nPaths: the board is ${prepQ(S.kicad.pcb_path)}; write ${prepQ(kicadCfgPath())} directly (next to the board, no browser save needed). `
    + `Run kisi with the CLI that came with the KiCad plugin: node ${prepQ(c.path)} <command> … (${node.ok ? `Node.js ${node.version} is installed` : "Node.js ≥ 18 is needed: https://nodejs.org"}). `
    + `When it is written, I click kisi in KiCad again to load it.`;
}
function prepPrompt(gs, scope) {
  scope = scope || S.prep.tab;
  const hs = prepHandover(scope), all = scope === "all";
  const what = all ? "analyses" : `${PREP_VIEWS[scope].toLowerCase()} analysis`, cmd = all ? "`kisi gaps`" : `\`kisi gaps --for ${scope}\``;
  const tail = `Merge the results into ${S.name}.kisi.json next to the board – keep my entries and the dismissed gaps – and run \`kisi config-check\`.` + prepKicadPromptTail();
  if (!hs.length) return `Prepare ${prepBoardFile()} for kisi's ${what} with the skill kisi-prepare. Start from ${cmd}, research ${all ? "the" : "this category's"} open gaps from datasheets, BOM and position files, with a source for every value. ${tail}`;
  const lines = all ? Object.keys(PREP_VIEWS).flatMap((t) => { const ls = hs.filter((h) => h.tab === t); return ls.length ? [`${PREP_VIEWS[t]} (--for ${t}):`, ...ls.map(prepHandLine)] : []; }) : hs.map(prepHandLine);
  return `Prepare ${prepBoardFile()} for kisi's ${what} with the skill kisi-prepare. Start from ${cmd} and research only what I handed over (gap: what is needed → where it goes – items), from datasheets, BOM and position files, with a source for every value:\n${lines.join("\n")}\n${tail}`;
}

// ---- the page
// a full redraw waits while a mouse button or finger is down on the page: a redraw between press and
// release would replace the button under it and swallow the click (Apply "disappearing")
let prepPointer = false, prepRedrawAfter = false;
document.addEventListener("pointerdown", (e) => { if (e.target.closest && e.target.closest("#prep")) prepPointer = true; }, true);
const prepPointerUp = () => { if (!prepPointer) return; prepPointer = false; if (prepRedrawAfter) { prepRedrawAfter = false; setTimeout(() => prepRender(true), 0); } };
document.addEventListener("pointerup", prepPointerUp, true);
document.addEventListener("pointercancel", prepPointerUp, true);
function prepRender(full) {
  prepRenderStatus();
  if (!S.prep.open || !S.scene) return;
  if (full !== false && prepPointer) { prepRedrawAfter = true; prepRenderSide(); return; }
  if (full !== false) { S.prep.drawing = true; try { prepRenderMain(); } finally { S.prep.drawing = false; } }
  prepRenderSide();
}
function prepRenderMain() {
  const base = S.prep.base, tab = S.prep.tab, vnow = prepViewsNow() || {};
  const mk = (v) => { const m = prepMark(v); return `<span class="mk ${m.cls}">${m.t}</span>`; };
  $("prep-ready").innerHTML = Object.entries(PREP_VIEWS).map(([v, name]) => `<button data-prep-tab="${v}" aria-current="${v === tab}">${mk(v)}<b>${name}</b><span>${esc(prepStateText(v))}</span></button>`).join("");
  $("prep-tabs").innerHTML = Object.entries(PREP_VIEWS).map(([v, name]) => `<button role="tab" aria-selected="${v === tab}" data-prep-tab="${v}">${mk(v)}${name}<small>${esc(prepStateText(v))}</small></button>`).join("");
  // keep the field being typed in: its element, value and caret survive the redraw
  const ae = document.activeElement, inMain = ae && ae.closest && ae.closest("#prep-gaps");
  const focus = inMain && ae.dataset && Object.keys(ae.dataset).length ? { sel: prepSel(ae), value: ae.value, start: ae.selectionStart, end: ae.selectionEnd, type: ae.type } : null;
  const scroll = $("prep").scrollTop;
  if (!base) {
    $("prep-head").innerHTML = `<p class="note">${esc(S.prep.note || "Computing the gap list (kisi gaps)…")}</p>`;
    $("prep-gaps").innerHTML = ""; return;
  }
  const x = vnow[tab] || base.views[tab], na = x && !x.applicable;
  const gs = prepSteps(tab);
  // the first open step is open when the tab is first shown
  if (!S.prep.started[S.name + tab]) {
    S.prep.started[S.name + tab] = true;
    const first = gs.filter((g) => prepStepState(g, tab).k === "open"), f = first.find(prepRequired) || first[0];
    for (const g of gs) S.prep.cards[S.name + g.id] = !!f && !na && g.id === f.id;
  }
  const p = prepProgress(tab);
  $("prep-head").innerHTML = na
    ? `<div class="prep-na"><b>Nothing to check for ${PREP_VIEWS[tab]}:</b> ${esc(x.why)}. ${tab === "impedance" ? "The laminate data below still matters once net classes get a target impedance (and for loss in a channel check); it blocks nothing now." : "The steps below matter only once there is something to check; they block nothing now."}</div>`
    : prepFinish(tab) || `<div class="prep-progress"><span><b>${p.done} of ${p.req} required step${p.req === 1 ? "" : "s"} done</b>${p.agent ? ` · ${p.agent} with the agent` : ""}${p.opt ? ` · ${p.opt} optional` : ""}<span class="note"> – ${esc(x && x.why ? x.why : "")}</span></span>
      <span class="note">Work down the list: fill in what a step asks for and press Apply – the step gets a check mark when kisi no longer reports it, and the next one opens. Save the kisi.json at the end.</span>
      <div class="prep-bar"><span style="width:${p.req ? Math.round(100 * (p.done + p.agent) / p.req) : 100}%"></span></div></div>`;
  // dismissals whose gap is no longer listed for this tab
  const listed = new Set(gs.map((g) => g.id));
  const stale = prepStale().filter((s) => !listed.has(s.gap) && (((S.prep.base.gaps.find((g) => g.id === s.gap) || {}).views) || (s.gap === "nets-without-target" ? ["impedance", "channel"] : ["power"])).includes(tab));
  const staleHtml = stale.length ? `<div class="prep-gap prep-stalecard"><header><span class="mk open m">!</span><h3>Stale dismissals</h3><span class="state">${stale.length}</span><p>Items in gaps.dismissed that this board does not report (any more) – remove them, or keep them if the board will change back.</p></header><div class="body">${stale.map((s) => prepStaleList(s.gap)).join("")}</div></div>` : "";
  const steps = gs.length ? gs.map((g, i) => prepCard(g, gs, na, tab, i + 1)).join("") : `<div class="prep-empty">kisi gaps reports nothing missing for ${PREP_VIEWS[tab]}.</div>`;
  $("prep-gaps").innerHTML = steps + staleHtml + prepSaveStep(gs.length + 1, tab) + prepEntriesHtml(tab, gs);
  if (focus) {
    const el = document.querySelector(`#prep-gaps ${focus.sel}`);
    if (el) {
      el.focus({ preventScroll: true });
      // the typed text (a number being typed, "3."); not for the new-entry row, which is drawn from its own state
      if (el.value !== focus.value && "value" in el && focus.type !== "checkbox" && focus.type !== "radio" && el.dataset.a !== "new") el.value = focus.value;
      if (el.setSelectionRange && focus.type === "text" && focus.start != null) try { el.setSelectionRange(focus.start, focus.end); } catch (e) { /* not a text field */ }
    }
  }
  $("prep").scrollTop = scroll;
  if (S.prep.scrollTo) {
    const el = S.prep.scrollTo === "save" ? $("prep-step-save") : $("prep-gap-" + S.prep.scrollTo);
    S.prep.scrollTo = null;
    if (el) { el.scrollIntoView({ block: "start", behavior: "instant" }); const f = el.querySelector(".body .prep-need:is(input, select, button):not([disabled]), .body .prep-need input:not([disabled])") || el.querySelector(".body input:not([type=checkbox]):not([disabled]), .body select:not([disabled])"); if (f && el.dataset.gap !== "save") f.focus({ preventScroll: true }); }
  }
  S.prep.gs = gs; S.prep.na = na;
}
function prepSel(el) { return el.tagName.toLowerCase() + Object.entries(el.dataset).map(([k, v]) => `[data-${k.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase())}="${CSS.escape(v)}"]`).join(""); }
function prepMsgs() {
  // the live check of the draft, plus what only the analysed board can tell (dismissals) from the gap list
  const ck = S.prep.check, msgs = ck ? [...ck.messages] : [];
  const cc = S.prep.cur && S.prep.cur.config;
  if (ck && cc) for (const [level, list] of [["error", cc.errors || []], ["warning", cc.warnings || []]]) for (const t of list) if (t.startsWith("gaps.dismissed") && !msgs.some((m) => m.text === t)) msgs.push({ level, text: t, views: (((S.prep.base || { gaps: [] }).gaps.find((g) => t.includes(`[${g.id}]`)) || {}).views) || Object.keys(PREP_VIEWS) });
  return msgs;
}
function prepRenderSide() {
  const tab = S.prep.tab, entries = (S.prep.check && S.prep.check.entries) || 0, d = prepD();
  const lc = prepLoaded(), lt = prepLoadedText();
  $("prep-cfg").textContent = lc ? `${S.lastFiles.cfg.name} loaded` : lt != null ? `${S.lastFiles.cfg.name}: not JSON` : "no kisi.json";
  const diff = prepDiff(), dirty = diff.n > 0;
  const parts = [diff.add && `${diff.add} added`, diff.chg && `${diff.chg} changed`, diff.del && `${diff.del} removed`].filter(Boolean).join(", ");
  $("prep-draft").innerHTML = (dirty ? `<b>Draft:</b> ${parts} – compared with ${lc ? "the loaded file" : "no file"}; kept in this browser until saved or discarded.` : lc ? `No changes to the loaded file (${prepEntries(prepW()).size} entries).` : "Nothing entered yet.")
    + (d.foreign ? ` <span class="warn">This draft was made on ${lc ? "an earlier version of this" : "another"} kisi.json – check it, or discard it to start from ${lc ? "the loaded file" : "scratch"}.</span>` : "");
  $("prep-stale").hidden = !d.stale;
  if (d.stale) $("prep-stale").innerHTML = `<span>Unsaved draft from ${lc && d.stale.baseText != null ? "an earlier version of this kisi.json" : d.stale.baseText != null ? "a kisi.json that is not loaded now" : "before this kisi.json was loaded"}${d.stale.v !== PREP_DRAFT_V ? " (an earlier version of kisi)" : ""} – not applied.</span>
    <button class="btn small" data-b="stale-restore">Restore</button><button class="btn small" data-b="stale-discard">Discard</button>`;
  $("prep-discard").hidden = !dirty;
  const scopes = [["all", "save everything"], [tab, `save only ${PREP_VIEWS[tab]}`]];
  if (!scopes.some((s) => s[0] === S.prep.scope)) S.prep.scope = "all";
  $("prep-scope").innerHTML = scopes.map(([k, t]) => `<option value="${k}" ${S.prep.scope === k ? "selected" : ""}>${t}</option>`).join("");
  const blocked = prepBlocked(S.prep.scope), can = dirty || !!lc;
  $("prep-save").disabled = !can || blocked.length > 0;
  $("prep-save").title = !can ? "nothing entered" : blocked.length ? "fix the errors first (warnings are fine)" : "";
  const fsa = typeof window.showSaveFilePicker === "function", kc = kicadActive();
  // a board from the KiCad plugin: the file belongs into the board's folder, which the page knows but cannot open
  const where = !kc ? (fsa ? "Saves next to the board (you choose the place)" : "Downloads")
    : fsa ? `Saves next to the board: choose the board's folder in the dialog (or paste the path below as the file name) – kisi remembers it for this board. Saves`
      : `Downloads – with the browser set to ask where to save (Firefox: Settings → General → Downloads → “Always ask you where to save files”; Safari: Settings → General → File download location → “Ask for each download”) choose the board's folder (path below); otherwise click kisi in KiCad again and the plugin takes the file over from the downloads folder. Downloads`;
  $("prep-save-note").textContent = S.prep.saved ? S.prep.saved : !can ? "" : blocked.length
    ? `${blocked.length} error${blocked.length > 1 ? "s" : ""} in ${S.prep.scope === "all" ? "the draft" : `the ${PREP_VIEWS[S.prep.scope]} entries`}${S.prep.scope === "all" && !prepBlocked(tab).length ? ` – “save only ${PREP_VIEWS[tab]}” works` : ""}.`
    : `${where} ${S.name}.kisi.json${dirty ? "" : " as loaded"}, then loads it.`;
  $("prep-kicad").hidden = !kc;
  if (kc) $("prep-kicad-path").textContent = kicadCfgPath();
  const nn = (prepW().notes || []).length;
  $("prep-notes-n").textContent = nn ? `(${nn})` : "";
  if (document.activeElement !== $("prep-notes")) $("prep-notes").value = (prepW().notes || []).join("\n");
  const msgs = prepMsgs(), mine = msgs.filter((m) => m.views.includes(tab)), other = msgs.filter((m) => !m.views.includes(tab));
  const nerr = msgs.filter((m) => m.level === "error").length, any = !!S.prep.check;
  $("prep-check-sum").textContent = any ? (nerr ? `${nerr} error${nerr === 1 ? "" : "s"}` : "ok") : "";
  $("prep-msgs").innerHTML = !any ? `<li class="o">Nothing entered yet.</li>` : mine.map((m) => `<li class="${m.level === "warning" ? "w" : ""}">${esc(m.text)}</li>`).join("") +
    (other.length ? `<li class="o">${other.length} more in entries of other categories</li>` : "") + (!nerr ? `<li class="ok">ok – ${entries} entr${entries === 1 ? "y" : "ies"}</li>` : "");
  const gs = S.prep.gs || [], na = S.prep.na;
  const asc = S.prep.agentScope === "all" ? "all" : tab, hs = prepHandover(asc);
  $("prep-agent-for").textContent = asc === "all" ? "all categories" : `--for ${tab}`;
  $("prep-agent-scope").innerHTML = [[ "tab", `this category (${PREP_VIEWS[tab]})`], ["all", "all categories"]].map(([k, t]) => `<option value="${k}" ${(S.prep.agentScope === "all") === (k === "all") ? "selected" : ""}>${t}</option>`).join("");
  $("prep-agent-list").innerHTML = hs.length ? hs.map((h) => `<li class="w"><b>${esc(PREP_TITLES[h.g.id] || h.g.id)}</b> <code>${h.g.id}</code>${h.units.length ? ` – ${h.whole && h.units.length === h.total ? "all " : ""}${h.units.length} item${h.units.length === 1 ? "" : "s"}${h.whole ? "" : `: ${esc(h.units.slice(0, 6).map((u) => u.label).join(", "))}${h.units.length > 6 ? " …" : ""}`}` : ""}</li>`).join("")
    : `<li class="o">nothing handed over – the prompt asks for ${asc === "all" ? "the" : "this category's"} open gaps</li>`;
  const nap = asc !== "all" && na;
  $("prep-prompt").textContent = nap ? `Nothing to check for ${PREP_VIEWS[tab]} on this board – no prompt needed.` : prepPrompt(gs, asc);
  // a board from the KiCad plugin: the ready command with the installed CLI, or where to get Node.js
  const kcli = kicadCli();
  $("prep-agent-kicad").hidden = !kcli;
  if (kcli) {
    const node = kcli.node || {};
    $("prep-agent-kicad").innerHTML = `<span class="note">The CLI came with the KiCad plugin – the agent writes <code>${esc(S.name)}.kisi.json</code> next to the board itself:</span>
      <pre class="prep-cmd" id="prep-agent-cmd">${esc(prepKicadCmd(asc))}</pre>`
      + (node.ok ? `<span class="note">Node.js ${esc(node.version)} found.</span>`
        : `<span class="warn">Node.js ≥ 18 was not found (${esc(node.reason || "not checked")}) – install the LTS version from <a href="https://nodejs.org" target="_blank" rel="noopener">nodejs.org</a>, then click kisi in KiCad again.</span>`);
  }
  $("prep-copy").disabled = !!nap;
  $("prep-json-name").textContent = S.name + ".kisi.json";
  let txt = "";
  try { txt = prepText(S.prep.scope); } catch (e) { txt = "(" + e.message + ")"; }
  $("prep-json").textContent = can ? txt : "(nothing entered yet – the file appears here as you fill in values)";
  $("prep-json-note").textContent = S.prep.scope === "all" ? (lc ? "The loaded file with your changes." : "Everything entered.") : `The loaded file with only the ${PREP_VIEWS[S.prep.scope]} changes.`;
}
// what is saved: the draft, or the loaded file with one category's changes (by entry, from the core's subset)
function prepScoped(scope) {
  const w = prepW();
  if (!scope || scope === "all") return w;
  const base = prepBaseObj();
  const sub = (o) => JSON.parse(S.engine.configSubset(JSON.stringify(o), scope));
  const W = sub(w), L = sub(base), out = prepClone(base);
  const byIdx = (o) => ((o.stackup && o.stackup.dielectrics) || []);
  if (byIdx(L).length || byIdx(W).length) {
    out.stackup = out.stackup || {};
    const keep = (out.stackup.dielectrics || []).filter((x) => !byIdx(L).some((y) => y.index === x.index));
    out.stackup.dielectrics = keep.concat(byIdx(W).map((y) => prepClone(byIdx(w).find((x) => x.index === y.index)))).sort((a, b) => a.index - b.index);
  }
  if ((L.stackup && L.stackup.solder_mask) || (W.stackup && W.stackup.solder_mask)) { out.stackup = out.stackup || {}; if (W.stackup && W.stackup.solder_mask) out.stackup.solder_mask = prepClone(w.stackup.solder_mask); else delete out.stackup.solder_mask; }
  for (const [c, e] of Object.entries(L.net_classes || {})) if (out.net_classes && out.net_classes[c]) { for (const k of Object.keys(e)) delete out.net_classes[c][k]; if (!Object.keys(out.net_classes[c]).length) delete out.net_classes[c]; }
  for (const [c, e] of Object.entries(W.net_classes || {})) { out.net_classes = out.net_classes || {}; out.net_classes[c] = out.net_classes[c] || {}; for (const k of Object.keys(e)) out.net_classes[c][k] = prepClone(w.net_classes[c][k]); }
  for (const key of ["connector_ends", "variants"]) {
    for (const k of Object.keys(L[key] || {})) if (out[key]) delete out[key][k];
    for (const k of Object.keys(W[key] || {})) (out[key] = out[key] || {})[k] = prepClone(w[key][k]);
  }
  const fk = (f) => `${f.ref}|${String(f.name).toUpperCase()}`;
  if ((L.fields || []).length || (W.fields || []).length) {
    const lk = new Set((L.fields || []).map(fk));
    out.fields = (out.fields || []).filter((f) => !lk.has(fk(f))).concat((W.fields || []).map((f) => prepClone((w.fields || []).find((x) => fk(x) === fk(f)))));
  }
  if ("variant" in L) delete out.variant;
  if ("variant" in W) out.variant = w.variant;
  const dl = (L.gaps && L.gaps.dismissed) || [], dw = (W.gaps && W.gaps.dismissed) || [];
  if (dl.length || dw.length) {
    out.gaps = out.gaps || {};
    out.gaps.dismissed = (out.gaps.dismissed || []).filter((x) => !dl.some((y) => y.gap === x.gap)).concat(dw.map((y) => prepClone(w.gaps.dismissed.find((x) => x.gap === y.gap))));
  }
  for (const k of ["net_classes", "connector_ends", "fields", "variants"]) if (k in out && prepEmpty(out[k]) && !(k in base)) delete out[k];
  if (out.stackup && prepEmpty(out.stackup.dielectrics) && !(base.stackup && base.stackup.dielectrics)) delete out.stackup.dielectrics;
  if (out.stackup && prepEmpty(out.stackup) && !base.stackup) delete out.stackup;
  if (out.gaps && prepEmpty(out.gaps.dismissed) && !(base.gaps && base.gaps.dismissed)) delete out.gaps.dismissed;
  if (out.gaps && prepEmpty(out.gaps) && !base.gaps) delete out.gaps;
  return out;
}
function prepText(scope) { return JSON.stringify(prepScoped(scope), null, 2) + "\n"; }
// errors in what would be saved (warnings do not block)
function prepBlocked(scope) { return prepMsgs().filter((m) => m.level === "error" && (scope === "all" || m.views.includes(scope))); }
async function prepSave() {
  const scope = S.prep.scope, name = `${S.name}.kisi.json`;
  let text;
  try { text = prepText(scope); } catch (e) { S.prep.saved = "Not saved: " + e.message; prepRender(); return; }
  let ck;
  try { ck = S.engine.configCheck(prepBoardFile(), text); } catch (e) { ck = { ok: false, errors: [e.message] }; }
  if (!ck.ok) { S.prep.saved = "Not saved: " + ck.errors.slice(0, 2).join("; "); prepRender(); return; }
  let how = "downloaded";
  if (typeof window.showSaveFilePicker === "function") {
    try {
      const start = (S.fileHandles || []).find((h) => /\.kicad_pcb$/i.test(h.name));
      // id: the browser remembers the folder chosen for this board; a handle of the board file, where there is one,
      // starts the dialog next to it. A board from KiCad has no handle: no startIn, so nothing competes with the id
      const kc = kicadActive(), id = "kisi-" + textHash(kc ? S.kicad.dir : S.name);
      const opts = { id, suggestedName: name, types: [{ description: "kisi board configuration", accept: { "application/json": [".json"] } }] };
      if (start || !kc) opts.startIn = start || "documents";
      const h = await window.showSaveFilePicker(opts);
      const wr = await h.createWritable(); await wr.write(text); await wr.close();
      if (S.fileHandles) S.fileHandles = S.fileHandles.filter((x) => !/\.kisi\.json$/i.test(x.name)).concat([h]);
      how = "saved";
    } catch (e) { if (e && e.name === "AbortError") return; console.warn(e); prepDownload(name, text); }
  } else prepDownload(name, text);
  S.prep.lastSaved = text;
  if (kicadActive()) S.kicad.config = how;
  // the saved file is the new base; what was not saved (other categories) stays in the draft
  const d = prepD();
  const rest = scope === "all" ? JSON.parse(text) : d.work;
  const rec = prepRec(d, text, rest);
  S.prep.mem[S.name] = rec; lsSet(PREP_KEY(), rec);
  S.prep.savedNext = `${name} ${how} and loaded${scope === "all" ? "" : ` (${PREP_VIEWS[scope]} changes; the rest stays in the draft)`}.`
    + (kicadActive() && how === "downloaded" ? " Saved somewhere else than the board's folder (path below)? Click kisi in KiCad again: the plugin takes it over from the downloads folder." : "");
  const l = S.lastFiles;
  await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, { name, text, csv: (l.cfg && l.cfg.csv) || {} });
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
  if (on) { if (PREP_OF_TAB[S.tab] && !S.prep.touched) S.prep.tab = PREP_OF_TAB[S.tab]; S.prep.saved = ""; prepCheck(); prepRender(true); window.scrollTo(0, 0); }
}

// ---- events: an input changes the draft (and the preview); a change redraws the forms
function prepOnInput(t, full) {
  // a field that loses the focus while the forms are drawn again fires "change" from inside the redraw
  if (!t.isConnected || S.prep.drawing) return;
  const stepEl = t.closest && t.closest(".prep-step[data-gap]");
  if (stepEl && stepEl.dataset.gap !== "save" && t.dataset.a && !["agent", "agentitem", "opt"].includes(t.dataset.a)) S.prep.pending[S.name + stepEl.dataset.gap] = true;
  const a = t.dataset.a, val = t.type === "checkbox" ? t.checked : t.value, k = t.dataset.k;
  if (t.classList.contains("prep-need") && !prepBlank(val)) t.classList.remove("prep-need");
  if (t.type === "radio") { const ng = t.closest(".prep-need"); if (ng) ng.classList.remove("prep-need"); }
  if (t.id === "prep-scope") { S.prep.scope = t.value; prepRender(false); return; }
  if (t.id === "prep-agent-scope") { S.prep.agentScope = t.value; prepRender(false); return; }
  if (t.id === "prep-notes") { const w = prepW(), ls = String(t.value).split("\n").map((x) => x.trim()).filter(Boolean); if (ls.length) w.notes = ls; else { delete w.notes; if ("notes" in prepBaseObj()) w.notes = []; } prepEdited(false); return; }
  if (!a) return;
  if (a === "take") { S.prep.takeSrc = { ...(S.prep.takeSrc || {}), [k]: val }; if (!String(val).trim()) delete S.prep.takeSrc[k]; const b = document.querySelector('#prep [data-b="take"]'); if (b) b.disabled = !prepSrcOk(S.prep.takeSrc); return; }
  if (a === "agent") {
    if (!full) return;
    prepD().agent[t.dataset.g] = t.checked; if (!t.checked) delete prepD().agent[t.dataset.g];
    prepPersist();
    if (t.checked) prepAdvance(S.prep.tab, t.dataset.g);
    clearTimeout(prepDrawTimer); prepDrawTimer = setTimeout(() => prepRender(true), 0);
    return;
  }
  if (a === "agentitem") {
    if (!full) return;
    const d = prepD(), g = t.dataset.g, ks = new Set(d.agentItems[g] || []);
    if (t.checked) ks.add(k); else ks.delete(k);
    if (ks.size) d.agentItems[g] = [...ks]; else delete d.agentItems[g];
    S.prep.handOpen[S.name + g] = true;
    prepPersist();
    clearTimeout(prepDrawTimer); prepDrawTimer = setTimeout(() => prepRender(true), 0);
    return;
  }
  if (a === "opt") { if (!full) return; S.prep.opt[S.name + t.dataset.g] = t.checked; S.prep.moreOpen[S.name + t.dataset.g] = true; clearTimeout(prepDrawTimer); prepDrawTimer = setTimeout(() => prepRender(true), 0); return; }
  if (a === "src") {
    const e = prepEntry(t.dataset.e, false);
    if (!e) return;
    prepSetSrc(e, k, val);
    if (k === "kind" || /^(url|page|note)$/.test(k)) S.prep.openSrc = /^(url|page|note)$/.test(k) ? t.dataset.e : S.prep.openSrc;
  } else if (a === "shared") prepSetShared(t.dataset.t, k, val);
  else if (a === "diel") {
    const i = +t.dataset.i, e = prepDiel(i, true);
    prepSet(e, k, prepVal(val, k !== "material")); prepAutoSrc(e, "stackup"); prepPrune("diel", i);
  } else if (a === "mask") { const e = prepMask(true); prepSet(e, k, prepVal(val, true)); prepAutoSrc(e, "stackup"); prepPrune("mask", ""); }
  else if (a === "cls") {
    const c = t.dataset.c, e = prepCls(c, true);
    if (k === "diff") e.diff = !!val; else prepSet(e, k, prepVal(val, k === "target_ohm"));
    // a target without its type: take the class's (the checkbox shows it)
    if (k === "target_ohm" && e.target_ohm != null && e.diff == null) { const cl = (S.an.classes || []).find((x) => x.name === c); e.diff = !!(cl && cl.target && cl.target.diff); }
    if (k === "target_ohm" && e.target_ohm == null && !("profile" in e)) delete e.diff;
    prepAutoSrc(e, "classes"); prepPrune("cls", c);
  } else if (a === "nc" || a === "ncnet") {
    // the class for nets in "Default": created with the first value, renamed by its name field
    const key = S.name + t.dataset.g, w = prepW(), name = t.dataset.n;
    const card = t.closest(".prep-nc"), extraEl = card.querySelector('input[data-k="extra"]');
    const nets = [...card.querySelectorAll('input[data-a="ncnet"]')].filter((x) => x.checked).map((x) => x.dataset.net).concat(prepList(extraEl ? extraEl.value : ""));
    let e = w.net_classes && w.net_classes[name];
    if (a === "nc" && k === "name") {
      if (!full) return;
      const nn = String(val).trim().replace(/[^A-Za-z0-9_+-]/g, "_");
      if (!nn || nn === name) return;
      S.prep.ncName[key] = nn;
      if (e && !(w.net_classes[nn])) { w.net_classes[nn] = e; delete w.net_classes[name]; }
    } else {
      S.prep.ncNets[key] = nets;
      if (!e && a === "nc" && String(val ?? "").trim() !== "" && val !== false) { e = prepCls(name, true); }
      if (e) {
        if (nets.length) e.nets = nets; else delete e.nets;
        if (a === "nc" && k !== "extra") { if (k === "diff") e.diff = !!val; else prepSet(e, k, prepVal(val, k === "target_ohm")); }
        if (e.target_ohm != null && e.diff == null) e.diff = !!card.querySelector('input[data-k="diff"]').checked;
        prepAutoSrc(e, "classes");
        if (!e.nets && !("target_ohm" in e) && !("profile" in e)) { delete w.net_classes[name]; prepTidy(); }
      }
    }
  } else if (a === "fld") prepSetFld(t.dataset.r, t.dataset.n, val, t.dataset.n);
  else if (a === "fldref") { if (!full) return; const f = prepFld(t.dataset.r, t.dataset.n), r = String(val).trim(); if (f && r && !prepFld(r, t.dataset.n)) f.ref = r; }
  else if (a === "new") {
    // the row's two inputs are read from the page (a change of the row drawn over meanwhile does not count)
    const p = (S.prep.pend[t.dataset.n] ||= {}), row = t.closest("tr");
    p[k] = val;
    if (!full) return;
    for (const x of row ? row.querySelectorAll('input[data-a="new"]') : []) p[x.dataset.k] = x.value;
    if (String(p.ref || "").trim() && String(p.value || "").trim()) {
      const ref = p.ref.trim(), old = prepFld(ref, t.dataset.n);
      prepSetFld(ref, t.dataset.n, (old ? old.value + (t.dataset.n === "PI_REG" ? "; " : ", ") : "") + p.value.trim(), t.dataset.n);
      delete S.prep.pend[t.dataset.n];
      S.prep.openSrc = `fld:${ref}|${t.dataset.n}`;
    } else return;
  } else if (a === "load") {
    const ref = t.dataset.r, f = prepFld(ref, "PI_LOAD");
    if (t.dataset.rail) { const nf = prepSetFld(ref, "PI_LOAD", prepLoadSet(f ? f.value : "", t.dataset.rail, val), "PI_LOAD"); if (nf) prepAutoSrc(nf, "PI_LOAD"); }
    else if (k === "basis" && f) { const s = f.source || {}; const note = String(s.note || "").replace(/\b(typical|maximum)\b,?\s*/g, "").trim(); prepSetSrc(f, "note", [val, note].filter(Boolean).join(", ")); }
  } else if (a === "kv") { const f = prepFld(t.dataset.r, t.dataset.n); prepSetFld(t.dataset.r, t.dataset.n, prepKVSet(f ? f.value : "", k, val), t.dataset.n); }
  else if (a === "capgrp") {
    for (const ref of (S.prep.capGroups || {})[t.dataset.g] || []) {
      const f = prepFld(ref, "PI_CAP"), v = f ? f.value : "";
      prepSetFld(ref, "PI_CAP", k === "diel" ? prepCapDielSet(v, val) : prepKVSet(v, k, val), "PI_CAP");
    }
  } else if (a === "conn") {
    const r = t.dataset.r, e = prepConn(r, true);
    if (k.startsWith("ts_")) {
      const ts = { ...(e.touchstone || {}) };
      if (k === "ts_file") prepSet(ts, "file", prepVal(val));
      else if (k === "ts_order") prepSet(ts, "port_order", prepVal(val));
      else if (k === "ts_ports") {
        const ports = {};
        for (const p of prepList(val)) { const m = p.match(/^([^:\s]+)\s*:\s*(\d+)\s*\/\s*(\d+)$/); if (m) ports[m[1]] = [+m[2], +m[3]]; }
        if (Object.keys(ports).length) ts.ports = ports; else delete ts.ports;
      }
      if (ts.file) e.touchstone = ts; else delete e.touchstone;
    } else prepSet(e, k, prepVal(val, k !== "model"));
    prepAutoSrc(e, "conn"); prepPrune("conn", r);
  } else if (a === "variant") { prepW().variant = val; }
  else if (a === "vdef") {
    const v = prepVar(t.dataset.v, true);
    if (["description", "bom"].includes(k)) prepSet(v, k, prepVal(val)); else { const l = prepList(val); if (l.length) v[k] = l; else delete v[k]; }
    prepAutoSrc(v, "variants");
  } else if (a === "dis-item") {
    const x = prepDis(t.dataset.g, true);
    x.items = t.checked ? [...new Set([...x.items, t.dataset.item])] : x.items.filter((i) => i !== t.dataset.item);
  } else if (a === "dis-reason") { prepDis(t.dataset.g, true).reason = val; }
  else return;
  prepEdited(!!full);
}
$("prep-btn").onclick = () => { if (S.scene) prepOpen(!S.prep.open); };
for (const k of ["si", "ch", "pi"]) $("view-" + k).addEventListener("click", () => { if (S.prep.open) prepOpen(false); });
$("prep").addEventListener("input", (ev) => { const t = ev.target; if (t.matches("select, input[type=checkbox], input[type=radio]")) return; prepOnInput(t, false); });
$("prep").addEventListener("change", (ev) => prepOnInput(ev.target, true));
$("prep").addEventListener("toggle", (ev) => { const d = ev.target; if (d.classList && d.classList.contains("prep-more")) { const c = d.closest("[data-gap]"); if (c) S.prep.moreOpen[S.name + c.dataset.gap] = d.open; return; } if (d.classList && d.classList.contains("prep-hand")) { S.prep.handOpen[S.name + d.dataset.hand] = d.open; return; } if (d.classList && d.classList.contains("prep-srcmore")) { const e = d.querySelector("[data-e]"); if (e) S.prep.openSrc = d.open ? e.dataset.e : (S.prep.openSrc === e.dataset.e ? null : S.prep.openSrc); } }, true);
$("prep").addEventListener("click", async (ev) => {
  const t = ev.target.closest("button, header[data-toggle]");
  if (!t) return;
  const b = t.dataset.b, g = t.dataset.g;
  if (t.dataset.toggle) { const card = t.closest(".prep-gap"); S.prep.cards[S.name + t.dataset.toggle] = card.getAttribute("aria-expanded") !== "true"; prepRender(true); return; }
  if (t.dataset.prepTab) { S.prep.tab = t.dataset.prepTab; S.prep.touched = true; prepRender(true); return; }
  if (t.id === "prep-save") { await prepSave(); return; }
  if (t.id === "prep-kicad-copy") { try { await navigator.clipboard.writeText(kicadCfgPath()); t.textContent = "Copied"; } catch (err) { t.textContent = "Select and copy the path"; } setTimeout(() => { t.textContent = "Copy path"; }, 1500); return; }
  if (t.id === "prep-discard") { prepDiscard(); return; }
  if (t.id === "prep-copy") { try { await navigator.clipboard.writeText($("prep-prompt").textContent); } catch (e) { /* no clipboard (insecure context) */ } $("prep-copied").hidden = false; setTimeout(() => ($("prep-copied").hidden = true), 1500); return; }
  if (!b) return;
  if (b === "stale-restore") { prepStaleRestore(); return; }
  if (b === "stale-discard") { prepStaleDiscard(); return; }
  if (b === "apply") {
    // the step's entries are in the draft already (as typed); Apply commits a filled new row, runs the
    // gap list at once and then marks the step or says what is missing
    prepCommitRows(t.closest(".prep-gap"));
    S.prep.applied[S.name + g] = true; delete S.prep.pending[S.name + g];
    S.prep.applying = { name: S.name, gap: g, tab: S.prep.tab };
    S.prep.saved = ""; prepPersist(); prepUpdate(true); prepRender(true);
    return;
  }
  if (b === "save") { await prepSave(); return; }
  if (b === "goview") { prepOpen(false); $("view-" + t.dataset.view).click(); return; }
  if (b === "dismiss") { S.prep.dismissing = g; prepRender(true); return; }
  // "select all": what the gap still reports (items filled meanwhile are not dismissed)
  if (b === "dis-all") { const gap = prepGapsNow().find((x) => x.id === g) || S.prep.base.gaps.find((x) => x.id === g); const x = prepDis(g, true); x.items = [...new Set([...x.items, ...prepDismissable(gap)])]; }
  else if (b === "undo") { prepDropDis(g); S.prep.dismissing = null; S.prep.cards[S.name + g] = true; }
  else if (b === "dis-drop") { const x = prepDis(g, false); if (x) { x.items = x.items.filter((i) => i !== t.dataset.item); if (!x.items.length) prepDropDis(g); } }
  else if (b === "del-fld") prepSetFld(t.dataset.r, t.dataset.n, "");
  else if (b === "sugg-reg") { const old = prepFld(t.dataset.r, "PI_REG"); prepSetFld(t.dataset.r, "PI_REG", (old ? old.value + "; " : "") + t.dataset.val, "PI_REG"); S.prep.openSrc = `fld:${t.dataset.r}|PI_REG`; }
  else if (b === "cap-sugg") {
    const src = { kind: "design", document: "the parts' fields in the board file", note: t.dataset.from };
    for (const ref of (S.prep.capGroups || {})[g] || []) { const f = prepFld(ref, "PI_CAP"); let v = f ? f.value : ""; const m = prepKV(t.dataset.val); for (const tok of m.toks) v = PREP_DIEL.test(tok) ? prepCapDielSet(v, tok) : prepKVSet(v, tok.split("=")[0], tok.split("=")[1]); const nf = prepSetFld(ref, "PI_CAP", v); if (nf && !prepSrcOk(nf.source)) nf.source = prepClone(src); }
  } else if (b === "no-variant") delete prepW().variant;
  else if (b === "add-var") { const n = String($("prep-newvar").value || "").trim(); if (!n || n.toLowerCase() === "default") return; prepVar(n, true); }
  else if (b === "del-var") { const w = prepW(); delete w.variants[t.dataset.v]; if (w.variant === t.dataset.v) delete w.variant; prepTidy(); }
  else if (b === "take") {
    const src = prepClone(S.prep.takeSrc || {});
    if (!prepSrcOk(src)) return;
    const notes = prepViewerEdits(S.prep.tab).map((e) => e.apply(src)).filter(Boolean);
    S.prep.takeNote = `Taken into the draft${notes.length ? ". " + notes.join(" ") : "."}`;
  } else return;
  prepEdited(true);
});
