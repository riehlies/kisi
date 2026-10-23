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
function thermReset() { S.therm = null; S.thermImg = null; if (S.piMode === "t") S.piMode = S.piSolved ? "v" : "b"; }

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
  S.therm = th; S.thermImg = null;
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
    $("therm-step").textContent = `Done in ${t.ms} ms. Board up to ${fmt(t.t_max_c, 0)} °C (${layerName(t.t_max_layer)})` +
      (h ? `, hottest part ${h.refdes} with Tj ≈ ${fmt(h.tj_c, 0)} °C` : "") + `. Estimate for ${fmt(t.parts_w + t.copper_w, 2)} W in ${t.cooling.airflow_m_s ? `air flow of ${t.cooling.airflow_m_s} m/s` : "still air"} at ${fmt(t.ambient_c, 0)} °C.`;
    return;
  }
  $("therm-step").textContent = S.piSolved
    ? "Steady-state temperature of the board from the losses of step 2: copper of every layer, laminate and vias conduct, both sides lose heat to the air; junction temperature per part (θ values from PI_THERMAL or package estimates) and the IR drop at temperature."
    : "Needs the losses of step 2.";
}
const layerName = (l) => (S.scene.stackup.copper[l] || {}).name || `layer ${l}`;
const thermHottest = () => S.therm.parts.reduce((a, p) => (!a || p.tj_c > a.tj_c ? p : a), null);

