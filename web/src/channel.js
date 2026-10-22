"use strict";
// ---- channel view (lanes in lanes.js): interface profiles, transmitter/receiver, eye and checks
//
// Data:   DATA.interfaces (data/interfaces.json) = standard profiles with sources;
//         user overrides per profile in localStorage (PROF_KEY), class → profile
//         assignment per board (ifcKey()).
// Engine: S.engine.rf({..., bitrate, tx, rx, eye_rise}) for one net,
//         kisi_check for many nets (metrics only; status is judged here).

const PROF_KEY = "kisi.profiles.v1";
const ifcKey = () => "kisi.ifc.v1." + S.name;
const IFC = (typeof DATA !== "undefined" && DATA.interfaces) || { defaults: { limits: {}, sources: [] }, profiles: [] };

// deep merge of plain objects (overrides win)
function merge(a, b) {
  if (b == null) return a;
  if (typeof a !== "object" || typeof b !== "object" || Array.isArray(a)) return b;
  const o = { ...a };
  for (const k of Object.keys(b)) o[k] = merge(a[k], b[k]);
  return o;
}
function profOverrides() { return S.profOv || (S.profOv = lsGet(PROF_KEY) || {}); }
function profile(id) {
  const base = IFC.profiles.find((p) => p.id === id);
  if (!base) return null;
  const p = merge(base, profOverrides()[id]);
  p.limits = { ...IFC.defaults.limits, ...(base.limits || {}), ...((profOverrides()[id] || {}).limits || {}) };
  p.edited = !!profOverrides()[id];
  return p;
}
function setProfileValue(id, path, value) {
  const ov = profOverrides(), o = ov[id] || (ov[id] = {});
  let cur = o;
  for (let i = 0; i < path.length - 1; i++) cur = cur[path[i]] || (cur[path[i]] = {});
  cur[path[path.length - 1]] = value;
  lsSet(PROF_KEY, ov);
}
function resetProfile(id) { const ov = profOverrides(); delete ov[id]; lsSet(PROF_KEY, ov); }

// ---- class → profile assignment
function classAssign() {
  if (!S.ifc || S.ifc.board !== S.name) S.ifc = { board: S.name, map: lsGet(ifcKey()) || {} };
  return S.ifc.map;
}
function className(net) { const ci = S.scene.net_class[net] ?? 0; return S.an.classes[ci]?.name || "Default"; }
function autoProfile(net) {
  const t = classTarget(net);
  return S.engine.profileAuto({ net: S.scene.nets[net] || "", class: className(net), diff: !!(t && t.diff) });
}
function cfgProfile(cls) { return (S.config && S.config.profiles && S.config.profiles[cls]) || null; }
function profileIdOf(net) { const c = className(net); return classAssign()[c] || cfgProfile(c) || autoProfile(net); }
function profileOf(net) { return profile(profileIdOf(net)) || profile("se50"); }
// the end of a lane at a connector (termination when the connector receives, driver when it
// sends) and the channel behind it, per connector (interface): the viewer's choice (stored per
// board, set in the bus card of the interfaces overview) over kisi.json (connector_ends.<ref>);
// null = the profile's receiver / transmitter, no channel behind
const connEndKey = () => "kisi.connend.v2." + S.name;
function connEnds() {
  if (!S.cend || S.cend.board !== S.name) S.cend = { board: S.name, map: lsGet(connEndKey()) || {} };
  return S.cend.map;
}
function cfgConnEnd(ref) { return (S.config && S.config.connector_ends && S.config.connector_ends[ref]) || null; }
function connEndOf(ref) { return connEnds()[ref] || cfgConnEnd(ref) || null; }
function laneConnRef(l) { const e = l.kind === "ic-connector" ? l.ends.find((x) => x.role === "connector") : null; return e ? e.refdes : null; }
function laneConnEnd(l) { const r = laneConnRef(l); return r ? connEndOf(r) : null; }
const CONN_END_NAME = { profile: "as the profile", matched: "matched (Z₀)", high_impedance: "high impedance / ideal source", custom: "custom R, C" };

