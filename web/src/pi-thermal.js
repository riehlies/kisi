"use strict";
// ---- temperature (step 3, "Temperature" view): kisi_pi::thermal through the engine – the board's
// steady-state temperature from the losses of step 2 (copper loss of every rail and the ground
// return, heat of the parts), per copper layer, the junction temperature of every part with heat
// and the IR drop of every rail at that temperature (one electro-thermal step). An estimate:
// board in air at the ambient temperature, package data from PI_THERMAL or package defaults.

const THERM_COOLING = {
  still: { vertical: false, airflow_m_s: 0 },
  vertical: { vertical: true, airflow_m_s: 0 },
  air1: { vertical: false, airflow_m_s: 1 },
  air3: { vertical: false, airflow_m_s: 3 },
};
function thermParams() {
  const c = THERM_COOLING[$("therm-cool").value] || THERM_COOLING.still;
  const a = parseFloat($("therm-amb").value);
  return { ambient_c: isFinite(a) ? Math.min(150, Math.max(-40, a)) : 25, vertical: c.vertical, airflow_m_s: c.airflow_m_s };
}
const thermOk = () => !!(S.therm && !S.therm.error && S.therm.t);
// a new DC result makes the temperature stale
function thermReset() { S.therm = null; S.thermImg = null; S.thermMark = null; if (S.piMode === "t") S.piMode = S.piSolved ? "v" : "b"; }

async function solveThermal() {
  if (!S.piSolved || S.thermRunning) return;
  const t0 = performance.now();
  const solved = (r) => { const x = S.piRes.get(r.net); return x && x.res; };
  const rails = [...S.piSetups.values()].filter(solved).concat([...(S.piGround || new Map()).values()].filter(solved));
  const n = rails.length + 1;
  stepProgress("therm", 0, n);
  await frame();
  let th;
  try { th = S.engine.piThermal(thermParams()); } catch (e) { S.therm = { error: e.message }; stepProgress("therm", null); renderPi(); return; }
  th.rails = [];
  for (let i = 0; i < rails.length; i++) {
    stepProgress("therm", i + 1, n);
    await frame();
    try { th.rails.push(S.engine.piThermalRail(rails[i], PI_PARAMS)); } catch (e) { th.rails.push({ net: rails[i].net, ground: !!rails[i].ground, error: e.message }); }
  }
  stepProgress("therm", null);
  th.ms = Math.round(performance.now() - t0);
  S.therm = th; S.thermImg = null; S.thermMark = null;
  // default layer: the one with the hottest cell
  S.thermLayer = th.t_max_layer;
  setPiMode("t");
}
$("therm-run").onclick = () => solveThermal();
// a result stays until "Recompute": say that changed settings are not applied yet
$("therm-amb").onchange = $("therm-cool").onchange = () => { if (thermOk()) $("therm-step").textContent = "Settings changed: “Recompute” applies them."; };

