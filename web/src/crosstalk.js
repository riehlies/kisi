"use strict";
// ============================================================== couplings (step 4)
// Stage 1: formula estimate of NEXT/FEXT for every net with a target (kisi_core::rf::crosstalk_scan),
// map mode "Crosstalk" and the critical list. Stage 2: the field solver for the coupling
// cross-sections of the critical nets only.
const XT_CRIT = 0.01, XT_HIGH = 0.03;

function xtKey() { return `${S.anGen}|${JSON.stringify(rfParams())}`; }
// (re)compute the scan when the analysis changed; while the field solver runs at most every 8 s
function xtUpdate(force) {
  if (!S.xtOn || !S.full) return;
  const key = xtKey();
  if (!force && S.xtScan && S.xtScan.key === key) return;
  const busy = S.pool && S.pool.running;
  if (!force && busy && S.xtScan && performance.now() - S.xtScan.t < 8000) { S.xtScan.stale = true; return; }
  const nets = S.an.nets.filter((n) => n.target).map((n) => n.net);
  let r;
  try { r = S.engine.xtScan({ nets, rf: rfParams(), threshold: XT_CRIT }); } catch (e) { r = { rows: [], critical_geoms: [], threshold: XT_CRIT, error: e.message }; }
  const map = new Map();
  for (const row of r.rows) { map.set(row.net, row); if (row.partner != null) map.set(row.partner, row); }
  S.xtScan = { ...r, key, t: performance.now(), map, stale: false };
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

function xtStart() {
  showBusy("Calculating couplings…");
  requestAnimationFrame(() => setTimeout(() => {
    const t0 = performance.now();
    S.xtOn = true;
    try { xtUpdate(true); } catch (e) { busyError(e.message); return; }
    S.xtMs = Math.round(performance.now() - t0);
    hideBusy();
    setMode("xt");
    renderFieldStatus();
  }, 0));
}

function renderXtStep() {
  const li = $("st4");
  if (!li) return;
  li.classList.toggle("locked", !S.full);
  const sc = S.xtOn && S.full ? S.xtScan : null;
  $("xt-start").hidden = !!sc;
  $("xt-start").disabled = !S.full;
  $("xt-res").hidden = !sc;
  li.classList.toggle("done", !!sc && sc.critical_geoms.length === 0);
  if (!sc) {
    $("st4-text").textContent = S.full
      ? "Crosstalk from neighbouring tracks for every net with a target: formula estimate in a few seconds, then the field solver for the critical couplings only."
      : "Available after step 2. Crosstalk from neighbouring tracks for every net with a target: formula estimate first, then the field solver for the critical couplings only.";
    return;
  }
  const crit = sc.rows.filter((r) => r.critical), field = sc.rows.filter((r) => r.source !== "formula").length;
  const bs = sc.rows.reduce((a, r) => a + (r.broadside > 0 ? 1 : 0), 0);
  $("st4-text").textContent = `${sc.rows.length} victims${S.xtMs != null ? ` in ${S.xtMs} ms` : ""}: ${crit.length} critical (NEXT or FEXT from 1 % of the aggressor swing, at ${fmt(rfParams().rise * 1e12, 0)} ps rise)` +
    (field ? `, ${field} with field-solver couplings.` : ", formula estimate.") + (sc.stale ? " Updating when the field solver pauses." : "");
  const open = sc.critical_geoms.length, pool = S.pool, busy = pool && (pool.running || S.fieldPending.size > 0);
  const rb = $("xt-refine");
  rb.textContent = open ? `Refine ${crit.length} critical coupling${crit.length === 1 ? "" : "s"}` : crit.length ? "Critical couplings refined" : "Nothing critical";
  rb.title = open ? `${open} coupling cross-sections for the field solver (those carrying at least 0.1 %), most important first` : "";
  rb.disabled = !open || busy || !!(pool && pool.failed);
  $("xt-sum").textContent = open ? `${open} cross-sections` : "";
  const rows = crit.slice(0, 12).map((r) => {
    const nm = r.partner != null ? pairName(r.name, S.scene.nets[r.partner]) : netShort(r.name);
    const w = r.worst ? (r.worst_pair ? pairName(r.worst, r.worst_pair) : netShort(r.worst)) : "–";
    const tip = `${r.name}${r.partner != null ? " / " + S.scene.nets[r.partner] : ""} · worst aggressor ${r.worst || "–"}${r.worst_pair ? " / " + r.worst_pair : ""} · ${r.source === "formula" ? "formula estimate" : r.source}`;
    return `<tr class="click" data-net="${r.net}" title="${esc(tip)}"><td><span class="sw" style="background:${xtColor(xtWorst(r))}"></span>${esc(nm)}</td><td class="num">${fmt(r.next * 100, 1)}</td><td class="num">${fmt(r.fext * 100, 1)}</td><td>${esc(w)}</td></tr>`;
  }).join("");
  $("xt-rows").innerHTML = rows || `<tr><td colspan="4" class="muted">No net above 1 %.</td></tr>`;
  $("xt-rows").querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => { S.detailTab = "xt"; selectNet(+tr.dataset.net); });
  $("xt-note").textContent = (crit.length > 12 ? `${crit.length - 12} more critical nets: Crosstalk colours on the board. ` : "") +
    "NEXT reaches the near end, FEXT the far end; pairs: differential noise, common mode in the net's Crosstalk tab. Neighbours on the same layer" +
    (bs ? `; ${bs} net${bs === 1 ? "" : "s"} also run over or under a track on the adjacent layer (broadside coupling, listed in the net's tab, not computed yet).` : "; broadside coupling (adjacent layers) is detected but not computed yet.");
}

$("xt-start").onclick = () => xtStart();
$("xt-refine").onclick = () => {
  const sc = S.xtScan;
  if (!sc || !sc.critical_geoms.length) return;
  if (S.pool && !S.pool.running && !S.pool.mainThread && S.pool.n !== threadCount()) { S.pool.terminate(); S.pool = null; }
  if (!S.pool) S.pool = new FieldPool();
  const wmax = Math.max(...sc.critical_geoms.map((g) => g[2]), 1e-9);
  S.pool.start(sc.critical_geoms.map(([id, text, w]) => ({ id, text, prio: w / wmax })));
};
$("mode-xt").onclick = () => setMode("xt");

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
