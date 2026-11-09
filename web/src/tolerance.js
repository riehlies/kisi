"use strict";
// ============================================================== manufacturing tolerances
// One set of tolerances for every view: kisi's defaults (data/stackups.json, mostly estimates) <
// the board's kisi.json ("tolerances") < the edits in the Compare & tolerances view (per board in this
// browser, moved into kisi.json through Prepare PCB). With the option on, the analysis gives every
// cross-section and net with a target a band (kisi_si::study::rel_band, NetStat.z_tol_*); this file shows
// it the same way everywhere (zSpread); the calculation accuracy is only in the value's tooltip.
const TOL_KEYS = ["dielectric_thickness_pct", "er_pct", "copper_pct", "width"];
// the option "Fab stackups & tolerances" (off by default): the viewer's choice per board, else on when the
// board's kisi.json has a "tolerances" entry. Off, nothing of it shows but the switch in the Stackup card.
const fabKey = () => "kisi.fab.v1." + S.name;
function fabOn() { const v = lsGet(fabKey()); return v == null ? !!(S.config && S.config.tolerances) : !!v; }
function setFabOn(on) {
  lsSet(fabKey(), on === !!(S.config && S.config.tolerances) ? null : on);
  if (!on && S.mode === "tol") S.mode = "dev";
  if (!on && typeof stkOpen === "function" && !$("stk-panel").hidden) stkOpen(false);
  if (typeof exportHint === "function") exportHint();
  runAnalysis();
}
const tolEditKey = () => "kisi.tol.v1." + S.name;
function tolEdits() {
  if (!S.tolEd || S.tolEd.board !== S.name) S.tolEd = { board: S.name, v: lsGet(tolEditKey()) || {} };
  return S.tolEd.v;
}
function setTolEdits(v) { S.tolEd = { board: S.name, v }; lsSet(tolEditKey(), Object.keys(v).length ? v : null); }
function tolDefaults() {
  let d = null;
  try { d = stkPresets().tolerance_defaults; } catch (e) { /* engine without presets */ }
  return d;
}
// the tolerances in use: values and where each comes from ("default", "kisi.json", "viewer")
function tolEff() {
  const d = tolDefaults();
  if (!d) return null;
  const c = (S.config && S.config.tolerances) || {}, e = tolEdits();
  const t = { dielectric_thickness_pct: d.dielectric_thickness_pct, er_pct: d.er_pct, copper_pct: d.copper_pct, width_mm: d.width_mm, width_pct: null };
  const from = Object.fromEntries(TOL_KEYS.map((k) => [k, "default"]));
  for (const [src, label] of [[c, "kisi.json"], [e, "viewer"]]) {
    for (const k of ["dielectric_thickness_pct", "er_pct", "copper_pct"]) if (src[k] != null) { t[k] = src[k]; from[k] = label; }
    if (src.width_pct != null) { t.width_pct = src.width_pct; from.width = label; }
    else if (src.width_mm != null) { t.width_mm = src.width_mm; t.width_pct = null; from.width = label; }
  }
  return { ...t, from, default_window_pct: d.default_window_pct ?? 10, estimates: d.estimates || [] };
}
// where a value comes from, for the inputs and tooltips
function tolFromText(k) {
  const f = (tolEff() || { from: {} }).from[k];
  return f === "kisi.json" ? "from kisi.json" : f === "viewer" ? "edited here (in this browser; Prepare PCB moves it into kisi.json)" : "kisi default (estimate, data/stackups.json)";
}
function tolText() {
  const t = tolEff();
  if (!t) return "";
  return `dielectric thickness ±${fmt(t.dielectric_thickness_pct, 0)} %, εr ±${fmt(t.er_pct, 0)} %, copper ±${fmt(t.copper_pct, 0)} %, width ±${t.width_pct != null ? fmt(t.width_pct, 0) + " %" : fmtSig(t.width_mm, 2) + " mm"}`;
}
// a class's target window (± %): its interface profile's, else the default
function tolWindow(cls) {
  const id = stkAssign()[cls], p = id && typeof profile === "function" ? profile(id) : null;
  return p && p.z_tol_pct > 0 ? p.z_tol_pct : (tolEff() || { default_window_pct: 10 }).default_window_pct;
}
// the analysis parameter (kisi_si::study::TolSpec)
function tolSpec() {
  const t = fabOn() ? tolEff() : null, windows = {};
  if (!t) return null;
  for (const c of (S.scene && S.scene.netclasses) || []) windows[c.name] = tolWindow(c.name);
  return { tol: { dielectric_thickness_pct: t.dielectric_thickness_pct, er_pct: t.er_pct, copper_pct: t.copper_pct, width_mm: t.width_mm, width_pct: t.width_pct }, windows, default_window_pct: t.default_window_pct };
}
// for kisi_stackup_study and the exports' header: the values with their origin
function tolSet() { const t = tolEff(); return t && { dielectric_thickness_pct: t.dielectric_thickness_pct, er_pct: t.er_pct, copper_pct: t.copper_pct, width_mm: t.width_mm, width_pct: t.width_pct, from: t.from }; }
function tolStatus(z, target, win, rss) {
  const lo = target * (1 - win / 100), hi = target * (1 + win / 100);
  return z < lo || z > hi ? "nominal_outside" : z - rss < lo || z + rss > hi ? "band_leaves" : "inside";
}
const TOL_STATUS = { inside: "band inside the window", band_leaves: "nominal inside, the band leaves the window", nominal_outside: "nominal outside the window" };
// the band of one piece of copper: its cross-section's relative band at its compared impedance
function pieceBand(i) {
  const xs = S.an.cross_sections[S.pieces.xs[i]], net = S.pieces.net[i], t = classTarget(net);
  if (!t || xs.geom == null) return null;
  const g = S.an.geometries[xs.geom], z = comparedZ(xs, t);
  if (!g || !g.tol || !z) return null;
  const k = t.diff ? 2 : 0, cls = S.an.classes[S.scene.net_class[net] ?? 0]?.name ?? "", win = tolWindow(cls);
  const rss = g.tol[k] * z.v, wc = g.tol[k + 1] * z.v;
  return { z: z.v, rss, wc, win, status: tolStatus(z.v, t.z, win, rss) };
}
// the model's accuracy of an impedance, for the title of the value (not shown as text)
function zAccTitle(unc, fs) {
  if (unc == null) return "";
  const lv = fs >= 0.999 ? "the 2D field solver" : fs <= 0.001 ? "the formulas" : `the field solver for ${fmt(fs * 100, 0)} % of the length, formulas for the rest`;
  return `Model accuracy of this impedance: computed with ${lv}, typically within ±${fmt(unc, unc < 10 ? 1 : 0)} % of kisi's validation references (data/accuracy.json; docs/models.md, Accuracy levels) – not a bound.`;
}
// the fab band of an impedance (option on): "fab ±4.1 Ω RSS / ±6.9 Ω worst", coloured by its status
function zSpread(b) {
  if (!b || b.rss == null || !fabOn()) return "";
  const title = `Fab tolerances (${tolText()}): RSS and worst case of the band; ${TOL_STATUS[b.status] || ""} (±${fmt(b.win, 0)} %).`;
  return ` <span class="zb ${b.status}" title="${esc(title)}">fab ±${fmt(b.rss, 1)} Ω RSS / ±${fmt(b.wc, 1)} Ω worst</span>`;
}
// a net's band (NetStat) as zSpread wants it
function netBand(st) {
  if (!st || st.z_tol_rss_pct == null || st.z_mean == null) return null;
  return { rss: st.z_tol_rss_pct / 100 * st.z_mean, wc: st.z_tol_wc_pct / 100 * st.z_mean, win: st.z_window_pct, status: st.z_tol_status };
}
// the share of the length with a target per status (Signals card)
function tolShares() {
  const m = { inside: 0, band_leaves: 0, nominal_outside: 0 };
  let l = 0;
  for (const n of S.an.nets) if (n.z_tol_status) { m[n.z_tol_status] += n.length; l += n.length; }
  return l ? { inside: m.inside / l, band_leaves: m.band_leaves / l, nominal_outside: m.nominal_outside / l } : null;
}
function renderTolSummary() {
  const el = $("k-tol");
  if (!el) return;
  const sh = S.an && !S.an.skeleton && fabOn() ? tolShares() : null;
  el.hidden = !sh;
  if (!sh) return;
  const p = (v) => Math.round(v * 100) + " %";
  const allDef = Object.values(tolEff().from).every((f) => f === "default");
  el.innerHTML = `<b>Fab tolerances</b> (RSS band, <span title="${esc(tolText())}">${allDef ? "kisi's defaults, estimates" : "this board's values"}</span>): <span class="zb inside">${p(sh.inside)}</span> of the length with a target stays in its window, <span class="zb band_leaves">${p(sh.band_leaves)}</span> nominal inside but the band leaves, <span class="zb nominal_outside">${p(sh.nominal_outside)}</span> nominal outside. <button class="linklike" id="k-tol-edit">tolerances</button>`;
  $("k-tol-edit").onclick = () => { stkOpen(true); const el2 = $("stk-tol-thk"); if (el2) el2.scrollIntoView({ block: "center" }); };
}