// side card: state of step 3
function renderThermStep() {
  const run = $("therm-run");
  run.textContent = S.therm && !S.therm.error ? "Recompute" : "Compute";
  run.disabled = !S.piSolved || !!S.thermRunning;
  run.title = S.piSolved ? "" : "after “Compute” in step 2";
  $("ps-t").classList.toggle("done", thermOk());
  const t = S.therm;
  if (t && t.error) { $("therm-step").innerHTML = `<span class="err">${esc(t.error)}</span>`; return; }
  if (thermOk()) {
    const h = thermHottest();
    $("therm-step").textContent = `Done in ${fmtSI(t.ms / 1e3, "s", 2)}. Board up to ${fmtT(t.t_max_c)} (${layerName(t.t_max_layer)})` +
      (h ? `, hottest part ${h.refdes} with Tj ≈ ${fmtT(h.tj_c)}` : "") + `. Estimate for ${fmtSI(t.parts_w + t.copper_w, "W")} in ${t.cooling.airflow_m_s ? `air flow of ${t.cooling.airflow_m_s} m/s` : "still air"} at ${fmtT(t.ambient_c)}.`;
    return;
  }
  $("therm-step").textContent = S.piSolved
    ? "Steady-state temperature of the board from the losses of step 2: copper of every layer, laminate and vias conduct, both sides lose heat to the air; junction temperature per part (θ values from PI_THERMAL or package estimates), heat through the vias and the IR drop at temperature."
    : "Needs the losses of step 2.";
}
const layerName = (l) => (S.scene.stackup.copper[l] || {}).name || `layer ${l}`;
const thermHottest = () => S.therm.parts.reduce((a, p) => (!a || p.tj_c > a.tj_c ? p : a), null);
// temperatures in °C (no prefixes), differences in K
const fmtT = (v) => (v == null || !isFinite(v) ? "–" : `${fmt(v, Math.abs(v) < 100 ? 1 : 0)} °C`);
// a temperature in a table column whose header names the unit
const fmtTn = (v) => (v == null || !isFinite(v) ? "–" : fmt(v, Math.abs(v) < 100 ? 1 : 0));
const fmtK = (v) => (v == null || !isFinite(v) ? "–" : `${fmt(v, Math.abs(v) < 10 ? 1 : 0)} K`);

// colour scale of the map: ambient … hottest cell of all layers (linear), shared by the tables
function thermScale() { const t = S.therm; return { lo: t.ambient_c, hi: Math.max(t.t_max_c, t.ambient_c + 0.1) }; }
const thermFrac = (v) => { const s = thermScale(); return 0.05 + 0.9 * Math.min(1, Math.max(0, (v - s.lo) / (s.hi - s.lo))); };
// a temperature with a chip in the map's colour (also above the board's maximum: junctions)
const tCell = (v, extra = "") => (v == null || !isFinite(v) ? "–" : `<span class="tchip" style="background:${turbo(thermFrac(v))}"></span>${fmtTn(v)}${extra}`);

function thermImage() {
  const t = S.therm, l = S.thermLayer;
  if (S.thermImg && S.thermImg.layer === l) return S.thermImg;
  const per = t.nx * t.ny, { lo, hi } = thermScale();
  const c = document.createElement("canvas"); c.width = t.nx; c.height = t.ny;
  const cx = c.getContext("2d"), img = cx.createImageData(t.nx, t.ny);
  for (let k = 0; k < per; k++) {
    const v = t.t[l * per + k], o = k * 4;
    if (!isFinite(v)) { img.data[o + 3] = 0; continue; }
    const col = turboRGB(0.05 + 0.9 * Math.min(1, Math.max(0, (v - lo) / (hi - lo))));
    img.data[o] = col[0]; img.data[o + 1] = col[1]; img.data[o + 2] = col[2]; img.data[o + 3] = 235;
  }
  cx.putImageData(img, 0, 0);
  S.thermImg = { layer: l, canvas: c, lo, hi };
  return S.thermImg;
}
// parts shown on a layer: the top side on the top layer, the bottom side on the bottom layer
function thermPartsOn(l) {
  const n = S.scene.stackup.copper.length;
  return S.therm.parts.filter((p) => p.at && (l === 0 ? p.side === "top" : l === n - 1 ? p.side === "bottom" : true));
}
// vias drawn: the ones with at least 2 % of the largest heat flow (the list holds the 400 largest)
function thermVias() {
  const v = S.therm.vias || [], qmax = v.length ? Math.abs(v[0].q_w) : 0;
  return { list: qmax > 0 ? v.filter((x) => Math.abs(x.q_w) >= 0.02 * qmax) : [], qmax };
}
// via marker: size and grey level by the heat through it (log scale over two decades); grey reads on
// every colour of the temperature map, and heat flow (not ΔT) is what a via array is for
function viaStyle(q, qmax) {
  const f = Math.min(1, Math.max(0, 1 + Math.log10(Math.max(Math.abs(q), 1e-12) / qmax) / 2));
  const g = Math.round(235 - 215 * f);
  return { r: 2 + 4 * f, fill: `rgb(${g},${g},${g})` };
}