// colour scale: ambient … hottest cell of all layers (linear)
function thermImage() {
  const t = S.therm, l = S.thermLayer;
  if (S.thermImg && S.thermImg.layer === l) return S.thermImg;
  const per = t.nx * t.ny, lo = t.ambient_c, hi = Math.max(t.t_max_c, lo + 0.1);
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

function drawThermal(px) {
  if (!thermOk()) return;
  const t = S.therm, im = thermImage();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(im.canvas, t.x0, t.y0, t.nx * t.cell, t.ny * t.cell);
  // the hottest parts of this side with their junction temperature
  const labels = [], cs = getComputedStyle(document.documentElement);
  const col = { over: cs.getPropertyValue("--z-high2").trim(), near: cs.getPropertyValue("--z-high1").trim(), ok: cs.getPropertyValue("--fg").trim() };
  for (const p of thermPartsOn(S.thermLayer).slice(0, 6)) {
    partBox(fpBBox(p.refdes), col[p.status] || col.ok, 1.5 * px, `${p.refdes} ${fmt(p.tj_c, 0)} °C`, { bold: p.status !== "ok" }, labels, px);
  }
  drawPartLabels(labels, px);
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
  const t = S.therm, im = thermImage(), lay = t.layers[S.thermLayer];
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  L.innerHTML = `<strong>Temperature · ${esc(layerName(S.thermLayer))} · estimate</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>${fmt(im.lo, 0)}</span><span>${fmt((im.lo + im.hi) / 2, 0)}</span><span>${fmt(im.hi, 0)} °C</span></div>
    <div class="row">This layer: up to ${fmt(lay.t_max_c, 1)} °C, mean ${fmt(lay.t_mean_c, 1)} °C. Labels: junction temperature of the hottest parts on this side.</div>`;
}

function showThermTip(mx, my) {
  const tip = $("tip");
  if (!thermOk()) { tip.hidden = true; return; }
  const t = S.therm;
  const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s;
  const i = Math.floor((x - t.x0) / t.cell), j = Math.floor((y - t.y0) / t.cell);
  if (i < 0 || j < 0 || i >= t.nx || j >= t.ny) { tip.hidden = true; return; }
  const v = t.t[S.thermLayer * t.nx * t.ny + j * t.nx + i];
  if (!isFinite(v)) { tip.hidden = true; return; }
  const near = thermPartsOn(S.thermLayer).map((p) => ({ p, d: Math.hypot(p.at[0] - x, p.at[1] - y) })).sort((a, b) => a.d - b.d)[0];
  tip.innerHTML = `<div class="t1">Temperature · estimate</div><div class="kv">
    <span>Layer</span><span>${esc(layerName(S.thermLayer))}</span>
    <span>Board</span><span>${fmt(v, 1)} °C</span>
    ${near && near.d < 3 ? `<span>Part</span><span>${esc(near.p.refdes)} ${esc(near.p.part)} · ${mW(near.p.w)} · Tj ${fmt(near.p.tj_c, 1)} °C</span>` : ""}</div>`;
  tip.hidden = false;
  const rs = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let tx = mx + 16, ty = my + 16;
  if (tx + tw > rs.width - 8) tx = mx - tw - 16;
  if (ty + th > rs.height - 8) ty = my - th - 16;
  tip.style.left = Math.max(8, tx) + "px"; tip.style.top = Math.max(8, ty) + "px";
}

// side panel of the Temperature view: summary, hottest parts, rails at temperature
function renderThermDetail() {
  const box = $("pi-detail");
  if (!thermOk()) { box.hidden = true; return; }
  const t = S.therm, c = t.cooling, hot = thermHottest();
  const st = (p) => p.status === "over" ? "over Tj max" : p.status === "near" ? "near Tj max" : "";
  const parts = t.parts.slice(0, 12).map((p) => `<tr class="${p.status === "over" ? "t-over" : p.status === "near" ? "t-near" : ""}" title="${esc(`${p.model === "theta_jb" ? `θJB ${fmt(p.theta_jb, 1)}${p.theta_jc ? `, θJC ${fmt(p.theta_jc, 1)}` : ""} K/W` : p.model === "psi_jb" ? `ψJB ${fmt(p.psi_jb, 1)} K/W` : `θJA ${fmt(p.theta_ja, 1)} K/W (JEDEC board)`} – ${p.data_source}; Tj max: ${p.tj_max_source}${p.w_source === "PI_THERMAL p" ? "; heat from PI_THERMAL" : ""}`)}">
      <td><button class="t-part" data-tpart="${esc(p.refdes)}" title="Show ${esc(p.refdes)} on the board">${esc(p.refdes)}</button></td><td>${esc(p.part)}</td><td class="n">${fmt(p.w * 1e3, 0)}</td><td class="n">${fmt(p.t_board_c, 1)}</td><td class="n">${fmt(p.tj_c, 1)}${p.estimate ? "*" : ""}</td><td class="n">${fmt(p.tj_max_c, 0)}</td><td>${st(p)}</td></tr>`).join("");
  const rails = t.rails.filter((r) => !r.error && r.drop_mv_20c != null && r.drop_mv_hot != null).sort((a, b) => (b.drop_mv_hot - b.drop_mv_20c) - (a.drop_mv_hot - a.drop_mv_20c)).slice(0, 8);
  const railRows = rails.map((r) => `<tr><td>${esc(netShort(r.net))}${r.ground ? " <span class=\"muted\">return</span>" : ""}</td><td class="n">${fmt(r.t_max_c, 1)}</td><td class="n">${fmt(r.drop_mv_20c, 2)}</td><td class="n">${fmt(r.drop_mv_hot, 2)}</td></tr>`).join("");
  const nDef = t.parts.filter((p) => p.estimate).length;
  box.innerHTML = `<header><div><h3>Temperature · estimate</h3><div class="muted">steady state, ${fmt(t.parts_w + t.copper_w, 2)} W, ${c.airflow_m_s ? `air flow ${c.airflow_m_s} m/s` : `still air, ${c.orientation}`}, ambient ${fmt(t.ambient_c, 0)} °C · grid ${fmt(t.cell, 2)} mm</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>
    <div class="dstats"><span>Board max <b>${fmt(t.t_max_c, 1)} °C</b> (${esc(layerName(t.t_max_layer))})</span><span>Hottest junction <b>${hot ? `${fmt(hot.tj_c, 1)} °C` : "–"}</b>${hot ? ` ${esc(hot.refdes)}` : ""}</span><span>Air: h <b>${fmt(c.h_top, 1)} / ${fmt(c.h_bottom, 1)}</b> W/m²K top / bottom${c.estimated ? " (estimate)" : ""}</span></div>
    <h4 class="sub">Parts: over or near Tj max first, then the hottest <em>mW, °C</em></h4>
    <div class="tablewrap"><table class="xs-table ctab"><thead><tr><th>Part</th><th></th><th>mW</th><th>board</th><th>Tj</th><th>max</th><th></th></tr></thead><tbody>${parts || `<tr><td>no part with heat (power tree in step 1)</td></tr>`}</tbody></table></div>
    <h4 class="sub">Rails at temperature <em>°C, mV</em></h4>
    <div class="tablewrap"><table class="xs-table ctab"><thead><tr><th>Rail</th><th>copper max</th><th>drop 20 °C</th><th>at temp.</th></tr></thead><tbody>${railRows || `<tr><td>–</td></tr>`}</tbody></table></div>
    <div class="dstats"><span>Heat in ${fmt(t.balance.in_w, 2)} W = convection ${fmt(t.balance.convection_w, 2)} W + radiation ${fmt(t.balance.radiation_w, 2)} W</span></div>
    <p class="note">${nDef ? `* ${nDef} part${nDef === 1 ? "" : "s"} with estimated package data or θJA only: add PI_THERMAL (theta_jb, theta_jc, tj_max) from the datasheet. ` : ""}${esc(t.notes[0] || "")} The copper's resistance rises by 0.393 %/K: the IR drop at temperature is one electro-thermal step.</p>
    ${t.warnings.length ? `<div>${t.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}</div>` : ""}`;
  box.hidden = false;
  $("closePi").onclick = () => { box.hidden = true; };
  box.querySelectorAll("[data-tpart]").forEach((b) => b.onclick = () => {
    const bbox = fpBBox(b.dataset.tpart);
    if (bbox) { const cc = [(bbox[0] + bbox[2]) / 2, (bbox[1] + bbox[3]) / 2], h = Math.max(8, (bbox[2] - bbox[0]), (bbox[3] - bbox[1])) * 1.5; fitView([cc[0] - h, cc[1] - h, cc[0] + h, cc[1] + h]); }
  });
}
