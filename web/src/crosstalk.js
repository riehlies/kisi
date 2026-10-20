"use strict";
// ============================================================== couplings (crosstalk)
// Stage 1, with step 2: formula estimate of NEXT/FEXT for every net with a target
// (kisi_core::rf::crosstalk_scan), map mode "Crosstalk" and the critical list in the Couplings
// card. Stage 2, with step 3 ("Refine critical couplings"): the field solver for the coupling
// cross-sections of the critical nets only.
const XT_CRIT = 0.01, XT_HIGH = 0.03;

function xtKey() { return `${S.anGen}|${JSON.stringify(rfParams())}`; }
// (re)compute the scan with every analysis after step 2; while the field solver runs at most every 8 s
function xtUpdate(force) {
  if (!S.full || !S.an || S.an.skeleton) { S.xtScan = null; return; }
  const key = xtKey();
  if (!force && S.xtScan && S.xtScan.key === key) return;
  const busy = S.pool && S.pool.running;
  if (!force && busy && S.xtScan && performance.now() - S.xtScan.t < 8000) { S.xtScan.stale = true; return; }
  const nets = S.an.nets.filter((n) => n.target).map((n) => n.net);
  const t0 = performance.now();
  let r;
  try { r = S.engine.xtScan({ nets, rf: rfParams(), threshold: XT_CRIT }); } catch (e) { r = { rows: [], critical_geoms: [], map: [], threshold: XT_CRIT, error: e.message }; }
  const byNet = new Map();
  for (const row of r.rows) { byNet.set(row.net, row); if (row.partner != null) byNet.set(row.partner, row); }
  S.xtMs = Math.round(performance.now() - t0);
  S.xtScan = { ...r, key, t: performance.now(), byNet, stale: false, paths: null };
}
const xtWorst = (row) => Math.max(row.next, row.fext);
// both nets of a pair in one short name: "USRIO_4_N" + "USRIO_4_P" → "USRIO_4_P/N"
function pairName(a, b) {
  const x = netShort(a), y = netShort(b);
  let k = 0;
  while (k < x.length && k < y.length && x[k] === y[k]) k++;
  const sa = x.slice(k), sb = y.slice(k);
  if (k === 0 || sa.length > 4 || sb.length > 4) return `${x} / ${y}`;
  const [p, n] = /^(p|\+|t)/i.test(sb) || /^(n|-|c)/i.test(sa) ? [sb, sa] : [sa, sb];
  return `${x.slice(0, k)}${p}/${n}`;
}
function xtColor(v) { const C = S.colors; return v == null ? C.none : v < XT_CRIT ? C.ok : v < XT_HIGH ? C.high1 : C.high2; }

// the Couplings card under the nets: summary and the critical list
function renderXtCard() {
  const sc = S.full ? S.xtScan : null;
  $("xt-wrap").hidden = !sc;
  if (!sc) {
    $("xt-count").textContent = "";
    $("xt-text").textContent = "Crosstalk from neighbouring tracks, estimated with step 2 for every net with a target.";
    $("xt-note").textContent = "";
    return;
  }
  const crit = sc.rows.filter((r) => r.critical), field = sc.rows.filter((r) => r.source !== "formula").length;
  const bs = sc.rows.reduce((a, r) => a + (r.broadside > 0 ? 1 : 0), 0);
  $("xt-count").textContent = `${crit.length} critical`;
  $("xt-text").textContent = `${sc.rows.length} nets with a target${S.xtMs != null ? ` in ${S.xtMs} ms` : ""}; critical: NEXT or FEXT from 1 % of the aggressor swing at ${fmt(rfParams().rise * 1e12, 0)} ps rise. ` +
    (field ? `${field} with field-solver couplings` : "Formula estimate") + (sc.critical_geoms.length ? `; ${sc.critical_geoms.length} coupling cross-sections left for the field solver (step 3, “Refine critical couplings”).` : ".") +
    (sc.stale ? " Updating when the field solver pauses." : "");
  const rows = crit.slice(0, 15).map((r) => {
    const nm = r.partner != null ? pairName(r.name, S.scene.nets[r.partner]) : netShort(r.name);
    const w = r.worst ? (r.worst_pair ? pairName(r.worst, r.worst_pair) : netShort(r.worst)) : "–";
    const tip = `${r.name}${r.partner != null ? " / " + S.scene.nets[r.partner] : ""} · worst aggressor ${r.worst || "–"}${r.worst_pair ? " / " + r.worst_pair : ""} · ${r.source === "formula" ? "formula estimate" : r.source}`;
    return `<tr class="click" data-net="${r.net}" aria-current="${r.net === S.selNet || r.partner === S.selNet}" title="${esc(tip)}"><td><span class="sw" style="background:${xtColor(xtWorst(r))}"></span>${esc(nm)}</td><td class="num">${fmt(r.next * 100, 1)}</td><td class="num">${fmt(r.fext * 100, 1)}</td><td>${esc(w)}</td></tr>`;
  }).join("");
  $("xt-rows").innerHTML = rows || `<tr><td colspan="4" class="muted">No net above 1 %.</td></tr>`;
  $("xt-rows").querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => { S.detailTab = "xt"; selectNet(+tr.dataset.net); });
  $("xt-note").textContent = (crit.length > 15 ? `${crit.length - 15} more: map mode “Crosstalk”. ` : "") +
    "NEXT reaches the near end, FEXT the far end; pairs: differential noise (common mode in the net's Crosstalk tab). Neighbours on the same layer" +
    (bs ? `; ${bs} net${bs === 1 ? "" : "s"} also run over or under a track on the adjacent layer (broadside, listed in the net's tab, not computed yet).` : "; broadside coupling (adjacent layers) is detected but not computed yet.");
}
$("mode-xt").onclick = () => setMode("xt");