// ---- metrics and judgement (the verdict comes from kisi-si::profile via WASM,
// identical for viewer and CLI; here only the presentation)
const ST_MAP = { green: "ok", yellow: "high1", red: "high2" };
function judge(m, p) {
  let v;
  try { v = S.engine.judge({ metrics: m, profile: p, limits: p.limits }); }
  catch (e) { return { rows: [{ key: "err", name: "Verdict", value: e.message, limit: "", st: "high2" }], status: "high2" }; }
  const nyq = p.rate_gbps / 2, f1 = (x, d = 1) => fmt(x, d);
  const rows = v.criteria.map((c) => {
    const st = ST_MAP[c.status];
    switch (c.key) {
      case "error": return { key: c.key, name: "Computation", value: m.error || "error", limit: "", st };
      case "z": return { key: c.key, name: "Impedance mean (min … max)", value: `${f1(c.value)} Ω (${fmt(m.z_min, 0)} … ${fmt(m.z_max, 0)})`, limit: `${f1(c.lo)} … ${f1(c.hi)} Ω`, st, note: c.note };
      case "eye_h": return { key: c.key, name: "Eye height", value: `${fmt(c.value * 100, 0)} % (${fmt(c.value * p.tx.swing_mv, 0)} mV)`, limit: `red < ${fmt(c.red_below * 100, 0)} %, yellow < ${fmt(c.lo * 100, 0)} %`, st };
      case "eye_w": return { key: c.key, name: "Eye width", value: `${fmt(c.value * 100, 0)} % UI`, limit: `≥ ${fmt(c.lo * 100, 0)} % UI`, st };
      case "il": return { key: c.key, name: `Insertion loss at ${fmt(nyq, nyq < 1 ? 2 : 1)} GHz`, value: `${fmt(c.value, 2)} dB`, limit: c.lo != null ? `≥ ${f1(c.lo)} dB` : "no limit", st };
      case "rl": return { key: c.key, name: "Return loss", value: `${f1(c.value)} dB`, limit: `≥ ${fmt(c.lo, 0)} dB up to ${f1(1.5 * nyq)} GHz`, st };
      case "skew": return { key: c.key, name: "Intra-pair skew", value: `${f1(c.value)} ps${m.skew_uncertain ? " (uncertain)" : ""}`, limit: `≤ ${f1(c.hi)} ps`, st, note: c.note };
      case "stub": return { key: c.key, name: "Via stub resonance", value: `${f1(c.value)} GHz`, limit: `> ${f1(c.lo)} GHz`, st };
      default: return { key: c.key, name: c.key, value: String(c.value), limit: "", st };
    }
  });
  return { rows, status: ST_MAP[v.status] };
}
const ST_NAME = { ok: "green", high1: "yellow", high2: "red" };

// ---- side cards
function renderIfcCard() {
  const box = $("ifc-body");
  if (!box || !S.an) return;
  const map = classAssign();
  const classes = S.an.classes.map((c, i) => ({ c, i })).filter(({ c }) => c.nets > 0 && c.target);
  const opts = (sel, auto, cfgId) => `<option value="">${cfgId ? `kisi.json (${esc(profile(cfgId)?.name || cfgId)})` : `automatic (${esc(profile(auto)?.name || auto)})`}</option>` +
    IFC.profiles.map((p) => `<option value="${p.id}" ${p.id === sel ? "selected" : ""}>${esc(p.name)}</option>`).join("");
  box.innerHTML = classes.length ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Class</th><th>Standard (profile)</th></tr></thead><tbody>${classes.map(({ c }) => {
    const anyNet = S.an.nets.find((n) => n.class === c.name);
    const auto = anyNet ? autoProfile(anyNet.net) : "se50";
    return `<tr><td title="${c.nets} nets, target ${fmt(c.target.z, 0)} Ω ${c.target.diff ? "diff" : "SE"}">${esc(c.name)}</td><td><select data-cls="${esc(c.name)}" aria-label="Profile for ${esc(c.name)}">${opts(map[c.name] || "", auto, cfgProfile(c.name))}</select></td></tr>`;
  }).join("")}</tbody></table></div>
    <p class="note">Automatic: by net name first, then by class name (rules in data/interfaces.json)${S.config ? `; “kisi.json”: from ${esc(S.config.name)}` : ""}. Transmitter, receiver and limits of a standard: “Profile” card; what sits at a connector: the interface's card in the overview. Your choice is stored per board.</p>` : `<p class="note">No net classes with target impedance.</p>`;
  box.querySelectorAll("select[data-cls]").forEach((s) => s.onchange = () => {
    const m = classAssign();
    if (s.value) m[s.dataset.cls] = s.value; else delete m[s.dataset.cls];
    lsSet(ifcKey(), m); S.chCache = null; markCheckStale(); renderProfCard(); if (S.selNet >= 0 && S.tab === "ch") selectNet(S.selNet, false);
  });
}

