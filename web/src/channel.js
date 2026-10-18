"use strict";
// ---- channel view ("Kanal"): interface profiles, transmitter/receiver, eye and checks
//
// Data:   DATA.interfaces (data/interfaces.json) = standard profiles with sources;
//         user overrides per profile in localStorage (PROF_KEY), class → profile
//         assignment per board (ifcKey()).
// Engine: S.engine.rf({..., bitrate, tx, rx, eye_rise}) for one net,
//         kisi_check for many nets (metrics only; status is judged here).

const PROF_KEY = "kisi.profiles.v1";
const ifcKey = () => "kisi.ifc.v1." + S.name;
const lsGet = (k) => { try { const t = localStorage.getItem(k); return t ? JSON.parse(t) : null; } catch (e) { return null; } };
const lsSet = (k, v) => { try { if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, JSON.stringify(v)); } catch (e) { /* storage unavailable */ } };
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
  const nm = (S.scene.nets[net] || "").split("/").pop(), cls = className(net);
  const test = (s) => IFC.profiles.find((p) => (p.match || []).some((re) => new RegExp(re, "i").test(s)));
  const p = test(nm) || test(cls);
  if (p) return p.id;
  const t = classTarget(net);
  return t && t.diff ? "diff100" : "se50";
}
function profileIdOf(net) { return classAssign()[className(net)] || autoProfile(net); }
function profileOf(net) { return profile(profileIdOf(net)) || profile("se50"); }

// ---- metrics and judgement (shared by the table and the net detail)
function judge(m, p) {
  const L = p.limits, nyq = p.rate_gbps / 2, ui = 1000 / p.rate_gbps;
  const skewLim = L.skew_ps != null ? L.skew_ps : 0.05 * ui;
  const rows = [];
  const add = (key, name, value, limit, st, note) => rows.push({ key, name, value, limit, st, note });
  if (m.error) add("err", "Berechnung", m.error, "", "high2");
  if (m.z_mean != null) {
    const zl = p.z * (1 - p.z_tol_pct / 100), zh = p.z * (1 + p.z_tol_pct / 100);
    add("z", "Impedanz Ø (min … max)", `${fmt(m.z_mean, 1)} Ω (${fmt(m.z_min, 0)} … ${fmt(m.z_max, 0)})`, `${fmt(zl, 1)} … ${fmt(zh, 1)} Ω`, m.z_mean < zl || m.z_mean > zh ? "high1" : "ok",
      "Mittelwert entlang der Leitung aus der Impedanzanalyse; min/max enthalten Pads und kurze Engstellen");
  }
  if (m.eye_h != null) {
    const st = m.eye_h < L.eye_h_red ? "high2" : m.eye_h < L.eye_h_yellow ? "high1" : "ok";
    add("eye_h", "Augenhöhe", `${fmt(m.eye_h * 100, 0)} % (${fmt(m.eye_h * p.tx.swing_mv, 0)} mV)`, `rot < ${fmt(L.eye_h_red * 100, 0)} %, gelb < ${fmt(L.eye_h_yellow * 100, 0)} %`, st);
  }
  if (m.eye_w_ui != null) add("eye_w", "Augenbreite", `${fmt(m.eye_w_ui * 100, 0)} % UI`, `≥ ${fmt(L.eye_w_min_ui * 100, 0)} % UI`, m.eye_w_ui < L.eye_w_min_ui ? "high1" : "ok");
  if (m.il_nyq_db != null) add("il", `Dämpfung bei ${fmt(nyq, nyq < 1 ? 2 : 1)} GHz`, `${fmt(m.il_nyq_db, 2)} dB`, L.il_board_db != null ? `≥ ${fmt(L.il_board_db, 1)} dB` : "keine Grenze", L.il_board_db != null && m.il_nyq_db < L.il_board_db ? "high1" : "ok");
  if (m.rl_db != null) add("rl", "Rückflussdämpfung", `${fmt(m.rl_db, 1)} dB`, `≥ ${fmt(L.rl_min_db, 0)} dB bis ${fmt(1.5 * nyq, 1)} GHz`, m.rl_db < L.rl_min_db ? "high1" : "ok");
  if (m.skew_ps != null) add("skew", "Skew im Paar", `${fmt(Math.abs(m.skew_ps), 1)} ps${m.skew_uncertain ? " (unsicher)" : ""}`, `≤ ${fmt(skewLim, 1)} ps`, !m.skew_uncertain && Math.abs(m.skew_ps) > skewLim ? "high1" : "ok",
    m.skew_uncertain ? "Pfad verzweigt oder über Bauteile geteilt" : "");
  if (m.stub_ghz != null) add("stub", "Via-Stub-Resonanz", `${fmt(m.stub_ghz, 1)} GHz`, `> ${fmt(L.stub_factor * nyq, 1)} GHz`, m.stub_ghz < L.stub_factor * nyq ? "high1" : "ok");
  const rank = { high2: 0, high1: 1, ok: 2 };
  const worst = rows.reduce((w, r) => (rank[r.st] < rank[w] ? r.st : w), "ok");
  return { rows, status: worst };
}
const ST_NAME = { ok: "grün", high1: "gelb", high2: "rot" };