// map mode "Crosstalk": only the stretches whose own crosstalk (within half the rise length
// around them) reaches 1 %, coloured 1–3 % / > 3 %, over the neutral board
function drawXtMap(px) {
  const sc = S.xtScan;
  if (S.tab !== "si" || S.mode !== "xt" || !sc || !sc.map.length) return;
  if (!sc.paths) {
    const n = S.scene.stackup.copper.length;
    sc.paths = Array.from({ length: n }, () => [[], []]);
    for (const h of sc.map) (sc.paths[h.layer] || sc.paths[0])[Math.max(h.next, h.fext) >= XT_HIGH ? 1 : 0].push(h);
  }
  const sel = S.selNet, partner = sel >= 0 ? (S.an.nets.find((x) => x.net === sel) || {}).partner : null;
  ctx.save();
  ctx.lineCap = "round";
  for (let l = sc.paths.length - 1; l >= 0; l--) {
    if (!S.layerOn[l]) continue;
    for (const k of [0, 1]) {
      const col = k ? S.colors.high2 : S.colors.high1;
      for (const pass of [0, 1]) {
        ctx.strokeStyle = pass ? col : S.colors.halo;
        for (const h of sc.paths[l][k]) {
          ctx.globalAlpha = sel >= 0 && h.net !== sel && h.net !== partner ? 0.35 : 1;
          ctx.lineWidth = Math.max(h.w, 2 * px) + (pass ? 0 : 2.5 * px);
          ctx.beginPath(); ctx.moveTo(h.a[0], h.a[1]); ctx.lineTo(h.b[0], h.b[1]); ctx.stroke();
        }
      }
    }
  }
  ctx.restore();
}

// coupled stretches of the selected net on the board while its Crosstalk tab is open: the
// victim's stretch and the aggressor's, coloured by the backward coupling; a hovered row
// shows that aggressor only
function drawXtMarks(px) {
  if (S.tab !== "si" || S.selNet < 0 || $("detail").hidden || S.detailTab !== "xt" || S.an.skeleton) return;
  const r = S.rfCache && S.rfCache.net === S.selNet ? S.rfCache.res : null;
  const segs = r && r.crosstalk && r.crosstalk.segments;
  if (!segs || !segs.length) return;
  const hov = S.xtHover ?? -1;
  ctx.save();
  ctx.lineCap = "butt";
  for (const pass of [0, 1]) {
    for (const s of segs) {
      if (!S.layerOn[s.layer] || (hov >= 0 && s.net !== hov)) continue;
      if (pass === 0) {
        ctx.strokeStyle = S.colors.halo; ctx.lineWidth = 7 * px;
        ctx.beginPath(); ctx.moveTo(s.aa[0], s.aa[1]); ctx.lineTo(s.ab[0], s.ab[1]); ctx.stroke();
      } else {
        ctx.strokeStyle = xtColor(Math.max(s.kb, 1e-6)); ctx.globalAlpha = 0.9; ctx.lineWidth = 4 * px;
        ctx.beginPath(); ctx.moveTo(s.aa[0], s.aa[1]); ctx.lineTo(s.ab[0], s.ab[1]); ctx.stroke();
        ctx.globalAlpha = 0.55; ctx.lineWidth = 2 * px;
        ctx.beginPath(); ctx.moveTo(s.va[0], s.va[1]); ctx.lineTo(s.vb[0], s.vb[1]); ctx.stroke();
        ctx.globalAlpha = 1;
      }
    }
  }
  ctx.restore();
}