// connector end controls for one connector in a bus card of the interfaces overview:
// termination or driver there and the channel behind it (none, a lossy line or a Touchstone file)
function connEndControls(ref, roles, target, diff) {
  const own = connEnds()[ref], cfg = cfgConnEnd(ref), cur = own || cfg || { model: "profile" };
  const what = roles.has("rx") && roles.has("tx") ? "termination / driver" : roles.has("rx") ? "termination" : "driver";
  const o = `<option value="">${cfg ? `kisi.json (${esc(CONN_END_NAME[cfg.model || "profile"] || cfg.model)})` : "as the profile"}</option>` +
    ["matched", "high_impedance", "custom"].map((m) => `<option value="${m}" ${own && own.model === m ? "selected" : ""}>${CONN_END_NAME[m]}</option>`).join("");
  const custom = cur.model === "custom" ? `<label>R <input type="number" min="0" step="any" data-ce-r="${esc(ref)}" value="${cur.r_ohm ?? ""}"> Ω</label><label>C <input type="number" min="0" step="any" data-ce-c="${esc(ref)}" value="${cur.c_pf ?? ""}"> pF</label>` : "";
  const mode = cur.touchstone ? "ts" : cur.rest_loss_db != null ? "line" : "";
  const rm = `<label><select data-ce-rest="${esc(ref)}" aria-label="Channel behind connector ${esc(ref)}"><option value="" ${mode === "" ? "selected" : ""}>none</option><option value="line" ${mode === "line" ? "selected" : ""}>lossy line</option><option value="ts" ${mode === "ts" ? "selected" : ""}>Touchstone file</option></select></label>`;
  let rest = "";
  if (mode === "line") rest = `<label><input type="number" min="0" step="any" data-ce-loss="${esc(ref)}" value="${cur.rest_loss_db ?? ""}" aria-label="Loss behind connector ${esc(ref)}"> dB at Nyquist</label><label><input type="number" min="0" step="any" data-ce-delay="${esc(ref)}" value="${cur.rest_delay_ps ?? ""}" placeholder="500" aria-label="Delay behind connector ${esc(ref)}"> ps</label>`;
  if (mode === "ts") {
    const tr = cur.touchstone, name = (tr.file || "").split(/[\\/]/).pop();
    const names = [...new Set([...Object.keys(S.tsFiles), ...(name ? [name] : [])])];
    const f = S.tsFiles[name];
    const info = !name ? "" : !f ? `<span class="err">not loaded</span>` : f.error ? `<span class="err">${esc(f.error)}</span>` : `<span class="muted">${f.info.nports}-port, ${fmt(f.info.f_min_ghz, 2)}–${fmt(f.info.f_max_ghz, 1)} GHz${f.info.nports === 2 && diff ? ", the same on each line" : ""}</span>`;
    rest = `<label><select data-ce-ts="${esc(ref)}" aria-label="Touchstone file behind connector ${esc(ref)}">${names.length ? "" : '<option value="">– load a file –</option>'}${names.map((n) => `<option value="${esc(n)}" ${n === name ? "selected" : ""}>${esc(n)}</option>`).join("")}</select></label>
      <label class="btn small ce-load">Load…<input type="file" accept=".s2p,.s4p" data-ce-file="${esc(ref)}" hidden></label>${info}
      ${f && f.info && f.info.nports === 4 ? `<label>ports <select data-ce-po="${esc(ref)}">${["1-3,2-4", "1-2,3-4"].map((p) => `<option value="${p}" ${(tr.port_order || "1-3,2-4") === p ? "selected" : ""}>${p === "1-3,2-4" ? "P 1→3, N 2→4" : "P 1→2, N 3→4"}</option>`).join("")}</select></label>` : ""}
      <label title="port 2 of the file at the connector (else port 1)"><input type="checkbox" data-ce-flip="${esc(ref)}" ${tr.flip ? "checked" : ""}> port 2 at the connector</label>`;
  }
  return `<div class="ce-box" data-ce-target="${esc(JSON.stringify(target || null))}">
    <span class="ce-h">Connector <b>${esc(ref)}</b> · ${what}</span>
    <label><select data-ce="${esc(ref)}" aria-label="End at connector ${esc(ref)}">${o}</select></label>${custom}
    <span class="ce-h" title="The channel behind the connector (cable, mating board): a lossy line at the target impedance (loss at the profile's Nyquist frequency, half √f, half f) or a Touchstone file (.s2p, also on each line of a pair; .s4p of a pair). None: the eye covers the board share only and is information.">behind</span>${rm}${rest}
  </div>`;
}
function bindConnEnd(box, after) {
  const save = () => { lsSet(connEndKey(), connEnds()); S.chCache = null; markCheckStale(); after(); };
  const entry = (ref) => { const m = connEnds(); if (!m[ref]) m[ref] = JSON.parse(JSON.stringify(cfgConnEnd(ref) || { model: "profile" })); return m[ref]; };
  box.querySelectorAll("select[data-ce-rest]").forEach((s) => s.onchange = () => {
    const e = entry(s.dataset.ceRest);
    if (s.value !== "line") { delete e.rest_loss_db; delete e.rest_delay_ps; }
    if (s.value !== "ts") delete e.touchstone;
    if (s.value === "line") e.rest_loss_db = e.rest_loss_db ?? 0;
    if (s.value === "ts") e.touchstone = { file: Object.keys(S.tsFiles)[0] || "" };
    save();
  });
  box.querySelectorAll("select[data-ce-ts]").forEach((s) => s.onchange = () => { const e = entry(s.dataset.ceTs); e.touchstone = { ...(e.touchstone || {}), file: s.value }; save(); });
  box.querySelectorAll("input[data-ce-file]").forEach((inp) => inp.onchange = async () => {
    const f = inp.files && inp.files[0]; if (!f) return;
    loadTouchstone(f.name, await f.text());
    const e = entry(inp.dataset.ceFile); e.touchstone = { ...(e.touchstone || {}), file: f.name }; save();
  });
  box.querySelectorAll("select[data-ce-po]").forEach((s) => s.onchange = () => { const e = entry(s.dataset.cePo); e.touchstone.port_order = s.value; save(); });
  box.querySelectorAll("input[data-ce-flip]").forEach((c) => c.onchange = () => { const e = entry(c.dataset.ceFlip); e.touchstone.flip = c.checked; if (!c.checked) delete e.touchstone.flip; save(); });
  box.querySelectorAll("select[data-ce]").forEach((s) => s.onchange = () => {
    const m = connEnds(), ref = s.dataset.ce, keep = m[ref] || cfgConnEnd(ref) || {};
    const t = JSON.parse(s.closest(".ce-box").dataset.ceTarget || "null");
    const rest = { ...(keep.rest_loss_db != null ? { rest_loss_db: keep.rest_loss_db, rest_delay_ps: keep.rest_delay_ps } : {}), ...(keep.touchstone ? { touchstone: keep.touchstone } : {}) };
    if (!s.value && !Object.keys(rest).length) delete m[ref];
    else m[ref] = { ...(s.value === "custom" ? { model: "custom", r_ohm: keep.r_ohm ?? (t ? (t[1] ? t[0] / 2 : t[0]) : 50), c_pf: keep.c_pf ?? 0 } : { model: s.value || "profile" }), ...rest };
    save();
  });
  box.querySelectorAll("input[data-ce-loss], input[data-ce-delay]").forEach((inp) => inp.onchange = () => {
    const m = connEnds(), ref = inp.dataset.ceLoss ?? inp.dataset.ceDelay, v = inp.value === "" ? null : Math.max(0, parseFloat(inp.value));
    const e = m[ref] || { ...(cfgConnEnd(ref) || { model: "profile" }) };
    if (inp.dataset.ceLoss != null) { if (v == null) { delete e.rest_loss_db; delete e.rest_delay_ps; } else e.rest_loss_db = v; }
    else if (v == null) delete e.rest_delay_ps; else e.rest_delay_ps = v;
    m[ref] = e; save();
  });
  box.querySelectorAll("input[data-ce-r], input[data-ce-c]").forEach((inp) => inp.onchange = () => {
    const m = connEnds(), ref = inp.dataset.ceR ?? inp.dataset.ceC, v = inp.value === "" ? null : Math.max(0, parseFloat(inp.value));
    if (!m[ref]) return;
    if (inp.dataset.ceR != null) m[ref].r_ohm = v ?? 0; else m[ref].c_pf = v ?? 0;
    save();
  });
}