function drawThermal(px) {
  if (!thermOk()) return;
  const t = S.therm, im = thermImage();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(im.canvas, t.x0, t.y0, t.nx * t.cell, t.ny * t.cell);
  ctx.imageSmoothingEnabled = true;
  // vias with the most heat flow (screen-size markers, the largest on top)
  const { list, qmax } = thermVias();
  ctx.lineWidth = 1 * px; ctx.strokeStyle = S.colors.halo;
  for (let i = list.length - 1; i >= 0; i--) {
    const v = list[i], st = viaStyle(v.q_w, qmax);
    ctx.beginPath(); ctx.arc(v.at[0], v.at[1], st.r * px, 0, Math.PI * 2); ctx.fillStyle = st.fill; ctx.fill(); ctx.stroke();
  }
  // a rail or part picked in the panel
  const m = S.thermMark;
  if (m) {
    const acc = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
    ctx.lineWidth = 2.5 * px; ctx.strokeStyle = S.colors.halo;
    ctx.beginPath(); ctx.arc(m.at[0], m.at[1], 12 * px, 0, Math.PI * 2); ctx.stroke();
    ctx.lineWidth = 1.5 * px; ctx.strokeStyle = acc; ctx.stroke();
  }
  // the hottest parts of this side with their junction temperature (constant screen size, no overlaps)
  const cs = getComputedStyle(document.documentElement);
  const col = { over: cs.getPropertyValue("--z-high2").trim(), near: cs.getPropertyValue("--z-high1").trim() };
  const items = thermPartsOn(S.thermLayer).slice(0, 25).map((p) => ({ x: p.at[0], y: p.at[1], text: `${p.refdes} ${fmtT(p.tj_c)}`, color: col[p.status] }));
  if (m) items.unshift({ x: m.at[0], y: m.at[1], text: m.label });
  S.thermLabels = drawBoardLabels(items, { max: 12 });
}