// ---- side cards
function renderIfcCard() {
  const box = $("ifc-body");
  if (!box || !S.an) return;
  const map = classAssign();
  const classes = S.an.classes.map((c, i) => ({ c, i })).filter(({ c }) => c.nets > 0 && c.target);
  const opts = (sel, auto) => `<option value="">automatisch (${esc(profile(auto)?.name || auto)})</option>` +
    IFC.profiles.map((p) => `<option value="${p.id}" ${p.id === sel ? "selected" : ""}>${esc(p.name)}</option>`).join("");
  box.innerHTML = classes.length ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Klasse</th><th>Profil</th></tr></thead><tbody>${classes.map(({ c }) => {
    const anyNet = S.an.nets.find((n) => n.class === c.name);
    const auto = anyNet ? autoProfile(anyNet.net) : "se50";
    return `<tr><td title="${c.nets} Netze, Ziel ${fmt(c.target.z, 0)} Ω ${c.target.diff ? "diff" : "SE"}">${esc(c.name)}</td><td><select data-cls="${esc(c.name)}" aria-label="Profil für ${esc(c.name)}">${opts(map[c.name] || "", auto)}</select></td></tr>`;
  }).join("")}</tbody></table></div>
    <p class="note">Automatisch: zuerst nach dem Netznamen, dann nach dem Klassennamen (Regeln in data/interfaces.json). Gilt je Board.</p>` : `<p class="note">Keine Netzklassen mit Zielimpedanz.</p>`;
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
  const src = (p.sources || []).concat(IFC.defaults.sources || []).map((s) => `<li><b>${{ spec: "Norm", guideline: "Richtlinie", kisi: "kisi" }[s.kind] || s.kind}:</b> ${esc(s.text)}${s.url ? ` <a href="${esc(s.url)}" target="_blank" rel="noopener">Quelle</a>` : ""}</li>`).join("");
  box.innerHTML = `<label class="opt" for="prof-sel"><span>Profil</span><select id="prof-sel">${IFC.profiles.map((q) => `<option value="${q.id}" ${q.id === id ? "selected" : ""}>${esc(q.name)}${profOverrides()[q.id] ? " *" : ""}</option>`).join("")}</select></label>
    <div class="pf-grid">
      <h4>Signal</h4>
      ${num(["rate_gbps"], "Datenrate", "Gbit/s")}${num(["z"], `Ziel ${p.signal === "diff" ? "diff." : "einzeln"}`, "Ω")}${num(["z_tol_pct"], "Toleranz", "%")}
      <h4>Sender <i class="muted">je Leitung</i></h4>
      ${num(["tx", "swing_mv"], "Hub", "mV")}${num(["tx", "r"], "Innenwiderstand", "Ω")}${num(["tx", "c_pf"], "Pin-Kapazität", "pF")}${num(["tx", "l_nh"], "Gehäuse-L", "nH")}${num(["tx", "rise_ps"], "Anstieg 10–90 %", "ps")}
      <h4>Empfänger <i class="muted">je Leitung</i></h4>
      ${num(["rx", "r"], "Abschluss", "Ω", "any", "0 = hochohmig")}${num(["rx", "c_pf"], "Pin-Kapazität", "pF")}${num(["rx", "l_nh"], "Gehäuse-L", "nH")}
      <h4>Grenzen</h4>
      ${num(["limits", "eye_h_red"], "Auge rot unter", "× Hub", "0.05")}${num(["limits", "eye_h_yellow"], "Auge gelb unter", "× Hub", "0.05")}${num(["limits", "eye_w_min_ui"], "Augenbreite min.", "UI", "0.05")}
      ${num(["limits", "skew_ps"], "Skew max.", "ps", "any", "leer = 5 % UI")}${num(["limits", "rl_min_db"], "Rückflussd. min.", "dB")}${num(["limits", "stub_factor"], "Stub-Resonanz über", "× Nyquist")}${num(["limits", "il_board_db"], "Dämpfung Board min.", "dB", "any", "leer = keine")}
    </div>
    ${p.il_total ? `<p class="note">Gesamtbudget laut Quelle: ${fmt(p.il_total.db, 0)} dB bei ${fmt(p.il_total.f_ghz, 0)} GHz – ${esc(p.il_total.scope)}.</p>` : ""}
    <div class="fs-row"><span class="note">${p.edited ? "Geändert gegenüber dem Standard (gespeichert in diesem Browser)." : "Standardwerte."}</span><button class="btn small" id="prof-reset" ${p.edited ? "" : "disabled"}>Standard</button></div>
    <details><summary>Quellen</summary><ul class="src">${src}</ul></details>`;
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
  if (!ch) return { error: r.error || "kein Kanal", ...zStats(net) };
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
  d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}${partner ? " · Paar mit " + esc(partner) : ""}</div></div><button class="x" id="closeCh" aria-label="Auswahl aufheben">×</button></header>
    <div class="dstats"><span>Profil <b>${esc(p.name)}</b>${p.edited ? " *" : ""}</span><span><button class="btn small" id="ch-edit">Profil bearbeiten</button></span></div>
    <div class="dstats muted"><span>Sender ${fmt(p.tx.r, 0)} Ω${p.tx.c_pf ? ", " + fmt(p.tx.c_pf, 1) + " pF" : ""}, ${fmt(p.tx.rise_ps, 0)} ps, ${fmt(p.tx.swing_mv, 0)} mV</span><span>Empfänger ${p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "hochohmig"}${p.rx.c_pf ? ", " + fmt(p.rx.c_pf, 1) + " pF" : ""}</span><span>je Leitung</span></div>
    <div class="tabs" role="tablist" aria-label="Kanal">${[["check", "Prüfung"], ["eye", "Auge"]].map(([k, n]) => `<button role="tab" data-ct="${k}" aria-selected="${tab === k}">${n}</button>`).join("")}</div>
    <div id="ch-body"></div>`;
  d.hidden = false;
  $("closeCh").onclick = () => selectNet(-1);
  $("ch-edit").onclick = () => { S.profEdit = p.id; renderProfCard(); $("h-prof").scrollIntoView({ behavior: "smooth", block: "start" }); };
  d.querySelectorAll("[data-ct]").forEach((b) => b.onclick = () => { S.chTab = b.dataset.ct; renderChDetail(net); });
  const body = $("ch-body");
  if (tab === "eye") return renderEye(body, r);
  const m = metricsOf(r, net), j = judge(m, p);
  const xt = r.crosstalk && r.crosstalk.aggressors ? `<tr><td></td><td>Übersprechen (Summe)</td><td>NEXT ${fmt(r.crosstalk.next_total * 100, 2)} %, FEXT ${fmt(r.crosstalk.fext_total * 100, 2)} %</td><td>Information</td></tr>` : "";
  body.innerHTML = `<div class="dstats"><span>Ergebnis <b style="color:${S.colors[j.status]}">${ST_NAME[j.status]}</b></span><span>${fmt(p.rate_gbps, p.rate_gbps < 1 ? 2 : p.rate_gbps % 1 ? 2 : 0)} Gbit/s</span>${r.channel ? `<span>Pfad ${fmt(r.channel.length_mm, 1)} mm, ${r.channel.vias} Vias</span>` : ""}</div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th></th><th>Kriterium</th><th>Wert</th><th>Grenze</th></tr></thead><tbody>
    ${j.rows.map((x) => `<tr title="${esc(x.note || "")}"><td><span class="dot" style="background:${S.colors[x.st]}" aria-label="${ST_NAME[x.st]}"></span></td><td>${x.name}</td><td>${esc(String(x.value))}</td><td>${x.limit}</td></tr>`).join("")}${xt}
    </tbody></table></div>
    <p class="note">Impedanz aus der Impedanzanalyse (Formel oder Feldlöser) gegen das Profil. Dämpfung, Rückflussdämpfung und TDR sind mit dem Bezugswiderstand gerechnet, das Auge mit Sender und Empfänger des Profils. Ohne Entzerrung, Jitter und Rauschen.</p>
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
    $("ck-text").textContent = `Prüfe ${Math.min(i + 4, items.length)} von ${items.length} Kanälen …`;
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
    $("ck-text").textContent = "Alle Netze mit Zielimpedanz mit dem Profil ihrer Klasse: Auge mit Sender und Empfänger, Dämpfung, Rückflussdämpfung, Skew, Via-Stubs.";
    return;
  }
  const rows = S.check.rows.map((r) => ({ r, p: profileOf(r.net) })).map((x) => ({ ...x, st: judge(rowMetrics(x.r), x.p).status }));
  const rank = { high2: 0, high1: 1, ok: 2 };
  rows.sort((a, b) => rank[a.st] - rank[b.st] || (a.r.eye_height ?? 0) - (b.r.eye_height ?? 0));
  const n = { high2: 0, high1: 0, ok: 0 }; rows.forEach((x) => n[x.st]++);
  const stale = S.check.gen !== S.anGen || S.check.stale;
  $("ck-text").textContent = `${rows.length} Kanäle: ${n.ok} grün, ${n.high1} gelb, ${n.high2} rot${stale ? " · veraltet, neu prüfen" : ""}`;
  const shown = S.checkAll ? rows : rows.slice(0, 15);
  box.innerHTML = `<div class="tablewrap"><table class="xs-table ck-table"><thead><tr><th></th><th>Netz</th><th>Profil</th><th>IL dB</th><th>Auge %</th><th>Skew ps</th></tr></thead><tbody>${shown.map(({ r, p, st }) => {
    const nm = r.name.split("/").pop();
    const sk = r.skew_ps == null ? "–" : fmt(Math.abs(r.skew_ps), 1) + (r.skew_uncertain ? "?" : "");
    return `<tr class="click" data-net="${r.net}" title="${esc(r.name)} · ${r.diff ? "Paar" : "einzeln"}, ${fmt(r.length_mm, 1)} mm, ${r.vias} Vias, RL ${fmt(r.rl_db, 1)} dB${r.stub_ghz != null ? ", Stub-Resonanz " + fmt(r.stub_ghz, 1) + " GHz" : ""}${r.error ? " · " + esc(r.error) : ""}"><td><span class="dot" style="background:${S.colors[st]}" aria-label="${ST_NAME[st]}"></span></td><td>${esc(nm)}</td><td>${esc(p.name.split(" (")[0])}</td><td>${fmt(r.il_nyq_db, 1)}</td><td>${r.error ? "–" : fmt(r.eye_height * 100, 0)}</td><td>${sk}</td></tr>`;
  }).join("")}</tbody></table></div>
  ${rows.length > 15 ? `<button class="btn small" id="ck-all">${S.checkAll ? "Nur die ersten 15" : `Alle ${rows.length} zeigen`}</button>` : ""}
  <p class="note">Grenzen je Profil (Karte „Profil“). „?“ = Pfad verzweigt, Skew unsicher.</p>`;
  box.querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => { S.chTab = "check"; selectNet(+tr.dataset.net); });
  if ($("ck-all")) $("ck-all").onclick = () => { S.checkAll = !S.checkAll; renderCheck(); };
}

function renderEye(body, r) {
  const ch = r.channel, e = r.eye, p = r.profile;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "kein Kanal")}</p>`; return; }
  if (!e || e.error) { body.innerHTML = `<p class="err">${esc(e?.error || "kein Augendiagramm")}</p>`; return; }
  const g = p.rate_gbps, uiPct = (x) => fmt(100 * x / e.ui_ps, 0);
  const warn = (e.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  body.innerHTML = `<div class="dstats"><span>${fmt(g, g < 1 ? 2 : g % 1 ? 2 : 0)} Gbit/s NRZ, PRBS7</span><span>UI ${fmt(e.ui_ps, 1)} ps</span><span>Gleichanteil ${fmt(e.dc * 100, 0)} % des Hubs</span></div>
    <div class="dstats"><span>Augenhöhe <b>${fmt(e.height * 100, 0)} %</b> = ${fmt(e.height * p.tx.swing_mv, 0)} mV</span><span>Augenbreite <b>${fmt(e.width_ps, 1)} ps</b> (${uiPct(e.width_ps)} % UI)</span><span>ISI-Summe <b>${fmt(e.isi_sum * 100, 0)} %</b></span></div>
    <div class="chart"><canvas id="ch-eye" style="height:220px" aria-label="Augendiagramm"></canvas></div>
    <p class="note">Einzelbit-Antwort des Kanals zwischen Sender (${fmt(p.tx.r, 0)} Ω, Flanke ${fmt(p.tx.rise_ps, 0)} ps) und Empfänger (${p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "hochohmig"}), PRBS7 linear überlagert. 100 % = Sendehub an angepasster Last. Ohne Entzerrung, Jitter und Rauschen.${ch.diff ? " Differentiell: ungerade Mode." : ""}</p>${warn}`;
  drawEye($("ch-eye"), e);
}