function renderProfCard() {
  const box = $("prof-body");
  if (!box) return;
  const id = S.profEdit || (S.selNet >= 0 ? profileIdOf(S.selNet) : "pcie4");
  S.profEdit = id;
  const p = profile(id);
  const num = (path, label, unit, step = "any", hint = "") => {
    const v = path.reduce((o, k) => (o == null ? o : o[k]), p);
    return `<label class="pf"><span>${label}${hint ? ` <i class="muted">${hint}</i>` : ""}</span><span><input type="number" step="${step}" data-path="${path.join(".")}" value="${v == null ? "" : v}" placeholder="–"> ${unit}</span></label>`;
  };
  const src = (p.sources || []).concat(IFC.defaults.sources || []).map((s) => `<li><b>${{ spec: "Standard", guideline: "Guideline", kisi: "kisi" }[s.kind] || s.kind}:</b> ${esc(s.text)}${s.url ? ` <a href="${esc(s.url)}" target="_blank" rel="noopener">source</a>` : ""}</li>`).join("");
  box.innerHTML = `<label class="opt" for="prof-sel"><span>Profile</span><select id="prof-sel">${IFC.profiles.map((q) => `<option value="${q.id}" ${q.id === id ? "selected" : ""}>${esc(q.name)}${profOverrides()[q.id] ? " *" : ""}</option>`).join("")}</select></label>
    <div class="pf-grid">
      <h4>Signal</h4>
      ${num(["rate_gbps"], "Data rate", "Gbit/s")}${num(["z"], `Target ${p.signal === "diff" ? "diff." : "single-ended"}`, "Ω")}${num(["z_tol_pct"], "Tolerance", "%")}
      <h4>Transmitter <i class="muted">per line</i></h4>
      ${num(["tx", "swing_mv"], "Swing", "mV")}${num(["tx", "r"], "Output resistance", "Ω")}${num(["tx", "c_pf"], "Pin capacitance", "pF")}${num(["tx", "l_nh"], "Package L", "nH")}${num(["tx", "rise_ps"], "Rise 10–90 %", "ps")}
      <h4>Receiver <i class="muted">per line</i></h4>
      ${num(["rx", "r"], "Termination", "Ω", "any", "0 = high impedance")}${num(["rx", "c_pf"], "Pin capacitance", "pF")}${num(["rx", "l_nh"], "Package L", "nH")}
      <h4>Limits</h4>
      ${num(["limits", "eye_h_red"], "Eye red below", "× swing", "0.05")}${num(["limits", "eye_h_yellow"], "Eye yellow below", "× swing", "0.05")}${num(["limits", "eye_w_min_ui"], "Eye width min.", "UI", "0.05")}
      ${num(["limits", "skew_ps"], "Skew max.", "ps", "any", "empty = 5 % UI")}${num(["limits", "rl_min_db"], "Return loss min.", "dB")}${num(["limits", "stub_factor"], "Stub resonance above", "× Nyquist")}${num(["limits", "il_board_db"], "Board loss min.", "dB", "any", "empty = none")}
    </div>
    ${p.il_total ? `<p class="note">Total budget per source: ${fmt(p.il_total.db, 0)} dB at ${fmt(p.il_total.f_ghz, 0)} GHz – ${esc(p.il_total.scope)}.</p>` : ""}
    <div class="fs-row"><span class="note">${p.edited ? "Changed from the default (stored in this browser)." : "Default values."}</span><button class="btn small" id="prof-reset" ${p.edited ? "" : "disabled"}>Reset</button></div>
    <details><summary>Sources</summary><ul class="src">${src}</ul></details>`;
  $("prof-sel").onchange = (e) => { S.profEdit = e.target.value; renderProfCard(); };
  $("prof-reset").onclick = () => { resetProfile(id); profChanged(); };
  box.querySelectorAll("input[data-path]").forEach((inp) => inp.onchange = () => {
    const v = inp.value === "" ? null : parseFloat(inp.value);
    setProfileValue(id, inp.dataset.path.split("."), v);
    profChanged();
  });
}
function profChanged() { S.chCache = null; markCheckStale(); renderProfCard(); renderIfcCard(); if (S.selNet >= 0 && S.tab === "ch") selectNet(S.selNet, false); }
function markCheckStale() { if (S.check) S.check.stale = true; renderCheck(); if (S.tab === "ch" && S.chMode === "o") renderChOverview(); }

