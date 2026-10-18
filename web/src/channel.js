"use strict";
// ---- channel view: interface profiles, transmitter/receiver, eye and checks
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
function profileIdOf(net) { return classAssign()[className(net)] || autoProfile(net); }
function profileOf(net) { return profile(profileIdOf(net)) || profile("se50"); }

// ---- metrics and judgement (the verdict comes from kisi-si::profile via WASM,
// identical for viewer, CLI and MCP; here only the presentation)
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
  const opts = (sel, auto) => `<option value="">automatic (${esc(profile(auto)?.name || auto)})</option>` +
    IFC.profiles.map((p) => `<option value="${p.id}" ${p.id === sel ? "selected" : ""}>${esc(p.name)}</option>`).join("");
  box.innerHTML = classes.length ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Class</th><th>Profile</th></tr></thead><tbody>${classes.map(({ c }) => {
    const anyNet = S.an.nets.find((n) => n.class === c.name);
    const auto = anyNet ? autoProfile(anyNet.net) : "se50";
    return `<tr><td title="${c.nets} nets, target ${fmt(c.target.z, 0)} Ω ${c.target.diff ? "diff" : "SE"}">${esc(c.name)}</td><td><select data-cls="${esc(c.name)}" aria-label="Profile for ${esc(c.name)}">${opts(map[c.name] || "", auto)}</select></td></tr>`;
  }).join("")}</tbody></table></div>
    <p class="note">Automatic: by net name first, then by class name (rules in data/interfaces.json). Stored per board.</p>` : `<p class="note">No net classes with target impedance.</p>`;
  box.querySelectorAll("select[data-cls]").forEach((s) => s.onchange = () => {
    const m = classAssign();
    if (s.value) m[s.dataset.cls] = s.value; else delete m[s.dataset.cls];
    lsSet(ifcKey(), m); S.chCache = null; markCheckStale(); renderProfCard(); if (S.selNet >= 0 && S.tab === "ch") selectNet(S.selNet, false);
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
function markCheckStale() { if (S.check) S.check.stale = true; renderCheck(); }

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
function metricsOf(r, net) {
  const ch = r.channel, p = r.profile;
  if (!ch) return { error: r.error || "no channel", ...zStats(net) };
  const nyq = p.rate_gbps * 0.5e9, df = ch.freq[0];
  const k = Math.min(ch.freq.length, Math.max(1, Math.round(nyq / df))) - 1;
  const k2 = Math.min(ch.freq.length, Math.max(1, Math.round(1.5 * nyq / df)));
  const stubs = ch.via_info.filter((v) => v.stub_mm > 0.01 && v.stub_f_ghz != null).map((v) => v.stub_f_ghz);
  const e = r.eye && !r.eye.error ? r.eye : null;
  return {
    il_nyq_db: ch.s21_db[k], rl_db: -Math.max(...ch.s11_db.slice(0, k2)),
    eye_h: e ? e.height : null, eye_w_ui: e ? e.width_ps / e.ui_ps : null,
    skew_ps: r.skew ? r.skew.skew_ps : null, skew_uncertain: r.skew ? r.skew.uncertain : false,
    stub_ghz: stubs.length ? Math.min(...stubs) : null, error: r.eye && r.eye.error ? r.eye.error : null, ...zStats(net),
  };
}
function renderChDetail(net) {
  const d = $("ch-detail"), st = S.an.nets.find((x) => x.net === net), r = chFor(net), p = r.profile;
  const partner = st && st.partner != null ? S.scene.nets[st.partner] : null;
  const tab = S.chTab || "check";
  d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}${partner ? " · pair with " + esc(partner) : ""}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header>
    <div class="dstats"><span>Profile <b>${esc(p.name)}</b>${p.edited ? " *" : ""}</span><span><button class="btn small" id="ch-edit">Edit profile</button></span></div>
    <div class="dstats muted"><span>Transmitter ${fmt(p.tx.r, 0)} Ω${p.tx.c_pf ? ", " + fmt(p.tx.c_pf, 1) + " pF" : ""}, ${fmt(p.tx.rise_ps, 0)} ps, ${fmt(p.tx.swing_mv, 0)} mV</span><span>Receiver ${p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "high impedance"}${p.rx.c_pf ? ", " + fmt(p.rx.c_pf, 1) + " pF" : ""}</span><span>per line</span></div>
    <div class="tabs" role="tablist" aria-label="Channel">${[["check", "Check"], ["eye", "Eye"]].map(([k, n]) => `<button role="tab" data-ct="${k}" aria-selected="${tab === k}">${n}</button>`).join("")}</div>
    <div id="ch-body"></div>`;
  d.hidden = false;
  $("closeCh").onclick = () => selectNet(-1);
  $("ch-edit").onclick = () => { S.profEdit = p.id; renderProfCard(); $("h-prof").scrollIntoView({ behavior: "smooth", block: "start" }); };
  d.querySelectorAll("[data-ct]").forEach((b) => b.onclick = () => { S.chTab = b.dataset.ct; renderChDetail(net); });
  const body = $("ch-body");
  if (tab === "eye") return renderEye(body, r);
  const m = metricsOf(r, net), j = judge(m, p);
  const xt = r.crosstalk && r.crosstalk.aggressors ? `<tr><td></td><td>Crosstalk (total)</td><td>NEXT ${fmt(r.crosstalk.next_total * 100, 2)} %, FEXT ${fmt(r.crosstalk.fext_total * 100, 2)} %</td><td>information only</td></tr>` : "";
  body.innerHTML = `<div class="dstats"><span>Result <b style="color:${S.colors[j.status]}">${ST_NAME[j.status]}</b></span><span>${fmt(p.rate_gbps, p.rate_gbps < 1 ? 2 : p.rate_gbps % 1 ? 2 : 0)} Gbit/s</span>${r.channel ? `<span>Path ${fmt(r.channel.length_mm, 1)} mm, ${r.channel.vias} vias</span>` : ""}</div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th></th><th>Criterion</th><th>Value</th><th>Limit</th></tr></thead><tbody>
    ${j.rows.map((x) => `<tr title="${esc(x.note || "")}"><td><span class="dot" style="background:${S.colors[x.st]}" aria-label="${ST_NAME[x.st]}"></span></td><td>${x.name}</td><td>${esc(String(x.value))}</td><td>${x.limit}</td></tr>`).join("")}${xt}
    </tbody></table></div>
    <p class="note">Impedance from the impedance analysis (formula or field solver) against the profile. Insertion loss, return loss and TDR use the reference resistance; the eye uses the profile's transmitter and receiver. No equalisation, jitter or noise.</p>
    ${(r.channel?.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
}

// ---- board-level check
function checkItems() {
  const out = [];
  for (const st of S.an.nets) {
    const t = classTarget(st.net);
    if (!t) continue;
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
const rowMetrics = (r) => ({ ...zStats(r.net), il_nyq_db: r.il_nyq_db, rl_db: r.rl_db, eye_h: r.error ? null : r.eye_height, eye_w_ui: r.error ? null : r.eye_width_ui, skew_ps: r.skew_ps, skew_uncertain: r.skew_uncertain, stub_ghz: r.stub_ghz, error: r.error });
async function runCheck() {
  const items = checkItems(), rows = [], gen = S.anGen, p = rfParams();
  const btn = $("ck-start"); btn.disabled = true;
  for (let i = 0; i < items.length; i += 4) {
    if (gen !== S.anGen) break;
    $("ck-text").textContent = `Checking ${Math.min(i + 4, items.length)} of ${items.length} channels …`;
    await new Promise((res) => setTimeout(res, 0));
    try {
      S.engine.call(S.engine.k.kisi_check, JSON.stringify({ rf: p, items: items.slice(i, i + 4) }));
      rows.push(...JSON.parse(S.engine.out()));
    } catch (e) { console.warn(e); }
  }
  btn.disabled = false;
  S.check = { gen, rows, stale: false };
  S.chStatus = new Map();
  for (const r of rows) {
    const st = judge(rowMetrics(r), profileOf(r.net)).status;
    S.chStatus.set(r.net, st);
    const n = S.an.nets.find((x) => x.net === r.net);
    if (n && n.partner != null) S.chStatus.set(n.partner, st);
  }
  if (S.tab === "ch") { buildPaths(); if (S.selNet >= 0) selectNet(S.selNet, false); renderLegend(); draw(); }
  renderCheck();
}
function renderCheck() {
  const box = $("ck-out");
  if (!box) return;
  if (!S.check || !S.check.rows.length) {
    box.innerHTML = "";
    $("ck-text").textContent = "All nets with target impedance against their class profile: eye with transmitter and receiver, insertion loss, return loss, skew, via stubs.";
    return;
  }
  const rows = S.check.rows.map((r) => ({ r, p: profileOf(r.net) })).map((x) => ({ ...x, st: judge(rowMetrics(x.r), x.p).status }));
  const rank = { high2: 0, high1: 1, ok: 2 };
  rows.sort((a, b) => rank[a.st] - rank[b.st] || (a.r.eye_height ?? 0) - (b.r.eye_height ?? 0));
  const n = { high2: 0, high1: 0, ok: 0 }; rows.forEach((x) => n[x.st]++);
  const stale = S.check.gen !== S.anGen || S.check.stale;
  $("ck-text").textContent = `${rows.length} channels: ${n.ok} green, ${n.high1} yellow, ${n.high2} red${stale ? " · outdated, check again" : ""}`;
  const shown = S.checkAll ? rows : rows.slice(0, 15);
  box.innerHTML = `<div class="tablewrap"><table class="xs-table ck-table"><thead><tr><th></th><th>Net</th><th>Profile</th><th>IL dB</th><th>Eye %</th><th>Skew ps</th></tr></thead><tbody>${shown.map(({ r, p, st }) => {
    const nm = r.name.split("/").pop();
    const sk = r.skew_ps == null ? "–" : fmt(Math.abs(r.skew_ps), 1) + (r.skew_uncertain ? "?" : "");
    return `<tr class="click" data-net="${r.net}" title="${esc(r.name)} · ${r.diff ? "pair" : "single-ended"}, ${fmt(r.length_mm, 1)} mm, ${r.vias} vias, RL ${fmt(r.rl_db, 1)} dB${r.stub_ghz != null ? ", stub resonance " + fmt(r.stub_ghz, 1) + " GHz" : ""}${r.error ? " · " + esc(r.error) : ""}"><td><span class="dot" style="background:${S.colors[st]}" aria-label="${ST_NAME[st]}"></span></td><td>${esc(nm)}</td><td>${esc(p.name.split(" (")[0])}</td><td>${fmt(r.il_nyq_db, 1)}</td><td>${r.error ? "–" : fmt(r.eye_height * 100, 0)}</td><td>${sk}</td></tr>`;
  }).join("")}</tbody></table></div>
  ${rows.length > 15 ? `<button class="btn small" id="ck-all">${S.checkAll ? "Only the first 15" : `Show all ${rows.length}`}</button>` : ""}
  <p class="note">Limits per profile (“Profile” card). “?” = branched path, skew uncertain.</p>`;
  box.querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => { S.chTab = "check"; selectNet(+tr.dataset.net); });
  if ($("ck-all")) $("ck-all").onclick = () => { S.checkAll = !S.checkAll; renderCheck(); };
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
    <p class="note">Single-bit response of the channel between transmitter (${fmt(p.tx.r, 0)} Ω, edge ${fmt(p.tx.rise_ps, 0)} ps) and receiver (${p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "high impedance"}), PRBS7 superposed linearly. 100 % = transmit swing (at the receiver termination; open circuit for a high-impedance receiver). No equalisation, jitter or noise.${ch.diff ? " Differential: odd mode." : ""}</p>${warn}`;
  drawEye($("ch-eye"), e);
}