function renderThermChips() {
  const n = S.scene.stackup.copper.length;
  if (!(S.thermLayer >= 0 && S.thermLayer < n)) S.thermLayer = 0;
  $("layers").innerHTML = S.scene.stackup.copper.map((c, l) => `<button class="chip" data-tl="${l}" aria-pressed="${l === S.thermLayer}"><i></i>${esc(c.name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = () => { S.thermLayer = +b.dataset.tl; renderThermChips(); renderThermLegend(); draw(); });
}

function renderThermLegend() {
  const L = $("legend");
  if (!thermOk()) { L.innerHTML = `<strong>Temperature</strong><div class="row">${S.therm && S.therm.error ? esc(S.therm.error) : "after “Compute” in step 3"}</div>`; return; }
  const t = S.therm, im = thermImage(), lay = t.layers[S.thermLayer], { list, qmax } = thermVias();
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  L.innerHTML = `<strong>Temperature · ${esc(layerName(S.thermLayer))} · estimate</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>${fmtT(im.lo)}</span><span>${fmtT((im.lo + im.hi) / 2)}</span><span>${fmtT(im.hi)}</span></div>
    <div class="row">This layer: up to ${fmtT(lay.t_max_c)}, mean ${fmtT(lay.t_mean_c)}. Labels: junction temperature of the hottest parts on this side.</div>
    ${list.length ? `<div class="row"><span class="viadot" style="background:${viaStyle(qmax, qmax).fill}"></span>vias with the most heat flow, up to ${fmtSI(qmax, "W")} (larger and darker = more); hover for the values</div>` : ""}`;
}

// a via under the mouse (screen distance), else null
function thermViaAt(mx, my) {
  const { list, qmax } = thermVias();
  let best = null, bd = 1e9;
  for (const v of list) {
    const sx = S.view.x + v.at[0] * S.view.s, sy = S.view.y + v.at[1] * S.view.s, d = Math.hypot(sx - mx, sy - my);
    if (d < Math.max(5, viaStyle(v.q_w, qmax).r + 2) && d < bd) { bd = d; best = v; }
  }
  return best;
}

function showThermTip(mx, my) {
  const tip = $("tip");
  if (!thermOk()) { tip.hidden = true; return; }
  const t = S.therm, via = thermViaAt(mx, my);
  if (via) {
    tip.innerHTML = `<div class="t1">${via.hole ? "Plated hole" : "Via"} · ${esc(netLabel(via.net))} · estimate</div><div class="kv">
      <span>Heat through it</span><span>${fmtSI(via.q_w, "W")}${via.q_w < 0 ? " (upwards)" : ""}</span>
      <span>ΔT ${esc(layerName(via.from))} → ${esc(layerName(via.to))}</span><span>${fmtK(via.dt_k)}</span>
      <span>Temperature</span><span>${fmtT(via.t_c)}</span>
      <span>Drill</span><span>${fmtSI(via.drill * 1e-3, "m")}</span></div>`;
  } else {
    const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s;
    const i = Math.floor((x - t.x0) / t.cell), j = Math.floor((y - t.y0) / t.cell);
    if (i < 0 || j < 0 || i >= t.nx || j >= t.ny) { tip.hidden = true; return; }
    const v = t.t[S.thermLayer * t.nx * t.ny + j * t.nx + i];
    if (!isFinite(v)) { tip.hidden = true; return; }
    const near = thermPartsOn(S.thermLayer).map((p) => ({ p, d: Math.hypot(p.at[0] - x, p.at[1] - y) })).sort((a, b) => a.d - b.d)[0];
    tip.innerHTML = `<div class="t1">Temperature · estimate</div><div class="kv">
      <span>Layer</span><span>${esc(layerName(S.thermLayer))}</span>
      <span>Board</span><span>${fmtT(v)}</span>
      ${near && near.d < 3 ? `<span>Part</span><span>${esc(near.p.refdes)} ${esc(near.p.part)} · ${fmtSI(near.p.w, "W")} · Tj ${fmtT(near.p.tj_c)}${near.p.t_case_c != null ? ` · case ${fmtT(near.p.t_case_c)}` : ""}</span>` : ""}</div>`;
  }
  tip.hidden = false;
  const rs = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let tx = mx + 16, ty = my + 16;
  if (tx + tw > rs.width - 8) tx = mx - tw - 16;
  if (ty + th > rs.height - 8) ty = my - th - 16;
  tip.style.left = Math.max(8, tx) + "px"; tip.style.top = Math.max(8, ty) + "px";
}

// show a point of the map: its layer, zoomed and marked (rails and parts in the panel)
function thermJump(at, layer, label) {
  if (!at) return;
  S.thermLayer = layer; S.thermMark = { at, label };
  renderThermChips(); renderThermLegend();
  fitView([at[0] - 8, at[1] - 8, at[0] + 8, at[1] + 8]);
}

// side panel of the Temperature view: summary, parts, rails at temperature
function renderThermDetail() {
  const box = $("pi-detail");
  if (!thermOk()) { box.hidden = true; return; }
  const t = S.therm, c = t.cooling, hot = thermHottest(), n = S.scene.stackup.copper.length;
  const st = (p) => p.status === "over" ? "over Tj max" : p.status === "near" ? "near Tj max" : "";
  const thermTitle = (p) => `${p.model === "theta_jb" ? `θJB ${fmt(p.theta_jb, 1)}${p.theta_jc ? `, θJC ${fmt(p.theta_jc, 1)}` : ""} K/W` : p.model === "psi_jb" ? `ψJB ${fmt(p.psi_jb, 1)} K/W` : `θJA ${fmt(p.theta_ja, 1)} K/W (JEDEC board)`} – ${p.data_source}; Tj max: ${p.tj_max_source}${p.w_source === "PI_THERMAL p" ? "; heat from PI_THERMAL" : ""}`;
  const viaCell = (p) => {
    const v = p.vias;
    if (!v) return "–";
    const tip = `into the board ${fmtSI(v.to_board_w, "W")}: ${fmtSI(v.via_w, "W")} through ${v.count} via${v.count === 1 ? "" : "s"}, ${fmtSI(v.laminate_w, "W")} through the laminate, ${fmtSI(v.lateral_w, "W")} sideways in the pad layer; ΔT pad layer → next layer ${fmtK(v.dt_next_k)}, → other side ${fmtK(v.dt_through_k)}`;
    return `<span title="${esc(tip)}">${v.count} · ${v.to_board_w > 0 ? fmt(100 * v.via_w / v.to_board_w, 0) : "–"} %${v.hint ? " ⚠" : ""}</span>`;
  };
  const parts = t.parts.slice(0, 12).map((p) => `<tr class="${p.status === "over" ? "t-over" : p.status === "near" ? "t-near" : ""}" title="${esc(thermTitle(p))}">
      <td><button class="t-part" data-tpart="${esc(p.refdes)}" title="Show ${esc(p.refdes)} on the map">${esc(p.refdes)}</button></td><td>${esc(p.part)}</td><td class="n">${fmtSI(p.w, "W")}</td><td class="n">${tCell(p.t_board_c)}</td><td class="n">${tCell(p.tj_c, p.estimate ? "*" : "")}</td><td class="n">${fmtTn(p.t_case_c)}</td><td class="n">${fmtTn(p.tj_max_c)}</td><td class="n">${viaCell(p)}</td><td>${st(p)}</td></tr>`).join("");
  const hints = t.parts.filter((p) => p.vias && p.vias.hint).map((p) => `<div class="warn"><button class="t-part" data-tpart="${esc(p.refdes)}">${esc(p.refdes)}</button>: ${esc(p.vias.hint)} (estimate)</div>`).join("");
  const rails = t.rails.filter((r) => !r.error && r.drop_mv_20c != null && r.drop_mv_hot != null).sort((a, b) => b.drop_mv_hot - a.drop_mv_hot).slice(0, 10);
  const railRows = rails.map((r) => `<tr><td><button class="t-part" data-trail="${esc(r.net)}" title="Show where the copper of ${esc(netShort(r.net))} is hottest">${esc(netShort(r.net))}</button>${r.ground ? " <span class=\"muted\">return</span>" : ""}</td>
      <td class="n">${tCell(r.t_max_c)}</td><td class="n">${fmtTn(r.t_mean_c)}</td><td class="n">${fmtSI(r.loss_w_hot, "W")}</td><td class="n">${fmtSI(r.drop_mv_20c * 1e-3, "V")}</td><td class="n">${fmtSI(r.drop_mv_hot * 1e-3, "V")}</td><td class="n">+${fmt(r.drop_mv_20c > 0 ? 100 * (r.drop_mv_hot / r.drop_mv_20c - 1) : 0, 0)} %</td></tr>`).join("");
  const nDef = t.parts.filter((p) => p.estimate).length;
  box.innerHTML = `<header><div><h3>Temperature · estimate</h3><div class="muted">steady state, ${fmtSI(t.parts_w + t.copper_w, "W")}, ${c.airflow_m_s ? `air flow ${c.airflow_m_s} m/s` : `still air, ${c.orientation}`}, ambient ${fmtT(t.ambient_c)} · grid ${fmtSI(t.cell * 1e-3, "m")}</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>
    <div class="dstats"><span>Board max <b>${fmtT(t.t_max_c)}</b> (${esc(layerName(t.t_max_layer))})</span><span>Hottest junction <b>${hot ? fmtT(hot.tj_c) : "–"}</b>${hot ? ` ${esc(hot.refdes)}` : ""}</span><span title="surface heat transfer coefficient per side: convection and radiation">Air: h <b>${fmt(c.h_top, 1)} / ${fmt(c.h_bottom, 1)}</b> W/(m²·K) top / bottom${c.estimated ? " (estimate)" : ""}</span></div>
    <h4 class="sub">Parts: over or near Tj max first, then the hottest</h4>
    <div class="tablewrap"><table class="xs-table ctab"><thead><tr>
      <th>Part</th><th></th>
      <th class="n" title="heat dissipated in the part">heat</th>
      <th class="n" title="board temperature under the part's pads (°C)">board °C</th>
      <th class="n" title="junction temperature (°C); * = estimated package data or θJA only">Tj °C</th>
      <th class="n" title="top of the package (°C): Tj − ψJT·P (PI_THERMAL psi_jt) or the case node of the two-resistor model (Tj − θJC·heat through the top)">case top °C</th>
      <th class="n" title="maximum junction temperature (°C; PI_THERMAL tj_max, else 125 °C assumed)">Tj max °C</th>
      <th class="n" title="vias under the part's pads · share of the part's heat into the board that goes down through them (⚠: looks short of thermal vias; hover for the split)">vias · down</th><th></th></tr></thead>
      <tbody>${parts || `<tr><td>no part with heat (power tree in step 1)</td></tr>`}</tbody></table></div>
    ${hints}
    <h4 class="sub">Rails at temperature</h4>
    <div class="tablewrap"><table class="xs-table ctab"><thead><tr>
      <th>Rail</th>
      <th class="n" title="highest temperature of the copper that carries the rail's current, °C (click the rail to show it)">copper max °C</th>
      <th class="n" title="copper temperature weighted by the copper loss, °C">copper mean °C</th>
      <th class="n" title="copper loss (I²R) of the rail with the copper at its temperature">copper loss</th>
      <th class="n" title="largest IR drop at a load (ground return: largest offset), copper at 20 °C – the value of step 2">IR drop 20 °C</th>
      <th class="n" title="the same with the copper resistance at the estimated temperature (+0.393 %/K)">IR drop hot</th>
      <th class="n" title="rise of the IR drop from 20 °C to the estimated temperature">rise</th></tr></thead>
      <tbody>${railRows || `<tr><td>–</td></tr>`}</tbody></table></div>
    <div class="dstats"><span>Heat in ${fmtSI(t.balance.in_w, "W")} = convection ${fmtSI(t.balance.convection_w, "W")} + radiation ${fmtSI(t.balance.radiation_w, "W")}</span></div>
    <p class="note">${nDef ? `* ${nDef} part${nDef === 1 ? "" : "s"} with estimated package data or θJA only: add PI_THERMAL (theta_jb, theta_jc, psi_jt, tj_max) from the datasheet. ` : ""}${esc(t.notes[0] || "")} Case top: what a finger or a heat sink meets on the package; touch limits (IEC 62368-1, metal held over 1 min) are about 48 °C – an enclosure is not modelled.</p>
    ${t.warnings.length ? `<div>${t.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}</div>` : ""}`;
  box.hidden = false;
  $("closePi").onclick = () => { box.hidden = true; };
  box.querySelectorAll("[data-tpart]").forEach((b) => b.onclick = () => {
    const p = t.parts.find((x) => x.refdes === b.dataset.tpart);
    if (p) thermJump(p.at, p.side === "bottom" ? n - 1 : 0, `${p.refdes} Tj ${fmtT(p.tj_c)}`);
  });
  box.querySelectorAll("[data-trail]").forEach((b) => b.onclick = () => {
    const r = t.rails.find((x) => x.net === b.dataset.trail);
    if (r && r.max_at) thermJump(r.max_at, r.max_layer, `${netShort(r.net)} ${fmtT(r.t_max_c)}`);
  });
}