// ---- one net: channel with the profile's ends
function chFor(net) {
  const t = classTarget(net), p = profileOf(net), rp = rfParams();
  rp.fmax = Math.max(rp.fmax, p.rate_gbps * 1.25e9);
  const key = `${net}|${S.anGen}|${JSON.stringify(rp)}|${JSON.stringify(p)}`;
  if (S.chCache && S.chCache.key === key) return S.chCache.res;
  let res;
  try {
    res = S.engine.rf({ net, rf: rp, target: t ? [t.z, t.diff] : null, bitrate: p.rate_gbps * 1e9, tx: p.tx, rx: p.rx, eye_rise: p.tx.rise_ps * 1e-12 });
  } catch (e) { res = { channel: null, error: e.message, crosstalk: { aggressors: [], geoms: [] } }; }
  res.profile = p;
  S.chCache = { key, res };
  return res;
}
function zStats(net) {
  const st = S.an.nets.find((x) => x.net === net);
  return st && st.z_mean != null ? { z_mean: st.z_mean, z_min: st.z_min, z_max: st.z_max } : {};
}
function metricsOf(r, net, lane) {
  const ch = r.channel, p = r.profile;
  const zs = lane ? laneZStats(lane) : zStats(net);
  if (!ch) return { error: r.error || "no channel", ...zs };
  const nyq = p.rate_gbps * 0.5e9, df = ch.freq[0];
  const k = Math.min(ch.freq.length, Math.max(1, Math.round(nyq / df))) - 1;
  const k2 = Math.min(ch.freq.length, Math.max(1, Math.round(1.5 * nyq / df)));
  const stubs = ch.via_info.filter((v) => v.stub_mm > 0.01 && v.stub_f_ghz != null).map((v) => v.stub_f_ghz);
  // the eye of a lane ending at a connector without a channel model behind it: information only
  const share = !!lane && lane.kind === "ic-connector" && !r.rest_applied;
  const e = r.eye && !r.eye.error && !share ? r.eye : null;
  return {
    il_nyq_db: ch.s21_db[k], rl_db: -Math.max(...ch.s11_db.slice(0, k2)),
    eye_h: e ? e.height : null, eye_w_ui: e ? e.width_ps / e.ui_ps : null,
    skew_ps: r.skew ? r.skew.skew_ps : null, skew_uncertain: r.skew ? r.skew.uncertain : false,
    stub_ghz: stubs.length ? Math.min(...stubs) : null, error: r.eye && r.eye.error && !share ? r.eye.error : null, ...zs,
  };
}
function renderChDetail(net) {
  const d = $("ch-detail"), st = S.an.nets.find((x) => x.net === net);
  const lane = S.an.skeleton ? null : (S.selLane && allLanes().find((l) => laneKey(l) === S.selLane && (l.nets.includes(net) || l.partner_nets.includes(net)))) || laneOfNet(net);
  if (lane && CHECKABLE.has(lane.kind)) return renderLaneDetail(lane);
  // a net on the board map that is no channel: not used in this variant, or no target
  const why = netUnused(net) ? `Not used in this assembly variant${variantShown() ? ` (${esc(variantShown())})` : ""}: the net only connects parts that are not fitted. It has no lane and is not checked; the other variant uses it.`
    : !variantNeeded() && !chActiveNets().has(net) ? (lane ? `Part of a ${esc(lane.kind === "ic-ic" || lane.kind === "ic-connector" ? lane.kind : "multi-drop or open")} line (${esc(lane.ends.map((e) => e.refdes).join(", ") || "no devices")}): listed in the overview, not checked.` : "No channel: the net has no target impedance (net class) and is on no lane.") : null;
  if (why) {
    d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header><div id="ch-body"><p class="note">${why}</p></div>`;
    d.hidden = false; $("closeCh").onclick = () => selectNet(-1);
    return;
  }
  if (S.an.skeleton) {
    d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header><div id="ch-body"></div>`;
    d.hidden = false; $("closeCh").onclick = () => selectNet(-1); needEstimate($("ch-body"));
    const sel = S.selLane && allLanes().find((l) => laneKey(l) === S.selLane && (l.nets.includes(net) || l.partner_nets.includes(net)));
    if (sel) highlightLane(sel);
    return;
  }
  const r = chFor(net), p = r.profile;
  const partner = st && st.partner != null ? S.scene.nets[st.partner] : null;
  const tab = S.chTab || "check";
  d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}${partner ? " · pair with " + esc(partner) : ""}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header>
    <div class="dstats"><span>Profile <b>${esc(p.name)}</b>${p.edited ? " *" : ""}</span><span><button class="btn small" id="ch-edit">Edit profile</button></span></div>
    <div class="dstats muted"><span>Transmitter ${fmt(p.tx.r, 0)} Ω${p.tx.c_pf ? ", " + fmt(p.tx.c_pf, 1) + " pF" : ""}, ${fmt(p.tx.rise_ps, 0)} ps, ${fmt(p.tx.swing_mv, 0)} mV</span><span>Receiver ${p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "high impedance"}${p.rx.c_pf ? ", " + fmt(p.rx.c_pf, 1) + " pF" : ""}</span><span>per line</span></div>
    <div class="tabs" role="tablist" aria-label="Channel">${[["check", "Check"], ["eye", "Eye"], ["path", "Path"]].map(([k, n]) => `<button role="tab" data-ct="${k}" aria-selected="${tab === k}">${n}</button>`).join("")}</div>
    <div id="ch-body"></div>`;
  d.hidden = false;
  $("closeCh").onclick = () => selectNet(-1);
  $("ch-edit").onclick = () => { S.profEdit = p.id; renderProfCard(); $("h-prof").scrollIntoView({ behavior: "smooth", block: "start" }); };
  d.querySelectorAll("[data-ct]").forEach((b) => b.onclick = () => { S.chTab = b.dataset.ct; renderChDetail(net); });
  const body = $("ch-body");
  if (tab === "eye") return renderEye(body, r);
  if (tab === "path") return renderNetPath(body, net, r);
  const m = metricsOf(r, net), j = judge(m, p);
  const xt = r.crosstalk && r.crosstalk.aggressors ? `<tr><td></td><td>Crosstalk (total)</td><td>NEXT ${fmt(r.crosstalk.next_total * 100, 2)} %, FEXT ${fmt(r.crosstalk.fext_total * 100, 2)} %</td><td>information only</td></tr>` : "";
  body.innerHTML = `<div class="dstats"><span>Result <b style="color:${S.colors[j.status]}">${ST_NAME[j.status]}</b></span><span>${fmt(p.rate_gbps, p.rate_gbps < 1 ? 2 : p.rate_gbps % 1 ? 2 : 0)} Gbit/s</span>${r.channel ? `<span>Path ${fmt(r.channel.length_mm, 1)} mm, ${r.channel.vias} vias</span>` : ""}</div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th></th><th>Criterion</th><th>Value</th><th>Limit</th></tr></thead><tbody>
    ${j.rows.map((x) => `<tr title="${esc(x.note || "")}"><td><span class="dot" style="background:${S.colors[x.st]}" aria-label="${ST_NAME[x.st]}"></span></td><td>${x.name}</td><td>${esc(String(x.value))}</td><td>${x.limit}</td></tr>`).join("")}${xt}
    </tbody></table></div>
    <p class="note">Impedance from the impedance analysis (formula or field solver) against the profile. Insertion loss, return loss and TDR use the reference resistance; the eye uses the profile's transmitter and receiver. No equalisation, jitter or noise.</p>
    ${(r.channel?.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
}

// nets of the channels the check covers in this assembly variant: the lanes from device to
// device or connector, and nets with a target on no lane (board map before the check)
function chActiveNets() {
  const c = S.chActive;
  if (c && c.buses === S.buses && c.gen === S.anGen && c.variant === S.variant) return c.set;
  const set = new Set(), onLane = new Set();
  if (S.buses && S.an && !variantNeeded()) {
    for (const l of allLanes()) {
      for (const n of [...l.nets, ...l.partner_nets]) { onLane.add(n); if (CHECKABLE.has(l.kind)) set.add(n); }
    }
    for (const st of S.an.nets) {
      if (onLane.has(st.net) || netUnused(st.net) || !classTarget(st.net)) continue;
      set.add(st.net);
      if (st.partner != null) set.add(st.partner);
    }
  }
  S.chActive = { buses: S.buses, gen: S.anGen, variant: S.variant, set };
  return set;
}

// path of a net with a target that is on no lane: from end A to end B of its longest path
function renderNetPath(body, net, r) {
  const ch = r.channel, np = netPath(net);
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  const br = np ? np.branches.map((b) => `${fmt(b.len_mm, 1)} mm → ${esc(b.end.label)}`) : [];
  body.innerHTML = `<div class="dstats">${np ? `<span>A ${esc(np.start.label)} → B ${esc(np.end.label)}</span>` : ""}<span>${fmt(ch.length_mm, 1)} mm, delay ${fmt(ch.delay_ps, 0)} ps, ${ch.vias} vias</span></div>
    ${pathRows(ch)}
    <p class="note">No lane: kisi found no pair of devices (ICs, connectors) at the ends of this net through series resistors (≤ 100 Ω), AC-coupling capacitors (≥ 1 nF) or small inductors. It is checked on its own, from end A to end B of its longest path, with the profile's transmitter and receiver.${br.length ? ` Branches (open stubs): ${br.join("; ")}.` : ""}${np && np.parts > 1 ? ` ${np.parts} copper parts; the longest path is shown.` : ""}</p>`;
}

// ---- board-level check
function checkItems() {
  const out = [];
  const onLane = new Set();
  for (const l of allLanes()) {
    for (const n of [...l.nets, ...l.partner_nets]) onLane.add(n);
    if (!CHECKABLE.has(l.kind)) continue;
    const net = laneNet(l), t = classTarget(net), p = laneProfile(l);
    out.push({ lane: l.nets[0], to: l.ends[l.ends.length - 1].refdes, reversed: laneReversed(l), laneKey: laneKey(l), net, gbps: p.rate_gbps, target: t ? [t.z, t.diff] : null, tx: p.tx, rx: p.rx, rise: p.tx.rise_ps * 1e-12, conn_end: laneConnEnd(l) });
  }
  // nets with a target that are not part of a device-to-device lane (no devices found); nets
  // that only connect parts not fitted have no function in this assembly
  for (const st of S.an.nets) {
    const t = classTarget(st.net);
    if (!t || onLane.has(st.net) || netUnused(st.net)) continue;
    if (st.partner != null) {
      // one line per pair, preferably the positive line
      const neg = /(_N|N|-|_M)$/i.test(st.name), pneg = /(_N|N|-|_M)$/i.test(S.scene.nets[st.partner] || "");
      if (neg !== pneg ? neg : st.partner < st.net) continue;
    }
    const p = profileOf(st.net);
    out.push({ net: st.net, gbps: p.rate_gbps, target: [t.z, t.diff], tx: p.tx, rx: p.rx, rise: p.tx.rise_ps * 1e-12 });
  }
  return out;
}
// the eye of a lane ending at a connector (board share) is shown, not judged – as in the CLI
const eyeJudged = (r) => !(r.error || r.eye === false || r.eye_board_share);
const rowMetrics = (r, lane) => ({ ...(lane ? laneZStats(lane) : zStats(r.net)), il_nyq_db: r.il_nyq_db, rl_db: r.rl_db, eye_h: eyeJudged(r) ? r.eye_height : null, eye_w_ui: eyeJudged(r) ? r.eye_width_ui : null, skew_ps: r.skew_ps, skew_uncertain: r.skew_uncertain, stub_ghz: r.stub_ghz, error: r.error });
async function runCheck() {
  if (variantNeeded()) return;
  const items = checkItems(), rows = [], gen = S.anGen, p = rfParams();
  const btn = $("ck-start"); btn.disabled = true; S.checkRunning = true;
  const lanes = new Map(allLanes().map((l) => [laneKey(l), l]));
  $("ck-barbox").hidden = false;
  for (let i = 0; i < items.length; i += 4) {
    if (gen !== S.anGen) break;
    $("ck-text").textContent = `Checking ${Math.min(i + 4, items.length)} of ${items.length} channels …`;
    $("ck-bar").style.width = `${Math.round(100 * i / Math.max(items.length, 1))}%`;
    await new Promise((res) => requestAnimationFrame(() => setTimeout(res, 0)));
    const chunk = items.slice(i, i + 4);
    try {
      S.engine.call(S.engine.k.kisi_check, JSON.stringify({ rf: p, items: chunk }));
      JSON.parse(S.engine.out()).forEach((r, k) => { r.laneKey = chunk[k].laneKey; r.net = chunk[k].laneKey ? chunk[k].net : r.net; rows.push(r); });
    } catch (e) { console.warn(e); }
  }
  S.checkRunning = false; $("ck-barbox").hidden = true;
  S.check = { gen, rows, stale: false };
  S.chStatus = new Map();
  S.laneRes = new Map();
  for (const r of rows) {
    const lane = r.laneKey ? lanes.get(r.laneKey) : null;
    r.lane = lane;
    const st = judge(rowMetrics(r, lane), lane ? laneProfile(lane) : profileOf(r.net)).status;
    if (lane) {
      S.laneRes.set(r.laneKey, r);
      for (const n of [...lane.nets, ...lane.partner_nets]) if (!S.chStatus.has(n) || st === "high2" || (st === "high1" && S.chStatus.get(n) === "ok")) S.chStatus.set(n, st);
    } else {
      S.chStatus.set(r.net, st);
      const n = S.an.nets.find((x) => x.net === r.net);
      if (n && n.partner != null) S.chStatus.set(n.partner, st);
    }
  }
  if (S.tab === "ch") { if (S.chMode === "o") renderChOverview(); else { buildPaths(); if (S.selNet >= 0) selectNet(S.selNet, false); renderLegend(); draw(); } }
  renderCheck();
}
// side card "Channel check": four steps like the impedance and power cards (number → ✓ when
// done, the next step greyed until the one before is done), then all channels, worst first
function renderCheck() {
  const box = $("ck-out");
  if (!box || !S.an) return;
  const need = variantNeeded(), skel = !!S.an.skeleton;
  const stale = !!S.check && (S.check.gen !== S.anGen || S.check.stale);
  // 1 lanes and profiles (on loading; with variants after the choice)
  const ls = need || !S.buses ? [] : allLanes();
  const nIc = ls.filter((l) => l.kind === "ic-ic").length, nCon = ls.filter((l) => l.kind === "ic-connector").length;
  $("cs1").classList.toggle("done", !need && !!S.buses);
  $("cs1-text").textContent = need ? "Choose the assembly variant (card above): lanes depend on the fitted parts."
    : `${(S.buses || []).length} buses: ${nIc} lanes device to device, ${nCon} device to connector, ${ls.length - nIc - nCon} not checked (multi-drop, open). Profiles per net class (“Interfaces per net class”).${variantShown() ? ` Variant ${variantShown()}.` : ""}`;
  // 2 impedance estimate (the same as step 2 of the impedance view)
  $("cs2").classList.toggle("done", !skel);
  $("ck-est").hidden = !skel;
  $("cs2-text").textContent = skel ? "Closed-form formulas for every track, about a second. Loss, eye and skew need it."
    : "Done: formula values for every track.";
  // 4 the check of all channels (the field solver, step 3, is optional)
  const locked = need || skel;
  $("cs3").classList.toggle("locked", locked);
  $("cs3").classList.toggle("done", !!S.check && !stale && !locked);
  const btn = $("ck-start");
  btn.textContent = S.check ? "Recompute" : "Compute";
  btn.disabled = locked || !!S.checkRunning;
  $("ck-count").textContent = S.check && S.check.rows.length && !need ? `${S.check.rows.length} channels` : "";
  if (S.checkRunning) return;
  if (locked || !S.check || !S.check.rows.length) {
    box.innerHTML = "";
    $("ck-text").textContent = locked ? "Available after steps 1 and 2." : "All lanes against the profile of their class: eye (IC to IC) with transmitter and receiver, insertion loss, return loss, skew, via stubs; nets with a target but no devices on their own. A few seconds.";
    return;
  }
  const rows = S.check.rows.map((r) => ({ r, p: r.lane ? laneProfile(r.lane) : profileOf(r.net) })).map((x) => ({ ...x, st: judge(rowMetrics(x.r, x.r.lane), x.p).status }));
  const rank = { high2: 0, high1: 1, ok: 2 };
  rows.sort((a, b) => rank[a.st] - rank[b.st] || (a.r.eye_height ?? 0) - (b.r.eye_height ?? 0));
  const n = { high2: 0, high1: 0, ok: 0 }; rows.forEach((x) => n[x.st]++);
  const nl = rows.filter((x) => x.r.lane).length;
  $("ck-text").textContent = `${stale ? "Outdated (settings changed): compute again. " : "Done. "}${rows.length} channels (${nl} lanes${rows.length > nl ? `, ${rows.length - nl} nets without devices` : ""}): ${n.ok} green, ${n.high1} yellow, ${n.high2} red. Click a row for the channel.`;
  box.innerHTML = `<div class="ck-scroll"><table class="xs-table ck-table"><thead><tr><th></th><th>Net</th><th>Profile</th><th>IL dB</th><th>Eye %</th><th>Skew ps</th></tr></thead><tbody>${rows.map(({ r, p, st }) => {
    const nm = netShort(r.name) + (r.lane ? ` (${r.lane.ends.map((e) => e.refdes).join("–")})` : "");
    const sk = r.skew_ps == null ? "–" : fmt(Math.abs(r.skew_ps), 1) + (r.skew_uncertain ? "?" : "");
    return `<tr class="click" data-net="${r.net}" title="${esc(r.name)} · ${r.diff ? "pair" : "single-ended"}, ${fmt(r.length_mm, 1)} mm, ${r.vias} vias, RL ${fmt(r.rl_db, 1)} dB${r.stub_ghz != null ? ", stub resonance " + fmt(r.stub_ghz, 1) + " GHz" : ""}${r.error ? " · " + esc(r.error) : ""}"><td><span class="dot" style="background:${S.colors[st]}" aria-label="${ST_NAME[st]}"></span></td><td>${esc(nm)}</td><td>${esc(p.name.split(" (")[0])}</td><td>${fmt(r.il_nyq_db, 1)}</td><td>${r.error || r.eye === false ? "–" : fmt(r.eye_height * 100, 0)}</td><td>${sk}</td></tr>`;
  }).join("")}</tbody></table></div>
  <p class="note">Limits per profile (“Profile” card). “?” = branched path, skew uncertain.</p>`;
  box.querySelectorAll("tr[data-net]").forEach((tr, i) => tr.onclick = () => { S.chTab = "check"; const r = rows[i].r; S.selLane = r.laneKey || null; if (S.chMode === "o") setChMode("m"); selectNet(+tr.dataset.net); });
}

function renderEye(body, r) {
  const ch = r.channel, e = r.eye, p = r.profile;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  if (!e || e.error) { body.innerHTML = `<p class="err">${esc(e?.error || "no eye diagram")}</p>`; return; }
  const g = p.rate_gbps, uiPct = (x) => fmt(100 * x / e.ui_ps, 0);
  const warn = (e.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  body.innerHTML = `<div class="dstats"><span>${fmt(g, g < 1 ? 2 : g % 1 ? 2 : 0)} Gbit/s NRZ, PRBS7</span><span>UI ${fmt(e.ui_ps, 1)} ps</span><span>DC level ${fmt(e.dc * 100, 0)} % of swing</span></div>
    <div class="dstats"><span>Eye height <b>${fmt(e.height * 100, 0)} %</b> = ${fmt(e.height * p.tx.swing_mv, 0)} mV</span><span>Eye width <b>${fmt(e.width_ps, 1)} ps</b> (${uiPct(e.width_ps)} % UI)</span><span>ISI total <b>${fmt(e.isi_sum * 100, 0)} %</b></span></div>
    <div class="chart"><canvas id="ch-eye" style="height:220px" aria-label="Eye diagram"></canvas></div>
    <p class="note">${r.end_note ? (/behind the connector/.test(r.end_note) ? `${esc(r.end_note)}. ` : `<b>Board share only</b> (${esc(r.end_note)}): the channel behind the connector is missing, so this eye is information, not judged; set one in the interface's card of the overview. `) : ""}Single-bit response of the channel between transmitter (${(r.tx_used || p.tx).r > 0 ? fmt((r.tx_used || p.tx).r, 0) + " Ω" : "ideal source"}, edge ${fmt(p.tx.rise_ps, 0)} ps) and receiver (${(r.rx_used || p.rx).r > 0 ? fmt((r.rx_used || p.rx).r, 0) + " Ω" : "high impedance"}), PRBS7 superposed linearly. 100 % = transmit swing (at the receiver termination; open circuit for a high-impedance receiver). No equalisation, jitter or noise.${ch.diff ? " Differential: odd mode." : ""}</p>${warn}`;
  drawEye($("ch-eye"), e);
}
